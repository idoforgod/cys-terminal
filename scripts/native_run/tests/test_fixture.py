import pathlib,sys,tempfile,unittest,json
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import fixture,job
from protocol import sha
from boundary import Unavailable
class CompletionTests(unittest.TestCase):
    def test_editor_file_appearance_is_not_completion(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);captures=root/'child/captures';captures.mkdir(parents=True);(root/'evidence').mkdir()
            receiver=object.__new__(fixture.ProductReceiver);receiver.root=root
            receiver.finish_editor=lambda *unused:None;receiver.returned_composer=lambda data,before:{'test_only':True}
            receiver.key=lambda unused:(captures/'one.bin').write_bytes(b'partial')
            def await_complete(description,probe,seconds):
                self.assertIsNone(probe())
                (captures/'one.bin').write_bytes(b'complete')
                (captures/'one.json').write_text(json.dumps({'bytes':8,'sha256':sha(b'complete')}))
                return probe()
            with patch.object(fixture,'bounded_observation',side_effect=await_complete):
                self.assertEqual(receiver.export(),b'complete')
    def test_collection_error_overrides_worker_success(self):
        with patch.object(job,'read_json',return_value={'digest':'x','context':{},'job_start_utc':'unused'}),patch.object(pathlib.Path,'read_bytes',return_value=b'authority'),patch.object(job,'checked_authority',return_value={'mode':'feasibility','latest_end_utc':'unused'}),patch.object(job,'bind_manifest',return_value={}),patch('supervisor.run_owned_worker',return_value={'exit_code':0}),patch('archive.collect',return_value={'errors':['missing artifact']}):
            with self.assertRaisesRegex(Unavailable,'artifact collection incomplete'):job.execute('feasibility')
if __name__=='__main__':unittest.main()
