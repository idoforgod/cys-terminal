#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_ticket_snapshot.py — 위임 티켓의 todo 선언 스냅샷 (C7).

배경(DESIGN §6-5): 티켓 문자열과 javis_orchestra 내부 무결성 assertion 은 **원자 동기** 대상이다.
한쪽만 바뀌면 전 티켓 발급이 실패한다. 여기서는 그 짝을 밖에서 한 번 더 못박는다:

  4) javis_orchestra --self-test(내부 assertion) 통과 = 티켓·assertion 동기 확인
  5) 하위호환 — probes/success 미지정 시 해당 블록 부재는 그대로
  6) todo 선언 블록(설계 DESIGN_declared-state.md §4-1) — ★0.14.56 #6-a 부터 티켓은 선언을 동봉하지 않고
     수신 좌석의 생산자(`cys todo-path --emit-decl`)를 지시한다. 여기서는 발부자 선언·팩 이름·작업명이 티켓에
     번지지 않는 것과 위치·은퇴 계약을 핀한다. 선언 문법의 핀은 생산자 쪽 검체(src/bin/cys.rs)가 소유한다.
  7) ★W14 S14 → 0.14.56 #6-a — 경로와 선언 scope 는 수신 좌석의 한 생산자에서 나온다(더 강한 형태)

⚠이 브랜치에서 **의도적으로 빠진 것**(원천 상류에는 있다): 1)~3) 보고 채널 3분류 규약 스냅샷
  (FYI=javis_push · ACTION=--queued · 긴급=직접 send). 그 규약과 그것이 지시하는 도구
  (`bin/javis_push.py`)가 이 브랜치에 아직 없어서(티켓은 구 단일 `보고 채널:` 한 줄을 싣는다),
  스냅샷만 먼저 들이면 존재하지 않는 기능을 단언하게 된다. 3분류 규약을 이식할 때 상류의
  1)~3) 블록을 원문 그대로 함께 되살려라 — 이 주석이 그 미이식의 유일한 기록이다.

실행: python3 test_orchestra_ticket_snapshot.py
"""
import os
import re
import subprocess
import sys
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_orchestra as O                                                # noqa: E402


class TicketSnapshotTest(unittest.TestCase):
    def ticket(self, **kw):
        return O.build_task_ticket("T", "S", "C", "worker", rules=O.FALLBACK_RULES, **kw)

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

    # ── 6) todo 선언 블록(설계 §4-1) ──
    #    소비자 파서(javis_todo_decl / src/todo_decl.rs)의 문법을 티켓 쪽에서 독립 재현해
    #    핀한다. 파서를 import 하지 않는 것은 의도적이다 — 같은 구현을 양쪽에서 부르면
    #    둘이 함께 틀려도 초록이 된다(계약 검증이 아니라 자기 반사가 된다).
    DECL_RE = re.compile(r"<!--\s*javis:todo\s+(v\d+)\s+(.*?)\s*-->$")
    KV_RE = re.compile(r"^([A-Za-z][A-Za-z0-9_]*)=([A-Za-z0-9._:-]+)$")

    def decl_line(self, ticket):
        for ln in ticket.splitlines():
            if "javis:todo" in ln and ln.strip().startswith("<!--"):
                return ln.strip()
        return ""

    def test_ticket_carries_no_issuer_declaration(self):
        """0.14.56 #6-a — 선언 한 줄은 발부자가 아니라 수신 좌석의 생산자(`cys todo-path --emit-decl`)가 만든다.
        종전에는 발부자가 `--to` 역할로 `owner=` 를 박았고, 수신 좌석의 역할이 다르면 그 선언은 남의 것이었다.
        문법(G4 값 문자집합 · G5 필수 키)의 핀은 생산자 쪽 검체(src/bin/cys.rs build_todo_decl_line)가 소유한다."""
        t = self.ticket()
        self.assertEqual(self.decl_line(t), "", "발부자가 선언을 티켓에 박았다")
        self.assertIn("cys todo-path --emit-decl", t, "선언을 수신 좌석 생산자에게서 받으라는 지시가 없다")

    def test_issuer_pack_never_reaches_ticket(self):
        """발부자의 팩 이름(scope)은 티켓에 새지 않는다 — 수신 좌석은 다른 레인일 수 있다(CEO → 부서)."""
        old = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = "/tmp/pack-dept-dept-9"
        try:
            t = self.ticket()
            self.assertNotIn("pack-dept-dept-9", t)
            self.assertNotIn("scope=", t)
        finally:
            if old is None:
                os.environ.pop("CYS_PACK_DIR", None)
            else:
                os.environ["CYS_PACK_DIR"] = old

    def test_task_text_never_becomes_a_declaration(self):
        """작업명(한글·공백 포함)이 선언 값으로 번지지 않는다 — 발부자 선언이 없으므로 lane 슬러그도 없다."""
        for task in ("유령 todo 결함 수정", "유령 결함"):
            t = O.build_task_ticket(task, "S", None, "worker", rules=O.FALLBACK_RULES)
            self.assertEqual(self.decl_line(t), "")
            self.assertNotIn("lane=", t)

    def test_todo_decl_instruction_carries_position_and_retire(self):
        """선언 문구는 두 계약을 반드시 실어야 한다: ①위치(첫 체크박스보다 위 — 어기면 본문이
        스스로를 무효화하는 A2 자해 경로) ②은퇴 전이(아무도 retired를 선언하지 않으면
        미선언·유령이 영구 누적된다)."""
        t = self.ticket()
        self.assertIn("todo 선언", t)
        self.assertIn("첫 체크박스보다 위", t)
        self.assertIn("status=retired", t)

    # ── 7) ★W14 S14 → 0.14.56 #6-a — 경로와 선언 scope 는 **한 생산자**에서 나온다 ──
    #    S14 가 막던 것: 경로(워커 셸에서 늦게 전개)와 선언 scope(master 에서 즉시)가 갈려 워커 파일이
    #    `foreign-scope` 로 조용히 배제되는 것. 이제 둘 다 수신 좌석의 `cys todo-path` 가 만든다 —
    #    발부자 쪽 바인딩이 아예 없으므로 갈릴 두 바인딩이 존재하지 않는다(더 강한 형태).

    def test_todo_path_and_declaration_come_from_one_recipient_producer(self):
        """todo 지시는 `cys todo-path` 를 가리키고 파일을 이름 붙이지 않으며, 수신자 역할과 무관하게 동일하다."""
        def todo_line(t):
            lines = [ln for ln in t.splitlines() if ln.startswith("todo 영속:")]
            self.assertEqual(len(lines), 1, "todo 영속 지시가 1줄이 아니다")
            return lines[0]
        a = todo_line(self.ticket())
        b = todo_line(O.build_task_ticket("T", "S", "C", "worker-2", rules=O.FALLBACK_RULES))
        self.assertIn("`cys todo-path`", a)
        self.assertNotIn("_TODO.md", a, "발부자가 todo 파일을 이름 붙였다")
        self.assertNotIn("${CYS_PACK_DIR", a, "경로가 워커 셸에서 늦게 전개된다(S14 재발)")
        self.assertEqual(a, b, "todo 지시가 수신자 역할에 따라 달라졌다(발부자가 파일을 고른다)")

    def test_illegal_pack_identity_cannot_reach_the_ticket(self):
        """팩 이름이 G4 밖이어도 그럴듯한 정체성이 배포되지 않는다 — 발부자 정체성은 티켓에 없고,
        수신 좌석 생산자는 G4 밖 이름에서 시끄럽게 실패한다(그때 손으로 만들지 말고 보고하라는 지시)."""
        old = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = "/tmp/자비스"
        try:
            t = self.ticket()
            self.assertEqual(self.decl_line(t), "")
            self.assertNotIn("scope=", t, "폴백 정체성이 티켓에 실렸다")
            self.assertNotIn("자비스", t)
            self.assertIn("실패하면 파일을 손으로 만들지 말고", t, "생산자 실패 시 처리 지시가 없다(조용한 실패)")
        finally:
            if old is None:
                os.environ.pop("CYS_PACK_DIR", None)
            else:
                os.environ["CYS_PACK_DIR"] = old

    def test_e1_p3_evidence_gates_still_present(self):
        t = self.ticket()
        self.assertIn("완료 증거(E1 evidence-artifact 게이트", t)
        self.assertIn("done 증거 게이트(P3)", t)


if __name__ == "__main__":
    unittest.main(verbosity=2)
