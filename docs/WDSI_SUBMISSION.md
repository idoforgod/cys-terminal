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
