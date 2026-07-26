#!/bin/bash
# pack-guard — 팩 쓰기에 대한 **두 개의 독립 게이트**. 등록 이벤트 2종(stdin의 hook_event_name으로 분기):
#   PreToolUse(Write|Edit|MultiEdit)  → 게이트 A만
#   PostToolUse(Write|Edit|MultiEdit) → 게이트 A + 게이트 B
#
# ── 게이트 A (CU-2A · 2026-07-26) — 교차-scope 팩 쓰기 ────────────────────────────────────────
#   "남의 scope 팩"(부서 pane → 본사 팩, 본사 → 부서 팩 …)에 쓰는 **경계 침범**을 판정한다.
#   판정 SOT = `cys pack-scope-check --path <실경로>`(JSON: verdict·own_scope·target_scope·suggest)
#   — sh 재구현 금지. 모드 = $CYS_PACK_DIR/state/scope-guard.mode (`log`|`deny`, 부재·미지=log).
#   log = additionalContext 경고 + state/scope-guard.log jsonl 적재(오탐 실측 자료).
#   deny = PreToolUse JSON permissionDecision:"deny"(ADR-3 — exit code로 차단하지 않는다).
#
# ── 게이트 B (W-C1 커스텀 생존 설계 2026-07-17 · 원안 그대로) — 자기 팩 vendor 파일 경고 ────────
#   vendor(system·임베드) 팩 파일 수정 감지 → "다음 부트 치유" 예고 + 정식 영속 경로 안내.
#   채널: additionalContext(모델 주입) — commit-memory-nudge.sh 와 동일한 검증된 패턴.
#   경계: 오너 Rejected "오버레이 BLOCK 게이트(자기발화 봉쇄)" 준수 — 어떤 실패에도 exit 0(비차단).
#   판정 SOT: `cys pack-ownership --quiet`(임베드 여부 포함 effective 등급) — sh 재구현 금지(SOT 분산 차단).
#   코얼레싱: 세션·파일당 1회만 경고(경고 피로 → 무시 학습 방지).
#
# ★ADR-2 — 두 게이트는 **별개**다. 오너가 과거 Rejected 한 것은 게이트 B의 BLOCK 승격(자기 팩
#   vendor 수정 차단 = 자기발화 봉쇄 위험)이고, 게이트 A는 "남의 scope 팩 쓰기"라는 다른 의미의
#   경계 침범을 다룬다(오너 별도 승인: log-only → 차단 2단계). 그래서 아래 게이트 B 구현은
#   **바이트 불변**으로 보존하고, 게이트 A를 그 앞단에 얹었다.
# ★INV-1 — 위반이 **확정**된 경우(verdict=cross-scope ∧ mode=deny ∧ PreToolUse) 외의 어떤 내부
#   상태(cys 부재·python3 부재·깨진 stdin·JSON 파싱 실패·타임아웃·데몬 무응답)도 도구 실행을
#   막지 않는다. 전 경로 exit 0 · set +e 유지.
set +e

INPUT=$(cat 2>/dev/null)
FP=$(printf '%s' "$INPUT" | python3 -c "import json,sys
try:
  ti = json.load(sys.stdin).get('tool_input', {})
  print(ti.get('file_path', '') or ti.get('path', ''))
except Exception:
  print('')" 2>/dev/null)
[ -z "$FP" ] && exit 0

# ══════════════ 게이트 A — 교차-scope 팩 쓰기 (CU-2A) ══════════════
# 메타 3종을 한 번의 python 호출로 회수(핫패스 spawn 최소화): 이벤트명·세션·실경로(심링크 해소).
# python3 부재·stdin 파손이면 전부 빈 값 → 게이트 A는 조용히 통과(fail-open).
GA_META=$(printf '%s' "$INPUT" | python3 -c "import json,os,sys
try:
  d = json.load(sys.stdin)
except Exception:
  d = {}
if not isinstance(d, dict):
  d = {}
ti = d.get('tool_input') or {}
if not isinstance(ti, dict):
  ti = {}
p = ti.get('file_path', '') or ti.get('path', '')
print(str(d.get('hook_event_name', '') or ''))
print(str(d.get('session_id', '') or 'nosession'))
print(os.path.realpath(p) if p else '')
print(os.path.realpath(os.path.expanduser('~')))" 2>/dev/null)
GA_EVT=$(printf '%s\n' "$GA_META" | sed -n 1p)
GA_SID=$(printf '%s\n' "$GA_META" | sed -n 2p)
GA_RP=$(printf '%s\n' "$GA_META" | sed -n 3p)
GA_HOME=$(printf '%s\n' "$GA_META" | sed -n 4p)
OWN_PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"

# 대상은 `$HOME/.cys/` 바로 아래 **pack 계열**(pack·pack-dept-*·pack-ceo·pack.prev …)뿐이다.
# 레포 워크트리·프로젝트 소스는 여기서 걸러진다(개발 무간섭 계약).
# ★HOME 자체도 realpath로 정규화한다 — 대상 경로만 해소하면 심링크 HOME(macOS /var→/private/var,
#   NFS 홈 등)에서 접두 매칭이 통째로 빗나가 게이트가 조용히 무력화된다.
GA_CROSS=0
[ -n "$GA_HOME" ] && case "$GA_RP" in
  "$GA_HOME"/.cys/pack*/*) GA_CROSS=1 ;;
esac
# cys 부재(fresh 기계) = 판정 불가 = 통과.
[ "$GA_CROSS" = 1 ] && ! command -v cys >/dev/null 2>&1 && GA_CROSS=0

if [ "$GA_CROSS" = 1 ]; then
  if command -v timeout >/dev/null 2>&1; then
    GA_JSON=$(timeout 5 cys pack-scope-check --path "$GA_RP" 2>/dev/null)
  else
    GA_JSON=$(cys pack-scope-check --path "$GA_RP" 2>/dev/null)
  fi
  # 계약 4필드 + authority(선택 — 데몬 무응답 폴백 표식) 를 줄 단위로 평탄화. 파싱 실패=빈 줄=통과.
  GA_P=$(printf '%s' "$GA_JSON" | python3 -c "import json,sys
try:
  d = json.load(sys.stdin)
except Exception:
  d = {}
if not isinstance(d, dict):
  d = {}
for k in ('verdict', 'own_scope', 'target_scope', 'suggest', 'authority'):
  v = d.get(k, '')
  print(str(v).replace('\n', ' ') if v is not None else '')" 2>/dev/null)
  GA_VERDICT=$(printf '%s\n' "$GA_P" | sed -n 1p)
  GA_OWN=$(printf '%s\n' "$GA_P" | sed -n 2p)
  GA_TGT=$(printf '%s\n' "$GA_P" | sed -n 3p)
  GA_SUGGEST=$(printf '%s\n' "$GA_P" | sed -n 4p)
  GA_AUTH=$(printf '%s\n' "$GA_P" | sed -n 5p)

  if [ "$GA_VERDICT" = "cross-scope" ]; then
    GA_MODE=$(cat "$OWN_PACK/state/scope-guard.mode" 2>/dev/null | tr -d ' \t\r\n')
    case "$GA_MODE" in
      deny|log) ;;
      *) GA_MODE="log" ;;                     # 부재·오타·파손 = 안전측(log)
    esac
    # 권위 보수화: pack-scope-check 가 로컬 폴백으로 판정했다고 신고하면(데몬 무응답) deny 강등.
    # 필드 부재(구 CLI)=데몬 권위 가정=현행. 판정 권위가 불확실할 때 차단하지 않는다(SIM-3 교훈).
    [ -n "$GA_AUTH" ] && [ "$GA_AUTH" != "daemon" ] && GA_MODE="log"
    [ -z "$GA_SUGGEST" ] && GA_SUGGEST="cys todo-path (역할·scope에 맞는 경로를 데몬 권위로 산출)"

    # 원장 적재(모드 무관 — 오탐 실측·deny 승격 결재 자료). 실패해도 무시.
    mkdir -p "$OWN_PACK/state" 2>/dev/null
    python3 -c "import json,os,sys,time
p = sys.argv[1]
rec = {'ts': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'session': sys.argv[2], 'event': sys.argv[3],
       'path': sys.argv[4], 'own_scope': sys.argv[5], 'target_scope': sys.argv[6], 'mode': sys.argv[7]}
try:
  with open(p, 'a', encoding='utf-8') as f:
    f.write(json.dumps(rec, ensure_ascii=False) + '\n')
except Exception:
  pass" "$OWN_PACK/state/scope-guard.log" "${GA_SID:-nosession}" "${GA_EVT:-unknown}" \
       "$GA_RP" "$GA_OWN" "$GA_TGT" "$GA_MODE" 2>/dev/null

    if [ "$GA_MODE" = "deny" ] && [ "$GA_EVT" = "PreToolUse" ]; then
      # ADR-3: 차단은 JSON 결정 채널로만. 프로세스 exit 는 그래도 0.
      GA_REASON="[pack-guard] 교차-scope 팩 쓰기 차단: 이 노드의 scope는 '$GA_OWN' 인데 대상 경로는 '$GA_TGT' 팩입니다 ($GA_RP). 남의 scope 팩에 직접 쓰지 마세요 — 자기 scope의 정식 경로는 \`$GA_SUGGEST\` 로 산출하고, 다른 scope에 전달할 내용은 cys send 로 그 scope의 노드에 위임하세요. (오탐이면 $OWN_PACK/state/scope-guard.mode 를 log 로 되돌리면 즉시 경고 전용으로 복귀합니다)"
      printf '%s' "$GA_REASON" | python3 -c "import json,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':'PreToolUse','permissionDecision':'deny','permissionDecisionReason':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
      exit 0
    fi

    GA_MSG="[pack-guard] 교차-scope 팩 쓰기 감지(WARN — 차단 아님): 이 노드의 scope='$GA_OWN' / 대상='$GA_TGT' ($GA_RP). 자기 scope 경로는 \`$GA_SUGGEST\` 로 얻고, 다른 scope의 파일은 그 scope 노드에 위임하세요. 이 경고는 $OWN_PACK/state/scope-guard.log 에 적재됩니다(오탐 실측용)."
    printf '%s' "$GA_MSG" | GA_EVT="${GA_EVT:-PostToolUse}" python3 -c "import json,os,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':os.environ.get('GA_EVT','PostToolUse'),'additionalContext':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
    exit 0
  fi
fi

# 게이트 B는 PostToolUse 전용(사후 경고). PreToolUse 등록분은 여기서 종료.
# 이벤트명 미상(구 하네스·파싱 실패)은 종전 동작 보존을 위해 게이트 B 로 진행한다.
[ "$GA_EVT" = "PreToolUse" ] && exit 0

# ══════════════ 게이트 B — 자기 팩 vendor 경고 (원안 바이트 불변 · ADR-2) ══════════════
PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
case "$FP" in
  "$PACK"/*) ;;
  *) exit 0 ;;
esac
REL="${FP#"$PACK"/}"

# 세션·파일당 1회 코얼레싱 스탬프.
SID=$(printf '%s' "$INPUT" | python3 -c "import json,sys
try: print(json.load(sys.stdin).get('session_id', 'nosession'))
except Exception: print('nosession')" 2>/dev/null)
STAMP_DIR="${TMPDIR:-/tmp}/cys-pack-guard"
mkdir -p "$STAMP_DIR" 2>/dev/null
KEY=$(printf '%s' "$REL" | tr '/. ' '___')
STAMP="$STAMP_DIR/${SID:-nosession}-${KEY}"
[ -e "$STAMP" ] && exit 0

# effective 등급 판정(임베드 vendor system 만 경고 대상 — 자작 신규 파일 'custom' 은 불가침이라 침묵).
OWN=$(cys pack-ownership --quiet "$REL" 2>/dev/null)
[ "$OWN" = "system" ] || exit 0
: > "$STAMP" 2>/dev/null

MSG="[pack-guard] '$REL' 은 vendor(system) 파일 — 이 수정은 다음 부트 설치 스윕에 vendor 본으로 치유됩니다(수정본은 $REL.user 로 보존·병합 원장 기록). 영속 경로: ① 자작 기능은 새 파일로(비임베드=업데이트 불가침) ② 스킬 커스텀은 ~/.cys/local/skills(shadowing, cys pack-merge --to-local) ③ vendor 개선 제안은 cys pack-merge --file $REL --propose. (WARN — 차단 아님·개발 기계의 upstream 승격 작업이면 무시)"

printf '%s' "$MSG" | python3 -c "import json,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':'PostToolUse','additionalContext':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
exit 0
