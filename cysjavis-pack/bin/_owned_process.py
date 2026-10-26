"""Opt-in owned child lifecycle. No discovery, RPC, or process-name termination.

The parent retains Popen until tree teardown, and keeps stdin outside communicate.
READY / GO / DONE is private: diagnostic stdout cannot manufacture completion.
A child holds its group identity until release. Standalone imports arm nothing.
"""
import ctypes
import errno
import json
import os
import queue
import signal
import subprocess
import sys
import threading
import time
from multiprocessing.connection import Connection

CANCEL = threading.Event()
_CONTEXT = None


def remaining(deadline):
    return max(0.0, deadline - time.monotonic())


class Job:
    """One unnamed Windows Job, owned only by the launching parent."""
    def __init__(self):
        from ctypes import wintypes as w
        class Basic(ctypes.Structure):
            _fields_ = [('PerProcessUserTimeLimit', ctypes.c_int64),
                        ('PerJobUserTimeLimit', ctypes.c_int64), ('LimitFlags', w.DWORD),
                        ('MinimumWorkingSetSize', ctypes.c_size_t),
                        ('MaximumWorkingSetSize', ctypes.c_size_t), ('ActiveProcessLimit', w.DWORD),
                        ('Affinity', ctypes.c_size_t), ('PriorityClass', w.DWORD),
                        ('SchedulingClass', w.DWORD)]
        class IO(ctypes.Structure):
            _fields_ = [(n, ctypes.c_uint64) for n in
                        ('ReadOperationCount', 'WriteOperationCount', 'OtherOperationCount',
                         'ReadTransferCount', 'WriteTransferCount', 'OtherTransferCount')]
        class Extended(ctypes.Structure):
            _fields_ = [('BasicLimitInformation', Basic), ('IoInfo', IO),
                        ('ProcessMemoryLimit', ctypes.c_size_t), ('JobMemoryLimit', ctypes.c_size_t),
                        ('PeakProcessMemoryUsed', ctypes.c_size_t), ('PeakJobMemoryUsed', ctypes.c_size_t)]
        self.api = ctypes.WinDLL('kernel32', use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, w.LPCWSTR]
        self.api.CreateJobObjectW.restype = w.HANDLE
        self.api.SetInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD]
        self.api.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
        self.api.TerminateJobObject.argtypes = [w.HANDLE, w.UINT]
        self.api.QueryInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD, ctypes.c_void_p]
        self.api.CloseHandle.argtypes = [w.HANDLE]
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        info = Extended(); info.BasicLimitInformation.LimitFlags = 0x2000
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(info), ctypes.sizeof(info)):
            error = ctypes.WinError(ctypes.get_last_error()); self.close(); raise error

    def assign(self, proc):
        if not self.api.AssignProcessToJobObject(self.handle, int(proc._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def terminate(self):
        if not self.api.TerminateJobObject(self.handle, 124):
            raise ctypes.WinError(ctypes.get_last_error())

    def empty(self):
        from ctypes import wintypes as w
        class Accounting(ctypes.Structure):
            _fields_ = [('TotalUserTime', ctypes.c_int64), ('TotalKernelTime', ctypes.c_int64),
                        ('ThisPeriodTotalUserTime', ctypes.c_int64), ('ThisPeriodTotalKernelTime', ctypes.c_int64),
                        ('TotalPageFaultCount', w.DWORD), ('TotalProcesses', w.DWORD),
                        ('ActiveProcesses', w.DWORD), ('TotalTerminatedProcesses', w.DWORD)]
        info = Accounting()
        if not self.api.QueryInformationJobObject(self.handle, 1, ctypes.byref(info), ctypes.sizeof(info), None):
            raise ctypes.WinError(ctypes.get_last_error())
        return info.ActiveProcesses == 0

    def close(self):
        if self.handle:
            handle, self.handle = self.handle, None
            if not self.api.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())


def _self_stop(group):
    if os.name == 'posix':
        # Workers may only target the scanner group validated by their owner.
        if os.getpgrp() != group:
            os._exit(125)
        os.killpg(group, signal.SIGKILL)
    else:
        # Parent-owned Job closure is the Windows tree authority.
        os._exit(125)


class Managed:
    def __init__(self, control):
        self.group = os.getpgrp() if os.name == 'posix' else None
        if os.name == 'posix' and (self.group != os.getpid() or os.getsid(0) != os.getpid()):
            raise RuntimeError('managed child is not its own session/group leader')
        self.control = control
        self.downstream = set()
        self.lock = threading.Lock()
        self.control.write(b'READY\n')
        if os.read(0, 1) != b'G':
            os._exit(125)
        signal.signal(signal.SIGTERM, lambda *_: CANCEL.set())
        threading.Thread(target=self.watch, daemon=True, name='owned-lifeline').start()

    def watch(self):
        def read_eof():
            try:
                while os.read(0, 1):
                    pass
            except OSError:
                pass
            CANCEL.set()
        threading.Thread(target=read_eof, daemon=True, name='owned-eof').start()
        CANCEL.wait()
        with self.lock:
            writers = list(self.downstream)
        for writer in writers:
            try:
                writer.close()
            except OSError:
                pass
        # Close downstream writers before killing this group. Their EOF watchers
        # own their separate groups, including a scanner's read-side workers.
        _self_stop(self.group)

    def finish(self, code):
        sys.stdout.flush(); sys.stderr.flush()
        if not CANCEL.is_set():
            self.control.write(('DONE %d\n' % code).encode('ascii'))
        # Stay unreaped until parent releases ownership (EOF/tree teardown).
        threading.Event().wait()


def managed_main(main, argv=None, report_only=False):
    """CLI entry; consumes private flags only in explicitly managed mode."""
    global _CONTEXT
    args = sys.argv[1:] if argv is None else argv
    if '--owned-lifeline' not in args:
        return main()
    i = args.index('--owned-lifeline')
    try:
        handle = int(args[i + 1])
        del args[i:i + 2]
        if argv is None:
            sys.argv[1:] = args
        if report_only and any(x in args for x in ('--fix', '--safe', '--dry-run', '--seed-trust', '--self-test')):
            return 125
        if os.name == 'nt':
            import msvcrt
            handle = msvcrt.open_osfhandle(handle, os.O_WRONLY | os.O_BINARY)
        control = os.fdopen(handle, 'wb', buffering=0)
        os.set_inheritable(control.fileno(), False)
        _CONTEXT = Managed(control)
        try:
            code = main()
        except SystemExit as exc:
            code = exc.code if isinstance(exc.code, int) else 1
        _CONTEXT.finish(0 if code is None else code)
    except (OSError, ValueError, IndexError) as exc:
        print('owned lifecycle unavailable: %s' % exc, file=sys.stderr)
        return 125


def worker_lifeline():
    """Picklable Connection: spawn duplicates its read handle for EACH worker."""
    if _CONTEXT is None:
        return None
    if os.name == 'nt':
        import msvcrt
        from multiprocessing import reduction
        handle = reduction.duplicate(msvcrt.get_osfhandle(0))
        from multiprocessing.connection import PipeConnection
        return PipeConnection(handle, readable=True, writable=False)
    return Connection(os.dup(0), readable=True, writable=False)


def worker_observer(conn, group):
    if conn is None:
        return
    # Only fork inherits this context. A numeric parent fd must NEVER be
    # closed in a spawned worker: that number may identify an unrelated pipe.
    if os.name == 'posix' and _CONTEXT is not None:
        _CONTEXT.control.close()
        for writer in _CONTEXT.downstream:
            writer.close()
    if os.name == 'posix' and os.getpgrp() != group:
        os._exit(125)
    def watch():
        fd = conn.fileno()
        if os.name == 'nt':
            import msvcrt
            from multiprocessing import reduction
            # PipeConnection is the spawn transport, not a framed reader. This
            # anonymous lifeline carries no frames after GO; read until EOF.
            fd = msvcrt.open_osfhandle(reduction.duplicate(fd), os.O_RDONLY | os.O_BINARY)
        try:
            while os.read(fd, 1):
                pass
        except OSError:
            pass
        finally:
            if os.name == 'nt':
                os.close(fd)
        _self_stop(group)
    threading.Thread(target=watch, daemon=True, name='pool-lifeline').start()


def pool_initializer_args():
    if _CONTEXT is None:
        return (None, None)
    return (worker_lifeline(), _CONTEXT.group)


def _reader(stream, events, label, chunks):
    try:
        while True:
            data = os.read(stream.fileno(), 65536)
            if not data:
                break
            if chunks is not None:
                chunks.append(data)
            else:
                events.put((label, data))
    except OSError as exc:
        events.put(('error', str(exc)))
    finally:
        stream.close()
        events.put((label + '-eof', None))


def run_owned(cmd, timeout, grace=0.0, force=5.0, cancel=None, managed=True, shell=False, env=None):
    """Bounded capture. Never poll/reap before teardown of the owned tree.

    timeout includes startup/execution; grace+force are shared cleanup allowances.
    managed=False preserves explicit custom command exit semantics, but cannot
    claim private completion proof. Such commands still get bounded group cleanup.
    """
    start = time.monotonic(); execution = start + timeout
    events = queue.Queue(); chunks = {'out': [], 'err': []}
    eof = set(); frame = b''; done = None; ready = False; fault = None
    proc = None; writer = None; control_read = control_write = None; job = None
    timed_out = False; teardown = False; cleanup = 'unknown'
    cancel = cancel or CANCEL
    try:
        if os.name == 'posix' and signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
            raise RuntimeError('incompatible SIGCHLD reaper ownership')
        if cancel.is_set():
            raise RuntimeError('cancelled before launch')
        kw = dict(stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True,
                  stdin=subprocess.PIPE if managed else subprocess.DEVNULL, env=env)
        launch = list(cmd) if not shell else cmd
        if os.name == 'posix':
            kw['start_new_session'] = True
        else:
            kw['creationflags'] = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW
            job = Job()
        if managed:
            control_read, control_write = os.pipe()
            os.set_inheritable(control_read, False)
            if os.name == 'posix':
                kw['pass_fds'] = (control_write,)
                handle = control_write
            else:
                import msvcrt
                handle = msvcrt.get_osfhandle(control_write)
                os.set_handle_inheritable(handle, True)
                si = subprocess.STARTUPINFO()
                si.lpAttributeList = {'handle_list': [handle]}
                kw['startupinfo'] = si
            launch += ['--owned-lifeline', str(handle)]
        try:
            proc = subprocess.Popen(launch, shell=shell, **kw)
        finally:
            if control_write is not None:
                os.close(control_write); control_write = None
        writer = proc.stdin
        proc.stdin = None  # communicate must NEVER close the retained lifeline
        if writer is not None and _CONTEXT is not None:
            with _CONTEXT.lock:
                _CONTEXT.downstream.add(writer)
        if job is not None:
            job.assign(proc)  # child is waiting: no GO before containment
        for label, stream in [('out', proc.stdout), ('err', proc.stderr)]:
            threading.Thread(target=_reader, args=(stream, events, label, chunks[label]), daemon=True).start()
        if managed:
            control = os.fdopen(control_read, 'rb', buffering=0); control_read = None
            threading.Thread(target=_reader, args=(control, events, 'control', None), daemon=True).start()
        while remaining(execution) > 0:
            if cancel.is_set():
                fault = 'cancelled'; break
            try:
                label, data = events.get(timeout=min(.02, remaining(execution)))
            except queue.Empty:
                continue
            if label.endswith('-eof'):
                eof.add(label)
                if label == 'control-eof' and done is None:
                    fault = 'control closed without DONE'; break
            elif label == 'error':
                fault = 'capture error: ' + data; break
            elif label == 'control':
                frame += data
                if len(frame) > 256:
                    fault = 'oversized control frame'; break
                while b'\n' in frame:
                    line, frame = frame.split(b'\n', 1)
                    if line == b'READY' and not ready and done is None:
                        ready = True
                        if cancel.is_set() or not remaining(execution):
                            fault = 'cancelled before GO'; break
                        writer.write(b'G'); writer.flush()
                    elif line.startswith(b'DONE ') and ready and done is None:
                        try:
                            value = line[5:]
                            if not value.isdigit() or not 0 <= int(value) <= 255:
                                raise ValueError()
                            done = int(value)
                        except ValueError:
                            fault = 'malformed DONE'
                    else:
                        fault = 'invalid or duplicate control frame'
                if fault or done is not None:
                    break
            if not managed and {'out-eof', 'err-eof'} <= eof:
                break
        else:
            timed_out = True
        if time.monotonic() >= execution:
            timed_out = True
    except Exception as exc:
        fault = str(exc)
    finally:
        cleanup_end = time.monotonic() + (grace if timed_out or fault else 0) + force
        if writer is not None:
            try:
                writer.close()
            except OSError:
                pass
            if _CONTEXT is not None:
                with _CONTEXT.lock:
                    _CONTEXT.downstream.discard(writer)
        if proc is not None:
            # Group leader is retained even on normal DONE. No poll/wait above.
            try:
                if job is not None:
                    job.terminate()
                    while remaining(cleanup_end) > 0 and not job.empty():
                        time.sleep(min(.01, remaining(cleanup_end)))
                    if not job.empty():
                        raise RuntimeError('Job still has active processes')
                    job.close(); job = None
                elif os.name == 'posix':
                    os.killpg(proc.pid, signal.SIGKILL)
                else:
                    proc.kill()
                teardown = True
            except ProcessLookupError:
                teardown = True
            except Exception as exc:
                fault = fault or ('tree teardown: ' + str(exc))
            try:
                proc.wait(timeout=remaining(cleanup_end))
            except subprocess.TimeoutExpired:
                fault = fault or 'reap deadline'
            expected_eof = {'out-eof', 'err-eof'} | ({'control-eof'} if managed else set())
            if frame:
                fault = fault or 'partial control frame'
            while remaining(cleanup_end) > 0 and not expected_eof <= eof:
                try:
                    label, data = events.get(timeout=min(.02, remaining(cleanup_end)))
                except queue.Empty:
                    continue
                if label.endswith('-eof'):
                    eof.add(label)
                elif label == 'error':
                    fault = fault or ('capture error: ' + data)
                elif label == 'control':
                    fault = fault or 'extra terminal control bytes'
            if not expected_eof <= eof:
                fault = fault or 'drain deadline'
            if teardown and proc.returncode is not None:
                if os.name == 'posix':
                    try:
                        os.killpg(proc.pid, 0)  # observation only after reap
                    except ProcessLookupError:
                        cleanup = 'verified_empty'
                    except OSError:
                        pass
                else:
                    cleanup = 'verified_empty'
        if job is not None:
            job.close()
        for fd in (control_read, control_write):
            if fd is not None:
                os.close(fd)
    complete = managed and done is not None and not fault and not timed_out and cleanup == 'verified_empty'
    return {'returncode': done if complete else (proc.returncode if proc else None),
            'stdout': b''.join(chunks['out']).decode('utf-8', 'replace'),
            'stderr': b''.join(chunks['err']).decode('utf-8', 'replace'),
            'timed_out': timed_out, 'complete': complete, 'cleanup': cleanup,
            'reason': fault or '', 'elapsed': time.monotonic() - start}


def command_main():
    """Private execution adapter for an explicit custom command, never eval data."""
    args = sys.argv[1:]
    if args[0] == '--python':
        import runpy
        script, argv = args[1], args[1:]
        sys.argv = argv
        sys.path.insert(0, os.path.dirname(os.path.abspath(script)))
        runpy.run_path(script, run_name='__main__')
        return 0
    if args[0] == '--shell':
        return subprocess.call(args[1], shell=True, stdin=subprocess.DEVNULL, close_fds=True)
    if args[0] == '--command':
        return subprocess.call(args[1:], stdin=subprocess.DEVNULL, close_fds=True)
    return 125


def bounded_cli():
    """Opt-in timeout interface; NUL records are data, never shell source.

    An absolute start and excluded interval let a caller share one monotonic
    budget across multiple phases without resetting it at each subprocess.
    """
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument('--bounded', type=float, required=True)
    ap.add_argument('--records', action='store_true')
    ap.add_argument('--input-env')
    ap.add_argument('--start', type=float)
    ap.add_argument('--exclude', type=float, default=0)
    ap.add_argument('--budget', type=float, default=float('inf'))
    ap.add_argument('--total-budget', type=float, default=float('inf'))
    ap.add_argument('--spent', type=float, default=0)
    ap.add_argument('--managed-command', action='store_true')
    ap.add_argument('command', nargs=argparse.REMAINDER)
    args = ap.parse_args()
    started = time.monotonic()
    origin = args.start if args.start is not None else started
    cap = min(max(0, args.bounded-args.spent), max(0, args.budget - (started-origin-args.exclude)),
              max(0, args.total_budget-(started-origin)))
    command = args.command
    if command and command[0] == '--':
        command = command[1:]
    env = os.environ.copy()
    result = None
    if args.input_env:
        inputs = queue.Queue()
        def read_input():
            try:
                inputs.put(sys.stdin.buffer.read())
            except OSError as exc:
                inputs.put(exc)
        threading.Thread(target=read_input, daemon=True).start()
        try:
            data = inputs.get(timeout=max(0, cap-min(1,cap/10)))
            if isinstance(data, Exception) or b'\0' in data:
                raise ValueError('invalid input')
            env[args.input_env] = data.decode('utf-8', 'replace')
        except (queue.Empty, ValueError):
            result = {'complete': False, 'stdout': '', 'stderr': '',
                      'reason': 'input incomplete (EOF/read deadline)', 'timed_out': True}
    left = cap - (time.monotonic()-started)
    reserve = min(1.0, max(0.0, left/10))
    if result is None and left > reserve:
        if not args.managed_command:
            command = [sys.executable, '-B', os.path.abspath(__file__), '--command'] + command
        result = run_owned(command, left-reserve, force=reserve, env=env)
    if result is None:
        result = {'complete': False, 'stdout': '', 'stderr': '',
                  'reason': 'phase budget exhausted; unmeasured', 'timed_out': True}
    elapsed = time.monotonic()-started
    if args.records:
        if result['complete'] and result.get('returncode') == 0:
            sys.stdout.write(result['stdout'])
        else:
            sys.stdout.write('_OWNED_FAILURE\0%s\0' % (result['reason'] or 'timeout/incomplete'))
        sys.stdout.write('_OWNED_STARTED\0%.9f\0_OWNED_ELAPSED\0%.9f\0' % (origin, elapsed))
        sys.stdout.write(('_OWNED_SPENT\0%.9f\0_OWNED_RETURNED\0' + '1\0') % (args.spent+elapsed))
    else:
        sys.stdout.write(result['stdout'])
    sys.stderr.write(result['stderr'])
    if not result['complete']:
        print('owned execution incomplete: ' + (result['reason'] or 'deadline'), file=sys.stderr)
    return (result.get('returncode') or 0) if result['complete'] else (124 if result['timed_out'] else 125)


if __name__ == '__main__':
    # runpy-loaded tools and spawn workers must share this exact context instance.
    sys.modules['_owned_process'] = sys.modules[__name__]
    sys.exit(bounded_cli() if '--bounded' in sys.argv[1:] else managed_main(command_main))
