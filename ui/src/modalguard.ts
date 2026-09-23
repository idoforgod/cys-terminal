// 모달 층 판정 — 순수 함수(DOM 은 인자로 받는다 · 최상위 부수효과 0).
//
// ★왜 있는가(U6 · 0.14.41 반박 D2 blocking): 3초 자가치유 틱의 자동 입양과 `surface.exited`
// 이벤트는 setFocus → term.focus() 를 부른다. 사용자가 모달 입력칸(피드백·입력·확인 창)이나
// ⌘K 팔레트 검색칸에 글을 쓰는 도중 그 호출이 오면, 키보드 포커스가 **뒤에 가려진 pane(주로
// 마스터)** 의 xterm 으로 넘어간다. 창은 그대로 떠 있어 사용자는 모르고, 계속 친 글자와 Enter 가
// 마스터 PTY 에 들어간다 — 불만 글이 에이전트 지시로 읽히는 경로(①폭주·역할 분리 축).
// 전역 keydown 의 `.modal-overlay` 검사는 **단축키만** 막고 이 경로는 막지 못한다.
//
// 판정은 DOM 존재로만 한다(별도 플래그 없음) — 플래그가 꼬여 "모든 pane 입력이 조용히 막히는"
// 고착을 구조적으로 없애기 위해서다. 판정이 던지면 false(= 종전 동작: term.focus()) 로 떨어진다.

/** 키보드 입력을 가진 오버레이 층. 확인·입력·피드백 창은 전부 `.modal-overlay`, 팔레트는 따로. */
export const MODAL_LAYER_SELECTOR = ".modal-overlay, .palette-overlay";

/** 피드백 창 오버레이(= `.modal-overlay` 의 한 종류). pane 드롭 리스너 첫 줄 가드가 본다. */
export const FEEDBACK_OVERLAY_SELECTOR = ".feedback-overlay";

type QueryRoot = { querySelector(sel: string): unknown } | null | undefined;

function has(root: QueryRoot, sel: string): boolean {
  try {
    return root ? root.querySelector(sel) != null : false;
  } catch {
    return false;
  }
}

/** 모달·팔레트가 떠 있는가 — true 면 setFocus 는 xterm 에 포커스를 주지 않는다. */
export function modalLayerOpen(root: QueryRoot): boolean {
  return has(root, MODAL_LAYER_SELECTOR);
}

/** 피드백 창이 떠 있는가 — true 면 OS 드롭은 그 창의 첨부이지 pane 주입이 아니다. */
export function feedbackOverlayOpen(root: QueryRoot): boolean {
  return has(root, FEEDBACK_OVERLAY_SELECTOR);
}

/**
 * 피드백 창의 focusin 되찾기 판정 — 포커스가 **모달 층 밖**(xterm 숨은 textarea 등)으로 나갔으면
 * true(창이 포커스를 되찾는다). 모달 층 안(피드백 창 자신, 그 위에 뜬 다른 확인 창)이면 false —
 * 위에 뜬 확인 창과 포커스를 두고 다투지 않는다.
 */
export function shouldReclaimFocus(target: unknown): boolean {
  const t = target as { closest?: (sel: string) => unknown } | null | undefined;
  if (!t || typeof t.closest !== "function") return false;
  try {
    return t.closest(MODAL_LAYER_SELECTOR) == null;
  } catch {
    return false;
  }
}
