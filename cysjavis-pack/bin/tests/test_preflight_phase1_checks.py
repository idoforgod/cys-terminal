#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_phase1_checks.py — Phase 1 신규 부트 진단 C72~C75 회귀 하네스 (standalone).

★왜 이 파일이 존재하는가(E2 표본 감사 T4 `blocking` · R-31 · E3-3 이행):
  Wave B2 는 공용 부트 진단 `javis_preflight.py` 에 +329줄(C72~C75)을 더했는데 그 도구에는
  `--self-test` **진입점 자체가 없어** 자동 회귀 하네스가 0건이었다. 같은 Phase 의 다른 신규
  도구 7종은 전부 self-test 를 갖는다 — 일관성 붕괴이자, 더 나쁘게는 **OT-2 카나리아의 중단
  기준이 이 진단에 의존**하는데 그 진단 자신이 검증되지 않는 상태였다(C72 FAIL 전환 = 중단).
  E2 는 C72 표적 케이스만 격리 스크래치에 만들고 끝냈다(휘발) — 여기서 C73·C74·C75 까지
  덮고 **영속화**한다.

★계측 타당성(이 하네스가 스스로에게 거는 게이트 — 2026-07-23 부트 사고 교훈):
  "PASS 가 나온다"는 하네스는 아무것도 증명하지 않는다. 모든 검사에 대해 **결함을 주입해
  판정이 실제로 뒤집히는지**(PASS→FAIL/WARN) 함께 잰다. 뒤집히지 않으면 그 검사는 감시
  능력이 없는 것이고, 그 사실이 여기서 빨갛게 드러나야 한다.

라이브 무접촉: 픽스처 팩·픽스처 HOME·픽스처 JAVIS_ROOT 전부 임시 디렉터리이고, 쓰기는
그 안에서만 일어난다. `--fix` 는 어떤 경로에서도 켜지 않는다(Preflight(fix=False)).

    JAVIS_ROOT=<scratch> CYS_PROBE_RUNS=<scratch>/probe_runs.jsonl \
    HUD_STATE_DIR=<scratch>/hud CYS_NO_AUTOSTART=1 \
    python3 cysjavis-pack/bin/tests/test_preflight_phase1_checks.py
"""
import json
import os
import shutil
import sys
import tempfile
import unittest


def _hard_assert_isolation():
    missing = [k for k in ("JAVIS_ROOT", "CYS_PROBE_RUNS") if not os.environ.get(k)]
    if missing:
        sys.stderr.write(
            "REFUSE(hard assert): 격리 env 미설정 %s — cwd 폴백의 라이브 오염 차단(G1). "
            "JAVIS_ROOT=<scratch> CYS_PROBE_RUNS=<scratch>/probe_runs.jsonl 로 재실행하라.\n"
            % missing)
        sys.exit(2)


_hard_assert_isolation()

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
PACK_SRC = os.path.dirname(BIN)
sys.path.insert(0, BIN)

import javis_preflight as pf  # noqa: E402  — 형제 모듈 직접 구동(서브프로세스 아님)

PASS, WARN, FAIL, SKIP = pf.PASS, pf.WARN, pf.FAIL, pf.SKIP

def rd(p, mode="r"):
    with open(p, mode, **({} if "b" in mode else {"encoding": "utf-8"})) as f:
        return f.read()


def wr(p, data, mode="w"):
    with open(p, mode, encoding="utf-8") as f:
        f.write(data)


# 픽스처 팩이 실물로 갖춰야 하는 파일(C72~C74 가 경로로 읽는 것 전부).
PACK_FILES = ("bin/javis_task.py", "bin/javis_completion_guard.py",
              "bin/javis_brief_lint.py", "hooks/brief-lint-warn.sh",
              "hooks/completion-guard.sh", "schemas/verify_spec_schema.json")


def mkpack(dst):
    """실 팩에서 필요한 파일만 **복사**한 픽스처 팩(원본 무접촉 · 결함 주입 가능)."""
    for rel in PACK_FILES:
        src = os.path.join(PACK_SRC, rel)
        out = os.path.join(dst, rel)
        os.makedirs(os.path.dirname(out), exist_ok=True)
        if os.path.isfile(src):
            shutil.copy2(src, out)
    os.makedirs(os.path.join(dst, "state"), exist_ok=True)
    return dst


class Base(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.mkdtemp(prefix="pf-phase1-")
        self.addCleanup(shutil.rmtree, self.td, True)
        self.pack = mkpack(os.path.join(self.td, "pack"))
        self.root = os.path.join(self.td, "root")
        self.tasks = os.path.join(self.root, "_round", "tasks")
        os.makedirs(self.tasks, exist_ok=True)
        self.home = os.path.join(self.td, "home")
        os.makedirs(self.home, exist_ok=True)
        self._env0 = dict(os.environ)
        os.environ["CYS_PACK_DIR"] = self.pack
        os.environ["JAVIS_ROOT"] = self.root
        self.addCleanup(self._restore)

    def _restore(self):
        os.environ.clear()
        os.environ.update(self._env0)

    def check(self, method, cid_prefix):
        """검사 1개를 돌리고 (status, detail) 반환 — 결과 1건만 나오는지도 함께 확인."""
        p = pf.Preflight(fix=False, skips=[])
        getattr(p, method)()
        rows = [r for r in p.results if r["id"].startswith(cid_prefix)]
        self.assertEqual(len(rows), 1, "결과 행 %d개(1 기대): %r" % (len(rows), p.results))
        return rows[0]["status"], rows[0]["detail"]

    def mktask(self, tid, status="in_progress", spec=None, **extra):
        rec = {"id": tid, "title": tid, "status": status}
        if spec is not None:
            rec["verify_spec"] = spec
        rec.update(extra)
        with open(os.path.join(self.tasks, "%s.json" % tid), "w", encoding="utf-8") as f:
            json.dump(rec, f, ensure_ascii=False)

    def mkprofile(self, name, body='{\n  "hooks": {}\n}\n'):
        d = os.path.join(self.home, name)
        os.makedirs(d, exist_ok=True)
        p = os.path.join(d, "settings.json")
        with open(p, "w", encoding="utf-8") as f:
            f.write(body)
        return p


# ══ C72 — 게이트 3점 세트 세대 정합(E2 스크래치 케이스의 영속화) ═══════════════════
class TestC72(Base):
    def test_pass_and_generation_tear(self):
        st, detail = self.check("c72_phase1_gate_set", "C72")
        self.assertIn(st, (PASS, WARN), detail)   # 평시(태스크 0) = PASS/WARN
        # 결함 주입: guard 의 예약 접미 튜플만 구세대로 변조 → 두 구성요소 세대 찢김
        gp = os.path.join(self.pack, "bin", "javis_completion_guard.py")
        txt = rd(gp)
        self.assertIn("RESERVED_ID_SUFFIXES", txt, "핀 대상 심볼 부재 — 계측기 무효")
        import re
        txt2 = re.sub(r"RESERVED_ID_SUFFIXES\s*=\s*\([^)]*\)",
                      'RESERVED_ID_SUFFIXES = (".guard.json",)', txt, count=1)
        self.assertNotEqual(txt, txt2, "변조 실패 — 계측기 무효")
        wr(gp, txt2)
        st2, detail2 = self.check("c72_phase1_gate_set", "C72")
        self.assertEqual(st2, FAIL, "세대 찢김이 FAIL 로 뒤집히지 않음: %s" % detail2)
        self.assertIn("RESERVED_ID_SUFFIXES", detail2)

    def test_silence_detector_surfaced(self):
        """E2-1(c) 침묵 탐지 상태 파일이 부트 WARN 으로 올라오는가(런타임 경보의 짝)."""
        with open(os.path.join(self.tasks, ".guard-silence.7.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"streak": 6, "threshold": 5, "last_reason": "claim 부재",
                       "last_ts": "2026-07-31T00:00:00", "alerted_at": "x"}, f)
        st, detail = self.check("c72_phase1_gate_set", "C72")
        self.assertEqual(st, WARN, detail)
        self.assertIn("guard 침묵 지속", detail)
        # 임계 미달은 무발화(오탐 억제)
        with open(os.path.join(self.tasks, ".guard-silence.7.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"streak": 2, "threshold": 5, "last_reason": "claim 부재"}, f)
        st2, detail2 = self.check("c72_phase1_gate_set", "C72")
        self.assertNotIn("guard 침묵 지속", detail2)
        self.assertIn("verifier tax", detail2)     # E2-2: 활성/비활성을 말하게 만든다


# ══ C73 — guard 훅 무결성(D4: 블록 압박 하 훅 자가제거) ═══════════════════════════
class TestC73(Base):
    def expected(self, entries):
        p = os.path.join(self.pack, "state", "guard-hook-expected.json")
        with open(p, "w", encoding="utf-8") as f:
            json.dump({"schema_version": 1, "profiles": entries}, f, ensure_ascii=False)
        return p

    def test_absent_marker_is_skip(self):
        """등록 전(OT-2 이전) 상태 = SKIP. 이 경로가 FAIL 이면 배포 즉시 부트가 빨개진다."""
        st, detail = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st, SKIP, detail)
        self.assertIn("등록 전", detail)

    def test_registered_pass_then_self_removal_fail(self):
        sp = self.mkprofile(".claude-worker",
                            '{"hooks":{"Stop":[{"hooks":[{"type":"command",'
                            '"command":"sh /p/hooks/completion-guard.sh","timeout":60}]}]}}')
        import hashlib
        sha = hashlib.sha256(rd(sp, "rb")).hexdigest()
        self.expected([{"settings": sp, "sha256": sha,
                        "must_contain": ["sh /p/hooks/completion-guard.sh"]}])
        st, detail = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st, PASS, detail)
        # 결함 ①: 훅 자가제거(D4 실측 시나리오) → FAIL
        wr(sp, '{"hooks":{"Stop":[]}}')
        st2, detail2 = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st2, FAIL, "등록 소실이 FAIL 로 뒤집히지 않음: %s" % detail2)
        self.assertIn("훅 자가제거 의심", detail2)
        # 결함 ②: settings 파일 자체 소실 → FAIL(다른 사유 문면)
        os.remove(sp)
        st3, detail3 = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st3, FAIL, detail3)
        self.assertIn("등록 소실 의심", detail3)

    def test_sha_drift_is_warn_not_fail(self):
        """G7g 판정 서열: must_contain 통과 시 sha 불일치는 WARN 강등(오경보 양산 차단)."""
        sp = self.mkprofile(".claude-worker",
                            '{"hooks":{"Stop":[{"hooks":[{"type":"command",'
                            '"command":"sh /p/hooks/completion-guard.sh"}]}]}}')
        self.expected([{"settings": sp, "sha256": "0" * 64,
                        "must_contain": ["sh /p/hooks/completion-guard.sh"]}])
        st, detail = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st, WARN, detail)
        self.assertIn("체크섬 불일치", detail)

    def test_corrupt_marker_is_warn(self):
        """마커 손상 = 감시 불능인데 부트는 비차단(WARN) — 이 강등이 유지되는지 핀."""
        wr(os.path.join(self.pack, "state", "guard-hook-expected.json"), "{ not json")
        st, detail = self.check("c73_guard_hook_integrity", "C73")
        self.assertEqual(st, WARN, detail)
        self.assertIn("감시 불능", detail)


# ══ C74 — brief 게이트 2경로 배선 ════════════════════════════════════════════════
class TestC74(Base):
    def setUp(self):
        super().setUp()
        # 실 팩이 임시 디렉터리 아래라 `_path_under_tempdir` 가 home-glob 을 차단한다
        # (임시 pack 누수 방지 가드 · 의도된 동작). 대상표 경로를 실제로 재려면 그 가드만
        # 이 테스트에서 무력화한다 — 가드 자체는 test_temp_pack_isolation 이 따로 핀한다.
        self._orig_tmp = pf._path_under_tempdir
        os.environ["HOME"] = self.home
        os.environ.pop("CLAUDE_CONFIG_DIR", None)
        os.environ.pop("CYS_BRIEF_WARN_PROFILES", None)
        self.addCleanup(setattr, pf, "_path_under_tempdir", self._orig_tmp)

    def unblock(self):
        pf._path_under_tempdir = lambda p: False

    def test_temp_pack_isolation_guard_holds(self):
        """가드 살아 있음 확인 — 임시 팩 컨텍스트에서 home-glob 대상은 0이어야 한다."""
        self.mkprofile(".claude")
        self.assertEqual(pf.discover_claude_settings(), [],
                         "임시 팩인데 home-glob 이 열렸다(실 settings 오염 경로)")

    def test_missing_engine_is_fail(self):
        self.unblock()
        os.remove(os.path.join(self.pack, "bin", "javis_brief_lint.py"))
        st, detail = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st, FAIL, detail)
        self.assertIn("javis_brief_lint.py 부재", detail)
        # 훅 실체 소실도 FAIL
        os.remove(os.path.join(self.pack, "hooks", "brief-lint-warn.sh"))
        st2, detail2 = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st2, FAIL, detail2)
        self.assertIn("인세션 경고 경로 실체 소실", detail2)

    def test_hard_path_code_regression_is_fail(self):
        """checkout --brief hard 경로 코드가 사라지면(T1 회귀) FAIL — grep 핀의 검출력."""
        self.unblock()
        tp = os.path.join(self.pack, "bin", "javis_task.py")
        txt = rd(tp)
        self.assertIn("javis_brief_lint", txt, "핀 대상 문자열 부재 — 계측기 무효")
        wr(tp, txt.replace("javis_brief_lint", "XXX"))
        st, detail = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st, FAIL, detail)
        self.assertIn("hard 경로 코드 부재", detail)

    def test_table_driven_targets(self):
        """대상표(env) 기준 미등록 = WARN, 전량 등록 = PASS. ★R-04(b) 회귀 핀:
        표에 `.claude-3` 가 빠지면 실제 master 세션에 경고가 도달하지 않는데도 PASS 가
        난다 — 그래서 표는 손타이핑이 아니라 hook-targets.json 에서 파생해야 한다."""
        self.unblock()
        reg = ('{"hooks":{"PostToolUse":[{"matcher":"Task|Agent","hooks":'
               '[{"type":"command","command":"sh /p/hooks/brief-lint-warn.sh"}]}]}}')
        self.mkprofile(".claude", reg)
        self.mkprofile(".claude-3")                      # 미등록
        os.environ["CYS_BRIEF_WARN_PROFILES"] = ".claude,.claude-3"
        st, detail = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st, WARN, detail)
        self.assertIn(".claude-3", detail)
        # 전량 등록 → PASS
        self.mkprofile(".claude-3", reg)
        st2, detail2 = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st2, PASS, detail2)
        # ★표에서 .claude-3 를 빼면(종전 3줄 예시 상태) 미등록인데도 PASS — 침묵 경로
        self.mkprofile(".claude-3")                      # 다시 미등록으로
        os.environ["CYS_BRIEF_WARN_PROFILES"] = ".claude"
        st3, detail3 = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st3, PASS,
                         "표 누락 시 PASS 라는 **현행 한계**가 바뀌었다 — 대상표 파생 "
                         "계약(HOOK_TARGETS_CONTRACT §5-1 ③)을 재확인하라: %s" % detail3)

    def test_derived_table_from_hook_targets_drives_c74(self):
        """★E3-1 이음매(계약 §5-1 ③): `javis_guard_register --emit-warn-targets` 가 만든
        C74 대상표를 preflight C74 가 **그대로 소비**하는가. 두 도구가 같은 파일을 서로
        다르게 읽으면(예: 행말 주석) '표는 있는데 아무것도 매칭되지 않는' 침묵 미배선이
        된다 — 살아남는 결함은 이음매에 있다."""
        self.unblock()
        import javis_guard_register as gr
        tpath = os.path.join(self.pack, "state", "hook-targets.json")
        doc = {"schema_version": 1, "measured_at": "harness",
               "policy": {"unknown_profile": "deny"},
               "profiles": [
                   {"basename": b, "path": "~/" + b, "role": r,
                    "eligibility": {"guard_stop": gs, "brief_warn": bw}}
                   for b, r, gs, bw in (
                       (".claude", "master", "deny", "allow"),
                       (".claude-3", "master(라이브)", "deny", "allow"),
                       (".claude-worker", "worker", "allow", "allow"),
                       (".claude-2", "unclear", "deny", "deny"))]}
        with open(tpath, "w", encoding="utf-8") as f:
            json.dump(doc, f, ensure_ascii=False)
        table, err = gr._load_targets(tpath)
        self.assertIsNone(err, "대상표 로드 실패: %s" % err)
        out = os.path.join(self.pack, "state", "brief-warn-expected.txt")
        gr.emit_warn_targets(table, out, True, out=open(os.devnull, "w"))

        reg = ('{"hooks":{"PostToolUse":[{"matcher":"Task|Agent","hooks":'
               '[{"type":"command","command":"sh /p/hooks/brief-lint-warn.sh"}]}]}}')
        for b in (".claude", ".claude-3", ".claude-worker"):
            self.mkprofile(b, reg)
        self.mkprofile(".claude-2")                      # deny 대상 — 미등록이어도 무관
        st, detail = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st, PASS,
                         "파생 대상표가 C74 에 소비되지 않았다(이음매 결함): %s" % detail)
        # ★R-04(b) 회귀 핀: allow 인 .claude-3 가 미등록이면 이름을 대며 WARN 이어야 한다
        self.mkprofile(".claude-3")
        st2, detail2 = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st2, WARN, detail2)
        self.assertIn(".claude-3", detail2)

    def test_no_table_scan(self):
        """대상표 미공급 = 전 프로필 스캔 · 등록 0 = WARN(라이브 등록은 OT-2 승인 라인)."""
        self.unblock()
        self.mkprofile(".claude")
        st, detail = self.check("c74_brief_gate_paths", "C74")
        self.assertEqual(st, WARN, detail)
        self.assertIn("경고 훅 등록 0 프로필", detail)


# ══ C75 — 배포 직전 열린+무spec 재스캔 ═══════════════════════════════════════════
class TestC75(Base):
    SPEC = {"mode": "command", "cmd": "true",
            "pass_rule": {"kind": "exit_map", "pass_exits": [0]}, "timeout_s": 5}

    def test_no_board_is_skip(self):
        shutil.rmtree(os.path.join(self.root, "_round"))
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, SKIP, detail)

    def test_under_threshold_pass(self):
        for i in range(5):
            self.mktask("T%d" % i)                       # 열린 + 무spec 5건 = 경계(≤5)
        self.mktask("Tok", spec=self.SPEC)
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, PASS, detail)
        self.assertIn("열린+무spec(비-grandfathered) 5건", detail)

    def test_over_threshold_warn_by_default_fail_under_strict(self):
        """★E1-4 강등 계약: 출하 기본(warn/off)에서는 WARN, strict 추적 중이면 FAIL.
        이 분기가 뒤집히면 팩 install 직후 부트가 오너 승인 없이 NOT READY 로 뒤집힌다."""
        for i in range(6):
            self.mktask("T%d" % i)
        os.environ.pop("JAVIS_VERIFY_GATE", None)
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, WARN, detail)
        self.assertIn("차단 중인 것은", detail)
        os.environ["JAVIS_VERIFY_GATE"] = "strict"
        st2, detail2 = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st2, FAIL, "strict 추적 중인데 FAIL 로 승격 안 됨: %s" % detail2)
        os.environ.pop("JAVIS_VERIFY_GATE")
        # 마커 경로로도 승격되는가(env 없이 grandfather 마커만 있는 실운영 형태)
        import javis_task as _jt
        marker = getattr(_jt, "GATE_MARKER", "")
        self.assertTrue(marker, "javis_task.GATE_MARKER 부재 — 계측기 무효")
        wr(os.path.join(self.tasks, marker), "x")
        st3, detail3 = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st3, FAIL, "마커 경로 승격 실패: %s" % detail3)

    def test_grandfathered_excluded_from_count(self):
        """G3 과계상 수리: grandfathered 는 초과 계수에서 제외하되 목록으로는 표면화."""
        for i in range(6):
            self.mktask("G%d" % i, grandfathered=True)
        os.environ["JAVIS_VERIFY_GATE"] = "strict"
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, PASS, "grandfathered 가 초과 계수에 이중 계상됨: %s" % detail)
        self.assertIn("grandfathered(열린) 6건", detail)
        # closed grandfathered 는 목록이 아니라 분리 집계
        self.mktask("Gdone", status="done", grandfathered=True)
        _st, detail2 = self.check("c75_verify_spec_rescan", "C75")
        self.assertIn("closed 1건 분리 집계", detail2)

    def test_closed_tasks_not_counted(self):
        for i in range(9):
            self.mktask("D%d" % i, status="done")
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, PASS, detail)
        self.assertIn("열린+무spec(비-grandfathered) 0건", detail)

    def test_reserved_sidecars_not_counted(self):
        """예약 사이드카(.guard.json 등)를 태스크로 오인하면 무spec 계수가 폭증한다."""
        for i in range(9):
            with open(os.path.join(self.tasks, "T%d.guard.json" % i), "w") as f:
                json.dump({"id": "T%d" % i, "status": "in_progress"}, f)
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertEqual(st, PASS, detail)
        self.assertIn("열린+무spec(비-grandfathered) 0건", detail)

    def test_parse_failure_is_reported_not_swallowed(self):
        with open(os.path.join(self.tasks, "bad.json"), "w") as f:
            f.write("{ broken")
        st, detail = self.check("c75_verify_spec_rescan", "C75")
        self.assertIn("파싱 실패 1건", detail)


# ══ C76 — 앱 번들 코드서명 봉인(2026-08-01 실사고 M3) ═══════════════════════════════
#
# 계측 타당성: "정상 번들 PASS" 만으로는 아무것도 증명하지 않는다 — 파일 **하나**를 번들 안에
# 넣었을 때 판정이 PASS→WARN 으로 실제로 뒤집히는지(변이검증)까지 잰다. 뒤집히지 않으면 이
# 검사는 실사고를 다시 놓친다.
# 라이브 무접촉: /Applications/cys.app 는 절대 대상이 되지 않는다 — _find_app_bundle 를 픽스처로
# 대체해 임시 디렉터리 안의 번들만 검사한다.
_CODESIGN = "/usr/bin/codesign"
_CAN_SIGN = sys.platform == "darwin" and os.path.exists(_CODESIGN)


class TestC76(Base):
    def _fixture_bundle(self, sign=True):
        """임시 .app 골격 + ad-hoc 서명(라이브 번들 무접촉)."""
        import subprocess
        app = os.path.join(self.td, "Fixture.app")
        os.makedirs(os.path.join(app, "Contents", "MacOS"), exist_ok=True)
        os.makedirs(os.path.join(app, "Contents", "Resources"), exist_ok=True)
        wr(os.path.join(app, "Contents", "Info.plist"),
           '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>'
           '<key>CFBundleExecutable</key><string>fixture</string>'
           '<key>CFBundleIdentifier</key><string>com.example.cysfixture</string>'
           '</dict></plist>')
        wr(os.path.join(app, "Contents", "Resources", "data.txt"), "hello\n")
        # 주 실행파일은 실제 Mach-O 여야 서명이 성립한다(시스템 바이너리 복제 → ad-hoc 재서명).
        # copy2 는 금물 — /bin/echo 의 SIP 제한 플래그(chflags)까지 옮기려다 EPERM 이 난다.
        exe = os.path.join(app, "Contents", "MacOS", "fixture")
        shutil.copyfile("/bin/echo", exe)
        os.chmod(exe, 0o755)
        if sign:
            r = subprocess.run([_CODESIGN, "--force", "--sign", "-", app],
                               capture_output=True, timeout=60)
            self.assertEqual(r.returncode, 0,
                             "픽스처 ad-hoc 서명 실패 — 계측기 무효: %r" % r.stderr)
        return app

    def _use(self, bundle):
        """검사가 볼 번들을 픽스처로 고정(라이브 /Applications 유입 차단)."""
        orig = pf.Preflight._find_app_bundle
        pf.Preflight._find_app_bundle = lambda _self: bundle
        self.addCleanup(setattr, pf.Preflight, "_find_app_bundle", orig)

    def test_non_macos_is_skip_not_fail(self):
        """비 macOS 는 판정 불가 = SKIP. 여기서 FAIL 을 내면 리눅스 부트가 거짓 적색이 된다."""
        import unittest.mock as mock
        with mock.patch.object(pf.sys, "platform", "linux"):
            st, detail = self.check("c76_app_seal", "C76")
        self.assertEqual(st, SKIP, detail)
        self.assertIn("macOS 아님", detail)

    def test_no_bundle_is_skip(self):
        """번들 미발견(비번들 설치·개발 빌드)은 SKIP — 없는 것을 파손으로 적지 않는다."""
        if sys.platform != "darwin":
            self.skipTest("darwin 전용 경로")
        self._use(None)
        st, detail = self.check("c76_app_seal", "C76")
        self.assertEqual(st, SKIP, detail)
        self.assertIn("검사 대상 없음", detail)

    @unittest.skipUnless(_CAN_SIGN, "codesign 부재 — 봉인 검사 판정 불가 환경")
    def test_clean_bundle_pass_then_added_file_flips_to_warn(self):
        """★변이검증: 정상 서명 = PASS / 파일 1개 추가 = WARN(원인 파일·복구 안내 포함)."""
        app = self._fixture_bundle()
        self._use(app)
        st, detail = self.check("c76_app_seal", "C76")
        self.assertEqual(st, PASS, "정상 서명 번들이 PASS 가 아님: %s" % detail)

        # 결함 주입 — 실사고와 같은 모양(번들 안 __pycache__ 생성) 파일 하나.
        pyc = os.path.join(app, "Contents", "Resources", "runtime", "python",
                           "lib", "python3.12", "__pycache__")
        os.makedirs(pyc, exist_ok=True)
        wr(os.path.join(pyc, "_compression.cpython-312.pyc"), "x")
        st2, detail2 = self.check("c76_app_seal", "C76")
        self.assertEqual(st2, WARN, "파일 1개 추가가 WARN 으로 뒤집히지 않음: %s" % detail2)
        self.assertNotEqual(st2, FAIL, "부팅을 막는 FAIL 이어서는 안 된다(WARN 고정 계약)")
        self.assertIn("__pycache__", detail2, "원인 파일이 보고에 없다")
        self.assertIn("자기유발", detail2, "전부 __pycache__ 면 자기유발로 지목해야 한다")
        self.assertIn("App Management", detail2, "복구 안내(부분 수정 차단 이유)가 없다")
        # 진단은 읽기 전용 — 주입한 파일을 지우지 않는다(부분 수리 금지).
        self.assertTrue(os.path.exists(os.path.join(pyc, "_compression.cpython-312.pyc")),
                        "preflight 가 번들 안 파일을 삭제했다(부분 수리 금지 위반)")

    @unittest.skipUnless(_CAN_SIGN, "codesign 부재 — 봉인 검사 판정 불가 환경")
    def test_modified_file_is_also_caught(self):
        """추가뿐 아니라 기존 sealed 파일 '수정'도 잡아야 한다(파손 갈래 누락 방지)."""
        app = self._fixture_bundle()
        self._use(app)
        wr(os.path.join(app, "Contents", "Resources", "data.txt"), "tampered")
        st, detail = self.check("c76_app_seal", "C76")
        self.assertEqual(st, WARN, detail)
        self.assertIn("수정 1건", detail)
        self.assertNotIn("자기유발", detail, "__pycache__ 아닌 파손에 자기유발을 단정하면 오진")

    @unittest.skipUnless(_CAN_SIGN, "codesign 부재 — 봉인 검사 판정 불가 환경")
    def test_unsigned_bundle_is_warn_not_false_alarm_of_seal_break(self):
        """미서명 개발 번들: codesign 은 실패하지만 '봉인 파손 파일'을 단정하지 않는다."""
        app = self._fixture_bundle(sign=False)
        self._use(app)
        st, detail = self.check("c76_app_seal", "C76")
        self.assertIn(st, (PASS, WARN), detail)
        if st == WARN:
            self.assertIn("특정되지 않음", detail)


if __name__ == "__main__":
    unittest.main(verbosity=2)
