import pathlib,sys,unittest,json,tempfile
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import observer
class ObserverTests(unittest.TestCase):
    def test_wrong_boolean_never_truthiness(self):
        for value in ['false',1,0,[],None]:
            with self.subTest(value=value),self.assertRaises(observer.InvalidEvidence):observer.boolean(value,'axis')
    def test_byte_oracle_has_length_controls(self):
        good=observer.calibrate_exports(b'BASE',b'BASE',b'BASE!',b'BASEX',b'BAS',b'!')
        self.assertTrue(all(good.values()))
        bad=observer.calibrate_exports(b'BASE',b'BASE',b'BASE',b'BASE',b'BASE',b'!')
        self.assertFalse(bad['visible']);self.assertFalse(bad['append']);self.assertFalse(bad['remove'])
    def test_network_silence_is_not_calibration(self):
        self.assertFalse(observer.calibrated_no_submit(b'',b'',no_session='a',yes_session='b',sentinel='S'))
        record={'type':'user','sessionId':'b','message':{'role':'user','content':'S'}}
        self.assertTrue(observer.calibrated_no_submit(b'',json.dumps(record).encode(),no_session='a',yes_session='b',sentinel='S'))
    def test_flag_only_query_is_null(self):
        self.assertIsNone(observer.genuine_window([{'kind':'paste_open','mono_ns':1,'input_paste_open':True}],b'\x1b[6n',b'\x1b[1;1R'))
    def test_bad_artifact_path_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(observer.InvalidEvidence):observer.artifact(d,{'path':'../secret','sha256':'0'*64})


class OpenControls(unittest.TestCase):
    def frame(self,text):return {'text':text,'composer':{'output_gen':2}}
    def evaluate(self,a,b,**kw):
        args=dict(baseline=b'BASE',sentinel=b'TOKEN',open_final=b'BASETOKEN',plain_final=b'BASETOKEN',open_accepted=True,plain_accepted=True)
        args.update(kw)
        return observer.open_differential(self.frame(a),self.frame(b),**args)
    def test_discrimination_requires_both_arms_and_exports(self):
        self.assertIs(self.evaluate('BASE','BASETOKEN')['open_ack'],True)
        for a,b in [('BASE','BASE'),('BASETOKEN','BASETOKEN'),('BASETOKEN','BASE')]:
            self.assertIsNone(self.evaluate(a,b)['open_ack'])
        self.assertIsNone(self.evaluate('BASE','BASETOKEN',open_accepted=None)['open_ack'])
        row=self.evaluate('BASE','BASETOKEN',open_final=b'LOST')
        self.assertIsNone(row['open_ack']);self.assertTrue(row['preservation_failure'])
    def test_receipt_is_not_rpc_success_or_gap(self):
        rows=[{'seq':0,'surface':7,'kind':'native_write_all','data':{'requested':[65,66],'accepted':2,'error_kind':None}}]
        self.assertIs(observer.accepted_input(rows,7,-1,b'AB'),True)
        self.assertIsNone(observer.accepted_input([],7,-1,b'AB'))
        rows[0]['seq']=1
        self.assertIsNone(observer.accepted_input(rows,7,-1,b'AB'))
        rows[0]['seq']=0;rows[0]['data']['accepted']=None
        self.assertIsNone(observer.accepted_input(rows,7,-1,b'AB'))

if __name__=='__main__':unittest.main()
