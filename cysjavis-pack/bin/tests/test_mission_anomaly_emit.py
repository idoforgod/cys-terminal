#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_mission_anomaly_emit.py — MISSION-ANOMALY-EMIT 검체 (0.14.52 PROMOTE-3 · B-328 #3).

무엇을 막는가: 본부 팩에서 손으로 고쳐 쓰던 javis_mission.py 수정(2026-10-13)을 제품에 올리면서
그 수정이 약속한 세 가지가 다시 깨지는 것.
  ① `machine-origin` 은 수집한 이상징후를 stderr 로 **발행한다** — 형식은 `status` 와 같은 한 줄
     (`[mission] ★이상징후(<code>): <detail> — 오너에게 보고하라(은폐 금지)`). 종전에는 수집만
     하고 발행하지 않아 보고 의무가 충족 불가능했다.
  ② stdin 을 읽는 세 서브커맨드(record·machine-origin·hook-triage)의 `--help`/`-h` 는 stdin 을
     **읽기 전에** 도움말로 답한다. 종전에는 stdin 을 기다리며 멈췄다. 멈춤은 출력이 없어 실패로
     보이지 않으므로, 이 검체는 호출을 **별도 스레드**에서 돌리고 시한을 넘기면 FAIL 로 적는다.
  ③ 판정 경로 무접촉 — ★단 예외가 하나 있다(결정 A): 첫 인자가 `--help`/`-h` 인 record·machine-origin·
     hook-triage 는 exit 와 stdout 이 **바뀐다**(②의 결과 — PROMOTE-3 수정 전·후 100행 대조(증거
     BEFORE-AFTER.md)에서 다른 행은 이 12행뿐이다. rc 숫자가 바뀌는 것은 10행이고, record + stdin 프롬프트
     2행은 rc 0 그대로 stdout 이 도움말이 되고 임무 기록이 사라진다 · 아래 ★ · 5·8·9절). 그 예외를 뺀
     판정 — 그 대조의 나머지 88행과 7절 표 15행 — 에서 exit 와 stdout 이 수정 전과 같다. 7절 표의
     기대값은 **수정 전 파일**(v0.14.51 출하본 · sha256-16 a2b9376e)로 잰 값이고, 같은 표를 수정 전·후
     두 파일이 모두 통과해야 한다(표에 도움말 행은 없다 — 이 절은 수정 전 파일에서도 초록이어야 정상이다).

★알려진 성질(결함 아님 · 결정 A): record·machine-origin·hook-triage 의 exit 0 은 이제 **두 뜻**이다 —
  "도움말만 인쇄하고 아무것도 판정하지 않았다"(main 의 처리기 전 반환) 또는 진짜 판정 0. rc 하나로는 둘을
  가를 수 없다 — 판정이 필요한 호출자는 `--help`/`-h` 를 넘기지 않는다(현재 호출자 0 · PROMOTE-3 census).
  종전 바이트는 `record --help` + stdin 프롬프트를 **임무로 기록했다**(처리기 전 도움말 분기가 없었다) —
  9절이 그 해를 rc 가 아니라 대장 파일 바이트로 잰다.

밀폐: 모든 서브프로세스는 케이스마다 새 임시 디렉터리로 `CYS_PACK_DIR`·`CYS_STATE_DIR` 를 덮고 좌석·레인
변수(`CYS_*`·`AITERM_*`·`JAVIS_*`)를 전부 벗긴다 — 라이브 팩·실 임무 대장·실 배달 원장 무접촉, 데몬 스폰 0,
진짜 `cys` 호출 0(`set` 은 빈 PATH 또는 rig 안의 **가짜** `cys` 셸 스크립트로만 부른다). 자식 인터프리터는
`-B` 로 띄우고 이 프로세스도 캐시를 쓰지 않는다 — 체크아웃에 아무것도 쓰지 않는다.

  1  ① machine-origin: env 이상(env_not_int) + 원장 부재(ledger_absent)를 stderr 에 발행 · 형식 정규식
  2  ① 같은 상태의 status(임무 있는 대장)와 이상징후 줄이 **바이트 동일**(순서는 다를 수 있다)
  3  ① 층1 이상(ledger_rotated · delivery_substring · ledger_bad_lines)도 발행 · stdout 은 토큰 1줄
  4  ① 이상이 없는 상태(원장 정상 · env 정상)에서는 이상징후 줄 0
  5  ② --help/-h × 3 명령: 시한 안에 exit 0 · 도움말 · stdin 무판독(열린 파이프에 아무것도 안 써도 끝남)
  6  ② 범위: --help 가 첫 인자가 아니면 분기하지 않는다(record x --help 는 stdin 을 읽는다)
  7  ③ 판정 경로 표: 8개 서브커맨드 × 입력 — exit 와 stdout(경로는 표준화) 이 수정 전 값과 같다(도움말 입력은 이
     표에 없다 — 새 거동이라 5·8·9절이 잰다)
  8  ③ exit 코드 census — main() 에서 출발한 **제어 흐름**으로 rc 표면을 닫는다: (a) AST 로 main 과 디스패치
     처리기의 모든 return 을 따라 값을 출처까지 해소(해소 불가 0) (b) 행의 (도구, 서브커맨드) 집합 == main 디스패치
     키 + --self-test + 64 + 도움말(집합 동치) (c) 추적기로 그 모든 반환·대입 지점이 실행됐다(커버리지). 행마다 기대
     rc 는 수정 전 파일 실측값(도움말 행만 새 거동). except 경로만 이름 지정 결함 주입. POSIX 전용(가짜 cys = sh).
  9  ② 해(害) 측정: 세 명령 × {--help,-h} × stdin 프롬프트 — 임무 대장 파일이 바이트 그대로(부재는 부재로)

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 MISSION-ANOMALY-EMIT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_mission_anomaly_emit.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
MISSION = os.path.join(BIN, "javis_mission.py")
PY = sys.executable or "python3"
HANG_LIMIT_S = 20.0           # 콜드 스타트 여유를 둔 시한 — 정상 도움말은 1초 안에 끝난다
ANOMALY_LINE = re.compile(r"^\[mission\] ★이상징후\(([a-z_]+)\): (.+) — 오너에게 보고하라\(은폐 금지\)$")
OWNER_PROMPT = "윈도우 실사고 근본수정 진행해"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


SEAT_PREFIXES = ("CYS_", "AITERM_", "JAVIS_")


def make_env(tmp, extra=None, no_cys=False):
    """밀폐 env. 좌석·레인 변수(`CYS_*`·`AITERM_*`·`JAVIS_*`)는 **전부** 벗기고 필요한 셋만 다시 건다 —
    좌석 안에서 돌리면 `CYS_SOCKET` 이 레인 이름을 바꿔 원장·대장 경로가 달라진다(목록으로 몇 개만
    벗기면 목록 밖 변수가 결과를 바꾼다)."""
    env = {k: v for k, v in os.environ.items() if not k.startswith(SEAT_PREFIXES)}
    env["CYS_PACK_DIR"] = os.path.join(tmp, "pack")
    env["CYS_STATE_DIR"] = os.path.join(tmp, "state")
    env["PYTHONPATH"] = BIN + os.pathsep + env.get("PYTHONPATH", "")
    env["CYS_SURFACE_ID"] = "emit"
    if no_cys:
        # `set` 이 오너 확인 채널(`cys feed push --wait` — 살아 있는 데몬에 승인 요청을 띄우고 120초 기다린다)을
        # 열지 못하게 PATH 를 **빈 디렉터리 하나**로 둔다. 인터프리터 디렉터리를 남기면 그 옆에 cys 가 있는
        # 기계(예: 둘 다 /usr/local/bin)에서 실제 승인 요청이 나간다. 인터프리터는 절대경로로 부르므로 PATH 가 비어도 된다.
        empty = os.path.join(tmp, "empty-path")
        os.makedirs(empty, exist_ok=True)
        env["PATH"] = empty
    env.update(extra or {})
    return env


def run(args, env, stdin=None):
    return subprocess.run([PY, "-B", MISSION] + args, capture_output=True, text=True,
                          timeout=180, env=env, input=stdin)


def anomaly_lines(stderr):
    return [ln for ln in stderr.splitlines() if ln.startswith("[mission] ★이상징후(")]


def hook_json(prompt):
    return json.dumps({"prompt": prompt}, ensure_ascii=False)


root = tempfile.mkdtemp(prefix="mission-emit-")
n_case = [0]


def fresh(extra=None, no_cys=False):
    n_case[0] += 1
    d = os.path.join(root, "c%02d" % n_case[0])
    os.makedirs(d)
    return d, make_env(d, extra, no_cys)


# 이 검체는 체크아웃에 아무것도 쓰지 않는다 — 자식 인터프리터는 `-B` 로 띄우고, 이 프로세스의 import 도 캐시를 쓰지 않는다.
sys.dont_write_bytecode = True
sys.path.insert(0, BIN)
for _k in ("CYS_MISSION", "CYS_DELIVERY_WINDOW_S", "CYS_MISSION_TTL_S"):
    os.environ.pop(_k, None)
import javis_mission as jm                                             # noqa: E402


def ledger_path_for(env):
    """그 env 로 띄운 서브프로세스가 읽을 배달 원장 경로 — 경로 규약의 소유자(javis_bootstrap)에게 묻는다.

    경로를 이 검체가 손으로 조립하면 규약이 바뀌는 순간 엉뚱한 파일에 원장을 쓰고 '이상 없음'으로
    초록이 된다. 그래서 같은 env 를 이 프로세스에 잠시 걸고 모듈 함수로 구한 뒤 원래대로 돌린다."""
    keep = {k: v for k, v in os.environ.items() if k.startswith(SEAT_PREFIXES)}
    for k in keep:
        del os.environ[k]
    os.environ.update({k: v for k, v in env.items() if k.startswith(SEAT_PREFIXES)})
    try:
        return jm.delivery_ledger_path()
    finally:
        for k in [k for k in os.environ if k.startswith(SEAT_PREFIXES)]:
            del os.environ[k]
        os.environ.update(keep)

try:
    # ── 1 machine-origin 이 env 이상 + 원장 부재를 stderr 에 발행 ─────────────
    d1, e1 = fresh({"CYS_DELIVERY_WINDOW_S": "abc"})
    r1 = run(["machine-origin"], e1, stdin=hook_json(OWNER_PROMPT))
    a1 = anomaly_lines(r1.stderr)
    codes1 = [ANOMALY_LINE.match(ln).group(1) if ANOMALY_LINE.match(ln) else None for ln in a1]
    check("1a machine-origin 이 env_not_int 를 stderr 에 발행", "env_not_int" in codes1, repr(a1))
    check("1b machine-origin 이 ledger_absent 를 stderr 에 발행", "ledger_absent" in codes1, repr(a1))
    check("1c 이상징후 줄 전부가 status 와 같은 형식 정규식에 맞는다",
          bool(a1) and all(ANOMALY_LINE.match(ln) for ln in a1), repr(a1))
    check("1d 판정은 그대로: exit 1 · stdout 은 토큰 1줄 'machine-origin: human'",
          r1.returncode == 1 and r1.stdout == "machine-origin: human\n",
          repr((r1.returncode, r1.stdout)))
    check("1e stdout 에 이상징후가 새지 않는다(훅은 stdout 토큰을 1차 근거로 읽는다)",
          "★이상징후" not in r1.stdout, repr(r1.stdout))

    # ── 2 같은 상태의 status 와 바이트 동일 ───────────────────────────────────
    #    status 는 임무 대장이 있을 때만 이상징후를 싣는다 — 같은 디렉터리에 record 로 대장을 만든다.
    run(["record"], e1, stdin=hook_json(OWNER_PROMPT))
    r2 = run(["status"], e1)
    a2 = anomaly_lines(r2.stderr)
    r2mo = run(["machine-origin"], e1, stdin=hook_json(OWNER_PROMPT))
    a2mo = anomaly_lines(r2mo.stderr)
    common = sorted(set(a2) & set(a2mo))
    check("2a status 도 같은 두 코드를 싣는다(비교 대상 실재)",
          {"env_not_int", "ledger_absent"} <= set(
              ANOMALY_LINE.match(ln).group(1) for ln in a2 if ANOMALY_LINE.match(ln)),
          repr(a2))
    check("2b machine-origin 의 이상징후 줄 집합 == status 의 이상징후 줄 집합(바이트 동일)",
          bool(common) and set(a2mo) == set(a2), repr({"status": a2, "machine-origin": a2mo}))

    # ── 3 층1 이상도 발행 — 판정표 fixture 의 케이스로 원장을 만든다 ───────────
    FIXTURE = os.path.join(SELF, "fixtures", "mission-origin-corpus.json")
    with open(FIXTURE, encoding="utf-8") as f:
        corpus = json.load(f)
    want = {"L1-rotated-generation-hit": ("ledger_rotated", "machine"),
            "L1-substring-long-delivery": ("delivery_substring", "machine"),
            "L1-bad-line-mixed": ("ledger_bad_lines", "machine")}
    cases = {c["name"]: c for c in corpus["layer1"] if c["name"] in want}
    check("3a fixture 케이스 3건 실재", len(cases) == 3, repr(sorted(cases)))
    for name, (code, token) in sorted(want.items()):
        c = cases.get(name)
        if c is None:
            continue
        c = json.loads(json.dumps(c))
        c["now_epoch"] = time.time()          # 서브프로세스는 실시간으로 읽는다 — 오프셋을 지금에 붙인다
        d3, e3 = fresh()
        e3["CYS_SURFACE_ID"] = c.get("my_surface") or ""
        dp = ledger_path_for(e3)
        if not dp or not os.path.abspath(dp).startswith(os.path.abspath(d3)):
            check("3 %s 밀폐(원장 경로가 임시 디렉터리 안)" % name, False, repr(dp))
            continue
        os.makedirs(os.path.dirname(dp), exist_ok=True)
        led = c["ledger"]
        body = led["raw"] if isinstance(led.get("raw"), str) else jm._corpus_ledger_lines(c, 0)
        with open(dp, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        g1 = "" if isinstance(led.get("raw"), str) else jm._corpus_ledger_lines(c, 1)
        if g1:
            with open(dp + ".1", "w", encoding="utf-8", newline="\n") as f:
                f.write(g1)
        r3 = run(["machine-origin"], e3, stdin=hook_json(jm._corpus_text(c["prompt"])))
        a3 = anomaly_lines(r3.stderr)
        got = [ANOMALY_LINE.match(ln).group(1) for ln in a3 if ANOMALY_LINE.match(ln)]
        check("3 %s: %s 발행 · 형식 일치 · 토큰 %s · stdout 1줄" % (name, code, token),
              code in got and all(ANOMALY_LINE.match(ln) for ln in a3)
              and r3.stdout == "machine-origin: %s\n" % token,
              repr((r3.returncode, r3.stdout, a3)))

    # ── 4 이상이 없으면 이상징후 줄 0 ─────────────────────────────────────────
    c4 = json.loads(json.dumps([c for c in corpus["layer1"] if c["name"] == "L1-exact-match"][0]))
    c4["now_epoch"] = time.time()
    d4, e4 = fresh()
    e4["CYS_SURFACE_ID"] = c4.get("my_surface") or ""
    dp4 = ledger_path_for(e4)
    if not dp4 or not os.path.abspath(dp4).startswith(os.path.abspath(d4)):
        print("FAIL 4 밀폐(원장 경로가 임시 디렉터리 안) — %r" % dp4)
        sys.exit(1)
    os.makedirs(os.path.dirname(dp4), exist_ok=True)
    with open(dp4, "w", encoding="utf-8", newline="\n") as f:
        f.write(jm._corpus_ledger_lines(c4, 0))
    r4 = run(["machine-origin"], e4, stdin=hook_json(jm._corpus_text(c4["prompt"])))
    check("4 정상 원장·정상 env: 이상징후 줄 0 · 토큰 machine",
          anomaly_lines(r4.stderr) == [] and r4.stdout == "machine-origin: machine\n"
          and r4.returncode == 0, repr((r4.returncode, r4.stdout, r4.stderr[-300:])))

    # ── 5 --help/-h 는 stdin 을 읽기 전에 답한다(멈춤 = FAIL) ─────────────────
    def help_call(cmd, flag):
        d5, e5 = fresh()
        p = subprocess.Popen([PY, "-B", MISSION, cmd, flag], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=e5)
        box = {}

        def waiter():
            # stdin 파이프는 **열어 둔 채** 아무것도 쓰지 않는다 — 읽으려 들면 영원히 기다린다.
            box["out"], box["err"] = p.stdout.read(), p.stderr.read()
            box["rc"] = p.wait()
        t = threading.Thread(target=waiter, daemon=True)
        t.start()
        t.join(HANG_LIMIT_S)
        hung = t.is_alive()
        if hung:
            p.kill()
            t.join(5)
        p.stdin.close()
        return hung, box.get("rc"), (box.get("out") or b"").decode("utf-8", "replace"), \
            (box.get("err") or b"").decode("utf-8", "replace")

    for cmd in ("record", "machine-origin", "hook-triage"):
        for flag in ("--help", "-h"):
            hung, rc, out, err = help_call(cmd, flag)
            check("5 %s %s: %.0fs 안에 끝남 · exit 0 · stdout 이 도움말" % (cmd, flag, HANG_LIMIT_S),
                  (not hung) and rc == 0 and out.startswith("usage: javis_mission.py"),
                  repr((hung, rc, out[:80], err[-200:])))

    # ── 6 범위: --help 가 첫 인자가 아니면 분기하지 않는다 ───────────────────
    d6, e6 = fresh()
    r6 = run(["record", "x", "--help"], e6, stdin=hook_json(OWNER_PROMPT))
    check("6a record x --help 는 도움말로 새지 않고 stdin 을 판정한다(exit 0 · stdout 0바이트)",
          r6.returncode == 0 and r6.stdout == "", repr((r6.returncode, r6.stdout[:80])))
    d6b, e6b = fresh()
    r6b = run(["status", "--help"], e6b)
    check("6b status --help 는 이 분기 밖(stdin 비판독 명령 · 종전 동작 = 판정 exit 1)",
          r6b.returncode == 1 and not r6b.stdout.startswith("usage:"),
          repr((r6b.returncode, r6b.stdout[:80])))

    # ── 7 판정 경로 표 — exit 와 stdout 이 수정 전과 같다 ────────────────────
    #    기대값 = 수정 전 파일(v0.14.51 출하본)로 잰 값. 경로는 케이스 디렉터리를 <D> 로 표준화한다.
    LABELED = "[보고] 작업 완료 — 다음 단계로 넘어간다"
    table = [
        ("record/owner", ["record"], hook_json(OWNER_PROMPT), {}, 0, ""),
        ("record/parse-fail", ["record"], "{깨진", {}, 2, ""),
        ("status/none", ["status"], None, {}, 1, ""),
        ("status/json-none", ["status", "--json"], None, {}, 1, "<json>"),
        ("set/no-cys", ["set", "임무 텍스트 하나"], None, {"no_cys": True}, 1, ""),
        ("clear/none", ["clear"], None, {}, 0,
         "[mission] 임무 대장 폐기 — 다음 자율 착수는 오너 지정 전까지 금지된다(명시 폐기)\n"),
        ("path", ["path"], None, {}, 0, "<D>/state/mission.json\n"),
        ("delivery-path", ["delivery-path"], None, {}, 0, "<D>/state/delivery-base.jsonl\n"),
        ("machine-origin/owner", ["machine-origin"], hook_json(OWNER_PROMPT), {}, 1,
         "machine-origin: human\n"),
        ("machine-origin/label", ["machine-origin"], hook_json(LABELED), {}, 0,
         "machine-origin: machine\n"),
        ("machine-origin/parse-fail", ["machine-origin"], "{깨진", {}, 2,
         "machine-origin: unknown\n"),
        ("machine-origin/empty", ["machine-origin"], hook_json("   "), {}, 2,
         "machine-origin: unknown\n"),
        ("hook-triage/owner", ["hook-triage"], hook_json(OWNER_PROMPT), {}, 1,
         "record: rc=0\nmachine-origin: human\npath: <D>/state/mission.json\n"),
        ("hook-triage/label", ["hook-triage"], hook_json(LABELED), {}, 0,
         "record: rc=1\nmachine-origin: machine\npath: <D>/state/mission.json\n"),
        ("unknown-subcommand", ["nope"], None, {}, 64, ""),
    ]
    # status --json 의 고정값. `hooks_effective` 는 이 레인 팩의 진단 필드라(javis_preflight 판독 ·
    # None=미측정) 환경에 따라 갈릴 수 있으므로 값 대신 형(bool 또는 None)만 고정한다.
    STATUS_JSON = {"have_mission": False, "source": "ledger", "mission": None,
                   "reason": "임무 대장 없음 — 이 세션에 오너 임무 지정이 없다", "anomalies": []}
    for name, args, stdin, opt, want_rc, want_out in table:
        d7, e7 = fresh(no_cys=opt.get("no_cys", False))
        r7 = run(args, e7, stdin=stdin)
        out = r7.stdout.replace(d7, "<D>")
        if want_out == "<json>":
            try:
                obj = json.loads(out)
                he = obj.pop("hooks_effective", "<absent>")
                out_ok = obj == STATUS_JSON and (he is None or isinstance(he, bool))
            except Exception:
                out_ok = False
        else:
            out_ok = out == want_out
        check("7 %s: exit %d · stdout 고정값" % (name, want_rc),
              r7.returncode == want_rc and out_ok, repr((r7.returncode, out[:160])))

    # ── 8 exit-code census — rc surface from CONTROL FLOW through main() ─────
    #    Method (printed below): (a) AST walk — start at main(), follow every `return` of main and of every handler in
    #    main's dispatch dict, resolving each returned value to its source (int literal, module EXIT_* constant, local
    #    variable -> its assignments, call of a module function -> that function's returns, `f()[0]` / tuple element
    #    -> element 0). Anything unresolvable FAILS the closure. (b) SET EQUALITY — the (tool, subcommand) rows below
    #    must equal main's dispatch keys + `--self-test` + the two main-level codes (64 unknown subcommand, 0 --help).
    #    (c) COVERAGE — every return/assignment site found in (a) must be executed by at least one row below; each row
    #    runs under a line tracer limited to javis_mission.py (fault injection only for except paths, by name).
    import ast
    SRC = open(MISSION, encoding="utf-8").read()
    TREE = ast.parse(SRC)
    FUNCS = {n.name: n for n in TREE.body if isinstance(n, ast.FunctionDef)}
    CONSTS = {n.targets[0].id: n.value.value for n in TREE.body
              if isinstance(n, ast.Assign) and len(n.targets) == 1 and isinstance(n.targets[0], ast.Name)
              and isinstance(n.value, ast.Constant) and type(n.value.value) is int}

    def own_nodes(fn):
        """fn's own statements and expressions - a nested def/lambda/class is a different function (its returns are
        not fn's returns), so the walk never enters it."""
        out, stack = [], list(fn.body)
        while stack:
            node = stack.pop()
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)):
                continue
            out.append(node)
            stack.extend(ast.iter_child_nodes(node))
        return out

    DISPATCH = {}
    for node in own_nodes(FUNCS["main"]):
        if isinstance(node, ast.Assign) and isinstance(node.value, ast.Dict) and \
                any(isinstance(t, ast.Name) and t.id == "table" for t in node.targets):
            DISPATCH = {k.value: v.id for k, v in zip(node.value.keys, node.value.values)}
    SITES, UNRESOLVED, MEMO = {}, [], {}

    def resolve(func, expr, idx):
        if isinstance(expr, ast.Constant) and type(expr.value) is int and idx is None:
            return {expr.value}
        if isinstance(expr, ast.Tuple) and idx is not None:
            return resolve(func, expr.elts[idx], None)
        if isinstance(expr, ast.Subscript) and isinstance(expr.slice, ast.Constant) and isinstance(expr.value, ast.Call):
            return resolve(func, expr.value, expr.slice.value)
        if isinstance(expr, ast.Call) and isinstance(expr.func, ast.Name):
            if expr.func.id in FUNCS:
                return walk(expr.func.id, idx)
            if func == "main" and expr.func.id == "fn":          # fn = table.get(cmd) — every dispatch target
                out = set()
                for target in DISPATCH.values():
                    out |= walk(target, idx)
                return out
        if isinstance(expr, ast.Name):
            if expr.id in CONSTS and idx is None:
                return {CONSTS[expr.id]}
            out, found = set(), False
            for node in own_nodes(FUNCS[func]):
                if not isinstance(node, ast.Assign):
                    continue
                for t in node.targets:
                    if isinstance(t, ast.Name) and t.id == expr.id:
                        found = True
                        SITES[(func, node.lineno)] = "assign %s = %s" % (expr.id, ast.unparse(node.value)[:60])
                        out |= resolve(func, node.value, idx)
                    elif isinstance(t, ast.Tuple):
                        for i, e in enumerate(t.elts):
                            if isinstance(e, ast.Name) and e.id == expr.id:
                                found = True
                                SITES[(func, node.lineno)] = "assign %s <- [%d] %s" % (expr.id, i, ast.unparse(node.value)[:60])
                                out |= resolve(func, node.value, i)
            if found:
                return out
        UNRESOLVED.append("%s:%s %s" % (func, getattr(expr, "lineno", "?"), ast.unparse(expr)[:80]))
        return set()

    def walk(func, idx):
        key = (func, idx)
        if key in MEMO:
            return MEMO[key]
        MEMO[key] = set()                                          # recursion guard
        vals = set()
        for node in own_nodes(FUNCS[func]):
            if isinstance(node, ast.Return):
                SITES[(func, node.lineno)] = "return %s" % (ast.unparse(node.value)[:60] if node.value else "")
                if node.value is None:
                    UNRESOLVED.append("%s:%d bare return" % (func, node.lineno))
                else:
                    vals |= resolve(func, node.value, idx)
        MEMO[key] = vals
        return vals

    MAIN_VALUES = walk("main", None)
    EXIT_CALLS = [n.lineno for n in ast.walk(TREE) if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
                  and n.func.attr in ("exit", "_exit") and isinstance(n.func.value, ast.Name)
                  and n.func.value.id in ("sys", "os")]
    RAISES = [n.lineno for n in ast.walk(TREE) if isinstance(n, ast.Raise) and n.exc is not None
              and "SystemExit" in ast.unparse(n.exc)]
    per_sub = {k: sorted(walk(v, None)) for k, v in DISPATCH.items()}
    print("8 method: AST walk of every return reachable from main() (values resolved to source), set equality of "
          "(tool, subcommand) rows vs dispatch keys, line coverage of every site under a tracer")
    print("   process exit calls (sys.exit/os._exit): lines %s · raise SystemExit: %s" % (EXIT_CALLS, RAISES))
    print("   dispatch keys (%d): %s" % (len(DISPATCH), sorted(DISPATCH)))
    for k in sorted(per_sub):
        print("   javis_mission.py %-15s rc %s" % (k, per_sub[k]))
    print("   javis_mission.py %-15s rc %s · main-level: 64 unknown subcommand, 0 --help on %s"
          % ("--self-test", sorted(walk("cmd_self_test", None)), "/".join(jm._STDIN_COMMANDS)
             if hasattr(jm, "_STDIN_COMMANDS") else "(absent)"))
    print("   sites found by the walk: %d in %d functions" % (len(SITES), len({f for f, _ in SITES})))
    check("8a closure: every returned value resolves to its source (unresolved 0)", not UNRESOLVED, repr(UNRESOLVED))
    check("8b one process exit site: sys.exit(main(sys.argv)) only (no other sys.exit/os._exit/raise SystemExit)",
          len(EXIT_CALLS) == 1 and not RAISES and "sys.exit(main(sys.argv))" in SRC, repr((EXIT_CALLS, RAISES)))
    check("8c main()'s rc surface is {0,1,2,64}", MAIN_VALUES == {0, 1, 2, 64}, repr(sorted(MAIN_VALUES)))

    # Census rows: (subcommand key, args, stdin, fixture, fault, expected rc). Expected rc = value measured on the OLD
    # v0.14.51 file for every row except the `<help>` row (new behaviour, decision A); see BEFORE-AFTER evidence.
    LABELED = "[보고] 작업 완료 — 다음 단계로 넘어간다"
    MATCH = "다음 액션 착수 — 원장에 있는 기계 배달 문장"
    HARNESS = "<system-reminder>\nThe task tools haven't been used recently.\n</system-reminder>"
    SURF = "mx"

    def mission_rec(**over):
        rec = {"schema": jm.SCHEMA_VERSION, "mission": "근본수정 임무", "source": "prompt", "reason": "fixture",
               "surface": SURF, "ts": "2026-10-20T00:00:00+0900", "ts_epoch": time.time() - 10,
               "boot_epoch": None, "ledger_status": "ok", "anomalies": []}
        rec.update(over)
        return rec

    def build(rig, fixture):
        state = os.path.join(rig, "state")
        env = make_env(rig)
        env["CYS_SURFACE_ID"] = SURF
        keep = {k: v for k, v in os.environ.items() if k.startswith(SEAT_PREFIXES)}
        for k in keep:
            del os.environ[k]
        os.environ.update({"CYS_STATE_DIR": state, "CYS_SURFACE_ID": SURF})
        try:
            mp, dp, ep = jm.ledger_path(), jm.delivery_ledger_path(), jm.delivery_epoch_path()
        finally:
            for k in [k for k in os.environ if k.startswith(SEAT_PREFIXES)]:
                del os.environ[k]
            os.environ.update(keep)
        for p in (mp, dp, ep):
            if not os.path.abspath(p).startswith(os.path.abspath(rig)):
                raise SystemExit("FAIL 8 밀폐: fixture path outside the rig: %r" % p)
            os.makedirs(os.path.dirname(p), exist_ok=True)

        def w(p, obj):
            with open(p, "w", encoding="utf-8") as f:
                f.write(obj if isinstance(obj, str) else json.dumps(obj, ensure_ascii=False))
        for part in fixture.split("+"):
            if part == "absent":
                pass
            elif part == "valid":
                w(mp, mission_rec())
            elif part == "envmission":
                env["CYS_MISSION"] = "환경변수 임무"
            elif part == "badjson":
                w(mp, "{not json")
            elif part == "nullmission":
                w(mp, mission_rec(mission=None, source="harness_notification"))
            elif part == "othersurface":
                w(mp, mission_rec(surface="other"))
            elif part == "nots":
                r = mission_rec()
                r.pop("ts_epoch")
                w(mp, r)
            elif part == "badts":
                w(mp, mission_rec(ts_epoch="x"))
            elif part == "expired":
                w(mp, mission_rec(ts_epoch=time.time() - jm.MISSION_TTL_S - 100))
            elif part == "epochnofile":
                w(mp, mission_rec(boot_epoch=1700000000.0))
            elif part == "epochmismatch":
                w(mp, mission_rec(boot_epoch=1700000000.0))
                w(ep, {"daemon_epoch": 1700000999.0})
            elif part == "epochmatch":
                w(mp, mission_rec(boot_epoch=1700000000.0))
                w(ep, {"daemon_epoch": 1700000000.0})
            elif part == "lsunreadable":
                w(mp, mission_rec(ledger_status="unreadable"))
            elif part == "delmatch":
                w(dp, jm._corpus_ledger_lines({"now_epoch": time.time(), "ledger": {"records": [
                    {"gen": 0, "surface": SURF, "ts_offset_s": -30.0, "body": MATCH, "origin": "send"}]}}, 0))
            elif part == "statefile":
                shutil.rmtree(state, ignore_errors=True)
                w(state, "not a directory")
            elif part.startswith("fakecys"):
                # A FAKE `cys` in the rig stands in for the owner-confirm channel (never the real cys): exit code given.
                b = os.path.join(rig, "fakebin")
                os.makedirs(b, exist_ok=True)
                with open(os.path.join(b, "cys"), "w") as f:
                    f.write("#!/bin/sh\nexit %s\n" % part[len("fakecys"):])
                os.chmod(os.path.join(b, "cys"), 0o755)
                env["PATH"] = b
            elif part == "nocys":
                b = os.path.join(rig, "emptybin")
                os.makedirs(b, exist_ok=True)
                env["PATH"] = b
            else:
                raise SystemExit("FAIL 8 unknown fixture %r" % part)
        return env, mp

    TRACER = r'''
import importlib.util, json, os, runpy, sys, threading
script, hits_path, fault = sys.argv[1], sys.argv[2], sys.argv[3]
target = os.path.abspath(script)
hits = set()
def local(frame, event, arg):
    if event == "line":
        hits.add(frame.f_lineno)
    return local
def glob(frame, event, arg):
    return local if frame.f_code.co_filename == target else None
def dump():
    with open(hits_path, "w") as f:
        json.dump(sorted(hits), f)
import atexit
atexit.register(dump)
sys.argv = [script] + sys.argv[4:]
sys.path.insert(0, os.path.dirname(target))
sys.dont_write_bytecode = True
if fault == "-":
    threading.settrace(glob); sys.settrace(glob)
    runpy.run_path(target, run_name="__main__")
else:
    spec = importlib.util.spec_from_file_location("javis_mission", target)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["javis_mission"] = mod
    threading.settrace(glob); sys.settrace(glob)
    spec.loader.exec_module(mod)
    name, action = fault.split("=")
    def _raise(*a, **k):
        raise RuntimeError("fault injected by test_mission_anomaly_emit: %s" % name)
    def _none(*a, **k):
        return None
    def _appendfail(fails, *a, **k):
        fails.append("fault injected by test_mission_anomaly_emit")
    setattr(mod, name, {"raise": _raise, "none": _none, "appendfail": _appendfail}[action])
    sys.exit(mod.main(sys.argv))
'''
    J = hook_json
    ROWS8 = [
        ("--self-test", ["--self-test"], None, "absent", "-", 0),
        ("--self-test", ["--self-test"], None, "absent", "_selftest_anomaly_registry=appendfail", 1),
        ("--self-test", ["--self-test"], None, "absent", "_detect_mod=none", 1),
        ("<unknown>", ["nope"], None, "absent", "-", 64),
        ("<help>", ["machine-origin", "--help"], "", "absent", "-", 0),
        ("status", ["status"], None, "envmission", "-", 0),
        ("status", ["status"], None, "badjson", "-", 2),
        ("status", ["status"], None, "absent", "-", 1),
        ("status", ["status"], None, "nullmission", "-", 1),
        ("status", ["status"], None, "othersurface", "-", 1),
        ("status", ["status"], None, "nots", "-", 2),
        ("status", ["status"], None, "badts", "-", 2),
        ("status", ["status"], None, "expired", "-", 1),
        ("status", ["status"], None, "epochnofile", "-", 1),
        ("status", ["status"], None, "epochmismatch", "-", 1),
        ("status", ["status"], None, "lsunreadable", "-", 1),
        ("status", ["status"], None, "valid", "-", 0),
        ("status", ["status"], None, "epochmatch", "-", 0),
        ("record", ["record"], J(OWNER_PROMPT), "absent", "_detect_mod=none", 2),
        ("record", ["record"], "{깨진", "absent", "-", 2),
        ("record", ["record"], J(""), "absent", "-", 1),
        ("record", ["record"], J(LABELED), "absent", "-", 1),
        ("record", ["record"], J(HARNESS), "absent", "-", 1),
        ("record", ["record"], J("cys boot"), "absent", "-", 1),
        ("record", ["record"], J("너는 마스터다. 윈도우 실사고 근본수정 진행해"), "absent", "-", 0),
        ("record", ["record"], J(OWNER_PROMPT), "absent", "-", 0),
        ("set", ["set"], None, "nocys", "-", 64),
        ("set", ["set", "임무 텍스트 하나"], None, "nocys", "-", 1),
        ("set", ["set", "임무 텍스트 하나"], None, "fakecys0+statefile", "-", 2),
        ("set", ["set", "임무 텍스트 하나"], None, "fakecys0", "-", 0),
        ("clear", ["clear"], None, "statefile", "-", 2),
        ("clear", ["clear"], None, "valid", "-", 0),
        ("path", ["path"], None, "absent", "ledger_path=none", 2),
        ("path", ["path"], None, "absent", "-", 0),
        ("delivery-path", ["delivery-path"], None, "absent", "delivery_ledger_path=none", 2),
        ("delivery-path", ["delivery-path", "--json"], None, "absent", "-", 0),
        ("machine-origin", ["machine-origin"], "{깨진", "absent", "-", 2),
        ("machine-origin", ["machine-origin"], J("  "), "absent", "-", 2),
        ("machine-origin", ["machine-origin"], J(OWNER_PROMPT), "absent", "read_delivery=raise", 2),
        ("machine-origin", ["machine-origin"], J(MATCH), "delmatch", "-", 0),
        ("machine-origin", ["machine-origin"], J(OWNER_PROMPT), "absent", "-", 1),
        ("hook-triage", ["hook-triage"], J(OWNER_PROMPT), "absent", "_machine_origin_step=raise", 2),
        ("hook-triage", ["hook-triage"], J(LABELED), "absent", "-", 0),
        ("hook-triage", ["hook-triage"], J(OWNER_PROMPT), "absent", "-", 1),
    ]
    covered, results = set(), []
    for i, (sub, args, stdin, fixture, fault, want) in enumerate(ROWS8):
        rig = os.path.join(root, "census%02d" % i)
        os.makedirs(rig)
        if os.name == "nt" and "fakecys" in fixture:
            results.append((sub, args, fixture, fault, want, "SKIP(nt: fake cys is a sh script)"))
            continue
        env8, _mp8 = build(rig, fixture)
        hits_path = os.path.join(rig, "hits.json")
        r8 = subprocess.run([PY, "-B", "-c", TRACER, MISSION, hits_path, fault] + args, capture_output=True,
                            text=True, timeout=600, env=env8, input=stdin if stdin is not None else "", cwd=rig)
        try:
            with open(hits_path) as f:
                covered |= set(json.load(f))
        except Exception:
            pass
        results.append((sub, args, fixture, fault, want, r8.returncode))
    print("   census rows (tool javis_mission.py):")
    for sub, args, fixture, fault, want, got in results:
        print("     %-15s %-34s fixture=%-18s fault=%-38s want %-3s got %s"
              % (sub, " ".join(args)[:34], fixture, fault, want, got))
    bad_rc = [(r[0], " ".join(r[1]), r[2], r[3], r[4], r[5]) for r in results if r[5] != r[4]]
    check("8d every census row returns its expected rc (%d rows)" % len(results), not bad_rc, repr(bad_rc))
    rows_set = {r[0] for r in results}
    want_set = set(DISPATCH) | {"--self-test", "<unknown>", "<help>"}
    check("8e SET EQUALITY: census (tool, subcommand) set == main dispatch keys + --self-test + 64 + help (%d)"
          % len(want_set), rows_set == want_set,
          "missing %s extra %s" % (sorted(want_set - rows_set), sorted(rows_set - want_set)))
    exit_line = [i + 1 for i, ln in enumerate(SRC.splitlines()) if ln.strip() == "sys.exit(main(sys.argv))"]
    missing_sites = [("%s:%d" % (f, ln), SITES[(f, ln)]) for (f, ln) in sorted(SITES, key=lambda x: x[1])
                     if ln not in covered]
    missing_sites += [("<module>:%d" % ln, "sys.exit(main(sys.argv))") for ln in exit_line if ln not in covered]
    print("   coverage: %d return/assignment sites + %d process exit line · not executed %d"
          % (len(SITES), len(exit_line), len(missing_sites)))
    for (f, ln) in sorted(SITES, key=lambda x: x[1]):
        print("     site %-22s :%-5d %-3s %s" % (f, ln, "hit" if ln in covered else "MISS", SITES[(f, ln)]))
    check("8f COVERAGE: every return/assignment site reachable from main() and the exit line were executed (%d+%d)"
          % (len(SITES), len(exit_line)), not missing_sites and len(exit_line) == 1, repr(missing_sites))

    # ── 9 --help/-h with a prompt on stdin judges and records NOTHING (decision A) ──
    #    The v0.14.51 bytes had no help check before the handler: `record --help` reached cmd_record, read the prompt
    #    on stdin and recorded it as a mission (javis_mission.py module docstring: a sentence on stdin is recorded and
    #    flips the gate). The pre-handler return removes that. Measured here as harm, not as rc: the mission state file
    #    must be byte-unchanged - absent stays absent, an existing ledger keeps its exact bytes.
    #    Known property (documented, not a defect): for record / machine-origin / hook-triage, exit 0 now means EITHER
    #    "help printed, nothing judged" (the pre-handler return) OR a real verdict 0 - rc alone does not discriminate;
    #    a consumer that needs the verdict must not pass --help/-h (no caller does - see the PROMOTE-3 census).
    for cmd in ("record", "machine-origin", "hook-triage"):
        for flag in ("--help", "-h"):
            for start in ("absent", "valid"):
                rig = os.path.join(root, "harm-%s-%s-%s" % (cmd, flag.strip("-"), start))
                os.makedirs(rig)
                env9, mp9 = build(rig, start)
                before = open(mp9, "rb").read() if os.path.exists(mp9) else None
                r9 = subprocess.run([PY, "-B", MISSION, cmd, flag], capture_output=True, text=True, timeout=120,
                                    env=env9, input=hook_json(OWNER_PROMPT), cwd=rig)
                after = open(mp9, "rb").read() if os.path.exists(mp9) else None
                check("9 %s %s + prompt on stdin (ledger %s): mission file byte-unchanged · help - no judgment (exit 0, usage)"
                      % (cmd, flag, start),
                      before == after and r9.returncode == 0 and r9.stdout.startswith("usage: javis_mission.py"),
                      repr((r9.returncode, r9.stdout[:40], "unchanged" if before == after else
                            "CHANGED: %r -> %r" % (before and before[:60], after and after[:80]))))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("MISSION-ANOMALY-EMIT-OK")
sys.exit(0)
