import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { validateInputs, inspectInstallers, verdict } from './measure-contract.mjs';
const hash = b => crypto.createHash('sha256').update(b).digest('hex');
const lock = () => ({ reading:'R1', plan_sha256:'ddf2c693c7e93598593a3f06c7c2e77aa8dd8b538efe5baf64fec9bdc8c3ee44', base_url:'https://github.com/idoforgod/cys-terminal/releases/download/v0.14.52/cys_0.14.52_x64-setup.exe', base_sha256:hash('base'), candidate_sha256:hash('candidate'), candidate_name:'cys_0.14.53_x64-setup.exe', product_sha:'a'.repeat(40), run_id:123, run_attempt:1, artifact_id:456, artifact_name:'cys-windows-x64-nsis' });
const good = () => ({ reading:'R1', input:true, baseline:true, installer_exit:0, versions:true, daemon:true, pack:true, seat:true, tracebacks:0, logs_complete:true, evidence_complete:true, harness_error:null });
test('valid pinned inputs accepted',()=>assert.equal(validateInputs(lock()).reading,'R1'));
for (const [key,value] of [['candidate_sha256',''],['run_id',0],['product_sha','main'],['reading','R3'],['base_url','https://evil.invalid/base.exe'],['candidate_name','another.exe']]) test('reject invalid '+key,()=>assert.throws(()=>validateInputs({...lock(),[key]:value}),/UNMEASURED_INPUT/));
test('candidate missing refuses before any installation authorization',()=>assert.throws(()=>inspectInstallers(lock(),'/absent/base','/absent/candidate'),/UNMEASURED_INPUT/));
test('candidate hash mismatch refuses; valid pair accepted',()=>{
 const d=fs.mkdtempSync(path.join(os.tmpdir(),'winmeasure-input-')); try {
  const b=path.join(d,'base.exe'),c=path.join(d,lock().candidate_name);fs.writeFileSync(b,'base');fs.writeFileSync(c,'wrong');
  assert.throws(()=>inspectInstallers(lock(),b,c),/UNMEASURED_INPUT.*hash/);
  fs.writeFileSync(c,'candidate');assert.equal(inspectInstallers(lock(),b,c).candidate_sha256,lock().candidate_sha256);
 } finally {fs.rmSync(d,{recursive:true,force:true});}
});
test('R1 requires every measured cell',()=>assert.equal(verdict(good()).status,'PASS'));
for(const key of ['baseline','versions','daemon','pack','seat']) test('R1 false '+key+' fails',()=>assert.equal(verdict({...good(),[key]:false}).status,'FAIL'));
for(const key of ['installer_exit','daemon','pack','seat','tracebacks']) test('R1 unknown '+key+' is not pass',()=>assert.equal(verdict({...good(),[key]:null}).status,'UNMEASURED'));
test('Traceback hit fails',()=>assert.equal(verdict({...good(),tracebacks:1}).status,'FAIL'));
test('missing logs do not count as zero',()=>assert.equal(verdict({...good(),logs_complete:false}).status,'UNMEASURED'));
test('observer failure is unmeasured',()=>assert.equal(verdict({...good(),harness_error:'lost'}).status,'UNMEASURED'));
const r2=()=>({...good(),reading:'R2',sac_before:true,sac_after:true,neg_blocked:true,pos_allowed:true,launch_known:true,launch_blocked:true,old_restart:true,old_survives:true,files_changed:false,state_complete:true,restored:true});
test('SAC block old usable is measured, never PASS',()=>assert.equal(verdict(r2()).status,'MEASURED_BLOCKED_OLD_USABLE'));
test('SAC block old cannot restart distinguished',()=>assert.equal(verdict({...r2(),old_restart:false}).status,'MEASURED_BLOCKED_OLD_NOT_RESTARTABLE'));
test('SAC replacement mixed is measured',()=>assert.equal(verdict({...r2(),files_changed:true,versions:false,daemon:false}).status,'MEASURED_REPLACED_BROKEN_OR_MIXED'));
for(const key of ['sac_before','sac_after','neg_blocked','pos_allowed','state_complete','restored']) test('R2 unknown/off '+key+' unmeasured',()=>assert.equal(verdict({...r2(),[key]:false}).status,'UNMEASURED'));

test('metadata binds final product SHA, run attempt and artifact id',async()=>{
 const {validateMetadata}=await import('./measure-contract.mjs'); const x=lock();
 const run={id:x.run_id,run_attempt:1,head_sha:x.product_sha,status:'completed',conclusion:'success',path:'.github/workflows/windows-build.yml'};
 const artifacts=[{id:x.artifact_id,name:x.artifact_name,expired:false,size_in_bytes:100}];
 assert.equal(validateMetadata(x,run,artifacts).artifact.id,456);
 for(const delta of [{head_sha:'b'.repeat(40)},{run_attempt:2},{conclusion:'failure'},{path:'.github/workflows/other.yml'}])assert.throws(()=>validateMetadata(x,{...run,...delta},artifacts),/UNMEASURED_INPUT/);
 for(const delta of [{expired:true},{id:457},{size_in_bytes:0}])assert.throws(()=>validateMetadata(x,run,[{...artifacts[0],...delta}]),/UNMEASURED_INPUT/);
 assert.throws(()=>validateMetadata(x,run,[...artifacts,...artifacts]),/UNMEASURED_INPUT/);
});
test('blocked installer with absent log remains unmeasured',()=>assert.equal(verdict({...r2(),logs_complete:false}).status,'UNMEASURED'));
test('incomplete post-state must not manufacture measured SAC result',()=>assert.equal(verdict({...r2(),old_restart:null}).status,'UNMEASURED'));
test('R2 requires established baseline, even for definitive candidate block',()=>assert.equal(verdict({...r2(),baseline:false}).status,'UNMEASURED'));
test('R2 missing installer exit after successful launch is unmeasured',()=>assert.equal(verdict({...r2(),launch_blocked:false,installer_exit:null}).status,'UNMEASURED'));
test('a similarly prefixed workflow is not the Windows build',async()=>{
 const {validateMetadata}=await import('./measure-contract.mjs');const x=lock();
 assert.throws(()=>validateMetadata(x,{id:x.run_id,run_attempt:1,head_sha:x.product_sha,status:'completed',conclusion:'success',path:'.github/workflows/windows-build.yml.evil'},[{id:x.artifact_id,name:x.artifact_name,size_in_bytes:1,expired:false}]),/UNMEASURED_INPUT/);
});
test('R2 copied files with unknown pack or seat cannot be called measured mixed',()=>assert.equal(verdict({...r2(),files_changed:true,pack:null}).status,'UNMEASURED'));
