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

# Wave2 대기 핀: D2 합성 서문(gen_ceo_template.py 재합성)이 넣을 문구 — 재합성 착지 전에는
# repo 템플릿에 없는 것이 정상이라 부재 시 skip(착지 후에는 자동으로 상주 단언이 된다).
CEO_WAVE2_PIN = "직접 구현은 §1-A 사소 예외 없이 금지"


class ContentPinsParity(unittest.TestCase):
    maxDiff = None

    def _text(self, fname):
        p = os.path.join(DIRECTIVES_DIR, fname)
        self.assertTrue(os.path.isfile(p), "repo 디렉티브 부재: %s" % p)
        with open(p, encoding="utf-8") as f:   # encoding 명시(계약 — 로케일 비의존)
            return f.read()

    def _assert_pins(self, fname, exclude=()):
        """부재 핀을 **전수 목록**으로 보고한다(1개씩 죽는 단언 금지 — 정확 보고 임무)."""
        text = self._text(fname)
        missing = [(pin, label) for pin, label in pf.CONTENT_PINS[fname]
                   if pin not in exclude and pin not in text]
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
        """CEO 템플릿 핀 — Wave2 대기 핀만 제외한 나머지(표지 3핀)는 지금 실존해야 한다."""
        self._assert_pins("CEO_TEMPLATE.md", exclude=(CEO_WAVE2_PIN,))

    def test_ceo_wave2_preface_pin(self):
        """Wave2 합성 서문 핀 — 부재 = skip(재합성 대기·정상), 존재 = 상주 단언(자동 승격)."""
        if CEO_WAVE2_PIN not in self._text("CEO_TEMPLATE.md"):
            self.skipTest("Wave2 재합성 대기: CEO_TEMPLATE.md 에 %r 부재 — D2 재합성"
                          "(gen_ceo_template.py) 착지 후 이 skip 은 자동으로 상주 단언이 된다"
                          % CEO_WAVE2_PIN)
        # 존재하면 그 자체가 단언 통과 — 별도 assert 불요(위 멤버십 검사가 곧 검증).

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


if __name__ == "__main__":
    unittest.main(verbosity=2)
