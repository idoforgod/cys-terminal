# 릴리스 절차 (cys 터미널)

## 0-0. 현행 릴리스 폐쇄 절차 (2026-07-21 정본)

> 이 절과 `release/assets-policy.json`, `release/credential-contract.json`,
> `.github/workflows/release*.yml`이 현행 정본이다. 아래 절과 충돌하는 종전 수동 발행·직접 공개
> 설명은 역사적 참고(legacy)로만 읽는다. Windows 인스톨러는 NSIS이며 MSI/WiX 경로는 폐기됐다.

바이너리 태그 `vX.Y.Z`를 push하면 `release.yml`은 플랫폼별 서명 산출물을 **암호화된 Actions
artifact**로만 넘긴다. 공개 저장소의 Actions artifact는 읽기 권한자에게 노출될 수 있으므로
평문 후보를 직접 업로드하지 않는다. 단일 조립 잡만 전달물을 복호화해 정확한 자산 집합과 해시를
검증하고 GitHub Release를
**draft 한 개**로 생성한다. 이 워크플로에는 공개 전환 경로가 없다. 업로드가 일부라도 실패하면
불완전 draft를 삭제하고 실패한다.

공개 전환은 다음 조건을 모두 만족한 뒤 `release-publish.yml`을 수동 실행할 때만 가능하다.

1. `release-production` GitHub Environment의 필수 승인 통과
2. 입력 태그가 기존 draft이고 소스 버전과 일치
3. 검토한 `SHA256SUMS.txt` 자체의 SHA-256을 `release_bundle_sha256` 입력으로 제공
4. 허용목록·중복 0·누락 0, 전 자산 해시, Windows ZIP 구조, updater 3플랫폼을 재검증
5. `confirm` 입력이 정확히 `PUBLISH`

### 필수 CI 입력 이름

값은 GitHub Secrets/Variables에만 저장하고 문서·로그·명령행 출력에 기록하지 않는다.

- macOS secrets: `APPLE_CERTIFICATE_B64`, `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_KEYCHAIN_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`,
  `APPLE_TEAM_ID`, `TAURI_SIGNING_PRIVATE_KEY`
- Windows secrets: `WINDOWS_CERTIFICATE_B64`, `WINDOWS_CERTIFICATE_PASSWORD`,
  `WINDOWS_EXPECTED_PUBLISHER`, `TAURI_SIGNING_PRIVATE_KEY`
- Windows variable: `WINDOWS_TIMESTAMP_URL`(HTTPS RFC3161 TSA)
- 공통 secret: `RELEASE_HANDOFF_KEY`(무작위 32자 이상, 공개 저장소 Actions handoff 암호화)
- Browser Runtime 공통 secrets: `CYS_BROWSER_RUNTIME_SECRET_KEY_B64`,
  `CYS_BROWSER_RUNTIME_PUBLIC_KEY_B64`, `CYS_BROWSER_RUNTIME_KEY_ID`,
  `CYS_BROWSER_RUNTIME_POLICY_EPOCH`, `CYS_BROWSER_RUNTIME_EXPIRES_AT`. 앞의 두 값은 minisign
  키 파일 전체 바이트의 base64이며, workflow가 러너 임시 디렉터리에만 복원하고 항상 삭제한다.
  key id와 public key는 바이너리에 컴파일된 active/non-revoked 신뢰근원과 일치해야 한다.

GitHub 저장소 설정도 코드 밖의 차단 게이트다. 공개 승격 전에 다음 조회가 성공하고,
`release-production` Environment에 required reviewers와 publish용 secrets가 설정되어 있어야 한다.
404/빈 protection rule이면 태그·draft를 공개하지 않는다.

```sh
gh api repos/idoforgod/cys-terminal/environments/release-production \
  --jq '{name:.name, protection_rules:.protection_rules}'
```

입력이 하나라도 비거나 형식이 잘못되면 빌드를 시작하지 않는다. macOS arm64/x64는 각각 앱과
DMG 공증 제출·staple·Gatekeeper 검증을 통과해야 한다. Windows sidecar와 NSIS 설치 파일은
SHA-256 Authenticode 서명, HTTPS RFC3161 타임스탬프, 게시자 일치 검증을 통과해야 한다.

### Browser Runtime 소스·스테이징·서명 계약

`release/browser-runtime-sources.json`이 Rust 1.95.0, Bun 1.3.8, Playwright 1.49.1과
세 플랫폼의 Bun compiler 및 Chromium 131.0.6778.33 headless-shell archive를 URL·바이트 수·
SHA-256으로 고정한다. 릴리스 잡은 태그의 동일 source object에서 supervisor와 engine을 빌드하고,
archive 경로·형식·크기·대상 아키텍처·라이선스를 검증해 Tauri resource tree에 원자적으로
stage하며 archive 안의 symbolic link는 내부 상대경로라도 전부 거부한다. 전체 `Chromium.app`
archive는 framework symlink를 실파일로 펼쳤을 때 서명 bundle이
모호해지는 것이 확인되어 사용하지 않는다. 제품 런타임은 headless CDP이므로 symlink가 없는 공식
Playwright headless-shell archive를 사용한다.

순서는 반드시 **stage → 플랫폼 코드 서명/검증 → 최종 파일·트리 hash 계산 → minisign
attestation/policy 생성·검증 → Tauri bundle**이다. macOS는 staged tree의 모든 Mach-O를
Developer ID hardened-runtime으로 inside-out 서명하고, Windows는 모든 EXE/DLL을 SHA-256
Authenticode와 RFC3161 timestamp로 서명·검증한다. 코드 서명이 바이트를 바꾸므로 이보다 앞서
생성한 runtime hash는 출시 근거가 될 수 없다.

Bun standalone compiler는 source 절대경로를 결과물에 포함할 수 있으므로 이 계약은 서로 다른
workspace 사이의 byte-for-byte 재현성을 주장하지 않는다. 대신 source·도구체인·입력을 고정하고,
해당 릴리스에서 실제로 생성·플랫폼 서명된 출력 digest를 최종 runtime metadata에 묶어 minisign한다.
로컬 macOS arm64에서 stage 후 Developer ID 검증과 실제 persistent-context 첫 페이지 smoke가
통과했지만, macOS 두 target의 공증 DMG와 Windows CI Authenticode/NSIS 결과는 매 릴리스의 독립
필수 게이트이며 문서상의 로컬 증거로 대체할 수 없다.

### 바이너리 릴리스의 정확한 공개 자산 14개

`X.Y.Z`는 태그에서 `v`를 제외한 버전이다. 이 목록 이외 자산은 조립과 공개를 실패시킨다.

- 설치본 4개: `cys_X.Y.Z_aarch64.dmg`, `cys_X.Y.Z_x64.dmg`,
  `cys_X.Y.Z_x64-setup.exe`, `cys_X.Y.Z_x64-setup.zip`
- updater 6개: `cys_X.Y.Z_aarch64.app.tar.gz`, 같은 이름의 `.sig`,
  `cys_X.Y.Z_x64.app.tar.gz`, 같은 이름의 `.sig`,
  `cys_X.Y.Z_x64-setup.exe.sig`, `latest.json`
- pack 3개: `pack.tar.gz`, `pack-manifest.json`, `pack-manifest.json.minisig`
- 무결성 1개: `SHA256SUMS.txt`(나머지 13개 전부, 누락 0·중복 0)

Windows ZIP은 설치 EXE 하나만 루트에 담는 결정론적 flat ZIP이다. 다운로드 페이지의 Windows
Defender 안내는 `data-cys-release-marker="windows-defender-guidance-v1"`로 보존한다.

### 홈페이지 승격과 원격 폐쇄 검증

조립 잡의 `assembled-vX.Y.Z` artifact에는 암호화된 `assembled-vX.Y.Z.tgz.enc` 한 파일만
들어 있다. 보안 저장소에서 `RELEASE_HANDOFF_KEY`를 환경으로 불러온 뒤
`bash scripts/release-handoff.sh unpack <암호문> <검토 디렉터리>`로 복호화하면 검토 대상
`release-out/`, 홈페이지 반영본 `release-site/downloads/`, `release-bundle.sha256`이 나온다.
승인 후 검토된 동일 바이트를 Hostinger의 `/downloads/`에 승격하고, 다운로드 인덱스와 복구
페이지까지 원격 검증한다.

```sh
bash scripts/verify-release-remote.sh \
  --version X.Y.Z \
  --release-dir /path/to/reviewed/release-out \
  --site-root https://www.cysinsight.com
```

검증기는 다운로드 인덱스의 Defender 안내 marker/문구, macOS 격리 우회 문구 부재, 복구 페이지,
원격 `SHA256SUMS.txt`의 검토본 일치, 체크섬에 적힌 **모든 원격 자산의 실제 바이트 해시**를
fail-closed로 확인한다.

팩-only 태그는 `pack-release.yml`이 5개 자산을 별도 draft로만 만든다. 공개는 검토한
`PACK_SHA256SUMS.txt` digest와 `release-production` 승인을 받는
`pack-release-publish.yml`에서만 수행한다. 바이너리/팩 후보 워크플로에서는 공개 업로드가 0이다.

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
- **직전 latest.json 동봉 필수**. 누락 시 `releases/latest/download/latest.json`이 404가 되어 전체
  사용자의 바이너리 확인 채널이 파손된다.
- 팩-온리 적용 후 "디스크 팩 > 바이너리 임베드 팩" 상태가 일상화된다 — 이때 부트 스윕은
  pack_current_for 게이트(디스크 ≥ 바이너리 = 스킵)로 아예 실행되지 않는 것이 **정상 동작**이다
  (2026-07-12 도입 — 종전의 "스윕 실행 → 다운그레이드 가드 no-op" 소음 제거). 수동 `cys init-pack`은
  게이트를 타지 않고 여전히 다운그레이드 가드에 막힌다(동일 최종 상태·이중 방어).

## 0. 버전 위치 (범프 시 모두 갱신 — **게이트 강제 6곳**)

> ★제목 정정(2026-07-20): 이전 표기 "실측 4곳"은 **게이트와 어긋난다**. `scripts/version-check.sh`와
> `release.yml`이 wxs 2곳을 포함한 **6곳을 하드 게이트로 강제**하므로, 4곳만 범프하면 **태그 CI가 즉사**한다.
> 아래 취소선(legacy 표기)은 배포 자산 기준의 역사 맥락이고, **범프 대상은 6곳 전부**다.

- `Cargo.toml` / `src-tauri/Cargo.toml` — `version`
- `src-tauri/tauri.conf.json` — `version`
- `ui/package.json` — `version`
- ~~`dist-win/cys.wxs` / `dist-win/cys-x64.wxs`~~ (legacy MSI — NSIS 전환으로 폐기)

> ⚠ **문서-게이트 드리프트(2026-07-12 기록)**: `scripts/version-check.sh`는 아직 wxs 2곳을 포함한
> **6곳**을 강제한다. 스크립트가 정리되기 전까지는 wxs 2곳도 함께 범프해야 게이트를 통과한다.

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
- [ ] **T7 두 기능 핀** — `python3 cysjavis-pack/bin/tests/test_banner_truth.py` ·
      `test_cli_probe.py` · `test_formation.py` 전부 green. (부트 배너가 사실을 말하는가 ·
      CLI 감지가 로그인셸 프로브 단일 경로인가 · 편성 상태 enum 이 계약대로인가 —
      셋 다 릴리스 후 사용자 머신에서만 드러나는 부류라 사전 핀이 유일한 방어다.)
- [ ] **T9 혼잡 드릴** — `python3 cysjavis-pack/bin/javis_queue_drill.py` **29판정 전부 pass**
      (exit 0). 격리 데몬을 띄워 소프트캡 거부·TTL 이관·멱등 병합·OOB 통지·하드캡을 실제로
      돌린다. 라이브 데몬 무접촉(Z1)·격리 데몬 잔여 0(Z2)까지 판정에 포함된다.
- [ ] 신규 머신 시뮬레이션: 빈 HOME에서 `cys list` → 데몬 자동기동 + pack 자동설치 확인
- [ ] DMG에서 설치 → 앱 실행 → `cys status` 동작
- [ ] 버전 문자열 4곳(+wxs 2곳) 일치
- [ ] **홈페이지 메인 DL-HERO 버전 동기**(`cys-homepage/_round/dlhero/RELEASE_BUMP_CHECK.md` 5지점:
      링크 3개·S 슬롯 카피·용량·zip 전환 경로) 후 **원격 메인 페이지 구버전 문자열 0 확인**
      (`curl -s https://www.cysinsight.com/ | grep -c '<이전버전>'` → 0). /downloads/만 갱신하면
      메인 밴드가 구버전을 계속 배포한다(무증상 실패 — 2026-07-26 v0.13.17 사고).
- [ ] 릴리스 노트(RELEASE_NOTES_0.2.0.md) 작성
