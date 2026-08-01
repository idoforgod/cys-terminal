---
name: vibecoding
description: 자연어 코딩(NLC)을 헌법·복잡도 라우팅·결정론 게이트로 굴리는 대표 오케스트레이터 스킬. 요청을 Level 판정(javis_viberoute)→Level별 문서 세트(vibecoding-docs 위임)→계획 리뷰 게이트(ACE-FCA)→구현 위임(구현자≠검증자)→vibecheck 증거로 완료 판정(javis_task 단일 게이트)→에러·리뷰 증류(javis_distill)까지 순서대로 굴린다. "바이브코딩 / 자연어 코딩 / vibecoding / 스펙 기반으로 만들어 / 이 요청 Level 판정해줘 / 코드부터 짜지 말고 헌법부터", 또는 워커가 신규 웹/앱·기능 구현을 시작할 때 발동(신규 웹/앱 구축의 단일 진입점 — appbuild는 이 안에서 호출).
---

# vibecoding

NLC(자연어 코딩)의 **헌법·문서 파이프라인(무엇을)** 을, 자비스의 **멀티에이전트·결정론
게이트(어떻게)** 위에, **Level 라우팅(언제 얼마나)** 으로 얹고, 매 작업이 규칙을 증류하는
**복리 루프(어떻게 성장하는가)** 로 돌리는 단일 진입점.

> 설계 SOT: `_research/vibecoding-mastery/PROPOSAL-jarvis-vibecoding-system-v3.md`.
> 헌법 정본: `directives/VIBECODING_CONSTITUTION.md`(10조) · 집행: `directives/VIBECODING_ENFORCEMENT.md`(§C1~C11).
> 핵심 철학: **AI는 판단 주체가 아니라 문서 집행기다** · **증거 없는 완료는 인정하지 않는다** ·
> **검증자는 만든 에이전트가 아닌 다른 모델**.

## ⚠ 진입점 규정 — appbuild와의 관계 (트리거 충돌 방지 · §C5 감사 확정)

**신규 웹/앱·기능 구축의 진입점은 vibecoding이다.** `appbuild`는 폐기가 아니라 vibecoding의
**기획·검증 프런트엔드로 하부 편입**된다 — appbuild 단계(plan·screen-spec·tasks·supervisor·
orchestrate)는 vibecoding 파이프라인 **안에서 호출**된다. 트리거가 겹칠 때 우선순위:

- "웹/앱 만들어"·"앱 빌드"·신규 구현 → **vibecoding이 먼저 발동** → Level 판정 후 문서 세트가
  필요하면 그 안에서 `[[appbuild-plan]]`·`[[appbuild-screen-spec]]` 등을 도구로 호출한다.
- 이미 `.appbuild/` 마커가 있는 진행 중 프로젝트의 후속 단계 → `[[appbuild]]` 파이프라인을 그대로 잇는다.
- 둘 다 코드 선행 금지 원칙은 동일하다. vibecoding은 그 위에 **Level 라우팅 + 헌법 게이트 + 증류
  루프**를 더한다.

## 파이프라인 (단계 → 게이트)

```
1. Level 판정      → javis_viberoute judge (§C4 · unknown 처리·needs-grill·fail-closed)
2. 문서 세트       → [[vibecoding-docs]] 위임 (Level별 필수 템플릿 인스턴스화 + critic 감사)
3. ★계획 리뷰 게이트 → ACE-FCA 레버리지 (리서치 리뷰 > 계획 리뷰 > 코드 리뷰) — 필수 통과
4. 구현 위임        → 워커/서브에이전트 (구현자≠검증자)
5. 증거 수집·완료   → javis_vibecheck → javis_task done (단일 완료 게이트)
6. 증류            → javis_distill (에러·리뷰 수반 작업만 · §C8 lifecycle)
```

## 1. Level 판정 — `javis_viberoute` (§C4)

구현 절차 강도를 결정론으로 판정한다. Level은 **오직 여기서만** 산출·기록된다(단일 SOT).

- 6신호를 `evidence`(파일:줄 또는 명령 출력)와 함께 JSON으로 채워 판정한다:
  `persistent_data`·`external_integration`·`deploy_exposure`·`scale_modules`·`brownfield`·`new_service`.
- 실행: `python3 $PACK/bin/javis_viberoute.py judge --input task.json` (stdin은 `--input -`).
- **판정표(first-match-wins, unknown→true 정규화 후)**:

| Level | 조건 | 문서 강도 |
|---|---|---|
| L1-2 | 전 신호 false (스크립트·데모) | 문서 0~2, verify 상시 |
| L3 | scale_modules ∨ brownfield (기존 수정·단일 기능) | requirement + spec + test |
| L4 | persistent_data ∨ external_integration ∨ new_service ∨ deploy_exposure | L3 + state + database + external |
| L5 | deploy_exposure ∧ (new_service ∨ (persistent_data ∧ external_integration)) | 풀 세트(28문서) |

- **unknown 처리(C4.3)**: 신호 unknown은 true로 간주(보수적 격상). unknown이 2개 이상이면
  `needs-grill` 플래그 → `[[grill-me]]`로 의도 합의. **합의 불가·응답 지연 시 폴백은 "격상된
  Level로 진행"** — grill-me가 결정론 폴백을 대체하지 않는다.
- **fail-closed(C4.2)**: 스키마 위반(신호 누락·enum 밖 값·hash 불일치) 입력은 Level을 낮게 추정하지
  않고 **차단**(exit 4). 통과가 아니라 차단이 기본값이다.
- **격상 허용·격하 금지(헌법 7조)**: silent Level 변경 금지. 재분류는 `javis_viberoute reclassify`로
  master/doctor 승인(APR)+reason code(RC-01~04)를 기록해야만 가능하며, 격하(RC-02)는 실행 경로
  무변경 기계 증거를 첨부해야 한다.

## 2. 문서 세트 — `[[vibecoding-docs]]` 위임

판정된 Level의 필수 템플릿을 프로젝트에 인스턴스화한다. **문서를 만드는 노동은 vibecoding-docs에
위임**하고, vibecoding은 Level·완료 기준만 관리한다. Level별 필수 세트·doc-critic 감사·부트스트랩
(CLAUDE.md 브릿지)·세션 sanity check는 전부 `[[vibecoding-docs]]`가 책임진다.

> 헌법 1·8조: Level 임계 이상에서 설계 문서 없는 구현 금지, 계약·상태 변경은 문서 동반 갱신 없이 미완.

## 3. ★계획 리뷰 게이트 — ACE-FCA 레버리지 (필수)

**교정의 레버리지는 뒤로 갈수록 급감한다: 리서치 리뷰 > 계획 리뷰 > 코드 리뷰.** 잘못된 한 줄의
계획이 수백 줄의 잘못된 코드를 만든다 — 그러므로 코드가 아니라 **리서치·계획 단계에서 리뷰를
집중**한다.

- **계획 리뷰 게이트는 필수 통과 관문이다.** 문서 세트(특히 spec·plan)가 완성되면 구현 위임 **전**에
  이종모델(agy·codex) 리뷰 라운드를 돌린다(§6 오케스트레이션). 리뷰 프롬프트에 엄격 제약(지정 파일만·
  무관 파일 배회 금지)을 강제한다.
- 리뷰 verdict는 `_round/REVIEWER_VERDICT_CONTRACT.md` 타입(ACCEPT|REVISE|BLOCK|ESCALATE +
  evidence:file:line)으로 받는다. score(0-100) 금지.
- 계획 리뷰를 통과하지 못하면 구현에 **착수하지 않는다**. 이 게이트가 스코프 크립(헌법 5조)의
  1차 차단선이다.

## 4. 구현 위임 — 구현자≠검증자 (헌법 3조)

- 구현 노동은 워커(Agent 도구 model=opus, 또는 cys 워커)에게 위임한다. master는 직접 구현하지 않는다
  (사소한 마무리 제외).
- **검증은 만든 에이전트가 아닌 다른 페인·다른 모델**이 한다(자기채점 금지). 이종모델 검증(agy·codex),
  벤더 장애 시 failover는 §C10.3.
- 위임 브리프에 Level·필수 문서·완료 기준(통과해야 할 vibecheck 게이트)·"완료 시 master surface로 결과
  한 줄 push"를 동봉한다.

## 5. 증거 수집·완료 — `javis_vibecheck` → `javis_task done` (단일 게이트)

**완료 게이트는 `javis_task` evidence 한 곳뿐이다.** vibecheck·테스트·증류는 **별도 done 게이트가
아니라 evidence 공급자**다(이중 게이트·상이한 skip 의미 제거 — §C5).

- `javis_vibecheck docs` — 문서체인 무결성.
- `javis_vibecheck security` — 보안 Tier 1(secrets·RLS·서버측 인가·관리자 노출). 배포 노출 Level 필수.
- `javis_vibecheck integrity` — test-suite integrity gate(§C7.4: pre-run hash 센서스 → 변조·
  assertion 감소·skip 삽입 검출 시 run 폐기). 기능 테스트보다 **먼저** 돈다.
- 위 증거를 모아 `javis_task set-status <id> done --evidence "<검증명령 → 결과>"`로 완료 전이한다.
  evidence·skip-reason 없는 done은 거부(exit 5). "다 됐다"는 말은 완료가 아니다.

## 6. 증류 — `javis_distill` (§C8 · 헌법 10조)

**에러 수정·리뷰 수반 작업의 종결마다만** 재발 방지 규칙을 증류한다(모든 done 의무 아님 — 의례화 방지).

- lifecycle: confirmed root cause → regression test → `distill propose`(candidate, rule_id 발급 —
  워커 권한) → holdout 재발 검증 → `distill promote`(active 승격, master + holdout ref 필수).
- canonical SOT는 프로젝트 규칙 md 단일 경로(immutable `canonical_locator`). memory는 파생 색인,
  커밋 trailer는 append-only 영수증. 3저장소 동기화 책임자는 **master 단일**(`distill sync-check`).
- 삭제는 없다 — status 전이만(active/superseded/retired).

## 오케스트레이션 규칙

- **헌법 상시·시행령 온디맨드**: `VIBECODING_CONSTITUTION.md`(10조, 짧게)만 상시 로드,
  시행령(§C 조문·`[[vibecoding-runbook]]`·doc 템플릿)은 온디맨드.
- **상황별 런북**: 코드 변경 유형(코딩·리팩터·핫픽스·마이그레이션·테스트·상태·보안·연동·릴리스·
  doc-sync)은 `[[vibecoding-runbook]]`이 라우팅한다.
- **Phase 순서 불변**: 측정 계약(pilot·A/B eval)이 전면 배포에 선행한다 — 규율 먼저 구축 후 효과
  확인의 순서 역전 금지.
- **엔진 재사용**: 리뷰=§6 agy·codex 라운드, 구현=master 위임+autopilot, 검증=eval-driven·
  producer≠evaluator. 재구현이 아니라 배선이다.

## 출력 계약

Level 판정 기록(route-log) + 인스턴스화된 문서 세트 + 계획 리뷰 verdict + 구현 산출물 +
vibecheck 증거 + `javis_task done` 전이 + (에러·리뷰 시) 증류 rule_id. 종료 보고: Level·통과한
게이트 증거·미해결 0. 게이트 미통과면 종료하지 말고 루프를 유지한다.
