"""Independent raw-artifact collector. Never imports producer adjudication/simulation.

Known failures survive incomplete evidence. Rates retain the entire frozen
population and become NULL on missing/invalid/duplicate records.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
from protocol import read_json, plan, COUNTS, TARGET, SKIPS, atomic_json

class Rejected(ValueError):pass

def exact(value,kind,label):
    if type(value) is not kind:raise Rejected(label+': wrong type')
    return value

def digest(data):return hashlib.sha256(data).hexdigest()

def blob(root,reference):
    exact(reference,dict,'artifact')
    if set(reference)!={'path','sha256'}:raise Rejected('artifact fields')
    name=exact(reference['path'],str,'artifact path');expected=exact(reference['sha256'],str,'artifact digest')
    if not re.fullmatch('[0-9a-f]{64}',expected):raise Rejected('artifact digest format')
    rel=Path(name)
    if rel.is_absolute() or '..' in rel.parts or not rel.parts:raise Rejected('artifact path escape')
    base=Path(root).resolve(strict=True);path=base/rel
    for node in (path,*path.parents):
        if node==base.parent:break
        if node.is_symlink() or (hasattr(node,'is_junction') and node.is_junction()):raise Rejected('artifact reparse/link')
    raw=path.read_bytes()
    if digest(raw)!=expected:raise Rejected('artifact digest mismatch')
    return raw

SUMMARY=re.compile(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in ([0-9.]+)s$',re.M)
HEADER=re.compile(r'^test ([A-Za-z0-9_:]+)(?: - should panic)? \.\.\.',re.M)
FAILED=re.compile(r'^test ([A-Za-z0-9_:]+)(?: - should panic)? \.\.\. FAILED\s*$',re.M)

def witness_owner(raw):
    names=HEADER.findall(raw.decode('utf-8'))
    if len(set(names))!=1 or len(names)!=1:raise Rejected('witness owner absent/ambiguous; filename is not authority')
    return names[0]

def parent_summary(raw,selected,ignored,filtered,rc):
    exact(selected,list,'selected tests');exact(ignored,list,'ignored tests');exact(filtered,int,'filtered count')
    if not selected or any(type(x) is not str for x in selected+ignored):raise Rejected('empty/wrong test inventory')
    if len(set(selected))!=len(selected) or len(set(ignored))!=len(ignored) or not set(ignored)<=set(selected):raise Rejected('inventory duplicate/membership')
    text=raw.decode('utf-8');candidates=[]
    for m in SUMMARY.finditer(text):
        values=list(map(int,m.group(2,3,4,5,6)))
        if sum(values[:3])==len(selected) and values[2]==len(ignored) and values[3]==0 and values[4]==filtered:
            candidates.append((m,values))
    observed=set()
    headers=set(HEADER.findall(text))
    if not set(selected)<=headers:raise Rejected('expected test names absent from raw headers')
    if len(candidates)!=1:raise Rejected('parent summary absent/ambiguous')
    m,v=candidates[0];before=text[:m.start()]
    # libtest's final failures block, never a child summary or a panic-only line.
    blocks=re.findall(r'^failures:\s*\n((?:\s*\n|    [A-Za-z0-9_:]+\s*\n)+)',before,re.M)
    if v[1]:
        if not blocks:raise Rejected('parent failure names missing')
        observed.update(re.findall(r'^    ([A-Za-z0-9_:]+)\s*$',blocks[-1],re.M))
    if not observed<=set(selected) or len(observed)!=v[1]:raise Rejected('failure names/count do not reconcile')
    if rc is not None and (rc==0)!=(v[1]==0 and m.group(1)=='ok'):raise Rejected('exit/parent disagreement')
    return {'passed':v[0],'failed':v[1],'ignored':v[2],'filtered':v[4],
            'parent_line':m.group(0),'failures':sorted(observed),
            'all_summary_lines':[x.group(0) for x in SUMMARY.finditer(text)]}

def wilson(k,n):
    if type(k) is not int or type(n) is not int or not 0<=k<=n or n<=0:raise Rejected('Wilson domain')
    z=1.959963984540054;p=k/n;den=1+z*z/n;mid=(p+z*z/(2*n))/den
    width=z*math.sqrt(p*(1-p)/n+z*z/(4*n*n))/den
    return [max(0.0,mid-width),min(1.0,mid+width)]

def raw_failed_names(raw):
    text=raw.decode('utf-8');names=set(FAILED.findall(text))
    for block in re.findall(r'^failures:\s*\n((?:\s*\n|    [A-Za-z0-9_:]+\s*\n)+)',text,re.M):
        names.update(re.findall(r'^    ([A-Za-z0-9_:]+)\s*$',block,re.M))
    return sorted(names)

def count_submissions(raw,session,sentinel):
    if not session:raise Rejected('empty application session')
    count=0
    for line in raw.splitlines():
        if not line.strip():continue
        row=read_json_bytes(line);exact(row,dict,'application event')
        if row.get('sessionId') not in (None,session):raise Rejected('mixed application session')
        kind=exact(row.get('type'),str,'application event type')
        if kind=='user':
            if row.get('sessionId')!=session:raise Rejected('user event session absent')
            message=exact(row.get('message'),dict,'user message')
            if message.get('role')!='user':raise Rejected('wrong message role')
            content=message.get('content')
            if type(content) is str:text=content
            elif type(content) is list:
                parts=[]
                for item in content:
                    exact(item,dict,'content item')
                    if item.get('type')!='text':raise Rejected('unbound content kind')
                    parts.append(exact(item.get('text'),str,'content text'))
                text=''.join(parts)
            else:raise Rejected('unbound content shape')
            if sentinel is None or sentinel in text:count+=1
        elif kind not in ('assistant','system','progress','file-history-snapshot','summary','queue-operation'):
            raise Rejected('unbound application-event schema')
    return count

def submission_facts(raw,session,sentinel):
    count=0;errors=[]
    for number,line in enumerate(raw.splitlines(),1):
        if not line.strip():continue
        try:count+=count_submissions(line,session,sentinel)
        except (Rejected,ValueError,TypeError,UnicodeError) as error:errors.append('line '+str(number)+': '+str(error))
    return count,errors

def semantic(root,record,case,issues,failures):
    artifacts=record['artifacts']
    evidence=read_json_bytes(blob(root,artifacts['trace']))
    exact(evidence,dict,'trace')
    for key in ('native_windows','cleanup_complete','oracle_ready'):
        value=evidence.get(key)
        if value is not None:exact(value,bool,key)
        if value is not True:issues.append(key+' unavailable' if value is None else key+' false')
    if evidence.get('platform')!='windows' or evidence.get('transport')!='conpty':issues.append('native ConPTY identity missing')
    if evidence.get('request_id')!=evidence.get('original_request_id') or not evidence.get('request_id'):
        failures.append('request identity changed/absent')
    for key in ('body_offset_before','body_offset_after','original_proof_admissions','successor_proof_admissions'):
        if evidence.get(key) is None:issues.append(key+' missing')
        else:exact(evidence[key],int,key)
    if evidence.get('successor_proof_admissions',0)>0:failures.append('successor proof used')
    if case['arm']=='R':
        before=blob(root,artifacts['composer_before']);after=blob(root,artifacts['composer_after'])
        sentinel=blob(root,artifacts['sentinel'])
        expected=before+sentinel if case['cell']=='visible' else before
        if after!=expected:failures.append('composer preservation violation')
        # Independently evaluate changed-input, append and removal controls.
        seed=blob(root,artifacts['oracle_seed'])
        if not seed or not sentinel:issues.append('empty oracle calibration')
        checks=[('oracle_baseline',seed),('oracle_visible',seed+sentinel),('oracle_append',seed+b'X'),('oracle_remove',seed[:-1])]
        for key,want in checks:
            if blob(root,artifacts[key])!=want:issues.append('oracle control failed: '+key)
        stream=blob(root,artifacts['protected_input'])
        if b'\r' in stream or b'\n' in stream:failures.append('submit byte in protected interval')
        sessions=exact(evidence.get('application_sessions'),dict,'application sessions')
        needle=exact(case.get('calibration_sentinel'),str,'frozen calibration sentinel')
        counts={}
        for key,artifact_name in [('main','application_events'),('no','calibration_no_events'),('yes','calibration_yes_events')]:
            try:
                session=exact(sessions.get(key),str,'session identity')
                count,parse_errors=submission_facts(blob(root,artifacts[artifact_name]),session,needle if key!='main' else None)
                counts[key]=count
                if key=='main' and count>0:failures.append('application submission observed')
                if key=='no' and count>0:failures.append('unsubmitted calibration control submitted')
                if key=='yes' and count>1:failures.append('positive calibration multiple submissions')
                issues.extend(artifact_name+': '+error for error in parse_errors)
            except (Rejected,OSError,KeyError,ValueError,TypeError) as error:
                issues.append(artifact_name+': '+str(error))
        if counts.get('no')!=0 or counts.get('yes')!=1:issues.append('no-submit oracle uncalibrated')
        for name in ('network_boundary','open_interval_discriminated'):
            v=evidence.get(name)
            if v is not None:exact(v,bool,name)
            if v is not True:issues.append(name+' unavailable/failed')
        if case['cell'] not in ('baseline','visible'):
            if evidence.get('query_origin') not in ('claude','conpty'):issues.append('genuine query origin unavailable')
            for name in ('genuine_query','query_inside_open_interval','reply_completed'):
                value=evidence.get(name)
                if value is not None:exact(value,bool,name)
                if value is not True:issues.append(name+' unavailable/failed')
        if case['cell'] in ('dsr_outer','dsr_claim','successor') and evidence.get('decision')!='original_proof_refusal':
            failures.append('wrong proof refusal attribution')
    else:
        expected_action=case.get('expected_action')
        if not expected_action:issues.append('D contract not frozen')
        elif evidence.get('decision')!=expected_action:failures.append('D action differs from frozen contract')
        actual=blob(root,artifacts['receiver_bytes'])
        wire=case.get('expected_wire_hex')
        if type(wire) is not str or not re.fullmatch('(?:[0-9a-f]{2})+',wire):raise Rejected('nonempty frozen wire contract absent')
        if actual!=bytes.fromhex(wire):failures.append('native receiver bytes differ')
        if evidence.get('query_origin') not in ('fixture_child','claude','conpty'):issues.append('D query provenance missing')
    return evidence

def read_json_bytes(raw):
    def pairs(items):
        d={}
        for k,v in items:
            if k in d:raise Rejected('duplicate trace key')
            d[k]=v
        return d
    return json.loads(raw.decode('utf-8'),object_pairs_hook=pairs,parse_constant=lambda x:(_ for _ in ()).throw(Rejected('nonfinite JSON')))

def assess(root,record,case,manifest):
    issues=[];failures=[];details={};raw=None;identity_valid=False;raw_failures=[]
    # Retain raw failures even if another axis/record is incomplete. Never trust producer outcome.
    try:
        if type(record.get('artifacts')) is dict and 'stdout' in record['artifacts']:
            raw=blob(root,record['artifacts']['stdout']);raw_failures=raw_failed_names(raw)
    except (Rejected,OSError,ValueError) as e:issues.append(str(e))
    if case['arm'] in ('I','L','K') and raw is not None:
        try:
            details['parent']=parent_summary(raw,case.get('tests'),case.get('ignored'),case.get('filtered'),record.get('rc') if type(record.get('rc')) is int else None)
            failures.extend(details['parent']['failures'])
        except (Rejected,ValueError,TypeError) as e:issues.append(str(e))
    if case['arm']=='R':
        # Submission facts are independent of byte-oracle/other calibration
        # availability. Inspect each stream before any all-axis gate can throw.
        try:
            trace=read_json_bytes(blob(root,record['artifacts']['trace']))
            sessions=exact(trace.get('application_sessions'),dict,'application sessions')
            for role,key in [('main','application_events'),('no','calibration_no_events'),('yes','calibration_yes_events')]:
                try:
                    session=exact(sessions.get(role),str,'session identity')
                    sentinel=None if role=='main' else exact(case.get('calibration_sentinel'),str,'calibration sentinel')
                    count,errors=submission_facts(blob(root,record['artifacts'][key]),session,sentinel)
                    if role=='main' and count>0:failures.append('application submission observed')
                    if role=='no' and count>0:failures.append('unsubmitted calibration control submitted')
                    if role=='yes' and count>1:failures.append('positive calibration multiple submissions')
                    issues.extend(key+': '+e for e in errors)
                except (Rejected,OSError,KeyError,TypeError,ValueError) as e:issues.append(key+': '+str(e))
        except (Rejected,OSError,KeyError,TypeError,ValueError) as e:issues.append(str(e))
        try:
            before=blob(root,record['artifacts']['composer_before']);after=blob(root,record['artifacts']['composer_after'])
            want=before+blob(root,record['artifacts']['sentinel']) if case['cell']=='visible' else before
            if after!=want:failures.append('composer preservation violation')
        except (Rejected,OSError,KeyError,TypeError,ValueError) as e:issues.append(str(e))
    try:
        exact(record,dict,'record')
        for key in ('id','product_sha','diagnostic_sha','run_id','release_verdict'):exact(record.get(key),str,key)
        exact(record.get('attempt'),int,'attempt');exact(record.get('exit_observed'),bool,'exit_observed')
        if record.get('rc') is not None:exact(record['rc'],int,'rc')
        if record['id']!=case['id']:raise Rejected('case identity mismatch')
        for key in ('product_sha','diagnostic_sha','run_id','attempt'):
            if record[key]!=manifest[key]:raise Rejected('wrong '+key)
        if record['release_verdict']!='NONE':raise Rejected('release authority forged')
        identity_valid=True
        if not record['exit_observed'] or record['rc'] is None:issues.append('no process exit')
        if case['arm'] in ('I','L','K'):
            if raw is None:raise Rejected('stdout absent')
            if 'parent' not in details:raise Rejected('no verified parent summary')
            c8=record.get('c8')
            if c8 not in ('NO_UNMEASURED_MARKER','UNMEASURED','UNREADABLE'):raise Rejected('C8 axis absent/invalid')
            details['c8']=c8
            if 'C8_NATIVE_UNMEASURED' in raw.decode('utf-8') and c8=='NO_UNMEASURED_MARKER':raise Rejected('C8 marker concealed')
        else:
            details['trace']=semantic(root,record,case,issues,failures)
    except (Rejected,OSError,KeyError,TypeError,ValueError) as e:issues.append(str(e))
    failures=sorted(set(failures));issues=list(dict.fromkeys(issues));complete=not issues
    return {'id':case['id'],'complete':complete,'outcome':'FAIL' if failures and identity_valid else ('PASS' if complete else None),
            'identity_valid':identity_valid,'observed_failures':failures,
            'unattributed_raw_failures':raw_failures if 'parent' not in details or not identity_valid else [],'issues':issues,'details':details,'release_verdict':'NONE'}

def collect(root,manifest,records):
    exact(manifest,dict,'manifest');expected=exact(manifest.get('cases'),list,'planned cases')
    for c in expected:
        exact(c,dict,'planned case');exact(c.get('id'),str,'planned ID');exact(c.get('arm'),str,'planned arm')
    ids=[c['id'] for c in expected]
    if len(ids)!=len(set(ids)) or not ids:raise Rejected('planned population invalid')
    if {a:sum(c['arm']==a for c in expected) for a in COUNTS}!=COUNTS:raise Rejected('fixed population changed')
    for k in ('product_sha','diagnostic_sha','run_id'):exact(manifest.get(k),str,k)
    exact(manifest.get('attempt'),int,'manifest attempt')
    if manifest['attempt']!=1 or manifest.get('release_verdict')!='NONE':raise Rejected('manifest attempt/release')
    buckets={};unplanned=[];observed=[];unattributed=[]
    for record in records:
        if type(record) is not dict or type(record.get('id')) is not str:raise Rejected('record ID type')
        if record['id'] not in ids:
            unplanned.append(record['id'])
            try:
                for name in raw_failed_names(blob(root,record['artifacts']['stdout'])):unattributed.append({'id':record['id'],'failure':name,'reason':'unplanned record'})
            except (KeyError,Rejected,OSError,ValueError,TypeError):pass
        buckets.setdefault(record['id'],[]).append(record)
    trials=[]
    for case in expected:
        group=buckets.get(case['id'],[])
        if not group:trials.append({'id':case['id'],'complete':False,'outcome':None,'observed_failures':[],'issues':['missing planned record']});continue
        results=[assess(root,r,case,manifest) for r in group]
        for r in results:
            observed.extend({'id':case['id'],'failure':f,'identity_valid':r['identity_valid']} for f in r['observed_failures'])
            unattributed.extend({'id':case['id'],'failure':f,'reason':'unverified root/identity'} for f in r['unattributed_raw_failures'])
        result=results[0]
        if len(group)!=1:
            result={**result,'complete':False,'outcome':'FAIL' if any(r['observed_failures'] for r in results) else None,
                    'observed_failures':[f for r in results for f in r['observed_failures']], 'issues':result['issues']+['duplicate planned ID']}
        trials.append(result)
    arms={}
    byid={t['id']:t for t in trials}
    for arm,n in COUNTS.items():
        selected=[byid[c['id']] for c in expected if c['arm']==arm];completed=sum(x['complete'] for x in selected)
        fails=sum(x['outcome']=='FAIL' for x in selected);complete=completed==n and not unplanned
        arms[arm]={'planned':n,'completed':completed,'observed_failed_trials':fails,'failure_rate':fails/n if complete else None,
                   'wilson95':wilson(fails,n) if complete else None,'rate_null':not complete}
    return {'arms':arms,'trials':trials,'observed_failures':observed,'unattributed_failures':unattributed,'unplanned_ids':unplanned,
            'complete':all(not a['rate_null'] for a in arms.values()),'native_hard_gate':'OPEN', 'release_verdict':'NONE'}

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',required=True);p.add_argument('--manifest',required=True);p.add_argument('--records',nargs='+',required=True);p.add_argument('--output',required=True);a=p.parse_args()
    try:result=collect(a.root,read_json(a.manifest),[read_json(f) for f in a.records])
    except (Rejected,ValueError,OSError) as e:atomic_json(a.output,{'rejected':str(e),'release_verdict':'NONE'});raise SystemExit(2)
    atomic_json(a.output,result)
    raise SystemExit(1 if result['observed_failures'] else (0 if result['complete'] else 2))
