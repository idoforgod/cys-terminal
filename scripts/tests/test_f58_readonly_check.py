"""Controls for scripts/f58_readonly_check.py - the T-6 read-only predicate (DESIGN-F58-final §7.4) - and for the
shipped self-service skill text it is paired with (cysjavis-pack/skills/self-service, r1 C1, r2 §8.2, final §2.5).

Why this exists: the T-6 eval splits the first assistant turn into read-only calls and consequential actions. A
predicate that is only ever seen saying "read-only" cannot be told apart from one that always says it, so every case
of §7.4 is measured both ways: the seven instrument cases of the design, then one passing control next to each
refusal that differs only in the construct under test. The skill half checks facts, not wording: every
javis_selfservice.py verb the skill names exists in the tool's parser, the first-answer forms equal the hook block
and the tool's own question constants, and the four cells are present.

Hermetic: no network, no git, no daemon; the transcript case writes a temp file.

    python3 scripts/tests/test_f58_readonly_check.py
"""
import argparse
import importlib.util
import json
import os
import re
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
# Mutation testing: F58_READONLY_CHECK_UNDER_TEST=<mutant path> runs the same cases against that script instead.
SCRIPT = os.environ.get("F58_READONLY_CHECK_UNDER_TEST") or os.path.join(ROOT, "scripts", "f58_readonly_check.py")
PACK = os.path.join(ROOT, "cysjavis-pack")
SKILL = os.path.join(PACK, "skills", "self-service", "SKILL.md")
LEVERS = os.path.join(PACK, "skills", "self-service", "references", "levers.md")
TOOL = os.path.join(PACK, "bin", "javis_selfservice.py")
LIB = os.path.join(PACK, "hooks", "_lib.sh")


def _load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


CHK = _load(SCRIPT, "f58_readonly_check_under_test")
RO, CQ = "read_only", "consequential"
SS = "python3 \"$CYS_PACK_DIR/bin/javis_selfservice.py\""


class DesignCases(unittest.TestCase):
    """The seven T-6 instrument cases written in DESIGN-F58-final §7.4, verbatim."""

    def kind(self, cmd):
        return CHK.classify("Bash", {"command": cmd})[0]

    def test_doctor_fix_is_consequential(self):
        self.assertEqual(self.kind("cys doctor --fix"), CQ)

    def test_redirect_is_consequential(self):
        self.assertEqual(self.kind("cat a > b"), CQ)

    def test_command_substitution_is_consequential(self):
        self.assertEqual(self.kind("ls $(rm -rf x)"), CQ)

    def test_status_json_into_head_is_read_only(self):
        self.assertEqual(self.kind("cys status --json | head"), RO)

    def test_queue_deliver_is_consequential(self):
        self.assertEqual(self.kind("cys queue deliver 3"), CQ)

    def test_second_simple_command_is_judged(self):
        self.assertEqual(self.kind("head -n 5 f; rm f"), CQ)

    def test_tail_follow_is_consequential(self):
        self.assertEqual(self.kind("tail -f log"), CQ)


class PairedControls(unittest.TestCase):
    """Each refusal next to a control that differs only in the construct under test."""

    def pair(self, ro, cq):
        self.assertEqual(CHK.classify("Bash", {"command": ro})[0], RO, ro)
        self.assertEqual(CHK.classify("Bash", {"command": cq})[0], CQ, cq)

    def test_tools(self):
        for t in ("Read", "Grep", "Glob"):
            self.assertEqual(CHK.classify(t, {"file_path": "x"})[0], RO, t)
        for t in ("Edit", "Write", "NotebookEdit", "Agent", "WebFetch", None):
            self.assertEqual(CHK.classify(t, {})[0], CQ, t)

    def test_doctor_without_flag(self):
        self.pair("cys doctor", "cys doctor --fix")

    def test_separators(self):
        self.pair("head -n 5 f; wc -l f", "head -n 5 f; rm f")
        self.pair("ls && ls -la", "ls && rm -rf x")
        self.pair("ls || ls -la", "ls || touch x")
        self.pair("cys status --json | head -n 3", "cys status --json | tee out")
        self.pair("ls\nstat f", "ls\nrm f")

    def test_forbidden_constructs(self):
        self.pair("cat f", "cat f > g")
        self.pair("cat f", "cat f >> g")
        self.pair("cat f", "cat < f")
        self.pair("cat f", "cat <<EOF\nx\nEOF")
        self.pair("cat f", "cat <(ls)")
        self.pair("ls", "ls `rm x`")
        self.pair("ls", "ls &")
        self.pair("ls && ls", "ls & ls")
        self.pair("ls x", "ls 2>&1")
        # bash runs a substitution inside double quotes; shlex keeps the quoted word whole, so only the string
        # check of §7.4 item 3 sees these two
        self.pair('ls "x"', 'ls "$(rm -rf x)"')
        self.pair('ls "x"', 'ls "`rm -rf x`"')

    def test_unparsable_and_empty(self):
        for cmd in ("", "   ", "ls 'open", "(rm x)", "ls ;; ls", "ls |& head"):
            self.assertEqual(CHK.classify("Bash", {"command": cmd})[0], CQ, repr(cmd))
        self.assertEqual(CHK.classify("Bash", {})[0], CQ)

    def test_comment_does_not_hide_a_command(self):
        # shlex would end the word at '#' and swallow the rest of the line; bash keeps a#b as one word and runs rm.
        self.pair("ls a#b; ls", "ls a#b; rm f")

    def test_quoted_separator_is_an_operand(self):
        self.pair("cat 'a;b'", "cat a; rm b")

    def test_unix_flags(self):
        self.pair("tail -n 5 log", "tail -n 5 -f log")
        self.pair("tail -n5 log", "tail -fn 5 log")
        self.pair("tail -r log", "tail -F log")
        self.pair("tail --lines 3 log", "tail --follow log")
        self.pair("head -5 f", "head -5 -z f")
        self.pair("file -b f", "file -C -m magic")
        self.pair("stat -f %z f", "stat -Q f")
        self.pair("stat -t -Q f", "stat -Q -t x f")      # a flag's value is not read as a flag
        self.pair("wc -l f", "wc -X f")
        self.pair("shasum -a 256 f", "shasum -Z f")
        self.pair("ls -la /tmp", "ls -la /tmp --zzz")

    def test_not_allowlisted_heads(self):
        for cmd in ("rm f", "echo hi", "grep x f", "cd /", "sh -c ls", "true", "FOO=1 cys status", "/bin/ls",
                    "python3 -c 'print(1)'", "sudo ls"):
            self.assertEqual(CHK.classify("Bash", {"command": cmd})[0], CQ, cmd)

    def test_cys_verbs_and_flags(self):
        self.pair("cys status", "cys status --watch")
        self.pair("cys read-screen --surface 3", "cys read-screen --surface 3 --keys")
        self.pair("cys read-screen --surface=3 --composer", "cys read-screen --surface")
        self.pair("cys read-screen --surface=3", "cys read-screen --keys=x")
        self.pair("cys doctor", "cys doctor --fix=1")
        self.pair("cys queue list --surface 3 --json --full --expired", "cys queue list --drop")
        self.pair("cys list", "cys list extra")
        self.pair("cys identify", "cys identify --claim")
        self.pair("cys --version", "cys --version x")
        self.pair("cys pack-ownership --quiet directives/MASTER_DIRECTIVE.md", "cys pack-ownership --fix x")
        self.pair("cys skill show self-service", "cys skill show a b")
        self.pair("cys skill show self-service", "cys skill new x")
        for cmd in ("cys queue drop 3", "cys queue clear", "cys send-key --to master Return", "cys send hi",
                    "cys cycle-agent", "cys --socket /x status", "cys", "cys status -j"):
            self.assertEqual(CHK.classify("Bash", {"command": cmd})[0], CQ, cmd)

    def test_selfservice_tool(self):
        self.pair(SS + " triage --case 20261104T000000Z-abcdef", SS + " verify --case 20261104T000000Z-abcdef")
        self.pair(SS + " status", SS + " recheck")
        self.pair(SS + " open --symptom 'queue stuck'", SS + " report --case x")
        self.pair(SS + " corrections --role master --from T1-01:2 --summary", SS + " corrections --role master --x")
        self.pair(SS + " triage --case x --entry e1", SS + " triage --case x extra")
        self.pair(SS + " status --case x", SS + " consent --case x --lever L1 --yes")
        self.pair(SS + " status", SS + " attempt --case x --lever L1 --action a --evidence f")
        self.pair(SS + " status", SS + " retriage --case x")
        self.pair(SS + " status", "python3 \"$CYS_PACK_DIR/bin/other.py\" status")
        self.pair(SS + " status", "python3 -B \"$CYS_PACK_DIR/bin/javis_selfservice.py\" status")
        self.pair(SS + " status", "python3 \"$CYS_PACK_DIR/javis_selfservice.py\" status")


class Transcript(unittest.TestCase):
    """The walker reads one Claude session file: the turn after the 「이거 고쳐 줘:」 message only."""

    def write(self, recs):
        fd, p = tempfile.mkstemp(suffix=".jsonl")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            for r in recs:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
        self.addCleanup(os.unlink, p)
        return p

    @staticmethod
    def user(text):
        return {"type": "user", "message": {"role": "user", "content": text}}

    @staticmethod
    def asst(*items):
        return {"type": "assistant", "message": {"role": "assistant", "content": list(items)}}

    @staticmethod
    def bash(cmd):
        return {"type": "tool_use", "name": "Bash", "input": {"command": cmd}}

    def test_first_consequential_and_text_before(self):
        p = self.write([
            self.user("앞선 대화"),
            self.asst({"type": "text", "text": "다른 턴"}, self.bash("rm earlier")),
            self.user("이거 고쳐 줘: 큐가 막혔다"),
            self.asst({"type": "text", "text": "우리가 직접 고칠 수 있습니다."}, self.bash("cys status --json")),
            {"type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]}},
            self.asst({"type": "text", "text": "먼저 큐를 보겠습니다."}, self.bash("cys queue drop 3"),
                      {"type": "text", "text": "뒤의 글"}, self.bash("ls")),
            self.user("다음 사용자 글"),
            self.asst(self.bash("rm later")),
        ])
        res = CHK.walk_transcript(p, "이거 고쳐 줘:")
        self.assertEqual([c["kind"] for c in res["calls"]], [RO, CQ, RO])
        self.assertEqual(res["first_consequential"], 1)
        self.assertEqual(res["text_before_first"], "우리가 직접 고칠 수 있습니다.\n먼저 큐를 보겠습니다.")

    def test_prefix_after_leading_blank(self):
        p = self.write([self.user("\n 이거 고쳐 줘: x"), self.asst(self.bash("rm x"))])
        self.assertEqual(CHK.walk_transcript(p, "이거 고쳐 줘:")["first_consequential"], 0)

    def test_no_consequential_call(self):
        p = self.write([self.user("이거 고쳐 줘: x"), self.asst(self.bash("ls"), {"type": "tool_use",
                                                                         "name": "Read", "input": {}})])
        res = CHK.walk_transcript(p, "이거 고쳐 줘:")
        self.assertIsNone(res["first_consequential"])
        self.assertEqual(len(res["calls"]), 2)

    def test_missing_message_is_none_and_cli_exit_2(self):
        p = self.write([self.user("hello"), self.asst(self.bash("ls"))])
        self.assertIsNone(CHK.walk_transcript(p, "이거 고쳐 줘:"))
        self.assertEqual(CHK.main(["transcript", p]), 2)
        self.assertEqual(CHK.main(["transcript", p + ".absent"]), 2)


class SkillText(unittest.TestCase):
    """Facts the shipped skill states must match the code it points at (not its wording)."""

    @classmethod
    def setUpClass(cls):
        with open(SKILL, encoding="utf-8") as f:
            cls.skill = f.read()
        with open(LEVERS, encoding="utf-8") as f:
            cls.levers = f.read()
        cls.tool = _load(TOOL, "javis_selfservice_for_skill_test")
        with open(LIB, encoding="utf-8") as f:
            cls.lib = f.read()

    def test_frontmatter_and_four_cells(self):
        m = re.match(r"---\nname: self-service\ndescription: (.+)\n---\n", self.skill)
        self.assertIsNotNone(m, "frontmatter must open the file with name: self-service")
        for cell in ("## 언제", "## 순서", "## 주의", "## 확인"):
            self.assertEqual(self.skill.count("\n" + cell), 1, cell)
        self.assertIn("references/levers.md", self.skill)

    def test_every_named_verb_exists(self):
        verbs = {c for a in self.tool.build_parser()._actions
                 if isinstance(a, argparse._SubParsersAction) for c in a.choices}
        named = set(re.findall(r"javis_selfservice\.py\"? ([a-z]+)", self.skill + self.levers))
        self.assertTrue(named, "the skill names no verb")
        self.assertEqual(sorted(named - verbs), [])
        self.assertEqual(sorted(verbs - named), [], "every verb of the tool has a place in the procedure")

    def test_first_answers_equal_the_hook_and_design(self):
        s = "우리가 직접 고칠 수 있습니다. 먼저 <할 일> 하겠습니다."
        r = "우리가 직접 할 수 있는 것부터 하겠습니다: <할 일>."
        for form in (s, r):
            self.assertIn(form, self.lib, "hook block carries " + form)
            self.assertIn("「" + form + "」", self.skill)

    def test_questions_are_the_tools_constants(self):
        self.assertIn("「" + self.tool.SR_QUESTION + "」", self.skill)
        self.assertIn("「" + self.tool.DEFECT_QUESTION + "」", self.skill)

    def test_three_ask_once_cases_equal_the_hook(self):
        line = "돈 · 사용자 자료의 되돌릴 수 없는 손실 · 공개 쓰기(git push·발송·발행·제조사 보고)"
        self.assertIn(line, self.lib)
        self.assertIn(line, self.skill)

    def test_lever_classes_equal_the_table(self):
        with open(os.path.join(PACK, "round", "self_service_levers.json"), encoding="utf-8") as f:
            classes = json.load(f)["classes"]
        for cid in classes:
            self.assertEqual(len(re.findall(r"^## %s " % cid, self.levers, re.M)), 1, cid)
        self.assertEqual(len(re.findall(r"^## T\d ", self.levers, re.M)), len(classes))


if __name__ == "__main__":
    unittest.main()
