#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_cso_directive_rev.py — CSO alert inbox 개정 계약의 회귀를 막는다(0.14.31 WP-3 C · codex 초안 → 워커 전 행 검토 채택 · R1 리뷰 반영).

왜 존재하는가: Pack P2 표지 판정·preflight 핀·벤치 문구가 실제 배포 지시문과
엇갈리거나, 폐기한 직접 구독이 되살아나는 것을 음성 대조군과 함께 검출한다.
R1 추가 범위: 정기 60분 요구(10분 복원 거부) · `cys events` 언급은 정본 금지 문구
3종(정확 접두)만 허용(1회 조회 예외·긍정 지시 거부) · plan-B 경보 리터럴 패리티와
두 이벤트 집합(inbox 라우팅·즉시 각성)의 **집합 등가** · 표지는 정확 행 등가(P2 의
어떤 판정보다 좁은 안전 부분집합).
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
    "종결 없는 스트림", "alert_route", "구 데몬 폴백", "tool_calls",
    "1,500회", "2,000회", "예산 deny 에서만 면제", "스크린샷 정책", "1장",
    "TTL 인지", "cys queue list", "예외는 우회가 아니라 승인",
    "게이트 deny 는 고장이 아니라 승인 요청 신호다",
    "cys send --queued --to master", "last_fired",
    "Skill 도구", "재등록·재기동은 §1-1 게이트 안", "데몬 재시작까지",
    "생존 확인 불가", "구독에는 예외가 없다",
)
# plan §4 WP-3 B 리터럴 — Rust 레인 alert_route 와의 패리티 상수는 통합 항목이다(바뀌면 여기와 지침 §1 갱신).
ALERT_EVENTS = (
    "health.alert", "watchdog.*", "surface.exited", "context.threshold",
    "queue.starved", "queue.depth_high",
)
ALERT_ROUTE_LITERALS = (
    "[alert] <이벤트명> surface:<id>", "5분 쿨다운", "시간당 20건", "300s",
)
# `cys events` 는 어떤 플래그로도 종결 없는 events.stream 이다(cys.rs stream_events · main.rs
# run_event_stream) — 본문의 모든 언급은 아래 정본 금지 문구 중 하나의 **정확 접두**여야 한다.
# 금지 토큰 존재를 의미 판정으로 쓰지 않는다: "Monitor 는 금지; `cys events ...` 를 실행하라" 류를
# 통과시키기 때문이다. 문구를 고치면 여기도 의식적으로 재핀한다.
CANONICAL_EVENT_MENTIONS = (
    "`cys events` 는 플래그와 무관하게 종결 없는 스트림이라 금지",
    "`cys events` (`--after-seq` 를 붙여도 `events.stream` 을 여는 종결 없는 스트림이다 — "
    "1회 조회형은 없다)·Monitor 도구·백그라운드 tail 로 직접 구독하지 마라",
    "`cys events` 는 어떤 플래그로도 접두 밖(deny · 스트림 — TTL 승인 대상도 아니다: 구독에는 예외가 없다)",
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
INBOX_LIST_START, INBOX_LIST_END = "alert 라우팅이", "`[alert]"
WAKE_LIST_START, WAKE_LIST_END = "이상 이벤트는 주기를 기다리지 않는다", "수신 시"
EVENT_NAME_RE = re.compile(r"`([a-z_]+\.[a-z_*]+)`")

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


def normalize(text: str) -> str:
    """강조 표식을 걷고 공백 연쇄(줄바꿈 포함)를 한 칸으로 접는다 — 줄바꿈 위치와 무관한 접두 대조용."""
    return re.sub(r"\s+", " ", text.replace("**", ""))


def marker_line_index(text: str, marker: str) -> int | None:
    """공백을 제거하지 않고 행 전체가 정확히 일치하는 첫 표지의 1 기반 행 번호를 반환한다."""
    return next((i for i, line in enumerate(text.splitlines(), 1)
                 if line == marker), None)


def affirmative_subscription_phrases(body: str) -> list[str]:
    """주석을 뺀 본문에서 긍정 구독 지시만 찾고 reconnect 단독 언급은 허용한다."""
    return [phrase for phrase in AFFIRMATIVE_SUBSCRIPTION_PHRASES if phrase in body]


def event_stream_violations(body: str) -> list[str]:
    """정본 금지 문구의 정확 접두가 아닌 `cys events` 언급 전부를 돌려준다(빈 목록이 합격)."""
    normalized = normalize(body)
    out = []
    for m in re.finditer(r"cys events", normalized):
        start = m.start() - 1  # 여는 백틱까지 포함해 대조한다 — 백틱 없는 언급은 그 자체로 위반
        tail = normalized[start:] if start >= 0 else ""
        if not any(tail.startswith(c) for c in CANONICAL_EVENT_MENTIONS):
            out.append(normalized[max(start, 0):m.end() + 60])
    return out


def event_names_between(body: str, start: str, end: str) -> set[str] | None:
    """두 표식 사이의 백틱 이벤트 이름 집합 — 표식이 없으면 None(결측은 값이 아니다)."""
    _, found, rest = normalize(body).partition(start)
    if not found:
        return None
    segment, found_end, _ = rest.partition(end)
    if not found_end:
        return None
    return set(EVENT_NAME_RE.findall(segment))


def active_check_bullet(body: str) -> str:
    """'- **능동 점검(' 로 시작하는 bullet 본문(다음 bullet 직전까지)."""
    start = body.find("- **능동 점검(")
    if start < 0:
        return ""
    end = body.find("\n- **", start + 1)
    return body[start:end if end >= 0 else len(body)]


def sixty_minute_interval_present(body: str) -> bool:
    """능동 점검 bullet 안에서 제목·경과 조건이 60분이고 10분 언급이 0 이어야 한다."""
    bullet = active_check_bullet(body)
    return ("정기 60분" in bullet and "**60분** 경과" in bullet
            and "10분" not in bullet)


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
        """Pack P2 가 신판으로 판정할 정확한 표지는 첫 20행 안에 정확 행으로 있어야 한다."""
        self.assertTrue(marker_within_first_20(self.cso),
                        "CSO 개정 표지 누락 또는 20행 초과")

    def test_no_affirmative_subscription_or_ten_minute_duty(self):
        """주석 밖에서 직접 구독 지시와 폐기된 10분 의무가 사라져야 한다."""
        self.assertEqual(affirmative_subscription_phrases(self.body), [])
        self.assertEqual(self.body.count("10분 의무"), 0)

    def test_sixty_minute_interval(self):
        """능동 점검 bullet 은 정기 60분을 요구하고 10분 언급이 없어야 한다(10분 복원 차단)."""
        self.assertTrue(active_check_bullet(self.body), "능동 점검 bullet 부재")
        self.assertTrue(sixty_minute_interval_present(self.body))

    def test_event_stream_mentions_are_canonical_prohibitions(self):
        """본문의 모든 `cys events` 언급은 정본 금지 문구 3종의 정확 접두여야 한다(1회 조회 예외 없음)."""
        self.assertEqual(event_stream_violations(self.body), [])
        self.assertGreaterEqual(normalize(self.body).count("cys events"), 3,
                                "금지 문구 3종 중 일부가 본문에서 사라졌다")

    def test_required_clause_tokens(self):
        """inbox·게이트·예산·승인 조항 토큰의 부재를 전수 목록으로 보고한다."""
        missing = [token for token in REQUIRED_CLAUSE_TOKENS if token not in self.body]
        self.assertFalse(missing, "CSO 본문 조항 부재: %r" % missing)

    def test_alert_route_literals_parity(self):
        """plan-B 경보 이름·라우팅 리터럴이 본문에 있고 두 이벤트 목록은 6종과 집합 등가여야 한다."""
        missing = [token for token in ALERT_EVENTS + ALERT_ROUTE_LITERALS
                   if token not in self.body]
        self.assertFalse(missing, "CSO 경보 리터럴 부재: %r" % missing)
        expected = set(ALERT_EVENTS)
        self.assertEqual(event_names_between(self.body, INBOX_LIST_START, INBOX_LIST_END), expected)
        self.assertEqual(event_names_between(self.body, WAKE_LIST_START, WAKE_LIST_END), expected)

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

    def test_master_termination_count_matches_fourth_cause(self):
        """MASTER §7 (5-8) 의 종결 사유 계수는 §9 의 도구 판정을 넷째로 센다."""
        master = self.raw["MASTER_DIRECTIVE.md"]
        self.assertNotIn("셋 중 먼저 온 것", master)
        self.assertIn("넷 중 먼저 온 것", master)
        self.assertIn("넷째는 §9", master)

    def test_master_subscription_backlog_guard(self):
        """master 구독은 의도적 백로그이며 이번 CSO 개정에서 수정할 대상이 아니다."""
        # master 측 구독 행은 0.14.31 범위 밖이다. 이 단언이 미래에 실패하면 그 뜻은
        # "백로그 작업 뒤 의식적으로 재핀하라"이지, "구독을 복원하라"가 절대 아니다.
        # 이 백로그 가드를 '고치기' 위해 구독을 삭제하지 않는다.
        self.assertIn("cys events --category feed --category watchdog --category queue",
                      self.raw["MASTER_DIRECTIVE.md"])

    def test_utf8_and_lf(self):
        """세 지시문 모두 엄격 UTF-8 로 읽히고 CRLF 가 없어야 한다(Windows 체크아웃 회귀)."""
        for name, raw in self.raw.items():
            with self.subTest(directive=name):
                self.assertNotIn("\r\n", raw)
                self.assertEqual(raw.encode("utf-8").decode("utf-8", errors="strict"), raw)

    def test_negative_marker_controls(self):
        """표지 삭제·25행 이동·들여쓰기·접미 텍스트는 표지 승인 판정을 뒤집어야 한다."""
        removed = self.cso.replace(MARKER, "")
        self.assertIsNone(marker_line_index(removed, MARKER))
        self.assertFalse(marker_within_first_20(removed))
        moved = "\n" * 24 + MARKER + "\n" + removed
        # 전체 위치 helper 는 25 를 반환하지만, 첫 20행 검색에는 표지가 없다.
        self.assertEqual(marker_line_index(moved, MARKER), 25)
        self.assertIsNone(marker_line_index("\n".join(moved.splitlines()[:20]), MARKER))
        self.assertFalse(marker_within_first_20(moved))
        self.assertIsNone(marker_line_index(MARKER + " trailing text", MARKER))
        self.assertIsNone(marker_line_index("  " + MARKER, MARKER))
        # 들여쓴 2행은 정확 행이 아니므로 건너뛰고 3행의 정확 표지가 잡힌다.
        self.assertEqual(marker_line_index("\n  " + MARKER + "  \n" + MARKER, MARKER), 3)

    def test_negative_sixty_minute_controls(self):
        """60분→10분 일괄 변조와 bullet 안 10분 지시 추가는 모두 거부돼야 한다."""
        self.assertFalse(sixty_minute_interval_present(self.body.replace("60분", "10분")))
        bullet = active_check_bullet(self.body)
        injected = self.body.replace(bullet, bullet + "\n  10분마다 점검하라.")
        self.assertNotEqual(injected, self.body)
        self.assertFalse(sixty_minute_interval_present(injected))
        self.assertFalse(sixty_minute_interval_present(""))

    def test_negative_event_stream_controls(self):
        """1회 조회 지시·옛 허용 예외·금지 토큰 동거 지시·부정 금지문은 위반이고 정본 금지 문구는 통과한다."""
        self.assertTrue(event_stream_violations(
            self.body + "\n1회 조회 `cys events --after-seq 0`"))
        old_exception = ("근거 확인용 **1회 조회** `cys events --after-seq <n>` "
                         "(비-reconnect)만 허용된다.")
        self.assertTrue(event_stream_violations(old_exception))
        # codex 반례: 금지 토큰이 같은 문장에 있어도 긍정 지시는 위반이다.
        self.assertTrue(event_stream_violations(
            "Monitor 는 금지; 근거 확인은 `cys events --after-seq 0` 을 실행하라."))
        self.assertTrue(event_stream_violations("`cys events` 를 금지하지 않는다."))
        self.assertTrue(event_stream_violations("cys events 구독 금지"))  # 백틱 없는 언급
        for canonical in CANONICAL_EVENT_MENTIONS:
            with self.subTest(canonical=canonical[:30]):
                self.assertEqual(event_stream_violations("앞 문장. **" + canonical + " — 뒤.**"), [])
                # 줄바꿈·들여쓰기가 섞여도 정규화 뒤 접두 대조는 같다.
                wrapped = canonical.replace(" ", "\n  ", 3)
                self.assertEqual(event_stream_violations(wrapped), [])

    def test_negative_event_set_controls(self):
        """즉시 각성 목록에서 하나를 빼거나 목록 밖 이벤트를 더하면 집합 등가가 깨져야 한다."""
        expected = set(ALERT_EVENTS)
        wake = event_names_between(self.body, WAKE_LIST_START, WAKE_LIST_END)
        self.assertEqual(wake, expected)
        dropped = self.body.replace("`queue.depth_high`·`context.threshold`·`surface.exited` 수신 시",
                                    "`context.threshold`·`surface.exited` 수신 시")
        self.assertNotEqual(dropped, self.body)
        self.assertEqual(event_names_between(dropped, WAKE_LIST_START, WAKE_LIST_END),
                         expected - {"queue.depth_high"})
        added = self.body.replace("`surface.exited` 수신 시", "`surface.exited`·`pane.idle` 수신 시")
        self.assertNotEqual(added, self.body)
        self.assertEqual(event_names_between(added, WAKE_LIST_START, WAKE_LIST_END),
                         expected | {"pane.idle"})
        self.assertIsNone(event_names_between("표식 없음", WAKE_LIST_START, WAKE_LIST_END))

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
