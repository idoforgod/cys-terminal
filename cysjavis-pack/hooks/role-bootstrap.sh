#!/bin/bash
# javis 결정론 부트스트랩 발화 — UserPromptSubmit hook
#
# 절대요구(오너 2026-07-15): "너는 마스터다" 류 마스터 선언이 입력되면, LLM의 재량·환각·누락과
# 무관하게 하네스가 부트스트랩을 100% 예외없이 발화한다. 부트 완료 = **필수 역할 전원+master**가
# 화면에 뜨는 것(구성·개수는 javis_orchestra.team_roster_note 파생 — B18: 리터럴 금지).
# 종전엔 "각성한 마스터가 cys boot 실행"이 산문 계약이라 LLM이 건너뛰면
# (부서장 단독 대기 환각) 팀이 안 떴다 — 그 호출 자체를 코드 결정론(이 훅)으로 격상한다.
#
# 메커니즘: UserPromptSubmit은 프롬프트 제출 시 하네스가 강제 실행하는 훅이다(모델 우회 불가).
# 마스터 선언 감지 시 javis_bootstrap.py(preflight[비치명]→master 등록→cys boot 팀 기동→생존확인)를
# 백그라운드로 발화한다. env 상속 → 부서 pane이면 CYS_SOCKET=부서소켓으로 그 부서 데몬 대상.
#
# ★2회 성찰(적대검증+30년차 아키텍트) 반영:
#  - role-aware 게이트: 워커·CSO·리뷰어 pane에서 "너는 마스터다"(인용·과제문 포함)를 받아도 마스터
#    부트를 오발화하지 않는다(role-blind 결합 결함 수리·arch#1).
#  - 감지 정밀화: 토큰 사이 filler 허용("너는 이제 마스터다")·인용/의문 오발화 억제.
#  - 중복 억제: python 싱글플라이트 락(javis_lock)이 단일 소유 — 훅은 dumb trigger(증분1).
#  - 출력: hookSpecificOutput.additionalContext JSON(팩 javis_memory_inject.py 관례·adv#6).
#  - 발화 폴백: setsid→nohup→& (adv#12).
#
# ★T-0147-7 W1a 반영:
#  - A2 surface 이중 게이트 / A6 cygpath 변환 + 발화 생존확인(상태 파생 보고) /
#    A16·R3 런별 로그(truncate 소멸) / G22 프리루드 source.
#
# ★T-0147-7 W1b 반영(이 파일의 현재 구조를 결정하는 것):
#  - **A4·P3-A-NEGA·G9·G25 → 감지기 전량 이관**: 선언 감지·억제 판정이 더 이상 이 파일에 없다.
#    `bin/javis_detect.py` 단일 함수가 소유하고, 훅은 stdin을 그대로 넘겨 **1왕복**으로 판정만
#    회수한다. 셸 grep 스택(구 :46-56)·`cut -c1-200`(바이트 슬라이스)·`tr '\n' ' '`은 제거됐다.
#  - **A3 allowlist 반전**: 구 denylist(`worker|cso|reviewer-*|reviewer`)는 worker-2·cso-1·
#    reviewer-claude-1·미지 role(verifier 등)을 전부 통과시켰다 → 이제 **master 또는 빈(미claim)
#    만** 발화하고 그 밖 전부(미지 role 포함) 차단한다.
#  - **A5 fail-closed 3상 + 게이트 최선두**: `cys surface-role`을 데드라인(cys_timeout_run)으로
#    감싸고, rc≠0(=판정 불가)은 **무발화+로그**로 처리한다(빈값=미claim 과 분리). 게이트를 비용
#    큰 호출(stdin 판정 왕복) **앞**으로 옮겼다.
#  - **A22 cannot-judge loud**: CYS_PY 미해소(python 전무)·감지기 부재는 조용히 꺼지지 않는다 —
#    프롬프트에 '마스터/master' 토큰이 있을 때만(cannot-judge ↔ judged-no 분리) feed/send로
#    시끄럽게 알리고 additionalContext로 명시한다.
#  - **A7 skip exit 소비**: javis_bootstrap 의 싱글플라이트 패자는 이제 exit 11(정상 skip)이다 —
#    발화 생존확인이 이를 실패로 오판하지 않는다.
#  - **A10 §0 계약 정렬**: NOTE 문안이 '오너 지시 대기'가 아니라 **next-action 자율 착수**를
#    가리킨다(MASTER_DIRECTIVE §0 ⑥·§14 축1과 동일 문장).
#
# 안전: 모든 단계 graceful, 반드시 exit 0 (훅 실패가 세션을 깨지 않게).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(role-bootstrap)" >&2; exit 0; }

# ── A2: surface 이중 게이트(최선두) — 비-cys 터미널은 무발화·무부작용 ──
# 종전엔 게이트가 없어 임의 claude 세션에서 "너는 마스터다"를 치면 preflight 변형·데몬 autostart·
# boot-last 오염이 일어났다. session-start.sh:16에는 있던 게이트를 이 훅에도 세운다.
cys_require_surface

# 인터프리터: 프리루드가 python3→python→py로 해소(미해소=빈 문자열 — cannot-judge 신호).
# ★이 훅은 CYS_PY를 **빈 값으로도** 받는다: 빈 값은 A22 loud 분기의 입력이므로 여기서
#   `|| CYS_PY=python3` 로 채우면 '존재하지 않는 인터프리터'가 해소된 것처럼 보여 판정불가가
#   침묵으로 접힌다(구 계약의 결함). 아래 A22 블록이 유일한 소비자다.

# ── A3(allowlist)+A5(fail-closed 3상): role 게이트를 **최선두**로 ──
# 이 pane의 데몬 권위 역할이 master(또는 미claim)가 아니면 마스터 부트를 발화하지 않는다.
# 종전 구조의 두 결함을 한 번에 고친다:
#   ① 구 denylist(`worker|cso|reviewer-*|reviewer`)는 **열거 밖 전부 통과** — 데몬이 실제로
#      발권하는 worker-2(dedup)·cso-1·reviewer-claude-1(대체)·미지 role(verifier)이 마스터 부트를
#      오발화했다(A3=B7 실측). allowlist 반전이 유일한 구조적 해법이다.
#   ② `cys surface-role`은 데몬 미응답 시 rc0+빈출력으로 오류를 삼킬 수 있고(cys.rs:4740-4757 —
#      3상화는 W2), 무한 대기 표면도 있었다. 여기서는 **데드라인(2s)** 을 씌우고 rc≠0을
#      '판정 불가'로 분리해 **무발화+로그**한다(빈값=미claim 은 정상 통과 — 융합 금지).
#      ※timeout 단독 적용은 hang을 '오발화'로 바꾸는 악화라 3상화와 **동시** 적용해야 한다.
CYS_ROLE_GATE_TIMEOUT_S=2
if command -v cys >/dev/null 2>&1; then
  # ★`</dev/null`: 이 게이트는 훅 stdin(hook JSON)을 읽기 **전**에 돈다 — 자식이 실수로
  #   stdin을 삼키면 아래 `cat`이 굶어 프롬프트 판정이 무음 실패한다.
  MYROLE_RAW="$(cys_timeout_run "$CYS_ROLE_GATE_TIMEOUT_S" cys surface-role </dev/null 2>/dev/null)"
  ROLE_RC=$?
else
  MYROLE_RAW=""; ROLE_RC=127
fi
if [ "$ROLE_RC" -ne 0 ]; then
  # cannot-judge: 데몬 미응답(124=데드라인 초과)·cys 부재(127) 등. 발화하지 않는다 —
  # 남의 pane에서 마스터 부트를 터뜨리는 것이 판정 보류보다 나쁘다(fail-closed).
  echo "[cys-hook] role-bootstrap: surface-role 판정 불가(rc=$ROLE_RC) — 무발화(fail-closed)" >&2
  exit 0
fi
MYROLE="$(printf '%s' "$MYROLE_RAW" | head -1 | tr -d '[:space:]')"
case "$MYROLE" in
  master|"") : ;;   # 발화 허용: master 좌석 또는 미claim(빈) 좌석만
  *)
    echo "[cys-hook] role-bootstrap: 비-master role($MYROLE) — 마스터 부트 발화 금지(allowlist)" >&2
    exit 0 ;;
esac

INPUT=$(cat 2>/dev/null)
[ -z "$INPUT" ] && exit 0

# ── 정적 additionalContext 발행기(python 없이도 동작) ──
# JSON 특수문자(",\,개행)를 **포함하지 않는 문안만** 넘긴다 — 이 경로는 python이 없을 때도
# 써야 하므로 셸 printf가 유일한 수단이다(인용 안전은 호출자의 문안 규율로 보장).
_static_ctx() {
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$1"
}
# 프롬프트가 마스터 선언일 **가능성**이 있는가(cannot-judge ↔ judged-no 분리용 보수 선별).
# 선언은 '마스터' 또는 'master' 토큰 없이 성립하지 않는다(javis_detect.MASTER) — 토큰이 없으면
# 판정기가 없어도 '선언 아님'이 확정이므로 침묵이 정당하다. 토큰이 있으면 판정 불가 =시끄럽게.
# ★외부 명령 0(셸 `case` 글롭만): 이 술어는 **python·감지기가 없는 경로**에서 쓰인다 — 여기서
#   grep 에 의존하면 grep 부재(최소 PATH·복구 셸)가 cannot-judge 를 다시 침묵으로 접는다.
# ★두 인코딩 모두 본다: 훅 stdin JSON 은 리터럴 UTF-8(Node JSON.stringify)로 오는 것이 정상이지만,
#   `ensure_ascii=True` 직렬화기(python json.dumps 기본값)를 거친 입력은 `\ub9c8...` 형태로
#   온다 — 한쪽만 보면 그 경로에서 cannot-judge 가 다시 침묵으로 접힌다(실측으로 확인된 구멍).
_maybe_declaration() {
  case "$INPUT" in
    *마스터*|*master*|*Master*|*MASTER*) return 0 ;;
    *'\ub9c8\uc2a4\ud130'*|*'\uB9C8\uC2A4\uD130'*) return 0 ;;   # JSON \u 이스케이프 '마스터'(대소문자 hex 양쪽)
    *) return 1 ;;
  esac
}
_notify_bg() {   # 승인 채널 best-effort(백그라운드·graceful — 훅을 죽이거나 행 걸지 않게)
  ( cys feed push --kind bootstrap-fail --title "$1" --body "$2" >/dev/null 2>&1 \
    || cys send --queued --to master "[$1] $2" >/dev/null 2>&1 ) &
}

# ── A22: 인터프리터 미해소(python 전무) — cannot-judge 를 시끄럽게 ──
# 종전엔 `[ -n "$CYS_PY" ] || CYS_PY=python3` 로 채워, 존재하지 않는 인터프리터를 향해
# 프롬프트 파싱을 시도하고 조용히 exit 0 했다(판정불가가 '선언 아님'으로 접힘 — A22 클래스).
if [ -z "${CYS_PY:-}" ]; then
  if _maybe_declaration; then
    MSG="python 인터프리터(python3/python/py)를 찾지 못해 마스터 선언 감지를 수행할 수 없습니다. 팀 기동이 발화되지 않았습니다 - python 설치 또는 PATH를 확인하세요."
    _notify_bg "부트스트랩 판정 불가(python 부재)" "$MSG"
    _static_ctx "[결정론 부트스트랩 판정 불가 - python 부재] 이 기계에서 python3/python/py 중 어느 것도 해소되지 않아 마스터 선언 감지 자체가 불가능하다(선언 아님이 아니라 판정 불가다). 팀은 뜨지 않았다 - 부트가 시작됐다고 보고하지 마라. 조치: python 설치 또는 PATH 배선을 확인한 뒤 재선언하라. 승인 Feed에도 알림을 시도했다."
  else
    echo "[cys-hook] role-bootstrap: CYS_PY 미해소 — 선언 토큰 없어 침묵 종료(judged-no)" >&2
  fi
  exit 0
fi

# ── 감지기 해소(CS-8 단일 소유) ──
# 2단 해소: ①형제 팩(`hooks/../bin`) ②CYS_PACK_DIR 레인 팩. 프리루드(_lib.sh)와 **같은 순서**다 —
# 훅이 팩 밖으로 복사돼 실행되는 배선(테스트 스텁·local/hooks 오버레이)에서도 감지기를 집는다.
PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
DETECT="$(dirname "$0")/../bin/javis_detect.py"
[ -f "$DETECT" ] || DETECT="$PACK/bin/javis_detect.py"
if [ ! -f "$DETECT" ]; then
  if _maybe_declaration; then
    MSG="이 레인의 팩($PACK)에 bin/javis_detect.py가 없어 마스터 선언 감지를 수행할 수 없습니다. 팩 배포(preflight --fix - pack-heal)를 확인하세요."
    _notify_bg "부트스트랩 판정 불가(감지기 부재)" "$MSG"
    _static_ctx "[결정론 부트스트랩 판정 불가 - 감지기 부재] 팩에 bin/javis_detect.py가 없어 마스터 선언 감지가 불가능하다(조용한 무산 아님). 팀은 뜨지 않았다 - 부트가 시작됐다고 보고하지 마라. 조치: 팩 배포 상태(preflight --fix - pack-heal)와 CYS_PACK_DIR 레인 정합을 확인하라."
  else
    echo "[cys-hook] role-bootstrap: javis_detect.py 부재 — 선언 토큰 없어 침묵 종료(judged-no)" >&2
  fi
  exit 0
fi

# ── 마스터 선언 감지(1왕복) ──
# stdin(hook JSON)을 그대로 감지기에 넘긴다 — prompt 추출·200자 창(문자)·절 경계 억제·부정 억제가
# 전부 그 안에 있다. exit: 0=발화 / 1=선언 없음(침묵) / 3=선언이지만 억제(감지기가 stderr 1줄) /
# 그 외=판정 불가. ★Windows 네이티브 python 대비 경로 변환은 BOOT와 동일 규약(cys_native_path).
DETECT_VERDICT="$(printf '%s' "$INPUT" | "$CYS_PY" "$(cys_native_path "$DETECT")" hook-gate)"
DETECT_RC=$?
case "$DETECT_RC" in
  0) : ;;                                        # FIRE
  1|3) exit 0 ;;                                 # 선언 없음 / 억제(로그는 감지기가 남겼다)
  *)
    echo "[cys-hook] role-bootstrap: 감지기 판정 불가(rc=$DETECT_RC) — 무발화. verdict=$DETECT_VERDICT" >&2
    if _maybe_declaration; then
      _notify_bg "부트스트랩 판정 불가(감지 실패)" \
        "javis_detect.py가 프롬프트를 판정하지 못했습니다(rc=$DETECT_RC). 팀 기동이 발화되지 않았습니다."
      _static_ctx "[결정론 부트스트랩 판정 불가 - 감지 실패] 마스터 선언 감지기가 판정에 실패했다(입력 파싱 불가 등). 팀은 뜨지 않았다 - 부트가 시작됐다고 보고하지 마라. 조치: 훅 stderr의 detect 로그를 확인하고 필요하면 명시적으로 다시 선언하라."
    fi
    exit 0 ;;
esac

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
# 훅은 dumb trigger로 강등한다 — 중복 발화가 있어도 javis_bootstrap.py가 락 비획득 시 **exit 11**
# (skipped_inflight verdict)로 즉시 안전 종료하므로 이중 방어(락+PIDF)는 불필요하고, PIDF는
# 실패로 죽은 부트가 재시도를 막던 결함이 있었다. ★W1a A8py: 그 락은 이제 javis_lock(posix flock /
# windows msvcrt / pidfile+스테일 회수)이라 Windows에서도 실효한다.
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

# ★G15(W3): boot-last 는 **레인별** 파일이다 — 안내 경로를 하드코딩하면 부서 레인에서 거짓 경로를
#   가리킨다(그 레인의 진단은 boot-last-<lane>.json 에 있다). 경로 규약의 소유자는
#   javis_bootstrap.lane_state_path 하나이고, 훅은 `lane-path` 로 물어본다(사본 0).
LANE_BOOT_LAST="$("$CYS_PY" "$BOOT_N" lane-path boot_last 2>/dev/null | tail -1)"
[ -n "$LANE_BOOT_LAST" ] || LANE_BOOT_LAST="$HOME/.cys/state/boot-last.json(레인 경로 판독 실패 — base 추정)"

# ── 발화 전 실행 가능성 검증(A6 — 상태 파생 보고의 전제) ──
FIRE_FAIL=""
command -v "$CYS_PY" >/dev/null 2>&1 || [ -x "$CYS_PY" ] || FIRE_FAIL="인터프리터 미해소($CYS_PY)"

# 결정론 부트스트랩 백그라운드 발화(env 상속). 부모(claude) 종료와 무관하게 완주.
BOOT_PID=""
if [ -z "$FIRE_FAIL" ]; then
  # ★A18 조건부 내재화(W2 · W1b probe_pgid 실측 확정): 세션 분리를 python 안에서 한 번 더 시도한다.
  #   W1b 실측 = **nohup 분기는 pgid 를 분리하지 않아** 하네스 group-kill 에 부트가 함께 죽는다
  #   (대조군 생존 = 계측 타당). 그래서 발화부가 `--detach-session` 을 넘기고, python 이
  #   **세션 리더 검사 후** os.setsid() 를 부른다(이미 setsid(1) 로 감싼 1순위 분기에서는 no-op —
  #   가드가 없으면 EPERM 으로 훅 경로 전체가 크래시한다).
  #   ★이 인자는 **훅 발화부 전용**이다: MASTER_DIRECTIVE §0 폴백의 포그라운드 직접 실행
  #   (`python3 javis_bootstrap.py`)에는 넘기지 않는다 — 그쪽에 세션 분리를 걸면 호출자가 포기한
  #   뒤에도 스폰이 계속되는 고아를 신설한다(job control 보존).
  BOOT_ARGS="--detach-session"
  if command -v setsid >/dev/null 2>&1; then
    setsid "$CYS_PY" "$BOOT_N" $BOOT_ARGS >"$LOG" 2>&1 &
    BOOT_PID=$!
  elif command -v nohup >/dev/null 2>&1; then
    nohup "$CYS_PY" "$BOOT_N" $BOOT_ARGS >"$LOG" 2>&1 &
    BOOT_PID=$!
  else
    "$CYS_PY" "$BOOT_N" $BOOT_ARGS >"$LOG" 2>&1 &
    BOOT_PID=$!
    disown 2>/dev/null
  fi
  # latest 포인터(심링크 우선·미지원 환경은 경로를 담은 평문 파일로 폴백).
  ln -sf "$LOG" "$LATEST" 2>/dev/null || printf '%s\n' "$LOG" > "$LATEST" 2>/dev/null

  # ── A6: 발화 직후 생존확인 — '발화됨'은 상수 약속이 아니라 관측 파생이다 ──
  # 잠깐 기다린 뒤 pid가 살아 있으면 발화 성공. 죽어 있으면 exit code로 판정한다:
  #   0     = 정상 조기 종료(부트 완주·단독 각성 등) → 발화 자체는 성공
  #   11    = 싱글플라이트 skip(다른 pane이 이미 부트 중 — A7 skipped_inflight) → **정상**
  #   그 외 = exec/인터프리터/경로 실패(127·126 등)·부트 단계 실패 → **발화 실패**로 보고
  # 죽었는데 rc를 못 얻는 경우(setsid 중간 fork 등)는 실패로 단정하지 않는다 —
  # 허위 '발화됨'을 막는 것이 목적이고, 허위 '실패'를 만드는 것은 반대 방향 결함이다.
  # ★11을 성공으로 읽는 것이 A7 타입드 종료의 소비면이다: 구 계약은 skip도 exit 0이라
  #   '정상 조기 종료'와 구분되지 않았고, 구분 exit를 도입하면서 이 소비부를 같이 고치지 않으면
  #   정상 skip이 매번 '발화 실패' 오보로 바뀐다(생산자·소비자 동시 착지 규율).
  sleep 0.3 2>/dev/null || sleep 1
  if ! kill -0 "$BOOT_PID" 2>/dev/null; then
    wait "$BOOT_PID" 2>/dev/null; BOOT_RC=$?
    case "$BOOT_RC" in
      0|11) : ;;
      *) FIRE_FAIL="발화 프로세스 즉시 종료(exit $BOOT_RC)" ;;
    esac
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
      "원인은 발화 로그 %s (최근 런 포인터: %s)와 이 레인의 boot-last(%s)에 있다. "
      "조치: ①python 인터프리터 해소 여부 ②팩 경로(CYS_PACK_DIR) 정합 ③위 로그의 "
      "첫 오류 줄을 그대로 오너에게 보고. 승인 Feed에도 알림을 시도했다."
      ) % (sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4])
print(json.dumps({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":note}}, ensure_ascii=False))' \
    "$FIRE_FAIL" "$LOG" "$LATEST" "$LANE_BOOT_LAST"
  exit 0
fi

# ★A10 §0 단일 계약의 훅면: 이 문안이 곧 "훅 컨텍스트 존재" 신호다 — 모델은 부트를 재실행하지 않고
#   **잔여 의무(③복원·⑤승인채널·⑥보고+next-action 자율 착수)** 만 수행한다. 종전 "오너 지시를 받아
#   지휘하라"는 §0 ⑥('오너 지시 대기'는 폐기 — 앵커6 축1)과 정면 충돌하는 낡은 문구였다.
# ★H-TIME-2(W2): 안내 숫자(생존확인 창)는 **하드코딩하지 않는다** — javis_budget 파생값을 주입한다.
#   종전 "최대 120s" 는 CHECK_RETRIES×CHECK_INTERVAL_S 를 손으로 곱한 사본이라, 예산이 바뀌면
#   문서만 거짓이 됐다(P3-A-120S 의 문서면). 예산 모듈 소비 불가 시에만 파생 실패를 명시한다.
CHECK_WINDOW_S="$("$CYS_PY" "$PACK/bin/javis_budget.py" --note-check-window 2>/dev/null)"
[ -n "$CHECK_WINDOW_S" ] || CHECK_WINDOW_S="예산 모듈 미소비(javis_budget 확인)"
# ★B18/H-DOC-2(W4): 팀 구성·노드 수도 **하드코딩하지 않는다** — 종전 "(5노드)" 리터럴은
#   REQUIRED_ROLES 와 무관하게 늙어 문서만 거짓이 됐다(편성이 바뀌어도 훅 note 는 그대로).
#   `javis_orchestra --note-team-roster`(=REQUIRED_ROLES+master 파생)를 인용한다. required 집합에
#   master 를 추가해 숫자를 맞추는 것은 금지 방향 ②(레거시 master 부트 사망).
TEAM_ROSTER="$("$CYS_PY" "$PACK/bin/javis_orchestra.py" --note-team-roster 2>/dev/null)"
[ -n "$TEAM_ROSTER" ] || TEAM_ROSTER="필수 역할 전원+master(로스터 모듈 미소비 — javis_orchestra 확인)"
"$CYS_PY" -c 'import json,sys
note=("[결정론 부트스트랩 발화됨 — 하네스 강제] \"너는 마스터다\" 선언을 UserPromptSubmit 훅이 감지해 "
      "javis_bootstrap.py를 백그라운드로 실행 중이다(발화 후 프로세스 생존 확인됨): preflight(비치명) → "
      "master 역할 등록 → cys boot(팀 기동) → 생존확인(최대 %ss). "
      "완료 = %s가 "
      "화면에 뜨는 것. 지침: 너(LLM)는 이 부트스트랩을 재실행하지 마라(훅이 이미 결정론 집행 중) — "
      "MASTER_DIRECTIVE §0의 개별 명령(preflight·claim-role·cys boot·check)을 손으로 재현하는 것도 "
      "재실행이다. 너의 잔여 의무는 §0의 ③복원 점검·⑤승인 채널 확보·⑥구동 보고뿐이다. "
      "\"부서장은 단독 대기\" 같은 규칙은 존재하지 않는다(환각 금지) — 모든 마스터는 팀을 갖는다"
      "(단, ④-c 분기: 부서 레인은 CEO 티켓 부재 시 단독 각성으로 강등되는 것이 정상이다 — 팀 미기동은 "
      "실패가 아니며 이 레인 boot-last(%s)의 solo_awakening으로 확인한다. "
      "\"팀을 갖는다\"는 티켓 발급이 전제다). "
      "cys list로 팀 기동을 확인하고, 완료되면 §0 ⑥대로 구동 보고 후 orchestra next-action으로 "
      "다음 액션 큐를 결정론 확인해 미완 작업이 있으면 자율 착수하라(\"오너 지시 대기\"는 폐기). "
      "만약 팀이 안 뜨면 원인이 "
      "이번 런 로그 %s (최근 런 포인터: %s)·이 레인 boot-last에 있고 실패 시 승인 Feed에 알림이 뜬다."
      ) % (sys.argv[3], sys.argv[5], sys.argv[4], sys.argv[1], sys.argv[2])
print(json.dumps({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":note}}, ensure_ascii=False))' \
  "$LOG" "$LATEST" "$CHECK_WINDOW_S" "$LANE_BOOT_LAST" "$TEAM_ROSTER"
exit 0
