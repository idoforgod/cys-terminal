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
        # ★데몬 소켓 실물(빈 파일로 충분 — preflight 는 실재만 본다): 없으면 `_capgate_gate` 는
        #   데몬을 **깨우지 않고** 미등록을 낸다(H-SEED-2 경합의 원인 제거). 아래 대부분의
        #   케이스는 "데몬은 있는데 응답이 이러할 때" 를 재므로 소켓을 먼저 놓는다.
        self.sock = self.home / ".local" / "state" / "cys" / "cys.sock"
        self.sock.parent.mkdir(parents=True, exist_ok=True)
        self.sock.write_text("", encoding="utf-8")
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
        """★expected 는 **3값**이다(R2): "on"|"off"|"unknown".

        판정 불능(unknown)과 조건 거짓(off)은 다른 사실이다 — 전자는 해제 사유가 아니다.
        """
        self._status_stub(status, rc)
        self.directive.write_text(text, encoding="utf-8")
        state, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertEqual(state, expected, "등록 상태는 %r 기대, 실제 %r · 사유=%s"
                         % (expected, state, why))
        self.assertIsInstance(why, str, "판정 사유는 문자열이어야 한다")
        for condition in missing:
            self.assertIn(condition, why, "미충족 조건 %r을 사유에 밝혀야 한다" % condition)
        for condition in present:
            self.assertNotIn(condition, why, "충족 조건 %r을 결핍으로 보고하면 안 된다" % condition)
        if rc == 0:
            # 순수 판정기는 **둘 다 잰** 문맥의 2값 요약 — 상태와 어긋나면 안 된다.
            ok, _pw = pf.capgate_registration_verdict(status, text)
            self.assertIs(ok, expected == pf.CAPGATE_ON,
                          "순수 판정과 설치본 판정이 갈렸다(%s vs %s)" % (ok, expected))
        self._assert_calls(["cys status --json"])

    def _c82(self):
        preflight = pf.Preflight(fix=False, skips=[])
        preflight.c82_gate_corpus_drift()
        self.assertEqual(len(preflight.results), 1, "C82 결과는 정확히 한 건이어야 한다")
        result = preflight.results[0]
        self.assertEqual(result["id"], "C82.gate-corpus-drift", "C82 결과 식별자를 유지해야 한다")
        return result

    def _profile_with_capgate(self, extra_user_hook=True):
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        cmd = pf._cys_hook_cmd(pf.CAPGATE_HOOK[0])
        blocks = [{"hooks": [{"type": "command", "command": cmd, "timeout": 15}]}]
        if extra_user_hook:
            blocks.append({"hooks": [{"type": "command", "command": "echo user-own-hook"}]})
        sp.write_text(json.dumps({"hooks": {"PreToolUse": blocks,
                                            "Stop": [{"hooks": [{"type": "command",
                                                                 "command": "echo keep-me"}]}]}},
                                 indent=2), encoding="utf-8")
        return sp

    # ── ★R2: C28 을 **실제로 실행**해서 잰다(codex: 소스 문자열 검사는 집행의 증거가 아니다) ──
    def _run_c28(self, settings_path, fix=True, status=None, rc=0, directive=None,
                 sock=True, table=None):
        """C28 을 격리 상태에서 1회 실행하고 (결과 dict, settings 본문) 을 돌려준다."""
        hooks = self.pack / "hooks"
        hooks.mkdir(exist_ok=True)
        (hooks / pf.CAPGATE_HOOK[0]).write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        if directive is not None:
            self.directive.write_text(directive, encoding="utf-8")
        if table is not None:
            tdir = self.pack / "state"
            tdir.mkdir(exist_ok=True)
            (tdir / "hook-targets.json").write_text(json.dumps(table), encoding="utf-8")
        if status is None:
            self._stub("cys", {})
        else:
            self._status_stub(status, rc)
        if not sock and self.sock.exists():
            self.sock.unlink()
        p = pf.Preflight(fix=fix, skips=[])
        with mock.patch.object(pf, "resolve_registration_targets",
                               return_value=([str(settings_path)], None)):
            p.c28_self_correction()
        res = [r for r in p.results if r["id"] == "C28.self-correction"]
        self.assertEqual(len(res), 1, "C28 결과는 한 건이어야 한다: %s" % p.results)
        return res[0], json.loads(settings_path.read_text(encoding="utf-8"))

    @staticmethod
    def _capgate_cmds(doc):
        return [h.get("command", "") for b in doc.get("hooks", {}).get("PreToolUse", [])
                for h in b.get("hooks", [])
                if pf.CAPGATE_HOOK[0] in h.get("command", "")]


class CapgateRegistration(_CapgateEnv):
    """등록 조건 · C82 · 배선 계약."""

    def test_both_conditions_enable_registration(self):
        # AND 조건을 과도하게 닫아 정상 신판도 등록되지 않는 회귀를 잡는다.
        self.assertIs(pf.capgate_alert_route_enabled(self.good_status), True,
                      "JSON boolean true는 경보 라우팅 지원이어야 한다")
        self.assertIs(pf.capgate_marker_ok(self.new_directive), True,
                      "첫 20행 내 독립 표지는 신판이어야 한다")
        self._assert_registration(self.good_status, self.new_directive, pf.CAPGATE_ON)


    def test_only_daemon_support_missing(self):
        # 지침은 준비됐어도 데몬 미지원이면 경보 없는 능력 제한을 등록하면 안 된다.
        self._assert_registration({"alert_route": {"enabled": False}}, self.new_directive,
                                  pf.CAPGATE_OFF, ("데몬 alert_route 미지원",), ("신판 표지",))


    def test_only_directive_marker_missing(self):
        # 데몬만 배포된 상태를 준비 완료로 접는 OR 조건 회귀를 잡는다.
        self._assert_registration(self.good_status, "# CSO\n구판 지침\n", pf.CAPGATE_OFF,
                                  ("설치본 CSO_DIRECTIVE", "신판 표지"), ("데몬 alert_route",))


    def test_both_conditions_missing(self):
        # 둘 다 빠졌을 때 한 조건만 보고하면 부분 배포 원인을 놓친다.
        self._assert_registration({"alert_route": {"enabled": False}}, "# 구판\n", pf.CAPGATE_OFF,
                                  ("데몬 alert_route 미지원", "설치본 CSO_DIRECTIVE", "신판 표지"))


    def test_old_status_nonzero_defers_registration(self):
        # 성공 JSON이 stdout에 있어도 실패 종료를 지원 증거로 사용하면 안 된다.
        # ★R2: rc≠0 은 **판정 불능**이지 '미지원' 이 아니다 — 해제 사유가 되면 안 된다.
        self._assert_registration(self.good_status, self.new_directive, pf.CAPGATE_UNKNOWN,
                                  ("rc=2", "판정 불능"), rc=2)


    def test_old_status_without_alert_route_defers_registration(self):
        # 구 status 스키마의 결측을 기본 true로 보정하는 회귀를 잡는다.
        # ★R2: '구 데몬' 은 **상태 문서로 식별되는** 응답에 alert_route 가 없을 때다.
        self._assert_registration({"daemon": {"version": "0.14.30"}, "surfaces": []},
                                  self.new_directive, pf.CAPGATE_OFF,
                                  ("데몬 alert_route 미지원",))


    def test_unidentifiable_status_is_undecidable_not_unsupported(self):
        """★R2 blocking(codex): 빈 객체·다른 도구의 JSON 은 **미지원의 증거가 아니다**.

        이것을 '조건 거짓' 으로 읽으면 부분 응답 한 번이 살아 있는 게이트를 지운다.
        """
        self._assert_registration({}, self.new_directive, pf.CAPGATE_UNKNOWN,
                                  ("상태 문서로 식별되지 않는다",))


    def test_empty_directive_is_undecidable_not_old(self):
        """★R2 blocking(codex): 설치·병합이 제자리 갱신하는 **찰나의 0바이트**를 '구판' 으로
        읽으면 정상 게이트를 지운다. 읽었지만 내용이 없는 것은 결측이다."""
        self._assert_registration(self.good_status, "   \n", pf.CAPGATE_UNKNOWN,
                                  ("비어 있다",))


    def test_prose_quotation_is_not_a_revision_marker(self):
        # substring 검사로 바뀌면 표지를 설명하는 구판 산문까지 신판이 된다.
        text = "# CSO\n표지 %s 를 확인하라.\n" % pf.CSO_DIRECTIVE_REV_MARKER
        self.assertIs(pf.capgate_marker_ok(text), False, "산문에 인용된 표지는 신판이 아니다")
        self._assert_registration(self.good_status, text, pf.CAPGATE_OFF, ("신판 표지",))


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
        self._assert_registration(status, self.new_directive, pf.CAPGATE_OFF,
                                  ("데몬 alert_route 미지원",))


    def test_string_enabled_is_unsupported(self):
        # 비어 있지 않은 문자열의 truthiness를 지원 여부로 읽으면 안 된다.
        status = {"alert_route": {"enabled": "true"}}
        self.assertIs(pf.capgate_alert_route_enabled(status), False, "문자열 true는 boolean true가 아니다")
        self._assert_registration(status, self.new_directive, pf.CAPGATE_OFF,
                                  ("데몬 alert_route 미지원",))


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
        state, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertEqual(state, pf.CAPGATE_UNKNOWN, "cys 부재는 판정 불능이다: %s" % why)
        self.assertIn("cys 바이너리 미발견", why, "판정 불능 사유를 밝혀야 한다")
        self._assert_calls([])

    def test_unreadable_directive_defers_registration(self):
        # 지침을 못 읽는 것과 구판인 것은 다른 사실이고, 둘 다 등록 근거는 아니다.
        self._status_stub(self.good_status)
        self.directive.unlink()
        state, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertEqual(state, pf.CAPGATE_UNKNOWN, "지침 판독 불가는 판정 불능이다: %s" % why)
        self.assertIn("판독 불가", why, "판독 불가를 '구판' 으로 접으면 안 된다")

    def test_missing_socket_defers_without_waking_daemon(self):
        """★소켓이 없으면 **데몬을 깨우지 않고** 미등록이다(R1).

        종전에는 이 축이 실제 `cys` 를 띄웠고, 그 바이너리는 자기 HOME 아래에 팩·상태를
        부트스트랩한다 — HOME 이 임시 디렉터리인 문맥에서 그 부수효과가 정리와 경합해
        부트 헬스 검체(H-SEED-2)가 `Directory not empty` 로 크래시했다. preflight 는 관측이다.
        """
        self._status_stub(self.good_status)
        self.sock.unlink()
        state, why = pf.Preflight(fix=False, skips=[])._capgate_gate()
        self.assertEqual(state, pf.CAPGATE_UNKNOWN, "소켓 부재는 판정 불능이다: %s" % why)
        self.assertIn("표지 0건", why)
        self._assert_calls([])          # ★호출 0 — 데몬을 깨우지 않았다

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


class CapgateDeregistration(_CapgateEnv):
    """★R1: 조건이 **거짓이 되면** 등록을 되돌린다 — 목록에서 빼는 것과 지우는 것은 다르다.

    종전 결함(두 리뷰어 동시 지적): 조건이 거짓이면 `_reg_hooks` 에서 빼기만 해서, 이미
    settings.json 에 실린 훅은 계속 발화하는데 C28 은 '등록 보류' 라고 보고했다 —
    판정문이 사실과 반대였다(§8 검증 결과 재작성 금지 · 부분 배포 = 봉인표 ③).
    """

    def test_unregister_removes_only_our_hook(self):
        sp = self._profile_with_capgate()
        p = pf.Preflight(fix=True, skips=[])
        self.assertTrue(p._event_hook_registered(str(sp), "PreToolUse", pf.CAPGATE_HOOK[0]),
                        "선행 조건: 능력 게이트가 등록돼 있어야 한다")
        err = p._unregister_event_hook(str(sp), "PreToolUse", pf.CAPGATE_HOOK[0])
        self.assertIsNone(err, "해제 실패: %s" % err)
        self.assertFalse(p._event_hook_registered(str(sp), "PreToolUse", pf.CAPGATE_HOOK[0]),
                         "해제 후에도 등록으로 판정된다(훅이 계속 발화한다)")
        data = json.loads(sp.read_text(encoding="utf-8"))
        cmds = [h["command"] for b in data["hooks"]["PreToolUse"] for h in b["hooks"]]
        self.assertEqual(cmds, ["echo user-own-hook"], "사용자 자신의 훅까지 지웠다: %s" % cmds)
        self.assertEqual(len(data["hooks"]["Stop"]), 1, "다른 이벤트를 건드렸다")

    def test_unregister_drops_empty_block(self):
        sp = self._profile_with_capgate(extra_user_hook=False)
        p = pf.Preflight(fix=True, skips=[])
        self.assertIsNone(p._unregister_event_hook(str(sp), "PreToolUse", pf.CAPGATE_HOOK[0]))
        data = json.loads(sp.read_text(encoding="utf-8"))
        self.assertEqual(data["hooks"]["PreToolUse"], [],
                         "빈 훅 블록이 남았다(하네스가 읽는 잡음): %s" % data["hooks"]["PreToolUse"])

    def test_unregister_is_noop_when_absent(self):
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        sp.write_text(json.dumps({"hooks": {"PreToolUse": [
            {"hooks": [{"type": "command", "command": "echo other"}]}]}}), encoding="utf-8")
        p = pf.Preflight(fix=True, skips=[])
        self.assertIsNone(p._unregister_event_hook(str(sp), "PreToolUse", pf.CAPGATE_HOOK[0]))
        data = json.loads(sp.read_text(encoding="utf-8"))
        self.assertEqual([h["command"] for b in data["hooks"]["PreToolUse"] for h in b["hooks"]],
                         ["echo other"], "없는 훅을 지우려다 남의 것을 건드렸다")

    def test_undecidable_keeps_live_registration(self):
        """★R2 blocking: 판정 불능(소켓 미실재)이 **살아 있는 게이트를 해제하면 안 된다**.

        리뷰어 PoC: 콜드 부트(데몬 미기동)에서 `preflight --fix` 가 PreToolUse 배열을 `[]` 로
        만들었고, 그 뒤 `cys boot` 가 CSO·reviewer 좌석을 무게이트로 띄웠다(감사 에러 1·3 재현).
        """
        sp = self._profile_with_capgate()
        res, doc = self._run_c28(sp, fix=True, status=None, sock=False)
        self.assertEqual(len(self._capgate_cmds(doc)), 1,
                         "판정 불능인데 등록이 해제됐다(콜드 부트마다 게이트가 꺼진다): %s" % doc)
        self.assertIn("판정 불능", res["detail"], "문면이 사실과 달라졌다: %s" % res["detail"])
        self.assertIn("유지", res["detail"], "유지했다는 사실을 적어야 한다: %s" % res["detail"])

    def test_undecidable_status_rc_keeps_live_registration(self):
        """★R2 blocking PoC ⓐ: 소켓은 실재하는데 `cys status` rc≠0 → 해제 금지."""
        sp = self._profile_with_capgate()
        res, doc = self._run_c28(sp, fix=True, status=self.good_status, rc=1)
        self.assertEqual(len(self._capgate_cmds(doc)), 1,
                         "rc≠0(판정 불능)인데 등록이 해제됐다: %s" % doc)
        self.assertIn("판정 불능", res["detail"])

    def test_positively_false_condition_unregisters(self):
        """조건이 **양성으로 거짓**(상태 문서에 alert_route 없음)이면 해제한다."""
        sp = self._profile_with_capgate()
        res, doc = self._run_c28(sp, fix=True,
                                 status={"daemon": {"version": "0.14.30"}, "surfaces": []})
        self.assertEqual(self._capgate_cmds(doc), [],
                         "조건 거짓인데 등록이 남았다: %s" % doc)
        self.assertEqual([h["command"] for b in doc["hooks"]["PreToolUse"] for h in b["hooks"]],
                         ["echo user-own-hook"], "사용자 훅까지 지웠다: %s" % doc)
        self.assertIn("해제", res["detail"], "해제 사실을 문면에 적어야 한다: %s" % res["detail"])

    def test_differently_quoted_registration_is_seen_and_removed(self):
        """★R2 blocking(codex): 따옴표 표기가 다른 **정상 등록**을 '미등록'으로 오보고하지 않는다."""
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        quoted = 'sh "%s"' % os.path.join(pf.pack_dir(), "hooks", pf.CAPGATE_HOOK[0])
        sp.write_text(json.dumps({"hooks": {"PreToolUse": [
            {"hooks": [{"type": "command", "command": quoted, "timeout": 15}]}]}}),
            encoding="utf-8")
        self.assertFalse(pf.Preflight._event_hook_registered(str(sp), "PreToolUse",
                                                             pf.CAPGATE_HOOK[0]),
                         "선행 조건: 바이트 동등 술어는 이 표기를 못 본다")
        self.assertTrue(pf.Preflight._event_hook_present_any(str(sp), "PreToolUse",
                                                             pf.CAPGATE_HOOK[0]),
                        "소유 술어가 표기 차이를 흡수하지 못한다")
        res, doc = self._run_c28(sp, fix=True,
                                 status={"daemon": {}, "surfaces": []})
        self.assertEqual(self._capgate_cmds(doc), [],
                         "표기가 다른 등록이 해제되지 않았다: %s" % doc)
        self.assertIn("능력 게이트 조건 거짓 — 해제", res["detail"],
                      "살아 있던 등록을 보지 못하고 '보류'로 보고했다: %s" % res["detail"])

    def test_conditions_met_registers(self):
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        sp.write_text("{}", encoding="utf-8")
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive)
        self.assertEqual(len(self._capgate_cmds(doc)), 1,
                         "조건 충족인데 등록되지 않았다: %s · %s" % (doc, res["detail"]))

    def test_table_deny_profile_is_not_registered(self):
        """★R2 blocking: 대상표가 `capgate: deny` 라고 선언한 프로필은 **부팅 경로도** 제외한다."""
        prof = self.home / ".claude-2"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        sp.write_text("{}", encoding="utf-8")
        table = {"schema_version": 1, "policy": {"unknown_profile": "deny"},
                 "profiles": [{"basename": ".claude-2",
                               "eligibility": {"guard_stop": "deny", "brief_warn": "deny",
                                               "capgate": "deny"}}]}
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive, table=table)
        self.assertEqual(self._capgate_cmds(doc), [],
                         "표가 deny 한 프로필에 등록했다: %s" % doc)

    def test_table_deny_profile_is_unregistered_even_when_conditions_hold(self):
        sp = self._profile_with_capgate()
        # `.claude` 를 deny 로 선언한 표
        table = {"schema_version": 1, "policy": {"unknown_profile": "deny"},
                 "profiles": [{"basename": ".claude",
                               "eligibility": {"guard_stop": "deny", "brief_warn": "allow",
                                               "capgate": "deny"}}]}
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive, table=table)
        self.assertEqual(self._capgate_cmds(doc), [],
                         "표 deny 프로필의 잔존 등록이 해제되지 않았다: %s" % doc)

    def test_corrupt_table_defers_registration(self):
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        sp.write_text("{}", encoding="utf-8")
        tdir = self.pack / "state"
        tdir.mkdir(exist_ok=True)
        (tdir / "hook-targets.json").write_text("{", encoding="utf-8")
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive)
        self.assertEqual(self._capgate_cmds(doc), [],
                         "손상 표에서 등록했다(하드코딩 폴백 금지): %s" % doc)
        self.assertIn("능력 게이트 판정 불능", res["detail"],
                      "손상 표를 조용히 무시했다: %s" % res["detail"])
        self.assertIsNotNone(pf.capgate_table_denied_basenames(str(self.pack))[1],
                             "손상 표가 err 없이 통과했다")


class CapgateRegistrarEligibility(_CapgateEnv):
    """★R1: 두 번째 등록 경로(javis_guard_register)도 **같은** 자격 판정을 통과해야 한다."""

    def _run(self, **kw):
        import io as _io
        buf = _io.StringIO()
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        (prof / "settings.json").write_text("{}", encoding="utf-8")
        rc, rows, _cmd, _spec = gr.process([str(prof)], "capgate", True, False,
                                           str(self.pack), out=buf, **kw)
        return rc, rows, buf.getvalue(), (prof / "settings.json").read_text(encoding="utf-8")

    def test_ineligible_registration_is_refused_with_zero_writes(self):
        # `cys` 부재(구 데몬) — 자격 판정 불능은 미자격이고, 그때 쓰기는 0이어야 한다.
        rc, rows, out, settings = self._run()
        self.assertEqual(rc, gr.EXIT_TARGET, "자격 미충족인데 rc=%s: %s" % (rc, out))
        self.assertEqual(rows, [], "자격 미충족인데 프로필 행이 생겼다")
        self.assertIn("등록 중단(쓰기 0)", out)
        self.assertEqual(json.loads(settings), {}, "쓰기 0 계약 위반: %s" % settings)

    def test_force_flag_is_the_only_bypass(self):
        rc, _rows, out, _s = self._run(force_ineligible=True)
        self.assertNotIn("등록 중단(쓰기 0)", out, "--force-ineligible 이 막혔다")
        self.assertIn("--force-ineligible", out, "우회 사실을 문면에 남겨야 한다")

    def test_eligible_registration_proceeds(self):
        rc, rows, out, _s = self._run(eligibility=lambda _p: (True, "검체: 조건 충족"))
        self.assertIn("등록 조건(CONTRACTS §C): 충족", out)
        self.assertNotIn("등록 중단(쓰기 0)", out)
        self.assertTrue(rows, "자격 충족인데 등록 행이 없다: %s" % out)

    def test_eligibility_requires_both_conditions(self):
        # 지침 표지만 참이면 자격이 아니다(데몬 조건이 사라지는 회귀).
        ok, why = gr._capgate_eligibility(str(self.pack))
        self.assertIs(ok, False, "cys 부재인데 자격이 났다: %s" % why)
        self.assertIn("cys", why)
        self.assertEqual(gr.CSO_DIRECTIVE_REV_MARKER, pf.CSO_DIRECTIVE_REV_MARKER,
                         "두 등록 경로의 표지 문자열이 갈라졌다(§C 계약은 하나다)")



# ─────────────────────────────────────────────────────────────────────────────
# ★독립 재유도(triage 2026-09-08 · P2-WP3A-capgate) — 등록기 잔여 5갈래.
#   전부 **실행**해서 잰다(소스 문자열 단언 0 — C28 을 돌리거나 판정 함수를 직접 부른다).
# ─────────────────────────────────────────────────────────────────────────────
class TriageRegistrationGaps(_CapgateEnv):
    """부팅 등록기의 잔여 결함(triage CONFIRMED 5종)."""

    def _env_recording_cys(self, status, varname="CYS_NO_AUTOSTART"):
        """`cys` 스텁 — 호출 시의 env 한 개를 `<log>.env` 에 남긴다."""
        envlog = self.root / "cys-env.log"
        path = self.bin / "cys"
        body = "\n".join([
            "#!/bin/sh",
            'printf "%s=%s\\n" ' + shlex.quote(varname)
            + ' "${' + varname + '-<unset>}" >> ' + shlex.quote(str(envlog)),
            'printf "%s\\n" ' + shlex.quote(json.dumps(status)),
        ]) + "\n"
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)
        return envlog

    # ── ① 역사적 표지만으로 데몬을 **깨우며** 조회한다 ─────────────────────────
    def test_alert_probe_seals_daemon_autostart(self):
        """★triage(codex major): 이 축은 `cys status --json` 를 **autostart 봉인 없이** 부른다.

        `_capgate_daemon_present()` 는 죽은 데몬이 남긴 `cysd.log`·`topology.json` 만으로도
        참이 되고(표지는 종료해도 남는다), 그 뒤의 `cys status` 는 `connect()` 실패 경로에서
        **형제 cysd 를 detached 로 기동**한다(src/bin/cys.rs:2258 — 옵트아웃은
        `CYS_NO_AUTOSTART=1` 뿐이다). 6초 타임아웃은 이미 태어난 데몬을 되돌리지 못한다.
        preflight 는 부트 체인의 첫 단계이고 이 축은 자기 주석에 '데몬을 깨우지 않는다'
        고 적고 있다(H-SEED-2 가 잡은 부수효과와 같은 층).
        """
        # 살아 있는 소켓을 지우고 **역사적 표지**만 남긴다 — 그래도 present 가 참이다.
        self.sock.unlink()
        (self.sock.parent / "cysd.log").write_text("stopped\n", encoding="utf-8")
        present, why = pf.Preflight(fix=False, skips=[])._capgate_daemon_present()
        self.assertTrue(present, "선행 사실: 역사적 표지만으로 present 가 참이다(%s)" % why)
        envlog = self._env_recording_cys(self.good_status)
        axis, _why = pf.Preflight(fix=False, skips=[])._capgate_alert_axis()
        self.assertIs(axis, True, "선행 사실: 이 축이 실제로 `cys status --json` 을 불렀다")
        self.assertEqual(envlog.read_text(encoding="utf-8").strip(),
                         "CYS_NO_AUTOSTART=1",
                         "등록 프로브가 데몬 autostart 를 봉인하지 않았다 — 부트 첫 단계의 "
                         "읽기 전용 관측이 데몬을 낳는다")

    def test_manual_registrar_probe_seals_daemon_autostart(self):
        """수동 등록기(`javis_guard_register._capgate_eligibility`)도 같은 봉인이 필요하다."""
        envlog = self._env_recording_cys(self.good_status)
        ok, why = gr._capgate_eligibility(str(self.pack))
        self.assertTrue(ok, "선행 사실: 자격 판정이 실제로 데몬에 물었다(%s)" % why)
        self.assertEqual(envlog.read_text(encoding="utf-8").strip(),
                         "CYS_NO_AUTOSTART=1",
                         "수동 등록기의 자격 프로브가 데몬 autostart 를 봉인하지 않았다")

    # ── ② matcher 가 달린 기존 등록이 '등록됨' 으로 인정된다 ────────────────────
    def test_matcher_scoped_registration_is_repaired(self):
        """★triage(codex major): `matcher: "Bash"` 가 달린 등록을 그대로 두면 게이트가
        **Bash 에만** 붙는다 — CronCreate·Agent·Edit/Write 는 무게이트가 되고 예산 계수도
        Bash 호출만 센다. `_event_hook_registered` 는 matcher 축을 아예 보지 않아서
        C28 이 '이미 등록됨' 으로 건너뛴다(조용한 게이트 면제).
        """
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        cmd = pf._cys_hook_cmd(pf.CAPGATE_HOOK[0])
        sp.write_text(json.dumps({"hooks": {"PreToolUse": [
            {"matcher": "Bash",
             "hooks": [{"type": "command", "command": cmd, "timeout": 15}]}]}}),
            encoding="utf-8")
        # 선행 사실: 계약상 capgate 는 matcher 가 없어야 한다(전 도구).
        self.assertEqual([m for _ev, m in pf.CAPGATE_HOOK[1]], [None],
                         "계약 확인: capgate 선언에는 matcher 가 없다")
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive)
        blocks = [b for b in doc["hooks"]["PreToolUse"]
                  if any(pf.CAPGATE_HOOK[0] in h.get("command", "")
                         for h in b.get("hooks", []))]
        self.assertTrue(blocks, "등록이 사라졌다: %s · %s" % (doc, res["detail"]))
        self.assertTrue(any(not b.get("matcher") for b in blocks),
                        "matcher 로 좁혀진 등록이 교정되지 않았다 — 게이트가 Bash 에만 붙는다: %s"
                        % doc)

    # ── ③ 부팅 등록기와 수동 등록기가 손상 표를 다르게 읽는다 ──────────────────
    def test_schema_mismatch_table_defers_like_the_manual_registrar(self):
        """★triage(codex major): 수동 등록기가 **손상**이라 거부하는 표를 부팅 등록기는
        조용히 통과시킨다(`capgate_table_denied_basenames` 는 schema_version 도 값 어휘도
        보지 않는다). 운영자가 선언한 제외가 부팅 경로에서만 사라진다.
        """
        tdir = self.pack / "state"
        tdir.mkdir(exist_ok=True)
        table = {"schema_version": 99, "policy": {"unknown_profile": "deny"},
                 "profiles": [{"basename": ".claude",
                               "eligibility": {"guard_stop": "allow", "brief_warn": "allow",
                                               "capgate": "DENY"}}]}
        tpath = tdir / "hook-targets.json"
        tpath.write_text(json.dumps(table), encoding="utf-8")
        _tbl, err = gr._load_targets(str(tpath))
        self.assertIsNotNone(err, "선행 사실: 수동 등록기는 이 표를 손상으로 거부한다")
        prof = self.home / ".claude"
        prof.mkdir(parents=True, exist_ok=True)
        sp = prof / "settings.json"
        sp.write_text("{}", encoding="utf-8")
        res, doc = self._run_c28(sp, fix=True, status=self.good_status,
                                 directive=self.new_directive)
        self.assertEqual(self._capgate_cmds(doc), [],
                         "손상 표(수동 등록기는 거부)인데 부팅 경로가 등록했다: %s · %s"
                         % (doc, res["detail"]))

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0,
                     "root 는 퍼미션으로 판독 불가를 만들 수 없다")
    def test_unreadable_installed_table_is_not_absence(self):
        """★triage(codex major): 설치된 표를 **읽지 못한 것**과 표가 **없는 것**이 같은 값으로
        접힌다 — 판독 실패가 조용히 예시표/기본값 폴백이 되어 운영자의 제외가 사라진다.
        """
        tdir = self.pack / "state"
        tdir.mkdir(exist_ok=True)
        tpath = tdir / "hook-targets.json"
        tpath.write_text(json.dumps({"schema_version": 1,
                                     "policy": {"unknown_profile": "deny"},
                                     "profiles": [{"basename": ".claude",
                                                   "eligibility": {"guard_stop": "allow",
                                                                   "brief_warn": "allow",
                                                                   "capgate": "deny"}}]}),
                         encoding="utf-8")
        os.chmod(str(tpath), 0)
        self.addCleanup(os.chmod, str(tpath), 0o600)
        self.assertIsNone(pf._read_text_tolerant(str(tpath)),
                          "선행 사실: 이 표는 판독 불가다")
        deny, err = pf.capgate_table_denied_basenames(str(self.pack))
        self.assertIsNotNone(err,
                             "판독 실패가 '표 부재'(폴백)로 접혔다 — 결측은 값이 아니다: %r"
                             % (deny,))

    # ── ④ 명시 명명 파이프 주소가 표지 폴백을 건너뛴다 ─────────────────────────
    def test_named_pipe_socket_falls_back_to_hub_markers(self):
        r"""★triage(codex major · Windows): `CYS_SOCKET=\\.\pipe\cys` 면 두 등록기 모두
        파일시스템 `exists()` 하나로 판정을 끝내고 허브 표지를 **보지 않는다**.

        명명 파이프는 인스턴스가 사용 중이거나 메타데이터 조회가 실패해도 stat 이 실패한다 —
        그때 살아 있는 데몬을 '미실재' 로 읽으면 preflight 는 영구 `unknown`, 수동 등록은 거부다
        (R2 B8 이 넣었다고 적은 '플랫폼 공통 표지 폴백' 이 이 환경에는 닿지 않는다).
        """
        pipe = "\\\\.\\pipe\\cys"
        self.assertFalse(os.path.exists(pipe), "선행 사실: 이 경로는 파일로 실재하지 않는다")
        self.assertTrue(self.sock.exists(), "선행 사실: 허브 표지는 살아 있다")
        with mock.patch.dict(os.environ, {"CYS_SOCKET": pipe}):
            present, why = pf.Preflight(fix=False, skips=[])._capgate_daemon_present()
            self.assertTrue(present,
                            "명명 파이프 주소에서 허브 표지 폴백이 동작하지 않았다: %s" % why)
            gpresent, gwhy = gr._daemon_present()
            self.assertTrue(gpresent,
                            "수동 등록기도 같은 갈래다: %s" % gwhy)

    # ── ⑤ 판정 불능이 다음 부팅으로 이어지지 않는다 ────────────────────────────
    def test_undecidable_capgate_is_persisted_for_recheck(self):
        """★triage(codex major): `unknown` 은 등록도 해제도 하지 않는데, 그 사실이 **어디에도
        남지 않는다**.

        preflight 를 자동으로 돌리는 유일한 지점은 `javis_bootstrap.py:2701` 이고 그 앞의
        레인 마커 fast path(`_marker_fresh`)는 **같은 pack_version 이면 preflight 를 통째로
        생략**한다. 즉 그 팩 버전의 첫 부팅이 판정 불능이면 게이트는 그 버전 내내 미등록으로
        남고, 두 번째 부팅부터는 경고조차 나오지 않는다.
        복구 재측정을 하려면 '미해소' 사실이 **부팅 체인이 읽을 수 있는 곳에 남아야** 한다.
        (이 검체는 지속화 축만 핀한다 — 소비 축(fast path 우회)은 부트 하네스가 따로 핀해야 한다.)
        """
        sp = self._profile_with_capgate()
        res, doc = self._run_c28(sp, fix=True, status=None, sock=False)
        self.assertIn("판정 불능", res["detail"], "선행 사실: 이 실행은 판정 불능이다")
        self.assertEqual(len(self._capgate_cmds(doc)), 1,
                         "선행 사실: 판정 불능은 등록을 건드리지 않는다")
        roots = [self.javis, self.pack / "state", self.sock.parent]
        found = []
        for root in roots:
            for dirpath, _dirs, files in os.walk(str(root)):
                for name in files:
                    fp = os.path.join(dirpath, name)
                    try:
                        body = open(fp, "rb").read()
                    except OSError:
                        continue
                    if b"capgate" in body or "capgate" in name:
                        found.append(fp)
        self.assertTrue(found,
                        "판정 불능이 지속 기록으로 남지 않았다 — 다음 부팅은 fast path 로 "
                        "preflight 자체를 건너뛰므로 재측정 기회가 없다(탐색 대상: %s)"
                        % [str(r) for r in roots])



if __name__ == "__main__":
    unittest.main(verbosity=2)
