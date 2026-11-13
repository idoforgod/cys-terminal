---
name: vibecoding-knowledge
description: NLC 교본 + 세계 최전선 리서치를 담은 바이브코딩 살아있는 지식 베이스. 코딩 중 "이 개념의 정본이 뭐냐"를 온디맨드로 조회하는 참조형 스킬. Level 체계·상태관리 이론·TDD 위치론·Git 철학·경계의 정의·컨텍스트 엔지니어링·헌법 계층론·SDD 생태계·보안 실패 포스트모템·compounding·ACE-FCA를 원전 절 단위로 색인한다. "바이브코딩 원칙/정본 확인, NLC 교본 근거, 이 규칙 왜, Level 몇이냐, 상태관리/경계/헌법 계층 이론" 트리거, 또는 vibecoding 설계·라우팅·리뷰 중 근거가 필요할 때 발동.
---

# vibecoding-knowledge

바이브코딩 체계의 **참조형 지식 베이스**(설계 제안서 §4-F). 절차를 굴리는 스킬이 아니라,
코딩·라우팅·리뷰 도중 "이 개념의 정본이 무엇이냐"를 물을 때 **원전 절 단위로 조회**하는
살아있는 교본이다. 학습지식으로 단정하지 말고, 아래 색인으로 원문을 열어 근거를 인용하라.

> 핵심 규율: 이 스킬은 **progressive disclosure** — SKILL.md는 색인만 담고, 본문은 로드하지
> 않는다. 필요한 주제 1개를 정해 해당 `assets/knowledge/<파일>`의 지정 절만 읽어라. 전 파일
> 통독 금지(컨텍스트 예산 낭비). 인용 시 `파일:절` 또는 `파일:줄`로 추적 가능하게 남긴다.

## 지식 원전 (assets/knowledge/ — 원문 무수정 보존)

- **NLC 다이제스트 4종** (최윤식 박사 교본 v4.0 정독 다이제스트):
  `nlc-digest-1-levels.md` · `nlc-digest-2-rules-meta.md` ·
  `nlc-digest-3-bridge-pipeline.md` · `nlc-digest-4-state-tdd-implement.md`
- **리서치 5종** (2025–2026 세계 최전선 정밀 조사):
  `research-claudecode-practices.md` · `research-sdd-ecosystem.md` ·
  `research-vibecoding-discourse.md` · `research-elite-harness.md` ·
  `research-security-eval.md`

각 리서치 파일 말미의 "출처 URL" 절이 1차 정본 링크를 보존한다(외부 인용 시 그 URL을 병기).

## 주제 → 어느 파일 어느 절 (progressive disclosure 색인)

| 주제 | 파일 | 절 (열 곳) |
|---|---|---|
| **Level 체계** (L1~L6·"성능 아닌 통제 가능성") | `nlc-digest-1-levels.md` | `## Level 체계 전체 구조 (6단계)` + `## Level 1`~`## Level 6` 각 절 · `## 저자 강조 대목 총정리` ③ |
| **상태관리 이론** (State=컴포넌트의 기억·Flux/Context·AI 한계) | `nlc-digest-4-state-tdd-implement.md` | `## 8. 상태관리 설계` + `### 왜 필수인가` + `### 저자 핵심 경고 — AI 한계` + `## 8-1. 페이지 상태 매핑` + `## 8-2. Visual Design` |
| **TDD 위치론** ("Why after spec, before plan"·문서 체인 5종) | `nlc-digest-4-state-tdd-implement.md` | `## 9. TDD` + `### 위치 논리(핵심)` + `### 문서 체인 5종` |
| **Git 철학** (Commit=책임 도장·Branch=판단 분리·5원칙·PR=검증 관문) | `nlc-digest-1-levels.md` | `## Level 2` (Git/GitHub 필수 절) + `## 저자 강조 대목 총정리` ④⑤ |
| **Git 지식-메모리 계층** (ADR/docs-as-memory·같은-PR ADR 규율) | `research-elite-harness.md` | `## 갭 5. 에이전트 메모리·지식 축적` (ADR 포맷·드리프트 방지) |
| **경계의 정의** (Boundary Definition·인간 고정 영역) | `nlc-digest-3-bridge-pipeline.md` | `## 5. ★ 경계의 정의 (Boundary Definition)` |
| **컨텍스트 엔지니어링** (유일 제약·Anthropic 공식) | `research-claudecode-practices.md` | `## 0. 관통 대전제` + `## 7. Context Engineering` |
| ↳ NLC 측 컨텍스트 엔지니어링 | `nlc-digest-1-levels.md` | `## Level 4 — 중급용 / Prompt Driven Restart / Context Engineering` |
| ↳ ACE-FCA 정본 (Frequent Intentional Compaction) | `research-elite-harness.md` | `## 갭 2` → `### ACE-FCA (Advanced Context Engineering ...)` |
| **헌법 계층론** (3문서 계층·SOT·부트스트랩 로더·시행령 온디맨드) | `nlc-digest-2-rules-meta.md` | `## 1단계: Rule 세팅` → `### 세 문서 계층` + `## 2단계: 환경 설정 — 전역 '헌법'` |
| ↳ 문서 지위표·부트스트랩 로더 verbatim | `nlc-digest-3-bridge-pipeline.md` | `### 1-1. "헌법은 완성, 집행은 아직"` + `### 1-7. ★ CLAUDE.md에 넣을 부트스트랩 로더` |
| **SDD 생태계** (Spec Kit·Kiro·EARS·도구 선택·실전 비판) | `research-sdd-ecosystem.md` | `## 1. GitHub Spec Kit` · `## 2. AWS Kiro` · `## 3. EARS 표기법` · `## 6. SDD 실전 비판·한계` · `## 7. 도구 선택 결정 프레임워크` |
| **보안 실패 포스트모템** (7선·RLS·Replit·공통 근본원인) | `research-security-eval.md` | `## 5. 2026 vibe coding 실패 포스트모템 7선` + `### 공통 근본원인 & 규칙(수렴)` |
| ↳ 보안 하드닝 체크리스트·능력 측정 eval | `research-security-eval.md` | `## 1. AI 생성 코드 보안 하드닝` + `## 3. 코딩 에이전트/하네스 능력 측정` (reward hacking 경고) |
| **compounding** (실수→영구교훈 루프·Kieran Klaassen 원형) | `research-elite-harness.md` | `## 갭 1. Compounding Engineering` → `### 에러→영구교훈 루프 (3단계)` |
| **바이브코딩 담론 지형** (타임라인·vs SDD·프로덕션 도입 수치) | `research-vibecoding-discourse.md` | `## 1. 담론의 시간순 흐름` · `## 2. Vibe Coding vs Spec-Driven` · `## 6. 대립 견해 비교` |
| **하네스 베스트 프랙티스** (공식 6원칙·오케스트레이션 5패턴·Ralph Loop) | `research-claudecode-practices.md` | `## 1. 공식 Best Practices` · `## 4. 멀티에이전트 오케스트레이션 5패턴` · `## 5. Ralph Wiggum Loop` |

조회 순서: ①주제 1개 확정 → ②표에서 파일·절 찾기 → ③`Read`로 그 절만 열기 →
④`파일:줄` 인용으로 근거 남기기. 여러 주제가 얽히면 각각 별도로 조회(한 번에 통독 금지).

## 갱신 루프 — 살아있는 교본 (§4-F)

이 지식 베이스는 동결본이 아니다. **분기별(3개월)** 다음 루프를 돈다:

1. **환경 스캐닝** — 리서치 에이전트가 각 리서치 파일의 "출처 URL" 정본을 스윕해 신규
   변화(도구 버전·벤치 수치·포스트모템 추가·공식 문서 개정)를 수집한다. NLC 교본 개정판이
   나오면 다이제스트도 재정독 대상.
2. **지식 스킬 개정** — 확인된 변화만 해당 `assets/knowledge/<파일>`에 반영하고(외과적 —
   원문 절 구조 보존), 이 SKILL.md 색인의 절 이름·줄을 doc-sync한다. 출처 URL은 삭제 금지·추가만.
3. **기억 증류** — 개정에서 나온 재사용 가능한 교훈은 자비스 memory(feedback/reference)로
   증류해 교차 프로젝트 회상에 편입한다(증류 수명주기는 제안서 §C8 lifecycle 준수 — canonical
   SOT는 이 지식 파일, memory는 파생 색인).

> 갱신 무결성: 개정치는 자기보고 금지 — 출처 URL 재현으로 확인한 값만 반영하고,
> self-reported(벤더·리더보드)와 재현 수치를 분리 표기한다([[eval-driven-self-improvement]] 원칙과 정합).

## 관련 스킬

- [[vibecoding-eval]] — 이 지식으로 세운 체계의 실력을 측정하는 eval 하네스.
- [[eval-driven-self-improvement]] — 개선 라운드에서 진짜 이득만 남기는 RSI 루프.
- [[tdd]] — TDD 위치론의 실행 절차(red-green-refactor).
- [[appbuild]] — 스펙 기반 빌드 오케스트레이터(Level 라우팅의 실행 계층).
