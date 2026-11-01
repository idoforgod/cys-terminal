"""Bounded native setup, invoked only inside the owned diagnostic worker.

CLI binary URL/version/hash are frozen external inputs; no moving alias or
installer is executed. The native build uses the diagnostic checkout, while
its manifest proves the product-path differences separately.
"""
from pathlib import Path
import json
import os
import subprocess
import sys
import urllib.parse
import urllib.request
from protocol import atomic_json,sha
from boundary import Unavailable

def build_command(args,receipt,**kwargs):
    row={'argv':args,'timeout_s':kwargs.get('timeout'),'returncode':None,'state':'started'}
    atomic_json(receipt,row)
    try:
        result=subprocess.run(args,**kwargs)
        row.update(returncode=result.returncode,state='returned')
        return result
    except Exception as error:
        row.update(state='exception',exception=type(error).__name__,error=str(error))
        raise
    finally:atomic_json(receipt,row)

def build_probe(doc,evidence):
    if sys.platform!='win32':raise Unavailable('native setup requires Windows')
    root=Path(evidence).resolve();binary=root/'binaries';binary.mkdir()
    config=doc.get('probe_config')
    if type(config) is not dict:raise Unavailable('frozen probe configuration absent')
    required=('receiver_url','receiver_sha256','receiver_version','canary_literal_address','controller_hashes')
    if any(k not in config for k in required):raise Unavailable('frozen receiver/setup inputs absent')
    url=urllib.parse.urlparse(config['receiver_url'])
    if url.scheme!='https' or url.username or url.password or url.fragment:raise Unavailable('explicit HTTPS binary source required')
    # Exact source URL is reviewed/frozen in the external authority. No redirect
    # or unpinned package install substitutes for that identity.
    from job import NoRedirect
    target=binary/'receiver.exe';digest=__import__('hashlib').sha256();count=0
    with urllib.request.build_opener(NoRedirect).open(config['receiver_url'],timeout=15) as source,target.open('xb') as dest:
        while True:
            block=source.read(1024*1024)
            if not block:break
            count+=len(block)
            if count>512*1024*1024:raise Unavailable(f'receiver binary size bound: bytes={count} limit={512*1024*1024}')
            dest.write(block);digest.update(block)
    if digest.hexdigest()!=config['receiver_sha256']:raise Unavailable('receiver binary hash mismatch: '+json.dumps({'expected':config['receiver_sha256'],'observed':digest.hexdigest(),'bytes':count,'path':str(target)}))
    # Never run --version before confinement. Identity here is the frozen hash.
    atomic_json(root/'receiver-identity.json',{'url':config['receiver_url'],'version_claim':config['receiver_version'],
        'sha256':digest.hexdigest(),'bytes':count,'executed_unconfined':False})
    compiler=Path(os.environ['SystemRoot'])/'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    if not compiler.is_file():raise Unavailable('pinned system C# compiler location unavailable: '+str(compiler))
    helper=binary/'receiver-helper.exe'
    args=[str(compiler),'/nologo','/target:exe','/platform:x64','/reference:System.Web.Extensions.dll','/out:'+str(helper),str(Path(__file__).with_name('receiver_helper.cs').resolve())]
    with (root/'helper-build.log').open('wb') as log:
        compiled=build_command(args,root/'helper-build-command.json',stdout=log,stderr=subprocess.STDOUT,timeout=60)
    if compiled.returncode:raise Unavailable(f'helper build failed: returncode={compiled.returncode}; see helper-build-command.json and helper-build.log')
    # Fresh per-job target. Full output is retained and JSON parsed only after rc0.
    build_env=dict(os.environ);build_env['CARGO_TARGET_DIR']=str(root/'cargo-target');build_env['CARGO_INCREMENTAL']='0'
    build_env['CARGO_BUILD_JOBS']='4'
    args=['cargo','test','--locked','--bin','cysd','--no-run','--message-format=json']
    with (root/'daemon-build.jsonl').open('wb') as out,(root/'daemon-build.stderr').open('wb') as err:
        built=build_command(args,root/'daemon-build-command.json',env=build_env,stdout=out,stderr=err,timeout=600)
    if built.returncode:raise Unavailable(f'diagnostic daemon build failed: returncode={built.returncode}; see daemon-build-command.json, daemon-build.jsonl and daemon-build.stderr')
    matches=[]
    for line in (root/'daemon-build.jsonl').read_text(encoding='utf-8').splitlines():
        row=json.loads(line)
        if row.get('reason')=='compiler-artifact' and row.get('target',{}).get('name')=='cysd' and row.get('profile',{}).get('test') is True and row.get('executable'):
            matches.append(row['executable'])
    if len(matches)!=1:raise Unavailable('diagnostic daemon artifact ambiguous: '+json.dumps({'matches':matches,'count':len(matches)}))
    daemon=Path(matches[0]);daemon_sha=sha(daemon.read_bytes())
    resolved={**config,'product_sha':doc['product_sha'],'diagnostic_sha':doc['diagnostic_sha'],
        'receiver_executable':str(target),'receiver_helper':str(helper),'receiver_helper_sha256':sha(helper.read_bytes()),
        'daemon_test_executable':str(daemon),'daemon_test_sha256':daemon_sha}
    atomic_json(root/'resolved-probe-config.json',resolved)
    atomic_json(root/'compiler-identity.json',{'csc_sha256':sha(compiler.read_bytes()),'daemon_sha256':daemon_sha,
        'helper_sha256':resolved['receiver_helper_sha256'],'release_verdict':'NONE'})
    return resolved
