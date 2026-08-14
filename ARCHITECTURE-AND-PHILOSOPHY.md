# Architecture & Philosophy

> cys-terminal + CYSJavis 팩이 **무엇을, 왜, 어떻게** 이렇게 만들었는지를 설명하는 문서입니다.
> 설치·사용법은 [User Manual](USER-MANUAL.md), 첫인상은 [README](README.md)를 보세요.
> 본문 주장의 근거는 저장소 소스 경로로 표기합니다. (v0.14.x 기준)

---

## 0. 한 문장 정의

**cys-terminal은 "AI 에이전트 함대"를 하나의 회사처럼 굴리기 위한 터미널·데몬·관제탑이고,
CYSJavis 팩은 그 회사의 취업규칙·운영도구·기억 골격이다.** 둘은 따로 설치하는 별개 제품이
아니라, 하나의 저장소에서 함께 빌드·서명·배포되어 **한 몸으로 동작**한다
(`cysjavis-pack/README.md`: "터미널의 기계 기능과 역할별 절대지침 문서가 한 몸으로 동작한다").

> 이 저장소의 코드 대부분은 **사람의 지휘 아래 AI 에이전트들이 작성**했다.
> 커밋 로그의 `Co-Authored-By` 체인이 그 기록이며, 이 저장소 자체가
> "여기서 설명하는 오케스트레이션이 실제로 동작한다"는 실증이다.

---

## 1. 문제의식 — 세 개의 벽

기존 터미널·멀티플렉서는 "사람이 명령을 치는 곳"으로 설계됐다. 그 위에 AI CLI 에이전트를
여러 개 띄우면 곧바로 세 개의 벽에 부딪힌다.

1. **대화의 벽** — pane끼리 서로 말을 걸 수 없다. 에이전트 A가 B에게 일을 시키려면 사람이
   복사·붙여넣기를 해야 하고, 결과 확인은 화면 폴링뿐이다.
2. **자원의 벽** — 에이전트가 남긴 고아 서버·프로세스가 쌓여 load가 폭주하고, 인증이 깨지고
   (401), 시스템이 hang 된다. 누구도 정리 책임을 지지 않는다.
3. **관측의 벽** — 누가 얼마나 쓰는지(토큰·비용·컨텍스트), 지금 무엇을 하는지 보이지 않는다.

cys-terminal은 이 세 가지를 **1급 기능**으로 해결하기 위해 처음부터 새로 작성한 독자
구현이다. 그리고 네 번째 벽 — **"에이전트를 어떻게 조직으로 묶을 것인가"** — 를 CYSJavis
팩(역할별 절대지침 + 결정론 운영 도구)으로 해결한다.

---

## 2. 3층 구조 — 코어 / 팩 / 개인 층

시스템 전체는 세 층으로 분리된다 (`cysjavis-pack/README.md` §3층 구조).

| 층 | 내용 | 출처 |
|---|---|---|
| **코어 (기계 기능)** | 양방향 소켓·승인 Feed·watchdog/프로세스 원장·이벤트 push·세션 영속·서명 검증 | cys-terminal 바이너리 (`src/`) |
| **CYSJavis 팩 (운영체계)** | 역할별 절대지침·결정론 운영 도구·훅·스킬·어댑터 | `cys init-pack` (`cysjavis-pack/`) |
| **개인 층** | soul.md(우선순위·금지선)·장기기억(memory/)·프로젝트 컨텍스트 | **사용자가 사용하며 축적** |

세 번째 층이 핵심 설계 결정이다. 배포되는 `soul.md`와 `memory/`는 **의도적으로 비어 있는
골격**이다.

> "이 파일은 의도적으로 비어 있는 골격이다. 시스템을 사용하면서 **당신의** 우선순위·취향·
> 금지선을 직접 채워라. 절대지침(directives/)이 '어떻게 일하는가'라면, soul은 '누구를 위해
> 왜 일하는가'다." — `cysjavis-pack/soul.md`
>
> "장기기억은 빌리는 것이 아니라 사용하며 축적하는 것이다." — `cysjavis-pack/memory/MEMORY.md`

작동 방식(디렉티브)과 능력(도구·스킬)은 완비해서 배포하되, 가치관과 기억은 소유자의 것으로
남긴다. 팩 설치기는 이 원칙을 코드로 강제한다 — soul.md·디렉티브·CLAUDE.md·schedule.json은
**사용자 수정 시 영구 보존**되고, 업데이트가 이를 덮어쓰지 않는다 (`src/pack.rs`의
`is_user_owned` / 사용자-수정 불가침 설치 로직).

---

## 3. 설계 철학 — 10개 명제

문서·디렉티브·코드를 관통하는 원칙들이다. 각 명제는 배포되는 팩 원문에서 인용했다.

### ① 산문 계약을 코드 불변식으로 격상한다
자연어 지침("서버를 정리해라", "완료 전에 검증해라")은 언젠가 무시된다. 그래서 반복되는
운영 의무는 **결정론 도구·스키마·게이트**로 내려앉힌다.
> "결정론으로 환원 가능한 작업은 LLM 자연어 추론으로 다시 풀지 마라. …
> 도구 출력과 너의 기억이 충돌하면 **항상 도구 출력이 이긴다**." — `directives/MASTER_DIRECTIVE.md`

부트 검증은 `javis_preflight.py`의 exit code가 사실이고, 진행률은 `javis_report.py`의
체크박스 산술이 사실이며, 이벤트는 `javis_event.py`의 닫힌 enum만 통과한다.

### ② producer ≠ evaluator — 자기채점 금지
산출물을 만든 주체가 그 품질을 채점하지 않는다.
> "후보 생산자가 자기 점수 산출 금지. 채점 = locked-eval launcher." — `directives/RSI_LEARNING_DIRECTIVE.md`

리뷰어 판정 스키마(`schemas/verdict_schema.json`)에는 **score/grade/rating 필드 자체가
없다** — 점수를 매길 수 없게 스키마 층에서 구조적으로 금지했다(다수결·평균·reward-hack 차단).
판정은 `ACCEPT | REVISE | BLOCK | ESCALATE` 닫힌 enum + `evidence(file:line)` 필수다.

### ③ 환각 0 — 검색 우선, garbage-in 차단
> "토대가 오염되면 아무리 다듬어도 거짓만 정교해진다." — `directives/MASTER_DIRECTIVE.md`
> "할루시네이션 자료로 학습하면 시스템 전체가 붕괴한다. … 부분 통과 = 전체 중단." — `directives/RSI_LEARNING_DIRECTIVE.md`

학습 루프(`javis_learn.py`)는 citation 없는 입력을 hard fail 시키고, 자기개선 봉쇄
게이트(`bin/rsi-gate.sh`)는 fail-closed로 동작한다(의심스러우면 차단).

### ④ 적대적 검증이 합의(다수결)에 우선한다
> "칭찬만 하는 리뷰는 리뷰가 아니다 — 결함을 찾는 것이 너의 직무다." — `directives/REVIEWER_DIRECTIVE.md`

리뷰어끼리 판정이 갈리면 다수결이 아니라 **독립 재유도**로 결착한다. 리뷰어를 서로 다른
모델 계열로 두는 이유도 명문화되어 있다 — 같은 모델이면 "사각지대(blind spot)가 상관되어
같은 실수를 함께 놓칠 수 있다"(`REVIEWER_DIRECTIVE.md`).

### ⑤ 롤백 우선 — baseline을 못 이기면 되돌린다
모든 자기개선·팩 변경은 되돌릴 수 있어야 한다. RSI 라운드는 checkpoint를 먼저 만들고
(`javis_rsi.py checkpoint`), baseline을 못 이기면 rollback 한다 — 이때도 콘텐츠 삭제로
점수를 올리는 reward-hack을 막기 위해 retention 게이트(비가역 삭제 차단)가 걸린다.
팩 설치는 저널 트랜잭션이라 중단되면 부팅 시 자동 롤백된다 (`src/pack.rs`).

### ⑥ fail-closed와 fail-open의 의도적 비대칭
모든 게이트가 같은 방향으로 실패하지 않는다. **보안·서명·능력 게이트는 fail-closed**
(팩 서명 검증 `src/packsig.rs`, capability 게이트, RSI 봉쇄), **관측·텔레메트리 훅은
fail-open**(항상 exit 0 — 관측이 에이전트를 깨뜨리면 안 된다, `hooks/cys-hook.sh`),
**통신 정책(ACL) 부재는 fail-open**(설정 파일이 없다고 함대가 벙어리가 되면 안 된다).
어느 쪽으로 실패할지는 각 게이트의 목적에 따라 선택된 설계 결정이며 코드 주석에 명시된다.

### ⑦ 로컬 우선 — 데이터는 머신을 떠나지 않는다
관제 데이터(사용량·비용·세션·전사)는 전부 로컬 SQLite(`analytics.db`·`transcripts.db`)에
쌓이고, 데몬은 **네트워크 리스너가 없다**(사용자 소유 Unix 소켓 / DACL 봉인 named pipe만).
외부 대시보드·클라우드 의존 0. PII는 `CYS_CONTROL_REDACT=1`로 가린 채 집계만 보존할 수 있다.

### ⑧ 판단과 구현의 분리 — 빠른 사고 / 느린 사고
master는 판단(분해·브리프·검증·승인)에 집중하고 구현 노동은 워커에게 위임한다. 간단한
것은 직접(빠른 사고), 대부분은 위임 + 철저한 관리감독(느린 사고). 그리고 경계:
> "Worker의 완료 보고를 그대로 믿지 마라 — diff와 테스트로 직접 확인한 뒤 승인한다."
> — `directives/MASTER_DIRECTIVE.md`

### ⑨ 실측만 완료로 인정한다 — started ≠ completed
> "도구가 `completed`를 반환하지 않은 작업을 완료라고 보고하지 않는다. `started`는 완료가
> 아니다." — `directives/WORKER_DIRECTIVE.md`
> "'될 것이다'가 아니라 '확인했다'로 보고한다." — 같은 문서

도구 반환어휘도 닫힌 8종 집합(`round/TOOL_RESULT_VOCAB.md`)으로 계약되어 있다.

### ⑩ 자율은 넓게, 정지는 denylist로 — 그리고 kill-switch
승인된 로드맵 안에서는 멈추지 않고 달린다. 멈추는 곳은 **금지선(denylist)뿐**이다:
로드맵 이탈 새 범위·헌장(soul/디렉티브) 변경·외부 발행(git push 등 비가역)·비가역 삭제·
오너 명시 보유 결정권.
> "로드맵 안·가역이면 달리고, 로드맵 밖·비가역이면 주차한다." — `directives/MASTER_DIRECTIVE.md`
> "오너의 어떤 입력이든 자율주행을 즉시 일시정지시킨다 — 오너가 항상 우선이다." — `soul.md`

이 자율주행 권한은 오너가 soul.md에 명시적으로 부여할 때만 발생하며("이 절이 없으면
master는 자율주행하지 않는다"), 자율화되는 것은 '전환을 누가 누르냐'뿐 — **품질 게이트의
엄격성은 불변**이다. 집행은 산문이 아니라 PreToolUse 훅(`hooks/guard.sh`, deny-by-default
allowlist·fail-closed)과 데몬 kill-switch(`cys pause` — 큐 배달·스케줄 발화 동결)가 맡는다.

---

## 4. 역할 체계 — 에이전트를 회사로 묶는 법

CYSJavis 팩은 에이전트를 다섯 역할로 조직한다. 각 역할은 기동 시 절대지침
(`directives/*.md`)을 자동 주입받아 "각성"한다 — 지침 없는 노드는 단순 단말로 수렴한다는
것이 반복 관찰된 실패 양식이기 때문이다.

| 역할 | 지침 | 하는 일 |
|---|---|---|
| **master** | MASTER_DIRECTIVE | 단일 지휘 노드. 분해·위임·검증·승인·오너 보고. 구현 노동 금지 |
| **worker** | WORKER_DIRECTIVE | 구현 전부. "창의적·능동적 직원이지 수동 단말이 아니다" |
| **CSO** | CSO_DIRECTIVE | 시스템 운영(자원·프로세스·컨텍스트 수명주기) 총괄·무한책임 |
| **reviewer** | REVIEWER_DIRECTIVE | 외부 검증·반박 전담(수정 권한 없음 — 훅이 물리적으로 차단) |
| **CEO** | CEO_TEMPLATE | master of master. 부서(독립 데몬)가 여럿일 때 승격 |

**라운드 루프** — 중요 산출물은 "작업 → 리뷰어 [문제·논쟁·조언] → 반박(vindication) →
재반박 → 수용·수정"의 라운드를 목표 품질 도달 또는 상한(10라운드)까지 반복한다. 상한
도달 시 무한 루프 대신 오너에게 격차를 보고한다(escalation).

역할은 이름뿐이 아니라 기계로 강제된다:
- 데몬은 **커널 peer pid**로 발신자를 검증하고(자기신고 role 불신), `acl.json`의
  role→role 정책으로 stdin 주입을 게이트한다 — 예: 리뷰어는 워커를 직접 조향할 수 없다
  (중재는 master 경유).
- 능력 모델(`src/bin/cysd/caps.rs`)은 deny-by-default — reviewer/planner는 읽기·검색만.
  에이전트 내부 도구는 PreToolUse 훅(`hooks/role-capability-gate.sh`)이 실 집행자다
  (리뷰어가 검토 대상을 직접 고쳐버리는 reward-hack 차단).
- master 특권 탈취(라이브 master role claim), 워커 무한 증식(active-limit), 승인 자기결재
  (`feed.push`한 노드가 스스로 `feed.reply`)는 데몬이 거부한다.

---

## 5. 시스템 아키텍처

### 5.1 컴포넌트 지도

```
┌─────────────────────────────── 한 저장소, 한 배포 ───────────────────────────────┐
│                                                                                │
│  cys.app   Tauri 2 데스크톱 앱 — 터미널 UI(xterm.js) + Control Center.          │
│            데몬의 thin client. PTY를 소유하지 않으므로 앱이 죽어도 세션 생존.       │
│                                                                                │
│  cysd      헤드리스 코어 데몬 — NDJSON 소켓 서버(UDS / named pipe), PTY 소유      │
│            (portable-pty: openpty·ConPTY), vt100 화면 재구성, 이벤트 버스,        │
│            watchdog·프로세스 원장, 사용량/비용 수집, SQLite 영속 분석, 스케줄러     │
│                                                                                │
│  cys       CLI — pane 안의 AI가 쓰는 동등 노드 클라이언트 (60+ 서브커맨드)          │
│                                                                                │
│  pack      cysjavis-pack/ — 절대지침 10·결정론 도구 86·훅 31·스킬 114·스키마 4.   │
│            빌드 시 바이너리에 임베드, 배포 시 minisign 서명                        │
└────────────────────────────────────────────────────────────────────────────────┘
```

모든 pane 프로세스에 `CYS_SURFACE_ID`·`CYS_SURFACE_REF`·`CYS_SOCKET`이 자동 주입된다 —
pane 안의 AI는 `cys identify` 한 번으로 자기 주소를 알고, 그 순간부터 소켓의 **동등
노드**다. "사람만 조종석에 앉는 터미널"과의 근본적 차이가 여기다.

### 5.2 cysd 데몬 내부

| 모듈 (`src/bin/cysd/`) | 역할 |
|---|---|
| `main.rs` | 진입점·accept loop·연결 핸들러·프레이밍·startup lock·자동 복원 |
| `handlers.rs` | RPC 디스패치(60+ 메서드)·발신자 신원 해소·ACL·능력 게이트 |
| `state.rs` | Surface(PTY) 수명주기·scrollback·역할 레지스트리·토폴로지·묘비·큐·헬스룰 |
| `governance.rs` | watchdog(5초)·고아 회수·중복 프로세스·에이전트 사망 감지·큐 배달 |
| `events.rs` | 이벤트 버스 — 단조 seq 링 + broadcast + 재시작 간 단조성 예약 |
| `usage.rs` / `cost.rs` | 사용량 수동 관측(트랜스크립트·쿼터)·모델 단가표 |
| `analytics.rs` | analytics.db(사용·이벤트·세션·변경 해시체인) |
| `recall.rs` | transcripts.db(FTS 전문검색 + attest 해시체인 + 보존정책) |
| `channels.rs` | Slack·Discord 브리지(원격 승인·수신·발신·lockdown) |
| `schedule.rs` | 30초 tick 스케줄러(원샷·반복·missed-fire) |
| `approval.rs` | HMAC signed-prefix 승인 |
| `deadman.rs` | hung 데몬 홀더 감지·안전 회수 |
| `hwmon.rs` | CPU 코어별·GPU·NPU·MEM 실시간 스냅샷 |

굵직한 견고성 장치: vt100 파서 패닉은 `catch_unwind`로 격리되어 그 청크만 버리고 리더는
불사(`state.rs`), watchdog tick·백그라운드 writer도 각각 패닉 격리, 응답은 8MB 상한 +
round-trip 자기검증 프레이밍(와이어 무결성 이중 가드), 데몬 중복 기동은 소켓 연결 확인 +
flock으로 거부.

### 5.3 프로토콜 — NDJSON, 한 줄 = JSON 하나

```
요청  {"id":1,"method":"surface.send_text","params":{"surface_id":"surface:2","text":"..."}}
응답  {"id":1,"ok":true,"result":{...}} | {"id":1,"ok":false,"error":{"code","message"}}
```

RPC는 60여 개 + `channel.*` 13종, 이벤트는 90여 종이 흐른다(전수 목록은
[User Manual §17](USER-MANUAL.md)). 서버→클라이언트 방향은 `events.stream` 푸시
스트림이며 시퀀스 번호로 재접속 이어받기가 된다 — 구독을 replay보다 먼저 등록해 갭을
막고, 밀리면(Lagged) 연결을 끊어 재접속 replay를 강제한다.

### 5.4 팩 실행 계층 — 지침을 집행하는 도구들

팩의 `bin/`에는 **결정론 도구 86종**이 있다(표준 라이브러리·네트워크 0·LLM 호출 0이
기본). 오케스트레이션 한 사이클이 도구로 어떻게 이어지는지:

```
부트      javis_preflight --fix (존재·매핑·훅 검증 — exit code만이 사실)
          → javis_orchestra check (필수 노드 생존 판정)
라우팅    javis_route --request "…" (fast/deliberate/slow 3단 판정)
위임      javis_task checkout (원자적 체크아웃 — 충돌=exit 9, 선행 미해소=exit 4)
착수 게이트 javis_resource_gate check (서버·노드·load·컨텍스트 사전 차단)
실행 중   javis_event emit (닫힌 enum 이벤트) · javis_wakeup (코얼레싱 웨이크업 큐)
진행 보고  javis_report (todo 체크박스 산술 → 주기 push)
검증      javis_manifest check-criteria (기계 FLOOR) + javis_verdict (리뷰어 판정 스키마)
자율 전진  javis_orchestra gate-status (수렴 판정) → next-action (다음 액션 큐)
종료 게이트 javis_memory add (장기기억 증류 — 색인과 원자적 동기) · javis_adr add (결정 기록)
복원      javis_state_snapshot (세대 보관) · javis_phoenix (부활 저널·정직 상태 enum)
```

훅(`hooks/`, 31종)은 두 계급으로 명시 분리된다 — **OBSERVABILITY**(절대 차단 금지·항상
exit 0: 사용량 관측, statusline)와 **GATE**(deny-by-default·차단이 목적: 자율주행 guard,
역할 능력 게이트, 기획 선행 게이트). 보안 스캐너(`javis_skillscan.py` — 46 규칙, 스킬
정적 스캔 + 복원 주입 경로의 메모리 포이즌 스캔)와 MCP 거버넌스 게이트(`javis_mcpgate.py`
— tool-poisoning·rug-pull 감지)가 공급망 방향을 지킨다.

스킬은 114종(`skills/`)이 실리며, 외부 유래 스킬은 커밋 핀 + 파일별 sha256 매니페스트
(`skills/_VENDOR_MANIFEST.json`)로 잠그고 `skills/THIRD_PARTY.md`에 귀속을 남긴다.

### 5.5 UI는 thin client다

앱(`src-tauri/` + `ui/`)은 데몬 소켓의 얇은 클라이언트다. 프런트엔드는 프레임워크 없는
순수 TypeScript + xterm.js(의존성 3개)이고, 빌드 산출물이 바이너리에 임베드된다. PTY는
데몬 소유이므로 **앱을 재시작·재설치해도 세션은 살아 있고 재-attach만 한다**. UI가 hang
이어도 소켓 제어 채널은 살아 있다(out-of-band 회생).

### 5.6 업데이트 아키텍처 — 이중 채널 + 스큐 교대

| 채널 | 서명 | 방식 |
|---|---|---|
| **앱(바이너리)** | Tauri updater 서명 | 시작 + 6시간마다 확인 → `!` 배지 → 세션 가드 → 설치·재시작 → 복원 |
| **팩(운영체계)** | minisign (공개키 바이너리 핀) | **무중단** — 서명 검증 → 저널 트랜잭션 반영 → 라이브 노드 재주입. 재시작 0, `↻` 배지 |

팩 검증 사슬은 전건 fail-closed다: 필수 필드 → 채널 → 키링(폐기·미지·만료 거부) →
minisign → 다이제스트 → 신선도 창 → **replay 단조성**(이미 수락한 것보다 오래된 팩 거부).
tar 전개는 검증 후에만, in-Rust 하드닝 전개기(심링크·절대경로·경로 이탈 거부)로 수행된다
(`src/packsig.rs`, `src/pack.rs`).

플랫폼 재설치(rename-swap) 후 "디스크는 새 버전, 프로세스는 구 데몬"인 스큐가 남으면 UI가
"데몬 vX · 앱 vY — 세션 보존 중" 배지를 띄우고, 클릭 교대 또는 **유휴 자동 교대**(라이브
세션 0일 때만 — 무손실)로 데몬을 갈아끼운다(`rotate_daemon`).

### 5.7 채널 계층 — 함대를 메신저로 연결

`channels.rs`는 Slack·Discord 브리지를 제공한다: 승인 요청·보고를 외부 메신저로 내보내고,
허가된 발신자의 원격 승인을 받아들인다. 신뢰 방향은 보수적이다 — 발신자 allowlist·원격
승인 별도 허가(`allow-remote-approve`)·잠금(`lockdown`)·모양 기반 redact(토큰·홈경로
차단)·중복/루프 억제가 내장된다.

### 5.8 free / pro 채널

팩은 free(내장)와 pro(서명 라이선스로 활성화되는 오버레이) 두 채널을 가진다. 라이선스는
verify-only(서명 검증만, 발급 능력 없음 — `src/license.rs`)이고, 채널 상태가 손상되면
pro 콘텐츠를 내장 free 팩으로 덮지 않도록 보호된다(강등은 명시적 명령
`pack-downgrade-to-free`로만).

---

## 6. 보안 아키텍처

**위협 모델의 전제**: 데몬은 네트워크 리스너가 없고, 같은 OS 사용자 계정 안의
로컬 프로세스들이 클라이언트다(단일-UID 신뢰 노드 모델). 그 안에서:

1. **발신자 신원 = 커널이 말한다** — 연결마다 peer pid(macOS `LOCAL_PEERPID`, Linux
   `SO_PEERCRED`, Windows `GetNamedPipeClientProcessId`)를 조회하고 조상 프로세스 체인으로
   소속 surface를 해소한다. 자기신고 role은 신뢰하지 않는다. pid 재사용은 start_time
   대조로 무효화한다.
2. **통신 정책** — `acl.json`의 role→role 규칙으로 stdin 주입을 게이트한다(예: 리뷰어→워커
   deny). 타이핑 가드는 사람 입력 직후의 기계 주입 충돌을 막는다.
3. **능력 게이트** — deny-by-default. 알 수 없는 역할은 능력 0.
4. **승인은 사람 또는 서명** — 화면의 승인 프롬프트에 자동 응답하는 코드는 없다(HITL).
   반복 위험 명령은 master가 HMAC-SHA256 signed-prefix로 1회 서명하면 guard 훅이 통과시킨다
   (`cys approval sign` — master surface 전용, 상수시간 비교, 시크릿 0600 파일).
5. **자기결재 차단** — 승인 요청을 올린 노드가 스스로 승인할 수 없다(pid/pgid/surface 각인).
6. **공급망** — 앱은 Tauri updater 서명, 팩은 minisign 핀. 발행 전 비밀/PII 게이트
   (`scripts/secret-scan.sh --all`, fail-closed)와 팩 전용 스캔이 CI 최우선 단계로 돈다.
7. **PII** — `CYS_CONTROL_REDACT=1`이면 세션 식별자를 해시로 가리고 집계만 보존.

**정직한 한계의 명문화**도 설계의 일부다: 단일-UID 모델에서 승인 서명·자기결재 정책은
"암호학적 보증"이 아니라 탐지·fail-safe 층임을 코드 주석이 스스로 밝힌다. 비밀 스캐너는
정적 패턴 매칭이라 난독화·신종 토큰을 못 잡는다고 명시한다. 한계를 숨기지 않는 것이
이 저장소의 문서 규약이다(README "알려진 한계" 섹션 상설).

---

## 7. 영속성과 부활

- **세션 영속** — PTY는 데몬 소유. UI 재시작·앱 재설치·업데이트에도 세션 유지.
- **이벤트 연속성** — seq 단조 + 재시작 간 예약 블록으로, 재접속 클라이언트가 이어받는다.
- **기록 영속** — 3개의 로컬 SQLite(analytics / transcripts+FTS / channels), 전부 WAL,
  열기 실패 시 기능 저하로 우아하게 계속(관측이 본체를 죽이지 않는다).
- **변조 증거성** — 전사 기록은 해시체인으로 이어지고, `cys attest pin/verify`로 외부
  보관·사후 대조가 된다(producer≠evaluator의 기록 버전).
- **부활(phoenix)** — 상태 스냅샷은 세대 보관되고(`javis_state_snapshot`), 복원은 부활
  저널 상태머신(`javis_phoenix`)이 수행한다. 복원 상태는 VERIFIED/UNVERIFIED/FAILED 정직
  enum으로 보고된다 — "무출력=성공" 해석은 금지되어 있다. 크래시 루프에는 회로차단기가
  걸린다. 복원 시 재주입되는 텍스트는 메모리 포이즌 스캔을 거친다(`hooks/inject_gate.py`).
- **역할 조직 복원** — `cys restore`가 토폴로지 스냅샷의 죽은 역할들을 일괄 재기동·지침
  재주입한다. 묘비(tombstone) 규약이 "사용자가 의도적으로 닫은 것"과 "죽어서 부활해야
  하는 것"을 구분한다.

---

## 8. 자기개선 루프 (RSI)

시스템은 자기 운영 경험을 자산으로 바꾸는 루프를 내장한다. 다만 **측정 무결성이 루프의
전부**라는 전제 위에서다(§3-②③⑤).

1. **수집** — 훅이 반복 교정 신호를 감지해 후보로 적재한다(자동 적용 0, 후보일 뿐).
2. **학습 5단계** — 제안 → 검색(citation 강제) → 추출(관찰 가능한 행동 주장으로 변환) →
   평가(외부 채점·baseline 대비 실측) → 저장/하네스화 (`javis_learn.py`).
3. **게이트** — `rsi-gate.sh`(fail-closed) + 디렉티브 회귀 트립와이어
   (`javis_directive_bench.py` — 지침이 결함 행동을 실제로 금지하는지 결정론 채점).
4. **가시화** — Control Center "학습" 탭에서 라운드 타임라인·채택/롤백·발견 누적을 본다.
5. **영속** — 통과한 것만 스킬(`cys skill`)·장기기억(`javis_memory`)·결정기록(`javis_adr`)
   으로 증류된다. "넘어진 사람이 팻말을 세운다" — 실패 경험이 스킬 주의칸으로 쌓인다.

---

## 9. 개발 방법론 — 이 저장소가 만들어진 방식

이 저장소 자체가 위 철학의 산물이다.

- **AI 함대가 작성, 사람이 지휘** — 커밋의 `Co-Authored-By` 체인이 기록.
- **클린룸 흡수** — 외부 오픈소스에서 배울 때 코드를 복사하지 않고 **규칙·패턴을 표준
  라이브러리로 재구현**한다. 설계 참고는 `NOTICE.md`에, 벤더링(실제 코드 반입)은 커밋 핀 +
  파일별 해시 매니페스트로 잠그고 귀속을 남긴다(`skills/THIRD_PARTY.md`,
  `javis_cleanroom.py`가 헤더·해시핀·카피레프트 게이트를 기계 검증).
- **롤백 우선** — 기능은 격리 폴더에서 시작하고, 코드보다 복원 수단을 먼저 만든다.
- **적대 리뷰 라운드** — 중요 변경은 서로 다른 모델 계열 리뷰어의 반박 라운드를 거친다.
- **발행 게이트** — 비밀/PII 스캔(fail-closed) → 버전 SOT 검사 → 테스트 → 서명 → 실서명
  실검증 폐포 게이트, 전부 CI가 강제한다(`.github/workflows/release.yml`).

---

## 10. 설계 불변식 (요약)

1. 데몬·PTY 리더·watchdog은 패닉에 죽지 않는다(격리·복구).
2. 보안·서명·능력은 fail-closed, 관측은 fail-open, 통신 정책 부재는 fail-open.
3. PTY·피드 원장·이벤트 seq는 단일 writer.
4. 데몬 중복 기동은 거부된다.
5. 의도적 닫힘(묘비)은 부활하지 않고, 죽음은 부활 대상이다.
6. kill-switch(`pause`)는 큐·스케줄을 동결하되 직접 send는 통과한다 — "신경 차단"이지
   행동 정지가 아니며, 재부팅에도 유지된다.
7. 와이어 응답은 상한과 자기검증 프레이밍의 이중 가드를 거친다.
8. 사용자 소유 파일(soul·디렉티브·CLAUDE.md·schedule)은 업데이트가 덮지 않는다.
9. 페르소나 커스터마이즈는 허용되지만 안전핵(denylist·복구·kill-switch)은 잠겨 있다.

---

## 11. 더 읽기

- 설치·운용·전체 레퍼런스: [User Manual](USER-MANUAL.md)
- 설치 상세: [INSTALL.md](docs/INSTALL.md) · [INSTALL-Windows-KR.md](docs/INSTALL-Windows-KR.md)
- 무중단 팩 업데이트 설계 정본: [DESIGN-noshutdown-pack-update.md](docs/DESIGN-noshutdown-pack-update.md)
- Control Center 설계: [CONTROL_CENTER_DESIGN.md](docs/CONTROL_CENTER_DESIGN.md)
- 보안 신고: [SECURITY.md](SECURITY.md) · 기여: [CONTRIBUTING.md](CONTRIBUTING.md)
