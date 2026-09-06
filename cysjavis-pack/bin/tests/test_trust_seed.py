#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_trust_seed.py — ★WP-2(0.14.31 · 감사 2026-09-06 에러4 원천봉쇄) 폴더 신뢰 사전 주입 핀.

`javis_preflight.py --seed-trust --config <acctdir> --cwd <cwd>` 와 C58 스코프(레지스트리 쌍 판정)·cys-dept 3지점
배선을 test_dept_creds_seed.py 동형(격리 HOME · 실물 추출 · 배선 완주)으로 단언한다. 라이브 무접촉: 모든 config dir·
cwd·레지스트리는 임시 디렉터리다(실 ~/.cys 계정 dir 은 절대 대상이 아니다 — 절대경로 인자로만 동작).

  1) 부재: .claude.json 없음 → 정확히 {"projects":{<key>:{"hasTrustDialogAccepted":true}}} 생성 · 0600 · 2회차 멱등(무쓰기)
  2) 부분: 다른 최상위 키·다른 항목·hasCompletedOnboarding=false 보존 · projects 부재 생성 · 동일성 같은 기존 키 재사용
     (중복 0) · 손상 JSON/심링크 = ERROR 무쓰기
  3) 라이브 프로세스: 그 CLAUDE_CONFIG_DIR 로 도는 claude 실행 형상 존재 → REFUSE(rc 2) 무쓰기 · 종료 후 OK ·
     같은 env 의 비-claude(python) 자식은 계수 0 · 검증 불가(None) → REFUSE, --force-unverified 만 통과
  4) 동시 변경: 다른 프로세스가 잠금 보유 → REFUSE lock-busy · 읽기~쓰기 사이 파일 변경 → REFUSE concurrent-change
     (상대 내용 생존) · 잠금 기구 미가용 → REFUSE lock-unavailable
  5) C58 레지스트리: 본부·부서 topology 쌍 판독(config 부재 항목 추정 귀속 0) · _is_cysjavis_workspace 가 CLAUDE.md/_round
     없이 쌍 판정 · 갭 = 항목 부재 포함 · report 모드 무쓰기 · --fix 는 seed_trust 경로(.bak-preflight 1회)
  6) cys-dept 배선: 3지점(launch/allocate/create)에서 seed_trust_acct 가 데몬 스폰·빈 셸 生成 앞 · 4 데몬 라인 env -u 접두 ·
     launch 완주(Windows 목·재사용 경로)에서 fork 계정 dir 에 .claude.json 착지(cwd=$HOME 한 쌍)
  7) codex(gpt-6-astra) 적대 반례(R2 · 워커가 전 행 검토 후 채택): 손상 형상 ERROR 무쓰기(명시 null 포함) · 파일/디렉터리 충돌 ·
     거부 경로는 디렉터리·파일 무생성 · 되읽기 실패 시 커밋 취소(롤백) · 유니코드/정규화/cwd=config · truthy 비-bool → 정확히 True ·
     ps '=' 값·접미 키·env 래퍼 · Windows 출력 변형·필터 문자열 · 손상 레지스트리·named pipe 폴백·귀속 0 · 심링크 별칭 동일성 ·
     셸 정의 순서·인자 수·opt-in 블록 밖

    CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" CYS_PROBE_RUNS="$JAVIS_ROOT/probe_runs.jsonl" \\
      python3 cysjavis-pack/bin/tests/test_trust_seed.py
"""
import builtins
import copy
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
from unittest.mock import patch

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PF = os.path.join(BIN, "javis_preflight.py")
DEPT = os.path.join(BIN, "cys-dept")
sys.path.insert(0, BIN)
import javis_preflight as pf  # noqa: E402

PY = sys.executable or "python3"


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

    def test_2c_equivalent_existing_key_reused_no_duplicate(self):
        alias = self.key + "/"          # 동일성 같은 다른 문자열 키
        _write(self.cfgfile, json.dumps({"projects": {alias: {"hasTrustDialogAccepted": False, "k": 1}}}))
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, err)
        got = _read_json(self.cfgfile)
        self.assertEqual(list(got["projects"]), [alias], "동일성 같은 키가 있는데 새 키를 만들었다(중복)")
        self.assertEqual(got["projects"][alias], {"hasTrustDialogAccepted": True, "k": 1})

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


class RegistryC58(unittest.TestCase):
    """C58 스코프 — 레지스트리 쌍 판정(임시 HOME · 격리 차단 우회는 판독기에 한정 monkeypatch)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustc58-")
        self.home = os.path.join(self.tmp, "home")
        self._saved = {k: os.environ.get(k) for k in
                       ("HOME", "CYS_DEPTS_JSON", "CYS_ACCOUNT_DIR", "CLAUDE_CONFIG_DIR", "CYS_PACK_DIR")}
        os.environ["HOME"] = self.home
        os.environ["CYS_DEPTS_JSON"] = os.path.join(self.home, ".cys", "depts.json")
        os.environ.pop("CYS_ACCOUNT_DIR", None)
        os.environ.pop("CLAUDE_CONFIG_DIR", None)
        # 격리 차단(임시 팩 컨텍스트 → 빈 레지스트리)은 실 config 보호 장치 — 여기선 HOME 자체가 임시라 판독기만 통과시킨다.
        self._iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)
        self.cfgA = os.path.join(self.home, ".cys", "claude")
        self.cfgB = os.path.join(self.home, ".cys", "claude-default-d1")
        self.X = os.path.join(self.home, "wsX")
        self.Y = os.path.join(self.home, "wsY")
        self.X2 = os.path.join(self.home, "wsX2")
        self.stale = os.path.join(self.home, "gone")
        for d in (self.cfgA, self.cfgB, self.X, self.Y, self.X2,
                  os.path.join(self.home, ".local", "state", "cys"),
                  os.path.join(self.home, ".local", "state", "cys-dept-d1"),
                  os.path.join(self.home, ".cys", "state")):
            os.makedirs(d, exist_ok=True)
        _write(os.path.join(self.home, ".local", "state", "cys", "topology.json"), json.dumps({
            "entries": [
                {"role": "master", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": self.X},
                {"role": "worker", "agent": "claude", "cwd": self.Y},          # config 부재 → 추정 귀속 금지
                {"role": "cso", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": self.stale},
            ]}))
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {"d1": {
            "socket": os.path.join(self.home, ".local", "state", "cys-dept-d1", "cys.sock"),
            "pack_dir": os.path.join(self.home, ".cys", "pack-dept-d1"), "role": "dept-master",
            "account_dir": self.cfgB}}}))
        _write(os.path.join(self.home, ".local", "state", "cys-dept-d1", "topology.json"), json.dumps({
            "entries": [
                {"role": "worker", "agent": "claude", "claude_config_dir": self.cfgB, "cwd": self.Y},
                {"role": "reviewer-codex", "agent": "codex", "claude_config_dir": self.cfgB, "cwd": self.X2},
            ]}))
        _write(os.path.join(self.home, ".cys", "state", "mission.json"),
               json.dumps({"schema": 1, "mission": "x", "surface": "109"}))   # cwd 없음(0.14.30 스키마)

    def tearDown(self):
        pf._discover_isolation_block = self._iso
        for k, v in self._saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def _pf(self, fix):
        return pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report", allow_irreversible=False)

    def test_5_registry_pairs_precise(self):
        reg = pf.cysjavis_registry()
        idA, idB = pf._path_identity(self.cfgA), pf._path_identity(self.cfgB)
        self.assertEqual(set(reg["pairs"][idA]), {pf._path_identity(self.X), pf._path_identity(self.stale)})
        self.assertEqual(set(reg["pairs"][idB]), {pf._path_identity(self.Y), pf._path_identity(self.X2)})
        self.assertNotIn(pf._path_identity(self.Y), reg["pairs"][idA], "config 부재 항목이 본부 config 에 추정 귀속됐다")
        self.assertEqual(set(reg["configs"]), {idA, idB})
        self.assertEqual(len(reg["sources"]), 2, reg["sources"])

    def test_5b_is_workspace_pair_scoped_without_markers(self):
        p = self._pf(fix=False)
        self.assertFalse(os.path.exists(os.path.join(self.X, "CLAUDE.md")))
        self.assertFalse(os.path.isdir(os.path.join(self.X, "_round")))
        self.assertTrue(p._is_cysjavis_workspace(self.X, self.cfgA))
        self.assertFalse(p._is_cysjavis_workspace(self.Y, self.cfgA), "부서 cwd 가 본부 config 쌍으로 인정됐다(합집합 살포)")
        self.assertTrue(p._is_cysjavis_workspace(self.Y, self.cfgB))
        self.assertTrue(p._is_cysjavis_workspace(self.X))
        self.assertFalse(p._is_cysjavis_workspace(self.stale, self.cfgA), "dir 부재(stale) 경로가 인정됐다")
        self.assertFalse(p._is_cysjavis_workspace(os.path.join(self.home, "random"), self.cfgA))

    def test_5c_gaps_include_missing_entry_and_missing_file(self):
        _write(os.path.join(self.cfgB, ".claude.json"),
               json.dumps({"projects": {self.Y: {"hasTrustDialogAccepted": True}}}))
        p = self._pf(fix=False)
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgB, ".claude.json")), [self.X2])
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgA, ".claude.json")), [self.X],
                         "파일 부재 config 의 등재 cwd 가 갭으로 잡히지 않았다(stale 은 제외)")

    def test_5d_report_mode_read_only_then_fix_via_seed(self):
        bfile = os.path.join(self.cfgB, ".claude.json")
        _write(bfile, json.dumps({"hasCompletedOnboarding": True,
                                  "projects": {self.Y: {"hasTrustDialogAccepted": True}}}, indent=2))
        raw = _read_bytes(bfile)
        p = self._pf(fix=False)
        p.c58_trust_harden()
        r = [x for x in p.results if x["id"] == "C58.trust-harden"]
        self.assertEqual(len(r), 1)
        self.assertEqual(r[0]["status"], pf.WARN, r)
        self.assertIn("trust gap", r[0]["detail"])
        self.assertEqual(_read_bytes(bfile), raw, "report 모드가 파일을 썼다")
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json")))
        p = self._pf(fix=True)
        p.c58_trust_harden()
        r = [x for x in p.results if x["id"] == "C58.trust-harden"][0]
        self.assertEqual(r["status"], pf.FIXED, r)
        gotB = _read_json(bfile)
        self.assertIs(gotB["hasCompletedOnboarding"], True)
        self.assertEqual(gotB["projects"][pf.claude_project_key(self.X2)], {"hasTrustDialogAccepted": True})
        self.assertTrue(os.path.exists(bfile + ".bak-preflight"), "C58 --fix 백업 1회 계약 소실")
        gotA = _read_json(os.path.join(self.cfgA, ".claude.json"))
        self.assertEqual(gotA, {"projects": {pf.claude_project_key(self.X): {"hasTrustDialogAccepted": True}}})
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json.bak-preflight")), "부재 파일에 백업을 만들었다")
        p = self._pf(fix=False)
        p.c58_trust_harden()
        r = [x for x in p.results if x["id"] == "C58.trust-harden"][0]
        self.assertEqual(r["status"], pf.PASS, r)

    def test_5e_fix_refused_when_live_claude(self):
        p = self._pf(fix=True)
        saved = pf.claude_procs_for_config
        pf.claude_procs_for_config = lambda d, **k: (1, "fake-live")
        try:
            p.c58_trust_harden()
        finally:
            pf.claude_procs_for_config = saved
        r = [x for x in p.results if x["id"] == "C58.trust-harden"][0]
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("REFUSE live-claude", r["detail"])
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json")))
        self.assertFalse(os.path.exists(os.path.join(self.cfgB, ".claude.json")))


def _block(src, start_label, end_label):
    i = src.find("\n  %s)" % start_label)
    assert i > 0, start_label
    j = src.find("\n  %s)" % end_label, i)
    assert j > i, end_label
    return src[i:j]


class DeptWiringStatic(unittest.TestCase):
    def setUp(self):
        self.src = _read_text(DEPT)

    def test_6_three_sites_seed_before_daemon_and_shell(self):
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
        launch = _block(self.src, "launch", "allocate")
        self.assertLess(launch.find("seed_credentials_win"), launch.find("seed_trust_acct"),
                        "launch: creds 시드 바로 뒤 동형 배치가 아니다")
        self.assertNotIn("CYS_DEPT_SEED_CREDS", launch[launch.find("seed_trust_acct") - 40:launch.find("seed_trust_acct")],
                         "launch: 신뢰 시드가 creds opt-in 블록 안에 갇혔다(기본 off = 시드 0)")
        self.assertIn('seed_trust_acct "$acctdir" "${CYS_DEPT_CWD:-$HOME}"', launch)
        self.assertIn('seed_trust_acct "$acctdir" "${CYS_DEPT_CWD:-$HOME}"', _block(self.src, "allocate", "create"))
        self.assertIn('seed_trust_acct "$acctdir" "$cwd"', _block(self.src, "create", "down"))

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

    def test_6c_daemon_lines_env_u_prefix(self):
        lines = [l for l in self.src.splitlines() if 'nohup "$CYSD"' in l]
        self.assertEqual(len(lines), 4, lines)
        for l in lines:
            self.assertIn("env -u CYS_ROLE -u CYS_SURFACE_ID -u CYS_SURFACE_REF -u CYS_SEAT_TOKEN nohup", l, l)
            self.assertNotIn("CYS_SOCKET=", l.split("env -u", 1)[1], "env -u 뒤에 대입이 남아 있다")


class DeptLaunchWiring(unittest.TestCase):
    """launch 재사용 경로 완주(test_dept_creds_seed.LaunchWiring 동형 · Windows uname 목) — fork 계정 dir 에
    .claude.json 착지(cwd=$HOME 한 쌍). 팩 bin 은 repo bin 심링크(도구 해석 경로 ${CYS_PACK_DIR:-$PACK_DEFAULT}/bin)."""

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
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {self.NAME: {
            "socket": r"\\.\pipe\cys-dept-%s" % self.NAME,
            "pack_dir": os.path.join(self.home, ".cys", "pack-dept-%s" % self.NAME),
            "role": "dept-master", "account_dir": self.fork}}}))
        packdir = os.path.join(self.home, ".cys", "pack")
        os.makedirs(packdir)
        _write(os.path.join(packdir, "agents.json"),
               json.dumps({"claude": {"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": self.base}}}))
        os.symlink(BIN, os.path.join(packdir, "bin"))

    def env(self, **extra):
        env = dict(os.environ)
        env.update({"HOME": self.home,
                    "CYS_DEPTS_JSON": os.path.join(self.home, ".cys", "depts.json"),
                    "PATH": self.bindir + os.pathsep + env.get("PATH", "")})
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR", "CYS_NO_AUTOSTART",
                  "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG", "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT",
                  "CYS_DEPT_SEED_CREDS", "CYS_DEPT_CWD", "CLAUDE_CONFIG_DIR", "CYS_SURFACE_ID",
                  "CYS_SURFACE_REF", "CYS_SEAT_TOKEN"):
            env.pop(k, None)
        env.update(extra)
        return env

    def test_7_launch_seeds_fork_acct_with_home_pair(self):
        r = subprocess.run(["bash", DEPT, "launch", self.NAME], capture_output=True, text=True,
                           encoding="utf-8", env=self.env(), cwd=self.tmp, timeout=120)
        self.assertEqual(r.returncode, 0, r.stderr)
        cfgfile = os.path.join(self.fork, ".claude.json")
        self.assertTrue(os.path.isfile(cfgfile), "launch 배선 미발동(fork 에 .claude.json 부재): %s" % r.stderr)
        key = pf.claude_project_key(self.home)
        self.assertEqual(_read_json(cfgfile), {"projects": {key: {"hasTrustDialogAccepted": True}}},
                         "fork 계정 dir 에 (acct, $HOME) 한 쌍만 있어야 한다")
        self.assertIn("seed-trust: OK seeded(", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.base, ".claude.json")), "base 계정 dir 이 오염됐다")
        # 2회차(rotate 재귀 재현): 멱등 · 여전히 rc 0
        r2 = subprocess.run(["bash", DEPT, "launch", self.NAME], capture_output=True, text=True,
                            encoding="utf-8", env=self.env(), cwd=self.tmp, timeout=120)
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertIn("already-trusted(", r2.stderr)

    def test_7b_launch_survives_seed_refusal(self):
        # 잠금을 다른 프로세스(테스트)가 보유 → 시드 REFUSE → launch 는 WARN 1줄로 계속(fail-open · 전 pane 0 금지)
        os.makedirs(self.fork, exist_ok=True)
        holder = open(os.path.join(self.fork, pf.SEED_TRUST_LOCK_NAME), "a+")
        self.addCleanup(holder.close)
        self.assertIs(pf._try_lock_nb(holder), True)
        r = subprocess.run(["bash", DEPT, "launch", self.NAME], capture_output=True, text=True,
                           encoding="utf-8", env=self.env(), cwd=self.tmp, timeout=120)
        self.assertEqual(r.returncode, 0, "시드 거부가 부서 기동을 죽였다: %s" % r.stderr)
        self.assertIn("신뢰 사전 주입 보류", r.stderr)
        self.assertIn("lock-busy", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.fork, ".claude.json")))
        self.assertIn("가동 완료", r.stdout + r.stderr, "launch 가 완주하지 않았다")


# ══ codex(gpt-6-astra) R2 적대 반례 — 워커가 전 행 검토·수정 후 채택(초안 _codex_trust_counterexamples_draft.py 는 폐기) ══
# 채택 시 바뀐 것: 명시 null 항목 → ERROR(종전 대체) · 프로세스 확인이 makedirs 앞(거부 경로 무생성) · 교체 前 임시파일 되읽기 +
# 교체 後 되읽기 실패 시 원본 롤백. 기각: 'env 가 argv 앞에 오는 ps 줄'(ps -E 는 argv→env 고정 · 파서가 argv 시작을 추측하면 안 된다).
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
            "HOME": self.tmp, "CYS_DEPTS_JSON": os.path.join(self.tmp, "depts.json"),
            "CYS_ACCOUNT_DIR": self.cfg, "CLAUDE_CONFIG_DIR": self.cfg})
        self.env.start()
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
        """a: 명시 null 은 '있는 비-object' — 대체 허가가 아니다(ERROR · 무쓰기)."""
        self.lock_fixture()
        cases = [[], {"projects": []}]
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

    def test_a_readback_error_rolls_back_commit(self):
        """a: 교체 後 되읽기 IO 실패만 주입(교체는 실제) → ERROR · 파일은 들어온 그대로(롤백)."""
        self.lock_fixture()
        for exists in (False, True):
            with self.subTest(existing=exists):
                if os.path.exists(self.file):
                    os.unlink(self.file)
                if exists:
                    _write_any(self.file, {"theme": "dark"})
                before = _snapshot(self.tmp)
                real_replace, real_open = os.replace, builtins.open
                committed = []

                def replace(src, dst):
                    real_replace(src, dst)
                    committed.append(dst)

                def open_readback(path, *args, **kwargs):
                    if committed and path == self.file and "r" in (args[0] if args else kwargs.get("mode", "r")):
                        raise OSError("injected readback failure")
                    return real_open(path, *args, **kwargs)

                with patch.object(pf.os, "replace", replace), patch("builtins.open", open_readback):
                    result = self.seed()
                self.assertEqual(result[:2], (1, "ERROR"), result)
                self.assertIn("롤백", result[2])
                self.assertEqual(_snapshot(self.tmp), before, result)

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
                self.assertEqual(result[:2], (2, "REFUSE"), result)
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
        """c: '=' 는 값의 일부 · 접미 키(XCLAUDE_CONFIG_DIR)는 비매칭 · env 래퍼 줄은 0/1 어느 쪽도 허용(exec 된 claude 가 자기
        줄을 가지며, 래퍼를 세는 쪽은 보수적 = 거부 방향)."""
        target = os.path.join(self.tmp, "config=a=b")
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude CLAUDE_CONFIG_DIR=%s OTHER=x" % target], target), (1, 1))
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude XCLAUDE_CONFIG_DIR=%s" % target], target), (0, 1))
        count, parsed = pf._count_claude_in_ps_lines(
            ["424242 env CLAUDE_CONFIG_DIR=%s claude" % target], target)
        self.assertEqual(parsed, 1)
        self.assertIn(count, (0, 1))

    def test_d_windows_output_and_process_filter(self):
        """d: 파싱된 전역 0 만 부재 증명(CRLF·공백 허용) · 빈/비숫자/≥1 은 None · 필터 문자열이 자기 pid·$PID·이름 4종을 담는다."""
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

    def test_e_malformed_registry_and_pipe_fallback_do_not_infer_pairs(self):
        """e: 손상 레코드 혼재 · named pipe 소켓 → ~/.local/state/cys-dept-x 폴백 · account_dir 로 쌍이 전파되지 않는다."""
        other = os.path.join(self.tmp, "other-config")
        topo = os.path.join(self.tmp, ".local", "state", "cys-dept-x", "topology.json")
        _write_any(os.environ["CYS_DEPTS_JSON"], {"depts": {
            "null": None, "string": "bad", "missing": {"account_dir": other},
            "x": {"socket": r"\\.\pipe\cys-dept-x", "account_dir": other}}})
        bad_entries = [None, "bad"] + [{"claude_config_dir": self.cfg, "cwd": v} for v in (None, 3, [], {})]
        bad_entries += [{"cwd": self.ws}, {"claude_config_dir": 3, "cwd": self.ws}]
        for payload, expected in (([], {}), (b"{broken", {}),
                                  ({"entries": bad_entries + [{"claude_config_dir": self.cfg, "cwd": self.ws}]},
                                   {pf._path_identity(self.cfg): {pf._path_identity(self.ws): self.ws}})):
            with self.subTest(payload=payload):
                _write_any(topo, payload)
                reg = pf.cysjavis_registry()
                self.assertEqual(reg["pairs"], expected)
                self.assertNotIn(pf._path_identity(other), reg["pairs"])
                self.assertIn(pf._path_identity(other), reg["configs"])
                self.assertEqual(reg["sources"], [topo] if expected else [])

    def test_f_symlink_trust_identity_and_seed_reuse(self):
        """f: C58 는 심링크 별칭 키를 동일성으로 신뢰로 인정 · false→true 는 그 별칭 키를 재사용(중복 0)."""
        alias = os.path.join(self.tmp, "alias")
        os.symlink(self.ws, alias)
        _write_any(os.path.join(self.tmp, ".local", "state", "cys", "topology.json"),
                   {"entries": [{"claude_config_dir": self.cfg, "cwd": self.ws}]})
        p = pf.Preflight(fix=False, skips=[], mode="report", allow_irreversible=False)
        _write_any(self.file, {"projects": {alias: {"hasTrustDialogAccepted": True, "keep": [1]}}})
        before = _snapshot(self.tmp)
        self.assertEqual(p._trust_gap_workspaces(self.file), [])
        self.assertEqual(_snapshot(self.tmp), before)
        _write_any(self.file, {"projects": {alias: {"hasTrustDialogAccepted": False, "keep": [1]}}})
        self.assertEqual(p._trust_gap_workspaces(self.file), [self.ws])
        self.assertEqual(self.seed()[:2], (0, "OK"))
        self.assertEqual(_read_json(self.file), {"projects": {alias: {"hasTrustDialogAccepted": True, "keep": [1]}}})
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
