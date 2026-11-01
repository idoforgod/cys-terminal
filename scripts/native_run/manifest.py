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

def make(repo,product,diagnostic,branch,mode,installed_candidate=None):
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
    installed_path=None
    if installed_candidate is not None:
        if type(installed_candidate) is not dict or set(installed_candidate)!={'path','sha256'}:raise ValueError('installed candidate identity fields')
        installed_path=installed_candidate['path']
        if type(installed_path) is not str or not re.fullmatch(r'\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml',installed_path):raise ValueError('exact installed workflow path required')
        if installed_path not in extra:raise ValueError('candidate is not a single new workflow addition')
        if sha(git(repo,'show',diagnostic+':'+installed_path))!=installed_candidate['sha256']:raise ValueError('installed workflow digest mismatch')
    if any(not p.startswith('scripts/native_run/') and p not in INSTRUMENTED and p!=installed_path for p in extra):raise ValueError('unexpected diagnostic-only path')
    for p in sorted(extra & INSTRUMENTED):exceptions.append({'path':p,'product_blob':None,'diagnostic_blob':diagnostic_blobs[p],'scope':'cfg(test) instrumentation','review_approval':None})
    workflow=git(repo,'show',product+':.github/workflows/windows-health.yml')
    name='diag-native-feasibility.yml' if mode=='feasibility' else 'diag-native-run.yml'
    if installed_path:name=Path(installed_path).name
    sources={p:sha(git(repo,'show',diagnostic+':'+p)) for p in sorted(diagnostic_blobs) if p.startswith('scripts/native_run/') or p in INSTRUMENTED}
    controllers={name:sources['scripts/native_run/'+name] for name in ('boundary.py','boundary_host.py','controller.py','protocol.py') if 'scripts/native_run/'+name in sources}
    return {'schema':1,'mode':mode,'product_sha':product,'diagnostic_sha':diagnostic,'branch':branch,'workflow_name':name,
            'product_blobs':product_blobs,'instrumentation':exceptions,'diagnostic_sources':sources,'controller_hashes':controllers,'installed_candidate':installed_candidate,'source_workflow_sha256':sha(workflow),
            'counts':COUNTS,'cases':plan() if mode=='main' else [],'resource_plan':cost() if mode=='main' else proposed_plan(),
            'native_inventory':None,'cli_identity':None,'binary_identities':None,'environment':None,'utc_window':None,
            'feasibility_receipts':None,'execution_authorized':False,'freeze_ready':False,'release_verdict':'NONE',
            'unproven':['auth_free_startup','genuine_open_query','no_submit_calibration','network_boundary'],
            'census':'MUST_REDERIVE_AT_FINAL_TIP'}

def workflow(manifest):
    mode=manifest['mode'];branch=manifest['branch'];jobs=matrix() if mode=='main' else [{'job':'feasibility','arm':'feasibility','minutes':30}]
    guard="github.event_name == 'push' && github.ref == 'refs/heads/"+branch+"' && github.run_attempt == 1"
    # External authority must be supplied by the separate E1. No commit embeds its own SHA.
    steps=[{'name':'Start fixed job clock','shell':'pwsh','run':'$p = Join-Path $env:RUNNER_TEMP ("cys-native-start-" + $env:GITHUB_RUN_ID + "-" + $env:GITHUB_RUN_ATTEMPT + ".txt"); if (Test-Path $p) { throw "Clock already exists" }; [IO.File]::WriteAllText($p, [DateTimeOffset]::UtcNow.ToString("o"))'},
           {'uses':'actions/checkout@v4','with':{'ref':'${{ github.sha }}','fetch-depth':0,'persist-credentials':False}},
           {'uses':'actions/setup-python@v5','with':{'python-version':'3.12'}},
           {'name':'Verify external authority before native work','id':'authorize','shell':'pwsh','env':{
               'NATIVE_AUTH_URI':'${{ vars.CYS_NATIVE_AUTH_URI }}','NATIVE_AUTH_SHA256':'${{ vars.CYS_NATIVE_AUTH_SHA256 }}'},
            'run':'python scripts/native_run/job.py authorize --mode '+mode+' --branch '+branch},
           {'uses':'actions/checkout@v4','with':{'ref':'${{ steps.authorize.outputs.product_sha }}','path':'product','fetch-depth':0,'persist-credentials':False}},
           {'uses':'dtolnay/rust-toolchain@stable','with':{'toolchain':'${{ steps.authorize.outputs.rust_toolchain }}'}},
           {'name':'Fixed owned diagnostic job','shell':'pwsh','run':'python scripts/native_run/job.py execute --job ${{ matrix.job }}'},
           {'name':'Retain every artifact, including NULL and inner reds','if':'always()','uses':'actions/upload-artifact@v4',
            'with':{'name':'native-${{ matrix.job }}-${{ github.run_id }}-attempt-${{ github.run_attempt }}','path':'native-evidence/**\n!native-evidence/cargo-target/**\n!native-evidence/binaries/**','if-no-files-found':'error'}}]
    return {'name':'native diagnostic '+mode,'on':{'push':{'branches':[branch]}},'permissions':{'contents':'read'},
            'concurrency':{'group':'native-'+branch.replace('/','-'),'cancel-in-progress':False},
            'jobs':{'native':{'if':guard,'runs-on':'windows-latest','timeout-minutes':'${{ matrix.minutes }}',
                'strategy':{'fail-fast':False,'max-parallel':10 if mode=='main' else 1,'matrix':{'include':jobs}},
                'env':{'PYTHONUTF8':'1','PYTHONDONTWRITEBYTECODE':'1'},'steps':steps}}}

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--repo',required=True);p.add_argument('--product-sha',required=True);p.add_argument('--diagnostic-sha',required=True);p.add_argument('--branch',required=True);p.add_argument('--mode',choices=['main','feasibility'],required=True);p.add_argument('--manifest',required=True);p.add_argument('--workflow',required=True);p.add_argument('--installed-workflow-path');p.add_argument('--installed-workflow-sha256');a=p.parse_args()
    installed=None
    if a.installed_workflow_path or a.installed_workflow_sha256:
        if not (a.installed_workflow_path and a.installed_workflow_sha256):p.error('installed path and digest required together')
        installed={'path':a.installed_workflow_path,'sha256':a.installed_workflow_sha256}
    out=Path(a.workflow)
    if '.github' in [x.lower() for x in out.resolve().parts] and 'workflows' in [x.lower() for x in out.resolve().parts]:raise SystemExit('workflow installation is not authorized')
    if Path(a.manifest).exists() or out.exists():raise SystemExit('refuse to overwrite a manifest/workflow artifact')
    data=make(a.repo,a.product_sha,a.diagnostic_sha,a.branch,a.mode,installed);atomic_json(a.manifest,data);atomic_json(out,workflow(data))

def validate_runtime(raw,expected_digest,authority,repo):
    """Recompute reviewed relationships before native build/fixture code starts."""
    from protocol import decode_json
    if type(expected_digest) is not str or not re.fullmatch('[0-9a-f]{64}',expected_digest) or sha(raw)!=expected_digest:
        raise ValueError('reviewed manifest digest mismatch')
    reviewed=decode_json(raw)
    if type(reviewed) is not dict:raise ValueError('reviewed manifest object required')
    for key in ('product_sha','diagnostic_sha','branch','mode'):
        if reviewed.get(key)!=authority.get(key):raise ValueError('manifest authority mismatch: '+key)
    installed=reviewed.get('installed_candidate')
    if installed is None:raise ValueError('runtime requires the exact installed workflow identity')
    computed=make(repo,authority['product_sha'],authority['diagnostic_sha'],authority['branch'],authority['mode'],installed)
    for key in ('product_blobs','diagnostic_sources','controller_hashes','installed_candidate','source_workflow_sha256','counts','resource_plan'):
        if reviewed.get(key)!=computed[key]:raise ValueError('reviewed source manifest differs: '+key)
    expected_instrumentation=[{k:v for k,v in row.items() if k!='review_approval'} for row in computed['instrumentation']]
    rows=reviewed.get('instrumentation')
    if type(rows) is not list or [{k:v for k,v in row.items() if k!='review_approval'} for row in rows]!=expected_instrumentation:
        raise ValueError('reviewed cfg(test) delta differs')
    if any(type(row.get('review_approval')) is not str or not row['review_approval'].strip() for row in rows):
        raise ValueError('cfg(test) delta lacks external review binding')
    controllers=reviewed['controller_hashes']
    required={'boundary.py','boundary_host.py','controller.py','protocol.py'}
    if set(controllers)!=required or authority.get('probe_config',{}).get('controller_hashes')!=controllers:
        raise ValueError('authority controller manifest differs')
    if git(repo,'rev-parse','HEAD').decode().strip()!=authority['diagnostic_sha']:raise ValueError('runtime checkout identity differs')
    subprocess.run(['git','-C',str(repo),'diff','--quiet','HEAD','--'],check=True)
    extra=git(repo,'ls-files','--others','--exclude-standard','--','scripts/native_run','src').strip()
    if extra:raise ValueError('untracked executable source in runtime checkout')
    for path,expected in reviewed['diagnostic_sources'].items():
        if sha((Path(repo)/path).read_bytes())!=expected:raise ValueError('runtime source bytes differ: '+path)
    return reviewed
