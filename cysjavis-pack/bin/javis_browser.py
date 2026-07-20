#!/usr/bin/env python3
"""javis_browser.py — browserd 배선 CLI (P1 · 팩 인큐베이션).

설계: _research/cmux-distillation/DESIGN-v1.2-2026-07-19.md §2B.
- browserd(bun+playwright-core)에 127.0.0.1 HTTP로 동사를 중계한다.
- browserd 미기동 시 자동 기동(백그라운드 spawn + state 대기, 타임아웃 20s).
- 모든 동사를 ~/.cys/browser/audit.jsonl 에 append (reviewer2 감사 대상).
- P1은 cys 데몬·제품 코드 무변경. 제품 `cys browser` 승격은 제품화 게이트 이후.

결정론 exit 코드:
  0 성공 · 2 BUSY · 3 APPROVAL_REQUIRED · 4 기동실패 · 5 verify FAIL · 6 HUMAN_ACTIVE
  · 7 HUMAN_PROFILE_PROTECTED · 8 PICK_TIMEOUT · 1 기타

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

# 에러 코드 → exit 코드 매핑
EXIT_BY_ERROR = {
    "BUSY": 2,
    "APPROVAL_REQUIRED": 3,
    "HUMAN_ACTIVE": 6,
    "HUMAN_PROFILE_PROTECTED": 7,
    "PICK_TIMEOUT": 8,
}
EXIT_OK = 0
EXIT_OTHER = 1
EXIT_START_FAIL = 4
EXIT_VERIFY_FAIL = 5
EXIT_APPROVAL_REQUIRED = 3
EXIT_HUMAN_PROFILE_PROTECTED = 7
EXIT_PICK_TIMEOUT = 8

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


def start_browserd(headless: bool, timeout: float = 20.0):
    """browserd를 백그라운드 spawn하고 live state가 뜰 때까지 대기."""
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


def _cross_process_lock(fh):
    """배타 파일락(블로킹) — POSIX fcntl / Windows msvcrt. 미지원 플랫폼은 best-effort no-op."""
    try:
        import fcntl
        fcntl.flock(fh.fileno(), fcntl.LOCK_EX)
        return
    except ImportError:
        pass
    try:
        import msvcrt
        msvcrt.locking(fh.fileno(), msvcrt.LK_LOCK, 1)
    except Exception:
        pass  # 락 미지원 → best-effort(락 없던 종전 거동으로 폴백)


def ensure_browserd(headless: bool):
    st = _live_state()
    if st:
        return st, None
    # ★동시 cold-start 경쟁 방지(리뷰어1 F1): ensure_browserd는 락 없는 check-then-act(TOCTOU)라
    # 두 진입(지구본 버튼 + cys browser)이 state.json 기록 전 창에 겹치면 browserd(bun+Chromium)가
    # 2개 뜨고, 사람과 에이전트가 서로 다른 창을 조작해 '하나의 브라우저 공유'가 무음 붕괴한다.
    # 파일락 임계구역 — 락 안에서 재확인(double-check) 후에만 spawn한다. start_browserd가 state
    # 등장까지 대기하므로, 락을 기다린 2차 호출은 fast-path로 1차의 live state를 재사용한다.
    BROWSER_ROOT.mkdir(parents=True, exist_ok=True)
    with open(BROWSER_ROOT / "browserd.lock", "w") as lf:
        _cross_process_lock(lf)
        st = _live_state()
        if st:
            return st, None
        return start_browserd(headless)


def rpc(st, verb: str, args: dict):
    url = f"http://127.0.0.1:{st['port']}/{st['token']}/rpc"
    data = json.dumps({"verb": verb, "args": args}).encode("utf8")
    req = urllib.request.Request(url, data=data, headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf8"))


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
    """argparse 네임스페이스 → RPC args (None 제거)."""
    keys = [
        "url", "profile", "context", "ref", "selector", "value", "text", "key",
        "expression", "path", "timeout", "load", "expect_text", "expect_selector",
        "action", "actor", "evidence_dir", "full_page", "approved",
    ]
    out = {}
    for k in keys:
        v = getattr(a, k, None)
        if v is not None:
            out[k] = v
    return out


def main():
    p = argparse.ArgumentParser(prog="javis_browser", description="browserd 배선 CLI (P1)")
    p.add_argument("--headless", action="store_true", help="browserd를 headless로 기동")
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("doctor", help="설치·경로·버전 결정론 점검")
    sub.add_parser("start", help="browserd 기동")
    sub.add_parser("stop", help="browserd 종료")
    sub.add_parser("status", help="browserd 상태")

    def add_common(sp):
        sp.add_argument("--context", default=None)
        sp.add_argument("--evidence-dir", dest="evidence_dir", default=None)

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

    a = p.parse_args()
    headless = a.headless

    if a.cmd == "doctor":
        sys.exit(cmd_doctor(a))

    if a.cmd == "start":
        st, err = ensure_browserd(headless)
        if not st:
            _emit({"ok": False, "error": {"code": "START_FAIL", "message": err}})
            sys.exit(EXIT_START_FAIL)
        _emit({"ok": True, "result": {"pid": st["pid"], "port": st["port"]}})
        sys.exit(EXIT_OK)

    if a.cmd == "stop":
        st = _live_state()
        if not st:
            _emit({"ok": True, "result": "미기동"})
            sys.exit(EXIT_OK)
        try:
            os.kill(st["pid"], 15)
        except ProcessLookupError:
            pass
        _emit({"ok": True, "result": f"SIGTERM → pid {st['pid']}"})
        sys.exit(EXIT_OK)

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

    sys.exit(run_verb(a.cmd, args, headless))


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


def cmd_observe(verb_label: str, url: str, profile, evidence_dir, headless_flag: bool) -> int:
    """P2-a 관측 — headful로 열고 관측 상태 반환. human 프로필은 결재 경유.
    관측은 headful이 본질(사람이 창을 봄) → --headless 무시하고 headful 강제."""
    args = {"url": url, "context": None}
    args = {k: v for k, v in args.items() if v is not None}
    if evidence_dir:
        args["evidence_dir"] = evidence_dir
    if profile == "human":
        args["profile"] = "human"
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
