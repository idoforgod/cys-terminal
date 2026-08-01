# appbuild — 웹/앱 빌드 스킬 묶음 (cysjavis 워커 필수)

코드부터 짜지 않고 **기획→검증→자율빌드 루프**로 웹/앱을 만든다. "던져 놓고 자고 일어나니
절반만 만들다 멈춰 있던" 문제의 해법 — cysjavis 엔진 위에 **스펙 기반 프런트엔드 + 감독관
계층**을 얹는다. (출처: 오너 영상 youtu.be/xFUgrOIgtNE — cys 멀티페인 데모.)

## 메우는 3빈칸
①만든 게 맞는지 **검증 루프** ②뭘 충족하면 끝인지 **완료 게이트** ③PRD·화면·작업이 서로
안 맞아도 빌드 끝까지 모르는 문제. 철학: **증거 없는 완료 불인정** · **검증자≠생산자**.

## 아키텍처 — 대표 + 하위 (1st-party 20종)

| 단계 | 대표 | 하위 | 산출 |
|---|---|---|---|
| 1 기획 | `appbuild-plan` | interview(↔grill-me)·debate(↔§6 라운드)·quick | 01-prd·03-architecture |
| 2 화면 | `appbuild-screen-spec` | flow·detail | 02-screens |
| 3 작업 | `appbuild-tasks` | slice(트레이서불릿)·order(INVEST) | 04-tasks (↔to-issues) |
| 4 ★감독관 | `appbuild-supervisor` | collect·verify(13항목)·fix·gate | 05-gate (파생) |
| 5 빌드 | `appbuild-orchestrate` | delegate·verify(다른페인 E2E)·route | 빌드 산출물 |
| — | `appbuild` (대표 오케스트레이터) | — | 전체 파이프라인 |

문서 컨벤션(자체 번호 체계): `.appbuild/` 아래 `01-prd`·`02-screens`·`03-architecture`·
`04-tasks`·`05-gate.md` + `loop-state.json`(중단·재개).

## ★감독관 13항목 검증 (근거: spec-kit /analyze·Kiro·BMad·RTM·INVEST·EARS)
- **일관성**: 고아 기능 0·모델 외 데이터 참조 0·부모 없는 작업 0·용어 드리프트 0
- **추적성**: 순방향 체인 완결·역방향 고아 0·ID 안정/유일/연속/참조해소 (커버리지% 표)
- **갭**: 비-해피 상태(로딩/빈/에러)·인증 권한 명시·입력검증 엣지·비기능 수치(형용사 아님)
- **품질**: 작업 INVEST·수직 슬라이스·모든 요구 테스트 가능(수용 기준)
CRITICAL은 빌드를 막는다. 수정은 승인 후·빠진 곳만·최대 3회, 후 escalation.

## 엔진 재사용 (재구현 아님 — 배선)
debate=§6 gemini·codex 라운드 · build=master 위임+autopilot · verify=`eval-driven-self-improvement`·
`verification-before-completion`(producer≠evaluator) · loop-state=SESSION_STATE 도리.

## 의무 강제 (3중)
1. **WORKER_DIRECTIVE 2-A**: 워커는 웹/앱을 코드부터 시작 금지 — appbuild 파이프라인 필수.
2. **hook 게이트**: `.appbuild` 프로젝트에서 `05-gate.md` 파생 전 소스 작성을 PreToolUse
   hook(`appbuild-gate.sh`, exit 2)이 차단. **비-appbuild 폴더는 fail-open**(무관 작업·cysjavis
   자체 개발 불간섭). self-test 6케이스(차단1·허용5).
3. **preflight C27**: 스킬 20종 프로필 심링크 + 게이트 hook 등록을 결정론 검증(비차단·옵트인).

## 통합
pack 임베드(단일 진실원천) → build.rs 자동 임베드 + 데몬 install + pack.rs 불변식 20종·hook
박제. C27이 프로필 심링크·hook 등록. video-creator와 동형.
