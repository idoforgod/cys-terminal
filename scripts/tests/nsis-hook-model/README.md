# nsis-hook-model — 설치 훅 배치 상태기계의 전수 분기 모델 (W7)

`model.py` 는 `src-tauri/nsis-hooks.nsh`(W4 잠금 무관 배치)의 **배치 상태기계를
파이썬으로 충실히 미러**한 실행 가능 모델이다. 주입 가능한 모든 파일시스템 실패
(delete/extract/verify/vacate/fill/re-check + 복구 연쇄)와 매 단계 뒤 중단
(cancel = `.onUserAbort` 콜백 / kill = 콜백 없음)을 **전수 DFS 로 탐사**하고,
모든 종단 상태에서 훅의 동결 불변식을 기계로 단언한다.

```
python3 scripts/tests/nsis-hook-model/model.py     # exit 0 + 탐사 상태 수 출력
```

## 왜 이 모델인가

이 개발 맥(arm64 · Rosetta 2 부재)에는 x86 PE 를 실행할 기층이 없어 설치기를
**실행**할 수 없다 — 실측 근거는
`_work/win-installer-fix-20260829/logs/W7-windows-runtime.log`. 컴파일 하네스
(`../nsis-hook-compile/`)가 "컴파일된다 + 컴파일 게이트가 살아 있다"를 증명하고,
이 모델이 그 위층을 증명한다 — 정확한 증명 범위는 **"미러 + census 가드"** 다:
(a) 훅의 파이썬 미러가 모든 실패 조합·중단에서 불변식을 지킨다, (b) 훅 실물이
핀 시점에서 표류하지 않았다(아래 가드), 까지. 훅 자체의 실행 증거가 아니며,
실기기 증거(windows-build.yml T4)와 대체 관계가 아니라 보완 관계다.

## 기계 단언 불변식 (훅 R1/R3/R4 의 정직 스코프 = NSIS-CONTRACT §9 반영)

- **I1** 정식(canonical) 이름의 무음 소실 없음: exit 0 이면 정식 3종 = 이번 빌드.
  정식 부재는 exit 3 + `unrecoverable:` 명명 또는 cancel/kill 종단에서만 가능하고,
  동작본으로 시작한 바이너리는 가족 이름 안 어딘가에 동작 사본이 남는다(부팅
  가드 재료 — §9-1 "무음 소실 경로 없음"). 절단 정식(<64KiB)은 exit 0/4 에 절대
  없고, cancel/kill 의 절단 잔존은 의도된 loud Hold 상태다(R2-r2 빈손 삭제 금지).
- **I3** 세대 접두(prefix)·유음 분할: "구 cys + 신 cysd" 방향의 세대 분할은
  exit 0 에서 불가능. 트랜잭션이 **시도한** 바이너리가 구본으로 끝나면 반드시
  토큰에 명명된다(시도조차 못 한 바이너리의 무명은 CONTRACT §3 동결 그대로).
- **I4** 거부는 유음이고 구본은 생존: 완주 실패는 실패 파일 + 동결 분류
  (`unrecoverable/rolled-back` ⇒ 3, 아니면 4). 거부 사유는 동결 6종 코드만.
  구본 소실은 (a) 템플릿이 이미 신본을 제자리 반영 또는 (b) §9-3 절단 창
  (항상 유음 + 가족 안에 동작본 재료 잔존)에서만 허용. 크기 통과 절단 정식은
  반드시 명명된다.

돌 때마다 36종 **심층 레인 커버리지 핀**(거부 6코드 전부, rolled-back, undo 3계급,
LASTDITCH/콜백 승격, tick 슬롯, 세대 분할 유음, §9-3 절단 창, 템플릿 tear, Hold,
★D11 문서-주장 핀 2종 — exit-4 크기 통과 절단 정식 도달 · exit-4 LASTDITCH 승격
분할 도달(NSIS-CONTRACT §9-3·체크리스트 종료코드 규약의 정정 주장을 코드에 단언) …)
이 전부 탐사됐는지 재확인한다 — 미러가 분기점을 잃으면 붉는다(무음 위축 차단).

## 충실도 계약 (훅을 고치는 사람에게)

- 미러 함수마다 모델링한 훅 **앵커**(매크로명·`cys_pl_*` 라벨 계열)를 주석으로
  인용한다 — 라인 번호 금지(N6 과 같은 규칙). **매크로 본문을 바꾸면 같은 커밋에서
  미러를 갱신하라.**
- 기동 시 가드가 훅의 매크로 집합·앵커·토큰 census **+ 모델링된 매크로·콜백
  본문의 정규화(주석·공백 제거) 해시 + POSTINSTALL `!insertmacro` 시퀀스**를
  해시해 `GUARD_PIN` 과 대조한다. 라벨을 다 지킨 본문만의 편집(예: undo 호출
  삭제, ⑧ commit 에 조기 Delete 추가)도 걸린다 — 훅이 표류했는데 모델이 안
  바뀌면 **exit 2 로 거부**한다. 미러 갱신 후 재핀:
  `CYS_MODEL_REPIN=1 python3 model.py`.
- 모델링 범위: PREINSTALL(taskkill GUI + unlock-sweep) · 템플릿 File 추출
  (`SetOverwrite try` + truncate-write tear) · POSTINSTALL 트랜잭션 전체 ·
  실패 분류/실패 파일 · `.onInstFailed`/`.onUserAbort` · NTFS 잠금 3계급
  (image=rename 허용 L2 · share-read=P6 · share-none=§9-6).
  범위 밖: 설치기 싱글톤 뮤텍스(단일 인스턴스 가정), PREUNINSTALL, cysd 부팅
  가드 자체(모델은 가드의 **입력**인 on-disk 재료 보존을 단언), 컴파일 게이트
  (S5 "VERSIONINFO 없는 사이드카"는 `!error` 빌드 실패라 런타임 도달 불가 —
  하네스 N1 이 전담).

## 노브

| env | 기본 | 의미 |
|---|---|---|
| `CYS_MODEL_FAULTS` | 4 | 런당 주입 실패 예산(전수 탐사 깊이) |
| `CYS_MODEL_INT_FAULT_CAP` | 3 | 중단 분기를 제안하는 실패 사용량 상한 |
| `CYS_MODEL_REPIN` | – | `1` 이면 새 GUARD_PIN 출력 후 종료 |

기본 예산: 13 시나리오 × ≈39k 종단 상태, 로컬 ≈1.4s. 예산 6/4 = 21만 상태
≈7s 까지 실측 전green (2026-08-29). CI 는 `ci-branch.yml` `nsis-hook-harness`
잡이 컴파일 하네스와 **같은 레인**에서 이 모델을 돌린다.
