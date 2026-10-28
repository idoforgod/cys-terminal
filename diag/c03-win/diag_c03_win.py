#!/usr/bin/env python3
"""C-03 win32 diagnostic - diag branch only, never merged or shipped.

Instrumented copies of the four cysjavis-pack/bin/tests/test_owned_process.py
tests that failed on win32 at f138f0fa. The fixtures are the originals plus:
  * an import of a small module (_c03diag) that writes per-process event files,
    a stderr tee file and periodic faulthandler stack dumps (file-based: a spawn
    worker of a console-less parent may have no usable stderr);
  * wrappers around the scanner's pool_initializer_args / worker_observer /
    ProcessPoolExecutor and the worker's _owned_process._self_stop;
  * an observation window of WINDOW seconds instead of 5 (the 5 s verdict of
    the original assertion is recorded next to it, unchanged);
  * the driver's stderr and its _run_skillscan_all return value kept, not dropped.
It REPORTS. It does not decide pass/fail of the product; it exits 1 only if
the instrument itself could not run (no scanner events at all).
"""
import json
import os
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BIN = REPO / 'cysjavis-pack' / 'bin'
PYTHON = sys.executable
WINDOW = float(os.environ.get('C03_DIAG_WINDOW', '45'))
ITER = sys.argv[1] if len(sys.argv) > 1 else '1'
CAP = 20000  # bytes printed per capture file; the size is always printed

DIAG_MOD = r'''
import faulthandler, json, os, sys, threading, time, traceback
_ROOT = None; _ROLE = None; _KEEP = []
def ev(name, **kw):
    rec = {'ev': name, 'pid': os.getpid(), 'role': _ROLE, 'mono': time.monotonic(),
           'wall': time.time(), 'thread': threading.current_thread().name}
    rec.update(kw)
    with open(os.path.join(_ROOT, 'ev-%d.jsonl' % os.getpid()), 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')
class _Tee:
    def __init__(self, orig, path):
        self.orig = orig
        self.f = open(path, 'a', encoding='utf-8', errors='replace')
    def write(self, s):
        try:
            self.f.write(s); self.f.flush()
        except Exception:
            pass
        if self.orig is not None:
            try:
                return self.orig.write(s)
            except Exception:
                pass
        return len(s)
    def flush(self):
        for x in (self.f, self.orig):
            try:
                if x is not None:
                    x.flush()
            except Exception:
                pass
    def __getattr__(self, k):
        return getattr(self.orig if self.orig is not None else self.f, k)
def install(root, role):
    global _ROOT, _ROLE
    _ROOT = root; _ROLE = role
    ev('import', executable=sys.executable, version=sys.version.split()[0],
       isolated=sys.flags.isolated, stderr_is_none=sys.stderr is None)
    sys.stderr = _Tee(sys.stderr, os.path.join(root, 'stderr-%d.txt' % os.getpid()))
    fh = open(os.path.join(root, 'stack-%d.txt' % os.getpid()), 'w', encoding='utf-8')
    _KEEP.append(fh)
    faulthandler.dump_traceback_later(4, repeat=True, file=fh)
    if role != 'main':
        try:
            import _owned_process as op
            orig = op._self_stop
            def _ss(group):
                ev('selfstop', group=group, stack=''.join(traceback.format_stack(limit=8)))
                return orig(group)
            op._self_stop = _ss
        except Exception as e:
            ev('selfstop-wrap-failed', err=repr(e))
def observer(conn, group):
    ev('observer-enter', conn=type(conn).__name__, group=group)
    import _owned_process as op
    try:
        r = op.worker_observer(conn, group)
    except BaseException as e:
        ev('observer-raised', err=repr(e)); raise
    ev('observer-returned')
    return r
def wrap_scanner(scanner):
    import concurrent.futures as cf
    o_args = scanner.pool_initializer_args
    def pargs():
        ev('pool-args-enter')
        r = o_args()
        ev('pool-args-returned', conn=type(r[0]).__name__, group=r[1])
        return r
    scanner.pool_initializer_args = pargs
    scanner.worker_observer = observer
    base = cf.ProcessPoolExecutor
    class P(base):
        def __init__(self, *a, **k):
            ev('pool-ctor-enter', max_workers=k.get('max_workers'))
            super().__init__(*a, **k)
            ev('pool-ctor-returned')
        def map(self, *a, **k):
            ev('map-enter')
            return super().map(*a, **k)
        def __exit__(self, *a):
            ev('pool-exit', exc=repr(a[1]))
            return super().__exit__(*a)
    cf.ProcessPoolExecutor = P
'''

# Instrumentation lines inserted into the original fixture text (marked DIAG).
DIAG_IMPORT = ("sys.path.insert(0,%r);import _c03diag  # DIAG\n"
               "_c03diag.install(%r,'main' if __name__=='__main__' else 'worker')  # DIAG\n"
               "_c03diag.wrap_scanner(scanner) if __name__=='__main__' else None  # DIAG\n")
DIAG_TASK = " _c03diag.ev('task')  # DIAG\n"


def emit(tag, obj):
    print('C03DIAG-%s %s' % (tag, json.dumps(obj, sort_keys=True)), flush=True)


class Harness:
    def __init__(self, name):
        self.name = name
        self.tmp = tempfile.TemporaryDirectory(prefix='c03d-')
        self.root = Path(self.tmp.name)
        (self.root / '_round').mkdir()
        (self.root / '_round' / 'SESSION_STATE.md').write_text('C03-ISOLATED-STATE\n')
        (self.root / '_c03diag.py').write_text(DIAG_MOD)
        self.children = []
        self.t0 = time.monotonic()

    def start(self, text):
        n = len(self.children)
        script = self.root / ('driver%d.py' % n)
        script.write_text(text)
        err = open(self.root / ('driver-stderr-%d.txt' % n), 'wb')
        p = subprocess.Popen([PYTHON, '-B', str(script)], stdout=subprocess.DEVNULL, stderr=err,
                             start_new_session=(os.name == 'posix'))
        err.close()
        self.children.append(p)
        return p

    def wait_marker(self, name, until):
        path = self.root / name
        while time.monotonic() < until:
            try:
                return int(path.read_text()), round(time.monotonic() - self.t0, 3)
            except (FileNotFoundError, ValueError):
                time.sleep(.02)
        return None, None

    def pids_seen(self):
        pids = set()
        for f in self.root.glob('ev-*.jsonl'):
            try:
                pids.add(int(f.name[3:-6]))
            except ValueError:
                pass
        return pids

    def kill_pid(self, pid):
        # Fixture identities only: pids that wrote this fixture's event files.
        try:
            if os.name == 'nt':
                import ctypes
                h = ctypes.windll.kernel32.OpenProcess(0x0001, False, pid)
                if h:
                    ctypes.windll.kernel32.TerminateProcess(h, 125)
                    ctypes.windll.kernel32.CloseHandle(h)
            else:
                os.kill(pid, signal.SIGKILL)
        except OSError:
            pass

    def cleanup_and_dump(self, summary):
        for p in self.children:
            if p.poll() is None:
                p.kill()
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                summary.setdefault('notes', []).append('driver did not exit after kill')
        for pid in sorted(self.pids_seen()):
            self.kill_pid(pid)
        summary['event_pids'] = sorted(self.pids_seen())
        emit('SUMMARY', summary)
        for pat in ('ev-*.jsonl', 'stderr-*.txt', 'stack-*.txt', 'driver-stderr-*.txt',
                    'driver-result*.txt', 'deadline-result'):
            for f in sorted(self.root.glob(pat)):
                try:
                    data = f.read_bytes()
                except OSError as e:
                    print('::group::%s %s iter %s %s (unreadable: %s)' % (self.name, f.name, ITER, '', e))
                    print('::endgroup::')
                    continue
                print('::group::%s %s iter %s (%d B%s)' % (
                    self.name, f.name, ITER, len(data), ', first %d B' % CAP if len(data) > CAP else ''))
                print(data[:CAP].decode('utf-8', 'replace'))
                print('::endgroup::', flush=True)
        events = list(self.root.glob('ev-*.jsonl'))
        try:
            self.tmp.cleanup()
        except OSError as e:
            print('C03DIAG-NOTE tmp cleanup failed: %s' % e)
        return bool(events)


def scan_text(h, worker_name, scanner_name, skills_parent, guard_style):
    # Original fixture text with the DIAG lines inserted (test_owned_process.py
    # :140-158 / :688-706 two-line guard; :739 one-line guard).
    head = ("import os,sys,time,multiprocessing\nfrom pathlib import Path\nsys.path.insert(0,%r)\n"
            "import javis_skillscan as scanner\n" % str(BIN))
    head += DIAG_IMPORT % (str(h.root), str(h.root))
    if guard_style == 'try':
        head += "try:\n from _owned_process import managed_main\nexcept ImportError:\n managed_main=lambda main:main()\n"
    else:
        head += "from _owned_process import managed_main\n"
    body = ("def slow_scan(path):\n" + DIAG_TASK +
            " _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n"
            " time.sleep(30)\n return {}\n"
            "def main():\n multiprocessing.set_start_method('spawn',force=True)\n scanner.scan_skill=slow_scan\n"
            " _marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker)\n"
            " return scanner.cmd_all([%r],True)\n") % (str(h.root / worker_name), str(h.root / scanner_name), str(skills_parent))
    tail = "if __name__=='__main__':\n managed_main(main)\n" if guard_style == 'try' else "if __name__=='__main__':managed_main(main)\n"
    return head + body + tail


def pool_case(name, worker_name, scanner_name, kind):
    h = Harness(name)
    skill = h.root / 'skills' / 'sample'; skill.mkdir(parents=True)
    (skill / 'SKILL.md').write_text('# fixture')
    if kind == 'deadline':
        scan = h.root / 'scan_deadline.py'
        scan.write_text(scan_text(h, worker_name, scanner_name, skill.parent, 'plain'))
        pre = h.root / 'preflight_deadline.py'
        pre.write_text("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nfrom _owned_process import managed_main\ndef main():\n Preflight._run_skillscan_all(%r,20)\n return 0\nmanaged_main(main)\n" % (str(BIN), str(scan)))
        p = h.start("import sys\nfrom pathlib import Path\nsys.path.insert(0,%r)\nimport javis_checklist as ck\nck.DEFAULT_PREFLIGHT=[sys.executable,%r]\nresult=ck.run_preflight(ck.DEFAULT_PREFLIGHT,timeout=2)\nPath(%r).write_text(str(result))\n" % (str(BIN), str(pre), str(h.root / 'deadline-result')))
    else:
        scan = h.root / 'scan_pool.py'
        scan.write_text(scan_text(h, worker_name, scanner_name, skill.parent, 'try'))
        # Original driver + one DIAG line keeping the return value (stderr of the scanner).
        p = h.start("import sys\nsys.path.insert(0,%r)\nfrom javis_preflight import Preflight\nr=Preflight._run_skillscan_all(%r,20)\nopen(%r,'w',encoding='utf-8').write(repr(r))  # DIAG\n" % (str(BIN), str(scan), str(h.root / 'driver-result.txt')))
    end = h.t0 + WINDOW
    s_pid, s_t = h.wait_marker(scanner_name, end)
    # The original marker() window for the worker starts when the scanner marker is seen.
    w_pid, w_t = h.wait_marker(worker_name, end) if s_pid else (None, None)
    while p.poll() is None and time.monotonic() < end:
        time.sleep(.05)
    summary = {'case': name, 'iter': ITER, 'window_s': WINDOW,
               'scanner_pid': s_pid, 'scanner_t': s_t, 'worker_pid': w_pid, 'worker_t': w_t,
               'worker_within_5s_of_scanner': bool(w_pid and s_t is not None and w_t - s_t <= 5),
               'same_pid_serial_fallback': bool(w_pid and w_pid == s_pid),
               'driver_rc': p.poll(), 'driver_exit_t': round(time.monotonic() - h.t0, 3) if p.poll() is not None else None}
    ok = h.cleanup_and_dump(summary)
    return ok


def held_output_case():
    # Copy of test_checklist_timeout_reaps_held_output_tree (:336-363) with the 3 s
    # result wait widened to 25 s and the elapsed time recorded.
    from unittest.mock import patch
    h = Harness('held_output_tree')
    sys.path.insert(0, str(BIN))
    import _owned_process as owned
    import javis_checklist as ck
    child = h.root / 'held_custom.py'
    code = "import os,time;from pathlib import Path;_marker=Path(%r);_tmp=_marker.with_name(_marker.name+'.tmp');_tmp.write_text(str(os.getpid()));__import__('os').replace(_tmp,_marker);time.sleep(30)" % str(h.root / 'held-grandchild')
    child.write_text("import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-B','-c',%r])\ntime.sleep(30)\n" % code)
    results = queue.Queue()
    clock = owned.monotonic; origin = clock(); armed = []
    def ready_clock():
        now = clock()
        if not armed and (h.root / 'held-grandchild').exists():
            armed.append(now)
        return origin + (now - armed[0] if armed else 0)
    def run():
        with patch.object(owned, 'monotonic', ready_clock), patch.object(subprocess, '_time', ready_clock):
            results.put(ck.run_preflight([PYTHON, '-B', str(child)], timeout=.5))
    threading.Thread(target=run, daemon=True).start()
    gpid, g_t = h.wait_marker('held-grandchild', h.t0 + WINDOW)
    t_seen = time.monotonic()
    try:
        result = results.get(timeout=25)
        elapsed = round(time.monotonic() - t_seen, 3)
    except queue.Empty:
        result = None; elapsed = None
    summary = {'case': 'held_output_tree', 'iter': ITER, 'grandchild_pid': gpid, 'grandchild_t': g_t,
               'result': repr(result), 'elapsed_from_grandchild_s': elapsed,
               'within_original_3s': bool(elapsed is not None and elapsed <= 3),
               'note': 'elapsed counts from the grandchild marker; the 0.5 s timeout starts there (patched clock)'}
    if gpid:
        h.kill_pid(gpid)
    h.cleanup_and_dump(summary)
    return True


def main():
    for _s in (sys.stdout, sys.stderr):  # the Windows runner's stdout is cp1252; captures carry Korean text
        _s.reconfigure(encoding='utf-8', errors='backslashreplace')
    emit('ENV', {'executable': sys.executable, 'version': sys.version.split()[0], 'platform': sys.platform,
                 'isolated': sys.flags.isolated, 'iter': ITER, 'window_s': WINDOW})
    instrument_ok = True
    instrument_ok &= pool_case('scanner_then_preflight_death', 'worker', 'scanner', 'driver')
    instrument_ok &= pool_case('scanner_death_alone', 'worker', 'scanner', 'driver')
    instrument_ok &= pool_case('checklist_deadline_pool', 'pool-worker', 'deadline-scanner', 'deadline')
    held_output_case()
    emit('END', {'iter': ITER, 'instrument_ok': bool(instrument_ok)})
    sys.exit(0 if instrument_ok else 1)


if __name__ == '__main__':
    main()
