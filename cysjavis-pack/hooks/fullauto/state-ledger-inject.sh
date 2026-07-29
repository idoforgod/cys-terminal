#!/bin/sh
# state-ledger-inject.sh — SessionStart 등록용 (설계 §2 컴포넌트 3 '복원 주입' · 구현물 4b)
#
# 설치 위치: ~/.cys/local/hooks/state-ledger-inject.sh  (오버레이 .d 디렉터리가 **아니다** —
#   cys-hook.sh 러너는 `<이벤트>.d/*.sh` 만 글롭하므로 평범한 파일은 이중 실행되지 않는다.
#   SessionStart 에는 오버레이 채널이 없어 settings.json 에 직접 가산 등록한다. INSTALL.md 참조)
#
# 역할: 원장 최근 12건을 **필드 화이트리스트 재구성**(verbatim 금지)으로 stdout 방출.
#   SessionStart 훅의 stdout 은 세션 컨텍스트로 주입된다(팩 inject-context.sh 와 동일 채널).
#   렌더는 javis_state_ledger.py 가 2048B 로 캡하고, 여기서 head -c 2048 로 한 번 더 조인다.
#
# 안전: 실패 시 **무출력 exit 0**(복원 생명선을 깨지 않는다). 항상 exit 0.
set +e
[ -n "$CYS_STATE_LEDGER_DISABLE" ] && exit 0

# 훅 JSON stdin 은 사용하지 않지만 소비한다(쓰기측 EPIPE 회피).
cat >/dev/null 2>&1

PY="$(command -v python3 || command -v python || command -v py)"
[ -n "$PY" ] || exit 0
# 스크립트 해소(팩 갱신 면역): 명시 env > 팩 bin > 사용자 로컬 bin. 셋 다 없으면 무동작.
BIN="${CYS_STATE_LEDGER_BIN:-}"
if [ -z "$BIN" ] || [ ! -f "$BIN" ]; then
  BIN="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_state_ledger.py"
  [ -f "$BIN" ] || BIN="${CYS_LOCAL_DIR:-$HOME/.cys/local}/bin/javis_state_ledger.py"
fi
[ -f "$BIN" ] || exit 0

TO=""
if command -v timeout >/dev/null 2>&1; then TO="timeout 3"
elif command -v gtimeout >/dev/null 2>&1; then TO="gtimeout 3"; fi

OUT=$($TO "$PY" "$BIN" tail --n 12 --render 2>/dev/null)
[ -n "$OUT" ] || exit 0
printf '%s' "$OUT" | head -c 2048
printf '\n'
exit 0
