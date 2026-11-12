---
name: harness-creator
description: "도메인 한 문장을 검증된·비용통제된·재개가능한 풀스택 Claude 프리미티브 하네스(orchestrator skill + .claude/agents + Sub-agents + Agent Teams + Hooks + .claude/skills + 장기기억)로 변환하는 메타스킬(CYS Harness Creator). '하네스 만들어줘/구성/설계', '에이전트 팀 설계', '이 도메인 자동화', '하네스 점검·감사·확장·진화·동기화' 요청 시 사용. 후속: 재실행·수정·보완·부분재생성·이전결과개선·드리프트수정·유지보수 요청 시에도 반드시 이 스킬을 사용."
---

# Harness Creator (CYS) — 메타스킬

도메인 한 문장 → 머신체크된 `graph.json`(불변 계약)에서 **풀스택 Claude 프리미티브 하네스**를 emit한다:
orchestrator skill + `.claude/agents/`(who) + 하이브리드 `.claude/skills/`(how) + Sub-agents(Agent) + Agent Teams(TeamCreate/SendMessage) + Hooks(settings.json) + 상속 AWF 게놈(L0-L2 게이트·SOT·적대적 리뷰·장기기억) + git repo.
idoforgod/harness 대비: prose 규칙이 아니라 **머신체크 게이트 + 프리미티브 실행 + 비용 거버넌스 + 게놈 발화 + 헤드투헤드 측정.**

## 핵심 원칙

1. **산출 하네스 실행 = 100% Claude Code 프리미티브 (절대규칙, A1).** 도메인 작업(추론·판단·생성)은 Agent/TeamCreate/SendMessage/TaskCreate가 수행한다. 결정론 Python(hook·검증기·메모리 스크립트)은 *실행이 아니라 가드레일*(발화·강제·기억)이다 — 제거 시 도메인 *답*이 바뀌면 불법(프리미티브여야), 차단/저장/측정/파싱만 하면 합법. **`workflow`(Mode-A workflow.js)는 제품에서 은퇴**(공장내부 측정 전용); `execution_mode` ∈ `team`/`agent`/`hybrid`.
2. **모든 빌드 하네스는 6종 프리미티브 전부 인스턴스화 (A2 floor).** orchestrator + agents + sub-agents + **≥1 team stage** + hooks + ≥1 skill. → `execution_mode`는 `team` 또는 `hybrid`(pure-`agent`는 TeamCreate가 없어 `ALL_PRIMITIVES_PRESENT` 실패). 팀은 실험플래그 부재 시 sub-agent로 graceful-degrade. (warrant `single-agent` 판정은 예외.)
3. **모든 규칙은 머신 강제** — `validate_harness.py`(49 머신체크 코드)가 위반 시 생성 실패. 런타임은 hook으로 *발화*: `qa_gate_runner`/`gate_or_block`(L0-L2 exit-2), `budget_block`+`spawn_counter`(spawn ceiling), `sot_init`(SOT), 보안 hook.
4. **장기기억 일급** — Tier I(Context Preservation: 스냅샷·`[CONTEXT RECOVERY]` 복원·RLM knowledge-index) + Tier II(`.harness/memory/` 교차-실행 도메인 기억, RLM 외부환경). 모든 산출 하네스에 emit·검증(`CONTEXT_PRESERVATION_FIRSTCLASS`·`MEMORY_STORE_INIT`).
5. **비용 = 사전 승인 + 런타임 ceiling** — `warrant.py` 토큰밴드 후 사람 승인, 런타임 `budget_block` spawn ceiling exit-2.
6. **하네스 = git repo + 진화 시스템** — `.harness/state.yaml`(단일쓰기 SOT), `.harness/change-history.jsonl`(진화 이력).

> 설계 전모·잠금결정·백로그: `design/STRATEGY-AND-DESIGN.md`. 구현 현황: `references/IMPLEMENTATION-STATUS.md`.

## 호출 & 경로 (전역 설치되어도 자족 작동)
- **트리거:** `/harness-creator <도메인 한 문장>` (또는 description 매칭).
- **TOOLS_ROOT** = `${CYS_HARNESS_HOME:-$HOME/.cys/harness-creator}` — 모든 도구·게놈(`genome/`)이 여기. 항상 `python3 "$TOOLS_ROOT"/<tool>.py`.
  (cysjavis pack 통합판: 다른 자리의 로컬 원본을 쓰려면 `CYS_HARNESS_HOME` 으로 지정한다.
  신규 머신은 preflight C21 `--fix`가 GitHub 핀 커밋을 자동 클론한다 — 직접 설치:
  `git clone https://github.com/idoforgod/cys-harness-creator "$HOME/.cys/harness-creator"`)
- **TARGET** = 하네스 경로. 미지정 시 `./<harness_name>/`. 이하 `<TARGET>`.
- **설치 모드 (2종):** 기본은 **자족(self-contained)** — `<TARGET>/`가 게놈 전체를 담는 독립 디렉토리. **`--in-project`** — `<TARGET>`가 **기존 호스트 프로젝트**일 때 idoforgod식 **오버레이 설치**: `.claude/`에 런타임 DNA만 얹고(.claude/hooks 갱신 + agents/skills/config 비클로버 union), 게놈 헌법·docs는 `.harness/genome/`로 **재배치**, 호스트 루트 파일(`CLAUDE.md`/`AGENTS.md`/`README.md`/`soul.md`)은 **보존**(host CLAUDE.md엔 포인터만 append), `prompt-runner`/`prompt`/`translations`는 미설치, 로그 디렉토리는 `.harness/` 하위. 설치모드는 `.harness/GENOME.json`의 `install_mode`에 stamp되어 `validate`가 자동 분기.

**실행 명령 (그대로 사용):**
```bash
TR="${CYS_HARNESS_HOME:-$HOME/.cys/harness-creator}"
python3 "$TR"/warrant.py --predicates <TARGET>/.harness/predicates.json   # PRE: 분류 게이트
python3 "$TR"/audit_harness.py        <TARGET>                            # RESEARCH R1: 상태감사 (new/extend/maintain + drift)
python3 "$TR"/warrant.py --graph      <TARGET>/.harness/graph.json        # PLANNING P4: 비용밴드 (→ 사람 승인)
python3 "$TR"/emit_orchestrator.py    <TARGET> [--in-project]             # IMPL I3: graph→orchestrator+agents+게놈+메모리store (오버레이는 --in-project)
python3 "$TR"/validate_harness.py     <TARGET>                            # IMPL I5: 빌드 게이트(install_mode 자동 감지)
# 진화: python3 "$TR"/evolve_harness.py <TARGET> --type <유형> --change "..." --reason "..."
```
> `emit_orchestrator.py`가 **`emit_domain_skill.py`(도메인 스킬) + `inherit_genome.py`(게놈+`.harness/memory/` 시드)를 자동 호출**한다. `emit_workflow.py`는 **공장내부 측정 전용**(제품 emit 아님).
> **실행 핸드오프**: 산출 후 하네스를 *실행*하려면 `cd <TARGET> && claude`로 **새 세션**을 열어야 그 세션 settings.json hook이 발화한다(공장 세션이 아님).

## 워크플로우 — 4 스테이지 (AWF Research→Planning→Implementation→Evolution ⊃ idoforgod 단계 ⊃ CYS 게이트)

### PRE: Warrant 게이트 (필요한가?)
5 술어 `{distinct_expertise_domains, has_dependent_or_parallel_stages, will_be_rerun, output_objective, noisy}` → `warrant.py --predicates`. `answer-directly`/`single-agent` → **종료**. `build-harness(topology, decision_mechanism, n_agents)` → RESEARCH.
> **결정론 경계**: `classify()`는 5 술어→{off-ramp 판정, topology, mechanism, n_agents}을 **결정론적으로** 매핑한다(LLM 재추론 금지 — 술어 추출만 LLM). 단 classify의 topology는 `pipeline/dispatch/producer-reviewer` 3종 **coarse 제안**이다. RESEARCH/PLANNING에서 LLM이 `architecture-patterns.md`의 7-토폴로지×4-메커니즘 매트릭스로 실제 graph를 저작하며(제안을 넘어설 수 있음), `validate_harness.py`가 graph의 **내부 일관성**(topology↔노드구조·메커니즘 파라미터·티어)을 강제한다(warrant 제안과의 등치가 아님). 술어를 `<TARGET>/.harness/predicates.json`에 저장하면 P4의 `warrant --graph`가 같은 술어로 **이 도메인의** verdict를 `warrant.json`에 기록한다.

### STAGE 1 — RESEARCH (수집·분석; 사람 게이트 없음)
- **R0 빌드 회상 (장기기억 층위 2 — self-host)** — graph 저작 전, 팩토리 자신의 빌드 메모리를 회상한다. **회상 키는 LLM이 토큰을 추정하지 않고 결정론적으로 계산**한다 — `KEY=$(python3 "$TR"/lib/query_norm.py "<도메인 한 문장>")` (쓰기측 `bootstrap_factory_memory`가 저장하는 `query_norm`과 **동일 함수**) → `Grep "$KEY" "$TR"/.harness/memory/runs/index.jsonl`로 유사 도메인의 과거 빌드를 찾고, **매치된 build_id만** `runs/<build_id>/`(있으면)와 `domain-knowledge.yaml`을 Read해 topology·tier·mechanism 패턴을 R3/P1 graph 저작에 주입한다(맹신 금지 — 현재 요건에 맞게 검증; 무매치/콜드면 "선례 없음"으로 진행 — 회상 miss는 안전한 cold-start이며 산출을 오염시키지 않는다). 이는 산출 하네스 오케스트레이터 Phase 0 회상(emit이 `query_norm(harness_name)`을 read·write 양쪽에 리터럴 baking)과 **동일 정규화기를 dogfood하는 팩토리 self-host**다(부트스트랩: `python3 "$TR"/bootstrap_factory_memory.py`). **입력 비대칭 주의(설계 선택)**: 쓰기측은 `query_norm(harness_name)`(짧은 슬러그)를 저장하므로, R0가 긴 도메인 *문장* 전체를 키로 쓰면 substring grep이 과특정되어 짧은 슬러그를 놓칠 수 있다 — 다중어 도메인은 핵심 토큰(예상 harness_name 어휘)으로 좁혀 grep하거나 무매치를 안전한 cold-start로 받아들인다(precision-over-recall; 오염 없음). 산출 하네스 측은 read=write 키가 `query_norm(harness_name)`로 byte-동일이라 비대칭이 없다.
- **R1 상태감사** — `audit_harness.py <TARGET>` → `.harness/audit.json {branch ∈ new/extend/maintain, drift[]}`. drift = 디스크 agents/skills ↔ graph 계약의 결정론 set-diff(idoforgod는 산문, CYS는 검증 가능한 사실).
- **R2 도메인 분해** — 노드(id, 역할, inputs/outputs, write_paths), 사용자 숙련도 감지, 기존 에이전트/스킬 중복검토(R1 인벤토리 대조).
- **R3 토폴로지+모드 분석** — 7 토폴로지(pipeline/dispatch/fan-out-fan-in/producer-reviewer/supervisor/expert-pool/hierarchical) + `execution_mode`(team 기본/hybrid). *분석만* — 아직 graph 미작성.
- **R4 모델티어** — 역할 → `model-tier-policy.js` role-class → tier. `n_agents ≤ MAX_FANOUT(5)`.

### STAGE 2 — PLANNING (계약 저작 + 단일 사람 승인)
- **P1 graph.json 저작 (단일 진실원천)** — graph.json은 하네스의 **단일 진실원천(SoT)**이며 이 스킬이 저작한다(emit가 `graph.lock`으로 provenance를 stamp → 사후 손편집은 `GRAPH_PROVENANCE` warn). validate가 내부 일관성을 강제한다. `graph.schema.json` 준수. 노드별: `model, decision_mechanism, mechanism_params, output_schema, write_paths` + **`skill_authoring{mode:inline|skill, reason}`**(하이브리드, locked-5) + `review{agent}`. top: `execution_mode`(team/hybrid), `topology`(7), `budget`.
- **P2 schema 저작** — node.output_schema → `schemas/<name>.json`(draft 2020-12, bare-filename $id, additionalProperties:false).
- **P3 팀 아키텍처 확정** — topology + execution_mode + 팀 구성/역할 확정 (all-6 floor 충족).
- **P4 비용밴드** — `warrant.py --graph` (team-aware).
- **P5 ⛔ 사람 승인 게이트 (AWF + CYS)** — 계획(토폴로지·에이전트·`skill_authoring`·R1 드리프트) + 비용밴드를 제시 → 사용자 승인 대기. **Implementation은 승인 전 실행 금지.**

### STAGE 3 — IMPLEMENTATION (실제 발화하는 하네스 emit)
- **I1 agent 생성** — node → `.claude/agents/<agent>.md`: frontmatter(name/description/**model**/**model_rationale**/tools=least-privilege/maxTurns) + 본문(역할/원칙/입출력[정확한 _workspace 경로 + schema]/에러핸들링/팀-통신). model은 **티어 해석**(전체 opus 아님 — `TIER_OVERSPEND` 강제).
- **I2 도메인 스킬 (하이브리드)** — `emit_domain_skill.py`: `skill_authoring.mode='skill'` 노드만 `.claude/skills/<harness>-<id>/SKILL.md`(how, pushy description). `inline`은 agent 본문. (emit_orchestrator가 자동 호출.)
- **I3 오케스트레이터 + emit** — `emit_orchestrator.py <TARGET>`: `<domain>-orchestrator/SKILL.md`(execution_mode로 분기 — **team이면 실제 TeamCreate/TaskCreate(deps)/SendMessage/TeamDelete emit**, 메모리 운영·진화 섹션 포함) + agent stamps + **게놈전수 + 메모리 store init** + `RUNTIME.json`(orchestrator-canonical) + `CLAUDE.md` 포인터. **`workflow.js` 미emit.**
- **I4 게놈 발화** — `inherit_genome.py`(자동): hooks를 자식 `settings.json`에 배선 → Context Preservation(스냅샷/복원)·**L0-L2(`qa_gate_runner`)·budget(`budget_block`+`spawn_counter`)·SOT(`sot_init`)·적대적 reviewer/fact-checker**가 실세션에서 *발화*. prompt-runner는 산출 하네스에서 제외(self-contained: `_NONPRIMITIVE_EXCLUDES`+`_purge`; in-project: 미설치) — `PROMPT_RUNNER_ABSENT`가 잔존을 error로 차단(게놈 source에만 vendored-but-inert).
- **I5 검증 게이트** — `validate_harness.py <TARGET>` → **error 시 생성 중단·보고**(고치고 재실행).
- **I6 측정·테스트** — **lift 게이트**(skill 노드): `lift_gate.py emit-probe`로 probe 생성 → 런타임 실행(with-skill sonnet vs baseline haiku, blind opus grader) → `lift_gate.py score <results> --out <skill>/lift_verdict.json`. validate가 **측정 미실시=`LIFT_UNMEASURED`**(constants 정책, 기본 warn→error 전환가능)·**측정했으나 baseline 미달=`LIFT_REFUSED`(hard error)**로 게이트. + 트리거 near-miss(should/should-NOT), dry-run(dead-link).

### STAGE 4 — EVOLUTION (실행 + 학습; idoforgod Phase 7 + CYS 측정)
- **E0 빌드 기록 (장기기억 층위 2 — self-host)** — 빌드 완료 후 이 빌드 경험을 팩토리 빌드 메모리에 단일쓰기로 기록한다: 임의 산출 하네스는 `bootstrap_factory_memory.record_build(graph, root="$TR")`로 `.harness/memory/runs/index.jsonl`에 1줄 추가(idempotent), examples는 `python3 "$TR"/bootstrap_factory_memory.py`. 재사용 가능한 설계 패턴(domain→topology/tier/mechanism)은 `domain-knowledge.yaml`에, 빌드 교훈·안티패턴은 `risk/decisions.jsonl`에 병합한다. 다음 빌드의 **R0 회상**이 이를 활용한다(회상→기록 사이클 폐합).
- **E1 git** — `git init && git add -A && git commit`.
- **E2 head-to-head (선택)** — `evals/<domain>.scorecard.json`(discriminating-assertion 테스트 정의)로 C2(이 하네스) vs C3(no-harness) n-run 실행 → `evals/<domain>.runs.json` → `h2h_aggregate.py` → `<domain>.verdict.json`(median, 15pp margin). 8 use case parity는 `eval_topology.py`. `MEASUREMENT_DRIFT`가 정직성 강제(현재 stamped: **n=5 +12.5pp INCONCLUSIVE — CYS 우세, 마진 미달**).
- **E3-E6 진화** — `evolve_harness.py`: 피드백 유형→대상 라우팅 + `.harness/change-history.jsonl`(append-only) + `--proactive`(같은 유형 2회↑ 자동 제안) + 유지보수(재감사→한 번에 하나 수정→재검증→CLAUDE.md 동기화). 라우팅된 수정은 해당 Implementation 단계 재진입 후 **validate 재통과**(진화가 계약을 퇴행시키지 못함).

## 산출 체크리스트
- [ ] `.harness/graph.json`(schema 통과) — **`workflow.js` 없음**(은퇴)
- [ ] `.claude/agents/*.md`(model + rationale + least-priv tools) · review 노드면 reviewer/fact-checker 존재
- [ ] `skill_authoring=skill` 노드의 `.claude/skills/<harness>-<id>/` + `<domain>-orchestrator` skill
- [ ] orchestrator SKILL: team이면 **TeamCreate/SendMessage 실제 emit**, **메모리 운영·진화** 섹션 포함, graceful-degrade 명시
- [ ] `validate_harness.py` **PASS 0/0** — all-6·메모리store·QA hook·런타임매니페스트 clean
- [ ] `.harness/memory/`(Tier II) + `.claude/context-snapshots/`(Tier I) + `.harness/state.yaml`(SOT)
- [ ] warrant 비용밴드 **사람 승인** 후 실행, `git init`
- [ ] 절대경로 없음 · model:opus 전역 아님

## 도구 (cys-harness-creator/)
- `warrant.py` — PRE 게이트 + 비용밴드 · `role-class-policy.json` — role→tier **단일 SoT**(`model-tier-policy.js`·`validate_harness.py`가 둘 다 로드 → 드리프트 불가) · `model-tier-policy.js` — role-class 해석(`resolveModel`) · `graph.schema.json` — 계약 스키마 · `lib/toposort.py` — 결정론 토소트
- `audit_harness.py` — R1 상태감사 · `emit_orchestrator.py` — I3 emit(team/agent/hybrid) · `emit_domain_skill.py` — I2 도메인 스킬(하이브리드) · `inherit_genome.py` — I4 게놈+메모리 전수 · `validate_harness.py` — I5 게이트(49 머신체크 코드)
- `lift_gate.py` · `h2h_aggregate.py` · `eval_topology.py`(8 use case parity) — E2 측정 · `evolve_harness.py` — E3-E6 진화
- `templates/hooks/` — `gate_or_block`·`budget_block`·`spawn_counter`·`sot_init`·`qa_gate_runner`(런타임 발화)
- `emit_workflow.py` — **공장내부 측정 전용**(제품 emit 아님) · `examples/` — 4 작동 예시(team 모드)

## 참고 — references/ (8-파일 맵; 필요 시 Read; **`IMPLEMENTATION-STATUS.md`가 모든 서술에 우선**)
1. `IMPLEMENTATION-STATUS.md` — **먼저 읽기.** 실구현/연기/폐기 현황(은퇴 workflow.js 포함). 다른 모든 서술에 우선.
2. `architecture-patterns.md` — R3/P3: 7 토폴로지×4 메커니즘 매트릭스, execution_mode, 에이전트 분리 4축, 8 use case→토폴로지, 실제 예시(examples 흡수).
3. `graph-and-orchestration.md` — P1/I3: graph.json 저작 + 노드필드(skill_authoring/review) + schema + 실제 team emit(TeamCreate/TaskCreate/SendMessage) + 데이터전달 + CLAUDE.md 포인터.
4. `skill-and-agent-authoring.md` — I1/I2: agent `.md`(frontmatter·team-comms) + 하이브리드 author-or-inline + pushy description + Why-not-ALWAYS + progressive disclosure + output_schema.
5. `genome-and-runtime.md` — I4: 상속 DNA가 hook으로 *발화*하는 방식(Context Preservation·L0-L2·budget·SOT·보안) + 2 설치모드(자족/in-project 오버레이) + prompt-runner 제외 + RUNTIME.json.
6. `evolution-and-memory.md` — Stage 4: Phase-7 진화 루프(피드백 라우팅·change-history·proactive·유지보수) + 2-tier 메모리(Context Preservation + RLM 교차실행 store).
7. `testing-and-measurement.md` — I6/E2: validate 코드 카탈로그·lift 게이트(score --out→LIFT_REFUSED)·h2h(n=5 +12.5pp INCONCLUSIVE, drop-not-zero)·8 use case eval·트리거 near-miss·dry-run.
8. `qa-guide.md` — QA 경계교차(존재확인 아님)·증분 QA·7 버그패턴·verify-before-assert·L1/L2 게이트 hook 매핑.
