# Windows Defender 오탐(WDSI) 신고 — 실측 기록과 절차

> 2026-08-23 실측. 매 릴리스마다 재사용한다. **바이너리 해시가 바뀌면 이전 신고는 새 빌드에 적용되지 않는다.**

## 1. 왜 오탐이 나는가 — 실측된 근본 원인

| 신호 | 현재 상태 | 판정 |
|---|---|---|
| Authenticode 코드서명 | **없음** (`src-tauri/tauri.conf.json` → `bundle.windows = {}`) | ★ 최대 기여 |
| PE 버전 리소스·매니페스트·아이콘 | 있음 (`build.rs:206~223`, `winresource`) | 조치 완료 |
| `%LOCALAPPDATA%` 에 실행파일 배치 후 실행 | 구조상 필수 | 완화 불가 |
| 예약 작업 + 레지스트리 등록 | 구조상 필수 | 완화 불가 |
| 자식 프로세스로 셸 기동(ConPTY) | 구조상 필수 | 완화 불가 |

**결론:** 버전 리소스는 이미 넣었다. 남은 지렛대는 **코드서명 인증서**(항구적 해결)와 **WDSI 오탐 신고**(빌드별 임시 해결) 둘뿐이다.

관측된 탐지명 2종 — 신고 시 **둘 다** 기재한다.
- `Program:Win32/Contebrew.A!ml`
- `Behavior:Win32/Execution.A!ml`

## 2. 자동화가 불가능한 이유 — 실측 (추정 아님)

| # | 측정 | 결과 |
|---|---|---|
| 1 | `GET https://www.microsoft.com/en-us/wdsi/filesubmission` | HTTP 200, 폼 도달 |
| 2 | 익명 제출 가능 여부 | 가능. 단 **CAPTCHA 존재** — `hipSolutionElementA` / `hipSolutionElementV`, 오디오 HIP 라벨, `captcha` 6회 출현 |
| 3 | Software developer 경로 | **로그인 필수** — `homeUserDeclinesLogin`·`enterpriseUserDeclinesLogin` 은 실재하나 개발자용 `*DeclinesLogin` 은 **0개**. `https://login.live.com/me.srf?wa=wsignin1.0` 로 유도 |
| 4 | 기존 브라우저 세션 재사용 | 불가. Claude 확장은 설치돼 있으나(`fcoeoabgfenejglbffodgkkbkcdhcgfn` v1.0.85) **페어링 없음** (`list_connected_browsers` → `[]`) |

두 경로 모두 에이전트가 넘을 수 없는 관문(캡차 / 비밀번호 입력)에 걸린다. **사람 손이 필요한 유일한 항목이다.**

## 3. 주인님 수행 절차 — 약 3분

WDSI 는 *"설치 패키지 전체가 아니라 문제되는 파일만"* 제출하라고 명시한다. 138MB 인스톨러가 아니라 **격리된 실행파일 자체**를 낸다.

1. **윈도우 기기에서** 격리 파일 경로를 확보한다. 통상 `C:\Users\<사용자>\AppData\Local\cys\cys.exe` 와 `cysd.exe`.
   격리돼 사라졌다면 먼저 복원한다 — **순서가 생명이다**:
   ```powershell
   # ① 제외 먼저 (복원 즉시 재격리 방지)
   Add-MpPreference -ExclusionPath "$env:LOCALAPPDATA\cys"
   # ② 그 다음 복원
   & "$env:ProgramFiles\Windows Defender\MpCmdRun.exe" -Restore -Name "Program:Win32/Contebrew.A!ml" -All
   ```
2. 행동 탐지 근거자료를 만든다(선택이지만 채택률을 크게 올린다):
   ```powershell
   & "$env:ProgramFiles\Windows Defender\MpCmdRun.exe" -GetFiles
   # 산출: C:\ProgramData\Microsoft\Windows Defender\Support\MpSupportFiles.cab
   ```
3. https://www.microsoft.com/en-us/wdsi/filesubmission 접속 → **Software developer** 선택 → Microsoft 계정으로 로그인.
4. 폼 입력값:

| 항목 | 값 |
|---|---|
| Microsoft security product | `Microsoft Defender Antivirus (Windows 11)` |
| Company name | CYS Insight |
| File | `cys.exe`, `cysd.exe` (그리고 `MpSupportFiles.cab`) |
| What do you believe this file is? | **Incorrectly detected as malware/malicious** |
| Detection name | `Program:Win32/Contebrew.A!ml` (및 `Behavior:Win32/Execution.A!ml`) |
| Definition version | Windows 보안 → 정보 에 표시된 값 |
| Additional information | 아래 문안 |

5. Additional information 문안:

```
cys-terminal is an open-source multi-agent terminal (AI orchestration workspace)
published at https://www.cysinsight.com/downloads/ and built in public CI at
https://github.com/idoforgod/cys-terminal (GitHub Actions, reproducible from source).

The flagged binaries are the CLI (cys.exe) and its local daemon (cysd.exe), written
in Rust. Behaviour that likely triggers the ML heuristic is inherent to a terminal
multiplexer and is fully documented in the source:
  - creates ConPTY child processes (shells) on the user's behalf
  - listens on a local named pipe (\\.\pipe\cys) for IPC between panes
  - installs a per-user scheduled task so the daemon restarts after logon
  - writes only under %LOCALAPPDATA%\cys

There is no network beaconing, no persistence outside the user profile, no code
injection, and no obfuscation. Binaries carry full PE version resources and are
byte-reproducible from the tagged commit. They are currently unsigned; an
Authenticode certificate is being obtained.

Requesting removal of the false-positive detection.
```

6. 제출 후 **Submission ID** 를 `docs/RELEASE.md` 릴리스 체크리스트에 기록한다. 판정은 보통 24~72시간.

## 4. 항구적 해결 — 코드서명 (주인님 결정 필요)

| 방안 | 비용 | 효과 | 비고 |
|---|---|---|---|
| **Microsoft Trusted Signing** | 약 $9.99/월 | SmartScreen·Defender 평판 즉시 상승 | Azure 구독 + 사업자 확인(업력 3년+) 필요 |
| OV 코드서명 인증서 | 연 $200~400 | 평판 축적에 수 주 | 사업자 확인 필요 |
| EV 코드서명 인증서 | 연 $300~600 | SmartScreen 평판 **즉시** | 하드웨어 토큰 배송, CI 연동 까다로움 |

셋 다 결제·법인 확인이 필요해 에이전트가 진행할 수 없다. 확보되면 `bundle.windows.certificateThumbprint` + `digestAlgorithm` + `timestampUrl` 배선은 자동으로 처리한다.

## 5. 릴리스마다 반복할 것

- [ ] 신규 태그의 `cys.exe`·`cysd.exe` 해시로 **재신고** (해시가 바뀌면 이전 판정 무효)
- [ ] 릴리스 노트에 Defender 안내 섹션 잔존 확인 (`docs/RELEASE.md` 체크리스트 ⑤)
- [ ] `SHA256SUMS.txt` 전 자산 갱신·누락 0

## 제출 기록

### v0.14.31 — 2026-09-10 — 제출하지 않음 (전제조건 미충족, blocked)

> ※ `v0.14.31` 태그는 이후 **발행되지 않았습니다**(태그 빌드의 pack-artifacts 관문이
> `scripts/scan-pack-secrets.sh` 71건으로 멈춤 · 초안 릴리스 폐기). 그 내용은 `v0.14.32`
> 로 재발행됩니다 — 아래 자산 이름은 **발행되지 않은 초안 빌드**의 것입니다.

`~/Desktop/CYSjavis/audit-2026-09-06/impl/wdsi-0.14.31.md` §0·§5-1이 못박은 전제조건
("탐지가 실제로 났는지부터 본다. 나지 않았으면 제출하지 않는다")을 이 실행 환경에서
충족할 수 없어 제출을 시도하지 않았다: v0.14.31을 실제로 설치하고 Windows Defender의
탐지 발생 여부를 관측할 Windows 기기가 이 세션에 없다(에이전트가 접근 가능한 Windows
기기가 CLAUDE.md·CONTRACTS.md 어디에도 등재돼 있지 않음을 확인). 이 전제조건은 §2에
기실측된 로그인/CAPTCHA 벽보다 **앞선** 차단점이다 — 확인되지 않은 탐지를 근거로 폼을
채워 실제 기관(Microsoft)에 제출하는 것은 사실관계가 불확실한 신고가 되므로 회피했다.

준비는 다 되어 있다: `cys_0.14.31_x64-setup.exe`·`cys_0.14.31_x64-setup.zip` sha256은
`impl/release/assets/SHA256SUMS.txt`에 있고, 제출 문안(§4 상당)은
`impl/wdsi-0.14.31.md` §4에 버전 줄만 갱신해 이미 마련돼 있다. 사람이 Windows 기기에서
실제 설치 → 탐지 관측(있다면) → 있을 때만 §3 절차대로 3분 안에 제출하면 된다.
상세: `impl/release/wdsi-result.md`, `impl/release/stage2.md`.

### v0.14.32 — 2026-09-11 — 제출하지 않음 (전제조건 미충족, blocked · v0.14.31과 동일 사유)

`v0.14.31`을 재발행한 태그다(pack-artifacts 관문 실패로 v0.14.31 초안이 폐기되고 내용
동일 + 수정 3건이 v0.14.32로 다시 태그됨). 위 v0.14.31 절의 차단 사유(Windows 기기 부재)가
**환경 변화 없이 그대로** 적용된다 — 새 근거 없이 재시도해도 같은 결과이므로 재시도하지
않았다(C3 §10: 탐지가 관측되지 않으면 제출 자체가 불요 · 1회 규칙 유지).

대상 바이트(v0.14.31과 바이트가 다르다 — 별도 CI 빌드, sha256 재사용 금지):

| 자산 | sha256 |
|---|---|
| `cys_0.14.32_x64-setup.exe` | `a87ba6af512bdae12f9af0dee94dbf937e4679d2fbb03d4936792a467eb15fdd` |
| `cys_0.14.32_x64-setup.zip` | `09df5ecb3fc15f0dd3a94e2ea5bca5fdc18f3f6e114e07b9d5642a2f607bf38d` |

제출 문안은 `impl/wdsi-0.14.31.md` §4의 "The submitted binaries are from release
v0.14.31." 한 줄을 `v0.14.32`로 교체해 그대로 쓴다(C3 §9-6). 사람이 Windows 기기에서
실제 설치 → 탐지 관측(있다면) → 있을 때만 §3 절차대로 제출.
상세: `impl/release/wdsi-result.md`, `impl/release/stage2.md`.

### v0.14.33 — 2026-09-11 — 제출하지 않음 (전제조건 미충족, blocked · v0.14.31/v0.14.32와 동일 사유)

`v0.14.31`(빌드 관문 실패로 폐기)·`v0.14.32`(pack-artifacts 관문 실패로 미발행)를 잇는 재발행
태그다. 위 두 절의 차단 사유(Windows 기기 부재)가 **환경 변화 없이 그대로** 적용된다 — 새 근거
없이 재시도해도 같은 결과이므로 재시도하지 않았다(C3 §10: 탐지가 관측되지 않으면 제출 자체가
불요 · 1회 규칙 유지).

이번 태그는 `release.yml`(pack-artifacts 포함 전 잡 success) → `release-postprocess.py --apply`
→ `release-publish.yml`(오너 위임 승인)까지 **완주해 실제로 공개(public)됐다** — 전판들과 달리
아래 바이트는 지금 실제로 다운로드 가능한 공개 자산이다.

대상 바이트(v0.14.31·v0.14.32와 바이트가 다르다 — 별도 CI 빌드, sha256 재사용 금지):

| 자산 | sha256 |
|---|---|
| `cys_0.14.33_x64-setup.exe` | `39ee4f9bb9b68ad213d3dd0b4cb49985b388c5120ae9f744670ef6973e397d30` |
| `cys_0.14.33_x64-setup.zip` | `2e2d28edad82b4ab205bf149f8725e3af044ed3aced0942a7d7b8204514d9888` |

제출 문안은 `impl/wdsi-0.14.31.md` §4의 "The submitted binaries are from release
v0.14.31." 한 줄을 `v0.14.33`으로 교체해 그대로 쓴다(C3 §9-6). 사람이 Windows 기기에서
실제 설치 → 탐지 관측(있다면) → 있을 때만 §3 절차대로 제출.
상세: `impl/release/wdsi-result.md`, `impl/release/stage2.md`.
