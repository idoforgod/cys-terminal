import pathlib,sys,tempfile,types,unittest
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import probe
class SecondaryExceptions(unittest.TestCase):
    def run_case(self,helper,error):
        with tempfile.TemporaryDirectory() as d:
            class Receiver:
                run_id='owned';root=pathlib.Path(d);n=0
                cleanup={'receiver':{'cleanup':{'confinement':True}},'confinement':True,'complete':True}
                def start(self):return {}
                def write(self,*a):pass
                def key(self,*a):pass
                def close(self):pass
                def export(self):
                    self.n+=1
                    return [b'NATIVE_VISIBLE_owned',b'NATIVE_VISIBLE_owned!',b'NATIVE_VISIBLE_ownedX',b'NATIVE_VISIBLE_owne',b'NATIVE_VISIBLE_owned'][self.n-1]
            grant=types.SimpleNamespace(product_sha='a'*40,diagnostic_sha='b'*40,digest='c'*64)
            phases=types.SimpleNamespace(enter=lambda name:None)
            good={'cleanup_complete':True,'subcases':[{'cleanup':Receiver.cleanup}]}
            with patch.object(probe.sys,'platform','win32'),patch.object(probe,'network_probe',return_value=(True,[{'cleanup':{'confinement':True}}])),patch.object(probe,'new_receiver',return_value=Receiver()),patch.object(probe,'origin_calibration',**({'side_effect':error} if helper=='origin' else {'return_value':good})),patch.object(probe,'open_control',side_effect=error):
                return probe.execute({},grant,pathlib.Path(d)/'result.json',phases)
    def test_origin_and_open_helper_exceptions_preserve_pending_obligation(self):
        for helper in ('origin','open'):
            for point in ('stop-write','close','evidence-publication'):
                with self.subTest(helper=helper,point=point):
                    result=self.run_case(helper,OSError(point+' after secondary start'))
                    self.assertIn('network_boundary',result['unmeasured'])


class ActualCleanupPaths(unittest.TestCase):
    def test_origin_stop_close_publication_faults_and_sticky_false(self):
        for point in ('stop','close','publication'):
            for observed in (True,False):
                with self.subTest(point=point,observed=observed),tempfile.TemporaryDirectory() as d:
                    parent=pathlib.Path(d);ledger={};instances=[];counter=[0]
                    def prepare(*unused):
                        root=parent/str(counter[0]);counter[0]+=1
                        (root/'child').mkdir(parents=True);(root/'evidence').mkdir()
                        return root,root/'inert.exe'
                    class Receiver:
                        def __init__(self,grant,root,spec):self.root=root;self.spec=spec;self.closed=0;instances.append(self)
                        def start(self):
                            name='origin/'+self.spec['control_args'][3]
                            self_test.assertIsNone(ledger[name]['confinement'])
                            return {}
                        def resize_sequence(self):return []
                        def observations(self):return {}
                        def close(self):
                            self.closed+=1;self.cleanup={'receiver':{'cleanup':{'confinement':observed}},'confinement':observed,'complete':True}
                            if point=='close':raise OSError('close publication failed')
                    self_test=self;original=pathlib.Path.write_bytes
                    def write(path,data):
                        if point=='stop' and path.name.endswith('.stop'):raise OSError('stop write failed')
                        return original(path,data)
                    def publish(path,value):
                        if point=='publication' and path.name=='origin-calibration.json':raise OSError('row publication failed')
                    grant=types.SimpleNamespace(mode='feasibility')
                    config={'receiver_helper':'inert','receiver_helper_sha256':'0'*64}
                    with patch.object(probe,'prepare_root',side_effect=prepare),patch.object(probe,'protect_native_root'),patch.object(probe,'register'),patch.object(probe,'ProductReceiver',Receiver),patch.object(probe,'bounded_observation',return_value={}),patch.object(pathlib.Path,'write_bytes',write),patch.object(probe,'atomic_json',side_effect=publish):
                        result=probe.origin_calibration(config,grant,parent,ledger)
                    self.assertEqual(instances[0].closed,1)
                    self.assertIs(ledger['origin/emit']['confinement'],None if observed else False)
                    self.assertFalse(result['cleanup_complete'])
    def test_fixture_stop_write_still_closes_daemon(self):
        import fixture
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);(root/'evidence').mkdir();calls=[]
            daemon=types.SimpleNamespace(close=lambda:calls.append('close'),cleanup={'root_exit_observed':True,'descendant_exit_receipt':True,'profile_retained':False,'errors':[]})
            item=object.__new__(fixture.ProductReceiver);item.root=root;item.daemon=daemon
            original=pathlib.Path.write_bytes
            def write(path,data):
                if path.name=='stop-receiver':raise OSError('stop unavailable')
                return original(path,data)
            with patch.object(pathlib.Path,'write_bytes',write):item.close()
            self.assertEqual(calls,['close']);self.assertIsNone(item.cleanup['confinement']);self.assertFalse(item.cleanup['complete'])
    def test_publication_error_cannot_be_cleared_by_idempotent_close(self):
        import fixture
        for observed in (True,False):
            with self.subTest(observed=observed),tempfile.TemporaryDirectory() as d:
                root=pathlib.Path(d);(root/'evidence').mkdir();clean={'root_exit_observed':True,'descendant_exit_receipt':True,'profile_retained':False,'errors':[]}
                item=object.__new__(fixture.ProductReceiver);item.root=root;item.daemon=types.SimpleNamespace(close=lambda:None,cleanup=clean)
                ledger={};slot=probe.pending_obligation(ledger,'child')
                with patch.object(fixture,'bounded_observation',return_value={'cleanup':{**clean,'confinement':observed}}),patch.object(fixture,'atomic_json',side_effect=OSError('publish failed')):
                    probe.close_owned(item,slot)
                probe.close_owned(item,slot)
                self.assertIs(slot['confinement'],None if observed else False)
                self.assertFalse(item.cleanup['complete'])
    def test_empty_exception_message_is_still_an_exception(self):
        slot=probe.pending_obligation({},'child')
        probe.obligation_exception(slot,OSError())
        probe.settle_obligation(slot,{'confinement':True},True)
        self.assertIsNone(slot['confinement'])
    def test_observed_false_survives_exception_and_later_true(self):
        ledger={};slot=probe.pending_obligation(ledger,'child')
        probe.settle_obligation(slot,{'confinement':False},True)
        probe.obligation_exception(slot,OSError('later error'))
        probe.settle_obligation(slot,{'confinement':True},True)
        self.assertIs(slot['confinement'],False)

class OpenActualPaths(unittest.TestCase):
    def test_plain_start_close_publication_and_positive_control(self):
        for point in ('none','constructor','start','close','publication'):
            with self.subTest(point=point),tempfile.TemporaryDirectory() as d:
                root=pathlib.Path(d);(root/'evidence').mkdir();ledger={};closed=[]
                probe.pending_obligation(ledger,'primary');group=probe.pending_obligation(ledger,'open')
                class Receiver:
                    sid=1
                    def __init__(self,plain=False):self.root=root;self.body=b'' if plain else b'BASE';self.pending=b'';self.plain=plain
                    def start(self):
                        test.assertIsNone(ledger['open/plain']['confinement'])
                        if point=='start':raise OSError('after plain child start')
                    def trace_sequence(self):return 0
                    def trace_events(self):return []
                    def write(self,text):
                        if text.startswith('\x1b[200~'):self.pending=text[6:].encode()
                        elif text=='\x1b[201~':self.body+=self.pending;self.pending=b''
                        else:self.body+=text.encode()
                    def export(self):return self.body
                    def resize_sequence(self):return []
                    def close(self):
                        closed.append(self.plain);self.cleanup={'confinement':True,'complete':True}
                        if self.plain and point=='close':raise OSError('plain close publication')
                test=self;primary=Receiver()
                def create(*args):
                    test.assertIsNone(ledger['open/plain']['confinement'])
                    if point=='constructor':raise OSError('plain preparation')
                    return Receiver(True)
                def frames(item,*args):return [{'frame':{'text':item.body.decode(),'composer':{'output_gen':2}}}]
                def publish(*args):
                    if point=='publication':raise OSError('open row publication')
                row={}
                with patch.object(probe,'new_receiver',side_effect=create),patch.object(probe,'passive_interval',side_effect=frames),patch.object(probe,'accepted_input',return_value=True),patch.object(probe,'atomic_json',side_effect=publish):
                    try:
                        value=probe.open_control(primary,{},None,root,'BASE',b'BASE',ledger,row)
                        probe.settle_obligation(group,value,value['cleanup_complete'])
                    except OSError as error:probe.obligation_exception(group,error)
                verdict=probe.boundary_conclusion(True,{'obligations':ledger})
                self.assertIs(verdict,True if point=='none' else None)
                if point!='constructor':self.assertIn(True,closed)

class SubmissionCallerPaths(unittest.TestCase):
    def test_each_submission_child_exception_and_complete_control(self):
        import json
        for target in (1,2):
            for point in ('none','constructor','start','close','transcript'):
                with self.subTest(target=target,point=point),tempfile.TemporaryDirectory() as d:
                    root=pathlib.Path(d);created=[]
                    class Receiver:
                        run_id='owned';sid=1
                        def __init__(self,index):self.index=index;self.root=root;self.body='';self.spec={'session_id':str(index)};self.submitted=False
                        def start(self):
                            if self.index==target and point=='start':raise OSError('after child start')
                            return {}
                        def write(self,text):self.body+=text
                        def key(self,key):
                            if key=='Backspace':self.body=self.body[:-1]
                            if key=='Return':self.submitted=True
                        def export(self):return self.body.encode()
                        def trace_sequence(self):return 0
                        def trace_events(self):return []
                        def close(self):
                            self.cleanup={'confinement':True,'complete':True}
                            if self.index==target and point=='close':raise OSError('child close publication')
                    def create(*args):
                        index=len(created);created.append(index)
                        if index==target and point=='constructor':raise OSError('child factory')
                        return Receiver(index)
                    def records(item):
                        if item.index==target and point=='transcript':raise OSError('transcript publication')
                        return (json.dumps({'type':'user','sessionId':str(item.index),'message':{'role':'user','content':item.body}}).encode() if item.submitted else b'')
                    grant=types.SimpleNamespace(product_sha='a'*40,diagnostic_sha='b'*40,digest='c'*64)
                    good={'cleanup_complete':True,'confinement':True}
                    with patch.object(probe.sys,'platform','win32'),patch.object(probe,'network_probe',return_value=(True,[{'confinement':True}])),patch.object(probe,'new_receiver',side_effect=create),patch.object(probe,'origin_calibration',return_value=good),patch.object(probe,'open_control',return_value=good),patch.object(probe,'passive_interval',return_value=[]),patch.object(probe,'application_records',side_effect=records),patch.object(probe,'accepted_input',return_value=True):
                        result=probe.execute({},grant,root/'result.json',types.SimpleNamespace(enter=lambda name:None))
                    if point=='none':
                        self.assertNotIn('network_boundary',result['unmeasured']);self.assertNotIn('no_submit_calibration',result['unmeasured'])
                    else:
                        self.assertIn('network_boundary',result['unmeasured'])
                        self.assertIsNone(result['details']['confinement_obligations']['submit/'+str(target==2)]['confinement'])

class DirectStartingPaths(unittest.TestCase):
    def test_primary_constructor_or_start_cannot_disappear(self):
        for point in ('constructor','start'):
            with self.subTest(point=point),tempfile.TemporaryDirectory() as d:
                root=pathlib.Path(d)
                def fail():raise OSError('primary '+point)
                receiver=types.SimpleNamespace(root=root,start=fail,close=lambda:None,cleanup={'confinement':True,'complete':True})
                grant=types.SimpleNamespace(product_sha='a'*40,diagnostic_sha='b'*40,digest='c'*64)
                with patch.object(probe.sys,'platform','win32'),patch.object(probe,'network_probe',return_value=(True,[{'confinement':True}])),patch.object(probe,'new_receiver',**({'side_effect':OSError('constructor')} if point=='constructor' else {'return_value':receiver})):
                    result=probe.execute({},grant,root/'result.json',types.SimpleNamespace(enter=lambda name:None))
                self.assertIsNone(result['details']['confinement_obligations']['primary']['confinement'])
                self.assertIn('network_boundary',result['unmeasured'])
    def test_direct_boundary_constructor_has_a_preexisting_obligation(self):
        import datetime,os
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);(root/'child').mkdir();(root/'evidence').mkdir();ledger={}
            class Canary:
                endpoint=('127.0.0.1',12345)
                def __enter__(self):return self
                def __exit__(self,*args):pass
            def launch(spec):
                self.assertIsNone(ledger['network/0']['confinement'])
                raise OSError('direct boundary constructor failed')
            grant=types.SimpleNamespace(latest_end_utc=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(minutes=5)).isoformat())
            config={'canary_literal_address':'192.0.2.1','receiver_helper':'inert','receiver_helper_sha256':'0'*64}
            with patch.object(probe,'LoopbackCanary',Canary),patch.object(probe,'prepare_root',return_value=(root,root/'inert.exe')),patch.object(probe,'protect_native_root'),patch.object(probe,'register'),patch.object(probe,'parent_connect',return_value={'connected':True}),patch.object(probe,'OwnedBoundary',side_effect=launch),patch.dict(os.environ,{'SystemRoot':'C:\\Windows'}):
                with self.assertRaises(OSError):probe.network_probe(config,grant,root,ledger)
            self.assertIsNone(ledger['network/0']['confinement'])

if __name__=='__main__':unittest.main()
