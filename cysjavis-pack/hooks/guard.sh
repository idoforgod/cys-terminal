#!/bin/bash
# guard.sh — Autopilot 집행 hook (Claude Code PreToolUse 진입점) · R3 deny-by-default allowlist
# SOT: _round/autopilot/SPEC.md · master 거버넌스(soul AUTONOMOUS PILOT ANCHOR 이행조건)
#
# ★주인님 (B') 결정: effect-denylist 는 shell Turing-complete 라 우회불가피(codex 입증)
#   → deny-by-default allowlist parser 근본전환(Phase3 R3 "무엇만 남길까" 명령레벨 적용).
#
# ★모드 분리:
#   - AUTOPILOT_ACTIVE 존재 = 자율주행 → STRICT: shlex grammar 파서, ALLOWLIST 외 전부 deny
#   - 플래그 無 = 평시(주인님 직접작업) → LOOSE : 명백 비가역만 차단(효과기반 denylist)
#   - AUTOPILOT_PAUSED 존재 = kill-switch(상위) → 비읽기 deny(autopilot.sh 도달성 예외)
#
# ★R3 반영(codex 재게이트 5잔여):
#   잔여1 sed --in-place/-i.bak/long-opt STRICT 차단
#   잔여2 git 가역 서브커맨드 내 파괴옵션(branch -d/-D·stash drop/clear·commit --amend·add 헌법경로) deny
#   잔여3 STRICT Write/Edit 의 guard 인프라(_round/autopilot/·플래그·settings.json) 자기보호(trust boundary)
#   잔여4 flag env override 는 GUARD_TEST_MODE 테스트 전용 — 운영은 canonical path 고정+심링크 거부
#   잔여5 STRICT 중 python 부재/크래시 = deny-by-default(LOOSE degraded 금지)
#
# ★F3 회귀 3건 수리(2026-08-01 · 근거 _work/absolute-rules-audit-20260731/PERF_COMPARISON_0148_vs_0149.md §4):
#   R-01(중대·지연) 데몬 무응답 시 판정 스톨 — LOOSE 8,084.5ms · STRICT 5,056.7ms(= 원장조회 5s +
#        승인확인 3s **직렬 합산**). 자원 회수가 필요한 바로 그 순간 회수가 마비되는 역설.
#        → 게이팅 경로 **전체를 합산 예산**(GATE_BUDGET_S)으로 묶고 초과 즉시 fail-closed DENY.
#   R-05(중간·오염) 가드의 원장 조회가 죽은 데몬을 **자동기동**(cys connect → spawn_detached_daemon ·
#        src/bin/cys.rs:1501·1535) — 실측에서 없는 소켓을 준 프로브가 cysd 1개를 띄우고 5노드를 부팅.
#        "가드가 상태를 바꾸지 않는다"는 주석이 거짓이 되는 경로다.
#        → 조회 전 **소켓 실재 선확인**(부재면 subprocess 자체를 띄우지 않음) + CYS_NO_AUTOSTART=1 이중봉인.
#   R-04(중간·게이트 정확도) `--socket` 재지정 시 guard 는 언제나 **기본 데몬**을 조회 → 대상 데몬에서
#        살아있는 노드가 기본 원장에선 안 보여 ALLOW. 현 머신 cysd 3개 동시가동 = 가설 아닌 실재 조건.
#        → cys_sub_strict 가 --socket 값을 보존해 조회 argv(`cys --socket <v> …`)에 그대로 전달.
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
import sys, json, re, unicodedata, os, shlex, time

PREFLIGHT = os.environ.get("GUARD_PREFLIGHT", "0") == "1"
PAUSED    = os.environ.get("GUARD_PAUSED", "0") == "1"
ACTIVE    = os.environ.get("GUARD_ACTIVE", "0") == "1"   # 자율주행 → STRICT
CONST_AUTH  = os.environ.get("GUARD_CONST_AUTH", "0") == "1"   # ★주인님 직접명령 헌법편집 인가 토큰 유효(bash 계산: 토큰존재+非ACTIVE+TTL30분)
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
# ★래퍼 **옵션**도 건너뛴다 (2026-08-16 감사 확정 · 실측 우회):
#   종전엔 래퍼 '이름'만 건너뛰어 `env -u CYS_SURFACE_ID cys factory-reset` 의 prog 가 `-u` 로
#   잡혔다 — cys 분기가 아예 발동하지 않아 오살 금지·R-02·R-02b 가 통째로 우회됐다.
#   값을 먹는 옵션은 다음 토큰까지 함께 건너뛴다. timeout/time 의 선행 기간 인자도 흡수한다.
WRAPPER_VAL_OPTS = {"-u", "-n", "-c", "-C", "-I", "-t", "-i", "--chdir", "--unset", "--niceness"}
DUR_RE = re.compile(r"^[0-9]+(\.[0-9]+)?[smhd]?$")
def skip_wrapper_opts(toks, i, n, wrapper):
    """래퍼 뒤의 옵션·값·기간 인자를 건너뛴 위치를 돌려준다(순수 인덱스 계산)."""
    while i < n:
        t = toks[i]
        if t.startswith("-") and t != "--":
            if t in WRAPPER_VAL_OPTS and i + 1 < n:
                i += 2
            else:
                i += 1
            continue
        if t == "--":
            i += 1
            continue
        if wrapper in ("timeout", "time") and DUR_RE.match(t):
            i += 1
            continue
        break
    return i

# ================= STRICT (deny-by-default allowlist) =================
ALLOWLIST = {"ls", "cat", "head", "tail", "grep", "rg", "find", "git", "pytest",
             "echo", "wc", "stat", "jq", "shasum", "cys", "sed", "python3"}
# 가역 서브명령(add·commit·diff·log·status·stash·show·branch·restore)=git reset/restore 로 되돌림 가능 → allow.
# ★보호파일(soul·CLAUDE·*_DIRECTIVE) basename 이 있어도 이들은 allow(staging·diff·commit 은 파일내용 변경 아님·가역) —
#   실제 보호는 Write/Edit·redirect 차단으로(격리). push·remote·tag생성=비가역 외부발행→제외(deny). (master 결정 2026-06-07)
GIT_OK = {"status", "log", "diff", "add", "show", "branch", "stash", "commit", "restore"}

# ★오살(誤殺) 금지 기계집행 (2026-08-01): cys 는 ALLOWLIST 상 **서브커맨드 구분 없이 통째 allow**였다
#   → STRICT 에서 `cys close-surface <살아있는 surface>` 가 무저항 통과(= 살아있는 노드 학살 경로).
#   준거 규범: CSO_DIRECTIVE [exited surface 자동 reap] "exited=true 를 발견 즉시 --reap 로 회수" ·
#   javis_reap_exited.py 불변식 "close-surface 는 오직 exited==true 로 수집된 ref 에만 호출된다.
#   live(exited=false) surface 는 어떤 인자·경로로도 대상이 되지 않는다".
#   → 산문 규범을 GIT_OK 와 같은 서브커맨드 게이팅으로 격상한다(deny-by-default 방향).
#
#   ★전수 식별(`cys --help` 전 서브커맨드 검토) — surface/프로세스 **종료** 능력 보유:
#     · close-surface = "Close a surface and force-kill its entire descendant process tree"  → 게이팅
#     · kill          = "Kill a ledger-registered process (group) by pid"                    → 게이팅
#   비대상(종료 능력 없음 → 기존대로 allow · **과차단 금지**):
#     send/send-key/list/status/read-screen/attach/events/identify/ps/run(자기 그룹만)/feed/
#     queue/quiesce/watch/todo-path/surface-role/set-status/recall/skill/approval/… 전부.
#     tombstone(=폐역 표식·`--remove` 로 가역·surface 를 죽이지 않음) · drain(저장 신호) ·
#     cycle-agent(clear) · node-recover/restore(죽은 노드 **재기동** = 복구 경로 — 막으면 §5-6 자기잠금)
#     는 '종료 능력' 요건 미충족이라 본 게이트 밖이다(별도 정책 층위로 남긴다).
#
#   ★모드 확장(2026-08-01 2차 수리 · 독립검증 갭 적발): 1차 수리는 게이팅을 **STRICT 경로에만**
#     걸었다 → LOOSE(평시)·PAUSED 에서 `cys close-surface <살아있는 surface>` 가 그대로 통과했다.
#     오살 금지는 모드에 딸린 편의규칙이 아니라 **모드 무관 절대규칙**이므로 판정 함수
#     (cys_kill_allowed)를 세 경로가 **공유**한다 — 진입로(STRICT=shlex 토큰 / LOOSE·PAUSED=
#     norm_loose 세그먼트)만 다르고 판정 기준은 하나다(단일 SOT · 기준 표류 방지).
#     · STRICT · LOOSE : exited=true 만 allow (동일 기준)
#     · PAUSED         : **무조건 deny** — 규범이 "pause 중 살아있는 타 노드 종료·재기동 전면 금지"·
#                        "pause 중 허용 상한 = 관측·저장·보고·자기 프로세스 종료 넷"이라
#                        exited=true 잔재 회수조차 pause 중엔 금지다(해제 후 하라).
CYS_KILL_SUBS = {"close-surface", "kill"}
# 값을 소비하는 cys 전역 옵션 — 건너뛰지 않으면 그 값이 위치인자(대상)로 오인된다.
CYS_VAL_OPTS = {"--socket"}
# deny 사유 공통 꼬리(주인님 가이드 문구 고정)
CYS_KILL_DENY_TAIL = ("오살 금지 — exited=true(죽은 잔재)만 회수 가능. 강제는 승인 경유"
                      "(주인님 승인 · `cys approval` 서명 prefix).")
# PAUSED 전용 사유(전면 금지 · 해제 경로 안내로 자기잠금 방지)
CYS_KILL_PAUSED_DENY = ("AUTOPILOT_PAUSED: cys %s 전면 금지 — pause 중 허용 상한은 "
                        "관측·저장·보고·자기 프로세스 종료 넷뿐이고, **살아있는 타 노드 종료·재기동은 "
                        "전면 금지**다. exited=true 잔재 회수도 pause 중엔 금지 — 해제 후 하라: "
                        "①`cys resume` ②AUTOPILOT_PAUSED 파일 2곳 제거"
                        "($PACK/AUTOPILOT_PAUSED · <프로젝트루트>/_round/AUTOPILOT_PAUSED)")

# ★R-02 부트스트랩 불가침 (2026-08-01 F2): cys 를 ALLOWLIST 에 **통째로** 등재한 부작용으로
#   STRICT 에서 팩 자기변경 서브커맨드가 열렸다(A=0.14.8 에선 cys 가 allowlist 밖이라 전부 DENY).
#   자율주행이 팩을 재설치·되돌릴 수 있으면 잔여3(STRICT Write/Edit guard 자기보호 trust boundary)이
#   한 명령으로 무력화된다 — Write/Edit 로는 못 건드리는 guard.sh 를 `cys pack-update` 가 통째로
#   갈아끼우기 때문이다. 그래서 CYS_KILL_SUBS 와 달리 대상 판정·조회 없이 **무조건 DENY** 한다.
#   ★전수 식별(`cys --help` 실측 2026-08-01 · /usr/local/bin/cys) — 팩 파일을 쓰는 서브커맨드:
#     · pack-update            "무중단 팩 업데이트(재시작 0) — 서명된 팩을 검증→디스크 반영→…reinject"
#     · init-pack              "Install the CYSJavis Pack … to ~/.cys/pack"
#     · pack-rollback          "직전 설치 보존본(<pack>.prev)에서 파일 단위 복원"
#     · pack-downgrade-to-free "pro 팩 설치를 free(내장 팩)로 강등 — 유일한 pro→free 경로"
#     · pack-merge             "커스터마이즈 병합 — …신버전(.new)·보존본(.user)을 검토·해소"
#   비대상(팩 본체 쓰기 없음 → 기존대로 allow · **과차단 금지**):
#     pack-plan("설치 **전에** 표시(쓰기 0)")·pack-ownership("소유권 등급 판정 — …결정론 조회")·
#     pack-manifest("stdout JSON으로 방출")·pack-repair-channel(".pack-state.json(채널 상태)" 한정)·
#     license(열쇠 번들)·doctor(--fix 도 "사용자 데이터·pack 본체·DB 미삭제").
#   ★LOOSE(평시)는 기존대로 allow — 주인님 직접 팩 업데이트가 사람 운영 경로다(막으면 자기잠금).
CYS_PACK_MUTATING_SUBS = {"pack-update", "init-pack", "pack-rollback",
                          "pack-downgrade-to-free", "pack-merge"}

# ★R-02b 완전 초기화 불가침 (2026-08-16 성찰 확정): `cys factory-reset` 은 팩 자기변경보다
#   **강한** 파괴력이다 — guard.sh 를 포함한 팩 전체·전 부서·대화기억·훅 등록을 격리하고 전 데몬을
#   죽인다(=오케스트레이션 자체의 소멸). R-02 가 "팩 재설치는 자기 안전장치 갈아끼우기라 무조건
#   DENY" 라면, 이 명령은 그 안전장치를 **통째로 들어내는** 상위 클래스다.
#   ★모드 무관 DENY 인 이유(R-02 와 다른 점): R-02 는 LOOSE 를 열어 뒀다 — 주인님의 팩 업데이트가
#   사람 운영 경로이기 때문이다. 그러나 factory-reset 의 사람 운영 경로는 **에이전트 pane 밖**
#   (GUI '완전 초기화' 버튼 · 외부 터미널)이고, CLI 자신도 pane 안 실행을 거부한다(cys.rs 자기살해
#   방지 가드). 즉 pane 안에서 이 명령이 나오는 상황은 정상 운영에 존재하지 않으므로 평시에도 막는다.
#   자기잠금 없음 — 주인님은 GUI 버튼·외부 터미널로 언제든 실행할 수 있다.
CYS_OWNER_ONLY_SUBS = {"factory-reset"}
# ★`cys run -- <명령>` 우회 봉인 (2026-08-16 감사 확정 · 실측 우회):
#   `run` 은 임의 명령을 프로세스 원장에 얹어 실행한다 — 그래서 `cys run -- cys factory-reset` 이
#   cys 서브커맨드 게이트(오살 금지·R-02·R-02b)를 통째로 빠져나갔다. `run` 자체는 서버 기동의
#   정당 경로라 전면 금지하지 않고(과차단 금지), **인자 안에 게이팅 대상 서브커맨드가 보이면**
#   거부한다. 판정은 토큰 정확 일치라 `--run-factory-reset-notes` 같은 무관 문자열은 걸리지 않는다.
CYS_RUN_GATED_TOKENS = CYS_OWNER_ONLY_SUBS | CYS_PACK_MUTATING_SUBS | CYS_KILL_SUBS
CYS_RUN_DENY = ("[우회 차단] cys run 의 내부 명령에 게이팅 대상 서브커맨드(%s)가 있다 — "
                "run 으로 감싸도 같은 규칙이 적용된다. 직접 실행 경로의 안내를 따르라.")
CYS_OWNER_ONLY_DENY = ("[완전 초기화 불가침] cys %s 금지 — 전 데몬 종료 + 팩(guard.sh 포함)·부서·"
                       "대화기억·훅 전량 격리는 에이전트가 실행할 명령이 아니다. 주인님이 직접 "
                       "GUI '완전 초기화' 버튼 또는 **에이전트 pane 밖 터미널**에서 실행한다.")
CYS_PACK_DENY = ("[부트스트랩 불가침] cys %s 금지 — 팩 자기변경(guard.sh 를 포함한 팩 전체 재설치·"
                 "복원)은 자율주행이 자기 안전장치를 갈아끼우는 경로다. 승인 경유로만 — 주인님 직접 "
                 "실행 또는 AUTOPILOT_ACTIVE 해제 후(평시 LOOSE 는 허용).")

# ★R-03 PAUSED 상한 화이트리스트 (2026-08-01 F2): `cys` 가 PAUSED readonly 집합에 **통째로** 등재된
#   상속 결함으로 launch-agent·boot·node-recover·restore·cycle-agent·new-surface·tombstone·
#   doctor --fix·persona set 이 pause 중에도 전부 통과했다. 규범(운영계약 v0.4 · 색인 §6 메타안전)의
#   pause 중 허용 상한은 **관측·저장·보고·자기 프로세스 종료** 넷이므로, cys 만은 denylist 가 아니라
#   **allowlist** 로 뒤집는다(guard 전체의 deny-by-default 철학과 같은 방향).
#   ★각 항목 근거 = `cys --help` 실측 설명(2026-08-01 · /usr/local/bin/cys):
#     관측 · status "T1-2 통합 관제 보드: 전 노드 상태를 1콜로" / list "List surfaces" /
#            read-screen "Read a surface's screen (vt100-accurate)…" / identify "Identify daemon + caller" /
#            ps "Show the process ledger" / events "Subscribe to the daemon event stream (push; no polling)" /
#            gate-check "preflight 게이트: exit 0 = running, 4 = paused" /
#            attest → pin "Print the current chain pin" · verify "Verify a previously saved pin
#                     (exit 0=match, 2=mismatch)" = 두 하위 모두 출력·대조 전용(쓰기 없음)
#     저장 · todo-path "Print (creating if absent) this surface's role-specific TODO file path"
#            (자기 역할 todo 파일뿐 — `--role` 은 "경로 산출 전용(파일 생성·기록 없음)") /
#            drain "살아있는 노드에 저장 신호 + 유예(best-effort drain)"(--verify 도 SESSION_STATE 확인)
#     보고 · send "Inject text into a surface's stdin" / send-key "Inject a named key" /
#            set-status "이 에이전트의 상태·컨텍스트%·작업을 데몬에 신고"
#     제어 · resume "kill-switch 해제 — 동결된 큐·스케줄 재개" / pause "T4-15 kill-switch"
#            (deny 사유가 안내하는 해제 경로는 PAUSED 에서도 도달 가능해야 한다 — 자기잠금 방지 §1-3)
#   ★그 외 전부 DENY. 특히 실측으로 '상태를 만든다'가 확인된 것들:
#     launch-agent "Launch an AI agent in a new role surface" · boot "Boot the standard node set" ·
#     new-surface "Create a new surface (PTY session)" · node-recover "죽은 에이전트를 …재기동" ·
#     restore "…죽은 역할들을 일괄 재기동" · cycle-agent "…clear→지침 재주입" ·
#     tombstone "역할을 topology 묘비에 심는다" · doctor(--fix 수리) · persona set(오버라이드 저장) ·
#     run "Run a command in a new process group"(임의 명령 실행) 등.
CYS_PAUSED_OK = {"status", "list", "read-screen", "identify", "ps", "todo-path",
                 "send", "send-key", "set-status", "resume", "pause", "gate-check",
                 "queue", "events", "attest", "drain"}
# 화이트리스트 안이지만 **하위 커맨드가 조회/파괴로 갈리는 것** — 조회 하위만 allow(그 외 fail-closed).
#   queue(실측): list "List undelivered queued messages"=조회 allow /
#                clear "Drop all undelivered queued messages for a surface"=파괴 deny
#   attest·drain 은 하위·플래그가 전부 조회·저장이라 분리 불필요(위 근거 주석 참조).
CYS_PAUSED_SUBSUB_OK = {"queue": {"list"}}
CYS_PAUSED_SUB_DENY = ("AUTOPILOT_PAUSED: cys %s 금지 — pause 중 허용 상한은 **관측·저장·보고·"
                       "자기 프로세스 종료** 넷뿐이다(cys 허용: %s). 해제는 주인님 명시 지시로만 — "
                       "①`cys resume` ②AUTOPILOT_PAUSED 파일 2곳 제거"
                       "($PACK/AUTOPILOT_PAUSED · <프로젝트루트>/_round/AUTOPILOT_PAUSED)")

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
                w = toks[i].rsplit("/", 1)[-1].lower()
                i += 1
                i = skip_wrapper_opts(toks, i, n, w)
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

def cys_sub_strict(args):
    """cys 서브커맨드 + 위치인자 + --socket 값 추출(git_sub_strict 와 동일 관용구).
    반환 (sub, positionals, socket) · 서브커맨드 불명이면 (None, [], socket).
    ★F3-R04: socket 은 **미지정이면 None**, `--socket` 이 붙었는데 값을 못 읽으면 `''`(빈 문자열)이다 —
      빈 문자열은 '어느 데몬을 볼지 불명'이므로 호출부가 fail-closed DENY 한다(값 판독 실패=거부)."""
    i = 0
    sub = None
    pos = []
    sock = None
    while i < len(args):
        a = args[i]
        if a.startswith("-"):
            if "=" in a:
                k, v = a.split("=", 1)
                if k in CYS_VAL_OPTS: sock = v            # --socket=/path/cys.sock
                i += 1; continue
            if a in CYS_VAL_OPTS:
                sock = args[i+1] if i + 1 < len(args) else ""   # 값 누락 → '' = 판독 불가
                i += 2; continue
            i += 1; continue
        if sub is None:
            sub = a
        else:
            pos.append(a)
        i += 1
    return sub, pos, sock

# ── F3-R01: 게이팅 경로 **합산 예산** ────────────────────────────────────────────────
# 회귀 실측(§R-01): 데몬 무응답 시 원장조회 timeout 5s + 승인확인 timeout 3s 가 직렬 합산돼
# 판정 하나가 LOOSE 8,084.5ms · STRICT 5,056.7ms 멈췄다. 타임아웃 상수가 지배하므로 머신 성능과
# 무관하게 재현되고, "데몬이 아플 때만" 나타나 정상 테스트에 안 걸리는 최악의 실패 양식이다.
# → 외부 조회 **전체**를 하나의 예산으로 묶는다. 예산 초과 = 판정 불가 = 즉시 fail-closed DENY
#   (느리게 통과시키느니 빠르게 거부한다).
# 수치 근거(실측): 정상 경로 `cys status --json` 20~30ms · `cys list`·`approval check` <10ms.
#   최악 산식 = 첫 조회 CAP(0.6s) + 잔여 예산(0.4s) = **BUDGET 1.0s**, 여기에 bash 프리루드·
#   인터프리터 기동 실측 45~55ms 를 더해도 **총 1.5초 계약** 대비 ~30% 여유다(예산 시계가 python
#   진입 시점 기준이므로 그 앞 구간은 헤드룸으로 흡수). 단일 조회 600ms 는 정상치의 20~30배라
#   정상 경로 오탐 0 — 그보다 느린 데몬은 '아픈 데몬'이고, 그때의 DENY 는 의도된 fail-closed 다.
GATE_BUDGET_S = 1.0      # 게이팅 1회당 외부 조회 합산 상한
PROBE_CAP_S   = 0.6      # 단일 조회 상한(하나가 예산을 독식해 뒤 조회를 굶기지 못하게)
MIN_SLICE_S   = 0.05     # 남은 예산이 이보다 적으면 조회를 시작하지 않는다(시작해도 무의미)
if os.environ.get("GUARD_TEST_MODE", "0") == "1" and os.environ.get("GUARD_TEST_GATE_BUDGET_MS", ""):
    try:                                                  # 테스트 전용(잔여4 GUARD_TEST_MODE 관용구)
        GATE_BUDGET_S = float(os.environ["GUARD_TEST_GATE_BUDGET_MS"]) / 1000.0
        PROBE_CAP_S = min(PROBE_CAP_S, GATE_BUDGET_S)
    except Exception:
        pass
GATE_T0 = time.monotonic()

def budget_left():
    return GATE_BUDGET_S - (time.monotonic() - GATE_T0)

def gate_timeout_deny(where):
    """예산 초과 → 즉시 fail-closed DENY. out_deny 는 아래 '메인' 절에서 정의되지만 이 함수는
    디스패치 시점에만 호출되므로(모듈 로드 시 아님) 이름 해석이 늦어도 안전하다."""
    out_deny("판정 시간 초과(게이팅 합산 예산 %.2fs · %s) — 안전상 거부(fail-closed). "
             "데몬 무응답 시 판정을 기다리지 않는다(회복 후 재시도)." % (GATE_BUDGET_S, where))

# ── F3-R04/R05: 조회 대상 데몬 해석 + 소켓 실재 선확인 ───────────────────────────────
# 해석 순서 근거(코드 실측): src/bin/cys.rs:957 `--socket` → CYS_SOCKET env 주입 ·
# src/lib.rs:20 ENV_SOCKET="CYS_SOCKET" · src/lib.rs:26 env_compat = CYS_→JAVIS_→AITERM_ 폴백 ·
# src/lib.rs:54 기본 = (XDG_STATE_HOME|~/.local/state)/cys/cys.sock · windows = \\.\pipe\cys.
def cys_socket_path(explicit=None):
    if explicit is not None:
        return explicit
    for k in ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET"):
        v = os.environ.get(k)
        if v:
            return v
    if os.name == "nt":
        return r"\\.\pipe\cys"
    base = os.environ.get("XDG_STATE_HOME") or os.path.join(os.path.expanduser("~"), ".local", "state")
    return os.path.join(base, "cys", "cys.sock")

def socket_ready(path):
    """★F3-R05 — 조회 **전** 소켓 실재 확인. cys 의 connect() 는 소켓이 없으면
    spawn_detached_daemon() 으로 데몬을 자동기동한다(src/bin/cys.rs:1501·1535) — 실측에서
    없는 소켓을 준 프로브가 cysd 를 띄우고 그 데몬이 5노드를 자동 부팅했다.
    가드는 **관측만** 해야 하므로 부재면 조회 자체를 하지 않는다(= 상태 무변경 주석의 참값 복구)."""
    if not path:
        return False
    if os.name == "nt":
        return True     # named pipe 는 stat 계약이 달라 존재검사 생략 — CYS_NO_AUTOSTART 로 방어
    try:
        import stat as _stat
        return _stat.S_ISSOCK(os.stat(path).st_mode)   # 심링크 추적 = cys connect 와 동일 관점
    except Exception:
        return False

def _cys_exec(tail, socket=None, soft=False):
    """cys 결정론 조회 러너 — (returncode, stdout str) · 조회 불가는 None(=fail-closed 신호).
    tail = 서브커맨드 이하(['status','--json']) · 전역 옵션은 여기서 붙인다.
    ★조회는 읽기전용 서브커맨드로만 한다 — 가드가 상태를 바꾸지 않는다(R-05 이중봉인:
      소켓 실재 선확인 + CYS_NO_AUTOSTART=1 · 후자는 cys 0.14.7 바이너리에 실재하는 옵트아웃).
    ★R-04: 해석된 소켓을 `--socket` 으로 **명시 전달** — 어느 데몬을 봤는지가 argv 에 남는다.
    ★R-01: 남은 예산으로 timeout 을 잘라 쓰고, 예산 소진은 즉시 DENY(soft=True 면 None 반환)."""
    import subprocess
    sock = cys_socket_path(socket)
    left = budget_left()
    if left <= MIN_SLICE_S:
        if soft:
            return None
        gate_timeout_deny("cys " + " ".join(tail[:2]))
    if not socket_ready(sock):
        return None                      # 소켓 부재 = 조회 불가 → 호출부 fail-closed(데몬 기동 시도 0)
    env = dict(os.environ)
    env["CYS_NO_AUTOSTART"] = "1"        # stale 소켓(파일만 남고 데몬 사망)에서도 자동기동 금지
    try:
        p = subprocess.run(["cys", "--socket", sock] + list(tail), capture_output=True,
                           timeout=min(left, PROBE_CAP_S), stdin=subprocess.DEVNULL, env=env)
    except Exception:
        return None                      # 타임아웃·spawn 실패 = 조회 불가(fail-closed)
    try:
        return (p.returncode, (p.stdout or b"").decode("utf-8", "replace"))
    except Exception:
        return None

def _cys_run(tail, socket=None):
    """성공(exit 0) 시 stdout(str), 실패·예외·비0 exit 은 None(=fail-closed 신호) — 기존 계약 유지."""
    r = _cys_exec(tail, socket=socket)
    if r is None or r[0] != 0:
        return None
    return r[1]

def cys_surfaces(socket=None):
    """`cys status --json` → surfaces[] (데몬 권위 판정). 조회·파싱 불가면 None = fail-closed 신호.
    ★javis_reap_exited.fetch_surfaces 와 동일 계약 — 화면 파싱 금지, JSON 계약만."""
    if os.environ.get("GUARD_TEST_MODE", "0") == "1" and os.environ.get("GUARD_TEST_CYS_STATUS", ""):
        raw = os.environ["GUARD_TEST_CYS_STATUS"]        # 테스트 전용(잔여4 GUARD_TEST_MODE 관용구)
    else:
        raw = _cys_run(["status", "--json"], socket=socket)
    if raw is None:
        return None
    try:
        d = json.loads(raw)
    except Exception:
        return None
    s = d.get("surfaces") if isinstance(d, dict) else None
    return s if isinstance(s, list) else None

def cys_live_pids(socket=None):
    """살아있는(exited != true) surface 의 pid 집합. 조회 불가면 None = fail-closed 신호.
    ★`cys status --json` 에는 pid 필드가 없다(실측) → pid↔surface 매핑의 유일 소스가 `cys list` 다.
      줄 형식: 'surface:238\\trole=worker\\tpid=97239\\texited=false\\t...' → 구분자 비의존 정규식으로 판독."""
    if os.environ.get("GUARD_TEST_MODE", "0") == "1" and os.environ.get("GUARD_TEST_CYS_LIST", ""):
        raw = os.environ["GUARD_TEST_CYS_LIST"]          # 테스트 전용
    else:
        raw = _cys_run(["list"], socket=socket)
    if raw is None:
        return None
    live = {}
    rows = 0
    for line in raw.splitlines():
        mp = re.search(r"\bpid=(\d+)\b", line)
        me = re.search(r"\bexited=(\w+)\b", line)
        if not mp or not me:
            continue
        rows += 1
        if me.group(1).strip().lower() == "true":        # 죽은 잔재의 pid 는 회수 대상 → live 아님
            continue
        mr = re.match(r"\s*(\S+)", line)
        live[mp.group(1)] = (mr.group(1) if mr else "?")
    # ★fail-open 봉인(자체 하네스 ⑥-b 적발): '판독 성공했고 live 0건'과 '아예 판독 불가'를 구분한다.
    #   판독 가능한 surface 행이 **한 줄도** 없으면 형식 변경·출력 잘림·오류문구이므로 None(=deny).
    #   빈 dict 를 그대로 돌려주면 `cys kill <살아있는 노드 pid>` 가 전부 통과한다(오살 재개통).
    if rows == 0:
        return None
    return live

def _surface_key(tok):
    """'surface:238' 과 '238' 을 같은 키로 정규화(대상 지정 표기 흔들림 흡수)."""
    t = (tok or "").strip()
    if t.lower().startswith("surface:"):
        t = t.split(":", 1)[1]
    return t.strip()

def cys_kill_allowed(sub, pos, socket=None):
    """오살 금지 판정 — (allow: bool, reason: str). **판정 불가는 전부 deny(fail-closed)**.
    ★F3-R04: socket 은 명령이 겨냥한 데몬이다 — 판정 원장은 **그 데몬**에서 읽는다."""
    if socket is not None and not str(socket).strip():
        return False, ("cys %s: --socket 값 판독 불가 — 어느 데몬의 원장으로 판정할지 불명 "
                       "fail-closed. %s" % (sub, CYS_KILL_DENY_TAIL))
    if not pos:
        return False, "cys %s 대상 인자 불명(fail-closed). %s" % (sub, CYS_KILL_DENY_TAIL)
    target = pos[0]
    if sub == "close-surface":
        surfaces = cys_surfaces(socket)
        if surfaces is None:
            return False, ("cys close-surface: 데몬 상태 조회 실패(`cys --socket %s status --json`) — "
                           "대상 생사 판정 불가 fail-closed. %s"
                           % (cys_socket_path(socket), CYS_KILL_DENY_TAIL))
        key = _surface_key(target)
        for s in surfaces:
            if not isinstance(s, dict):
                continue
            if _surface_key(str(s.get("surface_ref") or "")) != key and str(s.get("surface_id")) != key:
                continue
            if s.get("exited") is True:   # ★엄격 bool 비교(truthy 오염 차단 · reap 도구 불변식과 동일)
                return True, ""
            return False, ("cys close-surface %s: 대상이 **살아있다**(exited=false · role=%s). %s"
                           % (target, s.get("role"), CYS_KILL_DENY_TAIL))
        return False, ("cys close-surface %s: 데몬 원장에 없는 대상 — 생사 판정 불가 fail-closed. %s"
                       % (target, CYS_KILL_DENY_TAIL))
    # sub == "kill": 원장 프로세스(서버 잔재) 정리는 정당한 자원 거버넌스(§5-1 `cys ps`·`cys kill`)라
    # allow 가 기본이되, 그 pid 가 **살아있는 surface 의 pid** 면 노드 오살이므로 deny.
    if not re.match(r"^\d+$", target.strip()):
        return False, "cys kill 대상 pid 판독 불가('%s') fail-closed. %s" % (target, CYS_KILL_DENY_TAIL)
    live = cys_live_pids(socket)
    if live is None:
        return False, ("cys kill: surface 원장 조회 실패(`cys --socket %s list`) — pid 소유 판정 불가 "
                       "fail-closed. %s" % (cys_socket_path(socket), CYS_KILL_DENY_TAIL))
    ref = live.get(target.strip())
    if ref:
        return False, ("cys kill %s: 살아있는 surface(%s)의 pid — 노드 오살. %s"
                       % (target, ref, CYS_KILL_DENY_TAIL))
    return True, ""

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
    if prog == "cys":
        # ★오살 금지 게이팅: 종료 능력 보유 서브커맨드만 대상 생사를 결정론 조회해 판정한다.
        #   그 외 cys 서브커맨드(send·send-key·list·status·read-screen…)는 **기존과 동일하게 allow**
        #   — 노드 간 정상 통신을 막으면 오케스트레이션 자체가 죽는다(과차단 금지).
        sub, pos, sock = cys_sub_strict(args)
        if sub in CYS_KILL_SUBS:
            return cys_kill_allowed(sub, pos, sock)
        # ★R-02 부트스트랩 불가침(STRICT 전용 · LOOSE 는 사람 운영 경로라 불변):
        #   팩 자기변경은 대상·상태 판정 없이 무조건 deny(자율주행이 guard.sh 를 재설치하는 경로 봉인).
        if sub in CYS_PACK_MUTATING_SUBS:
            return False, CYS_PACK_DENY % sub
        # ★R-02b: 완전 초기화는 모드 무관 DENY(위 상수 주석의 근거 — 사람 운영 경로가 pane 밖이다).
        if sub in CYS_OWNER_ONLY_SUBS:
            return False, CYS_OWNER_ONLY_DENY % sub
        if sub == "run":
            hit = [a for a in args if a in CYS_RUN_GATED_TOKENS]
            if hit:
                return False, CYS_RUN_DENY % ", ".join(sorted(set(hit)))
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
        w = ws[0].rsplit("/", 1)[-1].lower()
        rest = ws[1:]
        rest = rest[skip_wrapper_opts(rest, 0, len(rest), w):]
        ws = l_strip_env(rest)
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

def cys_kill_scan(n):
    """LOOSE/PAUSED 진입로 — 정규화 명령문 n 의 **모든 세그먼트**에서 종료능력 cys 서브커맨드를
    찾아 (sub, positionals) 를 돌려준다(없으면 (None, [])).
    ★첫 세그먼트만 보면 `ls ; cys close-surface 238` 이 샌다(PAUSED 기존 판정은 첫 세그먼트만 본다).
      이 스캔은 close-surface/kill **두 서브커맨드에만** 작용하므로 다른 명령의 판정은 불변이다."""
    for seg in l_segments(n):
        prog, args = l_cmd_word(seg)
        if prog != "cys":
            continue
        sub, pos, sock = cys_sub_strict(args)
        if sub in CYS_KILL_SUBS:
            return sub, pos, sock
    return None, [], None

def cys_paused_scan(n):
    """R-03 PAUSED 진입로 — 정규화 명령문 n 의 **모든 세그먼트**에서 cys 호출을 찾아
    화이트리스트(CYS_PAUSED_OK) 밖 서브커맨드를 돌려준다(없으면 None).
    ★cys_kill_scan 과 같은 전(全) 세그먼트 진입로를 쓴다 — 첫 세그먼트만 보면
      `ls ; cys launch-agent --role worker` 가 그대로 샌다(기존 PAUSED 판정의 구멍).
    ★서브커맨드 판독 불가(`cys` 단독·옵션만)도 위반으로 본다(fail-closed · 판정 불가는 deny)."""
    for seg in l_segments(n):
        prog, args = l_cmd_word(seg)
        if prog != "cys":
            continue
        sub, pos, _sock = cys_sub_strict(args)
        if sub is None:
            return "(서브커맨드 불명)"
        if sub not in CYS_PAUSED_OK:
            return sub
        allowed2 = CYS_PAUSED_SUBSUB_OK.get(sub)
        if allowed2 is not None:                      # 하위 커맨드가 조회/파괴로 갈리는 것(queue)
            sub2 = pos[0] if pos else None
            if sub2 not in allowed2:
                return "%s %s" % (sub, sub2 if sub2 else "(하위 커맨드 불명)")
    return None

def loose_deny(n, n_slash=""):
    for seg in l_segments(n):
        prog, args = l_cmd_word(seg)
        if prog is None: continue
        if prog == "cys":
            # ★오살 금지는 모드 무관 절대규칙 — LOOSE 도 STRICT 와 **동일 기준**(exited=true 만 allow)을
            #   같은 판정 함수로 집행한다. 그 외 cys 서브커맨드(send·list·status·read-screen…)는
            #   여기서 아무 것도 하지 않는다 = 평시 동작 완전 무변화(과차단 금지).
            sub, pos, sock = cys_sub_strict(args)
            if sub in CYS_KILL_SUBS:
                ok, reason = cys_kill_allowed(sub, pos, sock)
                if not ok:
                    return reason
            # ★R-02b: 완전 초기화는 평시(LOOSE)에도 DENY — 사람 운영 경로가 pane 밖이라 자기잠금 0.
            if sub in CYS_OWNER_ONLY_SUBS:
                return CYS_OWNER_ONLY_DENY % sub
            if sub == "run":
                hit = [a for a in args if a in CYS_RUN_GATED_TOKENS]
                if hit:
                    return CYS_RUN_DENY % ", ".join(sorted(set(hit)))
        if prog == "git":
            sub, subargs = l_git_sub(args)
            if sub == "push": return "git push (외부발행=비가역). 주인님 승인 필요"
            if sub == "remote" and any(x in subargs for x in ("set-url", "add", "remove", "rm", "rename")):
                return "git remote set-url/add/remove/rename (발행대상 변경). 주인님 승인 필요"
            if sub == "tag":
                create = any(x in subargs for x in ("-a", "-s", "-d", "-f", "-m")) or any(not x.startswith("-") for x in subargs)
                if create and not any(x in subargs for x in ("-l", "--list", "-n")):
                    return "git tag 생성/삭제. 주인님 승인 필요"
        if prog == "gh":
            if "release" in args and any(x in args for x in ("create", "upload", "delete", "edit")):
                return "gh release 발행. 주인님 승인 필요"
            if "pr" in args and any(x in args for x in ("create", "merge")):
                return "gh pr create/merge. 주인님 승인 필요"
        if prog in ("rm", "rmdir", "mv", "truncate", "chmod", "chown"):
            if not l_fileop_allowed(args):
                return "%s (비가역 파일연산). _round/autopilot/ 외 → 주인님 승인 필요" % prog
        if prog == "scp":
            return "scp (외부전송). 주인님 승인 필요"
        if prog == "rsync":
            if any(("@" in a) or re.match(r"^[^/\-][^/]*:", a) or "::" in a for a in args):
                return "rsync 원격전송. 주인님 승인 필요"
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
                return "%s 업로드/POST (외부전송). 주인님 승인 필요" % prog
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
        out_deny("AUTOPILOT_PAUSED (주인님 kill-switch · 호칭 정의처=마스터 헌장 제1조): "
                 "쓰기 도구 차단. 해제는 주인님 명시 지시로만 — ①`cys resume` "
                 "②AUTOPILOT_PAUSED 파일 2곳 제거($PACK/AUTOPILOT_PAUSED · "
                 "<프로젝트루트>/_round/AUTOPILOT_PAUSED)")
    if tool == "Bash":
        nl = norm_loose(command); ntok = nl.split()
        # ★PAUSED 상한(운영계약 v0.4 · 색인 §6 자율주행 메타안전): "pause 중 허용 범위 상한은
        #   관측·저장·보고·자기 프로세스 종료 넷"이며 "살아있는 타 노드 종료·재기동 pause 중 전면 금지".
        #   → close-surface/kill 은 **exited=true 여도 무조건 deny**(회수는 해제 후).
        #   ★이 검사가 없으면 PAUSED 가 세 모드 중 가장 느슨해진다 — `cys` 가 아래 readonly 집합에
        #     있어 `cys close-surface <살아있는 노드>` 가 PAUSED 게이트를 그대로 통과하기 때문이다.
        #   ★readonly/is_control 판정보다 **먼저** 두어 규범 사유가 그대로 회신되게 한다.
        _psub, _ppos, _psock = cys_kill_scan(nl)
        if _psub:
            out_deny(CYS_KILL_PAUSED_DENY % _psub)
        # ★R-03 pause 상한 집행(2026-08-01 F2): cys 는 **화이트리스트 방식** — 상한(관측·저장·보고·
        #   자기 프로세스 종료) 밖 서브커맨드는 전부 deny. 아래 readonly/is_control 판정보다 **먼저**
        #   두어야 한다 — `cys` 가 readonly 집합에 통째로 있어 그 뒤로 가면 무조건 통과해버린다
        #   (이 상속 결함이 launch-agent·boot·node-recover·restore·cycle-agent·new-surface·
        #    tombstone·doctor --fix·persona set 을 pause 중에도 열어둔 원인이다).
        _bad = cys_paused_scan(nl)
        if _bad:
            out_deny(CYS_PAUSED_SUB_DENY % (_bad, " ".join(sorted(CYS_PAUSED_OK))))
        first = (l_cmd_word(l_segments(nl)[0]) if l_segments(nl) else (None, []))[0]
        readonly = {"ls", "cat", "head", "tail", "grep", "rg", "find", "pwd", "echo", "wc", "stat", "file",
                    "which", "ps", "df", "du", "date", "env", "git", "python3", "pytest", "shasum", "sha256sum", "jq", "cys"}
        # ★자기잠금 방지(운영계약 §1-3 준용): deny 메시지가 안내하는 해제 경로는 PAUSED
        #   상태에서도 도달 가능해야 한다. 존재하지 않는 스크립트 대신 실재 해제 명령
        #   (`cys resume`·`cys pause`·`cys gate-check`)만 통제 명령으로 인정한다.
        is_control = bool(re.match(r"\s*cys\s+(resume|pause|gate-check)\b", nl))
        has_redirect = (">" in ntok) or (">>" in ntok) or any(w.startswith(">") for w in ntok)
        if not is_control and (has_redirect or first not in readonly):
            out_deny("AUTOPILOT_PAUSED: 비읽기 Bash 차단(%s). 해제는 주인님 명시 지시로만 — "
                     "①`cys resume` ②AUTOPILOT_PAUSED 파일 2곳 제거"
                     "($PACK/AUTOPILOT_PAUSED · <프로젝트루트>/_round/AUTOPILOT_PAUSED)"
                     % (first or "?"))

if tool == "Bash" and command:
    if ACTIVE:
        r = strict_deny(command)
        if r: out_deny("[STRICT] " + r)
    else:
        r = loose_deny(norm_loose(command), norm_loose_slash(command))
        if r:
            # P1-2 HMAC 서명 우회: master가 서명한 명령 접두는 LOOSE deny를 우회한다.
            # cys approval check exit 0 = 서명됨 → 허용 / 미서명·cysd미가용 = fail-closed(deny 유지).
            # ★F3-R01: 이 확인도 **같은 합산 예산** 안에서 한다(종전 timeout 3s 가 원장조회 5s 뒤에
            #   직렬로 붙어 8.08s 스톨을 만들었다). 예산이 남지 않으면 확인을 **건너뛰고 즉시 DENY** —
            #   원장 조회가 이미 실패한 상황이라 결론은 어차피 DENY 로 동일하다(보고서 완화책 ②).
            # ★F3-R05: 이 경로도 cys connect → 자동기동 표면이므로 _cys_exec 의 소켓 실재 선확인·
            #   CYS_NO_AUTOSTART 를 그대로 탄다(가드는 관측만 한다).
            try:
                _cwd = os.getcwd()
            except Exception:
                _cwd = "."
            _ap = _cys_exec(["approval", "check", "--command", command, "--cwd", _cwd], soft=True)
            if _ap is not None and _ap[0] == 0:
                sys.exit(0)  # 서명됨 → LOOSE deny 우회 허용
            out_deny("[LOOSE] " + r)

if tool in WRITE_TOOLS and file_path:
    if protected(file_path):
        # ★오너 인가 루트(제품 기본 절차): 오너 직접명령('헌법에 넣어라'·'절대규칙에 기록')→
        #   master가 CONSTITUTION_EDIT_AUTHORIZED 토큰(스코프=헌법 basename 또는 '*') 생성→해당 파일 헌법편집 인가.
        #   ★autopilot(ACTIVE) 중엔 CONST_AUTH=0 강제(bash)=자율주행 자기인가·실수 차단 · TTL30분 · master 편집 직후 토큰 제거(단일배치).
        _scope_toks = (CONST_SCOPE.splitlines()[0].lower().split() if CONST_SCOPE.strip() else [])  # ★스코프=첫 줄 토큰만(주석의 * 오인 차단)·정확매칭
        if CONST_AUTH and (("*" in _scope_toks) or (basename(file_path).lower() in _scope_toks)):
            sys.stderr.write("AUTOPILOT GUARD: 헌법편집 인가됨(주인님 직접명령 토큰·%s)\n" % basename(file_path))
        else:
            out_deny("헌법파일(%s) %s 차단. 변경 불가 (주인님 직접명령 인가토큰 CONSTITUTION_EDIT_AUTHORIZED 필요)" % (basename(file_path), tool))
    if ACTIVE and guard_infra(file_path):   # 잔여3: STRICT trust boundary
        out_deny("[STRICT] guard 인프라(%s) %s 차단(자기보호 trust boundary)" % (basename(file_path), tool))

sys.exit(0)
PY

# ===== 실행 =====
GUARD_PAUSED=0; [ -e "$PAUSED_FILE" ] && GUARD_PAUSED=1
GUARD_ACTIVE=0; [ -e "$ACTIVE_FILE" ] && GUARD_ACTIVE=1

# ★오너 직접명령 헌법편집 인가 토큰: 非ACTIVE + 토큰존재(비심링크) + TTL 30분 → CONST_AUTH=1·스코프 전달
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
      emit_deny "백스톱: 비가역 명령. 주인님 승인 필요" ;;
  esac
  if printf '%s' "$low" | grep -Eq '(^| )rm +-[a-z]*[rf]|(^| )(rmdir|chmod|chown|truncate) '; then
    case "$low" in *"_round/autopilot/"*) : ;; *) emit_deny "백스톱: 파괴적 파일연산. 주인님 승인 필요" ;; esac
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

# ★H-WIN-1(2026-08-10 Windows 실기 run 31400677188): PYSRC(≈42k자)를 `-c` argv 로 넘기면
#   Windows CreateProcess 명령행 상한(32,767자)에 걸려 **파서 스폰 자체가 실패**한다(E2BIG) —
#   모든 판정이 bash 백스톱으로 강등돼, Write 차단은 우연히(cygwin basename 이 백슬래시를
#   성분으로 취급) 살아남지만 LOOSE 의 writers(tee 등) 헌법파일 차단이 무음 통과했다.
#   → 소스를 mktemp 파일로 내리고 파일 경로(cys_native_path 변환)로 실행한다. mktemp/쓰기
#   불가 환경만 종전 argv 경로로 폴백한다(POSIX 는 argv 상한이 커서 종전 경로도 안전).
GUARD_PYFILE="$(mktemp "${TMPDIR:-/tmp}/.guard_pysrc.XXXXXX" 2>/dev/null || printf '%s' '')"
if [ -n "$GUARD_PYFILE" ] && printf '%s' "$PYSRC" > "$GUARD_PYFILE" 2>/dev/null; then
  PYOUT="$(printf '%s' "$INPUT" | GUARD_PAUSED="$GUARD_PAUSED" GUARD_ACTIVE="$GUARD_ACTIVE" GUARD_CONST_AUTH="$GUARD_CONST_AUTH" GUARD_CONST_SCOPE="$GUARD_CONST_SCOPE" GUARD_PREFLIGHT="$PREFLIGHT" "$PYBIN" "$(cys_native_path "$GUARD_PYFILE")" 2>/tmp/.guard_py_err.$$)"
  RC=$?
  rm -f "$GUARD_PYFILE" 2>/dev/null
else
  [ -n "$GUARD_PYFILE" ] && rm -f "$GUARD_PYFILE" 2>/dev/null
  PYOUT="$(printf '%s' "$INPUT" | GUARD_PAUSED="$GUARD_PAUSED" GUARD_ACTIVE="$GUARD_ACTIVE" GUARD_CONST_AUTH="$GUARD_CONST_AUTH" GUARD_CONST_SCOPE="$GUARD_CONST_SCOPE" GUARD_PREFLIGHT="$PREFLIGHT" "$PYBIN" -c "$PYSRC" 2>/tmp/.guard_py_err.$$)"
  RC=$?
fi
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
