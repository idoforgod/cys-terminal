// 「스스로 해결」(0.15.0 · TRACKS §6.6) T0 골격 — 최종 승인 카드 kind 상수 핀. 순수 모듈이라 DOM·Tauri 불요.
import { describe, it, expect } from "bun:test";
import { REPAIR_APPROVAL_KIND, OWNER_CARD_FEED_KINDS, isOwnerCardKind, classifyPendingFeed } from "./feedclass";

describe("수리 승인 카드 kind — T0 는 상수만 · 오너 카드 목록은 그대로", () => {
  it("REPAIR_APPROVAL_KIND === \"repair-approval\"", () => {
    expect(REPAIR_APPROVAL_KIND).toBe("repair-approval");
  });
  // T5b (rewrite: T0 pinned "still four, the repair kind not yet in" — T5b wires the card, C5 site 12)
  it("OWNER_CARD_FEED_KINDS 는 다섯이고 수리 승인 kind 가 들어 있다 — 끝난 좌석 표지 없음 · 팔레트 일괄 승인 제외", () => {
    expect(OWNER_CARD_FEED_KINDS.length).toBe(5);
    expect(OWNER_CARD_FEED_KINDS.indexOf(REPAIR_APPROVAL_KIND)).toBeGreaterThanOrEqual(0);
    expect(isOwnerCardKind(REPAIR_APPROVAL_KIND)).toBe(true);
    expect(classifyPendingFeed({ kind: REPAIR_APPROVAL_KIND, request_id: "rq-01" })).toBe("repair-approval");
  });
});
