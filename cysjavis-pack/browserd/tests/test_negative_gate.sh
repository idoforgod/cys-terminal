#!/usr/bin/env bash
# test_negative_gate.sh — 음성(negative) 게이트 E2E.
# 결함 fixture에서 verify가 exit 5(FAIL), 정상 fixture에서 exit 0(PASS),
# evidence 번들 4파일(screenshot.png·snapshot.txt·dom.html·meta.json) 실재를 assert.
#
# browserd는 headless로 기동(창 포커스 훔침 방지). 실제 Chrome 채널 우선, 폴백 chromium.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
PACK="$(cd "$HERE/../.." && pwd)"
CLI="$PACK/bin/javis_browser.py"
FIX="$HERE/fixtures"
export CYS_BROWSER_HEADLESS=1
export CYS_ROLE="worker-test"

WORK="$(mktemp -d)"
# ★evidence 는 봉인된 루트(~/.cys/browser/evidence) 하위만 허용된다(P0-D③) — mktemp 경로는
# EVIDENCE_PATH_DENIED(18)로 거부된다. 실행마다 고유 하위 디렉터리를 쓰고 종료 시 지운다.
EVID="$HOME/.cys/browser/evidence/negative-gate-$$"
FAILED=0

log(){ printf '\n=== %s ===\n' "$1"; }
fail(){ printf 'ASSERT FAIL: %s\n' "$1"; FAILED=1; }

cleanup(){
  python3 "$CLI" stop >/dev/null 2>&1 || true
  [ -n "${HTTP_PID:-}" ] && kill "$HTTP_PID" 2>/dev/null || true
  rm -rf "$WORK"
  [ -n "${EVID:-}" ] && rm -rf "$EVID"
}
trap cleanup EXIT

# --- fixture 서빙 ---
log "python http.server 기동"
# 빈 포트를 결정론으로 선점 후 명시 기동 (stderr 로그 파싱 의존 제거)
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
( cd "$FIX" && exec python3 -m http.server "$PORT" --bind 127.0.0.1 ) >"$WORK/http.log" 2>&1 &
HTTP_PID=$!
# 서버가 실제 응답할 때까지 대기
BROKEN_URL="http://127.0.0.1:$PORT/broken.html"
OK_URL="http://127.0.0.1:$PORT/ok.html"
READY=0
for _ in $(seq 1 50); do
  if python3 -c "import urllib.request,sys;urllib.request.urlopen('$OK_URL',timeout=1)" 2>/dev/null; then READY=1; break; fi
  sleep 0.1
done
if [ "$READY" -ne 1 ]; then echo "http.server 응답 실패 (port $PORT)"; cat "$WORK/http.log"; exit 1; fi
echo "serving on port $PORT"

# --- doctor ---
log "doctor"
python3 "$CLI" doctor
DOC=$?
[ "$DOC" -eq 0 ] || fail "doctor exit $DOC (기대 0)"

# --- 결함 페이지: verify FAIL(exit 5) 기대 ---
log "결함 페이지 open + verify (기대 exit 5)"
python3 "$CLI" --headless open "$BROKEN_URL" >/dev/null
python3 "$CLI" --headless verify --expect-text "PAYMENT CONFIRMED"
RC_BROKEN=$?
echo "broken verify exit=$RC_BROKEN"
[ "$RC_BROKEN" -eq 5 ] || fail "결함 페이지 verify exit=$RC_BROKEN (기대 5=FAIL)"

# --- 다중 기대값: 2개 중 1개만 존재 → FAIL(exit 5) (Fix①: append 침묵 덮어씀 방지) ---
log "정상 페이지 open + verify 다중 기대값(1 존재·1 부재 → 기대 exit 5)"
python3 "$CLI" --headless open "$OK_URL" >/dev/null
MULTI_OUT="$(python3 "$CLI" --headless verify --expect-text "PAYMENT CONFIRMED" --expect-text "REFUND ISSUED")"
RC_MULTI=$?
echo "multi verify exit=$RC_MULTI"
[ "$RC_MULTI" -eq 5 ] || fail "다중 기대값(1부재) verify exit=$RC_MULTI (기대 5=FAIL — 마지막 값만 검증되면 오통과)"
# reasons에 존재·부재 항목이 둘 다 기록되는지(전부 대조 증거)
echo "$MULTI_OUT" | grep -q "expect_text 확인" || fail "multi verify: 존재 항목 reason 누락"
echo "$MULTI_OUT" | grep -q "expect_text 미발견" || fail "multi verify: 부재 항목 reason 누락"

# --- 다중 기대값: 둘 다 존재 → PASS(exit 0) ---
log "정상 페이지 verify 다중 기대값(2 존재 → 기대 exit 0)"
python3 "$CLI" --headless verify --expect-text "PAYMENT CONFIRMED" --expect-text "주문 상태" >/dev/null
RC_MULTI_OK=$?
[ "$RC_MULTI_OK" -eq 0 ] || fail "다중 기대값(2존재) verify exit=$RC_MULTI_OK (기대 0=PASS)"

# --- 정상 페이지: verify PASS(exit 0) + evidence 번들 ---
log "정상 페이지 open + verify --evidence-dir (기대 exit 0 + 4파일)"
python3 "$CLI" --headless open "$OK_URL" >/dev/null
python3 "$CLI" --headless verify --expect-text "PAYMENT CONFIRMED" --evidence-dir "$EVID"
RC_OK=$?
echo "ok verify exit=$RC_OK"
[ "$RC_OK" -eq 0 ] || fail "정상 페이지 verify exit=$RC_OK (기대 0=PASS)"

log "evidence 번들 4파일 실재 assert"
for f in screenshot.png snapshot.txt dom.html meta.json; do
  if [ -s "$EVID/$f" ]; then echo "  [OK] $f ($(wc -c <"$EVID/$f" | tr -d ' ') bytes)"; else fail "evidence 파일 없음/빈: $f"; fi
done

# meta.json 완결 마커 + dom_sha256 독립 재계산 대조
if [ -s "$EVID/meta.json" ] && [ -s "$EVID/dom.html" ]; then
  META_HASH="$(python3 -c "import json,sys;print(json.load(open('$EVID/meta.json'))['dom_sha256'])")"
  CALC_HASH="$(python3 -c "import hashlib;print(hashlib.sha256(open('$EVID/dom.html','rb').read()).hexdigest())")"
  if [ "$META_HASH" = "$CALC_HASH" ]; then echo "  [OK] dom_sha256 독립 재계산 일치"; else fail "dom_sha256 불일치 meta=$META_HASH calc=$CALC_HASH"; fi
fi

# --- F4: evidence_dir 재사용 세대혼합 방지 ---
# 재사용 dir에 이전 세대(gen1) 잔재를 심고 2회차(gen2) 정상 실행 → gen1 잔재 0 확인.
# 수정 전이면 gen1 meta.json(완결 마커)/dom.html이 gen2 screenshot과 섞여 세대혼합 번들이 완결로 오판됨.
log "F4: evidence_dir 재사용 세대혼합 방지 ($EVID 재사용)"
# gen1 잔재 심기: meta.json에 sentinel 주입 + dom.html을 다른 내용으로 오염
printf '{"gen":"STALE_GEN1_SENTINEL","dom_sha256":"deadbeef"}' >"$EVID/meta.json"
printf '<html>STALE GEN1 DOM</html>' >"$EVID/dom.html"
# gen2 정상 실행 — 같은 dir 재사용
python3 "$CLI" --headless open "$OK_URL" >/dev/null
python3 "$CLI" --headless verify --expect-text "PAYMENT CONFIRMED" --evidence-dir "$EVID" >/dev/null
RC_F4=$?
[ "$RC_F4" -eq 0 ] || fail "F4 gen2 verify exit=$RC_F4 (기대 0)"
# gen1 sentinel 잔존 0
if grep -q "STALE_GEN1_SENTINEL" "$EVID/meta.json"; then fail "F4: gen1 meta sentinel 잔존(선삭제 실패)"; else echo "  [OK] gen1 meta sentinel 제거됨"; fi
if grep -q "STALE GEN1 DOM" "$EVID/dom.html"; then fail "F4: gen1 dom 잔존(선삭제 실패)"; else echo "  [OK] gen1 dom 제거됨"; fi
# gen2 meta.json은 유효 JSON이고 dom_sha256이 gen2 dom.html과 일치(세대 일관)
F4_META="$(python3 -c "import json;print(json.load(open('$EVID/meta.json'))['dom_sha256'])" 2>/dev/null)"
F4_CALC="$(python3 -c "import hashlib;print(hashlib.sha256(open('$EVID/dom.html','rb').read()).hexdigest())")"
if [ -n "$F4_META" ] && [ "$F4_META" = "$F4_CALC" ]; then echo "  [OK] gen2 meta↔dom 세대 일관"; else fail "F4: gen2 meta/dom 세대 불일치 meta=$F4_META calc=$F4_CALC"; fi

# --- 비신뢰 라벨 확인 (snapshot 헤더) ---
log "snapshot 비신뢰 라벨 확인"
if grep -q "UNTRUSTED WEB CONTENT" "$EVID/snapshot.txt"; then echo "  [OK] 비신뢰 헤더 존재"; else fail "snapshot에 비신뢰 라벨 없음"; fi

# --- 감사로그 기록 확인 ---
log "audit.jsonl 기록 확인"
AUDIT="$HOME/.cys/browser/audit.jsonl"
if [ -s "$AUDIT" ] && grep -q '"verb": "verify"' "$AUDIT"; then echo "  [OK] audit에 verify 기록"; else fail "audit.jsonl에 verify 기록 없음"; fi

log "결과"
if [ "$FAILED" -eq 0 ]; then echo "ALL ASSERTS PASSED (음성 게이트 통과)"; exit 0; else echo "SOME ASSERTS FAILED"; exit 1; fi
