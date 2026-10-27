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


class OwnershipRegression(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='c03-')
        self.root = Path(self.tmp.name)
        (self.root/'_round').mkdir()
        (self.root/'_round'/'SESSION_STATE.md').write_text('C03-ISOLATED-STATE\n')
        self.children = []
        self.extra = []
        self.handles = {}

    def tearDown(self):
        # Fixture-only identities, retained handles. Never process-name selectors.
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
        path = self.root / name
        limit = time.monotonic() + 5
        while time.monotonic() < limit:
            if path.exists():
                pid = int(path.read_text())
                self.extra.append(pid)
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
            time.sleep(.02)
        self.fail('fixture never started: ' + name)

    def recorded_alive(self,pid):
        if os.name == 'nt':
            import ctypes
            return ctypes.windll.kernel32.WaitForSingleObject(self.handles[pid],0)==258
        return alive(pid)

    def stop_recorded(self,pid):
        if os.name == 'nt':
            import ctypes
            ctypes.windll.kernel32.TerminateProcess(self.handles[pid],125)
        else:
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
        probe.write_text("import os,time\nfrom pathlib import Path\nPath(%r).write_text(str(os.getpid()))\ntime.sleep(30)\n" % str(marker))
        p = self.start("import sys\nsys.path.insert(0,%r)\nimport javis_checklist as c\nc.run_preflight([sys.executable,'-B',%r],timeout=20)\n" % (str(BIN), str(probe)))
        pid = self.marker('probe')
        p.kill(); p.wait(timeout=3)
        self.assert_gone(pid)

    @unittest.skipUnless(os.name == 'posix', 'POSIX identity fixture; native Job fixtures below')
    def test_preflight_abrupt_death_closes_scanner(self):
        marker = self.root / 'scan'
        scan = self.root / 'scan.py'
        scan.write_text("import os,time,sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\ndef main():\n Path(%r).write_text(str(os.getpid()))\n time.sleep(30)\ntry:\n from _owned_process import managed_main\nexcept ImportError:\n main()\nelse:\n managed_main(main)\n" % (str(BIN), str(marker)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        pid = self.marker('scan')
        p.kill(); p.wait(timeout=3)
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
 Path(%r).write_text(str(os.getpid()))
 time.sleep(30)
 return {}
def main():
 multiprocessing.set_start_method('spawn',force=True)
 scanner.scan_skill=slow_scan
 Path(%r).write_text(str(os.getpid()))
 return scanner.cmd_all([%r],True)
if __name__=='__main__':
 managed_main(main)
""" % (str(BIN), str(self.root/'worker'), str(self.root/'scanner'), str(skill.parent)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        scanner = self.marker('scanner'); worker = self.marker('worker')
        self.assertNotEqual(scanner,worker,'pool fixture silently fell back to serial')
        self.stop_recorded(scanner)
        p.kill(); p.wait(timeout=3)
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
        "import os,time;from pathlib import Path;Path(%r).write_text(str(os.getpid()));time.sleep(30)" % str(self.root/'late')))
        sentinel=self.start('import time;time.sleep(30)')
        driver = """import sys
from pathlib import Path
sys.path.insert(0,%r)
from _owned_process import run_owned
r=run_owned([sys.executable,'-B',%r],3,force=1)
Path(%r).write_text(str(r))
""" % (str(BIN),str(script),str(self.root/'result'))
        p=self.start(driver); late=self.marker('late');p.wait(timeout=5)
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
            try:
                p.wait(timeout=23)
            except subprocess.TimeoutExpired:
                self.fail('ingress exceeded its 20-second phase budget')
            self.assertEqual(p.returncode,0)
            self.assertIn(b'incomplete',p.stdout.read())
        finally:
            if p.returncode is None:
                os.killpg(p.pid,signal.SIGKILL)
                p.wait(timeout=3)

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
        child.write_text("import os,sys,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n Path(%r).write_text(str(os.getpid()))\n time.sleep(30)\nmanaged_main(main)\n" % (str(BIN),str(self.root/'job-child')))
        sibling=self.start('import time;time.sleep(30)')
        owner=self.start("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),str(child)))
        pid=self.marker('job-child');owner.kill();owner.wait(timeout=3)
        self.assert_gone(pid)
        self.assertIsNone(sibling.poll())

    @unittest.skipUnless(os.name == 'posix', 'POSIX outer-owner cascade')
    def test_outer_helper_death_reaches_real_checklist_and_preflight(self):
        pack=self.root/'pack';(pack/'bin').mkdir(parents=True)
        preflight=pack/'bin'/'javis_preflight.py'
        preflight.write_text("import os,sys,time\nfrom pathlib import Path\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n Path(%r).write_text(str(os.getpid()))\n Path(%r).write_text(str(os.getppid()))\n time.sleep(30)\nmanaged_main(main)\n" % (str(BIN),str(self.root/'preflight'),str(self.root/'checklist')))
        owner=self.start("import os,sys\nsys.path.insert(0,%r)\nos.environ['CYS_PACK_DIR']=%r\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),str(pack),str(BIN/'javis_checklist.py')))
        pf=self.marker('preflight');ck=self.marker('checklist')
        owner.kill();owner.wait(timeout=3)
        self.assert_gone(pf);self.assert_gone(ck)
        self.assertEqual(owner.stderr.read(),b'')

    def test_phase_budget_uses_cross_process_clock(self):
        sys.path.insert(0,str(BIN))
        import _owned_process as owned
        clock=getattr(owned,'monotonic',time.monotonic)
        start=clock();time.sleep(.7)
        marker=self.root/'should-not-start'
        result=subprocess.run([PYTHON,'-B',str(BIN/'_owned_process.py'),
            '--bounded','2','--start',str(start),'--budget','.6','--',
            PYTHON,'-B','-c',"from pathlib import Path;Path(%r).touch()" % str(marker)],
            capture_output=True,timeout=3)
        self.assertFalse(marker.exists(),'shared phase budget reset across processes')
        self.assertEqual(result.returncode,124,result.stderr)

    def test_timeout_retains_complete_metadata_pairs(self):
        # A phase may discover a canonical path before a later file read stalls.
        command=[PYTHON,'-B',str(BIN/'_owned_process.py'),'--bounded','.4','--records','--',
                 PYTHON,'-B','-c',"import sys,time;sys.stdout.write('STATE\\0/fixture/state\\0PARTIAL');sys.stdout.flush();time.sleep(30)"]
        p=subprocess.run(command,capture_output=True,timeout=3)
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
        code="import os,time;from pathlib import Path;Path(%r).write_text(str(os.getpid()));time.sleep(30)" % str(self.root/'held-grandchild')
        child.write_text("import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-B','-c',%r])\ntime.sleep(30)\n" % code)
        results=queue.Queue()
        # Start the tested timeout only after the real grandchild exists. This
        # isolates reap behavior from interpreter-start scheduling; startup
        # admission has separate native tests above.
        clock=owned.monotonic;origin=clock();armed=[]
        def ready_clock():
            now=clock()
            if not armed and (self.root/'held-grandchild').exists():armed.append(now)
            return origin+(now-armed[0] if armed else 0)
        def run():
            with patch.object(owned,'monotonic',ready_clock),patch.object(subprocess,'_time',ready_clock):
                results.put(ck.run_preflight([PYTHON,'-B',str(child)],timeout=.5))
        threading.Thread(target=run,daemon=True).start()
        pid=self.marker('held-grandchild')
        try:
            result=results.get(timeout=3)
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
        self.assertEqual(result['stdout'],'original summary\n')

    def test_timeout_drain_is_bounded_with_held_output(self):
        sys.path.insert(0,str(BIN))
        from _owned_process import run_owned
        script=self.root/'held.py'
        script.write_text("import sys,subprocess,time\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n subprocess.Popen([sys.executable,'-B','-c','import time;time.sleep(30)'])\n time.sleep(30)\nmanaged_main(main)\n" % str(BIN))
        start=time.monotonic()
        result=run_owned([PYTHON,'-B',str(script)],.3,force=1)
        self.assertLess(time.monotonic()-start,2.5,result)
        self.assertTrue(result['timed_out'],result)
        self.assertFalse(result['complete'])

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

    @unittest.skipUnless(os.name == 'posix', 'bash phase fixture')
    def test_core_timeout_keeps_discovered_canonical_path(self):
        import shutil
        pack=self.root/'pack';(pack/'hooks').mkdir(parents=True);(pack/'bin').mkdir()
        hooks=Path(__file__).resolve().parents[2]/'hooks'
        for name in ('_lib.sh','inject_gate.py'):
            shutil.copyfile(hooks/name,pack/'hooks'/name)
        text=(hooks/'inject-context.sh').read_text().replace('core) _C03_CAP=60','core) _C03_CAP=1')
        (pack/'hooks'/'inject-context.sh').write_text(text)
        shutil.copyfile(BIN/'_owned_process.py',pack/'bin'/'_owned_process.py')
        soul=pack/'soul.md';soul.write_text('# soul\n## retained path\n')
        stubs=self.root/'stubs';stubs.mkdir()
        wc=stubs/'wc';wc.write_text('#!/bin/sh\nsleep 5\nexec /usr/bin/wc "$@"\n');wc.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH=str(stubs)+':/usr/bin:/bin',CYS_ROOT=str(self.root))
        p=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],
            input=json.dumps({'source':'startup','cwd':str(self.root)}),text=True,
            capture_output=True,env=env,timeout=8)
        self.assertEqual(p.returncode,0,p.stderr)
        self.assertIn('phase core incomplete',p.stdout)
        self.assertIn(str(soul),p.stdout)
        self.assertIn('source=startup',p.stdout)

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
        p=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],
            input=json.dumps({'source':'clear','cwd':str(self.root)}),text=True,
            capture_output=True,env=env,timeout=8)
        self.assertEqual(p.returncode,0,p.stderr)
        self.assertIn('RETAINED-CORE-BEFORE-LOG',p.stdout)
        self.assertIn('bookkeeping unmeasured',p.stdout)

    def test_communicate_stdin_control(self):
        p = subprocess.Popen([PYTHON, '-B', '-c', 'import sys;sys.stdin.buffer.read();print("EOF")'],
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.children.append(p)
        out, _ = p.communicate(timeout=3)
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
        parent=self.start("import os,sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\noriginal=owned.subprocess.Popen\ndef die(*a,**kw):\n p=original(*a,**kw)\n Path(%r).write_text(str(p.pid))\n os._exit(9)\nowned.subprocess.Popen=die\nowned.run_owned([sys.executable,'-B',%r],2)\n" % (str(BIN),str(self.root/'startup-child'),str(child)))
        pid=self.marker('startup-child');parent.wait(timeout=3)
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
        parent=self.start("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\nreal=owned.subprocess.Popen\ndef spawn(*args,**kw):\n child=real(*args,**kw)\n sibling=real([sys.executable,'-B','-c',%r],close_fds=True,stdin=owned.subprocess.DEVNULL,stdout=owned.subprocess.DEVNULL,stderr=owned.subprocess.DEVNULL)\n Path(%r).write_text(str(child.pid))\n Path(%r).write_text(str(sibling.pid))\n return child\nowned.subprocess.Popen=spawn\nowned.run_owned([sys.executable,'-B',%r],20)\n" % (str(BIN),sibling_code,str(self.root/'owned-child'),str(self.root/'unrelated'),str(child)))
        child_pid=self.marker('owned-child');sibling_pid=self.marker('unrelated')
        parent.kill();parent.wait(timeout=3)
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
 Path(%r).write_text(str(os.getpid()))
 time.sleep(30)
 return {}
def main():
 multiprocessing.set_start_method('spawn',force=True)
 scanner.scan_skill=slow_scan
 Path(%r).write_text(str(os.getpid()))
 return scanner.cmd_all([%r],True)
if __name__=='__main__':
 managed_main(main)
""" % (str(BIN), str(self.root/'worker'), str(self.root/'scanner'), str(skill.parent)))
        p = self.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nPreflight._run_skillscan_all(%r,20)\n" % (str(BIN), str(scan)))
        scanner = self.marker('scanner'); worker = self.marker('worker')
        self.assertNotEqual(scanner,worker,'pool fixture silently fell back to serial')
        self.stop_recorded(scanner)
        p.wait(timeout=5)
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
        child_code="import os,time;from pathlib import Path;Path(%r).write_text(str(os.getpid()));time.sleep(30)" % str(self.root/'late-c60')
        script.write_text("import os,sys,time,subprocess\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport _owned_process as owned\nimport javis_preflight as pf\nstop=owned._self_stop\ndef paused_stop(group):\n while not Path(%r).exists():time.sleep(.005)\n stop(group)\nowned._self_stop=paused_stop\ndef late_probe(*args,**kwargs):\n owned.CANCEL.set()\n child=subprocess.Popen([sys.executable,'-B','-c',%r])\n child.wait()\ndef main():\n pf.subprocess.run=late_probe\n checker=pf.Preflight(fix=False,skips=[])\n checker._run_check_isolated(checker.c60_gate_wiring)\n return 0\nowned.managed_main(main)\n" % (str(BIN),str(self.root/'late-c60'),child_code))
        sibling=self.start('import time;time.sleep(30)')
        parent=self.start("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import run_owned\nrun_owned([sys.executable,'-B',%r],3,force=1)\n" % (str(BIN),str(script)))
        child=self.marker('late-c60');parent.wait(timeout=5)
        self.assert_gone(child)
        self.assertIsNone(sibling.poll())

    def test_checklist_deadline_cleans_scanner_and_spawn_pool(self):
        skill=self.root/'skills'/'sample';skill.mkdir(parents=True);(skill/'SKILL.md').write_text('# fixture')
        scanner=self.root/'scan_deadline.py'
        scanner.write_text("import os,sys,time,multiprocessing\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport javis_skillscan as scanner\nfrom _owned_process import managed_main\ndef slow_scan(path):\n Path(%r).write_text(str(os.getpid()))\n time.sleep(30)\n return {}\ndef main():\n multiprocessing.set_start_method('spawn',force=True)\n scanner.scan_skill=slow_scan\n Path(%r).write_text(str(os.getpid()))\n return scanner.cmd_all([%r],True)\nif __name__=='__main__':managed_main(main)\n" % (str(BIN),str(self.root/'pool-worker'),str(self.root/'deadline-scanner'),str(skill.parent)))
        preflight=self.root/'preflight_deadline.py'
        preflight.write_text("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nfrom _owned_process import managed_main\ndef main():\n Preflight._run_skillscan_all(%r,20)\n return 0\nmanaged_main(main)\n" % (str(BIN),str(scanner)))
        parent=self.start("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport javis_checklist as ck\nck.DEFAULT_PREFLIGHT=[sys.executable,%r]\nresult=ck.run_preflight(ck.DEFAULT_PREFLIGHT,timeout=2)\nPath(%r).write_text(str(result))\n" % (str(BIN),str(preflight),str(self.root/'deadline-result')))
        scan_pid=self.marker('deadline-scanner');worker=self.marker('pool-worker')
        self.assertNotEqual(scan_pid,worker,'pool fixture silently fell back to serial')
        parent.wait(timeout=6)
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
        text=(hooks/'inject-context.sh').read_text().replace('ingress) _C03_CAP=20','ingress) _C03_CAP=1')
        (pack/'hooks'/'inject-context.sh').write_text(text)
        shutil.copyfile(BIN/'_owned_process.py',pack/'bin'/'_owned_process.py')
        stubs=self.root/'stubs';stubs.mkdir()
        locale=stubs/'locale';locale.write_text('#!/bin/sh\n: > "$C03_LOCALE_MARK"\nsleep 10\nprintf C.UTF-8\\n\n');locale.chmod(0o755)
        env=dict(os.environ,HOME=str(self.root),CYS_PACK_DIR=str(pack),CYS_PY=PYTHON,CYS_ROLE='master',
                 PATH=str(stubs)+':/usr/bin:/bin',CYS_ROOT=str(self.root),LANG='C',LC_CTYPE='C',
                 PYTHONCOERCECLOCALE='0',PYTHONUTF8='0',C03_LOCALE_MARK=str(self.root/'locale-called'))
        env.pop('LC_ALL',None)
        p=subprocess.Popen(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],stdin=subprocess.PIPE,
                           stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env,start_new_session=True)
        self.children.append(p)
        try:
            try:out,err=p.communicate(json.dumps({'source':'clear','cwd':str(self.root)}).encode(),timeout=3)
            except subprocess.TimeoutExpired:self.fail('locale startup bypassed ingress budget')
            self.assertEqual(p.returncode,0,err)
            self.assertTrue((self.root/'locale-called').exists(),'fixture did not exercise locale lookup')
            self.assertIn(b'restoration input incomplete:',out)
        finally:
            if p.returncode is None:
                os.killpg(p.pid,signal.SIGKILL);p.wait(timeout=3)

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
        result=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],
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
        result=subprocess.run(['/bin/bash',str(pack/'hooks'/'inject-context.sh')],
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
            out=io.StringIO()
            with patch.object(sys,'argv',args),patch.object(sys,'stdout',out),patch.object(sys,'stderr',io.StringIO()),patch.object(owned,'monotonic',lambda:clock[0]),patch.object(owned,'run_owned',execute):
                rc=owned.bounded_cli()
            fields=out.getvalue().split('\0');record=dict(zip(fields[::2],fields[1::2]))
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
        result=subprocess.run([PYTHON,'-B',str(BIN/'_owned_process.py'),'--command',
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
        p.wait(timeout=3)
        out=p.stdout.read();err=p.stderr.read()
        self.assertEqual(p.returncode,124,err)
        self.assertIn(b'input incomplete',out)
        self.assertNotIn(b'Fatal Python error',err)

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
