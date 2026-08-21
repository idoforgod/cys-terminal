// pending feed 항목의 조작면 분류 — 순수 판정자(main.ts 는 배선만 한다).
// CC 패널(refreshFeed)과 커맨드 팔레트('feed 승인')가 **같은 술어**를 쓰기 위한 단일
// 정의처다 — 패널에서 내린 기만 버튼이 팔레트에 되살아나던 결함(W-4 적대검증 2R)의
// 재발 방지 구조를 그대로 계승한다.
//
// [W4-B · 결함 7] "cycle-verify" 분류 신설 근거:
//   cycle-verify(컨텍스트 순환 전 저장 검증)의 판정자는 **지정 검증자 pane** 뿐이다 —
//   cycle-agent(cys.rs run_cycle_agent)의 영수증 검증 cycle_receipt_ok 가
//   resolver_surface == 지정 검증자 surface 를 요구하는데, GUI 의 Allow 는 operator 토큰
//   경로라 pane 미귀속(resolver_surface=None · state.rs resolve_feed_item_audited)이고,
//   눌러도 ①항목만 resolved 로 소모되고 ②cycle 은 영수증 불일치로 안전 중단(clear
//   미실행)되며 ③검증자의 정상 reply 기회까지 사라진다. 즉 GUI Allow 는 아무 승인도
//   성립시키지 못하는 **기만 버튼**이다(daemon-detected 부류와 동일 계급 — W-4 기만 버튼
//   재도입 금지). ∴ Allow/Deny 를 내리고 '지정 검증자 pane 에서만 판정 가능' 안내 +
//   목록 정리 전용 치우기만 남긴다.
//
// "daemon-detected" 의 판별 기준·위조 불가 논증·fail-closed 폴백 근거는 main.ts
// isDaemonDetectedApproval 의 주석에 있다(서버 파생 필드 daemon_issued 우선 · 필드
// 부재(구 데몬 스큐) 시 "daemon-" 접두 폴백 — 아래 폴백 한 줄이 저장소에 남은 마지막
// 접두 리터럴이다).
// ※ 특례 보존: ceo-promote-request 는 kind 가 달라 "standard" — Allow 경로 그대로
//   (main.ts 의 CEO 승격 Allow 분기 참조).

export type PendingFeedClass = "cycle-verify" | "daemon-detected" | "standard";

export function classifyPendingFeed(i: {
  kind: string;
  request_id: string;
  daemon_issued?: boolean;
}): PendingFeedClass {
  // kind 우선 — cycle-verify 의 발행처는 cys.rs feed.push(kind:"cycle-verify") 하나라
  // daemon_issued 와 실제로 겹치지 않지만, 겹치더라도 '버튼을 내리는' 분류가 이기는
  // 순서가 안전 방향이다(오판이 Allow 를 살리는 쪽으로 나지 않게).
  if (i.kind === "cycle-verify") return "cycle-verify";
  if (i.kind === "approval" && (i.daemon_issued ?? i.request_id.startsWith("daemon-")))
    return "daemon-detected";
  return "standard";
}

// cycle-verify 안내 문구 — refreshFeed 가 그대로 렌더한다(문구는 feedclass.test.ts 가 핀).
// '치우기'의 부작용(진행 중 cycle 안전 중단)을 숨기지 않는다 — 무고지 부작용 금지 관례.
export const CYCLE_VERIFY_NOTE =
  "컨텍스트 순환(cycle) 전 저장 검증 요청입니다 — 판정은 지정 검증자 pane에서 " +
  "`cys feed reply <id> allow|deny`로만 유효합니다. 여기서는 승인할 수 없습니다: " +
  "GUI 승인에는 검증자 영수증(resolver)이 없어 cycle이 안전 중단(clear 미실행)됩니다. " +
  "('알림 치우기'는 판정이 아니라 목록 정리 전용이며, 진행 중인 cycle이 있으면 역시 안전 중단됩니다.)";

export const CYCLE_VERIFY_DISMISS_TITLE =
  "이 알림 항목만 목록에서 지웁니다 — 판정이 아닙니다(decision=dismissed).\n" +
  "⚠ 이 요청을 기다리는 cycle-agent가 아직 돌고 있으면 '검증자 거부(dismissed)'로 " +
  "안전 중단됩니다(clear 미실행) — 검증자 응답 timeout·pane 사망 뒤 잔존 항목의 정리에 쓰세요.";
