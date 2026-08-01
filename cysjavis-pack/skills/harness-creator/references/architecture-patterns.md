> ⚠️ **구현 현황은 [`IMPLEMENTATION-STATUS.md`](IMPLEMENTATION-STATUS.md)가 우선한다.** 이 문서가 무엇을 설명하든, 실제 emit/validate에 들어갔는지는 그 문서로 확정한다. (이 문서는 `examples.md`를 흡수한 통합본이다 — 8 use case·3 실제 예시·설계 매트릭스가 한 파일에 있다.)

# 아키텍처 패턴 — Topology × Decision-Mechanism 설계 + 산출 예시

> ✅ **실행 기질(2026-05-31 기준)**: 산출 하네스의 canonical 실행모델은 **Claude Code 프리미티브**다. `emit_orchestrator.py`가 `graph.json` → **오케스트레이터 SKILL.md** + `.claude/agents/<agent>.md`(frontmatter의 model·tools·maxTurns를 Agent 도구가 **런타임 강제**)를 emit하고, 라이브 호스트 세션이 `Agent`(agent) / `TeamCreate`·`SendMessage`(team·**hybrid** — hybrid는 P0-2 이후 team과 동일하게 team 프리미티브를 emit)로 그래프를 돈다(미구현은 hybrid의 Phase별 기질 *혼합* emit뿐 — IMPLEMENTATION-STATUS.md 우선). 이 기질에서만 상속된 AWF 게놈(lifecycle hook·L0-L2 게이트·SOT·적대적 리뷰)이 실제로 발화한다.
>
> **Mode-A `workflow.js`는 제품에서 은퇴**(`WORKFLOW_RETIRED`)했다. `emit_workflow.py`·`h2h_suite.workflow.js`·lift 프로브는 **공장 내부 측정 전용 도구**로만 살아남는다 — 산출 하네스의 런타임이 아니다. 이 문서에서 `workflow.js`가 등장하는 곳은 전부 "공장 내부 측정"으로 라벨된 자리뿐이다.

> 출처: 원본 `agent-design-patterns.md`(team-vs-subagent 6패턴 모델)을 CYS 패러다임으로 적응 — **7 topology × 4 decision-mechanism 매트릭스 + agent/team/hybrid 실행기질 + graph.json 계약 + 머신체크 게이트**. 8 use case·3 실제 예시는 원본 `team-examples.md`을 흡수한 것이다.

이 문서는 **도메인 한 문장 → `graph.json`** 을 저작할 때 "어떤 구조로 짤 것인가"를 결정하는 설계 두뇌다. 원본의 설계 지혜(에이전트 분리 4축, 팀 크기 가이드, 복합 패턴 사고)는 보존하되, CYS의 계약 우선·머신체크·역할티어·프리미티브 기질 패러다임으로 재배선했다.

---

## 목차

1. [핵심 패러다임 전환](#1-핵심-패러다임-전환)
2. [graph.json — 모든 구조의 계약](#2-graphjson--모든-구조의-계약)
3. [두 직교 축: Topology × Decision-Mechanism](#3-두-직교-축-topology--decision-mechanism)
4. [Topology (7) — 데이터 흐름 축](#4-topology-7--데이터-흐름-축)
5. [Decision-Mechanism (4) — 조율 이론 축](#5-decision-mechanism-4--조율-이론-축)
6. [Topology × Mechanism 합성 매트릭스](#6-topology--mechanism-합성-매트릭스)
7. [원본 6패턴 → CYS 매핑](#7-원본-6패턴--cys-매핑)
8. [execution_mode 선택 (agent / team / hybrid)](#8-execution_mode-선택-agent--team--hybrid)
9. [도메인 → topology+mechanism 선택 알고리즘](#9-도메인--topologymechanism-선택-알고리즘)
10. [에이전트 분리 기준 (4축 — 원본 지혜 보존)](#10-에이전트-분리-기준-4축--원본-지혜-보존)
11. [에이전트 타입·모델 티어 선택](#11-에이전트-타입모델-티어-선택)
12. [팀 크기 / fan-out 가이드](#12-팀-크기--fan-out-가이드)
13. [복합 패턴 사고](#13-복합-패턴-사고)
14. [idoforgod 8 use case → topology 배정](#14-idoforgod-8-use-case--topology-배정)
15. [실제 예시 A — deep-research (pipeline + reflect-then-revise)](#15-실제-예시-a--deep-research-pipeline--reflect-then-revise)
16. [실제 예시 B — ticket-triage (dispatch + majority-vote)](#16-실제-예시-b--ticket-triage-dispatch--majority-vote)
17. [실제 예시 C — design-decision (producer-reviewer + debate-with-judge)](#17-실제-예시-c--design-decision-producer-reviewer--debate-with-judge)
18. [세 예시 교차 비교 + 보존된 설계 지혜](#18-세-예시-교차-비교--보존된-설계-지혜)
19. [산출물 패턴 요약 — graph.json이 척추다](#19-산출물-패턴-요약--graphjson이-척추다)
20. [외부 출처 근거 (학계·표준 1:1 대조)](#20-외부-출처-근거-학계표준-11-대조)

---

## 1. 핵심 패러다임 전환

원본은 "에이전트 팀이 기본, `.claude/agents/`를 직접 쓴다, 6개 패턴 중 하나를 고른다, 모두 opus"를 가르쳤다. CYS는 이를 다음으로 대체한다:

| 원본 (idoforgod/harness) | CYS Harness Creator |
|---|---|
| `.claude/agents/`를 계약 없이 직접 저작 | **먼저 `graph.json`(불변 계약)을 저작** → 도구가 컴파일·검증 |
| 6개 패턴(데이터 흐름 한 축) | **7 topology × 4 decision-mechanism** (두 직교 축) |
| 단일 실행모델(팀) | **team(빌드 기본) / hybrid / agent(개념 기질) 프리미티브 기질** — 상속된 AWF 게놈이 발화하는 곳(빌드 하네스는 A2 floor상 team/hybrid) |
| 모든 에이전트 `model: opus` | **role→tier 정책** (gather/extract/format/qa-scan=haiku, voter/debater/reviser=sonnet, synthesis/judge/critic/architecture=opus) |
| advisory prose 규칙 | **머신체크 게이트** (`validate_harness.py` 머신체크 세트가 위반 시 빌드 실패) |

> **불변 원칙:** 구조 선택은 산문이 아니라 `graph.json` 필드값으로 확정된다. `topology`·`decision_mechanism`·`model`·`execution_mode`은 모두 스키마 enum이며, `validate_harness.py`가 강제한다. "이렇게 하는 게 좋다"는 없다 — 게이트를 통과하거나 빌드가 실패하거나 둘 중 하나다.

---

## 2. graph.json — 모든 구조의 계약

모든 하네스는 단 하나의 `graph.json`(불변 spine, `graph.schema.json`으로 검증)으로 표현된다. 메타스킬만이 이 파일을 쓴다(single-writer). 구조 설계 = 이 파일의 필드를 채우는 일이다. agent 파일·schema 파일·오케스트레이터 SKILL은 **모두 graph.json에서 파생된다**.

```jsonc
{
  "schema_version": "0.1",
  "harness_name": "deep-research",            // ^[a-z][a-z0-9-]{1,48}[a-z0-9]$
  "harness_version": "0.1.0",
  "execution_mode": "team",                   // workflow | team | agent | hybrid
  "topology": "pipeline",                     // pipeline | dispatch | fan-out-fan-in | producer-reviewer | supervisor | expert-pool | hierarchical
  "budget": { "total_tokens": 600000, "approval_required": true },
  "nodes": [ /* … */ ],
  "edges": [ { "from": "gather", "to": "fetch" } ]  // ORDERING ONLY — depends_on 그래프 아님
}
```

**node 구조** (각 노드 = 그래프의 한 논리 단계):

```jsonc
{
  "id": "verify",                             // ^[a-z][a-z0-9_]{0,30}[a-z0-9]$
  "agent": "verifier",                        // -> .claude/agents/verifier.md
  "model": "sonnet",                          // haiku|sonnet|opus (REQUIRED=하드에러; agent frontmatter의 model_rationale은 emit가 항상 채움, 누락 시 RATIONALE_MISSING warn)
  "decision_mechanism": "reflect-then-revise",// single | majority-vote | debate-with-judge | reflect-then-revise
  "mechanism_params": { "max_rounds": 2, "critic": "opus" },
  "inputs": ["_workspace/02_fetch/findings.json"],
  "outputs": ["_workspace/03_verify/findings.json"],
  "write_paths": ["_workspace/03_verify/"],   // 노드 간 write_path 중첩 금지 (게이트)
  "output_schema": "schemas/findings.json",   // JSON-Schema 파일 (강제 출력)
  "retries": 0,
  "on_exhaust": "proceed-with-gap",           // proceed-with-gap | force-pass | escalate
  "max_rounds": 2,
  "review": { "agent": "reviewer" },          // (선택) 이 노드 후 L2 적대적 리뷰 agent를 일급 게이트로 spawn
  "skill_authoring": { "mode": "inline" }     // (선택) M3 하이브리드: mode=skill이면 .claude/skills/<harness>-<id> 저작
}
```

핵심 의미론:
- **`edges`는 순서(ordering)만 표현한다.** pipeline 스케줄링/parallel fan-out/loop 경계를 정의할 뿐, TaskCreate식 `depends_on` 의존성 그래프가 아니다. (다만 team 모드에서 오케스트레이터가 `TaskCreate(depends_on=…)`로 변환할 때 edges가 의존 정보로 *재사용*된다 — 아래 §16/§8.)
- **`topology`는 graph 전체의 형태**, **`decision_mechanism`은 노드 단위**다. 그래서 한 pipeline 안에서 노드마다 다른 mechanism을 쓸 수 있다(예: deep-research의 verify=reflect-then-revise, 나머지=single).
- 컴파일: `emit_orchestrator.py`가 `graph.json` → **오케스트레이터 SKILL.md**(프리미티브 실행 레시피: `Agent()` spawn / `TeamCreate`·`TaskCreate`·`SendMessage`·`TeamDelete` / 노드별 mechanism 확장) + 노드마다 `.claude/agents/<agent>.md`(model·tools·maxTurns 런타임강제 frontmatter)를 렌더한다.
- 이 계약은 **조언이 아니라 기계 게이트로 강제된다**: `validate_harness.py`(스키마·agent 파일 존재·model 티어·엣지 무결성·사이클·write-path 중복·절대경로·schema 파일 존재·genome 존재·runtime 선언·doc-drift), `warrant.py`(Phase -1 비용 게이트). 역할→티어 정책의 **단일 SoT는 `role-class-policy.json`**이며, `model-tier-policy.js`와 `validate_harness.py`가 **둘 다 이 파일을 로드**한다(손복사 in-Python 미러 없음·JS shell-out 없음·동기화 규칙 불필요 — 같은 파일을 읽으므로 둘이 드리프트할 수 없다; `tests/test_factory.py`의 `TestTierPolicyMirror`가 ids×agents×mechs 전수곱 코퍼스로 JS≡Py 분류 동치를 보증).

---

## 3. 두 직교 축: Topology × Decision-Mechanism

원본의 가장 큰 한계는 "데이터 흐름"이라는 **한 축**으로만 패턴을 나눈 것이다(파이프라인/팬아웃/풀/생성검증/감독자/계층). CYS는 이를 **두 직교 축**으로 분해한다:

```
                  DECISION-MECHANISM (노드가 결정을 내리는 방법 — 조율 이론 축)
                  single        majority-vote   debate-with-judge   reflect-then-revise
                ┌────────────┬────────────────┬──────────────────┬─────────────────────┐
  T  pipeline   │  순차 단순   │  단계 내 투표    │  단계 내 토론      │  단계 내 반복 정제     │
  O  dispatch   ├────────────┼────────────────┼──────────────────┼─────────────────────┤
  P  fan-out…   │  병렬 단순   │  병렬+투표 합성  │  병렬+토론 합성    │  병렬+sink 정제       │
  O  supervisor ├────────────┼────────────────┼──────────────────┼─────────────────────┤
  L  expert-pool│  라우팅 단순 │  …             │  …               │  …                  │
  O  producer-  ├────────────┼────────────────┼──────────────────┼─────────────────────┤
  G  reviewer   │  생성→검수   │  (드묾)        │  생성↔검수 토론     │  생성→비평→수정 루프   │
  Y  hierarch…  └────────────┴────────────────┴──────────────────┴─────────────────────┘
  (데이터가 노드 사이를 흐르는 형태 — 데이터 흐름 축)
```

- **Topology**는 "노드들이 시간·공간상 어떻게 배치되는가"(순차/병렬/루프/라우팅/계층).
- **Decision-Mechanism**은 "한 노드가 답을 어떻게 만드는가"(혼자/투표/토론/반복정제).
- 두 축은 **합성 가능(composable)**하다. topology를 고른 뒤, 그 안의 각 노드에 mechanism을 독립적으로 부여한다.

> 이 직교성이 원본 대비 CYS의 핵심 표현력이다. 원본의 "생성-검증"은 *데이터 흐름이면서 동시에 조율 방법*이라 두 개념이 엉켜 있었다. CYS는 이를 분리한다: producer-reviewer(topology) + reflect-then-revise(mechanism)는 별개이며 따로 조합할 수 있다.

---

## 4. Topology (7) — 데이터 흐름 축

`topology` enum은 7개다(`graph.schema.json`): `pipeline | dispatch | fan-out-fan-in | producer-reviewer | supervisor | expert-pool | hierarchical`. 이 중 `fan-out-fan-in`·`supervisor`·`expert-pool`·`hierarchical`은 `emit_orchestrator.py`의 `_topology_addendum`이 **각자의 Phase-2 프리미티브 레시피를 first-class로 emit**하며(`TOPOLOGY_PRIMITIVE_CONSISTENCY` 게이트), `eval_topology.py`가 `### 토폴로지: <hdr>` 마커 존재를 머신체크한다.

### 4.1 pipeline (순차)

이전 노드의 출력이 다음 노드의 입력. `edges`가 단일 선형 체인.

```
[gather] → [fetch] → [verify] → [synthesize]
```

- **적합:** 각 단계가 이전 단계 산출물에 강하게 의존하고, 순서가 의미를 가질 때.
- **예시:** deep-research(수집→페치→검증→합성 — 단, 8-use-case에서 Deep Research는 fan-out-fan-in으로 배정됨, §14), Website Development·Technical Documentation, 소설 집필(세계관→캐릭터→플롯→집필→편집).
- **graph.json:** `topology: "pipeline"`, edges는 선형 체인.
- **주의 (원본 지혜 보존):** 병목 한 단계가 전체를 지연시킨다. 각 단계를 가능한 독립적으로 설계해 재실행 시 변경 노드부터만 재개되게 한다(semantic-resume — SOT `state.yaml` + 단계 산출물 파일).

### 4.2 dispatch (병렬 fan-out + 단일 sink)

분배 노드가 독립 작업을 병렬로 펼치고, 단일 sink 노드가 결과를 통합.

```
          ┌→ [expert_a] ─┐
[dispatch]├→ [expert_b] ─┼→ [synthesize] (단일 sink)
          └→ [expert_c] ─┘
```

- **적합:** 동일 입력에 서로 다른 관점/영역의 처리가 필요하고, 독립 실행 가능하며 작업 수가 **설계 시 고정**될 때.
- **예시:** ticket-triage(category·priority 동시 분류 → route sink), 종합 리서치(공식/미디어/커뮤니티/배경 동시 조사 → 통합).
- **graph.json:** `topology: "dispatch"`, edges가 분배→워커들→sink. fan-out 폭 ≤ `MAX_FANOUT(5)`.
- **주의 (원본 지혜 보존):** **단일 sink의 품질이 전체 품질을 결정한다.** sink는 거의 항상 synthesis role-class → opus.

> **dispatch vs fan-out-fan-in:** dispatch는 "병렬 워커 + 단일 sink"의 일반형이다. `fan-out-fan-in`은 그 중 **팀 프리미티브(`TeamCreate` + 무의존 `TaskCreate` + `SendMessage` 상충공유)로 병렬 실행하고 Lead가 합성**하는 협업형을 별도 first-class 토폴로지로 둔 것이다(§4.3). 작업이 단순 독립이고 sink가 결정론 병합이면 dispatch, 워커들이 중간 산출을 실시간 공유하며 협업하면 fan-out-fan-in.

### 4.3 fan-out-fan-in (병렬 수집 → 합성, 팀 협업)

`_topology_addendum` 레시피(`### 토폴로지: fan-out/fan-in`): 독립 작업을 팀(`TeamCreate` + 무의존 `TaskCreate`)으로 병렬 실행하고 `SendMessage`로 상충을 공유한다. Lead가 `_workspace/`에서 결과를 수집해 합성 sub-agent로 통합한 뒤 `TeamDelete`.

- **적합:** 여러 워커가 같은 입력을 병렬로 처리하되 **중간 발견을 실시간 공유**하는 것이 품질을 높일 때.
- **예시(8-use-case):** Deep Research, Code Review & Refactoring — 둘 다 fan-out-fan-in + team.
- **graph.json:** `topology: "fan-out-fan-in"`, 보통 `execution_mode: "team"`.

### 4.4 producer-reviewer (경계 루프)

생성 노드와 검수 노드가 쌍을 이뤄 품질 기준 충족까지 경계된 횟수만큼 반복.

```
[produce] → [review] →(문제시)→ [produce] 재실행  (max_rounds로 경계)
```

- **적합:** 산출물 품질 보장이 중요하고 객관적 검증 기준이 존재할 때.
- **예시:** design-decision(propose↔adjudicate), Webtoon(artist 생성 → reviewer 검수 → 문제 패널 재생성), Marketing Campaign, 코드 생성→린트→수정.
- **graph.json:** `topology: "producer-reviewer"`, 두 노드 + 루프 edge. **무한 루프 방지:** producer의 `max_rounds`가 재제안 상한(스키마가 노드 `max_rounds` max 3 강제), reviewer가 반환하는 verdict(`approved`)가 종료 조건. 루프는 graph.json에 별도 노드로 보이지 않고 **topology가 곧 제어 흐름**이다.
- **CYS의 정련:** producer-reviewer는 종종 **단일 노드의 reflect-then-revise mechanism으로 압축**된다(§5.4). 두 에이전트(producer/reviewer)가 진짜로 다른 전문성·도구를 가질 때만 topology로, 같은 에이전트의 비평·수정 두 패스면 단일 노드 + reflect-then-revise가 더 단순하다.

### 4.5 supervisor (동적 작업 할당)

`_topology_addendum` 레시피(`### 토폴로지: supervisor`): Team Lead가 **supervisor**로서 초기 `TaskCreate` 배치를 만들고 팀원이 self-claim한다. 각 `TaskUpdate(status=completed)` 시 Lead가 결과를 보고 **런타임에 다음 배치 `TaskCreate`를 동적 발행**한다(정적 fan-out과 달리 작업이 동적으로 추가됨). 모든 작업 소진 시 종합 + `TeamDelete`.

- **적합:** 작업량이 가변적이고 런타임에 배치가 결정될 때(예: 파일 N개 마이그레이션, 가변 길이 콘텐츠 기획).
- **예시(8-use-case):** YouTube Content Planning — supervisor + team.

### 4.6 expert-pool (상황별 전문가 라우팅)

`_topology_addendum` 레시피(`### 토폴로지: expert-pool`): 먼저 **라우터 노드**(`Agent`, haiku/sonnet)가 입력을 분류한다. 오케스트레이터는 분류 결과에 따라 **매칭된 전문가만** `Agent(subagent_type=<expert>)`로 조건부 spawn한다(모든 전문가를 항상 부르지 않음 — 비용 절감). 팀이 아니라 sub-agent 디스패치다.

- **적합:** 입력 종류가 다양하고 각 종류마다 다른 전문가가 필요하지만, 한 입력에는 일부 전문가만 관여할 때(문의 분류 후 매칭 전문가 호출).
- **graph.json:** `topology: "expert-pool"`, 라우터 노드 + 조건부 전문가 노드들. `execution_mode: "team"`(또는 hybrid) — 동작은 라우터+조건부 전문가의 sub-agent 디스패치지만, A2 floor(`ALL_PRIMITIVES_PRESENT`)상 빌드 하네스는 team/hybrid로 emit해야 한다(팀 미사용 시 sub-agent로 graceful-degrade).

### 4.7 hierarchical (2단계 위임, depth ≤ 2)

`_topology_addendum` 레시피(`### 토폴로지: hierarchical-delegation`): Level-1은 sub-coordinator들의 팀(`TeamCreate`). Level-2는 각 coordinator가 자신의 sub-agent를 `Agent()`로 spawn한다(팀원은 sub-agent를 spawn할 수 있으나 **팀은 중첩 불가**). **위임 깊이는 2로 제한**한다. (★CYSjavis 현행 규범: 마스터 헌장 제8조 "위임 깊이는 1단까지만 허용하며, 받은 일을 통째로 다시 넘기는 위임은 금지한다"가 우선한다 — 이 레퍼런스의 2단은 일반론이며 본 시스템에서는 **1단이 상한**이다 · 근거 `~/.cys/pack/directives/MASTER_CHARTER.md:78` · 2026-08-01 master)

- **적합:** 자연히 계층적이고 각 하위 조정자가 독립 하위 작업군을 관리할 때(대규모 데이터 파이프라인 설계).
- **예시(8-use-case):** Data Pipeline Design — hierarchical + team.
- **주의:** 깊이 ≤ 2 하드 제한(원본의 "3단계 이상은 지연·컨텍스트 손실" 경고 보존). 더 깊은 도메인은 평탄화하거나 2단계 sink 합성으로 근사. (★CYSjavis에서는 위 §4.7 본문 주석대로 **헌장 제8조의 1단이 상한**이므로 이 "≤ 2"는 적용되지 않는다 · 2026-08-01 master)

---

## 5. Decision-Mechanism (4) — 조율 이론 축

원본에 없던 새 직교 축. 한 노드가 답을 만드는 방법을 결정한다. `mechanism_params`로 파라미터화되고 스키마가 mechanism별 필수 필드를 강제한다(`majority-vote`→`n`+`quorum`, `debate-with-judge`→`max_rounds`+`judge`, `reflect-then-revise`→`max_rounds`+`critic`).

### 5.1 single

노드가 한 번의 에이전트 호출로 답을 낸다. 가장 단순·저렴.

- **params:** 없음.
- **언제:** 답이 결정적이거나 단일 전문가로 충분할 때(수집·페치·포맷·최종 합성 중 추가 검증 불필요한 경우, sink의 결정론 병합).
- **fan-out 비용:** 1.

### 5.2 majority-vote (병렬 투표)

n개 voter가 독립적으로 같은 문제를 풀고 다수결로 답을 확정.

- **params (필수):** `n`(2~5), `quorum`(다수 임계). 선택: `tie_break`(first | highest-confidence).
- **언제:** 답이 **객관적이지만 단일 패스가 노이즈에 취약**할 때(웹 사실 추출, 모호한 분류). 같은 정답을 여러 번 독립 추정해 분산을 줄인다.
- **role-class:** voter → sonnet. **독립성이 본질** — ballot 간 통신이 *없어야* 작동한다(병렬 spawn이 서로 못 봄).
- **emit:** 오케스트레이터가 `n`개 `Agent`를 병렬 spawn(독립 투표) → `quorum`으로 다수결 집계(`_spawn_recipe`).
- **fan-out 비용:** n.

### 5.3 debate-with-judge (토론 + 심판)

n명의 debater가 max_rounds 동안 논쟁하고, judge가 최종 판정.

- **params (필수):** `max_rounds`(1~3), `judge`(model tier). 선택: `n`.
- **언제:** 답이 **주관적·평가적**이고 관점 충돌에서 더 나은 답이 나올 때(설계 트레이드오프, 전략 평가, 정성 심사).
- **role-class:** debater → sonnet, judge → opus (mechanism_params로 별도 티어링).
- **emit:** n명 debater를 max_rounds 토론 후 judge(`<tier>`) Agent가 판정(`_spawn_recipe`).
- **fan-out 비용:** 2·max_rounds + 1.

### 5.4 reflect-then-revise (비평→수정 루프)

한 에이전트가 max_rounds 동안 critic 패스(결함 지목)와 reviser 패스(결함 수정)를 번갈아 수행. 단일 산출물을 반복 정제.

- **params (필수):** `max_rounds`(1~3), `critic`(model tier).
- **언제:** **단일 산출물을 점진적으로 정제**해 품질을 끌어올릴 때(사실 검증, 초안 정제). critic이 통과시키면(`approved=true`) 그 라운드에서 루프가 끊겨 reviser는 호출되지 않는다.
- **role-class:** reviser → sonnet, critic → opus (critic은 mechanism_params로 별도 티어링).
- **emit:** critic(`<tier>`) Agent가 적대적 비평 → reviser가 수정, max_rounds 반복, `approved=true`면 조기 종료(`_spawn_recipe`).
- **fan-out 비용:** 2·max_rounds.
- **정준 예시:** deep-research의 verify 노드 — 한 `verifier` 에이전트가 critic 패스(opus, 약한·오인용·과장 claim 지목)와 reviser 패스(sonnet, 수정/삭제)를 라운드마다 수행.

> **mechanism과 model의 결합:** mechanism이 노드의 base role을 override한다(`_role_class_of`). majority-vote→voter, debate→debater, reflect→reviser. judge/critic은 `mechanism_params`에서 따로 티어링된다. 미매핑 노드는 id+agent 키워드 정규식(`gather|fetch|search…` 등)으로 base role-class를 결정하고, 매칭 실패 시 fail-safe로 synthesis(opus)다.

---

## 6. Topology × Mechanism 합성 매트릭스

각 셀 = 한 노드(또는 sink)에 mechanism을 부여한 조합. **언제 쓰는가**를 명시한다(대표 topology 3종으로 압축; fan-out-fan-in/supervisor/expert-pool/hierarchical은 dispatch 행을 일반화한 변형이다).

| Topology \ Mechanism | single | majority-vote | debate-with-judge | reflect-then-revise |
|---|---|---|---|---|
| **pipeline** | 단계가 결정적, 추가 검증 불필요 (gather/fetch/format) | 한 단계가 노이즈 취약한 객관 추출 | 한 단계가 주관 평가 | **정준** — 한 단계가 정제 필요한 검증/초안 (deep-research verify) |
| **dispatch / fan-out-fan-in** | 워커들이 독립 수집, sink가 단순 병합 (ticket-triage route) | **각 워커가 같은 문제 투표 후 sink 합성** (ticket-triage classify ×2) | 워커들이 관점 충돌, sink/judge가 판정 | sink가 통합 산출물을 반복 정제 |
| **producer-reviewer** | 생성→1회 검수→통과/재생성 | (드묾 — 검수에 투표 필요할 때만) | **producer↔reviewer가 토론** (design-decision adjudicate) | **정준 압축** — 단일 노드로 융합 |

읽는 법:
1. 먼저 도메인의 **데이터 흐름**으로 topology를 고른다(§9 알고리즘, §14 8-use-case 배정).
2. 그 다음 **각 노드**의 답 생성 특성으로 mechanism을 고른다(객관/노이즈→majority-vote, 주관→debate, 단일산출물 정제→reflect).
3. 대부분 노드는 `single`이고, **품질이 임계인 1~2개 노드만** 비용이 높은 mechanism을 받는다(AC-1 품질 우선 + 비용 거버넌스 균형).

---

## 7. 원본 6패턴 → CYS 매핑

원본의 6패턴을 어떻게 흡수했는지 명시한다(마이그레이션·이해용). **전문가 풀·계층적 위임은 더 이상 deferred가 아니라 first-class 토폴로지**다(M2-2 / `_topology_addendum`).

| 원본 패턴 | CYS 매핑 | 비고 |
|---|---|---|
| **1. 파이프라인** | `topology: pipeline` | 그대로 |
| **2. 팬아웃/팬인** | `topology: fan-out-fan-in` (또는 정적 단순형은 `dispatch`) | 팀 병렬 + Lead 합성 |
| **3. 전문가 풀** | `topology: expert-pool` | 라우터가 분류 → 매칭 전문가만 조건부 spawn (first-class) |
| **4. 생성-검증** | `topology: producer-reviewer` **+** `decision_mechanism: reflect-then-revise` | 두 개념으로 분해 — 진짜 다른 두 에이전트면 topology, 비평·수정 두 패스면 단일 노드 mechanism |
| **5. 감독자** | `topology: supervisor` | 런타임 동적 `TaskCreate` 발행 (first-class) |
| **6. 계층적 위임** | `topology: hierarchical` | 2단계 위임, depth ≤ 2 (first-class) |

---

## 8. execution_mode 선택 (agent / team / hybrid)

`graph.json`의 `execution_mode`가 산출 하네스의 런타임을 정한다. enum은 4개(`workflow | team | agent | hybrid`)이나 **산출 하네스는 프리미티브 기질 3종(agent/team/hybrid)만 사용**한다 — `emit_orchestrator.py`가 이 셋을 처리하고, 그래야 상속된 AWF 게놈(lifecycle hook·L0-L2 게이트·SOT·적대적 리뷰)이 발화하며 `.claude/agents`의 model·tools가 런타임 강제된다. `workflow`는 enum에 남아있지만 **제품에서 은퇴**했다(아래).

| mode | 런타임 | emit | 언제 |
|------|--------|------|------|
| **`agent`** | Agent 도구 순차/병렬 sub-spawn | `emit_orchestrator.py` | 단순 sub-agent 디스패치(expert-pool 등). 부모/자식 hook 발화가 가장 확실 |
| **`team`** | TeamCreate 피어팀 + TaskCreate(deps) + SendMessage + TeamDelete | `emit_orchestrator.py` | 팀원이 중간 산출을 실시간 공유·조율할 때. 8 use case·3 예시 모두 team |
| **`hybrid`** | (현재) **team 레시피**를 emit (TeamCreate/TaskCreate(deps)/SendMessage/TeamDelete) | `emit_orchestrator.py` | **P0-2 이후 `hybrid`는 `team`과 같은 분기**(`emit_orchestrator.py`의 `if mode in ("team","hybrid")` 분기 → `_team_recipe`)로 실제 team 프리미티브를 emit한다 — A2 `ALL_PRIMITIVES_PRESENT` floor를 통과한다. **미구현은 Phase별 agent/team 기질 *혼합* emit뿐**(future work), team 프리미티브 emit 자체가 아니다 |
| `workflow` | ~~Workflow 도구 `workflow.js`~~ | ~~`emit_workflow.py`~~ | **은퇴(`WORKFLOW_RETIRED`).** 산출 런타임 아님 — `emit_workflow.py`/`h2h_suite.workflow.js`는 **공장 내부 측정 전용** 도구로만 잔존 |

> **팀 graceful-degrade (A2-iii):** team 모드라도 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` 플래그가 없으면 `_team_recipe`가 각 task를 `Agent(subagent_type=…)` fan + `_workspace/` 핸드오프로 강등한다 — 팀 없이도 동일 그래프가 실행된다.

### 선택 의사결정

```
실시간 inter-agent 협상(팀원이 서로의 중간 산출을 보며 조율)이 품질에 본질적인가?
├── No  → 개념상 agent로 충분 — 단, **빌드 하네스는 A2 floor상 team/hybrid 필수**(아래 주의)
├── Yes → execution_mode = team        ← TeamCreate 피어팀 (8 use case·3 예시 디폴트)
└── Phase별 혼합 → execution_mode = hybrid  ← 현재 emit는 **team 레시피**와 동일(P0-2); Phase별 기질 *혼합* emit만 future work
```
> **A2 floor 주의:** 순수 `agent`는 `TeamCreate(`를 emit하지 않아 `ALL_PRIMITIVES_PRESENT`(6 프리미티브 전부)를 **validate에서 실패**한다(`emit→validate` 재현됨). 따라서 *빌드되는* 하네스의 `execution_mode`는 **team 또는 hybrid**여야 한다 — 위 트리에서 "No"로 떨어져도 team/hybrid로 emit하고, 팀 미사용 시 sub-agent로 graceful-degrade한다(SKILL.md 원칙 2 · IMPLEMENTATION-STATUS 우선).

> **피벗의 핵심:** 초기 CYS는 "workflow.js가 기본"이었으나, 그 기질에서 AWF 게놈 전체가 휴면함이 실측됐다(두 실행평면 직교). 피벗 후 **프리미티브가 유일한 산출 기질** — AWF가 설계된 곳이고, 게놈이 실제로 발화하며, 모델티어가 런타임 강제된다. 디스크의 4개 예제 하네스(deep-research·ticket-triage·design-decision·competitor-watch)는 전부 `execution_mode: team`으로 마이그레이션됐다.

### RUNTIME 라우팅

생성 하네스의 `.harness/RUNTIME.json`(프리미티브 모드는 `emit_orchestrator`가 작성)은 **단 하나의 실행 런타임**을 선언한다:
- **CANONICAL = `<name>-orchestrator` SKILL** — `cd <harness> && claude`로 연 라이브 세션에서 이 스킬을 트리거. 그 세션의 settings.json hook이 발화한다(**공장 세션이 아님 — R4 핸드오프**). in-project 오버레이면 호스트 프로젝트의 `claude` 세션에서 트리거.
- 은퇴한 `workflow.js`와 상속된 `prompt-runner` subprocess는 **런타임으로 광고되지 않는다**(`RUNTIME_MANIFEST_CLEAN`). prompt-runner는 vendored-but-inert(실행에 wire되지 않음). 한 작업을 두 런타임으로 돌리지 않는다.

---

## 9. 도메인 → topology+mechanism 선택 알고리즘

`warrant.py`의 `classify()`가 5개 술어로 이 결정을 내린다. LLM은 도메인 한 문장에서 5개 술어를 *한 번* 추출하고, 게이트가 나머지를 한다.

### 5개 술어

```jsonc
{
  "distinct_expertise_domains": 4,          // 서로 다른 전문성 영역 수
  "has_dependent_or_parallel_stages": true, // 순차 의존 또는 병렬 단계가 있는가
  "will_be_rerun": true,                     // 재사용·재실행 되는가
  "output_objective": true,                  // 출력이 객관적(정답 존재)인가
  "noisy": false                             // 단일 패스가 노이즈에 취약한가
}
```

### 분류 로직 (classify)

**1단계 — 하네스가 필요한가 (off-ramp):**
```
domains < 2 AND no staged?
├── not rerun AND not noisy AND objective  → answer-directly  (하네스 없이 직접 답)
└── else                                    → single-agent     (단일 에이전트 1회)
else → build-harness (아래 2단계)
```

**2단계 — topology 선택(기본 제안):**
```
staged AND domains >= 2  → pipeline           (순차/병렬 다단계, 단계 간 의존)
domains >= 2 (단일 단계)  → dispatch/fan-out-fan-in  (다영역 병렬 fan-out + sink)
else                      → producer-reviewer       (단일영역 다단계 정제)
```
> classify는 3개 기본 topology를 제안한다. 도메인이 **동적 작업량**이면 supervisor, **조건부 전문가 라우팅**이면 expert-pool, **2단계 위임**이면 hierarchical로 메타스킬이 격상한다(§14의 8-use-case 배정이 그 실례).

**3단계 — decision_mechanism 선택 (FIRST match):**
```
not objective  → debate-with-judge   (주관 평가 — 관점 충돌이 품질을 만듦)
staged         → reflect-then-revise (순차 파이프라인 — 단일 산출물 반복 정제)
noisy          → majority-vote       (단일단계 객관+노이즈 — 병렬 투표로 분산 감소)
else           → single              (결정적 — 추가 조율 불필요)
```

**n_agents:** `min(distinct_expertise_domains, MAX_FANOUT=5)`. 초과 시 도메인 묶기 또는 2단계 합성.

> 이 분류는 **제안(proposal)**이다. graph.json의 single-writer(메타스킬)가 최종 확정하며, 노드별로 mechanism을 다르게 줄 수 있다(예: pipeline 전체는 reflect 제안이라도 gather/fetch는 single, verify만 reflect-then-revise). classify의 mechanism은 "이 도메인의 가장 임계 단계에 어떤 조율이 어울리는가"의 기본값이다.

### 워크스루: deep-research

술어 `{domains:4, staged:true, rerun:true, objective:true, noisy:true}` →
- off-ramp 통과(domains≥2, staged) → build-harness
- topology: staged AND domains≥2 → **pipeline** (기본 제안)
- mechanism(first match): not-objective? 아니오 → staged? 예 → **reflect-then-revise** (가장 임계 단계 = verify)
- 실제 예제 graph: gather(single)→fetch(single)→verify(reflect-then-revise)→synthesize(single). pipeline + 노드별 mechanism 혼합.

---

## 10. 에이전트 분리 기준 (4축 — 원본 지혜 보존)

> 원본의 핵심 설계 지혜. **그대로 보존한다.** 한 노드를 하나의 에이전트로 둘지, 쪼갤지 판단하는 4축.

| 기준 | 분리 | 통합 |
|------|------|------|
| **전문성 (expertise)** | 판단의 성격이 다르면 분리 (deep-research의 critic=결함찾기 vs reviser=고치기는 정반대 사고) | 영역이 겹치면 통합 |
| **병렬성 (parallelism)** | 독립 실행이 이득이면 분리 (ticket-triage의 category·priority는 독립 → fan-out) | 순차 종속이면 통합 고려 |
| **컨텍스트 (context)** | 한 에이전트가 다 들면 컨텍스트가 터지면 분리 (단계별 파일 SOT) | 가볍고 빠르면 통합 |
| **재사용성 (reuse)** | 다른 하네스에서도 쓰면 분리 (reviewer·fact-checker·translator는 게놈 공통 에이전트로 상속) | 이 하네스에서만 쓰면 통합 고려 |

CYS 맥락에서의 적용:
- 분리는 **노드 수 증가 = 비용 증가**다(cost-band가 노드별 est_tokens 합산). 4축이 모두 분리를 가리킬 때만 쪼갠다.
- 분리한 두 에이전트는 각각 **`write_paths`가 겹치면 안 된다**(`validate_harness.py`가 write-path overlap을 error로 차단). 병렬 노드가 같은 파일을 쓰면 분리가 잘못된 것이다.
- reflect-then-revise처럼 **한 에이전트가 두 역할(critic/reviser)**을 패스로 수행하는 경우는 "분리"가 아니라 단일 agentType + mechanism으로 표현한다(deep-research verifier 참조).

---

## 11. 에이전트 타입·모델 티어 선택

### 모델 티어 — role→tier 정책 (원본의 "all opus" 대체)

원본은 모든 에이전트를 opus로 강제했다. CYS는 role-class별 티어를 강제한다(`validate_harness.py`의 `TIER_BY_ROLE_CLASS`):

| role-class | tier | 결정 방식 |
|---|---|---|
| gather, extract, format, qa-scan | **haiku** | id+agent 키워드 정규식 (gather/fetch/search/extract/parse/format/render/report/qa/lint/check/verify…) |
| voter, debater, reviser | **sonnet** | mechanism이 부여(majority-vote→voter, debate→debater, reflect→reviser) |
| synthesis, judge, critic, architecture | **opus** | 키워드(synth/aggregate/merge/judge/critic/review/architect/plan/design) + judge/critic은 mechanism_params 티어링 |

- **모든 노드는 `model:` 필수**(누락 시 `TIER_MISSING`/`AGENT_FRONTMATTER` 하드에러 — V1 게이트). `model_rationale:`은 **emit가 항상 채우며**, 누락 시 `RATIONALE_MISSING` **warn**(기본; 정책상 `MODEL_RATIONALE_MISSING`를 error로 승격 가능, 빌드를 막지는 않음).
- **pure-retrieval role에 opus 금지** — `tier_override_reason` 없이 gather/extract/format/qa-scan을 opus로 두면 error(V2 게이트).
- **node.model == agent frontmatter model** 일치 강제(V3 게이트).
- 미매핑 role-class는 fail-safe-expensive로 synthesis(opus) 처리 → 검증기가 명시적 model을 강제(은밀히 싸고 틀리는 것 방지).

### 에이전트 정의 (`.claude/agents/<agent>.md`)

모든 `node.agent`는 `.claude/agents/<agent>.md` 파일로 존재해야 한다(`validate_harness.py`가 agent-file-exists 체크). `emit_orchestrator._write_agent_files`가 frontmatter를 렌더한다:

```yaml
---
name: verifier
description: "역할 + 트리거 키워드 (pushy하게 — 트리거 메커니즘)"
model: sonnet               # node.model과 일치해야 함 (V3)
model_rationale: "왜 이 티어인가 1문장"   # emit가 항상 채움; 누락 시 RATIONALE_MISSING warn (기본)
tools: Read, Write, Glob, Grep   # least-privilege (role-class 디폴트 또는 node.tools)
maxTurns: 25                # Agent 도구가 런타임 강제
---
```

`tools` 디폴트(`_ROLE_TOOLS`)는 role-class별 least-privilege다(gather→`Read, Glob, Grep, WebSearch, WebFetch`, format→`Read, Write`, judge→`Read, Glob, Grep` 등); `node.tools`가 있으면 그것이 우선한다. **frontmatter의 model·tools·maxTurns를 Agent 프리미티브가 런타임 강제**한다 — 이것이 은퇴한 Mode-A의 general-purpose 다운그레이드의 정확한 역(inverse)이다.

본문: 핵심역할 / 작업원칙 / 입출력 프로토콜(정확한 `_workspace/` 경로 + 방출 schema) / 에러핸들링. mechanism 노드는 어떤 패스를 도는지 명시(critic 패스 vs reviser 패스).

> **원본 빌트인 타입(general-purpose/Explore/Plan)은 CYS에서 직접 쓰지 않는다.** 모든 에이전트는 명시적 `.claude/agents/` 파일로 정의하고 model 티어 + least-privilege tools를 선언한다.

---

## 12. 팀 크기 / fan-out 가이드

> 원본의 팀 크기·계층 깊이 지혜를 CYS fan-out 한계로 번역.

- **fan-out 폭 ≤ MAX_FANOUT(5).** dispatch/fan-out-fan-in의 병렬 워커, majority-vote의 n(스키마 max 5), debate의 debater 수 모두 이 한계. 초과 시: (a) 영역 묶기, (b) 2단계 합성(워커→중간 sink→최종 sink).
- **계층 깊이 ≤ 2.** hierarchical 토폴로지의 하드 제한(`_topology_addendum`). 원본의 "3단계 이상은 지연·컨텍스트 손실" 경고를 보존.
- **노드 수와 비용은 비례한다.** 노드를 늘리기 전에 4축 분리 기준(§10)을 통과하는지 확인. 통과 못 하면 통합이 옳다(단순성 우선). 깊이가 필요하면 노드를 늘리지 말고 그 노드의 mechanism을 single→vote/debate/reflect로 올린다.
- **mechanism fan-out 비용 인지:** single=1, majority-vote=n, debate=2·rounds+1, reflect=2·rounds. 임계 노드에만 비싼 mechanism을 부여(§6 마지막 원칙).

---

## 13. 복합 패턴 사고

원본은 단일 패턴보다 복합이 흔하다고 가르쳤다. CYS에서 "복합"은 **topology는 하나, 노드별 mechanism은 혼합**으로 표현된다(hybrid execution_mode의 emit는 **현재 team 레시피와 동일**(P0-2) — 미구현은 Phase별 agent/team 기질 *혼합* emit뿐, §8).

| 원본 복합 패턴 | CYS 표현 | 예시 |
|---|---|---|
| 팬아웃 + 생성-검증 | fan-out-fan-in topology + 각 워커 노드에 reflect-then-revise mechanism | 4언어 병렬 번역, 각 노드가 비평→수정 |
| 파이프라인 + 팬아웃 | pipeline topology + 중간 단계를 dispatch sink로 (hybrid 단계별 혼합은 미구현 — §8) | 분석(순차)→구현(병렬)→통합(순차) |
| 감독자 + 전문가 풀 | supervisor 또는 expert-pool topology (둘 다 first-class) | 문의 분류 후 동적 할당 |

복합 설계 원칙:
1. **topology는 도메인의 지배적 데이터 흐름** 하나로 정한다(graph 전체는 단일 topology 필드).
2. **mechanism은 노드마다** 그 노드의 답 생성 특성으로 정한다 — 이것이 "복합"을 만든다.
3. 진짜로 두 개의 다른 데이터 흐름이 필요하면 단일 sink로 수렴하는 fan-out-fan-in으로 표현하거나, pipeline 안에 병렬 구간을 둔다(hybrid execution_mode의 단계별 기질 혼합은 미구현 — §8).
4. 실시간 양방향 협업이 *본질적*이면 — `execution_mode: team`(§8).

> deep-research가 정준 복합 예시다: **pipeline(topology)** 하나에 노드별로 single·single·**reflect-then-revise**·single을 혼합. 원본이라면 "파이프라인 + 생성-검증 복합"이라 불렀을 것을, CYS는 단일 topology + 노드별 mechanism의 깔끔한 직교 표현으로 담는다.

---

## 14. idoforgod 8 use case → topology 배정

CYS의 parity bar(M7 / R2)는 idoforgod README의 **8 use case 전부에 conforming 하네스를 emit**하는 것이다. `eval_topology.py`가 build-level 적합성(topology + exec_mode + all-6 프리미티브 floor + 필수 DNA 섹션 + topology 레시피)을 머신체크한다. 다음 배정은 `tests/test_factory.py`의 `TestEightUseCases.USE_CASES`에 박혀 있는 그대로다(전부 conform, **5종 distinct 토폴로지** 행사 — 그 중 first-class는 3종: fan-out-fan-in·supervisor·hierarchical; expert-pool는 8 use case에 미배정):

| # | idoforgod use case | topology | execution_mode | 배정 근거 |
|---|---|---|---|---|
| 1 | Deep Research | `fan-out-fan-in` | team | 다영역 병렬 조사 + 실시간 상충 공유 → 팀 병렬 + Lead 합성 |
| 2 | Website Development | `pipeline` | team | 기획→디자인→구현→배포의 강한 순차 의존 |
| 3 | Webtoon / Comic Production | `producer-reviewer` | team | artist 생성 ↔ reviewer 검수의 경계 루프(문제 패널 재생성) |
| 4 | YouTube Content Planning | `supervisor` | team | 가변 길이 콘텐츠 기획 — 런타임 동적 작업 발행 |
| 5 | Code Review & Refactoring | `fan-out-fan-in` | team | 파일/관점 병렬 리뷰 + 교차영역 발견 공유 |
| 6 | Technical Documentation | `pipeline` | team | 개요→초안→예제→교정의 순차 의존 |
| 7 | Data Pipeline Design | `hierarchical` | team | sub-coordinator 팀(Level-1) → 각자 sub-agent(Level-2), depth ≤ 2 |
| 8 | Marketing Campaign | `producer-reviewer` | team | 캠페인안 생성 ↔ 검수의 반복 정제 루프 |

> 행사된 토폴로지: fan-out-fan-in(×2)·pipeline(×2)·producer-reviewer(×2)·supervisor·hierarchical — **5종 distinct 토폴로지**(그 중 first-class 3종: fan-out-fan-in·supervisor·hierarchical; pipeline·producer-reviewer는 base 토폴로지로 `_topology_addendum` 레시피 없음 — §4/§7 정의). 8개 모두 `execution_mode: team`이다(현 parity 기준은 build-level — graph가 계약에 conform하고, 오케스트레이터가 올바른 topology 레시피·A2 all-primitive floor·필수 DNA를 갖는지). 런레벨 head-to-head는 별도 quota-gated 레인이다.

다음 세 절(§15–§17)은 이 표의 일반론을 **디스크에 실재하는 3개 하네스**(`examples/{deep-research,ticket-triage,design-decision}/`)로 못박는다. 모든 필드·model·rationale은 디스크 그대로다.

---

## 15. 실제 예시 A — deep-research (pipeline + reflect-then-revise)

### 도메인
임의의 주제에 대해 웹 검색을 팬아웃하고, 소스를 fetch하고, claim을 적대적으로 검증한 뒤, 인용된 보고서를 합성한다.

### 위상·기제 선택 근거
- **왜 pipeline인가**: 단계마다 **의존하는 입력의 종류가 다르고 순서가 본질적**이다. gather가 후보 claim/source를 만들어야 fetch가 그것을 grounding하고, fetch가 정리해야 verify가 검증하고, verify가 통과시켜야 synthesize가 인용한다. 각 단계가 직전 단계의 산출에 전적으로 의존하므로 **pipeline**이다. (§14의 Deep Research use case는 다영역 병렬조사를 강조해 fan-out-fan-in으로 배정되지만, 이 실제 예제 하네스는 검증 깊이를 강조한 pipeline 변형이다 — 같은 도메인도 강조점에 따라 다른 좌표를 가질 수 있음의 실증.)
- **왜 verify 노드에만 reflect-then-revise인가**: 4단계 중 사실 검증만이 **적대적 자기교정**을 필요로 한다. gather/fetch/synthesize는 단일 패스로 충분하지만, claim의 사실성은 한 번 써놓고 끝낼 수 없다. 그래서 그 한 노드에만 `reflect-then-revise(max_rounds=2, critic=opus)`. 위상은 pipeline 그대로, 기제만 그 노드에서 교체된다(직교성의 실증).

### 노드·에이전트 분해

| node id | agent | model | rationale (frontmatter) | mechanism | on_exhaust |
|---------|-------|-------|-------------------------|-----------|-----------|
| `gather` | researcher | haiku | "순수 웹검색+claim 초안, 교차판단 없음 — 최저 티어" | single | proceed-with-gap |
| `fetch` | fetcher | haiku | "소스 fetch+claim-소스 grounding, 합성 없음 — 최저 티어" | single | proceed-with-gap |
| `verify` | verifier | sonnet | "Reviser 기본 티어; critic 패스는 mechanism_params로 opus 호출" | reflect-then-revise | proceed-with-gap |
| `synthesize` | synthesizer | opus | "교차소스 합성+검증 claim 위 판단 — 최고 티어" | single | escalate |

**model 티어가 역할을 따른다**: gather/fetch는 단순 수집(haiku), verify는 reviser급(sonnet)이되 critic 패스만 opus로 끌어올림, synthesize는 합성 판단(opus). **reflect-then-revise의 두 패스가 한 agentType에 산다** — verifier.md 하나가 critic 패스(opus)와 reviser 패스(sonnet)를 모두 담고, `approved=true`면 그 라운드에서 루프가 끊긴다.

### graph.json 발췌 (실제, `execution_mode: team`)

```json
{
  "schema_version": "0.1",
  "harness_name": "deep-research",
  "harness_version": "0.1.0",
  "execution_mode": "team",
  "topology": "pipeline",
  "budget": { "total_tokens": 600000, "approval_required": true },
  "nodes": [
    {
      "id": "verify", "agent": "verifier", "model": "sonnet", "decision_mechanism": "reflect-then-revise",
      "mechanism_params": { "max_rounds": 2, "critic": "opus" },
      "inputs": ["_workspace/02_fetch/findings.json"],
      "outputs": ["_workspace/03_verify/findings.json"],
      "write_paths": ["_workspace/03_verify/"],
      "output_schema": "schemas/findings.json",
      "retries": 0, "on_exhaust": "proceed-with-gap", "max_rounds": 2
    }
  ],
  "edges": [
    { "from": "gather", "to": "fetch" },
    { "from": "fetch", "to": "verify" },
    { "from": "verify", "to": "synthesize" }
  ]
}
```
(gather/fetch/synthesize 노드는 지면상 생략 — 전체는 `examples/deep-research/.harness/graph.json`.)

### 읽어야 할 디테일
- `edges`는 순서일 뿐, 의존(어떤 파일을 읽나)은 `inputs[]`가 표현한다 — verify의 input은 `02_fetch/findings.json`이지 "fetch 노드"가 아니다. (team 모드에서는 오케스트레이터가 이 edges를 `TaskCreate(depends_on=…)`로 변환한다.)
- `write_paths`가 노드마다 분리(`01_gather/`, `03_verify/`…)되어 validate의 write-path 중복 검사를 통과한다.
- 출력 노드(synthesize)의 `on_exhaust: escalate` — 최종 산출 실패는 사람에게 올린다. 중간 노드는 `proceed-with-gap`.

---

## 16. 실제 예시 B — ticket-triage (dispatch + majority-vote)

### 도메인
지원 티켓 하나를 받아 카테고리와 우선순위를 동시에 판정하고, 두 결과를 합쳐 큐·SLA로 라우팅한다.

### 위상·기제 선택 근거
- **왜 dispatch인가**: 카테고리 분류와 우선순위 판정은 **서로 독립적**이다 — 두 판단이 병렬로 fan-out하고, 단일 sink(route)로 fan-in한다. pipeline이면 둘을 인위적으로 직렬화해 느려지고, producer-reviewer는 루프가 필요 없는 이 작업에 과하다.
- **왜 두 source 노드에 majority-vote인가**: "이 티켓이 bug냐 feature_request냐", "P1이냐 P2냐"는 **경계가 모호한 주관 판단**이라 단일 에이전트는 흔들린다. 그래서 각 분류를 `majority-vote(n=3, quorum=2, tie_break=first)`로 — 3개 독립 ballot을 다수결로 안정화한다. sink 노드(route)는 결정이 아니라 **결정론적 병합**이므로 `single`로 충분하다.

### 노드·에이전트 분해

| node id | agent | model | rationale | mechanism | params |
|---------|-------|-------|-----------|-----------|--------|
| `classify_category` | classifier | sonnet | "모호한 티켓을 한 카테고리로 판단하는 독립 voter — voter 기본 티어" | majority-vote | n=3, quorum=2, tie_break=first |
| `classify_priority` | prioritizer | sonnet | "영향+긴급을 한 심각도로 종합하는 독립 voter — voter 기본 티어" | majority-vote | n=3, quorum=2, tie_break=first |
| `route` | router | haiku | "두 상류 승자를 큐/SLA로 결정론 병합 — format급, 최저 티어" | single | {} |

**voter는 sonnet, sink format은 haiku.** **독립성이 agent 본문에 박혀 있다** — classifier.md: "독립 투표다. 다른 ballot을 가정하거나 합의를 노리지 않는다." majority-vote는 ballot 간 통신이 **없어야** 작동한다(병렬 spawn이 서로 못 봄). sink router.md: 두 majority-vote 승자가 `[classification, priority]` 배열로 fan-in한다; 재판단 없이 병합만.

### graph.json 발췌 (실제, `execution_mode: team`)

```json
{
  "schema_version": "0.1",
  "harness_name": "ticket-triage",
  "execution_mode": "team",
  "topology": "dispatch",
  "budget": { "total_tokens": 120000, "approval_required": true },
  "nodes": [
    {
      "id": "classify_category", "agent": "classifier", "model": "sonnet",
      "decision_mechanism": "majority-vote",
      "mechanism_params": { "n": 3, "quorum": 2, "tie_break": "first" },
      "inputs": ["_workspace/00_input/ticket.md"],
      "outputs": ["_workspace/01_category/classification.json"],
      "write_paths": ["_workspace/01_category/"],
      "output_schema": "schemas/classification.json",
      "retries": 1, "on_exhaust": "proceed-with-gap", "max_rounds": 1
    },
    {
      "id": "route", "agent": "router", "model": "haiku", "decision_mechanism": "single",
      "mechanism_params": {},
      "inputs": ["_workspace/01_category/classification.json", "_workspace/02_priority/priority.json"],
      "outputs": ["_workspace/03_route/routing.json"],
      "write_paths": ["_workspace/03_route/"],
      "output_schema": "schemas/routing.json",
      "retries": 0, "on_exhaust": "escalate", "max_rounds": 1
    }
  ],
  "edges": [
    { "from": "classify_category", "to": "route" },
    { "from": "classify_priority", "to": "route" }
  ]
}
```
(`classify_priority` 노드는 지면상 생략 — `classify_category`와 동형. 전체는 `examples/ticket-triage/.harness/graph.json`.)

### 읽어야 할 디테일
- **두 source가 같은 input**(`00_input/ticket.md`)을 읽고 **서로 다른 output**에 쓴다 — fan-out의 전형. 두 edge가 모두 `route`로 들어가 fan-in.
- route의 `inputs[]`가 **두 개**다. agent 본문은 이것이 `[classification, priority]` 순서 배열로 전달된다고 명시 — 순서가 계약이다.
- budget이 120k로 작다(deep-research 600k와 대비) — budget은 도메인 규모에 맞춘다.
- route의 `on_exhaust: escalate` — 라우팅 실패는 티켓 미아이므로 사람에게. 단, 한쪽 source가 gap이면 router가 안전 기본값(other→triage_backlog, P3→72h)으로 채운다.

> **supervisor와의 차이:** 이 예시는 정적 dispatch(노드 수 고정). 작업 수가 런타임에 정해지면(예: "파일 N개 마이그레이션") `topology: supervisor`(Lead가 동적 `TaskCreate` 발행)다 — §4.5.

---

## 17. 실제 예시 C — design-decision (producer-reviewer + debate-with-judge)

### 도메인
기술 설계 결정에 대해 후보안을 제안하고, 적대적 토론으로 검증한 뒤, 미해결 우려가 있으면 재제안하는 경계 루프.

### 위상·기제 선택 근거
- **왜 producer-reviewer인가**: 좋은 설계 결정은 **생성과 비판의 왕복**에서 나온다. proposer가 안을 내면 adjudicate가 흔들고, 결함이 남으면 proposer가 그 concern을 안고 다시 낸다. pipeline은 한 방향이라 재제안을 표현 못 하고, dispatch는 병렬이라 변증법적 왕복이 없다.
- **왜 adjudicate 노드에 debate-with-judge인가**: 설계 채택은 **단일 검토자보다 양측 변론 후 심판**이 강하다. 추천안을 옹호하는 측(k=0)과 최강 대안을 미는 측(k=1)이 다투고, opus judge가 transcript를 근거로 `chosen`+`approved`를 결정한다. 그래서 `debate-with-judge(n=2, max_rounds=2, judge=opus)`. reflect-then-revise(자기교정)보다 **대립 구조**가 설계 비교에 적합하다.
- **두 기제가 한 노드에 중첩**: producer-reviewer 루프의 reviewer 자리(adjudicate)가 그 안에서 debate-with-judge를 돌린다. 위상의 루프 종료 조건이 곧 기제의 산출(`verdict.approved`)이다.

### 노드·에이전트 분해

| node id | agent | model | rationale | mechanism | params |
|---------|-------|-------|-----------|-----------|--------|
| `propose` | proposer | opus | "경쟁 설계 위의 아키텍처급 판단 — 최고 티어" | single | (node max_rounds=3) |
| `adjudicate` | debater | sonnet | "Debater 기본 티어; judge 패스는 mechanism_params로 opus 호출" | debate-with-judge | n=2, max_rounds=2, judge=opus |

**debate-with-judge의 두 패스가 한 agentType에 산다** — debater.md 하나가 debater 턴(sonnet, k=0/k=1)과 judge 패스(opus)를 모두 담는다. judge.md는 **사람이 읽는 그래프 뷰용 문서**일 뿐 실행 wiring은 debater agentType을 탄다(verifier가 critic+reviser를 한 몸에 담는 것과 동형). 두 파일은 동기화 유지가 규칙.

### graph.json 발췌 (실제 전체, `execution_mode: team`)

```json
{
  "schema_version": "0.1",
  "harness_name": "design-decision",
  "harness_version": "0.1.0",
  "execution_mode": "team",
  "topology": "producer-reviewer",
  "budget": { "total_tokens": 300000, "approval_required": true },
  "nodes": [
    {
      "id": "propose", "agent": "proposer", "model": "opus", "decision_mechanism": "single",
      "mechanism_params": {},
      "inputs": ["_workspace/00_input/decision.md"],
      "outputs": ["_workspace/01_propose/design.json"],
      "write_paths": ["_workspace/01_propose/"],
      "output_schema": "schemas/design.json",
      "retries": 0, "on_exhaust": "escalate", "max_rounds": 3
    },
    {
      "id": "adjudicate", "agent": "debater", "model": "sonnet",
      "decision_mechanism": "debate-with-judge",
      "mechanism_params": { "max_rounds": 2, "judge": "opus", "n": 2 },
      "inputs": ["_workspace/01_propose/design.json"],
      "outputs": ["_workspace/02_adjudicate/verdict.json"],
      "write_paths": ["_workspace/02_adjudicate/"],
      "output_schema": "schemas/verdict.json",
      "retries": 0, "on_exhaust": "proceed-with-gap", "max_rounds": 2
    }
  ],
  "edges": [
    { "from": "propose", "to": "adjudicate" }
  ]
}
```

### 읽어야 할 디테일
- **루프가 graph.json에 노드로 보이지 않는다.** producer-reviewer 위상이 propose↔adjudicate 왕복을 암시하고, propose의 `max_rounds=3`이 재제안 상한, adjudicate가 반환하는 `verdict.approved`가 종료 조건. 위상이 곧 제어 흐름이다.
- propose `on_exhaust: escalate`(producer 실패=사람), adjudicate `on_exhaust: proceed-with-gap`(합의 못 봐도 judge는 verdict를 내고 미해결분은 `concerns[]`로 넘김).
- `chosen`은 반드시 `design.options[].name` 중 하나여야 한다 — judge가 입력에 없는 옵션을 만들면 루프 종료 판정이 깨진다.
- `approved=true ⟺ concerns=[]` 일관성 강제 — 한쪽만 채우면 루프 제어가 모순된다.

---

## 18. 세 예시 교차 비교 + 보존된 설계 지혜

| | deep-research | ticket-triage | design-decision |
|--|--------------|---------------|-----------------|
| topology | pipeline | dispatch | producer-reviewer |
| execution_mode | team | team | team |
| 핵심 기제 | reflect-then-revise (verify) | majority-vote (×2 source) | debate-with-judge (adjudicate) |
| 노드 수 | 4 | 3 | 2 |
| budget | 600k | 120k | 300k |
| 흐름 형태 | 직렬 사슬 | fan-out → sink | 경계 루프 |
| 최고 티어 위치 | synthesize(opus) + critic 패스 | 없음(voter=sonnet, sink=haiku) | propose(opus) + judge 패스 |
| 종료 조건 | 마지막 노드 | sink 완료 | verdict.approved |

**한 줄 진단법** — 새 도메인을 받으면 두 질문을 순서대로 던진다:
1. **데이터가 어떻게 흐르나?** 한 줄로 흐르면 pipeline, 갈라졌다 모이면 dispatch/fan-out-fan-in, 만들고-부수고-다시 만들면 producer-reviewer, 동적 작업이면 supervisor, 조건부 라우팅이면 expert-pool, 2단계 위임이면 hierarchical → **topology 결정**.
2. **각 판단점에서 신뢰가 어떻게 확보되나?** 한 번이면 single, 표결로 안정화면 majority-vote, 대립 변론이면 debate-with-judge, 자기교정이면 reflect-then-revise → **노드별 mechanism 결정**.

### 보존된 원본 설계 지혜
원본의 패러다임은 바꿨지만, 패러다임과 무관한 설계 지혜는 그대로 가져온다:

- **에이전트 분할 4축**(§10) — 전문성·병렬성·컨텍스트·재사용. reviewer·fact-checker·translator는 게놈 공통 에이전트로 상속.
- **Pushy description = 트리거 메커니즘** — agent frontmatter의 `description`은 설명이 아니라 **트리거**다. 패턴: *Use [언제] + [무엇을] ONE [경계] + Trigger keywords(영/한) + [graph 내 좌표]*. "Use FIRST"(researcher/proposer), "Use LAST"(synthesizer/router) 같은 강한 동사가 라우팅을 끈다.
- **Why-not-ALWAYS 본문 원칙** — agent 본문은 항상 지킬 것만 단정형으로. 추측성 분기·과잉 유연성 없음(classifier "독립 투표다. 합의를 노리지 않는다").
- **Progressive disclosure** — 경량 본문(핵심역할·작업원칙·입출력 프로토콜·에러핸들링) + 깊은 references(schema·게놈 문서).
- **QA 경계 교차비교 + 실버그 패턴** — 검증 노드(verify의 critic, adjudicate의 debater)는 경계를 교차 점검한다. 적대성 규칙("친절하지 말 것. 통과시키면 안 되는 것은 반드시 흔든다") + 실버그 패턴(미인용 claim·과장·오인용·enum 이탈·입력에 없는 값 생성·빈/비-JSON 반환·gap 미표기)이 각 agent 에러핸들링에 박혀 있다. `review` 노드 속성이 있으면 오케스트레이터가 그 후 `Agent(subagent_type="reviewer")`를 일급 L2 게이트로 spawn한다.
- **lift/h2h 측정(공장 내부)** — harness 등록 전 `lift_gate.py`로 with-skill vs haiku-baseline을 독립 블라인드 채점해 register/refuse 결정(baseline에 진 스킬은 출하 불가 = `LIFT_REFUSED`). `h2h_suite.workflow.js`/`h2h_aggregate.py`로 n-run 중앙값 head-to-head — **이들은 공장 내부 측정 도구이지 산출 하네스의 런타임이 아니다.**
- **진화·피드백 / 팀 크기** — 노드 수는 작게 시작(2~4). 깊이가 필요하면 노드를 늘리지 말고 mechanism을 single→vote/debate/reflect로 올린다.

---

## 19. 산출물 패턴 요약 — graph.json이 척추다

새 harness를 만들 때 생성하는 것:

```
<harness>/
├── .harness/
│   ├── graph.json          ← 척추. 가장 먼저, 손으로 작성. graph.schema.json으로 검증.
│   ├── GENOME.json / RUNTIME.json / graph.lock / constants.json  ← 도구 생성
├── .claude/
│   ├── skills/<harness>-orchestrator/SKILL.md  ← emit_orchestrator.py가 graph.json에서 렌더 (canonical 런타임)
│   ├── agents/<agent>.md    ← 노드마다. model·model_rationale·tools·maxTurns frontmatter (Agent가 런타임 강제)
│   └── skills/<harness>-<id>/SKILL.md  ← (M3) skill_authoring.mode=skill 노드만
├── schemas/<name>.json      ← 노드 output_schema마다. JSON-Schema.
└── (상속 게놈: hooks·4계층 품질게이트·보안·공통 에이전트·메모리 스토어)
```

작성 순서 (계약 우선):
1. **graph.json** — topology + execution_mode + 노드별 model·decision_mechanism·on_exhaust·schema 경로 결정.
2. **schemas/*.json** — 각 노드 출력의 JSON-Schema. graph가 가리키는 경로에.
3. **.claude/agents/*.md** — 노드마다. pushy description + 5섹션 본문 + `model`/`model_rationale`(emit이 frontmatter 정규화).
4. `inherit_genome.py`로 게놈 상속(자식이 부모의 전체 운영 기계 내장 — `emit_orchestrator`가 호출).
5. `python3 ../../validate_harness.py .` — 머신체크 세트 통과.
6. `warrant.py` — Phase -1 비용 게이트 승인.
7. `emit_orchestrator.py .` → `.claude/skills/<harness>-orchestrator/SKILL.md` + agents + 게놈 emit.
8. `cd <harness> && claude` 라이브 세션에서 `<harness>-orchestrator` 스킬 트리거 → 그 세션의 게놈 hook이 발화.
9. (공장 내부) `lift_gate.py`로 baseline 대비 lift 입증 → register.

> 핵심 전환: 원본은 "에이전트 파일을 쓰고 팀으로 조율"했다. CYS는 **graph.json 계약을 먼저 쓰고, 도구가 컴파일·검증·측정**한다. 산출 하네스는 **오케스트레이터 SKILL + .claude/agents**(Claude Code 프리미티브)로 실행되며 상속된 AWF 게놈이 발화한다. 에이전트는 계약에서 파생되는 부품이지 출발점이 아니다.

---

## 20. 외부 출처 근거 (학계·표준 1:1 대조)

이 스킬의 결정론 환원 **불가능한** 설계 선택(LLM 추론에 본질적으로 위임되는 개념)은 임의 발명이 아니라 학계 주류·산업 표준 문헌으로 뒷받침된다. 4개 `decision_mechanism`·7개 topology·model-tier 라우팅을 1:1로 대조한다(검증 완료, 2026-05-31; 출처 없는 설계는 채택 금지 원칙).

| 스킬 개념 (원문) | 주류 명칭 | 출처 (저자·연도·제목·게재) |
|---|---|---|
| `decision_mechanism: "majority-vote"` (병렬 투표자 → quorum 다수결) | **Self-Consistency** (샘플링된 추론 경로의 다수결) | Wang, Wei, Schuurmans, Le, Chi, Narang, Chowdhery, Zhou (2022), *Self-Consistency Improves Chain of Thought Reasoning in Language Models*, ICLR 2023 (arXiv:2203.11171) |
| `decision_mechanism: "debate-with-judge"` (N debater × rounds → judge 판정) | **Multiagent Debate** + **LLM-as-judge** | Du, Li, Torralba, Tenenbaum, Mordatch (2023), *Improving Factuality and Reasoning in Language Models through Multiagent Debate*, ICML 2024 (arXiv:2305.14325) · Zheng et al. (2023), *Judging LLM-as-a-Judge with MT-Bench and Chatbot Arena*, NeurIPS 2023 D&B (arXiv:2306.05685) · 계보: Irving, Christiano, Amodei (2018), *AI Safety via Debate* (arXiv:1805.00899) |
| `decision_mechanism: "reflect-then-revise"` (critic → reviser 반복, approved 시 종료) | **Self-Refine / Reflexion** (자기피드백 반복 정련) | Madaan et al. (2023), *Self-Refine: Iterative Refinement with Self-Feedback*, NeurIPS 2023 (arXiv:2303.17651) · Shinn, Cassano, Berman, Gopinath, Narasimhan, Yao (2023), *Reflexion: Language Agents with Verbal Reinforcement Learning*, NeurIPS 2023 (arXiv:2303.11366) |
| 7 topology 중 `supervisor`·`hierarchical`·`pipeline`·`fan-out-fan-in`·`producer-reviewer` | 주류 멀티에이전트 오케스트레이션 패턴 (supervisor·hierarchical·network·parallelization·evaluator-optimizer) | LangGraph/LangChain (2024), *Multi-Agent Architectures* 공식 문서 (langchain-ai.github.io/langgraph) · Anthropic Engineering (2024), *Building Effective Agents* (orchestrator-workers·evaluator-optimizer·parallelization) |
| topology `dispatch` (라우터 → 병렬 워커 → 수집) | **Scatter-Gather** (표준 통합 패턴; `dispatch`는 본 스킬 고유 명칭) | Hohpe & Woolf (2003), *Enterprise Integration Patterns — Scatter-Gather / Recipient List*, Addison-Wesley (ISBN 0321200683) |
| model-tier 라우팅 (gather/extract/format/qa-scan→haiku, voter/debater/reviser→sonnet, synthesis/judge/critic/architecture→opus) | **비용-품질 라우팅 캐스케이드** | Chen, Zaharia, Zou (2023), *FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance* (arXiv:2305.05176, TMLR 2024) · Ong et al. (2024), *RouteLLM: Learning to Route LLMs with Preference Data* (arXiv:2406.18665, ICLR 2025) |

> 결정론 환원 **가능한** 단계(역할→티어 매핑·fanout/spawn 계산·번호/존재 검사·날짜·범위)는 LLM이 재추론하지 않도록 파이썬으로 강제한다(`role-class-policy.json` 단일 SoT·`sot_init.estimate_max_spawns` baking·`validate_harness.py` 49 머신체크 코드). 위 표의 개념들은 *환원 불가능한* LLM 추론 위임으로, 학계/표준 출처로 정당성을 입증한다.
