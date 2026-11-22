// purgeconfirm.ts 순수 함수 회귀 테스트 (bun test — 신규 의존성 0). 실사고(2026-07-16) 픽스처 잠금:
// 입력 "Dept-4"(자동 대문자화)·'"dept-4"'(따옴표 포함)가 dept-4 요구에 불일치로 판정되고
// 힌트가 사유를 말해주는지, 입력 가드 속성 3종이 계약대로인지 결정론 검증.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import { purgeNameMatches, purgeMismatchHint, PURGE_INPUT_GUARDS } from "./purgeconfirm";
import { TRASH_KEEP_NOTICE } from "./resetconfirm";

// tsconfig.check.json 은 @types 를 쓰지 않는다 — Bun 전역은 쓰는 만큼만 여기서 선언한다(csosidewiring.test.ts 와 같은 관례).
declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };

describe("purgeNameMatches — 대소문자·따옴표 함정 판정", () => {
  it("정확 일치만 통과(공백 관용)", () => {
    expect(purgeNameMatches("dept-4", "dept-4")).toBe(true);
    expect(purgeNameMatches("  dept-4  ", "dept-4")).toBe(true);
  });
  it("실사고 입력들은 전부 불일치", () => {
    expect(purgeNameMatches("Dept-4", "dept-4")).toBe(false); // macOS 자동 대문자화
    expect(purgeNameMatches('"Dept-4"', "dept-4")).toBe(false); // 스크린샷 그대로
    expect(purgeNameMatches('"dept-4"', "dept-4")).toBe(false); // 따옴표 포함
    expect(purgeNameMatches("", "dept-4")).toBe(false);
  });
});

describe("purgeMismatchHint — 불일치 사유 실시간 표시", () => {
  it("빈 입력·일치 시 빈 문자열(힌트 소음 금지)", () => {
    expect(purgeMismatchHint("", "dept-4")).toBe("");
    expect(purgeMismatchHint("   ", "dept-4")).toBe("");
    expect(purgeMismatchHint("dept-4", "dept-4")).toBe("");
  });
  it("불일치 시 요구 부서명을 명시", () => {
    const h = purgeMismatchHint("Dept-4", "dept-4");
    expect(h).toContain("일치하지 않습니다");
    expect(h).toContain("dept-4");
  });
});

describe("PURGE_INPUT_GUARDS — macOS 자동 교정 차단 계약", () => {
  it("자동 대문자화·자동수정·맞춤법 3종이 고정된다", () => {
    const m = new Map(PURGE_INPUT_GUARDS);
    expect(m.get("autocapitalize")).toBe("off");
    expect(m.get("autocorrect")).toBe("off");
    expect(m.get("spellcheck")).toBe("false");
    expect(PURGE_INPUT_GUARDS.length).toBe(3);
  });
});

// ★S1 M4(리뷰 지적 · RULING-S1-PRODUCT-04) — 완전 삭제(비가역) 확인 창은 부서를 **카드 이름**(주인이 고쳐 사이드바에서 보는
//   이름)으로 먼저 부르고 등록 이름을 그 옆에 둔다(둘이 다를 때 · 같으면 한 번). 입력해야 하는 이름(등록 이름)과 그 판정,
//   지우는 대상은 바뀌지 않는다. main.ts 의 **실제 purgeConfirmModal** 을 떼어 가짜 DOM 위에서 연다(문자열 핀이 아니라 실행).
const MAIN = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
class FakeEl {
  textContent = "";
  innerHTML = "";
  className = "";
  placeholder = "";
  value = "";
  disabled = false;
  attrs: Record<string, string> = {};
  on: Record<string, (e?: unknown) => void> = {};
  private kids = new Map<string, FakeEl>();
  setAttribute(k: string, v: string): void {
    this.attrs[k] = v;
  }
  addEventListener(n: string, f: (e?: unknown) => void): void {
    this.on[n] = f;
  }
  focus(): void {}
  remove(): void {}
  querySelector(sel: string): FakeEl {
    let k = this.kids.get(sel);
    if (!k) {
      k = new FakeEl();
      // 뼈대의 `<button class="modal-yes" disabled>` — 실제 DOM 처럼 처음에는 잠겨 있다
      if (sel === ".modal-yes") k.disabled = this.innerHTML.includes('class="modal-yes" disabled');
      this.kids.set(sel, k);
    }
    return k;
  }
}
type PurgeDialog = {
  title: string;
  body: string;
  placeholder: string;
  type: (v: string) => { enabled: boolean; hint: string };
  confirm: () => Promise<boolean>;
};
/** main.ts 의 실제 purgeConfirmModal(name = 입력해야 하는 등록 이름, 셋째 인자 = 카드 이름)을 가짜 DOM 에서 연다. */
function openPurgeDialog(name: string, ...card: string[]): PurgeDialog {
  const a = MAIN.indexOf("\nfunction purgeConfirmModal(");
  const b = MAIN.indexOf("\n}\n", a);
  expect(a).toBeGreaterThan(0);
  expect(b).toBeGreaterThan(a);
  const src = new Bun.Transpiler({ loader: "ts" }).transformSync(MAIN.slice(a + 1, b + 2));
  const made: FakeEl[] = [];
  const doc = {
    createElement: () => {
      const e = new FakeEl();
      made.push(e);
      return e;
    },
    addEventListener: () => {},
    removeEventListener: () => {},
    body: { appendChild: () => {} },
  };
  const modal = new Function(
    "document", "setTimeout", "purgeNameMatches", "purgeMismatchHint", "PURGE_INPUT_GUARDS", "TRASH_KEEP_NOTICE",
    `${src}\nreturn purgeConfirmModal;`,
  )(doc, () => 0, purgeNameMatches, purgeMismatchHint, PURGE_INPUT_GUARDS, TRASH_KEEP_NOTICE) as (...a: unknown[]) => Promise<boolean>;
  const result = modal(name, { sizeHuman: "1.0 KB", mtime: "2026. 11. 22.", isLast: false, exists: true }, ...card);
  expect(made.length).toBe(1);
  const q = (s: string) => made[0].querySelector(s);
  return {
    title: q("h3").textContent,
    body: q(".modal-label").textContent,
    placeholder: q(".modal-input").placeholder,
    type: (v) => {
      const inp = q(".modal-input");
      inp.value = v;
      inp.on.input?.();
      return { enabled: !q(".modal-yes").disabled, hint: q(".modal-hint").textContent };
    },
    confirm: () => {
      q(".modal-yes").on.click?.();
      return result;
    },
  };
}

describe("★S1 M4 — 완전 삭제 확인 창: 카드 이름 먼저 · 등록 이름은 그 옆 · 입력할 이름과 판정은 그대로(실제 purgeConfirmModal)", () => {
  const REG = "dept-3"; // 등록 이름(레지스트리 · 소켓에서 나온 이름 — 입력해야 하는 이름)
  const CARD = "마케팅팀"; // 주인이 고친 카드 이름
  const TYPED = [REG, `  ${REG}  `, "Dept-3", `"${REG}"`, "", CARD, "dept-"];
  const askLine = (d: PurgeDialog) => d.body.split("\n").filter((l) => l.includes("입력하세요"));

  it("고친 카드: 제목과 입력 안내가 두 이름을 다 부르고 카드 이름이 먼저다 · 안내는 입력할 것이 등록 이름이라고 말하고 그 이름을 보여 준다", () => {
    const d = openPurgeDialog(REG, CARD);
    expect(d.title).toContain(`"${CARD}"`);
    expect(d.title).toContain(`"${REG}"`);
    expect(d.title.indexOf(CARD)).toBeLessThan(d.title.indexOf(REG));
    const ask = askLine(d);
    expect(ask.length).toBe(1);
    expect(ask[0]).toContain(`등록 이름 "${REG}"`);
    expect(ask[0]).toContain(`"${CARD}"`);
    expect(ask[0].indexOf(CARD)).toBeLessThan(ask[0].indexOf(REG));
    expect(d.placeholder).toBe(REG); // 입력칸이 보여 주는 것도 입력할 이름 그대로
  });

  it("대조 — 고치지 않은 카드(카드 이름 = 등록 이름)·카드 이름을 넘기지 않은 호출: 이름은 한 번 · 창의 글은 종전 그대로", () => {
    for (const d of [openPurgeDialog(REG, REG), openPurgeDialog(REG)]) {
      expect(d.title).toBe(`부서 "${REG}" 완전 삭제(부활 차단)`);
      expect(d.body.split(REG).length - 1).toBe(1);
      expect(askLine(d)).toEqual([`계속하려면 아래에 부서명 "${REG}" 을 정확히 입력하세요.`]);
      expect(d.placeholder).toBe(REG);
    }
  });

  it("대조 — 확인 입력은 종전과 같은 것을 받고 같은 것을 거절한다(카드 이름은 받지 않는다) · 받은 뒤 누르면 창은 참을 돌려준다", async () => {
    for (const card of [[CARD], [REG], []]) {
      const d = openPurgeDialog(REG, ...card);
      expect(d.type("x").enabled).toBe(false);
      expect(TYPED.map((v) => d.type(v).enabled)).toEqual(TYPED.map((v) => purgeNameMatches(v, REG)));
      expect(TYPED.map((v) => d.type(v).enabled)).toEqual([true, true, false, false, false, false, false]);
      expect(d.type(CARD).hint).toBe(purgeMismatchHint(CARD, REG));
      d.type(REG);
      expect(await d.confirm()).toBe(true);
    }
  });
});
