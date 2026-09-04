# cys-terminal User Manual (사용자 매뉴얼)

> 설치부터 AI 함대 운용, 전체 레퍼런스까지. (v0.12.x 기준)
> 무엇을 왜 이렇게 만들었는지는 [Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md)를 보세요.

## 목차

1. [개요와 용어](#1-개요와-용어)
2. [설치](#2-설치)
3. [첫 실행 — 자동 온보딩](#3-첫-실행--자동-온보딩)
4. [터미널 UI](#4-터미널-ui)
5. [AI 함대 운용](#5-ai-함대-운용)
6. [승인 Feed와 승인 서명](#6-승인-feed와-승인-서명)
7. [자원 거버넌스](#7-자원-거버넌스)
8. [스케줄러](#8-스케줄러)
9. [Control Center](#9-control-center)
10. [스킬 보드와 스킬 라이브러리](#10-스킬-보드와-스킬-라이브러리)
11. [업데이트](#11-업데이트)
12. [CYSJavis 팩 운용](#12-cysjavis-팩-운용)
13. [채널 브리지 (Slack·Discord)](#13-채널-브리지-slackdiscord)
14. [기록·증거 (recall / attest)](#14-기록증거-recall--attest)
15. [CLI 레퍼런스](#15-cli-레퍼런스)
16. [환경변수 레퍼런스](#16-환경변수-레퍼런스)
17. [프로토콜 레퍼런스 (RPC·이벤트)](#17-프로토콜-레퍼런스-rpc이벤트)
18. [트러블슈팅 · 알려진 한계](#18-트러블슈팅--알려진-한계)

---

## 1. 개요와 용어

| 용어 | 뜻 |
|---|---|
| **cysd** | 헤드리스 코어 데몬. PTY(세션)·소켓 서버·이벤트·관제 데이터의 소유자 |
| **cys** | CLI. pane 안의 AI(그리고 사람)가 쓰는 동등 노드 클라이언트 |
| **cys.app** | Tauri 데스크톱 앱. 터미널 UI + Control Center — 데몬의 thin client |
| **surface** | PTY 세션 하나. `surface:12` 같은 ref로 주소화된다 |
| **역할(role)** | master·worker·cso·reviewer-* 등. surface에 역할을 등록하면 `--to worker`처럼 역할 이름으로 통신한다 |
| **팩(pack)** | CYSJavis 멀티에이전트 운영체계 — 역할별 절대지침·운영 도구·훅·스킬. `~/.cys/pack`에 설치 |
| **Feed** | 승인 요청함. 에이전트가 위험 작업 승인을 요청하면 여기 모인다 |
| **부서(dept)** | 독립 데몬(소켓 분리)으로 격리된 워크스페이스 묶음 |

핵심 그림: **앱이 아니라 데몬이 세션을 소유**한다. 앱을 껐다 켜도, 앱을 업데이트해도
세션은 살아 있고 앱은 다시 attach만 한다.

---

## 2. 설치

[Releases](https://github.com/idoforgod/cys-terminal/releases/latest)에서 받습니다.
**데몬을 따로 설치할 필요가 없습니다** — 앱이 자동 기동하고 팩도 자동 설치됩니다.
단 하나의 예외: **AI 노드 CLI `claude`(Claude Code)는 동봉되지 않아 별도 설치**가
필요합니다(§2.2 Windows 항목·미설치 시 앱이 안내 카드를 띄웁니다).

### 2.1 macOS (Apple Silicon)

1. `cys_<버전>_aarch64.dmg`를 열고 `cys.app`을 Applications로 드래그.
2. 첫 실행에서 Gatekeeper 경고가 뜨면: 공증된 빌드는 그대로 열리고, 아니면 우클릭 → "열기".
3. 앱이 데몬(cysd)을 자동 기동하고 launchd에 등록합니다(재부팅 후에도 유지).

### 2.2 Windows (x64)

1. `cys_<버전>_x64-setup.exe`(NSIS) 실행 — **자기완결 설치**: 데몬·CLI·런타임(Git Bash·
   Python·Node)이 동봉되어 별도 준비물이 없습니다 — **단 예외 하나, AI 노드 CLI
   `claude`(Claude Code)는 동봉되지 않습니다**. PowerShell 에서
   `irm https://claude.ai/install.ps1 | iex` 1줄로 설치하세요(**git·node 불요** — 네이티브
   설치기). 미설치여도 앱·터미널은 정상 동작하며 AI 각성만 안 됩니다(앱이 설치 안내 카드를
   띄웁니다 — 설치 후 자비스 재시작).
2. 앱을 1회 실행하면 온보딩이 자동으로 팩 설치·훅 등록·데몬 자동 기동(작업 스케줄러
   ONLOGON)을 마칩니다.
3. 확인: `dir %USERPROFILE%\.cys\pack` · `schtasks /Query /TN cysd`
4. 상세(비기술자용 안내 포함): [INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md)

### 2.3 데몬 상시 가동 (24/365, 선택)

```bash
cys daemon install      # macOS launchd KeepAlive / Windows 작업 스케줄러 등록
cys daemon status
cys daemon uninstall
```

이미 데몬이 돌고 있으면 `install`은 안전하게 거부됩니다.

### 2.4 외부 터미널에서 `cys` 쓰기 (셸 설치)

- macOS(권장): Control Center 헤더 → **"셸에 cys 설치"** 1클릭(관리자 승인 1회) — `/usr/local/bin/cys`·`cysd` 심볼릭 링크가 생기고, 앱 업데이트에도 자동 추종합니다. 이 버튼은 macOS에서만 나타납니다.
  - **알림 등급 규칙: 확인할 것이 하나라도 남으면 ✅ 가 아니라 ⚠ 입니다.** 링크는 만들어졌어도 백업이 일어났거나 `cysd`가 가려졌다면 제목이 "⚠ 셸 설치 완료 — 확인할 항목이 있습니다" 가 되고 본문에 그 항목이 적힙니다(설치 자체가 실패한 것은 아닙니다). 한 알림은 한 등급만 주장합니다 — ⚠ 한 줄이 ✅ 알림 안에 숨지 않습니다.
  - **알림은 액션당 하나입니다.** 설치·해제 결과와 "지금 남아 있는 것"(백업본·남의 파일)이 한 알림에 함께 옵니다. 예전에는 같은 사실이 서로 다른 문장의 알림 두 개로 나뉘어 떴습니다.
  - 결과 알림이 다음 셋 중 하나로 오면 **아직 끝난 게 아닙니다**(경고 알림은 60초 유지 · 이후 Control Center **알람 탭**에서 다시 볼 수 있습니다):
    - **"다른 cys가 앞을 가립니다"** — 링크는 생겼지만 PATH 앞쪽의 다른 cys가 먼저 잡힙니다. 알림이 그 경로를 지목합니다. 다만 확인 명령이 뱉은 줄이 **경로 한 개로 읽히지 않을 때**(로그인 프로필이 배너를 찍었거나 `cys`가 셸 함수인 경우)는 "가리는 경로를 하나로 특정하지 못했습니다"라고만 말하고 **지울 대상을 단정하지 않습니다** — 검증되지 않은 문자열을 근거로 파일을 지우라고 안내하지 않기 위해서입니다.
    - **"PATH에서 cys를 찾지 못했습니다"** — 확인은 정상 종료했고, 그 셸의 PATH에 `/usr/local/bin`이 없는 경우입니다(설치 실패가 아닙니다).
    - **"확인 불가"** — 확인 명령을 실행하지 못했거나 비정상 종료·무응답이라 무엇이 잡히는지 모르는 경우입니다.
    어느 쪽이든 새 터미널에서 `which -a cys` 로 1순위가 `/usr/local/bin/cys` 인지 확인하세요(조치 안내는 docs/INSTALL.md §B). 판정은 **로그인 셸(`$SHELL`, 기본 zsh) 기준**입니다.
  - ⚠ **이 경고는 거짓일 수 있습니다.** 확인은 **비대화형 로그인 셸**(`$SHELL -lc`)로 하는데, 그 셸은 zsh면 `~/.zshenv`·`~/.zprofile`·`~/.zlogin` 만 읽고 **`~/.zshrc`는 읽지 않습니다**(bash면 `~/.bash_profile` 만 읽고 `~/.bashrc`는 읽지 않습니다 — 실측 2026-08-25). PATH를 `~/.zshrc`에만 넣어 두었다면 터미널에서 `cys`가 잘 동작하는데도 이 경고가 뜹니다 — 그때는 **무시하면 됩니다**. 정말 고쳐야 한다면 `~/.zshrc`가 아니라 위에 적힌 **읽히는 파일**(예: `~/.zprofile`)에 `/usr/local/bin`을 추가하세요.
  - **그 자리에 이미 무언가 있으면 무엇이든 백업합니다.** 실제 파일(다른 도구 설치본)이든 **다른 앱을 가리키던 심볼릭 링크**든, 설치는 그것을 지우지 않고 `<원래 경로>.cys-backup-<숫자>` 로 옮겨 보관한 뒤 새 링크를 만듭니다(백업 경로는 결과 알림·버튼 툴팁에 표시). 백업하지 않는 유일한 경우는 **이미 이 앱을 가리키는 링크**일 때입니다. 그런 것이 있으면 버튼을 누르기 전에 고지가 먼저 뜹니다.
    - `<숫자>`는 백업한 시각의 **epoch 초**(숫자)입니다 — 사람이 읽는 날짜가 아닙니다. 예: `/usr/local/bin/cys.cys-backup-1756089600`. 이름 규칙과 손으로 되돌리는 절차는 docs/INSTALL.md §B **"백업본을 손으로 되돌리기"** 에 앱과 무관하게 읽을 수 있도록 적어 두었습니다.
    - (2026-08-25 이전 안내는 "다른 곳을 가리키던 링크는 백업 없이 새 링크로 바뀝니다"라고 적었는데 **사실이 아니었습니다.** 지금은 그 링크도 백업합니다.)
  - **`cysd`(데몬)가 가려질 수 있습니다.** 설치는 `cys`·`cysd` 두 링크를 만들고 둘 다 확인합니다. PATH 앞쪽에 다른 `cysd`가 있으면 터미널에서 뜨는 데몬이 이 앱의 것이 아니게 되어 **앱과 데몬 버전이 어긋날 수 있습니다.** 이때 결과 알림은 성공(✅)이 아니라 **경고(⚠)** 로 내려가고 어떤 경로가 앞에 있는지 알려 줍니다(예전에는 이 경고가 "✅ 셸 설치 완료" 알림 본문 속 한 줄로 섞여 나가 사실상 읽히지 않았습니다). 새 터미널에서 `which -a cysd` 로 1순위가 `/usr/local/bin/cysd` 인지 확인하세요. 확인 항목은 알림 **본문 맨 앞**에 옵니다 — 성공 서술을 먼저 읽고 창을 닫아 버리지 않도록 자리를 앞으로 옮겼습니다(등급 규칙은 그대로입니다).
  - **Control Center 상태 알림의 제목은 "실제로 무엇이 있는가" 만 말합니다.** 위 안내들은 Control Center를 열 때마다 상태 알림으로 다시 뜨는데, 제목은 **네 갈래**입니다 — ①`/usr/local/bin` 에 이 앱의 링크가 **하나도 없고 남의 것이 있을 때** "⚠ /usr/local/bin 에 이 앱의 것이 아닌 cys 파일이 있습니다" ②`cys`·`cysd` 중 **한쪽만 이 앱의 링크일 때**(반쪽 상태) "⚠ 셸 cys 설치가 한쪽만 되어 있습니다 — 아래 내용을 확인하세요" ③백업본이 남아 있으면 "설치 때 백업해 둔 원본이 남아 있습니다" ④그 밖의 안내(그림자·확인 실패)만 있으면 ⚠ 없는 중립 제목 "셸 cys 설치 상태 안내". 알릴 것이 없으면 아무 알림도 뜨지 않습니다. (2026-08-25 이전에는 안내 문장이 하나라도 있으면 무조건 ①제목이 나가서, 남의 파일이 하나도 없는 정상 사용자가 Control Center를 열 때마다 **거짓 경고**를 봤습니다. ②는 같은 날 뒤늦게 생긴 갈래입니다 — 남의 실제 `cysd` 가 자리를 차지한 반쪽 상태가 ⚠ 도 경고 테두리도 없는 중립 안내로 나가고 있었기 때문입니다. 그때 docs/INSTALL.md 만 "네 갈래"로 고쳐지고 이 문서는 "세 갈래"로 남아 있었습니다.)
  - **위 셋 중 하나로 끝났다면 버튼은 "셸에 cys 다시 설치"로 남습니다.** 링크 자체는 생겼기 때문에 예전에는 곧바로 **"셸 cys 해제"** 로 바뀌었고, "아직 끝나지 않았다"는 안내를 읽은 분이 같은 자리를 누르면 재시도가 아니라 **해제**가 실행됐습니다. 지금은 라벨과 실제 동작이 항상 같습니다. 해제하려면 Control Center를 닫았다 다시 열면 현재 상태에 맞는 라벨로 돌아옵니다.
  - 해제: 같은 버튼이 설치된 상태에서는 **"셸 cys 해제"** 로 바뀝니다(확인 창 → 관리자 승인 1회 → 심볼릭 링크만 제거). 건너뛴 항목·남은 파일이 있으면 **"부분 완료"** 알림이 사유와 복구 명령(`sudo rm <경로>`)을 그대로 보여줍니다. 이 `sudo rm` 은 **이 앱이 만든 링크 중 지우지 못하고 남은 것**에만 붙습니다 — "건드리지 않았습니다"라고 건너뛴 남의 파일에는 붙지 않습니다(확인 창의 약속을 화면이 스스로 어기지 않도록). 그리고 그 안내에는 `ls -l <경로>` 로 심볼릭 링크인지 **먼저 확인하라**는 절차가 함께 붙습니다 — 지우지 못하고 남은 자리에 남의 실제 파일이 와 있을 수 있기 때문입니다.
  - **해제 확인 창은 설치 때 백업해 둔 원본이 있으면 그 경로와 되돌리는 방법을 함께 보여 줍니다.** 그리고 **해제는 그 원본을 제자리에 되돌립니다**(우리 링크를 지운 자리에만 · 남의 파일 위에는 덮지 않습니다). 되돌린 경로는 결과 알림에 나오고, 되돌리지 못하고 남은 백업본은 계속 고지됩니다. ⚠ **한 자리에 백업본이 여러 개면 되돌아오는 것은 숫자가 가장 큰 하나뿐이고, 옛 백업본은 자동으로는 돌아오지 않습니다**(되돌릴 자리가 하나뿐입니다) — 그러니 계속 뜨는 고지를 보고 지우기 전에, docs/INSTALL.md §B "백업본을 손으로 되돌리기" 로 그것이 무엇인지 확인하고 직접 되돌리세요. **그 고지에 붙는 되돌리기 명령은 `sudo mv -n …`(덮어쓰기 금지) 이고, 화면에 함께 붙는 뜻풀이는 "원래 자리가 비어 있어야 옮겨집니다 — 실제 파일이나 살아 있는 링크가 있으면 아무 일도 일어나지 않습니다 · 다만 가리키던 대상이 이미 사라진 링크(끊어진 바로가기)는 비어 있는 것으로 보아 덮어씁니다" 입니다** (예: `sudo mv -n /usr/local/bin/cys.cys-backup-1756089600 /usr/local/bin/cys`). 뒤쪽 단서는 실측입니다 — macOS(BSD) `mv -n` 은 대상을 따라가서 있는지 보기 때문에, 끊어진 바로가기는 "비어 있다"로 읽고 덮어씁니다(2026-08-25 실측 · 거절할 때에도 종료코드가 0이라 결과는 `ls -l` 로 확인해야 합니다). 그래서 문서의 되돌리기 절차와 앱의 자동 되돌리기는 `-n` 하나에 기대지 않고 `[ -e ]`·`[ -L ]` 두 검사로 자리가 빈 것을 먼저 확인합니다 (docs/INSTALL.md §B "백업본을 손으로 되돌리기"). 그리고 그 자리를 이 앱의 링크가 차지하고 있다고 앱이 아는 경우에는 옮기는 명령을 **아예 내지 않고** "해제를 누르면 앱이 되돌립니다"라고만 말합니다. 자리 상태를 확정하지 못하면 `ls -l <원래 경로>` 로 먼저 확인하라고 안내합니다. ★**그리고 옮기는 명령이 붙는 것은 한 자리에 대해 최신 사본 하나뿐입니다** — 같은 자리의 옛 사본에는 자리 상태가 어느 갈래이든 붙지 않습니다. 목적지가 같기 때문입니다: 두 사본에 같은 목적지로 가는 명령이 나란히 나오면 뒤에 실행한 줄이 앞 줄이 되돌려 놓은 파일을 조용히 덮어쓰고, `mv -n` 은 거절할 때에도 종료코드가 0이라 화면에 오류도 남지 않습니다. 옛 사본에 대해서는 "이 사본은 **자동으로 되돌아오지 않습니다**"라는 사실만 말하고, 최신 사본이 제자리로 돌아간 **뒤에** `ls -l <원래 경로>` 로 그 자리를 보고 판단하라고 안내합니다(2026-08-25 12라운드에 좁힌 문장입니다 — 그전에는 옛 사본에도 같은 목적지의 명령이 함께 나올 수 있었습니다). 백업이 없으면 그 문장은 아예 나오지 않습니다.
  - 잔존 백업본은 Control Center를 열 때마다 **상태 알림과 버튼 툴팁에 상시** 표시됩니다 — 설치 직후 알림(60초)을 놓쳐도 자기 원본이 어디로 갔는지 다시 확인할 수 있습니다.
  - ✅ **"되돌렸습니다"는 실패가 아닙니다.** "⚠ 부분 완료"는 실제로 지우지 못한 것이 있거나, 우리 것이 아니어서 건너뛴 것이 있을 때만 뜹니다.
- macOS(폴백 · GUI를 못 쓸 때): ⚠ **`sudo ln -sfn …` 를 단독으로 치지 마세요** — `ln -f` 는 그 자리에 다른 도구가 설치한 **실제** `cys` 가 있어도 묻지 않고 지우고 링크로 갈아 끼웁니다(백업 없음 · 복구 불가). 버튼은 2026-08-25부터 지우지 않고 백업하는데 수동 절차만 옛 형태로 남아 있었고, 그래서 지금은 **문서 어디에도 맨 `ln -sfn` 한 줄을 두지 않습니다.** 먼저 `ls -l /usr/local/bin/cys /usr/local/bin/cysd` 로 무엇이 있는지 보고, 설치는 docs/INSTALL.md §B 의 **"폴백 — 수동 sudo"** 블록을 통째로 복사해 쓰세요 — 버튼과 같은 순서(있으면 `<원래 경로>.cys-backup-<epoch초>` 로 백업 → 그다음 링크 · 이미 이 앱의 링크면 백업하지 않음=멱등)로 돌고, 백업 이름도 버튼이 만드는 것과 같아 나중에 앱이 그것을 찾아내 되돌립니다. **해제** 수동 폴백도 같은 절에 있습니다(심볼릭 링크이고 그 대상이 `cys.app` 안일 때만 지웁니다 — 맨 `sudo rm` 은 남의 실제 파일도 지웁니다).
- Windows: 앱 pane 안에서는 PATH가 자동 주입되어 cys를 바로 쓸 수 있습니다. **설치기(setup.exe)는 외부 PATH를 등록하지 않습니다** — 현재 사용자 설치(`installMode: currentUser` · `%LOCALAPPDATA%\cys`)이고 PATH 항목을 쓰지 않습니다. 외부 터미널에서 쓰려면 `%LOCALAPPDATA%\cys` 를 PATH에 추가하거나 전체 경로로 실행하세요. (자동 PATH 편집은 제공하지 않습니다 — Windows PATH 자동 편집은 값 잘림·확장 변수 손상 사고가 알려져 있어 의도적으로 넣지 않았습니다. 참고: 폐기된 구 MSI는 PATH를 등록했으나 더 이상 배포되지 않습니다 — docs/INSTALL.md §Windows.)

### 2.5 설치 확인

```bash
cys ping            # 데몬 응답 확인
cys identify        # 데몬·내 주소 확인
cys status          # 전 노드 관제 보드
cys doctor          # 자기진단 (문제 시 --fix)
```

### 2.6 제거

1. `cys daemon uninstall`
2. 앱 삭제(macOS: 앱을 지우기 전에 §2.4의 **"셸 cys 해제"** 로 심링크를 먼저 정리 → Applications에서 제거 / Windows: 설정 → 앱 → 'cys' 제거 · PATH는 애초에 등록하지 않으므로 정리할 것이 없습니다)
3. 선택 — 데이터까지 완전 삭제(비가역): `~/.cys`(팩·설정)와 `~/.local/state/cys`(소켓·
   관제 DB) 삭제. 장기기억·soul.md도 함께 사라지므로 백업 후 진행하세요.

상세: [INSTALL.md](docs/INSTALL.md)

---

## 3. 첫 실행 — 자동 온보딩

앱 첫 실행 시 자동으로 수행됩니다(멱등 — 다시 실행해도 안전):

- 데몬 자동 기동(끄려면 `CYS_NO_AUTOSTART=1`) 및 상시 가동 등록
- 팩 설치(`~/.cys/pack`) — **이미 사용자가 수정한 파일(soul.md·디렉티브·CLAUDE.md·
  schedule.json)은 보존**되고, 수정하지 않은 파일만 갱신됩니다
- Claude Code SessionStart 훅 등록(역할 지침 자동 주입용 — `CYS_ROLE` 세션에서만 발동)
- pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET` 자동 주입

---

## 4. 터미널 UI

### 4.1 상단바

`+ New`(⌘T) · `Split →`(⌘D) · `Split ↓`(⌘⇧D) · `정렬`(역할 표준 배치) · `Close`(⌘W) ·
`Files`(파일 트리) · `Control Center`(승인 대기 배지) · `Update`(업데이트 배지) · `테마`.
좌측에 데몬 연결 상태가 표시됩니다.

### 4.2 pane 분할·이동·정렬

- 분할선을 드래그해 비율 조정.
- **pane 헤더를 드래그**해 다른 pane의 상/하/좌/우에 드롭 — 자유 재배치.
- **pane 전출**: pane 헤더를 좌측 사이드바의 **워크스페이스 탭 위로 드래그**하면 그
  워크스페이스로 이동합니다. 같은 데몬이면 즉시 이동(세션 유지), **다른 부서(다른 데몬)**면
  핸드오프 문서로 맥락을 승계한 **재기동**입니다 — 진행 중이던 응답(라이브 추론 상태)은
  이어지지 않으며, 확인 창에서 고지됩니다. 실패 시 원본 pane은 보존됩니다.
- `정렬` 버튼: 역할 기반 표준 배치(좌측 master/CSO · 가운데 worker · 우측 리뷰어).
- pane 닫기(×)와 워크스페이스 삭제는 **2-클릭 확인**(첫 클릭 후 2.5초 내 재클릭)입니다.

### 4.3 워크스페이스 탭·그룹

좌측 사이드바에서 워크스페이스를 전환합니다. 탭에는 pane 수·대표 제목·노드 상태·최악
컨텍스트%·승인 대기 `⚠` 배지가 표시됩니다. 탭은 드래그로 재정렬하고, 우클릭 메뉴로
**그룹**(접기·고정·색상·이름)을 만들 수 있습니다.

### 4.4 부서 (독립 데몬 워크스페이스)

`＋부서` 버튼으로 부서 워크스페이스를 만들면 **별도 cysd 데몬(별도 소켓)**이 뜹니다 —
프로젝트 간 장애·자원·통신이 격리됩니다. Control Center "작업" 탭과 `cys fleet`은 모든
부서를 집계해 보여줍니다.

- **마스터 선언 → 부서 자동 생성(위계 폴백)**: 이미 살아있는 마스터(CEO)가 있는 조직에서
  **오너가 직접 타이핑한** "너는 마스터다" 선언(base 레인·unix)은 거부로 끝나지 않고,
  새 부서를 자동 생성해 부서장(claude)·팀을 기동합니다. 첫 부서 생성 시 기존 마스터는
  CEO로 자동 승격됩니다. 에이전트가 배달한 선언·스크립트 직접 실행은 이 폴백을 타지
  않습니다(폭주 봉인 — 거부 안내만).
- **★`＋부서` 버튼으로 만든 부서도 팀이 자동으로 뜹니다 (v0.14.23)**: 부서의 팀 기동에는
  CEO가 발급한 **허가증(티켓)** 이 필요한데, 종전에는 위 '선언 폴백' 경로에만 자동 발급이
  배선돼 있어 **버튼으로 만든 부서는 부서장이 혼자 깨어난 채 멈췄습니다**. 이제 부서를
  만드는 행위 자체를 발급 동의로 보고 생성 직후 허가증을 발급하며, 혹시 허가증이 없으면
  **부서장이 스스로 CEO에게 요청**해 받아 팀을 기동합니다. 발급이 실패해도 부서 생성은
  무너지지 않습니다(경고 + 수동 발급 안내 후 단독 각성 유지).
- **★부서장은 자기를 '부서장'으로 압니다 (v0.14.23)**: 부서 운영헌장(soul)을 처음 만들 때
  부서 이름이 박힌 정체 문단이 새겨집니다. 부서 운영헌장은 **본부 운영헌장을 물려받아** 만들어
  지므로, 본부에만 유효한 문단(예: "이 데몬의 마스터는 CEO다")이 그대로 넘어가면 부서장이 자기
  정체를 오인합니다. 그래서 **승계 금지 마커**가 붙은 문단은 물려줄 때 제거됩니다.

  **마커 표기법** — 제외하려는 문단(heading) 안에 아래 한 줄을 넣습니다. 줄 전체가 이 주석이어야
  인식되며, 그 문단과 그 아래 하위 문단이 함께 제외됩니다. 제외 사실은 감사 기록에 남습니다.

  ```
  <!-- cys:no-inherit -->
  ```

  마커가 없는 문단은 **줄바꿈 형식까지 글자 그대로** 승계됩니다. 안전장치로 **문서의 첫 최상위
  문단(= 문서 몸통)은 마커가 붙어 있어도 제외되지 않고**, 제외한 결과가 빈 문서가 되면 **제외를
  전부 취소하고 원문을 그대로 물려줍니다.** 제외·취소 사실은 감사 기록에 남습니다.
- **★오너는 부서의 모든 창에 입력할 수 있습니다 (v0.14.23)**: 부서에는 "바깥에서 워커를
  직접 조종하지 못한다"는 규칙이 있는데, 종전에는 그 '바깥'에 **오너의 GUI까지 포함**돼
  부서 워커 창 입력이 `acl denied: external → worker` 로 거부됐습니다. 이제 데몬이 GUI
  세션을 **오너 등급**으로 식별해 전 노드 입력을 허용하고, CEO·타 노드가 부서장을 건너뛰어
  워커를 조종하는 것은 **그대로 차단**합니다.
- **킬스위치 `CYS_DEPT_FALLBACK=0`**: 위 자동 생성을 끄고 구계약(거부 rc=7 + 안내)으로
  되돌립니다(무배포 현장 롤백 채널). 예: `CYS_DEPT_FALLBACK=0` 을 셸 환경에 넣고 앱/데몬을
  재시작.

### 4.5 입력

- **한글 IME**: macOS에서 조합 중 자모 유출을 막는 상태 머신이 내장되어 있습니다.
- **붙여넣기**: ⌘V/Ctrl+V (bracketed paste 보존). **클립보드 이미지**를 붙여넣으면 임시
  파일로 저장된 경로가 타이핑됩니다(iTerm2 방식).
- **파일 드래그&드롭**: pane을 정확히 겨눠 드롭하면 경로가 입력됩니다(에이전트 pane은
  `@경로` 멘션, 일반 pane은 셸 인용 경로 — 자동 실행 없음, Enter는 직접). pane을 빗나간
  드롭은 주입하지 않고 안내만 띄웁니다. 에이전트가 응답 중이면 삽입 전에 확인을 묻습니다.

### 4.6 파일 트리

`Files` 버튼 — 포커스 pane의 현재 디렉터리를 루트로 트리를 보여주고(cd 추적), 헤더
아래에 루트의 **전체 경로**가 상시 표시됩니다.

- **파일 클릭**: 시스템 기본 앱으로 열기. 실패는 토스트로 알리고, **실행 파일**은 확인
  창을 거칩니다. 실행은 열기와 다를 수 있어 기본 차단입니다.
- **파일/폴더를 pane으로 드래그**: 경로 삽입(위 4.5 드롭과 동일 규칙).
- **우클릭 메뉴**: 전체 경로 표시·경로 복사·열기·Finder에서 보기·특정 pane에 경로 삽입·
  (폴더) 패널 루트 이동·cd 텍스트 삽입(전송 없음).
- **폴더 더블클릭**: 트리 루트를 그 폴더로 이동(📌 고정 — pane 경로 자동 추종 일시 해제,
  헤더의 📌를 클릭하면 추종 복귀). `▴ ..` 행을 더블클릭하면 한 단계 위로.

### 4.6b 스크롤·복사 (마우스)

- **휠 스크롤**: 일반 pane 과 inline TUI(macOS 의 claude 기본 화면 등)에서는 휠이 항상 이
  pane 의 스크롤백을 움직입니다(위로 굴리면 지나간 내용 읽기, 바닥 복귀·키 입력 시 다시
  최신 출력 따라감). **마우스를 요청하는 전체화면 TUI** — claude fullscreen·codex·vim
  `mouse=a`·tmux `mouse on` 등 — 에서는 휠이 **앱 자체 스크롤**로 들어갑니다(macOS —
  예: claude fullscreen 은 트랜스크립트가 굴러갑니다). 마우스를 요청하지 않는 전체화면
  앱(less·man)은 종전처럼 휠로 페이지가 굴러갑니다.
- **드래그 선택·복사**: 마우스 드래그로 선택하고 ⌘C(또는 우클릭 메뉴)로 복사합니다.
  **전체화면 마우스 TUI 에서는** 일반 드래그가 앱으로 가므로 **Option+드래그로 선택한 뒤
  ⌘C** 로 복사하세요.
- **앱 마우스 킬스위치**: 일반 화면에서도 클릭·드래그·휠 전부를 앱이 직접 받아야 하면
  `touch ~/.cys/allow-app-mouse` 후 **새 pane**(되돌리기 `rm ~/.cys/allow-app-mouse` ·
  env `CYS_ALLOW_APP_MOUSE=1` 동등). ⚠ **Windows 에서는 켜지 마세요** — ConPTY 가 마우스
  시퀀스를 깨뜨려 입력창에 `[555;98;34M` 같은 리터럴이 타이핑됩니다.
- **Windows 휠 가드**(v0.14 품질 라인 신규): Windows 에서는 마우스를 앱에 넘기는 경로가
  없어(위 킬스위치 금지 — ConPTY 결함) 전체화면 TUI 의 휠이 **방향키로 합성돼** 앱에
  들어갑니다. claude fullscreen 에서는 그것이 프롬프트 히스토리를 헤집어 **입력창이 지나간
  프롬프트로 바뀌어 버리므로**, 그 조합에서만 휠을 **소비**합니다(= 그 노치는 스크롤도
  방향키도 아닌 무동작이 됩니다). vim `mouse=a` 처럼 휠→방향키가 정상 UX 인 앱은 대상이
  아닙니다 — 다만 이 구분은 "앱이 어떤 마우스 추적 모드를 켰는가"로 기계가 판정하므로,
  그 모드(any-motion)를 켜는 vim 설정·플러그인 조합이라면 그 앱의 전체화면 휠도 무동작이
  될 수 있습니다. 그때는 아래 스위치로 가드를 통째로 끄십시오.
  · (억제 조건은 그 하나뿐) 위 판정에 걸리지 않는 앱 — `less`·`man` 처럼 마우스를 요청하지
  않는 전체화면 앱 — 의 휠은 **어떤 환경에서도 종전 그대로**입니다.
  · (적용 범위) 가드는 **앱이 마우스 모드를 선언한 뒤** 걸립니다. 그래서 GUI 를 재시작해
  **이미 떠 있던 전체화면 앱에 다시 붙은 pane**(세션 복원·재부착)에서는 걸리지 않을 수
  있습니다 — 재부착이 보내는 초기 화면 스냅샷에는 앱의 마우스 모드 선언이 들어 있지 않기
  때문입니다. 그 pane 에서 휠 오염이 보이면 앱을 한 번 나갔다 다시 들어가거나(모드 재선언)
  pane 을 새로 여세요.
  · (알려진 증상) 전체화면 앱이 **비정상 종료**(강제 종료·크래시)한 뒤 그 pane 의 휠이 계속
  안 들으면, 앱이 마우스 모드를 끄는 시퀀스를 못 보내고 죽어 판정이 남아 있는 경우입니다 —
  **pane 을 새로 여세요**(정상 종료·전체화면 이탈에서는 자동으로 풀립니다).
  · **끄는 법(롤백)** — 아래 중 하나 후 **새 pane**(이미 열린 pane 은 그대로입니다):
    - PowerShell(Windows 기본 셸): `New-Item -ItemType File -Force $HOME\.cys\win-wheel-guard-off`
      · 되돌리기 취소 `Remove-Item $HOME\.cys\win-wheel-guard-off`
      ※ `touch` 는 PowerShell·cmd 에 **없는 명령**입니다. 아래 macOS/Linux 표기와 혼동하지 마세요.
    - macOS·Linux 셸: `touch ~/.cys/win-wheel-guard-off` · 되돌리기 취소 `rm ~/.cys/win-wheel-guard-off`
    - env `CYS_WIN_WHEEL_GUARD_OFF=1` 도 동등하지만 **GUI 프로세스가 그 값을 상속해야** 합니다 —
      터미널에서 `setx` 로 설정해도 **이미 떠 있는 cys 에는 반영되지 않으니 GUI 를 재시작**하세요.
      (파일 방식은 새 pane 을 열 때마다 확인하므로 재시작이 필요 없습니다 — Windows 권장 수단입니다.)
    - devtools 가 있는 빌드라면 `localStorage.cysWinWheelGuardOff="1"` 도 동등(릴리스 빌드에는
      devtools 가 없어 최종 사용자 수단이 못 됩니다).
  · ⚠ **이것은 결함을 되살리는 스위치입니다.** 끄면 Windows 전체화면 휠이 다시 방향키로
  합성되므로, claude fullscreen 에서 휠을 굴리면 **프롬프트 입력창이 다시 오염될 수
  있습니다**(이번 릴리스가 막은 바로 그 증상). 즉 "vim 등에서 휠→방향키가 꼭 필요하다"는
  이유로만 끄고, 켜 둔 채 claude fullscreen 을 쓰면 그 오염을 감수하는 것입니다.
  · ⚠ 이 용도로 `allow-app-mouse` 를 켜지 마세요 — 그것은 입·출력 양측을 열어 위의 ConPTY
  리터럴 타이핑(`[555;98;34M` 이 입력창에 찍히는 결함)을 되살립니다. **휠 억제만** 끄는
  전용 스위치가 이것입니다.
- **Windows 에서 claude 를 전체화면 대신 inline 으로 띄우기**(옵트인 · **기본 off**): 위 휠 가드는
  오염을 *막을* 뿐 화면 모드를 바꾸지는 않습니다. claude 자체를 inline 으로 띄우고 싶다면
  `New-Item -ItemType File -Force $HOME\.cys\win-no-alt-screen` 후 **`cys launch-agent` 로 새
  pane 을 기동**하세요. 그 pane 의 claude 에 `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` 이 실립니다
  (macOS 는 이 스위치 없이 늘 실립니다).
  · 되돌리기 `Remove-Item $HOME\.cys\win-no-alt-screen` · env `CYS_WIN_NO_ALT_SCREEN=1` 도
  동등하지만 **GUI 가 상속한 값만** 읽으므로 `setx` 후 GUI 재시작이 필요합니다(권장은 파일).
  · **왜 Windows 만 기본 off 인가**: 이 env 가 Windows 의 claude 를 깨뜨리면 `cys boot` 로 띄운
  pane 이 **한꺼번에** 죽는 경로입니다. 실기 Windows 에서 그 확인(`cys boot` 4종 노드 정상 기동)을
  마치기 전에는 안전한 쪽을 기본값으로 둡니다 — 켜지 않아도 **휠 오염은 위 휠 가드가 막습니다**.
  · **적용 범위**: 이미 열린 pane 의 재기동(node-recover)·GUI 에서 연 pane 에는 실리지 않습니다
  (**새 surface 를 만들며 기동하는 `cys launch-agent` 한정**). 팩 `agents.json` env 에 `"0"` 을
  적어 두었다면 켜도 주입하지 않습니다(사용자 값이 언제나 우선).
- 전체화면 마우스 동작이 이상하면 정합기 롤백 스위치로 구동작에 복귀할 수 있습니다:
  콘솔에서 `localStorage.cysMouseReconcilerOff="1"`(새 pane 부터 적용). ※ 이것과 위 Windows
  휠 가드는 **다른 스위치**입니다 — 정합기 스위치는 출력 정합(mac 소비 경로)을, 휠 가드
  스위치는 Windows 휠 억제만 끕니다. ⚠ **Windows 에서는 정합기 스위치를 꺼도 휠 가드는 그대로
  삽니다** — 정합기(출력 소비)는 원래 macOS 전용이라 이 스위치는 Windows 에서 실질적으로 하는
  일이 없었고, 이번에 생긴 휠 가드도 이 스위치로는 꺼지지 않습니다. Windows 휠 가드를 끄는
  스위치는 위의 `win-wheel-guard-off` 하나뿐입니다.
- 릴리스 마이그레이션이 계정 settings(fullscreen `tui` 키 제거 등)를 정규화할 때는 반드시
  같은 자리에 `.bak-*` 백업을 먼저 남깁니다 — 되돌리기는 그 백업 복원입니다.

### 4.7 테마·폰트

- 다크 테마 고정 + **배경색 커스텀 피커**(`테마` 버튼). 밝은 배경을 고르면 글자색이 자동
  보정됩니다. OS 라이트/다크 자동 전환은 없습니다.
- **터미널 폰트 선택**: `테마` 버튼 → 폰트 드롭다운(기본값·Menlo·SF Mono·Cascadia Mono·
  Consolas·JetBrains Mono·D2Coding 등, 기억됨). 미설치 폰트는 기본 스택으로 자동 폴백,
  한글 폴백은 항상 보존됩니다.
- 터미널 폰트 크기: ⌘+ / ⌘- / ⌘0 (8–32px, 기억됨).

### 4.8 ⌘K Command Palette

퍼지 검색으로: 노드 점프 · 컨텍스트 60%+ 노드 순회 · 역할별 재기동 · 가장 오래된 승인
처리 · 새 탭/분할 · Control Center 토글 등을 키보드로 실행합니다.

### 4.9 Glance 모드 (⌘G)

비기술자용 큰 글씨 요약 화면(Live↔작업 전환)과 엔지니어용 상세 탭 화면을 오갑니다.

### 4.10 단축키 요약

| 키 | 동작 |
|---|---|
| ⌘T / ⌘D / ⌘⇧D / ⌘W | 새 pane / 가로 분할 / 세로 분할 / 닫기 |
| ⌘K | Command Palette |
| ⌘G | Glance/Ops 밀도 전환 |
| ⌘+ ⌘- ⌘0 | 폰트 크기 |
| ⇧Enter / ⌥Enter | 프롬프트 줄바꿈(개행 삽입 — 실행 아님) |

---

## 5. AI 함대 운용

### 5.1 역할과 주소

surface는 `surface:N` ref로, 역할 등록된 노드는 역할 이름으로 주소화됩니다.
역할 글롭도 됩니다: `--to 'reviewer-*'` = 리뷰어 전원 브로드캐스트.
역할 노드 pane은 제목 앞에 역할색 깜박이 점이 표시됩니다(master 파랑 · cso 보라 ·
worker 초록 · reviewer-gemini 주황 · reviewer-codex 시안 — Control Center와 동일 색).

```bash
cys identify                          # 나는 누구인가 (surface ref)
cys claim-role worker                 # launch-agent 없이 시작한 세션을 역할로 등록
cys surface-role                      # 데몬이 알고 있는 내 역할 1단어 출력
```

### 5.2 노드 기동

```bash
cys launch-agent --role worker --agent claude   # surface 생성 + CLI 기동 + 절대지침 자동 주입 + 역할 등록
cys boot                                        # 표준 노드 세트 일괄 기동(설치된 CLI 자동 감지)
```

`launch-agent`는 ①surface 생성(CYS_ROLE 주입) ②에이전트 CLI 기동 ③역할 절대지침 stdin
주입 ④역할 레지스트리 등록을 한 번에 수행합니다. 어댑터 정의는 팩의 `agents.json`에
있습니다(claude·gemini·codex·grok).

### 5.3 메시지 보내기

```bash
cys send --to worker "상태 보고해줘"    # 대상 PTY stdin에 직접 주입 (타이핑만)
cys send-key --to worker Return        # 전송 확정 (send 후 필수)
cys send --queued --to worker "..."    # followup 큐: 대상이 조용해지면 자동 배달(Return 불필요)
```

- 기본 send = **steer**(즉시 주입 — 실행 중 조향). `--queued` = **followup**(대상이 3초
  이상 조용할 때 한 틱에 한 건씩 배달).
- **타이핑 가드**: 사람이 방금 타이핑 중인 pane에는 기계 주입이 거부됩니다(기본 3초).

### 5.4 관제·이벤트

```bash
cys status --json                     # 전 노드 1콜 스냅샷 (폴링 대체)
cys fleet                             # 모든 부서×노드의 현재 업무
cys events --reconnect                # 이벤트 푸시 구독 (seq 이어받기)
cys read-screen --surface surface:3   # 화면 읽기 (vt100 정확) — 보조 수단
cys watch --surface surface:3 --until "DONE"   # scrollback이 regex에 맞을 때까지 대기
```

`read-screen --since N`은 단조 라인 커서로 델타만 읽습니다.

### 5.5 자기보고 (권장 규약)

에이전트는 화면 파싱 대신 스스로 신고합니다:

```bash
cys set-status --state working --context 57 --task "리팩터링 중"
```

컨텍스트%가 임계(기본 60%)에 닿으면 데몬이 `context.threshold` 이벤트로 통보합니다.

**각성 ACK(부트 v2)** — 부트가 노드에 지침을 주입한 직후 데몬은 그 좌석에 **논스**(1회용
난수)를 arm 하고, 노드가 그 논스를 되돌려주면 "지침을 실제로 읽었다"로 판정합니다. Claude
좌석은 훅(UserPromptSubmit)이 자동으로 제출하므로 사람이 칠 일이 없고, agy·codex 리뷰어는
`REVIEWER_DIRECTIVE.md` §1-1 절차대로 직접 제출합니다. arm 이전에 도착한 제출은 **무시**됩니다
(사전 ACK 봉인). ACK 가 없으면 부트는 실패가 아니라 `completed_degraded{ack_pending}` 로 닫히고
**리뷰 게이트만** 막힙니다(`orchestra check`·`review-prompt`·`round-init` 의 exit 12 — §12.3).
⚠ 신고용 플래그 `cys set-status --ack <논스>` 는 **이 문서 작성 시점(0.14.29) CLI 에 아직
없습니다** — 착지 전까지 화면 출력(에코) 경로만 유효합니다. 되돌리는 손잡이는
`CYS_BOOT_GATES=0` 입니다.

### 5.6 컨텍스트 사이클·복구

```bash
cys cycle-agent --role worker          # 저장 지시 → 파일 게이트 → clear → 지침 재주입 → 재개
cys node-recover --role worker         # 죽은 에이전트를 같은 surface에서 재기동+재주입
cys restore [--include-master]         # 토폴로지 스냅샷의 죽은 역할 일괄 복원
cys reinject --role worker [--check]   # 디렉티브 재주입 (--check: 드리프트 감지 후 필요 시에만)
```

에이전트 사망은 즉시 감지되어 `agent.exited/recovered` 이벤트가 흐르고, 옵션으로 자동
재기동(`CYS_AGENT_AUTORESTART=1`, 3회 상한·인증 오류 시 차단)이 가능합니다.

### 5.7 역할별 TODO 경로

```bash
cys todo-path        # 이 surface 역할 전용 TODO 파일 경로를 결정론으로 산출(없으면 생성)
```

---

## 6. 승인 Feed와 승인 서명

### 6.1 Feed — 승인 요청함

```bash
cys feed push --wait --title "git push 승인" --body "..."   # 결정까지 블록. exit 0=allow, 2=deny, 3=timeout
cys feed list --status pending
cys feed reply <request_id> allow                            # CLI로 응답 (UI Allow/Deny 버튼과 동일)
```

- 에이전트 훅 연동 예: PreToolUse 훅에서 `cys feed push --wait ...`를 호출하고 exit code로
  결정을 반영.
- pending이 오래 방치되면 `feed.item.aging` 이벤트로 재알림됩니다(기본 300초).
- **자동 응답은 없습니다**(HITL). 요청한 노드가 스스로 승인하는 것도 데몬이 거부합니다.
- UI: 승인 요청이 오면 배지·토스트·OS 알림이 뜨고, 30초 내 해소되지 않은(=사람 개입이
  필요한) 건만 Feed 탭으로 화면이 전환됩니다.

### 6.2 승인 서명 — 반복 위험 명령의 사전 허가

```bash
cys approval sign   # (master 전용) 위험 명령 prefix를 HMAC 서명 — 이후 guard 훅 자동 통과
cys approval check  # 서명 유효성 확인
```

---

## 7. 자원 거버넌스

에이전트가 남긴 고아 서버로 시스템이 마비되는 것을 막는 1급 기능입니다.

```bash
cys run -- python -m http.server            # 새 프로세스 그룹+원장 등록. 종료 시 그룹째 강제 정리
cys ps                                      # 프로세스 원장
cys kill <pid>                              # 원장 등록 프로세스(그룹) 종료
cys add-health-rule relogin "Not logged in" # 출력 라인 헬스룰 추가 → health.alert
cys health-rules
```

- **watchdog**(5초 주기): load 폭주·프로세스 수·중복 명령·idle(기본 300초 무출력)·에이전트
  사망·좀비를 감시해 이벤트를 발행합니다. 중복 프로세스 자동 kill은 opt-in
  (`CYS_AUTOKILL_DUP=1`, 최고(最古) 프로세스 보존).
- **중복 서버 판정(2026-08-01 개정)**: "이름이 같은 것"이 아니라 "같은 것을 두 번 점유한 것"만
  경보합니다. 노드 CLI(claude·codex·agy)·pane 셸·`cys` 자신은 **노드당 1개가 정상**이므로
  계수에서 빠지고, 나머지는 두 갈래로 봅니다 —
  ① `scope=surface`: **한 pane 안**에 같은 명령이 `CYS_DUP_THRESHOLD`(기본 3)개 이상
     (예: 워커가 `bun server.ts`를 쌓은 경우). 자동 kill 대상은 이쪽뿐입니다.
  ② `scope=endpoint`: **같은 포트·유닉스 소켓**을 `CYS_DUP_ENDPOINT_THRESHOLD`(기본 2)개 이상이
     점유(45초 이상 산 프로세스만 · `--port`/`--listen`/`--addr`/`--bind`/`--socket` 표기 인식).
     이름·소유 노드가 달라도 충돌이므로 경보하되, **자동 kill은 하지 않습니다**(정리는 `cys kill`).
  경보 쿨다운은 그룹당 60초입니다. 노드를 몇 개로 늘려도 정상 편성만으로는 경보가 늘지 않습니다.
- 기본 헬스룰: 로그인 풀림·401·token expired·rate limit (30초 디바운스).
- 헬스룰에 조치를 묶을 수 있습니다(opt-in): `--action pause-queue` — queued 배달만 일시정지.
- **자기증폭 차단(2026-08-01)**: 헬스룰은 화면 텍스트를 매칭하므로, 경보 자체가 화면에 다시
  찍히면 그 글이 또 경보가 되는 되먹임이 생깁니다(`cys events` 구독 pane·`cys status` 출력·
  노드가 경보를 논의한 문장). 두 겹으로 끊습니다 —
  ① **발신 봉인**: 경보에 실리는 문장은 매칭 부분이 `‹health-rule›`로 가려진 형태뿐이며,
     **한 줄에 트리거가 여럿이면 전부** 가립니다 — 그 문장은 어떤 헬스룰에도 다시 매칭되지
     않습니다(룰 이름은 경보의 `rule` 필드에 따로 실립니다).
  ② **수신 격리**: "경보를 논하는 문장"은 매칭에서 제외합니다 — 경보 기계장치 이름
     (`health.alert`·`rule=`·룰 이름 등)이 들어간 줄, 트리거를 따옴표로 인용한 줄,
     한글 산문 서술(매칭 구간 밖 한글 8자 이상 · `CYS_HEALTH_NARRATION_CJK_MIN`로 조정,
     `0`이면 비활성).
  진짜 고장 줄(`Error: not logged in`·`401 Unauthorized`·`HTTP 429 …`)은 그대로 경보가 뜹니다.

  **정직한 교환 고지(트레이드오프)** — 이 차단에는 대가가 있습니다.
  - 발신 봉인은 **진단 정보를 지웁니다**. 경보·`cys status`·HUD에 남는 문장은 `‹health-rule›`로
    가려진 형태라, "무엇이 걸렸는지"(어떤 URL·어떤 토큰·어떤 응답 문구)는 **읽을 수 없습니다**.
    남는 것은 어떤 룰이 어느 pane에서 걸렸는지(`rule`·`surface_id`)까지입니다. 원문 확인은
    해당 pane 화면을 직접 보셔야 합니다(`cys read-screen`).
  - 수신 격리는 **진짜 고장을 삼킬 수 있습니다**. 한국어로 실패를 알리는 도구·복구 스크립트
    출력이 "산문 서술"로 분류될 수 있습니다. 그래서 **억제는 경보(발신)까지만** 적용하고,
    안전 인터록(`recent_health` 기록 → 401 무한 재기동 차단)에는 **그대로 남깁니다**.
    억제된 항목은 `discourse` 표시가 붙습니다. 억제가 몇 번 일어났는지는 사유별로 셉니다.
    · 예외: 경보 기계장치 이름이 든 줄(우리 경보의 반사)은 인터록에도 남기지 않습니다.
  - `{"error":"rate limit"}` 같은 **구조화 출력(JSON·logfmt 값 자리)** 은 인용으로 보지 않고
    정상 경보를 냅니다(진짜 고장 은폐 차단).

### kill-switch

```bash
cys pause        # 큐 배달·스케줄 발화 동결 (직접 send는 통과 — '신경 차단')
cys resume
cys gate-check   # exit 0=running, 4=paused (자율주행이 매 action 전 확인)
cys queue list / clear   # 미배달 큐 검사·철회
cys queue deliver <surface> [--id <entry>] [--allow-reorder]
                 # ★사람 운영자 전용 단건 강제 배달 (exit 7=게이트 거부) —
                 # LLM 에이전트의 자동 강제배달 금지(안전 게이트는 강제여도 전부 유지)
cys reap-surface <surface>   # 죽은(exited) 좌석 수동 회수 — master/cso 전용·7조건 게이트
```

pause 상태는 재부팅에도 유지됩니다.

---

## 8. 스케줄러

```bash
cys schedule add --id wake --in 20m --text "[wakeup] 다음 액션 착수" --to master   # 원샷(발화 후 자동 삭제)
cys schedule list / remove <id> / run <id>
```

- 반복 잡은 팩의 `schedule.json`으로 정의됩니다(30초 tick·missed-fire 처리) — 기본으로
  진행 보고·비용 다이제스트·채널 헬스 잡이 들어 있습니다.
- `--fresh --agent claude`: 매 발화마다 새 surface를 띄워 과업을 주입(권한·컨텍스트 상속
  차단), `--close-after`로 TTL 정리.

---

## 9. Control Center

앱의 전용 풀 패널. 데몬이 단일 RPC로 관제 데이터를 제공하고(외부 대시보드 무의존), 영속
분석은 데몬 내장 SQLite에 쌓입니다. **로컬 우선 — 데이터가 머신 밖으로 나가지 않습니다.**

| 탭 | 내용 |
|---|---|
| **Live** | **계정 Rate Limit**(전 조직 병합 — 계정별 5h/7d 게이지·리셋·플랜·소진 예측·🔒라벨 가림)·노드 플릿·가동시간·오늘 토큰/비용/모델믹스·하드웨어(CPU 코어별·GPU·NPU·MEM 2초 실시간)·경보 스트립 |
| **비용·효율** | 기간별 총비용·캐시 절감·재사용율·토큰 4분해·모델별 비용(단가 미상 표시)·조직단위 비용 |
| **스킬·에이전트** | 툴/스킬/위임 호출 집계·실패율(exit≠0)·반복 실패 |
| **세션** | 세션 타임라인·활동 리본·전사 발췌 드릴다운·⭐즐겨찾기·🔒PII 가림 토글 |
| **추세·주간** | 주간 WoW% 델타·일별 오버레이·효율 리더·스킬 자산(신규/휴면) |
| **학습** | 자기개선(RSI) 라운드 타임라인·자산 성장(기억·스킬·directives)·채택/롤백(eval 델타)·발견 누적 |
| **스킬 보드** | 큐레이션 스킬 버튼 = 일회용 워커 실행(§10) — 최근 실행 카드·검색·★즐겨찾기(우클릭)·호출수 뱃지 |
| **작업** | 모든 부서×노드의 현재 업무(관측 전용)·자기보고/파생 신뢰 배지·컨텍스트 바 |
| **승인 Feed** | 승인 요청 목록·Allow/Deny |

**계정 Rate Limit**: 관측은 계정(oauth accountUuid) 단위로 귀속됩니다 — 같은 계정을 여러
프로필 디렉터리가 공유해도 하나의 행으로 병합(최신 관측 승자). 관측이 없는 계정은 0%가
아니라 "관측 없음"으로 정직하게 표시하고, 5h 창은 최근 60분 추세로 "이 속도면 HH:MM 소진"
예측을 붙입니다(표본 부족·리셋이 먼저면 표시하지 않음). 미래 LLM(grok·GLM 등)은
`~/.cys/accounts.json`에 `{provider, label, adapter:"cmd", cmd:"..."}`로 선언하면 합류합니다.

- PII 가림: `CYS_CONTROL_REDACT=1` — 세션 식별자를 가리고 집계는 보존.
- 경보(토큰/비용 임계·이상감지·반복실패)는 Live 스트립 + 헤더 배지로 표시되고, 임계값은
  팩의 `alerts-config.json`에서 조정합니다(핫로드).

---

## 10. 스킬 보드와 스킬 라이브러리

- **스킬 보드**(Control Center 탭): 큐레이션된 스킬을 버튼 클릭으로 실행 — 일회용 워커가
  기동되어 산출물을 만들고, HITL 입력 모달(카탈로그 `fields` 선언 시 다중 필드)과 산출물
  회수 패널이 붙습니다. 노출 목록은 팩의 `board-catalog.json` 큐레이션이 전부입니다(민감
  스킬은 미등재=차단). 실행마다 **최근 실행 카드**가 생애주기(진행중→완료/실패)와 산출물
  링크를 추적하고, 실행 전 자원 게이트(hard=차단·soft=경고)가 선행합니다. 카탈로그 확장은
  `python3 <pack>/bin/javis_board_catalog.py propose`가 후보를 **제안만** 하며 등재는 사람이
  승인합니다.
- **CLI**:

```bash
cys skill list / show <name> / run <name> / new   # 경험을 스킬로 영속·재사용
```

팩에는 스킬 102종이 실려 있고, 스킬 보안은 정적 스캐너(`javis_skillscan.py`)와 벤더링
해시 매니페스트로 게이트됩니다.

---

## 11. 업데이트

업데이트는 **두 채널**이고, 상단바 `Update` 배지 하나로 수렴합니다. 시작 시 + 6시간마다
조용히 확인합니다.

| 배지 | 뜻 | 동작 |
|---|---|---|
| `!` | 앱(바이너리) 업데이트 | 클릭 → 라이브 세션 있으면 확인 → 다운로드·설치 → 재시작 → 팩 반영+노드 자동 복귀 |
| `↻` | 팩만 변경 (무중단) | 클릭 → 서명 검증 → 원자 반영 → 라이브 노드 재주입. **재시작 0, 세션·데몬 생존** |
| `0` | 최신 | — (확인 실패 시엔 마지막 검증 상태를 보존) |

- 재설치(rename-swap) 후 "디스크는 새 버전·프로세스는 구 데몬" 스큐가 남으면 상단바에
  **"데몬 vX · 앱 vY — 세션 보존 중"** 배지가 뜹니다. 클릭해 교대하거나, 라이브 세션이
  0이 되면 자동 교대됩니다(무손실).
- 수동 팩 업데이트: `cys pack-update --from <디렉터리>` (pack.tar.gz + pack-manifest.json +
  .minisig). 서명·신선도·replay 검증은 전건 fail-closed입니다.
- **업데이트 전 미리보기**: `cys pack-plan` — 무엇이 갱신/보존/치유/병합대기/정리되는지
  설치 전에 표시합니다(쓰기 0). 팩을 커스터마이즈해 쓰고 있다면 §12.7을 꼭 읽으세요 —
  수정본은 파괴되지 않고 `.new`(신버전 병치)/`.user`(보존본)로 관리되며 `cys pack-merge`로
  병합합니다.
- 진단·수리: `cys doctor [--fix]` — 팩 스큐·stale lock·고아 소켓·훅 등록을 진단하고,
  `--fix`는 사용자 데이터·팩 본체·DB를 건드리지 않는 범위만 수리합니다.
- **완전 초기화(팩토리 리셋)**: 연습으로 쌓인 부서·세션·대화기억·작업기억·훅을 한 번에
  정리하고 "설치 초기 상태"로 되돌리려면 topbar **완전 초기화** 버튼 또는
  `cys factory-reset`(미리보기 `--plan`, 쓰기 0). 즉시 삭제가 아니라
  `~/.local/state/cys-trash/factory-reset-<시각>/`으로 **격리 보관**되어 그 안의
  `manifest.json`(중도 중단 시 `journal.ndjson`)으로 되돌릴 수 있습니다.
  보존되는 것: 라이선스, 직접 만든 오버레이 `~/.cys/local`(지침 append·스킬·훅),
  `~/.cys`에 직접 넣은 파일(인증서 `.env` 등), 그리고 **사용자 프로젝트 폴더 안의
  작업기억 `_round`**(어디에 남는지는 화면이 알려줍니다). 이것들까지 지우려면
  `--purge-license`·`--purge-local`·`--purge-round`를 붙입니다.
  되돌리기: `cys factory-reset --undo <격리폴더>` (격리 폴더의 `REPORT.txt`에 요약이 남습니다). 완료 후 앱을 다시 실행하면 온보딩이
  처음부터 시작됩니다. ⚠ CLI로 실행할 때는 **앱을 먼저 종료**하세요(앱이 살아 있으면
  초기화 도중 데몬을 되살립니다 — CLI가 이를 감지해 거부합니다).
  ★에이전트(마스터·워커)는 이 명령을 실행할 수 없습니다 — 오너 전용입니다(guard.sh R-02b).
  앱까지 지우는 완전 제거는 `docs/GUIDE-clean-reset-KR.md` 참조.

---

## 12. CYSJavis 팩 운용

팩은 터미널을 "멀티에이전트 회사"로 만드는 운영체계입니다. 개념은
[Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md) §2–4 참조.

### 12.1 설치·구성

```bash
cys init-pack                     # ~/.cys/pack 설치 (사용자 수정 파일 보존)
cys init-pack --install-hook --claude-settings ~/.claude/settings.json   # (선택) Claude Code 훅 강화 주입
```

구성: 절대지침 6(`directives/` — master·worker·CSO·reviewer·RSI 학습·CEO 템플릿) ·
결정론 도구 56(`bin/`) · 훅 18(`hooks/`) · 스킬 102(`skills/`) · 스키마 3(`schemas/`) ·
설정 6(acl·agents·board-catalog·alerts-config·schedule·trusted-keys) · **비어 있는 골격**
(soul.md·memory/ — 사용자가 채움).

### 12.2 역할 선언 부트스트랩

프로젝트 루트에 `CLAUDE.md.template`를 복사해 두면, 에이전트에게 "너는 마스터다/워커다"
라고 선언하는 것만으로 부트스트랩됩니다: 해당 디렉티브+soul.md 각성 → `cys claim-role` →
(마스터면) 결정론 프리플라이트:

```bash
python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_preflight.py" --fix
```

존재·매핑·훅 등록 검증은 **이 스크립트의 출력만이 사실**입니다(자연어 재추론 금지).

**부트 v2 의 수명 계약** — 선언 1건은 **인텐트 1건**으로 원자 등록되고, 인텐트 1건은 러너
1개가 집행하며, 그 끝은 **terminal 정확히 1개**입니다(`completed` / `completed_degraded` /
`declined` / `session_error` / `aborted` / `crashed` / `superseded` / `expired` /
`attempts_exhausted` / `skipped_inflight` / `state_unreadable` — 11종. 전문은
`MASTER_DIRECTIVE.md` §0-A). 같은 좌석에 이미 진행 중인 인텐트가 있으면 새 선언은 **기록되되
`superseded` 로 닫힙니다**(0건도 2건 실행도 아님). 실패했을 때 사용자가 볼 수 있는 것은 두
가지입니다 — 레인별 `boot-last.json`(경로는 `javis_bootstrap.py lane-path boot_last` 가
산출)과, 디스크 쓰기가 실패해도 남는 **stdout 미러 1줄**(`{"channel":"boot-last-mirror", …}`).

### 12.3 위임 루프 (orchestra)

```bash
P=${CYS_PACK_DIR:-$HOME/.cys/pack}/bin
python3 $P/javis_orchestra.py check           # 필수 노드 생존 결정론 확인
                                              #   exit 0=READY · 1=미달 · 2=측정불가
                                              #   12=ack_pending(노드는 다 떴으나 리뷰어
                                              #      각성 ACK 미확인 → 리뷰 게이트만 차단)
python3 $P/javis_orchestra.py task-prompt --task "<T>" --scope "<범위>" --success "<기준>"   # 위임 티켓 생성
python3 $P/javis_orchestra.py gate-status --task "<T>"   # 게이트 수렴 판정 (CONVERGED=다음 단계)
python3 $P/javis_orchestra.py next-action                # 다음 액션 큐 결정론 추출
```

### 12.4 보조 결정론 도구

```bash
python3 $P/javis_route.py --request "<요청>"          # fast/deliberate/slow 3단 사고 라우팅
python3 $P/javis_report.py                            # 진행% 결정론 산출 (todo 체크박스 산술)
python3 $P/javis_memory.py add --type <t> --name <slug> --desc "..." --body "..."   # 장기기억 증류(색인 원자 동기)
python3 $P/javis_task.py checkout <id> --owner <역할>  # 원자적 태스크 체크아웃 (충돌=exit 9)
python3 $P/javis_resource_gate.py check               # 착수 전 자원 게이트 (0 allow / 1 soft / 2 hard)
python3 $P/javis_event.py emit <type> ...             # 닫힌 enum 이벤트 방출 (미지 타입 거부)
python3 $P/javis_wakeup.py enqueue --to <역할> --task <key> --reason "..."   # 코얼레싱 웨이크업 큐
```

### 12.5 RSI 학습·페르소나

```bash
cys learn                 # RSI 학습 루프 — 제안 생성·라운드 상태 (Control Center 학습 탭과 연동)
cys persona list-params   # 노드 페르소나·운영 노브 (안전핵은 잠김)
cys persona show / set / reset
```

### 12.6 pro 채널 (선택)

```bash
cys license install / status    # 서명 라이선스 설치·진단 (검증 전용)
cys pack-repair-channel         # 채널 상태 진단·복구
cys pack-downgrade-to-free      # pro → free 강등의 유일한 경로 (명시적)
```

### 12.7 커스터마이징 — 업데이트와 공존하는 방법

팩·앱을 자기에게 맞게 고쳐 쓰는 것은 지원되는 사용 방식입니다. 다만 **어디를 고치느냐**에
따라 업데이트와의 관계가 다릅니다. 원칙은 하나 — *출하 파일을 직접 고치지 말고, 사용자
전용 계층에 두면 업데이트가 절대 건드리지 않습니다.*

> **생존 보증**: 여러분이 새로 만든 파일(자작 스킬·도구 등 출하물이 아닌 모든 파일)은
> 업데이트·자가치유·정리(prune)·재설치 어디에서도 **절대 삭제·변경되지 않습니다**
> (회귀 테스트로 상시 보증). 사라질 수 있는 것은 "출하 파일을 직접 고친 수정"뿐이며,
> 그마저 파괴되지 않고 `<파일>.user` 로 보존됩니다. 등급 확인: `cys pack-ownership <경로>`.

**사용자 전용 오버레이 `~/.cys/local/`** (업데이트·치유·정리가 존재 자체를 모르는 영역):

| 위치 | 효과 |
|---|---|
| `local/directives/<ROLE>_DIRECTIVE.local.md` | 역할 지침 **뒤에 자동 append** (예: `WORKER_DIRECTIVE.local.md`에 "보고는 존댓말로") |
| `local/skills/<이름>/SKILL.md` | 동명 팩 스킬을 **shadowing**(내 버전이 이김) · 자작 스킬 추가 |
| `local/hooks/<이벤트>.d/*.sh` | 팩 훅 **뒤에 후행 실행** (관측 전용 — 에이전트 차단 불가) |
| `local/notes/` | 자유 메모 영역 (`USER-NOTES.md` 등 — 관례상 여기에) |

단, 오버레이는 안전핵(정지 경계·복원 프로토콜·중단 스위치·운영 헌장)을 뒤집을 수 없습니다 —
해당 키워드 줄은 주입에서 자동 제외되고, 안전핵 재선언이 항상 마지막에 붙습니다. 로컬 스킬은
승격 시 정적 스캔 **경고**(차단 아님)를 출력합니다 — 사용자 책임 영역입니다.

**출하 파일을 이미 직접 고쳤다면** — 업데이트가 파괴하지 않습니다:

- **user-owned 파일**(디렉티브·soul.md·CLAUDE.md·schedule.json): 수정본은 **절대 덮지 않고**,
  vendor 신버전이 나오면 `<파일>.new`로 옆에 병치됩니다(병합 대기).
- **system 파일**(bin·hooks·skills 등): 무결성을 위해 vendor 본으로 치유되지만, 덮기 **전에**
  내 수정본을 `<파일>.user`로 보존합니다(파괴 0).

```bash
cys pack-plan                 # 업데이트 전 드라이런 — 갱신/보존/치유/병합대기/정리 + 오버레이 드리프트 표시
cys pack-merge                # 병합 대기 목록
cys pack-merge --file <경로> --take-new    # vendor 신버전 채택
cys pack-merge --file <경로> --keep-mine   # 내 수정 유지 (이번 신버전 소화)
cys pack-merge --file <경로>               # diff3 3-way 자동 병합 (조상=.pristine)
cys pack-merge --file <경로> --ai          # AI 3-way 병합 — 내 수정 "의도"를 신버전 위에 재적용
cys pack-merge --file skills/<이름>/SKILL.md --to-local   # 스킬 수정본을 오버레이로 승격(권장)
cys pack-merge --file <경로> --propose     # 내 수정을 개선 제안 patch 로 생성(환류 — 자동 전송 없음)
cys pack-rollback                          # 직전 설치 보존본과 다른 파일 목록(복원 후보)
cys pack-rollback --file <경로>            # 직전 설치 보존본에서 그 파일만 복원(원커맨드 되돌리기)
cys pack-ownership <경로>                  # 이 파일을 고치면 어떻게 되는지(소유권 등급) 조회
cys doctor --custom-report                 # 내 커스터마이즈 실태 리포트(로컬 파일 — 자발 제출용)
```

디렉티브·soul.md 같은 **헌법 파일**의 병합은 `--yes` 여도 반드시 대화형 확인을 거치며,
병합 결과에서 안전핵 조항이 사라지면 적용이 자동 거부됩니다(결정론 검증).

자작 스킬은 `cys skill new <이름>` 이 기본으로 `~/.cys/local/skills/` (업데이트 불가침)에
생성합니다. 헌법 병합·치유 원장이 오래 방치되면 부트 점검(C68)이 기한 초과를 경고하고
검토를 촉구합니다(기한: `CYS_MERGE_PENDING_MAX_DAYS`, 기본 14일).

앱 번들(.app/설치 폴더) 내부 수정은 지원하지 않습니다 — 업데이트가 번들을 통째로 교체하며
코드사이닝이 깨집니다. 위 오버레이 채널을 사용하세요. 테마·키바인딩·스케줄·페르소나 노브는
각각 전용 채널(§4 테마 버튼·§12.5 persona·§8 schedule)이 이미 업데이트와 무관하게 보존됩니다.

---

## 13. 채널 브리지 (Slack·Discord)

함대의 승인 요청·보고를 외부 메신저로 내보내고, 허가된 발신자의 원격 승인을 받을 수
있습니다.

```bash
cys channel --json <액션>   # start·stop·register·inbound·outbound·receipt·ack·
                            # allow·allow-remote-approve·revoke·lockdown·unlock·status
```

신뢰 방향은 보수적입니다: 발신자 allowlist(`allow`) · 원격 승인은 별도 허가
(`allow-remote-approve`) · 즉시 잠금(`lockdown`) · 발신 내용의 모양 기반 redact(토큰·홈
경로 차단) · 중복/루프 억제 내장.

---

## 14. 기록·증거 (recall / attest)

```bash
cys recall "<검색어>"      # 모든 에이전트 터미널 활동의 영속 전사 전문검색(FTS)
cys attest pin            # 전사 해시체인을 외부 보관 (평가자 분리)
cys attest verify         # 사후 변조 대조
cys cost-baseline lock / diff   # 비용·효율 baseline 잠금·전후 비교
```

전사 보존 기간은 `CYS_RECALL_RETAIN_DAYS`(기본 30일, 0=무제한)로 제어합니다 — 무한 성장
차단.

---

## 15. CLI 레퍼런스

`cys actions`를 실행하면 기계가 읽는 자기기술 카탈로그가 나옵니다(clap 정의가 단일
진실원). 아래는 사람용 요약입니다.

| 분류 | 명령 | 설명 |
|---|---|---|
| 기본 | `ping` `identify` `actions` `doctor` | 데몬 확인·자기 주소·명령 카탈로그·자기진단(`--fix`) |
| 초기화 | `factory-reset` | 완전 초기화 — 사용 흔적 전량 격리 후 설치 초기 상태로(`--plan` 미리보기·`--verbose` 전량 목록·`--yes` 확인 생략·`--purge-license`·`--purge-local`·`--purge-round`). 되돌리기 `--undo <격리폴더>`. **오너 전용**(에이전트 pane 안 실행 거부) |
| surface | `new-surface` `list` `attach` `read-screen` `resize` `close-surface` `reap-surface` `quiesce` `tombstone` | 세션 생성·목록·미러링·화면 읽기·크기·닫기(자식 트리 전멸)·죽은 좌석 수동 회수(master/cso 전용·exit 7=게이트 거부)·주입 보류·묘비 |
| 통신 | `send` `send-key` `events` `watch` | stdin 주입·키 주입·이벤트 구독·regex 완료 대기 |
| 역할·함대 | `launch-agent` `boot` `claim-role` `surface-role` `status` `fleet` `set-status` `todo-path` | 역할 노드 기동·일괄 부트·역할 등록/조회·관제 보드·자기보고·역할별 TODO 경로 |
| 사이클·복구 | `cycle-agent` `node-recover` `restore` `reinject` `drain` | 컨텍스트 사이클·재기동·조직 복원·지침 재주입·업데이트 전 저장 신호(`drain --verify`=노드별 체크포인트 저장을 nonce 마커로 결정론 검증 후 JSON+exit code) |
| 거버넌스 | `run` `ps` `kill` `add-health-rule` `health-rules` `pause` `resume` `gate-check` `queue` | scoped 실행·원장·강제 종료·헬스룰·kill-switch·큐 관리 |
| 승인 | `feed` `approval` | 승인 요청함(push/list/reply)·HMAC signed-prefix 서명(check/sign) |
| 팩·업데이트 | `init-pack` `pack-update` `pack-manifest` `license` `pack-repair-channel` `pack-downgrade-to-free` `persona` | 팩 설치·무중단 업데이트·매니페스트 방출·pro 라이선스·채널 복구·강등·페르소나 |
| 데몬 | `daemon install/status/uninstall` | 상시 가동 등록·상태·해제 |
| 기록·학습 | `recall` `attest` `learn` `skill` `cost-baseline` | 전사 검색·해시체인 증거·RSI 학습·스킬 라이브러리·비용 baseline |
| 스케줄 | `schedule add/list/remove/run` | 원샷·반복 발화 |
| 채널 | `channel <13 액션>` | Slack·Discord 브리지 |
| 내부 배관 | `usage-register` `usage-report-stdin` `usage-event-stdin` | 훅 전용(직접 쓸 일 없음) |

---

## 16. 환경변수 레퍼런스

### 코어(cysd·cys)가 읽는 변수 — 주요

| 변수 | 기본 | 뜻 |
|---|---|---|
| `CYS_SOCKET` | `~/.local/state/cys/cys.sock` / win `\\.\pipe\cys` | 소켓 경로 |
| `CYS_SHELL` | `$SHELL`→zsh | pane 셸 |
| `CYS_PACK_DIR` | `~/.cys/pack` | 팩 위치 |
| `CYS_LOAD_THRESHOLD` | 코어수×2 | watchdog load 임계 |
| `CYS_PROC_THRESHOLD` / `CYS_DUP_THRESHOLD` | 50 / 3 | 프로세스 수·**pane 내** 동일 명령 중복 임계 |
| `CYS_DUP_ENDPOINT_THRESHOLD` | 2 (0=off) | 동일 포트·소켓 점유 중복 임계 |
| `CYS_AUTOKILL_DUP` | 0 | 중복 프로세스 자동 kill (opt-in · `scope=surface`만) |
| `CYS_IDLE_SECONDS` | 300 | idle 감지 |
| `CYS_TYPING_GUARD_SECS` | 3 (0=off) | 사람 타이핑 보호 |
| `CYS_CONTEXT_THRESHOLD_PCT` | 60 | 컨텍스트 통보 임계 |
| `CYS_MAX_ACTIVE_WORKERS` | 8 | 워커 동시 상한 |
| `CYS_QUEUE_QUIET_SECS` / `CYS_QUEUE_DEPTH_ALERT` | 3 / 5 | followup 배달 조건·큐 깊이 경보 |
| `CYS_QUEUE_MAX_WAIT_SECS` | 0 (=비활성) | 단계형 quiet — 큐 머리가 이 값 이상 대기하면 quiet 임계를 낮춘 '제한 배달(overdue)' 자격. 0=현행 quiet 3s 그대로(활성 권장 120) |
| `CYS_QUEUE_OVERDUE_QUIET_SECS` | 1 | overdue 단계의 quiet 임계 — 1 미만 설정은 1로 승격(0초 강제주입 봉인) |
| `CYS_QUEUE_STARVE_ALERT_SECS` | 0 (=비활성) | 큐 머리 기아 경보 임계 — 이 값 이상 배달이 막혀 있으면 `queue.starved` 발행(쿨다운 5분·depth_high와 별도 축·발행뿐 자동 조치 없음. 활성 권장 600) |
| `CYS_FEED_REMIND_SECS` | 300 (0=off) | 승인 적체 재알림 |
| `CYS_MASTER_DEADMAN_SECS` | 900 | 오케스트레이터 **침묵(idle) 감지** 임계 — 이 시간 이상 출력이 없으면 `master.idle`(생존 확정 + 침묵) 정보 신호. **★v0.14.22 의미 변경: `0`은 이제 '전체 off'가 아니라 '침묵 감지만 off'다.** 사망 판정(`master.deadman` — 좌석 소멸·exited 등 커널 사실 축)은 0에서도 계속 발화한다(fail-closed). v0.14.21 이하에서 오경보를 끄려고 `0`을 설정해 둔 설치는 업그레이드 후 사망 축 경보를 다시 받게 된다 |
| `CYS_ROLE_DEADMAN_CONFIRM_TICKS` | 3 (최소 1) | 역할 데드맨 v2 — 사망 후보(DeadCandidate) 연속 관측 확증 틱 수 |
| `CYS_ROLE_DEADMAN_GRACE_SECS` | 60 | 역할 데드맨 v2 — 부트/승계 직후 무카운트 창(오살 방지) |
| `CYS_ROLE_DEADMAN_DEBOUNCE_SECS` | 300 | 역할 데드맨 v2 — `master.deadman` 디바운스 |
| `CYS_ROLE_DEADMAN_IDLE_DEBOUNCE_SECS` | =DEBOUNCE(300) | `master.idle`(생존+침묵 정보 신호) 전용 디바운스 — 미설정 시 위 값을 따름 |
| `CYS_ROLE_DEADMAN_ROLES` | `master` | 역할 데드맨 감시 role CSV(일반화 opt-in) |
| `CYS_AGENT_AUTORESTART` | 0 | 죽은 에이전트 자동 재기동 (3회 상한) |
| `CYS_RECALL_RETAIN_DAYS` | 30 (0=무제한) | 전사 보존 |
| `CYS_CONTROL_REDACT` | 0 | Control Center 세션 PII 가림 |
| `CYS_TODO_DIRS` | — | todo 감시 추가 루트(콜론 구분) |
| `CYS_NO_AUTOSTART` / `CYS_NO_AUTORESTORE` | — | 자동 기동/자동 복원 끄기 |
| `CYS_APPROVAL_SECRET_B64` | 자동 생성 | 승인 서명 시크릿 오버라이드 |
| `CYS_CHANNEL_RETAIN_DAYS` / `CYS_CHANNEL_OUTBOUND_TIMEOUT_SECS` | 7 / 30 | 채널 보존·발신 타임아웃 |
| `CYS_CLAUDE_CTX_WINDOW` | 200k (`[1m]`=1M) | 컨텍스트 창 크기 힌트 |
| `CYS_ALLOW_APP_MOUSE` | — (`1`=on) | 앱 마우스 킬스위치 — TUI가 마우스를 갖는다 (§4.6b · `~/.cys/allow-app-mouse` 파일과 동등 · 새 pane부터). ⚠ **Windows에서는 켜지 마세요** — ConPTY가 마우스 시퀀스를 깨뜨려 입력창에 `[555;98;34M` 같은 리터럴이 타이핑됩니다. Windows 휠 가드를 끄는 용도로도 쓰면 안 됩니다(그건 아래 전용 스위치) |
| `CYS_WIN_WHEEL_GUARD_OFF` | — (`1`=off로 되돌림) | **Windows 전용** 휠 가드 롤백 — 전체화면 TUI 휠 억제를 끄고 종전(방향키 합성)으로 복귀 (§4.6b). ⚠ **적용 시점이 파일 게이트와 다릅니다**: env는 **GUI 프로세스가 상속한 값만** 읽으므로 `setx` 후 **GUI 재시작**이 필요하고, 동등 수단인 `~/.cys/win-wheel-guard-off` **파일은 새 pane부터 즉시** 반영됩니다(Windows 권장 수단은 파일 — PowerShell `New-Item -ItemType File -Force $HOME\.cys\win-wheel-guard-off`, `touch`는 PowerShell에 없는 명령입니다). ⚠ **결함 복원 스위치** — 켜면 claude fullscreen에서 휠이 다시 방향키로 합성돼 **프롬프트 입력창이 오염될 수 있습니다.** ⚠ 이 용도로 `CYS_ALLOW_APP_MOUSE`를 대신 쓰지 마세요 — 입·출력 양측이 열려 ConPTY 리터럴 타이핑이 되살아납니다 |
| `CYS_WIN_NO_ALT_SCREEN` | — (`1`=on · **Windows 기본 off**) | **Windows 전용 옵트인** — claude를 기동할 때 `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1`을 함께 실어 **전체화면(alt screen) 대신 inline으로 뜨게** 합니다(아래 줄과 짝 · macOS는 이 스위치 없이 늘 주입됩니다). **왜 Windows만 기본 off인가**: 이 env가 Windows의 claude를 깨뜨리면 `cys boot`로 띄운 **모든 pane이 한꺼번에 죽는** 경로라, 실기 Windows에서 그 확인(`cys boot` 4종 노드 정상 기동)을 마치기 전에는 기본값으로 켜지 않습니다. 켜지 않아도 **전체화면 휠 오염은 Windows 휠 가드가 막습니다**(위 §4.6b — 그쪽이 본체 방어이고 이 env는 덧대는 벨트입니다). **켜는 법**: PowerShell `New-Item -ItemType File -Force $HOME\.cys\win-no-alt-screen` 후 **`cys launch-agent`로 새 pane 기동**(되돌리기 `Remove-Item …`). env `CYS_WIN_NO_ALT_SCREEN=1`도 동등하지만 **GUI가 상속한 값만** 읽으므로 `setx` 후 GUI 재시작이 필요합니다(권장 수단은 파일). ⚠ **켜도 새 surface를 만들며 기동한 pane에만** 도달합니다 — 이미 열린 pane의 재기동(node-recover)·GUI에서 연 pane에는 실리지 않습니다. ⚠ `agents.json` env에 `"0"`이 적혀 있으면 켜도 주입하지 않습니다(사용자 값 우선) |
| `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN` | macOS: claude 기동 시 `"1"` 기본 주입 · **Windows: 기본 주입 없음**(위 `CYS_WIN_NO_ALT_SCREEN` 옵트인 시에만) | cys가 읽는 게 아니라 **주입**하는 변수(claude가 소비) — fullscreen(alt screen) 진입 차단. 계정별 되살리기는 팩 `agents.json` env에 `"0"`(키가 있으면 주입하지 않음 · §4.6b). ⚠ Windows는 옵트인해도 **새 surface를 만들며 기동한 pane에만** 주입이 도달합니다(벨트 — 이것만으로 fullscreen을 막았다고 보면 안 됩니다) |
| `CYS_DEPT_FALLBACK` | — (`0`/`off`=끔) | 마스터 선언→부서 자동 생성 폴백 끄기 (§4.4 · 구계약 rc=7 복원) |

(이 밖에 진단·튜닝용 변수 다수 — `CYS_DEBUG`, `CYS_USAGE_POLL_SECS`, `CYS_REAP_EXITED*`,
`CYS_CRASH_WINDOW_SECS`, `CYS_MAX_RESPONSE_BYTES`, `CYS_ABI_VERIFY` 등. 소스 grep
`env_compat`가 전수 목록의 진실원입니다.)

### todo 선언 블록 — "이 파일은 누구의, 언제 것인가" (`CYS_TODO_DIRS`와 짝)

`CYS_TODO_DIRS`로 여러 폴더를 감시하면 **여러 사람·여러 시기의 todo 파일이 한 진행률에 섞입니다.**
예전에 끝난 작업의 파일이 폴더에 남아 있으면, 그 안의 체크 안 된 항목이 오늘의 진행률을 계속
끌어내립니다(파일 이름·수정 날짜만으로는 "지금 살아 있는 작업"인지 기계가 알 수 없기 때문입니다).

그래서 todo 파일은 **자기가 누구 것인지 파일 안에 직접 밝힐 수 있습니다.** 파일 맨 위(첫 체크박스
줄보다 위)에 아래 한 줄을 넣어 두면 됩니다.

```markdown
<!-- javis:todo v1 owner=worker scope=pack lane=my-project status=active since=2026-07-26 -->

# 오늘 할 일
- [ ] 첫 번째 항목
```

| 항목 | 뜻 |
|---|---|
| `owner` | 이 파일의 주인 역할 이름 (`worker`, `master` 등) |
| `scope` | 이 파일이 속한 팩 폴더 이름 (`~/.cys/pack`이면 `pack`) |
| `status` | `active`(진행 중) 또는 `retired`(끝난 작업 — 진행률에서 빠집니다) |
| `lane` | (선택) 사람이 알아보기 쉬운 작업 이름 |
| `since` | (선택) 시작한 날짜 |

- `<!-- ... -->`는 마크다운 주석이라 **문서로 볼 때는 보이지 않습니다.**
- 작업이 끝나면 `status=active`를 `status=retired`로 고치세요. 파일을 지우지 않고도 진행률에서
  빠지므로 기록은 그대로 남습니다.
- 값에는 **영문·숫자와 `. _ : -`만** 쓸 수 있습니다. 따옴표를 붙이거나 값 안에 띄어쓰기를 넣으면
  선언으로 인정되지 않습니다.
- **선언이 없어도 그대로 동작합니다.** 지금까지 쓰던 todo 파일을 고칠 필요는 없습니다. 선언이 없는
  파일은 "주인 미상"으로 따로 묶여 보고될 뿐, 진행률이나 내용이 사라지지 않습니다.
- 위임 티켓을 `javis_orchestra.py task-prompt`로 만들면 그 작업에 맞는 선언 한 줄이 티켓에 함께
  들어옵니다 — 손으로 짓지 말고 그대로 복사해 쓰는 편이 안전합니다.

### 팩 도구가 읽는 변수 (바이너리 아님)

| 변수 | 뜻 |
|---|---|
| `CYS_URL_ALLOW_HOSTS` | 외부 URL 허용 도메인 확장(또는 `~/.cys/url-allow-hosts` 파일) |
| `CYS_WORKER_PROFILE_DIR` | 워커 프로필 경로(또는 `~/.cys/worker-profile-dir` 파일) |

---

## 17. 프로토콜 레퍼런스 (RPC·이벤트)

NDJSON — 한 줄 = JSON 하나. 요청 `{"id","method","params"}` → 응답
`{"id","ok",result|error}`. 서버 push는 `events.stream` 구독.

### RPC 메서드 (v0.14.22 기준 전수)

```
system.ping / identify / claim_role / resolve_role / pause / resume / gate_check / topology
surface.create / list / send_text / send_key / read_text / resize / rename / close /
        attach / set_meta / quiesce / wait_for / reap
tombstone.set   events.stream   reinject.mark   status.set
ledger.register / deregister / list / kill
health.add_rule / list_rules
feed.push / reply / list
queue.list / clear / deliver
recall.search   attest.pin / verify   approval.check / sign
learn.propose / status / history
schedule.status / run_now
usage.register / report / event
org.status
control.dashboard / hw / analytics / cost_baseline / skills / weekly / alerts /
        sessions / session_detail / session_star
editor.action_catalog / action_info
channel.start / stop / register / inbound / outbound / receipt / ack / allow /
        allow-remote-approve / revoke / lockdown / unlock / status
```

### 이벤트 (계열별)

```
surface.created/exited/crashed/closed/reaped/reap_requested/reap_denied/zombie_reaped/
        close_denied/quiescing/input_injected
agent.exited/recovered/restart_blocked/exit_unrecoverable
watchdog.load_high/proc_count_high/duplicate_procs/tick_panic   pane.idle
queue.enqueued/delivered/dropped/depth_high/clear_denied/rehomed/migrated/starved/reordered
ledger.registered/killed
feed.item.created/resolved/aging/timeout   feed.backlog_high
health.alert/action
schedule.fired/missed/error/command_done/tick_panic
autopilot.paused/resumed/master_changed/approval_checked/approval_signed
role.claimed/claim_denied   worker.limit_denied
  └ claim_denied payload: reason·current_holder(보유자 있음) 또는 error_code=claim_caller_unresolved
    |claim_not_owner + reason=identity(발신 pane 미식별·소유 불일치 — 보유자 유무와 무관)
usage.session_registered/updated/register_denied/report_denied/tick_panic
channel.* (bridge.exited·auth.denied·registered·message·outbound.<ch>·lockdown·… 15종)
daemon.started/stopping   acl.denied   context.threshold   status.changed   task.changed
todo.updated   approval.request   approval.stalled   master.deadman   master.idle   osc.notify
```

v0.14.22 가산분(전부 additive — 기존 소비자 무해):
- `queue.rehomed` {role, count, queue_entry_ids, reordered} — WAL 복원 항목의 같은 role 생존
  surface 큐 (enqueued_at, seq) 정렬 병합. `reordered=true`=기존 항목이 뒤로 밀림(무음 재정렬 금지)
- `queue.migrated` {from_surface, to_surface, queue_entry_ids, role} — 좌석 승계 시 큐 이관
- `queue.starved` {surface_ref, role, head_entry_id, waited_secs, depth, blocked_by, hint} —
  큐 머리 기아 경보(`CYS_QUEUE_STARVE_ALERT_SECS`, 기본 0=비활성 · 발행뿐, 자동 조치 없음)
- `queue.reordered` {queue_entry_id, from_index, to_index, cause} — `queue deliver --allow-reorder`
  명시 재정렬
- `master.idle` (category=info) {role, surface_ref, axis:"silence", idle_secs, …} — 생존 확정 +
  침묵의 정보성 신호. 사망(`master.deadman`)과 축 분리 — idle은 alert가 아니다
- `surface.reap_requested` / `surface.reap_denied`(사유 코드) — `surface.reap` 감사 쌍(성패 무관
  요청 1건 + 거부 시 사유). 실행 성공은 기존 `surface.reaped`에 {reason:"manual_reclaim",
  by_surface, by_role} additive
- 기존 이벤트 가산 필드: queue 계열에 `queue_entry_id`/`queue_entry_ids`·`seq`·`enqueued_at`,
  `queue.dropped`(exited 예외 경유 시) `cleared_by`/`via:"exited_reclaim"`,
  `feed.item.resolved`에 `resolver_surface`(해소자 각인 — cycle 영수증 검증이 소비)

---

## 18. 트러블슈팅 · 알려진 한계

**트러블슈팅**

| 증상 | 조치 |
|---|---|
| macOS "손상되었기 때문에 열 수 없습니다" | **원인 두 가지 — ①반쪽 설치(덮어쓰기로 설치) ②quarantine/미공증.** ①이 훨씬 흔하다: 기존 `cys.app`을 **먼저 휴지통으로 옮긴 뒤** DMG에서 새로 드래그(덮어쓰기 금지 — 일부 파일만 막혀 세대 혼합 번들이 남는다). ②는 `xattr -d com.apple.quarantine /Applications/cys.app`. 원인 판별은 `cys doctor`(app-seal 항목)·`codesign --verify --strict /Applications/cys.app`. 전체 절차: `docs/INSTALL.md` → "손상되었기 때문에 열 수 없습니다" 해결 |
| 앱이 *"설치본이 온전하지 않습니다 — 재설치 필요"* 안내를 띄움 | 기동 자기점검이 **빠진 구성요소를 이름으로** 찾아낸 것이다(반쪽 설치). 안내에 적힌 파일을 보고 위와 같은 절차로 **통째 재설치**한다. 번들 안 파일만 채워 넣는 부분 수리는 통하지 않는다(macOS 보호에 막히고, 막혀도 서명 봉인이 깨진 채 남는다). 설정·대화기록(`~/.cys`)은 번들 밖이라 보존된다 |
| `cys ping` 실패 | 앱 실행(데몬 자동 기동) 또는 `cysd` 직접 기동. `cys doctor --fix` |
| 연습 흔적(부서·세션·기억)이 자꾸 살아나 충돌 | topbar **완전 초기화** 버튼 또는 `cys factory-reset` — 사용 흔적 전부를 격리 보관하고 설치 초기 상태로(§11 "완전 초기화" 참조·격리 폴더 manifest.json으로 복구). 미리보기는 `cys factory-reset --plan`(쓰기 0). CLI 실행 전 앱 종료 필요 |
| 데몬이 두 개 뜬 것 같음 | 실제로는 불가(중복 기동 거부). 업데이트 후 스큐 배지가 떠 있으면 클릭해 교대 |
| 팩 업데이트가 거부됨 | 정상일 수 있음 — 서명·신선도·replay 검증은 fail-closed. `cys pack-update --dry-run`·`cys doctor`로 원인 확인 |
| 노드에 메시지가 안 들어감 | 타이핑 가드(사람 입력 직후 3초)·ACL(`acl.denied` 이벤트)·kill-switch(`cys gate-check`) 순서로 확인 |
| Windows SmartScreen 경고 | 현재 Authenticode 미서명 — "추가 정보→실행" |

**알려진 한계** (정직성 규약 — 숨기지 않습니다)

- macOS에서 sysinfo가 cmdline 전체를 못 읽으면 프로세스명으로 중복 그룹핑(과탐 가능).
- `cys run` 중 Ctrl-C로 CLI가 죽으면 그룹 정리가 watchdog 주기(5초)로 넘어감.
- GPU/NPU 실시간은 macOS(Apple Silicon) 전용 — Windows는 CPU/MEM만. NPU는 활용률 공개
  API가 없어 실측 전력(W)으로 표시.
- 단일-UID 신뢰 모델: 승인 서명·자기결재 차단은 같은 계정 내 악성 프로세스에 대한
  암호학적 방어가 아니라 탐지·fail-safe 층입니다.
- 비밀 스캐너는 정적 패턴 매칭 — 난독화·신종 토큰은 못 잡습니다(1차 방어선일 뿐).

취약점 신고는 [SECURITY.md](SECURITY.md)를 따라 주세요.
