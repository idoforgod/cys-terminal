import fs from 'node:fs';import path from 'node:path';import {finalize} from './evidence-integrity.mjs';
const root=path.resolve(process.argv[2]);
try{console.log(JSON.stringify(finalize(root)));}catch(e){
 const r={status:'UNMEASURED',reasons:['finalization failed: '+e.message]};
 try{fs.writeFileSync(path.join(root,'verdict.json'),JSON.stringify(r,null,2)+'\n');}catch{}
 console.error(r.reasons[0]);process.exitCode=2;
}
