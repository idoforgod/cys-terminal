// 0.14.51 TOAST 축 시험 — 데몬 이벤트 출처의 사람 싱크가 전부 꺼졌는가, 그리고 꺼지지 말아야 할 것이 그대로인가.
//
// 방식: main.ts 는 DOM·Tauri 를 import 해 그대로 못 올린다 → 필요한 **함수 소스를 main.ts 에서 그대로 떠서**(최상위 `function` 은 0열에서 시작해 0열 `}` 로 끝난다)
//   Bun.Transpiler 로 TS→JS 로 바꾸고, `with(Proxy)` 샌드박스에서 실행한다. 모르는 식별자는 스파이(호출 기록)이고 document 는 가짜다.
//   시험이 보는 것은 **실제 main.ts 의 코드**다(사본을 따로 쓰지 않는다) — 소스를 바꾸면 이 시험이 그대로 따라 적색이 된다.
//
// 이 파일이 지키는 것:
//   T-1  이벤트 전수(2-B 13분기 + category×name 퍼즈) 에서 #toasts 자식 0 · OS 배너 0 · 알람 이력 0 · Feed 화면 전환 0  (+ 양성 대조: quiet=false 와 직접 호출은 그대로 보인다)
//   AT-10 같은 이벤트들이 부르는 비표시 부작용(refreshSidebarStatus·refreshFeed·upsertTaskCell·removeDeadPane·마우스 소등 정합기)이 기대표대로 보존
//   출처 핀  onDaemonEvent 본문에 toast/stickyToast/osBanner/openFeed 직접 호출 0 · toast/sticky/osBanner 본문에 통짜 가드 0
//   커버리지  소스의 alertsQuiet("<sink>") 개수 == EXPECTED_GUARDS · 런마다 선언 출력
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  CHANNEL_COUNT,
  CLI_PROSE_CALLSITES,
  EXPECTED_GUARDS,
  QUIET_SINKS,
  ROUTE_COVERED,
  alertsQuiet,
  coverageDeclaration,
} from "./alertquiet";

const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
const LINES = SRC.split("\n");

/** 최상위 `[async ]function name(` 0열 선언을 0열 `}` 까지 떠낸다. 없거나 둘이면 시험이 죽는다(소스 이동을 침묵 통과시키지 않는다). */
function extractFn(name: string): string {
  const re = new RegExp(`^(async )?function ${name}\\(`);
  const starts = LINES.map((l, i) => (re.test(l) ? i : -1)).filter((i) => i >= 0);
  if (starts.length !== 1) throw new Error(`function ${name}: ${starts.length} declarations`);
  let end = -1;
  for (let i = starts[0]; i < LINES.length; i++) {
    if (LINES[i] === "}") {
      end = i;
      break;
    }
  }
  if (end < 0) throw new Error(`function ${name}: no closing brace at column 0`);
  return LINES.slice(starts[0], end + 1).join("\n");
}

/** 주석(//, ///, /* *​/)을 걷는다 — 문자열 안의 `//` 는 이 소스의 대상 함수들에서 URL 이 없어 안전하다(아래 단언이 핀). */
function stripComments(code: string): string {
  return code
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .map((l) => l.replace(/(^|[^:"'`])\/\/.*$/, "$1"))
    .join("\n");
}

const FN_NAMES = [
  "recordAlarm",
  "addToastCloseButton",
  "toast",
  "stickyToast",
  "dismissToast",
  "osBanner",
  "daemonToast",
  "daemonStickyToast",
  "daemonOsBanner",
  "daemonOpenFeed",
  "onDaemonEvent",
  "renderAlerts",
  "refreshFeed",
  "renderBulkConfirm",
  "renderOtherWorkspacePending",
  "updatePendingBadges",
  "updateCcStale",
  "renderTasks",
  "refreshControlCenter",
] as const;

type Spy = ((...a: unknown[]) => unknown) & { calls: unknown[][] };
function spy(impl?: (...a: unknown[]) => unknown): Spy {
  const f = ((...a: unknown[]) => {
    f.calls.push(a);
    return impl ? impl(...a) : undefined;
  }) as Spy;
  f.calls = [];
  return f;
}

/** 가짜 DOM 요소 — 자식 수·텍스트를 센다. */
function fakeEl(): any {
  const el: any = {
    children: [] as unknown[],
    style: {},
    className: "",
    innerHTML: "",
    textContent: "",
    hidden: false,
    title: "",
    appendChild(c: unknown) {
      el.children.push(c);
      return c;
    },
    append(...c: unknown[]) {
      el.children.push(...c);
    },
    remove() {},
    addEventListener() {},
    querySelector: () => fakeEl(),
    querySelectorAll: () => [],
    classList: { add() {}, remove() {} },
    closest: () => null,
  };
  return el;
}

interface Sandbox {
  fn: Record<string, (...a: any[]) => any>;
  toasts: ReturnType<typeof fakeEl>;
  calls: Record<string, unknown[][]>;
  elements: Map<string, ReturnType<typeof fakeEl>>;
  stickyMap: Map<string, unknown>;
  store: Record<string, unknown>;
}

const transpiler = new (globalThis as unknown as { Bun: { Transpiler: new (o: { loader: string }) => { transformSync(s: string): string } } }).Bun.Transpiler({ loader: "ts" });

const RENDERERS = ["renderAlerts", "refreshFeed", "renderBulkConfirm", "renderOtherWorkspacePending", "updatePendingBadges", "updateCcStale", "renderTasks", "refreshControlCenter"];
/** renderers=false 면 그리기 함수 4개는 스파이로 남는다(이벤트 시험이 "refreshFeed 가 불렸는가"를 세려면 실물이 아니라 스파이여야 한다). */
function build(opts: { quiet: boolean; ccOpen?: boolean; slugs?: Record<string, string>; panes?: Map<string, unknown>; renderers?: boolean }): Sandbox {
  const names: string[] = FN_NAMES.filter((n) => opts.renderers || !RENDERERS.includes(n));
  const toasts = fakeEl();
  const elements = new Map<string, ReturnType<typeof fakeEl>>([["toasts", toasts]]);
  const calls: Record<string, unknown[][]> = {};
  const spies = new Map<string, Spy>();
  const getSpy = (n: string, impl?: (...a: unknown[]) => unknown): Spy => {
    let s = spies.get(n);
    if (!s) {
      s = spy(impl);
      spies.set(n, s);
      calls[n] = s.calls;
    }
    return s;
  };
  const store: Record<string, unknown> = {
    alarmHistory: [],
    ccOpen: opts.ccOpen ?? false,
    ccTab: "feed",
    ccFailStreak: 5,
    alertsView: null as unknown,
    HOLLOW_GRACE_UI: 0,
    feedRefreshGen: 0,
    TEAM_CREATE_KIND: "team-create-request",
    MOUSE_ALL_OFF: "\x1b[?1000l",
    BULK_CONFIRM_MAX: 20,
    panes: opts.panes ?? new Map(),
    socketForSlug: new Map(Object.entries(opts.slugs ?? {})),
    stickyToasts: new Map(),
    document: {
      getElementById: (id: string) => {
        getSpy("document.getElementById").calls.push([id]);
        return elements.get(id) ?? null;
      },
      createElement: () => fakeEl(),
    },
    window: { setTimeout: getSpy("window.setTimeout", () => 0) },
    setTimeout: () => 0,
    clearTimeout: () => {},
    alertsQuiet: opts.quiet ? alertsQuiet : () => false,
    pushAlarm: getSpy("pushAlarm", (h: unknown, a: unknown) => [...((h as unknown[]) ?? []), a]),
    toastTtl: () => ({ ttlMs: 1 }),
    toastClassName: () => "toast",
    toastTimerPlan: () => ({ clearPrevious: false, ttlMs: 1 }),
    needsExpiryBanner: () => false,
    isPermissionGranted: getSpy("isPermissionGranted", () => Promise.resolve(true)),
    requestPermission: getSpy("requestPermission", () => Promise.resolve("granted")),
    sendNotification: getSpy("sendNotification"),
    feedCreatedToastTitle: (k: unknown) => `feed:${String(k)}`,
    paneKey: (sid: number, sock: unknown) => `${String(sock)}:${sid}`,
    isClosingSid: () => false,
    formatAlarmTime: () => "t",
    ccEsc: (s: string) => s,
    isBulkConfirmable: () => true,
    deptPendingRows: () => [],
    renderAlarmHistory: getSpy("renderAlarmHistory"),
    openFeed: getSpy("openFeed"),
    alertsTracker: { step: () => (store as any).alertsView },
    readAlerts: getSpy("readAlerts", () => Promise.resolve([])),
    refreshAccountsShared: getSpy("refreshAccountsShared", () => Promise.resolve()),
    renderControlCenter: getSpy("renderControlCenter"),
    taskSeatIsWorking: () => false,
    taskSeatView: () => ({}),
    taskRow: () => "<row/>",
    invoke: getSpy("invoke", () => Promise.resolve({ items: [], groups: [], total: 0 })),
  };
  const out: Record<string, (...a: any[]) => any> = {};
  const proxy = new Proxy(store, {
    has: (_t, k) => typeof k === "string",
    get(t, k) {
      if (typeof k !== "string") return undefined;
      if (k in t) return t[k];
      if (k in globalThis) return (globalThis as any)[k];
      return getSpy(k);
    },
    set(t, k, v) {
      t[k as string] = v;
      return true;
    },
  });
  const src =
    "const stickyToasts = new Map();\n" +
    names.map((n) => transpiler.transformSync(extractFn(n))).join("\n") +
    `\n__out(${JSON.stringify(names)}.reduce((o,n)=>{o[n]=eval(n);return o;},{}));`;
  const run = new Function("__sb", `with(__sb){ ${src} }`);
  // 샌드박스 안에서 선언된 함수를 밖으로 꺼낸다 — eval(n) 은 with 범위 안의 선언을 본다.
  (store as any).__out = (o: Record<string, (...a: any[]) => any>) => Object.assign(out, o);
  run(proxy);
  return { fn: out, toasts, calls, elements, stickyMap: store.stickyToasts as Map<string, unknown>, store };
}

async function flush() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

// ---- 이벤트 전수: 2-B 13분기 + 분기 변형 ----
const EVENTS: { id: string; ev: Record<string, unknown>; expect: string[] }[] = [
  { id: "approval.request", ev: { name: "approval.request", category: "approval", payload: { role: "worker", surface_ref: "surface:3", excerpt: "run?" } }, expect: ["refreshFeed", "refreshSidebarStatus"] },
  { id: "approval.stalled", ev: { name: "approval.stalled", category: "approval", payload: { surface_ref: "surface:3", title: "t", age_secs: 400 } }, expect: ["refreshFeed", "refreshSidebarStatus"] },
  { id: "context.threshold>=80", ev: { name: "context.threshold", category: "context", payload: { context_pct: 91, threshold: 80, role: "master", action: "cycle" } }, expect: ["refreshSidebarStatus"] },
  { id: "context.threshold<80", ev: { name: "context.threshold", category: "context", payload: { context_pct: 61, threshold: 60 } }, expect: ["refreshSidebarStatus"] },
  { id: "pane.idle", ev: { name: "pane.idle", category: "pane", surface_id: 4, payload: { idle_seconds: 300 } }, expect: ["refreshSidebarStatus"] },
  { id: "master.idle", ev: { name: "master.idle", category: "info", surface_id: 1, socket_slug: "base", payload: { role: "master", idle_secs: 600, threshold_secs: 300 } }, expect: ["refreshSidebarStatus"] },
  { id: "agent.exited", ev: { name: "agent.exited", category: "agent", surface_id: 7, payload: { role: "worker" } }, expect: ["refreshSidebarStatus"] },
  { id: "master.deadman", ev: { name: "master.deadman", category: "alert", surface_id: 1, payload: { reason: "agent process dead", role: "master" } }, expect: [] },
  { id: "queue.starved", ev: { name: "queue.starved", category: "queue", surface_id: 2, payload: {} }, expect: [] },
  { id: "queue.delivered", ev: { name: "queue.delivered", category: "queue", surface_id: 2, payload: {} }, expect: [] },
  { id: "status.changed", ev: { name: "status.changed", category: "status", surface_id: 5, socket_slug: "dept-5", payload: { state: "working" } }, expect: ["refreshSidebarStatus", "upsertTaskCell"] },
  { id: "task.changed", ev: { name: "task.changed", category: "task", surface_id: 5, socket_slug: "dept-5", payload: { task: "x" } }, expect: ["upsertTaskCell"] },
  { id: "osc.notify", ev: { name: "osc.notify", category: "osc", surface_id: 6, payload: { title: "T", body: "B" } }, expect: [] },
  { id: "health", ev: { name: "health.stuck", category: "health", surface_id: 8, payload: { rule: "r", line: "l" } }, expect: [] },
  { id: "watchdog.dup", ev: { name: "watchdog.duplicate_procs", category: "watchdog", payload: { count: 2, cmdline: "node x" } }, expect: [] },
  { id: "watchdog.other", ev: { name: "watchdog.load_high", category: "watchdog", payload: { load: 9 } }, expect: [] },
  { id: "feed.created.team", ev: { name: "feed.item.created", category: "feed", payload: { kind: "team-create-request", title: "팀", request_id: "r1" } }, expect: ["refreshFeed", "refreshSidebarStatus"] },
  { id: "feed.created.wait", ev: { name: "feed.item.created", category: "feed", payload: { kind: "formation", title: "편성", request_id: "r2", wait: true } }, expect: ["refreshFeed", "refreshSidebarStatus"] },
  { id: "feed.other", ev: { name: "feed.item.resolved", category: "feed", payload: {} }, expect: ["refreshFeed", "refreshSidebarStatus"] },
  { id: "surface.exited", ev: { name: "surface.exited", category: "surface", surface_id: 9, payload: {} }, expect: ["removeDeadPane"] },
  { id: "surface.closed", ev: { name: "surface.closed", category: "surface", surface_id: 9, payload: {} }, expect: ["removeDeadPane"] },
  { id: "surface.reaped", ev: { name: "surface.reaped", category: "surface", surface_id: 9, payload: {} }, expect: ["removeDeadPane"] },
];

/** 비표시 부작용으로 세는 호출 이름(AT-10 기대표가 비교하는 모집단). */
const SIDE_EFFECTS = ["refreshSidebarStatus", "refreshFeed", "upsertTaskCell", "removeDeadPane"];
const SINK_CALLS = ["sendNotification", "isPermissionGranted", "requestPermission", "pushAlarm", "openFeed", "scheduleFeedSwitchIfStillPending"];

function sideEffectList(sb: Sandbox): string[] {
  const out: string[] = [];
  for (const n of SIDE_EFFECTS) for (let i = 0; i < (sb.calls[n]?.length ?? 0); i++) out.push(n);
  return out.sort();
}

describe("TOAST 축 T-1 — 데몬 이벤트가 밀어 올린 사람 싱크 0", () => {
  for (const c of EVENTS) {
    test(`quiet: ${c.id} → 박스 0 · OS 배너 0 · 알람 이력 0 · Feed 전환 0`, async () => {
      const sb = build({ quiet: true });
      sb.fn.onDaemonEvent(c.ev);
      await flush();
      expect(sb.toasts.children.length).toBe(0);
      for (const n of SINK_CALLS) expect([n, sb.calls[n]?.length ?? 0]).toEqual([n, 0]);
      expect(sb.stickyMap.size).toBe(0);
    });
  }

  test("category × name 퍼즈 — 어떤 조합도 박스·배너·알람 이력·Feed 전환을 만들지 않는다", async () => {
    const cats = ["", "health", "watchdog", "feed", "info", "alert", "approval", "context", "osc", "unknown-x"];
    const names = ["", "x", "approval.request", "approval.stalled", "context.threshold", "pane.idle", "master.idle", "agent.exited", "master.deadman", "queue.starved", "queue.delivered", "status.changed", "task.changed", "osc.notify", "feed.item.created", "watchdog.duplicate_procs", "surface.exited", "surface.closed", "surface.reaped", "health.stuck"];
    let n = 0;
    for (const category of cats)
      for (const name of names) {
        const sb = build({ quiet: true });
        sb.fn.onDaemonEvent({ name, category, surface_id: 3, payload: { kind: "k", title: "t", context_pct: 95 } });
        await flush();
        expect(`${category}/${name}:${sb.toasts.children.length}`).toBe(`${category}/${name}:0`);
        for (const s of SINK_CALLS) expect(`${category}/${name}:${s}:${sb.calls[s]?.length ?? 0}`).toBe(`${category}/${name}:${s}:0`);
        n++;
      }
    expect(n).toBe(cats.length * names.length);
  });

  // ---- 양성 대조: 시험이 '보이는 것'을 실제로 볼 수 있다는 증명 ----
  test("양성 대조 A — 가드를 푼 상태(quiet=false)에서는 같은 이벤트가 박스·배너·알람 이력·Feed 전환을 만든다", async () => {
    const sb = build({ quiet: false });
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "approval.request")!.ev);
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "approval.stalled")!.ev);
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "master.idle")!.ev);
    await flush();
    expect(sb.toasts.children.length).toBeGreaterThanOrEqual(3);
    expect(sb.calls.sendNotification?.length ?? 0).toBeGreaterThanOrEqual(2);
    expect(sb.calls.pushAlarm?.length ?? 0).toBeGreaterThanOrEqual(3);
    expect(sb.calls.openFeed?.length ?? 0).toBe(1);
  });

  test("양성 대조 B — 사용자 행동 응답·앱 백엔드 알림은 toast/stickyToast/osBanner 를 직접 불러 quiet=true 에서도 그대로 보인다", async () => {
    const sb = build({ quiet: true });
    sb.fn.toast("system", "전송 실패", "surface:3");
    sb.fn.stickyToast("update-x", "system", "업데이트", "새 판");
    await sb.fn.osBanner("앱 백엔드", "번들 손상");
    await flush();
    expect(sb.toasts.children.length).toBe(2);
    expect(sb.calls.sendNotification?.length ?? 0).toBe(1);
    expect(sb.calls.pushAlarm?.length ?? 0).toBe(2);
  });
});

describe("TOAST 축 AT-10 — 끄는 것이 비표시 부작용을 죽이지 않는다 (onDaemonEvent early return 금지)", () => {
  for (const c of EVENTS) {
    test(`부작용 보존: ${c.id} → [${c.expect.join(",")}]`, async () => {
      const quiet = build({ quiet: true });
      quiet.fn.onDaemonEvent(c.ev);
      await flush();
      const loud = build({ quiet: false });
      loud.fn.onDaemonEvent(c.ev);
      await flush();
      expect(sideEffectList(quiet)).toEqual([...c.expect].sort());
      // quiet 와 non-quiet 가 부르는 비표시 부작용 목록이 같다 = 가드가 부작용을 건드리지 않는다.
      expect(sideEffectList(quiet)).toEqual(sideEffectList(loud));
    });
  }

  test("agent.exited — 소켓이 해석되고 pane 이 있으면 마우스 소등 정합기 예약(window.setTimeout)이 그대로 걸린다", async () => {
    const term = { write: spy() };
    const rt = { trackFilter: { generation: () => 1, clearLedgerIfGeneration: spy() }, term };
    const panes = new Map<string, unknown>([["sockA:7", rt]]);
    const quiet = build({ quiet: true, slugs: { "dept-5": "sockA" }, panes });
    quiet.fn.onDaemonEvent({ name: "agent.exited", category: "agent", surface_id: 7, socket_slug: "dept-5", payload: { role: "worker" } });
    await flush();
    expect(quiet.calls["window.setTimeout"]?.length ?? 0).toBe(1);
    expect(quiet.toasts.children.length).toBe(0);
    const loud = build({ quiet: false, slugs: { "dept-5": "sockA" }, panes });
    loud.fn.onDaemonEvent({ name: "agent.exited", category: "agent", surface_id: 7, socket_slug: "dept-5", payload: { role: "worker" } });
    expect(loud.calls["window.setTimeout"]?.length ?? 0).toBe(1);
  });
});

describe("TOAST 축 — 가드 싱크(함수 실행)", () => {
  test("renderAlerts — 경보 뷰를 받아도 배지·스트립 요소를 건드리지 않는다 / 가드가 풀리면 건드린다(양성 대조)", () => {
    const view = { hidden: false, text: "3", cls: "warn", title: "t", rows: [{ sev: "warn", icon: "!", msg: "m" }] };
    const q = build({ quiet: true, renderers: true });
    q.elements.set("cc-alertbadge", fakeEl());
    q.elements.set("cc-alerts", fakeEl());
    q.fn.renderAlerts(view);
    expect(q.calls["document.getElementById"]?.length ?? 0).toBe(0);
    expect(q.elements.get("cc-alerts")!.innerHTML).toBe("");
    const l = build({ quiet: false, renderers: true });
    l.elements.set("cc-alertbadge", fakeEl());
    l.elements.set("cc-alerts", fakeEl());
    l.fn.renderAlerts(view);
    expect(l.elements.get("cc-alerts")!.innerHTML).toContain("cc-alert-row");
    expect(l.elements.get("cc-alertbadge")!.textContent).toBe("3");
  });

  test("양성 대조 C — 승인 Feed 상호작용 면(카드·묶음 확인)은 quiet=true 에서도 그대로 조회·그려진다(부서장 판정 R1 §3 · 오너 승인)", async () => {
    const q = build({ quiet: true, ccOpen: true, renderers: true });
    const feedBox = fakeEl();
    q.elements.set("cc-feed-items", feedBox);
    await q.fn.refreshFeed();
    expect(q.calls.invoke?.length ?? 0).toBeGreaterThan(0); // 가드 때문에 조용히 0 이 되지 않는다
    expect(feedBox.textContent).toBe("(비어 있음)"); // 실제로 그려졌다
    const box = fakeEl();
    q.fn.renderBulkConfirm(box, [{ id: "a" }], undefined);
    expect(box.children.length).toBeGreaterThan(0);
  });

  test("양성 대조 D — 업데이트·안전모드 고지(stickyToast 직접 호출)는 quiet=true 에서 박스로 그려지고 알람 이력에 남는다", () => {
    const sb = build({ quiet: true });
    sb.fn.stickyToast("upd-bin", "feed", "⬇ 업데이트 설치", "다운로드 시작…");
    sb.fn.stickyToast("safe-mode", "health", "안전모드 — 설치 위치를 옮겨 주세요", "g");
    expect(sb.toasts.children.length).toBe(2);
    expect(sb.calls.pushAlarm?.length ?? 0).toBe(2);
  });

  test("updatePendingBadges — quiet 에서 배지 요소를 건드리지 않고, 가드가 풀리면 건드린다(양성 대조)", () => {
    const q = build({ quiet: true, renderers: true });
    const b1 = fakeEl(); b1.hidden = true;
    q.elements.set("cc-pending-badge", b1);
    q.fn.updatePendingBadges(3);
    expect(b1.hidden).toBe(true);
    const l = build({ quiet: false, renderers: true });
    const b2 = fakeEl(); b2.hidden = true;
    l.elements.set("cc-pending-badge", b2);
    l.fn.updatePendingBadges(3);
    expect([b2.hidden, b2.textContent]).toEqual([false, "3"]);
  });
});

describe("TOAST 축 — R1b 구멍 넷 (자동 전환 · 푸터 · 부서 오류 행 · daemon-info) + 공허 초록 방지", () => {
  test("자동 화면 전환 — feed.item.created(wait)에서 quiet 면 예약 호출 0 / 가드를 풀면 예약된다(양성 대조)", () => {
    const ev = { name: "feed.item.created", category: "feed", payload: { kind: "formation", title: "t", request_id: "r", wait: true } };
    const q = build({ quiet: true });
    q.fn.onDaemonEvent(ev);
    expect(q.calls.scheduleFeedSwitchIfStillPending?.length ?? 0).toBe(0);
    const l = build({ quiet: false });
    l.fn.onDaemonEvent(ev);
    expect(l.calls.scheduleFeedSwitchIfStillPending?.length ?? 0).toBe(1);
  });

  test("AT-3 — renderAlerts 는 refreshControlCenter 경유로 구동한다(onDaemonEvent 입력만으로는 공허): quiet 면 경보 배지·스트립 0 / 가드를 풀면 그려진다", async () => {
    const view = { hidden: false, text: "3", cls: "warn", title: "t", rows: [{ sev: "warn", icon: "!", msg: "m" }] };
    const run = async (quiet: boolean) => {
      const sb = build({ quiet, renderers: true });
      sb.store.ccOpen = true; // 전제를 시험 안에서 세운다
      sb.store.ccTab = "live";
      sb.store.alertsView = view;
      sb.elements.set("cc-alertbadge", fakeEl());
      sb.elements.set("cc-alerts", fakeEl());
      await sb.fn.refreshControlCenter();
      return sb;
    };
    const q = await run(true);
    expect(q.calls.readAlerts?.length ?? 0).toBe(1); // 경로가 실제로 renderAlerts 앞까지 갔다(공허 방지)
    expect(q.elements.get("cc-alerts")!.innerHTML).toBe("");
    const l = await run(false);
    expect(l.elements.get("cc-alerts")!.innerHTML).toContain("cc-alert-row");
  });

  test("푸터 「⚠ 데몬 응답 없음」 — ccFailStreak≥3 전제를 세우고 단언: quiet 면 문구 0 / 가드를 풀면 문구가 선다", () => {
    const run = (quiet: boolean) => {
      const sb = build({ quiet, renderers: true });
      sb.store.ccFailStreak = 5;
      const f = fakeEl();
      sb.elements.set("cc-footer", f);
      sb.fn.updateCcStale();
      return f.textContent as string;
    };
    expect(run(true)).toBe("");
    expect(run(false)).toContain("데몬 응답 없음");
  });

  test("cc-fail-badge(9번째 채널)·부서 오류 행 — d.error 입력: quiet 면 배지·오류 문구 0 / 가드를 풀면 둘 다 그려진다", () => {
    const run = (quiet: boolean) => {
      const sb = build({ quiet, renderers: true });
      const host = fakeEl();
      sb.elements.set("cc-tasks-depts", host);
      sb.fn.renderTasks({ departments: [{ name: "dept-9", display_name: "부서9", error: "timeout", surfaces: [] }] });
      return host.innerHTML as string;
    };
    const q = run(true);
    expect(q).toContain("부서9"); // 부서 머리는 그려진다(렌더가 실제로 돌았다)
    expect(q).not.toContain("cc-fail-badge");
    expect(q).not.toContain("응답 없음");
    const l = run(false);
    expect(l).toContain("cc-fail-badge");
    expect(l).toContain("부서 데몬 응답 없음");
  });

  // ★남김 단언(부서장 판정 R2 · T-TOAST-FIX-DAEMONINFO): #daemon-info 의 세 표기는 부트·복구 실패를 구별하는 유일한 신호(제품 주석 :11259-11265 의 「백지」·「자가치유 영구 꺼짐」)라
  //   끄지 않는다. 억제 가드(alertsQuiet) 가 한 줄이라도 붙으면 적색 — 통상 변이가 아니라 억제 *추가* 가 죽는 시험이다.
  const KEPT_DAEMON_INFO: [string, string][] = [
    ["daemon-error 리스너(부트 대기 중 신호)", "info.textContent = `daemon error: ${e.payload}`;"],
    ["start().catch — startup failed", 'document.getElementById("daemon-info")!.textContent = `startup failed: ${e}`;'],
    ["내부 catch — recovery failed", 'document.getElementById("daemon-info")!.textContent = `startup failed: ${e} / recovery failed: ${e2}`;'],
  ];
  for (const [label, stmt] of KEPT_DAEMON_INFO) {
    test(`남김: ${label} — 무조건 실행(가드·조건 없이 줄 머리에서 시작)`, () => {
      const rows = SRC.split("\n").filter((l) => l.includes(stmt));
      expect(rows.length).toBe(1);
      expect(rows[0].trim()).toBe(stmt); // 앞에 if/가드/삼항이 붙지 않았다 · 뒤 주석도 없다
      expect(/alertsQuiet\("daemonInfo"\)/.test(SRC)).toBe(false);
      expect((QUIET_SINKS as readonly string[]).includes("daemonInfo")).toBe(false);
    });
  }
});

describe("TOAST 축 — 출처 핀 · 통짜 가드 금지 · 커버리지", () => {
  const body = stripComments(extractFn("onDaemonEvent"));

  test("onDaemonEvent 본문에는 toast/stickyToast/osBanner/openFeed 직접 호출이 없다(래퍼만)", () => {
    for (const n of ["toast", "stickyToast", "osBanner", "openFeed"]) {
      const re = new RegExp(`(?<![A-Za-z0-9_.])${n}\\(`, "g");
      expect([n, (body.match(re) ?? []).length]).toEqual([n, 0]);
    }
    expect((body.match(/daemonToast\(/g) ?? []).length).toBe(11);
    expect((body.match(/daemonStickyToast\(/g) ?? []).length).toBe(1);
    expect((body.match(/daemonOsBanner\(/g) ?? []).length).toBe(5);
    expect((body.match(/daemonOpenFeed\(/g) ?? []).length).toBe(1);
    // 유예 후 자동 전환 예약은 호출부 인라인 가드(probefail 핀 보존): 가드 줄 + 호출 1
    expect(body).toContain('(payload.wait === true || autoRoute) && !alertsQuiet("feedSwitch")');
    expect((body.match(/(?<![A-Za-z0-9_])scheduleFeedSwitchIfStillPending\(/g) ?? []).length).toBe(1);
  });

  test("onDaemonEvent 에는 앞쪽 early return 가드가 없다 — 첫 문장이 alertsQuiet 가 아니다", () => {
    expect((body.match(/alertsQuiet\(/g) ?? []).length).toBe(1); // 유일: feed.item.created 의 유예 후 전환 예약 인라인 가드(early return 아님)
    const first = body.split("\n").slice(1).find((l) => l.trim() !== "")!;
    expect(first.trim().startsWith("const name")).toBe(true);
  });

  test("toast/stickyToast/osBanner 본문에는 통짜 가드가 없다(출처를 구별할 수 없는 자리)", () => {
    for (const n of ["toast", "stickyToast", "osBanner", "recordAlarm"]) {
      expect([n, /alertsQuiet\(/.test(stripComments(extractFn(n)))]).toEqual([n, false]);
    }
  });

  test("feed 스케줄러의 자동 화면 전환은 daemonOpenFeed 를 지난다 / 팔레트 수동 열기는 openFeed 그대로", () => {
    expect(/^\s*open: openFeed,/m.test(SRC)).toBe(true); // 스케줄러 배선은 그대로(probefail 핀) — 가드는 호출부 daemonScheduleFeedSwitch
    expect(/^\s*open: daemonOpenFeed,/m.test(SRC)).toBe(false);
  });

  test("래퍼 4개는 첫 문장이 alertsQuiet 가드다", () => {
    const want: Record<string, string> = { daemonToast: "toast", daemonStickyToast: "stickyToast", daemonOsBanner: "osBanner", daemonOpenFeed: "feedSwitch" };
    for (const [fn, sink] of Object.entries(want)) {
      const first = stripComments(extractFn(fn)).split("\n").slice(1).find((l) => l.trim() !== "")!;
      expect(first.trim()).toBe(`if (alertsQuiet("${sink}")) return;`);
    }
  });

  test("업데이트·안전모드 고지 호출부는 래퍼가 아니라 직접 호출이다(최소 8곳) · Feed 상호작용 함수에는 가드가 없다", () => {
    const code = stripComments(SRC);
    const sites = code.split("\n").filter((l) => /["']upd-(pack|bin)["']|["']safe-mode["']|UPDATE_FAILED_TOAST_ID,/.test(l) && /[sS]tickyToast\(/.test(l));
    expect(sites.length).toBeGreaterThanOrEqual(8);
    for (const l of sites) expect(/daemonStickyToast\(/.test(l)).toBe(false);
    for (const n of ["refreshFeed", "renderBulkConfirm", "renderOtherWorkspacePending"]) expect([n, /alertsQuiet\(/.test(stripComments(extractFn(n)))]).toEqual([n, false]);
  });

  test("탭 부제 점(wsDot) — 점 부착·툴팁·심각도 색이 가드 뒤에 있고 판정(summarizeWsSigsSafe)은 그대로 돈다", () => {
    const i = SRC.indexOf('const wsQuiet = alertsQuiet("wsDot");');
    expect(i).toBeGreaterThan(0);
    const seg = SRC.slice(i, i + 700);
    expect(seg).toContain("if (!wsQuiet) sub.appendChild(dot);");
    expect(seg).toContain("sum.title && !wsQuiet");
    expect(seg).toContain("wsQuiet ? subBits.slice(0, firstTitle ? 2 : 1) : subBits");
  });

  test("사이드바 ⚠ 배지 · CC 실패 배지는 소스 슬라이스에서 가드를 지난다", () => {
    expect(/workspaces\.indexOf\(ws\) === activeWs && !alertsQuiet\("sidebarBadge"\)/.test(SRC)).toBe(true);
    expect(/const deadBadge = d\.error && !alertsQuiet\("ccFailBadge"\)/.test(SRC)).toBe(true);
  });

  test("커버리지 — 소스의 alertsQuiet(\"<sink>\") 호출 수가 EXPECTED_GUARDS 와 정확히 같다 + 선언 출력", () => {
    const counts: Record<string, number> = {};
    for (const m of stripComments(SRC).matchAll(/alertsQuiet\("(\w+)"\)/g)) counts[m[1]] = (counts[m[1]] ?? 0) + 1;
    for (const s of QUIET_SINKS) expect([s, counts[s] ?? 0]).toEqual([s, EXPECTED_GUARDS[s]]);
    expect(Object.keys(counts).sort()).toEqual([...QUIET_SINKS].sort());
    expect(CHANNEL_COUNT).toBe(13); // UI 가드 11 + 경로 덮임 1(recordAlarm) + CLI 군 1(호출부 7) = 13 (UI 12 + CLI 1)
    expect(ROUTE_COVERED).toEqual(["recordAlarm"]);
    expect(CLI_PROSE_CALLSITES).toBe(7);
    // eslint-disable-next-line no-console
    console.log(coverageDeclaration(counts));
  });

  test("전 이벤트 분기를 이 시험이 덮는다 — onDaemonEvent 의 name 분기 수와 EVENTS 표가 일치", () => {
    const names = new Set([...body.matchAll(/name === "([\w.]+)"/g)].map((m) => m[1]));
    for (const n of names) expect([n, EVENTS.some((e) => e.ev.name === n)]).toEqual([n, true]);
    expect(names.size).toBe(17); // approval.request/stalled · context.threshold · pane.idle · master.idle/deadman · agent.exited · queue.starved/delivered · status.changed · task.changed · osc.notify · feed.item.created · watchdog.duplicate_procs · surface.exited/closed/reaped
  });
});
