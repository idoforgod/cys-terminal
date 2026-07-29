# cys 터미널 v0.14.6

## 새 기능 — 컨텍스트 사이클 전자동화(fullauto-cycle) 동봉 · 기본 비활성

에이전트 노드의 "컨텍스트 임계 도달 → 저장 → clear → 복원" 사이클과 주요 이벤트 기계 원장 기록을,
사람·LLM 판단 없이 결정론 코드로 수행하는 외곽 자동화 3종이 팩에 동봉된다.

- `cysjavis-pack/bin/javis_cycle_autopilot.py` — 1분 틱 상태기계(측정→안전 게이트 7종→집행→사후검증). 모든 실패는 clear 미실행(fail-closed)
- `cysjavis-pack/bin/javis_cycle_verifier.py` — 전용 pane 상주 결정론 검증자(사이클 직전 baseline 전량 대조·유휴 재확인·모호=deny)
- `cysjavis-pack/bin/javis_state_ledger.py` — 기계 원장(커밋·task done·handoff·사이클 자동 기록, O_APPEND+flock)
- `cysjavis-pack/hooks/fullauto/` 4종 — 훅 템플릿(여기 있는 채로는 발동하지 않음)
- `docs/GUIDE-fullauto-cycle-KR.md` — 운영자 가이드(단계적 활성화 S0→S1·kill-switch 4중·롤백)

**기본 비활성**: 스케줄 잡·settings 훅을 동봉하지 않는다 — 가이드의 opt-in 절차 전까지 어떤 동작도 없다.
기존 `cys cycle-agent`·데몬은 한 줄도 수정되지 않았다.

검증 계보: 설계 판정단 7 + 외부 리뷰 3라운드(적대 검증·BLOCK 2건 전건 수용 반영) + shadow 음성대조 +
라이브 수용시험(실제 워커 노드에서 게이트 7/7 → 결정론 검증자 allow → 5단계 완주 → 컨텍스트 472k→247k 토큰 급락 실증).

## 동반 수리

- fix(gate): step_conditions 실구멍 2건(잡 수준 if·범위 병합) fail-closed + BLOCK-4 회귀 핀 3종 (433b5c4)

## 업데이트

기존 사용자는 앱 내 Update 버튼으로 자동 갱신. 신규 설치는 macOS `.dmg`(공증됨), Windows `-setup.exe`.
