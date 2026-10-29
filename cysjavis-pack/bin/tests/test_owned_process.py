#!/usr/bin/env python3
"""C-03 real-process ownership regressions; all children belong to this fixture."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

BIN = Path(os.environ.get('C03_TEST_BIN', Path(__file__).resolve().parents[1]))
PYTHON = sys.executable


def load(name):
    spec = importlib.util.spec_from_file_location(name, BIN / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


# marker() reads a pid the fixture publishes with os.replace. On Windows that read can hit a sharing race and raise
# PermissionError (run 3 finding, CEO ruling run-3 s2c/s4): retried there only, within BOTH bounds below.
MARKER_PERM_ATTEMPTS = 25
MARKER_PERM_BUDGET_S = 1.0


def _start_time(pid):
    """POSIX identity of a pid: ('ok', start time) | ('gone', None) | ('unavailable', None) when ps cannot answer."""
    try:
        r = subprocess.run(['ps', '-o', 'lstart=', '-p', str(pid)], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return ('unavailable', None)
    out = r.stdout.strip()
    if out:
        return ('ok', out)
    return ('gone', None) if r.returncode != 0 else ('unavailable', None)


class OwnershipRegression(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='c03-')
        self.root = Path(self.tmp.name)
        (self.root/'_round').mkdir()
        (self.root/'_round'/'SESSION_STATE.md').write_text('C03-ISOLATED-STATE\n')
        self.children = []
        self.extra = []
        self.handles = {}
        self.marker_stats = []
        self.identities = {}
        self.not_killed = []

    def tearDown(self):
        # Fixture-only identities, retained handles. Never process-name selectors.
        self._report_marker_stats(sys.stderr)
        for p in self.children:
            if p.poll() is None:
                p.kill()
            p.wait(timeout=3)
            for stream in (p.stdin,p.stdout,p.stderr):
                if stream is not None:
                    stream.close()
        for pid in self.extra:
            try:
                self.stop_recorded(pid)
            except ProcessLookupError:
                pass
        if self.not_killed:
            sys.stderr.write('marker-identity %s: not-killed %s\n' % (self.id(), ' '.join(self.not_killed)))
            sys.stderr.flush()
        if os.name == 'nt':
            import ctypes
            for handle in self.handles.values():
                ctypes.windll.kernel32.CloseHandle(handle)
        self.tmp.cleanup()

    def start(self, text):
        script = self.root / ('driver%d.py' % len(self.children))
        script.write_text(text)
        p = subprocess.Popen([PYTHON, '-B', str(script)], stdout=subprocess.DEVNULL,
                             stderr=subprocess.PIPE, start_new_session=(os.name == 'posix'))
        self.children.append(p)
        return p

    def marker(self, name):
        pid = self._marker_pid(self.root / name, retry_permission=(os.name == 'nt'))
        self.extra.append(pid)
        if os.name == 'posix':
            self.identities[pid] = _start_time(pid)
        if os.name == 'nt':
            import ctypes
            from ctypes import wintypes
            api=ctypes.windll.kernel32
            api.OpenProcess.argtypes=[wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
            api.OpenProcess.restype=wintypes.HANDLE
            api.WaitForSingleObject.argtypes=[wintypes.HANDLE,wintypes.DWORD]
            api.TerminateProcess.argtypes=[wintypes.HANDLE,wintypes.UINT]
            api.CloseHandle.argtypes=[wintypes.HANDLE]
            self.handles[pid]=api.OpenProcess(0x100001,False,pid)
            self.assertTrue(self.handles[pid], 'fixture process handle unavailable')
        return pid

    def _marker_pid(self, path, *, retry_permission, attempts=MARKER_PERM_ATTEMPTS, budget_s=MARKER_PERM_BUDGET_S,
                    limit_s=5.0):
        # Missing, empty or partial publication is not readiness. A PermissionError is retried only when
        # retry_permission (win32: a read racing the fixture's os.replace); it ends at the attempt bound or the time
        # bound, whichever comes first, as a FAIL chained to the last error - never a pid, never a pass.
        # Every call records (name, retries, ms) before it returns or raises; tearDown prints them.
        start = time.monotonic()
        limit = start + limit_s
        retries = 0
        first_permission = None
        last_permission = None
        def record():
            waited = 0 if first_permission is None else int((time.monotonic() - first_permission) * 1000)
            self.marker_stats.append((path.name, retries, waited))
        while time.monotonic() < limit:
            try:
                pid = int(path.read_text())
            except (FileNotFoundError, ValueError):
                pass
            except PermissionError as exc:
                if not retry_permission:
                    record()
                    raise
                retries += 1
                last_permission = exc
                now = time.monotonic()
                if first_permission is None:
                    first_permission = now
                bound = ('attempt bound' if retries >= attempts else
                         'time bound' if now - first_permission >= budget_s else None)
                if bound:
                    record()
                    raise self.failureException(
                        'marker %s: PermissionError persisted, %s reached after %d attempts / %d ms '
                        '(bounds: %d attempts, %d ms): %r' % (path.name, bound, retries,
                        int((now - first_permission) * 1000), attempts, int(budget_s * 1000), exc)) from exc
            else:
                record()
                return pid
            time.sleep(.02)
        record()
        if last_permission is not None:
            raise self.failureException(
                'marker %s: readiness limit %.1f s reached inside a PermissionError streak after %d attempts: %r'
                % (path.name, limit_s, retries, last_permission)) from last_permission
        self.fail('fixture never started: ' + path.name)

    def _report_marker_stats(self, stream):
        if self.marker_stats:
            stream.write('\nmarker-retries %s: %s\n' % (self.id(), ' '.join('%s=%d/%dms' % st for st in self.marker_stats)))
            stream.flush()

    def recorded_alive(self,pid):
        if os.name == 'nt':
            import ctypes
            return ctypes.windll.kernel32.WaitForSingleObject(self.handles[pid],0)==258
        return alive(pid)

    def stop_recorded(self,pid):
        if os.name == 'nt':
            import ctypes
            ctypes.windll.kernel32.TerminateProcess(self.handles[pid],125)
            return
        # POSIX: a pid is a number the OS reuses once the process ends. SIGKILL only the process whose start time
        # still equals the one read when marker() recorded it; an unreadable identity refuses (never kills blind).
        recorded = self.identities.get(pid, ('unavailable', None))
        now = _start_time(pid)
        if now[0] == 'gone':
            return
        if recorded[0] != 'ok' or now[0] != 'ok':
            self.not_killed.append('%d:identity-unavailable' % pid)
            return
        if now[1] != recorded[1]:
            self.not_killed.append('%d:start-time-changed' % pid)
            return
        os.kill(pid,signal.SIGKILL)

    def assert_gone(self, pid):
        end = time.monotonic() + 3
        while self.recorded_alive(pid) and time.monotonic() < end:
            time.sleep(.02)
        self.assertFalse(self.recorded_alive(pid), 'owned descendant survived parent death: %s' % pid)

    @unittest.skipUnless(os.name == 'posix', 'POSIX identity fixture; native Job fixtures below')
    def test_checklist_abrupt_death_closes_descendant(self):
        marker = self.root / 'probe'
        probe = self.root / 'probe.py'
        probe.write_text("import os,time\nfrom pathlib import Path\n_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\ntime.sleep(30)\n" % str(marker))
        p = self.start("import sys\nsys.path.insert(0,%r)\nimport javis_checklist as c\nc.run_preflight([sys.executable,'-B',%r],timeout=20)\n" % (str(BIN), str(probe)))
        pid = self.marker('probe')
        p.kill(); p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed driver; the verdict is assert_gone
        self.assert_gone(pid)

    @unittest.skipUnless(os.name == 'posix', 'POSIX identity fixture; native Job fixtures below')
    def test_preflight_abrupt_death_closes_scanner(self):
        marker = self.root / 'scan'
        scan = self.root / 'scan.py'
        scan.write_text("import os,time,sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\ndef main():\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n time.sleep(30)\ntry:\n from _owned_process import managed_main\nexcept ImportError:\n main()\nelse:\n managed_main(main)\n" % (str(BIN), str(marker)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        pid = self.marker('scan')
        p.kill(); p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed driver; the verdict is assert_gone
        self.assert_gone(pid)

    def test_scanner_then_preflight_death_reaches_spawn_worker(self):
        skill = self.root / 'skills' / 'sample'; skill.mkdir(parents=True)
        (skill / 'SKILL.md').write_text('# fixture')
        scan = self.root / 'scan_pool.py'
        scan.write_text("""import os,sys,time,multiprocessing
from pathlib import Path
sys.path.insert(0,%r)
import javis_skillscan as scanner
try:
 from _owned_process import managed_main
except ImportError:
 managed_main=lambda main:main()
def slow_scan(path):
 _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)
 time.sleep(30)
 return {}
def main():
 multiprocessing.set_start_method('spawn',force=True)
 scanner.scan_skill=slow_scan
 _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)
 return scanner.cmd_all([%r],True)
if __name__=='__main__':
 managed_main(main)
""" % (str(BIN), str(self.root/'worker'), str(self.root/'scanner'), str(skill.parent)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        scanner = self.marker('scanner'); worker = self.marker('worker')
        self.assertNotEqual(scanner,worker,'pool fixture silently fell back to serial')
        self.stop_recorded(scanner)
        p.kill(); p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed driver; the verdict is assert_gone(worker)
        self.assert_gone(worker)

    @unittest.skipUnless(os.name == 'posix', 'POSIX late-spawn barrier')
    def test_late_spawn_is_contained_after_cancel(self):
        script = self.root/'late.py'
        script.write_text("""import os,sys,time,subprocess
from pathlib import Path
sys.path.insert(0,%r)
import _owned_process as owned
stop=owned._self_stop
def paused_stop(group):
 while not Path(%r).exists(): time.sleep(.005)
 stop(group)
owned._self_stop=paused_stop
def main():
 assert not owned.CANCEL.is_set()
 # The fixture pauses the watcher at its teardown boundary, not the launcher.
 # Cancellation is set BEFORE Popen; the late child must really write its PID.
 owned.CANCEL.set()
 p=subprocess.Popen([sys.executable,'-B','-c',%r])
 time.sleep(30)
if __name__=='__main__': owned.managed_main(main)
""" % (str(BIN),str(self.root/'late'),
        "import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker);time.sleep(30)" % str(self.root/'late')))
        sentinel=self.start('import time;time.sleep(30)')
        driver = """import sys
from pathlib import Path
sys.path.insert(0,%r)
from _owned_process import run_owned
r=run_owned([sys.executable,'-B',%r],3,force=1)
Path(%r).write_text(str(r))
""" % (str(BIN),str(script),str(self.root/'result'))
        p=self.start(driver); late=self.marker('late');p.wait(timeout=5)  # wallclock: guard 5 s only ends a hang of the driver; the verdict is assert_gone, the sibling alive and complete False
        self.assert_gone(late)
        self.assertIsNone(sentinel.poll(), 'outside sibling was terminated')
        self.assertIn("'complete': False", (self.root/'result').read_text())

    def test_light_fence_does_not_run_cancelled_check(self):
        pf = load('javis_preflight')
        # The module's cancellation Event is the managed run's light fence.
        import threading
        event = threading.Event(); event.set()
        pf._CANCEL = event
        called = []
        pf.Preflight(fix=False, skips=[])._run_check_isolated(lambda: called.append(True))
        self.assertEqual(called, [], 'cancelled check still executed')

    @unittest.skipUnless(os.name == 'posix', 'bash ingress fixture')
    def test_hook_withheld_stdin_eof_is_bounded(self):
        hook=Path(__file__).resolve().parents[2]/'hooks'/'inject-context.sh'
        p=subprocess.Popen(['/bin/bash',str(hook)],stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
        self.children.append(p)
        try:
            # Stdin is withheld forever, so only the ingress phase budget can end the hook.
            try:
                p.wait(timeout=60)  # wallclock: guard 60 s only ends a hang; the verdict is that the hook ended (rc 0, 'incomplete')
            except subprocess.TimeoutExpired:
                self.fail('ingress exceeded its 20-second phase budget')
            self.assertEqual(p.returncode,0)
            self.assertIn(b'incomplete',p.stdout.read())
        finally:
            if p.returncode is None:
                os.killpg(p.pid,signal.SIGKILL)
                p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed hook group; cleanup only, not the verdict

    @unittest.skipUnless(os.name == 'posix', 'POSIX group identity')
    def test_no_destructive_signal_after_reap(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        events=[]; kill=owned.os.killpg; wait=owned.subprocess.Popen.wait
        def observe_kill(pid,sig):
            events.append(('signal',sig));return kill(pid,sig)
        def observe_wait(p,*args,**kwargs):
            events.append(('wait',));return wait(p,*args,**kwargs)
        script=self.root/'exit.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:0)\n" % str(BIN))
        with patch.object(owned.os,'killpg',observe_kill),patch.object(owned.subprocess.Popen,'wait',observe_wait):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertTrue(result['complete'],result)
        boundary=next(i for i,e in enumerate(events) if e[0]=='wait')
        self.assertFalse(any(e[0]=='signal' and e[1]!=0 for e in events[boundary+1:]),events)
        self.assertTrue(any(e[0]=='signal' and e[1]!=0 for e in events[:boundary]),events)

    @unittest.skipUnless(os.name == 'posix', 'POSIX uncertain observation')
    def test_uncertain_group_observation_is_not_complete(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        kill=owned.os.killpg
        def uncertain(pid,sig):
            if sig==0: raise PermissionError('fixture observation denied')
            return kill(pid,sig)
        script=self.root/'exit.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:0)\n" % str(BIN))
        with patch.object(owned.os,'killpg',uncertain):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertEqual(result['cleanup'],'unknown')
        self.assertFalse(result['complete'],result)
        self.assertEqual(result.get('diagnostic_returncode'),0)
        self.assertIsNotNone(result.get('kernel_returncode'))

    @unittest.skipUnless(os.name == 'nt', 'Windows Job Object: UNMEASURED on this host')
    def test_windows_assignment_refusal_never_sends_go(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        marker=self.root/'uncontained'
        script=self.root/'refused.py'
        script.write_text("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n Path(%r).touch()\n return 0\nmanaged_main(main)\n" % (str(BIN),str(marker)))
        with patch.object(owned.Job,'assign',side_effect=OSError('fixture nesting refusal')):
            result=owned.run_owned([PYTHON,'-B',str(script)],1,force=1)
        self.assertFalse(marker.exists(),'GO admitted uncontained work')
        self.assertFalse(result['complete'],result)
        self.assertIn('refusal',result['reason'])
        self.assertLess(result['elapsed'],3)

    @unittest.skipUnless(os.name == 'nt', 'Windows Job Object: UNMEASURED on this host')
    def test_windows_parent_death_closes_job_without_harming_sibling(self):
        child=self.root/'child.py'
        child.write_text("import os,sys,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n time.sleep(30)\nmanaged_main(main)\n" % (str(BIN),str(self.root/'job-child')))
        sibling=self.start('import time;time.sleep(30)')
        owner=self.start("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),str(child)))
        pid=self.marker('job-child');owner.kill();owner.wait(timeout=60)  # wallclock: guard 60 s only ends a hang reaping the killed owner; the verdict is assert_gone and the sibling alive
        self.assert_gone(pid)
        self.assertIsNone(sibling.poll())

    @unittest.skipUnless(os.name == 'posix', 'POSIX outer-owner cascade')
    def test_outer_helper_death_reaches_real_checklist_and_preflight(self):
        pack=self.root/'pack';(pack/'bin').mkdir(parents=True)
        preflight=pack/'bin'/'javis_preflight.py'
        preflight.write_text("import os,sys,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getppid()));__import__('os').replace(_tmp,_marker)\n time.sleep(30)\nmanaged_main(main)\n" % (str(BIN),str(self.root/'preflight'),str(self.root/'checklist')))
        owner=self.start("import os,sys\nsys.path.insert(0,%r)\nos.environ['CYS_PACK_DIR']=%r\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),str(pack),str(BIN/'javis_checklist.py')))
        pf=self.marker('preflight');ck=self.marker('checklist')
        owner.kill();owner.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed owner; the verdict is assert_gone and empty stderr
        self.assert_gone(pf);self.assert_gone(ck)
        self.assertEqual(owner.stderr.read(),b'')

    def test_phase_budget_uses_cross_process_clock(self):
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        clock=getattr(owned,'monotonic',time.monotonic)
        # A phase that started 10 s ago in another process has spent its .6 s budget.
        start=clock()-10.0
        marker=self.root/'should-not-start'
        result=subprocess.run([PYTHON,'-B',str(BIN/'_owned_process.py'),  # wallclock: guard 3 s only ends a hang; the verdict is no marker and rc 124
            '--bounded','2','--start',str(start),'--budget','.6','--',
            PYTHON,'-B','-c',"from pathlib import Path;Path(%r).touch()" % str(marker)],
            capture_output=True,timeout=3)
        self.assertFalse(marker.exists(),'shared phase budget reset across processes')
        self.assertEqual(result.returncode,124,result.stderr)

    def test_timeout_retains_complete_metadata_pairs(self):
        # A phase may discover a canonical path before a later file read stalls.
        command=[PYTHON,'-B',str(BIN/'_owned_process.py'),'--bounded','.4','--records','--',
                 PYTHON,'-B','-c',"import sys,time;sys.stdout.write('STATE\\0/fixture/state\\0PARTIAL');sys.stdout.flush();time.sleep(30)"]
        p=subprocess.run(command,capture_output=True,timeout=3)  # wallclock: guard 3 s only ends a hang; the verdict is rc 124 and the retained record pairs
        self.assertEqual(p.returncode,124,p.stderr)
        self.assertIn(b'STATE\0/fixture/state\0',p.stdout)
        self.assertNotIn(b'PARTIAL',p.stdout)
        self.assertIn(b'_OWNED_FAILURE\0',p.stdout)

    def test_cancelled_next_stage_cannot_spawn_late_child(self):
        import threading
        pf=load('javis_preflight');event=threading.Event();event.set();pf._CANCEL=event
        started=[]
        def next_stage():
            p=self.start('import time;time.sleep(30)')
            started.append(p.pid)
        pf.Preflight(fix=False,skips=[])._run_check_isolated(next_stage)
        self.assertEqual(started,[], 'late child started after cancellation was already visible')

    def test_checklist_timeout_reaps_held_output_tree(self):
        import queue,threading
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        sys.path.insert(0,str(BIN));import javis_checklist as ck
        child=self.root/'held_custom.py'
        code="import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker);time.sleep(3600)" % str(self.root/'held-grandchild')
        # Every timed actor outlives the 60 s guard, so no natural exit can end the run inside it.
        child.write_text("import os,subprocess,sys,time\nfrom pathlib import Path\n_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));os.replace(_tmp,_marker)\nsubprocess.Popen([sys.executable,'-B','-c',%r])\ntime.sleep(3600)\n" % (str(self.root/'held-child'),code))
        results=queue.Queue()
        # Start the tested timeout only after the real grandchild exists. This
        # isolates reap behavior from interpreter-start scheduling; startup
        # admission has separate native tests above.
        clock=owned.monotonic;origin=clock();armed=[]
        def ready_clock():
            now=clock()
            if not armed and (self.root/'held-grandchild').exists():armed.append(now)
            return origin+(now-armed[0] if armed else 0)
        # The grandchild holds the output for an hour and the cooperative grace is
        # 600 s, so returning at all is the event: a window waiting out either fails.
        # The test owns the patches: a thread left hung by a failure must not keep
        # them for the tests after it.
        for pt in (patch.object(owned,'monotonic',ready_clock),patch.object(subprocess,'_time',ready_clock),
                   patch.object(ck,'PREFLIGHT_GRACE',600)):
            pt.start();self.addCleanup(pt.stop)
        def run():
            results.put(ck.run_preflight([PYTHON,'-B',str(child)],timeout=.5))  # wallclock: guard .5 is the product timeout under test (an input), not a wait of this test
        threading.Thread(target=run,daemon=True).start()
        self.marker('held-child');pid=self.marker('held-grandchild')
        try:
            result=results.get(timeout=60)  # wallclock: guard 60 s only ends a hang; the verdict is that run_preflight returned 'timeout'
        except queue.Empty:
            self.fail('checklist blocked draining a grandchild-held output pipe after timeout')
        self.assertEqual(result[0],'timeout')
        self.assert_gone(pid)

    def test_private_done_preserves_exit_and_output(self):
        sys.path.insert(0,str(BIN))
        from _owned_process import run_owned
        script=self.root/'normal.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n print('original summary')\n return 7\nmanaged_main(main)\n" % str(BIN))
        result=run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertTrue(result['complete'],result)
        self.assertEqual(result['returncode'],7)
        self.assertEqual(result.get('diagnostic_returncode'),7)
        self.assertIsNotNone(result.get('kernel_returncode'))
        self.assertNotEqual(result['kernel_returncode'],7)
        self.assertEqual(result['stdout'],'original summary'+os.linesep)

    def test_timeout_drain_is_bounded_with_held_output(self):
        import queue,threading
        from unittest.mock import patch
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        script=self.root/'held.py'
        code="import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker);time.sleep(3600)" % str(self.root/'drain-grandchild')
        script.write_text("import os,sys,subprocess,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));os.replace(_tmp,_marker)\n subprocess.Popen([sys.executable,'-B','-c',%r])\n time.sleep(3600)\nmanaged_main(main)\n" % (str(BIN),str(self.root/'drain-leader'),code))
        results=queue.Queue()
        # As in the checklist held-output test: the timeout starts once the
        # output-holding grandchild exists. The leader and the grandchild both
        # outlive the 60 s guard, so returning at all is the event: an unbounded
        # drain or reap never returns.
        clock=owned.monotonic;origin=clock();armed=[]
        def ready_clock():
            now=clock()
            if not armed and (self.root/'drain-grandchild').exists():armed.append(now)
            return origin+(now-armed[0] if armed else 0)
        for pt in (patch.object(owned,'monotonic',ready_clock),patch.object(subprocess,'_time',ready_clock)):
            pt.start();self.addCleanup(pt.stop)
        def run():
            results.put(owned.run_owned([PYTHON,'-B',str(script)],.3,force=1))
        threading.Thread(target=run,daemon=True).start()
        self.marker('drain-leader');self.marker('drain-grandchild')
        try:
            result=results.get(timeout=60)  # wallclock: guard 60 s only ends a hang; the verdict is that run_owned returned, timed out and incomplete
        except queue.Empty:
            self.fail('drain blocked on output held by a grandchild after timeout')
        self.assertTrue(result['timed_out'],result)
        self.assertFalse(result['complete'])

    def run_fa(self, launch, timeout, arm, name):
        """Fix A specimens: run_owned with grace 600 / force 5 in a driver process (the old bytes block for the
        whole grace). The output holder publishes its pid as <name>; returns (returned, RESULT dict, output)."""
        script = self.root/'fa_driver.py'
        script.write_text(_FA_DRIVER % {'bin': str(BIN), 'arm': arm, 'launch': launch, 'timeout': timeout})
        # The with closes the test's own handle even when marker() fails; Windows cannot remove an open file.
        with open(self.root/'fa.out', 'w') as out_f:
            p = subprocess.Popen([PYTHON, '-B', str(script)], stdout=out_f, stderr=subprocess.STDOUT,
                                 start_new_session=(os.name == 'posix'))
            self.children.append(p)
            self.marker(name)
            try:
                p.wait(timeout=60)  # wallclock: guard 60 s only ends a hang; the old bytes spend the 600 s grace, the verdict is that run_owned returned
                returned = True
            except subprocess.TimeoutExpired:
                returned = False
                if os.name == 'posix':
                    os.killpg(p.pid, signal.SIGKILL)
        out = (self.root/'fa.out').read_text()
        result = [json.loads(l[7:]) for l in out.splitlines() if l.startswith('RESULT ')]
        return returned, (result[0] if result else None), out

    def test_run_fa_closes_its_output_file_when_the_holder_never_starts(self):
        # d2be23fe windows-build: the holder never started, marker() raised, and run_fa kept fa.out open, so tearDown's
        # rmtree failed with WinError 32 (log :588-604; the ResourceWarning for fa.out came after the E, log :544).
        from unittest.mock import patch
        class Driver:
            stdin = stdout = stderr = None
            def poll(self): return 0
            def kill(self): pass
            def wait(self, timeout=None): return 0
        handed = []
        def popen(args, **kw):
            handed.append(kw['stdout'])
            return Driver()
        def never_started(name):
            self.fail('fixture never started: ' + name)
        with patch.object(subprocess, 'Popen', popen), patch.object(self, 'marker', never_started), \
                self.assertRaises(self.failureException):
            self.run_fa([PYTHON, '-c', 'pass'], 1, '', 'fa-x')
        self.assertEqual(len(handed), 1)
        self.assertTrue(handed[0].closed, 'run_fa left its output file open after the holder failed to start')

    @unittest.skipUnless(os.name == 'posix', 'POSIX setsid twin of the nt held-output stall; on nt that is the held-output checklist test')
    def test_cooperative_window_ends_when_the_leader_is_gone(self):
        # F-A1: run_owned as run_preflight builds it. At the timeout the leader self-stops; a setsid grandchild keeps
        # the output open. Nobody is left to cooperate, so the 600 s grace must not be spent.
        code = "import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));os.replace(_tmp,_marker);time.sleep(3600)" % str(self.root/'fa1-holder')
        child = self.root/'fa1_child.py'
        child.write_text("import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-B','-c',%r],start_new_session=True)\ntime.sleep(3600)\n" % code)
        launch = [PYTHON, '-B', str(BIN/'_owned_process.py'), '--command', PYTHON, '-B', str(child)]
        returned, result, out = self.run_fa(launch, 1, str(self.root/'fa1-holder'), 'fa1-holder')
        self.assertTrue(returned, 'cooperative window outlived the leader (grace 600 s): %r' % out[-400:])
        self.assertTrue(result and result['timed_out'], out[-400:])
        self.assertFalse(result['complete'], result)

    def test_leader_crash_without_done_skips_the_cooperative_window(self):
        # F-A2: the leader crashes (control closed without DONE) while a grandchild keeps the output open.
        code = "import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));os.replace(_tmp,_marker);time.sleep(3600)" % str(self.root/'fa2-holder')
        script = self.root/'fa2_leader.py'
        script.write_text("import os,subprocess,sys,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n subprocess.Popen([sys.executable,'-B','-c',%r])\n while not Path(%r).exists(): time.sleep(.01)\n os._exit(3)\nmanaged_main(main)\n" % (str(BIN), code, str(self.root/'fa2-holder')))
        returned, result, out = self.run_fa([PYTHON, '-B', str(script)], 30, '', 'fa2-holder')
        self.assertTrue(returned, 'cooperative window outlived a crashed leader (grace 600 s): %r' % out[-400:])
        self.assertIn('control closed without DONE', (result or {}).get('reason', ''), out[-400:])
        self.assertFalse(result['complete'], result)

    @unittest.skipUnless(os.name == 'posix', 'hand-rolled POSIX lifeline leader')
    def test_cooperating_leader_finishes_inside_the_grace(self):
        # F-A3: fix A must never cut a COOPERATING leader short. This leader speaks the owner protocol by hand and,
        # once released (stdin EOF), needs a moment before its last line and its exit. The grace (30 s) must give it
        # that moment; tearing the tree down first loses the line. Its control end closes only when it exits.
        sys.path.insert(0, str(BIN))
        import _owned_process as owned
        leader = self.root/'coop_leader.py'
        leader.write_text("import os,sys,time\ncontrol=os.fdopen(int(sys.argv[-1]),'wb',buffering=0)\ncontrol.write(b'READY\\n')\n"
                          "if os.read(0,1)!=b'G': os._exit(125)\nwhile os.read(0,1): pass\ntime.sleep(.2)\n"
                          "sys.stdout.write('FINAL cooperative line\\n');sys.stdout.flush()\nos._exit(0)\n")
        result = owned.run_owned([PYTHON, '-B', str(leader)], 1, grace=30, force=5)
        self.assertTrue(result['timed_out'], result)
        self.assertIn('FINAL cooperative line', result['stdout'], result)

    def test_checklist_timeout_renders_could_not_measure(self):
        # C (H-13 at the C-03 point): a real run_preflight timeout renders the could-not-measure line, never a
        # BLOCK count. Fix A may change only how long this path takes, never the line.
        sys.path.insert(0,str(BIN))
        import javis_checklist as ck
        child=self.root/'h13_child.py'
        child.write_text("import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-B','-c','import time;time.sleep(30)'])\ntime.sleep(30)\n")
        line=ck._preflight_line([PYTHON,'-B',str(child)],timeout=1)
        self.assertEqual(line,'preflight: 실행불가(timeout)')
        self.assertNotIn('BLOCK',line)

    def test_normal_custom_command_retains_exit(self):
        sys.path.insert(0,str(BIN))
        import javis_checklist as ck
        code,last=ck.run_preflight([PYTHON,'-B','-c',"print('custom summary');raise SystemExit(3)"],timeout=10)
        self.assertEqual((code,last),(3,'custom summary'))

    def test_private_stdout_cannot_forge_done(self):
        sys.path.insert(0,str(BIN))
        from _owned_process import run_owned
        script=self.root/'forge.py'
        script.write_text("print('READY\\nDONE 0')\n")
        result=run_owned([PYTHON,'-B',str(script)],1,force=1)
        self.assertFalse(result['complete'],result)

    @unittest.skipUnless(os.name == 'posix', 'real core phase fixture')
    def test_core_timeout_keeps_discovered_canonical_path(self):
        import shutil
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True)
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py','inject-context.sh'):
            shutil.copyfile(hooks/name,pack/'hooks'/name)
        soul=pack/'soul.md';soul.write_text('# soul\n## retained path\n')
        stubs=self.root/'stubs';stubs.mkdir();started=self.root/'probe-started'
        wc=stubs/'wc'
        wc.write_text('#!/bin/sh\n: > "$C03_PROBE_STARTED"\nexec /bin/sleep 30\n');wc.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,
                 SOURCE='startup',CYS_ROLE='master',PATH=str(stubs)+':/usr/bin:/bin',
                 CYS_ROOT=str(self.root),C03_PROBE_STARTED=str(started))
        # Force expiration only once the real probe has started, after the
        # canonical checkpoint. No whole-hook startup time is being asserted.
        clock=owned.monotonic
        with patch.object(owned,'monotonic',lambda:clock()+(9 if started.exists() else 0)):
            result=owned.run_owned([PYTHON,'-B',str(BIN/'_owned_process.py'),'--command',
                '/bin/bash',str(pack/'hooks'/'inject-context.sh'),'--phase','core'],8,force=1,env=env)
        self.assertTrue(started.exists(),'controlled probe was never entered')
        self.assertTrue(result['timed_out'],result)
        self.assertFalse(result['complete'],result)
        self.assertIn('SOUL\0%s\0ROOT\0%s\0'%(soul,self.root),result['stdout'])

    @unittest.skipUnless(os.name == 'posix', 'controller checkpoint replay')
    def test_controller_replays_committed_core_records(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        shutil.copyfile(hooks/'inject-context.sh',pack/'hooks'/'inject-context.sh')
        # Test the unchanged controller with deterministic phase records, apart
        # from the real-core ownership test. A partial pair must not be replayed.
        (pack/'bin'/'_owned_process.py').touch()
        (pack/'hooks'/'_lib.sh').write_text('''cys_resolve_py() { CYS_PY=true; }
cys_timeout_run() {
  for phase; do :; done
  case "$phase" in
    ingress) printf 'SOURCE\\0startup\\0' ;;
    core) printf 'SOUL\\0/canonical/soul.md\\0_OUT_BAD\\0ignored\\0_OWNED_FAILURE\\0forced timeout\\0PARTIAL' ;;
  esac
  [ "$phase" = core ] || printf '_OWNED_RETURNED\\0001\\0'
}
''')
        result=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],  # wallclock: guard 8 s only ends a hang of the hook; the verdict is the output content
            env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack)),capture_output=True,text=True,timeout=8)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('phase core incomplete',result.stdout)
        self.assertIn('soul=/canonical/soul.md',result.stdout)
        self.assertNotIn('PARTIAL',result.stdout)

    @unittest.skipUnless(sys.platform == 'darwin', 'Darwin zombie-group EPERM')
    def test_eof_before_parent_kill_resolves_only_confirmed_absence(self):
        import errno
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        original=owned.os.killpg;refusals=[];signals=[]
        def after_eof(pid,sig):
            signals.append(sig)
            if sig:
                # The owner has already closed stdin. Keep its Popen unreaped
                # while the real child's EOF watcher kills its own group.
                end=time.monotonic()+1
                while time.monotonic()<end:
                    try:os.getpgid(pid)
                    except ProcessLookupError:break
                    time.sleep(.001)
                else:self.fail('child did not exit at the EOF barrier')
            try:return original(pid,sig)
            except PermissionError as exc:
                refusals.append(exc.errno);raise
        script=self.root/'exit.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:0)\n"%str(BIN))
        with patch.object(owned.os,'killpg',after_eof):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=.3)
        self.assertIn(errno.EPERM,refusals,'kernel EPERM path was not exercised')
        self.assertEqual(signals[0],signal.SIGKILL)
        self.assertTrue(all(sig==0 for sig in signals[1:]),signals)
        self.assertTrue(result['complete'],result)
        self.assertEqual(result['cleanup'],'verified_empty')

    @unittest.skipUnless(os.name == 'posix', 'EPERM negative classifications')
    def test_eperm_never_clears_other_uncertainty(self):
        import errno,threading
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        for mode in ('persistent-group','observation-permission','cancelled','missing-done','drain-held'):
            with self.subTest(mode=mode):
                original=owned.os.killpg;cancel=threading.Event()
                def refuse(pid,sig):
                    if sig:
                        original(pid,sig)
                        if mode=='cancelled':cancel.set()
                        raise PermissionError(errno.EPERM,'fixture teardown refusal')
                    if mode=='persistent-group':return None
                    if mode=='observation-permission':raise PermissionError(errno.EPERM,'fixture observation refusal')
                    return original(pid,sig)
                body='os._exit(0)' if mode=='missing-done' else '0'
                script=self.root/'negative.py'
                script.write_text("import os,sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:%s)\n"%(str(BIN),body))
                reader=owned._reader
                def hold_eof(stream,events,label,chunks):
                    if mode=='drain-held' and label=='err':return
                    return reader(stream,events,label,chunks)
                with patch.object(owned.os,'killpg',refuse),patch.object(owned,'_reader',hold_eof):
                    result=owned.run_owned([PYTHON,'-B',str(script)],2,force=.2,cancel=cancel)
                self.assertFalse(result['complete'],result)
                if mode in ('persistent-group','observation-permission','drain-held'):
                    self.assertEqual(result['cleanup'],'unknown',result)

    @unittest.skipUnless(sys.platform == 'darwin', 'native macOS signal permission denial')
    def test_real_signal_permission_denial_stays_incomplete(self):
        sandbox=Path('/usr/bin/sandbox-exec')
        if not sandbox.exists():self.skipTest('sandbox-exec unavailable: real permission UNMEASURED')
        script=self.root/'denied.py';marker=self.root/'denied-child'
        script.write_text("""import os,sys,json
from pathlib import Path
sys.path.insert(0,%r)
import _owned_process as owned
real=owned.subprocess.Popen
def record(*a,**kw):
 p=real(*a,**kw)
 marker=Path(%r);tmp=marker.with_suffix('.tmp');tmp.write_text(str(p.pid));os.replace(tmp,marker)
 return p
owned.subprocess.Popen=record
print(json.dumps({k:v for k,v in owned.run_owned([sys.executable,'-B',%r],2,force=.3).items() if not isinstance(v,bytes)}),flush=True)
"""%(str(BIN),str(marker),str(self.root/'denied-target.py')))
        (self.root/'denied-target.py').write_text("import sys,threading\nsys.path.insert(0,%r)\nimport _owned_process as owned\nowned._self_stop=lambda group:threading.Event().wait()\nowned.managed_main(lambda:0)\n"%str(BIN))
        p=subprocess.Popen([str(sandbox),'-p','(version 1)(allow default)(deny signal)',PYTHON,'-B',str(script)],
                           stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        self.children.append(p)
        pid=self.marker('denied-child')
        try:
            out,err=p.communicate(timeout=8)  # wallclock: guard 8 s only ends a hang; the verdict is the JSON result (complete False, cleanup unknown)
            self.assertEqual(p.returncode,0,err)
            result=json.loads(out)
            self.assertFalse(result['complete'],result)
            self.assertEqual(result['cleanup'],'unknown',result)
            self.assertIn('Operation not permitted',result['reason'])
            self.assertTrue(self.recorded_alive(pid),'permission denial did not leave a live child')
        finally:
            # Outside the process-scoped sandbox: only this recorded fixture.
            try:self.stop_recorded(pid)
            except ProcessLookupError:pass

    @unittest.skipUnless(os.name == 'posix', 'bash logging fixture')
    def test_blocked_logging_cannot_hide_restoration(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py'):
            shutil.copyfile(hooks/name,pack/'hooks'/name)
        text=(hooks/'inject-context.sh').read_text().replace('bookkeeping) _C03_CAP=10','bookkeeping) _C03_CAP=1')
        (pack/'hooks'/'inject-context.sh').write_text(text)
        shutil.copyfile(BIN/'_owned_process.py',pack/'bin'/'_owned_process.py')
        state=self.root/'_round';state.mkdir(exist_ok=True)
        (state/'SESSION_STATE.md').write_text('RETAINED-CORE-BEFORE-LOG\n')
        os.mkfifo(state/'.state_log')
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH='/usr/bin:/bin',CYS_ROOT=str(self.root))
        p=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],  # wallclock: guard 8 s only ends a hang of the hook; the verdict is the output content
            input=json.dumps({'source':'clear','cwd':str(self.root)}),text=True,
            capture_output=True,env=env,timeout=8)
        self.assertEqual(p.returncode,0,p.stderr)
        self.assertIn('RETAINED-CORE-BEFORE-LOG',p.stdout)
        self.assertIn('bookkeeping unmeasured',p.stdout)

    def test_communicate_stdin_control(self):
        p = subprocess.Popen([PYTHON, '-B', '-c', 'import sys;sys.stdin.buffer.read();print("EOF")'],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.children.append(p)
        out, _ = p.communicate(timeout=3)  # wallclock: guard 3 s only ends a hang after stdin was closed; the verdict is the EOF output
        self.assertEqual(out.strip(), b'EOF')
        p = subprocess.Popen([PYTHON, '-B', '-c', 'import sys;sys.stdin.buffer.read();print("EOF")'],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.children.append(p)
        writer = p.stdin; p.stdin = None
        try:
            with self.assertRaises(subprocess.TimeoutExpired):
                p.communicate(timeout=.1)
        finally:
            writer.close()
        out, _ = p.communicate(timeout=3)
        self.assertEqual(out.strip(), b'EOF')


    def test_cancel_after_ready_does_not_send_go(self):
        import queue,threading
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        cancel=threading.Event();observed=[];original=queue.Queue
        class ReadyQueue(original):
            def get(self,*args,**kwargs):
                item=super().get(*args,**kwargs)
                if item[0]=='control' and b'READY' in item[1]:
                    observed.append(True);cancel.set()
                return item
        marker=self.root/'forbidden-work';script=self.root/'startup.py'
        script.write_text("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n Path(%r).touch()\n return 0\nmanaged_main(main)\n" % (str(BIN),str(marker)))
        with patch.object(owned.queue,'Queue',ReadyQueue):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=1,cancel=cancel)
        self.assertTrue(observed,'READY was not reached')
        self.assertFalse(marker.exists(),'work started without GO admission')
        self.assertFalse(result['complete'],result)

    @unittest.skipUnless(os.name == 'posix', 'Popen return identity race')
    def test_parent_dies_during_popen_return_before_go(self):
        child=self.root/'startup_child.py'
        child.write_text("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n Path(%r).touch()\n return 0\nmanaged_main(main)\n" % (str(BIN),str(self.root/'forbidden-work')))
        parent=self.start("import os,sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\noriginal=owned.subprocess.Popen\ndef die(*a,**kw):\n p=original(*a,**kw)\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(p.pid));__import__('os').replace(_tmp,_marker)\n os._exit(9)\nowned.subprocess.Popen=die\nowned.run_owned([sys.executable,'-B',%r],2)\n" % (str(BIN),str(self.root/'startup-child'),str(child)))
        pid=self.marker('startup-child');parent.wait(timeout=3)  # wallclock: guard 3 s only ends a hang of the parent; the verdict is assert_gone and no forbidden work
        self.assert_gone(pid)
        self.assertFalse((self.root/'forbidden-work').exists())

    def test_duplicate_done_is_not_completion(self):
        sys.path.insert(0,str(BIN));import _owned_process as owned
        script=self.root/'duplicate.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nimport _owned_process as o\ndef main():\n o._CONTEXT.control.write(b'DONE 0\\nDONE 0\\n')\n return 0\no.managed_main(main)\n" % str(BIN))
        result=owned.run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertFalse(result['complete'],result)
        self.assertIn('control',result['reason'])

    def test_deadline_latch_wins_over_received_done(self):
        import queue
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        expired=[];clock=owned.monotonic;original=queue.Queue
        class DoneQueue(original):
            def get(self,*args,**kwargs):
                item=super().get(*args,**kwargs)
                if item[0]=='control' and b'DONE' in item[1]:expired.append(True)
                return item
        script=self.root/'done.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:0)\n" % str(BIN))
        with patch.object(owned.queue,'Queue',DoneQueue),patch.object(owned,'monotonic',lambda:clock()+(5 if expired else 0)):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertTrue(expired,'DONE was not observed')
        self.assertTrue(result['timed_out'],result)
        self.assertFalse(result['complete'],result)

    def test_c60_cancel_between_stages_stops_next_launch(self):
        import threading
        from unittest.mock import patch
        pf=load('javis_preflight');cancel=threading.Event();pf._CANCEL=cancel
        called=[]
        def memory_probe(*args,**kwargs):
            called.append('memory');cancel.set()
            return subprocess.CompletedProcess(args[0],0,b'',b'')
        with patch.object(pf.subprocess,'run',memory_probe),patch.object(pf.Preflight,'_run_skillscan_all') as scanner:
            result=pf.Preflight(fix=False,skips=[])._run_check_isolated(pf.Preflight(fix=False,skips=[]).c60_gate_wiring)
        self.assertEqual(called,['memory'])
        scanner.assert_not_called()
        self.assertTrue(result and result[-1]['unmeasured'])

    def test_cancelled_pool_never_enters_serial_fallback(self):
        import threading,concurrent.futures
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));scanner=load('javis_skillscan')
        cancel=threading.Event();scanner._CANCEL=cancel
        skill=self.root/'skills'/'sample';skill.mkdir(parents=True);(skill/'SKILL.md').write_text('# fixture')
        calls=[]
        def fail_pool(*args,**kwargs):
            cancel.set();raise RuntimeError('fixture pool cancellation')
        with patch.object(concurrent.futures,'ProcessPoolExecutor',fail_pool),patch.object(scanner,'scan_skill',lambda path:calls.append(path)):
            rc=scanner.cmd_all([str(skill.parent)],True)
        self.assertEqual(rc,125)
        self.assertEqual(calls,[],'cancelled scanner started a serial rescan')

    @unittest.skipUnless(os.name == 'nt', 'Windows inheritance: UNMEASURED on this host')
    def test_windows_unrelated_spawn_cannot_keep_parent_job_alive(self):
        child=self.root/'job_child.py'
        child.write_text("import sys,time\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:time.sleep(30))\n" % str(BIN))
        sibling_code="import time;time.sleep(30)"
        parent=self.start("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\nreal=owned.subprocess.Popen\ndef spawn(*args,**kw):\n child=real(*args,**kw)\n sibling=real([sys.executable,'-B','-c',%r],close_fds=True,stdin=owned.subprocess.DEVNULL,stdout=owned.subprocess.DEVNULL,stderr=owned.subprocess.DEVNULL)\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(child.pid));__import__('os').replace(_tmp,_marker)\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(sibling.pid));__import__('os').replace(_tmp,_marker)\n return child\nowned.subprocess.Popen=spawn\nowned.run_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),sibling_code,str(self.root/'owned-child'),str(self.root/'unrelated'),str(child)))
        child_pid=self.marker('owned-child');sibling_pid=self.marker('unrelated')
        parent.kill();parent.wait(timeout=60)  # wallclock: guard 60 s only ends a hang reaping the killed parent; the verdict is assert_gone and the sibling alive
        self.assert_gone(child_pid)
        self.assertTrue(self.recorded_alive(sibling_pid),'outside sibling was put in the owned Job')

    def test_scanner_death_alone_reaps_spawn_worker(self):
        skill = self.root / 'skills' / 'sample'; skill.mkdir(parents=True)
        (skill / 'SKILL.md').write_text('# fixture')
        scan = self.root / 'scan_pool.py'
        scan.write_text("""import os,sys,time,multiprocessing
from pathlib import Path
sys.path.insert(0,%r)
import javis_skillscan as scanner
try:
 from _owned_process import managed_main
except ImportError:
 managed_main=lambda main:main()
def slow_scan(path):
 _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)
 time.sleep(30)
 return {}
def main():
 multiprocessing.set_start_method('spawn',force=True)
 scanner.scan_skill=slow_scan
 _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)
 return scanner.cmd_all([%r],True)
if __name__=='__main__':
 managed_main(main)
""" % (str(BIN), str(self.root/'worker'), str(self.root/'scanner'), str(skill.parent)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        scanner = self.marker('scanner'); worker = self.marker('worker')
        self.assertNotEqual(scanner,worker,'pool fixture silently fell back to serial')
        self.stop_recorded(scanner)
        p.wait(timeout=5)  # wallclock: guard 5 s only ends a hang; the verdict is assert_gone(worker)
        self.assert_gone(worker)

    def test_job_close_failure_retains_handle_for_cleanup(self):
        import types
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        job=object.__new__(owned.Job);job.handle=123
        job.api=types.SimpleNamespace(CloseHandle=lambda handle:0)
        with patch.object(owned.ctypes,'WinError',lambda code:OSError(code,'fixture close refused'),create=True),patch.object(owned.ctypes,'get_last_error',lambda:5,create=True):
            with self.assertRaises(OSError):job.close()
        self.assertEqual(job.handle,123,'failed CloseHandle discarded the owned handle')

    @unittest.skipUnless(os.name == 'posix', 'POSIX C60 late-spawn barrier')
    def test_c60_real_late_spawn_after_fence_is_contained(self):
        script=self.root/'late_c60.py'
        child_code="import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker);time.sleep(30)" % str(self.root/'late-c60')
        script.write_text("import os,sys,time,subprocess\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\nimport javis_preflight as pf\nstop=owned._self_stop\ndef paused_stop(group):\n while not Path(%r).exists():time.sleep(.005)\n stop(group)\nowned._self_stop=paused_stop\ndef late_probe(*args,**kwargs):\n owned.CANCEL.set()\n child=subprocess.Popen([sys.executable,'-B','-c',%r])\n child.wait()\ndef main():\n pf.subprocess.run=late_probe\n checker=pf.Preflight(fix=False,skips=[])\n checker._run_check_isolated(checker.c60_gate_wiring)\n return 0\nowned.managed_main(main)\n" % (str(BIN),str(self.root/'late-c60'),child_code))
        sibling=self.start('import time;time.sleep(30)')
        parent=self.start("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],3,force=1)\n" % (str(BIN),str(script)))
        child=self.marker('late-c60');parent.wait(timeout=5)  # wallclock: guard 5 s only ends a hang of the parent; the verdict is assert_gone and the sibling alive
        self.assert_gone(child)
        self.assertIsNone(sibling.poll())

    def test_checklist_deadline_cleans_scanner_and_spawn_pool(self):
        skill=self.root/'skills'/'sample';skill.mkdir(parents=True);(skill/'SKILL.md').write_text('# fixture')
        scanner=self.root/'scan_deadline.py'
        scanner.write_text("import os,sys,time,multiprocessing\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport javis_skillscan as scanner\nfrom _owned_process import managed_main\ndef slow_scan(path):\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n time.sleep(3600)\n return {}\ndef main():\n multiprocessing.set_start_method('spawn',force=True)\n scanner.scan_skill=slow_scan\n _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n return scanner.cmd_all([%r],True)\nif __name__=='__main__':managed_main(main)\n" % (str(BIN),str(self.root/'pool-worker'),str(self.root/'deadline-scanner'),str(skill.parent)))
        preflight=self.root/'preflight_deadline.py'
        preflight.write_text("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nfrom _owned_process import managed_main\ndef main():\n Preflight._run_skillscan_all(%r,3600)\n return 0\nmanaged_main(main)\n" % (str(BIN),str(scanner)))
        # The scan and the inner skillscan deadline outlive the 60 s guard: only the
        # checklist's own 2 s deadline can end this run inside it.
        parent=self.start("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport javis_checklist as ck\nck.DEFAULT_PREFLIGHT=[sys.executable,%r]\nresult=ck.run_preflight(ck.DEFAULT_PREFLIGHT,timeout=2)\nPath(%r).write_text(str(result))\n" % (str(BIN),str(preflight),str(self.root/'deadline-result')))
        scan_pid=self.marker('deadline-scanner');worker=self.marker('pool-worker')
        self.assertNotEqual(scan_pid,worker,'pool fixture silently fell back to serial')
        parent.wait(timeout=60)  # wallclock: guard 60 s only ends a hang of the parent; the verdict is 'timeout' in the result and assert_gone
        self.assertIn('timeout',(self.root/'deadline-result').read_text())
        self.assert_gone(scan_pid);self.assert_gone(worker)

    def test_custom_command_preserves_callers_text_encoding(self):
        import locale
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import javis_checklist as ck
        command=[PYTHON,'-B','-c',"import os;os.write(1,'사용자 결과'.encode('cp949'))"]
        with patch.object(locale,'getpreferredencoding',return_value='cp949'):
            code,last=ck.run_preflight(command,timeout=2)
        self.assertEqual((code,last),(0,'사용자 결과'))


    @unittest.skipUnless(os.name == 'posix', 'bash prelude deadline')
    def test_locale_initialization_is_inside_ingress_budget(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py'):shutil.copyfile(hooks/name,pack/'hooks'/name)
        original=(hooks/'inject-context.sh').read_text()
        # A silent no-op replace would leave the 20 s cap, which the 60 s guard below cannot tell apart.
        self.assertIn('ingress) _C03_CAP=20',original)
        text=original.replace('ingress) _C03_CAP=20','ingress) _C03_CAP=1')
        (pack/'hooks'/'inject-context.sh').write_text(text)
        shutil.copyfile(BIN/'_owned_process.py',pack/'bin'/'_owned_process.py')
        stubs=self.root/'stubs';stubs.mkdir()
        # The locale lookup stalls for an hour, so only the 1 s ingress budget can end the hook.
        locale=stubs/'locale';locale.write_text('#!/bin/sh\n: > "$C03_LOCALE_MARK"\nsleep 3600\nprintf C.UTF-8\\n\n');locale.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH=str(stubs)+':/usr/bin:/bin',CYS_ROOT=str(self.root),LANG='C',LC_CTYPE='C',
                 PYTHONCOERCECLOCALE='0',PYTHONUTF8='0',C03_LOCALE_MARK=str(self.root/'locale-called'))
        env.pop('LC_ALL',None)
        p=subprocess.Popen(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,start_new_session=True)
        self.children.append(p)
        try:
            try:out,err=p.communicate(json.dumps({'source':'clear','cwd':str(self.root)}).encode(),timeout=60)  # wallclock: guard 60 s only ends a hang; the verdict is that the hook ended (rc 0, 'restoration input incomplete:')
            except subprocess.TimeoutExpired:self.fail('locale startup bypassed ingress budget')
            self.assertEqual(p.returncode,0,err)
            self.assertTrue((self.root/'locale-called').exists(),'fixture did not exercise locale lookup')
            self.assertIn(b'restoration input incomplete:',out)
        finally:
            if p.returncode is None:
                os.killpg(p.pid,signal.SIGKILL);p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang reaping the SIGKILLed hook group; cleanup only, not the verdict

    def test_hook_dir_derivation_splits_on_both_separators(self):
        # F-B1 (string level): the directory the hook sources _lib.sh from, computed by the hook's own prelude text
        # with BASH_SOURCE[0] replaced by each vector. The Windows shapes are plain strings here. One bash process
        # reads that text from a file (on nt, command-line quoting of a -c script would decide the result) and
        # prints one line per vector. On nt only the bundled Git bash counts (which('bash') may be WSL's).
        import shutil
        bash = os.environ.get('C03_TEST_BASH') or (shutil.which('bash') if os.name == 'posix' else None)
        if not bash:
            self.skipTest('no bash (on nt: C03_TEST_BASH, the bundled Git bash)')
        lines = (Path(__file__).resolve().parents[2]/'hooks'/'inject-context.sh').read_text(encoding='utf-8').splitlines()
        src = [i for i, l in enumerate(lines) if l.startswith('. "') and l.endswith('/_lib.sh" 2>/dev/null \\')]
        self.assertTrue(src, 'the first _lib.sh source line was not found')
        i = src[0]
        expr = lines[i][3:lines[i].index('/_lib.sh"')]
        start = next((j for j in range(i) if lines[j].startswith('_C03_HOOK_DIR=')), i)
        body = '\n'.join(['for _v in "$@"; do', 'set -- "$_v"'] + lines[start:i] + ['printf \'%s\\n\' "' + expr + '"', 'done', ''])
        script = self.root/'derive.sh'
        script.write_bytes(body.replace('${BASH_SOURCE[0]', '${1').encode('utf-8'))
        vectors = [
            ('D:\\a\\cys\\cys\\cysjavis-pack\\hooks\\inject-context.sh', 'D:\\a\\cys\\cys\\cysjavis-pack\\hooks'),
            ('/srv/cys/pack/hooks/inject-context.sh', '/srv/cys/pack/hooks'),
            ('D:\\a\\cys/cysjavis-pack\\hooks/inject-context.sh', 'D:\\a\\cys/cysjavis-pack\\hooks'),
            ('inject-context.sh', '.'),
            ('/inject-context.sh', '/'),
            ('./inject-context.sh', '.'),
            ('/Applications/cys.app/Contents/Resources/pack/hooks/inject-context.sh', '/Applications/cys.app/Contents/Resources/pack/hooks'),
            ('C:\\Program Files\\cys\\cysjavis-pack\\hooks\\inject-context.sh', 'C:\\Program Files\\cys\\cysjavis-pack\\hooks'),
            ('C:\\inject-context.sh', 'C:'),
        ]
        r = subprocess.run([bash, str(script)] + [given for given, _ in vectors], capture_output=True, text=True, timeout=10)
        got = r.stdout.split('\n')[:-1]
        self.assertEqual(len(got), len(vectors), 'one line per vector expected: %r %r' % (r.stdout, r.stderr))
        wrong = [(given, out, want) for (given, want), out in zip(vectors, got) if out != want]
        self.assertEqual(wrong, [], 'hook directory derived wrongly (given, got, want)')

    @unittest.skipUnless(os.name == 'posix', 'bash hook started by its bare name')
    def test_hook_started_by_bare_name_finds_its_lib(self):
        # F-B2: 'bash inject-context.sh' from inside the hooks directory, with no CYS_PACK_DIR fallback.
        import shutil
        hooks = Path(__file__).resolve().parents[2]/'hooks'
        pack = self.root/'pack'; (pack/'hooks').mkdir(parents=True); (pack/'bin').mkdir()
        for name in ('_lib.sh', 'inject_gate.py', 'inject-context.sh'):
            shutil.copyfile(hooks/name, pack/'hooks'/name)
        shutil.copyfile(BIN/'_owned_process.py', pack/'bin'/'_owned_process.py')
        home = self.root/'home'; home.mkdir()
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TMPDIR': str(home), 'CYS_PY': PYTHON,
               'CYS_ROLE': 'master', 'CYS_ROOT': str(self.root)}
        p = subprocess.run(['/bin/bash', 'inject-context.sh'], cwd=str(pack/'hooks'), env=env, capture_output=True, text=True,  # wallclock: guard 60 s only ends a hang of the hook; the verdict is the absent degrade line
                           input=json.dumps({'source': 'clear', 'cwd': str(self.root)}), timeout=60)
        self.assertNotIn('_lib.sh 소실', p.stderr)

    def run_backslash_hook(self):
        """The Windows path shape on POSIX: a FILE named 'C:\\x\\hooks\\inject-context.sh' beside a DIRECTORY named
        'C:\\x\\hooks' (backslashes are ordinary name characters here). bash gets the Windows-form $0, and only a split
        on '\\' finds the hook's directory. The gate there is a stub that records it ran."""
        import shutil
        hooks = Path(__file__).resolve().parents[2]/'hooks'
        d = self.root/'C:\\x\\hooks'; d.mkdir()
        shutil.copyfile(hooks/'_lib.sh', d/'_lib.sh')
        (d/'inject_gate.py').write_text("import os,sys,pathlib\npathlib.Path(os.environ['C03_GATE_MARK']).touch()\nsys.stdout.write(sys.stdin.read())\n")
        shutil.copyfile(hooks/'inject-context.sh', self.root/'C:\\x\\hooks\\inject-context.sh')
        (self.root/'bin').mkdir(); shutil.copyfile(BIN/'_owned_process.py', self.root/'bin'/'_owned_process.py')
        home = self.root/'home'; home.mkdir()
        mark = self.root/'gate-called'
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TMPDIR': str(home), 'CYS_PY': PYTHON,
               'CYS_ROLE': 'master', 'CYS_ROOT': str(self.root), 'C03_GATE_MARK': str(mark)}
        p = subprocess.run(['/bin/bash', 'C:\\x\\hooks\\inject-context.sh'], cwd=str(self.root), env=env, capture_output=True, text=True,  # wallclock: guard 60 s only ends a hang of the hook; the verdict is the output and the gate record
                           input=json.dumps({'source': 'clear', 'cwd': str(self.root)}), timeout=60)
        return p, mark

    @unittest.skipUnless(os.name == 'posix', 'backslashes are ordinary file name characters only on POSIX')
    def test_hook_with_windows_path_finds_its_lib(self):
        # F-B3: the H-WIN-2/3/5 shape end to end: the hook must find _lib.sh and inject the state.
        p, mark = self.run_backslash_hook()
        self.assertNotIn('_lib.sh 소실', p.stderr)
        self.assertIn('C03-ISOLATED-STATE', p.stdout, p.stderr[-400:])

    @unittest.skipUnless(os.name == 'posix', 'backslashes are ordinary file name characters only on POSIX')
    def test_phase_with_windows_path_reaches_the_poison_gate(self):
        # m1: a --phase re-entry gets the same Windows-form $0; the poison gate must be found next to the hook,
        # not next to the working directory (dirname of a backslash path is '.').
        p, mark = self.run_backslash_hook()
        self.assertIn('C03-ISOLATED-STATE', p.stdout, p.stderr[-400:])
        self.assertTrue(mark.exists(), 'the --phase body looked for inject_gate.py beside the cwd, not beside the hook')

    def crlf_env(self):
        """H-WIN-17 specimens: an env whose text-mode Python stdio writes LF as CR LF (Windows' premise). POSIX gets
        the _CRLF_SITECUSTOMIZE emulation plus its argv log; on nt the premise is native and an embeddable runtime
        ignores PYTHONPATH, so nothing is added there."""
        if os.name == 'posix':
            home = self.root/'home'; home.mkdir(exist_ok=True)
            emu = self.root/'crlf-emu'; emu.mkdir(exist_ok=True)
            (emu/'sitecustomize.py').write_text(_CRLF_SITECUSTOMIZE)
            return {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TMPDIR': str(home), 'PYTHONUTF8': '0',
                    'PYTHONPATH': str(emu), 'C03_EMU_LOG': str(self.root/'crlf-emu.log')}
        return dict(os.environ, PYTHONUTF8='0')

    def assert_crlf_premise(self, env, *leaf_args):
        # Prerequisite, before any verdict: the premise is live, and on POSIX it reached the process under test (a
        # process whose argv carries every leaf_args), not only a sibling - a green must not mean "emulation lost".
        sib = subprocess.run([PYTHON, '-B', '-c', "print('a')"], capture_output=True, env=env, timeout=10)  # wallclock: guard 10 s only ends a hang; the verdict is the printed bytes
        self.assertEqual(sib.stdout, b'a\r\n', 'premise not live: a text-mode print wrote %r' % sib.stdout)
        if os.name == 'posix':
            log = self.root/'crlf-emu.log'
            seen = log.read_text(encoding='utf-8').splitlines() if log.exists() else []
            self.assertTrue([l for l in seen if all(a in l for a in leaf_args)],
                            'the CRLF emulation never reached a process with %r: %r' % (leaf_args, seen[-6:]))

    def run_leaf(self, script, records):
        """The leaf CLI relaying a bash child that writes LF and a 0xFF byte; returns (rc, stdout, stderr) as bytes."""
        import shutil
        bash = os.environ.get('C03_TEST_BASH') or shutil.which('bash')
        if not bash:
            self.skipTest('no bash (on nt: C03_TEST_BASH, the bundled Git bash)')
        env = self.crlf_env()
        cmd = [PYTHON, '-B', str(BIN/'_owned_process.py'), '--bounded', '10'] + (['--records'] if records else [])
        p = subprocess.run(cmd + ['--', bash, '-c', script], capture_output=True, env=env, timeout=30)  # wallclock: guard 30 s only ends a hang (the leaf's own cap is 10 s); the verdict is the relayed bytes
        self.assert_crlf_premise(env, '--bounded', '10')
        return p.returncode, p.stdout, p.stderr

    def test_leaf_records_complete_relay_is_byte_exact(self):
        # H-WIN-17 S2 (:583): a complete phase's records leave the leaf as the child wrote them.
        rc, out, err = self.run_leaf(r"printf 'OUT\0a\nb\377\0'", True)
        self.assertEqual(rc, 0, err[-400:])
        self.assertNotIn(b'\r', out, 'complete records relay (:583) added CR: %r' % out[:40])
        self.assertTrue(out.startswith(b'OUT\x00a\nb\xff\x00'), 'complete records relay (:583) changed bytes: %r' % out[:40])

    def test_leaf_records_partial_relay_is_byte_exact(self):
        # H-WIN-17 S2 (:589): an incomplete phase keeps its committed pairs byte for byte.
        rc, out, err = self.run_leaf(r"printf 'OUT\0a\nb\377\0'; exit 3", True)
        self.assertIn(b'_OWNED_FAILURE\x00', out, 'not the partial-record path: %r' % out[:80])
        self.assertNotIn(b'\r', out, 'partial records relay (:589) added CR: %r' % out[:40])
        self.assertTrue(out.startswith(b'OUT\x00a\nb\xff\x00_OWNED_FAILURE\x00'), 'partial records relay (:589) changed bytes: %r' % out[:40])

    def test_leaf_plain_relay_is_byte_exact(self):
        # H-WIN-17 S2 (:594): the non-records relay (inject-context :497 CHK_OUT) passes stdout through unchanged.
        rc, out, err = self.run_leaf(r"printf 'OUT\0a\nb\377\0'", False)
        self.assertEqual(rc, 0, err[-400:])
        self.assertNotIn(b'\r', out, 'plain relay (:594) added CR: %r' % out)
        self.assertEqual(out, b'OUT\x00a\nb\xff\x00', 'plain relay (:594) changed bytes')

    def test_leaf_stderr_relay_is_byte_exact(self):
        # H-WIN-17 S2 (:595): the child's stderr leaves the leaf unchanged.
        rc, out, err = self.run_leaf(r"printf 'e1\ne2\377' >&2", False)
        self.assertEqual(rc, 0, err[-400:])
        self.assertNotIn(b'\r', err, 'stderr relay (:595) added CR: %r' % err)
        self.assertEqual(err, b'e1\ne2\xff', 'stderr relay (:595) changed bytes')

    def test_soul_headings_reach_the_hook_output_without_cr(self):
        # H-WIN-17 S1: the real hook, _lib.sh and leaf under the CRLF premise. The soul's real LF between headings
        # and the checklist's own lines (:497) pass every phase with no CR added. The checklist's own CR LF (a text
        # tool on the premise) is the in-path witness and must still arrive exactly once.
        import shutil
        bash = os.environ.get('C03_TEST_BASH') or shutil.which('bash')
        if not bash:
            self.skipTest('no bash (on nt: C03_TEST_BASH, the bundled Git bash)')
        hooks = Path(__file__).resolve().parents[2]/'hooks'
        pack = self.root/'pack'; (pack/'hooks').mkdir(parents=True); (pack/'bin').mkdir()
        for name in ('_lib.sh', 'inject_gate.py', 'inject-context.sh'):
            shutil.copyfile(hooks/name, pack/'hooks'/name)
        for name in ('_owned_process.py', 'javis_checklist.py'):
            shutil.copyfile(BIN/name, pack/'bin'/name)
        (pack/'bin'/'javis_preflight.py').write_text("import os,sys\nsys.path.insert(0,os.path.dirname(__file__))\nfrom _owned_process import managed_main\ndef main():\n print('fixture summary')\n return 0\nmanaged_main(main)\n")
        # No invalid UTF-8 byte here: the phase bash inherits LC_CTYPE=C.UTF-8 from the leaf's locale coercion and
        # BSD tr at inject-context.sh:288 cuts a heading at such a byte (pin bytes too) - that is the heading
        # pipeline, not the relay. The 0xFF byte rides through every relay site in the four leaf specimens above.
        heads = '## 금지선 하나\n## 오너 규칙 둘\n'.encode('utf-8')
        (pack/'soul.md').write_bytes(b'# s\n' + heads.replace(b'\n', b'\nx\n'))
        env = dict(self.crlf_env(), CYS_PY=PYTHON, CYS_ROLE='master', CYS_ROOT=str(self.root), CYS_PACK_DIR=str(pack))
        if os.name == 'nt':
            env['PATH'] = str(Path(bash).parent.parent/'usr'/'bin') + os.pathsep + env.get('PATH', '')
        p = subprocess.run([bash, str(pack/'hooks'/'inject-context.sh')], capture_output=True, env=env, cwd=str(self.root),  # wallclock: guard 60 s only ends a hang of the hook; the verdict is the output bytes
                           input=json.dumps({'source': 'startup', 'cwd': str(self.root)}).encode(), timeout=60)
        out = p.stdout
        self.assertEqual(p.returncode, 0, p.stderr[-400:])
        self.assert_crlf_premise(env, '--bounded', '--records')
        if os.name == 'posix':
            self.assert_crlf_premise(env, '--bounded', '--managed-command')
        self.assertIn(b'fixture summary\r', out, 'witness: the checklist line never arrived with its own CR: %r' % out[-300:])
        i = out.index(b'fixture summary\r') + len(b'fixture summary')
        self.assertEqual(out[i:i + 2], b'\r\n', 'checklist relay (:497/:594) added CR: %r' % out[i:i + 8])
        self.assertIn(heads, out, 'soul headings arrived changed (records relay :583): %r' % out[out.find(b'## '):][:120])
        self.assertNotIn(b'\r\r', out, 'a CR run in the hook output')

    @unittest.skipUnless(os.name == 'nt', 'Windows Git Bash hook: UNMEASURED locally')
    def test_windows_bundled_bash_hook_retains_core(self):
        import shutil
        bash=os.environ.get('C03_TEST_BASH') or shutil.which('bash')
        self.assertTrue(bash and Path(bash).is_file(),'packaged Bash fixture required')
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py','inject-context.sh'):shutil.copyfile(hooks/name,pack/'hooks'/name)
        for name in ('_owned_process.py','javis_checklist.py'):shutil.copyfile(BIN/name,pack/'bin'/name)
        (pack/'bin'/'javis_preflight.py').write_text("import os,sys\nsys.path.insert(0,os.path.dirname(__file__))\nfrom _owned_process import managed_main\ndef main():\n print('fixture summary')\n return 0\nmanaged_main(main)\n")
        state=self.root/'_round';state.mkdir(exist_ok=True);(state/'SESSION_STATE.md').write_text('C03-WINDOWS-CORE\n')
        runtime=Path(bash).parent.parent
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 CYS_ROOT=str(self.root),PYTHONUTF8='1',PATH=str(runtime/'usr/bin')+os.pathsep+os.environ.get('PATH',''))
        result=subprocess.run([bash,str(pack/'hooks'/'inject-context.sh')],
                  input=json.dumps({'source':'clear','cwd':str(self.root)}),text=True,encoding='utf-8',
                  capture_output=True,env=env,timeout=30)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('C03-WINDOWS-CORE',result.stdout)
        self.assertIn('fixture summary',result.stdout)
        self.assertNotIn('incomplete',result.stdout)

    @unittest.skipUnless(os.name == 'nt', 'Windows Job failure: UNMEASURED locally')
    def test_windows_failed_job_termination_is_bounded_and_degraded(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        script=self.root/'job_fail.py'
        script.write_text("import sys,time\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:time.sleep(30))\n" % str(BIN))
        with patch.object(owned.Job,'terminate',side_effect=OSError('fixture termination refused')):
            result=owned.run_owned([PYTHON,'-B',str(script)],.3,force=1)
        self.assertFalse(result['complete'],result)
        self.assertIn('termination refused',result['reason'])
        self.assertLess(result['elapsed'],2.5,result)


    def test_owned_launcher_has_no_unmanaged_escape(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        with patch.object(owned.subprocess,'Popen') as launch:
            with self.assertRaises(TypeError):
                owned.run_owned([PYTHON,'-B','-c','pass'],1,managed=False)
        launch.assert_not_called()

    @unittest.skipUnless(os.name == 'posix', 'post-phase checkpoint')
    def test_slow_rsi_keeps_completed_restore_mode_note(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py'):shutil.copyfile(hooks/name,pack/'hooks'/name)
        text=(hooks/'inject-context.sh').read_text().replace('post) _C03_CAP=30','post) _C03_CAP=1')
        (pack/'hooks'/'inject-context.sh').write_text(text)
        shutil.copyfile(BIN/'_owned_process.py',pack/'bin'/'_owned_process.py')
        (self.root/'_round'/'RSI_LEDGER.md').write_text('- [x] fixture\n')
        stubs=self.root/'stubs';stubs.mkdir();grep=stubs/'grep'
        grep.write_text('#!/bin/sh\ncase "$*" in *RSI_LEDGER.md*) : > "$C03_RSI_MARK"; sleep 10;; esac\nexec /usr/bin/grep "$@"\n');grep.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH=str(stubs)+':/usr/bin:/bin',CYS_ROOT=str(self.root),C03_RSI_MARK=str(self.root/'rsi-called'))
        result=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],  # wallclock: guard 8 s only ends a hang of the hook; the verdict is the output content and rsi-called
            input=json.dumps({'source':'resume','cwd':str(self.root)}),text=True,capture_output=True,env=env,timeout=8)
        self.assertTrue((self.root/'rsi-called').exists(),'RSI stall not reached')
        self.assertIn('C03-ISOLATED-STATE',result.stdout)
        self.assertIn('phase post incomplete',result.stdout)
        self.assertIn('source=resume',result.stdout,'completed mode note was discarded')

    @unittest.skipUnless(os.name == 'posix', 'shared ancillary budget receipt loss')
    def test_lost_role_receipt_cannot_restart_ancillary_allowance(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py','inject-context.sh'):shutil.copyfile(hooks/name,pack/'hooks'/name)
        leaf=(BIN/'_owned_process.py').read_text()
        leaf=leaf.replace('    args = ap.parse_args()',"    args = ap.parse_args()\n    if args.command and args.command[-1]=='role': os._exit(9)")
        (pack/'bin'/'_owned_process.py').write_text(leaf)
        stubs=self.root/'stubs';stubs.mkdir()
        for name,body in [('ps','#!/bin/sh\n: > "$C03_PS_MARK"\n'),('lsof','#!/bin/sh\nexit 0\n')]:
            path=stubs/name;path.write_text(body);path.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH=str(stubs)+':/usr/bin:/bin',CYS_ROOT=str(self.root),C03_PS_MARK=str(self.root/'ps-called'))
        result=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],  # wallclock: guard 8 s only ends a hang of the hook; the verdict is the output content and no ps-called
            input=json.dumps({'source':'clear','cwd':str(self.root)}),text=True,capture_output=True,env=env,timeout=8)
        self.assertIn('phase role incomplete',result.stdout)
        self.assertIn('C03-ISOLATED-STATE',result.stdout)
        self.assertFalse((self.root/'ps-called').exists(),'lost receipt restarted the shared30s allowance')

    def test_scaled_phase_model_consumes420_and_refuses_more(self):
        import io
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        clock=[0.0];calls=[];durations=iter([.2,.1,.6,.2,2.7,.3,.1])
        def execute(cmd,timeout,force,**kwargs):
            duration=next(durations);self.assertLessEqual(duration,timeout+force+1e-8)
            calls.append((timeout,force,duration));clock[0]+=duration
            return {'complete':True,'returncode':0,'stdout':'','stderr':'','reason':'','timed_out':False}
        role_spent=0;check_spent=0;origin=0
        for name,cap in [('ingress',.2),('role',.3),('core',.6),('ancillary',.3),('checklist',2.7),('post',.3),('bookkeeping',.1),('extra',.1)]:
            args=['owned','--bounded',str(cap),'--records','--start',str(origin),
                  '--exclude',str(check_spent),'--budget',str(4.2 if name=='checklist' else 1.5),'--total-budget','4.2']
            if name in ('role','ancillary'):args+=['--spent',str(role_spent)]
            args+=['--','fixture-command']
            # bounded_cli relays through .buffer (H-WIN-17), so the patched streams carry one.
            out=io.TextIOWrapper(io.BytesIO(),encoding='utf-8')
            with patch.object(sys,'argv',args),patch.object(sys,'stdout',out),patch.object(sys,'stderr',io.TextIOWrapper(io.BytesIO(),encoding='utf-8')),patch.object(owned,'monotonic',lambda:clock[0]),patch.object(owned,'run_owned',execute):
                rc=owned.bounded_cli()
            fields=out.buffer.getvalue().decode('utf-8').split('\0');record=dict(zip(fields[::2],fields[1::2]))
            if name in ('role','ancillary'):role_spent=float(record['_OWNED_SPENT'])
            if name=='checklist':check_spent=float(record['_OWNED_ELAPSED'])
            self.assertEqual(rc,124 if name=='extra' else 0,(name,record))
        self.assertAlmostEqual(clock[0],4.2)
        self.assertEqual(len(calls),7,'exhausted total budget started more work')
        self.assertAlmostEqual(role_spent,.3)


    def test_custom_command_flags_are_not_private_control(self):
        sys.path.insert(0,str(BIN));import javis_checklist as ck
        flags=['--bounded','--owned-lifeline','not-a-handle']
        command=[PYTHON,'-B','-c',"import sys;print(' '.join(sys.argv[1:]))"]+flags
        code,last=ck.run_preflight(command,timeout=2)
        self.assertEqual((code,last),(0,' '.join(flags)))

    def test_private_adapter_refuses_without_admission(self):
        marker=self.root/'uncontained'
        result=subprocess.run([PYTHON,'-B',str(BIN/'_owned_process.py'),'--command',  # wallclock: guard 3 s only ends a hang; the verdict is rc 125 and no marker
            PYTHON,'-B','-c',"from pathlib import Path;Path(%r).touch()" % str(marker)],
            capture_output=True,timeout=3)
        self.assertEqual(result.returncode,125)
        self.assertFalse(marker.exists())

    def test_forced_reap_does_not_borrow_cooperative_grace(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        waits=[];original=owned.subprocess.Popen.wait
        def record(proc,*args,**kwargs):
            waits.append(kwargs.get('timeout'));return original(proc,*args,**kwargs)
        script=self.root/'grace.py'
        script.write_text("import sys,time\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:time.sleep(30))\n" % str(BIN))
        with patch.object(owned.subprocess.Popen,'wait',record):
            result=owned.run_owned([PYTHON,'-B',str(script)],.2,grace=.4,force=.2)
        self.assertTrue(result['timed_out'],result)
        self.assertTrue(waits)
        self.assertTrue(all(t is not None and t<=.2 for t in waits),waits)

    @unittest.skipUnless(os.name == 'posix', 'POSIX bounded absence observation')
    def test_cleanup_observes_delayed_group_disappearance_within_budget(self):
        from unittest.mock import patch
        sys.path.insert(0,str(BIN));import _owned_process as owned
        observed=[];original=owned.os.killpg
        def delayed(pid,sig):
            if sig==0:
                observed.append(pid)
                if len(observed)<3:return None
            return original(pid,sig)
        script=self.root/'exit.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\nmanaged_main(lambda:0)\n" % str(BIN))
        with patch.object(owned.os,'killpg',delayed):
            result=owned.run_owned([PYTHON,'-B',str(script)],2,force=.3)
        self.assertTrue(result['complete'],result)
        self.assertGreaterEqual(len(observed),3)

    def test_managed_preflight_rejects_parsed_mutation_aliases(self):
        import contextlib,io,types
        from unittest.mock import patch
        pf=load('javis_preflight');pf._managed_active=lambda:True
        called=[]
        def checker(**kwargs):
            called.append(kwargs)
            return types.SimpleNamespace(run=lambda:[],planned=[])
        for flag in ('--fi','--fix','--dry','--safe','--allow-irreversible'):
            with self.subTest(flag=flag),patch.object(sys,'argv',['preflight',flag]),patch.object(pf,'Preflight',checker),contextlib.redirect_stdout(io.StringIO()),contextlib.redirect_stderr(io.StringIO()):
                rc=pf.main()
            self.assertEqual(rc,125,flag)
        self.assertEqual(called,[],'managed preflight reached a mutation/non-report constructor')
        for flag,entry in [('--seed-trust','_seed_trust_main'),('--self-test','_self_test')]:
            with patch.object(sys,'argv',['preflight',flag]),patch.object(pf,entry) as branch,contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(pf.main(),125)
                branch.assert_not_called()

    def test_ingress_owner_returns_timeout_without_reader_abort(self):
        p=subprocess.Popen([PYTHON,'-B',str(BIN/'_owned_process.py'),'--bounded','.4',
            '--input-env','INPUT','--records','--',PYTHON,'-c','pass'],
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        self.children.append(p)
        p.wait(timeout=3)  # wallclock: guard 3 s only ends a hang; the verdict is rc 124 and 'input incomplete'
        out=p.stdout.read();err=p.stderr.read()
        self.assertEqual(p.returncode,124,err)
        self.assertIn(b'input incomplete',out)
        self.assertNotIn(b'Fatal Python error',err)


# Fix A (design r1 15acd497 s2.4): after a timeout or a crash the owner must stop spending grace once the leader is
# gone (control-eof). The driver runs run_owned with grace 600 and force 5 and prints one RESULT line. With an arm
# path, the timeout starts only once that file exists (the output holder is up), as in the held-output tests.
# H-WIN-17: on Windows a text-mode Python stdout writes LF as CR LF, and the leaf relayed every phase's records that
# way, so each pass added one CR per LF inside OUT. On POSIX this sitecustomize (via PYTHONPATH) gives every Python
# in the env that premise and logs its argv, so a specimen can show the premise reached the leaf itself.
_CRLF_SITECUSTOMIZE = r'''
import os, sys
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(newline='\r\n')
    except Exception:
        pass
_log = os.environ.get('C03_EMU_LOG')
if _log:
    with open(_log, 'a', encoding='utf-8') as _f:
        _f.write(repr(sys.argv) + '\n')
'''


_FA_DRIVER = r'''
import json, os, subprocess, sys
from pathlib import Path
sys.path.insert(0, %(bin)r)
import _owned_process as owned
arm = %(arm)r
if arm:
    clock = owned.monotonic; origin = clock(); armed = []
    def ready_clock():
        now = clock()
        if not armed and Path(arm).exists():
            armed.append(now)
        return origin + (now - armed[0] if armed else 0)
    owned.monotonic = ready_clock; subprocess._time = ready_clock
r = owned.run_owned(%(launch)r, %(timeout)r, grace=600, force=5)
print('RESULT ' + json.dumps({k: r[k] for k in ('timed_out', 'complete', 'cleanup', 'reason')}), flush=True)
'''


# E(ii) (design r2 d4af60af): a pool that never becomes ready, or breaks while a worker outlives terminate(),
# must not hang the scan. One fixture script drives cmd_all (or the CLI entry) with a stand-in initializer and
# task per branch; workers record their pids so leftovers are counted, never guessed.
_E2_FIXTURE = r'''
import json, multiprocessing, os, signal, sys, time
sys.path.insert(0, %(bin)r)
import javis_skillscan as scanner
BR = os.environ['C03E2_BRANCH']; TMP = os.environ['C03E2_TMP']
def note():
    with open(os.path.join(TMP, 'pids.txt'), 'a') as f:
        f.write('%%d\n' %% os.getpid())
def init(*args):
    note()
    if BR in ('N', 'NI', 'SURV', 'SURVX'):
        if BR == 'NI':
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        time.sleep(3600)
    if BR == 'S':
        os._exit(3)
    if BR == 'X':
        try:
            fd = os.open(os.path.join(TMP, 'held'), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            return
        os.close(fd)
        time.sleep(3600)
def task(path):
    name = os.path.basename(path)
    if multiprocessing.current_process().name != 'MainProcess' and BR == 'B':
        if name == 'a-blocker':
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            open(os.path.join(TMP, 'blocker-armed'), 'w').close()
            time.sleep(3600)
        if name == 'b-dier':
            while not os.path.exists(os.path.join(TMP, 'blocker-armed')):
                time.sleep(0.01)
            os._exit(3)
    return {'skill': name, 'verdict': 'ACCEPT', 'severity': 'LOW', 'findings': []}
if __name__ == '__main__':
    multiprocessing.set_start_method('spawn', force=True)
    scanner.worker_observer = init
    scanner.scan_skill = task
    scanner.POOL_START_DEADLINE_S = float(os.environ['C03E2_D'])
    if BR in ('SURV', 'SURVX'):
        import multiprocessing.process as mpp
        mpp.BaseProcess.terminate = lambda self: None
        mpp.BaseProcess.kill = lambda self: None
        sys.argv = ['javis_skillscan.py', 'all', '--roots', os.environ['C03E2_ROOT'], '--json']
        if BR == 'SURVX':
            real_main = scanner.main
            def raising_main():
                real_main()
                raise RuntimeError('fixture: main raised after the pool stop')
            scanner.main = raising_main
        sys.exit(scanner.cli_main())
    rc = scanner.cmd_all([os.environ['C03E2_ROOT']], True)
    print('RESULT ' + json.dumps({'rc': rc, 'unbounded': scanner._POOL_UNBOUNDED}), flush=True)
'''


class PoolBoundedStop(unittest.TestCase):
    setUp = OwnershipRegression.setUp
    tearDown = OwnershipRegression.tearDown
    stop_recorded = OwnershipRegression.stop_recorded
    _report_marker_stats = OwnershipRegression._report_marker_stats

    def run_e2(self, branch, skills=('a-blocker', 'b-dier')):
        root = self.root/'e2skills'
        for name in skills:
            (root/name).mkdir(parents=True)
            (root/name/'SKILL.md').write_text('# fixture')
        script = self.root/'e2fixture.py'
        script.write_text(_E2_FIXTURE % {'bin': str(BIN)})
        env = dict(os.environ, C03E2_BRANCH=branch, C03E2_TMP=str(self.root), C03E2_ROOT=str(root), C03E2_D='1.0')
        out_f = open(self.root/'e2.out', 'w'); err_f = open(self.root/'e2.err', 'w')
        p = subprocess.Popen([PYTHON, '-B', str(script)], stdout=out_f, stderr=err_f, env=env,
                             start_new_session=(os.name == 'posix'))
        self.children.append(p)
        try:
            p.wait(timeout=40)
            finished = True
        except subprocess.TimeoutExpired:
            finished = False
            if os.name == 'posix':
                os.killpg(p.pid, signal.SIGKILL)
        out_f.close(); err_f.close()
        out = (self.root/'e2.out').read_text(); err = (self.root/'e2.err').read_text()
        pids = [int(x) for x in (self.root/'pids.txt').read_text().split()] if (self.root/'pids.txt').exists() else []
        self.addCleanup(self.reap_e2, pids)
        self.assertTrue(finished, 'pool fixture %s did not finish: the stop hung (stderr %r)' % (branch, err[-400:]))  # wallclock: guard 40 s only ends a hang; the verdict is finished/unfinished, the designed worst case is D 1 s + 3 phases of 1 s
        result = [json.loads(l[7:]) for l in out.splitlines() if l.startswith('RESULT ')]
        return p.returncode, (result[0] if result else None), err, pids

    def reap_e2(self, pids):
        # Fixture workers sleep 3600 s, so a recorded pid that is still alive is still that worker.
        for pid in pids:
            if alive(pid):
                try:
                    os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass

    def assert_no_leftover(self, pids):
        end = time.monotonic() + 3
        left = [x for x in pids if alive(x)]
        while left and time.monotonic() < end:
            time.sleep(.05)
            left = [x for x in left if alive(x)]
        self.assertEqual(left, [], 'pool workers outlived the scan')  # wallclock: guard 3 s only lets the init process reap orphans; a real survivor sleeps 3600 s

    @unittest.skipUnless(os.name == 'posix', 'SIGTERM-ignoring worker: POSIX only (Windows UNMEASURED)')
    def test_broken_pool_with_sigterm_ignoring_sibling_is_bounded(self):
        rc, res, err, pids = self.run_e2('B')
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertIn('직렬로 폴백', err)
        self.assertIn('BrokenProcessPool', err)
        self.assert_no_leftover(pids)

    def test_never_ready_pool_falls_back_at_the_start_deadline(self):
        rc, res, err, pids = self.run_e2('N')
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertIn('직렬로 폴백', err)
        self.assertIn('start deadline', err)
        self.assert_no_leftover(pids)

    @unittest.skipUnless(os.name == 'posix', 'SIGTERM-ignoring worker: POSIX only (Windows UNMEASURED)')
    def test_never_ready_worker_ignoring_sigterm_is_killed(self):
        rc, res, err, pids = self.run_e2('NI')
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertIn('start deadline', err)
        self.assert_no_leftover(pids)

    def test_one_never_ready_worker_does_not_hold_the_exit(self):
        rc, res, err, pids = self.run_e2('X', skills=('a', 'b', 'c', 'd'))
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertNotIn('직렬로 폴백', err)
        self.assert_no_leftover(pids)

    def test_worker_dying_at_start_falls_back_without_leftover(self):
        rc, res, err, pids = self.run_e2('S')
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertIn('BrokenProcessPool', err)
        self.assert_no_leftover(pids)

    def test_healthy_pool_is_silent_and_leaves_no_worker(self):
        rc, res, err, pids = self.run_e2('H', skills=('a', 'b', 'c'))
        self.assertEqual((res['rc'], res['unbounded']), (0, False), err)
        self.assertEqual(err, '')
        self.assert_no_leftover(pids)

    @unittest.skipUnless(os.name == 'posix', 'planted survivor via no-op signals: POSIX fixture (Windows UNMEASURED)')
    def test_cli_with_a_surviving_worker_exits_instead_of_hanging(self):
        rc, res, err, pids = self.run_e2('SURV')
        self.assertEqual(rc, 0, err)
        self.assertIn('unbounded', err)
        self.assertTrue(any(alive(pid) for pid in pids), 'planted survivor not present: the control did not plant')

    @unittest.skipUnless(os.name == 'posix', 'planted survivor via no-op signals: POSIX fixture (Windows UNMEASURED)')
    def test_cli_raising_with_a_surviving_worker_exits_instead_of_hanging(self):
        # cli_main's exception path: main() raises after the pool was left unbounded (mutant E4b survived without this).
        rc, res, err, pids = self.run_e2('SURVX')
        self.assertEqual(rc, 1, err)
        self.assertIn('fixture: main raised after the pool stop', err)
        self.assertIn('unbounded', err)
        self.assertTrue(any(alive(pid) for pid in pids), 'planted survivor not present: the control did not plant')

    def test_empty_skill_list_creates_no_pool(self):
        sys.path.insert(0, str(BIN)); scanner = load('javis_skillscan')
        import concurrent.futures
        from unittest.mock import patch
        created = []
        def no_pool(*a, **kw):
            created.append(a)
            raise RuntimeError('fixture: a pool was created')
        empty = self.root/'none'; empty.mkdir()
        with patch.object(concurrent.futures, 'ProcessPoolExecutor', no_pool):
            rc = scanner.cmd_all([str(empty)], True)
        self.assertEqual(rc, 0)
        self.assertEqual(created, [], 'a pool was created for zero skills')

    def test_cancel_during_the_start_wait_returns_125_without_serial(self):
        import threading, concurrent.futures
        from unittest.mock import patch
        sys.path.insert(0, str(BIN)); scanner = load('javis_skillscan')
        cancel = threading.Event(); scanner._CANCEL = cancel
        skill = self.root/'skills'/'one'; skill.mkdir(parents=True); (skill/'SKILL.md').write_text('# fixture')
        calls = []
        class NeverReady:
            _processes = {}
            def __init__(self, *a, **kw):
                pass
            def submit(self, fn, *a):
                cancel.set()
                return concurrent.futures.Future()
            def map(self, *a, **kw):
                cancel.set()
                return iter(())
            def shutdown(self, *a, **kw):
                pass
            def __enter__(self):
                return self
            def __exit__(self, *a):
                return False
        with patch.object(concurrent.futures, 'ProcessPoolExecutor', NeverReady), \
             patch.object(scanner, 'scan_skill', lambda path: calls.append(path)):
            rc = scanner.cmd_all([str(skill.parent)], True)
        self.assertEqual(rc, 125)
        self.assertEqual(calls, [], 'cancelled scanner started a serial rescan')

    def test_executor_worker_table_exists_on_this_python(self):
        import concurrent.futures
        ex = concurrent.futures.ProcessPoolExecutor(max_workers=1)
        try:
            self.assertEqual(ex.submit(os.getpid).result(timeout=60) > 0, True)  # wallclock: guard 60 s only ends a hung spawn
            table = getattr(ex, '_processes', None)
            self.assertIsInstance(table, dict, 'ProcessPoolExecutor._processes is gone: the bounded stop has no worker table')
            self.assertTrue(table and all(hasattr(p, 'sentinel') for p in table.values()))
        finally:
            ex.shutdown(wait=True)

    def test_start_deadline_and_stop_phases_fit_the_preflight_budget(self):
        sys.path.insert(0, str(BIN)); scanner = load('javis_skillscan'); preflight = load('javis_preflight')
        self.assertLess(scanner.POOL_START_DEADLINE_S + 3 * scanner.POOL_STOP_PHASE_S,
                        preflight.Preflight.SKILLSCAN_SCAN_TIMEOUT_S)



class MarkerReaderContract(unittest.TestCase):
    # marker() contract (CEO ruling run-3 s4): fail loudly when retries end, bound attempts AND time, print the
    # retries used every run; POSIX kills only a pid whose identity still matches. Specimens call _marker_pid
    # directly, so their synthetic pids never reach self.extra, OpenProcess or os.kill.
    setUp = OwnershipRegression.setUp
    tearDown = OwnershipRegression.tearDown
    _marker_pid = OwnershipRegression._marker_pid
    stop_recorded = OwnershipRegression.stop_recorded
    _report_marker_stats = OwnershipRegression._report_marker_stats

    def reads(self, script):
        from unittest.mock import patch
        calls = []
        def fake(path_self, *a, **kw):
            item = script[min(len(calls), len(script) - 1)]
            calls.append(item)
            if isinstance(item, type) and issubclass(item, BaseException):
                raise item('fixture')
            if callable(item):
                return item()
            return item
        return calls, patch.object(Path, 'read_text', fake)

    def test_transient_permission_error_is_retried_and_counted(self):
        calls, p = self.reads([PermissionError, PermissionError, PermissionError, '4242'])
        with p:
            pid = self._marker_pid(self.root/'m', retry_permission=True)
        self.assertEqual(pid, 4242)
        self.assertEqual(self.marker_stats[-1][:2], ('m', 3))

    def test_persistent_permission_error_fails_loudly_after_the_attempt_bound(self):
        calls, p = self.reads([PermissionError])
        with p, self.assertRaises(self.failureException) as cm:
            self._marker_pid(self.root/'m', retry_permission=True)
        self.assertIsInstance(cm.exception.__cause__, PermissionError)
        self.assertIn('25 attempts', str(cm.exception))
        self.assertEqual(self.marker_stats[-1][:2], ('m', 25))

    def test_attempt_bound_ends_the_retries(self):
        calls, p = self.reads([PermissionError])
        with p, self.assertRaises(self.failureException) as cm:
            self._marker_pid(self.root/'m', retry_permission=True, attempts=25, budget_s=600)
        self.assertEqual(len(calls), 25)
        self.assertIn('attempt bound', str(cm.exception))

    def test_time_bound_ends_the_retries(self):
        # budget 0: the time bound is reached at the first PermissionError, with no clock premise. Without a time
        # bound the call would run to the readiness limit and say so, which fails the assertion below.
        calls, p = self.reads([PermissionError])
        with p, self.assertRaises(self.failureException) as cm:
            self._marker_pid(self.root/'m', retry_permission=True, attempts=100000, budget_s=0.0, limit_s=2.0)
        self.assertIn('time bound', str(cm.exception))
        self.assertEqual(len(calls), 1)

    def test_posix_branch_surfaces_permission_error_at_once(self):
        calls, p = self.reads([PermissionError])
        with p, self.assertRaises(PermissionError):
            self._marker_pid(self.root/'m', retry_permission=False)
        self.assertEqual(len(calls), 1)

    @unittest.skipUnless(os.name == 'posix', 'marker() retries PermissionError on win32 only')
    def test_marker_on_posix_surfaces_permission_error_at_once(self):
        # marker() itself, not _marker_pid: its retry_permission argument is os.name == 'nt' (mutant M5b survived
        # without this). The read raises before any pid exists, so nothing reaches self.extra or the kill path.
        # Called explicitly, not borrowed into the class: a specimen with a successful read must never reach marker().
        calls, p = self.reads([PermissionError])
        with p, self.assertRaises(PermissionError):
            OwnershipRegression.marker(self, 'm')
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.extra, [])

    def test_teardown_prints_the_retries_line(self):
        # The real tearDown, run on an inner test, must print the retries line (mutant M8b survived without this).
        import io
        from unittest.mock import patch
        outer = self
        class Inner(unittest.TestCase):
            setUp = OwnershipRegression.setUp
            tearDown = OwnershipRegression.tearDown
            _marker_pid = OwnershipRegression._marker_pid
            stop_recorded = OwnershipRegression.stop_recorded
            _report_marker_stats = OwnershipRegression._report_marker_stats
            def runTest(self):
                calls, p = outer.reads(['9'])
                with p:
                    self._marker_pid(self.root/'inner', retry_permission=True)
        buf = io.StringIO()
        result = unittest.TestResult()
        with patch.object(sys, 'stderr', buf):
            Inner().run(result)
        self.assertEqual(result.errors + result.failures, [])
        self.assertIn('inner=0/', buf.getvalue())

    def test_retries_line_is_printed_even_for_zero(self):
        import io
        calls, p = self.reads(['55'])
        with p:
            self._marker_pid(self.root/'m', retry_permission=True)
        buf = io.StringIO()
        OwnershipRegression._report_marker_stats(self, buf)
        self.assertIn('m=0/', buf.getvalue())

    def test_not_yet_published_is_not_a_permission_retry(self):
        calls, p = self.reads([FileNotFoundError, FileNotFoundError, '7'])
        with p:
            self.assertEqual(self._marker_pid(self.root/'m', retry_permission=True), 7)
        self.assertEqual(self.marker_stats[-1][:2], ('m', 0))

    def test_readiness_limit_inside_a_permission_streak_names_the_permission_error(self):
        calls, p = self.reads([FileNotFoundError, FileNotFoundError, FileNotFoundError, PermissionError])
        with p, self.assertRaises(self.failureException) as cm:
            self._marker_pid(self.root/'m', retry_permission=True, attempts=100000, budget_s=600, limit_s=1.0)
        self.assertIn('readiness limit', str(cm.exception))
        self.assertIsInstance(cm.exception.__cause__, PermissionError)

    @unittest.skipUnless(os.name == 'posix', 'pid identity check is POSIX (Windows kills through the handle)')
    def test_recorded_pid_is_killed_when_its_identity_matches(self):
        p = subprocess.Popen([PYTHON, '-c', 'import time; time.sleep(3600)']); self.children.append(p)
        self.identities[p.pid] = _start_time(p.pid)
        self.stop_recorded(p.pid)
        p.wait(timeout=30)  # wallclock: guard 30 s only ends a hang; the verdict is the SIGKILL return code
        self.assertEqual(p.returncode, -signal.SIGKILL)

    def signals_sent(self):
        # SIGKILL lands and is reaped asynchronously, so p.poll() right after a kill can still read None: record the
        # signals stop_recorded sends instead (mutants K1c/K2c - kill AND record not-killed - survived a poll() check).
        # Record only, never forward: a refused or gone pid must not be signalled even when the rule under test is wrong.
        from unittest.mock import patch
        sent = []
        return sent, patch.object(os, 'kill', lambda target, sig: sent.append((target, sig)))

    @unittest.skipUnless(os.name == 'posix', 'pid identity check is POSIX (Windows kills through the handle)')
    def test_kill_refused_when_the_start_time_changed(self):
        p = subprocess.Popen([PYTHON, '-c', 'import time; time.sleep(3600)']); self.children.append(p)
        self.identities[p.pid] = ('ok', 'Mon Jan  1 00:00:00 2001')
        sent, spy = self.signals_sent()
        with spy:
            self.stop_recorded(p.pid)
        self.assertEqual(sent, [], 'a pid whose start time changed was signalled')
        self.assertIn('%d:start-time-changed' % p.pid, self.not_killed)

    @unittest.skipUnless(os.name == 'posix', 'pid identity check is POSIX (Windows kills through the handle)')
    def test_kill_refused_when_identity_is_unavailable(self):
        from unittest.mock import patch
        p = subprocess.Popen([PYTHON, '-c', 'import time; time.sleep(3600)']); self.children.append(p)
        self.identities[p.pid] = _start_time(p.pid)
        sent, spy = self.signals_sent()
        with spy, patch.object(subprocess, 'run', side_effect=PermissionError('fixture: ps blocked')):
            self.stop_recorded(p.pid)
        self.assertEqual(sent, [], 'a pid with no readable identity was signalled')
        self.assertIn('%d:identity-unavailable' % p.pid, self.not_killed)

    @unittest.skipUnless(os.name == 'posix', 'pid identity check is POSIX (Windows kills through the handle)')
    def test_gone_pid_is_not_signalled(self):
        p = subprocess.Popen([PYTHON, '-c', 'pass']); self.children.append(p)
        p.wait(timeout=30)  # wallclock: guard 30 s only ends a hang of a process that does nothing
        self.identities[p.pid] = ('ok', 'recorded')
        sent, spy = self.signals_sent()
        with spy:
            self.stop_recorded(p.pid)
        self.assertEqual(sent, [], 'a gone pid was signalled')
        self.assertEqual(self.not_killed, [])


if __name__ == '__main__':
    # Tests never inherit a live seat's paths or account configuration.
    for key in list(os.environ):
        if key.startswith('CYS_') or key == 'CLAUDE_CONFIG_DIR':
            os.environ.pop(key)
    with tempfile.TemporaryDirectory(prefix='c03-home-') as home:
        os.environ['HOME']=home
        if os.name == 'nt':
            os.environ['LOCALAPPDATA']=str(Path(home)/'localappdata')
        print(json.dumps({'executable': sys.executable, 'version': sys.version, 'platform': sys.platform}), flush=True)
        run=unittest.main(exit=False)
        if os.name == 'nt' and run.result.wasSuccessful():
            print('C03-WINDOWS-NATIVE-OK',flush=True)
        sys.exit(0 if run.result.wasSuccessful() else 1)
