// ★F-01 U-F05 — per-endpoint surface.list transport (surfacelist.ts) + its timing with the activity store
// (roleactivity.ts) + the three wiring seams that only exist as source (Tauri resolver, start() ownership hand-over,
// refreshPaneTitles dispatch position). Design: DESIGN-U-FROZEN-R3 §4 / R3-4 / U4-5 (head ruling).
//   P=3 s · D=T_LIST (8 s mac / 16 s Windows) · E=6 s · T=3P+2D+E=31/47 s anchored to request START · U4-5 stale=9 s.
import { describe, it, expect } from "bun:test";
import { readFileSync } from "node:fs";
import { SurfaceListTransport, type ListArrival } from "./surfacelist";
import { RoleActivityStore, projectActivity, activityExpiryMs } from "./roleactivity";

interface Deferred {
  promise: Promise<unknown>;
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}
function deferred(): Deferred {
  let resolve!: (v: unknown) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<unknown>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
/** 모든 마이크로태스크(해석 → 결속 → 미뤄 둔 디스패치 → 정산)가 돌 때까지. */
const flush = (): Promise<void> => new Promise<void>((r) => setTimeout(r, 0));

const DEFAULT_SOCK = "/default/cys.sock";

function fixture(opts: { resolve?: (eps: (string | null)[]) => Promise<unknown> } = {}) {
  const clock = { t: 1_000 };
  const arrivals: ListArrival[] = [];
  const calls: { arg: string | undefined; d: Deferred }[] = [];
  const resolveCalls: (string | null)[][] = [];
  const tr = new SurfaceListTransport({
    now: () => clock.t,
    deadlineMs: 8_000,
    resolve: (eps) => {
      resolveCalls.push(eps);
      // 기본 해석기 대역: 기본(null) → 기본 소켓 경로, 명시 경로 → 그대로(Tauri resolve_socket 과 같은 꼴)
      return opts.resolve ? opts.resolve(eps) : Promise.resolve(eps.map((e) => e ?? DEFAULT_SOCK));
    },
    onArrival: (a) => {
      arrivals.push(a);
    },
  });
  const launch = (arg: string | undefined): Promise<unknown> => {
    const d = deferred();
    calls.push({ arg, d });
    return d.promise;
  };
  const args = (): (string | undefined)[] => calls.map((c) => c.arg);
  return { clock, arrivals, calls, resolveCalls, tr, launch, args };
}

describe("★F-01 U-F05 — 끝점별 소유 원 호출(surfacelist.ts)", () => {
  it("hung first endpoint: siblings launch and project independently — no serial await, no Promise.all barrier", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock", "/b.sock", "/c.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.args()).toEqual(["/a.sock", "/b.sock", "/c.sock"]); // 첫 끝점이 매달려도 나머지가 같은 틱에 나간다
    f.calls[1].d.resolve({ surfaces: [{ surface_id: 1 }] });
    await flush();
    expect(f.arrivals.map((a) => a.endpoint)).toEqual(["/b.sock"]); // /a 를 기다리지 않고 그 응답만으로 투영
    f.calls[2].d.resolve({ surfaces: [] });
    await flush();
    expect(f.arrivals.map((a) => a.endpoint)).toEqual(["/b.sock", "/c.sock"]);
    expect(f.tr.outstanding()).toBe(1);
    f.clock.t += 3_000;
    f.tr.dispatch(f.launch);
    expect(f.args()).toEqual(["/a.sock", "/b.sock", "/c.sock", "/b.sock", "/c.sock"]); // /a 는 소유 중 — 새 원 호출 없음
  });

  it("default/explicit alias: the resolver result normalises both roster entries to one endpoint — one original, both members get the snapshot", async () => {
    const f = fixture();
    f.tr.setRoster([undefined, DEFAULT_SOCK, "/x.sock", undefined]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.resolveCalls).toEqual([[null, DEFAULT_SOCK, "/x.sock"]]); // 로스터는 소켓 값 중복 제거(main.ts 의 Set 과 같은 정의)
    expect(f.args()).toEqual([DEFAULT_SOCK, "/x.sock"]); // 데몬당 원 호출 1개 · 디스패치는 해석된 명시 경로로
    expect(f.tr.endpointOf(undefined)).toBe(DEFAULT_SOCK);
    expect(f.tr.endpointOf(DEFAULT_SOCK)).toBe(DEFAULT_SOCK);
    f.calls[0].d.resolve({ surfaces: [] });
    await flush();
    expect(f.arrivals.length).toBe(1);
    expect(f.arrivals[0].members).toEqual(["", DEFAULT_SOCK]);
    expect(f.tr.take(undefined)?.endpoint).toBe(DEFAULT_SOCK);
    expect(f.tr.take(DEFAULT_SOCK)?.endpoint).toBe(DEFAULT_SOCK);
    expect(f.tr.unresolved()).toEqual([]);
  });

  it("resolver failure (or an unrepresentable entry) keeps that socket per-socket — old behaviour, reported unresolved, not re-resolved every tick", async () => {
    const f = fixture({ resolve: () => Promise.reject(new Error("bridge missing")) });
    f.tr.setRoster([undefined, "/x.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.args()).toEqual([undefined, "/x.sock"]);
    expect(f.tr.unresolved()).toEqual(["", "/x.sock"]);
    f.tr.setRoster([undefined, "/x.sock"]); // 같은 로스터 — 다시 묻지 않는다
    expect(f.resolveCalls.length).toBe(1);
    const g = fixture({ resolve: () => Promise.resolve([null, "/y.sock"]) });
    g.tr.setRoster([undefined, "/y.sock"]);
    g.tr.dispatch(g.launch);
    await flush();
    expect(g.args()).toEqual([undefined, "/y.sock"]);
    expect(g.tr.unresolved()).toEqual([""]);
  });

  it("remove/re-add the same endpoint repeatedly while its original is unresolved: outstanding originals stay 1 and no second original starts", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.calls.length).toBe(1);
    const lives: number[] = [];
    for (let i = 0; i < 4; i++) {
      f.tr.setRoster([]);
      f.tr.dispatch(f.launch);
      await flush();
      expect(f.tr.lifetimeOf("/a.sock")).toBeUndefined(); // 끝점 수명은 끝났다
      f.tr.setRoster(["/a.sock"]);
      f.tr.dispatch(f.launch);
      await flush();
      lives.push(f.tr.lifetimeOf("/a.sock") ?? -1);
      expect({ round: i, outstanding: f.tr.outstanding() }).toEqual({ round: i, outstanding: 1 });
    }
    expect(f.calls.length).toBe(1); // 다시 붙은 워크스페이스가 같은 끝점에 둘째 원 호출을 시작하지 못한다
    expect(new Set(lives).size).toBe(4); // 붙을 때마다 새 수명
  });

  it("the old original finally settles after re-add: wrong-lifetime callback (no arrival, no snapshot), and only then the endpoint launches again", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    f.tr.setRoster([]);
    f.tr.dispatch(f.launch);
    await flush();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    const life = f.tr.lifetimeOf("/a.sock");
    f.calls[0].d.resolve({ surfaces: [{ surface_id: 9 }] });
    await flush();
    expect(f.arrivals).toEqual([]); // 옛 수명의 응답은 새 워크스페이스를 되살리지 못한다
    expect(f.tr.take("/a.sock")).toBeNull();
    expect(f.tr.outstanding()).toBe(0);
    f.tr.dispatch(f.launch);
    expect(f.args()).toEqual(["/a.sock", "/a.sock"]);
    f.calls[1].d.resolve({ surfaces: [] });
    await flush();
    expect(f.arrivals.map((a) => a.lifetime)).toEqual([life]);
  });

  it("frontend deadline and age never release or replace the owned original (D, 60 s, 120 s pass while it hangs)", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    for (const dt of [8_001, 60_000, 120_001]) {
      f.clock.t += dt;
      f.tr.dispatch(f.launch);
      expect({ dt, launched: f.calls.length, outstanding: f.tr.outstanding() }).toEqual({ dt, launched: 1, outstanding: 1 });
    }
  });

  it("token-owned release: an older original settling never releases ownership held by another unsettled original (startup alias hand-over)", async () => {
    const f = fixture();
    const a = deferred();
    const b = deferred();
    // start() 는 원래 소켓 값마다 직렬로 부른다 — 기본(undefined)과 같은 경로의 명시 소켓이 둘 다 상한을 넘겨 미정산이면 같은 데몬에 원 호출이 둘이다
    f.tr.adoptStartup(undefined, a.promise);
    f.tr.adoptStartup(DEFAULT_SOCK, b.promise);
    f.tr.setRoster([undefined, DEFAULT_SOCK]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.calls.length).toBe(0);
    expect(f.tr.outstandingFor(DEFAULT_SOCK)).toBe(2);
    a.resolve({ surfaces: [] });
    await flush();
    f.tr.dispatch(f.launch);
    expect({ launched: f.calls.length, outstanding: f.tr.outstandingFor(DEFAULT_SOCK) }).toEqual({ launched: 0, outstanding: 1 });
    b.resolve({ surfaces: [] });
    await flush();
    f.tr.dispatch(f.launch);
    expect(f.args()).toEqual([DEFAULT_SOCK]);
  });

  it("startup list_surfaces → first periodic dispatch: the unresolved startup original keeps owning its endpoint; its data is start()'s, not an activity arrival", async () => {
    const f = fixture();
    const s = deferred();
    f.tr.adoptStartup("/a.sock", s.promise);
    f.clock.t += 9_000; // 기동의 rpcT 는 D 에서 포기했고 started=true — 첫 주기 틱
    f.tr.setRoster(["/a.sock", "/b.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    expect(f.args()).toEqual(["/b.sock"]);
    expect(f.tr.outstanding()).toBe(2);
    s.resolve({ surfaces: [] });
    await flush();
    expect(f.arrivals.length).toBe(0);
    f.tr.dispatch(f.launch);
    expect(f.args()).toEqual(["/b.sock", "/a.sock"]);
    expect(f.tr.outstanding()).toBe(2);
  });

  it("one latest unconsumed snapshot per member: latest wins, consumed once; a response past D is projected but unavailable to the consumer", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    await flush();
    f.clock.t += 100;
    f.calls[0].d.resolve({ n: 1 });
    await flush();
    f.tr.dispatch(f.launch);
    f.clock.t += 100;
    f.calls[1].d.resolve({ n: 2 });
    await flush();
    expect(f.tr.take("/a.sock")?.value).toEqual({ n: 2 });
    expect(f.tr.take("/a.sock")).toBeNull();
    f.tr.dispatch(f.launch);
    f.clock.t += 8_001;
    f.calls[2].d.resolve({ n: 3 });
    await flush();
    expect(f.arrivals.length).toBe(3);
    expect(f.tr.take("/a.sock")).toBeNull();
  });

  it("settledFor waits only for this round's own flight; a rejected original yields no snapshot; nothing launched this round resolves at once", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock"]);
    f.tr.dispatch(f.launch);
    let done = false;
    void f.tr.settledFor("/a.sock").then(() => {
      done = true;
    });
    await flush();
    expect(done).toBe(false);
    f.calls[0].d.resolve({ v: 1 });
    await flush();
    expect(done).toBe(true);
    expect(f.tr.take("/a.sock")?.value).toEqual({ v: 1 });
    f.tr.dispatch(f.launch);
    let failed = false;
    void f.tr.settledFor("/a.sock").then(() => {
      failed = true;
    });
    f.calls[1].d.reject(new Error("rpc failed"));
    await flush();
    expect(failed).toBe(true);
    expect(f.tr.take("/a.sock")).toBeNull();
    f.tr.dispatch(f.launch); // 3번째 원 호출 — 미정산
    f.tr.dispatch(f.launch); // 소유 중 — 이번 회차에 보낸 것이 없다
    let idle = false;
    void f.tr.settledFor("/a.sock").then(() => {
      idle = true;
    });
    await flush();
    expect(idle).toBe(true);
    expect(f.calls.length).toBe(3);
  });

  it("a superseded roster resolution is never applied — the default is bound once per roster revision, never recomputed mid-flight", async () => {
    const pend: Deferred[] = [];
    const f = fixture({
      resolve: () => {
        const d = deferred();
        pend.push(d);
        return d.promise;
      },
    });
    f.tr.setRoster([undefined]);
    f.tr.dispatch(f.launch);
    f.tr.setRoster([undefined, "/x.sock"]);
    f.tr.dispatch(f.launch);
    pend[1].resolve(["/d2.sock", "/x.sock"]);
    await flush();
    pend[0].resolve(["/d1.sock"]);
    await flush();
    expect(f.args()).toEqual(["/d2.sock", "/x.sock"]);
    expect(f.tr.endpointOf(undefined)).toBe("/d2.sock");
  });

  it("a synchronous launch throw is local to that endpoint", async () => {
    const f = fixture();
    f.tr.setRoster(["/a.sock", "/b.sock"]);
    f.tr.dispatch((arg) => {
      if (arg === "/a.sock") throw new Error("boom");
      return f.launch(arg);
    });
    await flush();
    expect(f.args()).toEqual(["/b.sock"]);
    expect(f.tr.outstanding()).toBe(1);
  });
});

describe("★F-01 U-F05 — 시간 계약(요청 시작 앵커 · T=31/47 · U4-5 9초)", () => {
  const W = (seq: number, state = "waiting"): Record<string, unknown> => ({
    state, raw: state === "working" ? "active" : "idle", reason: state === "working" ? "active footer" : "settled idle", family: "claude",
    grammar_revision: "claude-1", observed_age_secs: 0.5, last_positive_age_secs: 1, stable_age_secs: 10, observation_seq: seq,
    display_epoch: "e1", retained: false, consecutive_rejections: 0,
  });

  it("T = 3P + 2D + E: 31 s macOS / 47 s Windows, independent of socket count", () => {
    expect(activityExpiryMs(8_000)).toBe(31_000);
    expect(activityExpiryMs(16_000)).toBe(47_000);
  });

  it("two skipped launches then a healthy response fits T (macOS and Windows)", () => {
    for (const [d, T] of [[8_000, 31_000], [16_000, 47_000]] as const) {
      const store = new RoleActivityStore(T);
      expect(store.accept("k", 1, 0, d, W(1))).toBe("accepted"); // 요청 A: 0 에 시작해 D 걸림
      const startB = d + 3 * 3_000; // 다음 실행 위상 + 두 번 건너뜀 = 3P
      const doneB = startB + d + 6_000; // 요청 B 의 D + 전경 지연 E
      expect(doneB).toBe(T);
      expect({ d, view: projectActivity(store.get("k"), doneB - 1, T).view }).toEqual({ d, view: "waiting" });
      expect(store.accept("k", 1, startB, doneB, W(2))).toBe("accepted");
      expect({ d, view: projectActivity(store.get("k"), doneB, T).view }).toEqual({ d, view: "waiting" });
    }
  });

  it("request-start expiry, not receipt: a late (past D) response is accepted, and still expires at start + T", () => {
    const store = new RoleActivityStore(31_000);
    expect(store.accept("k", 1, 0, 25_000, W(1))).toBe("accepted");
    expect(projectActivity(store.get("k"), 30_999, 31_000).view).toBe("waiting");
    expect(projectActivity(store.get("k"), 31_000, 31_000).view).toBe("unknown"); // receipt+T(56 s) 가 아니다
    expect(projectActivity(store.get("k"), 31_000, 31_000).text).toContain("응답 만료");
  });

  it("a late response already expired at receipt (received at start + T) is rejected and stores nothing", () => {
    const store = new RoleActivityStore(31_000);
    expect(store.accept("k", 1, 0, 31_000, W(1))).toBe("expired");
    expect(store.get("k")).toBeUndefined();
  });

  it("U4-5: Working whose response is older than 9 s drops the blink (stale, age in the text) on both OSes; T stays the hard expiry", () => {
    for (const T of [31_000, 47_000]) {
      const store = new RoleActivityStore(T);
      expect(store.accept("k", 1, 0, 100, W(1, "working"))).toBe("accepted");
      // conservative age = observed_age_secs (0.5 s) + elapsed since request start
      expect(projectActivity(store.get("k"), 8_500, T).view).toBe("working");
      const st = projectActivity(store.get("k"), 8_501, T);
      expect({ T, view: st.view }).toEqual({ T, view: "stale" });
      expect(st.text).toContain("마지막 관측 10초 전");
      expect(projectActivity(store.get("k"), T - 1, T).view).toBe("stale");
      expect(projectActivity(store.get("k"), T, T).view).toBe("unknown");
    }
  });
});

describe("★F-01 배선 핀 — 소스에만 있는 세 이음매", () => {
  const RS = readFileSync(new URL("../../src-tauri/src/main.rs", import.meta.url), "utf-8");
  const SRC = readFileSync(new URL("./main.ts", import.meta.url), "utf-8");
  const fnBody = (hay: string, head: string): string => {
    const a = hay.indexOf(head);
    expect({ head, found: a >= 0 }).toEqual({ head, found: true });
    return hay.slice(a, hay.indexOf("\n}\n", a));
  };

  it("Tauri resolve_list_endpoints is read-only (existing resolve_socket per roster entry, no RPC/probe) and registered right beside list_surfaces", () => {
    const prod = RS.slice(0, RS.indexOf("#[cfg(test)]\nmod tests {"));
    const body = fnBody(prod, "fn resolve_list_endpoints(endpoints: Vec<Option<String>>)");
    expect(body).toContain("resolve_socket(");
    for (const bad of ["rpc_on", "rpc_full", "rpc_oneshot", "connect_to", "Command::new", "async fn"])
      expect({ bad, present: body.includes(bad) }).toEqual({ bad, present: false });
    const i = prod.indexOf("tauri::generate_handler![");
    const reg = prod.slice(i, i + prod.slice(i).indexOf("\n        ])")).split("\n").map((l) => l.trim());
    expect(reg.indexOf("resolve_list_endpoints,")).toBe(reg.indexOf("list_surfaces,") + 1);
  });

  it("start() hands its list_surfaces original to listTransport.adoptStartup through rpcT's own hook (bounded call kept)", () => {
    const a = SRC.indexOf("async function start() {");
    const start = SRC.slice(a, SRC.indexOf("started = true;", a));
    expect(start).toContain(
      'rpcT(invoke("list_surfaces", { socket: sk }), T_LIST, (original) => listTransport.adoptStartup(sk, original))',
    );
    const r = SRC.slice(SRC.indexOf("const rpcT = "), SRC.indexOf("const listen = "));
    expect(r).toContain("own?.(pr);");
  });

  it("refreshPaneTitles dispatches the list RPC in the activity pass before the busy guard, inside its own outer catch", () => {
    const f = fnBody(SRC, "async function refreshPaneTitles(");
    const call = f.indexOf('pumpRoleActivity((ep) => invoke("list_surfaces", { socket: ep }));');
    expect(call).toBeGreaterThan(0);
    expect(call).toBeLessThan(f.indexOf("if (refreshing) return;"));
    expect(f.slice(f.lastIndexOf("try {", call), call).trim()).toBe("try {");
    expect(f.slice(call).trimStart().startsWith('pumpRoleActivity((ep) => invoke("list_surfaces", { socket: ep }));\n  } catch {')).toBe(true);
    expect(f.indexOf("if (!started) return;")).toBeLessThan(call); // 복원 전에는 보내지 않는다(기동 왕복과 겹침 금지)
  });
});
