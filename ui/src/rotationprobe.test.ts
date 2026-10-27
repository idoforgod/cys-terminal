// WP12 B2: execute the real UI functions, including their asynchronous control flow.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";

declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(s: string): string } };
const main = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
const shellRefusal = readFileSync(new URL("../../scripts/tests/fixtures/cys-dept-binprobe-stderr.txt", import.meta.url), "utf-8");
const begin = main.indexOf("let verSkewBadge:");
const end = main.indexOf("async function promptPackInstall", begin);
if (begin < 0 || end < begin) throw new Error("rotation function region missing");
const js = new Bun.Transpiler({ loader: "ts" }).transformSync(main.slice(begin, end));

type Call = { cmd: string; args?: Record<string, unknown> };
type Api = {
  checkVersionSkew(): Promise<void>;
  manualRotateSkewed(v: string, main: boolean, depts: { name: string; socket: string }[]): Promise<void>;
  manualRestartAllDaemons(): Promise<void>;
  restartAllDaemons(skip: boolean): Promise<unknown>;
  rotateMainDaemon(force: boolean): Promise<string>;
  rotateDeptDaemon(name: string, force: boolean): Promise<string>;
};
function rig(options: { mainSkew?: boolean; fail?: (call: Call) => unknown; delay?: (call: Call) => Promise<void> | undefined } = {}) {
  const calls: Call[] = [];
  const notices: unknown[][] = [];
  const confirmations: unknown[][] = [];
  const badges: { textContent: string; title: string; isConnected: boolean; remove(): void }[] = [];
  let version = "0.14.53";
  let confirm = true;
  // r1 m4: the departments-only transition needs departments that STOP being skewed, which is
  // what a successful departments-only retry leaves behind. Defaults to the old behaviour.
  let deptSkew = true;
  const scope = {
    rotatingDaemon: false, purgingDept: false, factoryResetting: false, resetCompleted: false,
    document: {
      getElementById: () => ({ appendChild: () => {} }),
      createElement: () => {
        const b = { textContent: "", title: "", isConnected: true, remove() { b.isConnected = false; } };
        badges.push(b); return b;
      },
    },
    invoke: async (cmd: string, args?: Record<string, unknown>) => {
      const call = { cmd, args }; calls.push(call);
      await options.delay?.(call);
      const failure = options.fail?.(call);
      if (failure !== undefined) throw failure;
      if (cmd === "app_version") return version;
      if (cmd === "daemon_status") {
        if (args?.socket) return { version: deptSkew ? "0.14.52" : version };
        return { version: options.mainSkew === false ? version : "0.14.52" };
      }
      if (cmd === "list_depts") return { depts: { first: { socket: "s1" }, later: { socket: "s2" } } };
      if (cmd === "drain_verify") return { all_saved: true, nodes: [], pending_loss_warning: [] };
      return { restore_ok: true };
    },
    toast: (...args: unknown[]) => notices.push(args),
    stickyToast: () => {}, dismissToast: () => {},
    confirmModal: async (...args: unknown[]) => { confirmations.push(args); return confirm; },
    daemonActionBlocked: () => scope.rotatingDaemon || scope.purgingDept || scope.factoryResetting || scope.resetCompleted,
    daemonActionBlockedMsg: () => "busy",
    classifyDrainVerifyFallback: () => null,
    drainVerifyFallbackToast: () => ({ title: "fallback", body: "fallback" }),
    drainVerifyUnresponsiveLines: () => [],
  };
  const api = new Function("scope", `with (scope) { ${js}\nreturn {checkVersionSkew, manualRotateSkewed, manualRestartAllDaemons, restartAllDaemons, rotateMainDaemon, rotateDeptDaemon}; }`)(scope) as Api;
  const rotations = () => calls.filter((c) => c.cmd === "rotate_daemon" || c.cmd === "rotate_dept_daemon");
  return { api, calls, notices, confirmations, badges, scope, rotations,
    setVersion: (v: string) => { version = v; }, setConfirm: (v: boolean) => { confirm = v; },
    setDeptSkew: (v: boolean) => { deptSkew = v; } };
}

describe("WP12 rotation refusal latch", () => {
  it("actual shell stderr stops later departments and all rotations on the next tick", async () => {
    const r = rig({ mainSkew: false, fail: (c) => c.cmd === "rotate_dept_daemon" && c.args?.name === "first" ? shellRefusal : undefined });
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.args?.name)).toEqual(["first"]);
    const noticeCount = r.notices.length;
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.args?.name)).toEqual(["first"]);
    expect(r.notices.length).toBe(noticeCount);
    expect(r.scope.rotatingDaemon).toBe(false);
    expect(r.badges.at(-1)?.textContent).toContain("확인 실패");
  });

  it("OS detection refusal uses the same latch and stops later automatic ticks", async () => {
    const line = "new_daemon_blocked:probe:os-detection-failed [cys-dept] demo: OS detection failed — rotate stopped (daemon unchanged)\n";
    const r = rig({ mainSkew: false, fail: (c) => c.cmd === "rotate_dept_daemon" ? line : undefined });
    await r.api.checkVersionSkew();
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.args?.name)).toEqual(["first"]);
    expect(r.notices.length).toBe(1);
    expect(r.badges.at(-1)?.textContent).toContain("확인 실패");
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("main refusal skips the entire department loop and has one automatic notice", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_daemon" ? "new_daemon_blocked:cys:grammar" : undefined });
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.cmd)).toEqual(["rotate_daemon"]);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(1);
    expect(r.notices.length).toBe(1);
  });

  it("all typed refusal reasons classify through the real wrappers", async () => {
    for (const why of ["grammar", "rc:1", "error:5", "timeout", "cleanup-unconfirmed", "worker-error:panic", "output-too-large", "path-mismatch", "path-resolution-failed", "nonabsolute-path"]) {
      for (const bin of ["cysd", "cys", "probe", "pair"]) {
        const error = `new_daemon_blocked:${bin}:${why}`;
        const r = rig({ fail: () => error });
        expect(await r.api.rotateMainDaemon(false)).toBe("blocked");
        expect(await r.api.rotateDeptDaemon("first", false)).toBe("blocked");
      }
    }
  });

  it("healthy rotation never latches; ordinary errors and mention-only text stay ordinary", async () => {
    for (const failure of [undefined, "ordinary failure", "log mentions new_daemon_blocked:cys:timeout", "new_daemon_blocked", "new_daemon_blockedness:bad"]) {
      const r = rig({ fail: (c) => c.cmd.startsWith("rotate_") ? failure : undefined });
      await r.api.checkVersionSkew();
      await r.api.checkVersionSkew();
      expect(r.rotations().length).toBe(6);
      expect(r.scope.rotatingDaemon).toBe(false);
      expect(await r.api.rotateMainDaemon(false)).toBe(failure === undefined ? "ok" : "err");
    }
    const held = rig({ fail: () => "live_sessions:2" });
    expect(await held.api.rotateMainDaemon(false)).toBe("held");
  });

  it("a new app version is not blocked by the previous version latch", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_daemon" ? shellRefusal : undefined });
    await r.api.checkVersionSkew();
    r.setVersion("0.14.54");
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(2);
    expect(r.notices.length).toBe(2);
  });

  it("main-only manual rotation does not preflight or latch on the department pair", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" && c.args?.includeDepartments === true ? shellRefusal : undefined });
    await r.api.manualRotateSkewed("0.14.53", true, []);
    const probe = r.calls.find((c) => c.cmd === "probe_rotation_binaries");
    expect(probe?.args).toEqual({ includeMain: true, includeDepartments: false });
    expect(r.rotations().map((c) => c.cmd)).toEqual(["rotate_daemon"]);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("department-only manual rotation does not preflight unrelated native siblings", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" && c.args?.includeMain === true ? shellRefusal : undefined });
    await r.api.manualRotateSkewed("0.14.53", false, [{ name: "first", socket: "s1" }]);
    const probe = r.calls.find((c) => c.cmd === "probe_rotation_binaries");
    expect(probe?.args).toEqual({ includeMain: false, includeDepartments: true });
    expect(r.rotations().map((c) => c.args?.name)).toEqual(["first"]);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  // P4 C-08 blocker 1 — AMENDED. The original cell asserted the first probe arguments were
  // `{ includeMain: true, includeDepartments: true }`, i.e. it pinned the WIDENING of an empty
  // retry, and that widening is blocker 1's mechanism: a PC with zero departments probed an
  // unrelated shell-selected pair and latched a healthy main. The cell's stated intent — an
  // empty retry must not clear a latch without any probe — is preserved and made stricter:
  // ZERO probe calls, no confirmation modal, and the pre-existing latch still holding after.
  it("empty-target retry neither probes nor clears a pre-existing latch", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_daemon" ? shellRefusal : undefined });
    await r.api.checkVersionSkew(); // a main refusal latches both routes (asymmetric, by design)
    const probesBefore = r.calls.filter((c) => c.cmd === "probe_rotation_binaries").length;
    const rotationsBefore = r.rotations().length;
    await r.api.manualRotateSkewed("0.14.53", false, []);
    expect(r.calls.filter((c) => c.cmd === "probe_rotation_binaries").length).toBe(probesBefore);
    expect(r.confirmations.length).toBe(0);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(rotationsBefore); // the latch survived the empty retry
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("manual restart refusal happens before confirmation, drain and restart; controls stay usable", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" ? shellRefusal : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.calls.some((c) => c.cmd === "probe_rotation_binaries")).toBe(true);
    expect(r.confirmations.length).toBe(0);
    expect(r.calls.some((c) => c.cmd === "drain_verify")).toBe(false);
    expect(r.rotations().length).toBe(0);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("manual skew refusal happens before confirmation and rotation", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" ? shellRefusal : undefined });
    await r.api.manualRotateSkewed("0.14.53", true, [{ name: "first", socket: "s1" }]);
    expect(r.confirmations.length).toBe(0);
    expect(r.rotations().length).toBe(0);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("manual successful probe clears a latch, then healthy flow keeps its drain", async () => {
    let blocked = true;
    const r = rig({ fail: (c) => c.cmd === "rotate_daemon" && blocked ? shellRefusal : undefined });
    await r.api.checkVersionSkew();
    blocked = false;
    await r.api.manualRestartAllDaemons();
    const probe = r.calls.findIndex((c) => c.cmd === "probe_rotation_binaries");
    const drain = r.calls.findIndex((c) => c.cmd === "drain_verify");
    expect(probe).toBeGreaterThanOrEqual(0);
    expect(drain).toBeGreaterThan(probe);
    const count = r.rotations().length;
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(count + 3);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  // P4 C-08 blocker 1 — AMENDED. The original cell expected 2 rotations, which pinned the
  // department -> main latch: one department's shell refusal stopped the healthy main for the
  // whole app version. That is the behaviour C-08 exists to remove. The department half of the
  // assertion is unchanged and still load-bearing (the refused department is not retried and
  // the later department is not reached); what changes is that main continues.
  it("a shell-only refusal during manual restart stops later departments but not the main", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_dept_daemon" && c.args?.name === "first" ? shellRefusal : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.rotations().map((c) => c.args?.name ?? "main")).toEqual(["main", "first"]);
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.args?.name ?? "main")).toEqual(["main", "first", "main"]);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("ordinary department restart errors still allow later departments", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_dept_daemon" && c.args?.name === "first" ? "ordinary error" : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.rotations().map((c) => c.args?.name ?? "main")).toEqual(["main", "first", "later"]);
    expect(r.scope.rotatingDaemon).toBe(false);
  });
  // P4 C-08 blocker 1 — HARNESS AMENDED (the product is unchanged by this edit). The design
  // issues ONE probe call PER ROUTE, so the single `release` slot was overwritten by the
  // second call and the first promise never settled: the cell timed out at 5000 ms. The gate
  // below holds every pending probe and, once opened, releases those already waiting and lets
  // later ones through. The cell still measures what it always measured: while a manual probe
  // is in flight it owns the guard, so an automatic tick rotates nothing and does not drain.
  // Do NOT "repair" this by collapsing the two per-route probe calls back into one combined
  // call — that combined call is exactly blocker 1.
  it("a pending manual probe owns the guard and prevents automatic rotation", async () => {
    let gateOpen = false;
    const waiting: (() => void)[] = [];
    const r = rig({ delay: (c) => {
      if (c.cmd !== "probe_rotation_binaries") return undefined;
      if (gateOpen) return Promise.resolve();
      return new Promise<void>((resolve) => { waiting.push(resolve); });
    } });
    const manual = r.api.manualRestartAllDaemons();
    for (let i = 0; i < 10 && waiting.length === 0; i++) await Promise.resolve();
    expect(waiting.length).toBeGreaterThan(0);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(0);
    expect(r.calls.some((c) => c.cmd === "drain_verify")).toBe(false);
    gateOpen = true;
    while (waiting.length) waiting.shift()!();
    await manual;
    expect(r.rotations().length).toBe(3);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("ordinary manual probe error does not latch and cancellation releases the UI guard", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" ? new Error("ordinary probe failure") : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.rotations().length).toBe(0);
    expect(r.scope.rotatingDaemon).toBe(false);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(3);
    const cancelled = rig();
    cancelled.setConfirm(false);
    await cancelled.api.manualRestartAllDaemons();
    await cancelled.api.manualRotateSkewed("0.14.53", true, []);
    expect(cancelled.rotations().length).toBe(0);
    expect(cancelled.calls.some((c) => c.cmd === "drain_verify")).toBe(false);
    expect(cancelled.scope.rotatingDaemon).toBe(false);
  });

  // ★(r1 m4 · codex-3) The departments-only transition. MB5 - `if (mainSkew &&
  // rotationIsBlocked(appVer, "main")) heldMain = true;` - was reported as a REDUNDANT guard on
  // the reasoning that a main refusal latches depts too, so rotationAllBlocked() returns first.
  // That reasoning misses this transition: a successful departments-only retry deletes ONLY the
  // depts key, so main stays latched while rotationAllBlocked() is false - and the next tick
  // reaches the guard. Without it heldMain is false, heldDepts is empty, and clearSkewBadge()
  // wipes the one badge the user can click to retry main. MB5 is a FORMER TEST GAP, not a
  // redundant guard; this cell is the specimen that was missing.
  it("a departments-only retry clears only that latch, and the next tick keeps the main retry badge", async () => {
    const r = rig({ mainSkew: true, fail: (c) => c.cmd === "rotate_daemon" ? shellRefusal : undefined });
    // 1. the main arm refuses: BOTH routes latch (a failing native pair is evidence about this build)
    await r.api.checkVersionSkew();
    expect(r.rotations().map((c) => c.cmd)).toEqual(["rotate_daemon"]);
    expect(r.badges.at(-1)?.textContent).toContain("확인 실패");
    // 2. the badge's departments-only retry: the probe passes, so ONLY the depts key is deleted
    await r.api.manualRotateSkewed("0.14.53", false, [{ name: "first", socket: "s1" }]);
    expect(r.calls.filter((c) => c.cmd === "probe_rotation_binaries")
            .some((c) => c.args?.includeDepartments === true && c.args?.includeMain === false)).toBe(true);
    // 3. those departments are now on the new version; main is still skewed AND still latched
    r.setDeptSkew(false);
    await r.api.checkVersionSkew();
    // the tick must NOT have been short-circuited by rotationAllBlocked() - main alone is blocked
    expect(r.rotations().map((c) => c.cmd)).toEqual(["rotate_daemon", "rotate_dept_daemon"]);
    const badge = r.badges.at(-1);
    expect(badge?.isConnected).toBe(true);
    expect(badge?.textContent).toBe("새 실행 파일 확인 실패 · 앱 v0.14.53 — 클릭하여 다시 확인");
  });

  it("Error-wrapped refusal uses its message, but mention-only errors do not latch", async () => {
    const r = rig({ fail: () => new Error(shellRefusal) });
    expect(await r.api.rotateMainDaemon(false)).toBe("blocked");
    expect(await r.api.rotateDeptDaemon("first", false)).toBe("blocked");
    const ordinary = rig({ fail: () => new Error("context: " + shellRefusal) });
    expect(await ordinary.api.rotateMainDaemon(false)).toBe("err");
  });

});
