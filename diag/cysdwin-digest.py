#!/usr/bin/env python3
# diag/cysdwin-digest.py -- turns the log of `cargo test --bin cysd -- --test-threads=1 --nocapture ...` (job cysd-win of
# .github/workflows/diag-win11-update.yml, windows-latest) into files that can be read anonymously from a results branch:
#   cysdwin-failures/<seq>-<test name>.txt   one file per failing test: the part of the log that belongs to it (capped)
#   cysdwin-digest.tsv                       one line per failing test: name, panic location, panic message (first 300 characters)
#   cysdwin-summary.txt                      test result line(s), counts, failures per module / panic location / message type, rules
#   cysdwin-digest.json                      the same numbers for programs
#   cysdwin-cargo.log                        the whole log (cut to its first and last part when it is too big for the publisher)
#   cysdwin-meta.json                        what was run (ref, filter, skip, rc, times, sizes)
# Python 3.8+, standard library only. ASCII-only source; the log is read as UTF-8 (undecodable bytes replaced).
# NEVER fails the job: any problem is written into the summary and the exit code stays 0.
#
# Two log shapes are understood (the job uses --nocapture, so the first one is the normal one):
#   nocapture  with --test-threads=1 libtest prints "test <name> ... " BEFORE the test runs, so everything the test prints (the panic
#              message "thread '<n>' panicked at <file>:<line>:<col>:" + text) sits between its "test <name> ... " line and the next
#              "test <other> ... " line; the result ("ok" / "FAILED") ends that part. The failing names are the list after the last
#              "failures:" line ("    <name>" lines).
#   captured   "---- <name> stdout ----" blocks after the first "failures:" line (used when a log comes from a run without --nocapture).
import argparse
import bisect
import collections
import io
import json
import os
import re
import shutil
import sys
import time
import traceback

VERSION = 1
ANSI = re.compile(r'\x1b\[[0-9;?]*[A-Za-z]')
START = re.compile(r'^test (\S+)(?: - should panic)? \.\.\. ?(.*)$')
RESULT = re.compile(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out(?:; finished in (.*))?$')
BLOCK = re.compile(r'^---- (\S+) (?:stdout|stderr) ----$')
PANIC_NEW = re.compile(r"thread '([^']*)'(?: \(\d+\))? panicked at (.+?):(\d+):(\d+):\s*$")
PANIC_OLD = re.compile(r"thread '([^']*)'(?: \(\d+\))? panicked at '(.*?)', (\S+?):(\d+):(\d+)", re.S)

# message type rules: (label, regex). The FIRST matching rule is the primary label of a failure (the primary counts add up to the
# number of failures); the "any match" table counts every rule that matches. Texts are matched case-insensitively.
RULES = [
    ('no_such_file', r'no such file or directory|cannot find the (?:path|file) specified|the system cannot find|\bos error (?:2|3)\b|\bNotFound\b'),
    ('permission', r'permission denied|access is denied|\bos error (?:5|13)\b|\bPermissionDenied\b'),
    ('symlink_privilege', r'symlink|a required privilege is not held|\bos error 1314\b'),
    ('pipe_socket', r'named pipe|\bpipe\b|socket|connection (?:refused|reset|aborted)|\bos error (?:109|231|232|10048|10054|10061)\b|\bConnectionRefused\b|\bBrokenPipe\b'),
    ('timeout', r'timed out|timeout|deadline'),
    ('encoding', r'utf-?8|invalid utf|code ?page|cp949|cp1252|encoding'),
    ('home_userprofile', r'(?-i:\bHOME\b|USERPROFILE|APPDATA|LOCALAPPDATA)|home_dir|home dir'),
    ('not_supported', r'not supported|unsupported|not implemented|only on (?:unix|macos|windows)'),
    ('path_backslash', r'[A-Za-z]:\\|\\\\\?\\|\\[A-Za-z0-9_.-]+\\|[A-Za-z0-9_.-]+\\[A-Za-z0-9_.-]+\.[A-Za-z0-9]+'),
    ('os_error_other', r'\bos error \d+\b|\bio::Error\b|Error \{ kind'),
    ('assert_left_right', r'assertion `?left (?:==|!=) right`? failed|^\s*left:\s|^\s*right:\s'),
    ('assert_failed', r'assertion failed'),
    ('unwrap_none', r'called `Option::unwrap\(\)` on a `None` value|Option::expect'),
    ('unwrap_err', r'called `Result::unwrap\(\)` on an `Err` value|Result::expect'),
]
RULES_C = [(label, re.compile(rx, re.I | re.M)) for label, rx in RULES]


def esc_annotation(s):
    return s.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')


def safe_name(name, limit=100):
    s = re.sub(r'[^A-Za-z0-9._-]+', '_', name.replace('::', '__'))
    s = s.strip('._') or 'test'
    return s[:limit]


def one_line(s, limit):
    s = re.sub(r'[\t\r\n]+', ' / ', str(s))
    s = re.sub(r' +', ' ', s).strip()
    return s if len(s) <= limit else s[:limit]


def module_of(name):
    # the module before "::tests::"; names without it: their first path segment; a bare "tests::x": (root tests)
    if '::tests::' in name:
        return name.split('::tests::', 1)[0]
    if name.startswith('tests::'):
        return '(root tests)'
    if '::' in name:
        return name.split('::', 1)[0]
    return '(no module)'


def modpath_of(name):
    return name.rsplit('::', 1)[0] if '::' in name else '(no module)'


def norm_message(s):
    s = s[:400]
    s = re.sub(r'[A-Za-z]:\\[^\s\'"`,;)]*', '<WINPATH>', s)
    s = re.sub(r'\\\\\?\\[^\s\'"`,;)]*', '<WINPATH>', s)
    s = re.sub(r'(?:/[A-Za-z0-9_.-]+){2,}', '<POSIXPATH>', s)
    s = re.sub(r'0x[0-9a-fA-F]+', '<HEX>', s)
    s = re.sub(r'\b[0-9a-fA-F]{8,}\b', '<ID>', s)
    s = re.sub(r'\.tmp[A-Za-z0-9]+', '.tmp<X>', s)
    s = re.sub(r'\d+', 'N', s)
    return s[:160]


def norm_loc(path, line):
    return '%s:%s' % (path.replace('\\', '/'), line)


def extract_panics(seg_lines):
    """[{thread, path, line, col, msg}] in log order; new format (message on the following lines) first, old format as a fallback."""
    out = []
    i = 0
    n = len(seg_lines)
    while i < n:
        m = PANIC_NEW.search(seg_lines[i]) if len(seg_lines[i]) <= 4000 else None
        if not m:
            i += 1
            continue
        msg_lines = []
        j = i + 1
        while j < n and len(msg_lines) < 40:
            t = seg_lines[j]
            if t.startswith('note: run with `RUST_BACKTRACE') or t.startswith('stack backtrace:') or START.match(t):
                break
            if len(t) <= 4000 and PANIC_NEW.search(t):
                break
            msg_lines.append(t)
            j += 1
        out.append({'thread': m.group(1), 'path': m.group(2), 'line': m.group(3), 'col': m.group(4), 'msg': '\n'.join(msg_lines).strip('\n')})
        i = j
    if not out:
        text = '\n'.join(seg_lines)
        for m in PANIC_OLD.finditer(text):
            out.append({'thread': m.group(1), 'path': m.group(3), 'line': m.group(4), 'col': m.group(5), 'msg': m.group(2)})
    return out


def classify(msg):
    primary = 'other'
    anys = []
    for label, rx in RULES_C:
        if rx.search(msg):
            anys.append(label)
    if anys:
        primary = anys[0]
    return primary, anys


def cap_bytes(text, limit):
    b = text.encode('utf-8', 'replace')
    if len(b) <= limit:
        return text
    half = max(1, (limit - 80) // 2)
    head = b[:half].decode('utf-8', 'ignore')
    tail = b[-half:].decode('utf-8', 'ignore')
    return head + '\n[... %d bytes cut here (a file is capped; head and tail are kept) ...]\n' % (len(b) - 2 * half) + tail


def parse(text):
    """-> dict with everything the writers need (no I/O)."""
    text = text.replace('\r\n', '\n').replace('\r', '\n')
    text = ANSI.sub('', text)
    lines = text.split('\n')
    a = {'lines_total': len(lines)}
    # test start markers
    starts = []
    for i, ln in enumerate(lines):
        if ln.startswith('test '):
            m = START.match(ln)
            if m:
                starts.append((i, m.group(1), m.group(2)))
    a['starts'] = starts
    # boundaries: "failures:" lines and "test result:" lines
    bounds = []
    result_lines = []
    fail_hdr = []
    for i, ln in enumerate(lines):
        if ln == 'failures:':
            bounds.append(i)
            fail_hdr.append(i)
        elif ln.startswith('test result:'):
            bounds.append(i)
            result_lines.append(ln.strip())
    a['result_lines'] = result_lines
    a['result_parsed'] = [RESULT.match(r) for r in result_lines]
    # failing names: the last non-empty "    name" list after a "failures:" line
    listed = []
    for i in reversed(fail_hdr):
        j = i + 1
        cand = []
        while j < len(lines) and lines[j].startswith('    ') and lines[j].strip():
            cand.append(lines[j].strip())
            j += 1
        if cand:
            listed = cand
            break
    a['listed'] = listed
    # captured-mode blocks
    blocks = {}
    cur = None
    buf = []
    for i, ln in enumerate(lines):
        m = BLOCK.match(ln)
        if m or ln == 'failures:' or ln.startswith('test result:'):
            if cur is not None:
                blocks.setdefault(cur, []).extend(buf)
            cur = m.group(1) if m else None
            buf = []
        elif cur is not None:
            buf.append(ln)
    if cur is not None:
        blocks.setdefault(cur, []).extend(buf)
    a['blocks'] = blocks
    # segments per test name (the last start line of a name wins)
    start_idx = [s[0] for s in starts]
    bound_idx = sorted(bounds)
    seg = {}
    status = {}
    for k, (i, name, rest) in enumerate(starts):
        end = starts[k + 1][0] if k + 1 < len(starts) else len(lines)
        b = bisect.bisect_right(bound_idx, i)
        if b < len(bound_idx) and bound_idx[b] < end:
            end = bound_idx[b]
        body = lines[i:end]
        while body and not body[-1].strip():
            body.pop()
        seg[name] = body
        st = None
        r = rest.strip()
        if r in ('ok', 'FAILED') or r.startswith('ignored'):
            st = r.split(',')[0]
        else:
            last = body[-1].strip() if body else ''
            if last in ('ok', 'FAILED') or last.startswith('ignored'):
                st = last.split(',')[0]
            elif r.endswith('FAILED'):
                st = 'FAILED'
        status[name] = st
    a['seg'] = seg
    a['status'] = status
    # the names that failed: the libtest list; else (cut log) the parts whose status is FAILED
    names = list(listed)
    a['names_source'] = 'failures list' if names else ''
    if not names:
        names = [n for (_, n, _) in starts if status.get(n) == 'FAILED']
        a['names_source'] = 'FAILED marks (no failures list in the log)' if names else 'none'
    seen = set()
    ordered = []
    for n in names:
        if n not in seen:
            seen.add(n)
            ordered.append(n)
    first_pos = {}
    for (i, n, _) in starts:
        first_pos.setdefault(n, i)
    ordered.sort(key=lambda n: (first_pos.get(n, 10 ** 9), n))
    a['names'] = ordered
    # last started test and whether it ended
    if starts:
        i, n, rest = starts[-1]
        a['last_started'] = n
        a['last_started_ended'] = status.get(n) is not None
    else:
        a['last_started'] = ''
        a['last_started_ended'] = False
    a['status_counts'] = collections.Counter(v if v else 'no result mark' for v in status.values())
    # per failure records
    recs = []
    for k, n in enumerate(ordered, 1):
        if n in blocks and blocks[n]:
            body = blocks[n]
            src = 'captured block "---- name stdout ----"'
        elif n in seg:
            body = seg[n]
            src = 'log part between "test <name> ... " and the next test line'
        else:
            body = []
            src = 'none (the name is listed but its part of the log was not found)'
        panics = extract_panics(body)
        msg = ''
        loc = ''
        thread = ''
        panic_at = ''
        if panics:
            p0 = panics[0]
            msg = p0['msg']
            loc = norm_loc(p0['path'], p0['line'])
            thread = p0['thread']
            panic_at = 'panicked at %s:%s:%s' % (p0['path'], p0['line'], p0['col'])
        if not msg.strip():
            for t in body:
                t2 = re.sub(r'^test \S+(?: - should panic)? \.\.\. ?', '', t).strip()
                if t2 and t2 not in ('FAILED', 'ok'):
                    msg = t2
                    break
        if not msg.strip():
            msg = '(no panic text found)'
        cls_text = msg if panics else msg + '\n' + '\n'.join(body[:15])
        primary, anys = classify(cls_text)
        recs.append({'seq': k, 'name': n, 'module': module_of(n), 'modpath': modpath_of(n), 'body': body, 'source': src, 'panics': panics,
                     'msg': msg, 'loc': loc, 'panic_at': panic_at, 'thread': thread, 'primary': primary, 'anys': anys})
    a['recs'] = recs
    return a


def write_text(path, text):
    with io.open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


def top(counter, n):
    return counter.most_common(n)


def build_summary(a, ctx):
    L = []
    add = L.append
    add('cysd-win digest (diag/cysdwin-digest.py v%d) - generated %s' % (VERSION, time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())))
    add('')
    add('== what was run ==')
    add('product ref   : %s' % (ctx.get('ref') or '(none)'))
    add('filter        : %s' % (ctx.get('filter') or '(none)'))
    add('skip          : %s' % (ctx.get('skip') or '(none)'))
    add('cargo command : %s' % (ctx.get('cmd') or '(not recorded)'))
    rc = ctx.get('rc')
    add('cargo rc      : %s' % (rc if rc not in (None, '') else 'none (no rc mark: the cargo step did not finish - cut by its time limit, or it never ran)'))
    if ctx.get('elapsed') is not None:
        add('elapsed       : %d s (%d min %d s)' % (ctx['elapsed'], ctx['elapsed'] // 60, ctx['elapsed'] % 60))
    if ctx.get('outcomes'):
        add('step outcomes : %s' % ', '.join(ctx['outcomes']))
    if ctx.get('why'):
        add('NO INPUT      : %s' % ctx['why'])
    add('')
    if a is None:
        add('== log ==')
        add('no cargo log was found (%s): nothing to digest.' % ctx.get('log_path', '?'))
        return '\n'.join(L) + '\n'
    add('== test result lines (%d) ==' % len(a['result_lines']))
    for r in a['result_lines']:
        add(r)
    if not a['result_lines']:
        add('(none: the run did not reach its summary - compile error, hang cut by the time limit, or the harness died)')
    add('')
    recs = a['recs']
    add('== counts ==')
    add('"test <name> ... " lines in the log      : %d' % len(a['starts']))
    add('result marks of those lines              : %s' % (', '.join('%s=%d' % kv for kv in sorted(a['status_counts'].items())) or 'none'))
    add('failing tests                            : %d   (taken from: %s)' % (len(recs), a['names_source'] or 'none'))
    add('  with a log part                        : %d' % sum(1 for r in recs if r['body']))
    add('  with a panic location                  : %d' % sum(1 for r in recs if r['loc']))
    add('  with more than one panic in their part : %d' % sum(1 for r in recs if len(r['panics']) > 1))
    parsed = [m for m in a['result_parsed'] if m]
    if parsed:
        failed_total = sum(int(m.group(3)) for m in parsed)
        add('failed according to the result line(s)   : %d -> %s' % (failed_total, 'agrees with the list' if failed_total == len(recs) else 'DOES NOT AGREE with the list (%d)' % len(recs)))
    add('last started test                        : %s%s' % (a['last_started'] or '(none)', '' if a['last_started_ended'] or not a['last_started'] else '  <- no result mark after it (a hang cut by the time limit?)'))
    add('')
    mod = collections.Counter(r['module'] for r in recs)
    add('== failures per module (name before "::tests::"; else the first path segment) ==')
    for k, v in sorted(mod.items(), key=lambda kv: (-kv[1], kv[0])):
        add('%5d  %s' % (v, k))
    if not mod:
        add('(none)')
    add('')
    mp = collections.Counter(r['modpath'] for r in recs)
    add('== failures per full module path (top 40) ==')
    for k, v in top(mp, 40):
        add('%5d  %s' % (v, k))
    if not mp:
        add('(none)')
    add('')
    loc = collections.Counter(r['loc'] for r in recs if r['loc'])
    add('== failures per panic location (file:line of the FIRST panic of the test; top 30; backslashes shown as /) ==')
    for k, v in top(loc, 30):
        add('%5d  %s' % (v, k))
    if not loc:
        add('(none)')
    add('')
    prim = collections.Counter(r['primary'] for r in recs)
    add('== failures per message type (PRIMARY label = first matching rule below; the counts add up to the number of failures) ==')
    for k, v in sorted(prim.items(), key=lambda kv: (-kv[1], kv[0])):
        add('%5d  %s' % (v, k))
    if not prim:
        add('(none)')
    add('')
    anyc = collections.Counter(lbl for r in recs for lbl in r['anys'])
    add('== failures per message type (ANY match: a failure counts for every rule that matches it) ==')
    for k, v in sorted(anyc.items(), key=lambda kv: (-kv[1], kv[0])):
        add('%5d  %s' % (v, k))
    if not anyc:
        add('(none)')
    add('')
    first = collections.Counter(norm_message(next((x for x in r['msg'].split('\n') if x.strip()), r['msg'])) for r in recs)
    add('== first message lines, normalized (paths -> <WINPATH>/<POSIXPATH>, hex ids, digits -> N; top 30) ==')
    for k, v in top(first, 30):
        add('%5d  %s' % (v, k))
    if not first:
        add('(none)')
    add('')
    add('== message type rules (text = the panic message; when a test has no panic line: its message + the first 15 lines of its log part) ==')
    add('matched case-insensitively, in this order; the first match is the primary label, "other" when nothing matches:')
    for label, rx in RULES:
        add('  %-18s %s' % (label, rx))
    add('')
    add('== files ==')
    add('cysdwin-failures/        %d file(s), one per failing test, each capped at %d bytes (head + tail kept when longer)' % (len(recs), ctx.get('failure_max', 0)))
    add('cysdwin-digest.tsv       seq, test, panic_at (as printed), location (file:line), message (first 300 characters), thread, panics, file')
    add('cysdwin-digest.json      the same numbers for programs')
    raw = ctx.get('raw') or {}
    add('cysdwin-cargo.log        %s' % raw.get('note', '(not written)'))
    add('cysdwin-meta.json / cysdwin-env.txt   what was run and the runner facts')
    add('')
    if not a['result_lines']:
        errs = [t for t in a.get('tail_all', []) if re.match(r'^error(\[E\d+\])?:', t)][:20]
        if errs:
            add('== lines that start with "error" (first 20 of the whole log) ==')
            for t in errs:
                add(t)
            add('')
        add('== last 40 lines of the log (no test result line: compile error or cut run) ==')
        tail = a.get('tail', [])
        for t in tail:
            add(t)
        add('')
    return '\n'.join(L) + '\n'


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument('--dir', default='', help='folder with cargo.log and the rc / start / end / cmd.txt marks of the cargo step')
    ap.add_argument('--log', default='', help='the cargo log (default: <dir>/cargo.log)')
    ap.add_argument('--out', required=True)
    ap.add_argument('--ref', default='')
    ap.add_argument('--filter', default='')
    ap.add_argument('--skip', default='')
    ap.add_argument('--why', default='', help='why the job had no usable input')
    ap.add_argument('--outcome', action='append', default=[], help='name=value of an earlier step (repeatable)')
    ap.add_argument('--raw-max-bytes', type=int, default=39 * 1024 * 1024)
    ap.add_argument('--failure-max-bytes', type=int, default=10 * 1024)
    ap.add_argument('--annotate', action='store_true', help='print ::notice workflow commands with the head of the summary')
    args = ap.parse_args(argv)
    out = args.out
    os.makedirs(out, exist_ok=True)

    def rd(name):
        try:
            with io.open(os.path.join(args.dir, name), 'r', encoding='utf-8', errors='replace') as f:
                return f.read().strip()
        except Exception:
            return None

    log_path = args.log or (os.path.join(args.dir, 'cargo.log') if args.dir else '')
    ctx = {'ref': args.ref, 'filter': args.filter, 'skip': args.skip, 'why': args.why, 'outcomes': args.outcome, 'log_path': log_path,
           'failure_max': args.failure_max_bytes, 'cmd': rd('cmd.txt') if args.dir else None}
    rc = rd('rc') if args.dir else None
    ctx['rc'] = rc
    st = rd('start') if args.dir else None
    en = rd('end') if args.dir else None
    try:
        if st and en:
            ctx['elapsed'] = int(en) - int(st)
        elif st:
            ctx['elapsed'] = None
    except Exception:
        ctx['elapsed'] = None
    a = None
    meta = {'tool': 'cysdwin-digest.py', 'version': VERSION, 'ref': args.ref, 'filter': args.filter, 'skip': args.skip, 'rc': rc, 'start': st, 'end': en,
            'outcomes': args.outcome, 'log_path': log_path}
    try:
        if log_path and os.path.isfile(log_path):
            size = os.path.getsize(log_path)
            meta['log_bytes'] = size
            raw_dest = os.path.join(out, 'cysdwin-cargo.log')
            if size == 0:
                ctx['raw'] = {'note': 'not written (the log is empty)'}
            elif size <= args.raw_max_bytes:
                shutil.copyfile(log_path, raw_dest)
                ctx['raw'] = {'note': '%d bytes, kept whole' % size, 'cut': False}
            else:
                half = max(1, (args.raw_max_bytes - 400) // 2)
                with open(log_path, 'rb') as f:
                    head = f.read(half)
                    f.seek(max(0, size - half))
                    tail = f.read()
                marker = ('\n[... CUT by cysdwin-digest.py: the log has %d bytes, the publisher takes at most 40 MiB per file; kept the first %d and the last %d bytes ...]\n' % (size, len(head), len(tail))).encode('ascii')
                with open(raw_dest, 'wb') as f:
                    f.write(head + marker + tail)
                ctx['raw'] = {'note': 'CUT: the log has %d bytes; kept its first %d and last %d bytes (a marker line sits between them)' % (size, len(head), len(tail)), 'cut': True}
            meta['raw'] = ctx['raw']
            with open(log_path, 'rb') as f:
                text = f.read().decode('utf-8', 'replace')
            a = parse(text)
            all_lines = [l for l in ANSI.sub('', text.replace('\r\n', '\n').replace('\r', '\n')).split('\n') if l.strip()]
            a['tail'] = all_lines[-40:]
            a['tail_all'] = all_lines if not a['result_lines'] else []
        else:
            ctx['raw'] = {'note': '(no log)'}
    except Exception:
        ctx['why'] = (ctx.get('why') or '') + ' | digest exception: ' + traceback.format_exc().replace('\n', ' // ')[:1500]
        a = a if a else None
    # per-failure files + digest
    fdir = os.path.join(out, 'cysdwin-failures')
    recs = a['recs'] if a else []
    try:
        if recs:
            os.makedirs(fdir, exist_ok=True)
        width = max(3, len(str(len(recs))))
        for r in recs:
            fname = '%s-%s.txt' % (str(r['seq']).zfill(width), safe_name(r['name']))
            r['file'] = 'cysdwin-failures/' + fname
            head = [
                'test        : %s' % r['name'],
                'sequence    : %d of %d' % (r['seq'], len(recs)),
                'module      : %s   (full path: %s)' % (r['module'], r['modpath']),
                'text source : %s' % r['source'],
                'panic       : %s%s' % (r['loc'] or '(no panic line found)', ("   thread '%s'" % r['thread']) if r['thread'] else ''),
                'panics      : %d in this part of the log' % len(r['panics']),
                'type        : %s   (any match: %s)' % (r['primary'], ', '.join(r['anys']) or '-'),
                'message     :',
                cap_bytes(r['msg'], 4000),
                '-' * 70,
            ]
            htxt = '\n'.join(head) + '\n'
            room = max(1024, args.failure_max_bytes - len(htxt.encode('utf-8', 'replace')))
            write_text(os.path.join(fdir, fname), htxt + cap_bytes('\n'.join(r['body']), room) + '\n')
        tsv = ['seq\ttest\tpanic_at\tlocation\tmessage_first_300\tthread\tpanics\tsource_file']
        for r in recs:
            tsv.append('\t'.join([str(r['seq']), r['name'], one_line(r['panic_at'], 300) if r['panic_at'] else '-', r['loc'] or '-', one_line(r['msg'], 300), r['thread'] or '-', str(len(r['panics'])), r.get('file', '-')]))
        if not recs:
            tsv.append('-\t%s\t-\t-\t-\t-\t0\t-' % ('(no failing test found in the log)' if a else '(no cargo log - nothing to digest)'))
        write_text(os.path.join(out, 'cysdwin-digest.tsv'), '\n'.join(tsv) + '\n')
    except Exception:
        ctx['why'] = (ctx.get('why') or '') + ' | writing the failure files failed: ' + traceback.format_exc().replace('\n', ' // ')[:1500]
    summary = build_summary(a, ctx)
    write_text(os.path.join(out, 'cysdwin-summary.txt'), summary)
    try:
        js = {'tool': 'cysdwin-digest.py', 'version': VERSION, 'ref': args.ref, 'rc': rc, 'result_lines': a['result_lines'] if a else [],
              'failed_count': len(recs), 'failed': [r['name'] for r in recs],
              'by_module': dict(collections.Counter(r['module'] for r in recs)),
              'by_location': dict(collections.Counter(r['loc'] for r in recs if r['loc'])),
              'by_type': dict(collections.Counter(r['primary'] for r in recs)),
              'last_started': a['last_started'] if a else '', 'last_started_ended': a['last_started_ended'] if a else False}
        write_text(os.path.join(out, 'cysdwin-digest.json'), json.dumps(js, indent=1, sort_keys=True) + '\n')
        meta['failed_count'] = len(recs)
        write_text(os.path.join(out, 'cysdwin-meta.json'), json.dumps(meta, indent=1, sort_keys=True) + '\n')
    except Exception:
        pass
    if args.annotate:
        try:
            head = summary[:3500]
            sys.stdout.buffer.write(('::notice title=cysd-win summary::%s\n' % esc_annotation(head)).encode('utf-8', 'replace'))
            if recs:
                lines = []
                for r in recs[:40]:
                    lines.append('%s | %s | %s' % (r['name'], r['loc'] or '-', one_line(r['msg'], 140)))
                sys.stdout.buffer.write(('::notice title=cysd-win first failures::%s\n' % esc_annotation('\n'.join(lines)[:3500])).encode('utf-8', 'replace'))
            sys.stdout.buffer.flush()
        except Exception:
            pass
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except BaseException:
        # never fail the job: leave the reason where a reader looks first
        try:
            o = sys.argv[sys.argv.index('--out') + 1] if '--out' in sys.argv else '.'
            os.makedirs(o, exist_ok=True)
            write_text(os.path.join(o, 'cysdwin-summary.txt'), 'cysdwin-digest.py died: ' + traceback.format_exc() + '\n')
        except Exception:
            pass
        sys.exit(0)
