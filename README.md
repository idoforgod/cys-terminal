# cys-terminal

**AI 에이전트 함대를 지휘하는 오케스트레이션 터미널.** macOS · Windows 크로스플랫폼.

터미널 멀티플렉서 + 로컬 데몬 + 관제 대시보드 + 멀티에이전트 운영체계(CYSJavis 팩)가
한 몸입니다. Claude Code·Codex 같은 CLI 에이전트 여러 개를 역할(마스터·워커·CSO·리뷰어)로
나눠 동시에 굴리고, 서로 소켓으로 대화시키고, 비용·컨텍스트·하드웨어를 실시간 관제합니다.

> 이 프로젝트의 코드는 대부분 **사람의 지휘 아래 AI 에이전트들이 작성**했습니다 —
> 커밋 로그의 `Co-Authored-By` 체인이 그 과정의 기록입니다. 이 저장소 자체가
> "AI 함대 오케스트레이션이 실제로 동작한다"는 실증입니다.

*Read this in [English](README.en.md).*

## 문서

| 문서 | 내용 |
|---|---|
| **[Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md)** | 설계 철학 10명제·시스템 아키텍처·보안 모델·불변식 |
| **[User Manual](USER-MANUAL.md)** | 설치부터 함대 운용, CLI·환경변수·프로토콜 전체 레퍼런스까지 |
| [INSTALL.md](docs/INSTALL.md) · [INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md) | 설치 상세 |
| [GUIDE-clean-reset-KR.md](docs/GUIDE-clean-reset-KR.md) | 초보자용 완전 초기화 가이드 (macOS·Windows) |
| [SECURITY.md](SECURITY.md) · [CONTRIBUTING.md](CONTRIBUTING.md) · [NOTICE.md](NOTICE.md) | 보안 신고 · 기여 · 서드파티 귀속 |

## 왜 만들었나

기존 터미널·멀티플렉서는 "사람이 명령을 치는 곳"입니다. AI 에이전트를 여러 개 띄우면
곧바로 한계가 옵니다 — pane끼리 서로 말을 걸 수 없고, 에이전트가 남긴 고아 서버가 쌓여
시스템이 마비되고, 누가 얼마나 쓰는지 보이지 않습니다. cys-terminal은 그 문제들을
1급 기능으로 해결하기 위해 처음부터 새로 작성한 독자 구현입니다.

그리고 네 번째 문제 — **에이전트들을 어떻게 조직으로 묶을 것인가** — 를 내장 팩
(CYSJavis: 역할별 절대지침 + 결정론 운영 도구)으로 해결합니다.

자원의 벽은 실행 중만이 아니라 **배포·업그레이드 중에도** 다룹니다 — 앱을 새 버전으로
교체하는 순간에도 "정지" 명령을 잃지 않는 것(크로스플랫폼 원자 교체)이 1급 목표입니다.

한 가지 문서 규약: 이 저장소는 개선을 서술할 때 **사용자가 잃는 것을 먼저 고지**합니다
— 마케팅 과장을 금지하고, 실제 코드·릴리스가 뒷받침하는 것만 적는 것이 규약입니다.

## 설계 원칙 (ABSOLUTE)

1. **양방향 소켓통신** — 단방향 send + capture 폴링을 쓰지 않는다.
   같은 소켓에 물린 모든 pane은 surface ID만 알면 서로에게 능동 push하는 **동등 노드**다.
   `cys send --surface surface:31 "..."` + `send-key Return` → 대상 pane의 **PTY stdin에 직접 주입** → 새 user turn 도착.
   서버→클라이언트 방향은 `cys events` 푸시 스트림(시퀀스 번호·재접속 이어받기).
2. **자원 거버넌스 1급 기능** — 고아 서버 누적 → load 폭주 → 401·hang을 원천 차단하는 완화책 내장.
3. **코어/UI 분리** — 데몬(cysd)은 UI와 무관하게 동작. UI가 hang이어도 소켓 제어 채널은 항상 살아있다(OOB 회생).
4. **fail-closed 서명** — 앱은 Tauri updater 서명, 팩은 minisign(공개키 바이너리 핀).
   검증에 실패하면 설치·전개 자체가 거부된다.
   *자기봉인 불변식* — 서명된 번들은 실행 중 자기 내용을 바꾸지 않는다(번들 `.pyc`
   자기생성 봉인 3층 + 서명 후 개수 대조 게이트).
5. **지침과 기계의 한 몸** — 역할별 절대지침·운영 도구·스킬(CYSJavis 팩)이 터미널과 함께
   빌드·서명·배포되고, 노드 기동 시 자동 주입된다.
6. **수동적 인지 계층(radio)** — 발견 알림과 결정을 물리적으로 분리한다.
   원칙1(능동 push)이 결정·조향 채널이라면, radio는 다중 워커가 병렬로 티켓을 돌 때
   서로의 **발견**을 알리는 수동적 계층이다. 승인·verdict·done 같은 결정 트래픽은
   radio에 싣지 않는다 — 결정의 단일 진실은 여전히 티켓·`gate-status`다.

## 설치

[Releases](https://github.com/idoforgod/cys-terminal/releases/latest)에서 받으세요.
받는 사람은 **데몬을 따로 설치할 필요가 없습니다** — 앱이 자동 기동하고 팩도 자동 설치됩니다.

- **macOS**: `cys_<버전>_aarch64.dmg` (Apple Silicon) — 동봉된 **"Install cys.app" 도우미**가
  숨김 스테이징 후 단일 시스템콜(`renamex_np`)로 원자 교체해, Finder 드래그가 복사 도중
  반쪽 번들을 노출하던 경합을 제거합니다(덮어쓰기 대신 도우미 설치 권장).
- **Windows**: `cys_<버전>_x64-setup.exe` — 데몬·CLI·런타임 동봉(자기완결 설치).
  PE 버전리소스·매니페스트·아이콘 임베드로 SmartScreen/Defender 마찰을 낮췄으나
  **여전히 미서명이라 첫 실행 경고가 뜰 수 있습니다**(정직 고지).
  상세: [docs/INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md)
- 24/365 상시 가동(선택): `cys daemon install` (launchd KeepAlive / 작업 스케줄러).
- 외부 터미널에서 `cys` 명령 쓰기: 앱 Control Center → **"셸에 cys 설치"** 1클릭.

설치·제거 상세는 [docs/INSTALL.md](docs/INSTALL.md), 사용법 전체는
[User Manual](USER-MANUAL.md).

## 빠른 시작

```bash
cys identify                                  # 내 surface 주소 확인
cys launch-agent --role worker --agent claude # 역할 노드 기동(절대지침 자동 주입)
cys send --to worker "상태 보고해줘"            # 역할 주소로 push
cys send-key --to worker Return               # 전송 확정
cys status --json                             # 전 노드 1콜 스냅샷
cys events --reconnect                        # 이벤트 푸시 구독 (폴링 대체)
cys run --scoped -- python -m http.server     # 생명주기 관리되는 스코프드 실행
cys boot                                      # 표준 노드 세트 일괄 기동(설치된 CLI 자동 감지)
```

## 구조

```
cys.app  Tauri 데스크톱 앱: 터미널 UI(xterm.js, TUI 위에서도 휠 스크롤·드래그 선택·복사
         기본 복원) + Control Center — 데몬의 thin client
cysd     헤드리스 코어 데몬: NDJSON 소켓 서버(UDS / win named pipe), PTY(portable-pty:
         macOS openpty·Windows ConPTY), vt100 화면 재구성, 이벤트 버스, watchdog,
         프로세스 원장, 사용량/비용 수집기, 영속 분석(SQLite), 스케줄러
cys      CLI: pane 안의 AI가 쓰는 동등 노드 클라이언트 (수십 종 서브커맨드 — `cys actions`로 열람)
pack     cysjavis-pack/: 절대지침 10·결정론 도구 90+·훅 25+·스킬 114+·스키마 4
         (빌드 시 임베드 · minisign 서명 배포 · 사용자 수정 파일 불가침)
```

모든 pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET` 자동 주입 —
pane 안의 AI는 `cys identify`로 자기 주소를 즉시 안다. PTY는 데몬 소유라서 앱을
재시작·재설치·업데이트해도 세션은 살아 있다(재attach).

## CYSJavis 팩 — 내장 멀티에이전트 운영체계

터미널을 설치하고 AI CLI를 연결하면 **master–worker–CSO–reviewer 멀티에이전트 운영체계**가
바로 구동됩니다. 시스템은 3층입니다:

| 층 | 내용 | 출처 |
|---|---|---|
| 코어 (기계 기능) | 양방향 소켓·승인 Feed·watchdog/원장·이벤트 push·세션 영속 | cys-terminal 코어 |
| CYSJavis 팩 | 역할별 절대지침·결정론 운영 도구·훅·스킬 | `cys init-pack` |
| 개인 층 | soul.md(우선순위·금지선)·장기기억 | **사용자가 사용하며 축적** |

**마스터 선언 위계 폴백** — 오너가 친 "너는 마스터다" 한 문장으로 5노드 팀이 자동
기동하고(부트/착수 분리), **2번째 선언은 충돌(거부)이 아니라 새 부서를 자동 생성**하며
첫 부서 생성 시 기존 마스터가 CEO로 자동 승격됩니다. 부서 생성 경로가 GUI 버튼 외에
'선언 경로'로 확장된 것입니다. 단, 에이전트가 배달·실행한 선언은 machine-origin 게이트로
이 폴백이 적용되지 않습니다(사람 채널에서만).

soul.md와 memory/는 **의도적으로 비어 있는 골격**입니다 — "운영 취향과 장기기억은 빌려
쓰는 것이 아니라 사용자 자신이 채워가는 것"이라는 설계 철학입니다. 자율주행(승인된 로드맵
자율 완주)은 오너가 soul.md에 명시적으로 부여할 때만 켜지며, **오너의 어떤 입력이든
즉시 일시정지시키는 kill-switch**가 최우선입니다. 대칭으로, 자율 착수의 **시작 권한도
오너 채널에서만** 나옵니다 — 큐에 미완 작업이 있어도 임무가 지정되지 않으면 보고 후
정지하는 **임무 게이트**가 kill-switch의 반대편(착수 게이트)을 지킵니다.

상세: [Architecture & Philosophy](ARCHITECTURE-AND-PHILOSOPHY.md) §2–4,
운용법: [User Manual](USER-MANUAL.md) §12.

## 세 가지 사용 구성 비교 — 온보딩 없이도 무엇을 얻는가

cys-terminal은 자비스 온보딩 없이 **그냥 claude만 연결해도** 전통 터미널과 다른 경험을 제공합니다.
세 구성을 33항목 × 6영역으로 비교한 결과입니다 (신선 기계 E2E 실측 골격 + 배포 코드 v0.14.x 라인 추적·팩 수치 재측정 기반):

- **①** 전통 터미널(iTerm 등) + claude CLI
- **②** cys-terminal + 순정 claude (자비스 온보딩 없이 일상 사용)
- **③** cys-terminal + 자비스 온보딩 ("너는 마스터다" 선언 → 5노드 풀 시스템, **기준**)

기호: **✕** 없음/불가 · **△** 부분/조건부 · **○** 제공 · **◎** 강화 체계 · **[실측]** 신선 기계 E2E 직접 확인

### A. 설치·시작 경험

| 항목 | ① 전통 터미널 + claude | ② cys + 순정 claude | ③ cys + 자비스 (기준) |
|---|---|---|---|
| 자동 배치 | ✕ | ○ 전 스킬 세트(팩 570+파일)+격리 config [실측] | ○ 동일 + preflight 전체 배선 |
| 개인 `~/.claude` 보호 | — (직접 사용) | ○ 불가침 [실측] | △ base 온보딩 시 계장 가능 |
| Claude 첫기동 게이트 | △ 5단 다이얼로그 1회 | △ 동일 — 첫 스킬런 막힘(F1) [실측] | △ 동일 함정 |
| 로그인 | △ 1회 | △ 격리 config 별도 1회(F2) [실측] | △ 프로필별 각 1회 |
| 온보딩 절차 | ✕ | ✕ 즉시 사용 (승격 문만 대기) | ○ 선언 → 5노드 부트 |

### B. 세션 컨텍스트 — claude가 알고 시작하는 것

| 항목 | ① 전통 터미널 | ② cys + 순정 | ③ cys + 자비스 |
|---|---|---|---|
| 자동 주입 지침 | ✕ | ○ cys 치트시트+4대 지침+품질 게이트 | ◎ 디렉티브+soul 전문 |
| 시작 주입량 | 0 | ~1.4KB [실측] | ~134KB [실측] |
| 등록 훅 수 | 0 | 2개 [실측] | 10개+ [실측] |
| 권한 모드 | 사용자 선택 | 사용자 선택 | bypass + guard 짝 |

### C. 에이전트 능력 — claude가 할 수 있는 것

| 항목 | ① 전통 터미널 | ② cys + 순정 | ③ cys + 자비스 |
|---|---|---|---|
| 타 pane 관측 | ✕ 서로 존재 모름 | ○ `cys list`·`read-screen` 자발 사용 실증 [실측] | ◎ 능동 모니터링 체계 |
| pane 간 메시지 | ✕ | ○ `cys send --surface` 자발 사용 실증 [실측] | ◎ 역할 주소·양방향 소켓 |
| GUI 승인 요청 | ✕ 터미널 프롬프트뿐 | ○ `cys feed push --wait` | ◎ 승인 자동화 체계 |
| 이벤트 push 구독 | ✕ 폴링뿐 | ○ `cys events` | ◎ EVT v1 계약 12종 |
| 서버 생명주기 | ✕ 수동 | ○ `cys run --scoped` | ◎ + 사전 자원 게이트 |
| 예약·웨이크업 | ✕ OS cron 수동 | ○ `cys schedule` | ◎ 자율주행 웨이크업 |
| 역할 주소 통신 | — | ✕ | ○ master·cso·worker·reviewer |

### D. 환경 서비스 — 데몬이 묻지 않고 주는 것

| 항목 | ① 전통 터미널 | ② cys + 순정 | ③ cys + 자비스 |
|---|---|---|---|
| 컨텍스트% 관측 | ✕ | ○ usage-register 자동 [실측] | ◎ + 60% /clear 관리 |
| 폭주·중복 서버 감지 | ✕ | ○ watchdog 전 surface | ◎ + 착수 전 게이트 |
| 크래시·좀비 감지 | ✕ | ○ 자동 | ◎ + 노드 복구 |
| pane 영속·복원 | △ 앱에 따라 | ○ phoenix [실측] | ◎ + 3층 복원 체계 |
| 부서·워크스페이스 GUI | ✕ | ○ | ◎ 조직 단위 운영 |
| 무중단 업데이트 | — | ○ | ○ |

### E. 스킬·지식 자산

| 항목 | ① 전통 터미널 | ② cys + 순정 | ③ cys + 자비스 |
|---|---|---|---|
| 스킬 보유 | ✕ 수동 설치 | ○ 114+종 + 보드 6종 [실측] | ◎ + 프로필 설치·role 주입 |
| 스킬 실행 | ✕ 수동 호출 | ○ 보드 → 75초 완주 [실측] | ◎ + 티켓·게이트·검증 |
| MCP 등록 | ✕ 수동 | ✕ | ○ 자동 (serena·nlm) |
| 장기기억 | ✕ | ✕ | ○ javis_memory + 훅 |
| 오너 SOT 연동 | ✕ | ✕ | ○ NotebookLM 의무 |

### F. 조직·자율성 — 자비스 온보딩의 고유 가치

| 항목 | ① 전통 터미널 | ② cys + 순정 | ③ cys + 자비스 |
|---|---|---|---|
| 팀 구성 | ✕ | ✕ (선언 1문장 승격 가능) | ○ 5노드 자동 + 부서 자동 기동(2번째 선언) |
| 위임·검증 루프 | ✕ | ✕ | ○ 티켓·리뷰어·RSI·eval |
| 노드 간 발견 공유(radio) | ✕ | ✕ | ○ 병렬 티켓 발견 채널(결정 트래픽 제외) |
| 세션 간 작업 복원 | ✕ | △ pane 수준만 | ○ SESSION_STATE·RECOVERY |
| 자율주행 | ✕ | ✕ | ○ 4자 수렴·denylist 경계 |
| 안전 게이트 | △ Claude 기본 | △ 기본 (+실행물만 워커 규율) | ◎ guard·grill·skillscan |
| 진행 보고·관제 | ✕ | ✕ | ○ report·HUD·Feed |

> **F1** = Claude Code 자체 첫기동 다이얼로그 5단(Enter→보안 노트→터미널 설정→폴더 신뢰→bypass 경고)에서
> 최초 1회 사람 통과 필요. **F2** = 격리 config는 Keychain 자격증명이 config 경로별로 분리되어
> (`Claude Code-credentials-<sha256(경로)[:8]>`) 자체 `/login` 1회 필요.

**한 줄 독법** — ①→②는 **설치만으로 생기는 격차**(관측·통신·보호·스킬: 전통 터미널엔 범주 자체가 없음),
②→③은 **조직·기억·자율성의 격차**(자비스 고유 가치, F영역). ②의 실질 마찰은 1회성 게이트 2개(F1·F2)뿐이며,
②에서 ③으로는 **"너는 마스터다" 선언 한 문장**으로 승격됩니다.

## JavisRadio vs AgentRadio — 우리가 우세한 것, 부족한 것

cys의 수동적 인지 계층(설계 원칙 6 · T5-20)은 Coral Protocol의 연구 **AgentRadio**
(arXiv:2607.28430)의 3 프리미티브를 이식해 기계 게이트로 격상한 재구현입니다.
원 연구는 "4개의 코딩 에이전트가 일을 멈추지 않고 서로의 방송을 듣게 하면"(passive
awareness) SWE-Atlas QnA 124태스크에서 단일 32.3% → 4에이전트 62.1%가 됨을 통계
검정(McNemar p=0.0023)으로 보였고, DeepSeek 재현(29.0%→50.8%, p=0.0026)과 **B1 예산
통제군**(1명에게 6배 예산을 줘도 37.9% — "컴퓨팅을 더 써서 이긴 것 아니냐"는 반박을
설계로 차단)까지 갖춘 교과서급 실험입니다. 아래는 원 논문·원 소스 전수조사(2026-08-14,
두 독립 세션 + 적대 검증 2기 + 수치 재실행의 삼중 검증)로 10개 축을 채점한 판정 —
**어디서 이기고 어디서 지는지**를 먼저 보입니다.

### 한눈에 보는 판정 — cys/자비스 기준: 우세 8 · 조건부 우세 1 · 열세 1

| # | 비교 축 | 판정 | 한 줄 근거 |
|---|---|:---:|---|
| 1 | 통신 (수동적 인지) | ⚠️ 조건부 우세 | 3 프리미티브 이식 + 방어 14종·유실 0 표면화(중복은 감사 가능한 예외)·철회 시 오염 연쇄 폐쇄·멱등 큐 — 단, **개념의 원조와 실전 데이터는 AgentRadio** |
| 2 | 역할 구성 | ✅ 우세 | 이종 3사 리뷰어(claude·agy·codex)로 상관 오류 차단 vs 동종 모델 4기(넷이 같은 착각을 하면 못 잡음) |
| 3 | 검증·품질 게이트 | ✅ 우세 | 증거 없는 완료 보고는 exit code로 기계 거부 + 4자 수렴 게이트 vs 파이프라인의 유일한 기계 게이트가 answer.txt 존재 확인 — 만장일치는 "APPROVE를 눈으로 세라"는 프롬프트 문장 |
| 4 | 복원·내구성 | ✅ 우세 | 다층 복원 정본(SESSION_STATE·RECOVERY·todo 영속) + 실사고 기원 수리(메시지 소실 AA20→단일 임계구역, 쿼터 72% 소진→미션 게이트) vs 프로세스 재개 단층 — 서버 사망=팀 상태 전멸, 토큰 만료=공회전 |
| 5 | 자원 통제 | ✅ 우세 | 착수 전 자원 게이트·프로세스 원장·그룹 정리 vs 없음(컨테이너 통째 폐기에 의존) |
| 6 | 사람 개입 (HITL) | ✅ 우세 | 승인 Feed(exit 0/2/3)·kill-switch·denylist 경계 vs "인간에게 묻지 마라"가 사양 |
| 7 | 실사용 범용성 | ✅ 우세 | 상시 운영 + 스킬 114종 + 다부서 + 오프라인 로컬(네트워크 리스너 0) vs 벤치마크 1도메인 재현용(Docker+Modal 클라우드+Harbor 버전 고정 필수, 레포 활동 17일) |
| 8 | 배포 성숙도 | ✅ 우세 | 공증·서명 2채널 자동 업데이트·6플랫폼·출고 게이트 CI vs 무패키징·버전 0.1.0 하드코딩·체크섬 없는 구글드라이브 106MB JAR |
| 9 | **성능 실측 증명** | ❌ **열세** | AgentRadio는 공개 벤치마크 124태스크 × 4구성 × 2모델 + 통계검정으로 자기 방식을 증명 — **우리는 시스템 수준 정확도 실측이 없고, radio 실전 파일럿도 미가동** (해소 착수: 같은 문제지로 자비스판 벤치마크 JAVIS-BENCH 파일럿 가동) |
| 10 | 생태계 | ✅ 우세 | 결정론 도구 86 + 스킬 114 + 이종 CLI 어댑터가 **이미 내부에서 가동** vs 프리미티브 3종(외부에 MCP 오픈 프로토콜·에이전트 생태계 야망은 보유) |

> 공정 각주: AgentRadio는 하나의 가설을 증명하기 위한 **연구 산출물**이라 5·6·8축의
> 부재는 설계 목적 밖입니다. 총점이 아니라 축별 근거로 읽어야 하며, 9축 열세는 우리의
> 다음 숙제로 명시합니다. 채점(같은 회사 AI 심판)도 전 구성에 동일 심판이 고정돼
> **L2→L3 상대 비교에서는 편향이 상쇄**됩니다 — 편향이 위협하는 건 절대 수치와 "1위"
> 서사 쪽(현 리더보드 단일 1위 63.17%가 62.1%를 상회하나 ±5 수준 신뢰구간이 서로
> 겹쳐 어느 쪽 우위도 통계적 단정은 불가 · AgentRadio는 미등재 자기보고). 비용도 저자 공개 수치로 태스크당 $2.96 → $19.45(6.6배)입니다.

### 정량 저울 — 숫자로 보는 체급 차이 (전 수치 실측·재검증)

| 지표 | AgentRadio | cys/자비스 스택 |
|---|---|---|
| 코드 규모 | 약 3,300줄 (Python 2,017 + 셸 1,301) | **약 169,000줄** (Rust 63,371 + 팩 Python 105,833 등) = **약 50 : 1** |
| 자체 코드 테스트 | **0건** (자체 하네스 테스트·CI 없음) | **약 1,700건** — Rust `#[test]` 883(src)·920(전 레포) + 팩 531 + radio 297(레드팀 23종 포함·당일 재실행 전건 PASS) + UI 테스트 파일 16 |
| 통신 표면적 | 프리미티브 3종 | CLI 서브커맨드 66종 (radio 하위명령 17·exit 계약 10종 포함) |
| CI | 없음 (보이는 커밋 1개) | 5레인 + 불안정 테스트 게이트 + 공증 회귀 검사 |
| 벤치마크 자산 | **124문제 · 루브릭 1,306개 · 오염 감지 카나리아 · 통계 검정** (상대의 최강점) | 없음 — JAVIS-BENCH로 착수 |
| 문서 언어 | README 6개 언어 (상대 우위) | 한국어 정본 + 영문 README |

> 규모 50:1은 양날의 검입니다 — 우리 깊이의 증거인 동시에 그만큼 복잡하다는 뜻이고,
> 그들의 작음(한나절이면 전수 감사 가능)은 과학적 미덕입니다. 단 그 미덕은 감사
> 불가능한 106MB 서버 바이너리가 깎아 먹습니다.

### ❌ 우리가 부족한 것 — 숨기지 않고 전부 (같은 전수조사의 반대면)

| # | 부족 항목 | 사실 |
|---|---|---|
| 1 | **공개 벤치마크 증거 0** | "오케스트레이션이 성적을 올린다"는 결과 수준 증명이 우리에게 없음 — 같은 문제지(SWE-Atlas QnA)로 단일 vs 자비스식 오케스트레이션을 채점하는 JAVIS-BENCH 파일럿을 착수해 해소 진행 중 |
| 2 | **radio 실전 0건** | 구현·테스트·계약은 완성이나 개통 티켓 없음 — 라디오 층위만 보면 실전 증거는 상대가 많음 |
| 3 | **지적 우선권은 상대** | 수동적 인지 개념·3 프리미티브는 AgentRadio의 것 — 우리 사양서가 명문으로 이식 선언. 우리는 강화 이식판 |
| 4 | **단일 머신·노드 인증 없음** | radio는 단일 머신 전용이고 노드 인증이 없음('master' 이름은 무조건 허용) — 상대의 MCP 서버는 크로스 프레임워크·멀티호스트 지향 |
| 5 | **문서 언어 접근성** | 문서량은 크게 앞서나 한국어 위주 — 상대는 README 6개 언어 |
| 6 | 기타 정직 고백 | 비용 공표치 없음(상대는 6.6배까지 공개) · Windows Authenticode 미서명 · radio 자체 문서도 완전한 exactly-once·무청각 0은 보장 못 함을 자인(격상은 부분적) |

### 상세 — radio 계층 1:1 대조 (구조 우위의 근거)

| | AgentRadio (원 연구) | JavisRadio (cys 팩) |
|---|---|---|
| 표면적 | 프리미티브 3종 (create_thread / send_message / wait_for_mention) | 대응 3종 + 방어 14종 = 서브커맨드 17종 |
| 방송 진위 | 검증 없음 — 내용 그대로 전파 | FACT는 증거(파일·라인·스니펫) 실존을 기계 검증 — 실패 시 가설·미검증으로 자동 강등 |
| 중복·유실 | 멱등키·ack·순번 없음(멘션은 서버 push, 타임아웃 폴백 감지가 grep 문자열 세기) — 중복 제거를 LLM 인지에 위탁 | 단조 seq + 표면화/수용 커서 분리 — "0회 금지 > 1회 초과 금지" 불변식 위계 |
| 거짓 방송 철회 | 개념 없음 | `retract` — 철회하면 그 방송을 인용한 방송까지 오염 연쇄 폐쇄 |
| 완료 게이트 | 유일한 기계 확인 = answer.txt 존재(2시간 폴링) — 만장일치는 프롬프트 문장 | done-check가 미표면화·미해소 방송을 exit code로 거부 (exit 계약 10종) |
| 인프라 | 상주 메시지 서버(106MB JAR·인증키 'test') — 서버 사망 = 팀 상태 전멸 | 상주 서버 없음 — append-only 파일이 정본 (로테이션 seq 연속·폐쇄 후 보관 이동) |
| 남용 방어 | 없음 | 쿨다운 · 발신 차단기 · 비밀 마스킹 · 레코드 캡 · pause 격리 |
| 검증단 | 동종 모델 4기의 상호 합의 | 이종 3사 리뷰어 · producer≠evaluator 게이트와 결합 |

JavisRadio의 현재 증거는 테스트 297건/73 밀폐 케이스 — 레드팀 23종 포함, "위반이
정확한 exit로 막힌다"를 잠그는 적대 테스트 — 까지이며 **실전 파일럿은 미가동**입니다
(알려진 한계 참조). 9축 열세를 해소하는 순서도 ①radio 실전 파일럿 → ②JAVIS-BENCH
본실험입니다.

## Control Center (실시간 관제 + 영속 분석)

앱의 전용 풀 패널 — cysd가 단일 RPC로 플릿·사용량·시스템을 제공하고(외부 대시보드 무의존),
영속 분석은 cysd 내장 SQLite(`analytics.db` · open 실패 시 graceful degrade)에 쌓입니다.
철학: **로컬 우선**(데이터가 머신 밖으로 나가지 않음) · 추가 인프라 0 · 에이전트 0ms
지연(hook은 fire-and-forget).

| 탭 (9) | 내용 |
|---|---|
| **Live** | 노드 플릿 · 하드웨어(CPU 코어별·GPU·NPU·MEM 2초 실시간) · 오늘 토큰/비용/모델믹스 · 경보 스트립 |
| **비용·효율** | 영속 집계 — 토큰 4분해 · 모델별 비용(단가미상 표시) · 캐시 절감·재사용율 · 조직단위 비용 |
| **스킬·에이전트** | 스킬/툴/위임 호출 집계 · 실패율(exit_code≠0) · 반복 실패 |
| **세션** | 세션 타임라인 · 활동 리본 · 전사 발췌 드릴다운 · ⭐즐겨찾기 · 🔒PII 가림 |
| **추세·주간** | 주간 WoW% 델타 · 효율 리더 · 스킬 자산(신규/휴면) |
| **학습** | 자기개선(RSI) 라운드 타임라인 · 채택/롤백 · 발견 누적 |
| **스킬 보드** | 큐레이션 스킬 버튼 클릭 = 일회용 워커 실행(HITL 미리보기) |
| **작업** | 모든 부서×노드의 현재 업무(관측 전용) · 자기보고/파생 신뢰 배지 |
| **승인 Feed** | 승인 요청 집중 처리(Allow/Deny) |

그 밖에: ⌘K Command Palette(노드 점프·60% 컨텍스트 순회·승인 처리) · Glance 모드(⌘G,
비기술자용 요약 화면) · 워크스페이스 그룹 · **부서**(독립 데몬으로 프로젝트 격리) ·
RBAC PII 가림(`CYS_CONTROL_REDACT=1`). 상세 설계: docs/CONTROL_CENTER_DESIGN.md

## 자비스 네이티브 기능 (22건)

> 설계 철학: **지침이 오케스트레이터에게 수동으로 시키는 모든 운영 의무 = 터미널의 기능 결함 목록.**
> ①규약→데몬 보증으로 기계화 ②자기보고 우선·화면 파싱은 fallback ③자동화 3단 안전등급(alert→escalate→act, deny-by-default).

| # | 기능 | 명령/이벤트 |
|---|---|---|
| T1-1 | **자기보고**: 에이전트가 상태·컨텍스트%·작업을 직접 신고 | `cys set-status --state working --context 57` → `status.changed` |
| T1-2 | **관제 보드**: 전 노드 1콜 요약 | `cys status [--json]` · `cys fleet`(전 부서) |
| T1-3 | **발신자 신원·ACL**: 커널 peer pid로 from 검증 + role→role 송신 정책 | `acl.json` · 거부 시 `acl.denied` |
| T2-4 | **컨텍스트 사이클 집행기**: 저장 지시→파일 게이트→clear→지침 재주입→재개 | `cys cycle-agent --role worker` |
| T2-5 | **에이전트 사망 즉시 감지** (+옵션 자동 재기동, 인증 오류 시 차단) | `agent.exited/recovered` · `cys node-recover --role X` |
| T2-6 | **조직 복원**: 토폴로지 영속 + 일괄 재기동·재주입 | `cys restore [--include-master]` |
| T2-7 | **디렉티브 드리프트 감지·재주입** | `cys reinject --role X [--check]` |
| T2-8 | **오케스트레이터 dead-man**: 단일 장애점 봉합 | `master.deadman` 이벤트 |
| T3-9 | **todo 워치**: 역할별 TODO 파일 mtime 감시→진행률 집계 | `todo.updated` · `cys todo-path` |
| T3-10 | **원샷 타이머** (+fresh TTL `--close-after`) | `cys schedule add --id x --in 20m --text ... --to role` |
| T3-11 | **역할 글롭 브로드캐스트** | `cys send --to 'reviewer-*' "..."` |
| T3-12 | **feed aging 재알림**: pending 승인 무음 적체 차단 | `feed.item.aging` |
| T3-13 | **입력 안전**: 타이핑 가드 · 원자 권위 전달 | `typing_guard` 거부 |
| T3-14 | **델타 읽기·완료 대기**: 단조 라인 커서 + 데몬측 regex 감시 | `cys read-screen --since N` · `cys watch --until <re>` |
| T4-15 | **kill-switch**: 큐 배달·스케줄 발화 동결 | `cys pause/resume` · `cys gate-check` |
| T4-16 | **승인 격상**: 화면 스캔→이벤트+feed (자동 응답 절대 없음) | `approval.request` |
| T4-17 | **헬스룰 조치 바인딩**(opt-in): queued 배달만 일시정지 | `cys add-health-rule n p --action pause-queue` |
| T4-18 | **트랜스크립트 해시체인 attest**: 변조 증거성(producer≠evaluator) | `cys attest pin/verify` |
| T4-19 | **recall 보존 정책**: 트랜스크립트 무한 성장 차단 | `CYS_RECALL_RETAIN_DAYS` |
| T5-20 | **수동적 인지(radio)**: 병렬 워커의 발견 공유 · FACT 진위검증(파일·라인·스니펫, 미검증은 자동 강등) · BLOCKER 게이트 · 결정 트래픽 금지 | `javis_radio open/send/wait/read` |
| T5-21 | **BOOT_SNAPSHOT**: clear/컴팩트 후 기억을 읽기전용 원장 다이제스트로 복원(명령형 문구 0 · 마스터 게이트 · 워커·리뷰어 pane 주입 0) | `javis_snapshot generate` |
| T5-22 | **귀속 판별 원장**: pane 텍스트 위조 의심 시 수정 착수 전 배달 원장 선행 조회(무증거 귀속 무효) | `javis_mission delivery-path`·machine-origin |

## 자원 거버넌스 (3대 완화책)

| 완화책 | 기능 | 명령/이벤트 |
|---|---|---|
| ① 로그인 감지 강화 | 모든 출력 라인에 헬스 룰(기본: Not logged in·401·token expired·rate limit) 매칭 → 30초 디바운스 push. **자기증폭 차단**: 경보 문장은 `‹health-rule›`로 전 트리거를 마스킹해 내보내고(발신 봉인 — 어떤 룰에도 재매칭 불가), 경보를 논하는 줄(기계장치 이름·인용·한글 산문)은 매칭에서 제외(수신 격리) | `health.alert` · `cys add-health-rule <name> <regex>` · `CYS_HEALTH_NARRATION_CJK_MIN` |
| ② 짧은 작업 단위 | idle(기본 300초 무출력) 감지 push → 분할·점검 판단 | `pane.idle` 이벤트 |
| ③ 서버 생명주기 강제 종료 | **scoped 실행**(새 프로세스 그룹+원장, 종료 시 그룹째 정리) · **close-surface**(자식 트리 전멸) · **watchdog**(load/자식 수/중복 명령 감지) | `cys run -- <cmd>` · `cys ps` · `cys kill <pid>` · `watchdog.*` |

## 승인 Feed · 인플라이트 큐

```bash
cys feed push --wait --title "git push 승인" --body "..."   # 결정까지 블록 (exit 0=allow, 2=deny, 3=timeout)
cys feed reply <request_id> allow                            # CLI 또는 UI Allow/Deny 버튼
```

자동 응답은 없습니다(HITL) — 요청 노드의 자기결재도 데몬이 거부합니다. 반복 위험 명령은
`cys approval sign`(master 전용, HMAC signed-prefix)으로 1회 서명해 통과시킵니다.

- 기본 전송(`cys send`)=**steer**: 즉시 stdin 주입 — 실행 중 입력을 조향으로 소화.
- `cys send --queued`=**followup**: 대상이 3초 이상 조용해지면 한 틱에 한 건씩 자동 배달.

## 업데이트 — 이중 채널 + 무중단

| 배지 | 채널 | 방식 |
|---|---|---|
| `!` | 앱(바이너리) | Tauri updater 서명 검증 → 세션 가드 → 설치·재시작 → 팩 반영+노드 자동 복귀 |
| `↻` | 팩(운영체계) | **무중단** — minisign 검증 → 원자 트랜잭션 → 라이브 노드 재주입. 재시작 0, 세션·데몬 생존 |

시작 시 + 6시간마다 조용히 확인. 재설치 후 "디스크는 새 버전·프로세스는 구 데몬" 스큐가
남으면 배지 클릭 교대 또는 유휴 자동 교대(라이브 세션 0일 때 — 무손실)로 해소됩니다.
진단·수리는 `cys doctor [--fix]`, 설치본 코드서명 봉인 자가진단은 `cys doctor app-seal`.

**커스터마이즈와 공존**: 사용자 수정본은 업데이트가 파괴하지 않습니다 — user-owned 파일은
보존+신버전 `.new` 병치, system 파일은 치유 전 `.user` 보존, `~/.cys/local/` 오버레이
(디렉티브 append·스킬 shadowing·훅 후행)는 업데이트가 존재 자체를 모릅니다.
`cys pack-plan`(사전 미리보기) · `cys pack-merge`(3-way/AI 병합) — 상세는
[User Manual §12.7](USER-MANUAL.md#127-커스터마이징--업데이트와-공존하는-방법).

## 채널 브리지 (Slack·Discord)

함대의 승인 요청·보고를 외부 메신저로 내보내고, 허가된 발신자의 원격 승인을 받습니다 —
발신자 allowlist · 원격 승인 별도 허가 · 즉시 잠금(lockdown) · 모양 기반 redact 내장.
`cys channel status` 참조.

## 프로토콜 · 환경변수

NDJSON(한 줄 = JSON 하나), RPC 수십 종 + `channel.*` 13종, 이벤트 수십 종.
전수 목록과 환경변수 표는 [User Manual §16–17](USER-MANUAL.md)에 있습니다.

## 소스 빌드 (기여 시)

```bash
git clone https://github.com/idoforgod/cys-terminal
cargo build --release
./target/release/cysd &                      # 데몬 (중복 기동 자동 거부)

cd ui && sh build.sh                          # 프런트엔드 번들 (bun)
cargo build -p cys-app                        # dev 실행: ./target/debug/cys-app
bun x @tauri-apps/cli build                   # 배포 번들
```

주의: ui/ 수정 후 앱 재빌드 필요(프런트엔드가 바이너리에 임베드됨). 세션(PTY)은 데몬 소유 —
UI 재시작·앱 재설치에도 세션 유지(재attach).

## 보안 모델

- 네트워크 리스너 없음 — 사용자 소유 Unix 소켓(macOS) / DACL 봉인 named pipe(Windows)만.
- 발신자 신원은 커널 peer pid로 검증(자기신고 불신) · role→role ACL · 능력 게이트는
  deny-by-default(리뷰어는 읽기 전용).
- **귀속(누가 보냈나)은 화면 문자열이 아니라 배달 원장이 판정한다** — 자기신고·pane 문자열을
  불신하고, 원장 조회 증거 없이는 귀속 주장을 무효로 다룬다.
- 업데이트 이중 서명 — 앱은 Tauri updater 서명, 팩은 minisign(공개키 바이너리 핀·replay
  단조성·fail-closed).
- 승인 자동응답 없음(HITL) · 자기결재 차단 · 외부 URL은 하드 허용목록(로컬 설정으로만 확장).
- 발행 전 비밀/PII 게이트: `scripts/secret-scan.sh --all` (fail-closed). 비밀 마스킹은
  비가시 문자 우회를 하드코딩 열거가 아니라 **유니코드 카테고리 포괄**(Cc/Cf/Zl/Zp 제거)로
  차단한다("열거는 다음 우회에 진다 — 범주로 막는다").
- **출고 게이트**: 릴리스 전 실사용자 경로를 CI가 재현 — macOS는 사본에 quarantine을 부착 후
  Gatekeeper 실평가(spctl/codesign/stapler), Windows는 PE 평판으로 검증하고, 못 통과하면
  업로드 자체를 차단한다(fail-closed · 정적 패턴이 아니라 사용자가 실제로 걷는 경로를 재현).

취약점 신고: [SECURITY.md](SECURITY.md) · 상세: [Architecture & Philosophy §6](ARCHITECTURE-AND-PHILOSOPHY.md)

## 알려진 한계

- macOS에서 sysinfo가 cmdline 전체를 못 읽으면 프로세스명으로 중복 그룹핑(과탐 가능).
- `cys run` 중 Ctrl-C로 CLI가 죽으면 그룹 정리가 watchdog 주기(5초)로 넘어감.
- Control Center의 GPU/NPU 실시간은 현재 macOS(Apple Silicon) 전용 — Windows는 CPU/MEM만.
- NPU는 활용률(%) 공개 API가 없어 실측 전력(W)으로 표시(macOS).
- 단일-UID 신뢰 모델 — 승인 서명·자기결재 차단은 같은 계정 내 악성 프로세스에 대한
  암호학적 방어가 아니라 탐지·fail-safe 층입니다.
- **임무 게이트**는 동일 UID의 위조를 암호학적으로 막지 못합니다 — 배달 원장 감사흔적으로 다룹니다.
- **Windows 업그레이드 원자성** 수리는 맥 개발기 코드판독·모델검증까지이며, 실기 확인은 진행 중입니다(정직 고지).
- **macOS 설치본은 무인증서**입니다 — 설치도우미로 마찰을 낮췄어도 첫 실행 경고가 뜰 수 있고,
  '반쪽 설치 vs quarantine' 판별은 `cys doctor app-seal`로 합니다.
- **radio**는 교차채널 exactly-once·난청(놓침) 창 0을 원리적으로 보장하지 못합니다(해소 불가 — 관리 대상 잔여 리스크).

## 문제 해결 · 초기화

- macOS **"손상되어 열 수 없음"** 은 두 원인입니다 — ① 반쪽 설치(드래그 복사 경합) ② quarantine 속성.
  판별은 `cys doctor app-seal`, 설치는 **"Install cys.app" 도우미** 사용을 권장합니다.
- **완전 초기화**(윈도우 WebView2 저장값·잔존 부서 격리본 삭제 포함)는
  [docs/GUIDE-clean-reset-KR.md](docs/GUIDE-clean-reset-KR.md)를 따르세요.

## 기여 · 라이선스

기여는 [CONTRIBUTING.md](CONTRIBUTING.md), 서드파티 귀속은 [NOTICE.md](NOTICE.md) 참조.
MIT License ([LICENSE](LICENSE)) · 문의: **cysinsight@gmail.com**
