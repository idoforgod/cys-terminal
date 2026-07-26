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
  6) todo 선언 블록(설계 DESIGN_declared-state.md §4-1) — 티켓이 동봉하는 선언 한 줄의 **문법**을
     핀한다. 문구가 아니라 문법을 보는 이유: 티켓이 문법 위반 선언을 배포하면 소비자 파서가
     전건 '미선언'으로 버려 선언 배선 전체가 조용히 무의미해진다(실패가 티켓 쪽에 안 보인다).

실행: python3 test_orchestra_ticket_snapshot.py
"""
import io
import os
import re
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

    # ── ★B6③ 채널별 의미 단언(스냅샷과 독립 신호) ──
    def test_each_channel_carries_its_own_semantics(self):
        """스냅샷(test_report_channel_snapshot)은 **문구 표류 탐지**용이라, 문구를 고치면
        스냅샷도 같이 고쳐지며 통과한다 — 즉 "무엇이 왜 그 채널에 있어야 하는가"는
        아무도 지키지 않는다. 여기서는 리터럴 복사 대신 **채널별 필수 의미**를 못박아,
        스냅샷을 갱신하면서 규약 자체를 무너뜨리는 개정을 잡는다.
        cmd_self_test(내부 assertion)와도 독립이다 — 저쪽은 티켓 조립 무결성을,
        여기서는 3채널의 의미 분리를 본다."""
        block = channel_block(self.ticket())
        self.assertEqual(len(block), 4, "3분류 항목이 3개가 아니다: %s" % block)
        header, fyi, action, urgent = block

        self.assertIn("3분류", header, "머리글이 3분류 규약임을 밝히지 않는다")

        # ① FYI = 파일 전문 + 포인터 1줄 · master 무회신 · javis_push 경유(직접 send 금지)
        self.assertIn("[FYI]", fyi)
        self.assertIn("javis_push.py", fyi, "FYI 가 javis_push 경유가 아니다")
        self.assertIn("--body-file", fyi, "FYI 에 전문 파일 채널이 없다")
        self.assertIn("회신하지 않는다", fyi, "FYI 의 무회신 계약이 빠졌다")
        self.assertNotIn("[ACTION]", fyi, "FYI 항목에 ACTION 이 섞였다")

        # ② ACTION = --queued 직접 push · ACK 1회(ACK에 대한 ACK 금지)
        self.assertIn("[ACTION]", action)
        self.assertIn("--queued", action, "ACTION 이 --queued 경로가 아니다")
        self.assertIn("ACK", action)
        self.assertIn("ACK에 대한 ACK 금지", action, "ACK 무한왕복 차단 문언이 빠졌다")
        self.assertNotIn("javis_push.py", action, "ACTION 을 FYI 채널로 흘리고 있다")

        # ③ 긴급 = 직접 send + send-key Return · 남용 금지
        self.assertIn("send-key", urgent, "긴급 채널에 직접 send 경로가 없다")
        self.assertIn("Return", urgent)
        self.assertIn("남용 금지", urgent, "긴급 남용 억제 문언이 빠졌다")

        # 채널 간 배타: 각 항목은 서로 다른 배달 수단을 가리켜야 한다(한 채널로 수렴 금지).
        self.assertNotEqual(fyi, action)
        self.assertNotEqual(action, urgent)

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
