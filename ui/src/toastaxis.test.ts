// 0.14.51 TOAST 축 시험 — 데몬 이벤트 출처의 사람 싱크가 전부 꺼졌는가, 그리고 꺼지지 말아야 할 것이 그대로인가.
//
// 방식: main.ts 는 DOM·Tauri 를 import 해 그대로 못 올린다 → 필요한 **함수 소스를 main.ts 에서 그대로 떠서**(최상위 `function` 은 0열에서 시작해 0열 `}` 로 끝난다)
//   Bun.Transpiler 로 TS→JS 로 바꾸고, `with(Proxy)` 샌드박스에서 실행한다. 모르는 식별자는 스파이(호출 기록)이고 document 는 가짜다.
//   시험이 보는 것은 **실제 main.ts 의 코드**다(사본을 따로 쓰지 않는다) — 소스를 바꾸면 이 시험이 그대로 따라 적색이 된다.
//
// 이 파일이 지키는 것:
//   T-1  이벤트 전수(2-B 13분기 + category×name 퍼즈) 에서 #toasts 자식 0 · OS 배너 0 · 알람 이력 0 · Feed 화면 전환 0  (+ 양성 대조: quiet=false 와 직접 호출은 그대로 보인다)
//        ※ 예외는 하나: `--wait` 대기 항목(wait===true ∧ auto_route!==true)의 feed.item.created 가 내는 OS 배너 1면 — 아래 AT-19/AT-20 이 따로 지킨다.
//   AT-10 같은 이벤트들이 부르는 비표시 부작용(refreshSidebarStatus·refreshFeed·upsertTaskCell·removeDeadPane·마우스 소등 정합기)이 기대표대로 보존
//   R2   AT-19  `--wait` 요청자 대기 항목에는 stalled 가 닿지 않는다(그 스캔은 kind==approval ∧ 데몬 발행 id 만 · feed.push 는 일반 id 만 만든다) — 소스를 읽어 단언
//        AT-20  producer(handlers.rs)가 실제로 내는 feed.item.created 모양으로 소비자(onDaemonEvent)까지: 배너 정확히 1 · 토스트 0 · 전환 0 + 양성 대조(auto_route 항목 0 · approval.request 0)
//   출처 핀  onDaemonEvent 본문의 직접 호출은 **넷뿐**(approval.stalled 의 toast/osBanner/openFeed 3 + feed.item.created 의 osBanner 1) · approval.request 는 직접 호출 0 · toast/sticky/osBanner 본문에 통짜 가드 0
//   커버리지  소스의 alertsQuiet("<sink>") 개수 == EXPECTED_GUARDS · 런마다 선언 출력(채널 13 = UI 가드 11 + 경로 덮임 1 + CLI 군 1)
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
import { FEED_TOAST_APPROVAL_TITLE, feedCreatedToastTitle } from "./feedclass";
import { bodyPrint, fingerprint, findAnchor, normLine } from "./siteprint";

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
    feedCreatedToastTitle, // 실물(feedclass.ts) — 배너 제목을 스텁이 아니라 실제 분류로 단언한다
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
  // ★MINOR-2(codex R1): 예전에는 여기서 `const stickyToasts = new Map();` 을 한 번 더 선언해 store.stickyToasts 를 가렸고, 반환한 맵은 아무도 쓰지 않는 것이라
  //   「sticky 맵 0」 단언이 sticky 토스트가 떠 있어도 초록이었다. 이제 선언하지 않는다 — 추출된 함수의 `stickyToasts` 는 프록시를 거쳐 store 의 **그 맵**에 닿는다.
  const src =
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
/** 부서장 판정 R5: approval.stalled 세 표면(박스·OS 배너·Feed 전환)은 보류 = 복원이라 quiet 에서도 뜬다(아래 남김 단언). */
const KEPT_IDS = ["approval.stalled"];
const SIDE_EFFECTS = ["refreshSidebarStatus", "refreshFeed", "upsertTaskCell", "removeDeadPane"];
/** OS 배너 한 면이 거치는 호출 이름(sendNotification 과 그 권한 확인) — `--wait` 대기 항목(waitBannerEvent)의 feed.item.created 에서만 남는다(R2 · AT-20 이 정확한 호출 수를 단언). */
const BANNER_CALLS = ["sendNotification", "isPermissionGranted", "requestPermission"];
const SINK_CALLS = ["sendNotification", "isPermissionGranted", "requestPermission", "pushAlarm", "openFeed", "scheduleFeedSwitchIfStillPending"];
/** 배너가 남는 유일한 모집단 술어 — 기계 라우트가 없는 `--wait` 대기 항목. T-1 의 예외도, AT-20 의 기대값도 이 한 함수에서 나온다(두 곳이 따로 늙지 않게). */
function waitBannerEvent(ev: Record<string, unknown>): boolean {
  const p = (ev.payload ?? {}) as Record<string, unknown>;
  return ev.name === "feed.item.created" && p.wait === true && p.auto_route !== true;
}

function sideEffectList(sb: Sandbox): string[] {
  const out: string[] = [];
  for (const n of SIDE_EFFECTS) for (let i = 0; i < (sb.calls[n]?.length ?? 0); i++) out.push(n);
  return out.sort();
}

describe("TOAST 축 T-1 — 데몬 이벤트가 밀어 올린 사람 싱크 0", () => {
  for (const c of EVENTS.filter((e) => !KEPT_IDS.includes(e.id))) {
    test(`quiet: ${c.id} → 박스 0 · OS 배너 0 · 알람 이력 0 · Feed 전환 0`, async () => {
      const sb = build({ quiet: true });
      sb.fn.onDaemonEvent(c.ev);
      await flush();
      expect(sb.toasts.children.length).toBe(0);
      // R2: `--wait` 대기 항목(waitBannerEvent)의 OS 배너 한 면만 남는다(AT-20 이 호출 수와 문면을 단언). 그 밖의 모든 이벤트·싱크는 0 — approval.request 도 0(R6 의 직접 배너는 되돌렸다).
      for (const n of SINK_CALLS.filter((x) => !(waitBannerEvent(c.ev) && BANNER_CALLS.includes(x)))) expect([n, sb.calls[n]?.length ?? 0]).toEqual([n, 0]);
      expect(sb.stickyMap.size).toBe(0); // MINOR-2: 이제 실제 레지스트리를 본다 — 양성 대조는 아래 「sticky 레지스트리」 시험
    });
  }

  test("category × name 퍼즈 — 어떤 조합도 박스·배너·알람 이력·Feed 전환을 만들지 않는다", async () => {
    const cats = ["", "health", "watchdog", "feed", "info", "alert", "approval", "context", "osc", "unknown-x"];
    const names = ["", "x", "approval.request", "context.threshold", "pane.idle", "master.idle", "agent.exited", "master.deadman", "queue.starved", "queue.delivered", "status.changed", "task.changed", "osc.notify", "feed.item.created", "watchdog.duplicate_procs", "surface.exited", "surface.closed", "surface.reaped", "health.stuck"];
    let n = 0;
    for (const category of cats)
      for (const name of names) {
        const sb = build({ quiet: true });
        sb.fn.onDaemonEvent({ name, category, surface_id: 3, payload: { kind: "k", title: "t", context_pct: 95 } });
        await flush();
        expect(`${category}/${name}:${sb.toasts.children.length}`).toBe(`${category}/${name}:0`);
        for (const s of SINK_CALLS) expect(`${category}/${name}:${s}:${sb.calls[s]?.length ?? 0}`).toBe(`${category}/${name}:${s}:0`); // 퍼즈 payload 에는 wait 가 없다 — 배너 예외도 없다(R2)
        n++;
      }
    expect(n).toBe(cats.length * names.length);
  });

  // ---- 양성 대조: 시험이 '보이는 것'을 실제로 볼 수 있다는 증명 ----
  test("양성 대조 A — 가드를 푼 상태(quiet=false)에서는 같은 이벤트가 박스·배너·알람 이력·Feed 전환을 만든다", async () => {
    const sb = build({ quiet: false });
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "approval.request")!.ev);
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "master.idle")!.ev);
    await flush();
    expect(sb.toasts.children.length).toBeGreaterThanOrEqual(2);
    expect(sb.calls.sendNotification?.length ?? 0).toBeGreaterThanOrEqual(1);
    expect(sb.calls.pushAlarm?.length ?? 0).toBeGreaterThanOrEqual(2);
  });

  // ---- 남김 단언 3건(T-TOAST-RESTORE-STALLED · 부서장 판정 R5): approval.stalled 의 세 표면은 quiet=true 에서도 실제로 뜬다 ----
  const STALLED = EVENTS.find((e) => e.id === "approval.stalled")!.ev;
  test("남김 1/3 — approval.stalled 박스(toast)가 quiet=true 에서 그려지고 알람 이력에 남는다", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(STALLED);
    await flush();
    expect(sb.toasts.children.length).toBe(1);
    expect(sb.calls.pushAlarm?.length ?? 0).toBe(1);
  });
  test("남김 2/3 — approval.stalled OS 배너(osBanner)가 quiet=true 에서 나간다", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(STALLED);
    await flush();
    expect(sb.calls.sendNotification?.length ?? 0).toBe(1);
  });
  test("남김 3/3 — approval.stalled Feed 화면 전환(openFeed)이 quiet=true 에서 1회 불린다", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(STALLED);
    await flush();
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
    expect(sb.stickyMap.size).toBe(2); // MINOR-2: 추출된 stickyToast 가 쓰는 **그 레지스트리**에 두 id 가 실제로 선다(0 에 고정된 헛맵이 아니다)
  });

  test("sticky 레지스트리(MINOR-2) — master.idle: quiet 면 레지스트리 0·박스 0·알람 0 / 가드를 풀면 레지스트리 1·박스 1·알람 1(양성 대조: 이 맵 단언이 0 이 아닌 값을 볼 수 있다)", async () => {
    const ev = EVENTS.find((e) => e.id === "master.idle")!.ev;
    const q = build({ quiet: true });
    q.fn.onDaemonEvent(ev);
    await flush();
    expect([q.stickyMap.size, q.toasts.children.length, q.calls.pushAlarm?.length ?? 0]).toEqual([0, 0, 0]);
    const l = build({ quiet: false });
    l.fn.onDaemonEvent(ev);
    await flush();
    expect([l.stickyMap.size, l.toasts.children.length, l.calls.pushAlarm?.length ?? 0]).toEqual([1, 1, 1]);
    expect([...l.stickyMap.keys()]).toEqual(["master-idle:base:master"]); // 데몬 이벤트가 만든 id — role별 안정 id(데몬 출처 sticky 는 이 하나뿐)
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

describe("TOAST 축 R2 — `--wait` 대기 항목의 생성 시점 배너 1면 (AT-19 근거 · AT-20 producer→consumer)", () => {
  // 배경(codex R1 MAJOR-1 · 부서장 결정 2026-10-15T21:26 ④): 이전 판(R6)은 `approval.request`(pane 화면 패턴이 올린 승인)에 배너를 붙였다. 그러나 `cys feed push --wait` 요청자의
  //   대기 항목은 **다른 이벤트**(feed.item.created)로 오고, stalled 는 그 항목을 세지 않는다 — 측정한 모집단과 구제한 모집단이 달랐다(범주 오류). AT-19 가 그 두 사실을 소스에서 읽고,
  //   AT-20 이 producer 가 실제로 내는 모양을 소비자까지 흘린다.
  const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");
  const CYS_RS = read("../../src/bin/cys.rs");
  const GOV_RS = read("../../src/bin/cysd/governance.rs");
  const HDL_RS = read("../../src/bin/cysd/handlers.rs");
  const STATE_RS = read("../../src/bin/cysd/state.rs");
  /** 히트를 파일 전체에서 센다 — 시험 모듈에 숨은 같은 모양의 문장은 **개수를 바꿔** 적색이 된다(정확한 개수 단언 · fail-closed). */
  const hits = (src: string, re: RegExp): RegExpMatchArray[] => [...src.matchAll(re)];

  test("AT-19 ⑴ 요청자 — `cys feed push --wait` 의 기본 대기 시간은 소스에서 120초다 (히트 정확히 1)", () => {
    const h = hits(CYS_RS, /wait: bool,\s*\n\s*#\[arg\(long, default_value_t = (\d+)\)\]\s*\n\s*timeout_secs: u64,/g);
    expect(h.length).toBe(1);
    const why = `요청자 기본이 ${h[0]?.[1]} 로 바뀌었다. 이 배너의 근거는 120 이라는 수가 아니라 「--wait 항목에 stalled 가 닿지 않는다」(⑵⑶)다 — 그 둘이 아직 참인지 확인하고 이 수를 갱신하라(TOAST R2).`;
    expect([why, Number(h[0][1])]).toEqual([why, 120]);
  });

  test("AT-19 ⑵ stalled — 방치 스캔은 `kind == approval ∧ 데몬 발행 id` 항목만 센다: 일반 feed 항목은 대상이 아니다", () => {
    // 대기 집합과 방치 집합, 두 필터가 같은 세 조건(pending ∧ kind==approval ∧ is_daemon_issued)을 쓴다 → 2. 방치를 내는 쪽만 거기에 나이 조건이 붙는다 → 1.
    const three = hits(GOV_RS, /i\.status == "pending"\s*&&\s*i\.kind == "approval"\s*&&\s*crate::state::is_daemon_issued\(&i\.request_id\)/g);
    const aged = hits(GOV_RS, /i\.status == "pending"\s*&&\s*i\.kind == "approval"\s*&&\s*crate::state::is_daemon_issued\(&i\.request_id\)\s*&&\s*now - i\.created_at >= stall as f64/g);
    expect([three.length, aged.length]).toEqual([2, 1]);
    // 그 스캔이 approval.stalled 를 내는 유일한 생산 자리다(이벤트 이름 → 스캔 연결).
    expect(hits(GOV_RS, /daemon\.bus\.publish\(\s*"approval\.stalled",/g).length).toBe(1);
  });

  test("AT-19 ⑶ 일반 id — feed.push 는 `req-…` id 를 만들고 `daemon-` 접두 지정을 거부한다 · is_daemon_issued 는 그 접두 판정이다", () => {
    expect(hits(HDL_RS, /format!\(\s*"req-\{\}-\{\}-\{\}",/g).length).toBe(1);
    expect(
      hits(HDL_RS, /if crate::state::is_daemon_issued\(&request_id\) \{\s*return Reply::Single\(err_response\(\s*&id,\s*"invalid_params",\s*"request_id prefix 'daemon-' is reserved for daemon-issued items",/g).length,
    ).toBe(1);
    expect(hits(STATE_RS, /pub const DAEMON_REQ_PREFIX: &str = "daemon-";/g).length).toBe(1);
    expect(hits(STATE_RS, /pub fn is_daemon_issued\(request_id: &str\) -> bool \{\s*request_id\.starts_with\(DAEMON_REQ_PREFIX\)\s*\}/g).length).toBe(1);
    // ⑵⑶ ⇒ 일반 `feed push --wait` 항목(id `req-…`)은 stalled 대상이 아니다 → 생성 시점 알림이 그 항목이 사람에게 닿는 **유일한** 경로다.
  });

  // ---- AT-20: producer 가 실제로 내는 모양을 소비자(onDaemonEvent)까지 ----
  const PUB = hits(HDL_RS, /daemon\.bus\.publish\(\s*"feed\.item\.created",\s*"(\w+)",\s*surface_id,\s*json!\(\{([\s\S]*?)\}\),\s*\);/g);
  const PUBLISHED_KEYS: string[] = PUB.length ? [...PUB[0][2].matchAll(/"(\w+)":/g)].map((m) => m[1]).sort() : [];
  const PUB_CATEGORY = PUB[0]?.[1] ?? "";
  test("AT-20 전제 — producer 의 feed.item.created 발행은 소스에 정확히 한 자리이고 category 는 `feed`, 페이로드 키 여덟이 `wait`·`auto_route` 를 싣는다", () => {
    expect(PUB.length).toBe(1);
    expect(PUB[0][1]).toBe("feed");
    expect(PUBLISHED_KEYS).toEqual(["auto_route", "body", "kind", "request_id", "risk_class", "tier", "title", "wait"]);
  });

  /** producer 모양 그대로의 이벤트 — 키 집합이 handlers.rs 의 json! 과 다르면(키 추가·개명) 여기서 적색이라, 시험이 소비자만 보는 합성 입력으로 초록이 되지 않는다. */
  function published(over: Record<string, unknown> = {}, drop: string[] = []): Record<string, unknown> {
    const payload: Record<string, unknown> = {
      request_id: "req-4242-1760000000-0", // 일반 id: feed.push 가 만드는 `req-<pid>-<epoch>-<n>` (AT-19 ⑶)
      kind: "approval",
      title: "Bash 실행을 허용할까요?",
      body: "rm -rf ./tmp",
      wait: true,
      tier: "d",
      risk_class: "high",
      auto_route: false,
      ...over,
    };
    for (const k of drop) delete payload[k];
    expect(Object.keys(payload).sort()).toEqual(PUBLISHED_KEYS.filter((k) => !drop.includes(k)));
    expect(/^req-\d+-\d+-\d+$/.test(String(payload.request_id)) && !String(payload.request_id).startsWith("daemon-")).toBe(true);
    return { name: "feed.item.created", category: PUB_CATEGORY, surface_id: 3, socket_slug: "dept-5", payload };
  }

  test("AT-20 — producer 모양(wait=true · auto_route=false · 일반 id)의 feed.item.created: OS 배너 정확히 1 · 제목=승인 요청 분류 · 본문=항목 제목 · 같은 런에서 토스트 0·알람 0·Feed 전환(예약 포함) 0", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(published());
    await flush();
    const sent = sb.calls.sendNotification ?? [];
    expect(sent.length).toBe(1);
    expect(sent[0][0]).toEqual({ title: FEED_TOAST_APPROVAL_TITLE, body: "Bash 실행을 허용할까요?" });
    expect(sb.toasts.children.length).toBe(0); // 토스트는 꺼진 채(오너가 지목한 우하단 박스)
    expect(sb.calls.pushAlarm?.length ?? 0).toBe(0);
    expect(sb.calls.openFeed?.length ?? 0).toBe(0);
    expect(sb.calls.scheduleFeedSwitchIfStillPending?.length ?? 0).toBe(0); // 자동 Feed 전환 예약도 꺼진 채
    expect(sb.stickyMap.size).toBe(0);
    expect(sideEffectList(sb)).toEqual(["refreshFeed", "refreshSidebarStatus"]); // 비표시 부작용은 그대로(AT-10)
  });

  test("AT-20 양성 대조 1 — auto_route=true 항목(기계 라우트가 있는 쪽)은 같은 런에서 배너 0 · 토스트 0 · 전환 0", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(published({ auto_route: true }));
    await flush();
    expect([sb.calls.sendNotification?.length ?? 0, sb.toasts.children.length, sb.calls.scheduleFeedSwitchIfStillPending?.length ?? 0, sb.calls.openFeed?.length ?? 0]).toEqual([0, 0, 0, 0]);
  });

  test("AT-20 양성 대조 2 — approval.request(R6 에서 배너를 붙였던 이벤트)는 되돌렸으므로 배너 0 · 토스트 0", async () => {
    const sb = build({ quiet: true });
    sb.fn.onDaemonEvent(EVENTS.find((e) => e.id === "approval.request")!.ev);
    await flush();
    expect([sb.calls.sendNotification?.length ?? 0, sb.calls.isPermissionGranted?.length ?? 0, sb.toasts.children.length, sb.calls.pushAlarm?.length ?? 0]).toEqual([0, 0, 0, 0]);
  });

  test("AT-20 판별 행렬 — wait × auto_route × kind 전 조합: 배너 수 == (wait===true ∧ auto_route!==true ? 1 : 0), 토스트·전환·알람은 언제나 0", async () => {
    let n = 0;
    for (const wait of [true, false])
      for (const auto_route of [true, false])
        for (const kind of ["approval", "team-create-request", "formation-failed", "ceo-notice"]) {
          const ev = published({ wait, auto_route, kind });
          const sb = build({ quiet: true });
          sb.fn.onDaemonEvent(ev);
          await flush();
          const row = `wait=${wait} auto_route=${auto_route} kind=${kind}`;
          expect([row, sb.calls.sendNotification?.length ?? 0]).toEqual([row, waitBannerEvent(ev) ? 1 : 0]);
          expect([row, sb.toasts.children.length, sb.calls.pushAlarm?.length ?? 0, sb.calls.openFeed?.length ?? 0, sb.calls.scheduleFeedSwitchIfStillPending?.length ?? 0]).toEqual([row, 0, 0, 0, 0]);
          n++;
        }
    expect(n).toBe(16);
  });

  test("AT-20 — auto_route 키가 없는 이전 데몬의 페이로드는 auto_route 아님으로 읽어 배너 1 / 알림 종류(ℹ 알림)도 같은 술어를 따른다(종류로 거르지 않는다)", async () => {
    const old = build({ quiet: true });
    old.fn.onDaemonEvent(published({}, ["auto_route"]));
    await flush();
    expect(old.calls.sendNotification?.length ?? 0).toBe(1);
    const notice = build({ quiet: true });
    notice.fn.onDaemonEvent(published({ kind: "formation-failed", title: "부서 팀 편성 실패" }));
    await flush();
    expect(notice.calls.sendNotification?.[0]?.[0]).toEqual({ title: feedCreatedToastTitle("formation-failed"), body: "부서 팀 편성 실패" });
  });

  test("AT-20 대조(보이는 쪽) — 가드를 푼 같은 이벤트는 토스트 1 · 알람 1 · 전환 예약 1 · 배너 1 이 모두 선다: 시험이 '보이는 것'을 실제로 볼 수 있다", async () => {
    const sb = build({ quiet: false });
    sb.fn.onDaemonEvent(published());
    await flush();
    expect([sb.toasts.children.length, sb.calls.pushAlarm?.length ?? 0, sb.calls.scheduleFeedSwitchIfStillPending?.length ?? 0, sb.calls.sendNotification?.length ?? 0]).toEqual([1, 1, 1, 1]);
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

// ---------------------------------------------------------------------------------------------------------------------------
// 남긴 자리의 **필터된 소스 줄 표류 탐지**(44 적대 리뷰 major1 · 부서장 판정 R3·R5 조건; codex R1 minor 1 로 보장 범위를 좁혔다).
//   · 지문 = 그 문장 줄 + 조상 opener 줄 + 같은 들여쓰기(또는 조상) 수준에서 앞서는 제어 키워드/alertsQuiet/category 줄의 해시(siteprint.ts). 핀은 같은 알고리즘으로 계산해 박아 둔다.
//     early return · category 비교 가드 · 삼항 · && · 래핑 블록 · 같은 줄 접두 가드가 **그 줄들 중 하나로 나타나면** 적색이다.
//   · ⛔이 시험은 도달 가능성(실행 흐름)을 증명하지 않는다: 더 깊이 들여쓴 앞선 블록 속에 끼워 넣은 `return;` 은 지문이 건너뛰어 **초록으로 남는다**(아래 「알려진 한계」 시험이 그 사각을 못박는다).
//     실행 흐름 자체는 위 이벤트·싱크 실행 시험(T-1·AT-20 등)이 지킨다.
//   · 의도된 변경으로 지문이 바뀌면 실패 메시지가 실제 값을 찍는다 — 이 표의 pin 을 다시 계산해 넣는다(변경 이유를 커밋에 적는다).
//   · kind: daemon-info(상단바 오류 표기) · update(업데이트·팩 적용 고지) · K6(부트·설치본·온보딩 고지) · R2(`--wait` 대기 항목 배너 — 핀은 R2 수리 시점에서 계산, 나머지는 4bad39ca)
interface KeptSite {
  id: string;
  kind: "daemon-info" | "update" | "K6" | "R2";
  anchor: string;
  nth: number;
  pin: string;
}
const KEPT_SITES: KeptSite[] = [
  { id: "feed-wait-banner", kind: "R2", anchor: "if (payload.wait === true && payload.auto_route !== true) osBanner(feedCreatedToastTitle(payload.kind), String(payload.title ?? \"\"));", nth: 0, pin: "095ff275a023d511" },
  { id: "K5-daemon-error", kind: "daemon-info", anchor: "info.textContent = `daemon error: ${e.payload}`;", nth: 0, pin: "1fc56231bf45df6e" },
  { id: "K5-startup-failed", kind: "daemon-info", anchor: "document.getElementById(\"daemon-info\")!.textContent = `startup failed: ${e}`;", nth: 0, pin: "607b4ac9163cc886" },
  { id: "K5-recovery-failed", kind: "daemon-info", anchor: "document.getElementById(\"daemon-info\")!.textContent = `startup failed: ${e} / recovery failed: ${e2}`;", nth: 0, pin: "bd517611b51bc743" },
  { id: "info-verskew", kind: "daemon-info", anchor: "info.appendChild(verSkewBadge);", nth: 0, pin: "f1f0e053fc42afc9" },
  { id: "info-blocked", kind: "daemon-info", anchor: "info.textContent = daemonActionBlockedMsg();", nth: 0, pin: "c8b805cf81306f13" },
  { id: "info-pid", kind: "daemon-info", anchor: "info.textContent = `daemon pid=${status.daemon_pid} sock=${status.socket_path}`;", nth: 0, pin: "a16c137737a92fb7" },
  { id: "info-noresp", kind: "daemon-info", anchor: "info.textContent = \"데몬 응답 없음 — 화면은 계속 사용할 수 있습니다(연결되면 자동으로 붙습니다)\";", nth: 0, pin: "f9baef97e329c860" },
  { id: "upd-pack-start", kind: "update", anchor: "stickyToast(\"upd-pack\", \"feed\", \"↻ 무중단 팩 업데이트\", `팩 ${pv} 적용 중… 세션·데몬 유지(재시작 없음).`);", nth: 0, pin: "8ce6fd934d60bf6e" },
  { id: "upd-pack-fail-toast", kind: "update", anchor: "toast(\"health\", \"팩 업데이트 실패\", String(e));", nth: 0, pin: "d0cd2fbfa976eb51" },
  { id: "silent-update-toast", kind: "update", anchor: "if (t) toast(\"feed\", t.title, t.msg);", nth: 0, pin: "9c92b98f8b000ab0" },
  { id: "safe-mode-start", kind: "K6", anchor: "stickyToast(\"safe-mode\", \"health\", \"안전모드 — 설치 위치를 옮겨 주세요\", guidance);", nth: 0, pin: "a408a3d221e3cde7" },
  { id: "bundle-damaged-start", kind: "K6", anchor: "stickyToast(\"bundle-damaged\", \"health\", \"설치본이 온전하지 않습니다 — 재설치 필요\", damage);", nth: 0, pin: "8179f4e7bca32541" },
  { id: "claude-missing", kind: "K6", anchor: "stickyToast(\"claude-missing\", \"health\", \"Claude Code CLI가 없습니다\", hint);", nth: 0, pin: "cd63b1333f50d07d" },
  { id: "upd-bin-1", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 업데이트 설치\", \"다운로드 시작…\");", nth: 0, pin: "1ab88556f3ff9cd5" },
  { id: "upd-bin-2", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 업데이트 설치\", `다운로드 중 ${mb(updDownloaded)} / ${mb(p.total)} MB (${pct}%)`);", nth: 0, pin: "c47d425a280ecee0" },
  { id: "upd-bin-3", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 업데이트 설치\", `다운로드 중 ${mb(updDownloaded)} MB`);", nth: 0, pin: "410d4fb4a3da9819" },
  { id: "upd-bin-4", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 업데이트 설치\", \"세션 정리 중…\");", nth: 0, pin: "a5be5464d81aeea3" },
  { id: "upd-bin-5", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 업데이트 설치\", \"재시작 준비 중…\");", nth: 0, pin: "afdcd1969e2f7e93" },
  { id: "upd-pack-apply", kind: "update", anchor: "stickyToast(\"upd-pack\", \"feed\", \"🔄 무중단 적용 중\", \"서명검증 → 다운로드 → 원자적 팩 교체 → 노드 reinject…\");", nth: 0, pin: "c6179312c9d356cd" },
  { id: "pack-already", kind: "update", anchor: "toast(\"watchdog\", \"ℹ 이미 적용돼 있음\", `팩 ${p.disk_version ?? \"\"} — 반영할 것이 없습니다(이미 최신).`);", nth: 0, pin: "98d9acbe744e0208" },
  { id: "pack-partial", kind: "update", anchor: "toast(\"health\", \"⚠ 팩 일부 미각성\", p.message ?? \"디스크 팩은 갱신됐으나 일부 노드 reinject 보류/실패(라이브 유지).\");", nth: 0, pin: "3facd62da87e228a" },
  { id: "update-warn", kind: "update", anchor: "toast(\"health\", \"업데이트 경고\", msg);", nth: 0, pin: "30467694c59a4023" },
  { id: "boot-warn", kind: "K6", anchor: "stickyToast(\"boot-warn\", \"health\", \"팀 기동 경고\", msg);", nth: 0, pin: "567bfe96cc30d932" },
  { id: "boot-info", kind: "K6", anchor: "stickyToast(\"boot-info\", \"health\", \"팀 기동 안내\", msg);", nth: 0, pin: "63af881eb2cefa72" },
  { id: "boot-degrade", kind: "K6", anchor: "stickyToast(\"boot-degrade\", \"health\", \"팀 부트 경로 강등\", msg);", nth: 0, pin: "a0b21799994a3004" },
  { id: "safe-mode-evt", kind: "K6", anchor: "stickyToast(\"safe-mode\", \"health\", \"안전모드 — 설치 위치를 옮겨 주세요\", msg);", nth: 0, pin: "5ab03be213acbc0a" },
  { id: "bundle-damaged-evt", kind: "K6", anchor: "stickyToast(\"bundle-damaged\", \"health\", \"설치본이 온전하지 않습니다 — 재설치 필요\", msg);", nth: 0, pin: "a2652bf6c11d9a8c" },
  { id: "bundle-damaged-pull", kind: "K6", anchor: "stickyToast(\"bundle-damaged\", \"health\", \"설치본이 온전하지 않습니다 — 재설치 필요\", damage);", nth: 1, pin: "5a4ac8e7839c0e6f" },
  { id: "onboard-capped", kind: "K6", anchor: "if (kind === \"capped\") stickyToast(\"onboard-capped\", \"health\", \"Claude 설정 파일을 고쳐 주세요\", message);", nth: 0, pin: "ccaa6290cb9cd4c4" },
  { id: "onboard-restored", kind: "K6", anchor: "else if (kind === \"restored\") stickyToast(\"onboard-restored\", \"health\", \"Claude 연결 설정을 다시 넣었습니다\", message);", nth: 0, pin: "2c57cd9493198257" },
  { id: "onboard-devtools", kind: "K6", anchor: "else if (kind === \"devtools-missing\") stickyToast(\"onboard-devtools\", \"health\", \"개발자 도구 없이 동작 중입니다\", message);", nth: 0, pin: "d7780395e2c8965d" },
  { id: "upd-bin-patch", kind: "update", anchor: "stickyToast(\"upd-bin\", \"feed\", \"⬇ 패치 설치(자동 테스트)\", \"패치 업데이트 확인·설치 중…\");", nth: 0, pin: "f7cbba739b0fb1fa" },
  { id: "update-failed", kind: "update", anchor: "if (plan.kind === \"notice\") stickyToast(UPDATE_FAILED_TOAST_ID, \"health\", plan.title, plan.body);", nth: 0, pin: "995ec2c77d9286ab" },
];
const SINK_BODY_PINS: Record<string, string> = {
  toast: "0112c5af32addf86",
  stickyToast: "08c33dfc6cb3ae9f",
  osBanner: "0c60fbe8ba648c4a",
  recordAlarm: "fbca511a1e48fe1a",
};


const MAIN_LINES = SRC.split("\n");

describe("TOAST 축 — 남긴 자리 지문 (K5·K6·daemon-info 전수 · 업데이트 고지)", () => {
  for (const site of KEPT_SITES) {
    test(`남김 [${site.kind}] ${site.id} — 그 문장의 필터된 소스 줄(opener 사슬·제어 흐름 줄)이 핀과 같다 (표류 탐지 · 도달 가능성 증명 아님)`, () => {
      const i = findAnchor(MAIN_LINES, site.anchor, site.nth);
      expect([site.id, "앵커 존재", i >= 0]).toEqual([site.id, "앵커 존재", true]);
      const got = fingerprint(MAIN_LINES, i);
      expect([site.id, got]).toEqual([site.id, site.pin]);
    });
  }

  test("열거 완전성 — 남긴 id(업데이트·안전모드·설치본·부트·온보딩·CLI 없음)를 직접 부르는 줄은 전부 표에 있다(표에 없는 새 자리는 적색)", () => {
    const ids = /["'](upd-pack|upd-bin|safe-mode|bundle-damaged|claude-missing|boot-warn|boot-info|boot-degrade|onboard-capped|onboard-restored|onboard-devtools)["']|UPDATE_FAILED_TOAST_ID,/;
    const have = new Set(KEPT_SITES.map((k) => k.anchor));
    const calls = MAIN_LINES.map(normLine).filter((l) => ids.test(l) && /(^|[^A-Za-z])stickyToast\(/.test(l));
    expect(calls.length).toBeGreaterThanOrEqual(20);
    for (const l of calls) expect([l.slice(0, 80), have.has(l)]).toEqual([l.slice(0, 80), true]);
    for (const l of calls) expect(/daemonStickyToast\(/.test(l)).toBe(false);
  });

  test("싱크 본문 전체 지문 — toast/stickyToast/osBanner/recordAlarm 의 본문(주석·빈 줄 제외)이 핀과 같다: 조기 반환·가드가 어떤 모양으로 들어와도 본문 줄이 바뀌므로 적색(본문 표류 탐지)", () => {
    for (const n of Object.keys(SINK_BODY_PINS)) expect([n, bodyPrint(extractFn(n))]).toEqual([n, SINK_BODY_PINS[n]]);
  });

  test("지문의 변별력 — category 비교 조기 반환·래핑 블록·같은 줄 접두 가드가 **그 줄들 중 하나로 나타나면** 지문이 바뀐다(알고리즘이 보는 범위 안의 변별력)", () => {
    const base = ["function f() {", "  if (a) {", "    keep();", "  }", "  stickyToast(\"x\");", "}"];
    const fp0 = fingerprint(base, 4);
    const early = ["function f() {", '  if (event.category === "watchdog") return;', "  if (a) {", "    keep();", "  }", "  stickyToast(\"x\");", "}"];
    const wrapped = ["function f() {", "  if (a) {", "    keep();", "  }", "  if (!quiet) {", "    stickyToast(\"x\");", "  }", "}"];
    const prefixed = ["function f() {", "  if (a) {", "    keep();", "  }", "  if (!quiet) stickyToast(\"x\");", "}"];
    expect(fingerprint(early, 5)).not.toBe(fp0);
    expect(fingerprint(wrapped, 5)).not.toBe(fp0);
    expect(fingerprint(prefixed, 4)).not.toBe(fp0);
    // 무관한 줄(주석·빈 줄·무관 문장)은 지문을 바꾸지 않는다 — 평상시 편집이 거짓 적색을 만들지 않는다.
    const benign = ["function f() {", "  // 주석", "", "  other();", "  if (a) {", "    keep();", "  }", "  stickyToast(\"x\"); // 꼬리 주석", "}"];
    expect(fingerprint(benign, 7)).toBe(fp0);
  });

  test("알려진 한계(codex R1 minor 1) — 더 깊이 들여쓴 앞선 블록 속에 끼워 넣은 `return;` 은 뒤 자리의 지문을 바꾸지 못한다: 이 지문은 도달 가능성을 증명하지 않는다", () => {
    // 모형: 안전모드 토스트(앞선 try 안 if 블록) 직후에 `return;` 을 끼우면 뒤의 try 속 자리는 실행되지 못하지만, 그 return 은 뒤 자리보다 깊이 들여쓴 **다른 블록의 속**이라 지문 재료에서 건너뛴다.
    const base = ["function f() {", "  try {", "    if (g) {", "      first();", "    }", "  } catch {}", "  try {", "    later();", "  } catch {}", "}"];
    const mutated = ["function f() {", "  try {", "    if (g) {", "      first();", "      return;", "    }", "  } catch {}", "  try {", "    later();", "  } catch {}", "}"];
    expect(fingerprint(mutated, 8)).toBe(fingerprint(base, 7)); // 같다 = 사각. 이 단언이 적색이 되면 지문이 넓어진 것이니 위 주석과 siteprint.ts 머리말을 함께 갱신하라.
    // 같은 return 이 **그 자리 자신의 블록**(같은 들여쓰기 수준의 앞 줄)에 있으면 지문이 바뀐다 — 범위 안/밖의 경계.
    const sameLevel = ["function f() {", "  try {", "    if (g) {", "      first();", "    }", "  } catch {}", "  try {", "    return;", "    later();", "  } catch {}", "}"];
    expect(fingerprint(sameLevel, 8)).not.toBe(fingerprint(base, 7));
  });
});

describe("TOAST 축 — 출처 핀 · 통짜 가드 금지 · 커버리지", () => {
  const body = stripComments(extractFn("onDaemonEvent"));

  test("onDaemonEvent 본문의 직접 호출은 넷뿐이다 — approval.stalled 의 toast/osBanner/openFeed 셋 + feed.item.created 의 osBanner 하나 (approval.request 는 0 · 그 밖은 래퍼만)", () => {
    const rq = body.indexOf('name === "approval.request"');
    const st = body.indexOf('name === "approval.stalled"');
    const en = body.indexOf('name === "context.threshold"');
    const fi = body.indexOf('name === "feed.item.created"');
    const fe = body.indexOf('name === "surface.exited"');
    for (const [k, v] of Object.entries({ rq, st, en, fi, fe })) expect([k, v >= 0]).toEqual([k, true]);
    expect(rq).toBeLessThan(st);
    expect(st).toBeLessThan(en);
    expect(en).toBeLessThan(fi);
    expect(fi).toBeLessThan(fe);
    expect(body.indexOf('name === "feed.item.created"', fi + 1)).toBe(-1); // 분기는 한 자리
    const request = body.slice(rq, st);
    const stalled = body.slice(st, en);
    const feed = body.slice(fi, fe);
    const rest = body.slice(0, rq) + body.slice(en, fi) + body.slice(fe);
    const direct = (src: string, n: string) => (src.match(new RegExp(`(?<![A-Za-z0-9_.])${n}\\(`, "g")) ?? []).length;
    for (const n of ["toast", "stickyToast", "osBanner", "openFeed"]) {
      expect([n, "rest", direct(rest, n)]).toEqual([n, "rest", 0]);
      expect([n, "stalled", direct(stalled, n)]).toEqual([n, "stalled", n === "stickyToast" ? 0 : 1]);
      // R2: approval.request 는 직접 호출 0 — R6 의 배너 복원은 범주 오류라 되돌렸다(래퍼 경유 = 꺼짐)
      expect([n, "request", direct(request, n)]).toEqual([n, "request", 0]);
      // R2: feed.item.created 분기는 osBanner 한 면만 직접 호출(토스트·배지·유예 전환은 꺼진 채 — 최소성)
      expect([n, "feed", direct(feed, n)]).toEqual([n, "feed", n === "osBanner" ? 1 : 0]);
    }
    // 그 한 면의 술어는 정확히 이 문장이다(wait===true ∧ auto_route!==true) — AT-20 판별 행렬이 동작으로, 이 줄이 소스로 못박는다.
    expect(feed).toContain("if (payload.wait === true && payload.auto_route !== true) osBanner(");
    expect(SRC).not.toContain("function daemonOpenFeed("); // 호출자 0 이 된 죽은 래퍼는 남기지 않는다(실효 0 장치)
    expect(SRC).not.toContain("daemonOpenFeed(");
    expect((body.match(/daemonToast\(/g) ?? []).length).toBe(10);
    expect((body.match(/daemonStickyToast\(/g) ?? []).length).toBe(1);
    expect((body.match(/daemonOsBanner\(/g) ?? []).length).toBe(4); // approval.request 의 배너가 래퍼로 돌아와 호출자 4(context.threshold · agent.exited · master.deadman · approval.request)
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

  test("feed 스케줄러 배선은 openFeed 그대로(가드는 호출부 인라인) / 팔레트 수동 열기도 openFeed 그대로", () => {
    expect(/^\s*open: openFeed,/m.test(SRC)).toBe(true); // 스케줄러 배선은 그대로(probefail 핀) — 가드는 호출부 daemonScheduleFeedSwitch
    expect(/^\s*open: daemonOpenFeed,/m.test(SRC)).toBe(false);
  });

  test("래퍼 4개는 첫 문장이 alertsQuiet 가드다", () => {
    const want: Record<string, string> = { daemonToast: "toast", daemonStickyToast: "stickyToast", daemonOsBanner: "osBanner" };
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
