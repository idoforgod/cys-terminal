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

### 2.1 macOS (Apple Silicon)

1. `cys_<버전>_aarch64.dmg`를 열고 `cys.app`을 Applications로 드래그.
2. 첫 실행에서 Gatekeeper 경고가 뜨면: 공증된 빌드는 그대로 열리고, 아니면 우클릭 → "열기".
3. 앱이 데몬(cysd)을 자동 기동하고 launchd에 등록합니다(재부팅 후에도 유지).

### 2.2 Windows (x64)

1. `cys_<버전>_x64-setup.exe`(NSIS) 실행 — **자기완결 설치**: 데몬·CLI·런타임(Git Bash·
   Python)이 동봉되어 별도 준비물이 없습니다.
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

- **권장(macOS)**: 앱 Control Center 헤더 → **"셸에 cys 설치"** 1클릭(관리자 승인 1회) —
  `/usr/local/bin/cys`·`cysd` 심볼릭 링크가 생기고, 앱 업데이트에도 자동 추종합니다.
- Windows는 설치기가 PATH를 구성합니다.

### 2.5 설치 확인

```bash
cys ping            # 데몬 응답 확인
cys identify        # 데몬·내 주소 확인
cys status          # 전 노드 관제 보드
cys doctor          # 자기진단 (문제 시 --fix)
```

### 2.6 제거

1. `cys daemon uninstall`
2. 앱 삭제(macOS: Applications에서 제거 + 심링크 제거 / Windows: 제어판 제거)
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
cys send --queued --idempotency-key build-status --to master "..."   # 같은 키는 제자리 교체(최신 승리)
cys send --queued --important --to worker "..."                      # TTL 면제(만기로 사라지지 않음)
```

- 기본 send = **steer**(즉시 주입 — 실행 중 조향). `--queued` = **followup**(대상이 3초
  이상 조용할 때 한 틱에 한 건씩 배달).
- **타이핑 가드**: 사람이 방금 타이핑 중인 pane에는 기계 주입이 거부됩니다(기본 3초).
- `--idempotency-key <키>`(--queued 전용): 대상 큐에 같은 키의 항목이 있으면 **제자리에서
  교체**합니다(줄 뒤로 밀리지 않음 — 자주 갱신하는 주제일수록 늦게 배달되는 역전 방지).
  밀려난 구 텍스트는 dead-letter(`superseded`)에 남습니다. 키가 없으면 어떤 중복 억제도
  하지 않습니다(동일 문자열 재전송은 정당한 패턴).
  교체할 때 **기다린 시간은 원래 것을 그대로 물려받습니다** — 그러지 않으면 자주 갱신하는
  메시지가 만기 시계를 매번 되감아 큐에서 영원히 사라지지 않습니다. 그래서 만기까지 남은
  시간이 5분 미만인 항목에는 교체를 걸지 않고, 원래 항목을 원장으로 보낸 뒤 갱신본을
  **새 메시지로 처리**합니다(혼잡 상한을 다시 통과합니다 — 수명이 다한 주제의 갱신은
  사실상 새 발신이기 때문입니다). 이때 혼잡 상한에 걸리면 평소처럼 거부되며, 원래 항목과
  갱신본 **둘 다** 원장에 남아 있으므로 잃는 내용은 없습니다.
- `--important`(--queued 전용, master·CSO 계열 발신만): **TTL 면제**입니다.
  소프트캡 면제가 아닙니다 — 혼잡 상한은 지휘 메시지에도 적용됩니다(의도된 설계).

**큐 혼잡 상한(소프트캡) — 기본이 `enforce`입니다.** 한 대상 큐에 쌓인 **에이전트 발신**
항목이 소프트캡(기본 25건 · `CYS_QUEUE_AGENT_SOFTCAP`)에 닿으면, 그 뒤 발신은 관찰만 하는 게
아니라 **실제로 거부**됩니다(`queue_softcap_exceeded`로 비0 종료 · `queue.rejected` 이벤트).
거부된 메시지는 사라지지 않습니다 — 데몬이 전문을 `dead-letters.jsonl` 원장에 적은 뒤에 거부하며,
그래서 **거부는 실패가 아니라 종결입니다**(같은 메시지를 다시 보내면 혼잡만 커지고 원장이
오염됩니다). 적재만 허용하고 이벤트만 남기는 관찰 모드는 `CYS_QUEUE_POLICY_MODE=log`로
**명시해야** 켜지는 롤백 스위치입니다. 사람(GUI)·시스템 발신은 이 상한에서 면제됩니다.

**큐 항목 만기(TTL) — 발신 등급마다 다릅니다.** 아무도 받아가지 않는 메시지가 큐에 영원히
남지 않도록, 오래된 항목은 큐에서 빠져 원장으로 **이관**됩니다(폐기가 아닙니다).

| 발신 등급 | 기본 만기 | 이유 |
|---|---|---|
| 에이전트(노드 pane에서 보낸 것) | 1시간 (`CYS_QUEUE_TTL_SECS`) | 가장 흔한 폭주 주체입니다. |
| 시스템(스케줄러·거버넌스 등 pane 밖 발신) | **4시간** (`CYS_QUEUE_SYSTEM_TTL_SECS`) | 제어 메시지라 성급히 치우면 더 해롭습니다. 그래도 상한을 둡니다 — 종전에는 시스템 발신이 만기 자체가 없어, 배달이 막힌 pane 앞에서 **한없이 쌓였습니다**. |
| 사람이 GUI에서 큐잉한 것 | **24시간** (`CYS_QUEUE_GUI_TTL_SECS` · 0=면제) | 사실상 무기한에 준하는 장주기입니다 — 사람이 하루 안에 돌아오는 창을 덮고, 만기돼도 폐기가 아니라 **원장 보존 + 통지**입니다. 종전에는 완전 면제였는데, 이 등급을 가르는 `gui` 표시는 보내는 쪽이 스스로 붙이는 값이라 사람이 아닌 프로그램도 한 줄로 **무기한을 받아낼 수 있었습니다**. 상한을 두면 그렇게 얻을 게 24시간뿐이라 흉내 낼 이유가 사라집니다. |
| `--important` 선언분 | 면제 | 등급과 무관한 메시지 단위 의도 선언입니다. **진짜 무기한이 필요할 때 쓰는 명시 경로**입니다. |

만기가 일어나면 `queue.expired` 이벤트가 흐르고, 전문은 원장에 남습니다. **원장을 확인해
지금도 유효한 건만 다시 처리하면 됩니다 — 발신자에게 재전송을 요구할 필요가 없습니다.**
`CYS_QUEUE_TTL_SECS=0`은 등급과 무관하게 만기 기능을 통째로 끄는 전면 롤백 스위치입니다.
- 여러 대상(`--to 'reviewer-*'`)에 보낼 때 일부가 실패해도 나머지는 계속 시도하고,
  마지막에 대상별 실패를 요약한 뒤 **비0으로 종료**합니다.

### 5.4 관제·이벤트

```bash
cys status --json                     # 전 노드 1콜 스냅샷 (폴링 대체)
cys fleet                             # 모든 부서×노드의 현재 업무
cys events --reconnect                # 이벤트 푸시 구독 (seq 이어받기)
cys read-screen --surface surface:3   # 화면 읽기 (vt100 정확) — 보조 수단
cys watch --surface surface:3 --until "DONE"   # scrollback이 regex에 맞을 때까지 대기
```

`read-screen --since N`은 단조 라인 커서로 델타만 읽습니다.

#### 관측 명령과 autostart

`cys`는 데몬에 연결하지 못하면 **형제 `cysd`를 자동 기동**한 뒤 다시 시도합니다(신규 머신 zero-setup).
편리하지만 관측 명령에서는 부작용이 됩니다 — "데몬이 떠 있는가"를 물었는데 그 질문 자체가 데몬을
띄워버리면 답이 바뀝니다. 그래서 명령마다 성질이 다릅니다.

| 명령 | 자동 기동 | 이유 |
|---|---|---|
| `cys ping`, `cys daemon status` | **안 함** | 순수 생존 프로브 — 관측이 대상을 바꾸면 안 됩니다. |
| `cys list` | 함 (기본) | 복원 절차가 이 자동 기동에 의존합니다. 끄려면 `--no-autostart`. |
| `cys status`, `cys doctor` | 함 / 접촉 없음 | `status`는 관제 보드라 데몬이 필요하고, `doctor`는 애초에 자동 기동 경로를 타지 않습니다. |

```bash
cys list --no-autostart      # 죽은 데몬을 깨우지 않고 현재 상태만 본다
CYS_NO_AUTOSTART=1 cys list  # 동일 (env 옵트아웃 — 세션 전체에 적용)
```

데몬이 안 떠 있으면 `--no-autostart`는 연결 실패로 끝납니다. 이것이 정상이며, "죽어 있음"이 곧
답입니다. 장애를 조사할 때(특히 crashloop·락 경합을 볼 때)는 조사 행위가 증거를 바꾸지 않도록
이 플래그를 쓰세요.

### 5.5 자기보고 (권장 규약)

에이전트는 화면 파싱 대신 스스로 신고합니다:

```bash
cys set-status --state working --context 57 --task "리팩터링 중"
```

컨텍스트%가 임계(기본 60%)에 닿으면 데몬이 `context.threshold` 이벤트로 통보합니다.

**`source`가 `observed-uncertain`이면 그 통보는 "확실하지 않다"는 뜻입니다.** 에이전트가
스스로 신고하지 않을 때 데몬은 세션 기록을 읽어 소비량을 관측하는데, 한 폴더에서 여러 pane이
돌면 그 값이 **어느 pane 것인지 특정되지 않을 때**가 있습니다. 원칙적으로 그런 관측치는
임계 발화에 쓰지 않지만(엉뚱한 pane을 `/clear` 시키면 그 pane 작업이 날아갑니다), 85% 이상은
예외로 발화합니다 — 방치해서 100%에 닿아 전부 잃는 쪽이 더 나쁘기 때문입니다. 이 폴백으로
나온 통보는 `source: "observed-uncertain"`과 `attribution` 값을 함께 싣고, `action` 문구도
"바로 집행"이 아니라 **`cys read-screen`으로 대상 pane을 직접 확인한 뒤 집행**하라고
바뀝니다. 반대로 소유가 **다른 pane 것으로 판명된**(`evicted`) 관측치는 몇 %든 발화하지
않습니다.

### 5.6 컨텍스트 사이클·복구

```bash
cys cycle-agent --role worker          # 저장 지시 → 파일 게이트 → clear → 지침 재주입 → 재개
cys node-recover --role worker         # 죽은 에이전트를 같은 surface에서 재기동+재주입
cys restore [--include-master]         # 토폴로지 스냅샷의 죽은 역할 일괄 복원
cys reinject --role worker [--check]   # 디렉티브 재주입 (--check: 드리프트 감지 후 필요 시에만)
```

에이전트 사망은 즉시 감지되어 `agent.exited/recovered` 이벤트가 흐르고, 옵션으로 자동
재기동(`CYS_AGENT_AUTORESTART=1`, 3회 상한·인증 오류 시 차단)이 가능합니다.

**`cycle-agent --force-no-verify`의 의미가 바뀌었습니다.** 예전에는 "검증할 파일 목록이
비었을 때 그래도 진행"이라는 뜻이었는데, 목록이 비는 경우가 없어지면서 아무 일도 하지 않는
죽은 플래그가 돼 있었습니다. 지금은 **저장 검증 대기 자체를 건너뜁니다** — 저장 지시는
그대로 주입하되(지시조차 안 하면 저장할 기회가 사라집니다) 파일이 갱신되기를 기다리지 않고
다음 단계로 넘어갑니다. 대상이 멈춰(hang) 저장을 못 하는 상황에서 30분 타임아웃을 기다리지
않고 빠져나오는 **비상 탈출구**이며, 대신 **저장되지 않은 채로 clear될 수 있습니다**.
평시 사용은 금지이고, 검증자 handshake 단계는 이 플래그와 무관하게 그대로 적용됩니다.

### 5.7 역할별 TODO 경로 · 팩 경로 판정

```bash
cys todo-path                          # 이 surface 역할 전용 TODO 파일 경로를 결정론으로 산출(없으면 생성)
cys todo-path --role worker-2          # 다른 역할의 경로만 산출 (파일 생성·기록 없음)
cys todo-path --kind session-state     # 팩 정본 SESSION_STATE.md 경로만 산출 (생성 없음)
cys todo-path --kind recovery          # 팩 정본 RECOVERY.md 경로만 산출 (생성 없음)
```

팩 경로는 **데몬이 권위**입니다. `todo-path`는 역할을 물을 때 쓰는 같은 `surface.list` 응답의
봉투에서 `pack_dir`을 함께 읽어(추가 왕복 없음), 로컬 `CYS_PACK_DIR`과 다르면 경고 한 줄을 낸 뒤
**데몬 값을 채택**합니다. 부서 pane이 `CYS_PACK_DIR`을 잃으면 로컬 산출은 조용히 본사 팩
(`~/.cys/pack`)으로 폴백하는데, 그러면 부서 노드의 TODO가 본사 팩에 쓰입니다 — 데몬 권위가
이것을 막습니다. 봉투에 키가 없는 구버전 데몬이면 조용히 로컬로 폴백하고, 데몬에 묻지 못하면
그 사실을 stderr로 알립니다.

`--kind`가 산출하는 SESSION_STATE·RECOVERY는 역할과 무관한 팩 단위 파일이라 **역할이 등록되지
않은 surface에서도 동작**합니다(복원 초입에 읽히는 파일이기 때문입니다). 세 경로 모두 같은 팩
권위 산출을 공유합니다.

```bash
cys pack-scope-check --path <경로>     # 이 경로가 '남의 scope 팩'인지 판정 (stdout JSON · exit 0 고정)
```

`pack-guard` 훅이 쓰는 판정 SOT입니다. 출력은 5키
`{verdict, own_scope, target_scope, suggest, authority}`이고, `verdict`는 `ok` 또는 `cross-scope`,
`authority`는 자기 scope의 출처(`daemon`=surface.list 봉투 / `local`=로컬 env 산출)입니다.
**어떤 오류에서도 exit 0**이며 판정 불가는 `verdict: "ok"`로 표현합니다 — 훅 내부 사정이 도구
실행을 막지 않게 하려는 계약입니다. `authority`가 `daemon`이 아니면 소비자는 차단을 기록으로
강등해야 합니다(권위가 불확실할 때는 막지 않습니다).

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
cys run --scoped -- python -m http.server   # 새 프로세스 그룹+원장 등록. 종료 시 그룹째 강제 정리
cys ps                                      # 프로세스 원장
cys kill <pid>                              # 원장 등록 프로세스(그룹) 종료
cys add-health-rule relogin "Not logged in" # 출력 라인 헬스룰 추가 → health.alert
cys health-rules
```

- **watchdog**(5초 주기): load 폭주·프로세스 수·중복 명령·idle(기본 300초 무출력)·에이전트
  사망·좀비를 감시해 이벤트를 발행합니다. 중복 프로세스 자동 kill은 opt-in
  (`CYS_AUTOKILL_DUP=1`, 최고(最古) 프로세스 보존).
- 기본 헬스룰: 로그인 풀림·401·token expired·rate limit (30초 디바운스).
- 헬스룰에 조치를 묶을 수 있습니다(opt-in): `--action pause-queue` — queued 배달만 일시정지.

### kill-switch

```bash
cys pause        # 큐 배달·스케줄 발화 동결 (직접 send는 통과 — '신경 차단')
cys resume
cys gate-check   # exit 0=running, 4=paused (자율주행이 매 action 전 확인)
cys queue list / clear   # 미배달 큐 검사·철회
cys queue clear --surface <ref> --operator-token <토큰>   # 위임형 clear(남의 큐를 대신 비움)
cys queue request-clear --surface <ref>                   # ACL 보존형: 대신 비우지 않고 점검을 '요청'
```

pause 상태는 재부팅에도 유지됩니다.

**큐에서 사라지는 모든 메시지는 원장에 남습니다.** `clear`·surface 종료·TTL 만기·소프트캡
거부·멱등 교체 — 어느 경로든 전문이 state dir 의 `dead-letters.jsonl` 에 기록된 뒤에야
큐에서 제거됩니다. 기록에 실패하면 항목을 **지우지 않고 큐에 남깁니다**(응답의 `retained`
필드 · `health.dead_letter_write_failed` 이벤트). 사람이 읽는 형태로는 6시간 주기 built-in
잡(`deadletter-transcribe-6h`)이 `_round/dead_letters/<날짜>.md` 로 전사합니다.

- `--operator-token`: state dir 의 `operator.token`(0600·기동 시 재발급)과 일치하면 타
  surface 큐도 비울 수 있습니다. 토큰은 "사람 오퍼레이터 세션" 증명이지 새 권한이 아니며,
  이 경로로 제거된 항목은 `cleared_by_operator` 사유로 원장에 남습니다.
- `request-clear`: 남의 큐를 직접 비우지 않고 소유 노드에게 점검을 요청합니다(집행·거부
  판단은 그 노드 몫). 응답의 `delivered`가 false면 `reason`(`gate_blocked`·`human_typing`·
  `cooldown`·`channel_full`)이 함께 옵니다. **자동 재통지는 없습니다** — 쿨다운(60초)
  경과 후 직접 재발행하세요.

**큐 우회 적체 통지(OOB)는 '대상 1명당 한 통'의 다이제스트로 나갑니다.** 적체가 심하면
(depth가 경보 임계의 3배 또는 에이전트 소프트캡 도달) 데몬은 이벤트만 흘리는 데 그치지 않고
큐를 **우회해** 대상 노드와 지휘 역할(`master`·`cso`·`dept-master`)의 화면에 직접 통지를
꽂습니다. `dept-master`가 목록에 있는 이유는 부서 데몬에는 `master` 역할 자체가 없어서,
두 역할만 찾으면 부서 안에서는 통지 대상이 **아무도 없게** 되기 때문입니다. 종전에는 이
통지가 **적체된 노드마다 따로** 나갔습니다 — 통지 중복 억제 키에 적체 노드 번호가 들어 있어서,
막힌 노드가 N개면 한 사람이 같은 시간대에 최대 N회를 맞았습니다.

지금은 그 틱에 적체 판정된 **모든 노드를 한 통에 담은 다이제스트**가 대상 1명당
**최소 간격**(기본 300초 · `CYS_OOB_GLOBAL_MIN_SECS`)으로 들어갑니다. 즉 어느 노드가
원인이든 한 창에 **통지 1건**이고, 그 1건 안에 적체 노드 전 목록이 들어 있어 **정보가
사라지지 않습니다**. `queue.depth_high`·`queue.expired` 이벤트는 그대로 흐르고, 만기 전문은
`dead-letters.jsonl` 원장에, 현재 적체는 `cys queue list`에 그대로 있습니다(이쪽이
진실원입니다). TTL 만기 요약은 종전대로 노드별 키(30분 쿨다운)로 나가고, 사람이 직접 친
`request-clear`는 별도 레인(60초 쿨다운)이라 다이제스트 주기에 삼켜지지 않습니다.
`CYS_OOB_GLOBAL_MIN_SECS=0`은 쿨다운 해제입니다 — 적체가 지속되면 틱(5초)마다 다이제스트가
들어가므로 평시에는 권하지 않습니다.

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

### 12.3 위임 루프 (orchestra)

```bash
P=${CYS_PACK_DIR:-$HOME/.cys/pack}/bin
python3 $P/javis_orchestra.py check           # 필수 노드 생존 결정론 확인
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
| surface | `new-surface` `list` `attach` `read-screen` `resize` `close-surface` `quiesce` `tombstone` | 세션 생성·목록·미러링·화면 읽기·크기·닫기(자식 트리 전멸)·주입 보류·묘비 |
| 통신 | `send` `send-key` `events` `watch` | stdin 주입·키 주입·이벤트 구독·regex 완료 대기 |
| 역할·함대 | `launch-agent` `boot` `claim-role` `surface-role` `status` `fleet` `set-status` `todo-path` | 역할 노드 기동·일괄 부트·역할 등록/조회·관제 보드·자기보고·역할별 TODO 경로(`--role` 타역할 산출 · `--kind session-state\|recovery` 팩 정본 파일 경로 · 팩은 데몬 권위 우선) |
| 팩 경계 | `pack-scope-check` | 쓰기 대상이 남의 scope 팩인지 판정(훅 SOT · JSON 5키 `verdict`/`own_scope`/`target_scope`/`suggest`/`authority` · exit 0 고정) |
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
| `CYS_PROC_THRESHOLD` / `CYS_DUP_THRESHOLD` | 50 / 3 | 프로세스 수·중복 임계 |
| `CYS_AUTOKILL_DUP` | 0 | 중복 프로세스 자동 kill (opt-in) |
| `CYS_IDLE_SECONDS` | 300 | idle 감지 |
| `CYS_TYPING_GUARD_SECS` | 3 (0=off) | 사람 타이핑 보호 |
| `CYS_CONTEXT_THRESHOLD_PCT` | 60 | 컨텍스트 통보 임계 |
| `CYS_MAX_ACTIVE_WORKERS` | 8 | 워커 동시 상한 |
| `CYS_QUEUE_QUIET_SECS` / `CYS_QUEUE_DEPTH_ALERT` | 3 / 5 | followup 배달 조건·큐 깊이 경보 |
| `CYS_QUEUE_AGENT_SOFTCAP` | 25 (0=off) | 대상 큐 안의 **Agent 발신 항목 수** 상한. System·Human 발신은 면제 |
| `CYS_QUEUE_POLICY_MODE` | `enforce` | 소프트캡 도달 시 동작. 기본은 **거부+dead-letter 기록**이며, `log`(=이벤트만·적재 허용)는 명시해야 켜지는 관찰·롤백 모드입니다 |
| `CYS_QUEUE_TTL_SECS` | 3600 (0=off) | 큐 항목 만기(Agent 발신 기준). 만기분은 **폐기가 아니라** dead-letters.jsonl 로 이관(전문 보존). 0이면 등급 무관 전면 비활성 |
| `CYS_QUEUE_SYSTEM_TTL_SECS` | 14400=4h (0=off) | System 발신(스케줄러·거버넌스 제어 메시지) 만기 — 제어 메시지라 Agent(1h)보다 길게 잡되 무한 누적은 막습니다. 사람이 GUI에서 큐잉한 항목(`gui` 라벨)은 아래 전용 등급을 받고, `--important`는 면제. `CYS_QUEUE_TTL_SECS=0`이면 이 값과 무관하게 전면 비활성 |
| `CYS_QUEUE_GUI_TTL_SECS` | 86400=24h (0=면제) | 사람이 GUI에서 큐잉한 항목(`gui` 라벨) 전용 만기 — System(4h)보다 훨씬 길지만 **무기한은 아닙니다**. `gui` 표시는 보내는 쪽 자기신고라, 무기한이면 사람이 아닌 프로그램이 그것을 흉내 내 만기를 영구 회피할 수 있었습니다. `CYS_QUEUE_TTL_SECS=0`이면 이 값과 무관하게 전면 비활성 |
| `CYS_OOB_GLOBAL_MIN_SECS` | 300 (0=쿨다운 해제) | 큐 우회 적체 통지(OOB) **다이제스트**의 대상 1명당 최소 간격 — 적체 노드가 여럿이어도 한 통에 전 목록을 담아 1건만 들어갑니다(무손실). 0이면 적체 지속 시 틱(5초)마다 주입되므로 평시 비권장. TTL 만기 요약(노드별 30분)·`request-clear`(60초)는 별도 레인 |
| `CYS_FEED_REMIND_SECS` | 300 (0=off) | 승인 적체 재알림 |
| `CYS_MASTER_DEADMAN_SECS` | 900 (0=off) | 오케스트레이터 무반응 감지 |
| `CYS_AGENT_AUTORESTART` | 0 | 죽은 에이전트 자동 재기동 (3회 상한) |
| `CYS_RECALL_RETAIN_DAYS` | 30 (0=무제한) | 전사 보존 |
| `CYS_CONTROL_REDACT` | 0 | Control Center 세션 PII 가림 |
| `CYS_TODO_DIRS` | — | todo 감시 추가 루트(콜론 구분) |
| `CYS_NO_AUTOSTART` / `CYS_NO_AUTORESTORE` | — | 자동 기동/자동 복원 끄기 |
| `CYS_APPROVAL_SECRET_B64` | 자동 생성 | 승인 서명 시크릿 오버라이드 |
| `CYS_CHANNEL_RETAIN_DAYS` / `CYS_CHANNEL_OUTBOUND_TIMEOUT_SECS` | 7 / 30 | 채널 보존·발신 타임아웃 |
| `CYS_CLAUDE_CTX_WINDOW` | 200k (`[1m]`=1M) | 컨텍스트 창 크기 힌트 |

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

### RPC 메서드 (v0.12.28 기준 전수)

```
system.ping / identify / claim_role / resolve_role / pause / resume / gate_check / topology
surface.create / list / send_text / send_key / read_text / resize / rename / close /
        attach / set_meta / quiesce / wait_for
tombstone.set   events.stream   reinject.mark   status.set
ledger.register / deregister / list / kill
health.add_rule / list_rules
feed.push / reply / list
queue.list / clear / request_clear
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

`surface.list` · `system.identify` · `org.status`의 응답에는 **봉투 레벨**(surface 엔트리 안이
아니라 `result` 최상위)로 데몬 정체성 두 키가 실립니다.

| 키 | 뜻 |
|---|---|
| `pack_dir` | 이 데몬이 소속된 팩 디렉터리 절대경로 |
| `scope` | 그 팩의 이름(본사 `pack` · 부서 `pack-dept-<n>`) |

값은 데몬 기동 시 1회 캡처한 상수라 요청마다 흔들리지 않습니다. 클라이언트(`cys todo-path`·
`cys pack-scope-check`·pack-guard 훅)는 이 값을 자기 `CYS_PACK_DIR`보다 **우선하는 권위**로
씁니다. 추가된 키이므로 구버전 클라이언트는 무시하고, 구버전 데몬(키 없음)에 붙은 신버전
클라이언트는 로컬 env로 조용히 폴백합니다.

### 이벤트 (계열별)

```
surface.created/exited/crashed/closed/reaped/zombie_reaped/close_denied/quiescing/input_injected
agent.exited/recovered/restart_blocked/exit_unrecoverable
watchdog.load_high/proc_count_high/duplicate_procs/tick_panic   pane.idle
queue.enqueued/delivered/dropped/depth_high/clear_denied
queue.rejected/expired/merged/merge_demoted/oob_notified   health.dead_letter_write_failed
health.queue_wal_corrupt
ledger.registered/killed
feed.item.created/resolved/aging/timeout   feed.backlog_high
health.alert/action
schedule.fired/missed/error/command_done/tick_panic
autopilot.paused/resumed/master_changed/approval_checked/approval_signed
role.claimed/claim_denied   worker.limit_denied
usage.session_registered/updated/register_denied/report_denied/tick_panic
channel.* (bridge.exited·auth.denied·registered·message·outbound.<ch>·lockdown·… 15종)
daemon.started/stopping   acl.denied   context.threshold   status.changed   task.changed
todo.updated   approval.request   approval.stalled   master.deadman   osc.notify
```

---

## 18. 트러블슈팅 · 알려진 한계

**트러블슈팅**

| 증상 | 조치 |
|---|---|
| macOS "손상되어 열 수 없음" | 공증 빌드인지 확인, 우클릭→열기. 미서명 빌드는 quarantine 때문일 수 있음 |
| `cys ping` 실패 | 앱 실행(데몬 자동 기동) 또는 `cysd` 직접 기동. `cys doctor --fix` |
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
