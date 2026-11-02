#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_create_progress.py — 0.14.43 GP: 「팀 직접 만들기」 단계 표지 · 스폰 뒤 소켓 대기 예산 노브 · 실패 문구 핀.

배경: GUI 의 「팀 직접 만들기」는 Tauri allocate_dept_daemon 이 `bash cys-dept allocate [--team-spec-b64 …]` 또는
`bash cys-dept create <키>` 를 실행하고 끝날 때까지 기다린다(계약: stdout 마지막 비어 있지 않은 줄 = 부서 이름 · 실패 시 종료
코드와 stderr 가 그대로 사용자에게 간다). 그 사이 화면은 스피너뿐이었고(실측 시작→소켓 청취 약 23초) 스폰 뒤 대기 상한을
넘으면 "데몬 기동 실패" 한 줄이 전부였다. 이 판이 더한 것(전부 가산 · 기본 동작 무변경):

  A. A70 policy=min(180*max(1,N),900); explicit12..86400 override wins; invalid uses policy.
  B. A66 ready_wait — one owned elapsed budget including slow ping children; the historical precheck ready count is unchanged.
  C. dept_reserve_grace— 노브를 12 보다 올렸을 때만 create 예약 유예 = max(RESERVE_GRACE 또는 25, 노브+17)(R2F-PK — 벽시계 상한이 생겨 종전의 `노브/5` 항은 근거가 없어졌다) · 노브를 올렸는데 RESERVE_GRACE 가 정수가 아니면
                          stderr 경고 1줄(기본 경로에서는 경고 없음) · 노브가 기본이면 종전 인라인 값(`${CYS_DEPT_RESERVE_GRACE:-25}`) 그대로(명시한 낮은 값도) + 실흐름 분기 핀
  D. @stage 표지       — allocate·create 의 stderr 1줄 기계 판독 표지(순서 불변식 · stdout 0회 · launch 무변경) · **`spawn` 표지의 유무는 화면이 '새 팀'을 판정하는 계약**이다(재사용 갈래는 내지 않는다)
  E. 실패 문구         — 접두 "데몬 기동 실패" 유지 + (대기 예산 N초 · 실제 약 M초 · 로그 경로 · 현재 값 기준 노브 안내 — R2F-PK) · 스트림은 각 줄 종전 그대로 + launch·rotate 의 같은 꼬리는 stderr 에도 한 줄(가산)
  F. census            — 사전 검사 3곳은 `ready` · 스폰 뒤 3곳만 `ready_wait` · 표지 소재지 · bash 3.2/MSYS 안전 문법 · SECONDS 대입 금지 · 재기록 호출 지점
  G. 예약 재기록·reap  — (R2F-PK) 데몬을 띄우기 직전 `reserved_at` 을 한 번 다시 찍는다(create·allocate · 최선 노력 · 등재를 새로 만들지 않는다) · allocate 의 예약에도 `reserved_at` · reap 의 '죽은 등록' 갈래 부팅 유예 ·
                          기본 경로 출력은 종전과 같다

라이브 무접촉: 격리 HOME + 목 cys/cysd/sleep($HOME/.local/bin — cys-dept 의 PATH 선두). 실 데몬·실 팩·~/.cys 를 건드리지 않는다.
Flow fixtures explicitly set12 seconds; they do not assert the product default. Fake sleep accelerates polling; real elapsed/death tests use real child processes.
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
echo "$$" > "$HOME/.stub-cysd.pid"
[ "${STUB_CYSD_MODE:-up}" = dead ] && exit 0
mkdir -p "$(dirname "$CYS_SOCKET")"
[ "${STUB_CYSD_MODE:-up}" = silent ] || touch "$CYS_SOCKET"
# A70: a socket fixture must keep its daemon alive; bounded and private-HOME owned.
i=0
while [ -d "$HOME" ] && [ "$i" -lt 3600 ]; do /bin/sleep 0.05; i=$((i+1)); done
exit 0
'''
SLEEP_STUB = "#!/bin/sh\nexit 0\n"
# 재기록 호출(`reg_restamp` 의 파이썬 — argv 에 `restamp:<종류>` 표지가 있다)만 실패시키는 가짜 python3 — 그 밖의 호출은 진짜 인터프리터로 넘긴다(stdin 의 히어독 그대로 전달).
PYTHON_WRAP = "#!/bin/sh\ncase \" $* \" in\n  *\" restamp:\"*) echo \"[mock python3] 재기록 호출 실패 모사\" >&2; exit \"${STUB_RESTAMP_RC:-1}\" ;;\nesac\nexec \"@REALPY@\" \"$@\"\n"
# Flow fixtures use an explicit12-second override and no-op sleep, not the
# production default. Successful scenarios use a deterministic ping-count signal
# so socket-marker scheduling does not decide readiness. UP_AT_PING includes the
# fixed120 pre-probes plus any reservation probe. Spawn evidence is the parent's
# synchronous log redirection, not the asynchronously written child call log.
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
        env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_") and k != "CLAUDE_CONFIG_DIR"}
        for k in list(env):
            if k.startswith("STUB_") or k.startswith("_CYS_TT_"):
                env.pop(k)
        for k in ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_NO_AUTOSTART", "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG",
                  "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT", "CYS_DEPT_CWD", "CYS_DEPT_READY_SECS",
                  "CYS_DEPT_RESERVE_GRACE", "CYS_DEPT_CAP", "CYS_SURFACE_ID", "CYS_DEPT_NO_MASTER"):
            env.pop(k, None)
        env.update({"HOME": self.home, "CYS_DEPTS_JSON": self.reg, "CYS_DEPT_NO_MASTER": "1", "CYS_DEPT_READY_SECS": "12",
                    "PATH": bindir + os.pathsep + env.get("PATH", "")})
        env.update(env_extra)
        self.env = env

    def cleanup(self):
        marker = os.path.join(self.home, ".stub-cysd.pid")
        try: pid = int(_read(marker).strip())
        except (OSError, ValueError): pid = None
        shutil.rmtree(self.tmp, ignore_errors=True)
        # The private fixture observes HOME removal and exits itself; never kill
        # a guessed/recycled PID. Wait a bounded interval for its exit to land.
        if pid:
            for _ in range(40):
                try: os.kill(pid, 0)
                except ProcessLookupError: break
                time.sleep(.05)
            else: raise AssertionError("private daemon fixture did not exit after HOME cleanup")

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
    env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_") and k != "CLAUDE_CONFIG_DIR"}
    for k in ("OK_AT",) + tuple(unset): env.pop(k, None)
    with tempfile.TemporaryDirectory(prefix="ready-eval-") as home:
        env.update(HOME=home, CYS_DEPTS_JSON=os.path.join(home, "depts.json"), PYTHONDONTWRITEBYTECODE="1")
        env.update(env_extra or {})
        r = subprocess.run(["bash", "-c", script, DEPT], capture_output=True, text=True, encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stdout, r.stderr



# ════════════════════════════════════════════════════════════════════════════════════
# A. dept_ready_secs — 순수 해석 함수
# ════════════════════════════════════════════════════════════════════════════════════
class DeptReadySecs(unittest.TestCase):
    # A70: explicit12..86400 wins; an empty private registry defaults to180.
    CASES = [(None, "180"), ("", "180"), ("12", "12"), ("60", "60"), ("180", "180"),
             ("181", "181"), ("600", "600"), ("86400", "86400"), ("086400", "86400"),
             ("86401", "180"), ("11", "180"), ("0", "180"), ("-5", "180"), ("abc", "180"),
             ("30.5", "180"), ("012", "12"), ("08", "180"), ("060", "60"),
             ("99999999999999999999999", "180"), (" 60", "180"), ("60 ", "180"),
             ("1e2", "180"), ("+60", "180"), ("6 0", "180")]


    @classmethod
    def setUpClass(cls):
        cls.fn = "".join(func_text(_read(DEPT), n) for n in ("dept_ready_default", "dept_ready_secs"))

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
    """The unchanged pre-spawn probe still has its historical count contract."""

    def count(self, ok_at=0, knob="12"):
        script = "set -u\n" + func_text(_read(DEPT), "ready")
        script += 'sleep(){ :; }\nCOUNT=0\ncys_probe(){ COUNT=$((COUNT+1)); [ "$OK_AT" -gt 0 ] && [ "$COUNT" -ge "$OK_AT" ]; }\nCYS=cys_probe\nready sock\necho rc=$? count=$COUNT\n'
        rc, out, err = bash_eval(script, {"OK_AT": str(ok_at), "CYS_DEPT_READY_SECS": knob})
        self.assertEqual((rc, err), (0, ""))
        return out

    def test_precheck_count_is_knob_independent(self):
        for knob in ("12", "13", "60", "180", "bad"):
            self.assertEqual(self.count(knob=knob), "rc=1 count=120\n")

    def test_precheck_success_and_count_boundary(self):
        for n in (1, 7, 120):
            self.assertEqual(self.count(n), "rc=0 count=%d\n" % n)
        self.assertEqual(self.count(121), "rc=1 count=120\n")


class ReadyElapsedBound(unittest.TestCase):
    """A66: actual slow child pings must consume the whole wait's deadline."""

    def run_probe(self, delay, result, budget=1, daemon_delay=None):
        with tempfile.TemporaryDirectory(prefix="a66-ready-") as home:
            probe = os.path.join(home, "probe")
            _write_exec(probe, "#!/bin/sh\n/bin/sleep %s\nexit %s\n" % (delay, result))
            src = _read(DEPT)
            script = "set -u\n" + "".join(func_text(src, n) for n in ("ready_wait", "dept_ready_secs"))
            # Short budgets exercise the negative deadline cases. The successful
            # control uses the public 12-second floor; the parser accepts 12..86400.
            script += "dept_ready_secs(){ echo %s; }\nCYS=\"$1\"\nready_wait fixture\nrc=$?\necho result=$rc elapsed=$_READY_WAIT_SECS\n" % budget
            if daemon_delay is not None:
                script = script.replace('ready_wait fixture', '/bin/sleep %s & _dept_spawn_pid=$!\nready_wait fixture' % daemon_delay)
                script += 'kill "$_dept_spawn_pid" 2>/dev/null || true\nwait "$_dept_spawn_pid" 2>/dev/null || true\n'
            env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_") and k != "CLAUDE_CONFIG_DIR"}
            env.update(HOME=home, PYTHONDONTWRITEBYTECODE="1")
            started = time.monotonic()
            r = subprocess.run(["bash", "-c", script, DEPT, probe], env=env,
                               capture_output=True, text=True, timeout=max(12, budget + 10))
            elapsed = time.monotonic() - started
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(r.stderr, "")
            m = re.fullmatch(r"result=(\d+) elapsed=(\d+)\n", r.stdout)
            self.assertIsNotNone(m, r.stdout)
            return int(m.group(1)), elapsed

    def test_slow_failure_is_bounded_before_first_ten_probes_finish(self):
        rc, elapsed = self.run_probe(.3, 1)
        self.assertEqual(rc, 1)
        self.assertLess(elapsed, 2.5, "slow ping time escaped the one-second wait budget")

    def test_success_after_deadline_is_not_ready(self):
        rc, elapsed = self.run_probe(3, 0)
        self.assertEqual(rc, 1, "a late successful ping must not turn timeout into readiness")
        self.assertLess(elapsed, 2.5)

    def test_slow_success_inside_deadline_is_ready(self):
        rc, _ = self.run_probe(.1, 0, budget=12)
        self.assertEqual(rc, 0)

    def test_dead_child_exits_before_the_long_budget(self):
        rc, elapsed = self.run_probe(3, 0, budget=10, daemon_delay=0)
        self.assertEqual(rc, 1)
        self.assertLess(elapsed, 2.5)

    def test_child_dies_during_a_slow_ping(self):
        rc, elapsed = self.run_probe(3, 0, budget=10, daemon_delay=.2)
        self.assertEqual(rc, 1)
        self.assertLess(elapsed, 2.5)

    def test_live_silent_child_waits_to_the_bound(self):
        rc, elapsed = self.run_probe(.1, 1, budget=1, daemon_delay=10)
        self.assertEqual(rc, 1)
        self.assertGreater(elapsed, .7)
        self.assertLess(elapsed, 2.5)


class ReadyFleetPolicy(unittest.TestCase):
    def value(self, n, override=None, broken=False):
        with tempfile.TemporaryDirectory(prefix="ready-fleet-") as home:
            reg = os.path.join(home, "depts.json")
            with open(reg, "w") as f:
                f.write("broken" if broken else json.dumps({"depts": {"d%d" % i: {} for i in range(n)}}))
            src = _read(DEPT)
            names = (["dept_ready_default"] if "dept_ready_default(){" in src else []) + ["dept_ready_secs"]
            script = "set -u\n" + "".join(func_text(src, name) for name in names) + "dept_ready_secs\n"
            env = {"CYS_DEPTS_JSON": reg}
            if override is not None: env["CYS_DEPT_READY_SECS"] = override
            return bash_eval(script, env)

    def test_registered_count_policy_and_cap(self):
        for n, expected in ((0,180),(1,180),(2,360),(5,900),(9,900)):
            with self.subTest(n=n): self.assertEqual(self.value(n), (0,str(expected)+"\n",""))

    def test_override_wins_and_invalid_uses_dynamic_default(self):
        for override, expected in (("12",12),("180",180),("86400",86400),("086400",86400),("11",900),("86401",900),("bad",900)):
            with self.subTest(override=override): self.assertEqual(self.value(9,override), (0,str(expected)+"\n",""))

    def test_native_python_crlf_is_removed_before_shell_arithmetic(self):
        # Only the Python result transport is substituted; the production shell
        # function must normalize its output before arithmetic consumes it.
        script = "set -eu\n" + func_text(_read(DEPT), "dept_ready_default")
        script += "python3(){ cat >/dev/null; printf '360\\r\\n'; }\nv=\"$(dept_ready_default)\"\necho $((v * 10))\n"
        self.assertEqual(bash_eval(script), (0,"3600\n",""))

    def test_unreadable_registry_is_named_fallback_not_zero_claim(self):
        rc,out,err = self.value(9,broken=True)
        self.assertEqual((rc,out),(0,"180\n"))
        self.assertIn("registry unreadable",err)


class ReadyKnobWarn(unittest.TestCase):
    """Invalid nonempty overrides warn once and use the dynamic policy.
    Unset/empty/valid overrides do not warn. The warning belongs to the three
    pre-wait call sites; unreadable-registry fallback has its own diagnostic."""

    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("dept_ready_default", "dept_ready_secs", "dept_ready_knob_warn"))

    def warn(self, knob, calls=1):
        script = "set -u\n" + self.funcs + "dept_ready_knob_warn\n" * calls + "echo done\n"
        env = {} if knob is None else {"CYS_DEPT_READY_SECS": knob}
        return bash_eval(script, env)

    WARN = ["86401", "abc", "186400", "0", "11", "-5", "30.5", " 60", "60 ", "1e2", "0x20", "+60", "99999999999999999999", "086401", "100000"]
    QUIET = [None, "", "12", "13", "60", "179", "180", "300", "600", "86400", "012", "060", "00012", "00180"]

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
        rc, out, err = self.warn("86401", calls=3)
        self.assertEqual(len(err.splitlines()), 1, "같은 프로세스에서 경고가 되풀이됐다: %r" % err)

    def test_value_is_folded_to_one_short_line(self):
        rc, out, err = self.warn("12\n13")
        self.assertEqual(len(err.splitlines()), 1, "개행이 든 값이 경고를 여러 줄로 쪼갰다: %r" % err)
        self.assertIn("'12 13'", err)
        rc, out, err = self.warn("x" * 200)
        self.assertEqual(len(err.splitlines()), 1)
        self.assertIn("'" + "x" * 40 + "'", err)
        self.assertNotIn("x" * 41, err, "값은 40자까지만 싣는다")

    def test_pure_interpreter_stays_silent(self):
        # Real readiness silence is asserted by ReadyElapsedBound, including timeout.
        rc, out, err = bash_eval("set -u\n" + "".join(func_text(_read(DEPT), n) for n in ("dept_ready_default", "dept_ready_secs")) + "dept_ready_secs\n", {"CYS_DEPT_READY_SECS": "300"})
        self.assertEqual((rc, out, err), (0, "300\n", ""))


class KnobWarnFlow(unittest.TestCase):
    """A(R2F-PK): 호출 지점 세 곳(launch·allocate·create)이 스폰 뒤 대기 직전에 경고를 낸다 — 실흐름에서 무효 노브는 정확히 1줄(stderr · stdout 오염 없음 · 흐름 무변경), 기본·유효 값은 0줄,
    스폰이 없는 재사용 갈래(사전 검사가 이미 켜진 데몬을 찾음)에서는 기다리지 않으므로 경고도 없다."""

    WARN_RE = re.compile(r"^\[cys-dept\] WARN: CYS_DEPT_READY_SECS='86401' ")

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
                sb, (rc, out, err) = self.flow(args, "86401")
                self.assertEqual(rc, 0, out + err[-800:])
                w = self.warns(err)
                self.assertEqual(len(w), 1, "무효 노브 경고가 정확히 1줄이어야 한다:\n" + err[-1200:])
                self.assertRegex(w[0], self.WARN_RE)
                self.assertNotIn("READY_SECS", out, "경고가 stdout 으로 샜다(부서 이름·종전 stdout 계약 오염)")
                if args[0] != "launch":
                    self.assertEqual(last_line(out), "dept-1")

    def test_valid_override_and_fixture_explicit_12_do_not_warn(self):
        for knob in (None, "12", "60", "060"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                sb, (rc, out, err) = self.flow(("allocate",), knob)
                self.assertEqual(rc, 0, err[-800:])
                self.assertEqual(self.warns(err), [], "기본·유효 노브에서 경고가 났다")

    def test_no_wait_means_no_warning(self):
        sb, (rc, out, err) = self.flow(("allocate",), "86401", ok_from="2")   # 사전 검사가 이미 켜진 데몬을 찾는다 → 스폰도 대기도 없다
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(self.warns(err), [], "기다리지 않았는데 노브 경고가 났다(경고는 스폰 뒤 대기 직전의 것이다)")

    def test_invalid_knob_failure_uses_dynamic_budget_and_warns_before_failure(self):
        sb = Sandbox(STUB_CYSD_MODE="dead", CYS_DEPT_READY_SECS="86401")
        self.addCleanup(sb.cleanup)
        rc, out, err = sb.run("allocate")
        self.assertEqual(rc, 1)
        self.assertEqual(len(self.warns(err)), 1)
        lines = err.splitlines()
        wi = next(i for i, l in enumerate(lines) if "CYS_DEPT_READY_SECS=" in l and "WARN" in l)
        fi = next(i for i, l in enumerate(lines) if "데몬 기동 실패" in l)
        self.assertLess(wi, fi, "경고가 실패 줄보다 앞에 있어야 한다(실패 문구 앞 300자에 사유가 보이도록 짧게 앞에)")
        self.assertIn("대기 예산 180초", lines[fi], "invalid override uses the one-lane policy budget")
        self.assertLess(len(lines[wi]), 130, "경고 줄이 길어 실패 사유를 300자 밖으로 밀어낸다")


class ReadyFailNote(unittest.TestCase):
    """Failure note reports the captured wait budget and actual elapsed value;
    advice doubles the effective budget up to the explicit86400-second cap."""

    CASES = [(None,"180","CYS_DEPT_READY_SECS=360 처럼 더 늘릴 수 있다 · 상한 86400초"),
             ("12","12","CYS_DEPT_READY_SECS=24 처럼 더 늘릴 수 있다 · 상한 86400초"),
             ("60","60","CYS_DEPT_READY_SECS=120 처럼 더 늘릴 수 있다 · 상한 86400초"),
             ("86400","86400","대기 예산이 이미 상한(86400초)이다 — 로그를 확인하라")]


    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.funcs = "".join(func_text(src, n) for n in ("dept_ready_default", "dept_ready_secs", "ready_fail_note"))

    def note(self, knob, elapsed="7"):
        pre = "" if elapsed is None else "_READY_WAIT_SECS=%s\n" % elapsed
        script = ("set -u\n" + self.funcs + "dept_logdir(){ printf '%s' \"/L/cys-dept-$1\"; }\n" + pre + 'ready_fail_note nm\n')
        env = {} if knob is None else {"CYS_DEPT_READY_SECS": knob}
        rc, out, err = bash_eval(script, env)
        self.assertEqual((rc, err), (0, ""))
        return out

    def test_note_uses_captured_budget_instead_of_a_new_registry_read(self):
        script = "set -u\n" + self.funcs
        script += 'dept_logdir(){ echo /fixture; }\ndept_ready_secs(){ echo 900; }\n_READY_WAIT_BUDGET=360\n_READY_WAIT_SECS=4\nready_fail_note n\n'
        rc,out,err = bash_eval(script)
        self.assertEqual((rc,err),(0,""))
        self.assertIn("대기 예산 360초 · 실제 약 4초",out)
        self.assertNotIn("900",out)

    def test_note_table(self):
        for knob, budget, tail in self.CASES:
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertEqual(self.note(knob), " (대기 예산 %s초 · 실제 약 7초 · 로그: /L/cys-dept-nm/cysd.log · %s)" % (budget, tail))

    def test_raised_knob_users_are_not_told_to_use_60(self):
        for knob in ("60", "75", "90", "120", "179", "180"):
            with self.subTest(CYS_DEPT_READY_SECS=knob):
                self.assertNotIn("CYS_DEPT_READY_SECS=60 ", self.note(knob), "이미 60 이상으로 올린 사용자에게 '=60 처럼 늘릴 수 있다'고 안내했다")

    def test_elapsed_comes_from_the_wait_not_from_the_knob(self):
        self.assertIn("대기 예산 180초 · 실제 약 28초", self.note(None, "28"))
        self.assertIn("대기 예산 180초 · 실제 약 211초", self.note("180", "211"))
        self.assertIn("실제 약 ?초", self.note(None, None), "대기가 시작되지 않았다면 경과는 모른다고 적는다(지어내지 않는다)")

    def test_legacy_prefix_words_are_gone(self):
        self.assertNotIn("소켓 대기", self.note(None), "종전 문구('소켓 대기 N초' — N 은 노브 값이지 경과가 아니었다)가 남았다")


# ════════════════════════════════════════════════════════════════════════════════════
# C. dept_reserve_grace — 예약 유예 결합
# ════════════════════════════════════════════════════════════════════════════════════
class ReserveGraceUnit(unittest.TestCase):
    # Reservation grace follows the effective policy/override; explicit12 retains legacy grace.
    CASES = [(None,None,"197"),("10",None,"197"),("0",None,"197"),
             ("10","12","10"),("0","12","0"),("40","12","40"),
             (None,"13","30"),("31","13","31"),(None,"60","77"),
             ("76","60","77"),("77","60","77"),("78","60","78"),
             (None,"180","197"),(None,"181","198"),(None,"600","617"),
             ("10","bad","197"),(None,"86400","86417"),("2.5","600","2.5"),
             ("abc","60","abc"),("30.5","180","30.5")]
    WARN_ROWS = {("2.5","600"),("abc","60"),("30.5","180")}


    @classmethod
    def setUpClass(cls):
        src = _read(DEPT)
        cls.fns = "".join(func_text(src, n) for n in ("dept_ready_default", "dept_ready_secs", "dept_reserve_grace"))

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
            return r is None or not r.isdigit() or (12 < int(r) <= 86400)
        want = {(g, r) for g, r, _ in self.CASES if raised(r) and g is not None and not g.isdigit()}
        self.assertEqual(want, self.WARN_ROWS)

    def test_explicit_12_override_retains_legacy_grace_expression(self):
        """An explicit12 override alone retains the historical inline grace value."""
        import shlex
        graces = ["", "0", "10", "24", "25", "26", "40", "08", "0025", "2.5", "abc", "-5", " 30", "-n", "-e", "99999999999999999999"]
        loop = ('bad=""\nfor g in %s; do\n  export CYS_DEPT_RESERVE_GRACE="$g"\n'
                '  a="$(dept_reserve_grace)"; b="${CYS_DEPT_RESERVE_GRACE:-25}"\n  [ "$a" = "$b" ] || bad="$bad [$g]:new=[$a]:old=[$b]"\ndone\n'
                'unset CYS_DEPT_RESERVE_GRACE\na="$(dept_reserve_grace)"; b="${CYS_DEPT_RESERVE_GRACE:-25}"\n'
                '[ "$a" = "$b" ] || bad="$bad [unset]:new=[$a]:old=[$b]"\nprintf "bad=<%%s>\\n" "$bad"\n') % " ".join(shlex.quote(g) for g in graces)
        for ready in ("12",):   # Only explicit12 retains the legacy grace contract.
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

    def test_explicit_12_grace_25_age_40_is_dead_revive(self):
        self.sb.env["STUB_PING_OK_FROM"] = "2"   # 번호 점유 확인(파이썬 첫 핑)은 실패 · 사전 검사 첫 핑은 성공 → 재사용 분기로 짧게 끝낸다
        self.sb.seed_entry("dept-1", "m1", age=40)
        rc, out, err = self.sb.run("create", "k1")
        self.assertEqual(rc, 0, err[-800:])
        self.assertIn("등록부 잔존(dept-1)·데몬 사망(grace 경과)", err, "explicit12 fixture grace25 must classify age40 as REUSE_DEAD")
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

    def test_explicit_low_grace_is_honored_at_explicit_12(self):
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

    def test_no_grace_warning_at_explicit_12_or_with_integer_grace(self):
        # Sandbox supplies explicit12 unless overridden; fractional grace is quiet only at that floor.
        for tag, env in (("fixture explicit12 + fractional grace", {"CYS_DEPT_RESERVE_GRACE": "30.5"}),
                         ("explicit12 override + fractional grace", {"CYS_DEPT_READY_SECS": "12", "CYS_DEPT_RESERVE_GRACE": "30.5"}),
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
NOTE_TAIL = "/cysd.log · CYS_DEPT_READY_SECS=24 처럼 더 늘릴 수 있다 · 상한 86400초)"
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

    def test_allocate_failure_stages_stop_at_wait_and_registry_retained(self):
        b = self.box["alloc"]
        self.assertEqual(stages(b["err"]), ["reserve", "probe", "spawn", "wait"], "실패 경로는 낸 데까지 — up·done 이 있으면 안 된다")
        self.assertIn("dept-1", b["reg"], "readiness failure must retain the row")
        self.assertTrue(b["spawned"], "스폰 분기인데 cysd 로그 리다이렉트가 만들어지지 않았다")
        self.assertEqual(last_line(b["out"]), "", "실패 경로 stdout 은 비어 있다")

    def test_dead_child_shortcuts_explicit_12_fixture(self):
        # The explicit12 fixture has a dead child: stop before consuming a full post-spawn probe allowance.
        b = self.box["alloc"]
        self.assertLess(b["pings"], 240, "dead child did not stop readiness early")

    def test_dead_child_shortcuts_explicit_30_fixture(self):
        # Explicit30 must not make the caller wait on a child already known dead; precheck remains120 attempts.
        b = self.box["alloc30"]
        self.assertEqual(b["rc"], 1)
        note = self.note_of(b["err"], "dept-1")
        note_budget(self, note, "30")
        self.assertLess(b["pings"], 240, "dead child must not consume the longer budget")

    def test_create_failure_message_stage_retained(self):
        b = self.box["create"]
        self.assertEqual(b["rc"], 1)
        note = self.note_of(b["err"], "dept-1")
        note_budget(self, note, "12")
        self.assertTrue(note.endswith(NOTE_TAIL), note)
        self.assertIn("로그: %s/cysd.log" % b["sb"].logdir("dept-1"), note)
        self.assertNotIn("데몬 기동 실패", b["out"], "create 의 실패 줄은 종전대로 stderr 다")
        self.assertEqual(stages(b["err"]), ["reserve", "probe", "spawn", "wait"])
        self.assertIn("dept-1", b["reg"], "readiness failure must retain NEW row")

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
        self.assertIn("a", b["reg"], "launch readiness failure must retain row")


class SlowDaemonKnob(unittest.TestCase):
    """With the same scripted success at ping300, the explicit12-second
    fixture fails while explicit30 succeeds. This compares two overrides,
    not the production dynamic default; the elapsed bound may beat the count cap."""

    def run_alloc(self, **extra):
        sb = Sandbox(STUB_CYSD_MODE="silent", STUB_PING_OK_FROM="300", **extra)
        self.addCleanup(sb.cleanup)
        return sb, sb.run("allocate")

    def test_explicit_12_fixture_misses_scripted_late_readiness(self):
        sb, (rc, out, err) = self.run_alloc()
        self.assertEqual(rc, 1, err[-600:])
        self.assertRegex(err, r" \(대기 예산 12초 · 실제 약 \d+초 · 로그: ")
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait"])
        self.assertIn("dept-1", sb.read_reg())
        self.assertLess(sb.ping_count(), 300, "explicit12 fixture must stop before scripted success at ping300")

    def test_explicit_30_fixture_reaches_scripted_late_readiness(self):
        sb, (rc, out, err) = self.run_alloc(CYS_DEPT_READY_SECS="30")
        self.assertEqual(rc, 0, err[-800:])
        self.assertEqual(stages(err), ["reserve", "probe", "spawn", "wait", "up", "done"])
        self.assertEqual(last_line(out), "dept-1")
        self.assertGreaterEqual(sb.ping_count(), 300, "explicit30 fixture must reach the scripted success at ping300")
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
    """The explicit12-second Sandbox fixture preserves stage/output contracts
    on successful flows. The product dynamic default is tested by ReadyFleetPolicy."""

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
        self.assertEqual(len([l for l in self.code if re.search(r'(^|\s)ready "\$', l)]), 3, "`ready` 호출은 사전 검사 3곳뿐이어야 한다")
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
        for line in post:
            self.assertNotIn("reg_remove", line, "A68 timeout must not remove any row")
        self.assertIn('_lf="$(ready_fail_note "$name")"; echo "[cys-dept] ERROR: $name 데몬 기동 실패${_lf}" >&2;', post[0])
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

    def test_ready_wait_uses_one_owned_elapsed_budget(self):
        fn = func_text(self.src, "ready_wait")
        self.assertIn('--bounded "$secs" --native-shell -- "$BASH"', fn)
        self.assertIn('"$(dirname "$0")/_owned_process.py"', fn)
        self.assertNotIn('"$i" -gt 120', fn, "the old unbounded first-120 exemption returned")
        self.assertIn('n=$(( secs * 10 ))', fn)
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



class DeptCapDefaultUnlimited(unittest.TestCase):
    """B1(0.14.53 · 오너 2026-10-22 「부서 만들기 무한으로 하고, 제약을 해제해」): 살아 있는 팀 수 상한은 **기본 무제한**이다.
    두 자리(allocate 의 번호 예약 · create 의 카탈로그 NEW 예약)가 같은 규칙이다 — CYS_DEPT_CAP 미설정·빈 값·0 이면 살아 있는 팀이 몇이든 거부하지
    않고, 0보다 큰 N 을 주면 살아 있는 팀이 N 이상일 때 exit 8(등재·스폰 0). 살아 있는 팀 = 소켓 파일이 있는 등록(목 ping 이 생존으로 본다)."""

    VERBS = (("allocate",), ("create", "k1"))
    LIVE = 8   # 종전 기본 상한(8)과 같은 수 — 종전 코드는 여기서 9번째 팀을 거부했다

    def sandbox(self, **env):
        sb = Sandbox(STUB_PING_OK_FROM=UP_AT_PING, **env)
        self.addCleanup(sb.cleanup)
        depts = {}
        for i in range(1, self.LIVE + 1):
            name = "dept-%d" % i
            os.makedirs(sb.logdir(name), exist_ok=True)
            open(sb.sock(name), "w").close()
            depts[name] = {"socket": sb.sock(name), "pack_dir": os.path.join(sb.home, ".cys", "pack-dept-%s" % name),
                           "role": "dept-master", "mission_key": "other-%d" % i, "cwd": sb.home,
                           "account_dir": os.path.join(sb.home, "acct"), "reserved_at": time.time() - 3600}
        sb.write_reg(depts)
        sb.seed_catalog("k1", "m1")   # create 는 mission_key m1 이 등재에 없으니 NEW 갈래(cap 검사 자리)로 간다
        return sb

    def assert_not_refused(self, sb, args, rc, out, err):
        self.assertNotIn("live 부서 수", err, "%s: 살아 있는 팀 수로 거부했다" % args[0])
        self.assertEqual(rc, 0, "%s: rc=%s err=%s" % (args[0], rc, err[-800:]))
        self.assertEqual(last_line(out), "dept-9", "%s: 9번째 팀 이름이 아니다" % args[0])
        self.assertIn("dept-9", sb.read_reg(), "%s: 9번째 팀 등재가 없다" % args[0])

    def test_unset_cap_never_refuses_on_live_count(self):
        for args in self.VERBS:
            with self.subTest(args[0]):
                sb = self.sandbox()
                self.assertNotIn("CYS_DEPT_CAP", sb.env)
                self.assert_not_refused(sb, args, *sb.run(*args))

    def test_empty_and_zero_cap_mean_unlimited(self):
        for cap in ("", "0"):
            for args in self.VERBS:
                with self.subTest(verb=args[0], CYS_DEPT_CAP=cap):
                    sb = self.sandbox(CYS_DEPT_CAP=cap)
                    self.assert_not_refused(sb, args, *sb.run(*args))

    def test_positive_cap_refuses_at_live_ge_n_with_exit_8(self):
        word = {"allocate": "allocate 거부", "create": "create NEW 거부"}
        for args in self.VERBS:
            with self.subTest(args[0]):
                sb = self.sandbox(CYS_DEPT_CAP=str(self.LIVE))
                before = sb.read_reg()
                rc, out, err = sb.run(*args)
                self.assertEqual(rc, 8, "%s: rc=%s err=%s" % (args[0], rc, err[-800:]))
                self.assertIn("[cys-dept] %s: live 부서 수 %d ≥ CYS_DEPT_CAP=%d" % (word[args[0]], self.LIVE, self.LIVE), err)
                self.assertEqual(sb.read_reg(), before, "%s: 거부했는데 등재가 바뀌었다" % args[0])
                self.assertFalse(sb.spawned("dept-9"), "%s: 거부했는데 데몬을 띄웠다" % args[0])
                self.assertEqual(last_line(out), "", "%s: 거부했는데 stdout 에 이름이 나왔다" % args[0])

    def test_positive_cap_above_live_count_still_creates(self):
        for args in self.VERBS:
            with self.subTest(args[0]):
                sb = self.sandbox(CYS_DEPT_CAP=str(self.LIVE + 1))
                self.assert_not_refused(sb, args, *sb.run(*args))

    def test_both_sites_carry_the_same_rule(self):
        src = _read(DEPT)
        reads = re.findall(r'^    CAP=int\(os\.environ\.get\("CYS_DEPT_CAP","0"\) or 0\)', src, re.M)
        guards = re.findall(r"^    if CAP>0 and live>=CAP:$", src, re.M)
        self.assertEqual((len(reads), len(guards)), (2, 2), "두 자리(allocate·create)가 같은 규칙이 아니다")
        self.assertNotIn('get("CYS_DEPT_CAP","8")', src, "종전 기본 8 이 남아 있다")



class RegistryRollbackOwnership(unittest.TestCase):
    """A61: real command/registry, scripted readiness later than the probe budget.

    No live daemon: the private cysd stub exits without binding.
    A68/A70 retains both existing and new rows when readiness fails.
    """

    def box(self):
        sb = Sandbox(STUB_CYSD_MODE="dead")
        self.addCleanup(sb.cleanup)
        return sb

    def test_existing_launch_row_survives_readiness_failure(self):
        sb = self.box()
        sb.seed_entry("a", "existing-mission", age=300)
        before = sb.read_reg()["a"]
        rc, out, err = sb.run("launch", "a")
        self.assertEqual(rc, 1, (out, err))
        self.assertTrue(sb.spawned("a"))
        self.assertIn("a", sb.read_reg(), "readiness failure removed a pre-existing row")
        self.assertEqual(sb.read_reg()["a"], before)

    def test_failed_upsert_cannot_authorize_rollback(self):
        # Exercise the actual launch body and real registry helpers, replacing
        # the failed write/readiness edges only. No daemon or live socket.
        sb = self.box()
        sb.seed_entry("a", "existing-mission", age=300)
        before = _read(sb.reg)
        src = _read(DEPT)
        names = ("reg_init", "reg_remove", "rot_reg_quiet", "rot_reg_degraded", "launch_dept")
        script = "set -eu\n" + "".join(func_text(src, n) for n in names) + r'''
validate_dept_name(){ :; }
reject_slug_collision(){ :; }
dept_sock(){ echo "$HOME/rig.sock"; }
dept_pack(){ echo "$HOME/pack"; }
dept_logdir(){ echo "$HOME/log"; }
seed_acl(){ :; }
reg_upsert(){ return 10; }
resolve_lane_acctdir(){ :; }
resolve_dept_cwd(){ echo "$HOME"; }
reg_get_field(){ echo "$HOME"; }
ready(){ return 1; }
ready_wait(){ return 1; }
dept_ready_knob_warn(){ :; }
ready_fail_note(){ :; }
sock_len_diag(){ :; }
_CYS_ROTATE_SELF=1
CYSD=/usr/bin/true
REG="$CYS_DEPTS_JSON"
launch_dept a
'''
        r = subprocess.run(["bash", "-c", script], env=sb.env,
                           capture_output=True, text=True, timeout=15)
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("kill 이후", r.stderr)
        self.assertEqual(_read(sb.reg), before, "failed upsert authorized row removal")

    def test_new_launch_row_survives_readiness_failure(self):
        sb = self.box()
        rc, out, err = sb.run("launch", "a")
        self.assertEqual(rc, 1, (out, err))
        self.assertTrue(sb.spawned("a"))
        self.assertIn("a", sb.read_reg())

    def test_catalog_reuse_dead_row_survives_failure(self):
        sb = self.box()
        sb.env.pop("STUB_PING_OK_FROM", None)
        sb.seed_catalog("k1", "m1")
        sb.seed_entry("dept-1", "m1", age=300)
        rc, out, err = sb.run("create", "k1")
        self.assertEqual(rc, 1, (out, err))
        self.assertIn("dept-1", sb.read_reg())
        self.assertEqual(sb.read_reg()["dept-1"]["mission_key"], "m1")

    def test_allocate_failure_preserves_other_registered_rows(self):
        sb = self.box()
        sb.env.pop("STUB_PING_OK_FROM", None)
        sb.seed_entry("dept-1", "m1", age=300)
        before = sb.read_reg()["dept-1"]
        rc, out, err = sb.run("allocate")
        self.assertEqual(rc, 1, (out, err))
        self.assertTrue(sb.spawned("dept-2"))
        self.assertEqual(sb.read_reg()["dept-1"], before)
        self.assertIn("dept-2", sb.read_reg())

import datetime
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

import datetime
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
import sys

class RemovalAudit(unittest.TestCase):
    def setUp(self):
        self.box = tempfile.TemporaryDirectory(prefix="a66-audit-")
        self.addCleanup(self.box.cleanup)
        self.home = Path(self.box.name)
        self.reg = self.home / "depts.json"
        self.reg.write_text(json.dumps({"depts":{"a":{"x":1},"b":{"x":2}},"other":3}))
        self.audit = self.home / ".local/state/cys/dept-removals.jsonl"
        self.env = {k:v for k,v in os.environ.items() if not k.startswith(("CYS_","CLAUDE_"))}
        self.env.update(HOME=str(self.home),PYTHONDONTWRITEBYTECODE="1")
        source = Path(os.environ.get("A66_SOURCE", DEPT)).read_text()
        def fn(name):
            start=source.index(name+"(){")
            return source[start:source.index("\n}",start)+2]+"\n"
        self.functions = "reg_init(){ :; }\n" + fn("reg_remove")

    def run_remove(self, name="a", **extra):
        env=dict(self.env,**extra)
        script="set -eu\nREG="+shlex.quote(str(self.reg))+"\ncmd=reap\n"+self.functions+"reg_remove "+shlex.quote(name)+"\n"
        return subprocess.run(["/bin/bash","-c",script],env=env,capture_output=True,text=True,timeout=10)

    def test_removal_writes_one_durable_record_and_second_appends(self):
        before=hashlib.sha256(self.reg.read_bytes()).hexdigest()
        r=self.run_remove();self.assertEqual(r.returncode,0,r.stderr)
        self.assertTrue(self.audit.exists(), "removal left no durable audit")
        first=self.audit.read_bytes(); rows=[json.loads(x) for x in first.splitlines()]
        self.assertEqual(len(rows),1);row=rows[0]
        self.assertEqual((row["lane"],row["verb"],row["reason"],row["phase"]),("a","reap","dead-socket","remove_intent"))
        self.assertGreater(row["pid"],0);self.assertGreater(row["parent_pid"],0)
        self.assertEqual(row["before_sha256"],before)
        self.assertEqual(row["after_sha256"],hashlib.sha256(self.reg.read_bytes()).hexdigest())
        self.assertAlmostEqual(datetime.datetime.fromisoformat(row["utc"]).timestamp(),row["epoch"],places=5)
        self.assertEqual(self.run_remove("b").returncode,0)
        second=self.audit.read_bytes();self.assertTrue(second.startswith(first))
        self.assertEqual([json.loads(x)["lane"] for x in second.splitlines()],["a","b"])
        self.assertEqual(self.run_remove("b").returncode,0)
        self.assertEqual(self.audit.read_bytes(),second, "absent row made a duplicate removal record")

    def inject_fault(self):
        bindir=self.home/"bin";bindir.mkdir()
        wrapper=bindir/"python3"
        wrapper.write_text("#!"+sys.executable+"\n"+r'''
import os,sys
code=sys.stdin.read(); sys.argv=sys.argv[1:]
fault=os.environ.get("A66_TEST_FAULT")
write,fsync,replace=os.write,os.fsync,os.replace
owned=[None]
def badwrite(fd,data):
    owned[0]=fd
    if fault=="short": return write(fd,data[:5])
    return write(fd,data)
def badsync(fd):
    if fault=="fsync" and fd==owned[0]: raise OSError("injected audit fsync")
    return fsync(fd)
def badreplace(src,dst):
    if fault=="replace": raise OSError("injected registry replace")
    return replace(src,dst)
os.write=badwrite;os.fsync=badsync;os.replace=badreplace
exec(compile(code,"<audit-fault-fixture>","exec"))
''')
        wrapper.chmod(0o755)
        self.env["PATH"]=str(bindir)+os.pathsep+self.env["PATH"]

    def test_short_append_is_not_followed_by_registry_change(self):
        self.inject_fault();before=self.reg.read_bytes()
        r=self.run_remove(A66_TEST_FAULT="short")
        self.assertNotEqual(r.returncode,0);self.assertIn("short audit append",r.stderr)
        self.assertEqual(self.reg.read_bytes(),before)
        partial=self.audit.read_bytes();self.assertEqual(len(partial),5)
        again=self.run_remove()
        self.assertNotEqual(again.returncode,0);self.assertIn("incomplete audit tail",again.stderr)
        self.assertEqual(self.audit.read_bytes(),partial)
        self.assertEqual(self.reg.read_bytes(),before)

    def test_failed_audit_fsync_preserves_registry(self):
        self.inject_fault();before=self.reg.read_bytes()
        r=self.run_remove(A66_TEST_FAULT="fsync")
        self.assertNotEqual(r.returncode,0);self.assertIn("injected audit fsync",r.stderr)
        self.assertEqual(self.reg.read_bytes(),before)
        self.assertEqual(json.loads(self.audit.read_text())["phase"],"remove_intent")

    def test_replace_failure_leaves_honest_intent_not_false_commit(self):
        self.inject_fault();before=self.reg.read_bytes()
        r=self.run_remove(A66_TEST_FAULT="replace")
        self.assertNotEqual(r.returncode,0);self.assertIn("injected registry replace",r.stderr)
        self.assertEqual(self.reg.read_bytes(),before)
        self.assertEqual(json.loads(self.audit.read_text())["phase"],"remove_intent")

    def test_creation_failure_reason_reaches_both_teardown_routes(self):
        source=Path(os.environ.get("A66_SOURCE", DEPT)).read_text()
        def fn(name):
            start=source.index(name+"(){")
            return source[start:source.index("\n}",start)+2]+"\n"
        helpers=fn("dept_creation_failed")+fn("dept_teardown_owned")+"cleanup_owned_dept_spawn(){ :; }\n"
        for token in ("0","1"):
            with self.subTest(token=token):
                self.reg.write_text(json.dumps({"depts":{"a":{}}}))
                child=self.home/("down-"+token+".sh")
                child.write_text("#!/bin/bash\nset -eu\nREG="+shlex.quote(str(self.reg))+"\ncmd=down\n"+self.functions+'reg_remove "$2"\n')
                child.chmod(0o755)
                script="set -eu\nREG="+shlex.quote(str(self.reg))+"\ncmd=create\n_CYS_TT_OK="+token+"\n"+self.functions+helpers+"down_dept(){ reg_remove \"$2\"; }\n"+"dept_creation_failed 6 a down\n"
                r=subprocess.run(["/bin/bash","-c",script,str(child)],env=self.env,capture_output=True,text=True,timeout=10)
                self.assertEqual(r.returncode,6,r.stderr)
                row=json.loads(self.audit.read_text().splitlines()[-1])
                self.assertEqual(row["reason"],"creation-failed-rc-6")
                self.assertEqual(row["reason_source"],"cleanup-context")
                self.assertEqual(row["registry_path"],str(self.reg))

    def test_audit_open_failure_preserves_registry_bytes(self):
        self.audit.mkdir(parents=True)
        before=self.reg.read_bytes();r=self.run_remove()
        self.assertNotEqual(r.returncode,0)
        self.assertEqual(self.reg.read_bytes(),before)

    def test_full_audit_refuses_without_truncation_or_registry_change(self):
        self.audit.parent.mkdir(parents=True)
        self.audit.write_bytes(b"{}\n")
        before=self.reg.read_bytes();r=self.run_remove(CYS_DEPT_AUDIT_MAX_BYTES="3")
        self.assertNotEqual(r.returncode,0)
        self.assertEqual(self.audit.read_bytes(),b"{}\n")
        self.assertEqual(self.reg.read_bytes(),before)

    def test_partial_tail_refuses_without_rewriting_history(self):
        self.audit.parent.mkdir(parents=True)
        self.audit.write_bytes(b'{"unfinished":')
        before=self.reg.read_bytes();r=self.run_remove()
        self.assertNotEqual(r.returncode,0)
        self.assertEqual(self.audit.read_bytes(),b'{"unfinished":')
        self.assertEqual(self.reg.read_bytes(),before)


class ReadinessTimeoutPreservesRows(unittest.TestCase):
    def test_each_new_row_survives_dead_or_live_silent_readiness_failure(self):
        for mode in ("dead", "silent"):
            for args in (("launch", "a"), ("allocate",), ("create", "k1")):
                with self.subTest(mode=mode,args=args):
                    sb=Sandbox(STUB_CYSD_MODE=mode)
                    try:
                        if args[0]=="create": sb.seed_catalog("k1","m1")
                        source=_read(DEPT)
                        source=source.replace(func_text(source,"dept_ready_secs"),"dept_ready_secs(){ echo 1; }\n")
                        tool=os.path.join(sb.home,"cys-dept")
                        _write_exec(tool,source)
                        shutil.copyfile(os.path.join(BIN,"_owned_process.py"),os.path.join(sb.home,"_owned_process.py"))
                        started=time.monotonic()
                        r=subprocess.run(["bash",tool,*args],env=sb.env,capture_output=True,text=True,timeout=15)
                        elapsed=time.monotonic()-started
                        self.assertEqual(r.returncode,1,(r.stdout,r.stderr))
                        self.assertIn("데몬 기동 실패",r.stdout+r.stderr)
                        name="a" if args[0]=="launch" else "dept-1"
                        self.assertIn(name,sb.read_reg(),"readiness failure deleted a row")
                        if mode=="silent": self.assertGreater(elapsed,.7)
                    finally: sb.cleanup()

if __name__ == "__main__":
    unittest.main(verbosity=2)
