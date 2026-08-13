#!/bin/bash
# javis 영속성 — PreCompact·Stop 시 작업기억 write-ahead hook
# 설계: _round/PERSISTENCE_ARCHITECTURE.md §6 G1
# 역할: 압축·종료 직전 SESSION_STATE '최종 갱신' 타임스탬프 갱신 + .state_log append.
#        (본문은 master가 채운 것 보존 — 자동화는 타임스탬프·로그 세이프가드만)
# 경로: ROOT는 환경변수 CYS_ROOT로 오버라이드 가능(미설정 시 $HOME). cwd 상향탐색이 우선 동작.
# 안전: graceful, 반드시 exit 0
set +e

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(save-state)" >&2; exit 0; }

INPUT=$(cat 2>/dev/null)
# 인터프리터 해소는 프리루드(python3→python→py) — 기존 계약(비어 있으면 안 됨)은 명시 폴백.
[ -n "$CYS_PY" ] || CYS_PY="python3"
# ★CR 제거(2026-08-10 Windows 실기 H-WIN-5): 네이티브 python 은 파이프에도 \r\n 을 쓴다 —
#   꼬리 CR 이 cwd 상향탐색·이벤트 완전일치를 무너뜨린다(unix 무변 · inject-context 동일 규약).
CWD=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try: print(json.load(sys.stdin).get('cwd',''))
except Exception: print('')" 2>/dev/null | tr -d '\r')
# G19: 드라이브/UNC 경로 정규화 후 절대경로 판정(Windows `C:\` cwd 공란화 해소·POSIX 무변경)
CWD="$(cys_norm_cwd "$CWD")"
cys_is_abs "$CWD" || CWD=""   # 절대경로만 상향탐색 (무한루프 방지)
EVENT=$(printf '%s' "$INPUT" | "$CYS_PY" -c "import json,sys
try:
 d=json.load(sys.stdin); print(d.get('hook_event_name', d.get('trigger','event')))
except Exception: print('event')" 2>/dev/null | tr -d '\r')

ROOT="${CYS_ROOT:-$HOME}"
DIR="$CWD"; RD=""; PREV=""
while [ -n "$DIR" ] && [ "$DIR" != "/" ] && [ "$DIR" != "$PREV" ]; do
  if [ -d "$DIR/_round" ]; then RD="$DIR/_round"; break; fi
  PREV="$DIR"
  DIR=$(dirname "$DIR")
done
if [ -z "$RD" ] && [ -f "$ROOT/_round/ACTIVE_PROJECT" ]; then
  AP=$(head -1 "$ROOT/_round/ACTIVE_PROJECT" 2>/dev/null)
  [ -n "$AP" ] && [ -d "$AP/_round" ] && RD="$AP/_round"
fi
[ -z "$RD" ] && exit 0

NOW=$(date -Iseconds 2>/dev/null || date)
# append-only 로그 (write-ahead 증거)
echo "$NOW	$EVENT	cwd=$CWD" >> "$RD/.state_log" 2>/dev/null

# SESSION_STATE '최종 갱신' 타임스탬프 갱신 — 압축 직전(PreCompact)에만 (Stop은 로그만 = git noise 방지)
SS="$RD/SESSION_STATE.md"
if [ "$EVENT" = "PreCompact" ] && [ -f "$SS" ] && grep -q "최종 갱신:" "$SS" 2>/dev/null; then
  # 치환문 삽입 전 delimiter(#)·백슬래시·& 이스케이프 — NOW·EVENT 값 오염이 sed 치환식을 깨지 않게(H-HOOK).
  NOW_E=$(printf '%s' "$NOW" | sed 's/[#\\&]/\\&/g')
  EVENT_E=$(printf '%s' "$EVENT" | sed 's/[#\\&]/\\&/g')
  sed -i '' "s#^\(> *최종 갱신:\).*#\1 ${NOW_E} (auto write-ahead: ${EVENT_E})#" "$SS" 2>/dev/null \
    || sed -i "s#^\(> *최종 갱신:\).*#\1 ${NOW_E} (auto write-ahead: ${EVENT_E})#" "$SS" 2>/dev/null
fi

# ---------- (A) SESSION_STATE 비대 워치 (스냅샷 규율을 기계가 감시 — 백서 §6) ----------
# SESSION_STATE는 '덮어쓰기 스냅샷'이어야 한다. 임계 초과 = 완료항목 미정리 신호 → 로그에 WARN만(자동삭제 금지).
SS_MAX=8192
if [ -f "$SS" ]; then
  SZ=$(wc -c < "$SS" 2>/dev/null | tr -d ' ')
  if [ -n "$SZ" ] && [ "$SZ" -gt "$SS_MAX" ]; then
    echo "$NOW	WARN:bloat	SESSION_STATE=${SZ}B>${SS_MAX} — 완료항목 정리(스냅샷 유지) 필요" >> "$RD/.state_log" 2>/dev/null
  fi
fi

# ---------- (B) .state_log 회전 (append-only 무한증가 차단) ----------
LOG="$RD/.state_log"; LOG_MAX=1000; LOG_KEEP=200
if [ -f "$LOG" ]; then
  LC=$(wc -l < "$LOG" 2>/dev/null | tr -d ' ')
  if [ -n "$LC" ] && [ "$LC" -gt "$LOG_MAX" ]; then
    mkdir -p "$RD/archive" 2>/dev/null
    head -n $((LC - LOG_KEEP)) "$LOG" >> "$RD/archive/state_log_archive" 2>/dev/null
    tail -n "$LOG_KEEP" "$LOG" > "$LOG.tmp" 2>/dev/null && mv "$LOG.tmp" "$LOG" 2>/dev/null
    echo "$NOW	rotate	.state_log>${LOG_MAX} → archive (kept ${LOG_KEEP})" >> "$LOG" 2>/dev/null
  fi
fi

# ---------- (C) BOOT_SNAPSHOT 생성 (관측 전용 · 마스터 pane 한정 · 실패 무해 · W-수리1 배선) ----------
# 도구 부재 시 -f 가드로 조용히 생략 · 비마스터 skip 판정은 도구 내부 게이트(javis_snapshot.py) 소관.
# 어떤 실패(타임아웃 124·미설치 127 포함)도 exit 0 보존 — 기존 write-ahead 동작 불변.
# Windows(PortableGit sh) 패리티: 네이티브 python3는 POSIX 경로(/c/...)를 못 연다 — cygpath 변환(inject-context CHK·GATE 동일 규약).
PACK_BIN="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin"
SNAP_PY="$PACK_BIN/javis_snapshot.py"
command -v cygpath >/dev/null 2>&1 && SNAP_PY="$(cygpath -w "$SNAP_PY" 2>/dev/null || printf '%s' "$SNAP_PY")"
if [ -n "$RD" ] && [ -f "$SNAP_PY" ]; then
  cys_timeout_run 10 "$CYS_PY" "$SNAP_PY" generate --round-dir "$RD" >/dev/null 2>&1 || true
fi
exit 0
