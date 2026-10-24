#!/usr/bin/env python3
"""homepage-switch.py - staging verify and ordered, journaled switch for the homepage release (B-438, 0.14.53).

Why this exists (B-438, measured in the 0.14.52 apply 2026-10-23 00:12-01:03 +0700): the release script uploaded every asset
straight into the public downloads/ folder. Partial files were public under new names for 2,035 s, the version-less set
(app.tar.gz, .sig, pack mirror, latest.json, SHA256SUMS) was overwritten in place and mixed for 886 s, and SUMS was new while the
pages were old for 39 s. The homepage script now uploads into a staging folder outside the docroot, verifies it there, and calls
this file to make the public change: 16 moves in one process, in a fixed order, each journaled and fsynced.

Where it runs: on the homepage server (uploaded into the staging folder by homepage-v<ver>.sh) and, byte-identical, in the local
rig and in scripts/tests/test_homepage_switch.py. stdlib only. Python 3.6 is the floor: no 3.7+ syntax or stdlib API (no f-strings
either, so that an older interpreter reaches the version check below instead of a SyntaxError).

Publication uses only two primitives (BAR B4):
  - new names (the versioned installers, moves into a backup folder): os.link + os.unlink. link never overwrites (EEXIST) and
    never crosses a device (EXDEV), so a name that appeared meanwhile is never clobbered and nothing is copied.
  - replacements (version-less files, SUMS, pages): os.rename. It replaces the name in one step, refuses another device, and a
    reader that already opened the old file keeps reading the old inode to the end.
  `mv` is never used: GNU mv silently copies and then unlinks across devices.

Order rule (DESIGN s6): a file flips only after every file it names or vouches for is already in place.

Exit codes: 0 ok | 2 usage | 3 stopped before any public change | 4 stopped after a journaled prefix of moves |
            5 public state not recognised, or a planned file no longer holds its target bytes, after this run already made a
              public change (nothing moved by this call; read `status`, then `resume` or `rollback`).

Content rule (codex r1 F1): an inode number says which name holds the planned file, not that its bytes are still the planned
ones - an in-place write keeps the inode. Before any public move, every planned file is checked at its CURRENT location (staged
copy, or the public name it already holds): regular file, sha256 == the plan's target, on the filesystem this call verified.

Commands (all take --docroot/--stage etc. explicitly; nothing is read from the environment):
  preflight      read-only guards + moves stale staging folders to trash (G-outside, G-ts, G-dev, G-stale, G-disk)
  need           names that must be (re)uploaded into staging (missing or size/hash differs)
  verify-backup  the step-1 backup is a copy of the live set it will replace (and of the live old SUMS)
  verify-stage   listing, sizes, sha256, SUMS rule, SUMS self-hash, version binding - inside staging
  switch         plan (first call) + guards + the 16 journaled moves; calling it again resumes (idempotent)
  status         read-only: journal summary and the state of every planned move
  move-old       step 5: move cys_<old>_* out of downloads/ (link+unlink)
  rollback       the forward switch with the backup as its source; never writes over a live file
"""
import argparse
import errno
import hashlib
import io
import json
import os
import re
import signal
import sys
import time

if sys.version_info[:2] < (3, 6):
    sys.stderr.write("homepage-switch: python3 >= 3.6 required, found %s\n" % sys.version.split()[0])
    sys.exit(3)

OK, USAGE, STOP_CLEAN, STOP_PARTIAL, STOP_UNKNOWN = 0, 2, 3, 4, 5

SUMS_NAME = "SHA256SUMS.txt"
VERSIONED_SUFFIXES = ("_aarch64.dmg", "_x64.dmg", "_x64-setup.exe", "_x64-setup.exe.sig", "_x64-setup.zip")
VERSIONLESS_ORDER = (
    "cys_aarch64.app.tar.gz", "cys_aarch64.app.tar.gz.sig",
    "cys_x64.app.tar.gz", "cys_x64.app.tar.gz.sig",
    "pack.tar.gz", "pack-manifest.json.minisig", "pack-manifest.json",
    "latest.json",
    SUMS_NAME,
)
# (public path relative to the docroot, flat name inside <stage>/pages/)
PAGES_ORDER = (("downloads/index.html", "downloads-index.html"), ("index.html", "index.html"))
STAGING_PREFIX = "cys-site-staging-"
SUMS_LINE = re.compile(r"^([0-9a-f]{64})  (\S+)$")
DISK_MARGIN = 1.10
# Ignored only while the moves run: an ssh drop (HUP/PIPE) or a stray INT/TERM must not stop the loop half way.
# SIGKILL cannot be ignored - that is the one mixed case the journal + resume/rollback exist for.
LOOP_SIGNALS = ("SIGHUP", "SIGPIPE", "SIGINT", "SIGTERM")

DONE, PENDING_UNLINK, TODO, ALREADY, UNKNOWN = "done", "pending-unlink", "todo", "already", "unknown"
CHANGED = "changed"                         # the planned inode is where the plan expects it, but not the target bytes/filesystem


class Stop(Exception):
    def __init__(self, code, msg):
        Exception.__init__(self, msg)
        self.code = code


# ---------------------------------------------------------------------------------------------------------------- primitives
# The only three calls that change a public name. Kept as module functions so the tests can count and interrupt them.
def _do_link(src, dst):
    os.link(src, dst)


def _do_unlink(path):
    os.unlink(path)


def _do_rename(src, dst):
    os.rename(src, dst)


def _st_dev(path):
    return os.stat(path).st_dev


def _free_bytes(path):
    st = os.statvfs(path)
    return st.f_bavail * st.f_frsize


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(1 << 20)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def _read_text(path):
    # Explicit UTF-8: Python 3.6 under the C locale would read ASCII and turn a non-ASCII latest.json into a false STOP.
    with io.open(path, encoding="utf-8") as f:
        return f.read()


def _lstat(path):
    try:
        return os.lstat(path)
    except OSError as e:
        if e.errno == errno.ENOENT:
            return None
        raise


def _is_reg(st):
    return st is not None and (st.st_mode & 0o170000) == 0o100000


def _real(path):
    return os.path.realpath(path)


def _under(child, parent):
    c, p = _real(child), _real(parent)
    return c == p or c.startswith(p.rstrip(os.sep) + os.sep)


def _fsync_dir(path):
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return False
    try:
        os.fsync(fd)
        return True
    except OSError:
        return False
    finally:
        os.close(fd)


def _write_json_atomic(path, obj):
    """Write a JSON file inside a staging folder (never a public path): tmp + fsync + rename."""
    tmp = path + ".tmp"
    with io.open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, sort_keys=True, indent=1)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, path)
    _fsync_dir(os.path.dirname(path))


def _load_json(path, what):
    try:
        with io.open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise Stop(STOP_CLEAN, "%s unreadable: %s (%s)" % (what, path, e))


def _emit(obj):
    """Print the verdict. After the moves the ssh peer may be gone - a broken pipe must not change the exit code."""
    try:
        sys.stdout.write(json.dumps(obj, sort_keys=True) + "\n")
        sys.stdout.flush()
    except (OSError, ValueError):
        pass


class Journal(object):
    """Append-only JSON lines, flushed and fsynced per record, so a killed process leaves an exact account."""

    def __init__(self, path):
        self.path = path

    def add(self, **rec):
        rec["t_ns"] = int(time.time() * 1e9)
        line = json.dumps(rec, sort_keys=True) + "\n"
        fd = os.open(self.path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
        try:
            os.write(fd, line.encode("utf-8"))
            os.fsync(fd)
        finally:
            os.close(fd)

    def records(self):
        out = []
        try:
            with io.open(self.path, encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line:
                        try:
                            out.append(json.loads(line))
                        except ValueError:
                            out.append({"ev": "unparsable", "raw": line[:200]})
        except OSError:
            pass
        return out


# ---------------------------------------------------------------------------------------------------------------- the asset set
def versioned_names(ver):
    return ["cys_%s%s" % (ver, s) for s in VERSIONED_SUFFIXES]


def asset_names(ver):
    return versioned_names(ver) + list(VERSIONLESS_ORDER)


def sums_verdict(text, names, sums_sha=None, sums_self=None):
    """The SUMS rule as set equality (DESIGN s5): names(SUMS) == assets - {SUMS}, one '<64 hex>  <name>' per line, no
    duplicates; so lines == len(assets) - 1. SUMS never lists itself; its own hash is pinned instead (sums_self)."""
    bad = []
    want = set(n for n in names if n != SUMS_NAME)
    seen = []
    for i, raw in enumerate(text.splitlines(), 1):
        if not raw.strip():
            continue
        m = SUMS_LINE.match(raw)
        if not m:
            bad.append("line %d not '<64 hex>  <name>'" % i)
            continue
        seen.append(m.group(2))
    dup = sorted(set(n for n in seen if seen.count(n) > 1))
    if dup:
        bad.append("duplicate names: %s" % ", ".join(dup))
    got = set(seen)
    if got - want:
        bad.append("names not in the asset set: %s" % ", ".join(sorted(got - want)))
    if want - got:
        bad.append("assets missing from SUMS: %s" % ", ".join(sorted(want - got)))
    if len(seen) != len(names) - 1:
        bad.append("lines %d != assets-1 = %d" % (len(seen), len(names) - 1))
    if sums_self is not None and sums_sha != sums_self:
        bad.append("SUMS sha256 %s != pinned %s" % (sums_sha, sums_self))
    return bad


def sums_table(text):
    out = {}
    for raw in text.splitlines():
        m = SUMS_LINE.match(raw)
        if m:
            out[m.group(2)] = m.group(1)
    return out


def load_expect(path, ts=None):
    ex = _load_json(path, "expect.json")
    for k in ("ts", "new_ver", "old_ver", "sums_self", "assets"):
        if k not in ex:
            raise Stop(STOP_CLEAN, "expect.json has no %r" % k)
    names = [a["name"] for a in ex["assets"]]
    if sorted(names) != sorted(asset_names(ex["new_ver"])) or len(set(names)) != len(names):
        raise Stop(STOP_CLEAN, "expect.json asset set != the %d known assets of %s" % (len(asset_names(ex["new_ver"])),
                                                                                         ex["new_ver"]))
    if ts is not None and ex["ts"] != ts:
        raise Stop(STOP_CLEAN, "G-ts: expect.json ts %s != run ts %s" % (ex["ts"], ts))
    return ex


# ---------------------------------------------------------------------------------------------------------------- guards
def guard_outside(stage, docroot, what="staging"):
    if _under(stage, docroot) or _under(docroot, stage):
        raise Stop(STOP_CLEAN, "G-outside: %s %s overlaps the docroot %s" % (what, stage, docroot))


def guard_ts(stage, ts):
    if not os.path.basename(stage.rstrip(os.sep)).endswith("-" + ts):
        raise Stop(STOP_CLEAN, "G-ts: staging folder name does not end with -%s" % ts)


def guard_dev(paths):
    devs = {}
    for p in paths:
        try:
            devs[p] = _st_dev(p)
        except OSError as e:
            raise Stop(STOP_CLEAN, "G-dev: cannot stat %s (%s)" % (p, e))
    if len(set(devs.values())) != 1:
        raise Stop(STOP_CLEAN, "G-dev: not one filesystem: %s" % ", ".join("%s=%s" % kv for kv in sorted(devs.items())))
    return list(devs.values())[0]


def verify_stage(stage, ex):
    """DESIGN s5 lines 1-3 and 5-6 (line 4, the server's own `sha256sum -c`, is run by the shell script beside this)."""
    d = os.path.join(stage, "downloads")
    bad = []
    names = [a["name"] for a in ex["assets"]]
    try:
        listing = sorted(os.listdir(d))
    except OSError as e:
        raise Stop(STOP_CLEAN, "staging downloads/ unreadable: %s" % e)
    if listing != sorted(names):
        extra, miss = sorted(set(listing) - set(names)), sorted(set(names) - set(listing))
        bad.append("listing: extra %s missing %s" % (extra or "-", miss or "-"))
    shas = {}
    for a in ex["assets"]:
        p = os.path.join(d, a["name"])
        st = _lstat(p)
        if not _is_reg(st):
            bad.append("%s: not a regular file" % a["name"])
            continue
        if st.st_size != a["size"]:
            bad.append("%s: size %d != %d" % (a["name"], st.st_size, a["size"]))
            continue
        shas[a["name"]] = sha256_file(p)
        if shas[a["name"]] != a["sha256"]:
            bad.append("%s: sha256 differs from the local copy" % a["name"])
    sums_path = os.path.join(d, SUMS_NAME)
    if SUMS_NAME in shas:
        text = _read_text(sums_path)
        bad += ["SUMS: " + b for b in sums_verdict(text, names, shas[SUMS_NAME], ex["sums_self"])]
        tab = sums_table(text)
        for n, h in sorted(tab.items()):
            if n in shas and shas[n] != h:
                bad.append("SUMS line for %s does not match the staged bytes" % n)
    bad += binding(d, ex["new_ver"], shas.get("pack.tar.gz"))
    return bad


def binding(d, ver, pack_sha):
    """Today's verify-8 pre-check, on the staged copies: latest.json -> /v<ver>/, pack-manifest.digest == pack.tar.gz."""
    bad = []
    try:
        lat = json.loads(_read_text(os.path.join(d, "latest.json")))
    except (OSError, ValueError):
        return ["latest.json unreadable"]
    if lat.get("version") != ver:
        bad.append("latest.json version %r != %s" % (lat.get("version"), ver))
    plats = lat.get("platforms") or {}
    if not plats:
        bad.append("latest.json has no platforms")
    for k in sorted(plats):
        if ("/v%s/" % ver) not in ((plats[k] or {}).get("url") or ""):
            bad.append("latest.json platforms.%s url is not /v%s/" % (k, ver))
    try:
        man = json.loads(_read_text(os.path.join(d, "pack-manifest.json")))
    except (OSError, ValueError):
        return bad + ["pack-manifest.json unreadable"]
    if pack_sha is None or man.get("digest") != pack_sha:
        bad.append("pack-manifest.digest != sha256(pack.tar.gz)")
    return bad


# ---------------------------------------------------------------------------------------------------------------- plan + engine
def forward_items(stage, ex):
    """The DESIGN s6 table: 5 versioned (create), 9 version-less in order (replace), 2 pages (replace)."""
    items = []
    for n in versioned_names(ex["new_ver"]):
        items.append({"dst": "downloads/" + n, "src": os.path.join(stage, "downloads", n), "kind": "create"})
    for n in VERSIONLESS_ORDER:
        items.append({"dst": "downloads/" + n, "src": os.path.join(stage, "downloads", n), "kind": "replace"})
    for dst, flat in PAGES_ORDER:
        items.append({"dst": dst, "src": os.path.join(stage, "pages", flat), "kind": "replace"})
    for i, it in enumerate(items, 1):
        it["seq"] = i
    return items


def _holds_sha(path, st, want, dev):
    """The content rule at one location: regular file, on the filesystem this call verified, sha256 == want."""
    return _is_reg(st) and (dev is None or st.st_dev == dev) and sha256_file(path) == want


def _holds_target(path, st, it, dev):
    return _holds_sha(path, st, it["target_sha"], dev)


def _published(it, docroot):
    """Structural: the planned inode holds the public name, i.e. this run already changed that name (whatever its bytes)."""
    st = _lstat(os.path.join(docroot, it["dst"]))
    return _is_reg(st) and st.st_ino == it["src_ino"]


def classify(it, docroot, allow_already=False, dev=None):
    dst, src = os.path.join(docroot, it["dst"]), it["src"]
    dst_st, src_st = _lstat(dst), _lstat(src)
    if dst_st is not None and dst_st.st_ino == it["src_ino"] and _is_reg(dst_st):
        if src_st is None:
            return DONE if _holds_target(dst, dst_st, it, dev) else CHANGED
        if src_st.st_ino == it["src_ino"]:
            # killed between link and unlink: both names are the same inode
            return PENDING_UNLINK if _holds_target(dst, dst_st, it, dev) else CHANGED
    if _is_reg(src_st) and src_st.st_ino == it["src_ino"]:
        if (it["kind"] == "create" and dst_st is None) or \
                (it["kind"] == "replace" and _is_reg(dst_st) and sha256_file(dst) == it["before_sha"]):
            return TODO if _holds_target(src, src_st, it, dev) else CHANGED
    if allow_already and _holds_target(dst, dst_st, it, dev):
        return ALREADY
    return UNKNOWN


def _ignore_loop_signals():
    saved = {}
    for name in LOOP_SIGNALS:
        sig = getattr(signal, name, None)
        if sig is not None:
            saved[sig] = signal.signal(sig, signal.SIG_IGN)
    return saved


def _restore_signals(saved):
    for sig, old in saved.items():
        signal.signal(sig, old)


def check_plan(items, docroot, allow_already=False, label="switch", dev=None):
    """Read-only: classify every planned move and refuse unknown, changed or out-of-order states. Returns (states, ours).
    execute() runs it right before its loop; rollback also runs it before its first public move (codex r2 R2-2)."""
    states = [classify(it, docroot, allow_already, dev) for it in items]
    ours = [_published(it, docroot) for it in items]   # a published file changed in place later is still this run's change
    unknown = [it["dst"] for it, s in zip(items, states) if s == UNKNOWN]
    # where the bytes were checked: the public name when the planned inode holds it, else the staged (or rollback) copy
    changed = ["%s at %s" % (it["dst"], os.path.join(docroot, it["dst"]) if o else it["src"])
               for it, s, o in zip(items, states, ours) if s == CHANGED]
    if unknown or changed:
        why = []
        if unknown:
            why.append("public state not recognised for %s" % unknown)
        if changed:
            why.append("the planned file no longer holds its target sha256 (or not a regular file on this filesystem) for %s"
                       % changed)
        raise Stop(STOP_UNKNOWN if any(ours) else STOP_CLEAN, "%s: %s - nothing moved" % (label, "; ".join(why)))
    first_todo = next((i for i, s in enumerate(states) if s in (TODO, PENDING_UNLINK)), None)
    if first_todo is not None and any(states[j] == DONE for j in range(first_todo + 1, len(states))):
        raise Stop(STOP_UNKNOWN, "%s: moves done out of order - the journaled prefix is broken" % label)
    return states, ours


def execute(items, docroot, journal, allow_already=False, label="switch", dev=None):
    """Run the planned moves in order. Idempotent: a second call after a kill finishes what is left (resume)."""
    states, ours = check_plan(items, docroot, allow_already, label, dev)
    first_todo = next((i for i, s in enumerate(states) if s in (TODO, PENDING_UNLINK)), None)
    if first_todo is None:
        return {"moved": 0, "states": states}
    moved_before = any(ours)
    saved = _ignore_loop_signals()
    moved = 0
    try:
        journal.add(ev="begin", label=label, todo=[items[i]["dst"] for i, s in enumerate(states) if s in (TODO, PENDING_UNLINK)])
        for it, st in zip(items, states):
            if st in (DONE, ALREADY):
                continue
            dst = os.path.join(docroot, it["dst"])
            op = "unlink-src" if st == PENDING_UNLINK else ("link+unlink" if it["kind"] == "create" else "rename")
            journal.add(ev="move", phase="before", seq=it["seq"], dst=it["dst"], op=op, src_ino=it["src_ino"])
            try:
                if st == PENDING_UNLINK:
                    _do_unlink(it["src"])
                elif it["kind"] == "create":
                    _do_link(it["src"], dst)
                    _do_unlink(it["src"])
                else:
                    _do_rename(it["src"], dst)
            except OSError as e:
                journal.add(ev="fail", seq=it["seq"], dst=it["dst"], errno=errno.errorcode.get(e.errno, str(e.errno)))
                code = STOP_PARTIAL if (moved or moved_before) else STOP_CLEAN
                raise Stop(code, "%s: move %d %s failed: %s" % (label, it["seq"], it["dst"], e))
            dfs = _fsync_dir(os.path.dirname(dst))
            got = _lstat(dst)
            if got is None or got.st_ino != it["src_ino"]:
                journal.add(ev="fail", seq=it["seq"], dst=it["dst"], why="G-ino")
                raise Stop(STOP_PARTIAL, "G-ino: %s inode %s != planned %s" % (it["dst"], got and got.st_ino, it["src_ino"]))
            moved += 1
            journal.add(ev="move", phase="after", seq=it["seq"], dst=it["dst"], dst_ino=got.st_ino, dir_fsync=dfs)
        journal.add(ev="end", label=label, moved=moved)
    finally:
        _restore_signals(saved)
    final = [classify(it, docroot, allow_already, dev) for it in items]
    if any(s not in (DONE, ALREADY) for s in final):
        raise Stop(STOP_PARTIAL, "%s: after the loop not every move is done: %s" % (label, final))
    return {"moved": moved, "states": final}


def build_forward_plan(stage, docroot, backup, ex, pages):
    bad = verify_stage(stage, ex)
    if bad:
        raise Stop(STOP_CLEAN, "staging verify: " + "; ".join(bad))
    items = forward_items(stage, ex)
    want = dict((a["name"], a["sha256"]) for a in ex["assets"])
    for it in items:
        src_st = _lstat(it["src"])
        if not _is_reg(src_st):
            raise Stop(STOP_CLEAN, "staged %s missing" % it["src"])
        it["src_ino"] = src_st.st_ino
        live = os.path.join(docroot, it["dst"])
        if it["kind"] == "create":
            if _lstat(live) is not None:
                raise Stop(STOP_CLEAN, "G-versioned-new: %s already exists in public" % it["dst"])
            it["before_sha"], it["target_sha"] = None, want[os.path.basename(it["dst"])]
            continue
        if it["dst"].startswith("downloads/") and it["dst"] != "downloads/index.html":
            name = os.path.basename(it["dst"])
            bk = os.path.join(backup, "downloads", name)
            it["target_sha"] = want[name]
        else:
            page = pages.get(it["dst"]) or {}
            bk = os.path.join(backup, it["dst"])
            it["target_sha"] = page.get("want")
            if not page.get("want") or not page.get("pre"):
                raise Stop(STOP_CLEAN, "pages.json has no pre/want for %s" % it["dst"])
            if sha256_file(it["src"]) != page["want"]:
                raise Stop(STOP_CLEAN, "staged page %s != the locally built page" % it["dst"])
        if not _is_reg(_lstat(live)) or not _is_reg(_lstat(bk)):
            raise Stop(STOP_CLEAN, "G-live-old: live or backup copy of %s missing" % it["dst"])
        live_sha = sha256_file(live)
        if live_sha != sha256_file(bk):
            raise Stop(STOP_CLEAN, "G-live-old: live %s differs from its backup copy" % it["dst"])
        if it["dst"] in pages and live_sha != pages[it["dst"]]["pre"]:
            raise Stop(STOP_CLEAN, "G-pages: live %s changed since the page was built" % it["dst"])
        it["before_sha"] = live_sha
    return items


# ---------------------------------------------------------------------------------------------------------------- commands
def cmd_preflight(a):
    stage, docroot = a.stage, a.docroot
    ex = load_expect(os.path.join(stage, "expect.json"), a.ts)
    guard_outside(stage, docroot)
    guard_ts(stage, a.ts)
    dev = guard_dev([os.path.join(stage, "downloads"), os.path.join(stage, "pages"), docroot,
                     os.path.join(docroot, "downloads")])
    home = os.path.dirname(_real(stage))
    moved = []
    trash = os.path.join(home, "cys-site-trash-%s" % a.ts, "stale-staging")
    stale = [n for n in sorted(os.listdir(home)) if n.startswith(STAGING_PREFIX) and not n.endswith("-" + a.ts)
             and os.path.isdir(os.path.join(home, n))]
    # G-stale part 1: a folder whose switch or rollback loop began and never ended holds the only journal of a public state
    # that may be mixed - it is needed for --status/--resume/--rollback of ITS run. Check all before moving any.
    open_runs = [n for n in stale if _loop_unfinished(os.path.join(home, n))]
    if open_runs:
        raise Stop(STOP_CLEAN, "G-stale: %s holds a switch or rollback that did not finish - run --status, then --resume or "
                   "--rollback with that folder's ts first" % ", ".join(open_runs))
    todo = []
    for name in stale:                                 # every folder is checked before the first move (R3-FIX)
        p = os.path.join(home, name)
        guard_outside(p, docroot, "stale staging folder")   # a folder that holds the docroot is never moved
        if not os.path.isdir(trash):
            os.makedirs(trash)
        guard_dev([p, trash])
        target, k = os.path.join(trash, name), 0
        while os.path.lexists(target):                 # a move into trash never lands on an existing name
            k += 1
            target = os.path.join(trash, "%s.%d" % (name, k))
        todo.append((p, name, target))
    for p, name, target in todo:
        try:
            _do_rename(p, target)                      # G-stale: moved, never deleted, never reused
        except OSError as e:
            raise Stop(STOP_CLEAN, "G-stale: could not move %s to trash: %s" % (name, e))
        moved.append(os.path.basename(target))
    need = sum(x["size"] for x in ex["assets"] if not _staged_ok(stage, x, hash_it=False))
    free = _free_bytes(stage)
    if free < need * DISK_MARGIN:
        raise Stop(STOP_CLEAN, "G-disk: free %d B < %d B needed (+10%%)" % (free, need))
    return {"ok": True, "python": sys.version.split()[0], "dev": dev, "stale_moved": moved, "free": free, "need": need}


def _loop_unfinished(folder):
    """True when the folder planned moves and its journal's last 'begin' has no 'end' after it."""
    if not os.path.exists(os.path.join(folder, "plan.json")):
        return False
    open_loop = False
    for r in Journal(os.path.join(folder, "journal.jsonl")).records():
        if r.get("ev") == "begin":
            open_loop = True
        elif r.get("ev") == "end":
            open_loop = False
    return open_loop


def _staged_ok(stage, a, hash_it=True):
    p = os.path.join(stage, "downloads", a["name"])
    st = _lstat(p)
    if not _is_reg(st) or st.st_size != a["size"]:
        return False
    return (not hash_it) or sha256_file(p) == a["sha256"]


def cmd_need(a):
    ex = load_expect(os.path.join(a.stage, "expect.json"))
    names = [x["name"] for x in ex["assets"] if not _staged_ok(a.stage, x)]
    for n in names:
        sys.stdout.write(n + "\n")
    return None


def cmd_verify_backup(a):
    bk, docroot = a.backup, a.docroot
    bad = []
    live_sums = os.path.join(docroot, "downloads", SUMS_NAME)
    bk_sums = os.path.join(bk, "downloads", SUMS_NAME)
    try:
        old_tab = sums_table(_read_text(bk_sums))
        if sha256_file(bk_sums) != sha256_file(live_sums):
            bad.append("backup SUMS != live SUMS")
    except OSError as e:
        raise Stop(STOP_CLEAN, "backup SUMS unreadable: %s" % e)
    for n in VERSIONLESS_ORDER:
        if n == SUMS_NAME:
            continue
        p, live = os.path.join(bk, "downloads", n), os.path.join(docroot, "downloads", n)
        if not os.path.isfile(p) or not os.path.isfile(live):
            bad.append("%s: backup or live copy missing" % n)
            continue
        h = sha256_file(p)
        if n not in old_tab:
            bad.append("%s: no line in the live old SUMS - cannot verify the copy" % n)
        elif h != old_tab[n]:
            bad.append("%s: backup copy != the old SUMS line" % n)
        if h != sha256_file(live):
            bad.append("%s: backup copy != live" % n)
    for rel, _flat in PAGES_ORDER:
        p, live = os.path.join(bk, rel), os.path.join(docroot, rel)
        if not os.path.isfile(p) or sha256_file(p) != sha256_file(live):
            bad.append("%s: backup copy != live" % rel)
    if bad:
        raise Stop(STOP_CLEAN, "verify-backup: " + "; ".join(bad))
    return {"ok": True, "checked": len(VERSIONLESS_ORDER) + len(PAGES_ORDER)}


def cmd_verify_stage(a):
    ex = load_expect(os.path.join(a.stage, "expect.json"))
    bad = verify_stage(a.stage, ex)
    if bad:
        raise Stop(STOP_CLEAN, "verify-stage: " + "; ".join(bad))
    return {"ok": True, "assets": len(ex["assets"]), "sums_lines": len(ex["assets"]) - 1}


def cmd_switch(a):
    stage, docroot = a.stage, a.docroot
    ex = load_expect(os.path.join(stage, "expect.json"), a.ts)
    guard_ts(stage, a.ts)
    guard_outside(stage, docroot)
    dev = guard_dev([os.path.join(stage, "downloads"), os.path.join(stage, "pages"), docroot,
                     os.path.join(docroot, "downloads")])
    plan_path = os.path.join(stage, "plan.json")
    if os.path.exists(plan_path):
        plan = _load_json(plan_path, "plan.json")
        if plan.get("ts") != a.ts:
            raise Stop(STOP_CLEAN, "G-ts: plan.json ts %s != run ts %s" % (plan.get("ts"), a.ts))
        items = plan["items"]
    else:
        pages = _load_json(os.path.join(stage, "pages.json"), "pages.json")
        items = build_forward_plan(stage, docroot, a.backup, ex, pages)
        _write_json_atomic(plan_path, {"ts": a.ts, "items": items})
    r = execute(items, docroot, Journal(os.path.join(stage, "journal.jsonl")), label="switch", dev=dev)
    return {"ok": True, "moved": r["moved"], "items": len(items)}


def cmd_status(a):
    j = Journal(os.path.join(a.stage, "journal.jsonl")).records()
    out = {"journal": len(j), "begin": sum(1 for r in j if r.get("ev") == "begin"),
           "end": sum(1 for r in j if r.get("ev") == "end"), "fail": [r for r in j if r.get("ev") == "fail"]}
    begins = [r["t_ns"] for r in j if r.get("ev") == "begin"]
    ends = [r["t_ns"] for r in j if r.get("ev") == "end"]
    if begins and ends and ends[-1] >= begins[-1]:
        out["loop_ns"] = ends[-1] - begins[-1]            # the last switch/rollback loop (move-old records come later)
    plan_path = os.path.join(a.stage, "plan.json")
    out["plan"] = os.path.exists(plan_path)               # the template's --resume reads this to skip steps 2-4
    if os.path.exists(plan_path) and a.docroot:
        items = _load_json(plan_path, "plan.json")["items"]
        try:
            dev = _st_dev(os.path.join(a.docroot, "downloads"))
        except OSError:
            dev = None
        out["states"] = dict((it["dst"], classify(it, a.docroot, a.allow_already, dev)) for it in items)
    return out


def check_pairs(pairs, label, code=STOP_CLEAN):
    """Read-only: every pair is done (only dst), pending (src regular, dst absent) or half done (dst = the src inode)."""
    for src, dst in pairs:
        s, d = _lstat(src), _lstat(dst)
        if s is None and _is_reg(d):
            continue
        if not _is_reg(s):
            raise Stop(code, "%s: %s is not a regular file - nothing moved" % (label, src))
        if d is not None and d.st_ino != s.st_ino:
            raise Stop(code, "%s: %s already exists with other bytes - nothing moved" % (label, dst))


def move_files(pairs, journal, label, prior=0):
    """Create-moves (link+unlink) of whole files to new names; used by move-old and by rollback steps 1 and 4. Every pair is
    checked before the first move (R3-FIX: no move before its check); `prior` = public moves this call already made."""
    check_pairs(pairs, label, STOP_PARTIAL if prior else STOP_CLEAN)
    moved = 0
    for src, dst in pairs:
        s, d = _lstat(src), _lstat(dst)
        if s is None and _is_reg(d):
            continue                                   # done earlier
        if not _is_reg(s):
            raise Stop(STOP_PARTIAL if (moved or prior) else STOP_CLEAN, "%s: %s is not a regular file" % (label, src))
        if d is not None and d.st_ino != s.st_ino:
            raise Stop(STOP_PARTIAL if (moved or prior) else STOP_CLEAN, "%s: %s already exists with other bytes" % (label, dst))
        journal.add(ev="move", phase="before", label=label, src=src, dst=dst, op="link+unlink")
        try:
            if d is None:
                _do_link(src, dst)
            _do_unlink(src)
        except OSError as e:
            journal.add(ev="fail", label=label, dst=dst, errno=errno.errorcode.get(e.errno, str(e.errno)))
            raise Stop(STOP_PARTIAL if (moved or prior) else STOP_CLEAN, "%s: %s -> %s failed: %s" % (label, src, dst, e))
        _fsync_dir(os.path.dirname(dst))
        moved += 1
        journal.add(ev="move", phase="after", label=label, dst=dst)
    return moved


def _versioned_in(folder, ver):
    try:
        names = sorted(os.listdir(folder))
    except OSError:
        return []
    pre = "cys_%s_" % ver
    return [n for n in names if n.startswith(pre) and _is_reg(_lstat(os.path.join(folder, n)))]


def cmd_move_old(a):
    dl = os.path.join(a.docroot, "downloads")
    journal = Journal(a.journal)
    pairs, vers = [], []
    for spec in a.move:                                # every folder and pair is checked before the first move (R3-FIX)
        ver, _, dest = spec.partition("=")
        if not ver or not dest:
            raise Stop(USAGE, "--move wants VER=DIR")
        guard_outside(dest, a.docroot, "old-assets folder")       # moved out of public means out of the docroot
        if not os.path.isdir(dest):
            os.makedirs(dest)
        guard_dev([dl, dest])
        vers.append(ver)
        pairs += [(os.path.join(dl, n), os.path.join(dest, n)) for n in _versioned_in(dl, ver)]
    total = move_files(pairs, journal, "move-old")
    for ver in vers:
        left = _versioned_in(dl, ver)
        if left:
            raise Stop(STOP_PARTIAL, "move-old: %d cys_%s_* files remain" % (len(left), ver))
    return {"ok": True, "moved": total}


def _copy_into_stage(src, dst, stage_root):
    """Rollback staging only: copy a backup file INTO the rollback staging folder (never a public path), keeping mode and
    mtime like cp -p. The public change is still a rename from that copy, so the backup itself survives a second attempt."""
    if not _under(dst, stage_root):
        raise Stop(STOP_CLEAN, "copy target %s is outside the rollback staging folder" % dst)
    tmp = dst + ".part"
    with open(src, "rb") as f, open(tmp, "wb") as g:
        while True:
            b = f.read(1 << 20)
            if not b:
                break
            g.write(b)
        g.flush()
        os.fsync(g.fileno())
    st = os.stat(src)
    os.chmod(tmp, st.st_mode & 0o7777)
    os.utime(tmp, (st.st_atime, st.st_mtime))
    os.rename(tmp, dst)


def cmd_rollback(a):
    docroot, bk, rb = a.docroot, a.backup, a.rb_stage
    ex = load_expect(os.path.join(a.stage, "expect.json"), a.ts)
    pages = _load_json(os.path.join(a.stage, "pages.json"), "pages.json")
    guard_ts(rb, a.ts)
    guard_outside(rb, docroot)
    guard_outside(bk, docroot, "backup")
    guard_outside(a.trash, docroot, "trash")                     # checked before the makedirs below writes anything
    for sub in ("downloads", "pages"):
        p = os.path.join(rb, sub)
        if not os.path.isdir(p):
            os.makedirs(p)
    if not os.path.isdir(a.trash):
        os.makedirs(a.trash)
    dev = guard_dev([os.path.join(rb, "downloads"), os.path.join(rb, "pages"), docroot, os.path.join(docroot, "downloads"),
                     bk, a.trash])
    journal = Journal(os.path.join(rb, "journal.jsonl"))
    dl = os.path.join(docroot, "downloads")
    # R3-FIX (codex r2 R2-2): every check of this call runs before its first public move, on every retry path - the rollback
    # plan (ts, staged copies vs the backup and the old SUMS), the current bytes of every planned item, the old installers
    # at their current location, the new names' way out. The publication order stays: old installers, then the version-less
    # set + SUMS + pages, then the new names out.
    items = _rollback_plan(a, ex, pages, bk, rb)                         # step 2 (staging writes only, never public)
    check_plan(items, docroot, allow_already=True, label="rollback", dev=dev)
    back_pairs = _old_back_pairs(a, ex, bk, dl, docroot, dev)
    out_dir = os.path.join(a.trash, "rolled-back-new-assets")
    if not os.path.isdir(out_dir):
        os.makedirs(out_dir)
    out_pairs = [(os.path.join(dl, n), os.path.join(out_dir, n)) for n in _versioned_in(dl, ex["new_ver"])]
    check_pairs(back_pairs, "rollback-old-back")
    check_pairs(out_pairs, "rollback-new-out")
    # 1. old versioned files back first (the old pages that step 3 restores link them)
    back = move_files(back_pairs, journal, "rollback-old-back")
    # 3. the version-less set, SUMS and the pages back to the old bytes, same order rule as forward
    r = execute(items, docroot, journal, allow_already=True, label="rollback", dev=dev)
    # 4. the new versioned names out (no live page links them any more)
    out = move_files(out_pairs, journal, "rollback-new-out", prior=back + r["moved"])
    return {"ok": True, "old_back": back, "moved": r["moved"], "new_out": out}


def _rollback_plan(a, ex, pages, bk, rb):
    """Rollback step 2: load the plan of an earlier attempt (same ts) or stage verified copies of the backup and plan them."""
    plan_path = os.path.join(rb, "plan.json")
    if os.path.exists(plan_path):
        plan = _load_json(plan_path, "rollback plan.json")
        if plan.get("ts") != a.ts:
            raise Stop(STOP_CLEAN, "G-ts: rollback plan ts %s != %s" % (plan.get("ts"), a.ts))
        return plan["items"]
    old_tab = _old_sums(bk)
    new = dict((x["name"], x["sha256"]) for x in ex["assets"])
    items = []
    for n in VERSIONLESS_ORDER:
        items.append({"dst": "downloads/" + n, "bk": os.path.join(bk, "downloads", n),
                      "src": os.path.join(rb, "downloads", n), "before_sha": new[n]})
    for rel, flat in PAGES_ORDER:
        items.append({"dst": rel, "bk": os.path.join(bk, rel), "src": os.path.join(rb, "pages", flat),
                      "before_sha": pages[rel]["want"]})
    for i, it in enumerate(items, 1):
        it["seq"], it["kind"] = i, "replace"
        if not os.path.isfile(it["src"]) or sha256_file(it["src"]) != sha256_file(it["bk"]):
            _copy_into_stage(it["bk"], it["src"], rb)
        it["target_sha"] = sha256_file(it["src"])
        if it["target_sha"] != sha256_file(it["bk"]):
            raise Stop(STOP_CLEAN, "rollback copy of %s != backup" % it["dst"])
        name = os.path.basename(it["dst"])
        if it["dst"].startswith("downloads/") and name not in (SUMS_NAME, "index.html") and old_tab.get(name) != it["target_sha"]:
            raise Stop(STOP_CLEAN, "backup %s != the old SUMS line" % name)
        if it["dst"] in pages and it["target_sha"] != pages[it["dst"]]["pre"]:
            raise Stop(STOP_CLEAN, "backup page %s != the page the run replaced" % it["dst"])
        it["src_ino"] = os.lstat(it["src"]).st_ino
    _write_json_atomic(plan_path, {"ts": a.ts, "items": items})
    return items


def _old_sums(bk):
    try:
        return sums_table(_read_text(os.path.join(bk, "downloads", SUMS_NAME)))
    except OSError as e:
        raise Stop(STOP_CLEAN, "rollback: backup SUMS unreadable: %s" % e)


def _old_back_pairs(a, ex, bk, dl, docroot, dev):
    """Rollback step 1, checked in full (codex r2 R2-1): the old version's installer set comes from the old SUMS (the backup
    copy), and every one of them is checked where it is NOW - still in the old-assets folder, already public, or both after a
    link without its unlink - so an installer an earlier attempt already put back is checked again. Missing, changed,
    conflicting or unlisted installers stop the call before any public move. A stale older version has no SUMS line to check
    against (limit: its files are moved back as found)."""
    pairs, old_tab = [], None
    for spec in a.old_back or []:
        ver, _, src_dir = spec.partition("=")
        if not ver or not src_dir:
            raise Stop(USAGE, "--old-back wants VER=DIR")
        guard_outside(src_dir, docroot, "old-assets folder")
        have_dir = os.path.isdir(src_dir)                # absent: step 5 never ran, the old installers never left public
        if have_dir:
            guard_dev([src_dir, dl])
        if ver != ex["old_ver"]:
            pairs += [(os.path.join(src_dir, n), os.path.join(dl, n)) for n in (_versioned_in(src_dir, ver) if have_dir else [])]
            continue
        if old_tab is None:
            old_tab = _old_sums(bk)
        prefix = "cys_%s_" % ver
        want = dict((n, h) for n, h in old_tab.items() if n.startswith(prefix))
        bad, public = [], False
        for n in sorted(want):
            src, dst = os.path.join(src_dir, n), os.path.join(dl, n)
            s, d = _lstat(src), _lstat(dst)
            if _is_reg(s) and d is None:
                at, st = src, s                          # still to move
                pairs.append((src, dst))
            elif _is_reg(s) and _is_reg(d) and d.st_ino == s.st_ino:
                at, st = dst, d                          # linked, not yet unlinked: the public name is that inode
                pairs.append((src, dst))
            elif s is None and _is_reg(d):
                at, st = dst, d                          # put back earlier, or never moved out
            else:
                bad.append("%s (%s)" % (n, "missing" if s is None and d is None else "conflict at " + dst))
                public = public or d is not None
                continue
            if not _holds_sha(at, st, want[n], dev):
                bad.append("%s at %s" % (n, at))
                public = public or at == dst
        if have_dir:
            bad += ["%s at %s (no line in the old SUMS)" % (n, os.path.join(src_dir, n))
                    for n in _versioned_in(src_dir, ver) if n not in want]
        if bad:
            raise Stop(STOP_UNKNOWN if public else STOP_CLEAN,
                       "rollback-old-back: old installers do not match the old SUMS: %s - nothing moved" % bad)
    return pairs


def parser():
    p = argparse.ArgumentParser(prog="homepage-switch.py")
    sub = p.add_subparsers(dest="cmd")
    s = sub.add_parser("preflight")
    s.add_argument("--stage", required=True)
    s.add_argument("--docroot", required=True)
    s.add_argument("--ts", required=True)
    s = sub.add_parser("need")
    s.add_argument("--stage", required=True)
    s = sub.add_parser("verify-backup")
    s.add_argument("--backup", required=True)
    s.add_argument("--docroot", required=True)
    s = sub.add_parser("verify-stage")
    s.add_argument("--stage", required=True)
    s = sub.add_parser("switch")
    s.add_argument("--stage", required=True)
    s.add_argument("--docroot", required=True)
    s.add_argument("--ts", required=True)
    s.add_argument("--backup", required=True)
    s = sub.add_parser("status")
    s.add_argument("--stage", required=True)
    s.add_argument("--docroot")
    s.add_argument("--allow-already", action="store_true")
    s = sub.add_parser("move-old")
    s.add_argument("--docroot", required=True)
    s.add_argument("--journal", required=True)
    s.add_argument("--move", action="append", required=True, metavar="VER=DIR")
    s = sub.add_parser("rollback")
    s.add_argument("--docroot", required=True)
    s.add_argument("--backup", required=True)
    s.add_argument("--stage", required=True)
    s.add_argument("--rb-stage", required=True)
    s.add_argument("--trash", required=True)
    s.add_argument("--ts", required=True)
    s.add_argument("--old-back", action="append", metavar="VER=DIR")
    return p


COMMANDS = {"preflight": cmd_preflight, "need": cmd_need, "verify-backup": cmd_verify_backup,
            "verify-stage": cmd_verify_stage, "switch": cmd_switch, "status": cmd_status, "move-old": cmd_move_old,
            "rollback": cmd_rollback}


def main(argv=None):
    p = parser()
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return USAGE if e.code else OK
    if not a.cmd:
        p.print_usage(sys.stderr)
        return USAGE
    try:
        out = COMMANDS[a.cmd](a)
    except Stop as e:
        _emit({"ok": False, "cmd": a.cmd, "code": e.code, "why": str(e)})
        return e.code
    if out is not None:
        _emit(out)
    return OK


if __name__ == "__main__":
    sys.exit(main())
