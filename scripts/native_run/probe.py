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
from observer import compare_composer, calibrate_exports, calibrated_no_submit, InvalidEvidence
from protocol import atomic_json, read_json, sha

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
    return True,observations

def new_receiver(config,grant,parent):
    root,exe=prepare_root(parent,config['receiver_executable'],config['receiver_sha256'])
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
            item.close();row['cleanup']=item.cleanup
        atomic_json(root/'evidence/origin-calibration.json',row);records.append(row)
        if not row['cleanup'].get('complete'):break
    return {'subcases':records,'cleanup_complete':len(records)==2 and all(r['cleanup'].get('complete') is True for r in records),'discriminator':None,'release_verdict':'NONE',
            'meaning':'raw calibration only; no automatic attribution transfer to real receiver'}

def execute(config,grant,output):
    if sys.platform!='win32':raise Unavailable('native probe cannot execute on this host')
    parent=Path(tempfile.gettempdir());premises=dict.fromkeys(('network_boundary','auth_free_startup','genuine_open_query','no_submit_calibration'))
    details={};errors=[]
    # No receiver is started before the actual boundary discrimination succeeds.
    try:premises['network_boundary'],details['network']=network_probe(config,grant,parent)
    except (OSError,Unavailable,ValueError) as e:errors.append({'phase':'network_boundary','error':str(e)})
    if premises['network_boundary'] is True:
        details['origin_calibration']=origin_calibration(config,grant,parent)
        if details['origin_calibration']['cleanup_complete'] is not True:
            result={**decide_feasibility(premises),'details':details,'errors':[{'phase':'origin','error':'cleanup incomplete; no further children'}]}
            atomic_json(output,result);return result
        receiver=None
        try:
            receiver=new_receiver(config,grant,parent);details['receiver_root']=str(receiver.root);details['launch']=receiver.start()
            baseline='NATIVE_VISIBLE_'+receiver.run_id
            receiver.write(baseline);before=receiver.export()
            # A real editor export, not a static prompt/counter, establishes startup responsiveness.
            premises['auth_free_startup']=before==baseline.encode()
            if premises['auth_free_startup']:
                pre=receiver.observations()
                old_events=[read_json(p) for p in sorted((receiver.root/'trace').glob('*.json'))]
                previous_seq=max((e['seq'] for e in old_events),default=-1)
                receiver.write('\x1b[200~')
                interval_start=time.monotonic_ns();resizes=receiver.resize_sequence()
                # Fixed bounded observation only; no startup or query retries. Absence is NULL.
                end=time.monotonic()+20
                while time.monotonic()<end:
                    events=[read_json(p) for p in sorted((receiver.root/'trace').glob('*.json'))]
                    if any(e.get('seq',-1)>previous_seq and e.get('kind')=='reply_generated' and e.get('surface')==receiver.sid for e in events):break
                    time.sleep(.02)
                decision=receiver.observations();interval_end=time.monotonic_ns()
                receiver.write('\x1b[201~');after=receiver.export()
                details['query']={'pre':pre,'decision':decision,'resize_receipts':resizes,
                    'trace_sequence_before_open':previous_seq,'controller_start_ns':interval_start,'controller_end_ns':interval_end,
                    'before_sha256':sha(before),'after_sha256':sha(after),'preserved':compare_composer(before,after),
                    'application_open_ack':None,'genuine_origin':None,
                    'reason':'raw product trace retained; daemon flag/controller fence is not independent application-open/origin proof'}
                if before!=after:errors.append({'phase':'genuine_open_query','observed_failure':'composer_changed'})
                # Do not promote a raw query or daemon flag into the missing semantic premise.
                premises['genuine_open_query']=None
        except (OSError,Unavailable,ValueError,InvalidEvidence) as e:errors.append({'phase':'receiver','error':str(e)})
        finally:
            if receiver:
                receiver.close();details['cleanup']=receiver.cleanup
                if receiver.cleanup.get('complete') is not True:
                    premises['auth_free_startup']=None;premises['genuine_open_query']=None
                    errors.append({'phase':'receiver_cleanup','error':'cleanup incomplete; calibration not started'})
        if premises['auth_free_startup'] is True:
            pair=[];sentinel='SUBMIT_CALIBRATION_'+uuid.uuid4().hex
            try:
                for submit in (False,True):
                    item=new_receiver(config,grant,parent)
                    try:
                        item.start();item.write(sentinel)
                        if submit:item.key('Return')
                        # Only the deliberate calibration child receives Return, never R2/R3.
                        records=bounded_observation('application submission record',lambda: application_records(item) if list((item.root/'child/config').glob('projects/*/'+item.spec['session_id']+'.jsonl')) else None,10)
                        pair.append((item.spec['session_id'],records))
                    finally:
                        item.close();details.setdefault('calibration_cleanup',[]).append(item.cleanup)
                    if item.cleanup.get('complete') is not True:raise Unavailable('calibration cleanup incomplete; no more children')
                premises['no_submit_calibration']=calibrated_no_submit(pair[0][1],pair[1][1],no_session=pair[0][0],yes_session=pair[1][0],sentinel=sentinel)
            except (OSError,Unavailable,ValueError,InvalidEvidence) as e:errors.append({'phase':'no_submit_calibration','error':str(e)})
    result={**decide_feasibility(premises),'details':details,'errors':errors,'source':{'product_sha':grant.product_sha,'diagnostic_sha':grant.diagnostic_sha},'authorization_sha256':grant.digest}
    atomic_json(output,result);return result

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--plan',action='store_true');p.add_argument('--execute',action='store_true');p.add_argument('--config');p.add_argument('--authority');p.add_argument('--authority-sha256');p.add_argument('--output');a=p.parse_args()
    if a.plan and not a.execute:print(json.dumps(proposed_plan(),indent=2))
    elif a.execute and not a.plan:
        config=read_json(a.config)
        grant=authorize(a.authority,a.authority_sha256,mode='feasibility',diagnostic_sha=config['diagnostic_sha'],product_sha=config['product_sha'])
        config['authority_path']=a.authority
        execute(config,grant,a.output)
    else:p.error('choose inert --plan or separately authorized --execute')
