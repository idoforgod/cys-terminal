// S1 수정3(오너 2026-11-22) — 「이름 변경」이 주인이 끝낼 때까지 열려 있는가.
//
// ★무엇을 고정하나
//   1. 앱의 **실제 갱신 경로**: main.ts 의 `renderWsTabs` 와 `setFocus` 본문을 **소스에서 그대로 뽑아** 가짜 화면
//      위에서 돌린다. 이름을 고치는 중에 탭 목록이 지워지면(원인 A) · 터미널이 키보드를 가져가면(원인 B) 붉다.
//      v0.15.1 의 두 함수는 둘 다 그렇게 한다 — 이 검체가 그 판에서 그 이유로 붉어진다.
//   2. 순수부 inlinerename.ts: 끝내는 길은 Enter · 앱 안 클릭(창이 포커스를 가진 blur) · Escape(취소)뿐이고,
//      한글 조합을 확정하는 Enter 는 IME 가 글자를 쓴 뒤에 끝난다. 창이 뒤로 간 blur 는 끝이 아니다.
//   3. 배선: 카드·그룹 이름 바꾸기가 그 순수부를 쓰고, 옛 `blur → commit` 직결이 남아 있지 않다.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

// tsconfig.check.json 은 @types 를 쓰지 않는다 — Bun 전역은 쓰는 만큼만 여기서 선언한다(deptprogresswiring.test.ts 와 같은 관례).
declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };

const MAIN = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
// 새 모듈은 이름 가져오기가 아니라 동적으로 읽는다 — 모듈이 없는 판(v0.15.1)에서 파일 전체가 적재 실패로 죽지 않고
// 각 검체가 제 이유로 붉어지게 하려는 것이다.
const mod: any = await import("./inlinerename").catch(() => null);

/** main.ts 의 최상위 함수 하나를 소스 그대로 뽑아 JS 로 옮긴다(본문은 손대지 않는다). */
function realFunction(name: string): string {
  const start = MAIN.indexOf(`\nfunction ${name}(`);
  if (start < 0) throw new Error(`main.ts 에 function ${name} 이 없다`);
  const end = MAIN.indexOf("\n}\n", start);
  if (end < 0) throw new Error(`function ${name} 의 끝을 찾지 못했다`);
  return new Bun.Transpiler({ loader: "ts" }).transformSync(MAIN.slice(start + 1, end + 2));
}

// ---------- 가짜 화면(필요한 만큼만) ----------
class FakeEl {
  contentEditable = "inherit";
  textContent: string | null = "";
  attrs = new Map<string, string>();
  listeners = new Map<string, ((e: any) => void)[]>();
  focused = 0;
  constructor(text = "") {
    this.textContent = text;
  }
  focus() {
    this.focused++;
  }
  /** 브라우저처럼: blur() 는 blur 처리기를 부른다(옛 코드는 Enter 에서 label.blur() 로 끝냈다 — 그 길도 그대로 돌게 한다). */
  blur() {
    this.fire("blur");
  }
  setAttribute(n: string, v: string) {
    this.attrs.set(n, v);
  }
  removeAttribute(n: string) {
    this.attrs.delete(n);
  }
  addEventListener(t: string, fn: (e: any) => void) {
    this.listeners.set(t, [...(this.listeners.get(t) ?? []), fn]);
  }
  removeEventListener(t: string, fn: (e: any) => void) {
    this.listeners.set(t, (this.listeners.get(t) ?? []).filter((f) => f !== fn));
  }
  fire(t: string, e: any = {}) {
    for (const fn of [...(this.listeners.get(t) ?? [])]) fn(e);
  }
  count(t: string) {
    return (this.listeners.get(t) ?? []).length;
  }
}
const key = (k: string, extra: Record<string, unknown> = {}) => {
  const e = { key: k, prevented: 0, preventDefault() { e.prevented++; }, ...extra };
  return e;
};
/** 탭 목록(#ws-tabs) 가짜 — innerHTML 을 비우면 그 안의 칸들이 버려진다(브라우저와 같다). */
function fakeBar(children: FakeEl[]) {
  const bar = {
    kids: [...children] as unknown[],
    wipes: 0,
    set innerHTML(_v: string) {
      bar.wipes++;
      bar.kids = [];
    },
    get innerHTML() {
      return "";
    },
    appendChild(n: unknown) {
      bar.kids.push(n);
      return n;
    },
    querySelector(sel: string) {
      if (sel !== '[data-inline-rename="1"]') return null;
      return bar.kids.find((k) => k instanceof FakeEl && k.attrs.get("data-inline-rename") === "1") ?? null;
    },
  };
  return bar;
}

// ────────────────────────────────────────────────────────────────────────────
describe("★앱의 실제 갱신 경로 — main.ts renderWsTabs 를 소스 그대로 돌린다(원인 A)", () => {
  /** 실제 renderWsTabs 를, 탭 하나(ws)와 가짜 화면으로 세운다. buildTab 은 '저장된 이름으로 새 카드'를 만든다(실물과 같은 뜻). */
  function harness(label: FakeEl, activeIsLabel: boolean, windowFocused: boolean) {
    const bar = fakeBar([label]);
    const ws = { id: 1, name: "옛이름", pending: false, groupId: null };
    const document = {
      getElementById: (id: string) => (id === "ws-tabs" ? bar : null),
      activeElement: activeIsLabel ? label : null,
      hasFocus: () => windowFocused,
    };
    const make = new Function(
      "document", "workspaces", "groups", "buildTab", "buildGroupSection", "wsAcctIndex", "renameBlocksRebuild",
      `let wsAcctIdxForRender = null;\n${realFunction("renderWsTabs")}\nreturn renderWsTabs;`,
    );
    const renderWsTabs = make(
      document, [ws], [], (w: { name: string }) => new FakeEl(w.name), () => new FakeEl("group"), () => null,
      mod?.renameBlocksRebuild ?? (() => false),
    ) as () => void;
    return { bar, renderWsTabs };
  }
  const editing = (text: string) => {
    const el = new FakeEl(text);
    el.contentEditable = "true";
    el.attrs.set("data-inline-rename", "1"); // 이름을 고치는 중인 칸
    return el;
  };
  it("이름을 고치는 중(칸이 포커스를 가짐)에는 상태 이벤트·타이머의 재렌더가 와도 그 칸이 버려지지 않는다 — 10번 불러도 치던 글자가 그대로", () => {
    const label = editing("새 이름을 치는 중");
    const { bar, renderWsTabs } = harness(label, true, true);
    for (let i = 0; i < 10; i++) renderWsTabs(); // status.changed · 피드 이벤트 · 10초 타이머가 부르는 그 함수
    expect(bar.wipes).toBe(0);
    expect(bar.kids).toEqual([label]);
    expect(label.textContent).toBe("새 이름을 치는 중");
    expect(label.contentEditable).toBe("true");
  });
  it("앱 창이 잠깐 뒤로 간 동안에도(칸은 포커스 표시를 잃지 않았다) 재렌더가 그 칸을 버리지 않는다", () => {
    const label = editing("치다가 다른 앱을 봄");
    const { bar, renderWsTabs } = harness(label, false, false);
    renderWsTabs();
    expect(bar.wipes).toBe(0);
    expect(bar.kids).toEqual([label]);
  });
  it("고치는 칸이 없으면 종전처럼 통째로 다시 짓는다(신호 반영) — 가드가 평소 갱신을 막지 않는다", () => {
    const plain = new FakeEl("옛이름");
    const { bar, renderWsTabs } = harness(plain, false, true);
    renderWsTabs();
    expect(bar.wipes).toBe(1);
    expect(bar.kids.length).toBe(1);
    expect(bar.kids[0]).not.toBe(plain);
  });
  it("자가 회복: 표식은 남았는데 그 칸이 포커스를 잃었고 창은 앞에 있다(blur 를 못 받은 낡은 표식) → 막지 않고 다시 짓는다", () => {
    const stale = editing("낡은 표식");
    const { bar, renderWsTabs } = harness(stale, false, true);
    renderWsTabs();
    expect(bar.wipes).toBe(1); // 여기서 막으면 카드의 신호가 영영 멈춘다
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("★앱의 실제 포커스 경로 — main.ts setFocus 를 소스 그대로 돌린다(원인 B)", () => {
  function harness(renameOpen: boolean, modalOpen: boolean) {
    const counts = { termFocus: 0, deferred: 0, ftRoot: 0, toggled: 0 };
    const rt = { el: { classList: { toggle: () => void counts.toggled++ } }, term: { focus: () => void counts.termFocus++ } };
    const panes = new Map<string, typeof rt>([["k7", rt]]);
    const label = new FakeEl("고치는 중");
    if (renameOpen) label.attrs.set("data-inline-rename", "1");
    const document = { querySelector: (sel: string) => (sel === '[data-inline-rename="1"]' && renameOpen ? label : null) };
    const make = new Function(
      "document", "panes", "paneKey", "current", "modalLayerOpen", "deferPaneFocusUntilModalsClose", "updateFtRoot", "inlineRenameOpen",
      `let focusedSid = null;\n${realFunction("setFocus")}\nreturn { setFocus, focused: () => focusedSid };`,
    );
    const api = make(
      document, panes, (sid: number) => "k" + sid, () => ({ socket: undefined }), () => modalOpen,
      () => void counts.deferred++, () => void counts.ftRoot++, mod?.inlineRenameOpen ?? (() => false),
    ) as { setFocus: (sid: number) => void; focused: () => number | null };
    return { counts, api };
  }
  it("이름을 고치는 중에는 3초 틱의 자동 입양·좌석 종료가 setFocus 를 불러도 터미널이 키보드를 가져가지 않는다(표시는 갱신된다)", () => {
    const { counts, api } = harness(true, false);
    api.setFocus(7);
    api.setFocus(7);
    expect(counts.termFocus).toBe(0);
    expect(api.focused()).toBe(7); // 어느 칸이 선택됐는지는 종전대로 기억한다
    expect(counts.toggled).toBeGreaterThan(0);
    expect(counts.ftRoot).toBe(2);
  });
  it("고치는 중이 아니면 종전 그대로: 터미널에 포커스 · 모달이 떠 있으면 미뤘다가 되살린다", () => {
    const a = harness(false, false);
    a.api.setFocus(7);
    expect(a.counts.termFocus).toBe(1);
    expect(a.counts.deferred).toBe(0);
    const b = harness(false, true);
    b.api.setFocus(7);
    expect(b.counts.termFocus).toBe(0);
    expect(b.counts.deferred).toBe(1);
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("순수부 inlinerename.ts — 끝내는 것은 주인뿐이다", () => {
  function open(text: string, windowFocused = true) {
    const el = new FakeEl(text);
    const state = { front: windowFocused, later: [] as (() => void)[], done: [] as [boolean, string][], selected: 0 };
    const h = mod.beginInlineRename(el, {
      windowFocused: () => state.front,
      later: (fn: () => void) => void state.later.push(fn),
      selectAll: () => void state.selected++,
      onDone: (c: boolean, n: string) => void state.done.push([c, n]),
    });
    return { el, state, h };
  }
  it("시작: 칸이 편집 가능 · 표식 · 포커스 · 전체 선택 — 아직 아무것도 끝나지 않았다", () => {
    const { el, state, h } = open("기획부");
    expect(el.contentEditable).toBe("true");
    expect(el.attrs.get(mod.INLINE_RENAME_ATTR)).toBe("1");
    expect(el.focused).toBe(1);
    expect(state.selected).toBe(1);
    expect(h.isOpen()).toBe(true);
    expect(state.done).toEqual([]);
  });
  it("Enter → 지금 글자(앞뒤 공백 제거)로 확정 · 표식과 리스너가 걷힌다 · 줄바꿈은 들어가지 않는다", () => {
    const { el, state, h } = open("기획부");
    el.textContent = "  전략기획부 ";
    const e = key("Enter");
    el.fire("keydown", e);
    expect(e.prevented).toBe(1);
    expect(state.done).toEqual([[true, "전략기획부"]]);
    expect(el.contentEditable).toBe("false");
    expect(el.attrs.has(mod.INLINE_RENAME_ATTR)).toBe(false);
    expect(el.count("keydown") + el.count("blur")).toBe(0);
    expect(h.isOpen()).toBe(false);
  });
  it("앱 안의 다른 곳을 클릭(창이 포커스를 가진 채 blur) → 확정", () => {
    const { el, state } = open("기획부");
    el.textContent = "영업부";
    el.fire("blur");
    expect(state.done).toEqual([[true, "영업부"]]);
  });
  it("★앱 창이 뒤로 가서 생긴 blur 는 끝이 아니다 — 돌아와 이어 치고 Enter 로 끝낸다", () => {
    const { el, state, h } = open("기획부");
    el.textContent = "영";
    state.front = false;
    el.fire("blur"); // 다른 앱이 앞으로 왔다
    expect(state.done).toEqual([]);
    expect(h.isOpen()).toBe(true);
    expect(el.contentEditable).toBe("true");
    state.front = true;
    el.textContent = "영업부";
    el.fire("keydown", key("Enter"));
    expect(state.done).toEqual([[true, "영업부"]]);
  });
  it("Escape → 취소: 칸이 원래 이름으로 돌아가고 onDone(false, 원래 이름)", () => {
    const { el, state } = open("기획부");
    el.textContent = "잘못 친 이";
    const e = key("Escape");
    el.fire("keydown", e);
    expect(e.prevented).toBe(1);
    expect(el.textContent).toBe("기획부");
    expect(state.done).toEqual([[false, "기획부"]]);
  });
  it("★한글: 조합을 확정하는 Enter(isComposing) 는 그 박자에 끝내지 않는다 — IME 가 글자를 쓴 다음 박자에, 보이는 글자 그대로 확정", () => {
    const { el, state, h } = open("기획부");
    el.textContent = "전략기ㅎ"; // 마지막 글자가 아직 조합 중
    el.fire("keydown", key("Enter", { isComposing: true, keyCode: 229 }));
    expect(state.done).toEqual([]); // 반쯤 조합된 글자로 끝나지 않았다
    expect(h.isOpen()).toBe(true);
    el.textContent = "전략기획"; // IME 가 조합을 마쳐 칸에 쓴다
    expect(state.later.length).toBe(1);
    state.later[0]();
    expect(state.done).toEqual([[true, "전략기획"]]);
  });
  it("★한글(Safari·WKWebView): 조합이 끝난 뒤 오는 Enter 는 isComposing=false 이고 keyCode 만 229 다 — 같은 길로 간다", () => {
    const { el, state } = open("기획부");
    el.textContent = "영업";
    el.fire("keydown", key("Enter", { isComposing: false, keyCode: 229 }));
    expect(state.done).toEqual([]);
    state.later[0]();
    expect(state.done).toEqual([[true, "영업"]]);
  });
  it("한글(Chromium·WebView2): 조합 Enter(229) 뒤에 진짜 Enter(13)가 한 번 더 온다 — 확정은 한 번뿐이다", () => {
    const { el, state } = open("기획부");
    el.textContent = "영업";
    el.fire("keydown", key("Enter", { isComposing: true, keyCode: 229 }));
    el.fire("keydown", key("Enter", { isComposing: false, keyCode: 13 }));
    expect(state.done).toEqual([[true, "영업"]]);
    for (const fn of state.later) fn(); // 미뤄 둔 박자가 와도 다시 끝내지 않는다
    expect(state.done.length).toBe(1);
  });
  it("조합 중의 Escape 는 IME 몫(조합 취소) — 이름 바꾸기는 계속 열려 있다", () => {
    const { el, state, h } = open("기획부");
    const e = key("Escape", { isComposing: true, keyCode: 229 });
    el.fire("keydown", e);
    expect(e.prevented).toBe(0);
    expect(h.isOpen()).toBe(true);
    expect(state.done).toEqual([]);
  });
  it("다른 키는 아무것도 끝내지 않는다 · 끝난 뒤의 finish·키·blur 는 무시(onDone 은 정확히 한 번)", () => {
    const { el, state, h } = open("기획부");
    for (const k of ["a", "Tab", "ArrowLeft", "Backspace", " "]) el.fire("keydown", key(k));
    expect(state.done).toEqual([]);
    h.finish(true);
    h.finish(false);
    el.fire("blur");
    el.fire("keydown", key("Enter"));
    expect(state.done.length).toBe(1);
  });
  it("이름을 다 지우고 끝내면 빈 문자열로 확정된다(무엇으로 바꿀지는 호출측의 종전 규칙)", () => {
    const { el, state } = open("기획부");
    el.textContent = "   ";
    el.fire("keydown", key("Enter"));
    expect(state.done).toEqual([[true, ""]]);
  });
  it("판독 함수는 어떤 입력에도 던지지 않는다 · 낡은 표식은 재구성을 막지 않는다", () => {
    const boom = { querySelector: () => { throw new Error("boom"); } };
    for (const r of [null, undefined, boom, { querySelector: () => null }]) {
      expect(mod.inlineRenameOpen(r)).toBe(false);
      expect(mod.renameBlocksRebuild(r, null, true)).toBe(false);
    }
    const el = new FakeEl("x");
    const bar = { querySelector: () => el };
    expect(mod.inlineRenameOpen(bar)).toBe(true);
    expect(mod.renameBlocksRebuild(bar, el, true)).toBe(true); // 주인이 치는 중
    expect(mod.renameBlocksRebuild(bar, null, false)).toBe(true); // 창이 뒤에 있다 — 돌아와 이어 칠 수 있다
    expect(mod.renameBlocksRebuild(bar, null, true)).toBe(false); // 낡은 표식
    const thrower = new FakeEl("x");
    const h = mod.beginInlineRename(thrower, { windowFocused: () => { throw new Error("boom"); }, later: () => {}, onDone: () => {} });
    expect(() => thrower.fire("blur")).not.toThrow();
    expect(h.isOpen()).toBe(false); // 창 상태를 못 읽으면 종전처럼 blur 로 끝낸다
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★부서장 판정 02(2026-11-22): 원인 A·B 를 고쳐도, 주인이 확정한 이름이 **자리표시와 모양이 같으면**(dept-9 · … ·
//   non title · ws 3) 다음 기동의 이름 회복이 레지스트리 표시명으로 덮었다 — 오너 요구 4·5(새로고침·재시작 뒤에도
//   이름이 남는다)가 깨지는 자리다. 여기서는 main.ts 의 **실제 saveLayout** 과 **실제 기동 회복 루프**를 소스 그대로
//   뽑아 「이름 변경 → 저장 → 다시 읽기 → 기동 회복」을 돌린다.
describe("★판정 02 — 주인이 정한 이름은 저장되고, 기동의 이름 회복이 덮지 않는다(실제 saveLayout + 실제 회복 루프)", () => {
  const UNTITLED = "non title";
  const SOCK = "/tmp/s1-dept-9.sock";
  /** 실제 saveLayout 으로 저장한 JSON 문자열. */
  function persist(workspaces: Record<string, unknown>[]): string {
    let saved = "";
    const make = new Function(
      "layoutLoaded", "normalizeWorkspaces", "normalizeGroups", "workspaces", "groups", "activeWs", "wsCounter", "groupCounter",
      "localStorage", "LAYOUT_KEY", "csoSideDone",
      `${realFunction("saveLayout")}\nreturn saveLayout;`,
    );
    (make(true, (w: unknown) => w, (_n: unknown, g: unknown) => g, workspaces, [], 0, 9, 1,
      { setItem: (_k: string, v: string) => void (saved = v) }, "cys-layout-v2", true) as () => void)();
    return saved;
  }
  /** 실제 기동 회복 루프(start 안의 「구버전 자동 번호 이름」 ~ 다음 주석 사이)를 그 워크스페이스들에 돌린다. */
  function restoreNames(workspaces: Record<string, unknown>[], registryName: string | null) {
    const a = MAIN.indexOf('  // 구버전 자동 번호 이름("ws N")은 미정 표시로 이행');
    const b = MAIN.indexOf("  // ★이 세 줄은 이제 **그물이지 보장이 아니다.**", a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const loop = new Bun.Transpiler({ loader: "ts" }).transformSync(MAIN.slice(a, b));
    const displayBySocket = new Map<string, string>(registryName ? [[SOCK, registryName]] : []);
    new Function("workspaces", "UNTITLED", "displayBySocket", "sameSocket", loop)(workspaces, UNTITLED, displayBySocket, (x: string, y: string) => x === y);
  }
  for (const name of ["dept-9", "…", "non title", "ws 3", "전략기획부"]) {
    it(`부서 카드를 「${name}」 로 바꾼 뒤 저장 → 다시 읽기 → 기동 회복: 이름이 그대로다(레지스트리 표시명 「마케팅부」 로 덮이지 않는다)`, () => {
      // 이름 변경의 확정이 남기는 상태: 이름 + 「주인이 정했다」 표식
      const afterRename = [{ id: 1, name, nameByUser: true, socket: SOCK, tree: null }];
      const reloaded = JSON.parse(persist(afterRename)).workspaces as Record<string, unknown>[];
      expect(reloaded[0].nameByUser).toBe(true); // 저장본에 표식이 남는다 — saveLayout 의 털기가 떼어 내면 붉다
      restoreNames(reloaded, "마케팅부"); // 앱 재시작
      expect(reloaded[0].name).toBe(name);
      restoreNames(reloaded, "마케팅부"); // 한 번 더(두 번째 재시작)
      expect(reloaded[0].name).toBe(name);
    });
  }
  it("표식이 없는 워크스페이스(이 판 이전의 저장본)는 종전 추정 그대로다 — 자리표시 모양의 이름은 레지스트리 표시명으로 회복된다", () => {
    const old = [
      { id: 1, name: "dept-9", socket: SOCK, tree: null },
      { id: 2, name: "non title", socket: SOCK, tree: null },
      { id: 3, name: "…", socket: SOCK, tree: null },
      { id: 4, name: "ws 3", tree: null },
      { id: 5, name: "직접 지은 이름", socket: SOCK, tree: null },
    ];
    const reloaded = JSON.parse(persist(old)).workspaces as Record<string, unknown>[];
    restoreNames(reloaded, "마케팅부");
    expect(reloaded.map((w) => w.name)).toEqual(["마케팅부", "마케팅부", "마케팅부", UNTITLED, "직접 지은 이름"]);
  });
  it("배선: 카드의 확정이 표식을 세우고(이름을 비우면 세우지 않는다), 기동 회복은 표식 단 워크스페이스를 건너뛴다", () => {
    const a = MAIN.indexOf("  const startRename = () => {");
    const src = MAIN.slice(a, MAIN.indexOf('  label.addEventListener("dblclick", startRename);', a));
    expect(src).toContain("if (name) ws.nameByUser = true;");
    expect(src).toContain("else delete ws.nameByUser;");
    const l = MAIN.indexOf('  // 구버전 자동 번호 이름("ws N")은 미정 표시로 이행');
    const loop = MAIN.slice(l, MAIN.indexOf("  // ★이 세 줄은 이제 **그물이지 보장이 아니다.**", l));
    expect(loop.indexOf("if (ws.nameByUser === true) continue;")).toBeGreaterThan(0);
    expect(loop.indexOf("if (ws.nameByUser === true) continue;")).toBeLessThan(loop.indexOf("ws.name = UNTITLED"));
    expect(MAIN).toContain("  nameByUser?: boolean;");
  });
});

// ────────────────────────────────────────────────────────────────────────────
describe("배선 — 카드와 그룹의 이름 바꾸기가 순수부를 쓴다", () => {
  const body = (from: string, to: string) => {
    const a = MAIN.indexOf(from);
    const b = MAIN.indexOf(to, a + from.length);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    return MAIN.slice(a, b);
  };
  it("카드(startRename): beginInlineRename 를 부르고, 옛 `blur → commit` 직결과 조합을 안 보는 Enter 가 없다", () => {
    const src = body("  const startRename = () => {", '  label.addEventListener("dblclick", startRename);');
    expect(src).toContain("beginInlineRename(el,"); // el = 화면에 붙어 있는 이름 칸(옛 칸이면 지금 칸으로 바꿔 잡는다)
    expect(src).not.toContain('addEventListener("blur", commit');
    expect(src).not.toContain("label.blur()");
    expect(src).toContain("ws.name = name || UNTITLED");
  });
  it("그룹(startGroupRename): 같은 순수부 · 같은 종전 규칙(빈 이름 → 「그룹」)", () => {
    const src = body("\nfunction startGroupRename(", "\n}\n");
    expect(src).toContain("beginInlineRename(label,");
    expect(src).not.toContain('addEventListener("blur", commit');
    expect(src).toContain('g.name = name || "그룹"');
  });
  it("renderWsTabs 는 목록을 지우기 **전에** 가드를 묻는다 · setFocus 는 터미널 포커스 전에 묻는다", () => {
    const tabs = body("\nfunction renderWsTabs(", "\n}\n");
    expect(tabs.indexOf("renameBlocksRebuild(")).toBeGreaterThan(0);
    expect(tabs.indexOf("renameBlocksRebuild(")).toBeLessThan(tabs.indexOf('bar.innerHTML = ""'));
    const focus = body("\nfunction setFocus(", "\n}\n");
    expect(focus.indexOf("inlineRenameOpen(")).toBeGreaterThan(0);
    expect(focus.indexOf("inlineRenameOpen(")).toBeLessThan(focus.indexOf("term.focus()"));
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★배선의 **동작** — main.ts 의 실제 startRename(카드) · 실제 startGroupRename(그룹) 을 소스 그대로 떼어 돌린다.
//   위 describe 는 「순수부를 부른다」는 글자만 본다. 여기는 그 호출이 끝났을 때 앱 상태가 어떻게 되는지를 잰다:
//   취소는 이름을 건드리지 않고 · 확정만 이름을 쓰고 · 어느 쪽이든 화면을 한 번 다시 그리며 · 고치는 중에 한 번 더
//   시작해도(더블클릭 + 메뉴) 편집이 둘로 갈라지지 않는다.
describe("배선의 동작 — 실제 카드·그룹 이름 바꾸기 코드를 실행한다", () => {
  const TS = new Bun.Transpiler({ loader: "ts" });
  /** 편집 가능 여부를 브라우저처럼 답하는 이름 칸. */
  class Label extends FakeEl {
    isConnected = true; // 화면에 붙어 있는 칸(목록이 다시 지어지면 옛 칸은 false 가 된다)
    get isContentEditable() {
      return this.contentEditable === "true";
    }
  }
  const fakeDocument = { hasFocus: () => true };
  type CardOpts = { live?: Label | null; hasFocus?: () => boolean; timer?: (fn: () => void) => void };
  const fakeWindow = { getSelection: () => ({ selectAllChildren() {} }) };
  const now = (fn: () => void) => fn(); // setTimeout 대역 — 다음 박자를 그 자리에서 돈다

  /** 실제 startRename(buildTab 안의 클로저)을 그 카드의 label·ws 에 세운다. */
  function card(ws: Record<string, unknown>, label: Label, opts: CardOpts = {}) {
    const a = MAIN.indexOf("  const startRename = () => {");
    const b = MAIN.indexOf("\n  };\n", a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const renders = { n: 0 };
    const refocus = { n: 0 };
    const doc = { hasFocus: opts.hasFocus ?? (() => true) };
    const start = new Function(
      "label", "ws", "render", "UNTITLED", "beginInlineRename", "document", "window", "setTimeout",
      "liveWsNameLabel", "restorePaneFocusAfterRename",
      `${TS.transformSync(MAIN.slice(a, b + 5))}\nreturn startRename;`,
    )(label, ws, () => void renders.n++, "non title", mod?.beginInlineRename, doc, fakeWindow, opts.timer ?? now,
      () => opts.live ?? null, () => void refocus.n++) as () => void;
    return { start, renders, refocus };
  }
  it("카드 · Escape: 이름도 「주인이 정했다」 표식도 그대로 · 화면은 한 번 다시 그린다(밀린 신호 반영)", () => {
    const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
    const label = new Label("마케팅부");
    const c = card(ws, label);
    c.start();
    label.textContent = "지우다 만 이름";
    label.fire("keydown", key("Escape"));
    expect(ws).toEqual({ id: 1, name: "마케팅부" });
    expect(c.renders.n).toBe(1);
  });
  it("카드 · Enter: 친 이름이 저장되고 표식이 선다 · 화면은 한 번", () => {
    const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
    const label = new Label("마케팅부");
    const c = card(ws, label);
    c.start();
    label.textContent = "  전략기획부 ";
    label.fire("keydown", key("Enter"));
    expect(ws).toEqual({ id: 1, name: "전략기획부", nameByUser: true });
    expect(c.renders.n).toBe(1);
  });
  it("카드 · 이름을 다 지우고 Enter: 미정 표시로 돌아가고 표식은 없다(다음 기동에 부서 표시명으로 회복)", () => {
    const ws: Record<string, unknown> = { id: 1, name: "전략기획부", nameByUser: true };
    const label = new Label("전략기획부");
    const c = card(ws, label);
    c.start();
    label.textContent = "   ";
    label.fire("keydown", key("Enter"));
    expect(ws).toEqual({ id: 1, name: "non title" });
  });
  it("카드 · 고치는 중에 한 번 더 시작(더블클릭 + 메뉴)해도 편집은 하나다 — 끝이 한 번만 온다", () => {
    const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
    const label = new Label("마케팅부");
    const c = card(ws, label);
    c.start();
    label.textContent = "전략";
    c.start(); // 두 번째 시작 — 무시된다(치던 글자를 「원래 이름」으로 다시 잡지 않는다)
    expect(label.count("keydown")).toBe(1);
    label.textContent = "전략기획부";
    label.fire("keydown", key("Escape"));
    expect(label.textContent).toBe("마케팅부"); // 취소는 **처음** 이름으로 돌아간다
    expect(c.renders.n).toBe(1);
  });

  /** 실제 startGroupRename 을, 그룹 하나와 그 머리의 이름 칸으로 세운다. */
  function group(g: Record<string, unknown>, label: Label) {
    const renders = { n: 0 };
    const head = { querySelector: (sel: string) => (sel === ".ws-group-name" ? label : null) };
    const doc = { ...fakeDocument, querySelectorAll: (sel: string) => (sel === "#ws-tabs .ws-group-head" ? [head] : []) };
    const refocus = { n: 0 };
    const start = new Function(
      "groups", "render", "beginInlineRename", "document", "window", "setTimeout", "restorePaneFocusAfterRename",
      `${realFunction("startGroupRename")}\nreturn startGroupRename;`,
    )([g], () => void renders.n++, mod?.beginInlineRename, doc, fakeWindow, now, () => void refocus.n++) as (x: unknown) => void;
    return { start: () => start(g), renders, refocus };
  }
  it("그룹 · Escape: 이름은 그대로 · 화면은 한 번 다시 그린다", () => {
    const g: Record<string, unknown> = { id: 3, name: "영업 그룹", pinned: false };
    const label = new Label("영업 그룹");
    const x = group(g, label);
    x.start();
    label.textContent = "치다 만";
    label.fire("keydown", key("Escape"));
    expect(g.name).toBe("영업 그룹");
    expect(x.renders.n).toBe(1);
    expect(x.refocus.n).toBe(1); // 끝난 뒤 키보드를 포커스 pane 에 돌려주는 뒷처리
  });
  it("그룹 · Enter: 친 이름이 저장된다 · 다 지우면 「그룹」", () => {
    const g: Record<string, unknown> = { id: 3, name: "영업 그룹", pinned: false };
    const label = new Label("영업 그룹");
    const x = group(g, label);
    x.start();
    label.textContent = "해외 영업";
    label.fire("keydown", key("Enter"));
    expect(g.name).toBe("해외 영업");
    expect(x.renders.n).toBe(1);
    expect(x.refocus.n).toBe(1);
    x.start();
    label.textContent = "";
    label.fire("keydown", key("Enter"));
    expect(g.name).toBe("그룹");
  });
});

describe("isImeKey — 조합 판정의 두 신호는 각각으로 충분하다", () => {
  it("isComposing 만 참(keyCode 13) · keyCode 229 만 · 둘 다 아님", () => {
    expect(mod.isImeKey({ isComposing: true, keyCode: 13 })).toBe(true);
    expect(mod.isImeKey({ isComposing: false, keyCode: 229 })).toBe(true);
    expect(mod.isImeKey({ keyCode: 229 })).toBe(true);
    expect(mod.isImeKey({ isComposing: false, keyCode: 13 })).toBe(false);
    expect(mod.isImeKey({})).toBe(false);
  });
  it("조합 중 Enter(isComposing · keyCode 13)도 그 박자에 끝내지 않는다 — 다음 박자에 확정", () => {
    const el = new FakeEl("가");
    const done: [boolean, string][] = [];
    const queued: (() => void)[] = [];
    mod.beginInlineRename(el, { windowFocused: () => true, later: (fn: () => void) => void queued.push(fn), onDone: (c: boolean, n: string) => void done.push([c, n]) });
    el.textContent = "가나";
    el.fire("keydown", key("Enter", { isComposing: true, keyCode: 13 }));
    expect(done).toEqual([]);
    el.textContent = "가나다"; // IME 가 글자를 다 쓴 뒤
    for (const fn of queued) fn();
    expect(done).toEqual([[true, "가나다"]]);
  });
});

// ────────────────────────────────────────────────────────────────────────────
// ★독립 검토(reviews/ATTACK-fix1-fix3.md)가 찾은 것 — 전부 main.ts 의 실제 코드를 실행해 잰다.
describe("★메뉴가 쥔 옛 칸 — 목록이 다시 지어진 뒤의 「이름 변경」도 지금 화면의 칸에서 열린다", () => {
  class Cell extends FakeEl {
    isConnected = true;
    get isContentEditable() {
      return this.contentEditable === "true";
    }
  }
  const TS2 = new Bun.Transpiler({ loader: "ts" });
  function staleCard(ws: Record<string, unknown>, stale: Cell, live: Cell | null) {
    const a = MAIN.indexOf("  const startRename = () => {");
    const b = MAIN.indexOf("\n  };\n", a);
    const renders = { n: 0 };
    const asked: number[] = [];
    const start = new Function(
      "label", "ws", "render", "UNTITLED", "beginInlineRename", "document", "window", "setTimeout",
      "liveWsNameLabel", "restorePaneFocusAfterRename",
      `${TS2.transformSync(MAIN.slice(a, b + 5))}\nreturn startRename;`,
    )(stale, ws, () => void renders.n++, "non title", mod?.beginInlineRename, { hasFocus: () => true },
      { getSelection: () => ({ selectAllChildren() {} }) }, (fn: () => void) => fn(),
      (id: number) => (asked.push(id), live), () => {}) as () => void;
    return { start, renders, asked };
  }
  it("우클릭 → (목록 재구성) → 「이름 변경」: 옛 칸은 그대로 두고 **새 칸**이 편집 상태가 된다 · 친 이름이 저장된다", () => {
    const ws: Record<string, unknown> = { id: 7, name: "마케팅부" };
    const stale = new Cell("마케팅부");
    stale.isConnected = false; // 메뉴가 떠 있는 사이 renderWsTabs 가 카드를 새로 지었다
    const live = new Cell("마케팅부");
    const c = staleCard(ws, stale, live);
    c.start();
    expect(c.asked).toEqual([7]); // 이 워크스페이스의 지금 칸을 id 로 찾았다
    expect([live.contentEditable, live.attrs.get("data-inline-rename"), live.focused]).toEqual(["true", "1", 1]);
    expect([stale.contentEditable, stale.attrs.size, stale.count("keydown")]).toEqual(["inherit", 0, 0]); // 옛 칸에 편집 상태를 남기지 않는다
    live.textContent = "전략기획부";
    live.fire("keydown", key("Enter"));
    expect(ws).toEqual({ id: 7, name: "전략기획부", nameByUser: true });
  });
  it("카드가 화면에서 사라졌으면(워크스페이스가 그 사이 삭제) 아무 일도 없다 — 던지지 않는다", () => {
    const ws: Record<string, unknown> = { id: 7, name: "마케팅부" };
    const stale = new Cell("마케팅부");
    stale.isConnected = false;
    const c = staleCard(ws, stale, null);
    expect(() => c.start()).not.toThrow();
    expect([stale.contentEditable, stale.attrs.size, c.renders.n]).toEqual(["inherit", 0, 0]);
  });
  it("화면에 붙어 있는 칸에서는 찾으러 가지 않는다(더블클릭 · 방금 연 메뉴)", () => {
    const ws: Record<string, unknown> = { id: 7, name: "마케팅부" };
    const label = new Cell("마케팅부");
    const c = staleCard(ws, label, null);
    c.start();
    expect(c.asked).toEqual([]);
    expect(label.contentEditable).toBe("true");
  });
  it("liveWsNameLabel 은 그 id 의 카드 안 이름 칸을 #ws-tabs 에서 찾는다", () => {
    const seen: string[] = [];
    const fn = new Function("document", `${realFunction("liveWsNameLabel")}\nreturn liveWsNameLabel;`)({
      querySelector: (sel: string) => (seen.push(sel), "CELL"),
    }) as (id: number) => unknown;
    expect(fn(12)).toBe("CELL");
    expect(seen).toEqual(['#ws-tabs .ws-tab[data-ws-id="12"] .ws-name']);
  });
});

describe("★아무것도 바꾸지 않고 끝낸 이름 바꾸기는 아무것도 바꾸지 않는다", () => {
  class Cell extends FakeEl {
    isConnected = true;
    get isContentEditable() {
      return this.contentEditable === "true";
    }
  }
  const TS3 = new Bun.Transpiler({ loader: "ts" });
  function open(ws: Record<string, unknown>, label: Cell) {
    const a = MAIN.indexOf("  const startRename = () => {");
    const b = MAIN.indexOf("\n  };\n", a);
    const n = { renders: 0, refocus: 0 };
    (new Function(
      "label", "ws", "render", "UNTITLED", "beginInlineRename", "document", "window", "setTimeout",
      "liveWsNameLabel", "restorePaneFocusAfterRename",
      `${TS3.transformSync(MAIN.slice(a, b + 5))}\nreturn startRename;`,
    )(label, ws, () => void n.renders++, "non title", mod?.beginInlineRename, { hasFocus: () => true },
      { getSelection: () => ({ selectAllChildren() {} }) }, (fn: () => void) => fn(), () => null, () => void n.refocus++) as () => void)();
    return n;
  }
  for (const shown of ["non title", "dept-9", "…"]) {
    it(`자리표시 이름 「${shown}」 의 카드를 열었다가 Enter(또는 다른 곳 클릭): 이름도 「주인이 정했다」 표식도 그대로 — 다음 기동에 부서 표시명이 돌아온다`, () => {
      const ws: Record<string, unknown> = { id: 1, name: shown };
      const label = new Cell(shown);
      const n = open(ws, label);
      label.fire("keydown", key("Enter"));
      expect(ws).toEqual({ id: 1, name: shown });
      const ws2: Record<string, unknown> = { id: 1, name: shown };
      const label2 = new Cell(shown);
      open(ws2, label2);
      label2.fire("blur");
      expect(ws2).toEqual({ id: 1, name: shown });
      expect(n.renders).toBe(1); // 그래도 화면은 한 번 다시 그린다(밀린 신호)
    });
  }
  it("이미 주인이 정한 이름의 카드를 열었다 닫아도 표식은 남는다 · 같은 글자를 다시 쳐도 같다", () => {
    const ws: Record<string, unknown> = { id: 1, name: "dept-9", nameByUser: true };
    const label = new Cell("dept-9");
    open(ws, label);
    label.textContent = " dept-9 ";
    label.fire("keydown", key("Enter"));
    expect(ws).toEqual({ id: 1, name: "dept-9", nameByUser: true });
  });
  it("끝날 때마다 키보드를 포커스 pane 에 돌려주는 뒷처리를 한 번 부른다(확정 · 취소 · 무변경)", () => {
    for (const end of ["Enter", "Escape"]) {
      const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
      const label = new Cell("마케팅부");
      const n = open(ws, label);
      label.textContent = end === "Enter" ? "전략기획부" : "지우다 만";
      label.fire("keydown", key(end));
      expect({ end, refocus: n.refocus, renders: n.renders }).toEqual({ end, refocus: 1, renders: 1 });
    }
  });
});

describe("★이름 바꾸기가 끝난 뒤의 키보드 — 실제 restorePaneFocusAfterRename", () => {
  function run(activeElement: unknown, focusedSid: number | null, renameOpen: boolean) {
    const focused: number[] = [];
    const queue: (() => void)[] = [];
    const doc = { activeElement, body: "BODY" };
    (new Function(
      "document", "focusedSid", "setFocus", "setTimeout", "inlineRenameOpen",
      `${realFunction("restorePaneFocusAfterRename")}\nreturn restorePaneFocusAfterRename;`,
    )(doc, focusedSid, (sid: number) => void focused.push(sid), (fn: () => void) => void queue.push(fn), () => renameOpen) as () => void)();
    const before = [...focused];
    for (const fn of queue) fn();
    return { before, after: focused, queued: queue.length };
  }
  it("아무도 포커스를 갖지 않았으면(body) 다음 박자에 포커스 pane 에 준다 — 그 박자 전에는 주지 않는다", () => {
    expect(run("BODY", 22, false)).toEqual({ before: [], after: [22], queued: 1 });
    expect(run(null, 22, false)).toEqual({ before: [], after: [22], queued: 1 });
  });
  it("주인이 방금 누른 다른 입력 칸이 포커스를 가졌으면 빼앗지 않는다 · 포커스 pane 이 없으면 아무 일도 없다 · 또 다른 이름 칸이 열려 있으면 주지 않는다", () => {
    expect(run("SEARCH-INPUT", 22, false).after).toEqual([]);
    expect(run("BODY", null, false).after).toEqual([]);
    expect(run("BODY", 22, true).after).toEqual([]);
  });
});

describe("★배선의 두 인자 — 창 포커스와 다음 박자는 진짜 값이어야 한다(실제 startRename)", () => {
  class Cell extends FakeEl {
    isConnected = true;
    get isContentEditable() {
      return this.contentEditable === "true";
    }
  }
  const TS4 = new Bun.Transpiler({ loader: "ts" });
  function open(ws: Record<string, unknown>, label: Cell, hasFocus: () => boolean, timer: (fn: () => void) => void) {
    const a = MAIN.indexOf("  const startRename = () => {");
    const b = MAIN.indexOf("\n  };\n", a);
    (new Function(
      "label", "ws", "render", "UNTITLED", "beginInlineRename", "document", "window", "setTimeout",
      "liveWsNameLabel", "restorePaneFocusAfterRename",
      `${TS4.transformSync(MAIN.slice(a, b + 5))}\nreturn startRename;`,
    )(label, ws, () => {}, "non title", mod?.beginInlineRename, { hasFocus }, { getSelection: () => ({ selectAllChildren() {} }) },
      timer, () => null, () => {}) as () => void)();
  }
  it("앱 창이 뒤로 가서 생긴 blur(document.hasFocus() = false)는 카드의 이름 바꾸기를 끝내지 않는다", () => {
    const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
    const label = new Cell("마케팅부");
    open(ws, label, () => false, (fn) => fn());
    label.textContent = "전략";
    label.fire("blur");
    expect([label.contentEditable, ws.name]).toEqual(["true", "마케팅부"]); // 아직 고치는 중
  });
  it("한글 조합을 확정하는 Enter 는 카드에서도 다음 박자(setTimeout)에 끝난다 — 그 박자 전에는 저장하지 않는다", () => {
    const ws: Record<string, unknown> = { id: 1, name: "마케팅부" };
    const label = new Cell("마케팅부");
    const queue: (() => void)[] = [];
    open(ws, label, () => true, (fn) => void queue.push(fn));
    label.textContent = "전략기획";
    label.fire("keydown", key("Enter", { isComposing: true, keyCode: 229 }));
    expect(ws.name).toBe("마케팅부");
    label.textContent = "전략기획부"; // IME 가 마지막 글자를 다 쓴 뒤
    for (const fn of queue) fn();
    expect(ws).toEqual({ id: 1, name: "전략기획부", nameByUser: true });
  });
});

describe("★바꾼 이름은 앱의 다른 화면에서도 그 이름이다 — 실제 ownerNamedDept", () => {
  const SOCK = "/tmp/s1-dept-9.sock";
  const fn = (workspaces: Record<string, unknown>[]) =>
    new Function("workspaces", "sameSocket", `${realFunction("ownerNamedDept")}\nreturn ownerNamedDept;`)(
      workspaces, (a: unknown, b: unknown) => a === b,
    ) as (socket: string | null | undefined, registry: string) => string;
  it("주인이 카드에서 정한 이름이 있으면 그 이름 · 없으면 레지스트리 표시명", () => {
    expect(fn([{ id: 1, name: "전략기획부", nameByUser: true, socket: SOCK }])(SOCK, "마케팅부")).toBe("전략기획부");
    expect(fn([{ id: 1, name: "마케팅부", socket: SOCK }])(SOCK, "마케팅부")).toBe("마케팅부"); // 표식 없음 = 주인이 정하지 않았다
    expect(fn([{ id: 1, name: "옛 이름", socket: SOCK }])(SOCK, "마케팅부")).toBe("마케팅부"); // 표식 없는 카드의 이름은 쓰지 않는다
  });
  it("다른 부서의 카드 · 본부 탭(소켓 없음) · 소켓을 모르는 행은 레지스트리 표시명 그대로", () => {
    const ws = [{ id: 1, name: "전략기획부", nameByUser: true, socket: SOCK }, { id: 2, name: "내 본부", nameByUser: true }];
    expect(fn(ws)("/tmp/other.sock", "영업부")).toBe("영업부");
    expect(fn(ws)(undefined, "영업부")).toBe("영업부");
    expect(fn(ws)(null, "영업부")).toBe("영업부");
  });
  it("관제 센터 「작업」 머리와 승인 Feed 의 부서 구획이 그 함수를 거친다", () => {
    expect(MAIN.includes('ccEsc(ownerNamedDept(d.socket, d.display_name ?? d.name ?? ""))')).toBe(true);
    expect(MAIN.includes('label: x.dept === null ? "본부" : ownerNamedDept(x.socket, x.display_name || x.dept),')).toBe(true);
  });
});

describe("★이름을 고치는 중의 전역 단축키 — 실제 window keydown 처리기", () => {
  /** 처리기 본문을 그대로 떼어, 모르는 이름은 전부 '불린 것을 세는 함수'로 답하는 범위 안에서 돈다. */
  function press(k: string, renameOpen: boolean) {
    const a = MAIN.indexOf('window.addEventListener("keydown", (e) => {\n  if (e.isComposing || e.keyCode === 229) return; // IME 조합 중 무시');
    const b = MAIN.indexOf("\n});\n", a);
    expect(a).toBeGreaterThan(0);
    expect(b).toBeGreaterThan(a);
    const body = new Bun.Transpiler({ loader: "ts" }).transformSync(MAIN.slice(a, b + 4));
    const called: string[] = [];
    let handler: ((e: unknown) => void) | null = null;
    const known: Record<string, unknown> = {
      window: { addEventListener: (_t: string, fn: (e: unknown) => void) => void (handler = fn) },
      document: { querySelector: () => null },
      inlineRenameOpen: () => renameOpen,
      paletteOpen: false,
      ccOpen: false,
    };
    const scope = new Proxy(known, {
      has: () => true,
      get: (t, name) => {
        if (typeof name !== "string") return undefined;
        if (name in t) return t[name];
        return (..._a: unknown[]) => void called.push(name);
      },
    });
    new Function("__scope", `with (__scope) {\n${body}\n}`)(scope);
    expect(handler).not.toBeNull();
    let prevented = 0;
    (handler as unknown as (e: unknown) => void)({ key: k, metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, isComposing: false, keyCode: 0, preventDefault: () => void prevented++ });
    return { called, prevented };
  }
  it("대조군: 이름을 고치고 있지 않을 때 ⌘W 는 포커스 pane 을 닫는다(actionClose) — 이 검사가 붉어질 수 있음을 못박는다", () => {
    expect(press("w", false)).toEqual({ called: ["actionClose"], prevented: 1 });
  });
  for (const k of ["w", "t", "d", "k"]) {
    it(`이름 칸에 커서를 둔 채 누른 ⌘${k.toUpperCase()} 는 아무것도 하지 않는다(뒤의 세션을 건드리지 않는다)`, () => {
      expect(press(k, true)).toEqual({ called: [], prevented: 0 });
    });
  }
});

describe("★그룹 이름을 고치는 중의 클릭은 커서 이동이다", () => {
  it("그룹 이름의 click 처리기는 편집 중이면 워크스페이스를 바꾸기 전에 돌아간다", () => {
    const a = MAIN.indexOf('  name.addEventListener("click", () => {');
    const b = MAIN.indexOf("\n  });\n", a);
    expect(a).toBeGreaterThan(0);
    const body = MAIN.slice(a, b);
    const guard = body.indexOf("if (name.isContentEditable) return;");
    expect(guard).toBeGreaterThan(0);
    expect(guard).toBeLessThan(body.indexOf("activeWs = "));
    // 실행으로도 잰다: 편집 중이면 activeWs·render 가 그대로다
    const run = (editable: boolean) => {
      const st = { activeWs: 0, renders: 0, focus: 0 };
      const name = { isContentEditable: editable, addEventListener: (_t: string, fn: () => void) => void (click = fn) };
      let click: (() => void) | null = null;
      const anchor = { tree: null };
      const sync = new Function("name", "anchorWsOf", "workspaces", "g", "render", "collectSids", "setFocus", "st",
        `let activeWs = 0;\n${new Bun.Transpiler({ loader: "ts" }).transformSync(body + "\n  });")}\nreturn () => (st.activeWs = activeWs);`,
      )(name, () => anchor, [{ id: 0 }, anchor], { id: 3 }, () => void st.renders++, () => [], () => void st.focus++, st) as () => number;
      (click as unknown as () => void)();
      sync();
      return st;
    };
    expect(run(false)).toEqual({ activeWs: 1, renders: 1, focus: 0 }); // 대조군: 평소의 클릭은 그 그룹의 워크스페이스로 간다
    expect(run(true)).toEqual({ activeWs: 0, renders: 0, focus: 0 });
  });
});
