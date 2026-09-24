// 층서(z-index) 계약 회귀 핀 — 확인 창·토스트는 Control Center 패널 **위**에 떠야 한다(무반응 = 조용한 실패 금지).
//
// 사건(2026-09-23 · 0.14.41): 승인 Feed 카드의 [확인 창 열기]를 여러 번 눌러도 화면에 아무 변화가 없었다.
// 실체는 "확인 창(.modal-overlay z 1000)은 열렸으나 #cc-panel(z 1500 · 불투명 전체 덮개) 뒤에 깔려 보이지
// 않은 것"이고, 같은 이유로 안내 토스트(#toasts z 99)도 전부 가려져 실패가 조용해졌다(알람 탭에만 남았다).
// 팔레트는 "자기를 먼저 닫는" 우회로 피했지만 패널 안의 카드에는 그 우회가 없었다 — 두 값이 서로를 모른 채
// 따로 정해진 것이 결함의 형태다.
// 설계 정본: 팀만들기-확인창-무반응-수정설계안-최종-20260923.md §5 · §11 R9 · §13 P1.
//
// style.css 를 데이터로 읽어 층서를 **값이 아니라 관계**로 못박는다. 층은 :root 의 --z-* 토큰 한 곳에서
// 정하고(§5-1) 선택자는 var(--z-*) 로 참조한다 — zOf() 는 그 참조를 따라가 정수를 얻고, 숫자 리터럴도
// 그대로 읽는다(토큰 도입 전후를 같은 검사로 대조할 수 있게).
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const CSS = readFileSync(new URL("./style.css", import.meta.url), "utf-8");

type Rule = { sels: string[]; body: string };

/** 주석을 걷고 가장 안쪽 `셀렉터 { 선언 }` 블록만 모은다(@media 안 포함 · @규칙 머리 제외 — hiddenpair.test.ts 와 같은 방식). */
function cssRules(css: string): Rule[] {
  const nc = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const out: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(nc))) {
    const head = m[1].trim();
    if (head.startsWith("@")) continue;
    const sels = head
      .split(",")
      .map((s) => s.trim().replace(/\s+/g, " "))
      .filter(Boolean);
    out.push({ sels, body: m[2] });
  }
  return out;
}
const RULES = cssRules(CSS);

/** 선언 블록에서 속성 값(같은 블록에 두 번이면 뒤의 것). `padding-top` 같은 복합 이름은 `top` 으로 읽지 않는다. */
function declOf(body: string, prop: string): string | null {
  const re = new RegExp(`(?:^|[;\\s])${prop}\\s*:\\s*([^;]+)`, "g");
  let v: string | null = null;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body))) v = m[1].trim();
  return v;
}

/** :root 블록들의 --z-* 토큰 → 정수(뒤에 온 정의가 이긴다). */
function zTokens(): Map<string, number> {
  const t = new Map<string, number>();
  for (const r of RULES) {
    if (!r.sels.includes(":root")) continue;
    for (const m of r.body.matchAll(/(--z-[\w-]+)\s*:\s*(-?\d+)\s*(?=;|$)/g)) t.set(m[1], parseInt(m[2], 10));
  }
  return t;
}
const TOKENS = zTokens();

const VAR_REF = /^var\(\s*(--[\w-]+)\s*(?:,\s*(-?\d+)\s*)?\)$/;

/** z-index 값 → 정수. 숫자 리터럴 · var(--z-x) · var(--z-x, 폴백) 을 따라간다. 해석 불가 = null. */
function resolveZ(v: string | null): number | null {
  if (v == null) return null;
  const lit = /^(-?\d+)$/.exec(v);
  if (lit) return parseInt(lit[1], 10);
  const ref = VAR_REF.exec(v);
  if (ref) return TOKENS.get(ref[1]) ?? (ref[2] != null ? parseInt(ref[2], 10) : null);
  return null;
}

/**
 * 그 선택자 자체에 걸린 규칙(선택자 목록의 한 항목으로 정확히 같은 것 — `body.x #toasts` 같은 문맥 규칙은 제외)의
 * z-index 정수. 여러 규칙이면 뒤의 것(같은 명시도의 캐스케이드). 없으면 null.
 */
function zOf(selector: string): number | null {
  let z: number | null = null;
  for (const r of RULES) {
    if (!r.sels.includes(selector)) continue;
    const v = declOf(r.body, "z-index");
    if (v != null) z = resolveZ(v);
  }
  return z;
}

// ── §5-2 관계 4가지(설계 부록 E 와 같은 네 검사 — 수정 전 3 fail / 1 pass · 수정 후 4 pass) ──
describe("층서 계약 — Control Center 위에 확인 창·토스트", () => {
  const cc = zOf("#cc-panel");
  const modal = zOf(".modal-overlay");
  const toasts = zOf("#toasts");

  test("세 층의 z-index 가 style.css 에 모두 선언돼 있다", () => {
    expect(cc).not.toBeNull();
    expect(modal).not.toBeNull();
    expect(toasts).not.toBeNull();
  });

  test("확인 창(.modal-overlay)은 Control Center(#cc-panel)보다 위다 — 아니면 클릭이 무반응으로 보인다", () => {
    expect(modal!).toBeGreaterThan(cc!);
  });

  test("토스트(#toasts)는 Control Center보다 위다 — 아니면 실패 안내가 조용히 묻힌다", () => {
    expect(toasts!).toBeGreaterThan(cc!);
  });

  test("팔레트(.palette-overlay)보다도 확인 창이 위다 — 팔레트에서 연 확인 창이 가리지 않게", () => {
    const pal = zOf(".palette-overlay");
    expect(pal).not.toBeNull();
    expect(modal!).toBeGreaterThan(pal!);
  });
});

// ── §5-1 층서 토큰 표 — 층은 한 곳에서 **이름으로** 정하고, 다음 사람은 관계를 보고 새 층을 끼운다 ──
describe("층서 토큰 표(§5-1) — :root 한 곳 정의 · 선택자는 토큰 참조", () => {
  // 표의 순서(아래 → 위). §5-1 의 일곱 층 + 원래 값이 표 사이에 있던 드래그 표시 두 층.
  const ORDER = [
    "--z-pane-overlay",
    "--z-ctx-menu",
    "--z-drop-indicator",
    "--z-cc-panel",
    "--z-palette",
    "--z-toast-under-modal",
    "--z-modal",
    "--z-toast",
    "--z-drop-hint",
    "--z-drag-ghost",
  ];

  test("토큰이 전부 :root 에 정의돼 있고 값이 표의 순서대로 엄격히 커진다", () => {
    const missing = ORDER.filter((k) => !TOKENS.has(k));
    expect(missing).toEqual([]);
    const inversions: string[] = [];
    for (let i = 1; i < ORDER.length; i++) {
      const lo = TOKENS.get(ORDER[i - 1])!;
      const hi = TOKENS.get(ORDER[i])!;
      if (!(hi > lo)) inversions.push(`${ORDER[i - 1]}(${lo}) ≮ ${ORDER[i]}(${hi})`);
    }
    expect(inversions).toEqual([]);
  });

  test("핵심 층은 자기 이름의 토큰을 참조한다(숫자 리터럴이 아니다)", () => {
    const want: Record<string, string> = {
      "#cc-panel": "--z-cc-panel",
      ".palette-overlay": "--z-palette",
      ".modal-overlay": "--z-modal",
      "#toasts": "--z-toast",
      "#ctx-menu": "--z-ctx-menu",
      "#drag-ghost": "--z-drag-ghost",
    };
    const got: Record<string, string | null> = {};
    for (const sel of Object.keys(want)) {
      let v: string | null = null;
      for (const r of RULES) if (r.sels.includes(sel)) v = declOf(r.body, "z-index") ?? v;
      got[sel] = v == null ? null : (VAR_REF.exec(v)?.[1] ?? `리터럴 ${v}`);
    }
    expect(got).toEqual(want);
  });

  test("문서 루트 층(position: fixed + z-index)은 전부 토큰 참조다 — 표 밖의 숫자 층 금지", () => {
    const literal = RULES.filter((r) => declOf(r.body, "position") === "fixed")
      .map((r) => ({ sel: r.sels.join(", "), z: declOf(r.body, "z-index") }))
      .filter((x) => x.z != null && !VAR_REF.test(x.z))
      .map((x) => `${x.sel} → ${x.z}`);
    expect(literal).toEqual([]);
  });

  test("var(--z-*) 참조는 전부 정의된 토큰을 가리킨다 — 오타면 z-index 가 조용히 auto 로 떨어진다", () => {
    const dangling: string[] = [];
    for (const r of RULES) {
      const v = declOf(r.body, "z-index");
      const ref = v == null ? null : VAR_REF.exec(v);
      if (ref && ref[1].startsWith("--z-") && !TOKENS.has(ref[1])) dangling.push(`${r.sels.join(", ")} → ${ref[1]}`);
    }
    expect(dangling).toEqual([]);
  });

  test("토스트는 확인 창보다도 위다 — 확인 창이 떠 있는 동안 난 실패 안내도 보여야 한다(그래서 §5-3 회피가 필요하다)", () => {
    expect(zOf("#toasts")!).toBeGreaterThan(zOf(".modal-overlay")!);
  });
});

// ── §5-3 토스트 회피 — 토스트(z 위)가 확인 창 버튼을 덮어 [만들기] 클릭을 가로채지 않게 ──
describe("토스트 회피(§5-3) — 확인 창이 떠 있으면 토스트를 위로 비켜 놓는다", () => {
  const exact = (sel: string) => RULES.filter((r) => r.sels.length === 1 && r.sels[0] === sel);

  for (const sel of ["body:has(.modal-overlay) #toasts", "body.modal-open #toasts"]) {
    test(`${sel} — 아래 고정을 풀고(bottom:auto) 위(top)에 붙인다 · 높이 상한으로 화면 가운데를 넘지 않는다`, () => {
      const rs = exact(sel);
      expect(rs.length).toBeGreaterThan(0);
      const body = rs.map((r) => r.body).join(";");
      expect(declOf(body, "bottom")).toBe("auto");
      expect(declOf(body, "top")).not.toBeNull();
      expect(declOf(body, "max-height")).not.toBeNull();
    });
  }

  test(":has() 규칙과 폴백(body.modal-open) 규칙은 **따로** 선언한다 — 한 선택자 목록에 섞으면 :has() 를 못 읽는 엔진이 규칙 전체를 버린다", () => {
    const mixed = RULES.filter(
      (r) =>
        r.sels.some((s) => s.includes(":has(")) &&
        r.sels.some((s) => s.includes("body.modal-open") || s.includes("body.toast-under-modal")),
    ).map((r) => r.sels.join(", "));
    expect(mixed).toEqual([]);
  });

  // ★(0.14.42 리뷰 F2) 위의 회피는 창의 **아래쪽 절반**(확인 창 버튼 줄)만 지킨다. 피드백·입력·업데이트 창처럼
  //   위쪽에 조작부(닫기 ×·입력칸)가 있는 키 큰 창은 좁은 창에서 토스트(z 위)가 그 조작부를 덮어 클릭을
  //   가로챘다(headless Chrome 1024×700: elementFromPoint(.fb-x) → .toast.feed). 확인 창(confirmModal ·
  //   .confirm-overlay — 본문 + 아래 버튼 줄뿐)이 아닌 창이 떠 있으면 토스트는 그 창 **밑**(Control Center 위)이다.
  //   feedbackmodal.ts 의 안전 계약 "결과·오류는 창 안에 보인다(토스트는 창 밑)" 과 같은 방향이다.
  for (const sel of ["body:has(.modal-overlay:not(.confirm-overlay)) #toasts", "body.toast-under-modal #toasts"]) {
    test(`${sel} — 확인 창 밖의 창이 떠 있으면 토스트는 그 창 밑 · Control Center 위(클릭 가로채기 0 · 실패 안내는 계속 보인다)`, () => {
      const rs = exact(sel);
      expect(rs.length).toBeGreaterThan(0);
      const z = resolveZ(declOf(rs.map((r) => r.body).join(";"), "z-index"));
      expect(z).not.toBeNull();
      expect(z!).toBeLessThan(zOf(".modal-overlay")!);
      expect(z!).toBeGreaterThan(zOf("#cc-panel")!);
      expect(z!).toBeGreaterThan(zOf(".palette-overlay")!);
    });
  }
});
