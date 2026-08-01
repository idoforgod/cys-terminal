# MEMORY.md — 장기메모리 색인 (빈 골격에서 시작)

> 이 색인은 의도적으로 비어 있다. 장기기억은 빌리는 것이 아니라 사용하며 축적하는 것이다.
> 규칙: 메모리 1건 = 파일 1개(한 가지 사실). 이 파일에는 한 줄 포인터만 둔다.
> 이 색인은 **모든 노드(claude·agy·codex)에 기동 시 자동 주입**되는 공유 의미 기억이다 —
> 본문은 필요할 때 해당 파일을 읽어 점진 로드한다.

## 작성법

새 메모리 파일(`<type>_<slug>.md`):

```markdown
---
name: <kebab-case-슬러그>
description: <한 줄 요약 — 회상 시 관련성 판단에 쓰임>
metadata:
  type: user | feedback | project | reference
---

<사실 본문. feedback/project는 **Why:** 와 **How to apply:** 를 덧붙인다.>
```

- `user` — 오너가 누구인가(역할·전문성·취향)
- `feedback` — 오너가 준 작업 방식 교정·확인 (이유 포함)
- `project` — 진행 중인 일·목표·제약 (코드로 알 수 없는 것만)
- `reference` — 외부 자원 포인터(URL·대시보드·티켓)

저장 전 기존 파일이 이미 다루는지 확인하고, 중복이면 갱신한다. 틀린 메모리는 삭제한다.
**신규 저장은 손편집 대신 `python3 <pack>/bin/javis_memory.py add ...`를 쓴다** —
파일 생성·색인 1줄·중복 검사를 원자적으로 수행한다(`verify`로 정합 기계검증 가능).

## 색인 (한 줄 = 메모리 1건)

<!-- - [제목](파일.md) — 핵심 한 줄 -->
- [자율 진행 권한 템플릿](feedback_autonomous-pilot-mandate.md) — **기본 미부여**. 오너가 soul.md에 직접 부여했을 때만 발효하는 3축 계약·denylist에서만 정지·kill-switch 최우선 (🔒상주 필수 — 제거 금지)
