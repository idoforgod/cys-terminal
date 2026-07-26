#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_pack_guard_scope.py — pack-guard.sh 게이트 A(교차-scope, CU-2A) 회귀 + 게이트 B 무회귀.

설계 정본: _round/dept-scope-integrity/DESIGN_scope-first-class.md §4 CU-2A · ADR-2 · ADR-3 · INV-1,
근거 실측: SIM_REPORT.md SIM-3(악조건 8종).

핀하는 것:
  ① INV-1 fail-open — cys 부재·python3 부재·깨진 stdin·pack-scope-check 파손/비정상종료에서
     **절대 차단하지 않는다**(stdout에 permissionDecision 없음·exit 0).
  ② 교차 쓰기 log 모드 = additionalContext 경고 + state/scope-guard.log jsonl 적재.
  ③ 교차 쓰기 deny 모드 = PreToolUse JSON permissionDecision:"deny"(+ exit 0 — ADR-3).
     PostToolUse 는 사후라 deny 불가 → 경고로 강등.
  ④ 게이트 B(자기 팩 vendor 경고) 동작 불변 — ADR-2 구분(오너 과거 Rejected 무접촉).
  ⑤ 심링크 경유 교차 쓰기 탐지 — pack-scope-check 에 **실경로**가 전달된다.
  ⑥ ~/.cys 밖(레포 워크트리 등) = 무관 → 판정기 호출조차 없음(개발 무간섭).
  ⑦ 권위 보수화 — pack-scope-check 가 authority!=daemon(로컬 폴백) 을 신고하면 deny→log 강등.
  ⑧ preflight 등록 기대집합(SELFCORR_HOOKS)에 PreToolUse 가 있다(등록기=검사기 단일표).

관측 기법: 격리 HOME + 격리 CYS_PACK_DIR + PATH 앞 **스텁 cys**(실제 cys 호출 0 — 서브커맨드
`cys pack-scope-check` 는 타 워커 구현 중이라 계약 JSON 만 모사) + 스텁 argv 기록 파일.
실행: python3 test_pack_guard_scope.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
PACK_SRC = os.path.dirname(BIN)                                            # cysjavis-pack
HOOK = os.path.join(PACK_SRC, "hooks", "pack-guard.sh")
sys.path.insert(0, BIN)

STUB_CYS = r"""#!/bin/bash
# 스텁 cys — 계약 JSON만 모사. 호출 argv를 $STUB_LOG 에 1줄 기록(호출 여부·인자 채증).
[ -n "$STUB_LOG" ] && echo "$*" >> "$STUB_LOG"
case "$1" in
  pack-scope-check)
    [ -n "$STUB_SCOPE_RC" ] && exit "$STUB_SCOPE_RC"
    printf '%s' "$STUB_SCOPE_JSON"
    exit 0 ;;
  pack-ownership)
    printf '%s\n' "$STUB_OWNERSHIP"
    exit 0 ;;
esac
exit 0
"""


def _mk_toolbox(with_python=True, with_cys=True):
    """PATH 대역 디렉터리 — 훅이 쓰는 외부 도구만 심어 '부재'를 정밀 모사한다."""
    d = tempfile.mkdtemp(prefix="pg-tools-")
    needed = ["cat", "sed", "mkdir", "tr", "timeout", "realpath"]
    if with_python:
        needed.append("python3")
    for name in needed:
        src = shutil.which(name)
        if src:
            os.symlink(src, os.path.join(d, name))
    if with_cys:
        p = os.path.join(d, "cys")
        with open(p, "w") as f:
            f.write(STUB_CYS)
        os.chmod(p, 0o755)
    return d


class HookRun(object):
    def __init__(self, proc, home, pack, stub_log):
        self.rc = proc.returncode
        self.stdout = proc.stdout
        self.stderr = proc.stderr
        self.home, self.pack, self.stub_log = home, pack, stub_log
        try:
            self.json = json.loads(proc.stdout) if proc.stdout.strip() else None
        except ValueError:
            self.json = None

    @property
    def hso(self):
        return (self.json or {}).get("hookSpecificOutput", {})

    @property
    def denied(self):
        return self.hso.get("permissionDecision") == "deny"

    @property
    def context(self):
        return self.hso.get("additionalContext", "")

    @property
    def stub_calls(self):
        if not os.path.isfile(self.stub_log):
            return []
        with open(self.stub_log, encoding="utf-8") as f:
            return [ln.rstrip("\n") for ln in f if ln.strip()]

    def ledger(self):
        p = os.path.join(self.pack, "state", "scope-guard.log")
        if not os.path.isfile(p):
            return []
        with open(p, encoding="utf-8") as f:
            return [json.loads(ln) for ln in f if ln.strip()]


class Env(object):
    """격리 HOME + 팩 트리 1벌. 여러 번 훅을 실행해 코얼레싱 같은 상태 의존도 검증한다."""

    def __init__(self, own_pack="pack"):
        self.home = tempfile.mkdtemp(prefix="pg-home-")
        self.pack = os.path.join(self.home, ".cys", own_pack)
        os.makedirs(os.path.join(self.pack, "state"), exist_ok=True)
        self.stub_log = os.path.join(self.home, "stub-calls.txt")

    def path(self, *parts):
        return os.path.join(self.home, ".cys", *parts)

    def touch(self, abspath, body=""):
        os.makedirs(os.path.dirname(abspath), exist_ok=True)
        with open(abspath, "w", encoding="utf-8") as f:
            f.write(body)
        return abspath

    def set_mode(self, mode):
        with open(os.path.join(self.pack, "state", "scope-guard.mode"), "w") as f:
            f.write(mode)

    def run(self, file_path, event="PreToolUse", scope_json=None, scope_raw=None,
            scope_rc=None, ownership="", with_python=True, with_cys=True, raw_stdin=None,
            session="sess-1"):
        tools = _mk_toolbox(with_python=with_python, with_cys=with_cys)
        env = {
            "HOME": self.home,
            "CYS_PACK_DIR": self.pack,
            "PATH": tools,                       # 격리 — 시스템 PATH 미상속(도구 부재 정밀 모사)
            "TMPDIR": os.path.join(self.home, "tmp"),
            "STUB_LOG": self.stub_log,
            "STUB_SCOPE_JSON": (scope_raw if scope_raw is not None
                                else json.dumps(scope_json or {"verdict": "ok"})),
            "STUB_OWNERSHIP": ownership,
        }
        os.makedirs(env["TMPDIR"], exist_ok=True)
        if scope_rc is not None:
            env["STUB_SCOPE_RC"] = str(scope_rc)
        payload = raw_stdin if raw_stdin is not None else json.dumps({
            "hook_event_name": event, "session_id": session,
            "tool_name": "Write", "tool_input": {"file_path": file_path},
        })
        proc = subprocess.run(["/bin/bash", HOOK], input=payload, capture_output=True,
                              text=True, timeout=30, env=env)
        return HookRun(proc, self.home, self.pack, self.stub_log)


CROSS = {"verdict": "cross-scope", "own_scope": "pack-dept-dept-3",
         "target_scope": "pack", "suggest": "cys todo-path"}


class InvariantFailOpen(unittest.TestCase):
    """INV-1 — 위반 확정 외 어떤 내부 상태에서도 도구 실행을 막지 않는다."""

    def _assert_allowed(self, r, why):
        self.assertEqual(r.rc, 0, "%s: exit %s (전 경로 exit 0 규약)" % (why, r.rc))
        self.assertFalse(r.denied, "%s: 차단됨 — INV-1 위반" % why)

    def test_cys_absent_even_in_deny_mode(self):
        e = Env()
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS, with_cys=False)
        self._assert_allowed(r, "cys 부재(fresh 기계)")
        self.assertEqual(r.stdout.strip(), "", "판정기 없이 경고까지 낸다면 근거 없는 소음")

    def test_python3_absent(self):
        e = Env()
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS, with_python=False)
        self._assert_allowed(r, "python3 부재")

    def test_broken_stdin(self):
        e = Env()
        e.set_mode("deny")
        for payload in ("", "not-json-at-all", "[1,2,3]", '{"tool_input": null}'):
            r = e.run("", raw_stdin=payload)
            self._assert_allowed(r, "깨진 stdin(%r)" % payload)

    def test_scope_check_crash_or_garbage(self):
        e = Env()
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_rc=3, scope_json=CROSS)
        self._assert_allowed(r, "pack-scope-check 비정상 종료")
        r = e.run(fp, scope_json=None)                       # verdict=ok
        self._assert_allowed(r, "verdict=ok")
        for raw in ("", "This is not JSON {{{", "[1,2,3]", "null", '{"verdict": null}'):
            r = e.run(fp, scope_raw=raw)
            self._assert_allowed(r, "판정 출력 파손(%r)" % raw)
            self.assertEqual(r.stdout.strip(), "", "파싱 불가인데 경고까지 냈다: %r" % raw)

    def test_unknown_mode_file_is_log(self):
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("DENY-ish쓰레기")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS)
        self._assert_allowed(r, "모드 파일 파손")
        self.assertIn("교차-scope", r.context)

    def test_authority_local_downgrades_deny(self):
        """SIM-3 교훈 — 판정 권위가 불확실(데몬 무응답 폴백)하면 차단하지 않는다."""
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        payload = dict(CROSS)
        payload["authority"] = "local"
        r = e.run(fp, scope_json=payload)
        self._assert_allowed(r, "authority=local")
        self.assertIn("교차-scope", r.context)
        self.assertEqual(r.ledger()[-1]["mode"], "log")


class CrossScopeDetection(unittest.TestCase):

    def test_log_mode_warns_and_appends_ledger(self):
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("log")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS)
        self.assertEqual(r.rc, 0)
        self.assertFalse(r.denied, "log 모드에서 차단하면 안 된다")
        self.assertIn("교차-scope 팩 쓰기 감지", r.context)
        self.assertEqual(r.hso.get("hookEventName"), "PreToolUse")
        led = r.ledger()
        self.assertEqual(len(led), 1, led)
        self.assertEqual(led[0]["own_scope"], "pack-dept-dept-3")
        self.assertEqual(led[0]["target_scope"], "pack")
        self.assertEqual(led[0]["path"], os.path.realpath(fp))
        self.assertEqual(led[0]["mode"], "log")
        self.assertEqual(led[0]["session"], "sess-1")
        self.assertEqual(led[0]["event"], "PreToolUse")
        self.assertTrue(led[0]["ts"])

    def test_mode_file_absent_defaults_to_log(self):
        e = Env(own_pack="pack-dept-dept-3")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS)
        self.assertFalse(r.denied)
        self.assertIn("교차-scope", r.context)

    def test_deny_mode_pre_tool_use(self):
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS, event="PreToolUse")
        self.assertEqual(r.rc, 0, "ADR-3 — 차단해도 exit 는 0")
        self.assertTrue(r.denied, r.stdout)
        self.assertEqual(r.hso.get("hookEventName"), "PreToolUse")
        reason = r.hso.get("permissionDecisionReason", "")
        self.assertIn("pack-dept-dept-3", reason)
        self.assertIn("cys todo-path", reason, "교정 안내 부재")
        self.assertEqual(r.ledger()[-1]["mode"], "deny")

    def test_deny_mode_post_tool_use_degrades_to_warning(self):
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("deny")
        fp = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        r = e.run(fp, scope_json=CROSS, event="PostToolUse")
        self.assertFalse(r.denied, "사후 이벤트에서 deny 결정 채널은 무의미")
        self.assertIn("교차-scope", r.context)
        self.assertEqual(r.hso.get("hookEventName"), "PostToolUse")

    def test_symlink_route_is_resolved(self):
        """SIM-3: 심링크 경유 base 쓰기 — 실경로로 판정기가 호출돼야 탐지된다."""
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("deny")
        real = e.touch(e.path("pack", "round", "MASTER_TODO.md"))
        linkdir = tempfile.mkdtemp(prefix="pg-link-")
        link = os.path.join(linkdir, "shortcut")
        os.symlink(os.path.dirname(real), link)             # ~/.cys 밖 → 문자열 접두 매칭 회피 경로
        r = e.run(os.path.join(link, "MASTER_TODO.md"), scope_json=CROSS)
        self.assertTrue(r.denied, "심링크 우회로 게이트가 뚫렸다: %s" % r.stdout)
        calls = [c for c in r.stub_calls if c.startswith("pack-scope-check")]
        self.assertEqual(len(calls), 1, r.stub_calls)
        self.assertIn(os.path.realpath(real), calls[0])
        self.assertNotIn("shortcut", calls[0], "심링크 경로가 그대로 전달됨(realpath 미적용)")

    def test_pack_prev_is_in_family(self):
        e = Env(own_pack="pack-dept-dept-3")
        e.set_mode("log")
        fp = e.touch(e.path("pack.prev", "round", "X.md"))
        r = e.run(fp, scope_json=CROSS)
        self.assertIn("교차-scope", r.context)


class OutOfScopePaths(unittest.TestCase):

    def test_repo_worktree_untouched(self):
        """개발 무간섭 계약 — ~/.cys 밖 경로는 판정기 호출조차 없다."""
        e = Env()
        e.set_mode("deny")
        proj = tempfile.mkdtemp(prefix="pg-repo-")
        fp = os.path.join(proj, "src", "bin", "cys.rs")
        os.makedirs(os.path.dirname(fp), exist_ok=True)
        with open(fp, "w", encoding="utf-8") as f:
            f.write("fn main() {}\n")
        r = e.run(fp, scope_json=CROSS, event="PostToolUse", ownership="system")
        self.assertEqual(r.rc, 0)
        self.assertFalse(r.denied)
        self.assertEqual(r.stdout.strip(), "")
        self.assertEqual([c for c in r.stub_calls if c.startswith("pack-scope-check")], [])

    def test_non_pack_dir_under_cys(self):
        e = Env()
        e.set_mode("deny")
        fp = e.touch(e.path("state", "breaker.json"), "{}")
        r = e.run(fp, scope_json=CROSS)
        self.assertFalse(r.denied)
        self.assertEqual(r.stdout.strip(), "")


class VendorGateRegression(unittest.TestCase):
    """ADR-2 — 게이트 B(자기 팩 vendor 경고)는 종전 동작 그대로다."""

    def test_own_pack_system_file_warns_once(self):
        e = Env()                                            # own pack = ~/.cys/pack
        fp = e.touch(os.path.join(e.pack, "hooks", "pack-guard.sh"), "x")
        r = e.run(fp, event="PostToolUse", ownership="system")   # 게이트 A: verdict=ok → 통과
        self.assertFalse(r.denied)
        self.assertIn("vendor(system) 파일", r.context)
        self.assertEqual(r.hso.get("hookEventName"), "PostToolUse")
        r2 = e.run(fp, event="PostToolUse", ownership="system")  # 세션·파일당 1회 코얼레싱
        self.assertEqual(r2.stdout.strip(), "", "코얼레싱 회귀 — 2회째도 경고")

    def test_own_pack_custom_file_silent(self):
        e = Env()
        fp = e.touch(os.path.join(e.pack, "bin", "mine.py"), "x")
        r = e.run(fp, event="PostToolUse", ownership="custom")
        self.assertEqual(r.stdout.strip(), "")

    def test_pre_tool_use_does_not_fire_vendor_gate(self):
        """게이트 B는 사후 경고 — PreToolUse 등록분에서 발화하면 이벤트 계약 오염."""
        e = Env()
        fp = e.touch(os.path.join(e.pack, "hooks", "pack-guard.sh"), "x")
        r = e.run(fp, event="PreToolUse", ownership="system")
        self.assertEqual(r.stdout.strip(), "")

    def test_unknown_event_name_keeps_legacy_behavior(self):
        e = Env()
        fp = e.touch(os.path.join(e.pack, "hooks", "pack-guard.sh"), "x")
        r = e.run(fp, event="", ownership="system")
        self.assertIn("vendor(system) 파일", r.context)


class PreflightRegistration(unittest.TestCase):
    """CU-2A.6 — 등록기와 검사기가 같은 표(SELFCORR_HOOKS)를 보므로 '등록됐는데 WARN' 상태 불가."""

    def test_selfcorr_table_has_both_events(self):
        import javis_preflight as P
        entry = dict(P.SELFCORR_HOOKS).get("pack-guard.sh")
        self.assertIsNotNone(entry, "SELFCORR_HOOKS에 pack-guard.sh 없음")
        self.assertIn(("PostToolUse", "Write|Edit|MultiEdit"), entry)
        self.assertIn(("PreToolUse", "Write|Edit|MultiEdit"), entry)

    def test_register_and_detect_roundtrip(self):
        """실 settings.json 무접촉 — 임시 파일에 등록기/검사기 왕복(라이브 계정 미탐색)."""
        import javis_preflight as P
        with tempfile.TemporaryDirectory() as t:
            sp = os.path.join(t, "settings.json")
            with open(sp, "w", encoding="utf-8") as f:
                json.dump({"hooks": {}}, f)
            pf = P.Preflight(fix=True, skips=set(), mode="fix")
            for event, matcher in dict(P.SELFCORR_HOOKS)["pack-guard.sh"]:
                self.assertFalse(P.Preflight._event_hook_registered(sp, event, "pack-guard.sh"))
                self.assertIsNone(pf._register_event_hook(sp, event, "pack-guard.sh", matcher))
                self.assertTrue(P.Preflight._event_hook_registered(sp, event, "pack-guard.sh"))
            with open(sp, encoding="utf-8") as f:
                data = json.load(f)
            self.assertEqual(len(data["hooks"]["PreToolUse"]), 1)
            self.assertEqual(data["hooks"]["PreToolUse"][0]["matcher"], "Write|Edit|MultiEdit")


if __name__ == "__main__":
    unittest.main(verbosity=2)
