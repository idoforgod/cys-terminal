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
Links and names (RULINGS DU1–DU2, DN3–DN5, DL1 (3)–(4) — the closed form; DU replaces DN1–DN2):
  (1) an ARCHIVE is measurable only in the PLAIN SHAPE our own build produces (DU1), else it is not measurable
      (exit 2) naming the first member outside it: (a) every member is a directory, a regular file or a symbolic
      link (a hard link FAILS, (3)); (b) every member name and link target is in the one accepted spelling —
      relative, components non-empty and not `.` (one leading `./` of a member name dropped), `..` only in a link
      target; (c) no name twice unless every occurrence is a directory; (d) no member's name passes through a
      symbolic-link member (decided at the end of the pass — the link may come later). Then every member is
      extracted to exactly the path its name says, once, whatever the order. A refused member still proves that
      its parent directories exist (DU2). In a directory tree a link target follows (b);
  (2) a symbolic link is resolved the way the kernel resolves a path (`resolve`, iterative): component by
      component, every intermediate component must be a directory of the archive (or a link to one), `..` steps
      to the real parent, at most MAXSYMLINKS (32, the macOS SDK's <sys/param.h>) link expansions in total, a
      revisit of the same link with the same remaining path is a loop; a REQUIRED entry point that does not
      resolve FAILS with the reason (not a directory / missing component / too many links / loop / dangling) —
      unless its resolution stopped at a not-measurable path, which makes it not measurable (DN4);
  (3) EVERY tar hard-link member FAILS: the updater unpacks the updater archive (`cys.app.tar.gz`, the only tar
      this gate is given — release.yml, build-macos-signed.sh) with tauri-plugin-updater 2.10.1
      (`src/updater.rs:1236–1250`), whose `entry.unpack(<file>)` of the Rust `tar` crate 0.4.46
      (`src/entry.rs:203–204, 528–548`) takes a hard link's stored name relative to the process's working
      directory, not the extraction folder — so the archive does not guarantee which file the link names; it
      depends on what exists outside the extraction folder;
  (4) the checker's own failure — recursion, memory, any unexpected exception — is exit 2, never exit 1.
  Three result kinds, never mixed (DN4): proven wrong (exit 1) · not measurable (exit 2) · pass (0); exit 1 when
  anything is proven wrong, else 2 when anything is not measurable, else 0.
Every load command of every Mach-O is checked (RULINGS DI1, DL2): each must start and end inside `sizeofcmds`,
read one by one and forward; the streaming reader keeps only the lookback the next read needs (its buffer stays
around two 64 KiB chunks however many commands follow each other); a load-command area over 16 MiB, or a single
read over 1 MiB, exceeds the checker's resource cap and is not measurable (exit 2).

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
# RULINGS DL1 (3) — the sentence for EVERY hard-link member (its stored target is shown VERBATIM: all hard links are
# refused, so no spelling rule applies to it — DN5) (the updater's extractor, tauri-plugin-updater 2.10.1
# src/updater.rs:1236–1250 → tar 0.4.46 src/entry.rs:203–204, 528–548, takes the stored name relative to the process's
# working directory, not the extraction folder)
HARD_LINK_WHY = ("%s: a tar hard link (to %s) — the archive does not guarantee which file this link names: the "
                 "updater's extractor takes the stored name relative to its working directory, so the result depends "
                 "on what exists outside the extraction folder (RULINGS DL1 (3))")
# the platform's limit of link expansions in one path lookup — MAXSYMLINKS in the macOS SDK's <sys/param.h>
# (/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/sys/param.h:197: `#define MAXSYMLINKS 32`)
MAXSYMLINKS = 32
# RULINGS DL2 — the checker's resource caps (beyond them: not measurable, exit 2)
MAX_CMDS_AREA = 16 << 20      # a load-command area larger than 16 MiB (real ones are kilobytes)
READ_CAP = 1 << 20            # no single read of a header / table / command field is larger than this


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
    if sizeofcmds > MAX_CMDS_AREA:
        raise Unjudgeable("a load-command area of %d bytes exceeds the checker's resource cap of %d bytes — not "
                          "measurable (RULINGS DL2)" % (sizeofcmds, MAX_CMDS_AREA))
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
    """read(offset, n) over a stream that can only go forward (a tar member read in streaming mode). RULINGS DL2:
    it keeps only the lookback the next read can need — every caller reads at non-decreasing offsets, so bytes
    before the current offset are dropped (in steps of CHUNK, amortised); `peak` is the largest buffer it held."""
    CHUNK = 1 << 16

    def __init__(self, f):
        self.f, self.base, self.buf, self.peak = f, 0, b"", 0

    def read(self, off, n):
        if off < self.base:
            raise Unjudgeable("internal: a backward read at %d (stream at %d)" % (off, self.base))
        if n > READ_CAP:
            raise Unjudgeable("a read of %d bytes exceeds the checker's cap of %d bytes — not measurable" % (n, READ_CAP))
        have_end = self.base + len(self.buf)
        if off >= have_end:                      # nothing kept is needed: skip forward without keeping the bytes
            gap = off - have_end
            while gap > 0:
                chunk = self.f.read(min(gap, self.CHUNK))
                if not chunk:
                    break
                gap -= len(chunk)
            self.base, self.buf = off - gap, b""
        elif off - self.base >= self.CHUNK:      # drop what no later read can need (amortised)
            self.buf, self.base = self.buf[off - self.base:], off
        need = off + n - (self.base + len(self.buf))
        while need > 0:
            chunk = self.f.read(max(need, self.CHUNK))
            if not chunk:
                break
            self.buf += chunk
            need -= len(chunk)
        self.peak = max(self.peak, len(self.buf))
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


# ── RULINGS DN1–DN4: ONE spelling, one member per name, three result kinds ─────────────────────────────

def spelling_problem(name, kind):
    """DN1 — the ONE accepted spelling: a relative path whose components are all non-empty and not `.`; a symbolic
    link's target may also contain `..`. → None, or why NAME is not measurable. (Nothing is normalised.)"""
    if name.startswith("/"):
        return "%s %r is an absolute path" % (kind, name)
    parts = name.split("/")
    if any(p in ("", ".") for p in parts):
        return "%s %r has an empty or `.` component" % (kind, name)
    if kind != "link target" and ".." in parts:
        return "%s %r contains `..`" % (kind, name)
    return None


class Tree:
    """What one pass over a directory tree or an archive found. For an ARCHIVE the plain shape of RULINGS DU1 is
    checked: a violation is recorded in `shape` (ordered by member) — the archive is then not measurable."""

    def __init__(self, archive=False):
        self.archive = archive
        self.records, self.links, self.files, self.problems = {}, {}, set(), []
        self.hardlinks, self.dirs = {}, set()
        self.unmeasurable = {}      # path → why: a refused member, a rejected link, where an odd member could land
        self.uncertain = set()      # the parents of where an odd member could land (consulted only when a lookup fails)
        self.kinds = {}             # name → "dir" | "other" (DU1 (c))
        self.shape = []             # (member order, "<member>: not measurable — <why>") — DU1 violations
        self.names = []             # (member order, name) of every member in the accepted spelling (DU1 (d), DU2)
        self.symlinks = set()       # names of the symbolic-link members (DU1 (d))
        self.order = 0

    def forget(self, rel):
        for m in (self.records, self.links, self.hardlinks):
            m.pop(rel, None)
        self.files.discard(rel)
        self.dirs.discard(rel)

    def not_measurable(self, label, why, shadow=None, order=None):
        line = "%s: not measurable — %s" % (label, why)
        if self.archive:
            self.shape.append((self.order if order is None else order, line))
        else:
            self.problems.append(line)
        if shadow:
            self.forget(shadow)
            self.unmeasurable[shadow] = "%s (%s)" % (label, why)
            parts = shadow.split("/")
            self.uncertain.update("/".join(parts[:i]) for i in range(1, len(parts)))

    def claim(self, rel, kind):
        """DU1 (c): no name twice unless every occurrence is a directory. → True to go on."""
        if rel in self.unmeasurable:
            return False
        prev = self.kinds.get(rel)
        if prev is not None and (prev != "dir" or kind != "dir"):
            self.not_measurable(rel, "%s occurs more than once in the archive (one of them not a directory)" % rel, rel)
            return False
        self.kinds[rel] = kind
        return True

    def add_link(self, rel, target):
        p = spelling_problem(target, "link target")
        if p:
            self.not_measurable(rel, p)
            self.unmeasurable[rel] = p
            return
        self.links[rel] = target

    def finish(self):
        if not self.archive:
            for key in list(self.records) + list(self.files) + list(self.links) + list(self.hardlinks):
                parts = key.split("/")
                for i in range(1, len(parts)):   # every parent of a member exists after extraction
                    self.dirs.add("/".join(parts[:i]))
        below = []
        for order, name in self.names:            # DU2: a refused member still proves its parent directories
            parts = name.split("/")
            for i in range(1, len(parts)):
                prefix = "/".join(parts[:i])
                if prefix in self.symlinks:       # DU1 (d): the name passes through a symbolic-link member
                    below.append((order, name, prefix))
                    break
                self.dirs.add(prefix)
        for order, name, prefix in below:         # decided at the end of the pass: the link may come later
            self.not_measurable(name, "its name passes through the symbolic-link member %s (RULINGS DU1 (d))" % prefix,
                                name, order)
            real, _, kind = _walk(posixpath.dirname(name), self)
            if not kind and real is not None:     # the place it really lands: what was measured there is stale
                dest = real + "/" + posixpath.basename(name) if real else posixpath.basename(name)
                self.forget(dest)
                self.unmeasurable[dest] = "%s lands here through %s" % (name, prefix)


def _shadow(name):
    """Where a member with a rejected name could land after extraction (to call that place not measurable)."""
    s = posixpath.normpath(name.lstrip("/")) if name.strip("/") else ""
    return None if not s or s == "." or s.startswith("..") else s


def scan_dir(root, target_arch):
    """One pass over a directory tree → Tree (on a directory tree a hard link is simply another regular file)."""
    tree = Tree()
    base = os.path.basename(os.path.normpath(root)) if root.rstrip("/").endswith(".app") else ""

    def rel_of(full):
        rel = os.path.relpath(full, root).replace(os.sep, "/")
        return rel if not base else (base if rel == "." else base + "/" + rel)

    def one(full, rel):
        if os.path.islink(full):
            tree.add_link(rel, os.readlink(full))
            return
        if not os.path.isfile(full):
            return
        tree.files.add(rel)
        try:
            size = os.path.getsize(full)
            with open(full, "rb") as f:
                def read(off, n):
                    f.seek(off)
                    return f.read(n)
                info = inspect(read, size, target_arch)
        except Unjudgeable as e:
            tree.problems.append("%s: %s" % (rel, e))
            tree.unmeasurable[rel] = str(e)
            return
        except OSError as e:
            tree.problems.append("%s: unreadable (%s)" % (rel, e.strerror or e))
            tree.unmeasurable[rel] = "unreadable"
            return
        if info is not None:
            tree.records[rel] = info

    if os.path.isfile(root) and not os.path.islink(root):
        one(root, os.path.basename(root))
        tree.finish()
        return tree

    def onerror(e):
        tree.problems.append("%s: unreadable directory (%s)" % (rel_of(e.filename) if e.filename else root, e.strerror or e))

    for dirpath, dirnames, filenames in os.walk(root, followlinks=False, onerror=onerror):
        if rel_of(dirpath) != ".":
            tree.dirs.add(rel_of(dirpath))
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            one(full, rel_of(full))
        for d in sorted(dirnames):
            full = os.path.join(dirpath, d)
            if os.path.islink(full):
                tree.add_link(rel_of(full), os.readlink(full))
    tree.finish()
    return tree


def scan_tar(path, target_arch, out):
    """ONE streaming pass in archive order (CA1) → Tree. A member name is accepted in the one spelling of DN1 (one
    leading `./` dropped); a repeated name is DN2; a hard-link member is kept with its stored target VERBATIM
    (every hard link is refused — DL1 (3) — so its target needs no spelling rule)."""
    tree = Tree(archive=True)
    raw = CountingFile(path)
    members = 0
    try:
        with tarfile.open(fileobj=raw, mode="r|*") as tar:
            for m in tar:
                members += 1
                tree.order = members
                rel = m.name[2:] if m.name.startswith("./") else m.name
                p = spelling_problem(rel, "member name")                       # DU1 (b)
                if p:
                    tree.not_measurable(m.name, p, _shadow(rel))
                    continue
                tree.names.append((members, rel))
                if m.type not in PLAIN_TYPES:                                  # DU1 (a)
                    tree.not_measurable(rel, "member type %r is not a directory, a regular file, a symbolic link or "
                                        "a hard link (RULINGS DU1 (a))" % m.type.decode("latin-1"), rel)
                    continue
                if not tree.claim(rel, "dir" if m.isdir() else "other"):       # DU1 (c)
                    continue
                if m.issym():
                    tree.symlinks.add(rel)
                if m.islnk():
                    tree.hardlinks[rel] = m.linkname
                    continue
                if m.isdir():
                    tree.dirs.add(rel)
                    continue
                if m.issym():
                    tree.add_link(rel, m.linkname)
                    continue
                if not m.isfile():
                    continue
                tree.files.add(rel)
                fobj = tar.extractfile(m)
                try:
                    info = inspect(ForwardReader(fobj).read, m.size, target_arch)
                except Unjudgeable as e:
                    tree.problems.append("%s: %s" % (rel, e))
                    tree.unmeasurable[rel] = str(e)
                    continue
                if info is not None:
                    tree.records[rel] = info
        while raw.read(1 << 16):        # the end-of-archive padding / gzip trailer: count it, still one pass
            pass
    finally:
        raw.close()
    out.append("  archive read in one pass: %d members, %d of %d compressed bytes"
               % (members, raw.read_bytes, os.path.getsize(path)))
    tree.finish()
    return tree


WRONG, UNMEASURABLE = "wrong", "not measurable"
# RULINGS DU1 (a): the member types of the plain shape — regular file ('0', old '\0'), directory, symbolic link, and
# the hard link (refused as DL1 (3)); the long-name / extended-header records tarfile consumes are not members
PLAIN_TYPES = (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE, tarfile.SYMTYPE, tarfile.LNKTYPE)


def _walk(rel, tree):
    """DN3 (= DL1 (2)) — REL walked the way the kernel resolves a path, ITERATIVELY → (real path | None, why, kind):
    component by component; a symbolic-link member is replaced by its target (relative to its directory); every
    INTERMEDIATE component must be a directory of the archive (or a link to one); `..` steps to the real parent;
    at most MAXSYMLINKS link expansions; a revisit of the same link with the same remaining path is a loop.
    KIND (DN4): "" walked · WRONG proven wrong · UNMEASURABLE the walk stopped at a not-measurable path."""
    pending = rel.split("/") if rel else []
    done, expansions, seen = [], 0, set()
    while pending:
        c = pending.pop(0)
        if c == "..":
            if not done:
                return None, "outside the archive: `..` climbs above its root", WRONG
            done.pop()
            continue
        cand = "/".join(done + [c])
        if cand in tree.unmeasurable:
            return None, "not measurable — %s" % tree.unmeasurable[cand], UNMEASURABLE
        if cand in tree.hardlinks:
            return None, HARD_LINK_WHY % (cand, tree.hardlinks[cand]), WRONG
        if cand in tree.links:
            expansions += 1
            if expansions > MAXSYMLINKS:
                return None, ("too many links: more than %d link expansions (the platform's MAXSYMLINKS) — the "
                              "kernel would not resolve it" % MAXSYMLINKS), WRONG
            state = (cand, tuple(pending))
            if state in seen:
                return None, "link loop (%s is met again with the same remaining path)" % cand, WRONG
            seen.add(state)
            pending = tree.links[cand].split("/") + pending
            continue
        if pending:
            if cand in tree.dirs:
                done.append(c)
                continue
            if cand in tree.files:
                return None, "not a directory: %s is a file" % cand, WRONG
            if cand in tree.uncertain:
                return None, "not measurable — %s may be a directory of a member with a rejected name" % cand, UNMEASURABLE
            return None, "missing component: %s" % cand, WRONG
        done.append(c)
    return "/".join(done), "", ""


def resolve(rel, tree):
    """REL resolved to a Mach-O member (DN3) → (key | None, why, kind) — kinds as in `_walk`."""
    final, why, kind = _walk(rel, tree)
    if final is None:
        return None, why, kind
    if final in tree.records:
        return final, "", ""
    if final in tree.files:
        return None, "not a Mach-O file", WRONG
    if final in tree.dirs:
        return None, "a directory, not a Mach-O file", WRONG
    if final in tree.uncertain:
        return None, "not measurable — %s may be a directory of a member with a rejected name" % final, UNMEASURABLE
    return None, ("dangling link: no member %s" % final if final != rel else "missing"), WRONG


def check_one(path, target_arch, requires, quiet, out):
    """RULINGS DL1 (4): the checker's own failure — recursion, memory, any unexpected exception — is exit 2."""
    head = len(out)
    try:
        return _check_one(path, target_arch, requires, quiet, out)
    except Exception as e:                                    # noqa: BLE001 — never dressed as a FAIL (1)
        del out[head:]
        out.append("== %s" % path.rstrip("/"))
        out.append("CANNOT JUDGE: the checker itself failed (%s: %s) — not measurable" % (type(e).__name__, e))
        return EXIT_UNJUDGEABLE


def _check_one(path, target_arch, requires, quiet, out):
    label = path.rstrip("/")
    head = len(out)
    out.append("== %s" % label)
    is_tar = os.path.isfile(path) and (path.endswith((".tar.gz", ".tgz", ".tar")) or tarfile.is_tarfile(path))
    try:
        tree = scan_tar(path, target_arch, out) if is_tar else scan_dir(path, target_arch)
    except (tarfile.TarError, OSError, EOFError, zlib.error) as e:
        out.append("CANNOT JUDGE: %s unreadable (%s)" % (path, e))
        return EXIT_UNJUDGEABLE
    records, links, files, problems, hardlinks = tree.records, tree.links, tree.files, tree.problems, tree.hardlinks
    counts = {}
    failures = []                                     # DN4: proven wrong (exit 1) — `problems` are not measurable (2)
    for rel in sorted(records):
        info = records[rel]
        key = "+".join(info["archs"])
        counts[key] = counts.get(key, 0) + 1
        if not info["ok"]:
            failures.append("%s: %s (no slice that runs on every %s Mac)" % (rel, key, target_arch))
        for msg in info.get("mismatch", ()):
            failures.append("%s: %s — a loader picks the slice by the table and then loads what the slice holds "
                            "(RULINGS DB4)" % (rel, msg))
    hard = [HARD_LINK_WHY % (rel, hardlinks[rel]) for rel in sorted(hardlinks)]   # DL1 (3): every hard link fails
    entries = []
    for rel in sorted(set([r for r in records if is_entry(r)] + [r for r in links if is_entry(r)]
                          + [r for r in hardlinks if is_entry(r)])):
        real, _, _ = resolve(rel, tree)
        entries.append((rel, real, records.get(real)))
    for req in requires:
        req = req.strip("/")
        hits = sorted(r for r in set(records) | set(links) | files | set(hardlinks) | set(tree.unmeasurable)
                      if r == req or r.endswith("/" + req))
        if not hits:
            failures.append("required entry point %s: missing" % req)
            continue
        for r in hits:
            if r in hardlinks:
                continue                              # named above (DL1 (3))
            real, why, kind = resolve(r, tree)
            if kind == UNMEASURABLE:                  # DN4: stopped at a not-measurable path — never a proven failure
                problems.append("required entry point %s: %s" % (r, why))
            elif real is None:
                failures.append("required entry point %s: %s" % (r, why))
            elif not records[real]["ok"]:
                pass                                  # already named above with its architecture
    if tree.shape:                                    # RULINGS DU1: not the plain shape → the archive is not measurable
        tree.shape.sort()
        more = len(tree.shape) - 1
        problems.insert(0, "the archive is not in the plain shape (RULINGS DU1) — %s%s"
                        % (tree.shape[0][1], " (and %d more member(s) outside it)" % more if more else ""))
        problems.extend("not judged (the archive is not in the plain shape): %s" % f for f in failures)
        failures = hard                               # hard links stay proven wrong (DL1 (3))
    else:
        failures = failures + hard
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
            archs = "+".join(info["archs"]) if info else ("(tar hard link)" if rel in tree.hardlinks else "(not a Mach-O file)")
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
