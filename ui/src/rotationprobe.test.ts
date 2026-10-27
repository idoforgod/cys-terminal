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
      if (cmd === "daemon_status") return { version: !args?.socket && options.mainSkew === false ? version : "0.14.52" };
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
    setVersion: (v: string) => { version = v; }, setConfirm: (v: boolean) => { confirm = v; } };
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

  it("empty-target retry still validates before clearing the latch", async () => {
    const r = rig({ fail: (c) => c.cmd === "probe_rotation_binaries" && (c.args?.includeMain || c.args?.includeDepartments) ? shellRefusal : undefined });
    await r.api.manualRotateSkewed("0.14.53", false, []);
    expect(r.calls.find((c) => c.cmd === "probe_rotation_binaries")?.args).toEqual({ includeMain: true, includeDepartments: true });
    expect(r.confirmations.length).toBe(0);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(0);
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

  it("a shell-only refusal during manual restart stops later departments and future ticks", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_dept_daemon" && c.args?.name === "first" ? shellRefusal : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.rotations().map((c) => c.args?.name ?? "main")).toEqual(["main", "first"]);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(2);
    expect(r.scope.rotatingDaemon).toBe(false);
  });

  it("ordinary department restart errors still allow later departments", async () => {
    const r = rig({ fail: (c) => c.cmd === "rotate_dept_daemon" && c.args?.name === "first" ? "ordinary error" : undefined });
    await r.api.manualRestartAllDaemons();
    expect(r.rotations().map((c) => c.args?.name ?? "main")).toEqual(["main", "first", "later"]);
    expect(r.scope.rotatingDaemon).toBe(false);
  });
  it("a pending manual probe owns the guard and prevents automatic rotation", async () => {
    let release: (() => void) | undefined;
    const r = rig({ delay: (c) => c.cmd === "probe_rotation_binaries"
      ? new Promise<void>((resolve) => { release = resolve; }) : undefined });
    const manual = r.api.manualRestartAllDaemons();
    for (let i = 0; i < 10 && !release; i++) await Promise.resolve();
    expect(release === undefined).toBe(false);
    await r.api.checkVersionSkew();
    expect(r.rotations().length).toBe(0);
    expect(r.calls.some((c) => c.cmd === "drain_verify")).toBe(false);
    release!();
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

  it("Error-wrapped refusal uses its message, but mention-only errors do not latch", async () => {
    const r = rig({ fail: () => new Error(shellRefusal) });
    expect(await r.api.rotateMainDaemon(false)).toBe("blocked");
    expect(await r.api.rotateDeptDaemon("first", false)).toBe("blocked");
    const ordinary = rig({ fail: () => new Error("context: " + shellRefusal) });
    expect(await ordinary.api.rotateMainDaemon(false)).toBe("err");
  });

});
