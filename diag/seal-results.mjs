// Never trust a pre-existing PASS: validate mandatory records and captured bytes again.
import fs from 'node:fs';import path from 'node:path';import {sha256} from './measure-contract.mjs';import {sealedVerdict} from './evidence-integrity.mjs';
const root=path.resolve(process.argv[2]);fs.mkdirSync(root,{recursive:true});
const result=sealedVerdict(root);
fs.writeFileSync(path.join(root,'verdict.json'),JSON.stringify(result,null,2)+'\n');
const walk=d=>fs.readdirSync(d,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(d,e.name)):[path.join(d,e.name)]);
const files=walk(root).filter(f=>path.basename(f)!=='SHA256SUMS').sort();
fs.writeFileSync(path.join(root,'SHA256SUMS'),files.map(f=>sha256(fs.readFileSync(f))+'  '+path.relative(root,f).split(path.sep).join('/')).join('\n')+'\n');
console.log(JSON.stringify(result));
