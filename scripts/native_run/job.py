"""Workflow entry points. No native operation before external one-run authority.

This file does not install a workflow, push, dispatch, rerun, cancel, or publish.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.parse
import urllib.request
from protocol import atomic_json, decode_json, sha, read_json
from boundary import Unavailable

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):raise Unavailable('authority redirect refused')

def checked_authority(raw,expected,context):
    if not re.fullmatch('[0-9a-f]{64}',expected or '') or sha(raw)!=expected:raise Unavailable('external authority digest mismatch')
    doc=decode_json(raw)
    if type(doc) is not dict:raise Unavailable('authority object')
    for key in ('product_sha','diagnostic_sha'):
        if type(doc.get(key)) is not str or not re.fullmatch('[0-9a-f]{40}',doc[key]):raise Unavailable('immutable source identity')
    if doc.get('execution_authorized') is not True or doc.get('release_verdict')!='NONE':raise Unavailable('authority not granted')
    if type(doc.get('attempt')) is not int or doc['attempt']!=1 or context.get('attempt')!='1':raise Unavailable('attempt must be exactly1')
    if doc.get('mode')!=context['mode'] or doc.get('branch')!=context['branch']:raise Unavailable('authority mode/ref')
    if context.get('event')!='push' or context.get('ref')!='refs/heads/'+doc['branch']:raise Unavailable('not authorized branch push')
    if context.get('sha')!=doc['diagnostic_sha'] or context.get('runner_os')!='Windows':raise Unavailable('checkout/host mismatch')
    if doc.get('host')!='windows-latest':raise Unavailable('standard runner required')
    ref=doc.get('manifest')
    if type(ref) is not dict or set(ref)!={'url','sha256'} or type(ref.get('url')) is not str or type(ref.get('sha256')) is not str or not re.fullmatch('[0-9a-f]{64}',ref.get('sha256','')):
        raise Unavailable('digest-bound reviewed manifest required')
    if type(doc.get('rust_toolchain')) is not str or not re.fullmatch(r'\d+\.\d+\.\d+',doc['rust_toolchain']):raise Unavailable('exact toolchain version required')
    now=datetime.datetime.now(datetime.timezone.utc)
    start=datetime.datetime.fromisoformat(doc['earliest_start_utc']);end=datetime.datetime.fromisoformat(doc['latest_end_utc'])
    if start.tzinfo is None or end.tzinfo is None or not start<=now<end:raise Unavailable('outside approved UTC window')
    return doc

def fetch_manifest(reference):
    parsed=urllib.parse.urlparse(reference['url'])
    if parsed.scheme!='https' or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:raise Unavailable('external HTTPS manifest required')
    with urllib.request.build_opener(NoRedirect).open(reference['url'],timeout=10) as response:raw=response.read(16*1024*1024+1)
    if len(raw)>16*1024*1024 or sha(raw)!=reference['sha256']:raise Unavailable('manifest size/digest mismatch')
    return raw

def bind_manifest(doc,raw):
    from manifest import validate_runtime
    return validate_runtime(raw,doc['manifest']['sha256'],doc,Path.cwd())

def job_start_utc():
    run=os.environ.get('GITHUB_RUN_ID','');attempt=os.environ.get('GITHUB_RUN_ATTEMPT','')
    if not run.isdigit() or attempt!='1':raise Unavailable('job clock identity absent')
    stamp=(Path(os.environ['RUNNER_TEMP'])/('cys-native-start-'+run+'-'+attempt+'.txt')).read_text().strip()
    began=datetime.datetime.fromisoformat(stamp)
    now=datetime.datetime.now(datetime.timezone.utc)
    if began.tzinfo is None or not 0<=(now-began).total_seconds()<600:raise Unavailable('setup cap expired or invalid job clock')
    return stamp

def authorize(mode,branch):
    root=Path('native-evidence')
    if root.exists():raise Unavailable('evidence root already exists; no attempt adoption')
    root.mkdir()
    context={'mode':mode,'branch':branch,'attempt':os.environ.get('GITHUB_RUN_ATTEMPT'),
             'event':os.environ.get('GITHUB_EVENT_NAME'),'ref':os.environ.get('GITHUB_REF'),
             'sha':os.environ.get('GITHUB_SHA'),'runner_os':os.environ.get('RUNNER_OS')}
    uri=os.environ.get('NATIVE_AUTH_URI','');parsed=urllib.parse.urlparse(uri)
    if parsed.scheme!='https' or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        raise Unavailable('external HTTPS authority required')
    expected=os.environ.get('NATIVE_AUTH_SHA256','')
    if not re.fullmatch('[0-9a-f]{64}',expected):raise Unavailable('authority digest absent')
    opener=urllib.request.build_opener(NoRedirect)
    with opener.open(uri,timeout=10) as response:raw=response.read(2*1024*1024+1)
    if len(raw)>2*1024*1024:raise Unavailable('authority too large')
    doc=checked_authority(raw,expected,context)
    actual=subprocess.check_output(['git','rev-parse','HEAD']).decode().strip()
    if actual!=doc['diagnostic_sha']:raise Unavailable('working checkout not approved diagnostic SHA')
    manifest_raw=fetch_manifest(doc['manifest']);bind_manifest(doc,manifest_raw)
    (root/'manifest.json').write_bytes(manifest_raw)
    (root/'authority.json').write_bytes(raw)
    atomic_json(root/'authorization-receipt.json',{'digest':expected,'context':context,'authority_host':parsed.hostname,
                'run_id':os.environ.get('GITHUB_RUN_ID'),'job_start_utc':job_start_utc(),'release_verdict':'NONE'})
    with open(os.environ['GITHUB_OUTPUT'],'a',encoding='utf-8') as out:
        out.write('product_sha='+doc['product_sha']+'\n')
        out.write('rust_toolchain='+doc['rust_toolchain']+'\n')
    return doc

def execute(job):
    root=Path('native-evidence');receipt=read_json(root/'authorization-receipt.json');raw=(root/'authority.json').read_bytes()
    doc=checked_authority(raw,receipt['digest'],receipt['context'])
    bind_manifest(doc,(root/'manifest.json').read_bytes())
    # Execution driver is installed only when complete and reviewed. A partial
    # local WIP never substitutes simulation or returns a successful empty job.
    if doc['mode']=='feasibility':
        if job!='feasibility':raise Unavailable('unexpected feasibility matrix entry')
        from supervisor import run_owned_worker
        from archive import collect
        try:
            return run_owned_worker([str(Path(__file__).resolve()),'worker','--evidence',str(root.resolve())],root,doc['latest_end_utc'],receipt['job_start_utc'])
        finally:
            archived=collect(root)
            if archived.get('errors'):raise Unavailable('artifact collection incomplete; see archive.json')
    raise Unavailable('main native job driver is not yet executable; no empty success')

def worker(evidence):
    root=Path(evidence);receipt=read_json(root/'authorization-receipt.json')
    doc=checked_authority((root/'authority.json').read_bytes(),receipt['digest'],receipt['context'])
    bind_manifest(doc,(root/'manifest.json').read_bytes())
    from supervisor import PhasePublisher
    phases=PhasePublisher(root/'phase.json');phases.enter('setup')
    from setup import build_probe
    config=build_probe(doc,root)
    config['authority_path']=str(root/'authority.json');config['artifact_root']=str(root)
    from controller import authorize as probe_authorize
    grant=probe_authorize(root/'authority.json',receipt['digest'],mode='feasibility',diagnostic_sha=doc['diagnostic_sha'],product_sha=doc['product_sha'])
    from probe import execute as run_probe
    result=run_probe(config,grant,root/'probe-result.json',phases)
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser();s=p.add_subparsers(dest='command',required=True)
    a=s.add_parser('authorize');a.add_argument('--mode',choices=['main','feasibility'],required=True);a.add_argument('--branch',required=True)
    e=s.add_parser('execute');e.add_argument('--job',required=True)
    w=s.add_parser('worker');w.add_argument('--evidence',required=True);args=p.parse_args()
    try:
        if args.command=='authorize':authorize(args.mode,args.branch)
        elif args.command=='worker':worker(args.evidence)
        else:execute(args.job)
    except (Unavailable,ValueError,OSError,KeyError,subprocess.CalledProcessError) as error:
        atomic_json(Path('native-evidence')/'job-refusal.json',{'state':None,'error':str(error),'release_verdict':'NONE'})
        raise SystemExit(2)
