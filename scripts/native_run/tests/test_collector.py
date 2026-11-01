"""The actual collector, not the design simulation, owns these rejection contracts."""
import importlib.util
import pathlib
import sys
import unittest
ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import protocol

class ProtocolTests(unittest.TestCase):
    def test_fixed_population_and_cost(self):
        rows = protocol.plan()
        self.assertEqual(len({r['id'] for r in rows}), 740)
        self.assertEqual(protocol.cost()['runner_minutes'], 1190)
        self.assertEqual(len(protocol.matrix()), 14)


import tempfile,json,copy
import collector

class CollectorRejectionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=pathlib.Path(self.temp.name)
        self.case={'id':'I/000','arm':'I','tests':['the::target'],'ignored':[],'filtered':9}
        self.manifest={'product_sha':'a'*40,'diagnostic_sha':'b'*40,'run_id':'run1','attempt':1,'release_verdict':'NONE'}
        self.record={'id':'I/000',**self.manifest,'rc':0,'exit_observed':True,'c8':'NO_UNMEASURED_MARKER','artifacts':{}}
        self.put('stdout',b'test the::target ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.1s\n')
    def tearDown(self):self.temp.cleanup()
    def put(self,key,data):
        p=self.root/key;p.write_bytes(data);self.record['artifacts'][key]={'path':key,'sha256':protocol.sha(data)}
    def assess(self):return collector.assess(self.root,self.record,self.case,self.manifest)
    def test_valid_single_test_control(self):self.assertEqual(self.assess()['outcome'],'PASS')
    def test_wrong_types_are_rejected(self):
        for key,value in [('attempt',True),('attempt','1'),('rc',False),('rc','0'),('exit_observed','false')]:
            old=self.record[key];self.record[key]=value
            with self.subTest(key=key,value=value):self.assertFalse(self.assess()['complete'])
            self.record[key]=old
    def test_wrong_sha_and_attempt(self):
        for key,value in [('product_sha','c'*40),('diagnostic_sha','d'*40),('attempt',2),('run_id','other')]:
            old=self.record[key];self.record[key]=value
            with self.subTest(key=key):self.assertFalse(self.assess()['complete'])
            self.record[key]=old
    def test_child_green_cannot_replace_parent_red(self):
        self.record['rc']=101
        self.put('stdout',b'test child ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out; finished in 0.1s\ntest the::target ... FAILED\nfailures:\n\n    the::target\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.1s\n')
        r=self.assess();self.assertTrue(r['complete']);self.assertEqual(r['outcome'],'FAIL');self.assertEqual(r['observed_failures'],['the::target'])
    def test_ambiguous_parent_is_null(self):
        raw=(self.root/'stdout').read_bytes();self.put('stdout',raw+raw);self.assertIsNone(self.assess()['outcome'])
    def test_hash_and_missing_artifact_reject(self):
        (self.root/'stdout').write_bytes(b'changed');self.assertFalse(self.assess()['complete'])
        self.record['artifacts']={};self.assertFalse(self.assess()['complete'])
    def test_wrong_expected_inventory_reject(self):
        self.case['tests']=[];self.assertFalse(self.assess()['complete'])
    def test_mixed_known_failure_survives_null(self):
        self.case={'id':'R/00/dsr_outer','arm':'R','cell':'dsr_outer'};self.record['id']=self.case['id']
        self.put('composer_before',b'KEEP');self.put('composer_after',b'CORRUPTED')
        self.put('trace',json.dumps({'native_windows':True,'cleanup_complete':True,'oracle_ready':True,'platform':'windows','transport':'conpty','query_origin':None}).encode())
        r=self.assess();self.assertFalse(r['complete']);self.assertEqual(r['outcome'],'FAIL');self.assertIn('composer preservation violation',r['observed_failures'])
    def test_duplicate_and_missing_never_shrink_denominator(self):
        manifest={**self.manifest,'cases':protocol.plan()}
        # No records are still the complete PLANNED population, with NULL rates.
        r=collector.collect(self.root,manifest,[])
        self.assertEqual(r['arms']['I']['planned'],300);self.assertIsNone(r['arms']['I']['failure_rate'])
        manifest['cases'][0].update(self.case)
        r=collector.collect(self.root,manifest,[self.record,copy.deepcopy(self.record)])
        self.assertIn('duplicate planned ID',r['trials'][0]['issues']);self.assertIsNone(r['arms']['I']['failure_rate'])
    def test_witness_owner_comes_only_from_internal_header(self):
        self.assertEqual(collector.witness_owner(b'test actual::owner ... FAILED\n'),'actual::owner')
        with self.assertRaises(collector.Rejected):collector.witness_owner(b'filename says another owner\n')
    def test_nonempty_wrong_inventory_rejects(self):
        self.case['tests']=['wrong::name'];self.assertFalse(self.assess()['complete'])
    def test_unplanned_failure_is_retained(self):
        self.put('stdout',b'test real::failure ... FAILED\n');self.record['id']='I/unplanned'
        manifest={**self.manifest,'cases':protocol.plan()}
        r=collector.collect(self.root,manifest,[self.record]);self.assertTrue(r['unattributed_failures']);self.assertFalse(r['complete'])
    def test_wilson_boundary(self):
        self.assertAlmostEqual(collector.wilson(0,300)[1],0.012643,places=6)


class OracleGateControls(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=pathlib.Path(self.tmp.name)
        self.case={'id':'R/00/baseline','arm':'R','cell':'baseline','calibration_sentinel':'CAL'}
        self.manifest={'product_sha':'a'*40,'diagnostic_sha':'b'*40,'run_id':'unit-synthetic','attempt':1,'release_verdict':'NONE'}
        self.record={'id':self.case['id'],**self.manifest,'rc':0,'exit_observed':True,'artifacts':{}}
        trace={'native_windows':True,'cleanup_complete':True,'oracle_ready':True,'platform':'windows','transport':'conpty',
               'request_id':'same','original_request_id':'same','body_offset_before':4,'body_offset_after':4,
               'original_proof_admissions':1,'successor_proof_admissions':0,'network_boundary':True,
               'open_interval_discriminated':True,'application_sessions':{'main':'m','no':'n','yes':'y'}}
        self.put('trace',json.dumps(trace).encode())
        for key,value in {'composer_before':b'BODY','composer_after':b'BODY','sentinel':b'!',
                          'oracle_seed':b'BASE','oracle_baseline':b'BASE','oracle_visible':b'BASE!',
                          'oracle_append':b'BASEX','oracle_remove':b'BAS','protected_input':b'body',
                          'application_events':b'','calibration_no_events':b'',
                          'calibration_yes_events':json.dumps({'type':'user','sessionId':'y','message':{'role':'user','content':'CAL'}}).encode()}.items():self.put(key,value)
    def put(self,key,data):
        (self.root/key).write_bytes(data);self.record['artifacts'][key]={'path':key,'sha256':protocol.sha(data)}
    def tearDown(self):self.tmp.cleanup()
    def test_synthetic_positive_contract(self):self.assertEqual(collector.assess(self.root,self.record,self.case,self.manifest)['outcome'],'PASS')
    def test_constant_oracle_rejected_by_actual_collector(self):
        self.put('oracle_visible',b'BASE');self.put('oracle_append',b'BASE');self.put('oracle_remove',b'BASE')
        r=collector.assess(self.root,self.record,self.case,self.manifest)
        self.assertFalse(r['complete']);self.assertIn('oracle control failed: oracle_append',r['issues'])
    def test_unobserved_submission_calibration_not_zero(self):
        self.put('calibration_yes_events',b'');r=collector.assess(self.root,self.record,self.case,self.manifest)
        self.assertFalse(r['complete']);self.assertIn('no-submit oracle uncalibrated',r['issues'])

if __name__=='__main__':unittest.main()
