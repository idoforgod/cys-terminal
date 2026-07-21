#!/usr/bin/env python3
"""javis_browser.py — Browser v2 operation adapter.

Production lifecycle authority belongs exclusively to cysd.  This adapter sends
typed ensure/operation intents through ``cys`` and never selects or spawns a
browser executable, owns a process lock, or signals an engine PID.  The old
source browserd path remains available only with ``CYS_BROWSER_DEV=1`` for the
isolated browserd test harness.

- packaged mode: cys -> cysd BrowserAuthorityExtension -> private supervisor.
- explicit dev mode: browserd(bun+playwright-core) loopback RPC.
- 모든 동사를 ~/.cys/browser/audit.jsonl 에 append (reviewer2 감사 대상).

결정론 exit 코드:
  0 성공 · 2 BUSY · 3 APPROVAL_REQUIRED · 4 기동실패 · 5 verify FAIL · 6 HUMAN_ACTIVE
  · 7 HUMAN_PROFILE_PROTECTED · 8 PICK_TIMEOUT · 9 사용례 오류 · 10 NAV_FAILED
  · 11 NAV_UNAVAILABLE · 12 TAB_LIMIT · 13 NO_TAB · 14 SCHEME_DENIED · 15 BAD_SELECTOR
  · 16 NOT_VISIBLE · 17 NO_CONTEXT · 18 EVIDENCE_PATH_DENIED
  · 19 HUMAN_CID_RESERVED · 20 HUMAN_CID_REQUIRED · 21 PROFILE_MISMATCH(프로필 격리=보안 거부)
  · 22 CONTROL_OWNER_REQUIRED · 23 CONTROL_LEASE_MISMATCH
  · 1 기타

★9(사용례 오류)가 따로 있는 이유(4-T-10): argparse 는 미지 서브커맨드·인자 오류에 **고정 종료코드
  2**를 쓰는데, 그 2가 문서화된 `BUSY(2)="backoff 후 재시도"` 와 정확히 충돌한다. 서버에 동사를
  넣고 CLI 배선을 빠뜨리면 에이전트가 "바쁘다"로 오판해 **무한 백오프**(원인불명 hang)에 빠진다.

stdlib only (오피스 브리지 계보).
"""
import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

BROWSER_ROOT = Path.home() / ".cys" / "browser"
STATE_PATH = BROWSER_ROOT / "state.json"
AUDIT_PATH = BROWSER_ROOT / "audit.jsonl"
BROWSERD_DIR = Path(__file__).resolve().parent.parent / "browserd"
SERVER_TS = BROWSERD_DIR / "server.ts"

# 에러 코드 → exit 코드 매핑.
# ★신규 에러 코드는 반드시 여기 등재한다(4-T-10) — 미등재는 전부 exit 1 로 뭉개져 결정론 계약이
#   죽는다("실패했다"만 알고 "무엇이 실패했는지"를 잃는다).
EXIT_BY_ERROR = {
    "BUSY": 2,
    "APPROVAL_REQUIRED": 3,
    "HUMAN_ACTIVE": 6,
    "HUMAN_PROFILE_PROTECTED": 7,
    "PICK_TIMEOUT": 8,
    # 사용례 오류 계열 — argparse 오류(9)와 같은 의미축으로 모은다.
    "BAD_ARGS": 9,
    "UNKNOWN_VERB": 9,
    # Phase 3 신규
    "NAV_FAILED": 10,
    "NAV_UNAVAILABLE": 11,
    "TAB_LIMIT": 12,
    "NO_TAB": 13,
    "SCHEME_DENIED": 14,
    "BAD_SELECTOR": 15,
    "NOT_VISIBLE": 16,
    "NO_CONTEXT": 17,
    "EVIDENCE_PATH_DENIED": 18,
    # ★프로필 격리 위반(보안 거부) — exit 1(기타)로 뭉개면 에이전트가 "일반 오류"로 오판해
    #   재시도할지 중단할지 판정하지 못한다(4-T-10 ③ 미등재 문제의 정확한 사례).
    #   세 코드는 cid↔profile 상호 예약(P0-A)의 같은 가족이라 **함께** 등재한다 — 하나만 등재하면
    #   같은 위반이 방향에 따라 19 와 1 로 갈려 결정론 계약이 오히려 깨진다.
    "HUMAN_CID_RESERVED": 19,
    "HUMAN_CID_REQUIRED": 20,
    "PROFILE_MISMATCH": 21,
    "CONTROL_OWNER_REQUIRED": 22,
    "CONTROL_LEASE_MISMATCH": 23,
}
EXIT_OK = 0
EXIT_OTHER = 1
EXIT_START_FAIL = 4
EXIT_VERIFY_FAIL = 5
EXIT_APPROVAL_REQUIRED = 3
EXIT_HUMAN_PROFILE_PROTECTED = 7
EXIT_PICK_TIMEOUT = 8
EXIT_USAGE = 9  # argparse 사용례 오류 — BUSY(2)와의 충돌을 끊는다

# CLI 이름 → 서버 동사. `stop` 은 데몬 종료가 선점했으므로 페이지 로딩 중지는 `stop-loading`.
VERB_ALIAS = {"stop-loading": "stop"}


class _NoConflictParser(argparse.ArgumentParser):
    """argparse 사용례 오류를 exit 2 → **exit 9** 로 옮긴다(4-T-10).

    argparse 기본값 2는 문서화된 BUSY(2)와 충돌해, CLI 미배선을 에이전트가 '서버가 바쁘다'로
    오판하고 무한 백오프한다. 서브파서까지 같은 클래스를 쓰도록 parser_class 로 전파한다."""

    def error(self, message):
        self.print_usage(sys.stderr)
        sys.stderr.write(f"{self.prog}: 사용례 오류: {message}\n")
        sys.exit(EXIT_USAGE)

BRIEFS_DIR = BROWSER_ROOT / "briefs"
NOTEBOOKLM_URL = "https://notebooklm.google.com/"


def _pid_alive(pid: int) -> bool:
    if not pid or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def _read_state():
    if not STATE_PATH.exists():
        return None
    try:
        st = json.loads(STATE_PATH.read_text())
        if all(k in st for k in ("pid", "port", "token")):
            return st
        return None
    except Exception:
        return None


def _live_state():
    st = _read_state()
    if st and _pid_alive(st["pid"]):
        return st
    return None


def _which(name: str):
    for d in os.environ.get("PATH", "").split(os.pathsep):
        p = Path(d) / name
        if p.exists() and os.access(p, os.X_OK):
            return str(p)
    # bun 기본 설치 경로 폴백
    fallback = Path.home() / ".bun" / "bin" / name
    if fallback.exists():
        return str(fallback)
    return None


def _chrome_available() -> bool:
    mac = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    if mac.exists():
        return True
    for n in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        if _which(n):
            return True
    return False


def _development_mode() -> bool:
    """Source runtime is opt-in; no truthy aliases that packaging can set by accident."""
    return os.environ.get("CYS_BROWSER_DEV") == "1"


def _cys_command(args, timeout: float = 65.0):
    cys = _which("cys")
    if not cys:
        return None, "cys 바이너리 부재 — Browser Runtime authority에 연결할 수 없음"
    try:
        completed = subprocess.run(
            [cys, *args],
            text=True,
            capture_output=True,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return None, f"cysd Browser Runtime 요청 실패: {error}"
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "unknown failure").strip()
        return None, f"cysd Browser Runtime 거부(exit {completed.returncode}): {detail}"
    try:
        return json.loads(completed.stdout), None
    except (TypeError, json.JSONDecodeError) as error:
        return None, f"cysd Browser Runtime 응답 파싱 실패: {error}"


def _start_development_browserd(headless: bool, timeout: float = 20.0):
    """Explicit development-only source spawn. Never reachable in packaged mode."""
    bun = _which("bun")
    if not bun:
        return None, "bun 미설치 — https://bun.sh"
    if not SERVER_TS.exists():
        return None, f"server.ts 없음: {SERVER_TS}"
    BROWSER_ROOT.mkdir(parents=True, exist_ok=True)
    args = [bun, "run", str(SERVER_TS)]
    if headless:
        args.append("--headless")
    env = dict(os.environ)
    if headless:
        env["CYS_BROWSER_HEADLESS"] = "1"
    logf = open(BROWSER_ROOT / "browserd.log", "ab")
    # 부모와 분리 (detached), stdout/stderr 로그로
    subprocess.Popen(
        args,
        cwd=str(BROWSERD_DIR),
        stdout=logf,
        stderr=logf,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        env=env,
    )
    deadline = time.time() + timeout
    while time.time() < deadline:
        st = _live_state()
        if st:
            return st, None
        time.sleep(0.3)
    # 실패 시 로그 tail 반환
    try:
        tail = (BROWSER_ROOT / "browserd.log").read_text()[-2000:]
    except Exception:
        tail = ""
    return None, f"browserd 기동 타임아웃({timeout}s)\n{tail}"


def _acquire_lock(fh, deadline: float) -> bool:
    """배타 파일락(데드라인까지 블로킹). POSIX fcntl / Windows msvcrt non-blocking 재시도.
    반환 True=락 획득 · False=미지원/오류/데드라인 초과 → best-effort(락 없이 진행·크래시 금지).
    ★리뷰어1 N1: msvcrt.LK_LOCK은 ~10초 후 OSError를 던져(browserd 기동 20s와 불일치) 무음
      재개방을 유발 → LK_NBLCK non-blocking + 데드라인 재시도로 1차 cold-start를 끝까지 기다린다.
    ★리뷰어1 N3: fcntl 런타임 OSError(NFS·ENOLCK)를 삼켜 크래시를 막고 best-effort로 폴백한다."""
    try:
        import fcntl
        fcntl.flock(fh.fileno(), fcntl.LOCK_EX)  # 블로킹 배타락(with 블록 동안 유지)
        return True
    except ImportError:
        pass  # 비-POSIX → 아래 msvcrt
    except OSError:
        return False  # 네트워크FS 등 락 미지원 → best-effort
    try:
        import msvcrt
        while time.time() < deadline:
            try:
                fh.seek(0)
                msvcrt.locking(fh.fileno(), msvcrt.LK_NBLCK, 1)  # non-blocking 1바이트
                return True
            except OSError:
                time.sleep(0.3)  # busy → 데드라인까지 재시도(1차 cold-start 대기)
        return False
    except Exception:
        return False


def ensure_browserd(headless: bool):
    if _development_mode():
        st = _live_state()
        if st:
            return {**st, "transport": "dev-direct"}, None
        # Development still serializes its source-only singleton. Production never
        # enters this block and therefore never owns browserd.lock.
        BROWSER_ROOT.mkdir(parents=True, exist_ok=True)
        with open(BROWSER_ROOT / "browserd.lock", "a+") as lf:
            _acquire_lock(lf, time.time() + 22.0)
            st = _live_state()
            if st:
                return {**st, "transport": "dev-direct"}, None
            st, error = _start_development_browserd(headless)
            if st:
                st = {**st, "transport": "dev-direct"}
            return st, error

    # The shared production key is unconditionally headless. The flag is kept in
    # the adapter ABI for old callers but cannot widen the broker contract.
    result, error = _cys_command(["browser-runtime-ensure", "--headless"])
    if error:
        return None, error
    return {"transport": "broker", "status": result}, None


def rpc(st, verb: str, args: dict):
    if st.get("transport") == "broker":
        result, error = _cys_command(
            ["browser-runtime-operation", json.dumps({"verb": verb, "args": args}, separators=(",", ":"))]
        )
        if error:
            raise urllib.error.URLError(error)
        return result
    url = f"http://127.0.0.1:{st['port']}/{st['token']}/rpc"
    data = json.dumps({"verb": verb, "args": args}).encode("utf8")
    req = urllib.request.Request(url, data=data, headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf8"))


def stop_browserd():
    """Never let an adapter tear down a shared production runtime."""
    if not _development_mode():
        return False, "공유 Browser Runtime 수명주기는 cysd가 소유함 — adapter stop 거부"
    st = _live_state()
    if not st:
        return True, "미기동"
    try:
        os.kill(st["pid"], 15)
    except ProcessLookupError:
        pass
    return True, f"개발 runtime SIGTERM → pid {st['pid']}"


def audit(verb: str, args: dict, evidence_path, exit_code: int, extra: dict = None):
    try:
        BROWSER_ROOT.mkdir(parents=True, exist_ok=True)
        row = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "caller_role": os.environ.get("CYS_ROLE", "unknown"),
            "verb": verb,
            "url": args.get("url"),
            "profile": args.get("profile", "agent"),
            "evidence_path": evidence_path,
            "exit": exit_code,
        }
        if extra:
            row.update(extra)
        with open(AUDIT_PATH, "a") as f:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    except Exception:
        pass


def request_human_approval(verb: str, url: str):
    """human 프로필 요청 = CEO 결재(cys feed push --wait). fail-closed.

    반환 (approved: bool, decision: str). cys 부재/오류/deny/timeout → 전부 거부.
    feed push exit: 0=allow · 2=deny · 3=timeout.
    """
    cys = _which("cys")
    if not cys:
        return False, "no_cys"  # 개발 환경 등 cys 부재 → 기본 거부(fail-closed)
    body = f"{verb} {url}"
    try:
        r = subprocess.run(
            [cys, "feed", "push", "--wait", "--title", "browserd human 프로필 요청", "--body", body],
            timeout=130,  # feed --wait 자체 120s + 여유
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.TimeoutExpired:
        return False, "timeout"
    except Exception as e:
        return False, f"error:{e}"
    if r.returncode == 0:
        return True, "allow"
    if r.returncode == 2:
        return False, "deny"
    if r.returncode == 3:
        return False, "timeout"
    return False, f"exit:{r.returncode}"


def write_pick_brief(picked: dict, screenshot_path, url: str) -> Path:
    """P4: pick 결과 → 워커 브리프 md. 반환 md 경로."""
    BRIEFS_DIR.mkdir(parents=True, exist_ok=True)
    ts = time.strftime("%Y%m%d-%H%M%S")
    md_path = BRIEFS_DIR / f"{ts}-pick.md"
    sel = picked.get("selector", "")
    text = picked.get("text", "")
    rect = picked.get("rect", {})
    lines = [
        f"# 디자인 모드 브리프 — {ts}",
        "",
        f"- 대상 URL: {url or picked.get('url', '')}",
        f"- 선택 요소 selector: `{sel}`",
        f"- 요소 텍스트: {text!r}",
        f"- 요소 위치(rect): x={rect.get('x')} y={rect.get('y')} w={rect.get('width')} h={rect.get('height')}",
        f"- 스크린샷: {screenshot_path or '(없음)'}",
        "",
        "## 수정 지시",
        "",
        "<!-- 사람이 채울 칸: 이 요소를 어떻게 바꿀지 구체적으로 적으세요 -->",
        "",
    ]
    md_path.write_text("\n".join(lines), encoding="utf8")
    return md_path


def _emit(obj):
    print(json.dumps(obj, ensure_ascii=False, indent=2))


# --- doctor ---
def cmd_doctor(a) -> int:
    checks = []

    bun = _which("bun")
    checks.append(("bun 실행파일", bool(bun), bun or "미설치"))

    node_modules = BROWSERD_DIR / "node_modules" / "playwright-core"
    dep_ok = node_modules.exists()
    checks.append(("playwright-core 설치", dep_ok, str(node_modules) if dep_ok else "미설치 — `bun install` 필요"))

    lock = (BROWSERD_DIR / "bun.lock").exists() or (BROWSERD_DIR / "bun.lockb").exists()
    checks.append(("lockfile 존재", lock, "bun.lock" if lock else "없음"))

    chrome = _chrome_available()
    checks.append(("Chrome/chromium 가용", chrome, "found" if chrome else "미설치 (channel 폴백 시 `bunx playwright install chromium`)"))

    server_ok = SERVER_TS.exists()
    checks.append(("server.ts 존재", server_ok, str(SERVER_TS)))

    # state 정합
    st = _read_state()
    if st is None:
        checks.append(("state 파일", True, "없음(정상 — lazy)"))
    else:
        alive = _pid_alive(st["pid"])
        checks.append(("state 정합(pid 생존)", alive, f"pid={st['pid']} port={st['port']} {'live' if alive else 'stale(교체됨)'}"))

    ok = all(c[1] for c in checks[:5])  # 핵심 5항목 (state 스테일은 비치명)
    print("browserd doctor:")
    for name, good, detail in checks:
        print(f"  [{'OK' if good else 'FAIL'}] {name}: {detail}")
    print(f"\n결과: {'PASS (exit 0)' if ok else 'FAIL (exit 1)'}")
    return EXIT_OK if ok else EXIT_OTHER


# --- 동사 실행 공통 ---
def run_verb(verb: str, args: dict, headless: bool) -> int:
    st, err = ensure_browserd(headless)
    if not st:
        audit(verb, args, None, EXIT_START_FAIL)
        _emit({"ok": False, "error": {"code": "START_FAIL", "message": err}})
        return EXIT_START_FAIL
    try:
        resp = rpc(st, verb, args)
    except urllib.error.URLError as e:
        audit(verb, args, None, EXIT_OTHER)
        _emit({"ok": False, "error": {"code": "RPC_FAIL", "message": str(e)}})
        return EXIT_OTHER

    if not resp.get("ok"):
        code = resp.get("error", {}).get("code", "ERROR")
        exit_code = EXIT_BY_ERROR.get(code, EXIT_OTHER)
        audit(verb, args, None, exit_code)
        _emit(resp)
        return exit_code

    result = resp.get("result", {})
    evidence_path = result.get("evidence_path")

    # verify FAIL → exit 5
    if verb == "verify" and result.get("verdict") == "FAIL":
        audit(verb, args, evidence_path, EXIT_VERIFY_FAIL)
        _emit(resp)
        return EXIT_VERIFY_FAIL

    audit(verb, args, evidence_path, EXIT_OK)
    _emit(resp)
    return EXIT_OK


def build_args(a) -> dict:
    """argparse 네임스페이스 → RPC args (None 제거).

    ★4-S-12 함정: 여기 등재하지 않은 인자는 argparse 가 파싱해도 **RPC 에 실리지 않는다**.
      서버는 인자 없이 동작하고 CLI 는 exit 0 으로 잘못된 결과를 낸다(exit0 거짓말 계열).
      신규 동사에 새 인자를 붙일 때마다 이 목록을 함께 갱신해야 하며, 회귀 핀이 이를 지킨다."""
    keys = [
        "url", "profile", "context", "ref", "selector", "value", "text", "key",
        "expression", "path", "timeout", "load", "expect_text", "expect_selector",
        "action", "actor", "evidence_dir", "full_page", "approved",
        # --- Phase 3 신규 ---
        "what", "attr", "property", "dx", "dy", "width", "height", "id",
        "function", "snapshot_after",
    ]
    out = {}
    for k in keys:
        v = getattr(a, k, None)
        if v is not None:
            out[k] = v
    return out


def main():
    p = _NoConflictParser(prog="javis_browser", description="browserd 배선 CLI (P1)")
    p.add_argument("--headless", action="store_true", help="browserd를 headless로 기동")
    # parser_class 전파 — 서브커맨드 인자 오류도 exit 9 로 떨어져야 한다(4-T-10).
    sub = p.add_subparsers(dest="cmd", required=True, parser_class=_NoConflictParser)

    sub.add_parser("doctor", help="설치·경로·버전 결정론 점검")
    sub.add_parser("start", help="browserd 기동")
    sub.add_parser("stop", help="browserd 종료")
    sub.add_parser("status", help="browserd 상태")

    def add_common(sp):
        sp.add_argument("--context", default=None)
        sp.add_argument(
            "--evidence-dir",
            dest="evidence_dir",
            default=None,
            help="evidence 번들 경로 — ~/.cys/browser/evidence/ 하위만 허용(상대 경로는 그 아래로 해석). 루트 이탈은 EVIDENCE_PATH_DENIED(18)",
        )

    sp = sub.add_parser("open"); sp.add_argument("url"); sp.add_argument("--profile", default=None); add_common(sp)
    sp = sub.add_parser("snapshot"); add_common(sp)
    sp = sub.add_parser("click"); sp.add_argument("--ref"); sp.add_argument("--selector"); sp.add_argument("--timeout", type=int); add_common(sp)
    sp = sub.add_parser("fill"); sp.add_argument("--ref"); sp.add_argument("--selector"); sp.add_argument("--value", required=True); sp.add_argument("--timeout", type=int); add_common(sp)
    sp = sub.add_parser("type"); sp.add_argument("--text", required=True); sp.add_argument("--ref"); sp.add_argument("--selector"); sp.add_argument("--timeout", type=int); add_common(sp)
    sp = sub.add_parser("press"); sp.add_argument("--key", required=True); add_common(sp)
    sp = sub.add_parser("eval"); sp.add_argument("--expression", required=True); add_common(sp)
    sp = sub.add_parser("screenshot"); sp.add_argument("--path", required=True); sp.add_argument("--full-page", dest="full_page", action="store_true", default=None); add_common(sp)
    sp = sub.add_parser("wait"); sp.add_argument("--selector"); sp.add_argument("--text"); sp.add_argument("--url"); sp.add_argument("--load"); sp.add_argument("--timeout", type=int); add_common(sp)
    sp = sub.add_parser("verify"); sp.add_argument("--expect-text", dest="expect_text", action="append", help="기대 텍스트(반복 지정 가능 — 전부 대조)"); sp.add_argument("--expect-selector", dest="expect_selector", action="append", help="기대 셀렉터(반복 지정 가능 — 전부 대조)"); add_common(sp)
    sp = sub.add_parser("control"); sp.add_argument("action", choices=["acquire", "release"]); sp.add_argument("--actor", choices=["agent", "human"], default=None); add_common(sp)
    # P2-a 관측: headful로 열고 관측 상태 반환. --profile human 은 CEO 결재 경유.
    sp = sub.add_parser("observe", help="headful 관측(사람이 창을 직접 봄)"); sp.add_argument("url"); sp.add_argument("--profile", default=None); add_common(sp)
    # SOT 헬퍼: observe --profile human https://notebooklm.google.com/ 축약(결재 경로 경유).
    sub.add_parser("sot", help="박사님 생각 SOT(NotebookLM) human 프로필 관측 — 결재 경유")
    # P4 디자인 모드: 사람이 요소 클릭 → 워커 브리프 md 생성.
    sp = sub.add_parser("pick", help="디자인 모드 — 요소 선택→브리프 md"); sp.add_argument("--timeout", type=int); add_common(sp)

    # ════════ Phase 3 신규 동사 (서버 dispatch 와 1:1 · README 3면 일치) ════════
    def add_target(sp):
        sp.add_argument("--ref"); sp.add_argument("--selector"); sp.add_argument("--timeout", type=int)

    def add_snap(sp):
        # 변경성 동사 공통 플래그 — 성공 후 스냅샷 동봉(4-S-11 부분 성공 계약).
        sp.add_argument("--snapshot-after", dest="snapshot_after", action="store_true", default=None,
                        help="성공 후 스냅샷을 결과에 동봉(실패 시 snapshot=null + snapshot_error)")

    # 내비게이션
    sp = sub.add_parser("goto", help="현재 컨텍스트를 이동(open 과 달리 컨텍스트를 만들지 않음)")
    sp.add_argument("url"); sp.add_argument("--timeout", type=int); add_snap(sp); add_common(sp)
    # ★`stop` 은 이미 "browserd 데몬 종료"가 선점한 이름이다 — 페이지 로딩 중지는 `stop-loading`
    #   으로 노출하고 서버 동사 `stop` 으로 옮긴다(VERB_ALIAS). 같은 이름을 재사용하면 데몬을
    #   죽이려던 명령이 로딩만 멈추는(또는 그 반대) 치명적 혼동이 된다.
    for v, h in (("back", "뒤로"), ("forward", "앞으로"), ("reload", "새로고침"), ("stop-loading", "페이지 로딩 중지")):
        sp = sub.add_parser(v, help=h); add_snap(sp); add_common(sp)

    # 조회 — what 은 위치인자(기존 `control <action>` 관례와 동일)
    sp = sub.add_parser("get", help="페이지 조회(웹 문자열은 UNTRUSTED 경계 아래로 반환)")
    sp.add_argument("what", choices=["url", "title", "text", "html", "value", "attr", "count", "box", "styles"])
    sp.add_argument("--attr", help="what=attr 일 때 속성 이름")
    sp.add_argument("--property", action="append", help="what=styles 일 때 CSS 속성(반복 지정 가능)")
    add_target(sp); add_common(sp)

    # 상호작용
    for v, h in (("dblclick", "더블클릭"), ("hover", "마우스 올리기"), ("focus", "포커스"),
                 ("check", "체크박스 켜기"), ("uncheck", "체크박스 끄기")):
        sp = sub.add_parser(v, help=h); add_target(sp); add_snap(sp); add_common(sp)
    sp = sub.add_parser("select", help="select 요소 값 선택")
    sp.add_argument("--value", action="append", required=True, help="선택할 값(반복 지정 가능)")
    add_target(sp); add_snap(sp); add_common(sp)
    sp = sub.add_parser("scroll", help="스크롤(대상만 주면 화면 안으로)")
    sp.add_argument("--dx", type=int); sp.add_argument("--dy", type=int)
    add_target(sp); add_snap(sp); add_common(sp)

    # 탭
    sp = sub.add_parser("tab", help="탭 목록·생성·전환·닫기")
    sp.add_argument("action", choices=["list", "new", "switch", "activate", "close"])
    sp.add_argument("--id", help="대상 탭 id(tab list 로 확인)")
    sp.add_argument("--url", help="tab new 시 이동할 주소")
    sp.add_argument("--timeout", type=int); add_common(sp)

    # 뷰포트
    sp = sub.add_parser("viewport", help="뷰포트 지정/해제(지정은 사람 pane 리사이즈보다 우선)")
    sp.add_argument("action", nargs="?", choices=["reset"], default=None, help="reset=고정 해제")
    sp.add_argument("--width", type=int); sp.add_argument("--height", type=int)
    add_common(sp)

    a = p.parse_args()
    # CLI 는 브라우저 관례어 `switch` 를 받고 서버 계약어 `activate` 로 옮긴다(서버 단일 명칭 유지).
    if getattr(a, "cmd", None) == "tab" and getattr(a, "action", None) == "switch":
        a.action = "activate"
    headless = a.headless

    if a.cmd == "doctor":
        sys.exit(cmd_doctor(a))

    if a.cmd == "start":
        st, err = ensure_browserd(headless)
        if not st:
            _emit({"ok": False, "error": {"code": "START_FAIL", "message": err}})
            sys.exit(EXIT_START_FAIL)
        if st.get("transport") == "broker":
            _emit({"ok": True, "result": st["status"]})
        else:
            _emit({"ok": True, "result": {"pid": st["pid"], "port": st["port"], "development": True}})
        sys.exit(EXIT_OK)

    if a.cmd == "stop":
        ok, message = stop_browserd()
        _emit({"ok": ok, "result" if ok else "error": message})
        sys.exit(EXIT_OK if ok else EXIT_START_FAIL)

    # --- sot = observe --profile human notebooklm 축약 ---
    if a.cmd == "sot":
        sys.exit(cmd_observe("sot", NOTEBOOKLM_URL, "human", None, headless))

    # --- observe (P2-a) ---
    if a.cmd == "observe":
        sys.exit(cmd_observe("observe", a.url, a.profile, a.evidence_dir, headless))

    # --- pick (P4) ---
    if a.cmd == "pick":
        sys.exit(cmd_pick(a, headless))

    args = build_args(a)

    # --- human 프로필 결재 게이트 (open 등 --profile human) ---
    if getattr(a, "profile", None) == "human":
        rc = gate_human(a.cmd, args)
        if rc is not None:
            sys.exit(rc)

    sys.exit(run_verb(VERB_ALIAS.get(a.cmd, a.cmd), args, headless))


def gate_human(verb: str, args: dict):
    """--profile human 요청에 CEO 결재를 강제. 통과 시 args['approved']=True 세팅 후 None 반환.
    거부/오류 시 emit+audit 후 exit 코드 반환(EXIT_APPROVAL_REQUIRED)."""
    url = args.get("url", "")
    approved, decision = request_human_approval(verb, url)
    audit(verb, args, None, EXIT_APPROVAL_REQUIRED if not approved else EXIT_OK,
          extra={"human_approval": decision})
    if not approved:
        msg = {
            "no_cys": "cys 바이너리 부재 — human 프로필 거부(fail-closed)",
            "deny": "CEO 결재 거부(deny)",
            "timeout": "CEO 결재 타임아웃(미결재)",
        }.get(decision, f"결재 실패({decision})")
        _emit({"ok": False, "error": {"code": "APPROVAL_REQUIRED", "message": msg}})
        return EXIT_APPROVAL_REQUIRED
    args["approved"] = True
    return None


def guard_headful_required(verb_label: str, args: dict) -> int:
    """headful이 본질인 동사(observe·sot·pick) 앞 게이트.

    ensure_browserd는 live state가 있으면 headless 인자를 무시하고 재사용한다. 그래서 GUI cast가
    browserd를 --headless로 먼저 띄운 뒤 observe/pick/sot를 부르면 headful을 요구했는데도 살아있는
    headless가 재사용되어 **창이 안 뜨는데 exit 0 성공**을 보고한다(exit0 거짓말). 무음 거짓 성공
    대신 fail-loud 거부한다. 남의 세션은 죽이지 않는다(자동 kill·재기동 금지 — 세션 공유 원칙).
    구버전 state에 headless 키가 없으면 False로 간주해 기존 거동을 유지한다."""
    if not _development_mode():
        audit(verb_label, args, None, EXIT_START_FAIL)
        _emit({"ok": False, "error": {
            "code": "HEADFUL_UNAVAILABLE",
            "message": (
                "production Browser Runtime은 shared-default in-pane(headless) 전용 — "
                "일반 탐색은 'cys browser <url>' 또는 GUI Browser pane을 사용하고, "
                "headful 개발 관측은 CYS_BROWSER_DEV=1에서만 실행하라"
            ),
        }})
        return EXIT_START_FAIL

    st = _live_state()
    if st and st.get("headless", False):
        audit(verb_label, args, None, EXIT_START_FAIL)
        _emit({"ok": False, "error": {
            "code": "HEADLESS_ACTIVE",
            "message": (
                f"browserd가 headless로 상주 중(pid {st['pid']}) — headful 관측 불가. "
                f"개발 cast pane을 닫고 'javis_browser.py stop' 후 재시도하라"
            ),
        }})
        return EXIT_START_FAIL
    return 0


def cmd_observe(verb_label: str, url: str, profile, evidence_dir, headless_flag: bool) -> int:
    """P2-a 관측 — headful로 열고 관측 상태 반환. human 프로필은 결재 경유.
    관측은 headful이 본질(사람이 창을 봄) → --headless 무시하고 headful 강제."""
    # 결재(gate_human)보다 먼저 — 어차피 못 여는 관측에 CEO 결재를 소모하지 않는다.
    rc = guard_headful_required(verb_label, {"url": url, "profile": profile or "agent"})
    if rc:
        return rc
    args = {"url": url}
    if evidence_dir:
        args["evidence_dir"] = evidence_dir
    if profile == "human":
        args["profile"] = "human"
        # ★PRE-2: human 은 전용 cid "human" 에만 산다. 여기서 context 를 안 실으면 서버가
        # "default" 로 낙하시켜 **로그인 세션이 모든 cast pane·에이전트 동사의 기본 컨텍스트를
        # 점유**한다(sot 한 번이면 지구본 버튼이 human 화면을 가리키게 된다).
        # 서버도 profile=human 이면 cid 를 강제 분리하지만, CLI 도 명시해 두 층에서 고정한다.
        args["context"] = "human"
        rc = gate_human("observe", args)
        if rc is not None:
            return rc
    # 관측은 headful 강제(headless면 사람이 볼 창이 없음).
    return run_verb("observe", args, headless=False)


def cmd_pick(a, headless_flag: bool) -> int:
    """P4 디자인 모드 — 요소 선택→브리프 md. headless엔 클릭할 사람이 없어 거부."""
    if headless_flag:
        _emit({"ok": False, "error": {"code": "PICK_HEADLESS", "message": "pick은 headful 필요 — 사람이 요소를 클릭한다(--headless 불가)"}})
        return EXIT_OTHER
    # live browserd가 headless면 오버레이를 클릭할 창이 없어 60초 뒤 PICK_TIMEOUT — 미리 거부.
    rc = guard_headful_required("pick", {})
    if rc:
        return rc
    BRIEFS_DIR.mkdir(parents=True, exist_ok=True)
    ts = time.strftime("%Y%m%d-%H%M%S")
    shot = str(BRIEFS_DIR / f"{ts}-pick.png")
    args = {"path": shot}
    if a.context:
        args["context"] = a.context
    if a.timeout:
        args["timeout"] = a.timeout
    st, err = ensure_browserd(headless=False)
    if not st:
        audit("pick", args, None, EXIT_START_FAIL)
        _emit({"ok": False, "error": {"code": "START_FAIL", "message": err}})
        return EXIT_START_FAIL
    try:
        resp = rpc(st, "pick", args)
    except urllib.error.URLError as e:
        audit("pick", args, None, EXIT_OTHER)
        _emit({"ok": False, "error": {"code": "RPC_FAIL", "message": str(e)}})
        return EXIT_OTHER
    if not resp.get("ok"):
        code = resp.get("error", {}).get("code", "ERROR")
        exit_code = EXIT_BY_ERROR.get(code, EXIT_OTHER)
        audit("pick", args, None, exit_code)
        _emit(resp)
        return exit_code
    result = resp.get("result", {})
    picked = result.get("picked", {})
    md_path = write_pick_brief(picked, result.get("screenshot_path"), result.get("url", ""))
    audit("pick", args, str(md_path), EXIT_OK)
    _emit({"ok": True, "result": {"picked": picked, "brief": str(md_path), "screenshot": result.get("screenshot_path")}})
    print(f"\n브리프: {md_path}")
    return EXIT_OK


if __name__ == "__main__":
    main()
