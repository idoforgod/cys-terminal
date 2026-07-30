#!/bin/bash
# javis 결정론 부트스트랩 발화 — UserPromptSubmit hook
#
# 절대요구(오너 2026-07-15): "너는 마스터다" 류 마스터 선언이 입력되면, LLM의 재량·환각·누락과
# 무관하게 하네스가 부트스트랩을 100% 예외없이 발화한다. 부트 완료 = master·CSO·워커·리뷰어2
# (5노드)가 화면에 뜨는 것. 종전엔 "각성한 마스터가 cys boot 실행"이 산문 계약이라 LLM이 건너뛰면
# (부서장 단독 대기 환각) 팀이 안 떴다 — 그 호출 자체를 코드 결정론(이 훅)으로 격상한다.
#
# 메커니즘: UserPromptSubmit은 프롬프트 제출 시 하네스가 강제 실행하는 훅이다(모델 우회 불가).
# 마스터 선언 감지 시 javis_bootstrap.py(preflight[비치명]→master 등록→cys boot 팀 기동→생존확인)를
# 백그라운드로 발화한다. env 상속 → 부서 pane이면 CYS_SOCKET=부서소켓으로 그 부서 데몬 대상.
#
# ★2회 성찰(적대검증+30년차 아키텍트) 반영:
#  - role-aware 게이트: 워커·CSO·리뷰어 pane에서 "너는 마스터다"(인용·과제문 포함)를 받아도 마스터
#    부트를 오발화하지 않는다(role-blind 결합 결함 수리·arch#1). 미claim(빈)·master pane만 발화.
#  - 감지 정밀화: 토큰 사이 filler 허용("너는 이제 마스터다")·너가 추가·로/명령형 어미·인용/의문
#    오발화 억제·부정 범위 축소(adv#2/7/8).
#  - 중복 억제: python 싱글플라이트 락(flock)이 단일 소유 — 훅은 dumb trigger(증분1). 종전 소켓별
#    PIDF 진행-가드는 제거(실패로 죽은 부트가 재시도를 막던 결함·adv#3/9; 락은 프로세스 종료 시 자동 해제).
#  - 출력: hookSpecificOutput.additionalContext JSON(팩 javis_memory_inject.py 관례·adv#6).
#  - 발화 폴백: setsid→nohup→& (adv#12).
#
# ★T-0147-7 W1a 반영:
#  - A2 surface 이중 게이트: cys 밖(VS Code 등 임의 claude 세션)에서의 오발화 차단. cys pane 안
#    직접 타이핑은 CYS_SURFACE_ID가 있어 통과한다(T-0147-1 레거시 경로와 정합).
#  - A6 cygpath 변환 + 발화 생존확인: '발화됨' 상수 약속을 **상태 파생 보고**로 교체(허위 금지).
#  - A16/R3 런별 로그: truncate(`>`)로 선행 런 원인이 소실되던 것을 런별 파일+latest 포인터+
#    개수 상한으로. 레인 공유 LOG의 상호 truncate(R3)도 동시 소멸.
#  - G22 프리루드 source(CYS_PY·경로 정규화·로케일·상태 경로 단일 소유).
#
# 안전: 모든 단계 graceful, 반드시 exit 0 (훅 실패가 세션을 깨지 않게).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(role-bootstrap)" >&2; exit 0; }

# ── A2: surface 이중 게이트(최선두) — 비-cys 터미널은 무발화·무부작용 ──
# 종전엔 게이트가 없어 임의 claude 세션에서 "너는 마스터다"를 치면 preflight 변형·데몬 autostart·
# boot-last 오염이 일어났다. session-start.sh:10에는 있던 게이트를 이 훅에도 세운다.
cys_require_surface

INPUT=$(cat 2>/dev/null)
[ -z "$INPUT" ] && exit 0
# 인터프리터: 프리루드가 python3→python→py로 해소. 이 훅의 기존 계약(비어 있으면 안 됨)은
# 자기 자리에서 명시 폴백한다 — 계약 무변경.
[ -n "$CYS_PY" ] || CYS_PY="python3"

# 프롬프트 추출(JSON stdin의 prompt 필드)
PROMPT=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('prompt',''))
except Exception: print('')" 2>/dev/null)
[ -z "$PROMPT" ] && exit 0

# ── role-aware 게이트(arch#1): 이 pane의 데몬 권위 역할이 비-마스터면 오발화 금지 ──
# 워커/CSO/리뷰어 pane이 "너는 마스터다"를 포함한 프롬프트(위임 과제·인용·이 성찰문 자체)를 받아도
# 마스터 부트를 발화하면 안 된다. cys surface-role은 CYS_SURFACE_ID로 자기 surface 역할을 반환(미claim=빈).
MYROLE="$(cys surface-role 2>/dev/null | head -1 | tr -d '[:space:]')"
case "$MYROLE" in
  worker|cso|reviewer-*|reviewer) exit 0 ;;   # 비-마스터 pane — 마스터 부트 금지
esac

# ── 마스터 선언 감지 ──
# 첫 200자만 검사(선언은 프롬프트 앞머리·긴 문서 본문 오발화 억제). trim.
HEAD="$(printf '%s' "$PROMPT" | tr '\n' ' ' | cut -c1-200)"
# 의문/인용 오발화 억제(adv#8): "'너는 마스터다'가 무슨 뜻?" 류.
echo "$HEAD" | grep -Eq '(무슨|무엇|뜻|의미|가 뭐|가 무|\?|라고 (말|하지|입력)|처럼|예시|예를)' && exit 0
# 선언 감지: 너는/넌/너가/당신은/너 + (filler 최대 15자 — 아래 `.{0,15}`가 진실원천) + 마스터/master
# + 종결(다/야/이다/입니다/임/이야/로 각성/로 승격/가 되/가 돼/가 된다). you are ... master(영문).
# 부정은 선언 인접만 억제(adv#7).
FIRE=0
if echo "$HEAD" | grep -Eiq '(너는|넌|너가|당신은|너).{0,15}(마스터|master).{0,2}(다|야|이다|입니다|임|이야|여|로 *각성|로 *승격|가 *되|가 *돼|가 *된)'; then FIRE=1; fi
if echo "$HEAD" | grep -Eiq 'you[[:space:]]+are[[:space:]]+(the[[:space:]]+|our[[:space:]]+|now[[:space:]]+)*master'; then FIRE=1; fi
# 부정 인접 억제: "너는 마스터가 아니다/말고" (선언 자리 자체가 부정).
echo "$HEAD" | grep -Eq '(마스터|master)[^가-힣A-Za-z]{0,3}(가|는|를)?[^가-힣A-Za-z]{0,3}(아니|아냐|말고)' && FIRE=0
[ "$FIRE" = 1 ] || exit 0

PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
BOOT="$PACK/bin/javis_bootstrap.py"
# ★BOOT 부재 명시 실패(증분1): 부서 팩에 javis_bootstrap.py가 없는 레인은 종전엔 조용한 무산이라
# "팀이 뜬다"는 기대와 달리 아무 일도 없었다. 원인·조치를 additionalContext로 명시하고 승인 채널로도
# 시끄럽게 알린다. 알림은 백그라운드+graceful(데몬 부재 등 실패가 훅을 죽이거나 행 걸지 않게). 훅은 exit 0.
if [ ! -f "$BOOT" ]; then
  MSG="[부트스트랩 불가] 이 레인의 팩($PACK)에 bin/javis_bootstrap.py가 없어 마스터 팀을 기동할 수 없습니다. 팩 배포(preflight --fix·pack-heal)를 확인하거나 CYS_PACK_DIR이 올바른 레인을 가리키는지 점검하세요."
  ( cys feed push --kind bootstrap-fail --title "부트스트랩 불가(BOOT 부재)" --body "$MSG" >/dev/null 2>&1 \
    || cys send --queued --to master "$MSG" >/dev/null 2>&1 ) &
  "$CYS_PY" -c 'import json,sys
print(json.dumps({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":sys.argv[1]}}, ensure_ascii=False))' \
    "[결정론 부트스트랩 불가 — 명시 실패] 이 레인의 팩에 bin/javis_bootstrap.py가 없어 마스터 팀 기동을 발화할 수 없습니다(조용한 무산 아님). 조치: 팩 배포 상태(preflight --fix·pack-heal)와 CYS_PACK_DIR 레인 정합을 확인하세요. 승인 Feed에도 알림을 시도했습니다."
  exit 0
fi

# ── 중복 억제는 python 싱글플라이트 락이 단일 소유(증분1): 종전의 PIDF 진행-가드·SOCK_KEY는 제거했다.
# 훅은 dumb trigger로 강등한다 — 중복 발화가 있어도 javis_bootstrap.py가 락 비획득 시 no-op(exit 0)
# 으로 안전 종료하므로 이중 방어(락+PIDF)는 불필요하고, PIDF는 실패로 죽은 부트가 재시도를 막던 결함이
# 있었다. ★W1a A8py: 그 락은 이제 javis_lock(posix flock / windows msvcrt / pidfile+스테일 회수)
# 이라 Windows에서도 실효한다(종전 fcntl 단독은 windows에서 '항상 획득'으로 접혔다).
#
# ── A16/R3: 발화 로그를 **런별 파일 + latest 포인터 + 개수 상한**으로 ──
# 종전 `>"$STATE/role-bootstrap.log"` 는 매 발화가 선행 런의 로그를 truncate해 실패 원인을 소실시켰고
# (A16), 전 레인이 같은 파일을 쓰는 비대칭(락은 레인별·LOG는 공유)으로 base·부서 동시 부트가 서로를
# truncate했다(R3). 런별 파일은 두 결함을 동시에 없앤다. 상태 경로는 프리루드의 CYS_STATE_DIR
# (HOME 부재 시 USERPROFILE backfill — A17 훅면).
STATE="$CYS_STATE_DIR"; mkdir -p "$STATE" 2>/dev/null
_EPOCH="$(date +%s 2>/dev/null || echo 0)"
LOG="$STATE/role-bootstrap-$_EPOCH-$$.log"
LATEST="$STATE/role-bootstrap-latest.log"

# 개수 상한(**신규 런 포함** 최근 10개 유지) — 단조 증가 차단. mtime 역순으로 기존 9개만 남기고
# 삭제한다(여기서 `-gt`를 쓰면 신규 파일까지 11개가 되어 상한이 1개씩 새는 off-by-one이 된다).
_KEEP=10; _N=0
for _f in $(ls -1t "$STATE"/role-bootstrap-[0-9]*.log 2>/dev/null); do
  _N=$((_N + 1))
  [ "$_N" -ge "$_KEEP" ] && rm -f "$_f" 2>/dev/null
done

# ── A6: 경로 네이티브 변환(cygpath 규약 — inject-context.sh:38 선례) ──
# Windows(PortableGit sh)의 네이티브 python은 POSIX 경로(/c/...)를 열지 못한다. CYS_PACK_DIR가
# 네이티브로 주입된 주류 pane 경로에서는 무변환이 정답이고, `$HOME/.cys/pack` 폴백 경로에서만
# 변환이 필요하다 — cys_native_path가 양쪽을 한 규약으로 처리한다(unix는 무변환).
BOOT_N="$(cys_native_path "$BOOT")"

# ── 발화 전 실행 가능성 검증(A6 — 상태 파생 보고의 전제) ──
FIRE_FAIL=""
command -v "$CYS_PY" >/dev/null 2>&1 || [ -x "$CYS_PY" ] || FIRE_FAIL="인터프리터 미해소($CYS_PY)"

# 결정론 부트스트랩 백그라운드 발화(env 상속). 부모(claude) 종료와 무관하게 완주.
BOOT_PID=""
if [ -z "$FIRE_FAIL" ]; then
  if command -v setsid >/dev/null 2>&1; then
    setsid "$CYS_PY" "$BOOT_N" >"$LOG" 2>&1 &
    BOOT_PID=$!
  elif command -v nohup >/dev/null 2>&1; then
    nohup "$CYS_PY" "$BOOT_N" >"$LOG" 2>&1 &
    BOOT_PID=$!
  else
    "$CYS_PY" "$BOOT_N" >"$LOG" 2>&1 &
    BOOT_PID=$!
    disown 2>/dev/null
  fi
  # latest 포인터(심링크 우선·미지원 환경은 경로를 담은 평문 파일로 폴백).
  ln -sf "$LOG" "$LATEST" 2>/dev/null || printf '%s\n' "$LOG" > "$LATEST" 2>/dev/null

  # ── A6: 발화 직후 생존확인 — '발화됨'은 상수 약속이 아니라 관측 파생이다 ──
  # 잠깐 기다린 뒤 pid가 살아 있으면 발화 성공. 죽어 있으면 exit code로 판정한다:
  #   0     = 정상 조기 종료(싱글플라이트 skip 등) → 발화 자체는 성공
  #   비0   = exec/인터프리터/경로 실패(127·126 등) → **발화 실패**로 보고
  # 죽었는데 rc를 못 얻는 경우(setsid 중간 fork 등)는 실패로 단정하지 않는다 —
  # 허위 '발화됨'을 막는 것이 목적이고, 허위 '실패'를 만드는 것은 반대 방향 결함이다.
  sleep 0.3 2>/dev/null || sleep 1
  if ! kill -0 "$BOOT_PID" 2>/dev/null; then
    wait "$BOOT_PID" 2>/dev/null; BOOT_RC=$?
    [ "$BOOT_RC" = 0 ] || FIRE_FAIL="발화 프로세스 즉시 종료(exit $BOOT_RC)"
  fi
fi

# LLM 컨텍스트 주입(hookSpecificOutput.additionalContext JSON — 팩 관례) — 재실행/환각 차단.
# ★상태 파생(CS-3①): 성공/실패 문안이 관측 결과에서 갈린다. 실패 경로는 '발화됨'을 절대 말하지 않고
#   런 로그 경로를 안내한다.
if [ -n "$FIRE_FAIL" ]; then
  ( cys feed push --kind bootstrap-fail --title "부트스트랩 발화 실패" \
      --body "role-bootstrap 훅이 javis_bootstrap.py 발화에 실패했습니다($FIRE_FAIL). 로그: $LOG" >/dev/null 2>&1 \
    || cys send --queued --to master "[부트 발화 실패] $FIRE_FAIL — 로그: $LOG" >/dev/null 2>&1 ) &
  "$CYS_PY" -c 'import json,sys
note=("[결정론 부트스트랩 발화 실패 — 상태 파생 보고] \"너는 마스터다\" 선언은 감지했으나 "
      "javis_bootstrap.py 발화가 실패했다(사유: %s). 팀은 뜨지 않았다 — 부트가 시작됐다고 "
      "보고하지 마라(성공 문구 인용 금지). "
      "원인은 발화 로그 %s (최근 런 포인터: %s)와 ~/.cys/state/boot-last.json 에 있다. "
      "조치: ①python 인터프리터 해소 여부 ②팩 경로(CYS_PACK_DIR) 정합 ③위 로그의 "
      "첫 오류 줄을 그대로 오너에게 보고. 승인 Feed에도 알림을 시도했다.") % (sys.argv[1], sys.argv[2], sys.argv[3])
print(json.dumps({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":note}}, ensure_ascii=False))' \
    "$FIRE_FAIL" "$LOG" "$LATEST"
  exit 0
fi

"$CYS_PY" -c 'import json,sys
note=("[결정론 부트스트랩 발화됨 — 하네스 강제] \"너는 마스터다\" 선언을 UserPromptSubmit 훅이 감지해 "
      "javis_bootstrap.py를 백그라운드로 실행 중이다(발화 후 프로세스 생존 확인됨): preflight(비치명) → "
      "master 역할 등록 → cys boot(CSO·워커·리뷰어2 팀 기동) → 생존확인(최대 120s). "
      "완료 = master·cso·worker·reviewer×2 (5노드)가 "
      "화면에 뜨는 것. 지침: 너(LLM)는 이 부트스트랩을 재실행하지 마라(훅이 이미 결정론 집행 중). "
      "\"부서장은 단독 대기\" 같은 규칙은 존재하지 않는다(환각 금지) — 모든 마스터는 팀을 갖는다"
      "(단, ④-c 분기: 부서 레인은 CEO 티켓 부재 시 단독 각성으로 강등되는 것이 정상이다 — 팀 미기동은 "
      "실패가 아니며 boot-last.json의 solo_awakening으로 확인한다. \"팀을 갖는다\"는 티켓 발급이 전제다). "
      "cys list로 팀 기동을 확인하고, 완료되면 오너 지시를 받아 지휘하라. 만약 팀이 안 뜨면 원인이 "
      "이번 런 로그 %s (최근 런 포인터: %s)·boot-last.json에 있고 실패 시 승인 Feed에 알림이 뜬다.") % (sys.argv[1], sys.argv[2])
print(json.dumps({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":note}}, ensure_ascii=False))' \
  "$LOG" "$LATEST"
exit 0
