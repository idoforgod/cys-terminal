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
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile

# ★T10(DCE-3) 픽스처 계약: CEO 템플릿 = MASTER 전문의 상위집합(합성 계약 동형) — 스왑 직전
#   런타임 상위집합 검사를 통과해야 승격 계열 테스트가 승격 상태에 도달한다.
CEO_BODY = "CEO-HEADER\n---\nSTANDARD-MASTER\n"
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")


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


if __name__ == "__main__":
    unittest.main(verbosity=2)
