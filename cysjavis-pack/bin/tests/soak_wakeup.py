#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""soak_wakeup.py — M1 라이브 소킹 계측기 (T-0147-2 §1-A M1)

설계 §1-A가 요구하는 완료 기준은 **"정상 대기 2시간, master push 0"**이고, 계수점은
"target=master 인 wake 발행 + 실제 Inject"다. 이 스크립트는 그 계수를 **라이브 상태를 전혀
건드리지 않고** 남긴다.

    격리 state dir 에서 레포 게이트를 주기적으로 실행
      → 데몬은 **읽기 전용**으로만 접촉(`cys status --json` 수집)
      → 배달 계열 명령(`cys send` 등)은 전부 **shim 이 가로채 기록만** 한다
      → 게이트 대장·wakeup 큐 원장·shim 로그 3중으로 master push 를 계수

왜 shim 인가: 게이트는 판정 후 같은 실행에서 `javis_wakeup drain --deliver` 까지 수행하고,
그 배달은 `cys send --queued --to master` 다. 소킹이 진짜 master stdin 을 두드리면 그것은
측정이 아니라 **오염**이다(관측이 대상을 바꾼다). 그래서 읽기 명령만 실물로 통과시키고
쓰기 명령은 기록으로 대체한다 — 계수 대상은 "게이트가 보내려 했는가"이므로 손실이 없다.

    python3 soak_wakeup.py --cycles 24 --interval 300      # 2시간 실측(권장)
    python3 soak_wakeup.py --cycles 24 --interval 0        # 가속 스모크(수 초)
    python3 soak_wakeup.py --json                          # 기계 판독

exit 0 = master push 0(M1 충족) · 1 = push 관측(사유는 보고에) · 2 = 실행 불가.
라이브 무접촉 계약: `~/.cys/state`·`~/.cys/pack` 에 **쓰지 않는다**. 읽기도 데몬 소켓 조회뿐이다.
"""
import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.dirname(TESTS_DIR)
PACK_DIR = os.path.dirname(BIN_DIR)
GATE = os.path.join(BIN_DIR, "javis_report_gate.py")
PY = sys.executable or "python3"

# shim 이 **실물로 통과시키는** 서브커맨드 — 전부 읽기 전용이다. 이 목록에 없는 것은
# 기록 후 exit 0(성공 가장). 새 커맨드를 통과시킬 때는 "데몬 상태를 바꾸는가"를 먼저 물어라.
READ_ONLY_PASSTHROUGH = ("status", "list", "identify", "gate-check", "read-screen")

SHIM = """#!/bin/sh
# soak_wakeup.py 가 심은 cys shim — 읽기 명령만 실물로, 쓰기 명령은 기록만.
REAL="%(real)s"
LOG="%(log)s"
case "$1" in
%(cases)s)
    [ -n "$REAL" ] || { echo "no real cys" >&2; exit 127; }
    exec "$REAL" "$@" ;;
esac
printf '%%s\\n' "$(date +%%s)|$*" >> "$LOG"
exit 0
"""


def _write(path, body, mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def _read_jsonl(path):
    out = []
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        out.append(json.loads(line))
                    except ValueError:
                        pass
    except OSError:
        pass
    return out


def build_env(root, real_cys):
    """격리 실행 env — 게이트가 라이브 경로를 절대 해석하지 못하게 전부 고정한다."""
    gate_dir = os.path.join(root, "gate")
    bindir = os.path.join(root, "bin")
    log = os.path.join(root, "cys_calls.log")
    cases = "  " + " | ".join(READ_ONLY_PASSTHROUGH)
    _write(os.path.join(bindir, "cys"),
           SHIM % {"real": real_cys or "", "log": log, "cases": cases})
    env = dict(os.environ)
    env["CYS_REPORT_GATE_DIR"] = gate_dir          # 레인 격리(B7) — 라이브 대장 무접촉
    env["CYS_PACK_DIR"] = PACK_DIR                 # 레포 팩(라이브 ~/.cys/pack 아님)
    env["CYS_BIN"] = os.path.join(bindir, "cys")   # 게이트의 절대 해석 경로도 shim 으로
    env["PATH"] = bindir + os.pathsep + env.get("PATH", "")
    env["JAVIS_TASK_ROOT"] = os.path.join(root, "tasks")   # 티켓 원장도 격리(P3 술어 입력)
    return env, gate_dir, log


def count_master_pushes(gate_dir, call_log):
    """3중 계수 — 게이트 대장(pushes) · wakeup 큐 원장(queued/coalesced) · shim 호출 로그."""
    ledger = _read_jsonl(os.path.join(gate_dir, "ledger.jsonl"))
    from_gate = [p for e in ledger for p in (e.get("pushes") or [])
                 if p.get("target") == "master"]
    queue = _read_jsonl(os.path.join(gate_dir, "_round", "wakeups", "queue.jsonl"))
    from_queue = [e for e in queue
                  if e.get("target") == "master" and e.get("event") in ("queued", "coalesced")]
    sends = []
    try:
        with open(call_log, encoding="utf-8") as f:
            for line in f:
                if "send" in line and "--to master" in line:
                    sends.append(line.strip())
    except OSError:
        pass
    return {"gate_ledger": from_gate, "queue_ledger": from_queue, "cys_send": sends,
            "verdicts": [e.get("verdict") for e in ledger],
            "cycles_recorded": len(ledger)}


def badges(gate_dir):
    try:
        with open(os.path.join(gate_dir, "badges.json"), encoding="utf-8") as f:
            return [b["key"] for b in json.load(f).get("badges") or []]
    except (OSError, ValueError, KeyError):
        return []


def main(argv=None):
    ap = argparse.ArgumentParser(description="M1 라이브 소킹 — master push 계수(라이브 무접촉)")
    ap.add_argument("--cycles", type=int, default=24, help="실행 주기 수(기본 24 = 2시간)")
    ap.add_argument("--interval", type=float, default=300.0,
                    help="주기 간격 초(기본 300). 0 = 대기 없음(가속 스모크)")
    ap.add_argument("--state-dir", default=None, help="격리 루트(기본 mkdtemp)")
    ap.add_argument("--keep", action="store_true", help="종료 후 격리 루트 보존(사후 조사)")
    ap.add_argument("--json", action="store_true", help="기계 판독 결과")
    ap.add_argument("--quiet", action="store_true", help="주기별 진행 출력 억제")
    a = ap.parse_args(argv)

    if not os.path.isfile(GATE):
        print("게이트 스크립트 부재: %s" % GATE, file=sys.stderr)
        return 2
    root = a.state_dir or tempfile.mkdtemp(prefix="cys-soak-wakeup-")
    os.makedirs(root, exist_ok=True)
    # ★라이브 방어: 격리 루트가 라이브 state/pack 안이면 즉시 거부한다(오타 한 번이 사고다).
    live = [os.path.join(os.path.expanduser("~"), ".cys", "state"),
            os.path.join(os.path.expanduser("~"), ".cys", "pack")]
    rp = os.path.realpath(root)
    if any(rp == os.path.realpath(p) or rp.startswith(os.path.realpath(p) + os.sep)
           for p in live):
        print("거부: 격리 루트가 라이브 경로 안이다 — %s" % root, file=sys.stderr)
        return 2

    real_cys = shutil.which("cys")
    env, gate_dir, call_log = build_env(root, real_cys)
    started = time.time()
    rows = []
    for i in range(a.cycles):
        t0 = time.time()
        try:
            r = subprocess.run([PY, GATE, "run"], capture_output=True, text=True,
                               timeout=120, env=env)
            summary = (r.stdout or "").strip().splitlines()[-1:] or [""]
            rows.append({"cycle": i + 1, "exit": r.returncode, "summary": summary[0],
                         "secs": round(time.time() - t0, 2)})
        except (subprocess.SubprocessError, OSError) as e:
            rows.append({"cycle": i + 1, "exit": None, "summary": "실행 실패: %s" % e,
                         "secs": round(time.time() - t0, 2)})
        if not a.quiet and not a.json:
            print("  [%2d/%d] %s" % (i + 1, a.cycles, rows[-1]["summary"]))
        if a.interval > 0 and i < a.cycles - 1:
            time.sleep(a.interval)

    counts = count_master_pushes(gate_dir, call_log)
    pushes = max(len(counts["gate_ledger"]), len(counts["queue_ledger"]),
                 len(counts["cys_send"]))
    result = {
        "verdict": "PASS(M1 push 0)" if pushes == 0 else "OBSERVED(push %d)" % pushes,
        "cycles": a.cycles, "interval_secs": a.interval,
        "elapsed_secs": round(time.time() - started, 1),
        "root": root, "gate_dir": gate_dir, "real_cys": real_cys,
        "master_pushes": pushes,
        "counts": {k: len(v) for k, v in counts.items() if isinstance(v, list)},
        "verdicts": counts["verdicts"], "badges": badges(gate_dir),
        "push_detail": counts["gate_ledger"][:20],
        "cys_write_calls": counts["cys_send"][:20],
        "rows": rows,
    }
    if a.json:
        print(json.dumps(result, ensure_ascii=False, indent=1))
    else:
        print("\n== soak_wakeup 결과 ==")
        print("  판정      : %s" % result["verdict"])
        print("  주기      : %d회 / 간격 %.0fs / 경과 %.0fs"
              % (a.cycles, a.interval, result["elapsed_secs"]))
        print("  master push: %d  (대장 %d · 큐원장 %d · cys send %d)"
              % (pushes, len(counts["gate_ledger"]), len(counts["queue_ledger"]),
                 len(counts["cys_send"])))
        print("  verdict 분포: %s" % json.dumps(
            {v: result["verdicts"].count(v) for v in sorted(set(result["verdicts"]))},
            ensure_ascii=False))
        print("  최종 배지 : %s" % (", ".join(result["badges"]) or "-"))
        print("  격리 루트 : %s%s" % (root, "" if a.keep else " (정리됨)"))
        if pushes:
            print("\n  ↓ 관측된 push(계수 대상)")
            for p in counts["gate_ledger"][:20]:
                print("    - %s" % json.dumps(p, ensure_ascii=False))
    if not a.keep and not a.state_dir:
        shutil.rmtree(root, ignore_errors=True)
    return 0 if pushes == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
