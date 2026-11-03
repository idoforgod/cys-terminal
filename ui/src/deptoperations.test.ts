import { describe, it, expect } from "bun:test";
import { DepartmentOperations, type DeptOperationOutcome } from "./deptoperations";
import { sameSocket } from "./wsreconcile";

function rig() {
  const timers = new Map<number, () => void>(); let next = 0;
  const owner = new DepartmentOperations((f) => {const id=++next;timers.set(id,f);return id}, (id)=>{timers.delete(id as number)});
  const fire = () => { const fs=[...timers.values()]; timers.clear(); for(const f of fs)f(); };
  return {owner, fire, timers};
}
async function flush() {for(let i=0;i<10;i++)await Promise.resolve()}
function deferred<T>() {
  let resolve!: (v:T)=>void; let reject!: (e:unknown)=>void;
  const promise = new Promise<T>((yes,no)=>{resolve=yes;reject=no});
  return {promise,resolve,reject};
}

describe("A70 native-operation ownership outlives the UI view", () => {
  it("expiry is pending, repeated views never relaunch, and late success is watched once", async () => {
    const r=rig();const op=deferred<number>();let starts=0;const late:DeptOperationOutcome<number>[]=[];
    const first=r.owner.observe("dept:a",()=>{starts++;return op.promise},60_000,(v)=>late.push(v));
    await flush();r.fire();expect(await first).toEqual({state:"pending"});expect(r.owner.has("dept:a")).toBe(true);
    expect(await r.owner.observe("dept:a",async()=>{starts++;return 9},60_000,()=>{})).toEqual({state:"pending"});
    expect(starts).toBe(1);expect(late).toEqual([]);
    op.resolve(7);await flush();expect(late).toEqual([{ok:true,value:7}]);expect(r.owner.has("dept:a")).toBe(false);
    expect(await r.owner.observe("dept:a",async()=>9,60_000,()=>{})).toEqual({state:"done",value:9});
    expect(r.timers.size).toBe(0);
  });
  it("a late failure is observed and only true settlement releases ownership",async()=>{
    const r=rig();const op=deferred<number>();const late:DeptOperationOutcome<number>[]=[];
    const first=r.owner.observe("dept:a",()=>op.promise,60_000,(v)=>late.push(v));
    await flush();r.fire();await first;expect(r.owner.has("dept:a")).toBe(true);
    op.reject("real error");await flush();expect(late).toEqual([{ok:false,error:"real error"}]);expect(r.owner.has("dept:a")).toBe(false);
  });
  it("immediate errors still reject, never become success or pending",async()=>{
    const r=rig();let caught:unknown;try{await r.owner.observe("dept:a",()=>{throw Error("sync failure")},60_000,()=>{})}catch(e){caught=e}
    expect(String(caught)).toContain("sync failure");expect(r.owner.has("dept:a")).toBe(false);expect(r.timers.size).toBe(0);
  });
  it("base and a department have independent ownership, without cancelling either",async()=>{
    const r=rig();const base=deferred<void>();let later=0;
    const first=r.owner.observe("base",()=>base.promise,60_000,()=>{});await flush();r.fire();await first;
    expect(await r.owner.observe("dept:base",async()=>++later,60_000,()=>{})).toEqual({state:"done",value:1});
    expect(r.owner.has("base")).toBe(true);base.resolve();await flush();expect(r.owner.has("base")).toBe(false);
  });
  it("late display callback failure cannot retain the operation or retry it",async()=>{
    const r=rig();const op=deferred<void>();const first=r.owner.observe("dept:a",()=>op.promise,60_000,()=>{throw Error("display")});
    await flush();r.fire();await first;op.resolve();await flush();expect(r.owner.has("dept:a")).toBe(false);
  });
  it("R2: target metadata belongs to the original owner and Windows aliases remain blocked",async()=>{
    const r=rig();const op=deferred<void>();const socket="\\\\.\\pipe\\cys-dept-first";const other="\\\\.\\pipe\\cys-dept-other";
    const first=r.owner.observe("dept:first",()=>op.promise,60_000,()=>{}, {kind:"department",socket});
    await flush();r.fire();await first;
    await r.owner.observe("dept:first",async()=>{},60_000,()=>{}, {kind:"department",socket:other});
    expect(r.owner.blocksClose(socket.toUpperCase(),sameSocket)).toBe(true);
    expect(r.owner.blocksClose(other,sameSocket)).toBe(false);
    expect(r.owner.blocksClose(undefined,sameSocket)).toBe(false);
    op.resolve();await flush();expect(r.owner.blocksClose(socket,sameSocket)).toBe(false);
  });
  it("R2: unknown department targets refuse department close conservatively; base remains independent",async()=>{
    const r=rig();const op=deferred<void>();const first=r.owner.observe("dept:unknown",()=>op.promise,60_000,()=>{}, {kind:"department"});
    await flush();r.fire();await first;
    expect(r.owner.blocksClose("/fixture/other.sock",sameSocket)).toBe(true);
    expect(r.owner.blocksClose(undefined,sameSocket)).toBe(false);op.resolve();await flush();
    const base=deferred<void>();const second=r.owner.observe("base",()=>base.promise,60_000,()=>{}, {kind:"base"});
    await flush();r.fire();await second;expect(r.owner.blocksClose(undefined,sameSocket)).toBe(true);
    expect(r.owner.blocksClose("/fixture/other.sock",sameSocket)).toBe(false);base.resolve();await flush();
  });

});
