#!/bin/bash
# guard.sh — Autopilot 집행 hook (Claude Code PreToolUse 진입점) · R3 deny-by-default allowlist
# SOT: _round/autopilot/SPEC.md · master 거버넌스(soul AUTONOMOUS PILOT ANCHOR 이행조건)
#
# ★박사님 (B') 결정: effect-denylist 는 shell Turing-complete 라 우회불가피(codex 입증)
#   → deny-by-default allowlist parser 근본전환(Phase3 R3 "무엇만 남길까" 명령레벨 적용).
#
# ★모드 분리:
#   - AUTOPILOT_ACTIVE 존재 = 자율주행 → STRICT: shlex grammar 파서, ALLOWLIST 외 전부 deny
#   - 플래그 無 = 평시(박사님 직접작업) → LOOSE : 명백 비가역만 차단(효과기반 denylist)
#   - AUTOPILOT_PAUSED 존재 = kill-switch(상위) → 비읽기 deny(autopilot.sh 도달성 예외)
#
# ★R3 반영(codex 재게이트 5잔여):
#   잔여1 sed --in-place/-i.bak/long-opt STRICT 차단
#   잔여2 git 가역 서브커맨드 내 파괴옵션(branch -d/-D·stash drop/clear·commit --amend·add 헌법경로) deny
#   잔여3 STRICT Write/Edit 의 guard 인프라(_round/autopilot/·플래그·settings.json) 자기보호(trust boundary)
#   잔여4 flag env override 는 GUARD_TEST_MODE 테스트 전용 — 운영은 canonical path 고정+심링크 거부
#   잔여5 STRICT 중 python 부재/크래시 = deny-by-default(LOOSE degraded 금지)
#
# ★deny = exit 2 + stderr(무조건 차단 보장) + SPEC PreToolUse JSON(stdout). exit0+JSON 은 malformed시 fail-OPEN.
# ★fail-closed: 파싱불가·미해석·allowlist밖 → deny.
set -u

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
# ★이 훅은 GATE 클래스지만 프리루드 소실은 '가드 판정 불가'가 아니라 '가드 미설치'다 —
#   exit 0(강등)이 계약이다. 프리루드 실재는 preflight 핀 체크가 별도로 감시한다(이중 방어).
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(guard)" >&2; exit 0; }

SOURCE="${BASH_SOURCE[0]}"
while [ -h "$SOURCE" ]; do
  DIR="$(cd -P "$(dirname "$SOURCE")" >/dev/null 2>&1 && pwd)"
  SOURCE="$(readlink "$SOURCE")"; [[ $SOURCE != /* ]] && SOURCE="$DIR/$SOURCE"
done
SCRIPT_DIR="$(cd -P "$(dirname "$SOURCE")" >/dev/null 2>&1 && pwd)"
ROUND_DIR="$(cd -P "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd)"
PREFLIGHT="${GUARD_PREFLIGHT:-0}"

# 잔여4: 운영은 canonical path 고정(env override 무시), 테스트만 GUARD_TEST_MODE 로 override
if [ "${GUARD_TEST_MODE:-0}" = "1" ]; then
  PAUSED_FILE="${AUTOPILOT_PAUSED_FILE:-$ROUND_DIR/AUTOPILOT_PAUSED}"
  ACTIVE_FILE="${AUTOPILOT_ACTIVE_FILE:-$ROUND_DIR/AUTOPILOT_ACTIVE}"
else
  PAUSED_FILE="$ROUND_DIR/AUTOPILOT_PAUSED"
  ACTIVE_FILE="$ROUND_DIR/AUTOPILOT_ACTIVE"
fi

INPUT="$(cat)"

# G22: 인터프리터 후보에 python·py 추가 — Windows에는 `python3` 명령이 없고 python/py만 있는
# 경우가 흔하다. 후보 **순서 = 우선순위**이고 절대경로 후보(homebrew·/usr/bin)는 PATH 빈곤 환경
# (GUI 기동)의 belt-and-braces다. 프리루드가 해소한 CYS_PY를 최우선 후보로 둔다(단일 SOT).
PYBIN=""
for c in "${CYS_PY:-python3}" python3 python py \
         /opt/homebrew/bin/python3 /usr/bin/python3 /usr/local/bin/python3; do
  [ -n "$c" ] || continue
  if command -v "$c" >/dev/null 2>&1; then PYBIN="$(command -v "$c")"; break; fi
  [ -x "$c" ] && PYBIN="$c" && break
done
# 테스트 전용(GUARD_TEST_MODE): python 부재 시뮬레이션으로 잔여5(STRICT fail-closed) 검증
[ "${GUARD_TEST_MODE:-0}" = "1" ] && [ "${GUARD_FORCE_NOPY:-0}" = "1" ] && PYBIN=""

emit_deny() {
  local reason="$1"
  printf '%s\n' "AUTOPILOT GUARD DENY: $reason" >&2
  if command -v jq >/dev/null 2>&1; then
    jq -nc --arg r "$reason" '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
  else
    printf '%s\n' "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"guard deny\"}}"
  fi
  exit 2
}

# 잔여4: 운영 모드에서 flag 가 심링크면 trust boundary 위반 → fail-closed deny
if [ "${GUARD_TEST_MODE:-0}" != "1" ]; then
  for f in "$PAUSED_FILE" "$ACTIVE_FILE"; do
    [ -L "$f" ] && emit_deny "autopilot flag 심링크 거부(trust boundary): $(basename "$f")"
  done
fi

IFS= read -r -d '' PYSRC <<'PY'
import sys, json, re, unicodedata, os, shlex

PREFLIGHT = os.environ.get("GUARD_PREFLIGHT", "0") == "1"
PAUSED    = os.environ.get("GUARD_PAUSED", "0") == "1"
ACTIVE    = os.environ.get("GUARD_ACTIVE", "0") == "1"   # 자율주행 → STRICT
CONST_AUTH  = os.environ.get("GUARD_CONST_AUTH", "0") == "1"   # ★박사님 직접명령 헌법편집 인가 토큰 유효(bash 계산: 토큰존재+非ACTIVE+TTL30분)
CONST_SCOPE = os.environ.get("GUARD_CONST_SCOPE", "")          # 인가 스코프(헌법 basename 목록 또는 '*')

raw = sys.stdin.read()
try:
    data = json.loads(raw) if raw.strip() else {}
    if not isinstance(data, dict):
        raise ValueError("hook input not an object")
except Exception as e:
    print("DENY\tunparseable hook input (fail-closed): %s" % e)
    sys.exit(2)

tool = data.get("tool_name") or ""
ti = data.get("tool_input")
ti = ti if isinstance(ti, dict) else {}
command = ti.get("command") if isinstance(ti.get("command"), str) else ""
file_path = ti.get("file_path") if isinstance(ti.get("file_path"), str) else ""

WRITE_TOOLS = ("Write", "Edit", "NotebookEdit", "MultiEdit")

# ================= 공통 정규화 =================
def _strip_hidden(s):
    out = []
    for ch in s:
        if ch in ("\n", "\r", "\x85", "\u2028", "\u2029"):  # LF/CR/NEL/LS/PS = 명령 구분자 보존
            out.append(" ; ")   # ★공백패딩 — ';' 가 인접 토큰에 붙으면(soul.md;) 매칭 실패. 둘째 명령 흡수 차단
        elif ch.isspace():
            out.append(" ")
        elif unicodedata.category(ch).startswith("C"):       # 제어·포맷·zero-width 제거
            continue
        else:
            out.append(ch)
    return "".join(out)

def norm_base(s):
    s = unicodedata.normalize("NFKC", s)
    s = re.sub(r"\$\{?ifs[^}\s]*\}?", " ", s, flags=re.I)
    s = _strip_hidden(s)
    return s

# ★G18(T-0147-7 W1a): 경로 판정을 **정규화 경로** 기반으로 — 백슬래시 무음 우회 차단.
# 종전 `t.rsplit("/",1)[-1]` 은 Windows 경로 `C:\Users\me\.claude\soul.md` 에 '/' 가 없어
# **전체 문자열을 basename 으로 돌려주었다** → protected()·guard_infra() 가 전부 미스 →
# 헌법파일 쓰기·guard 자기보호가 Windows에서 무음 통과였다(실측 정적 판독 · 검체 H-WIN-1).
def norm_path(t):
    return (t or "").replace("\\", "/")

def basename(t):
    # .strip(): file_path 의 trailing 개행/공백(예: jq -Rs 산물·은닉)이 매칭 회피하는 것 차단(fail-closed)
    return norm_path(t).rsplit("/", 1)[-1].strip()

PROTECTED_NAMES = ("soul.md", "claude.md", "vibecoding_constitution.md",
                   "vibecoding_enforcement.md", "route_contract.md")

def protected(name):
    n = basename(name).lower()
    return n in PROTECTED_NAMES or n.endswith("_directive.md")

# 잔여3(R3): STRICT Write/Edit 가 건드리면 안 되는 guard 인프라(디렉터리 전체)
def guard_infra(fp):
    b = basename(fp).lower()
    if "_round/autopilot/" in norm_path(fp):   # G18: 백슬래시 경로도 같은 판정
        return True
    if b == "constitution_edit_authorized":   # ★인가 토큰 — STRICT 중 생성 차단(자율주행 자기인가 방지)
        return True
    if b in ("guard.sh", "autopilot.sh", "test_autopilot.sh"):
        return True
    if b in ("autopilot_active", "autopilot_paused"):
        return True
    if b == "settings.json":
        return True
    return False

# 잔여1(R4): 변조 시 guard 자체가 무력화되는 핵심 파일(redirect·write 양쪽 hard deny).
# guard_infra 와 달리 autopilot 디렉터리 임시로그(예: tmp.log)는 제외 → redirect 예외 유지.
def guard_critical(fp):
    b = basename(fp).lower()
    if b == "constitution_edit_authorized":   # ★인가 토큰 — redirect 변조 차단
        return True
    if b in ("guard.sh", "autopilot.sh", "test_autopilot.sh"):
        return True
    if b in ("autopilot_active", "autopilot_paused"):
        return True
    if b == "settings.json":
        return True
    return False

WRAPPERS = {"sudo", "doas", "env", "command", "exec", "nohup", "nice", "ionice",
            "time", "timeout", "stdbuf", "setsid", "caffeinate", "xargs", "builtin"}

# ================= STRICT (deny-by-default allowlist) =================
ALLOWLIST = {"ls", "cat", "head", "tail", "grep", "rg", "find", "git", "pytest",
             "echo", "wc", "stat", "jq", "shasum", "cmux", "sed", "python3"}
# 가역 서브명령(add·commit·diff·log·status·stash·show·branch·restore)=git reset/restore 로 되돌림 가능 → allow.
# ★보호파일(soul·CLAUDE·*_DIRECTIVE) basename 이 있어도 이들은 allow(staging·diff·commit 은 파일내용 변경 아님·가역) —
#   실제 보호는 Write/Edit·redirect 차단으로(격리). push·remote·tag생성=비가역 외부발행→제외(deny). (master 결정 2026-06-07)
GIT_OK = {"status", "log", "diff", "add", "show", "branch", "stash", "commit", "restore"}
SHELL_KEYWORDS = {"if", "then", "else", "elif", "fi", "for", "while", "until",
                  "do", "done", "case", "esac", "in", "{", "}", "!", "select", "function"}
SEPARATORS = {";", "|", "&", "&&", "||", "(", ")", "{", "}", "\n", "|&", ";;"}
REDIRECTS  = {">", ">>", ">|", "&>", "&>>", ">&"}

def redirect_safe(target):
    if not target:
        return False
    if guard_critical(target):   # 잔여1(R4): redirect 로 guard.sh·플래그·settings.json 변조 차단(Write 와 동일 trust boundary)
        return False
    if target == "/dev/null":
        return True
    t = norm_path(target)                       # G18: 백슬래시 경로도 같은 판정
    return ("_round/autopilot/" in t) and (".." not in t)

def extract_commands(toks):
    cmds = []
    i, n = 0, len(toks)
    expect = True
    while i < n:
        t = toks[i]
        if t in SEPARATORS:
            expect = True; i += 1; continue
        if expect:
            while i < n and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", toks[i]):
                i += 1
            if i < n and toks[i] in SHELL_KEYWORDS:
                i += 1; continue
            while i < n and toks[i].rsplit("/", 1)[-1].lower() in WRAPPERS:
                i += 1
                while i < n and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", toks[i]):
                    i += 1
            if i < n and toks[i] not in SEPARATORS:
                prog = toks[i].rsplit("/", 1)[-1]
                j = i + 1; args = []
                while j < n and toks[j] not in SEPARATORS:
                    args.append(toks[j]); j += 1
                cmds.append((prog, args))
                i = j; expect = False; continue
        i += 1
    return cmds

def git_sub_strict(args):
    i = 0
    val_opts = {"-C", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--super-prefix"}
    while i < len(args):
        a = args[i]
        if a == "-c" or a.startswith("-c"):
            return "__GITC__", []
        if a.startswith("-"):
            if "=" in a: i += 1; continue
            if a in val_opts: i += 2; continue
            i += 1; continue
        return a, args[i+1:]
    return None, []

def sed_has_write(args):
    # ★R6 근본전환(codex 권고·보수적 deny): sed 옵션을 정밀 파싱 — 안전옵션(read-only)만 허용하고
    # write/exec/inplace/외부스크립트/미지옵션·검사불가(붙임·클러스터)는 전부 fail-closed deny. read-only subset 보존.
    SAFE_CHARS = set("nErsuz")   # -n quiet, -E/-r extended, -s separate, -u unbuffered, -z null (전부 read-only·script 무운반)
    SAFE_LONG = {"--quiet", "--silent", "--regexp-extended", "--separate", "--null-data",
                 "--unbuffered", "--posix", "--sandbox", "--debug", "--help", "--version"}
    # ★R8 오탐수정(codex): -e/-f 없으면 첫 비옵션만 script·나머지 비옵션=file operand(검사 제외).
    # -e/--expression script 는 계속 검사 / -- 다음 첫 토큰만 script·나머지 operand.
    scripts = []
    have_expr = False     # -e/--expression 로 script 받음 → 이후 비옵션은 전부 file operand
    got_script = False     # 첫 비옵션 script 소비됨
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--":                                   # 옵션 종료 — 다음 첫 토큰만 script, 나머지 operand
            rest = args[i+1:]
            if rest and not (have_expr or got_script):
                scripts.append(rest[0]); got_script = True
            i = len(args); continue
        if a.startswith("--"):
            if a in SAFE_LONG:
                i += 1; continue
            if a == "--expression":
                if i + 1 < len(args): scripts.append(args[i+1]); have_expr = True; i += 2; continue
                return True
            if a.startswith("--expression="):
                scripts.append(a.split("=", 1)[1]); have_expr = True; i += 1; continue
            return True                                  # --in-place·--file·미지 long opt → fail-closed
        if a.startswith("-") and len(a) > 1:             # 단문자 옵션 클러스터
            j = 1
            consumed_next = False
            while j < len(a):
                ch = a[j]
                if ch in SAFE_CHARS:
                    j += 1; continue
                if ch in ("i", "f"):                     # inplace / file(외부스크립트) → deny
                    return True
                if ch == "e":
                    rest = a[j+1:]
                    if rest:
                        scripts.append(rest)             # -[safe]eSCRIPT 붙임형(★codex R6 잔여)
                    elif i + 1 < len(args):
                        scripts.append(args[i+1]); consumed_next = True   # -[safe]e SCRIPT 다음인자
                    have_expr = True                     # -e → 이후 비옵션은 file operand
                    j = len(a); break
                return True                              # 미지 단문자(-l 등) → fail-closed
            i += 2 if consumed_next else 1
            continue
        # 비옵션: -e/-f 또는 첫 script 이미 있으면 file operand(검사 제외), 아니면 첫 비옵션=script (R8)
        if have_expr or got_script:
            i += 1; continue
        scripts.append(a); got_script = True; i += 1
    # ★R7 과탐 보수전환(codex 권고·master 결단): sed grammar 정밀파싱은 끝없는 우회표면 →
    # '명백 read-only 만 allow'. s///·y/// 내용 + 정규식 주소(/re/·\cREc 대체구분자)를 마스킹한 뒤,
    # 남은 구조(명령·flag·주소연산)에 write/read-file/execute 지표 [wWrRe] 가 하나라도 있으면 fail-closed deny.
    # 주소문법 변종(\c..c·1,+N·first~step·구분자변형)·라벨·a/i/c 텍스트의 w/r/e 는 과탐 deny 로 흡수.
    # s/// 치환 '내용'(s/w/x/)은 마스킹되어 제외 → flag·command-position 과만 구분 판정.
    for sc in scripts:
        m = re.sub(r"([sy])(\W)(?:\\.|(?!\2).)*?\2(?:\\.|(?!\2).)*?\2", r"\1\2\2\2", sc)  # s///·y/// 내용 제거(flag 보존)
        m = re.sub(r"/(?:\\.|[^/])*/", "//", m)                                            # /regex/ 주소 내용 제거
        m = re.sub(r"\\(.)(?:\\.|(?!\1).)*?\1", r"\\\1\1", m)                              # \cREc 대체구분자 주소 제거
        if re.search(r"[wWrRe]", m):                                                       # 잔여 write/read/exec 지표
            return True
    return False

def prog_allowed(prog, args):
    if prog not in ALLOWLIST:
        return False, "allowlist 외 명령 '%s' (자율주행 deny-by-default)" % prog
    if prog == "git":
        sub, subargs = git_sub_strict(args)
        if sub == "__GITC__":
            return False, "git -c (alias 임의실행 우회) 금지"
        if sub is None:
            return False, "git 서브커맨드 불명(fail-closed)"
        if sub not in GIT_OK:
            return False, "git '%s' 비허용(허용: %s)" % (sub, "/".join(sorted(GIT_OK)))
        # 잔여2(R2)+잔여4(R4): 가역 서브커맨드 내 파괴 옵션 차단(subcommand별 option allowlist 축소)
        if sub == "branch" and any(a in ("-d", "-D", "--delete", "-f", "--force", "-m", "-M", "--move") for a in subargs):
            return False, "git branch 삭제/강제/이동(-d/-D/-f/-m) 금지"
        if sub == "stash":
            s2 = next((a for a in subargs if not a.startswith("-")), None)
            if s2 in ("drop", "clear", "pop", "apply", "branch"):
                return False, "git stash %s (손실/적용/분기) 금지(허용: stash·list·show)" % s2
        if sub == "commit" and any(a == "--amend" or a.startswith("--amend") for a in subargs):
            return False, "git commit --amend (히스토리 변경) 금지"
        # ★git add 헌법파일 stage 는 가역(git reset 언스테이지)이라 allow — 차단하면 master 정당커밋 막힘.
        #   보호는 Write/Edit·redirect 차단으로 격리(staging 은 파일내용 변경 아님). (master 결정 2026-06-07 오탐수정)
        return True, ""
    if prog == "find":
        if any(a in ("-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprintf", "-fprint", "-fls") for a in args):
            return False, "find -delete/-exec 류 금지"
        return True, ""
    if prog == "sed":
        # ★R6 근본전환: sed_has_write 가 -i/--in-place·w/W/r/R/e·s///w(숫자포함)·-f·-e붙임·미지옵션 전부 보수 deny.
        if sed_has_write(args):
            return False, "sed write/exec/inplace/외부스크립트/미지옵션 금지 — read-only subset만 허용(fail-closed)"
        return True, ""
    if prog == "python3":
        # python3 -c/-m/- 임의실행은 차단. 단 python3 <file>·pytest 는 ★근본한계(잔여2):
        # allowlisted interpreter 통한 임의코드 실행은 Turing-complete 라 정적 차단 불가.
        # → 충성노드(master·워커=신뢰) 전제 + AUTOPILOT_PAUSED kill-switch 로 커버.
        #   위협모델 = 악의가 아니라 '자율주행 중 실수'(잊고 위험명령) 방지. soul ANCHOR 에도 master 명문화.
        if any(a in ("-c", "-m", "-") for a in args):
            return False, "python3 -c/-m/- 임의실행 금지"
        return True, ""
    # pytest 도 동일 근본한계(conftest.py 임의코드) — 충성노드 전제+kill-switch 커버.
    return True, ""

def strict_deny(command):
    n = norm_base(command)
    if re.search(r"\$\(|\x60|<\(|>\(", n):   # \x60=backtick: command/process substitution
        return "command/process substitution($()·backtick·<()) 금지"
    try:
        lex = shlex.shlex(n, posix=True, punctuation_chars=True)
        lex.whitespace_split = True
        toks = list(lex)
    except ValueError as e:
        return "shlex 파싱불가(fail-closed): %s" % e
    for idx, t in enumerate(toks):
        if t in REDIRECTS:
            tgt = toks[idx+1] if idx+1 < len(toks) else ""
            if not redirect_safe(tgt):
                return "출력 redirect 금지(임시경로/dev/null 외): %s %s" % (t, tgt)
    for prog, args in extract_commands(toks):
        ok, reason = prog_allowed(prog, args)
        if not ok:
            return reason
    return None

# ================= LOOSE (평시 효과기반 denylist) =================
def norm_loose(s):
    s = norm_base(s)
    s = s.replace('"', "").replace("'", "").replace("\\", "")
    s = re.sub(r"([<>])", r" \1 ", s)
    return " ".join(s.split())

def norm_loose_slash(s):
    """G18 보조 변형 — 백슬래시를 '삭제'가 아니라 '/'로 바꾼다. 보호파일 basename 스캔에만 쓴다.
    norm_loose 의 백슬래시 **삭제**는 `s\\oul.md` 류 이스케이프 회피를 무력화하는 장치인데, 같은
    삭제가 Windows 경로 `C:\\Users\\me\\soul.md` 의 basename 경계도 지워 보호 판정을 미스한다.
    두 변형을 **모두** 스캔하면 양쪽이 다 막힌다 — 추가 deny만 발생하고 기존 통과 경로는 불변."""
    s = norm_base(s)
    s = s.replace('"', "").replace("'", "").replace("\\", "/")
    s = re.sub(r"([<>])", r" \1 ", s)
    return " ".join(s.split())

def l_words(seg): return seg.split()
def l_strip_env(ws):
    i = 0
    while i < len(ws) and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", ws[i]):
        i += 1
    return ws[i:]
def l_cmd_word(seg):
    ws = l_strip_env(l_words(seg))
    while ws and ws[0].rsplit("/", 1)[-1].lower() in WRAPPERS:
        ws = l_strip_env(ws[1:])
    if not ws: return None, []
    return ws[0].rsplit("/", 1)[-1], ws[1:]
def l_segments(n): return [p.strip() for p in re.split(r"[;&|\n]+", n) if p.strip()]
def l_git_sub(args):
    i = 0; val_opts = {"-c", "-C", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--super-prefix"}
    while i < len(args):
        a = args[i]
        if a.startswith("-"):
            if "=" in a: i += 1; continue
            if a in val_opts: i += 2; continue
            i += 1; continue
        return a, args[i+1:]
    return None, []
WHITELIST_DIR = "_round/autopilot/"
def l_fileop_allowed(args):
    # G18: 경로 판정 전 정규화 — 백슬래시 경로가 '경로가 아닌 것'으로 오분류되면
    # 화이트리스트 밖 파괴 연산이 통과한다(fail-open 방향 오류).
    nonflag = [norm_path(a) for a in args
               if not a.startswith("-") and a not in ("+x", "-x") and not re.match(r"^[0-7]{3,4}$", a)]
    paths = [a for a in nonflag if "/" in a]
    if not paths and not nonflag:
        return False
    targets = paths if paths else nonflag
    for t in targets:
        if ".." in t or (WHITELIST_DIR not in t):
            return False
    return True

def loose_deny(n, n_slash=""):
    for seg in l_segments(n):
        prog, args = l_cmd_word(seg)
        if prog is None: continue
        if prog == "git":
            sub, subargs = l_git_sub(args)
            if sub == "push": return "git push (외부발행=비가역). 박사님 승인 필요"
            if sub == "remote" and any(x in subargs for x in ("set-url", "add", "remove", "rm", "rename")):
                return "git remote set-url/add/remove/rename (발행대상 변경). 박사님 승인 필요"
            if sub == "tag":
                create = any(x in subargs for x in ("-a", "-s", "-d", "-f", "-m")) or any(not x.startswith("-") for x in subargs)
                if create and not any(x in subargs for x in ("-l", "--list", "-n")):
                    return "git tag 생성/삭제. 박사님 승인 필요"
        if prog == "gh":
            if "release" in args and any(x in args for x in ("create", "upload", "delete", "edit")):
                return "gh release 발행. 박사님 승인 필요"
            if "pr" in args and any(x in args for x in ("create", "merge")):
                return "gh pr create/merge. 박사님 승인 필요"
        if prog in ("rm", "rmdir", "mv", "truncate", "chmod", "chown"):
            if not l_fileop_allowed(args):
                return "%s (비가역 파일연산). _round/autopilot/ 외 → 박사님 승인 필요" % prog
        if prog == "scp":
            return "scp (외부전송). 박사님 승인 필요"
        if prog == "rsync":
            if any(("@" in a) or re.match(r"^[^/\-][^/]*:", a) or "::" in a for a in args):
                return "rsync 원격전송. 박사님 승인 필요"
        if prog in ("curl", "wget"):
            up = False
            if prog == "curl":
                up = any(a in ("-T", "--upload-file", "-d", "--data", "-F", "--form", "--upload") or a.startswith("--data") for a in args)
                for i, a in enumerate(args):
                    if a in ("-X", "--request") and i+1 < len(args) and args[i+1].upper() in ("POST", "PUT", "DELETE", "PATCH"):
                        up = True
                    if a.startswith("--request=") and a.split("=", 1)[1].upper() in ("POST", "PUT", "DELETE", "PATCH"):
                        up = True
            else:
                up = any(a.startswith("--post-data") or a.startswith("--post-file") or a.startswith("--method=post") for a in (x.lower() for x in args))
            if up:
                return "%s 업로드/POST (외부전송). 박사님 승인 필요" % prog
    # G18: 두 정규화 변형(백슬래시 삭제형 + 슬래시 변환형)의 토큰을 합쳐 보호파일을 스캔한다.
    alltoks = n.split() + (n_slash.split() if n_slash else [])
    if any(protected(t) for t in alltoks):
        writers = {"tee", "cp", "dd", "mv", "install", "ln", "truncate", "patch", "vim", "vi", "nano", "emacs", "ex"}
        sed_inplace = False; seg_progs = set()
        for s in l_segments(n):
            p, a = l_cmd_word(s); seg_progs.add(p)
            if p == "sed" and any(x == "-i" or x.startswith("-i") for x in a): sed_inplace = True
        if (">" in alltoks) or (seg_progs & writers) or sed_inplace:
            return "헌법파일 쓰기/덮어쓰기 차단(soul·CLAUDE·*_DIRECTIVE). 변경 불가"
    return None

# ================= 메인 =================
def out_deny(reason):
    print("DENY\t" + reason); sys.exit(2)

if PAUSED and not PREFLIGHT:
    if tool in WRITE_TOOLS:
        out_deny("AUTOPILOT_PAUSED (박사님 kill-switch): 쓰기 도구 차단. `autopilot.sh resume` 필요")
    if tool == "Bash":
        nl = norm_loose(command); ntok = nl.split()
        first = (l_cmd_word(l_segments(nl)[0]) if l_segments(nl) else (None, []))[0]
        readonly = {"ls", "cat", "head", "tail", "grep", "rg", "find", "pwd", "echo", "wc", "stat", "file",
                    "which", "ps", "df", "du", "date", "env", "git", "python3", "pytest", "shasum", "sha256sum", "jq", "cmux"}
        is_control = "autopilot.sh" in nl
        has_redirect = (">" in ntok) or (">>" in ntok) or any(w.startswith(">") for w in ntok)
        if not is_control and (has_redirect or first not in readonly):
            out_deny("AUTOPILOT_PAUSED: 비읽기 Bash 차단(%s). `autopilot.sh resume` 필요" % (first or "?"))

if tool == "Bash" and command:
    if ACTIVE:
        r = strict_deny(command)
        if r: out_deny("[STRICT] " + r)
    else:
        r = loose_deny(norm_loose(command), norm_loose_slash(command))
        if r:
            # P1-2 HMAC 서명 우회: master가 서명한 명령 접두는 LOOSE deny를 우회한다.
            # cys approval check exit 0 = 서명됨 → 허용 / 미서명·cysd미가용 = fail-closed(deny 유지).
            import subprocess
            try:
                _sc = subprocess.run(["cys", "approval", "check", "--command", command,
                                      "--cwd", os.getcwd()], capture_output=True, timeout=3).returncode
            except Exception:
                _sc = 1
            if _sc == 0:
                sys.exit(0)  # 서명됨 → LOOSE deny 우회 허용
            out_deny("[LOOSE] " + r)

if tool in WRITE_TOOLS and file_path:
    if protected(file_path):
        # ★박사님 인가 루트(2026-06-07 박사님 제정): 박사님 직접명령('헌법에 넣어라'·'절대규칙에 기록')→
        #   master가 CONSTITUTION_EDIT_AUTHORIZED 토큰(스코프=헌법 basename 또는 '*') 생성→해당 파일 헌법편집 인가.
        #   ★autopilot(ACTIVE) 중엔 CONST_AUTH=0 강제(bash)=자율주행 자기인가·실수 차단 · TTL30분 · master 편집 직후 토큰 제거(단일배치).
        _scope_toks = (CONST_SCOPE.splitlines()[0].lower().split() if CONST_SCOPE.strip() else [])  # ★스코프=첫 줄 토큰만(주석의 * 오인 차단)·정확매칭
        if CONST_AUTH and (("*" in _scope_toks) or (basename(file_path).lower() in _scope_toks)):
            sys.stderr.write("AUTOPILOT GUARD: 헌법편집 인가됨(박사님 직접명령 토큰·%s)\n" % basename(file_path))
        else:
            out_deny("헌법파일(%s) %s 차단. 변경 불가 (박사님 직접명령 인가토큰 CONSTITUTION_EDIT_AUTHORIZED 필요)" % (basename(file_path), tool))
    if ACTIVE and guard_infra(file_path):   # 잔여3: STRICT trust boundary
        out_deny("[STRICT] guard 인프라(%s) %s 차단(자기보호 trust boundary)" % (basename(file_path), tool))

sys.exit(0)
PY

# ===== 실행 =====
GUARD_PAUSED=0; [ -e "$PAUSED_FILE" ] && GUARD_PAUSED=1
GUARD_ACTIVE=0; [ -e "$ACTIVE_FILE" ] && GUARD_ACTIVE=1

# ★박사님 직접명령 헌법편집 인가 토큰(2026-06-07): 非ACTIVE + 토큰존재(비심링크) + TTL 30분 → CONST_AUTH=1·스코프 전달
GUARD_CONST_AUTH=0; GUARD_CONST_SCOPE=""
CONST_TOKEN="$SCRIPT_DIR/CONSTITUTION_EDIT_AUTHORIZED"
if [ "$GUARD_ACTIVE" != "1" ] && [ -f "$CONST_TOKEN" ] && [ ! -L "$CONST_TOKEN" ]; then
  _cnow=$(date +%s); _cmt=$(stat -f %m "$CONST_TOKEN" 2>/dev/null || stat -c %Y "$CONST_TOKEN" 2>/dev/null || echo 0)
  if [ $(( _cnow - _cmt )) -le 1800 ]; then
    GUARD_CONST_AUTH=1; GUARD_CONST_SCOPE="$(cat "$CONST_TOKEN" 2>/dev/null)"
  fi
fi

run_loose_backstop() {
  printf '%s\n' "AUTOPILOT GUARD: python3 미가용 → bash 백스톱(LOOSE degraded) 모드" >&2
  local cmd fp low
  cmd="$(printf '%s' "$INPUT" | jq -r '.tool_input.command // ""' 2>/dev/null)"
  fp="$(printf '%s' "$INPUT" | jq -r '.tool_input.file_path // ""' 2>/dev/null)"
  low="$(printf '%s' "$cmd" | tr 'A-Z' 'a-z' | tr -s ' \t' ' ')"
  case "$low" in
    *"git push"*|*"git remote set-url"*|*"git remote remove"*|*"gh release create"*|*"gh pr create"*|*"scp "*)
      emit_deny "백스톱: 비가역 명령. 박사님 승인 필요" ;;
  esac
  if printf '%s' "$low" | grep -Eq '(^| )rm +-[a-z]*[rf]|(^| )(rmdir|chmod|chown|truncate) '; then
    case "$low" in *"_round/autopilot/"*) : ;; *) emit_deny "백스톱: 파괴적 파일연산. 박사님 승인 필요" ;; esac
  fi
  case "$(basename "$fp" 2>/dev/null | tr 'A-Z' 'a-z')" in
    soul.md|claude.md|*_directive.md) emit_deny "백스톱: 헌법파일 쓰기. 변경 불가" ;;
  esac
  exit 0
}

if [ -z "$PYBIN" ]; then
  # 잔여5: STRICT(자율주행) 중 parser 부재 → deny-by-default (LOOSE degraded 금지)
  [ "$GUARD_ACTIVE" = "1" ] && emit_deny "[STRICT] python3 미가용 — parser 부재 fail-closed deny(자율주행)"
  run_loose_backstop
fi

PYOUT="$(printf '%s' "$INPUT" | GUARD_PAUSED="$GUARD_PAUSED" GUARD_ACTIVE="$GUARD_ACTIVE" GUARD_CONST_AUTH="$GUARD_CONST_AUTH" GUARD_CONST_SCOPE="$GUARD_CONST_SCOPE" GUARD_PREFLIGHT="$PREFLIGHT" "$PYBIN" -c "$PYSRC" 2>/tmp/.guard_py_err.$$)"
RC=$?
rm -f /tmp/.guard_py_err.$$ 2>/dev/null
if [ "$RC" -eq 0 ]; then
  exit 0
elif [ "$RC" -eq 2 ]; then
  reason="${PYOUT#DENY$'\t'}"; [ "$reason" = "$PYOUT" ] && reason="$PYOUT"
  emit_deny "$reason"
else
  # 잔여5: STRICT 중 parser 크래시 → deny. LOOSE 만 백스톱 강등.
  [ "$GUARD_ACTIVE" = "1" ] && emit_deny "[STRICT] parser 크래시(rc=$RC) — fail-closed deny(자율주행)"
  run_loose_backstop
fi
