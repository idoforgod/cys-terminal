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


def monotonic():
    """A machine-wide monotonic coordinate, including macOS Python3.9.

    time.monotonic() on that interpreter subtracts a per-process origin. A
    deadline passed between processes must use the underlying OS coordinate.
    """
    if os.name == 'nt':
        counter = ctypes.c_int64(); frequency = ctypes.c_int64()
        api = ctypes.WinDLL('kernel32', use_last_error=True)
        if not api.QueryPerformanceCounter(ctypes.byref(counter)) or not api.QueryPerformanceFrequency(ctypes.byref(frequency)):
            raise ctypes.WinError(ctypes.get_last_error())
        return counter.value / frequency.value
    return time.clock_gettime(time.CLOCK_MONOTONIC)


def remaining(deadline):
    return max(0.0, deadline - monotonic())


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
            handle = self.handle
            if not self.api.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())
            self.handle = None


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
    def __init__(self, control, lifeline_fd=0):
        # POSIX borrows fd 0; nt owns a private reader, separate from ordinary stdin.
        self._lifeline_fd = lifeline_fd
        self.group = os.getpgrp() if os.name == 'posix' else None
        if os.name == 'posix' and (self.group != os.getpid() or os.getsid(0) != os.getpid()):
            raise RuntimeError('managed child is not its own session/group leader')
        self.control = control
        self.downstream = set()
        self.lock = threading.Lock()
        self.control.write(b'READY\n')
        if os.read(self._lifeline_fd, 1) != b'G':
            os._exit(125)
        signal.signal(signal.SIGTERM, lambda *_: CANCEL.set())
        threading.Thread(target=self.watch, daemon=True, name='owned-lifeline').start()

    def watch(self):
        def read_eof():
            try:
                while os.read(self._lifeline_fd, 1):
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


def _utf8_stdio():
    # Managed diagnostic streams have an explicit wire encoding. Standalone
    # invocation does not change its caller's console/locale behavior.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='replace')


def is_managed():
    return _CONTEXT is not None


def _managed_io(is_windows):
    """OS seam for admission acquisition; fake implementations never touch runner fd 0."""
    if is_windows:
        import msvcrt
        from ctypes import wintypes as w
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.SetStdHandle.argtypes = [w.DWORD, w.HANDLE]
        kernel.SetStdHandle.restype = w.BOOL
        kernel.GetStdHandle.argtypes = [w.DWORD]
        kernel.GetStdHandle.restype = w.HANDLE
        kernel.CloseHandle.argtypes = [w.HANDLE]
        kernel.CloseHandle.restype = w.BOOL

    class IO:
        fdopen = staticmethod(os.fdopen)
        set_inheritable = staticmethod(os.set_inheritable)
        dup = staticmethod(os.dup)
        close = staticmethod(os.close)

        @staticmethod
        def control_fd(handle):
            return msvcrt.open_osfhandle(handle, os.O_WRONLY | os.O_BINARY)

        @staticmethod
        def open_null():
            return os.open(os.devnull, os.O_RDONLY | os.O_BINARY)

        @staticmethod
        def dup2(fd, dst):
            os.dup2(fd, dst, inheritable=True)  # only NUL is made an ordinary inheritable stream

        @staticmethod
        def get_osfhandle(fd):
            return msvcrt.get_osfhandle(fd)

        @staticmethod
        def set_stdin(fd):
            if not kernel.SetStdHandle(4294967286, msvcrt.get_osfhandle(fd)):
                raise ctypes.WinError(ctypes.get_last_error())

        @staticmethod
        def get_stdin():
            return kernel.GetStdHandle(4294967286)

        @staticmethod
        def close_raw_control(handle):
            if not kernel.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())
    return IO


def _unwind_managed(api, nul_fd=None, private_fd=None, control=None, raw_fd=None, raw_handle=None):
    """Attempt every untransferred close exactly once, even if an earlier close fails."""
    errors = []
    closers = [(api.close, fd) for fd in (nul_fd, private_fd) if fd is not None]
    if control is not None:
        closers.append((lambda stream: stream.close(), control))
    elif raw_fd is not None:
        closers.append((api.close, raw_fd))
    elif raw_handle is not None:
        closers.append((api.close_raw_control, raw_handle))
    for close, value in closers:
        try:
            close(value)
        except OSError as exc:
            errors.append(str(exc))
    if errors:
        raise OSError('managed setup unwind failed: ' + '; '.join(errors))


def _acquire_managed(handle, is_windows, api):
    """M1: one ownership seam for control acquisition AND nt stdin separation, before READY.

    A converted raw handle belongs to its CRT fd; an fdopen'd fd belongs to the stream.
    Failure closes each currently owned resource explicitly, never by GC. fd 0 is never
    an unwind resource: after replacement a failed admitted child exits, without rollback.
    """
    raw_handle = handle if is_windows else None
    raw_fd = None if is_windows else handle
    control = None; private_fd = None; nul_fd = None
    try:
        if is_windows:
            raw_fd = api.control_fd(raw_handle)
            raw_handle = None
        control = api.fdopen(raw_fd, 'wb', buffering=0)
        raw_fd = None
        api.set_inheritable(control.fileno(), False)
        if is_windows:
            private_fd = api.dup(0)
            # Duplicating a Windows standard stream is an inheritable-dup exception.
            api.set_inheritable(private_fd, False)
            nul_fd = api.open_null()
            api.dup2(nul_fd, 0)
            closing = nul_fd; nul_fd = None
            api.close(closing)
            # subprocess's captured-output path consults GetStdHandle, not CRT fd 0.
            api.set_stdin(0)
            if api.get_stdin() != api.get_osfhandle(0):
                raise OSError('managed stdin handle readback mismatch')
        return control, private_fd if is_windows else 0
    except BaseException:
        _unwind_managed(api, nul_fd, private_fd, control, raw_fd, raw_handle)
        raise


def managed_main(main, argv=None):
    """CLI entry; consumes private flags only in explicitly managed mode."""
    global _CONTEXT
    args = sys.argv[1:] if argv is None else argv
    # The owner appends one private trailer. Command arguments before it are
    # payload, including arguments that happen to spell our private flags.
    if len(args) < 2 or args[-2] != '--owned-lifeline':
        return main()
    _utf8_stdio()
    i = len(args)-2
    control = None; lifeline_fd = None; transferred = False
    is_windows = os.name == 'nt'
    api = _managed_io(is_windows)
    try:
        handle = int(args[i + 1])
        del args[i:i + 2]
        if argv is None:
            sys.argv[1:] = args
        control, lifeline_fd = _acquire_managed(handle, is_windows, api)
        _CONTEXT = Managed(control, lifeline_fd)
        transferred = True
        try:
            code = main()
        except SystemExit as exc:
            code = exc.code if isinstance(exc.code, int) else 1
        _CONTEXT.finish(0 if code is None else code)
    except (OSError, ValueError, IndexError) as exc:
        print('owned lifecycle unavailable: %s' % exc, file=sys.stderr)
        return 125
    finally:
        if control is not None and not transferred:
            _unwind_managed(api, private_fd=lifeline_fd if is_windows else None, control=control)


def worker_lifeline():
    """Picklable Connection: spawn duplicates its read handle for EACH worker."""
    if _CONTEXT is None:
        return None
    if os.name == 'nt':
        import msvcrt
        from multiprocessing import reduction
        handle = reduction.duplicate(msvcrt.get_osfhandle(_CONTEXT._lifeline_fd))
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


def run_owned(cmd, timeout, grace=0.0, force=5.0, cancel=None, env=None, encoding="utf-8"):
    """Bounded capture. Never poll/reap before teardown of the owned tree.

    timeout includes startup/execution; grace+force are shared cleanup allowances.
    Grace ends early once the leader is gone (control-eof): nobody is left to cooperate.
    Every launch requires private admission/completion. Explicit custom commands
    use the managed command adapter; there is no uncontained launch mode.
    stdout/stderr are decoded text; stdout_bytes/stderr_bytes are the raw bytes the
    leaf CLI relays (bytes, so the result dict is not JSON as a whole).
    """
    start = monotonic(); execution = start + timeout
    events = queue.Queue(); chunks = {'out': [], 'err': []}
    eof = set(); frame = b''; done = None; ready = False; fault = None
    proc = None; writer = None; control_read = control_write = None; job = None
    timed_out = False; teardown = False; cleanup = 'unknown'; readers_started = False
    teardown_permission = None
    cancel = cancel or CANCEL
    try:
        if os.name == 'posix' and signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
            raise RuntimeError('incompatible SIGCHLD reaper ownership')
        if cancel.is_set():
            raise RuntimeError('cancelled before launch')
        kw = dict(stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True,
                  stdin=subprocess.PIPE, env=env)
        launch = list(cmd)
        if os.name == 'posix':
            kw['start_new_session'] = True
        else:
            kw['creationflags'] = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW
            job = Job()
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
            proc = subprocess.Popen(launch, **kw)
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
        control = os.fdopen(control_read, 'rb', buffering=0); control_read = None
        threading.Thread(target=_reader, args=(control, events, 'control', None), daemon=True).start()
        readers_started = True
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
        else:
            timed_out = True
        if monotonic() >= execution:
            timed_out = True
    except Exception as exc:
        fault = str(exc)
    finally:
        if writer is not None:
            try:
                writer.close()
            except OSError:
                pass
            if _CONTEXT is not None:
                with _CONTEXT.lock:
                    _CONTEXT.downstream.discard(writer)
        expected_eof = {'out-eof', 'err-eof', 'control-eof'}
        cooperative_end = monotonic() + (grace if timed_out or fault else 0)
        # The leader closes control only by exiting (self-stop or crash), so
        # control-eof ends the window: the rest of the grace would be pure waiting.
        while (readers_started and remaining(cooperative_end) > 0 and not expected_eof <= eof
               and 'control-eof' not in eof):
            try:
                label, data = events.get(timeout=min(.02, remaining(cooperative_end)))
            except queue.Empty:
                continue
            if label.endswith('-eof'):
                eof.add(label)
            elif label == 'error':
                fault = fault or ('capture error: ' + data)
            elif label == 'control':
                fault = fault or 'control frame after cancellation'
        # A distinct absolute deadline: force/drain/reap cannot borrow grace.
        cleanup_end = monotonic() + force
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
                if os.name == 'posix' and isinstance(exc, OSError) and exc.errno == errno.EPERM:
                    # EOF may have killed the retained leader before our kill.
                    # Permission refusal is provisional, NEVER proof of absence.
                    teardown_permission = 'tree teardown: ' + str(exc)
                else:
                    fault = fault or ('tree teardown: ' + str(exc))
                if job is not None:
                    try:
                        job.close()  # kill-on-close fallback, still inside cleanup deadline
                        job = None
                    except Exception as close_exc:
                        fault += '; Job close: ' + str(close_exc)
            try:
                proc.wait(timeout=remaining(cleanup_end))
            except subprocess.TimeoutExpired:
                fault = fault or 'reap deadline'
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
            if (teardown or teardown_permission is not None) and proc.returncode is not None:
                if os.name == 'posix':
                    # Reaped group numbers have observation authority only. Give
                    # adopted zombies the remaining window to disappear; never
                    # signal a group again once its leader has been reaped.
                    while remaining(cleanup_end) > 0:
                        try:
                            os.killpg(proc.pid, 0)
                        except ProcessLookupError:
                            if teardown_permission is None or (
                                    done is not None and not fault and not timed_out
                                    and not cancel.is_set() and expected_eof <= eof):
                                cleanup = 'verified_empty'
                            break
                        except OSError as exc:
                            fault = fault or ('group observation: ' + str(exc))
                            break
                        time.sleep(min(.01, remaining(cleanup_end)))
                    if cleanup != 'verified_empty':
                        fault = fault or 'group absence not verified before cleanup deadline'
                else:
                    cleanup = 'verified_empty'
            if teardown_permission is not None and cleanup != 'verified_empty':
                fault = (fault + '; ' if fault else '') + teardown_permission
        if job is not None:
            try:
                job.close()
            except Exception as exc:
                fault = fault or ('Job close: ' + str(exc))
        for fd in (control_read, control_write):
            if fd is not None:
                os.close(fd)
    complete = done is not None and not fault and not timed_out and cleanup == 'verified_empty'
    out, err = b''.join(chunks['out']), b''.join(chunks['err'])
    return {'returncode': done if complete else (proc.returncode if proc else None),
            'diagnostic_returncode': done,
            'kernel_returncode': proc.returncode if proc else None,
            'stdout': out.decode(encoding, 'replace'),
            'stderr': err.decode(encoding, 'replace'),
            'stdout_bytes': out, 'stderr_bytes': err,
            'timed_out': timed_out, 'complete': complete, 'cleanup': cleanup,
            'reason': fault or '', 'elapsed': monotonic() - start}


def command_main():
    """Private execution adapter for an explicit custom command, never eval data."""
    args = sys.argv[1:]
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
    _utf8_stdio()
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
    ap.add_argument('--native-shell', action='store_true')
    ap.add_argument('command', nargs=argparse.REMAINDER)
    args = ap.parse_args()
    started = monotonic()
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
                chunks = []
                # A blocked BufferedReader owns a lock that aborts CPython at
                # shutdown. Raw reads let this daemon thread end with its owner.
                fd = sys.stdin.fileno()
                while True:
                    chunk = os.read(fd, 65536)
                    if not chunk:
                        break
                    chunks.append(chunk)
                inputs.put(b''.join(chunks))
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
    if result is None and args.native_shell and os.name == 'nt':
        # MSYS shell paths are not native CreateProcess executable paths. The
        # converter is itself owned and charged to this same phase deadline.
        import shutil
        converter = shutil.which('cygpath')
        available = max(0.0, cap-(monotonic()-started))
        reserve = min(1.0, available/10)
        if converter and available > reserve and command:
            native = run_owned([sys.executable, '-B', os.path.abspath(__file__),
                                '--command', converter, '-w', command[0]],
                               min(2.0, available-reserve), force=reserve, env=env)
            path = native['stdout'].rstrip('\r\n')
            if native['complete'] and native['returncode'] == 0 and path and '\n' not in path and '\r' not in path:
                command[0] = path
            else:
                result = {'complete': False, 'stdout': '', 'stderr': native['stderr'],
                          'reason': 'native shell path unavailable', 'timed_out': native['timed_out']}
        else:
            result = {'complete': False, 'stdout': '', 'stderr': '',
                      'reason': 'native shell converter unavailable/budget exhausted', 'timed_out': available <= reserve}
    left = cap - (monotonic()-started)
    reserve = min(1.0, max(0.0, left/10))
    if result is None and left > reserve:
        if not args.managed_command:
            command = [sys.executable, '-B', os.path.abspath(__file__), '--command'] + command
        result = run_owned(command, left-reserve, force=reserve, env=env)
    if result is None:
        result = {'complete': False, 'stdout': '', 'stderr': '',
                  'reason': 'phase budget exhausted; unmeasured', 'timed_out': True}
    elapsed = monotonic()-started
    # Relay the child's bytes as written (H-WIN-17): a text-mode stdout on Windows writes LF as CR LF, and records
    # pass this leaf once per phase, so every pass added a CR; decoding also turned invalid UTF-8 into U+FFFD.
    # Only our own framing and diagnostics are encoded here.
    out = result.get('stdout_bytes', result['stdout'].encode('utf-8'))
    write = sys.stdout.buffer.write
    if args.records:
        if result['complete'] and result.get('returncode') == 0:
            write(out)
        else:
            # A complete key/value pair is a committed phase checkpoint. Keep
            # it even if a subsequent file read stalls; drop any partial pair.
            fields = out.split(b'\0')
            for i in range(0, len(fields)-2, 2):
                write(fields[i]+b'\0'+fields[i+1]+b'\0')
            write(('_OWNED_FAILURE\0%s\0' % (result['reason'] or 'timeout/incomplete')).encode('utf-8', 'replace'))
        write(('_OWNED_STARTED\0%.9f\0_OWNED_ELAPSED\0%.9f\0' % (origin, elapsed)).encode('ascii'))
        write((('_OWNED_SPENT\0%.9f\0_OWNED_RETURNED\0' + '1\0') % (args.spent+elapsed)).encode('ascii'))
    else:
        write(out)
    sys.stdout.flush()
    sys.stderr.buffer.write(result.get('stderr_bytes', result['stderr'].encode('utf-8')))
    if not result['complete']:
        sys.stderr.buffer.write(('owned execution incomplete: %s\n' % (result['reason'] or 'deadline')).encode('utf-8', 'replace'))
    sys.stderr.flush()
    return (result.get('returncode') or 0) if result['complete'] else (124 if result['timed_out'] else 125)


if __name__ == '__main__':
    # runpy-loaded tools and spawn workers must share this exact context instance.
    sys.modules['_owned_process'] = sys.modules[__name__]
    if sys.argv[1:2] == ['--bounded']:
        sys.exit(bounded_cli())
    if len(sys.argv) >= 3 and sys.argv[-2] == '--owned-lifeline':
        sys.exit(managed_main(command_main))
    print('owned adapter requires admission', file=sys.stderr)
    sys.exit(125)
