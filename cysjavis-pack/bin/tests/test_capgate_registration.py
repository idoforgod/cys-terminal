#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_capgate_registration.py — WP-3 A(0.14.31) 능력 게이트 **등록 조건**과 C82 배선의 회귀를 막는다.

초안 codex(gpt-6-astra) · 워커 전 행 검토 후 채택 + 판정 불능 갈래(§2) 보강.

무엇을 막는가:
  ① 등록 조건 두 개(데몬 `alert_route.enabled` ∧ 설치본 지침 신판 표지)가 OR 로 느슨해지거나
     한쪽만 보고하는 회귀 — 부분 배포(A만 등록)는 CSO 가 경보 없이 능력만 잃는 상태이고
     그것이 봉인표 ③(자가치유 전멸)의 실현이다.
  ② `enabled` 를 truthiness 로 읽는 회귀(`1`·`"true"` — 파이썬에서 `1 == True` 다).
  ③ 표지를 부분 문자열로 읽는 회귀(표지를 **인용한 산문**이 신판으로 오독된다).
  ④ 판정 불능(구 바이너리 rc≠0 · `cys` 부재 · 지침 판독 실패)을 '등록해도 됨' 으로 접는 회귀.
  ⑤ C82 가 동사 부재를 PASS('드리프트 없음')로 접거나 측정 시각을 빠뜨리는 회귀.
  ⑥ capgate 를 상시 등록 목록(`SELFCORR_HOOKS`)에 편입하거나 matcher 를 다는 회귀.
  ⑦ eligibility 선택 키를 필수로 바꿔 **구 운영 표 전체를 손상으로 만드는** 회귀
     (그러면 stop·brief-warn 등록까지 함께 죽는다).

무엇을 **막지 못하는가**(정직): 등록 조건이 참이어도 경보가 실제로 CSO inbox 에 배달되는지는
여기서 재지 않는다 — 그것은 WP-3 B 의 드릴 소관이다. 이 검체는 "등록해도 되는가" 만 판정한다.

라이브 무접촉: HOME·PATH·CYS_PACK_DIR·TMPDIR 전부 임시 디렉터리이고 `cys`·`claude` 는 스텁이다.

실행: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_capgate_registration.py
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

BIN = Path(__file__).resolve().parent.parent
PACK = BIN.parent
sys.path.insert(0, str(BIN))
# 검체 실행이 배포 원본에 __pycache__ 파일을 추가하지 않게 한다(SEAL-1 census 와 같은 목적).
# ★환경변수 이름을 이 파일에 **적지 않는다**: `test_pyseal_census.py` 의 참조 파일 집합 핀은
#   그 문자열을 담은 파일을 '새 python 강제점' 으로 보고 등재를 요구한다. 여기서 필요한 것은
#   이 프로세스의 바이트코드 억제뿐이고, 훅이 스폰하는 python 은 `_lib.sh` 가 이미 봉인한다.
sys.dont_write_bytecode = True
import javis_preflight as pf  # noqa: E402
import javis_guard_register as gr  # noqa: E402


class _CapgateEnv(unittest.TestCase):
    """격리 환경 + 스텁 헬퍼(테스트 없음). 두 테스트 클래스가 공유한다 —
    상속으로 재실행되면 같은 케이스가 두 번 돌아 실행 시간만 두 배가 된다."""

    def setUp(self):
        # 매 테스트마다 환경 전체를 교체해 상속된 소켓·CYS_BIN·프로필 우회를 없앤다.
        tmp = tempfile.TemporaryDirectory(prefix="capgate-registration-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.home = self.root / "home"
        self.bin = self.root / "bin"
        self.pack = self.root / "pack"
        self.javis = self.root / "javis"
        self.scratch = self.root / "tmp"
        for directory in (self.home, self.bin, self.pack / "directives",
                          self.javis, self.scratch):
            directory.mkdir(parents=True)
        self.calls = self.root / "calls.log"
        self.calls.write_text("", encoding="utf-8")
        env = mock.patch.dict(os.environ, {
            "HOME": str(self.home), "PATH": str(self.bin),
            "CYS_PACK_DIR": str(self.pack), "JAVIS_ROOT": str(self.javis),
            "TMPDIR": str(self.scratch),
            "CYS_PY": str(self.bin / "python3"),
        }, clear=True)
        env.start()
        self.addCleanup(env.stop)
        previous_cwd = os.getcwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, previous_cwd)
        # PATH에는 임시 디렉터리 하나만 두고 필요한 인터프리터/유틸만 연결한다.
        # cys/claude는 아래의 인자 검증 스텁 외에는 발견될 수 없다.
        (self.bin / "python3").symlink_to(sys.executable)
        (self.bin / "bash").symlink_to("/bin/bash")
        (self.bin / "dirname").symlink_to("/usr/bin/dirname")
        self.good_status = {"alert_route": {"enabled": True}}
        self.new_directive = "# CSO\n%s\n본문\n" % pf.CSO_DIRECTIVE_REV_MARKER
        self.directive = self.pack / "directives" / "CSO_DIRECTIVE.md"
        self.directive.write_text(self.new_directive, encoding="utf-8")

    def _stub(self, name, responses):
        """실제 subprocess로 실행되며 예상하지 않은 동사는 실패하고 로그에 남는다."""
        lines = ["#!/bin/sh", "printf '%%s\\n' \"%s $*\" >> %s"
                 % (name, shlex.quote(str(self.calls))), 'case "$*" in']
        for args, (rc, output) in responses.items():
            lines.append("  %s) printf '%%s\\n' %s; exit %d ;;"
                         % (shlex.quote(args), shlex.quote(output), rc))
        lines.extend(["  *) echo 'unexpected stub arguments' >&2; exit 97 ;;", "esac"])
        path = self.bin / name
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        path.chmod(0o755)

    def _status_stub(self, status, rc=0):
        self._stub("cys", {"status --json": (rc, json.dumps(status))})

    def _assert_calls(self, expected):
        self.assertEqual(self.calls.read_text(encoding="utf-8").splitlines(), expected,
                         "예정한 스텁 명령만 정확히 호출해야 한다")

    def _assert_registration(self, status, text, expected, missing=(), present=(), rc=0):
        self._status_stub(status, rc)
        self.directive.write_text(text, encoding="utf-8")
        # 설치본 판독이 예외를 내더라도 순수 판정의 검증 결과는 별도로 남긴다.
        checks = [("순수 판정", lambda: pf.capgate_registration_verdict(
            status if rc == 0 else None, text)),
                  ("설치본 판정", pf.Preflight(fix=False, skips=[])._capgate_gate)]
        for source, check in checks:
            with self.subTest(source=source):
                ok, why = check()
                self.assertIs(ok, expected, "%s: 등록 허용은 %r 기대, 사유=%s"
                              % (source, expected, why))
                self.assertIsInstance(why, str, "판정 사유는 문자열이어야 한다")
                for condition in missing:
                    self.assertIn(condition, why, "미충족 조건 %r을 사유에 밝혀야 한다" % condition)
                for condition in present:
                    self.assertNotIn(condition, why, "충족 조건 %r을 결핍으로 보고하면 안 된다" % condition)
        self._assert_calls(["cys status --json"])

    def _c82(self):
        preflight = pf.Preflight(fix=False, skips=[])
        preflight.c82_gate_corpus_drift()
        self.assertEqual(len(preflight.results), 1, "C82 결과는 정확히 한 건이어야 한다")
        result = preflight.results[0]
        self.assertEqual(result["id"], "C82.gate-corpus-drift", "C82 결과 식별자를 유지해야 한다")
        return result


class CapgateRegistration(_CapgateEnv):
    """등록 조건 · C82 · 배선 계약."""

    def test_both_conditions_enable_registration(self):
        # AND 조건을 과도하게 닫아 정상 신판도 등록되지 않는 회귀를 잡는다.
        self.assertIs(pf.capgate_alert_route_enabled(self.good_status), True,
                      "JSON boolean true는 경보 라우팅 지원이어야 한다")
        self.assertIs(pf.capgate_marker_ok(self.new_directive), True,
                      "첫 20행 내 독립 표지는 신판이어야 한다")
        self._assert_registration(self.good_status, self.new_directive, True)


    def test_only_daemon_support_missing(self):
        # 지침은 준비됐어도 데몬 미지원이면 경보 없는 능력 제한을 등록하면 안 된다.
        self._assert_registration({"alert_route": {"enabled": False}}, self.new_directive,
                                  False, ("데몬 alert_route 미지원",), ("신판 표지",))


    def test_only_directive_marker_missing(self):
        # 데몬만 배포된 상태를 준비 완료로 접는 OR 조건 회귀를 잡는다.
        self._assert_registration(self.good_status, "# CSO\n구판 지침\n", False,
                                  ("설치본 CSO_DIRECTIVE", "신판 표지"), ("데몬 alert_route",))


    def test_both_conditions_missing(self):
        # 둘 다 빠졌을 때 한 조건만 보고하면 부분 배포 원인을 놓친다.
        self._assert_registration({"alert_route": {"enabled": False}}, "# 구판\n", False,
                                  ("데몬 alert_route 미지원", "설치본 CSO_DIRECTIVE", "신판 표지"))


    def test_old_status_nonzero_defers_registration(self):
        # 성공 JSON이 stdout에 있어도 실패 종료를 지원 증거로 사용하면 안 된다.
        self._assert_registration(self.good_status, self.new_directive, False,
                                  ("데몬 alert_route 미지원",), rc=2)


    def test_old_status_without_alert_route_defers_registration(self):
        # 구 status 스키마의 결측을 기본 true로 보정하는 회귀를 잡는다.
        self._assert_registration({}, self.new_directive, False, ("데몬 alert_route 미지원",))


    def test_prose_quotation_is_not_a_revision_marker(self):
        # substring 검사로 바뀌면 표지를 설명하는 구판 산문까지 신판이 된다.
        text = "# CSO\n표지 %s 를 확인하라.\n" % pf.CSO_DIRECTIVE_REV_MARKER
        self.assertIs(pf.capgate_marker_ok(text), False, "산문에 인용된 표지는 신판이 아니다")
        self._assert_registration(self.good_status, text, False, ("신판 표지",))


    def test_marker_line_boundary(self):
        # 첫 20행 포함 경계와 strip 후 정확 행 등가가 함께 유지되어야 한다.
        marker = pf.CSO_DIRECTIVE_REV_MARKER
        self.assertIs(pf.capgate_marker_ok("\n" * 19 + "  " + marker + "  \n"), True,
                      "20번째 행의 공백으로 둘러싸인 정확 표지는 유효해야 한다")
        self.assertIs(pf.capgate_marker_ok("\n" * 20 + marker), False,
                      "21번째 행 표지는 유효 범위 밖이어야 한다")


    def test_integer_enabled_is_unsupported(self):
        # Python의 1 == True 때문에 동등 비교로 느슨해지는 회귀를 잡는다.
        status = {"alert_route": {"enabled": 1}}
        self.assertIs(pf.capgate_alert_route_enabled(status), False, "정수 1은 boolean true가 아니다")
        self._assert_registration(status, self.new_directive, False, ("데몬 alert_route 미지원",))


    def test_string_enabled_is_unsupported(self):
        # 비어 있지 않은 문자열의 truthiness를 지원 여부로 읽으면 안 된다.
        status = {"alert_route": {"enabled": "true"}}
        self.assertIs(pf.capgate_alert_route_enabled(status), False, "문자열 true는 boolean true가 아니다")
        self._assert_registration(status, self.new_directive, False, ("데몬 alert_route 미지원",))


    def test_old_binary_gate_corpus_is_skip(self):
        # 미지원 동사를 PASS(드리프트 없음)나 치명 실패로 오독하는 회귀를 잡는다.
        self._stub("cys", {"gate-corpus --json": (2, "error: unrecognized subcommand 'gate-corpus'")})
        result = self._c82()
        self.assertEqual(result["status"], pf.SKIP, "구 바이너리는 C82 SKIP 기대")
        self.assertIn("구 바이너리", result["detail"], "SKIP 사유에 구 바이너리를 명시해야 한다")
        self._assert_calls(["cys gate-corpus --json"])

    def _assert_c82_version(self, live, expected):
        self._stub("cys", {"gate-corpus --json": (0, json.dumps({
            "measured_on": "2.1.241", "gates": []}))})
        self._stub("claude", {"--version": (0, live + " (Claude Code)")})
        # 시계만 고정하고 버전 조회는 실제 subprocess로 수행한다.
        measured_at = "2026-09-07 12:34:56+0900"
        with mock.patch.object(pf.time, "strftime", return_value=measured_at):
            result = self._c82()
        self.assertEqual(result["status"], expected, "claude=%s일 때 C82 %s 기대" % (live, expected))
        for evidence in ("measured_on=2.1.241", "claude=" + live, "측정 " + measured_at):
            self.assertIn(evidence, result["detail"], "C82 detail에 근거 %r이 있어야 한다" % evidence)
        self._assert_calls(["cys gate-corpus --json", "claude --version"])

    def test_c82_version_drift_warns_with_measurement_time(self):
        # 설치 버전이 달라졌는데 PASS하거나 측정 시각을 생략하는 회귀를 잡는다.
        self._assert_c82_version("2.1.261", pf.WARN)

    def test_c82_matching_version_passes_with_measurement_time(self):
        # 일치 분기도 시각이 없으면 언제 확인한 결과인지 알 수 없다.
        self._assert_c82_version("2.1.241", pf.PASS)

    def test_registration_wiring(self):
        # 상시 등록 목록 편입·matcher 추가는 조건부 등록/전 도구 관찰 계약을 깨뜨린다.
        self.assertNotIn("role-capability-gate.sh", [name for name, _ in pf.SELFCORR_HOOKS],
                         "capgate는 상시 SELFCORR_HOOKS에 없어야 한다")
        self.assertEqual(pf.CAPGATE_HOOK, ("role-capability-gate.sh", [("PreToolUse", None)]),
                         "CAPGATE_HOOK은 matcher 없는 PreToolUse 튜플이어야 한다")
        self.assertEqual(pf.HOOK_TIMEOUT_S[("role-capability-gate.sh", "PreToolUse")], 15,
                         "preflight capgate timeout은 15초여야 한다")
        spec = gr.HOOKS["capgate"]
        self.assertEqual(spec["event"], "PreToolUse", "등록 도구도 PreToolUse여야 한다")
        self.assertNotIn("matcher", spec, "matcher는 None 값도 아닌 키 자체 부재여야 한다")
        self.assertEqual(spec["timeout"], 15, "등록 도구 timeout도 15초여야 한다")
        self.assertEqual(spec["script"], "hooks/role-capability-gate.sh", "등록할 훅 경로가 일치해야 한다")
        self.assertEqual(gr.HOOK_ELIGIBILITY_KEY["capgate"], "capgate", "대상표 키는 capgate여야 한다")
        self.assertEqual(gr.REQUIRED_ELIGIBILITY_KEYS, ("guard_stop", "brief_warn"),
                         "구 대상표의 필수 키 두 개를 유지해야 한다")
        self.assertEqual(gr.ELIGIBILITY_DEFAULT, {"capgate": "allow"}, "capgate 결측 기본값은 allow여야 한다")

    def test_legacy_targets_without_capgate_are_valid_and_allowed(self):
        # 선택 키를 필수로 바꾸면 구 운영표 전체가 손상이 되어 기존 훅까지 멈춘다.
        path = self.pack / "state" / "hook-targets.json"
        path.parent.mkdir()
        path.write_text(json.dumps({
            "schema_version": 1, "policy": {"unknown_profile": "deny"},
            "profiles": [{"basename": ".claude", "role": "master",
                          "eligibility": {"guard_stop": "deny", "brief_warn": "allow"}}],
        }), encoding="utf-8")
        table, err = gr._load_targets(str(path))
        self.assertIsNone(err, "capgate 키 없는 구 대상표는 손상이 아니어야 한다: %s" % err)
        self.assertIsNotNone(table, "구 대상표 실물을 로드해야 하며 표 부재 폴백이면 안 된다")
        ok, why = gr._decide(table, ".claude", "capgate", gr.HOOKS["capgate"],
                             force_master=False, force_unknown=False)
        self.assertIs(ok, True, "구 대상표 capgate는 강제 우회 없이 allow 기대: %s" % why)

    def test_hook_self_test_exits_zero(self):
        # 라이브러리 부재의 조기 exit 0을 self-test 성공으로 착각하지 않도록 출력도 잰다.
        hooks = self.pack / "hooks"
        hooks.mkdir()
        for name in ("role-capability-gate.sh", "_lib.sh"):
            shutil.copyfile(PACK / "hooks" / name, hooks / name)
        self._stub("cys", {})
        result = subprocess.run([str(self.bin / "bash"), str(hooks / "role-capability-gate.sh"),
                                 "--self-test"], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, "훅 self-test exit 0 기대:\n%s\n%s"
                         % (result.stdout, result.stderr))
        self.assertIn("self-test OK:", result.stdout, "강등 종료가 아니라 내장 검체 완주가 필요하다")
        self._assert_calls([])


class CapgateUndecidable(_CapgateEnv):
    """★판정 불능 갈래 — '잴 수 없다' 를 '괜찮다' 로 접는 회귀를 막는다(결측은 값이 아니다)."""

    def test_missing_cys_binary_defers_registration(self):
        # PATH 에 cys 가 없으면 데몬 지원 여부를 **알 수 없다** — 그것은 등록 근거가 아니다.
        ok, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertIs(ok, False, "cys 부재인데 등록을 허용했다: %s" % why)
        self.assertIn("cys 바이너리 미발견", why, "판정 불능 사유를 밝혀야 한다")
        self._assert_calls([])

    def test_unreadable_directive_defers_registration(self):
        # 지침을 못 읽는 것과 구판인 것은 다른 사실이고, 둘 다 등록 근거는 아니다.
        self._status_stub(self.good_status)
        self.directive.unlink()
        ok, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertIs(ok, False, "지침 판독 불가인데 등록을 허용했다: %s" % why)
        self.assertIn("판독 불가", why, "판독 불가를 '구판' 으로 접으면 안 된다")

    def test_marker_rejects_non_string(self):
        # 판독 실패(None)를 빈 문자열로 흡수하면 예외 대신 조용한 오답이 된다.
        for bad in (None, 3, b"bytes", ["x"]):
            self.assertIs(pf.capgate_marker_ok(bad), False,
                          "비-문자열 %r 은 신판일 수 없다" % (bad,))

    def test_c82_skips_without_cys_binary(self):
        # cys 가 없으면 코퍼스 실측 버전 자체를 조회할 수 없다 — SKIP 이지 PASS 가 아니다.
        result = self._c82()
        self.assertEqual(result["status"], pf.SKIP, "cys 부재는 C82 SKIP 기대")
        self._assert_calls([])

    def test_c82_skips_when_claude_version_unavailable(self):
        # 코퍼스는 읽었으나 비교 대상이 없다 — 드리프트 '없음' 이 아니라 판정 불능이다.
        self._stub("cys", {"gate-corpus --json": (0, json.dumps(
            {"measured_on": "2.1.241", "gates": [{"id": "folder-trust"}]}))})
        result = self._c82()
        self.assertEqual(result["status"], pf.SKIP, "claude 부재는 C82 SKIP 기대")
        self.assertIn("measured_on=2.1.241", result["detail"], "읽어낸 사실은 남겨야 한다")
        self.assertIn("측정 ", result["detail"], "측정 시각 병기는 SKIP 갈래에도 적용된다")
        self._assert_calls(["cys gate-corpus --json"])

    def test_c82_warns_on_non_json_response(self):
        # rc=0 인데 JSON 이 아니면 스키마 스큐다 — 조용히 넘기면 드리프트를 영영 못 본다.
        self._stub("cys", {"gate-corpus --json": (0, "not json at all")})
        result = self._c82()
        self.assertEqual(result["status"], pf.WARN, "비-JSON 응답은 WARN 기대")
        self._assert_calls(["cys gate-corpus --json"])

    def test_hook_self_test_via_sh_exits_zero(self):
        # 등록되는 command 문자열은 `sh <path>` 다 — bash 로만 통과하면 배포 형상과 다르다.
        hooks = self.pack / "hooks"
        hooks.mkdir()
        for name in ("role-capability-gate.sh", "_lib.sh"):
            shutil.copyfile(PACK / "hooks" / name, hooks / name)
        self._stub("cys", {})
        (self.bin / "sh").symlink_to("/bin/sh")
        result = subprocess.run([str(self.bin / "sh"), str(hooks / "role-capability-gate.sh"),
                                 "--self-test"], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, "sh 실행 self-test exit 0 기대:\n%s\n%s"
                         % (result.stdout, result.stderr))
        self.assertIn("self-test OK:", result.stdout, "강등 종료가 아니라 내장 검체 완주가 필요하다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
