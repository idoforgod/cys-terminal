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
from contextlib import redirect_stdout

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_deadletter as D                                               # noqa: E402


def row(text="msg", reason="ttl", surface_id=7, role="worker", origin="agent"):
    return {"ts": 1753500000.0, "surface_id": surface_id, "role": role,
            "text": text, "origin": origin, "reason": reason, "seq": 1}


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


if __name__ == "__main__":
    unittest.main(verbosity=2)
