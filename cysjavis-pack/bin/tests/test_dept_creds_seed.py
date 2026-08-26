#!/usr/bin/env python3
"""test_dept_creds_seed.py — ★T11(P3-5·R3-P03-5) Windows 한정 credentials 시드 헬퍼 핀.

seed_credentials_win 의 4조건 계약(①uname Windows 분기 ②opt-in CYS_DEPT_SEED_CREDS=1 기본 off
③copy-if-absent만·덮어쓰기 절대 금지 ④포크 dir 한정·원천=해석된 그 계정 base·소유자 전용 권한)을
유닛(함수 추출 실행)+배선(launch 완주) 양층에서 단언한다. Windows 분기는 uname 목(PATH 선두
$HOME/.local/bin — cys-dept 가 스스로 prepend)으로 시뮬레이션한다 — 실기 Windows 거동(MSYS cp/
chmod↔ACL 매핑·refresh 토큰 회전 발산)은 본 테스트 범위 밖(opt-in 기본 off 가 그 방벽).

  유닛(함수 추출):
    1) opt-in off(env 부재) = 완전 무동작(Windows 목이어도 사본 0·출력 0)
    2) opt-in on + 비-Windows(uname=Darwin 목) = 무동작(macOS 분기 자체 부재 계약)
    3) opt-in on + Windows 목 = 사본 생성·내용 동일·권한 0600
    4) 대상 기존재 = 무동작·기존 내용 보존(rotate 갱신 토큰 클로버 금지) + 로그 1줄
    5) 원천 부재 = loud skip(수동 /login 안내)·rc 0·사본 0
    6) 원천=대상 동일(포크 아님) = skip(공유 계정 dir 클로버 금지)
    7) 원천 빈값(base 유도 실패) = loud skip·rc 0
  배선(launch 재사용 경로 완주 · Windows 목):
    8) opt-in on: fork dir 에 .credentials.json 착지(원천=resolve_default_base 유도 base)
    9) opt-in off: 착지 없음(기본 off = 완전 무동작) — launch 자체는 정상 완주
"""
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")


def _write_exec(path, content):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    os.chmod(path, 0o755)


def extract_fn(name):
    """cys-dept 에서 함수 본문 추출(^name(){ … ^}) — 소스와의 드리프트 0(사본 아닌 실물 추출)."""
    src = open(DEPT, encoding="utf-8").read()
    m = re.search(r"^%s\(\)\{\n.*?^\}$" % re.escape(name), src, re.M | re.S)
    if not m:
        raise AssertionError("cys-dept 에서 %s 함수를 찾지 못했다(추출 앵커 드리프트)" % name)
    return m.group(0)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="deptcreds-")
        self.home = os.path.join(self.tmp, "home")
        self.bindir = os.path.join(self.home, ".local", "bin")
        os.makedirs(self.bindir, exist_ok=True)
        os.makedirs(os.path.join(self.home, ".cys"), exist_ok=True)

    def mock_uname(self, sysname):
        _write_exec(os.path.join(self.bindir, "uname"),
                    '#!/bin/sh\necho "%s"\n' % sysname)

    def env(self, **extra):
        env = dict(os.environ)
        env.update({"HOME": self.home,
                    "CYS_DEPTS_JSON": os.path.join(self.home, ".cys", "depts.json"),
                    "PATH": self.bindir + os.pathsep + env.get("PATH", "")})
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR",
                  "CYS_NO_AUTOSTART", "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG",
                  "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT",
                  "CYS_DEPT_SEED_CREDS"):
            env.pop(k, None)
        env.update(extra)
        return env


class HelperUnit(Base):
    """유닛층 — 함수 실물 추출 + 격리 드라이버(bash)로 4조건을 축별 단언."""

    def run_fn(self, srcbase, fork, **envextra):
        drv = os.path.join(self.tmp, "driver.sh")
        _write_exec(drv, "#!/usr/bin/env bash\nset -euo pipefail\n"
                    + extract_fn("seed_credentials_win")
                    + '\nseed_credentials_win "$1" "$2"\n')
        r = subprocess.run(["bash", drv, srcbase, fork], capture_output=True,
                           text=True, encoding="utf-8", env=self.env(**envextra),
                           timeout=30)
        return r.returncode, r.stdout, r.stderr

    def mkpair(self, with_src=True):
        srcbase = os.path.join(self.home, ".cys", "claude-default")
        fork = srcbase + "-dept-1"
        os.makedirs(srcbase, exist_ok=True)
        os.makedirs(fork, exist_ok=True)
        if with_src:
            with open(os.path.join(srcbase, ".credentials.json"), "w") as f:
                f.write('{"tok":"BASE"}')
        return srcbase, fork

    def test_1_optin_off_complete_noop(self):
        self.mock_uname("MINGW64_NT-10.0")
        srcbase, fork = self.mkpair()
        rc, out, err = self.run_fn(srcbase, fork)  # CYS_DEPT_SEED_CREDS 부재
        self.assertEqual(rc, 0)
        self.assertFalse(os.path.exists(os.path.join(fork, ".credentials.json")),
                         "opt-in off 인데 사본이 생겼다(기본 off=완전 무동작 위반)")
        self.assertEqual(out + err, "", "opt-in off 는 출력까지 0이어야 한다(완전 무동작)")

    def test_2_non_windows_noop(self):
        self.mock_uname("Darwin")
        srcbase, fork = self.mkpair()
        rc, out, err = self.run_fn(srcbase, fork, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0)
        self.assertFalse(os.path.exists(os.path.join(fork, ".credentials.json")),
                         "비-Windows 에서 사본이 생겼다(macOS 분기 부재 계약 위반 — Keychain 브리지 불가)")

    def test_3_windows_optin_copies_0600(self):
        self.mock_uname("MINGW64_NT-10.0")
        srcbase, fork = self.mkpair()
        rc, out, err = self.run_fn(srcbase, fork, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0, err)
        dst = os.path.join(fork, ".credentials.json")
        self.assertTrue(os.path.isfile(dst), "Windows+opt-in 인데 사본 미생성: %s" % err)
        self.assertEqual(open(dst).read(), '{"tok":"BASE"}')
        mode = stat.S_IMODE(os.stat(dst).st_mode)
        self.assertEqual(mode, 0o600, "사본 권한이 소유자 전용(0600)이 아니다: %o" % mode)
        self.assertIn("creds 시드 완료", err)
        self.assertEqual(out, "", "stdout 오염 금지(전 진단 >&2) 위반")

    def test_4_existing_target_never_overwritten(self):
        self.mock_uname("MSYS_NT-10.0")
        srcbase, fork = self.mkpair()
        dst = os.path.join(fork, ".credentials.json")
        with open(dst, "w") as f:
            f.write('{"tok":"FORK-FRESH"}')   # rotate 가 이미 갱신한 살아있는 토큰 재현
        rc, out, err = self.run_fn(srcbase, fork, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0)
        self.assertEqual(open(dst).read(), '{"tok":"FORK-FRESH"}',
                         "기존재 대상이 덮였다 — copy-if-absent(rotate 갱신 토큰 보호) 위반")
        self.assertIn("무동작", err, "기존재 무동작 로그 1줄 부재")

    def test_5_source_absent_loud_skip(self):
        self.mock_uname("MINGW64_NT-10.0")
        srcbase, fork = self.mkpair(with_src=False)
        rc, out, err = self.run_fn(srcbase, fork, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0, "원천 부재는 fail-open(rc 0)이어야 한다")
        self.assertFalse(os.path.exists(os.path.join(fork, ".credentials.json")))
        self.assertIn("/login", err, "수동 /login 폴백 안내 부재(loud skip 계약)")

    def test_6_source_equals_dest_skip(self):
        self.mock_uname("MINGW64_NT-10.0")
        srcbase, _ = self.mkpair()
        rc, out, err = self.run_fn(srcbase, srcbase, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0)
        self.assertIn("원천=대상 동일", err, "포크 아님(공유 dir) belt 미발동")

    def test_7_empty_source_loud_skip(self):
        self.mock_uname("MINGW64_NT-10.0")
        _, fork = self.mkpair()
        rc, out, err = self.run_fn("", fork, CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(rc, 0)
        self.assertFalse(os.path.exists(os.path.join(fork, ".credentials.json")))
        self.assertIn("유도 실패", err)


class LaunchWiring(Base):
    """배선층 — launch 재사용 경로를 Windows 목으로 완주시켜 호출부(mkdir 직후)와 원천 유도
    (reg account 부재→기본계정 키→resolve_default_base)를 실배선으로 단언한다."""

    NAME = "w1"

    def setUp(self):
        super().setUp()
        self.mock_uname("MINGW64_NT-10.0")
        log = os.path.join(self.tmp, "calls.log")
        # 목 cys: ping 은 CYS_SOCKET 문자열을 파일명으로 보고 실존 판정(named pipe 리터럴을
        # cwd 상대 파일로 재현 — 테스트가 cwd=self.tmp 로 고정·사전 touch = '가동 중' 재사용 경로).
        _write_exec(os.path.join(self.bindir, "cys"),
                    '#!/bin/sh\necho "cys $@" >> "%s"\n'
                    'case "$1" in\n'
                    '  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;\n'
                    '  feed) exit 0 ;;\n  list) exit 0 ;;\n  tombstone) exit 0 ;;\n'
                    'esac\nexit 0\n' % log)
        _write_exec(os.path.join(self.bindir, "cysd"), "#!/bin/sh\nexit 0\n")
        # 재사용 경로: dept_sock(MINGW)=\\.\pipe\cys-dept-w1 리터럴 파일을 cwd(tmp)에 시드.
        open(os.path.join(self.tmp, r"\\.\pipe\cys-dept-%s" % self.NAME), "w").close()
        # base 계정(기본계정 키 default 규약 fallback)·원천 토큰.
        self.base = os.path.join(self.home, ".cys", "claude-default")
        os.makedirs(self.base, exist_ok=True)
        with open(os.path.join(self.base, ".credentials.json"), "w") as f:
            f.write('{"tok":"BASE"}')
        # 레지스트리: 부서 등재 + account_dir(fork) 기록(resolve_lane_acctdir 2순위).
        self.fork = self.base + "-" + self.NAME
        import json
        with open(os.path.join(self.home, ".cys", "depts.json"), "w") as f:
            json.dump({"depts": {self.NAME: {
                "socket": r"\\.\pipe\cys-dept-%s" % self.NAME,
                "pack_dir": os.path.join(self.home, ".cys", "pack-dept-%s" % self.NAME),
                "role": "dept-master", "account_dir": self.fork}}}, f)
        # verify_lane_account_seed → seed_agents_account 원천(base 팩 agents.json).
        packdir = os.path.join(self.home, ".cys", "pack")
        os.makedirs(packdir, exist_ok=True)
        with open(os.path.join(packdir, "agents.json"), "w") as f:
            json.dump({"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": self.base}}}, f)

    def run_launch(self, **envextra):
        return subprocess.run(["bash", DEPT, "launch", self.NAME],
                              capture_output=True, text=True, encoding="utf-8",
                              env=self.env(**envextra), cwd=self.tmp, timeout=60)

    def test_8_launch_optin_seeds_fork(self):
        r = self.run_launch(CYS_DEPT_SEED_CREDS="1")
        self.assertEqual(r.returncode, 0, r.stderr)
        dst = os.path.join(self.fork, ".credentials.json")
        self.assertTrue(os.path.isfile(dst),
                        "launch 배선 미발동(fork 에 사본 부재): %s" % r.stderr)
        self.assertEqual(open(dst).read(), '{"tok":"BASE"}')
        self.assertIn("creds 시드 완료", r.stderr)

    def test_9_launch_default_off_no_seed(self):
        r = self.run_launch()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertFalse(
            os.path.exists(os.path.join(self.fork, ".credentials.json")),
            "opt-in 기본 off 인데 launch 가 시드했다(완전 무동작 위반)")
        self.assertNotIn("creds 시드", r.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
