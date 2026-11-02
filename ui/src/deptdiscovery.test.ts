import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { missingKnownWorkspaces, sameSocket } from "./wsreconcile";
import { deptLaunchName, restoreLaunchDecision } from "./deptlabel";

declare const Bun: { Transpiler: new (o: { loader: "ts" }) => { transformSync(code: string): string } };
const src = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
const tr = new Bun.Transpiler({ loader: "ts" });
function fn(name: string): string {
  const m = new RegExp(`^(?:async )?function ${name}\\(`, "m").exec(src);
  if (!m) return "";
  return src.slice(m.index, src.indexOf("\n}\n", m.index) + 2);
}
const socket = "/fixture/cys-dept-sales/cys.sock";
function discoveryRig(override?: (name: string) => Promise<unknown>) {
  const calls: string[] = [];
  let reg: any = { depts: {} };
  let tombs: string[] | null = [];
  const deps = {
    invoke: async (name: string) => {
      calls.push(name);
      if (override && (name === "list_depts" || name === "dept_tombstones")) return override(name);
      if (name === "list_depts") return reg;
      if (name === "dept_tombstones") { if (tombs === null) throw Error("unavailable"); return tombs; }
      if (name === "org_status") return {};
      if (name === "ensure_dept_forwarders") return;
      throw Error(`Unexpected RPC ${name}`);
    },
    rpcT: async (p: Promise<unknown>) => p,
    T_REG: 1, T_LIST: 1, DEFAULT_SOCKET_KEY: "base", UNTITLED: "untitled",
    pendingBySocket: new Map(), nodeSig: new Map(), seatAccts: new Map(),
    claimFlight: () => true, releaseFlightWhenSettled: () => {},
    updatePendingBadges: () => {}, renderWsTabs: () => {}, refreshAccountsShared: () => {}, renderUsageBar: () => {},
    saveLayout: () => {}, missingKnownWorkspaces, sameSocket, deptLaunchName, restoreLaunchDecision,
  };
  const body = `let workspaces = []; let groups = []; let wsCounter = 1; let pendingApprovals = 0;
    let started = true; let factoryResetting = false; let resetCompleted = false;
    let deptDiscoveryInFlight = false; let deptDiscoveryLastAttempt = -Infinity; let deptLaunchInFlight = false;
    ${tr.transformSync(fn("refreshRegisteredWorkspaces"))}
    ${tr.transformSync(fn("refreshSidebarStatus"))}
    return {tick:refreshSidebarStatus, discover:typeof refreshRegisteredWorkspaces === "function" ? refreshRegisteredWorkspaces : async()=>{}, rows:()=>workspaces,
      reset:()=>{resetCompleted=true}, clear:()=>{workspaces=[]}, next:()=>{deptDiscoveryLastAttempt=-Infinity},
      seed:(rows)=>{workspaces=rows}, ready:()=>{started=true}, notReady:()=>{started=false}};`;
  const api = new Function(...Object.keys(deps), body)(...Object.values(deps));
  return { ...api, calls, registry: (r: any) => {reg=r}, tombs: (t: string[] | null) => {tombs=t} };
}
async function settle() { for(let i=0;i<20;i++) await Promise.resolve(); }

describe("A61 department discovery", () => {
  it("existing sidebar tick discovers a newly registered department without restart", async () => {
    const r = discoveryRig();
    r.registry({depts:{sales:{socket,display_name:"Sales"}}});
    await r.tick(); await settle();
    expect(r.rows().map((w: any)=>w.socket)).toEqual([socket]);
    expect(r.calls).not.toContain("launch_dept_daemon");
  });
  it("startup retains an unregistered offline tab for later passive adoption", async () => {
    const a=src.indexOf("  const deptWsList = workspaces.filter");
    const b=src.indexOf("  // 무음 생략 금지",a);
    expect(a).toBeGreaterThan(0); expect(b).toBeGreaterThan(a);
    const deps={restoreLaunchDecision, sameSocket, deptLaunchName, rpcT:async(p:Promise<unknown>)=>p,
      invoke:async(name:string)=>{throw Error(name)},T_STATUS:1,daemonIdentOf:()=>({}),startHaltedByReset:()=>false};
    const f=new Function(...Object.keys(deps),tr.transformSync(`async function run(){
      let workspaces=[{id:1,name:"Sales",tree:null,socket:${JSON.stringify(socket)}}];
      const ghosts=new Set(); const registered=new Set(); const regDepts={}; const deptTombs=new Set();
      const restoreDeadline=Infinity; const identBySock=new Map(); const info={};
      let budgetLaunch=0; let tombReapErrCount=0; let launched=0; let cappedLaunch=0; let unnamedLaunch=0;
      let tombUnknownLaunch=0; let failedLaunch=0; let firstLaunchErr=null; const MAX_DEPT_LAUNCH_PER_START=2;
      ${src.slice(a,b)}
      return workspaces;
    }`)+"return run;")(...Object.values(deps));
    expect((await f()).length).toBe(1);
  });
  it("does not recreate tombstoned departments or the deliberately closed base tab", async () => {
    const r=discoveryRig(); r.registry({depts:{sales:{socket}}}); r.tombs(["sales"]);
    await r.tick(); await settle(); expect(r.rows()).toEqual([]);
  });
  it("does not create departments when deletion records are unavailable", async () => {
    const r=discoveryRig(); r.registry({depts:{sales:{socket}}}); r.tombs(null);
    await r.tick(); await settle(); expect(r.rows()).toEqual([]);
  });
  it("reset completion prevents registry discovery", async () => {
    const r=discoveryRig(); r.registry({depts:{sales:{socket}}}); r.reset();
    await r.tick(); await settle(); expect(r.rows()).toEqual([]);
  });
  it("serializes overlapping reads until both RPCs settle, including an early rejection", async () => {
    let release!: (v: unknown) => void;
    const pending = new Promise((resolve) => {release=resolve});
    const r=discoveryRig((name)=>name==="list_depts" ? Promise.reject(Error("read failed")) : pending);
    const first=r.discover(); await settle(); r.next(); await r.discover();
    expect(r.calls.filter((n:string)=>n==="list_depts").length).toBe(1);
    release([]); await first;
    expect(r.rows()).toEqual([]);
  });
  it("does not restore a tab explicitly closed while the registry reads were pending", async () => {
    let release!: (v: unknown) => void;
    const pending = new Promise((resolve) => {release=resolve});
    const r=discoveryRig((name)=>name==="list_depts" ? pending : Promise.resolve([]));
    r.seed([{id:8,socket,name:"Sales",tree:null}]);
    const first=r.discover(); r.clear(); release({depts:{sales:{socket}}}); await first;
    expect(r.rows()).toEqual([]);
  });
  it("failed read can recover on a later tick without losing saved candidates", async () => {
    const r=discoveryRig(); r.seed([{id:8,socket,name:"Saved",tree:null}]);
    r.tombs(null); await r.discover(); expect(r.rows().length).toBe(1);
    r.next(); r.tombs([]); r.registry({depts:{sales:{socket},other:{socket:"/fixture/cys-dept-other/cys.sock"}}});
    await r.discover(); expect(r.rows().length).toBe(2);
  });
  it("does not duplicate named-pipe aliases or revive an alias tombstone", async () => {
    const pipe="\\\\.\\pipe\\cys-dept-Sales";
    const r=discoveryRig(); r.seed([{id:8,socket:pipe,name:"Saved",tree:null}]);
    r.registry({depts:{Sales:{socket:pipe.toUpperCase()}}}); await r.discover();
    expect(r.rows().length).toBe(1);
    const t=discoveryRig(); t.registry({depts:{sales:{socket:pipe}}}); t.tombs(["Sales"]);
    await t.discover(); expect(t.rows()).toEqual([]);
  });
  it("keeps discovery disabled until startup restore finishes", async () => {
    const r=discoveryRig(); r.notReady(); r.registry({depts:{sales:{socket}}});
    await r.discover(); expect(r.calls).toEqual([]); expect(r.rows()).toEqual([]);
  });

  it("A70: the startup view deadline is pending, not a launch failure", async () => {
    const a=src.indexOf("  const deptWsList = workspaces.filter");
    const b=src.indexOf("  // 무음 생략 금지",a);
    const deps={restoreLaunchDecision,sameSocket,deptLaunchName,
      rpcT:async(p:Promise<unknown>,ms:number)=>{if(ms===2)return p;throw Error("rpc timeout")},
      invoke:async(name:string)=>{if(name==="daemon_status") throw Error("offline");return new Promise(()=>{})},
      T_STATUS:2,T_LAUNCH:1,daemonIdentOf:()=>({}),startHaltedByReset:()=>false,
      watchDeptOperation:async(_name:string,_kind:string,start:()=>Promise<unknown>)=>{void start();return {state:"pending"}},
      applyDeptLaunchInfo:()=>{}};
    const f=new Function(...Object.keys(deps),tr.transformSync(`async function run(){
      let workspaces=[{id:1,name:"Sales",tree:null,socket:${JSON.stringify(socket)}}];
      const ghosts=new Set(); const registered=new Set([${JSON.stringify(socket)}]);
      const regDepts={sales:{socket:${JSON.stringify(socket)}}}; const deptTombs=new Set();
      const restoreDeadline=Infinity; const identBySock=new Map(); const info={};
      let budgetLaunch=0; let tombReapErrCount=0; let launched=0; let cappedLaunch=0; let unnamedLaunch=0;
      let tombUnknownLaunch=0; let failedLaunch=0; let pendingLaunch=0; let firstLaunchErr=null; const MAX_DEPT_LAUNCH_PER_START=2;
      ${src.slice(a,b)}
      return {failedLaunch,pendingLaunch};
    }`)+"return run;")(...Object.values(deps));
    expect(await f()).toEqual({failedLaunch:0,pendingLaunch:1});
  });

  it("A70: late launch updates only its still-open tab and never resurrects a removed object", () => {
    const rows:any[]=[]; const slugs=new Map(); let rendered=0;
    const deps={workspaces:rows, socketForSlug:slugs, renderWsTabs:()=>{rendered++}};
    const api=new Function(...Object.keys(deps),`let factoryResetting=false;let resetCompleted=false;${tr.transformSync(fn("applyDeptLaunchInfo"))};return {apply:applyDeptLaunchInfo,reset:()=>{resetCompleted=true}}`)(...Object.values(deps));
    const ws:any={id:1,name:"Sales",socket:"old",tree:null,stopFailed:true,autoCreated:true};
    rows.push(ws);api.apply(ws,{socket:"new",socket_slug:"slug"},true);
    expect(ws.socket).toBe("new");expect(ws.stopFailed).toBeUndefined();expect(ws.autoCreated).toBeUndefined();expect(rendered).toBe(1);
    rows.length=0;api.apply(ws,{socket:"removed"});expect(ws.socket).toBe("new");expect(rows).toEqual([]);
    rows.push(ws);ws.deleting=true;api.apply(ws,{socket:"deleting"});expect(ws.socket).toBe("new");
    ws.deleting=false;api.reset();api.apply(ws,{socket:"reset"});expect(ws.socket).toBe("new");expect(rendered).toBe(1);
  });

  it("A70: pending native operation remains a reset gate input after the UI walk returns", async () => {
    const deps={bootGateClosed:async()=>false,deptOperations:{busy:()=>true}};
    const f=new Function(...Object.keys(deps),`let startRestoringSince=null;let rotatingDaemon=false;let purgingDept=false;let factoryResetting=false;let resetCompleted=false;${tr.transformSync(fn("resetGateInput"))};return resetGateInput;`)(...Object.values(deps));
    expect((await f()).busy).toBe(true);
  });
  it("A70: pending native operation blocks purge before any preview or destructive RPC", async () => {
    const calls:string[]=[];
    const deps={daemonActionBlocked:()=>false,deptOperations:{busy:()=>true},toast:()=>{},stickyToast:()=>{},dismissToast:()=>{},invoke:async(name:string)=>{calls.push(name);throw Error("no real backend")}};
    const f=new Function(...Object.keys(deps),tr.transformSync(fn("purgeDept"))+";return purgeDept;")(...Object.values(deps));
    await f({socket:"fixture"});expect(calls).toEqual([]);
  });

});
