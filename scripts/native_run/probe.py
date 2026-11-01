"""Feasibility probe entry point. --plan is inert; --execute needs separate E1 authority.

The same OwnedBoundary, ProductReceiver and observer are used by arm R. A
successful process exit is not a premise. This probe never closes arm R's gate.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import threading
import time
import uuid
from boundary import OwnedBoundary, Unavailable, cleanup_valid
from controller import authorize, prepare_root, protect_native_root, fixture_environment, decide_feasibility
from fixture import ProductReceiver, bounded_observation, pinned_copy
from observer import compare_composer, calibrate_exports, calibrated_no_submit, InvalidEvidence, accepted_input, open_differential, submission_events
from protocol import atomic_json, read_json, sha
from archive import register

PHASES=[('setup',600),('network_boundary',180),('auth_free_startup',240),('origin_calibration',60),('genuine_open_query',180),('no_submit_calibration',240),('collection',300)]

def proposed_plan():
    assert sum(seconds for _,seconds in PHASES)==30*60
    return {'mode':'feasibility','jobs':1,'runner':'windows-latest','max_parallel':1,'attempts':1,
            'runner_minutes':30,'phases':[{'name':n,'cap_seconds':s} for n,s in PHASES],
            'main_populations_unchanged':True,'native_hard_gate':'OPEN','release_verdict':'NONE'}

class LoopbackCanary:
    def __enter__(self):
        self.socket=socket.socket();self.socket.bind(('127.0.0.1',0));self.socket.listen(4);self.socket.settimeout(.2)
        self.endpoint=self.socket.getsockname();self.stop=threading.Event();self.accepts=0
        def drain():
            while not self.stop.is_set():
                try:
                    client,_=self.socket.accept();self.accepts+=1;client.close()
                except socket.timeout:pass
                except OSError:break
        self.thread=threading.Thread(target=drain);self.thread.start();return self
    def __exit__(self,*unused):
        self.stop.set();self.socket.close();self.thread.join(2)
        if self.thread.is_alive():raise Unavailable('owned canary cleanup unobserved')

def parent_connect(address,port):
    try:
        with socket.create_connection((address,port),timeout=2):return {'connected':True,'error':None}
    except OSError as e:return {'connected':False,'error':str(e)}

def network_probe(config, grant, parent):
    observations=[]
    with LoopbackCanary() as listener:
        endpoints=[listener.endpoint,(config['canary_literal_address'],443)]
        for address,port in endpoints:
            if datetime.datetime.now(datetime.timezone.utc)>=datetime.datetime.fromisoformat(grant.latest_end_utc):raise Unavailable('authority expired before boundary control')
            root,exe=prepare_root(parent,config['receiver_helper'],config['receiver_helper_sha256'])
            register(root,config)
            protect_native_root(root)
            output=root/'child/connect.json'
            spec={'run_root':str(root),'executable':str(exe),'executable_sha256':config['receiver_helper_sha256'],
                  'args':['connect',str(root/'child'),str(output),address,str(port)],
                  'environment':fixture_environment(root,os.environ['SystemRoot']),'process_limit':2}
            before=parent_connect(address,port);child=None;result=None;cleanup=None
            try:
                child=OwnedBoundary(spec);receipt=child.release()
                code=child.wait(10000)
                if code==0 and output.is_file():result=read_json(output)
            finally:
                if child:child.close();cleanup=child.cleanup
            after=parent_connect(address,port)
            row={'expected_pid':None if child is None else child.process.dwProcessId,'endpoint':[address,port],'parent_before':before,'child':result,'parent_after':after,
                 'token':None if child is None else child.token,'cleanup':cleanup}
            observations.append(row);atomic_json(root/'evidence/network.json',row)
    for row in observations:
        child=row['child']
        if type(child) is not dict or type(child.get('pid')) is not int or child['pid']!=row['expected_pid'] or child.get('address')!=row['endpoint'][0] or type(child.get('port')) is not int or child['port']!=row['endpoint'][1]:return None,observations
        if child.get('connected') is True:return False,observations
        if not row['parent_before']['connected'] or not row['parent_after']['connected']:return None,observations
        if type(child) is not dict or type(child.get('connected')) is not bool:return None,observations
        if child['connected'] is not False or child.get('error')!='AccessDenied:10013':return None,observations
        cleanup=row['cleanup']
        if not cleanup_valid(cleanup):return None,observations
        if cleanup.get('confinement') is not True:return cleanup.get('confinement'),observations
    return True,observations

def new_receiver(config,grant,parent):
    root,exe=prepare_root(parent,config['receiver_executable'],config['receiver_sha256'])
    register(root,config)
    protect_native_root(root)
    spec={**config,'run_root':str(root),'executable':str(exe),'executable_sha256':config['receiver_sha256'],
          'mode':grant.mode,'session_id':str(uuid.uuid4())}
    return ProductReceiver(grant,root,spec)

def application_records(receiver):
    # Only the explicitly bound session file. No live-home scan or suffix/sample pairing.
    session=receiver.spec['session_id'];files=list((receiver.root/'child/config').glob('projects/*/'+session+'.jsonl'))
    if len(files)!=1:raise Unavailable('application submission transcript unavailable/ambiguous')
    data=files[0].read_bytes();(receiver.root/'evidence/application.jsonl').write_bytes(data);return data

def origin_calibration(config,grant,parent):
    records=[]
    for mode in ('emit','silent'):
        root,exe=prepare_root(parent,config['receiver_helper'],config['receiver_helper_sha256'])
        register(root,config)
        protect_native_root(root)
        sideband=root/'child/origin';go=root/'child/origin-go'
        spec={**config,'run_root':str(root),'executable':str(exe),'executable_sha256':config['receiver_helper_sha256'],
              'mode':grant.mode,'actor':'origin_control','session_id':str(uuid.uuid4()),
              'control_args':['origin',str(root/'child'),str(sideband),mode,str(go)]}
        item=ProductReceiver(grant,root,spec);row={'mode':mode,'query_role':'calibration_only','eligible_for_R':False}
        try:
            row['launch']=item.start()
            ready=bounded_observation('origin child ready',lambda:read_json(str(sideband)+'.ready.json') if Path(str(sideband)+'.ready.json').is_file() else None,10)
            go.write_bytes(b'one owned origin handshake')
            row['emission']=bounded_observation('origin child emission receipt',lambda:read_json(str(sideband)+'.emission.json') if Path(str(sideband)+'.emission.json').is_file() else None,10)
            row['ready']=ready;row['resizes']=item.resize_sequence()
            row['trace_root']=str(root/'trace')
            # Collect fixed observations; matching bytes/timestamps are not an attribution rule.
            row['product_observation']=item.observations();row['attribution']=None
        except (OSError,Unavailable,ValueError) as e:row['unmeasured']=str(e)
        finally:
            Path(str(go)+'.stop').write_bytes(b'owned calibration complete')
            try:
                row['child_input']=bounded_observation('origin input receipt',lambda:read_json(str(sideband)+'.input.json') if Path(str(sideband)+'.input.json').is_file() else None,3)
            except (Unavailable,OSError,ValueError) as error:row['input_unmeasured']=str(error)
            item.close();row['cleanup']=item.cleanup
        atomic_json(root/'evidence/origin-calibration.json',row);records.append(row)
        if not row['cleanup'].get('complete'):break
    return {'subcases':records,'cleanup_complete':len(records)==2 and all(r['cleanup'].get('complete') is True for r in records),'discriminator':None,'release_verdict':'NONE',
            'meaning':'raw calibration only; no automatic attribution transfer to real receiver'}

def passive_interval(receiver, seconds=2):
    """Fixed observation window, not a retry-until-good loop or a write."""
    frames=[];end=time.monotonic()+seconds
    while True:
        frame=receiver.passive_frame()
        frames.append({'controller_ns':time.monotonic_ns(),'trace_seq':receiver.trace_sequence(),'frame':frame})
        if time.monotonic()>=end:break
        time.sleep(.05)
    return frames

def open_control(receiver, config, grant, parent, baseline, before):
    # The OPEN branch uses the existing startup receiver. An independent
    # NOT-OPEN child gets the same baseline and perturbation, never OPEN.
    # All observations inside the interval are read-only; Ctrl-G occurs ONLY
    # before OPEN or after the designated CLOSE boundary.
    sentinel='PERTURB_'+uuid.uuid4().hex[:12]
    row={'sentinel':sentinel,'oracle_method':'passive_render_plus_final_byte_exports',
         'origin':None,'eligibility':'SUPPLEMENTAL: real receiver, origin unknown'}
    plain=None
    try:
        start=receiver.trace_sequence()
        receiver.write('\x1b[200~'+sentinel)
        row['open_frames']=passive_interval(receiver)
        row['resize_receipts']=receiver.resize_sequence()
        row['decision_frames']=passive_interval(receiver,2)
        row['open_input_accepted']=accepted_input(receiver.trace_events(),receiver.sid,start,('\x1b[200~'+sentinel).encode())
        row['boundary_before_close_seq']=receiver.trace_sequence()
        receiver.write('\x1b[201~')
        after=receiver.export()
        row['open_final_hex']=after.hex();row['baseline_hex']=before.hex()
        row['known_failures']=[] if after==before+sentinel.encode() else ['open_composer_changed']
        # Stop this receiver before starting another; failed cleanup is a stop,
        # not an excuse to accumulate jobs or adopt a live child.
        receiver.close();row['open_cleanup']=receiver.cleanup
        if receiver.cleanup.get('complete') is not True:raise Unavailable('OPEN cleanup incomplete')
        plain=new_receiver(config,grant,parent);row['plain_root']=str(plain.root)
        plain.start();plain.write(baseline);plain_before=plain.export()
        if plain_before!=before:raise Unavailable('NOT-OPEN baseline differs')
        start=plain.trace_sequence();plain.write(sentinel)
        row['plain_frames']=passive_interval(plain)
        row['plain_resize_receipts']=plain.resize_sequence()
        row['plain_decision_frames']=passive_interval(plain,2)
        row['plain_input_accepted']=accepted_input(plain.trace_events(),plain.sid,start,sentinel.encode())
        plain_after=plain.export();row['plain_final_hex']=plain_after.hex()
        row['discrimination']=open_differential(row['decision_frames'][-1]['frame'],row['plain_decision_frames'][-1]['frame'],
            baseline=before,sentinel=sentinel.encode(),open_final=after,plain_final=plain_after,
            open_accepted=row['open_input_accepted'],plain_accepted=row['plain_input_accepted'])
        if plain_after!=before+sentinel.encode():row['known_failures'].append('plain_composer_changed')
        # A calibrated temporal OPEN discriminator still supplies no query origin.
        row['genuine_open_query']=None
    except (OSError,Unavailable,ValueError,InvalidEvidence) as e:row['unmeasured']=str(e)
    finally:
        if plain:plain.close();row['plain_cleanup']=plain.cleanup
        row['cleanup_complete']=row.get('open_cleanup',{}).get('complete') is True and (plain is None or plain.cleanup.get('complete') is True)
        atomic_json(receiver.root/'evidence/open-control.json',row)
    return row

def execute(config,grant,output,phases):
    if sys.platform!='win32':raise Unavailable('native probe cannot execute on this host')
    parent=Path(tempfile.gettempdir());premises=dict.fromkeys(('network_boundary','auth_free_startup','genuine_open_query','no_submit_calibration'))
    details={};errors=[]
    # No receiver is started before the actual boundary discrimination succeeds.
    phases.enter('network_boundary')
    try:premises['network_boundary'],details['network']=network_probe(config,grant,parent)
    except (OSError,Unavailable,ValueError) as e:errors.append({'phase':'network_boundary','error':str(e)})
    if premises['network_boundary'] is True:
        phases.enter('auth_free_startup')
        receiver=None
        try:
            receiver=new_receiver(config,grant,parent);details['receiver_root']=str(receiver.root);details['launch']=receiver.start()
            baseline='NATIVE_VISIBLE_'+receiver.run_id
            receiver.write(baseline);before=receiver.export()
            # A real editor export, not a static prompt/counter, establishes startup responsiveness.
            premises['auth_free_startup']=before==baseline.encode()
            if premises['auth_free_startup']:
                receiver.write('!');visible=receiver.export()
                receiver.key('Backspace');receiver.write('X');append_one=receiver.export()
                receiver.key('Backspace');receiver.key('Backspace');remove_one=receiver.export()
                receiver.write(baseline[-1]);restored=receiver.export()
                checks=calibrate_exports(baseline.encode(),before,visible,append_one,remove_one,b'!')
                checks['restored']=restored==before
                details['export_controls']={'checks':checks,'baseline_hex':before.hex(),'visible_hex':visible.hex(),
                    'append_hex':append_one.hex(),'remove_hex':remove_one.hex(),'restored_hex':restored.hex()}
                if not all(checks.values()):
                    premises['auth_free_startup']=None
                    errors.append({'phase':'auth_free_startup','observed_failure':'export_oracle_control_failed'})
            if premises['auth_free_startup']:
                phases.enter('origin_calibration')
                details['origin_calibration']=origin_calibration(config,grant,parent)
                if details['origin_calibration']['cleanup_complete'] is not True:raise Unavailable('origin cleanup incomplete; no further children')
                phases.enter('genuine_open_query')
                details['query']=open_control(receiver,config,grant,parent,baseline,before)
                for failure in details['query'].get('known_failures',[]):errors.append({'phase':'genuine_open_query','observed_failure':failure})
                premises['genuine_open_query']=None

        except (OSError,Unavailable,ValueError,InvalidEvidence) as e:errors.append({'phase':'receiver','error':str(e)})
        finally:
            if receiver:
                receiver.close();details['cleanup']=receiver.cleanup
                confinement=(receiver.cleanup.get('receiver') or {}).get('cleanup',{}).get('confinement')
                if confinement is not True:premises['network_boundary']=confinement
                if receiver.cleanup.get('complete') is not True:
                    premises['auth_free_startup']=None;premises['genuine_open_query']=None
                    errors.append({'phase':'receiver_cleanup','error':'cleanup incomplete; calibration not started'})
        if premises['auth_free_startup'] is True and details.get('query',{}).get('cleanup_complete') is True:
            phases.enter('no_submit_calibration')
            pair=[];sentinel='SUBMIT_CALIBRATION_'+uuid.uuid4().hex
            try:
                for submit in (False,True):
                    item=new_receiver(config,grant,parent)
                    try:
                        item.start();start=item.trace_sequence();item.write(sentinel)
                        if submit:item.key('Return')
                        # Only the deliberate calibration child receives Return, never R2/R3.
                        # Fixed observation boundary for BOTH arms. File existence
                        # immediately after Return is not a completed event stream.
                        passive_interval(item,10)
                        records=application_records(item)
                        accepted=accepted_input(item.trace_events(),item.sid,start,(sentinel+('\r' if submit else '')).encode())
                        details.setdefault('calibration_input_accepted',[]).append(accepted)
                        pair.append((item.spec['session_id'],records))
                    except (OSError,Unavailable,ValueError,InvalidEvidence) as error:
                        pair.append((item.spec['session_id'],None))
                        errors.append({'phase':'no_submit_calibration','submit_control':submit,'error':str(error)})
                    finally:
                        item.close();details.setdefault('calibration_cleanup',[]).append(item.cleanup)
                    if item.cleanup.get('complete') is not True:raise Unavailable('calibration cleanup incomplete; no more children')
                if any(raw is None for _,raw in pair):raise Unavailable('calibration transcript unavailable; absence is not zero')
                counts=[len(submission_events(raw,session,sentinel)) for session,raw in pair]
                details['submission_counts']=counts
                if counts[0]>0:errors.append({'phase':'no_submit_calibration','observed_failure':'unsubmitted_control_submitted'})
                if counts[1]>1:errors.append({'phase':'no_submit_calibration','observed_failure':'positive_control_multiple_submissions'})
                # A missing positive receipt is unavailable, never a false FAIL
                # inferred from an asynchronous logger's bounded silence.
                if counts[1]>0 and details.get('calibration_input_accepted')==[True,True]:
                    premises['no_submit_calibration']=counts==[0,1]

            except (OSError,Unavailable,ValueError,InvalidEvidence) as e:errors.append({'phase':'no_submit_calibration','error':str(e)})
    confinements=[]
    def collect_boundaries(value):
        if type(value) is dict:
            if 'confinement' in value:confinements.append(value['confinement'])
            for child in value.values():collect_boundaries(child)
        elif type(value) is list:
            for child in value:collect_boundaries(child)
    collect_boundaries(details)
    if any(v is False for v in confinements):premises['network_boundary']=False
    elif any(v is None for v in confinements):premises['network_boundary']=None
    result={**decide_feasibility(premises),'details':details,'errors':errors,'source':{'product_sha':grant.product_sha,'diagnostic_sha':grant.diagnostic_sha},'authorization_sha256':grant.digest}
    result['observed_failures'] += [e['observed_failure'] for e in errors if 'observed_failure' in e]
    atomic_json(output,result);return result

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--plan',action='store_true');p.add_argument('--execute',action='store_true');p.add_argument('--config');p.add_argument('--authority');p.add_argument('--authority-sha256');p.add_argument('--output');a=p.parse_args()
    if a.plan and not a.execute:print(json.dumps(proposed_plan(),indent=2))
    elif a.execute and not a.plan:
        config=read_json(a.config)
        grant=authorize(a.authority,a.authority_sha256,mode='feasibility',diagnostic_sha=config['diagnostic_sha'],product_sha=config['product_sha'])
        config['authority_path']=a.authority
        raise Unavailable('direct execution refused; use the externally authorized owned-job supervisor')
    else:p.error('choose inert --plan or separately authorized --execute')
