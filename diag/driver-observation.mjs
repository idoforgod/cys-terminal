// Production adapter: the Windows driver and local fault injections use this exact reducer.
import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
export const known=value=>({known:true,value,error:null});
export const unknown=error=>({known:false,value:null,error:String(error||'missing observation')});
export function all(values){if(values.some(x=>!x||x.known!==true||typeof x.value!=='boolean'))return unknown(values.filter(x=>x?.known!==true).map(x=>x?.error||'missing').join('; '));return known(values.every(x=>x.value));}
export const value=x=>x?.known===true&&typeof x.value==='boolean'?x.value:null;
export function daemonVersion(text,version){const escaped=version.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');return new RegExp('^cysd '+escaped+' \\(build [0-9a-f]+\\)$').test(String(text).trim());}
export function processObservation(p){
 if(!p||!Number.isInteger(p.rc)||p.startError||p.timedOut)return unknown('process exit/start observation unavailable');
 // A nonzero observed exit is a failure without interpreting stdout.
 if(p.rc!==0)return known(false);
 if(p.stdout_complete!==true||p.pipes_not_drained!==false||p.stdout_error||p.stdout_truncated===true||typeof p.out!=='string')return unknown('stdout incomplete, undrained, failed decoding/read, truncated or absent');
 return known(true);
}
export function stateObservation(raw){
 if(raw.error)return {versions:unknown(raw.error),daemon:unknown(raw.error),pack:raw.pack??unknown('pack')};
 let daemon=unknown('CIM/identify unavailable');
 if(raw.cim?.known===true&&Array.isArray(raw.cim.daemons)){
  if(!raw.cim.daemons.length)daemon=known(false);
  else if(processObservation(raw.identify).known&&raw.identify?.rc===0&&raw.identify.data&&processObservation(raw.ping).known&&raw.ping?.rc===0){const id=raw.identify.data;daemon=known(id.version===raw.expected&&raw.cim.daemons.some(p=>p.ProcessId===id.daemon_pid));}
 }
 let app=unknown(raw.app?.error||'CDP unavailable');
 if(raw.cim?.known===true&&Array.isArray(raw.cim.apps)&&!raw.cim.apps.length)app=known(false);
 else if(raw.app?.known===true&&raw.app.data){const a=raw.app.data;app=known(a.attached===true&&a.app_version===raw.expected&&a.ui?.ready?.ok===true);}
 let binaries=unknown('binary version observation unavailable');
 const procKnown=p=>processObservation(p).known;
 if(raw.images&&procKnown(raw.cli)&&procKnown(raw.daemon_binary))binaries=known(['cys-app.exe','cys.exe','cysd.exe'].every(n=>raw.images[n]?.version===raw.expected)&&raw.marker===raw.expected&&raw.failure_file===false&&raw.cli.rc===0&&String(raw.cli.out).trim()==='cys '+raw.expected&&raw.daemon_binary.rc===0&&daemonVersion(raw.daemon_binary.out,raw.expected));
 // A directly observed missing file is independent negative evidence, but cannot hide
 // an exited-successfully process whose required stdout was lost.
 const lostSuccessfulOutput=[raw.cli,raw.daemon_binary].some(p=>p?.rc===0&&!processObservation(p).known);
 if(!lostSuccessfulOutput&&raw.images&&['cys-app.exe','cys.exe','cysd.exe'].some(n=>raw.images[n]===null))binaries=known(false);
 return {versions:all([binaries,app]),daemon,pack:raw.pack??unknown('pack')};
}
export function driverCells(run){
 const c={...(run.seed_cells||{})};c.baseline=value(all([run.baseline?.versions,run.baseline?.daemon,run.baseline?.pack]));
 c.versions=value(all([run.after?.versions,run.final_state?.versions]));c.daemon=value(all([run.after?.daemon,run.final_state?.daemon]));c.pack=value(all([run.after?.pack,run.final_state?.pack]));c.seat=value(run.seat);
 const p=run.old_restart_probe;
 if(c.files_changed===true)c.old_restart=false; // old image no longer installed; not a restart success.
 else if(p?.completed===true&&p.ok===false)c.old_restart=false;
 else if(p?.completed===true&&p.ok===true)c.old_restart=value(all([run.old_restart_state?.versions,run.old_restart_state?.daemon,run.old_restart_state?.pack,run.old_restart_seat]));
 else c.old_restart=null;
 c.state_complete=['versions','daemon','pack','seat','old_restart','files_changed'].every(k=>typeof c[k]==='boolean');
 return c;
}
if(process.argv[1]&&import.meta.url===pathToFileURL(path.resolve(process.argv[1])).href){const [mode,input,out]=process.argv.slice(2);const x=JSON.parse(fs.readFileSync(input));fs.writeFileSync(out,JSON.stringify(mode==='state'?stateObservation(x):mode==='process'?processObservation(x):driverCells(x),null,2)+'\n');}
