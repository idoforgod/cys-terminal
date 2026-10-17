// ★F-01 — 역할 점은 그 좌석 CLI 가 **작업 중으로 관측될 때만** 깜박인다(DESIGN-U-FROZEN-R3 §4 · AC-UF-1/4/5 · U4-5).
//   ① 순수: 데몬 display_activity 파싱 · 수명/세대/순번 거부 · 보수적 나이 · 투영 · 바뀐 것만 쓰는 칠하기
//   ② CSS: 무조건 깜박임 제거 — 작업 중만 깜박인다 · 움직임 줄임에서도 작업 중은 구별된다
//   ③ U-F06: main.ts 의 **실제** refreshPaneTitles·활동 패스 본문을 떼어 돌린다(가짜 DOM · 가짜 IPC · 가짜 단조 시계)
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import {
  parseDisplayActivity,
  RoleActivityStore,
  projectActivity,
  applyDotPaint,
  HIDDEN_DOT,
  STALE_WORKING_MS,
  activityExpiryMs,
} from "./roleactivity";
import { SurfaceListTransport } from "./surfacelist";
import { roleDotColor } from "./appearance";
import { seatPriority, annotateRoles, tidyHoles } from "./seatlayout";
import { advanceGhostStrikes } from "./wsreconcile";
import { deptLiveRoles } from "./deptprogress";

declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };

const WIRE = (over: Record<string, unknown> = {}): Record<string, unknown> => ({
  state: "working", raw: "active", reason: "active footer", family: "claude", grammar_revision: "claude-1",
  observed_age_secs: 0.4, last_positive_age_secs: 0.4, stable_age_secs: 5, observation_seq: 7, display_epoch: "ep-1",
  retained: false, consecutive_rejections: 0, ...over,
});

describe("★F-01 parseDisplayActivity — 데몬 꼴만 믿는다(빠짐·형식 오류 = Unknown)", () => {
  it("parses the full daemon wire object", () => {
    expect(parseDisplayActivity(WIRE())).toEqual({
      kind: "observation", state: "working", raw: "active", reason: "active footer", family: "claude", grammarRevision: "claude-1",
      observedAgeSecs: 0.4, lastPositiveAgeSecs: 0.4, stableAgeSecs: 5, seq: 7, epoch: "ep-1", retained: false, consecutiveRejections: 0,
    });
  });
  it("missing key (old daemon), the degenerate unknown form and malformed values all project Unknown", () => {
    expect(parseDisplayActivity(undefined)).toEqual({ kind: "unknown", reason: "활동 정보 없음(구버전 데몬)", malformed: false });
    expect(parseDisplayActivity({ state: "unknown", raw: "unknown", reason: "no observation", retained: false })).toEqual({
      kind: "unknown", reason: "no observation", malformed: false,
    });
    const bad: unknown[] = [
      null, 3, "working", [WIRE()],
      WIRE({ state: "busy" }), WIRE({ raw: "typing" }), WIRE({ reason: 5 }), WIRE({ retained: "no" }),
      WIRE({ observation_seq: -1 }), WIRE({ observation_seq: 1.5 }), WIRE({ display_epoch: "" }), WIRE({ display_epoch: undefined }),
      WIRE({ observed_age_secs: -1 }), WIRE({ observed_age_secs: null }), WIRE({ last_positive_age_secs: "1" }), WIRE({ family: "grok" }),
      WIRE({ consecutive_rejections: -2 }), WIRE({ grammar_revision: 4 }),
      { state: "working", raw: "active", reason: "x", retained: false }, // 신원 없는 Working
    ];
    for (const v of bad) {
      const p = parseDisplayActivity(v);
      expect({ v, kind: p.kind, malformed: p.kind === "unknown" && p.malformed }).toEqual({ v, kind: "unknown", malformed: true });
    }
    expect(projectActivity(undefined, 0, 31_000).view).toBe("unknown");
  });
});

describe("★F-01 RoleActivityStore — 수명·세대·순번·만료(UI 는 K 를 다시 적용하지 않고 리스를 연장하지 않는다)", () => {
  it("rejects wrong lifetime, epoch regression, non-increasing seq within an epoch; a new epoch or lifetime starts fresh", () => {
    const s = new RoleActivityStore(31_000);
    expect(s.accept("k", 2, 0, 10, WIRE({ observation_seq: 5 }))).toBe("accepted");
    expect(s.accept("k", 1, 100, 110, WIRE({ observation_seq: 9 }))).toBe("wrong-lifetime");
    expect(s.accept("k", 2, 100, 110, WIRE({ observation_seq: 5 }))).toBe("duplicate-sequence");
    expect(s.accept("k", 2, 100, 110, WIRE({ observation_seq: 4 }))).toBe("sequence-regression");
    expect(s.accept("k", 2, 200, 210, WIRE({ observation_seq: 1, display_epoch: "ep-2" }))).toBe("accepted");
    expect(s.accept("k", 2, 300, 310, WIRE({ observation_seq: 99 }))).toBe("epoch-regression"); // ep-1 은 물러났다
    expect(s.accept("k", 2, 150, 310, WIRE({ observation_seq: 2, display_epoch: "ep-2" }))).toBe("out-of-order"); // 앞선 요청의 늦은 응답
    expect(s.get("k")?.startMs).toBe(200);
    expect(s.accept("k", 3, 400, 410, WIRE({ observation_seq: 1 }))).toBe("accepted"); // 새 끝점 수명 — 처음부터
    expect(s.get("k")?.lifetime).toBe(3);
  });
  it("fresh producer Unknown overrides Working immediately — degenerate form and lease expiry at serialization (same seq, state unknown)", () => {
    const s = new RoleActivityStore(31_000);
    s.accept("k", 1, 0, 10, WIRE());
    expect(projectActivity(s.get("k"), 20, 31_000).view).toBe("working");
    expect(s.accept("k", 1, 3_000, 3_010, WIRE({ state: "unknown", reason: "lease expired" }))).toBe("accepted");
    expect(projectActivity(s.get("k"), 3_020, 31_000)).toEqual({ view: "unknown", text: "상태 모름 · lease expired · 관측 1초 전" });
    s.accept("k", 1, 6_000, 6_010, WIRE({ observation_seq: 8 }));
    expect(s.accept("k", 1, 9_000, 9_010, { state: "unknown", raw: "unknown", reason: "identity changed", retained: false })).toBe("accepted");
    expect(projectActivity(s.get("k"), 9_020, 31_000)).toEqual({ view: "unknown", text: "상태 모름 · identity changed" });
  });
  it("prune drops records the predicate does not keep (vanished surfaces / removed members)", () => {
    const s = new RoleActivityStore(31_000);
    s.accept("/a#1", 1, 0, 1, WIRE());
    s.accept("/a#2", 1, 0, 1, WIRE());
    s.accept("/b#1", 1, 0, 1, WIRE());
    s.prune((k) => k !== "/a#2");
    expect([s.get("/a#1") !== undefined, s.get("/a#2") !== undefined, s.get("/b#1") !== undefined]).toEqual([true, false, true]);
  });
});

describe("★F-01 projectActivity — 표시 상태와 접근 문구(상태 + 사유 + 나이)", () => {
  const at = (wire: Record<string, unknown>, startMs: number, nowMs: number, T = 31_000) => {
    const s = new RoleActivityStore(T);
    expect(s.accept("k", 1, startMs, startMs + 5, wire)).toBe("accepted");
    return projectActivity(s.get("k"), nowMs, T);
  };
  it("working / waiting / waiting_for_input / producer unknown — conservative age = daemon age + elapsed since request start", () => {
    expect(at(WIRE(), 1_000, 2_500)).toEqual({ view: "working", text: "작업 중 · active footer · 관측 2초 전" });
    expect(at(WIRE({ state: "waiting", raw: "idle", reason: "settled idle", observed_age_secs: 2 }), 0, 1_000)).toEqual({
      view: "waiting", text: "대기 · settled idle · 관측 3초 전",
    });
    expect(at(WIRE({ state: "waiting_for_input", raw: "permission", reason: "permission prompt" }), 0, 0)).toEqual({
      view: "input", text: "입력 대기 · permission prompt(permission) · 관측 1초 전",
    });
    expect(at(WIRE({ retained: true, consecutive_rejections: 2, reason: "transient: incomplete frame", last_positive_age_secs: 3.2 }), 0, 0)).toEqual({
      view: "working", text: "작업 중 · transient: incomplete frame · 관측 1초 전 · 일시 유지(연속 거부 2회 · 마지막 활동 4초 전)",
    });
  });
  it("U4-5 threshold is exactly producer lease + one list period (9 s) and T is 31/47 s", () => {
    expect(STALE_WORKING_MS).toBe(9_000);
    expect([activityExpiryMs(8_000), activityExpiryMs(16_000)]).toEqual([31_000, 47_000]);
    // conservative response age = observed_age_secs (0.4 s) + elapsed since request start
    expect(at(WIRE(), 0, 8_600).view).toBe("working");
    expect(at(WIRE(), 0, 8_601)).toEqual({ view: "stale", text: "상태 확인 지연 — 작업 중 표시를 멈춤 · 마지막 관측 10초 전" });
    // a Working observation that was already 8 s old when serialized goes stale 1 s after the request started
    expect(at(WIRE({ observed_age_secs: 8 }), 0, 1_001).view).toBe("stale");
    expect(at(WIRE({ state: "waiting", raw: "idle" }), 0, 20_000).view).toBe("waiting"); // U4-5 는 Working 만 낮춘다
    expect(at(WIRE(), 0, 31_000)).toEqual({ view: "unknown", text: "상태 모름 · 응답 만료 — 마지막 응답 31초 전" });
  });
});

/** 칠하기 대상의 최소 대역 — 쓰기를 센다. */
class FakeDot {
  writes: string[] = [];
  classes = new Set<string>();
  attrs: Record<string, string> = {};
  props: Record<string, string> = {};
  private _title = "";
  private _display = "";
  throwOnPaint = false;
  readonly style: { display: string; background: string; setProperty(k: string, v: string): void };
  readonly classList: { add(c: string): void; remove(c: string): void; contains(c: string): boolean };
  constructor() {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const self = this;
    this.style = {
      get display(): string {
        return self._display;
      },
      set display(v: string) {
        self.writes.push(`display=${v}`);
        self._display = v;
      },
      background: "",
      setProperty(k: string, v: string): void {
        if (self.throwOnPaint) throw new Error("paint boom");
        self.writes.push(`${k}=${v}`);
        self.props[k] = v;
      },
    };
    this.classList = {
      add(c: string): void {
        if (self.throwOnPaint) throw new Error("paint boom");
        self.writes.push(`+${c}`);
        self.classes.add(c);
      },
      remove(c: string): void {
        self.writes.push(`-${c}`);
        self.classes.delete(c);
      },
      contains: (c: string): boolean => self.classes.has(c),
    };
  }
  get title(): string {
    return this._title;
  }
  set title(v: string) {
    this.writes.push("title");
    this._title = v;
  }
  setAttribute(k: string, v: string): void {
    if (this.throwOnPaint) throw new Error("paint boom");
    this.writes.push(`@${k}`);
    this.attrs[k] = v;
  }
  getAttribute(k: string): string | null {
    return this.attrs[k] ?? null;
  }
  get state(): string {
    return [...this.classes].filter((c) => c.startsWith("act-")).join(" ");
  }
}

describe("★F-01 applyDotPaint — 바뀐 클래스·속성만 쓴다(역할색 그대로 · 접근 문구 = title + aria-label)", () => {
  it("first paint writes everything once; an identical paint writes nothing; a state change writes only the class pair and the label", () => {
    const el = new FakeDot();
    const p = { visible: true, color: "#22c55e", view: "working" as const, label: "역할: worker · 작업 중" };
    expect(applyDotPaint(el, p)).toBeGreaterThan(0);
    expect(el.props["--role-color"]).toBe("#22c55e");
    expect([el.state, el.title, el.attrs["aria-label"], el.attrs.role]).toEqual(["act-working", p.label, p.label, "img"]);
    el.writes = [];
    expect(applyDotPaint(el, { ...p })).toBe(0);
    expect(el.writes).toEqual([]);
    applyDotPaint(el, { ...p, view: "waiting", label: "역할: worker · 대기" });
    expect(el.writes).toEqual(["-act-working", "+act-waiting", "title", "@aria-label"]);
    el.writes = [];
    applyDotPaint(el, HIDDEN_DOT);
    expect(el.writes).toEqual(["display=none"]); // 무역할·종료 좌석은 숨김(종전 그대로)
    expect(el.style.background).toBe(""); // 인라인 배경을 쓰지 않는다 — 빈 원(hollow)이 CSS 로 이긴다
  });
});

describe("★F-01 style.css — 무조건 깜박임 제거", () => {
  const CSS = readFileSync(new URL("./style.css", import.meta.url), "utf-8");
  const rule = (sel: string): string => {
    const i = CSS.indexOf(`${sel} {`);
    expect({ sel, found: i >= 0 }).toEqual({ sel, found: true });
    return CSS.slice(i, CSS.indexOf("}", i));
  };
  it("the base .pane-role-dot no longer blinks; only the working state does; role colour stays the dot colour", () => {
    expect(rule(".pane .pane-role-dot")).not.toContain("animation");
    expect(rule(".pane .pane-role-dot")).toContain("var(--role-color");
    expect(rule(".pane .pane-role-dot.act-working")).toContain("animation: cc-blink");
    for (const c of ["act-waiting", "act-input", "act-stale", "act-unknown"]) {
      const sel = CSS.slice(0, CSS.indexOf("@media (prefers-reduced-motion")).match(new RegExp(`\\.pane \\.pane-role-dot\\.${c}[^{]*\\{([^}]*)\\}`));
      expect({ c, found: sel !== null, blinks: sel?.[1].includes("animation") ?? null }).toEqual({ c, found: true, blinks: false });
    }
    // input wait: a distinct steady attention SHAPE in the same role colour (design R3-5) — not a warn-coloured ring.
    const input = rule(".pane .pane-role-dot.act-input");
    expect(input).not.toContain("var(--warn");
    expect(input).not.toContain("background"); // keeps the base rule's role-colour fill
    expect(input).toContain("rotate(45deg)");
    expect(input).toContain("border-radius: 1px");
  });
  it("reduced motion: Working stops animating but keeps a distinct static work marker", () => {
    const blocks = CSS.split("@media (prefers-reduced-motion: reduce)").slice(1);
    const mine = blocks.find((b) => b.includes(".pane .pane-role-dot.act-working"));
    expect(mine !== undefined).toBe(true);
    const body = (mine as string).slice((mine as string).indexOf(".pane .pane-role-dot.act-working"));
    const decl = body.slice(0, body.indexOf("}"));
    expect(decl).toContain("animation: none");
    expect(decl).toContain("box-shadow");
  });
});

// ───────────────────────── U-F06 — main.ts 실제 본문 실행 ─────────────────────────
const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
function fnText(name: string): string {
  const m = new RegExp(`^(?:async )?function ${name}\\(`, "m").exec(SRC);
  expect({ 함수: name, 존재: m !== null }).toEqual({ 함수: name, 존재: true });
  const a = (m as RegExpExecArray).index;
  return SRC.slice(a, SRC.indexOf("\n}\n", a) + 2);
}
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyFn = (...a: any[]) => any;
/** 함수 본문을 그대로 떼어 `with (scope)` 안에서 연다 — 함수가 다시 대입하는 모듈 변수(refreshing 등)는 scope 의 속성으로 읽고 쓴다. */
function loadWith(names: string[], deps: Record<string, unknown>): { fns: Record<string, AnyFn>; scope: Record<string, unknown> } {
  const tr = new Bun.Transpiler({ loader: "ts" });
  const code = names.map((n) => tr.transformSync(fnText(n))).join("\n");
  const scope: Record<string, unknown> = Object.assign(Object.create(null), deps);
  const fns = new Function("__scope", `with (__scope) {\n${code}\nreturn { ${names.join(", ")} };\n}`)(scope) as Record<string, AnyFn>;
  return { fns, scope };
}
const flush = (): Promise<void> => new Promise<void>((r) => setTimeout(r, 0));

interface FakeRt {
  sid: number;
  socket?: string;
  roleEl: FakeDot;
  titleEl: { isContentEditable: boolean; textContent: string };
  usageEl: Record<string, never>;
  termHost: { scrollTop: number };
  el: Record<string, never>;
}
const FNS = ["collectSids", "setRoleDot", "paintRoleDot", "paintUnknownSafely", "sweepRoleDots", "projectListArrival", "pumpRoleActivity", "onVisibilityReturn", "refreshPaneTitles"];

function harness() {
  const mono = { t: 50_000 };
  const invokes: string[] = [];
  const replies: Record<string, () => Promise<unknown>> = {};
  const renders: string[] = [];
  const focus: number[] = [];
  const paneKey = (sid: number, socket?: string): string => `${socket ?? ""}#${sid}`;
  const makeRt = (sid: number, socket?: string): FakeRt => ({
    sid, socket, roleEl: new FakeDot(), titleEl: { isContentEditable: false, textContent: "" }, usageEl: {}, termHost: { scrollTop: 120 + sid }, el: {},
  });
  const panes = new Map<string, FakeRt>();
  for (const [sid, sk] of [[1, "/a.sock"], [2, "/a.sock"], [3, "/b.sock"]] as const) panes.set(paneKey(sid, sk), makeRt(sid, sk));
  const workspaces: Record<string, unknown>[] = [
    { id: 1, name: "A", socket: "/a.sock", tree: { type: "split", dir: "row", ratio: 0.5, a: { type: "pane", sid: 1 }, b: { type: "pane", sid: 2 } } },
    { id: 2, name: "B", socket: "/b.sock", tree: { type: "pane", sid: 3 } },
  ];
  // eslint-disable-next-line prefer-const
  let fns: Record<string, AnyFn> = {};
  const listTransport = new SurfaceListTransport({
    now: () => mono.t,
    deadlineMs: 8_000,
    resolve: (eps) => Promise.resolve(eps.map((e) => e ?? "/default.sock")),
    onArrival: (a) => fns.projectListArrival(a),
  });
  const nothing = (): void => undefined;
  const deps: Record<string, unknown> = {
    started: true, refreshing: false, workspaces, wsCounter: 9, activeWs: 0, UNTITLED: "…", focusedSid: 1,
    current: () => workspaces[0],
    invoke: (cmd: string, args: { socket?: string }): Promise<unknown> => {
      invokes.push(`${cmd}:${String(args.socket)}`);
      const r = replies[String(args.socket)];
      return r ? r() : new Promise<unknown>(() => undefined);
    },
    rpcT: (p: Promise<unknown>): Promise<unknown> =>
      Promise.race([p, new Promise<unknown>((_, rej) => setTimeout(() => rej(new Error("rpc timeout")), 15))]),
    T_LIST: 8_000,
    collectCwdBlocked: () => [], annotateRoles, advanceGhostStrikes, seatPriority, tidyHoles, deptLiveRoles,
    ghostStrike: new Map<string, number>(), paneKey, ROLE_SLOT_GRACE: 240_000,
    holdRolePane: () => false, detachPane: nothing, forgetDaemonEpoch: nothing, reanchorAuto: nothing, destroyPaneRuntime: nothing,
    panes, renderUsage: nothing, paneTitle: (t: string): string => String(t),
    makePane: async (sid: number, _title: string, sk?: string): Promise<FakeRt> => {
      const rt = makeRt(sid, sk);
      panes.set(paneKey(sid, sk), rt);
      return rt;
    },
    adoptSeat: (cands: { tree: unknown }[], _sk: string, sid: number) => ({ idx: 0, tree: { type: "split", dir: "row", ratio: 0.5, a: cands[0].tree, b: { type: "pane", sid } }, boundHole: null }),
    holeUntil: new Map<string, number>(), pruneHoleUntil: nothing, Date: { now: () => 9_000_000 },
    render: () => void renders.push("render"), refreshSidebarStatus: () => Promise.resolve(), setFocus: (sid: number) => void focus.push(sid),
    saveLayout: nothing, updateFtRoot: nothing, checkDeptFormationNotices: nothing,
    cwdBlockedNotices: () => ({ seen: new Set<string>(), notices: [] }), cwdBlockedSeen: new Set<string>(), openPrivacySettings: nothing, stickyToast: nothing,
    // ── F-01
    listTransport, roleActivity: new RoleActivityStore(31_000), dotRoles: new WeakMap<object, string | null>(), T_ACTIVITY: 31_000,
    roleDotColor, projectActivity, applyDotPaint, HIDDEN_DOT,
    performance: { now: () => mono.t }, document: { visibilityState: "visible" },
  };
  const loaded = loadWith(FNS, deps);
  fns = loaded.fns;
  const scope = loaded.scope;
  const rt = (sid: number, sk: string): FakeRt => panes.get(paneKey(sid, sk)) as FakeRt;
  const row = (sid: number, role: string, act?: unknown): Record<string, unknown> => ({
    surface_id: sid, title: `t${sid}`, role, live_cwd: null, exited: false, usage: null, cwd_blocked: null, ...(act === undefined ? {} : { display_activity: act }),
  });
  const reply = (sk: string, rows: Record<string, unknown>[]): void => {
    replies[sk] = () => Promise.resolve({ surfaces: rows });
  };
  const hang = (sk: string): void => {
    replies[sk] = () => new Promise<unknown>(() => undefined);
  };
  const tick = async (): Promise<boolean> => {
    let threw = false;
    try {
      await fns.refreshPaneTitles();
    } catch {
      threw = true;
    }
    await flush();
    return threw;
  };
  return { mono, invokes, renders, focus, panes, workspaces, deps, scope, fns, rt, row, reply, hang, tick };
}
const WORK = (seq: number, over: Record<string, unknown> = {}): Record<string, unknown> => WIRE({ observation_seq: seq, ...over });

describe("★F-01 U-F05/U-F06 — 실제 refreshPaneTitles 본문: 활동 패스는 바쁨 가드 앞 · 예외는 갇힌다 · 렌더러는 하나", () => {
  it("dispatch and projection run before the busy guard while adoption/title consumption is held by refreshing=true", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    await h.tick();
    expect([h.rt(3, "/b.sock").roleEl.state, h.rt(3, "/b.sock").titleEl.textContent]).toEqual(["act-working", "t3"]);
    h.reply("/b.sock", [{ ...h.row(3, "cso", WORK(2, { state: "waiting", raw: "idle", reason: "settled idle" })), title: "u3" }]);
    h.scope.refreshing = true; // 앞 틱의 입양·제목 소비가 아직 진행 중
    h.mono.t += 3_000;
    await h.tick();
    await flush();
    expect(h.invokes).toEqual(["list_surfaces:/a.sock", "list_surfaces:/b.sock", "list_surfaces:/a.sock", "list_surfaces:/b.sock"]);
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-waiting"); // 응답마다 독립 투영
    expect(h.rt(3, "/b.sock").titleEl.textContent).toBe("t3"); // 소비자(제목·입양)는 겹쳐 돌지 않았다
    expect(h.scope.refreshing).toBe(true);
  });

  it("a throw before the busy guard (activity pass) is contained by its outer catch — titles and adoption still run", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1)), h.row(4, "worker", WORK(1))]);
    (h.panes as unknown as { values: () => never }).values = () => {
      throw new Error("sweep iteration boom");
    };
    const threw = await h.tick();
    expect({ threw }).toEqual({ threw: false });
    expect([h.rt(1, "/a.sock").titleEl.textContent, h.rt(3, "/b.sock").titleEl.textContent]).toEqual(["t1", "t3"]);
    expect(h.panes.has("/b.sock#4")).toBe(true); // 입양도 돌았다
    expect(h.renders.length).toBe(1);
  });

  it("first-pane paint throw: the per-pane catch lets every sibling age (Working → stale) on the local sweep", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    await h.tick();
    expect([h.rt(2, "/a.sock").roleEl.state, h.rt(3, "/b.sock").roleEl.state]).toEqual(["act-working", "act-working"]);
    h.rt(1, "/a.sock").roleEl.throwOnPaint = true;
    h.hang("/a.sock");
    h.hang("/b.sock");
    h.mono.t += STALE_WORKING_MS + 1_000;
    await h.tick();
    expect([h.rt(2, "/a.sock").roleEl.state, h.rt(3, "/b.sock").roleEl.state]).toEqual(["act-stale", "act-stale"]);
    expect(/마지막 관측 1[01]초 전/.test(h.rt(3, "/b.sock").roleEl.title)).toBe(true);
  });

  it("malformed second row: only that dot is Unknown; siblings stay Working; the snapshot is still consumed for titles", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", { state: "busy", raw: "active", reason: "x", retained: false })]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    await h.tick();
    expect([1, 2].map((s) => h.rt(s, "/a.sock").roleEl.state).concat(h.rt(3, "/b.sock").roleEl.state)).toEqual(["act-working", "act-unknown", "act-working"]);
    expect(h.rt(2, "/a.sock").roleEl.title).toBe("역할: master · 상태 모름 · 활동 정보 형식 오류");
    expect([1, 2].map((s) => h.rt(s, "/a.sock").titleEl.textContent)).toEqual(["t1", "t2"]);
  });

  it("hung first endpoint: the healthy sibling's dot and title update; the hung endpoint gets no second original", async () => {
    const h = harness();
    h.hang("/a.sock");
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    await h.tick();
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-working");
    expect(h.rt(3, "/b.sock").titleEl.textContent).toBe("t3");
    expect(h.rt(1, "/a.sock").titleEl.textContent).toBe("");
    h.mono.t += 3_000;
    h.reply("/b.sock", [h.row(3, "cso", WORK(2, { state: "waiting", raw: "idle", reason: "settled idle" }))]);
    await h.tick();
    expect(h.invokes).toEqual(["list_surfaces:/a.sock", "list_surfaces:/b.sock", "list_surfaces:/b.sock"]);
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-waiting");
  });

  it("title refresh and adoption project through the one renderer: the activity class and reason/age survive (no role-only writer)", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1)), h.row(4, "worker", WORK(1))]);
    await h.tick();
    for (const [sid, sk, role] of [[1, "/a.sock", "worker"], [3, "/b.sock", "cso"], [4, "/b.sock", "worker"]] as const) {
      const el = h.rt(sid, sk).roleEl;
      expect({ sid, state: el.state, title: el.title }).toEqual({ sid, state: "act-working", title: `역할: ${role} · 작업 중 · active footer · 관측 1초 전` });
      expect(el.attrs["aria-label"]).toBe(el.title);
      expect(el.style.display).toBe("");
    }
  });

  it("dot updates never call render/setFocus, never replace a runtime or its terminal host, never touch scroll", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    const before = [...h.panes.values()].map((r) => [r, r.termHost, r.termHost.scrollTop] as const);
    for (let i = 2; i < 5; i++) {
      h.mono.t += 3_000;
      h.reply("/a.sock", [h.row(1, "worker", WORK(i)), h.row(2, "master", WORK(i, { state: "waiting", raw: "idle" }))]);
      await h.tick();
    }
    h.mono.t += 30_000;
    await h.tick();
    expect({ renders: h.renders.length, focus: h.focus.length }).toEqual({ renders: 0, focus: 0 });
    const after = [...h.panes.values()].map((r) => [r, r.termHost, r.termHost.scrollTop] as const);
    expect(after.length).toBe(before.length);
    after.forEach((x, i) => expect([x[0] === before[i][0], x[1] === before[i][1], x[2]]).toEqual([true, true, before[i][2]]));
  });

  it("visibility return expires an old Working synchronously before it is shown; hidden does nothing", async () => {
    const h = harness();
    h.reply("/a.sock", [h.row(1, "worker", WORK(1)), h.row(2, "master", WORK(1))]);
    h.reply("/b.sock", [h.row(3, "cso", WORK(1))]);
    await h.tick();
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-working");
    h.mono.t += 40_000;
    (h.deps.document as { visibilityState: string }).visibilityState = "hidden";
    h.fns.onVisibilityReturn();
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-working");
    (h.deps.document as { visibilityState: string }).visibilityState = "visible";
    h.fns.onVisibilityReturn();
    expect(h.rt(3, "/b.sock").roleEl.state).toBe("act-unknown");
    expect(h.rt(3, "/b.sock").roleEl.title).toContain("응답 만료");
  });
});

// ★U 게이트 R1 minor 11 — 데몬 wire ↔ UI 파서 **교차 핀**. 한쪽이 열쇠 이름이나 열거 값을 바꾸면 모든 점이 '형식 오류 → 빈 Unknown' 으로
//   조용히 떨어지는데(실패 신호 없음), 그것을 이 검체가 적색으로 바꾼다. 데몬 쪽은 src/bin/cysd/activity.rs 의 wire_at·unknown_wire 의 json! 열쇠와
//   Effective/Raw/Family 의 as_str 값을, UI 쪽은 이 파일이 시험하는 roleactivity.ts 의 parseDisplayActivity 가 읽는 열쇠와 STATES/RAWS/FAMILIES 를 소스에서 뽑는다.
//   한계: 문자열 추출이다 — 데몬이 열쇠를 json! 밖에서(삽입 등) 더하면 보이지 않는다. 그런 변경은 이 검체의 추출 규칙도 함께 고쳐야 한다.
describe("F-01 wire cross-pin — daemon display_activity keys and enums == UI parser", () => {
  const RS = readFileSync(new URL("../../src/bin/cysd/activity.rs", import.meta.url), "utf-8");
  const TS = readFileSync(new URL("./roleactivity.ts", import.meta.url), "utf-8");
  const slice = (src: string, head: string): string => {
    const a = src.indexOf(head);
    expect({ head, found: a >= 0 }).toEqual({ head, found: true });
    const b = src.indexOf("\n}\n", a);
    expect({ head, end: b > a }).toEqual({ head, end: true });
    return src.slice(a, b);
  };
  const jsonKeys = (body: string): string[] => {
    const a = body.indexOf("json!({");
    expect(a >= 0).toBe(true);
    const seg = body.slice(a, body.indexOf("})", a));
    return [...seg.matchAll(/"([a-z_]+)":/g)].map((m) => m[1]).sort();
  };
  const asStr = (enumName: string): string[] =>
    [...slice(RS, `impl ${enumName} {`).matchAll(/=> "([a-z_ ]+)"/g)].map((m) => m[1]).sort();
  const tsList = (name: string): string[] => {
    const m = new RegExp(`const ${name}: readonly string\\[\\] = \\[([^\\]]*)\\]`).exec(TS);
    expect({ name, found: m !== null }).toEqual({ name, found: true });
    return [...(m as RegExpExecArray)[1].matchAll(/"([a-z_]+)"/g)].map((x) => x[1]).sort();
  };
  const parserKeys = (): string[] => {
    const body = slice(TS, "export function parseDisplayActivity(");
    const ks = new Set<string>();
    for (const m of body.matchAll(/\bo\.([a-z_]+)/g)) ks.add(m[1]);
    for (const m of body.matchAll(/"([a-z_]+)" in o/g)) ks.add(m[1]);
    return [...ks].sort();
  };

  it("every key the daemon sends is read by the parser, and the parser reads no key the daemon does not send", () => {
    const daemon = jsonKeys(slice(RS, "pub(crate) fn wire_at("));
    expect(daemon.length).toBe(12);
    expect(parserKeys()).toEqual(daemon);
  });
  it("the degenerate Unknown carries exactly the four keys the parser accepts without an identity", () => {
    expect(jsonKeys(slice(RS, "fn unknown_wire("))).toEqual(["raw", "reason", "retained", "state"]);
  });
  it("state / raw / family values are the same sets on both sides", () => {
    expect(asStr("Effective")).toEqual(tsList("STATES"));
    expect(asStr("Raw")).toEqual(tsList("RAWS"));
    expect(asStr("Family")).toEqual(tsList("FAMILIES"));
  });
  it("an object built from the daemon's key spelling parses as an observation (not malformed)", () => {
    const vals: Record<string, unknown> = {
      state: "working", raw: "active", reason: "", family: "claude", grammar_revision: "claude-r1",
      observed_age_secs: 0.5, last_positive_age_secs: 0.5, stable_age_secs: 2, observation_seq: 7,
      display_epoch: "e1", retained: false, consecutive_rejections: 0,
    };
    const daemon = jsonKeys(slice(RS, "pub(crate) fn wire_at("));
    const obj = Object.fromEntries(daemon.map((k) => [k, vals[k]]));
    expect(Object.values(obj).includes(undefined)).toBe(false);
    const p = parseDisplayActivity(obj);
    expect(p.kind).toBe("observation");
  });
});
