#!/usr/bin/env python3
"""C-03 win32 diagnostic RUN 2 - diag branch only, never merged or shipped.

Run 1 showed the pool worker dying in multiprocessing spawn_main (DuplicateHandle of
the parent's pipe: WinError 5) and the executor not noticing. Run 2 isolates
(U1) which launch attribute of the scanner causes it and (U2) whether the worker
dies or hangs, with a watcher that has its own positive control (cell K).

Cells (scanner fixture identical to run 1; only the launch differs):
  a  run_owned as shipped by C-03 (Job, CREATE_NEW_PROCESS_GROUP|CREATE_NO_WINDOW,
     handle list, stdin = lifeline pipe with a pending read, worker lifeline)
  b  the PIN's launch (0.14.55 javis_preflight _run_skillscan_all): plain Popen,
     stdout/stderr PIPE, stdin DEVNULL, CREATE_NEW_PROCESS_GROUP, no Job; unmanaged
  c  a without the Job assignment          d  a without CREATE_NO_WINDOW
  e  a without CREATE_NEW_PROCESS_GROUP    f  a with the worker lifeline off
  g  b but stdin = PIPE (kept open) and a thread in the scanner blocked reading it
  K  b, then the harness kills the live pool worker: the watcher must see it
It reports; it does not judge. Exit 1 only if no cell produced scanner events.
"""
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I/._pth drops the script dir
import diag_c03_win as r1  # noqa: E402

WINDOW = float(os.environ.get('C03_DIAG_WINDOW', '35'))
ITER = sys.argv[1] if len(sys.argv) > 1 else '1'
r1.ITER = ITER

WATCH = r'''
def _watch(ex):
    seen = {}; known = {}
    end = time.monotonic() + 40
    while time.monotonic() < end:
        try:
            procs = dict(getattr(ex, '_processes', None) or {})
        except Exception as e:
            procs = {}; ev('watch-error', err=repr(e))
        known.update(procs)  # keep every process object ever seen: the executor drops dead ones
        for pid, p in list(known.items()):
            try:
                st = [p.is_alive(), p.exitcode]
            except Exception as e:
                st = ['err', repr(e)]
            if seen.get(pid) != st:
                seen[pid] = st
                ev('worker-state', wpid=pid, alive=st[0], exitcode=st[1])
        b = getattr(ex, '_broken', False)
        if b and not seen.get('broken'):
            seen['broken'] = True
            ev('executor-broken', cause=str(b)[:300])
        time.sleep(0.05)
def wrap_scanner2(scanner, lifeline_off=False, stdin_reader=False):
    if lifeline_off:
        scanner.pool_initializer_args = lambda: (None, None)
    wrap_scanner(scanner)
    import concurrent.futures as cf
    P = cf.ProcessPoolExecutor
    om = P.map
    def m(self, *a, **k):
        threading.Thread(target=_watch, args=(self,), daemon=True, name='diag-watch').start()
        return om(self, *a, **k)
    P.map = m
    if stdin_reader:
        def rd():
            try:
                ev('stdin-read-start'); os.read(0, 1); ev('stdin-read-returned')
            except OSError as e:
                ev('stdin-read-error', err=repr(e))
        threading.Thread(target=rd, daemon=True, name='diag-stdin').start()
        time.sleep(0.2)
'''


class H2(r1.Harness):
    def __init__(self, name):
        super().__init__(name)
        (self.root / '_c03diag.py').write_text(r1.DIAG_MOD + WATCH)


def scan_text2(h, lifeline_off, stdin_reader):
    t = r1.scan_text(h, 'worker', 'scanner', h.root / 'skills', 'try')
    old = "_c03diag.wrap_scanner(scanner) if __name__=='__main__' else None  # DIAG\n"
    assert t.count(old) == 1
    new = ("_c03diag.wrap_scanner2(scanner,%r,%r) if __name__=='__main__' else None  # DIAG\n"
           % (lifeline_off, stdin_reader))
    return t.replace(old, new)


OWNED_DRIVER = ("import sys\nsys.path.insert(0,%r)\n%s"
                "from javis_preflight import Preflight\nr=Preflight._run_skillscan_all(%r,20)\n"
                "open(%r,'w',encoding='utf-8').write(repr(r))  # DIAG\n")
PIN_DRIVER = ("import os,subprocess,sys\nkw={}\n"
              "if os.name=='posix': kw['start_new_session']=True\n"
              "else: kw['creationflags']=getattr(subprocess,'CREATE_NEW_PROCESS_GROUP',0)\n"
              "p=subprocess.Popen([sys.executable,%r],stdout=subprocess.PIPE,stderr=subprocess.PIPE,stdin=subprocess.%s,text=True,**kw)\n"
              "try:\n out,err=p.communicate(timeout=20);r=(p.returncode,out,err,False)\n"
              "except subprocess.TimeoutExpired:\n p.kill();out,err=p.communicate();r=(p.returncode,out,err,True)\n"
              "open(%r,'w',encoding='utf-8').write(repr(r))\n")
PATCH = {
    'c': "import _owned_process as _op\n_op.Job.assign=lambda self,proc: None  # DIAG cell c\n",
    'd': "import subprocess\nsubprocess.CREATE_NO_WINDOW=0  # DIAG cell d\n",
    'e': "import subprocess\nsubprocess.CREATE_NEW_PROCESS_GROUP=0  # DIAG cell e\n",
}


def wait_ev(h, pid, pred, until):
    path = h.root / ('ev-%d.jsonl' % pid)
    while time.monotonic() < until:
        try:
            for x in path.read_text().splitlines():
                try:
                    e = json.loads(x)
                except ValueError:
                    continue
                if pred(e):
                    return round(time.monotonic() - h.t0, 3)
        except FileNotFoundError:
            pass
        time.sleep(.02)
    return None


def cell(name):
    h = H2('cell-' + name)
    skill = h.root / 'skills' / 'sample'; skill.mkdir(parents=True)
    (skill / 'SKILL.md').write_text('# fixture')
    scan = h.root / 'scan_pool.py'
    scan.write_text(scan_text2(h, lifeline_off=(name == 'f'), stdin_reader=(name == 'g')))
    res = str(h.root / 'driver-result.txt')
    if name in ('b', 'g', 'K'):
        p = h.start(PIN_DRIVER % (str(scan), 'PIPE' if name == 'g' else 'DEVNULL', res))
    else:
        p = h.start(OWNED_DRIVER % (str(r1.BIN), PATCH.get(name, ''), str(scan), res))
    end = h.t0 + WINDOW
    s_pid, s_t = h.wait_marker('scanner', end)
    w_pid, w_t = h.wait_marker('worker', end) if s_pid else (None, None)
    killed = seen_alive = seen_dead = None
    if w_pid and w_pid != s_pid:
        # The watcher must SEE the live worker (all TASK cells); in K it must also see it die.
        seen_alive = wait_ev(h, s_pid, lambda e: e.get('ev') == 'worker-state' and e.get('wpid') == w_pid
                             and e.get('alive') is True, time.monotonic() + 5)
        if name == 'K' and seen_alive is not None:
            h.kill_pid(w_pid); killed = round(time.monotonic() - h.t0, 3)
            seen_dead = wait_ev(h, s_pid, lambda e: e.get('ev') == 'worker-state' and e.get('wpid') == w_pid
                                and e.get('exitcode') is not None, time.monotonic() + 10)
            time.sleep(3)  # let the executor react (broken pool / serial fallback) and be recorded
    elif w_pid is None:
        while p.poll() is None and time.monotonic() < end:
            time.sleep(.05)
    summary = {'case': 'cell-' + name, 'iter': ITER, 'window_s': WINDOW, 'scanner_pid': s_pid, 'scanner_t': s_t,
               'worker_pid': w_pid, 'worker_t': w_t, 'killed_worker_t': killed,
               'watcher_saw_alive_t': seen_alive, 'watcher_saw_dead_t': seen_dead,
               'same_pid_serial_fallback': bool(w_pid and w_pid == s_pid),
               'driver_rc': p.poll(), 'driver_exited': p.poll() is not None}
    return h.cleanup_and_dump(summary)


def main():
    r1.emit('ENV', {'executable': sys.executable, 'version': sys.version.split()[0], 'platform': sys.platform,
                    'isolated': sys.flags.isolated, 'iter': ITER, 'window_s': WINDOW, 'run': 2})
    ok = False
    for name in ('K', 'b', 'g', 'a', 'c', 'd', 'e', 'f'):
        ok |= cell(name)
    r1.emit('END', {'iter': ITER, 'instrument_ok': bool(ok), 'run': 2})
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
