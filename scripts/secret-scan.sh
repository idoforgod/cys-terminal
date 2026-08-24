#!/usr/bin/env bash
# secret-scan.sh — PUBLIC repo 발행 전 시크릿/개인정보 fail-closed 게이트 (오너 2026-06-14).
# '전부 올리기'를 안전하게 유지하는 가드레일. 제네릭화 회귀(개인경로·계정·프로필·토큰·이메일)를 차단한다.
# deny-by-default: 의심 패턴이 하나라도 걸리면 비-0으로 차단한다(통과 입증 책임은 산출물에 있다).
#
# 사용:
#   scripts/secret-scan.sh             # staged 파일 스캔 (pre-commit 용)
#   scripts/secret-scan.sh --all       # 추적 파일 전수 스캔 (sync-pack가 호출)
#   scripts/secret-scan.sh <path>...   # 지정 파일 스캔
#   pre-commit 설치: ln -sf ../../scripts/secret-scan.sh .git/hooks/pre-commit
#
# 한계(정직): 정적 패턴 매칭이다 — 난독화된 시크릿·신종 토큰 형식·이미지 내 텍스트는 못 잡는다.
#            이는 회귀 방지 1차선이지 완전한 비밀유출 방어가 아니다(근본 한계 명문화).
# exit 0=clean / 1=발견(차단) / 2=인자·환경 오류.
set -euo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "git repo 아님"; exit 2; }

mode="${1:-staged}"
files=()
case "$mode" in
  --all)      while IFS= read -r f; do files+=("$f"); done < <(git ls-files) ;;
  --staged|staged|"") while IFS= read -r f; do files+=("$f"); done \
                  < <(git diff --cached --name-only --diff-filter=ACM) ;;
  *)          files=("$@") ;;
esac
[ "${#files[@]}" -gt 0 ] || { echo "✓ secret-scan: 스캔 대상 없음"; exit 0; }

# 스캔 제외(노이즈·바이너리·잠금파일): 시크릿이 살지 않고 오탐만 만드는 파일들
# 스캐너 자신과 형제 스캐너(scan-pack-secrets.sh)는 제외 — 둘 다 자기 패턴/정책 정의에 /Users/cys·
# 토큰 형식·개인 핸들(ysfuture)이 리터럴로 들어 자기-오탐을 만든다(린터 관례)
skip_re='\.(lock|png|jpe?g|gif|ico|svg|woff2?|ttf|wasm|pdf|zip|dmg|msi|exe)$|(^|/)Cargo\.lock$|(^|/)LICENSE$|(^|/)secret-scan\.sh$|(^|/)scan-pack-secrets\.sh$'
# 더미 username(제네릭화된 테스트 픽스처) — 그 외 /Users/<name>은 개인경로로 차단.
# ★이름 목록은 **한 벌**이다(`dummy_names`). POSIX 판과 Windows 판이 각자 목록을 들면
#   한쪽만 늘어나 같은 이름이 한 OS 에서만 통과하는 비대칭이 생긴다.
dummy_names='user|x|youruser|USERNAME|runner|home'
dummy_user_re='/Users/('"$dummy_names"')(/|"|$)'
# Windows 홈의 더미 판 — `C:\Users\x\…`·`C:\Users\x>`. 경계는 '이름 문자가 아닌 것'으로 본다
# (뒤에 `\`·`>`·공백·따옴표 등 무엇이 오든 이름 자체가 더미면 통과).
win_dummy_user_re='[A-Za-z]:\\+Users\\+('"$dummy_names"')([^A-Za-z0-9._-]|$)'
# 이메일 허용(공개 연락처가 의도적으로 박힌 배포 문서만 — SECURITY.md 취약점 신고 연락처 포함)
email_allow_re='^(README\.md|README\.en\.md|SECURITY\.md)$'
email_fp_re='example\.(com|org|net)|noreply|@types/|@google/|@tauri|@scope|user@host|you@'
# 개인 계정 핸들 denylist(맨몸) — /Users·.claude- 접두 없이 계정키·설정값으로 박힌 개인 핸들도 차단한다.
# 넓은 패턴 대신 '알려진 개인 핸들'만 명시 등재해 제네릭 영어단어 오탐을 배제한다(deny-by-default 유지).
# ysfuture = 오너 개인 alias·이메일 prefix. 부분일치라 'claude-ysfuture'·'ysfuture@…'도 함께 걸린다.
# cys-macbook = 오너 macOS/Windows 기계 계정명(2026-08-24 실측 누출 3파일 22곳). 경로 접두 없이
#   pane 제목 꼬리(`cso-claude · cys-macbook`)·프롬프트 검체로도 박혀 있어 규칙 1·1′만으로는 안 걸린다.
# ★브랜드 `cysinsight` 는 여기 넣지 않는다 — LICENSE·README·홈페이지 URL 에 실린 공개 식별자다.
handle_deny_re='ysfuture|cys-macbook'

findings="$(mktemp)"; trap 'rm -f "$findings"' EXIT

for f in "${files[@]}"; do
  [ -f "$f" ] || continue
  printf '%s' "$f" | grep -qE "$skip_re" && continue

  # 1) 개인 절대경로 (/Users/<실명>) — 더미 제외
  grep -nE '/Users/[A-Za-z0-9._-]+' "$f" 2>/dev/null | grep -vE "$dummy_user_re" \
    | sed "s|^|PATH\t$f:|" >> "$findings" || true
  # 1') 개인 절대경로 Windows 판 (`C:\Users\<실명>`) — 규칙 1의 역슬래시 거울. 더미 제외.
  #     사각이었다: 규칙 1의 `/Users/` 는 정슬래시 전용이라 `C:\Users\<실명>` 을 **한 건도** 못 봤다
  #     (2026-08-24 실측 — 오너 계정명이 Windows 경로 형태로 문서·검체에 살아 있었다).
  #     이름 첫 글자를 영숫자로 못박아 문서의 생략 표기(`C:\Users\...`)를 오탐하지 않는다.
  grep -nE '[A-Za-z]:\\+Users\\+[A-Za-z0-9][A-Za-z0-9._-]*' "$f" 2>/dev/null | grep -vE "$win_dummy_user_re" \
    | sed "s|^|WIN-PATH\t$f:|" >> "$findings" || true
  # 2) 개인 프로필/홈 디렉터리명 (제네릭화 대상) — ★구분자 무관(`/Users/cys`·`C:\Users\cys` 둘 다).
  #    종전 `/Users/cys` 는 정슬래시 전용이라 Windows 표기의 같은 계정을 통과시켰다(사각 ②).
  grep -nE '\.claude-(ysfuture|cysinsight|cysfuturist)|[/\\]Users[/\\]+cys' "$f" 2>/dev/null \
    | sed "s|^|PROFILE\t$f:|" >> "$findings" || true
  # 3) 이메일 (허용 문서·오탐 제외)
  if ! printf '%s' "$f" | grep -qE "$email_allow_re"; then
    grep -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.(com|net|org|io|dev)' "$f" 2>/dev/null \
      | grep -vEi "$email_fp_re" | sed "s|^|EMAIL\t$f:|" >> "$findings" || true
  fi
  # 4) 자격증명/토큰/개인키. 일반 keyword 규칙은 *따옴표 친 리터럴 값*만(>=12자) 매칭한다 —
  #    'api_key = resolve_api_key()' 같은 함수호출·변수참조(따옴표 없음) 오탐을 배제한다.
  grep -nE 'sk-ant-[A-Za-z0-9]|sk-[A-Za-z0-9]{20}|ghp_[A-Za-z0-9]{10}|github_pat_[A-Za-z0-9]|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9]|-----BEGIN [A-Z ]*PRIVATE KEY-----|(password|passwd|secret|api[_-]?key|access[_-]?token)["'"'"' ]*[:=][ ]*["'"'"'][A-Za-z0-9/+=_-]{12,}' "$f" 2>/dev/null \
    | sed "s|^|SECRET\t$f:|" >> "$findings" || true
  # 5) 개인 계정 핸들(맨몸 denylist) — 접두(/Users·.claude-) 없이 계정키로 박혀도 차단(규칙2 보강)
  grep -nE "$handle_deny_re" "$f" 2>/dev/null \
    | sed "s|^|HANDLE\t$f:|" >> "$findings" || true
done

n=$(wc -l < "$findings" | tr -d ' ')
if [ "$n" -gt 0 ]; then
  echo "✗ secret-scan: $n 건 발견 — PUBLIC 발행 차단(fail-closed):"
  sed -E 's/([A-Za-z0-9/+_-]{20,})/***REDACTED***/g' "$findings" | head -40
  [ "$n" -gt 40 ] && echo "  …(외 $((n-40))건)"
  echo "→ 개인경로/프로필은 환경변수·더미값으로, 시크릿은 제거 후 재시도하라."
  exit 1
fi
echo "✓ secret-scan: clean (mode=$mode, ${#files[@]} 파일)"
exit 0
