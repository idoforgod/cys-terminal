#!/usr/bin/env bash
# restore-runtime-symlinks.sh — .app 안 runtime 트리에서 Tauri 가 역참조한 심볼릭링크를 원상복구한다.
#
# ★근본원인 (dedup-git-core.sh 와 **동일한 upstream 결함**, 다른 피해 부위)
#   Tauri 번들러는 `bundle.resources` 복사 시 심볼릭링크를 역참조(dereference)한다(upstream #13219, 미해결).
#   그래서 소스 `src-tauri/runtime` 에서 심볼릭링크였던 경로가 `.app` 안에서는 **일반 파일 실복사본**이 된다.
#   git-core 빌트인 142개는 dedup-git-core.sh 가 되돌리지만, 그 스크립트는 "git-core/git 과 바이트동일" 인
#   것만 대상으로 삼으므로 **나머지 15개는 그대로 남는다**(2026-08-02 실측, 아래).
#
# ★그 15개 중 3개는 단순 용량 낭비가 아니라 **기능 파손**이다 (node 툴체인 전면 불능)
#   node/bin/npm  -> ../lib/node_modules/npm/bin/npm-cli.js   (54B)
#   node/bin/npx  -> ../lib/node_modules/npm/bin/npx-cli.js   (2921B)
#   node/bin/corepack -> ../lib/node_modules/corepack/dist/corepack.js (174B)
#   이 셋은 링크가 아니라 **런처 스크립트 원본이 그 자리에 복사**된 것이라, 안에 든 상대 require
#   (`require('../lib/cli.js')`)가 realpath(=lib/node_modules/npm/) 가 아니라 **복사된 위치**(bin/)
#   기준으로 풀린다 → `runtime/node/lib/cli.js` 를 찾다 MODULE_NOT_FOUND 로 죽는다.
#   실측(v0.14.10 .app): npm/npx/corepack `--version` 전부 rc=1 + "Cannot find module '../lib/cli.js'".
#   파급: src/lib.rs:197·:827 이 `runtime/node/bin` 을 PATH 에 올리므로 **앱 안의 모든 npm/npx 호출이 실패**한다
#   (prep-mac-runtime.sh:57-59 의 npx 도구 사용, javis_preflight.py:251-252 의 `npx skills add` 안내 포함).
#   회귀가 아니라 **장기 출하 결함** — 배포본 0.13.18·0.14.9 둘 다 동일 파손 실측.
#
# ★나머지 12개는 기능은 살아있고(실측 rc=0) 용량만 낭비한다 — 그래도 같이 되돌린다(원본 레이아웃 충실 + 41.4MB 회수):
#   python/bin/{python,python3}(각 18.6MB!)·{2to3,idle3,pydoc3,python3-config}
#   git/libexec/git-core/git-remote-{ftp,ftps,https}(각 2.0MB · dedup-git-core.sh 는 기준 파일이
#     git-core/git(3406864B)이라 크기부터 달라 **원리적으로 못 잡는다** — 이 셋의 원본은 git-remote-http)
#   python/lib/pkgconfig/{python3.pc,python3-embed.pc} · python/share/man/man1/python3.1
#
# ★설계: 판정 기준은 '바이트 비교'가 아니라 **소스 트리의 구조**다 (dedup-git-core.sh 와 의도적으로 다름)
#   dedup-git-core.sh 는 링크 원본이 어디인지 모른 채 내용으로 추론해야 해서 cmp 를 쓴다. 여기서는
#   `src-tauri/runtime` 자체가 "이 경로는 원래 X 를 가리키는 심볼릭링크였다"는 **권위 있는 명세**다.
#   서명 빌드에서는 inside-out 재서명이 같은 내용에도 서로 다른 CMS 타임스탬프를 박아 바이트 대조가
#   깨질 수 있으므로(dedup-git-core.sh 머리 주석의 bin/git 사례와 동일한 함정), 바이트 동일성은
#   **게이트가 아니라 진단 출력**으로만 쓴다. 게이트는 ①링크 대상 실재 ②번들 이탈 없음 두 가지다.
#
# ★.app 안 바이너리를 **절대 exec 하지 않는다**(구조 검사만: readlink/stat/cmp).
#   `.app` 안에서 뭐라도 한 번 exec 하면 macOS 가 그 번들에 앱 번들 보호를 걸어, 뒤따르는 codesign 이
#   "Operation not permitted" 로 실패한다(2026-08-01 실측 · precompile-bundled-python.sh 머리 주석 참조).
#   npm/npx/corepack 의 **기능** rc=0 검증은 서명 이후 별도 사본에서 해야 한다
#   (verify-gatekeeper-user-path.sh 가 쓰는 '.app 확장자 뗀 probe 사본' 기법과 같은 자리).
#
# ★호출 위치는 '.app 생성 후 · 서명 전' 이어야 한다 — build-macos-signed.sh 의 dedup 단계 바로 뒤.
#   서명 전이어야 복원된 링크가 봉인에 **포함**되고, 서명 후면 봉인을 깨서 Gatekeeper 가 앱을 차단한다.
#
# 사용: scripts/restore-runtime-symlinks.sh <path-to-cys.app> [src-runtime-dir(기본 src-tauri/runtime)]
# 종료: 0=성공(소스 심볼릭링크 전량이 .app 에서도 링크) / 1=인자·경로 오류, 링크 대상 부재·번들 이탈,
#       또는 복원 후에도 역참조 잔존(=자가검증 실패). 어떤 경우에도 조용히 통과하지 않는다.
set -euo pipefail

APP="${1:?usage: restore-runtime-symlinks.sh <path-to-.app> [src-runtime-dir]}"
SRC="${2:-src-tauri/runtime}"
DST="$APP/Contents/Resources/runtime"

[ -d "$SRC" ] || { echo "✗ 소스 runtime 트리 없음: $SRC"; exit 1; }
[ -d "$DST" ] || { echo "✗ .app runtime 트리 없음: $DST"; exit 1; }

# 번들 이탈 검사용 절대경로 기준점(-P: 심볼릭링크 해석해 실제 위치로 고정)
DST_ABS="$(cd "$DST" && pwd -P)"

BEFORE_KB="$(du -sk "$DST" | awk '{print $1}')"
TOTAL=0; RESTORED=0; SKIP=0; SAME=0; DIFF=0

while IFS= read -r -d '' link; do
  rel="${link#"$SRC"/}"
  tgt="$(readlink "$link")"
  TOTAL=$((TOTAL+1))
  app_path="$DST/$rel"
  app_dir="$(dirname "$app_path")"

  # ── 게이트 ①: .app 에 해당 경로 자체가 있어야 한다(없으면 복사 누락 = 원인 규명 전 진행 금지) ──
  if [ ! -e "$app_path" ] && [ ! -L "$app_path" ]; then
    echo "✗ .app 에 경로 없음(복사 누락): $rel" >&2; exit 1
  fi

  # 이미 올바른 링크면 무동작 — 재실행 멱등 + 향후 upstream #13219 수정 시에도 그대로 안전.
  if [ -L "$app_path" ] && [ "$(readlink "$app_path")" = "$tgt" ]; then
    SKIP=$((SKIP+1)); continue
  fi

  # ── 게이트 ②: 링크 대상이 .app 안에 실재해야 한다(끊긴 링크를 만들어 더 크게 깨뜨리지 않는다) ──
  if [ ! -e "$app_dir/$tgt" ]; then
    echo "✗ 링크 대상이 .app 안에 없음: $rel -> $tgt (해석: $app_dir/$tgt)" >&2; exit 1
  fi
  # ── 게이트 ③: 해석된 대상이 번들 runtime 밖으로 나가면 안 된다(../ 탈출·외부 절대경로 차단) ──
  tgt_abs="$(cd "$app_dir" && cd "$(dirname "$tgt")" && pwd -P)/$(basename "$tgt")"
  case "$tgt_abs" in
    "$DST_ABS"/*) : ;;
    *) echo "✗ 링크 대상이 번들 runtime 밖을 가리킴: $rel -> $tgt ($tgt_abs)" >&2; exit 1 ;;
  esac

  # 진단(게이트 아님): 역참조 복사본이 대상과 바이트 동일한가 — 서명 타임스탬프로 갈릴 수 있다.
  if cmp -s "$app_path" "$app_dir/$tgt" 2>/dev/null; then SAME=$((SAME+1)); else DIFF=$((DIFF+1)); fi

  # 역참조본 제거 후 소스와 동일한 상대 링크로 복원. 디렉토리 역참조도 대비해 -rf.
  # (-n: 이미 디렉토리 심볼릭링크인 경우 그 '안에' 링크를 만드는 사고 방지)
  rm -rf "$app_path"
  ln -sfn "$tgt" "$app_path"
  RESTORED=$((RESTORED+1))
done < <(find "$SRC" -type l -print0)

AFTER_KB="$(du -sk "$DST" | awk '{print $1}')"
echo "✓ 심볼릭링크 복원 ${RESTORED}개 (이미 정상 ${SKIP}개 / 소스 링크 총 ${TOTAL}개) · 바이트동일 ${SAME}·상이 ${DIFF}"
echo "  runtime $((BEFORE_KB/1024))MB → $((AFTER_KB/1024))MB (회수 $(( (BEFORE_KB-AFTER_KB) ))KB)"

# ── fail-closed 자가검증: 소스 심볼릭링크 전량이 .app 에서도 '끊기지 않은 링크'여야 한다 ──
# 성공 판정을 '이번에 복원한 수'가 아니라 '잔존 역참조 수 = 0' 으로 둔다 → 멱등 실행·upstream 수정
# 이후에도 동일하게 옳다(dedup-git-core.sh 와 같은 판정 철학).
BAD=0
while IFS= read -r -d '' link; do
  rel="${link#"$SRC"/}"; tgt="$(readlink "$link")"; app_path="$DST/$rel"
  if [ ! -L "$app_path" ]; then
    echo "  ✗ 여전히 링크 아님: $rel" >&2; BAD=$((BAD+1)); continue
  fi
  if [ "$(readlink "$app_path")" != "$tgt" ]; then
    echo "  ✗ 링크 대상 불일치: $rel -> $(readlink "$app_path") (기대 $tgt)" >&2; BAD=$((BAD+1)); continue
  fi
  if [ ! -e "$app_path" ]; then   # -e 는 링크를 따라간다 → 끊긴 링크 검출
    echo "  ✗ 끊긴 링크: $rel -> $tgt" >&2; BAD=$((BAD+1))
  fi
done < <(find "$SRC" -type l -print0)
[ "$BAD" -eq 0 ] || { echo "✗ 자가검증 실패: 소스 심볼릭링크 ${BAD}개가 .app 에서 미복원/끊김 — 서명 전 중단" >&2; exit 1; }
echo "  ✓ 자가검증: 소스 심볼릭링크 ${TOTAL}개 전량이 .app 에서 정상 링크 (역참조 잔존 0)"
