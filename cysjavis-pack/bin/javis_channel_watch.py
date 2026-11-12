#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_channel_watch — silence-first 콘텐츠 채널 watch (AGENTREACH OPP-06).

조용히 죽은 콘텐츠 채널(silently dead content channel)을 능동 통보하는 결정론 헬스
watch. javis_channels.py(OPP-02 헬스 doctor)를 입력으로, 직전 스냅샷과 diff해 상태가
나빠진/회복된 채널만 silence-first 텍스트로 골라 cys 소켓에 push한다.

★중복 회피(보고서 OPP-06 self-검출):
  - javis_report.py 와 별개 층위 — report 는 부트 노드 건강(master/worker/CSO 생존·
    todo %·context)을, 본 watch 는 콘텐츠 채널 건강(YouTube/Reddit/X 공개 루트 도달성)을
    본다. 측정 로직 신규 0 — javis_channels.py(엔진) 재사용, watch 는 diff·silence-first·
    push 만 담당(producer≠evaluator: 측정=channels, 분류·diff=watch).
  - coverage_battery 함정은 javis_channels 가 이미 봉인(battery 부재 시 UNKNOWN graceful) —
    watch 는 그 verdict 를 소비만 하므로 트랩 재노출 없음.

스냅샷: ~/.cys/state/channel_health.json (CYS_CHANNEL_HEALTH_PATH override). atomic write.
2-strike: 단발 네트워크 깜빡임 오알림 방지 — 연속 2회 비-STRONG_OK 여야 DEGRADED push.

서브커맨드:
    javis_channel_watch.py --self-test          # 네트워크0 — diff/2-strike/silence-first 박제
    javis_channel_watch.py [--json] [--no-push] [--channels reddit,x]
종료: 0 = self-test ok 또는 watch 정상, 1 = self-test 실패.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

STRONG = "STRONG_OK"
EVICT_AFTER_FAILS = 2  # learning.py:32 동형 — 2-strike.


def pack_dir():
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def snapshot_path():
    p = os.environ.get("CYS_CHANNEL_HEALTH_PATH", "")
    if p:
        return p
    return os.path.join(os.path.expanduser("~"), ".cys", "state", "channel_health.json")


def load_snapshot(path):
    try:
        d = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        return {"schema": 1, "channels": {}}
    if d.get("schema") != 1:
        return {"schema": 1, "channels": {}}
    return d


def save_snapshot(path, snap):
    """atomic write(learning.py:110-119 동형). best-effort — 실패해도 watch 안 죽음."""
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(snap, f, ensure_ascii=False, indent=2, sort_keys=True)
        os.replace(tmp, path)
        return True
    except OSError:
        return False


def measure(channels):
    """javis_channels.py --json 호출 → {platform: verdict}. 측정 재구현 0(엔진 재사용)."""
    tool = os.path.join(pack_dir(), "bin", "javis_channels.py")
    if not os.path.isfile(tool):
        return {}, "channels-tool-absent"
    argv = [sys.executable, tool, "--json"] + list(channels)
    try:
        r = subprocess.run(argv, capture_output=True, timeout=200)
        d = json.loads((r.stdout or b"{}").decode("utf-8", "replace"))
    except (OSError, ValueError, subprocess.SubprocessError):
        return {}, "measure-failed"
    return {c["platform"]: c["verdict"] for c in d.get("channels", [])}, d.get("mode", "?")


def apply_strike(prev_channels, cur_verdicts):
    """2-strike 상태기 — 연속 비-STRONG 카운트 누적. 반환 = 갱신된 채널 스냅샷 dict."""
    out = {}
    for plat, verdict in cur_verdicts.items():
        prev = prev_channels.get(plat, {})
        strikes = int(prev.get("strikes", 0))
        if verdict == STRONG:
            strikes = 0
        else:
            strikes += 1
        # 확정 status: 2-strike 도달 전엔 직전 확정 status 유지(깜빡임 흡수).
        confirmed = verdict if (verdict == STRONG or strikes >= EVICT_AFTER_FAILS) \
            else prev.get("confirmed", STRONG)
        out[plat] = {"verdict": verdict, "strikes": strikes, "confirmed": confirmed}
    return out


def build_watch(prev_channels, cur_channels):
    """transitions: 직전 confirmed vs 현 confirmed 비교 → 4종 이벤트."""
    transitions = []
    for plat in sorted(cur_channels):
        prev_c = prev_channels.get(plat, {}).get("confirmed", STRONG)
        cur_c = cur_channels[plat]["confirmed"]
        if prev_c == STRONG and cur_c == STRONG:
            ev = "STABLE_OK"
        elif prev_c == STRONG and cur_c != STRONG:
            ev = "DEGRADED"
        elif prev_c != STRONG and cur_c == STRONG:
            ev = "RECOVERED"
        else:
            ev = "STILL_DOWN"
        transitions.append({"platform": plat, "event": ev,
                            "prev": prev_c, "cur": cur_c})
    return {"transitions": transitions}


def render_text(report):
    """silence-first(javis_report.py:163 톤 계승). DEGRADED/RECOVERED 0건이면 1줄."""
    tr = report["transitions"]
    changed = [t for t in tr if t["event"] in ("DEGRADED", "RECOVERED")]
    still = [t["platform"] for t in tr if t["event"] == "STILL_DOWN"]
    if not changed:
        tail = " (여전히 off: %s)" % ",".join(still) if still else " · 변화 없음"
        return "채널 헬스 정상 (%d/%d reachable%s)" % (
            sum(1 for t in tr if t["cur"] == STRONG), len(tr), tail)
    lines = ["오너에게 보고 — 콘텐츠 채널 상태 변화 감지:"]
    for t in changed:
        mark = "⚠" if t["event"] == "DEGRADED" else "✅"
        lines.append("  %s %s: %s (%s→%s)" % (mark, t["platform"], t["event"],
                                              t["prev"], t["cur"]))
    if still:
        lines.append("  (여전히 off: %s)" % ",".join(still))
    return "\n".join(lines)


def push_master(text):
    """DEGRADED/RECOVERED 시만 master 능동 push. cys 부재 시 생략(graceful)."""
    cys = shutil.which("cys")
    if not cys:
        return False
    try:
        subprocess.run([cys, "send", "--queued", "--to", "master",
                        "[채널watch] " + text], timeout=15)
        return True
    except (OSError, subprocess.SubprocessError):
        return False


def self_test():
    """네트워크0 — diff/2-strike/silence-first 순수함수 박제(LOCKED 기대값)."""
    fails = []
    # 1) 2-strike: 1회 FAIL 은 confirmed 유지(깜빡임 흡수), 2회 연속이면 confirmed 전환.
    s1 = apply_strike({}, {"reddit": "CHANNEL_DOWN"})
    if s1["reddit"]["confirmed"] != STRONG:
        fails.append("2-strike: 1st fail must keep confirmed STRONG (flap absorb)")
    s2 = apply_strike(s1, {"reddit": "CHANNEL_DOWN"})
    if s2["reddit"]["confirmed"] != "CHANNEL_DOWN":
        fails.append("2-strike: 2nd consecutive fail must confirm DOWN")
    # 2) 회복: STRONG 1회면 즉시 strikes=0·confirmed STRONG.
    s3 = apply_strike(s2, {"reddit": "STRONG_OK"})
    if s3["reddit"]["strikes"] != 0 or s3["reddit"]["confirmed"] != STRONG:
        fails.append("recovery: 1 STRONG must reset strikes/confirmed")
    # 3) transitions: STRONG→DOWN(confirmed) = DEGRADED.
    rep = build_watch({"reddit": {"confirmed": STRONG}}, {"reddit": {"confirmed": "CHANNEL_DOWN"}})
    if rep["transitions"][0]["event"] != "DEGRADED":
        fails.append("transition STRONG→DOWN must be DEGRADED")
    rep2 = build_watch({"reddit": {"confirmed": "CHANNEL_DOWN"}}, {"reddit": {"confirmed": STRONG}})
    if rep2["transitions"][0]["event"] != "RECOVERED":
        fails.append("transition DOWN→STRONG must be RECOVERED")
    # 4) silence-first: 변화 없으면 1줄, DEGRADED 있으면 상세.
    quiet = render_text(build_watch({"x": {"confirmed": STRONG}}, {"x": {"confirmed": STRONG}}))
    if "정상" not in quiet or "\n" in quiet:
        fails.append("silence-first: stable must be single line")
    loud = render_text(build_watch({"x": {"confirmed": STRONG}}, {"x": {"confirmed": "CHANNEL_DOWN"}}))
    if "DEGRADED" not in loud:
        fails.append("silence-first: change must detail DEGRADED")
    # 5) STILL_DOWN 은 재push 안 함(changed 0건이면 1줄 — 소음 차단).
    still = build_watch({"x": {"confirmed": "CHANNEL_DOWN"}}, {"x": {"confirmed": "CHANNEL_DOWN"}})
    if [t for t in still["transitions"] if t["event"] in ("DEGRADED", "RECOVERED")]:
        fails.append("STILL_DOWN must not re-trigger push")
    # 6) atomic snapshot round-trip.
    d = tempfile.mkdtemp(prefix="chwatch_")
    try:
        sp = os.path.join(d, "state", "ch.json")
        snap = {"schema": 1, "channels": s2}
        save_snapshot(sp, snap)
        back = load_snapshot(sp)
        if back.get("channels", {}).get("reddit", {}).get("confirmed") != "CHANNEL_DOWN":
            fails.append("snapshot round-trip must preserve confirmed")
    finally:
        shutil.rmtree(d, ignore_errors=True)
    print(json.dumps({"self_test": "ok" if not fails else "fail", "failures": fails},
                     ensure_ascii=False))
    return 1 if fails else 0


def main(argv):
    ap = argparse.ArgumentParser(description="silence-first 콘텐츠 채널 watch(OPP-06)")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--json", action="store_true", help="기계판 — full WatchReport+스냅샷")
    ap.add_argument("--no-push", action="store_true", help="push 생략(stdout만)")
    ap.add_argument("--channels", default="", help="콤마구분 부분집합(예: reddit,x)")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    channels = [c.strip() for c in args.channels.split(",") if c.strip()] \
        or os.environ.get("CYS_CHANNEL_WATCH_CHANNELS", "").split(",")
    channels = [c for c in channels if c]

    sp = snapshot_path()
    prev = load_snapshot(sp)
    cur_verdicts, mode = measure(channels or [])
    cur_channels = apply_strike(prev.get("channels", {}), cur_verdicts)
    report = build_watch(prev.get("channels", {}), cur_channels)
    text = render_text(report)

    new_snap = {"schema": 1,
                "generated_at": datetime.now(timezone.utc).isoformat(),
                "mode": mode, "channels": cur_channels}
    save_snapshot(sp, new_snap)

    changed = [t for t in report["transitions"] if t["event"] in ("DEGRADED", "RECOVERED")]
    if args.json:
        print(json.dumps({"mode": mode, "report": report, "snapshot": new_snap},
                         ensure_ascii=False, indent=2))
    else:
        print(text)
        if changed and not args.no_push:
            push_master(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
