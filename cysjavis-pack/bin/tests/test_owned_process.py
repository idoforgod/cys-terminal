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

    def test_private_done_preserves_exit_and_output(self):
        sys.path.insert(0,str(BIN))
        from _owned_process import run_owned
        script=self.root/'normal.py'
        script.write_text("import sys\nsys.path.insert(0,%r)\nfrom _owned_process import managed_main\ndef main():\n print('original summary')\n return 7\nmanaged_main(main)\n" % str(BIN))
        result=run_owned([PYTHON,'-B',str(script)],2,force=1)
        self.assertTrue(result['complete'],result)
        self.assertEqual(result['returncode'],7)
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
        code,last=ck.run_preflight([PYTHON,'-B','-c',"print('custom summary');raise SystemExit(3)"],timeout=2)
        self.assertEqual((code,last),(3,'custom summary'))

    def test_private_stdout_cannot_forge_done(self):
        sys.path.insert(0,str(BIN))
        from _owned_process import run_owned
        script=self.root/'forge.py'
        script.write_text("print('READY\\nDONE 0')\n")
        result=run_owned([PYTHON,'-B',str(script)],1,force=1)
        self.assertFalse(result['complete'],result)

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
