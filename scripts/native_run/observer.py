"""Independent byte and application-event observations shared by probe and R.

Screens/counters/network silence are never submission or composer oracles.
These functions inspect retained artifacts; they do not launch a receiver.
"""
from pathlib import Path
import json
import re
from protocol import sha

class InvalidEvidence(ValueError):
    pass

def boolean(value, name):
    if type(value) is not bool: raise InvalidEvidence(name + ': expected boolean')
    return value

def integer(value, name):
    if type(value) is not int or value < 0: raise InvalidEvidence(name + ': expected nonnegative integer')
    return value

def artifact(root, ref):
    if type(ref) is not dict or set(ref) != {'path', 'sha256'}: raise InvalidEvidence('artifact reference shape')
    if type(ref['path']) is not str or type(ref['sha256']) is not str or not re.fullmatch('[0-9a-f]{64}',ref['sha256']):
        raise InvalidEvidence('artifact reference types')
    rel=Path(ref['path'])
    if rel.is_absolute() or not rel.parts or '..' in rel.parts: raise InvalidEvidence('artifact outside root')
    base=Path(root).resolve(strict=True); p=base/rel
    for node in (p,*p.parents):
        if node==base.parent:break
        if node.is_symlink() or (hasattr(node,'is_junction') and node.is_junction()): raise InvalidEvidence('artifact link rejected')
    data=p.read_bytes()
    if sha(data)!=ref['sha256']:raise InvalidEvidence('artifact hash mismatch')
    return data

def compare_composer(before, after, *, appended=b''):
    if type(before) is not bytes or type(after) is not bytes or type(appended) is not bytes:
        raise InvalidEvidence('composer observations must be exact bytes')
    return after == before + appended

def calibrate_exports(base, baseline, visible, append_one, remove_one, sentinel):
    if not all(type(x) is bytes for x in (base,baseline,visible,append_one,remove_one,sentinel)):
        raise InvalidEvidence('oracle calibration types')
    if not base or not sentinel:raise InvalidEvidence('empty oracle calibration population')
    return {'baseline':baseline==base,'visible':visible==base+sentinel,
            'append':append_one==base+b'X','remove':remove_one==base[:-1]}

def submission_events(raw, session_id, sentinel):
    """Only literal application records; never infer submission from TUI text.

Schema availability for the pinned Windows CLI must be demonstrated natively.
Malformed/unknown records are rejected rather than interpreted as no-submit.
"""
    if type(session_id) is not str or not session_id or type(sentinel) is not str or not sentinel:
        raise InvalidEvidence('submission observer identity absent')
    events=[]
    for line in raw.decode('utf-8').splitlines():
        if not line.strip():continue
        record=json.loads(line)
        if type(record) is not dict:raise InvalidEvidence('application record not an object')
        if record.get('sessionId') not in (None,session_id):raise InvalidEvidence('mixed application session')
        kind=record.get('type')
        if type(kind) is not str:raise InvalidEvidence('application record type absent')
        # Known submission-bearing forms. Every other kind remains retained, not a guessed user event.
        if kind=='user':
            if record.get('sessionId')!=session_id:raise InvalidEvidence('user session identity absent')
            message=record.get('message')
            if type(message) is not dict or message.get('role')!='user':raise InvalidEvidence('user message shape')
            content=message.get('content')
            if type(content) is str:text=content
            elif type(content) is list and all(type(x) is dict and x.get('type')=='text' and type(x.get('text')) is str for x in content):
                text=''.join(x['text'] for x in content)
            else:raise InvalidEvidence('unknown user content shape')
            if sentinel in text:events.append(record)
        elif kind in ('attachment','queued_command'):
            # No guessed adapter: this version-specific form must be bound during feasibility.
            raise InvalidEvidence('queued-command submission schema unbound')
        elif kind not in ('assistant','system','progress','file-history-snapshot','summary','queue-operation'):
            raise InvalidEvidence('unbound application event kind: '+kind)
    return events

def calibrated_no_submit(unsubmitted_raw, submitted_raw, *, no_session, yes_session, sentinel):
    no=submission_events(unsubmitted_raw,no_session,sentinel)
    yes=submission_events(submitted_raw,yes_session,sentinel)
    return len(no)==0 and len(yes)==1

CPR=re.compile(rb'\x1b\[[0-9]+;[0-9]+R')
def genuine_window(events, raw_output, raw_input):
    if type(events) is not list or not events:raise InvalidEvidence('empty protected interval')
    allowed={'paste_open','application_ack','query','reply','decision','paste_close'}
    seen={};last=-1
    for e in events:
        if type(e) is not dict or e.get('kind') not in allowed:raise InvalidEvidence('unknown interval event')
        stamp=integer(e.get('mono_ns'),'mono_ns')
        if stamp<=last:raise InvalidEvidence('unordered interval')
        last=stamp
        if e['kind'] in seen:raise InvalidEvidence('duplicate interval event')
        seen[e['kind']]=e
    required={'paste_open','application_ack','query','reply','decision'}
    if not required<=set(seen):return None
    q=seen['query'];r=seen['reply'];opened=seen['paste_open']['mono_ns']
    if q.get('origin') not in ('claude','conpty'):return None
    if seen['application_ack'].get('method') not in ('validated_application_observer','acknowledged_export_equivalent'):return None
    if not opened < seen['application_ack']['mono_ns'] <= q['mono_ns'] < r['mono_ns'] < seen['decision']['mono_ns']:return None
    if 'paste_close' in seen and seen['paste_close']['mono_ns']<=seen['decision']['mono_ns']:return None
    qa=integer(q.get('offset'),'query offset');ra=integer(r.get('offset'),'reply offset');n=integer(r.get('length'),'reply length')
    if raw_output[qa:qa+4]!=b'\x1b[6n' or not CPR.fullmatch(raw_input[ra:ra+n]):return None
    if not q.get('query_id') or q.get('query_id')!=r.get('query_id'):return None
    if r.get('origin')!='parser_generated' or r.get('write_result')!='complete':return None
    # A daemon paste flag is intentionally not read here.
    return True
