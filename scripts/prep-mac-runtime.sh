#!/usr/bin/env bash
# prep-mac-runtime.sh — macOS 자기완결 동봉 런타임(Python·git·uv·node) 준비 (RC-20/T6b).
# 순정 맥(개발도구 없음) 소비자용: python3·git은 CLT-shim이라 실행 시 개발도구 프롬프트 → 동봉 필수.
# tauri.conf.json bundle.resources("runtime/")가 이 트리를 Contents/Resources/runtime 으로 싣고,
# runtime_bin_dirs(src/lib.rs·RC-18)가 PATH 해소, state.rs -lc 재선두주입(RC-19·D8)이 로그인셸 강등 회피.
# release.yml(CI)·build-macos-signed.sh(로컬 공증빌드)가 공유한다.
#
# 사용: scripts/prep-mac-runtime.sh [target]
#   target = aarch64-apple-darwin | x86_64-apple-darwin (기본=호스트 아키텍처)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

TARGET="${1:-}"
if [ -z "$TARGET" ]; then
  case "$(uname -m)" in
    arm64) TARGET=aarch64-apple-darwin ;;
    x86_64) TARGET=x86_64-apple-darwin ;;
    *) echo "unknown host arch $(uname -m)"; exit 1 ;;
  esac
fi
case "$TARGET" in
  aarch64-apple-darwin) PY_ARCH=aarch64; NODE_ARCH=arm64; DUGITE_ARCH=arm64; UV_ARCH=aarch64 ;;
  x86_64-apple-darwin)  PY_ARCH=x86_64;  NODE_ARCH=x64;   DUGITE_ARCH=x64;   UV_ARCH=x86_64 ;;
  *) echo "unexpected mac target $TARGET"; exit 1 ;;
esac

# 핀(실측 검증 2026-07-02): 갱신 시 URL/자산명 실재를 curl -I 로 확인할 것.
PBS_TAG=20260623            # python-build-standalone (astral) — cpython 3.12.13
DUGITE_TAG=v2.53.0-3        # desktop/dugite-native — git 2.53.0 (자산명에 커밋 f49d009 포함)
UV_VER=0.11.26             # astral-sh/uv (Windows 동봉과 동일 핀)
NODE_VER=22.17.1           # nodejs.org LTS

RT="src-tauri/runtime"
echo "== macOS 런타임 준비 ($TARGET) → $RT =="
rm -rf "$RT"; mkdir -p "$RT" "$RT/git" "$RT/uv" "$RT/node" "$RT/LICENSES"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

# ── Python 3.12.13 (python-build-standalone install_only · relocatable @rpath · pip 포함) ──
curl -fL -o "$TMP/py.tgz" "https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_TAG}/cpython-3.12.13+${PBS_TAG}-${PY_ARCH}-apple-darwin-install_only.tar.gz"
tar xzf "$TMP/py.tgz" -C "$RT"           # 최상위 python/ → runtime/python (bin/python3·pip)
# ── git (desktop/dugite-native 포터블 · CLT 불요) ──
# 이 tar.gz는 libexec/git-core 빌트인 143개를 이미 `git` 심볼릭링크로 dedup(추출 트리 ≈141MB)한다.
# ★여기서 추가 dedup을 하지 마라 — Tauri 번들러가 bundle.resources 복사 시 심볼릭링크를 역참조해
#   각 링크를 3.4MB 실복사본으로 부풀린다(upstream #13219). 이 단계 dedup은 복사 때 무효화된다.
#   실효 dedup은 '.app 생성 후·서명 전'에만 가능 → scripts/build-macos-signed.sh(RC-23 dedup 단계) 참조.
curl -fL -o "$TMP/git.tgz" "https://github.com/desktop/dugite-native/releases/download/${DUGITE_TAG}/dugite-native-v2.53.0-f49d009-macOS-${DUGITE_ARCH}.tar.gz"
tar xzf "$TMP/git.tgz" -C "$RT/git"      # bin/git·libexec/git-core
# ── uv/uvx (astral 단일 바이너리) ──
curl -fL -o "$TMP/uv.tgz" "https://github.com/astral-sh/uv/releases/download/${UV_VER}/uv-${UV_ARCH}-apple-darwin.tar.gz"
tar xzf "$TMP/uv.tgz" -C "$RT/uv" --strip-components=1   # uv-<triple>/{uv,uvx} → runtime/uv
# ── node + npm/npx (공식 tarball · Developer ID 서명본) ──
curl -fL -o "$TMP/node.txz" "https://nodejs.org/dist/v${NODE_VER}/node-v${NODE_VER}-darwin-${NODE_ARCH}.tar.xz"
tar xJf "$TMP/node.txz" -C "$RT/node" --strip-components=1   # bin/{node,npm,npx}·lib
# ── 무손실 트림 (성능>속도 Pareto: 기능·정보 불변, '불필요 입증분'만 제거 — 의심분은 보존) ──
# node/include(≈53MB) = C 애드온 컴파일용 헤더(node_api·v8·gyp). 제거 근거 3중: ①런타임에 로드되지
#   않음(순수 빌드타임 헤더) ②node-gyp는 헤더를 nodejs.org/~/.cache/node-gyp 에서 받지 이 디렉토리를
#   보지 않음(코드베이스에 --nodedir 사용 0건 실측) ③타깃 소비자=순정 맥(개발도구 없음)엔 C++ 컴파일러가
#   없어 네이티브 애드온 빌드 자체가 불가 → 헤더가 원천적으로 무용. 우리가 쓰는 npx 도구
#   (kordoc·pdfjs-dist·playwright·hyperframes·@playwright/mcp)는 전부 순수 JS/사전빌드본이라 컴파일 없음.
#   ※헤더는 런타임에 사람에게 보여지는 '정보'가 아니므로 정보-불변 게이트에도 저촉 없음.
rm -rf "$RT/node/include"
# ※npm/docs·npm/man(≈2.6MB)은 게이트 전부 통과했으나(`npm help` exit 0) `npm help <주제>` 출력 콘텐츠가
#   바뀌어 '정보 불변' 엄격 해석에 걸릴 여지가 있어 retention 원칙으로 보존 — 오너 명시 승인 시 추가 트림.

printf 'Bundled runtimes and their licenses (macOS):\n- CPython (python-build-standalone): PSF License (https://github.com/astral-sh/python-build-standalone)\n- git (desktop/dugite-native): GPLv2 (https://github.com/desktop/dugite-native)\n- uv (astral-sh): Apache-2.0 OR MIT (https://github.com/astral-sh/uv)\n- Node.js (+npm/npx): MIT (https://nodejs.org)\nWritten offer for corresponding source: contact the distributor.\n' > "$RT/LICENSES/BUNDLED-RUNTIMES.txt"

# ── SEAL-2: 동봉 python 선컴파일 (서명 전 필수 · 2026-08-01 실사고 구조적 봉인) ──
# install_only tar 에는 `.pyc` 가 없어서, 이 트리를 그대로 서명하면 나중에 **누가 번들 python 을
# 부르든** CPython 이 번들 안에 `__pycache__/*.pyc` 를 새로 써서 코드서명 봉인이 깨진다
# (→ quarantine 사본이 첫 실행에서 "손상됨"으로 차단). 여기서 미리 컴파일해 `.pyc` 가
# **서명 대상**이 되게 하면 이후 새로 쓸 것이 없다. 근거·모드 선택(unchecked-hash)·검증은
# scripts/precompile-bundled-python.sh 머리 주석. ★node(node-gyp/gyp) 의 python 도 같은 표면이라
# 런타임 트리 전체를 대상으로 한다 → 반드시 python·node 내려받기가 **끝난 뒤**에 돈다.
bash scripts/precompile-bundled-python.sh "$RT"   # 이 스크립트는 상단에서 repo 루트로 cd 한 상태다

ls -la "$RT/python/bin/python3" "$RT/git/bin/git" "$RT/uv/uv" "$RT/node/bin/node"
echo "✓ macOS 런타임 준비 완료 ($TARGET)"
