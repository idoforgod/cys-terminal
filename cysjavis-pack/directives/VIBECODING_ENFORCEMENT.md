# VIBECODING ENFORCEMENT — 집행 계약 (§C1 · §C2 · §C3 · §C11)

> 출처(정본): `_research/vibecoding-mastery/PROPOSAL-jarvis-vibecoding-system-v3.md` §C1(L104-131)·§C2(L133-155)·§C3(L157-162)·§C11(L332-343).
> 본 문서는 `VIBECODING_CONSTITUTION.md` 10조의 집행 주체·증거·예외를 **기계검증 가능하게** 확정한다.
> 도구 배선: §C2.3 waiver = `bin/javis_waiver.py` · §C11 DECISION = `bin/javis_decision.py` ·
> §C2.4 CT-1~4 = `bin/tests/test_contracts_ct.py` · §C3 prod 권한 거부 E2E = `bin/javis_prod_deny_e2e.sh`.

---

## §C1. Enforcement Contract — actor·권한·fail-closed [B-1]

### C1.1 Actor 5분류 (조문 속 "인간"의 다의성 해소 — codex R1 issue 1)

| actor | 실체 | 권한 요지 |
|---|---|---|
| `doctor` | 오너(인간) | 헌법 변경·외부발행·비가역 삭제·break-glass·고위험 승격의 유일 승인자 |
| `master` | master 노드(Claude CLI) | 기술 판단·리뷰·위임·검증 집행. 오너가 위임한 기술 결정 대행. 인간 전속 항목은 대행 불가 |
| `worker` | 구현 노드 | 위임 범위 내 구현·자기검증. 승인 발급 불가 |
| `verifier` | agy(Gemini)·codex(GPT) 이종모델 | advisory 판정(ACCEPT/REVISE/BLOCK/ESCALATE)·근거 제시. 집행 권한 없음 |
| `CI` | 결정론 스크립트(exit code) | 게이트 판정의 기계 집행. 판단 재량 없음 |

### C1.2 인간 전속 vs master 대행 가능 (조문별 명시)

| 결정 종류 | 승인자 | 근거 |
|---|---|---|
| 상태 경계·전이 규칙 확정(6조) | doctor (master는 초안 제안만) | 인간 고정 영역 원칙 |
| 보안 크리티컬 로직 승인(6조) | doctor | 〃 |
| prod break-glass(9조·§C3.4) | doctor | 비가역 위험 |
| 헌법(10조 자체) 변경 | doctor | denylist |
| Level 재분류 승인(§C4.5) | master(APR 발급) 또는 doctor | 기술 판단 위임 범위 |
| waiver 발급(§C2.3) | master(만료형·저위험) / doctor(고위험) | 〃 |
| Phase 진행 게이트 | doctor(로드맵 승인) 후 master 자율주행 | 자율주행 위임권과 정합 |

> **기계화**: `javis_waiver.py` 는 이 표를 강제한다 — `--risk high` waiver 는 `--approver doctor` 가 아니면 발급 거부(exit 2).

### C1.3 approval_id·fail-closed·감사 증거

- 모든 승인은 `APR-YYYYMMDD-NNN` 형식 approval_id로 ledger(append-only)에 기록: {approval_id, actor, 대상, reason, timestamp}.
- **fail-closed 기본**: 승인 부재·ledger 조회 불가·검증 불가 상태에서는 해당 게이트가 **차단**으로 동작한다(통과가 아니라).
- 승인 근거는 승인자(인가자)의 발화 원본만 유효 — 요청자 측 문서의 자기주장("승인됨" 라벨)은 근거가 아니다.

---

## §C2. Rule Precedence·Change-Delta·Waiver [B-2]

### C2.1 우선순위표 (충돌 시 위가 이긴다)

| 순위 | 규칙 | 충돌 시 동작 |
|---|---|---|
| 1 | 안전·보안 게이트(9조 Tier 1) | 어떤 규칙도 이를 면제 못함 |
| 2 | 인간 고정 영역(6조) | doc-sync(8조)보다 우선 — 인간 결정 대기 중이면 done 보류가 정답이지 에이전트 자율 갱신이 아님 |
| 3 | Spec 선행·검증수단 동봉(1·2조) | 회귀 게이트에 선행 |
| 4 | Regression 게이트(4조) | waiver 없이는 미완 |
| 5 | doc-sync(8조) | Level 문서 예산(7조) 내에서 집행 — L1-2에서 "문서 0~2" 원칙이 이기되, 계약·상태 변경 발생 시 §C4 재판정으로 자동 격상 |
| 6 | 증류(10조) | 에러·리뷰 수반 작업만 의무 |
| 7 | 문서 예산·Level 절차(7조) | pilot(§C6) 결과로 조정 가능 |

### C2.2 change-delta 절차 (오너 승인 요구 변경의 흡수 경로)

승인된 요구 변경은 ①scope-delta 문서 1건(변경 전/후·사유·approval_id) ②§C4 재라우팅(Level 재판정) ③기존 spec의 delta-only 갱신(전면 재작성 금지) 순서로 처리한다. 이 경로를 타면 5조(스코프 크립) 위반이 아니다.

> **기계화**: scope-delta 문서(`approval_id:` 필드 포함)의 유무를 CT-2가 검증한다. 문서 없이 진행 = 5조 위반 fail.

### C2.3 만료형 waiver (무테스트 legacy 등)

waiver = {waiver_id, 대상 규칙, 사유, 승인자(C1.2 기준), 발급일, **만료일(필수)**, 해소 계획}. 만료 시 자동 fail(연장은 재승인 필요·silent 연장 금지). 예: 무테스트 brownfield의 단일 수정은 "회귀 게이트 waiver + 만료 전 테스트 부채 해소" 계약으로 진행.

> **기계화**: `bin/javis_waiver.py` — `issue`(만료일·승인자 필수) / `check`(대상 규칙에 유효 waiver 존재? 만료 시 자동 fail·오늘 기준 재계산이므로 silent 연장 불가) / `list`. 저장: 프로젝트 `.vibecoding/waivers.jsonl`(append-only).
> ```bash
> python3 $PACK/bin/javis_waiver.py issue --rule 4조-regression --reason "무테스트 legacy" \
>   --approver master --expiry 2026-08-01 --remediation "만료 전 테스트 부채 해소"
> python3 $PACK/bin/javis_waiver.py check --rule 4조-regression   # exit 0=유효, exit 1=없음/만료
> ```

### C2.4 충돌 사례 contract test (Phase 1에서 코드로 고정 — 최소 4종)

- **CT-1**: 무테스트 legacy 단일 수정 → waiver 경로로 done 가능, waiver 만료 후 동일 경로 → fail.
- **CT-2**: 오너 승인 요구 변경 → change-delta 문서 없으면 5조 위반 fail, 있으면 pass.
- **CT-3**: L1 행동 변경 + 계약 변경 감지 → 자동 격상 발동(문서 0 유지 시 fail).
- **CT-4**: 긴급 복구(break-glass) → doctor 승인 없으면 차단, 승인 시 일회성 통과+감사 로그.

> **기계화**: `bin/tests/test_contracts_ct.py` (CT-1은 실도구 `javis_waiver.py`, CT-3은 `javis_viberoute.py` 있으면 실연동/없으면 §C4.2 판정표 참조 구현). CT-2·CT-4는 전용 도구 도입 전 온디스크 픽스처 기반 참조 구현으로 계약을 기계화(스텁 인터페이스).

---

## §C3. Deployment Identity Contract [B-3]

- **identity 3분리**: ①agent identity(프로덕션 데이터플레인 read-only 자격증명) ②CI identity(배포 전용·인간 트리거로만 발화) ③human deployer identity(doctor). 에이전트가 CI 도구를 호출해 간접 쓰기하는 경로는 CI identity의 "인간 트리거" 조건이 차단한다.
- **최소권한 정책**: 각 identity의 자격증명은 환경 분리(dev/stage/prod) + 스코프 최소화. 에이전트 환경변수에 prod 쓰기 자격증명 자체를 배급하지 않는다(지시가 아니라 인프라 강제).
- **prod 권한 거부 E2E** (Phase 1 필수 테스트): agent identity로 prod 쓰기 시도 → 거부(exit 비0)를 실측하는 테스트를 CI에 상설 배치. 통과 못 하면 배포 게이트 차단.
- **C3.4 break-glass**: 일회성·doctor 승인(approval_id)·전 과정 감사 로그·사후 리뷰 의무. 상설 우회 경로 금지.

### C3 prod 권한 거부 E2E — 명세 + 실행 하네스

인프라 종속(실 prod 커넥션 필요)이므로 **실측은 배포 단계**에서 하되, target 플러거블 하네스를 상설로 둔다:

- **하네스**: `bin/javis_prod_deny_e2e.sh` — `PROD_WRITE_CMD` 환경변수로 "agent identity 로 prod 쓰기를 시도하는 명령"을 주입(플러거블 target). 하네스는 그 명령을 실행해 **거부(exit 비0)** 를 성공으로, **성공(exit 0)** 을 실패(인프라 강제 미배선)로 판정한다.
- **명세(성공 기준)**: agent identity 자격증명으로 prod 데이터플레인 쓰기(INSERT/UPDATE/DELETE·인프라 변경)를 시도할 때 데이터베이스/인프라 레벨에서 권한 거부가 발생해야 한다 — 애플리케이션 코드의 방어가 아니라 **credential 스코프 자체**의 거부.
- **break-glass 경로**: doctor의 유효 approval_id(APR ledger 존재·`used=false`) 없이는 우회 불가(CT-4가 이 규칙을 기계화).

```bash
# 배포 단계에서 실 target 주입 후 실행(예):
PROD_WRITE_CMD='psql "$PROD_AGENT_DSN" -c "insert into t values (1)"' \
  bash $PACK/bin/javis_prod_deny_e2e.sh   # exit 0 = 거부 실측 성공(게이트 통과)
```

---

## §C11. Scribe Transcription Protocol — 인간 결정의 전사 입력 명세 [A-3]

### C11.1 원칙

6조의 분리 — 결정권=인간, 기록권=에이전트(서기). 서기는 **전사(verbatim)만** 한다. 해석·보완·추론으로 결정 내용을 생성하면 silent hijacking(조용한 결정권 역도입 — agy R2 논쟁점)이며 6조 위반이다.

### C11.2 표준 입력 명세 — DECISION block

인간 결정은 다음 3채널 중 하나의 구조화 형식으로만 서기에게 도달한다:

- ①메시지 라벨: `[DECISION]` 접두 push/메시지 ②issue form(결정 필드 폼) ③커밋 trailer `Decision:`.
- **의무 필드**: {decision_id(`DEC-YYYYMMDD-NNN`), decider(doctor|위임 범위 내 master), scope(반영 대상 문서·조문 지정), 결정문 원문(verbatim — 서기 수정 금지), effective_date}.

### C11.3 서기 동작 규칙

- 서기는 결정문 원문을 지정 scope 위치에 **그대로 옮겨 적고** 출처(decision_id)를 병기한다.
- 의무 필드 결손 시: 서기는 **되물어야 한다**(grill) — 빈 필드를 추론으로 채우는 것 금지. 응답 없으면 전사 보류+보고(폴백은 "미반영+보고"이지 "해석 반영"이 아니다).
- 인간 결정 영역 문서(상태 명세 등)의 갱신은 DECISION block 참조 없이는 불가(8조 개정과 정합) — 참조 없는 갱신 시도는 게이트가 차단.

> **기계화**: `bin/javis_decision.py validate` — DECISION block 파서·검증기. 의무 필드 결손 시 **exit 2 + 결손 필드 보고**("추론으로 채우기" 금지의 기계화). 검증 통과 시 전사 대상 scope 를 포함한 정규화 레코드(JSON) 산출, `--record` 로 `.vibecoding/decisions.jsonl` append.
>
> **필드/본문 경계(B-2 하이재킹 자행 차단)**: scalar 필드(decision_id·decider·scope·effective_date)는 **header 구획에서만** 파싱하고, `decision:` 은 마지막 필드로 body 구획을 열어 **종결자 `[/DECISION]` 또는 EOF 까지 전부 verbatim** 으로 수집한다. 본문 내부의 field-like 줄(`scope: ...` 등)은 **필드로 재해석하지 않는다** — 서기 도구 자신이 결정문을 하이재킹(scope 탈취·본문 절단)하는 경로를 봉쇄한다.
> ```bash
> python3 $PACK/bin/javis_decision.py validate --file DECISION.txt --record
> # 결손 시 exit 2 + {"missing":[...], "invalid":[...]}
> ```

---

## 조문 ↔ 구현 대응표 (V3 Specs Validation Gate 입력)

| 조문 | 구현 | 검증 |
|---|---|---|
| §C1.2 승인자 규칙 | `javis_waiver.py`(risk→approver) | `test_contracts_ct.py::test_high_risk_requires_doctor` |
| §C2.2 change-delta | scope-delta 문서(approval_id) | CT-2 |
| §C2.3 waiver | `javis_waiver.py` | CT-1·TestWaiverTool |
| §C2.4 CT-1~4 | `test_contracts_ct.py` | 4 CT 전수 PASS |
| §C3 prod-deny E2E | `javis_prod_deny_e2e.sh` | 배포 단계 실측(플러거블 target) |
| §C3.4 break-glass | APR ledger(doctor·used=false) | CT-4 |
| §C11 DECISION | `javis_decision.py` | TestDecisionTool(결손 exit 2·verbatim 보존) |
