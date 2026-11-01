"""Re-derive push isolation from immutable Git objects; candidate is UNINSTALLED.

Conservative over paths: a branch-compatible existing filter is a possible
match for some changed path. Unknown syntax refuses measurement, never means0.
"""
import argparse
import copy
import json
import re
import subprocess
from pathlib import Path
import yaml
from protocol import sha

class Refused(ValueError):pass
class Loader(yaml.SafeLoader):pass
Loader.yaml_implicit_resolvers=copy.deepcopy(yaml.SafeLoader.yaml_implicit_resolvers)
for key, values in list(Loader.yaml_implicit_resolvers.items()):
    Loader.yaml_implicit_resolvers[key]=[(tag,rx) for tag,rx in values if tag!='tag:yaml.org,2002:bool']
Loader.add_implicit_resolver('tag:yaml.org,2002:bool',re.compile(r'^(?:true|false)$',re.I),list('tTfF'))
def unique_mapping(loader,node,deep=False):
    loader.flatten_mapping(node);result={}
    for key,value in node.value:
        k=loader.construct_object(key,deep=deep)
        if k in result:raise Refused('duplicate YAML key')
        result[k]=loader.construct_object(value,deep=deep)
    return result
Loader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,unique_mapping)


def load(raw):
    value=yaml.load(raw,Loader=Loader)
    if type(value) is not dict:raise Refused('workflow is not an object')
    return value

def glob(pattern,name):
    if type(pattern) is not str or not pattern or any(x in pattern for x in ('$','[',']','{','}','+','\\')):
        raise Refused('unsupported filter pattern: '+repr(pattern))
    out='';i=0
    while i<len(pattern):
        if pattern[i:i+2]=='**':out+='.*';i+=2
        elif pattern[i]=='*':out+='[^/]*';i+=1
        elif pattern[i]=='?':out+='[^/]';i+=1
        else:out+=re.escape(pattern[i]);i+=1
    return re.fullmatch(out,name) is not None

def patterns(value):
    if type(value) is not list or not value or any(type(x) is not str for x in value):raise Refused('filter must be a nonempty string list')
    for item in value:glob(item.lstrip('!'),'validation')
    return value

def included(values,name):
    result=False
    for item in patterns(values):
        negative=item.startswith('!');pattern=item[1:] if negative else item
        if glob(pattern,name):result=not negative
    return result

def push_match(workflow,kind,name):
    if kind not in ('branch','tag'):raise Refused('unknown ref kind')
    triggers=workflow.get('on')
    if type(triggers) is str:triggers={triggers:None}
    elif type(triggers) is list:
        if any(type(x) is not str for x in triggers):raise Refused('event list types')
        triggers=dict.fromkeys(triggers)
    if type(triggers) is not dict:raise Refused('on declaration unavailable')
    known={'push','pull_request','pull_request_target','workflow_dispatch','schedule','workflow_call','release'}
    if set(triggers)-known:raise Refused('unmodelled trigger/indirect chain: '+repr(set(triggers)-known))
    if 'push' not in triggers:return False
    rule=triggers['push']
    if rule is None:return True
    if type(rule) is not dict:raise Refused('push shape')
    allowed={'branches','branches-ignore','tags','tags-ignore','paths','paths-ignore'}
    if set(rule)-allowed:raise Refused('unsupported push fields')
    for key,value in rule.items():patterns(value)
    for axis in ('branches','tags','paths'):
        if axis in rule and axis+'-ignore' in rule:raise Refused('conflicting positive/negative filter')
    axis='branches' if kind=='branch' else 'tags';other='tags' if kind=='branch' else 'branches'
    if axis in rule:matched=included(rule[axis],name)
    elif axis+'-ignore' in rule:matched=not included(rule[axis+'-ignore'],name)
    else:matched=not (other in rule or other+'-ignore' in rule)
    # Path filters cannot prove universal rejection of a branch. Report possible match.
    return matched

def exact_candidate(candidate,branch):
    if candidate.get('on')!={'push':{'branches':[branch]}}:raise Refused('candidate is not exact branch-only push')
    if re.fullmatch(r'work/[A-Za-z0-9][A-Za-z0-9._/-]*',branch) is None:raise Refused('experiment branch is not literal work/ ref')
    if candidate.get('permissions')!={'contents':'read'}:raise Refused('candidate permissions broadened')
    if candidate.get('concurrency',{}).get('cancel-in-progress') is not False:raise Refused('candidate cancellation policy')
    jobs=candidate.get('jobs')
    if type(jobs) is not dict or not jobs:raise Refused('no candidate jobs')
    for job in jobs.values():
        if job.get('runs-on')!='windows-latest':raise Refused('nonstandard runner')
        if 'uses' in job:raise Refused('reusable workflow outside census')
    return True

def census(repo,tip,branch,candidate_path,positive_ref='fix/0.14.56-candidate',installed_candidate=None):
    if not re.fullmatch('[0-9a-f]{40}',tip):raise Refused('immutable full tip required')
    def git(*args):return subprocess.check_output(['git','-C',str(repo),*args])
    files=[p for p in git('ls-tree','-r','--name-only',tip,'--','.github/workflows').decode().splitlines() if p.endswith(('.yml','.yaml'))]
    if not files:raise Refused('empty workflow population')
    workflows={p:load(git('show',tip+':'+p)) for p in files}
    hashes={p:sha(git('show',tip+':'+p)) for p in files}
    candidate_raw=Path(candidate_path).read_bytes();candidate=load(candidate_raw);exact_candidate(candidate,branch)
    installed_path=None
    if installed_candidate is not None:
        if type(installed_candidate) is not dict or set(installed_candidate)!={'path','sha256'}:raise Refused('installed candidate identity fields')
        installed_path=installed_candidate['path']
        if type(installed_path) is not str or type(installed_candidate['sha256']) is not str or not re.fullmatch('[0-9a-f]{64}',installed_candidate['sha256']):raise Refused('installed candidate identity types')
        if installed_path not in workflows or hashes[installed_path]!=installed_candidate['sha256'] or sha(candidate_raw)!=installed_candidate['sha256']:
            raise Refused('installed candidate path/digest mismatch')
    others={p:w for p,w in workflows.items() if p!=installed_path}
    existing=[p for p,w in others.items() if push_match(w,'branch',branch)]
    controls=[]
    for kind,ref in [('branch',branch),('branch','work/unrelated-probe'),('branch','work/other-branch'),('branch',branch+'-suffix'),('branch','main'),('branch',positive_ref),('tag',branch),('tag','v0.14.56')]:
        old=[p for p,w in others.items() if push_match(w,kind,ref)]
        own=push_match(candidate,kind,ref);controls.append({'kind':kind,'ref':ref,'existing_matches':old,'candidate_matches':own})
        if own!=(kind=='branch' and ref==branch):raise Refused('candidate negative/positive control failed')
    positive=next(x for x in controls if x['kind']=='branch' and x['ref']==positive_ref)
    expected={'.github/workflows/ci-branch.yml','.github/workflows/windows-build.yml','.github/workflows/windows-health.yml'}
    if set(positive['existing_matches'])!=expected:raise Refused('existing positive control changed or parser blind')
    if existing:raise Refused('P1 existing workflow may match experiment: '+repr(existing))
    broad=copy.deepcopy(candidate);broad['on']['push']['branches']=['work/**']
    try:exact_candidate(broad,branch)
    except Refused:broad_rejected=True
    else:raise Refused('broad-filter negative control survived')
    return {'scope':'FINAL_INSTALLED_TIP' if installed_path else 'LOCAL_PREVIEW_NOT_FINAL','tip':tip,'branch':branch,'candidate_installed':bool(installed_path),'installed_candidate':installed_candidate,
            'existing_workflows':len(others),'existing_experiment_matches':existing,'workflow_hashes':hashes,
            'candidate_sha256':sha(candidate_raw),'prospective_population':len(others)+1,
            'prospective_experiment_matches':1,'P1':True,'P2':True,'P3_by_construction':True,
            'P3_population_note':('candidate counted once in installed Git population' if installed_path else 'candidate is included in prospective population, not installed in this Git tip'),
            'broad_filter_rejected':broad_rejected,'controls':controls,'release_verdict':'NONE'}

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--repo',required=True);p.add_argument('--tip',required=True);p.add_argument('--branch',required=True);p.add_argument('--candidate',required=True);p.add_argument('--output',required=True);p.add_argument('--installed-workflow-path');p.add_argument('--installed-workflow-sha256');a=p.parse_args()
    installed=None
    if a.installed_workflow_path or a.installed_workflow_sha256:
        if not (a.installed_workflow_path and a.installed_workflow_sha256):p.error('installed path and digest required together')
        installed={'path':a.installed_workflow_path,'sha256':a.installed_workflow_sha256}
    try:result=census(a.repo,a.tip,a.branch,a.candidate,installed_candidate=installed)
    except (Refused,ValueError,subprocess.CalledProcessError) as e:
        Path(a.output).write_text(json.dumps({'verdict':'REFUSE','reason':str(e),'release_verdict':'NONE'},indent=2));raise SystemExit(2)
    Path(a.output).write_text(json.dumps(result,indent=2))
