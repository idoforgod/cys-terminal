#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_guard_rows.py — F58 U6: the guard rows T1-27..T1-29 and the constitution-edit token route (T-13).

Design: DESIGN-F58-r2.md §6.1-6.7 (T-13a-e, T-13w), HEAD-DECISIONS-F58-r2.md H9, and the head decision D-U6-1
(2026-11-04): the H9 premise ("a dead token route is fail-closed") was refuted. Under GNU `stat` the token block of
guard.sh made the script die on `set -u` with exit 0 while ANY token file existed (fresh, empty or stale), so every
call was allowed. F58 U6 accepts a stat result only when it is all digits. T-13g pins that on every OS.

Rig: a temp dir with a COPY of the pack's hooks/ (CONSTITUTION_EDIT_AUTHORIZED is read from the folder of guard.sh,
so the token lands in the copy), a temp HOME and TMPDIR (POSIX; on Windows the runner's), CYS_SOCKET at a missing path (guard.sh then asks no daemon and starts
none), GUARD_TEST_MODE=1 with the autopilot flag files in the temp dir, cwd in the temp dir. guard.sh runs the way
the hook runs it: `bash guard.sh`, the hook JSON on stdin.

  T-13a  Write soul.md, no token -> exit 2, deny JSON, reason names CONSTITUTION_EDIT_AUTHORIZED and the Write tool
         (Write notes.md -> 0).
  T-13b  token first line soul.md, fresh -> Write soul.md 0, Write MASTER_DIRECTIVE.md 2; only the first line is the
         scope; '*' opens every protected name; the match ignores case.
  T-13c  emptied token ('' and '\\n') -> 2 with the T1-27 text (not the backstop text); token 1,801 s old -> 2
         (1,700 s old -> 0); AUTOPILOT_ACTIVE present -> 2 (absent -> 0); token as a symlink -> 2.
  T-13d  Bash `cat soul.md > /tmp/x` -> 2 with the Read-tool text (`cat soul.md` -> 0); Bash
         `rm <hooks>/CONSTITUTION_EDIT_AUTHORIZED` -> 2 (`ls` of it -> 0), with a fresh token present.
  T-13e  degraded mode -> Write soul.md 2 with the 축소 모드 text, Read soul.md 2 (the backstop ignores the tool),
         Write notes.md 0. Degraded mode is reached through the script's own no-python branch (GUARD_TEST_MODE=1 +
         GUARD_FORCE_NOPY=1, guard.sh:85) instead of an extracted function: the real wrapper runs end to end.
         The backstop reads the input with jq; without jq on PATH the class is skipped and says so.
  T-13g  the 16-cell table {token absent, fresh soul.md, empty, 2 h old} x {Write soul.md, Write MASTER_DIRECTIVE.md,
         Bash git push, Bash rm -rf} with a GNU-shaped stat MODEL first on PATH must equal EXPECTED_16, the table the
         real BSD stat gives. The model is NOT GNU coreutils: GNU `-f` takes no argument (GNU coreutils manual, stat
         invocation), so `stat -f %m F` reads %m as a missing file operand (rc 1) and prints F's file-system block;
         `stat -c %Y F` prints the mtime. A second model prints text for both forms (no digits from any form): the
         token must then never be honoured. Positive controls prove the model is the `stat` guard.sh resolves.
  Table16Real  the same 16 cells with the system's real stat (BSD on macOS; on the ubuntu lanes this is the real
         GNU measurement the model stands in for).
  T-13w  (Windows CI, report-only, H9) runs `T13b` only under Git Bash.

Cells whose expectation differs between guard.sh at 31f87878 and the F58 tip: the reason-text cells of T-13a, T-13d
and T-13e (T1-27..T1-29 are new texts) and T-13g (fail-open at 31f87878 and be304b8a). The exit codes of every other
cell are equal on both (r2 §6.4: 17 of 17).

Run: python3 cysjavis-pack/bin/tests/test_f58_guard_rows.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
HOOKS = os.path.join(PACK, "hooks")
BASH = shutil.which("bash") or "/bin/bash"
TOKEN_NAME = "CONSTITUTION_EDIT_AUTHORIZED"
DENY_PREFIX = "AUTOPILOT GUARD DENY: "
CRASH_MARKS = ("unbound variable", "syntax error")

GNU_MODEL = r'''#!/bin/bash
# MODEL of GNU coreutils stat for test_f58_guard_rows.py - NOT the real binary.
# GNU -f takes no argument, so in `stat -f %m F` the %m is a file operand: missing -> error on stderr, rc 1;
# F exists -> its file-system block on stdout. -c %Y F -> mtime in seconds.
rc=0
if [ "$1" = "-f" ]; then shift
  for op in "$@"; do
    if [ -e "$op" ]; then printf '  File: "%s"\n    ID: 0        Namelen: 255     Type: ext2/ext3\nBlock size: 4096       Fundamental block size: 4096\n' "$op"
    else printf "stat: cannot read file system information for '%s': No such file or directory\n" "$op" >&2; rc=1; fi
  done; exit $rc
fi
if [ "$1" = "-c" ] && [ "$2" = "%Y" ]; then
  exec "$F58_MODEL_PY" -c 'import os, sys; print(int(os.stat(sys.argv[1]).st_mtime))' "$3"
fi
printf 'model stat: unsupported arguments: %s\n' "$*" >&2; exit 2
'''

NODIGIT_MODEL = r'''#!/bin/bash
# MODEL of a stat that yields no digits in any form - NOT a real binary. Both forms print text and fail.
printf '  File: "%s"\n' "$*"; exit 1
'''

CALLS = ("W_SOUL", "W_DIR", "B_PUSH", "B_RMRF")
STATES = ("absent", "fresh", "empty", "stale")
# The decided table: only a fresh token scoped to soul.md opens Write soul.md; everything else is refused.
EXPECTED_16 = {(s, c): (0 if (s, c) == ("fresh", "W_SOUL") else 2) for s in STATES for c in CALLS}


def _write_exec(p, body):
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(p, 0o755)


class Res(object):
    def __init__(self, rc, out, err):
        self.rc, self.out, self.err = rc, out, err

    def reason(self):
        lines = [ln[len(DENY_PREFIX):] for ln in self.err.splitlines() if ln.startswith(DENY_PREFIX)]
        return lines[-1] if lines else None

    def __repr__(self):
        return "rc=%s stdout=%r stderr=%r" % (self.rc, self.out[-400:], self.err[-600:])


class Rig(object):
    def __init__(self):
        self.tmp = tempfile.mkdtemp(prefix="f58u6-")
        self.hooks = os.path.join(self.tmp, "hooks")
        shutil.copytree(HOOKS, self.hooks)
        self.guard = os.path.join(self.hooks, "guard.sh")
        self.token = os.path.join(self.hooks, TOKEN_NAME)
        if os.path.lexists(self.token):
            os.remove(self.token)
        self.home = os.path.join(self.tmp, "home")
        self.round = os.path.join(self.tmp, "round")
        self.proj = os.path.join(self.tmp, "proj")
        for d in (self.home, self.round, self.proj):
            os.makedirs(d)
        self.active = os.path.join(self.round, "AUTOPILOT_ACTIVE")
        self.paused = os.path.join(self.round, "AUTOPILOT_PAUSED")

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def path(self, name):
        return os.path.join(self.proj, name)

    def env(self, extra=None, path_front=None):
        if os.name == "nt":
            # Windows (T-13w only): the runner's own environment minus cys/Claude/guard variables, the way the H-WIN-1
            # specimen runs guard.sh (run_bootstrap_health.py _base_env); HOME and TMPDIR stay the runner's.
            e = {k: v for k, v in os.environ.items()
                 if not (k.startswith("CYS_") or k.startswith("CLAUDE") or k.startswith("GUARD_")
                         or k.startswith("AUTOPILOT_"))}
            base_path = e.get("PATH", "")
        else:
            e = {"LANG": os.environ.get("LANG", "C.UTF-8"), "HOME": self.home, "TMPDIR": self.tmp}
            base_path = os.pathsep.join([os.path.dirname(sys.executable), "/usr/bin", "/bin"])
        e.update({"CYS_SOCKET": os.path.join(self.tmp, "no.sock"),
                  "GUARD_TEST_MODE": "1", "AUTOPILOT_ACTIVE_FILE": self.active,
                  "AUTOPILOT_PAUSED_FILE": self.paused, "F58_MODEL_PY": sys.executable,
                  "PATH": os.pathsep.join(([path_front] if path_front else []) + [base_path])})
        if extra:
            e.update(extra)
        return e

    def run(self, payload, extra=None, path_front=None):
        p = subprocess.run([BASH, self.guard], input=json.dumps(payload).encode("utf-8"),
                           env=self.env(extra, path_front), cwd=self.round,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        return Res(p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace"))

    def bash_c(self, script, path_front=None):
        p = subprocess.run([BASH, "-c", script], env=self.env(None, path_front), cwd=self.round,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
        return Res(p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace"))

    def set_token(self, text, age_s=0):
        if os.path.lexists(self.token):
            os.remove(self.token)
        with open(self.token, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        if age_s:
            t = time.time() - age_s
            os.utime(self.token, (t, t))

    def clear_token(self):
        if os.path.lexists(self.token):
            os.remove(self.token)

    def set_state(self, state):
        if state == "absent":
            self.clear_token()
        elif state == "fresh":
            self.set_token("soul.md\n")
        elif state == "empty":
            self.set_token("")
        elif state == "stale":
            self.set_token("soul.md\n", age_s=7200)
        else:
            raise ValueError(state)

    def call(self, name):
        if name == "W_SOUL":
            return W(self.path("soul.md"))
        if name == "W_DIR":
            return W(self.path("MASTER_DIRECTIVE.md"))
        if name == "B_PUSH":
            return B("git push origin main")
        if name == "B_RMRF":
            return B("rm -rf " + self.path("old"))
        raise ValueError(name)

    def table(self, path_front=None):
        got, crashes = {}, {}
        for s in STATES:
            self.set_state(s)
            for c in CALLS:
                r = self.run(self.call(c), path_front=path_front)
                got[(s, c)] = r.rc
                if any(m in r.err for m in CRASH_MARKS):
                    crashes[(s, c)] = r.err.strip().splitlines()[-1][-160:]
        self.clear_token()
        return got, crashes


def W(path, tool="Write"):
    return {"tool_name": tool, "tool_input": {"file_path": path, "content": "x\n"}}


def B(cmd):
    return {"tool_name": "Bash", "tool_input": {"command": cmd}}


class GuardCase(unittest.TestCase):
    def setUp(self):
        self.rig = Rig()

    def tearDown(self):
        self.rig.close()

    def assertDeny(self, r, *needles):
        self.assertEqual(r.rc, 2, r)
        reason = r.reason()
        self.assertIsNotNone(reason, r)
        doc = json.loads(r.out.strip().splitlines()[-1])
        self.assertEqual(doc["hookSpecificOutput"]["permissionDecision"], "deny", r)
        if doc["hookSpecificOutput"]["permissionDecisionReason"] != "guard deny":  # no-jq fallback text
            self.assertEqual(doc["hookSpecificOutput"]["permissionDecisionReason"], reason, r)
        for n in needles:
            self.assertIn(n, reason, r)
        return reason

    def assertAllow(self, r):
        self.assertEqual(r.rc, 0, r)
        self.assertIsNone(r.reason(), r)
        for m in CRASH_MARKS:
            self.assertNotIn(m, r.err, r)


class T13a(GuardCase):
    def test_write_soul_without_token_is_held_with_the_route(self):
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))),
                        TOKEN_NAME, "Write 도구", "보류", "30분", "pre-ss-")

    def test_control_write_notes_without_token_passes(self):
        self.assertAllow(self.rig.run(W(self.rig.path("notes.md"))))

    def test_edit_tool_names_itself(self):
        r = self.rig.run(W(self.rig.path("soul.md"), tool="Edit"))
        self.assertIn("soul.md) Edit 보류", self.assertDeny(r, TOKEN_NAME))


class T13b(GuardCase):
    def test_fresh_scoped_token_opens_soul_only(self):
        self.rig.set_token("soul.md\n")
        self.assertAllow(self.rig.run(W(self.rig.path("soul.md"))))
        self.assertDeny(self.rig.run(W(self.rig.path("MASTER_DIRECTIVE.md"))), TOKEN_NAME)

    def test_only_the_first_line_is_the_scope(self):
        self.rig.set_token("MASTER_DIRECTIVE.md\nsoul.md\n")
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))), TOKEN_NAME)
        self.assertAllow(self.rig.run(W(self.rig.path("MASTER_DIRECTIVE.md"))))

    def test_star_opens_every_protected_name(self):
        self.rig.set_token("*\n")
        self.assertAllow(self.rig.run(W(self.rig.path("soul.md"))))
        self.assertAllow(self.rig.run(W(self.rig.path("MASTER_DIRECTIVE.md"))))

    def test_scope_match_ignores_case(self):
        self.rig.set_token("SOUL.MD\n")
        self.assertAllow(self.rig.run(W(self.rig.path("soul.md"))))


class T13c(GuardCase):
    def test_emptied_token_holds_with_the_route_text(self):
        for text in ("", "\n"):
            self.rig.set_token(text)
            reason = self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))), TOKEN_NAME, "보류")
            self.assertNotIn("백스톱", reason)

    def test_token_older_than_1800_s_is_not_honoured(self):
        self.rig.set_token("soul.md\n", age_s=1801)
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))), TOKEN_NAME)

    def test_control_token_1700_s_old_is_honoured(self):
        self.rig.set_token("soul.md\n", age_s=1700)
        self.assertAllow(self.rig.run(W(self.rig.path("soul.md"))))

    def test_autopilot_active_closes_the_route(self):
        self.rig.set_token("soul.md\n")
        with open(self.rig.active, "w") as f:
            f.write("1\n")
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))), TOKEN_NAME)
        os.remove(self.rig.active)
        self.assertAllow(self.rig.run(W(self.rig.path("soul.md"))))

    def test_symlinked_token_is_not_honoured(self):
        target = os.path.join(self.rig.tmp, "elsewhere-token")
        with open(target, "w") as f:
            f.write("soul.md\n")
        try:
            os.symlink(target, self.rig.token)
        except (OSError, NotImplementedError) as e:
            self.skipTest("symlink unavailable here: %s" % e)
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"))), TOKEN_NAME)


class T13d(GuardCase):
    def test_redirect_read_of_soul_is_refused_with_read_tool_text(self):
        self.assertDeny(self.rig.run(B("cat soul.md > /tmp/x")), "[LOOSE] ", "Read 도구", "Write/Edit 도구")

    def test_control_plain_read_of_soul_passes(self):
        self.assertAllow(self.rig.run(B("cat soul.md")))

    def test_rm_of_the_token_is_refused(self):
        self.rig.set_token("soul.md\n")
        self.assertDeny(self.rig.run(B("rm " + self.rig.token)), "[LOOSE] ", "rm (비가역 파일연산)")

    def test_control_ls_of_the_token_passes(self):
        self.rig.set_token("soul.md\n")
        self.assertAllow(self.rig.run(B("ls " + self.rig.token)))


class T13e(GuardCase):
    NOPY = {"GUARD_FORCE_NOPY": "1"}

    def setUp(self):
        super(T13e, self).setUp()
        if shutil.which("jq", path=self.rig.env()["PATH"]) is None:
            self.skipTest("jq not on PATH: the bash backstop reads the hook input with jq (r2 §6.1)")

    def test_degraded_mode_refuses_soul_with_the_cause(self):
        r = self.rig.run(W(self.rig.path("soul.md")), extra=self.NOPY)
        self.assertDeny(r, "축소 모드", "python3")
        self.assertIn("bash 백스톱", r.err)

    def test_degraded_mode_ignores_the_tool(self):
        self.assertDeny(self.rig.run(W(self.rig.path("soul.md"), tool="Read"), extra=self.NOPY), "축소 모드")

    def test_control_degraded_mode_passes_notes(self):
        self.assertAllow(self.rig.run(W(self.rig.path("notes.md")), extra=self.NOPY))


class T13g(GuardCase):
    """GNU-shaped stat model first on PATH: the decided table, and no crash in any cell (head D-U6-1)."""

    def _model(self, body):
        d = os.path.join(self.rig.tmp, "model-bin")
        os.makedirs(d, exist_ok=True)
        _write_exec(os.path.join(d, "stat"), body)
        return d

    def _assert_model_is_resolved(self, d):
        r = self.rig.bash_c('command -v stat', path_front=d)
        if os.name == "nt":  # Git Bash prints its own path form; T-13w does not run this class
            self.assertTrue(r.out.strip().endswith("/model-bin/stat"), r)
        else:
            self.assertEqual(r.out.strip(), os.path.join(d, "stat"), r)

    def test_gnu_model_positive_controls(self):
        d = self._model(GNU_MODEL)
        self._assert_model_is_resolved(d)
        f = os.path.join(self.rig.tmp, "probe")
        with open(f, "w") as fh:
            fh.write("x")
        r = self.rig.bash_c('stat -f %m "' + f + '"', path_front=d)
        self.assertEqual(r.rc, 1, r)
        self.assertTrue(r.out.startswith("  File: "), r)
        r = self.rig.bash_c('stat -c %Y "' + f + '"', path_front=d)
        self.assertEqual(r.rc, 0, r)
        self.assertEqual(int(r.out.strip()), int(os.stat(f).st_mtime), r)

    def test_gnu_model_table_equals_the_decided_table(self):
        got, crashes = self.rig.table(path_front=self._model(GNU_MODEL))
        bad = {k: (got[k], EXPECTED_16[k]) for k in EXPECTED_16 if got[k] != EXPECTED_16[k]}
        self.assertEqual(bad, {}, "cells (state, call): (got, expected); crashes: %r" % crashes)
        self.assertEqual(crashes, {})

    def test_stat_without_digits_never_honours_the_token(self):
        d = self._model(NODIGIT_MODEL)
        self._assert_model_is_resolved(d)
        self.rig.set_token("soul.md\n")
        r = self.rig.run(W(self.rig.path("soul.md")), path_front=d)
        self.assertDeny(r, TOKEN_NAME)
        for m in CRASH_MARKS:
            self.assertNotIn(m, r.err, r)
        self.assertDeny(self.rig.run(B("git push origin main"), path_front=d), "[LOOSE] ")


class Table16Real(GuardCase):
    """The same 16 cells with the system's own stat (BSD on macOS, GNU on the ubuntu lanes)."""

    def test_real_stat_table_equals_the_decided_table(self):
        got, crashes = self.rig.table()
        bad = {k: (got[k], EXPECTED_16[k]) for k in EXPECTED_16 if got[k] != EXPECTED_16[k]}
        self.assertEqual(bad, {}, "cells (state, call): (got, expected); crashes: %r" % crashes)
        self.assertEqual(crashes, {})


if __name__ == "__main__":
    unittest.main(verbosity=2)
