// 인라인 이름 바꾸기(부서·워크스페이스 카드, 그룹 머리)의 **순수부** — main.ts 는 여기에 배선만 한다
// (modalguard.ts·seatlayout.ts 와 같은 관례: 판정은 사이드 모듈 · DOM/부작용은 main.ts).
//
// ────────────────────────────────────────────────────────────────────────────
// ★무엇이 깨져 있었나 (오너 2026-11-22 · S1 수정3 「이름바꾸기가 너무 빨리 닫혀버린다」)
//
//   이름 칸(span)을 그 자리에서 contentEditable 로 바꿔 고치고 blur 에서 끝냈다. 그런데
//   A. 탭 목록은 `renderWsTabs()` 가 매번 **통째로 지우고 다시 짓는다**(bar.innerHTML = ""). 그 호출이
//      좌석 상태 이벤트(status.changed)마다, 승인·피드 이벤트마다, 10초 타이머마다 돈다 — 고치던 칸이
//      버려지고 저장된 옛 이름의 새 카드가 선다. 좌석이 일하는 동안에는 1초에도 여러 번이다.
//   B. `setFocus` 는 **모달**이 떠 있을 때만 터미널 포커스를 참는다. 3초 틱의 자동 입양·좌석 종료가 그것을
//      부르면 터미널이 키보드를 가져가고, 이름 칸은 blur 로 '끝난 것'이 되어 치던 데까지만 저장됐다.
//   그 밖에: 앱 창이 뒤로 가도 blur 로 끝났고, 한글 조합을 확정하는 Enter 를 조합 중에 가로채 끝냈다.
//
// ★수리의 형태
//   · 고치는 칸에 **표식 속성**(data-inline-rename="1")을 단다. 탭 목록 재구성과 터미널 포커스는 이 표식이
//     살아 있는 동안 물러선다(renameBlocksRebuild · inlineRenameOpen). 끝내는 것은 주인뿐이다:
//       Enter(조합 밖) · 앱 안의 다른 곳 클릭(= 창이 포커스를 가진 채 일어난 blur) → 확정
//       Escape → 취소(원래 이름으로)
//     창이 뒤로 가서 생긴 blur 는 끝이 아니다 — 돌아오면 이어서 고친다.
//   · 한글: 조합을 확정하는 Enter(isComposing 또는 keyCode 229)는 그 박자에 끝내지 않고, IME 가 글자를 다
//     쓴 **다음 박자**에 끝낸다 — 확정되는 이름이 화면에 보이는 글자와 같다.
//
// ★불변식(이 모듈이 틀리면 무엇이 깨지는가)
//   1. **자가 회복**: 표식이 남았는데 그 칸이 더는 편집 중이 아니면(포커스를 잃었는데 blur 를 못 받은 경우)
//      재구성을 막지 않는다 — 막으면 카드의 신호가 영영 멈춘다(오너 4대 위험 ③ 주기 자가치유).
//      그래서 재구성 차단은 「표식 ∧ (그 칸이 지금 포커스 ∨ 창이 뒤에 있음)」일 때만이다.
//   2. **전역(total)**: 판독 함수는 어떤 입력에도 던지지 않는다(3초 틱·이벤트 경로에서 불린다).
//   3. **무부작용**: DOM 전역·타이머·저장소를 import 하지도 직접 부르지도 않는다 — 필요한 것은 인자로 받는다.
//      구형 WKWebView 비호환 문법 0.
// ────────────────────────────────────────────────────────────────────────────

/** 고치는 중인 칸에 다는 표식 속성 — CSS·판독의 단일 정의처. */
export const INLINE_RENAME_ATTR = "data-inline-rename";
export const INLINE_RENAME_SELECTOR = '[data-inline-rename="1"]';

interface QueryRoot {
  querySelector(sel: string): unknown;
}

/** root 안에서 이름을 고치는 칸(없으면 null). 전역 — 던지지 않는다. */
export function inlineRenameTarget(root: QueryRoot | null | undefined): unknown {
  try {
    return root ? (root.querySelector(INLINE_RENAME_SELECTOR) ?? null) : null;
  } catch {
    return null;
  }
}

/** root 안에서 이름을 고치는 중인가 — `setFocus` 가 터미널에 키보드를 주기 전에 묻는다. 전역. */
export function inlineRenameOpen(root: QueryRoot | null | undefined): boolean {
  return inlineRenameTarget(root) != null;
}

/**
 * 탭 목록(#ws-tabs)을 지금 다시 지으면 안 되는가.
 * 표식 단 칸이 그 안에 있고, 그 칸이 지금 포커스를 갖고 있거나(주인이 치는 중) 앱 창이 뒤에 있을 때(돌아와 이어 칠
 * 수 있다)만 참이다. 표식은 있는데 창이 앞에 있고 포커스가 다른 데 있으면 **낡은 표식**이다 — 막지 않는다(불변식 1).
 */
export function renameBlocksRebuild(bar: QueryRoot | null | undefined, activeElement: unknown, windowFocused: boolean): boolean {
  const el = inlineRenameTarget(bar);
  if (el == null) return false;
  return el === activeElement || windowFocused !== true;
}

/** 이름 칸에서 쓰는 키 이벤트의 최소 모양. */
export interface RenameKeyEvent {
  key: string;
  isComposing?: boolean;
  keyCode?: number;
  preventDefault(): void;
}
/** 이름 칸(HTMLElement)의 최소 모양 — 테스트는 가짜로, main.ts 는 진짜 엘리먼트로 준다. */
export interface RenameTarget {
  contentEditable: string;
  textContent: string | null;
  focus(): void;
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
  addEventListener(type: string, fn: (e: any) => void): void;
  removeEventListener(type: string, fn: (e: any) => void): void;
}
export interface RenameDeps {
  /** 앱 창이 지금 키보드 포커스를 갖고 있나(document.hasFocus()). */
  windowFocused: () => boolean;
  /** 다음 박자에 실행(setTimeout 0) — IME 가 조합 글자를 다 쓴 뒤에 끝내려고. */
  later: (fn: () => void) => void;
  /** 칸의 글자를 전부 선택(선택 사항 · 던져도 무시한다). */
  selectAll?: () => void;
  /** 끝났다 — commit=true: name(앞뒤 공백을 뗀 지금 글자)으로 확정 · false: 취소(칸은 원래 글자로 되돌려져 있다). 정확히 한 번 불린다. */
  onDone: (commit: boolean, name: string) => void;
}
export interface RenameHandle {
  /** 밖에서 끝낸다(이미 끝났으면 아무 일도 없다). */
  finish(commit: boolean): void;
  /** 아직 고치는 중인가. */
  isOpen(): boolean;
}

/** IME 가 조합 중이거나 조합을 방금 확정한 키인가(Safari·WKWebView 는 조합이 끝난 뒤의 Enter 에도 keyCode 229 를 싣는다). */
export function isImeKey(ke: { isComposing?: boolean; keyCode?: number }): boolean {
  return ke.isComposing === true || ke.keyCode === 229;
}

/**
 * 이름 칸을 그 자리에서 고치기 시작한다. 끝내는 길은 넷뿐이다:
 *   Enter(조합 밖) → 확정 · 조합을 확정하는 Enter → 다음 박자에 확정 · 앱 안 다른 곳 클릭(창이 포커스를 가진 blur) → 확정 ·
 *   Escape(조합 밖) → 취소. 그 밖의 어떤 것(재렌더·상태 이벤트·창이 뒤로 감)도 끝내지 않는다.
 */
export function beginInlineRename(el: RenameTarget, deps: RenameDeps): RenameHandle {
  const before = el.textContent ?? "";
  let open = true;
  let enterPending = false;
  const finish = (commit: boolean): void => {
    if (!open) return;
    open = false;
    el.removeEventListener("keydown", onKey); // rename 마다 리스너 누적 방지
    el.removeEventListener("blur", onBlur);
    el.contentEditable = "false";
    el.removeAttribute(INLINE_RENAME_ATTR);
    if (!commit) el.textContent = before;
    deps.onDone(commit, (el.textContent ?? "").trim());
  };
  const onKey = (ke: RenameKeyEvent): void => {
    if (ke.key === "Enter") {
      ke.preventDefault(); // 이름은 한 줄이다 — 줄바꿈을 넣지 않는다
      if (isImeKey(ke)) {
        // 조합 중인 글자를 확정하는 Enter — IME 가 그 글자를 칸에 다 쓴 뒤(다음 박자)에 끝낸다.
        if (!enterPending) {
          enterPending = true;
          deps.later(() => {
            enterPending = false;
            finish(true);
          });
        }
        return;
      }
      finish(true);
      return;
    }
    if (ke.key === "Escape") {
      if (isImeKey(ke)) return; // 조합을 취소하는 Esc 는 IME 몫
      ke.preventDefault();
      finish(false);
    }
  };
  const onBlur = (): void => {
    let front = true;
    try {
      front = deps.windowFocused() === true;
    } catch {
      front = true;
    }
    if (!front) return; // 창이 뒤로 갔다 — 주인이 끝낸 것이 아니다(돌아오면 이어서 고친다)
    finish(true);
  };
  el.contentEditable = "true";
  el.setAttribute(INLINE_RENAME_ATTR, "1");
  el.addEventListener("keydown", onKey);
  el.addEventListener("blur", onBlur);
  try {
    el.focus();
    if (deps.selectAll) deps.selectAll();
  } catch {
    /* 포커스·선택 실패는 편집을 막지 않는다 */
  }
  return { finish, isOpen: () => open };
}
