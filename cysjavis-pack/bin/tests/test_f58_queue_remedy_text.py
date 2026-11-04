#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_queue_remedy_text.py — F58 TIP3: queue.starved remedy CLI syntax and CSO revive authority pin.

Verifies two invariants across all shipped pack text (directives/*.md including
CEO_TEMPLATE.md, round/self-service-corrections.json `new` fields, skills/**/*.md,
and hooks/*.sh):
  1. Every prescribed `cys queue deliver <args>` invocation uses the CLI's
     `<좌석> --id <id>` shape (`src/bin/cys.rs` `QueueCommand::Deliver { surface, --id }`).
     Bare subcommand naming in prose (`` `cys queue deliver` `` with no arguments)
     and the capability-gate self-test vector (`want(..., {"command": "cys queue deliver 12"})`)
     are distinguished from agent-facing remedy prescriptions.
  2. Every CSO-facing `cys queue revive` prescription (`CSO_DIRECTIVE.md` and
     CSO rows in `self-service-corrections.json` `new`) scopes revive to items
     the CSO sent itself (`네가 보낸 항목만`) and hands expired items sent by
     another seat or by daemon/schedule (refused by daemon, master-only in
     `src/bin/cysd/handlers.rs` `revive_denial`) to `master` with the screen it
     read, while `MASTER_DIRECTIVE.md` keeps master's unrestricted revive as-is.

Run:
    python3 cysjavis-pack/bin/tests/test_f58_queue_remedy_text.py
"""
import json
import os
import pathlib
import re
import sys
import unittest

sys.dont_write_bytecode = True

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

TESTS_DIR = pathlib.Path(__file__).resolve().parent
PACK_DIR = TESTS_DIR.parent.parent

# Matches `cys queue deliver` followed by whitespace and argument tokens before
# closing backtick, quote, paren, or end of line.
DELIVER_WITH_ARGS_RE = re.compile(
    r"cys\s+queue\s+deliver(?=[ \t]+[^\s`'\"\)])([ \t]+[^`'\"\n\)]*)"
)

ALLOWED_DELIVER_ARG_PREFIX = "<좌석> --id <id>"

# Capability-gate self-test vector line in hooks/role-capability-gate.sh
GATE_SELFTEST_LINE_RE = re.compile(
    r'^\s*want\((?:True|False),\s*"Bash",\s*\{"command":\s*"cys queue deliver \d+"\}'
)

CSO_REVIVE_REQUIRED_SNIPPETS = (
    "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만",
    "다른 좌석·데몬·스케줄이 보낸 만료 항목은 데몬이 거부하므로(master 전용)",
    "읽은 그 화면과 함께 master 에게 넘긴다",
)


def scan_deliver_violations(target_id: str, text: str, *, is_hook: bool = False):
    """Return a list of human-readable violation strings for `cys queue deliver`."""
    violations = []
    for lineno, line in enumerate(text.splitlines(), 1):
        if is_hook and GATE_SELFTEST_LINE_RE.search(line):
            continue
        for m in DELIVER_WITH_ARGS_RE.finditer(line):
            args = m.group(1).strip()
            if not args.startswith(ALLOWED_DELIVER_ARG_PREFIX):
                violations.append(
                    f"{target_id}:{lineno}: invalid `cys queue deliver` args {args!r} "
                    f"(expected prefix {ALLOWED_DELIVER_ARG_PREFIX!r}) in line: {line.strip()}"
                )
    return violations


def scan_cso_revive_violations(target_id: str, text: str):
    """Return a list of violation strings if a CSO text prescribes `cys queue revive` without sender scoping."""
    violations = []
    for lineno, line in enumerate(text.splitlines(), 1):
        if "cys queue revive" not in line:
            continue
        missing = [s for s in CSO_REVIVE_REQUIRED_SNIPPETS if s not in line]
        if missing:
            violations.append(
                f"{target_id}:{lineno}: CSO `cys queue revive` missing required sender-scope/handoff "
                f"clauses {missing!r} in line: {line.strip()}"
            )
    return violations


def collect_pack_targets(pack_dir: pathlib.Path):
    """Collect all shipped pack text targets: directives, CEO_TEMPLATE, soul.md, manifest `new` fields, levers, skills, hooks."""
    directives = sorted((pack_dir / "directives").glob("*.md"))
    soul_path = pack_dir / "soul.md"
    if soul_path.exists():
        directives.append(soul_path)
    skills = sorted((pack_dir / "skills").rglob("*.md"))
    levers_path = pack_dir / "round" / "self_service_levers.json"
    if levers_path.exists():
        skills.append(levers_path)
    hooks = sorted((pack_dir / "hooks").rglob("*.sh")) + sorted((pack_dir / "hooks").rglob("*.py"))
    manifest_path = pack_dir / "round" / "self-service-corrections.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    return directives, skills, hooks, manifest


class QueueRemedyTextTests(unittest.TestCase):
    def test_shipped_pack_targets_nonempty_and_complete(self):
        directives, skills, hooks, manifest = collect_pack_targets(PACK_DIR)
        dir_names = {p.name for p in directives}
        for req in ("MASTER_DIRECTIVE.md", "CSO_DIRECTIVE.md", "CEO_TEMPLATE.md", "WORKER_DIRECTIVE.md"):
            self.assertIn(req, dir_names, f"missing directive {req}")
        self.assertGreaterEqual(len(skills), 3, "expected at least 3 skill markdown files")
        self.assertGreaterEqual(len(hooks), 20, "expected at least 20 hook files")
        self.assertEqual(len(manifest.get("rows", [])), 68, "expected 68 manifest rows")

    def test_all_shipped_pack_texts_use_seat_and_id_for_queue_deliver(self):
        directives, skills, hooks, manifest = collect_pack_targets(PACK_DIR)
        violations = []
        positive_hits = 0

        for p in directives + skills:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            positive_hits += txt.count(f"cys queue deliver {ALLOWED_DELIVER_ARG_PREFIX}")
            violations.extend(scan_deliver_violations(rel, txt, is_hook=False))

        for p in hooks:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            violations.extend(scan_deliver_violations(rel, txt, is_hook=True))

        for row in manifest.get("rows", []):
            rid = f"round/self-service-corrections.json[{row['id']}.new]"
            new_txt = row.get("new", "")
            positive_hits += new_txt.count(f"cys queue deliver {ALLOWED_DELIVER_ARG_PREFIX}")
            violations.extend(scan_deliver_violations(rid, new_txt, is_hook=False))

        if violations:
            for v in violations:
                print(f"FAIL_LINE: {v}", file=sys.stderr)
        self.assertEqual(violations, [], "\n".join(violations))
        # Master, CSO, CEO_TEMPLATE + 3 manifest rows = 6 positive prescriptions
        self.assertEqual(
            positive_hits,
            6,
            f"expected 6 positive `cys queue deliver {ALLOWED_DELIVER_ARG_PREFIX}` prescriptions across directives+manifest, got {positive_hits}",
        )

    def test_cso_revive_is_scoped_to_own_sent_items_and_master_kept_as_is(self):
        directives, _skills, _hooks, manifest = collect_pack_targets(PACK_DIR)
        violations = []

        cso_path = PACK_DIR / "directives" / "CSO_DIRECTIVE.md"
        cso_txt = cso_path.read_text(encoding="utf-8")
        self.assertIn("cys queue revive <id>", cso_txt)
        violations.extend(scan_cso_revive_violations("directives/CSO_DIRECTIVE.md", cso_txt))

        for row in manifest.get("rows", []):
            if row.get("file") == "directives/CSO_DIRECTIVE.md" or "cso" in row.get("roles", []):
                rid = f"round/self-service-corrections.json[{row['id']}.new]"
                violations.extend(scan_cso_revive_violations(rid, row.get("new", "")))

        if violations:
            for v in violations:
                print(f"FAIL_LINE: {v}", file=sys.stderr)
        self.assertEqual(violations, [], "\n".join(violations))

        # MASTER_DIRECTIVE.md and CEO_TEMPLATE.md keep master's unrestricted revive as-is
        for name in ("MASTER_DIRECTIVE.md", "CEO_TEMPLATE.md"):
            body = (PACK_DIR / "directives" / name).read_text(encoding="utf-8")
            self.assertIn(
                "②만료 항목은 `cys queue revive <id>`(같은 항목은 한 번만), 막힌 머리는",
                body,
                f"{name} must keep master's revive clause as-is",
            )
            self.assertNotIn(
                "네가 보낸 항목만",
                body,
                f"{name} must not restrict master's revive authority to own-sent items",
            )

    def test_adversarial_variants_caught(self):
        # 1) Wrong deliver shapes must all be flagged
        for bad_cmd in (
            "`cys queue deliver <id>`",
            "`cys queue deliver <항목>`",
            "`cys queue deliver {id}`",
            "`cys queue deliver 12`",
            "`cys queue deliver --id <id>`",
        ):
            with self.subTest(bad_deliver=bad_cmd):
                v = scan_deliver_violations("synthetic.md", f"막힌 머리는 {bad_cmd}(단건)", is_hook=False)
                self.assertEqual(len(v), 1, f"adversarial variant {bad_cmd} was not caught")

        # 2) Bare subcommand reference in prose (e.g. levers.md or manifest `old`) must pass
        for ok_prose in (
            "큐 강제 배달(`cys queue deliver`)은 데몬이 허락하는 막힘에서만 듣는다.",
            "강제 배달 `cys queue deliver`·드레인·키 주입 금지",
            "막힌 머리는 `cys queue deliver <좌석> --id <id>`(단건)",
        ):
            with self.subTest(ok_deliver=ok_prose):
                v = scan_deliver_violations("synthetic.md", ok_prose, is_hook=False)
                self.assertEqual(v, [], f"valid text {ok_prose!r} was falsely flagged: {v}")

        # 3) Un-scoped or partially-scoped CSO revive must be flagged
        for bad_cso in (
            "만료 항목은 `cys queue revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만 — 다른 좌석·데몬·스케줄이 보낸 만료 항목은 데몬이 거부하므로(master 전용) 직접 폐기한다)",
        ):
            with self.subTest(bad_cso=bad_cso):
                v = scan_cso_revive_violations("directives/CSO_DIRECTIVE.md", bad_cso)
                self.assertEqual(len(v), 1, f"un-scoped CSO revive {bad_cso!r} was not caught")


if __name__ == "__main__":
    unittest.main(verbosity=2)
