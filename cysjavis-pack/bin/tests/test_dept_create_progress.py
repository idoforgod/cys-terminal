#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_create_progress.py — 0.14.43 GP: 「팀 직접 만들기」 단계 표지 · 스폰 뒤 소켓 대기 예산 노브 · 실패 문구 핀.

배경: GUI 의 「팀 직접 만들기」는 Tauri allocate_dept_daemon 이 `bash cys-dept allocate [--team-spec-b64 …]` 또는
`bash cys-dept create <키>` 를 실행하고 끝날 때까지 기다린다(계약: stdout 마지막 비어 있지 않은 줄 = 부서 이름 · 실패 시 종료
코드와 stderr 가 그대로 사용자에게 간다). 그 사이 화면은 스피너뿐이었고(실측 시작→소켓 청취 약 23초) 스폰 뒤 대기 상한을
넘으면 "데몬 기동 실패" 한 줄이 전부였다. 이 판이 더한 것(전부 가산 · 기본 동작 무변경):

  A. dept_ready_secs   — CYS_DEPT_READY_SECS 해석(정수 12~180 만 유효 · 그 밖 전부 12 — 상한 180 은 부트 폴백의 240초 제한시간보다 낮게 잡은 값) · 순수 함수 · 비어 있지 않은 무효 값은 경고 1줄(R2F-PK · 프로세스당 1회 · 기본 경로 무경고)
  B. ready_wait/ready  — 스폰 뒤 대기 = 횟수 상한(초×10 · 기본 120 = 종전) **그리고** 121번째 반복부터 `$SECONDS` 경과가 노브 초에 닿으면 끝(R2F-PK · 첫 120회는 시계를 보지 않는다 — 어느 플랫폼에서도 기본보다 짧아지지 않는다) ·
                          사전 검사 `ready` 는 env 무관 120 고정 · 가짜 시계(핑 목이 SECONDS 를 민다)로 기한을 결정론으로 잰다
  C. dept_reserve_grace— 노브를 12 보다 올렸을 때만 create 예약 유예 = max(RESERVE_GRACE 또는 25, 노브+17)(R2F-PK — 벽시계 상한이 생겨 종전의 `노브/5` 항은 근거가 없어졌다) · 노브를 올렸는데 RESERVE_GRACE 가 정수가 아니면
                          stderr 경고 1줄(기본 경로에서는 경고 없음) · 노브가 기본이면 종전 인라인 값(`${CYS_DEPT_RESERVE_GRACE:-25}`) 그대로(명시한 낮은 값도) + 실흐름 분기 핀
  D. @stage 표지       — allocate·create 의 stderr 1줄 기계 판독 표지(순서 불변식 · stdout 0회 · launch 무변경) · **`spawn` 표지의 유무는 화면이 '새 팀'을 판정하는 계약**이다(재사용 갈래는 내지 않는다)
  E. 실패 문구         — 접두 "데몬 기동 실패" 유지 + (대기 예산 N초 · 실제 약 M초 · 로그 경로 · 현재 값 기준 노브 안내 — R2F-PK) · 스트림은 각 줄 종전 그대로 + launch·rotate 의 같은 꼬리는 stderr 에도 한 줄(가산)
  F. census            — 사전 검사 3곳은 `ready` · 스폰 뒤 3곳만 `ready_wait` · 표지 소재지 · bash 3.2/MSYS 안전 문법 · SECONDS 대입 금지 · 재기록 호출 지점
  G. 예약 재기록·reap  — (R2F-PK) 데몬을 띄우기 직전 `reserved_at` 을 한 번 다시 찍는다(create·allocate · 최선 노력 · 등재를 새로 만들지 않는다) · allocate 의 예약에도 `reserved_at` · reap 의 '죽은 등록' 갈래 부팅 유예 ·
                          기본 경로 출력은 종전과 같다

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
            if l.rstrip().endswith("}") and "<<" not in l:   # 첫 줄이 `}` 로 끝나도 히어독을 열면 여러 줄 함수다(예 reg_set_field 의 `|| { …; }` 꼬리)
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
    if [ -n "${STUB_PING_RAT:-}" ]; then
      rat="$(grep -o '"reserved_at": *[0-9.eE+-]*' "$CYS_DEPTS_JSON" 2>/dev/null | head -1 | sed 's/.*: *//')"
      echo "ping $CYS_SOCKET rat=${rat:-none}" >> "@PINGS@"
    else
      echo "ping $CYS_SOCKET" >> "@PINGS@"
    fi
    if [ -n "${STUB_BACKDATE_AT:-}" ]; then
      c="@STATE@/backdate.count"; bn=0; [ -f "$c" ] && read -r bn < "$c"
      bn=$((bn + 1)); echo "$bn" > "$c"
      if [ "$bn" -eq "$STUB_BACKDATE_AT" ]; then
        python3 - "$CYS_DEPTS_JSON" "${STUB_BACKDATE_S:-40}" <<'PYB'
import json, sys
p, s = sys.argv[1], float(sys.argv[2])
d = json.load(open(p, encoding="utf-8"))
for e in d["depts"].values():
    if "reserved_at" in e:
        e["reserved_at"] -= s
json.dump(d, open(p, "w", encoding="utf-8"), ensure_ascii=False)
PYB
      fi
    fi
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
# 재기록 호출(`reg_restamp` 의 파이썬 — argv 에 `restamp:<종류>` 표지가 있다)만 실패시키는 가짜 python3 — 그 밖의 호출은 진짜 인터프리터로 넘긴다(stdin 의 히어독 그대로 전달).
PYTHON_WRAP = "#!/bin/sh\ncase \" $* \" in\n  *\" restamp:\"*) echo \"[mock python3] 재기록 호출 실패 모사\" >&2; exit \"${STUB_RESTAMP_RC:-1}\" ;;\nesac\nexec \"@REALPY@\" \"$@\"\n"
# ★결정론(부하 경주 차단): 목 sleep 이 no-op 이라 `ready_wait` 120회는 수백 ms 에 끝난다 — 백그라운드로 뜬 목 cysd 가 그 안에 소켓 파일을 못 만들면
#   (부하·nice·샌드박스 exec 지연) 성공 경로가 "데몬 기동 실패"로 뒤집힌다(실측 2회: 부하 중 실행 — 자식이 수 초 늦게 뜬다). 그래서 **성공해야 하는** 신규
#   스폰 시나리오는 소켓 파일이 아니라 핑 횟수로 '데몬이 떴다'를 정한다 — 소켓별 130 번째 핑부터 응답(번호 점유 확인 1 + 사전 검사 120 은 실패 · 스폰 뒤 대기 10번째 핑에서
#   성공). 같은 이유로 목 cysd 의 호출 기록(백그라운드 자식이 쓴다)은 어떤 단언에도 쓰지 않고, 스폰 여부는 동기 증거(`Sandbox.spawned` — 부모 셸이 만드는 cysd.log)로 본다.
UP_AT_PING = "130"


class Sandbox(object):
    def __init__(self, restamp_fail=False, **env_extra):
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
        if restamp_fail:   # 재기록만 실패 — python3 를 가짜로 덮는다(실패 시나리오 전용 · 기본은 진짜 인터프리터 그대로)
            _write_exec(os.path.join(bindir, "python3"), PYTHON_WRAP.replace("@REALPY@", shutil.which("python3") or sys.executable))
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

    def ping_rats(self):
        """STUB_PING_RAT 로 남긴 핑별 reserved_at — [(핑 번호(1부터), 값 문자열 또는 'none')] · 값이 같은 구간 = 그동안 등재 시각이 안 바뀌었다."""
        out = []
        for i, l in enumerate(_read(self.pings).splitlines() if os.path.exists(self.pings) else [], 1):
            m = re.search(r" rat=(\S+)$", l)
            out.append((i, m.group(1) if m else "none"))
        return out

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


class ReadyWallClock(unittest.TestCase):
    """B(R2F-PK): 스폰 뒤 대기 = 횟수 상한(초×10) **그리고** 121번째 반복부터 `$SECONDS` 경과가 노브 초에 닿으면 끝. 핑 목이 `SECONDS` 를 대입으로 밀어
    '느린 기계'를 흉내 낸다(실제로 기다리지 않는다 · bash 3.2 에서 동작 — 4번째 핑마다 +1초 = 윈도우 11 러너(반복 ≈ 0.237초) 모양 · 8번째마다 = 맥(≈ 0.118초) 모양).
    기존 횟수 핀(ReadyBudget — 목 sleep 이 무동작이라 시계가 안 가므로 횟수로 끝난다)은 그대로 통과해야 하고, 이 클래스가 기한 쪽을 잰다."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("ready", "ready_wait", "dept_ready_secs"))   # 기존 ReadyBudget 과 같은 세 함수 — ready_wait 는 자기완결·무소음이다

    def run_wait(self, knob, every=1, step=1, ok_at=0, call="ready_wait sock"):
        # 핑 k 번째마다(k % every == 0) 시계를 step 초 민다. SECONDS 대입은 이 가짜 핑에만 있다(제품 코드는 대입하지 않는다 — Census).
        script = ("set -u\n" + self.funcs + "sleep(){ :; }\nCOUNT=0\n"
                  'cys_probe(){ COUNT=$((COUNT + 1)); if [ $((COUNT % EVERY)) -eq 0 ]; then SECONDS=$((SECONDS + STEP)); fi; '
                  '[ "${OK_AT:-0}" -gt 0 ] && [ "$COUNT" -ge "$OK_AT" ]; }\n'
                  "CYS=cys_probe\n" + call + '\nrc=$?\necho "rc=$rc count=$COUNT elapsed=${_READY_WAIT_SECS:-none}"\n')
        env = {"OK_AT": str(ok_at), "EVERY": str(every), "STEP": str(step)}
        if knob is not None:
            env["CYS_DEPT_READY_SECS"] = knob
        rc, out, err = bash_eval(script, env)
        self.assertEqual((rc, err), (0, ""), "하네스 오류: %r" % err)
        m = re.fullmatch(r"rc=(\d+) count=(\d+) elapsed=(\d+|none)\n", out)
        self.assertIsNotNone(m, "출력 형식: %r" % out)
        return int(m.group(1)), int(m.group(2)), m.group(3)

    def test_deadline_cuts_after_the_first_120_iterations(self):
        # 1초/핑(아주 느린 기계): 120회까지는 시계를 보지 않는다 — 노브가 13·60 이어도 120회를 채우고, 121번째 반복에서 기한이 이미 지나 있어 거기서 끊는다(횟수 상한 130·600 에 못 미친다).
        for knob in ("13", "60"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                rc, count, _el = self.run_wait(knob, every=1, step=1)
                self.assertEqual((rc, count), (1, 121), "120회 뒤 첫 판정(121번째)에서 기한이 끊어야 한다")
        # 기한이 121번째 이후에 있으면 기한(노브 초)에서 끊는다 — 횟수 상한(1800)보다 한참 일찍. 초 경계를 걸치는 실행을 위해 ±1.
        rc, count, _el = self.run_wait("180", every=1, step=1)
        self.assertEqual(rc, 1)
        self.assertIn(count, (179, 180), "노브 180·1초/핑: 기한(180초)에서 끊겨야 한다(횟수 상한은 1800)")

    def test_knobs_13_to_28_are_never_shorter_than_the_default(self):
        # 윈도우 11 러너 모양(4번째 핑마다 +1초): 첫 120회가 이미 30초 — 기한(13~28초)은 121번째 시점에 지나 있다. 시계를 처음부터 봤다면(순수 기한판) 노브 13~28 은 13~28초(약 52~112회)에서 끝나
        #   기본(120회)보다 **짧아진다**(비단조 — "느려서 올렸는데 더 빨리 포기"). 첫 120회는 시계를 보지 않으므로 어느 값도 기본보다 짧지 않다.
        base = self.run_wait(None, every=4, step=1)
        self.assertEqual(base[:2], (1, 120), "노브 미설정 = 기본 120회")
        for knob in range(13, 29):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                rc, count, _el = self.run_wait(str(knob), every=4, step=1)
                self.assertEqual(rc, 1)
                self.assertGreaterEqual(count, base[1], "노브 %d 가 기본(%d회)보다 짧아졌다 — 첫 120회에 시계를 본다" % (knob, base[1]))
                self.assertEqual(count, 121, "첫 120회 뒤 첫 판정에서 끊겨야 한다")

    def test_count_is_monotone_in_the_knob_on_both_machine_shapes(self):
        # 노브를 올릴수록 포기까지의 핑 수가 줄지 않는다(윈도우 모양 · 맥 모양 · 아주 느린 모양) — 비단조 회귀(시계를 초반부터 보는 변이)를 한 번에 잡는다.
        for every in (1, 4, 8):
            counts = [(k, self.run_wait(str(k), every=every, step=1)[1]) for k in (12, 13, 14, 20, 28, 29, 30, 31, 40, 60, 100, 180)]
            with self.subTest(every=every):
                for (k0, c0), (k1, c1) in zip(counts, counts[1:]):
                    self.assertGreaterEqual(c1, c0 - 1, "노브 %d(%d회) → %d(%d회): 줄었다 — 비단조 %r" % (k0, c0, k1, c1, counts))
                self.assertEqual(counts[0][1], 120)

    def test_count_cap_still_binds_when_the_clock_barely_moves(self):
        # 시계가 거의 안 가는 기계(5000핑에 1초): 기한에 닿지 못해도 횟수 상한(노브 × 10)에서 끝난다 — 횟수 상한은 그대로다.
        for knob, want in (("13", 130), ("60", 600), ("180", 1800)):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertEqual(self.run_wait(knob, every=5000, step=1)[:2], (1, want))

    def test_default_path_never_reads_the_clock(self):
        # 노브 미설정·12 이하·무효: 반복이 120회를 넘지 않아 시계 판정이 아예 없다 — 시계가 폭주(핑마다 1000초)해도 120회를 다 채운다(기본 경로 = 종전과 같은 횟수).
        for knob in (None, "12", "11", "abc", "181", "", "600"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertEqual(self.run_wait(knob, every=1, step=1000)[:2], (1, 120))
        # 사전 검사 `ready` 는 노브·시계와 무관한 120회 고정이다.
        for knob in (None, "60", "180"):
            with self.subTest(precheck=knob):
                self.assertEqual(self.run_wait(knob, every=1, step=1000, call="ready sock")[:2], (1, 120))

    def test_success_after_120_under_the_deadline_still_returns_zero(self):
        rc, count, _el = self.run_wait("180", every=1, step=1, ok_at=150)
        self.assertEqual((rc, count), (0, 150), "기한 안(150초 < 180초)의 성공은 닿아야 한다")
        rc, count, _el = self.run_wait("60", every=1, step=1, ok_at=121)
        self.assertEqual((rc, count), (0, 121), "121번째 핑의 성공은 같은 반복의 기한 판정보다 먼저다(마지막 기회)")

    def test_elapsed_is_recorded_from_seconds_on_success_and_failure(self):
        # `_READY_WAIT_SECS` = 이번 대기의 `$SECONDS` 차이 — 실패 문구의 '실제 약 M초'가 읽는다. 시계가 +1초/핑이므로 핑 수만큼(초 경계 걸침 ±1).
        rc, count, el = self.run_wait("60", every=1, step=1)
        self.assertEqual((rc, count), (1, 121))
        self.assertIn(int(el), (121, 122))
        rc, count, el = self.run_wait("60", every=1, step=1, ok_at=30)
        self.assertEqual((rc, count), (0, 30))
        self.assertIn(int(el), (30, 31))


class ReadyKnobWarn(unittest.TestCase):
    """A(R2F-PK · S4 m4): 노브가 **비어 있지 않은데** 해석이 받아들이지 않은 값(범위 밖·비정수 → 12)이면 stderr 에 정확히 1줄. 미설정·빈 값·유효 값(선행 0 포함)은 경고 0(기본 경로 무변경).
    경고는 순수 함수(`dept_ready_secs` — 여러 번 불리고 DeptReadySecs 가 stderr 를 비어 있다고 못박는다)도, `ready_wait`(기존 횟수 핀 검체가 무효 노브에서도 stderr 가 비어 있다고 못박는다)도 아니고
    **스폰 뒤 대기의 호출 지점**(launch·allocate·create)이 `ready_wait` 직전에 부르는 `dept_ready_knob_warn` 의 몫이다 — 함수 단위(아래)와 흐름 단위(KnobWarnFlow)로 잰다."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("dept_ready_secs", "dept_ready_knob_warn"))

    def warn(self, knob, calls=1):
        script = "set -u\n" + self.funcs + "dept_ready_knob_warn\n" * calls + "echo done\n"
        env = {} if knob is None else {"CYS_DEPT_READY_SECS": knob}
        return bash_eval(script, env)

    WARN = ["300", "abc", "181", "0", "11", "-5", "30.5", " 60", "60 ", "1e2", "0x20", "+60", "99999999999999999999", "0181", "600"]
    QUIET = [None, "", "12", "13", "60", "179", "180", "012", "060", "00012", "00180"]

    def test_invalid_nonempty_value_warns_exactly_once_on_stderr(self):
        for val in self.WARN:
            with self.subTest(CYS_DEPT_READY_SECS=val):
                rc, out, err = self.warn(val)
                self.assertEqual((rc, out), (0, "done\n"), "경고가 stdout·종료코드에 영향을 줬다")
                lines = err.splitlines()
                self.assertEqual(len(lines), 1, "경고는 정확히 1줄이어야 한다: %r" % err)
                self.assertTrue(lines[0].startswith("[cys-dept] WARN: CYS_DEPT_READY_SECS='%s' " % val), lines[0])
                self.assertIn("12", lines[0])

    def test_unset_empty_and_valid_values_are_silent(self):
        for val in self.QUIET:
            with self.subTest(CYS_DEPT_READY_SECS=val):
                self.assertEqual(self.warn(val), (0, "done\n", ""), "기본 경로·유효 값에서 경고가 났다")

    def test_warns_once_per_process_even_if_called_again(self):
        rc, out, err = self.warn("300", calls=3)
        self.assertEqual(len(err.splitlines()), 1, "같은 프로세스에서 경고가 되풀이됐다: %r" % err)

    def test_value_is_folded_to_one_short_line(self):
        rc, out, err = self.warn("12\n13")
        self.assertEqual(len(err.splitlines()), 1, "개행이 든 값이 경고를 여러 줄로 쪼갰다: %r" % err)
        self.assertIn("'12 13'", err)
        rc, out, err = self.warn("x" * 200)
        self.assertEqual(len(err.splitlines()), 1)
        self.assertIn("'" + "x" * 40 + "'", err)
        self.assertNotIn("x" * 41, err, "값은 40자까지만 싣는다")

    def test_pure_interpreter_and_ready_wait_stay_silent(self):
        # 경고를 순수 해석 함수 안에 넣으면(여러 번 불린다) 한 실행에서 되풀이되고, ready_wait 안에 넣으면 기존 횟수 핀의 `err == ""` 가 깨진다 — 둘 다 여기서 못박는다.
        src = _read(DEPT)
        fns = "".join(func_text(src, n) for n in ("ready", "ready_wait", "dept_ready_secs"))
        rc, out, err = bash_eval("set -u\n" + fns + "sleep(){ :; }\ncys_probe(){ return 1; }\nCYS=cys_probe\ndept_ready_secs; ready_wait sock; echo rc=$?\n", {"CYS_DEPT_READY_SECS": "300"})
        self.assertEqual((rc, out, err), (0, "12\nrc=1\n", ""))


class KnobWarnFlow(unittest.TestCase):
    """A(R2F-PK): 호출 지점 세 곳(launch·allocate·create)이 스폰 뒤 대기 직전에 경고를 낸다 — 실흐름에서 무효 노브는 정확히 1줄(stderr · stdout 오염 없음 · 흐름 무변경), 기본·유효 값은 0줄,
    스폰이 없는 재사용 갈래(사전 검사가 이미 켜진 데몬을 찾음)에서는 기다리지 않으므로 경고도 없다."""

    WARN_RE = re.compile(r"^\[cys-dept\] WARN: CYS_DEPT_READY_SECS='300' ")

    def flow(self, args, knob, **kw):
        sb = Sandbox(STUB_PING_OK_FROM=kw.pop("ok_from", UP_AT_PING), **({"CYS_DEPT_READY_SECS": knob} if knob is not None else {}))
        self.addCleanup(sb.cleanup)
        if args[0] == "create":
            sb.seed_catalog("k1", "m1")
            sb.write_reg({})
        return sb, sb.run(*args)

    def warns(self, err):
        return [l for l in err.splitlines() if "CYS_DEPT_READY_SECS=" in l and "WARN" in l]

    def test_invalid_knob_warns_once_in_each_waiting_verb(self):
        for args in (("allocate",), ("create", "k1"), ("launch", "a")):
            with self.subTest(args[0]):
                sb, (rc, out, err) = self.flow(args, "300")
                self.assertEqual(rc, 0, out + err[-800:])
                w = self.warns(err)
                self.assertEqual(len(w), 1, "무효 노브 경고가 정확히 1줄이어야 한다:\n" + err[-1200:])
                self.assertRegex(w[0], self.WARN_RE)
                self.assertNotIn("READY_SECS", out, "경고가 stdout 으로 샜다(부서 이름·종전 stdout 계약 오염)")
                if args[0] != "launch":
                    self.assertEqual(last_line(out), "dept-1")

    def test_valid_and_default_knob_do_not_warn(self):
        for knob in (None, "12", "60", "060"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                sb, (rc, out, err) = self.flow(("allocate",), knob)
                self.assertEqual(rc, 0, err[-800:])
                self.assertEqual(self.warns(err), [], "기본·유효 노브에서 경고가 났다")

    def test_no_wait_means_no_warning(self):
        sb, (rc, out, err) = self.flow(("allocate",), "300", ok_from="2")   # 사전 검사가 이미 켜진 데몬을 찾는다 → 스폰도 대기도 없다
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(self.warns(err), [], "기다리지 않았는데 노브 경고가 났다(경고는 스폰 뒤 대기 직전의 것이다)")

    def test_invalid_knob_failure_flow_keeps_budget_12_and_warns_before_the_failure_line(self):
        sb = Sandbox(STUB_CYSD_MODE="dead", CYS_DEPT_READY_SECS="300")
        self.addCleanup(sb.cleanup)
        rc, out, err = sb.run("allocate")
        self.assertEqual(rc, 1)
        self.assertEqual(len(self.warns(err)), 1)
        lines = err.splitlines()
        wi = next(i for i, l in enumerate(lines) if "CYS_DEPT_READY_SECS=" in l and "WARN" in l)
        fi = next(i for i, l in enumerate(lines) if "데몬 기동 실패" in l)
        self.assertLess(wi, fi, "경고가 실패 줄보다 앞에 있어야 한다(실패 문구 앞 300자에 사유가 보이도록 짧게 앞에)")
        self.assertIn("대기 예산 12초", lines[fi], "무효 노브(300)는 12 로 읽혀 예산 12초로 적힌다")
        self.assertLess(len(lines[wi]), 130, "경고 줄이 길어 실패 사유를 300자 밖으로 밀어낸다")


class ReadyFailNote(unittest.TestCase):
    """E(R2F-PK · S4 m4): 실패 꼬리 = `(대기 예산 N초 · 실제 약 M초 · 로그: <디렉터리>/cysd.log · 안내)`. N = 노브(공칭 상한) · M = `_READY_WAIT_SECS`(`$SECONDS` 차이 — 대기 동작은 안 바뀐다) ·
    안내 예시는 **현재 값 기준**: 60 미만 = 종전 그대로 `=60` · 60 이상 = 그 두 배(상한 180) · 이미 상한 = 늘릴 수 없다(이미 60 이상으로 올린 사용자에게 '=60 처럼 늘릴 수 있다'고 안내하지 않는다)."""

    TAIL_LOW = "느린 디스크라면 CYS_DEPT_READY_SECS=60 처럼 대기 예산을 늘릴 수 있다"
    CASES = [
        (None, "12", TAIL_LOW), ("12", "12", TAIL_LOW), ("13", "13", TAIL_LOW), ("30", "30", TAIL_LOW), ("59", "59", TAIL_LOW),
        ("60", "60", "CYS_DEPT_READY_SECS=120 처럼 더 늘릴 수 있다 · 상한 180초"),
        ("89", "89", "CYS_DEPT_READY_SECS=178 처럼 더 늘릴 수 있다 · 상한 180초"),
        ("90", "90", "CYS_DEPT_READY_SECS=180 처럼 더 늘릴 수 있다 · 상한 180초"),
        ("100", "100", "CYS_DEPT_READY_SECS=180 처럼 더 늘릴 수 있다 · 상한 180초"),
        ("179", "179", "CYS_DEPT_READY_SECS=180 처럼 더 늘릴 수 있다 · 상한 180초"),
        ("180", "180", "대기 예산이 이미 상한(180초)이다 — 더 늘릴 수 없으니 로그를 확인하라"),
        # 무효 값(범위 밖·비정수)은 12 로 읽힌다 → 예산 12 · 종전 안내
        ("300", "12", TAIL_LOW), ("181", "12", TAIL_LOW), ("abc", "12", TAIL_LOW), ("11", "12", TAIL_LOW),
    ]

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("dept_ready_secs", "ready_fail_note"))

    def note(self, knob, elapsed="7"):
        pre = "" if elapsed is None else "_READY_WAIT_SECS=%s\n" % elapsed
        script = ("set -u\n" + self.funcs + "dept_logdir(){ printf '%s' \"/L/cys-dept-$1\"; }\n" + pre + 'ready_fail_note nm\n')
        env = {} if knob is None else {"CYS_DEPT_READY_SECS": knob}
        rc, out, err = bash_eval(script, env)
        self.assertEqual((rc, err), (0, ""))
        return out

    def test_note_table(self):
        for knob, budget, tail in self.CASES:
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertEqual(self.note(knob), " (대기 예산 %s초 · 실제 약 7초 · 로그: /L/cys-dept-nm/cysd.log · %s)" % (budget, tail))

    def test_raised_knob_users_are_not_told_to_use_60(self):
        for knob in ("60", "75", "90", "120", "179", "180"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertNotIn("CYS_DEPT_READY_SECS=60 ", self.note(knob), "이미 60 이상으로 올린 사용자에게 '=60 처럼 늘릴 수 있다'고 안내했다")

    def test_elapsed_comes_from_the_wait_not_from_the_knob(self):
        self.assertIn("대기 예산 12초 · 실제 약 28초", self.note(None, "28"))
        self.assertIn("대기 예산 180초 · 실제 약 211초", self.note("180", "211"))
        self.assertIn("실제 약 ?초", self.note(None, None), "대기가 시작되지 않았다면 경과는 모른다고 적는다(지어내지 않는다)")

    def test_legacy_prefix_words_are_gone(self):
        self.assertNotIn("소켓 대기", self.note(None), "종전 문구('소켓 대기 N초' — N 은 노브 값이지 경과가 아니었다)가 남았다")


# ════════════════════════════════════════════════════════════════════════════════════
# C. dept_reserve_grace — 예약 유예 결합
# ════════════════════════════════════════════════════════════════════════════════════
class ReserveGraceUnit(unittest.TestCase):
    # (RESERVE_GRACE, READY_SECS, 파이썬 CYS_GRACE 로 넘어갈 값). None = 미설정.
    # master 결정(GP 검토): 유예 올림은 **노브를 12 보다 크게 올렸을 때만**이다 — 노브가 12 이하(미설정·명시 12·무효 값 → 12)면 종전 인라인 식 값 그대로.
    # 0.14.43 성찰(R1F-PK · S4 m1): 올림의 식이 `노브 + 13` → `노브 + 노브/5 + 17`(정수 나눗셈)로 바뀌었다 — 반복 1회가 0.1초가 아니라 약 0.118초라 `노브 + 13` 은
    #   실제 대기(사전 검사 + 노브 × 1.18)보다 짧았다.
    # ★2회차(R2F-PK · 티켓 §2): 스폰 뒤 대기가 벽시계로 묶이고(ready_wait — 121번째 반복부터 `$SECONDS` 기한) 생성자가 스폰 직전에 `reserved_at` 을 다시 찍으므로 `노브/5` 항의 근거가 없어졌다 →
    #   `노브 + 17`(17 = 재기록 → 스폰 사이와 첫 120회의 여유). **노브 ≤ 12 는 종전 값 그대로**(아래 기본 경로 행 · 동치 검체). 기대값은 식에서 손으로 계산했다:
    #   13→30 · 14→31 · 15→32 · 60→77 · 100→117 · 180→197.
    CASES = [
        # ── 노브 기본: 종전 `${CYS_DEPT_RESERVE_GRACE:-25}` 그대로 — 명시한 낮은 값도 올리지 않는다(종전 보존 핀)
        (None, None, "25"), ("10", None, "10"), ("0", None, "0"), ("10", "12", "10"), ("0", "12", "0"), ("40", "12", "40"),
        ("25", "12", "25"), ("26", "12", "26"), ("08", None, "08"), ("10", "abc", "10"), ("10", "601", "10"), ("10", "11", "10"),
        (None, "abc", "25"), (None, "601", "25"),
        # ── m2: 상한(180)을 넘어 12 로 접힌 노브도 기본 경로다 — 종전에 유효했던 600·300·181 도 유예는 인라인 식 그대로
        (None, "181", "25"), ("10", "181", "10"), ("10", "600", "10"), (None, "300", "25"), ("2.5", "600", "2.5"),
        # ── 노브를 12 보다 크게 올린 경우: max(설정값 또는 25, 노브 + 17)  (R2F-PK — 종전 `노브 + 노브/5 + 17` 아님)
        (None, "60", "77"), ("10", "60", "77"), ("100", "60", "100"), ("73", "60", "77"), ("74", "60", "77"), ("30", "60", "77"),
        ("08", "60", "77"), (None, "13", "30"), ("25", "13", "30"), ("30", "13", "30"), ("31", "13", "31"), ("32", "13", "32"),
        ("33", "13", "33"), (None, "14", "31"), (None, "15", "32"), (None, "100", "117"),
        # 경계(식의 정확한 값 — 한 칸 아래는 올리고 · 같거나 위는 그대로)
        ("76", "60", "77"), ("77", "60", "77"), ("78", "60", "78"), ("88", "60", "88"), ("89", "60", "89"), ("90", "60", "90"),
        (None, "180", "197"), ("196", "180", "197"), ("197", "180", "197"), ("198", "180", "198"), ("232", "180", "232"), ("233", "180", "233"),
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
        self.sb.env["CYS_DEPT_READY_SECS"] = "60"   # 유예 = 60 + 17 = 77(R2F-PK · 종전 60 + 60/5 + 17 = 89)
        self.sb.seed_entry("dept-1", "m1", age=40)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "노브 60 이면 유예 77 — 나이 40 은 아직 부팅 중으로 믿어야 한다(결합 소실 = 중복 기동 경합)")
        self.assertEqual(out.strip().splitlines()[-1], "dept-1")
        self.assertNotIn("cysd spawn", self.sb.read_calls(), "유예 안인데 데몬을 다시 띄웠다")
        self.assertFalse(self.sb.spawned("dept-1"), "유예 안인데 스폰 리다이렉트가 만들어졌다")
        self.assertEqual(stages(err), ["reserve", "done"])

    def test_knob_grace_has_an_upper_edge(self):
        self.sb.env["CYS_DEPT_READY_SECS"] = "60"
        self.sb.env["STUB_PING_OK_FROM"] = "2"
        self.sb.seed_entry("dept-1", "m1", age=84)   # 77 밖(7초 — 나이는 시간이 갈수록 커지므로 '밖' 쪽은 흔들리지 않는다 · R2F-PK: 종전 식의 89 밖 96 에서 옮겼다)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)", err, "유예(77)를 넘긴 등재는 REUSE_DEAD 여야 한다(영구 신뢰 금지)")

    def test_knob_grace_is_knob_plus_17_and_not_the_old_per_iteration_formula(self):
        # R2F-PK(티켓 §2): 스폰 뒤 대기가 벽시계로 묶였고(ready_wait) 생성자가 스폰 직전에 시각을 다시 찍으므로, 유예는 `노브 + 노브/5 + 17`(반복당 0.118초 초과분까지 덮으려던 종전 식 — 노브 180 에서 233)이 아니라
        #   `노브 + 17`(180 → 197)이다. 종전 이름 test_knob_grace_covers_the_measured_per_iteration_cost 는 그 옛 근거를 핀했다 — 계약이 바뀌어 새 근거로 바꿨다(약화가 아니라 교체: 옛 식으로 되돌리면 아래 둘째 단언이 붉다).
        self.sb.env["CYS_DEPT_READY_SECS"] = "180"
        self.sb.seed_entry("dept-1", "m1", age=185)   # 유예 197 안(여유 12초)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "노브 180 의 유예(197)가 나이 185 를 덮지 못했다")
        self.assertFalse(self.sb.spawned("dept-1"), "유예 안인데 스폰 리다이렉트가 만들어졌다")
        self.assertEqual(stages(err), ["reserve", "done"])
        sb2 = Sandbox(CYS_DEPT_READY_SECS="180", STUB_PING_OK_FROM="2")
        try:
            sb2.seed_catalog("k1", "m1")
            sb2.seed_entry("dept-1", "m1", age=215)   # 197 밖(18초) — 종전 식(233)이면 아직 안(BOOTING)이라 이 단언이 옛 식을 잡는다
            rc, out, err = sb2.run("create", "k1")
            self.assertEqual(rc, 0, err[-800:])
            self.assertIn("등록부 잔존(dept-1)·데몬 사망(grace 경과)", err, "나이 215 가 유예(197) 밖인데 BOOTING 으로 읽혔다 — `노브/5` 항이 되살아났다")
        finally:
            sb2.cleanup()

    def test_explicit_larger_grace_is_honored(self):
        # n8(S4): 종전 검체는 나이 90 vs 유예 100 이라 안쪽 여유가 10초뿐이었다(부하 큰 러너에서 예약까지 10초를 넘기면 흔들림). 나이를 60 으로 내려 여유 40초.
        #   단언의 뜻(명시한 더 큰 유예가 노브 유예로 덮이지 않는다)을 지키려면 노브 유예가 나이보다 작아야 한다 — 1회차 식에서 노브 60 의 유예는 89 라 나이 60 은
        #   둘 다 안쪽이 되어 변별력을 잃었고 노브를 30 으로 뒀다. R2F-PK 식(노브 + 17)에서도 노브 30 의 유예는 47: 나이 60 은 노브 유예(47) 밖(13초) · 명시 유예(100) 안(여유 40초).
        self.sb.env.update({"CYS_DEPT_RESERVE_GRACE": "100", "CYS_DEPT_READY_SECS": "30"})   # max(100, 47) = 100
        self.sb.seed_entry("dept-1", "m1", age=60)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err, "더 큰 명시 유예(100)를 노브 유예(47)로 덮어썼다")

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
        # 노브를 올리면(60) 같은 명시 저값(10)도 노브 + 17 = 77 로 올라 나이 15 를 아직 부팅 중으로 믿는다(중복 기동 경합의 틈을 줄인다)
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
                    sb.seed_entry("dept-1", "m1", age=5)   # 모든 조합의 유예(25·30.5·77)보다 20초 이상 안쪽
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
        self.assertEqual(stages(err), ["reserve", "probe", "up", "done"],
                         "재사용 분기에서 spawn·wait 를 내면 안 된다 — 화면(ui/src/main.ts pendingSpawned)은 `spawn` 표지의 유무로 '이 호출이 새 데몬을 띄웠다(새 팀)'를 판정한다: "
                         "재사용 호출이 spawn 을 내면 기존 팀에 '새 팀을 만들었다' 안내가 뜬다(cys-dept dept_stage 주석 계약 · R2F-PK)")
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
        self.assertEqual(stages(err), ["reserve", "probe", "up", "seat", "done"],
                         "재사용 분기에서 spawn·wait 를 내면 안 된다 — 화면은 `spawn` 표지의 유무로 '새 데몬을 띄운 호출(새 팀)'을 판정한다(cys-dept dept_stage 주석 계약 · R2F-PK)")
        self.assertIn("dept-1 이미 가동 — 재사용", err)
        assert_not_spawned(self, self.sb, "dept-1")


# ════════════════════════════════════════════════════════════════════════════════════
# E. 스폰 뒤 대기 실패 — 문구 · 표지 · 회수 · 대기 예산(실흐름 핑 횟수)
# ════════════════════════════════════════════════════════════════════════════════════
OLD_PREFIX = "[cys-dept] ERROR: %s 데몬 기동 실패"
NOTE_TAIL = "/cysd.log · 느린 디스크라면 CYS_DEPT_READY_SECS=60 처럼 대기 예산을 늘릴 수 있다)"
# R2F-PK(S4 m4): 꼬리의 숫자는 둘이다 — `대기 예산 N초`(노브 · 공칭 상한) + `실제 약 M초`(이번 스폰 뒤 대기의 $SECONDS 차이). 종전 `소켓 대기 N초` 의 N 은 노브 값이지 경과가 아니었다.
NOTE_HEAD = re.compile(r" \(대기 예산 (\d+)초 · 실제 약 (\d+)초 · 로그: ")


def note_budget(tc, note, want):
    """꼬리 머리를 새 꼴로 확인하고 '대기 예산' 숫자가 기대(노브 값)와 같은지 본다 — 돌려주는 값은 '실제 약 M초'."""
    m = NOTE_HEAD.match(note)
    tc.assertIsNotNone(m, "꼬리 문구가 `(대기 예산 N초 · 실제 약 M초 · 로그: …` 꼴이 아니다: %r" % note)
    tc.assertEqual(m.group(1), want, "꼬리의 '대기 예산' 은 노브 값이어야 한다: %r" % note)
    tc.assertNotIn("소켓 대기", note, "종전 문구('소켓 대기 N초' — N 은 노브 값이지 경과가 아니었다)가 남았다: %r" % note)
    return int(m.group(2))


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
        note_budget(self, note, "12")
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
        note_budget(self, note, "30")
        self.assertIn(b["pings"] - 300, (120, 121, 122, 123), "총 핑 %d — 사전 검사(120 고정) + 스폰 뒤 300 이 아니다" % b["pings"])

    def test_create_failure_message_stage_reclaim(self):
        b = self.box["create"]
        self.assertEqual(b["rc"], 1)
        note = self.note_of(b["err"], "dept-1")
        note_budget(self, note, "12")
        self.assertTrue(note.endswith(NOTE_TAIL), note)
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
        note_budget(self, note, "12")
        self.assertTrue(note.endswith(NOTE_TAIL), note)
        self.assertIn("로그: %s/cysd.log" % b["sb"].logdir("a"), note)
        # ★R2F-PK(S4 m7): 앱은 launch·rotate 실패에서 stderr 만 화면에 올린다 → 같은 꼬리를 stderr 에도 **정확히 1줄** 더 낸다(가산 — 위 stdout 줄·순서·그 핀은 그대로).
        #   종전 단언(`stderr 에 없다`)은 stdout 줄이 stderr 로 '옮겨 갔다(스트림 변경)'를 막으려던 것이다 — 이제 stdout 줄은 그대로 있는 채 같은 줄의 **복사본**이 stderr 에 한 번 있다:
        #   (이름을 바꾸지 않고 단언만 새 계약으로 바꿨다 — 약화가 아니라 '양쪽에 정확히 한 번씩'으로 더 엄격하다)
        err_hits = [l for l in b["err"].splitlines() if l.startswith(pre)]
        self.assertEqual(err_hits, out_hits, "launch 실패 꼬리가 stderr 에 stdout 줄과 같은 꼴로 정확히 1줄 있어야 한다:\nerr=%s" % b["err"][-800:])
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
        self.assertRegex(err, r" \(대기 예산 12초 · 실제 약 \d+초 · 로그: ")
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
# G. 예약 재기록(reg_restamp) · allocate 예약의 reserved_at · reap 의 부팅 유예 (R2F-PK · 티켓 §2)
# ════════════════════════════════════════════════════════════════════════════════════
def runs_of(sb):
    """핑 순서대로 등재의 reserved_at 값이 바뀐 구간 — [[첫 핑 번호, 마지막 핑 번호, 값]]. 값 'none' = 그 순간 등재에 시각이 없었다(예약 전)."""
    runs = []
    for i, v in sb.ping_rats():
        if runs and runs[-1][2] == v:
            runs[-1][1] = i
        else:
            runs.append([i, i, v])
    return runs


def stamped(runs):
    return [r for r in runs if r[2] != "none"]


class RestampFlow(unittest.TestCase):
    """생성자가 **데몬을 띄우기 직전에** 자기 예약의 `reserved_at` 을 한 번 다시 찍는다 — 예약은 사전 검사(맥 약 13초 · 윈도우 약 28초) **전**에 찍히므로, 재기록이 없으면 예약 → 소켓이 열리기까지가 윈도우에서
    약 39초(실측)라 기본 유예 25초보다 길다(같은 카탈로그 키를 25~39초 사이에 다시 만들면 데몬 중복 기동). 관측 도구: STUB_PING_RAT — 목 cys 가 핑마다 그 순간 등재의 reserved_at 을 pings.log 에 남긴다
    (사전 검사 120핑 동안은 같은 값 · 재기록 뒤 스폰 뒤 대기 핑부터 새 값 — 시점을 '핑 번호'로 결정론 판정한다)."""

    def new_sb(self, **kw):
        sb = Sandbox(STUB_PING_RAT="1", **kw)
        self.addCleanup(sb.cleanup)
        sb.seed_catalog("k1", "m1")
        return sb

    def assert_single_restamp_between_precheck_and_wait(self, sb, first_extra=0):
        """핑 번호 해석: [번호 점유 확인 핑(예약 전 · 값 없음 또는 시드 값)] + 사전 검사 120핑(예약 값 r0) + 스폰 뒤 대기 핑(재기록 값 r1) — 값은 정확히 r0 → r1 한 번."""
        runs = runs_of(sb)
        vals = stamped(runs)
        self.assertEqual(len(vals), 2 + first_extra, "예약 시각이 바뀐 구간이 %d 개여야 한다(r0 → r1 한 번의 재기록): %r" % (2 + first_extra, runs))
        (a0, b0, r0), (a1, b1, r1) = vals[-2], vals[-1]
        self.assertEqual(b0 - a0 + 1, 120, "사전 검사 120핑 동안 예약 시각이 그대로여야 한다(재기록이 사전 검사 **전**이나 도중에 일어났다): %r" % runs)
        self.assertEqual(a1, b0 + 1, "재기록은 사전 검사 마지막 핑 **바로 뒤**(= 스폰 직전)에 일어나야 한다: %r" % runs)
        self.assertGreater(float(r1), float(r0) + 0.05, "재기록한 시각이 사전 검사를 지난 뒤의 시각이어야 한다: r0=%s r1=%s" % (r0, r1))
        return float(r0), float(r1)

    def test_create_new_restamps_exactly_once_right_before_spawn(self):
        sb = self.new_sb(STUB_PING_OK_FROM=UP_AT_PING)
        sb.write_reg({})
        rc, out, err = sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        r0, r1 = self.assert_single_restamp_between_precheck_and_wait(sb)
        self.assertEqual(sb.read_reg()["dept-1"]["reserved_at"], r1, "등재의 최종 reserved_at 이 재기록한 값이어야 한다")
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "seat", "done"], "재기록은 표지 순서를 바꾸지 않는다")
        self.assertEqual([l for l in err.splitlines() if "재기록" in l], [], "성공한 재기록은 아무것도 내지 않는다(기본 경로 출력 무변경)")

    def test_create_reuse_dead_restamps_exactly_once_right_before_spawn(self):
        sb = self.new_sb(STUB_PING_OK_FROM=UP_AT_PING)
        sb.seed_entry("dept-1", "m1", age=100)   # 소켓 없음 + 유예(25초) 밖 → REUSE_DEAD(재기동)
        rc, out, err = sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)·데몬 사망(grace 경과)", err)
        runs = runs_of(sb)
        self.assertNotEqual(runs[0][2], "none", "첫 핑(생사 확인)은 시드된 옛 시각을 본다: %r" % runs)
        # 시드 옛 값 → 재청구 값(r0 · 사전 검사 120핑) → 재기록 값(r1) — 세 구간
        self.assert_single_restamp_between_precheck_and_wait(sb, first_extra=1)

    def test_allocate_reservation_carries_reserved_at_and_restamps_once(self):
        sb = self.new_sb(STUB_PING_OK_FROM=UP_AT_PING)
        rc, out, err = sb.run("allocate")
        self.assertEqual(rc, 0, err[-800:])
        r0, r1 = self.assert_single_restamp_between_precheck_and_wait(sb)
        ent = sb.read_reg()["dept-1"]
        self.assertEqual(ent["reserved_at"], r1, "allocate 의 예약에도 reserved_at(가산 키)이 있고 재기록된 값이 남는다")
        self.assertEqual(sorted(ent), ["account_dir", "cwd", "pack_dir", "reserved_at", "role", "socket"], "allocate 등재의 키는 종전 + reserved_at 하나뿐이다")

    def test_allocate_team_proposal_restamps_once_for_its_own_proposal(self):
        import base64
        spec = {"v": 1, "id": "tp-20261003-0042", "display": "영상편집팀", "purpose": "유튜브 영상을 편집한다."}
        b64 = base64.urlsafe_b64encode(json.dumps(spec, ensure_ascii=False).encode("utf-8")).decode("ascii")
        sb = self.new_sb(STUB_PING_OK_FROM=UP_AT_PING)
        rc, out, err = sb.run("allocate", "--team-spec-b64", b64)
        self.assertEqual(rc, 0, err[-800:])
        self.assert_single_restamp_between_precheck_and_wait(sb)
        ent = sb.read_reg()["dept-1"]
        self.assertEqual(ent["team_proposal_id"], spec["id"])
        self.assertIn("reserved_at", ent)

    def test_no_restamp_when_the_precheck_finds_a_live_daemon(self):
        # 사전 검사가 이미 켜진 데몬을 찾으면 스폰이 없다 → 재기록도 없다(예약 시각은 예약 때 값 그대로 — 재사용 갈래는 건드리지 않는다)
        for verb, args in (("allocate", ("allocate",)), ("create", ("create", "k1"))):
            with self.subTest(verb):
                sb = self.new_sb(STUB_PING_OK_FROM="2")
                if verb == "create":
                    sb.write_reg({})
                rc, out, err = sb.run(*args)
                self.assertEqual(rc, 0, err[-800:])
                vals = stamped(runs_of(sb))
                self.assertEqual(len(vals), 1, "재사용 갈래에서 예약 시각이 바뀌었다: %r" % runs_of(sb))
                self.assertEqual(sb.read_reg()["dept-1"]["reserved_at"], float(vals[0][2]))
                assert_not_spawned(self, sb, "dept-1")

    def test_restamp_failure_does_not_stop_creation(self):
        # 재기록 호출만 실패(가짜 python3 가 `restamp:` 표지 호출에 exit 1) — 생성은 계속되고(최선 노력) 경고는 정확히 1줄이며 예약 시각은 사전 검사 때 값 그대로다.
        for verb, args in (("allocate", ("allocate",)), ("create", ("create", "k1"))):
            with self.subTest(verb):
                sb = self.new_sb(restamp_fail=True, STUB_PING_OK_FROM=UP_AT_PING)
                if verb == "create":
                    sb.write_reg({})
                rc, out, err = sb.run(*args)
                self.assertEqual(rc, 0, "재기록 실패가 생성을 막았다:\n" + err[-1200:])
                self.assertEqual(last_line(out), "dept-1")
                warns = [l for l in err.splitlines() if "재기록" in l and "WARN" in l]
                self.assertEqual(len(warns), 1, "재기록 실패 경고가 정확히 1줄이어야 한다:\n" + err[-1200:])
                self.assertIn("생성은 계속한다", warns[0])
                self.assertEqual(len(stamped(runs_of(sb))), 1, "재기록이 실패했는데 예약 시각이 바뀌었다")
                self.assertEqual(stages(err)[-1], "done")

    def test_following_call_is_booting_by_the_restamp_time_not_the_original_reservation_time(self):
        # 첫 호출(REUSE_DEAD · 데몬은 끝내 안 뜬다 → exit 1 · 등재는 보존)의 사전 검사 도중(60번째 핑)에 등재 시각을 120초 과거로 민다 = 사전 검사에 120초가 걸린 기계(윈도우는 약 28초 — 크게 잡아 부하에도
        #   흔들리지 않게 했다). 유예는 100초로 둔다: 재기록이 있으면 등재의 시각은 '스폰 직전'이라 방금(나이 = 스폰 뒤 대기 소요 W — 수 초)이고, 없으면 120초 전(원래 예약 시각 기준)이다.
        #   바로 뒤따른 호출: 원래 시각 기준으로는 유예(100초)가 지났지만 재기록 기준으로는 안 지났다(W < 100) → REUSE_BOOTING. 부하로 W 가 90초까지 늘어도 판정은 같다.
        sb = Sandbox(STUB_CYSD_MODE="dead", STUB_BACKDATE_AT="60", STUB_BACKDATE_S="120", CYS_DEPT_RESERVE_GRACE="100")
        self.addCleanup(sb.cleanup)
        sb.seed_catalog("k1", "m1")
        sb.seed_entry("dept-1", "m1", age=300)   # 유예(100초) 밖 → REUSE_DEAD(재기동)
        rc, out, err = sb.run("create", "k1")
        self.assertEqual(rc, 1, "첫 호출은 데몬이 안 떠 실패해야 한다(재기동 갈래):\n" + err[-800:])
        self.assertIn("등록부 잔존(dept-1)", err)
        ent = sb.read_reg().get("dept-1")
        self.assertIsNotNone(ent, "REUSE_DEAD 재기동 실패는 등재를 보존한다(종전 동작)")
        age = time.time() - ent["reserved_at"]
        self.assertLess(age, 90, "재기록된 시각이면 나이가 첫 호출의 스폰 뒤 대기 소요(수 초)여야 한다 — 120초 이상이면 재기록이 없다(%.1f초)" % age)
        del sb.env["STUB_BACKDATE_AT"]
        rc2, out2, err2 = sb.run("create", "k1")
        self.assertEqual(rc2, 0, err2[-800:])
        self.assertIn("생성자 부팅중(dept-1)", err2, "원래 예약 시각 기준이 아니라 재기록 기준으로 유예 안이어야 한다")
        self.assertEqual(stages(err2), ["reserve", "done"])


class RestampUnit(unittest.TestCase):
    """`reg_restamp` 함수 단위(실제 함수를 그대로 떼어 실행): 등재가 **아직 있고 내 예약일 때만** 같은 잠금 안에서 `reserved_at` 만 새로 찍는다 · 등재를 새로 만들지 않는다 · 모르는 키·다른 등재는 그대로 ·
    어떤 실패에도 종료 코드 0(최선 노력 — set -e 안전) + 경고 1줄."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = func_text(src, "reg_init") + func_text(src, "reg_restamp")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rs-")
        self.reg = os.path.join(self.tmp, "depts.json")

    def tearDown(self):
        os.chmod(self.tmp, 0o755)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def put(self, depts):
        with open(self.reg, "w", encoding="utf-8") as f:
            json.dump({"depts": depts, "_future_top_level": {"k": [1, 2]}}, f, ensure_ascii=False)

    def raw(self):
        with open(self.reg, "rb") as f:
            return f.read()

    def restamp(self, *args):
        import shlex
        script = "set -u\nREG=%s\n%sreg_restamp %s\necho rc=$?\n" % (shlex.quote(self.reg), self.funcs, " ".join(shlex.quote(a) for a in args))
        return bash_eval(script)

    def ent(self, name):
        return json.loads(self.raw().decode("utf-8"))["depts"][name]

    OLD = 1000.0

    def test_mine_is_restamped_and_everything_else_is_preserved(self):
        self.put({"d1": {"mission_key": "m", "reserved_at": self.OLD, "x_future": {"a": [1, {"b": None}]}},
                  "d2": {"mission_key": "other", "reserved_at": self.OLD, "role": "dept-master"}})
        rc, out, err = self.restamp("d1", "create", "m")
        self.assertEqual((rc, out, err), (0, "rc=0\n", ""))
        e1, e2 = self.ent("d1"), self.ent("d2")
        self.assertGreater(e1["reserved_at"], time.time() - 30, "내 예약의 reserved_at 이 지금 시각으로 바뀌어야 한다")
        self.assertEqual(e1["x_future"], {"a": [1, {"b": None}]}, "모르는 키를 지웠다(구·신 팩 혼재에서 가산 키 보존 계약)")
        self.assertEqual(e1["mission_key"], "m")
        self.assertEqual(e2, {"mission_key": "other", "reserved_at": self.OLD, "role": "dept-master"}, "다른 등재를 건드렸다")
        self.assertEqual(json.loads(self.raw().decode("utf-8"))["_future_top_level"], {"k": [1, 2]}, "최상위 모르는 키를 지웠다")

    def test_absent_entry_is_not_created(self):
        self.put({"d2": {"mission_key": "x", "reserved_at": self.OLD}})
        before = self.raw()
        rc, out, err = self.restamp("d1", "create", "m")
        self.assertEqual((rc, out), (0, "rc=0\n"))
        self.assertEqual(self.raw(), before, "등재가 없는데 파일을 바꿨다(등재를 새로 만들면 안 된다)")
        self.assertEqual(len([l for l in err.splitlines() if "WARN" in l]), 1)
        self.assertIn("이미 없다", err)

    def test_not_my_reservation_is_untouched(self):
        cases = [
            ("create · 다른 mission_key", ("d1", "create", "m"), {"mission_key": "other", "reserved_at": self.OLD}),
            ("create · mission_key 없음(번호 팀 등재)", ("d1", "create", "m"), {"reserved_at": self.OLD}),
            ("create · reserved_at 없는 옛 등재", ("d1", "create", "m"), {"mission_key": "m"}),
            ("번호 팀 allocate · create 가 만든 등재(mission_key 있음)", ("d1", "alloc", ""), {"mission_key": "m", "reserved_at": self.OLD}),
            ("번호 팀 allocate · 다른 팀 제안의 등재", ("d1", "alloc", ""), {"team_proposal_id": "tp-x", "reserved_at": self.OLD}),
            ("번호 팀 allocate · reserved_at 없는 옛 등재", ("d1", "alloc", ""), {"role": "dept-master"}),
        ]
        for label, args, entry in cases:
            with self.subTest(label):
                self.put({"d1": entry})
                before = self.raw()
                rc, out, err = self.restamp(*args)
                self.assertEqual((rc, out), (0, "rc=0\n"))
                self.assertEqual(self.raw(), before, "내 예약이 아닌 등재를 건드렸다")
                self.assertIn("내 예약이 아니다", err)

    def test_plain_alloc_restamps_a_plain_reservation(self):
        self.put({"d1": {"socket": "s", "pack_dir": "p", "role": "dept-master", "reserved_at": self.OLD}})
        rc, out, err = self.restamp("d1", "alloc", "")
        self.assertEqual((rc, out, err), (0, "rc=0\n", ""))
        self.assertGreater(self.ent("d1")["reserved_at"], time.time() - 30)

    def test_team_alloc_requires_the_same_proposal_id(self):
        import base64
        def b64(pid):
            return base64.urlsafe_b64encode(json.dumps({"v": 1, "id": pid, "display": "팀", "purpose": "일"}, ensure_ascii=False).encode("utf-8")).decode("ascii")
        self.put({"d1": {"team_proposal_id": "tp-mine", "reserved_at": self.OLD}})
        rc, out, err = self.restamp("d1", "alloc", b64("tp-mine"))
        self.assertEqual((rc, out, err), (0, "rc=0\n", ""))
        self.assertGreater(self.ent("d1")["reserved_at"], time.time() - 30)
        self.put({"d1": {"team_proposal_id": "tp-mine", "reserved_at": self.OLD}})
        before = self.raw()
        rc, out, err = self.restamp("d1", "alloc", b64("tp-other"))
        self.assertEqual((rc, out), (0, "rc=0\n"))
        self.assertEqual(self.raw(), before, "다른 제안 id 의 예약을 건드렸다")
        self.assertIn("내 예약이 아니다", err)

    def test_unreadable_registry_is_preserved_and_does_not_stop_creation(self):
        with open(self.reg, "wb") as f:
            f.write(b'{"depts": {"d1": ')   # 잘린 JSON
        before = self.raw()
        rc, out, err = self.restamp("d1", "create", "m")
        self.assertEqual((rc, out), (0, "rc=0\n"), "판독 실패가 종료 코드를 바꿨다(최선 노력 — 생성은 계속해야 한다)")
        self.assertEqual(self.raw(), before, "손상 원본을 건드렸다(원본 보존)")
        self.assertEqual(len([l for l in err.splitlines() if "WARN" in l]), 1)
        self.assertIn("읽지 못했다", err)

    def test_missing_registry_is_not_created(self):
        rc, out, err = self.restamp("d1", "create", "m")
        self.assertEqual((rc, out), (0, "rc=0\n"))
        self.assertFalse(os.path.exists(self.reg), "재기록이 등재 파일을 새로 만들었다")

    def test_lock_failure_is_best_effort(self):
        self.put({"d1": {"mission_key": "m", "reserved_at": self.OLD}})
        before = self.raw()
        os.mkdir(self.reg + ".lock")   # 잠금 파일을 열 수 없다(디렉터리) — 잠금 오류 모사
        rc, out, err = self.restamp("d1", "create", "m")
        self.assertEqual((rc, out), (0, "rc=0\n"), "잠금 실패가 종료 코드를 바꿨다")
        self.assertEqual(self.raw(), before)
        warns = [l for l in err.splitlines() if "WARN" in l]
        self.assertEqual(len(warns), 1, err)
        self.assertIn("잠금·쓰기 오류", warns[0])
        self.assertNotIn("Traceback", err, "파이썬 트레이스백이 사용자에게 샜다")

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0, "root 는 쓰기 권한 거부를 모사할 수 없다")
    def test_write_failure_is_best_effort_and_keeps_the_original(self):
        self.put({"d1": {"mission_key": "m", "reserved_at": self.OLD}})
        open(self.reg + ".lock", "w").close()
        before = self.raw()
        os.chmod(self.tmp, 0o555)   # 임시 파일을 만들 수 없다 — 쓰기 오류 모사
        rc, out, err = self.restamp("d1", "create", "m")
        os.chmod(self.tmp, 0o755)
        self.assertEqual((rc, out), (0, "rc=0\n"), "쓰기 실패가 종료 코드를 바꿨다")
        self.assertEqual(self.raw(), before, "쓰기 실패 뒤 원본이 바뀌었다")
        self.assertIn("잠금·쓰기 오류", err)


class RegistryWritersKeepUnknownKeys(unittest.TestCase):
    """allocate 예약에 더한 `reserved_at` 은 **가산 키**다 — 구 팩(0.14.42 이하)이 같은 등재를 쓸 때 모르는 키를 보존해야 한다. 등재 쓰기 도우미 4종(upsert·set_meta·set_field·remove)이 전부
    파일 전체를 읽어(load) 필요한 필드만 바꾸고 다시 쓴다(dump)는 것을 실제 함수로 확인한다 — 이 검체는 **현재** 도우미를 잰다(구 팩의 같은 4종은 R2F-PK WORKLOG 에서 v0.14.42 판으로 같은 방식으로 확인했다)."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "reject_slug_collision(){ :; }\n" + "".join(func_text(src, n) for n in ("reg_init", "reg_upsert", "reg_remove", "reg_set_meta", "reg_set_field"))

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wk-")
        self.reg = os.path.join(self.tmp, "depts.json")
        with open(self.reg, "w", encoding="utf-8") as f:
            json.dump({"depts": {"a": {"socket": "s", "pack_dir": "p", "role": "dept-master", "reserved_at": 1234.5, "x_future": [1, {"y": 2}]},
                                 "b": {"socket": "s2", "pack_dir": "p2", "role": "dept-master", "reserved_at": 99.0}}, "_top": 7}, f)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_fn(self, call):
        import shlex
        rc, out, err = bash_eval("set -u\nREG=%s\n%s%s\n" % (shlex.quote(self.reg), self.funcs, call))
        self.assertEqual(rc, 0, err[-600:])
        with open(self.reg, encoding="utf-8") as f:
            return json.load(f)

    def test_each_writer_keeps_reserved_at_and_unknown_keys(self):
        d = self.run_fn('reg_upsert a s-new p-new')
        self.assertEqual((d["depts"]["a"]["reserved_at"], d["depts"]["a"]["x_future"], d["depts"]["a"]["socket"]), (1234.5, [1, {"y": 2}], "s-new"))
        d = self.run_fn('reg_set_meta a 표시명 acct')
        self.assertEqual((d["depts"]["a"]["reserved_at"], d["depts"]["a"]["x_future"], d["depts"]["a"]["display_name"]), (1234.5, [1, {"y": 2}], "표시명"))
        d = self.run_fn('reg_set_field a cwd /somewhere')
        self.assertEqual((d["depts"]["a"]["reserved_at"], d["depts"]["a"]["x_future"], d["depts"]["a"]["cwd"]), (1234.5, [1, {"y": 2}], "/somewhere"))
        d = self.run_fn('reg_remove b')
        self.assertNotIn("b", d["depts"])
        self.assertEqual((d["depts"]["a"]["reserved_at"], d["depts"]["a"]["x_future"], d["_top"]), (1234.5, [1, {"y": 2}], 7), "다른 등재 제거가 남은 등재·최상위의 모르는 키를 지웠다")


class ReapBootGrace(unittest.TestCase):
    """reap 의 '죽은 등록'(소켓 무응답 = 즉시 등재 삭제) 갈래에 부팅 유예(R2F-PK · S4 m3): `reserved_at` 이 max(CYS_DEPT_REAP_GRACE(60), 예약 유예) 안이면 건너뛴다 — 부팅 중인 팀의 등재는 소켓이 아직 없어도
    죽은 등록이 아니다. `reserved_at` 이 없는(옛) 등재는 종전 그대로 지운다. `--dry` 출력에 그 사유가 보인다."""

    def new_sb(self, **kw):
        sb = Sandbox(**kw)
        self.addCleanup(sb.cleanup)
        return sb

    def put(self, sb, entries):
        depts = {}
        for name, age in entries.items():
            e = {"socket": sb.sock(name), "pack_dir": os.path.join(sb.home, ".cys", "pack-dept-%s" % name), "role": "dept-master"}
            if age is not None:
                e["reserved_at"] = time.time() - age
            depts[name] = e
        sb.write_reg(depts)

    SKIP_RE = re.compile(r"SKIP %s \(부팅 중 — 소켓은 아직 무응답이지만 예약 \d+초 전 ≤ 유예 %s초 · 죽은 등록으로 지우지 않는다\)")

    def test_dry_run_skips_a_booting_entry_and_says_why_on_stdout(self):
        sb = self.new_sb()
        self.put(sb, {"dept-1": 10})
        rc, out, err = sb.run("reap", "--dry")
        self.assertEqual(rc, 0, err[-600:])
        self.assertRegex(out, r"\[cys-dept\] reap\(dry\): " + self.SKIP_RE.pattern % ("dept-1", "60"))
        self.assertNotIn("DEAD dept-1", out + err)
        self.assertIn("DEAD=0 IDLE=0 SKIP=1", out)
        self.assertIn("dept-1", sb.read_reg(), "dry-run 이 등재를 건드렸다")

    def test_real_run_keeps_a_booting_entry_and_logs_the_skip(self):
        sb = self.new_sb()
        self.put(sb, {"dept-1": 10})
        rc, out, err = sb.run("reap")
        self.assertEqual(rc, 0, err[-600:])
        self.assertRegex(err, r"\[cys-dept\] reap: " + self.SKIP_RE.pattern % ("dept-1", "60"))
        self.assertNotIn("reg_remove", out + err)
        self.assertIn("dept-1", sb.read_reg(), "부팅 유예 안의 등재를 지웠다(데몬은 뜨는데 등재가 없는 팀)")
        self.assertIn("DEAD=0 IDLE=0 SKIP=1", out)

    def test_old_dead_entry_and_entry_without_reserved_at_are_removed_as_before(self):
        for label, age in (("유예 밖(200초)", 200), ("reserved_at 없음(옛 등재)", None)):
            with self.subTest(label):
                sb = self.new_sb()
                self.put(sb, {"dept-1": age})
                rc, out, err = sb.run("reap", "--dry")
                self.assertEqual(rc, 0, err[-600:])
                self.assertIn("reap(dry): DEAD dept-1 (sock 死) → reg_remove", out)
                self.assertNotRegex(out + err, r"reap(\(dry\))?: SKIP", "유예 밖·옛 등재를 부팅 중으로 건너뛰었다")
                rc, out, err = sb.run("reap")
                self.assertEqual(rc, 0, err[-600:])
                self.assertIn("reap: DEAD dept-1 → reg_remove", out)
                self.assertEqual(sb.read_reg(), {}, "죽은 등록이 지워지지 않았다(종전 동작)")

    def test_garbage_reserved_at_is_treated_as_old(self):
        sb = self.new_sb()
        sb.write_reg({"dept-1": {"socket": sb.sock("dept-1"), "pack_dir": "p", "role": "dept-master", "reserved_at": "abc"}})
        rc, out, err = sb.run("reap")
        self.assertEqual(rc, 0, err[-600:])
        self.assertEqual(sb.read_reg(), {}, "읽을 수 없는 reserved_at 은 '없음'(옛 등재)으로 접혀 종전대로 지워져야 한다")

    def test_only_the_booting_entry_is_spared_in_a_mixed_registry(self):
        sb = self.new_sb()
        self.put(sb, {"dept-1": 10, "dept-2": 500, "dept-3": None})
        rc, out, err = sb.run("reap")
        self.assertEqual(rc, 0, err[-600:])
        self.assertEqual(sorted(sb.read_reg()), ["dept-1"])
        self.assertIn("DEAD=2 IDLE=0 SKIP=1", out)

    def test_grace_is_the_max_of_reap_grace_and_the_reservation_grace(self):
        # 노브 180 → 예약 유예 197 > reap 유예 60: 나이 150 은 부팅 유예 안이다. 노브 기본이면 같은 나이가 죽은 등록이다.
        sb = self.new_sb(CYS_DEPT_READY_SECS="180")
        self.put(sb, {"dept-1": 150})
        rc, out, err = sb.run("reap", "--dry")
        self.assertRegex(out, self.SKIP_RE.pattern % ("dept-1", "197"))
        sb2 = self.new_sb()
        self.put(sb2, {"dept-1": 150})
        rc, out, err = sb2.run("reap", "--dry")
        self.assertIn("DEAD dept-1", out)
        # CYS_DEPT_REAP_GRACE 가 더 크면 그 값이 쓰인다
        sb3 = self.new_sb(CYS_DEPT_REAP_GRACE="300")
        self.put(sb3, {"dept-1": 200})
        rc, out, err = sb3.run("reap", "--dry")
        self.assertRegex(out, self.SKIP_RE.pattern % ("dept-1", "300"))

    def test_a_non_numeric_grace_setting_does_not_break_reap(self):
        sb = self.new_sb(CYS_DEPT_REAP_GRACE="abc", CYS_DEPT_RESERVE_GRACE="xyz")
        self.put(sb, {"dept-1": 10, "dept-2": 500})
        rc, out, err = sb.run("reap")
        self.assertEqual(rc, 0, err[-600:])
        self.assertEqual(sorted(sb.read_reg()), ["dept-1"], "숫자가 아닌 유예 설정에서도 기본값(60·25)으로 판정해야 한다")


class DefaultPathOutput(unittest.TestCase):
    """기본 경로(노브 미설정 · 성공 흐름)의 표지 순서·출력은 종전과 같다(R2F-PK 티켓 §2 · 기본 경로 무변경): 새 문구(재기록 · 노브 경고 · 대기 예산)가 어떤 줄에도 나오지 않고, 표지 순서와 stdout 계약이 종전 그대로다.
    (이 변경 전·후의 실제 출력을 같은 시나리오로 대조한 결과 — allocate·create·launch 의 stdout·stderr 가 글자 그대로 같고 allocate 등재에 reserved_at 키만 늘었다 — 는 R2F-PK WORKLOG 에 있다.)"""

    NEW_TOKENS = ("재기록", "reserved_at", "READY_SECS", "대기 예산", "실제 약")

    def run_flow(self, args, **kw):
        sb = Sandbox(STUB_PING_OK_FROM=UP_AT_PING, **kw)
        self.addCleanup(sb.cleanup)
        if args[0] == "create":
            sb.seed_catalog("k1", "m1")
            sb.write_reg({})
        return sb, sb.run(*args)

    def test_allocate_and_create_success_flows_emit_no_new_text(self):
        for args, want_stages in ((("allocate",), ["reserve", "probe", "spawn", "wait", "up", "done"]),
                                  (("create", "k1"), ["reserve", "probe", "spawn", "wait", "up", "seat", "done"])):
            with self.subTest(args[0]):
                sb, (rc, out, err) = self.run_flow(args)
                self.assertEqual(rc, 0, err[-800:])
                self.assertEqual(stages(err), want_stages)
                self.assertEqual(out, "dept-1\n", "stdout 은 부서 이름 한 줄이다(종전 계약)")
                for tok in self.NEW_TOKENS:
                    self.assertNotIn(tok, err + out, "기본 경로 출력에 이 판의 새 문구(%r)가 나왔다" % tok)
                self.assertEqual(last_line(err), "[cys-dept] @stage done")

    def test_launch_success_flow_is_unchanged(self):
        sb, (rc, out, err) = self.run_flow(("launch", "a"))
        self.assertEqual(rc, 0, out + err[-800:])
        for tok in self.NEW_TOKENS:
            self.assertNotIn(tok, err + out, "launch 기본 경로 출력에 새 문구(%r)가 나왔다" % tok)
        self.assertRegex(last_line(out), r"^\[cys-dept\] a 가동 완료 \(sock=.* pack=.* acct=none cwd=.*\)\. 부서 수=1$")
        self.assertNotIn("@stage", out + err)

    def test_launch_failure_only_adds_the_stderr_copy(self):
        # 실패 흐름: stdout 은 종전 그대로(실패 줄 1줄 + 종전 줄), stderr 만 같은 꼬리가 한 줄 늘었다(R2F-PK · S4 m7). 새 문구 중 stdout 에 나오는 것은 실패 줄의 꼬리뿐이다.
        sb = Sandbox(STUB_CYSD_MODE="dead")
        self.addCleanup(sb.cleanup)
        rc, out, err = sb.run("launch", "a")
        self.assertEqual(rc, 1)
        out_lines = [l for l in out.splitlines() if "데몬 기동 실패" in l]
        err_lines = [l for l in err.splitlines() if "데몬 기동 실패" in l]
        self.assertEqual((len(out_lines), len(err_lines)), (1, 1))
        self.assertEqual(out_lines, err_lines)
        self.assertEqual([l for l in out.splitlines() if "재기록" in l or "reserved_at" in l], [])


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
        # R2F-PK(S4 m7): launch 줄만 같은 꼬리를 stderr 에도 한 줄 더 낸다(stdout 줄·순서·위 핀은 그대로 — 복사본은 `reg_remove` **앞**에 있다). allocate·create 는 이미 stderr 라 복사본이 없다.
        self.assertIn('_lf="$(ready_fail_note "$name")"; echo "[cys-dept] ERROR: $name 데몬 기동 실패${_lf}" >&2; reg_remove', post[0])
        self.assertNotIn("_lf", post[1] + post[2], "allocate·create 의 실패 줄은 이미 stderr 다 — 복사본을 더하면 같은 줄이 두 번 난다")

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

    BASH4_ONLY = ((r"\$\{[^}]*(,,|\^\^)", "${var,,}/${var^^}(bash 4+)"), (r"declare\s+-A|typeset\s+-A", "연관 배열"),
                  (r"\bmapfile\b|\breadarray\b", "mapfile/readarray"), (r"local\s+-n\b", "nameref"), (r"&>>|\|&", "bash 4 리다이렉션"),
                  (r"\[\[", "[[ ]] (정수 판정은 case 꼴)"))

    @staticmethod
    def strip_heredoc_python(text):
        """함수 본문에서 파이썬 히어독 몸통(`<<'PY'` … `PY`)을 걷는다 — 셸 문법 검사에 파이썬이 섞이지 않게."""
        return re.sub(r"<<'PY'.*?\nPY\n", "<<'PY'\n", text, flags=re.S)

    def assert_bash32_safe(self, block, what):
        block = "\n".join(l for l in block.splitlines() if not l.lstrip().startswith("#"))
        for pat, why in self.BASH4_ONLY:
            self.assertIsNone(re.search(pat, block), "%s 가 bash 3.2/MSYS 불안전 문법을 쓴다: %s" % (what, why))

    def test_new_helpers_use_only_bash32_msys_safe_syntax(self):
        a = self.src.index('dept_stage(){ echo')
        z = self.src.index("\n}\n", self.src.index("ready_fail_note(){")) + 2   # ready_fail_note 는 R2F-PK 에서 여러 줄 함수가 됐다 — 본문 끝까지 본다
        self.assert_bash32_safe(self.src[a:z], "이 판의 도우미(표지·노브·대기·유예·실패 문구)")
        for name in ("reg_restamp", "dept_reap_boot_grace"):
            self.assert_bash32_safe(self.strip_heredoc_python(func_text(self.src, name)), "R2F-PK 도우미 %s" % name)

    def test_seconds_is_never_assigned_by_the_product(self):
        # `$SECONDS` 는 셸 전체의 경과 기준이다 — 대입하면 실패 문구의 '실제 약 M초'·기한이 흔들린다. 가짜 시계(검체)만 민다.
        bad = [l for l in self.code if re.search(r"(^|[^A-Za-z0-9_$])SECONDS=", l)]
        self.assertEqual(bad, [], "제품 코드가 SECONDS 에 대입한다: %r" % bad)

    def test_ready_wait_reads_the_clock_only_after_iteration_120(self):
        fn = func_text(self.src, "ready_wait")
        guard = 'if [ "$i" -gt 120 ] && [ $(( SECONDS - t0 )) -ge "$secs" ]; then break; fi'
        self.assertEqual(fn.count(guard), 1, "벽시계 상한 판정이 `121번째 반복부터`(i > 120) 한 곳에만 있어야 한다")
        self.assertEqual(len(re.findall(r"SECONDS", "\n".join(l for l in fn.splitlines() if not l.lstrip().startswith("#")))), 4,
                         "ready_wait 의 SECONDS 읽기는 t0 · 성공 경과 · 기한 판정 · 실패 경과 네 곳뿐이다(첫 120회에 시계를 보는 읽기가 늘면 기본 경로가 달라진다)")
        self.assertIn('n=$(( secs * 10 ))', fn, "횟수 상한(초 × 10)은 그대로여야 한다")
        self.assertEqual(self.src.count('ready_wait(){'), 1)

    def test_knob_warn_is_called_only_at_the_three_wait_sites_and_ready_wait_stays_self_contained(self):
        # 경고(dept_ready_knob_warn)는 스폰 뒤 대기의 호출 지점 세 곳이 `ready_wait` 직전에 부른다. `ready_wait` 자신은 dept_ready_secs 만 쓰는 자기완결·무소음이다 —
        #   기존 횟수 핀 검체(ReadyBudget)가 ready·ready_wait·dept_ready_secs 세 함수만 떼어 돌리고 무효 노브에서도 stderr 가 비어 있음을 못박기 때문이다(무수정 통과 계약).
        self.assertEqual(self.src.count("\ndept_ready_knob_warn(){"), 1)
        calls = [i for i, l in enumerate(self.code) if re.match(r"dept_ready_knob_warn(\s|$)", l.strip())]
        self.assertEqual(len(calls), 3, "경고 호출 지점은 launch·allocate·create 세 곳이어야 한다: %r" % [self.code[i] for i in calls])
        for i in calls:
            nxt = next(l for l in self.code[i + 1:] if l.strip())
            self.assertRegex(nxt, r'\bready_wait "\$sock" \|\|', "경고는 스폰 뒤 대기(ready_wait) 직전이어야 한다: %r" % nxt)
        rw = "\n".join(l for l in func_text(self.src, "ready_wait").splitlines() if not l.lstrip().startswith("#"))
        self.assertNotIn("dept_ready_knob_warn", rw, "ready_wait 가 경고 함수에 기대면 기존 횟수 핀이 그 함수를 떼어 오지 않아 깨진다")
        self.assertNotIn(">&2", rw, "ready_wait 는 무소음이어야 한다")

    def test_restamp_call_sites_and_helper(self):
        # 호출은 정확히 두 곳 — allocate_dept 와 create 동사의 **스폰 분기 첫머리**(`dept_stage spawn` 바로 아래 · 스폰 명령 앞). 사전 검사(`ready`) 뒤 · 스폰 앞이다.
        #   launch 본체엔 예약 개념이 없다(호출 0). 도우미는 한 번만 정의된다.
        self.assertEqual(self.src.count("\nreg_restamp(){"), 1)
        lines = self.src.splitlines()
        calls = [i for i, l in enumerate(lines) if l.strip().startswith("reg_restamp ") and not l.lstrip().startswith("#")]
        self.assertEqual(len(calls), 2, "reg_restamp 호출 지점 수: %r" % [lines[i] for i in calls])
        spawn_at = [i for i, l in enumerate(lines) if l.strip().startswith("dept_stage spawn")]
        self.assertEqual(len(spawn_at), 2)
        for c, s in zip(calls, spawn_at):
            self.assertEqual(c, s + 1, "재기록은 `dept_stage spawn` 바로 아래여야 한다(스폰 직전): %r" % lines[c])
            nxt = next(l for l in lines[c + 1:] if not l.lstrip().startswith("#") and l.strip())
            self.assertTrue("nohup" in nxt or "_cys_detach_ok" in nxt, "재기록 다음 실행 줄이 스폰이어야 한다(그 사이에 다른 일이 끼면 '스폰 직전'이 아니다): %r" % nxt)
        self.assertIn('reg_restamp "$name" alloc "$TEAM_B64"', lines[calls[0]])
        self.assertIn('reg_restamp "$name" create "$mkey"', lines[calls[1]])
        lstart = next(i for i, l in enumerate(lines) if l.startswith("launch_dept(){"))
        lend = next(i for i in range(lstart, len(lines)) if lines[i] == "}")
        self.assertFalse(any(lstart < c < lend for c in calls), "launch 본체엔 예약이 없다 — 재기록 호출이 들어갔다")

    def test_allocate_reservation_writes_reserved_at(self):
        lines = self.src.splitlines()
        start = next(i for i, l in enumerate(lines) if l.startswith("allocate_dept(){"))
        end = next(i for i in range(start, len(lines)) if lines[i] == "}")
        seg = "\n".join(lines[start:end + 1])
        self.assertIn("import json,sys,os,re,time,socket as S", seg)
        self.assertIn("'reserved_at':time.time()", seg, "allocate 의 예약에 reserved_at(가산 키)이 없다")

    def test_reap_dead_branch_has_the_boot_grace_before_the_removal(self):
        lines = self.src.splitlines()
        rs = next(i for i, l in enumerate(lines) if l == "  reap)")
        re_ = next(i for i in range(rs, len(lines)) if lines[i] == "  promote-ceo)")
        seg = "\n".join(l for l in lines[rs:re_] if not l.lstrip().startswith("#"))
        i_grace = seg.index("dept_reap_boot_grace")
        i_dead = seg.index("reap: DEAD $nm → reg_remove")
        self.assertLess(i_grace, i_dead, "부팅 유예 판정이 '죽은 등록' 삭제보다 앞이어야 한다")
        self.assertIn("reap(dry): SKIP", seg, "--dry 출력에 건너뜀 사유가 없다")
        self.assertIn('dept_reserve_grace 2>/dev/null', seg, "예약 유예는 reap 에서 한 번 계산하고 경고는 버린다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
