#!/usr/bin/env python3
"""check_macho_arch.py — every Mach-O file of a macOS bundle must carry the target architecture
(0.15.0 · track T11 · RULINGS BQ1(b) · owner order addendum 11).

Why: the Intel (x86_64) macOS leg is a CROSS build on an arm64 runner, the runtime tree
(`src-tauri/runtime`: python, node, git, uv) is downloaded per target, and nothing checked the
architecture of what ended up inside `cys.app` — a wrong-architecture runtime (for example an
arm64-only `runtime/python/bin/python3` in the Intel installer) would have been signed, notarised and
shipped without any red light. This gate reads the Mach-O / fat (universal) headers of every file and
fails, naming each file, when one lacks the target architecture:
  · thin Mach-O of the target architecture            → pass
  · thin Mach-O of another architecture               → FAIL
  · universal (fat) containing the target              → pass
  · universal (fat) without the target                 → FAIL
  · a file that starts like Mach-O but is cut short    → FAIL (a broken binary is never a pass)
  · not Mach-O (text, data, a Java class file that also starts with 0xCAFEBABE) → ignored
It also prints the count per architecture and the minimum macOS of the entry points
(`LC_BUILD_VERSION` / `LC_VERSION_MIN_MACOSX`), and for an `.app` its Info.plist
`LSMinimumSystemVersion` with an informational WARN when an entry point needs a newer macOS
(RULINGS BQ1(e): whether that value changes is the owner's decision — the gate does not fail on it).

Reads bytes only: no `lipo`, `otool`, `file` or `vtool` (they are not on a stock Mac and not needed),
no execution of anything it inspects, no network. Standard library only (python 3.8+).

Inputs: an `.app`, any directory (e.g. `src-tauri/runtime`), a single file, or a tar archive
(`.tar.gz` / `.tgz` / `.tar` — the updater archive `cys.app.tar.gz`), one or more.

    python3 scripts/check_macho_arch.py --target x86_64-apple-darwin target/x86_64-apple-darwin/release/bundle/macos/cys.app
    python3 scripts/check_macho_arch.py --target x86_64-apple-darwin --quiet \\
        --require python/bin/python3 --require node/bin/node --require git/bin/git --require uv/uv src-tauri/runtime

Exit: 0 = every Mach-O file has the target · 1 = at least one does not (or is cut short) — each named
      2 = cannot judge (path missing, archive unreadable, no Mach-O at all, a --require entry missing or
          not Mach-O, bad arguments) — never a pass
"""
import argparse
import os
import plistlib
import posixpath
import struct
import sys
import tarfile

EXIT_OK, EXIT_FAIL, EXIT_UNJUDGEABLE = 0, 1, 2

MH_MAGIC, MH_MAGIC_64 = 0xFEEDFACE, 0xFEEDFACF
FAT_MAGIC, FAT_MAGIC_64 = 0xCAFEBABE, 0xCAFEBABF
LC_VERSION_MIN_MACOSX, LC_BUILD_VERSION = 0x24, 0x32
FAT_MAX_ARCHS = 30          # a Java class file reads its version (major >= 45) where nfat_arch would be
CPU_NAMES = {
    0x01000007: "x86_64", 0x0100000C: "arm64", 0x0200000C: "arm64_32",
    7: "i386", 12: "arm", 18: "ppc", 0x01000012: "ppc64",
}
TARGETS = {
    "x86_64-apple-darwin": "x86_64", "x86_64": "x86_64", "x64": "x86_64",
    "aarch64-apple-darwin": "arm64", "aarch64": "arm64", "arm64": "arm64",
}
# entry points whose minimum macOS is printed (relative-path suffixes; Contents/MacOS/* is added by rule)
ENTRY_SUFFIXES = ("python/bin/python3", "node/bin/node", "git/bin/git", "uv/uv")
HEAD_BYTES = 4096


class Truncated(Exception):
    pass


def cpu_name(cputype):
    return CPU_NAMES.get(cputype, "cpu-0x%08x" % cputype)


def _read_exact(read, off, n):
    b = read(off, n)
    if len(b) < n:
        raise Truncated("needs %d bytes at offset %d, file ends earlier" % (n, off))
    return b


def parse_header(read):
    """→ None when the bytes are not Mach-O; else a dict {archs: [name…], slices: [(name, offset)],
    kind: thin|fat}. Raises Truncated when the bytes start like Mach-O but the header is cut short."""
    head = read(0, 8)
    if len(head) < 4:
        return None
    le = struct.unpack("<I", head[:4])[0]
    be = struct.unpack(">I", head[:4])[0]
    if le in (MH_MAGIC, MH_MAGIC_64) or be in (MH_MAGIC, MH_MAGIC_64):
        endian = "<" if le in (MH_MAGIC, MH_MAGIC_64) else ">"
        if len(head) < 8:
            raise Truncated("thin Mach-O header shorter than 8 bytes")
        cputype = struct.unpack(endian + "I", head[4:8])[0]
        name = cpu_name(cputype)
        return {"kind": "thin", "archs": [name], "slices": [(name, 0)]}
    if be in (FAT_MAGIC, FAT_MAGIC_64):
        if len(head) < 8:
            raise Truncated("fat header shorter than 8 bytes")
        nfat = struct.unpack(">I", head[4:8])[0]
        if nfat == 0 or nfat > FAT_MAX_ARCHS:
            return None      # a Java class file (0xCAFEBABE + version >= 45) or other data — not Mach-O
        entry = 20 if be == FAT_MAGIC else 32
        table = _read_exact(read, 8, nfat * entry)
        archs, slices = [], []
        for i in range(nfat):
            e = table[i * entry:(i + 1) * entry]
            cputype = struct.unpack(">I", e[0:4])[0]
            offset = struct.unpack(">I", e[8:12])[0] if entry == 20 else struct.unpack(">Q", e[8:16])[0]
            name = cpu_name(cputype)
            archs.append(name)
            slices.append((name, offset))
        return {"kind": "fat", "archs": archs, "slices": slices}
    return None


def _decode_version(v):
    major, minor, patch = v >> 16, (v >> 8) & 0xFF, v & 0xFF
    return "%d.%d.%d" % (major, minor, patch) if patch else "%d.%d" % (major, minor)


def min_macos(read, offset):
    """Minimum macOS of the thin Mach-O starting at OFFSET (LC_BUILD_VERSION, else LC_VERSION_MIN_MACOSX),
    or None when neither load command is present. Raises Truncated on a cut-short header."""
    magic_bytes = _read_exact(read, offset, 4)
    le = struct.unpack("<I", magic_bytes)[0]
    endian = "<" if le in (MH_MAGIC, MH_MAGIC_64) else ">"
    magic = struct.unpack(endian + "I", magic_bytes)[0]
    if magic not in (MH_MAGIC, MH_MAGIC_64):
        raise Truncated("slice at offset %d does not start with a Mach-O magic" % offset)
    hsize = 32 if magic == MH_MAGIC_64 else 28
    hdr = _read_exact(read, offset, hsize)
    ncmds, sizeofcmds = struct.unpack(endian + "II", hdr[16:24])
    cmds = _read_exact(read, offset + hsize, sizeofcmds)
    pos, legacy = 0, None
    for _ in range(ncmds):
        if pos + 8 > len(cmds):
            raise Truncated("load commands end early")
        cmd, cmdsize = struct.unpack(endian + "II", cmds[pos:pos + 8])
        if cmdsize < 8:
            raise Truncated("load command of size %d" % cmdsize)
        if cmd == LC_BUILD_VERSION and pos + 16 <= len(cmds):
            return _decode_version(struct.unpack(endian + "I", cmds[pos + 12:pos + 16])[0])
        if cmd == LC_VERSION_MIN_MACOSX and pos + 12 <= len(cmds):
            legacy = _decode_version(struct.unpack(endian + "I", cmds[pos + 8:pos + 12])[0])
        pos += cmdsize
    return legacy


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


# ── sources: a directory tree or a tar archive, both seen as {rel: reader | link target} ──────────────

class DirSource:
    def __init__(self, root):
        self.root = root
        self.base = os.path.basename(os.path.normpath(root)) if root.endswith(".app") or root.endswith(".app/") else ""

    def items(self):
        """(rel, kind, payload): kind = 'file' (payload = absolute path) or 'link' (payload = link target)."""
        if os.path.isfile(self.root) and not os.path.islink(self.root):
            yield os.path.basename(self.root), "file", self.root
            return
        for dirpath, dirnames, filenames in os.walk(self.root, followlinks=False):
            for name in sorted(filenames) + sorted(d for d in dirnames if os.path.islink(os.path.join(dirpath, d))):
                full = os.path.join(dirpath, name)
                rel = os.path.relpath(full, self.root).replace(os.sep, "/")
                if self.base:
                    rel = self.base + "/" + rel
                if os.path.islink(full):
                    yield rel, "link", os.readlink(full)
                elif os.path.isfile(full):
                    yield rel, "file", full

    @staticmethod
    def reader(path):
        def read(off, n):
            with open(path, "rb") as f:
                f.seek(off)
                return f.read(n)
        return read


class TarSource:
    def __init__(self, path):
        self.path = path
        self.tar = tarfile.open(path, "r:*")

    def items(self):
        for m in self.tar:
            rel = m.name[2:] if m.name.startswith("./") else m.name
            if m.issym():
                yield rel, "link", m.linkname
            elif m.isfile():
                yield rel, "file", m

    def reader(self, member):
        fobj = self.tar.extractfile(member)

        def read(off, n):
            fobj.seek(off)
            return fobj.read(n)
        return read


def _resolve(rel, links, files, depth=0):
    if rel in files or depth > 8:
        return rel if rel in files else None
    target = links.get(rel)
    if target is None:
        return None
    nxt = posixpath.normpath(posixpath.join(posixpath.dirname(rel), target)) if not target.startswith("/") else None
    return _resolve(nxt, links, files, depth + 1) if nxt else None


def check_tree(source, label, target_arch, requires, quiet, out):
    """→ (exit code for this input, stats)."""
    files, links = {}, {}
    for rel, kind, payload in source.items():
        (files if kind == "file" else links)[rel] = payload
    counts, failures, entries, n_macho = {}, [], [], 0
    parsed = {}
    for rel in sorted(files):
        read = source.reader(files[rel])
        try:
            info = parse_header(read)
        except Truncated as e:
            failures.append("%s: Mach-O header cut short (%s)" % (rel, e))
            continue
        except OSError as e:
            failures.append("%s: unreadable (%s)" % (rel, e))
            continue
        if info is None:
            continue
        n_macho += 1
        parsed[rel] = info
        key = "+".join(info["archs"])
        counts[key] = counts.get(key, 0) + 1
        if target_arch not in info["archs"]:
            failures.append("%s: %s (no %s slice)" % (rel, key, target_arch))
    # entry points: Contents/MacOS/* and the runtime's interpreters / tools (symlinks resolved)
    entry_rels = sorted(set([r for r in files if is_entry(r)] + [r for r in links if is_entry(r)]))
    for rel in entry_rels:
        real = rel if rel in files else _resolve(rel, links, files)
        info = parsed.get(real) if real else None
        if info is None:
            entries.append((rel, real, None, None))
            continue
        read = source.reader(files[real])
        slice_off = dict(info["slices"]).get(target_arch, info["slices"][0][1])
        try:
            mos = min_macos(read, slice_off)
        except (Truncated, OSError):
            mos = None
        entries.append((rel, real, "+".join(info["archs"]), mos))
    plist_min = None
    if label.endswith(".app") and isinstance(source, DirSource):
        try:
            with open(os.path.join(source.root, "Contents", "Info.plist"), "rb") as f:
                plist_min = plistlib.load(f).get("LSMinimumSystemVersion")
        except (OSError, plistlib.InvalidFileException, ValueError):
            plist_min = None
    missing = []
    for req in requires:
        hit = [r for r in entry_rels + sorted(files) if r == req or r.endswith("/" + req)]
        real = None
        for r in hit:
            real = r if r in files else _resolve(r, links, files)
            if real:
                break
        if not real or real not in parsed:
            missing.append(req)
    rc = EXIT_OK
    if failures:
        rc = EXIT_FAIL
    if missing or n_macho == 0:
        rc = max(rc, EXIT_UNJUDGEABLE) if not failures else rc
    # ── report ──
    if not quiet or rc != EXIT_OK:
        out.append("== %s" % label)
        out.append("  Mach-O files: %d%s" % (n_macho, "".join(" · %s %d" % kv for kv in sorted(counts.items()))))
        if entries and not quiet:
            out.append("  entry points (minimum macOS):")
            for rel, real, archs, mos in entries:
                shown = rel if real in (None, rel) else "%s -> %s" % (rel, posixpath.basename(real))
                out.append("    %-60s %-14s min %s" % (shown, archs or "(not Mach-O)", mos or "?"))
        if plist_min is not None and not quiet:
            out.append("  Info.plist LSMinimumSystemVersion = %s" % plist_min)
            newer = [(posixpath.basename(r), m) for r, _, _, m in entries
                     if m and _vtuple(m) > _vtuple(str(plist_min))]
            if newer:
                out.append("  WARN (informational, RULINGS BQ1(e) — owner decision): entry points need a newer "
                           "macOS than LSMinimumSystemVersion %s: %s"
                           % (plist_min, ", ".join("%s %s" % nm for nm in newer)))
        for f in failures:
            out.append("FAIL %s" % f)
        for req in missing:
            out.append("CANNOT JUDGE: required entry point %s is missing or not Mach-O" % req)
        if n_macho == 0:
            out.append("CANNOT JUDGE: no Mach-O file found — an empty tree is not a pass")
    return rc, (n_macho, len(failures))


def main(argv=None):
    ap = argparse.ArgumentParser(description="fail when a Mach-O file lacks the target architecture")
    ap.add_argument("--target", required=True, help="x86_64-apple-darwin | aarch64-apple-darwin (or x86_64 / arm64)")
    ap.add_argument("--require", action="append", default=[], metavar="REL",
                    help="an entry point (relative-path suffix) that must exist and be Mach-O; repeatable")
    ap.add_argument("--quiet", action="store_true", help="print only failures and the result line")
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
    rc, total, bad = EXIT_OK, 0, 0
    for p in a.paths:
        label = p.rstrip("/")
        if not os.path.exists(p):
            out.append("CANNOT JUDGE: %s does not exist" % p)
            rc = max(rc, EXIT_UNJUDGEABLE) if rc != EXIT_FAIL else rc
            continue
        try:
            if os.path.isfile(p) and (p.endswith((".tar.gz", ".tgz", ".tar")) or tarfile.is_tarfile(p)):
                src = TarSource(p)
            else:
                src = DirSource(p)
            r, (n, f) = check_tree(src, label, target_arch, a.require, a.quiet, out)
        except (tarfile.TarError, OSError, EOFError) as e:
            out.append("CANNOT JUDGE: %s unreadable (%s)" % (p, e))
            r, n, f = EXIT_UNJUDGEABLE, 0, 0
        total += n
        bad += f
        if r == EXIT_FAIL or rc == EXIT_FAIL:
            rc = EXIT_FAIL
        else:
            rc = max(rc, r)
    if rc == EXIT_OK:
        out.append("RESULT: PASS — every Mach-O file (%d) has %s (exit 0)" % (total, target_arch))
    elif rc == EXIT_FAIL:
        out.append("RESULT: FAIL — %d Mach-O file(s) without %s or cut short (exit 1)" % (bad, target_arch))
    else:
        out.append("RESULT: CANNOT JUDGE (exit 2) — not a pass")
    print("\n".join(out))
    return rc


if __name__ == "__main__":
    sys.exit(main())
