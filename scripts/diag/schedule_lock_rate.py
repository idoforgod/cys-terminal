#!/usr/bin/env python3
"""Unmerged diagnostic only. One exact test; no release verdict or repair."""
import argparse, hashlib, json, math, os, platform, re, shutil, subprocess, sys, time
from pathlib import Path

BASE = '0b25c51158270804911bca82f23dc7f7ed066590'
TEST = 'schedule::tests::schedule_writers_serialize_under_the_lock_and_no_add_is_lost'
MARKER = 'SCHEDULE_LOCK_RATE '
N = 300

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def wilson(k, n):
    if not n:
        return None
    z = 1.959963984540054
    p = k / n
    d = 1 + z*z/n
    c = (p + z*z/(2*n))/d
    h = z*math.sqrt(p*(1-p)/n + z*z/(4*n*n))/d
    return [0.0 if k == 0 else max(0, c-h), 1.0 if k == n else min(1, c+h)]

def classify(rc, text, timed_out=False):
    if timed_out:
        return 'timeout'
    rows = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', text)
    if len(rows) != 1 or TEST not in text:
        return 'unclassified'
    verdict, passed, failed, ignored, measured, _ = rows[0]
    counts = tuple(map(int, (passed, failed, ignored, measured)))
    if rc == 0 and verdict == 'ok' and counts == (1, 0, 0, 0):
        return 'pass'
    if rc != 0 and verdict == 'FAILED' and counts == (0, 1, 0, 0):
        return 'fail'
    return 'unclassified'

def diagnostic_events(text):
    events, errors = [], []
    for line in text.splitlines():
        if MARKER in line:
            try:
                payload = line.split(MARKER, 1)[1]
                obj, end = json.JSONDecoder().raw_decode(payload)
                if payload[end:].strip():
                    raise ValueError('Trailing bytes after diagnostic JSON')
                events.append(obj)
            except (ValueError, TypeError) as e:
                errors.append(str(e))
    return events, errors

def sample_complete(rows, error, reconciled):
    return error is None and reconciled and len(rows) == N and all(row['outcome'] in ('pass', 'fail') for row in rows)

def valid_diagnostics(row):
    if row['event_parse_errors']:
        return False
    if row['writer_acquire_panic'] and not row['events']:
        return False
    for event in row['events']:
        if not isinstance(event, dict):
            return False
        if not {'schema', 'pid', 'lock_dir', 'wait_ms', 'remaining_ms', 'reason', 'kind', 'raw_os_error'} <= event.keys():
            return False
        if event.get('schema') != 1 or event.get('pid') != row['pid'] or not isinstance(event.get('lock_dir'), str):
            return False
        if not isinstance(event.get('wait_ms'), int) or not isinstance(event.get('remaining_ms'), int):
            return False
        if event.get('reason') == 'io':
            if not isinstance(event.get('kind'), str) or event['kind'] == 'AlreadyExists' or not event['kind']:
                return False
            if event.get('raw_os_error') is not None and not isinstance(event['raw_os_error'], int):
                return False
        elif event.get('reason') == 'deadline':
            if event.get('kind') is not None or event.get('raw_os_error') is not None:
                return False
        else:
            return False
    return True

def execute(command, cwd, env, path, timeout):
    start = time.time()
    monotonic_start = time.monotonic()
    kwargs = {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == 'nt' else {'start_new_session': True}
    with path.open('wb') as log:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, **kwargs)
        timed_out = False
        try:
            rc = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            # Only this directly owned child tree. Never a discovered/foreign PID.
            if os.name == 'nt':
                killed = subprocess.run(['taskkill', '/PID', str(child.pid), '/T', '/F'], capture_output=True, timeout=15)
                path.with_suffix('.kill.json').write_text(json.dumps({'rc': killed.returncode, 'stdout': killed.stdout.decode(errors='replace'), 'stderr': killed.stderr.decode(errors='replace')}))
                if killed.returncode != 0:
                    raise RuntimeError('Owned-tree termination unverified; stop all subsequent launches')
            else:
                import signal
                os.killpg(child.pid, signal.SIGKILL)
            rc = child.wait(timeout=15)
    return {'rc': rc, 'timeout': timed_out, 'pid': child.pid, 'start': start, 'end': time.time(), 'elapsed_s': time.monotonic()-monotonic_start, 'command': command, 'log': path.name, 'sha256': digest(path)}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--arm', choices=['baseline', 'instrumented'], required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[2]
    manifest = {'arm': args.arm, 'planned_n': N, 'test': TEST, 'base': BASE, 'started': time.time(), 'platform': platform.platform(), 'python': sys.version, 'python_executable': sys.executable, 'python_resolved': str(Path(sys.executable).resolve()), 'runner_image': {k: os.environ.get(k) for k in ['ImageOS', 'ImageVersion', 'RUNNER_OS', 'RUNNER_ARCH', 'GITHUB_SHA', 'GITHUB_REF', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT']}}
    rows = []
    started_count = 0
    error = None
    try:
        if os.name != 'nt':
            raise RuntimeError('Native measurement requires Windows; local parser controls are separate')
        def git(*command):
            return subprocess.check_output(['git', *command], cwd=repo, text=True).strip()
        tip = git('rev-parse', 'HEAD')
        if os.environ.get('GITHUB_SHA') != tip or os.environ.get('GITHUB_REF') != 'refs/heads/diag/0.14.56-schedule-lock-rate':
            raise RuntimeError('Workflow association is not the approved diagnostic tip/ref')
        first = git('rev-parse', 'HEAD^')
        if git('rev-parse', 'HEAD^^^') != BASE:
            raise RuntimeError('Expected exactly three diagnostic commits above the frozen base')
        if set(git('diff', '--name-only', BASE, first).splitlines()) != {'.github/workflows/diag-schedule-lock-rate.yml', 'scripts/diag/schedule_lock_rate.py'}:
            raise RuntimeError('Baseline commit changed unexpected inputs')
        if git('diff', '--name-only', first, tip) != 'src/bin/cysd/schedule.rs':
            raise RuntimeError('Instrumentation commit changed unexpected inputs')
        ref = BASE if args.arm == 'baseline' else tip
        source = out / 'source'
        subprocess.run(['git', 'worktree', 'add', '--detach', str(source), ref], cwd=repo, check=True)
        schedule = source / 'src/bin/cysd/schedule.rs'
        base_bytes = subprocess.check_output(['git', 'show', BASE + ':src/bin/cysd/schedule.rs'], cwd=repo)
        if args.arm == 'baseline' and schedule.read_bytes() != base_bytes:
            raise RuntimeError('Baseline schedule source differs from frozen candidate')
        env = {k: v for k, v in os.environ.items() if not k.startswith('CYS_') and k != 'CLAUDE_CONFIG_DIR'}
        env.update(CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(out / 'target'), CYS_PACK_DIR=str(out / 'build-pack'), RUST_MIN_STACK='8388608', PYTHONUTF8='1')
        Path(env['CYS_PACK_DIR']).mkdir()
        manifest['build_environment'] = {key: env.get(key) for key in ['PATH', 'HOME', 'USERPROFILE', 'TEMP', 'TMP', 'CYS_PACK_DIR', 'RUST_MIN_STACK', 'CARGO_TARGET_DIR', 'CARGO_INCREMENTAL', 'CARGO_BUILD_JOBS', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'PYTHONUTF8']}
        manifest['cargo_path'] = shutil.which('cargo', path=env['PATH'])
        manifest['rustc_path'] = shutil.which('rustc', path=env['PATH'])
        manifest.update(cpu_count=os.cpu_count(), free_bytes_before_build=shutil.disk_usage(out).free, tip=tip, protocol_commit=first, measured_ref=ref, schedule_sha256=digest(schedule), baseline_schedule_sha256=hashlib.sha256(base_bytes).hexdigest(), rustc=subprocess.check_output(['rustc', '-Vv'], env=env, text=True), cargo=subprocess.check_output(['cargo', '-V'], env=env, text=True), limits={'build_seconds': 1200, 'iteration_seconds': 30, 'measurement_budget_seconds': 1800}, rust_min_stack=env['RUST_MIN_STACK'])
        (out / 'manifest.json').write_text(json.dumps(manifest, indent=2))
        build = execute(['cargo', 'test', '--bin', 'cysd', '--no-run', '--message-format=json'], source, env, out / 'build.log', 1200)
        (out / 'build.json').write_text(json.dumps(build, indent=2))
        if build['rc'] != 0 or build['timeout']:
            raise RuntimeError('Build incomplete; no rate measurement')
        executables = set()
        for line in (out / 'build.log').read_text(errors='replace').splitlines():
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if message.get('reason') == 'compiler-artifact' and message.get('target', {}).get('name') == 'cysd' and message.get('profile', {}).get('test') and message.get('executable'):
                executables.add(message['executable'])
        if len(executables) != 1:
            raise RuntimeError('Cannot identify one test harness from Cargo output')
        exe = executables.pop()
        manifest['harness_sha256'] = digest(exe)
        listed = execute([exe, TEST, '--exact', '--list'], source, env, out / 'test-list.log', 30)
        listing = (out / 'test-list.log').read_text(errors='replace')
        if listed['rc'] != 0 or listed['timeout'] or [x for x in listing.splitlines() if x.endswith(': test')] != [TEST + ': test']:
            raise RuntimeError('Exact selector did not enumerate exactly the intended test')
        deadline = time.monotonic() + 1800
        for iteration in range(1, N + 1):
            if time.monotonic() + 60 > deadline:
                error = 'measurement budget exhausted; no optional extension/retry'
                break
            local = out / 'samples' / str(iteration)
            local.mkdir(parents=True)
            for leaf in ['temp', 'pack']:
                (local / leaf).mkdir()
            sample_env = dict(env, TEMP=str(local / 'temp'), TMP=str(local / 'temp'), CYS_PACK_DIR=str(local / 'pack'))
            started_count += 1
            with (out / 'iterations-started.jsonl').open('a') as journal:
                journal.write(json.dumps({'iteration': iteration, 'utc_epoch': time.time()}) + '\n')
            result = execute([exe, TEST, '--exact', '--nocapture', '--test-threads=1'], source, sample_env, out / ('iteration-%03d.log' % iteration), 30)
            text = (out / result['log']).read_text(errors='replace')
            events, parse_errors = diagnostic_events(text)
            result.update(iteration=iteration, outcome=classify(result['rc'], text, result['timeout']), writer_acquire_panic=('writer 잠금' in text), events=events, event_parse_errors=parse_errors, temp=str(local / 'temp'), pack=str(local / 'pack'))
            with (out / 'iterations.jsonl').open('a') as journal:
                journal.write(json.dumps(result) + '\n')
                journal.flush()
                os.fsync(journal.fileno())
            rows.append(result)
            # Never retry a failure, timeout, crash or parsing error. Continue planned independent samples.
    except Exception as exc:
        error = repr(exc)
    finally:
        counts = {name: sum(row['outcome'] == name for row in rows) for name in ['pass', 'fail', 'timeout', 'unclassified']}
        reconciled = False
        try:
            recorded = [json.loads(line) for line in (out / 'iterations.jsonl').read_text().splitlines()] if rows else []
            reconciled = recorded == rows
        except Exception as exc:
            error = str(error) + '; journal reconciliation: ' + repr(exc)
        complete = sample_complete(rows, error, reconciled)
        diagnostics_complete = None if args.arm == 'baseline' else all(valid_diagnostics(row) for row in rows) and len(rows) == N
        summary = {'planned_n': N, 'attempted': started_count, 'completed_records': len(rows), 'counts': counts, 'complete_classified_sample': complete, 'journal_reconciled': reconciled, 'diagnostic_attribution_complete': diagnostics_complete, 'error': error, 'rate': counts['fail']/N if complete else None, 'wilson_95': wilson(counts['fail'], N) if complete else None, 'unknown_including_unattempted': N-counts['pass']-counts['fail'], 'failure_fraction_bounds_if_incomplete': [counts['fail']/N, (N-counts['pass'])/N], 'blocks_50': [{'first': a+1, 'attempted': len(rows[a:a+50]), 'fail': sum(x['outcome']=='fail' for x in rows[a:a+50])} for a in range(0, N, 50)], 'zero_inner_failures_observed': (counts['fail'] == 0) if complete else None, 'release_verdict': 'NONE; CEO gate remains separate'}
        manifest['ended'] = time.time()
        (out / 'manifest.json').write_text(json.dumps(manifest, indent=2))
        (out / 'summary.json').write_text(json.dumps(summary, indent=2))
        print(json.dumps(summary))
    # A complete experiment may be RED. Exit 0 means collected, never release acceptance.
    return 0 if complete and diagnostics_complete is not False else 2

if __name__ == '__main__':
    sys.exit(main())
