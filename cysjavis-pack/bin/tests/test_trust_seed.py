#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_trust_seed.py — ★WP-2(0.14.31 · 감사 2026-09-06 에러4 원천봉쇄) 폴더 신뢰 사전 주입 핀.

`javis_preflight.py --seed-trust --config <acctdir> --cwd <cwd>` 와 C58 스코프(레지스트리 쌍 판정)·cys-dept 3지점
배선을 test_dept_creds_seed.py 동형(격리 HOME · 실물 추출 · 배선 완주)으로 단언한다. 라이브 무접촉: 모든 config dir·
cwd·레지스트리는 임시 디렉터리다(실 ~/.cys 계정 dir 은 절대 대상이 아니다 — 절대경로 인자로만 동작).

  1) 부재: .claude.json 없음 → 정확히 {"projects":{<key>:{"hasTrustDialogAccepted":true}}} 생성 · 0600 · 2회차 멱등(무쓰기)
  2) 부분: 다른 최상위 키·다른 항목·hasCompletedOnboarding=false 보존 · projects 부재 생성 · ★R1 정확 키 정책(별칭 키 무접촉 ·
     별칭 true 는 '이미 신뢰' 아님) · 손상 JSON/심링크 = ERROR 무쓰기 · 0B 기존 파일 = 존재(권한 보존 · 0B 백업)
  3) 라이브 프로세스: 그 CLAUDE_CONFIG_DIR 로 도는 claude 실행 형상 존재 → REFUSE(rc 2) 무쓰기 · 종료 후 OK ·
     같은 env 의 비-claude(python) 자식은 계수 0 · 검증 불가(None) → REFUSE, --force-unverified 만 통과 · ★R1 이미 신뢰면
     프로브 0(가동 중 부서 재사용 경로 WARN 0) · env 비노출 claude 형상 → None
  4) 동시 변경: 다른 프로세스가 잠금 보유 → REFUSE lock-busy · 읽기~쓰기 사이 파일 변경 → REFUSE concurrent-change
     (상대 내용 생존) · 잠금 기구 미가용 → REFUSE lock-unavailable · ★R1 교체 後 기록자 = 롤백 0(상대 내용 보존 · 플래그 보존이면
     OK · 아니면 REFUSE post-commit · 되읽기 실패 = ERROR 커밋 유지) · 알려진 한계 핀(대조~교체 사이 창)
  5) C58 레지스트리: 본부·부서 topology 쌍 판독(config 부재 항목 추정 귀속 0 · ★R1 agent=claude 만 · depts.json (account_dir,
     cwd) 쌍 · 절대경로만) · _is_cysjavis_workspace 가 CLAUDE.md/_round 없이 쌍 판정 · 갭 = 항목 부재·별칭 true 포함 ·
     report 모드 무쓰기 · --fix 는 seed_trust 경로(.bak-preflight 1회) · ★R1 쌍 0 → SKIP(PASS 아님)
  5') ★R1 격리 컨텍스트(실물 _discover_isolation_block): 부서 컨텍스트 = 자기 계정 config 만 판정·수리(타 계정 무접촉 스파이) ·
     계정 미상 부서/임시 팩 = SKIP · Windows state 경로(%LOCALAPPDATA%\\cys\\<pipe_slug> · LOCALAPPDATA 부재 = 판독불가 고지)
  6) cys-dept 배선: 3지점(launch/allocate/create)에서 seed_trust_acct 가 데몬 스폰·빈 셸 生成 앞 · 4 데몬 라인 env -u 접두 ·
     ★R1 resolve_dept_cwd 로 확정한 **같은 cwd** 가 시드·빈 셸·formation_ensure_async(--cwd) 에 전달 · launch 완주(Windows 목·
     재사용 경로)에서 fork 계정 dir 에 .claude.json 착지 + formation 스텁 argv 실측(호출자 cwd≠HOME · CYS_DEPT_CWD · 등재 cwd · "/")
  7) codex(gpt-6-astra) 적대 반례(R2 · 워커가 전 행 검토 후 채택) + R1 재검토분

    CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" CYS_PROBE_RUNS="$JAVIS_ROOT/probe_runs.jsonl" \\
      python3 cysjavis-pack/bin/tests/test_trust_seed.py
"""
import builtins
import copy
import io
import json
import os
import re
import shlex
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PF = os.path.join(BIN, "javis_preflight.py")
DEPT = os.path.join(BIN, "cys-dept")
sys.path.insert(0, BIN)
import javis_preflight as pf  # noqa: E402

PY = sys.executable or "python3"
# 격리 env 키 — HOME 만으로는 Windows python 의 expanduser("~")(USERPROFILE) 가 안 바뀐다(codex R1) → 함께 잡는다.
_HOME_KEYS = ("HOME", "USERPROFILE")
_ISO_KEYS = ("CYS_DEPTS_JSON", "CYS_ACCOUNT_DIR", "CLAUDE_CONFIG_DIR", "CYS_PACK_DIR", "CYS_SOCKET",
             "LOCALAPPDATA", "XDG_STATE_HOME")


def _write(path, text, mode=None):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    if mode is not None:
        os.chmod(path, mode)


def _read_json(path):
    with open(path, "rb") as f:
        return json.loads(f.read().decode("utf-8"))


def _read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def _read_text(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def seed_cli(config, cwd, *extra, env=None):
    """subprocess 실물 호출 — (rc, stdout, stderr)."""
    e = dict(os.environ)
    e.pop("CLAUDE_CONFIG_DIR", None)
    if env:
        e.update(env)
    r = subprocess.run([PY, PF, "--seed-trust", "--config", config, "--cwd", cwd, *extra],
                       capture_output=True, text=True, encoding="utf-8", env=e, timeout=60)
    return r.returncode, r.stdout, r.stderr


def _no_probe(d):
    raise AssertionError("이미 신뢰 경로에서 프로세스 프로브가 호출됐다(R1 위반)")


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustseed-")
        self.cfg = os.path.join(self.tmp, "acct")
        self.ws = os.path.join(self.tmp, "ws")
        os.makedirs(self.ws)
        self.key = pf.claude_project_key(self.ws)
        self.cfgfile = os.path.join(self.cfg, ".claude.json")


class Absent(Base):
    def test_1_absent_creates_minimal_then_idempotent(self):
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, err)
        self.assertTrue(out.startswith("seed-trust: OK seeded("), out)
        self.assertEqual(_read_json(self.cfgfile),
                         {"projects": {self.key: {"hasTrustDialogAccepted": True}}},
                         "부재 생성 문서가 최소 형태가 아니다(다른 키 주입 금지)")
        self.assertEqual(stat.S_IMODE(os.stat(self.cfgfile).st_mode), 0o600)
        raw1 = _read_bytes(self.cfgfile)
        mtime1 = os.stat(self.cfgfile).st_mtime_ns
        time.sleep(0.02)
        rc, out, err = seed_cli(self.cfg, self.ws, "--json")
        self.assertEqual(rc, 0, err)
        j = json.loads(out)
        self.assertEqual(j["verdict"], "OK")
        self.assertTrue(j["reason"].startswith("already-trusted("), j)
        self.assertEqual(_read_bytes(self.cfgfile), raw1, "멱등 2회차가 파일을 다시 썼다")
        self.assertEqual(os.stat(self.cfgfile).st_mtime_ns, mtime1, "멱등 2회차가 mtime 을 바꿨다(무쓰기 위반)")

    def test_1b_usage_errors_rc1_no_write(self):
        rc, out, err = seed_cli(self.cfg, "relative/ws")
        self.assertEqual(rc, 1)
        self.assertIn("ERROR", out)
        self.assertFalse(os.path.exists(self.cfgfile))
        r = subprocess.run([PY, PF, "--seed-trust", "--config", self.cfg],
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 1, "usage 오류는 rc 1(REFUSE=2 와 충돌 금지)")


class Partial(Base):
    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)

    def test_2_partial_only_target_key_touched(self):
        base = {"hasCompletedOnboarding": False, "theme": "dark", "numStartups": 3,
                "projects": {"/somewhere/else": {"hasTrustDialogAccepted": False, "allowedTools": []}}}
        _write(self.cfgfile, json.dumps(base, indent=2), 0o644)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, err)
        got = _read_json(self.cfgfile)
        self.assertIs(got["hasCompletedOnboarding"], False, "hasCompletedOnboarding 이 건드려졌다")
        self.assertEqual(got["theme"], "dark")
        self.assertEqual(got["numStartups"], 3)
        self.assertEqual(got["projects"]["/somewhere/else"],
                         {"hasTrustDialogAccepted": False, "allowedTools": []}, "다른 항목이 변경됐다")
        self.assertEqual(got["projects"][self.key], {"hasTrustDialogAccepted": True})
        self.assertEqual(set(got), set(base), "최상위 키 집합이 변했다")
        self.assertEqual(stat.S_IMODE(os.stat(self.cfgfile).st_mode), 0o644, "기존 파일 권한 미보존")
        self.assertFalse(os.path.exists(self.cfgfile + ".bak-preflight"), "시드 경로는 백업을 만들지 않는다")

    def test_2b_projects_absent_created(self):
        _write(self.cfgfile, '{"hasCompletedOnboarding": true}')
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, err)
        self.assertEqual(_read_json(self.cfgfile),
                         {"hasCompletedOnboarding": True,
                          "projects": {self.key: {"hasTrustDialogAccepted": True}}})

    def test_2c_alias_key_untouched_exact_key_created(self):
        """★R1(codex): claude 는 projects[getcwd()] 정확 키만 읽는다 — 별칭(꼬리 슬래시) 항목은 신뢰 판정에 쓰지도 손대지도 않는다."""
        alias = self.key + "/"
        _write(self.cfgfile, json.dumps({"projects": {alias: {"hasTrustDialogAccepted": False, "k": 1}}}))
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, err)
        got = _read_json(self.cfgfile)
        self.assertEqual(got["projects"][alias], {"hasTrustDialogAccepted": False, "k": 1}, "별칭 항목이 변경됐다")
        self.assertEqual(got["projects"][self.key], {"hasTrustDialogAccepted": True}, "정확 키가 생성되지 않았다")

    def test_2c2_alias_true_exact_false_conflict_sets_exact(self):
        """codex R1 반례: {"/work/": true, "/work": false} — 종전 구현은 별칭을 골라 already-trusted 로 정확 키를 false 로 남겼다."""
        alias = self.key + "/"
        _write(self.cfgfile, json.dumps({"projects": {alias: {"hasTrustDialogAccepted": True},
                                                      self.key: {"hasTrustDialogAccepted": False}}}))
        # 정확 키 false = 신뢰 아님 → 프로브가 돈다(양성이면 REFUSE)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (1, "live"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("live-claude", reason)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("seeded(", reason)
        got = _read_json(self.cfgfile)
        self.assertIs(got["projects"][self.key]["hasTrustDialogAccepted"], True)
        self.assertEqual(got["projects"][alias], {"hasTrustDialogAccepted": True})
        # 별칭만 true · 정확 키 부재 → 역시 '이미 신뢰' 아님
        _write(self.cfgfile, json.dumps({"projects": {alias: {"hasTrustDialogAccepted": True}}}))
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (1, "live"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)

    def test_2d_corrupt_json_error_untouched(self):
        _write(self.cfgfile, '{"projects": {')
        raw = _read_bytes(self.cfgfile)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 1)
        self.assertIn("파싱 실패", out)
        self.assertEqual(_read_bytes(self.cfgfile), raw, "손상 파일을 건드렸다")

    def test_2e_symlink_refused(self):
        target = os.path.join(self.tmp, "real.json")
        _write(target, '{"projects": {}}')
        os.symlink(target, self.cfgfile)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 1)
        self.assertIn("symlink", out)
        self.assertEqual(_read_bytes(target), b'{"projects": {}}')
        self.assertTrue(os.path.islink(self.cfgfile))

    def test_2f_pristine_refused(self):
        pris = os.path.join(self.tmp, "pack", ".pristine", "claude")
        rc, out, err = seed_cli(pris, self.ws)
        self.assertEqual(rc, 1)
        self.assertIn(".pristine", out)
        self.assertFalse(os.path.exists(pris))

    def test_2g_empty_existing_file_is_existing(self):
        """codex R1: 0B 기존 파일은 '부재' 가 아니다 — 비기본 권한 보존 · backup=True 면 0B 백업 · 삭제 0."""
        _write(self.cfgfile, "", 0o640)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"), backup=True)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        self.assertEqual(stat.S_IMODE(os.stat(self.cfgfile).st_mode), 0o640, "0B 기존 파일의 권한이 보존되지 않았다")
        bak = self.cfgfile + ".bak-preflight"
        self.assertTrue(os.path.isfile(bak), "0B 기존 파일 백업이 없다(부재 취급)")
        self.assertEqual(_read_bytes(bak), b"")
        self.assertEqual(stat.S_IMODE(os.stat(bak).st_mode), 0o640)


def _env_visible_for_python_child():
    """darwin `ps -E` 는 Apple 플랫폼 바이너리(/bin/sh·sleep)의 env 를 숨긴다 — 픽스처는 사용자 python 으로 띄우되,
    그 python 조차 env 가 안 보이는 환경(Apple 제공 python 등)이면 라이브 케이스는 skip(정직)."""
    if sys.platform.startswith("linux"):
        return True
    if sys.platform != "darwin":
        return False
    probe = subprocess.Popen([PY, "-c", "import time; time.sleep(20)"],
                             env=dict(os.environ, CLAUDE_CONFIG_DIR="/probe/visible"))
    try:
        time.sleep(0.3)
        rc, out, _e = pf._run_capture(["ps", "-ax", "-ww", "-E", "-o", "pid=,command="])
        return rc == 0 and any(l.split(None, 1)[0] == str(probe.pid) and "CLAUDE_CONFIG_DIR=/probe/visible" in l
                               for l in out.splitlines() if l.strip())
    finally:
        probe.kill()
        probe.wait()


class LiveProcess(Base):
    def _spawn(self, script_name, env_cfg):
        path = os.path.join(self.tmp, script_name)
        _write(path, "import time\ntime.sleep(30)\n")
        p = subprocess.Popen([PY, path], env=dict(os.environ, CLAUDE_CONFIG_DIR=env_cfg))
        self.addCleanup(lambda: (p.kill(), p.wait()))
        time.sleep(0.4)
        return p

    def test_3_live_claude_refuses_then_ok_after_exit(self):
        if not _env_visible_for_python_child():
            self.skipTest("이 플랫폼/인터프리터에선 자식 env 가 ps 에 보이지 않는다(라이브 판정 실측 불가)")
        p = self._spawn("claude", self.cfg)          # argv basename 'claude' = claude 실행 형상
        os.makedirs(self.cfg, exist_ok=True)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 2, "라이브 claude 가 있는데 거부하지 않았다: %s %s" % (out, err))
        self.assertIn("REFUSE live-claude(n=1", out)
        self.assertFalse(os.path.exists(self.cfgfile), "거부인데 파일을 썼다")
        # 타 config 의 claude 는 계수 대상이 아니다(쌍 스코프)
        rc2, out2, _ = seed_cli(os.path.join(self.tmp, "acct2"), self.ws)
        self.assertEqual(rc2, 0, out2)
        p.kill()
        p.wait()
        time.sleep(0.2)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, out + err)
        self.assertIn("OK seeded(", out)

    def test_3b_non_claude_child_with_same_env_not_counted(self):
        if not _env_visible_for_python_child():
            self.skipTest("자식 env 가 ps 에 보이지 않는다")
        self._spawn("mcp_server.py", self.cfg)       # 같은 env · claude 실행 형상 아님(MCP 자식 재현)
        count, detail = pf.claude_procs_for_config(self.cfg)
        self.assertEqual(count, 0, detail)
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, out + err)

    def test_3c_unverified_refuses_unless_forced(self):
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (None, "no-ps"))
        self.assertEqual((rc, verdict), (2, "REFUSE"))
        self.assertIn("unverified(no-ps)", reason)
        self.assertFalse(os.path.exists(self.cfgfile), "검증 불가 거부인데 파일을 썼다")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, force_unverified=True,
                                            proc_counter=lambda d: (None, "no-ps"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("force-unverified(no-ps)", reason, "강행 사실이 사유에 남지 않았다")
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        rc, out, err = seed_cli(self.cfg, self.ws, "--force-unverified", "--json")
        self.assertEqual(rc, 0, err)
        self.assertEqual(json.loads(out)["verdict"], "OK")

    def test_3d_force_unverified_does_not_bypass_live_claude(self):
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, force_unverified=True,
                                            proc_counter=lambda d: (2, "fake"))
        self.assertEqual((rc, verdict), (2, "REFUSE"))
        self.assertIn("live-claude(n=2", reason)
        self.assertFalse(os.path.exists(self.cfgfile))

    def test_3e_already_trusted_skips_probe_and_write(self):
        """★R1: 정확 키 true 면 프로세스 프로브 0(라이브 dept 의 rotate/launch 재사용 경로가 매번 REFUSE WARN 을 내던 것) · 무쓰기."""
        os.makedirs(self.cfg)
        alias = self.key + "/"
        _write(self.cfgfile, json.dumps({"projects": {self.key: {"hasTrustDialogAccepted": True},
                                                      alias: {"hasTrustDialogAccepted": False}}}), 0o600)
        raw, mt = _read_bytes(self.cfgfile), os.stat(self.cfgfile).st_mtime_ns
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("already-trusted", reason)
        self.assertEqual((_read_bytes(self.cfgfile), os.stat(self.cfgfile).st_mtime_ns), (raw, mt))
        # CLI 도 같은 경로(ps 를 부르지 않는다 — PATH 에서 ps 를 지워도 OK)
        rc, out, err = seed_cli(self.cfg, self.ws, env={"PATH": "/nonexistent"})
        self.assertEqual(rc, 0, out + err)
        self.assertIn("already-trusted", out)

    def test_3f_hidden_env_claude_shape_is_unresolved(self):
        """codex R1: env 세그먼트 없는 claude 형상 줄은 '검증된 0' 이 아니다 → None → REFUSE unverified(강행 시 통과) · 양성이 있으면 n."""
        runner = lambda c: (0, "  7 /Users/o/.local/bin/claude --continue\n  8 python3 x.py\n", "")
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=runner, os_name="posix", platform="darwin")
        self.assertIsNone(cnt, detail)
        self.assertIn("env 비노출", detail)
        counter = lambda d: pf.claude_procs_for_config(d, runner=runner, os_name="posix", platform="darwin")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=counter)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("unverified", reason)
        pos = lambda c: (0, "  7 /Users/o/.local/bin/claude\n  9 /Users/o/.local/bin/claude CLAUDE_CONFIG_DIR=%s\n" % self.cfg, "")
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=pos, os_name="posix", platform="darwin")
        self.assertEqual(cnt, 1, detail)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, force_unverified=True, proc_counter=lambda d: (1, "x"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), "양성 관측을 강행이 넘었다")

    def test_3g_linux_procfs_permission_same_uid_is_unresolved(self):
        """codex R1: /proc/<pid>/environ EACCES 인데 같은 uid → 미해결(None) · 다른 uid → 범위 외 · 양성이 있으면 n 우선."""
        root = os.path.join(self.tmp, "proc")
        os.makedirs(os.path.join(root, "100"))
        os.makedirs(os.path.join(root, "200"))
        for pid, cfgv in (("100", self.cfg), ("200", "/other")):
            with open(os.path.join(root, pid, "environ"), "wb") as f:
                f.write(b"CLAUDE_CONFIG_DIR=" + cfgv.encode() + b"\0HOME=/x\0")
            with open(os.path.join(root, pid, "cmdline"), "wb") as f:
                f.write(b"/usr/bin/claude\0--continue\0")
        me = os.getuid() if hasattr(os, "getuid") else 0
        real_open = builtins.open

        def denied_open(path, *a, **k):
            if path.endswith(os.path.join("100", "environ")):
                raise PermissionError("denied")
            return real_open(path, *a, **k)

        with patch("builtins.open", denied_open), patch.object(pf.os, "stat", lambda p, **k: type("S", (), {"st_uid": me})()):
            cnt, detail = pf._count_claude_procfs(self.cfg, root)
        self.assertIsNone(cnt, detail)
        self.assertIn("미해결", detail)
        with patch("builtins.open", denied_open), patch.object(pf.os, "stat", lambda p, **k: type("S", (), {"st_uid": me + 1})()):
            cnt, detail = pf._count_claude_procfs(self.cfg, root)
        self.assertEqual(cnt, 0, detail)
        cnt, detail = pf._count_claude_procfs(self.cfg, root)
        self.assertEqual(cnt, 1, detail)


class Concurrent(Base):
    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)

    def test_4_lock_held_by_other_process_refuses(self):
        lock_path = os.path.join(self.cfg, pf.SEED_TRUST_LOCK_NAME)
        holder = open(lock_path, "a+")
        self.addCleanup(holder.close)
        self.assertIs(pf._try_lock_nb(holder), True, "테스트가 잠금을 잡지 못했다(하네스 결함)")
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 2, out + err)
        self.assertIn("REFUSE lock-busy", out)
        self.assertFalse(os.path.exists(self.cfgfile), "잠금 경합 거부인데 파일을 썼다")
        holder.close()
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, out + err)

    def test_4b_concurrent_change_between_read_and_write_refused(self):
        _write(self.cfgfile, '{"projects": {}}')
        other = '{"projects": {"/live/session": {"hasTrustDialogAccepted": true}}, "liveWrote": 1}'

        def racer():
            _write(self.cfgfile, other)   # 라이브 세션이 읽기 직후 파일을 갱신한 상황

        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                            _pre_write_hook=racer)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("concurrent-change", reason)
        self.assertEqual(_read_text(self.cfgfile), other, "상대의 쓰기가 clobber 됐다")
        leftovers = [n for n in os.listdir(self.cfg) if n.startswith(".claude.json.seed-") and n != pf.SEED_TRUST_LOCK_NAME]
        self.assertEqual(leftovers, [], "임시 파일 잔재: %s" % leftovers)
        # 재시도는 상대 내용을 보존하며 한 키만 더한다
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual(rc, 0, reason)
        got = _read_json(self.cfgfile)
        self.assertEqual(got["liveWrote"], 1)
        self.assertEqual(got["projects"]["/live/session"], {"hasTrustDialogAccepted": True})
        self.assertEqual(got["projects"][self.key], {"hasTrustDialogAccepted": True})

    def test_4b2_existence_transitions_are_changes(self):
        """codex R1: 부재→0B 생성 · 기존→삭제 도 '변경'(바이트만 비교하면 둘 다 b'' 로 같아 보인다)."""
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                            _pre_write_hook=lambda: _write(self.cfgfile, ""))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("concurrent-change", reason)
        self.assertEqual(_read_bytes(self.cfgfile), b"", "상대가 만든 0B 파일이 덮였다")
        _write(self.cfgfile, "")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                            _pre_write_hook=lambda: os.unlink(self.cfgfile))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertFalse(os.path.exists(self.cfgfile), "상대가 지운 파일을 되살렸다")

    def test_4c_lock_facility_unavailable_refused(self):
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                            lock_fn=lambda f: None)
        self.assertEqual((rc, verdict), (2, "REFUSE"))
        self.assertIn("lock-unavailable", reason)
        self.assertFalse(os.path.exists(self.cfgfile))

    def test_4d_tmp_cleanup_on_write_failure(self):
        # 교체 직전 심링크 스왑 감지 경로: 임시파일 잔재 0 · 원본 무접촉
        real = os.path.join(self.tmp, "real.json")
        _write(real, '{"projects": {}}')
        _write(self.cfgfile, '{"projects": {}}')

        def swap():
            os.unlink(self.cfgfile)
            os.symlink(real, self.cfgfile)

        # hook 는 재읽기 대조 앞에서 불린다 → 내용 동일하면 대조 통과 → 교체 직전 islink 재검이 잡아야 한다
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                            _pre_write_hook=swap)
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("symlink", reason)
        self.assertEqual(_read_text(real), '{"projects": {}}', "심링크 타깃이 덮였다")
        leftovers = [n for n in os.listdir(self.cfg) if n.startswith(".claude.json.seed-") and n != pf.SEED_TRUST_LOCK_NAME]
        self.assertEqual(leftovers, [])

    def _seed_with_post_replace_writer(self, payload):
        """os.replace 실물 뒤에 다른 기록자가 payload 를 쓰는 상황(교체~되읽기 사이)."""
        real_replace = os.replace

        def replace(src, dst):
            real_replace(src, dst)
            with open(dst, "wb") as f:
                f.write(payload)

        with patch.object(pf.os, "replace", replace):
            return pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))

    def test_4e_post_commit_writer_preserved_no_rollback(self):
        """★R1(codex): 교체 後 기록자 내용은 **보존**(롤백 0). 플래그 보존 → OK · 없음 → REFUSE post-commit · 판독 불가 → ERROR 커밋 유지."""
        _write(self.cfgfile, '{"theme": "dark", "projects": {}}')
        kept = json.dumps({"theme": "light", "projects": {self.key: {"hasTrustDialogAccepted": True, "z": 1}}}).encode()
        rc, verdict, reason = self._seed_with_post_replace_writer(kept)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("플래그 보존", reason)
        self.assertEqual(_read_bytes(self.cfgfile), kept, "다른 기록자의 내용이 롤백/덮임")
        dropped = json.dumps({"theme": "light", "projects": {"/x": {"hasTrustDialogAccepted": True}}}).encode()
        _write(self.cfgfile, '{"theme": "dark", "projects": {}}')
        rc, verdict, reason = self._seed_with_post_replace_writer(dropped)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("post-commit", reason)
        self.assertEqual(_read_bytes(self.cfgfile), dropped, "다른 기록자의 내용이 롤백됐다(데이터 파괴)")
        for weird in (b'{"projects": [1]}', b'{"projects": {"%s": 5}}' % self.key.encode(), b"[]"):
            with self.subTest(weird=weird):
                _write(self.cfgfile, dropped.decode())
                rc, verdict, reason = self._seed_with_post_replace_writer(weird)
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)   # 유효 JSON · 형상 이상 → 예외 0 · 보존
                self.assertEqual(_read_bytes(self.cfgfile), weird)
        _write(self.cfgfile, dropped.decode())
        rc, verdict, reason = self._seed_with_post_replace_writer(b"{broken")
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("커밋 상태로 둔다", reason)
        self.assertEqual(_read_bytes(self.cfgfile), b"{broken", "판독 불가 내용을 롤백했다(진행 중 기록자 파괴 가능)")

    def test_4f_readback_io_error_leaves_commit(self):
        """R2 반례 재검토(R1): 되읽기 IO 실패는 롤백이 아니라 ERROR + 커밋 유지(임시파일 검증 통과분)."""
        _write(self.cfgfile, '{"theme": "dark"}')
        real = pf._read_claude_json_bytes
        calls = []

        def flaky(path):
            calls.append(path)
            if len(calls) == 3:          # 초기 읽기 · 교체 직전 대조 · 되읽기(3번째) 만 실패
                raise OSError("injected readback failure")
            return real(path)

        with patch.object(pf, "_read_claude_json_bytes", flaky):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("커밋 상태로 둔다", reason)
        self.assertEqual(_read_json(self.cfgfile), {"theme": "dark", "projects": {self.key: {"hasTrustDialogAccepted": True}}})
        self.assertNotIn("롤백:", reason)

    def test_4g_known_limit_writer_between_compare_and_replace(self):
        """정직한 한계 핀(codex R1): 대조~os.replace 사이의 기록자는 이 프로토콜이 잡지 못한다(advisory 잠금 · 비협조 기록자).
        결과는 '그 기록자의 1회 쓰기가 우리 문서로 덮임' — 손상·부분 쓰기는 아니며 우리 문서는 그 직전 내용 + 플래그 1개다.
        이 창을 닫으려면 기록자(claude) 협조가 필요하다 — 닫히면 이 핀을 뒤집어라."""
        _write(self.cfgfile, '{"projects": {}}')
        real_replace = os.replace
        late = b'{"projects": {"/late": {"hasTrustDialogAccepted": true}}}'

        def replace(src, dst):
            with open(dst, "wb") as f:
                f.write(late)
            real_replace(src, dst)

        with patch.object(pf.os, "replace", replace):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})


class _IsoEnv(unittest.TestCase):
    """격리 HOME/USERPROFILE + 레지스트리·state 관련 env 전부 격리(codex R1: HOME 만으론 부족)."""

    def _isolate(self, home):
        self._saved = {k: os.environ.get(k) for k in _HOME_KEYS + _ISO_KEYS}
        for k in _HOME_KEYS:
            os.environ[k] = home
        for k in _ISO_KEYS:
            os.environ.pop(k, None)
        os.environ["CYS_DEPTS_JSON"] = os.path.join(home, ".cys", "depts.json")

    def _restore(self):
        for k, v in self._saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    @staticmethod
    def _pf(fix):
        return pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report", allow_irreversible=False)

    @staticmethod
    def _c58(p):
        p.c58_trust_harden()
        r = [x for x in p.results if x["id"] == "C58.trust-harden"]
        assert len(r) == 1, r
        return r[0]


class RegistryC58(_IsoEnv):
    """C58 스코프 — 레지스트리 쌍 판정(임시 HOME · 본부 컨텍스트). 격리 차단은 판독기에 한정 monkeypatch: 임시 HOME 은 tempdir
    아래라 실물 _discover_isolation_block 이 '임시 팩' 으로 접는다(부서 컨텍스트 실물 검증은 DeptContextC58)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustc58-")
        self.home = os.path.join(self.tmp, "home")
        self._isolate(self.home)
        self._iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)
        self.cfgA = os.path.join(self.home, ".cys", "claude")
        self.cfgB = os.path.join(self.home, ".cys", "claude-default-dept-1")
        self.cfgC = os.path.join(self.home, ".cys", "claude-default-dept-2")
        self.X = os.path.join(self.home, "wsX")
        self.Y = os.path.join(self.home, "wsY")
        self.X2 = os.path.join(self.home, "wsX2")
        self.X3 = os.path.join(self.home, "wsX3-codex")
        self.X4 = os.path.join(self.home, "wsX4-nullshell")
        self.Z = os.path.join(self.home, "wsZ-registered")
        self.stale = os.path.join(self.home, "gone")
        for d in (self.cfgA, self.cfgB, self.cfgC, self.X, self.Y, self.X2, self.X3, self.X4, self.Z,
                  os.path.join(self.home, ".local", "state", "cys"),
                  os.path.join(self.home, ".local", "state", "cys-dept-dept-1"),
                  os.path.join(self.home, ".cys", "state")):
            os.makedirs(d, exist_ok=True)
        _write(os.path.join(self.home, ".local", "state", "cys", "topology.json"), json.dumps({
            "entries": [
                {"role": "master", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": self.X},
                {"role": "worker", "agent": "claude", "cwd": self.Y},          # config 부재 → 추정 귀속 금지
                {"role": "cso", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": self.stale},
                {"role": "reviewer-gemini", "agent": "gemini", "claude_config_dir": self.cfgA, "cwd": self.X3},
                {"role": "worker-r", "agent": "claude", "claude_config_dir": "rel/cfg", "cwd": "rel/ws"},   # 상대경로 → 제외
            ]}))
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {
            "dept-1": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "cys.sock"),
                       "pack_dir": os.path.join(self.home, ".cys", "pack-dept-dept-1"), "role": "dept-master",
                       "account_dir": self.cfgB},
            "dept-2": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-2", "cys.sock"),
                       "pack_dir": os.path.join(self.home, ".cys", "pack-dept-dept-2"), "role": "dept-master",
                       "account_dir": self.cfgC, "cwd": self.Z}}}))            # topology 없음 · 등재 cwd 만(create 기록)
        _write(os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "topology.json"), json.dumps({
            "entries": [
                {"role": "worker", "agent": "claude", "claude_config_dir": self.cfgB, "cwd": self.Y},
                {"role": "worker-2", "agent": "claude", "claude_config_dir": self.cfgB, "cwd": self.X2},
                {"role": "reviewer-codex", "agent": "codex", "claude_config_dir": self.cfgB, "cwd": self.X3},
                {"role": "master", "agent": None, "claude_config_dir": self.cfgB, "cwd": self.X4},
                {"role": "legacy", "claude_config_dir": self.cfgB, "cwd": self.X4},
            ]}))
        _write(os.path.join(self.home, ".cys", "state", "mission.json"),
               json.dumps({"schema": 1, "mission": "x", "surface": "109"}))   # cwd 없음(0.14.30 스키마)

    def tearDown(self):
        pf._discover_isolation_block = self._iso
        self._restore()

    def test_5_registry_pairs_precise(self):
        reg = pf.cysjavis_registry()
        I = pf._path_identity
        self.assertEqual(set(reg["pairs"][I(self.cfgA)]), {I(self.X), I(self.stale)},
                         "본부: gemini 좌석 cwd/상대경로 항목이 쌍으로 들어갔거나 claude 쌍이 빠졌다")
        self.assertEqual(set(reg["pairs"][I(self.cfgB)]), {I(self.Y), I(self.X2)},
                         "부서: codex/null/키부재 항목이 쌍으로 들어갔다(agent=claude 만)")
        self.assertEqual(reg["pairs"][I(self.cfgC)], {I(self.Z): self.Z}, "depts.json 등재 cwd 가 쌍이 아니다")
        self.assertNotIn(I(self.Y), reg["pairs"][I(self.cfgA)], "config 부재 항목이 본부 config 에 추정 귀속됐다")
        self.assertEqual(set(reg["configs"]), {I(self.cfgA), I(self.cfgB), I(self.cfgC)})
        self.assertEqual(len(reg["sources"]), 3, reg["sources"])       # 본부 topo · depts.json · dept-1 topo
        self.assertEqual(reg["unreadable"], [])
        self.assertEqual(reg["scope"], "full")

    def test_5b_is_workspace_pair_scoped_without_markers(self):
        p = self._pf(fix=False)
        self.assertFalse(os.path.exists(os.path.join(self.X, "CLAUDE.md")))
        self.assertFalse(os.path.isdir(os.path.join(self.X, "_round")))
        self.assertTrue(p._is_cysjavis_workspace(self.X, self.cfgA))
        self.assertFalse(p._is_cysjavis_workspace(self.Y, self.cfgA), "부서 cwd 가 본부 config 쌍으로 인정됐다(합집합 살포)")
        self.assertTrue(p._is_cysjavis_workspace(self.Y, self.cfgB))
        self.assertFalse(p._is_cysjavis_workspace(self.X3, self.cfgB), "codex 좌석 cwd 가 claude 쌍으로 인정됐다")
        self.assertTrue(p._is_cysjavis_workspace(self.X))
        self.assertFalse(p._is_cysjavis_workspace(self.stale, self.cfgA), "dir 부재(stale) 경로가 인정됐다")
        self.assertFalse(p._is_cysjavis_workspace(os.path.join(self.home, "random"), self.cfgA))

    def test_5c_gaps_include_missing_entry_missing_file_and_alias_only(self):
        K = pf.claude_project_key
        _write(os.path.join(self.cfgB, ".claude.json"),
               json.dumps({"projects": {K(self.Y): {"hasTrustDialogAccepted": True},
                                        K(self.X2) + "/": {"hasTrustDialogAccepted": True}}}))   # X2 는 별칭 true 만
        p = self._pf(fix=False)
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgB, ".claude.json")), [self.X2],
                         "별칭(꼬리 슬래시) true 를 신뢰로 인정했다(claude 는 정확 키만 읽는다 · R1)")
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgA, ".claude.json")), [self.X],
                         "파일 부재 config 의 등재 cwd 가 갭으로 잡히지 않았다(stale 은 제외)")
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgC, ".claude.json")), [self.Z],
                         "depts.json 등재 cwd 갭이 잡히지 않았다")
        _write(os.path.join(self.cfgC, ".claude.json"), json.dumps({"projects": [1]}))   # 형상 이상 → 전부 갭 · 예외 0
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgC, ".claude.json")), [self.Z])

    def test_5d_report_mode_read_only_then_fix_via_seed(self):
        bfile = os.path.join(self.cfgB, ".claude.json")
        _write(bfile, json.dumps({"hasCompletedOnboarding": True,
                                  "projects": {pf.claude_project_key(self.Y): {"hasTrustDialogAccepted": True}}}, indent=2))
        raw = _read_bytes(bfile)
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("trust gap", r["detail"])
        self.assertNotIn(self.X3, r["detail"], "codex/gemini 좌석 cwd 가 갭으로 보고됐다(영구 미수리 WARN 재현)")
        self.assertEqual(_read_bytes(bfile), raw, "report 모드가 파일을 썼다")
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json")))
        r = self._c58(self._pf(fix=True))
        self.assertEqual(r["status"], pf.FIXED, r)
        gotB = _read_json(bfile)
        self.assertIs(gotB["hasCompletedOnboarding"], True)
        self.assertEqual(gotB["projects"][pf.claude_project_key(self.X2)], {"hasTrustDialogAccepted": True})
        self.assertNotIn(pf.claude_project_key(self.X3), gotB["projects"], "codex 좌석 cwd 에 신뢰가 살포됐다")
        self.assertTrue(os.path.exists(bfile + ".bak-preflight"), "C58 --fix 백업 1회 계약 소실")
        gotA = _read_json(os.path.join(self.cfgA, ".claude.json"))
        self.assertEqual(gotA, {"projects": {pf.claude_project_key(self.X): {"hasTrustDialogAccepted": True}}})
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json.bak-preflight")), "부재 파일에 백업을 만들었다")
        gotC = _read_json(os.path.join(self.cfgC, ".claude.json"))
        self.assertEqual(gotC, {"projects": {pf.claude_project_key(self.Z): {"hasTrustDialogAccepted": True}}})
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.PASS, r)

    def test_5e_fix_refused_when_live_claude(self):
        p = self._pf(fix=True)
        saved = pf.claude_procs_for_config
        pf.claude_procs_for_config = lambda d, **k: (1, "fake-live")
        try:
            r = self._c58(p)
        finally:
            pf.claude_procs_for_config = saved
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("REFUSE live-claude", r["detail"])
        for c in (self.cfgA, self.cfgB, self.cfgC):
            self.assertFalse(os.path.exists(os.path.join(c, ".claude.json")))

    def test_5f_zero_pairs_is_skip_not_pass(self):
        """★R1: 판정할 쌍 0(출처 0 · 등재 0 · 판독불가) → SKIP. config 존재만으론 PASS 를 말하지 않는다."""
        for f in (os.path.join(self.home, ".local", "state", "cys", "topology.json"),
                  os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "topology.json")):
            os.unlink(f)
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {
            "dept-1": {"socket": "/nonexistent/cys-dept-dept-1/cys.sock", "account_dir": self.cfgB}}}))
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("쌍 0", r["detail"])
        self.assertIn("출처 1", r["detail"])          # depts.json 은 판독했다(대상 없음 ≠ 판정 불가 를 구분해 표기)
        _write(os.path.join(self.home, ".local", "state", "cys", "topology.json"), "{broken")
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("판독불가", r["detail"])
        os.unlink(os.path.join(self.home, ".cys", "depts.json"))
        os.unlink(os.path.join(self.home, ".local", "state", "cys", "topology.json"))
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("출처 0", r["detail"])

    def test_5g_registered_pair_with_missing_config_dir_is_warned(self):
        """★R1(codex): 등재 쌍은 있는데 config dir 이 없다 → 침묵 통과 아님 · 되살리지도 않는다."""
        import shutil
        shutil.rmtree(self.cfgC)
        r = self._c58(self._pf(fix=True))
        self.assertIn("config dir 부재", r["detail"])
        self.assertIn(self.cfgC, r["detail"])
        self.assertFalse(os.path.exists(self.cfgC), "--fix 가 지워진 계정 dir 을 되살렸다")


class DeptContextC58(_IsoEnv):
    """★R1 실물 _discover_isolation_block: 부서 컨텍스트(CYS_PACK_DIR=…/pack-dept-<n> · CYS_ACCOUNT_DIR=계정 dir) 에서 C58 은
    **자기 계정 config 의 쌍만** 보고·수리한다(종전: 빈 레지스트리 → 'PASS 0쌍 출처 0' 침묵 오판 — 감사 에러4 ③ 재현)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustdept-")
        self.home = os.path.join(self.tmp, "home")
        self._isolate(self.home)
        self.cfgA = os.path.join(self.home, ".cys", "claude")
        self.cfgB = os.path.join(self.home, ".cys", "claude-default-dept-1")
        self.cfgC = os.path.join(self.home, ".cys", "claude-default-dept-2")
        self.packB = os.path.join(self.home, ".cys", "pack-dept-dept-1")
        self.X = os.path.join(self.home, "wsX")
        self.Y = os.path.join(self.home, "wsY")
        self.Z = os.path.join(self.home, "wsZ")
        for d in (self.cfgA, self.cfgB, self.cfgC, self.packB, self.X, self.Y, self.Z,
                  os.path.join(self.home, ".local", "state", "cys"),
                  os.path.join(self.home, ".local", "state", "cys-dept-dept-1"),
                  os.path.join(self.home, ".local", "state", "cys-dept-dept-2")):
            os.makedirs(d, exist_ok=True)
        _write(os.path.join(self.home, ".local", "state", "cys", "topology.json"), json.dumps({"entries": [
            {"role": "master", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": self.X}]}))
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {
            "dept-1": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "cys.sock"),
                       "pack_dir": self.packB, "account_dir": self.cfgB},
            "dept-2": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-2", "cys.sock"),
                       "pack_dir": os.path.join(self.home, ".cys", "pack-dept-dept-2"), "account_dir": self.cfgC}}}))
        _write(os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "topology.json"), json.dumps({"entries": [
            {"role": "cso", "agent": "claude", "claude_config_dir": self.cfgB, "cwd": self.Y}]}))
        _write(os.path.join(self.home, ".local", "state", "cys-dept-dept-2", "topology.json"), json.dumps({"entries": [
            {"role": "cso", "agent": "claude", "claude_config_dir": self.cfgC, "cwd": self.Z}]}))
        self.keyY = pf.claude_project_key(self.Y)
        _write(os.path.join(self.cfgB, ".claude.json"), json.dumps({"projects": {self.keyY: {"hasTrustDialogAccepted": False}}}))
        os.environ["CYS_PACK_DIR"] = self.packB
        os.environ["CYS_ACCOUNT_DIR"] = self.cfgB

    def tearDown(self):
        self._restore()

    def test_8_dept_context_sees_and_repairs_only_own_account(self):
        reason, narrow = pf._discover_isolation_block()
        self.assertIsNotNone(reason, "픽스처가 부서 컨텍스트로 판정되지 않았다(하네스 결함)")
        self.assertEqual(narrow, [os.path.join(self.cfgB, "settings.json")])
        reg = pf.cysjavis_registry()
        self.assertEqual(reg["scope"], "account")
        self.assertEqual(set(reg["configs"]), {pf._path_identity(self.cfgB)}, "타 계정 config 가 부서 레지스트리에 보인다")
        self.assertEqual(set(reg["pairs"]), {pf._path_identity(self.cfgB)})
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.WARN, "부서 컨텍스트에서 자기 갭을 보지 못했다(종전 침묵 PASS): %s" % r)
        self.assertIn(self.Y, r["detail"])
        self.assertNotIn(os.path.join(self.cfgA, ".claude.json"), r["detail"])
        self.assertNotIn(os.path.join(self.cfgC, ".claude.json"), r["detail"])
        # --fix: seed_trust 스파이 — 호출 config 는 전부 자기 계정 · 타 계정 dir 스냅샷 불변
        calls = []
        real = pf.seed_trust

        def spy(config_dir, cwd, **kw):
            calls.append(config_dir)
            return real(config_dir, cwd, proc_counter=lambda d: (0, "t"), **kw)

        beforeA, beforeC = _snapshot(self.cfgA), _snapshot(self.cfgC)
        with patch.object(pf, "seed_trust", spy):
            r = self._c58(self._pf(fix=True))
        self.assertEqual(r["status"], pf.FIXED, r)
        self.assertEqual(calls, [self.cfgB])
        self.assertEqual((_snapshot(self.cfgA), _snapshot(self.cfgC)), (beforeA, beforeC), "타 계정 dir 이 변했다")
        self.assertIs(_read_json(os.path.join(self.cfgB, ".claude.json"))["projects"][self.keyY]["hasTrustDialogAccepted"], True)
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.PASS, r)
        self.assertIn("scope=account", r["detail"])

    def test_8b_dept_without_account_and_temp_pack_are_skip(self):
        os.environ.pop("CYS_ACCOUNT_DIR")
        reason, narrow = pf._discover_isolation_block()
        self.assertIsNotNone(reason)
        self.assertEqual(narrow, [])
        self.assertEqual(pf.cysjavis_registry()["scope"], "none")
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        os.environ["CYS_PACK_DIR"] = tempfile.mkdtemp(prefix="snap_grill_")     # 임시 팩 컨텍스트
        self.assertEqual(pf.cysjavis_registry()["scope"], "none")
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("scope=none", r["detail"])

    def test_8c_own_topology_malformed_is_reported_not_pass(self):
        _write(os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "topology.json"), "{broken")
        reg = pf.cysjavis_registry()
        self.assertEqual(reg["pairs"], {})
        self.assertEqual(len(reg["unreadable"]), 1, reg)
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("판독불가", r["detail"])


class WindowsStatePaths(_IsoEnv):
    """★R1: cysd 의 Windows 영속 위치(%LOCALAPPDATA%\\cys · \\<pipe_slug> · state.rs) 를 그대로 읽는다 — LOCALAPPDATA 부재는
    위치를 발명하지 않고 판독불가로 고지."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustwin-")
        self.home = os.path.join(self.tmp, "home")
        self._isolate(self.home)
        self._iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)
        self.la = os.path.join(self.tmp, "LocalAppData")
        self.cfg = os.path.join(self.home, ".cys", "claude-default-dept-x")
        self.cfgh = os.path.join(self.home, ".cys", "claude")
        self.ws = os.path.join(self.home, "ws")
        self.wsh = os.path.join(self.home, "wsh")
        for d in (self.cfg, self.cfgh, self.ws, self.wsh, os.path.join(self.la, "cys", "cys-dept-x"),
                  os.path.join(self.home, ".cys")):
            os.makedirs(d, exist_ok=True)
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {
            "x": {"socket": r"\\.\pipe\cys-dept-x", "account_dir": self.cfg}}}))
        _write(os.path.join(self.la, "cys", "cys-dept-x", "topology.json"), json.dumps({"entries": [
            {"role": "cso", "agent": "claude", "claude_config_dir": self.cfg, "cwd": self.ws}]}))
        _write(os.path.join(self.la, "cys", "topology.json"), json.dumps({"entries": [
            {"role": "master", "agent": "claude", "claude_config_dir": self.cfgh, "cwd": self.wsh}]}))

    def tearDown(self):
        pf._discover_isolation_block = self._iso
        self._restore()

    def test_9_pipe_socket_reads_localappdata_slug_dir(self):
        os.environ["LOCALAPPDATA"] = self.la
        reg = pf.cysjavis_registry()
        self.assertEqual(reg["pairs"].get(pf._path_identity(self.cfg)), {pf._path_identity(self.ws): self.ws},
                         "named pipe 부서의 topology 를 %%LOCALAPPDATA%%\\cys\\<slug> 에서 읽지 않았다: %s" % reg)
        self.assertIn(os.path.join(self.la, "cys", "cys-dept-x", "topology.json"), reg["sources"])
        self.assertEqual(pf._hub_state_dir(os_name="nt"), os.path.join(self.la, "cys"))
        self.assertEqual(pf._dept_state_dir("x", r"\\.\pipe\cys-dept-x"), os.path.join(self.la, "cys", "cys-dept-x"))
        self.assertEqual(pf._dept_state_dir("x", r"\\.\pipe\cys"), os.path.join(self.la, "cys"), "슬러그 'cys' 는 루트")
        self.assertEqual(pf._dept_state_dir("x", None, os_name="nt"), os.path.join(self.la, "cys", "cys-dept-x"))
        # 본부(nt)도 같은 루트에서 읽힌다
        with patch.object(pf, "_hub_state_dir", lambda **k: os.path.join(self.la, "cys")):
            reg = pf.cysjavis_registry()
        self.assertEqual(reg["pairs"].get(pf._path_identity(self.cfgh)), {pf._path_identity(self.wsh): self.wsh})

    def test_9b_missing_localappdata_is_unreadable_not_invented(self):
        os.environ.pop("LOCALAPPDATA", None)
        self.assertIsNone(pf._win_state_root())
        self.assertIsNone(pf._dept_state_dir("x", r"\\.\pipe\cys-dept-x"))
        reg = pf.cysjavis_registry()
        self.assertEqual(reg["pairs"], {})
        self.assertTrue(any("LOCALAPPDATA" in u for u in reg["unreadable"]), reg)
        self.assertFalse(os.path.exists(os.path.join(self.home, ".local", "state", "cys-dept-x")), "발명된 폴백 경로")
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.SKIP, r)
        self.assertIn("LOCALAPPDATA", r["detail"])

    def test_9c_unix_socket_dirname_no_fallback_and_xdg_linux_only(self):
        self.assertEqual(pf._dept_state_dir("d", "/vanished/parent/cys.sock", os_name="posix"), "/vanished/parent")
        os.environ["XDG_STATE_HOME"] = os.path.join(self.tmp, "xdg")
        self.assertEqual(pf._hub_state_dir(os_name="posix", platform="linux"), os.path.join(self.tmp, "xdg", "cys"))
        self.assertEqual(pf._hub_state_dir(os_name="posix", platform="darwin"),
                         os.path.join(self.home, ".local", "state", "cys"), "darwin 은 dirs::state_dir=None → home 폴백")
        self.assertEqual(pf._dept_state_dir("d", None, os_name="posix", platform="linux"),
                         os.path.join(self.home, ".local", "state", "cys-dept-d"), "cys-dept(bash) 는 XDG 를 모른다")


def _block(src, start_label, end_label):
    i = src.find("\n  %s)" % start_label)
    assert i > 0, start_label
    j = src.find("\n  %s)" % end_label, i)
    assert j > i, end_label
    return src[i:j]


class DeptWiringStatic(unittest.TestCase):
    def setUp(self):
        self.src = _read_text(DEPT)

    def test_6_three_sites_seed_before_daemon_and_shell_same_cwd(self):
        for blk, nxt in (("launch", "allocate"), ("allocate", "create"), ("create", "down")):
            body = _block(self.src, blk, nxt)
            i_seed = body.find("seed_trust_acct ")
            i_daemon = body.find('nohup "$CYSD"')
            self.assertGreater(i_seed, 0, "%s: seed_trust_acct 호출 부재" % blk)
            self.assertGreater(i_daemon, 0, blk)
            self.assertLess(i_seed, i_daemon, "%s: 시드가 데몬 스폰 뒤에 있다(claude 기동 前 계약 위반)" % blk)
            i_shell = body.find('"$CYS" new-surface')   # 실제 호출(주석 속 단어 아님)
            if i_shell > 0:
                self.assertLess(i_seed, i_shell, "%s: 시드가 빈 셸 생성 뒤에 있다" % blk)
                self.assertIn('--cwd "$dept_cwd"', body[i_shell:i_shell + 200], "%s: 빈 셸 cwd 가 시드 cwd 와 다르다" % blk)
            # ★R1: 확정한 하나의 cwd 가 시드·편성에 같이 간다
            self.assertIn('seed_trust_acct "$acctdir" "$dept_cwd"', body, blk)
            self.assertIn('formation_ensure_async "$name" "$sock" "$dept_cwd"', body,
                          "%s: 편성이 시드 cwd 를 받지 않는다(claude 가 다른 폴더에서 뜬다)" % blk)
            i_res = body.find('dept_cwd="$(resolve_dept_cwd ')
            self.assertGreater(i_res, 0, "%s: resolve_dept_cwd 미사용" % blk)
            self.assertLess(i_res, i_seed, "%s: cwd 확정이 시드 뒤" % blk)
        launch = _block(self.src, "launch", "allocate")
        self.assertLess(launch.find("seed_credentials_win"), launch.find("seed_trust_acct"),
                        "launch: creds 시드 바로 뒤 동형 배치가 아니다")
        self.assertNotIn("CYS_DEPT_SEED_CREDS", launch[launch.find("seed_trust_acct") - 40:launch.find("seed_trust_acct")],
                         "launch: 신뢰 시드가 creds opt-in 블록 안에 갇혔다(기본 off = 시드 0)")
        self.assertIn('resolve_dept_cwd "${CYS_DEPT_CWD:-}" "$name"', launch, "launch: 등재 cwd 복원 인자 부재")
        self.assertIn('resolve_dept_cwd "$cwd" ""', _block(self.src, "create", "down"), "create: 카탈로그 cwd 확정")

    def test_6b_helper_contract(self):
        m = re.search(r"^seed_trust_acct\(\)\{\n.*?^\}$", self.src, re.M | re.S)
        self.assertIsNotNone(m, "seed_trust_acct 함수 부재")
        fn = m.group(0)
        self.assertIn("--seed-trust --config", fn)
        self.assertIn("env -u CYS_SOCKET", fn)
        self.assertIn("javis_preflight.py", fn)
        self.assertTrue(fn.rstrip().endswith("return 0\n}"), "fail-open 계약(항상 rc 0) 위반")
        self.assertNotIn("exit ", fn, "헬퍼가 exit 한다 — 부서 부트를 죽일 수 있다")
        self.assertNotIn(" >&1", fn)
        self.assertEqual(fn.count(">&2"), 3, "진단은 전부 stderr(stdout 은 name 파서 계약)")
        m2 = re.search(r"^resolve_dept_cwd\(\)\{\n.*?^\}$", self.src, re.M | re.S)
        self.assertIsNotNone(m2, "resolve_dept_cwd 부재")
        fn2 = m2.group(0)
        self.assertIn('[ "$c" = "/" ] && c="$HOME"', fn2, "루트 cwd → $HOME 교정(cys.rs 와 동일) 부재")
        self.assertIn('[ -d "$c" ]', fn2, "부재 dir 건너뛰기 부재(PTY 실패 = 좌석 0)")
        self.assertTrue(fn2.rstrip().endswith('echo "$HOME"; return 0\n}'), fn2)
        self.assertLess(m2.start(), self.src.find("\n  launch)"), "resolve_dept_cwd 정의가 첫 사용 뒤")
        fm = re.search(r"^formation_ensure_async\(\)\{\n.*?^\}$", self.src, re.M | re.S).group(0)
        self.assertIn('ensure --socket "$sock" --cwd "$cwd" --json', fm, "formation 에 --cwd 전달 부재")
        self.assertIn('cwd="${3:-}"', fm)

    def test_6c_daemon_lines_env_u_prefix(self):
        lines = [l for l in self.src.splitlines() if 'nohup "$CYSD"' in l]
        self.assertEqual(len(lines), 4, lines)
        for l in lines:
            self.assertIn("env -u CYS_ROLE -u CYS_SURFACE_ID -u CYS_SURFACE_REF -u CYS_SEAT_TOKEN nohup", l, l)
            self.assertNotIn("CYS_SOCKET=", l.split("env -u", 1)[1], "env -u 뒤에 대입이 남아 있다")


class DeptLaunchWiring(unittest.TestCase):
    """launch 재사용 경로 완주(test_dept_creds_seed.LaunchWiring 동형 · Windows uname 목) — fork 계정 dir 에
    .claude.json 착지 + ★R1 formation 스텁 argv 실측(시드 cwd == 편성 --cwd). 팩 bin 은 실 dir: javis_preflight.py 는 repo
    심링크 · javis_formation.py 는 argv 기록 스텁."""

    NAME = "w1"

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustlaunch-")
        self.home = os.path.join(self.tmp, "home")
        self.bindir = os.path.join(self.home, ".local", "bin")
        os.makedirs(self.bindir)
        os.makedirs(os.path.join(self.home, ".cys"))
        _write(os.path.join(self.bindir, "uname"), '#!/bin/sh\necho "MINGW64_NT-10.0"\n', 0o755)
        log = os.path.join(self.tmp, "calls.log")
        _write(os.path.join(self.bindir, "cys"),
               '#!/bin/sh\necho "cys $@" >> "%s"\ncase "$1" in\n'
               '  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;\n'
               '  feed) exit 0 ;;\n  list) exit 0 ;;\n  tombstone) exit 0 ;;\nesac\nexit 0\n' % log, 0o755)
        _write(os.path.join(self.bindir, "cysd"), "#!/bin/sh\nexit 0\n", 0o755)
        open(os.path.join(self.tmp, r"\\.\pipe\cys-dept-%s" % self.NAME), "w").close()
        self.base = os.path.join(self.home, ".cys", "claude-default")
        os.makedirs(self.base)
        self.fork = self.base + "-" + self.NAME
        self.depts = os.path.join(self.home, ".cys", "depts.json")
        self._write_depts()
        packdir = os.path.join(self.home, ".cys", "pack")
        os.makedirs(os.path.join(packdir, "bin"))
        _write(os.path.join(packdir, "agents.json"),
               json.dumps({"claude": {"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": self.base}}}))
        os.symlink(PF, os.path.join(packdir, "bin", "javis_preflight.py"))
        self.fmlog = os.path.join(self.tmp, "formation.argv")
        _write(os.path.join(packdir, "bin", "javis_formation.py"),
               "import sys, json\nopen(%r, 'a').write(json.dumps(sys.argv[1:]) + '\\n')\nprint('{}')\n" % self.fmlog)

    def _write_depts(self, **extra):
        meta = {"socket": r"\\.\pipe\cys-dept-%s" % self.NAME,
                "pack_dir": os.path.join(self.home, ".cys", "pack-dept-%s" % self.NAME),
                "role": "dept-master", "account_dir": self.fork}
        meta.update(extra)
        _write(self.depts, json.dumps({"depts": {self.NAME: meta}}))

    def env(self, **extra):
        env = dict(os.environ)
        env.update({"HOME": self.home, "USERPROFILE": self.home,
                    "CYS_DEPTS_JSON": self.depts,
                    "PATH": self.bindir + os.pathsep + env.get("PATH", "")})
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR", "CYS_NO_AUTOSTART",
                  "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG", "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT",
                  "CYS_DEPT_SEED_CREDS", "CYS_DEPT_CWD", "CLAUDE_CONFIG_DIR", "CYS_SURFACE_ID",
                  "CYS_SURFACE_REF", "CYS_SEAT_TOKEN", "LOCALAPPDATA", "XDG_STATE_HOME"):
            env.pop(k, None)
        env.update(extra)
        return env

    def _launch(self, **extra):
        if os.path.exists(self.fmlog):
            os.unlink(self.fmlog)
        r = subprocess.run(["bash", DEPT, "launch", self.NAME], capture_output=True, text=True,
                           encoding="utf-8", env=self.env(**extra), cwd=self.tmp, timeout=120)   # 호출자 cwd = tmp ≠ HOME
        for _ in range(50):                       # formation 스텁은 백그라운드 — 기록을 기다린다
            if os.path.exists(self.fmlog):
                break
            time.sleep(0.1)
        argv = json.loads(_read_text(self.fmlog).splitlines()[-1]) if os.path.exists(self.fmlog) else None
        return r, argv

    def _formation_cwd(self, argv):
        self.assertIsNotNone(argv, "formation 스텁이 호출되지 않았다")
        self.assertIn("--cwd", argv, "formation ensure 에 --cwd 가 없다(claude 가 호출자 cwd 에서 뜬다): %s" % argv)
        return argv[argv.index("--cwd") + 1]

    def test_7_launch_seeds_fork_acct_with_home_pair_and_formation_same_cwd(self):
        r, argv = self._launch()
        self.assertEqual(r.returncode, 0, r.stderr)
        cfgfile = os.path.join(self.fork, ".claude.json")
        self.assertTrue(os.path.isfile(cfgfile), "launch 배선 미발동(fork 에 .claude.json 부재): %s" % r.stderr)
        key = pf.claude_project_key(self.home)
        self.assertEqual(_read_json(cfgfile), {"projects": {key: {"hasTrustDialogAccepted": True}}},
                         "fork 계정 dir 에 (acct, $HOME) 한 쌍만 있어야 한다")
        self.assertIn("seed-trust: OK seeded(", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.base, ".claude.json")), "base 계정 dir 이 오염됐다")
        # ★R1: 호출자 cwd(self.tmp) ≠ HOME 인데 편성은 시드와 같은 HOME 을 받는다(종전: --cwd 없음 = 호출자 cwd)
        self.assertEqual(self._formation_cwd(argv), self.home)
        # 2회차(rotate 재귀 재현): 멱등 · 여전히 rc 0 · 이미 신뢰라 프로브 0(무프로브 증명은 test_3e 의 PATH=/nonexistent)
        r2, argv2 = self._launch()
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertIn("already-trusted(", r2.stderr)
        self.assertNotIn("보류", r2.stderr, "이미 신뢰인데 WARN 보류가 났다(R1 위반)")

    def test_7b_launch_survives_seed_refusal(self):
        # 잠금을 다른 프로세스(테스트)가 보유 → 시드 REFUSE → launch 는 WARN 1줄로 계속(fail-open · 전 pane 0 금지)
        os.makedirs(self.fork, exist_ok=True)
        holder = open(os.path.join(self.fork, pf.SEED_TRUST_LOCK_NAME), "a+")
        self.addCleanup(holder.close)
        self.assertIs(pf._try_lock_nb(holder), True)
        r, argv = self._launch()
        self.assertEqual(r.returncode, 0, "시드 거부가 부서 기동을 죽였다: %s" % r.stderr)
        self.assertIn("신뢰 사전 주입 보류", r.stderr)
        self.assertIn("lock-busy", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.fork, ".claude.json")))
        self.assertIn("가동 완료", r.stdout + r.stderr, "launch 가 완주하지 않았다")
        self.assertEqual(self._formation_cwd(argv), self.home, "거부에도 편성은 진행·같은 cwd")

    def test_7c_cwd_sources_explicit_registered_root(self):
        """★R1: CYS_DEPT_CWD(명시) > 등재 cwd(create 기록) > $HOME · "/" → $HOME · 부재 dir 건너뜀 — 시드·편성 동일값."""
        req = os.path.join(self.tmp, "requested")
        os.makedirs(req)
        r, argv = self._launch(CYS_DEPT_CWD=req)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self._formation_cwd(argv), req)
        self.assertIn(pf.claude_project_key(req), _read_json(os.path.join(self.fork, ".claude.json"))["projects"])
        regd = os.path.join(self.tmp, "registered")
        os.makedirs(regd)
        self._write_depts(cwd=regd)
        r, argv = self._launch()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self._formation_cwd(argv), regd, "등재 cwd 가 복원되지 않았다")
        self.assertIn(pf.claude_project_key(regd), _read_json(os.path.join(self.fork, ".claude.json"))["projects"])
        self._write_depts(cwd=os.path.join(self.tmp, "vanished"))
        r, argv = self._launch()
        self.assertEqual(self._formation_cwd(argv), self.home, "부재 등재 dir 를 편성에 넘겼다(PTY 실패 = 좌석 0)")
        r, argv = self._launch(CYS_DEPT_CWD="/")
        self.assertEqual(self._formation_cwd(argv), self.home, '"/" 는 $HOME 으로(cys.rs 루트 교정과 정합)')
        self.assertNotIn("/", _read_json(os.path.join(self.fork, ".claude.json"))["projects"], '"/" 가 신뢰 키로 들어갔다')


# ══ codex(gpt-6-astra) R2 적대 반례 — 워커가 전 행 검토·수정 후 채택(초안 _codex_trust_counterexamples_draft.py 는 폐기) ══
# 채택 시 바뀐 것: 명시 null 항목 → ERROR(종전 대체) · 프로세스 확인이 makedirs 앞(거부 경로 무생성) · 교체 前 임시파일 되읽기.
# ★R1 재검토: 되읽기 실패 롤백(R2 채택분)은 codex R1 이 '동시 기록자 파괴' 로 뒤집었다 → 롤백 0(Concurrent.test_4e/4f 가 새 계약).
# 기각: 'env 가 argv 앞에 오는 ps 줄'(ps -E 는 argv→env 고정 · 파서가 argv 시작을 추측하면 안 된다).
# 잠금 파일(.claude.json.seed-lock)은 설계상 영속(phoenix lease 와 동형 · unlink 는 flock 경합을 만든다) → 픽스처가 미리 만든다.
def _write_any(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data if isinstance(data, bytes) else json.dumps(data).encode("utf-8"))


def _snapshot(root):
    """디렉터리명·파일 바이트·mtime 스냅샷(atime 무시) — '무쓰기' 단언용."""
    result = {}
    for parent, dirs, files in os.walk(root):
        for name in dirs + files:
            path = os.path.join(parent, name)
            key = os.path.relpath(path, root)
            if os.path.islink(path):
                result[key] = ("link", os.readlink(path))
            elif os.path.isdir(path):
                result[key] = ("dir",)
            else:
                with open(path, "rb") as f:
                    result[key] = ("file", f.read(), os.stat(path).st_mtime_ns)
    return result


class CodexCounterexamples(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trust-counter-")
        self.cfg = os.path.join(self.tmp, "config")
        self.ws = os.path.join(self.tmp, "workspace")
        os.makedirs(self.cfg)
        os.makedirs(self.ws)
        self.file = os.path.join(self.cfg, ".claude.json")
        self.key = pf.claude_project_key(self.ws)
        self.env = patch.dict(os.environ, {
            "HOME": self.tmp, "USERPROFILE": self.tmp, "CYS_DEPTS_JSON": os.path.join(self.tmp, "depts.json"),
            "CYS_ACCOUNT_DIR": self.cfg, "CLAUDE_CONFIG_DIR": self.cfg, "LOCALAPPDATA": os.path.join(self.tmp, "LA")})
        self.env.start()
        os.environ.pop("XDG_STATE_HOME", None)
        self.iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)

    def tearDown(self):
        pf._discover_isolation_block = self.iso
        self.env.stop()

    def seed(self, **kwargs):
        return pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "injected"),
                             lock_fn=lambda f: True, **kwargs)

    def lock_fixture(self, cfg=None):
        _write_any(os.path.join(cfg or self.cfg, pf.SEED_TRUST_LOCK_NAME), b"")

    def test_a_malformed_json_shapes_are_errors_without_writes(self):
        """a: 명시 null 은 '있는 비-object' — 대체 허가가 아니다(ERROR · 무쓰기). ★R1: 파일 전체 `null` · {"projects": null} 포함."""
        self.lock_fixture()
        cases = [[], {"projects": []}, None, {"projects": None}, "str", 7]
        cases += [{"projects": {self.key: x}} for x in (None, [], "bad", 7)]
        for data in cases:
            with self.subTest(data=data):
                _write_any(self.file, data)
                before = _snapshot(self.tmp)
                result = self.seed()
                self.assertEqual(result[:2], (1, "ERROR"), result)
                self.assertEqual(_snapshot(self.tmp), before, result)

    def test_a_file_directory_collisions_preserve_everything(self):
        """a: .claude.json 이 디렉터리 · config 가 파일 → ERROR · 무접촉 · 파일 config 는 프로브 앞에서 거부."""
        self.lock_fixture()
        os.makedirs(self.file)
        _write_any(os.path.join(self.file, "sentinel"), b"keep")
        before = _snapshot(self.tmp)
        self.assertEqual(self.seed()[:2], (1, "ERROR"))
        self.assertEqual(_snapshot(self.tmp), before)
        file_config = os.path.join(self.tmp, "file-config")
        _write_any(file_config, b"keep")
        before = _snapshot(self.tmp)
        result = pf.seed_trust(file_config, self.ws,
                               proc_counter=lambda d: self.fail("파일 config 는 프로브 앞에서 거부해야 한다"))
        self.assertEqual(result[:2], (1, "ERROR"), result)
        self.assertEqual(_snapshot(self.tmp), before)

    def test_a_refusal_creates_no_directory_or_file(self):
        """a(문자 그대로): 거부 경로는 config 디렉터리도 파일도 만들지 않는다. 잠금 종류는 잠금 파일이 영속 설계라 미리 둔다."""
        for kind in ("live", "unverified", "lock-busy", "lock-unavailable"):
            with self.subTest(kind=kind):
                cfg = os.path.join(self.tmp, kind)
                if kind.startswith("lock"):
                    os.makedirs(cfg)
                    self.lock_fixture(cfg)
                before = _snapshot(self.tmp)
                count = {"live": 1, "unverified": None}.get(kind, 0)
                result = pf.seed_trust(cfg, self.ws, proc_counter=lambda d: (count, kind),
                                       lock_fn=lambda f: False if kind == "lock-busy" else None)
                self.assertEqual(result[:2], (2, "REFUSE"), result)
                self.assertEqual(_snapshot(self.tmp), before, result)
                if not kind.startswith("lock"):
                    self.assertFalse(os.path.exists(cfg), "거부인데 config dir 을 만들었다")

    def test_a_unicode_normalized_and_config_equal_cwd(self):
        """a: 유니코드·꼬리 슬래시·'..' 세그먼트·cwd=config 도 정규화된 키 하나 · 거부는 보존."""
        unicode_ws = os.path.join(self.tmp, "작업-é-雪")
        os.makedirs(os.path.join(unicode_ws, "child"))
        for cwd in (self.cfg, unicode_ws + "/", unicode_ws + "/child/../"):
            with self.subTest(cwd=cwd):
                _write_any(self.file, {"projects": {}, "keep": ["雪"]})
                result = pf.seed_trust(self.cfg, cwd, proc_counter=lambda d: (0, "test"), lock_fn=lambda f: True)
                self.assertEqual(result[:2], (0, "OK"), result)
                self.assertEqual(_read_json(self.file), {"projects": {
                    pf.claude_project_key(cwd): {"hasTrustDialogAccepted": True}}, "keep": ["雪"]})
                before = _snapshot(self.tmp)
                result = pf.seed_trust(self.cfg, cwd, proc_counter=lambda d: (1, "live"))
                self.assertEqual(result[:2], (0, "OK"), "이미 신뢰인데 프로브 결과가 판정을 바꿨다(R1: 무프로브)")
                self.assertEqual(_snapshot(self.tmp), before)

    def test_b_truthy_non_booleans_become_true_without_other_changes(self):
        """b: 1·"yes" 같은 truthy 비-bool 은 정확히 True 로 · 문서 전체 깊은 비교 · 입력 독립(깊은 복사)."""
        for value in (1, "yes"):
            with self.subTest(value=value):
                data = {"hasCompletedOnboarding": False, "nested": [{"x": [1, None]}],
                        "projects": {self.key: {"hasTrustDialogAccepted": value, "tools": [{"keep": ["x"]}]},
                                     os.path.join(self.tmp, "other"): {"keep": [False]}}}
                before = copy.deepcopy(data)
                new, changed, key = pf.trust_plan(data, self.key)
                self.assertEqual(data, before)
                self.assertEqual(key, self.key)
                self.assertIs(changed, True)
                self.assertIs(new["projects"][key]["hasTrustDialogAccepted"], True)
                expected = copy.deepcopy(before)
                expected["projects"][key]["hasTrustDialogAccepted"] = True
                self.assertEqual(new, expected)
                new["projects"][key]["tools"][0]["keep"].append("changed")
                new["nested"][0]["x"].append("changed")
                self.assertEqual(data, before)

    def test_c_equals_suffix_key_and_env_wrapper(self):
        """c: '=' 는 값의 일부 · 접미 키(XCLAUDE_CONFIG_DIR)는 비매칭(env 는 보이므로 unresolved 0) · env 래퍼 줄은 0/1 어느 쪽도
        허용(exec 된 claude 가 자기 줄을 가지며, 래퍼를 세는 쪽은 보수적 = 거부 방향)."""
        target = os.path.join(self.tmp, "config=a=b")
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude CLAUDE_CONFIG_DIR=%s OTHER=x" % target], target), (1, 1, 0))
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude XCLAUDE_CONFIG_DIR=%s" % target], target), (0, 1, 0))
        count, parsed, unresolved = pf._count_claude_in_ps_lines(
            ["424242 env CLAUDE_CONFIG_DIR=%s claude" % target], target)
        self.assertEqual(parsed, 1)
        self.assertIn(count, (0, 1))

    def test_d_windows_output_and_process_filter(self):
        """d: 파싱된 전역 0 만 부재 증명(CRLF·공백 허용) · 빈/비숫자/≥1 은 None · 필터 문자열이 자기 pid·$PID·이름 4종·★R1
        이름 단독 계수(claude*)·CommandLine 부재 계수·ErrorActionPreference Stop 을 담는다."""
        for output, expected in (("0\r\n", 0), (" 0 ", 0), ("", None), ("abc", None), ("1", None)):
            with self.subTest(output=output):
                commands = []

                def runner(cmd):
                    commands.append(cmd)
                    return 0, output, ""

                count, detail = pf.claude_procs_for_config(self.cfg, runner=runner, os_name="nt", platform="win32")
                self.assertEqual(count, expected, detail)
                self.assertEqual(len(commands), 1)
                command = commands[0][-1]
                self.assertRegex(command, r"ProcessId\s+-ne\s+%d\b" % os.getpid())
                self.assertIn("ProcessId -ne $PID", command)
                for name in ("claude.exe", "claude", "node.exe", "node"):
                    self.assertIn("Name='%s'" % name, command)
                self.assertIn("$_.Name -like 'claude*'", command)
                self.assertIn("-not $_.CommandLine", command)
                self.assertTrue(command.startswith("$ErrorActionPreference='Stop'"))

    def test_e_malformed_registry_records_do_not_infer_pairs(self):
        """e: 손상 레코드 혼재 · named pipe 소켓 → %LOCALAPPDATA%\\cys\\cys-dept-x(★R1 · 종전 ~/.local/state 폴백은 cysd 위치가
        아니었다) · account_dir 로 쌍이 전파되지 않는다 · depts.json 은 판독 출처로 기록."""
        other = os.path.join(self.tmp, "other-config")
        topo = os.path.join(self.tmp, "LA", "cys", "cys-dept-x", "topology.json")
        depts = os.environ["CYS_DEPTS_JSON"]
        _write_any(depts, {"depts": {
            "null": None, "string": "bad", "missing": {"account_dir": other},
            "x": {"socket": r"\\.\pipe\cys-dept-x", "account_dir": other}}})
        bad_entries = [None, "bad"] + [{"agent": "claude", "claude_config_dir": self.cfg, "cwd": v} for v in (None, 3, [], {})]
        bad_entries += [{"agent": "claude", "cwd": self.ws}, {"agent": "claude", "claude_config_dir": 3, "cwd": self.ws},
                        {"agent": "codex", "claude_config_dir": self.cfg, "cwd": self.ws}]
        for payload, expected in (([], {}), (b"{broken", {}),
                                  ({"entries": bad_entries + [{"agent": "claude", "claude_config_dir": self.cfg, "cwd": self.ws}]},
                                   {pf._path_identity(self.cfg): {pf._path_identity(self.ws): self.ws}})):
            with self.subTest(payload=payload):
                _write_any(topo, payload)
                reg = pf.cysjavis_registry()
                self.assertEqual(reg["pairs"], expected)
                self.assertNotIn(pf._path_identity(other), reg["pairs"])
                self.assertIn(pf._path_identity(other), reg["configs"])
                self.assertEqual(reg["sources"], [depts, topo] if expected else [depts])
                # 존재하는데 dict 가 안 나오는 파일([] · 손상)은 '판독불가' 로 고지(침묵 0)
                self.assertEqual(reg["unreadable"], [topo] if not expected else [])
        self.assertFalse(os.path.exists(os.path.join(self.tmp, ".local", "state", "cys-dept-x")))

    def test_f_symlink_alias_key_is_not_trust_and_is_left_alone(self):
        """f(★R1 재검토): 심링크 별칭 키 true 는 C58 갭 판정에서 신뢰가 아니다(claude 는 realpath 키를 읽는다) · 시드는 정확 키를
        만들고 별칭 항목은 무접촉."""
        alias = os.path.join(self.tmp, "alias")
        os.symlink(self.ws, alias)
        _write_any(os.path.join(self.tmp, ".local", "state", "cys", "topology.json"),
                   {"entries": [{"agent": "claude", "claude_config_dir": self.cfg, "cwd": self.ws}]})
        p = pf.Preflight(fix=False, skips=[], mode="report", allow_irreversible=False)
        _write_any(self.file, {"projects": {alias: {"hasTrustDialogAccepted": True, "keep": [1]}}})
        before = _snapshot(self.tmp)
        self.assertEqual(p._trust_gap_workspaces(self.file), [self.ws], "별칭 true 를 신뢰로 인정했다")
        self.assertEqual(_snapshot(self.tmp), before)
        self.assertEqual(self.seed()[:2], (0, "OK"))
        self.assertEqual(_read_json(self.file), {"projects": {alias: {"hasTrustDialogAccepted": True, "keep": [1]},
                                                              self.key: {"hasTrustDialogAccepted": True}}})
        self.assertEqual(p._trust_gap_workspaces(self.file), [])
        # 등재 cwd 가 별칭 경로여도 정확 키는 realpath(claude getcwd) — 쌍의 cwd 문자열이 아니다
        _write_any(os.path.join(self.tmp, ".local", "state", "cys", "topology.json"),
                   {"entries": [{"agent": "claude", "claude_config_dir": self.cfg, "cwd": alias}]})
        p = pf.Preflight(fix=False, skips=[], mode="report", allow_irreversible=False)
        self.assertEqual(p._trust_gap_workspaces(self.file), [])

    def test_g_shell_definition_arity_and_opt_in_scope(self):
        """g: seed_trust_acct 정의가 첫 호출 앞 · 호출 3건 모두 인자 정확히 2 · 어느 호출도 CYS_DEPT_SEED_CREDS opt-in if 블록 안이 아님."""
        src = _read_text(DEPT)
        definition = re.search(r"^seed_trust_acct\(\)\{.*?^\}", src, re.M | re.S)
        self.assertIsNotNone(definition)
        calls = list(re.finditer(r"^\s*seed_trust_acct[ \t]+([^\n]+)", src, re.M))
        self.assertEqual(len(calls), 3)
        for call in calls:
            self.assertLess(definition.end(), call.start())
            self.assertEqual(len(shlex.split(call.group(1), comments=True)), 2, call.group())
        for gate in re.finditer(r"^[ \t]*if\b[^\n]*CYS_DEPT_SEED_CREDS[^\n]*", src, re.M):
            depth = 0
            end = None
            for token in re.finditer(r"(?:^|;)\s*(if|fi)\b", src[gate.start():], re.M):
                depth += 1 if token.group(1) == "if" else -1
                if depth == 0:
                    end = gate.start() + token.end()
                    break
            self.assertIsNotNone(end, "unclosed credentials opt-in")
            for call in calls:
                self.assertFalse(gate.start() <= call.start() < end, call.group())


# ══ codex(gpt-6-astra) R1 적대 반례 18 — 워커가 전 행 검토 후 채택(초안 _codex_r1_counterexamples_draft.py 는 폐기 · import 배선만 수정) ══
# 초안 실행에서 실패한 2건이 구현 결함이었다 → 수정: ⑦ 교체 후 되읽기 판정을 dict 동등(1/1.0 == True 통과) 대신 엄격 `is True` 로 ·
# ⑩ depts.json 이 object 면(depts 키 부재/형상 이상 포함) 판독 출처 — 비-object(null·[]·손상)만 판독불가.

class CodexR1Counterexamples(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="codex-r1-")
        self.root = self.temp.name
        self.cfg = os.path.join(self.root, "account")
        self.ws = os.path.join(self.root, "WorkSpace")
        os.makedirs(self.cfg)
        os.makedirs(self.ws)
        self.file = os.path.join(self.cfg, ".claude.json")
        self.bak = self.file + ".bak-preflight"
        self.key = os.path.realpath(self.ws)
        self.depts = os.path.join(self.root, "depts.json")
        self.env = patch.dict(os.environ, {
            "HOME": self.root, "USERPROFILE": self.root,
            "CYS_DEPTS_JSON": self.depts,
            "LOCALAPPDATA": os.path.join(self.root, "LocalAppData"),
            "XDG_STATE_HOME": os.path.join(self.root, "xdg"),
            "CYS_PACK_DIR": os.path.join(self.root, ".cys", "pack"),
            "CYS_ACCOUNT_DIR": self.cfg, "CLAUDE_CONFIG_DIR": self.cfg,
            "CYS_SOCKET": "",
        })
        self.env.start()
        # Restore the environment even if fixture cleanup fails.
        self.addCleanup(self.temp.cleanup)
        self.addCleanup(self.env.stop)

    def tearDown(self):
        self.env.stop()

    def write(self, path, data):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data if isinstance(data, bytes) else json.dumps(data).encode())

    def read(self, path):
        with open(path, "rb") as f:
            return f.read()

    def doc(self, flag=True):
        return {"projects": {self.key: {"hasTrustDialogAccepted": flag}}}

    def seed(self, **kw):
        options = {"proc_counter": lambda d: (0, "t"), "lock_fn": lambda f: True}
        options.update(kw)
        return pf.seed_trust(self.cfg, self.ws, **options)

    def preflight(self, fix=False):
        return pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report")

    def c58(self, fix=False):
        p = self.preflight(fix)
        p.c58_trust_harden()
        self.assertEqual(len(p.results), 1)
        return p.results[0]

    def hub_registry(self):
        with patch.object(pf, "_discover_isolation_block", lambda: (None, None)):
            return pf.cysjavis_registry()

    def topology(self, entries):
        path = os.path.join(pf._hub_state_dir(), "topology.json")
        self.write(path, {"entries": entries})
        return path

    def entry(self, agent="claude", cfg=None, cwd=None):
        return {"agent": agent, "claude_config_dir": cfg or self.cfg, "cwd": cwd or self.ws}

    def test_01_malformed_aliases_do_not_poison_exact_plan_or_c58(self):
        """Catches case-folded alias reuse and validation of unrelated malformed alias entries."""
        alias = os.path.join(self.root, "alias")
        os.symlink(self.ws, alias)
        case_alias = self.key[:-len("WorkSpace")] + "workspace"
        aliases = {case_alias: {"hasTrustDialogAccepted": True}, alias: None, self.key + "/": []}
        data = {"projects": copy.deepcopy(aliases)}
        self.assertEqual(pf.claude_project_key(alias), self.key)
        self.assertFalse(pf._trusted_exact(data, self.key))
        new, changed, key = pf.trust_plan(data, self.key)
        self.assertEqual((changed, key), (True, self.key))
        self.assertEqual(new, {"projects": dict(aliases, **{self.key: {"hasTrustDialogAccepted": True}})})
        self.assertEqual(data, {"projects": aliases})
        self.write(self.file, data)
        self.topology([self.entry(cwd=alias)])
        with patch.object(pf, "_discover_isolation_block", lambda: (None, None)):
            self.assertEqual(self.preflight()._trust_gap_workspaces(self.file), [alias])
        # Direct API validation: invalid exact nodes differ from invalid aliases.
        for bad in (None, [], False, {"projects": None}, {"projects": {self.key: None}}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                pf.trust_plan(bad, self.key)
        self.assertEqual(pf.trust_plan({}, self.key)[0], self.doc())

    def test_02_lock_update_is_seen_before_read_plan_and_noop(self):
        """Catches reads or probes before locking and backup/write activity on the no-op path."""
        self.write(self.file, self.doc(False))
        self.write(os.path.join(self.cfg, pf.SEED_TRUST_LOCK_NAME), b"")
        events = []
        saved = []
        raw = b'\xef\xbb\xbf' + json.dumps(self.doc(), indent=4).encode() + b'\n'
        real_read, real_plan = pf._read_claude_json_bytes, pf.trust_plan

        def lock(f):
            events.append("lock")
            self.write(self.file, raw)
            saved.append(os.stat(self.file).st_mtime_ns)
            return True

        def read(path):
            events.append("read")
            return real_read(path)

        def plan(data, key):
            events.append("plan")
            return real_plan(data, key)

        probe = Mock(side_effect=AssertionError("no-op probed"))
        with patch.object(pf, "_read_claude_json_bytes", read), patch.object(pf, "trust_plan", plan), \
                patch.object(pf.os, "replace", side_effect=AssertionError("no-op wrote")):
            result = self.seed(lock_fn=lock, proc_counter=probe, backup=True)
        self.assertEqual(result[:2], (0, "OK"), result)
        self.assertTrue(result[2].startswith("already-trusted("))
        self.assertEqual(events, ["lock", "read", "plan"])
        probe.assert_not_called()
        self.assertEqual((self.read(self.file), os.stat(self.file).st_mtime_ns), (raw, saved[0]))
        self.assertFalse(os.path.exists(self.bak))

    def test_03_invalid_plan_preempts_forced_live_probe(self):
        """Catches probe-before-plan returning live refusal instead of the existing document error."""
        for raw in (b"null", b'{"projects":null}', json.dumps({"projects": {self.key: None}}).encode()):
            with self.subTest(raw=raw):
                self.write(self.file, raw)
                probe = Mock(return_value=(9, "live"))
                result = self.seed(proc_counter=probe, force_unverified=True, backup=True)
                self.assertEqual(result[:2], (1, "ERROR"), result)
                probe.assert_not_called()
                self.assertEqual(self.read(self.file), raw)
                self.assertFalse(os.path.exists(self.bak))

    def test_04_backup_is_captured_bytes_and_exclusive_before_replace(self):
        """Catches copying a mutable original into backup or replacing before the backup exists."""
        original = b'\xef\xbb\xbf{ "theme": "old" }\n'
        late = b'{"late-writer":true}'
        self.write(self.file, original)
        real_open, real_replace = os.open, os.replace
        events = []

        def hook():
            events.append("hook")
            temps = [n for n in os.listdir(self.cfg) if n.startswith(".claude.json.seed-") and n != pf.SEED_TRUST_LOCK_NAME]
            self.assertEqual(len(temps), 1)
            self.assertTrue(pf._trusted_exact(json.loads(self.read(os.path.join(self.cfg, temps[0]))), self.key))
            self.assertFalse(os.path.exists(self.bak))

        def opening(path, flags, *args, **kw):
            if path == self.bak:
                events.append("backup")
                self.assertTrue(flags & os.O_EXCL)
                self.write(self.file, late)  # Deliberately after the final comparison.
            return real_open(path, flags, *args, **kw)

        def replacing(src, dst):
            events.append("replace")
            self.assertEqual(os.path.dirname(src), self.cfg)
            self.assertEqual(self.read(self.bak), original)
            return real_replace(src, dst)

        with patch.object(pf.os, "open", opening), patch.object(pf.os, "replace", replacing):
            result = self.seed(backup=True, _pre_write_hook=hook)
        self.assertEqual(result[:2], (0, "OK"), result)
        self.assertEqual(events, ["hook", "backup", "replace"])
        self.assertEqual(self.read(self.bak), original)

    def test_05_racing_backup_winner_is_never_truncated(self):
        """Catches an exists-then-open backup implementation overwriting a concurrent backup winner."""
        self.write(self.file, b"{}")
        real_open = os.open
        winner = b"first backup wins\x00"
        attempts = []

        def opening(path, flags, *args, **kw):
            if path == self.bak:
                attempts.append(flags)
                self.write(self.bak, winner)
            return real_open(path, flags, *args, **kw)

        with patch.object(pf.os, "open", opening):
            result = self.seed(backup=True)
        self.assertEqual(result[:2], (0, "OK"), result)
        self.assertEqual(len(attempts), 1)
        self.assertTrue(attempts[0] & os.O_EXCL)
        self.assertEqual(self.read(self.bak), winner)
        self.write(self.file, b'{"another":"seed"}')
        self.assertEqual(self.seed(backup=True)[:2], (0, "OK"))
        self.assertEqual(self.read(self.bak), winner)

    def test_06_existence_races_do_not_create_backup_or_reach_replace(self):
        """Catches bytes-only comparison and premature backups on empty-file existence transitions."""
        for initially_present in (False, True):
            with self.subTest(initially_present=initially_present):
                if initially_present:
                    self.write(self.file, b"")
                elif os.path.exists(self.file):
                    os.unlink(self.file)
                def hook():
                    if initially_present:
                        os.unlink(self.file)
                    else:
                        self.write(self.file, b"")
                with patch.object(pf.os, "replace") as replace:
                    result = self.seed(backup=True, _pre_write_hook=hook)
                self.assertEqual(result[:2], (2, "REFUSE"), result)
                self.assertIn("concurrent-change", result[2])
                replace.assert_not_called()
                self.assertFalse(os.path.exists(self.bak))
                self.assertEqual(os.path.exists(self.file), not initially_present)
                self.assertFalse(any(n.startswith(".claude.json.seed-") and n != pf.SEED_TRUST_LOCK_NAME
                             for n in os.listdir(self.cfg)))

    def test_07_post_commit_numeric_true_is_not_boolean_true(self):
        """Catches Python dict equality treating a post-commit numeric 1 as the required boolean True."""
        for value in (1, 1.0):
            with self.subTest(value=value):
                self.write(self.file, b"{}")
                payload = json.dumps(self.doc(value)).encode()
                real_replace = os.replace
                def replacing(src, dst):
                    real_replace(src, dst)
                    self.write(dst, payload)
                with patch.object(pf.os, "replace", side_effect=replacing) as replace:
                    result = self.seed(backup=True)
                self.assertEqual(self.read(self.file), payload)
                self.assertEqual(replace.call_count, 1, "rollback attempted")
                self.assertEqual(result[:2], (2, "REFUSE"), result)
                self.assertIn("post-commit", result[2])

    def test_08_post_commit_alias_and_invalid_utf8_preserve_writer_and_backup(self):
        """Catches post-commit identity trust reuse, rollback, and overwriting the preflight backup."""
        kept = self.doc()
        kept["writer"] = "changed"
        cases = [({"projects": {self.key + "/": {"hasTrustDialogAccepted": True}}}, (2, "REFUSE"), "post-commit"),
                 (kept, (0, "OK"), "플래그 보존"),
                 (b"\xff\xfeinvalid", (1, "ERROR"), "커밋 상태로 둔다")]
        for data, expected, note in cases:
            with self.subTest(data=data):
                original = b'{"preserve":"original"}'
                self.write(self.file, original)
                payload = data if isinstance(data, bytes) else json.dumps(data).encode()
                real_replace = os.replace
                def replacing(src, dst):
                    real_replace(src, dst)
                    self.write(dst, payload)
                with patch.object(pf.os, "replace", side_effect=replacing) as replace:
                    result = self.seed(backup=True)
                self.assertEqual(result[:2], expected, result)
                self.assertIn(note, result[2])
                self.assertEqual(replace.call_count, 1)
                self.assertEqual(self.read(self.file), payload)
                self.assertEqual(self.read(self.bak), original)

    def test_09_topology_filters_each_path_independently(self):
        """Catches OR-based absolute validation and acceptance of missing, null, or non-Claude agents."""
        entries = [self.entry()]
        for field in ("cwd", "claude_config_dir"):
            for value in ("relative", "", None, False, 7, []):
                e = self.entry()
                e[field] = value
                entries.append(e)
                self.assertFalse(pf._abs_str(value))
        for agent in (None, "codex", "gemini", "CLAUDE", ""):
            entries.append(self.entry(agent))
        missing = self.entry()
        del missing["agent"]
        entries.append(missing)
        path = self.topology(entries)
        self.assertEqual(pf._topology_pairs(path), [(self.cfg, self.ws)])
        # Independently invalid depts paths must not be inferred from the valid half.
        self.write(self.depts, {"depts": {"a": {"account_dir": "relative", "cwd": self.ws},
                                         "b": {"account_dir": self.cfg, "cwd": "relative"}}})
        self.assertEqual(self.hub_registry()["pairs"], {pf._path_identity(self.cfg): {pf._path_identity(self.ws): self.ws}})

    def test_10_every_parsed_depts_dict_is_a_source(self):
        """Catches treating a valid depts.json object as unreadable because its depts member is absent or malformed."""
        for data in ({}, {"version": 1}, {"depts": None}, {"depts": []}, {"depts": {}}):
            with self.subTest(data=data):
                self.write(self.depts, data)
                reg = self.hub_registry()
                self.assertEqual((reg["sources"], reg["unreadable"]), ([self.depts], []))
                self.assertEqual(reg["pairs"], {})

    def test_11_nonobject_sources_are_reported_even_with_a_valid_pair(self):
        """Catches silently dropping existing non-object registries when another topology supplies a valid pair."""
        topo = self.topology([self.entry()])
        for raw in (b"null", b"[]", b"false", b"{broken"):
            with self.subTest(raw=raw):
                self.write(self.depts, raw)
                reg = self.hub_registry()
                self.assertEqual(reg["unreadable"], [self.depts])
                self.assertEqual(reg["sources"], [topo])
                self.assertTrue(reg["pairs"])

    def test_12_scope_uses_parent_identity_and_never_prefix_matches(self):
        """Catches prefix account matching, failure to resolve narrow aliases, and copying the no-scope object."""
        other = self.cfg + "-other"
        alias = os.path.join(self.root, "account-link")
        os.symlink(self.cfg, alias)
        ident = pf._path_identity
        reg = {"pairs": {ident(c): {ident(self.ws): self.ws} for c in (self.cfg, other)},
               "configs": {ident(c): c for c in (self.cfg, other)},
               "sources": [self.depts], "unreadable": [self.file], "scope": "full"}
        original = copy.deepcopy(reg)
        self.assertIs(pf._scope_registry(reg, None, []), reg)
        for narrow in (None, []):
            self.assertEqual(pf._scope_registry(reg, "isolated", narrow),
                             {"pairs": {}, "configs": {}, "sources": [], "unreadable": [], "scope": "none"})
        scoped = pf._scope_registry(reg, "dept", [os.path.join(alias, "settings.json"), None, ""])
        self.assertEqual(scoped["scope"], "account")
        self.assertEqual(set(scoped["pairs"]), {ident(self.cfg)})
        self.assertEqual(scoped["configs"], {ident(self.cfg): self.cfg})
        self.assertEqual(scoped["sources"], reg["sources"])
        self.assertEqual(scoped["unreadable"], reg["unreadable"])
        self.assertEqual(reg, original)

    def test_13_state_slugs_empty_components_and_relative_xdg(self):
        """Catches whole-path slug sanitization, empty-slug fallback invention, and relative XDG acceptance."""
        root = os.path.join(os.environ["LOCALAPPDATA"], "cys")
        for sock, slug in ((r"\\.\pipe\discard/me\é A.!_-", "éA_-"),
                           (r"\\.\pipe\!!!", ""), ("//./pipe/cys/", ""), ("//./pipe/cys", "cys")):
            with self.subTest(sock=sock):
                self.assertEqual(pf._pipe_slug(sock), slug)
                self.assertEqual(pf._dept_state_dir("d", sock, os_name="posix"),
                                 root if slug in ("", "cys") else os.path.join(root, slug))
        sock = os.path.join(self.root, "gone", "..", "literal", "cys.sock")
        self.assertEqual(pf._dept_state_dir("d", sock, os_name="posix"), os.path.dirname(sock))
        self.assertEqual(pf._hub_state_dir("posix", "linux"), os.path.join(self.root, "xdg", "cys"))
        os.environ["XDG_STATE_HOME"] = "relative/state"
        fallback = os.path.join(self.root, ".local", "state")
        self.assertEqual(pf._unix_state_root("linux"), fallback)
        self.assertEqual(pf._hub_state_dir("posix", "darwin"), os.path.join(fallback, "cys"))
        self.assertEqual(pf._dept_state_dir("d", None, "posix", "linux"), os.path.join(fallback, "cys-dept-d"))
        os.environ["LOCALAPPDATA"] = ""
        self.assertIsNone(pf._win_state_root())
        self.assertIsNone(pf._hub_state_dir("nt", "win32"))
        self.assertIsNone(pf._dept_state_dir("d", "//./pipe/!!!", "posix"))
        self.assertIsNone(pf._dept_state_dir("d", None, "nt"))

    def test_14_ps_default_alias_and_self_exclusions_keep_unresolved_separate(self):
        """Catches default-config string matching and counting excluded self lines as unresolved."""
        default = os.path.join(self.root, ".claude")
        os.makedirs(default)
        alias = os.path.join(self.root, "default-link")
        os.symlink(default, alias)
        lines = ["710001 claude HOME=" + self.root, "710002 claude --continue",
                 "710003 claude", "junk", "710004 python HOME=" + self.root]
        self.assertEqual(pf._count_claude_in_ps_lines(lines, alias, {"710003"}), (1, 4, 1))
        self.assertEqual(pf._count_claude_in_ps_lines(lines, self.cfg, {"710003"}), (0, 4, 1))
        with patch.object(pf.os, "getpid", return_value=710003), patch.object(pf.os, "getppid", return_value=710004):
            for cfg, expected in ((alias, 1), (self.cfg, None)):
                count, detail = pf.claude_procs_for_config(cfg, os_name="posix", platform="darwin",
                                                         runner=lambda cmd: (0, "\n".join(lines), ""))
                self.assertEqual(count, expected, detail)

    def test_15_procfs_unknown_uid_stays_unresolved_but_positive_wins(self):
        """Catches treating unknown ownership as another user and allowing force to bypass a positive procfs match."""
        root = os.path.join(self.root, "proc")
        denied = os.path.join(root, "710011")
        readable = os.path.join(root, "710012")
        self.write(os.path.join(denied, "environ"), b"")
        self.write(os.path.join(readable, "environ"), b"HOME=" + self.root.encode() + b"\0")
        self.write(os.path.join(readable, "cmdline"), b"python\0")
        real_open, real_stat = builtins.open, os.stat
        def opening(path, *args, **kw):
            if os.fspath(path) == os.path.join(denied, "environ"):
                raise PermissionError("injected environ denial")
            return real_open(path, *args, **kw)
        for owner, expected in (("unknown", None), (1000, None), (1001, 0)):
            with self.subTest(owner=owner):
                def statting(path, *args, **kw):
                    if os.fspath(path) == denied:
                        if owner == "unknown":
                            raise PermissionError("injected uid denial")
                        return SimpleNamespace(st_uid=owner)
                    return real_stat(path, *args, **kw)
                with patch("builtins.open", opening), patch.object(pf.os, "stat", statting), \
                        patch.object(pf.os, "getuid", return_value=1000, create=True):
                    count, detail = pf._count_claude_procfs(self.cfg, root)
                self.assertEqual(count, expected, detail)
        self.write(os.path.join(readable, "environ"), b"CLAUDE_CONFIG_DIR=" + self.cfg.encode() + b"\0")
        self.write(os.path.join(readable, "cmdline"), b"claude\0")
        absent = os.path.join(self.root, "absent-config")
        # Attribute the positive process to the absent config, then force must still refuse.
        self.write(os.path.join(readable, "environ"), b"CLAUDE_CONFIG_DIR=" + absent.encode() + b"\0")
        with patch("builtins.open", opening), patch.object(pf.os, "getuid", return_value=os.stat(denied).st_uid, create=True):
            result = pf.seed_trust(absent, self.ws, force_unverified=True, lock_fn=lambda f: True,
                                   proc_counter=lambda d: pf._count_claude_procfs(d, root))
        self.assertEqual(result[:2], (2, "REFUSE"), result)
        self.assertIn("live-claude", result[2])
        self.assertFalse(os.path.exists(absent))

    def test_16_c58_missing_account_warn_survives_successful_other_repair(self):
        """Catches FIXED masking a missing registered account and seed calls for codex or gemini seats."""
        missing = os.path.join(self.root, "missing-account")
        excluded = os.path.join(self.root, "excluded-seat")
        os.makedirs(excluded)
        self.topology([self.entry(), self.entry(cfg=missing),
                       self.entry("codex", cwd=excluded), self.entry("gemini", cwd=excluded)])
        real_seed = pf.seed_trust
        def seed(cfg, cwd, **kw):
            return real_seed(cfg, cwd, proc_counter=lambda d: (0, "t"), lock_fn=lambda f: True, **kw)
        with patch.object(pf, "_discover_isolation_block", lambda: (None, None)), \
                patch.object(pf, "seed_trust", side_effect=seed) as spy:
            result = self.c58(True)
        self.assertEqual(result["status"], pf.WARN, result)
        self.assertIn("config dir 부재", result["detail"])
        self.assertIn(missing, result["detail"])
        self.assertNotIn(excluded, result["detail"])
        spy.assert_called_once_with(self.cfg, self.ws, backup=True)
        self.assertFalse(os.path.exists(missing))
        self.assertEqual(json.loads(self.read(self.file)), self.doc())

    def test_17_real_dept_context_with_only_foreign_pairs_skips(self):
        """Catches counting global pairs before account scoping and reporting PASS for an empty own account."""
        pack = os.path.join(self.root, ".cys", "pack-dept-d1")
        os.makedirs(pack)
        os.environ["CYS_PACK_DIR"] = pack
        os.environ["CYS_ACCOUNT_DIR"] = self.cfg
        foreign = os.path.join(self.root, "foreign-account")
        os.makedirs(foreign)
        self.topology([self.entry(cfg=foreign)])
        self.write(self.depts, {"depts": {"d1": {"account_dir": self.cfg}}})
        reason, narrow = pf._discover_isolation_block()  # Real isolation logic, intentionally unpatched.
        self.assertIsNotNone(reason)
        self.assertEqual(narrow, [os.path.join(self.cfg, "settings.json")])
        reg = pf.cysjavis_registry()
        self.assertEqual(reg["scope"], "account")
        self.assertEqual(reg["pairs"], {})
        self.assertEqual(reg["configs"], {pf._path_identity(self.cfg): self.cfg})
        with patch.object(pf, "seed_trust", side_effect=AssertionError("empty scope seeded")):
            result = self.c58(True)
        self.assertEqual(result["status"], pf.SKIP, result)
        self.assertIn("쌍 0", result["detail"])
        self.assertFalse(os.path.exists(self.file))
        self.assertEqual(os.listdir(foreign), [])

    def test_18_temp_verification_failure_preempts_hook_backup_and_replace(self):
        """Catches verification after the race hook or after backup/replace instead of before commit."""
        original = b'{"keep":42}'
        self.write(self.file, original)
        real_open = builtins.open
        def opening(path, mode="r", *args, **kw):
            if (isinstance(path, str) and os.path.dirname(path) == self.cfg
                    and os.path.basename(path).startswith(".claude.json.seed-") and mode == "rb"):
                return io.BytesIO(b'{"tampered":true}')
            return real_open(path, mode, *args, **kw)
        hook = Mock()
        with patch("builtins.open", opening), patch.object(pf.os, "replace") as replace:
            result = self.seed(backup=True, _pre_write_hook=hook)
        self.assertEqual(result[:2], (1, "ERROR"), result)
        hook.assert_not_called()
        replace.assert_not_called()
        self.assertEqual(self.read(self.file), original)
        self.assertFalse(os.path.exists(self.bak))
        self.assertFalse(any(n.startswith(".claude.json.seed-") and n != pf.SEED_TRUST_LOCK_NAME
                             for n in os.listdir(self.cfg)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
