// 모달 층 판정 — 순수 함수(DOM 은 인자로 받는다 · 최상위 부수효과 0).
//
// ★왜 있는가(U6 · 0.14.41 반박 D2 blocking): 3초 자가치유 틱의 자동 입양과 `surface.exited`
// 이벤트는 setFocus → term.focus() 를 부른다. 사용자가 모달 입력칸(입력·확인 창)이나
// ⌘K 팔레트 검색칸에 글을 쓰는 도중 그 호출이 오면, 키보드 포커스가 **뒤에 가려진 pane(주로
// 마스터)** 의 xterm 으로 넘어간다. 창은 그대로 떠 있어 사용자는 모르고, 계속 친 글자와 Enter 가
// 마스터 PTY 에 들어간다 — 불만 글이 에이전트 지시로 읽히는 경로(①폭주·역할 분리 축).
// 전역 keydown 의 `.modal-overlay` 검사는 **단축키만** 막고 이 경로는 막지 못한다.
//
// 판정은 DOM 존재로만 한다(별도 플래그 없음) — 플래그가 꼬여 "모든 pane 입력이 조용히 막히는"
// 고착을 구조적으로 없애기 위해서다. 판정이 던지면 false(= 종전 동작: term.focus()) 로 떨어진다.

/** 키보드 입력을 가진 오버레이 층. 확인·입력 창은 전부 `.modal-overlay`, 팔레트는 따로. */
export const MODAL_LAYER_SELECTOR = ".modal-overlay, .palette-overlay";

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

/** createDeferredPaneFocus 의 `arm()` 이 받는 의존성 — DOM·타이머는 전부 주입(순수 함수 유지). */
export interface DeferredPaneFocusDeps {
  /** 모달·팔레트 층이 아직 떠 있는가. */
  layerOpen: () => boolean;
  /** 포커스를 되살려도 되는가(다른 칸이 이미 포커스를 가졌으면 false — 빼앗지 않는다). */
  focusLost: () => boolean;
  /** 건너뛴 pane 포커스를 실제로 되살린다. */
  restore: () => void;
  /** 층 변화를 지켜본다(예: MutationObserver) — 변화마다 콜백을 부르고, 관찰을 멈추는 함수를 돌려준다. */
  watch: (onChange: () => void) => () => void;
  /** 한 박자 뒤로 미룬다(닫는 키의 남은 이벤트가 pane 으로 가지 않게 — 보통 `setTimeout(fn, 0)`). */
  later: (fn: () => void) => void;
}

/**
 * 모달이 떠 있는 동안 setFocus 가 건너뛴 pane 포커스를, 모달이 전부 닫힌 뒤 한 박자 지나 한 번
 * 되살리는 팔(arm)을 만든다(리뷰 minor #3). `arm()` 은 여러 번 불려도 관찰자를 하나만 둔다(누적
 * 0). 기다리는 동안 새 모달이 뜨면(확인 창 → 다음 창) 복귀를 미루고 계속 기다린다. 관찰자를 못
 * 만들거나 복귀가 던져도 삼킨다 — setFocus·3초 틱을 보호하는 편의 기능이라 예외를 내지 않는다.
 */
export function createDeferredPaneFocus(): (deps: DeferredPaneFocusDeps) => void {
  let unwatch: (() => void) | null = null;
  return function arm(deps: DeferredPaneFocusDeps): void {
    if (unwatch) return; // 이미 지켜보는 중(또는 마무리를 기다리는 중) — 새로 만들지 않는다.
    try {
      unwatch = deps.watch(() => {
        if (deps.layerOpen()) return; // 아직 다른 모달이 떠 있다(토스트 등 다른 변화일 수도).
        deps.later(() => {
          if (deps.layerOpen()) return; // 한 박자 사이 새 모달이 떴다 — 그 창이 닫힐 때까지 계속 기다린다.
          const stop = unwatch;
          unwatch = null;
          try {
            stop?.();
          } catch {
            /* 관찰 중단 실패는 무시 — 이미 마무리 단계다 */
          }
          try {
            if (deps.focusLost()) deps.restore();
          } catch {
            /* 복귀는 편의 — 실패해도 창은 이미 닫혔다 */
          }
        });
      });
    } catch {
      unwatch = null; // 관찰자를 못 만들었다 — 다음 arm() 호출이 다시 시도한다.
    }
  };
}

/** main.ts 가 바로 쓰는 단일 팔(arm) — 앱 전체에 창은 한 번에 하나만 뜨므로 관찰자도 하나면 된다. */
export const armDeferredPaneFocus = createDeferredPaneFocus();
