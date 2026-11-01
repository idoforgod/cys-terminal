"""Reviewer counterexamples, executed against the actual probe and collector."""
import pathlib,sys,json,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from probe import calibration_findings
from test_collector import OracleGateControls
import collector

def event(session):return json.dumps({'type':'user','sessionId':session,'message':{'role':'user','content':'CAL'}}).encode()+b'\n'
class ProbeRetention(unittest.TestCase):
    def test_positive_stream_missing(self):
        r=calibration_findings([('n',event('n')),('y',None)],'CAL')
        self.assertIn('unsubmitted_control_submitted',r['observed_failures']);self.assertTrue(r['errors'])
    def test_positive_schema_unbound(self):
        r=calibration_findings([('n',event('n')),('y',b'{"type":"queued_command"}\n')],'CAL')
        self.assertIn('unsubmitted_control_submitted',r['observed_failures']);self.assertTrue(r['errors'])
    def test_valid_failure_then_bad_tail(self):
        r=calibration_findings([('n',event('n')+b'BROKEN\n'),('y',event('y'))],'CAL')
        self.assertIn('unsubmitted_control_submitted',r['observed_failures']);self.assertTrue(r['errors'])
class MainRetention(OracleGateControls):
    def test_main_submission_survives_missing_calibration(self):
        self.put('application_events',event('m'))
        self.record['artifacts'].pop('calibration_yes_events')
        r=collector.assess(self.root,self.record,self.case,self.manifest)
        self.assertIn('application submission observed',r['observed_failures']);self.assertFalse(r['complete'])
    def test_main_submission_survives_missing_other_oracle(self):
        self.put('application_events',event('m'))
        self.record['artifacts'].pop('oracle_visible')
        r=collector.assess(self.root,self.record,self.case,self.manifest)
        self.assertIn('application submission observed',r['observed_failures']);self.assertFalse(r['complete'])
    def test_main_submission_survives_bad_tail(self):
        self.put('application_events',event('m')+b'BROKEN\n')
        r=collector.assess(self.root,self.record,self.case,self.manifest)
        self.assertIn('application submission observed',r['observed_failures']);self.assertFalse(r['complete'])
if __name__=='__main__':unittest.main()
