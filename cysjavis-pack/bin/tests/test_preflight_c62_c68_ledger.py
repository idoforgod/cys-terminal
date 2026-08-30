#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_c62_c68_ledger.py — C62(원장 가시화)·C68(체류 기한) 병합 원장 판정 회귀 하네스.

★왜 이 파일이 존재하는가(0.14.29 성찰 2R-6 · 무측정 변경 봉인):
  T5 는 C62 에 conflicted/quarantined 신설 판정·at-rest 병기(javis_preflight.py c62)를,
  T3 는 C68 에 영속 kind 기한 제외 산식(_exempt)을 더했는데 tests/ 에는 c62·c68 검체가
  0건이었다(grep 실측 — run_bootstrap_health h_* 135검체 포함 전무). 사고 시정 채널(C62)의
  신 충돌 클래스 판정과 wakeup 스팸 차단 산식(C68)이 전부 기계 무검증 — Rust 쪽 42본 핀과
  대조되는 비대칭. 여기서 픽스처 원장으로 상태 전수를 박제한다.

★계측 타당성(test_preflight_c03_states·phase1 관용): "PASS 가 나온다"는 하네스는 아무것도
  증명하지 않는다 — kind·state·ts 를 바꿔 판정이 실제로 뒤집히는지(PASS↔WARN)를 함께 잰다.

라이브 무접촉: 픽스처 팩(임시 디렉터리)을 CYS_PACK_DIR 로 주입(pack_dir() 1순위 키 —
javis_preflight.PACK_DIR_ENV_KEYS 실측). 쓰기는 픽스처 안에서만. --fix 는 켜지 않는다
(Preflight(fix=False) — C68 의 wakeup enqueue 는 fix 전용이라 report 모드에서 관찰만).

    python3 cysjavis-pack/bin/tests/test_preflight_c62_c68_ledger.py
"""
import json
import os
import shutil
import sys
import tempfile
import time
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402 — 형제 모듈 직접 구동(서브프로세스 아님)

PASS, WARN, FAIL, SKIP = pf.PASS, pf.WARN, pf.FAIL, pf.SKIP

DAY = 86400.0


class Base(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.mkdtemp(prefix="pf-c62c68-")
        self.addCleanup(shutil.rmtree, self.td, True)
        self.pack = os.path.join(self.td, "pack")
        os.makedirs(self.pack, exist_ok=True)
        self._env0 = dict(os.environ)
        for k in pf.PACK_DIR_ENV_KEYS:  # 다른 팩 env 오버라이드 전부 제거(결정론)
            os.environ.pop(k, None)
        os.environ["CYS_PACK_DIR"] = self.pack
        os.environ.pop("CYS_MERGE_PENDING_MAX_DAYS", None)  # 기본 14일 고정
        self.addCleanup(self._restore)

    def _restore(self):
        os.environ.clear()
        os.environ.update(self._env0)

    def ledger(self, entries):
        """픽스처 원장 기록 — entries: {rel: {kind, ...}}. ts 기본 = 지금."""
        now = time.time()
        out = {}
        for rel, spec in entries.items():
            e = dict(spec)
            e.setdefault("ts", now)
            out[rel] = e
        with open(os.path.join(self.pack, ".merge-pending.json"), "w",
                  encoding="utf-8") as f:
            json.dump(out, f, ensure_ascii=False)

    def check(self, method, cid_prefix):
        """검사 1개를 돌리고 (status, detail) 반환 — 결과 1건만 나오는지도 함께 확인."""
        p = pf.Preflight(fix=False, skips=[])
        getattr(p, method)()
        rows = [r for r in p.results if r["id"].startswith(cid_prefix)]
        self.assertEqual(len(rows), 1, "결과 행 %d개(1 기대): %r" % (len(rows), p.results))
        return rows[0]["status"], rows[0]["detail"]

    def c62(self):
        return self.check("c62_pack_heal_ledger", "C62")

    def c68(self):
        return self.check("c68_merge_pending_age", "C68")


# ══ C62 — 신 충돌 클래스(conflicted·quarantined) 가시화 + at-rest 병기 ═══════════
class TestC62(Base):
    def test_no_ledger_and_atrest_only_pass(self):
        """원장 부재 = PASS. kept-drift·merged 단독(at-rest 보존 상태) = PASS 유지 —
        WARN 판정 집합 비대상 계약(발동 조건 불변)의 양방향 확인."""
        st, detail = self.c62()
        self.assertEqual(st, PASS, detail)
        self.ledger({
            "bin/a.py": {"kind": "kept-drift"},
            "bin/b.py": {"kind": "merged"},
        })
        st2, detail2 = self.c62()
        self.assertEqual(st2, PASS, "at-rest kind 단독은 PASS 여야 함: %s" % detail2)

    def test_conflicted_warns_with_sidecar_guidance(self):
        """신설 판정 ①: conflicted → WARN + vendor 적용·.user·.base 보존 안내 + rel 노출."""
        self.ledger({"hooks/x.sh": {"kind": "conflicted", "reason": "json-gate"}})
        st, detail = self.c62()
        self.assertEqual(st, WARN, detail)
        self.assertIn("병합 충돌(conflicted)", detail)
        self.assertIn("hooks/x.sh", detail)
        self.assertIn(".base", detail)

    def test_conflicted_at_rest_annotated_not_exempt(self):
        """conflicted 의 state=at-rest 는 '제외'가 아니라 '(at-rest)' 병기다(master 결정 자구)."""
        self.ledger({"hooks/y.sh": {"kind": "conflicted", "state": "at-rest"}})
        st, detail = self.c62()
        self.assertEqual(st, WARN, "at-rest conflicted 도 판정 집합 유지: %s" % detail)
        self.assertIn("hooks/y.sh (at-rest)", detail)

    def test_quarantined_warns(self):
        """신설 판정 ②: quarantined → WARN + 무접촉 보존 안내."""
        self.ledger({"bin/q.py": {"kind": "quarantined", "reason": "backup-failed"}})
        st, detail = self.c62()
        self.assertEqual(st, WARN, detail)
        self.assertIn("격리(quarantined)", detail)
        self.assertIn("bin/q.py", detail)

    def test_atrest_info_annexed_only_when_warn_fires(self):
        """at-rest 정보 병기는 WARN 발동 시 상세에만 붙는다(단독 존재 = PASS 는 위에서 검증)."""
        self.ledger({
            "hooks/h.sh": {"kind": "healed"},
            "bin/k1.py": {"kind": "kept-drift"},
            "bin/k2.py": {"kind": "kept-drift"},
            "bin/m1.py": {"kind": "merged"},
        })
        st, detail = self.c62()
        self.assertEqual(st, WARN, detail)
        self.assertIn("원복(healed) 1건", detail)
        self.assertIn("kept-drift 2건", detail)
        self.assertIn("merged 1건", detail)
        self.assertIn("조치 불요 보존 상태", detail)

    def test_parse_failure_warns(self):
        """원장 파손 = 침묵 통과가 아니라 WARN(수동 확인 안내)."""
        with open(os.path.join(self.pack, ".merge-pending.json"), "w",
                  encoding="utf-8") as f:
            f.write("NOT-JSON{{{")
        st, detail = self.c62()
        self.assertEqual(st, WARN, detail)
        self.assertIn("파싱 실패", detail)


# ══ C68 — 영속 kind 기한 제외(_exempt) 산식 ═══════════════════════════════════
class TestC68(Base):
    def old(self, days=30.0):
        return time.time() - days * DAY

    def test_persistent_kinds_exempt_from_age(self):
        """kept-drift·merged 는 기한 개념이 없다(체류 = 정상) — 30일 체류도 PASS +
        제외 계상 문구. 수리 전 산식이면 fingerprint 일일 갱신 → wakeup 스팸."""
        self.ledger({
            "bin/a.py": {"kind": "kept-drift", "ts": self.old()},
            "bin/b.py": {"kind": "merged", "ts": self.old()},
        })
        st, detail = self.c68()
        self.assertEqual(st, PASS, detail)
        self.assertIn("영속 kind 기한 제외 2건", detail)

    def test_conflicted_keeps_deadline(self):
        """conflicted(비 at-rest)는 조치 가능(actionable) — 기한 유지 = 초과 시 WARN."""
        self.ledger({"hooks/x.sh": {"kind": "conflicted", "ts": self.old()}})
        st, detail = self.c68()
        self.assertEqual(st, WARN, detail)
        self.assertIn("기한(14일) 초과 1건", detail)
        self.assertIn("hooks/x.sh", detail)
        # report 모드 = 관찰만(큐 적재는 --fix 전용) — 가역 부작용 0 계약.
        self.assertIn("관찰만", detail)

    def test_conflicted_at_rest_exempt(self):
        """state=at-rest 가 붙은 conflicted(revert-merge 등 보존 상태)는 기한 제외."""
        self.ledger({
            "hooks/y.sh": {"kind": "conflicted", "state": "at-rest", "ts": self.old()},
        })
        st, detail = self.c68()
        self.assertEqual(st, PASS, detail)
        self.assertIn("영속 kind 기한 제외 1건", detail)

    def test_adopted_exempt_from_age(self):
        """★0.14.29 성찰 차단 수리(T4 신 kind 분류 누락): adopted(복권 확정 — 다음 스윕에
        kept-drift 정규화)는 워커가 할 일이 없는 스윕 대기 정상 체류다. 수리 전 실측:
        adopted 15일 원장 → C62 PASS '병합 대기 0건' ∧ C68 WARN '기한(14일) 초과 1건'
        비대칭 + fingerprint 일일 갱신 = wakeup 일일 재배달. 수리 후 = 양쪽 PASS.
        (기존 11검체에 adopted 0건이던 격차의 봉합 픽스처.)"""
        self.ledger({"hooks/x.sh": {"kind": "adopted", "side": "hooks/x.sh",
                                    "ts": self.old(15.0)}})
        st62, detail62 = self.c62()
        self.assertEqual(st62, PASS, "adopted 는 C62 판정 집합 비대상 유지: %s" % detail62)
        st, detail = self.c68()
        self.assertEqual(st, PASS, "adopted 15일 체류는 기한 제외여야 함: %s" % detail)
        self.assertIn("영속 kind 기한 제외 1건", detail)
        # 계측 타당성(판정 뒤집힘): 같은 픽스처에서 kind 만 healed 로 바꾸면 WARN — 제외가
        # kind 판별에서 나오는지(전면 PASS 하네스가 아닌지)를 잰다.
        self.ledger({"hooks/x.sh": {"kind": "healed", "side": "hooks/x.sh.user",
                                    "ts": self.old(15.0)}})
        st2, detail2 = self.c68()
        self.assertEqual(st2, WARN, "kind 치환이 WARN 으로 뒤집히지 않음: %s" % detail2)
        self.assertIn("기한(14일) 초과 1건", detail2)

    def test_actionable_within_deadline_pass_and_flip(self):
        """healed 신선(1일) = PASS → 같은 항목 ts 만 30일로 되감으면 WARN — 판정이 실제로
        뒤집히는지(계측 타당성)."""
        self.ledger({"hooks/h.sh": {"kind": "healed", "ts": self.old(1.0)}})
        st, detail = self.c68()
        self.assertEqual(st, PASS, detail)
        self.ledger({"hooks/h.sh": {"kind": "healed", "ts": self.old(30.0)}})
        st2, detail2 = self.c68()
        self.assertEqual(st2, WARN, "ts 되감기가 WARN 으로 뒤집히지 않음: %s" % detail2)

    def test_mixed_ledger_counts_only_actionable(self):
        """혼합 원장: 영속 2 + 초과 actionable 1 → WARN 은 1건만 계상(제외 산식의 합성 검증)."""
        self.ledger({
            "bin/a.py": {"kind": "kept-drift", "ts": self.old()},
            "bin/b.py": {"kind": "merged", "ts": self.old()},
            "hooks/x.sh": {"kind": "new-pending", "ts": self.old()},
        })
        st, detail = self.c68()
        self.assertEqual(st, WARN, detail)
        self.assertIn("초과 1건", detail)
        self.assertIn("hooks/x.sh", detail)


# ══ kind 전수 행동 census — MERGE_LEDGER_KINDS(정본 미러) ↔ C62·C68 분류 완전성 ═════
class TestKindCensus(Base):
    """★0.14.29 성찰 차단 수리(계상 SOT): 등재 kind 전수의 (C62, C68@30일) 판정을 박제한다.
    기대표 키 집합 == MERGE_LEDGER_KINDS 를 강제하므로, 신 kind 가 정본(src/pack.rs
    LEDGER_KINDS → 2언어 census)을 거쳐 들어오면 여기서 분류 미결정으로 멈춘다 —
    'adopted 분류 누락 → wakeup 일일 재배달' 사각의 구조 봉인."""

    # kind → (C62 판정, C68 판정 @ 체류 30일). C62: 판정 집합 = healed·new-pending·
    # conflicted·quarantined(가시화 대상)만 WARN. C68: C68_EXEMPT_KINDS + state:at-rest 만
    # 기한 제외.
    EXPECTED = {
        "healed": (WARN, WARN),
        "new-pending": (WARN, WARN),
        "kept-drift": (PASS, PASS),
        "merged": (PASS, PASS),
        "conflicted": (WARN, WARN),
        "quarantined": (WARN, WARN),
        "adopted": (PASS, PASS),
    }

    def test_every_registered_kind_is_classified(self):
        self.assertEqual(set(self.EXPECTED), set(pf.MERGE_LEDGER_KINDS),
                         "기대표 ≠ 등재 kind 집합 — 신 kind 의 C62/C68 분류를 여기서 결정하라")
        for kind, (want62, want68) in sorted(self.EXPECTED.items()):
            self.ledger({"f/%s.txt" % kind: {"kind": kind, "ts": time.time() - 30.0 * DAY}})
            st62, d62 = self.c62()
            self.assertEqual(st62, want62, "C62[%s]: %s" % (kind, d62))
            st68, d68 = self.c68()
            self.assertEqual(st68, want68, "C68[%s]: %s" % (kind, d68))


if __name__ == "__main__":
    unittest.main(verbosity=2)
