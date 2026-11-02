import { describe,it,expect } from "bun:test";
import { readFileSync } from "node:fs";
import { DepartmentOperations } from "./deptoperations";
import { sameSocket } from "./wsreconcile";
declare const Bun:{Transpiler:new(o:{loader:"ts"})=>{transformSync(s:string):string}};
const src=readFileSync(new URL('./main.ts',import.meta.url),'utf8');const tr=new Bun.Transpiler({loader:'ts'});
function fn(name:string){const m=new RegExp(`^(?:async )?function ${name}(?:<[^>]+>)?\\(`,'m').exec(src);if(!m)throw Error(name);return src.slice(m.index,src.indexOf('\n}\n',m.index)+2)}
function deferred<T>(){let resolve!:(v:T)=>void,reject!:(e:unknown)=>void;const promise=new Promise<T>((a,b)=>{resolve=a;reject=b});return{resolve,reject,promise}}
async function flush(){for(let i=0;i<30;i++)await Promise.resolve()}
function rig(){
 const timers=new Map<number,()=>void>();let tid=0;const calls:string[]=[];const notices:unknown[][]=[];
 const resolve=deferred<{name:string;regQueried:boolean}>();const launch=deferred<unknown>();
 const ws:any={id:1,socket:'/fixture/cys-dept-first/cys.sock',tree:null};
 let click!:()=>Promise<void>;
 const btn={disabled:false,textContent:'지금 켜기',addEventListener:(_n:string,f:()=>Promise<void>)=>{click=f}};
 const scope:any={factoryResetting:false,resetCompleted:false,purgingDept:false,rotatingDaemon:false,IS_WINDOWS:false,T_LAUNCH:60_000,T_REG:1,
   workspaces:[ws],ws,btn,host:{isConnected:true},isDept:true,socketForSlug:new Map(),sameSocket,
   deptOperations:new DepartmentOperations((f)=>{timers.set(++tid,f);return tid},id=>{timers.delete(id as number)}),
   resolveDeptLaunchName:()=>resolve.promise, rpcT:(p:Promise<unknown>)=>p,
   invoke:(cmd:string)=>{calls.push(cmd);return launch.promise},
   toast:(...a:unknown[])=>notices.push(a),render:()=>{},renderWsTabs:()=>{},refreshPaneTitles:()=>{},
   rotationRefused:()=>false,noteRotationBlocked:()=>{},
 };
 const idle=fn('renderIdleWorkspace');const a=idle.indexOf('  btn.addEventListener("click", async () => {');const b=idle.indexOf('\n  box.append(msg, btn);',a);
 expect(a).toBeGreaterThan(0);expect(b).toBeGreaterThan(a);
 const code=tr.transformSync(fn('watchDeptOperation')+'\n'+fn('applyDeptLaunchInfo')+'\n'+idle.slice(a,b));
 const api=new Function('scope',`with(scope){${code};return {watch:watchDeptOperation}}`)(scope);
 return {scope,btn,ws,calls,notices,resolve,launch,api,click:()=>click(),fire:()=>{const f=[...timers.values()];timers.clear();f.forEach(x=>x())}};
}
describe('A70 review: idle wake and pending settlement',()=>{
 it('name lookup yielding to reset must not launch afterwards',async()=>{
  const r=rig();const click=r.click();await flush();r.scope.factoryResetting=true;r.resolve.resolve({name:'first',regQueried:true});await flush();
  expect(r.calls).toEqual([]);r.fire();await click;
 });
 it('central owner refuses launches during reset or purge, but not its authorized rotating caller',async()=>{
  for(const flag of ['factoryResetting','resetCompleted','purgingDept']){
   const r=rig();r.scope[flag]=true;let started=0;
   await r.api.watch('first','launch',async()=>{started++;return {}}).catch(()=>{});expect(started).toBe(0);
  }
  const r=rig();r.scope.rotatingDaemon=true;expect(await r.api.watch('first','rotate',async()=>({}))).toEqual({state:'done',value:{}});
 });
 it('expiry followed by actual late error enables the still-visible wake button for retry',async()=>{
  const r=rig();r.resolve.resolve({name:'first',regQueried:true});const click=r.click();await flush();r.fire();await click;
  expect(r.btn.disabled).toBe(false);expect(r.btn.textContent).toBe('진행 확인');r.launch.reject(Error('late native error'));await flush();
  expect(r.btn.disabled).toBe(false);expect(r.btn.textContent).toBe('다시 시도');
 });
 it('a reset starting between observation and the invocation microtask still prevents launch',async()=>{
  const r=rig();let starts=0;const view=r.api.watch('first','launch',async()=>{starts++;return {}}).catch(()=>{});
  r.scope.resetCompleted=true;await view;expect(starts).toBe(0);
 });
 it('a deleting target refuses a new launch, while an independent target remains usable',async()=>{
  const r=rig();r.ws.deleting=true;let starts=0;
  await r.api.watch('first','launch',async()=>{starts++;return {}},undefined,'',false,{socket:r.ws.socket}).catch(()=>{});
  expect(starts).toBe(0);
  expect(await r.api.watch('other','launch',async()=>{starts++;return {}},undefined,'',false,{socket:'/fixture/other.sock'})).toEqual({state:'done',value:{}});
  expect(starts).toBe(1);
 });

 it('a second idle view of startup-owned work remains usable and never launches a duplicate',async()=>{
  const r=rig();const original=deferred<unknown>();
  const first=r.scope.deptOperations.observe('dept:first',()=>original.promise,60_000,()=>{}, {kind:'department',socket:r.ws.socket});
  await flush();r.fire();await first;r.resolve.resolve({name:'first',regQueried:true});
  await r.click();expect(r.calls).toEqual([]);expect(r.btn.disabled).toBe(false);expect(r.btn.textContent).toBe('진행 확인');
  original.reject(Error('startup failed'));await flush();expect(r.btn.disabled).toBe(false);
  const retry=r.click();await flush();expect(r.calls).toEqual(['launch_dept_daemon']);r.launch.resolve({socket:r.ws.socket});await retry;
 });

});
