import importlib.util,json,pathlib,sys,tempfile,unittest,subprocess
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
spec=importlib.util.spec_from_file_location('native_setup',pathlib.Path(__file__).resolve().parents[1]/'setup.py')
setup=importlib.util.module_from_spec(spec);spec.loader.exec_module(setup)
class SetupDiagnostics(unittest.TestCase):
    def test_failed_command_receipt_has_argv_rc_timeout(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'receipt.json';args=['compiler','source.cs']
            with patch.object(setup.subprocess,'run',return_value=subprocess.CompletedProcess(args,17)):
                result=setup.build_command(args,path,timeout=60)
            self.assertEqual(result.returncode,17)
            row=json.loads(path.read_text());self.assertEqual(row['argv'],args);self.assertEqual(row['returncode'],17);self.assertEqual(row['timeout_s'],60)
    def test_exception_retains_started_command(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'receipt.json';args=['compiler','source.cs']
            with patch.object(setup.subprocess,'run',side_effect=subprocess.TimeoutExpired(args,60)):
                with self.assertRaises(subprocess.TimeoutExpired):setup.build_command(args,path,timeout=60)
            row=json.loads(path.read_text());self.assertEqual(row['state'],'exception');self.assertEqual(row['exception'],'TimeoutExpired');self.assertEqual(row['argv'],args);self.assertIsNone(row['returncode'])
if __name__=='__main__':unittest.main()
