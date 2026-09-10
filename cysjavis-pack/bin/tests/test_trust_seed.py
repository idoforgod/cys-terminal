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
     OK · 아니면 REFUSE post-commit · 되읽기 실패 = ERROR 커밋 유지) · ★R2 대조~커밋 사이 기록자 = 원자 교환이 드러내 REFUSE
     (상대 바이트 보존 · 종전 '알려진 한계' 핀 폐기) · ★R3 교환 기구 부재 = REFUSE exchange-unavailable(강행 불가 · os.replace 0) ·
     부재 파일 = link 만(경합 REFUSE · 하드링크 실패 = REFUSE link-failed · O_EXCL 폴백 0) · displaced 지문 청소 · lone surrogate
  4') ★R2 프로브는 기존 문서가 있을 때만(부재 파일 = 무프로브 · Windows 신규 부서 경로) · env 비노출 tail/less 는 형상 아님 ·
     ★R3 darwin 프로브 = ps 2회(argv 접두 대조로 env 경계 확정 · 구분자 없는 모드는 claude 형상에 검증된 0 을 주지 않는다)
  5) C58 레지스트리: 본부·부서 topology 쌍 판독(config 부재 항목 추정 귀속 0 · ★R1 agent=claude 만 · depts.json (account_dir,
     cwd) 쌍 · 절대경로만 · ★R2 entries 형상 이상 = 판독불가) · 워크스페이스 판정은 _trust_gap_workspaces 하나(마커 0 · 정확 키 ·
     ★R2 _is_cysjavis_workspace 삭제) · 갭 = 항목 부재·별칭 true 포함 ·
     report 모드 무쓰기 · --fix 는 seed_trust 경로(.bak-preflight 1회) · ★R1 쌍 0 → SKIP(PASS 아님)
  5') ★R1 격리 컨텍스트(실물 _discover_isolation_block): 부서 컨텍스트 = 자기 계정 config 만 판정·수리(타 계정 무접촉 스파이) ·
     계정 미상 부서/임시 팩 = SKIP · Windows state 경로(%LOCALAPPDATA%\\cys\\<pipe_slug> · LOCALAPPDATA 부재 = 판독불가 고지)
  6) cys-dept 배선: 3지점(launch/allocate/create)에서 seed_trust_acct 가 데몬 스폰·빈 셸 生成 앞 · 4 데몬 라인 env -u 접두 ·
     ★R1 resolve_dept_cwd 로 확정한 **같은 cwd** 가 시드·빈 셸·formation_ensure_async(--cwd) 에 전달 · launch 완주(Windows 목·
     재사용 경로)에서 fork 계정 dir 에 .claude.json 착지 + formation 스텁 argv 실측(호출자 cwd≠HOME · CYS_DEPT_CWD · 등재 cwd · "/")
  7) codex(gpt-6-astra) 적대 반례(R2 · 워커가 전 행 검토 후 채택) + R1 재검토분 + R3
  ★R3 플랫폼 계약: 기존 문서의 성공 경로는 원자 교환이 있는 FS 에서만 실행된다(_require_exchange · 없으면 정직 skip) · 교환 부재
     거부/오류/청소/판정 검체는 어디서나 실행(교환을 None/_ExchangeUnavailable 로 주입).

    CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" CYS_PROBE_RUNS="$JAVIS_ROOT/probe_runs.jsonl" \\
      python3 cysjavis-pack/bin/tests/test_trust_seed.py
"""
import builtins
import contextlib
import copy
import errno
import hashlib
import inspect
import io
import json
import math
import os
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
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


def _write_bytes(path, raw):
    """바이트 그대로 쓴다 — 0바이트·공백·비 UTF-8 활성 문서 반례용(★R6)."""
    with open(path, "wb") as f:
        f.write(raw)


def _read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def _read_text(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


_STUB_PS_DIR = [None]


def _stub_ps_dir():
    """darwin 전용 결정론 프로브: PATH 앞에 `ps` 스텁(claude 형상 0 · 파싱 1줄) — 파일 수준 CLI 검체가 호스트 프로세스 군에
    의존하지 않게(codex R2). linux(/proc)·nt(powershell)는 스텁 대상이 아니다(그 플랫폼의 CLI 기존-파일 검체는 in-process 주입)."""
    if _STUB_PS_DIR[0] is None:
        d = tempfile.mkdtemp(prefix="trustseed-ps-")
        _write(os.path.join(d, "ps"), '#!/bin/sh\necho "    1 /sbin/launchd"\n', 0o755)
        _STUB_PS_DIR[0] = d
    return _STUB_PS_DIR[0]


def seed_cli(config, cwd, *extra, env=None, stub_ps=True):
    """subprocess 실물 호출 — (rc, stdout, stderr). 격리 env(호출자의 os.environ 이 Base 에서 격리돼 있다) · darwin 은 stub ps."""
    e = dict(os.environ)
    e.pop("CLAUDE_CONFIG_DIR", None)
    if stub_ps and sys.platform == "darwin":
        e["PATH"] = _stub_ps_dir() + os.pathsep + e.get("PATH", "")
    if env:
        e.update(env)
    # ★R5(리뷰 codex minor): 자식 파이썬은 전부 `-B` 로 봉인한다(SEAL-1) — 부모의 -B 는 상속되지 않고,
    #   R4 에서 봉인 env 대입을 지우며 이 헬퍼가 물려받던 봉인이 함께 사라졌다(그 변수명을 여기 적지 않는 이유는
    #   봉인 census ⓑ 의 참조 파일 집합을 늘리지 않기 위해서다 — 검증은 `R5TestChildSeal` 이 행위로 한다).
    r = subprocess.run([PY, "-B", PF, "--seed-trust", "--config", config, "--cwd", cwd, *extra],
                       capture_output=True, text=True, encoding="utf-8", env=e, timeout=60)
    return r.returncode, r.stdout, r.stderr


def _no_probe(d):
    raise AssertionError("프로세스 프로브가 호출됐다(이미 신뢰 · 부재 문서 경로는 무프로브 — R1/R2 위반)")


def _verified_zero(d):
    return 0, "injected-zero"


_EXCHANGE_OK = [None]


def _exchange_supported():
    """실물 교환 가능 여부(1회 캐시) — 픽스처와 같은 tmp FS 에서 두 파일을 실제로 맞바꿔 본다(codex R3: 함수 반환값만 믿지 않고
    바이트가 실제로 바뀌었는지 확인 · 예외는 실패로 올린다)."""
    if _EXCHANGE_OK[0] is None:
        d = tempfile.mkdtemp(prefix="trustseed-xchg-")
        a, b = os.path.join(d, "a"), os.path.join(d, "b")
        _write(a, "A")
        _write(b, "B")
        r = pf._exchange_paths(a, b)
        ok = r is True and _read_text(a) == "B" and _read_text(b) == "A"
        if r is True and not ok:
            raise AssertionError("_exchange_paths 가 True 를 돌려줬는데 바이트가 안 바뀌었다")
        _EXCHANGE_OK[0] = ok
    return _EXCHANGE_OK[0]


def _require_exchange(tc):
    """기존 문서 커밋(교환) 성공을 전제하는 검체의 정직 skip(Windows · 교환 미지원 FS). 거부/오류 검체엔 쓰지 않는다."""
    if not _exchange_supported():
        tc.skipTest("이 플랫폼/FS 엔 원자 교환 기구가 없다(기존 문서 커밋 = REFUSE exchange-unavailable 이 계약)")


def _refused_without_exchange(tc, result, path, before):
    """★R6(리뷰 codex major#5): **기존 문서** 커밋을 무조건 성공으로 단언하던 검체의 능력 분기.
    교환 기구가 있으면 False 를 돌려 호출자가 성공 경로를 그대로 단언하게 한다(지원 플랫폼의 잘못된 REFUSE 는
    여전히 잡힌다 — 분기 근거는 **결과가 아니라 능력**이다 · codex R6). 없으면 Windows 계약을 **양성으로** 단언한다:
    `REFUSE exchange-unavailable` · 문서 바이트 불변 · 공개 전 잔재(mkstemp·displaced·저널) 0.
    백업은 **교환 전에** 만들어지므로 여기서 '백업 0' 을 보편 계약으로 삼지 않는다(codex R6) — 백업 단언은 호출자가.

    전수 감사 방법(재현): `pf._exchange_paths` 를 `_ExchangeUnavailable("platform:nt")` 로 강제하고 이 파일 전체를
    돌린다 → 능력 분기가 빠진 검체만 실패한다(2026-09-07 09:2x 실측 13건 → 이 라운드에서 전부 분기)."""
    if _exchange_supported():
        return False
    tc.assertEqual(result[:2], (2, "REFUSE"), result)
    tc.assertIn("exchange-unavailable", result[2])
    tc.assertEqual(_read_bytes(path), before, "거부인데 문서 바이트가 바뀌었다")
    cfg_dir = os.path.dirname(path)
    litter = [n for n in os.listdir(cfg_dir)
              if (n.startswith(pf.SEED_TRUST_TMP_PREFIX) and n != pf.SEED_TRUST_LOCK_NAME)
              or n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX) or n.startswith(pf.SEED_TRUST_INTENT_PREFIX)]
    tc.assertEqual(litter, [], "거부 경로가 공개 전 잔재를 남겼다")
    return True


def _ps2(env_text):
    """darwin 2회 ps 주입 러너(R3): -E 없는 호출엔 각 줄의 env 세그먼트를 벗긴 argv 줄(실 ps 의 접두 관계 재현)."""
    def runner(cmd):
        if "-E" in cmd:
            return 0, env_text, ""
        lines = []
        for l in env_text.splitlines():
            parts = l.split(None, 1)
            if len(parts) == 2:
                lines.append(parts[0] + " " + pf._PS_ENV_SPLIT_RE.split(parts[1])[0])
        return 0, "\n".join(lines), ""
    return runner


class Base(unittest.TestCase):
    """파일 수준 검체 공통 — HOME/USERPROFILE/LOCALAPPDATA/XDG/CYS_*/CLAUDE_CONFIG_DIR/JAVIS_ROOT 전부 tmp 로 격리(codex R2:
    seed_cli 가 호스트 env 를 상속하던 것)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trustseed-")
        self.home = os.path.join(self.tmp, "home")
        os.makedirs(self.home)
        iso = {k: self.home for k in _HOME_KEYS}
        iso.update({"LOCALAPPDATA": os.path.join(self.home, "LA"), "XDG_STATE_HOME": os.path.join(self.home, "xdg"),
                    "CYS_DEPTS_JSON": os.path.join(self.home, ".cys", "depts.json"), "JAVIS_ROOT": os.path.join(self.home, "jr")})
        self._envp = patch.dict(os.environ, iso)
        self._envp.start()
        for k in ("CYS_ACCOUNT_DIR", "CLAUDE_CONFIG_DIR", "CYS_PACK_DIR", "CYS_SOCKET", "CYS_PROBE_RUNS"):
            os.environ.pop(k, None)
        self.addCleanup(self._envp.stop)
        self.cfg = os.path.join(self.tmp, "acct")
        self.ws = os.path.join(self.tmp, "ws")
        os.makedirs(self.ws)
        self.key = pf.claude_project_key(self.ws)
        self.cfgfile = os.path.join(self.cfg, ".claude.json")

    def seed(self, **kw):
        """in-process · 프로브 주입(기본 verified-zero) — 파일 수준 계약은 프로세스 군과 무관해야 한다."""
        kw.setdefault("proc_counter", _verified_zero)
        return pf.seed_trust(self.cfg, self.ws, **kw)

    def untrusted_file(self, text='{"projects": {}}'):
        os.makedirs(self.cfg, exist_ok=True)
        _write(self.cfgfile, text)


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
        r = subprocess.run([PY, "-B", PF, "--seed-trust", "--config", self.cfg],
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 1, "usage 오류는 rc 1(REFUSE=2 와 충돌 금지)")

    def test_1c_nonexistent_cwd_is_error_no_write(self):
        """★R2(리뷰): 부재 cwd·정규 파일 cwd·깨진 심링크 cwd 는 ERROR(stale 경로 무변경) — 잠금·dir 생성·계획 앞."""
        gone = os.path.join(self.tmp, "nonexistent-ws")
        rc, out, err = seed_cli(self.cfg, gone, "--json")
        self.assertEqual(rc, 1, out + err)
        self.assertIn("cwd 가 존재하는 디렉터리가 아니다", json.loads(out)["reason"])
        self.assertFalse(os.path.exists(self.cfg), "부재 cwd 거부인데 config dir 을 만들었다")
        f = os.path.join(self.tmp, "file-as-cwd")
        _write(f, "x")
        self.assertEqual(pf.seed_trust(self.cfg, f, proc_counter=_no_probe)[:2], (1, "ERROR"))
        dangling = os.path.join(self.tmp, "dangling")
        os.symlink(gone, dangling)
        self.assertEqual(pf.seed_trust(self.cfg, dangling, proc_counter=_no_probe)[:2], (1, "ERROR"))
        self.assertFalse(os.path.exists(self.cfg))

    def test_1d_absent_file_skips_process_probe(self):
        """★R2(리뷰 Windows major · 원칙): 프로브가 지키는 것은 라이브 claude 가 메모리에 든 **기존** 문서다 — 부재 파일은 프로브
        없이 link 로 만든다(Windows hub 좌석 아래 node/claude 상존 → 전역 ≥1 → 종전엔 모든 신규 부서 시드가 REFUSE unverified).
        기존 문서 + 플래그 부재는 여전히 프로브(unverified → REFUSE · 강행만 통과)."""
        rc, verdict, reason = self.seed(proc_counter=_no_probe)                       # 부재 dir + 부재 파일
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("no-probe(.claude.json 부재", reason)
        self.assertIn("commit=link", reason)
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        os.unlink(self.cfgfile)                                                         # 기존 dir + 부재 파일도 무프로브
        self.assertEqual(self.seed(proc_counter=_no_probe)[:2], (0, "OK"))
        windows_like = lambda d: (None, "windows: claude/node 프로세스 3 — config dir 귀속 불가")
        self.untrusted_file('{"projects": {}}')                                         # 기존 문서 · 플래그 부재 → 프로브 → REFUSE
        rc, verdict, reason = self.seed(proc_counter=windows_like)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("unverified", reason)
        self.assertEqual(_read_text(self.cfgfile), '{"projects": {}}', "거부인데 기존 문서가 바뀌었다")
        self.untrusted_file("")                                                          # 0B 도 '존재' → 프로브
        self.assertEqual(self.seed(proc_counter=windows_like)[:2], (2, "REFUSE"))
        # ★R4 재핀(리뷰 codex minor · 이 WP 자기 핀): 강행은 **프로브 단계만** 넘는다 — 기존 문서의 커밋은 원자 교환이
        #   있는 FS 에서만 성립하고, 교환 기구가 없는 플랫폼(Windows)에서는 강행해도 REFUSE exchange-unavailable 이
        #   계약이다(종전 무조건 OK 단언은 Windows 에서 결정론적 실패였다 · 바이트 불변까지 확인한다).
        rc, verdict, reason = self.seed(proc_counter=windows_like, force_unverified=True)
        if _exchange_supported():
            self.assertEqual((rc, verdict), (0, "OK"), reason)
            self.assertIn("force-unverified(", reason)
        else:
            self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
            self.assertIn("exchange-unavailable(", reason)
            self.assertEqual(_read_bytes(self.cfgfile), b"", "거부인데 기존 문서가 바뀌었다")

    def test_1e_cli_contracts_are_platform_independent(self):
        """CLI 판정 계약(OK already-trusted · REFUSE lock-busy · ERROR 손상 JSON)은 프로세스 군·플랫폼과 무관하게 결정론(codex R2)."""
        self.untrusted_file(json.dumps({"projects": {self.key: {"hasTrustDialogAccepted": True}}}))
        rc, out, _ = seed_cli(self.cfg, self.ws, env={"PATH": "/nonexistent"}, stub_ps=False)   # ps 없어도 무프로브
        self.assertEqual(rc, 0, out)
        self.assertIn("OK already-trusted(", out)
        self.untrusted_file("{broken")
        rc, out, _ = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 1, out)
        self.assertIn("ERROR 파싱 실패", out)
        self.untrusted_file('{"projects": {}}')
        holder = open(os.path.join(self.cfg, pf.SEED_TRUST_LOCK_NAME), "a+")
        self.addCleanup(holder.close)
        self.assertIs(pf._try_lock_nb(holder), True)
        rc, out, _ = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 2, out)
        self.assertIn("REFUSE lock-busy", out)


class Partial(Base):
    """기존 문서 부분 갱신 — in-process + 프로브 주입(verified-zero). CLI 경로는 test_2h(darwin stub ps) 하나로 대표."""

    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)

    def test_2_partial_only_target_key_touched(self):
        _require_exchange(self)
        base = {"hasCompletedOnboarding": False, "theme": "dark", "numStartups": 3,
                "projects": {"/somewhere/else": {"hasTrustDialogAccepted": False, "allowedTools": []}}}
        _write(self.cfgfile, json.dumps(base, indent=2), 0o644)
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertIn("commit=", reason)
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
        _require_exchange(self)
        _write(self.cfgfile, '{"hasCompletedOnboarding": true}')
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertEqual(_read_json(self.cfgfile),
                         {"hasCompletedOnboarding": True,
                          "projects": {self.key: {"hasTrustDialogAccepted": True}}})

    def test_2c_alias_key_untouched_exact_key_created(self):
        """★R1(codex): claude 는 projects[getcwd()] 정확 키만 읽는다 — 별칭(꼬리 슬래시) 항목은 신뢰 판정에 쓰지도 손대지도 않는다."""
        _require_exchange(self)
        alias = self.key + "/"
        _write(self.cfgfile, json.dumps({"projects": {alias: {"hasTrustDialogAccepted": False, "k": 1}}}))
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        got = _read_json(self.cfgfile)
        self.assertEqual(got["projects"][alias], {"hasTrustDialogAccepted": False, "k": 1}, "별칭 항목이 변경됐다")
        self.assertEqual(got["projects"][self.key], {"hasTrustDialogAccepted": True}, "정확 키가 생성되지 않았다")

    def test_2c2_alias_true_exact_false_conflict_sets_exact(self):
        """codex R1 반례: {"/work/": true, "/work": false} — 종전 구현은 별칭을 골라 already-trusted 로 정확 키를 false 로 남겼다."""
        _require_exchange(self)
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
        rc, verdict, reason = self.seed(proc_counter=_no_probe)      # 구조 거부는 프로브 앞
        self.assertEqual(rc, 1)
        self.assertIn("파싱 실패", reason)
        self.assertEqual(_read_bytes(self.cfgfile), raw, "손상 파일을 건드렸다")

    def test_2e_symlink_refused(self):
        target = os.path.join(self.tmp, "real.json")
        _write(target, '{"projects": {}}')
        os.symlink(target, self.cfgfile)
        rc, verdict, reason = self.seed(proc_counter=_no_probe)
        self.assertEqual(rc, 1)
        self.assertIn("symlink", reason)
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
        _require_exchange(self)
        _write(self.cfgfile, "", 0o640)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"), backup=True)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        self.assertEqual(stat.S_IMODE(os.stat(self.cfgfile).st_mode), 0o640, "0B 기존 파일의 권한이 보존되지 않았다")
        bak = self.cfgfile + ".bak-preflight"
        self.assertTrue(os.path.isfile(bak), "0B 기존 파일 백업이 없다(부재 취급)")
        self.assertEqual(_read_bytes(bak), b"")
        self.assertEqual(stat.S_IMODE(os.stat(bak).st_mode), 0o640)

    def test_2h_cli_partial_with_stub_ps(self):
        """CLI 경로 기존-문서 갱신 1건(darwin stub ps · 실 프로브 코드 경로 통과 · 결정론). 다른 플랫폼은 in-process 검체가 담당."""
        if sys.platform != "darwin":
            self.skipTest("stub ps 는 darwin 프로브에만 해당(linux=/proc · nt=powershell)")
        _require_exchange(self)
        _write(self.cfgfile, '{"theme": "dark", "projects": {}}')
        rc, out, err = seed_cli(self.cfg, self.ws, "--json")
        self.assertEqual(rc, 0, out + err)
        j = json.loads(out)
        self.assertTrue(j["reason"].startswith("seeded("), j)
        self.assertIn("probe=darwin: ps -E 1줄(argv 대조 1)", j["reason"], "stub ps 가 아니라 호스트 ps 를 읽었다(R3: 2회 호출 · argv 대조 계수)")
        self.assertEqual(_read_json(self.cfgfile), {"theme": "dark", "projects": {self.key: {"hasTrustDialogAccepted": True}}})

    def test_2i_fifo_at_claude_json_is_refused_before_open(self):
        """codex R2: O_NOFOLLOW 는 FIFO 를 막지 않는다 — 열기 전 lstat S_ISREG 로 거르고(블로킹 0) ERROR."""
        if not hasattr(os, "mkfifo"):
            self.skipTest("mkfifo 부재(Windows)")
        os.mkfifo(self.cfgfile)
        rc, verdict, reason = self.seed(proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("정규 파일이 아니다", reason)
        self.assertTrue(stat.S_ISFIFO(os.lstat(self.cfgfile).st_mode), "FIFO 를 건드렸다")


class ProjectKeyOracle(Base):
    """정확 키의 **독립 오라클**(codex R2): 자식 프로세스가 실제로 chdir 한 뒤 getcwd()(libuv uv_cwd 와 같은 계열)로 본 문자열과
    claude_project_key 가 같아야 한다 — 생산 함수가 자기 기대값을 만드는 순환을 끊는다. node 가 있으면 process.cwd() 도 대조."""

    def _oracle_py(self, cwd):
        r = subprocess.run([PY, "-B", "-c", "import os,sys; os.chdir(sys.argv[1]); print(os.getcwd())", cwd],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.rstrip("\r\n")

    def _aliases(self):
        real = os.path.join(self.tmp, "real ws")           # 공백 포함
        os.makedirs(os.path.join(real, "child"))
        link = os.path.join(self.tmp, "alias-link")
        os.symlink(real, link)
        return real, [real, real + os.sep, os.path.join(real, "child", ".."), link, os.path.join(link, "child", "..")]

    def test_key_equals_child_getcwd(self):
        real, forms = self._aliases()
        seen = set()
        for form in forms:
            with self.subTest(form=form):
                oracle = self._oracle_py(form)
                self.assertEqual(pf.claude_project_key(form), oracle)
                seen.add(oracle)
        self.assertEqual(len(seen), 1, "같은 디렉터리의 표기들이 서로 다른 키가 됐다: %s" % seen)
        # 시드된 키가 곧 오라클 문자열 — 시드 후 그 키로 판독 가능
        self.assertEqual(self.seed()[0], 0)
        self.assertIn(self._oracle_py(self.ws), _read_json(self.cfgfile)["projects"])

    def test_key_equals_node_process_cwd(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("node 부재 — process.cwd() 오라클 생략(python getcwd 오라클은 test_key_equals_child_getcwd 가 실행)")
        real, forms = self._aliases()
        for form in forms:
            with self.subTest(form=form):
                r = subprocess.run([node, "-e", "process.stdout.write(process.cwd())"], cwd=form,
                                   capture_output=True, text=True, timeout=30)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(pf.claude_project_key(form), r.stdout)


def _env_visible_for_python_child():
    """darwin `ps -E` 는 Apple 플랫폼 바이너리(/bin/sh·sleep)의 env 를 숨긴다 — 픽스처는 사용자 python 으로 띄우되,
    그 python 조차 env 가 안 보이는 환경(Apple 제공 python 등)이면 라이브 케이스는 skip(정직)."""
    if sys.platform.startswith("linux"):
        return True
    if sys.platform != "darwin":
        return False
    probe = subprocess.Popen([PY, "-B", "-c", "import time; time.sleep(20)"],
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
        p = subprocess.Popen([PY, "-B", path], env=dict(os.environ, CLAUDE_CONFIG_DIR=env_cfg))
        self.addCleanup(lambda: (p.kill(), p.wait()))
        time.sleep(0.4)
        return p

    def test_3_live_claude_refuses_then_ok_after_exit(self):
        _require_exchange(self)
        if not _env_visible_for_python_child():
            self.skipTest("이 플랫폼/인터프리터에선 자식 env 가 ps 에 보이지 않는다(라이브 판정 실측 불가)")
        p = self._spawn("claude", self.cfg)          # argv basename 'claude' = claude 실행 형상
        self.untrusted_file('{"projects": {}}')        # ★R2: 프로브는 기존 문서가 있을 때 — 라이브 claude 가 든 문서를 재현
        rc, out, err = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 2, "라이브 claude 가 있는데 거부하지 않았다: %s %s" % (out, err))
        self.assertIn("REFUSE live-claude(n=1", out)
        self.assertEqual(_read_text(self.cfgfile), '{"projects": {}}', "거부인데 파일을 썼다")
        # 타 config 의 claude 는 계수 대상이 아니다(쌍 스코프)
        acct2 = os.path.join(self.tmp, "acct2")
        os.makedirs(acct2)
        _write(os.path.join(acct2, ".claude.json"), '{"projects": {}}')
        rc2, out2, _ = seed_cli(acct2, self.ws, stub_ps=False)
        self.assertEqual(rc2, 0, out2)
        p.kill()
        p.wait()
        time.sleep(0.2)
        rc, out, err = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 0, out + err)
        self.assertIn("OK seeded(", out)
        self.assertIn("commit=", out)

    def test_3b_non_claude_child_with_same_env_not_counted(self):
        _require_exchange(self)
        if not _env_visible_for_python_child():
            self.skipTest("자식 env 가 ps 에 보이지 않는다")
        self._spawn("mcp_server.py", self.cfg)       # 같은 env · claude 실행 형상 아님(MCP 자식 재현)
        count, detail = pf.claude_procs_for_config(self.cfg)
        self.assertEqual(count, 0, detail)
        self.untrusted_file('{"projects": {}}')
        rc, out, err = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 0, out + err)

    def test_3b2_hidden_env_tail_on_claude_named_file_is_not_claude(self):
        """★R2(리뷰 major · 실측 재현): `tail -f <dir>/logs/claude`(Apple 플랫폼 바이너리 = ps -E env 비노출) 1건이 함대 전체의
        시드를 REFUSE unverified 로 돌렸다. 인자 속 claude 토큰은 형상이 아니다 — 실 프로세스로 검증."""
        if sys.platform != "darwin":
            self.skipTest("darwin ps -E 전용 재현")
        logdir = os.path.join(self.tmp, "logs")
        os.makedirs(logdir)
        target = os.path.join(logdir, "claude")
        _write(target, "")
        p = subprocess.Popen(["tail", "-f", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: (p.kill(), p.wait()))
        time.sleep(0.4)
        rc, out, _e = pf._run_capture(["ps", "-ax", "-ww", "-E", "-o", "pid=,command="])
        mine = [l for l in out.splitlines() if l.split(None, 1)[0] == str(p.pid)]
        self.assertEqual(len(mine), 1, "tail 프로세스가 ps 에 없다")
        if "=" in mine[0].split(None, 1)[1]:
            self.skipTest("이 호스트는 tail 의 env 를 노출한다(재현 전제 불성립) — 순수 검체가 대신 판정")
        count, detail = pf.claude_procs_for_config(self.cfg)
        self.assertIsNotNone(count, "tail -f …/claude 가 unresolved 를 만들었다(R2 회귀): %s" % detail)
        _require_exchange(self)
        self.untrusted_file('{"projects": {}}')
        rc, out, err = seed_cli(self.cfg, self.ws, stub_ps=False)
        self.assertEqual(rc, 0, out + err)
        self.assertIn("OK seeded(", out)

    def test_3c_unverified_refuses_unless_forced(self):
        _require_exchange(self)
        self.untrusted_file('{"hasCompletedOnboarding": true}')   # ★R2: 기존 문서 → 프로브
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (None, "no-ps"))
        self.assertEqual((rc, verdict), (2, "REFUSE"))
        self.assertIn("unverified(no-ps)", reason)
        self.assertEqual(_read_text(self.cfgfile), '{"hasCompletedOnboarding": true}', "검증 불가 거부인데 파일을 썼다")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, force_unverified=True,
                                            proc_counter=lambda d: (None, "no-ps"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("force-unverified(no-ps)", reason, "강행 사실이 사유에 남지 않았다")
        self.assertEqual(_read_json(self.cfgfile), {"hasCompletedOnboarding": True,
                                                    "projects": {self.key: {"hasTrustDialogAccepted": True}}})
        os.unlink(self.cfgfile)
        rc, out, err = seed_cli(self.cfg, self.ws, "--force-unverified", "--json", env={"PATH": "/nonexistent"}, stub_ps=False)
        self.assertEqual(rc, 0, err)
        self.assertEqual(json.loads(out)["verdict"], "OK")       # 부재 파일: 프로브 자체가 없다(ps 부재 무관)

    def test_3d_force_unverified_does_not_bypass_live_claude(self):
        self.untrusted_file('{"projects": {}}')
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, force_unverified=True,
                                            proc_counter=lambda d: (2, "fake"))
        self.assertEqual((rc, verdict), (2, "REFUSE"))
        self.assertIn("live-claude(n=2", reason)
        self.assertEqual(_read_text(self.cfgfile), '{"projects": {}}')

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
        runner = _ps2("  7 /Users/user/.local/bin/claude --continue\n  8 python3 x.py\n")
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=runner, os_name="posix", platform="darwin")
        self.assertIsNone(cnt, detail)
        self.assertIn("env 비노출", detail)
        counter = lambda d: pf.claude_procs_for_config(d, runner=runner, os_name="posix", platform="darwin")
        self.untrusted_file('{"projects": {}}')
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=counter)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("unverified", reason)
        # ★R2: env 비노출이라도 인자 속 claude(tail/less)는 형상 아님 → 검증된 0
        benign = _ps2("  7 tail -f /x/logs/claude\n  8 less /Users/user/.local/bin/claude\n  9 zsh -lc export X=1; claude\n")
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=benign, os_name="posix", platform="darwin")
        self.assertEqual(cnt, 0, detail)
        pos = _ps2("  7 /Users/user/.local/bin/claude\n  9 /Users/user/.local/bin/claude CLAUDE_CONFIG_DIR=%s\n" % self.cfg)
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=pos, os_name="posix", platform="darwin")
        self.assertEqual(cnt, 1, detail)
        # ★R3(codex major 재현): 인자 속 CLAUDE_CONFIG_DIR=/other 가 env 의 실제 값(우리 config)을 가리던 줄 → argv 대조로 양성
        hide = _ps2("  7 claude -p CLAUDE_CONFIG_DIR=/other CLAUDE_CONFIG_DIR=%s HOME=/x\n" % self.cfg)
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=hide, os_name="posix", platform="darwin")
        self.assertEqual(cnt, 1, "인자 속 NAME= 가 env 를 가렸다(검증된 0): %s" % detail)
        calls = []
        def order(cmd):
            calls.append(list(cmd))
            return _ps2("  7 claude HOME=/x\n")(cmd)
        pf.claude_procs_for_config(self.cfg, runner=order, os_name="posix", platform="darwin")
        self.assertEqual([("-E" in c) for c in calls], [False, True], "argv 전용 ps 가 -E 앞에 오지 않았다: %s" % calls)
        cnt, detail = pf.claude_procs_for_config(self.cfg, runner=lambda c: (1, "", "") if "-E" not in c else (0, "7 claude HOME=/x", ""),
                                                 os_name="posix", platform="darwin")
        self.assertIsNone(cnt, "argv ps 실패가 검증된 0 이 됐다: %s" % detail)
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

        real_stat = os.stat

        def owner_stat(uid):
            """★R5: uid **만** 덧씌우고 나머지 필드는 실제 stat 그대로 — 종전 부분 페이크(st_uid 뿐)는 동일성 판정
            (_same_dir 의 st_dev/st_ino)이 들어오자 AttributeError 로 러너를 깨뜨렸다(하네스 결함 · 계약 무변경)."""
            def _stat(path, **k):
                st = real_stat(path, **k)      # 부재 경로는 여기서 그대로 OSError — '부재' 판정이 살아 있어야 한다
                return type("S", (), {"st_uid": uid, "st_dev": st.st_dev, "st_ino": st.st_ino})()
            return _stat

        with patch("builtins.open", denied_open), patch.object(pf.os, "stat", owner_stat(me)):
            cnt, detail = pf._count_claude_procfs(self.cfg, root)
        self.assertIsNone(cnt, detail)
        self.assertIn("미해결", detail)
        with patch("builtins.open", denied_open), patch.object(pf.os, "stat", owner_stat(me + 1)):
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
        # 재시도 — ★R5(리뷰 codex major): 성공은 **원자 교환이 있는 FS 에서만** 계약이다(Windows 는 기존 문서 커밋을
        #   반드시 거부한다). 능력으로 분기하되 거부 쪽에서도 '상대 바이트 불변' 은 그대로 단언한다.
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        if not _exchange_supported():
            self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
            self.assertIn("exchange-unavailable(", reason)
            self.assertEqual(_read_text(self.cfgfile), other, "거부인데 상대 바이트가 바뀌었다")
            return
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
        """커밋(교환) 실물 **뒤에** 다른 기록자가 payload 를 쓰는 상황(교체~되읽기 사이). ★R2: 기존 파일 커밋은 os.replace 가 아니라
        _exchange_paths 라 그 뒤에 끼운다(교환 기구가 없는 플랫폼이면 os.replace 폴백 뒤)."""
        real_ex, real_replace = pf._exchange_paths, os.replace

        def late(dst):
            with open(dst, "wb") as f:
                f.write(payload)

        def exchange(a, b):
            r = real_ex(a, b)
            if r:
                late(b)
            return r

        def replace(src, dst):
            real_replace(src, dst)
            late(dst)

        with patch.object(pf, "_exchange_paths", exchange), patch.object(pf.os, "replace", replace):
            return pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))

    def test_4e_post_commit_writer_preserved_no_rollback(self):
        """★R1(codex): 교체 後 기록자 내용은 **보존**(롤백 0). 플래그 보존 → OK · 없음 → REFUSE post-commit · 판독 불가 → ERROR 커밋 유지."""
        _require_exchange(self)
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
        _require_exchange(self)
        _write(self.cfgfile, '{"theme": "dark"}')
        real = pf._read_claude_json_bytes
        calls = []

        def flaky(path):
            calls.append(path)
            # ★재핀(★codex 설계비평 8 · plan §8): 교환 성공 뒤 옛 원본(displaced) 폐기에 **바이트 증명** 판독이
            #   한 번 더 들어왔다(활성이 우리 payload 임을 확인해야 지운다) — 되읽기는 이제 4번째다.
            #   (초기 · 교체 직전 대조 · 폐기 증명 · 되읽기)
            if path == self.cfgfile and len([c for c in calls if c == self.cfgfile]) == 4:
                raise OSError("injected readback failure")
            return real(path)

        with patch.object(pf, "_read_claude_json_bytes", flaky):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("커밋 상태로 둔다", reason)
        self.assertEqual(_read_json(self.cfgfile), {"theme": "dark", "projects": {self.key: {"hasTrustDialogAccepted": True}}})
        self.assertNotIn("롤백:", reason)

    def test_4f2_displaced_read_failure_restores_original(self):
        """★R2: 교환되어 나온 옛 inode 를 판독 못 하면 '낯선 것' → 되교환(원본 그대로) → REFUSE · 우리 문서 잔재 0."""
        original = '{"theme": "dark", "projects": {}}'
        _write(self.cfgfile, original)
        real = pf._read_claude_json_bytes
        hits = []

        def flaky(path):
            if os.path.basename(path).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                hits.append(path)
                if len(hits) == 1:                       # 교환 직후 옛 inode 판독만 실패(되교환 뒤 우리 payload 검증은 정상)
                    raise OSError("injected displaced read failure")
            return real(path)

        _require_exchange(self)
        with patch.object(pf, "_read_claude_json_bytes", flaky):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("concurrent-change", reason)
        self.assertEqual(_read_text(self.cfgfile), original, "되교환이 원본을 복원하지 않았다")
        self.assertEqual([n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME], [],
                         "우리 문서 잔재가 남았다")

    def test_4g_writer_between_compare_and_commit_is_detected_losslessly(self):
        """★R2 핀 전환(리뷰 codex BLOCK · 종전 '알려진 한계' 핀 뒤집음): 대조~커밋 사이의 기록자(in-place · rename-over 둘 다)를
        원자 교환이 드러낸다 → 되교환(상대 inode 그대로) → REFUSE concurrent-change · 상대 바이트 그대로 · 우리 잔재 0."""
        _write(self.cfgfile, '{"projects": {}}')
        real_ex = pf._exchange_paths
        late = b'{"projects": {"/late": {"hasTrustDialogAccepted": true}}}'
        calls = []

        def exchange(a, b):
            calls.append((a, b))
            if len(calls) == 1:                       # 대조는 이미 통과 · 교환 직전에 기록자가 끼어든다
                with open(b, "wb") as f:
                    f.write(late)
            return real_ex(a, b)

        _require_exchange(self)
        with patch.object(pf, "_exchange_paths", exchange):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("concurrent-change", reason)
        self.assertEqual(len(calls), 2, "되교환이 없었다: %s" % calls)
        self.assertEqual(_read_bytes(self.cfgfile), late, "기록자의 쓰기가 덮였다(데이터 손실)")
        leftovers = [n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME]
        self.assertEqual(leftovers, [], leftovers)
        # rename-over 기록자(새 inode)도 같다
        calls.clear()

        def exchange2(a, b):
            calls.append((a, b))
            if len(calls) == 1:
                t = b + ".writer-tmp"
                with open(t, "wb") as f:
                    f.write(late)
                os.replace(t, b)
            return real_ex(a, b)

        _write(self.cfgfile, '{"projects": {}}')
        with patch.object(pf, "_exchange_paths", exchange2):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertEqual(_read_bytes(self.cfgfile), late)
        # 기록자 없으면 커밋 · 옛 inode 폐기 · 잔재 0
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("commit=exchange", reason)
        self.assertEqual(_read_json(self.cfgfile)["projects"]["/late"], {"hasTrustDialogAccepted": True})
        self.assertEqual([n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME], [])

    def test_4g3_exchange_real_error_is_error_and_leaves_no_payload_litter(self):
        """교환이 실 오류(ENOENT 등)로 실패하면 교환은 안 된 것 — ERROR '쓰기 실패' · 원본 불변 · displaced(우리 payload) 잔재 0."""
        original = '{"projects": {}}'
        _write(self.cfgfile, original)
        with patch.object(pf, "_exchange_paths", side_effect=OSError(2, "injected ENOENT")):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("쓰기 실패", reason)
        self.assertEqual(_read_text(self.cfgfile), original)
        self.assertEqual([n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME], [])

    def test_4g2_exchange_unavailable_refuses_without_rename_over(self):
        """★R3 핀 전환(리뷰 codex BLOCK · 종전 '폴백 한계 고지' 핀 폐기): 교환 기구 부재(Windows · 미지원 FS)는 **REFUSE exchange-unavailable**
        — os.replace 는 호출되지 않고(대조~교체 창 자체가 열리지 않는다) 기존 문서·끼어든 기록자 바이트 그대로 · 잔재 0 ·
        --force-unverified 로도 열리지 않는다(귀속 불확실 감수 ≠ 알려진 손실 감수). None(구형 스텁)도 부재로 해석."""
        _write(self.cfgfile, '{"projects": {}}')
        late = b'{"projects": {"/late": {"hasTrustDialogAccepted": true}}}'
        for unavailable, token in ((pf._ExchangeUnavailable("ENOTSUP"), "exchange-unavailable(ENOTSUP"),
                                   (None, "exchange-unavailable(")):
            for force in (False, True):
                with self.subTest(unavailable=unavailable, force=force):
                    _write(self.cfgfile, '{"projects": {}}')

                    def unavailable_after_writer(a, b, _u=unavailable):
                        with open(b, "wb") as f:                 # 대조 뒤·커밋 순간에 끼어든 기록자 — 폴백이 있었다면 덮였을 바이트
                            f.write(late)
                        return _u

                    def no_rename_over(a, b, _f=os.replace):
                        self.assertNotEqual(os.path.abspath(b), os.path.abspath(self.cfgfile),
                                            "rename-over 폴백이 호출됐다")
                        return _f(a, b)

                    with patch.object(pf, "_exchange_paths", unavailable_after_writer), \
                            patch.object(pf.os, "replace", no_rename_over):
                        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"),
                                                            force_unverified=force)
                    self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                    self.assertIn(token, reason)
                    self.assertEqual(_read_bytes(self.cfgfile), late, "기록자 바이트가 덮였다")
                    leftovers = [n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME]
                    # ★성찰 P12 재핀(major · 2026-09-10 · **의도적 방향 변경**): 종전(triage I4)은 '활성이 우리 두 지문
                    #   중 하나와 바이트가 같은가' 만 봤고, 그 바이트 축에서는 이 서브케이스가 증명 실패라 conflict 를
                    #   남겼다. 그런데 여기서 우리 사본은 **시더가 다음 부트에 스스로 다시 쓰는 플래그뿐**이다(캡처
                    #   원본이 `{"projects": {}}` 였다) — 지워도 잃는 것이 0 인데 좌석 수만큼 매 부트 쌓였다(실측 축).
                    #   축을 바이트에서 **구조**로 올려(`_strip_regenerable_flags`+`_covers`) 재생성 가능한 차이만
                    #   흡수한다: 잔재 0.
                    self.assertEqual(leftovers, [], "재생성 가능한 사본이 conflict 로 쌓였다(P12 누적 축)")
        # 부재 파일은 교환과 무관(os.link) — 기구 부재 플랫폼에서도 신규 부서 시드는 동작
        os.unlink(self.cfgfile)
        with patch.object(pf, "_exchange_paths", lambda a, b: pf._ExchangeUnavailable("platform:nt")):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("commit=link", reason)

    def test_4g2b_uncommitted_copy_with_user_data_is_still_preserved(self):
        """★성찰 P12 양성 대조(2026-09-10): 위 핀이 '잔재 0' 으로 바뀐 근거는 **그 사본이 재생성 가능**했기 때문이지
        '미커밋 사본은 지워도 된다' 가 아니다 — 같은 형상(교환 기구 부재 + 대조 뒤 끼어든 기록자)에서 캡처 원본이
        사용자 데이터를 담고 있으면 우리 사본은 그 데이터의 유일한 사본이므로 conflict 로 보존된다."""
        original = b'{"userID": "u-1", "oauthAccount": {"emailAddress": "a@b"}, "projects": {}}'
        _write_bytes(self.cfgfile, original)
        late = b'{"projects": {"/late": {"hasTrustDialogAccepted": true}}}'   # 활성이 원본을 통째로 잃었다

        def unavailable_after_writer(a, b):
            with open(b, "wb") as f:
                f.write(late)
            return pf._ExchangeUnavailable("ENOTSUP")

        with patch.object(pf, "_exchange_paths", unavailable_after_writer):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("사본 보존", reason)
        saved = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_CONFLICT_PREFIX)]
        self.assertEqual(len(saved), 1, os.listdir(self.cfg))
        doc = json.loads(_read_bytes(os.path.join(self.cfg, saved[0])).decode("utf-8"))
        self.assertEqual(doc["userID"], "u-1", "보존한 사본에 사용자 데이터가 없다")
        self.assertIs(doc["projects"][self.key]["hasTrustDialogAccepted"], True)

    def test_4h_absent_file_create_if_absent_refuses_when_writer_creates_first(self):
        """★R2: 부재 파일 커밋 = os.link(원자 create-if-absent) — 그 사이 다른 기록자가 만든 파일은 덮이지 않고 REFUSE."""
        os.rmdir(self.cfg) if os.path.isdir(self.cfg) and not os.listdir(self.cfg) else None
        real_link = os.link
        theirs = b'{"projects": {"/theirs": {}}}'

        def link(src, dst):
            with open(dst, "wb") as f:
                f.write(theirs)
            return real_link(src, dst)

        with patch.object(pf.os, "link", link):
            rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("생겨났다", reason)
        self.assertEqual(_read_bytes(self.cfgfile), theirs, "상대가 만든 파일이 덮였다")
        self.assertEqual([n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME], [])
        # ★R3(리뷰 codex major): 하드링크 실패(미지원 FS EPERM/EXDEV · ENOSPC/EIO)는 O_EXCL 배타 생성 폴백이 아니라 REFUSE link-failed
        #   — 이름이 내용보다 먼저 공개되는 경로 0(잘린 .claude.json 이 영구 손상 문서로 남던 것) · cfg 미생성 · 잔재 0 · 쓰기 open 0
        os.unlink(self.cfgfile)
        real_open = os.open
        for err in (errno.EPERM, errno.EXDEV, errno.ENOSPC, errno.EIO):
            with self.subTest(errno=errno.errorcode[err]):
                opened = []

                def opening(path, flags, *a, **kw):
                    if os.fspath(path) == self.cfgfile:
                        opened.append(flags)
                    return real_open(path, flags, *a, **kw)

                with patch.object(pf.os, "link", side_effect=OSError(err, "injected")), patch.object(pf.os, "open", opening):
                    rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=_no_probe)
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                self.assertIn("link-failed(%s" % errno.errorcode[err], reason)
                self.assertFalse(os.path.lexists(self.cfgfile), "link 실패인데 .claude.json 이 생겼다(부분 공개)")
                self.assertEqual(opened, [], "cfg 경로를 직접 열었다(O_EXCL 폴백 잔존): %s" % opened)
                self.assertEqual([n for n in os.listdir(self.cfg) if n.startswith(".claude.json.") and n != pf.SEED_TRUST_LOCK_NAME], [])

    def test_4i_crash_after_exchange_leaves_displaced_untouched_by_next_sweep(self):
        """★R2(codex): 교환 직후 죽으면 옛 inode 는 .claude.json.displaced-* 에 남는다 — 다음 시더의 잔재 청소(.seed-<8자>)가 그것을
        지우지 않고, 거기 든 낯선 데이터(기록자의 유일 사본일 수 있다)가 살아남는다."""
        _require_exchange(self)
        _write(self.cfgfile, '{"projects": {}}')
        foreign = b'{"projects": {"/foreign": {"hasTrustDialogAccepted": true}}}'
        real_ex = pf._exchange_paths

        def exchange_then_die(a, b):
            with open(b, "wb") as f:
                f.write(foreign)                       # 기록자가 끼어들고
            r = real_ex(a, b)                          # 교환은 성공
            raise KeyboardInterrupt("SIGINT right after exchange")   # 그 직후 시더 사망

        with patch.object(pf, "_exchange_paths", exchange_then_die):
            with self.assertRaises(KeyboardInterrupt):
                pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        displaced = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX)]
        self.assertEqual(len(displaced), 1, os.listdir(self.cfg))
        self.assertEqual(_read_bytes(os.path.join(self.cfg, displaced[0])), foreign, "낯선 데이터가 displaced 에 없다")
        # ★R5 재핀(리뷰 codex major D4 · 이 WP 의 R2 자기 핀): 종전 계약은 다음 실행이 `already-trusted` 로 **조용히 OK**
        #   를 내는 것이었다 — 그 순간 상대의 더 새 문서는 displaced 에 고립된 채 아무도 모른다. 이제 교환 직전에 남긴
        #   의도 저널이 그 창을 표시하고, 다음 실행은 두 파일을 무접촉으로 두고 **REFUSE interrupted-transaction** 한다
        #   (호출자는 fail-open WARN · 좌석은 죽지 않는다 · 관문 보류가 2차 방어). 보존 단언은 그대로 유지한다.
        journals = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_INTENT_PREFIX)]
        self.assertEqual(len(journals), 1, os.listdir(self.cfg))
        _write(os.path.join(self.cfg, ".claude.json.seed-abcd1234"), "litter")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("interrupted-transaction", reason)
        self.assertIn(displaced[0], reason, "거부 사유가 고립된 문서의 이름을 대지 않는다")
        self.assertEqual(_read_bytes(os.path.join(self.cfg, displaced[0])), foreign, "displaced 가 청소됐다(데이터 파괴)")
        self.assertTrue(os.path.exists(os.path.join(self.cfg, ".claude.json.seed-abcd1234")),
                        "거부 경로가 잔재 청소보다 앞이어야 한다(회수 판정 前 상태 변경 0)")
        # 사람이 병합·정리하면(저널 삭제) 다음 실행은 종전대로 진행한다 — 영구 잠김이 아니다
        os.unlink(os.path.join(self.cfg, journals[0]))
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual(rc, 0, reason)   # 현 .claude.json(우리 문서)은 이미 신뢰
        self.assertIn("already-trusted", reason)
        self.assertFalse(os.path.exists(os.path.join(self.cfg, ".claude.json.seed-abcd1234")), "mkstemp 잔재가 남았다")
        self.assertEqual(_read_bytes(os.path.join(self.cfg, displaced[0])), foreign, "displaced 가 청소됐다(데이터 파괴)")


    def test_4j_crash_before_exchange_leaves_payload_only_displaced_that_next_sweep_reclaims(self):
        """★R3(리뷰): rename(tmp→displaced) 와 교환 사이에 죽으면 displaced 엔 **우리 payload 만** 있다 — 이름의 sha256 지문과 바이트가
        같으므로 다음 시더의 잠금 아래 청소가 그것만 회수한다(반복 crash 에도 무한 누적 0). 지문이 다른 displaced(교환 뒤 옛 inode) ·
        지문 없는 구형 이름 · conflict · 지문이 맞아도 심링크/FIFO 는 무접촉."""
        _write(self.cfgfile, '{"projects": {}}')

        def die_before_exchange(a, b):
            raise KeyboardInterrupt("SIGINT before exchange")

        with patch.object(pf, "_exchange_paths", die_before_exchange):
            with self.assertRaises(KeyboardInterrupt):
                pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        displaced = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX)]
        self.assertEqual(len(displaced), 1, os.listdir(self.cfg))
        payload = _read_bytes(os.path.join(self.cfg, displaced[0]))
        self.assertEqual(json.loads(payload.decode("utf-8")), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        m = pf._SEED_DISPLACED_RE.match(displaced[0])
        self.assertIsNotNone(m, displaced[0])
        self.assertEqual(m.group(1), pf._payload_digest(payload), "이름의 지문이 payload sha256 이 아니다")
        self.assertEqual(len(m.group(1)), 64)
        # 보존돼야 하는 이웃들
        foreign = os.path.join(self.cfg, pf.SEED_TRUST_DISPLACED_PREFIX + m.group(1) + "-foreign")
        _write(foreign, "foreign bytes that do not match the digest")
        legacy = os.path.join(self.cfg, pf.SEED_TRUST_DISPLACED_PREFIX + "20260906T000000Z-1")
        with open(legacy, "wb") as f:
            f.write(payload)
        conflict = os.path.join(self.cfg, pf.SEED_TRUST_CONFLICT_PREFIX + m.group(1) + "-x")
        with open(conflict, "wb") as f:
            f.write(payload)
        target = os.path.join(self.tmp, "link-target")
        with open(target, "wb") as f:
            f.write(payload)
        link = os.path.join(self.cfg, pf.SEED_TRUST_DISPLACED_PREFIX + m.group(1) + "-link")
        os.symlink(target, link)
        fifo = os.path.join(self.cfg, pf.SEED_TRUST_DISPLACED_PREFIX + m.group(1) + "-fifo")
        if hasattr(os, "mkfifo"):        # ★R4: Windows 엔 mkfifo 가 없다 — 그 항목만 건너뛴다(검체 전체 skip 아님)
            os.mkfifo(fifo)
        # ★성찰 P2 재핀(blocking · 2026-09-10 · **의도적 기본값 변경**): 잔재 청소의 기본값이 뒤집혔다 — 판독·파싱에
        #   실패한 바이트는 이제 '지킬 데이터 있음' 이다(종전엔 '읽을 수 없으면 잃을 것도 없다' 로 접혀, 사용자 필드가
        #   남은 채 잘린 임시 JSON 이 지워졌다 · 키 이름만으로는 값의 뜻을 판정할 수 없다). 그래서 '청소된다' 를 재는
        #   이 자리의 잔재는 **실제 부분 기록**(우리가 쓰다 만 payload 접두)이어야 한다 — 그 바이트는 지금 다시 계획해도
        #   나오므로 `_copy_supersedable` 의 접두 증명(⑤)이 삭제를 인가한다. 반대 방향은 아래 음성 대조가 잰다.
        _write_bytes(os.path.join(self.cfg, ".claude.json.seed-abcd1234"), payload[:20])
        _require_exchange(self)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)      # 원본은 crash 에 무접촉 → 이번엔 정상 커밋
        # ★triage I1 재핀(plan §8): 저널이 있는 자리에서는 **회수(ⓑ)가 스스로** 사본을 지운다 — 청소는 저널의
        #   `captured_sha256` 을 모르므로 더 약한 증거로 같은 결정을 다시 내려야 했다(활성 = captured 인 이 형상에서
        #   바이트 증명에 실패해 애먼 격리가 된다). 회수 1 + mkstemp 잔재 청소 1 로 나뉘고 합계는 그대로다.
        self.assertIn("recovery-reclaimed(1", reason)
        self.assertIn("stale-tmp swept 1", reason)               # mkstemp 잔재 1(payload displaced 는 회수가 처리)
        self.assertIn("commit=exchange", reason)
        self.assertFalse(os.path.lexists(os.path.join(self.cfg, displaced[0])), "우리 payload displaced 가 회수되지 않았다")
        self.assertFalse(os.path.lexists(os.path.join(self.cfg, ".claude.json.seed-abcd1234")))
        for keep in (foreign, legacy, conflict, link) + ((fifo,) if hasattr(os, "mkfifo") else ()):
            self.assertTrue(os.path.lexists(keep), "보존 대상이 지워졌다: %s" % keep)
        self.assertEqual(_read_bytes(target), payload, "심링크 타깃이 지워졌다")
        self.assertEqual(_read_json(self.cfgfile), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})
        # ★성찰 P2 음성 대조: 같은 자리에 **읽을 수 없는** 바이트를 두면 청소되지 않고 보존 네임스페이스로 간다
        #   (파싱 실패를 '비어 있다' 로 읽던 것이 사용자 필드가 남은 잘린 임시본을 지운 축이다).
        _write_bytes(os.path.join(self.cfg, ".claude.json.seed-ffff0000"), b"{not json at all")
        rc2, verdict2, reason2 = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc2, verdict2), (0, "OK"), reason2)
        self.assertIn("stale-tmp preserved", reason2)
        self.assertFalse(os.path.lexists(os.path.join(self.cfg, ".claude.json.seed-ffff0000")))
        kept_conflicts = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_CONFLICT_PREFIX)]
        self.assertTrue(any(_read_bytes(os.path.join(self.cfg, n)) == b"{not json at all" for n in kept_conflicts),
                        kept_conflicts)

    def test_4k_lone_surrogate_document_is_seeded_without_traceback(self):
        """★R3(리뷰 claude): 기존 문서의 고아 서로게이트 이스케이프(`"\\ud800"`)는 json.loads 는 받지만 utf-8 인코딩이 UnicodeEncodeError 를
        냈다(try 밖 → C58 --fix 경유 시 preflight 전체 중단). 이제 ASCII 이스케이프로 재직렬화 → OK · 값 보존 · CLI 도 traceback 0."""
        _require_exchange(self)
        _write(self.cfgfile, '{"projects": {"/x": {"note": "\\ud800"}}, "theme": "d"}')
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (0, "t"))
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("ascii-escaped", reason)
        back = _read_json(self.cfgfile)
        self.assertEqual(back["projects"]["/x"]["note"], "\ud800", "고아 서로게이트 값이 바뀌었다")
        self.assertEqual(back["projects"][self.key], {"hasTrustDialogAccepted": True})
        self.assertEqual(back["theme"], "d")
        _write(self.cfgfile, '{"projects": {"/y": {"note": "\\udfff"}}}')
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, out + err)
        self.assertNotIn("Traceback", err)
        self.assertIn("OK seeded(", out)

    def test_4l_post_commit_reasons_carry_commit_note(self):
        """★R3(codex D9): post-commit REFUSE/ERROR 사유에도 commit=/probe= 진단이 남는다(성공 경로만 진단하던 것)."""
        _require_exchange(self)
        _write(self.cfgfile, '{"projects": {}}')
        dropped = b'{"projects": {"/x": {"hasTrustDialogAccepted": true}}}'
        rc, verdict, reason = self._seed_with_post_replace_writer(dropped)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("commit=exchange", reason)
        self.assertIn("probe=", reason)
        _write(self.cfgfile, '{"projects": {}}')
        rc, verdict, reason = self._seed_with_post_replace_writer(b"{broken")
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("commit=exchange", reason)


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

    def test_5b_gap_judgment_is_pair_scoped_without_markers(self):
        """★R2(리뷰): _is_cysjavis_workspace 삭제 — 워크스페이스 판정은 _trust_gap_workspaces(등재 쌍 → 정확 키) 하나. 마커 파일
        없이 · 합집합 살포 0 · codex 좌석 제외 · stale 제외 · 미등재 제외 · 별칭 true 불인정을 **행동**으로 판정."""
        self.assertFalse(os.path.exists(os.path.join(self.X, "CLAUDE.md")))
        self.assertFalse(os.path.isdir(os.path.join(self.X, "_round")))
        self.assertFalse(hasattr(pf.Preflight, "_is_cysjavis_workspace"))
        p = self._pf(fix=False)
        K = pf.claude_project_key
        random_ws = os.path.join(self.home, "random")
        os.makedirs(random_ws)
        # 본부 config: X 만 갭(stale 제외 · 부서 cwd Y·codex X3·미등재 random 은 이 config 의 쌍이 아니다)
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgA, ".claude.json")), [self.X])
        _write(os.path.join(self.cfgA, ".claude.json"), json.dumps({"projects": {
            K(self.X): {"hasTrustDialogAccepted": True}, K(self.Y): {"hasTrustDialogAccepted": False},
            K(self.X3): {"hasTrustDialogAccepted": False}, K(random_ws): {"hasTrustDialogAccepted": False}}}))
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgA, ".claude.json")), [],
                         "본부 config 에 등재되지 않은 cwd(부서 Y·codex X3·random)가 갭으로 잡혔다(합집합 살포)")
        # 부서 config: Y·X2 갭 · X3(codex)·X4(null/legacy) 제외
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgB, ".claude.json")), sorted([self.Y, self.X2]))
        _write(os.path.join(self.cfgB, ".claude.json"), json.dumps({"projects": {
            K(self.Y): {"hasTrustDialogAccepted": True}, K(self.X2) + "/": {"hasTrustDialogAccepted": True}}}))
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.cfgB, ".claude.json")), [self.X2], "별칭 true 를 신뢰로 인정")
        # 등재 0 config → 갭 0(판정 대상 없음) · 미등재 config 도 0
        self.assertEqual(p._trust_gap_workspaces(os.path.join(self.home, "unregistered", ".claude.json")), [])

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
        if not _exchange_supported():
            # ★R6(리뷰 codex major#5): 교환 없는 플랫폼의 혼합 결과 — **부재 문서** config(A·C)는 link 로 성립하고
            #   **기존 문서** config(B)는 REFUSE exchange-unavailable 이다. 기대값은 config 의 초기 상태로 갈린다.
            self.assertEqual(r["status"], pf.WARN, r)
            self.assertIn("exchange-unavailable", r["detail"])
            self.assertEqual(_read_bytes(bfile), raw, "거부인데 기존 문서가 바뀌었다")
            self.assertEqual(_read_json(os.path.join(self.cfgA, ".claude.json")),
                             {"projects": {pf.claude_project_key(self.X): {"hasTrustDialogAccepted": True}}})
            self.assertEqual(_read_json(os.path.join(self.cfgC, ".claude.json")),
                             {"projects": {pf.claude_project_key(self.Z): {"hasTrustDialogAccepted": True}}})
            return
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
        """--fix 도 seed_trust 경로: 기존 문서가 있는 config 는 라이브 claude 면 REFUSE(문서 불변) · ★R2 부재 문서 config 는 프로브
        없이 생성(지킬 문서가 없다) — 둘이 한 결과 줄에 함께 남는다."""
        for c in (self.cfgA, self.cfgB):
            _write(os.path.join(c, ".claude.json"), '{"projects": {}}')     # 기존 문서 · 플래그 부재 → 프로브 → 거부
        p = self._pf(fix=True)
        saved = pf.claude_procs_for_config
        pf.claude_procs_for_config = lambda d, **k: (1, "fake-live")
        try:
            r = self._c58(p)
        finally:
            pf.claude_procs_for_config = saved
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("REFUSE live-claude", r["detail"])
        for c in (self.cfgA, self.cfgB):
            self.assertEqual(_read_text(os.path.join(c, ".claude.json")), '{"projects": {}}', "라이브 거부인데 문서가 바뀌었다")
        self.assertIn("trust set: %s" % os.path.join(self.cfgC, ".claude.json"), r["detail"], "부재 문서 config(C)가 시드되지 않았다")
        self.assertEqual(_read_json(os.path.join(self.cfgC, ".claude.json")),
                         {"projects": {pf.claude_project_key(self.Z): {"hasTrustDialogAccepted": True}}})

    def test_5h_malformed_topology_shapes_are_unreadable_not_fatal(self):
        """★R2(리뷰 codex major): {"entries": 1|null|false|{}|"x"} 는 TypeError 로 preflight 를 죽이지 않고 판독불가 1건 · 건강한 출처의
        쌍은 그대로 · 키 부재는 빈 로스터(판독됨)."""
        hub = os.path.join(self.home, ".local", "state", "cys", "topology.json")
        for bad in (1, None, False, {}, "x", [1, "s", None]):
            with self.subTest(bad=bad):
                _write(hub, json.dumps({"entries": bad}))
                reg = pf.cysjavis_registry()
                if isinstance(bad, list):
                    self.assertNotIn(hub, reg["unreadable"])           # list 인데 항목이 비-dict → 항목만 건너뜀
                else:
                    self.assertIn(hub, reg["unreadable"], (bad, reg))
                self.assertIn(pf._path_identity(self.cfgB), reg["pairs"], "건강한 부서 topology 쌍이 사라졌다")
                r = self._c58(self._pf(fix=False))
                self.assertIn(r["status"], (pf.WARN, pf.PASS, pf.FIXED), r)
                if not isinstance(bad, list):
                    self.assertIn("판독불가", r["detail"])
        _write(hub, json.dumps({"version": 1}))
        reg = pf.cysjavis_registry()
        self.assertNotIn(hub, reg["unreadable"])
        self.assertNotIn(hub, reg["sources"])          # 쌍 0 이라 출처 표기도 없다(종전 계약)

    def test_5i_catalog_cwd_resolves_like_launcher_but_topology_is_preserved(self):
        """★R3(리뷰 codex major): depts.json 카탈로그 cwd "/"·"///"·"C:\\"·부재 dir·루트 심링크 는 기동기(resolve_dept_cwd)처럼 $HOME 으로
        해석돼 쌍이 된다(--fix 가 claude 가 결코 뜨지 않는"/" 를 시드하던 것) · 존재 dir 는 원값 · 상대값 제외 · topology 의 "/" 는
        관측 쌍 그대로 보존(해석 0) · depts.json 파일 자체는 무변경."""
        K, I = pf.claude_project_key, pf._path_identity
        cfgD = os.path.join(self.home, ".cys", "claude-default-dept-3")
        cfgE = os.path.join(self.home, ".cys", "claude-default-dept-4")
        cfgF = os.path.join(self.home, ".cys", "claude-default-dept-5")
        cfgG = os.path.join(self.home, ".cys", "claude-default-dept-6")
        rootlink = os.path.join(self.home, "rootlink")
        os.symlink("/", rootlink)
        for d in (cfgD, cfgE, cfgF, cfgG):
            os.makedirs(d)
        depts = {"depts": {
            "dept-3": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-3", "cys.sock"), "account_dir": cfgD, "cwd": "/"},
            "dept-4": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-4", "cys.sock"), "account_dir": cfgE,
                       "cwd": os.path.join(self.home, "vanished")},
            "dept-5": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-5", "cys.sock"), "account_dir": cfgF,
                       "cwd": rootlink},
            "dept-6": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-6", "cys.sock"), "account_dir": cfgG,
                       "cwd": "rel/ws"},
            "dept-2": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-2", "cys.sock"), "account_dir": self.cfgC,
                       "cwd": self.Z}}}
        raw = json.dumps(depts).encode("utf-8")
        with open(os.path.join(self.home, ".cys", "depts.json"), "wb") as f:
            f.write(raw)
        _write(os.path.join(self.home, ".local", "state", "cys", "topology.json"), json.dumps({"entries": [
            {"role": "master", "agent": "claude", "claude_config_dir": self.cfgA, "cwd": "/"}]}))
        for root_form in ("/", "///", "C:\\"):
            with self.subTest(root_form=root_form):
                depts["depts"]["dept-3"]["cwd"] = root_form
                _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps(depts))
                reg = pf.cysjavis_registry()
                self.assertEqual(reg["pairs"][I(cfgD)], {I(self.home): os.path.expanduser("~")}, reg["pairs"].get(I(cfgD)))
        self.assertEqual(reg["pairs"][I(cfgE)], {I(self.home): os.path.expanduser("~")}, "부재 등재 dir → $HOME(기동기 폴백 미러)")
        self.assertEqual(reg["pairs"][I(cfgF)], {I(self.home): os.path.expanduser("~")}, "루트 심링크 → $HOME(물리 경로 루트 거부 미러)")
        self.assertNotIn(I(cfgG), reg["pairs"], "상대 카탈로그 cwd 가 쌍이 됐다")
        self.assertEqual(reg["pairs"][I(self.cfgC)], {I(self.Z): self.Z}, "존재 dir 원값이 바뀌었다")
        self.assertEqual(reg["pairs"][I(self.cfgA)], {I("/"): "/"}, "topology 의 관측 cwd '/' 가 해석됐다(관측 쌍 보존 위반)")
        p = self._pf(fix=False)
        self.assertEqual(p._trust_gap_workspaces(os.path.join(cfgD, ".claude.json")), [os.path.expanduser("~")])
        self.assertEqual(p._trust_gap_workspaces(os.path.join(cfgE, ".claude.json")), [os.path.expanduser("~")])
        for gaps in (p._trust_gap_workspaces(os.path.join(c, ".claude.json")) for c in (cfgD, cfgE, cfgF)):
            self.assertNotIn("/", gaps)
            self.assertTrue(all(os.path.isdir(g) for g in gaps), gaps)
        # --fix 가 시드하는 키 = 기동기가 좌석을 띄우는 $HOME 의 정확 키
        p2 = self._pf(fix=True)
        with patch.object(pf, "claude_procs_for_config", lambda d, **k: (0, "t")):
            r = self._c58(p2)
        self.assertIn(K(self.home), _read_json(os.path.join(cfgD, ".claude.json"))["projects"])
        self.assertNotIn("/", _read_json(os.path.join(cfgD, ".claude.json"))["projects"])
        self.assertNotIn("%s / / (" % os.path.join(cfgD, ".claude.json"), r["detail"], "카탈로그 '/' 가 그대로 시드됐다")
        self.assertIn("%s / / (" % os.path.join(self.cfgA, ".claude.json"), r["detail"], "topology 관측 쌍 '/' 은 보존·시드 대상")
        depts["depts"]["dept-3"]["cwd"] = "C:\\"
        self.assertEqual(_read_json(os.path.join(self.home, ".cys", "depts.json")), depts, "depts.json 이 수정됐다(등재 계약 불변)")

    def test_5j_fix_seed_exception_is_one_warn_not_preflight_abort(self):
        """★R3(리뷰 claude): --fix 의 seed_trust 예외(예: lone surrogate UnicodeEncodeError 였던 것)는 WARN 1줄로 접히고 다른 쌍의 수리는
        계속된다 — run() 의 fut.result() 로 올라가 preflight 전체가 죽지 않는다."""
        real = pf.seed_trust
        calls = []

        def flaky(config_dir, cwd, **kw):
            calls.append((config_dir, cwd))
            if pf._path_identity(config_dir) == pf._path_identity(self.cfgA):
                raise RuntimeError("injected seed failure")
            return real(config_dir, cwd, **kw)

        p = self._pf(fix=True)
        with patch.object(pf, "seed_trust", flaky), patch.object(pf, "claude_procs_for_config", lambda d, **k: (0, "t")):
            r = self._c58(p)
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("예외 RuntimeError: injected seed failure", r["detail"])
        self.assertIn("trust set:", r["detail"], "예외 뒤 다른 쌍의 수리가 멈췄다")
        self.assertGreaterEqual(len(calls), 3, calls)
        self.assertFalse(os.path.exists(os.path.join(self.cfgA, ".claude.json")))
        projs = _read_json(os.path.join(self.cfgB, ".claude.json"))["projects"]
        if not _exchange_supported():
            # ★R6(codex R6): 부재 문서 config 의 **첫** cwd 는 link 로 성립하고, 그 뒤 같은 config 의 나머지 cwd 는
            #   이미 '기존 문서' 라 REFUSE 다 — 기대값을 config 의 초기 상태만으로 정하지 않는다.
            self.assertIn("exchange-unavailable", r["detail"])
            self.assertEqual(sorted(projs), [pf.claude_project_key(self.X2)])
            return
        self.assertIn(pf.claude_project_key(self.Y), projs)

    # ── ★R6(리뷰 codex major#4): 미해결 교환 트랜잭션은 신뢰 플래그가 **이미 활성**이라 갭이 0 이다 —
    #     갭만 보는 C58 은 그것을 안고 PASS 를 냈고 `--fix` 에서도 판정 경로가 아예 돌지 않았다.
    def _trust_all(self):
        """세 config 를 전부 '갭 없음' 으로 만든다(저널 축만 남기는 기준선 — 이 상태의 C58 은 PASS 다)."""
        K = pf.claude_project_key
        _write(os.path.join(self.cfgA, ".claude.json"),
               json.dumps({"projects": {K(self.X): {"hasTrustDialogAccepted": True}}}))
        _write(os.path.join(self.cfgB, ".claude.json"),
               json.dumps({"projects": {K(self.Y): {"hasTrustDialogAccepted": True},
                                        K(self.X2): {"hasTrustDialogAccepted": True}}}))
        _write(os.path.join(self.cfgC, ".claude.json"),
               json.dumps({"projects": {K(self.Z): {"hasTrustDialogAccepted": True}}}))

    def _plant_journal(self, cfg_dir, displaced_bytes=None, payload=b'{"payload":1}', captured=b'{"captured":1}'):
        """중단된 교환의 잔재를 심는다 → (저널 경로, displaced 경로|None). displaced_bytes=None 이면 displaced 부재
        (교환 전 사망 뒤 처분까지 끝난 형상 = 회수 가능)."""
        dname = (pf.SEED_TRUST_DISPLACED_PREFIX + pf._payload_digest(payload) + "-20260907T000000Z-1")
        dpath = os.path.join(cfg_dir, dname)
        if displaced_bytes is not None:
            _write_bytes(dpath, displaced_bytes)
        jpath = os.path.join(cfg_dir, pf.SEED_TRUST_INTENT_PREFIX + "20260907T000000Z-4242")
        _write(jpath, json.dumps({"v": 1, "displaced": dname,
                                  "captured_sha256": pf._payload_digest(captured),
                                  "payload_sha256": pf._payload_digest(payload)}))
        return jpath, (dpath if displaced_bytes is not None else None)

    def test_5k_unresolved_journal_is_warn_not_pass_and_report_is_read_only(self):
        """리뷰 codex major#4: 갭 0 + 미해결 저널 → PASS 가 아니라 WARN 이고, report 모드는 아무것도 건드리지 않는다."""
        self._trust_all()
        self.assertEqual(self._c58(self._pf(fix=False))["status"], pf.PASS, "기준선이 PASS 가 아니다(하네스 결함)")
        jpath, dpath = self._plant_journal(self.cfgB, displaced_bytes=b'{"foreign":"newer"}')
        before = _snapshot(self.home)
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("미해결 교환 저널", r["detail"])
        self.assertIn(self.cfgB, r["detail"])
        self.assertEqual(_snapshot(self.home), before, "report 모드가 파일을 건드렸다")
        self.assertTrue(os.path.exists(jpath) and os.path.exists(dpath))

    def test_5l_fix_adjudicates_the_journal_through_the_single_seed_path(self):
        """--fix 는 기존 단일 경로(seed_trust ⑬ 회수)로 판정한다 — 회수 가능하면 저널이 사라지고(FIXED),
        낯선 바이트면 REFUSE interrupted-transaction 이 WARN 줄로 남는다(임의 삭제 0)."""
        self._trust_all()
        jpath, _ = self._plant_journal(self.cfgB, displaced_bytes=None)      # displaced 부재 + 활성 유효 = 회수 가능
        r = self._c58(self._pf(fix=True))
        self.assertEqual(r["status"], pf.FIXED, r)
        self.assertIn("중단된 교환 저널 회수", r["detail"])
        self.assertFalse(os.path.exists(jpath), "회수했다면서 저널이 남았다")
        self.assertEqual(self._c58(self._pf(fix=False))["status"], pf.PASS, "회수 뒤에도 WARN 이 남는다")
        # 낯선 바이트(상대의 더 새 문서)는 사람 몫 — 도구가 지우지 않는다
        jpath, dpath = self._plant_journal(self.cfgB, displaced_bytes=b'{"foreign":"newest"}')
        r = self._c58(self._pf(fix=True))
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("interrupted-transaction", r["detail"])
        self.assertEqual(_read_bytes(dpath), b'{"foreign":"newest"}')
        self.assertTrue(os.path.exists(jpath))

    def test_5m_journal_enumeration_failure_is_never_pass(self):
        """열거가 막히면 '저널 없음' 이 아니다 — PASS 로 접지 않는다(결측은 값이 아니다)."""
        self._trust_all()
        real_listdir = os.listdir

        def blind(path):
            if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(self.cfgB):
                raise OSError(errno.EACCES, "injected")
            return real_listdir(path)

        with patch.object(pf.os, "listdir", blind):
            r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("교환 저널 상태 판독 불가", r["detail"])
        self.assertIn("EACCES", r["detail"])

    def test_5n_zero_pairs_with_a_journal_is_warn_not_skip(self):
        """등재 cwd 가 없는 config 의 저널도 보여야 한다 — 저널 점검이 쌍 0(SKIP)보다 **앞**인 이유."""
        for f in (os.path.join(self.home, ".local", "state", "cys", "topology.json"),
                  os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "topology.json")):
            os.unlink(f)
        _write(os.path.join(self.home, ".cys", "depts.json"), json.dumps({"depts": {
            "dept-1": {"socket": "/nonexistent/cys-dept-dept-1/cys.sock", "account_dir": self.cfgB}}}))
        self.assertEqual(self._c58(self._pf(fix=False))["status"], pf.SKIP, "기준선이 SKIP 이 아니다(하네스 결함)")
        jpath, _ = self._plant_journal(self.cfgB, displaced_bytes=b'{"foreign":1}')
        r = self._c58(self._pf(fix=False))
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("미해결 교환 저널", r["detail"])
        self.assertIn("쌍 0", r["detail"], "판정 불가라는 사실도 함께 말해야 한다")
        self.assertTrue(os.path.exists(jpath))

    def test_5o_fix_reenumerates_even_when_the_repair_creates_a_journal(self):
        """★R6(codex 위임 반례 ②): 수리가 **새 저널을 남기는** 정상 결과가 있다(교환은 성립했는데 청소가 실패).
        수리 전 스냅샷에 그 config 가 없었다는 이유로 재열거를 건너뛰면 FIXED 를 내며 잔존을 감춘다 —
        재열거는 스냅샷 유무와 무관하게 전 대상을 다시 훑어야 한다."""
        self._trust_all()
        _write(os.path.join(self.cfgB, ".claude.json"),      # X2 갭 하나를 만들어 수리 경로를 태운다
               json.dumps({"projects": {pf.claude_project_key(self.Y): {"hasTrustDialogAccepted": True}}}))
        real = pf.seed_trust
        planted = []

        def commit_with_residual(config_dir, cwd, **kw):
            got = real(config_dir, cwd, **kw)
            if pf._path_identity(config_dir) == pf._path_identity(self.cfgB) and not planted:
                planted.append(self._plant_journal(self.cfgB, displaced_bytes=b'{"foreign":"newer"}'))
            return got

        with patch.object(pf, "seed_trust", commit_with_residual):
            r = self._c58(self._pf(fix=True))
        self.assertTrue(planted, "수리 경로가 돌지 않았다(하네스 결함)")
        self.assertEqual(r["status"], pf.WARN, r)
        self.assertIn("미해결 교환 저널", r["detail"])
        self.assertTrue(os.path.exists(planted[0][0]))

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
        bfile = os.path.join(self.cfgB, ".claude.json")
        rawB = _read_bytes(bfile)
        with patch.object(pf, "seed_trust", spy):
            r = self._c58(self._pf(fix=True))
        self.assertEqual(calls, [self.cfgB], "스코프 밖 계정에 시드를 시도했다")
        self.assertEqual((_snapshot(self.cfgA), _snapshot(self.cfgC)), (beforeA, beforeC), "타 계정 dir 이 변했다")
        if not _exchange_supported():
            # ★R6: 이 픽스처의 자기 계정 config 는 **기존 문서**다 — 교환이 없으면 REFUSE 이고 바이트는 불변이다.
            #   스코프 계약(자기 계정만 호출·타 계정 무접촉)은 능력과 무관하게 그대로 단언한다.
            self.assertEqual(r["status"], pf.WARN, r)
            self.assertIn("exchange-unavailable", r["detail"])
            self.assertEqual(_read_bytes(bfile), rawB, "거부인데 기존 문서가 바뀌었다")
            return
        self.assertEqual(r["status"], pf.FIXED, r)
        self.assertIs(_read_json(bfile)["projects"][self.keyY]["hasTrustDialogAccepted"], True)
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
    # ★성찰 P1(blocking · 2026-09-10): `launch` 본체는 case 갈래가 아니라 **함수**(`launch_dept(){ … }`)다 —
    #   rotate 가 자식 프로세스(`bash "$0" launch … --rotate`)를 띄우면 프리루드·단일소유 게이트를 처음부터 다시
    #   돌게 되고, 자기 부서를 rotate 하는 CSO 는 **자기가 방금 죽인 데몬**에게 권위를 물어 exit 7 로 끝났다(부서가
    #   내려간 채 남는 반파괴). 인가를 마친 부모 안에서 본체를 부르도록 옮겼으므로 이 정적 핀도 **함수 본체**를 본다 —
    #   핀의 의도(시드가 데몬·셸보다 앞이고 하나의 cwd 가 시드·편성에 같이 간다)는 그대로다.
    if start_label == "launch":
        i = src.find("\nlaunch_dept(){")
        assert i > 0, "launch_dept() 함수 부재(본체가 다시 case 갈래로 돌아갔는가)"
        j = src.find("\n}\n", i)
        assert j > i, "launch_dept() 종결 부재"
        return src[i:j]
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

    def test_6c_allocate_records_the_resolved_cwd(self):
        """★R5(리뷰 minor): allocate 의 `reg_set_field cwd` 를 어떤 검체도 덮지 않았다(지워도 전수 통과) — 여기서 못 박는다.
        계약: ⓐ account_dir 등재 **뒤**, 데몬 스폰 **앞** ⓑ 값은 확정 물리 경로 `$dept_cwd`(allocate 의 원값 소스는
        `CYS_DEPT_CWD` 뿐이라 상대값이 재기동 호출자의 cwd 에 좌우되는 것을 막는다) ⓒ 실패는 fail-open WARN 1줄."""
        body = _block(self.src, "allocate", "create")
        i_acct = body.find('reg_set_field "$name" account_dir "$acctdir"')
        i_cwd = body.find('reg_set_field "$name" cwd "$dept_cwd"')
        i_daemon = body.find('nohup "$CYSD"')
        self.assertGreater(i_acct, 0, "allocate: account_dir 등재 부재")
        self.assertGreater(i_cwd, 0, "allocate: 확정 cwd 등재 부재(launch/rotate 가 좌석을 $HOME 으로 이주시킨다)")
        self.assertLess(i_acct, i_cwd)
        self.assertLess(i_cwd, i_daemon, "allocate: cwd 등재가 데몬 스폰 뒤")
        self.assertIn('|| echo "[cys-dept] WARN: $name cwd 등재 실패', body, "등재 실패가 fail-open WARN 이 아니다")
        self.assertNotIn('exit', body[i_cwd:body.find("\n", i_cwd)], "등재 실패가 부서 부트를 죽인다")

    def test_6d_allocate_and_create_cwd_are_read_the_same_way(self):
        """★R5(리뷰 minor · 의미 이종화 고지): create 는 카탈로그 **원값**을, allocate 는 **확정 물리 경로**를 같은
        `depts.json.depts[].cwd` 키에 쓴다. 소비자(preflight `_resolve_catalog_cwd` · 기동기 `resolve_dept_cwd`)는
        **존재하는 절대 dir 원값을 그대로** 두므로 두 표기의 해석이 갈리지 않는다 — 이 등가가 깨지면 여기서 잡힌다."""
        home = os.path.join(self.tmp_home, "home") if hasattr(self, "tmp_home") else tempfile.mkdtemp(prefix="deptcwd-")
        physical = tempfile.mkdtemp(prefix="deptcwd-real-")
        self.addCleanup(shutil.rmtree, physical, True)
        self.assertEqual(pf._resolve_catalog_cwd(physical, home=home), physical,
                         "확정 물리 경로가 카탈로그 해석에서 다른 값이 됐다(allocate 등재값의 의미가 갈린다)")
        self.assertEqual(pf.claude_project_key(pf._resolve_catalog_cwd(physical, home=home)),
                         pf.claude_project_key(physical))

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
        self.assertIn('_dept_cwd_is_root "$c" && c="$HOME"', fn2, "루트 cwd → $HOME 교정(cys.rs sanitize_launch_cwd 미러 · R3) 부재")
        self.assertIn('! _dept_cwd_is_root "$r"', fn2, "물리 경로 루트 거부 부재")
        self.assertIn('[ -d "$c" ]', fn2, "부재 dir 건너뛰기 부재(PTY 실패 = 좌석 0)")
        # ★R2 재핀: 채택 dir 는 절대·물리 경로(_dept_cwd_canon = cd -P && pwd -P · CDPATH 제거) · $HOME 폴백도 같은 규칙 · printf
        self.assertIn('r="$(_dept_cwd_canon "$c")"', fn2, "채택 dir 절대경로 확정 부재(CYS_DEPT_CWD=. 가 그대로 흐른다)")
        self.assertTrue(fn2.rstrip().endswith("""printf '%s\\n' "${r:-$HOME}"; return 0\n}"""), fn2)
        canon = re.search(r"^_dept_cwd_canon\(\)\{.*$", self.src, re.M).group(0)
        for tok in ("unset CDPATH", "cd -P --", "pwd -P", "|| true"):
            self.assertIn(tok, canon, canon)
        self.assertLess(self.src.find("_dept_cwd_canon(){"), m2.start(), "_dept_cwd_canon 정의가 resolve_dept_cwd 뒤")
        self.assertLess(m2.start(), self.src.find("\n  launch)"), "resolve_dept_cwd 정의가 첫 사용 뒤")
        rgf = re.search(r"^reg_get_field\(\)\{[^\n]*$", self.src, re.M).group(0)
        self.assertIn("| tr -d '\\r'", rgf, "reg_get_field 가 CRLF 를 벗기지 않는다(Windows 등재 cwd 복원 실패)")
        fm = re.search(r"^formation_ensure_async\(\)\{\n.*?^\}$", self.src, re.M | re.S).group(0)
        self.assertIn('ensure --socket "$sock" --cwd "$cwd" --json', fm, "formation 에 --cwd 전달 부재")
        self.assertIn('cwd="${3:-}"', fm)

    def _run_fn(self, fn_names, body, env=None, cwd=None, path_prepend=None):
        """cys-dept 에서 함수 실물을 추출해 격리 드라이버로 실행(test_dept_creds_seed.HelperUnit 동형)."""
        parts = []
        for fn in fn_names:
            m = (re.search(r"^%s\(\)\{[^\n]*\}[ \t]*$" % re.escape(fn), self.src, re.M)          # 한 줄 함수
                 or re.search(r"^%s\(\)\{.*?^\}$" % re.escape(fn), self.src, re.M | re.S))      # 여러 줄(닫는 } 단독 행)
            self.assertIsNotNone(m, fn)
            parts.append(m.group(0))
        driver = "set -u\nreg_init(){ :; }\n" + "\n".join(parts) + "\n" + body + "\n"
        d = tempfile.mkdtemp(prefix="deptfn-")
        _write(os.path.join(d, "driver.sh"), driver)
        e = dict(os.environ)
        if path_prepend:
            e["PATH"] = path_prepend + os.pathsep + e.get("PATH", "")
        if env:
            e.update(env)
        r = subprocess.run(["bash", os.path.join(d, "driver.sh")], capture_output=True, text=True, env=e,
                           cwd=cwd or d, timeout=60)
        return r, d

    def test_6d_reg_get_field_strips_crlf(self):
        """★R2(리뷰): Windows 네이티브 python 은 \\r\\n 을 찍는다 — reg_get_field 정의 1지점에서 CR 을 벗겨 호출부 3곳(:631 account_dir ·
        resolve_dept_cwd 등재 cwd · :1195 account)의 [ -d ]/[ -n ] 비교가 어긋나지 않게(python3 스텁으로 CRLF 재현)."""
        d = tempfile.mkdtemp(prefix="crlf-")
        stub = os.path.join(d, "bin")
        os.makedirs(stub)
        _write(os.path.join(stub, "python3"), '#!/bin/sh\nprintf "%s\\r\\n" "/some/registered/cwd"\n', 0o755)
        r, _ = self._run_fn(["reg_get_field"], 'REG=/dev/null\nv="$(reg_get_field n cwd)"\nprintf "[%s]" "$v"\n', path_prepend=stub)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, "[/some/registered/cwd]", repr(r.stdout))

    def test_6e_resolve_dept_cwd_returns_absolute_physical_path(self):
        """★R2(리뷰 codex major): CYS_DEPT_CWD=. · 상대 경로 · symlink/.. 는 절대·물리 경로로 확정(시드 키 == 좌석 getcwd) · "/" → $HOME ·
        부재 dir 건너뛰고 $HOME(물리) · 등재 조회는 명시값이 못 쓸 때만."""
        d = tempfile.mkdtemp(prefix="resolve-")
        home = os.path.join(d, "home")
        real = os.path.join(d, "real")
        os.makedirs(os.path.join(real, "child"))
        os.makedirs(home)
        link = os.path.join(d, "link")
        os.symlink(real, link)
        stub = os.path.join(d, "bin")
        os.makedirs(stub)
        _write(os.path.join(stub, "python3"), '#!/bin/sh\necho called >> "%s"\necho "%s"\n' % (os.path.join(d, "calls"), real), 0o755)
        body = ('REG=/dev/null\n'
                'for c in "." "child" "%s" "%s" "/" "%s" ""; do printf "%%s\\n" "$(resolve_dept_cwd "$c" "n")"; done\n'
                % (os.path.join(link, "child", ".."), link, os.path.join(d, "vanished")))
        r, _ = self._run_fn(["reg_get_field", "_dept_cwd_canon", "_dept_cwd_is_root", "resolve_dept_cwd"], body, env={"HOME": home, "CDPATH": "/tmp"},
                            cwd=real, path_prepend=stub)
        self.assertEqual(r.returncode, 0, r.stderr)
        got = r.stdout.splitlines()
        R = os.path.realpath
        self.assertEqual(got[:4], [R(real), R(os.path.join(real, "child")), R(real), R(real)], got)
        self.assertEqual(got[4], R(home), '"/" → $HOME(물리)')
        self.assertEqual(got[5], R(real), "부재 명시 dir → 등재 cwd(스텁 python 이 real 을 준다)")
        self.assertEqual(got[6], R(real), "빈 명시 → 등재 cwd")
        calls = _read_text(os.path.join(d, "calls")).count("called")
        self.assertEqual(calls, 2, "등재 조회는 명시값이 못 쓸 때만 python 을 띄운다(호출 %d)" % calls)
        for line in got:
            self.assertTrue(os.path.isabs(line) and line == R(line), "절대·물리 경로가 아니다: %s" % line)

    def test_6f_root_forms_resolve_to_home_and_match_preflight_catalog_resolution(self):
        """★R3(codex major): 루트 형("/" · "///" · "\\" · "C:\\" · 루트 심링크)과 부재 dir 는 $HOME(물리) — bash 기동기와 python
        레지스트리 소비자(_resolve_catalog_cwd → claude_project_key)가 같은 입력에 같은 값을 낸다(패리티)."""
        d = tempfile.mkdtemp(prefix="rootforms-")
        home = os.path.join(d, "home")
        ws = os.path.join(d, "ws")
        os.makedirs(home)
        os.makedirs(ws)
        rootlink = os.path.join(d, "rootlink")
        os.symlink("/", rootlink)
        inputs = ["/", "///", "\\\\", "C:\\", "c:", rootlink, ws, ws + "/", os.path.join(d, "vanished")]
        body = "REG=/dev/null\n" + "\n".join('printf "%%s\\n" "$(resolve_dept_cwd %s "")"' % shlex.quote(x) for x in inputs) + "\n"
        r, _ = self._run_fn(["reg_get_field", "_dept_cwd_canon", "_dept_cwd_is_root", "resolve_dept_cwd"], body,
                            env={"HOME": home, "CDPATH": "/tmp"}, cwd=ws)
        self.assertEqual(r.returncode, 0, r.stderr)
        got = r.stdout.splitlines()
        self.assertEqual(len(got), len(inputs), got)
        R = os.path.realpath
        for x, g in zip(inputs, got):
            with self.subTest(input=x):
                expect_py = pf.claude_project_key(pf._resolve_catalog_cwd(x, home=home)) if os.path.isabs(x) else R(home)
                self.assertEqual(g, expect_py, "bash 기동기와 python 소비자가 갈린다")
                self.assertNotEqual(g, "/", "루트가 그대로 나왔다")
        self.assertEqual(got[:6], [R(home)] * 6, got)
        self.assertEqual(got[6:8], [R(ws), R(ws)], got)
        self.assertEqual(got[8], R(home))

    def test_6g_cwd_verb_is_read_only_and_refuses_unknown(self):
        """★R3(리뷰 claude 심박 --cwd 공백 · codex: 읽기 전용 계약): `cys-dept cwd <name>` — 레지스트리 부재 → exit 3 · 파일 미생성 ·
        미등재/손상 → exit 3 · 부적격 이름 exit 2 · 등재 cwd 는 물리 경로 1줄 · "/" 등재 → $HOME · cys/cysd 스폰 0."""
        d = tempfile.mkdtemp(prefix="cwdverb-")
        home = os.path.join(d, "home")
        ws = os.path.join(d, "ws")
        bindir = os.path.join(d, "bin")
        os.makedirs(home)
        os.makedirs(ws)
        os.makedirs(bindir)
        calls = os.path.join(d, "calls")
        for tool in ("cys", "cysd"):
            _write(os.path.join(bindir, tool), '#!/bin/sh\necho "%s $@" >> "%s"\nexit 0\n' % (tool, calls), 0o755)
        reg = os.path.join(home, ".cys", "depts.json")
        env = dict(os.environ)
        env.update({"HOME": home, "USERPROFILE": home, "CYS_DEPTS_JSON": reg, "PATH": bindir + os.pathsep + env.get("PATH", "")})
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_DEPT_CWD", "CYS_SURFACE_ID"):
            env.pop(k, None)

        def verb(*args):
            return subprocess.run(["bash", DEPT, "cwd", *args], capture_output=True, text=True, env=env, cwd=d, timeout=60)

        r = verb("dept-1")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(r.stdout, "")
        self.assertFalse(os.path.exists(reg), "읽기 전용 verb 가 레지스트리를 만들었다(reg_init 경유)")
        os.makedirs(os.path.dirname(reg))
        _write(reg, json.dumps({"depts": {"dept-1": {"socket": "/s/1/cys.sock", "cwd": ws},
                                          "dept-2": {"socket": "/s/2/cys.sock", "cwd": "/"},
                                          "dept-3": {"socket": "/s/3/cys.sock"}}}))
        self.assertEqual(verb("dept-1").stdout.strip(), os.path.realpath(ws))
        self.assertEqual(verb("dept-2").stdout.strip(), os.path.realpath(home), '"/" 등재 → $HOME')
        # ★성찰 P4(major · 2026-09-10 · **의도적 방향 변경**): 등재 cwd 가 없으면 이 verb 는 `$HOME` 을 **확정값으로
        #   내주지 않는다** — 소비자는 편성 심박(`javis_formation ensure --cwd`)이고, 거기에 `$HOME` 을 실어 보내면
        #   시드되지 않은 폴더에서 좌석이 다시 떠 신뢰 관문에 걸린다(감사 에러 4 의 재발 경로). `$HOME` 폴백은
        #   launch 의 결정이지 이 verb 의 값이 아니므로 exit 4 · stdout 0 으로 접고, 소비자는 `--cwd` 를 생략해
        #   종전 동작으로 떨어진다. 음성 대조: 등재값이 있는 부서(dept-1·dept-2)는 그대로 값을 낸다(위 두 줄).
        r3 = verb("dept-3")
        self.assertEqual(r3.returncode, 4, r3.stdout + r3.stderr)
        self.assertEqual(r3.stdout, "", "확정값 없음인데 값을 냈다")
        self.assertIn("확정값 없음", r3.stderr)
        for bad in ("nope", ):
            r = verb(bad)
            self.assertEqual(r.returncode, 3, r.stderr)
            self.assertEqual(r.stdout, "")
        for bad in ("", "-x"):
            r = verb(bad)
            self.assertEqual(r.returncode, 2, r.stderr)
            self.assertEqual(r.stdout, "")
        before = _read_bytes(reg)
        _write(reg, "{broken")
        r = verb("dept-1")
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertEqual(_read_bytes(reg), b"{broken", "손상 레지스트리를 건드렸다")
        self.assertFalse(os.path.exists(calls), "verb 가 cys/cysd 를 스폰했다")
        usage = subprocess.run(["bash", DEPT, "--help"], capture_output=True, text=True, env=env, cwd=d, timeout=60)
        self.assertIn("cys-dept cwd <name>", usage.stdout + usage.stderr)

    def test_6c_daemon_lines_env_u_prefix(self):
        lines = [l for l in self.src.splitlines() if 'nohup "$CYSD"' in l]
        self.assertEqual(len(lines), 4, lines)
        for l in lines:
            # ★재핀(0.14.31 P6 R1 · 항목 추가): 핀의 의도("좌석 env 를 벗기고 데몬을 스폰한다")는
            #   그대로이고 벗기는 **목록이 늘었다**. `CYS_DEPT_ROTATE` 는 rotate 재귀 표식으로
            #   자식 cysd → 그 좌석 전부에 상속돼 부서 단일소유 게이트를 영구히 껐다(2026-09-08
            #   라이브 실측: dept-2 cysd pid 2634 · 좌석 4147/5087/7981). 이미 샌 값은 재기동으로 회수된다.
            self.assertIn("env -u CYS_ROLE -u CYS_SURFACE_ID -u CYS_SURFACE_REF"
                          " -u CYS_SEAT_TOKEN -u CYS_DEPT_ROTATE nohup", l, l)
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
        # ★R2: 그 값은 절대·물리 경로(= 시드 키 그대로)
        self.assertEqual(self._formation_cwd(argv), key)
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
        self.assertEqual(self._formation_cwd(argv), os.path.realpath(self.home), "거부에도 편성은 진행·같은 cwd")

    def test_7c_cwd_sources_explicit_registered_root(self):
        """★R1: CYS_DEPT_CWD(명시) > 등재 cwd(create 기록) > $HOME · "/" → $HOME · 부재 dir 건너뜀 — 시드·편성 동일값."""
        req = os.path.join(self.tmp, "requested")
        os.makedirs(req)
        r, argv = self._launch(CYS_DEPT_CWD=req)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self._formation_cwd(argv), pf.claude_project_key(req))
        self.assertIn(pf.claude_project_key(req), _read_json(os.path.join(self.fork, ".claude.json"))["projects"])
        regd = os.path.join(self.tmp, "registered")
        os.makedirs(regd)
        self._write_depts(cwd=regd)
        r, argv = self._launch()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self._formation_cwd(argv), pf.claude_project_key(regd), "등재 cwd 가 복원되지 않았다")
        self.assertIn(pf.claude_project_key(regd), _read_json(os.path.join(self.fork, ".claude.json"))["projects"])
        self._write_depts(cwd=os.path.join(self.tmp, "vanished"))
        r, argv = self._launch()
        self.assertEqual(self._formation_cwd(argv), os.path.realpath(self.home), "부재 등재 dir 를 편성에 넘겼다(PTY 실패 = 좌석 0)")
        r, argv = self._launch(CYS_DEPT_CWD="/")
        self.assertEqual(self._formation_cwd(argv), os.path.realpath(self.home), '"/" 는 $HOME 으로(cys.rs 루트 교정과 정합)')
        self.assertNotIn("/", _read_json(os.path.join(self.fork, ".claude.json"))["projects"], '"/" 가 신뢰 키로 들어갔다')
        # ★R2(codex major): CYS_DEPT_CWD=. (호출자 cwd = tmp) → 절대·물리 경로로 시드·편성 · 시드 ERROR 0
        r, argv = self._launch(CYS_DEPT_CWD=".")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("절대경로만 허용", r.stderr, "상대 cwd 가 시드까지 흘러 ERROR 를 냈다")
        self.assertEqual(self._formation_cwd(argv), pf.claude_project_key(self.tmp))
        self.assertIn(pf.claude_project_key(self.tmp), _read_json(os.path.join(self.fork, ".claude.json"))["projects"])


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
        """a: 거부 경로는 어떤 파일도 만들거나 바꾸지 않는다. ★R2 재핀: 프로브 거부(live/unverified)는 **기존 문서**가 있을 때만
        생기므로 그 종류는 문서를 둔 dir 로 재현(문서 바이트 불변 · 잔재 0) · 잠금 종류는 잠금 파일이 영속 설계라 미리 둔다 ·
        부재 dir + 프로브 스텁(호출되면 실패)은 프로브 없이 시드된다(Windows hub 좌석 아래 신규 부서 경로)."""
        for kind in ("live", "unverified", "lock-busy", "lock-unavailable"):
            with self.subTest(kind=kind):
                cfg = os.path.join(self.tmp, kind)
                os.makedirs(cfg)
                self.lock_fixture(cfg)                    # 잠금 파일은 영속 설계(획득이 만든다) — 스냅샷 비교 전에 둔다
                if not kind.startswith("lock"):
                    _write_any(os.path.join(cfg, ".claude.json"), {"projects": {}})
                before = _snapshot(self.tmp)
                count = {"live": 1, "unverified": None}.get(kind, 0)
                result = pf.seed_trust(cfg, self.ws, proc_counter=lambda d: (count, kind),
                                       lock_fn=lambda f: False if kind == "lock-busy" else None)
                self.assertEqual(result[:2], (2, "REFUSE"), result)
                self.assertEqual(_snapshot(self.tmp), before, result)
        fresh = os.path.join(self.tmp, "fresh-fork")
        result = pf.seed_trust(fresh, self.ws, proc_counter=lambda d: self.fail("부재 문서에 프로브를 돌렸다(R2)"),
                               lock_fn=lambda f: True)
        self.assertEqual(result[:2], (0, "OK"), result)
        self.assertIn("no-probe(", result[2])
        self.assertEqual(_read_json(os.path.join(fresh, ".claude.json")), {"projects": {self.key: {"hasTrustDialogAccepted": True}}})

    def test_a_unicode_normalized_and_config_equal_cwd(self):
        """a: 유니코드·꼬리 슬래시·'..' 세그먼트·cwd=config 도 정규화된 키 하나 · 거부는 보존."""
        unicode_ws = os.path.join(self.tmp, "작업-é-雪")
        os.makedirs(os.path.join(unicode_ws, "child"))
        for cwd in (self.cfg, unicode_ws + "/", unicode_ws + "/child/../"):
            with self.subTest(cwd=cwd):
                _write_any(self.file, {"projects": {}, "keep": ["雪"]})
                before = _read_bytes(self.file)
                result = pf.seed_trust(self.cfg, cwd, proc_counter=lambda d: (0, "test"), lock_fn=lambda f: True)
                if _refused_without_exchange(self, result, self.file, before):
                    continue                       # ★R6: 교환 없는 플랫폼의 계약은 REFUSE + 문서 보존이다
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
        # ★R3 재핀: 구분자 없는 모드에서 claude 형상 + 대상 불일치는 0 이 아니라 unresolved(인자가 env 를 가릴 수 있다) · argv 경계를
        #   주면 정확한 0
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude XCLAUDE_CONFIG_DIR=%s" % target], target), (0, 1, 1))
        self.assertEqual(pf._count_claude_in_ps_lines(
            ["424242 claude XCLAUDE_CONFIG_DIR=%s" % target], target, argv_lines=["424242 claude"]), (0, 1, 0))
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
        raw_before = _read_bytes(self.file)
        result = self.seed()
        if _refused_without_exchange(self, result, self.file, raw_before):
            # 교환 없는 플랫폼: 갭 판정(읽기 전용)은 그대로여야 하고 별칭 항목도 그대로다
            self.assertEqual(p._trust_gap_workspaces(self.file), [self.ws])
            self.assertEqual(_read_json(self.file), {"projects": {alias: {"hasTrustDialogAccepted": True, "keep": [1]}}})
            return
        self.assertEqual(result[:2], (0, "OK"))
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

        real_ex = pf._exchange_paths

        def exchanging(a, b):
            events.append("exchange")
            self.assertEqual(os.path.dirname(a), self.cfg)
            self.assertEqual(self.read(self.bak), original)
            return real_ex(a, b)

        # ★성찰 P2 조임(2026-09-10): 금지의 대상은 `os.replace` **호출 자체**가 아니라 **활성 문서를 덮는 rename** 이다.
        #   되교환 뒤 우리 사본을 보존하는 경로가 `os.replace(tmp, <방금 배타 생성한 conflict 이름>)` 을 쓴다(Windows 는
        #   대상이 있으면 `os.rename` 이 실패하므로 `replace` 가 유일한 이식 가능 이동이다). 술어를 목적지로 좁히면
        #   핀의 의도(교체 창을 rename-over 로 열지 않는다)는 더 정확해지고, 보존 이동은 통과한다.
        real_replace_fn = os.replace

        def replacing(src, dst, *a, **kw):
            self.assertNotEqual(os.path.abspath(dst), os.path.abspath(self.file),
                                "exchange path must not rename-over the active document")
            self.assertTrue(os.path.basename(dst).startswith(pf.SEED_TRUST_CONFLICT_PREFIX),
                            "보존 이동 외의 rename 이 생겼다: %s" % dst)
            return real_replace_fn(src, dst, *a, **kw)

        with patch.object(pf.os, "open", opening), patch.object(pf, "_exchange_paths", exchanging), \
                patch.object(pf.os, "replace", replacing):
            result = self.seed(backup=True, _pre_write_hook=hook)
        # ★R2 재핀(리뷰 codex BLOCK): 대조 뒤 백업 open 중 끼어든 기록자(late)는 교환이 드러낸다 → 되교환 → REFUSE · late 보존 ·
        #   백업은 캡처 바이트(원본) 그대로(1회 보존 계약) · 종전 핀은 'late 가 덮이고 OK' 였다(데이터 손실 핀 폐기).
        # ★R3: 교환 기구 없는 플랫폼에선 REFUSE exchange-unavailable — 백업 배타·캡처 바이트·late 보존 단언은 그대로 실행(codex D8).
        self.assertEqual(result[:2], (2, "REFUSE"), result)
        if _exchange_supported():
            self.assertIn("concurrent-change", result[2])
            self.assertEqual(events, ["hook", "backup", "exchange", "exchange"])
        else:
            self.assertIn("exchange-unavailable", result[2])
            self.assertEqual(events, ["hook", "backup", "exchange"])
        self.assertEqual(self.read(self.file), late, "끼어든 기록자의 내용이 덮였다")
        self.assertEqual(self.read(self.bak), original)
        self.assertFalse(any(n.startswith((".claude.json.seed-", ".claude.json.displaced-")) and n != pf.SEED_TRUST_LOCK_NAME
                             for n in os.listdir(self.cfg)), os.listdir(self.cfg))

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
        if not _exchange_supported():
            # ★R6: 교환이 없으면 커밋은 REFUSE 지만 **백업은 교환 전에** 만들어진다 — 경쟁 승자 보존 계약은 그대로다
            self.assertEqual(result[:2], (2, "REFUSE"), result)
            self.assertIn("exchange-unavailable", result[2])
            self.assertEqual(self.read(self.file), b"{}", "거부인데 문서가 바뀌었다")
            self.assertEqual(len(attempts), 1)
            self.assertTrue(attempts[0] & os.O_EXCL)
            self.assertEqual(self.read(self.bak), winner, "거부 경로가 백업 승자를 덮었다")
            return
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
                real_ex = pf._exchange_paths
                def exchanging(a, b):
                    r = real_ex(a, b)
                    if r:
                        self.write(b, payload)           # 커밋 직후(되읽기 前) 기록자
                    return r
                with patch.object(pf, "_exchange_paths", side_effect=exchanging) as exchange:
                    result = self.seed(backup=True)
                self.assertEqual(exchange.call_count, 1, "rollback attempted")
                self.assertEqual(result[:2], (2, "REFUSE"), result)
                if not _exchange_supported():
                    # ★R6: 교환이 없으면 post-commit 창 자체가 없다 — 기록자는 불리지 않고 문서는 불변이다
                    self.assertIn("exchange-unavailable", result[2])
                    self.assertEqual(self.read(self.file), b"{}", "거부인데 문서가 바뀌었다")
                    continue
                self.assertEqual(self.read(self.file), payload)
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
                real_ex = pf._exchange_paths
                def exchanging(a, b):
                    r = real_ex(a, b)
                    if r:
                        self.write(b, payload)
                    return r
                with patch.object(pf, "_exchange_paths", side_effect=exchanging) as exchange:
                    result = self.seed(backup=True)
                self.assertEqual(exchange.call_count, 1)
                if not _exchange_supported():
                    # ★R6: 교환이 없으면 커밋 후 형상 3종은 도달 불가다 — REFUSE · 문서 불변 · 백업은 캡처 바이트
                    self.assertEqual(result[:2], (2, "REFUSE"), result)
                    self.assertIn("exchange-unavailable", result[2])
                    self.assertEqual(self.read(self.file), original, "거부인데 문서가 바뀌었다")
                    self.assertEqual(self.read(self.bak), original, "백업이 캡처 바이트가 아니다")
                    os.unlink(self.bak)                     # 1회 보존 계약 — 다음 case 를 위해 초기화
                    continue
                self.assertEqual(result[:2], expected, result)
                self.assertIn(note, result[2])
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
        # ★R3 재핀: 구분자 없는 모드의 claude+env(CLAUDE_CONFIG_DIR 없음)+비기본 대상은 0 이 아니라 unresolved → (0, 4, 2) ·
        #   argv 경계를 주면 정확히 0 → (0, 4, 1)
        self.assertEqual(pf._count_claude_in_ps_lines(lines, self.cfg, {"710003"}), (0, 4, 2))
        argv = ["710001 claude", "710002 claude --continue", "710003 claude", "710004 python"]
        self.assertEqual(pf._count_claude_in_ps_lines(lines, self.cfg, {"710003"}, argv_lines=argv), (0, 4, 1))
        with patch.object(pf.os, "getpid", return_value=710003), patch.object(pf.os, "getppid", return_value=710004):
            for cfg, expected in ((alias, 1), (self.cfg, None)):
                count, detail = pf.claude_procs_for_config(cfg, os_name="posix", platform="darwin",
                                                         runner=_ps2("\n".join(lines)))
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
        target = os.path.join(self.root, "target-config")
        self.write(os.path.join(target, ".claude.json"), b'{"projects": {}}')   # ★R2: 프로브는 기존 문서가 있을 때 돈다
        # Attribute the positive process to the target config, then force must still refuse.
        self.write(os.path.join(readable, "environ"), b"CLAUDE_CONFIG_DIR=" + target.encode() + b"\0")
        with patch("builtins.open", opening), patch.object(pf.os, "getuid", return_value=os.stat(denied).st_uid, create=True):
            result = pf.seed_trust(target, self.ws, force_unverified=True, lock_fn=lambda f: True,
                                   proc_counter=lambda d: pf._count_claude_procfs(d, root))
        self.assertEqual(result[:2], (2, "REFUSE"), result)
        self.assertIn("live-claude", result[2])
        self.assertEqual(self.read(os.path.join(target, ".claude.json")), b'{"projects": {}}')

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


# ══ codex(gpt-6-astra) R2 적대 반례 18 — 워커가 전 행 검토 후 채택(초안 impl/codex/P1-WP2-trust-r2-tests-draft.py · 절대경로 BIN → 모듈 상수 ·
#   클래스명만 수정). 계약: 프로브 행렬(0B·별칭만·이미 신뢰 무프로브) · 부재 link/excl-create 무프로브 · 배타 생성 경합 · displaced 네임스페이스 ·
#   되교환 창 제3 쓰기 conflict 보존 · 되교환 실패 시 낯선 inode 보존 · 심링크 끼어듦 복원 · 기구 부재 폴백 vs 실 오류 · 청소 이름 정확성 ·
#   strict argv 위치 · 비정규 파일 열기 전 거부 · cwd 선검사 · chmod 실패 · topology 형상 · bash resolver(symlink/.. 물리 · 지연 조회 · CRLF).
class CodexR2Counterexamples(unittest.TestCase):
    def setUp(self):
        td = tempfile.TemporaryDirectory(prefix='r2-adversarial-')
        self.addCleanup(td.cleanup)
        self.root = Path(td.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir()
        self.cfg = self.root / 'account'
        self.cfg.mkdir()
        self.ws = self.root / 'workspace'
        self.ws.mkdir()
        self.file = self.cfg / '.claude.json'
        self.key = pf.claude_project_key(str(self.ws))
        env = {'HOME': str(self.home), 'USERPROFILE': str(self.home),
               'PATH': '/usr/bin:/bin', 'LOCALAPPDATA': str(self.root / 'local'),
               'XDG_STATE_HOME': str(self.root / 'state'),
               'JAVIS_ROOT': str(self.root / 'javis'),
               'CYS_DEPTS_JSON': str(self.root / 'depts.json')}
        p = patch.dict(os.environ, env, clear=True)
        p.start()
        self.addCleanup(p.stop)

    def seed(self, **kw):
        kw.setdefault('proc_counter', lambda d: (0, 't'))
        kw.setdefault('lock_fn', lambda f: True)
        return pf.seed_trust(str(self.cfg), str(self.ws), **kw)

    def result(self, got, rc, token):
        self.assertEqual(got[:2], (rc, {0: 'OK', 1: 'ERROR', 2: 'REFUSE'}[rc]), got)
        self.assertIn(token, got[2])

    def exchange_supported(self):
        if os.name != 'posix':
            self.skipTest('real atomic exchange requires POSIX')
        a, b = self.root / 'swap-a', self.root / 'swap-b'
        a.write_bytes(b'a')
        b.write_bytes(b'b')
        if pf._exchange_paths(str(a), str(b)) is not True:
            self.skipTest('filesystem has no atomic exchange')
        self.assertEqual((a.read_bytes(), b.read_bytes()), (b'b', b'a'))

    def leftovers(self, prefix):
        return sorted(p for p in self.cfg.glob('.claude.json.' + prefix + '-*') if p.name != pf.SEED_TRUST_LOCK_NAME)

    def test_01_probe_matrix_zero_bytes_and_exact_trust(self):
        # ★R5(리뷰 codex major): 강행(unverified)이 넘는 것은 **프로브 단계뿐**이다 — 기존 문서의 커밋은 원자 교환이
        #   있어야 하고, Windows·미지원 FS 는 그 자리에서 REFUSE exchange-unavailable 이 계약이다(강행 무관 · R3).
        #   종전 이 표는 그 커밋을 무조건 성공으로 단언해 Windows 에서 반드시 실패했다 → 능력으로 분기하되,
        #   거부 쪽에서도 '프로브는 정확히 1회' 와 '바이트 불변' 을 그대로 단언한다.
        exchange = _exchange_supported()
        for raw in (b'', b'{}', json.dumps({'projects': {self.key + '/':
                                      {'hasTrustDialogAccepted': True}}}).encode()):
            for count, force, rc, token in ((None, False, 2, 'unverified'),
                                           (None, True, 0, 'force-unverified'),
                                           (2, True, 2, 'live-claude')):
                if not exchange and (rc, token) == (0, 'force-unverified'):
                    rc, token = 2, 'exchange-unavailable('
                with self.subTest(raw=raw, count=count, force=force, exchange=exchange):
                    self.file.write_bytes(raw)
                    probe = Mock(return_value=(count, 't'))
                    self.result(self.seed(proc_counter=probe, force_unverified=force), rc, token)
                    probe.assert_called_once_with(str(self.cfg))
                    if rc:
                        self.assertEqual(self.file.read_bytes(), raw)
        self.file.write_text(json.dumps({'projects': {self.key: {'hasTrustDialogAccepted': True}}}))
        before = self.file.stat()
        probe = Mock(side_effect=AssertionError('trusted document probed'))
        self.result(self.seed(proc_counter=probe), 0, 'already-trusted')
        self.assertEqual(self.file.stat().st_mtime_ns, before.st_mtime_ns)
        probe.assert_not_called()

    def test_02_absent_link_never_probes_and_link_failure_refuses(self):
        """★R3 재핀(codex major): link 실패(EPERM/EXDEV)는 O_EXCL 배타 생성 폴백('commit=excl-create')이 아니라 REFUSE link-failed ·
        cfg 미생성 · 프로브 0 유지."""
        real = os.link
        with patch.object(pf.os, 'link', side_effect=real):
            got = self.seed(proc_counter=Mock(side_effect=AssertionError('absent probed')))
        self.result(got, 0, 'commit=link')
        self.assertIn('no-probe(', got[2])
        self.assertEqual(stat.S_IMODE(self.file.stat().st_mode), 0o600)
        self.assertEqual(json.loads(self.file.read_bytes()), {'projects': {self.key: {'hasTrustDialogAccepted': True}}})
        for failure in (errno.EPERM, errno.EXDEV):
            with self.subTest(failure=failure):
                self.file.unlink(missing_ok=True)
                with patch.object(pf.os, 'link', side_effect=OSError(failure, 'injected')):
                    got = self.seed(proc_counter=Mock(side_effect=AssertionError('absent probed')))
                self.result(got, 2, 'link-failed(%s' % errno.errorcode[failure])
                self.assertIn('no-probe(', got[2])
                self.assertFalse(self.file.exists())
                self.assertEqual(self.leftovers('seed'), [])

    def test_03_link_failure_never_opens_destination_for_writing(self):
        """★R3 재핀(codex major): 종전 'O_EXCL 경합 승자 무절단' 검체를 대체 — link 실패 뒤 cfg 경로에 대한 **쓰기 open 이 0** 이어야 한다
        (이름 선공개 경로 자체가 없다 · 승자 파일도 무접촉)."""
        real_open = os.open
        winner = b'creator\x00\xff'
        calls = []
        writable = os.O_WRONLY | os.O_RDWR | os.O_CREAT
        def opening(path, flags, *args, **kw):
            if os.fspath(path) == str(self.file) and flags & writable:
                calls.append(flags)
            return real_open(path, flags, *args, **kw)
        with patch.object(pf.os, 'link', side_effect=OSError(errno.EXDEV, 'injected')), \
                patch.object(pf.os, 'open', side_effect=opening):
            got = self.seed(proc_counter=Mock(side_effect=AssertionError('absent probed')))
        self.result(got, 2, 'link-failed')
        self.assertEqual(calls, [], 'cfg 경로를 열었다(O_EXCL 폴백 잔존)')
        self.assertFalse(self.file.exists())
        self.file.write_bytes(winner)
        with patch.object(pf.os, 'link', side_effect=OSError(errno.EXDEV, 'injected')), \
                patch.object(pf.os, 'open', side_effect=opening):
            got = self.seed(proc_counter=lambda d: (0, 't'))
        self.assertEqual(calls, [], calls)
        self.assertEqual(self.file.read_bytes(), winner)

    def test_04_exchange_uses_displaced_namespace_and_old_inode(self):
        self.exchange_supported()
        self.file.write_bytes(b'{}')
        old_inode = self.file.stat().st_ino
        rename, exchange = os.rename, pf._exchange_paths
        events = []
        def renaming(a, b):
            self.assertRegex(Path(a).name, r'^\.claude\.json\.seed-[A-Za-z0-9_]{8}$')
            self.assertRegex(Path(b).name, r'^\.claude\.json\.displaced-[0-9a-f]{64}-\d{8}T\d{6}Z-\d+(?:-\d+)?$')   # ★R3: payload 지문
            events.append('rename')
            return rename(a, b)
        def swapping(a, b):
            self.assertEqual(events, ['rename'])
            self.assertEqual(Path(b), self.file)
            out = exchange(a, b)
            self.assertEqual(os.stat(a).st_ino, old_inode)
            self.assertEqual(Path(a).read_bytes(), b'{}')
            events.append('exchange')
            return out
        with patch.object(pf.os, 'rename', renaming), patch.object(pf, '_exchange_paths', swapping):
            self.result(self.seed(), 0, 'commit=exchange')
        self.assertEqual(events, ['rename', 'exchange'])
        self.assertEqual(self.leftovers('displaced'), [])

    def test_05_third_writer_during_swapback_survives_as_conflict(self):
        self.exchange_supported()
        self.file.write_bytes(b'{}')
        exchange = pf._exchange_paths
        first, third = b'first writer', b'third writer\x00\xff'
        calls, inodes = [], []
        def swapping(a, b):
            calls.append(a)
            payload = first if len(calls) == 1 else third
            writer = self.root / 'writer'
            writer.write_bytes(payload)
            inodes.append(writer.stat().st_ino)
            os.replace(writer, b)
            return exchange(a, b)
        with patch.object(pf, '_exchange_paths', swapping):
            self.result(self.seed(), 2, 'concurrent-change')
        self.assertEqual(len(calls), 2)
        self.assertEqual(self.file.read_bytes(), first)
        self.assertEqual(self.file.stat().st_ino, inodes[0])
        conflicts = self.leftovers('conflict')
        self.assertEqual(len(conflicts), 1)
        self.assertEqual(conflicts[0].read_bytes(), third)
        self.assertEqual(conflicts[0].stat().st_ino, inodes[1])
        pf._sweep_stale_seed_tmp(str(self.cfg))
        self.assertEqual(conflicts[0].read_bytes(), third)

    def test_06_failed_swapback_retains_foreign_displaced_inode(self):
        self.exchange_supported()
        for failure in (None, OSError(errno.ENOENT, 'swapback failed')):
            with self.subTest(failure=failure):
                self.file.write_bytes(b'{}')
                exchange = pf._exchange_paths
                calls = []
                def swapping(a, b):
                    calls.append(a)
                    if len(calls) == 2:
                        if isinstance(failure, OSError):
                            raise failure
                        return failure
                    self.file.write_bytes(b'foreign unique bytes')
                    return exchange(a, b)
                with patch.object(pf, '_exchange_paths', swapping):
                    self.result(self.seed(), 1, '되교환 실패')
                self.assertEqual(len(calls), 2)
                self.assertEqual(Path(calls[0]).read_bytes(), b'foreign unique bytes')
                self.assertIs(json.loads(self.file.read_bytes())['projects'][self.key]['hasTrustDialogAccepted'], True)
                pf._sweep_stale_seed_tmp(str(self.cfg))
                self.assertEqual(Path(calls[0]).read_bytes(), b'foreign unique bytes')
                # ★R5(리뷰 codex D4 · 이 WP 자기 핀 확장): 되교환 실패는 **해결되지 않은 트랜잭션**이다 — 의도 저널이
                #   남아 다음 실행이 조용히 진행하지 못하게 막고(REFUSE), 두 파일은 무접촉이다. 사람이 병합하고 저널을
                #   지우기 전에는 계속 거부다(자가 치유 아님 = 데이터가 걸려 있다는 뜻).
                journals = self.leftovers('seed-intent')
                self.assertEqual(len(journals), 1, self.leftovers('seed-intent'))
                self.result(self.seed(), 2, 'interrupted-transaction')
                self.assertEqual(Path(calls[0]).read_bytes(), b'foreign unique bytes')
                journals[0].unlink()                       # 사람이 병합·정리한 자리
                Path(calls[0]).unlink()

    def test_07_foreign_symlink_is_restored_without_following_target(self):
        self.exchange_supported()
        self.file.write_bytes(b'{}')
        target = self.root / 'foreign-target'
        target.write_bytes(b'never touch target')
        exchange = pf._exchange_paths
        calls = []
        def swapping(a, b):
            calls.append(a)
            if len(calls) == 1:
                self.file.unlink()
                self.file.symlink_to(target)
            return exchange(a, b)
        with patch.object(pf, '_exchange_paths', swapping):
            self.result(self.seed(), 2, 'concurrent-change')
        self.assertEqual(len(calls), 2)
        self.assertTrue(self.file.is_symlink())
        self.assertEqual(os.readlink(self.file), str(target))
        self.assertEqual(target.read_bytes(), b'never touch target')

    def test_08_unreadable_own_inode_after_restore_is_preserved(self):
        self.exchange_supported()
        self.file.write_bytes(b'{}')
        exchange, read = pf._exchange_paths, pf._read_claude_json_bytes
        calls = []
        def swapping(a, b):
            calls.append(a)
            if len(calls) == 1:
                self.file.write_bytes(b'foreign')
            return exchange(a, b)
        def reading(path):
            if len(calls) == 2 and Path(path).name.startswith('.claude.json.displaced-'):
                raise PermissionError('cannot prove payload intact')
            return read(path)
        with patch.object(pf, '_exchange_paths', swapping), patch.object(pf, '_read_claude_json_bytes', reading):
            self.result(self.seed(), 2, 'concurrent-change')
        self.assertEqual(self.file.read_bytes(), b'foreign')
        conflicts = self.leftovers('conflict')
        self.assertEqual(len(conflicts), 1)
        self.assertIs(json.loads(conflicts[0].read_bytes())['projects'][self.key]['hasTrustDialogAccepted'], True)

    def test_09_exchange_unavailable_is_refusal_not_error_and_never_replaces(self):
        """★R3 재핀(codex BLOCK): 기구 부재 = REFUSE(강행 무관 · os.replace 0 · 사유에 why) · 실 오류 = ERROR · 어느 쪽도 원본 무접촉."""
        self.file.write_bytes(b'{}')
        for unavailable, why in ((pf._ExchangeUnavailable('EXDEV'), 'EXDEV'), (None, '')):
            for force in (False, True):
                with self.subTest(unavailable=unavailable, force=force):
                    with patch.object(pf, '_exchange_paths', return_value=unavailable), \
                            patch.object(pf.os, 'replace', side_effect=AssertionError('rename-over fallback')):
                        self.result(self.seed(force_unverified=force), 2, 'exchange-unavailable(' + why)
                    self.assertEqual(self.file.read_bytes(), b'{}')
                    self.assertEqual(self.leftovers('displaced'), [])
                    self.assertEqual(self.leftovers('seed'), [])
        with patch.object(pf, '_exchange_paths', side_effect=OSError(errno.ENOENT, 'vanished')), \
                patch.object(pf.os, 'replace', side_effect=AssertionError('error must not fall back')):
            self.result(self.seed(), 1, '쓰기 실패')
        self.assertEqual(self.file.read_bytes(), b'{}')
        self.assertFalse(pf._ExchangeUnavailable('x'), 'ExchangeUnavailable 은 falsy 여야 한다')
        self.assertIs(pf._exchange_paths.__globals__.get('_EXCHANGE_LAST_UNAVAILABLE'), None, '전역 가변 진단이 남아 있다')

    def test_10_sweep_exact_names_only_and_only_after_lock(self):
        import hashlib
        d_match = hashlib.sha256(b'precious').hexdigest()
        stale = ['.claude.json.seed-aB_019zX', '.claude.json.seed-________',
                 '.claude.json.displaced-%s-20260906T000000Z-1' % d_match]          # ★R3: 지문 일치 = payload 동등 잔재 회수
        kept = ['.claude.json.seed-lock', '.claude.json.seed-short', '.claude.json.seed-123456789',
                '.claude.json.seed-abcd-123', '.claude.json.displaced-12345678',
                '.claude.json.displaced-%s-20260906T000000Z-1' % ('0' * 64),          # 지문 불일치 = 낯선 inode 보존
                '.claude.json.displaced-%s-20260906T000000Z-1' % d_match[:16],       # 16hex 구형 = 지문 없음
                '.claude.json.conflict-%s-20260906T000000Z-1' % d_match,             # conflict 는 지문이 맞아도 무접촉
                '.claude.json.conflict-12345678', '.claude.json.bak-preflight']
        for name in stale + kept:
            (self.cfg / name).write_bytes(b'precious')
        sweep = pf._sweep_stale_seed_tmp
        held = []
        def sweeping(d, *a, **k):        # ★triage: 청소가 사유 꼬리(note)를 받는다
            self.assertTrue(held, 'sweep ran before lock acquisition')
            return sweep(d, *a, **k)
        def lock_ok(f):
            held.append(True)
            return True
        displaced = self.cfg / stale[-1]
        with patch.object(pf, '_sweep_stale_seed_tmp', sweeping):
            self.result(self.seed(lock_fn=lambda f: False), 2, 'lock-busy')
            self.assertTrue(all((self.cfg / n).exists() for n in stale))
            # ★R7 재핀(리뷰 codex major · plan §8 '의도적 기본값 변경만 재핀'): 활성 문서가 유효하지 않은데 저널 없는
            #   보존 사본이 있으면 **대체 문서를 만들지 않는다**. R6 는 사본을 남기고 계속 가서 '플래그만 든 새 문서'
            #   를 만들었고(rc 0), 그러면 다음 실행이 그 새 문서를 '건강' 으로 읽어 사본을 청소했다 — 두 번의
            #   성공적인 재시도가 사용자 필드를 영구히 지웠다. 이제 사본은 **자동 삭제되지 않는 이름**으로 옮겨지고
            #   이 실행은 REFUSE 다(다음 실행에는 사본이 없으므로 1회로 끝난다).
            self.result(self.seed(lock_fn=lock_ok), 2, 'unresolved-recovery')
            self.assertFalse(self.file.exists(), '거부인데 대체 문서를 만들었다')
            self.assertTrue(all((self.cfg / n).exists() for n in stale[:2]), '거부 경로가 잔재를 청소했다(무접촉 위반)')
            self.assertFalse(displaced.exists(), '보존 사본이 청소 네임스페이스에 그대로 남았다(보호가 1회용이다)')
            # ★성찰 P12 재핀(major · 2026-09-10 · **의도적 방향 변경**): 보존은 네임스페이스 이동이라 되돌릴 수 없다 —
            #   **같은 바이트가 conflict 에 이미 있으면 다시 보존하지 않는다**(중복 보존 자체가 무한 누적 축이다).
            #   이 픽스처의 `kept` 에는 b'precious' 를 담은 conflict 가 이미 둘 있으므로 **새** 사본은 0 이고,
            #   데이터는 그 쌍둥이가 계속 지킨다(데이터 손실 0 · 상한 있음).
            saved = [n for n in self.leftovers('conflict') if n.name not in kept]
            self.assertEqual(saved, [], [p.name for p in self.leftovers('conflict')])
            twins = [p.name for p in self.leftovers('conflict') if p.read_bytes() == b'precious']
            self.assertTrue(twins, '보존한 바이트가 어디에도 남지 않았다')
            # 재시도 2회차: 사본이 보호 네임스페이스로 빠졌으므로 이제 정상 시드다(잔재 청소는 mkstemp 2건만)
            self.result(self.seed(lock_fn=lock_ok), 0, 'stale-tmp swept 2')
        self.assertTrue(all(not (self.cfg / n).exists() for n in stale[:2]), 'mkstemp 잔재가 남았다')
        self.assertTrue([p for p in self.leftovers('conflict') if p.read_bytes() == b'precious'],
                        '보존 사본이 다음 실행에서 청소됐다')
        # ★triage I1 재핀(plan §8): 지문 청소의 근거는 '활성 문서가 유효하다' 가 **아니라 바이트 증명**이다.
        # 양성 대조: 활성이 바로 그 사본이면(순수 중복) 정상적으로 회수된다(청소 계약 보존)
        displaced.write_bytes(b'precious')
        self.file.write_bytes(b'precious')
        self.assertEqual(pf._sweep_stale_seed_tmp(str(self.cfg)), 1)
        self.assertFalse(displaced.exists(), '활성이 그 사본과 바이트 동등한데도 displaced 를 남겼다(청소가 죽었다)')
        # 음성 대조: 잘린 활성뿐 아니라 **유효하지만 다른** 활성 문서에서도 무접촉이다 — `{}` 재생성이 고아 보호를
        #   우회하던 자리(triage I1/I3 이 확정한 손실)
        for active in (b'', b'   \n', b'{}', b'{"projects": {}}'):
            displaced.write_bytes(b'precious')
            self.file.write_bytes(active)
            self.assertEqual(pf._sweep_stale_seed_tmp(str(self.cfg)), 0, active)
            self.assertTrue(displaced.exists(), active)
            self.assertEqual(displaced.read_bytes(), b'precious', active)
        self.assertEqual([(self.cfg / n).read_bytes() for n in kept], [b'precious'] * len(kept))

    def test_11_strict_argv_positions_and_hidden_visible_ps(self):
        cases = [(['node', '/x/claude-code/cli.mjs'], True),
                 (['/x/claude-code/cli.cjs'], True),
                 (['/x/claude/versions/2.1'], True), (['CLAUDE.EXE'], True),
                 (['node', '--inspect', '/x/claude-code/cli.js'], True),      # ★R3(codex): JS 런타임 뒤 어느 위치의 번들도 형상
                 (['bun', 'run', '/x/claude-code/cli.js'], True),
                 (['python3', 'x.py', '/x/claude-code/cli.js'], False),
                 (['tail', '/x/claude-code/debug.log'], False),
                 (['less', '/x/claude/versions/2.1'], False),
                 (['grep', 'claude.cmd'], False),
                 (['node', '/x/not-claude-code/cli.js'], False),
                 (['node', '/x/claude-code/cli.js.map'], False)]
        for tokens, expected in cases:
            with self.subTest(tokens=tokens):
                self.assertIs(pf._is_claude_command(tokens, strict=True), expected)
        lines = ['1 tail /logs/claude', '2 tail /logs/claude CLAUDE_CONFIG_DIR=' + str(self.cfg),
                 '3 node --inspect /x/claude-code/cli.js', '4 node /x/claude-code/cli.mjs',
                 '5 claude.cmd', '6 claude', 'junk', '7', '']
        self.assertEqual(pf._count_claude_in_ps_lines(lines, str(self.cfg), {'6'}), (1, 6, 3))   # ★R3: node --inspect 도 unresolved

    def test_12_nonregular_and_linklike_rejected_before_open(self):
        directory = self.root / 'directory'
        directory.mkdir()
        link = self.root / 'link'
        link.symlink_to(directory)
        paths = [directory, link]
        if hasattr(os, 'mkfifo'):
            fifo = self.root / 'fifo'
            os.mkfifo(fifo)
            paths.append(fifo)
        for path in paths:
            with self.subTest(path=path), patch.object(pf, '_open_nofollow') as opening:
                with self.assertRaises(ValueError):
                    pf._read_claude_json_bytes(str(path))
                opening.assert_not_called()  # A FIFO regression cannot hang this test.
        regular = self.root / 'junction-surrogate'
        regular.write_bytes(b'{}')
        with patch.object(pf, '_is_link_like', return_value=True), patch.object(pf, '_open_nofollow') as opening:
            with self.assertRaises(ValueError):
                pf._read_claude_json_bytes(str(regular))
            opening.assert_not_called()

    def test_13_invalid_cwd_preempts_even_lock_creation(self):
        missing = self.root / 'missing'
        regular = self.root / 'regular'
        regular.write_bytes(b'x')
        dangling = self.root / 'dangling'
        dangling.symlink_to(missing)
        for cwd in (missing, regular, dangling):
            with self.subTest(cwd=cwd), patch.object(pf.os, 'makedirs') as mkdir, \
                    patch.object(pf, '_open_nofollow') as opening:
                got = pf.seed_trust(str(self.root / 'uncreated'), str(cwd),
                                    proc_counter=Mock(side_effect=AssertionError('probe')), lock_fn=lambda f: True)
                self.result(got, 1, 'cwd')
                mkdir.assert_not_called()
                opening.assert_not_called()
        self.assertFalse((self.root / 'uncreated').exists())

    def test_14_chmod_failure_preempts_commit_and_backup(self):
        self.file.write_bytes(b'{}')
        before = self.file.stat()
        with patch.object(pf.os, 'chmod', side_effect=PermissionError('mode copy denied')), \
                patch.object(pf, '_exchange_paths') as exchange, patch.object(pf.os, 'replace') as replace:
            self.result(self.seed(backup=True), 1, '권한 보존 실패')
            exchange.assert_not_called()
            replace.assert_not_called()
        self.assertEqual(self.file.read_bytes(), b'{}')
        self.assertEqual(self.file.stat().st_ino, before.st_ino)
        self.assertEqual(set(p.name for p in self.cfg.iterdir()), {'.claude.json', pf.SEED_TRUST_LOCK_NAME})

    def test_15_topology_explicit_bad_entries_preserves_existing_diagnostics(self):
        topo = self.root / 'topology.json'
        for value in (None, 1, False, {}, 'x'):
            with self.subTest(value=value):
                topo.write_text(json.dumps({'entries': value}))
                reg = {'unreadable': ['earlier']}
                self.assertEqual(pf._topology_pairs(str(topo), reg), [])
                self.assertEqual(reg['unreadable'], ['earlier', str(topo)])
        good = {'agent': 'claude', 'cwd': str(self.ws), 'claude_config_dir': str(self.cfg)}
        for document, expected in (({}, []), ({'entries': [None, 1, False, 'x', [], good]},
                                             [(str(self.cfg), str(self.ws))])):
            topo.write_text(json.dumps(document))
            reg = {'unreadable': ['earlier']}
            self.assertEqual(pf._topology_pairs(str(topo), reg), expected)
            self.assertEqual(reg['unreadable'], ['earlier'])

    def shell(self, body, value='', home=None):
        source = Path(DEPT).read_text()
        parts = []
        for name in ('reg_get_field', '_dept_cwd_canon', '_dept_cwd_is_root', 'resolve_dept_cwd'):   # ★R3: 루트 판정 헬퍼 동반 추출
            m = (re.search(r'^%s\(\)\{[^\n]*\}[ \t]*$' % name, source, re.M)
                 or re.search(r'^%s\(\)\{.*?^\}$' % name, source, re.M | re.S))
            self.assertIsNotNone(m, name)
            parts.append(m.group())
        stub = self.root / 'bin'
        stub.mkdir(exist_ok=True)
        python = stub / 'python3'
        python.write_text('#!/bin/sh\nprintf "%s\\n" "$@" >> "$CALLS"\nprintf "%s\\r\\n" "$VALUE"\n')
        python.chmod(0o700)
        calls = self.root / 'calls'
        calls.unlink(missing_ok=True)
        env = dict(os.environ, PATH=str(stub) + ':/usr/bin:/bin', VALUE=value, CALLS=str(calls),
                   REG=str(self.root / 'dummy-registry'), HOME=str(home or self.home), CDPATH=str(self.home))
        driver = 'set -eu\nreg_init(){ :; }\n' + '\n'.join(parts) + '\n' + body
        r = subprocess.run(['/bin/bash', '--noprofile', '--norc', '-c', driver],
                           cwd=self.root, env=env, capture_output=True, timeout=10)
        self.assertEqual(r.returncode, 0, r.stderr)
        # ★성찰 P4 ⓒ(major · 2026-09-10 · **의도적 방향 변경**): 값이 있었는데 못 써서 폴백한 것은 이제 **침묵이 아니다**.
        #   `~` 미전개(카탈로그 `"cwd": "~/work/sales"`)가 리터럴로 남아 `[ -d ]` 거짓 → 무경고 `$HOME` 으로 접히던 자리가
        #   오설정을 영구히 숨겼다(종전 표현이던 'WARN: 셸 생성 실패' 라는 가시적 실패조차 사라졌다). stderr 는 비어 있거나
        #   `[cys-dept] WARN:` 줄뿐이어야 한다 — 그 밖의 stderr 출력은 여전히 계약 위반이다.
        self.last_stderr = r.stderr
        for line in r.stderr.splitlines():
            self.assertTrue(line.startswith(b'[cys-dept] WARN: '), r.stderr)
        return r.stdout, calls.read_text().splitlines() if calls.exists() else []

    def test_16_resolver_symlink_dotdot_is_physical_and_lazy(self):
        deep = self.root / 'physical' / 'deep'
        deep.mkdir(parents=True)
        (self.root / 'alias').symlink_to(deep)
        (self.home / 'alias').mkdir()  # CDPATH must not redirect cd.
        out, calls = self.shell('resolve_dept_cwd "alias/.." "dept name"', value=str(self.home))
        self.assertEqual(out, (str(deep.parent) + '\n').encode())
        self.assertEqual(calls, [], 'usable explicit path must not query registry')
        out, calls = self.shell('resolve_dept_cwd "." "dept name"')
        self.assertEqual(out, (str(self.root) + '\n').encode())
        self.assertEqual(calls, [])

    def test_17_registry_crlf_spaces_relative_path_and_fallback_home(self):
        target = self.root / 'registered space'
        target.mkdir()
        home_alias = self.root / 'home-alias'
        home_alias.symlink_to(self.home)
        # ★성찰 P4 ⓑ(major · 2026-09-10 · **의도적 방향 변경**): 등재값은 이제 **절대경로만** 인정한다 — 상대 등재값은
        #   그 파일을 읽는 프로세스의 cwd 에 따라 다른 폴더로 풀려 부서 좌석·시드가 조용히 이동한다(같은 결함을 R4 가
        #   allocate 에서 이미 고쳤고 create 카탈로그에만 남아 있었다). 공백이 든 이름은 그대로 지켜야 하므로
        #   픽스처를 **절대경로 + 공백**으로 올린다(핀의 원 의도 = CRLF·공백 보존).
        out, calls = self.shell('resolve_dept_cwd "missing" "dept name"', value=str(target), home=home_alias)
        self.assertEqual(out, (str(target) + '\n').encode())
        self.assertEqual(calls, ['-', str(self.root / 'dummy-registry'), 'dept name', 'cwd'])
        # ★성찰 P4 ⓒ: 못 쓴 명시값은 한 줄로 말한다(등재값이 대신 서더라도 — 요청이 조용히 무시된 것이다)
        self.assertIn(b"explicit cwd 'missing'", self.last_stderr)
        # 상대 등재값은 **부재로 취급**(호출자 cwd 의존) — 존재하는 상대 dir 이어도 쓰지 않는다(음성 대조)
        out, calls = self.shell('resolve_dept_cwd "" "dept name"', value='registered space', home=home_alias)
        self.assertEqual(out, (str(self.home) + '\n').encode())
        self.assertIn('절대경로가 아니다'.encode('utf-8'), self.last_stderr)
        for value in ('/', 'gone', ''):
            with self.subTest(value=value):
                out, calls = self.shell('resolve_dept_cwd "missing" "dept name"', value=value, home=home_alias)
                self.assertEqual(out, (str(self.home) + '\n').encode())
                self.assertEqual(len(calls), 4)
                self.assertIn(b"explicit cwd 'missing'", self.last_stderr)
                # ★성찰 P4 ⓑ: 상대 등재값은 **부재로 취급**한다(호출자 cwd 에 좌우돼 부서 폴더가 이동한다) — 그 사실도 한 줄로.
                if value == 'gone':
                    self.assertIn('등재 cwd \'gone\' 는 절대경로가 아니다'.encode('utf-8'), self.last_stderr)
                else:
                    self.assertNotIn('절대경로가 아니다'.encode('utf-8'), self.last_stderr)
        out, calls = self.shell('resolve_dept_cwd "/" "dept name"', value=str(target), home=home_alias)
        self.assertEqual(out, (str(self.home) + '\n').encode())
        self.assertEqual(calls, [])
        # 음성 대조: 루트 교정은 '못 쓴 값' 이 아니라 계약된 정규화다 — 경고 0(WARN 을 남발하면 아무도 안 읽는다)
        self.assertEqual(self.last_stderr, b'')

    def test_17b_tilde_registry_value_is_expanded_not_taken_literally(self):
        """★성찰 P4 ⓑ(major · 2026-09-10): 카탈로그가 `"cwd": "~/work/sales"` 를 주면 종전엔 `expandvars` 만 거쳐
        리터럴 `~/work/sales` 로 남았고 `[ -d ]` 가 거짓이라 **무경고 `$HOME`** 으로 접혔다(오설정 → 침묵). 읽는 쪽이
        `~`·`~/x` 를 푼다 — 하위호환(종전 등재값은 전부 절대경로라 이 갈래를 타지 않는다)."""
        (self.home / 'work').mkdir()
        (self.home / 'work' / 'sales').mkdir()
        out, _calls = self.shell('resolve_dept_cwd "" "dept name"', value='~/work/sales')
        self.assertEqual(out, (os.path.realpath(self.home / 'work' / 'sales') + '\n').encode())
        self.assertEqual(self.last_stderr, b'', '푼 값인데 경고를 냈다')
        out, _calls = self.shell('resolve_dept_cwd "" "dept name"', value='~')
        self.assertEqual(out, (os.path.realpath(self.home) + '\n').encode())

    def test_18_reg_get_field_strips_all_cr_without_losing_spaces(self):
        out, calls = self.shell('reg_get_field "department with spaces" account_dir', value='  /a\rb c  ')
        self.assertEqual(out, b'  /ab c  \n')  # bytes capture avoids universal-newline masking.
        self.assertEqual(calls, ['-', str(self.root / 'dummy-registry'), 'department with spaces', 'account_dir'])


# ══ codex(gpt-6-astra) R3 적대 반례 — 워커가 전 행 검토 후 채택(초안 impl/codex/P1-WP2-trust-r3-tests-draft.py · 스크래치 사본에서 실행 25/27) ══
# 채택 시 바뀐 것: T5 'python3 만' 단언 → cys/cysd 스폰 0 + coreutils 허용(위임 프롬프트의 과잉 명세) · T4 `C:\\` 실패는 스크래치 사본이
#   _abs_str 앞 해석 수정(이 트리) 이전 스냅샷이라 여기서는 통과 · harness 참조 → 모듈 내 이름. 구현 결함 발견 0.
class CodexR3Counterexamples(Base):
    def setUp(self):
        super().setUp()
        self.addCleanup(shutil.rmtree, self.tmp)
        self.root = Path(self.tmp)
        # Base already isolates HOME, USERPROFILE, LOCALAPPDATA, XDG_STATE_HOME,
        # CYS_DEPTS_JSON and JAVIS_ROOT. Keep pack/probe paths temporary as well.
        # ★R4(리뷰 BLOCKING): 종전 여기 있던 바이트코드 봉인 env 대입은 (a) 모듈 최상단 `import javis_preflight` 보다
        #   늦어 부모를 봉인하지 못하면서 (b) SEAL-1 census(test_pyseal_census.py ⓑ 참조 파일 집합 21)의 핀을 깨뜨렸다
        #   (실측 22 · CI 3레인·릴리스 레인 적색). 자식 파이썬은 호출부에서 `-B` 로 봉인한다.
        os.environ.update(CYS_PACK_DIR=str(self.root / 'pack'),
                          CYS_PROBE_RUNS=str(self.root / 'probes.jsonl'))
        self.src = Path(BIN, 'cys-dept').read_text()

    def require_exchange(self):
        a, b = self.root / 'exchange-a', self.root / 'exchange-b'
        a.write_bytes(b'left'); b.write_bytes(b'right')
        try:
            try:
                result = pf._exchange_paths(str(a), str(b))
            except OSError as exc:
                self.skipTest('real exchange unavailable: %s' % exc)
            if not result:
                self.skipTest('real exchange unavailable: %r' % result)
            self.assertIs(result, True)
            self.assertEqual((a.read_bytes(), b.read_bytes()), (b'right', b'left'))
        finally:
            a.unlink(missing_ok=True); b.unlink(missing_ok=True)

    def assert_clean(self, existing):
        expected = {pf.SEED_TRUST_LOCK_NAME}
        if existing:
            expected.add('.claude.json')
        self.assertEqual(set(os.listdir(self.cfg)), expected)

    def count(self, argv, envline, target='/target'):
        return pf._count_claude_in_ps_lines([envline], target, argv_lines=argv)

    def test_t1_argv_assignment_cannot_hide_real_env(self):
        self.assertEqual(self.count(['71 claude -p CLAUDE_CONFIG_DIR=/other'],
            '71 claude -p CLAUDE_CONFIG_DIR=/other CLAUDE_CONFIG_DIR=/target'), (1, 1, 0))

    def test_t1_target_argument_is_not_env(self):
        self.assertEqual(self.count(['71 claude -p CLAUDE_CONFIG_DIR=/target'],
            '71 claude -p CLAUDE_CONFIG_DIR=/target CLAUDE_CONFIG_DIR=/other'), (0, 1, 0))

    def test_t1_hidden_env_strict_shapes(self):
        for command, unresolved in [('claude', 1), ('node --inspect /x/claude-code/cli.js', 1),
                                    ('tail -f /x/logs/claude', 0),
                                    ('python3 x.py /x/claude-code/cli.js', 0)]:
            with self.subTest(command=command):
                self.assertEqual(pf._is_claude_command(command.split(), strict=True), bool(unresolved))
                line = '71 ' + command
                self.assertEqual(self.count([line], line), (0, 1, unresolved))

    def test_t1_unknown_mismatched_duplicate_argv_are_unresolved(self):
        line = '71 claude -p changed CLAUDE_CONFIG_DIR=/other'
        for argv in ([], ['72 claude'], ['71 claude -p old'],
                     ['71 claude -p changed', '71 claude -p changed']):
            with self.subTest(argv=argv):
                self.assertEqual(self.count(argv, line), (0, 1, 1))
        self.assertEqual(pf._ps_argv_map(['71 claude', '71 claude', '71 claude',
                                         '72 tail -f x', 'bad', 'xx claude']),
                         {'71': None, '72': 'tail -f x'})

    def test_t1_argv_prefix_requires_space_boundary(self):
        # Using the default target makes a bogus env suffix produce a false positive.
        self.assertEqual(self.count(['71 claude'], '71 claudeX HOME=/tmp',
                                    pf._default_claude_config_dir()), (0, 1, 0))

    def test_t1_delimiterless_cannot_verify_negative_claude(self):
        cases = [('claude CLAUDE_CONFIG_DIR=/other', (0, 1, 1)),
                 ('claude HOME=/tmp', (0, 1, 1)), ('claude', (0, 1, 1)),
                 ('tail -f /x/logs/claude', (0, 1, 0)),
                 ('sleep 5 CLAUDE_CONFIG_DIR=/target', (0, 1, 0)),
                 ('claude CLAUDE_CONFIG_DIR=/other CLAUDE_CONFIG_DIR=/target', (1, 1, 0))]
        for command, expected in cases:
            with self.subTest(command=command):
                self.assertEqual(self.count(None, '71 ' + command), expected)

    def test_t1_darwin_runner_order_and_failures(self):
        argv_cmd = ['ps', '-ax', '-ww', '-o', 'pid=,command=']
        env_cmd = ['ps', '-ax', '-ww', '-E', '-o', 'pid=,command=']
        for fails in (False, True):
            calls = []
            def runner(cmd):
                calls.append(cmd)
                if cmd == argv_cmd:
                    return (1, '', 'denied') if fails else (0, '987654 claude -p CLAUDE_CONFIG_DIR=/other', '')
                self.assertEqual(cmd, env_cmd)
                return 0, '987654 claude -p CLAUDE_CONFIG_DIR=/other CLAUDE_CONFIG_DIR=/target', ''
            with self.subTest(argv_failure=fails):
                n, _ = pf.claude_procs_for_config('/target', runner, 'posix', 'darwin')
                self.assertEqual(n, None if fails else 1)
                self.assertEqual(calls, [argv_cmd] if fails else [argv_cmd, env_cmd])
        # ★R4 재핀(이 WP 자기 핀 · 08496d2 R3 에서 워커가 넣은 것): 종전 계약은 "' NAME=' 대상은 ps **前** 에 거부" 였다.
        #   그 조기 반환은 unverified 를 만들고 `--force-unverified` 는 unverified 를 넘기므로, 그런 대상에서는 라이브
        #   claude 를 **관측할 기회 자체가 없어** 강행이 살아 있는 좌석의 config 를 덮었다(codex R4 설계 비평).
        #   새 계약: 스캔하고 분류한다 — 양성은 원문 바이트 대조로 관측(강행 불가) · 불일치는 unresolved → None ·
        #   claude 가 없으면 정당한 검증된 0. 판정은 같거나 더 안전하고 관측만 되살아난다.
        seen = []
        def scanning(cmd):
            seen.append(cmd)
            if cmd == argv_cmd:
                return 0, '987654 claude -p', ''
            return 0, '987654 claude -p CLAUDE_CONFIG_DIR=/target X=y HOME=/h', ''
        self.assertEqual(pf.claude_procs_for_config('/target X=y', scanning, 'posix', 'darwin')[0], 1,
                         "' NAME=' 대상의 라이브 claude 가 관측되지 않았다(강행이 넘어간다)")
        self.assertEqual(seen, [argv_cmd, env_cmd])
        def mismatching(cmd):
            if cmd == argv_cmd:
                return 0, '987654 claude -p', ''
            return 0, '987654 claude -p CLAUDE_CONFIG_DIR=/other HOME=/h', ''
        self.assertIsNone(pf.claude_procs_for_config('/target X=y', mismatching, 'posix', 'darwin')[0])
        def none_running(cmd):
            return (0, '987654 python x.py', '') if cmd == argv_cmd else (0, '987654 python x.py HOME=/h', '')
        self.assertEqual(pf.claude_procs_for_config('/target X=y', none_running, 'posix', 'darwin')[0], 0)

    def test_t1_env_value_assignment_never_verified_zero(self):
        for argv in (None, ['71 claude']):
            with self.subTest(argv=argv):
                n, parsed, unresolved = self.count(argv,
                    '71 claude CLAUDE_CONFIG_DIR=/other FOO=a CLAUDE_CONFIG_DIR=/target')
                self.assertEqual(parsed, 1)
                self.assertGreater(n + unresolved, 0)

    def test_t2_unavailable_never_replace_even_forced(self):
        self.untrusted_file('{"foreign":"keep exactly", "projects":{}}\n')
        original = _read_bytes(self.cfgfile)
        self.assertFalse(pf._ExchangeUnavailable('ENOTSUP'))
        for unavailable in (pf._ExchangeUnavailable('ENOTSUP'), None):
            for force in (False, True):
                with self.subTest(unavailable=unavailable, force=force):
                    with patch.object(pf, '_exchange_paths', return_value=unavailable) as swap, \
                         patch.object(pf.os, 'replace', side_effect=AssertionError('lossy replacement')) as replace:
                        rc, verdict, reason = self.seed(force_unverified=force)
                    self.assertEqual((rc, verdict), (2, 'REFUSE'), reason)
                    self.assertIn('exchange-unavailable(', reason)
                    if unavailable is not None:
                        self.assertIn('exchange-unavailable(ENOTSUP', reason)
                    swap.assert_called_once(); replace.assert_not_called()
                    self.assertEqual(_read_bytes(self.cfgfile), original)
                    self.assert_clean(existing=True)

    def test_t2_exchange_oserror_keeps_foreign_bytes(self):
        self.untrusted_file('{"foreign":"keep"}')
        original = _read_bytes(self.cfgfile)
        with patch.object(pf, '_exchange_paths', side_effect=OSError(errno.EIO, 'r3-exchange-failed')), \
             patch.object(pf.os, 'replace', side_effect=AssertionError('lossy replacement')):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, 'ERROR'), reason)
        self.assertIn('r3-exchange-failed', reason)
        self.assertEqual(_read_bytes(self.cfgfile), original)
        self.assert_clean(existing=True)

    def test_t2_link_errors_never_publish_or_open_writable(self):
        real_open = os.open
        for code in (errno.EPERM, errno.EXDEV, errno.ENOSPC, errno.EIO):
            opens = []
            def observe(path, flags, *args, **kwargs):
                opens.append((os.fspath(path), flags))
                if os.path.abspath(os.fspath(path)) == self.cfgfile:
                    self.assertFalse(flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC),
                                     'cfg opened for writing before atomic publication')
                return real_open(path, flags, *args, **kwargs)
            with self.subTest(errno=errno.errorcode[code]):
                with patch.object(pf.os, 'link', side_effect=OSError(code, 'injected')) as link, \
                     patch.object(pf.os, 'open', side_effect=observe):
                    rc, verdict, reason = self.seed()
                link.assert_called_once()
                self.assertEqual((rc, verdict), (2, 'REFUSE'), reason)
                self.assertIn('link-failed(' + errno.errorcode[code], reason)
                self.assertTrue(any(Path(path).name == pf.SEED_TRUST_LOCK_NAME
                                    for path, flags in opens), 'open spy did not observe lock')
                self.assertFalse(os.path.lexists(self.cfgfile))
                self.assert_clean(existing=False)

    def test_t2_surrogate_preserved_and_cli_no_traceback(self):
        self.require_exchange()
        raw = r'{"projects":{"/x":{"note":"\ud800"}}}'
        self.untrusted_file(raw)
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (0, 'OK'), reason)
        self.assertIn('ascii-escaped', reason)
        self.assertEqual(_read_json(self.cfgfile)['projects']['/x']['note'], '\ud800')
        self.assertIs(_read_json(self.cfgfile)['projects'][self.key]['hasTrustDialogAccepted'], True)
        self.untrusted_file(raw)  # CLI must serialize too, not just take already-trusted.
        rc, out, err = seed_cli(self.cfg, self.ws)
        self.assertEqual(rc, 0, (out, err))
        self.assertNotIn('Traceback', err)
        self.assertIn('ascii-escaped', out)
        self.assertEqual(_read_json(self.cfgfile)['projects']['/x']['note'], '\ud800')
        self.assertIs(_read_json(self.cfgfile)['projects'][self.key]['hasTrustDialogAccepted'], True)

    def test_t2_post_commit_refuse_and_error_keep_commit_note(self):
        real_link = os.link
        for foreign, expected in [(b'{"foreign":42}', (2, 'REFUSE')), (b'{broken', (1, 'ERROR'))]:
            with self.subTest(expected=expected):
                Path(self.cfgfile).unlink(missing_ok=True)
                def commit_then_foreign(src, dst):
                    real_link(src, dst)
                    Path(dst).write_bytes(foreign)
                with patch.object(pf.os, 'link', side_effect=commit_then_foreign) as link:
                    rc, verdict, reason = self.seed()
                link.assert_called_once()
                self.assertEqual((rc, verdict), expected, reason)
                self.assertIn('commit=link', reason)
                self.assertEqual(_read_bytes(self.cfgfile), foreign)

    def test_t3_crash_before_exchange_is_swept_on_retry(self):
        self.require_exchange()
        self.untrusted_file('{"foreign":"original"}')
        before = _read_bytes(self.cfgfile)
        with patch.object(pf, '_exchange_paths', side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        self.assertEqual(_read_bytes(self.cfgfile), before)
        displaced = list(Path(self.cfg).glob('.claude.json.displaced-*'))
        self.assertEqual(len(displaced), 1)
        payload = displaced[0].read_bytes()
        self.assertIs(json.loads(payload)['projects'][self.key]['hasTrustDialogAccepted'], True)
        self.assertIn(hashlib.sha256(payload).hexdigest(), displaced[0].name)
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (0, 'OK'), reason)
        self.assertIn('recovery-reclaimed(1', reason)   # ★triage I1 재핀: 저널이 있으면 회수 ⓑ 가 스스로 회수한다
        self.assertIn('commit=exchange', reason)
        self.assertFalse(displaced[0].exists())
        self.assert_clean(existing=True)

    def test_t3_digest_is_full_sha256(self):
        payload = b'not a truncated digest\x00\xff'
        expected = hashlib.sha256(payload).hexdigest()
        self.assertEqual(pf._payload_digest(payload), expected)
        self.assertEqual(len(pf._payload_digest(payload)), 64)
        self.assertIsNone(pf._SEED_DISPLACED_RE.match('.claude.json.displaced-' + expected[:16] + '-x'))
        self.assertIsNotNone(pf._SEED_DISPLACED_RE.match('.claude.json.displaced-' + expected + '-x'))

    def test_t3_sweep_preserves_foreign_legacy_symlink_fifo_conflict(self):
        os.makedirs(self.cfg)
        data = b'precious foreign bytes'
        digest = hashlib.sha256(data).hexdigest()
        prefix = '.claude.json.displaced-'
        protected = {}
        for name, content in [(prefix + digest + '-mismatch', b'different bytes'),
                              (prefix + '20260906T000000Z-1', data),
                              ('.claude.json.conflict-' + digest + '-x', data),
                              (prefix + digest[:16] + '-short', data)]:
            path = Path(self.cfg, name); path.write_bytes(content); protected[path] = content
        target = self.root / 'symlink-target'; target.write_bytes(data)
        symlink = Path(self.cfg, prefix + digest + '-symlink'); symlink.symlink_to(target)
        fifo = Path(self.cfg, prefix + digest + '-fifo')
        if hasattr(os, 'mkfifo'):        # ★R4: Windows 엔 mkfifo 부재 — 그 항목만 건너뛴다
            os.mkfifo(fifo)
        good = Path(self.cfg, prefix + digest + '-good'); good.write_bytes(data)
        # ★triage I1 재핀(plan §8 · R6 재핀의 후속): 지문 청소의 전제가 '활성 문서가 유효하다' 에서 **바이트 증명**
        #   (활성 = 그 사본)으로 바뀌었다 — '유효한 문서' 는 '그 데이터가 그 안에 있다' 가 아니어서, 외부가 활성을
        #   `{}` 로 재생성하기만 하면 종전 규칙이 유일한 사용자 사본을 지웠다. 양성 대조의 활성을 그 바이트로 두고,
        #   아래에서 **잘린·다른 활성 문서**로 무접촉을 못 박는다.
        active = Path(self.cfg, '.claude.json'); active.write_bytes(data)
        # Run in a child with Python's portable timeout: a wrong FIFO open cannot hang the suite.
        script = ('import sys; sys.path.insert(0, sys.argv[1]); import javis_preflight as pf; '
                  'print(pf._sweep_stale_seed_tmp(sys.argv[2]))')

        def sweep_in_child():
            try:
                return subprocess.run([sys.executable, '-B', '-c', script, BIN, self.cfg],   # -B: 저장소 트리에 __pycache__ 0(SEAL-1)
                                      capture_output=True, text=True, env=dict(os.environ), timeout=10)
            except subprocess.TimeoutExpired:
                self.fail('sweep hung while encountering a FIFO')

        r = sweep_in_child()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, '1\n')
        self.assertFalse(good.exists(), 'positive control was not swept')
        for raw in (b'', b' \t\n', b'{broken', b'[]', b'{}', b'{"projects": {}}'):
            with self.subTest(active=raw):
                good.write_bytes(data)
                active.write_bytes(raw)
                r = sweep_in_child()
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(r.stdout, '0\n', '활성이 그 사본과 바이트 동등하지 않은데 displaced 를 지웠다')
                self.assertEqual(good.read_bytes(), data)
        good.unlink()
        for path, content in protected.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertTrue(symlink.is_symlink()); self.assertEqual(target.read_bytes(), data)
        if hasattr(os, 'mkfifo'):
            self.assertTrue(stat.S_ISFIFO(fifo.lstat().st_mode))

    def registry(self, entries):
        os.makedirs(self.cfg, exist_ok=True)
        path = Path(os.environ['CYS_DEPTS_JSON']); path.parent.mkdir(parents=True, exist_ok=True)
        _write(path, json.dumps({'depts': entries}))
        # Match RegistryC58's full-scope fixture; all discovered paths are under HOME.
        with patch.object(pf, '_discover_isolation_block', return_value=(None, None)):
            return pf.cysjavis_registry()

    def test_t4_catalog_roots_absent_and_physical_paths(self):
        alias = self.root / 'root-alias'; alias.symlink_to('/')
        for value in ('/', '///', str(self.root / 'absent'), str(alias), self.ws):
            with self.subTest(cwd=value):
                reg = self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': value}})
                expected = self.key if value == self.ws else os.path.realpath(self.home)
                self.assertEqual(set(reg['pairs'].get(pf._path_identity(self.cfg), {})), {expected})
                p = _IsoEnv._pf(False); p._registry_cache = reg
                gaps = p._trust_gap_workspaces(self.cfgfile)
                self.assertEqual([pf.claude_project_key(x) for x in gaps], [expected])
                self.assertNotIn('/', gaps)
                self.assertTrue(all(os.path.isdir(x) for x in gaps))

    def test_t4_catalog_windows_root_maps_to_home(self):
        reg = self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': 'C:\\'}})
        self.assertEqual(set(reg['pairs'].get(pf._path_identity(self.cfg), {})),
                         {os.path.realpath(self.home)}, 'drive root must reach catalog resolver on POSIX too')

    def test_t4_relative_catalog_excluded_topology_root_preserved(self):
        hub = Path(pf._hub_state_dir()); hub.mkdir(parents=True)
        _write(hub / 'topology.json', json.dumps({'entries': [
            {'agent': 'claude', 'claude_config_dir': self.cfg, 'cwd': '/'}]}))
        reg = self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': 'relative/work'}})
        self.assertEqual(reg['pairs'][pf._path_identity(self.cfg)], {'/': '/'})
        # Root filtering above is a catalog contract; observed topology is intentionally retained.

    def shell_functions(self, body):
        parts = []
        for name in ('_dept_cwd_canon', '_dept_cwd_is_root', 'resolve_dept_cwd'):
            m = (re.search(r'^%s\(\)\{[^\n]*\}[ \t]*$' % name, self.src, re.M)
                 or re.search(r'^%s\(\)\{.*?^\}$' % name, self.src, re.M | re.S))
            self.assertIsNotNone(m, name); parts.append(m.group())
        driver = 'set -eu\nreg_get_field(){ :; }\n' + '\n'.join(parts) + '\n' + body
        return subprocess.run(['/bin/bash', '--noprofile', '--norc', '-c', driver],
                              capture_output=True, text=True, cwd=self.tmp, env=dict(os.environ), timeout=10)

    def test_t4_bash_python_absolute_physical_parity(self):
        for value in ('/', '///', 'C:\\', self.ws, self.ws + '/', str(self.root / 'missing')):
            with self.subTest(cwd=value):
                r = self.shell_functions('resolve_dept_cwd %s ""' % shlex.quote(value))
                self.assertEqual(r.returncode, 0, r.stderr)
                expected = pf.claude_project_key(pf._resolve_catalog_cwd(value, home=self.home))
                self.assertEqual(r.stdout, expected + '\n')
                # ★성찰 P4 ⓒ(2026-09-10): 값이 있었는데 **못 써서** $HOME 으로 접힌 것만 WARN 1줄이다 — 루트 교정(`/`·`///`·
                #   `C:\`)은 계약된 정규화라 여전히 침묵이고, 부재 dir 은 이제 말한다(무경고 $HOME 이 오설정을 숨기던 자리).
                if os.path.isabs(value) and not pf._is_root_cwd(value) and not os.path.isdir(value):
                    self.assertIn("WARN", r.stderr)
                    self.assertIn(value, r.stderr)
                else:
                    self.assertEqual(r.stderr, '')
                self.assertTrue(os.path.isabs(expected))
                root = self.shell_functions('_dept_cwd_is_root %s' % shlex.quote(value))
                self.assertEqual(root.returncode, 0 if pf._is_root_cwd(value) else 1)

    def dept(self, name):
        stub = Path(self.home, '.local', 'bin'); stub.mkdir(parents=True, exist_ok=True)
        calls = self.root / 'children'
        # cys-dept prepends this HOME directory itself. Prevent host cys/cysd resolution.
        for command in ('cys', 'cysd'):
            _write(stub / command, '#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(command) +
                   ' >> "$R3_CALLS"\nexit 97\n', 0o755)
        for command in ('python3', 'grep', 'tr', 'tail'):
            real = sys.executable if command == 'python3' else shutil.which(command)
            self.assertIsNotNone(real)
            _write(stub / command, '#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(command) +
                   ' >> "$R3_CALLS"\nexec ' + shlex.quote(real) + ' "$@"\n', 0o755)
        calls.unlink(missing_ok=True)
        env = dict(os.environ, PATH=str(stub) + ':/usr/bin:/bin', R3_CALLS=str(calls))
        r = subprocess.run(['/bin/bash', str(Path(BIN, 'cys-dept')), 'cwd', name],
                           capture_output=True, text=True, cwd=self.tmp, env=env, timeout=10)
        spawned = calls.read_text().splitlines() if calls.exists() else []
        self.assertNotIn('cys', spawned); self.assertNotIn('cysd', spawned)
        return r, spawned

    def test_t5_absent_registry_stays_absent(self):
        path = Path(os.environ['CYS_DEPTS_JSON'])
        self.assertFalse(path.exists())
        r, _ = self.dept('dept-1')
        self.assertEqual(r.returncode, 3, r.stderr); self.assertEqual(r.stdout, '')
        self.assertFalse(path.exists()); self.assertFalse(path.parent.exists())

    def test_t5_unregistered_and_malformed_are_read_only(self):
        path = Path(os.environ['CYS_DEPTS_JSON']); path.parent.mkdir(parents=True)
        for raw in ('{"depts":{"someone-else":{}}}\n', '{malformed\n'):
            with self.subTest(raw=raw):
                _write(path, raw); before = path.read_bytes()
                r, _ = self.dept('dept-1')
                self.assertEqual(r.returncode, 3, r.stderr); self.assertEqual(r.stdout, '')
                self.assertEqual(path.read_bytes(), before)

    def test_t5_unreadable_registry_is_read_only(self):
        self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': self.ws}})
        path = Path(os.environ['CYS_DEPTS_JSON'])
        before = path.read_bytes()
        path.chmod(0)
        try:
            if os.access(path, os.R_OK):
                self.skipTest('effective user can read mode-000 files')
            r, _ = self.dept('dept-1')
            self.assertEqual(r.returncode, 3, r.stderr)
            self.assertEqual(r.stdout, '')
        finally:
            path.chmod(0o600)
        self.assertEqual(path.read_bytes(), before)

    def test_t5_registered_root_and_real_directory(self):
        alias = self.root / 'workspace-alias'; alias.symlink_to(self.ws)
        for value, expected in [('/', os.path.realpath(self.home)), (self.ws, self.key), (str(alias), self.key)]:
            with self.subTest(cwd=value):
                self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': value}})
                path = Path(os.environ['CYS_DEPTS_JSON']); before = path.read_bytes()
                r, _ = self.dept('dept-1')
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertEqual(r.stdout, expected + '\n')
                self.assertEqual(path.read_bytes(), before)

    def test_t5_bad_names(self):
        for name in ('', '-x'):
            with self.subTest(name=name):
                r, _ = self.dept(name)
                self.assertEqual(r.returncode, 2, r.stderr); self.assertEqual(r.stdout, '')
                self.assertFalse(Path(os.environ['CYS_DEPTS_JSON']).exists())

    def test_t5_cwd_spawns_only_python3_and_coreutils(self):
        # 워커 검토 수정: 위임 프롬프트의 'python3 만' 은 과잉 명세 — 읽기 전용 계약은 cys/cysd 스폰 0(dept() 가 단언)이고
        #   reg_names/reg_get_field 파이프라인의 coreutils(grep/tr/tail)는 허용된다.
        self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': self.ws}})
        r, spawned = self.dept('dept-1')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('python3', spawned, 'child-command spy positive control')
        self.assertLessEqual(set(spawned), {'python3', 'grep', 'tr', 'tail'}, spawned)

    def test_t6_c58_exception_warns_and_continues_success(self):
        second = self.root / 'zz-second'; second.mkdir()
        reg = self.registry({'dept-1': {'account_dir': self.cfg, 'cwd': self.ws},
                             'dept-2': {'account_dir': self.cfg, 'cwd': str(second)}})
        p = _IsoEnv._pf(True); p._registry_cache = reg
        def seed(config, cwd, **kw):
            self.assertEqual(config, self.cfg); self.assertTrue(kw['backup'])
            if cwd == self.ws:
                raise RuntimeError('r3-injected-failure')
            self.assertEqual(cwd, str(second))
            return 0, 'OK', 'seeded(r3-success)'
        with patch.object(pf, 'seed_trust', side_effect=seed) as mock_seed:
            p.c58_trust_harden()
        self.assertEqual([call.args[1] for call in mock_seed.call_args_list], [self.ws, str(second)])
        self.assertEqual(len(p.results), 1)
        result = p.results[0]
        self.assertEqual(result['id'], 'C58.trust-harden'); self.assertEqual(result['status'], pf.WARN)
        self.assertIn('RuntimeError', result['detail']); self.assertIn('r3-injected-failure', result['detail'])
        self.assertIn('trust set:', result['detail']); self.assertIn(str(second), result['detail'])
        self.assertIn('seeded(r3-success)', result['detail'])


# ══ R4(리뷰 반영 4 · 워커 작성) — 비정규 파일 무한대기 · ps 꼬리 모호 대상 · 커밋 뒤 청소 · C43 잠금 · CLI 마감 감시 ══
_CHILD_TIMEOUT = 20          # 자식 프로세스 상한(초) — macOS 에 `timeout` 명령이 없으므로 subprocess 인자로만(플랜 §0)


def _child(code, *args, env=None, timeout=_CHILD_TIMEOUT):
    """저장소 bin 을 import 하는 자식 파이썬 1회 실행 → CompletedProcess. `-B`: 저장소 트리에 __pycache__ 0(SEAL-1).
    막힐 수 있는 검체는 전부 이 경로로 — 잘못된 open 이 suite 를 영구 정지시키지 못한다."""
    script = "import sys; sys.path.insert(0, %r)\n" % BIN + code
    e = dict(os.environ)
    if env:
        e.update(env)
    return subprocess.run([PY, "-B", "-c", script, *args], capture_output=True, text=True,
                          encoding="utf-8", env=e, timeout=timeout)


class R4NonRegularAndProbe(Base):
    """★R4 리뷰(codex major 2건): ①비정규 파일(FIFO)에서의 영구 블록 ②꼬리 공백 대상의 거짓 '검증된 0'."""

    def setUp(self):
        super().setUp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        os.makedirs(self.cfg)

    def _fifo(self, path):
        if not hasattr(os, "mkfifo"):
            self.skipTest("mkfifo 부재(Windows) — FIFO 반례는 POSIX 계약")
        os.mkfifo(path)

    # ── T1 비정규 파일: C58 report/fix 가 멈추지 않는다 ──
    def _c58_child(self, fix):
        """격리 HOME 에서 C58 1회 실행(자식) — stdout 마지막 줄 = 'status|detail 앞 80자'."""
        code = (
            "import json, os\n"
            "import javis_preflight as pf\n"
            "pf._discover_isolation_block = lambda: (None, None)\n"
            "p = pf.Preflight(fix=(sys.argv[1] == '1'), skips=[], mode=('fix' if sys.argv[1] == '1' else 'report'),\n"
            "                 allow_irreversible=False)\n"
            "p.c58_trust_harden()\n"
            "r = [x for x in p.results if x['id'] == 'C58.trust-harden']\n"
            "print('RESULT', r[0]['status'], (r[0]['detail'] or '')[:120].replace(chr(10), ' '))\n")
        return _child(code, "1" if fix else "0")

    def test_r4_c58_fifo_config_hangs_neither_report_nor_fix(self):
        """FIFO `.claude.json`(writer 없음) — 종전 `_read_json_tolerant` 의 무가드 open 이 report·fix 양쪽을 영구 정지시켰다."""
        reg = Path(os.environ["CYS_DEPTS_JSON"])
        reg.parent.mkdir(parents=True, exist_ok=True)
        _write(str(reg), json.dumps({"depts": {"dept-1": {"account_dir": self.cfg, "cwd": self.ws}}}))
        self._fifo(self.cfgfile)
        for fix in (False, True):
            with self.subTest(fix=fix):
                try:
                    r = self._c58_child(fix)
                except subprocess.TimeoutExpired:
                    self.fail("C58(%s)가 FIFO .claude.json 에서 멈췄다(무한 대기)" % ("fix" if fix else "report"))
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("RESULT", r.stdout)
                self.assertIn("WARN", r.stdout.split("RESULT", 1)[1][:20], r.stdout)
        self.assertTrue(stat.S_ISFIFO(os.lstat(self.cfgfile).st_mode), "검체가 FIFO 를 정규 파일로 바꿔 놨다")

    def test_r4_seed_trust_on_fifo_refuses_without_blocking(self):
        """시더 자신도 FIFO 에서 막히지 않고 ERROR(무쓰기) — 비정규 거부는 선검사와 fstat 이 이중으로 한다."""
        self._fifo(self.cfgfile)
        code = ("import javis_preflight as pf\n"
                "print('RC', pf.seed_trust(sys.argv[1], sys.argv[2], proc_counter=lambda d: (0, 't'))[:2])\n")
        try:
            r = _child(code, self.cfg, self.ws)
        except subprocess.TimeoutExpired:
            self.fail("seed_trust 가 FIFO .claude.json 에서 멈췄다")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("RC (1, 'ERROR')", r.stdout, r.stdout)

    def test_r4_toctou_regular_lstat_then_fifo_open_is_refused(self):
        """선검사(lstat)를 정규 파일로 위장해 통과시켜도 open 이 막히지 않고 fstat 이 거부한다(lstat~open TOCTOU)."""
        self._fifo(self.cfgfile)
        reg_file = os.path.join(self.tmp, "regular")
        _write(reg_file, "{}")
        code = ("import os\n"
                "import javis_preflight as pf\n"
                "real = os.lstat(sys.argv[2])\n"
                "pf.os.lstat = lambda p, *a, **k: real          # 선검사만 속인다(실제 대상은 FIFO)\n"
                "try:\n"
                "    pf._read_claude_json_bytes(sys.argv[1]); print('NOGUARD')\n"
                "except ValueError as e:\n"
                "    print('VALUEERROR', '정규 파일이 아니다' in str(e))\n")
        try:
            r = _child(code, self.cfgfile, reg_file)
        except subprocess.TimeoutExpired:
            self.fail("위장된 lstat 뒤 open 이 FIFO 에서 멈췄다(TOCTOU 미차단)")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("VALUEERROR True", r.stdout, r.stdout)

    def test_r4_registry_fifo_is_unreadable_symlink_still_followed(self):
        """레지스트리(depts.json)가 FIFO 여도 판독은 None(멈춤 0) · 심링크 → 정규 JSON 은 종전대로 따라간다(정책 보존)."""
        fifo = os.path.join(self.tmp, "reg-fifo.json")
        self._fifo(fifo)
        real = os.path.join(self.tmp, "reg-real.json")
        _write(real, json.dumps({"depts": {"dept-9": {"account_dir": self.cfg, "cwd": self.ws}}}))
        link = os.path.join(self.tmp, "reg-link.json")
        os.symlink(real, link)
        code = ("import javis_preflight as pf\n"
                "print('FIFO', pf._read_json_tolerant(sys.argv[1]))\n"
                "print('LINK', bool(pf._read_json_tolerant(sys.argv[2])))\n"
                "print('DIR', pf._read_json_tolerant(sys.argv[3]))\n")
        try:
            r = _child(code, fifo, link, self.tmp)
        except subprocess.TimeoutExpired:
            self.fail("_read_json_tolerant 가 FIFO 레지스트리에서 멈췄다")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("FIFO None", r.stdout, r.stdout)
        self.assertIn("LINK True", r.stdout, r.stdout)
        self.assertIn("DIR None", r.stdout, r.stdout)     # 디렉터리도 비정규 = None(예외 0)

    # ── T2 ps 모호 대상(꼬리·머리 공백 · 값 속 ' NAME=' · 줄 나누는 문자)과 찢긴 출력 ──
    def test_r4_ambiguous_target_keeps_positive_and_never_verified_zero(self):
        """모호 대상에서도 **양성은 원문 바이트 대조로 관측**되고(강행이 라이브 좌석을 못 넘는다), **불일치**만 검증된 0
        에서 unresolved 로 접힌다. 보통 대상의 검증된 0 은 불변이어야 한다(사라지면 WP-2 가 inert)."""
        t = "/w/account "                                   # 꼬리 공백 — 분할기가 값의 꼬리 공백을 구분자로 먹는다
        hit = "71 claude CLAUDE_CONFIG_DIR=/w/account  HOME=/x"
        self.assertEqual(pf._count_claude_in_ps_lines([hit], t, argv_lines=["71 claude"]), (1, 1, 0),
                         "꼬리 공백 대상의 실제 일치가 관측되지 않았다(강행이 라이브 claude 를 넘게 된다)")
        self.assertEqual(pf._count_claude_in_ps_lines([hit], t), (1, 1, 0))            # 구분자 없는 모드도 동일
        miss = "71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"
        self.assertEqual(pf._count_claude_in_ps_lines([miss], t, argv_lines=["71 claude"]), (0, 1, 1),
                         "모호 대상의 불일치가 '검증된 0' 이 됐다(라이브 config 에 쓴다)")
        # env 가 노출된 줄의 형상 판정은 비-strict(any-token)다 — `tail -f …/claude` 도 claude 형상이므로 모호 대상에선
        # 함께 접힌다(과포함 = 거부 방향). env 비노출 줄만 strict 로 걸러진다(R2).
        self.assertEqual(pf._count_claude_in_ps_lines(["71 tail -f /x/logs/claude CLAUDE_CONFIG_DIR=/w/o HOME=/x"], t,
                                                      argv_lines=["71 tail -f /x/logs/claude"]), (0, 1, 1))
        self.assertEqual(pf._count_claude_in_ps_lines(["71 python3 x.py CLAUDE_CONFIG_DIR=/w/o HOME=/x"], t,
                                                      argv_lines=["71 python3 x.py"]), (0, 1, 0), "비-claude 형상이 접혔다")
        for normal in ("/w/account", "/w/my account", "/w/a=b", "/w/a\tb"):
            with self.subTest(target=normal):
                self.assertFalse(pf._ps_target_ambiguous(normal))
                self.assertEqual(pf._count_claude_in_ps_lines([miss], normal, argv_lines=["71 claude"]), (0, 1, 0),
                                 "보통 대상의 '검증된 0' 이 사라졌다(WP-2 가 inert 가 된다)")

    def test_r4_leading_space_target_is_ambiguous(self):
        """머리 공백 대상 — 값의 `.strip()` 이 앞을 깎아 일치가 불일치로 보인다(꼬리만 보던 판정의 사각)."""
        t = " /w/account"
        self.assertTrue(pf._ps_target_ambiguous(t))
        self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR= /w/account HOME=/x"], t,
                                                      argv_lines=["71 claude"]), (1, 1, 0), "머리 공백 일치가 안 보인다")
        self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"], t,
                                                      argv_lines=["71 claude"]), (0, 1, 1))

    def test_r4_name_eq_target_is_scanned_not_short_circuited(self):
        """경로 속 ' NAME=' 대상: 종전엔 `claude_procs_for_config` 가 **스캔 前** None 을 돌려줬다 — `--force-unverified`
        는 unverified 를 넘기므로 그 대상에선 라이브 claude 를 관측할 기회 자체가 없어 강행이 살아 있는 좌석을 덮었다.
        이제는 스캔해서 양성 1 을 관측한다(강행 불가) · 불일치는 unresolved → None · claude 가 없으면 정당한 0."""
        t = "/w/a X=y"
        self.assertTrue(pf._ps_target_ambiguous(t))
        self.assertEqual(pf.claude_procs_for_config(
            t, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=/w/a X=y HOME=/x"), os_name="posix", platform="darwin")[0], 1)
        self.assertIsNone(pf.claude_procs_for_config(
            t, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"), os_name="posix", platform="darwin")[0])
        self.assertEqual(pf.claude_procs_for_config(
            t, runner=_ps2("71 python3 x.py HOME=/x"), os_name="posix", platform="darwin")[0], 0)

    def test_r4_torn_ps_record_is_never_a_verified_zero(self):
        """env 값 하나에 줄바꿈이 있으면 `splitlines()` 가 레코드를 쪼개고 뒤 조각(거기에 CLAUDE_CONFIG_DIR 이 있다)은
        pid 가 없어 버려진다 — 앞 조각만 보고 '검증된 0' 이 되던 구멍(codex 실증). **보통 대상**에서도 접혀야 한다."""
        torn = ["71 claude OTHER=x", "CLAUDE_CONFIG_DIR=/w/account  HOME=/x"]
        self.assertTrue(pf._ps_lines_torn(torn))
        # 미해결 2 = 조각 자체(우리 대상을 달았는데 어느 pid 인지 모른다) + 찢김 전역 모호에 걸린 claude 형상 줄
        self.assertEqual(pf._count_claude_in_ps_lines(torn, "/w/account", argv_lines=["71 claude"]), (0, 1, 2))
        self.assertIsNone(pf.claude_procs_for_config(
            "/w/account", runner=_ps2("\n".join(torn)), os_name="posix", platform="darwin")[0])
        self.assertFalse(pf._ps_lines_torn(["71 claude", "", "  ", "72 node x.js"]))    # 정상 출력·빈 줄은 조각이 아니다
        self.assertFalse(pf._ps_lines_torn(["71 claude HOME=/x", "junk", "72 python"]))  # 형상 litter 는 귀속을 감추지 않는다

    # ── T3 codex 위임 반례(워커가 전 행 검토 후 채택 · `codex/P1-WP2-trust-R1fix-tests.md`) ──
    def test_r4_last_env_value_trailing_bytes_survive(self):
        """줄 끝 공백은 **마지막 env 값의 바이트**다 — 판독이 rstrip 하면 실제 일치가 사라져 강행이 넘어간다."""
        for suffix in (" ", "\t", "\u00a0"):
            t = "/w/account" + suffix
            with self.subTest(suffix=repr(suffix)):
                self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=" + t], t,
                                                              argv_lines=["71 claude"]), (1, 1, 0))
                self.assertEqual(pf.claude_procs_for_config(
                    t, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=" + t), os_name="posix", platform="darwin")[0], 1)

    def test_r4_torn_argv_fragment_carrying_target_is_unresolved(self):
        """claude **설치 경로** 속 줄바꿈은 argv 자체를 쪼갠다 — 남은 레코드가 claude 형상이 아니어도 우리 대상을 단
        조각이 있으면 검증된 0 이 아니다(찢김 전역 모호는 claude 형상 줄에만 걸리므로 조각 규칙이 따로 필요하다)."""
        torn = ["71 /w/inst", "part/claude CLAUDE_CONFIG_DIR=/w/account HOME=/x"]
        self.assertEqual(pf._count_claude_in_ps_lines(torn, "/w/account", argv_lines=["71 /w/inst"]), (0, 1, 1))
        self.assertNotEqual(pf.claude_procs_for_config(
            "/w/account", runner=_ps2("\n".join(torn)), os_name="posix", platform="darwin")[0], 0)

    def test_r4_unknown_argv_boundary_keeps_visible_match_positive(self):
        """argv 경계를 모르는 줄(두 ps 사이 신생 · pid 중복)에 **우리 대상 바이트**가 보이면 양성이다 — 종전 unresolved 는
        `--force-unverified` 가 넘을 수 있었다. 경계를 모르니 argv 속 값도 env 로 본다(거부 방향의 과포함)."""
        line = "71 claude -p CLAUDE_CONFIG_DIR=/w/account HOME=/x"
        for argv_lines in ([], ["71 claude", "71 claude"], ["99 other"]):
            with self.subTest(argv_lines=argv_lines):
                self.assertEqual(pf._count_claude_in_ps_lines([line], "/w/account", argv_lines=argv_lines), (1, 1, 0))
        # 경계를 **아는** 줄에서는 R3 계약 그대로 — 인자 속 값은 env 가 아니라 검증된 0 이다(그 0 이 WP-2 의 주경로다).
        #   (알려진 한계: pid 재사용으로 이 argv 가 죽은 프로세스의 것이면 새 프로세스의 env 를 argv 로 먹는다 —
        #    닫으려면 인자 속 값을 env 로 세야 하는데 그것이 바로 R3 이 닫은 결함이라 여기서는 열어 두고 고지한다.)
        self.assertEqual(pf._count_claude_in_ps_lines([line], "/w/account", argv_lines=["71 claude -p CLAUDE_CONFIG_DIR=/w/account"]),
                         (0, 1, 0))
        # 형상이 아닌 줄이 대상을 달고 경계도 모르면 미해결(양성 아님 · 검증된 0 도 아님)
        self.assertEqual(pf._count_claude_in_ps_lines(["71 noise CLAUDE_CONFIG_DIR=/w/account"], "/w/account",
                                                      argv_lines=["99 other"]), (0, 1, 1))

    def test_r4_non_string_or_empty_target_never_verified_zero(self):
        """None/bytes/빈 문자열 대상은 귀속 자체가 불가하다 — 조용한 '검증된 0'(= 라이브 config 에 쓴다)이 되면 안 된다."""
        for t in (None, b"/w/account", ""):
            with self.subTest(target=t):
                self.assertTrue(pf._ps_target_ambiguous(t))
                n, parsed, unresolved = pf._count_claude_in_ps_lines(
                    ["71 claude CLAUDE_CONFIG_DIR=/w/account HOME=/x"], t, argv_lines=["71 claude"])
                self.assertEqual((n, parsed), (0, 1))
                self.assertGreater(unresolved, 0)

    def test_r4_exact_helper_boundaries(self):
        """원문 대조는 **경계**에서만 참이다 — 변수명 접미(XCLAUDE_/MY_)·경로 접두(자식 경로)·정규화는 일치가 아니다."""
        t = "/w/account"
        self.assertTrue(pf._ps_env_value_present("CLAUDE_CONFIG_DIR=" + t, "CLAUDE_CONFIG_DIR", t))
        self.assertTrue(pf._ps_env_value_present("A=1 CLAUDE_CONFIG_DIR=" + t + " HOME=/x", "CLAUDE_CONFIG_DIR", t))
        self.assertTrue(pf._ps_env_value_present("XCLAUDE_CONFIG_DIR=" + t + " CLAUDE_CONFIG_DIR=" + t,
                                                 "CLAUDE_CONFIG_DIR", t), "앞선 비-경계 출현이 뒤의 진짜 일치를 가렸다")
        for bad in ("XCLAUDE_CONFIG_DIR=" + t, "MY_CLAUDE_CONFIG_DIR=" + t, "CLAUDE_CONFIG_DIR=" + t + "/child",
                    "CLAUDE_CONFIG_DIR=" + t + "x", "ARG=p" + "CLAUDE_CONFIG_DIR=" + t):
            with self.subTest(text=bad):
                self.assertFalse(pf._ps_env_value_present(bad, "CLAUDE_CONFIG_DIR", t))
        for norm in (t + "/", "/w/./account"):        # 정규화는 동일성 비교의 몫이지 바이트 대조의 몫이 아니다
            self.assertFalse(pf._ps_env_value_present("CLAUDE_CONFIG_DIR=" + norm, "CLAUDE_CONFIG_DIR", t))

    def test_r4_surrogate_and_unicode_spellings_are_byte_exact(self):
        """os.fsdecode 의 대리쌍(디코딩 불가 바이트)과 NFC/NFD 철자는 **있는 그대로** 대조된다 — 정규화를 발명하지 않는다."""
        t = "/w/account" + os.fsdecode(b"\xff\xfe")
        self.assertEqual(os.fsencode(t)[-2:], b"\xff\xfe")
        self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=" + t + " HOME=/x"], t,
                                                      argv_lines=["71 claude"]), (1, 1, 0))
        nfc, nfd = "/w/caf\u00e9", "/w/cafe\u0301"
        for t2 in (nfc, nfd):
            with self.subTest(spelling=t2):
                self.assertFalse(pf._ps_target_ambiguous(t2))
                self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=" + t2 + " HOME=/x"], t2,
                                                              argv_lines=["71 claude"]), (1, 1, 0))
        self.assertFalse(pf._ps_env_value_present("CLAUDE_CONFIG_DIR=" + nfd, "CLAUDE_CONFIG_DIR", nfc))

    def test_r4_ordinary_verified_zero_matrix_survives(self):
        """I3 — 보통 대상 + 정상 출력의 '검증된 0' 이 사라지면 WP-2 는 통째로 inert 다. 이 표가 그 방벽이다."""
        t = "/w/account"
        for argv, env in (("claude", "CLAUDE_CONFIG_DIR=/w/other HOME=/x"),
                          ("claude", "HOME=/x PATH=/bin"),
                          ("claude", "XCLAUDE_CONFIG_DIR=" + t + " HOME=/x"),
                          ("claude", "MY_CLAUDE_CONFIG_DIR=" + t + " HOME=/x"),
                          ("claude -p CLAUDE_CONFIG_DIR=" + t, "CLAUDE_CONFIG_DIR=/w/other"),
                          ("claude -p CLAUDE_CONFIG_DIR=" + t, "HOME=/x"),
                          ("python3 worker.py", "CLAUDE_CONFIG_DIR=" + t)):
            with self.subTest(argv=argv, env=env):
                self.assertEqual(pf._count_claude_in_ps_lines(["71 " + argv + " " + env], t,
                                                              argv_lines=["71 " + argv]), (0, 1, 0))
        self.assertEqual(pf.claude_procs_for_config(
            t, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"), os_name="posix", platform="darwin")[0], 0)

    def test_r4_path_identity_aliases_still_positive(self):
        """동일성 비교는 남아 있다 — 심링크·꼬리 슬래시·'/./' 별칭은 바이트가 달라도 양성(원문 대조가 그것을 대체하지 않는다)."""
        seat = os.path.join(self.tmp, "seat")
        os.makedirs(seat, exist_ok=True)
        alias = os.path.join(self.tmp, "alias")
        os.symlink(seat, alias)
        for v in (alias, seat + "/", os.path.join(self.tmp, ".", "seat")):
            with self.subTest(value=v):
                self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=" + v + " HOME=/x"], seat,
                                                              argv_lines=["71 claude"]), (1, 1, 0))

    def test_r4_line_breaking_targets_are_ambiguous(self):
        """`splitlines()` 가 나누는 문자를 담은 대상은 줄 자체가 끊겨 귀속이 불가능하다(codex R4 반례: 개행)."""
        for ch in ("\n", "\r", "\x0b", "\x0c", "\x1c", "\x85", "\u2028"):
            with self.subTest(ch=repr(ch)):
                self.assertTrue(pf._ps_target_ambiguous("/w/acc" + ch + "ount"))
        self.assertEqual(pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/acc"], "/w/acc\nount",
                                                      argv_lines=["71 claude"]), (0, 1, 1))

    def test_r4_probe_refuses_ambiguous_target_but_keeps_positive(self):
        """프로브 전체 경로: 모호 대상 + 불일치 claude → None(거부 방향) · 양성이 있으면 n(강행이 못 넘는다)."""
        amb = "/w/account "
        self.assertIsNone(pf.claude_procs_for_config(
            amb, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"), os_name="posix", platform="darwin")[0])
        self.assertEqual(pf.claude_procs_for_config(
            amb, runner=_ps2("71 claude CLAUDE_CONFIG_DIR=/w/account  HOME=/x"), os_name="posix", platform="darwin")[0], 1)
        self.assertEqual(pf.claude_procs_for_config(
            amb, runner=_ps2("71 python3 x.py HOME=/x"), os_name="posix", platform="darwin")[0], 0)

    def test_r4_ambiguous_target_seed_refuses_and_force_cannot_pass_positive(self):
        """모호 대상의 시드는 REFUSE unverified(강행만 프로브를 넘는다) · 양성 관측은 강행으로도 못 넘는다(계약 불변)."""
        self.untrusted_file('{"projects": {}}')
        raw = _read_bytes(self.cfgfile)
        amb_probe = lambda d: (None, "darwin: 꼬리 공백 대상 — 귀속 불가")
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=amb_probe)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("unverified", reason)
        self.assertEqual(_read_bytes(self.cfgfile), raw)
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=lambda d: (2, "라이브 2건"),
                                            force_unverified=True)
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("live-claude", reason)
        self.assertEqual(_read_bytes(self.cfgfile), raw)


class R4CommitCleanupAndLock(Base):
    """★R4 리뷰(minor 2건): 커밋 뒤 청소 실패의 오보고 · C43 무잠금 lost update."""

    def setUp(self):
        super().setUp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        os.makedirs(self.cfg)

    def test_r4_exchange_commit_survives_displaced_unlink_failure(self):
        """교환이 성립한 뒤의 `os.unlink(displaced)` 실패는 성공을 ERROR 로 뒤집지 않는다 — 잔재는 고지(자동 회수 대상 아님)."""
        _require_exchange(self)
        self.untrusted_file('{"projects": {}}')
        real_unlink = os.unlink
        def flaky(path, *a, **kw):
            if pf.SEED_TRUST_DISPLACED_PREFIX in os.path.basename(os.fspath(path)):
                raise OSError(errno.EIO, "injected displaced unlink failure")
            return real_unlink(path, *a, **kw)
        with patch.object(pf.os, "unlink", side_effect=flaky):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("commit=exchange", reason)
        self.assertIn("displaced-left(EIO", reason)
        self.assertIs(_read_json(self.cfgfile)["projects"][self.key]["hasTrustDialogAccepted"], True)
        left = [n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX)]
        self.assertEqual(len(left), 1, os.listdir(self.cfg))
        self.assertEqual(_read_text(os.path.join(self.cfg, left[0])), '{"projects": {}}', "잔재가 상대 원본이 아니다")

    def test_r4_link_commit_survives_tmp_unlink_failure(self):
        """부재 파일 경로(link 공개)도 같다 — 공개 뒤 tmp 청소 실패는 사유 꼬리(tmp-left)."""
        real_unlink = os.unlink
        def flaky(path, *a, **kw):
            if pf._SEED_TMP_LITTER_RE.match(os.path.basename(os.fspath(path))):
                raise OSError(errno.EIO, "injected tmp unlink failure")
            return real_unlink(path, *a, **kw)
        with patch.object(pf.os, "unlink", side_effect=flaky):
            rc, verdict, reason = self.seed(proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertIn("commit=link", reason)
        self.assertIn("tmp-left(EIO", reason)
        self.assertIs(_read_json(self.cfgfile)["projects"][self.key]["hasTrustDialogAccepted"], True)

    def _c43(self, fix=True):
        return pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report", allow_irreversible=False)

    def test_r4_c43_holds_seed_lock_and_preserves_trust_flag(self):
        """C43 `_enable_mcp_server` 는 시더와 같은 잠금 아래에서만 쓴다 — 잠금 경합이면 무쓰기 + 사유 · 잠금 뒤엔
        C58 이 방금 커밋한 hasTrustDialogAccepted 를 보존한 채 활성화한다(lost update 부류 봉인)."""
        proj = pf.SERENA_PROJECT
        _write(self.cfgfile, json.dumps({"projects": {proj: {"hasTrustDialogAccepted": True}}}, indent=2))
        before = _read_bytes(self.cfgfile)
        holder = open(os.path.join(self.cfg, pf.SEED_TRUST_LOCK_NAME), "a+")
        self.addCleanup(holder.close)
        self.assertIs(pf._try_lock_nb(holder), True)
        r = self._c43()._enable_mcp_server(self.cfgfile, proj, "serena", set_trust=True)
        self.assertIsInstance(r, str)
        self.assertIn("쓰기 보류", r)
        self.assertEqual(_read_bytes(self.cfgfile), before, "잠금 경합인데 썼다")
        self.assertFalse(os.path.exists(self.cfgfile + ".tmp"))
        holder.close()
        self.assertIsNone(self._c43()._enable_mcp_server(self.cfgfile, proj, "serena", set_trust=True))
        got = _read_json(self.cfgfile)["projects"][proj]
        self.assertIn("serena", got["enabledMcpjsonServers"])
        self.assertIs(got["hasTrustDialogAccepted"], True, "C43 이 신뢰 플래그를 잃었다")

    def test_r4_c43_symlink_still_refused_before_lock(self):
        """심링크 거부는 잠금보다 앞(잠금 파일을 낯선 dir 에 만들지 않는다)."""
        target = os.path.join(self.tmp, "elsewhere.json")
        _write(target, "{}")
        link = os.path.join(self.cfg, "linked.json")
        os.symlink(target, link)
        r = self._c43()._enable_mcp_server(link, pf.SERENA_PROJECT, "serena", set_trust=True)
        self.assertIn("symlink 거부", r)
        self.assertEqual(_read_text(target), "{}")


class R4SeedDeadline(Base):
    """★R4(codex 잔여): 부트 호출자가 시드 I/O 에 무한정 잡히지 않는다 — CLI 마감 감시."""

    def test_r4_timeout_secs_table(self):
        self.assertEqual(pf._seed_trust_timeout_secs({}), 20.0)
        self.assertEqual(pf._seed_trust_timeout_secs({pf._SEED_TRUST_TIMEOUT_ENV: "0.5"}), 0.5)
        self.assertIsNone(pf._seed_trust_timeout_secs({pf._SEED_TRUST_TIMEOUT_ENV: "0"}))
        self.assertIsNone(pf._seed_trust_timeout_secs({pf._SEED_TRUST_TIMEOUT_ENV: "-1"}))
        self.assertEqual(pf._seed_trust_timeout_secs({pf._SEED_TRUST_TIMEOUT_ENV: "abc"}), 20.0)

    def test_r4_deadline_emits_refuse_line_and_exits_two(self):
        """무한 대기 자식: 마감 감시가 REFUSE 1줄을 **버퍼 유실 없이** 찍고 rc 2 로 끝난다(사람/JSON 형식 모두)."""
        for json_mode in (False, True):
            with self.subTest(json_mode=json_mode):
                code = ("import time\n"
                        "import javis_preflight as pf\n"
                        "pf._start_seed_deadline(0.3, lambda: pf._seed_trust_emit(\n"
                        "    sys.argv[1] == '1', pf.SEED_TRUST_REFUSE, 'REFUSE', 'timeout(주입)', '/c', '/w'))\n"
                        "time.sleep(60)\n")
                try:
                    r = _child(code, "1" if json_mode else "0", timeout=15)
                except subprocess.TimeoutExpired:
                    self.fail("마감 감시가 발화하지 않았다(자식이 살아 있다)")
                self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
                if json_mode:
                    j = json.loads(r.stdout.strip())
                    self.assertEqual((j["verdict"], j["rc"], j["config"]), ("REFUSE", 2, "/c"))
                    self.assertIn("timeout(", j["reason"])
                else:
                    self.assertTrue(r.stdout.startswith("seed-trust: REFUSE timeout("), r.stdout)

    def test_r4_deadline_does_not_kill_normal_path(self):
        """정상 경로는 취소된다 — 짧은 상한에서도 성공 시드가 죽지 않는다(감시가 게이트가 아니다)."""
        rc, out, err = seed_cli(self.cfg, self.ws, env={pf._SEED_TRUST_TIMEOUT_ENV: "10"})
        self.assertEqual(rc, 0, out + err)
        self.assertIn("OK seeded(", out)
        rc, out, err = seed_cli(self.cfg, self.ws, env={pf._SEED_TRUST_TIMEOUT_ENV: "0"})   # 감시 끔(롤백 노브)
        self.assertEqual(rc, 0, out + err)
        self.assertIn("already-trusted(", out)


# ══════════════════════════════════════════════════════════════════════════════════════════════════════
# R5 리뷰(Claude 적대 + codex gpt-6-astra 감사) 반례 — 표기 오라클 · 값 후보 동일성 · 의도 저널 · 마감 감시
# ══════════════════════════════════════════════════════════════════════════════════════════════════════
class R5Identity(Base):
    """D1: 표기(spelling)와 동일성(identity)과 '판정 불가' 를 가른다."""

    def _case_insensitive(self, d):
        """이 FS 가 대소문자를 접는가 — 접지 않으면 별칭 검체는 성립하지 않는다(정직 skip)."""
        probe = os.path.join(d, "CaseProbe")
        os.makedirs(probe, exist_ok=True)
        return os.path.isdir(os.path.join(d, "caseprobe"))

    def test_r5_case_alias_is_same_directory_not_a_mismatch(self):
        """리뷰 codex major: `os.path.samefile()==True` 인 별칭이 `_path_identity` 로는 다른 값이었다 → 3값 동일성."""
        real = os.path.join(self.tmp, "CYSjavis-ws")
        os.makedirs(real)
        alias = os.path.join(self.tmp, "cysjavis-ws")
        if not self._case_insensitive(self.tmp):
            self.skipTest("대소문자 구분 FS — 별칭 표기 검체 불가")
        self.assertTrue(os.path.samefile(real, alias))
        self.assertIs(pf._same_dir(alias, real), True, "같은 디렉터리를 다르다고 봤다")
        line = "71 claude CLAUDE_CONFIG_DIR=%s HOME=/x" % alias
        self.assertEqual(pf._count_claude_in_ps_lines([line], real, argv_lines=["71 claude"]), (1, 1, 0),
                         "별칭 표기로 도는 라이브 claude 가 '검증된 0' 이 됐다(라이브 config 에 쓴다)")

    def test_r5_stat_failure_is_unresolved_never_verified_zero(self):
        """리뷰 codex: 조회 실패(EACCES/ESTALE)를 '다름' 으로 접으면 안 된다 — 부재(ENOENT)만 증명된 다름이다."""
        other = os.path.join(self.tmp, "other-cfg")
        os.makedirs(other)
        line = "71 claude CLAUDE_CONFIG_DIR=%s HOME=/x" % other
        self.assertEqual(pf._count_claude_in_ps_lines([line], self.cfg, argv_lines=["71 claude"]), (0, 1, 0),
                         "정상 불일치는 검증된 0 이어야 한다(사라지면 WP-2 가 inert)")
        real_stat = os.stat

        def flaky(path, **k):
            if isinstance(path, str) and path == other:
                raise PermissionError(errno.EACCES, "denied")
            return real_stat(path, **k)

        with patch.object(pf.os, "stat", flaky):
            self.assertIsNone(pf._same_dir(other, self.cfg))
            self.assertEqual(pf._count_claude_in_ps_lines([line], self.cfg, argv_lines=["71 claude"]), (0, 1, 1))
        for e, expected in ((errno.ENOENT, False), (errno.ENOTDIR, False), (errno.ENAMETOOLONG, False),
                            (errno.ELOOP, False), (errno.ESTALE, None), (errno.EIO, None)):
            with self.subTest(errno=e):
                def raiser(path, _e=e, **k):
                    if isinstance(path, str) and path == other:
                        raise OSError(_e, "injected")
                    return real_stat(path, **k)
                with patch.object(pf.os, "stat", raiser):
                    self.assertIs(pf._same_dir(other, self.cfg), expected)

    def test_r5_identity_lookups_go_through_the_injection_point(self):
        """판정부는 파일시스템 동일성 조회를 **주입점 하나**로만 한다 — os.stat 직접 호출 0(codex D1 순수성 지적)."""
        seen = []

        def pure(a, b):
            seen.append((a, b))
            return a == b

        with patch.object(pf.os, "stat", side_effect=AssertionError("판정부가 os.stat 을 직접 불렀다")):
            got = pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/other HOME=/x"], "/w/cfg",
                                               argv_lines=["71 claude"], same_dir=pure)
        self.assertEqual(got, (0, 1, 0))
        self.assertTrue(seen, "주입점이 호출되지 않았다")

    def test_r5_pristine_guard_survives_identity_change(self):
        """codex D1: 동일성 표현을 바꾸면서 `.pristine` 경로 가드가 무력화되면 안 된다(경로 성분 검사는 별도)."""
        pristine = os.path.join(self.tmp, "pack", ".pristine", "acct")
        os.makedirs(pristine)
        rc, verdict, reason = pf.seed_trust(pristine, self.ws, proc_counter=_no_probe)
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn(".pristine", reason)
        self.assertFalse(os.path.exists(os.path.join(pristine, ".claude.json")))


class R5ProjectKey(Base):
    """D1 후반: 우리가 박는 키 == 자식의 getcwd()(claude 가 읽는 키)."""

    def _oracle(self, cwd):
        r = subprocess.run([PY, "-B", "-c", "import os;print(os.getcwd())"], cwd=cwd,
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout.rstrip("\r\n")

    def test_r5_case_alias_key_equals_child_getcwd(self):
        """리뷰 codex major: 별칭 표기의 cwd 에서 realpath 키는 자식 getcwd 와 갈렸다(시드가 조용히 무효)."""
        real = os.path.join(self.tmp, "WorkSpace")
        os.makedirs(real)
        alias = os.path.join(self.tmp, "workspace")
        if not os.path.isdir(alias):
            self.skipTest("대소문자 구분 FS — 별칭 표기 검체 불가")
        oracle = self._oracle(alias)
        self.assertEqual(pf.claude_project_key(alias), oracle,
                         "claude 가 읽지 않는 키를 시드한다(관문이 다시 뜬다)")
        rc, verdict, reason = pf.seed_trust(self.cfg, alias, proc_counter=_no_probe)
        self.assertEqual(rc, 0, reason)
        self.assertIn(oracle, _read_json(self.cfgfile)["projects"])

    def test_r5_unicode_normalization_key_equals_child_getcwd(self):
        """정규화는 우리가 하지 않는다 — 저장된 형태를 그대로 돌려받아야 한다(NFC/NFD 양방향)."""
        import unicodedata
        for created, requested in ((unicodedata.normalize("NFC", "café-a"), unicodedata.normalize("NFD", "café-a")),
                                   (unicodedata.normalize("NFD", "café-b"), unicodedata.normalize("NFC", "café-b"))):
            with self.subTest(created=created):
                d = os.path.join(self.tmp, created)
                try:
                    os.makedirs(d)
                except (OSError, UnicodeError):
                    self.skipTest("이 FS 에 유니코드 디렉터리를 만들 수 없다")
                req = os.path.join(self.tmp, requested)
                if not os.path.isdir(req):
                    continue          # 정규화를 접지 않는 FS — 두 이름은 서로 다른 디렉터리다(검체 대상 아님)
                self.assertEqual(pf.claude_project_key(req), self._oracle(req))

    def test_r5_key_shape_is_never_restructured(self):
        """오라클이 **구조가 다른** 경로를 주면 채택하지 않는다(펌링크·마운트 별칭 방벽 · codex D1)."""
        with patch.object(pf, "_fgetpath", lambda p: "/System/Volumes/Data" + p):
            self.assertEqual(pf.claude_project_key(self.ws), os.path.realpath(self.ws))
        with patch.object(pf, "_fgetpath", lambda p: "relative/not/absolute"):
            self.assertEqual(pf.claude_project_key(self.ws), os.path.realpath(self.ws))
        with patch.object(pf, "_fgetpath", lambda p: None):
            self.assertEqual(pf.claude_project_key(self.ws), os.path.realpath(self.ws))
        # 구조가 같아도 **다른 디렉터리**면(samestat 불일치) 채택하지 않는다
        other = os.path.join(self.tmp, "OtherWs")
        os.makedirs(other)
        with patch.object(pf, "_fgetpath", lambda p: other):
            self.assertEqual(pf.claude_project_key(self.ws), os.path.realpath(self.ws))


class R5EnvBoundary(Base):
    """D2: env 값의 끝은 확정 불가 — 후보 전부 × 동일성으로 닫는다."""

    def setUp(self):
        super().setUp()
        self.target = os.path.join(self.tmp, "target-cfg")
        os.makedirs(self.target)

    def line(self, value_tail):
        return "71 claude CLAUDE_CONFIG_DIR=%s" % value_tail

    def test_r5_non_shell_variable_names_do_not_defeat_detection(self):
        """리뷰 codex major: `BAD-NAME=`·`BASH_FUNC_f%%=` 는 분할기가 경계로 보지 못해 양성·모호 판정이 함께 무너졌다."""
        for tail in ("%s BAD-NAME=x HOME=/h" % self.target,
                     "%s BASH_FUNC_f%%%%=() { :; } HOME=/h" % self.target,
                     "%s 987654=x HOME=/h" % self.target):
            with self.subTest(tail=tail):
                self.assertEqual(pf._count_claude_in_ps_lines([self.line(tail)], self.target,
                                                              argv_lines=["71 claude"]), (1, 1, 0),
                                 "비-식별자 변수명이 라이브 claude 관측을 지웠다")

    def test_r5_alias_with_inner_assignment_is_never_verified_zero(self):
        """codex 반례: `/alias X=y` 가 대상의 심링크면 판독기는 `/alias` 로 잘라 '불일치' 를 만든다."""
        alias = os.path.join(self.tmp, "alias X=y")
        os.symlink(self.target, alias)
        got = pf._count_claude_in_ps_lines([self.line("%s HOME=/h" % alias)], self.target, argv_lines=["71 claude"])
        self.assertEqual(got, (1, 1, 0), "별칭+내부 대입 형상이 '검증된 0' 이 됐다")

    def test_r5_conservative_positive_is_documented(self):
        """완화한 경계는 **보수적 양성**을 만든다 — 정직하게 핀으로 남긴다(안전 방향 · 가용성 손해)."""
        got = pf._count_claude_in_ps_lines([self.line("%s archive HOME=/h" % self.target)], self.target,
                                           argv_lines=["71 claude"])
        self.assertEqual(got, (1, 1, 0))

    def test_r5_long_alias_beyond_the_candidate_window_is_not_verified_zero(self):
        """★codex 위임 반례(FAILS-NOW 였다): 공백 30개짜리 심링크 별칭은 참값이 후보 창 밖으로 밀려 '검증된 0' 이 됐다.
        열거는 **경로 길이 상한까지 소진**해야 하고, 개수 상한에서 잘렸다면 그 불일치는 증명이 아니다(미해결)."""
        alias = os.path.join(self.tmp, "alias " * 30 + "end")
        os.symlink(self.target, alias)
        self.assertTrue(os.path.samefile(alias, self.target))
        text = "CLAUDE_CONFIG_DIR=%s HOME=/isolated" % alias
        self.assertIn(alias, pf._ps_env_value_candidates(text), "길이 상한 안인데 후보에서 빠졌다")
        self.assertFalse(pf._ps_env_candidates_truncated(text))
        got = pf._count_claude_in_ps_lines([self.line("%s HOME=/isolated" % alias)], self.target,
                                           argv_lines=["71 claude"])
        self.assertEqual(got, (1, 1, 0), "긴 별칭이 '검증된 0' 이 됐다(라이브 config 에 쓴다)")
        # 개수 상한에 실제로 걸리면(비정상적으로 긴 env) 그 줄은 미해결이어야 한다
        many = " ".join("t%d=x" % i for i in range(pf._PS_VALUE_CANDIDATE_MAX + 20))
        self.assertTrue(pf._ps_env_candidates_truncated("CLAUDE_CONFIG_DIR=/w/other " + many))
        got = pf._count_claude_in_ps_lines(["71 claude CLAUDE_CONFIG_DIR=/w/other " + many], self.target,
                                           argv_lines=["71 claude"])
        self.assertEqual(got, (0, 1, 1))

    def test_r5_ordinary_mismatch_still_verified_zero(self):
        """주경로 보존: 보통 대상 + 다른 config = 검증된 0(사라지면 부서 부트마다 REFUSE)."""
        other = os.path.join(self.tmp, "other-cfg")
        os.makedirs(other)
        lines = [self.line("%s HOME=/h" % other), "72 python3 x.py", "73 /bin/sh -c true"]
        self.assertEqual(pf._count_claude_in_ps_lines(lines, self.target,
                                                      argv_lines=["71 claude", "72 python3 x.py", "73 /bin/sh -c true"]),
                         (0, 3, 0))

    def test_r5_default_config_target_apparent_assignment_is_unresolved(self):
        """codex D2(정보 이론): `OTHER="x CLAUDE_CONFIG_DIR=/other"` 와 실제 지정은 직렬화가 같다 — 기본 config 대상에서
        겉보기 지정을 '그 프로세스는 기본이 아니다' 의 증거로 쓰지 않는다."""
        default = pf._default_claude_config_dir()
        got = pf._count_claude_in_ps_lines([self.line("/w/other HOME=/h")], default, argv_lines=["71 claude"])
        self.assertEqual(got, (0, 1, 1))
        self.assertEqual(pf._count_claude_in_ps_lines(["71 claude HOME=/h"], default, argv_lines=["71 claude"]),
                         (1, 1, 0), "지정이 없는 claude 는 기본 config 사용 = 양성")

    def test_r5_value_candidates_are_pure_and_bounded(self):
        c = pf._ps_env_value_candidates("CLAUDE_CONFIG_DIR=/a b c HOME=/h")
        self.assertEqual(c[:3], ["/a", "/a b", "/a b c"])
        self.assertLessEqual(len(pf._ps_env_value_candidates("CLAUDE_CONFIG_DIR=" + " ".join("x" * 3 for _ in range(200)))),
                             pf._PS_VALUE_CANDIDATE_MAX)
        self.assertEqual(pf._ps_env_value_candidates("XCLAUDE_CONFIG_DIR=/a"), [])
        self.assertEqual(pf._ps_env_value_candidates(None), [])
        # 상한 초과 길이는 syscall 없이 '부재'(값 후보 열거가 만드는 긴 문자열 · 실측 ENAMETOOLONG)
        self.assertEqual(pf._stat_ident("/" + "a" * (pf._MAX_PATH_PROBE + 10)), (None, "absent"))


class R6EnvValueBoundary(Base):
    """★R6(리뷰 minor#5): `_ps_env_value_present` 끝 경계 완화가 '후보 열거에 가려져 무검증' 이라는 지적에 대한
    반례. 후보 열거(`_ps_env_value_candidates`)가 **없는** 분기가 둘 있다 — ①argv 경계 미확정(두 ps 호출 사이에
    생긴 pid · pid 중복) ②레코드가 아닌 조각(찢긴 출력). 그 둘은 원문 대조 하나로 양성/미해결이 갈린다."""

    def test_r6_non_identifier_next_variable_keeps_the_positive_observation(self):
        """뒤따르는 변수 이름이 셸 식별자가 아니면(`BAD-NAME=` · `BASH_FUNC_f%%=` · `1BAD=`) 종전 형태는 경계를
        못 봐 **양성 관측이 사라졌다** → 미해결. 미해결은 `--force-unverified` 가 넘고 양성은 못 넘는다 —
        그것이 이 완화의 관측 가능한 차이다(실측: 완화 (1,1,0) vs 종전 (0,1,1))."""
        for tail in ("BAD-NAME=x", "BASH_FUNC_f%%=() {", "1BAD=x", "HOME=/h"):
            with self.subTest(tail=tail):
                line = "424242 claude --print CLAUDE_CONFIG_DIR=%s %s" % (self.cfg, tail)
                self.assertEqual(pf._count_claude_in_ps_lines([line], self.cfg, argv_lines=["999999 other"]),
                                 (1, 1, 0), "비식별자 변수명 뒤에서 양성 관측이 사라졌다(강행이 넘는다)")
        # 찢긴 출력의 뒷조각도 같은 대조로 잡는다(후보 열거 없음 · '검증된 0' 금지)
        torn = ["424242 claude --print", "CLAUDE_CONFIG_DIR=%s BAD-NAME=x" % self.cfg]
        count, parsed, unresolved = pf._count_claude_in_ps_lines(torn, self.cfg, argv_lines=["424242 claude --print"])
        self.assertEqual((count, parsed), (0, 1))
        self.assertGreaterEqual(unresolved, 1, "찢긴 조각이 우리 대상을 달고 있는데 미해결로 세지 않았다")

    def test_r6_relaxed_boundary_does_not_break_ordinary_verified_zero(self):
        """가용성 대조: 보통 대상 + 다른 config 는 여전히 **검증된 0** 이다(사라지면 WP-2 가 inert)."""
        other = os.path.join(self.tmp, "other-cfg")
        os.makedirs(other)
        os.makedirs(self.cfg, exist_ok=True)
        line = "424242 claude --print CLAUDE_CONFIG_DIR=%s BAD-NAME=x" % other
        self.assertEqual(pf._count_claude_in_ps_lines([line], self.cfg, argv_lines=["424242 claude --print"]),
                         (0, 1, 0), "정상 불일치가 검증된 0 이 아니게 됐다")


class R5Procfs(Base):
    """D3: /proc 은 파일시스템 바이트를 보존해 대조한다(replace 디코드 금지)."""

    def _proc(self, entries):
        root = os.path.join(self.tmp, "proc")
        for pid, env_b, cmd_b in entries:
            d = os.path.join(root, pid)
            os.makedirs(d)
            with open(os.path.join(d, "environ"), "wb") as f:
                f.write(env_b)
            with open(os.path.join(d, "cmdline"), "wb") as f:
                f.write(cmd_b)
        return root

    # 주의: APFS 는 비-UTF8 파일명을 거부한다(EILSEQ) — 디렉터리를 만들지 않고 **디코드 계약**만 본다. `_same_dir` 는
    #   문자열이 정확히 같으면 stat 없이 True 이고, 다르면 부재/부재 → False 다. 그래서 두 검체는 플랫폼과 무관하게
    #   '바이트가 보존되는가' 하나만 묻는다(리눅스 실기에서도 같은 결론 · /proc 픽스처는 주입이다).
    def test_r5_surrogate_bytes_are_not_replaced(self):
        """리뷰 codex major: `\\xff` 가 U+FFFD 로 바뀌어 살아 있는 config 가 '불일치' 가 됐다."""
        raw = os.path.join(self.tmp, "cfg-").encode() + b"\xff"
        cfg = os.fsdecode(raw)
        root = self._proc([("100", b"CLAUDE_CONFIG_DIR=" + raw + b"\0HOME=/x\0", b"/usr/bin/claude\0")])
        cnt, detail = pf._count_claude_procfs(cfg, root)
        self.assertEqual(cnt, 1, detail)

    def test_r5_replacement_char_directory_is_not_conflated(self):
        """`\\xff` 디렉터리와 U+FFFD 이름 디렉터리는 **다른** 디렉터리다(디코드가 둘을 섞으면 안 된다)."""
        raw = os.path.join(self.tmp, "conf-").encode() + b"\xff"
        a = os.fsdecode(raw)
        b = os.path.join(self.tmp, "conf-�")
        self.assertNotEqual(a, b)
        root = self._proc([("100", b"CLAUDE_CONFIG_DIR=" + raw + b"\0", b"/usr/bin/claude\0")])
        self.assertEqual(pf._count_claude_procfs(b, root)[0], 0)
        self.assertEqual(pf._count_claude_procfs(a, root)[0], 1)

    def test_r5_argv_bytes_are_filesystem_decoded(self):
        raw_cmd = (os.path.join(self.tmp, "install-").encode() + b"\xff" + b"/claude")
        cfg = os.path.join(self.tmp, "cfg-argv")
        os.makedirs(cfg)
        root = self._proc([("100", b"CLAUDE_CONFIG_DIR=" + cfg.encode() + b"\0", raw_cmd + b"\0")])
        self.assertEqual(pf._count_claude_procfs(cfg, root)[0], 1)

    def test_r5_identity_lookup_failure_is_unresolved(self):
        other = os.path.join(self.tmp, "other-cfg")
        os.makedirs(other)
        cfg = os.path.join(self.tmp, "mine-cfg")
        os.makedirs(cfg)
        root = self._proc([("100", b"CLAUDE_CONFIG_DIR=" + other.encode() + b"\0", b"/usr/bin/claude\0")])
        self.assertEqual(pf._count_claude_procfs(cfg, root)[0], 0)
        real_stat = os.stat

        def flaky(path, **k):
            if isinstance(path, str) and path == other:
                raise PermissionError(errno.EACCES, "denied")
            return real_stat(path, **k)

        with patch.object(pf.os, "stat", flaky):
            cnt, detail = pf._count_claude_procfs(cfg, root)
        self.assertIsNone(cnt, detail)


class R5InterruptedTransaction(Base):
    """D4: 교환~검증 창을 의도 저널로 표시하고, 회수는 **두 파일의 바이트**로 판정한다."""

    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)
        _require_exchange(self)

    def journals(self):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_INTENT_PREFIX))

    def displaced(self):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX))

    def crash_after_exchange(self, foreign=b'{"foreign":"newer"}'):
        """교환은 성립시키고 그 직후 죽인다(마감 감시 os._exit 와 같은 상태 집합)."""
        real = pf._exchange_paths

        def exchange_then_die(a, b):
            _write(self.cfgfile, foreign.decode())
            r = real(a, b)
            raise KeyboardInterrupt("killed right after exchange")

        with patch.object(pf, "_exchange_paths", exchange_then_die):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()

    def test_r5_stranded_foreign_document_is_refused_not_silently_ok(self):
        """리뷰 codex major: 종전엔 다음 실행이 `already-trusted` 로 OK 를 내 고립된 사용자 문서가 영영 묻혔다."""
        self.untrusted_file('{"projects": {}, "user": "original"}')
        foreign = b'{"projects": {}, "user": "newest"}'
        self.crash_after_exchange(foreign)
        self.assertEqual(len(self.journals()), 1, os.listdir(self.cfg))
        self.assertEqual(len(self.displaced()), 1, os.listdir(self.cfg))
        d = os.path.join(self.cfg, self.displaced()[0])
        self.assertEqual(_read_bytes(d), foreign)
        for _ in range(2):                      # 멱등: 몇 번을 돌려도 같은 거부, 상태 무변경
            rc, verdict, reason = self.seed()
            self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
            self.assertIn("interrupted-transaction", reason)
            self.assertEqual(_read_bytes(d), foreign)
            self.assertEqual(len(self.journals()), 1)
        # 활성 문서가 그 사이 더 새로 바뀌어도 양쪽 다 보존한다
        _write(self.cfgfile, '{"projects": {}, "user": "even-newer"}')
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 2, reason)
        self.assertEqual(_read_bytes(d), foreign)
        self.assertEqual(_read_json(self.cfgfile)["user"], "even-newer")

    def test_r5_crash_before_exchange_is_not_labelled_foreign(self):
        """codex D4 표: 저널은 있는데 displaced 가 **우리 payload** 면 낯선 데이터가 아니다 — 거부하지 않는다."""
        self.untrusted_file('{"projects": {}}')
        with patch.object(pf, "_exchange_paths", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        self.assertEqual(len(self.journals()), 1, os.listdir(self.cfg))
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertIn("recovery-reclaimed", reason)   # ★triage I1 재핀: 회수 ⓑ 가 스스로 지운다(청소 위임 폐기)
        self.assertEqual(self.journals(), [])
        self.assertEqual(self.displaced(), [])
        self.assertIs(_read_json(self.cfgfile)["projects"][self.key]["hasTrustDialogAccepted"], True)

    def test_r5_crash_after_validation_reclaims_the_captured_document(self):
        """검증까지 끝나고 청소 전에 죽으면 displaced 는 **우리가 읽은 원본**이다 — 활성 문서가 유효하면 회수한다."""
        original = '{"projects": {}, "user": "original"}'
        self.untrusted_file(original)
        real_unlink = os.unlink

        def die_before_cleanup(path, *a, **k):
            if os.path.basename(path).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                raise KeyboardInterrupt("killed before displaced cleanup")
            return real_unlink(path, *a, **k)

        with patch.object(pf.os, "unlink", die_before_cleanup):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        d = os.path.join(self.cfg, self.displaced()[0])
        self.assertEqual(_read_text(d), original)
        self.assertEqual(len(self.journals()), 1)
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertIn("already-trusted", reason)
        self.assertFalse(os.path.exists(d), "원본과 바이트가 같은 displaced 가 회수되지 않았다")
        self.assertEqual(self.journals(), [])

    def test_r5_captured_displaced_is_kept_when_active_is_broken(self):
        """codex D4: 지문 동등만으로 삭제를 인가하지 않는다 — 활성이 깨졌으면 displaced 가 유일한 유효 사본일 수 있다."""
        original = '{"projects": {}, "user": "original"}'
        self.untrusted_file(original)
        real_unlink = os.unlink

        def die_before_cleanup(path, *a, **k):
            if os.path.basename(path).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                raise KeyboardInterrupt("killed")
            return real_unlink(path, *a, **k)

        with patch.object(pf.os, "unlink", die_before_cleanup):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        d = os.path.join(self.cfg, self.displaced()[0])
        _write(self.cfgfile, "{broken")                       # 활성 문서가 손상됐다
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("interrupted-transaction", reason)
        self.assertEqual(_read_text(d), original, "유일할 수 있는 유효 사본을 지웠다")

    def crash_before_exchange(self, original='{"projects": {}, "precious": "original"}'):
        """교환 **직전**(저널은 쓰였고 displaced 엔 우리 payload = 원본+플래그)에서 죽는다."""
        self.untrusted_file(original)
        with patch.object(pf, "_exchange_paths", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        self.assertEqual(len(self.journals()), 1, os.listdir(self.cfg))
        d = os.path.join(self.cfg, self.displaced()[0])
        self.assertEqual(_read_json(d)["precious"], "original")
        return d

    def test_r5_payload_leftover_is_kept_when_active_is_broken(self):
        """★codex 위임 반례(FAILS-NOW 였다): 그 잔재는 '우리 payload' 지만 내용은 **원본 + 플래그** 다 — 활성이 깨진
        뒤 저널만 지우면 곧바로 지문 청소가 삭제해 사용자 필드까지 잃었다."""
        d = self.crash_before_exchange()
        payload = _read_bytes(d)
        _write(self.cfgfile, "{broken")                      # 다른 기록자가 중간에 죽었다
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("interrupted-transaction", reason)
        self.assertTrue(os.path.exists(d), "유일한 완전한 문서를 지웠다")
        self.assertEqual(_read_bytes(d), payload)

    def test_r5_payload_leftover_is_kept_when_active_vanishes(self):
        """같은 결함의 다른 형: 활성 이름이 사라진 뒤 재시작하면 종전엔 새 최소 문서를 시드하고 원본 필드를 잃었다."""
        d = self.crash_before_exchange()
        os.unlink(self.cfgfile)
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertTrue(os.path.exists(d))
        self.assertEqual(_read_json(d)["precious"], "original")

    def test_r5_missing_displaced_with_healthy_active_recovers(self):
        self.untrusted_file('{"projects": {}}')
        self.crash_after_exchange()
        d = os.path.join(self.cfg, self.displaced()[0])
        os.unlink(d)                                          # 외부에서 옮겨졌다/치워졌다
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertEqual(self.journals(), [])

    def test_r5_malformed_journal_refuses_and_touches_nothing(self):
        """저널이 손상·심링크·FIFO·경로 탈출·잘림이면 '모른다' 이고 무접촉 거부다."""
        self.untrusted_file('{"projects": {}}')
        before = _read_bytes(self.cfgfile)
        victim = os.path.join(self.tmp, "victim.json")
        _write(victim, "precious")
        good = pf.SEED_TRUST_DISPLACED_PREFIX + "a" * 64 + "-x"
        cases = {
            "truncated": '{"v":1,"displaced":',
            "escape": json.dumps({"v": 1, "displaced": "../victim.json",
                                  "captured_sha256": "a" * 64, "payload_sha256": "b" * 64}),
            "badname": json.dumps({"v": 1, "displaced": "other-file",
                                   "captured_sha256": "a" * 64, "payload_sha256": "b" * 64}),
            "baddigest": json.dumps({"v": 1, "displaced": good,
                                     "captured_sha256": "zz", "payload_sha256": "b" * 64}),
            "notdict": '["x"]',
        }
        for name, body in cases.items():
            with self.subTest(case=name):
                j = os.path.join(self.cfg, pf.SEED_TRUST_INTENT_PREFIX + "case")
                _write(j, body)
                rc, verdict, reason = self.seed()
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                self.assertIn("interrupted-transaction", reason)
                self.assertEqual(_read_bytes(self.cfgfile), before)
                self.assertEqual(_read_text(victim), "precious")
                os.unlink(j)
        if hasattr(os, "mkfifo"):
            j = os.path.join(self.cfg, pf.SEED_TRUST_INTENT_PREFIX + "fifo")
            os.mkfifo(j)
            rc, verdict, reason = self.seed()
            self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
            os.unlink(j)

    def test_r5_journal_write_failure_prevents_exchange(self):
        """저널을 못 쓰면 창을 열지 않는다 — 교환 0 · 문서 무변경 · 잔재 0."""
        original = '{"projects": {}}'
        self.untrusted_file(original)
        with patch.object(pf, "_write_seed_intent", side_effect=OSError(errno.ENOSPC, "no space")), \
                patch.object(pf, "_exchange_paths", side_effect=AssertionError("저널 없이 교환했다")):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("의도 저널", reason)
        self.assertEqual(_read_text(self.cfgfile), original)
        self.assertEqual(self.displaced(), [])
        self.assertEqual(self.journals(), [])

    def test_r5_absent_document_path_writes_no_journal(self):
        """부재 문서(link 커밋)와 교환 기구 부재는 교환 창이 없다 — 저널도 남기지 않는다(Windows 경로 포함)."""
        rc, verdict, reason = pf.seed_trust(self.cfg, self.ws, proc_counter=_no_probe)
        self.assertEqual(rc, 0, reason)
        self.assertEqual(self.journals(), [])
        _write(self.cfgfile, '{"projects": {}}')
        with patch.object(pf, "_exchange_paths", lambda a, b: pf._ExchangeUnavailable("ENOTSUP")):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("exchange-unavailable", reason)
        self.assertEqual(self.journals(), [])
        self.assertEqual(self.displaced(), [])


class R6RecoveryHealth(Base):
    """★R6 — 리뷰 codex major 3종: ①잘린 활성 문서를 '건강' 으로 보지 않는다 ②저널 열거 실패는 '저널 없음' 이 아니다
    ③저널 내구성 실패(디렉터리 fsync)는 교환을 열지 않는다. 전부 **보존 방향**이다."""

    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)
        _require_exchange(self)

    def journals(self):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_INTENT_PREFIX))

    def displaced(self):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(pf.SEED_TRUST_DISPLACED_PREFIX))

    def crash_before_exchange(self, original):
        """교환 직전에 죽인다 — displaced 에는 **우리 payload**(원본 + 플래그)만 있다."""
        self.untrusted_file(original)
        with patch.object(pf, "_exchange_paths", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        self.assertEqual(len(self.journals()), 1, os.listdir(self.cfg))
        self.assertEqual(len(self.displaced()), 1, os.listdir(self.cfg))
        return os.path.join(self.cfg, self.displaced()[0])

    def crash_after_exchange(self, original, foreign):
        """교환 뒤 검증 전에 죽인다 — displaced 에는 **우리가 읽은 원본**, 활성엔 상대 문서가 앉는다."""
        self.untrusted_file(original)
        real = pf._exchange_paths

        def exchange_then_die(a, b):
            real(a, b)
            raise KeyboardInterrupt("killed right after exchange")

        with patch.object(pf, "_exchange_paths", exchange_then_die):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        _write(self.cfgfile, foreign)
        self.assertEqual(len(self.journals()), 1, os.listdir(self.cfg))
        return os.path.join(self.cfg, self.displaced()[0])

    def test_r6_truncated_active_never_authorises_deleting_the_recovery_copy(self):
        """리뷰 codex major#1: `_parse_claude_json` 은 0바이트/공백을 `{}` 로 바꾼다 — 그 관용을 '건강' 으로 읽으면
        중단 뒤 잘린 활성 문서가 **유일한 완전한 사본**의 삭제를 인가했다. 이제 전부 REFUSE + 두 파일 무접촉."""
        for broken in (b"", b"   \n\t", b"{broken", b"[]", b'"str"'):
            with self.subTest(active=broken, branch="displaced==payload"):
                dpath = self.crash_before_exchange('{"projects": {}, "user": "original"}')
                before = _read_bytes(dpath)
                _write_bytes(self.cfgfile, broken)
                rc, verdict, reason = self.seed()
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                self.assertIn("interrupted-transaction", reason)
                self.assertEqual(_read_bytes(dpath), before, "보존 사본이 사라졌다")
                self.assertEqual(_read_bytes(self.cfgfile), broken, "잘린 활성 문서를 건드렸다")
                self.assertEqual(len(self.journals()), 1, "저널을 지웠다(다음 실행이 못 본다)")
                shutil.rmtree(self.cfg)
                os.makedirs(self.cfg)
        for broken in (b"", b"  ", b"{broken"):
            with self.subTest(active=broken, branch="displaced==captured"):
                dpath = self.crash_after_exchange('{"projects": {}, "user": "original"}', '{"foreign": 1}')
                before = _read_bytes(dpath)
                _write_bytes(self.cfgfile, broken)
                rc, verdict, reason = self.seed()
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                self.assertIn("interrupted-transaction", reason)
                self.assertEqual(_read_bytes(dpath), before, "유일한 유효 사본을 지웠다")
                self.assertEqual(len(self.journals()), 1)
                shutil.rmtree(self.cfg)
                os.makedirs(self.cfg)

    def test_r6_healthy_active_still_recovers(self):
        """음성 대조(주경로 보존): 활성 문서가 **실제 JSON 객체**면 회수는 종전대로 돈다 — 이 수정이 회수를 죽이지 않았다."""
        dpath = self.crash_after_exchange('{"projects": {}, "user": "original"}',
                                          '{"projects": {}, "user": "original"}')
        self.assertTrue(os.path.exists(dpath))
        _write(self.cfgfile, '{"projects": {}, "user": "original"}')
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertEqual(self.journals(), [], "회수가 저널을 남겼다")
        self.assertFalse(os.path.exists(dpath), "회수가 displaced 를 남겼다")

    def test_r6_journal_scan_failure_is_refuse_not_silence(self):
        """리뷰 codex major#2: 디렉터리 열거가 막히면 종전엔 `None`(계속) → 미해결 저널을 안고 `already-trusted OK`.
        이제 REFUSE `journal-scan-failed` 이고 청소·판정보다 **먼저** 접힌다(sweep·계획 진입 0)."""
        _write(self.cfgfile, json.dumps({"projects": {self.key: {"hasTrustDialogAccepted": True}}}))
        real_listdir = os.listdir
        for e in (errno.EACCES, errno.EIO, errno.ENOENT, errno.ESTALE):
            with self.subTest(errno=e):
                def blind(path, _e=e):
                    if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(self.cfg):
                        raise OSError(_e, "injected")
                    return real_listdir(path)
                with patch.object(pf.os, "listdir", blind), \
                        patch.object(pf, "_sweep_stale_seed_tmp", side_effect=AssertionError("열거 실패인데 청소했다")):
                    rc, verdict, reason = self.seed()
                self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
                self.assertIn("journal-scan-failed", reason)
                self.assertIn(errno.errorcode[e], reason)

    def test_r6_unstattable_displaced_is_not_treated_as_absent(self):
        """리뷰 codex(추가): `os.path.lexists` 는 EACCES/EIO/ESTALE 를 '없다' 로 접는다 — 회수에서 그 접힘은 곧
        '지워도 된다' 가 된다. 부재류(ENOENT/ENOTDIR)만 증명된 부재이고 나머지는 REFUSE(무접촉)여야 한다."""
        real_lstat = os.lstat
        for e, expect_refuse in ((errno.EACCES, True), (errno.EIO, True), (errno.ESTALE, True),
                                 (errno.ENOENT, False), (errno.ENOTDIR, False)):
            with self.subTest(errno=e):
                shutil.rmtree(self.cfg)
                os.makedirs(self.cfg)
                dpath = self.crash_before_exchange('{"projects": {}, "user": "original"}')
                _write(self.cfgfile, '{"projects": {}, "user": "original"}')   # 활성은 유효(부재류의 회수 조건)

                def flaky(path, _e=e, **kw):
                    if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(dpath):
                        raise OSError(_e, "injected")
                    return real_lstat(path, **kw)

                with patch.object(pf.os, "lstat", flaky):
                    got = pf._recover_interrupted_seed(self.cfg, self.cfgfile)
                if expect_refuse:
                    self.assertIsNotNone(got, "조회 실패를 '부재' 로 접었다(다음 단계가 지운다)")
                    self.assertEqual(got[:2], (2, "REFUSE"), got)
                    self.assertIn("존재를 조회할 수 없다", got[2])
                    self.assertEqual(len(self.journals()), 1, "무접촉이어야 하는데 저널을 지웠다")
                    self.assertTrue(os.path.exists(dpath))
                else:
                    self.assertIsNone(got, got)        # 증명된 부재 + 활성 유효 → 저널만 회수하고 계속
                    self.assertEqual(self.journals(), [], "증명된 부재인데 저널이 남았다")
                    self.assertTrue(os.path.exists(dpath), "저널 회수가 displaced 파일까지 지웠다")

    def test_r6_journal_durability_failure_blocks_the_exchange(self):
        """리뷰 codex major#3: `_fsync_dir` 가 EIO 까지 삼켜 '이름이 먼저 내구적' 이라는 의무 없이 교환을 열었다.
        저널 기록기를 통째로 mock 하지 않고 **밑단 open/fsync 실패**를 주입한다(codex R6 요구)."""
        original = '{"projects": {}, "user": "original"}'
        real_fsync, real_open = os.fsync, os.open
        cases = [("fsync", errno.EIO), ("fsync", errno.ENOSPC), ("open", errno.EACCES), ("open", errno.EIO)]
        for where, e in cases:
            with self.subTest(where=where, errno=e):
                self.untrusted_file(original)

                def bad_fsync(fd, _e=e):
                    if stat.S_ISDIR(os.fstat(fd).st_mode):
                        raise OSError(_e, "injected")
                    return real_fsync(fd)

                def bad_open(path, flags, *a, _e=e, **kw):
                    if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(self.cfg):
                        raise OSError(_e, "injected")
                    return real_open(path, flags, *a, **kw)

                patcher = (patch.object(pf.os, "fsync", bad_fsync) if where == "fsync"
                           else patch.object(pf.os, "open", bad_open))
                with patcher, patch.object(pf, "_exchange_paths",
                                           side_effect=AssertionError("내구적 저널 없이 교환했다")):
                    rc, verdict, reason = self.seed()
                self.assertEqual((rc, verdict), (1, "ERROR"), reason)
                self.assertIn("의도 저널", reason)
                self.assertEqual(_read_text(self.cfgfile), original, "거부인데 문서가 바뀌었다")
                self.assertEqual(self.journals(), [], "내구성을 못 세운 저널 이름이 남았다")
                self.assertEqual(self.displaced(), [], "displaced 잔재가 남았다")
                shutil.rmtree(self.cfg)
                os.makedirs(self.cfg)

    def test_r6_unsupported_directory_fsync_still_seeds(self):
        """가용성 대조: 디렉터리 fsync 가 **원리적으로 없는** FS(EINVAL/ENOTSUP/ENOSYS)는 통과한다 — 그 errno 까지
        막으면 그런 FS 의 부트가 전부 REFUSE 가 된다(치명위험 ④ 방향)."""
        real_fsync = os.fsync
        for e in (errno.EINVAL, errno.ENOTSUP, errno.ENOSYS):
            with self.subTest(errno=e):
                self.untrusted_file('{"projects": {}, "user": "original"}')

                def bad_fsync(fd, _e=e):
                    if stat.S_ISDIR(os.fstat(fd).st_mode):
                        raise OSError(_e, "injected")
                    return real_fsync(fd)

                with patch.object(pf.os, "fsync", bad_fsync):
                    rc, verdict, reason = self.seed()
                self.assertEqual(rc, 0, reason)
                self.assertIs(_read_json(self.cfgfile)["projects"][self.key]["hasTrustDialogAccepted"], True)
                self.assertEqual(_read_json(self.cfgfile)["user"], "original", "원본 필드를 잃었다")
                shutil.rmtree(self.cfg)
                os.makedirs(self.cfg)

    def test_r6_trailing_newline_in_a_digest_is_not_valid_hex(self):
        r"""★R6(codex 위임 반례 ①): 파이썬 정규식 `$` 는 **문자열 끝 개행 앞**에도 일치한다 — `<64hex>\n` 이
        '유효한 지문' 으로 통과하면 형식 위반 저널이 회수 경로로 들어가(displaced 부재 + 활성 유효 분기) 조용히
        지워졌다. 이제 `\Z` 라 판독은 None 이고 회수는 무접촉 REFUSE 다."""
        _write(self.cfgfile, '{"projects": {}}')
        dname = pf.SEED_TRUST_DISPLACED_PREFIX + ("a" * 64) + "-20260907T000000Z-1"
        jpath = os.path.join(self.cfg, pf.SEED_TRUST_INTENT_PREFIX + "20260907T000000Z-9")
        for field in ("captured_sha256", "payload_sha256"):
            with self.subTest(field=field):
                rec = {"v": 1, "displaced": dname, "captured_sha256": "b" * 64, "payload_sha256": "c" * 64}
                rec[field] = rec[field] + "\n"
                _write(jpath, json.dumps(rec))
                self.assertIsNone(pf._read_seed_intent(jpath), "개행이 붙은 지문을 유효로 봤다")
                got = pf._recover_interrupted_seed(self.cfg, self.cfgfile)
                self.assertEqual(got[:2], (2, "REFUSE"), got)
                self.assertTrue(os.path.exists(jpath), "형식 위반 저널을 지웠다(증거 소실)")
        os.unlink(jpath)
        # 잔재 이름 정규식도 같은 축이다(POSIX 파일명은 개행을 담을 수 있다)
        self.assertIsNone(pf._SEED_TMP_LITTER_RE.match(".claude.json.seed-abcdefgh\n"))
        self.assertIsNotNone(pf._SEED_TMP_LITTER_RE.match(".claude.json.seed-abcdefgh"))
        self.assertIsNone(pf._SEED_HEX64_RE.match("0" * 64 + "\n"))
        self.assertIsNotNone(pf._SEED_HEX64_RE.match("0" * 64))

    def test_r6_cleanup_failure_leaves_the_journal_for_the_next_run(self):
        """codex R6(추가 ③): displaced 폐기가 실패하면 데이터 판정은 끝났어도 **정리는 못 했다** — 저널을 남겨
        다음 실행이 다시 시도하고 C58 이 잔존을 드러낸다('회수 성공 = 저널 0' 을 단정하지 않는다)."""
        original = '{"projects": {}, "user": "original"}'
        dpath = self.crash_after_exchange(original, original)
        _write(self.cfgfile, original)
        real_unlink = os.unlink

        def sticky(path, **kw):
            if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(dpath):
                raise OSError(errno.EIO, "injected")
            return real_unlink(path, **kw)

        with patch.object(pf.os, "unlink", sticky):
            rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)                    # 데이터는 안전하다 — 부트 경로를 막지 않는다
        self.assertEqual(len(self.journals()), 1, "정리 실패인데 저널을 지웠다(잔존이 안 보인다)")
        self.assertTrue(os.path.exists(dpath))


class R7OrphanRecovery(Base):
    """★R7 — 리뷰 codex major('재시도가 보호된 회수 사본을 지워도 되는 잔재로 바꾼다') + 그 설계 비평이 든 두 구멍.
    계약: ①의도 저널은 payload rename **보다 먼저** 공개된다 ②저널 없는 지문 일치 사본 + 불건강 활성 = **대체 문서를
    만들지 않는다**(사본을 자동 삭제되지 않는 이름으로 옮기고 1회 REFUSE) ③교환이 성립하지 않은 자리의 사본 삭제는
    활성 문서가 유효할 때만 ④회수 ⓑ 는 '건강' 이 아니라 **바이트 증명**으로만 사본을 지운다.
    전부 보존 방향이고, 거부는 한 번으로 끝난다(다음 실행에는 사본이 보호 네임스페이스에 있다)."""

    ORIGINAL = '{"user": "ONLY-COPY", "projects": {}}'

    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)

    def names(self, prefix):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(prefix))

    def payload_bytes(self, original=None):
        """시더가 만들었을 payload(원본 + 플래그) 바이트 — 직렬화 형식까지 시더와 같게."""
        new, _changed, _k = pf.trust_plan(json.loads(original or self.ORIGINAL), self.key)
        return json.dumps(new, ensure_ascii=False, indent=2).encode("utf-8")

    def assert_only_copy_survives(self):
        """★codex 위임 반례 채택: 어느 파일에서든 `user` 가 살아 있는가 — 사본 이름·경로가 아니라 **데이터**로 본다."""
        survivors = []
        for root, _dirs, files in os.walk(self.tmp):
            for name in files:
                try:
                    doc = json.loads(_read_bytes(os.path.join(root, name)))
                except (ValueError, UnicodeDecodeError, OSError):
                    continue
                if isinstance(doc, dict) and doc.get("user") == "ONLY-COPY":
                    survivors.append(os.path.join(root, name))
        self.assertTrue(survivors, "유일한 사용자 필드가 모든 파일에서 사라졌다")

    def plant_orphan(self, payload):
        """저널 **없는** 지문 일치 보존 사본 — 종전 순서(rename → 저널)가 중단됐을 때 남던 바로 그 잔재."""
        name = pf.SEED_TRUST_DISPLACED_PREFIX + pf._payload_digest(payload) + "-20260907T000000Z-1"
        _write_bytes(os.path.join(self.cfg, name), payload)
        return name

    def test_r7_two_retries_after_interruption_before_journal_keep_the_only_copy(self):
        """리뷰 codex major 회귀: 저널 공개 전 중단 + 활성 소실 뒤 **연속 두 번**의 재시도가 사용자 필드를 지웠다 —
        1회차가 '플래그만 든 새 문서' 를 만들어 OK 를 냈고 2회차가 그 새 문서를 '건강' 으로 읽어 사본을 청소했다."""
        payload = self.payload_bytes()
        self.plant_orphan(payload)
        rc, verdict, reason = self.seed()                       # 재시도 1
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("unresolved-recovery", reason)
        self.assertFalse(os.path.exists(self.cfgfile), "거부인데 대체 문서를 만들었다")
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertIn(kept[0], reason, "거부 사유가 보존한 파일 이름을 대지 않는다")
        self.assertEqual(self.names(pf.SEED_TRUST_DISPLACED_PREFIX), [], "사본이 청소 네임스페이스에 남았다")
        self.assertEqual(_read_bytes(os.path.join(self.cfg, kept[0])), payload)
        self.assert_only_copy_survives()
        rc, verdict, reason = self.seed()                       # 재시도 2 — 이제 정상 시드다(거부는 1회)
        self.assertEqual(rc, 0, reason)
        self.assertIs(_read_json(self.cfgfile)["projects"][self.key]["hasTrustDialogAccepted"], True)
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), kept, "재시도가 보존 사본을 지웠다")
        self.assertEqual(json.loads(_read_bytes(os.path.join(self.cfg, kept[0])))["user"], "ONLY-COPY",
                         "사용자 필드가 모든 파일에서 사라졌다(리뷰가 지적한 그 손실)")
        self.assert_only_copy_survives()

    def test_r7_journal_is_published_before_the_payload_rename(self):
        """순서를 **행위로** 못 박는다: 저널을 못 쓰면 payload 는 아직 mkstemp 이름에 있고 보존 이름으로 옮겨진 적이
        없다(종전 순서에서는 rename 이 먼저라 '저널 없는 사본' 이 잠시 존재했다)."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        renames, observed = [], []
        real_rename = os.rename

        def watched(a, b, *r, **k):
            renames.append((a, b))
            return real_rename(a, b, *r, **k)

        def fail_intent(*a, **k):
            observed.append(self.names(pf.SEED_TRUST_DISPLACED_PREFIX))   # ★codex: **그 순간**의 상태
            raise OSError(errno.ENOSPC, "no space")

        with patch.object(pf.os, "rename", watched), \
                patch.object(pf, "_write_seed_intent", side_effect=fail_intent), \
                patch.object(pf, "_exchange_paths", side_effect=AssertionError("저널 없이 교환했다")):
            rc, verdict, reason = self.seed()
        self.assertEqual(observed, [[]], "저널 공개 시도 시점에 payload 가 이미 보존 이름에 있었다(창이 열렸다)")
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("의도 저널", reason)
        self.assertEqual([b for _a, b in renames
                          if os.path.basename(b).startswith(pf.SEED_TRUST_DISPLACED_PREFIX)], [],
                         "저널 실패 전에 payload 를 보존 이름으로 옮겼다(창이 열렸다)")
        self.assertEqual(_read_bytes(self.cfgfile), before)
        self.assertEqual(self.names(pf.SEED_TRUST_DISPLACED_PREFIX), [])
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [])
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [])

    def test_r7_exchange_failure_after_the_active_vanishes_preserves_our_copy(self):
        """리뷰 codex 설계 비평 ②: '교환 안 됨 = 사본을 지워도 안전' 은 거짓이다 — 교환이 ENOENT 로 실패하는 바로 그
        형상(외부가 활성을 지웠다)에서 그 사본이 유일한 완전한 문서다."""
        self.untrusted_file(self.ORIGINAL)

        def vanish_then_fail(a, b):
            os.unlink(b)                                        # 외부 기록자가 활성 문서를 지웠다
            raise OSError(errno.ENOENT, "injected ENOENT")

        with patch.object(pf, "_exchange_paths", vanish_then_fail):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("사본 보존", reason)
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertEqual(json.loads(_read_bytes(os.path.join(self.cfg, kept[0])))["user"], "ONLY-COPY")
        self.assertEqual(self.names(pf.SEED_TRUST_DISPLACED_PREFIX), [])
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [], "처분이 끝났는데 저널이 남았다")

    def test_r7_exchange_failure_with_a_healthy_active_leaves_no_litter(self):
        """음성 대조(주경로 보존): 활성 문서가 그대로면 우리 사본은 낡은 스냅샷이므로 지운다 — conflict 잔재 0.
        (여기서 바이트 동등까지 요구하면 Windows 의 기존 문서 경로마다 conflict 가 쌓인다)"""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        with patch.object(pf, "_exchange_paths", side_effect=OSError(errno.EIO, "injected")):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertNotIn("사본 보존", reason)
        self.assertEqual(_read_bytes(self.cfgfile), before)
        for prefix in (pf.SEED_TRUST_CONFLICT_PREFIX, pf.SEED_TRUST_DISPLACED_PREFIX, pf.SEED_TRUST_INTENT_PREFIX):
            self.assertEqual(self.names(prefix), [], prefix)

    def test_r7_recovery_preserves_when_the_active_was_recreated_empty(self):
        """리뷰 codex 설계 비평 ①: 중단 뒤 외부가 활성을 `{}` 로 재생성하면 '건강' 판정이 통과한다 — 종전엔 저널을
        지우고 지문 청소가 유일한 사용자 사본을 삭제했다. 이제 바이트 증명이 없으면 보존 이름으로 옮긴다."""
        self.untrusted_file(self.ORIGINAL)
        with patch.object(pf, "_exchange_paths", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        self.assertEqual(len(self.names(pf.SEED_TRUST_INTENT_PREFIX)), 1, os.listdir(self.cfg))
        self.assertEqual(len(self.names(pf.SEED_TRUST_DISPLACED_PREFIX)), 1, os.listdir(self.cfg))
        _write(self.cfgfile, "{}")                              # 외부가 새 빈 문서를 만들었다
        before = _read_bytes(self.cfgfile)
        r = self.seed()
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertEqual(json.loads(_read_bytes(os.path.join(self.cfg, kept[0])))["user"], "ONLY-COPY",
                         "유일한 사용자 사본을 지웠다")
        self.assertEqual(self.names(pf.SEED_TRUST_DISPLACED_PREFIX), [])
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [], "처분이 끝났는데 저널이 남았다")
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return                                              # Windows 계약(사본 보존은 위에서 이미 단언했다)
        self.assertEqual(r[0], 0, r)
        self.assertIn("recovery-preserved", r[2], "보존 사실을 사유에 남기지 않았다")

    def test_r7_guard_ignores_copies_that_no_sweep_can_delete(self):
        """가드는 **지문 청소가 지울 수 있는 것**만 막는다 — 지문 불일치 displaced(낯선 문서)는 이미 안전하므로 막지
        않는다(막으면 사람이 손대기 전까지 매번 거부한다 = 1회 계약 위반)."""
        foreign = os.path.join(self.cfg, pf.SEED_TRUST_DISPLACED_PREFIX + ("0" * 64) + "-20260907T000000Z-1")
        _write_bytes(foreign, b'{"foreign": "document"}')
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertEqual(_read_bytes(foreign), b'{"foreign": "document"}', "낯선 문서를 건드렸다")
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [], "안전한 파일을 옮겼다")

    def test_r7_unknown_existence_never_becomes_a_rename_target(self):
        """리뷰 claude minor: `os.path.lexists` 는 EACCES/EIO/ESTALE 를 '없다' 로 접는다 — 그 접힘은 곧 `os.rename`
        의 말없는 덮어쓰기(사람이 병합해야 할 낯선 문서 파괴)다. 조회가 실패한 이름은 쓰지 않고, 상한까지 확정하지
        못하면 **아무것도 옮기지 않는다**(무변경 ERROR)."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        real_lstat = os.lstat

        def blind(path, *a, **k):
            if isinstance(path, (str, bytes)) and os.path.basename(path).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                raise OSError(errno.EACCES, "injected")         # '없다' 가 아니라 '모른다'
            return real_lstat(path, *a, **k)

        with patch.object(pf.os, "lstat", blind), \
                patch.object(pf.os, "rename", side_effect=AssertionError("조회 불가 이름으로 옮겼다")):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("보존 이름 확정 실패", reason)
        self.assertEqual(_read_bytes(self.cfgfile), before)
        for prefix in (pf.SEED_TRUST_DISPLACED_PREFIX, pf.SEED_TRUST_INTENT_PREFIX, pf.SEED_TRUST_CONFLICT_PREFIX):
            self.assertEqual(self.names(prefix), [], prefix)

    def test_r7_guard_scan_failure_is_refuse_not_silence(self):
        """가드의 열거 실패는 '사본 없음' 이 아니다(결측은 값이 아니다) — 저널 열거와 같은 규율."""
        payload = self.payload_bytes()
        self.plant_orphan(payload)
        real_listdir = os.listdir
        seen = []

        def blind(path, *a, **k):
            if isinstance(path, str) and os.path.abspath(path) == os.path.abspath(self.cfg):
                seen.append(path)
                if len(seen) == 2:                              # 1회차 = 저널 열거(정상) · 2회차 = 가드 열거
                    raise OSError(errno.EACCES, "injected")
            return real_listdir(path, *a, **k)

        with patch.object(pf.os, "listdir", blind), \
                patch.object(pf, "_sweep_stale_seed_tmp", side_effect=AssertionError("열거 실패인데 청소했다")):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assertIn("orphan-scan-failed", reason)
        self.assertIn("EACCES", reason)
        self.assertFalse(os.path.exists(self.cfgfile), "거부인데 대체 문서를 만들었다")


class R7ExclusiveName(unittest.TestCase):
    """★R7 — 보존 이름 생성기의 3값 규율(★codex 위임 반례에서 채택 · 워커가 전 행 검토). 순수 함수라 격리 env 불요."""

    def test_r7_unknown_existence_exhausts_the_cap_and_raises(self):
        probe = Mock(return_value=None)                          # 조회 실패만 — '없다' 가 아니다
        with self.assertRaises(OSError):
            pf._exclusive_name("/nowhere/copy-", lexists=probe)
        self.assertEqual(probe.call_count, pf._EXCLUSIVE_NAME_MAX_TRIES)
        self.assertEqual(len({c.args[0] for c in probe.call_args_list}), pf._EXCLUSIVE_NAME_MAX_TRIES,
                         "같은 후보를 다시 물었다(무한 루프의 다른 형)")

    def test_r7_existing_name_moves_to_the_next_candidate(self):
        probe = Mock(side_effect=[True, False])
        with patch.object(pf.time, "strftime", return_value="20260907T000000Z"), \
                patch.object(pf.os, "getpid", return_value=123):
            got = pf._exclusive_name("/nowhere/copy-", lexists=probe)
        base = "/nowhere/copy-20260907T000000Z-123"
        self.assertEqual(got, base + "-1")
        self.assertEqual([c.args[0] for c in probe.call_args_list], [base, base + "-1"])

    def test_r7_proven_absence_uses_the_first_name(self):
        probe = Mock(return_value=False)
        with patch.object(pf.time, "strftime", return_value="20260907T000000Z"), \
                patch.object(pf.os, "getpid", return_value=123):
            got = pf._exclusive_name("/nowhere/copy-", lexists=probe)
        self.assertEqual(got, "/nowhere/copy-20260907T000000Z-123")
        probe.assert_called_once_with(got)

    def test_r7_default_probe_is_the_strict_one(self):
        with patch.object(pf, "_lexists_strict", return_value=None) as strict:
            with self.assertRaises(OSError):
                pf._exclusive_name("/nowhere/copy-")
        self.assertEqual(strict.call_count, pf._EXCLUSIVE_NAME_MAX_TRIES)


class R5Watchdog(Base):
    """D5: 마감 감시의 완료 판정·유한값·종료 보장."""

    def test_r5_timeout_table_covers_non_finite_and_overflow(self):
        env = pf._SEED_TRUST_TIMEOUT_ENV
        cases = {"": 20.0, "1.5": 1.5, "0": None, "-1": None, "0.5": 0.5,
                 "nan": 20.0, "inf": 20.0, "-inf": 20.0, "1e309": 20.0, "zzz": 20.0,
                 "1e300": threading.TIMEOUT_MAX}
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(pf._seed_trust_timeout_secs({env: raw} if raw else {}), expected)

    def test_r5_cancel_wins_the_race_after_return(self):
        """리뷰 minor: 커밋 뒤 취소 직전에 타이머가 들어와도 성공을 timeout 으로 뒤집지 않는다(먼저 잡은 쪽만 이긴다).
        (이 검체는 `Timer.cancel()` 만으로도 통과한다 — **경쟁 창 자체**는 아래 `…_late_timer…` 가 못 박는다.)"""
        fired = []
        cancel = pf._start_seed_deadline(0.05, lambda: fired.append(1))
        self.assertIsNotNone(cancel)
        cancel()
        time.sleep(0.25)
        self.assertEqual(fired, [], "취소 뒤에도 감시가 발화했다")

    def test_r6_cancel_claims_before_an_uncancellable_late_timer_fires(self):
        """★R6(리뷰 minor#3): 실제 경쟁 창은 `Timer.cancel()` 이 **이미 무효**인 순간이다 — 타이머 스레드가 발화
        경로에 들어간 뒤엔 cancel 이 아무 것도 못 막으므로, 취소가 이기려면 `_claim()` 이 먼저여야 한다.
        종전 검체는 `cancel()` 직후 즉시 검사라 `t.cancel()` 만으로도 통과해서(뮤테이션 실측) 이 축이 무보증이었다.
        여기선 취소 불가 타이머를 주입하고 `_fire` 를 **손으로** 뒤늦게 들여보낸다 — 조용히 반환해야 한다."""
        fired, exits, made = [], [], []

        class LateTimer:
            """`cancel()` 이 무효인 타이머(이미 발화 경로) — 발화 함수는 검체가 직접 부른다."""
            daemon = True

            def __init__(self, interval, fn):
                made.append(fn)

            def start(self):
                pass

            def cancel(self):
                pass

        with patch.object(pf.threading, "Timer", LateTimer), \
                patch.object(pf.os, "_exit", lambda code: exits.append(code)):
            cancel = pf._start_seed_deadline(5.0, lambda: fired.append(1))
            self.assertIsNotNone(cancel)
            self.assertEqual(len(made), 1, "마감 타이머가 만들어지지 않았다(하네스 결함)")
            cancel()                      # 정상 경로가 먼저 끝났다 — 여기서 승자를 확정해야 한다
            made[0]()                     # 무효화되지 못한 타이머가 뒤늦게 들어온다
        self.assertEqual(fired, [], "취소가 이겼는데 timeout 진단을 냈다(커밋된 실행이 뒤집힌다)")
        self.assertEqual(exits, [], "취소가 이겼는데 os._exit 했다")

    def test_r6_fire_wins_when_it_claims_first(self):
        """음성 대조: 반대로 발화가 먼저 잡으면 취소는 그것을 되돌리지 못한다(둘 다 조용해지는 회귀 차단)."""
        fired, exits, made = [], [], []

        class LateTimer:
            daemon = True

            def __init__(self, interval, fn):
                made.append(fn)

            def start(self):
                pass

            def cancel(self):
                pass

        with patch.object(pf.threading, "Timer", LateTimer), \
                patch.object(pf.os, "_exit", lambda code: exits.append(code)):
            cancel = pf._start_seed_deadline(5.0, lambda: fired.append(1))
            made[0]()                     # 발화가 먼저
            cancel()
        self.assertEqual(fired, [1], "발화가 먼저 잡았는데 진단을 내지 않았다")
        self.assertEqual(exits, [pf.SEED_TRUST_REFUSE], exits)

    def test_r5_watchdog_start_failure_refuses_without_seeding(self):
        """감시 스레드를 못 띄우면 시간 상한 없이 부트 경로를 붙잡지 않는다 — 무쓰기 REFUSE."""
        with patch.object(pf, "_start_seed_deadline", lambda secs, emit: None), \
                patch.object(pf, "seed_trust", side_effect=AssertionError("감시 없이 시드했다")):
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = pf._seed_trust_main(["--seed-trust", "--config", self.cfg, "--cwd", self.ws])
        self.assertEqual(rc, 2)
        self.assertIn("watchdog-unavailable", buf.getvalue())
        self.assertFalse(os.path.exists(self.cfgfile))

    def test_r5_blocked_emit_still_terminates_even_if_hard_timer_fails(self):
        """★codex 위임 반례(FAILS-NOW 였다): `emit()` 이 영원히 막히는데 하드 종료 타이머 기동까지 실패하면 자식이
        영원히 살아 부트 호출자를 붙잡았다 — 종료 보증이 없으면 **진단을 포기**하고 즉시 끝낸다."""
        code = ("import threading, time\n"
                "import javis_preflight as pf\n"
                "real = threading.Timer\n"
                "made = []\n"
                "class Dead:\n"
                "    daemon = True\n"
                "    def start(self):\n"
                "        raise RuntimeError('no thread')\n"
                "    def cancel(self):\n"
                "        pass\n"
                "def timer(interval, fn):\n"
                "    made.append(1)\n"
                "    return real(interval, fn) if len(made) == 1 else Dead()\n"
                "threading.Timer = timer\n"
                "def blocked():\n"
                "    sys.stdout.write('EMIT\\n'); sys.stdout.flush(); time.sleep(600)\n"
                "pf._start_seed_deadline(0.05, blocked)\n"
                "time.sleep(600)\n")
        try:
            r = _child(code, timeout=20)
        except subprocess.TimeoutExpired:
            self.fail("하드 타이머 기동 실패 시 자식이 끝나지 않았다(부트 호출자가 영원히 붙잡힌다)")
        self.assertEqual(r.returncode, pf.SEED_TRUST_REFUSE, (r.stdout, r.stderr))
        self.assertNotIn("EMIT", r.stdout, "종료 보증이 없는데 막히는 진단을 먼저 시도했다")

    def test_r5_timeout_line_does_not_claim_no_write(self):
        """codex D5: '공개 전이면 무쓰기' 단정은 거짓일 수 있다 — 문면이 커밋 여부를 단정하지 않는다."""
        src = inspect.getsource(pf._seed_trust_main)
        self.assertIn("커밋 여부는", src)
        self.assertIn("의도 저널", src)
        fire = inspect.getsource(pf._start_seed_deadline)
        self.assertIn("_SEED_TRUST_EXIT_GRACE", fire, "emit 이 막혀도 종료를 보장하는 backstop 이 없다")


class R6WindowsAxisSweep(unittest.TestCase):
    """★R6(리뷰 codex major#5): '기존 문서 성공' 을 무조건 단언하는 검체는 Windows(교환 기구 부재)에서 반드시
    실패한다. 리뷰어가 든 3건은 전수가 아니었다 — **기계로** 전수를 본다: 교환 기구를 강제로 없애고 이 모듈
    전체를 자식 프로세스에서 돌려 실패 0 을 요구한다(능력 분기 누락의 유일한 결정론 장치).
    재귀 방지: 자식에는 `CYS_TRUST_WIN_AXIS=1` 을 넣고, 그 env 가 있으면 이 검체 자신은 skip 한다."""

    AXIS_ENV = "CYS_TRUST_WIN_AXIS"
    BUDGET_ENV = "CYS_TRUST_WIN_AXIS_TIMEOUT"
    BUDGET_DEFAULT = 900.0        # ★R7(리뷰 claude minor): 종전 420s. 로컬 실측(2026-09-07 15:5x)은 부모 전체 21s ·
                                  #   축 자식 4~6s 라 여유율은 이미 크지만, CI 3레인(특히 windows-health 러너)이 겹치면
                                  #   **코드 회귀가 아닌 시간**으로 적색이 될 수 있다 → 상한을 올리고 env 로 연다.
                                  #   상한은 감시이지 성능 핀이 아니다(초과의 대가는 '자식이 영원히 붙잡히지 않는다' 하나).

    @classmethod
    def budget(cls, env=None):
        """축 상한(초) — 미설정/비수치/비유한/0 이하는 기본값(감시를 잃지 않는 방향 · _seed_trust_timeout_secs 와 같은 규율)."""
        raw = (os.environ if env is None else env).get(cls.BUDGET_ENV, "")
        try:
            v = float(raw)
        except (TypeError, ValueError):
            return cls.BUDGET_DEFAULT
        return v if (math.isfinite(v) and v > 0) else cls.BUDGET_DEFAULT

    def test_r7_axis_budget_is_overridable_and_never_lost(self):
        """상한 파싱 표 — 잘못된 값이 감시를 지우지 않는다."""
        for raw, want in (("", 900.0), ("abc", 900.0), ("0", 900.0), ("-5", 900.0), ("nan", 900.0),
                          ("inf", 900.0), ("120", 120.0), ("1800.5", 1800.5)):
            with self.subTest(raw=raw):
                self.assertEqual(self.budget({self.BUDGET_ENV: raw}), want)
        self.assertEqual(self.budget({}), 900.0)

    def test_r6_no_test_requires_the_exchange_unconditionally(self):
        if os.environ.get(self.AXIS_ENV):
            self.skipTest("교환 강제 비활성 축 안에서는 이 검체 자신을 다시 돌리지 않는다(재귀 방지)")
        if not _exchange_supported():
            self.skipTest("이미 교환 기구가 없는 플랫폼 — 이 축이 곧 본 실행이다")
        # 대상은 **in-process 시더를 부르는 클래스**뿐이다: 시더를 자식 프로세스로만 부르는 클래스(cys-dept bash ·
        #   seed_cli 자식)는 이 monkeypatch 가 닿지 않아 신호가 0 이고, 그 bash 검체들이 축의 시간을 지배해
        #   부하 아래 timeout 으로 흔들렸다(실측). 선택은 **기계**가 한다 — 새 클래스가 `self.seed(`/`seed_trust(`/
        #   `self._c58(` 를 쓰면 자동으로 축에 든다(수기 목록의 노후화 0).
        code = (
            "import os, sys, inspect, unittest\n"
            "sys.dont_write_bytecode = True\n"
            "sys.path.insert(0, sys.argv[1])\n"
            "sys.path.insert(0, os.path.join(sys.argv[1], 'tests'))\n"
            "import javis_preflight as pf\n"
            "pf._exchange_paths = lambda a, b: pf._ExchangeUnavailable('platform:nt')\n"
            "import test_trust_seed as t\n"
            "t.pf._exchange_paths = pf._exchange_paths\n"
            "needles = ('seed_trust(', 'self.seed(', 'self._c58(')\n"
            "loader, suite = unittest.TestLoader(), unittest.TestSuite()\n"
            "picked = []\n"
            "for name in sorted(dir(t)):\n"
            "    obj = getattr(t, name)\n"
            "    if not (isinstance(obj, type) and issubclass(obj, unittest.TestCase)):\n"
            "        continue\n"
            "    try:\n"
            "        src = inspect.getsource(obj)\n"
            "    except OSError:\n"
            "        src = ''\n"
            "    if any(n in src for n in needles):\n"
            "        picked.append(name)\n"
            "        suite.addTests(loader.loadTestsFromTestCase(obj))\n"
            "res = unittest.TextTestRunner(verbosity=0).run(suite)\n"
            "print('PICKED:' + '|'.join(picked))\n"
            "print('IDS:' + '|'.join(sorted(c.id() for c, _ in list(res.failures) + list(res.errors))))\n")
        env = dict(os.environ)
        env[self.AXIS_ENV] = "1"
        budget = self.budget()
        t0 = time.monotonic()
        try:
            r = subprocess.run([sys.executable, "-B", "-c", code, BIN],      # -B: 저장소 트리에 __pycache__ 0(SEAL-1)
                               capture_output=True, text=True, env=env, timeout=budget)
        except subprocess.TimeoutExpired:
            self.fail("교환 강제 비활성 축이 %.0fs 안에 끝나지 않았다(로컬 실측은 한 자릿수 초다 — 부하로 느려진 것이라면 "
                      "%s 로 상한을 올린다 · 이 실패는 능력 분기 회귀가 아닐 수 있다)" % (budget, self.BUDGET_ENV))
        elapsed = time.monotonic() - t0
        ids = [line for line in r.stdout.splitlines() if line.startswith("IDS:")]
        picked = [line for line in r.stdout.splitlines() if line.startswith("PICKED:")]
        self.assertEqual(len(ids), 1, r.stdout[-4000:] + r.stderr[-4000:])
        chosen = [x for x in picked[0][7:].split("|") if x] if picked else []
        # 능력 분기를 넣은 다섯 클래스가 축에 **실제로** 들어 있어야 한다(선택기가 조용히 비어 축이 항진명제가 되는 것 차단)
        for required in ("CodexCounterexamples", "CodexR1Counterexamples", "RegistryC58",
                         "DeptContextC58", "R6RecoveryHealth", "R7OrphanRecovery"):
            self.assertIn(required, chosen, "축이 %s 를 고르지 않았다(선택기 회귀)" % required)
        failed = [x for x in ids[0][4:].split("|") if x]
        self.assertEqual(failed, [], "교환 기구 없는 플랫폼에서 실패하는 검체(능력 분기 누락 · 축 %.1fs): %s"
                         % (elapsed, failed))


class R5TestChildSeal(unittest.TestCase):
    """리뷰 codex minor: 이 파일이 띄우는 파이썬 자식은 전부 `-B` 로 봉인돼야 한다(SEAL-1 · 부모 -B 는 상속되지 않는다)."""

    def test_r5_every_python_child_is_sealed(self):
        src = _read_text(os.path.abspath(__file__))
        # 음성 대조군(-B 없는 자식)은 아래 검체가 **일부러** 띄운다 — 그 본문만 빼고 전수 검열한다.
        src = src.replace(inspect.getsource(R5TestChildSeal.test_r5_child_reports_sealed_interpreter), "")
        spawns = re.findall(r"subprocess\.(?:run|Popen)\(\[(PY|sys\.executable)([^\]]*)\]", src)
        self.assertGreaterEqual(len(spawns), 5, "스폰 지점 탐지 자체가 깨졌다(정규식 회귀)")
        for head, tail in spawns:
            with self.subTest(spawn=(head + tail)[:60]):
                self.assertTrue(tail.lstrip().startswith(', "-B"') or tail.lstrip().startswith(", '-B'"),
                                "봉인되지 않은 파이썬 자식: %s%s" % (head, tail[:60]))

    def test_r5_child_reports_sealed_interpreter(self):
        """봉인은 **행위**로 검증한다 — 자식이 import 한 모듈의 바이트코드 캐시가 생기는가. 플래그 이름을 이 파일에
        적지 않는 것은 의도적이다: 봉인 census(`test_pyseal_census.py` ⓑ)의 참조 파일 집합을 늘리지 않는다."""
        d = tempfile.mkdtemp(prefix="sealprobe-")
        self.addCleanup(shutil.rmtree, d, True)
        _write(os.path.join(d, "sealprobe.py"), "VALUE = 1\n")
        cache = os.path.join(d, "__pycache__")
        sealed = subprocess.run([PY, "-B", "-c", "import sealprobe"], cwd=d,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(sealed.returncode, 0, sealed.stderr)
        self.assertFalse(os.path.isdir(cache), "봉인된 자식이 캐시를 썼다")
        unsealed = subprocess.run([PY, "-c", "import sealprobe"], cwd=d,
                                  capture_output=True, text=True, timeout=30)
        self.assertEqual(unsealed.returncode, 0, unsealed.stderr)
        if not os.path.isdir(cache):
            self.skipTest("이 실행 환경은 자식 파이썬을 이미 봉인한다 — 음성 대조군 성립 불가(양성 단언은 통과)")



class TriageP1WP2Trust(Base):
    """독립 재유도(triage · CONTRACTS §E-1) — P1-WP2-trust 잔여 major 4건의 회귀 핀.
    공통 불변(워커가 §R7-2 에서 스스로 세운 것): **보존 사본은 '지워도 잃는 것이 없다' 는 증명 없이는 지우지 않는다**.
    아래 검체는 그 증명(`_copy_is_redundant`)이 적용되지 **않는** 세 자리에서 유일한 사용자 문서가 사라지는 것을
    데이터로 단언한다(파일 이름·경로가 아니라 `user` 필드의 생존으로 본다)."""

    ORIGINAL = '{"user": "ONLY-COPY", "projects": {}}'

    def setUp(self):
        super().setUp()
        os.makedirs(self.cfg)

    def names(self, prefix):
        return sorted(n for n in os.listdir(self.cfg) if n.startswith(prefix))

    def payload_bytes(self, original=None):
        new, _changed, _k = pf.trust_plan(json.loads(original or self.ORIGINAL), self.key)
        return json.dumps(new, ensure_ascii=False, indent=2).encode("utf-8")

    def assert_only_copy_survives(self, why):
        survivors = []
        for root, _dirs, files in os.walk(self.tmp):
            for name in files:
                try:
                    doc = json.loads(_read_bytes(os.path.join(root, name)))
                except (ValueError, UnicodeDecodeError, OSError):
                    continue
                if isinstance(doc, dict) and doc.get("user") == "ONLY-COPY":
                    survivors.append(os.path.join(root, name))
        self.assertTrue(survivors, "%s — 유일한 사용자 필드가 모든 파일에서 사라졌다(%s)"
                        % (why, sorted(os.listdir(self.cfg))))

    # ── ⑪(reviewer-claude major) · 'A healthy replacement bypasses orphan protection'(reviewer-codex major)
    def test_triage_orphan_survives_when_the_active_is_recreated_empty(self):
        """저널 없는 지문 일치 사본 + 외부가 활성을 `{}` 로 재생성 → 가드는 '건강' 에서 즉시 통과하고 지문 청소가
        그 사본을 지운다. `_sweep_stale_seed_tmp` 의 전제는 `_active_document_healthy` 하나이고 R7 의 바이트 증명
        (`_copy_is_redundant`)이 여기엔 적용되지 않는다 — '유효한 문서' 는 '그 데이터가 그 안에 있다' 가 아니다.
        R7 회귀 검체(`test_r7_two_retries_…`)는 활성 **부재**만 덮는다."""
        payload = self.payload_bytes()
        orphan = self.plant_orphan_copy(payload)
        _write(self.cfgfile, "{}")                       # 외부 기록자가 빈 문서를 만들었다(건강 · 사용자 필드 0)
        self.seed()
        self.assertTrue(os.path.exists(os.path.join(self.cfg, orphan))
                        or self.names(pf.SEED_TRUST_CONFLICT_PREFIX),
                        "보존 사본이 증명 없이 사라졌다: %s" % sorted(os.listdir(self.cfg)))
        self.assert_only_copy_survives("빈 활성 문서가 고아 보호를 우회했다")

    def plant_orphan_copy(self, payload):
        name = pf.SEED_TRUST_DISPLACED_PREFIX + pf._payload_digest(payload) + "-20260907T000000Z-1"
        _write_bytes(os.path.join(self.cfg, name), payload)
        return name

    # ── 'Journal-first recovery can delete the unrenamed payload'(reviewer-codex major)
    def test_triage_crash_between_journal_and_rename_keeps_the_only_copy(self):
        """저널 공개 성공 → rename 전 사망(SIGKILL)의 실제 디스크 상태: 저널 + mkstemp payload + 활성.
        그 뒤 외부가 활성을 `{}` 로 바꾸면 회수 ⓐ 는 '가리키는 displaced 부재 + 활성 건강' 으로 저널을 지우고
        `_sweep_stale_seed_tmp` 는 mkstemp 잔재를 **활성 건강과 무관하게 무조건** 지운다(리터럴 분기: `continue`
        앞에서 healthy 판정을 거치지 않는다). payload = 원본 + 플래그이므로 사용자 필드가 전부 사라진다."""
        self.untrusted_file(self.ORIGINAL)
        captured = _read_bytes(self.cfgfile)
        payload = self.payload_bytes()
        displaced = pf.SEED_TRUST_DISPLACED_PREFIX + pf._payload_digest(payload) + "-20260907T000000Z-1"
        litter = os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34")   # mkstemp 접미 8자 형식
        _write_bytes(litter, payload)
        jpath = pf._write_seed_intent(self.cfg, displaced, pf._payload_digest(captured),
                                      pf._payload_digest(payload))
        self.assertTrue(os.path.exists(jpath))
        _write(self.cfgfile, "{}")                       # 외부 기록자가 활성을 빈 문서로 교체
        self.seed()
        self.assert_only_copy_survives("저널~rename 창의 payload 를 회수+청소가 함께 지웠다")

    def test_triage_litter_sweep_is_unconditional_even_with_no_active_document(self):
        """저널 **공개 전** 사망 + 그 뒤 활성 소실 — 저널이 없으니 회수도 가드도 개입하지 않고, 청소는 mkstemp
        잔재를 무조건 지운다. 이 payload 는 원본이 있던 실행에서 만들어진 것이라 '플래그뿐' 이 아니다."""
        payload = self.payload_bytes()
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34"), payload)
        self.seed()
        self.assert_only_copy_survives("활성 부재인데 mkstemp payload 를 지웠다")

    # ── 'Unsuccessful-exchange cleanup still equates validity with redundancy'(reviewer-codex major)
    def test_triage_release_after_a_foreign_replacement_keeps_the_only_copy(self):
        """대조(⑥) 뒤 · 교환 실패 사이에 다른 기록자가 활성을 `{}` 로 바꾸면 `_release_uncommitted_copy` 는
        '유효한 문서가 있다' 만 보고 우리 사본(원본 + 플래그)과 저널을 함께 지운다. 원본은 그 사본에만 있었다."""
        self.untrusted_file(self.ORIGINAL)

        def replace_then_fail(a, b):
            _write(b, "{}")                              # 대조를 통과한 뒤 끼어든 기록자
            raise OSError(errno.EIO, "injected")

        with patch.object(pf, "_exchange_paths", replace_then_fail):
            rc, verdict, reason = self.seed()
        self.assertEqual(rc, 1, reason)
        self.assert_only_copy_survives("교환 실패 경로가 유일한 사본을 지웠다")

    def test_triage_release_on_the_exchange_unavailable_path_keeps_the_only_copy(self):
        """같은 결함의 Windows(기구 부재) 경로 — `_release_uncommitted_copy` 호출 지점이 둘이다."""
        self.untrusted_file(self.ORIGINAL)

        def replace_then_unavailable(a, b):
            _write(b, "{}")
            return pf._ExchangeUnavailable("platform:nt")

        with patch.object(pf, "_exchange_paths", replace_then_unavailable):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (2, "REFUSE"), reason)
        self.assert_only_copy_survives("exchange-unavailable 경로가 유일한 사본을 지웠다")


class TriageConvergence(TriageP1WP2Trust):
    """★triage 수렴(2026-09-08) — 재유도 CONFIRMED 4건을 고치며 **새로 생긴 계약**의 회귀 핀.
    검체가 지키는 것: ①의도 저널이 아직 rename 되지 않은 mkstemp 이름을 담고 회수 ⓐ 가 그것까지 바이트 증명으로
    판정한다 ②통상 경로(활성 무변경)는 그 크래시 뒤에도 잔재·conflict 0 이다(보수화가 상시 잔재가 되지 않는다)
    ③저널이 아직 책임지는 사본은 고아 가드의 대상이 아니다(codex 설계비평 4) ④보존 rename 이 실패하면 저널을
    남긴다(codex 설계비평 2) ⑤플래그뿐인 최소 문서 임시본은 종전대로 청소된다(격리 위양성 0).
    ★수렴 R2(2026-09-08 · 최종 리뷰 claude) 추가분: ⑥저널 공개 **전** 크래시(triage 가 '더 넓다' 고 적은 ⓑ 창)의
    무손실 잔재는 conflict 로 격리되지 않는다 — '지금 다시 계획한 payload 와 바이트 동등' 증명 ⑦그 창은 교환 기구
    상시 부재(Windows) 축에서 부트마다 다시 열리는데 **누적이 없다** ⑧청소가 격리를 해 놓고 `_read_plan()` 오류로
    빠져나가도 사유를 버리지 않는다 ⑨교환 성공 뒤 보존은 성공/실패를 갈라 적는다 ⑩플래그뿐 문서 통의 **실제 범위**
    (다중 키는 든다 · `true` 아닌 플래그는 지킨다).
    `TriageP1WP2Trust` 를 상속하는 것은 **의도**다 — 헬퍼(`payload_bytes`·`assert_only_copy_survives`·`plant_orphan_copy`)를
    공유하고, 재유도 5핀이 이 클래스에서 한 번 더 도는 것이 수렴 계약의 음성 대조가 된다(중복 실행 0.03s)."""

    def crash_between_journal_and_rename(self):
        """SIGKILL 모사 — `finally` 가 돌지 않으므로 mkstemp payload 가 그대로 남는다(KeyboardInterrupt 는 finally 가
        돌아 tmp 를 지우므로 이 창을 재현하지 못한다)."""
        real_rename, real_unlink_quiet = os.rename, pf._unlink_quiet

        def die(a, b, *r, **k):
            if os.path.basename(b).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                raise KeyboardInterrupt("저널 공개 뒤 · rename 전 사망")
            return real_rename(a, b, *r, **k)

        with patch.object(pf.os, "rename", die), patch.object(pf, "_unlink_quiet", lambda path: None):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()

    def test_conv_journal_records_the_unrenamed_payload_and_recovery_judges_it(self):
        self.untrusted_file(self.ORIGINAL)
        self.crash_between_journal_and_rename()
        journals = self.names(pf.SEED_TRUST_INTENT_PREFIX)
        self.assertEqual(len(journals), 1, os.listdir(self.cfg))
        rec = json.loads(_read_bytes(os.path.join(self.cfg, journals[0])).decode("utf-8"))
        self.assertIn("tmp", rec, "저널이 아직 rename 되지 않은 임시 이름을 담지 않았다")
        self.assertIsNotNone(pf._SEED_TMP_LITTER_RE.match(rec["tmp"]), rec["tmp"])
        self.assertEqual(_read_bytes(os.path.join(self.cfg, rec["tmp"])), self.payload_bytes())
        _write(self.cfgfile, "{}")                              # 외부 기록자가 활성을 파괴했다
        rc, verdict, reason = self.seed()
        self.assertIn("recovery-preserved", reason, reason)
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertEqual(json.loads(_read_bytes(os.path.join(self.cfg, kept[0])))["user"], "ONLY-COPY")
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [], "처분이 끝났는데 저널이 남았다")
        self.assert_only_copy_survives("저널이 가리키는 임시본을 판정하지 못했다")

    def test_conv_the_same_crash_on_the_normal_path_leaves_no_conflict(self):
        """음성 대조: 활성이 그대로면(통상) 그 임시본은 **증명된 중복**이라 회수된다 — 보수화가 상시 잔재가 되지 않는다."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        self.crash_between_journal_and_rename()
        r = self.seed()
        self.assertIn("recovery-reclaimed", r[2], r[2])
        self.assertNotIn("recovery-preserved", r[2], r[2])
        for prefix in (pf.SEED_TRUST_CONFLICT_PREFIX, pf.SEED_TRUST_DISPLACED_PREFIX,
                       pf.SEED_TRUST_INTENT_PREFIX, pf.SEED_TRUST_TMP_PREFIX):
            self.assertEqual([n for n in self.names(prefix) if n != pf.SEED_TRUST_LOCK_NAME], [], prefix)
        self.assertEqual(json.loads(_read_bytes(self.cfgfile))["user"], "ONLY-COPY")
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return          # ★R6 축: 교환 기구가 없으면 커밋은 REFUSE 다 — 회수·잔재 계약은 위에서 이미 단언했다
        self.assertEqual(r[0], 0, r)

    def test_conv_guard_ignores_a_copy_that_a_journal_still_claims(self):
        """codex 설계비평 4: 회수 ⓑ 가 정리에 실패하면 **저널을 남긴다** — 그 사본은 고아가 아니므로 가드가 격리·거부하지
        않는다(종전 설계였다면 저널이 남은 사본까지 고아로 잡아 애먼 conflict·REFUSE 를 냈다)."""
        self.untrusted_file(self.ORIGINAL)
        with patch.object(pf, "_exchange_paths", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()
        displaced = self.names(pf.SEED_TRUST_DISPLACED_PREFIX)
        self.assertEqual(len(displaced), 1, os.listdir(self.cfg))
        real_unlink = os.unlink

        def blocked(path, *a, **k):
            if os.path.basename(path).startswith(pf.SEED_TRUST_DISPLACED_PREFIX):
                raise OSError(errno.EACCES, "injected")
            return real_unlink(path, *a, **k)

        note = []
        with patch.object(pf.os, "unlink", blocked):
            self.assertIsNone(pf._recover_interrupted_seed(self.cfg, self.cfgfile, note))
        self.assertEqual(note, [], "회수가 실패했는데 회수했다고 사유에 적었다")
        self.assertEqual(len(self.names(pf.SEED_TRUST_INTENT_PREFIX)), 1, "정리 실패인데 저널을 지웠다")
        self.assertIsNone(pf._orphan_recovery_guard(self.cfg, self.cfgfile),
                          "저널이 책임지는 사본을 고아로 잡았다")
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [], "저널이 있는 사본을 격리했다")
        self.assertEqual(self.names(pf.SEED_TRUST_DISPLACED_PREFIX), displaced)

    def test_conv_release_keeps_the_journal_when_preserving_fails(self):
        """codex 설계비평 2: 보존 rename 이 막히면 사본은 청소 네임스페이스에 그대로 남는다 — 저널까지 지우면 다음
        실행이 두 지문을 잃고 고아 경로로 떨어진다. 저널을 남기고 그 사실을 사유에 적는다."""
        self.untrusted_file(self.ORIGINAL)

        def replace_then_fail(a, b):
            _write(b, "{}")
            raise OSError(errno.EIO, "injected")

        with patch.object(pf, "_exchange_paths", replace_then_fail), \
                patch.object(pf, "_preserve_copy", return_value=None):
            rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("사본 보존 실패", reason)
        self.assertEqual(len(self.names(pf.SEED_TRUST_INTENT_PREFIX)), 1, "보존에 실패했는데 저널을 지웠다")
        self.assertEqual(len(self.names(pf.SEED_TRUST_DISPLACED_PREFIX)), 1, os.listdir(self.cfg))
        self.assert_only_copy_survives("보존 실패 경로가 사본을 지웠다")

    def test_conv_exchange_success_preserves_the_old_original_when_the_active_is_replaced(self):
        """codex 설계비평 8(후반): 교환은 성립했는데 **그 직후** 외부가 활성을 `{}` 로 바꾸면 displaced 의 옛 원본이
        유일한 사본이다 — 폐기도 바이트 증명 아래에 둔다(종전엔 무조건 unlink 였다)."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        real_ex = pf._exchange_paths

        def exchange_then_replace(a, b):
            r = real_ex(a, b)
            if r is True:
                _write(b, "{}")                                 # 교환 직후 끼어든 파괴자
            return r

        with patch.object(pf, "_exchange_paths", exchange_then_replace):
            r = self.seed()
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return                                              # 교환 기구가 없으면 이 창 자체가 열리지 않는다
        self.assertEqual(r[:2], (2, "REFUSE"), r)               # 되읽기가 플래그 없는 문서를 본다(post-commit)
        self.assertIn("displaced-preserved", r[2], r[2])
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertEqual(json.loads(_read_bytes(os.path.join(self.cfg, kept[0])))["user"], "ONLY-COPY")
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [], "처분이 끝났는데 저널이 남았다")
        self.assert_only_copy_survives("교환 직후 파괴자가 유일한 원본을 지우게 했다")

    def test_conv_exchange_success_on_the_normal_path_leaves_no_conflict(self):
        """음성 대조: 아무도 끼어들지 않으면 활성 = 우리 payload 라 증명이 성립한다 — 옛 원본은 종전대로 폐기되고
        conflict 잔재 0(보수화가 통상 경로의 잔재를 만들지 않는다)."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        r = self.seed()
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return
        self.assertEqual(r[0], 0, r)
        self.assertIn("commit=exchange", r[2])
        self.assertNotIn("displaced-preserved", r[2], r[2])
        for prefix in (pf.SEED_TRUST_CONFLICT_PREFIX, pf.SEED_TRUST_DISPLACED_PREFIX, pf.SEED_TRUST_INTENT_PREFIX):
            self.assertEqual(self.names(prefix), [], prefix)
        self.assertEqual(json.loads(_read_bytes(self.cfgfile))["user"], "ONLY-COPY")

    def test_conv_flag_only_litter_is_still_swept(self):
        """격리 위양성 0: 부재 dir 경로가 만드는 **최소 문서**(플래그뿐)는 지킬 데이터가 없다 — 활성이 없어도 청소된다.
        (여기까지 격리하면 평범한 크래시 잔재가 영구 conflict 파일이 된다 · codex 설계비평 3)"""
        minimal = json.dumps({"projects": {self.key: {"hasTrustDialogAccepted": True}}},
                             ensure_ascii=False, indent=2).encode("utf-8")
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34"), minimal)
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd35"),
                     b'{"v": 1, "displaced": "x"}')                 # 저널 조각도 지킬 데이터 0
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertIn("stale-tmp swept 2", reason)
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [], "지킬 데이터가 없는 잔재를 격리했다")

    def crash_before_journal(self):
        """SIGKILL 모사 — 저널 **공개 전** 사망(triage 가 '더 넓다' 고 적은 ⓑ 창: mkstemp 기록~저널 공개).
        `finally` 가 돌지 않으므로 mkstemp payload(= 원본 + 플래그)가 그대로 남고 **활성은 무접촉**이다 —
        즉 이 창의 잔재는 정의상 활성과 바이트가 다르고, 잃을 것은 하나도 없다."""
        with patch.object(pf, "_write_seed_intent", side_effect=KeyboardInterrupt), \
                patch.object(pf, "_unlink_quiet", lambda path: None):
            with self.assertRaises(KeyboardInterrupt):
                self.seed()

    def test_conv_crash_before_the_journal_leaves_no_conflict_on_the_normal_path(self):
        """★수렴 R2(최종 리뷰 claude major) — 음성 대조가 비어 있던 ⓑ 창.
        저널 공개 전 사망이 남기는 잔재는 언제나 `원본 + 플래그`이고 그때 활성은 `원본` 그대로다 → 활성 대조
        (`_active_digest == _payload_digest`)는 **정의상** 성립하지 않는다. 종전 규칙은 그래서 원본이 사용자 문서이기만
        하면 **잃을 것이 하나도 없는 통상 경로**에서 격리를 돌렸다(실측 2026-09-08: 부트 1회에 conflict 1개 · 그 내용이
        최종 활성과 바이트 동일한 순수 쓰레기인데 사유는 '사람이 병합').
        '잔재 == 지금 다시 계획한 payload' 증명이 그 자리를 덮는다: conflict 0 · 잔재 0 · 활성 무손실."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        self.crash_before_journal()
        litter = [n for n in self.names(pf.SEED_TRUST_TMP_PREFIX) if n != pf.SEED_TRUST_LOCK_NAME]
        self.assertEqual(len(litter), 1, os.listdir(self.cfg))              # 창이 실제로 재현됐다
        self.assertEqual(_read_bytes(os.path.join(self.cfg, litter[0])), self.payload_bytes())
        self.assertEqual(_read_bytes(self.cfgfile), before, "활성 무접촉 창이어야 한다(전제)")
        self.assertEqual(self.names(pf.SEED_TRUST_INTENT_PREFIX), [], "저널 공개 전 창이어야 한다(전제)")
        r = self.seed()
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [],
                         "잃을 것이 없는 잔재를 영구 conflict 로 격리했다: %s" % sorted(os.listdir(self.cfg)))
        self.assertIn("stale-tmp swept 1", r[2], r[2])
        self.assertNotIn("stale-tmp preserved", r[2], r[2])
        self.assertEqual([n for n in self.names(pf.SEED_TRUST_TMP_PREFIX) if n != pf.SEED_TRUST_LOCK_NAME], [])
        self.assertEqual(json.loads(_read_bytes(self.cfgfile))["user"], "ONLY-COPY")
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return                      # ★R6 축: 교환 기구가 없어도 청소 계약은 위에서 이미 단언했다
        self.assertEqual(r[0], 0, r)

    def test_conv_the_pre_journal_crash_does_not_accumulate_conflicts(self):
        """상한 없는 누적의 회귀 핀(같은 major) — 교환 기구가 **상시 부재**면(Windows) 플래그가 영영 커밋되지 않아
        `changed` 가 매 부트 참이라 그 창이 좌석마다 다시 열린다. 실측(수정 전): conflict 0→1→2→3→4→5 · 총 0→875B ·
        상한 없음(어느 자동 경로도 conflict 네임스페이스를 지우지 않는다 · 실 `.claude.json` 은 수 MB 도 흔하다).
        수정 뒤에는 몇 번을 돌아도 conflict 0 이고 활성 바이트는 불변이다."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        unavailable = lambda a, b: pf._ExchangeUnavailable("platform:nt")   # noqa: E731 — 축 강제(플랫폼 무관 재현)
        for boot in range(3):
            with patch.object(pf, "_exchange_paths", unavailable):
                self.crash_before_journal()
                r = self.seed()
            self.assertEqual(r[:2], (2, "REFUSE"), r)
            self.assertIn("exchange-unavailable", r[2])
            self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [],
                             "부트 %d: 무손실 크래시 잔재가 conflict 로 쌓였다 %s" % (boot + 1, sorted(os.listdir(self.cfg))))
            self.assertEqual([n for n in self.names(pf.SEED_TRUST_TMP_PREFIX) if n != pf.SEED_TRUST_LOCK_NAME], [],
                             "부트 %d: 잔재가 남았다" % (boot + 1))
            self.assertEqual(_read_bytes(self.cfgfile), before, "거부 축인데 활성 바이트가 바뀌었다")

    def test_conv_a_litter_from_another_cwd_is_dropped_only_when_lossless(self):
        """★성찰 P12 재핀(major · 2026-09-10 · **의도적 방향 변경**). 종전 핀은 '다른 cwd 의 잔재는 바이트가 다르니
        무조건 격리' 였는데, 그 축(바이트)에서는 **잃을 것이 하나도 없는** 사본이 좌석 수만큼 매 부트 conflict 로 쌓였다
        (`changed` 가 매 부트 참이라 그 창이 좌석마다 다시 열린다 · 어느 자동 경로도 conflict 를 지우지 않는다).
        축을 **구조**로 올린다: 시더가 스스로 다시 쓰는 `true` 플래그 차이만 흡수하고, 활성이 그 사본의 비재생성
        데이터를 **전부** 담을 때만 지운다. 그래서 두 방향을 한 자리에서 잰다 —
          ⓐ 다른 cwd 의 잔재라도 사용자 데이터가 활성에 그대로 있으면 지운다(누적 0)
          ⓑ 활성이 **담지 않은** 사용자 데이터가 그 잔재에 있으면 종전대로 격리한다(증명이 없으면 보존)."""
        self.untrusted_file(self.ORIGINAL)
        other, _c, _k = pf.trust_plan(json.loads(self.ORIGINAL), "/some/other/cwd")
        foreign = json.dumps(other, ensure_ascii=False, indent=2).encode("utf-8")
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34"), foreign)
        r = self.seed()
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [],
                         "무손실 잔재가 conflict 로 쌓였다 %s" % sorted(os.listdir(self.cfg)))
        self.assertNotIn("stale-tmp preserved", r[2], r[2])
        self.assertEqual(json.loads(_read_bytes(self.cfgfile))["user"], "ONLY-COPY", "활성이 사용자 데이터를 잃었다")
        # ⓑ 음성 대조: 활성에 없는 사용자 필드(mcpServers)를 담은 잔재는 증명이 서지 않는다 → 격리
        richer = json.loads(self.ORIGINAL)
        richer["mcpServers"] = {"x": {"command": "y"}}
        other2, _c2, _k2 = pf.trust_plan(richer, "/some/other/cwd")
        foreign2 = json.dumps(other2, ensure_ascii=False, indent=2).encode("utf-8")
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "cd34ab12"), foreign2)
        r2 = self.seed()
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertIn("stale-tmp preserved", r2[2], r2[2])
        self.assertEqual(_read_bytes(os.path.join(self.cfg, kept[0])), foreign2)

    def test_conv_flag_only_scope_is_pinned_and_a_declined_flag_is_kept(self):
        """★수렴 R2(최종 리뷰 claude minor) — '플래그뿐 문서' 통의 **실제 범위** 검체(문면을 술어에 맞춘 뒤의 고지).
        ① 다른 폴더의 신뢰 플래그만 든 다중 키 문서도 그 통에 든다 = 청소된다 — 근거는 '작다' 가 아니라 **재생성 가능**
           (그 좌석의 다음 부트가 그 자리에서 다시 심는다). 종전 독스트링은 이 통을 '부재 dir 의 최소 문서' 로만 설명했다.
        ② 값이 정확히 `true` 가 **아닌** 플래그(`false` = 사람이 관문에서 거절한 결정)는 시더가 쓰지 않는 값이므로 지킨다."""
        multi = json.dumps({"projects": {"/some/other/folder": {"hasTrustDialogAccepted": True},
                                         "/new": {"hasTrustDialogAccepted": True}}},
                           ensure_ascii=False, indent=2).encode("utf-8")
        declined = b'{"projects": {"/o": {"hasTrustDialogAccepted": false}}}'
        self.assertFalse(pf._document_carries_user_data(multi))
        self.assertTrue(pf._document_carries_user_data(declined))
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34"), multi)
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd35"), declined)
        rc, verdict, reason = self.seed()
        self.assertEqual(rc, 0, reason)
        self.assertIn("stale-tmp swept 1", reason)                 # 다중 키 플래그 문서만 청소
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertIn("stale-tmp preserved", reason)
        self.assertEqual(_read_bytes(os.path.join(self.cfg, kept[0])), declined)

    def test_conv_sweep_reason_survives_a_read_plan_error(self):
        """★수렴 R2(최종 리뷰 claude minor): 청소가 **격리(conflict 이동)** 를 해 놓고 `_read_plan()` 오류로 빠져나가면서
        사유를 통째로 버렸다 — 디스크는 바뀌었는데 '사람이 병합해야 할 파일이 생겼다' 가 침묵하는 파일시스템 변경이다."""
        self.untrusted_file("{broken")                              # 활성 손상 → _read_plan 이 ERROR
        _write_bytes(os.path.join(self.cfg, pf.SEED_TRUST_TMP_PREFIX + "ab12cd34"),
                     b'{"user": "ONLY-COPY", "projects": {"/o": {"hasTrustDialogAccepted": true}}}')
        rc, verdict, reason = self.seed()
        self.assertEqual((rc, verdict), (1, "ERROR"), reason)
        self.assertIn("파싱 실패", reason)
        kept = self.names(pf.SEED_TRUST_CONFLICT_PREFIX)
        self.assertEqual(len(kept), 1, os.listdir(self.cfg))
        self.assertIn("stale-tmp preserved", reason)
        self.assertIn(kept[0], reason, "격리한 파일 이름을 사유에 적지 않았다")
        self.assert_only_copy_survives("손상 활성 경로가 사본을 지웠다")

    def test_conv_exchange_success_reports_a_failed_preserve_as_failed(self):
        """★수렴 R2(최종 리뷰 claude minor): `_preserve_copy` 가 실패하면 사본은 conflict 네임스페이스로 **옮겨지지
        않았다** — 그런데 문면은 `displaced-preserved(...)` 로 성공을 단언했다(이름 모양으로만 실패를 유추).
        `_orphan_recovery_guard`(moved/stuck) · `_sweep_stale_seed_tmp`(preserved/preserve-failed) ·
        `_release_uncommitted_copy`('사본 보존'/'사본 보존 실패')와 같은 규율로 가른다."""
        self.untrusted_file(self.ORIGINAL)
        before = _read_bytes(self.cfgfile)
        real_ex = pf._exchange_paths

        def exchange_then_replace(a, b):
            r = real_ex(a, b)
            if r is True:
                _write(b, "{}")                                     # 교환 직후 끼어든 파괴자
            return r

        with patch.object(pf, "_exchange_paths", exchange_then_replace), \
                patch.object(pf, "_preserve_copy", return_value=None):
            r = self.seed()
        if _refused_without_exchange(self, r, self.cfgfile, before):
            return                                                  # 교환 기구가 없으면 이 창 자체가 열리지 않는다
        self.assertIn("displaced-preserve-failed", r[2], r[2])
        self.assertNotIn("displaced-preserved(", r[2], r[2])
        self.assertEqual(len(self.names(pf.SEED_TRUST_INTENT_PREFIX)), 1, "보존에 실패했는데 저널을 지웠다")
        self.assertEqual(len(self.names(pf.SEED_TRUST_DISPLACED_PREFIX)), 1, os.listdir(self.cfg))
        self.assertEqual(self.names(pf.SEED_TRUST_CONFLICT_PREFIX), [], "실패인데 conflict 사본이 생겼다")
        self.assert_only_copy_survives("보존 실패 경로가 옛 원본을 지웠다")

    def test_conv_journal_tmp_field_is_validated(self):
        """codex 설계비평 5: `tmp` 는 **mkstemp 이름 형식의 basename** 만 — 경로 탈출·타 네임스페이스·개행 꼬리는 형식
        위반(판독 None → 호출자는 REFUSE) · 필드 **부재**는 구판 저널로 유효하다."""
        base = {"v": 1, "displaced": pf.SEED_TRUST_DISPLACED_PREFIX + "a" * 64 + "-20260907T000000Z-1",
                "captured_sha256": "b" * 64, "payload_sha256": "c" * 64}
        path = os.path.join(self.cfg, pf.SEED_TRUST_INTENT_PREFIX + "probe")
        for tmp, ok in ((None, True), (".claude.json.seed-ab12cd34", True), ("../victim", False),
                        ("/etc/passwd", False), (".claude.json.seed-lock", False),
                        (".claude.json.seed-abcdefgh\n", False), (".claude.json.displaced-x", False),
                        (".claude.json.seed-short", False), (12, False), (None if False else "", False)):
            with self.subTest(tmp=tmp):
                rec = dict(base)
                if tmp is not None:
                    rec["tmp"] = tmp
                _write_bytes(path, json.dumps(rec, ensure_ascii=True).encode("utf-8"))
                got = pf._read_seed_intent(path)
                self.assertEqual(got is not None, ok, (tmp, got))


if __name__ == "__main__":
    unittest.main(verbosity=2)
