// Local-only checks. Never contacts Actions or opens a remote result.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
const root=path.resolve('diag');
const base='https://github.com/idoforgod/cys-terminal/releases/download/v0.14.52/cys_0.14.52_x64-setup.exe';
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
const d=fs.mkdtempSync(path.join(root,'local-checks/input-control-'));
const rows=[];
try{
 const b=path.join(d,'base.exe'),c=path.join(d,'cys_0.14.53_x64-setup.exe'),p=path.join(d,'permit.json'),mark=path.join(d,'install-marker');
 fs.writeFileSync(b,'base');
 const x={...JSON.parse(fs.readFileSync('diag/win-measure-input.json')),base_url:base,base_sha256:hash('base'),candidate_sha256:hash('candidate'),product_sha:'a'.repeat(40),run_id:1,run_attempt:1,artifact_id:1};
 fs.writeFileSync(path.join(d,'input.json'),JSON.stringify(x));
 for(const mode of ['missing','hash-mismatch','valid']){
  if(mode==='hash-mismatch')fs.writeFileSync(c,'wrong');if(mode==='valid')fs.writeFileSync(c,'candidate');
  const r=spawnSync(process.execPath,['diag/measure-contract.mjs','authorize',path.join(d,'input.json'),b,c,p],{encoding:'utf8'});
  if(r.status===0)fs.writeFileSync(mark,'installation would begin only now');
  rows.push({case:mode,exit:r.status,stderr:r.stderr,permit_exists:fs.existsSync(p),installation_reached:fs.existsSync(mark)});
  if(mode!=='valid'&&(r.status!==2||fs.existsSync(p)||fs.existsSync(mark)))throw new Error('negative case bypassed: '+mode);
  if(mode==='valid'&&(r.status!==0||!fs.existsSync(mark)))throw new Error('positive control refused');
 }
 console.log(JSON.stringify({platform:os.platform(),native_windows:'UNMEASURED',cases:rows},null,2));
}finally{fs.rmSync(d,{recursive:true,force:true});}
