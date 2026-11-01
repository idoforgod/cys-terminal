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
from observer import compare_composer, calibrate_exports, calibrated_no_submit, InvalidEvidence, accepted_input, open_differential, submission_events, submission_findings
from protocol import atomic_json, read_json, sha
from archive import register

PHASES=[('setup',600),('network_boundary',180),('auth_free_startup',240),('origin_calibration',60),('genuine_open_query',180),('no_submit_calibration',240),('collection',300)]

def proposed_plan():
    assert sum(seconds for _,seconds in PHASES)==30*60
    return {'mode':'feasibility','jobs':1,'runner':'windows-latest','max_parallel':1,'attempts':1,
            'runner_minutes':30,'phases':[{'name':n,'cap_seconds':s} for n,s in PHASES],
            'main_populations_unchanged':True,'native_hard_gate':'OPEN','release_verdict':'NONE'}

def pending_obligation(ledger,name):
    if name in ledger:raise Unavailable('confinement obligation reused: '+name)
    slot={'confinement':None,'state':'pending'};ledger[name]=slot;return slot

def settle_obligation(slot,evidence,complete):
    value=boundary_conclusion(True,evidence)
    # Once observed, a failure cannot be erased by any later exception/absence.
    if slot['confinement'] is False or value is False:
        slot.update(confinement=False,state='observed_failure');return
    if 'exception' in slot:
        slot.update(confinement=None,state='unmeasured');return
    if complete is True and value is True:slot.update(confinement=True,state='verified')
    else:slot.update(confinement=None,state='unmeasured')

def obligation_exception(slot,error):
    slot['exception']=str(error)
    if slot['confinement'] is not False:slot.update(confinement=None,state='unmeasured')

def close_owned(item,slot):
    errors=[]
    try:item.close()
    except Exception as error:errors.append(str(error))
    receipt=getattr(item,'cleanup',None)
    if type(receipt) is not dict:receipt={'receiver':None,'daemon':None,'confinement':None,'complete':False}
    if errors:
        obligation_exception(slot,'; '.join(errors));receipt={**receipt,'complete':False,'close_errors':errors}
    settle_obligation(slot,receipt,receipt.get('complete') is True and not errors)
    return receipt

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

def network_probe(config, grant, parent,ledger):
    observations=[]
    with LoopbackCanary() as listener:
        endpoints=[listener.endpoint,(config['canary_literal_address'],443)]
        for ordinal,(address,port) in enumerate(endpoints):
            obligation=pending_obligation(ledger,'network/'+str(ordinal))
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
                if code==0 and output.is_file():
                    result=read_json(output)
                    if type(result) is dict and type(result.get('pid')) is int and result['pid']==child.process.dwProcessId and result.get('address')==address and type(result.get('port')) is int and result['port']==port and result.get('connected') is True:
                        settle_obligation(obligation,{'confinement':False},False)
            except Exception as error:
                settle_obligation(obligation,getattr(error,'partial_cleanup',None),False)
                obligation_exception(obligation,error)
                raise
            finally:
                if child:
                    try:child.close()
                    finally:
                        cleanup=getattr(child,'cleanup',None)
                        settle_obligation(obligation,cleanup,False)
            after=parent_connect(address,port)
            row={'expected_pid':None if child is None else child.process.dwProcessId,'endpoint':[address,port],'parent_before':before,'child':result,'parent_after':after,
                 'token':None if child is None else child.token,'cleanup':cleanup}
            observations.append(row);atomic_json(root/'evidence/network.json',row)
            settle_obligation(obligation,row,cleanup_valid(cleanup))
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

def origin_calibration(config,grant,parent,ledger,output=None):
    output={} if output is None else output
    records=[];output['subcases']=records
    for mode in ('emit','silent'):
        obligation=pending_obligation(ledger,'origin/'+mode)
        root,exe=prepare_root(parent,config['receiver_helper'],config['receiver_helper_sha256'])
        register(root,config)
        protect_native_root(root)
        sideband=root/'child/origin';go=root/'child/origin-go'
        spec={**config,'run_root':str(root),'executable':str(exe),'executable_sha256':config['receiver_helper_sha256'],
              'mode':grant.mode,'actor':'origin_control','session_id':str(uuid.uuid4()),
              'control_args':['origin',str(root/'child'),str(sideband),mode,str(go)]}
        item=ProductReceiver(grant,root,spec);row={'mode':mode,'query_role':'calibration_only','eligible_for_R':False}
        records.append(row)
        try:
            row['launch']=item.start()
            ready=bounded_observation('origin child ready',lambda:read_json(str(sideband)+'.ready.json') if Path(str(sideband)+'.ready.json').is_file() else None,10)
            go.write_bytes(b'one owned origin handshake')
            row['emission']=bounded_observation('origin child emission receipt',lambda:read_json(str(sideband)+'.emission.json') if Path(str(sideband)+'.emission.json').is_file() else None,10)
            row['ready']=ready;row['resizes']=item.resize_sequence()
            row['trace_root']=str(root/'trace')
            # Collect fixed observations; matching bytes/timestamps are not an attribution rule.
            row['product_observation']=item.observations();row['attribution']=None
        except Exception as e:
            row['unmeasured']=str(e);obligation_exception(obligation,e)
        finally:
            # Stop publication must never suppress the independent close attempt.
            try:
                Path(str(go)+'.stop').write_bytes(b'owned calibration complete')
                try:
                    row['child_input']=bounded_observation('origin input receipt',lambda:read_json(str(sideband)+'.input.json') if Path(str(sideband)+'.input.json').is_file() else None,3)
                except (Unavailable,OSError,ValueError) as error:row['input_unmeasured']=str(error)
            except Exception as error:
                row['stop_error']=str(error);obligation_exception(obligation,error)
            finally:row['cleanup']=close_owned(item,obligation)
        try:atomic_json(root/'evidence/origin-calibration.json',row)
        except Exception as error:
            row['publication_error']=str(error);obligation_exception(obligation,error)
        complete=row['cleanup'].get('complete') is True and not any(key in row for key in ('stop_error','publication_error','unmeasured','input_unmeasured'))
        settle_obligation(obligation,row,complete)
        if obligation['confinement'] is not True:break
    output.update({'subcases':records,'cleanup_complete':len(records)==2 and all(ledger['origin/'+r['mode']]['confinement'] is True for r in records),'discriminator':None,'release_verdict':'NONE',
            'meaning':'raw calibration only; no automatic attribution transfer to real receiver'})
    return output

def passive_interval(receiver, seconds=2):
    """Fixed observation window, not a retry-until-good loop or a write."""
    frames=[];end=time.monotonic()+seconds
    while True:
        frame=receiver.passive_frame()
        frames.append({'controller_ns':time.monotonic_ns(),'trace_seq':receiver.trace_sequence(),'frame':frame})
        if time.monotonic()>=end:break
        time.sleep(.05)
    return frames

def open_control(receiver, config, grant, parent, baseline, before,ledger,output=None):
    # The OPEN branch uses the existing startup receiver. An independent
    # NOT-OPEN child gets the same baseline and perturbation, never OPEN.
    # All observations inside the interval are read-only; Ctrl-G occurs ONLY
    # before OPEN or after the designated CLOSE boundary.
    sentinel='PERTURB_'+uuid.uuid4().hex[:12]
    row={} if output is None else output
    row.update({'sentinel':sentinel,'oracle_method':'passive_render_plus_final_byte_exports',
         'origin':None,'eligibility':'SUPPLEMENTAL: real receiver, origin unknown'})
    plain=None;plain_obligation=None
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
        row['open_cleanup']=close_owned(receiver,ledger['primary'])
        if row['open_cleanup'].get('complete') is not True:raise Unavailable('OPEN cleanup incomplete')
        plain_obligation=pending_obligation(ledger,'open/plain')
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
    except Exception as e:
        row['unmeasured']=str(e)
        obligation_exception(ledger['open'],e)
        if plain_obligation is not None:obligation_exception(plain_obligation,e)
    finally:
        if plain:row['plain_cleanup']=close_owned(plain,plain_obligation)
        row['cleanup_complete']=row.get('open_cleanup',{}).get('complete') is True and plain is not None and row.get('plain_cleanup',{}).get('complete') is True and 'unmeasured' not in row
        atomic_json(receiver.root/'evidence/open-control.json',row)
    return row

def calibration_findings(pair,sentinel):
    """Each stream contributes facts even when its sibling is unavailable."""
    result={'counts':[],'observed_failures':[],'errors':[],'complete':True}
    for index,(session,raw) in enumerate(pair):
        findings=submission_findings(raw,session,sentinel);count=len(findings['events'])
        result['counts'].append(count)
        if index==0 and count>0:result['observed_failures'].append('unsubmitted_control_submitted')
        if index==1 and count>1:result['observed_failures'].append('positive_control_multiple_submissions')
        if findings['errors']:
            result['complete']=False
            result['errors'].extend({'control':index,'error':error} for error in findings['errors'])
    if len(pair)!=2:result['complete']=False;result['errors'].append('calibration pair incomplete')
    return result

def boundary_conclusion(initial,details):
    confinements=[]
    def collect(value):
        if type(value) is dict:
            if 'receiver' in value and ('daemon' in value or 'complete' in value):
                confinements.append(value.get('confinement'))
            elif 'confinement' in value:confinements.append(value['confinement'])
            for child in value.values():collect(child)
        elif type(value) is list:
            for child in value:collect(child)
    collect(details)
    if initial is False or any(v is False for v in confinements):return False
    if initial is not True or not confinements or any(v is not True for v in confinements):return None
    return True

def execute(config,grant,output,phases):
    if sys.platform!='win32':raise Unavailable('native probe cannot execute on this host')
    parent=Path(tempfile.gettempdir());premises=dict.fromkeys(('network_boundary','auth_free_startup','genuine_open_query','no_submit_calibration'))
    ledger={};details={'confinement_obligations':ledger};errors=[];network_control=None
    # No receiver is started before the actual boundary discrimination succeeds.
    phases.enter('network_boundary')
    network_obligation=pending_obligation(ledger,'network')
    try:
        network_control,details['network']=network_probe(config,grant,parent,ledger)
        settle_obligation(network_obligation,{'confinement':network_control,'observations':details['network']},True)
    except Exception as e:
        obligation_exception(network_obligation,e);errors.append({'phase':'network_boundary','error':str(e)})
    if network_control is True:
        phases.enter('auth_free_startup')
        receiver=None;primary_obligation=pending_obligation(ledger,'primary');active_obligation=primary_obligation
        try:
            receiver=new_receiver(config,grant,parent);details['receiver_root']=str(receiver.root);details['launch']=receiver.start()
            baseline='NATIVE_VISIBLE_'+receiver.run_id
            receiver.write(baseline);before=receiver.export()
            # A real editor export, not a static prompt/counter, establishes startup responsiveness.
            if before!=baseline.encode():
                premises['auth_free_startup']=False
                errors.append({'phase':'auth_free_startup','observed_failure':'startup_baseline_mismatch'})
            if before==baseline.encode():
                receiver.write('!');visible=receiver.export()
                receiver.key('Backspace');receiver.write('X');append_one=receiver.export()
                receiver.key('Backspace');receiver.key('Backspace');remove_one=receiver.export()
                receiver.write(baseline[-1]);restored=receiver.export()
                checks=calibrate_exports(baseline.encode(),before,visible,append_one,remove_one,b'!')
                checks['restored']=restored==before
                details['export_controls']={'checks':checks,'baseline_hex':before.hex(),'visible_hex':visible.hex(),
                    'append_hex':append_one.hex(),'remove_hex':remove_one.hex(),'restored_hex':restored.hex()}
                if all(checks.values()):premises['auth_free_startup']=True
                else:
                    premises['auth_free_startup']=None
                    errors.append({'phase':'auth_free_startup','observed_failure':'export_oracle_control_failed'})
            if premises['auth_free_startup']:
                phases.enter('origin_calibration')
                origin_obligation=pending_obligation(ledger,'origin');active_obligation=origin_obligation
                details['origin_calibration']={}
                details['origin_calibration']=origin_calibration(config,grant,parent,ledger,details['origin_calibration'])
                settle_obligation(origin_obligation,details['origin_calibration'],details['origin_calibration'].get('cleanup_complete') is True)
                if details['origin_calibration']['cleanup_complete'] is not True:raise Unavailable('origin cleanup incomplete; no further children')
                phases.enter('genuine_open_query')
                open_obligation=pending_obligation(ledger,'open');active_obligation=open_obligation
                details['query']={}
                details['query']=open_control(receiver,config,grant,parent,baseline,before,ledger,details['query'])
                settle_obligation(open_obligation,details['query'],details['query'].get('cleanup_complete') is True)
                premises['genuine_open_query']=None

        except Exception as e:
            obligation_exception(active_obligation,e);errors.append({'phase':'receiver','error':str(e)})
        finally:
            for failure in details.get('query',{}).get('known_failures',[]):errors.append({'phase':'genuine_open_query','observed_failure':failure})
            if receiver:
                details['cleanup']=close_owned(receiver,primary_obligation)
                if details['cleanup'].get('complete') is not True:
                    premises['auth_free_startup']=None;premises['genuine_open_query']=None
                    errors.append({'phase':'receiver_cleanup','error':'cleanup incomplete; calibration not started'})
        if premises['auth_free_startup'] is True and details.get('query',{}).get('cleanup_complete') is True:
            phases.enter('no_submit_calibration')
            pair=[];sentinel='SUBMIT_CALIBRATION_'+uuid.uuid4().hex
            try:
                for submit in (False,True):
                    submit_obligation=pending_obligation(ledger,'submit/'+str(submit))
                    item=new_receiver(config,grant,parent);records=None
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
                    except Exception as error:
                        obligation_exception(submit_obligation,error)
                        errors.append({'phase':'no_submit_calibration','submit_control':submit,'error':str(error)})
                    finally:
                        try:
                            pair.append((item.spec['session_id'],records))
                            partial=calibration_findings(pair,sentinel)
                            errors.extend({'phase':'no_submit_calibration','observed_failure':f} for f in partial['observed_failures'])
                        finally:
                            cleanup=close_owned(item,submit_obligation);details.setdefault('calibration_cleanup',[]).append(cleanup)
                    if cleanup.get('complete') is not True:raise Unavailable('calibration cleanup incomplete; no more children')
                findings=calibration_findings(pair,sentinel)
                details['submission_counts']=findings['counts']
                errors.extend({'phase':'no_submit_calibration','observed_failure':f} for f in findings['observed_failures'])
                errors.extend({'phase':'no_submit_calibration','error':e} for e in findings['errors'])
                counts=findings['counts']
                if findings['complete'] and counts[1]>0 and details.get('calibration_input_accepted')==[True,True]:
                    premises['no_submit_calibration']=counts==[0,1]

            except Exception as e:errors.append({'phase':'no_submit_calibration','error':str(e)})
    premises['network_boundary']=boundary_conclusion(network_control,details)
    result={**decide_feasibility(premises),'details':details,'errors':errors,'source':{'product_sha':grant.product_sha,'diagnostic_sha':grant.diagnostic_sha},'authorization_sha256':grant.digest}
    result['observed_failures'] = sorted(set(result['observed_failures']+[e['observed_failure'] for e in errors if 'observed_failure' in e]))
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
