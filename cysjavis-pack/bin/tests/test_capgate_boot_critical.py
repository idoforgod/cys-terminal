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

    def run_hook(self, tool, tool_input, cwd=None, **envkw):
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
        r = subprocess.run([SH, str(HOOK)], input=json.dumps(payload), env=env,
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
        for role in ("worker", "master", ""):
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
                self.assertIn("cys repair submit", reason)
                self.assertIn("기동 필수", reason)
                seen = self.calls_seen()
                self.assertEqual(seen, ["1|repair boot-critical --path %s --json" % target],
                                 "cys 한 번 · CYS_NO_AUTOSTART=1 이어야 한다: %r" % seen)

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
        old = "    bc_reason = None if block else boot_critical_deny_reason(tool, tool_input, data.get(\"cwd\"))"
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

    def test_control_variants_allowed(self):
        rel = "hooks/README.md"
        for spelled, cwd in self.variants(rel):
            r, _ = self.run_hook("Bash", {"command": "echo x > %s" % spelled}, cwd=cwd)
            self.assertNotEqual(self.decision(r)[0], "deny", "%s (cwd=%s)" % (spelled, cwd))
        staging = self.home / ".cys" / "local" / "repairs" / "R-1" / "staging" / "hooks"
        staging.mkdir(parents=True)
        r, _ = self.run_hook("Write", {"file_path": str(staging / "inject-context.sh"), "content": "x"})
        self.assertNotEqual(self.decision(r)[0], "deny", "작업 사본 쓰기가 막혔다")


if __name__ == "__main__":
    unittest.main()
