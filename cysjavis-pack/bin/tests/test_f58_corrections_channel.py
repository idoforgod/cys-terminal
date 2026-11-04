#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_corrections_channel.py — F58 U2: the corrections channel and the self-service hook block.

Design: DESIGN-F58-final.md (sha16 74cc8cc559f93ffa) §7.2, §8.1-8.2 over DESIGN-F58-r2.md §5.2-5.6, §7.3, §9.1
(T-4, T-10, T-12) and DESIGN-F58-r1.md C2-a.

  T-12  R2-a..R2-d (r2 §5.5): stock file, edited clause, appended restriction, local overlay. Every user-class
        file and every file under the local dir hashes the same before and after (the channel writes nothing).
  T-10p (r2 §5.6, final §7.2) on a pack whose user-class files are the 0.14.56 bytes: the hook block once per
        hook output with N = the role's row count, the directive read-pointer line still ending before character
        2,000, every row printed with the shipped `new` text and the end line; the same block after a clear; a
        0.14.57 pack prints N = 0; negative controls (pointer line removed, one `new` altered) fail the checks.
        The real-binary advance (0.14.56 init-pack -> 0.14.57 init-pack --no-install-hook) needs a cargo build
        and is owed to the build window (U2-SPEC D-U2-2).
  T-4   block at most 1,600 bytes in the worst case, no backslash, once per output, placed after the read
        pointer / local overlay / soul / memory index and before the U16 team block (session-start.sh) and at
        the end of the payload (inject-context.sh); an old _lib.sh without the function prints nothing and the
        hooks still exit 0; a missing, failing or hanging tool prints the pointer without N.
  plus  file gate and row gate cases, --summary, per-role byte totals of final §8.1, paging and parts (§8.2).

The 0.14.56 user-class files are rebuilt by reverse-applying the manifest rows to the shipped files and are
proved equal to the 31f87878 bytes by git blob id (computed here, no git). The install record is written the
way `install` writes it (sha256 of the installed text, src/pack.rs content_hash) and each changed file gets
its `.new` the way the user-class Keep arm writes it.

Run: python3 cysjavis-pack/bin/tests/test_f58_corrections_channel.py
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
TOOL = os.path.join(BIN, "javis_selfservice.py")
HOOKS = os.path.join(PACK, "hooks")
SS_HOOK = os.path.join(HOOKS, "session-start.sh")
IC_HOOK = os.path.join(HOOKS, "inject-context.sh")
LIB = os.path.join(HOOKS, "_lib.sh")
OWNED = os.path.join(BIN, "_owned_process.py")
MANIFEST = os.path.join(PACK, "round", "self-service-corrections.json")
BASH = shutil.which("bash") or "/bin/bash"

ROLES = ("master", "worker", "cso", "reviewer")
BLOCK_HEAD = "■ 셀프 서비스(F58)"
POINTER_RE = re.compile(
    r"^- 제조사 기본값 정정(?: (\d+)건)?\(이 역할\) — 읽기: "
    r"`python3 \"\$CYS_PACK_DIR/bin/javis_selfservice\.py\" corrections --role (\S+)` · 사용자 지침이 우선$", re.M)
END_RE = re.compile(r"^끝: (\d+)/(\d+) 건 · ids sha256 ([0-9a-f]{16}) · manifest sha256 ([0-9a-f]{16})$", re.M)
HEADER_RE = re.compile(r"^== (\S+) · (\S+)$", re.M)
OVERLAY_TEXT = "로컬 지침("
RESTRICTION = "이 작업 공간에서는 큐 자동 배달을 하지 않는다."
FORBIDDEN_IN_BLOCK = ("set-status", "--ack", "CYS_BOOT_NONCE", "부트 브리지", "\\")
DESIGN_TOTALS = {"master": 8859, "worker": 6241, "reviewer": 993}  # final §8.1 (cso changed: T1-12d withdrawn)


def _read_bytes(p):
    with open(p, "rb") as f:
        return f.read()


def _write_bytes(p, data):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as f:
        f.write(data)


def _write_exec(p, body):
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(p, 0o755)


def _sha(b):
    return hashlib.sha256(b).hexdigest()


def git_blob_id(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def load_manifest():
    with open(MANIFEST, encoding="utf-8") as f:
        return json.load(f)


def user_rows(manifest=None):
    m = manifest or load_manifest()
    return [r for r in m["rows"] if r.get("owner_class") == "user" and r.get("file")]


def role_rows(role):
    return [r for r in user_rows() if role in r.get("roles", [])]


def rows_by_file():
    out = {}
    for r in user_rows():
        out.setdefault(r["file"], []).append(r)
    return out


def framed(row, overlay=""):
    return ("== %s · %s\n%s- old:\n%s\n- new:\n%s\n\n" % (row["id"], row["file"], overlay, row["old"], row["new"])
            ).encode("utf-8")


def block_of(out):
    """The self-service block: from its heading to the end of its held lines (two-space indented)."""
    i = out.find(BLOCK_HEAD)
    if i < 0:
        return None
    lines = out[i:].split("\n")
    keep = [lines[0]]
    for l in lines[1:]:
        if l.startswith("- ") or l.startswith("  "):
            keep.append(l)
        else:
            break
    return "\n".join(keep) + "\n"


class Rig(object):
    """A pack holding the user-class files (0.14.56 bytes or the shipped 0.14.57 bytes), the install record,
    the tool and the correction manifest; a home, a local dir and stub cys/ps/lsof. Hooks run from the repo."""

    def __init__(self, mode="056", pack_name="pack"):
        self.tmp = tempfile.mkdtemp(prefix="f58u2-")
        self.home = os.path.join(self.tmp, "home")
        self.local = os.path.join(self.tmp, "local")
        self.work = os.path.join(self.tmp, "work")
        self.stub = os.path.join(self.tmp, "stubbin")
        self.pack = os.path.join(self.tmp, pack_name)
        for d in (self.home, self.local, self.work, self.stub, os.path.join(self.pack, "memory"),
                  os.path.join(self.pack, "bin")):
            os.makedirs(d, exist_ok=True)
        shutil.copy(TOOL, os.path.join(self.pack, "bin", "javis_selfservice.py"))
        _write_bytes(os.path.join(self.pack, "round", "self-service-corrections.json"), _read_bytes(MANIFEST))
        _write_bytes(os.path.join(self.pack, "memory", "MEMORY.md"), b"MEMORY-INDEX\n")
        self.record = {}
        for rel, rows in rows_by_file().items():
            tip = _read_bytes(os.path.join(PACK, *rel.split("/")))
            data = tip
            if mode == "056":
                for r in reversed(rows):
                    n, o = r["new"].encode("utf-8"), r["old"].encode("utf-8")
                    assert data.count(n) == 1, "%s: new text occurs %d times in the shipped file" % (
                        r["id"], data.count(n))
                    data = data.replace(n, o, 1)
                blobs = {r["base_blob"] for r in rows}
                assert git_blob_id(data) in blobs, "%s rebuilt bytes are not the 31f87878 blob %s" % (rel, blobs)
                _write_bytes(os.path.join(self.pack, *(rel + ".new").split("/")), tip)
            _write_bytes(os.path.join(self.pack, *rel.split("/")), data)
            self.record[rel] = _sha(data)
        self.save_record()
        _write_exec(os.path.join(self.stub, "cys"),
                    "#!/bin/sh\ncase \"$1\" in\n"
                    "  reclaim-role) printf 'role=%s\\nreason=already_roled\\nenv_role=self\\ndetail=\\n' \"$CYS_ROLE\";"
                    " exit 0 ;;\n"
                    "esac\nexit 0\n")
        _write_exec(os.path.join(self.stub, "ps"), "#!/bin/sh\necho '9000 claude /x/claude'\n")
        _write_exec(os.path.join(self.stub, "lsof"), "#!/bin/sh\nprintf 'n%s\\n' \"$PWD\"\n")

    def save_record(self):
        _write_bytes(os.path.join(self.pack, ".install-manifest.json"),
                     json.dumps(self.record, indent=1).encode("utf-8"))

    def path(self, rel):
        return os.path.join(self.pack, *rel.split("/"))

    def rewrite(self, rel, data, record=False):
        _write_bytes(self.path(rel), data)
        if record:
            self.record[rel] = _sha(data)
            self.save_record()

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def env(self, role=None, surface=True):
        e = {"PATH": os.pathsep.join([self.stub, os.path.dirname(sys.executable), "/usr/bin", "/bin"]),
             "HOME": self.home, "CYS_PACK_DIR": self.pack, "CYS_LOCAL_DIR": self.local, "CYS_ROOT": self.tmp,
             "TMPDIR": self.tmp, "LANG": os.environ.get("LANG", "C.UTF-8")}
        if role is not None:
            e["CYS_ROLE"] = role
        if surface:
            e["CYS_SURFACE_ID"] = "3"
        return e

    def tool(self, role, *extra):
        r = subprocess.run([sys.executable, "-B", os.path.join(self.pack, "bin", "javis_selfservice.py"),
                            "corrections", "--role", role] + list(extra),
                           capture_output=True, env=self.env(), timeout=60)
        return r.returncode, r.stdout

    def tool_capped(self, cap, role, *extra):
        code = ("import importlib.util,sys\n"
                "s=importlib.util.spec_from_file_location('ss_under_test',sys.argv[1])\n"
                "m=importlib.util.module_from_spec(s);s.loader.exec_module(m)\n"
                "m.CORR_CAP=int(sys.argv[2])\n"
                "sys.exit(m.main(sys.argv[3:]))\n")
        r = subprocess.run([sys.executable, "-B", "-c", code, os.path.join(self.pack, "bin", "javis_selfservice.py"),
                            str(cap), "corrections", "--role", role] + list(extra),
                           capture_output=True, env=self.env(), timeout=60)
        return r.returncode, r.stdout

    def session_start(self, role, hook=SS_HOOK):
        r = subprocess.run(["sh", hook], capture_output=True, env=self.env(role), stdin=subprocess.DEVNULL,
                           timeout=120)
        return r.returncode, r.stdout.decode("utf-8", "replace")

    def inject(self, role, source="startup", hook=IC_HOOK, surface=False):
        payload = json.dumps({"source": source, "cwd": self.work})
        r = subprocess.run([BASH, hook], input=payload.encode("utf-8"), capture_output=True,
                           env=self.env(role, surface=surface), timeout=180)
        return r.returncode, r.stdout.decode("utf-8", "replace")

    def snapshot(self):
        snap = {}
        for rel in rows_by_file():
            snap[rel] = _sha(_read_bytes(self.path(rel)))
        for root, _dirs, files in os.walk(self.local):
            for n in files:
                p = os.path.join(root, n)
                snap[p] = _sha(_read_bytes(p))
        snap[".install-manifest.json"] = _sha(_read_bytes(os.path.join(self.pack, ".install-manifest.json")))
        return snap


def parse_listing(out_bytes):
    """(ids in order, {id: framed bytes}, end-line match or None, continuation or None)."""
    text = out_bytes.decode("utf-8")
    ids = [m.group(1) for m in HEADER_RE.finditer(text) if m.group(2) != "part"]
    end = END_RE.search(text)
    cont = re.search(r"^계속: --from (\S+)$", text, re.M)
    return ids, text, end, cont


def hook_copy(tmp, lib_transform):
    """A copy of hooks/ (and the owned-process leaf beside it) with a transformed _lib.sh."""
    root = os.path.join(tmp, "hookcopy")
    hd = os.path.join(root, "hooks")
    os.makedirs(hd, exist_ok=True)
    os.makedirs(os.path.join(root, "bin"), exist_ok=True)
    for n in os.listdir(HOOKS):
        p = os.path.join(HOOKS, n)
        if os.path.isfile(p):
            shutil.copy(p, os.path.join(hd, n))
    shutil.copy(OWNED, os.path.join(root, "bin", "_owned_process.py"))
    with open(LIB, encoding="utf-8") as f:
        src = f.read()
    with open(os.path.join(hd, "_lib.sh"), "w", encoding="utf-8", newline="\n") as f:
        f.write(lib_transform(src))
    return os.path.join(hd, "session-start.sh"), os.path.join(hd, "inject-context.sh")


def without_function(src):
    i = src.find("cys_self_service_note() {")
    assert i >= 0, "cys_self_service_note is not in _lib.sh"
    j = src.index("\n}\n", i) + 3
    return src[:i] + src[j:]


def without_pointer(src):
    """The pointer lines become no-ops (`:`), so the shell stays valid and only the pointer is missing."""
    out = "\n".join(("    :" if "제조사 기본값 정정" in l else l) for l in src.split("\n"))
    assert out != src, "no pointer line found in _lib.sh"
    return out


def step5_problems(out, role, n, session_start=True):
    """r2 §5.6 step 5 as amended by final §7.2. Returns the list of failed conditions (empty = pass)."""
    p = []
    if out.count(BLOCK_HEAD) != 1:
        p.append("block count %d" % out.count(BLOCK_HEAD))
    ms = POINTER_RE.findall(out)
    if len(ms) != 1:
        p.append("pointer lines %d" % len(ms))
    elif ms[0] != (str(n), role):
        p.append("pointer %r, want (%r, %r)" % (ms[0], str(n), role))
    if session_start:
        line = [l for l in out.split("\n") if "이미 있으면" in l and "다시 읽지 않는다" in l]
        if len(line) != 1:
            p.append("read-pointer lines %d" % len(line))
        else:
            end = out.find(line[0]) + len(line[0])
            if end >= 2000:
                p.append("read pointer ends at character %d" % end)
            if out.find(BLOCK_HEAD) < end:
                p.append("block before the read pointer")
    return p


def step6_problems(rig, role, n):
    """r2 §5.6 step 6: every row of the role printed with the shipped `new` text, the end line, count == N."""
    p = []
    rc, out = rig.tool(role)
    if rc != 0:
        return ["rc %d" % rc]
    ids, text, end, _cont = parse_listing(out)
    want = [r["id"] for r in role_rows(role)]
    if ids != want:
        p.append("ids %r != %r" % (ids, want))
    rig_rows = [r for r in json.loads(_read_bytes(os.path.join(rig.pack, "round",
                                                              "self-service-corrections.json")))["rows"]
                if r.get("owner_class") == "user" and role in r.get("roles", [])]
    for r in rig_rows:
        if framed(r) not in out:
            p.append("%s not printed as framed" % r["id"])
        shipped = _read_bytes(os.path.join(PACK, *r["file"].split("/")))
        if r["new"].encode("utf-8") not in shipped:
            p.append("%s printed `new` is not the shipped text" % r["id"])
    if not end:
        p.append("no end line")
    elif (int(end.group(1)), int(end.group(2))) != (n, n):
        p.append("end line %s/%s, want %d/%d" % (end.group(1), end.group(2), n, n))
    return p


class Base(unittest.TestCase):
    mode = "056"

    def setUp(self):
        self.rig = Rig(self.mode)

    def tearDown(self):
        self.rig.cleanup()


# ───────────────────────── T-12: R2-a..R2-d ─────────────────────────

class T12Precedence(Base):
    def test_r2a_stock_lists_every_role_row_and_writes_nothing(self):
        before = self.rig.snapshot()
        for role in ROLES:
            rc, out = self.rig.tool(role)
            self.assertEqual(rc, 0)
            ids, _text, end, cont = parse_listing(out)
            self.assertEqual(ids, [r["id"] for r in role_rows(role)], role)
            self.assertIsNone(cont, "%s fits one call (final §8.1)" % role)
            self.assertIsNotNone(end, "%s end line" % role)
            self.assertEqual((int(end.group(1)), int(end.group(2))), (len(ids), len(ids)))
        self.assertEqual(self.rig.snapshot(), before, "the channel wrote to a user file or the local dir")

    def _edit_master(self, data):
        rel = "directives/MASTER_DIRECTIVE.md"
        before = self.rig.snapshot()
        self.rig.rewrite(rel, data)  # the user edits; the install record keeps the installer's hash
        mid = self.rig.snapshot()
        self.assertNotEqual(before[rel], mid[rel])
        rc, out = self.rig.tool("master")
        self.assertEqual(rc, 0)
        ids, text, end, _ = parse_listing(out)
        held = [r["id"] for r in role_rows("master") if r["file"] == rel]
        self.assertTrue(held)
        self.assertFalse(set(ids) & set(held), "rows of the user-edited file were listed: %r" % (set(ids) & set(held)))
        others = [r["id"] for r in role_rows("master") if r["file"] != rel]
        self.assertEqual(ids, others, "rows of untouched files stay listed")
        self.assertEqual(int(end.group(2)), len(others))
        rc, summ = self.rig.tool("master", "--summary")
        lines = summ.decode("utf-8").splitlines()
        self.assertEqual(lines[0], str(len(others)))
        self.assertEqual(lines[1:], ["사용자가 고친 파일 %s: 이 역할의 제조사 정정 %d건은 %s.new 에 있다 · "
                                     "사용자 글이 우선한다" % (rel, len(held), rel)])
        rc, ss = self.rig.session_start("master")
        self.assertEqual(rc, 0)
        self.assertIn("사용자가 고친 파일 %s" % rel, block_of(ss) or "")
        self.assertEqual(self.rig.snapshot(), mid, "the channel or the hook wrote to a user file")
        return text

    def test_r2b_modified_clause_holds_the_whole_file(self):
        rel = "directives/MASTER_DIRECTIVE.md"
        data = _read_bytes(self.rig.path(rel))
        old = [r for r in role_rows("master") if r["id"] == "T1-01+T2-01"][0]["old"].encode("utf-8")
        self.assertEqual(data.count(old), 1)
        self._edit_master(data.replace(old, old[:-1] + b" ", 1))

    def test_r2c_appended_restriction_holds_the_whole_file(self):
        rel = "directives/MASTER_DIRECTIVE.md"
        data = _read_bytes(self.rig.path(rel))
        text = self._edit_master(data + ("\n" + RESTRICTION + "\n").encode("utf-8"))
        t101 = [r for r in role_rows("master") if r["id"] == "T1-01+T2-01"][0]
        self.assertNotIn(t101["new"], text, "the new auto-delivery default was presented over the restriction")

    def test_r2d_overlay_marks_every_row_and_stays_injected(self):
        ov = os.path.join(self.rig.local, "directives", "MASTER_DIRECTIVE.local.md")
        _write_bytes(ov, (RESTRICTION + "\n").encode("utf-8"))
        before = self.rig.snapshot()
        rc, out = self.rig.tool("master")
        self.assertEqual(rc, 0)
        ids, text, end, _ = parse_listing(out)
        self.assertEqual(ids, [r["id"] for r in role_rows("master")])
        self.assertEqual(text.count(OVERLAY_TEXT), len(ids), "every presented row carries the overlay line")
        for chunk in text.split("== ")[1:]:
            self.assertIn("— 그 지침이 이 정정보다 우선한다\n", chunk.split("- old:")[0])
        rc, ss = self.rig.session_start("master")
        self.assertEqual(rc, 0)
        self.assertIn("■ 사용자 로컬 지침", ss)
        self.assertIn(RESTRICTION, ss, "the overlay is still injected (reader unchanged)")
        rc, other = self.rig.tool("worker")
        self.assertNotIn(OVERLAY_TEXT, other.decode("utf-8"), "a master overlay does not mark worker rows")
        self.assertEqual(self.rig.snapshot(), before)


# ───────────────────────── gates, summary, roles ─────────────────────────

class Gates(Base):
    def test_role_prefixes_and_unknown_role(self):
        self.assertEqual(self.rig.tool("worker-59", "--summary")[1].decode().splitlines()[0],
                         str(len(role_rows("worker"))))
        self.assertEqual(self.rig.tool("cso-2", "--summary")[1].decode().splitlines()[0],
                         str(len(role_rows("cso"))))
        self.assertEqual(self.rig.tool("reviewer-codex", "--summary")[1].decode().splitlines()[0],
                         str(len(role_rows("reviewer"))))
        for bad in ("ceo", "masterx", "", "Worker"):
            self.assertEqual(self.rig.tool(bad)[0], 2, bad)

    def test_unreadable_or_odd_install_record_holds_every_file(self):
        for content in (b"{not json", b"[1, 2]", b""):
            _write_bytes(os.path.join(self.rig.pack, ".install-manifest.json"), content)
            rc, out = self.rig.tool("master")
            self.assertEqual(rc, 0)
            ids, _t, end, _ = parse_listing(out)
            self.assertEqual(ids, [])
            self.assertEqual(end.group(2), "0")
            rc, summ = self.rig.tool("master", "--summary")
            lines = summ.decode("utf-8").splitlines()
            self.assertEqual(lines[0], "0")
            self.assertEqual(lines[1:], ["설치 기록(.install-manifest.json)을 읽지 못해 정정을 보이지 않는다 · "
                                         "사용자 글이 우선한다"])
        os.remove(os.path.join(self.rig.pack, ".install-manifest.json"))
        self.assertEqual(parse_listing(self.rig.tool("worker")[1])[0], [])

    def test_missing_record_key_holds_only_that_file(self):
        rel = "directives/RSI_LEARNING_DIRECTIVE.md"
        del self.rig.record[rel]
        self.rig.save_record()
        ids = parse_listing(self.rig.tool("worker")[1])[0]
        self.assertEqual(ids, [r["id"] for r in role_rows("worker") if r["file"] != rel])
        self.rig.record[rel] = "0" * 64
        self.rig.save_record()
        self.assertEqual(parse_listing(self.rig.tool("worker")[1])[0], ids, "a wrong hash holds the file too")

    def test_row_gate_old_exactly_once(self):
        rel = "directives/WORKER_DIRECTIVE.md"
        row = [r for r in role_rows("worker") if r["file"] == rel][0]
        data = _read_bytes(self.rig.path(rel)) + b"\n" + row["old"].encode("utf-8") + b"\n"
        self.rig.rewrite(rel, data, record=True)  # as if the installer had shipped a file with the text twice
        ids = parse_listing(self.rig.tool("worker")[1])[0]
        self.assertNotIn(row["id"], ids)
        self.assertEqual(ids, [r["id"] for r in role_rows("worker") if r["id"] != row["id"]])

    def test_row_gate_new_already_present(self):
        rel = "directives/WORKER_DIRECTIVE.md"
        row = [r for r in role_rows("worker") if r["file"] == rel and r["op"] == "replace"][0]
        data = _read_bytes(self.rig.path(rel)) + b"\n" + row["new"].encode("utf-8") + b"\n"
        self.rig.rewrite(rel, data, record=True)
        self.assertNotIn(row["id"], parse_listing(self.rig.tool("worker")[1])[0])

    def test_corrupt_correction_manifest_is_exit_2(self):
        _write_bytes(os.path.join(self.rig.pack, "round", "self-service-corrections.json"), b"{")
        self.assertEqual(self.rig.tool("master")[0], 2)
        self.assertEqual(self.rig.tool("master", "--summary")[0], 2)

    def test_per_role_bytes_match_final_8_1(self):
        for role in ROLES:
            rc, out = self.rig.tool(role)
            want = b"".join(framed(r) for r in role_rows(role))
            self.assertTrue(out.startswith(want), role)
            self.assertEqual(len(out), len(want) + len(out.split(b"\n")[-2]) + 1, role)
            if role in DESIGN_TOTALS:
                self.assertEqual(len(out), DESIGN_TOTALS[role], "%s total vs final §8.1" % role)
            print("final 8.1 %s: %d rows, %d B" % (role, len(role_rows(role)), len(out)))

    def test_end_line_hashes(self):
        rc, out = self.rig.tool("master")
        end = END_RE.search(out.decode("utf-8"))
        ids = [r["id"] for r in role_rows("master")]
        self.assertEqual(end.group(3), _sha("\n".join(ids).encode("utf-8"))[:16])
        self.assertEqual(end.group(4), _sha(_read_bytes(MANIFEST))[:16])


class Fresh057(Base):
    mode = "057"

    def test_new_install_has_nothing_to_present(self):
        prefix = [r for r in user_rows() if r["op"] == "prefix"]
        self.assertTrue(prefix)
        for r in prefix:  # D-U2-1: the old text is still there, inside the new text
            self.assertEqual(_read_bytes(self.rig.path(r["file"])).count(r["old"].encode("utf-8")), 1, r["id"])
        for role in ROLES:
            rc, out = self.rig.tool(role)
            self.assertEqual(rc, 0)
            text = out.decode("utf-8")
            self.assertTrue(re.fullmatch(END_RE.pattern + r"\n", text, re.M), "%s: %r" % (role, text))
            self.assertTrue(text.startswith("끝: 0/0 건"), text)
            self.assertEqual(self.rig.tool(role, "--summary")[1], b"0\n")
            rc, ss = self.rig.session_start(role)
            self.assertEqual(step5_problems(ss, role, 0), [], role)


# ───────────────────────── paging (final §8.2) ─────────────────────────

class Paging(Base):
    def _chain(self, cap, role):
        calls, frm = [], None
        for _ in range(100):
            extra = ("--from", frm) if frm else ()
            rc, out = self.rig.tool_capped(cap, role, *extra)
            self.assertEqual(rc, 0)
            self.assertLessEqual(len(out), cap, "a call printed more than the cap")
            calls.append(out)
            m = re.search(rb"^\xea\xb3\x84\xec\x86\x8d: --from (\S+)\n\Z", out, re.M)
            if not m:
                break
            frm = m.group(1).decode("utf-8")
        return calls

    def _rebuild(self, calls):
        body = b""
        for i, out in enumerate(calls):
            lines = out.split(b"\n")
            if i < len(calls) - 1:
                self.assertIsNone(END_RE.search(out.decode("utf-8")), "end line before the last call")
                out = out[:out.rindex("계속:".encode("utf-8"))]
            else:
                out = out[:out.rindex("끝:".encode("utf-8"))]
            m = re.match(rb"== (\S+) \xc2\xb7 part (\d+)/(\d+)\n", out)
            if m:  # a part call holds one piece and its one separator newline (final §8.2 as built)
                out = out[m.end():]
                self.assertTrue(out.endswith(b"\n"))
                out = out[:-1]
            body += out
        return body

    def test_whole_row_paging_reconstructs_the_list(self):
        full = self.rig.tool("master")[1]
        calls = self._chain(3000, "master")
        self.assertGreater(len(calls), 2)
        self.assertEqual(self._rebuild(calls), full[:full.rindex("끝:".encode("utf-8"))])
        end = END_RE.search(calls[-1].decode("utf-8"))
        self.assertEqual(end.group(0), END_RE.search(full.decode("utf-8")).group(0))

    def test_a_row_larger_than_the_cap_is_printed_in_parts(self):
        full = self.rig.tool("master")[1]
        calls = self._chain(1500, "master")
        parts = [c for c in calls if re.match(rb"== \S+ \xc2\xb7 part \d+/\d+\n", c)]
        self.assertGreaterEqual(len(parts), 2, "the 2,206 B row must split at cap 1,500")
        self.assertTrue(any(re.search(rb"--from \S+:\d+\n\Z", c) for c in calls))
        self.assertEqual(self._rebuild(calls), full[:full.rindex("끝:".encode("utf-8"))])
        for c in calls:
            c.decode("utf-8")  # every call cuts at a character boundary

    def test_a_call_that_prints_a_last_part_stops_after_it(self):
        # U2 mutant P08 (2026-11-04): a call that printed the last part of a split row and then went on with whole
        # rows passed every other test, because at cap 1,500 the row after the split row never fits beside its last
        # part. The reader takes everything between the part header and the call's last line as the piece (final
        # §8.2 as built, _rebuild above), so that call must end right after the piece with the next row's
        # continuation line (or the end line). The witness below proves at least one cap left room for the next
        # row under the tool's own rule (len + next frame <= cap - CORR_TAIL), so this cannot pass vacuously.
        with open(TOOL, encoding="utf-8") as f:
            tail = int(re.search(r"^CORR_TAIL = (\d+)", f.read(), re.M).group(1))
        ids = [r["id"] for r in role_rows("master")]
        full = self.rig.tool("master")[1]
        heads = [m.start() for m in re.finditer(rb"^== \S+ \xc2\xb7 \S+\n", full, re.M)]
        heads.append(full.rindex("끝:".encode("utf-8")))
        frames = {}
        for a, b in zip(heads, heads[1:]):
            frames[re.match(rb"== (\S+) ", full[a:]).group(1).decode("utf-8")] = full[a:b]
        self.assertEqual(sorted(frames), sorted(ids), "every master row has one frame in the one-call listing")
        roomy, seen = [], 0
        for cap in (1800, 2100, 2400):
            for c in self._chain(cap, "master"):
                m = re.match(rb"== (\S+) \xc2\xb7 part (\d+)/(\d+)\n", c)
                if not m or m.group(2) != m.group(3):
                    continue
                seen += 1
                k = ids.index(m.group(1).decode("utf-8"))
                body = c[:c.rindex(b"\n", 0, len(c) - 1) + 1]  # the call without its last line
                if k + 1 == len(ids):
                    self.assertIsNotNone(END_RE.search(c.decode("utf-8")),
                                         "cap %d: the last row's last part ends the list" % cap)
                    continue
                nxt = ids[k + 1]
                if len(body) + len(frames[nxt]) <= cap - tail:
                    roomy.append((cap, nxt))
                self.assertTrue(c.endswith(("계속: --from %s\n" % nxt).encode("utf-8")),
                                "cap %d: the call that printed the last part of %s went on: %r" % (cap, ids[k], c[-80:]))
        self.assertGreater(seen, 0, "no cap split a row")
        self.assertTrue(roomy, "no cap left room for the next row beside a last part: the check would be vacuous")

    def test_unknown_from_is_exit_2(self):
        self.assertEqual(self.rig.tool("master", "--from", "NO-SUCH")[0], 2)
        self.assertEqual(self.rig.tool("master", "--from", "T1-01+T2-01:9")[0], 2)
        self.assertEqual(self.rig.tool("master", "--from", "T1-01+T2-01:0")[0], 2)
        self.assertEqual(self.rig.tool("worker", "--from", "T1-01+T2-01")[0], 2, "a master row is not in the worker list")


# ───────────────────────── T-10p and T-4: the hooks ─────────────────────────

class Hooks(Base):
    def test_t10p_session_start_block_for_every_role(self):
        for role in ROLES:
            rc, out = self.rig.session_start(role)
            self.assertEqual(rc, 0, role)
            n = len(role_rows(role))
            self.assertEqual(step5_problems(out, role, n), [], role)
            self.assertEqual(step6_problems(self.rig, role, n), [], role)
            self.assertGreater(out.find(BLOCK_HEAD), out.find("MEMORY-INDEX"), "block after the memory index")

    def test_t10p_inject_context_startup_and_clear(self):
        for role in ("master", "worker-59", "cso-1", "reviewer-codex"):  # the hook's prefix rules, not exact names
            base = role.split("-")[0]
            n = len(role_rows(base))
            blocks = []
            for src in ("startup", "clear"):
                rc, out = self.rig.inject(role, src)
                self.assertEqual(rc, 0, (role, src))
                self.assertEqual(step5_problems(out, base, n, session_start=False), [], (role, src))
                blk = block_of(out)
                self.assertTrue(out.rstrip("\n").endswith(blk.rstrip("\n")), "block ends the payload (%s %s)" % (role, src))
                blocks.append(blk)
            self.assertEqual(blocks[0], blocks[1], "the same block after a clear (step 7)")

    def test_t10p_negative_control_pointer_removed(self):
        ss, ic = hook_copy(self.rig.tmp, without_pointer)
        rc, out = self.rig.session_start("master", ss)
        self.assertEqual(rc, 0)
        self.assertNotEqual(step5_problems(out, "master", len(role_rows("master"))), [])
        rc, out = self.rig.inject("master", "startup", ic)
        self.assertNotEqual(step5_problems(out, "master", len(role_rows("master")), session_start=False), [])

    def test_t10p_negative_control_altered_new(self):
        p = os.path.join(self.rig.pack, "round", "self-service-corrections.json")
        m = json.loads(_read_bytes(p))
        row = [r for r in m["rows"] if r["id"] == "T1-14"][0]
        row["new"] = row["new"] + " (altered)"
        _write_bytes(p, json.dumps(m, ensure_ascii=False).encode("utf-8"))
        self.assertNotEqual(step6_problems(self.rig, "master", len(role_rows("master"))), [])

    def test_t4_old_lib_without_the_function(self):
        ss, ic = hook_copy(self.rig.tmp, without_function)
        rc, out = self.rig.session_start("worker", ss)
        self.assertEqual(rc, 0)
        self.assertIn("■ CYSJavis 역할 각성", out)
        self.assertNotIn(BLOCK_HEAD, out)
        rc, out = self.rig.inject("worker", "startup", ic)
        self.assertEqual(rc, 0)
        self.assertNotIn(BLOCK_HEAD, out)

    def test_t4_worst_case_size_and_text(self):
        for rel in ("directives/MASTER_DIRECTIVE.md", "directives/RSI_LEARNING_DIRECTIVE.md", "soul.md"):
            self.rig.rewrite(rel, _read_bytes(self.rig.path(rel)) + b"\nuser line\n")
        rc, out = self.rig.session_start("master")
        blk = block_of(out)
        self.assertEqual(blk.count("사용자가 고친 파일"), 3)
        self.assertEqual(step5_problems(out, "master", 0), [])
        rc, unknown = self.rig.inject(None, "startup")
        ublk = block_of(unknown)
        self.assertIn("--role <ROLE>`", ublk)
        self.assertEqual(POINTER_RE.findall(unknown), [("", "<ROLE>")])
        for b in (blk, ublk):
            self.assertLessEqual(len(b.encode("utf-8")), 1600, "block %d B" % len(b.encode("utf-8")))
            for w in FORBIDDEN_IN_BLOCK:
                self.assertNotIn(w, b, w)
        print("T-4 worst case: %d B (3 held lines), unknown role %d B" % (len(blk.encode("utf-8")),
                                                                         len(ublk.encode("utf-8"))))

    def test_t4_block_before_the_team_block(self):
        rig = Rig("056", "pack-dept-dept-7")
        try:
            _write_bytes(os.path.join(rig.pack, "TEAM.md"), "# 팀 소개 (참고 정보)\n- 팀 이름: 시험팀\n".encode("utf-8"))
            rc, out = rig.session_start("worker")
            self.assertEqual(rc, 0)
            i, j = out.find(BLOCK_HEAD), out.find("■ 팀 소개")
            self.assertGreater(i, 0)
            self.assertGreater(j, i, "the F58 block comes before the U16 team block")
            self.assertEqual(step5_problems(out, "worker", len(role_rows("worker"))), [])
        finally:
            rig.cleanup()

    def test_t4_fail_open_tool_missing_or_failing(self):
        tool = os.path.join(self.rig.pack, "bin", "javis_selfservice.py")
        _write_bytes(os.path.join(self.rig.pack, "round", "self-service-corrections.json"), b"{")
        rc, out = self.rig.session_start("worker")
        self.assertEqual(rc, 0)
        self.assertEqual(POINTER_RE.findall(out), [("", "worker")])
        os.remove(tool)
        rc, out = self.rig.session_start("worker")
        self.assertEqual(rc, 0)
        self.assertEqual(POINTER_RE.findall(out), [("", "worker")])
        rc, out = self.rig.inject("worker", "startup")
        self.assertEqual(rc, 0)
        self.assertEqual(POINTER_RE.findall(out), [("", "worker")])

    def test_t4_fail_open_tool_hangs(self):
        _write_exec(os.path.join(self.rig.pack, "bin", "javis_selfservice.py"),
                    "import time\ntime.sleep(60)\n")
        t0 = time.time()
        rc, out = self.rig.session_start("worker")
        dt = time.time() - t0
        self.assertEqual(rc, 0)
        self.assertEqual(POINTER_RE.findall(out), [("", "worker")])
        self.assertLess(dt, 45, "the N call is bounded (5 s cap), took %.1f s" % dt)


if __name__ == "__main__":
    unittest.main(verbosity=2)
