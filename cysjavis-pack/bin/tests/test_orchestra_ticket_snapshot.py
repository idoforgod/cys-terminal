#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_ticket_snapshot.py — 위임 티켓·리뷰 프롬프트의 보고 채널 스냅샷 (C7).

배경(DESIGN §6-5): 티켓 문자열과 javis_orchestra 내부 무결성 assertion 은 **원자 동기** 대상이다.
한쪽만 바뀌면 전 티켓 발급이 실패한다. 여기서는 그 짝을 밖에서 한 번 더 못박는다:

  1) 보고 채널 블록 스냅샷 — 3분류(FYI=javis_push · ACTION=--queued · 긴급=직접 send+send-key)
     4줄을 **정확한 문자열**로 핀(문구 표류 즉시 탐지 → 의도적 변경이면 이 스냅샷도 같이 고친다)
  2) 구 규약("[보고] ..." 단일 채널) 회귀 차단
  3) cmd_review_prompt 회신 채널도 동일 3분류
  4) javis_orchestra --self-test(내부 assertion) 통과 = 티켓·assertion 동기 확인
  5) 하위호환 — probes/success 미지정 시 해당 블록 부재는 그대로

실행: python3 test_orchestra_ticket_snapshot.py
"""
import io
import os
import subprocess
import sys
import unittest
from contextlib import redirect_stdout

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_orchestra as O                                                # noqa: E402

# ── 스냅샷: 위임 티켓의 보고 채널 블록(3분류 규약) ──
EXPECTED_TICKET_CHANNEL = [
    "보고 채널(3분류 규약 — 이 분류를 어기면 master 큐가 포화된다):",
    "  ① 진행·상태 보고(FYI — master는 회신하지 않는다): 전문을 md 파일로 쓰고 "
    "`python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_push.py\" --to master "
    "--key <task-slug> --pointer \"[FYI] 1줄 요약\" --body-file <전문 md 절대경로>` "
    "— 채팅에는 포인터 1줄만 간다(같은 --key의 연속 보고는 자동 병합). "
    "진행률·중간 산출물·조사 결과는 전부 이 채널이다.",
    "  ② 게이트 전이·완료·질문·막힘(ACTION — 1회 ACK 대상): "
    "`cys send --queued --to master \"[ACTION] ...\"` 로 직접 push하라"
    "(--queued는 자동 Return 배달 — send-key 불필요·타이핑 가드 안전). "
    "ACK에 대한 ACK 금지 — 받았다는 답에 다시 답하지 마라.",
    "  ③ 즉시 끼어들어야 할 긴급: 직접 send 후 `cys send-key --to master "
    "Return`(가드 차단 시 --queued로 전환). 남용 금지 — 위 ①②로 대체 가능하면 긴급이 아니다.",
]


def channel_block(text):
    """'보고 채널'/'회신 채널' 로 시작하는 줄부터 분류 항목(①②③)까지."""
    lines = text.splitlines()
    for i, ln in enumerate(lines):
        if ln.startswith("보고 채널") or ln.startswith("회신 채널"):
            block = [ln]
            for nxt in lines[i + 1:]:
                if nxt.startswith("  ①") or nxt.startswith("  ②") or nxt.startswith("  ③"):
                    block.append(nxt)
                else:
                    break
            return block
    return []


class TicketSnapshotTest(unittest.TestCase):
    def ticket(self, **kw):
        return O.build_task_ticket("T", "S", "C", "worker", rules=O.FALLBACK_RULES, **kw)

    # ── 1) 스냅샷 ──
    def test_report_channel_snapshot(self):
        got = channel_block(self.ticket())
        self.assertEqual(got, EXPECTED_TICKET_CHANNEL,
                         "보고 채널 문구가 표류했다 — 의도적 개정이면 이 스냅샷과 "
                         "javis_orchestra 내부 assertion을 같은 커밋에서 함께 고쳐라")

    def test_three_channels_all_present(self):
        t = self.ticket()
        for must in ("[FYI]", "javis_push.py", "--body-file",
                     "[ACTION]", "--queued", "send-key", "ACK에 대한 ACK 금지"):
            self.assertIn(must, t, must)

    # ── 2) 구 규약 회귀 차단 ──
    def test_legacy_single_channel_gone(self):
        t = self.ticket()
        self.assertNotIn('"[보고] ..."', t, "구 단일 보고 채널 문구 부활(3분류 무력화)")

    # ── 3) 리뷰 프롬프트 ──
    def test_review_prompt_channel(self):
        class A:
            task, scope, reviewer, round = "T", "S", None, 2
        buf = io.StringIO()
        with redirect_stdout(buf):
            O.cmd_review_prompt(A())
        out = buf.getvalue()
        block = channel_block(out)
        self.assertTrue(block and block[0].startswith("회신 채널(3분류 규약"), block[:1])
        self.assertEqual(len(block), 4, "회신 채널 3분류 항목이 3개가 아니다: %s" % block)
        for must in ("[ACTION]", "--queued", "[FYI]", "javis_push.py", "--body-file", "send-key"):
            self.assertIn(must, out, must)

    # ── 4) 내부 assertion 동기 ──
    def test_orchestra_self_test_passes(self):
        env = dict(os.environ)
        env.setdefault("CYS_PACK_DIR", BIN)
        r = subprocess.run([sys.executable, os.path.join(BIN, "javis_orchestra.py"), "--self-test"],
                           capture_output=True, text=True, timeout=120, env=env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("self-test OK", r.stdout)

    # ── 5) 하위호환 ──
    def test_optional_blocks_absent_by_default(self):
        t = self.ticket()
        self.assertNotIn("필수 probe", t)
        self.assertNotIn("무접촉", t)
        self.assertIn("필수 probe", self.ticket(probes=["submit"]))
        self.assertNotIn("성공 기준", O.build_task_ticket("T", "S", None, "worker",
                                                        rules=O.FALLBACK_RULES))

    def test_e1_p3_evidence_gates_still_present(self):
        t = self.ticket()
        self.assertIn("완료 증거(E1 evidence-artifact 게이트", t)
        self.assertIn("done 증거 게이트(P3)", t)


if __name__ == "__main__":
    unittest.main(verbosity=2)
