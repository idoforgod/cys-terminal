#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_create_progress.py — 0.14.43 GP: 「팀 직접 만들기」 단계 표지 · 스폰 뒤 소켓 대기 예산 노브 · 실패 문구 핀.

배경: GUI 의 「팀 직접 만들기」는 Tauri allocate_dept_daemon 이 `bash cys-dept allocate [--team-spec-b64 …]` 또는
`bash cys-dept create <키>` 를 실행하고 끝날 때까지 기다린다(계약: stdout 마지막 비어 있지 않은 줄 = 부서 이름 · 실패 시 종료
코드와 stderr 가 그대로 사용자에게 간다). 그 사이 화면은 스피너뿐이었고(실측 시작→소켓 청취 약 23초) 스폰 뒤 대기 상한을
넘으면 "데몬 기동 실패" 한 줄이 전부였다. 이 판이 더한 것(전부 가산 · 기본 동작 무변경):

  A. dept_ready_secs   — CYS_DEPT_READY_SECS 해석(정수 12~180 만 유효 · 그 밖 전부 12 — 상한 180 은 부트 폴백의 240초 제한시간보다 낮게 잡은 값) · 순수 함수
  B. ready_wait/ready  — 스폰 뒤 대기 횟수 = 초×10(기본 120 = 종전) · 사전 검사 `ready` 는 env 무관 120 고정
  C. dept_reserve_grace— 노브를 12 보다 올렸을 때만 create 예약 유예 = max(RESERVE_GRACE 또는 25, 노브+노브/5+17) — 유예가 따라 올라 틈을 줄인다
                          (공칭 기준이라 실제 대기보다 수 초~수십 초 짧을 수 있다 — 틈을 닫지는 못한다) · 노브를 올렸는데 RESERVE_GRACE 가 정수가 아니면
                          stderr 경고 1줄(기본 경로에서는 경고 없음) · 노브가 기본이면 종전 인라인 값(`${CYS_DEPT_RESERVE_GRACE:-25}`) 그대로(명시한 낮은 값도) + 실흐름 분기 핀
  D. @stage 표지       — allocate·create 의 stderr 1줄 기계 판독 표지(순서 불변식 · stdout 0회 · launch 무변경)
  E. 실패 문구         — 접두 "데몬 기동 실패" 유지 + (소켓 대기 N초 · 로그 경로 · 노브 안내) · 스트림은 각 줄 종전 그대로
  F. census            — 사전 검사 3곳은 `ready` · 스폰 뒤 3곳만 `ready_wait` · 표지 소재지 · bash 3.2/MSYS 안전 문법

라이브 무접촉: 격리 HOME + 목 cys/cysd/sleep($HOME/.local/bin — cys-dept 의 PATH 선두). 실 데몬·실 팩·~/.cys 를 건드리지 않는다.
목 sleep 은 no-op 이라 목 cysd 가 소켓을 열지 않는 실패 시나리오의 12초+12초 대기가 수 초로 줄고, 핑 횟수는 목 cys 가 센다.
함수 단위 핀은 cys-dept 에서 함수 정의를 **그대로 떼어** bash 로 평가한다(사본 금지 — test_dept_name_guard.SockLenDiag 와 같은 관례).

    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_dept_create_progress.py
돌연변이 검증용: CYS_DEPT_UNDER_TEST=<변이본 경로> — 제품 대신 그 스크립트를 대상으로 같은 핀을 돌린다(옆 파일은 변이본 폴더에 둔다).
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
DEPT = os.environ.get("CYS_DEPT_UNDER_TEST") or os.path.join(BIN, "cys-dept")

STAGE_LINE = re.compile(r"^\[cys-dept\] @stage ([a-z]+)$")


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def _write_exec(path, content):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    os.chmod(path, 0o755)


def func_text(src, name):
    """cys-dept 에서 함수 정의 전문을 **그대로** 뗀다 — 한 줄형(`name(){ …; }`)과 여러 줄형(`name(){` … 줄머리 `}`) 둘 다."""
    lines = src.splitlines()
    for i, l in enumerate(lines):
        if l.startswith(name + "(){"):
            if l.rstrip().endswith("}"):
                return l + "\n"
            out = [l]
            for m in lines[i + 1:]:
                out.append(m)
                if m == "}":
                    return "\n".join(out) + "\n"
            raise AssertionError("함수 %s 의 닫는 `}` 를 찾지 못했다" % name)
    raise AssertionError("cys-dept 에 함수 %s 정의가 없다(이 판의 도우미 소실)" % name)


def code_lines(src):
    return [l for l in src.splitlines() if not l.lstrip().startswith("#")]


def stages(err):
    """stderr 의 단계 표지 키를 순서대로 — 고정 형식(`[cys-dept] @stage <키>`)이 아닌 `@stage` 줄이 있으면 즉시 실패."""
    keys = []
    for l in err.splitlines():
        if "@stage" not in l:
            continue
        m = STAGE_LINE.match(l)
        if not m:
            raise AssertionError("표지 줄이 고정 형식이 아니다: %r" % l)
        keys.append(m.group(1))
    return keys


def last_line(text):
    nz = [l for l in text.splitlines() if l.strip()]
    return nz[-1].strip() if nz else ""


# ── 격리 하네스(목 cys · cysd · sleep) ───────────────────────────────────────────────
# 목 cys: ping 은 "소켓 파일 실존"으로 생사를 재현한다(allocate/create 의 lowest-unused 루프가 '모든 소켓 생존' 목에서 무한 루프하지
#   않게). STUB_PING_OK_FROM=N 이면 소켓별 N 번째 핑부터 성공한다 — 번호 점유 확인(파이썬 첫 핑)은 실패하고 사전 검사 `ready` 의 첫 핑이
#   성공하는 '이미 켜져 있다(재사용)' 분기를 만든다. 핑은 pings.log 에 한 줄씩(횟수 핀), 그 밖의 호출은 calls.log 에.
CYS_STUB = r'''#!/bin/bash
case "$1" in
  ping)
    echo "ping $CYS_SOCKET" >> "@PINGS@"
    if [ -n "${STUB_PING_OK_FROM:-}" ]; then
      f="@STATE@/ping.${CYS_SOCKET//\//_}"
      n=0; [ -f "$f" ] && read -r n < "$f"
      n=$((n + 1)); echo "$n" > "$f"
      [ "$n" -ge "$STUB_PING_OK_FROM" ] && exit 0
    fi
    [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;
  status|identify) echo "cys $*" >> "@CALLS@"; exit 1 ;;
esac
echo "cys $*" >> "@CALLS@"
exit 0
'''
# 목 cysd: 스폰 사실을 남기고 소켓 파일을 만든다(STUB_CYSD_MODE=dead 면 소켓 없이 즉시 종료 = 스폰 뒤 대기 실패 재현).
CYSD_STUB = r'''#!/bin/sh
echo "cysd spawn $CYS_SOCKET" >> "@CALLS@"
[ "${STUB_CYSD_MODE:-up}" = dead ] && exit 0
mkdir -p "$(dirname "$CYS_SOCKET")"
touch "$CYS_SOCKET"
exit 0
'''
SLEEP_STUB = "#!/bin/sh\nexit 0\n"
# ★결정론(부하 경주 차단): 목 sleep 이 no-op 이라 `ready_wait` 120회는 수백 ms 에 끝난다 — 백그라운드로 뜬 목 cysd 가 그 안에 소켓 파일을 못 만들면
#   (부하·nice·샌드박스 exec 지연) 성공 경로가 "데몬 기동 실패"로 뒤집힌다(실측 2회: 부하 중 실행 — 자식이 수 초 늦게 뜬다). 그래서 **성공해야 하는** 신규
#   스폰 시나리오는 소켓 파일이 아니라 핑 횟수로 '데몬이 떴다'를 정한다 — 소켓별 130 번째 핑부터 응답(번호 점유 확인 1 + 사전 검사 120 은 실패 · 스폰 뒤 대기 10번째 핑에서
#   성공). 같은 이유로 목 cysd 의 호출 기록(백그라운드 자식이 쓴다)은 어떤 단언에도 쓰지 않고, 스폰 여부는 동기 증거(`Sandbox.spawned` — 부모 셸이 만드는 cysd.log)로 본다.
UP_AT_PING = "130"


class Sandbox(object):
    def __init__(self, **env_extra):
        self.tmp = tempfile.mkdtemp(prefix="gp-")
        self.home = os.path.join(self.tmp, "home")
        self.calls = os.path.join(self.tmp, "calls.log")
        self.pings = os.path.join(self.tmp, "pings.log")
        state = os.path.join(self.tmp, "stubstate")
        bindir = os.path.join(self.home, ".local", "bin")
        for d in (state, bindir, os.path.join(self.home, ".cys")):
            os.makedirs(d, exist_ok=True)
        sub = {"@CALLS@": self.calls, "@PINGS@": self.pings, "@STATE@": state}
        for fname, body in (("cys", CYS_STUB), ("cysd", CYSD_STUB)):
            for k, v in sub.items():
                body = body.replace(k, v)
            _write_exec(os.path.join(bindir, fname), body)
        _write_exec(os.path.join(bindir, "sleep"), SLEEP_STUB)
        # seed_agents_account 소스(메인 팩 agents.json — env 맵 구조 · test_team_create_u16 과 같은 픽스처)
        pack = os.path.join(self.home, ".cys", "pack")
        os.makedirs(pack, exist_ok=True)
        with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
            json.dump({"claude": {"cmd": "claude", "env": {"CLAUDE_CONFIG_DIR": "/base"}}}, f)
        self.reg = os.path.join(self.home, ".cys", "depts.json")
        env = dict(os.environ)
        for k in list(env):
            if k.startswith("STUB_") or k.startswith("_CYS_TT_"):
                env.pop(k)
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_NO_AUTOSTART", "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG",
                  "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT", "CYS_DEPT_CWD", "CYS_DEPT_READY_SECS",
                  "CYS_DEPT_RESERVE_GRACE", "CYS_DEPT_CAP", "CYS_SURFACE_ID", "CYS_DEPT_NO_MASTER"):
            env.pop(k, None)
        env.update({"HOME": self.home, "CYS_DEPTS_JSON": self.reg, "CYS_DEPT_NO_MASTER": "1",
                    "PATH": bindir + os.pathsep + env.get("PATH", "")})
        env.update(env_extra)
        self.env = env

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run(self, *args, **kw):
        r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True, encoding="utf-8",
                           env=self.env, timeout=kw.get("timeout", 120))
        return r.returncode, r.stdout, r.stderr

    # ── 사실 조회 ──
    def read_calls(self):
        return _read(self.calls) if os.path.exists(self.calls) else ""

    def ping_count(self):
        return len(_read(self.pings).splitlines()) if os.path.exists(self.pings) else 0

    def spawned(self, name):
        """스폰이 **실제로 일어났는가**의 동기 증거 — 스폰 명령의 `>"<로그디렉터리>/cysd.log"` 리다이렉트는 **부모 셸이** 자식을 띄우는 순간 만든다.
        (목 cysd 의 호출 기록은 백그라운드 자식이 쓰므로 부하에서 늦거나 — 실패 경로의 회수가 자식을 먼저 죽이면 — 아예 없을 수 있다 → 단언에 쓰지 않는다.)"""
        return os.path.isfile(os.path.join(self.logdir(name), "cysd.log"))

    def read_reg(self):
        try:
            return json.loads(_read(self.reg)).get("depts", {})
        except (OSError, ValueError):
            return {}

    def write_reg(self, depts):
        with open(self.reg, "w", encoding="utf-8") as f:
            json.dump({"depts": depts}, f, ensure_ascii=False)

    def sock(self, name):
        return os.path.join(self.home, ".local", "state", "cys-dept-%s" % name, "cys.sock")

    def logdir(self, name):
        return os.path.join(self.home, ".local", "state", "cys-dept-%s" % name)

    def seed_catalog(self, key, mkey):
        acct = os.path.join(self.home, "acct")
        os.makedirs(acct, exist_ok=True)
        with open(os.path.join(self.home, ".cys", "dept-catalog.json"), "w", encoding="utf-8") as f:
            json.dump({"accounts": {"test": acct},
                       "departments": {key: {"display": "테스트부", "account": "test",
                                             "mission_key": mkey, "cwd": self.home}}}, f, ensure_ascii=False)

    def seed_entry(self, name, mkey, age, live_sock=False):
        """같은 mission_key 의 기존 등재 — age 초 전에 예약됨. live_sock 이면 소켓 파일을 만든다(목 ping 이 생존으로 본다)."""
        sk = self.sock(name)
        if live_sock:
            os.makedirs(os.path.dirname(sk), exist_ok=True)
            open(sk, "w").close()
        self.write_reg({name: {"socket": sk, "pack_dir": os.path.join(self.home, ".cys", "pack-dept-%s" % name),
                               "role": "dept-master", "mission_key": mkey, "cwd": self.home,
                               "account_dir": os.path.join(self.home, "acct"), "reserved_at": time.time() - age}})


def bash_eval(script, env_extra=None, unset=()):
    env = dict(os.environ)
    for k in ("CYS_DEPT_READY_SECS", "CYS_DEPT_RESERVE_GRACE", "OK_AT") + tuple(unset):
        env.pop(k, None)
    env.update(env_extra or {})
    r = subprocess.run(["bash", "-c", script], capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stdout, r.stderr


# ════════════════════════════════════════════════════════════════════════════════════
# A. dept_ready_secs — 순수 해석 함수
# ════════════════════════════════════════════════════════════════════════════════════
class DeptReadySecs(unittest.TestCase):
    # (환경변수 값 — None=미설정, 기대 초). 앞 11행은 GP 티켓이 지정한 핀이다 — 0.14.43 성찰(R1F-PK · S4 m2)에서 상한이 600 → 180 으로 내려가 그중 '600 → 600' 행만
    #   '180 → 180' 으로 바뀌었다(범위 밖은 종전처럼 12 로 읽는다 — 상한으로 접지 않는다). 종전 유효 값(600·599·300 …)은 아래에서 12 로 못박는다.
    CASES = [(None, "12"), ("12", "12"), ("60", "60"), ("180", "180"), ("181", "12"), ("11", "12"), ("0", "12"),
             ("-5", "12"), ("abc", "12"), ("30.5", "12"), ("", "12"),
             # m2 — 상한 경계(179·180·181)와 종전 유효였던 값(600·599·300·182 …): 이제 상한 초과 → 12
             ("179", "179"), ("182", "12"), ("300", "12"), ("599", "12"), ("600", "12"), ("601", "12"),
             # 경계·형식 — 선행 0 은 10진(08·09 가 8진 오류로 죽지 않는다) · 공백/부호/지수/긴 숫자는 무효
             ("13", "13"), ("012", "12"), ("08", "12"), ("09", "12"), ("060", "60"), ("00180", "180"), ("0181", "12"),
             ("0601", "12"), ("00600", "12"), ("99999999999999999999", "12"), (" 60", "12"), ("60 ", "12"), ("1e2", "12"),
             ("+60", "12"), ("6 0", "12")]

    @classmethod
    def setUpClass(cls):
        cls.fn = func_text(_read(DEPT), "dept_ready_secs")

    def test_interpretation_table(self):
        for val, want in self.CASES:
            with self.subTest(CYS_DEPT_READY_SECS=val):
                env = {} if val is None else {"CYS_DEPT_READY_SECS": val}
                rc, out, err = bash_eval("set -u\n" + self.fn + "dept_ready_secs\n", env)
                self.assertEqual((rc, out, err), (0, want + "\n", ""), "해석 불일치: %r → %r" % (val, out))

    def test_pure_no_stdout_noise_no_env_mutation(self):
        rc, out, err = bash_eval("set -u\n" + self.fn + 'dept_ready_secs >/dev/null; printf "%s" "${CYS_DEPT_READY_SECS-unset}"\n',
                                 {"CYS_DEPT_READY_SECS": "60"})
        self.assertEqual((rc, out, err), (0, "60", ""), "해석 함수가 env 를 바꿨거나 소음을 낸다")


# ════════════════════════════════════════════════════════════════════════════════════
# B. ready_wait / ready — 핑 횟수 예산
# ════════════════════════════════════════════════════════════════════════════════════
class ReadyBudget(unittest.TestCase):
    """sleep 을 no-op 함수로, CYS 를 호출 횟수를 세는 함수로 바꿔 횟수만 잰다(제품 함수 정의를 그대로 평가)."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("ready", "ready_wait", "dept_ready_secs"))

    def count(self, call, ready_secs=None, ok_at=0):
        script = ("set -u\n" + self.funcs + "sleep(){ :; }\nCOUNT=0\n"
                  'cys_probe(){ COUNT=$((COUNT + 1)); [ "${OK_AT:-0}" -gt 0 ] && [ "$COUNT" -ge "$OK_AT" ]; }\n'
                  "CYS=cys_probe\n" + call + '\necho "rc=$? count=$COUNT"\n')
        env = {"OK_AT": str(ok_at)}
        if ready_secs is not None:
            env["CYS_DEPT_READY_SECS"] = ready_secs
        rc, out, err = bash_eval(script, env)
        self.assertEqual((rc, err), (0, ""), "하네스 오류: %r" % err)
        m = re.fullmatch(r"rc=(\d+) count=(\d+)\n", out)
        self.assertIsNotNone(m, "출력 형식: %r" % out)
        return int(m.group(1)), int(m.group(2))

    def test_ready_wait_default_is_120_like_ready(self):
        self.assertEqual(self.count("ready_wait sock"), (1, 120), "기본에서 ready_wait 는 ready 와 같은 120회여야 한다(기본 무변경)")
        self.assertEqual(self.count("ready sock"), (1, 120))

    def test_ready_wait_follows_knob(self):
        for secs, want in (("12", 120), ("13", 130), ("60", 600), ("180", 1800)):
            with self.subTest(CYS_DEPT_READY_SECS=secs):
                self.assertEqual(self.count("ready_wait sock", secs), (1, want))
        for bad in ("11", "181", "600", "601", "abc", "", "30.5", "-5"):   # 무효 값(상한 180 초과 포함)은 12 로 → 120회
            with self.subTest(CYS_DEPT_READY_SECS=bad):
                self.assertEqual(self.count("ready_wait sock", bad), (1, 120))

    def test_ready_precheck_is_env_independent(self):
        for secs in (None, "13", "60", "180", "600", "abc"):
            with self.subTest(CYS_DEPT_READY_SECS=secs):
                self.assertEqual(self.count("ready sock", secs), (1, 120), "사전 검사 ready 는 노브와 무관한 120회 고정이어야 한다")

    def test_success_on_kth_probe_returns_zero_at_k(self):
        for k in (1, 7, 120, 121, 130):
            with self.subTest(k=k):
                self.assertEqual(self.count("ready_wait sock", "13", ok_at=k), (0, k))
        self.assertEqual(self.count("ready_wait sock", "13", ok_at=131), (1, 130), "예산(130) 밖의 성공은 닿지 못한다")
        self.assertEqual(self.count("ready_wait sock", None, ok_at=121), (1, 120), "기본 예산(120) 밖의 성공은 닿지 못한다")
        for k in (1, 7, 120):
            with self.subTest(ready_k=k):
                self.assertEqual(self.count("ready sock", "60", ok_at=k), (0, k))
        self.assertEqual(self.count("ready sock", "60", ok_at=121), (1, 120), "사전 검사는 노브를 올려도 120 에서 끝난다")


# ════════════════════════════════════════════════════════════════════════════════════
# C. dept_reserve_grace — 예약 유예 결합
# ════════════════════════════════════════════════════════════════════════════════════
class ReserveGraceUnit(unittest.TestCase):
    # (RESERVE_GRACE, READY_SECS, 파이썬 CYS_GRACE 로 넘어갈 값). None = 미설정.
    # master 결정(GP 검토): 유예 올림은 **노브를 12 보다 크게 올렸을 때만**이다 — 노브가 12 이하(미설정·명시 12·무효 값 → 12)면 종전 인라인 식 값 그대로.
    # 0.14.43 성찰(R1F-PK · S4 m1): 올림의 식이 `노브 + 13` → `노브 + 노브/5 + 17`(정수 나눗셈)로 바뀌었다 — 반복 1회가 0.1초가 아니라 약 0.118초라 `노브 + 13` 은
    #   실제 대기(사전 검사 + 노브 × 1.18)보다 짧았다. 기대값은 식에서 손으로 계산했다: 13→32 · 14→33 · 15→35 · 60→89 · 100→137 · 180→233.
    CASES = [
        # ── 노브 기본: 종전 `${CYS_DEPT_RESERVE_GRACE:-25}` 그대로 — 명시한 낮은 값도 올리지 않는다(종전 보존 핀)
        (None, None, "25"), ("10", None, "10"), ("0", None, "0"), ("10", "12", "10"), ("0", "12", "0"), ("40", "12", "40"),
        ("25", "12", "25"), ("26", "12", "26"), ("08", None, "08"), ("10", "abc", "10"), ("10", "601", "10"), ("10", "11", "10"),
        (None, "abc", "25"), (None, "601", "25"),
        # ── m2: 상한(180)을 넘어 12 로 접힌 노브도 기본 경로다 — 종전에 유효했던 600·300·181 도 유예는 인라인 식 그대로
        (None, "181", "25"), ("10", "181", "10"), ("10", "600", "10"), (None, "300", "25"), ("2.5", "600", "2.5"),
        # ── 노브를 12 보다 크게 올린 경우: max(설정값 또는 25, 노브 + 노브/5 + 17)
        (None, "60", "89"), ("10", "60", "89"), ("100", "60", "100"), ("73", "60", "89"), ("74", "60", "89"), ("30", "60", "89"),
        ("08", "60", "89"), (None, "13", "32"), ("25", "13", "32"), ("30", "13", "32"), ("31", "13", "32"), ("32", "13", "32"),
        ("33", "13", "33"), (None, "14", "33"), (None, "15", "35"), (None, "100", "137"),
        # 경계(식의 정확한 값 — 한 칸 아래는 올리고 · 같거나 위는 그대로)
        ("88", "60", "89"), ("89", "60", "89"), ("90", "60", "90"), (None, "180", "233"), ("232", "180", "233"), ("233", "180", "233"),
        ("234", "180", "234"),
        # ── 정수가 아니면 노브와 무관하게 그대로(파이썬이 판독 — max 는 정수일 때만) · 노브를 올렸다면 stderr 경고 1줄(아래 WARN_ROWS)
        ("2.5", None, "2.5"), ("2.5", "60", "2.5"), ("30.5", "60", "30.5"), ("abc", "60", "abc"), ("-5", "60", "-5"),
        (" 30", "60", " 30"), ("30.5", "180", "30.5"),
        # ── 값이 옵션으로 삼켜지지 않는다(echo 였다면 -n·-e 가 사라진다)
        ("-n", None, "-n"), ("-e", "60", "-e")]
    # m1: 노브를 올렸는데(>12) RESERVE_GRACE 가 정수가 아닌 조합 — 결합이 말없이 풀리지 않게 stderr 경고 정확히 1줄(stdout 값은 그대로). 이 밖의 모든 행은 stderr 가 비어 있다
    #   (기본 경로 무경고 · 정수 유예 무경고).
    WARN_ROWS = {("2.5", "60"), ("30.5", "60"), ("abc", "60"), ("-5", "60"), ("-e", "60"), (" 30", "60"), ("30.5", "180")}

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.fns = "".join(func_text(src, n) for n in ("dept_ready_secs", "dept_reserve_grace"))

    def test_value_table(self):
        for grace, ready, want in self.CASES:
            with self.subTest(RESERVE_GRACE=grace, READY_SECS=ready):
                env = {}
                if grace is not None:
                    env["CYS_DEPT_RESERVE_GRACE"] = grace
                if ready is not None:
                    env["CYS_DEPT_READY_SECS"] = ready
                rc, out, err = bash_eval("set -u\n" + self.fns + "dept_reserve_grace\n", env)
                self.assertEqual((rc, out), (0, want + "\n"), "유예 불일치: %r/%r → %r" % (grace, ready, out))
                if (grace, ready) in self.WARN_ROWS:
                    lines = err.splitlines()
                    self.assertEqual(len(lines), 1, "경고는 정확히 1줄이어야 한다: %r" % err)
                    self.assertTrue(lines[0].startswith("[cys-dept] WARN: "), lines[0])
                    for needle in ("CYS_DEPT_RESERVE_GRACE", "CYS_DEPT_READY_SECS=%s" % ready, "'%s'" % grace):
                        self.assertIn(needle, lines[0], "경고에 %r 가 있어야 원인을 알 수 있다" % needle)
                else:
                    self.assertEqual(err, "", "경고가 없어야 하는 조합에서 stderr 가 비지 않았다(기본 경로 무경고): %r/%r → %r" % (grace, ready, err))

    def test_warn_rows_are_exactly_the_non_integer_graces_under_a_raised_knob(self):
        # 표의 WARN_ROWS 가 정의(노브 > 12 이고 유예가 정수 아님)와 일치하는지 — 정의를 구현과 따로 한 번 더 적어 표가 구현을 베끼지 않았음을 확인한다.
        def raised(r):
            return r is not None and r.isdigit() and len(r) <= 3 and 12 < int(r) <= 180
        want = {(g, r) for g, r, _ in self.CASES if raised(r) and g is not None and not g.isdigit()}
        self.assertEqual(want, self.WARN_ROWS)

    def test_default_knob_value_equals_the_old_inline_expression(self):
        """노브가 12 이하(미설정·명시 12·무효 값)이면 어떤 RESERVE_GRACE 값이든(빈 값·낮은 값·선행 0·비정수·옵션 꼴 포함) 종전 인라인 식
        `${CYS_DEPT_RESERVE_GRACE:-25}` 와 같은 값이다 — '기본에서 종전과 같다'를 명시 설정까지 지킨다(master 결정)."""
        import shlex
        graces = ["", "0", "10", "24", "25", "26", "40", "08", "0025", "2.5", "abc", "-5", " 30", "-n", "-e", "99999999999999999999"]
        loop = ('bad=""\nfor g in %s; do\n  export CYS_DEPT_RESERVE_GRACE="$g"\n'
                '  a="$(dept_reserve_grace)"; b="${CYS_DEPT_RESERVE_GRACE:-25}"\n  [ "$a" = "$b" ] || bad="$bad [$g]:new=[$a]:old=[$b]"\ndone\n'
                'unset CYS_DEPT_RESERVE_GRACE\na="$(dept_reserve_grace)"; b="${CYS_DEPT_RESERVE_GRACE:-25}"\n'
                '[ "$a" = "$b" ] || bad="$bad [unset]:new=[$a]:old=[$b]"\nprintf "bad=<%%s>\\n" "$bad"\n') % " ".join(shlex.quote(g) for g in graces)
        for ready in (None, "12", "11", "abc", "", "601", "0", "-5", "30.5", "181", "300", "600"):   # 181·300·600 = 상한(180) 초과 → 12 → 기본 경로(R1F-PK · m2)
            with self.subTest(READY_SECS=ready):
                env = {} if ready is None else {"CYS_DEPT_READY_SECS": ready}
                rc, out, err = bash_eval("set -u\n" + self.fns + loop, env)
                self.assertEqual((rc, out, err), (0, "bad=<>\n", ""), "노브 기본인데 종전 인라인 식과 값이 갈렸다: %r %r" % (out, err))


class ReserveGraceBehavior(unittest.TestCase):
    """실흐름: 같은 mission_key 의 기존 등재(소켓 없음)를 만난 create 가 유예 안이면 REUSE_BOOTING(생성자를 믿고 즉시 반환 · 데몬을 다시 띄우지 않는다),
    유예 밖이면 REUSE_DEAD(재기동). 노브를 올리면 유예가 따라 올라가 같은 나이의 등재가 BOOTING 으로 읽힌다 — 중복 기동 경합의 틈이 줄어든다
    (공칭 기준이라 실제 대기보다 수 초~수십 초 짧을 수 있어 틈이 닫히는 것은 아니다 · 0.14.43 성찰 M1)."""

    def setUp(self):
        self.sb = Sandbox()
        self.sb.seed_catalog("k1", "m1")

    def tearDown(self):
        self.sb.cleanup()

    def test_default_grace_25_age_40_is_dead_revive(self):
        self.sb.env["STUB_PING_OK_FROM"] = "2"   # 번호 점유 확인(파이썬 첫 핑)은 실패 · 사전 검사 첫 핑은 성공 → 재사용 분기로 짧게 끝낸다
        self.sb.seed_entry("dept-1", "m1", age=40)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)·데몬 사망(grace 경과)", err, "기본 유예(25)에서 나이 40 은 REUSE_DEAD 여야 한다")
        self.assertEqual(stages(err), ["reserve", "probe", "up", "seat", "done"])

    def test_knob_raises_grace_so_same_age_is_booting(self):
        self.sb.env["CYS_DEPT_READY_SECS"] = "60"   # 유예 = 60 + 60/5 + 17 = 89
        self.sb.seed_entry("dept-1", "m1", age=40)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "노브 60 이면 유예 89 — 나이 40 은 아직 부팅 중으로 믿어야 한다(결합 소실 = 중복 기동 경합)")
        self.assertEqual(out.strip().splitlines()[-1], "dept-1")
        self.assertNotIn("cysd spawn", self.sb.read_calls(), "유예 안인데 데몬을 다시 띄웠다")
        self.assertFalse(self.sb.spawned("dept-1"), "유예 안인데 스폰 리다이렉트가 만들어졌다")
        self.assertEqual(stages(err), ["reserve", "done"])

    def test_knob_grace_has_an_upper_edge(self):
        self.sb.env["CYS_DEPT_READY_SECS"] = "60"
        self.sb.env["STUB_PING_OK_FROM"] = "2"
        self.sb.seed_entry("dept-1", "m1", age=96)   # 89 밖(7초 — 나이는 시간이 갈수록 커지므로 '밖' 쪽은 흔들리지 않는다)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)", err, "유예(89)를 넘긴 등재는 REUSE_DEAD 여야 한다(영구 신뢰 금지)")

    def test_knob_grace_covers_the_measured_per_iteration_cost(self):
        # m1(S4): 스폰 뒤 대기 반복 1회는 0.1초가 아니라 약 0.118초(sleep 0.1 + 핑 프로세스)라 노브 180 의 실제 대기는 약 212초 + 사전 검사 약 14초 = 226초다.
        #   종전 식(노브 + 13 = 193)은 그보다 33초 짧았고, 새 식(180 + 36 + 17 = 233)은 덮는다 — 나이 200(종전 식이면 유예 밖 = REUSE_DEAD)이 부팅 중으로 읽힌다.
        #   안쪽 여유 33초(나이 200 → 233): 부하 큰 러너에서 예약부터 검사까지 수 초 밀려도 흔들리지 않는다.
        self.sb.env["CYS_DEPT_READY_SECS"] = "180"
        self.sb.seed_entry("dept-1", "m1", age=200)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "노브 180 의 유예(233)가 나이 200 을 덮지 못했다 — 종전 식(193)으로 돌아갔다")
        self.assertFalse(self.sb.spawned("dept-1"), "유예 안인데 스폰 리다이렉트가 만들어졌다")
        self.assertEqual(stages(err), ["reserve", "done"])

    def test_explicit_larger_grace_is_honored(self):
        # n8(S4): 종전 검체는 나이 90 vs 유예 100 이라 안쪽 여유가 10초뿐이었다(부하 큰 러너에서 예약까지 10초를 넘기면 흔들림). 나이를 60 으로 내려 여유 40초.
        #   단언의 뜻(명시한 더 큰 유예가 노브 유예로 덮이지 않는다)을 지키려면 노브 유예가 나이보다 작아야 한다 — 새 식에서 노브 60 의 유예는 89 라 나이 60 은
        #   둘 다 안쪽이 되어 변별력을 잃는다. 그래서 노브를 30(유예 30 + 6 + 17 = 53)으로 둔다: 나이 60 은 노브 유예(53) 밖 · 명시 유예(100) 안(여유 40초).
        self.sb.env.update({"CYS_DEPT_RESERVE_GRACE": "100", "CYS_DEPT_READY_SECS": "30"})   # max(100, 53) = 100
        self.sb.seed_entry("dept-1", "m1", age=60)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "더 큰 명시 유예(100)를 노브 유예(53)로 덮어썼다")

    def test_explicit_low_grace_is_honored_at_default_knob(self):
        # 종전 보존(master 결정): 노브가 기본이면 명시한 낮은 유예(10)가 그대로 파이썬으로 간다 — 나이 15 는 유예(10) 밖이라 REUSE_DEAD.
        #   (유예를 25 로 올려 버리면 같은 등재가 REUSE_BOOTING 으로 즉시 반환된다 = 0.14.42 와 다른 동작)
        self.sb.env.update({"CYS_DEPT_RESERVE_GRACE": "10", "STUB_PING_OK_FROM": "2"})
        self.sb.seed_entry("dept-1", "m1", age=15)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)·데몬 사망(grace 경과)", err, "노브 기본에서 명시 유예 10 이 25 로 올랐다(종전 동작 아님)")
        self.assertEqual(stages(err), ["reserve", "probe", "up", "seat", "done"])

    def test_raised_knob_lifts_an_explicit_low_grace(self):
        # 노브를 올리면(60) 같은 명시 저값(10)도 노브 + 노브/5 + 17 = 89 로 올라 나이 15 를 아직 부팅 중으로 믿는다(중복 기동 경합의 틈을 줄인다)
        self.sb.env.update({"CYS_DEPT_RESERVE_GRACE": "10", "CYS_DEPT_READY_SECS": "60"})
        self.sb.seed_entry("dept-1", "m1", age=15)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "노브를 올렸는데 명시 저값(10)이 유예에 그대로 남았다")
        self.assertFalse(self.sb.spawned("dept-1"), "유예 안인데 스폰 리다이렉트가 만들어졌다")
        self.assertEqual(stages(err), ["reserve", "done"])

    @staticmethod
    def grace_warns(err):
        return [l for l in err.splitlines() if "WARN" in l and "CYS_DEPT_RESERVE_GRACE" in l]

    def test_non_integer_grace_with_raised_knob_warns_once_and_keeps_the_flow(self):
        # m1(S4): 노브를 올렸는데 유예가 정수가 아니면(30.5 — 파이썬이 float 로 읽는다) 결합이 말없이 풀린다 → 경고 정확히 1줄. 흐름은 종전 그대로
        #   (유예 30.5 는 나이 5 를 덮는다 = 부팅 중 · stdout 이름 계약·종료코드 불변).
        self.sb.env.update({"CYS_DEPT_READY_SECS": "60", "CYS_DEPT_RESERVE_GRACE": "30.5"})
        self.sb.seed_entry("dept-1", "m1", age=5)   # 유예 30.5 안쪽 여유 25초(부하 큰 러너에서도 흔들리지 않게)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(len(self.grace_warns(err)), 1, "경고가 정확히 1줄이어야 한다:\n" + err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err)
        self.assertEqual(stages(err), ["reserve", "done"])
        self.assertEqual(last_line(out), "dept-1")
        self.assertNotIn("WARN", out, "경고가 stdout 으로 샜다(부서 이름 계약 오염)")

    def test_no_grace_warning_on_default_knob_or_integer_grace(self):
        # 기본 경로(노브 미설정·무효·12 이하)와 정수 유예에서는 경고가 없다 — 비정수 유예여도 노브가 기본이면 종전 동작 그대로(경고 없음).
        for tag, env in (("기본 노브 + 비정수 유예", {"CYS_DEPT_RESERVE_GRACE": "30.5"}),
                         ("상한 초과(181 → 12) + 비정수 유예", {"CYS_DEPT_READY_SECS": "181", "CYS_DEPT_RESERVE_GRACE": "30.5"}),
                         ("노브 올림 + 정수 유예", {"CYS_DEPT_READY_SECS": "60", "CYS_DEPT_RESERVE_GRACE": "30"}),
                         ("노브 올림 + 유예 미설정", {"CYS_DEPT_READY_SECS": "60"})):
            with self.subTest(tag):
                sb = Sandbox(**env)
                try:
                    sb.seed_catalog("k1", "m1")
                    sb.seed_entry("dept-1", "m1", age=5)   # 모든 조합의 유예(25·30.5·89)보다 20초 이상 안쪽
                    rc, out, err = sb.run("create", "k1")
                    self.assertEqual(rc, 0, err[-800:])
                    self.assertEqual(self.grace_warns(err), [], "경고가 없어야 하는 조합에서 경고가 났다:\n" + err[-800:])
                    self.assertIn("생성자 부팅중(dept-1)", err)
                finally:
                    sb.cleanup()


# ════════════════════════════════════════════════════════════════════════════════════
# D. 단계 표지 — allocate
# ════════════════════════════════════════════════════════════════════════════════════
def assert_not_spawned(tc, sb, name, why="재사용/조기 반환인데 데몬을 띄웠다"):
    """음성 단언 — 동기 증거(부모 셸이 만드는 cysd.log)와 목 cysd 호출 기록 둘 다 없어야 한다."""
    tc.assertFalse(sb.spawned(name), why + "(cysd 로그 리다이렉트가 만들어졌다)")
    tc.assertNotIn("cysd spawn", sb.read_calls(), why)


def assert_order(tc, err, needles):
    pos = []
    for n in needles:
        i = err.find(n)
        tc.assertGreaterEqual(i, 0, "stderr 에 %r 가 없다:\n%s" % (n, err[-1500:]))
        pos.append(i)
    tc.assertEqual(pos, sorted(pos), "순서 위반: %r → %r" % (needles, pos))


class AllocateStages(unittest.TestCase):
    def setUp(self):
        self.sb = Sandbox()

    def tearDown(self):
        self.sb.cleanup()

    def test_new_spawn_order_and_contract(self):
        self.sb.env["STUB_PING_OK_FROM"] = UP_AT_PING
        rc, out, err = self.sb.run("allocate")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "done"])   # NO_MASTER=1 → seat 단계 없음
        self.assertEqual(last_line(out), "dept-1", "stdout 마지막 비어 있지 않은 줄 = 부서 이름(종전 계약)")
        self.assertNotIn("@stage", out, "표지가 stdout 으로 샜다(이름 계약 오염)")
        self.assertEqual(last_line(err), "[cys-dept] @stage done", "done 은 이름을 stdout 에 내기 직전의 마지막 stderr 줄이어야 한다")
        self.assertTrue(self.sb.spawned("dept-1"), "스폰 분기인데 cysd 로그 리다이렉트가 만들어지지 않았다")
        # 기존 줄과의 끼워짐: 표지는 그 단계의 기존 줄을 건드리지 않고 사이사이에만 든다
        assert_order(self, err, ["@stage reserve", "@stage probe", "@stage spawn", "@stage wait", "@stage up",
                                 "CYS_DEPT_NO_MASTER=1 — 셸 미생성(빈 데몬)", "allocate 완료", "@stage done"])

    def test_seat_stage_when_master_seat_step_exists(self):
        del self.sb.env["CYS_DEPT_NO_MASTER"]
        self.sb.env["STUB_PING_OK_FROM"] = UP_AT_PING
        rc, out, err = self.sb.run("allocate")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "seat", "done"])
        assert_order(self, err, ["@stage up", "@stage seat", "role=master 빈 셸 생성 완료", "@stage done"])
        self.assertEqual(last_line(out), "dept-1")

    def test_reuse_branch_skips_spawn_and_wait(self):
        self.sb.env["STUB_PING_OK_FROM"] = "2"   # 번호 점유 확인(파이썬 첫 핑)은 실패 · 사전 검사 첫 핑은 성공
        rc, out, err = self.sb.run("allocate")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "up", "done"], "재사용 분기에서 spawn·wait 를 내면 안 된다")
        assert_not_spawned(self, self.sb, "dept-1")
        self.assertIn("dept-1 이미 가동 중 — 재사용", err, "기존 줄이 사라졌다")
        self.assertEqual(last_line(out), "dept-1")
        self.assertNotIn("@stage", out)

    def test_idempotent_team_proposal_early_return_emits_only_done(self):
        # [판단] 같은 제안으로 이미 만든 팀의 멱등 반환은 예약·스폰이 없다 — 성공 종료 직전 done 만 낸다.
        import base64
        spec = {"v": 1, "id": "tp-20261003-0001", "display": "영상편집팀", "purpose": "유튜브 영상을 편집한다."}
        b64 = base64.urlsafe_b64encode(json.dumps(spec, ensure_ascii=False).encode("utf-8")).decode("ascii")
        self.sb.write_reg({"dept-1": {"socket": self.sb.sock("dept-1"), "pack_dir": "x", "role": "dept-master",
                                      "team_proposal_id": spec["id"]}})
        rc, out, err = self.sb.run("allocate", "--team-spec-b64", b64)
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["done"])
        self.assertEqual(last_line(out), "dept-1")
        assert_not_spawned(self, self.sb, "dept-1")

    def test_rejected_before_reservation_emits_no_stage(self):
        rc, out, err = self.sb.run("allocate", "--team-spec-b64", "@@not-b64@@")
        self.assertEqual(rc, 2, err[-400:])
        self.assertEqual(stages(err), [], "예약 전 거부(exit 2)에서 reserve 를 내면 안 된다")
        self.assertEqual(self.sb.read_reg(), {})


# ════════════════════════════════════════════════════════════════════════════════════
# D. 단계 표지 — create
# ════════════════════════════════════════════════════════════════════════════════════
class CreateStages(unittest.TestCase):
    def setUp(self):
        self.sb = Sandbox()
        self.sb.seed_catalog("k1", "m1")

    def tearDown(self):
        self.sb.cleanup()

    def test_new_spawn_order_with_seat(self):
        self.sb.env["STUB_PING_OK_FROM"] = UP_AT_PING
        self.sb.write_reg({})
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "seat", "done"])
        self.assertEqual(last_line(out), "dept-1")
        self.assertNotIn("@stage", out)
        self.assertEqual(last_line(err), "[cys-dept] @stage done")
        assert_order(self, err, ["@stage reserve", "@stage probe", "@stage spawn", "@stage wait", "@stage up",
                                 "@stage seat", "role=master 빈 셸 생성 완료", "create '테스트부'(dept-1) 완료", "@stage done"])

    def test_reuse_up_early_return_is_reserve_then_done(self):
        self.sb.seed_entry("dept-1", "m1", age=5, live_sock=True)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("k1 이미 생존(dept-1) — 재사용", err)
        self.assertEqual(stages(err), ["reserve", "done"])
        self.assertEqual(last_line(out), "dept-1")
        self.assertNotIn("@stage", out)

    def test_reuse_booting_early_return_is_reserve_then_done(self):
        self.sb.seed_entry("dept-1", "m1", age=5)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err)
        self.assertEqual(stages(err), ["reserve", "done"])
        self.assertEqual(last_line(out), "dept-1")
        assert_not_spawned(self, self.sb, "dept-1")

    def test_reuse_branch_skips_spawn_and_wait(self):
        self.sb.env["STUB_PING_OK_FROM"] = "2"
        self.sb.write_reg({})
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "up", "seat", "done"])
        self.assertIn("dept-1 이미 가동 — 재사용", err)
        assert_not_spawned(self, self.sb, "dept-1")


# ════════════════════════════════════════════════════════════════════════════════════
# E. 스폰 뒤 대기 실패 — 문구 · 표지 · 회수 · 대기 예산(실흐름 핑 횟수)
# ════════════════════════════════════════════════════════════════════════════════════
OLD_PREFIX = "[cys-dept] ERROR: %s 데몬 기동 실패"
NOTE_TAIL = "/cysd.log · 느린 디스크라면 CYS_DEPT_READY_SECS=60 처럼 대기 예산을 늘릴 수 있다)"


class WaitFailure(unittest.TestCase):
    """목 cysd 가 소켓을 열지 않는다(STUB_CYSD_MODE=dead) → 사전 검사 120회 + 스폰 뒤 대기 W회 모두 실패."""

    @classmethod
    def setUpClass(cls):
        cls.box = {}
        for tag, extra, verb in (("alloc", {}, "allocate"), ("alloc30", {"CYS_DEPT_READY_SECS": "30"}, "allocate"),
                                 ("create", {}, "create"), ("launch", {}, "launch")):
            sb = Sandbox(STUB_CYSD_MODE="dead", **extra)
            if verb == "create":
                sb.seed_catalog("k1", "m1")
            args = {"allocate": ("allocate",), "create": ("create", "k1"), "launch": ("launch", "a")}[verb]
            t0 = time.time()
            rc, out, err = sb.run(*args)
            cls.box[tag] = {"sb": sb, "rc": rc, "out": out, "err": err, "pings": sb.ping_count(), "secs": time.time() - t0,
                            "reg": sb.read_reg(), "spawned": sb.spawned("dept-1" if verb != "launch" else "a")}

    @classmethod
    def tearDownClass(cls):
        for v in cls.box.values():
            v["sb"].cleanup()

    def note_of(self, text, name):
        pre = OLD_PREFIX % name
        hits = [l for l in text.splitlines() if l.startswith(pre)]
        self.assertEqual(len(hits), 1, "실패 줄이 정확히 1줄이어야 한다(%r):\n%s" % (pre, text[-1500:]))
        return hits[0][len(pre):]

    def test_allocate_failure_message_prefix_note_stream_exit(self):
        b = self.box["alloc"]
        self.assertEqual(b["rc"], 1, "종료 코드(1) 계약")
        self.assertNotIn("데몬 기동 실패", b["out"], "allocate 의 실패 줄은 종전대로 stderr 다(stdout 오염 금지)")
        note = self.note_of(b["err"], "dept-1")
        self.assertTrue(note.startswith(" (소켓 대기 12초 · 로그: "), "꼬리 문구: %r" % note)
        self.assertTrue(note.endswith(NOTE_TAIL), "꼬리 문구: %r" % note)
        self.assertIn("로그: %s/cysd.log" % b["sb"].logdir("dept-1"), note, "로그 경로가 실제 스폰 리다이렉트 대상과 다르다")
        self.assertTrue(os.path.isfile(os.path.join(b["sb"].logdir("dept-1"), "cysd.log")), "스폰이 그 로그 파일을 실제로 만들지 않았다")
        # 접두는 줄머리에 그대로(기존 문구 보존)
        self.assertIn("\n" + OLD_PREFIX % "dept-1" + " (", "\n" + b["err"])

    def test_allocate_failure_stages_stop_at_wait_and_registry_reclaimed(self):
        b = self.box["alloc"]
        self.assertEqual(stages(b["err"]), ["reserve", "probe", "spawn", "wait"], "실패 경로는 낸 데까지 — up·done 이 있으면 안 된다")
        self.assertEqual(b["reg"], {}, "실패인데 레지스트리 등재가 남았다(예약 회수 종전 동작)")
        self.assertTrue(b["spawned"], "스폰 분기인데 cysd 로그 리다이렉트가 만들어지지 않았다")
        self.assertEqual(last_line(b["out"]), "", "실패 경로 stdout 은 비어 있다")

    def test_allocate_budget_default_pings(self):
        # 핑 총수 = 사전 검사 120(고정) + 스폰 뒤 W + 번호 점유 확인(파이썬) 소수 회. W=120(기본 = 종전과 같음)
        b = self.box["alloc"]
        self.assertIn(b["pings"] - 120, (120, 121, 122, 123), "총 핑 %d — 기본에서 사전 검사 120 + 스폰 뒤 120 이 아니다" % b["pings"])

    def test_allocate_budget_follows_knob_and_precheck_stays_fixed(self):
        # 노브 30 → W=300. 사전 검사까지 노브를 따르면(돌연변이 M1) 총수가 300 + 300 이 된다 → 아래 범위를 벗어난다.
        b = self.box["alloc30"]
        self.assertEqual(b["rc"], 1)
        note = self.note_of(b["err"], "dept-1")
        self.assertTrue(note.startswith(" (소켓 대기 30초 · 로그: "), "꼬리의 N 은 노브 값이어야 한다: %r" % note)
        self.assertIn(b["pings"] - 300, (120, 121, 122, 123), "총 핑 %d — 사전 검사(120 고정) + 스폰 뒤 300 이 아니다" % b["pings"])

    def test_create_failure_message_stage_reclaim(self):
        b = self.box["create"]
        self.assertEqual(b["rc"], 1)
        note = self.note_of(b["err"], "dept-1")
        self.assertTrue(note.startswith(" (소켓 대기 12초 · 로그: ") and note.endswith(NOTE_TAIL), note)
        self.assertIn("로그: %s/cysd.log" % b["sb"].logdir("dept-1"), note)
        self.assertNotIn("데몬 기동 실패", b["out"], "create 의 실패 줄은 종전대로 stderr 다")
        self.assertEqual(stages(b["err"]), ["reserve", "probe", "spawn", "wait"])
        self.assertEqual(b["reg"], {}, "NEW 실패인데 등재가 남았다(예약 회수 종전 동작)")

    def test_launch_failure_keeps_stdout_stream_and_has_no_stage(self):
        b = self.box["launch"]
        self.assertEqual(b["rc"], 1)
        pre = OLD_PREFIX % "a"
        out_hits = [l for l in b["out"].splitlines() if l.startswith(pre)]
        self.assertEqual(len(out_hits), 1, "launch 의 실패 줄은 종전대로 **stdout** 이다:\nout=%s\nerr=%s" % (b["out"], b["err"][-600:]))
        note = out_hits[0][len(pre):]
        self.assertTrue(note.startswith(" (소켓 대기 12초 · 로그: ") and note.endswith(NOTE_TAIL), note)
        self.assertIn("로그: %s/cysd.log" % b["sb"].logdir("a"), note)
        self.assertNotIn(pre, b["err"], "launch 실패 줄이 stderr 로 옮겨 갔다(스트림 변경)")
        self.assertNotIn("@stage", b["out"] + b["err"], "launch 는 무변경 동사 — 표지를 내면 안 된다")
        self.assertEqual(b["reg"], {}, "launch 실패 뒤 등재 회수(종전 동작)")


class SlowDaemonKnob(unittest.TestCase):
    """같은 '느린 데몬'(소켓이 300 번째 핑부터 응답 — 번호 점유 확인 1 + 사전 검사 120 + 스폰 뒤 179)을 기본 예산(스폰 뒤 120)으로는
    종전처럼 놓치고, 노브 30(스폰 뒤 300)으로는 잡는다 — 이 노브가 존재하는 이유(느린 디스크)의 종단 증명."""

    def run_alloc(self, **extra):
        sb = Sandbox(STUB_CYSD_MODE="dead", STUB_PING_OK_FROM="300", **extra)
        self.addCleanup(sb.cleanup)
        return sb, sb.run("allocate")

    def test_default_budget_misses_the_slow_daemon_as_before(self):
        sb, (rc, out, err) = self.run_alloc()
        self.assertEqual(rc, 1, err[-600:])
        self.assertIn(" (소켓 대기 12초 · 로그: ", err)
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait"])
        self.assertEqual(sb.read_reg(), {}, "실패 뒤 등재 회수(종전 동작)")
        self.assertLess(sb.ping_count(), 300, "기본 예산은 종전(사전 검사 120 + 스폰 뒤 120)이어야 한다")

    def test_knob_30_catches_the_slow_daemon(self):
        sb, (rc, out, err) = self.run_alloc(CYS_DEPT_READY_SECS="30")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "done"])
        self.assertEqual(last_line(out), "dept-1")
        self.assertGreaterEqual(sb.ping_count(), 300, "스폰 뒤 대기가 종전 상한(120)을 넘어 300 번째 핑까지 갔어야 한다")
        self.assertIn("dept-1", sb.read_reg())


class LaunchHasNoStage(unittest.TestCase):
    def test_launch_success_emits_no_stage(self):
        sb = Sandbox(STUB_PING_OK_FROM=UP_AT_PING)
        try:
            rc, out, err = sb.run("launch", "a")
            self.assertEqual(rc, 0, out + err[-800:])
            self.assertNotIn("@stage", out + err, "launch 는 무변경 동사 — 표지 0")
            self.assertIn("a", sb.read_reg())
        finally:
            sb.cleanup()


# ════════════════════════════════════════════════════════════════════════════════════
# F. census — 소스 구조 핀
# ════════════════════════════════════════════════════════════════════════════════════
class Census(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src = _read(DEPT)
        cls.code = code_lines(cls.src)

    def test_precheck_sites_are_ready_and_postspawn_sites_are_ready_wait(self):
        pre = [l for l in self.code if re.search(r'\bif ready "\$sock"', l)]
        self.assertEqual(len(pre), 3, "사전 검사 3곳(launch·allocate·create)은 `ready` 여야 한다: %r" % pre)
        post = [l for l in self.code if re.search(r'\bready_wait "\$sock" \|\|', l)]
        self.assertEqual(len(post), 3, "스폰 뒤 대기 3곳은 `ready_wait` 여야 한다: %r" % post)
        stale = [l for l in self.code if re.search(r'(^|[^_\w])ready "\$sock" \|\|', l)]
        self.assertEqual(stale, [], "스폰 뒤 대기에 노브를 안 타는 `ready` 가 남았다: %r" % stale)
        self.assertEqual(len([l for l in self.code if re.search(r'(^|[^_\w])ready "\$', l)]), 3, "`ready` 호출은 사전 검사 3곳뿐이어야 한다")
        for l in post:
            self.assertIn("데몬 기동 실패", l)
            self.assertIn("ready_fail_note", l)
            self.assertIn("sock_len_diag", l, "K2-07 census(test_dept_name_guard)와 같은 줄 배선")

    def test_failure_lines_keep_prefix_and_per_line_streams(self):
        post = [l for l in self.code if re.search(r'\bready_wait "\$sock" \|\|', l)]
        self.assertEqual(len(post), 3)
        # 파일 순서 = launch(stdout) · allocate(stderr) · create(stderr) — 스트림은 각 줄 종전 그대로
        self.assertIn('echo "[cys-dept] ERROR: $name 데몬 기동 실패$(ready_fail_note "$name")"; sock_len_diag "$sock"; exit 1; }', post[0])
        self.assertIn('echo "[cys-dept] ERROR: $name 데몬 기동 실패$(ready_fail_note "$name")" >&2; sock_len_diag "$sock"; exit 1; }', post[1])
        self.assertIn('echo "[cys-dept] ERROR: $name 데몬 기동 실패$(ready_fail_note "$name")" >&2; sock_len_diag "$sock"; exit 1; }', post[2])
        self.assertIn('reg_remove "$name" || exit $?; echo', post[0])
        self.assertIn('reg_remove "$name" || exit $?; echo', post[1])
        self.assertIn('then reg_remove "$name" || exit $?; fi; echo', post[2])

    def test_ready_definition_unchanged(self):
        self.assertIn('ready(){ local s="$1" i; for i in $(seq 1 120); do CYS_SOCKET="$s" "$CYS" ping >/dev/null 2>&1 && return 0; sleep 0.1; done; return 1; }',
                      self.src.splitlines(), "사전 검사 ready(120회 고정)의 정의가 바뀌었다")

    def test_stage_helper_pinned_and_only_route_to_stage_text(self):
        self.assertIn('dept_stage(){ echo "[cys-dept] @stage $1" >&2; }', self.src.splitlines(), "표지 도우미 형식은 고정이다")
        direct = [l for l in self.code if "@stage" in l and not l.startswith('dept_stage(){')]
        self.assertEqual(direct, [], "표지는 dept_stage 도우미로만 낸다(stdout 오염 방지): %r" % direct)

    def test_stage_sites_by_verb(self):
        lines = self.src.splitlines()

        def keys(seg):
            return re.findall(r'\bdept_stage ([a-z]+)\b', "\n".join(l for l in seg if not l.lstrip().startswith("#")))
        start = next(i for i, l in enumerate(lines) if l.startswith("allocate_dept(){"))
        end = next(i for i in range(start, len(lines)) if lines[i] == "}")
        self.assertEqual(keys(lines[start:end + 1]), ["done", "reserve", "probe", "spawn", "wait", "up", "seat", "done"],
                         "allocate_dept 의 표지 소재지/순서(첫 done = 멱등 재사용 반환)")
        cstart = next(i for i, l in enumerate(lines) if l == "  create)")
        cend = next(i for i in range(cstart, len(lines)) if lines[i] == "  down)")
        self.assertEqual(keys(lines[cstart:cend]), ["reserve", "done", "done", "probe", "spawn", "wait", "up", "seat", "done"],
                         "create 동사의 표지 소재지/순서(둘째·셋째 done = REUSE_UP·REUSE_BOOTING 조기 반환)")
        # 무변경 동사: launch 본체 · 그 뒤의 down/rotate/reap/… 갈래에는 표지가 없다
        lstart = next(i for i, l in enumerate(lines) if l.startswith("launch_dept(){"))
        lend = next(i for i in range(lstart, len(lines)) if lines[i] == "}")
        self.assertEqual(keys(lines[lstart:lend + 1]), [], "launch 본체는 무변경이어야 한다")
        self.assertEqual(keys(lines[cend:]), [], "down/rotate/reap 등 다른 동사는 무변경이어야 한다")

    def test_grace_wiring(self):
        self.assertEqual(len([l for l in self.code if 'CYS_GRACE="$(dept_reserve_grace)"' in l]), 1)
        self.assertEqual([l for l in self.code if 'CYS_GRACE="${CYS_DEPT_RESERVE_GRACE' in l], [], "종전 인라인 유예 식이 남았다(결합 소실)")
        self.assertIn('GRACE=float(os.environ.get("CYS_GRACE","25"))', self.src, "파이썬 GRACE 판독부는 무변경이어야 한다")

    def test_spawn_redirects_share_the_log_path_expression_used_by_the_note(self):
        spawns = [l for l in self.code if "nohup" in l and "cysd.log" in l]
        self.assertEqual(len(spawns), 5, "cysd 스폰 지점 수(launch 2 · allocate 2 · create 1)가 바뀌었다: %d" % len(spawns))
        for l in spawns:
            self.assertIn('>"$(dept_logdir "$name")/cysd.log" 2>&1 &', l, "스폰 로그 경로 표현이 실패 문구의 것과 다르다")
        note = func_text(self.src, "ready_fail_note")
        self.assertIn("로그: %s/cysd.log", note, "실패 문구 꼬리가 `<로그디렉터리>/cysd.log` 형식이 아니다")
        self.assertIn('"$(dept_logdir "$1")"', note, "꼬리의 로그 디렉터리는 스폰 리다이렉트와 같은 dept_logdir(이름) 이어야 한다")

    def test_new_helpers_use_only_bash32_msys_safe_syntax(self):
        a = self.src.index('dept_stage(){ echo')
        z = self.src.index("ready_fail_note(){")
        z = self.src.index("\n", z)
        block = "\n".join(l for l in self.src[a:z].splitlines() if not l.lstrip().startswith("#"))
        for pat, why in ((r"\$\{[^}]*(,,|\^\^)", "${var,,}/${var^^}(bash 4+)"), (r"declare\s+-A|typeset\s+-A", "연관 배열"),
                         (r"\bmapfile\b|\breadarray\b", "mapfile/readarray"), (r"local\s+-n\b", "nameref"), (r"&>>|\|&", "bash 4 리다이렉션"),
                         (r"\[\[", "[[ ]] (정수 판정은 case 꼴)")):
            self.assertIsNone(re.search(pat, block), "이 판의 도우미가 bash 3.2/MSYS 불안전 문법을 쓴다: %s" % why)


if __name__ == "__main__":
    unittest.main(verbosity=2)
