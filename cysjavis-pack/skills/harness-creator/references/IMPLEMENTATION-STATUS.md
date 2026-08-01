# IMPLEMENTATION-STATUS — GROUND TRUTH (M0–M8 구현 완료)

> **이 문서가 다른 모든 reference의 aspirational 서술에 우선한다.** reference가 무엇을 설명하든, 실제로 emit/validate에 구현됐는지는 여기로 확정한다. (출처: `emit_orchestrator.py`·`emit_domain_skill.py`·`audit_harness.py`·`evolve_harness.py`·`eval_topology.py`·`inherit_genome.py`·`validate_harness.py`·`templates/hooks/`·`tests/test_factory.py` 실측. 설계 전모: `design/STRATEGY-AND-DESIGN.md`.)
>
> **현 상태: 134 factory tests green, 4 예제 validate 0/0, idoforgod 8 use case 전부 conform, in-project 오버레이 설치·lift 배선·h2h 보강 완료 + 7-dim 적대적 전수감사(18 confirmed) 후 P0×3·robustness×4·P1×4·P2(A/B/C/G) 보강 완료 + 자동 교정 루프(린터+맞춤법+프리커밋, 팩토리 자기적용·게놈 전수) + 3층위 장기기억 self-hosting(회상→주입→릴레이→증분 전 도관 배선) 추가.**

## 🔧 적대적 전수감사 보강 (audit wkm8lt82v → reinforcement)
7-dimension 적대적 audit(A1/A2/parity/DNA/factory-workflow/contract/robustness)에서 18 confirmed 결함 발견(verdict PARTIAL) → 수정:
- **P0-1** SOT 경로 불일치(genome `sot_paths` `.claude/`만 읽어 Tier-I 스냅샷이 CYS `.harness/state.yaml` 누락) → `.harness/` 우선 해석.
- **P0-2** `hybrid`가 A2 실패하는 死모드 → team 레시피 emit.
- **P0-3** 자족 설치가 prompt-runner `claude -p` 실행기·workflow.js CANONICAL 광고(A1 위반) → 제외+`PROMPT_RUNNER_ABSENT` 파일시스템 게이트+self-heal+매니페스트 재배선.
- **robustness×4**: validate 크래시(malformed graph)→clean fail; `GRAPH_SCHEMA_DEGRADED` warn; LIFT 수학신뢰(forged register 차단); 기본 RUNTIME_MANIFEST 프리미티브화.
- **P1-1** A2 substring 게이트 주석무력화 → HTML 주석 strip.
- **P1-2** L2 리뷰 예제 0노드 → deep-research/design-decision에 `review` + `PRODUCER_REVIEWER_REVIEW` 코드.
- **P1-3** L1/L1.5/L2 fire-on-presence → **L1 필수(fail-closed)**: verification log 없으면 exit-2(L0·L1·budget 필수, L1.5·L2 fire-on-presence).
- **P1-4** 4-스테이지 워크플로 미강제 → e2e self-test(predicates→warrant→audit→emit→validate) + `warrant --graph`가 `.harness/warrant.json` 기록 + opt-in `BUILD_GATES` 정책(기본 off; warn/error 시 warrant.json·audit.json·APPROVED 필수).
- **P2-A** topology↔구조 미결합(1-node가 fan-out-fan-in/hierarchical에 conform) → `TOPOLOGY_STRUCTURE`(fan-out-fan-in=≥2 producer→sink, hierarchical=≥3노드) + TestEightUseCases가 토폴로지별 현실 구조 사용.
- **P2-B** emit된 agent 본문이 2줄 stub → graph 필드에서 **rich body 생성**(핵심역할·작업원칙(role-class별)·I/O 프로토콜·에러핸들링·팀통신·L2). 손작성 본문은 보존(fallback만 교체).
- **P2-C** graph.json single-writer 미강제(손편집 0/0 통과) → emit가 `.harness/graph.lock`(sha256) stamp + `GRAPH_PROVENANCE` warn(사후 변경 감지) + SKILL P1 주장 정직화(SoT).
- **P2-G** qa-token-trap(이름에 qa+critic 토큰 → qa-scan/haiku 강등) → `QA_TOKEN_TRAP` 정밀 warn(verifier 같은 정상 노드는 미발동).
- 신규 validate 코드(총): `PROMPT_RUNNER_ABSENT`·`GRAPH_SCHEMA_DEGRADED`·`PRODUCER_REVIEWER_REVIEW`·`BUILD_GATES_SKIPPED`·`TOPOLOGY_STRUCTURE`·`GRAPH_PROVENANCE`·`QA_TOKEN_TRAP`. **76 factory tests.**
- **의도적 미구현(결함 아님, 정직 기록)**: (D) i18n — 팩토리 문서 KO 전용은 scope 선택; 산출 하네스는 게놈 `@translator`+glossary로 번역 *가능*. (E) 8 use case **run-level** — 강제 대상은 build-level conformance, run-level은 별도 quota-gated h2h 레인(deep-research n=5 +12.5pp INCONCLUSIVE만 verdict 아티팩트로 stamp; verification-heavy 레인은 rate-limit으로 미stamp). (F) RLM Tier-II recall/record가 prose인 것은 **A1 준수**(회상·판단은 프리미티브/agent의 도메인 작업; Python으로 코드백업하면 A1 위반 — 시드·인덱스·파싱만 코드). 예제 deep-research를 실제 fan-out으로 전환은 별도 enhancement. audit 전체: workflow output wkm8lt82v.

## 🔍 잠복 결함 전수 감사 (2026-06-02, /diagnose) — 16 수정 + 알려진 결함
green 상태(`test_factory.py` 134 green·4 예제 validate 0/0)에서 **그 신호가 못 잡는** 잠복 결함을 5-에이전트 병렬 감사 + verify-before-assert(직접 재현)로 확정·수정. TDD(전부 수정 전 실패 확인), 회귀 `tests/test_diagnose_regression.py` **40개**(`test_factory.py` 134는 불변 — `make test` 합계 174). 커밋 `40f16de`(A·B 10) + `9bfbd69`(C 6).

- **A(확정 6)**: role-class 부분문자열/순서 오분류(`search`→`\bsearch`로 'research' 오매치 제거 + format 패턴 모호어 `report` 제거; JSON SoT + Python/JS fallback 3미러 동기) · `MEASUREMENT_DRIFT` 손상-verdict 흡수 차단("파일 없음"=정직한 미측정 vs "전부 손상" 구분) · 재emit이 `model_rationale`·`tools`를 stale 보존(자기모순·최소권한 회귀)→항상 graph에서 재유도 · 비-UTF8 로케일 크래시→text-mode `open()` 11곳 `encoding="utf-8"` · h2h verdict 이중 반올림→raw median delta 판정.
- **B(코드명백 4)**: `GRAPH_CYCLE`가 pipeline/dispatch만 검사→`producer-reviewer` 외 전 토폴로지(emit 무조건 toposort와 정합) · `spawn_counter`가 `TeamCreate`(멤버 N)를 +1만 증분→members 수만큼(advisory-safe) · `qa_gate_runner` high-water int→gated 집합(비순차 기록 스텝 누락 방지, 구 포맷 하위호환) · emit가 id/agent를 경로검사 없이 사용(emit이 validate보다 선행)→`_require_valid_graph`에 path-safe basename 검사.
- **C(minor/robustness 6)**: `query_norm` 비-Latin 빈 키→안정 hex digest fallback · `tier_override_reason` 공백으로 `TIER_OVERSPEND` 강등→`.strip()` 비어있지 않을 때만 · lint/spell `SKIP_PARTS` substring-anywhere→프로젝트 루트 anchored(사용자 `src/examples/` false-SKIP 해소) · `change-history`가 read-modify-rewrite→진짜 append(동시 lost-update 차단) · `_condition_keys`가 `overall_pass_rate` 흡수→`^c\d+_pass_rate$` 제한 · `genome_file_count`가 `__pycache__` 포함→`_count_genome_files` 헬퍼로 제외(결정론).

**알려진 결함 (미수정 — 정직한 disposition; 수정이 AC-3상 부적절하거나 비활성)**:
- **by-design(결함 아님)**: (B-F6) pure-retrieval 노드 + 비-single mechanism이 voter/debater/reviser(sonnet)로 분류 — mechanism이 추가 추론을 함의하므로 타당. (D-F4) `evolve_harness`가 graph·validate를 안 건드림 — 진화는 라우팅 로거이고 validate 재통과는 오케스트레이터가 Implementation 재진입으로 강제(워크플로 계약).
- **mitigated/protected**: (C-F1) `spawn_counter` bump이 첫 `spawns_used:` 매칭 — `sot_init` 시드가 `budget`을 `audit_log`보다 앞에 두어 보호(방어적 hardening 여지만). (C-F7) `precommit_gate`의 `ruff check .` — `inherit_genome`이 vendored 제외 `harness-ruff.toml`을 설치해 완화; in-project + host 자체 ruff.toml(제외 누락) 엣지만 잔존.
- **dormant**: (C-F4/C-F5) PyYAML 부재 시 `qa_gate_runner`/`budget_block` regex fallback이 블록 밖 토큰 매칭 — 게놈이 PyYAML 동봉이라 비활성. 향후 PyYAML 제거 시 재평가.
- **저영향**: (D-F3) `read_history`가 손상 줄 silent skip — D-F2(append) 수정으로 손상 가능성 감소. (D-F5/D-F6) `emit_workflow` substring invariant 크래시 / 누락 키 KeyError — `emit_workflow`는 **공장 내부 측정 전용**(제품 emit 은퇴, README A1).

## ✅ 구현·검증 완료 (M0–M8)

### M0 — 프리미티브-위임 모순 봉쇄 (절대규칙)
- `workflow`(Mode-A `workflow.js`)는 **제품에서 은퇴**(`WORKFLOW_RETIRED`). 예제 4개 team 모드 마이그레이션. `emit_workflow.py`는 **공장내부 측정 전용**.
- 자식 `RUNTIME.json`/`CLAUDE.md`가 은퇴 런타임을 광고하던 누수 스크럽(`RUNTIME_MANIFEST_CLEAN`). `emit_orchestrator.py` `_spawn_recipe` 스키마-참조 버그 수정(node.output_schema 사용).
- **발화 hook 3종 신규**(`templates/hooks/`): `spawn_counter`(`budget.spawns_used` 증분 → 천장 발화), `sot_init`(`state.yaml` 시드), `qa_gate_runner`(L0-L2를 `gate_or_block`로 발화, evidence-gated — L0 in-hook anti-skip + **L1 필수(fail-closed, P1-3)** + L1.5/L2 fire-on-presence). 실증: spawn ceiling exit-2 발화, 누락 산출물 → L0 차단, verification log 부재 → L1 exit-2 차단.
- **실제 team emit**: `execution_mode=team`이 `TeamCreate/TaskCreate(deps)/SendMessage/TeamDelete` 생성(`TEAM_EMIT_PRESENT`) — agent emit과 더 이상 byte-동일 아님.

### M1 — 4계층 QA + Context Preservation 일급화
- 오케스트레이터가 `## 메모리 운영`(Tier I) 섹션 emit(`CONTEXT_PRESERVATION_FIRSTCLASS`). review 노드의 reviewer/fact-checker 파일 존재 강제(`REVIEW_AGENT_PRESENT`). `HOOK_REGISTERED`에 `save_context` 추가.

### M2 — A2 all-6 floor + 7 토폴로지
- 모든 빌드 하네스가 6종 프리미티브 전부 인스턴스화(`ALL_PRIMITIVES_PRESENT`: 호출형 `TeamCreate(`+`Agent(`). 팀 graceful-degrade(`TEAM_GRACEFUL_DEGRADE`).
- `topology` enum 3→7. **supervisor·expert-pool·hierarchical·fan-out-fan-in을 first-class emit 타겟으로 구현**(`_topology_addendum` + `TOPOLOGY_PRIMITIVE_CONSISTENCY`).

### M3 — 하이브리드 도메인 스킬 (idoforgod who=agent / how=skill)
- `node.skill_authoring{mode:inline|skill, reason, shared_by}`(머신체크). `emit_domain_skill.py`: `mode=skill` 노드만 `.claude/skills/<harness>-<id>/SKILL.md` 저작. `SKILL_AUTHORING_JUSTIFIED`(reason 검증, reuse→shared_by≥2)·`INLINE_NO_ORPHAN_SKILL`·`LIFT_UNMEASURED`(warn).

### M4 — Phase-0 상태감사
- `audit_harness.py`: new/extend/maintain 분기 + **결정론 드리프트**(디스크 agents/skills ↔ graph 계약 set-diff) → `.harness/audit.json`(`AUDIT_VERDICT_PRESENT`).

### M5 — Phase-7 진화 루프
- `evolve_harness.py`: 피드백 유형→대상 라우팅 테이블 + `.harness/change-history.jsonl`(append-only) + `--proactive`(같은 유형 2회↑ 자동 제안). `EVOLUTION_WIRED`·`EVOLUTION_LOG_PRESENT`.

### M6 — RLM 교차-실행 메모리 (Tier II)
- `inherit_genome._init_memory_store`: `.harness/memory/`(`archive.manifest.json`·`domain-knowledge.yaml`·`runs/index.jsonl`·`risk/decisions.jsonl`) 시드, **idempotent 누적**(재emit이 기존 run 미파괴). 오케스트레이터 Tier-II RLM 회상(Grep index)·기록 레시피. `MEMORY_SKILL_SECTION`·`MEMORY_STORE_INIT`.

### M7 — 8 use case parity 평가
- `eval_topology.py`(순수 matcher) + `TestEightUseCases`: idoforgod README 8 use case 전부 **빌드레벨 conform**(토폴로지+exec_mode+all-6+DNA). 5개 토폴로지(fan-out-fan-in·pipeline·producer-reviewer·supervisor·hierarchical) 행사. 런레벨 h2h는 별도 레인.

### M8 — 정직성
- design 문서의 죽은 `+38pp` 교정 + `STALE_BENCHMARK` factory 가드(design/ 스캔). `MEASUREMENT_DRIFT` 구현됨(produced-harness README/SKILL 스캔).

### 측정 (head-to-head, stamped)
- **n=5(deep-research) + 다도메인 추가 → median(C2)=1.0 vs median(C3)=0.875 → +12.5pp `INCONCLUSIVE`** (CYS 우세, 15pp 마진 미달). 이전 n=1 −16.67pp `BASELINE-WINS`를 뒤집음. 활성 **L2 적대적 리뷰**가 baseline(no-harness opus)의 A4(미검증 주장 잔존)·A6(통계 날조) 실패를 잡은 것이 격차. **약한 데이터를 날조하지 않음**(+37.5pp 교훈). `evals/deep-research.verdict.json`.

### P1.2 — in-project 오버레이 설치 (B2) ✅
- `emit_orchestrator.py <TARGET> --in-project`: idoforgod식 **기존 호스트 프로젝트 오버레이 설치**. 자족(self-contained) 기본은 불변.
- **호스트 보존**: 루트 `CLAUDE.md`(포인터만 append)/`AGENTS.md`/`README.md`/`soul.md`, 호스트 `.claude/agents|skills|config|hooks`(동명 파일은 `rsync --ignore-existing`로 **호스트 우선** — 튜닝된 보안 hook 포함) 절대 미클로버. 노드 agent는 `cys_emitted` provenance 마커 + **동명 호스트 agent 충돌 시 emit 거부**.
- **L2 DNA 예외(force-install)**: 적대적 리뷰 agent(`reviewer`/`fact-checker`)는 head-to-head 변별력의 핵심이라 **게놈판을 강제 설치**한다(host-wins 비클로버에서 제외). 충돌 시 호스트 원본은 `.harness/genome/displaced/`로 **백업**(파괴 없음) + stderr 통지 → `REVIEW_AGENT_PRESENT`가 "엉뚱한 호스트 reviewer"로 통과하는 것을 방지.
- **게놈 부분집합**: `.claude/hooks`(런타임 DNA) + `.claude/{agents,skills,config}` 모두 비클로버 union. ~440KB 헌법 + `docs/`는 **`.harness/genome/`로 재배치**(어떤 런타임 hook도 루트 .md를 *읽지 않음* — guarded 문서-동기 lint뿐 — 검증됨). `prompt`/`prompt-runner`/`translations` 미설치. 로그 디렉토리는 `.harness/` 하위.
- **settings 안전**: 호스트 hook·permissions 보존 + 게놈 hook union + 게놈 `permissions.deny` 보안 union(`_union_perms`). 호스트 settings.json이 **비객체(`[]`/`null`)면 graceful coerce**, **파싱불가면 emit 거부**(호스트 제어 무단 폐기 금지).
- **모드 전환 가드**: `install_mode` 마커(`.harness/GENOME.json`)로 `validate`가 `GENOME_PRESENT`(루트 vs `.harness/genome/`)·`W1_GENOME`·doc/measurement-drift 경로 자동 분기(마커 손상 시 `.harness/genome/CLAUDE.md` 존재로 구조적 감지). **다른 모드로 재emit은 거부**(in-project↔self-contained 전환 시 호스트 클로버 방지). 재emit idempotent(포인터 1회).
- **검증**: CLI emit+validate 0/0, **52 factory tests**(+6 in-project), **3-lens 적대적 리뷰**(correctness·host-safety·parity)에서 발견된 4 MAJOR(비객체 settings 크래시·파싱불가 호스트손실·hook 클로버·reviewer 누락) 전부 수정·재현테스트 추가.

### P1.3 — lift 빌드 배선 ✅ (게이트에 이빨)
- `lift_gate.py score <results> --out <skill>/lift_verdict.json`: 측정 결과(verdict)를 validate가 읽는 정확한 경로에 기록.
- validate: 미측정=`LIFT_UNMEASURED`(constants.json `LIFT_UNMEASURED` 정책, 기본 `warn`→`error` 전환가능); **측정했으나 baseline 미달(`decision≠register`)=`LIFT_REFUSED`(hard error)** — baseline에 진 스킬은 출하 불가(inline하거나 개선). 측정-실패가 빌드를 실제로 막는다(이전엔 presence-warn만).

### P1.4 — h2h 측정 보강 ✅ (StructuredOutput flakiness)
- `h2h_suite.workflow.js`: `tryAgent` 재시도 래퍼(null/throw 시 ATTEMPTS회, 라벨 변주로 캐시회피) + **flake run은 0점이 아니라 DROP**(보고서/채점 누락 run을 median에서 제외 → 가짜 0이 중앙값 왜곡 불가) + provenance `n_attempted/n_valid/n_dropped`.
- `h2h_aggregate.py`: 무효 run(`valid:false`/키누락) 첫 발견에 raise하던 것을 **필터링**으로 변경(부분실패 suite를 정직하게 집계) + `n_dropped` 보고. 이전 n=5 측정의 7/12 실패 패턴을 구조적으로 흡수.

### references/ 전면 재구성 ✅ (D1 8-파일 맵)
- 옛 `examples.md`→`architecture-patterns.md` 흡수, `skill-writing-guide.md`→`skill-and-agent-authoring.md` 개명·확장, 신설 `genome-and-runtime.md`·`evolution-and-memory.md`. 7 파일의 옛 'workflow.js=제품 런타임' 전제 서술을 프리미티브 기질로 재배선(workflow.js는 공장내부 측정 라벨만 잔존). draft→adversarial-verify(코드 대조)→fix 워크플로우로 생산; 발견된 fabrication(없는 critique.json 게이트·날조 측정 description·miscoped STALE_BENCHMARK·BLOCKER qa role-class·절대경로·edges≠depends_on 오기) 전부 코드대조 수정. SKILL.md §참고 8-파일 맵으로 갱신.

### P2 — 라이브 DNA 발화 end-to-end 증명 ✅
- 중첩 인터랙티브 `claude` 세션은 띄울 수 없으므로, **emit된 하네스의 wired hook을 Claude Code가 부르는 방식(`CLAUDE_PROJECT_DIR` + 실제 `.harness/state.yaml`)으로 subprocess 구동**해 DNA 발화를 증명(durable, CI-able). `TestEmittedHarnessDNAFires`: SessionStart `sot_init`→SOT 시드(graph에서 max_spawns) → `spawn_counter`로 spawns_used 증분 → `budget_block` 천장 **exit-2** 발화 / QA L0 누락산출물 **exit-2** / 게놈 보안 hook `rm -rf`·`git reset --hard` 차단 / Tier-II 메모리 시드 / 오케스트레이터 실제 team 프리미티브. 60 tests green.

### P2 — CYS-WINS 재측정 (정직한 null 결과) ✅
- **유일하게 stamp된 수치 데이터포인트는 deep-research** (`examples/deep-research/evals/deep-research.verdict.json`: n=5, median(C2)=1.0 vs median(C3)=0.875, **+12.5pp `INCONCLUSIVE`**). 이것이 repo에 verdict 아티팩트로 백업된 유일한 h2h 결과다.
- verification-heavy 도메인(passkey/WebAuthn sync, 인용·검증 8 assertion)의 추가 h2h 레인은 **시도했으나 rate-limit으로 미완**(`_workspace/h2h/runs.json`: `n_actual:0, status:RATE-LIMITED`) — **수치 결과를 stamp하지 않았다**(verdict 아티팩트 없음 → 인용 가능한 결과로 주장하지 않음). 정성적 가설(검증-규율 스코어카드에서 single-pass opus가 천장을 포화시켜 마진 확보가 어려울 수 있음)은 *측정 전 가설*로만 둔다.
- **NOT CYS-WINS** — 약한·미측정 데이터를 날조하지 않는다(+37.5pp 교훈). 백업된 정직 데이터포인트는 deep-research +12.5pp INCONCLUSIVE 하나뿐.
- **P1.4 하드닝 실전 검증**: deep-research n=5 레인에서 `n_dropped=0`(5/5 valid) — 이전 7/12 flake 패턴이 재시도+drop으로 해소됨.
- **구조적 결론**: 현대 opus single-pass가 객관적 scorecard에서 이미 천장(8/8)이라 하네스가 **마진으로 이기는 것은 구조적으로 불가**. 더 많은 도메인을 뒤져 ≥15pp를 낚는 것은 **벤치마크 게이밍**(+37.5pp 교훈)이므로 중단. 하네스의 가치는 scorecard 승리가 아니라 **parity(실제 목표) + 결정론적 인프라**(DNA 발화·비용거버넌스·머신체크 계약 — scorecard가 못 잡는 보장)에 있다. **parity 목표는 충족.**

### 자동 교정 루프 — 린터 + 맞춤법 + 프리커밋 (팩토리 자기적용 + 게놈 전수) ✅
PostToolUse/PreToolUse hook의 `exit-2 + stderr` 자가수정 메커니즘 위에 3종 가드를 추가(`templates/hooks/`):
- **`lint_guard.py`** (PostToolUse Edit|Write) — 방금 쓴 `.py`를 ruff로 검사. 기계적 위반은 `--fix` **자동 적용**(사람 개입 0), 남은 의미 위반(F821 등)은 **exit-2 + stderr**로 Claude 자가수정 유도. vendored 트리(`genome/`·`.harness/genome/`·`.claude/hooks/scripts/`·`examples/`)는 스코프 제외.
- **`spell_guard.py`** (PostToolUse Edit|Write) — `.md`/`.txt`의 **고신뢰 한국어 오타**(`됬`→`됐`·`역활`→`역할` 등 11종, 거의 항상 틀린 형태만)를 exit-2 차단. 인라인/펜스 코드 안의 인용은 제외(오타를 *논하는* 문서 false-positive 방지). 문맥 의존 맞춤법은 Claude(A1) — 사전은 보수적.
- **`precommit_gate.py`** (PreToolUse Bash) — `git commit`을 인터셉트(서브커맨드 정밀 판정, 전역옵션 스킵)해 ruff(스코프)+테스트 통과 못하면 **exit-2 "잠깐, 이것부터"** — 빨간 트리가 히스토리에 못 들어감.
- **A1 준수**: ruff·테스트러너는 결정론 가드레일(차단/측정), `--fix`는 기계적 안전변환, *의미* 수정은 프리미티브(Claude).
- **점진 도입**: `.lint-guard` 토글 파일로 on/off(기본 off — 산출 하네스는 운영자가 켬, 팩토리는 가동). 룰선택은 `ruff.toml`(`select=F,E4,E7,E9`/`ignore=E702`=CYS 압축 스타일). 안전제일: ruff 부재·토글 off·스코프 외·내부예외 → exit-0(절대 차단 안 함).
- **배선**: 팩토리는 루트 `.claude/settings.json`이 `templates/hooks/`를 직접 참조(dogfood, 복사본 0). 산출 하네스는 `inherit_genome._CYS_HOOKS`+`_merge_settings`+`_install_ruff_config`(harness-scoped `ruff.toml`, 호스트 ruff 설정은 in-project 보존)로 전수. emit→전수 end-to-end 검증.
- **Phase 0 정리**: 팩토리 1급 도구(루트·lib·templates/hooks·tests) ruff clean(genome 제외 시 위반 37→0; F821 94개는 전부 vendored genome `_context_*.py`라 별도 상류 트랙). **27 신규 테스트**(lint 8·precommit 6·inheritance 6·spell 7).

### 3층위 장기기억 self-hosting — 회상 배선 (메모리가 prose가 아니라 실행 단계) ✅
산출 하네스 run 평가에서 "장기기억이 데이터 파이프라인 연결조직으로 작동했는가 → NO"가 나온 근본 원인은 메모리의 **read-side(회상·주입)가 오케스트레이터 SKILL의 prose 섹션에만 있고 실행 단계에 배선되지 않았으며, validate가 "존재"만 검사하고 "배선"을 검사하지 않은 것**("presence ≠ wiring"). 이에 장기기억이 **3층위 모두**에서 단일 메커니즘 dogfood로 작동하도록 수정(설계: `design/adr-3tier-memory-self-hosting.md`, project memory `three-tier-memory-self-hosting`).
- **P0a — 층위 1(팩토리 자체)**: `inherit_genome._init_memory_store(dir, kind="domain"|"build")` — seed 문구를 kind로 분기. `bootstrap_factory_memory.py`가 팩토리 루트에 `kind="build"` 빌드 메모리를 시드하고 **기존 4 예제를 빌드 이력으로 임포트**(콜드 해소). `record_build(graph)`로 임의 빌드 1줄 기록(idempotent).
- **P0b — 층위 2(빌드 과정)**: `harness-creator SKILL`에 **R0 빌드 회상**(RESEARCH 이전, 팩토리 `memory/runs/index.jsonl` Grep → 유사 도메인의 topology·tier·mechanism 패턴을 P1 graph 저작에 주입) + **E0 빌드 기록**(EVOLUTION) 배선. 팩토리 self-test(`TestBuildRecallWired`)가 강제.
- **P0 — 층위 3(산출 하네스 런타임)**: `emit_orchestrator` Phase 0을 "장기기억 회상 + 컨텍스트 + SOT"로 확장, 본문에 회상 실행 단계 추가(Grep `runs/index.jsonl` → 매치 run + `domain-knowledge.yaml` Read → `_workspace/_recall.json` 릴레이 + `audit_log` 기록; 콜드는 `{"cold":true}`). **신규 게이트 `MEMORY_RECALL_WIRED`**: 오케스트레이터에 `_recall.json` 릴레이(실행 배선 증거)가 없으면 error — prose-only 회귀를 차단(presence → wiring 승격).
- **self-host 일관성**: 회상→주입→릴레이→기록 단일 메커니즘을 팩토리가 dogfood(산출 하네스 = `validate MEMORY_RECALL_WIRED`, 팩토리 = `TestBuildRecallWired`). A1·RLM 경계 유지(벌크는 `_workspace/`, 검증된 사실 atom만 메모리; 회상·증류는 프리미티브, Python은 경로·인덱스·게이트만).
- **P1 — 에이전트 메모리 입력 계약 ✅**: `_agent_body`에 `## 메모리 입력` 블록 + `_write_agent_files`가 hand-written 본문에도 계약을 append(본문 보존, idempotent) → 하류 노드가 `_workspace/_recall.json` + `domain-knowledge.yaml`을 **Read**(회상 *소비*, "메모리-맹목" 해소). 신규 게이트 `AGENT_MEMORY_CONTRACT`(에이전트가 `_recall.json`을 안 읽으면 error). 이로써 **회상→주입 도관 폐합**(Phase 0 회상 → `_recall.json` → 에이전트 Read).
- **P2 — 단계간 사실 릴레이 ✅**: emit Phase 2에 메모리 릴레이 배선 — 단계 게이트 통과 직후 검증된 사실 atom만 `.harness/memory/runs/<run_id>/stage-<N>-facts.jsonl`에 증류 + SOT `outputs.step-<N>` 기록, 하류 노드 입력 프로토콜이 하드코딩 경로 대신 **SOT outputs.step-N + 직전 stage-facts Read**로 해소. 신규 게이트 `MEMORY_RELAY_WIRED`. 이로써 **검증된 사실 척추가 단계마다 메모리를 통과**하도록 *배선*됨(설계 기준 "데이터가 메모리를 통해 파이프라인처럼"의 배선 완성; 벌크는 `_workspace/` 유지. 실측은 회상 hit가 나는 다음 run에서 확인).
- **P3 — Tier-II 증분 스트리밍 ✅**: emit Phase 0에 **run START `status="in_progress"`** 기록(`runs/index.jsonl`) + Phase 2에 단계별 `domain-knowledge` 증분 병합 + Phase 3에 `status="completed"` 종결. 신규 게이트 `MEMORY_INCREMENTAL_WIRED`(in_progress 마커 없으면 error — "예치-at-end" 회귀 차단). 크래시·재개·병렬 run이 부분 진행을 회상.
- **메모리 도관 완성 (평가 4대 갭 폐합)**: 회상(P0)→주입(P1)→릴레이(P2)→증분(P3)을 4 게이트가 wiring으로 강제 — `MEMORY_RECALL_WIRED`·`AGENT_MEMORY_CONTRACT`·`MEMORY_RELAY_WIRED`·`MEMORY_INCREMENTAL_WIRED`. 3층위(팩토리·빌드·산출 하네스)에서 단일 메커니즘 dogfood. 실측은 회상 hit가 나는 다음 run에서 확인. **134 factory tests green**(7-dim 적대 재감사 후 tier-SoT 동치·max_spawns/recall baking·malformed-node·agent-label·recall-key 회귀가드 추가분 포함).

## ❌ 폐기된 규칙 — 더 이상 적용 안 함
- **NO_COMMANDS** 폐기(게놈 commands 정상; 새 도메인 커맨드는 직접 안 만듦). **"모든 에이전트 opus"** → role-tier 정책. **"team이 기본 / Mode-A(workflow)가 기본"** → 둘 다 폐기: **workflow 은퇴**, 빌드 하네스는 **all-6(team/hybrid)**.

## 규약 메모
- **producer-reviewer topology ≠ reflect-then-revise mechanism** — topology(2노드 루프)와 mechanism은 독립.
- **skill_authoring 하이브리드** — `reuse`(shared_by≥2)/`complex`/`conditional`일 때만 도메인 스킬 저작, 그 외 inline(throwaway 스킬 방지).
- **A1 경계** — Python은 결정론 가드레일(차단/저장/측정/파싱)만; 도메인 판단·생성은 프리미티브.
