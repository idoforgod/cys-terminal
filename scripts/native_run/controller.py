"""Shared probe/R controller. Preparation is inert; execution requires a frozen authority.

A missing native premise yields NULL. This module never synthesizes a query,
resets counters, paints a parser, strips CR, or treats a daemon flag as an oracle.
"""
from dataclasses import dataclass
from pathlib import Path
import datetime
import json
import os
import re
import shutil
import sys
import uuid
from protocol import atomic_json, read_json, sha
from boundary import OwnedBoundary, Unavailable

@dataclass(frozen=True)
class Authority:
    digest: str
    mode: str
    diagnostic_sha: str
    product_sha: str
    latest_end_utc: str

def authorize(path, expected_digest, *, mode, diagnostic_sha, product_sha, now=None):
    raw=Path(path).read_bytes()
    if not re.fullmatch('[0-9a-f]{64}',expected_digest) or sha(raw)!=expected_digest:raise Unavailable('external authority digest mismatch')
    data=read_json(path)
    if data.get('mode')!=mode or data.get('diagnostic_sha')!=diagnostic_sha or data.get('product_sha')!=product_sha:
        raise Unavailable('authority mode/source mismatch')
    if data.get('release_verdict')!='NONE' or type(data.get('attempt')) is not int or data['attempt']!=1:
        raise Unavailable('authority attempt/release boundary')
    if data.get('execution_authorized') is not True:raise Unavailable('native execution not authorized')
    if mode not in ('feasibility','R'):raise Unavailable('unknown execution mode')
    now=now or datetime.datetime.now(datetime.timezone.utc)
    start=datetime.datetime.fromisoformat(data['earliest_start_utc']);end=datetime.datetime.fromisoformat(data['latest_end_utc'])
    if start.tzinfo is None or end.tzinfo is None or not start<=now<end:raise Unavailable('outside frozen UTC window')
    if data.get('host')!='windows-latest':raise Unavailable('nonstandard host refused')
    return Authority(expected_digest,mode,diagnostic_sha,product_sha,data['latest_end_utc'])

def prepare_root(parent, executable, expected_sha, *, run_id=None):
    """Copy a pinned binary to a NEW private root, never modify an installed binary."""
    parent=Path(parent).resolve(strict=True);run_id=run_id or uuid.uuid4().hex
    if not re.fullmatch('[0-9a-f]{32}',run_id):raise Unavailable('invalid run identity')
    source=Path(executable)
    if source.is_symlink() or sha(source.read_bytes())!=expected_sha:raise Unavailable('receiver source identity')
    root=parent/('cys-native-'+run_id);root.mkdir()  # existing roots are not adopted
    for rel in ('bin','child','child/home','child/config','child/tmp','child/local','child/appdata','evidence'):(root/rel).mkdir(exist_ok=True)
    atomic_json(root/'owned-run.json',{'kind':'native_fixture','run_id':run_id})
    dest=root/'bin'/source.name;shutil.copy2(source,dest)
    if sha(dest.read_bytes())!=expected_sha:raise Unavailable('receiver copy mismatch')
    return root,dest

def fixture_environment(root, system_root):
    root=Path(root)
    return {'SystemRoot':str(system_root),'WINDIR':str(system_root),'PATH':str(Path(system_root)/'System32'),
            'HOME':str(root/'child/home'),'USERPROFILE':str(root/'child/home'),
            'CLAUDE_CONFIG_DIR':str(root/'child/config'),'TMP':str(root/'child/tmp'),'TEMP':str(root/'child/tmp'),
            'CLAUDE_CODE_TMPDIR':str(root/'child/tmp'),'CLAUDE_TMPDIR':str(root/'child/tmp'),
            'LOCALAPPDATA':str(root/'child/local'),'APPDATA':str(root/'child/appdata'),
            'ANTHROPIC_API_KEY':'native-fixture-not-a-credential','ANTHROPIC_BASE_URL':'http://127.0.0.1:1',
            'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC':'1','TERM':'xterm-256color'}

PREMISES=('network_boundary','auth_free_startup','genuine_open_query','no_submit_calibration')

def decide_feasibility(observations):
    if type(observations) is not dict:raise ValueError('observations must be object')
    failures=[];missing=[]
    for name in PREMISES:
        value=observations.get(name)
        if value is not None and type(value) is not bool:raise ValueError('wrong premise type: '+name)
        if value is False:failures.append(name)
        if value is None:missing.append(name)
    return {'complete':not missing,'observed_failures':failures,'unmeasured':missing,
            'feasibility':None if missing else ('FAIL' if failures else 'ESTABLISHED'),
            'native_hard_gate':'OPEN','release_verdict':'NONE'}

class ReceiverController:
    def __init__(self, authority, spec):
        if not isinstance(authority,Authority):raise Unavailable('frozen authority required')
        self.authority=authority;self.spec=spec;self.child=None
        self.observations=dict.fromkeys(PREMISES)

    def launch(self):
        # Native probe execution is deliberately not exposed by an import or unit check.
        if sys.platform!='win32':raise Unavailable('native Windows receiver unavailable')
        if datetime.datetime.now(datetime.timezone.utc)>=datetime.datetime.fromisoformat(self.authority.latest_end_utc):
            raise Unavailable('authority expired before launch')
        self.child=OwnedBoundary(self.spec)
        return self.child.release()

    def observe(self, name, value):
        if name not in PREMISES or (value is not None and type(value) is not bool):raise ValueError('premise type/name')
        # Caller must use raw-artifact observer checks; missing is never upgraded by a default.
        self.observations[name]=value

    def finish(self):
        if self.child:self.child.close()
        result=decide_feasibility(self.observations)
        result['cleanup']=None if self.child is None else self.child.cleanup
        return result
