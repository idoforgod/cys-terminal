#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_deadletter.py — javis_deadletter.py(C7 dead-letter 전사·다이제스트) 회귀.

검증 축:
  1) 증분 오프셋 — 1회차 전사 후 재실행은 무작업(exit 5), 새 행 추가분만 전사
  2) 다중 state dir — main + dept 를 모두 순회하고 dir 별 섹션으로 분리
  3) 회전(rename) — 회전본을 재전사하지 않는다((dev,ino) 키)
  4) --dry-run — md·오프셋·push 부작용 0
  5) 원장 부재 = exit 5(정상 무작업) · 비밀 마스킹(scrub) · 반쪽 라인 미소비

push_digest 는 대역으로 치환한다(실제 wakeup·cys 미호출).

실행: python3 test_deadletter.py   (unittest·파일 직접 실행 관례)
"""
import io
import json
import os
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout, redirect_stderr

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_deadletter as D                                               # noqa: E402


def row(text="msg", reason="ttl", surface_id=7, role="worker", origin=None, seq=1):
    """★B3⑤: **실제 데몬 출력 형식**의 픽스처.

    종전 픽스처는 origin 을 문자열 "agent" 로 넣었는데, 데몬(queue_policy::record_dead_letter)은
    QueueOrigin 을 그대로 직렬화해 **객체**를 쓴다. 픽스처가 실물과 달라서 렌더러가 dict 를
    받는 경로가 테스트에서 한 번도 실행되지 않았다(그 결과 md 에 dict repr 이 박혔다).
    필드 집합도 데몬 계약 그대로 맞춘다(kind·enqueued_at·idem_key·merged_count 포함)."""
    if origin is None:
        origin = {"class": "agent", "surface": surface_id, "role": role}
    return {"ts": 1753500000.0, "surface_id": surface_id, "role": role,
            "text": text, "origin": origin, "reason": reason, "seq": seq,
            "kind": "text", "enqueued_at": 1753499900.0,
            "idem_key": None, "merged_count": 0}


def append_rows(state_dir, rows, name=D.DEAD_LETTER_FILE):
    os.makedirs(state_dir, exist_ok=True)
    with open(os.path.join(state_dir, name), "a", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


class FakeDigest:
    def __init__(self):
        self.calls = []

    def __call__(self, summary, dry_run=False):
        self.calls.append((summary, dry_run))
        return 0, "fake"


class DeadLetterTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.main_dir = os.path.join(self.tmp.name, "state", "cys")
        self.dept_dir = os.path.join(self.tmp.name, "state", "cys-dept-ceo")
        self.out = os.path.join(self.tmp.name, "_round", "dead_letters")
        self.digest = FakeDigest()
        self._orig_digest = D.push_digest
        D.push_digest = self.digest

    def tearDown(self):
        D.push_digest = self._orig_digest
        self.tmp.cleanup()

    def run_tool(self, *extra, dirs=None):
        argv = []
        for d in (dirs if dirs is not None else [self.main_dir]):
            argv += ["--state-dir", d]
        argv += ["--out-dir", self.out]
        argv += list(extra)
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = D.main(argv)
        return rc, buf.getvalue()

    def md_path(self):
        return os.path.join(self.out, time.strftime("%Y-%m-%d") + ".md")

    def md(self):
        with open(self.md_path(), encoding="utf-8") as f:
            return f.read()

    # ── 5) 원장 부재 ──
    def test_no_ledger_is_exit_5(self):
        os.makedirs(self.main_dir)
        rc, out = self.run_tool()
        self.assertEqual(rc, D.EXIT_EMPTY)
        self.assertFalse(os.path.exists(self.md_path()))
        self.assertEqual(self.digest.calls, [])

    def test_missing_state_dir_is_exit_5(self):
        rc, _ = self.run_tool(dirs=[os.path.join(self.tmp.name, "없다")])
        self.assertEqual(rc, D.EXIT_EMPTY)

    # ── 1) 증분 오프셋 ──
    def test_incremental_offsets(self):
        append_rows(self.main_dir, [row("첫 번째"), row("두 번째", reason="softcap_rejected")])
        rc, out = self.run_tool()
        self.assertEqual(rc, D.EXIT_OK)
        self.assertEqual(json.loads(out)["new_rows"], 2)
        body = self.md()
        self.assertIn("첫 번째", body)
        self.assertIn("두 번째", body)
        self.assertIn("| ttl | 1 |", body)
        self.assertIn("| softcap_rejected | 1 |", body)
        self.assertEqual(len(self.digest.calls), 1)
        self.assertIn("신규 2건", self.digest.calls[0][0])

        # 재실행 = 신규 0 → 무작업(exit 5)·md 불변·push 없음
        before = self.md()
        rc2, out2 = self.run_tool()
        self.assertEqual(rc2, D.EXIT_EMPTY)
        self.assertEqual(self.md(), before)
        self.assertEqual(len(self.digest.calls), 1)

        # 새 행 추가 → 추가분만 전사
        append_rows(self.main_dir, [row("세 번째")])
        rc3, out3 = self.run_tool()
        self.assertEqual(rc3, D.EXIT_OK)
        self.assertEqual(json.loads(out3)["new_rows"], 1)
        added = self.md()[len(before):]
        self.assertIn("세 번째", added)
        self.assertNotIn("첫 번째", added, "이미 전사한 행이 재전사됐다(증분 실패)")

    def test_offsets_file_written(self):
        append_rows(self.main_dir, [row()])
        self.run_tool()
        off = D.load_offsets(os.path.join(self.out, ".offsets.json"))
        self.assertEqual(len(off), 1)
        rec = list(off.values())[0]
        self.assertGreater(rec["offset"], 0)
        self.assertTrue(rec["path"].endswith(D.DEAD_LETTER_FILE))

    def test_truncated_file_restarts_from_zero(self):
        append_rows(self.main_dir, [row("옛것")])
        self.run_tool()
        # 파일이 더 작아짐(교체/truncate) → 0부터 재판독
        with open(os.path.join(self.main_dir, D.DEAD_LETTER_FILE), "w", encoding="utf-8") as f:
            f.write(json.dumps(row("새것"), ensure_ascii=False) + "\n")
        rc, out = self.run_tool()
        self.assertEqual(rc, D.EXIT_OK)
        self.assertIn("새것", self.md())

    def test_partial_last_line_not_consumed(self):
        path = os.path.join(self.main_dir, D.DEAD_LETTER_FILE)
        os.makedirs(self.main_dir, exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(json.dumps(row("완결"), ensure_ascii=False) + "\n")
            f.write('{"ts": 1, "text": "반쪽')          # 개행 없는 미완 라인
        rc, out = self.run_tool()
        self.assertEqual(json.loads(out)["new_rows"], 1)
        self.assertNotIn("반쪽", self.md())
        # 라인이 완결되면 그때 전사된다
        with open(path, "a", encoding="utf-8") as f:
            f.write('"}\n')
        rc2, out2 = self.run_tool()
        self.assertEqual(json.loads(out2)["new_rows"], 1)
        self.assertIn("반쪽", self.md())

    # ── 2) 다중 dir ──
    def test_multiple_state_dirs_get_separate_sections(self):
        append_rows(self.main_dir, [row("메인건")])
        append_rows(self.dept_dir, [row("부서건", reason="wakeup_cap")])
        rc, out = self.run_tool(dirs=[self.main_dir, self.dept_dir])
        self.assertEqual(rc, D.EXIT_OK)
        self.assertEqual(json.loads(out)["new_rows"], 2)
        body = self.md()
        self.assertIn("## " + self.main_dir, body)
        self.assertIn("## " + self.dept_dir, body)
        self.assertIn("메인건", body)
        self.assertIn("부서건", body)

    # ── 3) 회전본 ──
    def test_rotated_file_not_retranscribed(self):
        path = os.path.join(self.main_dir, D.DEAD_LETTER_FILE)
        append_rows(self.main_dir, [row("회전전")])
        self.run_tool()
        os.rename(path, os.path.join(self.main_dir, "dead-letters.1753500001.jsonl"))
        append_rows(self.main_dir, [row("회전후")])
        rc, out = self.run_tool()
        self.assertEqual(json.loads(out)["new_rows"], 1, "회전본 재전사 발생((dev,ino) 키 실패)")
        self.assertEqual(self.md().count("회전전"), 1)
        self.assertIn("회전후", self.md())

    def test_rotated_backlog_is_transcribed_once(self):
        """첫 실행 전에 이미 회전이 있었으면 회전본도 1회 전사된다."""
        append_rows(self.main_dir, [row("과거건")], name="dead-letters.1753500001.jsonl")
        append_rows(self.main_dir, [row("현행건")])
        rc, out = self.run_tool()
        self.assertEqual(json.loads(out)["new_rows"], 2)
        self.assertIn("과거건", self.md())
        self.assertIn("현행건", self.md())

    # ── 4) dry-run ──
    def test_dry_run_has_no_side_effects(self):
        append_rows(self.main_dir, [row("드라이")])
        rc, out = self.run_tool("--dry-run")
        self.assertEqual(rc, D.EXIT_OK)
        self.assertIn("드라이", out)
        self.assertFalse(os.path.exists(self.md_path()), "dry-run이 md를 썼다")
        self.assertFalse(os.path.exists(os.path.join(self.out, ".offsets.json")),
                         "dry-run이 오프셋을 갱신했다")
        self.assertEqual(len(self.digest.calls), 1)
        self.assertTrue(self.digest.calls[0][1], "dry-run인데 실제 push 경로가 탔다")
        # dry-run 뒤 실제 실행은 여전히 전량 전사한다(오프셋 미전진 확인)
        rc2, out2 = self.run_tool()
        self.assertEqual(json.loads(out2)["new_rows"], 1)

    # ── 부가: scrub ──
    def test_secrets_are_masked_in_transcript(self):
        append_rows(self.main_dir, [row("api_key=SUPERSECRETVALUE123")])
        self.run_tool()
        body = self.md()
        self.assertNotIn("SUPERSECRETVALUE123", body)
        self.assertIn("마스킹된 비밀값", body)

    def test_parse_error_row_is_preserved_not_dropped(self):
        os.makedirs(self.main_dir, exist_ok=True)
        with open(os.path.join(self.main_dir, D.DEAD_LETTER_FILE), "w", encoding="utf-8") as f:
            f.write("{깨진 json}\n")
        rc, out = self.run_tool()
        self.assertEqual(json.loads(out)["new_rows"], 1)
        self.assertIn("깨진 json", self.md())


    # ── ★B3⑤ origin 렌더 ──
    def test_origin_object_is_rendered_readably(self):
        append_rows(self.main_dir, [row("객체 origin")])
        self.run_tool()
        body = self.md()
        self.assertIn("origin=agent(surface:7,worker)", body,
                      "origin 객체가 사람이 읽을 형태로 렌더되지 않았다")
        self.assertNotIn("{'class'", body, "dict repr 이 md 에 그대로 박혔다")
        self.assertNotIn('{"class"', body)

    def test_render_origin_handles_system_and_legacy_forms(self):
        self.assertEqual(D.render_origin({"class": "system", "label": "boot"}), "system(boot)")
        self.assertEqual(D.render_origin({"class": "human"}), "human")
        self.assertEqual(D.render_origin("agent"), "agent", "구 문자열 origin 은 그대로 통과")
        self.assertEqual(D.render_origin(None), "None")

    # ── ★B3⑥ 동일 크기 교체 → 지문 불일치 → 재전사 ──
    def test_same_size_replacement_is_detected_and_retranscribed(self):
        """(dev,ino) 는 같고 **크기까지 같은** 교체는 오프셋만으로는 구분되지 않는다 —
        head 지문 분기가 그래서 있는데, 종전 테스트는 truncate(크기 감소)만 덮어
        **이 분기를 한 번도 실행하지 않았다**. 같은 바이트 수로 내용을 갈아끼운다."""
        path = os.path.join(self.main_dir, D.DEAD_LETTER_FILE)
        os.makedirs(self.main_dir, exist_ok=True)
        pad = "P" * (D.HEAD_N + 64)          # 선두 지문 길이를 확실히 넘긴다
        first = json.dumps(row("A" + pad, surface_id=7), ensure_ascii=False) + "\n"
        second = json.dumps(row("B" + pad, surface_id=7), ensure_ascii=False) + "\n"
        self.assertEqual(len(first), len(second), "전제: 두 내용의 바이트 수가 같아야 한다")
        self.assertGreater(len(first), D.HEAD_N, "전제: 선두 지문 길이보다 길어야 한다")

        with open(path, "w", encoding="utf-8") as f:
            f.write(first)
        rc, out = self.run_tool()
        self.assertEqual(json.loads(out)["new_rows"], 1)
        st1 = os.stat(path)

        # 제자리 덮어쓰기 — 아이노드·크기 동일, 내용만 교체.
        with open(path, "r+", encoding="utf-8") as f:
            f.write(second)
        st2 = os.stat(path)
        self.assertEqual((st1.st_dev, st1.st_ino), (st2.st_dev, st2.st_ino),
                         "전제: 같은 아이노드여야 한다")
        self.assertEqual(st1.st_size, st2.st_size, "전제: 같은 크기여야 한다")

        rc2, out2 = self.run_tool()
        self.assertEqual(json.loads(out2)["new_rows"], 1,
                         "동일 크기 교체를 탐지하지 못했다(head 지문 분기 미실행)")
        self.assertIn("B" + pad, self.md())

    # ── ★B3⑦ 소멸 파일 오프셋 prune ──
    def test_offsets_prune_vanished_files(self):
        path = os.path.join(self.main_dir, D.DEAD_LETTER_FILE)
        append_rows(self.main_dir, [row("회전 대상")])
        self.run_tool()
        rotated = os.path.join(self.main_dir, "dead-letters.1753500001.jsonl")
        os.rename(path, rotated)
        append_rows(self.main_dir, [row("현행")])
        self.run_tool()
        self.assertEqual(len(D.load_offsets(os.path.join(self.out, ".offsets.json"))), 2)

        # 회전본을 지우면 그 오프셋 항목도 사라져야 한다((dev,ino) 재사용 오염 차단).
        os.remove(rotated)
        append_rows(self.main_dir, [row("추가")])
        self.run_tool()
        off = D.load_offsets(os.path.join(self.out, ".offsets.json"))
        self.assertEqual(len(off), 1, "소멸 파일의 오프셋 항목이 남았다: %s" % off)
        self.assertTrue(list(off.values())[0]["path"].endswith(D.DEAD_LETTER_FILE))

    # ── ★B3④ 빈 결과에 검사 경로 노출 ──
    def test_empty_result_lists_checked_paths(self):
        os.makedirs(self.main_dir)
        rc, out = self.run_tool()
        self.assertEqual(rc, D.EXIT_EMPTY)
        self.assertIn(self.main_dir, out, "검사한 경로가 출력되지 않아 오진단을 유발한다")

    def test_no_new_rows_reports_checked_dirs(self):
        append_rows(self.main_dir, [row()])
        self.run_tool()
        rc, out = self.run_tool()
        self.assertEqual(rc, D.EXIT_EMPTY)
        self.assertIn(self.main_dir, json.loads(out)["checked"])

    # ── ★B3③ digest 실패 = degraded(exit 4) ──
    def test_digest_failure_is_degraded_not_silent_success(self):
        class FailingDigest:
            def __init__(self):
                self.calls = []

            def __call__(self, summary, dry_run=False):
                self.calls.append((summary, dry_run))
                return 6, "배달 위임 실패(rc=2): boom"

        D.push_digest = FailingDigest()
        append_rows(self.main_dir, [row("전사는 된다")])
        buf, errbuf = io.StringIO(), io.StringIO()
        with redirect_stdout(buf), redirect_stderr(errbuf):
            rc = D.main(["--state-dir", self.main_dir, "--out-dir", self.out])
        self.assertEqual(rc, D.EXIT_DEGRADED, "digest 실패가 무음 성공으로 처리됐다")
        self.assertIn("전사는 된다", self.md(), "degraded 여도 전사 자체는 완료돼야 한다")
        self.assertIn("digest push 실패", errbuf.getvalue())


class DigestDelegationTest(unittest.TestCase):
    """★B3②: digest push 는 javis_push.push() 형제 import 로 위임한다 —
    자체 서브프로세스로 wakeup 을 부르면 보고 채널 규약(B1 코얼레스 의미론)을 우회한다."""

    def setUp(self):
        self.calls = []
        self._orig = D.javis_push.push

        def fake_push(to, key, pointer, body_file=None):
            self.calls.append((to, key, pointer, body_file))
            return self.result

        self.result = (D.javis_push.EXIT_OK, "enqueued", None)
        D.javis_push.push = fake_push

    def tearDown(self):
        D.javis_push.push = self._orig

    def test_push_digest_delegates_to_javis_push(self):
        rc, msg = D.push_digest("[dead-letter] 신규 3건")
        self.assertEqual(rc, 0)
        self.assertEqual(len(self.calls), 1)
        to, key, pointer, body = self.calls[0]
        self.assertEqual((to, key), ("master", D.DIGEST_KEY))
        self.assertIn("신규 3건", pointer)
        self.assertIsNone(body)

    def test_suppressed_is_success_not_failure(self):
        self.result = (D.javis_push.EXIT_SUPPRESSED, "suppressed", None)
        rc, _ = D.push_digest("x")
        self.assertEqual(rc, 0, "멱등 억제는 무작업이지 실패가 아니다")

    def test_delegate_failure_propagates_nonzero(self):
        self.result = (D.javis_push.EXIT_DELEGATE, None, "boom")
        rc, msg = D.push_digest("x")
        self.assertEqual(rc, D.javis_push.EXIT_DELEGATE)
        self.assertIn("boom", msg)

    def test_dry_run_does_not_call_push(self):
        rc, msg = D.push_digest("x", dry_run=True)
        self.assertEqual((rc, self.calls), (0, []))
        self.assertIn("DRYRUN", msg)


if __name__ == "__main__":
    unittest.main(verbosity=2)
