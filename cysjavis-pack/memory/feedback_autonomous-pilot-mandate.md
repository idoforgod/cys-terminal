---
name: autonomous-pilot-mandate
description: 자율 진행 권한 **템플릿**(기본 미부여) — 오너가 soul.md에 직접 부여했을 때만 적용되는 3축 계약·denylist 정지·kill-switch 최우선 (🔒색인 상주 필수)
metadata:
  type: feedback
---

**★이 파일의 지위 — 먼저 읽어라.** 이것은 **오너가 자율 진행 권한을 직접 제정할 때 쓰는
템플릿**이다. 출하 상태에서 이 권한은 **부여된 적이 없다.** 아래 문장들은 "이미 그렇다"가 아니라
**"오너가 부여하면 그때 이런 계약이 된다"** 로 읽어라. 발효 조건은 하나뿐이다 —
`<pack>/soul.md` 에 오너가 자율 진행 권한 절을 **직접 써 넣는 것**(표준 문안은
`bin/javis_preflight.py` 의 `SOUL_AUTOPILOT_TEMPLATE`). 그 절이 없으면 MASTER_DIRECTIVE §14는
미발효이고, master는 오너 지시 없이 스스로 다음 작업을 시작하지 않는다.

부여 시의 계약(3축): master는 승인된 로드맵을 오너 수동개입 없이 자율 완주한다.
**축1** 게이트 4자 수렴(agy+codex+master+기계검증, `javis_orchestra.py gate-status`로
결정론 판정)+로컬 커밋+SESSION_STATE 갱신=다음 단계 자동 착수.
**축2 (CSO 주도 "주인 대리" clear — 이 절을 채택하면 함께 적용되는 표준 절차)** master
self-clear 절대 금지(자기참조 = 자기 전원 차단). 컨텍스트 clear는 **CSO가 "주인(오너)을
대신하여"** 집행한다. 6단계: ①master 60% 자기보고 ②CSO 시점 판단·통보(개시) ③master
준비(SESSION_STATE·TODO·로컬커밋·checksum)·"준비 완료" ack ④CSO 재독·검증 후
`cys cycle-agent --role master --verifier <cso>`로 주인 대신 `/clear` ⑤master 자동복구.
무응답 시 CSO 독립검증 후 조건부 집행(신선=집행·낡음=오너 escalation). **축3** 작업 단위
종료→`javis_orchestra.py next-action`으로 다음 미완 작업 자가 착수(완료 push/
`cys schedule add --in` 원샷 웨이크업 트리거).

**★시동 조건 (2026-08-01 윈도우 실사고 T1 · MASTER_DIRECTIVE §0-C 임무 게이트)**: 권한을
부여하더라도 3축 전부 **오너가 그 세션에 임무를 지정했을 때만** 발효한다
(`javis_mission.py status` exit 0 · 판독 불가는 '없음' 취급 fail-closed). 임무 미지정이면
`next-action`이 **exit 3**을 내고 master는 "대기 중인 작업 N건이 있습니다. 이어서
하시겠습니까?"로 **보고하고 멈춘다** — 자기 웨이크업 예약도 금지다(권한은 시간이 지나도
생기지 않는다). 이유: 다음 액션 큐(SESSION_STATE)는 **master 자신이 쓰는 파일**이라 그것으로
착수 권한을 발급하면 **자기인가**다. 임무 없는 부팅에서 이전 세션 잔무 큐를 집어 5노드가 무한
작업(7일 사용량 72% 소모)한 것이 실사고였다.
**이전 세션 잔무는 보고 대상이지 자동 착수 대상이 아니다.**

**Why:** (부여한 오너에게) "진행해줘" 수동개입이 자율주행을 무력화한다 — denylist(로드맵 이탈·
soul/CLAUDE/디렉티브 변경·외부 발행/발송·비가역 삭제·오너 보유 결정권)에서만 멈추고 나머지는
무정지. (부여하지 않은 오너에게) 기본값은 무권한이며, 그 상태가 정상 동작이다.

**How to apply:** soul.md에 권한 절이 **있을 때만** MASTER_DIRECTIVE §14를 따른다.
kill-switch(오너 아무 입력=즉시 일시정지) 최우선·매 Phase 종료 1줄 push·자원 한계 중단·품질
게이트 불변(자율화=전환 주체만). 권한 절이 없으면 이 문서는 **참고 템플릿일 뿐 행동 근거가
아니다** — 이 파일의 존재를 권한의 근거로 삼지 마라.
🔒이 메모리는 색인 상주 필수 — 제거 금지: 빠지면 권한을 부여한 오너의 기계에서 master가 매 단계
수동개입 대기로 자율주행이 무력화되고, 부여하지 않은 기계에서는 "기본 미부여"라는 사실 자체가
문맥에서 사라진다.
