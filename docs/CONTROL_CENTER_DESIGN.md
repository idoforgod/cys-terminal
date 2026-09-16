# Control Center 극대화 설계안

> 작성 2026-06-17. 근거: 팀 단위 관측·분석 도구 1·2차 전수조사.
> 목표: 현 T6 Control Center(실시간 관제)를
> **실시간 관제 + 영속 분석 + 효율 최적화 + 자동 거버넌스** 단일 네이티브 플랫폼으로 격상.
> 철학: 로컬 우선(데이터 머신 밖으로 안 나감)·추가 인프라 0(cysd 내장 SQLite)·에이전트 0ms 지연.

---

## 0. 현재 상태 vs 목표

**현재(T5·T6)**: cysd가 claude `.jsonl`·codex rollout tail + agy RPC → `control.dashboard` RPC →
Tauri 풀 패널(플릿·rate 5h/7d·CPU/MEM·소비·12h 스파크라인·가동시간). **휘발성**(in-memory·재시작 소실),
**양(volume)만** 측정. 툴/스킬/에이전트 호출·비용·세션 전사·추세 영속 없음.

**목표**: 팀 단위 관측 도구가 준 것을 **로컬 멀티-노드 플릿**(master·CSO·worker·reviewer)에 적용 —
"누가/무엇을 얼마나 효율적으로" + "어디서 막히나" + "곧 소진/이상" 자동 경보 + "성공 세션 재현".

---

## 1. 아키텍처 — 분석 척추(spine)

```
[hooks 확장]                       [수집/저장 — cysd]                 [노출]                  [UI — cys-app]
SessionStart ─┐                  ┌─ usage.rs collector (tail)        control.dashboard ──┐   Control Center
PreToolUse  ──┤  cys hook ──push─┤  ingest.rs (event 적재)            control.analytics ─┼─→ 탭: Live / 비용·효율
PostToolUse ──┤  (즉시 exit0,    ├─ cost.rs (모델단가→$)              control.sessions ──┤      / 스킬·에이전트
Stop        ──┤   detached)      ├─ rollup.rs (일/주 롤업·lazy캐시)   control.skills ────┤      / 세션 / 추세·주간
SubagentStop ─┘                  └─ SQLite analytics.db (영속)        control.alerts ────┘   + 경보 배지·master push
                                       ↑ rusqlite (이미 보유)
[5분 스케줄러] → 주간 다이제스트 push   [context.threshold 에지게이트] → 경보 push
```

**핵심 enabler 3가지**
1. **SQLite 영속**(`analytics.db`) — cysd엔 이미 rusqlite(transcripts.db) → 추가 의존성 0. 휘발성 해소.
2. **이벤트 캡처 hook 확장** — 현 SessionStart(transcript 등록)·statusline(rate) 위에 PreToolUse/PostToolUse/
   Stop/SubagentStop 추가 → 툴·스킬·에이전트 호출·exit_code·duration·세션 메타 포착. fire-and-forget(에이전트 0ms).
3. **엔진 3종** — cost(단가표)·rollup(일/주 집계 lazy 캐시)·metrics(효율 파생).

---

## 2. 데이터 모델 (cysd `analytics.db` — SQLite)

```sql
-- 세션 메타 (claude/codex 1세션 = 1행)
CREATE TABLE sessions (
  session_id   TEXT PRIMARY KEY,
  role         TEXT,            -- master|cso|worker|reviewer-* (노드 = 사용자 단위 대응)
  agent        TEXT,            -- claude|codex|gemini
  cwd          TEXT,
  started_at   REAL, ended_at REAL,
  title        TEXT, summary TEXT,    -- transcript summary 라인/첫 HUMAN 200자
  turn_count   INTEGER
);
-- 토큰 사용 (메시지/턴 단위 — 비용 환산 포함)
CREATE TABLE usage_records (
  id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT, model TEXT,
  input_tokens INT, output_tokens INT, cache_read_tokens INT, cache_creation_tokens INT,
  cost_usd REAL, is_subagent INT, ts REAL
);
CREATE INDEX ix_usage_ts ON usage_records(ts);
CREATE INDEX ix_usage_session ON usage_records(session_id);
-- 호출 이벤트 (툴·스킬·에이전트)
CREATE TABLE events (
  id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, agent TEXT,
  event_type TEXT,            -- PRE_TOOL|POST_TOOL|STOP|SUBAGENT_STOP|SLASH
  tool_name TEXT, is_skill INT, skill_name TEXT, is_slash INT,
  is_agent INT, agent_type TEXT, agent_id TEXT,
  exit_code INT, duration_ms INT, ts REAL
);
CREATE INDEX ix_ev_skill ON events(is_skill, ts);
CREATE INDEX ix_ev_agent ON events(is_agent, ts);
-- 메시지 (전사 — 세션 상세/온보딩)
CREATE TABLE messages (
  id INTEGER PRIMARY KEY, session_id TEXT, seq INT,
  role TEXT,                  -- HUMAN|ASSISTANT|TOOL
  content TEXT,               -- 50k 절단
  tool_name TEXT, tool_use_id TEXT, duration_ms INT, ts REAL
);
-- 일별 롤업 캐시 (과거=캐시, 오늘=live)
CREATE TABLE daily_rollups (
  date TEXT PRIMARY KEY,
  session_count INT, turn_count INT, active_roles_json TEXT,
  input_tokens INT, output_tokens INT, cache_read_tokens INT, cache_creation_tokens INT,
  cost_usd REAL,
  skill_counts_json TEXT, agent_counts_json TEXT, model_tokens_json TEXT, role_stats_json TEXT,
  computed_at REAL
);
CREATE TABLE stars (session_id TEXT PRIMARY KEY, note TEXT, starred_at REAL); -- 성공 세션 즐겨찾기
```

설계 원칙: 토큰 4분해·결정론 정렬(`count DESC, name ASC`)·lazy 캐시(과거 DB·오늘 live 30s)·
stale 마커(정의 변경 시 재계산). 보존 정책(기본 60일·`retention` 설정).

---

## 3. 페이즈별 설계 (E1 → E9)

> **진행(2026-06-17)** — E1 ①②③④ 완료(로컬 커밋·미배포):
> - ① 비용 엔진(7bafbee): `cost.rs`(단가표·`normalize_model_name`·`calculate_cost` 4팩터) +
>   `Consumption.today_cost_usd`/`model_tokens` + `control.dashboard` 노출.
> - ② 비용 UI(47ce67c): Control Center 토큰 섹션 `오늘 비용($)`·모델믹스 바.
> - ③ SQLite 영속(d430320·척추): `analytics.rs`(`analytics.db`·`usage_records` 적재·부트 12h
>   리플레이 seed) → 재시작 보존. cargo 206/206 · E2E 7/7(`docs/cost_persist_e2e.py`).
> - ④ 이벤트 캡처(★): hook(PreToolUse/PostToolUse/Stop/SubagentStop)→`cys-hook.sh`→`cys usage-event-stdin`
>   →`usage.event` RPC→`events` 테이블. `derive_tool`(Skill→스킬·Task/Agent→에이전트 파생)·`record_event`·
>   PostToolUse `tool_response.is_error`→exit_code(E3 반복실패 토대). pack 임베드·preflight C33(멱등 등록).
>   cargo 208/208(신규 2종) · E2E 7/7(`docs/event_capture_e2e.py`) · C33 격리 검증(등록·체인보존·멱등).
>
> **E2 비용·효율 탭 완료**(로컬 커밋·미배포):
> - 백엔드: `analytics.rs` `summarize`(순수)·`analytics_summary`·`window_since`(today/7d/all) + `control.analytics` RPC.
>   토큰 4분해 totals·🔥캐시절감$(Σ cache_read×(input−cache_read 단가))·by_model(비용정렬)·by_agent(토큰정렬)·
>   생산성(턴/세션·토큰/턴·비용/세션·평균 세션길이).
> - 프런트: Control Center **Live/비용·효율 탭** 전환 + 윈도우 선택(오늘/7일/전체) + KPI 카드·토큰 스택바·
>   모델별 비용 바·에이전트 믹스·생산성 카드. Tauri `control_analytics` 커맨드.
> - 검증: cargo 209/209(신규 `summarize_costs_and_productivity`) · E2E 17/17(`docs/analytics_e2e.py`) · UI 번들 OK.
>
> **E3 스킬·에이전트 탭 완료**(로컬 커밋·미배포):
> - 백엔드: `analytics.rs` `summarize_skills`(순수)·`skills_summary` + `control.skills` RPC. events 롤업 —
>   호출 TOP(by_skill·by_tool, calls=PRE_TOOL)·🔥실패율(by_tool fail=POST_TOOL exit≠0)·스킬×역할·
>   서브에이전트 위임(by_agent + by_role)·🔥반복실패 TOP(failures, fail desc)·totals(fail_rate).
> - 프런트: Control Center **스킬·에이전트 탭** + 윈도우 선택 + KPI(툴/스킬/위임/🔥실패율)·🔥반복실패·
>   스킬 TOP·툴 TOP·위임 — 실패율 색상 배지(0초록/≥10%경고/≥30%위험). Tauri `control_skills`.
> - **정직 범위**: duration p50·미사용 4주 diff는 현재 미수집(events.duration_ms NULL·4주 축적 필요) — 후속.
>   slash 명령 UNION도 캡처 경로(UserPromptSubmit) 미구현 — Skill 툴 기반만.
> - 검증: cargo 210/210(신규 `summarize_skills_calls_and_failrate`) · E2E 15/15(`docs/skills_e2e.py`) · UI 번들 OK.
>
> **E6 경보·이상감지 완료**(로컬 커밋·미배포):
> - 백엔드: `alerts.rs` `AlertConfig`(pack `alerts-config.json` 핫로드·부분설정·기본값)·`evaluate`(순수)·
>   `snapshot`(rate=observed_usage + 7d 비용/토큰=usage_records + 7d 반복실패=events). 경보 3종:
>   rate_limit(노드 쿼터 ≥90%·≥95% crit)·weekly_budget(비용/토큰 한도, 0=비활성)·repeated_failure(fail수·
>   실패율·최소표본 동시충족·≥50% crit). `governance.rs` watchdog **check_alerts**(30초·에지 디바운스
>   1800s·해소 시 재무장)→`alert.<kind>` "alert" 이벤트 발화. `control.alerts` RPC(동일 평가기=단일 진실원).
> - 프런트: Control Center 헤더 **경보 배지**(개수·crit 점멸) + Live 뷰 상단 경보 스트립. Tauri `control_alerts`.
> - **설계 갱신**: "cys send --to master push"는 governance 교리(★자동응답 금지 — 감지·격상만)에 맞춰
>   **이벤트 발화 + UI 배지**로 정합화(master PTY 주입은 kill-switch 위험·기존 패턴 위배라 회피).
> - **정직 범위**: 노드 토큰 급증 이상감지·스킬 ROI 패턴(commit→재edit)은 per-node 시계열 베이스라인 미보존 → 후속.
> - 검증: cargo 213/213(신규 alerts 3종) · E2E 7/7(`docs/alerts_e2e.py` — 핫로드·예산·반복실패·심각도·재무장) · UI 번들 OK.
>
> **E4 세션 타임라인 탭 완료**(로컬 커밋·미배포):
> - 백엔드: `analytics.rs` `summarize_sessions`(순수·usage_records+events 병합)·`ribbon`(활동 24칸)·`session_list`·
>   `session_detail`(이벤트 타임라인+토큰/비용 요약)·`starred_set`/`set_star`(stars 테이블). 세션별: agent·role·
>   started/ended·duration·턴·토큰·비용·skill_calls(PRE만)·fail_calls(POST exit≠0)·top_skill·활동리본·⭐.
>   `control.sessions`·`control.session_detail`·`control.session_star` RPC.
> - 프런트: Control Center **세션 탭** + 윈도우(오늘/7일/전체)·⭐만 필터 + 세션 리스트(활동 리본 strip·역할 색·
>   ⭐ 토글·턴/토큰/비용/실패) + 클릭 시 상세(이벤트 타임라인 ✓/✗/▸). Tauri 3커맨드.
> - **정직 범위**: 전사 원문(HUMAN/ASSISTANT/TOOL 콘텐츠)·LLM title/summary는 데이터 미수집(messages 테이블
>   미적재·recall은 surface_id 기준 스크롤백) → **이벤트 타임라인으로 대체**, title은 UI에서 메타로 합성. 후속.
> - 검증: cargo 215/215(신규 `summarize_sessions`·`ribbon`) · E2E 16/16(`docs/sessions_e2e.py`) · UI 번들 OK.
>
> **★8커밋 일괄 배포 완료**(2026-06-17 21:4x): /opt/homebrew/bin+/Applications/cys.app 신본 os.replace(.bak-t7)·
> 데몬 단일 재기동(pid 88170 v0.2.2)·pack 동기·preflight C32 PASS+C33 FIXED·앱 ad-hoc 재서명 valid.
> 5 RPC 라이브 실측. ★교훈: release는 strings|grep 부정확(기능검사 권위)·cys.app가 target/release/cysd 재spawn.
>
> **E5 추세·주간 다이제스트 탭 완료**(로컬 커밋·미배포):
> - 백엔드: `analytics.rs` `summarize_weekly`(순수·이번주 vs 지난주)·`weekly_summary` + `control.weekly` RPC.
>   WoW 델타(토큰·비용·세션·메시지, 지난주 0이면 null)·일별 오버레이(각 주 7칸)·효율 리더(역할별 토큰/비용/세션/
>   스킬다양성, session→role 귀속)·스킬자산(신규/휴면/최다). 14일 윈도우 1회 로드.
> - 프런트: Control Center **추세·주간 탭** + WoW KPI(▲▼ 델타)·일별 this/last 오버레이 막대·효율 리더 바·
>   스킬 자산 칩(🆕신규/💤휴면/🔝최다). Tauri `control_weekly`.
> - **정직 분리**: "주간 다이제스트 master push"는 governance 교리(자동응답 금지=PTY 주입 금지)상 이벤트/feed로만
>   가능 → 조회형 탭+RPC 우선 완성, 스케줄러 자동 이벤트 push는 후속(E6 alert 패턴 재사용 가능).
> - 검증: cargo 216/216(신규 `summarize_weekly`) · E2E 11/11(`docs/weekly_e2e.py`) · UI 번들 OK.
>
> **E7 RSI 무결성 도구 완료**(`javis_rsi.py` — Control Center 외 트랙·로컬 커밋):
> - 결정론 도구(eval-driven 원칙 박제·producer≠evaluator=점수 주입만): `checkpoint`(HEAD SHA+기준score+복구 ref
>   `refs/rsi/ckpt/<id>`)·`progress`(delta·verdict improved/regressed/flat)·`markers`(커밋 trailer `iter-id` 파싱)·
>   `rollback`·`status`. _round/rsi/{state.json,ledger.jsonl} 영속.
> - ★rollback 안전모델: **기본 dry-run(무실행·계획만)**, `--execute`만 실행하되 **현재 HEAD를 `rsi-abandoned-<id>-<ts>`
>   브랜치에 먼저 박제(retention=비가역 삭제 차단)** 후 reset · 더티(추적변경)·비조상은 `--force` 없이 거부 · 자동
>   `git reset --hard` 없음. autopilot kill-switch/self-clear 하만 **불간섭**(독립 도구).
> - 검증: E2E 17/17(`docs/rsi_e2e.py` — 순수로직·checkpoint·progress·markers·dry-run 무실행·execute 백업보존·더티거부).
>
> **E9 RBAC(PII 차단) 완료 — 로컬 의미 부분만(로컬 커밋)**:
> - `analytics.rs` `redact_session_id`(sha256→`sess-<8hex>`·안정적·경로 미노출)·`redact_sessions`(집계 보존).
>   `control.sessions {redact}` 파라미터 OR 환경변수 `CYS_CONTROL_REDACT=1` → session_id 경로 PII 가림.
>   UI 세션 탭 **🔒 PII 가림 토글**(켜면 집계만=드릴다운 비활성·"VIEWER=집계만" 충실 해석). Tauri 동형.
> - 검증: cargo 217/217(신규 `redact_session_id_stable_and_pii_free`) · E2E 7/7(`docs/redact_e2e.py`).
> - **★정직 분리(자율 구축 안 함)**: ▶보존(retention 자동삭제)=프로젝트 eval-driven "삭제 reward-hack 차단·
>   retention hard gate" 원칙과 상충→미구현. ▶멀티머신 중앙 cysd·--api-url=네트워크/외부발행 경계+로컬우선
>   철학 정면위배+존재하지 않는 멀티유저 시나리오용 투기 인프라→오너 명시 결정 전까지 미구축.
>
> **▣ T7 Control Center 9페이즈 전부 처리: E1-E6 완결·배포 · E7 RSI 도구 · E8 감사충족 · E9 RBAC(로컬부분).**

### E1 — 영속 분석 기반 (척추) 【선행·필수】
- **백엔드**: `analytics.db` 스키마 생성(state.rs init) · `ingest.rs`(hook 이벤트→events/messages 적재) ·
  Stop 시 transcript 전체 파싱→usage_records/messages bulk(현 usage.rs 파서 재사용·`parse_claude_message_io` 확장) ·
  `cost.rs`(MODEL_PRICING Rust 포트 + `normalize_model_name` + 4토큰 공식).
- **hook**: pack에 `cys hook` 서브커맨드 + PreToolUse/PostToolUse/Stop/SubagentStop을 settings.json에
  멱등 주입(preflight C33). statusline 래퍼와 동일 fire-and-forget(즉시 exit0).
- **검증**: 단위(파서·cost·정규화) + E2E(가짜 hook→DB 적재) + 회귀.
- **산출**: 세션·토큰·호출·메시지·비용이 **영속 저장**(재시작 무관).

### E2 — 비용·효율 탭
- **지표**: 비용$(오늘/세션/모델별)·🔥캐시절감$·모델믹스(claude/codex/gemini)·생산성(턴/세션·토큰/턴·비용/세션·세션 duration)·토큰 4분해 스택.
- **백엔드**: `control.analytics` RPC(rollup 집계). **프런트**: Control Center 신규 탭 "비용·효율"(KPI 카드 + 스택 영역 + 모델 도넛).
- **산출**: "지금 효율적으로 쓰고 있나"를 **금액·캐시·모델**로 정량화.

### E3 — 스킬·에이전트 탭
- **지표**: 호출 TOP(스킬=Skill툴 events ∪ `/slash` UNION)·🔥실패율(`exit_code!=0`)·🔥duration p50·미사용(4주 diff)·스킬×역할(노드) 분포·서브에이전트 위임트리(master→worker→subagent).
- **백엔드**: `control.skills` RPC. **프런트**: TOP 바·실패율 배지·duration 히트맵·위임 트리.
- **산출**: "무엇이 효과적/방치/반복실패"인지 — 실패율 추적을 **선점**.

### E4 — 세션 타임라인·전사 탭
- **기능**: 세션 리스트(필터: 역할·agent·기간·⭐)·자동 title/summary·**활동 리본**(8px 색상 strip)·전사 뷰어(HUMAN/ASSISTANT/TOOL + tool input/output/duration)·⭐즐겨찾기(성공 세션 재현=온보딩).
- **백엔드**: `control.sessions`(목록)·`control.session_detail`(전사) RPC. **프런트**: 바닐라 TS 리스트+상세(가상화 자체구현).
- **산출**: "성공 세션 열람·공유"로 노드/사람 온보딩 가속.

### E5 — 추세·주간 다이제스트 탭
- **지표**: WoW% 델타(↑↓ 색상)·이번주 vs 지난주 오버레이·🔥효율 리더(노드/역할별 토큰·세션·스킬다양성·간결도)·위임 인사이트·스킬자산(신규/최다/휴면).
- **백엔드**: 주간 롤업 + `control.weekly` RPC. **연동**: 기존 5분 스케줄러(`javis_report`) 확장 → **주간 다이제스트 master push**.
- **산출**: 팀(=플릿) 차원 추세·인사이트 자동 보고.

### E6 — 경보·이상감지·ROI
- **기능**: 임계값 경고(주간 토큰/비용 한도·rate 90%)·🔥이상감지(노드 토큰 급증)·🔥반복실패 스킬 경고·스킬 ROI 패턴(예: commit→재edit 반복).
- **백엔드**: `alerts.rs` — **context.threshold와 동일 에지게이트**로 1회 발화 → `cys send --to master` push + UI 배지. `alerts-config.json`(임계값).
- **산출**: 수동 관찰→**능동 경보**. autopilot 자원 거버넌스와 직결.

### E7 — RSI/autopilot 강화 (Control Center 외 트랙)
- 🔥사전 HEAD 체크포인트 + 원자 rollback(라운드 시작 시 SHA→SESSION_STATE·실패 시 reset) · 🔥진척도 신호(eval-driven 점수 improved/regressed vs 이전 라운드) · marker SOT(커밋 trailer `iter-id`) · task phase 상태 파일.
- **산출**: RSI 무결성·실패 복원력↑(eval-driven 원칙 강화).

### E8 — 엔지니어링 품질 (횡단)
- 컨텍스트 감지 `cys status`/boot(누락 단계만) · self-heal 원자쓰기 · hook 멱등+부트스트랩 · 설정 우선순위+stale 정리 · HTTP retry/timeout · HARD/SOFT 게이트.

### E9 — (선택) 팀/멀티머신 확장
- RBAC 4단(VIEWER=집계만·PII 차단) · 보존/redaction · self-host 다머신 집계(`--api-url`·중앙 cysd). **큰 결정**(로컬 우선 철학과 trade-off).
- **잔여 현황(2026-07-03)**: redact는 sessions·session_detail 대칭 적용 완료(로컬 부분). retention 자동삭제는 eval-driven "삭제 reward-hack 차단" 원칙과 상충해 보류. 멀티머신 중앙 cysd는 로컬우선 철학 위배 소지 — 오너 명시 결정 대기.

---

## 4. RPC 계약 (신규/확장) — 2026-07-03 현행화
- `control.dashboard`(확장): 기존 + 비용$·캐시절감·모델믹스·생산성. today 카운터는 읽기 시 날짜 가드(자정 직후 어제 누계 표시 방지).
- `control.hw`: 하드웨어 모니터링 — CPU 코어수(P/E)·코어별/전체 활용률·MEM(전 플랫폼, sysinfo 지속 인스턴스·무sleep) + GPU 코어수/활용률(macOS ioreg) + NPU 코어수/전력W(macOS IOReport 무sudo — 활용률%는 공개 API 부재라 null 고정). UI Live 탭이 2초 폴링.
- `control.analytics {window: today|7d|all}`: 비용·효율 롤업. `cache_roi_x`는 폐기(전 클로드 모델 캐시단가=입력의 10% → 항상 0.9인 무정보 상수였음) — `cache_efficiency`·`cache_savings_usd`만 제공. by_model 행에 `pricing_known`(false=단가표 미등재 → Sonnet 폴백 추정) 포함. 비용류는 반올림(round4).
- `control.skills {window}`: 스킬/에이전트 TOP·실패율·by_tool `p50_ms`(데몬 PRE→POST 페어링 산출). slash UNION은 미배선(UserPromptSubmit hook 채널 자체가 없음 — 배선 시 events.is_slash 소비 예정).
- `control.cost_baseline {window=7d}`: D3 eval baseline — by_tier+rework+캐시 지표 합본.
- `control.sessions {window, redact}` / `control.session_detail {session_id, redact}`: 목록(⭐note 포함)·상세. 상세는 이벤트 타임라인 + **전사 발췌**(세션 파일 꼬리 64KB 온디맨드·최근 30턴·턴당 400자 — messages 테이블 적재 없음). redact는 두 RPC 대칭 적용(session_id 해시·전사 생략). **두 RPC 모두 응답에 `redacted: bool` 을 에코**한다(상세도 2026-09-15 부터 — 가림 여부를 응답만 보고 판정 가능). `redact` 는 JSON bool 만 받는다 — `"true"`·`1` 같은 비-bool 은 조용한 off 가 아니라 `invalid_params` 거절이고, `null`/부재는 off(tauri `Option<bool>` 브리지 호환). **`redacted:true` 가 보증하는 범위는 `session_id` 해시(+상세의 `transcript` 생략)뿐이다** — `control.sessions` 의 `star_note`(⭐ 사용자 자유 텍스트)는 `redact=true` 에서도 raw 로 나간다(백로그 #31 · PII 가능 · 오너 결정 대기 — `analytics::redact_sessions` 는 `session_id` 만 가린다), 그러므로 반출 게이트가 `redacted` 만 보고 "가림 완료"로 판정하면 안 된다.
- `control.session_star {session_id, starred, note}`: ⭐ 토글+노트(재스타 시 starred_at 갱신).
- `control.weekly`: WoW·리더·인사이트.
- `control.alerts` + `alert.<kind>` 이벤트(경보는 이벤트+UI 배지로 정합화 — E5/E6의 "master push" 원안은 governance 교리상 채택하지 않음).
- `learn.status`: 학습 탭 데이터 — `{pack}/round/learn/state.json`(또는 $CYS_ROUND_DIR/learn). javis_rsi.py가 `_round/rsi/state.json` 저장 시 이 위치로 rounds/discovery를 미러링한다.
- Tauri 커맨드 동형 노출(`src-tauri/main.rs`).

### 소비 수집 경계 (usage.rs — 2026-07-03 현행화)
- pane 수집: claude(트랜스크립트)·**codex(rollout token_count의 last_token_usage + turn_context 모델 귀속)** 소비 적재. statusline 신선 시에도 소비 적재는 계속(관측 스냅샷만 statusline 우선).
- 외부(비-pane) 세션: `~/.claude*/projects` 15초 스윕, role=`external[:프로필]`.
- tail 오프셋은 analytics `tail_offsets`에 영속 — 재시작 시 정확 재개(재파싱 중복 INSERT 근절).
- 미수집(알려진 한계): gemini(agy)=쿼터만(토큰 소비 로컬 파일 부재·Phase 2-B), grok=수집기 없음. 데몬 기동 이전 외부 세션 히스토리는 소급 안 함(설계).

## 5. UI 정보구조 (Control Center 탭) — 2026-07-03 현행화 (9탭)
`Live`(T6+하드웨어 섹션) · `비용·효율`(E2) · `스킬·에이전트`(E3) · `세션`(E4) · `추세·주간`(E5) · `학습`(E7 편입 — RSI 라운드 타임라인·retention·발견·자율추천 대기. 데이터=learn.status) · `스킬 보드`(D5 — 클릭=일회용 워커 task-prompt 실행. 실행 경계는 make_ticket 무계약 차단+gate:hitl 미리보기) · `작업`(부서×노드 현재업무 — 이벤트 드리븐+5초 reconcile) · `승인 Feed` + 상단 경보 배지(E6).
바닐라 TS·차트 라이브러리 무의존(현 스파크라인·바·도넛 자체 SVG 확장).
갱신 주기: 대시보드 5초(Live 본문은 live 탭 표시 중에만 재렌더) · 하드웨어 2초 · 시계 1초 · 전 탭 5초 주기 재조회(sessions·weekly 포함). 데몬 3틱 연속 무응답이면 footer가 stale 경고로 전환.
glance("한눈에") 모드: 분석 위젯은 숨기되 **하드웨어 섹션은 유지**(Live면 목적=시스템부하).

---

## 6. 기대 업그레이드 (완료 시)

### 성능·정확도
- **영속성**: 재시작·셧다운에도 추세·소비·세션 보존(현재 휘발 → 무손실).
- **정밀 비용**: 토큰→$ 모델별 환산 — 자원 거버넌스를 **금액으로** 측정(현재 0).
- **실시간 유지 + 영속 분석 동시**(세션종료 후 확정 방식은 실시간이 약함 → 우리만의 조합).
- **결정론 집계**(정렬·롤업 캐시) — UI 일관성·부하↓.

### 신규 기능 (사용자가 새로 얻는 것)
| 영역 | 추가 기능 |
|---|---|
| 비용·효율 | 비용$·캐시 절감액·모델 믹스·세션당 비용·생산성(턴/세션·토큰/턴)·duration p50 |
| 스킬·에이전트 | 호출 TOP·**반복실패율**·미사용(휴면) 탐지·스킬×노드 분포·위임 트리 |
| 세션 | 타임라인·활동 리본·전사 뷰어·자동 title/summary·⭐성공세션 재현(온보딩) |
| 추세 | 주간 WoW%·이번주vs지난주·효율 리더(노드별)·위임/스킬자산 인사이트·주간 다이제스트 master push |
| 경보 | 토큰/비용 임계·이상감지(급증)·반복실패·ROI 추천 → 능동 push |
| RSI | 라운드 체크포인트 자동 rollback·진척도 신호·marker SOT |
| 품질 | 컨텍스트 감지 setup·self-heal·멱등 hook·retry/timeout |

### 전략적 효과
- **"모니터 → 최적화 → 거버넌스"** 격상: 우리가 AI를 **효율적으로** 쓰는지 정량 측정·자동 경보.
- **플릿 자기개선 신호**: 효율 리더·반복실패·진척도 = RSI 입력.
- **차별화 포인트**: 실시간성(우리 강점) + 반복실패율·이상감지·ROI + 로컬우선 프라이버시.

---

## 7. 비기능·리스크
- **에이전트 0ms 지연**: hook은 fire-and-forget 즉시 exit. 절대 claude/codex 안 막음.
- **로컬 우선**: 데이터 머신 밖으로 안 나감(E9 다머신은 명시적 옵트인).
- **부하**: 일별 롤업 lazy·오늘 live 30s 캐시·인덱스. SQLite WAL.
- **프라이버시**: 전사 50k 절단·보존 정책·(E9) RBAC VIEWER 집계만.
- **마이그레이션**: analytics.db 스키마 버전·expand-contract.

## 8. 권고 착수 순서
**E1(척추) → E2(효율) → E6(경보) → E3(스킬·실패) → E5(주간) → E4(세션) → E8(품질) → E7(RSI) → E9(팀)**.
E1 없이는 나머지 불가(영속·이벤트가 토대). E2·E6이 체감 가치 최고(비용·경보). E7은 독립 트랙(병행 가능).
