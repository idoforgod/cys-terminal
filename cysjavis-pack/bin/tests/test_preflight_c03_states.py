#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c03_states.py — C03 두 축(핀·건전성) 상태 전수 회귀 하네스 (스펙 v4 §D3(ii)+A6 · W2).

★왜 이 파일이 존재하는가: C03 재구성은 승격 상태 기계(표지 3경로·.pre-ceo 검사 표면·구본화/
  퇴화/소실 분류·지문 dedupe)를 신설했는데, 종전 tests/ 에는 CONTENT_PINS 관련 테스트가
  0건이었다(R1 시나리오9 권고 — 'C03 무-fix'를 회귀로 고정). 여기서 상태 전수를 박제한다.

★계측 타당성(test_preflight_phase1_checks·run_bootstrap_health 관용): "PASS 가 나온다"는
  하네스는 아무것도 증명하지 않는다 — 모든 상태에 **결함을 주입해 판정이 실제로 뒤집히는지**
  (PASS→FAIL/WARN)를 함께 잰다.

라이브 무접촉: 픽스처 HOME(임시 디렉터리) 아래 **정확히 $HOME/.cys/pack** 에 픽스처 팩을
둔다 — is_dept_pack() 이 기본 팩 실경로 대조라서, 다른 경로에 두면 전 케이스가 dept 면제로
빠져 아무것도 검증하지 못한다. 쓰기(지문 원장 포함)는 픽스처 HOME 안에서만 일어난다.
--fix 는 '무변형 계약' 검증 케이스에서만 켠다(C03 에 --fix 경로가 없음을 실측).

    python3 cysjavis-pack/bin/tests/test_preflight_c03_states.py
"""
import hashlib
import json
import os
import shutil
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402 — 형제 모듈 직접 구동(서브프로세스 아님)

PASS, WARN, FAIL, SKIP = pf.PASS, pf.WARN, pf.FAIL, pf.SKIP

MASTER_PINS = pf.CONTENT_PINS["MASTER_DIRECTIVE.md"]
CEO_PINS = pf.CONTENT_PINS["CEO_TEMPLATE.md"]
ROLE_FILES = ("WORKER_DIRECTIVE.md", "CSO_DIRECTIVE.md", "REVIEWER_DIRECTIVE.md")


def master_body(drop=()):
    """표준 MASTER 픽스처 — CONTENT_PINS 와 동기(핀 개정 시 자동 추종). drop=제외 핀."""
    return "\n".join(p for p, _ in MASTER_PINS if p not in drop) + "\n표준 MASTER 본문\n"


def tmpl_body(kind="current"):
    """CEO 템플릿 픽스처 — 표지 3핀(MARKER_PINS)+Wave2 핀 실존(post-Wave2 형상).
    구·신 구분은 꼬리 바이트로만 낸다(표지 핀 불변식: 구·신 양쪽 실존)."""
    return ("\n".join(p for p, _ in CEO_PINS) + "\nCEO 거버넌스 머리글\n"
            + ("현행 템플릿\n" if kind == "current" else "구판 템플릿\n"))


def sha256(b):
    return hashlib.sha256(b).hexdigest()


class Base(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.mkdtemp(prefix="pf-c03-")
        self.addCleanup(shutil.rmtree, self.td, True)
        self.home = os.path.join(self.td, "home")
        self.pack = os.path.join(self.home, ".cys", "pack")   # base 팩 = $HOME/.cys/pack 필수
        self.dirs = os.path.join(self.pack, "directives")
        os.makedirs(self.dirs, exist_ok=True)
        self._env0 = dict(os.environ)
        os.environ["HOME"] = self.home
        for k in pf.PACK_DIR_ENV_KEYS:            # env 팩 오버라이드 전부 제거(기본 경로 강제)
            os.environ.pop(k, None)
        self.addCleanup(self._restore)
        # 역할 3종은 기본 전핀 green(다른 축의 소음 제거 — 각 테스트가 필요 시 덮어쓴다).
        for f in ROLE_FILES:
            self.wf(f, "\n".join(p for p, _ in pf.CONTENT_PINS[f]) + "\n")

    def _restore(self):
        os.environ.clear()
        os.environ.update(self._env0)

    # ── 픽스처 헬퍼 ──
    def wf(self, rel, data):
        """directives/ 상대 경로에 쓰기(바이트 정확 — newline 변형 금지)."""
        p = os.path.join(self.dirs, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8", newline="") as f:
            f.write(data)
        return p

    def wpristine(self, rel, data):
        p = os.path.join(self.pack, ".pristine", "directives", rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8", newline="") as f:
            f.write(data)
        return p

    def run_c03(self, fix=False, skips=()):
        p = pf.Preflight(fix=fix, skips=list(skips))
        p.c03_content_pins()
        return p.results

    def row(self, results, cid):
        rows = [r for r in results if r["id"] == cid]
        self.assertEqual(len(rows), 1,
                         "cid %s 행 %d개(1 기대): %r" % (cid, len(rows), results))
        return rows[0]

    def no_row(self, results, cid):
        self.assertFalse([r for r in results if r["id"] == cid],
                         "cid %s 행이 없어야 한다: %r" % (cid, results))


class StandardMachine(Base):
    """비승격 표준 머신 — 핀 축 ① + 일반 소실 + 원인 분류."""

    def test_all_pass_then_flip(self):
        self.wf("MASTER_DIRECTIVE.md", master_body())
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], PASS)
        self.assertIn("사용자 주권 편집 허용", r["detail"])
        # 비승격 = 건전성 축 미발화(소음 0)
        self.no_row(res, "C03.demote-guard")
        for cid in ("C03.pin.worker", "C03.pin.cso", "C03.pin.reviewer", "C03.pin.ceo"):
            self.assertEqual(self.row(res, cid)["status"], PASS, cid)
        # ★결함 주입 flip: 핀 1개 제거 → FAIL 로 뒤집혀야 감시 능력이 증명된다.
        drop = MASTER_PINS[0][0]
        self.wf("MASTER_DIRECTIVE.md", master_body(drop=(drop,)))
        r2 = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r2["status"], FAIL)
        self.assertIn(MASTER_PINS[0][1], r2["detail"])        # 소실 라벨 명시
        self.assertIn("★복원 절차", r2["detail"])              # 부등(무 pristine) = 현행 문안

    def test_cause_line_vendor_lag_vs_user_edit(self):
        """원인 분류 1줄(R1 시나리오10): live==.pristine → 무수정 배포본(+.new 병치 시
        pack-merge --take-new 안내) / 부등 → 현행 복원 절차 문안 유지."""
        self.wf("MASTER_DIRECTIVE.md", master_body())
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        wpins = pf.CONTENT_PINS["WORKER_DIRECTIVE.md"]
        lag = "\n".join(p for p, _ in wpins[:-1]) + "\n"       # 마지막 핀 부재(벤더 선행)
        self.wf("WORKER_DIRECTIVE.md", lag)
        self.wpristine("WORKER_DIRECTIVE.md", lag)             # live==.pristine(무수정)
        self.wf("WORKER_DIRECTIVE.md.new",
                "\n".join(p for p, _ in wpins) + "\n")         # .new 병치
        r = self.row(self.run_c03(), "C03.pin.worker")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("무수정 배포본", r["detail"])
        # ★0.15.0 M2(설계 v0.5 §2-1·§4-2): 원인 분류는 남에게 수리를 넘기는 문구를 쓰지 않는다(종전 문안의 두 낱말
        #   묶음은 아래 단언이 부재를 확인한다 — 설계가 책임 전가 원인으로 지목한 1530행). 뜻(사용자 탓이 아님)은 유지.
        self.assertIn("사용자 편집이 원인이 아니다", r["detail"])
        for old_phrase in ("벤더 동기", "과실 아님"):
            self.assertNotIn(old_phrase, r["detail"])
        self.assertIn("cys pack-merge --file directives/WORKER_DIRECTIVE.md --take-new",
                      r["detail"])
        self.assertNotIn("★복원 절차", r["detail"])            # 오진 처방 차단
        # flip: 사용자 수정 흔적(live≠.pristine) → 현행 복원 절차 문안으로 복귀.
        self.wf("WORKER_DIRECTIVE.md", lag + "사용자 수정\n")
        r2 = self.row(self.run_c03(), "C03.pin.worker")
        self.assertEqual(r2["status"], FAIL)
        self.assertIn("★복원 절차", r2["detail"])
        self.assertNotIn("무수정 배포본", r2["detail"])

    def test_ceo_template_pin_row(self):
        """C03.pin.ceo — 라이브 템플릿 파일 검사(Wave2 핀 포함). 결함 주입 flip 동반."""
        self.wf("MASTER_DIRECTIVE.md", master_body())
        self.wf("CEO_TEMPLATE.md", tmpl_body())                # 4핀 전부(post-Wave2 형상)
        self.assertEqual(self.row(self.run_c03(), "C03.pin.ceo")["status"], PASS)
        # Wave2 핀 제거(현 repo 출하 형상) → FAIL + 라벨 명시.
        wave2 = CEO_PINS[-1]
        self.wf("CEO_TEMPLATE.md",
                "\n".join(p for p, _ in CEO_PINS[:-1]) + "\nCEO 머리글\n")
        r = self.row(self.run_c03(), "C03.pin.ceo")
        self.assertEqual(r["status"], FAIL)
        self.assertIn(wave2[1], r["detail"])


class PromotedMachine(Base):
    """승격 상태 기계 — 표지 3경로·검사 표면 전환·건전성 축."""

    def test_promoted_byte_equal_then_backup_corruption_flip(self):
        tmpl = tmpl_body()
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", tmpl)                   # md==tmpl 바이트 등가
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], PASS)
        self.assertIn("표준 핀은 MASTER_DIRECTIVE.md.pre-ceo에서 검사", r["detail"])
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], PASS)
        self.assertIn("승격 백업 건전", g["detail"])
        # ★결함 주입 flip(퇴화): .pre-ceo 를 템플릿 사본으로 오염 → 핀 축 FAIL(전용 분류)
        #   + 건전성 축 WARN. '구본화' 오표기 억제는 별도 케이스에서 검증.
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", tmpl)
        res2 = self.run_c03()
        r2 = self.row(res2, "C03.pin.master")
        self.assertEqual(r2["status"], FAIL)
        self.assertIn("승격 백업 파괴", r2["detail"])
        g2 = self.row(res2, "C03.demote-guard")
        self.assertEqual(g2["status"], WARN)
        self.assertIn("퇴화", g2["detail"])

    def test_promoted_legacy_template_warn(self):
        """구판 템플릿 승격 = 정상 WARN + promote-ceo 재실행 안내(위경보 아님 — R2 시나리오6)."""
        self.wf("CEO_TEMPLATE.md", tmpl_body("current"))
        self.wf("MASTER_DIRECTIVE.md", tmpl_body("legacy"))    # 표지 3핀 有·라이브와 부등
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], WARN)
        self.assertIn("정상 승격(구판 템플릿)", r["detail"])
        self.assertIn("promote-ceo", r["detail"])
        self.assertEqual(self.row(res, "C03.demote-guard")["status"], PASS)

    def test_promoted_stale_backup_fail(self):
        """구본화(R1 E1): .pre-ceo 핀 부족 ∧ 동일 핀 전부 .new 에 존재 → 신본 채택 대기 분류."""
        tmpl = tmpl_body()
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", tmpl)
        drop = MASTER_PINS[-1][0]
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body(drop=(drop,)))
        self.wf("MASTER_DIRECTIVE.md.new", master_body())      # 신본에는 존재
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("구본화 — 신본 채택 대기", r["detail"])
        self.assertIn("--keep-mine", r["detail"])              # D1(a)형 절차(역전 금지)
        self.assertNotIn("--take-new", r["detail"])            # 승격 중 MASTER take-new 처방 금지
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], WARN)
        self.assertIn("구본화", g["detail"])

    def test_degenerate_suppresses_stale_classification(self):
        """★연동 규칙(R3 conflicts2): 퇴화 머신에서 .new 병치로 구본화 술어가 참이어도
        '구본화' 분류를 억제하고 '승격 백업 파괴' 전용 분류로 승격한다."""
        tmpl = tmpl_body()
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", tmpl)           # 퇴화(.pre-ceo==템플릿)
        self.wf("MASTER_DIRECTIVE.md.new", master_body())      # 구본화 술어도 참이 되는 병치
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("승격 백업 파괴", r["detail"])
        self.assertNotIn("구본화", r["detail"])                 # 오표기 억제 확인
        self.assertIn(".pristine", r["detail"])                # pristine 재건 안내
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], WARN)
        self.assertIn("승격 백업 파괴", g["detail"])
        self.assertNotIn("구본화", g["detail"])

    def test_backup_missing_fail_and_guard_duplicate(self):
        """④ 사분면: md==템플릿 ∧ .pre-ceo 부재 = 복원 백업 소실 FAIL. 건전성 축의 부재
        WARN 과 중복 발화는 의도(독립 축 — 모순 아닌 중복 신호)."""
        tmpl = tmpl_body()
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", tmpl)
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("복원 백업 소실", r["detail"])
        self.assertIn("강등 불능", r["detail"])
        self.assertIn(".pristine", r["detail"])
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], WARN)
        self.assertIn(".pre-ceo 부재", g["detail"])

    def test_pins_pass_does_not_shadow_guard(self):
        """R2 E2 봉합의 핵심: 핀 축 ①(md 핀 전수 PASS — post-D2 합성본)이 건전성 축을
        차폐하지 않는다 — 승격 표지면 .pre-ceo 소실이 WARN 으로 상시 가시화된다."""
        synth = master_body() + "\n" + "\n".join(p for p, _ in CEO_PINS) + "\n"
        self.wf("CEO_TEMPLATE.md", synth)                      # 합성본(전 핀 포함)
        self.wf("MASTER_DIRECTIVE.md", synth)                  # md==tmpl ∧ ① 전수 PASS
        res = self.run_c03()                                   # .pre-ceo 부재
        self.assertEqual(self.row(res, "C03.pin.master")["status"], PASS)
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], WARN)
        self.assertIn(".pre-ceo 부재", g["detail"])

    def test_anomalous_promotion_fail(self):
        """③ 비정형 승격 상태 — 표지 전부 실패 ∧ .pre-ceo 존재. 파괴 금지·cmp 첨부·
        promote-ceo 분기·'개행 변경도 비정형' 문구(스펙 §D3(ii)①③)."""
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        self.wf("MASTER_DIRECTIVE.md", "오염된 본문 — 표준 핀도 표지 핀도 없음\n")
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        for phrase in ("비정형 승격 상태", "파괴적 조치 금지", "개행 변경도 비정형 판정됨",
                       "cmp", "promote-ceo 재실행"):
            self.assertIn(phrase, r["detail"])
        # ★AH4(a)(T8): 안전 의미는 그대로(보고한 뒤 지시 대기), 벤더 호칭 없이(AH5 와 같은 치환).
        self.assertIn("오너에게 보고한 뒤 지시 대기", r["detail"])
        self.assertNotIn("주인님", r["detail"])
        # ★T8: 조립은 지금 파일을 본문으로 쓰므로 재승격'만'으로 풀린다고 하지 않는다 — 백업을 되돌린 뒤 재승격.
        self.assertIn("되돌린 뒤 promote-ceo 재실행", r["detail"])
        self.no_row(res, "C03.demote-guard")                   # 표지 미검출 = 건전성 축 없음

    def test_receipt_marker_and_stale_receipt(self):
        """영수증 경로(스펙 결정 D3): 해시 등가 = 승격 표지(표지 핀 폴백보다 우선) /
        stale 영수증(부등) = 경고 없이 무시 → 비정형 FAIL."""
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        body = "영수증 전용 승격 본문(표지 핀 없음)\n"
        self.wf("MASTER_DIRECTIVE.md", body)
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        self.wf(".ceo-template-applied", sha256(body.encode("utf-8")) + "\n")
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], WARN)                    # md≠라이브 템플릿 = 구판 취급
        self.assertIn("영수증", r["detail"])
        self.assertNotIn("비정형", r["detail"])
        self.assertEqual(self.row(res, "C03.demote-guard")["status"], PASS)
        # flip: stale 영수증(해시 부등) → 표지 소멸 → 비정형 FAIL.
        self.wf(".ceo-template-applied", sha256(b"other") + "\n")
        r2 = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r2["status"], FAIL)
        self.assertIn("비정형 승격 상태", r2["detail"])


class ComposedMachine(Base):
    """★0.15.0 T8(「스스로 해결」 M14·M15-나 · C11 11.8–11.9 · 설계 §4-14): 조립된 CEO 지침 =
    생성된 머리말 + 구분선 + 사용자 본부 지침. 검사 표면 = 구분선 뒤 본문 · 강등 원본 = 같은 본문 ·
    '지침 ≠ 틀'은 정상(경고·'재적용' 권유 없음) · 영수증(.ceo-receipt.json) 손상은 건전성 축에서 보인다."""

    SEP = "\n---\n\n# [본문 — 표준 MASTER 운영 계약 전문]\n\n"
    HEADER = "\n".join(p for p, _ in CEO_PINS) + "\nCEO 거버넌스 머리글\n"

    def compose(self, body):
        return self.HEADER + self.SEP + body

    def receipt(self, header, body, composed, **over):
        d = {"schema": 1, "header_sha256": sha256(header.encode("utf-8")),
             "master_sha256_at_compose": sha256(body.encode("utf-8")),
             "composed_sha256": sha256(composed.encode("utf-8")), "separator": self.SEP,
             "composed_at": "2026-11-13T00:00:00Z"}
        d.update(over)
        self.wf(".ceo-receipt.json", json.dumps(d, ensure_ascii=False))

    def test_user_edited_body_is_normal_not_drift(self):
        """사용자가 본문을 고친 조립 지침 = PASS(틀과 달라도 경고·'재적용' 권유 없음) · 강등 경로 PASS(구분선 뒤 본문)."""
        body = master_body() + "USERFIX 사용자가 고친 줄\n"
        tmpl = self.compose(master_body())
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", self.compose(body))
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", body)
        self.receipt(self.HEADER, body, self.compose(body))
        res = self.run_c03()
        r = self.row(res, "C03.pin.master")
        self.assertEqual(r["status"], PASS)
        self.assertNotIn("구판 템플릿", r["detail"])
        g = self.row(res, "C03.demote-guard")
        self.assertEqual(g["status"], PASS)
        self.assertIn("구분선 뒤 본문", g["detail"])
        self.assertNotIn("영수증 없음", g["detail"])
        # .pre-ceo 가 없어도 조립된 지침은 강등할 수 있다(종전: '강등 불능' FAIL/WARN).
        os.remove(os.path.join(self.dirs, "MASTER_DIRECTIVE.md.pre-ceo"))
        res2 = self.run_c03()
        self.assertEqual(self.row(res2, "C03.pin.master")["status"], PASS)
        g2 = self.row(res2, "C03.demote-guard")
        self.assertEqual(g2["status"], PASS)
        self.assertNotIn("강등 불능", g2["detail"])

    def test_body_pins_lost_surface_is_the_body(self):
        """본문이 핀을 잃음 → FAIL(검사 표면 = 구분선 뒤 본문) · 재조립을 권하지 않음 · 결함 주입 flip 두 갈래."""
        drop = MASTER_PINS[0][0]
        body = master_body(drop=(drop,))
        self.wf("CEO_TEMPLATE.md", self.compose(master_body()))
        self.wf("MASTER_DIRECTIVE.md", self.compose(body))
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())       # 백업이 온전해도 살아 있는 본문이 기준
        r = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("구분선 뒤 본문에서 검사", r["detail"])
        self.assertIn(MASTER_PINS[0][1], r["detail"])
        self.assertNotIn("promote-ceo", r["detail"], "조립은 본문을 바꾸지 않는다 — 재승격 권유는 오진")
        self.assertIn("★복원 절차", r["detail"])
        # flip ①: 본문 == .pristine(무수정 배포본) → 원인 분류(사용자 탓 아님 · 수리를 남에게 넘기지 않음).
        #   (같은 지문이면 detail 이 축약되므로 — 분류가 같다 — 지문 원장을 비우고 전문을 본다.)
        self.wpristine("MASTER_DIRECTIVE.md", body)
        os.remove(pf.c03_fingerprint_path())
        r2 = self.row(self.run_c03(), "C03.pin.master")
        self.assertIn("사용자 편집이 원인이 아니다", r2["detail"])
        for phrase in ("벤더 동기", "과실 아님", "주인님"):
            self.assertNotIn(phrase, r2["detail"])
        # flip ②: 신본(.new)에 소실 조항 전부 → 신본 채택 대기(워커가 본문에 합침 · take-new 는 A12 거부).
        self.wf("MASTER_DIRECTIVE.md.new", master_body())
        os.remove(pf.c03_fingerprint_path())
        r3 = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r3["status"], FAIL)
        self.assertIn("신본 채택 대기", r3["detail"])
        self.assertIn("A12", r3["detail"])

    def test_receipt_and_body_health_in_guard_axis(self):
        body = master_body()
        self.wf("CEO_TEMPLATE.md", self.compose(body))
        self.wf("MASTER_DIRECTIVE.md", self.compose(body))
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", body)
        # 영수증 없음(0.15.0 이전 승격) — PASS + 다음 확인 때 기록한다는 주석.
        g = self.row(self.run_c03(), "C03.demote-guard")
        self.assertEqual(g["status"], PASS)
        self.assertIn("영수증 없음", g["detail"])
        # 영수증 손상 → WARN(승격 확인이 성공으로 보고되지 않음).
        self.wf(".ceo-receipt.json", "{broken")
        g2 = self.row(self.run_c03(), "C03.demote-guard")
        self.assertEqual(g2["status"], WARN)
        self.assertIn("영수증", g2["detail"])
        # 구분선 뒤가 비었다 → 강등 경로 이상 WARN(핀 축은 별도로 FAIL).
        os.remove(os.path.join(self.dirs, ".ceo-receipt.json"))
        self.wf("MASTER_DIRECTIVE.md", self.HEADER + self.SEP)
        g3 = self.row(self.run_c03(), "C03.demote-guard")
        self.assertEqual(g3["status"], WARN)
        self.assertIn("강등 경로 이상", g3["detail"])

    def test_backup_holding_a_composition_is_degenerate(self):
        """백업에 조립된 지침(구분선)이 들어갔다 = 퇴화(백업은 조립되지 않은 MASTER 여야 한다)."""
        body = master_body()
        legacy_md = tmpl_body("legacy")                          # 구분선 없는 옛 형상(표지 3핀)
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        self.wf("MASTER_DIRECTIVE.md", legacy_md)
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", self.compose(body))
        g = self.row(self.run_c03(), "C03.demote-guard")
        self.assertEqual(g["status"], WARN)
        self.assertIn("퇴화", g["detail"])

    def test_legacy_backup_pins_lost_cause_has_no_vendor_deferral(self):
        """AH4(a): 옛 형상의 '무수정 배포본' 원인 문장 — T1b 의 문안(사용자 탓 아님 · 수리를 남에게 넘기지 않음)."""
        tmpl = tmpl_body()
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md", tmpl)
        lag = master_body(drop=(MASTER_PINS[-1][0],))
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", lag)
        self.wpristine("MASTER_DIRECTIVE.md", lag)
        r = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r["status"], FAIL)
        self.assertIn("사용자 편집이 원인이 아니다", r["detail"])
        self.assertIn("오너 지시로 신본을 받는다", r["detail"])
        for phrase in ("벤더 동기", "과실 아님"):
            self.assertNotIn(phrase, r["detail"])


class FingerprintAndContracts(Base):
    """상태 지문 dedupe · --fix 무변형 · dept 면제 선행."""

    def test_fingerprint_dedupe_and_change(self):
        self.wf("CEO_TEMPLATE.md", tmpl_body("current"))
        self.wf("MASTER_DIRECTIVE.md", tmpl_body("legacy"))
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        r1 = self.row(self.run_c03(), "C03.pin.master")        # 1회차 = 전문
        self.assertNotIn("[지문 동일 — 축약]", r1["detail"])
        self.assertIn("promote-ceo", r1["detail"])
        fp_path = pf.c03_fingerprint_path()
        self.assertTrue(os.path.isfile(fp_path), "지문 원장 미기록: %s" % fp_path)
        with open(fp_path, encoding="utf-8") as f:
            rec = json.load(f)
        self.assertEqual(len(rec["fingerprint"]), 4)           # md·분류·pre·guard-bits 4항
        # 2회차(동일 상태) = detail 1줄 축약(status 는 불변).
        res2 = self.run_c03()
        r2 = self.row(res2, "C03.pin.master")
        self.assertEqual(r2["status"], WARN)
        self.assertIn("[지문 동일 — 축약]", r2["detail"])
        g2 = self.row(res2, "C03.demote-guard")
        self.assertIn("[지문 동일 — 축약]", g2["detail"])
        # 상태 전이① (.pre-ceo 해시만 변화 — 분류 불변): 지문 성분이라 축약을 뚫고 전문 재발화.
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body() + "오너 추가 조항\n")
        r3 = self.row(self.run_c03(), "C03.pin.master")
        self.assertEqual(r3["status"], WARN)
        self.assertNotIn("[지문 동일 — 축약]", r3["detail"])
        self.assertIn("promote-ceo", r3["detail"])
        # 상태 전이② (.pre-ceo 소실 — 폴백 표지까지 사멸·분류 전이): 즉시 전문 재발화.
        #   md 는 표지 3핀뿐(구판 템플릿)이라 marker 소멸 → 일반 소실 FAIL(pins-fail)로 전이.
        os.remove(os.path.join(self.dirs, "MASTER_DIRECTIVE.md.pre-ceo"))
        res4 = self.run_c03()
        r4 = self.row(res4, "C03.pin.master")
        self.assertEqual(r4["status"], FAIL)
        self.assertNotIn("[지문 동일 — 축약]", r4["detail"])
        self.assertIn("★복원 절차", r4["detail"])
        self.no_row(res4, "C03.demote-guard")                  # 표지 소멸 = 건전성 축도 소멸

    def test_fix_never_mutates_c03_surfaces(self):
        """C03 무-fix 계약(스펙 §D3(ii)2 · R1 시나리오9): --fix 로 돌려도 디렉티브 표면
        (md·tmpl·.pre-ceo·.new·영수증)의 바이트가 1도 변하지 않고 FIXED 도 없다."""
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        self.wf("MASTER_DIRECTIVE.md", "오염된 본문\n")          # 비정형 FAIL 상태
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        self.wf(".ceo-template-applied", sha256(b"stale") + "\n")
        snap = {}
        for root, _, files in os.walk(self.dirs):
            for fn in files:
                p = os.path.join(root, fn)
                with open(p, "rb") as f:
                    snap[p] = f.read()
        res = self.run_c03(fix=True)
        for p, before in snap.items():
            with open(p, "rb") as f:
                self.assertEqual(f.read(), before, "--fix 가 %s 를 변형" % p)
        self.assertFalse([r for r in res if r["status"] == pf.FIXED],
                         "C03 에 FIXED 발생: %r" % res)

    def test_dept_pack_exemption_precedes_everything(self):
        """dept 면제 early-return 선행 불변(F1) — 승격 로직·지문 어느 것도 발화하지 않는다."""
        other = os.path.join(self.td, "pack-dept-x")
        os.makedirs(os.path.join(other, "directives"), exist_ok=True)
        os.environ["CYS_PACK_DIR"] = other
        res = self.run_c03()
        self.assertEqual(len(res), 1, "면제는 단일 행: %r" % res)
        self.assertEqual(res[0]["id"], "C03.pin")
        self.assertEqual(res[0]["status"], WARN)
        self.assertIn("면제", res[0]["detail"])
        self.assertFalse(os.path.isfile(pf.c03_fingerprint_path()),
                         "dept 면제인데 지문 원장이 기록됨")


class BootContractDelivery(Base):
    """[배달 축] C03.boot-contract — §0-A session_error 행(P0-3 래치 소비면)의 **도달** 관측.

    R3-DELIVERY-1: `directives/*_DIRECTIVE.md` 는 `pack.rs ownership()` 상 User 라 팩 갱신이
    본문을 덮지 않는다(신본은 `<rel>.new` 병치 + `cys pack-merge`). 그래서 §0-A 개정은 기존
    설치본에 자동 도달하지 않는데 그 행을 소비하도록 안내하는 훅은 System 등급이라 전원에게
    도달한다 — 결손이 관측되지 않으면 '재실행 금지 vs 1회 재실행'의 이중 진실이 조용히 남는다.
    이 축은 **WARN 고정**이다(병합 대기 ≠ 파손 · 기계 거동은 자기완결 훅 브리지가 유지) —
    FAIL 로 승격하면 병합 전 전 함대가 상시 NOT READY 가 된다."""

    ROW = "state:session_error 이고 result.retry_eligible:true\n"

    def test_present_passes(self):
        self.wf("MASTER_DIRECTIVE.md", master_body() + self.ROW)
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        r = self.row(self.run_c03(), "C03.boot-contract")
        self.assertEqual(r["status"], PASS)
        self.assertIn("session_error", r["detail"])

    def test_absent_warns_with_merge_command(self):
        """결손(구 디렉티브) → WARN + 해소 명령. 부트를 막지 않는 등급이어야 한다."""
        self.wf("MASTER_DIRECTIVE.md", master_body())      # 캠페인 이전 §0-A(행 부재)
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        self.wf("MASTER_DIRECTIVE.md.new", master_body() + self.ROW)   # vendor 신본 병치
        r = self.row(self.run_c03(), "C03.boot-contract")
        self.assertEqual(r["status"], WARN, "배달 축이 WARN 이 아니다: %r" % r)
        self.assertIn("retry_eligible", r["detail"])
        self.assertIn("pack-merge", r["detail"])
        self.assertIn("MASTER_DIRECTIVE.md.new", r["detail"])
        # 핀 축은 이 결손에 **무영향**(독립 축 — 배달 결손이 부트 준비 판정을 볼모로 잡지 않는다)
        self.assertEqual(self.row(self.run_c03(), "C03.pin.master")["status"], PASS)

    def test_absent_without_new_still_warns(self):
        """병치본조차 없는 기계(구설치본) — 처방은 init-pack→pack-merge 로 갈린다."""
        self.wf("MASTER_DIRECTIVE.md", master_body())
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        r = self.row(self.run_c03(), "C03.boot-contract")
        self.assertEqual(r["status"], WARN)
        self.assertIn("init-pack", r["detail"])

    def test_promoted_machine_gets_promotion_path(self):
        """승격 기계(md=CEO 템플릿 사본)의 결손은 `--take-new` 가 아니라 재승격 경로다
        (A12 가드 동형 — 승격 중 MASTER 에 take-new 를 처방하면 CEO 문면이 파괴된다)."""
        tmpl = tmpl_body()
        self.wf("MASTER_DIRECTIVE.md", tmpl)      # 바이트 등가 = 승격 표지
        self.wf("CEO_TEMPLATE.md", tmpl)
        self.wf("MASTER_DIRECTIVE.md.pre-ceo", master_body())
        r = self.row(self.run_c03(), "C03.boot-contract")
        self.assertEqual(r["status"], WARN)
        self.assertIn("promote-ceo", r["detail"])
        self.assertNotIn("take-new", r["detail"])

    def test_unreadable_is_not_a_pass(self):
        """판독 불가 = 판정 불가(WARN) — 측정 불능을 초록으로 접지 않는다."""
        p = self.wf("MASTER_DIRECTIVE.md", master_body())
        with open(p, "wb") as f:
            f.write(b"\xff\xfe\x00")              # 비UTF-8 — errors='replace' 로도 행은 부재
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        r = self.row(self.run_c03(), "C03.boot-contract")
        self.assertNotEqual(r["status"], PASS)

    def test_dept_pack_exempt(self):
        """dept/CEO 팩은 커스텀 디렉티브가 정상 — 배달 축도 발화하지 않는다(면제 선행)."""
        other = os.path.join(self.td, "pack-dept-y")
        os.makedirs(os.path.join(other, "directives"), exist_ok=True)
        os.environ["CYS_PACK_DIR"] = other
        self.no_row(self.run_c03(), "C03.boot-contract")

    def test_skip_is_honored(self):
        """--skip 은 판정을 건너뛰되 **SKIP 행으로 가시화**한다(조용한 미측정 금지 — 다른
        축과 동일 관용)."""
        self.wf("MASTER_DIRECTIVE.md", master_body())
        self.wf("CEO_TEMPLATE.md", tmpl_body())
        r = self.row(self.run_c03(skips=["C03.boot-contract"]), "C03.boot-contract")
        self.assertEqual(r["status"], SKIP)


class SoulC04(Base):
    """C04.soul — ★0.15.0 M2(설계 v0.5 §4-2 · 부록 A 1절 35번 · 시험 계획 H-02-A1-35-01).
    이 파일에 둔 이유: 픽스처 팩 하네스($HOME/.cys/pack)를 이미 갖췄고 CI 세 단계에 등록돼 있다.
    지키는 것: 점검 도구는 soul.md 를 **쓰지 않는다** — 오너가 지운 자율 조항을 되살리지 않고(35번), 호칭 줄을
    되써넣지 않는다(벤더 개인 호칭이 모든 사용자 설치에 되살아나는 경로 · 규칙 문서를 주인 지시 없이 쓰는 경로)."""

    OWNER_EDITED = ("# soul.md — 운영 헌장\n\n## 정체\n\n- 오너의 이름: 김사용\n"
                    "- master의 역할: 단일 지휘 노드\n\n## 금지선 (denylist — 오너 승인 없이 절대 금지)\n\n"
                    "- 외부 발행\n")   # 오너가 호칭 줄과 자율주행 절을 둘 다 지웠다

    def soul(self):
        return os.path.join(self.pack, "soul.md")

    def write_soul(self, text):
        with open(self.soul(), "w", encoding="utf-8", newline="") as f:
            f.write(text)

    def run_c04(self, fix):
        p = pf.Preflight(fix=fix, skips=[])
        p.c04_soul()
        return p.results

    def test_fix_twice_leaves_owner_edited_soul_byte_identical(self):
        """H-02-A1-35-01: --fix 두 번 → 오너가 지운 자율 조항도, 호칭 줄도 다시 생기지 않는다(바이트 동일)."""
        self.write_soul(self.OWNER_EDITED)
        before = open(self.soul(), "rb").read()
        for _ in range(2):
            r = self.row(self.run_c04(fix=True), "C04.soul")
            self.assertIn(r["status"], (PASS, WARN), r)
            self.assertNotEqual(r["status"], pf.FIXED, r)
        after = open(self.soul(), "rb").read()
        self.assertEqual(before, after, "점검 도구가 soul.md 를 썼다(되써넣기 회귀)")
        self.assertNotIn(pf.SOUL_AUTOPILOT_MARKER.encode("utf-8"), after)
        self.assertNotIn("주인님".encode("utf-8"), after)

    def test_missing_honorific_is_not_a_failure(self):
        """호칭 줄 부재는 FAIL 이 아니다(종전: FAIL + --fix 로 특정 호칭 보강) — 관측 문구만 남긴다."""
        self.write_soul(self.OWNER_EDITED)
        r = self.row(self.run_c04(fix=False), "C04.soul")
        self.assertNotEqual(r["status"], FAIL, r)
        self.assertIn("오너 호칭", r["detail"])
        self.assertNotIn("주인님", r["detail"])

    def test_shipped_template_owner_identity_first_and_neutral(self):
        """배포 틀 「정체」의 첫 줄 = 오너는 설치한 사용자·제작자는 주인이 아님·승인은 대화 중인 사용자에게(설계 §4-2).
        호칭 표지는 있되 특정 호칭은 없다. 그 틀로는 C04 가 PASS/WARN 이고 바이트를 바꾸지 않는다."""
        tpl = os.path.join(BIN, os.pardir, "soul.md")
        with open(tpl, encoding="utf-8") as f:
            text = f.read()
        ident = text[text.index("## 정체"):]
        first = ident.split("\n- ", 1)[1]
        self.assertTrue(first.startswith("**이 자비스의 주인(이하 \"오너\")은 이것을 설치해 쓰고 있는 사용자다."), first[:80])
        self.assertIn("제작자는 주인이 아니다.", first)
        self.assertIn(pf.SOUL_MARKER, text)
        self.assertNotIn("주인님", text)
        self.assertNotIn(b"\r", open(tpl, "rb").read())
        self.write_soul(text)
        before = open(self.soul(), "rb").read()
        r = self.row(self.run_c04(fix=True), "C04.soul")
        self.assertIn(r["status"], (PASS, WARN), r)
        self.assertEqual(open(self.soul(), "rb").read(), before)

    def test_c04_source_has_no_soul_write(self):
        """구조 핀: c04_soul 본문에 파일 쓰기가 없다(되써넣기 분기를 되살리면 적색 — 위 바이트 시험과 이중)."""
        import inspect
        src = inspect.getsource(pf.Preflight.c04_soul)
        for bad in ('open(p, "w"', "open(p,'w'", ".write(", "SOUL_APPEND", "주인님"):
            self.assertNotIn(bad, src, "c04_soul 에 %r 가 돌아왔다" % bad)


if __name__ == "__main__":
    unittest.main(verbosity=2)
