#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_wakeup_liveness.py — `_target_alive` 3값 판정 + drain zombie 가드 행동 회귀 (R2 라운드2).

왜 이 스위트인가: 종전 유일한 회귀 장치는 Rust 소스-substring 핀
(src/lib.rs `pack_wakeup_liveness_probe_seals_autostart` 의 `proc.returncode != 0` 포함 검사)
이었는데, 그것은 **봉인 키 문자열의 드리프트 감시**일 뿐 행동을 못 지킨다 — M8 변이 실측:
핀이 찾는 조건줄을 남겨둔 채 블록 본문을 `return "dead"` 로 바꿔도 cargo 핀은 초록이었다.
여기가 **행동 정본**이다(Rust 핀은 드리프트 감시로 강등 — lib.rs 주석 참조).

지키는 계약 (javis_wakeup.py 모듈 docstring · R1 라운드1 수리):
  ① 측정 실패 = 'unknown' — rc!=0 · 빈 stdout · 예외 · cys 바이너리 부재. 'dead' 로 접으면
     drain 의 zombie 가드가 pending 을 영구 삭제해 **주기 자가치유가 무음 전멸**한다(앵커 ③).
  ② 파서 드리프트 fail-safe = 'unknown' — 출력은 있는데 파싱이 0행이면 "우리가 못 읽었다"다.
  ③ 진짜 판정은 완화하지 않는다 — 다른 live 행만 있으면 'dead', 대상 live 행이 있으면 'alive',
     exited=true 행은 후보에서 배제(G27), JAVIS_WAKEUP_LIVENESS 강제 주입은 그대로 우선.
  ④ drain 배선: 'unknown' 은 **경고 배달**(실패 시 pending 유지)이지 삭제가 아니다.
     'dead' 는 종전대로 skip+삭제(zombie 가드 완화 금지 — 죽은 런에 병합하면 불멸화).

실행: python3 test_wakeup_liveness.py   (unittest·파일 직접 실행 — 저장소 관례 준거)
"""
import json
import os
import subprocess
import sys
import tempfile
import types
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_wakeup as W                                                 # noqa: E402


class _FakeCompleted:
    def __init__(self, rc, stdout):
        self.returncode = rc
        self.stdout = stdout
        self.stderr = ""


def _fake_run(rc, stdout):
    def run(cmd, **kw):
        return _FakeCompleted(rc, stdout)
    return run


def _raise_run(cmd, **kw):
    raise subprocess.SubprocessError("probe blew up")


LIVE_MASTER = "surface:1\trole=master\texited=false\n"
LIVE_OTHER = "surface:2\trole=reviewer-gemini\texited=false\n"
DEAD_MASTER = "surface:1\trole=master\texited=true\n"


class _Patched(unittest.TestCase):
    """공통 하네스 — 상태 디렉터리를 스크래치로 옮기고 외부 관측(cys 실행)을 전부 봉인한다."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wakeup-liveness-")
        self._saved_attrs = {}
        for name, val in [
            ("ROOT", self.tmp),
            ("WK_DIR", os.path.join(self.tmp, "_round", "wakeups")),
            ("PENDING_DIR", os.path.join(self.tmp, "_round", "wakeups", "pending")),
            ("LEDGER", os.path.join(self.tmp, "_round", "wakeups", "queue.jsonl")),
            ("FAILCOUNT", os.path.join(self.tmp, "_round", "wakeups", "failcount.json")),
        ]:
            self._saved_attrs[name] = getattr(W, name)
            setattr(W, name, val)
        self._saved_which = W.shutil.which
        W.shutil.which = lambda name: "/fake/bin/cys"
        self._saved_run = W.subprocess.run
        self._saved_alive = W._target_alive
        self._saved_env = {
            k: os.environ.get(k)
            for k in ("JAVIS_WAKEUP_LIVENESS", "JAVIS_FASTFAIL_MAX", "CYS_PACK_DIR")
        }
        os.environ.pop("JAVIS_WAKEUP_LIVENESS", None)
        os.environ.pop("JAVIS_FASTFAIL_MAX", None)
        os.environ["CYS_PACK_DIR"] = self.tmp   # AUTOPILOT_PAUSED 2경로가 스크래치를 보게

    def tearDown(self):
        for name, val in self._saved_attrs.items():
            setattr(W, name, val)
        W.shutil.which = self._saved_which
        W.subprocess.run = self._saved_run
        W._target_alive = self._saved_alive
        for k, v in self._saved_env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    # drain 하네스 보조 ---------------------------------------------------------
    def _enqueue(self, target="master", task="t1"):
        rc = W.main(["enqueue", "--to", target, "--task", task, "--reason", "회귀 하네스"])
        self.assertEqual(rc, W.EXIT_OK)

    def _pending_files(self):
        d = W.PENDING_DIR
        return sorted(os.listdir(d)) if os.path.isdir(d) else []

    def _ledger_events(self):
        try:
            with open(W.LEDGER, encoding="utf-8") as f:
                return [json.loads(ln) for ln in f if ln.strip()]
        except FileNotFoundError:
            return []


class TargetAliveVerdicts(_Patched):
    """① ② ③ — `_target_alive` 의 3값 판정 그 자체."""

    def test_probe_rc_nonzero_and_empty_stdout_is_unknown(self):
        # 봉인(NO_AUTOSTART) 아래 죽은 소켓의 실측 모양: stdout 0바이트 + rc!=0. 죽음의
        # 증거가 아니라 **미측정**이다 — 'dead' 로 접으면 drain 이 pending 을 영구 삭제한다.
        W.subprocess.run = _fake_run(1, "")
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_probe_rc_zero_but_empty_stdout_is_unknown(self):
        W.subprocess.run = _fake_run(0, "")
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_parseable_output_but_zero_rows_is_unknown(self):
        # 파서 드리프트 fail-safe: 출력은 있는데 surface 행을 0개 뽑았다 = "우리가 못 읽었다".
        W.subprocess.run = _fake_run(0, "totally different format v99\nno surfaces here\n")
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_only_exited_rows_fold_to_unknown_not_dead(self):
        # exited 행만 있으면 live 행 0개 — 전문에 role 토큰이 있어도(G27) dead 단정은 못 한다.
        W.subprocess.run = _fake_run(0, DEAD_MASTER)
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_probe_exception_is_unknown(self):
        W.subprocess.run = _raise_run
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_missing_cys_binary_is_unknown(self):
        W.shutil.which = lambda name: None
        self.assertEqual(W._target_alive("master"), "unknown")

    def test_live_row_for_target_is_alive(self):
        W.subprocess.run = _fake_run(0, LIVE_MASTER + LIVE_OTHER)
        self.assertEqual(W._target_alive("master"), "alive")

    def test_other_live_rows_without_target_is_dead(self):
        # 가드 완화 금지의 짝: 관측이 성립했고(다른 live 행 실재) 대상만 없다 = 진짜 dead.
        W.subprocess.run = _fake_run(0, LIVE_OTHER)
        self.assertEqual(W._target_alive("master"), "dead")

    def test_exited_target_row_does_not_resurrect_it(self):
        # G27: 죽은 행의 role 토큰(litter)에 속지 않는다 — live 행이 따로 있으니 dead 확정.
        W.subprocess.run = _fake_run(0, DEAD_MASTER + LIVE_OTHER)
        self.assertEqual(W._target_alive("master"), "dead")

    def test_env_override_still_wins(self):
        os.environ["JAVIS_WAKEUP_LIVENESS"] = "dead"
        W.subprocess.run = _raise_run   # 호출되더라도 무관 — override 가 선행
        self.assertEqual(W._target_alive("master"), "dead")


class DrainZombieGuardWiring(_Patched):
    """④ — drain 배선: unknown 은 삭제가 아니라 배달 시도다(실패 시 pending 유지)."""

    def test_unknown_with_failing_send_keeps_pending(self):
        # 원 사고의 형태: 측정 실패가 dead 로 접혀 pending 영구 삭제 → 자가치유 무음 전멸.
        # 수리 후: unknown → 배달 시도 → 실패 → pending 유지(deliver_failed 원장·삭제 0건).
        self._enqueue()
        self.assertEqual(len(self._pending_files()), 1)
        W._target_alive = lambda target: "unknown"
        W.subprocess.run = _raise_run   # cys send 실패(대상이 정말 안 닿는 순간)
        rc = W.cmd_drain(types.SimpleNamespace(target=None, deliver=True))
        self.assertEqual(rc, W.EXIT_OK)
        self.assertEqual(len(self._pending_files()), 1,
                         "unknown 대상의 pending 이 삭제됐다 — zombie 가드가 미측정을 죽음으로 접었다")
        events = [e["event"] for e in self._ledger_events()]
        self.assertIn("deliver_failed", events)
        self.assertNotIn("skipped", events, "unknown 인데 skip(영구 종결) 경로를 탔다")

    def test_unknown_with_working_send_delivers(self):
        self._enqueue()
        sent = []
        W._target_alive = lambda target: "unknown"
        W.subprocess.run = lambda cmd, **kw: sent.append(list(cmd))
        rc = W.cmd_drain(types.SimpleNamespace(target=None, deliver=True))
        self.assertEqual(rc, W.EXIT_OK)
        self.assertEqual(len(sent), 1, "unknown 은 경고 **배달**인데 send 가 호출되지 않았다")
        self.assertEqual(self._pending_files(), [], "배달 성공 후 pending 이 종결되지 않았다")
        events = [e["event"] for e in self._ledger_events()]
        self.assertIn("delivered", events)
        self.assertNotIn("skipped", events)

    def test_dead_still_skips_and_removes(self):
        # zombie 가드 본체는 완화 금지 — 진짜 dead 는 종전대로 skip+삭제(죽은 런 병합=불멸화 차단).
        self._enqueue()
        sent = []
        W._target_alive = lambda target: "dead"
        W.subprocess.run = lambda cmd, **kw: sent.append(list(cmd))
        rc = W.cmd_drain(types.SimpleNamespace(target=None, deliver=True))
        self.assertEqual(rc, W.EXIT_OK)
        self.assertEqual(sent, [], "dead 대상에 배달을 시도했다")
        self.assertEqual(self._pending_files(), [], "dead pending 이 종결되지 않았다")
        events = [e for e in self._ledger_events() if e["event"] == "skipped"]
        self.assertEqual(len(events), 1)
        self.assertIn("target_dead", events[0]["why"])


class DrainRefusedWhilePaused(unittest.TestCase):
    """H-05-N4-01 (설계 §6-5 "깨우기 큐가 일시정지 중 거부하는 동작" · RULINGS HN3) — the real CLI end to end: with the
    kill-switch mark present at EITHER of its two paths (`<pack>/AUTOPILOT_PAUSED`, `<root>/_round/AUTOPILOT_PAUSED`) and
    repair notices queued for the master, `javis_wakeup.py drain --deliver` exits 4 with 0 deliveries — nothing reaches
    `cys send`, the notices stay pending, the ledger records the refusal. Control: the same queue without a mark IS
    delivered (one digest send), so the refusal is the pause guard's and not a broken rig.
    Sealed: private HOME / JAVIS_ROOT / CYS_PACK_DIR, every CYS_* / JAVIS_* / AITERM_* / CLAUDE* variable of the caller
    removed, a mock `cys` first on PATH that only records its arguments (no daemon, no live queue)."""

    # one log line per invocation (the digest text itself spans several lines, so only the verb and its target are logged)
    MOCK_CYS = ("#!/bin/sh\nprintf 'CALL %s %s %s %s\\n' \"$1\" \"$2\" \"$3\" \"$4\" >> \"$MOCK_CALLS\"\n"
                "echo 'QUEUED (depth 1) · durable=true'\nexit 0\n")
    NOTICES = (("repair-R-0007", "수리 R-0007 검증 대기 — 리뷰어 서명 요청"),
               ("repair-R-0008", "수리 R-0008 병합 충돌 — 워커 배정 필요"))

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wakeup-paused-")
        self.root = os.path.join(self.tmp, "ws")
        self.pack = os.path.join(self.tmp, "pack")
        mockbin = os.path.join(self.tmp, "bin")
        for d in (os.path.join(self.tmp, "home"), os.path.join(self.root, "_round"), self.pack, mockbin):
            os.makedirs(d)
        mock = os.path.join(mockbin, "cys")
        with open(mock, "w", encoding="utf-8") as f:
            f.write(self.MOCK_CYS)
        os.chmod(mock, 0o755)
        self.calls = os.path.join(self.tmp, "cys-calls.log")
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_", "CLAUDE"))}
        env.update(HOME=os.path.join(self.tmp, "home"), JAVIS_ROOT=self.root, CYS_PACK_DIR=self.pack,
                   JAVIS_WAKEUP_LIVENESS="alive", MOCK_CALLS=self.calls,
                   PATH=mockbin + os.pathsep + env.get("PATH", ""))
        self.env = env

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _run(self, *args):
        return subprocess.run([sys.executable, os.path.join(BIN, "javis_wakeup.py"), *args], env=self.env,
                              cwd=self.root, capture_output=True, text=True, timeout=60)

    def _pending(self):
        d = os.path.join(self.root, "_round", "wakeups", "pending")
        return sorted(n for n in os.listdir(d) if n.endswith(".json")) if os.path.isdir(d) else []

    def _cys_calls(self):
        try:
            with open(self.calls, encoding="utf-8") as f:
                return [ln for ln in f.read().splitlines() if ln.strip()]
        except FileNotFoundError:
            return []

    def _events(self):
        try:
            with open(os.path.join(self.root, "_round", "wakeups", "queue.jsonl"), encoding="utf-8") as f:
                return [json.loads(ln)["event"] for ln in f if ln.strip()]
        except FileNotFoundError:
            return []

    def _queue_repair_notices(self):
        for task, reason in self.NOTICES:
            r = self._run("enqueue", "--to", "master", "--task", task, "--reason", reason, "--idempotency-key", task)
            self.assertEqual(r.returncode, 0, "enqueue: %s %s" % (r.stdout, r.stderr))
        self.assertEqual(len(self._pending()), 2, "premise: two repair notices pending for the master")

    def _assert_refused(self, mark):
        self._queue_repair_notices()
        before = self._pending()
        with open(mark, "w", encoding="utf-8") as f:
            f.write("paused by the owner\n")
        r = self._run("drain", "--deliver")
        self.assertEqual(r.returncode, 4, "drain --deliver under %s must exit 4 (paused): rc=%s\n%s\n%s"
                         % (mark, r.returncode, r.stdout, r.stderr))
        out = json.loads([ln for ln in r.stdout.splitlines() if ln.startswith("{")][-1])
        self.assertEqual((out["delivered"], out.get("refused")), (0, "paused"), out)
        self.assertEqual(self._cys_calls(), [], "a paused drain reached `cys send`")
        self.assertEqual(self._pending(), before, "the queued notices must stay pending while paused")
        self.assertIn("deliver_refused_paused", self._events())

    def test_pack_mark_refuses_delivery(self):
        self._assert_refused(os.path.join(self.pack, "AUTOPILOT_PAUSED"))

    def test_workspace_mark_refuses_delivery(self):
        self._assert_refused(os.path.join(self.root, "_round", "AUTOPILOT_PAUSED"))

    @unittest.skipIf(os.name == "nt" or (hasattr(os, "geteuid") and os.geteuid() == 0),
                     "a folder without search permission cannot be made here (Windows / root) — "
                     "PausedEvidenceUnknownState measures the same rule in-process")
    def test_unknown_pause_state_is_refused(self):
        """RULINGS JE1 (H-05-N4-01 "확인 불능도 거부"): the owner's mark is written, then its folder loses search
        permission, so the pause state cannot be read (stat -> EACCES). That is a pause: exit 4, nothing delivered,
        the notices stay pending, and the evidence names the path and the error."""
        self._queue_repair_notices()
        before = self._pending()
        mark = os.path.join(self.pack, "AUTOPILOT_PAUSED")
        with open(mark, "w", encoding="utf-8") as f:
            f.write("paused by the owner\n")
        os.chmod(self.pack, 0)
        try:
            r = self._run("drain", "--deliver")
        finally:
            os.chmod(self.pack, 0o700)
        self.assertEqual(r.returncode, 4, "unknown pause state must be refused: rc=%s\n%s\n%s"
                         % (r.returncode, r.stdout, r.stderr))
        out = json.loads([ln for ln in r.stdout.splitlines() if ln.startswith("{")][-1])
        self.assertEqual((out["delivered"], out.get("refused")), (0, "paused"), out)
        self.assertTrue(any(mark in ev and "확인 불능" in ev for ev in out["evidence"]),
                        "the evidence must name the unreadable path and say 확인 불능: %r" % out["evidence"])
        self.assertEqual(self._cys_calls(), [], "an unreadable pause state let the drain reach `cys send`")
        self.assertEqual(self._pending(), before, "the notices must stay pending")

    def test_control_without_a_mark_the_same_queue_is_delivered(self):
        self._queue_repair_notices()
        r = self._run("drain", "--deliver")
        self.assertEqual(r.returncode, 0, "%s\n%s" % (r.stdout, r.stderr))
        out = json.loads([ln for ln in r.stdout.splitlines() if ln.startswith("{")][-1])
        self.assertEqual(out["delivered"], 2, out)
        calls = self._cys_calls()
        self.assertEqual(len(calls), 1, "one digest send for the master: %r" % calls)
        self.assertEqual(calls[0], "CALL send --queued --to master", calls)
        self.assertEqual(self._pending(), [])



class PausedEvidenceUnknownState(_Patched):
    """RULINGS JE1 — `paused_evidence()` asks the file system for each mark: "no such file" / "not a directory" means no
    mark; ANY other error means the state is unknown, and unknown is a pause (fail-closed, "확인 불능도 거부"). In-process
    and portable: the file-system calls are made to fail with EACCES for the mark only (stat and lstat both, whichever
    the implementation asks)."""

    @staticmethod
    def _deny_mark(real):
        def call(path, *a, **kw):
            if os.path.basename(os.fspath(path)) == W.PAUSED_BASENAME:
                raise PermissionError(13, "Permission denied", os.fspath(path))
            return real(path, *a, **kw)
        return call

    def test_unreadable_mark_counts_as_paused(self):
        from unittest import mock
        with mock.patch.object(W.os, "stat", self._deny_mark(os.stat)), \
                mock.patch.object(W.os, "lstat", self._deny_mark(os.lstat)):
            paused, evidence = W.paused_evidence()
        self.assertTrue(paused, "an unreadable pause state was read as 'not paused'")
        self.assertEqual(len(evidence), 2, evidence)   # both paths are unknown
        for ev, path in zip(evidence, W.paused_paths()):
            self.assertTrue(ev.startswith(path) and "확인 불능" in ev, ev)

    def test_no_mark_and_not_a_directory_are_not_a_pause(self):
        self.assertEqual(W.paused_evidence(), (False, []), "no mark at either path")
        blocker = os.path.join(self.tmp, "a-file")
        with open(blocker, "w", encoding="utf-8") as f:
            f.write("x")
        os.environ["CYS_PACK_DIR"] = blocker          # <file>/AUTOPILOT_PAUSED -> ENOTDIR = no such mark
        self.assertEqual(W.paused_evidence(), (False, []), "'not a directory' is 'no mark', not 'unknown'")

    def test_a_present_mark_is_a_pause(self):
        mark = W.paused_paths()[0]
        with open(mark, "w", encoding="utf-8") as f:
            f.write("paused\n")
        self.assertEqual(W.paused_evidence(), (True, [mark]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
