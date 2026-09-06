#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_cso_directive_rev.py — CSO alert inbox 개정 계약의 회귀를 막는다(0.14.31 WP-3 C · codex 초안 → 워커 전 행 검토 채택).

왜 존재하는가: Pack P2 표지 판정·preflight 핀·벤치 문구가 실제 배포 지시문과
엇갈리거나, 폐기한 직접 구독이 되살아나는 것을 음성 대조군과 함께 검출한다.
주석 제거의 권위는 DOTALL 비탐욕 정규식이다. sed 범위 삭제는 한 줄 표지 주석
뒤의 본문까지 삼킬 수 있으므로 측정에 사용하지 않는다.
repo 지시문만 읽으며 HOME·라이브 팩을 건드리지 않는다. 변조는 메모리 사본뿐이다.

    python3 cysjavis-pack/bin/tests/test_cso_directive_rev.py
"""
from __future__ import annotations

import os
import re
import sys
import unittest

MARKER = "<!-- cso-directive-rev: 2026-09-06-alert-inbox -->"
SYNC = "stopped_stagnation은 종결이며 minor는 백로그 목록으로 인계"
REQUIRED_CLAUSE_TOKENS = (
    "직접 구독 금지", "hooks/role-capability-gate.sh", "Monitor",
    "cys events --after-seq", "alert_route", "구 데몬 폴백", "tool_calls",
    "1,500회", "2,000회", "예산 deny 에서만 면제", "스크린샷 정책", "1장",
    "TTL 인지", "cys queue list", "예외는 우회가 아니라 승인",
    "게이트 deny 는 고장이 아니라 승인 요청 신호다",
    "cys send --queued --to master", "last_fired",
)
AFFIRMATIVE_SUBSCRIPTION_PHRASES = (
    "상시 구독하라", "구독을 걸고",
    "cys events --category watchdog --category health --category queue --reconnect",
    "cys events --category watchdog\n--category health --reconnect",
    "cys events --category watchdog --category health --reconnect",
    "send-key --to master Return",
)
CLAUSE_PINS = ("exited surface 자동 reap", "즉시성")
BENCH_KEYWORDS = ("확인했다", "실측")

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
# import 시에도 repo 에 __pycache__ 를 쓰지 않는다.
sys.dont_write_bytecode = True
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402 — 핀의 SOT 는 preflight

DIRECTIVES_DIR = os.path.join(os.path.dirname(BIN), "directives")


def strip_html_comments(text: str) -> str:
    """한 줄·여러 줄 HTML 주석만 제거하고 본문은 보존한다."""
    return re.sub(r"<!--.*?-->", "", text, flags=re.DOTALL)


def marker_line_index(text: str, marker: str) -> int | None:
    """공백 제거 후 정확히 일치하는 첫 표지의 1 기반 행 번호를 반환한다."""
    return next((i for i, line in enumerate(text.splitlines(), 1)
                 if line.strip() == marker), None)


def affirmative_subscription_phrases(body: str) -> list[str]:
    """주석을 뺀 본문에서 긍정 구독 지시만 찾고 reconnect 단독 언급은 허용한다."""
    return [phrase for phrase in AFFIRMATIVE_SUBSCRIPTION_PHRASES if phrase in body]


def marker_within_first_20(text: str) -> bool:
    """Pack P2 와 공유하는 표지 위치 계약을 판정한다."""
    index = marker_line_index(text, MARKER)
    return index is not None and index <= 20


def sync_occurs_once(text: str) -> bool:
    """동기화 문장의 누락과 중복을 같은 술어로 거부한다."""
    return text.count(SYNC) == 1


def section_body(text: str, heading: str) -> str:
    """지정한 제목 접두부터 다음 2단계 제목 직전까지 본문을 추출한다."""
    lines = text.splitlines()
    start = next(i for i, line in enumerate(lines)
                 if line == heading or line.startswith(heading + " "))
    end = next((i for i in range(start + 1, len(lines))
                if lines[i].startswith("## ")), len(lines))
    return "\n".join(lines[start + 1:end])


class CsoDirectiveRevision(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls.raw = {}
        for name in ("CSO_DIRECTIVE.md", "REVIEWER_DIRECTIVE.md", "MASTER_DIRECTIVE.md"):
            path = os.path.join(DIRECTIVES_DIR, name)
            # newline="" 로 CRLF 를 보존해야 LF 계약을 실제로 검사할 수 있다.
            with open(path, encoding="utf-8", errors="strict", newline="") as source:
                cls.raw[name] = source.read()
        cls.cso = cls.raw["CSO_DIRECTIVE.md"]
        cls.body = strip_html_comments(cls.cso)

    def test_revision_marker(self):
        """Pack P2 가 신판으로 판정할 정확한 표지는 첫 20행 안에 있어야 한다."""
        self.assertTrue(marker_within_first_20(self.cso),
                        "CSO 개정 표지 누락 또는 20행 초과")

    def test_no_affirmative_subscription_or_ten_minute_duty(self):
        """주석 밖에서 직접 구독 지시와 폐기된 10분 의무가 사라져야 한다."""
        self.assertEqual(affirmative_subscription_phrases(self.body), [])
        self.assertEqual(self.body.count("10분 의무"), 0)

    def test_required_clause_tokens(self):
        """inbox·게이트·예산·승인 조항 토큰의 부재를 전수 목록으로 보고한다."""
        missing = [token for token in REQUIRED_CLAUSE_TOKENS if token not in self.body]
        self.assertFalse(missing, "CSO 본문 조항 부재: %r" % missing)

    def test_raw_preflight_and_bench_pins(self):
        """원문 핀 패리티를 저비용으로 중복 검증하고 벤치 어휘도 보존한다."""
        pins = list(pf.CONTENT_PINS["CSO_DIRECTIVE.md"])
        pins += [(pin, "preflight 조항") for pin in CLAUSE_PINS]
        pins += [(pin, "directive-bench") for pin in BENCH_KEYWORDS]
        missing = [(pin, label) for pin, label in pins if pin not in self.cso]
        self.assertFalse(missing, "CSO 원문 핀 부재: %r" % missing)

    def test_sync_sentence_once_in_correct_sections(self):
        """종결·minor 인계 문장은 양쪽 원문에서 한 번씩 올바른 절에 있어야 한다."""
        for name, heading in (("REVIEWER_DIRECTIVE.md", "## 4. 라운드 루프"),
                              ("MASTER_DIRECTIVE.md", "## 9. 복원 체크포인트")):
            with self.subTest(directive=name):
                raw = self.raw[name]
                self.assertTrue(sync_occurs_once(raw),
                                "%s 동기화 문장 횟수: %d" % (name, raw.count(SYNC)))
                self.assertTrue(sync_occurs_once(section_body(raw, heading)))

    def test_master_subscription_backlog_guard(self):
        """master 구독은 의도적 백로그이며 이번 CSO 개정에서 수정할 대상이 아니다."""
        # master 측 구독 행은 0.14.31 범위 밖이다. 미래 수정은 의식적인 재핀을
        # 요구하도록 보존한다. 이 백로그 가드를 '고치기' 위해 구독을 삭제하지 않는다.
        self.assertIn("cys events --category feed --category watchdog --category queue",
                      self.raw["MASTER_DIRECTIVE.md"])

    def test_cso_utf8_and_lf(self):
        """엄격한 UTF-8 읽기가 성공하고 원문에 CRLF 가 없어야 한다."""
        self.assertNotIn("\r\n", self.cso)
        self.assertEqual(self.cso.encode("utf-8").decode("utf-8", errors="strict"),
                         self.cso)

    def test_negative_marker_controls(self):
        """표지 삭제·25행 이동·접미 텍스트는 표지 승인 판정을 뒤집어야 한다."""
        removed = self.cso.replace(MARKER, "")
        self.assertIsNone(marker_line_index(removed, MARKER))
        self.assertFalse(marker_within_first_20(removed))
        moved = "\n" * 24 + MARKER + "\n" + removed
        # 전체 위치 helper 는 25 를 반환하지만, 첫 20행 검색에는 표지가 없다.
        self.assertEqual(marker_line_index(moved, MARKER), 25)
        self.assertIsNone(marker_line_index("\n".join(moved.splitlines()[:20]), MARKER))
        self.assertFalse(marker_within_first_20(moved))
        self.assertIsNone(marker_line_index(MARKER + " trailing text", MARKER))
        self.assertEqual(marker_line_index("\n  " + MARKER + "  \n" + MARKER, MARKER), 2)

    def test_negative_comment_controls(self):
        """주석 안팎을 구별하고 sed 의 한 줄 주석 이후 본문 소실 결함을 막는다."""
        comment = "<!--\n개정 근거: 10분 의무\n-->"
        self.assertEqual(strip_html_comments(comment).count("10분 의무"), 0)
        self.assertEqual(strip_html_comments(comment + "\n10분 의무").count("10분 의무"), 1)
        sample = MARKER + "\n본문 10분 의무\n<!-- 다음 주석 -->\n끝"
        self.assertEqual(strip_html_comments(sample), "\n본문 10분 의무\n\n끝")

    def test_negative_subscription_controls(self):
        """구독 지시 주입은 검출하고 금지문의 reconnect 언급은 허용한다."""
        injected = self.body + "\n상시 구독하라: `cys events ... --reconnect`"
        self.assertTrue(affirmative_subscription_phrases(injected))
        for phrase in AFFIRMATIVE_SUBSCRIPTION_PHRASES:
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, affirmative_subscription_phrases(phrase))
        prohibition = "`cys events --reconnect`(상시 구독)·Monitor 도구 ... 직접 구독하지 마라"
        self.assertEqual(affirmative_subscription_phrases(prohibition), [])

    def test_negative_sync_controls(self):
        """리뷰어 사본의 동기화 문장 삭제와 중복은 모두 횟수 계약을 깨야 한다."""
        reviewer = self.raw["REVIEWER_DIRECTIVE.md"]
        self.assertTrue(sync_occurs_once(reviewer))
        self.assertFalse(sync_occurs_once(reviewer.replace(SYNC, "")))
        self.assertFalse(sync_occurs_once(reviewer + "\n" + SYNC))


if __name__ == "__main__":
    unittest.main(verbosity=2)
