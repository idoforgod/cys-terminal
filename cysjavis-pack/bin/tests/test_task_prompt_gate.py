#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_task_prompt_gate.py — 0.15.0 「스스로 해결」 T7b: 위임 티켓을 짓는 모든 진입점이 작업 배정 관문을 묻는다.

계약(C11 O-C11-3 · RULINGS I1 / I7 / Q4 / K6 / H3 / AU6 / AR4 · HH7): `task-prompt`(누가 부르든)와 `phase-plan` 은
티켓이 **향하는 레인**의 `cys repair gate --lane <key> --json` 을 묻는다 — 열림이면 티켓이 종전과 같고, 보류면
일반 작업 티켓을 내지 않고 까닭을 말하며, 수리 티켓(`--repair` · `--merge` · `--owner-token`)은 언제나 낸다.
판독 자체를 할 수 없으면(cys 부재 · 형식 밖 답 · 시간 초과) 일반 티켓을 보류하고 까닭을 말한다(I1 (5) — 시간이
지났다고 열리지 않는다). 병합 티켓의 명령 줄은 `--files` 와 R 자신의 `--trigger` 를 싣고, CLI 가 그대로 받는다.
`--owner-token` 은 정확히 한 줄을 싣는다. HQ 레인에서는 충돌 요약 한 줄을 요약마다 한 번 낸다(표식은 이 도구의
상태 파일 — 수리 장부 밖). 가짜 `cys` 를 PATH 앞에 두어 데몬도 실제 장부도 건드리지 않는다.

    python3 cysjavis-pack/bin/tests/test_task_prompt_gate.py
"""
import ast
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import types
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_orchestra as orch  # noqa: E402

GATE_OPEN = {"lane": "x", "verdict": "open", "cause": None, "conflict_summary": None}
GATE_HELD = {"lane": "x", "verdict": "held",
             "cause": {"kind": "pending", "text": "이 부서가 아직 따라잡지 못한 수리가 있습니다(대기 1건 · 충돌 0건)",
                       "pending": 1, "conflict": 0, "rows": None},
             "conflict_summary": None}


def make_stub(d, gate_reply, gate_rc, status_reply=None, log=None):
    """A fake `cys` that answers `repair gate` and `repair status` and records its argv."""
    os.makedirs(d, exist_ok=True)
    payload = os.path.join(d, "replies.json")
    with open(payload, "w", encoding="utf-8") as f:
        json.dump({"gate": gate_reply, "gate_rc": gate_rc, "status": status_reply, "log": log}, f)
    script = os.path.join(d, "cys_stub.py")
    with open(script, "w", encoding="utf-8") as f:
        f.write(
            "import json, sys\n"
            "r = json.load(open(%r, encoding='utf-8'))\n"
            "if r['log']:\n"
            "    open(r['log'], 'a', encoding='utf-8').write(json.dumps(sys.argv[1:]) + '\\n')\n"
            "a = sys.argv[1:]\n"
            "if a[:2] == ['repair', 'gate']:\n"
            "    print(json.dumps(r['gate'])); sys.exit(r['gate_rc'])\n"
            "if a[:2] == ['repair', 'status'] and r['status'] is not None:\n"
            "    print(json.dumps(r['status'])); sys.exit(0)\n"
            "sys.exit(3)\n" % payload)
    if os.name == "nt":
        with open(os.path.join(d, "cys.cmd"), "w", encoding="utf-8") as f:
            f.write('@"%s" "%s" %%*\r\n' % (sys.executable, script))
    else:
        p = os.path.join(d, "cys")
        with open(p, "w", encoding="utf-8") as f:
            f.write("#!/bin/sh\nexec %s %s \"$@\"\n" % (sys.executable, script))
        os.chmod(p, 0o755)
    return d


@contextlib.contextmanager
def path_with(d):
    old = os.environ.get("PATH", "")
    os.environ["PATH"] = (d + os.pathsep + old) if d else "/nonexistent-cys-free"
    try:
        yield
    finally:
        os.environ["PATH"] = old


def run_tp(**kw):
    args = types.SimpleNamespace(task="T", scope="S", success="C", to="worker", surface=None,
                                 no_survival_gate=True, repair=None, merge=None, owner_token=None,
                                 owner_token_path=None)
    for k, v in kw.items():
        setattr(args, k, v)
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = orch.cmd_task_prompt(args)
    return rc, out.getvalue(), err.getvalue()


def run_pp(**kw):
    args = types.SimpleNamespace(task="T", scope="S", success="C", to="worker", phases="설계;구현",
                                 repair=None)
    for k, v in kw.items():
        setattr(args, k, v)
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        rc = orch.cmd_phase_plan(args)
    return rc, out.getvalue(), err.getvalue()


class TicketGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="t7b-gate-")
        self.pack = os.path.join(self.tmp, "pack")
        os.makedirs(os.path.join(self.pack, "state"))
        self._env = {k: os.environ.get(k) for k in ("CYS_PACK_DIR", "CYS_SOCKET", "CYS_DEPTS_JSON", "CYS_LOCAL_DIR")}
        os.environ["CYS_PACK_DIR"] = self.pack
        os.environ.pop("CYS_SOCKET", None)
        os.environ["CYS_DEPTS_JSON"] = os.path.join(self.tmp, "depts.json")
        os.environ["CYS_LOCAL_DIR"] = os.path.join(self.tmp, "local")
        self.log = os.path.join(self.tmp, "argv.log")

    def tearDown(self):
        for k, v in self._env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def stub(self, gate, rc, status=None):
        return make_stub(os.path.join(self.tmp, "bin"), gate, rc, status, self.log)

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log, encoding="utf-8") as f:
            return [json.loads(x) for x in f if x.strip()]

    # T-3 · I7 (1)/(2): OPEN → the ticket is exactly the ticket built without the gate (stdout unchanged)
    def test_open_gate_leaves_the_ticket_unchanged(self):
        with path_with(self.stub(GATE_OPEN, 0)):
            rc, out, err = run_tp()
        self.assertEqual(rc, 0, err)
        rules = orch.extract_worker_rules() or orch.FALLBACK_RULES
        prereq = orch.resolve_prereq_block(orch._split_csv(None), orch._split_csv(None), os.path.join(self.pack, "memory"))
        self.assertEqual(out, orch.build_task_ticket("T", "S", "C", "worker", rules=rules, prereq_block=prereq,
                                                    probes=orch._split_csv(None)) + "\n")
        self.assertIn("작업 배정 관문: 열림", err)
        self.assertEqual(self.calls()[0][:4], ["repair", "gate", "--lane", "base"])

    # T-1 · I1: HELD → an ordinary ticket is refused, the cause said, nothing on stdout
    def test_held_gate_refuses_an_ordinary_ticket_and_says_why(self):
        with path_with(self.stub(GATE_HELD, 95)):
            rc, out, err = run_tp()
        self.assertEqual(rc, 1)
        self.assertEqual(out, "")
        self.assertIn("보류", err)
        self.assertIn("대기 1건", err)

    # T-2 · H3: a repair ticket always goes out (held or not)
    def test_repair_tickets_pass_a_held_gate(self):
        with path_with(self.stub(GATE_HELD, 95)):
            rc, out, err = run_tp(repair="R-3")
        self.assertEqual(rc, 0, err)
        self.assertIn("[작업 위임", out)
        self.assertIn("수리 티켓이라 낸다", err)

    # T-4 · I1 (5): the local read cannot run → ordinary tickets held, saying why; a repair ticket goes out
    def test_an_unreadable_gate_holds_ordinary_tickets(self):
        cases = [
            ("no cys", None),
            ("exit 2", (GATE_OPEN, 2)),
            ("open with 95", (GATE_OPEN, 95)),
            ("not json", ("garbage", 0)),
        ]
        for name, stub in cases:
            d = None
            if stub is not None:
                d = make_stub(os.path.join(self.tmp, "b-%s" % name.replace(" ", "_")), stub[0], stub[1])
            with path_with(d):
                rc, out, err = run_tp()
                rc2, out2, _ = run_tp(repair="R-1")
            self.assertEqual(rc, 1, name)
            self.assertEqual(out, "", name)
            self.assertIn("확인할 수 없어", err, name)
            self.assertEqual(rc2, 0, name)
            self.assertIn("[작업 위임", out2, name)

    # T-5 · Q4: HQ building a ticket for a department asks THAT department's gate
    def test_a_ticket_for_a_department_asks_the_departments_gate(self):
        sock = os.path.join(self.tmp, "state", "cys-dept-dept-a", "cys.sock")
        with open(os.environ["CYS_DEPTS_JSON"], "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-a": {"socket": sock}}}, f)
        import javis_lane
        want = javis_lane.lane_key(sock)
        self.assertNotEqual(want, "base")
        with path_with(self.stub(GATE_OPEN, 0)):
            rc, _, err = run_tp(surface="dept-a/surface:5")
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.calls()[0][:4], ["repair", "gate", "--lane", want])
        # a department the registry does not know → not resolvable → held (never "open by default")
        with path_with(self.stub(GATE_OPEN, 0)):
            rc, out, err = run_tp(surface="dept-zz/surface:5")
        self.assertEqual((rc, out), (1, ""))
        self.assertIn("dept-zz", err)

    # T-6 · Q4: phase-plan asks the same gate
    def test_phase_plan_is_gated_too(self):
        with path_with(self.stub(GATE_HELD, 95)):
            rc, out, err = run_pp()
            rc2, out2, _ = run_pp(repair="R-2")
        self.assertEqual((rc, out), (1, ""))
        self.assertIn("보류", err)
        self.assertEqual(rc2, 0)
        self.assertIn("════════ P1", out2)

    # T-8 · K6 / T4's hand-over: exactly ONE owner-token line, from the gate's one function
    def test_owner_token_line_is_exactly_one_line(self):
        tok = "ot-" + "a" * 26
        with path_with(self.stub(GATE_HELD, 95)):
            rc, out, err = run_tp(owner_token=tok, owner_token_path="directives/MASTER_DIRECTIVE.md")
        self.assertEqual(rc, 0, err)
        lines = [ln for ln in out.splitlines() if ln.startswith("주인 지시 표:")]
        self.assertEqual(lines, ["주인 지시 표: %s (규칙 문서 directives/MASTER_DIRECTIVE.md)" % tok])
        # the path from the ledger's token record when not given
        import hashlib
        rec = os.path.join(os.environ["CYS_LOCAL_DIR"], "repairs", "owner-tokens", hashlib.sha256(tok.encode()).hexdigest() + ".json")
        os.makedirs(os.path.dirname(rec))
        with open(rec, "w", encoding="utf-8") as f:
            json.dump({"paths": ["soul.md"]}, f)
        with path_with(self.stub(GATE_OPEN, 0)):
            rc, out, _ = run_tp(owner_token=tok)
        self.assertEqual(rc, 0)
        self.assertIn("주인 지시 표: %s (규칙 문서 soul.md)" % tok, out)
        with path_with(self.stub(GATE_OPEN, 0)):
            rc, out, err = run_tp(owner_token="ot-short")
        self.assertEqual((rc, out), (2, ""))
        # one function: the gate's line, the token line and the merge lines all come from `_ticket_gate`
        src = open(orch.__file__, encoding="utf-8").read()
        tree = ast.parse(src)
        holders = {n.name for n in ast.walk(tree) if isinstance(n, ast.FunctionDef)
                   and any(isinstance(c, ast.Call) and getattr(c.func, "id", None) == "_owner_token_line" for c in ast.walk(n))}
        self.assertEqual(holders, {"_ticket_gate"})

    # T-9 / T-10 · E-4-11 · AU6 / AR4: the merge ticket's lines carry --files and R's OWN trigger; a
    # boot-critical repair gets the `--resolves` line (R1 / T1 · HJ1)
    def test_merge_ticket_lines(self):
        status = {"repair": "R-7", "trigger": "update", "classification": "general",
                  "files": [{"rel": "bin/a.py", "kind": "edit"}, {"rel": "bin/b c.py", "kind": "edit"},
                            {"rel": "settings:Stop:x.sh", "kind": {"hook_unregister": {"event": "Stop", "script": "x.sh"}}}]}
        with path_with(self.stub(GATE_HELD, 95, status)):
            rc, out, err = run_tp(merge="R-7")
        self.assertEqual(rc, 0, err)
        self.assertIn("  1) cys repair begin --files bin/a.py 'bin/b c.py' --joins R-7 --trigger update", out)
        self.assertIn("  3) cys repair recheck R-7", out)
        self.assertNotIn("owner-order", out, "never a made-up trigger")
        status.update(classification="boot_critical", trigger="owner_order")
        with path_with(self.stub(GATE_HELD, 95, status)):
            rc, out, _ = run_tp(merge="R-7")
        self.assertIn("  1) cys repair begin --files bin/a.py 'bin/b c.py' --trigger owner-order --resolves R-7", out)
        self.assertNotIn("--joins", out)
        with path_with(self.stub(GATE_HELD, 95, None)):
            rc, out, err = run_tp(merge="R-7")
        self.assertEqual((rc, out), (2, ""), "a record that cannot be read gives no ticket")

    # T-9 (the real CLI): the merge lines parse with the built `cys` when it is there (exit ≠ 2 = not a
    # usage error; no daemon is started — `cys repair` never autostarts, the socket points nowhere)
    def test_merge_lines_are_accepted_by_the_real_cli(self):
        td = os.environ.get("CARGO_TARGET_DIR")
        exe = os.path.join(td, "debug", "cys.exe" if os.name == "nt" else "cys") if td else None
        if not exe or not os.path.exists(exe):
            self.skipTest("the built cys binary is not available here (CARGO_TARGET_DIR/debug/cys)")
        env = dict(os.environ, HOME=self.tmp, CYS_SOCKET=os.path.join(self.tmp, "no.sock"), CYS_NO_AUTOSTART="1")
        for line in ["repair begin --files bin/a.py bin/b.py --joins R-7 --trigger update",
                     "repair begin --files bin/a.py --trigger owner-order --resolves R-7",
                     "repair recheck R-7"]:
            r = subprocess.run([exe] + line.split(), capture_output=True, text=True, env=env, timeout=60)
            self.assertNotEqual(r.returncode, 2, "%s → %s" % (line, r.stderr[-300:]))
        bad = subprocess.run([exe, "repair", "begin", "--joins", "R-7", "--trigger", "update"], capture_output=True, text=True, env=env, timeout=60)
        self.assertEqual(bad.returncode, 2, "a line without --files is refused by the parser (the mutation)")

    # RULINGS IK1 (blocking — queue blocking): a department whose HOME holds Korean letters or a space. The
    # key `javis_lane.lane_key` makes from its socket reaches the REAL `cys repair gate` as ONE argument (no
    # shell); with an empty ledger the answer is OPEN and the department's ordinary ticket goes out.
    def test_a_korean_or_spaced_home_gets_the_open_gate_of_the_real_cli(self):
        td = os.environ.get("CARGO_TARGET_DIR")
        exe = os.path.join(td, "debug", "cys.exe" if os.name == "nt" else "cys") if td else None
        if not exe or not os.path.exists(exe):
            self.skipTest("the built cys binary is not available here (CARGO_TARGET_DIR/debug/cys)")
        import javis_lane
        saved = {k: os.environ.get(k) for k in ("HOME", "CYS_NO_AUTOSTART")}
        os.environ["HOME"] = self.tmp
        os.environ["CYS_NO_AUTOSTART"] = "1"
        os.makedirs(os.environ["CYS_LOCAL_DIR"])  # the repairs root's parent exists, the ledger is empty
        try:
            for i, home in enumerate(("홍길동", "Hong Gil Dong", "홍 길동")):
                sock = os.path.join(self.tmp, "Users", home, ".local", "state", "cys-dept-sales %d" % i, "cys.sock")
                with open(os.environ["CYS_DEPTS_JSON"], "w", encoding="utf-8") as f:
                    json.dump({"depts": {"sales": {"socket": sock}}}, f, ensure_ascii=False)
                key, why = orch._gate_lane_key("sales")
                self.assertIsNone(why)
                self.assertEqual(key, javis_lane.lane_key(sock))
                with path_with(os.path.dirname(exe)):
                    state, v, why = orch._repair_gate(key)
                    self.assertEqual(state, "open", "%s → %s" % (key, why))
                    self.assertEqual(v["lane"], key)
                    rc, out, err = run_tp(surface="sales/surface:5")
                self.assertEqual(rc, 0, err)
                self.assertIn("[작업 위임", out)
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

    # HH7: in the HQ lane the conflict summary is printed once per distinct summary; the mark lives in
    # the tool's own state file, never under the repairs folder
    def test_conflict_summary_is_printed_once_per_summary(self):
        from unittest import mock
        g = dict(GATE_OPEN, conflict_summary="수리 충돌로 남은 부서 1곳: dept-a(R-1)")
        with path_with(self.stub(g, 0)):
            _, _, e0 = run_tp()  # not the HQ lane (this pack is no lane pack) → not printed
            with mock.patch.object(orch, "_issuer_lane", lambda: ("base", None)):
                _, _, e1 = run_tp()
                _, _, e2 = run_tp()
        self.assertNotIn("dept-a(R-1)", e0)
        self.assertIn("dept-a(R-1)", e1)
        self.assertNotIn("dept-a(R-1)", e2)
        self.assertTrue(os.path.exists(os.path.join(self.pack, "state", orch.REPAIR_SUMMARY_STATE)))
        self.assertFalse(os.path.exists(os.path.join(os.environ["CYS_LOCAL_DIR"], "repairs")))
        g2 = dict(GATE_OPEN, conflict_summary="수리 충돌로 남은 부서 2곳: dept-a(R-1), dept-b(R-1)")
        with path_with(self.stub(g2, 0)), mock.patch.object(orch, "_issuer_lane", lambda: ("base", None)):
            _, _, e3 = run_tp()
        self.assertIn("dept-b(R-1)", e3)

    # T-7 · Q4 census: every place that builds a delegation ticket calls the gate; a new builder
    # without it fails (mutation below)
    def test_every_ticket_builder_calls_the_gate(self):
        src = open(orch.__file__, encoding="utf-8").read()
        self.assertEqual(self.ungated(src), set())
        mutated = src + "\n\ndef cmd_new_builder(args):\n    return build_task_ticket('T', 'S', 'C', 'worker', rules=[])\n"
        self.assertEqual(self.ungated(mutated), {"cmd_new_builder"})

    @staticmethod
    def ungated(src):
        tree = ast.parse(src)
        selftest_only = {"cmd_self_test"}  # in-process self-test: builds sample tickets it never delivers
        bad = set()
        for fn in (n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef)):
            if fn.name in ("build_task_ticket",) or fn.name in selftest_only:
                continue
            names = {getattr(c.func, "id", None) for c in ast.walk(fn) if isinstance(c, ast.Call)}
            if "build_task_ticket" in names and "_ticket_gate" not in names:
                bad.add(fn.name)
        return bad


if __name__ == "__main__":
    unittest.main()
