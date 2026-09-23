// 모달 층 판정 핀 — 모달·팔레트가 떠 있는 동안 xterm 이 키보드 포커스를 빼앗지 못하게 하는 판정.
//
// ★왜 필요한가(U6 반박 D2 · blocking): 3초 자가치유 틱의 자동 입양과 `surface.exited` 가 부르는
// setFocus → term.focus() 가, 사용자가 모달 입력칸에 글을 쓰는 도중 **뒤에 가려진 pane(주로 마스터)**
// 으로 포커스를 옮긴다. 창은 그대로 떠 있어 사용자는 모르고, 계속 친 글자와 Enter 가 마스터 PTY 에
// 들어간다 — 불만 글이 에이전트 지시로 읽히는 경로(①폭주·역할 분리 축).
import { describe, it, expect } from "bun:test";
import {
  FEEDBACK_OVERLAY_SELECTOR,
  MODAL_LAYER_SELECTOR,
  feedbackOverlayOpen,
  modalLayerOpen,
  shouldReclaimFocus,
} from "./modalguard";

/** 주어진 클래스 요소들이 문서에 있다고 가정하는 가짜 root(쉼표 선택자 목록을 이해한다). */
const fakeRoot = (present: string[]) => ({
  querySelector: (sel: string): unknown =>
    sel
      .split(",")
      .map((s) => s.trim())
      .some((s) => present.includes(s))
      ? {}
      : null,
});

describe("modalLayerOpen — setFocus 가드의 판정", () => {
  it("아무 층도 없으면 false(종전 동작 그대로 term.focus())", () => {
    expect(modalLayerOpen(fakeRoot([]))).toBe(false);
  });
  it("확인·입력 모달(.modal-overlay)이 있으면 true", () => {
    expect(modalLayerOpen(fakeRoot([".modal-overlay"]))).toBe(true);
  });
  it("⌘K 팔레트(.palette-overlay)가 있어도 true — 검색어가 PTY 로 새지 않게", () => {
    expect(modalLayerOpen(fakeRoot([".palette-overlay"]))).toBe(true);
  });
  it("판정이 던지면 false(fail-open — 포커스 가드가 pane 입력을 영구히 막는 쪽으로 틀리지 않는다)", () => {
    const boom = {
      querySelector: (): unknown => {
        throw new Error("x");
      },
    };
    expect(modalLayerOpen(boom)).toBe(false);
    expect(modalLayerOpen(null)).toBe(false);
  });
  it("선택자 계약", () => {
    expect(MODAL_LAYER_SELECTOR).toContain(".modal-overlay");
    expect(MODAL_LAYER_SELECTOR).toContain(".palette-overlay");
  });
});

describe("feedbackOverlayOpen — pane 드롭 리스너 첫 줄 가드의 판정", () => {
  it("피드백 창이 있을 때만 true(다른 모달에서는 기존 드롭 동작 불변)", () => {
    expect(feedbackOverlayOpen(fakeRoot([".feedback-overlay"]))).toBe(true);
    expect(feedbackOverlayOpen(fakeRoot([".modal-overlay"]))).toBe(false);
    expect(feedbackOverlayOpen(fakeRoot([]))).toBe(false);
    expect(FEEDBACK_OVERLAY_SELECTOR).toBe(".feedback-overlay");
  });
});

describe("shouldReclaimFocus — 피드백 창의 focusin 되찾기", () => {
  const el = (insideLayer: boolean) => ({
    closest: (sel: string): unknown => (insideLayer && sel === MODAL_LAYER_SELECTOR ? {} : null),
  });
  it("포커스가 모달 층 밖(xterm 숨은 textarea 등)으로 나가면 되찾는다", () => {
    expect(shouldReclaimFocus(el(false))).toBe(true);
  });
  it("모달 층 안(피드백 창 자신 · 위에 뜬 다른 확인 창)이면 건드리지 않는다", () => {
    expect(shouldReclaimFocus(el(true))).toBe(false);
  });
  it("대상이 없거나 closest 가 없으면 건드리지 않는다", () => {
    expect(shouldReclaimFocus(null)).toBe(false);
    expect(shouldReclaimFocus({})).toBe(false);
  });
});
