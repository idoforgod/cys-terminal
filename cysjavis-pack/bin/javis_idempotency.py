#!/usr/bin/env python3
"""관찰 명령 부작용 금지 — cys 헬스 probe 멱등성 계약 (AGENTREACH OPP-21).

AR PHIL-04('관찰이 상태를 바꾸지 않는다')의 cysjavis 일반화. AR 의 python pytest spy 를
복붙하지 않고 stdlib(unittest.mock + ast)로 재현한다 — 클린룸 stdlib 포트.

세 가지 계약을 실행 가능 테스트(`--self-test`)로 잠근다:
  1. orchestra.cmd_check 는 관찰 전용 — calls ∩ MUTATE_VERBS == ∅ (negative assertion).
  2. preflight C12.daemon 관찰 단계(fix=False)는 비멱등 0 — cysd Popen 미발생.
  3. coverage_battery 는 관찰 전용 — POST/PUT/DELETE·--cookies·login·yt-dlp 다운로드 토큰 0(AST).

★verdict 는 score 금지(producer≠evaluator) — 위반 시 `verb` 또는 `file:line` evidence 출력.

Run:  python3 javis_idempotency.py --self-test     # 결정론 배터리(preflight C53 이 호출)
      python3 javis_idempotency.py classify 'cys send --to worker x'   # verb 분류 디버그
"""
from __future__ import annotations

import ast
import os
import subprocess
import sys
from unittest import mock

# ── A. 멱등 verb 분류기 (단일 진실) ──────────────────────────────────────────
# 시드셋은 실측 `cys actions`(clap 정의 = self-describing SOT) 표면에 동기화한다.
# self-test 가 `cys actions ⊆ OBSERVE ∪ MUTATE ∪ UNCLASSIFIED_ALLOWED` 를 검증해
# '덮는다'는 주장을 실측 가능하게 만든다(과장 차단·PHIL-09 negative knowledge).
OBSERVE_VERBS = frozenset({          # 부작용 없음 — 관찰 경로 허용
    "ping", "identify", "actions", "list", "status",
    "gate-check", "queue", "read-screen", "attach", "events",
    "ps", "health-rules", "recall", "surface-role", "todo-path",
    # 'agent-detect' 읽기 전용 실측 근거(3중): ①Rust 구현 run_agent_detect(cys.rs:4827-4943)는
    # agents.json read_to_string + which/access(2) X_OK 조회 + println 뿐 — 쓰기·데몬 RPC·기동 0.
    # ②`cys agent-detect --help` = "데몬 무의존"이고 `--socket /nonexistent` 로도 exit 0.
    # ③실측: 3회 연속 실행 후 ~/.cys 쓰기 0(대조군 동일). cmd_check 관찰 경로의 정당한 호출
    # (어댑터 감지 단일 오라클 · javis_orchestra.py:225).
    "agent-detect",
    # ★(0.14.31 · WP-1 H-2 · CONTRACTS §C) 'gate-corpus' — 첫기동 관문 코퍼스 보고(신설).
    # 읽기 전용 실측 근거(3중): ①Rust 구현 run_gate_corpus(cys.rs)는 resolve_gate_corpus →
    # first_run_gates::report_json → println 뿐 — 데몬 RPC·서브프로세스·send_key·파일 쓰기 0
    # (소스 핀 gate_corpus_verb_is_a_daemon_free_subprocess_free_observation_source_pin 이 집행).
    # ②실측 2026-09-08: `CYS_SOCKET=/nonexistent/nope.sock cys gate-corpus --json` 이 exit 0
    # (데몬 무의존). ③버전을 스스로 재지 않는다 — `--detected-version` 을 호출부가 넣는다.
    # 소비자: preflight C82.gate-corpus-drift(팩 레인).
    "gate-corpus",
})
MUTATE_VERBS = frozenset({           # 상태 변경 — repair/실행 의도 뒤에서만
    "new-surface", "send", "send-key", "set-status",
    "usage-register", "usage-report-stdin", "usage-event-stdin",
    "pause", "resume", "cycle-agent", "node-recover", "restore",
    "reinject", "watch", "attest", "daemon", "resize", "close-surface",
    "run", "kill", "add-health-rule", "feed", "learn", "init-pack",
    "skill", "persona", "schedule", "claim-role", "launch-agent", "boot",
    # ★(0.14.31 · CONTRACTS B-6) reclaim-role(WP-4 · 데몬 역할 재결합 RPC) 은 상태 변경.
    "reclaim-role",
    # 'new-split' 부재 — 실제 cys CLI 에 없음(cmux 잔재·CLAUDE.md 치환표상 폐기 verb).
})

# ★(0.14.31 · WP-5 · CONTRACTS B-6) 서브동사 세분 분류 — `queue` 동사는 OBSERVE 이지만 그 아래
# revive/drop/clear/deliver 는 상태 변경(만료 항목 재활성·폐기·큐 인멸·강제 배달)이다. verb 단위
# 분류만 있으면 `cys queue drop <id>` 가 관찰 경로를 통과한다. (verb, subverb) 쌍이 verb 분류보다
# **우선**한다. 목록 밖 서브동사(`queue <unknown>`)는 fail-closed(unknown → 관찰 경로 거부).
MUTATE_SUBVERBS = frozenset({
    ("queue", "revive"), ("queue", "drop"), ("queue", "clear"), ("queue", "deliver"),
})
OBSERVE_SUBVERBS = frozenset({
    ("queue", "list"),
})
# 서브동사를 갖는 verb 집합 — 이 verb 는 서브동사 없이는 분류하지 않는다(clap 도 서브커맨드 필수).
SUBVERB_REQUIRED = frozenset({"queue"})

# 현재 cys 표면에 존재하나 멱등 분류가 모호해 의도적으로 미분류로 둔 verb(정직한 미커버).
# 분류 미정이라도 self-test 표면 커버리지가 RED 되지 않게 하되, default-deny 로 관찰 경로에선
# unknown→mutate 강등돼 거부된다(안전). 무한 확장 차단을 위해 self-test 가 상한을 박제한다.
UNCLASSIFIED_ALLOWED = frozenset(set())

# AR fail-safe 정신: self-heal upsert(예: SESSION_STATE 부재 시 생성)는 관찰도 mutate 도 아닌
# 별도 분류로 면제. javis_session.py:5-7 'well-known idempotent upsert 는 보호 대상 아님' 근거.
# 면제 자체를 박제해 무한 확장(계약 형해화) 차단 — self-test 가 상한(<=3) 검증.
SELF_HEAL_EXEMPT = ("todo-path",)    # 부재 시 결정론 생성하는 자기수복 upsert


def classify_cys_verb(argv):
    """'cys <verb> ...' 에서 verb 추출 → "observe" | "mutate" | "unknown".

    unknown 은 보수적으로 'unknown' 으로 반환하되, is_observe_only 가 fail-closed 로 거부한다
    (safety classify_url default-deny 동형). 순수 함수 — 입력 argv 만으로 판정."""
    verb = _extract_verb(argv)
    if verb is None:
        return "unknown"
    if verb in SUBVERB_REQUIRED:
        sub = _extract_subverb(argv)
        if sub is None:
            return "unknown"                 # `cys queue` 단독 — clap 도 거부(fail-closed)
        if (verb, sub) in MUTATE_SUBVERBS:
            return "mutate"
        if (verb, sub) in OBSERVE_SUBVERBS:
            return "observe"
        return "unknown"                     # 미지 서브동사 — 관찰 경로 거부(default-deny)
    if verb in OBSERVE_VERBS:
        return "observe"
    if verb in MUTATE_VERBS:
        return "mutate"
    return "unknown"


def _is_cys_binary(first):
    """argv[0] 이 cys 바이너리인가 — Windows(Git Bash) 의 `cys.exe`·`cys.cmd` 도 cys 다."""
    # 구분자 두 종류 모두에서 마지막 토큰 — macOS/Linux 파이썬은 백슬래시를 구분자로 보지 않아
    # Windows 경로(`C:\\...\\cys.exe`)의 basename 이 전체 경로가 된다(self-test 는 macOS CI 에서도 돈다).
    base = str(first).replace("\\", "/").rsplit("/", 1)[-1]
    low = base.lower()
    for ext in (".exe", ".cmd", ".bat"):
        if low.endswith(ext):
            low = low[: -len(ext)]
            break
    return low == "cys"


def _extract_subverb(argv):
    """`cys <verb> <subverb> …` 에서 서브동사(verb 다음 첫 비옵션 토큰)를 추출. 없으면 None."""
    if not argv or not _is_cys_binary(argv[0]):
        return None
    i, seen_verb = 1, False
    while i < len(argv):
        tok = str(argv[i])
        if tok == "--socket":
            i += 2
            continue
        if tok.startswith("-"):
            i += 1
            continue
        if not seen_verb:
            seen_verb = True
            i += 1
            continue
        return tok
    return None


def _extract_verb(argv):
    """argv 리스트에서 cys 서브커맨드(verb)를 추출. cys 바이너리/옵션을 건너뛴다.

    예: ['/opt/homebrew/bin/cys', 'status', '--json'] → 'status'
        ['cys', '--socket', '/x', 'send', ...]          → 'send'
    cys 호출이 아니면(예: yt-dlp·python) None."""
    if not argv:
        return None
    if not _is_cys_binary(argv[0]):        # ★(0.14.31) Windows Git Bash `cys.exe` 도 cys 다
        return None
    i = 1
    while i < len(argv):
        tok = str(argv[i])
        if tok == "--socket":          # 값 1개를 먹는 전역 옵션
            i += 2
            continue
        if tok.startswith("-"):        # 기타 플래그
            i += 1
            continue
        return tok                     # 첫 비옵션 토큰 = verb
    return None


def is_observe_only(argv):
    """관찰 경로 허용 여부 — observe 만 True(unknown·mutate 는 fail-closed 거부)."""
    return classify_cys_verb(argv) == "observe"


def assert_observe_phase(calls):
    """관찰 단계 호출 로그(argv 리스트들)에서 mutate/unknown cys verb 부재 단언.

    AR 의 "['doctor'] not in calls" 의 cys 일반화: calls 중 cys 호출은 전부 observe 여야 한다.
    cys 가 아닌 호출(python·yt-dlp 등)은 무시한다. 반환: (ok, violations[{verb, argv}])."""
    violations = []
    for argv in calls:
        verb = _extract_verb(argv)
        if verb is None:               # cys 호출 아님 — 본 계약 범위 밖
            continue
        kind = classify_cys_verb(argv)
        if kind != "observe":          # mutate 또는 unknown(fail-closed)
            violations.append({"verb": verb, "kind": kind, "argv": list(argv)})
    return (len(violations) == 0, violations)


# ── B. spy 하니스 (self-test 내장) ───────────────────────────────────────────
class _SubprocessSpy:
    """subprocess.run/Popen 을 가로채 argv 를 calls 에 기록하고 가짜 성공을 반환.

    네트워크·프로세스 0 — 완전 격리·결정론(cys 미설치/CI 무데몬 환경에서도 통과)."""

    def __init__(self, stdout=b'{"surfaces":[]}'):
        self.calls = []
        self.popen_calls = []
        self._stdout = stdout

    def run(self, args, *a, **kw):
        self.calls.append(list(args) if isinstance(args, (list, tuple)) else [args])

        class _R:
            returncode = 0
            stdout = self._stdout
            stderr = b""
        return _R()

    def popen(self, args, *a, **kw):
        self.popen_calls.append(list(args) if isinstance(args, (list, tuple)) else [args])

        class _P:
            pid = 0

            def wait(self_inner, *a, **k):
                return 0
        return _P()


def _spy_cmd_check():
    """orchestra.cmd_check 를 spy 로 감싸 calls ∩ MUTATE == ∅ 박제(회귀 잠금).

    누군가 cmd_check 에 launch-agent/boot/send/set-status 를 끼우면 즉시 RED."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import javis_orchestra as orch
    spy = _SubprocessSpy()

    class _Args:
        pass
    import contextlib
    import io
    with mock.patch.object(orch.subprocess, "run", side_effect=spy.run), \
            contextlib.redirect_stdout(io.StringIO()):   # cmd_check 진단 출력 흡수
        orch.cmd_check(_Args())
    ok, violations = assert_observe_phase(spy.calls)
    if not ok:
        raise AssertionError(
            "cmd_check 가 관찰 단계에서 mutate/unknown verb 호출: %s" % violations)
    # 실측 양성 핀(화이트리스트): cmd_check 의 cys 호출은 'status'(관제 스냅샷 · cys_status)와
    # 'agent-detect'(어댑터 감지 단일 오라클 · javis_orchestra.py:225) 둘뿐이어야 한다.
    # observe 분류와 *별개 층위*라 셋째 verb 는 observe 로 분류되더라도 여기서 RED 다.
    #   ★`list` 추가(2026-09-04 부트 v2 W-A · R1 codex #1): 역할 **주소 레지스트리** 판정이
    #     `cys list`(surface.list RPC)를 소비한다. `cys status`(프로세스 표 스냅샷)와 **다른
    #     자료원**이어야 주소성 게이트가 공허해지지 않기 때문이다 — 같은 집합을 두 번 보면
    #     불일치가 구조적으로 발생할 수 없다. `list` 는 순수 관찰 verb 라 이 화이트리스트의
    #     목적(관찰 단계에서 mutate 금지)과 충돌하지 않는다.
    _EXPECTED_CHECK_VERBS = frozenset({"status", "agent-detect", "list"})
    cys_calls = [c for c in spy.calls if _extract_verb(c) is not None]
    assert cys_calls, "cmd_check 가 cys status 조차 호출하지 않음(spy 미작동 의심)"
    assert all(_extract_verb(c) in _EXPECTED_CHECK_VERBS for c in cys_calls), \
        "cmd_check 의 cys 호출이 %s 외 verb 포함: %s" % (
            sorted(_EXPECTED_CHECK_VERBS), cys_calls)
    assert any(_extract_verb(c) == "status" for c in cys_calls), \
        "cmd_check 가 관제 스냅샷(cys status)을 호출하지 않음(핀 약화 방지)"


def _spy_negative_assertion_self_attack():
    """자기공격 변이검증(defensive-security-gate): 금지 verb 를 calls 에 주입하면 RED 인가."""
    poisoned = [["cys", "status", "--json"], ["cys", "launch-agent", "--role", "worker"]]
    ok, violations = assert_observe_phase(poisoned)
    assert not ok, "주입된 launch-agent 를 negative assertion 이 못 잡음(spy 무력)"
    assert violations[0]["verb"] == "launch-agent", "위반 evidence verb 부정확"


def _spy_preflight_daemon_observe_phase():
    """preflight C12.daemon 관찰 단계(fix=False)는 cysd Popen 0(비멱등 0) 박제.

    AR 'doctor 자동기동 회피' 의 cys 판. fix=True 일 때만 Popen≤1 허용(의도된 수리)."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import javis_preflight as pf
    spy = _SubprocessSpy(stdout=b"")          # ping 성공 모킹(returncode 0)
    # fix=False: 관찰 단계 — cysd Popen 은 절대 발생하면 안 된다.
    pre = pf.Preflight(fix=False, skips=[], mode="report")
    with mock.patch.object(pf.subprocess, "run", side_effect=spy.run), \
            mock.patch.object(pf.subprocess, "Popen", side_effect=spy.popen):
        pre.c12_daemon()
    assert spy.popen_calls == [], \
        "C12.daemon 관찰 단계(fix=False)가 Popen 호출(비멱등): %s" % spy.popen_calls
    # spy.run 의 가짜 ping 성공으로 PASS 경로를 타며 Popen 분기 미진입을 실측 확인.


# ── B-3. coverage_battery 정적 AST 검사 ──────────────────────────────────────
# 보고서 라벨 경로(engine/tests/)는 부정확 — 실측: SOT=skills/insane-search/tests/.
# 두 후보를 모두 탐색하고, 부재 시 graceful skip(LIVE 미벤더 환경 정합).
_COVERAGE_BATTERY_CANDIDATES = (
    os.path.join("skills", "insane-search", "tests", "coverage_battery.py"),
    os.path.join("skills", "insane-search", "engine", "tests", "coverage_battery.py"),
)
# 관찰 전용 위반 토큰(코드 토큰 대상·소문자). 두 부류로 나눠 false-positive 차단:
#  - EXACT: HTTP 변경동사 문자열 리터럴 등 — 정확히 일치해야 위반(position·output 오탐 차단).
#  - SUBSTR: CLI 플래그·메서드 호출 — 부분 문자열 일치(--cookies·cookiefile 등).
_FORBIDDEN_EXACT = frozenset({"post", "put", "delete", "login"})
_FORBIDDEN_SUBSTR = ("--cookies", "cookiefile", "--extract-audio")


def _find_coverage_battery():
    """pack 루트 기준 coverage_battery 후보 경로 중 실재하는 것 반환(없으면 None)."""
    here = os.path.dirname(os.path.abspath(__file__))
    pack_root = os.path.dirname(here)              # bin/ 의 상위 = pack 루트
    for rel in _COVERAGE_BATTERY_CANDIDATES:
        p = os.path.join(pack_root, rel)
        if os.path.isfile(p):
            return p
    return None


def _code_tokens(src):
    """소스에서 *실행 코드* 토큰만 추출(주석·docstring 제외) → [(text_lower, lineno)].

    프로세스/HTTP 호출 인자(문자열 리터럴·식별자·연산자)는 보존하되, 산문(주석·docstring)은
    제외한다 → 관찰 전용 계약 헤더 같은 prose 가 금지토큰을 false-trip 하지 않는다.
    tokenize 가 실패하면(드묾) None 을 반환해 호출측이 전체 소스 폴백하게 한다."""
    import io
    import tokenize
    toks = []
    try:
        gen = tokenize.generate_tokens(io.StringIO(src).readline)
        prev_meaningful = None     # docstring 판별용 직전 유의미 토큰 타입
        for t in gen:
            if t.type in (tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE,
                          tokenize.INDENT, tokenize.DEDENT, tokenize.ENCODING,
                          tokenize.ENDMARKER):
                continue
            if t.type == tokenize.STRING:
                # docstring(문장 시작 위치의 bare string)은 제외, 호출 인자 문자열은 보존.
                if prev_meaningful in (None, tokenize.NEWLINE, tokenize.INDENT,
                                       tokenize.DEDENT):
                    prev_meaningful = t.type
                    continue
            toks.append((t.string.lower(), t.start[0]))
            prev_meaningful = t.type
        return toks
    except tokenize.TokenError:
        return None


def scan_coverage_battery(path):
    """coverage_battery 소스를 읽어 금지 토큰·yt-dlp 다운로드(-o/--output) 부재 단언.

    네트워크 미실행(정적 AST 파싱 + 코드토큰 검사 — 주석·docstring 제외).
    반환: (ok, violations[str])."""
    with open(path, "r", encoding="utf-8") as f:
        src = f.read()
    violations = []
    base = os.path.basename(path)
    code = _code_tokens(src)
    if code is None:                   # tokenize 실패 폴백 — 전체 소스 보수 검사
        low = src.lower()
        for tok in tuple(_FORBIDDEN_SUBSTR) + tuple('"%s"' % e for e in _FORBIDDEN_EXACT):
            idx = low.find(tok)
            if idx != -1:
                violations.append("%s:%d 금지토큰 %r(폴백 전체검사)"
                                  % (base, src.count("\n", 0, idx) + 1, tok))
    else:
        # 금지토큰을 *코드 토큰* 텍스트에 한해 검사(prose 면제).
        for text, ln in code:
            stripped = text.strip('"\'')         # 문자열 리터럴 따옴표 제거
            if stripped in _FORBIDDEN_EXACT:     # HTTP 변경동사 등 — 정확 일치
                violations.append("%s:%d 금지 변경동사/세션 %r" % (base, ln, stripped))
            for sub in _FORBIDDEN_SUBSTR:        # CLI 플래그 등 — 부분 일치
                if sub in stripped:
                    violations.append("%s:%d 금지토큰 %r" % (base, ln, sub))
    # AST: yt-dlp 호출에 다운로드 플래그(-o/--output)가 없는지 검사(--skip-download 만 허용).
    try:
        tree = ast.parse(src)
    except SyntaxError as e:
        return (False, ["%s AST 파싱 실패: %s" % (os.path.basename(path), e)])
    for node in ast.walk(tree):
        if isinstance(node, ast.List):
            elts = [e.value for e in node.elts
                    if isinstance(e, ast.Constant) and isinstance(e.value, str)]
            if elts and elts[0] == "yt-dlp":
                for bad in ("-o", "--output"):
                    if bad in elts:
                        ln = getattr(node, "lineno", 0)
                        violations.append(
                            "%s:%d yt-dlp 다운로드 플래그 %r(--skip-download 위반)"
                            % (os.path.basename(path), ln, bad))
    return (len(violations) == 0, violations)


def _ast_coverage_battery():
    """coverage_battery 정적 검사 self-test. 부재 시 graceful skip(LIVE 미벤더 정합)."""
    path = _find_coverage_battery()
    if path is None:
        return "SKIP(coverage_battery 미벤더)"
    ok, violations = scan_coverage_battery(path)
    if not ok:
        raise AssertionError("coverage_battery 관찰 전용 위반: %s" % violations)
    return "OK(%s)" % os.path.basename(path)


def _ast_self_attack():
    """자기공격: 금지 토큰을 가진 가짜 소스를 만들면 scan 이 RED 인가."""
    import tempfile
    poisoned = (
        'import subprocess\n'
        'def f():\n'
        '    subprocess.run(["yt-dlp", "-o", "x.mp4", "url"])\n'
        '    r.request("POST", "https://x", cookiefile="c")\n'
    )
    fd, p = tempfile.mkstemp(suffix=".py")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(poisoned)
        ok, violations = scan_coverage_battery(p)
        assert not ok, "주입된 yt-dlp -o / POST / cookiefile 를 scan 이 못 잡음"
        joined = " ".join(violations).lower()
        # 두 검출 경로(금지 토큰 + yt-dlp AST 다운로드 플래그)가 모두 작동함을 실측 확인.
        assert "yt-dlp" in joined, "yt-dlp -o(AST) 미검출"
        assert "post" in joined or "cookiefile" in joined, \
            "HTTP 변경동사/인증세션 토큰 미검출: %s" % violations
    finally:
        os.unlink(p)


# ── 표면 커버리지·불변식 self-test ───────────────────────────────────────────
def _cys_actions_surface():
    """실측 `cys actions` verb 표면 frozenset. cys 부재 시 None(검사 skip)."""
    import shutil
    cys = shutil.which("cys")
    if not cys:
        return None
    try:
        r = subprocess.run([cys, "actions"], capture_output=True, timeout=10)
        if r.returncode != 0:
            return None
    except Exception:
        return None
    verbs = set()
    for ln in r.stdout.decode("utf-8", "replace").splitlines():
        ln = ln.rstrip()
        if not ln or ln[0].isspace():
            continue
        verb = ln.split()[0]
        if verb and verb not in ("help",):     # help 는 메타 — 분류 제외
            verbs.add(verb)
    return frozenset(verbs)


def _invariants():
    """순수 불변식 — OBSERVE∩MUTATE=∅·default-deny·면제 상한·표면 커버리지."""
    assert OBSERVE_VERBS & MUTATE_VERBS == frozenset(), \
        "OBSERVE ∩ MUTATE ≠ ∅: %s" % (OBSERVE_VERBS & MUTATE_VERBS)
    # default-deny: 미등록 verb 는 관찰 경로에서 거부(fail-closed).
    assert not is_observe_only(["cys", "totally-unknown-verb"]), \
        "미등록 verb 가 관찰 경로 통과(default-deny 위반)"
    assert is_observe_only(["cys", "status", "--json"]), "status 가 observe 가 아님"
    assert not is_observe_only(["cys", "launch-agent"]), "launch-agent 가 거부 안 됨"
    assert _extract_verb(["yt-dlp", "--dump-json"]) is None, "cys 아닌 호출이 verb 로 추출됨"
    # ★(0.14.31 · WP-5 · B-6) 서브동사 세분 — queue list 만 observe, revive/drop/clear/deliver 는 mutate.
    assert is_observe_only(["cys", "queue", "list", "--surface", "surface:3"]), "queue list 가 observe 가 아님"
    assert is_observe_only(["/usr/local/bin/cys", "--socket", "/x", "queue", "list", "--json"]), \
        "전역 옵션 뒤 queue list 가 observe 가 아님"
    for sub in ("revive", "drop", "clear", "deliver"):
        assert classify_cys_verb(["cys", "queue", sub, "q1.2"]) == "mutate", \
            "queue %s 가 mutate 가 아님" % sub
        assert not is_observe_only(["cys", "queue", sub, "q1.2"]), "queue %s 가 관찰 경로 통과" % sub
    assert classify_cys_verb(["cys", "queue"]) == "unknown", "서브동사 없는 queue 가 분류됨"
    assert classify_cys_verb(["cys", "queue", "purge"]) == "unknown", "미지 서브동사가 분류됨(default-deny)"
    assert MUTATE_SUBVERBS & OBSERVE_SUBVERBS == frozenset(), "서브동사 분류 충돌"
    # Windows Git Bash 형상 — `cys.exe`·`cys.cmd`·백슬래시 경로도 cys 호출이다.
    # ★(리뷰 R5) 경로는 **개인 프로필이 아닌** 더미로 쓴다 — `C:\Users\<이름>` 형상은 H-SECRET-1
    #   (산 트리 개인정보 스캔 · 발행 fail-closed)이 잡는다. 재는 것은 백슬래시·`.exe` 처리이지
    #   프로필 경로가 아니다(WP-5 가 남긴 적색 1건 · 이 레인 소유 파일이라 여기서 닫는다).
    assert is_observe_only(["D:\\opt\\cys\\bin\\cys.exe", "queue", "list"]), "cys.exe queue list 가 observe 가 아님"
    assert classify_cys_verb(["cys.cmd", "queue", "drop", "q1.2"]) == "mutate", "cys.cmd queue drop 가 mutate 가 아님"
    assert classify_cys_verb(["cys", "reclaim-role", "--auto"]) == "mutate", "reclaim-role 이 mutate 가 아님"
    # 면제 상한(계약 형해화 차단).
    assert len(SELF_HEAL_EXEMPT) <= 3, "SELF_HEAL_EXEMPT 무한 확장(계약 형해화)"
    # 표면 커버리지(과장 차단): cys actions ⊆ OBSERVE ∪ MUTATE ∪ UNCLASSIFIED_ALLOWED.
    surface = _cys_actions_surface()
    if surface is None:
        return "SKIP(cys 부재 — 표면 커버리지 검증 생략)"
    covered = OBSERVE_VERBS | MUTATE_VERBS | UNCLASSIFIED_ALLOWED
    uncovered = surface - covered
    # 드리프트: 미분류 verb 는 fail-closed 로 안전(관찰 경로 거부) — WARN 수준이라 RED 아님.
    # 단 self-test 는 '현재 미커버'를 정직하게 보고(PHIL-09)하고 통과시킨다.
    if uncovered:
        return "COVERED with drift(미분류 %d개 fail-closed 안전): %s" % (
            len(uncovered), ", ".join(sorted(uncovered)))
    return "FULL(cys 표면 전부 분류됨)"


def self_test():
    _invariants_msg = _invariants()
    _spy_cmd_check()
    _spy_negative_assertion_self_attack()
    _spy_preflight_daemon_observe_phase()
    _ast_self_attack()
    _ast_msg = _ast_coverage_battery()
    print("javis_idempotency self-test OK "
          "(불변식·표면커버리지[%s] · cmd_check 관찰멱등 negative assertion · "
          "자기공격 변이검증 RED · C12.daemon fix=False Popen 0 · "
          "coverage_battery AST 관찰전용[%s])" % (_invariants_msg, _ast_msg))
    return 0


def main(argv):
    if "--self-test" in argv:
        return self_test()
    if argv and argv[0] == "classify":
        if len(argv) < 2:
            print("usage: javis_idempotency.py classify '<argv...>'", file=sys.stderr)
            return 2
        toks = argv[1].split()
        print("%s → %s" % (toks, classify_cys_verb(toks)))
        return 0
    print(__doc__)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
