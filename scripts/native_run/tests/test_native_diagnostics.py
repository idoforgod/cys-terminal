import ctypes as C, json, pathlib, sys, tempfile, unittest
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from boundary import WindowsAPI, Unavailable
class NativeDiagnostics(unittest.TestCase):
    def api(self,path):
        api=WindowsAPI.__new__(WindowsAPI);api.audit_path=path;api.audit_error=None;api.last_call=None;api.call_index=0
        return api
    def test_native_failure_retains_arguments_result_and_last_error(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'calls.jsonl';api=self.api(path)
            with patch.object(C,'get_last_error',return_value=5,create=True),patch.object(C,'set_last_error',create=True):
                call=api.bind_call('OpenProcess',lambda rights,inherit,pid:0)
                result=call(0x1000,False,42)
                with self.assertRaises(Unavailable) as caught:api.check(result,'open editor')
            self.assertIn('OpenProcess',str(caught.exception));self.assertIn('42',str(caught.exception))
            self.assertTrue(path.exists(),'native-call journal missing')
            rows=[json.loads(x) for x in path.read_text().splitlines()]
            self.assertEqual(rows[-1]['last_error'],5);self.assertEqual(rows[-1]['result'],0)
            self.assertEqual(rows[0]['stage'],'enter');self.assertEqual(rows[-1]['stage'],'return')
    def test_output_and_error_preserved_without_credentials(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'calls.jsonl';api=self.api(path);size=C.c_uint32()
            def fn(*args):size.value=56;return 1
            with patch.object(C,'get_last_error',return_value=122,create=True),patch.object(C,'set_last_error',create=True) as restore:
                self.assertEqual(api.bind_call('GetTokenInformation',fn)(123,29,None,0,C.byref(size)),1)
                self.assertEqual(api.bind_call('CreateProcessW',fn)('owned.exe','secret-command',None,None,False,4,'secret-env','owned',None,None),1)
            self.assertTrue(path.exists(),'native-call journal missing')
            raw=path.read_text();self.assertNotIn('secret-command',raw);self.assertNotIn('secret-env',raw)
            rows=[json.loads(x) for x in raw.splitlines()]
            self.assertEqual(rows[1]['arguments'][4]['byref'],56)
            restore.assert_called_with(122)
    def test_call_exception_has_its_own_operands_not_a_stale_call(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'calls.jsonl';api=self.api(path)
            def broken(*args):raise TypeError('bad native argument')
            with patch.object(C,'get_last_error',return_value=87,create=True),patch.object(C,'set_last_error',create=True):
                with self.assertRaises(TypeError):api.bind_call('GetAce',broken)(42,9,None)
            row=json.loads(path.read_text().splitlines()[-1])
            self.assertEqual(row['stage'],'exception');self.assertEqual(row['arguments'],[42,9,None]);self.assertEqual(row['exception'],'TypeError')

    def test_logging_failure_cannot_lose_a_returned_handle(self):
        with tempfile.TemporaryDirectory() as d:
            api=self.api(pathlib.Path(d)/'absent'/'calls.jsonl')
            with patch.object(C,'get_last_error',return_value=0,create=True),patch.object(C,'set_last_error',create=True):
                self.assertEqual(api.bind_call('CreateJobObjectW',lambda *a:777)(None,None),777)
            self.assertIsNotNone(api.audit_error,'missing audit must be latched')
            with self.assertRaisesRegex(Unavailable,'audit unavailable'):api.ensure_audit()

class BindingCoverage(unittest.TestCase):
    def test_every_declared_native_function_gets_the_journal_wrapper(self):
        import types
        bound=[];loaded=[]
        class DLL:
            def __getattr__(self,name):
                def fn(*args):return 1
                loaded.append(name);return fn
        def wrap(api,name,fn):bound.append(name);return fn
        with patch('boundary.sys.platform','win32'),patch.object(C,'WinDLL',side_effect=lambda *a,**kw:DLL(),create=True),patch.object(WindowsAPI,'bind_call',wrap):
            WindowsAPI('unused')
        self.assertEqual(bound,loaded);self.assertEqual(len(bound),len(set(bound)))
        self.assertIn('GetAce',bound);self.assertIn('CreateProcessW',bound);self.assertIn('CloseHandle',bound)

class SizingDiagnostics(unittest.TestCase):
    def test_sizing_refusal_retains_result_error_and_size(self):
        import types
        for result,error,size in [(1,0,16),(0,5,16),(0,122,0),(0,122,2**24)]:
            api=WindowsAPI.__new__(WindowsAPI);api.last_call=None;api.audit_error=None
            def call(handle,kind,buf,capacity,out):
                C.cast(out,C.POINTER(C.c_uint32))[0]=size;return result
            api.a=types.SimpleNamespace(GetTokenInformation=call)
            with patch.object(C,'get_last_error',return_value=error,create=True):
                with self.assertRaises(Unavailable) as caught:api.token_info(7,29,4)
            self.assertIn('"size": '+str(size),str(caught.exception));self.assertIn('"error": '+str(error),str(caught.exception))
    def test_successful_size_then_oversized_output_refused(self):
        import types
        api=WindowsAPI.__new__(WindowsAPI);api.last_call=None;api.audit_error=None
        def call(handle,kind,buf,capacity,out):
            C.cast(out,C.POINTER(C.c_uint32))[0]=4 if buf is None else 8
            return 0 if buf is None else 1
        api.a=types.SimpleNamespace(GetTokenInformation=call)
        with patch.object(C,'get_last_error',return_value=122,create=True):
            with self.assertRaisesRegex(Unavailable,'output size refused'):api.token_info(7,29,4)

if __name__=='__main__':unittest.main()
