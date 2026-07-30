#!/bin/sh
# owner-active.sh — UserPromptSubmit 등록용 (설계 §2 개시 안전 게이트 3 '오너 존재 신호' · 구현물 4b)
#
# 설치 위치: ~/.cys/local/hooks/owner-active.sh (settings.json UserPromptSubmit 배열에 가산 등록)
#
# 역할: 사람이 실제로 타이핑한 프롬프트가 도착하면 $PACK/round/OWNER_ACTIVE 를 touch 한다.
#   컴포넌트 1의 개시 게이트가 이 파일의 mtime 이 10분 이내면 사이클을 skip 한다(오너 작업 중 개입 금지).
#
# 기계 push 제외 휴리스틱: 프롬프트 첫 글자가 '[' 이면 기계 발신으로 보고 무동작한다.
#   근거(실측): 워커·스케줄·훅이 master stdin 으로 밀어넣는 문구는 전부 라벨 접두다
#   ("[heartbeat] …", "[자동·컨텍스트 자기보고] …", "[CYCLE-PRE] …", "[EVT v2] …").
#   ★한계(설계 §6 잔여리스크 1): 오너가 '['로 시작하는 문장을 직접 치면 미감지 → 하위 방어는
#   idle_secs 게이트와 타이핑 가드다. 이 휴리스틱은 보수적으로 '기계로 오인'하는 방향이라
#   실패해도 사이클이 더 자주 도는 게 아니라 오너 보호가 약해지는 쪽임을 문서화한다.
#
# 안전: **무출력**(UserPromptSubmit 의 stdout 은 모델 컨텍스트로 주입된다 — 오염 금지)·항상 exit 0.
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(owner-active)" >&2; exit 0; }
set +e
[ -n "$CYS_STATE_LEDGER_DISABLE" ] && exit 0

IN=$(cat 2>/dev/null)
[ -n "$IN" ] || exit 0

# G22: 인터프리터 해소는 프리루드 단일 소유(python3→python→py). 사본 재구현 금지(RC4).
PY="${CYS_PY:-}"
[ -n "$PY" ] || exit 0

# 선두 공백을 제거한 뒤 첫 글자를 본다(설계의 '첫 문자' 를 공백 내성으로 확장 — IMPL_NOTES 기재).
VERDICT=$(printf '%s' "$IN" | "$PY" -c 'import json,sys
try:
    d = json.load(sys.stdin)
    p = (d.get("prompt") or "") if isinstance(d, dict) else ""
except Exception:
    p = ""
p = p.lstrip()
head = p[:400]
machine = (
    p.startswith("[") or p.startswith("<")  # 라벨 push · XML 래퍼(task-notification, system-reminder, command-name)
    or "task-notification" in head or "SYSTEM NOTIFICATION" in head or "system-reminder" in head
)
print("MACHINE" if machine else ("OWNER" if p else "EMPTY"))' 2>/dev/null)

[ "$VERDICT" = "OWNER" ] || exit 0

D="${CYS_PACK_DIR:-$HOME/.cys/pack}/round"
[ -d "$D" ] || mkdir -p "$D" 2>/dev/null || exit 0
F="$D/OWNER_ACTIVE"
if command -v touch >/dev/null 2>&1; then
  touch "$F" 2>/dev/null
else
  : > "$F" 2>/dev/null
fi
exit 0
