#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_capgate_hook_shell.py — 능력 게이트 훅의 **셸 배선**을 종단으로 재는 회귀 검체(0.14.31 R1).

왜 이 파일이 필요한가(리뷰 R1 · 두 리뷰어가 같은 곳을 짚었다):
  `role-capability-gate.sh --self-test` 는 **파이썬 판정기만** 돈다 — 임시파일 입력·역할 해소·
  캐시·boot-epoch 키·python 부재 비대칭·deny JSON 방출은 그 분기를 통째로 **건너뛴다**.
  즉 종전에는 신설된 셸 층(약 130행)에 자동 검체가 0이었고, 캐시로 게이트가 열리는 결함을
  잡을 수 있는 검체가 하나도 없었다. 여기서는 훅을 **실제로 실행**해서 잰다.

무엇을 막는가:
  ① 캐시 오염으로 게이트가 열리는 회귀(CYS_ROLE=cso 인데 `master` 캐시가 통과시키던 것).
  ② 게이트 대상 캐시를 fast-path 로 써서 **역할 승계**(reviewer→CSO)에 옛 정책이 남는 회귀.
  ③ 비대상 좌석이 매 도구 호출마다 데몬 RPC 를 내는 회귀(전 pane 지연 · 봉인표 ④ 방향).
  ④ 권위 있는 '역할 없음'을 캐시하지 않아 무역할 pane 이 매 호출 RPC 를 내는 회귀.
  ⑤ python 부재 비대칭(reviewer exit 2 / CSO 강등 exit 0)의 회귀.
  ⑥ 큰 Write 본문이 판정에 **도달하지 못하는** 회귀(임시 파일 경로) · 임시 파일 잔재.
  ⑦ 예산 카운터가 동시 호출에서 유실되는 회귀(원자 증가).
  ⑧ 역할 미확정(캐시·env 불일치)에서 한쪽 정책만 적용되는 회귀(교집합 판정).
  ⑨ **안전 검사를 지워도 self-test 가 통과하는 사각지대**(음성 대조 — 이 파일의 마지막 클래스).

라이브 무접촉: HOME·TMPDIR·CYS_STATE_DIR·PATH 전부 임시 디렉터리이고 `cys` 는 스텁이다.
데몬·소켓에 접속하지 않는다(스텁이 stdout 한 줄을 낼 뿐이다).

실행: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" \
      python3 cysjavis-pack/bin/tests/test_capgate_hook_shell.py
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

BIN = Path(__file__).resolve().parent.parent
PACK = BIN.parent
HOOK = PACK / "hooks" / "role-capability-gate.sh"
LIB = PACK / "hooks" / "_lib.sh"
sys.dont_write_bytecode = True

SH = shutil.which("sh") or shutil.which("bash")
# 셸이 없으면(이론상 Windows 비-GitBash) 이 파일은 통째로 건너뛴다 — 없는 셸을 부르는 것보다
# '재지 못했다'를 드러내는 것이 낫다(판정 불능은 통과가 아니다).
NEED_SH = unittest.skipIf(not SH, "sh 부재 — 셸 종단 검체 실행 불가")


def _slug(v):
    """훅의 `capgate_slug`(`tr -c 'A-Za-z0-9._-' '_'`) 미러."""
    return re.sub(r"[^A-Za-z0-9._-]", "_", v or "")


class HookRun(object):
    """훅 1회 실행 결과."""

    def __init__(self, rc, out, err):
        self.rc, self.out, self.err = rc, out, err

    @property
    def denied(self):
        try:
            doc = json.loads(self.out or "{}")
        except ValueError:
            return False
        hso = doc.get("hookSpecificOutput") or {}
        return hso.get("permissionDecision") == "deny"

    @property
    def reason(self):
        try:
            return (json.loads(self.out or "{}").get("hookSpecificOutput") or {}).get(
                "permissionDecisionReason", "")
        except ValueError:
            return ""


@NEED_SH
class _HookEnv(unittest.TestCase):
    """격리 환경 + 실행 헬퍼(테스트 없음)."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="capgate-hook-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.home = self.root / "home"
        self.tmpdir = self.root / "tmp"
        self.state = self.home / ".cys" / "state"
        self.pack = self.root / "pack"
        self.fakebin = self.root / "fakebin"
        for d in (self.home, self.tmpdir, self.state, self.pack / "round",
                  self.pack / "bin", self.fakebin):
            d.mkdir(parents=True, exist_ok=True)
        self.cyslog = self.root / "cys-calls.log"
        self.env = dict(os.environ)
        self.env.update({
            "HOME": str(self.home),
            "TMPDIR": str(self.tmpdir),
            "CYS_STATE_DIR": str(self.state),
            "CYS_PACK_DIR": str(self.pack),
            "CYS_PY": sys.executable,
            "CYS_FAKE_LOG": str(self.cyslog),
            "PATH": os.pathsep.join([str(self.fakebin)] + self._clean_path()),
        })
        for k in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SURFACE_ID", "CYS_SOCKET",
                  "CYS_BIN", "CAPGATE_ROLE_ALT", "CAPGATE_INPUT", "CAPGATE_INPUT_FILE"):
            self.env.pop(k, None)

    # ── 스텁 ────────────────────────────────────────────────────────────────
    @staticmethod
    def _clean_path():
        """실 `cys` 가 들어 있는 PATH 항목을 뺀다 — 검체가 **라이브 데몬**에 묻지 않게 한다
        (읽기 전용 RPC 라도 검체는 자기 세계 안에서 결정론이어야 한다)."""
        out = []
        for d in (os.environ.get("PATH", "") or "").split(os.pathsep):
            if not d:
                continue
            if any(os.path.exists(os.path.join(d, n)) for n in ("cys", "cys.exe")):
                continue
            out.append(d)
        return out

    def fake_cys(self, role="", rc=0):
        """PATH 위의 `cys` 스텁 — 호출을 기록하고 지정한 역할 1줄을 낸다."""
        p = self.fakebin / "cys"
        body = ["#!/bin/sh", 'printf "%s\\n" "$*" >> "$CYS_FAKE_LOG"']
        if role:
            body.append('printf "%s\\n"' % role)
        body.append("exit %d" % rc)
        p.write_text("\n".join(body) + "\n", encoding="utf-8")
        p.chmod(0o755)
        return p

    def cache_path(self, surface="7", socket=""):
        return self.tmpdir / ("cys-capgate-role-%s-%s-%s"
                              % (_slug(surface), _slug(socket or "none"), ""))

    def plant_cache(self, role, surface="7", age=0, socket=""):
        import time
        p = self.cache_path(surface, socket)
        p.write_text("%d %s\n" % (int(time.time()) - age, role), encoding="utf-8")
        p.chmod(0o600)
        return p

    def calls_log(self):
        return self.cyslog.read_text(encoding="utf-8") if self.cyslog.exists() else ""

    # ── 실행 ────────────────────────────────────────────────────────────────
    def run_hook(self, tool="Bash", tool_input=None, session_id="s-1", **envkw):
        env = dict(self.env)
        env.update({k: str(v) for k, v in envkw.items()})
        payload = json.dumps({"session_id": session_id, "tool_name": tool,
                              "tool_input": tool_input or {}})
        r = subprocess.run([SH, str(HOOK)], input=payload, env=env,
                           capture_output=True, text=True, timeout=90)
        return HookRun(r.returncode, r.stdout, r.stderr)


class HookRoleResolution(_HookEnv):
    """역할 해소·캐시 — self-test 가 건드리지 못하는 층."""

    def test_env_role_denies_without_daemon(self):
        """`cys` 부재 + CYS_ROLE=cso → env 폴백으로 게이트가 산다(deny JSON · exit 0)."""
        r = self.run_hook("CronCreate", {}, CYS_ROLE="cso")
        self.assertEqual(r.rc, 0, r.err)
        self.assertTrue(r.denied, "CronCreate 가 통과했다: %r / %r" % (r.out, r.err))
        self.assertIn("CSO 능력 게이트", r.reason)

    def test_poisoned_cache_cannot_open_gate(self):
        """★캐시에 `master` 를 심어도 CYS_ROLE=cso 좌석의 게이트는 열리지 않는다(R1 major).

        종전: ③폴백이 신선한 캐시를 무검사로 채택하고 그 값이 CYS_ROLE 보다 앞서서,
        같은 uid 의 아무 프로세스나 60초 동안 게이트를 열 수 있었다.
        """
        self.plant_cache("master")
        r = self.run_hook("CronCreate", {}, CYS_ROLE="cso", CYS_SURFACE_ID="7")
        self.assertTrue(r.denied, "오염된 캐시가 게이트를 열었다: %r / %r" % (r.out, r.err))

    def test_nongated_cache_is_fastpath_no_rpc(self):
        """비대상(master) 캐시는 fast-path — 데몬 RPC 0(전 pane 지연 방지)."""
        self.fake_cys(role="master")
        self.plant_cache("master")
        r = self.run_hook("Bash", {"command": "rm -rf /x"}, CYS_SURFACE_ID="7")
        self.assertEqual(r.rc, 0)
        self.assertFalse(r.denied, r.out)
        self.assertEqual(self.calls_log(), "",
                         "비대상 좌석이 매 호출 데몬 RPC 를 냈다: %r" % self.calls_log())

    def test_gated_cache_is_never_fastpath(self):
        """★게이트 대상 캐시는 fast-path 가 아니다 — 승계(reviewer→CSO)를 데몬이 정정한다."""
        self.fake_cys(role="master")          # 데몬 권위: 지금 이 좌석은 master 다
        self.plant_cache("reviewer-codex")    # 60초 안의 옛 역할
        r = self.run_hook("Edit", {"file_path": "/nonexistent-repo/a.rs"},
                          CYS_SURFACE_ID="7")
        self.assertIn("surface-role", self.calls_log(),
                      "게이트 대상 캐시를 권위로 써서 데몬에 묻지 않았다")
        self.assertFalse(r.denied, "정정된 master 좌석이 reviewer 정책으로 막혔다: %s" % r.reason)

    def test_authoritative_none_is_cached(self):
        """권위 있는 '역할 없음'도 캐시된다 — 무역할 pane 이 매 호출 RPC 를 내지 않게."""
        self.fake_cys(role="", rc=0)
        r1 = self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        self.assertEqual(r1.rc, 0)
        self.assertEqual(self.cache_path().read_text(encoding="utf-8").split()[1], "-")
        n1 = len(self.calls_log().splitlines())
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        self.assertEqual(len(self.calls_log().splitlines()), n1,
                         "무역할 캐시가 재사용되지 않았다(매 호출 RPC)")

    def test_stale_cache_is_requeried(self):
        """TTL 밖 캐시는 권위가 아니다(시간 창의 상한을 고정한다)."""
        self.fake_cys(role="master")
        self.plant_cache("master", age=999)
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        self.assertIn("surface-role", self.calls_log(), "만료 캐시를 그대로 썼다")

    def test_failed_query_backs_off(self):
        """★조회 실패는 **짧게 기억**한다 — 데몬 무응답이 도구 호출마다 데드라인을 무는
        전 좌석 폭풍(봉인표 ④ 방향)이 되지 않게."""
        self.fake_cys(role="", rc=3)          # 조회 실패
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        n1 = len(self.calls_log().splitlines())
        self.assertGreaterEqual(n1, 1, "첫 호출은 조회해야 한다")
        for _ in range(3):
            self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        self.assertEqual(len(self.calls_log().splitlines()), n1,
                         "실패 백오프 창 안에서 재조회했다(폭주): %s" % self.calls_log())
        # 백오프 표시를 지우면 다시 조회한다(영구 차단이 아니다).
        (self.cache_path().parent / (self.cache_path().name + ".fail")).unlink()
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        self.assertGreater(len(self.calls_log().splitlines()), n1,
                           "백오프 표시 제거 뒤에도 조회하지 않는다(영구 정지)")

    def test_ambiguous_candidates_intersect(self):
        """★조회 실패 + 캐시(reviewer)와 env(cso) 불일치 → **두 정책 모두** 적용한다."""
        self.fake_cys(role="", rc=3)          # 조회 실패
        self.plant_cache("reviewer-codex")
        cron = self.run_hook("CronCreate", {}, CYS_ROLE="cso", CYS_SURFACE_ID="7")
        self.assertTrue(cron.denied, "CSO 금지 도구가 reviewer 후보로 통과했다: %r" % cron.out)
        # ★임시 루트 **밖**의 경로여야 한다 — reviewer 정책은 tmp 쓰기를 허용하므로
        #   검체 경로가 TMPDIR 안이면 두 정책이 모두 허용해 교집합을 재지 못한다.
        edit = self.run_hook("Edit", {"file_path": "/nonexistent-repo/a.rs"},
                             CYS_ROLE="cso", CYS_SURFACE_ID="7")
        self.assertTrue(edit.denied, "reviewer 변형 금지가 사라졌다: %r" % edit.out)
        save = self.run_hook("Write", {"file_path": str(self.pack / "round" / "CSO_TODO.md"),
                                       "content": "저장"},
                             CYS_ROLE="cso", CYS_SURFACE_ID="7")
        self.assertFalse(save.denied,
                         "역할 미확정이 사이클 필수 저장을 막았다(봉인표 ②): %s" % save.reason)


class HookDegradation(_HookEnv):
    """python 부재·큰 입력·임시 파일 수명."""

    def _nopython_bin(self):
        """python 만 없는 PATH 디렉터리(coreutils 는 심링크로 남긴다)."""
        need = ["date", "head", "tr", "dirname", "mktemp", "cat", "rm", "mv", "id",
                "printf", "sed", "uname", "command"]
        d = self.root / "nopy"
        d.mkdir(exist_ok=True)
        for t in need:
            src = shutil.which(t)
            if src:
                dst = d / t
                if not dst.exists():
                    try:
                        os.symlink(src, dst)
                    except OSError:
                        shutil.copy2(src, dst)
        if not (d / "date").exists() or not (d / "mktemp").exists():
            self.skipTest("coreutils 심링크 불가 — python 부재 분기를 잴 수 없다")
        return d

    def test_python_missing_asymmetry(self):
        """★reviewer=fail-closed(exit 2) · CSO=강등(exit 0) — 비대칭이 코드에 살아 있는가."""
        d = self._nopython_bin()
        env = {"PATH": str(d), "CYS_PY": ""}
        rev = self.run_hook("Edit", {"file_path": "/x/a.rs"}, CYS_ROLE="reviewer-codex", **env)
        self.assertEqual(rev.rc, 2, "reviewer 가 fail-closed 가 아니다: rc=%d %r" % (rev.rc, rev.err))
        cso = self.run_hook("CronCreate", {}, CYS_ROLE="cso", **env)
        self.assertEqual(cso.rc, 0, "CSO 좌석이 벽돌이 됐다(봉인표 ②③): %r" % cso.err)
        self.assertIn("강등", cso.err)

    def test_large_write_reaches_cap_and_leaves_no_residue(self):
        """2MB Write 본문이 **판정에 도달**하고(상한 deny) 임시 파일이 남지 않는다."""
        todo = self.pack / "round" / "CSO_TODO.md"
        todo.write_text("x" * 10, encoding="utf-8")
        r = self.run_hook("Write", {"file_path": str(todo), "content": "가" * 700000},
                          CYS_ROLE="cso")
        self.assertTrue(r.denied, "큰 본문이 상한 판정에 도달하지 못했다: %r %r" % (r.out, r.err))
        self.assertIn("상한", r.reason)
        left = [p.name for p in self.tmpdir.iterdir() if p.name.startswith("cys-capgate-in.")]
        self.assertEqual(left, [], "훅 입력 임시 파일이 남았다: %s" % left)

    def test_allow_path_is_silent(self):
        """허용 경로는 stdout 무출력 + exit 0(defer) — 하네스 계약."""
        r = self.run_hook("Bash", {"command": "cys status --json"}, CYS_ROLE="cso")
        self.assertEqual(r.rc, 0)
        self.assertEqual(r.out.strip(), "", "허용인데 stdout 에 출력이 있다: %r" % r.out)


class HookBudgetCounter(_HookEnv):
    """예산 카운터 — 원자 증가(동시 호출 유실 금지)."""

    def _count_file(self, session_id):
        key = hashlib.sha256(session_id.encode("utf-8")).hexdigest()[:32]
        return self.state / "capgate" / (key + ".calls")

    def test_counter_counts_every_call(self):
        for _ in range(3):
            self.run_hook("Bash", {"command": "cys status"}, session_id="s-count",
                          CYS_ROLE="cso")
        self.assertEqual(self._count_file("s-count").stat().st_size, 3)

    def test_counter_is_atomic_under_parallel_calls(self):
        """★동시 12회 호출에서 **하나도 유실되지 않는다**(read-modify-write 회귀 차단)."""
        n = 12
        errs = []

        def one():
            try:
                self.run_hook("Bash", {"command": "cys status"}, session_id="s-par",
                              CYS_ROLE="cso")
            except Exception as e:      # noqa: BLE001 — 스레드 예외를 삼키지 않는다
                errs.append(e)

        ts = [threading.Thread(target=one) for _ in range(n)]
        for t in ts:
            t.start()
        for t in ts:
            t.join(120)
        self.assertEqual(errs, [])
        self.assertEqual(self._count_file("s-par").stat().st_size, n,
                         "동시 증가가 유실됐다(원자 증가 아님)")


class HookInputHandoff(_HookEnv):
    """★R2: 셸→파이썬 **입력 인계**의 실패 양식(Windows Git Bash 경로 변환).

    종전: 셸이 stdin 을 임시 파일에 쓰고 `cys_native_path` 로 변환해 넘겼는데, 파이썬이 그 파일을
    열지 못하면 입력이 빈 문자열로 강등되어 `json.loads("")` 가 터졌다 — reviewer/planner 는
    **모든 도구 호출마다** exit 2 로 벽돌이 됐다(cygpath 부재·변환 어긋남). 기준 커밋의 훅은
    env 만 읽어 이 의존이 없었으므로 **이번 변경이 새로 만든 실패 양식**이다.
    """

    def _break_cygpath(self):
        """PATH 에 **어긋난 변환**을 내는 cygpath 를 심는다(Git Bash 실패 형상의 재현).

        이 형상에서는 변환 경로가 열리지 않아도 **POSIX 원본**이 남아 있다 — 판정기는 두 경로를
        차례로 열어 보므로 판정이 계속되어야 한다.
        """
        p = self.fakebin / "cygpath"
        p.write_text("#!/bin/sh\nprintf '%s\\n' '/nonexistent/converted/path'\n",
                     encoding="utf-8")
        p.chmod(0o755)
        return p

    def _destroy_input(self):
        """변환도 어긋나고 **원본도 사라진** 최악 형상(파일 인계 전면 실패)."""
        p = self.fakebin / "cygpath"
        p.write_text("#!/bin/sh\nrm -f \"$2\" 2>/dev/null\n"
                     "printf '%s\\n' '/nonexistent/converted/path'\n", encoding="utf-8")
        p.chmod(0o755)
        return p

    def test_broken_native_path_does_not_brick_reviewer(self):
        """★인계 실패에서도 reviewer 는 **판정을 수행**한다 — 벽돌이 되지 않는다.

        종전: 변환 경로 open 실패 → 입력이 빈 문자열 → `json.loads("")` → **매 도구 호출마다**
        exit 2. 기준 커밋의 훅은 env 만 읽어 이 의존이 없었으므로 이번 변경이 만든 실패 양식이다.
        """
        self._break_cygpath()
        r = self.run_hook("Edit", {"file_path": "/nonexistent-repo/a.rs"},
                          CYS_SURFACE_ROLE="reviewer-codex")
        self.assertEqual(r.rc, 0, "인계 실패가 reviewer 좌석을 exit 2 로 죽였다: %s" % r.err)
        self.assertTrue(r.denied, "판정이 수행되지 않았다(집행 0): %r / %r" % (r.out, r.err))

    def test_broken_native_path_still_judges_cso(self):
        self._break_cygpath()
        r = self.run_hook("CronCreate", {}, CYS_SURFACE_ROLE="cso")
        self.assertEqual(r.rc, 0)
        self.assertTrue(r.denied, "인계 실패로 CSO 게이트가 조용히 꺼졌다: %r / %r"
                        % (r.out, r.err))

    def test_broken_native_path_allows_normal_call(self):
        self._break_cygpath()
        r = self.run_hook("Bash", {"command": "cys status --json"}, CYS_SURFACE_ROLE="cso")
        self.assertEqual(r.rc, 0)
        self.assertFalse(r.denied, "정상 호출이 인계 실패로 막혔다: %s" % r.reason)

    def test_destroyed_input_small_payload_uses_env_fallback(self):
        """원본까지 사라져도 **소용량은 env 로도 실려 있다** — 판정이 계속된다."""
        self._destroy_input()
        r = self.run_hook("CronCreate", {}, CYS_SURFACE_ROLE="cso")
        self.assertEqual(r.rc, 0)
        self.assertTrue(r.denied, "env 폴백이 동작하지 않아 게이트가 꺼졌다: %r / %r"
                        % (r.out, r.err))

    def test_destroyed_input_large_payload_fails_closed_for_reviewer(self):
        """★대용량(>64KB)은 env 로 못 싣는다 — 그때 통과시키면 **배관 실패가 권한 확대**가 된다.

        codex R2 반례: reviewer 의 70KB Write 가 무검사로 나가면 producer≠evaluator 의 기계
        집행이 사라진다. 그래서 이 갈래는 종전 계약(reviewer fail-closed)을 그대로 지킨다.
        """
        self._destroy_input()
        r = self.run_hook("Write", {"file_path": "/nonexistent-repo/a.rs",
                                    "content": "x" * 70000},
                          CYS_SURFACE_ROLE="reviewer-codex")
        self.assertEqual(r.rc, 2, "판독 불능인데 대형 Write 가 통과했다: rc=%s %r"
                         % (r.rc, r.out))

    def test_destroyed_input_large_payload_degrades_for_cso(self):
        """CSO 는 같은 상황에서 **강등**이다(전 도구 차단 = 좌석 사망 금지 · 비대칭 유지)."""
        self._destroy_input()
        r = self.run_hook("Write", {"file_path": "/nonexistent-repo/a.rs",
                                    "content": "x" * 70000},
                          CYS_SURFACE_ROLE="cso")
        self.assertEqual(r.rc, 0, "CSO 좌석이 판독 불능으로 죽었다: %s" % r.err)
        self.assertIn("게이트 강등", r.err)

    def test_no_temp_file_leftover_on_broken_conversion(self):
        """★`exec` 뒤에는 셸 trap 이 돌지 않는다 — 판정기가 **두 경로를 다** 지워야 한다."""
        self._break_cygpath()
        self.run_hook("Bash", {"command": "cys status"}, CYS_SURFACE_ROLE="cso")
        leftovers = [p.name for p in self.tmpdir.iterdir()
                     if p.name.startswith("cys-capgate-in.")]
        self.assertEqual(leftovers, [], "임시 입력 파일이 남았다: %s" % leftovers)


class NegativeControls(unittest.TestCase):
    """★음성 대조 — 안전 검사를 **지우면** 내장 self-test 가 실패해야 한다.

    "정상 예제 개수"는 우회 차단의 증거가 아니다(codex R1). 각 검사마다 코드를 한 줄 바꾼
    사본을 만들어 `--self-test` 를 돌리고, **여전히 통과하면 그 검사에는 검체가 없는 것**이다.
    """

    MUTATIONS = [
        ("shlex 주석 처리 끄기", '        lex.commenters = ""\n', "        pass\n"),
        ("python 옵션 정확 토큰",
         "            if t in CSO_PY_OK_FLAGS:",
         '            if t.startswith("-B") or t in CSO_PY_OK_FLAGS:'),
        ("fd 복제/파일 구분",
         "        if target.isdigit() and _is_fd_dup_op(op):",
         "        if target.isdigit():"),
        ("경로 대소문자 접기", '    return (p or "").lower()', '    return (p or "")'),
        ("세그먼트별 승인",
         '        if approval_allows(seg_command, ctx):\n            return True, "TTL 승인 확인됨',
         '        if approval_allows(seg_command, ctx) or True:\n            return True, "TTL 승인 확인됨'),
        ("데몬 소유 상태 보호",
         "    if _is_protected_state_path(path, ctx):",
         "    if False and _is_protected_state_path(path, ctx):"),
        ("경로 지정 실행 금지",
         '        if "/" in raw_head or "\\\\" in raw_head:',
         "        if False:"),
        ("argparse 접두 축약",
         '    return (head.startswith("--") and len(head) > 2 and bad.startswith("--")\n'
         "            and bad.startswith(head))",
         "    return False"),
        ("Read-before-Write 면제",
         "        essential = _read_tool_essential(tool, ti, ctx)",
         "        essential = False"),
        ("역할 미확정 교집합",
         "    verdicts = [(r,) + tuple(decide(tool, tool_input, r, ctx)) for r in roles]",
         "    verdicts = [(roles[0],) + tuple(decide(tool, tool_input, roles[0], ctx))]"),
        ("go env 쓰기 옵션",
         "    for bad in BUILDER_SUB_WRITE_OPTS.get((base, sub), ()):",
         "    for bad in ():"),
        ("tail 결합 follow 플래그",
         '                    or (not a.startswith("--") and CSO_TAIL_FOLLOW_RE.match(a))',
         "                    or False"),
        ("주석 제거(단어 시작 `#`)",
         '        if ch == "#" and at_word_start:',
         '        if ch == "#" and at_word_start and False:'),
        # ★R1-2(codex 위임 검체에서 나온 실증 우회) — 셸이 실행하는 것과 판정기가 보는 것의 차이
        ("줄 이어붙이기(`\\`+개행) 제거",
         '            if ch == "\\n":\n'
         '                if out and out[-1] == "\\\\":\n'
         "                    out.pop()\n"
         "                    mask.pop()\n"
         "                esc = False\n"
         "                i += 1\n"
         "                continue\n",
         ""),
        ("영폭 문자 거부", "    if has_invisible(command):\n        return None\n", ""),
        ("변수 확장 게이트",
         '        if ("$" in st or _has_sentinel(st)) and _resolve_token(t, ctx) is None:',
         "        if False:"),
        ("중괄호 확장 게이트(쉼표·범위)",
         "            if has_comma or has_range:",
         "            if False:"),
        # ★R2 — 리뷰 2차가 실증한 우회·오탐의 짝. 검사를 지우면 위 self_test_r2 가 실패해야 한다.
        ("리터럴 `$`·`~` 센티널(인용 인지)",
         '        if m in ("s", "e") and ch == "$":\n'
         "            out.append(SENT_DOLLAR)\n"
         '        elif m in ("s", "e") and ch == "~":\n'
         "            out.append(SENT_TILDE)\n",
         "        if False:\n            pass\n"),
        ("인용 밖 변수 확장 거부", '        if ch == "$":', "        if False:"),
        ("글롭 거부", "        if ch in GLOB_CHARS:", "        if False:"),
        ("변수 이름 경계", "_VAR_PACK_RE = re.compile(r\"\\$CYS_PACK_DIR(?![A-Za-z0-9_])\")",
         "_VAR_PACK_RE = re.compile(r\"\\$CYS_PACK_DIR\")"),
        ("심링크 해소(realpath)", "        ap = os.path.realpath(ap)\n", ""),
        ("승인의 세그먼트 범위",
         "        seg_command = _seg_command(seg)", "        seg_command = command"),
        ("승인 대상 복합 실행 금지",
         "            ok, why, ess = _cys_segment_verdict(seg, ctx, seg_command, n_segs)",
         "            ok, why, ess = _cys_segment_verdict(seg, ctx, seg_command, 1)"),
        ("`cys` 하위 명령은 동사 바로 뒤",
         "        sub = rest[0] if rest else None",
         "        sub = next((a for a in rest if not a.startswith(\"-\")), None)"),
        ("읽기 명령의 **명령별** 값 옵션",
         "        VALUE_OPTS = CSO_TARGET_VALUE_OPTS.get(base, CSO_TARGET_VALUE_OPTS_DEFAULT)",
         '        VALUE_OPTS = ("-a", "-n", "-c", "--algorithm", "--lines", "--bytes")'),
        ("카운터 내용 검증",
         '    if len(body) > COUNTER_MAX_BYTES or body.strip(b"\\x01"):',
         "    if False:"),
        ("git 파일 출력 옵션",
         "    if _opt_hit(sub_args, GIT_FILE_OUT_OPTS):", "    if False:"),
        # ※`ch == "\\n"`(CR 은 개행이 아니다)은 **이중 방어**라 단독 변이가 판정을 바꾸지 않는다 —
        #   CR 이 든 명령은 아래 `DIVERGENT_CHARS` 가 먼저 거부한다. 공허한 변이를 넣지 않는다.
        ("CR 거부(판정·실행 갈림)",
         'DIVERGENT_CHARS = ZERO_WIDTH_CHARS + ("\\r",)',
         "DIVERGENT_CHARS = ZERO_WIDTH_CHARS"),
        ("cargo --config 키 allowlist",
         '            if base == "cargo" and t == "--config":',
         "            if False:"),
        ("선행 환경 할당 거부",
         '        _eqh = raw_head.split("=", 1)[0]', "        _eqh = \"\""),
        ("빌드 임의 실행·소스 대체 옵션",
         '        if any(t == d or t.startswith(d + "=") for d in BUILDER_DENY_OPTS):',
         "        if False:"),
        ("옵션 종료 `--` 처리",
         '    args = raw_args[:raw_args.index("--")] if "--" in raw_args else raw_args',
         "    args = raw_args"),
    ]

    @NEED_SH
    def test_removing_a_check_fails_self_test(self):
        src = HOOK.read_text(encoding="utf-8")
        base = subprocess.run([SH, str(HOOK), "--self-test"], capture_output=True, text=True,
                              timeout=120)
        self.assertEqual(base.returncode, 0,
                         "기준 self-test 가 이미 실패한다: %s" % base.stderr[-400:])
        with tempfile.TemporaryDirectory(prefix="capgate-mut-") as td:
            shutil.copy2(str(LIB), os.path.join(td, "_lib.sh"))
            blind, missing = [], []
            for label, old, new in self.MUTATIONS:
                if src.count(old) != 1:
                    missing.append(label)
                    continue
                p = os.path.join(td, "mut.sh")
                with open(p, "w", encoding="utf-8") as f:
                    f.write(src.replace(old, new))
                r = subprocess.run([SH, p, "--self-test"], capture_output=True, text=True,
                                   timeout=120)
                if r.returncode == 0:
                    blind.append(label)
            self.assertEqual(missing, [],
                             "음성 대조 패턴이 코드와 어긋난다(검체가 낡았다): %s" % missing)
            self.assertEqual(blind, [],
                             "검사를 지워도 self-test 가 통과한다(사각지대): %s" % blind)


if __name__ == "__main__":
    unittest.main(verbosity=2)
