"""release-postprocess.py 의 Gatekeeper 게이트 훅(F2)의 밀폐 unittest — 네트워크·토큰·실 DMG 불요.

★왜 이 테스트가 필요한가
  후처리(`scripts/release-postprocess.py`)는 **발행될 실물 바이트**(draft 백업 자산)를 만지는
  마지막 로컬 지점이다. 거기 신설된 Gatekeeper 게이트(`gatekeeper_gate`)가 fail-open 이면 —
  게이트 rc 1(FAIL)·2(판정 불가)를 통과로 세거나, macOS 밖에서 조용히 skip 하거나, main 이
  업로드 **뒤에** 게이트를 부르면 — 봉인 파손 DMG 가 그대로 --apply 되어 2026-08-01 사고
  ("손상되었기 때문에 열 수 없습니다")가 발행 층위에서 재발한다.
  **"통과해선 안 될 rc"를 여기 전부 못박는다.**

★설계
  실제 게이트 스크립트(hdiutil·spctl — macOS 전용·무겁다) 대신 **페이크 게이트**(호출을 기록
  파일에 남기고 지정 rc 로 죽는 셸 스크립트)를 주입한다. 주입 지점은 `gatekeeper_gate` 의
  테스트 전용 키워드 인자(gate_script·user_path_script·sys_platform·machine)다.
  실물 대조는 별도다 — `python3 scripts/release-postprocess.py v0.14.19`(dry-run · 백업 실물
  DMG 에 실평가). 경로는 전부 tempfile — 개인 경로·실 홈 디렉터리 금지(test_release_verify.py 관례).

★확장(2026-08-20 · codex REVISE 수리): 이 파일은 release-gate-gatekeeper.sh 의 두 계약도 박제한다.
  · F1 — SEAL-2 전칭 검사 적대 픽스처 2종(총계 상쇄·표본 밖 flags 변조 → FAIL) — 합성 트리
    (tempfile · 라이브 앱·저장소 무접촉)에 --seal2-only(진단 전용 · ⑤ 단독)로 실검증.
  · F2 — degraded(spctl 실평가 불능)=판정 불가(exit 2) 폐쇄 + 진단 플래그
    (--diagnose-degraded-ok·--seal2-only)가 발행 경로(release-postprocess.py·release.yml)에
    실리지 않는다는 문자열 핀 2건.

사용: python3 scripts/tests/test_release_postprocess_gate.py
"""

import contextlib
import importlib.util
import io
import os
import struct
import subprocess
import sys
import tempfile
import unittest

# ★하이픈 파일명(`release-postprocess.py`)은 `import` 문으로 못 부른다 — importlib 로 직접
#   적재한다(test_release_verify.py 와 같은 이유·같은 관례).
_RP_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "release-postprocess.py")
_spec = importlib.util.spec_from_file_location("release_postprocess", _RP_PATH)
rp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rp)

_HERE = os.path.dirname(os.path.abspath(__file__))
_GATE_SH = os.path.join(_HERE, "..", "release-gate-gatekeeper.sh")
_RELEASE_YML = os.path.join(_HERE, "..", "..", ".github", "workflows", "release.yml")

V = "0.14.19"


class GatekeeperGateHookTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = self._tmp.name
        for arch in ("aarch64", "x64"):
            with open(os.path.join(self.root, "cys_%s_%s.dmg" % (V, arch)), "wb") as fh:
                fh.write(b"\x78\x01fake-dmg-" + arch.encode())
        self.log = os.path.join(self.root, "calls.log")

    def tearDown(self):
        self._tmp.cleanup()

    # ── 헬퍼 ──────────────────────────────────────────────────────────────
    def fake_gate(self, name, rc):
        """호출을 기록하고 지정 rc 로 끝나는 페이크 게이트를 만든다(exit 0/1/2 주입 지점)."""
        path = os.path.join(self.root, name)
        with open(path, "w") as fh:
            fh.write('#!/bin/sh\necho "%s $*" >> "%s"\nexit %d\n' % (name, self.log, rc))
        os.chmod(path, 0o755)
        return path

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return fh.read().splitlines()

    def run_gate(self, gate_rc=0, user_rc=0, **kw):
        kw.setdefault("sys_platform", "darwin")
        kw.setdefault("machine", "arm64")
        err = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            rc = rp.gatekeeper_gate(self.root, V,
                                    gate_script=self.fake_gate("gate.sh", gate_rc),
                                    user_path_script=self.fake_gate("userpath.sh", user_rc),
                                    **kw)
        return rc, err.getvalue()

    # ── 0. 기준선 ─────────────────────────────────────────────────────────
    def test_00_exit0_passes_and_covers_both_dmgs(self):
        """이게 깨지면 정상 릴리스가 발행 불능(과차단) — 또는 DMG 한쪽이 무검증으로 샌다."""
        rc, _ = self.run_gate(gate_rc=0, user_rc=0)
        self.assertEqual(rc, 0)
        static_calls = [c for c in self.calls() if c.startswith("gate.sh")]
        self.assertEqual(len(static_calls), 2, "정적판이 DMG 2종 전부를 보지 않았다")
        joined = "\n".join(static_calls)
        self.assertIn("cys_%s_aarch64.dmg" % V, joined)
        self.assertIn("cys_%s_x64.dmg" % V, joined)

    def test_01_native_dmg_also_gets_user_path_gate(self):
        """arm64 맥에서 aarch64 DMG 에 상위판(⑥ 봉인 자기파괴 재현)이 안 얹히면
        2026-08-01 사고 경로(.pyc 봉인 자기파괴)가 발행 층위에서 무검증으로 남는다."""
        rc, _ = self.run_gate(machine="arm64")
        self.assertEqual(rc, 0)
        user_calls = [c for c in self.calls() if c.startswith("userpath.sh")]
        self.assertEqual(len(user_calls), 1)
        self.assertIn("cys_%s_aarch64.dmg" % V, user_calls[0])

    def test_02_nonnative_dmg_stays_static_only(self):
        """arm64 에서 x64 DMG 에 상위판을 걸면 Rosetta 2 의존으로 구조적 FAIL 이 되고,
        늘 빨간 게이트는 사람이 끄게 된다(release-gate-gatekeeper.sh 머리 주석
        「verify-gatekeeper-user-path.sh 와의 관계」의 분리 근거)."""
        self.run_gate(machine="arm64")
        self.assertEqual([c for c in self.calls()
                          if c.startswith("userpath.sh") and ("cys_%s_x64.dmg" % V) in c], [])

    def test_03_intel_host_flips_native_to_x64(self):
        """호스트 아키 판정이 고정(aarch64)이면 Intel 맥에서 돌릴 때 상위판이
        자기가 실행 못 하는 DMG 를 잡아 구조적 FAIL — native 는 호스트를 따라야 한다."""
        rc, _ = self.run_gate(machine="x86_64")
        self.assertEqual(rc, 0)
        user_calls = [c for c in self.calls() if c.startswith("userpath.sh")]
        self.assertEqual(len(user_calls), 1)
        self.assertIn("cys_%s_x64.dmg" % V, user_calls[0])

    # ── 1. fail-closed: 통과해선 안 될 rc ─────────────────────────────────
    def test_04_gate_exit1_blocks_with_nonzero(self):
        """rc=1(FAIL)을 통과로 세면 봉인 파손 DMG 가 그대로 --apply 된다."""
        rc, err = self.run_gate(gate_rc=1)
        self.assertEqual(rc, 1)
        self.assertIn("::error::", err)

    def test_05_gate_exit2_blocks_identically(self):
        """rc=2(판정 불가)를 통과로 세면 '측정 불능=통과' 구멍 — 1과 똑같이 죽어야 한다."""
        rc, err = self.run_gate(gate_rc=2)
        self.assertEqual(rc, 2)
        self.assertIn("::error::", err)

    def test_06_user_path_failure_blocks_even_when_static_passed(self):
        """정적판 PASS 가 상위판 FAIL 을 덮으면 ⑥(봉인 자기파괴)이 장식이 된다."""
        rc, _ = self.run_gate(gate_rc=0, user_rc=1)
        self.assertEqual(rc, 1)

    def test_07_non_macos_is_fail_closed_not_skip(self):
        """macOS 밖 무음 skip = 무검증 발행. 판정 불가(비영)로 죽고, 게이트를 돌린
        척(호출 기록)도 남기면 안 된다."""
        rc, err = self.run_gate(sys_platform="linux")
        self.assertEqual(rc, 2)
        self.assertIn("::error::", err)
        self.assertEqual(self.calls(), [])

    def test_08_missing_dmg_is_fail_closed(self):
        """대상 DMG 부재를 통과로 세면 '자산이 없어서 검사를 못 한 묶음'이 발행된다."""
        os.remove(os.path.join(self.root, "cys_%s_x64.dmg" % V))
        rc, _ = self.run_gate()
        self.assertEqual(rc, 2)

    # ── 2. 비상 탈출구 ────────────────────────────────────────────────────
    def test_09_unsafe_skip_opens_loudly_and_runs_nothing(self):
        """탈출구는 열리되 **조용히** 열리면 안 된다 — LOUD 경고 2줄이 사라지면
        평시 우회 플래그로 변질된다(--force-no-verify 선례 동형)."""
        rc, err = self.run_gate(unsafe_skip=True)
        self.assertEqual(rc, 0)
        self.assertEqual(self.calls(), [])
        loud = [ln for ln in err.splitlines() if ln.startswith("!!!!")]
        self.assertGreaterEqual(len(loud), 2, "LOUD 경고 2줄 계약 위반: %r" % err)


class MainWiringContractTests(unittest.TestCase):
    """main() 배선 계약 — 함수가 아무리 옳아도 main 이 안 부르거나(또는 업로드 뒤에 부르면)
    게이트는 장식이다. main 은 token()이 필요해 밀폐 실행이 불가하므로 원문으로 박제한다
    (javis_cycle_verifier.py 의 소스 계약 검사 관례)."""

    def setUp(self):
        with open(_RP_PATH, encoding="utf-8") as fh:
            self.src = fh.read()

    def test_10_gate_called_after_selfverify_and_before_upload(self):
        call = self.src.index("gate_rc = gatekeeper_gate(")
        selfverify = self.src.index("자기 검증 통과")
        upload = self.src.index("── 6. 업로드")
        self.assertGreater(call, selfverify, "게이트가 자기 검증(4단계)보다 앞이다")
        self.assertLess(call, upload, "게이트가 업로드 뒤다 — --apply 를 못 막는다")

    def test_11_unsafe_flag_parsed_from_argv_default_off(self):
        self.assertIn('"--unsafe-skip-gatekeeper" in argv', self.src)

    def test_12_gate_return_value_terminates_main(self):
        """반환값을 버리면(호출만 하면) 비영 rc 가 통과로 둔갑한다."""
        self.assertIn("if gate_rc:\n        return gate_rc", self.src)


class DiagnoseFlagAbsencePins(unittest.TestCase):
    """F2 핀 — 진단 전용 플래그(--diagnose-degraded-ok·--seal2-only)가 발행 경로에 실리는
    순간 빨개진다. 게이트 스크립트가 아무리 옳아도 발행 경로가 진단 플래그를 실으면
    degraded 가 도로 rc=0 이 된다 — 문자열 층위에서 못박는다(F2 수리 2026-08-20)."""

    DIAG_FLAGS = ("--diagnose-degraded-ok", "--seal2-only")

    def test_13_postprocess_source_carries_no_diagnose_flag(self):
        with open(_RP_PATH, encoding="utf-8") as fh:
            src = fh.read()
        for flag in self.DIAG_FLAGS:
            self.assertNotIn(flag, src,
                             "release-postprocess.py 가 진단 전용 플래그를 실었다: %s" % flag)

    def test_14_release_yml_carries_no_diagnose_flag(self):
        with open(_RELEASE_YML, encoding="utf-8") as fh:
            yml = fh.read()
        # 핀의 전제: 게이트 스텝 실재 — 스텝 자체가 사라지면 플래그 부재 단언은 공허하다.
        self.assertIn("release-gate-gatekeeper.sh", yml,
                      "게이트 스텝이 release.yml 에서 사라졌다 — 무검증 발행 경로")
        for flag in self.DIAG_FLAGS:
            self.assertNotIn(flag, yml,
                             "release.yml 이 진단 전용 플래그를 실었다: %s" % flag)


class Seal2UniversalCheckTests(unittest.TestCase):
    """F1 적대 픽스처 — SEAL-2 전칭 검사(파일별 대응·고아·flags 전수)를 합성 트리로 박제.

    구판(레벨별 총계 동일성 + 표본 25개 flags)이 통과시키던 두 결함을 FAIL 로 못박는다:
      (a) 결손 1 + 동수 고아 1 = 총계 상쇄   (b) 구판 표본 밖 1개 flags 변조.
    트리는 tempfile 합성(라이브 앱·저장소 무접촉) — 검사는 파일명 + 헤더 8바이트만 보므로
    pyc 본문은 위조로 충분하다. 호출은 --seal2-only(⑤ 단독 · hdiutil/spctl 불요)."""

    TAG = "cpython-312"   # 픽스처 안 태그 — 게이트는 이 값을 하드코딩하지 않고 파일명에서 추출한다

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.app = os.path.join(self._tmp.name, "Fake.app")
        self.pkg = os.path.join(self.app, "Contents", "Resources", "runtime",
                                "python", "lib", "pkg")
        self.cache = os.path.join(self.pkg, "__pycache__")
        os.makedirs(self.cache)
        # 3N > 25(구판 표본 상한) — "표본 밖" 변조 지점이 실제로 존재하도록 N=12(pyc 36개).
        for i in range(12):
            with open(os.path.join(self.pkg, "m%02d.py" % i), "w") as fh:
                fh.write("x = %d\n" % i)
            for opt in ("", ".opt-1", ".opt-2"):
                self._pyc(os.path.join(self.cache, "m%02d.%s%s.pyc" % (i, self.TAG, opt)))

    def tearDown(self):
        self._tmp.cleanup()

    def _pyc(self, path, flags=1):
        with open(path, "wb") as fh:
            fh.write(b"\x6f\x0d\x0d\x0a")          # magic 4B — 게이트는 값 대조를 안 한다(버전 무관)
            fh.write(struct.pack("<I", flags))     # flags 4B (PEP 552 · 1 = unchecked-hash)
            fh.write(b"\x00" * 8)                  # source-hash 8B — 내용 무관

    def _run(self):
        p = subprocess.run(["bash", _GATE_SH, "--seal2-only", self.app],
                           capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr

    def test_15_baseline_synthetic_tree_passes(self):
        """기준선 rc=0 — 이게 깨지면 아래 FAIL 단언들은 '무조건 빨간 검사'를 오독한 것이다."""
        rc, out = self._run()
        self.assertEqual(rc, 0, out)
        self.assertIn("OK:", out)

    def test_16_missing_plus_orphan_cancellation_fails(self):
        """(a) 한 소스의 pyc 1종 삭제 + 동수 고아 추가 — 레벨별 총계가 그대로라 구판은 초록."""
        os.remove(os.path.join(self.cache, "m00.%s.opt-2.pyc" % self.TAG))
        self._pyc(os.path.join(self.cache, "ghost.%s.opt-2.pyc" % self.TAG))  # 소스 없는 고아
        rc, out = self._run()
        self.assertEqual(rc, 1, out)
        self.assertIn("MISSING", out)
        self.assertIn("ORPHAN", out)

    def test_17_out_of_sample_flags_tamper_fails(self):
        """(b) 구판 표본(정렬 선두 25개) 밖의 1개를 flags=3 으로 변조 — 전수 판독만 잡는다."""
        victim = sorted(os.listdir(self.cache))[-1]   # 정렬 마지막 = 36개 중 36번째 — 구판 [:25] 밖
        self._pyc(os.path.join(self.cache, victim), flags=3)
        rc, out = self._run()
        self.assertEqual(rc, 1, out)
        self.assertIn("BADFLAGS", out)


@unittest.skipUnless(sys.platform == "darwin",
                     "게이트 전체 실행은 macOS 도구(hdiutil·spctl 등)가 필요하다")
class DegradedClosureTests(unittest.TestCase):
    """F2 — degraded(spctl 실평가 불능)는 기본 모드에서 판정 불가(exit 2)로 폐쇄된다.
    주입점 CYS_GATE_FORCE_DEGRADED=1 은 degraded **방향으로만** 강제한다(full 을 강제하는
    주입점은 우회 벡터라 없다)."""

    def _mini_app(self, root):
        pkg = os.path.join(root, "Fake.app", "Contents", "Resources", "runtime",
                           "python", "lib", "pkg")
        cache = os.path.join(pkg, "__pycache__")
        os.makedirs(cache)
        with open(os.path.join(pkg, "a.py"), "w") as fh:
            fh.write("x = 1\n")
        for opt in ("", ".opt-1", ".opt-2"):
            with open(os.path.join(cache, "a.cpython-312%s.pyc" % opt), "wb") as fh:
                fh.write(b"\x6f\x0d\x0d\x0a" + struct.pack("<I", 1) + b"\x00" * 8)
        return os.path.join(root, "Fake.app")

    def test_18_degraded_closes_exit2_with_gate_mode_line(self):
        env = dict(os.environ, CYS_GATE_FORCE_DEGRADED="1")
        p = subprocess.run(["bash", _GATE_SH, "/nonexistent-target.dmg"],
                           capture_output=True, text=True, env=env)
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        # 폐쇄 직전 GATE_MODE=degraded 를 stdout 마지막 줄로 출력한다(헤더 GATE_MODE 계약).
        lines = [ln for ln in p.stdout.splitlines() if ln.strip()]
        self.assertEqual(lines[-1], "GATE_MODE=degraded", p.stdout)
        # 폐쇄는 대상 평가 이전이다 — 개별 검사(①-⑤)가 하나도 돌지 않아야 한다.
        self.assertNotIn("── 대상 앱", p.stdout)

    def test_19_diagnose_flag_keeps_degraded_open_with_loud_notice(self):
        """진단 옵트인은 폐쇄를 열되(판정 도달 exit 0·1) LOUD 고지를 남긴다 — 가짜 앱은
        codesign 에서 FAIL 이므로 판정 도달 = exit 1 + GATE_MODE=degraded 마지막 줄."""
        with tempfile.TemporaryDirectory() as td:
            app = self._mini_app(td)
            env = dict(os.environ, CYS_GATE_FORCE_DEGRADED="1")
            p = subprocess.run(["bash", _GATE_SH, "--diagnose-degraded-ok", app],
                               capture_output=True, text=True, env=env)
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)   # 판정 도달(폐쇄 아님)
        lines = [ln for ln in p.stdout.splitlines() if ln.strip()]
        self.assertEqual(lines[-1], "GATE_MODE=degraded", p.stdout)
        loud = [ln for ln in p.stdout.splitlines() if ln.startswith("!!!!")]
        self.assertGreaterEqual(len(loud), 2, "LOUD 고지 2줄 계약 위반: %r" % p.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
