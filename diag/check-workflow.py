"""Local YAML/trigger census. No API, dispatch, push or CI results access."""
import fnmatch, json, pathlib, subprocess, yaml
ROOT=pathlib.Path(__file__).resolve().parent.parent
BASE='f0ac3b0c'
BRANCH='work/0.14.53-win-measure'
class UniqueLoader(yaml.BaseLoader): pass
def mapping(loader,node,deep=False):
    out={}
    for k,v in node.value:
        key=loader.construct_object(k,deep=deep)
        if key in out: raise ValueError('duplicate YAML key '+key)
        out[key]=loader.construct_object(v,deep=deep)
    return out
UniqueLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,mapping)
def loads(t): return yaml.load(t,Loader=UniqueLoader)
def allows(on):
    if not isinstance(on,dict) or 'push' not in on:return False
    push=on['push'] or {}
    if 'tags' in push and 'branches' not in push:return False
    return any(fnmatch.fnmatchcase(BRANCH,p) for p in push.get('branches',['**']))
def on_text(text):
    lines=text.splitlines();start=next(i for i,l in enumerate(lines) if l=='on:');end=start+1
    while end<len(lines) and (not lines[end] or lines[end][0].isspace() or lines[end].startswith('#')):end+=1
    return '\n'.join(lines[start:end]).rstrip()
raw=ROOT/'diag/local-checks/triggers';raw.mkdir(exist_ok=True)
listing=subprocess.check_output(['git','ls-tree','--name-only',BASE+':.github/workflows'],cwd=ROOT,text=True)
rows=[]
for name in listing.splitlines():
    if not name.endswith(('.yml','.yaml')):continue
    cmd=['git','show',BASE+':.github/workflows/'+name]
    text=subprocess.check_output(cmd,cwd=ROOT,text=True)
    (raw/name).write_text(text);(raw/(name+'.command.json')).write_text(json.dumps({'command':cmd,'rc':0},indent=2)+'\n')
    obj=loads(text);match=allows(obj.get('on'));assert not match,name
    rows.append((name,on_text(text),match))
new=loads((ROOT/'.github/workflows/win-measure.yml').read_text())
assert new['on']['push']['branches']==[BRANCH]
assert new['on']['push']['paths']==['diag/**','.github/workflows/win-measure.yml']
assert list(new['on'])==['push'] and allows(new['on'])
assert new['concurrency']['cancel-in-progress']=='false'
assert new['jobs']['measure']['needs']=='inputs'
assert len(new['jobs'])==2
# Controls for the predicate, including tag-only pushes and a dangerous broad branch.
assert allows({'push':{'branches':['work/**']}})
assert not allows({'push':{'tags':['v*']}})
assert not allows({'workflow_dispatch':{}})
report=['# Trigger census at '+BASE,'','Command: git ls-tree --name-only '+BASE+':.github/workflows; each raw/<name>.command.json holds git show command. Here raw files are local-checks/triggers/. No remote results read.','','| Workflow | Branch push triggers? |','|---|---|']
report += [f'| {n} | NO |' for n,_,_ in rows]
report += ['| win-measure.yml (new) | YES, only exact branch plus diag/workflow path filter |','']
for n,t,_ in rows:report.extend(['## '+n,'','```yaml',t,'```',''])
report.extend(['## win-measure.yml (new)','','```yaml',on_text((ROOT/'.github/workflows/win-measure.yml').read_text()),'```'])
(ROOT/'diag/TRIGGER-CENSUS.md').write_text('\n'.join(report)+'\n')
print(json.dumps({'yaml_parse':'PASS','base_workflows':len(rows),'other_matching_workflows':0,'new_matching_workflows':1,'predicate_controls':'PASS'}))
