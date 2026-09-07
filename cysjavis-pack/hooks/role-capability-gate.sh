#!/usr/bin/env bash
# PreToolUse hook (matcher 없음 = 전 도구): 역할-기반 능력 가드 (T4-4/T6-P3 · 0.14.31 WP-3 A).
#
# 두 역할군을 집행한다.
#   (a) reviewer-*/planner — 에이전트-내부 변형 도구(Edit/Write/NotebookEdit, write-shell Bash)를
#       **툴 실행 전** deny(producer≠evaluator: 리뷰어 산출물 자기수정 reward-hack 차단).
#   (b) cso*(0.14.31 신설) — CSO 본연(좌석 건강·자원 게이트·컨텍스트 사이클) 밖의 도구·명령을 deny.
#       정본은 `directives/CSO_DIRECTIVE.md` §1-1 이고 이 훅은 그 조항의 **집행부**다.
#   master/worker 는 통과(full-trust).
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
# ★CSO 는 python 부재에서 **fail-open**이다(reviewer 와 비대칭 — 의도적 · 0.14.31):
#   이 훅은 matcher 없이 **전 도구**에 붙는다. 인터프리터가 없다고 CSO 의 모든 도구 호출을
#   exit 2 로 막으면 그 좌석은 아무것도 못 하는 벽돌이 된다 — 그것은 봉인표 ②(무clear)·③(자가치유
#   전멸)의 실현이고, "막는 쪽으로만 틀린다"(오탐의 귀결은 보류·안내이지 좌석 사망이 아니다)의 위반이다.
#   python 부재는 **게이트 등록 이전 상태와 같음**(= 규율만 남음)이므로 강등이지 후퇴가 아니다.
#   stderr 1줄로 loud 하게 알린다. reviewer/planner 의 종전 fail-closed(exit 2)는 **그대로** 둔다.
#
# ★훅 실패의 하네스 의미(정직): PreToolUse 는 exit 0=진행 · exit 2=차단 · 그 밖=비차단 오류다.
#   즉 이 훅의 버그·예외·타임아웃은 **도구를 통과시킨다**(게이트가 조용히 꺼진다). 그래서 이 훅은
#   좌석을 죽이지 않지만, 그 대가로 "훅이 있으니 막힐 것"을 전제하면 안 된다 — 규율이 먼저다(§1-1).
#
# 신원 = cysd 권위: 자기 surface 역할은 `cys surface-role`(CYS_SURFACE_ID→데몬 roles 맵)로 읽는다.
#   self-declared가 아니라 claim_role/launch-agent가 신원검증 후 등록한 값. CYS_SURFACE_ID는
#   데몬이 PTY에 주입·상속하므로 에이전트가 임의 위조 불가(커널 peer-pid 신원의 파생).
#   `CYS_ROLE` env 는 **폴백 전용**이다(승계 후 stale — plan §8). 데몬 조회가 먼저다.
#
# ★60s 캐시의 권위 경계(0.14.31): 캐시는 **게이트 대상 역할(cso*/reviewer*/planner)에만 권위**다.
#   비대상 값(master 등)을 캐시에서 읽어 곧장 통과시키면 캐시 파일 오염이 게이트를 여는 경로가
#   된다 — 그래서 비대상 캐시 값은 조회 실패 시의 **폴백으로만** 쓴다(그 방향은 종전 동작 = 통과).
#   캐시 키 = surface + 소켓 + `<state>/boot-epoch`(데몬이 부트마다 bump). 데몬이 재시작하면 키가
#   바뀌어 캐시가 자동 무효화된다. boot-epoch 를 못 읽으면(Windows 파이프 등) 키에서 빠지고
#   TTL 60s 만 남는다 — 그 창의 stale 은 **과차단 방향**(옛 게이트 대상이 계속 게이트 대상)이다.
#
# Threat model (defensive-security-gate 9원칙): 비-악의 협력 에이전트의 *오작동* + reviewer의
#   직접 변형 시도 차단. 근본한계(명문화·은폐 금지): ① 인터프리터 우회(`bash -c "..."`·스크립트)·
#   git alias·셸 변수 확장은 Bash 토큰화 검사가 못 잡는다(block-dangerous-git와 동일 한계) →
#   reviewer 는 write-shell의 *대표* 위험 동사만 deny 하고, **CSO 는 반대로 allowlist**(허용 접두
#   밖은 전부 deny)라서 이 한계가 좁다. 단 CSO 경로도 명령치환(`$(…)`·백틱)·프로세스 치환은
#   토큰화로 안을 볼 수 없으므로 **문자 발견 즉시 deny** 한다(해석 불가 = 거부 방향).
#   ② cysd 인증(peer-pid)이 붕괴하면 role 조회가 오염될 수 있다(ADR: 소켓 동등노드 모델의 신뢰 뿌리).
#   kill-switch = 사람의 세션 리뷰.
#
# Design:
# - fail-CLOSED(reviewer/planner): python3 부재·JSON 파싱 실패·셸 파싱 불가·해석 불가 토큰 → BLOCK.
#   (단 role 조회 실패=역할 미상은 deny-by-default가 아니라 *통과* — 무역할 pane은 사람/일반
#    셸이라 일상 작업을 막지 않는다. reviewer-*/planner/cso*로 *확인된* surface만 차단한다.)
# - 비변형 도구(Read/Grep/Glob 등)도 이제 matcher 없이 도달한다 — CSO 도구 이름 deny 목록과
#   `tool_calls` 예산이 전 도구를 봐야 하기 때문이다. 목록 밖 도구는 통과한다.
# - reviewer의 tmp/로그 write는 과도차단 방지를 위해 허용(검증 대상 경로 한정 — propmap T6-P3 §5).
# - 검증: 내장 배터리 --self-test.

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(role-capability-gate)" >&2; exit 0; }

# ── 역할 해소(데몬 권위 우선 · 60s 캐시) ────────────────────────────────────────
# 캐시 파일 1줄 형식: `<epoch초> <역할>`. 키에 boot-epoch 를 넣어 데몬 재시작에 자동 무효화된다
# (감독자가 꺼졌거나 Windows 파이프면 그 성분이 빠지고 TTL 60s 만 남는다 — 정직한 강등).
capgate_slug() { printf '%s' "${1:-}" | tr -c 'A-Za-z0-9._-' '_' 2>/dev/null || printf 'x'; }

capgate_gated_role() {   # 게이트 대상 역할인가(캐시 권위 경계 · 파이썬 판정과 같은 접두 규칙)
  case "${1:-}" in
    cso*|reviewer*|planner|planner-*) return 0 ;;
  esac
  return 1
}

capgate_resolve_role() {
  CAPGATE_ROLE_SOURCE="none"
  CYS_SURFACE_ROLE_RESOLVED=""
  _cg_now="$(date +%s 2>/dev/null || printf '0')"
  case "$_cg_now" in ''|*[!0-9]*) _cg_now=0 ;; esac
  _cg_epoch=""
  if [ -n "${CYS_SOCKET:-}" ]; then
    _cg_epoch_file="$(dirname "$CYS_SOCKET" 2>/dev/null)/boot-epoch"
    [ -f "$_cg_epoch_file" ] && _cg_epoch="$(head -n1 "$_cg_epoch_file" 2>/dev/null)"
  fi
  CAPGATE_CACHE="${TMPDIR:-/tmp}/cys-capgate-role-$(capgate_slug "${CYS_SURFACE_ID:-none}")-$(capgate_slug "${CYS_SOCKET:-none}")-$(capgate_slug "$_cg_epoch")"
  _cg_cached=""; _cg_fresh=0
  if [ -f "$CAPGATE_CACHE" ]; then
    _cg_line="$(head -n1 "$CAPGATE_CACHE" 2>/dev/null)"
    _cg_ts="${_cg_line%% *}"
    _cg_cached="${_cg_line#* }"
    [ "$_cg_ts" = "$_cg_line" ] && _cg_cached=""      # 구형·손상 형식은 무시
    case "$_cg_ts" in ''|*[!0-9]*) _cg_ts=0 ;; esac
    # 미래 시각(시계 역행)은 신선이 아니다 — 그러면 캐시가 무기한 유효해진다.
    if [ "$_cg_now" -gt 0 ] && [ "$_cg_ts" -gt 0 ] && [ "$_cg_ts" -le "$_cg_now" ] \
       && [ $(( _cg_now - _cg_ts )) -lt 60 ]; then
      _cg_fresh=1
    fi
  fi
  # ①신선한 캐시가 **게이트 대상**이면 그것으로 확정(빠른 경로 — 게이트가 실제로 일하는 좌석).
  #   비대상 값은 이 빠른 경로를 타지 않는다: 캐시 오염이 게이트를 여는 문을 만들지 않는다.
  if [ "$_cg_fresh" = "1" ] && capgate_gated_role "$_cg_cached"; then
    CYS_SURFACE_ROLE_RESOLVED="$_cg_cached"; CAPGATE_ROLE_SOURCE="cache"
    return 0
  fi
  # ②데몬 권위 조회(데드라인 2s — `cys_timeout_run` 3단: timeout→gtimeout→CYS_PY 프로세스그룹).
  if command -v cys >/dev/null 2>&1; then
    _cg_out="$(cys_timeout_run 2 cys surface-role 2>/dev/null)"; _cg_rc=$?
    _cg_role="$(printf '%s' "$_cg_out" | head -n1 | tr -d '\r')"
    if [ "$_cg_rc" -eq 0 ] && [ -n "$_cg_role" ]; then
      CYS_SURFACE_ROLE_RESOLVED="$_cg_role"; CAPGATE_ROLE_SOURCE="daemon"
      if [ -n "${CYS_SURFACE_ID:-}" ]; then
        printf '%s %s\n' "$_cg_now" "$_cg_role" > "$CAPGATE_CACHE" 2>/dev/null || :
      fi
      return 0
    fi
    # ★권위 있는 **역할 없음**(rc 0 · 빈 줄)은 조회 실패가 아니다 — 옛 대상 캐시를 무효화한다.
    #   그러지 않으면 역할이 풀린 뒤에도 다음 조회 실패에서 옛 캐시가 부활한다(codex).
    if [ "$_cg_rc" -eq 0 ] && [ -z "$_cg_role" ]; then
      rm -f "$CAPGATE_CACHE" 2>/dev/null || :
      CAPGATE_ROLE_SOURCE="daemon-none"
      return 0
    fi
  fi
  # ③조회 실패 — 신선한 캐시 → env 순으로 폴백(env 는 폴백 전용 · plan §8).
  if [ "$_cg_fresh" = "1" ] && [ -n "$_cg_cached" ]; then
    CYS_SURFACE_ROLE_RESOLVED="$_cg_cached"; CAPGATE_ROLE_SOURCE="cache-fallback"
    return 0
  fi
  if [ -n "${CYS_SURFACE_ROLE:-}" ]; then
    CYS_SURFACE_ROLE_RESOLVED="$CYS_SURFACE_ROLE"; CAPGATE_ROLE_SOURCE="env-surface-role"
    return 0
  fi
  if [ -n "${CYS_ROLE:-}" ]; then
    CYS_SURFACE_ROLE_RESOLVED="$CYS_ROLE"; CAPGATE_ROLE_SOURCE="env-cys-role"
    return 0
  fi
  return 0
}

if [ "${1:-}" = "--self-test" ]; then
  [ -n "${CYS_PY:-}" ] || { echo "role-capability-gate: python 부재 — self-test 불가" >&2; exit 2; }
  export CAPGATE_SELF_TEST=1
else
  # ★훅 입력은 **파일**로 넘긴다 — 큰 Write 본문을 env 하나에 담으면 argv/env 크기 상한에 걸려
  #   64KB 판정 코드에 **도달하기 전에** 인터프리터 기동이 실패한다(큰 쓰기를 막는 검사가 큰
  #   쓰기 때문에 시작하지 못하는 구조). mktemp 이 없으면 종전 env 경로로 강등한다.
  CAPGATE_INPUT_FILE="$(mktemp "${TMPDIR:-/tmp}/cys-capgate-in.XXXXXX" 2>/dev/null)" || CAPGATE_INPUT_FILE=""
  if [ -n "$CAPGATE_INPUT_FILE" ]; then
    cat > "$CAPGATE_INPUT_FILE" || { rm -f "$CAPGATE_INPUT_FILE" 2>/dev/null; echo "role-capability-gate: cannot read stdin" >&2; exit 0; }
    export CAPGATE_INPUT_FILE
  else
    CAPGATE_INPUT="$(cat)" || { echo "role-capability-gate: cannot read stdin" >&2; exit 0; }
    export CAPGATE_INPUT
  fi
  # ★역할 해소는 **인터프리터 판정보다 먼저** 한다 — python 부재 분기가 역할을 알아야
  #   reviewer(fail-closed exit 2)와 CSO(fail-open 강등)를 가를 수 있다.
  if [ -z "${CYS_SURFACE_ROLE:-}" ] || [ -n "${CYS_SURFACE_ID:-}" ]; then
    capgate_resolve_role
    if [ -n "$CYS_SURFACE_ROLE_RESOLVED" ]; then
      CYS_SURFACE_ROLE="$CYS_SURFACE_ROLE_RESOLVED"
    elif [ "$CAPGATE_ROLE_SOURCE" = "daemon-none" ]; then
      CYS_SURFACE_ROLE=""
    fi
  fi
  export CYS_SURFACE_ROLE
  export CAPGATE_ROLE_SOURCE
  if [ -z "${CYS_PY:-}" ]; then
    [ -n "${CAPGATE_INPUT_FILE:-}" ] && rm -f "$CAPGATE_INPUT_FILE" 2>/dev/null
    case "${CYS_SURFACE_ROLE:-}" in
      reviewer*|planner|planner-*)
        echo "role-capability-gate: python missing — failing closed for reviewer/planner" >&2
        exit 2 ;;
      cso*)
        # ★비대칭(위 머리말 참조): CSO 는 강등(fail-open)이다 — 전 도구 차단은 좌석 사망이다.
        echo "role-capability-gate: python 부재 — CSO 능력 게이트 강등(집행 0 · CSO_DIRECTIVE §1-1 규율만 남는다)" >&2
        exit 0 ;;
      *) exit 0 ;;
    esac
  fi
fi

exec "$CYS_PY" - <<'PYEOF'
import hashlib, json, os, re, shlex, subprocess, sys, tempfile

# ── reviewer/planner 정책(종전 · 무변경) ─────────────────────────────────────
# 변형(mutation) 도구 — reviewer/planner에게 deny.
MUTATION_TOOLS = {"Edit", "Write", "NotebookEdit", "MultiEdit"}

# write-shell 대표 위험 동사(근본한계: 인터프리터 우회는 못 잡음 — block-dangerous-git와 동형).
WRITE_SHELL_CMDS = {
    "rm", "mv", "cp", "dd", "tee", "truncate", "install", "chmod", "chown",
    "ln", "mkdir", "rmdir", "touch", "sed",  # sed -i 등
}
# 패키지/빌드 설치자(상태 변형) — 대표만. git은 서브커맨드로 별도 판정(읽기 전용 다수).
WRITE_SHELL_INSTALLERS = {"npm", "pip", "pip3", "make", "apt", "brew"}
# cargo/go 는 하위 명령으로 가른다(0.14.31 완화 — 아래 계약을 정확히 읽어라).
WRITE_SHELL_BUILDERS = {"cargo", "go"}
# ★reviewer 검증 실행 완화(0.14.31 · 별 커밋) — **완화의 뜻을 정직하게 적는다**:
#   이것은 "쓰기 없음 보장" 이 **아니다**. `cargo test` 는 `target/` 산출물·캐시를 쓰고,
#   `build.rs`·proc-macro·테스트 본문은 사용자 권한으로 **임의 파일을 쓸 수 있다**(go 도 같다).
#   `--locked` 는 Cargo.lock 변경을 제한하는 옵션이지 파일시스템 샌드박스가 아니다.
#   여기서 집행하는 것은 **명령 수준의 변형 금지**뿐이다 — 즉 `publish`·`install`·`add`·`update`
#   처럼 그 명령 자체가 상태를 바꾸는 하위 명령을 막고, 검증용 하위 명령은 통과시킨다.
#   진짜 소스 불변성은 "검증 대상은 읽기 전용, 산출물·캐시·HOME 은 분리된 쓰기 공간" 인 실행
#   경계가 있어야 집행된다(Git Bash·기본 셸은 그런 격리가 아니다). 그 경계가 생기기 전까지
#   reviewer 의 빌드·테스트는 **신뢰 실행**이고, 사후 diff 는 탐지 수단이지 예방 장치가 아니다.
#   완화하지 않으면 reviewer 분기가 처음 활성화되는 이번 릴리스에서 `cargo test` 같은 **정당한
#   검증 명령이 전부 막힌다**(그 오탐의 귀결은 '리뷰 불가' = 산출자≠평가자 규율의 무력화다).
CARGO_VERIFY_SUBS = {"test", "build", "check", "clippy", "bench", "tree", "metadata", "doc", "nextest"}
GO_VERIFY_SUBS = {"test", "build", "vet", "list", "version", "env"}
BUILDER_VERIFY_SUBS = {"cargo": CARGO_VERIFY_SUBS, "go": GO_VERIFY_SUBS}
# 빌드 산출물을 **임의 경로로 내보내는** 옵션은 리다이렉트와 같은 부류다(대상이 허용 경로여야 한다).
BUILDER_OUT_OPTS = ("-o", "--out-dir", "--output", "--target-dir")


def builder_is_write(base, tokens, i):
    """cargo/go 세그먼트가 **명령 수준 변형**인가. 해석 불가·목록 밖 하위 명령은 True(거부 방향)."""
    subs = BUILDER_VERIFY_SUBS.get(base)
    if subs is None:
        return True
    sub = None
    n = len(tokens)
    j = i + 1
    while j < n:
        t = tokens[j]
        if is_separator(t) or _is_redirect_op(t):
            break
        # ★산출물 내보내기 옵션은 **세그먼트 전체**에서 찾는다 — `go build -o <path>` 처럼
        #   하위 명령 **뒤에** 오는 것이 보통이라, 하위 명령을 만나면 멈추는 스캔은 놓친다.
        if any(t == o or t.startswith(o + "=") for o in BUILDER_OUT_OPTS):
            val = t.split("=", 1)[1] if "=" in t else (tokens[j + 1] if j + 1 < n else "")
            if not path_is_allowed(val):
                return True              # 산출물을 검증 대상 트리로 내보낸다
            j += 2 if "=" not in t else 1
            continue
        if sub is None and not (t.startswith("-") or t.startswith("+")):
            sub = t
        j += 1
    if sub is None or sub not in subs:
        return True
    return False
# git 변형 서브커맨드(읽기 전용 status/log/diff/show/grep 등은 허용).
GIT_WRITE_SUBS = {"commit", "push", "add", "reset", "rebase", "merge", "checkout",
                  "restore", "clean", "stash", "rm", "mv", "apply", "cherry-pick",
                  "revert", "tag", "branch", "init", "am", "pull", "fetch"}
# ★값을 먹는 git **전역** 옵션(서브커맨드 앞) — 값을 건너뛰지 않으면 `git -C /repo reset --hard`
#   에서 `/repo` 를 서브커맨드로 오인해 **write 판정이 통째로 새어 나간다**(codex 실증 · 종전 결함).
GIT_GLOBAL_VALUE_OPTS = {"-C", "-c", "--git-dir", "--work-tree", "--namespace",
                         "--config-env", "--exec-path", "--super-prefix"}

WRAPPERS = {"command", "exec", "env", "sudo", "nohup", "time", "xargs"}

# 검증 대상 외 허용 경로 접두(과도차단 방지 — reviewer tmp/로그 write 허용).
ALLOW_PATH_PREFIXES = ("/tmp/", "/private/tmp/", "/var/tmp/", "/var/folders/")
ALLOW_PATH_SUBSTRS = ("/.cys/", "/logs/", "/log/", "/_round/")

# ★게이트 자신의 제어 상태 — **어떤 역할에게도** 쓰기 허용 경로가 아니다(codex P0).
#   예산 카운터가 CSO 쓰기 허용 영역(`~/.cys/state/`) 안에 있고 역할 캐시가 tmp 안에 있어서,
#   그 둘을 일반 규칙으로 두면 **집행 대상이 자기 집행 상태를 고쳐 게이트를 연다**.
GATE_STATE_DIRNAME = "capgate"
GATE_STATE_FILE_PREFIX = "cys-capgate-"

# ── CSO 정책(0.14.31 WP-3 A) — 정본은 directives/CSO_DIRECTIVE.md §1-1 ────────
# ★도구 이름 deny: 명령 접두로 표현되지 않으므로 **TTL 승인으로 열리지 않는다**(§1-1 말미).
#   `Task` 는 정본 목록의 `Agent` 와 같은 것의 다른 하네스 이름이다 — 둘 다 막는다(서브에이전트
#   스폰은 봉인표 ①(큐 폭주) 경로다 · 좁히는 방향이라 정본 위반이 아니다).
CSO_DENY_TOOLS = {
    "CronCreate", "CronDelete", "CronList", "Monitor", "TaskOutput",
    "Agent", "Task", "WebSearch", "WebFetch",
}
MCP_COMPUTER_USE_PREFIX = "mcp__computer-use__"
CSO_SKILL_ALLOW = {"hallucination-guard"}

# 도구 호출 예산(`tool_calls` — 턴이 아니라 **도구 호출 수**).
BUDGET_WARN = 1500
BUDGET_DENY = 2000

# CSO_TODO 상한(바이트). **초과이고 증가**일 때만 deny(축소는 언제나 허용).
CSO_TODO_CAP = 64 * 1024
CSO_STATE_BASENAMES = ("CSO_", "SESSION_STATE")

# `cys` 조회/사이클 동사(별도 인자 계약이 없는 것).
CSO_CYS_VERBS = {
    "status", "list", "ps", "identify", "read-screen", "gate-check",
    "reap-surface", "set-status", "surface-role", "todo-path",
}
CSO_CYS_SUBVERBS = {
    "queue": {"list"},
    # `feed push` = §1-2 ② 오너 채널(master 무응답 교착의 출구). 이 접두의 등재는 능력 게이트
    # 릴리스의 **수용 조건**이다(CSO_DIRECTIVE §1-2 ⑤ 이 조문을 명시한다).
    "feed": {"list", "push"},
    "schedule": {"list"},
    # `approval check` = §1-1 이 집행 **전**에 요구하는 확인 절차(읽기 전용). `sign` 은 master
    # 발신만 허용되므로(§1-2 ③-1) CSO 에게는 deny 다.
    "approval": {"check"},
}
CSO_CYS_TTL_VERBS = {"kill", "close-surface", "pause", "resume", "tombstone", "launch-agent"}
CSO_CYS_DENY_VERBS = {"events"}          # 종결 없는 스트림 — TTL 승인 대상도 아니다(§1-1)
# 허용 동사라도 **효과가 다른 옵션**은 따로 막는다(접두 허용 ≠ 인자 허용 · codex P0).
CSO_CYS_OPT_DENY = {
    "cycle-agent": ("--force-no-verify", "--clear-cmd", "--surface"),
    "send": ("--clear-first", "--surface"),
}

CSO_PY_INTERPRETERS = {"python3", "python", "py", "python3.exe", "python.exe", "py.exe"}
# 코드를 인자로 받는 실행 모드는 스크립트 경계를 무너뜨린다.
CSO_PY_DENY_FLAGS = ("-c", "-m", "-i", "--command")
CSO_PY_OK_FLAG_RE = re.compile(r"^-(?:[BEsSuOtvbdqIP]+|3(?:\.\d+)?|X$)")
# 스크립트별 허용 서브동사(None = 인자 무제한). **파일 하나를 통째로 허용하면 관측과 상태
# 변경을 구별하지 못한다**(codex: autopilot `reset` 은 lease 를 지우고 `bootstrap-verifier` 는
# pane 을 만든다).
CSO_PY_TOOLS = {
    "javis_cycle_autopilot.py": {"tick", "status", "audit", "self-test"},
    "javis_orchestra.py": {"check", "round-status", "gate-status", "next-action",
                           "channel-health", "silent-failure-catalog", "self-test"},
    "javis_resource_gate.py": None,
    "javis_reap_exited.py": None,
    "javis_report_gate.py": None,
    "javis_state_snapshot.py": None,
    "javis_mission.py": None,
    "javis_preflight.py": None,      # 아래 CSO_PY_ARG_DENY 가 변이 플래그를 막는다
}
CSO_PY_ARG_DENY = {
    "javis_preflight.py": ("--fix", "--seed-trust", "--allow-irreversible"),
}
CSO_ESSENTIAL_PY_TOOLS = {"javis_cycle_autopilot.py"}

# 읽기 전용 셸(인자 규칙이 없는 것들). `sed` 는 **정본 §10-1 에서 의도적으로 뺐다** —
# `-n` 은 자동출력 억제일 뿐 쓰기 금지가 아니고(`sed -n 'w /tmp/x'`·`-i` 동반), 스크립트 안의
# 쓰기 명령을 토큰 검사로 안전하게 가려낼 수 없다. 좁히는 방향이므로 §1-1 '좁은 쪽이 이긴다'.
CSO_RO_CMDS = {"ps", "grep", "rg", "cat", "head", "tail", "wc", "ls", "stat",
               "date", "df", "du", "uptime", "shasum", "sha256sum", "cksum", "md5sum"}
# 증거는 텍스트다(§1-1 스크린샷 정책) — sha256 계산기를 허용 목록에 넣는다(정본 §10-1 누락분).
CSO_DIGEST_CMDS = {"shasum", "sha256sum", "cksum", "md5sum"}
CSO_GIT_READ_SUBS = {"status", "log", "diff", "show"}
# git 조회 하위 명령이 **셸 리다이렉트 없이** 파일을 쓰거나 외부 프로그램을 돌리는 옵션들.
CSO_GIT_OPT_DENY = ("--output", "-o", "--ext-diff", "--exec", "--textconv", "--pager",
                    "--upload-pack", "-c", "-C", "--git-dir", "--work-tree")
CSO_TAIL_FOLLOW = ("-f", "-F", "--follow", "--retry")
CSO_RG_OPT_DENY = ("--pre", "--hostname-bin", "--search-zip", "-z")
CSO_SQLITE_FLAG_DENY = ("-cmd", "-init", "-A", "-archive", "-zip")

# 명령치환·프로세스치환 — 토큰화로 안을 볼 수 없다(해석 불가 = 거부 방향).
#   `"보고: $(cys events)"` 처럼 **인용 안에서도** 실행되므로 문자 발견 즉시 deny 한다.
#   대가(정직): `"시각: $(date)"` 같은 무해한 치환도 막힌다 — 값을 먼저 구해 인자로 넣어야 한다.
CSO_SUBST_MARKERS = ("$(", "`", "<(", ">(")
# 리다이렉트 대상으로 항상 허용되는 장치(파일 쓰기가 아니다).
NULL_SINKS = {"/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty", "nul", "NUL"}


# ── 공용 술어 ────────────────────────────────────────────────────────────────
def is_reviewer_or_planner(role):
    role = (role or "").strip()
    return role.startswith("reviewer") or role == "planner" or role.startswith("planner-")


def is_cso(role):
    """`cso*` 접두 — session-start.sh:161 `cso*)` 와 **글자 그대로 같은** 규칙(재발명 금지).

    `pack::role_directive_path` 의 접두 의미론 미러이기도 하다: cso·cso-1·cso-dept 전부 CSO 다.
    """
    return (role or "").strip().startswith("cso")


def is_separator(tok):
    return bool(tok) and set(tok) <= set(";&|()")


def _is_gate_state_path(p):
    """게이트 제어 상태(예산 카운터·역할 캐시)인가 — 어떤 역할에게도 쓰기 대상이 아니다."""
    ap = (os.path.abspath(os.path.expanduser(str(p or "")))).replace("\\", "/")
    base = ap.rsplit("/", 1)[-1]
    if base.startswith(GATE_STATE_FILE_PREFIX):
        return True
    return ("/" + GATE_STATE_DIRNAME + "/") in ap + "/"


def path_is_allowed(p):
    if not p:
        return False
    if _is_gate_state_path(p):
        return False   # ★게이트 제어 상태는 예외 없음(자기 집행 상태 조작 봉인)
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
    """순수 출력 리다이렉트 연산자(>, >>, 2>, &>, >&)면 True. 입력(<)은 제외."""
    if not tok:
        return False
    t = tok
    if t in (">", ">>", "&>", ">|", ">&"):
        return True
    if t.endswith(">") and "<" not in t:  # '2>', '1>>' 등
        return True
    if t.endswith(">&") and "<" not in t:
        return True
    return False


def split_unquoted_newlines(command):
    """인용 **밖**의 개행만 `;` 로 바꾼다.

    ★왜 필요한가(codex 실증): 개행을 공백으로 흘리면 `cys status\\ncys kill 123` 이
      `['cys','status','cys','kill','123']` 한 세그먼트가 되어 뒤 명령이 **인자로 위장**된다.
      반대로 개행을 통째로 `;` 로 바꾸면(reviewer 경로의 종전 방식) 인용된 여러 줄 **보고 본문**
      까지 명령 경계로 변형된다 — `cys send --queued --to master "1줄\\n2줄"` 이 그렇다.
      그래서 인용 상태를 세면서 **밖의 개행만** 경계로 만든다.
    """
    out = []
    quote = None
    esc = False
    for ch in command:
        if esc:
            out.append(ch)
            esc = False
            continue
        if quote is None and ch == "\\":
            out.append(ch)
            esc = True
            continue
        if quote is None and ch in ("'", '"'):
            quote = ch
            out.append(ch)
            continue
        if quote is not None:
            if ch == quote:
                quote = None
            elif quote == '"' and ch == "\\":
                out.append(ch)
                esc = True
                continue
            out.append(ch)
            continue
        if ch in ("\n", "\r"):
            out.append(";")
            continue
        out.append(ch)
    return "".join(out)


def _tokenize(command):
    """셸 토큰 목록. 파싱 실패는 None(호출측이 fail-closed 로 읽는다)."""
    for zw in ("\u200b", "\u200c", "\u200d", "\ufeff"):
        command = command.replace(zw, "")
    try:
        lex = shlex.shlex(command, posix=True, punctuation_chars=True)
        lex.whitespace_split = True
        return list(lex)
    except ValueError:
        return None


def bash_has_write(command):
    """Bash 명령에 write-shell 동사 또는 (비-허용경로) 출력 리다이렉트가 있으면 True(=변형).
    해석불가=True(fail-closed). 리다이렉트 대상이 허용경로(tmp/log)면 그 리다이렉트는 무시."""
    tokens = _tokenize(command.replace("\n", " ; ").replace("\r", " "))
    if tokens is None:
        return True  # 따옴표 불일치 등 — fail-closed(변형으로 간주)

    cmd_pos = True
    i = 0
    n = len(tokens)
    while i < n:
        tok = tokens[i]
        # 출력 리다이렉트: 대상이 허용경로면 통과, 아니면 변형.
        if _is_redirect_op(tok):
            target = tokens[i + 1] if i + 1 < n else ""
            if target not in NULL_SINKS and not target.isdigit() and not path_is_allowed(target):
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
            if base in WRITE_SHELL_BUILDERS:
                if builder_is_write(base, tokens, i):
                    return True
                cmd_pos = False
                i += 1
                continue
            if base in WRITE_SHELL_CMDS or base in WRITE_SHELL_INSTALLERS:
                return True
            if base == "git":
                # ★값을 먹는 전역 옵션은 **값까지** 건너뛴다 — 그러지 않으면 `git -C /repo reset`
                #   에서 `/repo` 를 서브커맨드로 보고 검사를 끝낸다(종전 결함 · codex 실증).
                j = i + 1
                while j < n:
                    t = tokens[j]
                    if is_separator(t) or _is_redirect_op(t):
                        break
                    if t in GIT_GLOBAL_VALUE_OPTS:
                        j += 2
                        continue
                    if t.startswith("-"):
                        j += 1
                        continue
                    if t in GIT_WRITE_SUBS:
                        return True
                    break  # 첫 서브커맨드만 본다(읽기 전용이면 통과)
            cmd_pos = False
        i += 1
    return False


# ── CSO 문맥 · 경로 ──────────────────────────────────────────────────────────
def _norm(p):
    """비교용 정규화: 절대경로 + 백슬래시→슬래시. Windows 드라이브 문자는 대문자로 접는다.

    ★한계(정직): symlink·junction 을 따라가지 않는다(realpath 는 없는 파일에서 의미가 흔들리고
      훅 핫패스에 stat 왕복을 더한다). 같은 uid 로 임의 코드를 돌릴 수 있는 상대에게 파일 기반
      제어 상태의 무결성을 보장하지 못한다는 근본한계와 같은 층의 한계다.
    """
    if p is None:
        return ""
    try:
        ap = os.path.abspath(os.path.expanduser(str(p)))
    except (TypeError, ValueError):
        return ""
    ap = ap.replace("\\", "/")
    if len(ap) > 1 and ap[1] == ":":
        ap = ap[0].upper() + ap[1:]
    return ap


def _under(path, root):
    """path 가 root **아래**(또는 root 자신)인가. 접두 문자열 비교의 형제 디렉터리 오판
    (`/a/pack-dept-1` 이 `/a/pack` 접두를 만족)을 경계 슬래시로 막는다."""
    p, r = _norm(path), _norm(root)
    if not p or not r:
        return False
    r = r.rstrip("/")
    return p == r or p.startswith(r + "/")


def pack_dir(env=None):
    """src/pack.rs pack_dir() 4단 폴백 미러(javis_guard_register._pack_dir 와 동일 순서)."""
    env = os.environ if env is None else env
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = env.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys", "pack")


def state_root(env=None):
    """상태 파일 루트 — `CYS_STATE_DIR` 우선, 없으면 `~/.cys/state`(javis_lane·javis_bootstrap 관례)."""
    env = os.environ if env is None else env
    v = env.get("CYS_STATE_DIR", "")
    return v or os.path.join(os.path.expanduser("~"), ".cys", "state")


class Ctx(object):
    """판정 문맥 — 순수 판정기에 환경을 주입한다(검체가 같은 판정기를 다른 세계로 부를 수 있게)."""

    def __init__(self, pack=None, state=None, home=None, tool_calls=None,
                 approver=None, reader=None, tempdir=None, background=False):
        env = os.environ
        self.pack = pack if pack is not None else pack_dir(env)
        self.state = state if state is not None else state_root(env)
        self.home = home if home is not None else os.path.expanduser("~")
        self.tool_calls = tool_calls          # None = 계수 불가 → 예산 판정 없음
        self.approver = approver              # (command) -> bool
        self.reader = reader                  # (path) -> str|None (없으면 실제 파일)
        self.tempdir = tempdir if tempdir is not None else tempfile.gettempdir()
        self.background = bool(background)

    def read_text(self, path):
        if self.reader is not None:
            return self.reader(path)
        try:
            with open(path, "r", encoding="utf-8", errors="surrogateescape") as f:
                return f.read()
        except (OSError, ValueError):
            return None


def cso_write_roots(ctx):
    """CSO 가 써도 되는 뿌리(§1-1 '허용 경로'). 자기 레인 팩 `round/` 가 첫째다 —
    타 레인 팩(`pack-dept-*`)은 이 뿌리 **밖**이라 자동으로 deny 된다."""
    return (
        os.path.join(ctx.pack, "round"),
        os.path.join(ctx.home, "Desktop", "CYSjavis", "cso"),
        os.path.join(ctx.home, ".cys", "state"),
        ctx.state,
        ctx.tempdir,
        "/tmp", "/private/tmp", "/var/tmp", "/var/folders",
    )


def is_cso_state_file(path):
    """자기 역할 소유 상태 파일인가(`CSO_*` · `SESSION_STATE*`)."""
    base = os.path.basename(_norm(path))
    return any(base.startswith(n) for n in CSO_STATE_BASENAMES)


def cso_path_allowed(path, ctx):
    """(ok, reason). ★'자기 레인 팩'과 '자기 역할 소유 파일'은 다른 경계다(codex).

    팩 `round/` 안에서는 **자기 소유 파일만** 허용한다 — 같은 팩의 `MASTER_TODO.md` 나 다른
    CSO 변형의 파일을 쓰는 것은 타 레인 편집과 같은 부류다.
    """
    if not path:
        return False, "대상 경로 없음"
    if _is_gate_state_path(path):
        return False, "게이트 제어 상태(예산 카운터·역할 캐시)는 쓰기 대상이 아니다"
    round_root = os.path.join(ctx.pack, "round")
    if _under(path, round_root):
        if is_cso_state_file(path):
            return True, "자기 레인 팩 round/ 의 자기 소유 상태 파일"
        return False, ("자기 레인 팩 round/ 이지만 자기 소유 파일이 아니다"
                       "(허용 = CSO_* · SESSION_STATE*)")
    for r in cso_write_roots(ctx)[1:]:
        if _under(path, r):
            return True, "허용 경로(%s)" % r
    return False, "허용 경로 밖(자기 TODO·~/Desktop/CYSjavis/cso/·~/.cys/state/·scratchpad)"


def _blen(s):
    if s is None:
        return 0
    if isinstance(s, bytes):
        return len(s)
    return len(str(s).encode("utf-8", "surrogatepass"))


def _apply_edit(text, old, new, replace_all):
    if not isinstance(old, str) or not isinstance(new, str) or not old:
        return None
    if replace_all:
        return text.replace(old, new)
    idx = text.find(old)
    if idx < 0:
        return None
    return text[:idx] + new + text[idx + len(old):]


def todo_cap_verdict(tool, ti, path, ctx):
    """(deny, detail). **예상 결과 바이트가 상한 초과 이고 증가**일 때만 deny.

    축소·동률은 언제나 허용한다 — 상한의 목적은 파일이 계속 부푸는 것을 막는 것이지 이미 큰
    파일을 편집 불가로 만들어 로그 이관조차 못 하게 하는 것이 아니다(그 오탐은 봉인표 ②·③
    방향이다). 현재 내용을 못 읽으면 판정하지 않는다(**결측은 값이 아니다** · 크기 0 으로
    접지 않는다 — 그러면 없는 증가를 만들어낸다).
    ★`replace_all`·MultiEdit 누적은 실제 치환을 적용해 잰다(1바이트 치환 × 1만 회를 1바이트
      증가로 세던 단순식은 codex 반례에서 10KB 를 놓친다).
    """
    if not is_cso_state_file(path):
        return False, ""
    cur_text = ctx.read_text(path)
    if tool == "Write":
        cur = 0 if cur_text is None else _blen(cur_text)
        projected = _blen(ti.get("content"))
    elif tool in ("Edit", "MultiEdit"):
        if cur_text is None:
            return False, ""            # 판독 불가 → 무판정
        cur = _blen(cur_text)
        text = cur_text
        edits = ti.get("edits") if tool == "MultiEdit" else [ti]
        if not isinstance(edits, list):
            return False, ""
        for e in edits:
            if not isinstance(e, dict):
                return False, ""
            nxt = _apply_edit(text, e.get("old_string"), e.get("new_string"),
                              bool(e.get("replace_all")))
            if nxt is None:
                return False, ""        # 도구 동작을 재현 못 함 → 무판정
            text = nxt
        projected = _blen(text)
    else:
        return False, ""
    if projected > CSO_TODO_CAP and projected > cur:
        return True, "예상 %dB > 상한 %dB 이고 증가(현재 %dB)" % (projected, CSO_TODO_CAP, cur)
    return False, ""


# ── CSO Bash 접두 판정(allowlist) ────────────────────────────────────────────
def cso_split(command):
    """(segments, redirect_targets, err). err 가 있으면 deny 사유다.

    허용 구두점은 세그먼트 경계(`;` `&&` `||` `|` `(` `)`)와 출력 리다이렉트뿐이다.
    나머지 구두점(`&` 단독=백그라운드 · `<` `<<` `<<<` `<>` `<(` `>(`)은 **모르는 문법**이므로
    거부한다 — 아는 것만 통과시킨다(allowlist 의 뜻).
    """
    for m in CSO_SUBST_MARKERS:
        if m in command:
            return None, None, ("명령 치환·프로세스 치환(%s)은 게이트가 안을 볼 수 없다 — "
                                "값을 먼저 구해 인자로 넣어라" % m)
    tokens = _tokenize(split_unquoted_newlines(command))
    if tokens is None:
        return None, None, "셸 파싱 불가(따옴표 불일치 등) — 해석 불가는 거부다"
    segs, cur, redirects = [], [], []
    i, n = 0, len(tokens)
    while i < n:
        tok = tokens[i]
        if _is_redirect_op(tok):
            redirects.append(tokens[i + 1] if i + 1 < n else "")
            i += 2
            continue
        if is_separator(tok):
            if tok in (";", "&&", "||", "|"):
                if cur:
                    segs.append(cur)
                cur = []
                i += 1
                continue
            if tok in ("(", ")"):
                if cur:
                    segs.append(cur)
                cur = []
                i += 1
                continue
            if tok == "&":
                return None, None, "백그라운드 실행(`&`)은 CSO 경계 밖이다(종결 없는 관측)"
            return None, None, "해석 불가 셸 연산자 `%s` — 아는 문법만 통과한다" % tok
        if set(tok) & set("<") and tok.strip("<") == "":
            return None, None, "입력 리다이렉트·here-doc(`%s`)은 CSO 경계 밖이다" % tok
        cur.append(tok)
        i += 1
    if cur:
        segs.append(cur)
    return segs, redirects, None


def _resolve_pack_token(tok, ctx):
    """지침이 쓰는 **유한한** 변수 표기만 결정론으로 푼다(셸 확장 실행 0).

    허용 표기: `$CYS_PACK_DIR` · `${CYS_PACK_DIR}` · `${CYS_PACK_DIR:-$HOME/.cys/pack}` ·
               `$HOME` · `${HOME}` · `~`. 그 밖의 `$` 가 남으면 해소 불가(None)다.
    """
    s = str(tok or "")
    s = s.replace("${CYS_PACK_DIR:-$HOME/.cys/pack}", ctx.pack)
    s = s.replace("${CYS_PACK_DIR:-${HOME}/.cys/pack}", ctx.pack)
    s = s.replace("${CYS_PACK_DIR}", ctx.pack).replace("$CYS_PACK_DIR", ctx.pack)
    s = s.replace("${HOME}", ctx.home).replace("$HOME", ctx.home)
    if s.startswith("~/") or s == "~":
        s = ctx.home + s[1:]
    if "$" in s:
        return None
    return s


def _py_segment_verdict(tokens, ctx):
    """(ok, reason, essential) — `python3 <pack>/bin/javis_*.py <sub>` 형태."""
    script = None
    idx = None
    for j, t in enumerate(tokens[1:], start=1):
        if t.startswith("-"):
            if t in CSO_PY_DENY_FLAGS or t.startswith("-c") or t.startswith("-m"):
                return False, "python 실행 모드 `%s` 는 스크립트 경계를 무너뜨린다" % t, False
            if CSO_PY_OK_FLAG_RE.match(t):
                continue
            return False, "알 수 없는 python 옵션 `%s`" % t, False
        script, idx = t, j
        break
    if script is None:
        return False, "실행할 스크립트가 없다(대화형 python 은 경계 밖)", False
    resolved = _resolve_pack_token(script, ctx)
    if resolved is None:
        return False, ("스크립트 경로의 변수를 해소할 수 없다 — 허용 표기는 "
                       "`${CYS_PACK_DIR:-$HOME/.cys/pack}`·`$CYS_PACK_DIR`·`$HOME`·`~` 뿐이다"), False
    base = os.path.basename(resolved.replace("\\", "/"))
    if base not in CSO_PY_TOOLS:
        return False, "판정 도구 목록 밖 스크립트: %s" % base, False
    # ★basename 신뢰 금지(codex P0): 허용된 scratchpad 에 같은 이름을 써 두고 부르는 경로를
    #   막는다 — 실제 경로가 **설치 팩의 bin/** 아래여야 한다.
    bin_root = os.path.join(ctx.pack, "bin")
    if not _under(resolved, bin_root):
        return False, ("판정 도구는 설치 팩 `%s` 아래에서만 실행한다(같은 이름의 사본은 "
                       "판정 도구가 아니다): %s" % (bin_root, resolved)), False
    args = tokens[idx + 1:]
    for bad in CSO_PY_ARG_DENY.get(base, ()):  # 변이 플래그
        if bad in args or any(a.startswith(bad + "=") for a in args):
            return False, "`%s %s` 는 변이 경로다 — 조회 모드만 허용" % (base, bad), False
    allowed_subs = CSO_PY_TOOLS[base]
    sub = next((a for a in args if not a.startswith("-")), None)
    if allowed_subs is not None:
        if sub is None or sub not in allowed_subs:
            return False, ("`%s` 하위 명령 `%s` 는 관측이 아니다 — 허용: %s"
                           % (base, sub, "|".join(sorted(allowed_subs)))), False
    essential = base in CSO_ESSENTIAL_PY_TOOLS
    return True, "판정 도구 %s %s" % (base, sub or ""), essential


def _cys_segment_verdict(tokens, ctx, raw_command):
    """(ok, reason, essential) — `cys <verb> …`."""
    verb = next((t for t in tokens[1:] if not t.startswith("-")), None)
    if verb is None:
        return False, "`cys` 동사 없음", False
    args = tokens[1:]
    if verb in CSO_CYS_DENY_VERBS:
        return False, ("`cys %s` 는 종결 없는 스트림이라 어떤 플래그로도 접두 밖이다"
                       "(TTL 승인 대상도 아니다 — 구독에는 예외가 없다)" % verb), False
    for bad in CSO_CYS_OPT_DENY.get(verb, ()):
        if bad in args or any(a.startswith(bad + "=") for a in args):
            return False, "`cys %s %s` 는 효과가 다른 옵션이다(접두 허용 ≠ 인자 허용)" % (verb, bad), False
    if verb == "send":
        tos = []
        for j, a in enumerate(args):
            if a == "--to":
                tos.append(args[j + 1] if j + 1 < len(args) else "")
            elif a.startswith("--to="):
                tos.append(a[5:])
        if not tos:
            return False, "`cys send` 는 수신자를 명시해야 한다(`--to master`)", False
        if any(t != "master" for t in tos):
            return False, ("`cys send` 의 수신자는 master 뿐이다(경계까지 대조 — "
                           "`--to master-shadow` 는 master 가 아니다): %s" % ", ".join(tos)), False
        return True, "cys send --to master", True
    if verb == "cycle-agent":
        return True, "cys cycle-agent(사이클 필수 도구)", True
    if verb in CSO_CYS_VERBS:
        return True, "cys %s" % verb, verb in ("status", "list", "identify", "set-status",
                                               "read-screen")
    if verb in CSO_CYS_SUBVERBS:
        rest = args[args.index(verb) + 1:]
        sub = next((a for a in rest if not a.startswith("-")), None)
        if sub in CSO_CYS_SUBVERBS[verb]:
            # 관측(queue/feed list)과 오너 채널 상신(feed push)은 사이클 절차의 일부다.
            return True, "cys %s %s" % (verb, sub), True
        return False, ("`cys %s %s` 는 허용 서브동사가 아니다 — 허용: %s"
                       % (verb, sub, "|".join(sorted(CSO_CYS_SUBVERBS[verb])))), False
    if verb in CSO_CYS_TTL_VERBS:
        if approval_allows(raw_command, ctx):
            return True, "TTL 승인 확인됨(`approval check --require-ttl` exit 0)", False
        return False, ("`cys %s` 는 master 의 **시간 한정** 승인이 필요하다 — "
                       "`cys approval sign --prefix \"<정확 명령>\" --ttl <초>` 발급 뒤 "
                       "`cys approval check --prefix \"<정확 명령>\" --require-ttl` 통과 후에만. "
                       "플래그 부재(구 바이너리)는 승인됨이 아니라 **미승인**이다" % verb), False
    return False, "`cys %s` 는 CSO 허용 접두 목록 밖이다" % verb, False


def _ro_segment_verdict(tokens, ctx):
    """(ok, reason, essential) — 읽기 전용 셸."""
    base = os.path.basename(tokens[0].replace("\\", "/"))
    args = tokens[1:]
    if base == "git":
        for a in args:
            if any(a == d or a.startswith(d + "=") for d in CSO_GIT_OPT_DENY):
                return False, "`git %s` 는 파일 출력·외부 실행 경로다" % a, False
        sub = args[0] if args else None
        if sub not in CSO_GIT_READ_SUBS:
            return False, ("git 은 조회 서브커맨드가 **바로 뒤에** 와야 한다"
                           "(허용: %s · 전역 옵션 금지)" % "|".join(sorted(CSO_GIT_READ_SUBS))), False
        return True, "git %s" % sub, False
    if base == "sqlite3":
        for a in args:
            if any(a == d or a.startswith(d + "=") for d in CSO_SQLITE_FLAG_DENY):
                return False, "`sqlite3 %s` 는 임의 SQL 실행 경로다" % a, False
        if "-readonly" not in args:
            return False, ("sqlite3 은 `-readonly` 를 붙여야 한다 — SQL 문자열이 select 로 "
                           "시작해도 뒤 문장은 쓰기일 수 있다(엔진이 거부하게 만든다)"), False
        return True, "sqlite3 -readonly", False
    if base == "pmset":
        if "-g" not in args:
            return False, "`pmset` 은 `-g`(조회)만 허용한다", False
        return True, "pmset -g", False
    if base in ("tail", "head"):
        for a in args:
            if a in CSO_TAIL_FOLLOW or a.startswith("--follow"):
                return False, "`%s %s` 는 종결 없는 관측이다(스트림 금지)" % (base, a), False
    if base == "rg":
        for a in args:
            if any(a == d or a.startswith(d + "=") for d in CSO_RG_OPT_DENY):
                return False, "`rg %s` 는 외부 프로그램 실행 경로다" % a, False
    if base not in CSO_RO_CMDS:
        return False, "`%s` 는 CSO 허용 접두 목록 밖이다" % base, False
    essential = False
    if base in CSO_DIGEST_CMDS or base in ("cat", "head", "tail", "stat", "wc"):
        # 경로처럼 생긴 인자만 본다 — `shasum -a 256 <path>` 의 `256` 은 옵션 값이지 대상이 아니다.
        paths = [a for a in args
                 if not a.startswith("-") and ("/" in a or "\\" in a)]
        if paths and all(is_cso_state_file(_resolve_pack_token(q, ctx) or q) for q in paths):
            essential = True     # 저장 재독·checksum 대조는 사이클 절차의 일부다
    return True, base, essential


def cso_bash_verdict(command, ti, ctx):
    """(deny, reason, essential) — 전 세그먼트가 허용 접두여야 통과."""
    if not isinstance(command, str) or not command.strip():
        return False, "빈 명령", False
    if ctx.background or ti.get("run_in_background"):
        return True, ("백그라운드 실행(`run_in_background`)은 CSO 경계 밖이다 — "
                      "종결 없는 관측을 도구 필드로 요청하는 경로다"), False
    segs, redirects, err = cso_split(command)
    if err:
        return True, err, False
    for target in redirects:
        if target in NULL_SINKS or target.isdigit():
            continue
        ok, why = cso_path_allowed(target, ctx)
        if not ok:
            return True, "출력 리다이렉트 대상 %r: %s" % (target, why), False
        if is_cso_state_file(target):
            return True, ("상태 파일(%s)에 셸 리다이렉트로 쓰면 64KB 상한 검사를 건너뛴다 — "
                          "Write/Edit 도구를 써라" % target), False
    if not segs:
        return False, "실행 세그먼트 없음", False
    essential = True
    reasons = []
    for seg in segs:
        head = os.path.basename(str(seg[0]).replace("\\", "/"))
        if head in ("cys", "cys.exe"):
            ok, why, ess = _cys_segment_verdict(seg, ctx, command)
        elif head in CSO_PY_INTERPRETERS:
            ok, why, ess = _py_segment_verdict(seg, ctx)
        else:
            ok, why, ess = _ro_segment_verdict(seg, ctx)
        if not ok:
            return True, why, False
        essential = essential and ess
        reasons.append(why)
    return False, " · ".join(reasons), essential


# ── TTL 승인(§1-1 '예외는 우회가 아니라 승인') ────────────────────────────────
def approval_allows(command, ctx=None, timeout=5.0):
    """`cys approval check --prefix "<정확 명령>" --require-ttl` 이 exit 0 일 때만 True.

    구 바이너리는 `--prefix`·`--require-ttl` 을 모르므로 clap 이 rc≠0 을 내고 → **deny**
    (CONTRACTS §B-3 · 안전 방향). '플래그 부재 = 미승인' 이지 '승인됨' 이 아니다.
    ★한계(정직): 이것은 **검사 시점의 유효성**이지 실행 시점의 원자 승인·소비가 아니다.
      검사와 실행 사이에 증표가 만료되거나 같은 증표로 병렬 호출이 통과할 수 있다 —
      위험 행위의 최종 방어는 데몬 실행 지점의 상태·승인 동시 검증이다.
    """
    if ctx is not None and ctx.approver is not None:
        return bool(ctx.approver(command))
    cys = os.environ.get("CYS_BIN") or _which("cys")
    if not cys:
        return False
    try:
        r = subprocess.run([cys, "approval", "check", "--prefix", command, "--require-ttl"],
                           stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=timeout)
    except Exception:
        return False
    return r.returncode == 0


def _which(name):
    for d in (os.environ.get("PATH") or "").split(os.pathsep):
        if not d:
            continue
        cand = os.path.join(d, name)
        for c in (cand, cand + ".exe", cand + ".cmd"):
            if os.path.isfile(c) and os.access(c, os.X_OK):
                return c
    return None


# ── 예산 카운터(`tool_calls`) ─────────────────────────────────────────────────
def session_key(sid):
    """원문 session_id → 파일명 1컴포넌트(sha256 앞 32hex).

    ★왜 해시인가(codex): 단순 sanitize 는 `a/b` 와 `a?b` 를 같은 이름으로 접어 **서로 다른
      세션이 예산을 공유**하게 만들고, Windows 예약 이름(`CON`·`NUL`)·길이 상한도 남는다.
      해시는 충돌·주입·예약어를 한 번에 없앤다. 타입 오류·빈 값은 `None`(판정 불능)이며
      `none` 같은 공용 키로 합치지 않는다 — 결측은 값이 아니다.
    """
    if not isinstance(sid, str):
        return None
    s = sid.strip()
    if not s:
        return None
    return hashlib.sha256(s.encode("utf-8", "surrogatepass")).hexdigest()[:32]


def bump_tool_calls(key, root):
    """세션 카운터 +1 후 새 값. 실패는 None(계수 불가 = 예산 판정 없음).

    ★동시성(정직 · codex 지적 인정): 읽기~쓰기 사이에 다른 호출이 끼면 **과소 계수**가 된다
      (원자 교체는 원자 증가가 아니다). 잠금을 두지 않은 이유는 ⓐWindows Git Bash 에 `flock`
      이 없고 ⓑ훅 핫패스의 잠금 대기가 **전 도구 호출**을 느리게 하며 ⓒ남은 잠금 파일이 좌석을
      영구 차단하는 실패 양식을 만들기 때문이다. 과소 계수의 방향은 **통과**이고, 이 카운터의
      목적은 정밀 계량이 아니라 사이클 신호다. 저장 실패도 통과 방향으로 접는다(카운터 고장이
      필수 도구를 막지 않는다 — 0 초기화와 저장 장애를 같게 취급하지 않는다는 뜻이다).
    """
    if not key:
        return None
    d = os.path.join(root, "capgate")
    p = os.path.join(d, key + ".count")
    try:
        os.makedirs(d, exist_ok=True)
    except OSError:
        return None
    n = 0
    try:
        with open(p, "r", encoding="utf-8") as f:
            n = int((f.read() or "0").strip() or "0")
    except (OSError, ValueError):
        n = 0
    if n < 0:
        n = 0
    n += 1
    tmp = None
    try:
        fd, tmp = tempfile.mkstemp(prefix=".capgate-", dir=d)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write("%d\n" % n)
        os.replace(tmp, p)
    except OSError:
        if tmp:
            try:
                os.unlink(tmp)
            except OSError:
                pass
    return n


def warn_once(key, root):
    """1,500 경고를 세션당 1회만 낸다(같은 경고 반복은 잡음이고 컨텍스트를 먹는다)."""
    if not key:
        return False
    p = os.path.join(root, "capgate", key + ".warned")
    if os.path.exists(p):
        return False
    try:
        with open(p, "w", encoding="utf-8") as f:
            f.write("1\n")
    except OSError:
        return True     # 표시를 못 남겨도 경고 자체는 낸다(반복은 감수)
    return True


# ── 판정 ─────────────────────────────────────────────────────────────────────
def _skill_name(ti):
    for k in ("skill", "name", "skill_name", "skillName"):
        v = ti.get(k)
        if isinstance(v, str) and v.strip():
            n = v.strip()
            return n.split(":")[-1] if ":" in n else n
    return None


def _target_path(tool, ti):
    if tool == "NotebookEdit":
        return ti.get("notebook_path") or ti.get("file_path")
    return ti.get("file_path")


def decide_cso(tool, tool_input, role, ctx):
    """(block, reason). 판정 순서는 **도구 이름 → 경로 → 상한 → 예산**으로 고정한다 —
    면제(essential)를 먼저 돌려주면 허용 경로·64KB 검사를 건너뛴다(codex 지적)."""
    ti = tool_input if isinstance(tool_input, dict) else {}
    if tool in CSO_DENY_TOOLS:
        return True, ("CSO 범위 밖 도구 `%s` — 도구 이름 deny 는 명령 접두로 표현되지 않으므로 "
                      "TTL 승인으로 열리지 않는다(§1-1). 필요하면 master 에 사유 1줄을 상신하고 "
                      "보류하라" % tool)
    if tool.startswith(MCP_COMPUTER_USE_PREFIX):
        return True, ("이미지 캡처(`%s`)는 게이트 등록 여부와 무관하게 예외가 없다 — 증거는 "
                      "`cys read-screen` 출력의 sha256 + 텍스트 요약 1줄이다(§1-1 스크린샷 정책)"
                      % tool)
    if tool == "Skill":
        name = _skill_name(ti)
        if name not in CSO_SKILL_ALLOW:
            return True, ("Skill `%s` 는 CSO 허용 밖이다(허용: %s)"
                          % (name, "|".join(sorted(CSO_SKILL_ALLOW))))
        essential = False
    elif tool in MUTATION_TOOLS:
        path = _target_path(tool, ti)
        ok, why = cso_path_allowed(path, ctx)
        if not ok:
            return True, "%s 대상 %r: %s" % (tool, path, why)
        over, detail = todo_cap_verdict(tool, ti, path, ctx)
        if over:
            return True, ("상태 파일 상한 초과 — %s. 완료·무행동 로그를 "
                          "`~/Desktop/CYSjavis/cso/logs/` 로 이관하고 다시 시도하라" % detail)
        essential = is_cso_state_file(path)
    elif tool == "Bash":
        deny, why, essential = cso_bash_verdict(ti.get("command"), ti, ctx)
        if deny:
            return True, ("%s — 게이트 deny 는 고장이 아니라 **승인 요청 신호**다: 보류하고 "
                          "master 에 사유 1줄을 상신하라(§1-1)" % why)
    else:
        essential = False
    if (ctx.tool_calls is not None and ctx.tool_calls >= BUDGET_DENY and not essential):
        return True, ("도구 호출 예산 소진(tool_calls=%d ≥ %d) — 비필수 도구 deny. 예산 소진은 "
                      "고장이 아니라 **사이클 신호**다: SESSION_STATE·CSO_TODO 를 저장하고 §2 "
                      "절차대로 사이클을 준비하라(필수 도구는 계속 열려 있다)"
                      % (ctx.tool_calls, BUDGET_DENY))
    return False, "cso 허용"


def decide(tool, tool_input, role, ctx=None):
    """(block: bool, reason). cso*/reviewer*/planner 만 차단 대상."""
    if is_cso(role):
        return decide_cso(tool, tool_input, role, ctx if ctx is not None else Ctx())
    if not is_reviewer_or_planner(role):
        return False, "not reviewer/planner/cso (role=%r) — pass" % role
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


def deny_payload(reason):
    return ('{"hookSpecificOutput":{"hookEventName":"PreToolUse",'
            '"permissionDecision":"deny","permissionDecisionReason":"%s"}}'
            % _json_escape(reason))


def context_payload(text):
    """차단 없이 **모델에게 닿는** 고지(경고). exit 0 의 stderr 는 모델에 전달되지 않으므로
    경고를 stderr 로만 내면 '경고 선행' 봉인(②)이 사실이 아니게 된다(codex P0).
    ★정직: 이 필드의 수용은 하네스 버전에 달렸다 — 모르는 필드면 무시되고(무해) 그때는
      경고가 전달되지 않는다. 그래서 stderr 1줄도 함께 낸다(둘 다 보장은 아니다)."""
    return ('{"hookSpecificOutput":{"hookEventName":"PreToolUse",'
            '"additionalContext":"%s"}}' % _json_escape(text))


def emit_deny(reason):
    """modern Claude Code permission-decision deny JSON을 stdout에 내고 exit 0.
    printf 고정 형태(외부 jq/python 의존 없음 — reason만 보간·이스케이프)."""
    sys.stdout.write(deny_payload(reason) + "\n")
    sys.exit(0)


def _read_hook_input():
    """훅 stdin 전문. **파일 경로 우선**(큰 Write 본문이 env 크기 상한에 걸려 판정 코드에
    도달조차 못 하는 경로를 없앤다 — codex: 64KB 검사가 큰 쓰기 때문에 시작하지 못했다)."""
    p = os.environ.get("CAPGATE_INPUT_FILE")
    if p:
        try:
            with open(p, "r", encoding="utf-8", errors="surrogateescape") as f:
                return f.read()
        except OSError:
            return ""
        finally:
            try:
                os.unlink(p)
            except OSError:
                pass
    return os.environ.get("CAPGATE_INPUT", "")


def main():
    raw = _read_hook_input()
    role = os.environ.get("CYS_SURFACE_ROLE", "")
    try:
        data = json.loads(raw)
    except ValueError:
        # JSON 파싱 실패 — reviewer면 fail-closed. CSO 는 강등(전 도구 차단 = 좌석 사망).
        if is_reviewer_or_planner(role):
            print("role-capability-gate: malformed hook JSON — failing closed (reviewer)", file=sys.stderr)
            sys.exit(2)
        if is_cso(role):
            print("role-capability-gate: 훅 입력 판독 불가 — CSO 게이트 강등(집행 0)", file=sys.stderr)
        sys.exit(0)
    if not isinstance(data, dict):
        sys.exit(0)
    tool = data.get("tool_name") or data.get("tool") or ""
    tool_input = data.get("tool_input") if isinstance(data.get("tool_input"), dict) else {}
    ctx = None
    if is_cso(role):
        root = state_root()
        key = session_key(data.get("session_id"))
        count = bump_tool_calls(key, root)
        ctx = Ctx(tool_calls=count)
        if count is not None and BUDGET_WARN <= count < BUDGET_DENY and warn_once(key, root):
            msg = ("[CSO 예산 경고] tool_calls=%d (경고 %d · 비필수 deny %d). 지금 "
                   "SESSION_STATE·CSO_TODO 를 저장하고 §2 절차대로 사이클을 준비하라 — "
                   "예산 소진은 고장이 아니라 사이클 신호다." % (count, BUDGET_WARN, BUDGET_DENY))
            sys.stdout.write(context_payload(msg) + "\n")
            print("role-capability-gate: " + msg, file=sys.stderr)
    block, reason = decide(tool, tool_input, role, ctx)
    if block:
        # 진단은 stderr(transcript), 차단 판정은 modern JSON permission-decision(stdout)+exit 0.
        print("role-capability-gate DENY: %s [role=%s tool=%s src=%s]"
              % (reason, role, tool, os.environ.get("CAPGATE_ROLE_SOURCE", "?")), file=sys.stderr)
        if is_cso(role):
            emit_deny("[CSO 능력 게이트] %s" % reason)
        emit_deny("%s surface는 producer 산출물 수정 금지 (producer != evaluator)" % (role or "reviewer"))
    sys.exit(0)


def self_test():
    fails = []
    # ── ① reviewer/planner(종전 핀 · 무변경) ────────────────────────────────
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
        # ★0.14.31 반례 추가(재핀 아님): 값을 먹는 git 전역 옵션 뒤의 write 서브커맨드.
        ("reviewer-codex", "Bash", {"command": "git -C /repo reset --hard"}),
        ("reviewer-codex", "Bash", {"command": "git -c user.name=x commit -m y"}),
        # 완화가 **넓히지 않는 것**: 명령 자체가 상태를 바꾸는 하위 명령과 산출물 내보내기.
        ("reviewer-codex", "Bash", {"command": "cargo publish"}),
        ("reviewer-codex", "Bash", {"command": "cargo install cargo-nextest"}),
        ("reviewer-codex", "Bash", {"command": "cargo add serde"}),
        ("reviewer-codex", "Bash", {"command": "cargo update"}),
        ("reviewer-codex", "Bash", {"command": "cargo fmt"}),
        ("reviewer-codex", "Bash", {"command": "go get example.com/x"}),
        ("reviewer-codex", "Bash", {"command": "go mod tidy"}),
        ("reviewer-codex", "Bash", {"command": "cargo"}),          # 하위 명령 없음 = 해석 불가
        ("reviewer-codex", "Bash", {"command": "go build -o /w/repo/bin/app ./cmd"}),
        # ★게이트 제어 상태는 reviewer 의 tmp 예외에서도 빠진다.
        ("reviewer-codex", "Write", {"file_path": "/tmp/cys-capgate-role-3-x-y"}),
    ]
    cases_allow = [
        ("worker", "Edit", {"file_path": "/x/a.rs"}),
        ("worker-2", "Bash", {"command": "rm -rf /x"}),
        ("master", "Write", {"file_path": "/x/b"}),
        # ★종전 픽스처 `("cso","Bash",{"command":"npm install"})` 는 이 WP 의 **목적 그 자체**
        #   (CSO full-trust 폐기)로 allow 에서 빠졌다 — 아래 CSO 배터리가 그 자리를 대신한다.
        ("master", "Bash", {"command": "npm install"}),
        ("", "Edit", {"file_path": "/x/a.rs"}),
        ("-", "Bash", {"command": "rm x"}),
        ("reviewer-codex", "Bash", {"command": "grep -rn foo ."}),
        ("reviewer-codex", "Bash", {"command": "cat /x/f && ls -la"}),
        ("reviewer-codex", "Bash", {"command": "git status"}),
        ("reviewer-codex", "Bash", {"command": "git -C /repo status"}),
        ("reviewer-codex", "Write", {"file_path": "/tmp/review-notes.md"}),
        ("reviewer-codex", "Edit", {"file_path": "/Users/x/.cys/scratch.txt"}),
        ("reviewer-codex", "Bash", {"command": "echo hi > /tmp/out.log"}),
        # ★0.14.31 완화(별 커밋 · 반례 추가): reviewer 의 정당한 **검증 실행**.
        #   완화의 뜻은 '명령 수준 변형 없음' 이지 '파일 쓰기 없음' 이 아니다(위 주석 참조).
        ("reviewer-codex", "Bash", {"command": "cargo test --locked --offline"}),
        ("reviewer-codex", "Bash", {"command": "cargo build"}),
        ("reviewer-codex", "Bash", {"command": "cargo +nightly clippy -- -D warnings"}),
        ("reviewer-codex", "Bash", {"command": "cargo test --lib readiness:: 2>&1 | tail -3"}),
        ("reviewer-gemini", "Bash", {"command": "go test ./..."}),
        ("reviewer-codex", "Bash", {"command": "go vet ./..."}),
        ("reviewer-codex", "Bash", {"command": "python3 -m pytest -q"}),
        ("reviewer-codex", "Bash", {"command": "cargo build --target-dir /tmp/rv"}),
    ]
    for role, tool, ti in cases_block:
        b, _ = decide(tool, ti, role)
        if not b:
            fails.append("BYPASS: role=%s %s %r" % (role, tool, ti))
    for role, tool, ti in cases_allow:
        b, r = decide(tool, ti, role)
        if b:
            fails.append("FALSE-POSITIVE(%s): role=%s %s %r" % (r, role, tool, ti))

    # ── ② CSO 픽스처 ───────────────────────────────────────────────────────
    PACK = "/w/pack"
    HOME = "/w/home"
    STATE = "/w/home/.cys/state"
    FILES = {
        PACK + "/round/CSO_TODO.md": "x" * 60000,
        PACK + "/round/SESSION_STATE.md": "s",
        PACK + "/round/MASTER_TODO.md": "m",
    }

    def reader(p):
        return FILES.get(_norm(p))

    def ctx(tool_calls=None, approver=lambda c: False):
        return Ctx(pack=PACK, state=STATE, home=HOME, tool_calls=tool_calls,
                   approver=approver, reader=reader, tempdir="/w/tmp")

    # ★실제 CSO 트랜스크립트 5종(감사 2026-09-06 에러 1·3의 실물 행동) — 전부 deny 여야 한다.
    cso_transcript_denies = [
        ("CronCreate", {"schedule": "*/10 * * * *", "prompt": "10분 점검"},
         "크론 재등록"),
        ("Write", {"file_path": HOME + "/Desktop/CYSjavis/_round/PROTOCOL_CSO.md",
                   "content": "규약"}, "규약 md 생성"),
        ("Write", {"file_path": PACK + "/bin/javis_cso_probe.py", "content": "#!/usr/bin/env python3"},
         "bin 도구 신설"),
        ("mcp__computer-use__screenshot", {}, "computer-use 스크린샷"),
        ("Edit", {"file_path": "/w/home/.cys/pack-dept-1/round/CSO_TODO.md",
                  "old_string": "a", "new_string": "b"}, "타 레인 TODO 편집"),
    ]
    for tool, ti, label in cso_transcript_denies:
        b, r = decide(tool, ti, "cso", ctx())
        if not b:
            fails.append("CSO-BYPASS(%s): %s %r" % (label, tool, ti))

    # 정상 점검 20종 — 전부 allow.
    cso_allow = [
        ("Bash", {"command": "cys status --json"}),
        ("Bash", {"command": "cys list"}),
        ("Bash", {"command": "cys ps"}),
        ("Bash", {"command": "cys identify"}),
        ("Bash", {"command": "cys read-screen --to master --lines 40"}),
        ("Bash", {"command": "cys queue list"}),
        ("Bash", {"command": "cys feed list --status pending"}),
        ("Bash", {"command": "cys schedule list"}),
        ("Bash", {"command": "cys gate-check"}),
        ("Bash", {"command": "cys reap-surface 12"}),
        ("Bash", {"command": "cys surface-role"}),
        ("Bash", {"command": "cys todo-path"}),
        ("Bash", {"command": "cys set-status --state busy"}),
        ("Bash", {"command": 'cys send --queued --to master "[CSO] 점검 완료"'}),
        ("Bash", {"command": 'cys feed push --wait --request-id cso-master-hang-2026 '
                             '--title "[CSO] master hang" --body "근거 1줄"'}),
        ("Bash", {"command": 'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_orchestra.py" check'}),
        ("Bash", {"command": "python3 /w/pack/bin/javis_resource_gate.py check"}),
        ("Bash", {"command": "cys status --json | grep alert_route"}),
        ("Bash", {"command": "cys status 2>&1"}),
        ("Bash", {"command": "shasum -a 256 /w/pack/round/SESSION_STATE.md"}),
        ("Bash", {"command": "git status"}),
        ("Bash", {"command": "sqlite3 -readonly /w/x.db \"select 1\""}),
        ("Write", {"file_path": PACK + "/round/CSO_TODO.md", "content": "짧게"}),
        ("Read", {"file_path": "/anywhere/x"}),
        ("Skill", {"skill": "hallucination-guard"}),
    ]
    for tool, ti in cso_allow:
        b, r = decide(tool, ti, "cso", ctx())
        if b:
            fails.append("CSO-FALSE-POSITIVE(%s): %s %r" % (r, tool, ti))

    # 예산 초과 중에도 필수는 열려 있다(봉인표 ② — 무clear 방지).
    over = ctx(tool_calls=BUDGET_DENY + 5)
    essential_over = [
        ("Bash", {"command": "cys cycle-agent --role master --verifier cso --timeout 120 "
                             "--save-file '/w/cwd/_round/SESSION_STATE.md' "
                             "--save-file '/w/pack/round/MASTER_TODO.md'"}),
        ("Bash", {"command": "cys cycle-agent --role master --verifier cso"}),
        ("Bash", {"command": 'cys send --to master "예산 소진 보고"'}),
        ("Bash", {"command": 'cys send --queued --to master "예산 소진 보고"'}),
        ("Bash", {"command": "cys status --json"}),
        ("Bash", {"command": "cys queue list"}),
        ("Bash", {"command": "cys feed push --wait --request-id k --title t --body b"}),
        ("Bash", {"command": "cat /w/pack/round/SESSION_STATE.md"}),
        ("Bash", {"command": "shasum -a 256 /w/pack/round/SESSION_STATE.md"}),
        ("Bash", {"command": "python3 /w/pack/bin/javis_cycle_autopilot.py tick"}),
        ("Write", {"file_path": PACK + "/round/SESSION_STATE.md", "content": "저장"}),
        ("Edit", {"file_path": PACK + "/round/CSO_TODO.md", "old_string": "x" * 100,
                  "new_string": ""}),
    ]
    for tool, ti in essential_over:
        b, r = decide(tool, ti, "cso", over)
        if b:
            fails.append("BUDGET-KILLS-CYCLE(%s): %s %r" % (r, tool, ti))
    b, _ = decide("Bash", {"command": "cys gate-check"}, "cso", over)
    if not b:
        fails.append("BUDGET-NO-DENY: 예산 초과인데 비필수가 통과했다")
    b, _ = decide("WebSearch", {"query": "x"}, "cso", ctx())
    if not b:
        fails.append("CSO-BYPASS: WebSearch")
    return fails, len(cases_block), len(cases_allow), cso_allow, cso_transcript_denies


def self_test_contracts(fails):
    """★배선·경계 계약(codex 적대 반례 반영) — 순수 판정기로 잴 수 있는 것만 여기서 잰다."""
    PACK, HOME, STATE = "/w/pack", "/w/home", "/w/home/.cys/state"
    FILES = {
        PACK + "/round/CSO_TODO.md": "x" * 60000,
        PACK + "/round/SESSION_STATE.md": "ab" * 10,
    }

    def reader(p):
        return FILES.get(_norm(p))

    def ctx(tool_calls=None, approver=lambda c: False, background=False):
        return Ctx(pack=PACK, state=STATE, home=HOME, tool_calls=tool_calls,
                   approver=approver, reader=reader, tempdir="/w/tmp", background=background)

    def want(deny, tool, ti, label, c=None, role="cso"):
        b, r = decide(tool, ti, role, c if c is not None else ctx())
        if b != deny:
            fails.append("CONTRACT[%s]: 기대 %s 인데 %s (%s)"
                         % (label, "deny" if deny else "allow", "deny" if b else "allow", r))

    # 세그먼트 전수 검사 — 첫 세그먼트가 필수라고 전체를 면제하지 않는다.
    want(True, "Bash", {"command": "cys status && cys events"}, "복합: 뒤 세그먼트 deny")
    want(True, "Bash", {"command": "cys status; cys kill 123"}, "복합: `;` 뒤 TTL 동사")
    want(True, "Bash", {"command": "cys status\ncys kill 123"}, "개행 경계(인자 위장 차단)")
    want(False, "Bash", {"command": 'cys send --queued --to master "1줄\n2줄"'},
         "인용 안 개행은 경계가 아니다")
    # 명령 치환·백그라운드·스트림.
    want(True, "Bash", {"command": 'cys send --to master "결과: $(cys events)"'}, "명령 치환")
    want(True, "Bash", {"command": "cat `cys status`"}, "백틱 치환")
    want(True, "Bash", {"command": "cat <(cys status)"}, "프로세스 치환")
    want(True, "Bash", {"command": "tail -f /w/x.log"}, "tail -f 스트림")
    want(True, "Bash", {"command": "cys status &"}, "백그라운드 `&`")
    want(True, "Bash", {"command": "tail -n 5 /w/x.log", "run_in_background": True},
         "run_in_background 필드")
    # 읽기 명령의 쓰기 옵션.
    want(True, "Bash", {"command": "git diff --output=/w/pack/round/x.patch"}, "git --output")
    want(True, "Bash", {"command": "git -C /repo status"}, "git 전역 옵션(CSO는 금지)")
    want(True, "Bash", {"command": 'sqlite3 /w/x.db "select 1; delete from t"'},
         "sqlite3 -readonly 없음")
    want(True, "Bash", {"command": "sed -n '1,80p' /w/x"}, "sed 는 CSO 접두 밖(좁히는 방향)")
    want(True, "Bash", {"command": "rg --pre /w/h pat /w/f"}, "rg --pre 외부 실행")
    # `cys` 인자 계약.
    want(True, "Bash", {"command": "cys cycle-agent --role master --force-no-verify"},
         "--force-no-verify")
    want(True, "Bash", {"command": "cys cycle-agent --role master --clear-cmd 'x'"}, "--clear-cmd")
    want(True, "Bash", {"command": 'cys send --to master-shadow "x"'}, "수신자 경계")
    want(True, "Bash", {"command": 'cys send --to master --to worker "x"'}, "수신자 둘")
    want(False, "Bash", {"command": 'cys send --to=master "x"'}, "--to=master 형태")
    want(True, "Bash", {"command": 'cys send --to master --clear-first "x"'}, "--clear-first")
    want(True, "Bash", {"command": 'cys send --surface 7 "x"'}, "--surface 주소 우회")
    want(False, "Bash", {"command": 'cys send --to master "본문에 --to master-shadow 문자열"'},
         "본문 문자열은 수신자가 아니다")
    want(True, "Bash", {"command": "cys events --category queue"}, "events 전 플래그 deny")
    want(True, "Bash", {"command": "cys events --reconnect"}, "events --reconnect deny")
    # python 판정 도구 — basename 신뢰 금지 · 하위 명령 경계.
    want(True, "Bash", {"command": "python3 /w/tmp/javis_cycle_autopilot.py tick"},
         "scratchpad 동명 스크립트")
    want(True, "Bash", {"command": "python3 /w/pack/bin/javis_cycle_autopilot.py reset --role master"},
         "autopilot reset")
    want(True, "Bash", {"command": "python3 /w/pack/bin/javis_cycle_autopilot.py bootstrap-verifier"},
         "autopilot bootstrap-verifier")
    want(False, "Bash", {"command": "python3 -B /w/pack/bin/javis_cycle_autopilot.py tick"},
         "python -B 플래그")
    want(True, "Bash", {"command": "python3 -c 'import os'"}, "python -c")
    want(True, "Bash", {"command": "python3 /w/pack/bin/javis_preflight.py --fix"},
         "preflight --fix")
    want(False, "Bash", {"command": "python3 /w/pack/bin/javis_preflight.py --json"},
         "preflight 조회")
    want(True, "Bash", {"command": "python3 /w/pack/bin/javis_orchestra.py round-log --task t"},
         "orchestra 변이 하위 명령")
    # 게이트 제어 상태·상한 우회.
    want(True, "Write", {"file_path": STATE + "/capgate/abc.count", "content": "0"},
         "예산 카운터 직접 쓰기")
    want(True, "Bash", {"command": "echo 0 > /w/home/.cys/state/capgate/abc.count"},
         "예산 카운터 리다이렉트")
    want(True, "Bash", {"command": "cat /w/big > /w/pack/round/CSO_TODO.md"},
         "상태 파일 리다이렉트(상한 우회)")
    want(False, "Bash", {"command": "cys status > /dev/null"}, "/dev/null 싱크")
    want(True, "Write", {"file_path": PACK + "/round/MASTER_TODO.md", "content": "x"},
         "같은 팩의 남의 TODO")
    # 64KB 상한 — UTF-8 바이트 · replace_all · MultiEdit 누적 · 축소 허용.
    want(True, "Write", {"file_path": PACK + "/round/CSO_TODO.md", "content": "가" * 30000},
         "UTF-8 바이트(90,000B)")
    want(False, "Write", {"file_path": PACK + "/round/CSO_TODO.md", "content": "가" * 10000},
         "UTF-8 30,000B 는 상한 안")
    want(True, "Edit", {"file_path": PACK + "/round/CSO_TODO.md", "old_string": "x",
                        "new_string": "xy", "replace_all": True},
         "replace_all 누적 증가")
    want(False, "Edit", {"file_path": PACK + "/round/CSO_TODO.md", "old_string": "x" * 100,
                         "new_string": "x"}, "축소는 허용")
    want(True, "MultiEdit", {"file_path": PACK + "/round/CSO_TODO.md",
                             "edits": [{"old_string": "x", "new_string": "x" * 3000,
                                        "replace_all": True}]}, "MultiEdit 누적")
    # TTL 승인.
    want(True, "Bash", {"command": "cys close-surface 17"}, "TTL 없음 = 미승인")
    want(False, "Bash", {"command": "cys close-surface 17"}, "TTL 승인 있음",
         c=ctx(approver=lambda c: c == "cys close-surface 17"))
    want(True, "Bash", {"command": "cys close-surface 18"}, "다른 명령은 그 증표가 아니다",
         c=ctx(approver=lambda c: c == "cys close-surface 17"))
    # 도구 이름 deny 는 TTL 로 열리지 않는다.
    want(True, "CronCreate", {}, "CronCreate 는 TTL 로 안 열린다",
         c=ctx(approver=lambda c: True))
    want(True, "Skill", {"skill": "appbuild"}, "허용 밖 Skill")
    want(True, "Task", {"prompt": "x"}, "Task=Agent 서브에이전트")
    # 역할 접두 — session-start.sh `cso*)` 미러.
    for r in ("cso", "cso-1", "cso-dept-3"):
        b, _ = decide("Bash", {"command": "cys events"}, r, ctx())
        if not b:
            fails.append("CONTRACT[역할 접두 %s]: cso* 가 게이트 대상이 아니다" % r)
    for r in ("csosomething",):
        b, _ = decide("Bash", {"command": "cys events"}, r, ctx())
        if not b:
            fails.append("CONTRACT[역할 접두 %s]: pack.rs 접두 의미론과 다르다" % r)
    b, _ = decide("Bash", {"command": "cys events"}, "master", ctx())
    if b:
        fails.append("CONTRACT[master]: full-trust 역할이 막혔다")
    # session_key — 경로 주입·충돌·결측.
    if session_key("../../outside") == session_key("C:\\Temp\\outside"):
        fails.append("CONTRACT[session_key]: 서로 다른 세션이 같은 키가 됐다")
    for bad in ("../../outside", "C:\\Temp\\outside", "a/b"):
        k = session_key(bad)
        if not k or not re.match(r"^[0-9a-f]{32}$", k):
            fails.append("CONTRACT[session_key]: %r → %r (32hex 아님)" % (bad, k))
    for missing in (None, "", "   ", 3, {}):
        if session_key(missing) is not None:
            fails.append("CONTRACT[session_key]: 결측/타입오류를 공용 키로 접었다 (%r)" % (missing,))
    # 예산 판정 불능(계수 실패)은 deny 가 아니다.
    b, _ = decide("Bash", {"command": "cys gate-check"}, "cso", ctx(tool_calls=None))
    if b:
        fails.append("CONTRACT[예산 결측]: 계수 불가를 초과로 읽었다")
    return fails


def run_self_test():
    fails, n_block, n_allow, cso_allow, cso_deny = self_test()
    self_test_contracts(fails)

    # ★API 계약 검증: deny 경로는 permissionDecision==deny JSON을 내는가 / 허용은 무출력인가.
    emitted = deny_payload("reviewer-codex surface는 producer 산출물 수정 금지 (producer != evaluator)")
    try:
        hso = json.loads(emitted)["hookSpecificOutput"]
        if hso.get("hookEventName") != "PreToolUse":
            fails.append("EMIT: hookEventName != PreToolUse")
        if hso.get("permissionDecision") != "deny":
            fails.append("EMIT: permissionDecision != deny")
        if "producer != evaluator" not in hso.get("permissionDecisionReason", ""):
            fails.append("EMIT: reason missing producer!=evaluator")
        if "reviewer-codex" not in hso.get("permissionDecisionReason", ""):
            fails.append("EMIT: reason missing role")
    except (ValueError, KeyError) as e:
        fails.append("EMIT: deny JSON not valid/shaped: %s" % e)
    try:
        wso = json.loads(context_payload("경고 1줄"))["hookSpecificOutput"]
        if wso.get("hookEventName") != "PreToolUse" or "permissionDecision" in wso:
            fails.append("EMIT: 경고 payload 가 권한 판정을 싣는다(차단 없음 계약 위반)")
        if wso.get("additionalContext") != "경고 1줄":
            fails.append("EMIT: 경고 payload 에 본문이 없다")
    except (ValueError, KeyError) as e:
        fails.append("EMIT: 경고 JSON not valid/shaped: %s" % e)

    if fails:
        print("\n".join(fails), file=sys.stderr)
        print("self-test: %d failure(s)" % len(fails), file=sys.stderr)
        sys.exit(1)
    print("self-test OK: reviewer blocked %d · reviewer allowed %d · CSO 트랜스크립트 deny %d · "
          "CSO 정상 allow %d · 예산·경계 계약 통과 · deny=permissionDecision JSON(exit0)·"
          "allow=empty-stdout(exit0)·fail-closed(exit2) verified"
          % (n_block, n_allow, len(cso_deny), len(cso_allow)))
    sys.exit(0)


if os.environ.get("CAPGATE_SELF_TEST"):
    run_self_test()
else:
    main()
PYEOF
