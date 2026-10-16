// S6: actual close consumers and marker races, with no live UI or daemon.
// Execute production bodies/callbacks, not a second close algorithm. Raw RPC
// envelopes are tested in the Rust draft; Tauri normalizes accepted close to void.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  destinationCanReceive, destinationGone, handoffAckPath, handoffInstruction,
  originCloseVerdict, parseEnqueueReceipt, pickLaunchedAgentSid, transferRetryAction,
} from "./transfer";
import { deptCloseAfterStop, deptLaunchName, resolveDeptLaunchName } from "./deptlabel";
import { placeSeatSafe } from "./seatlayout";

declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(s: string): string } };
// Explicit dynamic scope matches deptprogresswiring.test.ts's actual-source seam.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Any = any;
const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const DENIED = "close_denied: S6 scripted refusal";
const SOURCE = "/fixture/source/cys.sock";
const DEST = "/fixture/destination/cys.sock";
const key = (sid: number, socket?: string) => `${socket ?? ""}#${sid}`;

function fnText(name: string): string {
  const m = new RegExp(`^(?:async )?function ${name}\\(`, "m").exec(SRC);
  if (!m) throw new Error(`SETUP: production function ${name} is absent; not a semantic red`);
  const end = SRC.indexOf("\n}\n", m.index);
  if (end < 0) throw new Error(`SETUP: function end missing for ${name}`);
  return SRC.slice(m.index, end + 2);
}

function declaration(name: string): string {
  const m = new RegExp(`^(?:const|let) ${name}\\b`, "m").exec(SRC);
  if (!m) throw new Error(`SETUP: production declaration ${name} absent`);
  const end = SRC.indexOf(";\n", m.index);
  if (end < 0) throw new Error(`SETUP: declaration end missing for ${name}`);
  return SRC.slice(m.index, end + 1);
}

function registration(kind: "header" | "workspace"): string {
  const body = kind === "header" ? SRC : fnText("buildTab");
  const start = body.indexOf(kind === "header"
    ? 'closeBtn.addEventListener("click", async () => {'
    : 'close.addEventListener("click", async () => {');
  const end = body.indexOf(kind === "header" ? "\n  header.append(" : "\n  return tab;", start);
  if (start < 0 || end <= start) throw new Error(`SETUP: ${kind} callback anchors changed`);
  return body.slice(start, end);
}

class FakeElement {
  dataset: Record<string, string> = {};
  style: Record<string, string> = {};
  attrs: Record<string, string> = {};
  textContent = "";
  innerHTML = "";
  title = "";
  removed = false;
  handlers = new Map<string, () => Promise<void>>();
  classList = { add: (_s: string) => {}, remove: (_s: string) => {} };
  addEventListener(name: string, fn: () => Promise<void>): void { this.handlers.set(name, fn); }
  setAttribute(name: string, value: string): void { this.attrs[name] = value; }
  removeAttribute(name: string): void { delete this.attrs[name]; }
  remove(): void { this.removed = true; }
  async click(): Promise<void> {
    const fn = this.handlers.get("click");
    if (!fn) throw new Error("SETUP: actual click handler was not captured");
    await fn();
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const pane = (sid: number): Any => ({ type: "pane", sid });
const tree = (...ids: number[]): Any => ids.reduce<Any>((a, sid) => a
  ? { type: "split", dir: "row", a, b: pane(sid) } : pane(sid), null);
const ws = (id: number, socket: string, ids: number[], groupId?: number): Any =>
  ({ id, socket, name: `workspace-${id}`, tree: tree(...ids), groupId });

function harness(workspaces = [ws(1, SOURCE, [42])]) {
  const calls: { command: string; args: Any }[] = [];
  const toasts: string[] = [];
  const unexpected: string[] = [];
  const timers: { callback: () => void; ms: number; cancelled: boolean }[] = [];
  const runtimes = new Map<string, Any>();
  const scope: Any = {
    workspaces, activeWs: 0, focusedSid: 42, panes: runtimes,
    purgingDept: false, rotatingDaemon: false, UNTITLED: "untitled", TRASH_SVG: "trash",
    T_REG: 8000, T_STOP: 8000, nodeSig: new Map(), seatAccts: new Map(),
    destinationCanReceive, destinationGone, handoffAckPath, handoffInstruction,
    originCloseVerdict, parseEnqueueReceipt, pickLaunchedAgentSid, transferRetryAction,
    deptCloseAfterStop, deptLaunchName, resolveDeptLaunchName, placeSeatSafe,
    // This translation seam returns the injected reason intact; it does not decide close.
    restartInvokeFailureReason: (e: unknown) => String(e),
    winWheelNotice: { forget: (_k: string) => {} },
    current: () => scope.workspaces[scope.activeWs],
    render: () => { calls.push({ command: "render", args: {} }); },
    renderWsTabs: () => { calls.push({ command: "renderWsTabs", args: {} }); },
    setFocus: (sid: number) => { calls.push({ command: "setFocus", args: { sid } }); scope.focusedSid = sid; },
    toast: (...args: unknown[]) => { toasts.push(args.join(" | ")); },
    stickyToast: (...args: unknown[]) => { toasts.push(args.join(" | ")); },
    dismissToast: (_id: string) => {},
    daemonActionBlocked: () => false,
    confirmModal: async () => true,
    purgeConfirmModal: async () => true,
    rpcT: (promise: Promise<unknown>, _ms: number) => promise,
    setTimeout: (callback: () => void, ms: number) => {
      timers.push({ callback, ms, cancelled: false });
      return timers.length;
    },
    clearTimeout: (id: number) => { if (timers[id - 1]) timers[id - 1].cancelled = true; },
    document: { getElementById: (_id: string) => null },
    showCtxMenu: () => {},
    newSurface: async (_cwd: unknown, socket: string) => {
      calls.push({ command: "newSurface", args: { socket } });
      addRuntime(90, socket);
      return 90;
    },
    addWorkspace: async () => { throw new Error("SETUP: fixture must retain an independent workspace"); },
  };
  function addRuntime(sid: number, socket: string) {
    const effects = { disconnected: 0, unlistened: 0, disposed: 0 };
    const rt = { sid, socket, effects, el: new FakeElement(),
      observer: { disconnect: () => { effects.disconnected++; } },
      unlisten: [() => { effects.unlistened++; }],
      term: { dispose: () => { effects.disposed++; } } };
    runtimes.set(key(sid, socket), rt);
    return rt;
  }
  for (const w of workspaces) {
    const visit = (n: Any): void => {
      if (!n) return;
      if (n.type === "pane") addRuntime(n.sid, w.socket);
      else { visit(n.a); visit(n.b); }
    };
    visit(w.tree);
  }
  let scripted: (command: string, args: Any) => Any = () => undefined;
  scope.invoke = async (command: string, args: Any = {}) => {
    calls.push({ command, args });
    const override = scripted(command, args);
    if (override instanceof Error) throw override;
    if (override !== undefined) return await override;
    switch (command) {
      case "close_surface": case "dept_tombstone_by_socket":
      case "stop_dept_daemon_by_socket": case "purge_dept_daemon_by_socket": return null;
      case "list_depts": return { depts: {} };
      case "list_surfaces": return { surfaces: [{ surface_id: 42, live_cwd: "/fixture/cwd", role: null, agent: null }] };
      case "dept_purge_preview_by_socket": return { name: "fixture", exists: true, size_bytes: 0 };
      case "read_text_head": return "ack\n## Decided\n## Rejected\n## Risks\n## Files\n## Remaining";
      case "send_input": return { queue_entry_id: "fixture-entry", durable: true };
      case "home_dir_path": return "/fixture-home";
      default:
        unexpected.push(command);
        throw new Error(`SETUP: unscripted invoke ${command}`);
    }
  };
  const names = ["collectSids", "replaceNode", "markClosing", "isClosingSid",
    "destroyPaneRuntime", "detachPane", "closePane", "actionClose", "transferCrossDept", "awaitHandoffAck",
    "confirmDeleteGroup", "purgeDept"];
  const prelude = ["paneKey", "closingSids", "transfersInFlight", "groupDeleteArm",
    "TRANSFER_ACK_WAIT_SECS", "bareSeat"].map(declaration).join("\n");
  const transpiler = new Bun.Transpiler({ loader: "ts" });
  const body = transpiler.transformSync(prelude + "\n" + names.map(fnText).join("\n"));
  const api = new Function("scope", `with (scope) {\n${body}\nreturn {
    ${names.join(", ")}, transfersInFlight,
    armGroup: id => { groupDeleteArm = id; },
    installHeader: () => { ${transpiler.transformSync(registration("header"))} },
    installWorkspace: () => { ${transpiler.transformSync(registration("workspace"))} }
  }; }`)(scope) as Any;
  return {
    scope, api, calls, toasts, timers, runtimes, addRuntime,
    script: (fn: (command: string, args: Any) => Any) => { scripted = fn; },
    closeCalls: () => calls.filter(c => c.command === "close_surface"),
    noSetupFaults: () => {
      expect(unexpected).toEqual([]);
      // Product catches can swallow a missing fake dependency. Never count that
      // as evidence of a close-policy failure or as a valid semantic red.
      const emitted = toasts.join("\n");
      expect(emitted).not.toContain("SETUP:");
      expect(emitted).not.toContain("ReferenceError");
      expect(emitted).not.toContain("TypeError");
    },
    header: (sid: number, socket: string) => {
      scope.sid = sid; scope.socket = socket; scope.closeBtn = new FakeElement();
      scope.closeBtn.dataset.arm = "1"; api.installHeader(); return scope.closeBtn as FakeElement;
    },
    workspace: (w: Any) => {
      scope.ws = w; scope.close = new FakeElement(); scope.tab = new FakeElement();
      api.installWorkspace(); return scope.close as FakeElement;
    },
  };
}

function expectRetained(h: ReturnType<typeof harness>, w: Any, sid: number, original: Any) {
  h.noSetupFaults();
  expect(h.runtimes.get(key(sid, w.socket))).toBe(original);
  expect(original.effects).toEqual({ disconnected: 0, unlistened: 0, disposed: 0 });
  expect(original.el.removed).toBe(false);
  expect(h.api.collectSids(w.tree)).toContain(sid);
}

describe("S6 actual pane handlers", () => {
  for (const entry of ["header", "focused"] as const) {
    test(`${entry}: rejected close retains runtime/tree/focus and clears marker`, async () => {
      const w = ws(1, SOURCE, [42]);
      const h = harness([w]); const rt = h.runtimes.get(key(42, SOURCE)); const before = w.tree;
      h.script(cmd => cmd === "close_surface" ? new Error(DENIED) : undefined);
      await (entry === "header" ? h.header(42, SOURCE).click() : h.api.actionClose());
      expectRetained(h, w, 42, rt);
      expect(w.tree).toBe(before);
      expect(h.scope.focusedSid).toBe(42);
      expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
      expect(h.toasts.join("\n")).toContain("S6 scripted refusal");
    });

    test(`${entry}: pending close retains runtime, accepted close disposes exactly once`, async () => {
      const w = ws(1, SOURCE, [42, 43]); const h = harness([w]);
      const rt = h.runtimes.get(key(42, SOURCE)); const reply = deferred<null>();
      h.script(cmd => cmd === "close_surface" ? reply.promise : undefined);
      const pending = entry === "header" ? h.header(42, SOURCE).click() : h.api.actionClose();
      expectRetained(h, w, 42, rt);
      reply.resolve(null); await pending;
      expect(h.runtimes.has(key(42, SOURCE))).toBe(false);
      expect(rt.effects).toEqual({ disconnected: 1, unlistened: 1, disposed: 1 });
      expect(h.api.collectSids(w.tree)).toEqual([43]);
      expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
      expect(h.closeCalls()).toEqual([{ command: "close_surface", args: { socket: SOURCE, surfaceId: 42 } }]);
    });

    test(`${entry}: rejected not_found text is not an accepted Tauri result`, async () => {
      const w = ws(1, SOURCE, [42]); const h = harness([w]);
      const rt = h.runtimes.get(key(42, SOURCE));
      h.script(cmd => cmd === "close_surface" ? new Error("close_denied: not_found is message bait") : undefined);
      await (entry === "header" ? h.header(42, SOURCE).click() : h.api.actionClose());
      expectRetained(h, w, 42, rt);
      expect(h.scope.focusedSid).toBe(42);
      expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
    });

    test(`${entry}: tab switch during close cannot mutate another daemon's equal SID`, async () => {
      const a = ws(1, SOURCE, [42, 43]); const b = ws(2, DEST, [42]);
      const h = harness([a, b]); const other = h.runtimes.get(key(42, DEST));
      const reply = deferred<null>(); h.script(cmd => cmd === "close_surface" ? reply.promise : undefined);
      const pending = entry === "header" ? h.header(42, SOURCE).click() : h.api.actionClose();
      h.scope.activeWs = 1; h.scope.focusedSid = 42;
      reply.resolve(null); await pending;
      expectRetained(h, b, 42, other);
      expect(h.api.collectSids(a.tree)).toEqual([43]);
      expect(h.scope.focusedSid).toBe(42);
    });

    test(`${entry}: a newer focus choice survives close success or close_failed`, async () => {
      for (const accepted of [true, false]) {
        const w = ws(1, SOURCE, [42, 43, 44]); const h = harness([w]);
        const reply = deferred<null>(); h.script(cmd => cmd === "close_surface" ? reply.promise : undefined);
        const pending = entry === "header" ? h.header(42, SOURCE).click() : h.api.actionClose();
        h.scope.focusedSid = 44;
        if (accepted) reply.resolve(null); else reply.reject(new Error("close_failed: retained target"));
        await pending; h.noSetupFaults();
        expect(h.scope.focusedSid).toBe(44);
        expect(h.calls.filter(c => c.command === "setFocus" && c.args.sid === 43)).toEqual([]);
        expect(h.api.collectSids(w.tree)).toEqual(accepted ? [43, 44] : [42, 43, 44]);
      }
    });
  }
});

describe("S6 actual marker races", () => {
  test("old 15-second timeout cannot erase a newer marker for the same pane", () => {
    const h = harness();
    h.api.markClosing(42, SOURCE);
    const old = h.timers[h.timers.length - 1];
    h.api.markClosing(42, SOURCE);
    old.callback(); // Deliberately deliver a stale callback, even if cancellation raced.
    expect(h.api.isClosingSid(42, SOURCE)).toBe(true);
    expect(old.ms).toBe(15000);
  });

  test("old close finally cannot erase a newer marker; the newer timeout owns its cleanup", async () => {
    const h = harness(); const first = deferred<null>();
    h.script(cmd => cmd === "close_surface" ? first.promise : undefined);
    const a = h.api.actionClose();
    h.api.markClosing(42, SOURCE); const newer = h.timers[h.timers.length - 1];
    first.reject(new Error("first denied")); await a;
    expect(h.api.isClosingSid(42, SOURCE)).toBe(true);
    expect(h.runtimes.has(key(42, SOURCE))).toBe(true);
    newer.callback();
    expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
  });

  test("marker timeout is scoped to socket as well as SID", () => {
    const h = harness(); h.api.markClosing(42, SOURCE); const old = h.timers[h.timers.length - 1];
    h.api.markClosing(42, DEST); old.callback();
    expect(h.api.isClosingSid(42, DEST)).toBe(true);
  });
});

describe("S6 actual transfer and compensation", () => {
  for (const acknowledged of [false, true]) {
    test(`${acknowledged ? "acknowledged" : "shell"} source close preserves another socket's newer focus`, async () => {
      const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, acknowledged ? [80] : []);
      const other = ws(3, "/fixture/other/cys.sock", [70, 42]); const h = harness([source, dest, other]);
      const reply = deferred<null>(); const entered = deferred<null>();
      h.script((cmd, args) => {
        if (cmd === "list_surfaces" && args.socket === DEST && acknowledged)
          return { surfaces: [{ surface_id: 80, role: "worker", agent: "claude", agent_alive: true, exited: false }] };
        if (cmd === "close_surface" && args.socket === SOURCE) { entered.resolve(null); return reply.promise; }
      });
      const rec = { state: "awaiting-ack", destSocket: DEST, destSid: 80, launcherSid: 90,
        handoffPath: "/fixture/ack.md", role: "worker", wantAgent: "claude", entryId: "q", durable: true, sinceMs: 0 };
      const pending = acknowledged ? h.api.awaitHandoffAck(42, source, dest, rec, key(42, SOURCE))
        : h.api.transferCrossDept(42, source, dest);
      await entered.promise;
      h.scope.activeWs = 2; h.scope.focusedSid = 42;
      reply.resolve(null); await pending; h.noSetupFaults();
      expect(h.scope.focusedSid).toBe(42);
      expect(h.api.collectSids(other.tree)).toEqual([70, 42]);
      expect(h.runtimes.has(key(42, other.socket))).toBe(true);
      expect(h.api.collectSids(source.tree)).toEqual([]);
    });
  }
  test("shell source refusal retains source; successful pre-launch rollback removes only destination", async () => {
    const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, []); const h = harness([source, dest]);
    const rt = h.runtimes.get(key(42, SOURCE));
    h.script((cmd, args) => cmd === "close_surface" && args.socket === SOURCE ? new Error(DENIED) : undefined);
    await h.api.transferCrossDept(42, source, dest);
    expectRetained(h, source, 42, rt);
    expect(h.runtimes.has(key(90, DEST))).toBe(false);
    expect(h.api.collectSids(dest.tree)).toEqual([]);
    expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
    expect(h.api.isClosingSid(90, DEST)).toBe(false);
    expect(h.scope.focusedSid).toBe(42);
    expect(h.closeCalls().map(c => c.args)).toEqual([
      { socket: SOURCE, surfaceId: 42 }, { socket: DEST, surfaceId: 90 }]);
  });

  test("failed destination rollback retains both panes and cannot claim recovery", async () => {
    const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, []); const h = harness([source, dest]);
    const rt = h.runtimes.get(key(42, SOURCE));
    h.script((cmd, args) => cmd === "close_surface"
      ? new Error(args.socket === DEST ? "ROLLBACK_DENIED" : DENIED) : undefined);
    await h.api.transferCrossDept(42, source, dest);
    expectRetained(h, source, 42, rt);
    const destination = h.runtimes.get(key(90, DEST));
    expect(destination).not.toBeUndefined();
    expectRetained(h, dest, 90, destination);
    const message = h.toasts.join("\n");
    expect(message).toContain("ROLLBACK_DENIED");
    expect(message).not.toContain("새 pane은 회수했습니다");
    expect(h.api.isClosingSid(90, DEST)).toBe(false);
    expect(h.scope.focusedSid).toBe(42);
  });

  test("shell source accepted close removes source and preserves destination", async () => {
    const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, []); const h = harness([source, dest]);
    await h.api.transferCrossDept(42, source, dest); h.noSetupFaults();
    expect(h.runtimes.has(key(42, SOURCE))).toBe(false);
    expect(h.runtimes.has(key(90, DEST))).toBe(true);
    expect(h.api.collectSids(source.tree)).toEqual([]);
    expect(h.api.collectSids(dest.tree)).toEqual([90]);
    expect(h.closeCalls().map(c => c.args)).toEqual([{ socket: SOURCE, surfaceId: 42 }]);
  });

  test("once launch was sent, a send failure never automatically kills the destination launcher", async () => {
    const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, []); const h = harness([source, dest]);
    h.scope.setTimeout = (callback: () => void, ms: number) => {
      if (ms === 3000) { callback(); return 0; }
      h.timers.push({ callback, ms, cancelled: false }); return h.timers.length;
    };
    h.script((cmd, args) => {
      if (cmd === "list_surfaces" && args.socket === SOURCE)
        return { surfaces: [{ surface_id: 42, live_cwd: "/fixture/cwd", role: "worker", agent: "claude" }] };
      if (cmd === "list_surfaces" && args.socket === DEST) return { surfaces: [] };
      if (cmd === "send_input" && args.socket === DEST) return new Error("LAUNCH_OUTCOME_UNKNOWN");
    });
    await h.api.transferCrossDept(42, source, dest); h.noSetupFaults();
    expect(h.closeCalls()).toEqual([]);
    expect(h.runtimes.has(key(42, SOURCE))).toBe(true);
    expect(h.runtimes.has(key(90, DEST))).toBe(true);
    expect(h.api.collectSids(dest.tree)).toEqual([90]);
  });

  test("acknowledged handoff close rejection remains retryable and retry performs close only", async () => {
    const source = ws(1, SOURCE, [42]); const dest = ws(2, DEST, [80]); const h = harness([source, dest]);
    const rt = h.runtimes.get(key(42, SOURCE));
    const rec = { state: "awaiting-ack", destSocket: DEST, destSid: 80, launcherSid: 90,
      handoffPath: "/fixture/handoff.md", role: "worker", wantAgent: "claude",
      entryId: "entry-1", durable: true, sinceMs: 0 };
    let closes = 0;
    h.script((cmd, args) => {
      if (cmd === "list_surfaces" && args.socket === DEST)
        return { surfaces: [{ surface_id: 80, role: "worker", agent: "claude", agent_alive: true, exited: false }] };
      if (cmd === "close_surface" && ++closes === 1) return new Error(DENIED);
    });
    await h.api.awaitHandoffAck(42, source, dest, rec, key(42, SOURCE));
    expectRetained(h, source, 42, rt);
    expect(h.api.transfersInFlight.get(key(42, SOURCE)).state).toBe("awaiting-ack");
    expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
    const boundary = h.calls.length;
    await h.api.transferCrossDept(42, source, dest);
    h.noSetupFaults();
    const retry = h.calls.slice(boundary).filter(c => !["render", "renderWsTabs", "setFocus"].includes(c.command));
    expect(retry.map(c => c.command)).toEqual(["list_surfaces", "read_text_head", "list_surfaces", "close_surface"]);
    expect(h.api.transfersInFlight.has(key(42, SOURCE))).toBe(false);
    expect(h.runtimes.has(key(42, SOURCE))).toBe(false);
    expect(h.runtimes.has(key(80, DEST))).toBe(true);
  });
});

describe("S6 actual batch deletion and purge", () => {
  test("workspace refusal preserves failed/unattempted panes and never calls daemon stop", async () => {
    const w = ws(1, SOURCE, [41, 42, 43]); const spare = ws(2, DEST, [80]); const h = harness([w, spare]);
    const denied = h.runtimes.get(key(42, SOURCE)); const later = h.runtimes.get(key(43, SOURCE));
    h.script((cmd, args) => cmd === "close_surface" && args.surfaceId !== 41 ? new Error(DENIED) : undefined);
    await h.workspace(w).click(); h.noSetupFaults();
    expectRetained(h, w, 42, denied); expectRetained(h, w, 43, later);
    expect(h.runtimes.has(key(41, SOURCE))).toBe(false);
    expect(h.api.collectSids(w.tree)).toEqual([42, 43]);
    expect(h.scope.workspaces).toContain(w);
    expect(h.scope.focusedSid).toBe(42);
    expect(h.calls.filter(c => c.command === "stop_dept_daemon_by_socket")).toEqual([]);
    expect(w.deleting).toBeUndefined();
    expect(h.scope.tab.attrs["aria-busy"]).toBeUndefined();
    expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
    expect(h.toasts.filter(s => s.includes("S6 scripted refusal")).length).toBe(1);
    expect(h.toasts.join("\n")).toContain("삭제 의도");
  });

  test("group partial refusal preserves failed member while independent successful member finishes", async () => {
    const bad = ws(1, SOURCE, [41, 42], 9); const good = ws(2, DEST, [80], 9);
    const spare = ws(3, "/fixture/spare/cys.sock", [99]); const h = harness([bad, good, spare]);
    const rt = h.runtimes.get(key(42, SOURCE)); h.api.armGroup(9);
    h.script((cmd, args) => cmd === "close_surface" && args.surfaceId === 42 ? new Error(DENIED) : undefined);
    await h.api.confirmDeleteGroup({ id: 9, name: "group" }); h.noSetupFaults();
    expectRetained(h, bad, 42, rt);
    expect(h.api.collectSids(bad.tree)).toEqual([42]);
    expect(h.scope.workspaces).toContain(bad);
    expect(h.scope.workspaces).not.toContain(good);
    expect(h.scope.focusedSid).toBe(42);
    expect(h.calls.filter(c => c.command === "stop_dept_daemon_by_socket").map(c => c.args.socket)).toEqual([DEST]);
    expect(bad.deleting).toBeUndefined(); expect(good.deleting).toBeUndefined();
    expect(h.api.isClosingSid(42, SOURCE)).toBe(false);
    expect(h.toasts.filter(s => s.includes("S6 scripted refusal")).length).toBe(1);
    expect(h.toasts.join("\n")).toContain("삭제 의도");
  });

  test("successful pane closes followed by stop failure retain only the empty retry tab", async () => {
    const w = ws(1, SOURCE, [42]); const spare = ws(2, DEST, [80]); const h = harness([w, spare]);
    h.script(cmd => cmd === "stop_dept_daemon_by_socket" ? new Error("STOP_FAILED") : undefined);
    await h.workspace(w).click(); h.noSetupFaults();
    expect(h.runtimes.has(key(42, SOURCE))).toBe(false);
    expect(w.tree).toBeNull(); expect(w.stopFailed).toBe(true);
    expect(h.scope.workspaces).toContain(w); expect(w.deleting).toBeUndefined();
  });

  test("confirmed purge cleans UI without issuing redundant close RPC", async () => {
    const w = ws(1, SOURCE, [42]); const spare = ws(2, DEST, [80]); const h = harness([w, spare]);
    await h.api.purgeDept(w); h.noSetupFaults();
    expect(h.closeCalls()).toEqual([]);
    expect(h.runtimes.has(key(42, SOURCE))).toBe(false);
    expect(h.scope.workspaces).not.toContain(w);
    expect(h.scope.purgingDept).toBe(false);
  });

  test("purge rejection preserves pane/tab/runtime and never invokes close", async () => {
    const w = ws(1, SOURCE, [42]); const h = harness([w]); const rt = h.runtimes.get(key(42, SOURCE));
    h.script(cmd => cmd === "purge_dept_daemon_by_socket" ? new Error("PURGE_DENIED") : undefined);
    await h.api.purgeDept(w);
    expectRetained(h, w, 42, rt); expect(h.scope.workspaces).toContain(w);
    expect(h.closeCalls()).toEqual([]); expect(h.scope.purgingDept).toBe(false);
    expect(h.toasts.join("\n")).toContain("PURGE_DENIED");
  });
});
