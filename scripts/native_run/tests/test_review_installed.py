import pathlib,sys,json,inspect,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from repo_fixture import Repo,base_files
import census,manifest
from protocol import sha
class InstalledTipTests(unittest.TestCase):
    def setUp(self):
        self.repo=Repo();self.branch='work/probe';self.files=base_files();self.product=self.repo.commit(self.files)
        self.raw=(json.dumps(manifest.workflow({'mode':'feasibility','branch':self.branch}),ensure_ascii=False,indent=2)+'\n').encode()
        self.path='.github/workflows/diag-native-feasibility.yml';self.expected={'path':self.path,'sha256':sha(self.raw)}
        self.candidate=self.repo.root/'reviewed.yml';self.candidate.write_bytes(self.raw)
        self.files[self.path]=self.raw;self.tip=self.repo.commit(self.files,self.product)
    def tearDown(self):self.repo.close()
    def test_final_census_accepts_itself_exactly_once(self):
        kwargs={'installed_candidate':self.expected} if 'installed_candidate' in inspect.signature(census.census).parameters else {}
        try:r=census.census(self.repo.root,self.tip,self.branch,self.candidate,**kwargs)
        except census.Refused as error:r=None;reason=str(error)
        self.assertIsNotNone(r,locals().get('reason'));self.assertTrue(r['P1']);self.assertEqual(r['prospective_population'],4)
    def test_final_manifest_allows_exact_reviewed_workflow_only(self):
        kwargs={'installed_candidate':self.expected} if 'installed_candidate' in inspect.signature(manifest.make).parameters else {}
        try:r=manifest.make(self.repo.root,self.product,self.tip,self.branch,'feasibility',**kwargs)
        except ValueError as error:r=None;reason=str(error)
        self.assertIsNotNone(r,locals().get('reason'))


class InstalledNegatives(InstalledTipTests):
    def test_duplicate_candidate_is_not_excluded_as_another_self(self):
        files={**self.files,'.github/workflows/duplicate.yml':self.raw};tip=self.repo.commit(files,self.product)
        with self.assertRaises(census.Refused):census.census(self.repo.root,tip,self.branch,self.candidate,installed_candidate=self.expected)
        with self.assertRaises(ValueError):manifest.make(self.repo.root,self.product,tip,self.branch,'feasibility',self.expected)
    def test_altered_candidate_fails_exact_digest(self):
        files={**self.files,self.path:self.raw+b'\n'};tip=self.repo.commit(files,self.product)
        with self.assertRaises(census.Refused):census.census(self.repo.root,tip,self.branch,self.candidate,installed_candidate=self.expected)
        with self.assertRaises(ValueError):manifest.make(self.repo.root,self.product,tip,self.branch,'feasibility',self.expected)

if __name__=='__main__':unittest.main()
