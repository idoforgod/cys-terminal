#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_seat_latch_negation.py — W-B1 좌석 판정 3겹 검체(래치 부정 · 부트경로 unknown=결손 · status timeout leaf).

★무엇을 봉인하는가(감사 blocker · 2026-08-21):
  `node_liveness` 의 `awakened_at` 래치는 영속·단방향이라, 한 번 각성한 좌석은 그 안의
  에이전트가 죽어도 영원히 awake_confirmed 로 읽혔다 → 결손 0 → `cys boot` 생략(BOOT_SKIP)
  → ⑤check 도 같은 술어라 통과 → **죽은 좌석 위에서 "기동 완료" 거짓 성공**.

★수리의 안전 계약(이 파일의 존재 이유 절반):
  래치 부정은 cys.rs `seat_death_confirmed` 가 이미 구현한 **3중 AND 를 그대로 재사용**한다
  (신술어 발명 금지): ⓐ seat=="empty"(커널 확정 — unknown·필드부재 트리거 금지)
  ∧ ⓑ agent_alive **is False**(명시적 false 만 — daemon agent_alive 는 3상 true/false/null 이고
  claim-role 관측 등록이 #[cfg(unix)] 라 **Windows master 는 null 이 영구**다. 낙관적 falsy 구현이면
  Windows master 래치가 매 check 무효화 → 결손 오판 → node-recover 가 살아있는 입력창에 주입 →
  치명 앵커 ④ 전 pane 사망) ∧ ⓒ 좌석 나이 > readiness 예산(기동 중 레이스 방지).
  → 그래서 이 검체의 핵심 단언은 "**agent_alive: null 에서 래치가 유지되는가**"다.

★파리티(python ↔ Rust): 판정 이원화(A1·B3 클래스)가 이 저장소의 반복 결함 계보다.
  python `latch_death_confirmed`/`node_liveness` 와 Rust `seat_death_confirmed`/`seat_liveness` 가
  같은 좌석에 같은 사실을 내는지를 ①행동 배터리(python 측 실행) ②소스 배선 핀(Rust 측 —
  래치 분기가 seat_death_confirmed 를 소비하는가) ③Rust 배터리 존재 핀(cys.rs 의
  `mod seat_latch_negation_tests` 가 같은 4상 표를 실행) ④ⓒ임계 수치 파리티(양쪽 60s)로 잰다.
  Rust 소스 부재(배포 팩 단독 설치)면 Rust 핀만 skip — python 행동 검증은 항상 돈다.

★W-B1 ③(부트 경로 unknown=결손): `javis_orchestra._shared_verdict_deficit` 는 unknown 등급을
  시한부 해소(resolve_unknown_for_spawn — `cys boot` 스폰 경로와 동일 규약) 후 잔존 시 결손으로
  계상하되, **⑤check 의 satisfied 는 불변**이어야 한다(같은 함수에 넣으면 콜드스타트 창에서
  exit 6 라이브락 — 감사 확정). 두 사실을 함께 단언한다.

★W-B1 ④(status timeout leaf): javis_boot_node.cys_status 의 `timeout=12` 하드코딩이 budget()
  leaf `CYS_STATUS_TIMEOUT_S` 배선으로 승격됐는가 — 소스 핀 + 행동(run 주입 캡처) 양쪽으로 잰다.

순수 픽스처·서브프로세스 0·라이브 데몬 무접촉·stdlib 만. 실행:
    python3 cysjavis-pack/bin/tests/test_seat_latch_negation.py
"""
import os
import re
import sys
import time
import unittest

# 로케일 비의존 출력(W-A4 선례): cp949 파이프 캡처에서 한글 진단 UnicodeEncodeError 즉사 방지.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.dirname(TESTS_DIR)
PACK_DIR = os.path.dirname(BIN_DIR)
REPO_DIR = os.path.dirname(PACK_DIR)          # 레포 체크아웃일 때만 유효(배포 팩은 skip 분기)
CYS_RS = os.path.join(REPO_DIR, "src", "bin", "cys.rs")
if BIN_DIR not in sys.path:
    sys.path.insert(0, BIN_DIR)

import javis_boot_node as BN      # noqa: E402
import javis_orchestra as O       # noqa: E402


def fx(rows):
    """status fixture — surfaces 배열만(데몬 왕복 0 · run_bootstrap_health `_fx` 와 동형)."""
    return {"surfaces": rows}


def dead_seat(**over):
    """죽음 3중 확정(전부 참) 기준 픽스처 — 각 케이스는 여기서 **한 항씩만** 부정한다
    (실패 시 어느 항의 회귀인지 즉시 귀속 · Rust 검체 `dead_seat()` 와 같은 형상)."""
    base = {"role": "cso", "exited": False, "awakened_at": 1000.0,
            "seat": "empty", "agent_alive": False,
            # readiness 예산(60s) 대비 60배 마진 — 벽시계 틱에 무관한 결정론
            "created_at": time.time() - 3600.0}
    base.update(over)
    return base


class LatchNegationTripleAnd(unittest.TestCase):
    """요구 검증 표: 3중 AND 각 항을 개별로 거짓으로 만든 3케이스 + 전부 참 1케이스."""
    maxDiff = None

    # ── CASE-ALL: 전부 참 → 래치 부정 → absent(회수 대상) ──
    def test_case_all_true_negates_latch(self):
        st = fx([dead_seat()])
        grade, why = BN.node_liveness(st, "cso")
        self.assertEqual(grade, BN.LIVENESS_ABSENT,
                         "죽음 3중 확정 좌석이 여전히 awake_confirmed — blocker 미수리: %s" % why)
        self.assertIn("래치 부정", why, "부정 사유가 판정 이유에 남지 않음(진단 불가): %s" % why)
        # boot_node PRE-CHECK 게이트(already_up 거짓 성공)도 같은 술어로 닫혔는가
        ready, rwhy = BN.awake_ready(st, "cso")
        self.assertFalse(ready, "PRE-CHECK 가 죽은 좌석을 already_up 으로 보고(거짓 성공): %s" % rwhy)
        # 회수 마비 해소: node_alive False → reclaim 의 kill 판정이 열린다(pid 일치 시)
        self.assertFalse(BN.node_alive(st, "cso"), "죽음 확정 좌석이 node_alive=True(회수 영구 마비)")
        self.assertEqual(BN._reclaim_verdict(st, "cso", 100, 100), "kill",
                         "죽음 확정 좌석 회수 불가(막힌 좌석 영구화)")

    # ── CASE-A: ⓐ 거짓(seat != empty) → 래치 유지 ──
    def test_case_a_seat_not_empty_holds(self):
        for seat in ("unknown", "occupied"):
            st = fx([dead_seat(seat=seat)])
            self.assertEqual(BN.node_liveness(st, "cso")[0], BN.LIVENESS_AWAKE,
                             "seat=%r(비 empty)에서 래치 무효화 — 판정불가/점유에 침습 판정" % seat)
        # 필드 부재(구 데몬 무신호)도 절대 트리거 금지
        row = dead_seat()
        del row["seat"]
        self.assertEqual(BN.node_liveness(fx([row]), "cso")[0], BN.LIVENESS_AWAKE,
                         "seat 필드 부재(구 데몬)에서 래치 무효화 — 무신호를 empty 로 융합")

    # ── CASE-B: ⓑ 거짓(agent_alive 가 명시적 False 아님) → 래치 유지 ──
    def test_case_b_agent_alive_null_windows_master_holds(self):
        """★이 티켓의 핵심 단언 — `agent_alive: null`(Windows master 재현)에서 래치 **유지**.

        daemon 의 agent_alive 는 3상이다: meta 부재 → JSON null(handlers.rs — meta.as_ref().map(..)).
        claim-role 관측 등록은 #[cfg(unix)] 라 Windows 에서 "너는 마스터다"로 만들어진 master
        좌석은 null 이 영구다. 여기서 래치가 무효화되면: 매 check 결손 오판 → node-recover 가
        살아있는 claude 입력창에 기동 커맨드 주입 → 실패 시 reclaim kill → 치명 앵커 ④."""
        st = fx([dead_seat(agent_alive=None)])           # JSON null == python None
        grade, why = BN.node_liveness(st, "cso")
        self.assertEqual(grade, BN.LIVENESS_AWAKE,
                         "agent_alive=null 에서 래치 무효화 — Windows master 매 check 결손 오판"
                         "(치명 앵커 ④ 경로): %s" % why)
        self.assertTrue(BN.awake_ready(st, "cso")[0],
                        "agent_alive=null 에서 PRE-CHECK 각성 상실(Windows master 재주입 유도)")
        # 키 자체 부재·True 도 동일하게 유지(명시적 False 만 트리거)
        row = dead_seat()
        del row["agent_alive"]
        self.assertEqual(BN.node_liveness(fx([row]), "cso")[0], BN.LIVENESS_AWAKE,
                         "agent_alive 키 부재에서 래치 무효화(낙관적 falsy 구현 회귀)")
        self.assertEqual(BN.node_liveness(fx([dead_seat(agent_alive=True)]), "cso")[0],
                         BN.LIVENESS_AWAKE, "agent_alive=True(생존)에서 래치 무효화")

    # ── CASE-C: ⓒ 거짓(좌석 나이 ≤ 예산 / 나이 미상) → 래치 유지 ──
    def test_case_c_young_or_unknown_age_holds(self):
        st = fx([dead_seat(created_at=time.time() - 1.0)])
        self.assertEqual(BN.node_liveness(st, "cso")[0], BN.LIVENESS_AWAKE,
                         "기동 중 좌석(나이<예산)에서 래치 무효화 — 동시 boot/restore 레이스")
        row = dead_seat()
        del row["created_at"]
        self.assertEqual(BN.node_liveness(fx([row]), "cso")[0], BN.LIVENESS_AWAKE,
                         "created_at 미상(나이 측정 불가)에서 래치 무효화(보류 우선 위반)")
        self.assertEqual(BN.node_liveness(fx([dead_seat(created_at=0)]), "cso")[0],
                         BN.LIVENESS_AWAKE, "created_at=0(미상 규약)에서 래치 무효화")

    # ── 경계·순수 판정 세부(latch_death_confirmed 직접 — now 주입으로 벽시계 배제) ──
    def test_boundary_age_equals_floor_is_strictly_over(self):
        """경계는 엄격 초과(age > floor) — Rust `!(age > floor)` 거부와 자구 동등.
        (Rust 검체엔 이 케이스가 없다: 그쪽은 벽시계를 내부에서 읽어 정확 경계가 flaky —
        경계 규약은 now 주입이 가능한 python 이 잰다.)"""
        floor = BN._readiness_budget_s()
        row = dead_seat(created_at=1000.0)
        dead, why = BN.latch_death_confirmed(row, now=1000.0 + floor)      # age == floor
        self.assertFalse(dead, "age==floor 가 확정으로 접힘(엄격 초과 위반): %s" % why)
        dead2, _ = BN.latch_death_confirmed(row, now=1000.0 + floor + 1)   # age > floor
        self.assertTrue(dead2, "age>floor 인데 미확정(회수 불능)")

    def test_empty_row_never_confirms(self):
        self.assertFalse(BN.latch_death_confirmed({})[0], "무신호 행에서 죽음 확정(전면 보류 위반)")

    # ── 무회귀 핀: 기존 래치·legacy 계약 불변(H-PRED-3 와 같은 픽스처) ──
    def test_latch_only_and_legacy_contracts_unchanged(self):
        latched = fx([{"role": "cso", "exited": False, "awakened_at": 1700000000.0}])
        self.assertEqual(BN.node_liveness(latched, "cso")[0], BN.LIVENESS_AWAKE,
                         "래치 단독 좌석이 각성확정 상실(legacy 계약 회귀)")
        self.assertTrue(BN.awake_ready(latched, "cso")[0], "래치 단독 PRE-CHECK 회귀")
        # 래치 없는 죽은 좌석의 기존 결론(absent)은 부정 로직과 무관하게 불변
        no_latch = dead_seat()
        del no_latch["awakened_at"]
        self.assertEqual(BN.node_liveness(fx([no_latch]), "cso")[0], BN.LIVENESS_ABSENT,
                         "래치 없는 빈 좌석의 기존 absent 결론 변형")
        # 파괴 경로 Unknown=무조건 hold(이원 규칙)도 불변
        unk = fx([{"role": "cso", "exited": False, "agent_alive": False,
                   "status": None, "seat": "unknown"}])
        self.assertEqual(BN._reclaim_verdict(unk, "cso", 100, 100), "hold-alive",
                         "Unknown 좌석에 kill 허용(파괴 경로 fail-closed 붕괴)")


class BootDeficitUnknown(unittest.TestCase):
    """W-B1 ③ — 부트 경로 unknown=결손(시한부 해소 후 잔존만) ∧ ⑤check satisfied 불변."""

    @classmethod
    def setUpClass(cls):
        # 로스터 적응형(H-PRED-1 패턴): 이 기계의 감지 결과(네이티브/대체)에 무관하게 성립.
        cls.required = ["cso", "worker"] + [e["role"] for e in O.reviewer_roster()]

    def _healthy(self):
        return fx([{"role": r, "exited": False, "awakened_at": 1.0} for r in self.required])

    def _one_unknown(self):
        rows = [{"role": self.required[0], "exited": False, "seat": "unknown"}]
        rows += [{"role": r, "exited": False, "awakened_at": 1.0} for r in self.required[1:]]
        return fx(rows)

    def test_check_satisfied_invariant_on_unknown(self):
        """감사 확정 계약: unknown 은 ⑤check 에서 **충족측**(콜드스타트 exit 6 라이브락 금지)."""
        v, _ = O.check_verdicts(self._one_unknown())
        self.assertEqual(v[self.required[0]]["grade"], "unknown", "픽스처가 unknown 등급이 아님")
        self.assertTrue(v[self.required[0]]["satisfied"],
                        "⑤check 가 unknown 을 미충족으로 뒤집음 — 콜드스타트 exit 6 라이브락 재발")

    def test_deficit_counts_residual_unknown(self):
        st = self._one_unknown()
        has, why = O._shared_verdict_deficit(st, requery=lambda: st, tick_s=0)
        self.assertIs(has, True, "잔존 unknown 이 결손으로 계상되지 않음(BOOT_SKIP 잔존): %s" % why)
        self.assertIn("unknown", why, "결손 사유에 unknown 근거가 없음(진단 불가): %s" % why)

    def test_deficit_clears_when_unknown_resolves_alive(self):
        has, why = O._shared_verdict_deficit(self._one_unknown(),
                                             requery=self._healthy, tick_s=0)
        self.assertIs(has, False,
                      "재조회에서 생존 확인된 unknown 을 결손로 계상(불필요 boot·churn): %s" % why)

    def test_deficit_parity_with_plain_verdicts(self):
        """unknown 이 없을 때는 종전 결손 산출과 같은 결론(missing→True / 전원 충족→False)."""
        self.assertIs(O._shared_verdict_deficit(self._healthy(),
                                                requery=self._healthy, tick_s=0)[0], False)
        missing = fx([{"role": r, "exited": False, "awakened_at": 1.0}
                      for r in self.required[1:]])
        self.assertIs(O._shared_verdict_deficit(missing, requery=lambda: missing,
                                                tick_s=0)[0], True,
                      "의무 좌석 부재가 결손으로 계상되지 않음")

    def test_dead_latched_seat_now_counts_as_deficit(self):
        """blocker 원 사슬의 종단 검증: 죽음 3중 확정 + 래치 좌석 → 결손>0 → `cys boot` 호출 유도."""
        rows = [dead_seat(role=self.required[0])]
        rows += [{"role": r, "exited": False, "awakened_at": 1.0} for r in self.required[1:]]
        st = fx(rows)
        has, why = O._shared_verdict_deficit(st, requery=lambda: st, tick_s=0)
        self.assertIs(has, True, "죽음 확정 래치 좌석이 결손 0(BOOT_SKIP 거짓 성공 잔존): %s" % why)


class StatusTimeoutLeafWiring(unittest.TestCase):
    """W-B1 ④ — cys_status 의 timeout 이 leaf `CYS_STATUS_TIMEOUT_S` 를 실제로 소비하는가."""

    def test_behavioral_timeout_consumes_leaf(self):
        os.environ.pop("CYS_BUDGET_CYS_STATUS_TIMEOUT_S", None)   # 하네스 override 배제
        captured = {}
        orig_run = BN.run

        def fake_run(args, timeout=15):
            captured["args"], captured["timeout"] = list(args), timeout
            return 1, "", ""                       # rc!=0 → None 반환 경로·부작용 0

        BN.run = fake_run
        try:
            self.assertIsNone(BN.cys_status())
        finally:
            BN.run = orig_run
        self.assertEqual(captured["args"][:3], ["cys", "status", "--json"])
        self.assertEqual(captured["timeout"], BN.budget("CYS_STATUS_TIMEOUT_S", 12),
                         "cys_status timeout 이 budget leaf 를 소비하지 않음")
        try:
            import javis_budget as _b
            self.assertEqual(captured["timeout"], _b.leaf("CYS_STATUS_TIMEOUT_S"),
                             "boot_node 사본과 예산 SOT 의 값 불일치(드리프트 재발)")
        except ImportError:
            pass                                   # 배포 팩 결손 — budget() 폴백 12 는 위에서 검증됨

    def test_source_pin_no_hardcoded_status_timeout(self):
        with open(os.path.join(BIN_DIR, "javis_boot_node.py"), encoding="utf-8") as f:
            src = f.read()
        self.assertIn('timeout=budget("CYS_STATUS_TIMEOUT_S", 12)', src,
                      "cys_status 배선 핀 부재(하드코딩 복귀?)")
        self.assertNotIn('"--json"], timeout=12)', src,
                         "status --json 의 timeout=12 하드코딩 잔존(W-A4b 사본 드리프트 부활)")

    def test_orchestra_docstring_updated(self):
        doc = O._cys_status_timeout_s.__doc__ or ""
        self.assertNotIn("여전히 `timeout=12`", doc,
                         "orchestra 문서가 여전히 '미배선'을 주장(거짓 주석 = 이 저장소의 결함)")
        self.assertIn("배선 완료", doc, "배선 완료 사실이 문서에 갱신되지 않음")


class GatePendingFourthGrade(unittest.TestCase):
    """★(U-10) 좌석 **제4 등급** `gate_pending` — python 행동 배터리(4상 표).

    무엇을 봉인하는가:
      ⓐ 살아 있는 관문 좌석이 `alive_presumed` 로 접혀 '이미 가동 중'이 되지 않는다.
      ⓑ `null`·키 부재·비 dict 는 **무신호**로 접혀 종전 판정 그대로다(구 데몬 혼재 안전).
      ⓒ ★★충족이 아니라고 해서 **죽은 것이 아니다** — 파괴 게이트(`node_alive` → reclaim
        kill)는 이 등급을 반드시 **생존측**으로 읽는다. 이 축이 뒤집히면 첫기동 관문에 갇힌
        신규 프로필의 4종 노드가 전부 kill 대상이 되어 **전 pane 사망(글자 0)** 이다.
      ⓓ 래치 단방향 계약은 **무접촉**이다(금지 방향 ⑦).
      ⓔ 롤백 킬스위치(`CYS_GATE_PENDING=0`) 1지점으로 축 전체가 종전 판정으로 복귀한다.
    """
    maxDiff = None

    @staticmethod
    def gated(**over):
        row = {"role": "cso", "exited": False, "agent_alive": True, "seat": "occupied",
               "gate_pending": {"gate": "disclaimer", "since": 1.0}}
        row.update(over)
        return fx([row])

    def test_case_gate_all_gated_seat_is_its_own_grade(self):
        grade, why = BN.node_liveness(self.gated(), "cso")
        self.assertEqual(grade, BN.LIVENESS_GATED,
                         "관문 보류 좌석이 제4 등급을 받지 못함(허위 already_alive 경로): %s" % why)

    def test_case_gate_a_null_or_missing_is_no_signal(self):
        # 구 데몬(키 부재) + 신 팩 혼재: 종전 등급 그대로여야 한다(부재 ≠ 부정).
        self.assertEqual(BN.node_liveness(self.gated(gate_pending=None), "cso")[0],
                         BN.LIVENESS_PRESUMED, "null 이 종전 등급을 바꿨다(항 생략 규약 위반)")
        row = {"role": "cso", "exited": False, "agent_alive": True, "seat": "occupied"}
        self.assertEqual(BN.node_liveness(fx([row]), "cso")[0], BN.LIVENESS_PRESUMED,
                         "키 부재(구 데몬)에서 종전 등급이 변형됐다(혼재 안전 붕괴)")

    def test_case_gate_b_non_dict_folds_to_prior_grade(self):
        # 스큐·손상 값을 'gated' 로 접으면 판정불가가 미충족을 만들어 부트 재시도 라이브락(A1)이 된다.
        for bad in (True, "gated", 1, [], 0.5):
            self.assertEqual(BN.node_liveness(self.gated(gate_pending=bad), "cso")[0],
                             BN.LIVENESS_PRESUMED,
                             "손상 gate_pending(%r)이 등급을 움직였다(fail-open 방향 위반)" % (bad,))

    def test_case_gate_c_destruction_gate_reads_it_as_alive(self):
        # ★치명 앵커 ④ — 이 검체가 적색이면 절대 출하하지 않는다.
        st = self.gated()
        self.assertTrue(BN.node_alive(st, "cso"),
                        "관문 보류 좌석을 죽음으로 판정(오살 — 전 pane 사망 경로 신설)")
        self.assertEqual(BN._reclaim_verdict(st, "cso", 100, 100), "hold-alive",
                         "관문 보류 좌석에 kill 허용(파괴 경로 보류 우선 위반)")
        # 래치 부정 3중 AND(파괴 경로)는 이 등급을 보지 않는다 — 무접촉 계약.
        dead, _ = BN.latch_death_confirmed(self.gated()["surfaces"][0])
        self.assertFalse(dead, "살아 있는 관문 좌석이 죽음 3중 확정으로 읽힘")

    def test_case_gate_d_latch_contract_untouched(self):
        self.assertEqual(BN.node_liveness(self.gated(awakened_at=1_700_000_000.0), "cso")[0],
                         BN.LIVENESS_AWAKE,
                         "래치 단방향 계약이 관문 신호로 뒤집혔다(금지 방향 ⑦ 위반)")

    def test_case_gate_e_kill_switch_reverts_axis(self):
        prev = os.environ.get(BN.GATE_PENDING_ENV)
        try:
            os.environ[BN.GATE_PENDING_ENV] = "0"
            self.assertFalse(BN.gate_pending_axis_enabled(), "'0' 이 축을 끄지 못한다")
            self.assertEqual(BN.node_liveness(self.gated(), "cso")[0], BN.LIVENESS_PRESUMED,
                             "킬스위치 off 인데 제4 등급이 살아 있다(롤백 1지점 계약 붕괴)")
            for loose in ("", "false", "off", "1"):
                os.environ[BN.GATE_PENDING_ENV] = loose
                self.assertTrue(BN.gate_pending_axis_enabled(),
                                "느슨한 값 %r 이 축을 껐다(오타로 안전장치 소실)" % loose)
        finally:
            if prev is None:
                os.environ.pop(BN.GATE_PENDING_ENV, None)
            else:
                os.environ[BN.GATE_PENDING_ENV] = prev


@unittest.skipUnless(os.path.isfile(CYS_RS),
                     "레포 소스 부재(배포 팩 단독) — Rust 파리티 핀은 체크아웃에서만")
class GatePendingRustParityPins(unittest.TestCase):
    """★(U-10) 제4 등급의 python ↔ Rust **짝 소실 검출**.

    행동 실행은 각 언어의 배터리가 한다(python=위 클래스 · Rust=cys.rs
    `mod seat_latch_negation_tests` 의 `gate_case_*` 4종, `cargo test --bin cys gate_case`).
    여기 핀은 한쪽만 고치고 다른 쪽을 잊는 사본 드리프트를 기계로 잡는다."""

    @classmethod
    def setUpClass(cls):
        with open(CYS_RS, encoding="utf-8") as f:
            cls.src = f.read()
        i = cls.src.find("fn seat_liveness(")
        cls.liveness_body = cls.src[i:cls.src.find("\n}\n", i)] if i >= 0 else ""

    def test_rust_has_the_fourth_grade(self):
        self.assertIn("GatePending", self.src, "Rust 측 제4 등급 변형 부재(파리티 붕괴)")
        self.assertIn("gate_pending_from_wire", self.liveness_body,
                      "Rust seat_liveness 가 관문 축을 읽지 않는다(python 만 제4 등급)")

    def test_rust_gate_branch_precedes_agent_alive(self):
        """★순서가 계약이다 — 관문 분기가 `agent_alive` 분기보다 **앞**이어야 한다.
        뒤에 있으면 관문 좌석도 프로세스는 살아 있으므로 그 분기가 영원히 도달 불가(죽은 코드)
        이고 보류 좌석이 다시 AlivePresumed → already_alive 로 접힌다. python 도 같은 순서다."""
        gi = self.liveness_body.find("gate_pending_from_wire")
        ai = self.liveness_body.find('s["agent_alive"].as_bool()')
        self.assertTrue(gi >= 0 and ai >= 0, "Rust 순서 판정 재료를 못 찾았다")
        self.assertLess(gi, ai, "Rust 관문 분기가 agent_alive 분기보다 뒤다(제4 등급 도달 불가)")
        # python 미러도 같은 순서인가 — 소스 오프셋으로 잰다(양쪽 같은 규약).
        bn_src = open(os.path.join(BIN_DIR, "javis_boot_node.py"), encoding="utf-8").read()
        li = bn_src.find("def node_liveness(")
        body = bn_src[li:bn_src.find("\ndef ", li + 10)]
        self.assertLess(body.find("gate_pending_info(s)"), body.find('s.get("agent_alive")'),
                        "python 관문 분기가 agent_alive 분기보다 뒤다(제4 등급 도달 불가)")

    def test_rust_battery_exists_with_same_case_table(self):
        for fn in ("gate_case_all_gated_seat_is_not_already_alive",
                   "gate_case_a_null_or_missing_is_no_signal_not_negation",
                   "gate_case_b_non_object_folds_to_prior_grade",
                   "gate_case_c_destruction_path_is_frozen"):
            self.assertIn(fn, self.src, "Rust 배터리에 4상 표 케이스 %s 부재(짝 소실)" % fn)

    def test_wire_key_and_env_name_parity(self):
        """키·env 이름이 언어 간에 갈리면 축이 조용히 사라진다(한쪽은 쓰고 한쪽은 못 읽는다)."""
        self.assertIn('pub const GATE_PENDING_KEY: &str = "gate_pending";',
                      open(os.path.join(REPO_DIR, "src", "lib.rs"), encoding="utf-8").read(),
                      "Rust wire 키 상수가 python 미러(GATE_PENDING_KEY)와 다르다")
        self.assertEqual(BN.GATE_PENDING_KEY, "gate_pending")
        self.assertEqual(BN.GATE_PENDING_ENV, "CYS_GATE_PENDING")
        self.assertIn('pub const ENV_GATE_PENDING: &str = "CYS_GATE_PENDING";',
                      open(os.path.join(REPO_DIR, "src", "lib.rs"), encoding="utf-8").read(),
                      "Rust 킬스위치 env 이름이 python 미러와 다르다(한쪽만 롤백되는 사고)")


@unittest.skipUnless(os.path.isfile(CYS_RS),
                     "레포 소스 부재(배포 팩 단독) — Rust 파리티 핀은 체크아웃에서만")
class RustParityPins(unittest.TestCase):
    """python↔Rust 판정 대조 — 배선 핀 + 배터리 존재 핀 + ⓒ임계 수치 파리티.

    행동 실행은 각 언어의 배터리가 한다(python=이 파일 위 클래스 · Rust=cys.rs
    `mod seat_latch_negation_tests`, `cargo test --bin cys seat_latch_negation`). 여기의 핀은
    **짝 소실**(한쪽 계약만 고치고 다른쪽을 잊는 사본 드리프트)을 기계 검출한다."""

    @classmethod
    def setUpClass(cls):
        with open(CYS_RS, encoding="utf-8") as f:
            cls.src = f.read()
        i = cls.src.find("fn seat_liveness(")
        cls.liveness_body = cls.src[i:cls.src.find("\n}\n", i)] if i >= 0 else ""
        j = cls.src.find("fn seat_death_confirmed(")
        cls.death_body = cls.src[j:cls.src.find("\n}\n", j)] if j >= 0 else ""

    def test_rust_latch_branch_consumes_death_gate(self):
        self.assertIn("seat_death_confirmed(s).is_ok()", self.liveness_body,
                      "Rust seat_liveness 래치 분기가 seat_death_confirmed 를 소비하지 않음"
                      "(python 만 부정 — 한쪽은 살았다, 한쪽은 죽었다)")
        self.assertIn("래치 부정", self.liveness_body, "Rust 부정 사유 라벨 부재")

    def test_rust_triple_and_contract_pins(self):
        """python `latch_death_confirmed` 가 미러하는 3중 AND 원계약이 Rust 에 그대로 있는가 —
        여기 핀이 깨지면 Rust 계약이 움직인 것이고, python 미러를 **같은 커밋에서** 함께
        움직여야 한다(H-SAFE-1 과 겹치는 핀은 의도적 이중 봉인)."""
        for pin, label in (('Some("empty") => {}', "ⓐ 명시적 empty 만"),
                           ('s["agent_alive"].as_bool() != Some(false)', "ⓑ 명시적 false 만"),
                           ("created <= 0.0", "ⓒ 나이 미상 보류"),
                           ("budget_readiness_max(0, false)", "ⓒ readiness 예산")):
            self.assertIn(pin, self.death_body, "Rust 3중 AND 계약 이동(%s): %r 부재" % (label, pin))

    def test_rust_battery_exists_with_same_case_table(self):
        self.assertIn("mod seat_latch_negation_tests", self.src,
                      "Rust 측 래치 배터리 부재(파리티 검체 짝 소실)")
        for fn in ("case_all_true_latch_negated_to_absent",
                   "case_a_seat_not_empty_holds_latch",
                   "case_b_agent_alive_not_explicit_false_holds_latch",
                   "case_c_young_or_unknown_age_holds_latch"):
            self.assertIn(fn, self.src, "Rust 배터리에 4상 표 케이스 %s 부재" % fn)

    def test_readiness_threshold_parity(self):
        """ⓒ항 임계의 언어 간 수치 파리티 — Rust 상수 산식(max(0,FLOOR)×MULT)과 python leaf
        파생(launch_readiness_max_s)이 같은 수를 내는가. 갈리면 같은 좌석 나이를 한쪽은
        '기동 중', 한쪽은 '죽음 확정'으로 읽는다."""
        floor = int(re.search(r"BUDGET_READINESS_FLOOR_SECS:\s*u64\s*=\s*(\d+)", self.src).group(1))
        mult = int(re.search(r"BUDGET_READINESS_MULT:\s*u64\s*=\s*(\d+)", self.src).group(1))
        os.environ.pop("CYS_BUDGET_LAUNCH_READINESS_FLOOR_S", None)
        os.environ.pop("CYS_BUDGET_LAUNCH_READINESS_MULT", None)
        rust_default = max(0, floor) * mult
        self.assertEqual(float(rust_default), float(BN._readiness_budget_s()),
                         "readiness 임계가 언어 간 상이(래치 부정 ⓒ항 파리티 붕괴)")


if __name__ == "__main__":
    unittest.main(verbosity=2)
