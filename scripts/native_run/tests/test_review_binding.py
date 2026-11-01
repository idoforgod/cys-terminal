import pathlib,sys,json,datetime,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import job
from protocol import sha
from boundary import Unavailable
class AuthorityManifestTests(unittest.TestCase):
    def test_digest_bound_authority_without_reviewed_manifest_is_refused(self):
        now=datetime.datetime.now(datetime.timezone.utc)
        doc={'product_sha':'0'*40,'diagnostic_sha':'b'*40,'execution_authorized':True,'release_verdict':'NONE','attempt':1,'mode':'feasibility','branch':'work/probe','host':'windows-latest','rust_toolchain':'1.90.0','earliest_start_utc':(now-datetime.timedelta(minutes=1)).isoformat(),'latest_end_utc':(now+datetime.timedelta(minutes=5)).isoformat()}
        raw=json.dumps(doc).encode();context={'mode':'feasibility','branch':'work/probe','attempt':'1','event':'push','ref':'refs/heads/work/probe','sha':'b'*40,'runner_os':'Windows'}
        with self.assertRaises(Unavailable):job.checked_authority(raw,sha(raw),context)


from repo_fixture import Repo,base_files
import manifest,subprocess,copy
class RuntimeBindingTests(unittest.TestCase):
    def setUp(self):
        self.repo=Repo();self.files=base_files();self.product=self.repo.commit(self.files);self.branch='work/bound'
        for name in ('boundary.py','boundary_host.py','controller.py','protocol.py'):self.files['scripts/native_run/'+name]=b'# inert source fixture\n'
        raw=(json.dumps(manifest.workflow({'mode':'feasibility','branch':self.branch}),indent=2)+'\n').encode()
        self.installed={'path':'.github/workflows/diag-native-feasibility.yml','sha256':sha(raw)}
        self.files[self.installed['path']]=raw;self.tip=self.repo.commit(self.files,self.product)
        self.repo.git('update-ref','HEAD',self.tip);self.repo.git('read-tree',self.tip)
        for name,value in self.files.items():
            p=self.repo.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(value)
        self.reviewed=manifest.make(self.repo.root,self.product,self.tip,self.branch,'feasibility',self.installed)
        self.doc={'mode':'feasibility','product_sha':self.product,'diagnostic_sha':self.tip,'branch':self.branch,'probe_config':{'controller_hashes':self.reviewed['controller_hashes']}}
    def tearDown(self):self.repo.close()
    def validate(self,reviewed=None,doc=None):
        raw=json.dumps(self.reviewed if reviewed is None else reviewed).encode()
        return manifest.validate_runtime(raw,sha(raw),self.doc if doc is None else doc,self.repo.root)
    def test_bound_existing_product_and_sources_pass(self):self.assertEqual(self.validate()['product_sha'],self.product)
    def test_wrong_existing_product_is_refused(self):
        other=self.repo.commit({**base_files(),'different.txt':b'existing other product'})
        with self.assertRaises(ValueError):self.validate(doc={**self.doc,'product_sha':other})
    def test_changed_reviewed_product_blob_is_refused(self):
        changed=copy.deepcopy(self.reviewed);key=next(iter(changed['product_blobs']));changed['product_blobs'][key]['oid']='f'*40
        with self.assertRaises(ValueError):self.validate(changed)
    def test_changed_controller_manifest_is_refused(self):
        changed=copy.deepcopy(self.reviewed);changed['controller_hashes']['controller.py']='0'*64
        with self.assertRaises(ValueError):self.validate(changed)
    def test_changed_runtime_controller_bytes_are_refused(self):
        (self.repo.root/'scripts/native_run/controller.py').write_bytes(b'# changed after checkout\n')
        with self.assertRaises((ValueError,subprocess.CalledProcessError)):self.validate()
    def test_missing_installed_candidate_is_refused(self):
        changed=copy.deepcopy(self.reviewed);changed['installed_candidate']=None
        with self.assertRaises(ValueError):self.validate(changed)

if __name__=='__main__':unittest.main()
