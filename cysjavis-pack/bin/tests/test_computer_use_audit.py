#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_computer_use_audit.py — javis_computer_use.py(computer-use 탑재 감사) exit 계약 회귀 (B-328 #6 · 2026-10-20).

## 무엇을 못박는가
도구의 사실은 exit code 다(도구 머리말 "결정론 판정"). 실제 CLI 를 서브프로세스로 돌려 각 갈래를 한 번씩 낸다.
  A0 OK          발견된 Claude 좌석 cwd 가 전부 탑재 = 0 · 살아 있는 Claude 좌석 0 = 0(좌석없음)
                 · Claude 가 아닌 좌석(codex·gemini)은 판정 대상이 아니다 = 0
  A1 GAP         미탑재 cwd 가 하나라도 있으면 1 · ★기본값은 감사뿐 — --fix 없이는 .claude.json 바이트가 그대로다
                 · 부서 레인은 그 부서 데몬 소켓(HOME/.local/state/cys-dept-dept-<N>/cys.sock)으로 묻는다
  F  --fix       미탑재 cwd 에 enabledMcpServers 를 메우고 재독 검증 뒤 0 · 기존 항목·다른 키·파일 권한 보존
                 · 임시 파일을 남기지 않는다 · 이미 탑재된 레인은 쓰지 않는다 · Claude 가 아닌 좌석 cwd 는 쓰지 않는다
  U2 판정불가    .claude.json 판독 실패 = 2(--fix 여도 쓰지 않는다) · `cys list` 실패 = 2 · PATH 에 cys 없음 = 2
  C  호출 범위   도구가 띄우는 외부 명령은 `cys list` 하나뿐이다(스텁 호출 기록으로 잰다 — 프로세스 제어·
                 mcp__computer-use 호출 없음)

## 밀폐
- HOME·cwd 는 케이스마다 새 임시 디렉터리다. 라이브 설정(~/.cys/claude*)·라이브 팩·데몬은 읽지도 쓰지도 않는다.
- 자식 PATH 는 스텁 cys(임시 폴더의 sh 스크립트 · 정해 둔 `cys list` 행만 출력) + /usr/bin:/bin 이다.
  부모의 CLAUDE_CONFIG_DIR·CYS_*·타사 프로바이더 env 는 자식에 넘기지 않는다.
- 체크아웃에는 쓰지 않는다.

## 한계
- 스텁 cys 가 sh 스크립트라 Windows 에서는 전부 건너뛴다(사유가 찍힌다). 이 테스트는 비-Windows 레인에만 등록된다.
- `cys list` 의 열 모양은 src/bin/cys.rs Command::List 의 println 형식을 스텁이 흉내 낸 것이다 — 실제 데몬 출력은 재지 않는다.
- 살아 있는 노드 프로세스의 실제 적재 여부는 도구도 이 테스트도 재지 않는다(도구 머리말 "계측의 유효 범위").

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_computer_use_audit.py
"""
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
TOOL = os.path.join(BIN, "javis_computer_use.py")
SRV = "computer-use"
DEPT = "7"

STUB = r"""#!/bin/sh
# 스텁 cys — 호출을 기록하고 정해 둔 `cys list` 행만 낸다. 소켓 env 로 레인을 고른다.
printf '%s|%s\n' "${CYS_SOCKET:-base}" "$*" >> "$RIG_LOG"
if [ -n "$CYS_SOCKET" ]; then key="$CYS_SOCKET"; else key="$RIG_DIR/base"; fi
[ "$1" = "list" ] || exit 64
[ -e "$key.fail" ] && exit 1
[ -f "$key.list" ] && cat "$key.list"
exit 0
"""


def sha(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


@unittest.skipIf(os.name == "nt", "Windows: 스텁 cys 가 sh 스크립트라 건너뛴다(비-Windows 레인 전용)")
class Rig(unittest.TestCase):
    """임시 HOME(base + dept-7 설정) + 스텁 cys — 케이스마다 새로 만든다."""

    def setUp(self):
        self.assertTrue(os.path.isfile(TOOL), "도구 부재: %s" % TOOL)
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="cu-audit-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.rigdir = os.path.join(self.tmp, "rig")
        self.stubbin = os.path.join(self.tmp, "stubbin")
        self.log = os.path.join(self.tmp, "cys-calls.log")
        for d in (self.home, self.rigdir, self.stubbin):
            os.makedirs(d)
        self.base_cfg = os.path.join(self.home, ".cys", "claude", ".claude.json")
        self.dept_cfg = os.path.join(self.home, ".cys", "claude-default-dept-%s" % DEPT, ".claude.json")
        self.dept_sock = os.path.join(self.home, ".local", "state", "cys-dept-dept-%s" % DEPT, "cys.sock")
        os.makedirs(os.path.dirname(self.dept_sock))
        stub = os.path.join(self.stubbin, "cys")
        with open(stub, "w", encoding="utf-8") as fh:
            fh.write(STUB)
        os.chmod(stub, 0o755)
        # 기본: 두 레인 모두 좌석 없음, 설정은 탑재 항목 없이 판독 가능.
        self.write_cfg(self.base_cfg, {})
        self.write_cfg(self.dept_cfg, {})

    # ── 리그 구성 ──
    def write_cfg(self, path, projects, extra=None):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        cfg = {"oauthAccount": {"organizationType": "claude_max"}, "numStartups": 3,
               "projects": projects}
        cfg.update(extra or {})
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(cfg, fh, ensure_ascii=False, indent=2)
        os.chmod(path, 0o644)

    def seats(self, lane, rows):
        """rows = [(sid, title, cwd)] — src/bin/cys.rs Command::List 형식(탭 6열)."""
        key = os.path.join(self.rigdir, "base") if lane == "base" else self.dept_sock
        with open(key + ".list", "w", encoding="utf-8") as fh:
            for sid, title, cwd in rows:
                fh.write("surface:%d\trole=%s\tpid=%d\texited=false\t%s\t%s\n"
                         % (sid, title.split("-")[0], 4000 + sid, title, cwd))

    def fail_list(self, lane):
        key = os.path.join(self.rigdir, "base") if lane == "base" else self.dept_sock
        open(key + ".fail", "w").close()

    def env(self, path=None):
        e = {k: v for k, v in os.environ.items()
             if not k.startswith(("CYS_", "CLAUDE_", "ANTHROPIC_"))}
        e.update({"HOME": self.home, "PATH": path or (self.stubbin + os.pathsep + "/usr/bin:/bin"),
                  "PYTHONUTF8": "1", "RIG_LOG": self.log, "RIG_DIR": self.rigdir})
        return e

    def run_tool(self, *argv, path=None):
        return subprocess.run([sys.executable, "-I", "-B", TOOL] + list(argv), cwd=self.tmp,
                              env=self.env(path), capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=60)

    def rc(self, want, r, what):
        self.assertEqual(r.returncode, want, "%s: rc=%d (기대 %d)\n--stdout--\n%s\n--stderr--\n%s"
                         % (what, r.returncode, want, r.stdout, r.stderr))

    def enabled(self, path, cwd):
        with open(path, encoding="utf-8") as fh:
            cfg = json.load(fh)
        return ((cfg.get("projects") or {}).get(cwd) or {}).get("enabledMcpServers")

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log, encoding="utf-8") as fh:
            return [ln.rstrip("\n") for ln in fh if ln.strip()]


class A0Ok(Rig):
    def test_all_mounted_is_0(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
        self.write_cfg(self.dept_cfg, {"/w/d7": {"enabledMcpServers": ["other", SRV]}})
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        self.seats("dept", [(2, "master-claude · d7", "/w/d7"), (3, "worker-claude · d7", "/w/d7")])
        r = self.run_tool()
        self.rc(0, r, "전부 탑재")
        self.assertIn("cwd 2개 전부 탑재", r.stdout)

    def test_no_live_claude_seat_is_0(self):
        r = self.run_tool()
        self.rc(0, r, "좌석 0")
        self.assertEqual(r.stdout.count("좌석없음"), 2, r.stdout)

    def test_non_claude_seats_are_not_judged(self):
        self.seats("base", [(4, "worker-codex · x", "/w/codex"), (5, "reviewer-gemini · y", "/w/gem")])
        r = self.run_tool()
        self.rc(0, r, "Claude 아닌 좌석만")
        self.assertNotIn("/w/codex", r.stdout)


class A1Gap(Rig):
    def test_gap_is_1_and_audit_writes_nothing(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": ["other"]}})
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        before = (sha(self.base_cfg), sha(self.dept_cfg))
        r = self.run_tool()
        self.rc(1, r, "미탑재 1")
        self.assertIn("미탑재", r.stdout)
        self.assertIn("/w/base", r.stdout)
        self.assertEqual((sha(self.base_cfg), sha(self.dept_cfg)), before, "감사(무 --fix)가 설정을 썼다")

    def test_dept_lane_asks_its_own_socket(self):
        self.seats("dept", [(9, "worker-claude · d7", "/w/d7")])
        r = self.run_tool()
        self.rc(1, r, "부서 레인 미탑재")
        self.assertIn("dept-%s" % DEPT, r.stdout)
        self.assertIn("%s|list" % self.dept_sock, self.calls())

    def test_missing_projects_entry_is_a_gap(self):
        self.write_cfg(self.base_cfg, {"/elsewhere": {"enabledMcpServers": [SRV]}})
        self.seats("base", [(1, "worker-claude · new", "/w/new")])
        self.rc(1, self.run_tool(), "새 작업 폴더")


class FFix(Rig):
    def test_fix_fills_gap_and_preserves(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": ["other"], "allowedTools": ["x"]}},
                       extra={"keep": {"a": 1}})
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        self.seats("dept", [(2, "worker-claude · d7", "/w/d7")])
        r = self.run_tool("--fix")
        self.rc(0, r, "--fix 뒤")
        self.assertEqual(self.enabled(self.base_cfg, "/w/base"), ["other", SRV])
        self.assertEqual(self.enabled(self.dept_cfg, "/w/d7"), [SRV])
        with open(self.base_cfg, encoding="utf-8") as fh:
            cfg = json.load(fh)
        self.assertEqual(cfg["keep"], {"a": 1})
        self.assertEqual(cfg["projects"]["/w/base"]["allowedTools"], ["x"])
        self.assertEqual(cfg["oauthAccount"], {"organizationType": "claude_max"})
        for p in (self.base_cfg, self.dept_cfg):
            self.assertEqual(stat.S_IMODE(os.stat(p).st_mode), 0o644, "권한 보존 실패: %s" % p)
            left = [n for n in os.listdir(os.path.dirname(p)) if n.startswith(".claude.json.cu-")]
            self.assertEqual(left, [], "임시 파일 잔존")
        self.rc(0, self.run_tool(), "--fix 뒤 재감사")

    def test_fix_leaves_mounted_lane_and_non_claude_cwd_untouched(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        self.seats("dept", [(2, "worker-claude · d7", "/w/d7"), (3, "worker-codex · c", "/w/codex")])
        base_before = sha(self.base_cfg)
        self.rc(0, self.run_tool("--fix"), "--fix")
        self.assertEqual(sha(self.base_cfg), base_before, "이미 탑재된 레인을 다시 썼다")
        self.assertIsNone(self.enabled(self.dept_cfg, "/w/codex"), "Claude 아닌 좌석 cwd 를 썼다")


class U2Unreadable(Rig):
    def test_unreadable_config_is_2_even_with_fix(self):
        with open(self.base_cfg, "w", encoding="utf-8") as fh:
            fh.write("{not json")
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        before = sha(self.base_cfg)
        r = self.run_tool("--fix")
        self.rc(2, r, "판독 불가 설정")
        self.assertIn("판정불가", r.stdout)
        self.assertEqual(sha(self.base_cfg), before)

    def test_cys_list_failure_is_2(self):
        self.fail_list("dept")
        self.rc(2, self.run_tool(), "cys list 실패")

    def test_no_cys_on_path_is_2(self):
        nobin = os.path.join(self.tmp, "nobin")
        os.makedirs(nobin)
        self.assertIsNone(shutil.which("cys", path=nobin))
        self.rc(2, self.run_tool(path=nobin), "cys 부재")


class CCalls(Rig):
    def test_only_cys_list_is_spawned(self):
        self.seats("base", [(1, "worker-claude · base", "/w/base")])
        self.seats("dept", [(2, "worker-claude · d7", "/w/d7")])
        self.run_tool("--fix")
        got = self.calls()
        self.assertEqual(sorted(got), sorted(["base|list", "%s|list" % self.dept_sock]), got)


if __name__ == "__main__":
    unittest.main(verbosity=2)
