#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_orchestra_ticket_snapshot.py — 위임 티켓의 todo 선언 스냅샷 (C7).

배경(DESIGN §6-5): 티켓 문자열과 javis_orchestra 내부 무결성 assertion 은 **원자 동기** 대상이다.
한쪽만 바뀌면 전 티켓 발급이 실패한다. 여기서는 그 짝을 밖에서 한 번 더 못박는다:

  4) javis_orchestra --self-test(내부 assertion) 통과 = 티켓·assertion 동기 확인
  5) 하위호환 — probes/success 미지정 시 해당 블록 부재는 그대로
  6) todo 선언 블록(설계 DESIGN_declared-state.md §4-1) — 티켓이 동봉하는 선언 한 줄의 **문법**을
     핀한다. 문구가 아니라 문법을 보는 이유: 티켓이 문법 위반 선언을 배포하면 소비자 파서가
     전건 '미선언'으로 버려 선언 배선 전체가 조용히 무의미해진다(실패가 티켓 쪽에 안 보인다).
  7) ★W14 S14 — 경로·scope 바인딩 시점 통일 + 실패의 시끄러움

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

    def test_todo_decl_line_syntax(self):
        """티켓 동봉 선언이 v1 문법(G4 값 문자집합 · G5 필수 키 3종)을 만족한다."""
        line = self.decl_line(self.ticket())
        self.assertTrue(line, "티켓에 todo 선언 한 줄이 없다")
        m = self.DECL_RE.match(line)
        self.assertTrue(m, "선언이 `<!-- javis:todo v1 k=v ... -->` 형태가 아니다: %r" % line)
        self.assertEqual(m.group(1), "v1", "미지 버전은 소비자가 미선언 취급한다: %r" % line)
        kv = {}
        for tok in m.group(2).split():
            km = self.KV_RE.match(tok)
            self.assertTrue(km, "값 문법 위반 토큰(따옴표·공백·대문자 키 금지): %r" % tok)
            kv[km.group(1)] = km.group(2)
        for req in ("owner", "scope", "status"):
            self.assertIn(req, kv, "필수 키 누락 → 미선언 판정: %r" % line)
        self.assertEqual(kv["owner"], "worker", "owner는 위임 대상 역할이어야 한다")
        self.assertEqual(kv["status"], "active", "신규 위임 todo는 active로 시작한다")
        self.assertTrue(kv["scope"], "scope(귀속 팩 정체성)가 비었다")
        self.assertRegex(kv.get("since", ""), r"^\d{4}-\d{2}-\d{2}$")

    def test_todo_decl_scope_follows_pack_dir(self):
        """scope는 하드코딩이 아니라 pack_dir() basename에서 나온다 — 부서 팩
        (`pack-dept-dept-1` 등)에서 하드코딩하면 그 팩의 todo가 전부 '남의 레인'으로
        오분류된다(설계 §4-2 foreign-scope)."""
        old = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = "/tmp/pack-dept-dept-9"
        try:
            self.assertIn("scope=pack-dept-dept-9", self.decl_line(self.ticket()))
        finally:
            if old is None:
                os.environ.pop("CYS_PACK_DIR", None)
            else:
                os.environ["CYS_PACK_DIR"] = old

    def test_todo_decl_lane_slug_normalized(self):
        """lane은 진단용 선택 키. 한글·공백이 그대로 들어가면 G4 위반으로 **선언 전체**가
        무효가 되므로, 슬러그화하고 남는 게 없으면 키 자체를 생략해야 한다."""
        t = O.build_task_ticket("유령 todo 결함 수정", "S", None, "worker", rules=O.FALLBACK_RULES)
        self.assertIn("lane=todo", self.decl_line(t))
        t2 = O.build_task_ticket("유령 결함", "S", None, "worker", rules=O.FALLBACK_RULES)
        self.assertNotIn("lane=", self.decl_line(t2))
        self.assertTrue(self.DECL_RE.match(self.decl_line(t2)), "lane 생략 시 선언이 깨졌다")

    def test_todo_decl_instruction_carries_position_and_retire(self):
        """선언 문구는 두 계약을 반드시 실어야 한다: ①위치(첫 체크박스보다 위 — 어기면 본문이
        스스로를 무효화하는 A2 자해 경로) ②은퇴 전이(아무도 retired를 선언하지 않으면
        미선언·유령이 영구 누적된다)."""
        t = self.ticket()
        self.assertIn("todo 선언", t)
        self.assertIn("첫 체크박스보다 위", t)
        self.assertIn("status=retired", t)

    # ── 7) ★W14 S14 — 경로·scope **바인딩 시점 통일** + 실패의 시끄러움 ──
    #    이 두 테스트가 지키는 것: 티켓 안에서 "어디에 쓸지"와 "누구 것이라 선언할지"가
    #    **같은 시점·같은 값**에서 나온다는 것. 갈리면 워커가 만든 파일이 자기 팩에서
    #    `foreign-scope`로 조용히 배제되고, 그 배제는 QUIET 불변식의 면제 대상이라
    #    마지막 방어선조차 통과한다(false QUIET → 세션 주차).

    def test_todo_path_is_bound_at_issue_time(self):
        """todo 경로는 **발부 시점 절대경로**다 — 워커 셸에서 늦게 전개되면 안 된다.

        종전 티켓은 `${CYS_PACK_DIR:-$HOME/.cys/pack}/round/…` 문자열을 실어, 경로는 워커
        셸에서(늦게) · 선언 scope는 master 프로세스에서(즉시) 전개됐다. 두 바인딩이 같다는
        보증이 어디에도 없었다.
        """
        old = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = "/tmp/pack-dept-dept-9"
        try:
            t = self.ticket()
            todo_line = [ln for ln in t.splitlines() if ln.startswith("todo 영속:")]
            self.assertEqual(len(todo_line), 1, "todo 영속 지시가 1줄이 아니다")
            self.assertIn("/tmp/pack-dept-dept-9/round/WORKER_TODO.md", todo_line[0])
            self.assertNotIn("${CYS_PACK_DIR", todo_line[0],
                             "경로가 여전히 워커 셸에서 늦게 전개된다(S14 재발)")
            # 같은 티켓의 선언 scope가 **같은 팩**을 가리킨다(= 같은 바인딩).
            self.assertIn("scope=pack-dept-dept-9", self.decl_line(t))
        finally:
            if old is None:
                os.environ.pop("CYS_PACK_DIR", None)
            else:
                os.environ["CYS_PACK_DIR"] = old

    def test_illegal_pack_identity_fails_loudly_instead_of_folding(self):
        """팩 이름이 G4 문자집합 밖이면 **선언을 배포하지 않는다**(접거나 폴백하지 않는다).

        종전 `decl_value`는 허용 밖 문자를 `-`로 접고 남는 게 없으면 `"pack"`으로 폴백해
        **그럴듯하지만 틀린 정체성**을 배포했다. 같은 상황에서 스탬프 도구는 정반대로
        시끄럽게 실패했다 — 두 생산자가 반대로 행동하는 상태를 여기서 끝낸다.
        """
        old = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = "/tmp/자비스"
        try:
            t = self.ticket()
            self.assertEqual(self.decl_line(t), "",
                             "G4 밖 팩 이름으로 선언을 만들어 배포했다(틀린 정체성)")
            self.assertNotIn("scope=pack ", t, "폴백 `scope=pack`이 살아 있다")
            self.assertIn("생성 실패", t, "실패를 티켓에 알리지 않았다(조용한 실패)")
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
