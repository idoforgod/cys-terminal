#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_f58_queue_remedy_text.py — F58 TIP3 (r2): queue.starved remedy CLI syntax and revive sender-scope guard.

Verifies three invariants across all shipped pack text (directives/*.md including
CEO_TEMPLATE.md and soul.md, round/self-service-corrections.json `new` fields and
provenance, round/self_service_levers.json, skills/**/*.md, hooks/*.{sh,py}, and
src/bin/cysd/governance.rs `REMEDY_LLM_SUFFIX` when present in the checkout):

  1. Every prescribed `[cys] queue deliver <args>` invocation (after normalising
     hard-wrapped lines and Unicode whitespace) is parsed against the clap shape
     in `src/bin/cys.rs:1031-1039` (`QueueCommand::Deliver { surface, --id, --allow-reorder }`):
       - exactly one valid seat/surface positional token (`<좌석>`, `<surface>`,
         `surface:<N>`, or a role/seat name),
       - optional `--id <id>` or `--id=<id>` (in any order),
       - optional `--allow-reorder`,
       - zero unknown flags or trailing positional tokens,
       - no function-call parens `deliver(...)`, and no closing backtick before args
         (`` `cys queue deliver` <id> ``).
     Bare subcommand naming in prose (`` `cys queue deliver` `` with no arguments)
     and the capability-gate self-test vector (`want(..., {"command": "cys queue deliver 12"})`)
     are distinguished from agent-facing remedy prescriptions.

  2. Every CSO-addressed `queue revive` / expired-item revive prescription
     (across `CSO_DIRECTIVE.md`, CSO rows in `self-service-corrections.json`, and
     any CSO-addressed block in `skills/` or `hooks/`, after the same whitespace
     and line-wrap normalisation) scopes revive to items the CSO sent itself
     (`**네가 보낸 항목만**`) and hands expired items sent by another seat or by
     daemon/schedule (refused by daemon, master-only in `src/bin/cysd/handlers.rs`
     `revive_denial`) to `master` with the screen it read, while
     `MASTER_DIRECTIVE.md` and `CEO_TEMPLATE.md` keep master's unrestricted
     revive as-is.

  3. Every multi-role / generic `queue revive` surface that is new in `0.14.57`
     (`skills/self-service/references/levers.md`, `round/self_service_levers.json`
     `g2-b`, `src/bin/cysd/governance.rs` `REMEDY_LLM_SUFFIX`, and
     `round/self-service-corrections.json` row `T1-04`) explicitly states the
     sender scope (`master` or the sender for its own items), and
     `round/self-service-corrections.json` records provenance for the TIP3 rows.

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
PACK_DIR = pathlib.Path(os.environ.get("CYS_TEST_PACK_DIR", str(TESTS_DIR.parent.parent))).resolve()
REPO_DIR = pathlib.Path(os.environ.get("CYS_TEST_REPO_DIR", str(PACK_DIR.parent))).resolve()

# Normalises ASCII + Unicode whitespace (including U+00A0 NO-BREAK SPACE and U+3000 IDEOGRAPHIC SPACE)
WS_RE = re.compile(r"[\s\u00a0\u1680\u2000-\u200a\u202f\u205f\u3000]+")

# Capability-gate self-test vector lines in hooks/role-capability-gate.sh
GATE_SELFTEST_LINE_RE = re.compile(
    r'^\s*want\((?:True|False),\s*"Bash",\s*\{"command":\s*"cys queue (?:deliver|revive) \d+"\}'
)

# Matches `[cys] queue deliver` in normalised text
DELIVER_CMD_RE = re.compile(r"(?:\bcys\s+)?\bqueue\s+deliver\b")

# Detects argument-like tokens placed immediately outside a closing backtick (`cys queue deliver` <id>)
OUTSIDE_ARG_RE = re.compile(
    r"^\s*(?:<[^>]+>|\{[^}]+\}|--?[A-Za-z0-9_-]+|surface:\S+|\d+\b|\b(?:id|entry|item)\b)"
)

# Extracts CLI-like argument tokens from an un-backticked `queue deliver ...` invocation
UNQUOTED_ARGS_RE = re.compile(
    r"^((?:(?:<[^>]+>|\{[^}]+\}|--?[A-Za-z0-9_=-]+|surface:\S+|[A-Za-z0-9_.-]+)(?:\s+|$))+)"
)

VALID_SEAT_PLACEHOLDERS = {
    "<좌석>",
    "<surface>",
    "<seat>",
    "<대상>",
    "<대상좌석>",
    "{surface}",
    "{seat}",
}
VALID_SEAT_SURFACE_RE = re.compile(r"^surface:(?:\d+|<[^>]+>|\{[^}]+\}|[A-Za-z0-9_-]+)$")
VALID_SEAT_NAME_RE = re.compile(
    r"^(?:master|cso|ceo|worker(?:-\d+)?|reviewer(?:-?\d+|-[a-z0-9]+)?|builder(?:-\d+)?)$"
)
INVALID_SEAT_TOKENS = {
    "<id>",
    "<항목>",
    "<entry>",
    "<item>",
    "{id}",
    "{entry}",
    "{item}",
    "id",
    "entry",
    "item",
}
VALID_ID_TOKEN_RE = re.compile(r"^(?:<[^>]+>|\{[^}]+\}|[A-Za-z0-9_.-]+)$")

# Triggers CSO revive sender-scope check on any normalised CSO-addressed block
CSO_REVIVE_TRIGGER_RE = re.compile(
    r"(?:\bcys\s+)?\bqueue\s+revive\b|만료[^.\n]{0,80}\brevive\b|\brevive\s*(?:한다|하되|하고|하여|로\s*재활성)"
)

CSO_REVIVE_REQUIRED_SNIPPETS = (
    "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만",
    "다른 좌석·데몬·스케줄이 보낸 만료 항목은 데몬이 거부하므로(master 전용)",
    "읽은 그 화면과 함께 master 에게 넘긴다",
)

AMENDED_MANIFEST_ROW_IDS = (
    "T1-01+T2-01",
    "T1-01+T2-01m",
    "T1-03+T2-01c",
    "T1-04",
)


def is_valid_seat_token(tok: str) -> bool:
    """Return True iff `tok` is a valid `surface` positional token per `src/bin/cys.rs:1031-1039`."""
    if not tok or tok.startswith("-") or tok in INVALID_SEAT_TOKENS or tok.isdigit():
        return False
    return (
        tok in VALID_SEAT_PLACEHOLDERS
        or bool(VALID_SEAT_SURFACE_RE.match(tok))
        or bool(VALID_SEAT_NAME_RE.match(tok))
    )


def is_valid_id_token(tok: str) -> bool:
    """Return True iff `tok` is a valid `--id` value token."""
    if not tok or tok.startswith("-") or tok in VALID_SEAT_PLACEHOLDERS:
        return False
    return bool(VALID_ID_TOKEN_RE.match(tok))


def parse_deliver_args(arg_str: str):
    """Parse `queue deliver` arguments against `QueueCommand::Deliver { surface, --id, --allow-reorder }`."""
    tokens = arg_str.strip().split()
    if not tokens:
        return False, "empty deliver args"
    seat_token = None
    seen_id = False
    seen_allow_reorder = False
    i = 0
    while i < len(tokens):
        tok = tokens[i]
        if tok == "--allow-reorder":
            if seen_allow_reorder:
                return False, "duplicate --allow-reorder"
            seen_allow_reorder = True
            i += 1
        elif tok == "--id":
            if seen_id:
                return False, "duplicate --id"
            if i + 1 >= len(tokens):
                return False, "--id missing value"
            val = tokens[i + 1]
            if not is_valid_id_token(val):
                return False, f"invalid --id value {val!r}"
            seen_id = True
            i += 2
        elif tok.startswith("--id="):
            if seen_id:
                return False, "duplicate --id"
            val = tok[len("--id=") :]
            if not is_valid_id_token(val):
                return False, f"invalid --id= value {val!r}"
            seen_id = True
            i += 1
        elif tok.startswith("-"):
            return False, f"unknown flag {tok!r}"
        else:
            if seat_token is not None:
                return (
                    False,
                    f"unexpected extra positional arg {tok!r} after seat {seat_token!r}",
                )
            if not is_valid_seat_token(tok):
                return (
                    False,
                    f"positional token {tok!r} is not a valid seat/surface token "
                    f"(expected <좌석>, <surface>, surface:N, or seat name)",
                )
            seat_token = tok
            i += 1
    if seat_token is None:
        return False, "missing required positional <surface>/<좌석> argument"
    return True, f"seat={seat_token} id={seen_id} allow_reorder={seen_allow_reorder}"


def normalise_blocks(text: str, *, is_hook: bool = False):
    """Return `(start_line, normalised_block)` pairs with hard-wrapped lines joined and Unicode spaces collapsed."""
    if is_hook:
        kept_lines = [
            "" if GATE_SELFTEST_LINE_RE.search(ln) else ln
            for ln in text.splitlines()
        ]
        text = "\n".join(kept_lines)

    # First join any newlines inside matched inline backtick spans (`cys queue\n  deliver <id>`)
    # Pairing from the start of `text` (and skipping ``` fences) prevents matching a closing
    # backtick on one line with an opening backtick on the next line.
    text_joined_bt = re.sub(
        r"```[\s\S]*?```|`([^`]+)`",
        lambda m: (
            "`" + WS_RE.sub(" ", m.group(1)).strip() + "`"
            if m.group(1) is not None
            else m.group(0)
        ),
        text,
    )

    # Split into logical blocks on blank lines, markdown table rows, headings, list items, or JSON elements
    boundary_re = re.compile(
        r"\n[ \t\u00a0\u3000]*\n|\n(?=[ \t]*(?:\||#|[-*+][ \t]|\d+\.[ \t]|[\{\}\[\]\"]))"
    )
    out = []
    pos = 0
    line_cursor = 1
    for m in boundary_re.finditer(text_joined_bt):
        rb = text_joined_bt[pos : m.start()]
        norm = WS_RE.sub(" ", rb).strip()
        if norm:
            out.append((line_cursor, norm))
        line_cursor += text_joined_bt[pos : m.end()].count("\n")
        pos = m.end()
    rb = text_joined_bt[pos:]
    norm = WS_RE.sub(" ", rb).strip()
    if norm:
        out.append((line_cursor, norm))
    return out


def scan_deliver_text(target_id: str, text: str, *, is_hook: bool = False):
    """Return `(violations, valid_invocations)` for all `[cys] queue deliver` occurrences in `text`."""
    violations = []
    valid_invocations = []
    for lineno, norm in normalise_blocks(text, is_hook=is_hook):
        for m in DELIVER_CMD_RE.finditer(norm):
            in_bt = (norm[: m.start()].count("`") % 2 == 1)
            tail = norm[m.end() :]
            if tail.startswith("("):
                violations.append(
                    f"{target_id}:{lineno}: invalid function-call parens after `queue deliver` "
                    f"({tail[:40]!r}) in block: {norm[:160]}"
                )
                continue
            if in_bt:
                close_idx = tail.find("`")
                inside_after = tail[:close_idx] if close_idx != -1 else tail
                outside_after = tail[close_idx + 1 :] if close_idx != -1 else ""
                if not inside_after.strip():
                    if OUTSIDE_ARG_RE.match(outside_after):
                        violations.append(
                            f"{target_id}:{lineno}: backtick closed before `queue deliver` args "
                            f"({outside_after.strip()[:40]!r}) in block: {norm[:160]}"
                        )
                    continue
                if OUTSIDE_ARG_RE.match(outside_after):
                    violations.append(
                        f"{target_id}:{lineno}: trailing arg outside closing backtick after `queue deliver` "
                        f"({outside_after.strip()[:40]!r}) in block: {norm[:160]}"
                    )
                    continue
                arg_str = inside_after.strip()
                ok, reason = parse_deliver_args(arg_str)
                if not ok:
                    violations.append(
                        f"{target_id}:{lineno}: invalid `queue deliver` args {arg_str!r} "
                        f"({reason}) in block: {norm[:160]}"
                    )
                else:
                    valid_invocations.append(arg_str)
            else:
                if not tail or tail[0] in ")'\"·,.;:!?/":
                    continue
                if tail.startswith(" "):
                    m_args = UNQUOTED_ARGS_RE.match(tail.lstrip())
                    if m_args:
                        arg_str = m_args.group(1).strip()
                        ok, reason = parse_deliver_args(arg_str)
                        if not ok:
                            violations.append(
                                f"{target_id}:{lineno}: invalid unquoted `queue deliver` args {arg_str!r} "
                                f"({reason}) in block: {norm[:160]}"
                            )
                        else:
                            valid_invocations.append(arg_str)
    return violations, valid_invocations


def scan_deliver_violations(target_id: str, text: str, *, is_hook: bool = False):
    """Return a list of human-readable violation strings for `[cys] queue deliver`."""
    violations, _ = scan_deliver_text(target_id, text, is_hook=is_hook)
    return violations


def scan_cso_revive_text(target_id: str, text: str, *, is_hook: bool = False):
    """Return `(violations, matched_blocks)` for CSO-addressed revive prescriptions."""
    violations = []
    matched_blocks = []
    for lineno, norm in normalise_blocks(text, is_hook=is_hook):
        if not CSO_REVIVE_TRIGGER_RE.search(norm):
            continue
        matched_blocks.append(norm)
        missing = [s for s in CSO_REVIVE_REQUIRED_SNIPPETS if s not in norm]
        if missing:
            violations.append(
                f"{target_id}:{lineno}: CSO revive prescription missing required sender-scope/handoff "
                f"clauses {missing!r} in block: {norm[:180]}"
            )
    return violations, matched_blocks


def scan_cso_revive_violations(target_id: str, text: str, *, is_hook: bool = False):
    """Return a list of violation strings if a CSO-addressed text prescribes `revive` without sender scoping."""
    violations, _ = scan_cso_revive_text(target_id, text, is_hook=is_hook)
    return violations


def collect_pack_targets(pack_dir: pathlib.Path):
    """Collect all shipped pack text targets: directives, CEO_TEMPLATE, soul.md, manifest, levers, skills, hooks."""
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
        self.assertGreaterEqual(len(skills), 3, "expected at least 3 skill files")
        self.assertGreaterEqual(len(hooks), 20, "expected at least 20 hook files")
        self.assertEqual(len(manifest.get("rows", [])), 68, "expected 68 manifest rows")

    def test_all_shipped_pack_texts_use_seat_and_id_for_queue_deliver(self):
        directives, skills, hooks, manifest = collect_pack_targets(PACK_DIR)
        violations = []
        valid_prescriptions = []

        for p in directives + skills:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            v, valid = scan_deliver_text(rel, txt, is_hook=False)
            violations.extend(v)
            valid_prescriptions.extend((rel, arg_str) for arg_str in valid)

        for p in hooks:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            v, _ = scan_deliver_text(rel, txt, is_hook=True)
            violations.extend(v)

        for row in manifest.get("rows", []):
            rid = f"round/self-service-corrections.json[{row['id']}.new]"
            new_txt = row.get("new", "")
            v, valid = scan_deliver_text(rid, new_txt, is_hook=False)
            violations.extend(v)
            valid_prescriptions.extend((rid, arg_str) for arg_str in valid)

        if violations:
            for v in violations:
                print(f"FAIL_LINE: {v}", file=sys.stderr)
        self.assertEqual(violations, [], "\n".join(violations))
        # Master, CSO, CEO_TEMPLATE + 3 manifest rows = 6 valid deliver prescriptions
        self.assertEqual(
            len(valid_prescriptions),
            6,
            f"expected 6 valid `queue deliver` prescriptions across directives+manifest, got {len(valid_prescriptions)}: {valid_prescriptions}",
        )

    def test_cso_revive_is_scoped_to_own_sent_items_and_master_kept_as_is(self):
        directives, skills, hooks, manifest = collect_pack_targets(PACK_DIR)
        violations = []
        cso_revive_matches = 0

        cso_path = PACK_DIR / "directives" / "CSO_DIRECTIVE.md"
        cso_txt = cso_path.read_text(encoding="utf-8")
        v, matched = scan_cso_revive_text("directives/CSO_DIRECTIVE.md", cso_txt, is_hook=False)
        violations.extend(v)
        cso_revive_matches += len(matched)
        self.assertGreaterEqual(
            len(matched),
            1,
            "directives/CSO_DIRECTIVE.md must contain at least one CSO revive prescription block",
        )

        for row in manifest.get("rows", []):
            if row.get("file") == "directives/CSO_DIRECTIVE.md" or "cso" in row.get("roles", []):
                rid = f"round/self-service-corrections.json[{row['id']}.new]"
                v, matched = scan_cso_revive_text(rid, row.get("new", ""), is_hook=False)
                violations.extend(v)
                cso_revive_matches += len(matched)

        # Widen CSO revive scan to any CSO-addressed block in skills/ or hooks/
        for p in skills:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            for lineno, norm in normalise_blocks(txt, is_hook=False):
                if re.search(r"\bcso\b", norm, re.IGNORECASE) and CSO_REVIVE_TRIGGER_RE.search(norm):
                    v, _ = scan_cso_revive_text(f"{rel}:{lineno}", norm, is_hook=False)
                    violations.extend(v)

        for p in hooks:
            rel = p.relative_to(PACK_DIR).as_posix()
            txt = p.read_text(encoding="utf-8")
            for lineno, norm in normalise_blocks(txt, is_hook=True):
                if re.search(r"\bcso\b", norm, re.IGNORECASE) and CSO_REVIVE_TRIGGER_RE.search(norm):
                    v, _ = scan_cso_revive_text(f"{rel}:{lineno}", norm, is_hook=True)
                    violations.extend(v)

        if violations:
            for v in violations:
                print(f"FAIL_LINE: {v}", file=sys.stderr)
        self.assertEqual(violations, [], "\n".join(violations))
        self.assertGreaterEqual(cso_revive_matches, 2, "expected CSO_DIRECTIVE.md + T1-03+T2-01c matches")

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

    def test_new_in_0_14_57_revive_surfaces_carry_sender_scope_and_provenance(self):
        violations = []

        # 1) skills/self-service/references/levers.md:37
        levers_md_path = PACK_DIR / "skills" / "self-service" / "references" / "levers.md"
        levers_md_txt = levers_md_path.read_text(encoding="utf-8")
        revive_blocks = [
            (lineno, norm)
            for lineno, norm in normalise_blocks(levers_md_txt, is_hook=False)
            if re.search(r"(?:\bcys\s+)?\bqueue\s+revive\b", norm)
        ]
        if not revive_blocks:
            violations.append("skills/self-service/references/levers.md: missing `cys queue revive` entry")
        for lineno, norm in revive_blocks:
            if "master" not in norm or "보낸 좌석 본인만" not in norm:
                violations.append(
                    f"skills/self-service/references/levers.md:{lineno}: `cys queue revive` missing "
                    f"sender scope ('master 또는 그 항목을 보낸 좌석 본인만') in: {norm}"
                )

        # 2) round/self_service_levers.json (entry g2-b)
        levers_json_path = PACK_DIR / "round" / "self_service_levers.json"
        levers_data = json.loads(levers_json_path.read_text(encoding="utf-8"))
        g2b_found = False
        for entry in levers_data.get("entries", []):
            for lv in entry.get("levers", []):
                act = lv.get("action", "")
                if "queue revive" in act:
                    g2b_found = True
                    if "master, or the sender for its own items" not in act:
                        violations.append(
                            f"round/self_service_levers.json[{entry.get('id')}]: `queue revive` action "
                            f"missing sender scope ('master, or the sender for its own items'): {act!r}"
                        )
        if not g2b_found:
            violations.append("round/self_service_levers.json: no lever action mentioning `queue revive` found")

        # 3) round/self-service-corrections.json row T1-04 + provenance
        manifest_path = PACK_DIR / "round" / "self-service-corrections.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        src_note = manifest.get("source", "")
        for ticket_name in (
            "HEAD-TICKET-TIP3-QUEUE-REMEDY-TEXT-w61.md",
            "HEAD-TICKET-TIP3-QUEUE-REMEDY-TEXT-r2-w61.md",
        ):
            if ticket_name not in src_note:
                violations.append(
                    f"round/self-service-corrections.json[source]: missing provenance reference to {ticket_name}: {src_note!r}"
                )

        rows_by_id = {r["id"]: r for r in manifest.get("rows", [])}
        t104 = rows_by_id.get("T1-04", {})
        if "revive(master 또는 보낸 좌석 본인만)·deliver" not in t104.get("new", ""):
            violations.append(
                f"round/self-service-corrections.json[T1-04.new]: missing sender scope "
                f"'revive(master 또는 보낸 좌석 본인만)·deliver': {t104.get('new')!r}"
            )
        for rid in AMENDED_MANIFEST_ROW_IDS:
            row = rows_by_id.get(rid, {})
            rnote = row.get("note", "")
            if "HEAD-TICKET-TIP3-QUEUE-REMEDY-TEXT" not in rnote:
                violations.append(
                    f"round/self-service-corrections.json[{rid}.note]: missing per-row provenance note: {rnote!r}"
                )

        # 4) src/bin/cysd/governance.rs REMEDY_LLM_SUFFIX, USER-MANUAL.md, and ui/src/starvednotice.test.ts
        #    (when repo source tree is present)
        expected_suffix = (
            " · LLM 에이전트는 자동 조치(remedy 대로 단건 revive(master 또는 보낸 좌석 본인만)·deliver)를 한다 — "
            "사용자 지침이 막으면 따르고, 드레인·항목 삭제·동결 해제·사람 글에 닿는 키는 사용자에게 한 번 묻는다"
        )
        stale_suffix_token = "단건 revive" + "·deliver"
        gov_path = REPO_DIR / "src" / "bin" / "cysd" / "governance.rs"
        if gov_path.exists():
            gov_txt = gov_path.read_text(encoding="utf-8")
            m_suf = re.search(r'pub\(crate\)\s+const\s+REMEDY_LLM_SUFFIX:\s*&str\s*=\s*"([^"]*)";', gov_txt)
            if not m_suf:
                violations.append("src/bin/cysd/governance.rs: REMEDY_LLM_SUFFIX declaration not found")
            elif m_suf.group(1) != expected_suffix:
                violations.append(
                    f"src/bin/cysd/governance.rs:10940: REMEDY_LLM_SUFFIX mismatch: {m_suf.group(1)!r}"
                )

        um_path = REPO_DIR / "USER-MANUAL.md"
        if um_path.exists():
            um_txt = um_path.read_text(encoding="utf-8")
            if expected_suffix not in um_txt or stale_suffix_token in um_txt:
                violations.append(
                    "USER-MANUAL.md:1117: REMEDY_LLM_SUFFIX quote does not match governance.rs"
                )

        sn_test_path = REPO_DIR / "ui" / "src" / "starvednotice.test.ts"
        if sn_test_path.exists():
            sn_txt = sn_test_path.read_text(encoding="utf-8")
            if expected_suffix not in sn_txt or stale_suffix_token in sn_txt:
                violations.append(
                    "ui/src/starvednotice.test.ts:42: TAIL fixture does not match governance.rs REMEDY_LLM_SUFFIX"
                )

        if violations:
            for v in violations:
                print(f"FAIL_LINE: {v}", file=sys.stderr)
        self.assertEqual(violations, [], "\n".join(violations))

    def test_adversarial_variants_and_must_pass_forms(self):
        # 1) Every must-fail deliver variant (including w191's 9 bypass forms) must be caught
        for bad_cmd in (
            "`cys queue deliver <id>`",
            "`cys queue deliver` <id>",
            "`cys queue\n  deliver <id>`",
            "`queue deliver <id>`",
            "cys queue deliver(<id>)",
            "`cys queue deliver <좌석> --id <id> <id>`",
            "`cys queue\u3000deliver\u3000<id>`",
            "`cys queue  deliver  <entry>`",
            "`cys queue deliver --id=<id>`",
            "`cys queue deliver <항목>`",
            "`cys queue deliver {id}`",
            "`cys queue deliver 12`",
            "`cys queue deliver --id <id>`",
        ):
            with self.subTest(bad_deliver=bad_cmd):
                v, valid = scan_deliver_text("synthetic.md", f"막힌 머리는 {bad_cmd}(단건)", is_hook=False)
                self.assertEqual(len(v), 1, f"adversarial variant {bad_cmd!r} was not caught: v={v}")
                self.assertEqual(valid, [], f"adversarial variant {bad_cmd!r} falsely accepted: {valid}")

        # 2) Valid CLI deliver shapes (clap QueueCommand::Deliver at src/bin/cys.rs:1031-1039) must pass
        for valid_cmd in (
            "`cys queue deliver <좌석> --id <id>`",
            "`cys queue deliver <좌석>`",
            "`cys queue deliver --id <id> <좌석>`",
            "`cys queue deliver <surface> --id <id>`",
            "`cys queue deliver surface:3 --id <id> --allow-reorder`",
            "`cys queue deliver worker-1 --id 42`",
            "`cys queue deliver <좌석> --id=<id>`",
        ):
            with self.subTest(valid_deliver=valid_cmd):
                v, valid = scan_deliver_text("synthetic.md", f"막힌 머리는 {valid_cmd}(단건)", is_hook=False)
                self.assertEqual(v, [], f"valid CLI form {valid_cmd!r} was falsely flagged: {v}")
                self.assertEqual(len(valid), 1, f"valid CLI form {valid_cmd!r} was not counted: {valid}")

        # 3) Bare subcommand reference in prose (no arguments) must pass with 0 violations and 0 invocations
        for ok_prose in (
            "큐 강제 배달(`cys queue deliver`)은 데몬이 허락하는 막힘에서만 듣는다.",
            "강제 배달 `cys queue deliver`·드레인·키 주입 금지",
        ):
            with self.subTest(ok_prose=ok_prose):
                v, valid = scan_deliver_text("synthetic.md", ok_prose, is_hook=False)
                self.assertEqual(v, [], f"valid prose {ok_prose!r} was falsely flagged: {v}")
                self.assertEqual(valid, [], f"bare prose {ok_prose!r} should not count as an invocation with args")

        # 4) Un-scoped, wrapped, or no-`cys` CSO revive prescriptions must all be caught
        for bad_cso in (
            "만료 항목은 `cys queue revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 `queue revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 `cys queue\n  revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 revive 한다",
            "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만)",
            "만료 항목은 **네가 보낸 항목만** `cys queue revive <id>`(같은 항목은 한 번만 — 다른 좌석·데몬·스케줄이 보낸 만료 항목은 데몬이 거부하므로(master 전용) 직접 폐기한다)",
        ):
            with self.subTest(bad_cso=bad_cso):
                v = scan_cso_revive_violations("directives/CSO_DIRECTIVE.md", bad_cso)
                self.assertEqual(len(v), 1, f"un-scoped CSO revive {bad_cso!r} was not caught: {v}")


if __name__ == "__main__":
    unittest.main(verbosity=2)
