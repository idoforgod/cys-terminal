import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';
const PLAN='ddf2c693c7e93598593a3f06c7c2e77aa8dd8b538efe5baf64fec9bdc8c3ee44';
const BASE='https://github.com/idoforgod/cys-terminal/releases/download/v0.14.52/cys_0.14.52_x64-setup.exe';
const fail=s=>{throw new Error('UNMEASURED_INPUT: '+s);};
export const sha256=b=>crypto.createHash('sha256').update(b).digest('hex');
export function validateInputs(x){
 if(!x||!['R1','R2'].includes(x.reading)||x.plan_sha256!==PLAN)fail('reading/plan lock');
 if(x.base_url!==BASE||x.candidate_name!=='cys_0.14.53_x64-setup.exe'||x.artifact_name!=='cys-windows-x64-nsis')fail('asset identity');
 for(const k of ['base_sha256','candidate_sha256'])if(!/^[a-f0-9]{64}$/.test(x[k]||'')||/^0+$/.test(x[k]))fail(k);
 if(!/^[a-f0-9]{40}$/.test(x.product_sha||''))fail('product_sha');
 for(const k of ['run_id','run_attempt','artifact_id'])if(!Number.isSafeInteger(x[k])||x[k]<1)fail(k);
 return x;
}
export function inspectInstallers(input,base,candidate){
 const x=validateInputs(input);
 if(path.basename(candidate)!==x.candidate_name)fail('candidate filename');
 for(const [f,k] of [[base,'base_sha256'],[candidate,'candidate_sha256']]){
  let b;try{if(!fs.statSync(f).isFile())fail('not a regular file');b=fs.readFileSync(f);}catch{fail('missing installer');}
  if(!b.length||sha256(b)!==x[k])fail(k+' hash mismatch');
 }
 return {...x,base_path:path.resolve(base),candidate_path:path.resolve(candidate),authorized:true};
}
export function verdict(x){
 const un=[];const bad=[];
 if(x.harness_error)un.push(x.harness_error);
 for(const k of ['input','logs_complete','evidence_complete'])if(x[k]!==true)un.push(k);
 if(x.reading==='R1'&&x.input===true&&x.evidence_complete===true&&!x.harness_error){
  if(x.baseline===false)return {status:'FAIL',reasons:['baseline']};
  if(x.baseline===true&&x.launch_known===true&&x.launch_ok===false)return {status:'FAIL',reasons:['installer_launch']};
 }
 if(x.reading==='R2'){
  if(x.baseline!==true)un.push('baseline');
  if(x.launch_blocked!==true&&!Number.isSafeInteger(x.installer_exit))un.push('installer_exit');
  for(const k of ['sac_before','sac_after','neg_blocked','pos_allowed','state_complete','restored','launch_known'])if(x[k]!==true)un.push(k);
  if(typeof x.old_restart!=='boolean'||typeof x.files_changed!=='boolean')un.push('old/file state');
  for(const k of ['versions','daemon','pack','seat'])if(typeof x[k]!=='boolean')un.push(k);
  if(!Number.isSafeInteger(x.tracebacks))un.push('tracebacks');
  if(un.length)return {status:'UNMEASURED',reasons:un};
  if(x.files_changed&&(x.versions!==true||x.daemon!==true||x.pack!==true||x.seat!==true))return {status:'MEASURED_REPLACED_BROKEN_OR_MIXED',reasons:[]};
  if(x.launch_blocked===true)return {status:x.old_restart?'MEASURED_BLOCKED_OLD_USABLE':'MEASURED_BLOCKED_OLD_NOT_RESTARTABLE',reasons:[]};
  if(x.files_changed!==true)return {status:'UNMEASURED',reasons:['updated outcome without changed files']};
  const normal=verdict({...x,reading:'R1'});
  return {status:normal.status==='PASS'?'MEASURED_UPDATED_USABLE':normal.status==='UNMEASURED'?'UNMEASURED':'MEASURED_OTHER_FAILURE',reasons:normal.reasons};
 }
 if(x.reading!=='R1')un.push('reading');
 for(const k of ['baseline','versions','daemon','pack','seat']){
  if(typeof x[k]!=='boolean')un.push(k);else if(!x[k])bad.push(k);
 }
 for(const k of ['installer_exit','tracebacks'])if(!Number.isSafeInteger(x[k])||x[k]<0)un.push(k);else if(x[k]!==0)bad.push(k);
 return {status:un.length?'UNMEASURED':bad.length?'FAIL':'PASS',reasons:[...un,...bad]};
}
export function validateMetadata(x,run,artifacts){
 validateInputs(x);
 if(run.id!==x.run_id||run.run_attempt!==x.run_attempt||run.head_sha!==x.product_sha||run.status!=='completed'||run.conclusion!=='success'||String(run.path).split('@')[0]!=='.github/workflows/windows-build.yml')fail('build run identity/status');
 const a=artifacts.filter(a=>a.name===x.artifact_name);
 if(a.length!==1||a[0].id!==x.artifact_id||a[0].expired||a[0].size_in_bytes<=0)fail('artifact identity/expiry');
 return {run,artifact:a[0]};
}
async function main(){
 const [mode,inputFile,...args]=process.argv.slice(2);const x=JSON.parse(fs.readFileSync(inputFile,'utf8'));
 if(mode==='validate'){validateInputs(x);console.log('input lock valid');}
 else if(mode==='prepare'){
  validateInputs(x);const repo=process.env.GITHUB_REPOSITORY;if(repo!=='idoforgod/cys-terminal')fail('repository');
  const api=async s=>{const r=await fetch('https://api.github.com/repos/'+repo+s,{headers:{Authorization:'Bearer '+process.env.GITHUB_TOKEN,Accept:'application/vnd.github+json'},signal:AbortSignal.timeout(30000)});if(!r.ok)fail('GitHub metadata HTTP '+r.status);return r.json();};
  const run=await api('/actions/runs/'+x.run_id+'/attempts/'+x.run_attempt);
  const arts=await api('/actions/runs/'+x.run_id+'/artifacts?per_page=100');if(arts.total_count>100)fail('artifact pagination required');
  const proof=validateMetadata(x,run,arts.artifacts);fs.writeFileSync(args[0],JSON.stringify(proof,null,2)+'\n');
  fs.appendFileSync(process.env.GITHUB_OUTPUT,`reading=${x.reading}\nrunner=${x.reading==='R1'?'windows-2025':'windows-11-arm'}\nrun_id=${x.run_id}\nartifact_name=${x.artifact_name}\nartifact_id=${x.artifact_id}\n`);
 }else if(mode==='authorize'){
  const [base,candidate,out]=args;fs.writeFileSync(out,JSON.stringify(inspectInstallers(x,base,candidate),null,2)+'\n');
 }else if(mode==='verdict'){
  const out=verdict(x);fs.writeFileSync(args[0],JSON.stringify(out,null,2)+'\n');console.log(JSON.stringify(out));
 }else fail('unknown mode');
}
if(process.argv[1]&&import.meta.url===pathToFileURL(path.resolve(process.argv[1])).href)main().catch(e=>{console.error(e.message);process.exitCode=2;});
