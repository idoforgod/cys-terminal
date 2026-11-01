import ctypes as C,json,pathlib,sys,tempfile,unittest
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import archive
from boundary import WindowsAPI,OwnedBoundary,PROCESS_INFORMATION,Unavailable,cleanup_valid
from protocol import sha

class AuditPresence(unittest.TestCase):
    def api(self,path):
        api=WindowsAPI.__new__(WindowsAPI);api.audit_path=path;api.audit_error=None;api.last_call=None;api.call_index=0
        return api
    def assert_cleanup_refuses(self,api):
        owner=OwnedBoundary.__new__(OwnedBoundary)
        owner.api=api;owner.closed=False;owner.job=None;owner.process=PROCESS_INFORMATION()
        owner.assigned=False;owner.sid=None;owner.profile_created=False
        owner.close()
        self.assertFalse(cleanup_valid(owner.cleanup),owner.cleanup)
        self.assertTrue(any('native_audit_unavailable' in x for x in owner.cleanup['errors']))
    def test_empty_message_enter_and_return_failures_are_sticky(self):
        for stage in (1,2):
            for failure in (MemoryError(),OSError()):
                with self.subTest(stage=stage,failure=type(failure).__name__),tempfile.TemporaryDirectory() as d:
                    api=self.api(pathlib.Path(d)/'calls');actual_open=pathlib.Path.open;calls=[]
                    def opened(path,*args,**kwargs):
                        calls.append(path)
                        if len(calls)==stage:raise failure
                        return actual_open(path,*args,**kwargs)
                    with patch.object(pathlib.Path,'open',opened),patch.object(C,'get_last_error',return_value=0,create=True),patch.object(C,'set_last_error',create=True):
                        handle=api.bind_call('CreateJobObjectW',lambda *args:777)(None,None)
                    self.assertEqual(handle,777,'ownership must reach the caller')
                    with self.assertRaises(Unavailable):api.ensure_audit()
                    self.assert_cleanup_refuses(api)
    def test_comparison_publication_and_later_empty_failure(self):
        with tempfile.TemporaryDirectory() as d:
            api=self.api(pathlib.Path(d)/'calls')
            with patch.object(pathlib.Path,'open',side_effect=MemoryError()):api.audit({'stage':'DACL comparison'})
            with self.assertRaises(Unavailable):api.ensure_audit()
            self.assert_cleanup_refuses(api)
            api=self.api(pathlib.Path(d)/'calls')
            with patch.object(pathlib.Path,'open',side_effect=OSError('first failure')):api.audit({'stage':'enter'})
            first=api.audit_error
            with patch.object(pathlib.Path,'open',side_effect=MemoryError()):api.audit({'stage':'return'})
            self.assertEqual(api.audit_error,first,'later failure must not erase first failure')
            with self.assertRaises(Unavailable):api.ensure_audit()

class OriginArchive(unittest.TestCase):
    def fixture(self,parent,reference=True,journal=True):
        parent=pathlib.Path(parent).resolve();root=parent/('cys-native-'+'a'*32);root.mkdir();(root/'child').mkdir()
        output=parent/'collected';output.mkdir()
        (root/'owned-run.json').write_text(json.dumps({'run_id':'a'*32,'kind':'native_fixture'}))
        (output/'owned-roots.json').write_text(json.dumps([str(root)]))
        path=root/'child/origin.native.jsonl';raw=b'{"name":"ReadConsoleInputW","last_error":6,"operands":{"capacity":32}}\n'
        if journal:path.write_bytes(raw)
        if reference:(root/'child/origin.input.json').write_text(json.dumps({'error':'read failed; operands/result in '+str(path)}))
        return root,output,path,raw
    def test_referenced_journal_bytes_and_digest_survive_real_collect(self):
        with tempfile.TemporaryDirectory() as d:
            root,output,path,raw=self.fixture(d)
            result=archive.collect(output);self.assertEqual(result['errors'],[])
            rows=[x for x in result['artifacts'] if x['original']==str(path)]
            self.assertEqual(len(rows),1,'referenced JSONL omitted')
            self.assertEqual((output/rows[0]['path']).read_bytes(),raw)
            self.assertEqual(rows[0]['sha256'],sha(raw));self.assertEqual(rows[0]['bytes'],len(raw))
    def test_missing_referenced_journal_is_an_archive_error(self):
        with tempfile.TemporaryDirectory() as d:
            root,output,path,raw=self.fixture(d,journal=False)
            result=archive.collect(output)
            self.assertTrue(result['errors'],'missing operands silently accepted')
    def test_referenced_journal_symlink_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            root,output,path,raw=self.fixture(d,journal=False)
            other=pathlib.Path(d)/'external';other.write_bytes(raw);path.symlink_to(other)
            result=archive.collect(output)
            self.assertTrue(result['errors'],'journal escaped containment')
    def test_unexpected_operand_reference_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            root,output,path,raw=self.fixture(d)
            (root/'child/origin.input.json').write_text(json.dumps({'error':'read failed; operands/result in '+str(root/'child/missing.jsonl')}))
            self.assertTrue(archive.collect(output)['errors'])
    def test_early_failure_without_origin_does_not_require_a_journal(self):
        with tempfile.TemporaryDirectory() as d:
            root,output,path,raw=self.fixture(d,reference=False,journal=False)
            self.assertEqual(archive.collect(output)['errors'],[])

if __name__=='__main__':unittest.main()
