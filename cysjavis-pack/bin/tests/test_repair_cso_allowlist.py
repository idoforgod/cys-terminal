#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_repair_cso_allowlist.py — 「스스로 해결」(0.15.0 · 설계 §4-6 · §6-3 (4) · track T3).

CSO 는 `cys repair report|list|status` 만 쓴다(결함 접수는 자료일 뿐 권한이 아니다 · 장부 읽기).
장부를 바꾸는 수리 명령은 능력 게이트 훅이 막고(이 파일), 데몬도 막는다(Rust 시험). 허용은 예산 면제가
아니다(좁은 쪽이 이긴다). 마지막 클래스는 음성 대조다: 훅에서 허용 줄을 지우면 이 시험이 붉어진다.

라이브 무접촉: `test_capgate_hook_shell._HookEnv` 의 격리(임시 HOME·TMPDIR·CYS_STATE_DIR·PATH, `cys` 부재)를
그대로 쓴다. 데몬·소켓에 닿지 않는다.

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_repair_cso_allowlist.py
"""
import ast
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
TESTS = Path(__file__).resolve().parent
sys.path.insert(0, str(TESTS))
import test_capgate_hook_shell as base  # noqa: E402  (sibling — the isolated hook runner)

ALLOWED = [
    "cys repair report --file bin/javis_a.py --error 'SyntaxError: invalid syntax'",
    "cys repair list",
    "cys repair list --json",
    "cys repair status",
    "cys repair status R-3 --json",
]
REFUSED = [
    "cys repair begin --files bin/javis_a.py",
    "cys repair record R-1 --note n",
    "cys repair submit R-1",
    "cys repair commit R-1",
    "cys repair attest-verify R-1 --verdict pass --note n",
    "cys repair attest-audit R-1 --verdict ok --note n",
    "cys repair revert R-1",
    "cys repair reapply R-1",
    "cys repair cleanup R-1",
    "cys repair recheck R-1",
    "cys repair grant on",
    "cys repair owner-token --path directives/MASTER_DIRECTIVE.md",
    "cys repair sweep",
]


def _hook_python(text):
    lines = text.splitlines()
    head = [i for i, l in enumerate(lines) if l.strip().endswith("<<'PYEOF'")]
    assert len(head) == 1, "hook python heredoc not found exactly once"
    end = [i for i in range(head[0] + 1, len(lines)) if lines[i].rstrip() == "PYEOF"]
    return "\n".join(lines[head[0] + 1:end[0]])


def _module_value(tree, name):
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == name for t in node.targets):
            return ast.literal_eval(node.value)
    raise AssertionError("%s not declared at module level" % name)


@base.NEED_SH
class CsoRepairAllowlist(base._HookEnv):
    """The real hook, run end to end as a CSO seat (env role, no daemon)."""

    def test_report_list_status_allowed(self):
        for cmd in ALLOWED:
            r = self.run_hook("Bash", {"command": cmd}, CYS_ROLE="cso")
            self.assertEqual(r.rc, 0, r.err)
            self.assertFalse(r.denied, "CSO refused `%s`: %s" % (cmd, r.reason))

    def test_ledger_changing_verbs_refused(self):
        for cmd in REFUSED:
            r = self.run_hook("Bash", {"command": cmd}, CYS_ROLE="cso")
            self.assertEqual(r.rc, 0, r.err)
            self.assertTrue(r.denied, "CSO was allowed `%s`" % cmd)
            self.assertIn("허용", r.reason)

    def test_allowed_but_not_budget_exempt(self):
        """Narrower wins: the three verbs are allowed, never in the budget-exemption set."""
        tree = ast.parse(_hook_python(base.HOOK.read_text(encoding="utf-8")))
        sub = _module_value(tree, "CSO_CYS_SUBVERBS")
        self.assertEqual(sub.get("repair"), {"report", "list", "status"})
        essential = _module_value(tree, "CSO_CYS_SUBVERB_ESSENTIAL")
        self.assertFalse([p for p in essential if p[0] == "repair"], essential)


@base.NEED_SH
class CsoRepairAllowlistMutationControl(base._HookEnv):
    """Negative control: with the allow line removed from a COPY of the hook, `report` is refused —
    so the positive test above would fail if the guard rail were taken away."""

    def test_removing_the_line_refuses_report(self):
        text = base.HOOK.read_text(encoding="utf-8")
        line = '    "repair": {"report", "list", "status"},\n'
        self.assertEqual(text.count(line), 1)
        d = Path(tempfile.mkdtemp(prefix="capgate-mut-"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "role-capability-gate.sh").write_text(text.replace(line, ""), encoding="utf-8")
        shutil.copy(base.LIB, d / "_lib.sh")
        payload = '{"session_id": "s-1", "tool_name": "Bash", "tool_input": {"command": "cys repair report --file a --error b"}}'
        env = dict(self.env, CYS_ROLE="cso")
        r = subprocess.run([base.SH, str(d / "role-capability-gate.sh")], input=payload, env=env,
                           capture_output=True, text=True, timeout=90)
        run = base.HookRun(r.returncode, r.stdout, r.stderr)
        self.assertTrue(run.denied, "the mutated hook still allowed `repair report` — the control is blind")


if __name__ == "__main__":
    unittest.main()
