#!/usr/bin/env python3
"""javis_lock.py — 공용 크로스플랫폼 락 + 원자 쓰기 헬퍼 (T-0147-7 W1a · CS-5② · A8py).

문제(A8·R1 재검증):
  ① `javis_bootstrap.py` 싱글플라이트가 `fcntl.flock` 단독이라 **Windows에서 직렬화가 전무**했다
     (락을 못 잡는 게 아니라 '항상 획득'으로 접혀 중복 부트가 무장). 지배 실패 모드는 RMW 유실이
     아니라 '공유 .tmp 교차 파손 → 등록부 수리 거부' + '중복 스폰'이다.
  ② 락 파일에 **보유자 신원이 없어** 죽은 보유자를 회수할 수단이 없었다 — 락을 잡은 부트가
     비정상 종료하는 백엔드(pidfile 폴백)에서 최대 ~330s(냉시작 예산)만큼 신규 부트가 거부되는
     창이 열렸다(R1).

설계:
  · 백엔드 3종을 **가용성 기반**으로 고른다: posix=`fcntl.flock` / windows=`msvcrt.locking` /
    둘 다 없으면 `O_EXCL` pidfile. 앞 둘은 OS가 프로세스 종료 시 자동 해제하므로 스테일이
    구조적으로 없고, pidfile 폴백만 **보유 pid 사망 검사로 회수**한다.
  · 어느 백엔드든 락 파일 본문에 `{"pid":…, "started":…, "owner":…}` 를 남긴다 — 회수 판정과
    진단(누가 붙잡고 있나)의 근거. 기록 실패는 락 획득을 깨지 않는다(best-effort).
  · 판정은 **타입드 3상**이다(CS-2 정합): `acquired` / `busy` / `unavailable`.
    - `acquired`  = 이 프로세스가 단독 소유
    - `busy`      = 살아있는 다른 보유자 존재 → 호출측 no-op
    - `unavailable`= 락 자체를 세울 수 없음(디렉터리 생성 불가 등) → 호출측 **보수적 진행**
    'busy'와 'unavailable'을 융합하면(구 코드) 락 인프라 고장이 조용한 부트 거부가 된다.
  · 컨텍스트 매니저 지원. 비차단 기본(blocking=False) — 부트 훅은 대기하면 안 된다
    (수렴 대기 금지 = 재감사 금지 방향 ⑨).

테스트 전용 스위치: `CYS_LOCK_BACKEND=auto|fcntl|msvcrt|pidfile` (검체 H-CONC-6가 pidfile
  폴백의 스테일 회수를 macOS에서 결정론 재현할 때만 사용한다 — 운영은 auto).

stdlib만 사용. 네트워크 0. import 부작용 0.
"""
import errno
import json
import os
import sys
import tempfile
import time

__all__ = [
    "ACQUIRED", "BUSY", "UNAVAILABLE", "FileLock", "LockError",
    "atomic_write_bytes", "atomic_write_text", "atomic_write_json",
    "backend_name", "pid_alive", "read_holder",
]

ACQUIRED = "acquired"
BUSY = "busy"
UNAVAILABLE = "unavailable"


class LockError(RuntimeError):
    """컨텍스트 매니저 진입 시 acquired가 아닐 때 — status를 실어 던진다."""

    def __init__(self, status, path, detail=""):
        self.status = status
        self.path = path
        self.detail = detail
        super().__init__("lock %s: %s%s" % (status, path, (" — " + detail) if detail else ""))


# ── 백엔드 감지 ───────────────────────────────────────────────────────────────
try:
    import fcntl as _fcntl
except ImportError:  # windows
    _fcntl = None
try:
    import msvcrt as _msvcrt
except ImportError:  # posix
    _msvcrt = None


def backend_name():
    """실제로 쓰일 백엔드 이름('fcntl'|'msvcrt'|'pidfile'). env 강제는 테스트 전용."""
    forced = (os.environ.get("CYS_LOCK_BACKEND") or "auto").strip().lower()
    if forced in ("fcntl", "msvcrt", "pidfile"):
        if forced == "fcntl" and _fcntl is None:
            return "pidfile"
        if forced == "msvcrt" and _msvcrt is None:
            return "pidfile"
        return forced
    if _fcntl is not None:
        return "fcntl"
    if _msvcrt is not None:
        return "msvcrt"
    return "pidfile"


# ── pid 생존 판정 ─────────────────────────────────────────────────────────────
def pid_alive(pid):
    """보유 pid 생존 여부. **판정불가는 '생존'으로 접는다**(보수) — 살아있는 보유자의 락을
    오회수해 중복 부트를 만드는 것이 유한 거부 창보다 훨씬 나쁘다(A8 재검증의 '중복 스폰')."""
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return False
    if pid <= 0:
        return False
    if os.name == "posix":
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True          # 남의 uid — 살아있다
        except OSError as e:
            return e.errno != errno.ESRCH
        return True
    # windows: OpenProcess + GetExitCodeProcess(STILL_ACTIVE=259)
    try:
        import ctypes
        k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        SYNCHRONIZE = 0x00100000
        h = k32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, False, pid)
        if not h:
            err = ctypes.get_last_error()
            # 87=ERROR_INVALID_PARAMETER(그런 프로세스 없음) / 5=ACCESS_DENIED(살아있음)
            return err != 87
        try:
            code = ctypes.c_ulong(0)
            if k32.GetExitCodeProcess(h, ctypes.byref(code)):
                return code.value == 259  # STILL_ACTIVE
            return True
        finally:
            k32.CloseHandle(h)
    except Exception:
        return True              # 판정불가 = 보수적 생존


# ── 원자 쓰기 (mkstemp 공용화 — 고정 .tmp 교차 파손 차단) ──────────────────────
def atomic_write_bytes(path, data, mode=0o644):
    """같은 디렉터리 mkstemp → fsync 없이 os.replace 원자 교체.
    ★고정 `.tmp` 이름을 쓰면 동시 writer가 **서로의 임시 파일을 파손**해(A8 재검증의 지배 실패
      모드) 등록부 수리가 영구 거부된다. mkstemp는 프로세스마다 유일 이름을 보장한다."""
    d = os.path.dirname(os.path.abspath(path)) or "."
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-" + os.path.basename(path) + "-")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        try:
            os.chmod(tmp, mode)
        except OSError:
            pass
        os.replace(tmp, path)
        tmp = None
    finally:
        if tmp is not None and os.path.exists(tmp):
            try:
                os.unlink(tmp)
            except OSError:
                pass


def atomic_write_text(path, text, mode=0o644):
    """CRLF 함정 회피(newline 변환 없음 — 바이트 그대로)."""
    return atomic_write_bytes(path, text.encode("utf-8"), mode=mode)


def atomic_write_json(path, obj, indent=1, ensure_ascii=False, trailing_newline=True):
    """boot-last.json 등 진단 JSON — 기존 `_atomic_write_json` 과 **바이트 동일** 계약
    (indent=1 · ensure_ascii=False · newline='\\n' · 말미 개행)."""
    body = json.dumps(obj, ensure_ascii=ensure_ascii, indent=indent)
    if trailing_newline:
        body += "\n"
    return atomic_write_text(path, body)


# ── 보유자 레코드 ─────────────────────────────────────────────────────────────
def _holder_blob(owner):
    return json.dumps({"pid": os.getpid(), "started": time.time(),
                       "owner": owner or "", "host": _short_host()},
                      ensure_ascii=False)


def _short_host():
    try:
        import socket
        return socket.gethostname()[:64]
    except Exception:
        return ""


def read_holder(path):
    """락 파일의 보유자 레코드(dict) 또는 None. 파손·빈 파일도 None(=신원 불명)."""
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            raw = f.read(4096).strip()
    except OSError:
        return None
    if not raw:
        return None
    try:
        d = json.loads(raw)
    except ValueError:
        return None
    return d if isinstance(d, dict) else None


class FileLock:
    """단일 실행 락. 사용법 3가지 — 어느 쪽이든 상태는 `.status` 로 읽는다.

        lk = FileLock(path, owner="bootstrap")
        if lk.acquire() == ACQUIRED: ...          # ① 타입드 3상 직접 소비
        with FileLock(path) as lk: ...            # ② busy/unavailable → LockError
        with FileLock(path, soft=True) as lk:     # ③ 예외 없이 진입, lk.status 로 분기
            if lk.status == ACQUIRED: ...

    `hold_open=True`(기본)면 release()까지 fd를 보유한다 — GC가 fd를 닫아 락이 조용히
    풀리는 사고(구 `_acquire_singleflight._fd` 우회의 원인)를 구조적으로 막는다.
    """

    def __init__(self, path, owner="", blocking=False, timeout=0.0, poll=0.05,
                 reclaim_stale=True, soft=False):
        self.path = path
        self.owner = owner
        self.blocking = bool(blocking)
        self.timeout = float(timeout)
        self.poll = float(poll)
        self.reclaim_stale = bool(reclaim_stale)
        self.soft = bool(soft)
        self.backend = backend_name()
        self.status = None
        self.detail = ""
        self.reclaimed_stale = False
        self.blocked_by = None      # busy 시 보유자 레코드(진단)
        self._fd = None

    # ── 내부: 백엔드별 1회 시도 ──
    def _try_os_lock(self):
        try:
            os.makedirs(os.path.dirname(os.path.abspath(self.path)) or ".", exist_ok=True)
            fd = os.open(self.path, os.O_CREAT | os.O_RDWR, 0o644)
        except OSError as e:
            self.detail = "락 파일 열기 실패(%s)" % e
            return UNAVAILABLE
        try:
            if self.backend == "fcntl":
                _fcntl.flock(fd, _fcntl.LOCK_EX | _fcntl.LOCK_NB)
            else:
                _msvcrt.locking(fd, _msvcrt.LK_NBLCK, 1)
        except OSError as e:
            os.close(fd)
            self.blocked_by = read_holder(self.path)
            self.detail = "보유자 존재(%s)" % (
                ("pid=%s" % self.blocked_by.get("pid")) if self.blocked_by else "신원 불명")
            return BUSY
        self._fd = fd
        self._stamp_holder()
        return ACQUIRED

    def _try_pidfile(self):
        """OS 락 부재 폴백 — O_EXCL 생성 + 보유 pid 사망 시 회수(R1의 유한 거부 창 해소)."""
        d = os.path.dirname(os.path.abspath(self.path)) or "."
        try:
            os.makedirs(d, exist_ok=True)
        except OSError as e:
            self.detail = "락 디렉터리 생성 실패(%s)" % e
            return UNAVAILABLE
        for _ in range(2):   # 1회 회수 후 1회 재시도까지만(회수 경쟁 무한루프 금지)
            try:
                fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o644)
            except FileExistsError:
                holder = read_holder(self.path)
                pid = holder.get("pid") if holder else None
                if self.reclaim_stale and not pid_alive(pid):
                    # 스테일: 보유 pid 사망(또는 신원 불명) → 회수하고 재시도.
                    try:
                        os.unlink(self.path)
                        self.reclaimed_stale = True
                        continue
                    except OSError:
                        pass
                self.blocked_by = holder
                self.detail = "보유자 생존(%s)" % (
                    ("pid=%s" % pid) if pid else "신원 불명")
                return BUSY
            except OSError as e:
                self.detail = "락 파일 생성 실패(%s)" % e
                return UNAVAILABLE
            self._fd = fd
            self._stamp_holder()
            return ACQUIRED
        self.detail = "회수 후 재획득 경쟁 — 다른 보유자 선점"
        return BUSY

    def _stamp_holder(self):
        """보유자 신원 기록(best-effort) — 실패해도 락은 유효하다."""
        try:
            os.lseek(self._fd, 0, os.SEEK_SET)
            os.ftruncate(self._fd, 0)
            os.write(self._fd, _holder_blob(self.owner).encode("utf-8"))
            try:
                os.fsync(self._fd)
            except OSError:
                pass
        except OSError:
            pass

    # ── 공개 API ──
    def acquire(self):
        """ACQUIRED | BUSY | UNAVAILABLE 반환(문자열 상수). 멱등 — 이미 보유 시 ACQUIRED."""
        if self._fd is not None:
            self.status = ACQUIRED
            return self.status
        deadline = time.time() + self.timeout if (self.blocking and self.timeout > 0) else None
        while True:
            st = self._try_pidfile() if self.backend == "pidfile" else self._try_os_lock()
            if st != BUSY or deadline is None or time.time() >= deadline:
                self.status = st
                return st
            time.sleep(self.poll)

    def release(self):
        if self._fd is None:
            return
        fd, self._fd = self._fd, None
        try:
            if self.backend == "fcntl" and _fcntl is not None:
                _fcntl.flock(fd, _fcntl.LOCK_UN)
            elif self.backend == "msvcrt" and _msvcrt is not None:
                os.lseek(fd, 0, os.SEEK_SET)
                _msvcrt.locking(fd, _msvcrt.LK_UNLCK, 1)
        except OSError:
            pass
        try:
            os.close(fd)
        except OSError:
            pass
        if self.backend == "pidfile":
            # 폴백 백엔드는 파일 존재가 곧 락이다 — 정상 종료 시 스스로 치운다.
            try:
                os.unlink(self.path)
            except OSError:
                pass

    def __enter__(self):
        st = self.acquire()
        if st != ACQUIRED and not self.soft:
            raise LockError(st, self.path, self.detail)
        return self

    def __exit__(self, exc_type, exc, tb):
        self.release()
        return False


# ── self-test (결정론 · 외부 의존 0) ─────────────────────────────────────────
def _self_test():
    import shutil
    import subprocess
    fails = []
    tmp = tempfile.mkdtemp(prefix="javis-lock-st-")
    try:
        # ① 원자 쓰기: 내용 일치 + 임시 파일 잔여 0
        p = os.path.join(tmp, "a.json")
        atomic_write_json(p, {"k": "한글"})
        with open(p, encoding="utf-8") as f:
            body = f.read()
        if json.loads(body) != {"k": "한글"}:
            fails.append("atomic_write_json 내용 불일치")
        if not body.endswith("\n"):
            fails.append("atomic_write_json 말미 개행 계약 위반")
        if [n for n in os.listdir(tmp) if n.startswith(".tmp-")]:
            fails.append("mkstemp 임시 파일 잔여")

        # ② 같은 프로세스 이중 획득 = 멱등
        lp = os.path.join(tmp, "x.lock")
        lk = FileLock(lp, owner="st")
        if lk.acquire() != ACQUIRED or lk.acquire() != ACQUIRED:
            fails.append("멱등 재획득 실패")
        # ③ 보유 중 다른 프로세스는 busy
        code = ("import sys,os;sys.path.insert(0,%r);import javis_lock as L;"
                "lk=L.FileLock(%r);print(lk.acquire())" % (os.path.dirname(os.path.abspath(__file__)), lp))
        r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
        if r.stdout.strip() != BUSY:
            fails.append("보유 중 타 프로세스가 busy 아님: %r %s" % (r.stdout.strip(), r.stderr[-200:]))
        lk.release()
        # ④ 해제 후 재획득 가능
        r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
        if r.stdout.strip() != ACQUIRED:
            fails.append("해제 후 타 프로세스 재획득 실패: %r" % r.stdout.strip())

        # ⑤ pidfile 폴백: 스테일(죽은 pid) 회수 — R1의 유한 거부 창 해소 핀
        env_bak = os.environ.get("CYS_LOCK_BACKEND")
        os.environ["CYS_LOCK_BACKEND"] = "pidfile"
        try:
            sp = os.path.join(tmp, "stale.lock")
            dead = _dead_pid()
            atomic_write_text(sp, json.dumps({"pid": dead, "started": 0, "owner": "ghost"}))
            s = FileLock(sp, owner="st")
            if s.acquire() != ACQUIRED or not s.reclaimed_stale:
                fails.append("스테일 락 회수 실패(status=%s reclaimed=%s)" % (s.status, s.reclaimed_stale))
            s.release()
            # ⑥ pidfile 폴백: 살아있는 보유자는 회수 금지(중복 스폰 방지)
            ap = os.path.join(tmp, "alive.lock")
            atomic_write_text(ap, json.dumps({"pid": os.getpid(), "started": time.time(), "owner": "me"}))
            a = FileLock(ap, owner="st")
            if a.acquire() != BUSY or a.reclaimed_stale:
                fails.append("살아있는 보유자의 락을 오회수(status=%s)" % a.status)
        finally:
            if env_bak is None:
                os.environ.pop("CYS_LOCK_BACKEND", None)
            else:
                os.environ["CYS_LOCK_BACKEND"] = env_bak

        # ⑦ 컨텍스트 매니저: busy 시 LockError / soft=True 는 예외 없이 status 노출
        hp = os.path.join(tmp, "cm.lock")
        held = FileLock(hp, owner="holder")
        held.acquire()
        code2 = ("import sys,os;sys.path.insert(0,%r);import javis_lock as L\n"
                 "try:\n"
                 "    with L.FileLock(%r): print('ENTERED')\n"
                 "except L.LockError as e: print('LOCKERROR', e.status)\n"
                 "with L.FileLock(%r, soft=True) as s: print('SOFT', s.status)\n"
                 % (os.path.dirname(os.path.abspath(__file__)), hp, hp))
        r = subprocess.run([sys.executable, "-c", code2], capture_output=True, text=True, timeout=30)
        if "LOCKERROR busy" not in r.stdout or "SOFT busy" not in r.stdout:
            fails.append("컨텍스트 매니저 3상 계약 위반: %r %s" % (r.stdout, r.stderr[-200:]))
        held.release()

        # ⑧ unavailable: 락 경로가 디렉터리면 파일 열기 불가 → busy 아님(융합 금지 핀)
        dp = os.path.join(tmp, "dir.lock")
        os.makedirs(dp, exist_ok=True)
        u = FileLock(dp, owner="st")
        if u.acquire() != UNAVAILABLE:
            fails.append("락 인프라 고장이 unavailable로 분리되지 않음(status=%s)" % u.status)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if fails:
        for f in fails:
            sys.stderr.write("FAIL %s\n" % f)
        return 1
    print("javis_lock self-test OK — 8 케이스(원자쓰기·멱등·busy·재획득·스테일 회수·"
          "생존 보유자 보호·컨텍스트 3상·unavailable 분리) · backend=%s" % backend_name())
    return 0


def _dead_pid():
    """확실히 죽은 pid 하나 — 자식을 즉시 낳고 회수한다(고정 상수 추측 금지)."""
    if os.name == "posix":
        pid = os.fork()
        if pid == 0:
            os._exit(0)
        os.waitpid(pid, 0)
        return pid
    import subprocess
    r = subprocess.Popen([sys.executable, "-c", "pass"])
    r.wait()
    return r.pid


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        sys.exit(_self_test())
    sys.stderr.write(__doc__.splitlines()[0] + "\n(사용: --self-test · 라이브러리 모듈)\n")
    sys.exit(0)
