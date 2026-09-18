#!/usr/bin/env python3
"""test_dept_name_guard.py — cys-dept 이름 검증·A11 dedupe·승격 영수증·demote 경보 핀 (v4 스펙 §D3(i)·A11·A6·G0 D3/D10).

`launch --help` 실사고(문자 이름 '--help' 유령 부서 등재→껍데기 CEO 승격) 봉합의 회귀 핀.
격리 HOME + 목 cys/cysd($HOME/.local/bin — cys-dept PATH prepend 1순위)로 실 데몬 무접촉:
  1) 수용/거부표 — 화이트리스트 ^[A-Za-z0-9][A-Za-z0-9_-]*$·≤40자. 경계: 빈·'-'선두·'.'포함·
     41자·한글·공백·'/'. 거부=exit 2+부작용 0(state/pack 디렉토리·레지스트리 등재 부재).
  2) cmd 위치 --help/-h = usage(stdout·exit 0·부작용 0) / name 위치 --help = exit 2·부작용 0.
  3) create — key 화이트리스트(카탈로그 존재 검사보다 선행)·3분기 공통 관문(REUSE 비정형 등재
     exit 2+자동 삭제 금지)·NEW 정상 경로 무회귀(stdout 마지막 줄=name).
  4) rotate — 등재된 비정형명은 kill 이전 exit 2(cys 호출 0·소켓 불변 = graceful_kill 미도달).
     ※ kill은 bash builtin이라 PATH 목 로깅 불가 — '검증이 ping/identify(kill 선행 단계)보다
     앞에서 끊는다'를 cys 호출 0+소켓 잔존으로 단언(동등 증거).
  5) CYS_DEPT_ROTATE=1이어도 launch 검증 유지.
  6) passthrough — 실존(비정형 포함) 통과 / 비실존·정형 통과+CYS_NO_AUTOSTART=1(G0 D10) /
     비실존·비정형 exit 2. sock 동사 '-' 선두 exit 2(레거시 비정형 등재명은 fan-out 호환 통과).
  7) 인자 위생 — cys tombstone 호출이 `--dept [--remove] -- <name>`(clap `--` 종단) 형식.
  8) 기존명 재-launch 통과(slug-fold 자기 제외) / slug 충돌쌍(대소·'.'-fold) 거부.
  9) 승격 영수증 — promote 시 directives/.ceo-template-applied=적용 템플릿 sha256, 강등 시 삭제.
 10) demote 무음 경보 — 승격 표지+.pre-ceo 부재='강등 불능' stderr 경보+feed push(비대기),
     미승격 머신은 무경보(위경보 금지).
 11) A11 — promote-if-pending --request-only가 미해결 동종(제목 'CEO 승격 대기') pending 존재 시
     push 생략(로그 1줄), 부재 시 발행.
 12) K2-07(2026-09-17 한글 사용자명 감사 · 2라운드) — unix 소켓 sun_path 초과는 **cysd 의 bind 가 판정**하고
     cys-dept 는 "데몬 기동 실패" 에 사유(sock_len_diag)만 덧붙인다. 스폰 전 exit 2 가드는 이 하네스
     (격리 HOME=mkdtemp → macOS /var/folders 50B 라 1글자 부서명도 105B · 목 cysd 는 bind 안 함)를
     깨뜨렸던 결함 — mkdtemp HOME 이 상한을 넘어도 목 cysd 로 launch 가 완주한다(회귀 표본).
 13) CRLF 위생 census — `read < <(python3 …)` 파이프 소비자 전부 `| tr -d '\r'`(cys-dept reg_names 규칙).
 14) K2-03 범위 일치 — depts.json 판독 전 지점 utf-8-sig(BOM 레지스트리가 RMW 에서 비워지지 않는다).
     P1(2026-09-17 부트체인 감사): 비UTF-8·손상 JSON 은 exit 10·원본 보존(빈 등재 복구·CEO 오강등 금지).
 15) R6/S1 — 스폰 경쟁 패자의 사망 뒤 rc 10 회수에서도 살아 있는 승자의 소켓/lock·목 ping 응답 보존.
"""
import ast
import hashlib
import json
import os
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import unicodedata

# ★T10(DCE-3) 픽스처 계약: CEO 템플릿 = MASTER 전문의 상위집합(합성 계약 동형) — 스왑 직전
#   런타임 상위집합 검사를 통과해야 승격 계열 테스트가 승격 상태에 도달한다.
CEO_BODY = "CEO-HEADER\n---\nSTANDARD-MASTER\n"
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")


def startup_lock(sock):
    """cysd 의 startup-lock 경로 — Rust 는 `socket_path.with_extension("lock")` 로 만든다
    (src/bin/cysd/main.rs:1132·1137). 즉 `…/cys-dept-<n>/cys.sock` 의 락은 `cys.sock.lock` 이 아니라
    **`cys.lock`** 이다. 검체가 실재하지 않는 파일을 지키면 '보존됐다' 는 단언이 공허해진다
    (2026-09-17 8라운드 · codex 4차 minor)."""
    return os.path.splitext(sock)[0] + ".lock"


def _write_exec(path, content):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    os.chmod(path, 0o755)


def make_home(tmp):
    """격리 HOME + 목 cys/cysd. cys ping은 CYS_SOCKET 파일 실존으로 생사 재현(죽은 소켓=exit 1 —
    allocate/create lowest-unused 루프가 '모든 소켓 생존' 목에서 무한루프하는 픽스처 결함 방지)."""
    home = os.path.join(tmp, "home")
    bindir = os.path.join(home, ".local", "bin")
    os.makedirs(bindir, exist_ok=True)
    os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
    log = os.path.join(tmp, "calls.log")
    feedlist = os.path.join(tmp, "feed-list.txt")
    _write_exec(os.path.join(bindir, "cys"),
                '#!/bin/sh\n'
                'echo "cys $@" >> "%(log)s"\n'
                'case "$1" in\n'
                '  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;\n'
                '  status) exit 1 ;;\n'
                '  identify) exit 1 ;;\n'
                '  feed) if [ "$2" = "list" ]; then [ -f "%(fl)s" ] && cat "%(fl)s"; exit 0; fi; exit 0 ;;\n'
                '  list) exit 0 ;;\n'
                'esac\nexit 0\n' % {"log": log, "fl": feedlist})
    # 목 cysd: 소켓 파일 생성 후 즉시 종료(ready()의 ping 파일-실존 프로브와 정합).
    _write_exec(os.path.join(bindir, "cysd"),
                '#!/bin/sh\nmkdir -p "$(dirname "$CYS_SOCKET")"\ntouch "$CYS_SOCKET"\nexit 0\n')
    return home, log, feedlist


def make_env(home):
    env = dict(os.environ)
    env.update({"HOME": home,
                "CYS_DEPTS_JSON": os.path.join(home, ".cys", "depts.json"),
                "PATH": os.path.join(home, ".local", "bin") + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_NO_AUTOSTART",
              "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG", "CYS_DEPT_DEFAULT_ACCOUNT",
              "CYS_PRIMARY_ACCOUNT"):
        env.pop(k, None)
    return env


def write_reg(env, depts):
    with open(env["CYS_DEPTS_JSON"], "w", encoding="utf-8") as f:
        json.dump({"depts": depts}, f, ensure_ascii=False)


def read_reg(env):
    try:
        return json.load(open(env["CYS_DEPTS_JSON"], encoding="utf-8")).get("depts", {})
    except (OSError, ValueError):
        return {}


def seed_sock(home, name):
    """부서 소켓 파일 시드 — 목 ping이 '가동 중(재사용)'으로 판정(launch가 cysd spawn 없이 완주)."""
    d = os.path.join(home, ".local", "state", "cys-dept-%s" % name)
    os.makedirs(d, exist_ok=True)
    sock = os.path.join(d, "cys.sock")
    open(sock, "w").close()
    return sock


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="deptguard-")
        self.home, self.log, self.feedlist = make_home(self.tmp)
        self.env = make_env(self.home)

    def run_dept(self, *args, env=None):
        r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                           encoding="utf-8", env=env or self.env, timeout=60)
        return r.returncode, r.stdout, r.stderr

    def calls(self):
        return open(self.log, encoding="utf-8").read() if os.path.exists(self.log) else ""

    def assert_no_side_effects(self, name):
        self.assertFalse(os.path.exists(os.path.join(
            self.home, ".local", "state", "cys-dept-%s" % name)),
            "거부됐는데 state 디렉토리 생성(%s)" % name)
        self.assertFalse(os.path.exists(os.path.join(
            self.home, ".cys", "pack-dept-%s" % name)),
            "거부됐는데 pack 디렉토리 생성(%s)" % name)
        self.assertNotIn(name, read_reg(self.env), "거부됐는데 레지스트리 등재(%s)" % name)

    def _seed_promotable(self, ndepts=1):
        """승격 가능 상태 시드: 디렉티브 쌍 + 부트 마커 + 부서 n개(승격·강등·A11 계열 공용).
        ★T10(DCE-3): CEO 템플릿은 합성 계약(머리글+구분선+MASTER 전문 verbatim)과 동형인
        **상위집합**이어야 스왑 직전 런타임 검사를 통과한다(스텁 픽스처는 보류가 정답 —
        test_ceo_pending_gate #7이 그 축을 핀)."""
        pack = os.path.join(self.home, ".cys", "pack", "directives")
        os.makedirs(pack, exist_ok=True)
        with open(os.path.join(pack, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
            f.write("STANDARD-MASTER\n")
        with open(os.path.join(pack, "CEO_TEMPLATE.md"), "w", encoding="utf-8") as f:
            f.write(CEO_BODY)
        with open(os.path.join(self.home, ".cys", ".master-bootstrapped"), "w") as f:
            f.write("{}")
        write_reg(self.env, {("d%d" % i): {"socket": "", "pack_dir": ""}
                             for i in range(ndepts)})
        return pack

    def _receipt(self):
        return os.path.join(self.home, ".cys", "pack", "directives", ".ceo-template-applied")


class AcceptRejectTable(Base):
    # 1) 거부표: 경계 이름 전부 exit 2 + 부작용 0 (launch = 부작용-이전 검증의 대표 생성 동사)
    def test_reject_table_exit2_no_side_effects(self):
        for bad in ["", "-x", "--help", "-h", "a.b", ".x", "a" * 41, "한글부서", "a b", "a/b", "a..b"]:
            rc, out, err = self.run_dept("launch", bad)
            self.assertEqual(rc, 2, "launch %r: exit=%d(≠2)\n%s" % (bad, rc, err))
            self.assertIn("usage: cys-dept launch", err,
                          "launch %r: 거부 진단에 동사 usage 1줄 부재" % bad)
            if bad and "/" not in bad:
                self.assert_no_side_effects(bad)
        # 레지스트리 파일 자체가 안 생겼어야 한다(검증이 reg_init보다 앞 = 부작용-이전)
        self.assertFalse(os.path.exists(self.env["CYS_DEPTS_JSON"]),
                         "거부만 했는데 depts.json 생성(검증이 부작용 이전이 아님)")

    # 1) 수용표: 정형 이름은 launch 완주(소켓 시드=재사용 경로·exit 0)
    def test_accept_table_launch_ok(self):
        write_reg(self.env, {})
        for good in ["a", "A-1_b", "x" * 40, "dept-1", "Z9"]:
            seed_sock(self.home, good)
            rc, out, err = self.run_dept("launch", good)
            self.assertEqual(rc, 0, "launch %r: exit=%d(≠0)\n%s%s" % (good, rc, out, err))
            self.assertIn(good, read_reg(self.env), "수용 이름 미등재(%s)" % good)


class HelpContract(Base):
    # 2) cmd 위치 --help/-h = usage(stdout)·exit 0·부작용 0 — 단일소유 가드(exit 7)보다 앞(역할 무관)
    def test_cmd_help_usage_stdout_exit0(self):
        for flag in ("--help", "-h"):
            rc, out, err = self.run_dept(flag)
            self.assertEqual(rc, 0, "%s: exit=%d(≠0)" % (flag, rc))
            self.assertIn("usage: cys-dept", out, "%s: usage가 stdout이 아님" % flag)
        # 역할 무관(가드 이전): CYS_ROLE=master여도 usage exit 0
        env = dict(self.env); env["CYS_ROLE"] = "master"
        rc, out, _ = self.run_dept("--help", env=env)
        self.assertEqual(rc, 0, "role=master cmd --help가 가드에 걸림(exit=%d)" % rc)
        self.assertIn("usage: cys-dept", out)
        self.assertFalse(os.path.exists(self.env["CYS_DEPTS_JSON"]), "--help가 부작용 발생")

    # 2) name 위치 --help = 검증 거부 exit 2 (usage 아님 — 실사고 재발 차단 핵심 핀)
    def test_name_position_help_rejected(self):
        rc, out, err = self.run_dept("launch", "--help")
        self.assertEqual(rc, 2)
        self.assertNotIn("usage: cys-dept <verb>", out, "name 위치 --help가 전체 usage로 오응답")
        self.assert_no_side_effects("--help")


class CreateGate(Base):
    def _seed_catalog(self, key, mkey):
        acct = os.path.join(self.home, "acct")
        os.makedirs(acct, exist_ok=True)
        cat = os.path.join(self.home, ".cys", "dept-catalog.json")
        with open(cat, "w", encoding="utf-8") as f:
            json.dump({"accounts": {"test": acct},
                       "departments": {key: {"display": "테스트부", "account": "test",
                                             "mission_key": mkey, "cwd": self.home}}},
                      f, ensure_ascii=False)
        # seed_agents_account 소스(메인 팩 agents.json — env 맵 구조)
        pack = os.path.join(self.home, ".cys", "pack")
        os.makedirs(pack, exist_ok=True)
        with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
            json.dump({"claude": {"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": "/base"}}}, f)

    # 3) key 화이트리스트 — 카탈로그 존재 검사(exit 3)보다 선행: 부적격 key는 카탈로그 없이도 exit 2
    def test_create_key_whitelist_before_catalog(self):
        for bad in ("bad.key", "--help", ""):
            rc, out, err = self.run_dept("create", bad)
            self.assertEqual(rc, 2, "create %r: exit=%d(≠2)\n%s" % (bad, rc, err))
            self.assertIn("usage: cys-dept create", err)

    # 3) 3분기 공통 관문: REUSE(mission_key 매칭 기존 엔트리)가 비정형이면 exit 2·자동 삭제 금지
    def test_create_reuse_nonconforming_reported_not_deleted(self):
        self._seed_catalog("k1", "m1")
        write_reg(self.env, {"we.ird": {"socket": os.path.join(self.home, "nosock"),
                                        "pack_dir": "", "mission_key": "m1",
                                        "reserved_at": time.time()}})
        rc, out, err = self.run_dept("create", "k1")
        self.assertEqual(rc, 2, "REUSE 비정형 등재인데 exit=%d(≠2)\n%s%s" % (rc, out, err))
        self.assertIn("비정형 등재", err, "비정형 등재 stderr 보고 부재")
        self.assertIn("we.ird", read_reg(self.env), "자동 삭제 금지 위반 — REUSE 엔트리 소실")

    # 3) NEW 정상 경로 무회귀: 관문이 정상 create를 막지 않는다(stdout 마지막 줄=name)
    def test_create_new_happy_path(self):
        self._seed_catalog("k2", "m2")
        write_reg(self.env, {})
        rc, out, err = self.run_dept("create", "k2")
        self.assertEqual(rc, 0, "정상 create 실패: exit=%d\n%s%s" % (rc, out, err))
        self.assertEqual(out.strip().splitlines()[-1], "dept-1", "stdout 마지막 줄=name 계약 위반")
        self.assertIn("dept-1", read_reg(self.env))


class RotateGuard(Base):
    # 4) 등재된 비정형명 rotate: kill 이전 exit 2 — cys 호출 0(ping/identify는 kill 선행 단계)·소켓 불변
    def test_rotate_precheck_before_kill(self):
        sock = seed_sock(self.home, "we.ird")
        write_reg(self.env, {"we.ird": {"socket": sock, "pack_dir": ""}})
        rc, out, err = self.run_dept("rotate", "we.ird")
        self.assertEqual(rc, 2, "rotate 비정형 등재인데 exit=%d(≠2)\n%s" % (rc, err))
        self.assertIn("kill 미수행", err, "kill 미수행 안내 부재")
        self.assertIn("down-sock", err, "down-sock/수동 정리 안내 부재")
        self.assertNotIn("ping", self.calls(), "검증 이전에 데몬 프로브(ping) 발생 — kill 경로 진입 의심")
        self.assertTrue(os.path.exists(sock), "kill 이전 거부인데 소켓 소실(rm 도달 = half-op)")
        self.assertIn("we.ird", read_reg(self.env), "rotate 거부가 등재를 파괴")

    # 4) 미등재 rotate는 기존 계약 유지(등재 게이트 exit 8 — 검증은 게이트 직후)
    def test_rotate_unregistered_still_exit8(self):
        write_reg(self.env, {})
        rc, out, err = self.run_dept("rotate", "ghost")
        self.assertEqual(rc, 8, "미등재 rotate exit=%d(≠8 — 기존 부활 금지 계약 회귀)" % rc)

    # 5) CYS_DEPT_ROTATE=1(rotate 재귀 신호)이어도 launch 검증 유지
    def test_launch_validates_even_under_rotate_env(self):
        write_reg(self.env, {"we.ird": {"socket": "", "pack_dir": ""}})
        env = dict(self.env); env["CYS_DEPT_ROTATE"] = "1"
        rc, out, err = self.run_dept("launch", "we.ird", env=env)
        self.assertEqual(rc, 2, "CYS_DEPT_ROTATE=1에서 launch 검증 우회(exit=%d)" % rc)


class PassthroughArm(Base):
    # 6) 실존(등재) 이름은 비정형이라도 통과 — 기존 부서 컨텍스트 실행 불차단(정리 동사형 규칙)
    def test_existing_nonconforming_passes(self):
        write_reg(self.env, {"a.b": {"socket": "", "pack_dir": ""}})
        rc, out, err = self.run_dept(
            "a.b", "--", "sh", "-c", 'echo "NA=${CYS_NO_AUTOSTART:-unset}"')
        self.assertEqual(rc, 0, "실존 비정형 passthrough 차단됨\n%s" % err)
        self.assertIn("NA=unset", out, "실존 이름인데 CYS_NO_AUTOSTART 오동봉")

    # 6) 비실존·정형 = 통과 + CYS_NO_AUTOSTART=1 동반(G0 D10 — autostart 유령 생성 봉합)
    def test_nonexistent_conforming_gets_no_autostart(self):
        write_reg(self.env, {})
        rc, out, err = self.run_dept(
            "ghostx", "--", "sh", "-c", 'echo "NA=${CYS_NO_AUTOSTART:-unset}"')
        self.assertEqual(rc, 0, "비실존·정형 passthrough 차단됨\n%s" % err)
        self.assertIn("NA=1", out, "비실존·정형인데 CYS_NO_AUTOSTART=1 미동봉(D10)")

    # 6) 비실존+비정형 = exit 2 (verb 오타 흡수 → 유령 생성 사고 절반 봉합)
    def test_nonexistent_nonconforming_rejected(self):
        write_reg(self.env, {})
        rc, out, err = self.run_dept("no.pe", "--", "sh", "-c", "echo run")
        self.assertEqual(rc, 2, "비실존·비정형 passthrough 통과(exit=%d)" % rc)
        self.assertNotIn("run", out, "거부인데 명령 실행됨")

    # R4/T9: depts.json 손상은 doctor --fix 진입을 막을 근거가 아니다. 디스크 실존과
    # account_dir의 명시/agents.json 폴백을 각각 검증한다(EISDIR=권한/root 비의존 판독 불가).
    def test_unreadable_registry_existing_disk_repair_and_account_fallback(self):
        for unreadable in (False, True):
            reg = self.env["CYS_DEPTS_JSON"]
            if os.path.isfile(reg):
                os.unlink(reg)
            if unreadable:
                os.mkdir(reg)
            else:
                with open(reg, "wb") as f:
                    f.write(b'{"depts":')
            for disk in ("state", "pack"):
                with self.subTest(unreadable=unreadable, disk=disk):
                    name = "repair.%s.%s" % (disk, unreadable)
                    env = dict(self.env)
                    env.pop("CYS_ACCOUNT_DIR", None)
                    acct = os.path.join(self.home, "repair-account")
                    os.makedirs(acct, exist_ok=True)
                    pack = os.path.join(self.home, ".cys", "pack-dept-" + name)
                    state = os.path.join(self.home, ".local", "state", "cys-dept-" + name)
                    os.makedirs(state if disk == "state" else pack)
                    if disk == "state":
                        env["CYS_ACCOUNT_DIR"] = acct
                    else:
                        with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
                            json.dump({"claude": {"env": {"CLAUDE_CONFIG_DIR": acct}}}, f)
                    rc, out, err = self.run_dept(name, "--", "sh", "-c",
                        'printf "REPAIR=%s|%s|%s|%s\\n" "$CYS_SOCKET" "$CYS_PACK_DIR" '
                        '"$CYS_ACCOUNT_DIR" "${CYS_NO_AUTOSTART:-unset}"', env=env)
                    self.assertEqual(rc, 0, "기존 부서 수리 경로 차단\n" + err)
                    self.assertIn("REPAIR=%s|%s|%s|unset" % (
                        os.path.join(state, "cys.sock"), pack, acct), out)
                    self.assertEqual(self.calls(), "", "수리 진입에서 데몬 호출 발생")
            if unreadable:
                self.assertTrue(os.path.isdir(reg), "판독 불가 registry 경로 변조")
            else:
                with open(reg, "rb") as f:
                    self.assertEqual(f.read(), b'{"depts":', "수리 진입이 registry를 변경")

    def test_corrupt_registry_unknown_names_keep_validation_and_no_autostart(self):
        with open(self.env["CYS_DEPTS_JSON"], "wb") as f:
            f.write(b'{"depts":')
        rc, out, err = self.run_dept("unknown.bad", "--", "sh", "-c", "echo RAN")
        self.assertEqual(rc, 2, "손상 registry가 비실존·비정형 검증을 우회\n" + err)
        self.assertNotIn("RAN", out)
        rc, out, err = self.run_dept("unknown-good", "--", "sh", "-c",
                                    'echo "NA=${CYS_NO_AUTOSTART:-unset}"')
        self.assertEqual(rc, 0, err)
        self.assertIn("NA=1", out, "손상 registry에서 유령 autostart 차단 소실")
        self.assertEqual(self.calls(), "", "읽기 전용 컨텍스트가 데몬 호출")

    # 6) sock: '-' 선두 exit 2 / 레거시 비정형 등재명은 통과(fan-out 루프 `sock "$d"` 호환)
    def test_sock_partial_validation(self):
        rc, out, err = self.run_dept("sock", "--help")
        self.assertEqual(rc, 2, "sock --help가 경로를 오출력(exit=%d)" % rc)
        self.assertNotIn("cys-dept---help", out)
        rc, out, err = self.run_dept("sock", "a.b")
        self.assertEqual(rc, 0, "레거시 비정형 등재명 sock 회귀(fan-out 파손)")
        self.assertIn("cys-dept-a.b", out)


class ArgHygiene(Base):
    # 7) cys tombstone 인자 위생: down(set)·launch(remove) 모두 `--dept [--remove] -- <name>` 형식
    def test_tombstone_double_dash(self):
        sock = seed_sock(self.home, "d1")
        write_reg(self.env, {"d1": {"socket": sock, "pack_dir": ""}})
        rc, out, err = self.run_dept("down", "d1")
        self.assertEqual(rc, 0, "down 실패\n%s%s" % (out, err))
        self.assertIn("tombstone --dept -- d1", self.calls(),
                      "down의 데몬 묘비 set이 `--dept -- <name>` 형식이 아님")
        # launch 성공 말미의 묘비 해소(remove)도 동일 위생
        seed_sock(self.home, "d2")
        write_reg(self.env, {"d2": {"socket": "", "pack_dir": ""}})
        rc, out, err = self.run_dept("launch", "d2")
        self.assertEqual(rc, 0, "launch 실패\n%s%s" % (out, err))
        self.assertIn("tombstone --dept --remove -- d2", self.calls(),
                      "launch의 묘비 해소가 `--dept --remove -- <name>` 형식이 아님")


class SlugFold(Base):
    # 8) 기존명 재-launch 통과(자기 제외 — GUI 복원·rotate 재귀 무회귀 핀)
    def test_existing_name_relaunch_passes(self):
        seed_sock(self.home, "sales")
        write_reg(self.env, {"sales": {"socket": "", "pack_dir": ""}})
        rc, out, err = self.run_dept("launch", "sales")
        self.assertEqual(rc, 0, "기존명 재-launch가 거부됨(자기 제외 결여)\n%s" % err)

    # 8) slug 충돌쌍 거부: 대소(Sales↔sales)·'.'-fold(ab↔a.b — win pipe_slug 점 소거 정합)
    def test_slug_collision_pairs_rejected(self):
        write_reg(self.env, {"sales": {"socket": "", "pack_dir": ""},
                             "a.b": {"socket": "", "pack_dir": ""}})
        for newname, existing in (("Sales", "sales"), ("SALES", "sales"), ("ab", "a.b")):
            rc, out, err = self.run_dept("launch", newname)
            self.assertEqual(rc, 2, "launch %r: 충돌쌍(기존 %r) 미거부(exit=%d)"
                             % (newname, existing, rc))
            self.assertIn("slug 충돌", err)
            self.assertNotIn(newname, read_reg(self.env), "충돌 거부인데 등재됨(%s)" % newname)


class PromotionReceiptAndDemote(Base):
    # 9) 승격 영수증: _swap 후 적용 템플릿 sha256(hex 1줄) 기록 → 강등 시 삭제
    def test_receipt_written_and_deleted(self):
        pack = self._seed_promotable(ndepts=1)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, "promote-ceo 실패\n%s%s" % (out, err))
        expected = hashlib.sha256(
            open(os.path.join(pack, "CEO_TEMPLATE.md"), "rb").read()).hexdigest()
        self.assertTrue(os.path.exists(self._receipt()), "승격 영수증 미기록")
        self.assertEqual(open(self._receipt(), encoding="utf-8").read().strip(), expected,
                         "영수증 내용≠적용 템플릿 sha256")
        self.assertTrue(os.path.exists(os.path.join(pack, "MASTER_DIRECTIVE.md.pre-ceo")))
        # 마지막 부서 down → ceo_demote: 원복+영수증 삭제
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, "down 실패\n%s%s" % (out, err))
        self.assertEqual(open(os.path.join(pack, "MASTER_DIRECTIVE.md"),
                              encoding="utf-8").read(), "STANDARD-MASTER\n", "강등 원복 실패")
        self.assertFalse(os.path.exists(self._receipt()), "강등 후 영수증 잔존(stale)")

    # 10) demote 무음 경보: 승격 표지(md==템플릿)+.pre-ceo 부재 → stderr 경보+feed push(비대기)
    def test_demote_missing_backup_alerts(self):
        pack = self._seed_promotable(ndepts=1)
        with open(os.path.join(pack, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
            f.write(CEO_BODY)   # 승격 표지(md==템플릿) — .pre-ceo는 없음(비가역 상태)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, "경보 경로가 teardown을 파괴(exit=%d)" % rc)
        self.assertIn("강등 불능", err, "무음 no-op 잔존 — stderr 경보 부재")
        self.assertIn("feed push --title CEO 강등 불능", self.calls(), "feed push 경보 부재")

    # 10) 미승격 머신: .pre-ceo 부재는 정상 no-op — 위경보 금지
    def test_demote_unpromoted_no_false_alarm(self):
        pack = self._seed_promotable(ndepts=1)   # md=STANDARD(미승격)·pre-ceo 無
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0)
        self.assertNotIn("강등 불능", err, "미승격 머신에 위경보")
        self.assertNotIn("CEO 강등 불능", self.calls(), "미승격 머신에 feed 위경보")


class FeedDedupe(Base):
    def _pend_state(self):
        state = os.path.join(self.home, ".cys", "state")
        os.makedirs(state, exist_ok=True)
        with open(os.path.join(state, "ceo-pending"), "w") as f:
            f.write("pending\n")

    # 11) A11: 미해결 동종 pending 존재 → push 생략(로그 1줄) / 부재 → 발행
    def test_request_only_dedupe(self):
        self._seed_promotable(ndepts=1)
        self._pend_state()
        with open(self.feedlist, "w", encoding="utf-8") as f:
            f.write("id1\t[pending]\tgeneric\tCEO 승격 대기\tdecision=-\n")
        rc, out, err = self.run_dept("promote-if-pending", "--request-only")
        self.assertEqual(rc, 0, out + err)
        self.assertIn("재발행 생략", out, "dedupe 생략 로그 1줄 부재")
        self.assertNotIn("feed push --title CEO 승격 대기", self.calls(),
                         "동종 pending 존재인데 push 재발행(A11 위반)")
        # pending 항목 소거 → 발행 재개(과차단 금지)
        os.unlink(self.feedlist)
        open(self.log, "w").close()
        rc, out, err = self.run_dept("promote-if-pending", "--request-only")
        self.assertEqual(rc, 0, out + err)
        self.assertIn("알림 발행", out)
        self.assertIn("feed push --title CEO 승격 대기", self.calls(),
                      "동종 pending 부재인데 push 미발행(과차단)")

    # 11) 제목이 다른 pending(무관 항목)은 dedupe 비대상
    def test_request_only_unrelated_pending_not_deduped(self):
        self._seed_promotable(ndepts=1)
        self._pend_state()
        with open(self.feedlist, "w", encoding="utf-8") as f:
            f.write("id9\t[pending]\tgeneric\t다른 승인 요청\tdecision=-\n")
        rc, out, err = self.run_dept("promote-if-pending", "--request-only")
        self.assertEqual(rc, 0, out + err)
        self.assertIn("feed push --title CEO 승격 대기", self.calls(),
                      "무관 pending에 오-dedupe(제목 정합 검사 결여)")


class SockLenDiag(Base):
    """12) K2-07: sock_len_diag 는 진단 전용(항상 0 반환·스폰 전 거부 없음) — 상한은 OS 별(darwin/BSD 104 · Linux 108 ·
    NUL 포함 = Rust std `UnixListener::bind` 의 "path must be shorter than SUN_LEN"). 제품 함수 **그 자체**를 소스에서
    추출해 실행한다(javis_bootstrap self-test 의 dept_name_ok 소스 대조와 같은 방식 — 사본 금지)."""

    def _func_src(self):
        src = open(DEPT, encoding="utf-8").read()
        m = re.search(r"^sock_len_diag\(\)\{\n.*?^\}\n", src, re.S | re.M)
        self.assertIsNotNone(m, "cys-dept 에 sock_len_diag 정의 부재(K2-07 진단 소실)")
        return m.group(0)

    def _run_diag(self, path, uname):
        env = dict(self.env)
        bindir = os.path.join(self.tmp, "unamebin-" + uname.split("_")[0])
        os.makedirs(bindir, exist_ok=True)
        _write_exec(os.path.join(bindir, "uname"), "#!/bin/sh\necho %s\n" % uname)
        env["PATH"] = bindir + os.pathsep + env["PATH"]
        r = subprocess.run(["bash", "-c", self._func_src() + '\nsock_len_diag "$1"\n', "x", path],
                           capture_output=True, text=True, encoding="utf-8", env=env, timeout=30)
        return r.returncode, r.stdout, r.stderr

    @staticmethod
    def _path_of_bytes(n):
        head, tail = "/h/.local/state/cys-dept-", "/cys.sock"
        p = head + "a" * (n - len(head) - len(tail)) + tail
        assert len(p.encode("utf-8")) == n
        return p

    # 12) 순수 판정: 상한-1 무출력 / 상한 = 사유 1줄(바이트 수·상한 병기) · 두 OS · 항상 rc 0 · stdout 무오염
    def test_pure_threshold_per_os(self):
        for uname, lim in (("Darwin", 104), ("Linux", 108)):
            rc, out, err = self._run_diag(self._path_of_bytes(lim - 1), uname)
            self.assertEqual((rc, out, err), (0, "", ""), "%s: 상한-1 에서 오경보 (%r)" % (uname, err))
            rc, out, err = self._run_diag(self._path_of_bytes(lim), uname)
            self.assertEqual((rc, out), (0, ""), "%s: 진단이 rc/stdout 을 오염(rc=%d out=%r)" % (uname, rc, out))
            self.assertIn("sun_path", err, "%s: 상한 도달인데 사유 부재" % uname)
            self.assertIn("%dB ≥ %dB" % (lim, lim), err, "%s: 바이트 수·상한 병기 부재: %r" % (uname, err))

    # 12) 문자 수가 아니라 **바이트** 수: NFD 한글 홈 + 40자(계약 상한 안) = 106B → darwin 사유
    def test_nfd_home_counts_bytes_not_chars(self):
        nfd = unicodedata.normalize("NFD", "홍길동")
        self.assertNotEqual(nfd, "홍길동")   # 픽스처가 실제로 NFD 인지(핀 무효화 방지)
        p = "/Users/" + nfd + "/.local/state/cys-dept-" + "a" * 40 + "/cys.sock"
        self.assertEqual(len(p.encode("utf-8")), 106)
        rc, out, err = self._run_diag(p, "Darwin")
        self.assertEqual(rc, 0)
        self.assertIn("106B ≥ 104B", err, "NFD 홈을 문자 수로 재어 사유를 놓침: %r" % err)
        rc, out, err = self._run_diag(p, "Linux")
        self.assertEqual((rc, err), (0, ""), "Linux(108) 에서 106B 를 오경보")

    # 12) Windows named pipe 는 상한 없음 — 무출력
    def test_named_pipe_never_flagged(self):
        rc, out, err = self._run_diag("\\\\.\\pipe\\cys-dept-" + "a" * 200, "MINGW64_NT-10.0")
        self.assertEqual((rc, out, err), (0, "", ""))

    # 12) ★회귀 표본: 격리 HOME 아래 소켓이 상한을 넘어도(어떤 TMPDIR 이든 보장) 목 cysd 로 launch 완주 — 스폰 전 거부 금지
    def test_mkdtemp_home_over_limit_is_not_rejected_before_spawn(self):
        home, log, feedlist = make_home(os.path.join(self.tmp, "x" * 80))
        env = make_env(home)
        sock = os.path.join(home, ".local", "state", "cys-dept-a", "cys.sock")
        self.assertGreaterEqual(len(sock.encode("utf-8")), 104, "픽스처가 상한을 넘지 않음")
        write_reg(env, {})
        rc, out, err = self.run_dept("launch", "a", env=env)          # 목 cysd 스폰 경로(bind 없이 touch)
        self.assertEqual(rc, 0, "상한 초과 경로를 스폰 전에 거부(하네스 파손 회귀): exit=%d\n%s%s" % (rc, out, err))
        self.assertNotIn("sun_path", err, "성공 경로에 소켓 길이 사유가 섞임")
        self.assertIn("a", read_reg(env), "launch 완주인데 미등재")

    # 12) 기동 실패 경로: 소켓을 만들지 않는 목 cysd(bind 실패 재현) → exit 1 · 등재 회수 · 상한 초과일 때만 사유
    def test_daemon_start_failure_names_sock_len(self):
        _write_exec(os.path.join(self.home, ".local", "bin", "cysd"), "#!/bin/sh\nexit 0\n")
        write_reg(self.env, {})
        sock = os.path.join(self.home, ".local", "state", "cys-dept-a", "cys.sock")
        lim = 108 if sys.platform.startswith("linux") else 104
        over = len(sock.encode("utf-8")) >= lim
        rc, out, err = self.run_dept("launch", "a")
        self.assertEqual(rc, 1, "기동 실패 exit 계약(1) 회귀: %d\n%s%s" % (rc, out, err))
        self.assertIn("데몬 기동 실패", out + err)
        if over:
            self.assertIn("%dB" % len(sock.encode("utf-8")), err, "상한 초과 실패인데 소켓 길이 사유 부재: %r" % err)
        else:
            self.assertNotIn("sun_path", err, "상한 미만 실패에 소켓 길이 오진단")
        self.assertNotIn("a", read_reg(self.env), "기동 실패인데 등재 잔존(롤백 회귀)")

    # 12) 배선 census: "데몬 기동 실패" 를 내는 전 지점이 sock_len_diag 를 부른다 · 스폰 전 가드(assert_sock_len) 부활 금지
    def test_failure_sites_wired_and_no_prespawn_guard(self):
        src = open(DEPT, encoding="utf-8").read()
        sites = [l for l in src.splitlines() if "데몬 기동 실패" in l and "echo" in l and not l.lstrip().startswith("#")]
        self.assertGreaterEqual(len(sites), 3, "launch/allocate/create 기동 실패 지점 수 변동: %r" % sites)
        for l in sites:
            self.assertIn("sock_len_diag", l, "기동 실패 지점에 사유 배선 없음: %s" % l.strip())
        self.assertIsNone(re.search(r"^\s*assert_sock_len\b", src, re.M),
                          "스폰 전 소켓 길이 거부(assert_sock_len)가 부활 — 목 cysd 하네스·Linux 108 과 충돌")


class CrlfHygiene(Base):
    # 13) `read < <(python3 …)` 파이프 소비자는 전부 `| tr -d '\r'` — Windows 임베디드 python 의 \r\n(cys-dept reg_names 규칙)
    def test_procsub_consumers_strip_cr(self):
        src = open(DEPT, encoding="utf-8").read()
        lines = [l for l in src.splitlines() if "< <(python3" in l and not l.lstrip().startswith("#")]
        self.assertGreaterEqual(len(lines), 1, "process-substitution 소비자 0 — 검사 대상이 사라짐(census 갱신 필요)")
        for l in lines:
            self.assertIn("tr -d '\\r'", l, "\\r 미소거 파이프 소비자(create cwd 등재 \\r 오염 경로): %s" % l.strip())


class PostSpawnRegistryFailure(Base):
    """R4/Q3: 판독 실패 EXIT는 이번 호출이 만든 PID만 정리한다(실제 생존 목으로 검증).

    ready 파일을 공개하기 전에 오염을 완료하므로 시간 경합 없이 후행 read가 실패한다.
    allocate는 정상 후행 read가 없어 계정시드 실패→down의 read 실패(rc10)로 들어간다.
    테스트 정리는 픽스처가 기록한 자기 PID만 대상으로 하므로 변이 실행도 orphan을 남기지 않는다.
    """
    BAD_REGISTRY = b'{"depts":{"keep":'

    def _fixture(self, verb, reuse_dead=False, existing=False, competing=False):
        CreateGate._seed_catalog(self, "k1", "m1")
        directives = self._seed_promotable(ndepts=0)
        self.env.pop("CYS_ACCOUNT_DIR", None)
        self.env["CYS_DEPT_DEFAULT_ACCOUNT"] = "test"
        # create/allocate 모두 미승격 상태에서 시작한다. 실패 전에 신규 CEO 승격이
        # 일어나지 않아야 한다(이미 승격된 fixture만 쓰면 순서 결함을 놓친다).
        paths = [os.path.join(directives, n) for n in (
            "MASTER_DIRECTIVE.md", "MASTER_DIRECTIVE.md.pre-ceo", "CEO_TEMPLATE.md",
            ".ceo-template-applied")]
        paths.append(os.path.join(self.home, ".cys", "state", "ceo-pending"))
        self.ceo_before = {p: self._bytes_or_missing(p) for p in paths}
        self.sock = os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "cys.sock")
        if reuse_dead:
            write_reg(self.env, {"dept-1": {"socket": self.sock, "mission_key": "m1",
                                            "reserved_at": 0}})
        self.pidfile = os.path.join(self.tmp, "owned-daemon.pid")
        self.termfile = os.path.join(self.tmp, "owned-daemon.term")
        self.startfile = os.path.join(self.tmp, "daemon-starts")
        self.env.update({"GUARD_PID_FILE": self.pidfile, "GUARD_TERM_FILE": self.termfile,
                         "GUARD_START_FILE": self.startfile, "GUARD_CALL_LOG": self.log,
                         "GUARD_PING_COUNT": os.path.join(self.tmp, "ping-count"),
                         "GUARD_TARGET_SOCKET": self.sock,
                         "GUARD_EXISTING": "1" if existing else "0",
                         "GUARD_COMPETING": "1" if competing else "0",
                         "GUARD_FAIL_SEED": "1" if verb == "allocate" else "0"})
        helper = os.path.join(self.tmp, "owned-daemon-fixture.py")
        with open(helper, "w", encoding="utf-8") as f:
            f.write('''import os, signal, sys, time
from pathlib import Path
env = os.environ
def corrupt():
    Path(env["CYS_DEPTS_JSON"]).write_bytes(%r)
    if env["GUARD_FAIL_SEED"] == "1":
        # 실제 계정시드 fail-closed → down의 registry read rc10 경로를 유발한다.
        (Path(env["HOME"]) / ".cys/pack/agents.json").write_text('{"claude":{}}')
if sys.argv[1] == "winner":
    # 이번 cys-dept의 자식이 스폰되고 경쟁 패자로 사망한 뒤에만 승자의 ready를 공개한다.
    # 실제 두 프로세스를 쓰되 소켓은 기존 하네스처럼 파일 기반 목(실 bind 불필요)이다.
    pidfile = Path(env["GUARD_PID_FILE"])
    while True:
        try:
            loser = int(pidfile.read_text())
            break
        except (OSError, ValueError):
            time.sleep(0.02)
    while True:
        try:
            os.kill(loser, 0)
        except ProcessLookupError:
            break
        time.sleep(0.02)
    corrupt()
    sock = Path(env["GUARD_TARGET_SOCKET"])
    sock.parent.mkdir(parents=True, exist_ok=True)
    Path(str(sock)).with_suffix(".lock").write_text(str(os.getpid()))
    sock.write_text(str(os.getpid()))
    while True:
        signal.pause()
elif sys.argv[1] == "daemon":
    def terminate(signum, frame):
        Path(env["GUARD_TERM_FILE"]).write_text(str(os.getpid()))
        sys.exit(0)
    signal.signal(signal.SIGTERM, terminate)
    Path(env["GUARD_PID_FILE"]).write_text(str(os.getpid()))
    with open(env["GUARD_START_FILE"], "a") as out:
        out.write(str(os.getpid()) + "\\n")
    if env["GUARD_COMPETING"] == "1":
        sys.exit(0)  # singleton 경쟁 패자: 소켓/lock을 소유하지 않은 채 종료
    if env["GUARD_EXISTING"] != "1":
        corrupt()
    sock = Path(env["CYS_SOCKET"])
    sock.parent.mkdir(parents=True, exist_ok=True)
    Path(str(sock)).with_suffix(".lock").write_text(str(os.getpid()))
    sock.touch()
    while True:
        signal.pause()
else:
    args = sys.argv[2:]
    with open(env["GUARD_CALL_LOG"], "a") as out:
        out.write("cys " + " ".join(args) + "\\n")
    if args and args[0] == "ping":
        sock = env.get("CYS_SOCKET", "")
        if env["GUARD_COMPETING"] == "1":
            if sock != env["GUARD_TARGET_SOCKET"]:
                sys.exit(1)
            try:
                winner = int(Path(sock).read_text())
                os.kill(winner, 0)  # 파일 존재만으로 성공시키지 않는다: 실제 승자가 살아 있어야 응답
            except (OSError, ValueError):
                sys.exit(1)
            with open(env["GUARD_CALL_LOG"], "a") as out:
                out.write("pong " + str(winner) + "\\n")
            print(winner)
            sys.exit(0)
        if env["GUARD_EXISTING"] == "1" and sock == env["GUARD_TARGET_SOCKET"]:
            # 예약 시 미생존 → 직후 ready에서 기존 데몬이 응답하는 경합을 재현.
            counter = Path(env["GUARD_PING_COUNT"])
            count = int(counter.read_text()) + 1 if counter.exists() else 1
            counter.write_text(str(count))
            if count == 1:
                sys.exit(1)
            corrupt()
        sys.exit(0 if sock and Path(sock).exists() else 1)
    if args and args[0] == "identify":
        print(Path(env["GUARD_PID_FILE"]).read_text())
    sys.exit(0)
''' % self.BAD_REGISTRY)
        bindir = os.path.join(self.home, ".local", "bin")
        for name, mode in (("cys", "client"), ("cysd", "daemon")):
            _write_exec(os.path.join(bindir, name), "#!/bin/sh\nexec %s %s %s \"$@\"\n" % (
                shlex.quote(sys.executable), shlex.quote(helper), mode))
        # Mutation control may leave the newly spawned child running. Always reclaim that exact PID.
        self.addCleanup(self._cleanup_owned_daemon)
        if competing:
            self.winner_process = subprocess.Popen(
                [sys.executable, helper, "winner"], env=self.env,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.addCleanup(self._cleanup_winner)
            self.assertIsNone(self.winner_process.poll(), "경쟁 승자 fixture 미기동")
        if existing:
            env = dict(self.env, CYS_SOCKET=self.sock)
            proc = subprocess.Popen([sys.executable, helper, "daemon"], env=env,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.addCleanup(lambda: proc.wait(timeout=5))
            # addCleanup is LIFO: signal before wait, and wait only our direct child.
            self.addCleanup(self._cleanup_owned_daemon)
            self.assertTrue(self._wait_for(lambda: os.path.exists(self.sock)), "기존 데몬 fixture 미기동")
            self.assertIsNone(proc.poll(), "기존 데몬 positive control이 이미 사망")
            self.existing_process = proc

    @staticmethod
    def _bytes_or_missing(path):
        if not os.path.exists(path):
            return None
        with open(path, "rb") as f:
            return f.read()

    @staticmethod
    def _wait_for(predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return True
            time.sleep(0.02)
        return predicate()

    @staticmethod
    def _pid_alive(pid):
        try:
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False

    def _cleanup_owned_daemon(self):
        if not os.path.exists(self.pidfile):
            return
        if hasattr(self, "existing_process") and self.existing_process.poll() is not None:
            return
        with open(self.pidfile) as f:
            pid = int(f.read())
        try:
            if self._pid_alive(pid):
                os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass  # 정상 EXIT cleanup과 fixture 종료가 겹칠 수 있다.

    def _cleanup_winner(self):
        proc = self.winner_process
        if proc.poll() is None:
            proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)

    def _assert_failure(self, verb, existing=False, reuse_dead=False, competing=False):
        self._fixture(verb, reuse_dead=reuse_dead, existing=existing, competing=competing)
        args = ("create", "k1") if verb == "create" else ("allocate",)
        rc, out, err = self.run_dept(*args)
        self.assertEqual(rc, 10, "스폰 이후 판독 실패가 exit10으로 전파되지 않음\n" + out + err)
        self.assertIn("depts.json 판독 실패", err)
        with open(self.pidfile) as f:
            pid = int(f.read())
        with open(self.startfile) as f:
            self.assertEqual(f.read().splitlines(), [str(pid)], "중복 데몬 스폰")
        if competing:
            winner = self.winner_process
            self.assertNotEqual(pid, winner.pid, "경쟁 승자/패자 PID가 같음")
            self.assertFalse(self._pid_alive(pid), "스폰 경쟁 패자가 아직 생존")
            self.assertIn("pong %d\n" % winner.pid, self.calls(), "후행 ready가 경쟁 승자의 응답을 못 받음")
            self.assertIsNone(winner.poll(), "스폰 경쟁 승자를 EXIT가 죽임")
            self.assertTrue(os.path.exists(self.sock), "경쟁 승자의 소켓을 EXIT가 삭제")
            self.assertEqual(self._bytes_or_missing(self.sock), str(winner.pid).encode())
            self.assertEqual(self._bytes_or_missing(startup_lock(self.sock)), str(winner.pid).encode(),
                             "경쟁 승자의 startup-lock(cys.lock)을 EXIT가 변경/삭제")
            probe = subprocess.run([os.path.join(self.home, ".local", "bin", "cys"), "ping"],
                                   env=dict(self.env, CYS_SOCKET=self.sock),
                                   capture_output=True, text=True, timeout=5)
            self.assertEqual(probe.returncode, 0, "회수 뒤 경쟁 승자의 소켓 응답 단절: " + probe.stderr)
            self.assertEqual(probe.stdout.strip(), str(winner.pid))
            self.assertFalse(os.path.exists(self.termfile), "이미 죽은 경쟁 패자에 SIGTERM 발생")
        elif existing:
            self.assertIsNone(self.existing_process.poll(), "ready 재사용 데몬을 EXIT가 죽임")
            self.assertTrue(os.path.exists(self.sock), "ready 재사용 소켓을 EXIT가 삭제")
            self.assertFalse(os.path.exists(self.termfile), "기존 PID에 SIGTERM 발송")
        else:
            self.assertTrue(self._wait_for(lambda: not self._pid_alive(pid)),
                            "판독 실패 뒤 이번 호출의 cysd PID %d가 orphan으로 생존" % pid)
            self.assertTrue(os.path.exists(self.termfile), "신규 PID의 graceful SIGTERM 정리 미수행")
            self.assertTrue(os.path.exists(self.sock), "소유권 미확인 소켓을 EXIT가 삭제")
        self.assertTrue(os.path.exists(startup_lock(self.sock)), "소유권 미확인 startup-lock(cys.lock)을 EXIT가 삭제")
        self.assertEqual(self._bytes_or_missing(self.env["CYS_DEPTS_JSON"]), self.BAD_REGISTRY,
                         "실패 정리가 손상 레지스트리 바이트를 변경")
        self.assertNotIn("tombstone", self.calls(), "판독 실패 정리가 묘비를 변경")
        for path, before in self.ceo_before.items():
            self.assertEqual(self._bytes_or_missing(path), before, "판독 실패 중 CEO 변경: " + path)

    def test_create_new_read_failure_reclaims_only_spawned_pid(self):
        self._assert_failure("create")

    def test_create_reuse_dead_read_failure_reclaims_only_spawned_pid(self):
        self._assert_failure("create", reuse_dead=True)

    def test_allocate_read_failure_reclaims_only_spawned_pid(self):
        self._assert_failure("allocate")

    def test_create_read_failure_keeps_ready_existing_daemon(self):
        self._assert_failure("create", existing=True)

    def test_allocate_read_failure_keeps_ready_existing_daemon(self):
        self._assert_failure("allocate", existing=True)

    def test_create_spawn_loser_preserves_live_winner(self):
        self._assert_failure("create", competing=True)

    def test_windows_cleanup_maps_owned_pid_and_never_targets_unowned_pid(self):
        """실기 검증 아님: 실제 함수 + uname/ps/taskkill/kill/wait 목으로 MSYS 분기 계약만 검증."""
        with open(DEPT, encoding="utf-8") as f:
            src = f.read()
        funcs = []
        for name in ("graceful_kill", "cleanup_owned_dept_spawn"):
            match = re.search(r"^%s\(\)\{\n.*?^\}\n" % name, src, re.M | re.S)
            self.assertIsNotNone(match, name + " 정의 부재")
            funcs.append(match.group(0))
        for mode in ("mapped", "unowned", "unmapped", "zero", "system", "survivor"):
            with self.subTest(mode=mode):
                sock = seed_sock(self.home, "win-cleanup-" + mode)
                env = dict(self.env, GUARD_WIN_LOG=os.path.join(self.tmp, "win-" + mode),
                           GUARD_WIN_PS="PPID PID WINPID COMMAND\n1 12345 92345 cysd.exe",
                           GUARD_WIN_CHILD="" if mode == "unowned" else "12345",
                           GUARD_WIN_MODE=mode)
                if mode == "unmapped":
                    env["GUARD_WIN_PS"] = "PID PPID COMMAND\n12345 1 cysd.exe"
                elif mode in ("zero", "system"):
                    env["GUARD_WIN_PS"] = "PID PPID WINPID COMMAND\n12345 1 %s cysd.exe" % (
                        "0" if mode == "zero" else "1")
                # 셸 builtin까지 목으로 가려 임의 숫자 PID를 실 OS에 전달할 수 없다.
                script = '''set -euo pipefail
uname(){ echo MINGW64_NT-10.0; }
kill(){ printf 'kill %s\\n' "$*" >> "$GUARD_WIN_LOG"; [ "${GUARD_WIN_KILLED:-0}" != 1 ]; }
wait(){ printf 'wait %s\\n' "$*" >> "$GUARD_WIN_LOG"; return 0; }
ps(){ printf '%s\\n' "$GUARD_WIN_PS"; }
sleep(){ :; }
taskkill(){
  printf 'taskkill %s\\n' "$*" >> "$GUARD_WIN_LOG"
  if [ "$GUARD_WIN_MODE" != survivor ]; then GUARD_WIN_KILLED=1; fi
  return 0
}
''' + "\n".join(funcs) + '''
_dept_spawn_pid="$GUARD_WIN_CHILD"
sock="$1"
cleanup_owned_dept_spawn
'''
                r = subprocess.run(["bash", "-c", script, "cleanup-pin", sock], env=env,
                                   capture_output=True, text=True, timeout=10)
                self.assertEqual(r.returncode, 0, r.stderr)
                calls = self._bytes_or_missing(env["GUARD_WIN_LOG"]) or b""
                if mode == "mapped":
                    self.assertIn(b"taskkill //PID 92345 //T //F\n", calls)
                    self.assertNotIn(b"taskkill //PID 12345", calls, "POSIX PID를 native taskkill에 전달")
                    self.assertIn(b"wait 12345\n", calls)
                    self.assertTrue(os.path.exists(sock), "소유 자식 종료만으로 소켓을 삭제")
                elif mode == "survivor":
                    self.assertIn(b"taskkill //PID 92345 //T //F\n", calls)
                    self.assertNotIn(b"wait ", calls, "종료 미확인 자식을 무기한 wait")
                    self.assertTrue(os.path.exists(sock), "종료 미확인 소켓을 삭제")
                    self.assertIn("종료 미확인", r.stderr)
                    self.assertLess(calls.count(b"kill -0 "), 20, "bounded 종료 확인 퇴행")
                else:
                    self.assertNotIn(b"taskkill", calls, "소유/매핑 미확정 PID를 종료")
                    self.assertTrue(os.path.exists(sock), "소유/매핑 미확정 소켓을 삭제")
                    if mode == "unowned":
                        self.assertEqual(calls, b"", "ready 재사용 경로에서 PID probe/종료 발생")
                    else:
                        self.assertIn("WINPID 미확인", r.stderr)


class RegistryBom(Base):
    def _registry_python(self):
        """$REG 를 받는 Python 본문 전수 추출 — 변수명이 아닌 호출 인자로 판독 범위를 정한다."""
        with open(DEPT, encoding="utf-8") as f:
            src = f.read()
        blocks = [(m.start(2), m.group(2)) for m in re.finditer(
            r"^([^\n]*\bpython3\b[^\n]*<<'PY'[^\n]*)\n(.*?)^PY\s*$", src, re.M | re.S)
                  if '"$REG"' in m.group(1)]
        for line in src.splitlines():
            if "python3 -c " in line and '"$REG"' in line and not line.lstrip().startswith("#"):
                args = shlex.split(line)
                blocks.append((src.index(line), args[args.index("-c") + 1]))
        self.assertTrue(blocks, "레지스트리 Python 본문 0 — census 추출 규칙 갱신 필요")
        return [(src.count("\n", 0, pos), ast.parse(body)) for pos, body in blocks]

    # 14) census: 모든 레지스트리 read open 은 명시적 utf-8-sig — utf-8 로 퇴행해도 실패해야 한다.
    def test_no_encoding_blind_registry_reader(self):
        readers, blind = 0, []
        for line, tree in self._registry_python():
            for node in ast.walk(tree):
                if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "open"):
                    continue
                kw = {k.arg: k.value for k in node.keywords}
                mode = ast.literal_eval(kw.get("mode", node.args[1] if len(node.args) > 1 else ast.Constant("r")))
                if not mode.startswith("r"):
                    continue    # .lock / 원자 교체 tmp 쓰기는 별도 계약(UTF-8 출력).
                readers += 1
                encoding = kw.get("encoding")
                if not (isinstance(encoding, ast.Constant) and encoding.value == "utf-8-sig"):
                    blind.append(line + node.lineno)
        self.assertGreater(readers, 0, "레지스트리 read open 0 — census가 무효")
        self.assertEqual(blind, [], "utf-8-sig 아닌 레지스트리 판독 잔존(cys-dept 줄): %r" % blind)

    # P1: except Exception → 빈 레지스트리 복구는 cp949/잘린 JSON 을 원자 덮어쓰기하던 데이터 소실 원인.
    def test_no_exception_resets_registry(self):
        resets = []
        for line, tree in self._registry_python():
            for node in ast.walk(tree):
                if not (isinstance(node, ast.ExceptHandler) and isinstance(node.type, ast.Name)
                        and node.type.id == "Exception"):
                    continue
                for stmt in ast.walk(node):
                    if isinstance(stmt, ast.Assign):
                        try:
                            if ast.literal_eval(stmt.value) == {"depts": {}}:
                                resets.append(line + stmt.lineno)
                        except (ValueError, TypeError):
                            pass
        self.assertEqual(resets, [], "Exception 을 빈 레지스트리로 접는 경로 잔존(cys-dept 줄): %r" % resets)

    def test_rmw_read_failure_survives_fail_open_or_consumer(self):
        # R4/Q1: allocate cwd WARN 리터럴은 CI 계약이다. 호출부의 `|| echo WARN`가 RMW
        # 판독 실패를 삼키지 못하도록 실제 reg_set_field 경계의 exit10을 행동으로 검증한다.
        with open(DEPT, encoding="utf-8") as f:
            src = f.read()
        init = re.search(r"^reg_init\(\)\{[^\n]+\n", src, re.M)
        setter = re.search(r"^reg_set_field\(\)\{.*?^\}\n", src, re.M | re.S)
        self.assertIsNotNone(init, "reg_init 정의 부재")
        self.assertIsNotNone(setter, "reg_set_field 정의 부재")
        script = ('set -euo pipefail\nREG="$1"\n' + init.group(0) + setter.group(0)
                  + '\nreg_set_field keep cwd "$HOME" || echo WARN-CONSUMED\necho SURVIVED\n')
        write_reg(self.env, {"keep": {}})
        good = subprocess.run(["bash", "-c", script, "rmw-pin", self.env["CYS_DEPTS_JSON"]],
                              env=self.env, capture_output=True, text=True, timeout=10)
        self.assertEqual(good.returncode, 0, good.stderr)
        self.assertIn("SURVIVED", good.stdout, "정상 RMW positive control 실패")
        self.assertEqual(read_reg(self.env)["keep"]["cwd"], self.home)
        data = b'{"depts":{"keep":'
        self._write_bad_registry(data)
        bad = subprocess.run(["bash", "-c", script, "rmw-pin", self.env["CYS_DEPTS_JSON"]],
                             env=self.env, capture_output=True, text=True, timeout=10)
        self.assertEqual(bad.returncode, 10, "OR 소비자가 판독 실패를 삼킴\n" + bad.stdout + bad.stderr)
        self.assertEqual(bad.stdout, "", "판독 실패 뒤 WARN/후행 명령 실행")
        self.assertIn("depts.json 판독 실패", bad.stderr)
        with open(self.env["CYS_DEPTS_JSON"], "rb") as f:
            self.assertEqual(f.read(), data, "실패 뒤 원본 바이트 변경")

    def _write_bad_registry(self, data):
        with open(self.env["CYS_DEPTS_JSON"], "wb") as f:
            f.write(data)

    def _assert_registry_read_failure(self, data, *args):
        self._write_bad_registry(data)
        rc, out, err = self.run_dept(*args)
        self.assertEqual(rc, 10, "%r: 손상 레지스트리 exit=%d(≠10)\n%s%s" % (args, rc, out, err))
        self.assertEqual(len(err.splitlines()), 1, "판독 실패 사유는 stderr 1줄이어야 한다: %r" % err)
        self.assertIn("[cys-dept] depts.json 판독 실패 — 원본 보존: " + self.env["CYS_DEPTS_JSON"] + ": ", err)
        with open(self.env["CYS_DEPTS_JSON"], "rb") as f:
            self.assertEqual(f.read(), data, "판독 실패 뒤 원본 레지스트리 바이트 변동")
        return out

    # P1: launch 의 slug 확인·RMW 어느 단계에서도 디코딩/파싱 실패를 빈 등재로 바꿀 수 없다.
    def test_cp949_registry_launch_preserves_bytes(self):
        data = json.dumps({"depts": {"keep": {"display_name": "영업부(한국)"}}}, ensure_ascii=False).encode("cp949")
        self._assert_registry_read_failure(data, "launch", "newer")

    def test_truncated_registry_launch_preserves_bytes(self):
        self._assert_registry_read_failure(b'{"depts":{"keep":{"account":"work"}', "launch", "newer")

    def test_utf16_registry_launch_preserves_bytes(self):
        data = json.dumps({"depts": {"keep": {"mission_key": "m1"}}}).encode("utf-16")
        self._assert_registry_read_failure(data, "launch", "newer")

    def test_invalid_registry_list_is_not_empty_success(self):
        out = self._assert_registry_read_failure(b'{"depts":', "list")
        self.assertEqual(out, "", "판독 실패 list가 정상 목록을 출력")

    # P1: reg_count 소비자도 실패를 0개로 접지 않는다(승격/강등 판단에 빈 값 사용 금지).
    def test_invalid_registry_promote_count_fails_closed(self):
        pack = self._seed_promotable(ndepts=1)
        self._assert_registry_read_failure(b'{"depts":', "promote-ceo")
        with open(os.path.join(pack, "MASTER_DIRECTIVE.md"), encoding="utf-8") as f:
            self.assertEqual(f.read(), "STANDARD-MASTER\n", "판독 실패인데 CEO 승격 발생")
        self.assertFalse(os.path.exists(self._receipt()), "판독 실패인데 승격 영수증 생성")

    # P1: 등재/역인덱스 판독 실패 시 teardown·reg_count=0 오판·CEO 강등 모두 금지.
    def test_invalid_registry_teardown_keeps_ceo_and_socket(self):
        pack = self._seed_promotable(ndepts=1)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        sock = seed_sock(self.home, "d0")
        paths = [os.path.join(pack, "MASTER_DIRECTIVE.md"),
                 os.path.join(pack, "MASTER_DIRECTIVE.md.pre-ceo"), self._receipt()]
        before = {}
        for p in paths:
            with open(p, "rb") as f:
                before[p] = f.read()
        for args in (("down", "d0"), ("down-sock", sock), ("rotate", "d0")):
            with self.subTest(args=args):
                seed_sock(self.home, "d0")
                before_calls = self.calls()
                self._assert_registry_read_failure(b'{"depts":', *args)
                self.assertTrue(os.path.exists(sock), "판독 실패인데 소켓 teardown 발생")
                self.assertEqual(self.calls(), before_calls, "판독 실패인데 cys 프로브/teardown 호출")
                for p, data in before.items():
                    with open(p, "rb") as f:
                        self.assertEqual(f.read(), data, "판독 실패인데 CEO 강등/영수증 변경: %s" % p)

    # 14) 동작: BOM 달린 depts.json(기존 등재 1건) → list 에 보이고, 다른 이름 launch(reg_upsert RMW) 뒤에도 기존 등재·메타 보존
    def test_bom_registry_survives_rmw(self):
        keep_sock = seed_sock(self.home, "keep")
        keep = {"socket": keep_sock, "pack_dir": "", "display_name": "영업부(한국)",
                "account": "work", "mission_key": "sales", "cwd": self.home}
        with open(self.env["CYS_DEPTS_JSON"], "wb") as f:
            f.write(b"\xef\xbb\xbf" + json.dumps(
                {"depts": {"keep": keep}}, ensure_ascii=False).encode("utf-8"))
        rc, out, err = self.run_dept("list")
        self.assertEqual(rc, 0, err)
        self.assertIn("keep", out.split(), "BOM 레지스트리의 등재가 list 에 안 보임")
        seed_sock(self.home, "newer")
        rc, out, err = self.run_dept("launch", "newer")
        self.assertEqual(rc, 0, "launch 실패\n%s%s" % (out, err))
        reg = read_reg(self.env)
        self.assertIn("newer", reg)
        self.assertIn("keep", reg, "BOM 레지스트리가 RMW 에서 비워짐(기존 등재 소실 · K2-03 회귀)")
        self.assertEqual(reg["keep"], keep, "기존 부서 display_name/account/mission_key/cwd 메타 소실")


if __name__ == "__main__":
    unittest.main(verbosity=2)
