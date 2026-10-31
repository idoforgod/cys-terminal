#!/usr/bin/env python3
"""Suite-regime observer; no execution, retry, repair or release approval."""
import argparse
import hashlib
import importlib.util
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from schedule_lock_rate import BASE, TEST, wilson

K = 20
REF = 'refs/heads/work/0.14.56-schedule-lock-rate'
COMMAND = 'CYS_PACK_DIR="$(mktemp -d)" cargo test --bin cysd -- --test-threads=1 --nocapture --skip hwmon::tests::snapshot_has_all_sections --skip attach_race_tests::process_and_send_are_atomic_under_parser_lock_no_dup_delivery 2>&1 | tee "$LOG"'
SUMMARY = re.compile(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;.*$')
TEST_LINE = re.compile(r'^test (\S+) \.\.\.\s?(.*)$')
NAME = re.compile(r'^    ([A-Za-z_][A-Za-z0-9_:]*)$')

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def c8_axis(text, reader_path, readable=True):
    # Reuse the BASE reader, never relax its native-marker contract.
    try:
        if sha(reader_path) != '5a60b8e03f92bd221184a5145038317f5865783efe8d4cdc5dc980c97becb749':
            raise RuntimeError('C8 reader differs from BASE')
        spec = importlib.util.spec_from_file_location('suite_base_c8', reader_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        axis = module.classify_native_markers(text) if readable else {'measurement_state': 'UNREADABLE', 'blocks_c8_gate': None}
        axis['reader_sha256'] = sha(reader_path)
    except Exception as error:
        axis = {'measurement_state': 'READER_ERROR', 'blocks_c8_gate': None, 'error': repr(error)}
    named_target = False
    for match in re.finditer(r'C8_NATIVE_UNMEASURED[^ \r\n]* ([^\r\n]*)', text):
        try:
            record = json.loads(match.group(1))
            named_target = named_target or (isinstance(record, dict) and record.get('test') == TEST)
        except ValueError:
            pass
    axis['target_named_unmeasured'] = named_target
    axis['interpretation'] = 'Independent marker axis; NO_UNMEASURED_MARKER is not a C8 pass or coverage proof.'
    return axis

def parse_suite(text, rc):
    visible = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)
    lines = visible.replace('\r\n', '\n').replace('\r', '\n').splitlines()
    reason = []
    launches = [i for i, line in enumerate(lines) if re.search(r'^\s*Running unittests .*cysd[\\/]main\.rs', line)]
    root_start, root_n = None, None
    if launches:
        for i in range(launches[0]+1, len(lines)):
            match = re.fullmatch(r'running (\d+) tests?', lines[i])
            if match:
                root_start, root_n = i, int(match[1])
                break
    candidates, all_summaries = [], []
    for i, line in enumerate(lines):
        match = SUMMARY.match(line)
        if not match:
            continue
        kind, passed, failed, ignored, measured, filtered = match.groups()
        row = {'line': i+1, 'kind': kind, 'passed': int(passed), 'failed': int(failed), 'ignored': int(ignored), 'measured': int(measured), 'filtered': int(filtered), 'text': line}
        all_summaries.append(row)
        if root_start is not None and i > root_start and row['passed']+row['failed']+row['ignored'] == root_n and row['measured'] == 0 and row['filtered'] == 2:
            candidates.append(row)
    if root_start is None:
        reason.append('missing root cysd launch/running count')
    if len(candidates) != 1:
        reason.append('missing or ambiguous parent summary')
    parent = candidates[0] if len(candidates) == 1 else None
    failed_names, target = [], 'unknown'
    if parent:
        end = parent['line']-1
        if parent['failed']:
            sections = [i for i in range(root_start+1, end) if lines[i] == 'failures:']
            if sections:
                failed_names = [m.group(1) for line in lines[sections[-1]+1:end] if (m := NAME.match(line))]
            if len(failed_names) != parent['failed'] or len(set(failed_names)) != len(failed_names):
                reason.append('parent failure-name/count mismatch')
        if rc is None or (parent['failed'] == 0 and (rc != 0 or parent['kind'] != 'ok')) or (parent['failed'] > 0 and (rc == 0 or parent['kind'] != 'FAILED')):
            reason.append('missing or contradictory cargo exit status')
        starts = [(i, m.group(2)) for i in range(root_start+1, end) if (m := TEST_LINE.match(lines[i])) and m.group(1) == TEST]
        if len(starts) == 1:
            start, rest = starts[0]
            next_test = next((i for i in range(start+1, end) if TEST_LINE.match(lines[i])), end)
            block = lines[start+1:next_test]
            ignored = rest.startswith('ignored') or any(line.startswith('ignored') for line in block)
            if ignored:
                target = 'ignored'
            elif TEST in failed_names:
                target = 'fail'
            elif rest == 'ok' or 'ok' in block:
                target = 'pass'
        else:
            target = 'not_reached' if not starts else 'ambiguous'
        if target not in ('pass', 'fail'):
            reason.append('target not observed as pass/fail')
    return {'complete': not reason, 'reasons': reason, 'root_test_count': root_n, 'parent_summary': parent, 'all_summaries': all_summaries, 'failed_tests': sorted(failed_names), 'target_status': target, 'other_failed_tests': sorted(n for n in failed_names if n != TEST), 'cargo_rc': rc}

def aggregate(records, expected_sha, expected_run, expected_attempt):
    reasons = []
    if any(not isinstance(r, dict) for r in records):
        return {'planned_K': K, 'complete_fixed_sample': False, 'reasons': ['malformed record'], 'target_rate': None, 'wilson_95': None}
    for r in records:
        if any(r.get(key) is not None and not isinstance(r.get(key), str) for key in ['workflow_sha', 'workflow_ref', 'run_id', 'run_attempt', 'source_head']):
            return {'planned_K': K, 'complete_fixed_sample': False, 'reasons': ['malformed association fields'], 'target_rate': None, 'wilson_95': None}
        if not isinstance(r.get('failed_tests', []), list) or any(not isinstance(x, str) for x in r.get('failed_tests', [])):
            return {'planned_K': K, 'complete_fixed_sample': False, 'reasons': ['malformed failure list'], 'target_rate': None, 'wilson_95': None}
    ids = [r.get('sample_id') for r in records]
    if len(records) != K or sorted(i for i in ids if isinstance(i, int)) != list(range(1, K+1)):
        reasons.append('fixed denominator requires exactly sample IDs1..20 once each')
    associations = {(r.get('workflow_sha'), r.get('run_id'), r.get('run_attempt'), r.get('workflow_ref')) for r in records}
    if len(associations) != 1 or any(not all(x) or x[3] != REF for x in associations):
        reasons.append('missing or mixed run/attempt/association')
    if any(r.get('source_head') != BASE or r.get('complete') is not True or r.get('target_status') not in ('pass', 'fail') for r in records):
        reasons.append('missing source identity or incomplete suite/target observation')
    if any(type(r.get('sample_id')) is not int or type(r.get('shard')) is not int for r in records):
        reasons.append('noninteger sample/shard')
    elif any(r.get('shard') != (r.get('sample_id', 0)+1)//2 or r.get('position') != (r.get('sample_id', 0)-1)%2+1 for r in records):
        reasons.append('sample assigned to wrong shard')
    if any((TEST in r.get('failed_tests', [])) != (r.get('target_status') == 'fail') for r in records):
        reasons.append('target and complete failure list disagree')
    if associations != {(expected_sha, str(expected_run), str(expected_attempt), REF)}:
        reasons.append('records do not match the caller-pinned approved run')
    complete = not reasons
    fails = sum(r.get('complete') is True and r.get('target_status') == 'fail' for r in records)
    all_failure_counts = {}
    for row in records:
        if row.get('complete') is not True:
            continue
        for name in row.get('failed_tests', []):
            all_failure_counts[name] = all_failure_counts.get(name, 0)+1
    return {'planned_K': K, 'received_records': len(records), 'complete_fixed_sample': complete, 'reasons': reasons, 'target_failures_observed': fails, 'target_rate': fails/K if complete else None, 'wilson_95': wilson(fails, K) if complete else None, 'suite_red_runs': sum(r.get('complete') is True and bool(r.get('failed_tests')) for r in records), 'failure_census_scope': 'Complete available records only; partial/unverified facts remain in per_job. No approved-run rate unless complete_fixed_sample.', 'all_failed_test_counts': all_failure_counts, 'c8_marker_axes': [{'sample_id': r.get('sample_id'), 'axis': r.get('c8_marker_axis')} for r in records], 'per_job': [{'shard': j, 'samples': [r for r in records if r.get('shard') == j]} for j in range(1, 11)], 'release_verdict': 'NONE; original suite red and CEO gate remain separate'}

def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest='mode', required=True)
    for mode in ('prepare', 'collect'):
        p = sub.add_parser(mode)
        p.add_argument('--out', type=Path, required=True)
        p.add_argument('--shard', type=int, required=True)
        if mode == 'collect':
            p.add_argument('--position', type=int, choices=(1, 2), required=True)
            p.add_argument('--step-outcome', required=True)
    p = sub.add_parser('aggregate')
    p.add_argument('--root', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--expected-sha', required=True)
    p.add_argument('--run-id', required=True)
    p.add_argument('--attempt', required=True)
    args = ap.parse_args()
    if args.mode == 'aggregate':
        if args.out.exists():
            print(json.dumps({'complete_fixed_sample': False, 'target_rate': None, 'reason': 'Output already exists; choose a fresh evidence path'}))
            return 2
        records = []
        for record_path in sorted(args.root.rglob('suite-sample.json')):
            try:
                row = json.loads(record_path.read_text(encoding='utf-8'))
                if not isinstance(row, dict):
                    raise ValueError('record is not an object')
            except (OSError, ValueError) as exc:
                records.append({'complete': False, 'reasons': [repr(exc)]})
                continue
            logfile = record_path.parent/'cargo.log'
            if not logfile.exists() or sha(logfile) != row.get('log_sha256'):
                row['complete'] = False
                row.setdefault('reasons', []).append('raw log missing or hash mismatch')
            else:
                logtext = logfile.read_text(encoding='utf-8', errors='replace')
                fresh = parse_suite(logtext, row.get('cargo_rc'))
                axis = c8_axis(logtext, Path(__file__).resolve().parents[1]/'c8_native_measurement.py')
                row['c8_marker_axis'] = axis
                if axis['target_named_unmeasured']:
                    row['complete'] = False
                if not fresh['complete'] or any(row.get(k) != fresh.get(k) for k in ['target_status', 'failed_tests', 'parent_summary']):
                    row['complete'] = False
                    row.setdefault('reasons', []).append('raw log reparsing disagrees')
            records.append(row)
        result = aggregate(records, args.expected_sha, args.run_id, args.attempt)
        with args.out.open('x', encoding='utf-8') as output:
            json.dump(result, output, indent=2)
        print(json.dumps(result))
        return 0 if result['complete_fixed_sample'] else 2
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if not 1 <= args.shard <= 10:
        raise RuntimeError('Unexpected shard')
    if args.mode == 'prepare':
        head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
        workflow_sha = os.environ.get('GITHUB_SHA', '')
        if head != BASE or os.environ.get('GITHUB_REF') != REF or not re.fullmatch(r'[0-9a-f]{40}', workflow_sha) or workflow_sha == BASE:
            raise RuntimeError('Wrong source or workflow association')
        subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--', 'src', 'Cargo.toml', 'Cargo.lock', 'build.rs', '.cargo', 'cysjavis-pack'], check=True)
        data = {'regime': 'full-cysd suite with original warmup prefix', 'source_head': head, 'workflow_sha': os.environ.get('GITHUB_SHA'), 'workflow_ref': os.environ.get('GITHUB_REF'), 'run_id': os.environ.get('GITHUB_RUN_ID'), 'run_attempt': os.environ.get('GITHUB_RUN_ATTEMPT'), 'shard': args.shard, 'sample_ids': [2*args.shard-1, 2*args.shard], 'prepared_at': time.time(), 'python': sys.version, 'python_executable': sys.executable, 'platform': platform.platform(), 'cpu_count': os.cpu_count(), 'image': {k: os.environ.get(k) for k in ['ImageOS', 'ImageVersion', 'RUNNER_OS', 'RUNNER_ARCH']}, 'parent_environment': {k: os.environ.get(k) for k in ['PATH', 'HOME', 'USERPROFILE', 'TEMP', 'TMP', 'CYS_PACK_DIR', 'RUST_MIN_STACK', 'RUSTFLAGS', 'CARGO_TARGET_DIR', 'CARGO_INCREMENTAL', 'PYTHONUTF8']}, 'command': COMMAND, 'command_local_env_note': 'CYS_PACK_DIR is a fresh mktemp per original command; Cargo applies BASE .cargo/config.toml; parent snapshot is not child-env proof', 'config_sha256': sha('.cargo/config.toml'), 'schedule_sha256': sha('src/bin/cysd/schedule.rs'), 'windows_health_sha256': sha('.github/workflows/windows-health.yml'), 'observer_sha256': sha(__file__), 'rustc': subprocess.check_output(['rustc', '-Vv'], text=True), 'cargo': subprocess.check_output(['cargo', '-V'], text=True)}
        (out/'manifest.json').write_text(json.dumps(data, indent=2), encoding='utf-8')
        return 0
    raw = Path(os.environ['RUNNER_TEMP'])/'cysd-win'
    dest = out/('sample-'+str(2*args.shard-2+args.position))
    dest.mkdir(exist_ok=False)
    manifest = json.loads((out/'manifest.json').read_text(encoding='utf-8')) if (out/'manifest.json').exists() else {}
    rc = None
    if (raw/'rc').exists():
        try:
            rc = int((raw/'rc').read_text().strip())
        except ValueError:
            pass
    if (raw/'cargo.log').exists():
        shutil.copyfile(raw/'cargo.log', dest/'cargo.log')
        text = (dest/'cargo.log').read_text(encoding='utf-8', errors='replace')
    else:
        text = ''
    result = parse_suite(text, rc)
    axis = c8_axis(text, Path(os.environ['GITHUB_WORKSPACE'])/'scripts/c8_native_measurement.py', bool(text))
    if axis['target_named_unmeasured']:
        result['complete'] = False
        result['target_status'] = 'unmeasured'
        result['reasons'].append('target itself named by an UNMEASURED marker')
    start_epoch = None
    try:
        start_epoch = int((raw/'start').read_text().strip())
    except (OSError, ValueError):
        pass
    stale = False
    if args.position == 2:
        previous = out/('sample-'+str(2*args.shard-1))/'suite-sample.json'
        try:
            previous_start = json.loads(previous.read_text(encoding='utf-8')).get('original_start_epoch')
            stale = start_epoch is None or previous_start is None or start_epoch <= previous_start
        except (OSError, ValueError):
            stale = True
    if args.step_outcome not in ('success', 'failure') or start_epoch is None or stale or (args.step_outcome == 'success' and rc != 0) or (args.step_outcome == 'failure' and rc == 0):
        result = {'complete': False, 'reasons': ['skipped, missing start, or stale prior-suite log'], 'target_status': 'unknown', 'failed_tests': [], 'other_failed_tests': [], 'cargo_rc': rc, 'untrusted_parse': result}
    result['c8_marker_axis'] = axis
    result.update({key: manifest.get(key) for key in ['source_head', 'workflow_sha', 'workflow_ref', 'run_id', 'run_attempt']})
    result.update(shard=args.shard, position=args.position, sample_id=2*args.shard-2+args.position, collected_at=time.time(), original_start_epoch=start_epoch, step_outcome=args.step_outcome, log_sha256=sha(dest/'cargo.log') if (dest/'cargo.log').exists() else None)
    for filename in ['start', 'rc', 'failures.py']:
        if (raw/filename).exists():
            shutil.copyfile(raw/filename, dest/filename)
    native_failures = Path(os.environ['GITHUB_WORKSPACE'])/'win-cysd-failures.json'
    if native_failures.exists():
        shutil.copyfile(native_failures, dest/'original-failures.json')
    (dest/'suite-sample.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps(result))
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as outputs:
            outputs.write('complete=' + str(result['complete']).lower() + '\n')
    return 0 if result['complete'] else 2

if __name__ == '__main__':
    sys.exit(main())

