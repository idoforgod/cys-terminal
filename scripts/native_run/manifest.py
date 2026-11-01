"""Parameterised draft manifest and UNINSTALLED workflow rendering.

Final authority is external to the diagnostic commit. Branch labels are inputs,
never derived from the hash of the commit that contains their own filter.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
from protocol import plan,matrix,cost,sha,atomic_json,COUNTS
from probe import proposed_plan

INSTRUMENTED={'src/bin/cysd/main.rs','src/bin/cysd/state.rs','src/bin/cysd/native_harness_trace.rs'}

def git(repo,*args):return subprocess.check_output(['git','-C',str(repo),*args])
def full_sha(value):
    if type(value) is not str or not re.fullmatch('[0-9a-f]{40}',value):raise ValueError('full immutable SHA required')
    return value

def make(repo,product,diagnostic,branch,mode):
    full_sha(product);full_sha(diagnostic)
    if re.fullmatch(r'work/[A-Za-z0-9][A-Za-z0-9._/-]*',branch) is None:raise ValueError('literal work/ branch required')
    subprocess.run(['git','check-ref-format','refs/heads/'+branch],check=True,capture_output=True)
    subprocess.run(['git','-C',str(repo),'merge-base','--is-ancestor',product,diagnostic],check=True)
    def entries(ref):
        result={}
        for row in git(repo,'ls-tree','-rz',ref).split(b'\0'):
            if row:
                meta,path=row.split(b'\t',1);mode,kind,oid=meta.decode().split();result[path.decode()]={'mode':mode,'type':kind,'oid':oid}
        return result
    product_blobs=entries(product);diagnostic_blobs=entries(diagnostic);exceptions=[]
    for p,old in product_blobs.items():
        new=diagnostic_blobs.get(p)
        if old!=new:
            if p not in INSTRUMENTED:raise ValueError('unapproved product path changed: '+p)
            if new is None or old['mode']!=new['mode'] or old['type']!=new['type']:raise ValueError('product mode/type/deletion changed')
            exceptions.append({'path':p,'product_blob':old,'diagnostic_blob':new,'scope':'cfg(test) instrumentation','review_approval':None})
    extra=set(diagnostic_blobs)-set(product_blobs)
    if any(not p.startswith('scripts/native_run/') and p not in INSTRUMENTED for p in extra):raise ValueError('unexpected diagnostic-only path')
    for p in sorted(extra & INSTRUMENTED):exceptions.append({'path':p,'product_blob':None,'diagnostic_blob':diagnostic_blobs[p],'scope':'cfg(test) instrumentation','review_approval':None})
    workflow=git(repo,'show',product+':.github/workflows/windows-health.yml')
    name='diag-native-feasibility.yml' if mode=='feasibility' else 'diag-native-run.yml'
    return {'schema':1,'mode':mode,'product_sha':product,'diagnostic_sha':diagnostic,'branch':branch,'workflow_name':name,
            'product_blobs':product_blobs,'instrumentation':exceptions,'source_workflow_sha256':sha(workflow),
            'counts':COUNTS,'cases':plan() if mode=='main' else [],'resource_plan':cost() if mode=='main' else proposed_plan(),
            'native_inventory':None,'cli_identity':None,'binary_identities':None,'environment':None,'utc_window':None,
            'feasibility_receipts':None,'execution_authorized':False,'freeze_ready':False,'release_verdict':'NONE',
            'unproven':['auth_free_startup','genuine_open_query','no_submit_calibration','network_boundary'],
            'census':'MUST_REDERIVE_AT_FINAL_TIP'}

def workflow(manifest):
    mode=manifest['mode'];branch=manifest['branch'];jobs=matrix() if mode=='main' else [{'job':'feasibility','arm':'feasibility','minutes':30}]
    guard="github.event_name == 'push' && github.ref == 'refs/heads/"+branch+"' && github.run_attempt == 1"
    # External authority must be supplied by the separate E1. No commit embeds its own SHA.
    steps=[{'uses':'actions/checkout@v4','with':{'ref':'${{ github.sha }}','fetch-depth':0,'persist-credentials':False}},
           {'uses':'actions/setup-python@v5','with':{'python-version':'3.12'}},
           {'uses':'dtolnay/rust-toolchain@stable'},
           {'name':'Verify external authority before native work','shell':'pwsh','env':{
               'NATIVE_AUTH_URI':'${{ vars.CYS_NATIVE_AUTH_URI }}','NATIVE_AUTH_SHA256':'${{ vars.CYS_NATIVE_AUTH_SHA256 }}'},
            'run':'python scripts/native_run/job.py authorize --mode '+mode+' --branch '+branch},
           {'name':'Fixed owned diagnostic job','shell':'pwsh','run':'python scripts/native_run/job.py execute --job ${{ matrix.job }}'},
           {'name':'Retain every artifact, including NULL and inner reds','if':'always()','uses':'actions/upload-artifact@v4',
            'with':{'name':'native-${{ matrix.job }}-${{ github.run_id }}-attempt-${{ github.run_attempt }}','path':'native-evidence/','if-no-files-found':'error'}}]
    return {'name':'native diagnostic '+mode,'on':{'push':{'branches':[branch]}},'permissions':{'contents':'read'},
            'concurrency':{'group':'native-'+branch.replace('/','-'),'cancel-in-progress':False},
            'jobs':{'native':{'if':guard,'runs-on':'windows-latest','timeout-minutes':'${{ matrix.minutes }}',
                'strategy':{'fail-fast':False,'max-parallel':10 if mode=='main' else 1,'matrix':{'include':jobs}},
                'env':{'PYTHONUTF8':'1','PYTHONDONTWRITEBYTECODE':'1'},'steps':steps}}}

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--repo',required=True);p.add_argument('--product-sha',required=True);p.add_argument('--diagnostic-sha',required=True);p.add_argument('--branch',required=True);p.add_argument('--mode',choices=['main','feasibility'],required=True);p.add_argument('--manifest',required=True);p.add_argument('--workflow',required=True);a=p.parse_args()
    out=Path(a.workflow)
    if '.github' in [x.lower() for x in out.resolve().parts] and 'workflows' in [x.lower() for x in out.resolve().parts]:raise SystemExit('workflow installation is not authorized')
    if Path(a.manifest).exists() or out.exists():raise SystemExit('refuse to overwrite a manifest/workflow artifact')
    data=make(a.repo,a.product_sha,a.diagnostic_sha,a.branch,a.mode);atomic_json(a.manifest,data);atomic_json(out,workflow(data))
