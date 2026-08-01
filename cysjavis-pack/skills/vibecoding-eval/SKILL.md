---
name: vibecoding-eval
description: 바이브코딩 체계의 실력을 자기채점 없이 측정하는 eval 하네스 절차. "public benchmarks are a filter, internal evals are the verdict" 원칙 위에서 threat model 6종·격리 실행·사전등록 A/B protocol(PR-ABEVAL-001)·integrity gate strict flow·auditor trajectory 감사·mutation 검산(PR-MUTOPS-001)을 굴린다. "체계 효과 측정, A/B eval, 벤치 말고 내부 eval, reward hacking 방지, mutation 검산, 개선이 진짜냐 검증" 트리거, 또는 신·구 체계 비교나 개선 라운드의 이득을 증명해야 할 때 발동.
---

# vibecoding-eval

바이브코딩 체계가 실제로 더 낫다는 것을 **측정으로 증명**하는 하네스(설계 제안서 §C7·§4-G).
자기채점·자랑성 벤치 인용은 금지. 원칙 한 줄:

> **"public benchmarks are a filter, internal evals are the verdict."**
> 공개 벤치는 최소 통과 필터일 뿐, 판정은 자비스 **내부 대표 과제 세트**로만 내린다
> (research-security-eval.md §3 — Cursor 2026.6 실증: 벤치 해결의 63%가 '검색된 답'이었다).

이 스킬은 절차 체크리스트다. RSI 개선 라운드의 keep-rule·locked eval 원칙은
[[eval-driven-self-improvement]]가 담당하고, 본 스킬은 그 위에 **A/B 체계 비교·위협 방어·
무결성 게이트**를 얹는다. 사전등록 봉인 record는 `_round/vibecoding-ledger/records/`
(`PR-ABEVAL-001.json`·`PR-MUTOPS-001.json`)이 정본이다 — 값은 그 record를 인용하라.

## 산출 순서 (제안서 §4-G 격상)

G축의 첫 산출물은 하네스가 아니라 **eval threat model + red-team fixture**다. 위협 모델·격리
설계를 먼저 승인받고(§C7.1~2), 그 다음에 A/B 실증(§C7.3)을 사전등록 protocol로 돌린다.

## 1. 위협 모델 6종 (§C7.1 — 방어 대상)

측정을 오염시키는 6개 벡터. 각각에 §2의 방어가 대응한다.

1. **직접 retrieval** — 검색된 답 복제(63% 실증).
2. **test-aware hardcode** — 테스트를 알고 정답을 박아넣음.
3. **test-suite 변조** — assertion 무력화·self-mocking·skip 삽입.
4. **비관측 외부 도움** — 네트워크 등 관측 밖 조력.
5. **A/B 간 상태 누수** — 구/신 run 사이 공유 상태 오염.
6. **auditor 오분류** — retrieve/derive를 잘못 가름.

## 2. 기본 방어 — 격리 실행 (§C7.2)

- no-network 또는 allowlist 실행.
- **sealed holdout** — 평가자만 접근(구현 주체 접근 불가).
- append-only raw trajectory 보존.
- sanitized repo — 미래 git history 제거.
- task/env hash 고정.
- auditor를 라벨된 retrieve/derive 사례로 보정 + 표본 인간 감사.

## 3. A/B eval protocol (§C7.3 · 사전등록 PR-ABEVAL-001 준수)

**control vs treatment 분리** — 처치(신체계의 route/skill/prompt/구현)까지 고정하면 처치가
소거된다. 두 계층으로 분리:
- **common control** (양 variant 공통 고정): 과제 스냅샷·기반 memory/skill·환경·모델·도구·예산.
- **variant treatment** (variant별로 다른 게 정당한 요소): route-contract/skill/prompt/구현 코드
  — 단 그 버전을 **commit 해시로 고정**.

**격리 7축 (전부 의무)**: ①독립 workspace/state(fresh clone·공유 디렉토리 0) ②common control
주입 동결(핀 고정 스냅샷·실행 중 갱신 금지) ③캐시 무공유(run별 초기화) ④model·tool 버전
pin(정확한 model id·도구 버전, alias 금지) ⑤counterbalanced 순서(ABBA — 순서 효과 상쇄)
⑥동일 재시도 예산(초과 시 해당 run fail) ⑦holdout 접근권 분리(구현 주체 접근 불가).

**사전등록(pre-registration)**: 과제 목록·측정 지표·판정 기준·7축 설정·**허용차이 목록**
(variant treatment 요소 열거)을 run **시작 전** ledger에 해시 봉인한다. 등록 후 변경=해당
배치 무효. → `PR-ABEVAL-001.json`이 이 봉인 record(status: pre-registered).

**run별 provenance manifest — 16 의무 필드** (하나라도 결손=해당 run 비교 불가·결과 미계상):
`run_id` · `system_variant(구|신)` · `common_control_snapshot_sha` ·
`variant_treatment_manifest_sha` · `model_id` · `tool_versions` · `task_hash` · `env_hash` ·
`workspace_snapshot_sha` · `run_order_index` · `retry_budget` · `retry_count` ·
`pre_registration_record_hash` · `seed` · `start/end time` · `network_policy`.

**comparability verdict** (효과 귀속 전 통과 필수):
- 기본 규칙 — 양 run의 차이를 전수 대조해 차이가 **사전등록된 허용차이 목록(treatment 요소)
  안에만 존재하고 그 외 차이가 0**일 때만 "비교 가능". 목록 밖 차이 1건이라도 발견=비교
  불가(효과 귀속 금지).
- 기계 대조 2건 — ①양 run의 `pre_registration_record_hash`가 서로 같고 ledger 봉인 해시와
  일치(불일치=비교 불가) ②각 run의 `retry_count ≤ retry_budget`이고 양 run의 `retry_budget`이
  동일(위반=해당 run fail). 판정 결과를 결과와 함께 보존한다.

## 4. Integrity gate — strict execution flow 6단계 (§C7.4·§C7.4→§C10)

**순서 고정 — 순서 변경 자체가 계약 위반이다.**

1. **pre-run 기록** — 테스트 스위트 전 파일의 git hash + assertion 정적 센서스(assert 수·skip
   마커 수) 기록.
2. **구현 run** 실행.
3. **integrity gate (기능 테스트보다 먼저)** — 테스트 파일 hash 대조로 인가되지 않은 변동
   검출 + 정적 필터(assertion 수 감소·self-mock 삽입·skip 마커 추가). **검출 시 그 run 즉시
   폐기·검역(결과 미계상) — 기능 테스트로 진행하지 않는다(fail-closed)**.
4. integrity 통과 후에만 **기능 테스트** 실행.
5. 기능 테스트 후 **mutation test**(§5).
6. 마지막으로 **auditor trajectory 감사·derived-fix 분류**(§6).

## 5. Mutation 검산 — 검증기 무결성 (§C10.2 · 사전등록 PR-MUTOPS-001)

검증기가 물지 않으면(공허 통과·vacuous pass) 모든 게이트가 false-green이 된다. 방어:

- 기능 테스트 pass 후, 검증 대상 핵심 경로에 mutation **N=5** 주입 → **테스트 스위트가 최소
  1개 이상 fail해야 검증기 유효**. 전 mutation 생존(테스트 전부 pass)=검증기 무효 → 해당 검증
  결과 전체 기각 + 검증기 수리 티켓 발행.
- **operator 최소 셋 5종(사전 고정 — 확장은 계약 개정으로만)**: ①비교 연산자 반전
  (`>`↔`<`·`>=`↔`<=`·`==`↔`!=`) ②산술 연산자 교체(`+`↔`-`·`*`↔`/`) ③논리 연산자 교체
  (`and`↔`or`) ④경계값 ±1(off-by-one) ⑤반환값 상수화(반환을 고정 상수로 치환).
- **분배 규칙** — N개는 무작위가 아닌 **사전등록된 분배**로 선택·기록(재현 가능). N=5 초기
  분배 = operator 1~5 각 1개씩 균등 주입. → `PR-MUTOPS-001.json`이 정본.
- **negative test 의무** — 각 게이트는 "막아야 할 입력을 실제로 막는지"의 적대 입력 케이스를
  최소 1개 동반한다(막을 대상이 없는 게이트=미완).

## 6. Auditor trajectory 감사·derived-fix rate 분리 (§4-G·§C7.6)

pass율과 실력은 다르다 — pass의 상당수가 '검색된 답'일 수 있다(63% 실증). 그래서:

- **auditor agent가 raw trajectory를 감사**해 각 해결을 **retrieve(검색된 답) vs derive(유도한
  해결)**로 분류한다.
- **derived-fix rate를 pass율과 별도 지표로 분리 측정**한다 — 판정의 기준은 pass율이 아니라
  derived-fix rate.
- auditor는 §2대로 라벨된 retrieve/derive 사례로 사전 보정하고, 표본은 인간 감사로 교차확인.
- 채점 기준 locked, harness 버전 핀 고정([[eval-driven-self-improvement]] Rule 1·LOCKED ref
  launcher 원칙과 접합).

## 7. 내부 과제 세트 구축 절차 (§4-G — "verdict"의 원천)

내부 eval이 판정이므로, 그 과제 세트의 품질이 곧 측정의 품질이다.

1. **초기 규모 10~20개** 과제로 시작(→ 성장 목표 50~100개). 과제는 자비스가 실제로 부딪히는
   대표 작업(Level 분포를 반영 — L3 brownfield·L4 상태/연동·L5 풀스택을 섞는다).
2. **오너 선정 게이트** — 고정 과제의 최종 선정은 오너 재가로 동결한다(남은 결정
   포인트: 오너 실제 프로젝트 중 1개를 고정 과제로 — 제안서 §9). 선정 후 과제 교체=재등록.
3. **holdout 봉인** — 세트의 일부는 sealed holdout으로 평가자 전용 보관(구현 주체 접근 불가).
4. **sanitize** — 각 과제 repo에서 미래 git history·정답 흔적 제거(§2), task/env hash 고정.
5. **producer≠evaluator** — 과제 수행 주체는 자기 지표를 집계하지 않는다(집계 책임=master 단일화).

## 도구·정본 위치

- 사전등록 봉인 record: `_round/vibecoding-ledger/records/PR-ABEVAL-001.json`(A/B 7축·manifest·
  comparability) · `PR-MUTOPS-001.json`(mutation operator·N·분배).
- 설계 계약 정본: `PROPOSAL-jarvis-vibecoding-system-v3.md` §C7(A/B·integrity)·§C10(mutation)·
  §C9(evidence manifest).
- 실행 도구: `cysjavis-pack/bin/javis_prereg.py`·`javis_evidence_manifest.py`로 **구현됨**
  (다음 pack 배포에 동봉). 배포 후 `$PACK/bin`에서 호출한다(`PACK=${CYS_PACK_DIR:-$HOME/.cys/pack}`).
  - **`javis_prereg.py {seal,verify,show}`** — 사전등록 ledger의 append-only SHA-256 봉인·freeze.
    `seal <record.json>`(record_id 필드 필수 — 재봉인 거부) / `verify <record_id>`(봉인 해시 재계산
    대조 — exit 0 일치·1 불일치) / `show <record_id>`(봉인 entry 출력). A/B run 시작 전 PR-ABEVAL-001·
    PR-MUTOPS-001 record를 `seal`로 봉인하고, run 준수 주장 시 `verify`로 `pre_registration_record_hash`
    (§3 manifest 필드)를 대조한다.
  - **`javis_evidence_manifest.py {generate,check}`** — §C9 근거 manifest 게이트(fail-closed).
    `generate --files <파일...> --out <manifest.json>`(경로·SHA-256·존재 봉인) / `check <manifest.json>`
    (존재·SHA-256 일치 대조 — 불일치=exit 1). 제안·설계 문서가 인용하는 근거 파일의 무결성을 결정론
    검증한다.
  - hook/스크립트 배선(integrity gate·A/B 자동화)은 Phase 4 하네스 구축에서 이 두 도구 위에 얹는다.

## 관련 스킬

- [[eval-driven-self-improvement]] — locked eval·keep-rule·7 rules(RSI 라운드 무결성).
- [[vibecoding-knowledge]] — 측정 대상 체계의 개념 정본.
- [[tdd]] — integrity gate가 검사하는 테스트 스위트의 작성 규율.
