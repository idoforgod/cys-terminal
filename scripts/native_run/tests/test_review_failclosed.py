import pathlib,sys,tempfile,types,unittest
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import probe,fixture
from boundary import Unavailable
class StartupExceptions(unittest.TestCase):
    def test_each_export_exception_keeps_startup_null(self):
        for index in (2,3,4,5):
            for error in (Unavailable('capture absent'),OSError('read failed'),ValueError('bad receipt'),probe.InvalidEvidence('oracle unavailable')):
                with self.subTest(export=index,error=type(error).__name__),tempfile.TemporaryDirectory() as d:
                    class Receiver:
                        run_id='owned';root=pathlib.Path(d);n=0
                        cleanup={'receiver':{'cleanup':{'confinement':True}},'confinement':True,'complete':True}
                        def start(self):return {}
                        def write(self,*a):pass
                        def key(self,*a):pass
                        def close(self):pass
                        def export(self):
                            self.n+=1
                            if self.n==index:raise error
                            return [b'NATIVE_VISIBLE_owned',b'NATIVE_VISIBLE_owned!',b'NATIVE_VISIBLE_ownedX',b'NATIVE_VISIBLE_owne',b'NATIVE_VISIBLE_owned'][self.n-1]
                    grant=types.SimpleNamespace(product_sha='a'*40,diagnostic_sha='b'*40,digest='c'*64)
                    phases=types.SimpleNamespace(enter=lambda name:None)
                    with patch.object(probe.sys,'platform','win32'),patch.object(probe,'network_probe',return_value=(True,[])),patch.object(probe,'new_receiver',return_value=Receiver()):
                        r=probe.execute({},grant,pathlib.Path(d)/'result.json',phases)
                    self.assertIn('auth_free_startup',r['unmeasured'])
class MissingConfinement(unittest.TestCase):
    def test_five_secondary_receipt_gaps_are_null(self):
        missing={'receiver':None,'daemon':{'root_exit_observed':True},'complete':False}
        shapes=[{'origin_calibration':{'subcases':[{'cleanup':missing}]}},
                {'origin_calibration':{'subcases':[{'cleanup':{'confinement':True}},{'cleanup':missing}]}},
                {'query':{'plain_cleanup':missing}},
                {'calibration_cleanup':[missing]},
                {'calibration_cleanup':[{'confinement':True},missing]}]
        for details in shapes:
            with self.subTest(details=details):self.assertIsNone(probe.boundary_conclusion(True,details))
    def test_fixture_missing_finish_publishes_explicit_null(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);(root/'evidence').mkdir()
            receiver=object.__new__(fixture.ProductReceiver);receiver.root=root;receiver.daemon=None
            with patch.object(fixture,'bounded_observation',side_effect=Unavailable('missing finish')):receiver.close()
            self.assertIn('confinement',receiver.cleanup);self.assertIsNone(receiver.cleanup['confinement'])
if __name__=='__main__':unittest.main()
