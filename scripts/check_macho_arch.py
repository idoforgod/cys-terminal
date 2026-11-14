#!/usr/bin/env python3
"""check_macho_arch.py — every Mach-O file of a macOS bundle must carry the target architecture
(0.15.0 · tracks T11, T11b · RULINGS BQ1(b), CA1–CA2, DB4–DB5 · owner order addendum 11).

Why: the Intel (x86_64) macOS leg is a CROSS build on an arm64 runner, the runtime tree
(`src-tauri/runtime`: python, node, git, uv) is downloaded per target, and nothing checked the
architecture of what ended up inside `cys.app` — a wrong-architecture runtime (for example an
arm64-only `runtime/python/bin/python3` in the Intel installer) would have been signed, notarised and
shipped without any red light. This gate reads the Mach-O / fat (universal) headers of every file and
fails, naming each file, when one cannot run on the target:
  · thin Mach-O of the target architecture            → pass
  · thin Mach-O of another architecture               → FAIL
  · universal (fat) with a slice of the target         → pass
  · universal (fat) without one                         → FAIL
  · universal whose fat table and a slice's OWN header disagree (CPU or subtype) → FAIL (RULINGS DB4: the
    loader picks a slice by the table and then loads what the slice holds; every slice header is read, in
    file order, and its header and load commands must lie within the slice's own bounds)
  · not Mach-O (text, data, a Java class file that also starts with 0xCAFEBABE) → ignored
  · a Mach-O whose header is cut short, a fat slice that lies outside the file or overlaps another, a header,
    load commands or one load command that run past the slice / file / sizeofcmds, an unreadable file or
    directory → CANNOT JUDGE (exit 2 — not measurable is not a pass)

SUBTYPE RULE (RULINGS CA2(a)): the target is satisfied only by a slice every Mac of that kind can run —
  x86_64 target ← cputype x86_64 with subtype ALL (3). `x86_64h` (subtype 8) runs only on Haswell-or-newer
                  Intel CPUs, and macOS 10.15 still supports older ones → alone it does not satisfy.
  arm64 target  ← cputype arm64 with subtype ALL (0) or V8 (1). `arm64e` (subtype 2) is the pointer-
                  authentication ABI that macOS loads only for Apple's own code — a third-party arm64e
                  executable is refused without a boot argument → alone it does not satisfy.
  The capability bits (the top byte of the subtype, e.g. 0x80000000) are masked off before comparing.

It also prints the count per architecture and the minimum macOS of the entry points
(`LC_BUILD_VERSION` / `LC_VERSION_MIN_MACOSX`), and for an `.app` its Info.plist
`LSMinimumSystemVersion` with an informational WARN when an entry point needs a newer macOS
(RULINGS BQ1(e): whether that value changes is the owner's decision — the gate does not fail on it).

`--require REL` names an entry point (a path suffix such as `Contents/Resources/runtime/python/bin/python3`)
that MUST exist, resolve (a dangling or looping link fails) and carry the target — every FINAL gate
(the app before notarisation, the updater archive, the app inside the DMG) passes the four runtime entry
points (python, node, git, uv) this way (RULINGS CA2(b)).

A tar archive (`.tar.gz` / `.tgz` / `.tar`, e.g. the updater archive `cys.app.tar.gz`) is read in ONE
streaming pass in archive order — never by seeking back in a compressed stream (RULINGS CA1: sorted-name
reads of the real 0.14.56 archive rewound the gzip ~3,864 times, ~35 minutes per pass). The output line
`archive read in one pass: <members> members, <read> of <size> compressed bytes` is measured from the real
reads (`read` can only exceed `size` if something rewound).
Tar HARD-LINK members are judged as the product's extractor treats them (RULINGS DB5, DI2). The tar archives this
gate reads are the updater archive `cys.app.tar.gz` (release.yml, build-macos-signed.sh); the app unpacks it in
tauri-plugin-updater 2.10.1 (`src/updater.rs:1236–1250`, macOS `install_inner`) by calling, for every member,
`entry.unpack(<file path>)` of the Rust `tar` crate 0.4.46 (Cargo.lock), and on the first error it deletes what it
extracted and abandons the update. `Entry::unpack(dst)` is `unpack(None, dst)` (`tar-0.4.46/src/entry.rs:203–204`);
for a hard link with no target base it takes the stored link name as it is (`:528–545`) — a path relative to the
updater process's working directory, not to the extraction folder — and calls `fs::hard_link` on it (`:548`). So a
hard-link member, whether its target comes before or after it, does not produce the packed file: every hard-link
member FAILs with "the archive cannot be extracted as packed". (The system tar's handling is not documented in the
local man page and is not the product's extractor.)
Symbolic links are followed component by component as the file system does after extraction (RULINGS DI3, DI4):
`.` and empty components are dropped, a component that is itself a symbolic-link member is replaced by its target,
`..` climbs one level (above the root of the tree: refused as an unsafe link); a loop is an actual revisit — a link
whose own resolution needs itself — and a valid chain of any length resolves (each link is resolved once, memoised).
Every load command of every Mach-O is checked (RULINGS DI1): each must start and end inside `sizeofcmds`, whatever
its size; the commands are read one by one (header, and the version field only where needed), forward, so a large
command area is read in bounded steps, never skipped.

Reads bytes only: no `lipo`, `otool`, `file` or `vtool` (they are not on a stock Mac and not needed),
no execution of anything it inspects, no network. Standard library only (python 3.8+).

    python3 scripts/check_macho_arch.py --target x86_64-apple-darwin \\
        --require Contents/Resources/runtime/python/bin/python3 … target/x86_64-apple-darwin/release/bundle/macos/cys.app

Exit: 0 = every Mach-O file has the target · 1 = at least one does not, or a required entry point is
      missing / dangling / not a target Mach-O — each named · 2 = cannot judge (path missing, unreadable
      file / directory / archive, a header cut short, a fat slice out of range, no Mach-O at all, bad
      arguments) — never a pass
"""
import argparse
import os
import plistlib
import posixpath
import struct
import sys
import tarfile
import zlib

EXIT_OK, EXIT_FAIL, EXIT_UNJUDGEABLE = 0, 1, 2

MH_MAGIC, MH_MAGIC_64 = 0xFEEDFACE, 0xFEEDFACF
FAT_MAGIC, FAT_MAGIC_64 = 0xCAFEBABE, 0xCAFEBABF
LC_VERSION_MIN_MACOSX, LC_BUILD_VERSION = 0x24, 0x32
FAT_MAX_ARCHS = 30          # a Java class file reads its version (major >= 45) where nfat_arch would be
CPU_X86_64, CPU_ARM64 = 0x01000007, 0x0100000C
SUBTYPE_MASK = 0x00FFFFFF   # drop the capability bits (CPU_SUBTYPE_MASK 0xff000000)
CPU_NAMES = {0x0200000C: "arm64_32", 7: "i386", 12: "arm", 18: "ppc", 0x01000012: "ppc64"}
TARGETS = {
    "x86_64-apple-darwin": "x86_64", "x86_64": "x86_64", "x64": "x86_64",
    "aarch64-apple-darwin": "arm64", "aarch64": "arm64", "arm64": "arm64",
}
SATISFIES = {"x86_64": ("x86_64",), "arm64": ("arm64", "arm64v8")}
# entry points whose minimum macOS is printed (relative-path suffixes; Contents/MacOS/* is added by rule)
ENTRY_SUFFIXES = ("python/bin/python3", "node/bin/node", "git/bin/git", "uv/uv")
HARD_LINK_WHY = ("the archive cannot be extracted as packed — this member is a tar hard link (to %s); the updater's "
                 "extractor (the Rust tar crate's Entry::unpack) makes it from that name taken relative to its working "
                 "directory, not inside the extraction folder, so it fails and the update is abandoned (RULINGS DI2)")


class Unjudgeable(Exception):
    """The bytes start like Mach-O but cannot be read to the end of what they declare."""


def arch_name(cputype, subtype):
    s = subtype & SUBTYPE_MASK
    if cputype == CPU_X86_64:
        return {3: "x86_64", 8: "x86_64h"}.get(s, "x86_64/sub%d" % s)
    if cputype == CPU_ARM64:
        return {0: "arm64", 1: "arm64v8", 2: "arm64e"}.get(s, "arm64/sub%d" % s)
    return CPU_NAMES.get(cputype, "cpu-0x%08x" % cputype)


def _exact(read, off, n, what):
    b = read(off, n)
    if len(b) < n:
        raise Unjudgeable("%s cut short (needs %d bytes at offset %d)" % (what, n, off))
    return b


def _decode_version(v):
    major, minor, patch = v >> 16, (v >> 8) & 0xFF, v & 0xFF
    return "%d.%d.%d" % (major, minor, patch) if patch else "%d.%d" % (major, minor)


def _thin(read, off, end, where):
    """(cputype, subtype, minimum macOS or None) of the thin Mach-O at OFF whose bytes end at END (the end of the
    file, or of the fat slice — RULINGS DB4; None = unknown); WHERE names that span in messages. Raises
    Unjudgeable when cut short, or when the header, the load commands or one load command run past END /
    sizeofcmds."""
    magic_b = _exact(read, off, 4, "Mach-O magic")           # the file itself ends: "cut short"
    if end is not None and off + 4 > end:
        raise Unjudgeable("Mach-O magic runs past the end of %s" % where)
    le = struct.unpack("<I", magic_b)[0]
    endian = "<" if le in (MH_MAGIC, MH_MAGIC_64) else ">"
    magic = struct.unpack(endian + "I", magic_b)[0]
    if magic not in (MH_MAGIC, MH_MAGIC_64):
        raise Unjudgeable("slice at offset %d does not start with a Mach-O magic" % off)
    hsize = 32 if magic == MH_MAGIC_64 else 28
    hdr = _exact(read, off, hsize, "Mach-O header")
    if end is not None and off + hsize > end:                # the file goes on, the slice does not
        raise Unjudgeable("Mach-O header runs past the end of %s" % where)
    cputype, subtype = struct.unpack(endian + "II", hdr[4:12])
    ncmds, sizeofcmds = struct.unpack(endian + "II", hdr[16:24])
    if end is not None and off + hsize + sizeofcmds > end:
        raise Unjudgeable("load commands (%d bytes) run past the end of %s" % (sizeofcmds, where))
    # RULINGS DI1: EVERY load command is checked (no early stop), read one by one and forward — a command area of
    # any size is read in bounded steps; a command that does not fit is not measurable
    build, legacy, pos, base = None, None, 0, off + hsize
    for k in range(ncmds):
        if pos + 8 > sizeofcmds:
            raise Unjudgeable("load commands end early (command %d of %d starts past sizeofcmds) in %s" % (k, ncmds, where))
        cmd, cmdsize = struct.unpack(endian + "II", _exact(read, base + pos, 8, "load command"))
        if cmdsize < 8:
            raise Unjudgeable("load command of size %d" % cmdsize)
        if pos + cmdsize > sizeofcmds:
            raise Unjudgeable("load command %d (%d bytes) runs past sizeofcmds (%d bytes) in %s"
                              % (k, cmdsize, sizeofcmds, where))
        if cmd == LC_BUILD_VERSION and build is None and cmdsize >= 16:
            build = _decode_version(struct.unpack(endian + "I", _exact(read, base + pos + 12, 4, "LC_BUILD_VERSION"))[0])
        elif cmd == LC_VERSION_MIN_MACOSX and legacy is None and cmdsize >= 12:
            legacy = _decode_version(struct.unpack(endian + "I", _exact(read, base + pos + 8, 4, "LC_VERSION_MIN_MACOSX"))[0])
        pos += cmdsize
    return cputype, subtype, build or legacy


def inspect(read, size, target_arch):
    """→ None for non-Mach-O; else {archs: [name…], ok: bool, minos: str|None} (minos of the slice that
    satisfies the target, else of the first slice). Reads only forward (works on a streaming member)."""
    head = read(0, 8)
    if len(head) < 4:
        return None
    le = struct.unpack("<I", head[:4])[0]
    be = struct.unpack(">I", head[:4])[0]
    if le in (MH_MAGIC, MH_MAGIC_64) or be in (MH_MAGIC, MH_MAGIC_64):
        cputype, subtype, minos = _thin(read, 0, size, "the file")
        name = arch_name(cputype, subtype)
        return {"archs": [name], "ok": name in SATISFIES[target_arch], "minos": minos, "mismatch": []}
    if be not in (FAT_MAGIC, FAT_MAGIC_64):
        return None
    if len(head) < 8:
        raise Unjudgeable("fat header cut short")
    nfat = struct.unpack(">I", head[4:8])[0]
    if nfat == 0 or nfat > FAT_MAX_ARCHS:
        return None              # a Java class file (0xCAFEBABE + version >= 45) or other data — not Mach-O
    entry = 20 if be == FAT_MAGIC else 32
    table = _exact(read, 8, nfat * entry, "fat table")
    entries = []
    for i in range(nfat):
        e = table[i * entry:(i + 1) * entry]
        cputype, subtype = struct.unpack(">II", e[0:8])
        if entry == 20:
            offset, ssize = struct.unpack(">II", e[8:16])
        else:
            offset, ssize = struct.unpack(">QQ", e[8:24])
        if size is not None and (offset < 8 + nfat * entry or offset + ssize > size):
            raise Unjudgeable("fat slice %d (offset %d, size %d) lies outside the file (%d bytes)" % (i, offset, ssize, size))
        entries.append((offset, i, cputype, subtype, ssize))
    # RULINGS DB4: every slice's OWN header, within the slice's own bounds, read in FILE order (a streamed member
    # can only be read forward; the table may list the slices in any order)
    own, mismatch, prev_end = {}, [], 0
    for offset, i, cputype, subtype, ssize in sorted(entries):
        if offset < prev_end:
            raise Unjudgeable("fat slice %d (offset %d) overlaps the slice before it (which ends at %d)" % (i, offset, prev_end))
        prev_end = offset + ssize
        hcpu, hsub, minos = _thin(read, offset, offset + ssize, "fat slice %d" % i)
        same = hcpu == cputype and (hsub & SUBTYPE_MASK) == (subtype & SUBTYPE_MASK)
        if not same:
            mismatch.append("fat table entry %d says %s but the slice's own header is %s"
                            % (i, arch_name(cputype, subtype), arch_name(hcpu, hsub)))
        own[i] = (arch_name(hcpu, hsub), same, minos)
    archs = [own[i][0] for i in range(nfat)]
    good = [own[i][2] for i in range(nfat) if own[i][1] and own[i][0] in SATISFIES[target_arch]]
    minos = good[0] if good else own[0][2]
    return {"archs": archs, "ok": bool(good), "minos": minos, "mismatch": mismatch}


def _vtuple(v):
    try:
        return tuple(int(x) for x in v.split("."))
    except (AttributeError, ValueError):
        return ()


def is_entry(rel):
    parts = rel.split("/")
    for i in range(len(parts) - 2):        # <x>.app/Contents/MacOS/<file> (direct child)
        if parts[i].endswith(".app") and parts[i + 1:i + 3] == ["Contents", "MacOS"] and len(parts) == i + 4:
            return True
    return any(rel == s or rel.endswith("/" + s) for s in ENTRY_SUFFIXES)


# ── sources: one pass over a directory tree or a tar archive → records ─────────────────────────────────

class ForwardReader:
    """read(offset, n) over a stream that can only go forward (a tar member read in streaming mode)."""

    def __init__(self, f):
        self.f, self.base, self.buf = f, 0, b""

    def read(self, off, n):
        if off < self.base:
            raise Unjudgeable("internal: a backward read at %d (stream at %d)" % (off, self.base))
        have_end = self.base + len(self.buf)
        if off > have_end:                       # skip forward without keeping the bytes
            gap = off - have_end
            while gap > 0:
                chunk = self.f.read(min(gap, 1 << 16))
                if not chunk:
                    break
                gap -= len(chunk)
            self.base, self.buf = off - gap, b""
        need = off + n - (self.base + len(self.buf))
        while need > 0:
            chunk = self.f.read(max(need, 1 << 16))
            if not chunk:
                break
            self.buf += chunk
            need -= len(chunk)
        start = off - self.base
        return self.buf[start:start + n]


class CountingFile:
    """A raw file whose bytes read are counted (the measured proof that an archive was read once)."""

    def __init__(self, path):
        self.f = open(path, "rb")
        self.read_bytes = 0

    def read(self, n=-1):
        b = self.f.read(n)
        self.read_bytes += len(b)
        return b

    def close(self):
        self.f.close()


def scan_dir(root, target_arch):
    """→ (records, links, files, problems, hardlinks): records {rel: inspect() result} for Mach-O files; links
    {rel: target}; files = every regular file (Mach-O or not); problems [str] (cannot judge); hardlinks {} (on a
    directory tree a hard link is simply another regular file)."""
    records, links, files, problems = {}, {}, set(), []
    base = os.path.basename(os.path.normpath(root)) if root.rstrip("/").endswith(".app") else ""

    def rel_of(full):
        rel = os.path.relpath(full, root).replace(os.sep, "/")
        return base + "/" + rel if base else rel

    def one(full, rel):
        if os.path.islink(full):
            links[rel] = os.readlink(full)
            return
        if not os.path.isfile(full):
            return
        files.add(rel)
        try:
            size = os.path.getsize(full)
            with open(full, "rb") as f:
                def read(off, n):
                    f.seek(off)
                    return f.read(n)
                info = inspect(read, size, target_arch)
        except Unjudgeable as e:
            problems.append("%s: %s" % (rel, e))
            return
        except OSError as e:
            problems.append("%s: unreadable (%s)" % (rel, e.strerror or e))
            return
        if info is not None:
            records[rel] = info

    if os.path.isfile(root) and not os.path.islink(root):
        one(root, os.path.basename(root))
        return records, links, files, problems, {}

    def onerror(e):
        problems.append("%s: unreadable directory (%s)" % (rel_of(e.filename) if e.filename else root, e.strerror or e))

    for dirpath, dirnames, filenames in os.walk(root, followlinks=False, onerror=onerror):
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            one(full, rel_of(full))
        for d in sorted(dirnames):
            full = os.path.join(dirpath, d)
            if os.path.islink(full):
                links[rel_of(full)] = os.readlink(full)
    return records, links, files, problems, {}


def _arc_rel(name):
    return name[2:] if name.startswith("./") else name


def scan_tar(path, target_arch, out):
    """ONE streaming pass in archive order (CA1). → (records, links, files, problems, hardlinks): hardlinks
    {rel: stored link name} — every one is judged as the updater's extractor treats it (RULINGS DI2)."""
    records, links, files, problems, hardlinks = {}, {}, set(), [], {}
    raw = CountingFile(path)
    members = 0
    try:
        with tarfile.open(fileobj=raw, mode="r|*") as tar:
            for m in tar:
                members += 1
                rel = _arc_rel(m.name)
                if m.islnk():
                    hardlinks[rel] = m.linkname
                    continue
                if m.issym():
                    links[rel] = m.linkname
                    continue
                if not m.isfile():
                    continue
                files.add(rel)
                fobj = tar.extractfile(m)
                try:
                    info = inspect(ForwardReader(fobj).read, m.size, target_arch)
                except Unjudgeable as e:
                    problems.append("%s: %s" % (rel, e))
                    continue
                if info is not None:
                    records[rel] = info
        while raw.read(1 << 16):        # the end-of-archive padding / gzip trailer: count it, still one pass
            pass
    finally:
        raw.close()
    out.append("  archive read in one pass: %d members, %d of %d compressed bytes"
               % (members, raw.read_bytes, os.path.getsize(path)))
    return records, links, files, problems, hardlinks


def _components(path):
    return [c for c in path.split("/") if c not in ("", ".")]


def _realpath(path, links, hardlinks, memo, active):
    """PATH (relative to the root of the tree) resolved component by component as the file system does after
    extraction → (components | None, why). A component that is a symbolic-link member is replaced by its target,
    relative to the link's directory; `..` climbs one level; each link is resolved once (MEMO) and a link met again
    while its own resolution is still under way (ACTIVE) is a real loop (RULINGS DI3, DI4)."""
    done = []
    for c in _components(path):
        if c == "..":
            if not done:
                return None, "unsafe link: it climbs above the root of the tree"
            done.pop()
            continue
        cand = "/".join(done + [c])
        if cand in hardlinks:
            return None, "through %s, which %s" % (cand, HARD_LINK_WHY % hardlinks[cand])
        if cand in links:
            if cand not in memo:
                if cand in active:
                    return None, "link loop"
                target = links[cand]
                if target.startswith("/"):
                    memo[cand] = (None, "absolute link outside the tree")
                else:
                    active.add(cand)
                    memo[cand] = _realpath("/".join(done + [target]), links, hardlinks, memo, active)
                    active.discard(cand)
            res = memo[cand]
            if res[0] is None:
                return res
            done = list(res[0])
            continue
        done.append(c)
    return done, ""


def _resolve(rel, links, records, files, hardlinks=None, memo=None):
    """Follow links inside the tree → (resolved rel | None, why)."""
    hardlinks = hardlinks or {}
    memo = {} if memo is None else memo
    if rel in hardlinks:
        return None, HARD_LINK_WHY % hardlinks[rel]
    try:
        comps, why = _realpath(rel, links, hardlinks, memo, set())
    except RecursionError:
        return None, "cannot judge: the link chain is too deep to follow"
    if comps is None:
        return None, why
    final = "/".join(comps)
    if final in records:
        return final, ""
    if final in files:
        return None, "not a Mach-O file"
    return None, ("missing" if final == "/".join(_components(rel)) else "dangling link")


def check_one(path, target_arch, requires, quiet, out):
    label = path.rstrip("/")
    head = len(out)
    out.append("== %s" % label)
    is_tar = os.path.isfile(path) and (path.endswith((".tar.gz", ".tgz", ".tar")) or tarfile.is_tarfile(path))
    try:
        if is_tar:
            records, links, files, problems, hardlinks = scan_tar(path, target_arch, out)
        else:
            records, links, files, problems, hardlinks = scan_dir(path, target_arch)
    except (tarfile.TarError, OSError, EOFError, zlib.error) as e:
        out.append("CANNOT JUDGE: %s unreadable (%s)" % (path, e))
        return EXIT_UNJUDGEABLE
    counts = {}
    failures = []
    for rel in sorted(records):
        info = records[rel]
        key = "+".join(info["archs"])
        counts[key] = counts.get(key, 0) + 1
        if not info["ok"]:
            failures.append("%s: %s (no slice that runs on every %s Mac)" % (rel, key, target_arch))
        for msg in info.get("mismatch", ()):
            failures.append("%s: %s — a loader picks the slice by the table and then loads what the slice holds "
                            "(RULINGS DB4)" % (rel, msg))
    for rel in sorted(hardlinks):                     # RULINGS DI2: the updater cannot extract a hard-link member
        failures.append("%s: %s" % (rel, HARD_LINK_WHY % hardlinks[rel]))
    memo = {}
    entries = []
    for rel in sorted(set([r for r in records if is_entry(r)] + [r for r in links if is_entry(r)]
                          + [r for r in hardlinks if is_entry(r)])):
        real, _ = _resolve(rel, links, records, files, hardlinks, memo)
        entries.append((rel, real, records.get(real)))
    for req in requires:
        req = req.strip("/")
        hits = sorted(r for r in set(records) | set(links) | files | set(hardlinks) if r == req or r.endswith("/" + req))
        if not hits:
            failures.append("required entry point %s: missing" % req)
            continue
        for r in hits:
            if r in hardlinks:
                continue                              # named above: the archive cannot be extracted as packed
            real, why = _resolve(r, links, records, files, hardlinks, memo)
            if real is None:
                failures.append("required entry point %s: %s" % (r, why))
            elif not records[real]["ok"]:
                pass                                  # already named above with its architecture
    plist_min = None
    if label.endswith(".app") and not is_tar:
        try:
            with open(os.path.join(path, "Contents", "Info.plist"), "rb") as f:
                plist_min = plistlib.load(f).get("LSMinimumSystemVersion")
        except (OSError, plistlib.InvalidFileException, ValueError):
            plist_min = None
    rc = EXIT_OK
    if failures:
        rc = EXIT_FAIL
    elif problems or not records:
        rc = EXIT_UNJUDGEABLE
    if quiet and rc == EXIT_OK:
        del out[head:]
        return rc
    out.append("  Mach-O files: %d%s" % (len(records), "".join(" · %s %d" % kv for kv in sorted(counts.items()))))
    if entries and not quiet:
        out.append("  entry points (minimum macOS):")
        for rel, real, info in entries:
            shown = rel if real in (None, rel) else "%s -> %s" % (rel, posixpath.basename(real))
            archs = "+".join(info["archs"]) if info else ("(tar hard link)" if rel in hardlinks else "(not a Mach-O file)")
            out.append("    %-60s %-14s min %s" % (shown, archs, (info or {}).get("minos") or "?"))
    if plist_min is not None and not quiet:
        out.append("  Info.plist LSMinimumSystemVersion = %s" % plist_min)
        newer = [(posixpath.basename(rel), info["minos"]) for rel, _, info in entries
                 if info and info.get("minos") and _vtuple(info["minos"]) > _vtuple(str(plist_min))]
        if newer:
            out.append("  WARN (informational, RULINGS BQ1(e) — owner decision): entry points need a newer "
                       "macOS than LSMinimumSystemVersion %s: %s" % (plist_min, ", ".join("%s %s" % nm for nm in newer)))
    for f in failures:
        out.append("FAIL %s" % f)
    for p in problems:
        out.append("CANNOT JUDGE %s" % p)
    if not records:
        out.append("CANNOT JUDGE: no Mach-O file found — an empty tree is not a pass")
    return rc


def main(argv=None):
    ap = argparse.ArgumentParser(description="fail when a Mach-O file lacks the target architecture")
    ap.add_argument("--target", required=True, help="x86_64-apple-darwin | aarch64-apple-darwin (or x86_64 / arm64)")
    ap.add_argument("--require", action="append", default=[], metavar="REL",
                    help="an entry point (relative-path suffix) that must exist, resolve and carry the target; repeatable")
    ap.add_argument("--quiet", action="store_true", help="print only problems and the result line")
    ap.add_argument("paths", nargs="+")
    try:
        a = ap.parse_args(argv)
    except SystemExit as e:
        return EXIT_UNJUDGEABLE if e.code else EXIT_OK
    target_arch = TARGETS.get(a.target)
    if target_arch is None:
        print("check_macho_arch: unknown --target %r (x86_64-apple-darwin | aarch64-apple-darwin)" % a.target)
        return EXIT_UNJUDGEABLE
    out = ["check_macho_arch: target %s (%s)" % (target_arch, a.target)]
    rc = EXIT_OK
    for p in a.paths:
        if not os.path.lexists(p):
            out.append("CANNOT JUDGE: %s does not exist" % p)
            r = EXIT_UNJUDGEABLE
        else:
            r = check_one(p, target_arch, a.require, a.quiet, out)
        rc = EXIT_FAIL if EXIT_FAIL in (r, rc) else max(rc, r)
    if rc == EXIT_OK:
        out.append("RESULT: PASS — every Mach-O file runs on %s (exit 0)" % target_arch)
    elif rc == EXIT_FAIL:
        out.append("RESULT: FAIL — see the FAIL lines (exit 1)")
    else:
        out.append("RESULT: CANNOT JUDGE (exit 2) — not a pass")
    print("\n".join(out))
    return rc


if __name__ == "__main__":
    sys.exit(main())
