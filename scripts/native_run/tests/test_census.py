import copy,pathlib,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from census import load,push_match,exact_candidate,Refused
from manifest import workflow
class CensusTests(unittest.TestCase):
    def setUp(self):self.branch='work/probe-exact';self.w=workflow({'mode':'feasibility','branch':self.branch})
    def test_literal_kind_and_prefix_controls(self):
        exact_candidate(self.w,self.branch)
        self.assertTrue(push_match(self.w,'branch',self.branch))
        for kind,ref in [('branch',self.branch+'-suffix'),('branch','work/unrelated-probe'),('branch','main'),('branch','fix/0.14.56-candidate'),('tag',self.branch),('tag','v0.14.56')]:self.assertFalse(push_match(self.w,kind,ref))
    def test_broad_mutant_is_rejected(self):
        self.w['on']['push']['branches']=['work/**']
        self.assertTrue(push_match(self.w,'branch','work/unrelated-probe'))
        with self.assertRaises(Refused):exact_candidate(self.w,self.branch)
    def test_on_is_not_yaml_boolean(self):self.assertEqual(load("on:\n  push:\n    branches: ['work/probe-exact']\n")['on'],{'push':{'branches':['work/probe-exact']}})
    def test_unknown_syntax_is_not_zero(self):
        for value in ['${{ vars.BRANCH }}','work/[a-z]*']:
            with self.assertRaises(Refused):push_match({'on':{'push':{'branches':[value]}}},'branch',self.branch)
    def test_indirect_chain_refuses(self):
        with self.assertRaises(Refused):push_match({'on':{'workflow_run':{}}},'branch',self.branch)
    def test_resource_allocations(self):
        w=workflow({'mode':'main','branch':self.branch});job=w['jobs']['native'];rows=job['strategy']['matrix']['include']
        self.assertEqual(len(rows),14);self.assertEqual(sum(r['minutes'] for r in rows),1190);self.assertEqual(job['strategy']['max-parallel'],10)

class SyntaxControls(unittest.TestCase):
    def test_duplicate_keys_refused(self):
        with self.assertRaises(Refused):load("on:\n  push:\n    branches: ['work/**']\n    branches: ['work/probe-exact']\n")
    def test_shell_metachar_ref_refused(self):
        for branch in ["work/x;whoami","work/x'","work/x&y","work/x`y"]:
            w=workflow({'mode':'feasibility','branch':branch})
            with self.assertRaises(Refused):exact_candidate(w,branch)

if __name__=='__main__':unittest.main()
