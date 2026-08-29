# 릴리스 절차 (cys 터미널)

> **현행 표준 절차(2026-07 정정)**: 릴리스는 **release.yml 자동화**가 정본이다 —
> ①버전 범프(아래 §0 4곳)+`cargo build`(Cargo.lock)+로컬 `bash scripts/secret-scan.sh --all` clean 확인
> ②main push ③`git tag vX.Y.Z && git push origin vX.Y.Z`(태그=오너 직접·가드)
> ④CI 4잡(mac signed·mac x86 sidecar·**windows NSIS**·pack) green + windows-build.yml T5 green
>  (windows-build.yml PTY 스모크는 **pane env 실주입 관측** — U-20 `CLAUDE_CODE_GIT_BASH_PATH`·
>  좌석 토큰 `CYS_SEAT_TOKEN` — 까지 게이트한다. 단 **벤더 링크 분절**(claude 가 그 env 로 훅을
>  **실발화**하는가)은 claude 인증 필요로 CI 게이트化 불가 — §4 「Windows 실기 수동 체크리스트」
>  수동 행이 그 몫이다. CI 초록을 그 분절의 증거로 읽지 마라. ★windows-build.yml 은
>  **태그 push 에 자동 트리거되지 않는다**(트리거=feat/windows-x64-dist push+workflow_dispatch
>  한정) — 릴리스 SHA 에서 **수동 dispatch** 로 돌려 green 을 받아라. 이전 런의 초록은
>  릴리스 증거가 아니다)
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

## 0-B. 빌드 도구 핀 — Tauri CLI 버전 SOT (2026-08-29 · v0.14.28 W5)

> ★릴리스 3레그(macOS 2 + Windows)가 쓰는 **Tauri CLI 는 `2.11.4` 로 고정**한다. 단일 SOT 는
> `.github/workflows/release.yml` 의 `env: TAURI_CLI_VERSION: "2.11.4"` 이며, 다른 호출 지점
> (`windows-build.yml` · `scripts/build-macos-signed.sh` — 종전에는 버전 태그조차 없이
> `@tauri-apps/cli@2` 유동이었다)이 같은 값을 소비한다. 바꿀 때는 **SOT 한 곳만** 고치고
> 소비 지점이 전파받는지 확인한다.
>
> ★하한 계약: 고정 버전을 **0.14.27 을 빌드한 버전(= 2.11.4) 미만으로 내리지 않는다.**
> 더 낮은 CLI 는 NSIS 템플릿이 `/UPDATE`→`$UpdateMode` 없는 세대로 회귀해 인앱 업데이트가
> "제거 후 설치"로 빠진다(전 세션 사망 · 무음 재앙). 2.11.4 는 감사가 installer.nsi 라인
> 단위로 검증한 버전이다(`_work` NSIS-CONTRACT §7).
>
> ★VERSIONINFO 상시 게이트(2026-08-29 신설 · W5-3): Windows 빌드 후 사이드카
> (`src-tauri/binaries/cys-*.exe`·`cysd-*.exe`)와 메인 exe 에 **VERSIONINFO(버전 리소스)가
> 실재하는지 CI 가 단언하고, 없으면 빌드를 실패**시킨다. 설치 훅의 신선도 판정이
> `GetDLLVersion` 오라클(fail-closed = unverified)이라, 크로스 빌드 회귀로 버전 리소스가
> 빠진 채 발행되면 **모든 기계의 설치가 exit 4 로 떨어지기 때문**이다.

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
  ⚠**이것만으로는 부족하다**(2026-08-01 실사고). 빌드 직후의 `.app`은 accepted 인데
  **사용자가 브라우저로 받아 설치하면 "손상되었기 때문에 열 수 없습니다"로 차단**될 수 있다.
  반드시 아래 실사용자 경로 게이트를 함께 돌려라.
- 공증 빌드는 **ad-hoc 재서명·`xattr` 우회가 전혀 불필요**하다.

#### ★실사용자 경로 게이트 — `scripts/verify-gatekeeper-user-path.sh` (2026-08-01 신설·필수)

```sh
bash scripts/verify-gatekeeper-user-path.sh dist-mac/cys-<V>-macos-arm64.dmg  # 로컬 빌드 산출물
bash scripts/verify-gatekeeper-user-path.sh --version <V> --arch aarch64      # 발행본(원격)
bash scripts/verify-gatekeeper-user-path.sh https://…/downloads/cys_<V>_x64.dmg  # 임의 URL
#   exit 0 = PASS · 1 = FAIL(배포 금지) · 2 = 판정 불가(도구·자산 문제 — 통과 아님)
```

**왜 필요한가 — 2026-08-01 사고의 근본원인**: 앱 번들의 **동봉 Python 이 실행 중
`Contents/Resources/runtime/python/lib/python3.12/**/__pycache__/*.pyc` 를 번들 안에 새로
써서 코드서명 봉인을 스스로 깨뜨린다**. `codesign --verify` 진단 원문:
`a sealed resource is missing or invalid / file added: …/__pycache__/_compression.cpython-312.pyc`.
공증·staple 자체는 **정상**이다. 그런데 사용자가 브라우저로 받으면 파일에
`com.apple.quarantine` 이 붙고, **첫 실행 시 Gatekeeper 가 전체 재검증**을 돌려 깨진 봉인을
잡아내 실행을 차단한다.

**검증 구멍**: 지금까지의 릴리스 검증(ⓓ)은 `curl` 사본만 봤다 — `curl` 로 받은 파일에는
quarantine 이 **안 붙어서** Gatekeeper 전체 재검증 경로 자체가 돌지 않는다. 그래서 이 사고
경로를 **한 번도 재현하지 못했다**. 이 스크립트가 그 구멍을 닫는다.

**하는 일 6단계** — ① 실제 브라우저 다운로드에서 실측한 형식으로 DMG 에 `com.apple.quarantine`
부착 → ② 마운트(격리 볼륨 확인) → ③ `ditto` 로 드래그 설치 모사 + **quarantine 상속 확인** →
④ `spctl --assess --type execute --verbose=4` = accepted → ⑤ `codesign --verify --deep --strict`
+ `stapler validate` → ⑥ ★**봉인 자기파괴 재현**: 동봉 python 을 1회 돌린 뒤 `codesign --verify`
재실행 → `file added` 가 나오면 **FAIL**.

**⑥ 판정의 두 갈래**
- **⑥-A**(판정 본체): env 완화를 못 타는 경로(셸·훅·pane 에서 도는 python)에서도 번들이 안
  깨져야 한다 = **패키징 층위 불변식**. 닫는 방법은 둘뿐이다 — 서명 **전** `compileall
  --invalidation-mode unchecked-hash` 로 stdlib `.pyc` 를 **서명 대상에 포함**시키거나,
  런타임을 봉인 밖으로 빼는 것.
- **⑥-B**: 앱이 python 을 스폰할 때 얹는 `PYTHONDONTWRITEBYTECODE=1`(SEAL-1 완화)이 실제로
  쓰기를 막는지. 실측 v0.14.9: **⑥-B PASS**(.pyc 3→3) · **⑥-A FAIL**(.pyc 3→30).
  즉 스폰 env 완화는 유효하지만 **번들 자체는 아직 깨질 수 있는 상태**다.

**⑥ 이 `.app` 확장자를 뗀 사본에서 도는 이유**(실측 2026-08-01, macOS 25.5.0): macOS 는
`.app` 번들에서 바이너리를 한 번이라도 exec 하면 그 번들을 **앱 번들 보호**로 잠가서, 그 뒤
셸이 띄운 python 의 쓰기를 EPERM 으로 막는다 → **결함이 있어도 아무 일 없는 것처럼 보인다**
(측정: `.app` 사본은 .pyc 3→3, codesign exit 0 = **거짓 PASS**). 실제 사고 경로에서는 쓰는
주체가 **앱 자신**(같은 Team ID)이라 이 보호를 통과해 쓰기가 성사된다. 확장자를 떼는 것은
검사를 약화시키는 우회가 아니라, **검증기 머신에서만 발생하는 OS 보호가 결함을 가리는 것을
걷어내는 조치**다(서명·봉인 내용은 동일).

**전제·비용**: macOS + Xcode CLT · 임시폴더에 약 2GB(DMG 227MB + 앱 사본 492MB × 3, 종료 시
자동 삭제) · 로컬 DMG 기준 약 1분 · **앱을 실행하지 않는다**(동봉 python 만 1회 스폰).
arm64 머신에서 x64 DMG 를 볼 땐 Rosetta 2 필요(없으면 ⑥-A 가 "검사 불성립"으로 FAIL —
측정 불능은 통과가 아니다).

**`scripts/verify-release-remote.py` 와의 관계 (직교 · 둘 다 필수)**

| | `verify-release-remote.py` | `verify-gatekeeper-user-path.sh` |
|---|---|---|
| 시점 | 발행 **후** | 빌드 직후 + 발행 후 |
| 보는 것 | 홈페이지 표기·링크·용량·SHA256 (=**올바른 파일이 올라갔나**) | 그 파일을 **받아서 설치했을 때 열리나** |
| 대상 | 원격 HTML·HTTP 헤더·SHA256SUMS.txt | DMG 바이트 → 마운트 → 앱 번들 서명·봉인 |
| 못 잡는 것 | 자산이 정상 링크·정상 해시로 올라가 있으면서 **열리지 않는 것** | 홈페이지 표기 오류·링크 누락 |

즉 앞의 것은 "제대로 배포됐나", 뒤의 것은 "제대로 열리나"다. **하나가 다른 하나를 대신하지
못한다.**

#### ★CI 자동판 게이트 — `scripts/release-gate-gatekeeper.sh` (2026-08-13 신설 · release.yml 이 업로드 전 자동 실행)

위 실사용자 경로 게이트의 **정적 CI 판(判)**이다. `release.yml` 의 macOS 두 레그가
tauri-action 업로드 **전에** 자기 아키텍처 DMG 에 자동으로 돌린다(= DMG 2종 커버).
v0.14.19 실측(run 32039644404): 두 레그 모두 `spctl: assessments enabled → 모드=full`
로 실평가가 돌았다(강등 아님).

```sh
bash scripts/release-gate-gatekeeper.sh <DMG | .app>
#   exit 0=PASS · 1=FAIL(업로드 금지) · 2=판정 불가(도구 부재·마운트 실패·degraded 폐쇄 — 통과 아님)
```

- 검사 = quarantine 부착·상속 → 마운트 → `codesign --verify --deep --strict` +
  `stapler validate` + `spctl --assess --type execute` + ⑤ SEAL-2 불변식 **전칭(∀) 정적
  검사**(모든 `.py` 의 3레벨 `.pyc` 파일별 대응 + 고아 `.pyc` 0 + 발견된 `.pyc` 전량 헤더
  flags==1 — 실행 0 · 표본화 제거 F1 격상 2026-08-20, 세목은 스크립트 머리 주석). 대상은
  DMG 안 **모든** `*.app`(설치 도우미 `Install cys.app` + 숨김 `.support/cys.app`).
  러너 정책이 `assessments disabled`(=degraded) 면 **판정 불가 exit 2 로 폐쇄**된다
  (F2 수리 2026-08-20 — 측정 불능≠통과 · `GATE_MODE=degraded` 1줄 출력 후 즉시 종료).
  강등 평가가 필요한 진단은 `--diagnose-degraded-ok`(LOUD 고지 · **발행 경로 사용 금지**
  — release.yml·release-postprocess.py 가 이 플래그를 싣지 않음은
  `scripts/tests/test_release_postprocess_gate.py` 의 문자열 핀이 지킨다)로만 연다.
- **`verify-gatekeeper-user-path.sh` 와의 관계 — 겹치지 않는 상·하위 게이트다.** 그쪽
  (로컬·수동)은 여기 검사 전부 + ⑥ **봉인 자기파괴 재현**(동봉 python 을 실제로 스폰)까지
  본다. 대신 대상 아키텍처 python 을 실행하므로 arm64 러너에서 x64 DMG 를 보려면 Rosetta 2
  가 필요하고(없으면 FAIL) 임시 디스크 ~2GB 를 쓴다 — 그대로 CI 매트릭스에 걸면 x64 레그가
  구조적으로 깨진다. 이 스크립트는 실행을 전혀 하지 않는 **정적 평가만** 남긴 CI 판이다.
  로컬 릴리스 절차에서는 여전히 위 실사용자 경로 게이트를 돌려라.
- ★**발행 층위 훅**(F2 · 2026-08-20 신설): `scripts/release-postprocess.py` 가 4단계
  (자기 검증) 직후에 draft 백업 DMG 2종 = **발행될 실물 바이트**에 이 게이트를 자동 실행하고,
  네이티브 아키텍처 DMG 에는 `verify-gatekeeper-user-path.sh`(⑥ 포함)까지 얹는다.
  rc≠0(1·2 모두)이면 postprocess 전체가 비영 종료해 `--apply` 가 거부된다(fail-closed ·
  측정 불능≠통과 · macOS 밖에서는 판정 불가로 fail-closed). 비상 탈출구
  `--unsafe-skip-gatekeeper`(LOUD 경고 2줄 · 평시 금지).

> 인증서가 없을 때(개발용): env 없이 `bun x @tauri-apps/cli build` → ad-hoc 빌드. 이 빌드는
> **다른 맥 전송 시 "손상됨"**이 뜨므로, 받은 맥에서 `xattr -dr com.apple.quarantine
> /Applications/cys.app` 로만 우회 가능(배포용 아님).

### ★비기술자(청중) 배포 전 게이트 체크리스트 (D6 제품 모드)
오너 대표 산출물을 제3자에게 패키징해 내보내기 전, 아래를 **모두** 확인한다.
- [ ] **공증 빌드**(`spctl -a -vv cys.app` = accepted) — 미공증은 비기술자 배포 금지(다른 맥에서 "손상됨" 차단).
- [ ] **실사용자 경로 게이트 exit 0** — `bash scripts/verify-gatekeeper-user-path.sh <DMG>`.
      accepted 인데도 "손상되었기 때문에 열 수 없습니다"로 막히는 경로(2026-08-01 사고)를
      잡는 유일한 검사다. 비기술자는 이 화면을 만나면 **문의 없이 그냥 이탈한다** —
      실패가 신고로 나타나지 않으므로 기계 게이트로만 막을 수 있다.
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
- [ ] **★★실사용자 경로 게이트 — DMG 2종 전부 exit 0 (2026-08-01 신설 · 필수 · 생략 불가)**

      ```sh
      # 로컬 빌드 산출물 이름은 dist-mac/cys-<V>-macos-{arm64,x64}.dmg 다
      # (발행본 이름 cys_<V>_{aarch64,x64}.dmg 와 다르다 — build-macos-signed.sh:221)
      bash scripts/verify-gatekeeper-user-path.sh dist-mac/cys-<V>-macos-arm64.dmg  # rc=0 필수
      bash scripts/verify-gatekeeper-user-path.sh dist-mac/cys-<V>-macos-x64.dmg    # rc=0 필수
      ```
      `spctl -a -vv` accepted **만으로는 부족하다**. 2026-08-01 사고에서 공증·staple 이 전부
      정상인 빌드가 사용자 머신에서 "손상되었기 때문에 열 수 없습니다"로 차단됐다 — 동봉
      Python 이 실행 중 번들 안에 `.pyc` 를 써서 **코드서명 봉인을 스스로 깨뜨렸고**,
      브라우저로 받은 사본에 붙은 `com.apple.quarantine` 때문에 첫 실행에서 Gatekeeper
      전체 재검증에 걸린 것이다. **이 게이트가 없으면 같은 사고가 그대로 재발한다.**
      · 실패 시 원인·수정 방향은 §1 「★실사용자 경로 게이트」 절 참조.
      · **exit 2(판정 불가)도 통과가 아니다** — 도구·자산 문제를 고치고 다시 돌려라.
      · 발행 후에는 원격 자산에 대고 한 번 더 돌린다:
        `bash scripts/verify-gatekeeper-user-path.sh --version <V> --arch aarch64` (x64 도)
      · **아키텍처별 머신에서 각자 돌리는 것이 정확하다**(교차 실행은 Rosetta 2 필요).
      · ★후처리 자동판(F2 · 2026-08-20): `scripts/release-postprocess.py` 가 draft 백업
        DMG 2종 = **발행될 실물 바이트**에 `release-gate-gatekeeper.sh` 실평가를 자동
        수행한다(네이티브 아키텍처 DMG 는 본 게이트 ⑥ 포함까지) — rc≠0(1·2 모두)이면
        postprocess 비영 종료·`--apply` 거부(fail-closed · §1 「★CI 자동판 게이트」 절 참조).
      · 복구 절차 주의 — **pre-SEAL 태그**(2026-08-20 SEAL 게이트 도입 이전 발행분)를 자산
        유실 복구 등으로 재후처리할 때는 ⑤ SEAL-2 정적 검사가 구조적으로 FAIL 한다: 이미
        발행·검증된 과거 실물 바이트에 한해 `--unsafe-skip-gatekeeper` 로 우회한다
        (LOUD 경고 감수 · 신규 발행에는 절대 사용 금지).
- [ ] **★메인 페이지(`/`) 원격 검증 — 6항목 전부 (S28 + 2026-07-29 오너 지시 ⓐⓑⓒ · 자동화 밖의 수동 게이트)**

      원 레인에서 이 격차의 형태는 "원격 검증기(`verify-release-remote.sh`)·조립기
      (`release-assemble.py`)가 `/downloads/` 만 보고 메인 페이지는 아예 보지 않는다"였다.
      **이 레인에는 그 두 스크립트가 존재하지 않았다**(실측) — 그래서 메인 페이지 검증은
      100% 수동 게이트였다. ★**2026-07-29 해소**: `scripts/verify-release-remote.py` 신설로
      6항목 전부를 기계로 돌린다(라이브 0.14.4 로 교정 — 6/6 PASS). 아래 수동 명령은
      그 스크립트가 못 돌 때의 폴백이자 판정 기준의 서술로 남긴다. 그래서 메인 밴드 누락은
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
              printf '%-32s %s bytes  = %s MB\n' "$f" "$L" "$((L/1024/1024))"
            done
            ```
            4줄 전부 값이 나와야 하고(빈 값 = 자산 부재), MB 표기가 메인 페이지 4토큰과
            일치해야 한다. 불일치 1건이라도 있으면 **미완**이다.
            ★**단위는 MiB(1024) 버림**이다 — 십진 MB(1000)로 계산하면 정상 배포에서도 어긋난다
            (실측: 231,644,695B → 라이브 표기 **220MB** · 1000 기준이면 231 · 반올림이면 221).
      - [ ] ④ **버튼 href 4종 HEAD 200** — 페이지에 박힌 다운로드 링크를 눈이 아니라 HTTP 로 확인.
            ★**정적 `href="…"` 만 grep 하면 3개만 잡힌다** — zip 링크는 JS 가
            `setAttribute('href', '/downloads/…zip')` 로 붙이기 때문이다(실측). 양쪽을 봐야 4개다.
            아래 명령은 정적만 본다 → **`python3 scripts/verify-release-remote.py <V> <구버전>` 을 쓰라**
            (6항목 전부를 기계로 돌리고 JS 주입 링크까지 센다):

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

      - [ ] ⑤ **Windows Defender/SmartScreen 안내 섹션 잔존** (2026-07-29 오너 지시 ⓐ·ⓒ)
            버전 범프 일괄 치환이 밴드 카피를 통째로 갈아끼우면 이 안내가 **조용히 사라진다**.
            사라지면 Windows 사용자가 SmartScreen 경고에서 그냥 이탈한다(설치 실패로 나타나지 않고
            **다운로드 후 침묵**으로 나타나므로 아무도 신고하지 않는다 — §2.6 관측 침묵과 같은 구조).

            ★**오너 체크리스트 정본 문언**(2026-07-29 · 이 문단이 그 문언의 리포 내 정본이다):
            > ⓐ **다운로드 페이지** Defender 안내 섹션 잔존 grep 확인
            > ⓑ SHA256SUMS 전 자산 갱신 · 누락 0
            > ⓒ **원격 검증(verify-release-remote)에 안내 섹션 출현 포함**

            ★2026-08-18 교정 — 검사 **대상 페이지와 방법이 둘 다 바뀌었다**. 종전 이 항목은
            루트(`/`)에서 낱말 grep 만 했다. 두 가지가 틀렸다:
              · **대상** — ⓐ 가 못박은 페이지는 루트가 아니라 **`/downloads/`** 다. 안내 섹션의
                실체는 그쪽의 `<section data-cys-release-marker="windows-defender-guidance-v2">`
                이고, 루트에는 밴드 카피의 낱말만 흩어져 있다
                (읽기 전용 실측 2026-08-18: 루트 마커 **0건** · `/downloads/` 마커 **1건**).
                즉 구 명령은 ⓐ 가 지목한 페이지를 **한 번도 받지 않았다** = ⓒ 미구현이었다.
              · **방법** — 낱말 grep 은 섹션이 통째로 사라져도 페이지 어딘가에 'Defender' 한
                낱말만 남아 있으면 통과한다(무증상 통과). 마커는 섹션 자체의 지문이라 섹션이
                빠지면 즉시 0이 되고, **개수까지 단언**하므로 중복 삽입(2건)도 잡는다.
            낱말 grep 은 **없애지 않고 보조 축으로 AND** 유지한다(회귀 감시 축을 줄이지 않는다).
            단, 낱말을 세기 전에 `data-cys-release-marker="…"` 속성을 **제거**한다 — 마커 문자열
            자체에 `defender` 가 들어 있어 제거하지 않으면 "마커가 있으면 낱말도 있다"가
            항진명제가 되고 보조 축이 무력해진다(실측으로 반증된 지점).

            ```sh
            # 정본 = 기계 검사. 6항목 전부를 한 번에 돌린다(⑤ 포함).
            python3 scripts/verify-release-remote.py <신버전> <구버전>

            # 손으로 볼 때(참고용) — 대상은 루트가 아니라 /downloads/ 다.
            curl -s https://www.cysinsight.com/downloads/ \
              | grep -c 'data-cys-release-marker="windows-defender-guidance-v2"'
            ```
            마커가 **정확히 1건**이어야 한다. 0 이면 안내 섹션이 사라진 것이고 2 이상이면 중복
            삽입이다 — 어느 쪽이든 복원·정리할 때까지 **미완**이다. 보조 축(마커 속성을 뺀
            `/downloads/` 본문 낱말 ≥1 · 루트 낱말 ≥1)도 함께 만족해야 통과다
            (읽기 전용 실측 2026-08-18: 마커 1 · 다운로드 본문 낱말 4 · 루트 낱말 8 → 통과).

      - [ ] ⑥ **SHA256SUMS 신버전 전체 갱신 · 누락 0** (2026-07-29 오너 지시 ⓑ)
            ★파일명은 **`SHA256SUMS.txt`** 다(`SHA256SUMS` 는 404 — 실측). CI 가 아니라
            **`scripts/release-postprocess.py`** 가 CI 완주 후 로컬에서 만든다(자기 자신을 뺀
            **전 자산** — 배포 4종만이 아니다. v0.14.4 기준 13줄). 홈페이지에도 같은 파일이
            올라가야 하고, **구버전 줄이 섞여 있으면 안 된다**:
            ```sh
            V=X.Y.Z; B=https://www.cysinsight.com/downloads
            curl -s "$B/SHA256SUMS.txt" | tee /tmp/sums.txt
            test "$(grep -c "cys_${V}_" /tmp/sums.txt)" -ge 4   # 신버전 4줄 이상(전 자산 등재)
            # 구버전 자산 줄 0 (버전 없는 공용 자산 cys_aarch64.app.tar.gz 등은 정상)
            test "$(grep -cE "cys_[0-9]+\\.[0-9]+\\.[0-9]+_" /tmp/sums.txt)" \
               -eq "$(grep -c "cys_${V}_" /tmp/sums.txt)"
            # 실자산과 대조(다운로드 후 검증) — 표기만 갱신되고 바이트가 구버전인 사고 차단
            cd "$(mktemp -d)" && for f in "cys_${V}_aarch64.dmg" "cys_${V}_x64.dmg" \
                 "cys_${V}_x64-setup.exe" "cys_${V}_x64-setup.zip"; do curl -sO "$B/$f"; done
            curl -sO "$B/SHA256SUMS.txt" && shasum -a 256 -c SHA256SUMS.txt
            ```
            **4줄 전건 OK** 여야 한다. 1건이라도 FAILED 면 미완이다.

      ⚠**이 6항목이 보지 않는 것 — 2026-08-01 사고의 정확한 사각지대**: 여기서 자산을 받는
      수단은 `curl` 이다. **`curl` 로 받은 파일에는 `com.apple.quarantine` 이 붙지 않는다.**
      quarantine 이 없으면 첫 실행 시 Gatekeeper 전체 재검증 경로가 **아예 돌지 않아서**,
      봉인이 깨진 앱도 이 6항목을 전부 통과한다(v0.14.9 실측: 링크·용량·해시 전건 정상,
      그런데 사용자 설치 시 차단). 그래서 위 체크리스트의 **★★실사용자 경로 게이트**가
      **별도 필수 항목**이다 — 이 6항목이 그것을 대신하지 못한다.

      ⚠**남은 한계**: 이 검증은 홈페이지의 SOT(밴드 구조·카피 규약)를 알지 못하고 **결과만** 본다.
      "링크가 200이고 버전이 맞다"는 "밴드가 의도대로 구성됐다"를 뜻하지 않는다.
      구조 자체의 게이트는 홈페이지 리포(`cys-homepage/_round/dlhero/RELEASE_BUMP_CHECK.md`)에
      두는 것이 옳다 — 여기서는 배포 결과 게이트로 고정한다.
- [ ] **★Windows 실기 수동 체크리스트 — CI 게이트化 불가 분절 (2026-08-26 P4-3 · P1 이월 · 바이너리 릴리스 시)**

      아래 두 행은 **CI 로 게이트化할 수 없음이 확정된** 분절이다 — 어느 CI 초록도 이 행들의
      증거가 아니다(돌지 않는 초록 금지). 실기 완주 전까지 각 행의 상태는 **'실기 미검증'** 으로
      정직하게 유지한다. 실기 절차의 몸통(Parallels VM 준비 등)은
      `docs/WINDOWS-UPGRADE-ATOMICITY-CHECKLIST.md` 와 같은 자리에서 함께 1회 완주한다(P7 인접).

      - [ ] **U-20 벤더 존중 확인 — 상태: 실기 미검증** (P4-3 사슬 분절③)
            CI 가 재는 것은 사슬의 두 분절뿐이다: ①설치 트리에 훅 bash
            (`runtime\git\bin\bash.exe`) 실재·실행(windows-build.yml 하드 단언) ②pane env 에
            `CLAUDE_CODE_GIT_BASH_PATH`·`CYS_SEAT_TOKEN` 실주입(같은 워크플로 PTY 스모크의
            env 관측 스텝). 마지막 분절 — **벤더(claude CLI)가 그 env 를 존중해 훅을 실제
            발화하는가** — 는 claude 인증이 필요해 어떤 CI 에도 실을 수 없다(src/lib.rs U-20
            주석 자인: "실제 훅이 뜨는가는 실기 재현의 몫 — 과장하지 않는다"). 실기 판정:
            Windows 실기의 cys pane 에서 `$env:CLAUDE_CODE_GIT_BASH_PATH` 값 + `Test-Path`
            확인 → `claude` 기동 → SessionStart 훅 실발화(역할 부트스트랩 배너) 관측.
            PASS 후에만 이 행의 상태 표기를 갱신한다.
      - [ ] **P1 좌석 토큰 '체인 단절 + 토큰 = 성공' 확인 — 상태: 실기 미검증** (P1 이월)
            조상 체인 해석이 끊기는 실기 조건(S1/S3 계급 — `cys claim-role` 이 rc 6 을 내던
            그 기계 상태)에서, pane PTY env 로 배달된 `CYS_SEAT_TOKEN` 만으로 claim 이
            성공(rc 0 · `registered:` 출력)해야 한다. CI 는 토큰 **배달**까지만 관측하고
            (위 ② 분절), 체인 단절 실조건은 실기 claude 세션에서만 재현된다.
            판정: 종전 rc 6 재현 조건에서 rc 0. PASS 전까지 '실기 미검증' 유지.
- [ ] **★codex ready_marker 라이브 재주입 왕복 실증 — 상태: 실기 미검증 (P0-6 측정 선행
      게이트 잔여 분절 · 2026-08-26 등재)**

      P0-6 확정설계의 통과 조건 3분절 중 ③ **'라이브 codex launch-agent → 디렉티브 재주입
      왕복 1회 실증'**은 선언 커밋(612e95b)이 배달성 검체(fill_missing_fields 픽스처)로
      갈음했다고 자인한 분절이다 — 어느 CI 초록·PTY 캡처도 이 분절의 증거가 아니다(캡처가
      실증한 것은 ready 화면 문면 `? for shortcuts` 의 출현·재현성까지다).

      실기 판정: 라이브 codex 로 `cys launch-agent`(reviewer-codex) 기동 → 디렉티브 주입 →
      재주입 경로(readiness `Site::Reinject` — scrollback 꼬리 마커 감지) 왕복 **1회**가
      실제 도달해야 PASS. 같은 라이브 런에서 P0-6 분절 ②(업데이트/로그인 화면 비출현)도
      함께 관측·기록한다.

      **실패 시 처분(안전측)**: `cysjavis-pack/agents.json` codex 항목의 `ready_marker` 키를
      **제거**한다 — 현행 idle+quiet 폴백 복귀가 안전측이다(오문면 마커는 Reinject 를 영구
      불능으로 악화 — 마커 선언 어댑터는 폴백으로 흐르지 않는다, readiness.rs 주석 명문).
      Windows 문면 미측정 고지(612e95b Not-tested)는 이 행의 PASS 와 무관하게 **유지**한다.
- [ ] **★user-owned 헌법 파일 개정의 배달 고지 (2026-08-26 R3-DELIVERY-1 · 디렉티브·soul·
      schedule/agents/acl.json 을 건드린 릴리스에서 필수)**

      `directives/*_DIRECTIVE.md`·`soul.md`·`CLAUDE.md`·`schedule.json`·`agents.json`·`acl.json`
      은 `src/pack.rs ownership()` 상 **`Ownership::User`** 다 — 업데이터는 디스크≠임베드이면
      매니페스트 해시가 일치해도·`--force` 여도 본문을 **덮지 않는다**(`decide_file_action` 의
      User 분기 = `Keep{new_pending}`). 즉 이번 릴리스의 그 개정은 **신규 설치본에만 자동
      도달**하고, 기존 인구에는 `<파일>.new` 병치 + `cys pack-merge` 를 거쳐야 도달한다.

      - [ ] 이번 diff 에서 위 목록의 파일이 바뀌었는지 확인:
            `git diff <이전태그>..HEAD --name-only -- cysjavis-pack/directives cysjavis-pack/soul.md cysjavis-pack/CLAUDE.md cysjavis-pack/schedule.json cysjavis-pack/agents.json cysjavis-pack/acl.json`
      - [ ] 바뀌었다면 **릴리스 노트 상단에** 해소 명령을 고지한다 —
            `cys pack-merge`(대기 목록) → `cys pack-merge --file directives/MASTER_DIRECTIVE.md --take-new`
            (무수정본) 또는 `--keep-mine`/3-way. **CEO 승격 기계는 md 가 CEO 템플릿 사본이라
            `--take-new` 를 쓰지 말고** preflight `C03.pin.master` 안내(D1(a)형 갱신)를 따른다.
      - [ ] 규칙이 **훅·데몬 등 System 층에도 실려** 기존 인구에서 기계 거동이 성립하는지 확인
            (System 파일은 강제 치유로 전원 도달 — 포인터만 실으면 '문서엔 없는 행'을 가리키는
            이중 진실이 배포된다). 부트 재실행 계약의 그 결박은 검체 `H-DOC-11` 이 단언한다.
      - [ ] 관측: 결손 기계에서 preflight `C03.boot-contract` 가 WARN 으로 뜨는지(부트 차단
            아님 — 병합 대기 가시화). 이 축이 FAIL 로 바뀌면 전 함대 상시 NOT READY 다.
- [ ] **★caller_cache 세대 무효화(P0-2) 채택 조건 — 상태: 미검증 (2026-08-26 R3-RELGATE-6 등재)**

      2차 성찰이 P0-2 채택의 **조건**으로 못 박은 두 실측이다(조건이 조용히 탈락하면 채택
      근거가 성립하지 않은 채 배포된다 — 완료정의 축소). 세대 무효화는
      `resolve_caller_surface` 를 공유하는 20+ 소비자(`check_send_acl` 의 from 등급·
      `usage.event` 귀속·배달 원장·publish surface 태깅 등)의 신원 해석 **시점**을 바꾼다
      (음성 60s 고착 → 등록·claim 마다 재해석). 실측 전까지 '미검증'으로 정직 유지.

      - [ ] **세대 무효화 후 send ACL 판정 무회귀** — 부서 자율성 deny 3형상 + external 등급
            3형상을 재실행해 판정이 종전과 같은지 확인(`cargo test --bin cysd` 의 ACL 검체군
            전량 green + 라이브 1회). 양성 항목이 세대를 보지 않는 계약은 소스 핀
            `caller_cache_positive_ignores_generation` 이 지킨다 — 그 핀이 사라졌으면 적색.
      - [ ] **GUI 타이핑 중 데몬 CPU 프로파일** — 편성/부트로 세대가 연속으로 오르는 구간을
            포함해 30초 샘플. 채택 근거였던 '장수 음성 peer 의 키스트로크당 전 프로세스
            스냅샷 방지'가 편성 폭풍 구간에서 부분 무력화되는지 실측으로 판정한다.
- [ ] 릴리스 노트(RELEASE_NOTES_0.2.0.md) 작성
