import fs from 'node:fs';import path from 'node:path';import {sha256,verdict} from './measure-contract.mjs';import {driverCells} from './driver-observation.mjs';
export const CHANNELS=['daemon:file','daemon:stdout','daemon:stderr','pack:file','pack:stdout','pack:stderr','app:file','app-base:stdout','app-base:stderr','app-updated:stdout','app-updated:stderr','installer-base:stdout','installer-base:stderr','installer-candidate:stdout','installer-candidate:stderr'];
export function requiredRecords(run){
 const common=['measurement.json','input-lock.json','permit.json','build-metadata.json','base-state.json','base-raw-state.json','base-logs.json'];
 if(run.baseline_timeout===true)return common;
 return [...common,'launch.json','installer-process-evidence.json','after-state.json','after-raw-state.json','final-state.json','final-raw-state.json','after-seat.json','final-logs.json','log-inventory.json','channel-capture.json',...(run.seed_cells?.reading==='R2'?['r2-state.json']:[])];
}
function safe(root,name){if(path.basename(name)!==name||name==='.'||name==='..')throw Error('invalid evidence name');return path.join(root,name);}
export function writeChecked(root,name,obj,receipts){
 const f=safe(root,name),text=JSON.stringify(obj,null,2)+'\n';fs.writeFileSync(f,text);
 const b=fs.readFileSync(f);JSON.parse(b);if(!b.length||!b.equals(Buffer.from(text)))throw Error('write/readback mismatch '+name);
 receipts[name]={sha256:sha256(b),bytes:b.length,format:'json'};return receipts[name];
}
function recordShape(name,j){
 const tri=x=>x&&typeof x.known==='boolean'&&(x.known?typeof x.value==='boolean':x.value===null&&typeof x.error==='string'&&x.error.length>0);
 if(['base-state.json','after-state.json','final-state.json'].includes(name)&&!['versions','daemon','pack'].every(k=>tri(j[k])))throw Error('state criterion schema');
 if(name.endsWith('-raw-state.json')&&(!j.expected||!Object.hasOwn(j,'error')))throw Error('raw observation schema');
 if(name==='after-seat.json'&&!tri(j))throw Error('seat criterion schema');
 if(name==='launch.json'&&(typeof j.completed!=='boolean'||typeof j.ok!=='boolean'))throw Error('launch schema');
 if(name==='installer-process-evidence.json'){
  if(j.not_launched===true){if(j.exit!==null||j.launch?.completed!==true||j.launch?.ok!==false)throw Error('refusal evidence schema');}
  else if(j.complete!==true||!Number.isInteger(j.exit)||!Array.isArray(j.events)||!j.events.length||!Array.isArray(j.roots)||j.roots.length!==1)throw Error('native installer exit proof missing');
 }
 if(name.endsWith('-logs.json')&&!Array.isArray(j.files))throw Error('log file inventory schema');
 if(name==='measurement.json'&&!['R1','R2'].includes(j.seed_cells?.reading))throw Error('measurement identity schema');
}
export function verifyRecords(root,names,receipts){
 const errors=[];
 for(const n of names)try{const b=fs.readFileSync(safe(root,n)),r=receipts[n];if(!b.length||!r||r.bytes!==b.length||r.sha256!==sha256(b))throw Error('absent/empty/hash mismatch');const j=JSON.parse(b);if(!j||typeof j!=='object')throw Error('not an evidence object');recordShape(n,j);}catch(e){errors.push(n+': '+e.message);}
 return {complete:errors.length===0,errors};
}
export function logCoverage(inventory,capture,root){
 const errors=[];let tracebacks=0;
 if(!inventory||inventory.product_sha!==capture?.product_sha||!/^[a-f0-9]{40}$/.test(inventory.product_sha||'')||!/^[a-f0-9]{64}$/.test(inventory.reviewed_source_sha256||'')||!capture?.interval||!Number.isFinite(capture.interval.start)||!Number.isFinite(capture.interval.end)||capture.interval.end<capture.interval.start)return {complete:false,tracebacks:null,errors:['unbound inventory/interval']};
 const required=[...CHANNELS,...(capture.reading==='R2'?['app-old-restart:stdout','app-old-restart:stderr']:[])];
 for(const id of required){
  const binds=(inventory.channels||[]).filter(x=>x.id===id),rows=(capture.channels||[]).filter(x=>x.id===id);
  if(binds.length!==1||rows.length!==1){errors.push(id+': absent/duplicate');continue;}
  const b=binds[0],r=rows[0];
  try{
   if(!b.source_ref||!b.sink||r.bound!==true||!Number.isInteger(r.producer_pid)||r.producer_pid<=0||r.sink!==b.sink||!Number.isFinite(r.producer_start)||!Number.isFinite(r.producer_end)||r.producer_end<r.producer_start||r.producer_start<capture.interval.start||r.producer_end>capture.interval.end||!Number.isFinite(r.start)||!Number.isFinite(r.end)||r.start>r.producer_start||r.end<r.producer_end||r.collection_ok!==true)throw Error('unbound producer/channel/window');
   if(!Number.isInteger(r.offset)||!Number.isInteger(r.end_offset)||r.offset<0||r.end_offset<r.offset)throw Error('offset');
   if(!path.resolve(root,r.copy).startsWith(path.resolve(root)+path.sep))throw Error('outside evidence');const full=fs.readFileSync(path.resolve(root,r.copy));
   if(full.length!==r.bytes||sha256(full)!==r.sha256||r.end_offset>full.length)throw Error('capture hash/length');
   const bytes=full.subarray(r.offset,r.end_offset);
   const encoding=bytes.length>=2&&bytes[0]===255&&bytes[1]===254?'utf16le':'utf8';
   if(bytes.length>=2&&bytes[0]===254&&bytes[1]===255)throw Error('UTF16BE capture requires explicit decoder');
   if(encoding==='utf8'&&bytes.includes(0))throw Error('NUL-containing capture has no bound text encoding');
   tracebacks+=(bytes.toString(encoding).match(/Traceback/g)||[]).length;
  }catch(e){errors.push(id+': '+e.message);}
 }
 return {complete:errors.length===0,tracebacks:errors.length?null:tracebacks,errors};
}
export function finalize(root){
 const read=n=>JSON.parse(fs.readFileSync(safe(root,n),'utf8'));
 const run=read('measurement.json'),receipts=read('evidence-receipts.json');
 const proof=verifyRecords(root,requiredRecords(run),receipts);const cells=driverCells(run);
 if(!run.baseline_timeout)try{const audit=read('installer-process-evidence.json'),launch=read('launch.json');if(launch.ok!==cells.launch_ok||(launch.ok&&audit.exit!==cells.installer_exit))throw Error('driver/audit mismatch');}catch(e){proof.complete=false;proof.errors.push(e.message);}
 let coverage={complete:false,tracebacks:null,errors:['no required capture']};
 try{coverage=logCoverage(read('log-inventory.json'),read('channel-capture.json'),root);}catch(e){coverage.errors=[e.message];}
 cells.logs_complete=coverage.complete;cells.tracebacks=coverage.tracebacks;cells.evidence_complete=proof.complete;
 if(!proof.complete)cells.harness_error='required evidence invalid: '+proof.errors.join('; ');
 writeChecked(root,'log-coverage.json',coverage,receipts);writeChecked(root,'cells.json',cells,receipts);
 const mandatory=[...requiredRecords(run),'cells.json','log-coverage.json'];
 writeChecked(root,'mandatory-artifacts.json',{required:mandatory,records:Object.fromEntries(mandatory.map(n=>[n,receipts[n]]))},receipts);
 const result=verdict(cells);writeChecked(root,'verdict.json',result,receipts);
 fs.writeFileSync(safe(root,'evidence-receipts.json'),JSON.stringify(receipts,null,2)+'\n');return result;
}
export function sealedVerdict(root){
 try{
  const run=JSON.parse(fs.readFileSync(safe(root,'measurement.json'))),m=JSON.parse(fs.readFileSync(safe(root,'mandatory-artifacts.json')));
  const required=[...requiredRecords(run),'cells.json','log-coverage.json'];
  if(JSON.stringify([...m.required].sort())!==JSON.stringify([...required].sort()))throw Error('mandatory set mismatch');
  const proof=verifyRecords(root,required,m.records);if(!proof.complete)throw Error(proof.errors.join('; '));
  const cells=JSON.parse(fs.readFileSync(safe(root,'cells.json')));
  // Recheck captured bytes too: a later missing stream invalidates a prior zero.
  if(!run.baseline_timeout){const cov=logCoverage(JSON.parse(fs.readFileSync(safe(root,'log-inventory.json'))),JSON.parse(fs.readFileSync(safe(root,'channel-capture.json'))),root);if(!cov.complete){cells.logs_complete=false;cells.tracebacks=null;}else if(cov.tracebacks!==cells.tracebacks)throw Error('Traceback count changed');}
  return verdict(cells);
 }catch(e){return {status:'UNMEASURED',reasons:['evidence integrity: '+e.message]};}
}
