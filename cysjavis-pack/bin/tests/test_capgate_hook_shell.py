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


CACHE_DIR_NAME = "cys-role-authority.d"
CACHE_PREFIX = "cys-capgate-role-"


def _slug(v):
    """공용 `cys_role_slug`(`tr -cs 'A-Za-z0-9.-' '_'` + 80자 절단) 미러.

    ★0.14.31 성찰 G1: 훅 전용 `capgate_slug`(손실 치환·절단 없음)를 없애고 공용층 규칙으로
      접었다. 이 미러가 어긋나면 아래 캐시 검체가 **엉뚱한 파일**을 심어 조용히 통과한다.
    """
    return re.sub(r"[^A-Za-z0-9.-]+", "_", v or "")[:80]


def _pct(v):
    """공용 `cys_role_pct_esc` 미러(`%`→`%25` 먼저, `:`→`%3A`)."""
    return (v or "").replace("%", "%25").replace(":", "%3A")


def sock_id(env):
    """공용 `cys_role_sock_id_init` 미러 — 캐시 레코드에 **원문 그대로** 실리는 종단점 신원."""
    s = env.get("CYS_SOCKET") or env.get("JAVIS_SOCKET") or env.get("AITERM_SOCKET") or ""
    if s:
        if not (s.startswith("/") or (("/" not in s) and
                                      (s.startswith("\\\\.\\pipe\\") or
                                       s.startswith("\\\\?\\pipe\\")) and len(s) > 9)):
            return ""
    else:
        s = "default:%s:%s" % (_pct(env.get("XDG_STATE_HOME", "")), _pct(env.get("HOME", "")))
    if len(s.encode("utf-8")) > 512 or "\n" in s or "\r" in s:
        return ""
    return s


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
        self.cysenvlog = self.root / "cys-env.log"
        self.env = dict(os.environ)
        self.env.update({
            "HOME": str(self.home),
            "TMPDIR": str(self.tmpdir),
            "CYS_STATE_DIR": str(self.state),
            "CYS_PACK_DIR": str(self.pack),
            "CYS_PY": sys.executable,
            "CYS_FAKE_LOG": str(self.cyslog),
            "CYS_FAKE_ENV_LOG": str(self.cysenvlog),
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
        body = ["#!/bin/sh", 'printf "%s\\n" "$*" >> "$CYS_FAKE_LOG"',
                # ★스텁이 **받은 env** 를 기록한다 — 소스 문자열 핀이 아니라 실제 자식 환경으로
                #   `CYS_NO_AUTOSTART` 봉인을 잰다(0.14.31 성찰 G5).
                'printf "%s\\n" "${CYS_NO_AUTOSTART:-unset}" >> "$CYS_FAKE_ENV_LOG"']
        if role:
            body.append('printf "%s\\n"' % role)
        body.append("exit %d" % rc)
        p.write_text("\n".join(body) + "\n", encoding="utf-8")
        p.chmod(0o755)
        return p

    def cache_dir(self):
        d = self.tmpdir / CACHE_DIR_NAME
        d.mkdir(mode=0o700, exist_ok=True)
        return d

    def cache_path(self, surface="7", socket="", epoch=None):
        """공용 캐시 디렉터리 안의 capgate 전용 레코드 경로(0.14.31 성찰 G1).

        세대(boot-epoch)는 **파일명이 아니라 레코드**에 있다 — 재기동 고아가 남지 않는다.
        """
        env = dict(self.env)
        if socket:
            env["CYS_SOCKET"] = socket
        sid = sock_id(env)
        return self.cache_dir() / (CACHE_PREFIX + "%s-%s" % (_slug(surface), _slug(sid)))

    def plant_cache(self, role, surface="7", age=0, socket="", epoch="-", sockid=None):
        """`"<ts> <역할|-> <세대> <소켓신원>"` 4필드 레코드를 심는다."""
        import time
        p = self.cache_path(surface, socket)
        env = dict(self.env)
        if socket:
            env["CYS_SOCKET"] = socket
        sid = sock_id(env) if sockid is None else sockid
        p.write_text("%d %s %s %s\n" % (int(time.time()) - age, role, epoch, sid),
                     encoding="utf-8")
        p.chmod(0o600)
        return p

    def calls_log(self):
        return self.cyslog.read_text(encoding="utf-8") if self.cyslog.exists() else ""

    def calls_env(self):
        """스텁 `cys` 가 실제로 받은 `CYS_NO_AUTOSTART` 값들."""
        if not self.cysenvlog.exists():
            return []
        return self.cysenvlog.read_text(encoding="utf-8").split()

    # ── 실행 ────────────────────────────────────────────────────────────────
    def run_hook(self, tool="Bash", tool_input=None, session_id="s-1", cwd=None, **envkw):
        env = dict(self.env)
        env.update({k: str(v) for k, v in envkw.items()})
        payload = json.dumps({"session_id": session_id, "tool_name": tool,
                              "tool_input": tool_input or {}})
        r = subprocess.run([SH, str(HOOK)], input=payload, env=env,
                           cwd=str(cwd) if cwd else None,
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
         '        elif ch == "~" and (m in ("s", "e", "d")\n'
         '                            or (m == "u" and not at_word_start)):\n'
         "            out.append(SENT_TILDE)\n",
         "        if False:\n            pass\n"),
        # ★triage(2026-09-08) — 판정기와 bash 가 갈리던 잔여 갈래. 검사를 지우면 self_test_r2 ⑭가
        #   실패해야 한다(음성 대조가 공허하지 않은지는 이 목록의 존재 이유다).
        ("인용/비-단어시작 틸드 인지(T4)",
         '        elif ch == "~" and (m in ("s", "e", "d")\n'
         '                            or (m == "u" and not at_word_start)):',
         '        elif m in ("s", "e") and ch == "~":'),
        ("인용된 셸 구두점 센티널(T5)",
         '        elif m != "u" and ch in PUNCT_SENTINELS:\n'
         "            out.append(PUNCT_SENTINELS[ch])\n",
         "        elif False:\n            pass\n"),
        ("중괄호 짝 스택(T2)",
         '        if ch == "{":\n            stack.append(i)',
         '        if ch == "{":\n            stack[:] = [i]'),
        ("reviewer 경로 중괄호 술어(T3)",
         "    _bz = brace_expansion_hazard(command)\n    if _bz:\n        return True, _bz\n",
         ""),
        # ★R2 수렴 — 이번 라운드에 신설한 검사도 **지우면 실패해야** 한다(공허한 검사 금지).
        ("ANSI-C 인용 거부(R2·codex)",
         "    _az = ansi_c_quote_hazard(command)\n    if _az:\n        return True, _az\n", ""),
        ("명령 이름 자리 글롭(R2·claude)",
         '            if tok not in ("[", "[[") and any(g in tok for g in GLOB_CHARS):',
         "            if False:"),
        ("선행 환경 할당 실행기 주입(R2·claude)",
         "                if env_assign_is_write(name):", "                if False:"),
        # ★수렴 R2: 래퍼 인식은 `_wrapper_name`(경로·`.exe`·대소문자 정규화) 한 규칙이 됐다 —
        #   그 정규화를 지우면(`tok in WRAPPERS` 문면 비교) `/usr/bin/env rm -rf` 가 다시 샌다.
        ("래퍼도 이름으로(R2·claude)",
         "            _wname = _wrapper_name(tok)",
         "            _wname = tok if tok in WRAPPERS else None"),
        # ★수렴 R2(major): 셸 실행기의 **값 옵션** 인식을 지우면 `bash -o pipefail -c '<위조>'` 가
        #   재귀를 건너뛴다(실측 ALLOW → 지금은 DENY).
        ("셸 실행기 값 옵션(수렴 R2)",
         "            if raw in SHELL_VALUE_OPTS:       # `-o pipefail` — 값은 스크립트가 아니다",
         "            if False:"),
        # 묶음 옵션의 **마지막 글자**가 값을 먹는 갈래(`bash -eo pipefail -c`)는 위 검사가 못 잡는다.
        ("셸 실행기 묶음 값 옵션(수렴 R2)",
         "            if body and (raw[0] + body[-1]) in SHELL_VALUE_OPTS:",
         "            if False:"),
        # ★수렴 R2(major): 래퍼의 **긴 옵션** 처리를 지우면 `env --unset X sh -c '<위조>'` 가 샌다.
        ("래퍼 긴 옵션(수렴 R2)",
         "                    if raw in wvals or raw in wlong:",
         "                    if raw in wvals:"),
        ("셸 축 폐기 장치(R2·T9)",
         '    shell = ("/dev/null",) + (("NUL", "nul") if osname == "nt" else ())',
         "    shell = builder"),
        ("이물 판정 비교 모양(R2·T9)",
         '        if "/" in f:\n            if n == f:\n                return True\n'
         "        elif os.path.basename(n) == f:\n            return True\n",
         "        if os.path.basename(n) == f:\n            return True\n"),
        ("reviewer deny 진단(R2·T3)",
         '            return True, ("reviewer/planner may not run write-shell"\n'
         '                          + (" — %s" % _wwhy if _wwhy else ""))',
         '            return True, "reviewer/planner may not run write-shell"'),
        ("값 옵션 결합 표기(T1)",
         '        _vopt = next((o for o in value_opts if t == o or t.startswith(o + "=")), None)',
         "        _vopt = t if t in value_opts else None"),
        ("cargo --config 키 경계(T1)",
         "        elif v == k or v.startswith(k + \"=\"):",
         "        elif v.startswith(k):"),
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
         '            if base == "cargo" and _vopt == "--config":',
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



# ─────────────────────────────────────────────────────────────────────────────
# ★독립 재유도(triage 2026-09-08 · P2-WP3A-capgate) — "판정기가 보는 명령 ≠ bash 가 실행하는
#   명령" 의 잔여 4갈래. 각 케이스는 **먼저 bash 로 실제 확장을 관측**해서 픽스처가 사실임을
#   증명하고(픽스처가 틀리면 검체가 공허하다 — codex 가 `/w/home` 틸드 검체에서 지적한 것이
#   바로 그것이다), 같은 문자열을 훅에 넣어 판정을 잰다.
# ─────────────────────────────────────────────────────────────────────────────
@NEED_SH
class TriageQuotingAndExpansion(_HookEnv):
    """인용·중괄호 표기로 게이트가 열리는 갈래(triage CONFIRMED 6종)."""

    def bash_words(self, snippet):
        """bash 가 그 조각을 어떤 **인자들**로 펼치는가(확장을 실행하지 않고 관측만)."""
        r = subprocess.run([SH, "-c", "printf '%s\\n' " + snippet],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.splitlines()

    def run_hook_in(self, cwd, tool="Bash", tool_input=None, session_id="s-1", **envkw):
        """`run_hook` 과 같되 **cwd 를 고정**한다 — 상대 경로 판정이 검체 실행 위치에 따라
        갈리면 그 검체는 우연히 통과한다(codex 가 지적한 픽스처 오염과 같은 층)."""
        env = dict(self.env)
        env.update({k: str(v) for k, v in envkw.items()})
        payload = json.dumps({"session_id": session_id, "tool_name": tool,
                              "tool_input": tool_input or {}})
        r = subprocess.run([SH, str(HOOK)], input=payload, env=env, cwd=str(cwd),
                           capture_output=True, text=True, timeout=90)
        return HookRun(r.returncode, r.stdout, r.stderr)

    def home_pack(self):
        """정상 설치 형상(`$HOME/.cys/pack`) — 틸드 판정을 실기와 같은 세계에서 잰다."""
        pack = self.home / ".cys" / "pack"
        (pack / "bin").mkdir(parents=True, exist_ok=True)
        (pack / "round").mkdir(parents=True, exist_ok=True)
        return pack

    # ── ① 인용된 틸드 = 리터럴 상대 경로(임의 python 사본 실행) ─────────────────
    def test_double_quoted_tilde_is_not_the_installed_pack_tool(self):
        """★triage blocking(codex): bash 는 **큰따옴표 안 틸드를 확장하지 않는다**.

        판정기는 `_norm`(expanduser)으로 그것을 설치 팩 도구로 정규화해 통과시키지만,
        셸은 cwd 아래 `./~/.cys/pack/bin/javis_preflight.py` 를 실행한다 — 같은 이름의
        **다른 파일**이다(동명 사본 실행 차단이 무너진다).
        """
        self.assertEqual(self.bash_words('"~/x"'), ["~/x"],
                         "선행 사실: bash 가 큰따옴표 안 틸드를 확장하지 않는다")
        pack = self.home_pack()
        r = self.run_hook("Bash",
                          {"command": 'python3 "~/.cys/pack/bin/javis_preflight.py" --self-test'},
                          CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertTrue(r.denied,
                        "인용된 틸드가 설치 팩 도구로 오인돼 통과했다(임의 사본 실행): %r/%r"
                        % (r.out, r.err))

    def test_single_quoted_tilde_is_not_the_installed_pack_tool(self):
        """작은따옴표도 같다 — 센티널로 복원한 뒤 `_norm` 이 다시 확장해 버린다."""
        self.assertEqual(self.bash_words("'~/x'"), ["~/x"],
                         "선행 사실: bash 가 작은따옴표 안 틸드를 확장하지 않는다")
        pack = self.home_pack()
        r = self.run_hook("Bash",
                          {"command": "python3 '~/.cys/pack/bin/javis_preflight.py' --self-test"},
                          CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertTrue(r.denied,
                        "작은따옴표 틸드가 통과했다(임의 사본 실행): %r/%r" % (r.out, r.err))

    # ── ② 인용된 리다이렉트 문자가 금지 옵션을 삼킨다 ──────────────────────────
    def test_quoted_redirection_operator_does_not_swallow_the_next_option(self):
        """★triage blocking(codex): `'>'` 는 **연산자가 아니라 인자**다.

        판정기는 그것을 리다이렉트로 읽고 **다음 토큰을 대상으로 소비**해서, 금지 옵션
        `--clear-first`(대상 pane 입력 버퍼 Ctrl-U)가 판정에서 사라진 채 실행된다.
        """
        self.assertEqual(self.bash_words("'>' --clear-first"), [">", "--clear-first"],
                         "선행 사실: 인용된 `>` 는 인자다")
        deny_plain = self.run_hook_in(self.tmpdir, "Bash",
                                      {"command": "cys send --to master --clear-first"},
                                      CYS_ROLE="cso")
        self.assertTrue(deny_plain.denied, "대조군: 인용 없는 --clear-first 는 이미 deny 다")
        r = self.run_hook_in(self.tmpdir, "Bash",
                             {"command": "cys send --to master '>' --clear-first"},
                             CYS_ROLE="cso")
        self.assertTrue(r.denied,
                        "인용된 `>` 가 금지 옵션을 삼켜 통과했다: %r/%r" % (r.out, r.err))

    # ── ③ 중첩 중괄호가 확장 위험 판정을 무력화한다 ───────────────────────────
    def test_nested_brace_expansion_is_refused(self):
        """★triage blocking(claude): `--{clear-first,x{y}}` 는 `--clear-first` 로 펼쳐진다.

        `cso_expansion_hazard` 의 중괄호 스캔은 `{` 마다 `depth_start` 를 **덮어써서**
        안쪽 쌍이 닫히면 바깥 쌍을 잃는다(쉼표를 가진 바깥 쌍이 통째로 무검사).
        """
        self.assertEqual(self.bash_words("--{clear-first,x{y}}"),
                         ["--clear-first", "--x{y}"],
                         "선행 사실: 중첩 중괄호도 인자를 늘린다")
        r = self.run_hook("Bash", {"command": "cys send --to master --{clear-first,x{y}}"},
                          CYS_ROLE="cso")
        self.assertTrue(r.denied,
                        "중첩 중괄호가 금지 옵션을 숨겼다: %r/%r" % (r.out, r.err))

    def test_nested_brace_expansion_is_refused_for_stream_flags(self):
        """같은 구멍으로 `tail -f`(종결 없는 관측)가 통과한다."""
        self.assertEqual(self.bash_words("-{f,x{y}}"), ["-f", "-x{y}"],
                         "선행 사실: 중첩 중괄호가 `-f` 를 만든다")
        pack = self.home_pack()
        target = str(pack / "round" / "SESSION_STATE.md")
        (pack / "round" / "SESSION_STATE.md").write_text("s", encoding="utf-8")
        plain = self.run_hook("Bash", {"command": "tail -f %s" % target},
                              CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertTrue(plain.denied, "대조군: `tail -f` 는 이미 deny 다")
        r = self.run_hook("Bash", {"command": "tail -{f,x{y}} %s" % target},
                          CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertTrue(r.denied,
                        "중첩 중괄호로 `tail -f` 가 통과했다: %r/%r" % (r.out, r.err))

    def test_reviewer_write_shell_survives_brace_expansion(self):
        """★triage(claude major): reviewer 의 write-shell deny 에는 중괄호 처리가 **아예 없다**.

        `rm{,x} <path>` 는 bash 에서 `rm rmx <path>` 이고 `rm` 이 실제로 돈다
        (producer≠evaluator 의 기계 집행이 표기 하나로 사라진다).
        """
        self.assertEqual(self.bash_words("rm{,x} /x/build"), ["rm", "rmx", "/x/build"],
                         "선행 사실: `rm{,x}` 의 첫 인자는 `rm` 이다")
        plain = self.run_hook("Bash", {"command": "rm -rf /x/build"},
                              CYS_ROLE="reviewer-codex")
        self.assertTrue(plain.denied, "대조군: `rm -rf` 는 이미 deny 다")
        r = self.run_hook("Bash", {"command": "rm{,x} /x/build"}, CYS_ROLE="reviewer-codex")
        self.assertTrue(r.denied,
                        "중괄호 표기로 write-shell deny 가 통과했다: %r/%r" % (r.out, r.err))

    # ── ④ `--opt=value` 결합 표기가 값 검사를 건너뛴다 ─────────────────────────
    def test_cargo_config_equals_form_is_validated(self):
        """★triage blocking(claude)/major(codex): `--config=<k=v>` 는 `t in value_opts` 를
        만족하지 못해 `CARGO_CONFIG_SAFE_KEYS` 검증을 통째로 건너뛴다 — 분리 표기는 deny 인데
        결합 표기는 allow 다(실행기 주입 경로가 그대로 남는다)."""
        inj = "target.aarch64-apple-darwin.runner=['/tmp/runner.sh']"
        sep = self.run_hook("Bash", {"command": "cargo --config %s test --offline" % inj},
                            CYS_ROLE="reviewer-codex")
        self.assertTrue(sep.denied, "대조군: 분리 표기는 이미 deny 다")
        r = self.run_hook("Bash", {"command": "cargo --config=%s test --offline" % inj},
                          CYS_ROLE="reviewer-codex")
        self.assertTrue(r.denied,
                        "`--config=` 결합 표기가 실행기 주입 검증을 건너뛰었다: %r/%r"
                        % (r.out, r.err))

    # ── ⑤ Windows 예약 이름이 unix 에서는 평범한 파일이다 ─────────────────────
    @unittest.skipIf(os.name == "nt", "unix 전용 계약(nt 에서는 NUL 이 실제 장치다)")
    def test_unix_NUL_redirect_is_an_ordinary_file(self):
        """★triage(codex major): `> NUL` 은 unix 에서 **cwd 의 일반 파일**을 만들거나 자른다.

        `NULL_SINKS` 가 철자만 보고 경로 보호를 면제한다(플랫폼 분기 없음).
        """
        r = self.run_hook_in(self.tmpdir, "Bash", {"command": "cys status > NUL"},
                             CYS_ROLE="cso")
        self.assertTrue(r.denied,
                        "`> NUL` 이 경로 검사를 면제받았다(unix 에서는 일반 파일이다): %r/%r"
                        % (r.out, r.err))



# ─────────────────────────────────────────────────────────────────────────────
# ★수렴 R2(2026-09-08) — 최종 리뷰어 2인이 남긴 잔여 지적의 회귀 검체.
# ─────────────────────────────────────────────────────────────────────────────
class NullSinkPlatformAxis(unittest.TestCase):
    """★R2 major(리뷰어 2인): `NULL_SINKS` 를 **네이티브 도구 축**에서 파생시키면 nt 에서
    `> /dev/null` 이 새로 거부된다(Git Bash 는 그것을 매핑한다) — Windows 전용 회귀다.

    Windows 가 없어도 잰다: 훅에 박힌 상수 정의문을 **`os.name` 만 바꾼 이름공간에서 실제로
    실행**해서 양 플랫폼 분기의 값을 둘 다 계산한다(문자열 검색이 아니라 실행이다).
    """

    def axes(self):
        """훅의 축 계산 함수 3개를 **그대로 실행**한다(문자열 검색이 아니라 실행이다)."""
        src = HOOK.read_text(encoding="utf-8")
        a = src.index("def _null_device_axes(")
        b = src.index("PLATFORM_NULL_DEVICES, SHELL_NULL_DEVICES = _null_device_axes(os.name)")
        c = src.index("def _is_foreign_null(")
        d = src.index("\n    return False\n", c) + len("\n    return False\n")

        class _OS(object):
            path = os.path
        ns = {"os": _OS}
        exec(compile(src[a:b] + "\n\n" + src[c:d], "<capgate-null-axes>", "exec"), ns)
        return ns

    def consts(self, osname):
        ns = self.axes()
        builder, shell = ns["_null_device_axes"](osname)
        return {"BUILDER_NULL_SINKS": builder, "SHELL_NULL_DEVICES": shell,
                "FOREIGN_NULL_NAMES": ns["_foreign_null_names"](shell),
                "NULL_SINKS": set(shell) | {"/dev/stdout", "/dev/stderr", "/dev/tty"},
                "_is_foreign_null": ns["_is_foreign_null"]}

    def test_git_bash_dev_null_survives_on_windows(self):
        nt = self.consts("nt")
        self.assertIn("/dev/null", nt["NULL_SINKS"],
                      "nt 에서 `> /dev/null` 이 리다이렉트 면제를 잃었다 — Git Bash 가 매핑하는 "
                      "관용구를 Windows 에서만 막는다(plan §7 Windows 행)")
        for d in ("/dev/stdout", "/dev/stderr", "/dev/tty"):
            self.assertIn(d, nt["NULL_SINKS"], "nt 에서 `%s` 가 사라졌다(MSYS 는 매핑한다)" % d)
        self.assertIn("NUL", nt["NULL_SINKS"], "nt 의 Win32 예약 장치가 빠졌다")

    def test_unix_NUL_is_not_a_sink(self):
        ux = self.consts("posix")
        self.assertNotIn("NUL", ux["NULL_SINKS"],
                         "unix 에서 `NUL` 이 면제를 받는다(cwd 의 일반 파일이다 — T9 본래 요구)")
        self.assertIn("/dev/null", ux["NULL_SINKS"])

    def test_foreign_axis_is_not_a_dead_branch_on_either_platform(self):
        """이물 이름 판정의 **양변 모양**이 같은가 — 종전 nt 분기는 영원히 거짓이었다."""
        ux, nt = self.consts("posix"), self.consts("nt")
        self.assertEqual(tuple(ux["FOREIGN_NULL_NAMES"]), ("NUL", "nul"),
                         "unix 의 이물 이름이 `NUL`/`nul` 이 아니다")
        self.assertEqual(tuple(nt["FOREIGN_NULL_NAMES"]), (),
                         "nt 에서 이물 이름이 남았다 — 그 철자는 Git Bash 가 여는 이름이고, "
                         "basename 비교와 모양이 달라 분기가 죽는다")
        # 남는 축(네이티브 도구)은 **갈라진 채로** 있어야 한다 — 두 축을 합치면 R1 회귀다.
        self.assertEqual(tuple(nt["BUILDER_NULL_SINKS"]), ("NUL", "nul"))
        self.assertEqual(tuple(ux["BUILDER_NULL_SINKS"]), ("/dev/null",))
        # 비교 **모양**: 경로형은 전체, 맨이름형은 basename — 한쪽만 접으면 분기가 죽는다.
        _f = ux["_is_foreign_null"]
        self.assertTrue(_f("/dev/null", ("/dev/null",)), "경로형 전체 비교가 깨졌다")
        self.assertFalse(_f("/w/repo/null", ("/dev/null",)),
                         "경로형을 basename 으로 비교해 평범한 파일 `null` 을 오탐한다")
        self.assertTrue(_f("sub/NUL", ("NUL", "nul")), "맨이름형을 디렉터리 아래에서 놓친다")


@NEED_SH
class TriageConvergenceR2(_HookEnv):
    """표기 축 잔여 4건 — 전부 **bash 실측으로 픽스처를 증명**하고 같은 문자열로 판정을 잰다."""

    def bash_words(self, snippet):
        r = subprocess.run([SH, "-c", "printf '%s\\n' " + snippet],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.splitlines()

    def test_ansi_c_quoted_cargo_option_is_validated(self):
        """★R2 major(codex): shlex 는 `$'…'` 를 모른다 — 게이트는 `$--config=…` 라는 **없는
        토큰**을 보고 옵션 검증을 건너뛰었고, bash 는 `--config=…` 를 cargo 에 넘겼다."""
        self.assertEqual(self.bash_words("""$'--config=build.rustc-wrapper="/tmp/w"'"""),
                         ['--config=build.rustc-wrapper="/tmp/w"'],
                         "선행 사실: bash 는 `$` 를 지우고 인용을 푼다")
        plain = self.run_hook("Bash",
                              {"command": 'cargo test --config=build.rustc-wrapper="/tmp/w"'},
                              CYS_ROLE="reviewer-codex")
        self.assertTrue(plain.denied, "대조군: 인용 없는 표기는 이미 deny 다")
        r = self.run_hook("Bash",
                          {"command": """cargo test $'--config=build.rustc-wrapper="/tmp/w"'"""},
                          CYS_ROLE="reviewer-codex")
        self.assertTrue(r.denied,
                        "ANSI-C 인용이 `CARGO_CONFIG_SAFE_KEYS` 검증을 비껴갔다: %r/%r"
                        % (r.out, r.err))

    def test_glob_in_command_name_position_is_refused(self):
        """★R2 minor(claude): `/bin/r?` 는 bash 가 `/bin/rm` 으로 바꾼다(중괄호와 같은 층)."""
        r = self.run_hook("Bash", {"command": "/bin/r? -rf /x/build"}, CYS_ROLE="reviewer-codex")
        self.assertTrue(r.denied, "명령 이름 글롭으로 rm 이 통과했다: %r/%r" % (r.out, r.err))
        r2 = self.run_hook("Bash", {"command": "/bin/r[m] -rf /x/build"},
                           CYS_ROLE="reviewer-codex")
        self.assertTrue(r2.denied, "문자 클래스로 rm 이 통과했다: %r/%r" % (r2.out, r2.err))
        ok = self.run_hook("Bash", {"command": "rg pat /x/src/*.rs"}, CYS_ROLE="reviewer-codex")
        self.assertFalse(ok.denied,
                         "인자 자리 글롭까지 막았다 — 읽기 전용 조회 오탐(계획 §3-3): %r" % ok.out)

    def test_leading_env_assignment_cannot_inject_a_runner(self):
        """★R2 minor(claude): `--config=build.rustc-wrapper=…` 를 막고 `RUSTC_WRAPPER=…` 를
        열어 두면 같은 통제가 표기 하나로 비껴간다(CSO 경로는 이미 거부한다)."""
        for cmd in ("RUSTC_WRAPPER=/tmp/r.sh cargo test",
                    "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER=/tmp/r.sh cargo test"):
            r = self.run_hook("Bash", {"command": cmd}, CYS_ROLE="reviewer-codex")
            self.assertTrue(r.denied, "선행 할당 실행기 주입이 통과했다(%s): %r" % (cmd, r.out))
        ok = self.run_hook("Bash", {"command": "RUST_BACKTRACE=1 cargo test"},
                           CYS_ROLE="reviewer-codex")
        self.assertFalse(ok.denied, "정상 환경 변수까지 막았다(오탐): %r" % ok.out)

    def test_reviewer_deny_names_the_notation_that_blocked_it(self):
        """★R2 minor(claude): 읽기 명령이 **표기 때문에** 막혔으면 문면이 그것을 가리켜야 한다."""
        r = self.run_hook("Bash", {"command": "ls dir/{a,b}"}, CYS_ROLE="reviewer-codex")
        self.assertTrue(r.denied, "대조군: 중괄호는 거부 방향이다(계획 §3-3)")
        self.assertIn("중괄호", r.reason,
                      "deny 문면이 무엇이 걸렸는지 말하지 않는다(좌석이 표기를 고칠 수 없다): %r"
                      % r.reason)

    def test_mixed_literal_token_still_cannot_forge_a_pack_tool(self):
        """codex 지적(deny→allow 확대)의 **안전 축**을 못으로 박는다: 혼합 토큰을 판정하게
        바꿨어도, 리터럴 센티널이 남은 경로는 설치 팩 도구로 정규화되지 않는다."""
        pack = self.home / ".cys" / "pack"
        (pack / "bin").mkdir(parents=True, exist_ok=True)
        cmd = ('python3 "$HOME/"\'$CYS_PACK_DIR/bin/javis_preflight.py\' --self-test')
        _w = self.bash_words('"$HOME/"\'$CYS_PACK_DIR/x\'')
        self.assertEqual(len(_w), 1, "선행 사실: 한 토큰이다: %r" % _w)
        self.assertTrue(_w[0].endswith("/$CYS_PACK_DIR/x"),
                        "선행 사실: 작은따옴표 안 `$` 는 확장되지 않는다(리터럴 디렉터리 "
                        "이름으로 남는다): %r" % _w)
        ok = self.run_hook("Bash",
                           {"command": 'python3 "$CYS_PACK_DIR/bin/javis_preflight.py" '
                                       "--self-test"},
                           CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertFalse(ok.denied,
                         "대조군: 온전히 확장되는 표기는 설치 팩 판정 도구다: %r" % ok.out)
        r = self.run_hook("Bash", {"command": cmd}, CYS_ROLE="cso", CYS_PACK_DIR=str(pack))
        self.assertTrue(r.denied,
                        "혼합 토큰의 **리터럴 부분이 확장돼** 설치 팩 도구로 오인됐다"
                        "(동명 사본 실행): %r/%r" % (r.out, r.err))


class HookCacheIdentity(_HookEnv):
    """★0.14.31 성찰 G1·G7·G14 — 캐시 레코드의 **종단점 신원**과 세대 성분.

    종전 훅은 캐시 파일명을 손실 슬러그로 만들고 레코드를 `<ts> <역할>` 2필드로 썼다:
    슬러그가 충돌하는 두 소켓(`…/s/x.sock` ↔ `…/s_x.sock`)이 **한 파일**을 공유하는데
    레코드에 신원이 없어 서로의 역할을 자기 것으로 읽었다. 그 방향은 **열림**이다 —
    앞 좌석의 비대상 역할(worker)이 뒤 좌석(reviewer)의 fast-path 가 되어 리뷰어가
    데몬에 묻지도 않고 자기 산출물을 고친다(로그에도 흔적이 없다).
    """

    def _colliding_sockets(self):
        a = self.root / "s" / "x.sock"
        b = self.root / "s_x.sock"
        (self.root / "s").mkdir(exist_ok=True)
        return str(a), str(b)

    def test_colliding_socket_slugs_share_a_file_but_not_a_verdict(self):
        """두 소켓이 **같은 캐시 파일**을 가리켜도 남의 레코드는 권위가 아니다."""
        a, b = self._colliding_sockets()
        pa, pb = self.cache_path(socket=a), self.cache_path(socket=b)
        self.assertEqual(pa, pb, "선행 사실: 두 종단점의 슬러그가 충돌해야 이 검체가 뜻이 있다")
        # A 좌석(worker)의 신선한 레코드를 심고 — B 좌석(reviewer)으로 도구를 부른다.
        self.plant_cache("worker", socket=a)
        self.fake_cys(role="reviewer-codex")
        r = self.run_hook("Write", {"file_path": "/nonexistent-repo/out.md", "content": "x"},
                          CYS_SURFACE_ID="7", CYS_SOCKET=b)
        self.assertIn("surface-role", self.calls_log(),
                      "남의 종단점 레코드를 fast-path 로 써서 데몬에 묻지 않았다(캐시 충돌)")
        self.assertTrue(r.denied,
                        "리뷰어가 캐시 충돌로 **구현 권한**을 얻었다: %r / %r" % (r.out, r.err))

    def test_matching_socket_identity_is_still_a_fastpath(self):
        """대조군 — 신원이 **맞으면** 종전대로 fast-path 다(비대상 좌석의 RPC 폭풍 방지)."""
        a, _b = self._colliding_sockets()
        self.plant_cache("worker", socket=a)
        self.fake_cys(role="reviewer-codex")
        r = self.run_hook("Write", {"file_path": "/nonexistent-repo/out.md", "content": "x"},
                          CYS_SURFACE_ID="7", CYS_SOCKET=a)
        self.assertEqual(self.calls_log(), "",
                         "신원이 맞는 비대상 캐시가 fast-path 가 아니다: %r" % self.calls_log())
        self.assertFalse(r.denied, "worker 좌석이 막혔다: %r" % r.out)

    def test_legacy_two_field_record_is_not_authoritative(self):
        """구형 `<ts> <역할>` 2필드 레코드(신원 없음)는 권위가 아니다 — 데몬에 묻는다."""
        import time
        self.fake_cys(role="reviewer-codex")
        p = self.cache_path()
        p.parent.mkdir(mode=0o700, exist_ok=True)
        p.write_text("%d worker\n" % int(time.time()), encoding="utf-8")
        p.chmod(0o600)
        r = self.run_hook("Write", {"file_path": "/nonexistent-repo/out.md", "content": "x"},
                          CYS_SURFACE_ID="7")
        self.assertIn("surface-role", self.calls_log(),
                      "신원 없는 구형 레코드를 권위로 읽었다")
        self.assertTrue(r.denied, "구형 레코드가 리뷰어의 구현 권한을 열었다: %r" % r.out)

    def test_cache_lives_in_the_shared_0700_directory(self):
        """캐시는 TMPDIR **루트**가 아니라 공용 0700 전용 디렉터리 안이다(이름 예측·심기 차단)."""
        self.fake_cys(role="master")
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        d = self.tmpdir / CACHE_DIR_NAME
        self.assertTrue(d.is_dir(), "공용 캐시 디렉터리가 없다")
        self.assertEqual(d.stat().st_mode & 0o777, 0o700, "캐시 디렉터리가 0700 이 아니다")
        stray = [p.name for p in self.tmpdir.iterdir()
                 if p.is_file() and p.name.startswith("cys-capgate-role-")]
        self.assertEqual(stray, [], "캐시가 TMPDIR 루트에 흩뿌려졌다: %s" % stray)
        rec = self.cache_path().read_text(encoding="utf-8").split()
        self.assertEqual(len(rec), 4, "레코드가 4필드(ts·역할·세대·신원)가 아니다: %r" % rec)
        self.assertEqual(rec[1], "master")
        self.assertEqual(rec[3], sock_id(self.env), "레코드에 종단점 신원이 원문으로 없다")

    def test_named_pipe_endpoint_does_not_read_cwd_boot_epoch(self):
        """★G7: 유닉스 `dirname '\\\\.\\pipe\\cys'` 는 `.` 이다 — cwd 의 `boot-epoch` 를 읽지 않는다.

        읽으면 같은 좌석이 **cwd 마다** 캐시를 따로 갖고(도구 호출마다 RPC), 저장소에 그
        이름의 파일이 있으면 전 pane 이 매 호출 2s 데드라인을 문다.
        """
        work = self.root / "repo"
        work.mkdir(exist_ok=True)
        (work / "boot-epoch").write_text("9999\n", encoding="utf-8")
        pipe = "\\\\.\\pipe\\cys"
        self.fake_cys(role="master")
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7", CYS_SOCKET=pipe, cwd=work)
        p = self.cache_path(socket=pipe)
        self.assertTrue(p.exists(), "명명 파이프 좌석이 캐시를 쓰지 않았다(백오프도 함께 꺼진다)")
        rec = p.read_text(encoding="utf-8").split()
        self.assertEqual(rec[2], "-",
                         "cwd 의 boot-epoch 가 세대 성분으로 실렸다: %r" % rec)

    def test_hostile_boot_epoch_folds_to_unknown(self):
        """★G7: 거대한 한 줄·FIFO `boot-epoch` 에서 유계로 접힌다(훅이 매달리지 않는다)."""
        import time
        sockdir = self.root / "sock"
        sockdir.mkdir(exist_ok=True)
        sock = str(sockdir / "cys.sock")
        self.fake_cys(role="master")
        # ⓐ 거대한 한 줄 — 4KB 유계 판독 + 토큰 문법에서 접힌다(파일명 ENAMETOOLONG 방지).
        (sockdir / "boot-epoch").write_text("A" * 200000, encoding="utf-8")
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7", CYS_SOCKET=sock)
        rec = self.cache_path(socket=sock).read_text(encoding="utf-8").split()
        self.assertEqual(rec[2], "-", "거대한 boot-epoch 가 그대로 세대 성분이 됐다: %r" % rec[2][:40])
        # ⓑ FIFO — 열기에서 매달리면 **모든 도구 호출**이 멈춘다.
        (sockdir / "boot-epoch").unlink()
        try:
            os.mkfifo(str(sockdir / "boot-epoch"))
        except (AttributeError, OSError):
            self.skipTest("mkfifo 불가 — FIFO 축을 잴 수 없다")
        t0 = time.time()
        r = self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7", CYS_SOCKET=sock)
        self.assertEqual(r.rc, 0)
        self.assertLess(time.time() - t0, 20,
                        "FIFO `boot-epoch` 에서 PreToolUse 훅이 매달렸다")

    def test_role_query_never_spawns_a_daemon(self):
        """★G5: 역할을 묻는 행위가 **데몬을 낳지 않는다**(`CYS_NO_AUTOSTART=1`).

        `cys` 는 소켓이 없으면 `connect()` 에서 형제 `cysd` 를 detached 로 **스폰한 뒤**
        폴링한다 — 밖의 `cys_timeout_run 2` 가 2s 에 죽여도 스폰은 이미 일어났다. 이 훅은
        matcher 없이 전 도구에 붙고 게이트 대상 좌석은 캐시 fast-path 를 쓰지 않으므로,
        데몬이 내려간 상태에서 **좌석당 5초에 한 번** 기동 시도가 된다(운영자가 의도적으로
        내린 데몬이 도구 호출 하나로 되살아난다 · 봉인표 ① 방향).
        """
        self.fake_cys(role="master")
        self.run_hook("Bash", {"command": "ls"}, CYS_SURFACE_ID="7")
        got = self.calls_env()
        self.assertTrue(got, "데몬 조회가 나지 않아 봉인을 잴 수 없다(계측 불능)")
        self.assertEqual(set(got), {"1"},
                         "훅이 autostart 를 막지 않고 `cys` 를 불렀다: %r" % got)

    def test_cache_dirname_matches_both_sibling_layers(self):
        """★G14: 역할 캐시 디렉터리 이름이 세 층에서 같은가(보호가 조용히 늙지 않게)."""
        sys.path.insert(0, str(BIN))
        try:
            import javis_role
        finally:
            sys.path.pop(0)
        hook = HOOK.read_text(encoding="utf-8")
        lib = LIB.read_text(encoding="utf-8")
        m = re.search(r'^ROLE_CACHE_DIRNAME = "([^"]+)"', hook, re.M)
        self.assertTrue(m, "훅에 ROLE_CACHE_DIRNAME 상수가 없다")
        ml = re.search(r'^CYS_ROLE_CACHE_DIRNAME="([^"]+)"', lib, re.M)
        self.assertTrue(ml, "_lib.sh 에 CYS_ROLE_CACHE_DIRNAME 상수가 없다")
        self.assertEqual(m.group(1), javis_role.CACHE_DIR_NAME,
                         "게이트의 자기상태 보호가 파이썬 짝과 다른 이름을 본다")
        self.assertEqual(ml.group(1), javis_role.CACHE_DIR_NAME,
                         "공용 셸층이 파이썬 짝과 다른 이름을 쓴다")
        self.assertEqual(m.group(1), CACHE_DIR_NAME, "이 검체의 미러가 낡았다")
        # capgate 캐시 basename 은 게이트 자기상태 접두 안에 있어야 이중으로 보호된다.
        mp = re.search(r'^CAPGATE_CACHE_PREFIX="([^"]+)"', hook, re.M)
        mg = re.search(r'^GATE_STATE_FILE_PREFIX = "([^"]+)"', hook, re.M)
        self.assertTrue(mp and mg)
        self.assertTrue(mp.group(1).startswith(mg.group(1)),
                        "capgate 캐시 이름이 게이트 자기상태 접두 밖이다: %r ⊄ %r"
                        % (mp.group(1), mg.group(1)))

if __name__ == "__main__":
    unittest.main(verbosity=2)
