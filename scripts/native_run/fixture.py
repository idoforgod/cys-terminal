"""Concrete private product-daemon/ConPTY carrier shared by feasibility and R.

Execution is Windows-only and external-authority gated. No live socket defaults,
role takeover, global process-name kill, counter reset or parser painting.
"""
from pathlib import Path
import json
import os
import shutil
import sys
import time
from boundary import OwnedProcess, Unavailable, cleanup_valid, WindowsAPI, U32, C
from controller import fixture_environment, Authority
from protocol import atomic_json, read_json, sha
from rpc import FixtureRPC

def bounded_observation(description, probe, seconds):
    end=time.monotonic()+seconds
    while True:
        result=probe()
        if result is not None:return result
        if time.monotonic()>=end:raise Unavailable('observation unavailable: '+description)
        time.sleep(.02)

def pinned_copy(source, expected, target):
    source=Path(source)
    if source.is_symlink() or sha(source.read_bytes())!=expected:raise Unavailable('input binary/code hash mismatch')
    target=Path(target);target.parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(source,target)
    if sha(target.read_bytes())!=expected:raise Unavailable('private copy hash mismatch')
    return target

def ps_quote(value):return "'"+str(value).replace("'","''")+"'"

class ProductReceiver:
    def __init__(self, authority, root, spec):
        if not isinstance(authority,Authority):raise Unavailable('external native authority required')
        if sys.platform!='win32':raise Unavailable('real ConPTY fixture requires Windows')
        self.authority=authority;self.root=Path(root);self.spec=spec;self.daemon=None;self.sid=None;self.rpc=None
        self.run_id=self.root.name.removeprefix('cys-native-')
        self.endpoint=r'\\.\pipe\cys-native-'+self.run_id
        self.operator=None

    def start(self):
        root=self.root;spec=self.spec
        for rel in ('daemon/home','daemon/local','daemon/pack','daemon/tmp','child/captures'):(root/rel).mkdir(parents=True,exist_ok=True)
        # The fixture pack belongs to this job. No user/installed pack is copied.
        atomic_json(root/'daemon/pack/acl.json',{'default':'allow','rules':[]})
        atomic_json(root/'daemon/pack/schedule.json',{'jobs':[]})
        daemon_exe=pinned_copy(spec['daemon_test_executable'],spec['daemon_test_sha256'],root/'bin/daemon-test.exe')
        env={'SystemRoot':os.environ['SystemRoot'],'WINDIR':os.environ['SystemRoot'],
             'PATH':os.pathsep.join([str(Path(sys.executable).parent),str(Path(os.environ['SystemRoot'])/'System32'),str(Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0')]),
             'HOME':str(root/'daemon/home'),'USERPROFILE':str(root/'daemon/home'),
             'LOCALAPPDATA':str(root/'daemon/local'),'TMP':str(root/'daemon/tmp'),'TEMP':str(root/'daemon/tmp'),
             'CYS_SOCKET':self.endpoint,'CYS_PACK_DIR':str(root/'daemon/pack'),
             'NATIVE_OWNED_ROOT':str(root),'NATIVE_EXECUTION_AUTHORIZED':'1',
             'PYTHONUTF8':'1','PYTHONDONTWRITEBYTECODE':'1','PYTHONNOUSERSITE':'1',
             'COMSPEC':str(Path(os.environ['SystemRoot'])/'System32/cmd.exe')}
        self.daemon=OwnedProcess(daemon_exe,['--exact','native_harness_trace::native_probe_daemon','--ignored','--nocapture','--test-threads=1'],env,root)
        self.rpc=FixtureRPC(self.endpoint,root,self.run_id,self.daemon.pid)
        def identify():
            try:return self.rpc.call('system.identify',{},timeout=2)
            except Unavailable:return None
        identity=bounded_observation('owned daemon identity',identify,30)
        atomic_json(root/'evidence/daemon-identity.json',identity)
        token=root/'daemon/local/cys'/('cys-native-'+self.run_id)/'operator.token'
        self.operator=token.read_text(encoding='utf-8').strip()
        if not self.operator:raise Unavailable('private fixture operator token absent')
        for name in ('boundary.py','boundary_host.py','controller.py','protocol.py'):
            source=Path(__file__).with_name(name);expected=spec['controller_hashes'][name]
            pinned_copy(source,expected,root/'bin'/name)
        helper=pinned_copy(spec['receiver_helper'],spec['receiver_helper_sha256'],root/'bin/receiver-helper.exe')
        exe=Path(spec['executable'])
        child_env=fixture_environment(root,os.environ['SystemRoot'])
        child_env['EDITOR']='"'+str(helper)+'" editor "'+str(root/'child')+'" "'+str(root/'child/captures')+'"'
        child_env['VISUAL']=child_env['EDITOR'];child_env['PYTHONNOUSERSITE']='1'
        atomic_json(root/'child/config/settings.json',{})
        atomic_json(root/'child/config/.claude.json',{'hasCompletedOnboarding':True,'theme':'dark','lastOnboardingVersion':spec['receiver_version']})
        # No auth material and no permission-bypass setting. These flags are pinned inputs.
        actor=spec.get('actor','receiver')
        if actor not in ('receiver','origin_control'):raise Unavailable('unbound fixture actor')
        args=spec['control_args'] if actor=='origin_control' else ['--bare','--tools','','--session-id',spec['session_id'],'--settings',str(root/'child/config/settings.json')]
        launch={**spec,'mode':self.authority.mode,'run_root':str(root),'environment':child_env,
                'args':args,'process_limit':8}
        atomic_json(root/'evidence/receiver-spec.json',launch)
        authority_path=pinned_copy(spec['authority_path'],self.authority.digest,root/'evidence/authority.json')
        command='& '+ ' '.join(ps_quote(x) for x in [sys.executable,'-B',root/'bin/boundary_host.py',root/'evidence/receiver-spec.json',authority_path,self.authority.digest])+'; exit $LASTEXITCODE'
        created=self.rpc.call('surface.create',{'cmd':command,'cwd':str(root),'rows':24,'cols':80,'title':'owned-native-receiver',
            'env':{'SHELL':str(Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe')}})
        self.sid=created.get('surface_id')
        if type(self.sid) is not int:raise Unavailable('surface creation identity missing')
        if actor=='receiver':self.rpc.call('surface.set_meta',{'surface_id':self.sid,'agent':'claude','agent_bin':str(exe)})
        (root/'evidence/release-receiver').write_bytes(b'owned controller release\n')
        receipt=bounded_observation('confined receiver launch',lambda:read_json(root/'evidence/boundary-start.json') if (root/'evidence/boundary-start.json').is_file() else None,30)
        token_receipt=receipt.get('token',{})
        if type(token_receipt.get('is_appcontainer')) is not int or type(token_receipt.get('capability_count')) is not int or token_receipt['is_appcontainer']!=1 or token_receipt['capability_count']!=0:
            raise Unavailable('receiver confinement receipt invalid')
        self.launch_receipt=receipt
        return receipt

    def write(self, text):
        if type(text) is not str:raise Unavailable('input must be explicit text')
        # This is a synthetic GUI-key path using ONLY this fixture daemon's token.
        # No token is persisted in request logs: the private RPC worker gets a separate ephemeral file.
        return self.rpc.call('surface.send_text',{'surface_id':self.sid,'text':text,'human':True,'operator_token':self.operator})

    def key(self,key):
        return self.rpc.call('surface.send_key',{'surface_id':self.sid,'key':key,'human':True,'operator_token':self.operator})

    def finish_editor(self,receipt_path,receipt):
        pid=receipt.get('pid')
        if type(pid) is not int or pid<=0:raise Unavailable('editor PID absent')
        api=WindowsAPI();handle=api.check(api.k.OpenProcess(0x100000|0x1000,False,pid),'open owned editor')
        try:
            buffer=C.create_unicode_buffer(32768);length=U32(len(buffer))
            api.check(api.k.QueryFullProcessImageNameW(handle,0,buffer,C.byref(length)),'editor image identity')
            expected=self.root/'bin/receiver-helper.exe'
            if Path(buffer.value).resolve()!=expected.resolve():raise Unavailable('editor process is not owned helper')
            token=api.token(handle)
            if token!=self.launch_receipt['token']:raise Unavailable('editor confinement differs from receiver')
            before=self.passive_frame().get('composer',{}).get('output_gen')
            if type(before) is not int:raise Unavailable('pre-editor-return frame unavailable')
            receipt_path.with_suffix('.ack').write_bytes(b'owned observer holds verified editor handle')
            if api.k.WaitForSingleObject(handle,10000)!=0:raise Unavailable('editor exit not observed')
            code=U32();api.check(api.k.GetExitCodeProcess(handle,C.byref(code)),'editor exit code')
            if code.value!=0:raise Unavailable('editor failed')
            atomic_json(self.root/'evidence'/('editor-exit-'+receipt_path.name),{'pid':pid,'image':str(expected),'token':token,'exit_code':code.value,'output_gen_before_ack':before})
            return before
        finally:api.k.CloseHandle(handle)

    def returned_composer(self,data,before):
        # Require a new application output generation after the editor release acknowledgement. This
        # is only a readiness receipt; the byte oracle remains the export.
        text=data.decode('ascii')
        def ready():
            frame=self.passive_frame();gen=frame.get('composer',{}).get('output_gen')
            return frame if type(gen) is int and gen>before and text in frame.get('text','') else None
        return bounded_observation('application return after editor exit',ready,10)

    def export(self):
        before=set((self.root/'child/captures').glob('*.json'))
        self.key('C-g')
        def capture():
            new=set((self.root/'child/captures').glob('*.json'))-before
            if len(new)>1:raise Unavailable('ambiguous editor export')
            return next(iter(new)) if new else None
        receipt_path=bounded_observation('completed real editor export',capture,20)
        receipt=read_json(receipt_path);path=receipt_path.with_suffix('.bin');data=path.read_bytes()
        if type(receipt.get('bytes')) is not int or receipt['bytes']!=len(data) or receipt.get('sha256')!=sha(data):
            raise Unavailable('editor completion/bytes mismatch')
        target=self.root/'evidence'/('composer-'+path.name);target.write_bytes(data)
        atomic_json(target.with_suffix('.json'),receipt)
        before=self.finish_editor(receipt_path,receipt)
        atomic_json(target.with_suffix('.returned.json'),self.returned_composer(data,before))
        return data

    def trace_events(self):
        return [read_json(p) for p in sorted((self.root/'trace').glob('*.json'))]

    def trace_sequence(self):
        return max((e['seq'] for e in self.trace_events()),default=-1)

    def passive_frame(self):
        # Read-only RPC: no key, editor command, CLOSE, CR or input byte.
        return self.rpc.call('surface.read_text',{'surface_id':self.sid,'composer':True})

    def resize_sequence(self):
        return [self.rpc.call('surface.resize',{'surface_id':self.sid,'rows':r,'cols':c}) for r,c in [(32,100),(24,80)]]

    def observations(self):
        return {'surface':self.rpc.call('surface.list',{}),'frame':self.rpc.call('surface.read_text',{'surface_id':self.sid,'composer':True})}

    def close(self):
        if hasattr(self,'cleanup'):return
        receipt=None;errors=[]
        try:
            try:(self.root/'evidence/stop-receiver').write_bytes(b'owned stop after capture\n')
            except Exception as error:errors.append('stop: '+str(error))
            if not errors:
                try:receipt=bounded_observation('receiver owned cleanup',lambda:read_json(self.root/'evidence/boundary-finish.json') if (self.root/'evidence/boundary-finish.json').is_file() else None,15)
                except Exception as error:errors.append('finish: '+str(error))
        finally:
            if self.daemon:
                try:self.daemon.close()
                except Exception as error:errors.append('daemon close: '+str(error))
        daemon_cleanup=getattr(self.daemon,'cleanup',None)
        receiver_cleanup=receipt.get('cleanup') if type(receipt) is dict else None
        self.cleanup={'receiver':receipt,'daemon':daemon_cleanup,'confinement':(receiver_cleanup or {}).get('confinement'),
                      'complete':not errors and cleanup_valid(receiver_cleanup) and cleanup_valid(daemon_cleanup),'errors':errors}
        try:atomic_json(self.root/'evidence/fixture-cleanup.json',self.cleanup)
        except Exception as error:
            # Idempotent later close cannot erase this publication failure.
            self.cleanup['complete']=False;self.cleanup['errors'].append('publication: '+str(error))
            raise
