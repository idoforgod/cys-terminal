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
  createDeferredPaneFocus,
  feedbackOverlayOpen,
  isTopModalLayer,
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

describe("isTopModalLayer — 피드백 창 Esc 는 맨 위 층일 때만 (리뷰 minor #4)", () => {
  const a = { id: "feedback" };
  const b = { id: "confirm" };
  const root = (els: unknown[]) => ({
    querySelectorAll: (sel: string): ArrayLike<unknown> => (sel === MODAL_LAYER_SELECTOR ? els : []),
  });
  it("문서 순서상 마지막 오버레이가 이 창이면 true", () => {
    expect(isTopModalLayer(root([a]), a)).toBe(true);
    expect(isTopModalLayer(root([b, a]), a)).toBe(true);
  });
  it("이 창 뒤에 다른 모달·팔레트가 붙었으면(= 위에 떴으면) false — 그 창의 Esc 를 가로채지 않는다", () => {
    expect(isTopModalLayer(root([a, b]), a)).toBe(false);
  });
  it("판정이 던지거나 root 가 없으면 true(종전 동작 — 피드백 창이 Esc 를 처리한다)", () => {
    const boom = {
      querySelectorAll: (): ArrayLike<unknown> => {
        throw new Error("x");
      },
    };
    expect(isTopModalLayer(boom, a)).toBe(true);
    expect(isTopModalLayer(null, a)).toBe(true);
  });
});

describe("createDeferredPaneFocus — 모달 때문에 건너뛴 pane 포커스를 닫힌 뒤 되살린다 (리뷰 minor #3)", () => {
  /** 모달 층·포커스·관찰자·타이머를 손으로 움직이는 가짜 환경. */
  function env() {
    const s = {
      layer: true,
      lost: true,
      restores: 0,
      watches: 0,
      stops: 0,
      onChange: null as null | (() => void),
      timers: [] as Array<() => void>,
    };
    const deps = {
      layerOpen: () => s.layer,
      focusLost: () => s.lost,
      restore: () => {
        s.restores++;
      },
      watch: (cb: () => void) => {
        s.watches++;
        s.onChange = cb;
        return () => {
          s.stops++;
          s.onChange = null;
        };
      },
      later: (fn: () => void) => {
        s.timers.push(fn);
      },
    };
    const mutate = () => s.onChange?.();
    const runTimers = () => {
      for (const t of s.timers.splice(0)) t();
    };
    return { s, deps, mutate, runTimers };
  }

  it("모달이 닫히면 한 박자 뒤 한 번 복귀하고 관찰을 멈춘다", () => {
    const { s, deps, mutate, runTimers } = env();
    const arm = createDeferredPaneFocus();
    arm(deps);
    expect(s.watches).toBe(1);
    mutate(); // 모달 층이 아직 있다(토스트 등 다른 변화)
    expect(s.timers.length).toBe(0);
    s.layer = false;
    mutate();
    expect(s.restores).toBe(0); // 닫는 키의 남은 이벤트가 pane 으로 가지 않게 즉시 복귀하지 않는다
    runTimers();
    expect(s.restores).toBe(1);
    expect(s.stops).toBe(1);
    mutate();
    runTimers();
    expect(s.restores).toBe(1); // 한 번만
  });
  it("관찰자는 하나만 — 기다리는 동안 여러 번 불려도 새로 만들지 않는다(누적 0)", () => {
    const { s, deps } = env();
    const arm = createDeferredPaneFocus();
    arm(deps);
    arm(deps);
    arm(deps);
    expect(s.watches).toBe(1);
  });
  it("다른 칸(입력칸 등)이 이미 포커스를 가졌으면 빼앗지 않는다", () => {
    const { s, deps, mutate, runTimers } = env();
    const arm = createDeferredPaneFocus();
    arm(deps);
    s.layer = false;
    s.lost = false;
    mutate();
    runTimers();
    expect(s.restores).toBe(0);
    expect(s.stops).toBe(1); // 기다림은 끝낸다
  });
  it("한 박자 사이에 새 모달이 뜨면 복귀하지 않고 그 모달이 닫힐 때까지 계속 기다린다", () => {
    const { s, deps, mutate, runTimers } = env();
    const arm = createDeferredPaneFocus();
    arm(deps);
    s.layer = false;
    mutate();
    s.layer = true; // 확인 창 → 다음 창으로 이어지는 흐름
    runTimers();
    expect(s.restores).toBe(0);
    expect(s.stops).toBe(0);
    s.layer = false;
    mutate();
    runTimers();
    expect(s.restores).toBe(1);
  });
  it("관찰자를 못 만들어도 던지지 않고(setFocus·3초 틱 보호), 다음 호출이 다시 시도한다", () => {
    const { s, deps } = env();
    const arm = createDeferredPaneFocus();
    const bad = {
      ...deps,
      watch: (): (() => void) => {
        throw new Error("no MutationObserver");
      },
    };
    expect(() => arm(bad)).not.toThrow();
    arm(deps);
    expect(s.watches).toBe(1);
  });
  it("복귀가 던져도 삼킨다 — 편의 기능이 타이머 예외를 만들지 않는다", () => {
    const { s, deps, mutate, runTimers } = env();
    const arm = createDeferredPaneFocus();
    arm({
      ...deps,
      restore: () => {
        throw new Error("gone");
      },
    });
    s.layer = false;
    mutate();
    expect(() => runTimers()).not.toThrow();
    expect(s.stops).toBe(1);
  });
});
