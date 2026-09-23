#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_bootstrap_resource_recheck.py — ④′ 부하 재확인(U11 · 0.14.41) 통합 회귀.

오너 지시(2026-09-23): 설치 직후처럼 **부하가 잠깐 높을 때** 한 번 재고 바로 멈추지 말고
30초 간격으로 최대 3분 다시 확인한다. 설계 정본(수정설계-0.14.41 §3 U11) 요지:
  · 재확인은 자원 게이트가 hard-block 이고 hard 트립이 **전부 fleet_cpu_ratio** 일 때만
    (비윈도우 · 측정 성공 `fleet_cpu_reason == "ok"`). servers·nodes·context 는 기다려도 안 풀린다.
  · 벽시계 마감 t0+TOTAL · 간격 INTERVAL 의 **절대 스케줄** · 재확인 회차에서 fleet 단독이면
    `cys list` 교차확인 생략 · 회차 수 int(TOTAL/INTERVAL+1e-9) · env 는 줄이기만(min(env,180)) ·
    TOTAL=0 이면 종전(1회).
  · 회차마다 boot-last `progress` 갱신(result.state 새 값 없음) · 대기 중 스폰·큐·Feed 0 ·
    최종 알림 1회 · 대기 뒤 ④ 진입 전 master 결속 재확인 · 처방에 '살아 있는 좌석을 닫지 마라'.

관측 기법: test_dept_doctrine_v1 계승 — 격리 HOME + PATH 앞 목 `cys`(호출 로깅) + 목 팩
(순서 응답 자원 게이트 · 목 orchestra). 라이브 데몬·라이브 팩 무접촉 · 노드 스폰 0.
테스트는 간격 0.5s·총 1.5s 로 축소한다(env 는 줄이기만 허용 — 그 자체가 핀 (j)).

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_bootstrap_resource_recheck.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 BOOT-RESOURCE-RECHECK-OK.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True      # 팩 봉인(SEAL-1) 정신 — 검체가 bin/ 에 캐시를 남기지 않는다

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
BOOTSTRAP = os.path.join(BIN, "javis_bootstrap.py")
PY = sys.executable or "python3"

INTERVAL = 0.5
TOTAL = 1.5
N_MAX = int(TOTAL / INTERVAL + 1e-9)      # 3 → 최대 측정 1 + 3 = 4회

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _w(path, body, mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


# ── 게이트 응답 형상 ──────────────────────────────────────────────────────────
def fleet_hard(value=1.3, reason="ok"):
    return {"exit": 2, "json": {
        "verdict": "hard_block",
        "trips": [{"metric": "fleet_cpu_ratio", "level": "hard", "value": value,
                   "soft": 0.5, "hard": 1.0}],
        "measured": {"fleet_cpu_ratio": value, "fleet_cpu_reason": reason, "ncpu": 4,
                     "fleet_cpu_top": [{"pid": 4242, "exe": "claude", "owner": "claude",
                                        "pcpu": 260.0}],
                     "nodes_hard_effective": 18, "measure_errors": []},
        "warnings": ["context_unmeasured"]}}


def servers_hard():
    return {"exit": 2, "json": {
        "verdict": "hard_block",
        "trips": [{"metric": "servers", "level": "hard", "value": 5}],
        "measured": {"nodes_hard_effective": 18, "fleet_cpu_reason": "ok",
                     "fleet_cpu_ratio": 0.1, "measure_errors": []},
        "warnings": ["context_unmeasured"]}}


def fleet_and_servers_hard():
    r = fleet_hard()
    r["json"]["trips"].append({"metric": "servers", "level": "hard", "value": 5})
    return r


def allow():
    return {"exit": 0, "json": {"verdict": "allow", "trips": [],
                                "measured": {"fleet_cpu_ratio": 0.2, "fleet_cpu_reason": "ok",
                                             "measure_errors": []},
                                "warnings": ["context_unmeasured"]}}


def soft_fleet():
    return {"exit": 1, "json": {
        "verdict": "soft_warn",
        "trips": [{"metric": "fleet_cpu_ratio", "level": "soft", "value": 0.7,
                   "soft": 0.5, "hard": 1.0}],
        "measured": {"fleet_cpu_ratio": 0.7, "fleet_cpu_reason": "ok", "measure_errors": []},
        "warnings": ["context_unmeasured"]}}


def soft_windows_shape():
    """윈도우 게이트의 실제 형상(코드상 필연): getloadavg 부재 → measure_errors → 최소 soft,
    fleet_cpu 는 ps 부재로 absent(측정 실패 아님) → 트립 없음."""
    return {"exit": 1, "json": {
        "verdict": "soft_warn", "trips": [],
        "measured": {"fleet_cpu_ratio": None, "fleet_cpu_reason": "absent",
                     "measure_errors": ["load(getloadavg)"]},
        "warnings": ["measure_error:load(getloadavg)", "context_unmeasured"]}}


# ── 격리 부트 하네스 ───────────────────────────────────────────────────────────
class Rig:
    """격리 HOME + 목 cys + 목 팩. `responses` 는 게이트 호출 순서대로의 응답(끝은 반복)."""

    def __init__(self, responses, gate_delay=0.0, status_after=None, status_on_call=2,
                 env_extra=None, surface="5"):
        self.home = tempfile.mkdtemp(prefix="u11-rig-")
        self.pack = os.path.join(self.home, ".cys", "pack")        # base 레인 ↔ 메인 팩(정합)
        self.mockbin = os.path.join(self.home, "mockbin")
        self.cys_log = os.path.join(self.home, "cys-calls.log")
        self.gate_log = os.path.join(self.home, "gate-calls.log")
        self.list_file = os.path.join(self.home, "cys-list.txt")
        self.status_file = os.path.join(self.home, "cys-status.json")
        self.resp_file = os.path.join(self.home, "gate-responses.json")
        self.counter = os.path.join(self.home, "gate-counter")
        open(self.list_file, "w").close()            # 빈 목록 → 라이브 노드 0 → 결손>0 → 게이트 발동
        with open(self.resp_file, "w", encoding="utf-8") as f:
            json.dump({"responses": responses, "delay": gate_delay,
                       "status_after": status_after, "status_on_call": status_on_call,
                       "status_file": self.status_file}, f)
        _w(os.path.join(self.mockbin, "cys"),
           '#!/bin/bash\n'
           'echo "$@" >> "%s" 2>/dev/null\n'
           'case "$1" in\n'
           '  surface-role) echo "master"; exit 0;;\n'
           '  list) [ -f "%s" ] && cat "%s"; exit 0;;\n'
           '  status) [ -f "%s" ] && cat "%s"; exit 0;;\n'
           '  *) exit 0;;\n'
           'esac\n' % (self.cys_log, self.list_file, self.list_file,
                       self.status_file, self.status_file))
        binp = os.path.join(self.pack, "bin")
        _w(os.path.join(binp, "javis_orchestra.py"),
           '#!/usr/bin/env python3\nimport sys\n'
           'sys.exit(0)\n')
        # 목 게이트: 호출 카운터로 응답을 고른다 · 시작/끝 시각을 로그에 남긴다(절대 스케줄 핀) ·
        # status_on_call 번째 호출에서 status 파일을 쓴다(대기 중 master 좌석 변화 재현).
        _w(os.path.join(binp, "javis_resource_gate.py"),
           '#!/usr/bin/env python3\n'
           'import json, os, sys, time\n'
           't0 = time.time()\n'
           'cfg = json.load(open(%r, encoding="utf-8"))\n'
           'cnt = %r\n'
           'n = int(open(cnt).read()) if os.path.exists(cnt) else 0\n'
           'n += 1\n'
           'open(cnt, "w").write(str(n))\n'
           'if cfg.get("status_after") is not None and n >= cfg.get("status_on_call", 2):\n'
           '    open(cfg["status_file"], "w").write(cfg["status_after"])\n'
           'if cfg.get("delay"):\n'
           '    time.sleep(cfg["delay"])\n'
           'rs = cfg["responses"]\n'
           'r = rs[min(n, len(rs)) - 1]\n'
           'print(json.dumps(r["json"]))\n'
           'open(%r, "a").write("%%d %%.4f %%.4f %%s\\n" %% (n, t0, time.time(), " ".join(sys.argv[1:])))\n'
           'sys.exit(r["exit"])\n' % (self.resp_file, self.counter, self.gate_log))
        with open(os.path.join(self.pack, "agents.json"), "w", encoding="utf-8") as f:
            json.dump({a: {"cmd": PY} for a in ("claude", "gemini", "codex")}, f)
        self.env = self._env(env_extra, surface)

    def _env(self, extra, surface):
        env = {k: v for k, v in os.environ.items()
               if not (k.startswith("CYS_") or k.startswith("JAVIS_") or k.startswith("AITERM_"))}
        env.update({"HOME": self.home, "CYS_PACK_DIR": self.pack,
                    "PATH": self.mockbin + os.pathsep + os.environ.get("PATH", ""),
                    "CYS_BOOT_CHECK_RETRIES": "1", "CYS_BOOT_CHECK_INTERVAL_S": "0",
                    "CYS_BOOT_RESOURCE_RECHECK_INTERVAL_S": str(INTERVAL),
                    "CYS_BOOT_RESOURCE_RECHECK_TOTAL_S": str(TOTAL)})
        if surface:
            env["CYS_SURFACE_ID"] = surface
        env.update(extra or {})
        return env

    def run(self):
        t = time.monotonic()
        r = subprocess.run([PY, BOOTSTRAP, "run"], capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=180, env=self.env)
        self.elapsed = time.monotonic() - t
        self.rc, self.stdout, self.stderr = r.returncode, r.stdout, r.stderr
        state = os.path.join(self.home, ".cys", "state")
        self.data = None
        for cand in sorted(os.listdir(state) if os.path.isdir(state) else []):
            if cand.startswith("boot-last") and cand.endswith(".json"):
                with open(os.path.join(state, cand), encoding="utf-8") as f:
                    self.data = json.load(f)
                break
        return self

    # ── 관측 ──
    def gate_calls(self):
        if not os.path.isfile(self.gate_log):
            return []
        out = []
        for line in open(self.gate_log, encoding="utf-8"):
            p = line.split()
            if len(p) >= 3:
                out.append((int(p[0]), float(p[1]), float(p[2])))
        return out

    def cys_calls(self):
        if not os.path.isfile(self.cys_log):
            return []
        return [l.rstrip("\n") for l in open(self.cys_log, encoding="utf-8")]

    def count(self, prefix):
        return sum(1 for l in self.cys_calls() if l.split()[:len(prefix.split())] == prefix.split())

    def steps(self):
        return [s["step"] for s in (self.data or {}).get("steps", [])]

    def result(self):
        return (self.data or {}).get("result") or {}

    def cleanup(self):
        shutil.rmtree(self.home, ignore_errors=True)


def _feed_bodies(rig):
    return [l for l in rig.cys_calls() if l.startswith("feed push")]


LIVE_MASTER = json.dumps({"surfaces": [{"surface_id": 5, "role": "master", "exited": False}]})
GONE_MASTER = json.dumps({"surfaces": [{"surface_id": 5, "role": "master", "exited": True}]})
MOVED_MASTER = json.dumps({"surfaces": [{"surface_id": 5, "role": "", "exited": True},
                                        {"surface_id": 9, "role": "master", "exited": False}]})


# ── (a) hard(fleet) → hard(fleet) → allow : 기다려서 이어간다 ────────────────────
rig = Rig([fleet_hard(), fleet_hard(1.2), allow()], status_after=LIVE_MASTER).run()
calls = rig.gate_calls()
check("a1 부하 해소 → 부트 계속(rc 0)", rig.rc == 0, "rc=%s stderr=%r" % (rig.rc, rig.stderr[-400:]))
check("a2 게이트 3회 측정(최초 1 + 재확인 2 — 해소 즉시 이탈)", len(calls) == 3, "calls=%d" % len(calls))
check("a3 팀 기동(cys boot) 진입", rig.count("boot") >= 1, "cys calls=%r" % rig.cys_calls()[-8:])
check("a4 재확인은 같은 단계의 #N 접미로 기록(새 단계 라벨 0)",
      "④′resource-gate" in rig.steps() and "④′resource-gate#2" in rig.steps()
      and "④′resource-gate#3" in rig.steps(), "steps=%r" % rig.steps())
check("a5 해소(allow)면 Feed 알림 0", len(_feed_bodies(rig)) == 0, "feed=%r" % _feed_bodies(rig))
prog = (rig.data or {}).get("progress") or {}
check("a6 boot-last progress 기록(phase=resource_wait · 종료 state=done · outcome=resolved · checks=3)",
      prog.get("phase") == "resource_wait" and prog.get("state") == "done"
      and prog.get("outcome") == "resolved" and prog.get("checks") == 3, "progress=%r" % prog)
check("a7 result.state 새 값 없음(완주=completed)",
      rig.result().get("state") == "completed", "result=%r" % rig.result())
check("a8 대기 뒤 master 결속 재확인 흔적(#bind · 살아 있는 master)",
      "④′resource-gate#bind" in rig.steps(), "steps=%r" % rig.steps())
check("a9 진행 표시가 stderr 에 남는다(사람 관찰자용)", "다시 확인" in rig.stderr, rig.stderr[-300:])
rig.cleanup()

# ── (b) hard(fleet) 지속 : 3분(축소판) 동안 N회 확인 후 1회만 알리고 멈춘다 ────────
rig = Rig([fleet_hard()]).run()
calls = rig.gate_calls()
check("b1 끝까지 부하 → exit 9", rig.rc == 9, "rc=%s" % rig.rc)
check("b2 측정 횟수 = 1 + int(TOTAL/INTERVAL) = %d" % (1 + N_MAX), len(calls) == 1 + N_MAX,
      "calls=%d" % len(calls))
check("b3 팀 기동(cys boot) 미호출", rig.count("boot") == 0)
fb = _feed_bodies(rig)
check("b4 Feed 알림 정확히 1회(대기 중 0 · 최종 1)", len(fb) == 1, "feed=%r" % fb)
check("b5 처방에 '살아 있는 좌석을 닫지 마' 명시", bool(fb) and "닫지 마" in fb[0], fb[0][:300] if fb else "")
check("b6 처방에 재선언 안내('너는 마스터다')", bool(fb) and "너는 마스터다" in fb[0])
rw = rig.result().get("resource_wait") or {}
check("b7 result.resource_wait 기계 필드(checks·trail·outcome)",
      rw.get("checks") == 1 + N_MAX and isinstance(rw.get("trail"), list)
      and len(rw.get("trail")) == 1 + N_MAX and rw.get("outcome") == "exhausted", "rw=%r" % rw)
check("b8 result 종전 계약 유지(failed · failed_step=resource-gate · exit 9)",
      rig.result().get("state") == "failed" and rig.result().get("failed_step") == "resource-gate"
      and rig.result().get("exit") == 9, "result=%r" % rig.result())
check("b10 대기 중 큐 전송(send) 0", rig.count("send") == 0, "cys calls=%r" % rig.cys_calls())
# 재확인 회차(fleet 단독 hard)는 cys list 교차확인을 생략한다 — 최초 측정의 1회만 남는다.
lists_b = rig.count("list")
rig.cleanup()

# ── (b9) 절대 스케줄 핀 — 게이트 소요를 주입해야 상대 스케줄(MU3)과 갈린다 ──────────
# ★리뷰1(2026-09-23): 위 rig(gate_delay=0.0)는 게이트 호출이 사실상 즉시 끝나서, 절대
#   스케줄(`due=t0+k×INTERVAL`)과 상대 스케줄(`due=now+INTERVAL`)의 드리프트가 실질적으로
#   같아진다(둘 다 ≈k×INTERVAL) — 그래서 옛 b9(문턱 0.2s)는 MU3(상대 스케줄 치환)를 못
#   잡았다(관측 드리프트 0.125s < 0.2s). 게이트 소요를 명시로 주입하면 갈린다: 절대 스케줄은
#   그 소요를 다음 회차 대기에서 **상쇄**하지만(due 는 t0 로부터 고정), 상대 스케줄은 매
#   회차 소요만큼 **누적**된다(회차 k 드리프트 ≈ k×게이트소요). 문턱을 게이트 소요에 비례시켜
#   (0.5×게이트) 잡으면 정상 구현(드리프트≈스폰 지터)은 통과하고 MU3(드리프트≈k×게이트)는
#   1회차부터 즉시 떨어진다.
GATE_DELAY_B9 = 0.15
rig9 = Rig([fleet_hard()], gate_delay=GATE_DELAY_B9).run()
calls9 = rig9.gate_calls()
# ★CI 러너 지터(2026-09-23 · ci-branch run 35884936211): 판정을 '최대 편차'로 두면 느린 러너에서
#   한 회차만 늦게 뜬 스폰 지터(starts=[0, .534, 1.166, 1.537] — 2회차만 +0.166s, 3회차는 +0.037s 로
#   제자리 복귀)가 RED 를 만든다. 절대 스케줄의 지터는 **회차마다 독립·비누적**이고 상대 스케줄(MU3)의
#   드리프트는 **누적**(≈k×게이트: .15/.30/.45)이므로, 재확인 회차(k≥1) 편차의 **중앙값**으로 가른다 —
#   지터 1회는 흡수하고 MU3 는 중앙값 .30 ≥ 문턱이라 그대로 떨어진다(판별력 유지).
if len(calls9) >= 2:
    first9 = calls9[0][1]
    starts9 = [c[1] - first9 for c in calls9]
    drifts9 = sorted(abs(s - k * INTERVAL) for k, s in enumerate(starts9) if k >= 1)
    med9 = drifts9[len(drifts9) // 2]
    thresh9 = 0.5 * GATE_DELAY_B9
    check("b9 절대 스케줄(게이트 소요 %.2fs 주입 · 재확인 편차 중앙값 %.3fs < %.3fs=0.5×게이트소요 · 최대 %.3fs)"
          % (GATE_DELAY_B9, med9, thresh9, drifts9[-1]), med9 < thresh9,
          "starts=%r" % [round(s, 3) for s in starts9])
else:
    check("b9 절대 스케줄(측정 불충분)", False, "calls=%d" % len(calls9))
rig9.cleanup()

rig0 = Rig([fleet_hard()], env_extra={"CYS_BOOT_RESOURCE_RECHECK_TOTAL_S": "0"}).run()
check("b11 재확인 회차는 cys list 교차확인 생략(추가 호출 0)", lists_b == rig0.count("list"),
      "재확인 런 list=%d · 1회 런 list=%d" % (lists_b, rig0.count("list")))
# ── (g) 롤백 손잡이 TOTAL=0 → 종전 1회 동작 ──
check("g1 TOTAL=0 → 게이트 1회 · exit 9(종전)", len(rig0.gate_calls()) == 1 and rig0.rc == 9,
      "calls=%d rc=%s" % (len(rig0.gate_calls()), rig0.rc))
rig0.cleanup()

# ── (c) hard(servers) : 기다려도 안 풀리는 축 → 대기 없이 종전대로 ────────────────────
rig = Rig([servers_hard()]).run()
check("c1 servers hard → 게이트 1회·exit 9(대기 0)", len(rig.gate_calls()) == 1 and rig.rc == 9,
      "calls=%d rc=%s" % (len(rig.gate_calls()), rig.rc))
check("c2 대기 흔적 없음(progress 부재)", "progress" not in (rig.data or {}), repr((rig.data or {}).get("progress")))
rig.cleanup()

# ── (c′) fleet+servers 혼합 hard → 재확인 대상 아님 ──
rig = Rig([fleet_and_servers_hard()]).run()
check("c3 fleet+servers 혼합 hard → 게이트 1회·exit 9", len(rig.gate_calls()) == 1 and rig.rc == 9,
      "calls=%d rc=%s" % (len(rig.gate_calls()), rig.rc))
rig.cleanup()

# ── (d) 윈도우 형상 soft 지속 : 재확인하면 윈도우 부트가 매번 3분 늦어진다 → 1회 ─────────
rig = Rig([soft_windows_shape()]).run()
check("d1 soft(측정 실패형) → 게이트 1회 · 진행(rc 0)", len(rig.gate_calls()) == 1 and rig.rc == 0,
      "calls=%d rc=%s" % (len(rig.gate_calls()), rig.rc))
check("d2 soft 경로 대기 0(progress 부재)", "progress" not in (rig.data or {}))
rig.cleanup()

# ── (d′) 측정 실패형 fleet hard(reason≠ok)는 재확인하지 않는다 ──
rig = Rig([fleet_hard(reason="override")]).run()
check("d3 fleet_cpu_reason≠ok 인 hard → 재확인 없음(게이트 1회)", len(rig.gate_calls()) == 1,
      "calls=%d" % len(rig.gate_calls()))
rig.cleanup()

# ── (e) hard(fleet) → hard(servers) : 다른 축이 나오면 즉시 이탈 ─────────────────────
rig = Rig([fleet_hard(), servers_hard()]).run()
check("e1 2회째에 servers 축 → 즉시 exit 9(게이트 2회)", len(rig.gate_calls()) == 2 and rig.rc == 9,
      "calls=%d rc=%s" % (len(rig.gate_calls()), rig.rc))
check("e2 알림 1회", len(_feed_bodies(rig)) == 1, "feed=%r" % _feed_bodies(rig))
rig.cleanup()

# ── (f) hard(fleet) → soft : 진행 + soft 알림 1회(종전 soft 경로) ─────────────────────
rig = Rig([fleet_hard(), soft_fleet()], status_after=LIVE_MASTER).run()
check("f1 soft 로 내려오면 진행(rc 0)", rig.rc == 0, "rc=%s" % rig.rc)
check("f2 soft 알림 정확히 1회", len(_feed_bodies(rig)) == 1, "feed=%r" % _feed_bodies(rig))
check("f3 팀 기동 진입", rig.count("boot") >= 1)
rig.cleanup()

# ── (h) 느린 게이트 : 벽시계 마감이 횟수보다 우선(최악 = TOTAL + 게이트 1회) ─────────────
SLOW = 0.8
rig = Rig([fleet_hard()], gate_delay=SLOW).run()
calls = rig.gate_calls()
span = (calls[-1][2] - calls[0][1]) if calls else 0.0
check("h1 느린 게이트에서 측정 횟수가 마감으로 줄어든다(< 1 + N_MAX)", 0 < len(calls) < 1 + N_MAX,
      "calls=%d" % len(calls))
check("h2 총 대기 ≤ TOTAL + 게이트 1회 + 여유(%.2fs)" % span, span <= TOTAL + SLOW + 0.6,
      "span=%.3f" % span)
check("h3 끝까지 부하 → exit 9", rig.rc == 9, "rc=%s" % rig.rc)
rig.cleanup()

# ── (k) 대기 중 master 좌석이 사라졌다 → 지휘자 없는 팀을 띄우지 않는다 ──────────────────
rig = Rig([fleet_hard(), allow()], status_after=GONE_MASTER).run()
check("k1 master 부재 → 팀 기동 0(cys boot 미호출)", rig.count("boot") == 0, "cys=%r" % rig.cys_calls()[-6:])
check("k2 exit 7(이 surface 는 master 아님 — 기존 exit 공간)", rig.rc == 7, "rc=%s" % rig.rc)
check("k3 result: declined · reason=master_gone(새 state 0)",
      rig.result().get("state") == "declined" and rig.result().get("reason") == "master_gone",
      "result=%r" % rig.result())
check("k4 알림 1회", len(_feed_bodies(rig)) == 1, "feed=%r" % _feed_bodies(rig))
rig.cleanup()

# ── (k′) master 가 다른 창으로 옮겨졌다 → 그 master 의 팀으로 계속(선언이 접히지 않게) ─────
rig = Rig([fleet_hard(), allow()], status_after=MOVED_MASTER).run()
check("k5 다른 살아 있는 master → 계속(cys boot 호출 · rc 0)",
      rig.count("boot") >= 1 and rig.rc == 0, "rc=%s" % rig.rc)
rig.cleanup()

# ── (k″) 상태 판독 불가 → 종전 동작(진행 · fail-open) ──
rig = Rig([fleet_hard(), allow()], status_after="not-json").run()
check("k6 master 판독 불가 → 진행(종전 동작 · rc 0)", rig.rc == 0 and rig.count("boot") >= 1,
      "rc=%s" % rig.rc)
rig.cleanup()

# ── (i) 순수 판정 + 윈도우 가드 (모듈 직접 import — 부작용 없는 최상위) ───────────────
sys.path.insert(0, BIN)
try:
    import javis_bootstrap as B          # noqa: E402
    imported = True
except Exception as e:                   # noqa: BLE001
    imported = False
    check("i0 javis_bootstrap import", False, repr(e))
if imported:
    rr = getattr(B, "_resource_recheckable", None)
    check("i1 순수 판정 함수 실재", callable(rr))
    if callable(rr):
        fj = fleet_hard()["json"]
        check("i2 fleet 단독 hard(비윈도우) → True", rr("hard-block", fj, windows=False) is True)
        check("i3 윈도우 호스트 → False(구조적 무진입)", rr("hard-block", fj, windows=True) is False)
        check("i4 soft → False", rr("soft", fj, windows=False) is False)
        check("i5 hard-overcount → False", rr("hard-overcount", fj, windows=False) is False)
        check("i6 json None(축 미상) → False", rr("hard-block", None, windows=False) is False)
        check("i7 fleet+servers → False",
              rr("hard-block", fleet_and_servers_hard()["json"], windows=False) is False)
        check("i8 nodes 단독 → False", rr("hard-block", {"trips": [
            {"metric": "nodes", "level": "hard", "value": 40}],
            "measured": {"fleet_cpu_reason": "ok"}}, windows=False) is False)
        check("i9 load_ratio hard(가상) → False(재확인 축은 fleet_cpu 단일)",
              rr("hard-block", {"trips": [{"metric": "load_ratio", "level": "hard", "value": 3}],
                                "measured": {"fleet_cpu_reason": "ok"}}, windows=False) is False)
        check("i10 fleet_cpu_reason absent → False",
              rr("hard-block", fleet_hard(reason="absent")["json"], windows=False) is False)
    wh = getattr(B, "_recheck_windows_host", None)
    check("i11 윈도우 호스트 판정 함수 실재", callable(wh))
    if callable(wh):
        try:
            import javis_resource_gate as G   # noqa: E402
            saved = {k: os.environ.get(k) for k in ("MSYSTEM", "WINDIR", "SYSTEMROOT", "OS")}
            combos = [{}, {"MSYSTEM": "MINGW64"}, {"MSYSTEM": "MINGW64", "WINDIR": "C:\\Windows"},
                      {"MSYSTEM": "MINGW64", "OS": "Windows_NT"}, {"WINDIR": "C:\\Windows"}]
            agree = True
            try:
                for c in combos:
                    for k in saved:
                        os.environ.pop(k, None)
                    os.environ.update(c)
                    if bool(wh()) != bool(G._is_windows_host()):
                        agree = False
            finally:
                for k, v in saved.items():
                    if v is None:
                        os.environ.pop(k, None)
                    else:
                        os.environ[k] = v
            check("i12 윈도우 호스트 판정이 게이트 `_is_windows_host` 와 일치(env 5조합)", agree)
        except Exception as e:           # noqa: BLE001
            check("i12 게이트 모듈 대조", False, repr(e))

# ── (j) env 는 줄이기만 — 오너 상한(180s·30s)을 넘길 수 없다 ──────────────────────────
def _consts(env_over):
    env = {k: v for k, v in os.environ.items() if not k.startswith("CYS_BOOT_RESOURCE")}
    env.update(env_over)
    code = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as B; "
            "print(B.RESOURCE_RECHECK_TOTAL_S, B.RESOURCE_RECHECK_INTERVAL_S)" % BIN)
    # -B: 이 import 가 bin/ 에 캐시를 남기지 않게(팩 봉인 SEAL-1 정신 — 모듈 자신의 봉인은 import 뒤다)
    r = subprocess.run([PY, "-B", "-c", code], capture_output=True, text=True, env=env, timeout=60)
    try:
        a, b = r.stdout.split()
        return float(a), float(b)
    except ValueError:
        return None, r.stderr[-300:]

t, i = _consts({})
check("j1 기본값 TOTAL 180 · INTERVAL 30", (t, i) == (180.0, 30.0), "got=%r,%r" % (t, i))
t, i = _consts({"CYS_BOOT_RESOURCE_RECHECK_TOTAL_S": "999", "CYS_BOOT_RESOURCE_RECHECK_INTERVAL_S": "999"})
check("j2 env 로 늘리기 불가(min(env,상한))", (t, i) == (180.0, 30.0), "got=%r,%r" % (t, i))
t, i = _consts({"CYS_BOOT_RESOURCE_RECHECK_TOTAL_S": "-5", "CYS_BOOT_RESOURCE_RECHECK_INTERVAL_S": "0"})
check("j3 음수 TOTAL → 0(종전) · 0 간격 → 하한 0.05(스폰 폭주 방지)", (t, i) == (0.0, 0.05),
      "got=%r,%r" % (t, i))
t, i = _consts({"CYS_BOOT_RESOURCE_RECHECK_TOTAL_S": "nan", "CYS_BOOT_RESOURCE_RECHECK_INTERVAL_S": "x"})
check("j4 비수치 env → 기본값(부트 크래시 금지)", (t, i) == (180.0, 30.0), "got=%r,%r" % (t, i))

print("\n%d FAIL" % len(fails) if fails else "\nBOOT-RESOURCE-RECHECK-OK")
sys.exit(1 if fails else 0)
