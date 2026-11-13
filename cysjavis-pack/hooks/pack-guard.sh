#!/bin/bash
# ★W-C1 pack-guard(커스텀 생존 설계 2026-07-17) — PostToolUse(Write|Edit|MultiEdit):
#   vendor(system·임베드) 팩 파일 수정 감지 → kept-drift 보존 고지 + 벤더 전진 시 자동 병합 예고 + 정식 영속 경로 안내.
# 채널: additionalContext(모델 주입) — commit-memory-nudge.sh 와 동일한 검증된 패턴.
# 경계: 오너 Rejected "오버레이 BLOCK 게이트(자기발화 봉쇄)" 준수 — 어떤 실패에도 exit 0(비차단).
# 판정 SOT: `cys pack-ownership --quiet`(임베드 여부 포함 effective 등급) — sh 재구현 금지(SOT 분산 차단).
# 코얼레싱: 세션·파일당 1회만 경고(경고 피로 → 무시 학습 방지).
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(pack-guard)" >&2; exit 0; }
command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"
# G22: 인터프리터 경성 참조 제거. 미해소면 판정 재료를 못 얻으므로 조용히 통과(기존 계약).
[ -n "$CYS_PY" ] || exit 0

INPUT=$(cat 2>/dev/null)
FP=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try:
  ti = json.load(sys.stdin).get('tool_input', {})
  print(ti.get('file_path', '') or ti.get('path', ''))
except Exception:
  print('')" 2>/dev/null | tr -d '\r')
# ★CR 제거(2026-08-10 Windows 실기 H-WIN-5): 네이티브 python \r\n — 경로 꼬리 CR 차단(unix 무변).
[ -z "$FP" ] && exit 0

PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
# ── G23: 팩 접두 판정 정규화 (Windows 백슬래시 미매칭 → vendor 수정 경고 무음) ──
# 종전 `case "$FP" in "$PACK"/*)` 는 tool_input.file_path 가 `C:\Users\x\.cys\pack\hooks\x.sh`
# 인데 PACK 이 `C:/Users/x/.cys/pack` (또는 반대)면 문자열이 안 맞아 **경고 자체가 안 났다**.
# 정규화 후 접두 비교(프리루드 cys_path_has_prefix)로 양쪽 표기를 흡수한다.
cys_path_has_prefix "$FP" "$PACK" || exit 0
_FP_N="$(cys_norm_path "$FP")"; _PACK_N="$(cys_norm_path "$PACK")"
_PACK_N="${_PACK_N%/}"
REL="${_FP_N#"$_PACK_N"/}"

# 세션·파일당 1회 코얼레싱 스탬프.
SID=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('session_id', 'nosession'))
except Exception: print('nosession')" 2>/dev/null | tr -d '\r')
STAMP_DIR="${TMPDIR:-/tmp}/cys-pack-guard"
mkdir -p "$STAMP_DIR" 2>/dev/null
KEY=$(printf '%s' "$REL" | tr '/. ' '___')
STAMP="$STAMP_DIR/${SID:-nosession}-${KEY}"
[ -e "$STAMP" ] && exit 0

# effective 등급 판정(임베드 vendor system 만 경고 대상 — 자작 신규 파일 'custom' 은 불가침이라 침묵).
OWN=$(cys pack-ownership --quiet "$REL" 2>/dev/null)
[ "$OWN" = "system" ] || exit 0
: > "$STAMP" 2>/dev/null

MSG="[pack-guard] '$REL' 은 vendor(system) 파일 — 이 수정은 보존됩니다(kept-drift). 다음 벤더 업데이트 때 자동 병합되며, 충돌하면 내 수정본이 살아 있고 vendor 본은 $REL.new 로 보관돼 워커가 합칩니다(기동 필수 파일은 vendor 본이 먼저 서고 내 수정본은 보관돼 워커가 승인 절차로 다시 입힙니다). 보안 잠금 파일(trusted-keys.json)은 예외 — 즉시 vendor 본으로 치유($REL.user 백업). 영속 경로: ① 자작 기능은 새 파일로(비임베드=업데이트 불가침) ② 스킬 커스텀은 ~/.cys/local/skills(shadowing, cys pack-merge --to-local) ③ 「스스로 해결」 수리는 워커가 한다 — 고치기 전에 cys repair begin 으로 시작하고(고친 뒤에 부른 begin 은 고친 내용을 원본으로 보관한다) 기동 필수·보호장치 관련 파일은 begin 이 알려 주는 작업 사본에서 고쳐 cys repair submit 으로 최종 승인을 받는다(수리 장부 = 업데이트 뒤 다시 입히기·되돌리기의 근거). (WARN — 차단 아님)"

# ── ★v2 §9 이중 활성 방지 안내(사건표 스킬 4건 처분 — 조건부 동승) ──
# skills/ rel 에 동명 로컬 오버레이(~/.cys/local/skills/<skill>)가 실재하면 스킬 색인
# shadowing(동명 = local 승리)으로 이 팩 사본 수정은 침묵 비활성 — 오버레이 실재 시에만 1줄 고지.
# 경로 유도는 Rust local_dir() 규약 동형: CYS_LOCAL_DIR > pack 형제 local(정규화 경로 기준).
case "$REL" in
  skills/*/*)
    _SKILL_DIR="${REL#skills/}"; _SKILL_DIR="${_SKILL_DIR%%/*}"
    _LOCAL_ROOT="${CYS_LOCAL_DIR:-$(dirname "$_PACK_N")/local}"
    if [ -e "$_LOCAL_ROOT/skills/$_SKILL_DIR" ]; then
      MSG="$MSG ★동명 로컬 오버레이($_LOCAL_ROOT/skills/$_SKILL_DIR)가 실재 — shadowing 으로 로컬이 이겨 이 팩 사본 수정은 비활성입니다(활성 반영은 로컬 오버레이 쪽에서)."
    fi
    ;;
  # ★F10-B(0.14.50): hooks/ 아래 vendor 파일의 수정 — 보존 사실 + 수리 장부 경로 + 영속 경로(로컬 훅 폴더)를 고지(0.15.0 M2).
  #   문장의 단일 출처는 src/pack.rs 의 HOOKS_LOCAL_GUIDANCE(<rel> 만 $REL 로 치환) — 낱말을 바꾸면 test_pack_guard_hooks_guidance.py 가 붉어진다. 차단 없음.
  hooks/*)
    MSG="$MSG hooks/ 아래 vendor 파일의 수정은 제자리에 보존됩니다. 「스스로 해결」 수리라면 워커가 고치기 전에 cys repair begin --files $REL --trigger <계기>(owner-order · grant · update 중 하나) 로 시작해 원본을 보관한 뒤 고치고 cys repair record 로 수리 장부에 남깁니다(장부가 업데이트 뒤 다시 입히기·되돌리기·감사의 근거) — 기동 필수·보호장치 관련 훅은 살아 있는 파일이 아니라 begin 이 알려 주는 작업 사본 repairs/R-<n>/staging/ 에서 고치고, 리뷰어1의 검증과 리뷰어2의 감사 서명을 받은 뒤 cys repair submit 으로 수정 최종 승인을 받습니다. 따로 두고 싶은 자기 훅은 ~/.cys/local/hooks/<이벤트>.d/ 에 둡니다(업데이트·치유 불가침)."
    ;;
esac

printf '%s' "$MSG" | "$CYS_PY" -c "import json,sys
print(json.dumps({'hookSpecificOutput':{'hookEventName':'PostToolUse','additionalContext':sys.stdin.read()}}, ensure_ascii=False))" 2>/dev/null
exit 0
