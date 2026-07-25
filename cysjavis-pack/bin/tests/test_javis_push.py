#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_javis_push.py — javis_push.py(C7 이중 채널 보고) 회귀.

검증 축:
  1) 포인터 조립 — 개행 접기(1줄 계약) + `· 전문: <절대경로>` 자동 첨부
  2) body-file 검증 — 부재·빈 파일은 exit 2 이고 **wakeup 위임을 하지 않는다**
     (없는 파일을 가리키는 포인터를 큐에 넣지 않는다)
  3) wakeup 위임 — enqueue 인자(task = --key, reason = 조립된 포인터).
     ★B1: **멱등키를 넘기지 않는다** — 넘기면 같은 키의 최신 보고가 suppressed 로 폐기된다.
  4) 결과 매핑 — queued→enqueued(0) · coalesced(0) · suppressed(5·호환) · 위임 실패(6)

실행: python3 test_javis_push.py   (unittest·파일 직접 실행 관례)
"""
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_push as P                                                     # noqa: E402


class FakeWakeup:
    """run_wakeup 대역 — 호출 인자를 기록하고 정해진 결과를 돌려준다."""

    def __init__(self, result="queued", rc=0, out=None, err=""):
        self.result, self.rc, self.err = result, rc, err
        self.out = out
        self.calls = []

    def __call__(self, argv, timeout=20):
        self.calls.append(list(argv))
        if self.out is not None:
            return self.rc, self.out, self.err
        return self.rc, json.dumps({"result": self.result, "id": "W-x"}) + "\n", self.err


class PushTest(unittest.TestCase):
    def setUp(self):
        self._orig = P.run_wakeup
        self.tmp = tempfile.TemporaryDirectory()
        self.body = os.path.join(self.tmp.name, "report.md")
        with open(self.body, "w", encoding="utf-8") as f:
            f.write("# 전문\n본문 여러 줄\n")

    def tearDown(self):
        P.run_wakeup = self._orig
        self.tmp.cleanup()

    # ── 1) 포인터 조립 ──
    def test_pointer_folds_newlines_and_appends_body_path(self):
        got = P.compose_pointer("첫 줄\n둘째  줄", "/x/report.md")
        self.assertEqual(got, "첫 줄 둘째 줄 · 전문: /x/report.md")

    def test_pointer_without_body_has_no_suffix(self):
        self.assertEqual(P.compose_pointer("요약만"), "요약만")
        self.assertNotIn("전문:", P.compose_pointer("요약만"))

    # ── 2) body-file 검증 ──
    def test_missing_body_file_rejected_without_delegation(self):
        fake = FakeWakeup()
        P.run_wakeup = fake
        code, result, why = P.push("master", "k1", "[FYI] x",
                                   os.path.join(self.tmp.name, "없다.md"))
        self.assertEqual(code, P.EXIT_USAGE)
        self.assertIsNone(result)
        self.assertIn("없음", why)
        self.assertEqual(fake.calls, [], "검증 실패인데 wakeup 위임 발생")

    def test_empty_body_file_rejected(self):
        empty = os.path.join(self.tmp.name, "empty.md")
        open(empty, "w").close()
        fake = FakeWakeup()
        P.run_wakeup = fake
        code, _, why = P.push("master", "k1", "[FYI] x", empty)
        self.assertEqual(code, P.EXIT_USAGE)
        self.assertIn("비어", why)
        self.assertEqual(fake.calls, [])

    def test_body_file_relative_path_is_absolutized(self):
        """★B6①: 종전 단언은 조건식이 거의 항상 else 로 떨어져 **절대경로를 절대경로로**
        검증했다(상대경로 분기가 한 번도 실행되지 않음). 실제로 tmp 안으로 chdir 해서
        순수 상대경로("report.md")를 먹인다."""
        prev = os.getcwd()
        os.chdir(self.tmp.name)
        try:
            rel = os.path.basename(self.body)
            self.assertFalse(os.path.isabs(rel), "전제: 입력이 상대경로여야 한다")
            ap, why = P.validate_body_file(rel)
        finally:
            os.chdir(prev)
        self.assertIsNone(why)
        self.assertTrue(os.path.isabs(ap), "상대경로가 절대화되지 않았다")
        self.assertEqual(os.path.realpath(ap), os.path.realpath(self.body))

    def test_relative_body_path_is_absolutized_in_pointer(self):
        """포인터에 실리는 것도 절대경로여야 한다 — 수신자의 cwd 는 발신자와 다르다."""
        fake = FakeWakeup()
        P.run_wakeup = fake
        prev = os.getcwd()
        os.chdir(self.tmp.name)
        try:
            code, _, why = P.push("master", "k", "[FYI] 상대", os.path.basename(self.body))
        finally:
            os.chdir(prev)
        self.assertEqual((code, why), (P.EXIT_OK, None))
        reason = fake.calls[0][fake.calls[0].index("--reason") + 1]
        embedded = reason.split(P.POINTER_BODY_SEP, 1)[1]
        self.assertTrue(os.path.isabs(embedded), "포인터에 상대경로가 실렸다: %r" % embedded)
        self.assertEqual(os.path.realpath(embedded), os.path.realpath(self.body))

    # ── 3) wakeup 위임 인자 ──
    def test_delegation_args(self):
        fake = FakeWakeup(result="queued")
        P.run_wakeup = fake
        code, result, why = P.push("master", "w6-progress", "[FYI] 진행 60%", self.body)
        self.assertEqual((code, result, why), (P.EXIT_OK, "enqueued", None))
        self.assertEqual(len(fake.calls), 1)
        argv = fake.calls[0]
        self.assertEqual(argv[0], "enqueue")
        self.assertEqual(argv[argv.index("--to") + 1], "master")
        self.assertEqual(argv[argv.index("--task") + 1], "w6-progress")
        # ★B1: 멱등키를 넘기면 wakeup 이 "중복 삽입 금지"로 해석해 **최신 보고를 폐기**한다.
        #      같은 주제의 연속 보고는 코얼레스(최신 reason 승리)여야 하므로 키를 넘기지 않는다.
        self.assertNotIn("--idempotency-key", argv,
                         "enqueue 에 멱등키를 넘기면 최신 보고가 suppressed 로 폐기된다(D5 역전)")
        reason = argv[argv.index("--reason") + 1]
        self.assertIn("[FYI] 진행 60%", reason)
        self.assertIn("· 전문: " + os.path.abspath(self.body), reason)

    def test_no_body_file_delegates_pointer_only(self):
        fake = FakeWakeup()
        P.run_wakeup = fake
        P.push("cso", "k", "[FYI] 한 줄")
        reason = fake.calls[0][fake.calls[0].index("--reason") + 1]
        self.assertEqual(reason, "[FYI] 한 줄")

    # ── 4) 결과 매핑 ──
    def test_result_mapping(self):
        for wakeup_result, exp_code, exp_result in (
            ("queued", P.EXIT_OK, "enqueued"),
            ("coalesced", P.EXIT_OK, "coalesced"),
            ("suppressed", P.EXIT_SUPPRESSED, "suppressed"),
        ):
            P.run_wakeup = FakeWakeup(result=wakeup_result)
            code, result, why = P.push("master", "k", "p")
            self.assertEqual((code, result), (exp_code, exp_result), wakeup_result)
            self.assertIsNone(why)

    def test_wakeup_failure_is_delegate_error_with_actionable_message(self):
        """★B6④: 위임 실패는 usage(2)가 아니라 전용 코드(6)다 — 호출자가 '내 인자가 틀렸나'와
        '배달 채널이 죽었나'를 구분할 수 있어야 한다. 메시지에는 재현에 필요한
        wakeup 경로·argv·후속 지시가 들어간다."""
        P.run_wakeup = FakeWakeup(rc=2, out="", err="boom")
        code, result, why = P.push("master", "k", "p")
        self.assertEqual(code, P.EXIT_DELEGATE)
        self.assertNotEqual(P.EXIT_DELEGATE, P.EXIT_USAGE, "코드가 분리되지 않았다")
        self.assertIsNone(result)
        self.assertIn("boom", why)
        self.assertIn("javis_wakeup.py", why, "재현할 wakeup 경로가 없다")
        self.assertIn("enqueue --to master", why, "재현할 argv 가 없다")
        self.assertIn("후속", why, "후속 지시가 없다")

    def test_unparsable_wakeup_output_is_error(self):
        P.run_wakeup = FakeWakeup(rc=0, out="not json")
        code, result, why = P.push("master", "k", "p")
        self.assertEqual(code, P.EXIT_USAGE)
        self.assertIn("해석 불가", why)

    # ── main() 배선 ──
    def test_main_prints_json_and_exit_codes(self):
        P.run_wakeup = FakeWakeup(result="queued")
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = P.main(["--to", "master", "--key", "k", "--pointer", "[FYI] x",
                         "--body-file", self.body])
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(buf.getvalue()), {"result": "enqueued"})

        P.run_wakeup = FakeWakeup(result="suppressed")
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = P.main(["--to", "master", "--key", "k", "--pointer", "[FYI] x"])
        self.assertEqual(rc, P.EXIT_SUPPRESSED)
        self.assertEqual(json.loads(buf.getvalue()), {"result": "suppressed"})

    def test_main_body_file_error_to_stderr(self):
        P.run_wakeup = FakeWakeup()
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = P.main(["--to", "master", "--key", "k", "--pointer", "x",
                         "--body-file", os.path.join(self.tmp.name, "nope.md")])
        self.assertEqual(rc, P.EXIT_USAGE)
        self.assertEqual(out.getvalue(), "")
        self.assertIn("javis_push", err.getvalue())


class PushIntegrationTest(unittest.TestCase):
    """대역 없이 실제 javis_wakeup.py 서브프로세스에 위임 — 계약 표류(인자명 변경 등) 탐지.
    cys 는 호출되지 않는다(enqueue 는 파일 큐만 만든다)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self._prev = os.environ.get("JAVIS_ROOT")
        os.environ["JAVIS_ROOT"] = self.tmp.name

    def tearDown(self):
        if self._prev is None:
            os.environ.pop("JAVIS_ROOT", None)
        else:
            os.environ["JAVIS_ROOT"] = self._prev
        self.tmp.cleanup()

    def test_real_enqueue_then_coalesces_latest_report(self):
        """★B1 의미론 교체: 2회째 push 는 **폐기(suppressed)가 아니라 코얼레스**다.
        진행 보고의 키 의미는 "이 주제의 최신 상태"이므로 최신이 이겨야 한다 —
        종전(멱등키 전달)에는 정확히 반대로 최신분이 버려지고 최초분이 남았다."""
        body = os.path.join(self.tmp.name, "b.md")
        with open(body, "w", encoding="utf-8") as f:
            f.write("전문\n")
        code, result, why = P.push("master", "int-key", "[FYI] 통합", body)
        self.assertEqual((code, result, why), (P.EXIT_OK, "enqueued", None))
        pend = os.path.join(self.tmp.name, "_round", "wakeups", "pending")
        files = os.listdir(pend)
        self.assertEqual(len(files), 1, files)
        rec = json.load(open(os.path.join(pend, files[0]), encoding="utf-8"))
        self.assertEqual(rec["task_key"], "int-key")
        self.assertIn("· 전문: " + os.path.abspath(body), rec["reason"])

        # 2회째: 코얼레스(exit 0) — 큐는 여전히 1건이고 reason 은 최신으로 갱신된다.
        code2, result2, why2 = P.push("master", "int-key", "[FYI] 통합 2 최신", body)
        self.assertEqual((code2, result2, why2), (P.EXIT_OK, "coalesced", None),
                         "최신 보고가 폐기됐다(멱등키 전달 회귀)")
        self.assertEqual(len(os.listdir(pend)), 1, "코얼레스인데 큐가 늘었다")
        rec2 = json.load(open(os.path.join(pend, files[0]), encoding="utf-8"))
        self.assertIn("[FYI] 통합 2 최신", rec2["reason"], "★최신 보고가 반영되지 않았다")
        self.assertEqual(rec2["coalesced_count"], 1)
        self.assertEqual(rec2["id"], rec["id"], "코얼레스는 같은 wakeup 에 병합된다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
