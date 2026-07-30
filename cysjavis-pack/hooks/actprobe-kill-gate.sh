#!/bin/sh
# actprobe-kill-gate.sh — PreToolUse Bash kill 가드 훅 (GATE 클래스)
# 설계 §2.3 · RECON §6 계약 · 라이브 배선 승인 시 ~/.cys/pack/hooks/ 로 복사 등록(P4).
#
# 역할: Bash 툴이 kill/pkill/killall 을 실행하려 할 때만 개입해 javis_actprobe.py
#       kill-preflight 를 자동 발동하고, 고아 오판 kill 을 결정론(exit code)으로 차단.
#
# ★fail 정책의 층위 구분 (설계 §2.3 · completion_guard 선례):
#   - 판정(probe FAIL exit2 / INDET exit3, 이름기반·pid 미추출) = fail-CLOSED → 차단(exit 2).
#   - 인프라 오류(stdin 파싱 불가·python3 부재·actprobe 부재·타임아웃·예상밖 rc)
#     = fail-OPEN → 통과(exit 0) + stderr 경고. 부트·온보딩 무해 보장.
#   exit0+deny JSON 은 fail-open 함정이므로 차단은 반드시 exit 2 (RECON §6).

set -u

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(actprobe-kill-gate)" >&2; exit 0; }

# ── 인터프리터 해석 (없으면 hook JSON 파싱 불가 = 인프라 fail-open) ───────────
# G22: 후보에 python·py 추가(Windows는 python3 명령이 없다). 프리루드 해소값을 최우선 후보로.
PYBIN=""
for c in "${CYS_PY:-python3}" python3 python py \
         /opt/homebrew/bin/python3 /usr/bin/python3 /usr/local/bin/python3; do
  [ -n "$c" ] || continue
  if command -v "$c" >/dev/null 2>&1; then PYBIN="$(command -v "$c")"; break; fi
  [ -x "$c" ] && { PYBIN="$c"; break; }
done
if [ -z "$PYBIN" ]; then
  echo "kill-gate: WARN python(3) 부재 — hook JSON 파싱 불가 · fail-open" >&2
  exit 0
fi

# ── timeout 바이너리(선택) — actprobe 폭주 방지 10s ──────────────────────
TIMEOUT_BIN=""
for t in timeout gtimeout; do
  command -v "$t" >/dev/null 2>&1 && { TIMEOUT_BIN="$t"; break; }
done

INPUT="$(cat)"

# ── hook JSON 판독 + kill 감지 + pid 추출 (python 1패스) ─────────────────
# 출력 1줄: NONBASH | NOTKILL | PARSEERR | NOPID | "PIDS <p1> <p2> ..."
PARSE_SRC='
import sys, json, re
raw = sys.stdin.read()
try:
    d = json.loads(raw)
except Exception:
    print("PARSEERR"); sys.exit(0)
if not isinstance(d, dict) or (d.get("tool_name") or "") != "Bash":
    print("NONBASH"); sys.exit(0)
ti = d.get("tool_input") or {}
cmd = ti.get("command") if isinstance(ti, dict) else None
cmd = cmd if isinstance(cmd, str) else ("" if cmd is None else str(cmd))

# ★G21(T-0147-7 W1a): KILL 동사 집합에 Windows 종료 동사 `taskkill` 추가.
#   종전 집합은 POSIX 3동사만이라 Windows의 실제 종료 경로가 가드를 **완전히 우회**했다
#   (파괴 경로는 이식됐는데 보호 술어는 미이식 — B13과 같은 비대칭 클래스).
#   `taskkill /F /PID 1234` → pid 추출 성공 → probe / `taskkill /IM claude.exe` → NOPID →
#   이름 기반이라 fail-closed 차단(정책 그대로).
KILL = {"kill", "pkill", "killall", "taskkill"}
# 투명 래퍼(뒤 토큰이 실제 명령) — sudo kill 등이 under-block 되지 않게 스킵
WRAP = {"sudo", "nohup", "exec", "command", "time", "doas", "builtin", "env", "then", "do"}

# 따옴표 상태를 추적하며 ①비인용 # 주석 제거 ②비인용 셸 구분자(; | & 개행)로 세그먼트 분할.
# = 각 실행 단위를 분리. 백슬래시 이스케이프·백틱은 인용으로 추적 안 함(단순 우선·아래 한계 명시).
def split_segments(s):
    segs = []; buf = []; in_s = in_d = False; prev = ""; i = 0; n = len(s)
    while i < n:
        c = s[i]
        if in_s:
            buf.append(c);  in_s = (c != "\x27")
        elif in_d:
            buf.append(c);  in_d = (c != "\x22")
        elif c == "\x27":
            in_s = True;  buf.append(c)
        elif c == "\x22":
            in_d = True;  buf.append(c)
        elif c == "#" and (prev == "" or prev in " \t\n"):
            while i < n and s[i] != "\n": i += 1     # 주석 폐기(행끝까지)
            prev = " ";  continue                     # \n 은 다음 회차에서 구분자로 처리
        elif c in ";|&\n":
            segs.append("".join(buf));  buf = []      # 실행 구분자 → 세그먼트 종료
        else:
            buf.append(c)
        prev = c;  i += 1
    segs.append("".join(buf))
    return segs

# 세그먼트를 따옴표 인지 공백 토큰화
def tokenize(seg):
    toks = []; cur = []; in_s = in_d = False
    for c in seg:
        if in_s:
            cur.append(c);  in_s = (c != "\x27")
        elif in_d:
            cur.append(c);  in_d = (c != "\x22")
        elif c == "\x27":
            in_s = True;  cur.append(c)
        elif c == "\x22":
            in_d = True;  cur.append(c)
        elif c in " \t":
            if cur: toks.append("".join(cur)); cur = []
        else:
            cur.append(c)
    if cur: toks.append("".join(cur))
    return toks

# 세그먼트의 첫 실행 토큰(명령어)과 인자부 — 선행 assignment(VAR=…)·래퍼는 스킵
def command_word(seg):
    toks = tokenize(seg); idx = 0
    while idx < len(toks):
        bare = toks[idx].replace("\x27", "").replace("\x22", "")
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", bare):   # 환경 대입 프리픽스
            idx += 1; continue
        if bare in WRAP:                                   # 투명 래퍼
            idx += 1; continue
        return bare, toks[idx + 1:]
    return "", []

# kill 을 "실행 위치"에서만 감지 — grep 인자·문자열·주석·로그 텍스트의 kill 은 제외(over-block 수리)
kill_args = []
for seg in split_segments(cmd):
    cw, rest = command_word(seg)
    if cw in KILL:
        kill_args.append(rest)
if not kill_args:
    print("NOTKILL"); sys.exit(0)

# kill 인자에 변수·치환($ · $( · 백틱)이 있으면 간접 대상 → decoy 숫자와 무관하게 NOPID 차단 우선
indirect = False; pids = []
for rest in kill_args:
    argtext = " ".join(rest)
    if re.search(r"[\$`]", argtext):
        indirect = True
    # 숫자 pid: 앞뒤 비단어 경계. "-9"(신호)는 선행 - 로 제외, "node2"(이름)는 선행 알파로 제외
    pids += re.findall(r"(?<![A-Za-z0-9_-])([0-9]+)(?![A-Za-z0-9_])", argtext)

if indirect:
    print("NOPID"); sys.exit(0)
if pids:
    print("PIDS " + " ".join(pids))
else:
    print("NOPID")
'
DECISION="$(printf '%s' "$INPUT" | "$PYBIN" -c "$PARSE_SRC" 2>/dev/null)" || DECISION="PARSEERR"
[ -z "$DECISION" ] && DECISION="PARSEERR"

# ── deny 방출: exit0+JSON 은 fail-open 함정 → 반드시 stdout JSON + exit 2 ──
emit_deny() {
  _reason="$1"
  printf '%s\n' "kill-gate DENY: $_reason" >&2
  printf '%s' "$_reason" | "$PYBIN" -c 'import sys, json; r=sys.stdin.read(); print(json.dumps({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":r}}, ensure_ascii=False, separators=(",", ":")))'
  exit 2
}

verb="${DECISION%% *}"
case "$verb" in
  NONBASH|NOTKILL)
    exit 0 ;;                       # 무개입
  PARSEERR)
    echo "kill-gate: WARN hook JSON 파싱 불가 — fail-open" >&2
    exit 0 ;;                       # 인프라 fail-open
  NOPID)
    # pkill/killall(이름 기반)·pid 미추출 = 판정불가 → fail-closed 차단 + 안내
    emit_deny "kill-preflight INDET: pid 미명시(pkill/killall/명령치환) — pid 명시 kill 로 바꾸거나 수동 확인" ;;
  PIDS)
    : ;;                            # 아래에서 처리
  *)
    echo "kill-gate: WARN 알 수 없는 판정 '$DECISION' — fail-open" >&2
    exit 0 ;;
esac

# ── PIDS: actprobe 경로 해석 (env CYS_ACTPROBE > CYS_PACK_DIR/bin, _work 폴백 금지) ─
ACTPROBE="${CYS_ACTPROBE:-}"
if [ -z "$ACTPROBE" ] || [ ! -f "$ACTPROBE" ]; then
  ACTPROBE="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_actprobe.py"
fi
if [ ! -f "$ACTPROBE" ]; then
  echo "kill-gate: WARN actprobe 부재 ($ACTPROBE) — fail-open" >&2
  exit 0                           # 인프라 fail-open
fi

run_probe() {
  _pid="$1"
  if [ -n "$TIMEOUT_BIN" ]; then
    _out="$("$TIMEOUT_BIN" 10 "$PYBIN" "$ACTPROBE" kill-preflight --pid "$_pid" 2>&1)"; _rc=$?
  else
    _out="$("$PYBIN" "$ACTPROBE" kill-preflight --pid "$_pid" 2>&1)"; _rc=$?
  fi
  _reason="${_out#*: }"            # actprobe 접두 "[VERDICT] probe pid: " 제거
  # ── ★G21: rc=2/3 을 **출력 형상**으로 '판정' vs '인프라 실패' 로 분리 ──
  # exit 2/3 은 actprobe의 판정 코드(FAIL/INDET)인데 **동시에** 사용오류의 코드이기도 하다:
  # argparse 서브파서 오류·미지 인자·python SystemExit(2) 가 같은 숫자로 나온다. 종전 코드는
  # 그 전부를 '판정'으로 읽어 **인프라 실패를 차단으로 승격**했다 — 헤더 §2.3이 명문화한
  # fail-OPEN 정책의 역전이다(부트·온보딩 무해 보장 파괴).
  # 판정 출력은 `[FAIL] …` / `[INDET] …` verdict 토큰을 반드시 포함한다(javis_actprobe.py:584).
  # 토큰이 없으면 인프라 실패로 보고 정책대로 fail-open + WARN.
  case "$_rc" in
    0)   return 0 ;;                                       # PASS → 이 pid 허용
    2)
      case "$_out" in
        *"[FAIL]"*) emit_deny "kill-preflight FAIL: $_reason" ;;   # 판정 → 차단
        *) echo "kill-gate: WARN actprobe rc=2 인데 verdict 형상 아님(사용오류·인프라): $_out — fail-open" >&2
           exit 0 ;;
      esac ;;
    3)
      case "$_out" in
        *"[INDET]"*) emit_deny "kill-preflight INDET: $_reason" ;; # 판정불가 → 안전측 차단
        *) echo "kill-gate: WARN actprobe rc=3 인데 verdict 형상 아님(인자 오류 등): $_out — fail-open" >&2
           exit 0 ;;
      esac ;;
    124) echo "kill-gate: WARN actprobe 타임아웃 (pid $_pid) — fail-open" >&2; exit 0 ;;
    *)   echo "kill-gate: WARN actprobe 예상밖 rc=$_rc (pid $_pid): $_out — fail-open" >&2; exit 0 ;;
  esac
}

# 다중 pid: 하나라도 FAIL/INDET 면 emit_deny 가 exit 2 (안전측)
_pids="${DECISION#PIDS }"
for pid in $_pids; do
  run_probe "$pid"
done
exit 0                             # 모든 pid PASS → 허용
