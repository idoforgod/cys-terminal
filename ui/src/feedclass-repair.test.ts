// 「스스로 해결」(0.15.0 · TRACKS §6.6) T0 골격 — 최종 승인 카드 kind 상수 핀. 순수 모듈이라 DOM·Tauri 불요.
import { describe, it, expect } from "bun:test";
import { REPAIR_APPROVAL_KIND, OWNER_CARD_FEED_KINDS, isOwnerCardKind } from "./feedclass";

describe("수리 승인 카드 kind — T0 는 상수만 · 오너 카드 목록은 그대로", () => {
  it("REPAIR_APPROVAL_KIND === \"repair-approval\"", () => {
    expect(REPAIR_APPROVAL_KIND).toBe("repair-approval");
  });
  it("OWNER_CARD_FEED_KINDS 는 여전히 넷이고 수리 승인 kind 는 아직 없다(배선은 T5b)", () => {
    expect(OWNER_CARD_FEED_KINDS.length).toBe(4);
    expect(OWNER_CARD_FEED_KINDS.indexOf(REPAIR_APPROVAL_KIND)).toBe(-1);
    expect(isOwnerCardKind(REPAIR_APPROVAL_KIND)).toBe(false);
  });
});
