#!/bin/sh
# test_pre_dispatch.sh — pre-dispatch.sh(D4) 회귀·동치 하니스
#
# 검증 목표: '기존 체인'(Claude Code 가 4개 PreToolUse 항목을 각자 실행)과 '디스패처'가
#   동일 입력 행렬에서 완전 동치임을 실측한다:
#     (1) 최종 차단 여부(exit 2 vs 0) 동일
#     (2) 실행된 서브훅 집합·순서 동일 (각 서브훅 래퍼가 호출을 로그에 남겨 관측)
#     (3) cys-hook 부수효과(사용량 이벤트) 발생 동일 — ★비-편집 도구(Read/Task)에서도 존속
#     (4) 차단 서브훅의 stdout(JSON)·stderr 가 디스패처 최종 출력으로 채택됨
#     (5) 시간 실측 before/after (Bash·Edit)
#
# 관측 기법: 실제 서브훅을 수정하지 않는다. 대신 테스트용 hooks 디렉토리에 '래퍼'를 두어
#   호출을 로그한 뒤 실제 pack 서브훅을 exec 한다. 디스패처 복사본의 HOOK_DIR 은 이 테스트
#   디렉토리로 해석되므로 래퍼를 경유해 실제 서브훅을 부른다. 기존-체인 시뮬레이션도 동일
#   래퍼를 Claude Code 의 정확-일치 matcher 로 직접 호출한다.
#
# 차단 유발 입력은 시뮬레이션 JSON 뿐(실제 파일 삭제·발행 없음).

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(test_pre_dispatch)" >&2; exit 0; }
set -u

REAL_HOOKS=${REAL_HOOKS:-$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd)}
REAL_GUARD=${REAL_GUARD:-$HOME/dev/cys-arp/_round/autopilot/guard.sh}

for f in "$REAL_HOOKS/pre-dispatch.sh" "$REAL_HOOKS/cys-hook.sh" "$REAL_HOOKS/appbuild-gate.sh" "$REAL_HOOKS/grill-gate.sh"; do
  [ -f "$f" ] || { echo "FATAL: 서브훅 없음 $f"; exit 1; }
done
[ -x "$REAL_GUARD" ] || { echo "FATAL: guard 실행불가 $REAL_GUARD"; exit 1; }

T=$(mktemp -d "${TMPDIR:-/tmp}/test-pre-dispatch.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT INT TERM

export INVOKE_LOG="$T/invoke.log"
export CYS_LOG="$T/cys.log"
export REAL_HOOKS
export REAL_GUARD

# ---- cys 스텁: usage-event 만 기록, approval check 는 미서명(exit 1)으로 guard LOOSE deny 유지 ----
mkdir -p "$T/bin"
cat >"$T/bin/cys" <<'STUB'
#!/bin/sh
case "$1" in
  usage-event-stdin) cat >/dev/null 2>&1; echo usage-event >> "$CYS_LOG"; exit 0 ;;
  approval) exit 1 ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$T/bin/cys"
export PATH="$T/bin:$PATH"

# ---- 서브훅 래퍼(호출 로그 → 실제 pack 서브훅 exec) ----
mkdir -p "$T/hooks"
cp "$REAL_HOOKS/pre-dispatch.sh" "$T/hooks/pre-dispatch.sh"
# ★공용 프리루드 동반 복사(T-0147-7 W1a): 스텁 훅 디렉터리는 팩 레이아웃을 **미러**해야 한다.
#   _lib.sh 가 없으면 복사된 pre-dispatch.sh 가 loud-skip 으로 전면 강등돼(설치된 팩이 구버전이면
#   2단 폴백도 못 잡는다) 이 하네스 전체가 무의미해진다(실측: 56/56 → 10/56).
cp "$REAL_HOOKS/_lib.sh" "$T/hooks/_lib.sh" 2>/dev/null || true
for name in cys-hook appbuild-gate grill-gate; do
  cat >"$T/hooks/$name.sh" <<WRAP
#!/bin/sh
echo $name >> "\$INVOKE_LOG"
exec sh "\$REAL_HOOKS/$name.sh" "\$@"
WRAP
  chmod +x "$T/hooks/$name.sh"
done
cat >"$T/guard-wrap.sh" <<'WRAP'
#!/bin/sh
echo guard >> "$INVOKE_LOG"
exec "$REAL_GUARD" "$@"
WRAP
chmod +x "$T/guard-wrap.sh"

# ---- appbuild 차단 픽스처: .appbuild 존재 + 05-gate.md 부재 ----
mkdir -p "$T/proj/.appbuild" "$T/proj/src" "$T/scratch"

# ================= 헬퍼 =================
# 정확-일치 목록 matcher (Claude Code 의미론 미러 — 부분문자열 아님)
match() { _n=$1; shift; case " $* " in *" $_n "*) return 0 ;; *) return 1 ;; esac; }

# 기존 체인: 각 서브훅을 정확-일치 matcher 로 독립 실행, block 은 OR
run_chain() {  # $1=tool  $2=json
  _tool=$1; _json=$2
  : > "$INVOKE_LOG"; : > "$CYS_LOG"
  CHAIN_BLOCK=0
  if match "$_tool" Bash Write Edit MultiEdit NotebookEdit; then
    printf '%s' "$_json" | CYS_GUARD_HOOK="$T/guard-wrap.sh" "$T/guard-wrap.sh" >/dev/null 2>&1
    [ $? -eq 2 ] && CHAIN_BLOCK=1
  fi
  printf '%s' "$_json" | sh "$T/hooks/cys-hook.sh" >/dev/null 2>&1   # 전 도구
  if match "$_tool" Edit Write NotebookEdit; then
    printf '%s' "$_json" | sh "$T/hooks/appbuild-gate.sh" >/dev/null 2>&1
    [ $? -eq 2 ] && CHAIN_BLOCK=1
  fi
  if match "$_tool" Edit Write NotebookEdit; then
    printf '%s' "$_json" | sh "$T/hooks/grill-gate.sh" >/dev/null 2>&1
    [ $? -eq 2 ] && CHAIN_BLOCK=1
  fi
  CHAIN_INVOKE=$(tr '\n' ',' < "$INVOKE_LOG")
  CHAIN_USAGE=$(wc -l < "$CYS_LOG" | tr -d ' ')
}

# 디스패처: 단일 진입점
run_dispatch() {  # $1=json
  _json=$1
  : > "$INVOKE_LOG"; : > "$CYS_LOG"
  DISP_OUT=$(printf '%s' "$_json" | CYS_GUARD_HOOK="$T/guard-wrap.sh" sh "$T/hooks/pre-dispatch.sh" 2>"$T/derr")
  DISP_RC=$?
  DISP_BLOCK=0; [ "$DISP_RC" -eq 2 ] && DISP_BLOCK=1
  DISP_ERR=$(cat "$T/derr")
  DISP_INVOKE=$(tr '\n' ',' < "$INVOKE_LOG")
  DISP_USAGE=$(wc -l < "$CYS_LOG" | tr -d ' ')
}

PASS=0; FAIL=0
check() {  # $1=설명 $2=기대 $3=실제
  if [ "$2" = "$3" ]; then PASS=$((PASS+1)); printf '    ok   %s (=%s)\n' "$1" "$3"
  else FAIL=$((FAIL+1)); printf '    FAIL %s : 기대[%s] 실제[%s]\n' "$1" "$2" "$3"; fi
}
contains() {  # $1=설명 $2=haystack $3=needle
  case "$2" in *"$3"*) PASS=$((PASS+1)); printf '    ok   %s (포함:%s)\n' "$1" "$3" ;;
    *) FAIL=$((FAIL+1)); printf '    FAIL %s : [%s] 에 [%s] 없음\n' "$1" "$2" "$3" ;; esac
}

# case: 설명·tool·json·기대invoke(등록순 CSV)·기대block
case_run() {  # $1=name $2=tool $3=json $4=exp_invoke $5=exp_block
  printf '\n[%s] tool=%s\n' "$1" "$2"
  run_chain "$2" "$3"
  run_dispatch "$3"
  check "block 동치(chain=$CHAIN_BLOCK)" "$CHAIN_BLOCK" "$DISP_BLOCK"
  check "block 기대치" "$5" "$DISP_BLOCK"
  check "invoke 집합·순서 동치" "$CHAIN_INVOKE" "$DISP_INVOKE"
  check "invoke 기대치" "$4" "$DISP_INVOKE"
  check "cys-hook 사용량 동치(chain=$CHAIN_USAGE)" "$CHAIN_USAGE" "$DISP_USAGE"
  check "사용량 정확히 1건" "1" "$DISP_USAGE"
}

echo "================ 회귀·동치 행렬 ================"

# C1 Bash 정상
case_run "C1 Bash 정상(ls)" Bash \
  '{"tool_name":"Bash","tool_input":{"command":"ls -la"}}' \
  "guard,cys-hook," 0

# C2 Bash guard 차단(git push = 외부발행 비가역)
case_run "C2 Bash guard차단(git push)" Bash \
  '{"tool_name":"Bash","tool_input":{"command":"git push origin main"}}' \
  "guard,cys-hook," 1
contains "디스패처 stdout=deny JSON" "$DISP_OUT" "permissionDecision"
contains "디스패처 stdout=deny" "$DISP_OUT" "deny"
contains "디스패처 stderr=차단사유" "$DISP_ERR" "DENY"

# C3 Edit 정상(무 .appbuild)
case_run "C3 Edit 정상" Edit \
  "{\"tool_name\":\"Edit\",\"tool_input\":{\"file_path\":\"$T/scratch/n.txt\"}}" \
  "guard,cys-hook,appbuild-gate,grill-gate," 0

# C4 Write appbuild 차단(.appbuild 존재·05-gate 부재·소스확장자)
case_run "C4 Write appbuild차단(.ts)" Write \
  "{\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"$T/proj/src/app.ts\"}}" \
  "guard,cys-hook,appbuild-gate,grill-gate," 1
contains "디스패처 stderr=appbuild BLOCKED" "$DISP_ERR" "appbuild-gate BLOCKED"

# C5 Read — ★비-편집 도구에서 cys-hook 만 실행(matcher 소실 검출)
case_run "C5 Read(비편집)" Read \
  '{"tool_name":"Read","tool_input":{"file_path":"/etc/hosts"}}' \
  "cys-hook," 0

# C6 Task — 비-편집 도구에서 cys-hook 만
case_run "C6 Task(비편집)" Task \
  '{"tool_name":"Task","tool_input":{"description":"x"}}' \
  "cys-hook," 0

# C7 Write soul.md — guard 헌법파일 차단
case_run "C7 Write soul.md(헌법)" Write \
  "{\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"$T/soul.md\"}}" \
  "guard,cys-hook,appbuild-gate,grill-gate," 1
contains "디스패처 stdout=deny JSON" "$DISP_OUT" "permissionDecision"

# C8 MultiEdit — ★정확-일치 의미론: appbuild/grill 은 MultiEdit 미매칭(부분문자열 아님)
case_run "C8 MultiEdit(정확일치)" MultiEdit \
  "{\"tool_name\":\"MultiEdit\",\"tool_input\":{\"file_path\":\"$T/scratch/m.txt\"}}" \
  "guard,cys-hook," 0

# ================ CASE-G: CYS_GUARD_HOOK 미설정 시 guard skip(exit0 유지) ================
printf '\n[CASE-G] CYS_GUARD_HOOK 미설정 — guard skip·나머지 실행·exit0\n'
: > "$INVOKE_LOG"; : > "$CYS_LOG"
G_OUT=$(printf '%s' '{"tool_name":"Bash","tool_input":{"command":"git push origin main"}}' \
  | env -u CYS_GUARD_HOOK sh "$T/hooks/pre-dispatch.sh" 2>"$T/gerr")
G_RC=$?
G_INVOKE=$(tr '\n' ',' < "$INVOKE_LOG")
check "guard 미설정 시 exit 0(차단 안 함)" "0" "$G_RC"
check "guard 미호출·cys-hook 만 실행" "cys-hook," "$G_INVOKE"
contains "stderr 경고 존재" "$(cat "$T/gerr")" "CYS_GUARD_HOOK 미설정"

# ================ 시간 실측 before/after ================
printf '\n================ 시간 실측 (N=8, 실제 pack 서브훅 직접·stub cys) ================\n'
REAL_HOOKS="$REAL_HOOKS" REAL_GUARD="$REAL_GUARD" DISP="$REAL_HOOKS/pre-dispatch.sh" \
"${CYS_PY:-python3}" - <<'PY'
import os, subprocess, time
RH=os.environ["REAL_HOOKS"]; RG=os.environ["REAL_GUARD"]; DISP=os.environ["DISP"]
N=8
def t(fn):
    fn(); fn()  # warmup
    s=time.perf_counter()
    for _ in range(N): fn()
    return (time.perf_counter()-s)/N*1000
def run(cmd, data, env=None):
    e=dict(os.environ);
    if env: e.update(env)
    subprocess.run(cmd, input=data.encode(), stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, env=e)
bash='{"tool_name":"Bash","tool_input":{"command":"ls -la"}}'
edit='{"tool_name":"Edit","tool_input":{"file_path":"/tmp/x.txt"}}'
genv={"CYS_GUARD_HOOK":RG}
# 기존 체인 = 각 서브훅을 별도 프로세스로(Claude Code 항목별 실행 근사)
def chain_bash():
    run([RG], bash, genv); run(["sh",f"{RH}/cys-hook.sh"], bash)
def chain_edit():
    run([RG], edit, genv); run(["sh",f"{RH}/cys-hook.sh"], edit)
    run(["sh",f"{RH}/appbuild-gate.sh"], edit); run(["sh",f"{RH}/grill-gate.sh"], edit)
def disp_bash(): run(["sh",DISP], bash, genv)
def disp_edit(): run(["sh",DISP], edit, genv)
cb, db = t(chain_bash), t(disp_bash)
ce, de = t(chain_edit), t(disp_edit)
print(f"  Bash: 기존 체인(2스폰) {cb:6.1f}ms  →  디스패처 {db:6.1f}ms  (Δ {cb-db:+.1f}ms)")
print(f"  Edit: 기존 체인(4스폰) {ce:6.1f}ms  →  디스패처 {de:6.1f}ms  (Δ {ce-de:+.1f}ms)")
print("  ※주의: 이 하니스는 서브훅 self-time 만 잰다. 실제 절감의 본체는 Claude Code 가")
print("    PreToolUse 항목을 4→1(Edit)·2→1(Bash)로 적게 '띄우는' 항목당 오버헤드이며,")
print("    이는 하니스 외부라 여기서 재현 불가(master 실측 항목당 33~72ms 참조).")
PY

# ================ 결과 ================
printf '\n================ 결과: PASS=%d FAIL=%d ================\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
