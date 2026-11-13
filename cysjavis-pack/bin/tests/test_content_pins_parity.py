#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_content_pins_parity.py — CONTENT_PINS 전 핀 ⊆ repo 배포 디렉티브 (스펙 v4 P0 공통 · W2).

★왜 이 파일이 존재하는가(R1 D3(ii)-1 critical): 2026-08-01 핀 재산정이 preflight 에만
  랜딩되고 대응 디렉티브 개정이 미출하되면 **신선 설치조차 C03 4종 FAIL** — 그런 스큐를
  이 패리티가 CI 상주로 봉인한다(worker 핀 주석의 orchestra RULE_MARKERS 패리티와 동형 발상).

★한계 주석(스펙 P0 공통): 패리티 = **존재 검사**다 — 핀 문구가 디렉티브에 실존하는가만
  단언하고, 조항 간 모순(예: §7 점수 루프 vs REVIEWER verdict 계약)은 검사하지 않는다.

★G1 게이트 주의: 디렉티브 개정 4종(P0 택(a))은 병렬 티켓(W5)이 개정 중이다 — 이 테스트는
  최종 G1 에서 green 이면 된다. 로컬에서 빨간 것은 '어느 핀이 어느 파일에 부재'를 정확히
  보고하는 것이 임무이며, 테스트를 약화시켜 가리는 것은 금지다.

읽기 전용: repo 의 cysjavis-pack/directives/ 만 읽는다(라이브 팩·HOME 무접촉).

    python3 cysjavis-pack/bin/tests/test_content_pins_parity.py
"""
import os
import sys
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402 — 핀의 SOT 는 preflight 모듈(사본 금지)

# repo 임베드 디렉티브(출하 원본) — 라이브 ~/.cys/pack 이 아니라 repo 트리를 대조한다.
DIRECTIVES_DIR = os.path.join(os.path.dirname(BIN), "directives")

# Wave2 핀: D2 합성 서문(gen_ceo_template.py 재합성) 문구 = 오너 절대 규칙 'CEO·마스터는 직접
# 구현하지 않는다' 의 주입 문면. ★착지 완료(18dc9a6b · 2026-08-16) — 종전의 '부재 시 skip(착지 대기)'
# 분기는 착지 뒤에는 해가 됐다: 이 문장이 편집·재합성으로 사라지면 실패가 아니라 skip → 3레인
# 전부 초록(U4 C4-④ · 2026-09-23 제거). 이제 다른 CEO 핀과 똑같이 **무조건 단언**이다.
CEO_WAVE2_PIN = "직접 구현은 §1-A 사소 예외 없이 금지"


class ContentPinsParity(unittest.TestCase):
    maxDiff = None

    def _text(self, fname):
        p = os.path.join(DIRECTIVES_DIR, fname)
        self.assertTrue(os.path.isfile(p), "repo 디렉티브 부재: %s" % p)
        with open(p, encoding="utf-8") as f:   # encoding 명시(계약 — 로케일 비의존)
            return f.read()

    def _assert_pins(self, fname):
        """부재 핀을 **전수 목록**으로 보고한다(1개씩 죽는 단언 금지 — 정확 보고 임무).
        ★면제(exclude) 인자는 없앴다 — 면제 경로가 있으면 '착지 대기' 같은 한시 사유가 영구 사각이 된다."""
        text = self._text(fname)
        missing = [(pin, label) for pin, label in pf.CONTENT_PINS[fname] if pin not in text]
        self.assertFalse(
            missing,
            "핀↔디렉티브 패리티 붕괴 — %s 에 부재 %d건:\n%s"
            % (fname, len(missing),
               "\n".join("  · %r (%s)" % (pin, label) for pin, label in missing)))

    def test_master_pins_shipped(self):
        self._assert_pins("MASTER_DIRECTIVE.md")

    def test_worker_pins_shipped(self):
        self._assert_pins("WORKER_DIRECTIVE.md")

    def test_cso_pins_shipped(self):
        self._assert_pins("CSO_DIRECTIVE.md")

    def test_reviewer_pins_shipped(self):
        self._assert_pins("REVIEWER_DIRECTIVE.md")

    def test_ceo_pins_shipped(self):
        """CEO 템플릿 핀 전수(표지 3핀 + Wave2 서문 핀) — 면제 없음."""
        self._assert_pins("CEO_TEMPLATE.md")

    def test_ceo_wave2_preface_pin(self):
        """Wave2 합성 서문 핀 — **무조건 상주 단언**(skip 분기 없음 · U4 C4-④).
        CONTENT_PINS 등재까지 함께 단언한다: 등재에서 빠지면 위 전수 검사가 이 핀을 보지 않는다."""
        self.assertIn(CEO_WAVE2_PIN, [pin for pin, _ in pf.CONTENT_PINS["CEO_TEMPLATE.md"]],
                      "Wave2 핀이 preflight CONTENT_PINS['CEO_TEMPLATE.md'] 에서 빠졌다 — "
                      "런타임 C03.pin.ceo 와 이 패리티가 그 문면을 더 이상 보지 않는다")
        self.assertIn(CEO_WAVE2_PIN, self._text("CEO_TEMPLATE.md"),
                      "CEO_TEMPLATE.md 에 Wave2 서문 핀 %r 부재 — 오너 절대 규칙(CEO·마스터 직접 구현 "
                      "금지)의 주입 문면이 사라졌다. gen_ceo_template.py 재합성 결과를 확인하라"
                      % CEO_WAVE2_PIN)

    def test_marker_pins_exist_in_live_template(self):
        """표지 핀 불변식의 신판 축: MARKER_PINS 는 repo(신) 템플릿에 전수 실존해야 한다.
        (구판 실존 축은 gen_ceo_template.py --check 가 단언 — 스펙 §D2. 여기서 구판 템플릿
        바이트를 갖고 있지 않으므로 신판 축만 CI 상주.)"""
        text = self._text("CEO_TEMPLATE.md")
        missing = [p for p in pf.MARKER_PINS if p not in text]
        self.assertFalse(missing, "표지 핀이 신 템플릿에 부재(불변식 붕괴): %r" % missing)

    def test_marker_pins_subset_of_ceo_pins(self):
        """표지 핀 ⊆ CEO 파일 핀 — 표지 술어가 파일 검사와 어긋난 채 표류하는 것 차단."""
        ceo_pins = [pin for pin, _ in pf.CONTENT_PINS["CEO_TEMPLATE.md"]]
        for m in pf.MARKER_PINS:
            self.assertIn(m, ceo_pins,
                          "표지 핀 %r 이 CONTENT_PINS['CEO_TEMPLATE.md'] 에 없음" % m)

    def test_wave2_pin_not_in_marker_pins(self):
        """Wave2 핀은 표지 술어 편입 금지(R3 A7 — 구 템플릿 부재라 구판 표지가 사멸)."""
        self.assertNotIn(CEO_WAVE2_PIN, pf.MARKER_PINS)


class Wave2PinRemovalMutation(unittest.TestCase):
    """★뮤테이션(U4 C4-④ · 2026-09-23): **임시 사본**의 CEO_TEMPLATE.md 에서 Wave2 핀
    (오너 절대 규칙 'CEO·마스터는 직접 구현하지 않는다' 의 주입 문면)을 지우면 CEO 핀 검사가
    **실패**해야 한다 — skip 이면 3레인 전부 초록으로 그 문면 소실을 통과시킨다.

    대상 검사 둘을 unittest 로 실제 실행해 결과 종류(failure/skip)를 잰다. 리포 트리·라이브 팩
    무접촉 — 임시 디렉터리 사본만 변조하고 모듈 전역 DIRECTIVES_DIR 은 finally 에서 되돌린다."""

    def test_pin_removal_in_temp_copy_is_failure_not_skip(self):
        import shutil
        import tempfile
        mod = sys.modules[ContentPinsParity.__module__]
        src = os.path.join(mod.DIRECTIVES_DIR, "CEO_TEMPLATE.md")
        with open(src, encoding="utf-8") as f:
            text = f.read()
        self.assertIn(CEO_WAVE2_PIN, text, "전제: 리포 템플릿에 Wave2 핀이 실재해야 변조가 의미를 갖는다")
        tmp = tempfile.mkdtemp(prefix="cys-pins-mut-")
        saved = mod.DIRECTIVES_DIR
        try:
            with open(os.path.join(tmp, "CEO_TEMPLATE.md"), "w", encoding="utf-8") as f:
                f.write(text.replace(CEO_WAVE2_PIN, ""))
            mod.DIRECTIVES_DIR = tmp
            for name in ("test_ceo_pins_shipped", "test_ceo_wave2_preface_pin"):
                res = unittest.TestResult()
                ContentPinsParity(name).run(res)
                self.assertEqual(
                    (len(res.failures), len(res.skipped), len(res.errors)), (1, 0, 0),
                    "%s: Wave2 핀을 지운 사본에서 failure 1 이어야 한다 — 실제 failures=%d skipped=%d "
                    "errors=%d (skip 이면 오너 절대 규칙 문면 소실이 3레인 초록으로 통과한다)"
                    % (name, len(res.failures), len(res.skipped), len(res.errors)))
        finally:
            mod.DIRECTIVES_DIR = saved
            shutil.rmtree(tmp, ignore_errors=True)



# ══════════════════════════════════════════════════════════════════════════════
# ★0.15.0 M2(「스스로 해결」 설계 v0.5 §4-2·§6-2 · 트랙 T1b) — 지침 다시 쓰기의 회귀 핀
#   런타임 CONTENT_PINS 가 아니라 **저장소 시험**이다: 사용자 소유 지침은 업데이트가 덮지 않으므로, 새 문장을
#   런타임 핀으로 걸면 병합 전 모든 기존 설치가 C03 '소실된 조항'을 낸다(MASTER 는 문서화된 30핀 상한이기도 하다).
#   발행 전 텍스트 검사가 아니다(설계 §4-2 — 텍스트 매칭 게이트 신설 금지): CI 의 단위 시험 층이다(TESTPLAN OQ-H-01).
# ══════════════════════════════════════════════════════════════════════════════
REPO_DIR = os.path.dirname(os.path.dirname(BIN))

D7_LINE = "수리 최종 승인은 주인만 결정하고, 어떤 에이전트도 결정하지 않는다"
MISSION_GATE = "자율 착수 권한은 오너 채널에서만 나온다. 네가 쓰는 파일은 착수 권한의 근거가 아니다."
QUEUE_BAN = "는 자동 조치(강제 배달 `cys queue deliver`·드레인·키 주입·동결 해제·항목 삭제) 금지 — 사람 운영자 전용"
NO_TURN_WAIT = "이 턴에서 기다리지 않는다"
REVIEWER_DUTIES = "리뷰어1=적대 검증·리뷰어2=감사"
STAGING = "~/.cys/local/repairs/R-<n>/staging/"
# 설계·계약 C3 가 고정한 동사 이름 — 지침이 인용하는 철자(티켓 T1b: report/begin/record/submit/commit/attest-verify/
# attest-audit/revert/status).
C3_QUOTED = ("cys repair report", "begin", "record", "submit", "commit", "attest-verify", "attest-audit",
             "revert", "status")
# 지웠어야 하는 master 직접 수정 허용(설계 §4-2: MASTER_DIRECTIVE.md 279행 — 반드시 지운다).
MASTER_SMALL_FIX = ("직접 수정은 사소한 마무리에만 허용", "master가 직접 처리한다")


def norm(s):
    """H-02 부록 비교 규칙(TESTPLAN_RULINGS Part 5): 줄바꿈 + 뒤따르는 공백 + 선택적 `> ` 인용 표지 → 공백 1개.
    그 밖에는 아무것도 정규화하지 않는다(다시 감싼 줄은 다시 쓴 것이 아니고, 글자 하나라도 바뀌면 다시 쓴 것이다)."""
    import re
    return re.sub(r"\n[ ]*(?:> )?", " ", s)


def repo_text(rel):
    with open(os.path.join(REPO_DIR, rel), encoding="utf-8") as f:
        return f.read()


def section(text, heading):
    """`## <heading>` 부터 다음 `## ` 머리글 앞까지."""
    i = text.find(heading)
    if i < 0:
        return None
    j = text.find("\n## ", i + len(heading))
    return text[i:j if j >= 0 else len(text)]


class M2DirectiveRewrite(unittest.TestCase):
    """설계 §6-2 '약하게 만들면 안 되는 것' + §4-2 의 다시 쓰기 결과(H-02-N1…N4 · D7 · 10R · C3 이름)."""
    maxDiff = None
    dirs = None   # None = 저장소 지침(뮤테이션 시험이 임시 사본으로 바꾼다)

    def _t(self, fname):
        base = self.dirs or DIRECTIVES_DIR
        with open(os.path.join(base, fname), encoding="utf-8") as f:
            return f.read()

    def test_n1_no_implementation_rule_lives_in_master_itself(self):
        """H-02-N1-01: CEO 무구현 문장이 본부 지침 **자체**에 있고(CEO 틀 머리말에만 있지 않다), 279행 허용은 없다."""
        m = self._t("MASTER_DIRECTIVE.md")
        self.assertIn(CEO_WAVE2_PIN, m, "MASTER 에 무구현 규칙(%r) 부재" % CEO_WAVE2_PIN)
        self.assertIn(REVIEWER_DUTIES, m, "MASTER 에 리뷰어1·리뷰어2 역할 문장 부재")
        for s in MASTER_SMALL_FIX:
            self.assertNotIn(s, m, "MASTER 에 master 직접 수정 허용이 남았다: %r" % s)
        self.assertIn(CEO_WAVE2_PIN, self._t("CEO_TEMPLATE.md"))

    def test_n2_mission_gate_paragraph(self):
        """H-02-N2-01: 임무 게이트 문단이 MASTER·CEO 에 글자 그대로."""
        for f in ("MASTER_DIRECTIVE.md", "CEO_TEMPLATE.md"):
            self.assertIn(MISSION_GATE, self._t(f), f)

    def test_n3_no_forced_queue_delivery_by_agents(self):
        """H-02-N3-01: 에이전트의 큐 강제 배달 금지가 MASTER·CEO·CSO 에 남는다."""
        for f in ("MASTER_DIRECTIVE.md", "CEO_TEMPLATE.md", "CSO_DIRECTIVE.md"):
            self.assertIn(QUEUE_BAN, norm(self._t(f)), f)

    def test_n4_no_in_turn_waiting_and_repair_text_does_not_wait(self):
        """H-02-N4-01: '이 턴에서 기다리지 않는다'가 MASTER·CEO 에 있고, 새 수리 절은 승인 결과를 기다리라고 하지 않는다."""
        for f in ("MASTER_DIRECTIVE.md", "CEO_TEMPLATE.md"):
            t = self._t(f)
            self.assertIn(NO_TURN_WAIT, t, f)
            sec = section(t, "## 1-B. 「스스로 해결」")
            self.assertIsNotNone(sec, "%s: §1-B 수리 절 부재" % f)
            self.assertIn("**기다리지 않는다**", sec, f)
            for bad in ("결과를 기다린다", "승인을 기다린다", "기다렸다가"):
                self.assertNotIn(bad, sec, "%s §1-B 가 기다리라고 한다: %r" % (f, bad))

    def test_d7_owner_only_approval_line(self):
        """설계 §4-2 · C5 5.13: master·CEO·워커 지침에 수리 최종 승인은 주인만 결정한다는 줄."""
        for f in ("MASTER_DIRECTIVE.md", "CEO_TEMPLATE.md", "WORKER_DIRECTIVE.md"):
            self.assertIn(D7_LINE, norm(self._t(f)), f)

    def test_worker_submit_not_wait_and_hourly_verb_and_staging(self):
        """워커: 수리 승인에 feed --wait 금지·submit 만(설계 §4-2) · 한 시간마다 장부 변경 명령(TRACKS v6) · 고정 작업 사본(U3)."""
        w = norm(self._t("WORKER_DIRECTIVE.md"))
        self.assertIn("`cys feed push --wait` 를 쓰지 않고 `cys repair submit <R-id>` 만 쓴다", w)
        self.assertIn("한 시간에 한 번 이상", w)
        self.assertIn(STAGING, w)
        self.assertIn("접수는 장부의 자료이지 master 에게 보내는 메시지가 아니다", w)
        # RULINGS AH1: the taught call is accepted by the daemon (--trigger required, AD10) and begin precedes the edit.
        self.assertIn("고치기 **전에** `cys repair begin --files <rel…> --trigger owner-order|grant|update`", w)
        self.assertIn("`--owner-token <표>`", w)
        # RULINGS AH2: `record` renews the lease only while editing — after asking for signatures it would void them.
        self.assertIn("**리뷰어 서명을 청한 뒤에는 `record` 를 부르지 않는다**", w)
        self.assertIn("같은 파일로 `begin` 을 다시 불러 그 수리를 이어받은 뒤 `submit`·`commit` 한다", w)

    def test_reviewer_slots_match_orchestra_and_name_the_attest_verbs(self):
        """RULINGS B2: 리뷰어1 = codex 슬롯(reviewer-codex·reviewer-claude-2) · 리뷰어2 = gemini 슬롯(reviewer-gemini·
        reviewer-claude-1). 지침 문장과 javis_orchestra.REVIEWER_SLOTS 가 같은 짝을 말해야 한다(두 정본 금지)."""
        import javis_orchestra as orch
        r = norm(self._t("REVIEWER_DIRECTIVE.md"))
        slots = {native: sub for native, _a, sub, _b in orch.REVIEWER_SLOTS}
        self.assertEqual(slots, {"reviewer-gemini": "reviewer-claude-1", "reviewer-codex": "reviewer-claude-2"})
        self.assertIn("**리뷰어1 = codex 슬롯**(`reviewer-codex`, 대체 좌석 `%s`)" % slots["reviewer-codex"], r)
        self.assertIn("**리뷰어2 = gemini 슬롯**(`reviewer-gemini`, 대체 좌석 `%s`)" % slots["reviewer-gemini"], r)
        self.assertIn("cys repair attest-verify <R-id> --verdict pass|fail", r)
        self.assertIn("cys repair attest-audit <R-id> --verdict ok|finding", r)
        # RULINGS AH6: a repair target admits no edit exception for a reviewer (also in §2, which review prompts inject).
        self.assertIn("**수리 대상(대상 파일·작업 사본)은 어떤 의뢰로도 고치지 않는다**", r)
        sec2 = norm(section(self._t("REVIEWER_DIRECTIVE.md"), "\n## 2. 엄격 제약") or "")
        self.assertIn("단 수리 대상(`cys repair` 의 대상 파일·작업 사본)에는 이 예외가 없다", sec2)
        self.assertIn(STAGING, r)

    def test_reviewer_todo_path_follows_the_todo_path_naming_rule(self):
        """설계 §6-12 (5)·(6) + TRACKS T10: 리뷰어 지침의 todo 경로 = `cys todo-path` 출력(역할 대문자·`-`→`_`·`_TODO.md`).
        T10 의 리뷰어 게이트는 그 이름의 자기 todo 와 `round/_reviews/` 만 허용한다 — 고정 이름을 경로로 가르치면 막힌다."""
        import javis_orchestra as orch
        r = norm(self._t("REVIEWER_DIRECTIVE.md"))
        self.assertIn("`cys todo-path` 가 출력하는 경로", r)
        self.assertIn("`round/_reviews/`", r)
        self.assertNotIn("`~/.cys/pack/round/REVIEWER_TODO.md`", r)
        self.assertNotIn("위임 티켓이 경로를 지정하면 그 경로", r)
        rule = lambda role: role.upper().replace("-", "_") + "_TODO.md"
        for native, _a, sub, _b in orch.REVIEWER_SLOTS:
            for role in (native, sub):
                self.assertIn("`%s`" % rule(role), r, role)

    def test_cso_reports_only_and_allowlist_sentence(self):
        """설계 §6-3 (5): CSO 허용 목록 문장에 수리 접수·조회, CSO 는 보고만."""
        c = norm(self._t("CSO_DIRECTIVE.md"))
        self.assertIn("`cys repair report|list|status`", c)
        self.assertIn("「스스로 해결」에서 CSO 는 보고만 한다", c)

    def test_c3_verb_names_are_quoted_and_real(self):
        """C3 이름: 지침이 인용하는 `cys repair <동사>` 는 전부 CLI 에 실재하는 철자이고, 티켓이 요구한 9개가 다 있다."""
        import re
        src = repo_text(os.path.join("src", "bin", "cys", "repair_cli.rs"))
        enum = src[src.index("pub(crate) enum RepairCmd"):]
        enum = enum[:enum.index("\n}\n")]
        variants = re.findall(r"(?m)^    ([A-Z][A-Za-z]+) ?[{,(]", enum)
        cli = {re.sub(r"(?<!^)([A-Z])", r"-\1", v).lower() for v in variants}
        self.assertTrue({"report", "begin", "record", "submit", "commit", "attest-verify", "attest-audit",
                         "revert", "status", "list"} <= cli, sorted(cli))
        joined = ""
        for f in ("MASTER_DIRECTIVE.md", "WORKER_DIRECTIVE.md", "REVIEWER_DIRECTIVE.md", "CSO_DIRECTIVE.md"):
            joined += norm(self._t(f)) + "\n"
        quoted = set(re.findall(r"cys repair ([a-z][a-z-]*)", joined))
        self.assertFalse(quoted - cli, "CLI 에 없는 철자 인용: %r" % sorted(quoted - cli))
        for v in C3_QUOTED:
            needle = v if v.startswith("cys repair") else "`%s`" % v
            self.assertIn(needle, joined, "C3 동사 %r 미인용" % v)

    def test_review_round_cap_is_ten(self):
        """설계 §4-2·§4-6: 10라운드 = 수리 반복 상한 = 오케스트레이션 도구 상한. '3R 상한' 문구는 남지 않는다."""
        import javis_orchestra as orch
        self.assertEqual(orch.MAX_ROUNDS, 10)
        for f in ("MASTER_DIRECTIVE.md", "WORKER_DIRECTIVE.md", "REVIEWER_DIRECTIVE.md", "CEO_TEMPLATE.md"):
            t = self._t(f)
            self.assertNotIn("3R 상한", t, f)
            self.assertNotIn("**3R**", t, f)
            self.assertNotIn("**3라운드**", t, f)
            self.assertIn("10R 상한", t, f)

    def test_ah6_master_boot_line_round_tool_and_skill_index(self):
        """RULINGS AH6: the master repairs nothing at boot (a remaining FAIL → `cys repair report` + a worker ticket);
        the round tool's 10-round line hands over the fact instead of asking whether to continue (MASTER §7);
        the author's name stays only as an authorship credit in the skill body, not in the index line that is
        composed into every master / CEO payload (design D:581)."""
        m = norm(self._t("MASTER_DIRECTIVE.md"))
        self.assertIn("FAIL이 남아도 master 는 직접 고치지 않는다 — 남은 FAIL 은 ①(데몬 확인) 뒤에 "
                      "`cys repair report --file <팩 기준 상대 경로> --error \"<FAIL 줄>\"` 로 접수하고", m)
        # RULINGS AM4: the report call is the form track T3's clap accepts — `--file` and `--error` are both required
        # there (`Report { file: String, error: String }`), so a FAIL that names no file is NOT filed with a missing
        # argument (the parser would refuse it); it goes onto the worker ticket as it is.
        self.assertIn("(파일을 가리키지 않는 FAIL 은 접수 없이 그 줄 그대로)", m)
        self.assertNotIn("출력의 지시대로 수리 후 재실행", m)
        orc = repo_text(os.path.join("cysjavis-pack", "bin", "javis_orchestra.py"))
        self.assertNotIn("추가 라운드 여부를 여쭈어라", orc)
        self.assertIn("추가 라운드 여부를 묻지 않는다", orc)
        sk = repo_text(os.path.join("cysjavis-pack", "skills", "vibecoding-knowledge", "SKILL.md"))
        head = sk.splitlines()[:10]
        desc = next(l for l in head if l.startswith("description:"))
        self.assertNotIn("최윤식", desc)
        self.assertIn("최윤식 박사 교본", sk, "저작 표시(본문)는 남는다")

    def test_files_are_lf_and_strict_utf8(self):
        """부록 A 3절 9번(CRLF): 다시 쓴 지침·틀은 LF·엄격 UTF-8."""
        for f in ("MASTER_DIRECTIVE.md", "WORKER_DIRECTIVE.md", "REVIEWER_DIRECTIVE.md", "CSO_DIRECTIVE.md",
                  "CEO_TEMPLATE.md", "VIBECODING_ENFORCEMENT.md", "ROUTE_CONTRACT.md"):
            base = self.dirs or DIRECTIVES_DIR
            with open(os.path.join(base, f), "rb") as fh:
                raw = fh.read()
            self.assertNotIn(b"\r", raw, f)
            raw.decode("utf-8")


# H-02 부록 — '멈추고 보고' 여섯 계열의 동결 목록(impl/testplan/H.md 'H-02 annex — frozen pin list', TESTPLAN_RULINGS
# Part 9 R6-6 · Part 11 R7-4: 태그 v0.14.56 에서 시험 계획 워커가 도출 · 90개). T1 은 핀을 **더할** 뿐 문장을 지우거나
# 다시 쓰지 않는다 — 다시 쓰려면 부록의 동결 개정(두 리뷰어 확인 + master 기록)이 먼저다(Part 10).
# ★동결 개정 반영(RULINGS AH5 · TESTPLAN_RULINGS Part 15 A7): P02·P14 "주인님 보고 후" → "오너에게 보고한 뒤" · P53 산문
#   `doctor` → `owner(사용자)` · P74·P82 개인 기억 파일 포인터 제거 · P11·P23 괄호 "(팩 업데이트가 강제 복원)" 제거.
STOP_REPORT_ANNEX = (
    ("P01", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "충돌 시 우선순위: 오너 명시 지시 > soul.md > 이 지침 > 개별 작업 브리프."),
    ("P02", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "★단 디렉티브·soul 소실 FAIL의 복구에서 **팩 템플릿 강제 복원은 절대 금지**다(계약 §1-1·§11-13) —\n   백업 없는 템플릿 덮어쓰기는 주권 편집을 비가역 소실시킨다. 복원은 preflight 출력이 안내하는\n   절차(백업 선행 → 오너에게 보고한 뒤 지시에 따라 복구)만 따른다."),
    ("P03", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "(금지선 작업은 애초에 feed 승인 요청 대상이 아니라 워커가 무조건\n  중단·보고할 의무다 — WORKER §7. 금지선이 의심되는 요청만 예외적으로 오너에게 격상한다.)"),
    ("P04", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "단 soul.md의 금지선(외부 발행·비가역 삭제 등)은 오너에게 보고하고 오너 결정을 받는다."),
    ("P05", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "금지선·검증 동요 같은 특수 상황만이 아니라\n  모든 비자명한 지시에 적용된다."),
    ("P06", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "**게이트**: 충돌 시 상위 기준 절대 우선. ②(b 검증)가 흔들리면 ①③(그 위에 쌓는 나머지\n  실행)을 중단하고 오너에게 보고한다 — 토대 오염 위에 쌓지 않는다. 워커가 검증 동요로\n  중단·보고해 오면 master는 이를 **오너에게 중계**한다(보고 종단은 항상 오너)."),
    ("P07", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "③진행 중 작업 로컬 커밋(push 금지 — denylist)"),
    ("P08", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "부여된 경우의 계약: **denylist에서만\n멈추고 나머지는 무정지로 달린다** — \"로드맵 안·가역이면 달리고, 로드맵 밖·비가역이면 주차한다.\""),
    ("P09", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "**정지 경계 (denylist — 이것만 멈춰서 오너 승인)**: ①승인된 로드맵(BUILD_PLAN/Phase)을\n  벗어나는 새 범위 ②soul·CLAUDE.md·헌법(디렉티브) 변경 ③외부 발행/발송(git push·메시지\n  전송·공개 배포 = 비가역. **로컬 커밋은 가역 = 허용**) ④비가역 삭제 ⑤오너가 명시 보유한\n  결정권."),
    ("P10", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "(Phase 중간에는 §4 비자명 지시 보고·금지선 격상 외 다변 보고를 억제한다 — 무정지 원칙)"),
    ("P11", "cysjavis-pack/directives/CEO_TEMPLATE.md", "템플릿은 **시스템 소유** 이고 살아 있는 규약 파일은 **사용자 소유**인데,\n  **승격은 사용자 파일을 템플릿으로 덮는다** ⇒ 살아 있는 파일에만 있는 조항은 **다음 부서 생성이\n  파괴한다.**"),
    ("P12", "cysjavis-pack/directives/CEO_TEMPLATE.md", "충돌 시 머리글>이 서문>본문 순 우선(전역: 오너>soul.md>이 지침>브리프)."),
    ("P13", "cysjavis-pack/directives/CEO_TEMPLATE.md", "충돌 시 우선순위: 오너 명시 지시 > soul.md > 이 지침 > 개별 작업 브리프."),
    ("P14", "cysjavis-pack/directives/CEO_TEMPLATE.md", "★단 디렉티브·soul 소실 FAIL의 복구에서 **팩 템플릿 강제 복원은 절대 금지**다(계약 §1-1·§11-13) —\n   백업 없는 템플릿 덮어쓰기는 주권 편집을 비가역 소실시킨다. 복원은 preflight 출력이 안내하는\n   절차(백업 선행 → 오너에게 보고한 뒤 지시에 따라 복구)만 따른다."),
    ("P15", "cysjavis-pack/directives/CEO_TEMPLATE.md", "(금지선 작업은 애초에 feed 승인 요청 대상이 아니라 워커가 무조건\n  중단·보고할 의무다 — WORKER §7. 금지선이 의심되는 요청만 예외적으로 오너에게 격상한다.)"),
    ("P16", "cysjavis-pack/directives/CEO_TEMPLATE.md", "단 soul.md의 금지선(외부 발행·비가역 삭제 등)은 오너에게 보고하고 오너 결정을 받는다."),
    ("P17", "cysjavis-pack/directives/CEO_TEMPLATE.md", "금지선·검증 동요 같은 특수 상황만이 아니라\n  모든 비자명한 지시에 적용된다."),
    ("P18", "cysjavis-pack/directives/CEO_TEMPLATE.md", "**게이트**: 충돌 시 상위 기준 절대 우선. ②(b 검증)가 흔들리면 ①③(그 위에 쌓는 나머지\n  실행)을 중단하고 오너에게 보고한다 — 토대 오염 위에 쌓지 않는다. 워커가 검증 동요로\n  중단·보고해 오면 master는 이를 **오너에게 중계**한다(보고 종단은 항상 오너)."),
    ("P19", "cysjavis-pack/directives/CEO_TEMPLATE.md", "③진행 중 작업 로컬 커밋(push 금지 — denylist)"),
    ("P20", "cysjavis-pack/directives/CEO_TEMPLATE.md", "부여된 경우의 계약: **denylist에서만\n멈추고 나머지는 무정지로 달린다** — \"로드맵 안·가역이면 달리고, 로드맵 밖·비가역이면 주차한다.\""),
    ("P21", "cysjavis-pack/directives/CEO_TEMPLATE.md", "**정지 경계 (denylist — 이것만 멈춰서 오너 승인)**: ①승인된 로드맵(BUILD_PLAN/Phase)을\n  벗어나는 새 범위 ②soul·CLAUDE.md·헌법(디렉티브) 변경 ③외부 발행/발송(git push·메시지\n  전송·공개 배포 = 비가역. **로컬 커밋은 가역 = 허용**) ④비가역 삭제 ⑤오너가 명시 보유한\n  결정권."),
    ("P22", "cysjavis-pack/directives/CEO_TEMPLATE.md", "(Phase 중간에는 §4 비자명 지시 보고·금지선 격상 외 다변 보고를 억제한다 — 무정지 원칙)"),
    ("P23", "scripts/ceo_template_header.md", "템플릿은 **시스템 소유** 이고 살아 있는 규약 파일은 **사용자 소유**인데,\n  **승격은 사용자 파일을 템플릿으로 덮는다** ⇒ 살아 있는 파일에만 있는 조항은 **다음 부서 생성이\n  파괴한다.**"),
    ("P24", "scripts/ceo_template_header.md", "충돌 시 머리글>이 서문>본문 순 우선(전역: 오너>soul.md>이 지침>브리프)."),
    ("P25", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "충돌 시: 오너 명시 지시 > master 지시 > 이 지침 > 개별 작업 브리프."),
    ("P26", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "정의처 고지: 이 문서는 헌장·운영계약 조문의 현장 배치본이다 — 문서 간 규정이 갈리면 정본이 이긴다\n> (**충돌 시 헌장 > 운영계약 > 이 디렉티브**)."),
    ("P27", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "**게이트**: 충돌 시 상위 기준 절대 우선. ②(b 할루시네이션 방지·검증)가 흔들리면 ①③(그 위에 쌓는 나머지 실행)을 중단하고 master에 보고한다 — 토대 오염 위에 쌓지 마라."),
    ("P28", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "오너 금지선(외부 발행·비가역 삭제)은 승인 요청 대상이 아니라 **무조건 중단·보고** 대상이다."),
    ("P29", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "**정지 경계(다섯 정지의 상주 배치 · 워커 헌장 제0조)**: 위 금지선 문장의 전수 목록이다 —\n  ①승인된 로드맵 밖 새 범위 ②soul·CLAUDE.md·디렉티브(헌법) 변경 ③외부 발행/발송(git push·\n  메시지 전송·공개 배포) ④비가역 삭제 ⑤오너가 명시 보유한 결정권. 이 다섯은 feed 승인 요청\n  대상이 아니라 **무조건 중단·master 보고** 대상이며, 작업 내내 상주 적용된다."),
    ("P30", "cysjavis-pack/directives/REVIEWER_DIRECTIVE.md", "정의처 고지: 이 문서는 헌장·운영계약 조문의 현장 배치본이다 — 문서 간 규정이 갈리면 정본이 이긴다\n> (**충돌 시 헌장 > 운영계약 > 이 디렉티브**)."),
    ("P31", "cysjavis-pack/directives/REVIEWER_DIRECTIVE.md", "**정지 경계(다섯 정지의 상주 배치 · 리뷰어 헌장 제0조)**: 로드맵 밖 새 범위 · soul/CLAUDE.md/\n  디렉티브(헌법) 변경 · 외부 발행/발송 · 비가역 삭제 · 오너 명시 보유 결정권 — 이 다섯은\n  리뷰어에게도 상주 적용된다. 해당 지점에 닿으면 멈추고 의뢰자·master에 보고한다."),
    ("P32", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "정의처 고지: 이 문서는 헌장·운영계약 조문의 현장 배치본이다 — 문서 간 규정이 갈리면 정본이 이긴다\n> (**충돌 시 헌장 > 운영계약 > 이 디렉티브**)."),
    ("P33", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "여는 유일한 경로는 권한자(오너·master)의 **명시적 정책 변경과 그 적용 상태\n  확인**이며, 그 변경은 정지 경계 ②(디렉티브 변경)라 네 자기인가로 하지 않는다 — 그때까지 증거는\n  sha256 + 텍스트 요약 1줄이다."),
    ("P34", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**이 경계가 푸는 것은 ④의 보류 중 §2 사이클 집행 1콜뿐이다** — AUTOPILOT_PAUSED·오너 실시간 입력·\n  master self-clear 금지·게이트 deny·§5-1 오살 금지는 그대로 적용된다."),
    ("P35", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "이 조항은 **이미 있는 범위의 확인**이지 자기인가의 창설이 아니다 — 넓히는\n  것은 오너·master 의 정책 변경뿐이고(정지 경계 ②), '상설 승인' 을 다른 행동에 확대 적용하지 마라."),
    ("P36", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "누적이 문제가 되면 **보고하고 멈춘다**(조치는 오너의 것이다 · §5 금지선과 같은 결)."),
    ("P37", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "## 5. 금지선"),
    ("P38", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "오너 soul.md의 denylist는 너에게도 적용된다. 시스템 정리를 이유로 사용자 데이터·작업 산출물을\n삭제하지 않는다. 의심스러우면 격리(프로세스 정지)하고 master에 묻는다."),
    ("P39", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "### 5-1. 정지 경계 + 오살 금지 + 일시정지 중 허용 범위 (CSO 헌장 제0조·제5조)"),
    ("P40", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**정지 경계(다섯 정지의 상주 배치)**: ①승인된 로드맵 밖 새 범위 ②soul·CLAUDE.md·디렉티브\n  (헌법) 변경 ③외부 발행/발송 ④비가역 삭제 ⑤오너가 명시 보유한 결정권 — 이 다섯은 자율 조치\n  대상이 아니라 멈추고 승인(오너/master)을 받는 항이며, 시스템 위기라는 이유로도 넘지 않는다."),
    ("P41", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**오살 금지(회생 가능 노드 종료 금지 원칙 · CSO 헌장 제5조)**: denylist에는\n  '**살아있는 타 노드·세션의 종료**'가 포함된다 — 판정은 명령 이름이 아니라 **효과 기반**이다\n  (kill·close-surface·reap 어느 도구든 효과가 산 노드의 종료면 같은 항이다). 자동 회수는 `exited=true`(죽은 pane)만\n  대상이라는 아래 [절대규칙]과 한 몸의 원칙이며, 판정 불능이면 산 노드로 보류한다."),
    ("P42", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**생명 유지 목록(AUTOPILOT_PAUSED 일시정지 중에도 계속하는 것)**: ⓐ관측·inbox 수신 유지(구독\n  아님 · 조치 없이 기록만) ⓑSESSION_STATE·CSO_TODO 영속 ⓒ오너·master 채널 응답(보고·질문 답변) ⓓ데이터 손실이\n  임박한 경우의 방어적 격리(프로세스 정지 — 조치 후 즉시 보고)."),
    ("P43", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**일시정지 중 허용 범위 = 바로 위 생명 유지 목록** 그 자체다(계약 §9-4·v0.4 집합 통일) —\n  목록 밖 행동(노드 기동·재기동·clear 집행·정리·소거)은 전부 보류하고, 정지 중이라도 살아있는\n  타 노드 종료 금지는 그대로다."),
    ("P44", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "사용자 데이터·작업 산출물은 삭제하지 않는다(§5 금지선 불변)."),
    ("P45", "cysjavis-pack/directives/RADIO_CONTRACT.md", "pause 중에는 스레드 기록(append)만 계속되고 stdin 직배달은 정지한다."),
    ("P46", "cysjavis-pack/directives/RADIO_CONTRACT.md", "pause 중 허용 범위는 관측·저장·보고·자기 프로세스 종료 넷뿐이다(운영계약 v0.4)."),
    ("P47", "cysjavis-pack/directives/RADIO_CONTRACT.md", "살아있는 타 노드의 종료·재기동은 pause 중 전면 금지 — watcher 재기동도 예외가 아니다."),
    ("P48", "cysjavis-pack/directives/RADIO_CONTRACT.md", "**pause 중에는 `close`·`fence`·`unfence`·`resume-release`가 도구 차원에서 exit 4로 거부**\n  된다. close는 타 노드 watcher를 강제 종료시키고, fence/unfence/resume-release는 격리\n  상태를 변형·방류하므로 pause 허용 범위 밖이다. resume 후 재시도하라."),
    ("P49", "cysjavis-pack/directives/RADIO_CONTRACT.md", "radio 개통 티켓의 워커는 **툴콜 20회 또는 30분 중 먼저 오는 주기**로 로컬 커밋 또는\n  스냅샷을 남긴다(가역 · denylist 비저촉)."),
    ("P50", "cysjavis-pack/directives/ROUTE_CONTRACT.md", "계약 개정은 정지 경계\n> (soul/CLAUDE 변경 층위) — 신호 정의·enum 신설은 문서 개정 + 토큰 게이트를 거친다."),
    ("P51", "cysjavis-pack/directives/RSI_LEARNING_DIRECTIVE.md", "**복구수단 불변**: 루프가 evaluator/rollback/denylist/**복구수단(.git·refs/rsi·롤백스크립트)** 수정 시 `rsi-gate.sh` DENY."),
    ("P52", "cysjavis-pack/directives/RSI_LEARNING_DIRECTIVE.md", "## 4. 경계 (denylist)\n\n- 자율추천은 '추천'까지만 자율. **④저장·⑤채택은 사람 승인**.\n- soul·CLAUDE·directive·헌법 변경은 사람 승인(자동 저장 금지).\n- budget cap 초과·자원/load 한계 시 중단·보고."),
    ("P53", "cysjavis-pack/directives/VIBECODING_ENFORCEMENT.md", "| 헌법(10조 자체) 변경 | owner(사용자) | denylist |"),
    ("P54", "cysjavis-pack/directives/VIBECODING_ENFORCEMENT.md", "### C2.1 우선순위표 (충돌 시 위가 이긴다)\n\n| 순위 | 규칙 | 충돌 시 동작 |\n|---|---|---|\n| 1 | 안전·보안 게이트(9조 Tier 1) | 어떤 규칙도 이를 면제 못함 |\n| 2 | 인간 고정 영역(6조) | doc-sync(8조)보다 우선 — 인간 결정 대기 중이면 done 보류가 정답이지 에이전트 자율 갱신이 아님 |\n| 3 | Spec 선행·검증수단 동봉(1·2조) | 회귀 게이트에 선행 |\n| 4 | Regression 게이트(4조) | waiver 없이는 미완 |\n| 5 | doc-sync(8조) | Level 문서 예산(7조) 내에서 집행 — L1-2에서 \"문서 0~2\" 원칙이 이기되, 계약·상태 변경 발생 시 §C4 재판정으로 자동 격상 |\n| 6 | 증류(10조) | 에러·리뷰 수반 작업만 의무 |\n| 7 | 문서 예산·Level 절차(7조) | pilot(§C6) 결과로 조정 가능 |"),
    ("P55", "cysjavis-pack/soul.md", "시스템을 사용하면서 **당신의** 우선순위·취향·금지선을\n> 직접 채워라."),
    ("P56", "cysjavis-pack/soul.md", "## 금지선 (denylist — 오너 승인 없이 절대 금지)\n\n- 외부 발행(git push·메시지 발송·공개 배포) — 비가역\n- 비가역 삭제\n- 이 파일과 directives/의 무단 변경\n- (오너가 추가)"),
    ("P57", "cysjavis-pack/soul.md", "부여하더라도 정지 경계는 위 **금지선(denylist)** 그대로이고, 오너의 어떤 입력이든 즉시\n  일시정지시키는 kill-switch가 항상 우선이며, 임무를 지정하지 않은 세션에서는 발효하지 않는다."),
    ("P58", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "아래 표와 §2·§3·[절대규칙]의 '경고 push·안내·키 입력·kill·close-surface·queue clear·`cys-dept reap`·잡\n재등록' 은 **§1-1 게이트 안에서** 읽는다 — 수신자는 master/오너 채널뿐이므로 타 노드 경고는 master 경유\n상신이고, 접두 목록 밖 명령은 **보류 + master 에 TTL 승인 요청 + 기록**이다(대체 명령·재시도로 우회 금지 ·\n승인 주체인 master 자신이 고장 대상이면 §1-2 의 오너 채널이 유일한 출구다)."),
    ("P59", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "| `watchdog.duplicate_procs` | 동일 명령 다중 인스턴스(서버 누적 징후) | `cys ps`로 원장 확인 → 소유 노드에 경고 push → 미정리 시 `cys kill <pid>` → master 보고 |"),
    ("P60", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "| `watchdog.proc_count_high` | 한 surface의 자식 폭증 | 해당 노드 점검·경고, 필요 시 `close-surface`(자식 트리 전멸) 건의 |"),
    ("P61", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "| `pane.idle` | 노드 장기 무출력 | read-screen으로 상태 확인 → hang이면 회생 조치(키 입력/재기동 건의)는 **§1-1 승인 안**에서만 — 대상이 master 자신이면 §1-2(오너 채널) |"),
    ("P62", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "`cys kill|close-surface|pause|resume|tombstone|launch-agent` 는 master 의 TTL 승인 동반 시만)"),
    ("P63", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "detach 거부는 송신 0건이다: `detach_requires_seat`(좌석 밖에서 불렀다) · `self_clear_denied`(대상이 너 자신 — self-clear 금지) · `paused`(kill-switch — 보류 기록)."),
    ("P64", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "그 밖의 회생(키 입력·`kill`·\n  `close-surface`·재기동·잡 재등록)은 '사이클' 이라는 이름을 붙여도 이 경계가 아니다 — 판정은 명령\n  이름이 아니라 **효과**다."),
    ("P65", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "컨텍스트가 무거워진 노드(스스로 보고하거나 idle 징후)는 핸드오프 저장 → 재기동 → 복원을 집행한다."),
    ("P66", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "이 집행은 §1-1 접두 목록 **안**의 사전 승인 절차이며 §1-2 ⑦ 의 경계 안이다 — 네 조건 중 하나라도\n  결측·판정 불능이면 §1-2 ④ 의 보류로 떨어지고, 사이클 밖의 회생 행동(키 입력·kill·재기동)은 언제나\n  §1-2 를 따른다."),
    ("P67", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "소유 surface가 사라진 고아는 데몬이\n자동 정리하지만, 정리 실패·예외는 네가 `cys kill <pid>`로 마무리하고 기록한다."),
    ("P68", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "scoped 원장 밖에서 상주 서버를 발견하면\n소유 노드에 scoped 재기동을 지시하고 master에 보고한다."),
    ("P69", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "단 노드 강제 종료·surface 폐쇄는\n  master 승인 후 집행한다(작업 손실 위험)."),
    ("P70", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**상설 의무**: CSO는 능동 모니터링 사이클마다 `cys list`를 점검해 `exited=true`(데몬 권위 판정 = 프로세스 종료된 죽은 pane) surface를 발견하면 **즉시 `cys reap-surface <surface>`로 자동 회수(kill)**한다(★G4: 전용 RPC `surface.reap` — 권위 role(master/cso) 게이트·7조건 판정·감사 이벤트. 구 바이너리에 명령이 없으면 기존 `cys close-surface <surface> --reap` 폴백). 이는 사전 승인된 청소 작업이다 — master 개별 승인 불요."),
    ("P71", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**★즉시성(제품 기본 절차)**: 사이클 폴링만 기다리지 않는다 — inbox 로 `[alert] surface.exited …` 항목을 수신하면 **수신 즉시** 위 reap을 집행한다(집행 도구가 `exited=true` 를 재확인한다 — 항목 문면으로 단정 금지 · [surface exited] 표시 pane이 다음 사이클까지 잔존하는 것 금지). 집행은 결정론 도구 `python3 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_reap_exited.py\"` 1콜로 한다(자동 스냅샷 `round/reap_log/` 보존 + 신/구 바이너리 분기·거부 사유별 처리 내장) — 판단은 이 스크립트의 exit code·stdout JSON만이 사실이다(화면 파싱·자연어 재추론 금지)."),
    ("P72", "cysjavis-pack/directives/CSO_DIRECTIVE.md", "**안전 경계(불가침·kill-safety)**: 오직 `exited=true`만 대상. **live(exited=false) surface는 절대 자동 kill 금지** — 데몬 게이트도 `active_surface`로 이중 거부한다(치명위험 앵커 ④). live 노드 강제종료는 master 승인 필요. '미등록=잔재'로 단정 금지. 판정 근거는 오직 데몬의 exited 플래그(화면 파싱·추측 금지)."),
    ("P73", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "**수정·복구·재기동 등 어떤 행동도 착수 전에** 배달 원장 조회를 선행한다:"),
    ("P74", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "**★'새 워커/병렬 작업' 지시 = 기존 노드 유지 + 새 surface 추가 (제품 기본 절차 · 심각 실수 재발방지)**: \"또 다른 워커를 띄워라\"·\"새 워커로 X 시작\"은 **기존 워커·작업을 그대로 두고 새 surface에 워커를 추가**하라는 뜻이다 — 기존 워커를 죽이거나 교체하는 게 절대 아니다(cys는 worker role 복수 surface 허용 — 동시 운영 가능). 동시에 도착한 지시들의 **대상 노드를 혼동하지 마라**(별개 surface·별개 미션). ★**파괴적·비가역 행동**(worker SIGKILL·kill·close-surface·작업 중단·노드 교체) **전에는 반드시 의도를 명시 확인**한다(절대 강조 4규칙 c) — 정당한 교착 재기동이라도 그 노드의 미션이 '중단'인지 '유지·재개'인지 먼저 확정한다. 추측으로 비가역 실행 절대 금지."),
    ("P75", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "가벼운 경보는 master도 직접 대응할 수 있다(`cys ps`·`cys kill`). 중대 조치(노드 강제 종료·\n  surface 폐쇄)는 master 승인 후 집행한다."),
    ("P76", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "죽은(`exited=true`) 좌석의 pane 잔재 회수는 `cys reap-surface <surface>`다(★G4 전용 RPC\n  `surface.reap` — master/cso pane 전용·7조건 게이트·exit 7=게이트 거부). 상설 집행 주체는\n  CSO(CSO_DIRECTIVE 절대규칙)이며, master는 CSO 부재 시 직접 집행할 수 있다.\n  `close-surface`(자기/생성자 한정·산 노드 폐쇄)와 별개 계약 — **산 노드는 어떤 조합에서도\n  reap 불가**다."),
    ("P77", "cysjavis-pack/directives/MASTER_DIRECTIVE.md", "**메타 안전장치 (불가침)**: ①**kill-switch 최우선** — 오너가 아무 메시지나 입력하면\n  자율주행을 즉시 일시정지하고 오너를 우선한다(재개도 오너 신호로만)"),
    ("P78", "cysjavis-pack/directives/CEO_TEMPLATE.md", "ⓐ **좌석 회수** `cys reap-surface <surface>`(본문 §8 · 죽은 `exited=true` pane 잔재 회수)는 단일소유 가드와 **무관**하며 CEO 직접 집행이 계약대로 허용된다."),
    ("P79", "cysjavis-pack/directives/CEO_TEMPLATE.md", "새 부서가 뜰 때 **기존 부서의 데몬·surface·작업은 절대 건드리지 않는다** — 새 (socket 디렉토리, pack_dir) 쌍이 신규 생성될 뿐이다(집행 주체가 CSO·GUI여도 이 불변식은 동일)."),
    ("P80", "cysjavis-pack/directives/CEO_TEMPLATE.md", "파괴적·비가역 행동(부서 데몬 kill·close-surface·디렉토리 삭제) 전에는 오너 의도를 명시 확인. 추측 비가역 실행 금지."),
    ("P81", "cysjavis-pack/directives/CEO_TEMPLATE.md", "**수정·복구·재기동 등 어떤 행동도 착수 전에** 배달 원장 조회를 선행한다:"),
    ("P82", "cysjavis-pack/directives/CEO_TEMPLATE.md", "**★'새 워커/병렬 작업' 지시 = 기존 노드 유지 + 새 surface 추가 (제품 기본 절차 · 심각 실수 재발방지)**: \"또 다른 워커를 띄워라\"·\"새 워커로 X 시작\"은 **기존 워커·작업을 그대로 두고 새 surface에 워커를 추가**하라는 뜻이다 — 기존 워커를 죽이거나 교체하는 게 절대 아니다(cys는 worker role 복수 surface 허용 — 동시 운영 가능). 동시에 도착한 지시들의 **대상 노드를 혼동하지 마라**(별개 surface·별개 미션). ★**파괴적·비가역 행동**(worker SIGKILL·kill·close-surface·작업 중단·노드 교체) **전에는 반드시 의도를 명시 확인**한다(절대 강조 4규칙 c) — 정당한 교착 재기동이라도 그 노드의 미션이 '중단'인지 '유지·재개'인지 먼저 확정한다. 추측으로 비가역 실행 절대 금지."),
    ("P83", "cysjavis-pack/directives/CEO_TEMPLATE.md", "가벼운 경보는 master도 직접 대응할 수 있다(`cys ps`·`cys kill`). 중대 조치(노드 강제 종료·\n  surface 폐쇄)는 master 승인 후 집행한다."),
    ("P84", "cysjavis-pack/directives/CEO_TEMPLATE.md", "죽은(`exited=true`) 좌석의 pane 잔재 회수는 `cys reap-surface <surface>`다(★G4 전용 RPC\n  `surface.reap` — master/cso pane 전용·7조건 게이트·exit 7=게이트 거부). 상설 집행 주체는\n  CSO(CSO_DIRECTIVE 절대규칙)이며, master는 CSO 부재 시 직접 집행할 수 있다.\n  `close-surface`(자기/생성자 한정·산 노드 폐쇄)와 별개 계약 — **산 노드는 어떤 조합에서도\n  reap 불가**다."),
    ("P85", "cysjavis-pack/directives/CEO_TEMPLATE.md", "**메타 안전장치 (불가침)**: ①**kill-switch 최우선** — 오너가 아무 메시지나 입력하면\n  자율주행을 즉시 일시정지하고 오너를 우선한다(재개도 오너 신호로만)"),
    ("P86", "scripts/ceo_template_header.md", "ⓐ **좌석 회수** `cys reap-surface <surface>`(본문 §8 · 죽은 `exited=true` pane 잔재 회수)는 단일소유 가드와 **무관**하며 CEO 직접 집행이 계약대로 허용된다."),
    ("P87", "scripts/ceo_template_header.md", "새 부서가 뜰 때 **기존 부서의 데몬·surface·작업은 절대 건드리지 않는다** — 새 (socket 디렉토리, pack_dir) 쌍이 신규 생성될 뿐이다(집행 주체가 CSO·GUI여도 이 불변식은 동일)."),
    ("P88", "scripts/ceo_template_header.md", "파괴적·비가역 행동(부서 데몬 kill·close-surface·디렉토리 삭제) 전에는 오너 의도를 명시 확인. 추측 비가역 실행 금지."),
    ("P89", "cysjavis-pack/directives/RADIO_CONTRACT.md", "## 5. pause(kill-switch) 중 처리"),
    ("P90", "cysjavis-pack/directives/WORKER_DIRECTIVE.md", "**행동 경계 probe (결정론 게이트)**: 위험·비가역 행동 직전 해당 `javis_actprobe.py <name> --task <자기 장부 태스크 id>` PASS(exit 0)를 확인한다 — 프로세스 kill 전 `kill-preflight`, 산출물 done 보고 전 `artifact`, verdict 수용 전 `verdict-match`, cys send 제출 확인엔 `submit`. FAIL(2)·판정불가(3)면 행동을 멈추고 master에 보고한다."),
)


def annex_missing(read=repo_text):
    """부록 항목 중 출하 파일에 (비교 규칙으로) 없는 것의 id 목록."""
    cache = {}
    out = []
    for pid, rel, text in STOP_REPORT_ANNEX:
        if rel not in cache:
            cache[rel] = norm(read(rel))
        if norm(text) not in cache[rel]:
            out.append(pid)
    return out


class StopReportFamiliesAnnex(unittest.TestCase):
    """H-02-N5-01(제품 쪽 회귀): 여섯 계열 90문장이 출하 지침·틀·머리말 조각에 그대로 있다."""

    def test_annex_is_complete(self):
        self.assertEqual(len(STOP_REPORT_ANNEX), 90)
        self.assertEqual([p for p, _r, _t in STOP_REPORT_ANNEX], ["P%02d" % i for i in range(1, 91)])

    def test_every_annex_sentence_is_present(self):
        missing = annex_missing()
        self.assertFalse(missing, "부록 문장 부재(지워졌거나 동결 개정 없이 다시 쓰였다): %s" % missing)

    def test_mutation_each_deleted_sentence_turns_red(self):
        """뮤테이션(H-02-N5-01 Control): 항목 하나씩 지운 사본마다 그 항목이 부재로 잡혀야 한다(90회)."""
        texts = {rel: repo_text(rel) for _p, rel, _t in STOP_REPORT_ANNEX}
        normed = {rel: norm(t) for rel, t in texts.items()}
        for pid, rel, text in STOP_REPORT_ANNEX:
            mutated = dict(normed)
            mutated[rel] = normed[rel].replace(norm(text), "", 1)
            got = annex_missing(read=lambda r, m=mutated: m[r])
            self.assertIn(pid, got, "%s 를 지운 사본에서 부재로 잡히지 않았다(시험 무효)" % pid)


class M2MutationControls(unittest.TestCase):
    """뮤테이션(설계 §8 H · INDEX-T2 §0-4): 지킴 문장을 **임시 사본**에서 지우면 해당 시험이 failure(skip 아님)가 된다."""

    CASES = (
        ("MASTER_DIRECTIVE.md", CEO_WAVE2_PIN, "test_n1_no_implementation_rule_lives_in_master_itself"),
        ("MASTER_DIRECTIVE.md", MISSION_GATE, "test_n2_mission_gate_paragraph"),
        ("CSO_DIRECTIVE.md", "사람 운영자 전용", "test_n3_no_forced_queue_delivery_by_agents"),
        ("MASTER_DIRECTIVE.md", NO_TURN_WAIT, "test_n4_no_in_turn_waiting_and_repair_text_does_not_wait"),
        ("WORKER_DIRECTIVE.md", "어떤 에이전트도 결정하지 않는다", "test_d7_owner_only_approval_line"),
        ("WORKER_DIRECTIVE.md", "한 시간에 한 번 이상", "test_worker_submit_not_wait_and_hourly_verb_and_staging"),
        ("REVIEWER_DIRECTIVE.md", "대체 좌석 `reviewer-claude-2`", "test_reviewer_slots_match_orchestra_and_name_the_attest_verbs"),
        ("CSO_DIRECTIVE.md", "`cys repair report|list|status`", "test_cso_reports_only_and_allowlist_sentence"),
        ("REVIEWER_DIRECTIVE.md", "`cys todo-path` 가 출력하는 경로", "test_reviewer_todo_path_follows_the_todo_path_naming_rule"),
    )

    def test_each_guard_sentence_removed_in_a_temp_copy_is_a_failure(self):
        import shutil
        import tempfile
        for fname, needle, test_name in self.CASES:
            tmp = tempfile.mkdtemp(prefix="cys-m2-mut-")
            try:
                for f in os.listdir(DIRECTIVES_DIR):
                    if f.endswith(".md"):
                        shutil.copy(os.path.join(DIRECTIVES_DIR, f), tmp)
                p = os.path.join(tmp, fname)
                with open(p, encoding="utf-8") as fh:
                    text = fh.read()
                self.assertIn(needle, text, "전제: %s 에 %r 실재" % (fname, needle))
                with open(p, "w", encoding="utf-8", newline="") as fh:
                    fh.write(text.replace(needle, ""))
                case = M2DirectiveRewrite(test_name)
                case.dirs = tmp
                res = unittest.TestResult()
                case.run(res)
                self.assertEqual((len(res.failures), len(res.skipped), len(res.errors)), (1, 0, 0),
                                 "%s: %s 에서 %r 를 지운 사본이 failure 1 이 아니다 — f=%d s=%d e=%d"
                                 % (test_name, fname, needle, len(res.failures), len(res.skipped), len(res.errors)))
            finally:
                shutil.rmtree(tmp, ignore_errors=True)

    def test_master_small_fix_allowance_reinserted_is_a_failure(self):
        """279행 허용을 되살린 사본은 N1 시험이 잡는다(설계 §6-2 '279행 허용은 반드시 지운다')."""
        import shutil
        import tempfile
        tmp = tempfile.mkdtemp(prefix="cys-m2-mut-")
        try:
            for f in os.listdir(DIRECTIVES_DIR):
                if f.endswith(".md"):
                    shutil.copy(os.path.join(DIRECTIVES_DIR, f), tmp)
            p = os.path.join(tmp, "MASTER_DIRECTIVE.md")
            with open(p, encoding="utf-8") as fh:
                text = fh.read()
            with open(p, "w", encoding="utf-8", newline="") as fh:
                fh.write(text + "\n(직접 수정은 사소한 마무리에만 허용)\n")
            case = M2DirectiveRewrite("test_n1_no_implementation_rule_lives_in_master_itself")
            case.dirs = tmp
            res = unittest.TestResult()
            case.run(res)
            self.assertEqual(len(res.failures), 1)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


class DeliveryPartCapRoom(unittest.TestCase):
    """RULINGS AA2 + BB2 (i): 데몬의 조각 상한 핀(src/bin/cysd/delivery.rs `deployed_directive_payload_fits_part_cap_with_headroom`·
    `deployed_ceo_directive_payload_fits_part_cap_with_headroom` · `MAX_PARTS` 4016 = 4배 여유로 1,004 제출단위)은
    올리지 않는다. 이 시험은 그 두 핀과 같이 **실제 합성 그대로** 잰다(BB2 (i)): 역할 디렉티브(CEO 는 CEO_TEMPLATE) →
    절 머리줄 → RSI → 절 머리줄 → soul.md → 절 머리줄(경로) → memory/MEMORY.md → 절 머리줄 → 스킬 색인(`- name: description`)
    → 트랙 T1a 의 역할 규칙 블록(src/repair/rules.rs 의 RULES_COMMON + RULES_MASTER_CEO). 단위 = 공백 정규화 뒤 비어 있지
    않은 **고유** 줄(delivery.rs `submit_units` 와 같은 규칙). 두 축 모두 1,004 이하."""
    BAR = 4016 // 4
    HEADERS = ("\n\n■ RSI_LEARNING_DIRECTIVE.md (5번째 절대지침 — 학습 루프)\n", "\n\n■ soul.md (운영 헌장)\n",
               "\n\n■ 장기메모리 색인 (%s — 노드 공유 의미 기억 · 증류는 bin/javis_memory.py add)\n",
               "\n\n■ 보유 스킬 색인 (본문: `cys skill show <name>`)\n")

    @staticmethod
    def _units(text):
        import re
        seen = set()
        for raw in re.split(r"[\n\r]", text):
            n = " ".join(raw.split())
            if n:
                seen.add(n)
        return len(seen)

    def _compose(self, head_rel):
        import re
        pack = os.path.dirname(DIRECTIVES_DIR)
        rd = lambda rel: (open(os.path.join(pack, rel), encoding="utf-8").read()
                          if os.path.isfile(os.path.join(pack, rel)) else "")
        h_rsi, h_soul, h_mem, h_skill = self.HEADERS
        t = (rd(head_rel) + h_rsi + rd("directives/RSI_LEARNING_DIRECTIVE.md") + h_soul + rd("soul.md")
             + h_mem % os.path.join(pack, "memory", "MEMORY.md") + rd("memory/MEMORY.md"))
        skills = []
        sk = os.path.join(pack, "skills")
        for e in (os.listdir(sk) if os.path.isdir(sk) else []):
            try:
                with open(os.path.join(sk, e, "SKILL.md"), encoding="utf-8") as fh:
                    s = fh.read()
            except OSError:
                continue
            head = s.splitlines()[:10]
            field = lambda k: next((l[len(k):].strip() for l in head if l.startswith(k)), "")
            if field("name:"):
                skills.append("- %s: %s\n" % (field("name:"), field("description:")))
        if skills:
            t += h_skill + "".join(skills)
        rules = repo_text(os.path.join("src", "repair", "rules.rs"))
        lit = lambda name: re.search(r'pub const %s: &str = r#"(.*?)"#;' % name, rules, re.S).group(1)
        return t + "\n\n" + lit("RULES_COMMON") + lit("RULES_MASTER_CEO")

    def test_headers_are_the_ones_compose_directive_writes(self):
        src = repo_text(os.path.join("src", "bin", "cys.rs"))
        body = src[src.index("fn compose_directive("):]
        body = body[:body.index("\n}\n")]
        for h in self.HEADERS:
            lit = '"%s"' % h.replace("\n", "\\n").replace("%s", "{}")
            self.assertIn(lit, body, "compose_directive 의 절 머리줄이 바뀌었다 — 이 시험의 머리줄도 고쳐라")

    def test_ceo_and_master_axes_fit_the_bar_as_composed(self):
        for axis, f in (("ceo", "directives/CEO_TEMPLATE.md"), ("master", "directives/MASTER_DIRECTIVE.md")):
            units = self._units(self._compose(f))
            self.assertLessEqual(units, self.BAR, "%s 축 %d 단위 > %d (RULINGS AA2 · BB2 (i))" % (axis, units, self.BAR))


class NoMasterShortcutAnywhere(unittest.TestCase):
    """RULINGS AH3 (AGREEMENT 28): no shipped directive OR skill text keeps a master / CEO implementation shortcut
    ("사소한 마무리"는 master 가 직접 한다 류). 문장 단위가 아니라 낱말 묶음으로 잰다 — 금지문("§1-A 사소 예외 없이 금지")은
    이 낱말을 쓰지 않는다."""
    PHRASES = MASTER_SMALL_FIX + ("사소한 마무리", "한두 줄 수정처럼 위임 오버헤드", "직접 수정은 사소")

    @classmethod
    def hits(cls, texts):
        return sorted({(rel, ph) for rel, text in texts.items() for ph in cls.PHRASES if ph in norm(text)})

    def shipped(self):
        pack = os.path.dirname(DIRECTIVES_DIR)
        out = {}
        for top in ("directives", "skills"):
            for root, _d, files in os.walk(os.path.join(pack, top)):
                for f in files:
                    if f.endswith(".md"):
                        p = os.path.join(root, f)
                        with open(p, encoding="utf-8", errors="replace") as fh:
                            out[os.path.relpath(p, pack)] = fh.read()
        return out

    def test_no_shortcut_in_any_directive_or_skill(self):
        texts = self.shipped()
        self.assertTrue(any(k.startswith("skills" + os.sep) for k in texts), "스킬 텍스트를 읽지 못했다(시험 무효)")
        self.assertEqual(self.hits(texts), [], "master 직접 수정 허용이 출하 텍스트에 남았다")

    def test_detector_fires_on_a_planted_shortcut(self):
        planted = {"skills/x/SKILL.md": "master는 직접 구현하지 않는다\n  (사소한 마무리 제외)."}
        self.assertEqual(self.hits(planted), [("skills/x/SKILL.md", "사소한 마무리")])


class MasterMayFileAReport(unittest.TestCase):
    """RULINGS BB2 (ii): the master's compiled role block and MASTER_DIRECTIVE §1-B say plainly that intake
    (`cys repair report`) is allowed to a master — only the ledger-changing verbs are not."""

    def test_block_and_directive_allow_report(self):
        rules = repo_text(os.path.join("src", "repair", "rules.rs"))
        self.assertIn("장부를 바꾸는 `cys repair` 동사(begin·commit 등)는 쓰지 않는다(접수 `cys repair report`는 쓴다).", rules)
        m = norm(open(os.path.join(DIRECTIVES_DIR, "MASTER_DIRECTIVE.md"), encoding="utf-8").read())
        self.assertIn("master 가 부르는 것은 결함 접수(`cys repair report` — 누구나, 장부를 바꾸지 않는다), 읽기(", m)

if __name__ == "__main__":
    unittest.main(verbosity=2)
