# 릴리스 절차 (cys 터미널)

> **현행 표준 절차(2026-07 정정)**: 릴리스는 **release.yml 자동화**가 정본이다 —
> ①버전 범프(아래 §0 4곳)+`cargo build`(Cargo.lock)+로컬 `bash scripts/secret-scan.sh --all` clean 확인
> ②main push ③`git tag vX.Y.Z && git push origin vX.Y.Z`(태그=오너 직접·가드)
> ④CI 4잡(mac signed·mac x86 sidecar·**windows NSIS**·pack) green + windows-build.yml T5 green
> ⑤릴리스 자산·`latest.json`(tauri v2 — darwin-aarch64·darwin-x86_64·windows-x86_64 3키) 실측 확인.
> Windows 인스톨러는 **NSIS**다(`src-tauri/tauri.windows.conf.json targets:["nsis"]`) — 아래 §2·부록의
> 수동 MSI/WiX 경로는 **legacy(폐기·참고용)**이며 따르지 마라.

## 0-A. 업데이트 발행 이원화 정책 (2026-07-12 오너 확정)

> **두 레인으로 발행한다.**
> ① **팩-온리 패치 (기본)** — Rust/GUI 코드가 안 바뀐 릴리스는 pack 3종
> (pack-manifest.json / .minisig / pack.tar.gz)만 발행. 사용자는 인앱 Update 버튼으로
> **무중단**(재시작 없음 · 세션/부서/직원 유지) 적용.
> ② **바이너리 릴리스 (드묾)** — 본체(Rust/GUI) 변경 시에만. v0.12.51+부터 인앱 원클릭
> 본체 설치는 제거되고 배지는 **안내 + 홈페이지(www.cysinsight.com) 다운로드 링크**만
> 제공한다(본체 교체 = 홈페이지 풀 설치본). v0.12.50 이하 사용자에게는 이 전환 릴리스가
> **마지막 인앱 바이너리 업데이트**로 배달된다.

**레인 판정 게이트(결정론)**: `git diff <직전태그>..HEAD --stat -- src/ src-tauri/ ui/ build.rs Cargo.toml`
출력이 비어 있으면(=`cysjavis-pack/`·docs만 변경) 팩-온리 대상. 한 줄이라도 있으면 바이너리 릴리스.

> ⚠ **레거시 도달 예외(2026-07-12 오너 확정)**: 수정이 팩에만 있어도 **구버전(≤0.12.50) 사용자에게
> 반드시 도달해야 하는 심각한 버그 수정이면 바이너리 릴리스로 발행**한다. 구버전의 유일한 수신
> 통로는 인앱 바이너리 업데이트(latest.json)뿐이고, 팩-온리는 min_binary 하한이 구버전을 (의도적으로)
> 차단하기 때문이다. 바이너리 릴리스의 latest.json·업데이터 자산 발행은 계속 유지한다 — 이것이
> 구버전 사용자의 원클릭 업그레이드 통로다("본체=홈페이지"는 v0.12.51+ 화면 동작이지 채널 폐쇄가 아님).

### 팩-온리 발행 절차 (현행 수동 — CI 자동화는 Phase2 별도 과제)

pack_version은 빌드 시점 `CARGO_PKG_VERSION`에 용접돼 있어(`cys.rs build_pack_manifest_value`)
팩만 발행해도 **버전 범프 + cys 재빌드**가 필요하다(§0 전 위치 갱신 — version-check.sh 통과).

1. 버전 범프(§0) → `cargo build --release --bin cys` (Tauri 빌드 불요 — cys 단독).
2. pack 3종 생성 — release.yml `pack-artifacts` 잡과 동일 파라미터(스캔 게이트 2종 선행 포함):
   `cys pack-manifest --key-id … --signed-at … --expires-at … --min-binary-version 0.12.48 > pack-manifest.json`
   → 결정론 tar(`--mtime` 고정) → minisign 서명.
3. **직전 릴리스의 latest.json + 바이너리 업데이트 자산을 그대로 동봉**해 새 릴리스를 만들고
   `--latest`로 마킹한다(바이너리 버전은 그대로 → 바이너리 배지 안 뜸).
4. 검증: 앱 배지 = `↻`(무중단 팩)만 표시, `!`(바이너리) 미표시. 구버전(≤0.12.47) 기기는
   "바이너리 업데이트 필요" 안내가 뜨는 것이 정상(하한 게이트 동작).

**불변 규칙 (실사고 이력 근거 — 위반 금지)**:
- `--min-binary-version` **0.12.48 이상 필수**. seed-once 상태 보호(memory/·SESSION_STATE 불가침)는
  0.12.48+ **바이너리 쪽 코드**다 — 공란이면 ≤0.12.47 사용자의 pack-update가 사용자 상태를 vendor
  골격으로 클로버한다(2026-07 팩 치유 원복 실사고 계열). 새 팩이 더 새로운 바이너리 기능에 의존하면
  그 버전으로 상향한다. (release.yml `PACK_MIN_BINARY` env가 CI 기본값.)
- **★기능 의존 하한은 사람이 올려야 한다 — 두 레인 모두 (2026-07-26 S26 레인 비대칭 교정)**.
  `scripts/min-binary-policy.sh` 는 **보안 하한·지원창만** 계산하며 **기능 의존을 모른다**
  (실측: `bash scripts/min-binary-policy.sh` → `0.12.48`). 따라서 **바이너리 릴리스가 새 CLI 표면
  (플래그·서브커맨드)을 추가하고 팩이 그것을 지시하면, 다음 pack 태그 전에 하한을 그 버전으로
  올려라.** 손대야 할 곳은 **두 곳(레인마다 하나)** 이다:
  - 본체 레인 — `.github/workflows/release.yml` 의 `PACK_MIN_BINARY`
  - 팩-온리 레인 — `.github/workflows/pack-release.yml` 의 `PACK_MIN_BINARY_OVERRIDE`
    (비면 정책 스크립트 값 `0.12.48` 이 쓰인다)

  한쪽만 고치면 다른 문으로 같은 실패가 들어온다 — 실제로 `--emit-decl` 의존(이 레인에서는
  v0.14.2 에서 처음 생긴다)을 release.yml 에만 반영했다가, 다음 `pack-v` 태그가
  `min_binary=0.12.48` 로 서명될 뻔했다. 두 값은 **같아야** 한다(다르면 어느 쪽이 옳은지
  아무도 모른다). 오버라이드 사용 시 `::warning::min_binary 정책 오버라이드 사용` 이
  워크플로 로그에 남는 것이 **정상**이다.
- **직전 latest.json 동봉 필수**. 누락 시 `releases/latest/download/latest.json`이 404가 되어 전체
  사용자의 바이너리 확인 채널이 파손된다.
- 팩-온리 적용 후 "디스크 팩 > 바이너리 임베드 팩" 상태가 일상화된다 — 이때 부트 스윕은
  pack_current_for 게이트(디스크 ≥ 바이너리 = 스킵)로 아예 실행되지 않는 것이 **정상 동작**이다
  (2026-07-12 도입 — 종전의 "스윕 실행 → 다운그레이드 가드 no-op" 소음 제거). 수동 `cys init-pack`은
  게이트를 타지 않고 여전히 다운그레이드 가드에 막힌다(동일 최종 상태·이중 방어).

## 0. 버전 위치 (범프 시 모두 갱신 — **게이트 강제 8곳**)

> ★표기 이력(종전 "실측 4곳" → **2026-07-26 재정정**): 이전 표기들(4 → 6)은 모두 **과소 집계**라
> **게이트와 어긋났다**. `scripts/version-check.sh`와 `release.yml`이 하드 게이트로 강제하는 위치는
> **8곳**이다 — 아래 **수동 편집 6개 + `Cargo.lock` 2패키지(자동 재생성)**. 8곳 중 하나라도
> 어긋나면 **태그 CI가 즉사한다**. 아래 취소선(legacy 표기)은 배포 자산 기준의 역사 맥락일 뿐
> **범프 면제가 아니다**.

**(A) 수동 편집 6개**

- `Cargo.toml` / `src-tauri/Cargo.toml` — `version`
- `src-tauri/tauri.conf.json` — `version`
- `ui/package.json` — `version`
- ~~`dist-win/cys.wxs` / `dist-win/cys-x64.wxs`~~ (legacy MSI — NSIS 전환으로 폐기)

> ⚠ **문서-게이트 드리프트(2026-07-12 기록 · 여전히 유효)**: 위 wxs 2개는 배포 자산으로는 폐기
> 표기지만 `scripts/version-check.sh`는 **여전히 강제한다**. 스크립트가 정리되기 전까지는 wxs 2개도
> 함께 범프해야 게이트를 통과한다 — **취소선을 보고 건너뛰면 태그 CI가 즉사한다**(실제 함정).

**(B) 자동 재생성 2개 — `Cargo.lock`**

> ★**+Cargo.lock 2패키지 (S23 · 2026-07-26 추가)**: `Cargo.lock` 의 `[[package]] cys-terminal` ·
> `cys-app` version 도 `version-check.sh` 가 강제한다(A 6개 + B 2개 = **합계 8곳**). Cargo.lock 은
> 손편집 대상이 **아니다** — 위 (A) 수동 편집 6개를 고친 뒤
> `cargo update -w -p cys-terminal -p cys-app`(또는 `cargo build`)로
> 재생성해 **범프 커밋에 함께 담는다**. 이 검사를 붙인 이유: 종전 유일한 lock 포획자는
> `release.yml` 의 `cargo build --locked -p cys-browserd` 하나였는데, aarch64 레그는 그 앞의
> unlocked 빌드가 lock 을 재생성해 드리프트를 **자가치유·은폐**하고, x64·Windows 레그만 **20분 뒤**
> 죽으면서 로그가 "브라우저 런타임 스테이징 실패"로 읽혀 원인을 감췄다.
> ⚠한계: 이건 lock 의 **버전 필드 2개**만 본다. 의존 그래프 전체의 최신성은 `--locked` 빌드만이
> 증명한다.

## 1. macOS 빌드 (DMG + 앱 번들 + 업데이트 아티팩트)

> **자동 업데이트가 켜져 있으므로(`createUpdaterArtifacts: true`) 빌드 시 서명 키가 필요합니다.**
> 키 없이 빌드하면 `.app.tar.gz.sig`가 안 생기고 업데이트 manifest를 만들 수 없습니다.

```sh
# 사전: bun, rustup(aarch64-apple-darwin / x86_64-apple-darwin)
#       서명 키: ~/.tauri/cys-updater.key (최초 1회 `bun x @tauri-apps/cli signer generate`로 생성, 분실 시 자동업데이트 영구 불가)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""   # 키에 암호를 걸었다면 그 값

bun x @tauri-apps/cli build
#  → target/release/bundle/dmg/cys_0.2.0_aarch64.dmg
#  → target/release/bundle/macos/cys.app             (cysd·cys externalBin 동봉)
#  → target/release/bundle/macos/cys.app.tar.gz(.sig) (자동 업데이트용 — 서명 키 있을 때만)

# 배포본으로 정리 (아키텍처 접미사 표준화)
cp target/release/bundle/dmg/cys_0.2.0_aarch64.dmg dist-mac/cys-0.2.0-macos-arm64.dmg

# 업데이트 manifest(latest.json) + 자산 생성
sh scripts/make-update-manifest.sh 0.2.0 <OWNER> cys-terminal
#  → dist-update/latest.json, dist-update/cys-0.2.0-macos-aarch64.app.tar.gz
```

`beforeBuildCommand`(scripts/bundle-prep.sh)가 UI 번들 + cys/cysd 릴리스 빌드 + `externalBin` 배치를
자동 수행합니다. Intel 빌드가 필요하면 `--target x86_64-apple-darwin` 추가(manifest의 `darwin-x86_64`에 키 추가).

### ★Apple 서명·공증 (다른 맥 배포의 유일한 정공법 — 2026-06-15)

**왜 필수인가**: ad-hoc 서명 빌드는 *빌드한 맥*에선 우클릭→열기로 되지만, **다른 맥으로
전송하면** 파일에 `com.apple.quarantine`가 붙고 macOS(Sequoia+)가 **ad-hoc·미공증 앱을
"손상됨"으로 차단**한다(실측 2026-06-15: `spctl -a`=rejected). 공증해야만 어떤 맥에서도
경고/손상됨 없이 열린다.

**1회 셋업 (사람 단계)**:
1. **Apple Developer Program 가입**($99/년, developer.apple.com)
2. **Developer ID Application 인증서** 발급 → Keychain 설치
   (Xcode > Settings > Accounts > Manage Certificates > + > Developer ID Application,
    또는 developer.apple.com > Certificates)
3. **notarytool 자격증명** — 둘 중 하나:
   - app-specific password: appleid.apple.com > 로그인 및 보안 > 앱 암호 생성
   - 또는 App Store Connect API key(.p8 + Key ID + Issuer ID)
4. **Team ID** 확인: developer.apple.com > Membership

**빌드 (자격증명 env + 헬퍼 스크립트가 자동 codesign+공증+staple+검증)**:
```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: NAME (TEAMID)"
export APPLE_ID="you@example.com" APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx" APPLE_TEAM_ID="TEAMID"
#   (또는 API key: APPLE_API_KEY_PATH=…/AuthKey_XXXX.p8 APPLE_API_KEY=KEYID APPLE_API_ISSUER=ISSUER)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bash scripts/build-macos-signed.sh  # env 검증 → tauri build(자동 공증) → spctl/stapler 검증 → dist-mac + manifest
#  (반드시 bash — 스크립트가 프로세스 치환 `< <(...)`(bash 전용)을 쓴다. `sh`로 실행하면 line 57 syntax error.)
```
- 배선: `tauri.conf.json > bundle.macOS.entitlements = entitlements.plist`(hardened runtime +
  사이드카 cysd·cys 로드 허용). Tauri가 빌드 중 Developer ID codesign + notarytool 제출 +
  staple 을 자동 수행한다(별도 `codesign`/`notarytool` 수동 호출 불요).
- **검증 통과 기준**: `spctl -a -vv cys.app` = **accepted**. (rejected면 공증 실패 — 빌드
  로그의 notarization 결과 확인.)
- 공증 빌드는 **ad-hoc 재서명·`xattr` 우회가 전혀 불필요**하다.

> 인증서가 없을 때(개발용): env 없이 `bun x @tauri-apps/cli build` → ad-hoc 빌드. 이 빌드는
> **다른 맥 전송 시 "손상됨"**이 뜨므로, 받은 맥에서 `xattr -dr com.apple.quarantine
> /Applications/cys.app` 로만 우회 가능(배포용 아님).

### ★비기술자(청중) 배포 전 게이트 체크리스트 (D6 제품 모드)
오너 대표 산출물을 제3자에게 패키징해 내보내기 전, 아래를 **모두** 확인한다.
- [ ] **공증 빌드**(`spctl -a -vv cys.app` = accepted) — 미공증은 비기술자 배포 금지(다른 맥에서 "손상됨" 차단).
- [ ] **신뢰선 라벨 활성** — 스킬 보드 산출물에 "🔒 AI 보조 생성 · 오너 검수 전"이 부착되는지(과대약속 "80~90%" 금지).
- [ ] **외부발행은 master 승인 경유** — 제3자 공유/전송은 자율주행 denylist의 "외부발행(비가역)"에 해당. `cys feed push --wait`(master 승인)를 거친다. 임의 전송 금지(§4 외부발행 원칙 계승).
- [ ] **HITL 미리보기 보존** — 제품 모드도 입력 모달·validate_ir 게이트·미리보기 확인을 우회하지 않는다("1클릭"이라도 게이트 제거는 REJECT).
- [ ] **청중 프로파일 확인** — `~/.cys/profile.json` audience가 대상 청중과 일치(민감 스킬은 카탈로그 미포함=암묵 차단).

## 2. [LEGACY·폐기] Windows 수동 빌드 (MSI + ZIP) — 현행은 CI NSIS, 따르지 말 것

> Windows 머신(또는 Parallels Win11 ARM64)에서 수행. 코어는 검증 완료.

```powershell
# 사전: rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
cargo build --release --bin cys --bin cysd --target x86_64-pc-windows-msvc
cargo build --release --bin cys --bin cysd --target aarch64-pc-windows-msvc

# WiX(candle/light)로 MSI 생성 — dist-win/cys.wxs(arm64)·cys-x64.wxs(x64) 사용
#   ProgramFiles에 cys.exe·cysd.exe 설치 + PATH 등록
candle dist-win\cys-x64.wxs -o cys-x64.wixobj
light  cys-x64.wixobj -o dist-win\cys-0.2.0-windows-x64.msi
candle dist-win\cys.wxs    -o cys.wixobj
light  cys.wixobj    -o dist-win\cys-0.2.0-windows-arm64.msi

# ZIP (설치 없이)
Compress-Archive target\x86_64-pc-windows-msvc\release\cys.exe,cysd.exe `
  dist-win\cys-0.2.0-windows-x64.zip
```

GUI 앱의 Windows Tauri 빌드는 잔여 — 현재 Windows는 CLI+데몬 중심 배포.

### ★macOS에서 Windows 크로스빌드 (Windows 머신 없이 — 2026-06-15 실증)

Windows 머신이 없어도 macOS에서 MSI까지 만들 수 있다. **windows-gnu 타깃**(wxs Source가
가리키는 `x86_64-pc-windows-gnu`·`aarch64-pc-windows-gnullvm`)을 zig 링커로 크로스컴파일하고,
WiX 대신 **msitools(wixl)**로 MSI를 만든다. (cys.wxs는 표준 WiX v3라 wixl이 그대로 읽는다.)

```sh
# 사전: rustup(homebrew rust와 별개) · cargo-zigbuild · zig · msitools(wixl)
#   brew install zig msitools && cargo install cargo-zigbuild
#   curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
rustup target add x86_64-pc-windows-gnu aarch64-pc-windows-gnullvm

# 바이너리 크로스컴파일 (GUI 없이 cys+cysd만)
cargo zigbuild --release --target x86_64-pc-windows-gnu      --bin cys --bin cysd
cargo zigbuild --release --target aarch64-pc-windows-gnullvm --bin cys --bin cysd

# MSI (wixl — wxs Source 상대경로가 ../target/... 이므로 dist-win에서 실행)
cd dist-win
wixl -o cys-0.2.1-windows-x64.msi   cys-x64.wxs
wixl -o cys-0.2.1-windows-arm64.msi cys.wxs
cd ..
# ZIP
zip -j dist-win/cys-0.2.1-windows-x64.zip   target/x86_64-pc-windows-gnu/release/cys.exe target/x86_64-pc-windows-gnu/release/cysd.exe
zip -j dist-win/cys-0.2.1-windows-arm64.zip target/aarch64-pc-windows-gnullvm/release/cys.exe target/aarch64-pc-windows-gnullvm/release/cysd.exe
```

⚠ **한계(정직)**: 크로스빌드 산출물은 PE 포맷·아키텍처는 검증되나(`file`로 PE32+ x86-64 /
Aarch64 확인) **실제 Windows에서 실행 검증은 불가**하다. 광범위 배포 전 Windows 머신에서
스모크테스트(설치→`cys status`) 권장.

## 3. GitHub 저장소 최초 설정 (1회)

자동 업데이트의 endpoint가 GitHub Releases이므로 **공개 repo가 있어야** 작동합니다.

```sh
# 1) GitHub에 공개 repo 생성 (이름은 cys-terminal 권장 — endpoint와 일치)
gh repo create <OWNER>/cys-terminal --public --source . --remote origin

# 2) tauri.conf.json의 updater.endpoints에서 OWNER를 실제 GitHub 사용자명으로 치환
#    "https://github.com/<OWNER>/cys-terminal/releases/latest/download/latest.json"
#    → 치환 후 반드시 앱을 다시 빌드해야 새 endpoint가 번들에 박힌다.

git push -u origin main
```

## 4. GitHub 릴리스

`latest.json`을 **항상 최신 릴리스에 포함**해야 updater가 찾습니다(endpoint가 `/releases/latest/`).

```sh
# 태그
git tag -a v0.2.0 -m "cys 0.2.0 — 자비스 네이티브 기능 19건 + zero-setup 온보딩 + 자동 업데이트"

# gh CLI 릴리스 (드래프트로 먼저 검토 권장)
gh release create v0.2.0 --draft --title "cys 0.2.0" --notes-file docs/RELEASE_NOTES_0.2.0.md \
  dist-update/latest.json \
  dist-update/cys-0.2.0-macos-aarch64.app.tar.gz \
  dist-mac/cys-0.2.0-macos-arm64.dmg \
  dist-win/cys-0.2.0-windows-x64.msi \
  dist-win/cys-0.2.0-windows-arm64.msi \
  dist-win/cys-0.2.0-windows-x64.zip
```

### 자동 업데이트 동작 요약 (사용자 입장)
- 앱이 시작 시 + 6시간마다 `latest.json`을 조용히 확인 → 새 버전이면 상단 **Update** 버튼에 `!` 배지.
- 버튼 클릭 → 세션이 0개면 자동 설치, 세션이 있으면 "N개 종료됩니다" 확인 후 설치.
- 설치 = 새 `.app` 교체 + 구 데몬 SIGTERM + 앱 재시작(새 cysd 자동 기동). **재설치 불필요.**

⚠ **`git push`·`gh release`·`gh repo create`는 외부 발행(비가역)** — 오너 명시 승인 후에만 실행.
본 문서의 명령은 절차 기록일 뿐, 에이전트가 임의 실행하지 않는다.

## 5. 서명 키 백업 (중요)

`~/.tauri/cys-updater.key`(private)를 **분실하면 이후 버전에 서명할 수 없어 자동 업데이트가 영구 중단**됩니다.
- 안전한 곳(암호 관리자·오프라인 백업)에 보관. git에 절대 커밋 금지.
- 공개키(`tauri.conf.json`의 `pubkey`)는 이미 사용자 앱에 박혀 있어, 같은 private 키로만 새 업데이트를 서명할 수 있습니다.

## 4. 릴리스 전 체크리스트

- [ ] `cargo build --release` 무오류 · `cargo clippy --bins` 0경고 · `cargo test` 통과
- [ ] 신규 머신 시뮬레이션: 빈 HOME에서 `cys list` → 데몬 자동기동 + pack 자동설치 확인
- [ ] DMG에서 설치 → 앱 실행 → `cys status` 동작
- [ ] 버전 문자열 4곳(+wxs 2곳) 일치 — `sh scripts/version-check.sh vX.Y.Z` rc=0
      (**Cargo.lock 2패키지 포함 8곳**. 범프 후 `cargo` 가 lock 을 다시 쓰게 하고 그 결과를
      범프 커밋에 함께 담아라. 손편집 금지 · S23)
- [ ] **★메인 페이지(`/`) 원격 검증 — 4항목 전부 (S28 · 자동화 밖의 수동 게이트)**

      원 레인에서 이 격차의 형태는 "원격 검증기(`verify-release-remote.sh`)·조립기
      (`release-assemble.py`)가 `/downloads/` 만 보고 메인 페이지는 아예 보지 않는다"였다.
      **이 레인(0.14.2)에는 그 두 스크립트가 존재하지도 않는다**(실측: `scripts/` 에 없음) —
      즉 메인 페이지 검증은 **부분 자동화조차 없는 100% 수동 게이트**다. 그래서 메인 밴드 누락은
      404 가 아니라 **무증상 구버전 배포**로 나타난다(구자산이 보존돼 링크는 200). v0.13.17 에서
      실제로 발생했다. 아래 4항목은 아무 스크립트도 대신해 주지 않으므로 **사람이 손으로 돌리고
      결과를 릴리스 노트에 붙인다.**

      선행: `cys-homepage/_round/dlhero/RELEASE_BUMP_CHECK.md` 5지점(링크 3개·S 슬롯 카피·
      용량·zip 전환 경로) 반영 → 업로드 → 그 다음 아래를 원격에 대고 실행한다.

      - [ ] ① **구버전 문자열 0** — `curl -s https://www.cysinsight.com/ | grep -c '<이전버전>'` → **0**
      - [ ] ② **신버전 문자열 9** — `curl -s https://www.cysinsight.com/ | grep -c '<신버전>'` → **9**
            (0 이면 미반영, 9 미만이면 부분 반영 = 밴드 일부가 구버전을 계속 배포한다)
      - [ ] ③ **용량 4토큰 개별 재계산** — ★버전 grep 이 **못 잡는 별개 축**이다.
            `sed 's/0.14.1/0.14.2/g'` 류 일괄 치환은 버전 9토큰만 바꾸고 페이지에 적힌 용량
            4토큰(예: 340MB·358MB·226MB·226MB)은 **그대로 남긴다** — 버전은 전부 새 값이라
            ①②는 통과하는데 표시 용량만 조용히 거짓이 된다. 자산 4종의 실제 바이트를
            `curl -sI` 의 `content-length` 로 **하나씩** 받아 페이지 표기와 대조하라:

            ```sh
            V=X.Y.Z; B=https://www.cysinsight.com/downloads
            for f in "cys_${V}_aarch64.dmg" "cys_${V}_x64.dmg" \
                     "cys_${V}_x64-setup.exe" "cys_${V}_x64-setup.zip"; do
              L=$(curl -sI "$B/$f" | awk 'tolower($1)=="content-length:"{print $2}' | tr -d '\r')
              printf '%-32s %s bytes  = %s MB\n' "$f" "$L" "$((L/1000/1000))"
            done
            ```
            4줄 전부 값이 나와야 하고(빈 값 = 자산 부재), MB 표기가 메인 페이지 4토큰과
            일치해야 한다. 불일치 1건이라도 있으면 **미완**이다.
      - [ ] ④ **버튼 href 4종 HEAD 200** — 페이지에 박힌 다운로드 링크를 눈이 아니라 HTTP 로 확인:

            ```sh
            curl -s https://www.cysinsight.com/ \
              | grep -oE 'href="[^"]*(dmg|setup\.exe|setup\.zip)"' | sed 's/href="//;s/"$//' \
              | sort -u | while read -r u; do
                  case "$u" in http*) U="$u";; *) U="https://www.cysinsight.com${u#.}";; esac
                  printf '%-70s %s\n' "$u" "$(curl -s -o /dev/null -w '%{http_code}' -I "$U")"
                done
            ```
            **4개 URL 이 나와야 하고 전부 200** 이어야 한다. 개수가 4 미만이면 밴드에 링크가
            빠진 것이고, 200 이 아니면 자산 경로가 어긋난 것이다.

      ⚠이 4항목을 `verify-release-remote.sh` 에 넣지 않는 이유: 홈페이지는 **이 리포 밖**이라
      스크립트가 그 SOT(밴드 구조·용량 표기 규약)를 알지 못한다. 자동화하려면 홈페이지 리포에
      게이트를 두는 것이 옳다 — 여기서는 문서 게이트로 고정한다.
- [ ] 릴리스 노트(RELEASE_NOTES_0.2.0.md) 작성
