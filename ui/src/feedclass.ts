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

// [U16 · 0.14.41] "team-create" 분류 신설 근거:
//   팀 만들기 제안(kind=team-create-request)의 일반 Allow 는 **팀을 만들지 않고** 항목만 소각하는
//   기만 버튼이다(생성은 확인 창 [만들기] → addDeptWorkspace 뒤에만 일어난다). 그래서 Allow/Deny 를
//   내리고 카드 전용 두 버튼([확인 창 열기]·[만들지 않기])만 둔다. 팔레트 'feed 승인'은 "standard" 만
//   고르므로 이 분류는 자동으로 제외된다(같은 술어 공유).
// ★성찰 B(minor): kind 리터럴은 teamproposal.ts TEAM_CREATE_KIND 가 정본이다(src/team_spec.rs
//   KIND 와 대조하는 그 상수) — 여기·main.ts 에 사본 리터럴을 두면 kind 가 바뀔 때 여러 파일을
//   같이 고쳐야 한다(샷건 서저리). import 로 단일화.
import { TEAM_CREATE_KIND } from "./teamproposal";

export type PendingFeedClass = "cycle-verify" | "daemon-detected" | "team-create" | "standard";

export function classifyPendingFeed(i: {
  kind: string;
  request_id: string;
  daemon_issued?: boolean;
}): PendingFeedClass {
  // kind 우선 — cycle-verify 의 발행처는 cys.rs feed.push(kind:"cycle-verify") 하나라
  // daemon_issued 와 실제로 겹치지 않지만, 겹치더라도 '버튼을 내리는' 분류가 이기는
  // 순서가 안전 방향이다(오판이 Allow 를 살리는 쪽으로 나지 않게).
  if (i.kind === "cycle-verify") return "cycle-verify";
  if (i.kind === TEAM_CREATE_KIND) return "team-create";
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

// ★U10(0.14.41) 새 feed 항목 토스트 제목 — 정보성 알림과 승인 요청을 가른다.
//   종전: 모든 feed.item.created 가 "📥 승인 요청" 이라, 대기자 없는 안내문(각성 훅 경고·부트 실패·
//   편성 상태·CEO 알림)까지 켤 때마다 '승인 알림' 으로 쌓여 보였다.
//   정보성 kind 의 정본은 데몬 `src/bin/cysd/state.rs` NOTICE_FEED_KINDS / NOTICE_FEED_KIND_PREFIXES 다 —
//   아래는 그 사본이고 feedclass.test.ts 가 두 리터럴을 대조한다(드리프트 = 테스트 적색).
//   모르는 kind 는 승인 요청으로 둔다(안내문을 승인으로 보이는 것보다 승인을 안내문으로 숨기는 쪽이 위험).
export const NOTICE_FEED_KINDS: readonly string[] = [
  "hook-missing",
  "bootstrap-fail",
  "warn",
  "error",
  "formation",
  "ceo-notice",
];
export const NOTICE_FEED_KIND_PREFIXES: readonly string[] = ["formation-"];

export function isNoticeFeedKind(kind: unknown): boolean {
  if (typeof kind !== "string") return false;
  return NOTICE_FEED_KINDS.indexOf(kind) >= 0 || NOTICE_FEED_KIND_PREFIXES.some((p) => kind.startsWith(p));
}

export const FEED_TOAST_NOTICE_TITLE = "ℹ 알림";
export const FEED_TOAST_APPROVAL_TITLE = "📥 승인 요청";

export function feedCreatedToastTitle(kind: unknown): string {
  return isNoticeFeedKind(kind) ? FEED_TOAST_NOTICE_TITLE : FEED_TOAST_APPROVAL_TITLE;
}

// ═════════ 0.14.44 C2·C3 — 부서 항목 · 끝난 좌석 표지 · 정보성 알림 「확인」 ═════════
// 데몬 `feed.list` 가 더한 파생 칸 둘: `waiter`(지금 이 항목의 결정을 기다리는 연결이 있는가) · `publisher_alive`(올린 좌석이 살아 있는가 —
// 올린 좌석을 모르거나 이 데몬이 뜨기 전의 항목이면 null). 옛 데몬은 두 칸이 없다(undefined) — 그때는 아래 판정이 전부 "아니오"라 화면은 종전 그대로다.
//
// ★고친 원칙 둘(설계 C3):
//   ① **표지는 정보일 뿐 단추를 바꾸지 않는다.** 어떤 판정이 틀려도 Allow 가 사라지지 않는다 — 「모르는 종류는 승인 요청으로 둔다」(위 isNoticeFeedKind 주석)와 같은 쪽.
//   ② **한 번에 정리는 정보성 알림에만** 둔다 — 그것도 기다리는 연결이 없고 데몬이 직접 올린 보고가 아닌 것만.

/** 오너의 카드·결정 카드 종류 — 요청한 좌석이 끝났어도 「요청한 좌석이 종료됨」 표지를 붙이지 않는다(기다리지 않고 올리는 것이 규칙인 카드가 있다 — `handlers.rs` 팀 제안은 `--wait` 불가).
 *  `learn_proposal` 은 데몬이 올리는 학습 제안(state.rs NOTICE_FEED_KINDS 주석이 결정성으로 못박은 kind). */
export const OWNER_CARD_FEED_KINDS: readonly string[] = [TEAM_CREATE_KIND, "ceo-promote-request", "cycle-verify", "learn_proposal"];

export function isOwnerCardKind(kind: unknown): boolean {
  return typeof kind === "string" && OWNER_CARD_FEED_KINDS.indexOf(kind) >= 0;
}

export interface FeedFacts {
  kind: string;
  request_id: string;
  status?: string;
  daemon_issued?: boolean;
  /** 데몬 파생 칸 — 옛 데몬은 없다(undefined). true/false 만 사실이고 null·없음은 모름. */
  waiter?: boolean | null;
  publisher_alive?: boolean | null;
}

const daemonIssuedOf = (i: FeedFacts): boolean => i.daemon_issued ?? i.request_id.startsWith("daemon-");

/** 표지 「요청한 좌석이 종료됨」 — 다섯이 모두 참일 때만: 대기 중 · 데몬이 올린 것이 아님 · 정보성 알림이 아님 · `waiter` = false · `publisher_alive` = false.
 *  그리고 오너의 카드 종류면 붙이지 않는다. 주제 좌석(surface_id)은 판정에 쓰지 않는다. 표지는 단추를 바꾸지 않는다(「치우기」를 **더할** 뿐). */
export function isEndedSeatRequest(i: FeedFacts): boolean {
  return (
    (i.status ?? "pending") === "pending" &&
    !daemonIssuedOf(i) &&
    !isNoticeFeedKind(i.kind) &&
    i.waiter === false &&
    i.publisher_alive === false &&
    !isOwnerCardKind(i.kind)
  );
}

/** 정보성 알림의 「확인」 — 정보성 종류 ∧ 기다리는 연결 없음(`waiter` = false)일 때만 Allow·Deny 대신 「확인」 하나(`dismissed`).
 *  종류는 올린 쪽이 스스로 적는 값이라, 정보성 종류로 올라왔어도 **기다리는 연결이 있으면 종전 단추(Allow·Deny)를 그대로 둔다** —
 *  「확인」의 응답은 기다리는 쪽에 거부로 가기 때문이다(마지막 확인 D2 · 원칙 ①). */
export function isConfirmableNotice(i: FeedFacts): boolean {
  return (i.status ?? "pending") === "pending" && isNoticeFeedKind(i.kind) && i.waiter === false;
}

/** 「정보성 알림 N건 모두 확인」 대상 — 정보성 종류 ∧ 기다리는 연결 없음 ∧ **데몬이 올린 것이 아님**. 데몬이 직접 올린 보고(부트 실패·경고·오류)는
 *  한꺼번에 닫지 않고 하나씩 「확인」으로 닫는다 — 부트 실패 보고는 코드가 '조용히 사라지면 안 되는 것'으로 다룬다(`boot_supervisor.rs`). */
export function isBulkConfirmable(i: FeedFacts): boolean {
  return isConfirmableNotice(i) && !daemonIssuedOf(i);
}

/** 한 번에 최대 건수(설계 C3). */
export const BULK_CONFIRM_MAX = 200;

/** 「모두 확인」 단추 옆에 보일 종류별 건수 문구 — 예 `warn 2 · formation-complete 1`(건수 많은 순 · 종류 이름 순). 대상이 없으면 빈 문자열. */
export function bulkConfirmSummary(items: readonly FeedFacts[]): string {
  const counts = new Map<string, number>();
  for (const i of items) {
    if (!isBulkConfirmable(i)) continue;
    counts.set(i.kind, (counts.get(i.kind) ?? 0) + 1);
  }
  const rows = [...counts.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  return rows.map(([k, n]) => `${k} ${n}`).join(" · ");
}

export const ENDED_SEAT_MARK = "요청한 좌석이 종료됨";
export const NOTICE_CONFIRM_LABEL = "확인";
export const BULK_CONFIRM_TITLE = "기다리는 쪽이 없는 정보성 알림을 한 번에 닫습니다(데몬이 직접 올린 보고·승인 요청·결정 카드는 닫지 않습니다).";

/** 부서 항목의 단추 규칙(설계 C2) — 본부 전용 절차가 붙은 두 종류(팀 만들기 제안 · CEO 승격 요청)는 부서 소켓에서 Allow 를 두지 않고 「그 부서로 이동」만. */
export function isHeadOnlyProcedureKind(kind: unknown): boolean {
  return kind === TEAM_CREATE_KIND || kind === "ceo-promote-request";
}
