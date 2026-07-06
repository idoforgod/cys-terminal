#!/bin/sh
# IME 계측 플래그 시딩 — cys.app 종료 상태에서 실행 (게이트 뒤 설치 절차용).
# WKWebView localStorage(sqlite)에 cysImeDebug=1 을 심어, 다음 기동부터 모든 pane이
# 이벤트 시퀀스를 $TMPDIR/cys-ime.log 로 기록하게 한다 (main.ts는 pane 생성 시 플래그를 읽음).
# 끄기: 인자로 off 전달 (키 삭제).
set -e
DB=$(find "$HOME/Library/WebKit/com.cysjavis.terminal" -name "localstorage.sqlite3" -path "*LocalStorage*" | head -1)
[ -n "$DB" ] || { echo "localstorage.sqlite3 없음 — cys.app을 한 번이라도 실행한 적 있어야 함"; exit 1; }
if pgrep -qf "cys.app/Contents/MacOS/cys-app"; then
  echo "⚠ cys.app 실행 중 — 종료 후 실행해야 반영됨(실행 중 편집은 종료 시 덮어써짐)"; exit 1
fi
if [ "$1" = "off" ]; then
  sqlite3 "$DB" "DELETE FROM ItemTable WHERE key='cysImeDebug';"
  echo "cysImeDebug 제거 완료: $DB"
else
  # value는 UTF-16LE BLOB ("1" = X'3100')
  sqlite3 "$DB" "INSERT OR REPLACE INTO ItemTable(key, value) VALUES ('cysImeDebug', X'3100');"
  echo "cysImeDebug=1 시딩 완료: $DB"
fi
