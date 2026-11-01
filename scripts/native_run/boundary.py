"""Owned Windows AppContainer launcher; never an unrestricted fallback.

No native call occurs on import. Native compatibility is UNMEASURED until the
separate authorized feasibility run. Only copied executables/private run roots
may receive ACL grants. No firewall, Defender or loopback-exemption changes.
"""
import ctypes as C
import os
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path
from protocol import sha

class Unavailable(RuntimeError):
    pass

U32 = C.c_uint32
U64 = C.c_uint64
P = C.c_void_p
SIZE = C.c_size_t
class STARTUPINFO(C.Structure):
    _fields_ = [('cb', U32), ('lpReserved', P), ('lpDesktop', P), ('lpTitle', P),
                ('dwX', U32), ('dwY', U32), ('dwXSize', U32), ('dwYSize', U32),
                ('dwXCountChars', U32), ('dwYCountChars', U32), ('dwFillAttribute', U32),
                ('dwFlags', U32), ('wShowWindow', C.c_uint16), ('cbReserved2', C.c_uint16),
                ('lpReserved2', P), ('hStdInput', P), ('hStdOutput', P), ('hStdError', P)]
class STARTUPINFOEX(C.Structure):
    _fields_ = [('StartupInfo', STARTUPINFO), ('lpAttributeList', P)]
class PROCESS_INFORMATION(C.Structure):
    _fields_ = [('hProcess', P), ('hThread', P), ('dwProcessId', U32), ('dwThreadId', U32)]
class SECURITY_CAPABILITIES(C.Structure):
    _fields_ = [('AppContainerSid', P), ('Capabilities', P), ('CapabilityCount', U32), ('Reserved', U32)]
class BASIC_LIMITS(C.Structure):
    _fields_ = [('PerProcessUserTimeLimit', C.c_int64), ('PerJobUserTimeLimit', C.c_int64),
                ('LimitFlags', U32), ('MinimumWorkingSetSize', SIZE), ('MaximumWorkingSetSize', SIZE),
                ('ActiveProcessLimit', U32), ('Affinity', SIZE), ('PriorityClass', U32), ('SchedulingClass', U32)]
class IO_COUNTERS(C.Structure):
    _fields_ = [(x, U64) for x in ('ReadOperationCount', 'WriteOperationCount', 'OtherOperationCount',
                                  'ReadTransferCount', 'WriteTransferCount', 'OtherTransferCount')]
class ACCOUNTING(C.Structure):
    _fields_ = [(x,C.c_int64) for x in ("user","kernel","period_user","period_kernel")]+[(x,U32) for x in ("faults","total","active","terminated")]
class EXTENDED_LIMITS(C.Structure):
    _fields_ = [('BasicLimitInformation', BASIC_LIMITS), ('IoInfo', IO_COUNTERS),
                ('ProcessMemoryLimit', SIZE), ('JobMemoryLimit', SIZE),
                ('PeakProcessMemoryUsed', SIZE), ('PeakJobMemoryUsed', SIZE)]

SECURITY_CAPABILITIES_ATTRIBUTE = 0x20009
CREATE_SUSPENDED = 0x4
EXTENDED_STARTUPINFO_PRESENT = 0x80000
CREATE_UNICODE_ENVIRONMENT = 0x400
JOB_KILL_ON_CLOSE = 0x2000
JOB_ACTIVE_LIMIT = 0x8

def owned_path(root, path):
    root = Path(root).resolve(strict=True)
    path = Path(path)
    if not path.is_absolute(): path = root/path
    if '..' in path.parts or (path != root and root not in path.parents):
        raise Unavailable('lexical path outside owned run root')
    # Reject symlinks before dereferencing even when their target happens to fit.
    for part in (path, *path.parents):
        if part == root.parent: break
        if part.is_symlink(): raise Unavailable('symlink in owned path')
    resolved = path.resolve(strict=True)
    if resolved != root and root not in resolved.parents:
        raise Unavailable('path outside owned run root')
    return resolved

def validate_launch(spec):
    if type(spec) is not dict: raise Unavailable('launch spec must be an object')
    root = Path(spec['run_root']).resolve(strict=True)
    marker = root / 'owned-run.json'
    if not re.fullmatch(r'cys-native-[0-9a-f]{32}', root.name) or not marker.is_file(): raise Unavailable('owned-run marker/name missing')
    exe = owned_path(root, spec['executable'])
    if root/'bin' not in exe.parents: raise Unavailable('executable must be in immutable private bin')
    if not exe.is_file() or sha(exe.read_bytes()) != spec['executable_sha256']:
        raise Unavailable('private executable identity mismatch')
    args = spec['args']
    if type(args) is not list or not all(type(x) is str and '\0' not in x for x in args):
        raise Unavailable('invalid argv')
    env = spec['environment']
    if type(env) is not dict or not all(type(k) is str and type(v) is str and '\0' not in k+v and '=' not in k for k,v in env.items()):
        raise Unavailable('invalid environment')
    for name in ('HOME', 'USERPROFILE', 'CLAUDE_CONFIG_DIR', 'TMP', 'TEMP', 'LOCALAPPDATA', 'APPDATA', 'CLAUDE_CODE_TMPDIR', 'CLAUDE_TMPDIR'):
        resolved=owned_path(root, env[name])
        if resolved != root/'child' and root/'child' not in resolved.parents: raise Unavailable('mutable environment directory outside child subtree')
    if any(k.startswith('AWS_') or k.endswith('TOKEN') or k.startswith('AZURE_') for k in env):
        raise Unavailable('credential-bearing environment key')
    if env.get('ANTHROPIC_API_KEY') not in (None, 'native-fixture-not-a-credential'):
        raise Unavailable('real credential forbidden')
    if type(spec['process_limit']) is not int or not 1 <= spec['process_limit'] <= 8:
        raise Unavailable('invalid owned process limit')
    return root, exe, args, env

def protected_sddl(owner,sid,category,directory):
    if not re.fullmatch(r'S-[0-9-]+',owner) or not re.fullmatch(r'S-1-15-2-[0-9-]+',sid):raise Unavailable('SID grammar')
    if category not in ('root','bin','child','evidence'):raise Unavailable('unknown ACL role')
    flags='OICI' if directory else ''
    value=f'D:P(A;{flags};FA;;;SY)(A;{flags};FA;;;{owner})'
    if category=='root':value+=f'(A;;RX;;;{sid})'
    elif category=='bin':value+=f'(A;{flags};RX;;;{sid})'
    elif category=='child':value+=f'(A;{flags};0x1301bf;;;{sid})'
    return value

class WindowsAPI:
    def __init__(self):
        if sys.platform != 'win32': raise Unavailable('Windows native boundary unavailable on this host')
        self.k = C.WinDLL('kernel32.dll', use_last_error=True, winmode=0x800)
        self.a = C.WinDLL('advapi32.dll', use_last_error=True, winmode=0x800)
        self.u = C.WinDLL('userenv.dll', use_last_error=True, winmode=0x800)
        signatures = [
            (self.u, 'CreateAppContainerProfile', [C.c_wchar_p,C.c_wchar_p,C.c_wchar_p,P,U32,C.POINTER(P)], C.c_int32),
            (self.u, 'DeleteAppContainerProfile', [C.c_wchar_p], C.c_int32),
            (self.a, 'FreeSid', [P], P),
            (self.a, 'ConvertSidToStringSidW', [P,C.POINTER(P)], C.c_int),
            (self.k, 'LocalFree', [P], P),
            (self.k, 'GetCurrentProcess', [], P),
            (self.k, 'OpenProcess', [U32,C.c_int,U32], P),
            (self.k, 'QueryFullProcessImageNameW', [P,U32,C.c_wchar_p,C.POINTER(U32)], C.c_int),
            (self.k, 'GetSystemDirectoryW', [C.c_wchar_p,U32], U32),
            (self.a, 'ConvertStringSecurityDescriptorToSecurityDescriptorW', [C.c_wchar_p,U32,C.POINTER(P),P], C.c_int),
            (self.a, 'ConvertSecurityDescriptorToStringSecurityDescriptorW', [P,U32,U32,C.POINTER(P),P], C.c_int),
            (self.a, 'GetSecurityDescriptorDacl', [P,C.POINTER(C.c_int),C.POINTER(P),C.POINTER(C.c_int)], C.c_int),
            (self.a, 'SetNamedSecurityInfoW', [C.c_wchar_p,C.c_int,U32,P,P,P,P], U32),
            (self.a, 'GetNamedSecurityInfoW', [C.c_wchar_p,C.c_int,U32,P,P,P,P,C.POINTER(P)], U32),
            (self.k, 'CreateJobObjectW', [P,C.c_wchar_p], P),
            (self.k, 'SetInformationJobObject', [P,C.c_int,P,U32], C.c_int),
            (self.k, 'QueryInformationJobObject', [P,C.c_int,P,U32,P], C.c_int),
            (self.k, 'TerminateJobObject', [P,U32], C.c_int),
            (self.k, 'AssignProcessToJobObject', [P,P], C.c_int),
            (self.k, 'InitializeProcThreadAttributeList', [P,U32,U32,C.POINTER(SIZE)], C.c_int),
            (self.k, 'UpdateProcThreadAttribute', [P,U32,SIZE,P,SIZE,P,P], C.c_int),
            (self.k, 'DeleteProcThreadAttributeList', [P], None),
            (self.k, 'CreateProcessW', [C.c_wchar_p,C.c_wchar_p,P,P,C.c_int,U32,P,C.c_wchar_p,P,P], C.c_int),
            (self.k, 'ResumeThread', [P], U32),
            (self.k, 'TerminateProcess', [P,U32], C.c_int),
            (self.k, 'WaitForSingleObject', [P,U32], U32),
            (self.k, 'GetExitCodeProcess', [P,C.POINTER(U32)], C.c_int),
            (self.k, 'GetProcessTimes', [P,C.POINTER(U64),C.POINTER(U64),C.POINTER(U64),C.POINTER(U64)], C.c_int),
            (self.k, 'CloseHandle', [P], C.c_int),
            (self.a, 'OpenProcessToken', [P,U32,C.POINTER(P)], C.c_int),
            (self.a, 'GetTokenInformation', [P,C.c_int,P,U32,C.POINTER(U32)], C.c_int),
        ]
        for dll, name, args, result in signatures:
            fn = getattr(dll, name); fn.argtypes = args; fn.restype = result

    def check(self, value, action):
        if not value: raise Unavailable(f'{action}: Win32 error {C.get_last_error()}')
        return value

    def sid_string(self, sid):
        result = P(); self.check(self.a.ConvertSidToStringSidW(sid,C.byref(result)), 'SID text')
        try: return C.wstring_at(result)
        finally: self.k.LocalFree(result)

    def user_sid(self):
        token=P();self.check(self.a.OpenProcessToken(self.k.GetCurrentProcess(),0x8,C.byref(token)),'current token')
        try:
            size=U32();self.a.GetTokenInformation(token,1,None,0,C.byref(size));buf=C.create_string_buffer(size.value)
            self.check(self.a.GetTokenInformation(token,1,buf,size,C.byref(size)),'current user SID')
            return self.sid_string(P.from_buffer(buf).value)
        finally:self.k.CloseHandle(token)

    def sddl(self,descriptor):
        text=P();self.check(self.a.ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor,1,4,C.byref(text),None),'DACL serialization')
        try:return C.wstring_at(text)
        finally:self.k.LocalFree(text)

    def protect(self,path,descriptor_text):
        descriptor=P();self.check(self.a.ConvertStringSecurityDescriptorToSecurityDescriptorW(descriptor_text,1,C.byref(descriptor),None),'DACL construction')
        try:
            expected=self.sddl(descriptor);present=C.c_int();defaulted=C.c_int();acl=P()
            self.check(self.a.GetSecurityDescriptorDacl(descriptor,C.byref(present),C.byref(acl),C.byref(defaulted)),'DACL extract')
            if not present.value or not acl.value:raise Unavailable('null DACL forbidden')
            code=self.a.SetNamedSecurityInfoW(str(path),1,0x80000004,None,None,acl,None)
            if code:raise Unavailable('private protected DACL failed: '+str(code))
            actual=P();code=self.a.GetNamedSecurityInfoW(str(path),1,4,None,None,None,None,C.byref(actual))
            if code:raise Unavailable('private DACL query failed: '+str(code))
            try:observed=self.sddl(actual)
            finally:self.k.LocalFree(actual)
            if observed!=expected:raise Unavailable('private DACL did not round-trip exactly')
            return {'path':str(path),'sddl':observed,'verified':True}
        finally:self.k.LocalFree(descriptor)

    def token(self, process):
        handle = P(); self.check(self.a.OpenProcessToken(process,0x8,C.byref(handle)), 'open child token')
        def info(kind):
            size=U32(); self.a.GetTokenInformation(handle,kind,None,0,C.byref(size))
            if not 0 < size.value <= 1024*1024: raise Unavailable('token evidence size unavailable')
            buf=C.create_string_buffer(size.value)
            self.check(self.a.GetTokenInformation(handle,kind,buf,size,C.byref(size)), 'read token')
            return buf
        try:
            is_app = U32.from_buffer(info(29)).value
            cap_count = U32.from_buffer(info(30)).value
            package = info(31); sid=P.from_buffer(package).value
            return {'is_appcontainer':is_app, 'capability_count':cap_count, 'package_sid':self.sid_string(sid)}
        finally: self.k.CloseHandle(handle)

class OwnedBoundary:
    """One owned suspended child: inspect token and assign job BEFORE release."""
    def __init__(self, spec):
        root, exe, args, env = validate_launch(spec)
        api = self.api = WindowsAPI()
        self.profile = 'cysnative.' + uuid.uuid4().hex
        self.sid=P(); self.job=None; self.process=PROCESS_INFORMATION(); self.resumed=False
        self.profile_created=False; self.closed=False; self.assigned=False
        hr=api.u.CreateAppContainerProfile(self.profile,self.profile,'owned native feasibility',None,0,C.byref(self.sid))
        if hr != 0: raise Unavailable(f'new private profile creation refused: HRESULT {hr & 0xffffffff:08x}')
        self.profile_created=True;self.lineage=[]
        try:
            self.sid_text=api.sid_string(self.sid)
            # Only this private root. Do not grant access to an installed CLI or user directory.
            owner=api.user_sid();receipts=[]
            # Explicit protected DACLs remove inherited grants; never recursively follow links.
            paths=[root,root/'bin',root/'child',root/'evidence']
            paths += sorted((root/'bin').rglob('*'),key=lambda p:len(p.parts))
            paths += sorted((root/'child').rglob('*'),key=lambda p:len(p.parts))
            paths += [p for p in (root/'daemon',root/'trace') if p.is_dir()]
            for path in paths:
                if path.is_symlink() or (hasattr(path,'is_junction') and path.is_junction()):raise Unavailable('reparse point before ACL setup')
                relative=path.relative_to(root)
                category='child' if relative.parts and relative.parts[0]=='child' else ('bin' if relative.parts and relative.parts[0]=='bin' else ('root' if path==root else 'evidence'))
                descriptor=protected_sddl(owner,self.sid_text,category,path.is_dir())
                receipts.append(api.protect(path,descriptor))
            buf=C.create_unicode_buffer(32768);n=api.k.GetSystemDirectoryW(buf,len(buf))
            if not 0<n<len(buf):raise Unavailable('trusted system directory unavailable')
            icacls=Path(buf.value)/'icacls.exe'
            low=subprocess.run([str(icacls),str(root/'child'),'/setintegritylevel','(OI)(CI)L','/T','/Q'],capture_output=True)
            if low.returncode:raise Unavailable('private child integrity label failed')
            self.acl_receipt={'protected_dacls':receipts,'low_label_rc':low.returncode,'icacls':str(icacls),'protected_evidence_root':str(root/'evidence')}
            self.job=api.check(api.k.CreateJobObjectW(None,None),'create owned job')
            limits=EXTENDED_LIMITS(); limits.BasicLimitInformation.LimitFlags=JOB_KILL_ON_CLOSE|JOB_ACTIVE_LIMIT
            limits.BasicLimitInformation.ActiveProcessLimit=spec['process_limit']
            api.check(api.k.SetInformationJobObject(self.job,9,C.byref(limits),C.sizeof(limits)), 'configure owned job')
            size=SIZE(); api.k.InitializeProcThreadAttributeList(None,1,0,C.byref(size))
            attrs=C.create_string_buffer(size.value)
            api.check(api.k.InitializeProcThreadAttributeList(attrs,1,0,C.byref(size)), 'init attributes')
            try:
                caps=SECURITY_CAPABILITIES(self.sid,None,0,0)
                api.check(api.k.UpdateProcThreadAttribute(attrs,0,SECURITY_CAPABILITIES_ATTRIBUTE,C.byref(caps),C.sizeof(caps),None,None), 'set zero capabilities')
                startup=STARTUPINFOEX(); startup.StartupInfo.cb=C.sizeof(startup); startup.lpAttributeList=C.cast(attrs,P)
                command=C.create_unicode_buffer(subprocess.list2cmdline([str(exe),*args]))
                block=C.create_unicode_buffer('\0'.join(k+'='+v for k,v in sorted(env.items(),key=lambda kv:kv[0].upper()))+'\0\0')
                flags=CREATE_SUSPENDED|CREATE_UNICODE_ENVIRONMENT|EXTENDED_STARTUPINFO_PRESENT
                api.check(api.k.CreateProcessW(str(exe),command,None,None,False,flags,block,str(root),C.byref(startup),C.byref(self.process)), 'create confined suspended child')
            finally: api.k.DeleteProcThreadAttributeList(attrs)
            api.check(api.k.AssignProcessToJobObject(self.job,self.process.hProcess), 'assign owned job')
            self.assigned=True
            self.token=api.token(self.process.hProcess)
            if self.token != {'is_appcontainer':1,'capability_count':0,'package_sid':self.sid_text}:
                raise Unavailable('child token is not the requested zero-capability AppContainer')
        except BaseException as error:
            self.close()
            error.partial_cleanup=self.cleanup
            error.owned_profile=self.profile
            raise

    def release(self):
        if self.closed or self.resumed: raise Unavailable('invalid child release state')
        if self.api.k.ResumeThread(self.process.hThread)==0xffffffff: raise Unavailable('ResumeThread failed')
        self.resumed=True
        self.sample_lineage()
        return {'pid':self.process.dwProcessId,'profile':self.profile,'token':self.token,
                'acl':self.acl_receipt,'capabilities_requested':[],'job_kill_on_close':True}

    def sample_lineage(self):
        # Query THIS owned job, never a host process census. Race/exit failures
        # remain explicit; a count gap prevents an exhaustive lineage claim.
        row={'monotonic_ns':time.monotonic_ns(),'processes':[],'errors':[]}
        buffer=C.create_string_buffer(8+8*128)
        if not self.api.k.QueryInformationJobObject(self.job,3,buffer,C.sizeof(buffer),None):
            row['errors'].append('owned_job_pid_list_unavailable')
        else:
            assigned=U32.from_buffer(buffer,0).value;listed=U32.from_buffer(buffer,4).value
            row.update(assigned=assigned,listed=listed)
            if assigned!=listed or listed>128:row['errors'].append('owned_job_pid_list_incomplete')
            else:
                for i in range(listed):
                    pid=SIZE.from_buffer(buffer,8+i*C.sizeof(SIZE)).value;handle=None
                    try:
                        handle=self.api.check(self.api.k.OpenProcess(0x1000,False,pid),'open owned job member')
                        created=U64();exited=U64();kernel=U64();user=U64()
                        self.api.check(self.api.k.GetProcessTimes(handle,C.byref(created),C.byref(exited),C.byref(kernel),C.byref(user)),'job member creation identity')
                        row['processes'].append({'pid':pid,'creation_filetime':created.value,'token':self.api.token(handle)})
                    except Unavailable as error:row['errors'].append({'pid':pid,'error':str(error)})
                    finally:
                        if handle:self.api.k.CloseHandle(handle)
        self.lineage.append(row)
        return row

    def wait(self, timeout_ms):
        if type(timeout_ms) is not int or not 0 <= timeout_ms <= 120000: raise Unavailable('invalid wait bound')
        status=self.api.k.WaitForSingleObject(self.process.hProcess,timeout_ms)
        if status==258: return None
        if status!=0: raise Unavailable('child wait failed')
        code=U32(); self.api.check(self.api.k.GetExitCodeProcess(self.process.hProcess,C.byref(code)), 'child exit')
        return code.value

    def close(self):
        if self.closed: return
        self.closed=True;errors=[];accounting=None
        # Terminate only this unnamed owned job, retaining its handle until the
        # kernel reports zero active members. No PID/name kill and no root-only inference.
        if self.process.hProcess and not self.assigned:
            if not self.api.k.TerminateProcess(self.process.hProcess,125):errors.append('suspended_child_terminate_failed')
        if self.job:
            if not self.api.k.TerminateJobObject(self.job,125):errors.append('owned_job_terminate_failed')
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                row=ACCOUNTING()
                if not self.api.k.QueryInformationJobObject(self.job,1,C.byref(row),C.sizeof(row),None):
                    errors.append('job_accounting_unavailable');break
                accounting={'total':row.total,'active':row.active,'terminated':row.terminated}
                if row.active==0:break
                time.sleep(.01)
            if not self.api.k.CloseHandle(self.job):errors.append('job_handle_close_failed')
            self.job=None
        exited=not self.process.hProcess or self.api.k.WaitForSingleObject(self.process.hProcess,10000)==0
        if self.process.hThread and not self.api.k.CloseHandle(self.process.hThread):errors.append('thread_handle_close_failed')
        if self.process.hProcess and not self.api.k.CloseHandle(self.process.hProcess):errors.append('process_handle_close_failed')
        if self.sid:self.api.a.FreeSid(self.sid)
        removed=False
        if self.profile_created and exited and (not self.process.hProcess or (accounting is not None and accounting['active']==0)) and not errors:
            removed=self.api.u.DeleteAppContainerProfile(self.profile)==0
            if not removed:errors.append('owned_profile_removal_failed')
        self.cleanup={'root_exit_observed':exited,'job_accounting':accounting,'errors':errors,
                      'descendant_exit_receipt':True if not self.process.hProcess else (None if accounting is None else accounting['active']==0),
                      'profile_removed':removed,'profile_retained':self.profile_created and not removed}
        if hasattr(self,'lineage'):
            self.cleanup['lineage']=self.lineage
            self.cleanup['confinement']=lineage_verdict(self.lineage,accounting,getattr(self,'token',None))

    def __enter__(self): return self
    def __exit__(self,*unused): self.close()

class OwnedProcess:
    """Trusted fixture daemon/controller lifetime, separate from the confined receiver.

The process starts suspended and is assigned to an unnamed kill-on-close job
before any instruction runs. This does not grant the receiver network access.
"""
    def __init__(self, executable, args, environment, cwd, process_limit=32):
        self.api=WindowsAPI();self.job=None;self.process=PROCESS_INFORMATION();self.assigned=False
        self.closed=False;self.sid=None;self.profile_created=False
        try:
            self.job=self.api.check(self.api.k.CreateJobObjectW(None,None),'fixture job')
            limits=EXTENDED_LIMITS();limits.BasicLimitInformation.LimitFlags=JOB_KILL_ON_CLOSE|JOB_ACTIVE_LIMIT
            limits.BasicLimitInformation.ActiveProcessLimit=process_limit
            self.api.check(self.api.k.SetInformationJobObject(self.job,9,C.byref(limits),C.sizeof(limits)),'fixture job limits')
            startup=STARTUPINFO();startup.cb=C.sizeof(startup)
            command=C.create_unicode_buffer(subprocess.list2cmdline([str(executable),*args]))
            block=C.create_unicode_buffer('\0'.join(k+'='+v for k,v in sorted(environment.items(),key=lambda kv:kv[0].upper()))+'\0\0')
            self.api.check(self.api.k.CreateProcessW(str(executable),command,None,None,False,CREATE_SUSPENDED|CREATE_UNICODE_ENVIRONMENT,block,str(cwd),C.byref(startup),C.byref(self.process)),'create fixture daemon')
            self.api.check(self.api.k.AssignProcessToJobObject(self.job,self.process.hProcess),'assign fixture daemon');self.assigned=True
            if self.api.k.ResumeThread(self.process.hThread)==0xffffffff:raise Unavailable('fixture resume failed')
        except BaseException:
            self.close();raise
    @property
    def pid(self):return self.process.dwProcessId
    wait=OwnedBoundary.wait
    close=OwnedBoundary.close
    def __enter__(self):return self
    def __exit__(self,*unused):self.close()

def cleanup_valid(receipt):
    return (type(receipt) is dict and receipt.get('root_exit_observed') is True
            and receipt.get('descendant_exit_receipt') is True
            and receipt.get('profile_retained') is False
            and type(receipt.get('errors')) is list and not receipt['errors'])

def lineage_verdict(snapshots,accounting,expected):
    if type(snapshots) is not list or not snapshots or type(expected) is not dict:return None
    seen=set();missing=False
    for snapshot in snapshots:
        if snapshot.get('errors'):missing=True
        for process in snapshot.get('processes',[]):
            if process.get('token')!=expected:return False
            identity=(process.get('pid'),process.get('creation_filetime'))
            if any(type(x) is not int or x<=0 for x in identity):missing=True
            else:seen.add(identity)
    if missing or type(accounting) is not dict or accounting.get('total')!=len(seen):return None
    return True
