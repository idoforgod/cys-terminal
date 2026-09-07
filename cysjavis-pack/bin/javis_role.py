#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_role.py — 좌석 역할 해소의 **단일 소유 모듈** (0.14.31 P6 · 감사 codex E).

정본 금지 조항(IMPLEMENTATION-PLAN.md §8): "`CYS_ROLE` env 를 권위로 쓰지 않는다(승계 후
stale). 데몬 조회 우선." — 그런데 팩의 파이썬 도구들은 각자 `os.environ.get("CYS_ROLE")` 를
직접 읽어 결정했다. 좌석이 승계되면(claim-role·takeover) env 는 **낡은 채로 살아남고**
데몬 roles 맵만 바뀌므로, 그 결정들은 전부 옛 신원으로 내려진다:
  · `javis_org.py require_cso()`  — 부서 lifecycle mutation 단일소유 게이트
  · `javis_snapshot.py is_master()` — BOOT_SNAPSHOT 생산 게이트
  · `javis_completion_guard.py _role()` — 이벤트 `agent` 귀속 라벨
이 모듈이 그 **한 곳**이다. 훅 셸의 짝은 `hooks/_lib.sh` 의 `cys_resolve_role()` 이며
**같은 캐시 파일 형식**(`"<epoch> <역할|->"` 1줄)을 공유한다 — 두 층이 갈리면
tests/test_role_authority.py 가 멈춘다.

────────────────────────────────────────────────────────────────────────────
계약(데몬 CLI `cys surface-role` = src/bin/cys.rs:10940 `run_surface_role` 3상)
  ⓐ rc=0 + 역할 문자열 → **권위 있는 역할**
  ⓑ rc=0 + 빈 줄       → **권위 있는 '역할 없음'**(내 surface 가 데몬 목록에 없을 때도 이것 ·
                          `CYS_SURFACE_ID` 부재/파싱 불가면 조회 없이 이것)
  ⓒ 그 밖(rc≠0·타임아웃·바이너리 부재) → **판정 불가**. '역할 없음'이 아니다.
ⓑ와 ⓒ를 뭉개면 데몬 사망이 '무역할'로 읽힌다 — Rust 가 그 둘을 rc 로 갈라 놓았으므로
여기서도 절대 합치지 않는다.

★신원 전제(이 모듈이 데몬에 묻는 조건): `CYS_SURFACE_ID`(구 `AITERM_SURFACE_ID`)가 있고
  `^(surface:)?[0-9]+$` 로 파싱될 때만 묻는다(Rust `parse_surface_ref` src/lib.rs:2263 과
  같은 규칙). 없으면 조회 자체를 하지 않고 env 로 답한다. 이유: surface 없는 실행(일반
  터미널 · `CYS_ROLE=cso python3 javis_org.py apply …` 같은 정식 위임 경로 · 검체 하네스)에서
  Rust 는 ⓑ(빈 줄·rc 0)를 내는데, 그것을 '권위 있는 무역할'로 채택하면 **주소가 없다는
  사실이 역할이 없다는 판정으로 승격**된다. 그 승격은 정상 경로를 죽인다.

★실패 방향(§3-3 "막는 쪽으로만 틀린다"): 판정 불가(ⓒ)의 귀결은 **현행 동작 그대로**(env
  폴백)다. 즉 이 모듈이 추가하는 실패 경로는 없다 — 데몬이 답할 때만 판정이 더 정확해진다.

★비용(부트체인 ④ '전 pane 사망' 회피): `_role()` 은 한 런에 20+회 불린다
  (javis_completion_guard.py :534 :626 :962 :1368 …). 그래서
    ① **프로세스 메모** — 한 프로세스에서 조회는 최대 1회
    ② **디스크 캐시 60s** — 훅이 초당 여러 번 떠도 왕복은 분당 1회
    ③ **실패 백오프 30s** — 데몬이 죽어 있을 때 매 호출 2s 정지가 전 pane 에 걸리지 않게
  세 겹으로 막는다. 승계 반영 지연 ≤60s 는 **명시적으로 수용**한다(승계는 로컬에서 감지할
  수 없다 · role-capability-gate.sh 의 TTL 15s 는 그 훅이 **능력 게이트**라 더 짧게 잡은
  것이고, 여기 소비처는 게이트가 아니거나(라벨) 기동 시 1회 판정(게이트)이다).

★자식에게 `CYS_NO_AUTOSTART=1` 을 건다: 소켓 파일이 없으면 `cys` 는 autostart 경로를 탄다
  (src/bin/cys.rs:2258). **역할을 묻는 행위가 데몬을 낳아서는 안 된다** — 특히 `cys-dept`
  가드에서 부르는 경로는 아직 데몬이 없을 때 도는 경로다.

stdlib 만 사용. import 부작용 0(파일 생성·env 변경·프로세스 스폰 전무 — 첫 `resolve_role()`
호출에서만 조회가 일어난다).

★출하 전제: `build.rs` 는 `git ls-files cysjavis-pack`(추적 파일 전용)으로 팩을 임베드한다.
  이 파일이 git 에 추적되지 않으면 설치본에 존재하지 않는다(javis_lane.py 헤더와 같은 경고).
"""
import os
import re
import subprocess
import sys
import tempfile
import time

__all__ = ["resolve_role", "resolve_role_detail", "reset_cache",
           "is_authoritative", "is_authoritative_none",
           "QUERY_TIMEOUT_S", "CACHE_TTL_S", "FAIL_BACKOFF_S"]

QUERY_TIMEOUT_S = 2.0      # 초 · 자식 `cys surface-role` 데드라인(Rust 내장 10s 보다 짧게)
CACHE_TTL_S = 60           # 초 · 디스크 캐시 수명(= 승계 반영 지연 상한 · 명시적 수용)
FAIL_BACKOFF_S = 30        # 초 · 조회 실패 후 재조회 유예(데몬 사망 시 폭주·정지 차단)
ROLE_MAX_LEN = 64          # 역할 문자열 상한(다중행·거대값 오염 차단)

# Rust `parse_surface_ref`(src/lib.rs:2263)와 **같은 규칙**: "surface:31" | "31".
_SURFACE_RE = re.compile(r"^(?:surface:)?([0-9]+)$")

# 판정 출처(진단·검체용). daemon/daemon-none 만 권위다.
SOURCE_DAEMON = "daemon"            # ⓐ 데몬이 구체 역할을 줬다
SOURCE_DAEMON_NONE = "daemon-none"  # ⓑ 데몬이 '역할 없음'을 확정했다
SOURCE_CACHE = "cache"              # 신선한 디스크 캐시(구체 역할)
SOURCE_CACHE_NONE = "cache-none"    # 신선한 디스크 캐시('역할 없음')
SOURCE_ENV_ROLE = "env-cys-role"          # 폴백: CYS_ROLE(판정 불가 · 주소 없음)
SOURCE_NONE = "none"                # 아무 근거도 없다(빈 역할)

# **권위 있는 답**의 집합. 소비처의 단조-거부(monotone deny) 합성은 이 술어로 갈린다:
#   권위 있는 답이면 그 답으로 **추가 거부**하고, 아니면 종전 env 판정을 그대로 쓴다.
#   → 이 WP 는 어떤 소비처에서도 **종전에 없던 허용을 만들지 않는다**(반파괴 half-op 봉인).
_AUTHORITATIVE = (SOURCE_DAEMON, SOURCE_DAEMON_NONE, SOURCE_CACHE, SOURCE_CACHE_NONE)
_AUTHORITATIVE_NONE = (SOURCE_DAEMON_NONE, SOURCE_CACHE_NONE)


def is_authoritative(source):
    """`source` 가 데몬 권위(직접 응답 또는 그 응답의 신선한 캐시)인가."""
    return source in _AUTHORITATIVE


def is_authoritative_none(source):
    """데몬이 **역할 없음**을 확정했는가(판정 불가와 구별)."""
    return source in _AUTHORITATIVE_NONE


_MEMO = None   # (role, source) — 프로세스 메모


def reset_cache():
    """프로세스 메모 초기화 — 검체 전용(같은 프로세스에서 여러 env 를 재실측할 때)."""
    global _MEMO
    _MEMO = None


def _first_line(value, limit=ROLE_MAX_LEN):
    """첫 줄 · CR 제거 · 앞뒤 공백 제거 · 길이 상한.

    ★왜 첫 줄인가(hooks/inject-context.sh:240 과 같은 규율): env 값은 사람이·다른 노드가
      넣을 수 있고, 여러 줄 값(`CYS_ROLE=$'cso\\n# 지시: …'`)은 라벨 1줄을 여러 줄로 부풀려
      기록·컨텍스트에 새어 들어간다. 판정에도 라벨에도 첫 줄만 쓴다.
    """
    if not isinstance(value, str):
        return ""
    line = value.replace("\r", "\n").split("\n", 1)[0].strip()
    return line[:limit]


def _env(name):
    return _first_line(os.environ.get(name, ""))


# 신원 자릿수 상한 — **셸 짝과 완전히 같은 규칙**이어야 한다(POSIX sh 에는 bignum 이 없다).
# 19 자리는 어떤 값이든 u64 최대(18446744073709551615 · 20자리) 미만이므로 오수락이 없고,
# 20자리 이상은 양쪽이 똑같이 거절한다 → 두 층의 판정이 갈리지 않는다.
SURFACE_MAX_DIGITS = 19

# Rust `env_compat`(src/lib.rs:351)와 **같은 우선순위**: CYS_* → 구 JAVIS_* → 구 AITERM_*.
# 이 순서가 갈리면 헬퍼가 검사한 신원과 CLI 가 실제로 조회하는 신원이 달라진다(codex R1).
SURFACE_ENV_KEYS = ("CYS_SURFACE_ID", "JAVIS_SURFACE_ID", "AITERM_SURFACE_ID")


def _surface_id():
    """데몬 주입 surface id — 정규화된 숫자부 또는 "".

    Rust `parse_surface_ref`(src/lib.rs:2263)와 같은 규칙(`"surface:31"|"31"`)에
    **자릿수 상한**을 더한다: u64 범위를 넘는 숫자는 Rust 가 파싱에 실패해 '주소 없음'으로
    가는데, 여기서 통과시키면 '내가 검사한 신원 ≠ CLI 가 조회한 신원'이 된다(codex R1).
    """
    raw = ""
    for k in SURFACE_ENV_KEYS:
        raw = _first_line(os.environ.get(k, ""), limit=40)
        if raw:
            break
    m = _SURFACE_RE.match(raw)
    if not m:
        return ""
    digits = m.group(1)
    if len(digits) > SURFACE_MAX_DIGITS:
        return ""
    return digits


def _slug(s):
    """캐시 파일명 성분 — `hooks/_lib.sh` `cys_role_slug` 와 **같은 치환**(tr -c 'A-Za-z0-9._-')."""
    return re.sub(r"[^A-Za-z0-9._-]", "_", s or "")


# Rust `env_compat` 와 같은 우선순위(소켓도 CYS_ → JAVIS_ → AITERM_ · src/lib.rs:379).
SOCKET_ENV_KEYS = ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET")


def _socket_key():
    for k in SOCKET_ENV_KEYS:
        v = _first_line(os.environ.get(k, ""), limit=4096)
        if v:
            return v
    return ""


def _boot_epoch():
    """`dirname($CYS_SOCKET)/boot-epoch` 첫 줄 — 데몬이 부트마다 bump(boot_supervisor.rs:856).

    캐시 키에 넣어 **데몬 재기동 = 캐시 무효**로 만든다(재기동 후 stale 역할 재사용 차단).

    ★정직한 한계(codex R1 · 과장 금지): 이것은 **권위가 아니라 캐시 키의 소금**이다.
      ⓐ Windows 의 소켓은 named pipe(`\\\\.\\pipe\\cys`)라 `dirname` 이 상태 디렉터리가 아니다 →
        epoch 는 빈 문자열이 된다. ⓑ 감독자 비활성·쓰기 실패면 파일이 없거나 옛 값이 남는다.
      그 경우 캐시 무효화는 **TTL 60s 하나만** 남는다 — 없어도 안전 방향이 바뀌지 않게(캐시는
      권위가 아니고 60초 사본일 뿐) 설계했고, 셸 짝도 **같은 규칙**을 써서 두 층이 같은 키를 만든다.
    """
    sock = _socket_key()
    if not sock:
        return ""
    try:
        with open(os.path.join(os.path.dirname(sock), "boot-epoch"),
                  encoding="utf-8", errors="replace") as f:
            return _first_line(f.readline(), limit=40)
    except Exception:
        return ""


def _cache_path(sid):
    # ★TMPDIR 우선(셸 짝 `_lib.sh` 가 `${TMPDIR:-/tmp}` 로 같은 파일을 집는다) · 부재 시
    #   `tempfile.gettempdir()`(네이티브 Windows 는 TMPDIR 대신 TEMP/TMP 라 "/tmp" 로 붕괴한다).
    base = os.environ.get("TMPDIR") or tempfile.gettempdir()
    name = "cys-role-authority-%s-%s-%s" % (
        _slug(sid or "none"),
        _slug(_socket_key() or "none"),
        _slug(_boot_epoch()))
    return os.path.join(base, name)


CACHE_READ_CAP = 4096      # 바이트 · 캐시 판독 상한(거대·FIFO 파일로 훅을 붙잡지 못하게)

_O_NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)   # Windows 에는 없다(0 = 무효과)
_O_BINARY = getattr(os, "O_BINARY", 0)


def _open_trusted(path):
    """캐시 파일을 **연 뒤** 그 fd 로 검증한다 — 검사 후 교체(TOCTOU)를 닫는다.

    ⓐ `O_NOFOLLOW` 로 심링크는 열리지 않는다(POSIX). ⓑ 연 fd 를 `fstat` 해 정규 파일·
    소유자 자신을 확인한다 — `lstat` 후 `open` 하는 순서였다면 그 사이에 바꿔치기가 된다.
    ⓒ 검사에 실패하면 **신뢰하지 않는다**(캐시 없음으로 읽는 쪽이 안전하다).
    ★Windows: `O_NOFOLLOW`·`st_uid` 가 없거나 무의미하다 → 그 축만 비고, TMPDIR 이 사용자별이라는
      OS 계약에 기댄다. 캐시는 **권한 증명이 아니다**(같은 사용자 프로세스는 어차피 쓸 수 있다) —
      권위는 데몬 응답이고 캐시는 그 응답의 60초 사본일 뿐이다.
    """
    import stat as _stat
    try:
        fd = os.open(path, os.O_RDONLY | _O_NOFOLLOW | _O_BINARY)
    except Exception:
        return None
    try:
        st = os.fstat(fd)
        if not _stat.S_ISREG(st.st_mode):
            os.close(fd)
            return None
        getuid = getattr(os, "geteuid", None) or getattr(os, "getuid", None)
        if getuid is not None and st.st_uid != getuid():
            os.close(fd)
            return None
        return fd
    except Exception:
        try:
            os.close(fd)
        except Exception:
            pass
        return None


def _read_first_line(path):
    """신뢰 가능한 캐시 파일의 첫 줄(상한 4KB). 못 읽으면 None."""
    fd = _open_trusted(path)
    if fd is None:
        return None
    try:
        raw = os.read(fd, CACHE_READ_CAP)
    except Exception:
        raw = b""
    finally:
        try:
            os.close(fd)
        except Exception:
            pass
    try:
        text = raw.decode("utf-8", "replace")
    except Exception:
        return None
    return text.replace("\r", "\n").split("\n", 1)[0]


def _cache_read(path, now):
    """(role, is_none, fresh) — 형식 `"<epoch> <역할|->"` 1줄.

    시계 역행(미래 타임스탬프)은 신선이 아니다 — 그러면 캐시가 무기한 유효해진다.
    """
    line = _read_first_line(path)
    if line is None:
        return "", False, False
    parts = line.split(" ", 1)
    if len(parts) != 2:
        return "", False, False          # 구형·손상 형식은 무시
    try:
        ts = int(parts[0])
    except ValueError:
        return "", False, False
    role = _first_line(parts[1])
    fresh = (ts > 0 and ts <= now and (now - ts) < CACHE_TTL_S)
    if role == "-":
        return "", True, fresh
    if not role:
        return "", False, False
    return role, False, fresh


def _cache_write(path, now, value):
    """0600 · 같은 디렉터리 원자 교체. 실패는 조용히 무시한다(캐시는 최적화지 사실이 아니다).

    ★`O_CREAT|O_EXCL` 로 임시 이름을 **새로** 만든다 — 기존 파일에 바로 쓰면 그 자리에 심어 둔
      심링크의 목적지를 truncate 하는 길이 된다. 최종 배치는 `os.replace`(원자 교체)다.
    """
    tmp = "%s.%d.tmp" % (path, os.getpid())
    try:
        try:
            os.unlink(tmp)
        except Exception:
            pass
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | _O_BINARY, 0o600)
        try:
            os.write(fd, ("%d %s\n" % (now, value)).encode("utf-8"))
        finally:
            os.close(fd)
        os.replace(tmp, path)
    except Exception:
        try:
            os.unlink(tmp)
        except Exception:
            pass


def _fail_fresh(path, now):
    """조회 실패 표식이 백오프 창 안인가(형식은 캐시와 동일 `"<epoch> -"` — 첫 토큰만 읽는다)."""
    line = _read_first_line(path)
    if line is None:
        return False
    try:
        ts = int(line.split(" ", 1)[0])
    except ValueError:
        return False
    return ts > 0 and ts <= now and (now - ts) < FAIL_BACKOFF_S


def _query_daemon():
    """(role, ok) — ok=False 는 **판정 불가**(ⓒ)다. role="" + ok=True 는 권위 있는 무역할(ⓑ).

    ★자식 env: `CYS_NO_AUTOSTART=1` 강제(역할 조회가 데몬을 낳지 않게) ·
      `PYTHONDONTWRITEBYTECODE`/`PYTHONUTF8` 은 부모 값을 그대로 상속한다.
    """
    exe = _first_line(os.environ.get("CYS_BIN", ""), limit=4096) or "cys"
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"
    try:
        p = subprocess.run([exe, "surface-role"], capture_output=True, text=True,
                           encoding="utf-8", errors="replace",
                           timeout=QUERY_TIMEOUT_S, env=env)
    except Exception:
        # FileNotFoundError(cys 부재) · TimeoutExpired · PermissionError … 전부 판정 불가.
        return "", False
    if p.returncode != 0:
        return "", False
    return _first_line(p.stdout), True


def resolve_role_detail():
    """(role, source) — 데몬 권위 우선 · 캐시 · env 폴백. **예외를 내지 않는다**.

    소비처는 `source` 로 '권위 있는 무역할'(daemon-none/cache-none)과 '판정 불가 후 env 폴백'
    을 구분할 수 있다 — `cys-dept` 단일소유 가드가 그 구분을 쓴다.
    """
    global _MEMO
    if _MEMO is not None:
        return _MEMO
    try:
        _MEMO = _resolve_uncached()
    except Exception:
        # 이 모듈이 소비처를 죽이는 경로는 없다(§3-3) — 최악이 현행(env) 동작이다.
        _MEMO = _env_fallback()
    return _MEMO


def _env_fallback():
    """폴백은 **`CYS_ROLE` 하나뿐**이다 — 현행 결정 지점들이 읽는 바로 그 키.

    ★`CYS_SURFACE_ROLE` 을 일반 폴백에 넣지 않는 이유(codex R1): 그 변수는
      `role-capability-gate.sh` 가 **해소 결과로 export 하는 산출물**이지 신원 입력이 아니다.
      폴백에 넣으면 `CYS_SURFACE_ROLE=cso` + `CYS_ROLE=worker` 조합에서 **현행이 거부하던 것을
      새로 허용**하게 된다 — "판정 불가면 현행 그대로"라는 이 WP 의 실패 방향 약속이 거짓이 된다.
      그 두 키를 함께 보는 소비처(`hooks/inject-context.sh`)는 자기 계약으로 직접 본다.
    """
    v = _env("CYS_ROLE")
    if v:
        return v, SOURCE_ENV_ROLE
    return "", SOURCE_NONE


def _resolve_uncached():
    sid = _surface_id()
    if not sid:
        # 주소가 없다 = 데몬에게 '나'를 물을 수 없다. 무역할이라는 **판정이 아니다**.
        return _env_fallback()
    now = int(time.time())
    cpath = _cache_path(sid)
    c_role, c_none, c_fresh = _cache_read(cpath, now)
    if c_fresh:
        return (c_role, SOURCE_CACHE) if c_role else ("", SOURCE_CACHE_NONE)
    fpath = cpath + ".fail"
    if not _fail_fresh(fpath, now):
        role, ok = _query_daemon()
        if ok:
            _cache_write(cpath, now, role if role else "-")
            try:
                os.unlink(fpath)
            except Exception:
                pass
            return (role, SOURCE_DAEMON) if role else ("", SOURCE_DAEMON_NONE)
        _cache_write(fpath, now, "-")   # 실패 표식(형식 공유 · 값은 안 읽는다)
    # ⓒ 판정 불가 — 신선하지 않은 캐시는 쓰지 않는다(옛 역할이 무기한 사는 길). env 폴백.
    return _env_fallback()


def resolve_role(default=""):
    """역할 문자열(없으면 `default`). 결정 지점의 **유일한 입구**."""
    role, _src = resolve_role_detail()
    return role or default


def _self_test():
    """밀폐 자기검증 — 데몬 3상 x 폴백을 스텁 `cys` 로 실측(팩 `--self-test` 관례).

    ★Windows-safe: 스텁은 `os.name` 으로 `.bat`/`sh` 를 갈라 쓴다(Git Bash 없이도 돈다).
    """
    import shutil
    fails = []

    def check(name, cond, detail=""):
        print("%s %s%s" % ("PASS" if cond else "FAIL", name,
                           (" - " + detail) if detail else ""))
        if not cond:
            fails.append(name)

    keys = ("CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_ROLE", "CYS_SURFACE_ROLE",
            "CYS_SOCKET", "CYS_BIN", "TMPDIR")
    saved = {k: os.environ.get(k) for k in keys}
    td = tempfile.mkdtemp(prefix="javis-role-st-")
    n = [0]
    try:
        def stub(rc, out):
            """rc·stdout 을 고정하는 `cys` 스텁을 만들고 CYS_BIN 으로 가리킨다."""
            n[0] += 1
            if os.name == "nt":
                p = os.path.join(td, "cys-stub-%d.bat" % n[0])
                body = "@echo off\r\n"
                body += ("echo(%s\r\n" % out) if out else "echo(\r\n"
                body += "exit /b %d\r\n" % rc
            else:
                p = os.path.join(td, "cys-stub-%d.sh" % n[0])
                body = "#!/bin/sh\n"
                body += ("printf '%s\\n' " + _sh_quote(out) + "\n") if out else "printf '\\n'\n"
                body += "exit %d\n" % rc
            with open(p, "w", encoding="utf-8", newline="") as f:
                f.write(body)
            os.chmod(p, 0o755)
            os.environ["CYS_BIN"] = p

        def fresh(**env):
            for k in keys:
                os.environ.pop(k, None)
            os.environ["TMPDIR"] = tempfile.mkdtemp(dir=td)
            os.environ.update({k: v for k, v in env.items() if v is not None})
            reset_cache()

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(0, "cso")
        check("데몬 답 우선(env=master 인데 데몬=cso)",
              resolve_role_detail() == ("cso", SOURCE_DAEMON), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(2, "")
        check("판정 불가(rc=2) -> env 폴백",
              resolve_role_detail() == ("master", SOURCE_ENV_ROLE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7")
        stub(2, "")
        check("둘 다 없음 -> 빈 역할",
              resolve_role_detail() == ("", SOURCE_NONE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="cso")
        stub(0, "")
        check("권위 있는 무역할(rc0+빈줄)이 env 를 덮는다",
              resolve_role_detail() == ("", SOURCE_DAEMON_NONE), repr(resolve_role_detail()))

        fresh(CYS_ROLE="cso")
        stub(0, "")
        check("surface 없음 -> 조회 없이 env(주소 부재는 무역할 판정이 아니다)",
              resolve_role_detail() == ("cso", SOURCE_ENV_ROLE), repr(resolve_role_detail()))

        fresh(CYS_SURFACE_ID="7", CYS_ROLE="master")
        stub(0, "cso")
        resolve_role_detail()
        reset_cache()
        stub(2, "")                       # 데몬 사망 — 신선 캐시가 답한다
        check("신선 캐시가 판정 불가를 메운다",
              resolve_role_detail() == ("cso", SOURCE_CACHE), repr(resolve_role_detail()))
    finally:
        shutil.rmtree(td, ignore_errors=True)
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        reset_cache()
    if fails:
        print("javis_role self-test FAIL (%d): %s" % (len(fails), fails))
        return 1
    print("javis_role self-test PASS")
    return 0


def _sh_quote(s):
    """POSIX sh 단일따옴표 인용(self-test 스텁 생성 전용 · shlex 의존 회피)."""
    return "'" + str(s).replace("'", "'\\''") + "'"


def main(argv):
    if "--self-test" in argv:
        return _self_test()
    role, src = resolve_role_detail()
    print("%s\t%s" % (role, src))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
