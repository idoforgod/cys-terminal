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

class WindowsAPI:
    def __init__(self):
        if sys.platform != 'win32': raise Unavailable('Windows native boundary unavailable on this host')
        self.k = C.WinDLL('kernel32', use_last_error=True)
        self.a = C.WinDLL('advapi32', use_last_error=True)
        self.u = C.WinDLL('userenv', use_last_error=True)
        signatures = [
            (self.u, 'CreateAppContainerProfile', [C.c_wchar_p,C.c_wchar_p,C.c_wchar_p,P,U32,C.POINTER(P)], C.c_int32),
            (self.u, 'DeleteAppContainerProfile', [C.c_wchar_p], C.c_int32),
            (self.a, 'FreeSid', [P], P),
            (self.a, 'ConvertSidToStringSidW', [P,C.POINTER(P)], C.c_int),
            (self.k, 'LocalFree', [P], P),
            (self.k, 'CreateJobObjectW', [P,C.c_wchar_p], P),
            (self.k, 'SetInformationJobObject', [P,C.c_int,P,U32], C.c_int),
            (self.k, 'AssignProcessToJobObject', [P,P], C.c_int),
            (self.k, 'InitializeProcThreadAttributeList', [P,U32,U32,C.POINTER(SIZE)], C.c_int),
            (self.k, 'UpdateProcThreadAttribute', [P,U32,SIZE,P,SIZE,P,P], C.c_int),
            (self.k, 'DeleteProcThreadAttributeList', [P], None),
            (self.k, 'CreateProcessW', [C.c_wchar_p,C.c_wchar_p,P,P,C.c_int,U32,P,C.c_wchar_p,P,P], C.c_int),
            (self.k, 'ResumeThread', [P], U32),
            (self.k, 'TerminateProcess', [P,U32], C.c_int),
            (self.k, 'WaitForSingleObject', [P,U32], U32),
            (self.k, 'GetExitCodeProcess', [P,C.POINTER(U32)], C.c_int),
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
        self.profile_created=True
        try:
            self.sid_text=api.sid_string(self.sid)
            # Only this private root. Do not grant access to an installed CLI or user directory.
            receipts=[]
            for target,grant in [(root,f'*{self.sid_text}:RX'),(root/'bin',f'*{self.sid_text}:(OI)(CI)RX'),(root/'child',f'*{self.sid_text}:(OI)(CI)M')]:
                argv=['icacls.exe',str(target),'/grant',grant,'/Q']
                if target!=root:argv.append('/T')
                result=subprocess.run(argv,capture_output=True)
                if result.returncode:raise Unavailable('private ACL grant failed')
                receipts.append({'root':str(target),'grant':grant,'rc':result.returncode,'stdout_sha256':sha(result.stdout),'stderr_sha256':sha(result.stderr)})
            low=subprocess.run(['icacls.exe',str(root/'child'),'/setintegritylevel','(OI)(CI)L','/T','/Q'],capture_output=True)
            if low.returncode:raise Unavailable('private child integrity label failed')
            self.acl_receipt={'grants':receipts,'low_label_rc':low.returncode,'protected_evidence_root':str(root/'evidence')}
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
        except BaseException:
            self.close()
            raise

    def release(self):
        if self.closed or self.resumed: raise Unavailable('invalid child release state')
        if self.api.k.ResumeThread(self.process.hThread)==0xffffffff: raise Unavailable('ResumeThread failed')
        self.resumed=True
        return {'pid':self.process.dwProcessId,'profile':self.profile,'token':self.token,
                'acl':self.acl_receipt,'capabilities_requested':[],'job_kill_on_close':True}

    def wait(self, timeout_ms):
        if type(timeout_ms) is not int or not 0 <= timeout_ms <= 120000: raise Unavailable('invalid wait bound')
        status=self.api.k.WaitForSingleObject(self.process.hProcess,timeout_ms)
        if status==258: return None
        if status!=0: raise Unavailable('child wait failed')
        code=U32(); self.api.check(self.api.k.GetExitCodeProcess(self.process.hProcess,C.byref(code)), 'child exit')
        return code.value

    def close(self):
        if self.closed: return
        self.closed=True
        # Only handles returned by this instance; never PID/name-based termination.
        if self.process.hProcess and not self.assigned:
            # Failed assignment leaves only our still-suspended child; terminate by its owned handle.
            self.api.k.TerminateProcess(self.process.hProcess,125)
        if self.job: self.api.k.CloseHandle(self.job); self.job=None
        exited = not self.process.hProcess or self.api.k.WaitForSingleObject(self.process.hProcess,10000)==0
        if self.process.hThread: self.api.k.CloseHandle(self.process.hThread)
        if self.process.hProcess: self.api.k.CloseHandle(self.process.hProcess)
        if self.sid: self.api.a.FreeSid(self.sid)
        # Retain profile on uncertain cleanup; never delete an existing/reused profile.
        self.cleanup={'root_exit_observed':exited,'profile_removed':False}
        # Preserve the unique profile/files for audit. No recursive deletion. Native job-event
        # receipts must establish descendant completion before a controller can claim cleanup.
        self.cleanup['profile_retained']=self.profile_created
        self.cleanup['descendant_exit_receipt']=None

    def __enter__(self): return self
    def __exit__(self,*unused): self.close()
