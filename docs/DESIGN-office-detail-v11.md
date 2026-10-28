# 오피스 3D 직원 상태 디테일 12종 — 월드 계약 v1.1 (2026-07-12 오너 승인)

> 원천 지시: "직원들이 일하는 상태를 더 디테일하게 — 제안 12개 모두 구현."
> 이 문서가 백엔드(javis_hud_bridge.py)와 프론트엔드(office3d.html) 간 **단일 인터페이스 계약**이다.
> 계약 버전: `"v": 1` 유지(전 필드 additive — 구 프론트가 새 필드를 무시해도 동작 불변).

## 1. 기능 → 계층 매핑

| # | 기능 | 데이터 | 백엔드 | 프론트엔드 |
|---|---|---|---|---|
| 1 | ctx 게이지 상시 | `node.ctx.pct` (기존) | — | 책상 옆 세로 바(<60 청록·60~89 노랑·≥90 빨강 점멸) |
| 2 | 활동 강도 | `node.activity` (기존) | — | 모니터 코드 스크롤 속도·책상 서류 더미 3단 |
| 3 | rate LED | `node.rate[]` (기존) | — | 모니터 상단 사용률 바(5h 우선) |
| 4 | task 전문 | `node.task` (기존 80자) | — | hover/선택 시 풀텍스트 말풍선 |
| 5 | 방치 시간 | `node.idle_secs`·`presence` (기존) | — | drowsy/sleeping 위 "N분째" |
| 6 | 진행률 링 | `node.progress` (신규) | EVT spool `task_progress` | 머리 위 링(pct)+stage 텍스트 |
| 7 | 작업 카드 | `node.run` (신규) + fx `runcard` | EVT spool `run.*` | 인박스 트레이 카드 스택·실패 빨간 카드 |
| 8 | 의존 화살표 | `world.blocked[]` (신규) + fx `blocked/unblocked` | EVT spool `task.blocked/unblocked` | 노드 간 점선(양측 귀속 시)·부서 보드 표기 |
| 9 | 칸반 벽면 | `world.kanban` (신규) | `$JAVIS_ROOT/_round/tasks/*.json` 스캔 | 부서 층 벽 3열 보드 |
| 10 | 리뷰 라운드 | `world.review` (신규) + fx `verdict` | `$JAVIS_ROOT/_round/*VERDICT*.md` 워치 | 회의 테이블 집결+판정색 |
| 11 | 배달 모션 | fx `doc` (기존) | — | from→to 보행 배달(보행 오버라이드 재사용) |
| 12 | 전광판 | `world.board` (신규) | presence 히트 누적+비용 best-effort | 로비 전광판(24h 히트맵·오늘 비용) |

## 2. 스키마 확장 (전부 additive)

### 노드 뷰 추가 필드
```json
"progress": {"task":"…","stage":"…","pct":42,"detail":"…","ts":1720000000} | null,
"run": {"queued":2,"active":{"task":"…","started":1720000000}|null,"done_today":5,"failed_today":1}
```

### 월드 스냅샷 추가 필드
```json
"blocked": [{"task":"…","blocked_by":["…"],"key":"surface:12"|null,"ts":…}],
"kanban": {"ts":…,"tasks":[{"id":"…","title":"…","status":"todo|doing|done|blocked","owner":"…"|null,"blocked_by":["…"]}]},
"review": {"ts":…,"items":[{"reviewer":"agy|codex|…","verdict":"ACCEPT|REVISE|BLOCK|ESCALATE","target":"…","ts":…}]},
"board": {"heat":{"<key>":[24개 0..1]},"cost_today":{"usd":1.23|null,"tokens":123456|null}}
```
- `board.heat[key][h]` = 최근 24시간 h시(로컬) 버킷의 active 비율. 브리지 재시작 생존(STATE_DIR 영속).
- 상단 필드 갱신은 `{"t":"patch_top","field":"kanban|review|board|blocked","value":…}` 프레임으로 push. 프론트는 미지 필드·프레임 무시(하위호환).

### 신규 fx 프레임
```json
{"t":"fx","kind":"progress","key":…,"task":…,"stage":…,"pct":…}
{"t":"fx","kind":"runcard","key":…,"phase":"queued|started|succeeded|failed","task":…,"summary":…}
{"t":"fx","kind":"blocked","task":…,"blocked_by":[…],"key":…|null}
{"t":"fx","kind":"unblocked","task":…,"key":…|null}
{"t":"fx","kind":"verdict","reviewer":…,"verdict":…,"target":…}
```
전 fx는 기존 Coalescer 예산·백로그 게이트(BACKLOG_FX_SECS) 대상.

## 3. EVT spool (신규 수송로)

- 경로: `<pack>/state/evt_spool.jsonl` — ★(0.14.56 X-8 CARRIER) 경로의 정본은 leaf `bin/javis_spool_path.py` 의 `spool_path_for_pack(pack_dir)` 하나다(방출자·브리지 공용 · 환경변수 무관 · `HUD_STATE_DIR` 은 브리지 사설 상태 폴더만 뜻한다). 브리지는 레인마다(본부 = 자기 팩, 부서 = `cys fleet --json` 의 `pack_dir`) 그 레인의 spool 을 읽고, 레인별 오프셋 `<HUD_STATE_DIR>/evt_spool-<레인키>.offset`, 로테이션은 같은 폴더의 `evt_spool.<UTC>.jsonl` 로 **이름만 바꾸고 지우지 않는다**(보관본 전량 유지). 상태는 `/health` 의 `spool` 항목(`spool_schema` 1)에 레인별로 나오고, 각 데몬의 spool watch 가 자기 레인 항목을 읽어 자기 master 에게 알린다. 한 줄 = `{"ts":…,"type":"<EVT enum>","payload":{…},"key":"surface:N"(선택)}`.
- 방출: `javis_event.py emit … --spool` 이 wire 출력에 **더해** spool에 append(O_APPEND 원자 줄쓰기). `--surface <ref>` 로 key 귀속(선택).
- 소비: 브리지가 1s 폴링 tail(오프셋 STATE_DIR 영속·truncate 감지 시 0부터). 노드 귀속: `key` 명시 > `payload.agent`==role 유일 일치 > 미귀속(부서 보드로만).
- 이유: EVT wire는 surface stdin 텍스트로 흘러 데몬 이벤트 스트림에 안 잡힘 — 데몬 무변경 원칙상 파일 spool이 최소 결합.

## 4. 불변 제약

- 데몬(cysd)·CC(ui/src) 무변경. `"v":1` 유지. 기존 필드 의미 불변(`feedback_shared_field_semantic_contract`).
- office3d: refitCam·fit 카메라 로직 수정 금지(56933bd Directive). embed 모드·타임라인 리플레이·기존 presence 애니 유지. sprite 텍스트 갱신 ≥1s 스로틀, per-frame 할당 금지.
- 브리지: 순수 로직은 함수 분리 + stdlib unittest(`cysjavis-pack/tests/`). transcripts.db는 read-only(URI mode=ro)·실패 시 null(비용은 best-effort).
