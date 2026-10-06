// 0.14.44 C2·C3 — 끝난 좌석 표지 · 정보성 알림 「확인」 · 「모두 확인」 판정(feedclass.ts) 회귀 핀. 순수 모듈이라 DOM·Tauri 불요.
import { describe, it, expect } from "bun:test";
import {
  classifyPendingFeed,
  isEndedSeatRequest,
  isConfirmableNotice,
  isBulkConfirmable,
  bulkConfirmSummary,
  isOwnerCardKind,
  isHeadOnlyProcedureKind,
  OWNER_CARD_FEED_KINDS,
  NOTICE_FEED_KINDS,
  BULK_CONFIRM_MAX,
  type FeedFacts,
} from "./feedclass";
import { TEAM_CREATE_KIND } from "./teamproposal";

const f = (o: Partial<FeedFacts> = {}): FeedFacts => ({ kind: "permission", request_id: "c-1", status: "pending", daemon_issued: false, waiter: false, publisher_alive: false, ...o });

describe("표지 — 다섯이 모두 참일 때만(대기 · 데몬이 올린 것 아님 · 정보성 아님 · waiter=false · publisher_alive=false)", () => {
  it("기준 검체는 표지", () => expect(isEndedSeatRequest(f())).toBe(true));
  it("하나라도 거짓이면 표지 없음", () => {
    expect(isEndedSeatRequest(f({ status: "resolved" }))).toBe(false);
    expect(isEndedSeatRequest(f({ daemon_issued: true }))).toBe(false);
    expect(isEndedSeatRequest(f({ kind: "warn" }))).toBe(false); // 정보성
    expect(isEndedSeatRequest(f({ kind: "formation-complete" }))).toBe(false);
    expect(isEndedSeatRequest(f({ waiter: true }))).toBe(false);
    expect(isEndedSeatRequest(f({ publisher_alive: true }))).toBe(false);
  });
  it("모름(null) · 칸 없음(옛 데몬)은 표지 없음 — 화면은 종전 그대로", () => {
    expect(isEndedSeatRequest(f({ publisher_alive: null }))).toBe(false); // 데몬이 다시 뜬 뒤의 항목 — 좌석 번호로 말할 수 없다
    expect(isEndedSeatRequest(f({ waiter: null }))).toBe(false);
    const old: FeedFacts = { kind: "permission", request_id: "c-1" };
    expect(isEndedSeatRequest(old)).toBe(false);
    expect(isConfirmableNotice({ kind: "warn", request_id: "c-2" })).toBe(false);
  });
  it("오너의 카드(팀 만들기 제안 · CEO 승격 요청 · 저장 검증 · 학습 제안)는 좌석이 끝났어도 표지 없음 — Allow 가 사라지지 않는다", () => {
    for (const kind of [TEAM_CREATE_KIND, "ceo-promote-request", "cycle-verify", "learn_proposal"]) {
      expect({ kind, 카드: isOwnerCardKind(kind), 표지: isEndedSeatRequest(f({ kind })) }).toEqual({ kind, 카드: true, 표지: false });
    }
    expect(OWNER_CARD_FEED_KINDS.length).toBe(4);
  });
  it("request_id 가 daemon- 접두인데 daemon_issued 칸이 없는 옛 데몬 — 데몬이 올린 것으로 본다(표지 없음)", () => {
    expect(isEndedSeatRequest({ kind: "approval", request_id: "daemon-1-2", waiter: false, publisher_alive: false })).toBe(false);
  });
  it("표지는 분류(classifyPendingFeed)를 바꾸지 않는다 — 표지가 붙을 항목도 standard(Allow·Deny 유지)", () => {
    expect(classifyPendingFeed({ kind: "permission", request_id: "c-1", daemon_issued: false })).toBe("standard");
    expect(isEndedSeatRequest(f())).toBe(true);
  });
});

describe("정보성 알림 「확인」 — waiter=false 일 때만 · 기다리는 연결이 있으면 종전 단추", () => {
  it("정보성 종류 여섯 + formation- 접두 ∧ waiter=false → 확인", () => {
    for (const kind of [...NOTICE_FEED_KINDS, "formation-pending"]) expect({ kind, 확인: isConfirmableNotice(f({ kind })) }).toEqual({ kind, 확인: true });
  });
  it("정보성 종류로 올라왔어도 waiter=true 면 확인 아님(Allow·Deny 유지) · waiter 모름이면 확인 아님", () => {
    expect(isConfirmableNotice(f({ kind: "warn", waiter: true }))).toBe(false);
    expect(isConfirmableNotice(f({ kind: "warn", waiter: null }))).toBe(false);
    expect(isConfirmableNotice(f({ kind: "permission", waiter: false }))).toBe(false); // 승인 요청은 정보성이 아니다
  });
  it("데몬이 올린 정보성 알림에도 「확인」은 있다(하나씩 닫는다)", () => {
    expect(isConfirmableNotice(f({ kind: "bootstrap-fail", daemon_issued: true, request_id: "daemon-1-1" }))).toBe(true);
  });
});

describe("「모두 확인」 — 정보성 ∧ waiter 없음 ∧ 데몬이 올린 것 아님", () => {
  const mixed: FeedFacts[] = [
    f({ kind: "warn", request_id: "c-1" }), // 대상
    f({ kind: "formation-complete", request_id: "c-2" }), // 대상
    f({ kind: "warn", request_id: "c-3" }), // 대상
    f({ kind: "bootstrap-fail", request_id: "daemon-9-1", daemon_issued: true }), // 데몬 보고 — 하나씩
    f({ kind: "warn", request_id: "c-4", waiter: true }), // 기다리는 연결 있음
    f({ kind: "permission", request_id: "c-5" }), // 승인 요청
    f({ kind: TEAM_CREATE_KIND, request_id: "c-6" }), // 오너의 카드
    f({ kind: "approval", request_id: "daemon-9-2", daemon_issued: true }), // 화면 감지 승인
    f({ kind: "warn", request_id: "c-7", waiter: null }), // 모름
    f({ kind: "warn", request_id: "c-8", status: "resolved" }), // 이미 닫힘
  ];
  it("대상은 정확히 앞의 셋", () => {
    expect(mixed.filter(isBulkConfirmable).map((i) => i.request_id)).toEqual(["c-1", "c-2", "c-3"]);
  });
  it("승인 요청 · 오너의 카드 · 데몬이 올린 보고(부트 실패 포함) · 기다리는 연결이 있는 정보성 항목은 0건", () => {
    const hit = mixed.filter(isBulkConfirmable).map((i) => i.request_id);
    for (const id of ["c-4", "c-5", "c-6", "c-7", "c-8", "daemon-9-1", "daemon-9-2"]) expect({ id, 대상: hit.includes(id) }).toEqual({ id, 대상: false });
  });
  it("종류별 건수 문구 — 건수 많은 순 · 대상이 없으면 빈 문자열", () => {
    expect(bulkConfirmSummary(mixed)).toBe("warn 2 · formation-complete 1");
    expect(bulkConfirmSummary([f({ kind: "permission" })])).toBe("");
    expect(bulkConfirmSummary([])).toBe("");
  });
  it("한 번에 최대 200건", () => expect(BULK_CONFIRM_MAX).toBe(200));
});

describe("부서 소켓의 단추 규칙", () => {
  it("본부 전용 절차가 붙은 두 종류는 부서에서 Allow 를 두지 않는다 — 팀 만들기 제안 · CEO 승격 요청", () => {
    expect(isHeadOnlyProcedureKind(TEAM_CREATE_KIND)).toBe(true);
    expect(isHeadOnlyProcedureKind("ceo-promote-request")).toBe(true);
    for (const k of ["permission", "approval", "cycle-verify", "warn", "learn_proposal"]) expect({ k, 본부전용: isHeadOnlyProcedureKind(k) }).toEqual({ k, 본부전용: false });
  });
});
