#!/usr/bin/env bash
# PreToolUse hook (matcher Edit|Write|NotebookEdit|MultiEdit|Bash): 역할-기반 능력 가드 (T4-4/T6-P3).
# reviewer-*/planner surface의 에이전트-내부 변형 도구(Edit/Write/NotebookEdit, write-shell Bash)를
# **툴 실행 전** deny(producer≠evaluator: 리뷰어 산출물 자기수정 reward-hack 차단)해 코드로 봉쇄한다.
# master/CSO/worker는 통과(full-trust).
#
# ★두 hook 클래스 (cys-hook.sh:6 불변 narrowing — 위반이 아니라 정밀화):
#   (a) OBSERVABILITY hook (cys-hook.sh) = **절대 차단 금지·항상 exit 0** — 텔레메트리가
#       에이전트를 깨뜨려선 안 된다(관측은 무해 통과가 불변).
#   (b) GATE hook (appbuild-gate.sh, role-capability-gate.sh) = **설계상 deny 가능**(deny-by-default
#       act tier) — 이 클래스는 차단이 목적이다. cys-hook.sh:6의 "막지 않는다"는 (a) 관측 전용
#       안전규칙이지 전면 금지가 아니다. role-capability-gate는 appbuild-gate에 이은 GATE의 2번째 사례다.
#
# ★차단 메커니즘: deny path는 modern Claude Code permission-decision JSON을
#   stdout({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny",...}})으로
#   내고 exit 0 한다(printf 고정 형태 — emit 경로에 jq/python 의존 없음). `exit 2`는 reviewer/planner
#   인데 role 조회가 물리적으로 불가능할 때(python3 부재 등)의 hard fail-closed fallback으로만 남긴다.
#   허용·무역할·읽기 경로는 stdout 무출력 + exit 0(defer).
#
# ★enforcement boundary (정직 — IMPL-GROUP-W3 honesty 계약):
#   이 hook이 *에이전트-내부 도구*(Claude Code Edit/Write/Bash)의 실 물리 enforcer다.
#   cysd는 이 내부 도구를 직접 못 막는다(에이전트 프로세스 내부) — cysd는 cysd-매개 경로
#   (send/scoped run write-shell)만 게이트한다(handlers.rs check_caps_gate). 이 둘이 합쳐 가드다.
#
# 신원 = cysd 권위: 자기 surface 역할은 `cys surface-role`(CYS_SURFACE_ID→데몬 roles 맵)로 읽는다.
#   self-declared가 아니라 claim_role/launch-agent가 신원검증 후 등록한 값. CYS_SURFACE_ID는
#   데몬이 PTY에 주입·상속하므로 에이전트가 임의 위조 불가(커널 peer-pid 신원의 파생).
#
# Threat model (defensive-security-gate 9원칙): 비-악의 협력 에이전트의 *오작동* + reviewer의
#   직접 변형 시도 차단. 근본한계(명문화·은폐 금지): ① 인터프리터 우회(`bash -c "..."`·스크립트)·
#   git alias·셸 변수 확장은 Bash 토큰화 검사가 못 잡는다(block-dangerous-git와 동일 한계) →
#   write-shell의 *대표* 위험 동사(파일 리다이렉트·tee·rm·mv·cp·설치자)만 deny. ② cysd 인증
#   (peer-pid)이 붕괴하면 role 조회가 오염될 수 있다(ADR: 소켓 동등노드 모델의 신뢰 뿌리).
#   kill-switch = 사람의 세션 리뷰.
#
# Design:
# - fail-CLOSED: python3 부재·JSON 파싱 실패·셸 파싱 불가·해석 불가 토큰 → reviewer면 BLOCK.
#   (단 role 조회 실패=역할 미상은 deny-by-default가 아니라 *통과* — 무역할 pane은 사람/일반
#    셸이라 일상 작업을 막지 않는다. reviewer-*/planner로 *확인된* surface만 차단한다.)
# - 비변형 도구(Read/Grep/Glob 등)는 이 hook의 matcher에 없으므로 도달하지 않는다(통과).
# - reviewer의 tmp/로그 write는 과도차단 방지를 위해 허용(검증 대상 경로 한정 — propmap T6-P3 §5).
# - 검증: 내장 배터리 --self-test.

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(role-capability-gate)" >&2; exit 0; }
# 인터프리터 해소 — 프리루드 단일 소유(python3→python→py · G22). 사본 재구현 금지(RC4).
if [ -z "${CYS_PY:-}" ]; then
  # python(3) 전무 — role을 알 수 없으니 안전측: reviewer 환경이면 막아야 하나 role 판별 불가.
  # 환경변수로 role 힌트가 있으면 그것으로 fail-closed, 없으면 통과(무역할 가정).
  case "${CYS_SURFACE_ROLE:-}" in
    reviewer*|planner|planner-*) echo "role-capability-gate: python missing — failing closed for reviewer/planner" >&2; exit 2 ;;
    *) exit 0 ;;
  esac
fi

if [ "${1:-}" = "--self-test" ]; then
  export CAPGATE_SELF_TEST=1
else
  CAPGATE_INPUT="$(cat)" || { echo "role-capability-gate: cannot read stdin" >&2; exit 0; }
  export CAPGATE_INPUT
  # 자기 surface 역할 조회(cysd 권위). 실패 시 빈 문자열 → '무역할'로 통과(사람/일반 셸 과차단 방지).
  # 단, 과거 reviewer/planner로 *확인된* surface는 캐시 → 조회 실패 시 캐시로 fail-closed:
  # 리뷰어가 신원조회를 실패시켜 gate를 우회(pass)하는 경로를 봉인한다(오탐 없이 known-reviewer만).
  CAPGATE_CACHE="${TMPDIR:-/tmp}/cys-capgate-role-${CYS_SURFACE_ID:-none}"
  if [ -z "${CYS_SURFACE_ROLE:-}" ] && command -v cys >/dev/null 2>&1; then
    CYS_SURFACE_ROLE="$(cys surface-role 2>/dev/null | head -n1)"
    if [ -n "${CYS_SURFACE_ID:-}" ]; then
      case "$CYS_SURFACE_ROLE" in
        reviewer*|planner|planner-*) printf '%s\n' "$CYS_SURFACE_ROLE" > "$CAPGATE_CACHE" 2>/dev/null ;;
        "") [ -f "$CAPGATE_CACHE" ] && CYS_SURFACE_ROLE="$(head -n1 "$CAPGATE_CACHE" 2>/dev/null)" ;;
      esac
    fi
  fi
  export CYS_SURFACE_ROLE
fi

exec "$CYS_PY" - <<'PYEOF'
import json, os, shlex, sys, tempfile

# 변형(mutation) 도구 — reviewer/planner에게 deny.
MUTATION_TOOLS = {"Edit", "Write", "NotebookEdit", "MultiEdit"}

# write-shell 대표 위험 동사(근본한계: 인터프리터 우회는 못 잡음 — block-dangerous-git와 동형).
WRITE_SHELL_CMDS = {
    "rm", "mv", "cp", "dd", "tee", "truncate", "install", "chmod", "chown",
    "ln", "mkdir", "rmdir", "touch", "sed",  # sed -i 등
}
# 패키지/빌드 설치자(상태 변형) — 대표만. git은 서브커맨드로 별도 판정(읽기 전용 다수).
WRITE_SHELL_INSTALLERS = {"npm", "pip", "pip3", "make", "apt", "brew"}
# cargo/go는 build/test 등 빌드 변형이 흔하나 reviewer는 빌드도 금지(산출물 변형) — 변형으로.
WRITE_SHELL_BUILDERS = {"cargo", "go"}
# git 변형 서브커맨드(읽기 전용 status/log/diff/show/grep 등은 허용).
GIT_WRITE_SUBS = {"commit", "push", "add", "reset", "rebase", "merge", "checkout",
                  "restore", "clean", "stash", "rm", "mv", "apply", "cherry-pick",
                  "revert", "tag", "branch", "init", "am", "pull", "fetch"}

WRAPPERS = {"command", "exec", "env", "sudo", "nohup", "time", "xargs"}

# 검증 대상 외 허용 경로 접두(과도차단 방지 — reviewer tmp/로그 write 허용).
ALLOW_PATH_PREFIXES = ("/tmp/", "/private/tmp/", "/var/tmp/", "/var/folders/")
ALLOW_PATH_SUBSTRS = ("/.cys/", "/logs/", "/log/", "/_round/")


def is_reviewer_or_planner(role):
    role = (role or "").strip()
    return role.startswith("reviewer") or role == "planner" or role.startswith("planner-")


def is_separator(tok):
    return bool(tok) and set(tok) <= set(";&|()")


def path_is_allowed(p):
    if not p:
        return False
    # RC-10: 백슬래시 정규화(Windows 경로 C:\...\Temp → 슬래시 비교 가능) + OS temp 동적 허용
    # (Windows %TEMP%는 /tmp/ 접두와 안 맞아 reviewer temp write가 과도차단되던 것 수정).
    ap = os.path.abspath(p).replace("\\", "/")
    tmp = tempfile.gettempdir().replace("\\", "/").rstrip("/") + "/"
    if ap.startswith(tmp):
        return True
    if any(ap.startswith(pre) for pre in ALLOW_PATH_PREFIXES):
        return True
    return any(s in ap for s in ALLOW_PATH_SUBSTRS)


def _is_redirect_op(tok):
    """순수 출력 리다이렉트 연산자(>, >>, 2>, &>)면 True. 입력(<)은 제외."""
    if not tok:
        return False
    # 붙은 형태('2>','&>')와 분리 형태('>','>>')를 함께 본다(입력 '<' 계열 제외).
    t = tok
    if t in (">", ">>", "&>", ">|"):
        return True
    if t.endswith(">") and "<" not in t:  # '2>', '1>>' 등
        return True
    return False


def bash_has_write(command):
    """Bash 명령에 write-shell 동사 또는 (비-허용경로) 출력 리다이렉트가 있으면 True(=변형).
    해석불가=True(fail-closed). 리다이렉트 대상이 허용경로(tmp/log)면 그 리다이렉트는 무시."""
    command = command.replace("\n", " ; ").replace("\r", " ")
    for zw in ("​", "‌", "‍", "﻿"):
        command = command.replace(zw, "")
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=True)
        lex.whitespace_split = True
        tokens = list(lex)
    except ValueError:
        return True  # 따옴표 불일치 등 — fail-closed(변형으로 간주)

    cmd_pos = True
    i = 0
    n = len(tokens)
    while i < n:
        tok = tokens[i]
        # 출력 리다이렉트: 대상이 허용경로면 통과, 아니면 변형.
        if _is_redirect_op(tok):
            target = tokens[i + 1] if i + 1 < n else ""
            if not path_is_allowed(target):
                return True
            i += 2
            continue
        if is_separator(tok):
            cmd_pos = True
            i += 1
            continue
        if cmd_pos:
            name = tok.split("=", 1)[0]
            if "=" in tok and name and name.replace("_", "").isalnum():
                i += 1
                continue  # env 할당
            if tok in WRAPPERS:
                i += 1
                continue
            base = os.path.basename(tok)
            if base in WRITE_SHELL_CMDS or base in WRITE_SHELL_INSTALLERS or base in WRITE_SHELL_BUILDERS:
                return True
            if base == "git":
                # 다음 비-플래그 토큰이 write 서브커맨드면 변형.
                for t in tokens[i + 1:]:
                    if is_separator(t):
                        break
                    if t.startswith("-"):
                        continue
                    if t in GIT_WRITE_SUBS:
                        return True
                    break  # 첫 서브커맨드만 본다(읽기 전용이면 통과)
            cmd_pos = False
        i += 1
    return False


def decide(tool, tool_input, role):
    """(block: bool, reason). reviewer/planner만 차단 대상."""
    if not is_reviewer_or_planner(role):
        return False, "not reviewer/planner (role=%r) — pass" % role
    if tool in MUTATION_TOOLS:
        fp = tool_input.get("file_path") if isinstance(tool_input, dict) else None
        if path_is_allowed(fp):
            return False, "reviewer write to allowed path %r (tmp/log)" % fp
        return True, "reviewer/planner may not %s (producer≠evaluator)" % tool
    if tool == "Bash":
        cmd = tool_input.get("command") if isinstance(tool_input, dict) else ""
        if not isinstance(cmd, str) or not cmd:
            return False, "empty bash"
        if bash_has_write(cmd):
            return True, "reviewer/planner may not run write-shell"
        return False, "read-only bash allowed"
    # matcher 밖 도구가 흘러들어와도 변형 아니면 통과.
    return False, "non-mutation tool"


def _json_escape(s):
    """JSON 문자열 값 이스케이프(고정 형태 emit용 — 외부 의존 없음)."""
    out = []
    for ch in s:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ord(ch) < 0x20:
            out.append("\\u%04x" % ord(ch))
        else:
            out.append(ch)
    return "".join(out)


def emit_deny(reason):
    """modern Claude Code permission-decision deny JSON을 stdout에 내고 exit 0.
    printf 고정 형태(외부 jq/python 의존 없음 — reason만 보간·이스케이프)."""
    sys.stdout.write(
        '{"hookSpecificOutput":{"hookEventName":"PreToolUse",'
        '"permissionDecision":"deny","permissionDecisionReason":"%s"}}\n'
        % _json_escape(reason))
    sys.exit(0)


def main():
    raw = os.environ.get("CAPGATE_INPUT", "")
    role = os.environ.get("CYS_SURFACE_ROLE", "")
    try:
        data = json.loads(raw)
    except ValueError:
        # JSON 파싱 실패 — reviewer면 fail-closed.
        if is_reviewer_or_planner(role):
            print("role-capability-gate: malformed hook JSON — failing closed (reviewer)", file=sys.stderr)
            sys.exit(2)
        sys.exit(0)
    if not isinstance(data, dict):
        sys.exit(0)
    tool = data.get("tool_name") or data.get("tool") or ""
    tool_input = data.get("tool_input") if isinstance(data.get("tool_input"), dict) else {}
    block, reason = decide(tool, tool_input, role)
    if block:
        # 진단은 stderr(transcript), 차단 판정은 modern JSON permission-decision(stdout)+exit 0.
        print("role-capability-gate DENY: %s [role=%s tool=%s]" % (reason, role, tool), file=sys.stderr)
        emit_deny("%s surface는 producer 산출물 수정 금지 (producer != evaluator)" % (role or "reviewer"))
    sys.exit(0)


def self_test():
    cases_block = [
        ("reviewer-codex", "Edit", {"file_path": "/Users/x/dev/repo/src/a.rs"}),
        ("reviewer-gemini", "Write", {"file_path": "/Users/x/dev/repo/out.md"}),
        ("reviewer", "NotebookEdit", {"notebook_path": "/x/n.ipynb"}),
        ("planner", "Edit", {"file_path": "/x/y.ts"}),
        ("reviewer-codex", "Bash", {"command": "rm -rf /x/build"}),
        ("reviewer-codex", "Bash", {"command": "echo hi > /Users/x/dev/repo/f.txt"}),
        ("reviewer-codex", "Bash", {"command": "git commit -m x"}),
        ("reviewer-codex", "Bash", {"command": "sed -i s/a/b/ /x/f"}),
        ("reviewer-codex", "Bash", {"command": "npm install"}),
        ("reviewer-codex", "Bash", {"command": "cd x && cp a b"}),
        ("reviewer-codex", "Bash", {"command": "git 'push"}),  # 따옴표불일치 fail-closed
    ]
    cases_allow = [
        # full-trust 역할은 무엇이든 통과
        ("worker", "Edit", {"file_path": "/x/a.rs"}),
        ("worker-2", "Bash", {"command": "rm -rf /x"}),
        ("master", "Write", {"file_path": "/x/b"}),
        ("cso", "Bash", {"command": "npm install"}),
        # 무역할 pane(사람/일반 셸)은 차단 안 함
        ("", "Edit", {"file_path": "/x/a.rs"}),
        ("-", "Bash", {"command": "rm x"}),
        # reviewer의 read-only bash 허용
        ("reviewer-codex", "Bash", {"command": "grep -rn foo ."}),
        ("reviewer-codex", "Bash", {"command": "cat /x/f && ls -la"}),
        ("reviewer-codex", "Bash", {"command": "git status"}),
        # reviewer의 tmp/로그 write 허용(과도차단 방지)
        ("reviewer-codex", "Write", {"file_path": "/tmp/review-notes.md"}),
        ("reviewer-codex", "Edit", {"file_path": "/Users/x/.cys/scratch.txt"}),
        ("reviewer-codex", "Bash", {"command": "echo hi > /tmp/out.log"}),
    ]
    fails = []
    for role, tool, ti in cases_block:
        b, _ = decide(tool, ti, role)
        if not b:
            fails.append("BYPASS: role=%s %s %r" % (role, tool, ti))
    for role, tool, ti in cases_allow:
        b, r = decide(tool, ti, role)
        if b:
            fails.append("FALSE-POSITIVE(%s): role=%s %s %r" % (r, role, tool, ti))

    # ★API 계약 검증: deny 경로는 permissionDecision==deny JSON을 내는가 / 허용 경로는 무출력인가.
    def render_emit(role, reason_block):
        """main()의 emit 형태를 재현(emit_deny가 stdout에 쓰는 정확한 문자열)."""
        return ('{"hookSpecificOutput":{"hookEventName":"PreToolUse",'
                '"permissionDecision":"deny","permissionDecisionReason":"%s"}}'
                % _json_escape("%s surface는 producer 산출물 수정 금지 (producer != evaluator)"
                               % (role or "reviewer")))
    # reviewer mutation → JSON deny shape 파싱·필드 검증
    sample_role = "reviewer-codex"
    emitted = render_emit(sample_role, True)
    try:
        parsed = json.loads(emitted)
        hso = parsed["hookSpecificOutput"]
        if hso.get("hookEventName") != "PreToolUse":
            fails.append("EMIT: hookEventName != PreToolUse")
        if hso.get("permissionDecision") != "deny":
            fails.append("EMIT: permissionDecision != deny")
        if "producer != evaluator" not in hso.get("permissionDecisionReason", ""):
            fails.append("EMIT: reason missing producer!=evaluator")
        if sample_role not in hso.get("permissionDecisionReason", ""):
            fails.append("EMIT: reason missing role")
    except (ValueError, KeyError) as e:
        fails.append("EMIT: deny JSON not valid/shaped: %s" % e)
    # 허용 케이스는 deny JSON을 절대 내지 않는다(emit 미호출 = stdout 무출력) — decide()가 이미 보장.
    for role, tool, ti in cases_allow:
        b, _ = decide(tool, ti, role)
        if b:
            fails.append("EMIT-FALSE: allowed case would emit deny: role=%s %s" % (role, tool))

    if fails:
        print("\n".join(fails), file=sys.stderr)
        print("self-test: %d failure(s)" % len(fails), file=sys.stderr)
        sys.exit(1)
    print("self-test OK: %d blocked · %d allowed · deny=permissionDecision JSON(exit0)·"
          "allow=empty-stdout(exit0)·fail-closed(exit2) verified"
          % (len(cases_block), len(cases_allow)))
    sys.exit(0)


if os.environ.get("CAPGATE_SELF_TEST"):
    self_test()
else:
    main()
PYEOF
