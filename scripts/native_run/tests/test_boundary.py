import pathlib,sys,unittest,tempfile,json,datetime
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import boundary,controller
from protocol import sha
class BoundaryTests(unittest.TestCase):
    def test_no_native_api_on_nonwindows(self):
        if sys.platform!='win32':
            with self.assertRaises(boundary.Unavailable):boundary.WindowsAPI()
    def test_missing_authority_does_not_launch(self):
        with self.assertRaises(boundary.Unavailable):controller.ReceiverController(None,{})
    def test_mixed_failure_retained_with_null(self):
        r=controller.decide_feasibility({'network_boundary':False})
        self.assertIsNone(r['feasibility']);self.assertEqual(r['observed_failures'],['network_boundary']);self.assertEqual(r['native_hard_gate'],'OPEN')
    def test_wrong_types_rejected(self):
        for x in ['false',0,1,[]]:
            with self.assertRaises(ValueError):controller.decide_feasibility({'network_boundary':x})
    def test_private_copy_and_escape_guard(self):
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d);binary=p/'fixture.exe';binary.write_bytes(b'not executed')
            root,exe=controller.prepare_root(p,binary,sha(binary.read_bytes()),run_id='a'*32)
            spec={'run_root':str(root),'executable':str(exe),'executable_sha256':sha(exe.read_bytes()),'args':[],
                  'environment':controller.fixture_environment(root,'C:\\Windows'),'process_limit':8}
            self.assertEqual(boundary.validate_launch(spec)[1],exe)
            spec['executable']=str(binary)
            with self.assertRaises(boundary.Unavailable):boundary.validate_launch(spec)
    def test_authority_wrong_bool_and_attempt(self):
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'auth.json';record={'mode':'feasibility','diagnostic_sha':'a'*40,'product_sha':'b'*40,'attempt':True,'release_verdict':'NONE','execution_authorized':True,'host':'windows-latest','earliest_start_utc':'2026-01-01T00:00:00+00:00','latest_end_utc':'2027-01-01T00:00:00+00:00'}
            p.write_text(json.dumps(record))
            with self.assertRaises(boundary.Unavailable):controller.authorize(p,sha(p.read_bytes()),mode='feasibility',diagnostic_sha='a'*40,product_sha='b'*40,now=datetime.datetime(2026,2,1,tzinfo=datetime.timezone.utc))
if __name__=='__main__':unittest.main()
