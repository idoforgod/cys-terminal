#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_capgate_boot_critical.py — 기동 필수 파일 조언 관문의 종단 검체(0.15.0 · T5a · 설계 §4-7).

무엇을 재는가(CONTRACTS C4 4.8 · O-C4-5 · TESTPLAN_RULINGS OQ-D1 · testplan D-1-01…D-1-10):
  ① 등록된 능력 게이트 훅(셸 + 파이썬 판정기)을 **실제로** 기록된 PreToolUse 입력으로 돌린다 —
     살아 있는 기동 필수 파일에 대한 직접 쓰기는 거부되고, 사유는 그 파일과 `cys repair submit` 을 가리킨다.
  ② 비용 규칙: 읽기·팩 밖 쓰기는 `cys` 를 띄우지 않고, 쓰기 한 번에 `cys` 는 많아야 한 번이며,
     자식은 `CYS_NO_AUTOSTART=1` 을 받는다(부록 A 1-17).
  ③ fail-open: 이진 부재·비정상 종료·출력 불량·무응답 → 통과(15초 훅 시한 안에 돌아온다).
  ④ 파이썬 부재 비대칭(reviewer fail-closed · CSO 강등)은 그대로다(부록 A 1-29).
  ⑤ 안전 검사를 지운 사본에서는 내장 self-test 가 실패한다(음성 대조 — 통과 조건 H).
  ⑥ 빌드된 `cys`(있을 때: `$CARGO_TARGET_DIR/debug/cys`)를 상대로 D-1-01…03 의 경로 변형
     v1–v5 와 대조군을 격리 HOME 에서 잰다(본체가 판정 — 훅은 정규화하지 않는다).

라이브 무접촉: HOME·TMPDIR·CYS_STATE_DIR·CYS_PACK_DIR 은 임시 디렉터리, `CYS_SOCKET` 은 없는 경로,
`cys` 는 스텁이거나 격리 HOME 의 빌드 산출물이며 데몬에 접속하지 않는다(질의는 로컬 명령이다).
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

BIN = Path(__file__).resolve().parent.parent
PACK = BIN.parent
HOOK = PACK / "hooks" / "role-capability-gate.sh"
LIB = PACK / "hooks" / "_lib.sh"
sys.dont_write_bytecode = True
SH = shutil.which("bash") or shutil.which("sh")
NEED_SH = unittest.skipIf(not SH or os.name == "nt", "bash·sh 부재 또는 Windows — 셸 종단 검체 실행 불가")

REL = "hooks/inject-context.sh"


def _clean_path():
    """실 `cys` 가 들어 있는 PATH 항목을 뺀다(라이브 데몬에 묻지 않게)."""
    out = []
    for d in (os.environ.get("PATH", "") or "").split(os.pathsep):
        if d and not any(os.path.exists(os.path.join(d, n)) for n in ("cys", "cys.exe")):
            out.append(d)
    return out


def _built_cys():
    td = os.environ.get("CARGO_TARGET_DIR")
    if not td:
        return None
    p = os.path.join(td, "debug", "cys")
    try:
        if os.path.isfile(p) and os.access(p, os.X_OK) and os.path.getsize(p) > 0:
            return p
    except OSError:
        pass
    return None


@NEED_SH
class _Env(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="t5a-gate-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(os.path.realpath(tmp.name))
        self.home = self.root / "home"
        self.pack = self.home / ".cys" / "pack"
        self.dept = self.home / ".cys" / "pack-dept-d1"
        self.tmpdir = self.root / "tmp"
        self.fakebin = self.root / "fakebin"
        for d in (self.pack / "hooks", self.pack / "bin", self.dept / "hooks", self.dept / "bin",
                  self.tmpdir, self.fakebin, self.home / ".cys" / "state"):
            d.mkdir(parents=True, exist_ok=True)
        for p in (self.pack, self.dept):
            (p / REL).write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            (p / "hooks" / "README.md").write_text("x\n", encoding="utf-8")
            (p / "bin" / "javis_bootstrap.py").write_text("x = 1\n", encoding="utf-8")
            (p / "agents.json").write_text("{}\n", encoding="utf-8")
        self.calls = self.root / "calls.log"
        self.env = dict(os.environ)
        for k in list(self.env):
            if k.startswith(("CYS_", "JAVIS_", "AITERM_", "CAPGATE_")):
                self.env.pop(k)
        self.env.update({
            "HOME": str(self.home),
            "TMPDIR": str(self.tmpdir),
            "CYS_STATE_DIR": str(self.home / ".cys" / "state"),
            "CYS_PACK_DIR": str(self.pack),
            "CYS_PY": sys.executable,
            "CYS_ROLE": "worker",
            "CYS_SOCKET": str(self.root / "no-daemon.sock"),
            "CYS_NO_AUTOSTART": "1",
            "CYS_NO_OFFICE_BRIDGE": "1",
            "CYS_NO_PERSONAL_HOOK_MERGE": "1",
            "CYS_NO_AUTORESTORE": "1",
            "PATH": os.pathsep.join([str(self.fakebin)] + _clean_path()),
        })

    def fake(self, name, body):
        """`CYS_BIN` 스텁 — 받은 인자와 CYS_NO_AUTOSTART 를 기록한다(실제 자식 환경으로 잰다)."""
        p = self.fakebin / name
        p.write_text("#!/bin/sh\nprintf '%%s|%%s\\n' \"${CYS_NO_AUTOSTART:-unset}\" \"$*\" >> '%s'\n%s"
                     % (self.calls, body), encoding="utf-8")
        p.chmod(0o755)
        return str(p)

    def yes_json(self, rel=REL):
        return "printf '%s'\n" % json.dumps({"decision": "yes", "rel": rel})

    def calls_seen(self):
        return self.calls.read_text(encoding="utf-8").splitlines() if self.calls.exists() else []

    def run_hook(self, tool, tool_input, cwd=None, hook=None, **envkw):
        env = dict(self.env)
        for k, v in envkw.items():
            if v is None:
                env.pop(k, None)        # 키 제거(예: 게이트가 스스로 싣는지 재려고 CYS_NO_AUTOSTART 를 뺀다)
            else:
                env[k] = str(v)
        payload = {"session_id": "t5a", "tool_name": tool, "tool_input": tool_input,
                   "hook_event_name": "PreToolUse"}
        if cwd is not None:
            payload["cwd"] = str(cwd)
        t0 = time.time()
        r = subprocess.run([SH, str(hook or HOOK)], input=json.dumps(payload), env=env,
                           cwd=str(cwd) if cwd else str(self.root),
                           capture_output=True, text=True, timeout=60)
        return r, time.time() - t0

    @staticmethod
    def decision(r):
        try:
            hso = json.loads(r.stdout or "{}").get("hookSpecificOutput") or {}
        except ValueError:
            return None, ""
        return hso.get("permissionDecision"), hso.get("permissionDecisionReason", "")


class GateWithStub(_Env):
    """훅 종단 — 판정은 스텁 `cys`(CYS_BIN)가 낸다."""

    def test_d1_01_direct_write_denied_and_points_to_submit(self):
        target = str(self.pack / REL)
        for role in ("worker", "master"):
            for tool, ti in (("Edit", {"file_path": target, "old_string": "a", "new_string": "b"}),
                             ("Write", {"file_path": target, "content": "x"}),
                             ("Bash", {"command": "echo x > %s" % target})):
                self.calls.unlink() if self.calls.exists() else None
                # CYS_NO_AUTOSTART 를 호출 env 에서 빼고 돌린다 — 자식이 받은 1 은 게이트가 실은 값이어야 한다
                r, _ = self.run_hook(tool, ti, CYS_BIN=self.fake("yes", self.yes_json()), CYS_ROLE=role,
                                     CYS_NO_AUTOSTART=None)
                dec, reason = self.decision(r)
                self.assertEqual(r.returncode, 0)
                self.assertEqual(dec, "deny", "%s/%s 가 거부되지 않았다: %r %r" % (role, tool, r.stdout, r.stderr))
                self.assertIn(REL, reason)
                self.assertIn("기동 필수", reason)
                # RULINGS CY2 — the sentence says what is true: startup OR a hook's essential function
                self.assertIn("망가지면 제품이 켜지지 않거나, 훅이 하는 필수 기능이 꺼질 수 있다", reason)
                self.assertNotIn("(망가지면 제품이 켜지지 않는다)", reason)
                if role == "worker":     # RULINGS AI5 — the worker opens the repair and submits it
                    self.assertIn("cys repair begin --files %s" % REL, reason)
                    self.assertIn("cys repair submit", reason)
                    # RULINGS AM2 / AN6 — `--trigger <계기>` + the three values in words, no `|` to run
                    self.assertIn("--trigger <계기>", reason)
                    self.assertIn("`owner-order` · `grant` · `update`", reason)
                    self.assertNotIn("owner-order|grant|update", reason)
                else:                    # master / CEO: a ticket to a worker of the lane, never begin/submit
                    self.assertIn("티켓", reason)
                    self.assertNotIn("cys repair submit", reason)
                seen = self.calls_seen()
                self.assertEqual(seen, ["1|repair boot-critical --path %s --json" % target],
                                 "cys 한 번 · CYS_NO_AUTOSTART=1 이어야 한다: %r" % seen)

    def test_role_less_session_passes_untouched(self):
        """RULINGS AI4 — 좌석 역할이 없는 세션(주인 자신 · 사람 · 일반 세션)은 종전 그대로 통과하고
        `cys` 를 띄우지도 않는다."""
        target = str(self.pack / REL)
        for tool, ti in (("Write", {"file_path": target, "content": "x"}),
                         ("Edit", {"file_path": target, "old_string": "a", "new_string": "b"}),
                         ("Bash", {"command": "echo x > %s" % target})):
            r, _ = self.run_hook(tool, ti, CYS_BIN=self.fake("yes", self.yes_json()), CYS_ROLE="")
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertNotEqual(self.decision(r)[0], "deny", "역할 없는 세션이 거부됐다(AI4)")
        self.assertEqual(self.calls_seen(), [], "역할 없는 세션에 cys 를 띄웠다")

    def test_other_lanes_pack_is_named(self):
        """RULINGS AI5 — 다른 레인의 팩 파일이면 그 팩을 문면에 적는다."""
        target = str(self.dept / REL)
        body = "printf '%s'\n" % json.dumps({"decision": "yes", "rel": REL, "lane_pack": str(self.dept)})
        r, _ = self.run_hook("Write", {"file_path": target, "content": "x"}, CYS_BIN=self.fake("dept", body))
        dec, reason = self.decision(r)
        self.assertEqual(dec, "deny")
        self.assertIn(str(self.dept), reason)
        self.assertIn("다른 레인", reason)

    def test_control_not_on_list_is_allowed(self):
        target = str(self.pack / "hooks" / "README.md")
        r, _ = self.run_hook("Write", {"file_path": target, "content": "x"},
                             CYS_BIN=self.fake("no", "printf '{\"decision\":\"no\"}'; exit 1\n"))
        self.assertEqual((r.returncode, r.stdout.strip()), (0, ""))

    def test_reads_and_non_pack_writes_never_spawn(self):
        stub = self.fake("yes", self.yes_json())
        for tool, ti in (("Read", {"file_path": str(self.pack / REL)}),
                         ("Grep", {"pattern": "x", "path": str(self.pack)}),
                         ("Bash", {"command": "cat %s" % (self.pack / REL)}),
                         ("Bash", {"command": "sed -n 1p %s" % (self.pack / REL)}),
                         ("Write", {"file_path": str(self.root / "elsewhere.txt"), "content": "x"}),
                         ("Bash", {"command": "echo x > %s" % (self.root / "elsewhere.txt")})):
            r, _ = self.run_hook(tool, ti, CYS_BIN=stub)
            self.assertNotEqual(self.decision(r)[0], "deny", "%s %r" % (tool, ti))
        self.assertEqual(self.calls_seen(), [], "읽기·팩 밖 쓰기에 cys 를 띄웠다")

    def test_one_process_per_tool_call(self):
        cmd = "echo a > %s; echo b > %s; rm %s" % (self.pack / REL, self.dept / REL,
                                                    self.pack / "hooks" / "README.md")
        r, _ = self.run_hook("Bash", {"command": cmd}, CYS_BIN=self.fake("yes", self.yes_json()))
        self.assertEqual(self.decision(r)[0], "deny")
        self.assertEqual(len(self.calls_seen()), 1)

    def test_fail_open_on_every_error(self):
        target = str(self.pack / REL)
        cases = {
            "missing": str(self.fakebin / "no-such-cys"),
            "nonzero": self.fake("nonzero", "exit 1\n"),
            "garbage": self.fake("garbage", "echo not-json\n"),
            "undecidable": self.fake("undecidable", "printf '{\"decision\":\"undecidable\"}'; exit 2\n"),
            "hang": self.fake("hang", "exec sleep 30\n"),
        }
        for name, stub in cases.items():
            r, dt = self.run_hook("Write", {"file_path": target, "content": "x"}, CYS_BIN=stub)
            self.assertEqual(r.returncode, 0, "%s: %r" % (name, r.stderr))
            self.assertNotEqual(self.decision(r)[0], "deny", "%s 가 거부로 접혔다(fail-open 위반)" % name)
            self.assertLess(dt, 12.0, "%s: 훅이 %.1fs 걸렸다(선언 시한 15s)" % (name, dt))

    def test_reviewer_refusal_stays_t10s_and_spawns_nothing(self):
        """T10(reviewer 쓰기 정확 목록)과의 상호작용 — reviewer 좌석의 기동 필수 파일 쓰기는 **T10 규칙이**
        거부한다(그 거부를 이 조언 관문이 허용으로 바꾸지 않고, 이미 거부된 쓰기에 `cys` 를 띄우지 않는다).
        T10 이 허용한 reviewer 쓰기(판정 파일 자리)는 이 관문이 한 번 묻고, 본체 답이 '아님'이면 통과한다."""
        target = str(self.pack / REL)
        if target.startswith(("/tmp/", "/private/tmp/", "/var/tmp/", "/var/folders/")):
            self.skipTest("검체 팩이 reviewer 임시 폴더 허용 접두 아래라 T10 거부를 재현할 수 없다: %s" % target)
        stub = self.fake("yes", self.yes_json())
        for tool, ti in (("Write", {"file_path": target, "content": "x"}),
                         ("Edit", {"file_path": target, "old_string": "a", "new_string": "b"}),
                         ("Bash", {"command": "echo x > %s" % target})):
            r, _ = self.run_hook(tool, ti, CYS_ROLE="reviewer-codex", CYS_BIN=stub)
            dec, reason = self.decision(r)
            self.assertEqual(dec, "deny", "%s: reviewer 쓰기가 거부되지 않았다: %r" % (tool, r.stderr[-300:]))
            self.assertIn("producer != evaluator", reason, "%s: T10 의 거부가 아니다: %r" % (tool, reason))
            self.assertNotIn("기동 필수 파일", reason)
        self.assertEqual(self.calls_seen(), [], "T10 이 이미 거부한 쓰기에 cys 를 띄웠다")
        verdict = self.pack / "round" / "_reviews" / "v1.json"
        verdict.parent.mkdir(parents=True, exist_ok=True)
        r, _ = self.run_hook("Write", {"file_path": str(verdict), "content": "{}"}, CYS_ROLE="reviewer-codex",
                             CYS_BIN=self.fake("no", "printf '{\"decision\":\"no\"}'; exit 1\n"))
        self.assertNotEqual(self.decision(r)[0], "deny", "T10 이 허용한 판정 파일 쓰기가 막혔다: %r" % r.stdout)
        self.assertEqual(len(self.calls_seen()), 1, "팩 아래 쓰기는 본체에 한 번 묻는다")

    def test_no_python_asymmetry_unchanged(self):
        """부록 A 1-29 — python 이 없으면 reviewer 는 fail-closed(2), CSO·worker 는 열림(0)."""
        need = ["date", "head", "tr", "dirname", "mktemp", "cat", "rm", "mv", "id",
                "printf", "sed", "uname", "wc"]
        d = self.root / "nopy"
        d.mkdir()
        for t in need:
            src = shutil.which(t)
            if src:
                os.symlink(src, str(d / t))
        if not (d / "mktemp").exists():
            self.skipTest("coreutils 심링크 불가")
        env = {"PATH": str(d), "CYS_PY": ""}
        target = str(self.pack / REL)
        rev, _ = self.run_hook("Edit", {"file_path": target}, CYS_ROLE="reviewer-codex", **env)
        self.assertEqual(rev.returncode, 2, rev.stderr)
        for role in ("cso", "worker"):
            r, _ = self.run_hook("Write", {"file_path": target, "content": "x"}, CYS_ROLE=role, **env)
            self.assertEqual(r.returncode, 0, "%s: %r" % (role, r.stderr))
            self.assertNotEqual(self.decision(r)[0], "deny")


class GateNegativeControls(_Env):
    """★음성 대조 — 이 조언 관문의 안전 검사를 지운 사본에서는 `--self-test` 가 실패해야 한다."""

    MUTATIONS = [
        ("자식에 CYS_NO_AUTOSTART=1(부록 A 1-17)",
         '    env["CYS_NO_AUTOSTART"] = "1"', "    pass"),
        ("팩 밖·읽기는 띄우지 않는다(O-C4-5)",
         "            if _bc_under_pack_root(p):", "            if True:"),
        ("판정 'yes' 일 때만 거부(fail-open)",
         '        if not isinstance(ans, dict) or ans.get("decision") != "yes":',
         '        if isinstance(ans, dict) and ans.get("decision") == "no":'),
        ("비정상 종료는 답이 아니다(fail-open)",
         "    if r.returncode != 0:\n        return None\n    try:\n        v = json.loads(r.stdout",
         "    try:\n        v = json.loads(r.stdout"),
        ("짧은 시한(무응답 fail-open)",
         "timeout=min(BOOT_CRITICAL_QUERY_TIMEOUT_S, left - 0.5))",
         "timeout=min(BOOT_CRITICAL_QUERY_TIMEOUT_S, left - 0.5) + 5)"),
        ("읽기 sed 는 쓰기가 아니다",
         '        elif base == "sed" and any(', '        elif base == "sed" and True or any('),
        ("좌석 역할이 없으면 보지 않는다(AI4)",
         "        seat = _bc_seat_role(roles)\n        if not seat:\n            return None",
         "        seat = _bc_seat_role(roles) or \"worker\"\n        if not seat:\n            return None"),
        ("예외는 통과(fail-open)",
         "    except Exception:\n        return None\n\n\ndef _json_escape",
         "    except KeyError:\n        return None\n\n\ndef _json_escape"),
    ]

    def test_removing_a_guard_fails_self_test(self):
        src = HOOK.read_text(encoding="utf-8")
        env = dict(self.env)
        base = subprocess.run([SH, str(HOOK), "--self-test"], capture_output=True, text=True,
                              timeout=180, env=env)
        self.assertEqual(base.returncode, 0, "기준 self-test 가 이미 실패한다: %s" % base.stderr[-600:])
        self.assertIn("self-test OK", base.stdout)
        with tempfile.TemporaryDirectory(prefix="t5a-mut-") as td:
            shutil.copy2(str(LIB), os.path.join(td, "_lib.sh"))
            blind, missing = [], []
            for label, old, new in self.MUTATIONS:
                if src.count(old) != 1:
                    missing.append(label)
                    continue
                p = os.path.join(td, "mut.sh")
                with open(p, "w", encoding="utf-8") as f:
                    f.write(src.replace(old, new))
                r = subprocess.run([SH, p, "--self-test"], capture_output=True, text=True,
                                   timeout=180, env=env)
                if r.returncode == 0:
                    blind.append(label)
            self.assertEqual(missing, [], "음성 대조 패턴이 코드와 어긋난다: %s" % missing)
            self.assertEqual(blind, [], "검사를 지워도 self-test 가 통과한다: %s" % blind)

    def test_main_calls_the_advisory_block(self):
        """main() 의 호출 한 줄을 지우면 종단 거부가 사라진다(self-test 는 main 을 돌지 않는다)."""
        src = HOOK.read_text(encoding="utf-8")
        old = ("    bc_reason = None if block else boot_critical_deny_reason(tool, tool_input, data.get(\"cwd\"),\n"
               "                                                             roles=roles)")
        self.assertEqual(src.count(old), 1)
        with tempfile.TemporaryDirectory(prefix="t5a-mut-") as td:
            shutil.copy2(str(LIB), os.path.join(td, "_lib.sh"))
            p = os.path.join(td, "mut.sh")
            with open(p, "w", encoding="utf-8") as f:
                f.write(src.replace(old, "    bc_reason = None"))
            env = dict(self.env, CYS_BIN=self.fake("yes", self.yes_json()))
            payload = json.dumps({"session_id": "t5a", "tool_name": "Write",
                                  "tool_input": {"file_path": str(self.pack / REL), "content": "x"}})
            mut = subprocess.run([SH, p], input=payload, env=env, capture_output=True, text=True, timeout=60)
            real = subprocess.run([SH, str(HOOK)], input=payload, env=env, capture_output=True,
                                  text=True, timeout=60)
            self.assertEqual(self.decision(real)[0], "deny")
            self.assertNotEqual(self.decision(mut)[0], "deny")


@unittest.skipIf(_built_cys() is None, "빌드된 cys 없음($CARGO_TARGET_DIR/debug/cys) — 본체 대조 미측정")
class GateWithBuiltBinary(_Env):
    """D-1-01…03 — 빌드된 본체를 상대로 경로 변형 v1–v5 와 대조군(격리 HOME)."""

    def setUp(self):
        super().setUp()
        self.env["CYS_BIN"] = _built_cys()
        self.outside = self.root / "outside"
        self.outside.mkdir()
        os.symlink(str(self.pack / "hooks"), str(self.outside / "hooks-link"))
        os.symlink(str(self.pack / "bin"), str(self.outside / "bin-link"))

    def variants(self, rel):
        d, leaf = rel.rsplit("/", 1) if "/" in rel else ("", rel)
        out = [
            (str(self.pack / rel), None),                                  # 절대
            ("./" + rel, self.pack),                                        # v1
            ("%s/../%s" % (d, rel) if d else "bin/../" + rel, self.pack),   # v2 ..
            ("../" + rel, self.pack / "bin"),                               # v2 from bin/
            (rel.replace(leaf, leaf.title()).replace(d, d.title()) if d else rel.upper(), self.pack),  # v3
            (str(self.dept / rel), None),                                   # v5 다른 레인
        ]
        if d in ("hooks", "bin"):
            out.append((str(self.outside / ("%s-link" % d) / leaf), None))  # v4 심링크 부모
        return out

    def test_binary_answers_list_locally(self):
        r = subprocess.run([self.env["CYS_BIN"], "repair", "boot-critical", "--list", "--json"],
                           env=self.env, capture_output=True, text=True, timeout=20)
        self.assertEqual(r.returncode, 0, r.stderr)
        doc = json.loads(r.stdout)
        self.assertEqual(doc["schema"], 1)
        rels = {e["rel"] for e in doc["entries"]}
        for rel in (REL, "bin/javis_bootstrap.py", "agents.json"):
            self.assertIn(rel, rels)
        self.assertFalse(any(":" in r for r in rels), "등록 항목이 섞였다(W1/Y3)")
        self.assertFalse(os.path.exists(self.env["CYS_SOCKET"]), "질의가 소켓을 만들었다")

    def test_query_never_contacts_a_daemon(self):
        """4.7 / 부록 A 1-17 — 질의는 데몬에 닿지 않는다: `CYS_SOCKET` 에 리스너를 열어 두고
        `--path`·`--list` 를 물어도 연결이 하나도 오지 않는다(소켓이 살아 있으니 자동 기동도 없다)."""
        import socket
        sock_path = os.path.join(tempfile.mkdtemp(prefix="t5a-s", dir="/tmp"), "d.sock")
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(srv.close)
        self.addCleanup(shutil.rmtree, os.path.dirname(sock_path), True)
        srv.bind(sock_path)
        srv.listen(8)
        srv.setblocking(False)
        env = dict(self.env, CYS_SOCKET=sock_path)
        for argv in (["--path", str(self.pack / REL), "--json"], ["--list", "--json"],
                     ["--path", str(self.pack / "hooks" / "README.md")]):
            r = subprocess.run([self.env["CYS_BIN"], "repair", "boot-critical"] + argv, env=env,
                               capture_output=True, text=True, timeout=20)
            self.assertIn(r.returncode, (0, 1), r.stderr)
        try:
            conn, _ = srv.accept()
            conn.close()
            self.fail("cys repair boot-critical 가 데몬 소켓에 접속했다")
        except BlockingIOError:
            pass

    def test_d1_kinds_and_path_variants_denied(self):
        for rel in (REL, "bin/javis_bootstrap.py", "agents.json"):
            for spelled, cwd in self.variants(rel):
                for tool, ti in (("Write", {"file_path": spelled, "content": "x"}),
                                 ("Bash", {"command": "echo x > %s" % spelled})):
                    if tool == "Write" and not os.path.isabs(spelled):
                        continue        # 파일 도구는 절대 경로를 넘긴다 — 상대 철자는 셸 경로로 잰다
                    r, _ = self.run_hook(tool, ti, cwd=cwd)
                    dec, reason = self.decision(r)
                    self.assertEqual(dec, "deny", "%s %s (cwd=%s) 가 거부되지 않았다: %r %r"
                                     % (tool, spelled, cwd, r.stdout, r.stderr[-300:]))
                    self.assertIn("cys repair submit", reason)

    def test_in_pack_aliases_are_denied(self):
        """RULINGS CY1 — the J3 counterexample end to end through the gate and the built binary: a
        shell redirection (and a Write) to a listed file through an alias INSIDE the pack — an alias
        directory, a chain of two links, an alias of the file, `..` after a link — is denied for a
        worker and a master; an alias pointing OUT of the pack names no row of this lane.
        RULINGS DY2 — "the live file is untouched" is backed by real attempts: every call is followed
        by what the harness does with it (the tool runs unless the hook blocks or denies), so a missed
        deny writes the live file; the out-of-pack call, not denied, runs and its write is observed
        (positive control: the emulation really writes)."""
        p = self.pack
        (p / "hooks" / "sub").mkdir(exist_ok=True)
        (p / "notes").mkdir(exist_ok=True)
        os.symlink(str(p / "hooks"), str(p / "hooks-alias"))
        os.symlink(str(p / "a2"), str(p / "a1"))
        os.symlink(str(p / "hooks"), str(p / "a2"))
        os.symlink(str(p / REL), str(p / "notes" / "ic-copy.sh"))
        os.symlink(str(p / "hooks" / "sub"), str(p / "notes" / "sub-link"))
        os.symlink(str(self.outside), str(p / "out-alias"))
        before = (p / REL).read_text(encoding="utf-8")
        self.assertNotEqual(before, "x")
        leaf = REL.rsplit("/", 1)[1]
        missed = []
        for spelled in ("hooks-alias/" + leaf, "a1/" + leaf, "notes/ic-copy.sh", "notes/sub-link/../" + leaf):
            target = "%s/%s" % (p, spelled)
            for role in ("worker", "master"):
                for tool, ti in (("Bash", {"command": "printf x > %s" % target}),
                                 ("Write", {"file_path": target, "content": "x"})):
                    r, _ = self.run_hook(tool, ti, CYS_ROLE=role)
                    dec, reason = self.decision(r)
                    if self.attempt(tool, ti, r) or REL not in reason:
                        missed.append("%s %s %s: %r %r" % (role, tool, spelled, r.stdout, r.stderr[-300:]))
        self.assertEqual(missed, [], "거부되지 않았다(또는 사유가 행을 가리키지 않는다)")
        self.assertEqual((p / REL).read_text(encoding="utf-8"), before, "살아 있는 파일이 바뀌었다")
        ti = {"command": "printf x > %s/out-alias/%s" % (p, leaf)}
        r, _ = self.run_hook("Bash", ti)
        self.assertNotEqual(self.decision(r)[0], "deny", "팩 밖을 가리키는 별칭은 이 레인의 행이 아니다")
        self.assertTrue(self.attempt("Bash", ti, r), "the out-of-pack call was not run")
        self.assertEqual((self.outside / leaf).read_text(encoding="utf-8"), "x",
                         "positive control: the harness emulation did not write")

    def attempt(self, tool, ti, r):
        """What the harness does with a PreToolUse answer: the tool runs unless the hook blocked (exit 2)
        or denied. Runs it for real (Bash through the shell, Write as a file write); True if it ran."""
        if r.returncode == 2 or self.decision(r)[0] == "deny":
            return False
        if tool == "Bash":
            subprocess.run([SH, "-c", ti["command"]], cwd=str(self.root), capture_output=True, timeout=30)
        elif tool == "Write":
            Path(ti["file_path"]).write_text(ti["content"], encoding="utf-8")
        return True

    def test_control_variants_allowed(self):
        rel = "hooks/README.md"
        for spelled, cwd in self.variants(rel):
            r, _ = self.run_hook("Bash", {"command": "echo x > %s" % spelled}, cwd=cwd)
            self.assertNotEqual(self.decision(r)[0], "deny", "%s (cwd=%s)" % (spelled, cwd))
        staging = self.home / ".cys" / "local" / "repairs" / "R-1" / "staging" / "hooks"
        staging.mkdir(parents=True)
        r, _ = self.run_hook("Write", {"file_path": str(staging / "inject-context.sh"), "content": "x"})
        self.assertNotEqual(self.decision(r)[0], "deny", "작업 사본 쓰기가 막혔다")


NEW_START = "\"$CYS_PY\" -I -S -B -X utf8 - <<'PYEOF'\n"
# the starts of the earlier heads, rebuilt in a copy of the hook (`exec` = the shell is replaced, so the
# judge's own blocking exit must be 2 again in such a copy — see `_CG_RC_BLOCK`)
START_017768BF = ("exec \"$CYS_PY\" -B -s - <<'PYEOF'\nimport sys\n"
                  "sys.path[:] = [p for p in sys.path if p and (p[0] in \"/\\\\\" or p[1:3] in (\":\\\\\", \":/\"))]\n")
START_DBEC50E0 = "exec \"$CYS_PY\" - <<'PYEOF'\n"


class JudgeIsolation(_Env):
    """RULINGS CX1 → DY1 — the judge's interpreter starts isolated: its one module source is the
    interpreter's own standard library, decided by start-up flags (`-I -S -B -X utf8`) before it starts.
    A module planted in an allowed (temporary) working directory is NOT executed, while a legitimate
    verdict / own-TODO / temp write still passes and a forbidden write is still refused — in the base
    lane, a department lane and a custom pack dir. Negative controls: copies of the hook with the starts
    of earlier heads DO run the planted code — the tests see the faults they guard against."""

    def hook_with_start(self, start, tag):
        """A copy of the hook (+ `_lib.sh`) whose interpreter start is `start` — the judge body unchanged."""
        src = HOOK.read_text(encoding="utf-8")
        self.assertEqual(src.count(NEW_START), 1, "the isolated start is not in the hook")
        old = src.replace(NEW_START, start)
        if start.startswith("exec "):
            self.assertEqual(old.count("_CG_RC_BLOCK = 77\n"), 1)
            old = old.replace("_CG_RC_BLOCK = 77\n", "_CG_RC_BLOCK = 2\n")
        copy = self.root / ("hooks-" + tag)
        copy.mkdir()
        (copy / "role-capability-gate.sh").write_text(old, encoding="utf-8")
        shutil.copy(str(LIB), str(copy / "_lib.sh"))
        return copy / "role-capability-gate.sh"

    def plant(self):
        cwd = self.tmpdir / "review-ws"
        cwd.mkdir(exist_ok=True)
        marker = self.root / "PLANTED-RAN"
        for mod in ("json", "hashlib", "tempfile"):
            (cwd / ("%s.py" % mod)).write_text(
                "open(%r, 'a').write(%r)\nraise SystemExit(0)\n" % (str(marker), mod + "\n"), encoding="utf-8")
        return cwd, marker

    def reviewer(self, tool_input, cwd, pack, hook=None):
        return self.run_hook("Write", tool_input, cwd=cwd, hook=hook, CYS_ROLE="reviewer-codex",
                             CYS_PACK_DIR=str(pack))

    def test_planted_module_is_not_executed_and_writes_still_pass(self):
        cwd, marker = self.plant()
        custom = self.root / "custom-pack"
        for name, pack in (("base", self.pack), ("department", self.dept), ("custom", custom)):
            (pack / "round" / "_reviews").mkdir(parents=True, exist_ok=True)
            for target in (pack / "round" / "_reviews" / "verdict-x.md",
                           pack / "round" / "REVIEWER_CODEX_TODO.md", cwd / "notes.md"):
                r, _ = self.reviewer({"file_path": str(target), "content": "x"}, cwd, pack)
                self.assertFalse(marker.exists(), "%s: a planted module ran inside the judge: %r"
                                 % (name, marker.read_text(encoding="utf-8") if marker.exists() else ""))
                self.assertEqual(r.returncode, 0, "%s %s: %r" % (name, target, r.stderr[-300:]))
                self.assertNotEqual(self.decision(r)[0], "deny", "%s %s: %r" % (name, target, r.stdout))
            # the judge really judged: a write the reviewer may not make is still refused
            r, _ = self.reviewer({"file_path": str(pack / "hooks" / "x.sh"), "content": "x"}, cwd, pack)
            self.assertTrue(r.returncode == 2 or self.decision(r)[0] == "deny",
                            "%s: a forbidden reviewer write passed: %r %r" % (name, r.stdout, r.stderr[-200:]))
            self.assertFalse(marker.exists())

    def test_the_judge_starts_on_every_interpreter_the_hook_can_pick(self):
        """DY1 — the isolated start (`-I -S -B -X utf8 -`) starts and passes the built-in self-test on
        every CPython 3 the hook can pick here (this interpreter, the system `python3` = the macOS CLT
        3.9, a Homebrew one); the self-test runs from the planted working directory and nothing planted
        runs."""
        cwd, marker = self.plant()
        seen = set()
        for py in (sys.executable, "/usr/bin/python3", "/opt/homebrew/bin/python3", "/usr/local/bin/python3",
                   shutil.which("python3", path=self.env["PATH"])):
            if not py or not os.path.isfile(py) or os.path.realpath(py) in seen:
                continue
            ver = subprocess.run([py, "-c", "import sys; print('%d.%d' % sys.version_info[:2])"],
                                 capture_output=True, text=True, timeout=30)
            if ver.returncode != 0:
                continue            # a stub that cannot run (macOS CLT shim) is not a pick of the hook
            seen.add(os.path.realpath(py))
            env = dict(self.env, CYS_PY=py)
            r = subprocess.run([SH, str(HOOK), "--self-test"], env=env, cwd=str(cwd),
                               capture_output=True, text=True, timeout=180)
            self.assertEqual(r.returncode, 0, "%s (%s): %s" % (py, ver.stdout.strip(), r.stderr[-400:]))
            self.assertIn("self-test OK", r.stdout, py)
            self.assertFalse(marker.exists(), "%s: a planted module ran" % py)
        self.assertTrue(seen, "no interpreter to measure")

    def test_negative_control_old_start_executes_the_planted_module(self):
        cwd, marker = self.plant()
        r, _ = self.reviewer({"file_path": str(cwd / "notes.md"), "content": "x"}, cwd, self.pack,
                             hook=self.hook_with_start(START_DBEC50E0, "dbec50e0"))
        self.assertTrue(marker.exists(), "the old start did not execute the planted module — the "
                        "regression above would not see the fault: %r %r" % (r.stdout, r.stderr[-300:]))


    # ── RULINGS DY1 — what a seat can put in the judge's environment or in a folder it can write ──
    def _planted(self, d, marker, tag, mods=("json", "hashlib", "tempfile", "re", "sitecustomize", "usercustomize")):
        d.mkdir(parents=True, exist_ok=True)
        for mod in mods:
            (d / ("%s.py" % mod)).write_text("open(%r, 'a').write(%r)\nraise SystemExit(0)\n"
                                            % (str(marker), "%s:%s\n" % (tag, mod)), encoding="utf-8")
        return d

    def vectors(self, marker):
        """(name, extra env, working directory) — every folder below is inside the reviewer's allowed temp dir."""
        t = self.tmpdir
        ws = t / "review-ws"
        ws.mkdir(exist_ok=True)
        ver = "%d.%d" % sys.version_info[:2]
        home = t / "home-plant"
        # an interpreter that took this PYTHONHOME would import `encodings` from it first
        self._planted(home / "lib" / ("python" + ver) / "encodings", marker, "PYTHONHOME", mods=("__init__",))
        ub = t / "userbase"
        r = subprocess.run([sys.executable, "-I", "-S", "-c", "import site; print(site.getusersitepackages())"],
                           env=dict(self.env, PYTHONUSERBASE=str(ub)), capture_output=True, text=True, timeout=30)
        usersite = Path(r.stdout.strip())
        self.assertTrue(str(usersite).startswith(str(ub)), "user site does not follow PYTHONUSERBASE: %r" % r.stdout)
        self._planted(usersite, marker, "PYTHONUSERBASE")
        (usersite / "planted.pth").write_text("import os; open(%r, 'a').write('PYTHONUSERBASE:pth\\n')\n"
                                              % str(marker), encoding="utf-8")
        rel_cwd = t / "rel-cwd"
        self._planted(rel_cwd / "ws", marker, "rel")
        return [
            ("absolute PYTHONPATH in a seat-writable folder",
             {"PYTHONPATH": str(self._planted(t / "pp-abs", marker, "abs"))}, ws),
            ("relative PYTHONPATH", {"PYTHONPATH": "ws"}, rel_cwd),
            ("module in the current directory", {}, self._planted(t / "cwd-plant", marker, "cwd")),
            ("PYTHONSTARTUP", {"PYTHONSTARTUP": str(self._planted(t / "startup", marker, "startup",
                                                                   mods=("startup",)) / "startup.py")}, ws),
            ("PYTHONHOME", {"PYTHONHOME": str(home)}, ws),
            ("PYTHONUSERBASE", {"PYTHONUSERBASE": str(ub)}, ws),
            ("sitecustomize.py alone on PYTHONPATH",
             {"PYTHONPATH": str(self._planted(t / "pp-site", marker, "site", mods=("sitecustomize",)))}, ws),
            ("usercustomize.py alone on PYTHONPATH",
             {"PYTHONPATH": str(self._planted(t / "pp-user", marker, "user", mods=("usercustomize",)))}, ws),
            ("PYTHONINSPECT", {"PYTHONINSPECT": "1"}, ws),
            ("PYTHONIOENCODING", {"PYTHONIOENCODING": "utf-16"}, ws),
            ("PYTHONUTF8 not 0/1", {"PYTHONUTF8": "x"}, ws),
        ]

    def run_raw(self, hook, tool_input, cwd, extra):
        env = dict(self.env, CYS_ROLE="reviewer-codex", CYS_PACK_DIR=str(self.pack))
        env.update(extra)
        payload = {"session_id": "t5c", "tool_name": "Write", "tool_input": tool_input,
                   "hook_event_name": "PreToolUse", "cwd": str(cwd)}
        r = subprocess.run([SH, str(hook)], input=json.dumps(payload).encode(), env=env, cwd=str(cwd),
                           capture_output=True, timeout=60)
        return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")

    @staticmethod
    def whole_stdout_one_deny(out):
        try:
            obj = json.loads(out)
        except ValueError:
            return False
        return isinstance(obj, dict) and (obj.get("hookSpecificOutput") or {}).get("permissionDecision") == "deny"

    def faults(self, hook, vectors, marker):
        """Per vector: why the judge did NOT decide on its own (planted code ran / an allowed write did not
        pass / a forbidden write was not refused) — an empty list = the vector holds."""
        (self.pack / "round" / "_reviews").mkdir(parents=True, exist_ok=True)
        out = {}
        for name, extra, cwd in vectors:
            if marker.exists():
                marker.unlink()
            why = []
            for target in (self.pack / "round" / "_reviews" / "verdict-x.md",
                           self.pack / "round" / "REVIEWER_CODEX_TODO.md"):
                rc, so, se = self.run_raw(hook, {"file_path": str(target), "content": "x"}, cwd, extra)
                if rc != 0 or self.whole_stdout_one_deny(so):
                    why.append("allowed %s: rc=%d stdout=%r stderr=%r" % (target.name, rc, so[:120], se[-160:]))
            rc, so, se = self.run_raw(hook, {"file_path": str(self.pack / "hooks" / "x.sh"), "content": "x"},
                                      cwd, extra)
            if not (rc == 2 or (rc == 0 and self.whole_stdout_one_deny(so))):
                why.append("forbidden write not refused: rc=%d stdout=%r stderr=%r" % (rc, so[:120], se[-160:]))
            if marker.exists():
                why.append("planted code RAN: %r" % marker.read_text(encoding="utf-8"))
            out[name] = why
        return out

    def test_no_python_variable_or_planted_module_reaches_the_judge(self):
        """DY1 negative controls on the hook as it is: for each vector (an absolute / relative PYTHONPATH
        into a seat-writable folder, a module in the current directory, PYTHONSTARTUP / PYTHONHOME /
        PYTHONUSERBASE into a seat-writable folder, sitecustomize.py / usercustomize.py on those paths, and
        PYTHONINSPECT / PYTHONIOENCODING / an invalid PYTHONUTF8 — three variables that, without isolation,
        turn every decision into a non-blocking exit or unreadable output) the planted code does NOT run
        (its marker file is absent) AND the decision is complete: the verdict and own-TODO writes pass,
        the forbidden write gives exit 2 or, as the WHOLE stdout, one JSON
        deny object with exit 0 (an allowed write: exit 0 and no deny)."""
        marker = self.root / "PLANTED-RAN"
        got = self.faults(HOOK, self.vectors(marker), marker)
        bad = {k: v for k, v in got.items() if v}
        self.assertEqual(bad, {}, "the judge did not decide on its own:\n%s"
                         % "\n".join("%s: %s" % (k, " | ".join(v)) for k, v in bad.items()))

    def test_negative_control_the_earlier_starts_fail_these_vectors(self):
        """The same table against copies of the hook with the starts of the earlier heads — the faults the
        test above guards against are real. `017768bf` (`-B -s -` + the first-statement path clean-up):
        an absolute / relative PYTHONPATH, sitecustomize.py on it, PYTHONHOME, PYTHONINSPECT,
        PYTHONIOENCODING and an invalid PYTHONUTF8. `dbec50e0` (plain `-`): additionally the module in
        the current directory, PYTHONUSERBASE and usercustomize.py on PYTHONPATH. (PYTHONSTARTUP runs on
        no head: the judge's stdin is not a terminal — it stays as a pin.)"""
        marker = self.root / "PLANTED-RAN"
        vectors = self.vectors(marker)
        expected = {
            "017768bf": {"absolute PYTHONPATH in a seat-writable folder", "relative PYTHONPATH",
                         "sitecustomize.py alone on PYTHONPATH", "PYTHONHOME", "PYTHONINSPECT",
                         "PYTHONIOENCODING", "PYTHONUTF8 not 0/1"},
            "dbec50e0": {"module in the current directory", "PYTHONUSERBASE",
                         "usercustomize.py alone on PYTHONPATH", "absolute PYTHONPATH in a seat-writable folder"},
        }
        for tag, start in (("017768bf", START_017768BF), ("dbec50e0", START_DBEC50E0)):
            got = self.faults(self.hook_with_start(start, tag), vectors, marker)
            caught = sorted(k for k, v in got.items() if v)
            sys.stderr.write("DY1 negative control, start of %s: fails %s; holds %s\n"
                             % (tag, caught, sorted(k for k, v in got.items() if not v)))
            self.assertEqual(sorted(expected[tag] - set(caught)), [],
                             "%s: these vectors did not fail on the old start — the test above would not "
                             "see the fault: %r" % (tag, {k: got[k] for k in expected[tag]}))

    def fake_python(self, name, body):
        p = self.fakebin / name
        p.write_text(body, encoding="utf-8")
        p.chmod(0o755)
        return p

    def test_an_interpreter_that_refuses_the_flags_is_a_broken_gate_not_a_wall(self):
        """DY1 failure mode (unchanged): an interpreter that refuses the isolation flags (a Python 2 /
        pre-3.4 `-I` → usage error, exit 2) or cannot start (a dead `#!`) is a BROKEN gate = pass-through
        with one stderr line from the gate — for every role, also for a write the judge would refuse;
        never a blocking exit, never a deny; ONE interpreter launch per tool call (no probe); the
        `--self-test` fails loudly. Negative control: the same isolated start under `exec` (no exit-code
        mapping) blocks every tool call of every seat."""
        log = self.root / "py-launches.log"
        refuses = self.fake_python("python2-like", "#!/bin/sh\nprintf '%%s\\n' \"$*\" >> %s\n"
                                   "for a in \"$@\"; do [ \"$a\" = -I ] && { echo 'Unknown option: -I' >&2; "
                                   "echo 'usage: python [option] ... [-c cmd | -m mod | file | -] [arg] ...' >&2; "
                                   "exit 2; }; done\nexec %s \"$@\"\n" % (log, sys.executable))
        dead = self.fake_python("python-dead", "#!/nonexistent/interpreter/python3\n")
        (self.pack / "round" / "_reviews").mkdir(parents=True, exist_ok=True)
        forbidden = {"file_path": str(self.pack / "hooks" / "x.sh"), "content": "x"}
        for py in (refuses, dead):
            for role in ("reviewer-codex", "cso", "worker"):
                n0 = len(log.read_text().splitlines()) if log.exists() else 0
                r, _ = self.run_hook("Write", forbidden, CYS_ROLE=role, CYS_PY=str(py))
                self.assertEqual(r.returncode, 0, "%s %s: a broken gate must not block: rc=%d %r"
                                 % (py.name, role, r.returncode, r.stderr[-300:]))
                self.assertEqual(r.stdout.strip(), "", "%s %s: a broken gate must not deny: %r" % (py.name, role, r.stdout))
                self.assertEqual(r.stderr.count("role-capability-gate: 판정기 인터프리터가 격리 기동"), 1,
                                 "%s %s: not the one stderr line: %r" % (py.name, role, r.stderr[-400:]))
                if py == refuses:
                    lines = log.read_text().splitlines()[n0:]
                    self.assertEqual(len(lines), 1, "%s: one interpreter per tool call, got %r" % (role, lines))
                    self.assertTrue(lines[0].startswith("-I -S -B -X utf8 -"), lines)
            st = subprocess.run([SH, str(HOOK), "--self-test"], env=dict(self.env, CYS_PY=str(py)),
                                capture_output=True, text=True, timeout=60)
            self.assertEqual(st.returncode, 2, "%s: the self-test must fail loudly: %r" % (py.name, st.stderr[-300:]))
        # negative control: `exec` with the same flags and no mapping → exit 2 = every tool call blocked
        walled = self.hook_with_start("exec " + NEW_START, "exec-no-mapping")
        r, _ = self.run_hook("Write", {"file_path": str(self.pack / "round" / "_reviews" / "v.md"),
                                       "content": "x"}, hook=walled, CYS_ROLE="worker", CYS_PY=str(refuses))
        self.assertEqual(r.returncode, 2, "the negative control did not reproduce the wall: rc=%d %r"
                         % (r.returncode, r.stderr[-300:]))


if __name__ == "__main__":
    unittest.main()
