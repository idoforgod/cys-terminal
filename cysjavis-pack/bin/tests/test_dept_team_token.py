#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_team_token.py — 0.14.42 P5 `cys-dept create --team-token <토큰>` 단일소유 가드 갈래 회귀 핀.

설계 정본: 팀만들기-확인창-무반응-수정설계안-최종-20260923.md §11 R8 · §8-2 ① · §13 P5.
  "한 갈래만 추가한다 — --team-token <tok> 이 있고 **데몬 검증이 통과**하면 master 좌석도 create 를
   통과한다. cys-dept 는 스스로 판단하지 않고 데몬에 물어본다. 토큰이 없거나 검증 실패면 종전 그대로 exit 7."

무엇을 재는가(가드 판정은 데몬의 답 — 여기서는 목 `cys` 가 데몬 역할을 한다):
  T1 토큰 없이(master 좌석) create → exit 7 · 부작용 0(종전 그대로)
  T2 `--team-token` 값 없음 → exit 7 · 데몬에 **물었다**(consume 호출 기록) · 부작용 0
  T3 위조 토큰 → 데몬 거부 코드(token_unknown)를 stderr 에 그대로 · exit 7 · 부작용 0 · settle 0
  T4 데몬 무응답(daemon_unreachable) → exit 7 · 부작용 0
  T5 정상 토큰 → 생성 1회(등재 team_proposal_id · TEAM.md · stdout 마지막 줄 = dept-N) + settle created --dept
  T6 같은 토큰 재실행 → 데몬 token_consumed + inspect(같은 좌석·created) → **새 팀 0** · 같은 이름 · exit 0
  T7 재실행인데 settle 이 빠진 상태(consumed) → 멱등 보고 + settle created 재시도
  T8 재실행인데 다른 좌석(same_seat=false) → exit 7(보고도 하지 않는다)
  T9 생성 실패(cap 초과 exit 8) → exit 8 그대로 + settle failed --code 8 · 등재 0
  T10 데몬 통과 응답의 명세가 제안 id 와 어긋남 → 만들지 않음(exit 7) + settle failed
  T11 인자 과다 → exit 2 · 데몬에 묻기 전(소비 0)
  T12 env 주입 — 외부에서 export 한 `_CYS_TT_*` 는 관문 통과로 읽히지 않는다(위조 토큰 · 카탈로그 create 둘 다)
  T13 데몬 질의는 CYS_NO_AUTOSTART=1 로(묻는 행위가 데몬을 낳지 않는다) · 토큰은 argv 로만(env 無)
  T14 CSO 좌석이라도 토큰 갈래는 토큰 판정만 따른다(위조 토큰 → exit 7 · 역할로 새 허용 0)

라이브 무접촉: 격리 HOME + 목 cys/cysd($HOME/.local/bin). 실 데몬·실 팩·launchd·~/.cys 를 건드리지 않는다.
실제 부서 데몬 스폰·편성(에이전트 기동)은 목 cysd·편성 스크립트 부재(격리 팩)로 **실행되지 않는다** —
allocate 본체(번호 예약·등재·TEAM.md·계정 폴더)는 격리 HOME 안에서 실제로 돈다.

    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_dept_team_token.py
"""
import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
DEPT = os.path.join(BIN, "cys-dept")

TOKEN = "0123456789abcdef0123456789abcdef"
PID = "tp-1790203425-5048x"   # 라이브 대기 제안(tp-1790203425-5048)과 **다른** id — 무접촉 확인용 접미


def _write_exec(path, content):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    os.chmod(path, 0o755)


def spec_b64(obj):
    return base64.urlsafe_b64encode(json.dumps(obj, ensure_ascii=False).encode("utf-8")).decode("ascii")


def good_spec(i=PID):
    return {"v": 1, "id": i, "display": "일반저술출판부", "purpose": "책을 쓰고 출판한다.\n원고를 편집한다."}


def consume_ok(spec=None, pid=PID):
    spec = spec or good_spec()
    return json.dumps({"ok": True, "code": "consumed", "proposal_id": pid, "surface": 22,
                       "spec_b64": spec_b64(spec), "display": spec["display"]}, ensure_ascii=False)


def refusal(code, message="안내 문구"):
    return json.dumps({"ok": False, "code": code, "message": message}, ensure_ascii=False)


def inspect_ok(state, same_seat=True, pid=PID):
    return json.dumps({"ok": True, "code": "inspected", "state": state, "proposal_id": pid,
                       "surface": "22", "same_seat": same_seat})


# 목 cys — 데몬 역할(team-token)은 env 로 받은 JSON·rc 를 그대로 낸다. 모든 호출을 기록한다
# (인자 · CYS_NO_AUTOSTART · 토큰이 env 로 새는지 보려고 env 안의 토큰 문자열 여부).
MOCK_CYS = r'''#!/bin/sh
leak=no
env | grep -q "%(token)s" && leak=yes
printf 'cys %%s NA=%%s LEAK=%%s\n' "$*" "${CYS_NO_AUTOSTART-unset}" "$leak" >> "%(log)s"
case "$1" in
  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;
  status|identify) exit 1 ;;
  team-token)
    case "$2" in
      consume) printf '%%s\n' "${TT_CONSUME_OUT-}"; exit "${TT_CONSUME_RC:-1}" ;;
      inspect) printf '%%s\n' "${TT_INSPECT_OUT-}"; exit "${TT_INSPECT_RC:-1}" ;;
      settle)  out="${TT_SETTLE_OUT-}"; [ -n "$out" ] || out='{"ok":true,"code":"settled"}'
               printf '%%s\n' "$out"; exit "${TT_SETTLE_RC:-0}" ;;
    esac
    exit 2 ;;
esac
exit 0
'''


def make_home(tmp):
    home = os.path.join(tmp, "home")
    bindir = os.path.join(home, ".local", "bin")
    os.makedirs(bindir, exist_ok=True)
    os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
    log = os.path.join(tmp, "calls.log")
    _write_exec(os.path.join(bindir, "cys"), MOCK_CYS % {"log": log, "token": TOKEN})
    _write_exec(os.path.join(bindir, "cysd"),
                '#!/bin/sh\necho "cysd spawn $CYS_SOCKET" >> "%(log)s"\n'
                'mkdir -p "$(dirname "$CYS_SOCKET")"\ntouch "$CYS_SOCKET"\nexit 0\n' % {"log": log})
    pack = os.path.join(home, ".cys", "pack")
    os.makedirs(pack, exist_ok=True)
    with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
        json.dump({"claude": {"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": "/base"}}}, f)
    return home, log


def make_env(home, role="master"):
    env = dict(os.environ)
    env.update({"HOME": home,
                "CYS_DEPTS_JSON": os.path.join(home, ".cys", "depts.json"),
                "CYS_DEPT_NO_MASTER": "1",
                "CYS_STATE_DIR": os.path.join(home, ".cys", "state"),
                "PATH": os.path.join(home, ".local", "bin") + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_NO_AUTOSTART", "CYS_DEPT_ROTATE",
              "CYS_DEPT_CATALOG", "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT", "CYS_DEPT_CWD",
              "CYS_DEPT_CAP", "CYS_SURFACE_ID", "TT_CONSUME_OUT", "TT_CONSUME_RC", "TT_INSPECT_OUT",
              "TT_INSPECT_RC", "TT_SETTLE_OUT", "TT_SETTLE_RC"):
        env.pop(k, None)
    for k in list(env):
        if k.startswith("_CYS_TT_"):
            env.pop(k)
    if role:
        env["CYS_ROLE"] = role
        env["CYS_SURFACE_ID"] = "22"
    return env


def read_reg(env):
    try:
        with open(env["CYS_DEPTS_JSON"], encoding="utf-8") as f:
            return json.load(f).get("depts", {})
    except (OSError, ValueError):
        return {}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="p5-dept-tt-")
        self.home, self.log = make_home(self.tmp)
        self.env = make_env(self.home)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_dept(self, *args, **extra):
        env = dict(self.env)
        env.update({k: str(v) for k, v in extra.items()})
        r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                           encoding="utf-8", env=env, timeout=120)
        return r.returncode, r.stdout, r.stderr

    def calls(self):
        try:
            with open(self.log, encoding="utf-8") as f:
                return f.read().splitlines()
        except OSError:
            return []

    def tt_calls(self, verb):
        return [c for c in self.calls() if c.startswith("cys team-token %s " % verb)]

    def assert_no_side_effects(self):
        self.assertEqual(read_reg(self.env), {}, "거부됐는데 레지스트리 등재")
        cys_dir = os.path.join(self.home, ".cys")
        self.assertEqual([d for d in os.listdir(cys_dir) if d.startswith("pack-dept-")], [], "거부됐는데 팀 팩 생성")
        self.assertFalse(any(c.startswith("cysd spawn") for c in self.calls()), "거부됐는데 데몬 기동")

    def assert_created_once(self, out, name="dept-1"):
        self.assertEqual(out.strip().splitlines()[-1], name, "stdout 마지막 줄 = 확정 이름 계약")
        reg = read_reg(self.env)
        mine = [k for k, e in reg.items() if isinstance(e, dict) and e.get("team_proposal_id") == PID]
        self.assertEqual(mine, [name], "이 제안으로 만든 팀이 정확히 1개가 아니다: %r" % reg)
        team_md = os.path.join(self.home, ".cys", "pack-dept-%s" % name, "TEAM.md")
        with open(team_md, encoding="utf-8") as f:
            body = f.read()
        self.assertIn("일반저술출판부", body)
        self.assertIn(PID, body)


class T1NoToken(Base):
    def test_master_create_without_token_is_exit7(self):
        rc, out, err = self.run_dept("create", "somekey")
        self.assertEqual(rc, 7, "토큰 없는 master create 가 exit 7 이 아니다: rc=%s err=%s" % (rc, err[-400:]))
        self.assert_no_side_effects()
        self.assertEqual(self.tt_calls("consume"), [], "토큰 없는 create 가 토큰 소비를 물었다")


class T2EmptyToken(Base):
    def test_flag_without_value_asks_daemon_and_is_exit7(self):
        rc, out, err = self.run_dept("create", "--team-token",
                                     TT_CONSUME_OUT=refusal("token_missing", "토큰 없음"), TT_CONSUME_RC=1)
        self.assertEqual(rc, 7, "값 없는 --team-token 이 exit 7 이 아니다: rc=%s err=%s" % (rc, err[-400:]))
        self.assertEqual(len(self.tt_calls("consume")), 1, "데몬에 묻지 않았다(스스로 판단 금지)")
        self.assertIn("token_missing", err)
        self.assert_no_side_effects()


class T3Forged(Base):
    def test_forged_token_gets_daemon_refusal_code_and_exit7(self):
        rc, out, err = self.run_dept("create", "--team-token", "f" * 32,
                                     TT_CONSUME_OUT=refusal("token_unknown", "승인 말씀이 시스템에 닿지 않았습니다"),
                                     TT_CONSUME_RC=1)
        self.assertEqual(rc, 7, "위조 토큰이 exit 7 이 아니다: rc=%s err=%s" % (rc, err[-400:]))
        self.assertIn("token_unknown", err, "데몬 거부 코드가 안내에 없다(무안내 거부)")
        self.assertIn("승인 말씀이 시스템에 닿지 않았습니다", err, "데몬(P3) 오너 문구가 안내에 없다")
        self.assertEqual(len(self.tt_calls("consume")), 1)
        self.assertEqual(self.tt_calls("settle"), [], "만들지 않았는데 결과 기록(settle)을 보냈다")
        self.assert_no_side_effects()


class T4Unreachable(Base):
    def test_daemon_unreachable_is_exit7(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN,
                                     TT_CONSUME_OUT=refusal("daemon_unreachable", "cannot connect"), TT_CONSUME_RC=3)
        self.assertEqual(rc, 7, "데몬 무응답이 exit 7 이 아니다: rc=%s" % rc)
        self.assert_no_side_effects()
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT="", TT_CONSUME_RC=0)
        self.assertEqual(rc, 7, "빈 응답(rc 0)이 통과로 읽혔다: rc=%s" % rc)
        self.assert_no_side_effects()


class T5Normal(Base):
    def test_valid_token_creates_once_and_settles_created(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0)
        self.assertEqual(rc, 0, "정상 토큰 생성 실패: rc=%s err=%s" % (rc, err[-1200:]))
        self.assert_created_once(out)
        settles = self.tt_calls("settle")
        self.assertEqual(len(settles), 1, "settle 이 정확히 1회가 아니다: %r" % settles)
        self.assertIn("--outcome created", settles[0])
        self.assertIn("--dept dept-1", settles[0])
        self.assertIn("--token %s" % TOKEN, settles[0])


class T6Rerun(Base):
    def test_rerun_same_token_is_idempotent(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0)
        self.assertEqual(rc, 0, err[-800:])
        spawns = sum(1 for c in self.calls() if c.startswith("cysd spawn"))
        rc2, out2, err2 = self.run_dept("create", "--team-token", TOKEN,
                                        TT_CONSUME_OUT=refusal("token_consumed", "제안 내용이 그사이 바뀌어"),
                                        TT_CONSUME_RC=1, TT_INSPECT_OUT=inspect_ok("created"), TT_INSPECT_RC=0)
        self.assertEqual(rc2, 0, "같은 토큰 재실행이 멱등이 아니다: rc=%s err=%s" % (rc2, err2[-800:]))
        self.assertEqual(out2.strip().splitlines()[-1], "dept-1", "재실행이 같은 이름을 돌려주지 않는다")
        self.assert_created_once(out2)
        self.assertEqual(sum(1 for c in self.calls() if c.startswith("cysd spawn")), spawns, "재실행이 데몬을 또 띄웠다")
        self.assertEqual(len(self.tt_calls("settle")), 1, "created 상태 재실행이 settle 을 또 보냈다")
        self.assertIn("멱등", err2)
        # 카드 정리(allow 소비 = done) 뒤의 재실행도 같은 팀을 보고한다(샌드박스 E2E 11단계 실측의 목 판).
        rc3, out3, err3 = self.run_dept("create", "--team-token", TOKEN,
                                        TT_CONSUME_OUT=refusal("token_consumed"), TT_CONSUME_RC=1,
                                        TT_INSPECT_OUT=inspect_ok("done"), TT_INSPECT_RC=0)
        self.assertEqual(rc3, 0, "정리 뒤 재실행이 멱등이 아니다: rc=%s err=%s" % (rc3, err3[-400:]))
        self.assert_created_once(out3)
        self.assertEqual(len(self.tt_calls("settle")), 1, "done 상태 재실행이 settle 을 보냈다")


class T7RerunUnsettled(Base):
    def test_rerun_after_lost_settle_reports_and_settles(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0,
                                     TT_SETTLE_OUT=refusal("lock_unavailable"), TT_SETTLE_RC=1)
        self.assertEqual(rc, 0, "settle 실패가 생성 성공을 뒤집었다: rc=%s err=%s" % (rc, err[-800:]))
        self.assertIn("lock_unavailable", err, "결과 기록 실패가 조용했다")
        rc2, out2, err2 = self.run_dept("create", "--team-token", TOKEN,
                                        TT_CONSUME_OUT=refusal("token_consumed"), TT_CONSUME_RC=1,
                                        TT_INSPECT_OUT=inspect_ok("consumed"), TT_INSPECT_RC=0)
        self.assertEqual(rc2, 0, err2[-800:])
        self.assert_created_once(out2)
        settles = self.tt_calls("settle")
        self.assertEqual(len(settles), 2, "재실행이 빠진 settle 을 다시 보내지 않았다: %r" % settles)
        self.assertIn("--outcome created", settles[-1])


class T8RerunOtherSeat(Base):
    def test_rerun_from_other_seat_is_exit7(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0)
        self.assertEqual(rc, 0, err[-800:])
        rc2, out2, err2 = self.run_dept("create", "--team-token", TOKEN,
                                        TT_CONSUME_OUT=refusal("token_consumed"), TT_CONSUME_RC=1,
                                        TT_INSPECT_OUT=inspect_ok("created", same_seat=False), TT_INSPECT_RC=0)
        self.assertEqual(rc2, 7, "다른 좌석의 재실행이 exit 7 이 아니다: rc=%s" % rc2)
        self.assertIn("token_consumed", err2)


class T9CreateFails(Base):
    def test_create_failure_passes_rc_and_settles_failed(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0,
                                     CYS_DEPT_CAP=0)
        self.assertEqual(rc, 8, "cap 초과 rc 가 전파되지 않았다: rc=%s err=%s" % (rc, err[-800:]))
        self.assertEqual(read_reg(self.env), {}, "실패했는데 등재가 남았다")
        settles = self.tt_calls("settle")
        self.assertEqual(len(settles), 1, settles)
        self.assertIn("--outcome failed", settles[0])
        self.assertIn("--code 8", settles[0])
        self.assertIn("제안은 그대로 남아 있습니다", err, "§10 생성 실패 문구 꼬리가 없다")


class T10Inconsistent(Base):
    def test_spec_not_matching_proposal_is_not_created(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN,
                                     TT_CONSUME_OUT=consume_ok(spec=good_spec("tp-other-1")), TT_CONSUME_RC=0)
        self.assertEqual(rc, 7, "제안 id 와 어긋난 명세로 만들었다: rc=%s" % rc)
        self.assert_no_side_effects()
        settles = self.tt_calls("settle")
        self.assertEqual(len(settles), 1, "소비된 토큰의 실패 기록이 없다: %r" % settles)
        self.assertIn("--outcome failed", settles[0])


class T11ExtraArgs(Base):
    def test_extra_args_exit2_before_asking(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, "extra",
                                     TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0)
        self.assertEqual(rc, 2, "인자 과다가 exit 2 가 아니다: rc=%s err=%s" % (rc, err[-400:]))
        self.assertEqual(self.tt_calls("consume"), [], "인자 오류인데 토큰을 소비했다")
        self.assert_no_side_effects()


class T12EnvInjection(Base):
    def test_exported_internal_markers_do_not_pass_the_gate(self):
        inj = {"_CYS_TT_OK": "1", "_CYS_TT_B64": spec_b64(good_spec()), "_CYS_TT_PID": PID,
               "_CYS_TT_TOKEN": TOKEN, "_CYS_TT_NAME": "dept-1", "_CYS_TT_STATE": "created"}
        rc, out, err = self.run_dept("create", "--team-token", "f" * 32,
                                     TT_CONSUME_OUT=refusal("token_unknown"), TT_CONSUME_RC=1, **inj)
        self.assertEqual(rc, 7, "export 한 내부 표식이 관문 통과로 읽혔다: rc=%s" % rc)
        self.assert_no_side_effects()
        rc, out, err = self.run_dept("create", "somekey", **inj)
        self.assertEqual(rc, 7, "export 한 내부 표식이 카탈로그 create 를 토큰 갈래로 바꿨다: rc=%s" % rc)
        self.assert_no_side_effects()
        inj["_CYS_TT_OK"] = "again"
        rc, out, err = self.run_dept("create", "--team-token", "f" * 32,
                                     TT_CONSUME_OUT=refusal("token_unknown"), TT_CONSUME_RC=1, **inj)
        self.assertEqual(rc, 7, "export 한 멱등 표식이 통과로 읽혔다: rc=%s" % rc)


class T13QueryHygiene(Base):
    def test_daemon_is_asked_without_autostart_and_token_only_in_argv(self):
        self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT=consume_ok(), TT_CONSUME_RC=0)
        tt = [c for c in self.calls() if c.startswith("cys team-token ")]
        self.assertTrue(tt, "데몬 질의가 없다")
        for c in tt:
            self.assertIn("NA=1", c, "묻는 행위가 autostart 를 막지 않았다: %s" % c)
            self.assertIn("LEAK=no", c, "토큰이 env 로 새어 자식에게 상속된다: %s" % c)


class T15CliSkew(Base):
    """E2E 실측(2026-09-23)에서 드러난 스큐: cys-dept 는 PATH 앞에 /usr/local/bin 을 붙이므로 **설치된 구버전
    cys**(team-token 동사 없음 → clap 사용 오류 exit 2 · 출력 0)를 부를 수 있다. 거부(exit 7)는 옳지만 안내가
    '데몬에 닿지 못했다' 였다 — 원인(구버전 CLI)과 처방(갱신)을 말해야 한다(무안내 거부 금지)."""

    def test_old_cli_without_team_token_verb_is_named(self):
        rc, out, err = self.run_dept("create", "--team-token", TOKEN, TT_CONSUME_OUT="", TT_CONSUME_RC=2)
        self.assertEqual(rc, 7, "구버전 CLI 에서 exit 7 이 아니다: rc=%s" % rc)
        self.assertIn("구버전", err, "구버전 CLI 스큐가 안내에 없다: %s" % err[-400:])
        self.assert_no_side_effects()


class T14CsoSeat(Base):
    def test_role_does_not_open_token_branch(self):
        self.env = make_env(self.home, role="cso")
        rc, out, err = self.run_dept("create", "--team-token", "f" * 32,
                                     TT_CONSUME_OUT=refusal("token_unknown"), TT_CONSUME_RC=1)
        self.assertEqual(rc, 7, "CSO 좌석의 위조 토큰이 통과했다: rc=%s" % rc)
        self.assert_no_side_effects()


if __name__ == "__main__":
    if not os.environ.get("CYS_PACK_DIR"):
        sys.stderr.write("CYS_PACK_DIR 를 격리 경로로 지정하고 실행하라(라이브 팩 무접촉): "
                         'CYS_PACK_DIR="$(mktemp -d)" python3 %s\n' % __file__)
        sys.exit(2)
    unittest.main(verbosity=2)
