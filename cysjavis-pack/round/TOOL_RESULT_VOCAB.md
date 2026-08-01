# TOOL_RESULT_VOCAB v1 (초안) — 도구 반환어휘 계약

> 근거: ViMax 연구 OPP-01(도구-증명 계약) · 위치 확정 시 `_round/TOOL_RESULT_VOCAB.md`로 승격.
> 층위: 이것은 **도구 반환값 층** 어휘다 — `_round/EVENT_CONTRACT.md`(노드 간 이벤트 12종)와 별개 층이며 개정 대상이 아니다.
> 목적: "도구가 이 어휘로 보고하지 않으면 해당 상태를 주장하지 않는다"는 환각0 계약의 기계 어휘.

## 어휘 8종 (닫힌 집합 — 미지 값 금지)

| 어휘 | 의미 | exit code 관례 |
|---|---|---|
| `ok` | 요청 동작 완수(단발 동작) | 0 |
| `started` | 장기 작업 개시됨(완료 아님 — 완료 주장 금지) | 0 |
| `completed` | 장기 작업 완료·산출물 실존 확인됨 | 0 |
| `skipped` | 체크포인트 적중으로 생략(산출물 기실존) — B2 계약 연동 | 0 |
| `dependency_missing` | 선행 산출물 부재로 시작하지 않음(부분 실행 없음) | 4 |
| `rejected` | 게이트(자원·비용·보안·검증)가 거부 — 재시도 무의미 아님, 조건 충족 후 재요청 | 3 |
| `fatal` | 영구 실패(4xx류·검증 불가) — 동일 입력 재시도 금지 | 1 |
| `retryable` | 일시 실패(5xx·네트워크·타임아웃) — 유한 재시도 허용(javis_retry 분류와 동일) | 5 |

표준 출력 형식(사람+기계 겸용): `RESULT=<어휘> <자유 서술>` 1줄을 stdout 마지막 줄로.

## 기존 도구 매핑 (1차 적용 대상 — 승격 시 도구별 병기, 기존 exit code 무변경)

| 도구 | 기존 exit | 어휘 병기 |
|---|---|---|
| javis_task checkout | 0 / 9(충돌) / 4(blocker) | ok / rejected / dependency_missing |
| javis_resource_gate check | 0 / 1(soft) / 2(hard) | ok / rejected(soft=경고 후 진행 관례 유지) / rejected |
| javis_retry classify (A1) | 0 / 1 | retryable / fatal |
| B2 check_manifest check | 0 / 1 | skipped / (생성 필요 — 어휘 없음, 판정 전 상태) |
| cys feed push --wait | 0 / 2 / 3 | ok / rejected / retryable(timeout) |

## 디렉티브 반영 문안 (ESCALATE ③ — 오너 승인 대기, 여기서는 초안만)

> "도구가 `completed`를 반환하지 않은 작업을 완료라고 보고하지 않는다. `started`는 완료가 아니다. `skipped`는 산출물 실존이 확인된 경우에만 유효하다."

## 설계 원칙
- 기존 exit code를 바꾸지 않는다(하위호환) — 어휘는 stdout 병기로 도입.
- 새 도구(A1·A2·A3·B2)는 처음부터 이 어휘로 작성.
- 어휘 확장은 이 문서 개정으로만(닫힌 집합 유지 — EVENT_CONTRACT의 미지 타입 거부 원칙과 대칭).
