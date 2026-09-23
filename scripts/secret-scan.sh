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
# exit 0=clean / 1=발견(차단) / 2=인자·환경 오류 **또는 판정 불가**(아래 U4 C4-⑤ 4종).
#
# ★'못 본 것 = clean' 폐쇄(U4 C4-⑤ · 2026-09-23). 종전에는 아래 네 경로가 전부 `✓` exit 0 이었다:
#   ① 비-git 작업 디렉터리 — `cd "$(git rev-parse …)" || exit 2` 가드가 **죽은 코드**였다(`cd ""` 는
#      bash 에서 성공한다) → `--all` 이 `git ls-files` 실패 → 0건 → '✓ 스캔 대상 없음'.
#   ② `--all` 대상 0건 — 형제 `scan-pack-secrets.sh` 는 같은 상황에서 exit 2 인데 이쪽만 초록(비대칭).
#   ③ 명시 경로 모드의 부재 경로 — `[ -f ] || continue` 가 조용히 건너뛰고 clean.
#   ④ 판독 실패 — `grep … 2>/dev/null … || true` 가 rc=2(읽기 실패·정규식 엔진 오류)까지 삼켰다.
#   이제 넷 다 exit 2(판정 불가)다. staged 모드의 0건(스테이징 없음)만 정상 '대상 없음' 으로 남는다.
#   검체: 부트 건강성 러너의 H-SECRET-2(5축 · 양성 대조 포함 · 러너 파일명을 여기 적지 않는다 —
#   H-SECRET-1 ⓒ 가 이 파일에서 그 이름을 '스캐너 자기 면제' 로 읽는다).
set -euo pipefail
top="$(git rev-parse --show-toplevel 2>/dev/null)" || top=""
if [ -z "$top" ] || ! cd "$top"; then
  echo "✗ secret-scan: git 저장소가 아니다 — 스캔 대상을 정할 수 없다(판정 불가 · exit 2)" >&2
  exit 2
fi

findings="$(mktemp)"; grep_errs="$(mktemp)"; grep_raw="$(mktemp)"; list_file="$(mktemp)"
trap 'rm -f "$findings" "$grep_errs" "$grep_raw" "$list_file"' EXIT

mode="${1:-staged}"
files=()
# ★목록은 NUL 구분(-z)으로 받는다 — 줄 단위는 비ASCII 경로를 `"…\355…"` 로 따옴표 인용해
#   `[ -f ]` 가 거짓이 되고 그 파일이 **조용히** 빠진다(같은 계급). git 실패는 판정 불가다.
case "$mode" in
  --all)
    git ls-files -z > "$list_file" \
      || { echo "✗ secret-scan: git ls-files 실패 — 판정 불가(exit 2)" >&2; exit 2; } ;;
  --staged|staged|"")
    git diff --cached --name-only --diff-filter=ACM -z > "$list_file" \
      || { echo "✗ secret-scan: git diff --cached 실패 — 판정 불가(exit 2)" >&2; exit 2; } ;;
esac
case "$mode" in
  --all|--staged|staged|"") while IFS= read -r -d '' f; do files+=("$f"); done < "$list_file" ;;
  *)          files=("$@") ;;
esac
if [ "${#files[@]}" -eq 0 ]; then
  if [ "$mode" = "--all" ]; then
    echo "✗ secret-scan: --all 인데 추적 파일 0건 — 아무것도 보지 않은 '0건 발견' 은 clean 이 아니다(판정 불가 · exit 2)" >&2
    exit 2
  fi
  echo "✓ secret-scan: 스캔 대상 없음"; exit 0
fi

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
# 이메일 허용 — ★성찰 A·B(minor): 종전에는 README·SECURITY·feedback.rs **파일 전체**를 이메일
#   스캔에서 뺐다(`email_allow_re` 파일 단위 skip). FEEDBACK_TO 한 줄(28행)을 허용하려고
#   1,439행짜리 feedback.rs(테스트 포함) 전체가 면제돼, 앞으로 그 파일에 실수로 들어오는
#   다른 실주소(디버그 프린트·오타 픽스처 등)를 H-SECRET 게이트가 조용히 통과시킨다 — 파일
#   단위 skip 자체가 오탐(false-negative) 확대 경로다. 네 파일 모두 **같은 공개 주소**
#   (cysinsight@gmail.com — README·SECURITY 취약점 신고 연락처 = feedback.rs FEEDBACK_TO,
#   feedbackwiring.test.ts 가 두 문서와 대조)뿐이므로, 파일을 통째로 빼는 대신 **그 주소 하나만**
#   전역 오탐 목록(email_fp_re)에 올린다 — 이 네 파일을 포함해 저장소 어디서든 다른 실주소는
#   그대로 잡힌다(허용 폭이 파일에서 리터럴 주소로 좁아졌다).
email_fp_re='example\.(com|org|net)|noreply|@types/|@google/|@tauri|@scope|user@host|you@|cysinsight@gmail\.com'
# 개인 계정 핸들 denylist(맨몸) — /Users·.claude- 접두 없이 계정키·설정값으로 박힌 개인 핸들도 차단한다.
# 넓은 패턴 대신 '알려진 개인 핸들'만 명시 등재해 제네릭 영어단어 오탐을 배제한다(deny-by-default 유지).
# ysfuture = 오너 개인 alias·이메일 prefix. 부분일치라 'claude-ysfuture'·'ysfuture@…'도 함께 걸린다.
# cys-macbook = 오너 macOS/Windows 기계 계정명(2026-08-24 실측 누출 3파일 22곳). 경로 접두 없이
#   pane 제목 꼬리(`cso-claude · cys-macbook`)·프롬프트 검체로도 박혀 있어 규칙 1·1′만으로는 안 걸린다.
# ★브랜드 `cysinsight` 는 여기 넣지 않는다 — LICENSE·README·홈페이지 URL 에 실린 공개 식별자다.
handle_deny_re='ysfuture|cys-macbook'

# 첫 단 grep(파일 판독) — rc 0=매치 · 1=무매치 · ≥2=판독 실패/정규식 엔진 오류.
#   ≥2 는 버리지 않고 기록한다(끝에서 판정 불가 exit 2). 매치 줄은 stdout 으로 넘겨 기존
#   후단(더미 제외 grep -v · sed 라벨)이 그대로 처리한다 — 규칙 문면은 한 글자도 바꾸지 않았다.
g() {  # $1=파일 · 나머지=grep 인자(옵션·패턴) — 매치 줄은 grep 이 곧장 stdout(파이프)으로 쓴다
  local f="$1" rc=0
  shift
  grep "$@" -- "$f" 2>>"$grep_raw" || rc=$?
  if [ "$rc" -ge 2 ]; then
    printf 'rc=%s\t%s\n' "$rc" "$f" >> "$grep_errs"
  fi
  return 0
}

missing_args=()
scanned=0
for f in "${files[@]}"; do
  if [ ! -f "$f" ]; then
    # 명시 경로 모드의 부재·비정규 경로는 판정 불가다(③). --all/staged 의 부재(작업트리에서
    # 지웠지만 인덱스에는 있는 항목)는 종전대로 건너뛴다 — 그건 스캔할 작업트리 파일이 없는 정상이다.
    case "$mode" in --all|--staged|staged|"") ;; *) missing_args+=("$f") ;; esac
    continue
  fi
  printf '%s' "$f" | grep -qE "$skip_re" && continue
  scanned=$((scanned + 1))

  # 1) 개인 절대경로 (/Users/<실명>) — 더미 제외
  g "$f" -nE '/Users/[A-Za-z0-9._-]+' | grep -vE "$dummy_user_re" \
    | sed "s|^|PATH\t$f:|" >> "$findings" || true
  # 1') 개인 절대경로 Windows 판 (`C:\Users\<실명>`) — 규칙 1의 역슬래시 거울. 더미 제외.
  #     사각이었다: 규칙 1의 `/Users/` 는 정슬래시 전용이라 `C:\Users\<실명>` 을 **한 건도** 못 봤다
  #     (2026-08-24 실측 — 오너 계정명이 Windows 경로 형태로 문서·검체에 살아 있었다).
  #     이름 첫 글자를 영숫자로 못박아 문서의 생략 표기(`C:\Users\...`)를 오탐하지 않는다.
  g "$f" -nE '[A-Za-z]:\\+Users\\+[A-Za-z0-9][A-Za-z0-9._-]*' | grep -vE "$win_dummy_user_re" \
    | sed "s|^|WIN-PATH\t$f:|" >> "$findings" || true
  # 2) 개인 프로필/홈 디렉터리명 (제네릭화 대상) — ★구분자 무관(`/Users/cys`·`C:\Users\cys` 둘 다).
  #    종전 `/Users/cys` 는 정슬래시 전용이라 Windows 표기의 같은 계정을 통과시켰다(사각 ②).
  g "$f" -nE '\.claude-(ysfuture|cysinsight|cysfuturist)|[/\\]Users[/\\]+cys' \
    | sed "s|^|PROFILE\t$f:|" >> "$findings" || true
  # 3) 이메일 (오탐 제외 — 파일 단위 면제 없음, 모든 파일을 같은 규칙으로 스캔)
  g "$f" -nE '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.(com|net|org|io|dev)' \
    | grep -vEi "$email_fp_re" | sed "s|^|EMAIL\t$f:|" >> "$findings" || true
  # 4) 자격증명/토큰/개인키. 일반 keyword 규칙은 *따옴표 친 리터럴 값*만(>=12자) 매칭한다 —
  #    'api_key = resolve_api_key()' 같은 함수호출·변수참조(따옴표 없음) 오탐을 배제한다.
  g "$f" -nE 'sk-ant-[A-Za-z0-9]|sk-[A-Za-z0-9]{20}|ghp_[A-Za-z0-9]{10}|github_pat_[A-Za-z0-9]|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9]|-----BEGIN [A-Z ]*PRIVATE KEY-----|(password|passwd|secret|api[_-]?key|access[_-]?token)["'"'"' ]*[:=][ ]*["'"'"'][A-Za-z0-9/+=_-]{12,}' \
    | sed "s|^|SECRET\t$f:|" >> "$findings" || true
  # 5) 개인 계정 핸들(맨몸 denylist) — 접두(/Users·.claude-) 없이 계정키로 박혀도 차단(규칙2 보강)
  g "$f" -nE "$handle_deny_re" \
    | sed "s|^|HANDLE\t$f:|" >> "$findings" || true
done

# 판정 불가 사유(③ 부재 경로 · ④ 판독 실패)를 먼저 모은다 — 발견(exit 1)이 있으면 그것이 우선이지만
# 사유는 함께 적는다(둘 다 발행 차단이다).
nerr=$(wc -l < "$grep_errs" | tr -d ' ')
unmeasured=0
if [ "${#missing_args[@]}" -gt 0 ]; then
  echo "✗ secret-scan: 명시한 경로 ${#missing_args[@]}건이 없거나 정규 파일이 아니다 — 보지 않은 파일은 clean 이 아니다:" >&2
  printf '  %s\n' "${missing_args[@]}" | head -20 >&2
  unmeasured=1
fi
if [ "$nerr" -gt 0 ]; then
  echo "✗ secret-scan: 판독 실패 ${nerr}건(grep rc≥2 · 읽기 실패·정규식 엔진 오류) — 못 읽은 파일은 검사한 것이 아니다:" >&2
  sort -u "$grep_errs" | head -20 >&2
  head -5 "$grep_raw" | sed 's/^/  grep: /' >&2
  unmeasured=1
fi
if [ "$mode" = "--all" ] && [ "$scanned" -eq 0 ]; then
  echo "✗ secret-scan: --all 인데 실제로 연 파일 0건(목록 ${#files[@]}건 전부 제외·부재) — 판정 불가" >&2
  unmeasured=1
fi

n=$(wc -l < "$findings" | tr -d ' ')
if [ "$n" -gt 0 ]; then
  echo "✗ secret-scan: $n 건 발견 — PUBLIC 발행 차단(fail-closed):"
  sed -E 's/([A-Za-z0-9/+_-]{20,})/***REDACTED***/g' "$findings" | head -40
  [ "$n" -gt 40 ] && echo "  …(외 $((n-40))건)"
  echo "→ 개인경로/프로필은 환경변수·더미값으로, 시크릿은 제거 후 재시도하라."
  exit 1
fi
if [ "$unmeasured" -ne 0 ]; then
  echo "✗ secret-scan: 판정 불가(exit 2) — 발견 0건이지만 위 사유로 **전부 본 것이 아니다**. 통과가 아니다." >&2
  exit 2
fi
echo "✓ secret-scan: clean (mode=$mode, ${#files[@]} 파일)"
exit 0
