#!/usr/bin/env python3
"""S5 Darwin C6/roster/create/reap integration: exact binaries, one isolated daemon.
Agent spawn/readiness are inert fixtures; C6, roster, create and queue lifecycle are real.
No old harness lifecycle, installed binary fallback, AI launch or live endpoint.
"""
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
from unittest import mock

REPO = Path(__file__).resolve().parents[3]
CYS, CYSD = [REPO / ('target/debug/' + n) for n in ('cys', 'cysd')]
GRACE = 20


# Reuse the S7-tested kernel sec+usec identity reader; no harness lifecycle calls.
_hspec = importlib.util.spec_from_file_location('s5_process_identity', REPO/'cysjavis-pack/bin/javis_phoenix_harness.py')
H = importlib.util.module_from_spec(_hspec); _hspec.loader.exec_module(H)


def processes():
    r = subprocess.run(['/bin/ps', '-axo', 'pid=,ppid='], capture_output=True, text=True, check=True, timeout=3)
    return {int(p[0]): int(p[1]) for line in r.stdout.splitlines() if len(p := line.split()) == 2}


def same_process(pid, identity):
    info = H._process_info(pid)
    return info is not None and info[0] == identity


def main():
    if sys.platform != 'darwin':
        print('SKIP: Darwin-only S5 fixture; Windows native remains unrun')
        return 0
    assert CYS.is_file() and CYSD.is_file(), 'S5 requires exact built worktree binaries'
    base = REPO / 'target/u-tmp'
    base.mkdir(parents=True, exist_ok=True)  # a fresh CI runner has no target/u-tmp (release run 39747056302)
    base = base.resolve()
    with tempfile.TemporaryDirectory(prefix='s5n-', dir=base) as temp:
        root = Path(temp).resolve()
        endpoint = root / 'd.sock'
        assert root.parent == base and len(os.fsencode(endpoint)) < 100
        for name in ('home', 'pack', 'ledger', 'tmp'):
            (root / name).mkdir()
        env = {'HOME':str(root/'home'), 'TMPDIR':str(root/'tmp'), 'PATH':'/usr/bin:/bin:/usr/sbin:/sbin',
               'CYS_SOCKET':str(endpoint), 'CYS_STATE_DIR':str(root/'ledger'), 'CYS_PACK_DIR':str(root/'pack'),
               'CYS_SHELL':'/bin/sh', 'CYS_NO_AUTORESTORE':'1', 'CYS_REAP_EXITED':'1',
               'CYS_REAP_EXITED_GRACE_SECS':str(GRACE), 'CYS_REAP_EXITED_NONROLE_GRACE_SECS':str(GRACE),
               'PHOENIX_CYS':str(CYS), 'PHOENIX_SPAWN_RETRIES':'0', 'PYTHONDONTWRITEBYTECODE':'1'}
        owned, cleanup_errors = {}, []
        daemon, ready = None, False
        def capture(roots=()):
            parents = processes()
            todo = list(roots) + [pid for pid, identity in owned.items() if same_process(pid, identity)]
            seen = set()
            while todo:
                pid = todo.pop()
                if pid in seen: continue
                seen.add(pid)
                info = H._process_info(pid)
                if info is None:
                    assert pid in owned, 'new owned root vanished before identity capture'
                    continue
                if pid in owned and owned[pid] != info[0]: continue
                owned[pid] = info[0]
                for child, parent in parents.items():
                    if parent != pid or child in seen: continue
                    now, child_info = H._process_info(pid), H._process_info(child)
                    assert now == info, 'owned parent incarnation changed during capture'
                    if child_info is None: continue
                    assert child_info[1] == pid, 'descendant parent changed during capture'
                    if child in owned: assert owned[child] == child_info[0], 'owned descendant reused'
                    owned[child] = child_info[0]
                    todo.append(child)
        def rpc(method, params=None):
            assert endpoint.parent == root
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                s.settimeout(6)
                s.connect(str(endpoint))
                s.sendall((json.dumps({'id':'s5-native','method':method,'params':params or {}})+'\n').encode())
                with s.makefile('rb') as stream:
                    reply = json.loads(stream.readline(2_000_000))
            assert reply.get('ok'), 'isolated RPC failed: %s %s' % (method, reply.get('error'))
            return reply['result']
        def until(fn, seconds=8):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                try:
                    if value := fn(): return value
                except (OSError, ValueError): pass
                time.sleep(0.05)
            raise AssertionError('S5 bounded observation timed out')
        def surfaces(): return rpc('surface.list')['surfaces']
        def record_surface(sid):
            capture([next(s for s in surfaces() if s['surface_id'] == sid)['pid']])
        def create(role, command):
            sid = rpc('surface.create', {'role':role,'cmd':command,'cwd':str(root)})['surface_id']
            record_surface(sid)
            return sid
        def cli(*args):
            return subprocess.run([str(CYS),'--socket',str(endpoint),*args], env=env, cwd=root,
                                  capture_output=True, text=True, timeout=12)
        manifest = {'result':'failed','body_deadline_secs':90,'cleanup_deadline_secs':45,'owned_residuals':'unmeasured','grace_secs':GRACE,'endpoint':str(endpoint),'state_root':str(root),
                    'binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (CYS,CYSD)}}
        def save_evidence():
            manifest['owned_original_identities'] = [{'pid':pid,'start_identity':list(identity)} for pid,identity in sorted(owned.items())]
            artifact = os.environ.get('S5_NATIVE_EVIDENCE')
            if artifact:
                out = Path(artifact); out.mkdir(parents=True,exist_ok=True)
                body = (root/'daemon.log').read_text(errors='replace') if (root/'daemon.log').exists() else ''
                if (root/'operator.token').exists():
                    token = (root/'operator.token').read_text().strip()
                    if token: body = body.replace(token, '[REDACTED]')
                (out/'daemon.log').write_text(body)
                (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        def timeout(*args): raise TimeoutError('S5 body or cleanup deadline')
        prior = signal.signal(signal.SIGALRM, timeout)
        signal.alarm(90)
        try:
            with open(root/'daemon.log','wb') as log:
                daemon = subprocess.Popen([str(CYSD)], env=env, cwd=root, stdout=log, stderr=log, start_new_session=True)
            capture([daemon.pid])
            until(lambda: rpc('system.ping'), 30)
            ready = True
            rpc('system.pause', {'reason':'isolated S5 fixture; preserve queue without injection'})
            live = create(None, 'sleep 120; :')
            old = create('worker-s5-native', 'sleep 2; :')
            rpc('surface.send_text', {'surface_id':old,'text':'S5 retained native work','queued':True})
            queue_id = next(e['id'] for e in rpc('queue.list',{'full':True})['entries'] if e.get('text') == 'S5 retained native work')
            with mock.patch.dict(os.environ, env, clear=True):
                spec = importlib.util.spec_from_file_location('phoenix_s5_native',REPO/'cysjavis-pack/bin/javis_phoenix.py')
                p = importlib.util.module_from_spec(spec); spec.loader.exec_module(p)
                p.CYS = str(CYS)
                p.observe_and_persist_roster(str(endpoint))
                until(lambda: any(s['surface_id'] == old and s['exited'] for s in surfaces()))
                exited_observed = time.monotonic()
                until(lambda: any(e['id'] == queue_id and e.get('restored') for e in rpc('queue.list')['entries']))
                spawn_ids = []
                def spawn(sock, role, observed, **kwargs):
                    assert sock == str(endpoint) and role == 'worker-s5-native'
                    r = cli('new-surface','--role',role,'--cmd','sleep 120; :','--cwd',str(root))
                    assert r.returncode == 0, r.stderr
                    ref = re.search(r'surface:\d+', r.stdout).group(0)
                    sid = int(ref.split(':')[1]); spawn_ids.append(sid); record_surface(sid)
                    return ref, 'owned inert replacement; no real agent'
                with contextlib.ExitStack() as stack:
                    stack.enter_context(mock.patch.object(p,'spawn_surrogate',spawn))
                    for name in ('stage_ready','stage_reinject','stage_g2_ack'):
                        stack.enter_context(mock.patch.object(p,name,lambda *a:(True,'fixture-only')))
                    stack.enter_context(mock.patch.object(p,'stage_observe_session',lambda *a,**k:(None,'no real agent session')))
                    first = p.run_restore(str(endpoint),ticket='s5-first',stub=True,no_breaker=True,print_result=False)
                    manifest['first_deferred_after_exit_observation_secs'] = round(time.monotonic()-exited_observed,3)
                    assert any(e['surface'] == 'surface:%d' % old for e in first['c6_reap_deferred']), 'S5 fresh deferred must be observed'
                    assert 'surface:%d' % old not in first['c6_reaped']
                    assert first['target_roles'] == ['worker-s5-native'] and len(spawn_ids) == 1
                    new = spawn_ids[0]
                    second = p.run_restore(str(endpoint),ticket='s5-second',stub=True,no_breaker=True,print_result=False)
                    assert second['target_roles'] == [] and spawn_ids == [new], 'S5 duplicate restore'
                until(lambda: any(e['id'] == queue_id and e.get('surface_id') == new and not e.get('restored') for e in rpc('queue.list')['entries']))
                assert any(s['surface_id'] == live and not s['exited'] for s in surfaces()), 'S5 live control lost'
                assert rpc('system.resolve_role',{'role':'worker-s5-native'})['surface_id'] == new
                # Only this fixture waits; restore does not wait through grace.
                deadline = exited_observed + GRACE + 0.25
                while time.monotonic() < deadline: time.sleep(min(0.1,deadline-time.monotonic()))
                due = cli('reap-due','surface:%d' % old)
                assert due.returncode == 0, due.stderr
                outcome = json.loads(due.stdout)
                assert outcome['surface_id'] == old and outcome['outcome'] in ('reaped','gone'), outcome
                assert all(s['surface_id'] != old for s in surfaces())
                assert rpc('system.resolve_role',{'role':'worker-s5-native'})['surface_id'] == new
                entries = [e for e in rpc('queue.list')['entries'] if e['id'] == queue_id]
                assert len(entries) == 1 and entries[0]['surface_id'] == new
                assert not json.loads((root/'topology.json').read_text()).get('tombstones')
                manifest.update(old=old,replacement=new,queue_id=queue_id,spawn_count=len(spawn_ids),later_due=outcome,
                                assertions='deferred, once-restored, once-rehomed, mapping, live control, no tombstones')
        except BaseException as exc:
            manifest['failure_type'] = type(exc).__name__
            raise
        finally:
            signal.alarm(0)
            cleanup_started = time.monotonic()
            try:
                signal.alarm(45)
                if daemon is not None:
                    capture()
                    if ready:
                        try:
                            token = (root/'operator.token').read_text().strip()
                            for row in surfaces(): rpc('surface.close',{'surface_id':row['surface_id'],'owner_token':token})
                            assert surfaces() == []
                        except Exception as exc: cleanup_errors.append(type(exc).__name__)
                    for pid, identity in owned.items():
                        if pid != daemon.pid and same_process(pid, identity):
                            try: os.kill(pid, signal.SIGKILL)
                            except ProcessLookupError: pass
                    if daemon.poll() is None:
                        daemon.terminate()
                        try: daemon.wait(timeout=5)
                        except subprocess.TimeoutExpired: daemon.kill(); daemon.wait(timeout=5)
                    until(lambda: all(not same_process(pid, identity) for pid,identity in owned.items()),5)
                    manifest['owned_residuals'] = 0
                    manifest['daemon_ready'] = ready
                    assert not cleanup_errors, 'graceful cleanup failed: '+repr(cleanup_errors)
                manifest['result'] = 'passed' if 'assertions' in manifest and 'failure_type' not in manifest else 'failed'
            except BaseException as exc:
                manifest['cleanup_failure_type'] = type(exc).__name__
                raise
            finally:
                signal.alarm(0); signal.signal(signal.SIGALRM, prior)
                manifest['cleanup_errors'] = cleanup_errors
                manifest['cleanup_elapsed_secs'] = round(time.monotonic()-cleanup_started,3)
                save_evidence()  # Runs even if body, capture, close, wait, or residual proof failed.
                print(json.dumps(manifest,sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
