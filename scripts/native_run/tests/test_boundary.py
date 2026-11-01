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


class AuditControls(unittest.TestCase):
    def test_authority_uses_the_hashed_bytes(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'auth.json'
            raw={'mode':'feasibility','diagnostic_sha':'a'*40,'product_sha':'b'*40,'attempt':1,'release_verdict':'NONE','execution_authorized':False,'host':'windows-latest','earliest_start_utc':'2026-01-01T00:00:00+00:00','latest_end_utc':'2027-01-01T00:00:00+00:00'}
            path.write_text(json.dumps(raw));changed={**raw,'execution_authorized':True}
            with patch.object(controller,'read_json',return_value=changed):
                with self.assertRaises(boundary.Unavailable):controller.authorize(path,sha(path.read_bytes()),mode='feasibility',diagnostic_sha='a'*40,product_sha='b'*40,now=datetime.datetime(2026,2,1,tzinfo=datetime.timezone.utc))
    def test_protected_acl_has_no_app_write_outside_child(self):
        owner='S-1-5-21-123';app='S-1-15-2-456'
        evidence=boundary.protected_sddl(owner,app,'evidence',True)
        binary=boundary.protected_sddl(owner,app,'bin',True)
        child=boundary.protected_sddl(owner,app,'child',True)
        self.assertTrue(evidence.startswith('D:P'));self.assertNotIn(app,evidence)
        self.assertIn(';RX;;;'+app,binary);self.assertNotIn('0x1301bf',binary)
        self.assertIn('0x1301bf',child)
    def test_cleanup_never_defaults_to_complete(self):
        valid={'root_exit_observed':True,'descendant_exit_receipt':True,'profile_retained':False,'errors':[]}
        self.assertTrue(boundary.cleanup_valid(valid))
        for key,value in [('root_exit_observed',None),('descendant_exit_receipt',False),('profile_retained',True),('errors',['close failed'])]:
            self.assertFalse(boundary.cleanup_valid({**valid,key:value}))

if __name__=='__main__':unittest.main()
