#!/usr/bin/env python3
"""
queue_remeasure.py — WP-5 0단계: 큐 배달 대기(queue.delivered.wait_secs)·보류 사유(blocked_by) 분포 재측정.

입력(데몬별):
  * `cys events` NDJSON 캡처 파일(한 줄에 {"type":"event","name":...,"payload":{...},"timestamp":epoch})
    - queue.delivered  → payload.wait_secs / overdue / forced / merged / surface_ref
    - queue.depth_high · queue.starved → payload.blocked_by (쿨다운 5분 표본 — 틱 전수 아님)
  * (--live) `cys queue list --json --socket <sock>` — 측정 시각의 미배달 항목과 blocked_by(상시 사실 · 표본 아님)
  * (--ledger) ~/.cys/state/delivery-*.jsonl — origin=="queue" 레코드 계수 교차 확인(wait_secs 없음 · 계수만)

주의:
  * 데몬 이벤트 버스는 메모리 링(4096)뿐이다 — 디스크 이벤트 로그는 `cys events` 구독자가 만든 캡처만 존재한다.
    캡처가 없는 구간은 측정 불능(결측)이며, 이 스크립트는 결측을 0 으로 읽지 않고 `coverage_hours`/`sources` 로 명시한다.
  * 보류 사유의 정확 문자열(0.14.30 · governance.rs bc01f43)은 BLOCKED_REASONS 에 고정. 키는 '(' 앞 코드로 정규화한다.
  * 백분위는 nearest-rank(정렬 후 ceil(p*n)번째). --since 의 naive ISO 는 로컬 시간대(KST)로 읽는다.

사용:
  queue_remeasure.py --since 2026-09-06T07:03:00 \
     --daemon hub=~/.cys/pack/round/_events-s66.log --daemon hub=<impl>/events/queue-events-hub.jsonl \
     --daemon dept-1=<impl>/events/queue-events-dept-1.jsonl ... --live --ledger-dir ~/.cys/state \
     --out-json <path.json> --out-md <path.md>
  (--daemon 미지정 시 DEFAULT_SOURCES 를 쓴다)
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import subprocess
import sys
import time
from collections import Counter, defaultdict

HOME = os.path.expanduser("~")
# ★저장소 편입(v0.14.31 · CONTRACTS §G-3): 이 스크립트는 원래 감사 작업 디렉터리
#   (/Users/<개인계정>/…/audit-2026-09-06) 의 캡처 파일을 기본 소스로 잡았다 — 개인 절대경로라
#   저장소에 실을 수 없다(secret-scan 규칙 1·2). 그 경로는 이 저장소 밖(감사 산출물 폴더)에만
#   있고 배포 팩에도 없으므로, 기본값은 아무도 못 찾는 빈 디렉터리로 제네릭화하고, 실제 감사
#   산출물 위치는 환경변수(CYS_QUEUE_REMEASURE_AUDIT_DIR)로만 주입한다(값 미지정이면 DEFAULT_SOURCES
#   의 각 파일이 전부 '없음'으로 잡혀 capture_available=False — 결측을 0으로 읽지 않는 기존 설계
#   그대로 정직하게 드러난다. --daemon 으로 직접 캡처 경로를 넘기면 이 기본값과 무관하게 동작한다).
AUDIT = os.environ.get("CYS_QUEUE_REMEASURE_AUDIT_DIR", "")
EVENTS_DIR = os.path.join(AUDIT, "impl", "events") if AUDIT else os.path.join(HOME, ".cys", "state", "queue-remeasure-events")

# 0.14.30 (bc01f43) src/bin/cysd/governance.rs — mark_queue_blocked() 에 전달되는 정확 문자열.
# 코드(‘(’ 앞) → (전체 문자열, 소스 행, 뜻). 이벤트 payload.blocked_by · queue.list blocked_by 에 그대로 실린다.
BLOCKED_REASONS = {
    "queue_paused": ("queue_paused(헬스 조치)", "governance.rs:5300", "T4-17 pause-queue 헬스 조치 중"),
    "input_pending": ("input_pending(입력줄에 미제출 입력)", "governance.rs:5357", "프롬프트 경계 판정: 입력줄 점유"),
    "prompt_unknown": ("prompt_unknown(프롬프트 경계 관측 불능)", "governance.rs:5358", "프롬프트 경계 판정: 커서행 판독 불능(observe_prompt)"),
    "prompt_not_ready": ("prompt_not_ready(프롬프트 경계 미도달)", "governance.rs:5359", "프롬프트 경계 판정: 마커 미표시(출력 중)"),
    "busy": ("busy(출력 중)", "governance.rs:5373", "비마커 어댑터 quiet 규칙(queue_quiet_verdict WaitBusy)"),
    "human_typing": ("human_typing(사람 입력 직후)", "governance.rs:5393", "사람 입력 30s 이내"),
    "empty_seat": ("empty_seat(좌석에 에이전트 미연결)", "governance.rs:5415", "role 좌석에 에이전트 없음"),
}
# 참고: 운영자 강제배달(queue.deliver RPC) 거부 코드는 별 체계(ForceDeliverDenied::code · governance.rs):
FORCE_DENY_CODES = ["queue_paused", "typing_guard", "empty_seat", "output_busy", "queue_empty",
                    "not_head_requires_allow_reorder", "not_found", "delivery_failed"]

DAEMONS = ["hub", "dept-1", "dept-2", "dept-3"]
SOCKETS = {
    "hub": f"{HOME}/.local/state/cys/cys.sock",
    "dept-1": f"{HOME}/.local/state/cys-dept-dept-1/cys.sock",
    "dept-2": f"{HOME}/.local/state/cys-dept-dept-2/cys.sock",
    "dept-3": f"{HOME}/.local/state/cys-dept-dept-3/cys.sock",
}
STATE_DIRS = {
    "hub": f"{HOME}/.local/state/cys",
    "dept-1": f"{HOME}/.local/state/cys-dept-dept-1",
    "dept-2": f"{HOME}/.local/state/cys-dept-dept-2",
    "dept-3": f"{HOME}/.local/state/cys-dept-dept-3",
}
LEDGERS = {
    "hub": ["delivery-base.jsonl", "delivery-base.jsonl.1"],
    "dept-1": ["delivery-Users_cys_.local_state_cys-dept-dept-1_cys.sock.jsonl",
               "delivery-Users_cys_.local_state_cys-dept-dept-1_cys.sock.jsonl.1"],
    "dept-2": ["delivery-Users_cys_.local_state_cys-dept-dept-2_cys.sock.jsonl"],
    "dept-3": ["delivery-Users_cys_.local_state_cys-dept-dept-3_cys.sock.jsonl"],
}
DEFAULT_SOURCES = {
    # 본부: 구 세션의 `cys events` 캡처(pid 61127 · 0.14.29 구간 · 07:03 재시작에서 끊김) + 이 감사의 수집기 출력
    "hub": [f"{HOME}/.cys/pack/round/_events-s66.log",
            f"{HOME}/.cys/pack.prev/round/_events-s66.log",
            f"{EVENTS_DIR}/queue-events-hub.replay.jsonl",
            f"{EVENTS_DIR}/queue-events-hub.jsonl"],
    # dept-1: CSO 좌석의 캡처(0.14.29 구간 · 07:05 재시작에서 끊김) + 수집기 출력
    "dept-1": [f"{HOME}/.cys/pack-dept-dept-1/round/cso_events.log",
               f"{EVENTS_DIR}/queue-events-dept-1.replay.jsonl",
               f"{EVENTS_DIR}/queue-events-dept-1.jsonl"],
    "dept-2": [f"{EVENTS_DIR}/queue-events-dept-2.replay.jsonl", f"{EVENTS_DIR}/queue-events-dept-2.jsonl"],
    "dept-3": [f"{EVENTS_DIR}/queue-events-dept-3.replay.jsonl", f"{EVENTS_DIR}/queue-events-dept-3.jsonl"],
}
PROTECTED_ROOTS = (os.path.realpath(f"{HOME}/.cys"), os.path.realpath(f"{HOME}/.local/state"))
EXACT_BLOCKED_STRINGS = {v[0] for v in BLOCKED_REASONS.values()}


def assert_writable_outside_protected(path: str) -> None:
    """출력은 감사 디렉터리 등 바깥에만 — ~/.cys · ~/.local/state 아래(심링크 해소 후)는 거부한다."""
    rp = os.path.realpath(os.path.expanduser(path))
    for root in PROTECTED_ROOTS:
        if rp == root or rp.startswith(root + os.sep):
            raise SystemExit(f"refuse to write under protected root: {path} -> {rp}")


BASELINE_0_14_29 = {"p90_secs": 2113, "over_600s_pct": 16.0, "note": "IMPLEMENTATION-PLAN.md §2 표 5행·WP-5 0단계(본부 0.14.29 실측)"}


def parse_iso(s: str | None) -> float | None:
    if not s:
        return None
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    d = dt.datetime.fromisoformat(s)
    if d.tzinfo is None:
        d = d.astimezone()  # naive → 로컬(KST)
    return d.timestamp()


def iso_local(t: float | None) -> str | None:
    if t is None:
        return None
    return dt.datetime.fromtimestamp(t).astimezone().isoformat(timespec="seconds")


def nearest_rank(sorted_vals: list, p: float):
    if not sorted_vals:
        return None
    k = max(1, math.ceil(p * len(sorted_vals)))
    return sorted_vals[min(k, len(sorted_vals)) - 1]


def reason_code(reason: str | None) -> str:
    if reason is None:
        return "null"
    return reason.split("(", 1)[0]


def load_events(path: str, since: float, until: float | None):
    """캡처 파일 1개 → (이벤트 리스트[시간창 내], 파일 메타). 비-JSON 행(재연결 경고 등)은 계수만."""
    meta = {"path": path, "exists": os.path.exists(path), "lines": 0, "events_total": 0, "events_in_window": 0,
            "non_json_lines": 0, "first_event": None, "last_event": None, "mtime": None, "started_sidecar": None}
    out = []
    if not meta["exists"]:
        return out, meta
    meta["mtime"] = iso_local(os.path.getmtime(path))
    side = path + ".started"
    if os.path.exists(side):
        try:
            meta["started_sidecar"] = open(side, encoding="utf-8").read().strip()
        except OSError:
            pass
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            meta["lines"] += 1
            if not line.startswith("{"):
                meta["non_json_lines"] += 1
                continue
            try:
                e = json.loads(line)
            except json.JSONDecodeError:
                meta["non_json_lines"] += 1
                continue
            if e.get("type") != "event":
                continue
            ts = e.get("timestamp")
            if not isinstance(ts, (int, float)):
                continue
            meta["events_total"] += 1
            if meta["first_event"] is None or ts < meta["first_event"]:
                meta["first_event"] = ts
            if meta["last_event"] is None or ts > meta["last_event"]:
                meta["last_event"] = ts
            if ts < since or (until is not None and ts > until):
                continue
            meta["events_in_window"] += 1
            out.append(e)
    meta["first_event"] = iso_local(meta["first_event"])
    meta["last_event"] = iso_local(meta["last_event"])
    return out, meta


def summarize_daemon(name: str, files: list[str], since: float, until: float | None):
    events = []
    metas = []
    seen = set()
    for f in files:
        evs, meta = load_events(f, since, until)
        metas.append(meta)
        for e in evs:
            key = (e.get("seq"), e.get("name"), e.get("timestamp"))
            if key in seen:  # pack/ 와 pack.prev/ 의 동일 사본 등 중복 제거
                continue
            seen.add(key)
            events.append(e)
    events.sort(key=lambda e: e.get("timestamp", 0))
    first = events[0]["timestamp"] if events else None
    last = events[-1]["timestamp"] if events else None
    # 캡처 가용성: 파일이 하나라도 존재하고(ack/heartbeat 포함) 줄이 있으면 '캡처 있음'. 없으면 모든 계수는 결측(None).
    capture_available = any(m["exists"] and m["lines"] > 0 for m in metas)
    # 이벤트 스팬(첫~마지막 이벤트) — 캡처 단절을 구분하지 못한다(이름을 그대로 둔다: event_span_hours).
    event_span_hours = (round((last - max(since, first)) / 3600.0, 3) if events else (0.0 if capture_available else None))
    # 캡처 스팬: 소스들의 (sidecar .started 또는 첫 이벤트) ~ 마지막 mtime(heartbeat 가 15s 마다 갱신) — 근사값.
    cap_start, cap_end = None, None
    for m in metas:
        if not m["exists"]:
            continue
        st = None
        if m.get("started_sidecar"):
            try:
                st = parse_iso(m["started_sidecar"].splitlines()[0].split()[0])
            except (ValueError, IndexError):
                st = None
        if st is None and m["first_event"]:
            st = parse_iso(m["first_event"])
        if st is not None:
            cap_start = st if cap_start is None else min(cap_start, st)
        mt = os.path.getmtime(m["path"])
        cap_end = mt if cap_end is None else max(cap_end, mt)
    capture_span_hours = (round((cap_end - max(since, cap_start)) / 3600.0, 3)
                          if capture_available and cap_start is not None and cap_end is not None else None)

    names = Counter(e.get("name") for e in events)
    waits, overdue, forced, merged_multi = [], 0, 0, 0
    delivered_events = 0
    per_surface = Counter()
    for e in events:
        if e.get("name") != "queue.delivered":
            continue
        delivered_events += 1
        p = e.get("payload") or {}
        w = p.get("wait_secs")
        if isinstance(w, (int, float)):
            waits.append(float(w))
        if p.get("overdue"):
            overdue += 1
        if p.get("forced"):
            forced += 1
        if isinstance(p.get("merged"), int) and p["merged"] > 1:
            merged_multi += 1
        per_surface[p.get("surface_ref") or f"surface:{e.get('surface_id')}"] += 1
    waits.sort()
    n = len(waits)
    delivered = {
        "count": delivered_events if capture_available else None,
        "wait_secs_present": n,
        "wait_secs_missing": delivered_events - n,
        "p50": nearest_rank(waits, 0.50),
        "p90": nearest_rank(waits, 0.90),
        "p99": nearest_rank(waits, 0.99),
        "max": waits[-1] if waits else None,
        "mean": round(sum(waits) / n, 1) if n else None,
        "over_60s": sum(1 for w in waits if w > 60),
        "over_600s": sum(1 for w in waits if w > 600),
        "over_60s_pct": round(100.0 * sum(1 for w in waits if w > 60) / n, 1) if n else None,
        "over_600s_pct": round(100.0 * sum(1 for w in waits if w > 600) / n, 1) if n else None,
        "overdue": overdue,
        "forced": forced,
        "merged_multi": merged_multi,
        "top_surfaces": per_surface.most_common(5),
        "percentile_method": "nearest-rank",
    }

    blocked_samples = Counter()
    blocked_full = Counter()
    starved = []
    for e in events:
        if e.get("name") not in ("queue.depth_high", "queue.starved"):
            continue
        p = e.get("payload") or {}
        b = p.get("blocked_by")
        blocked_samples[reason_code(b)] += 1
        blocked_full[b or "null"] += 1
        if e.get("name") == "queue.starved":
            starved.append({"ts": iso_local(e.get("timestamp")), "surface_ref": p.get("surface_ref"),
                            "waited_secs": p.get("waited_secs"), "depth": p.get("depth"), "blocked_by": b})
    tot = sum(blocked_samples.values())
    blocked = {
        "source": "queue.depth_high + queue.starved payload.blocked_by (surface당 5분 쿨다운 표본 · 틱 전수 아님)",
        "samples": tot,
        "by_code": dict(blocked_samples.most_common()),
        "by_code_pct": {k: round(100.0 * v / tot, 1) for k, v in blocked_samples.items()} if tot else {},
        "prompt_unknown_share_pct": round(100.0 * blocked_samples.get("prompt_unknown", 0) / tot, 1) if tot else None,
        "exact_strings_seen": dict(blocked_full.most_common()),
        "unknown_strings": sorted(k for k in blocked_full if k != "null" and reason_code(k) not in BLOCKED_REASONS),
        "nonexact_strings": sorted(k for k in blocked_full if k != "null" and k not in EXACT_BLOCKED_STRINGS),
        "starved_events": starved[:50],
        "starved_count": len(starved),
    }
    return {
        "daemon": name,
        "sources": metas,
        "capture_available": capture_available,
        "events_in_window": len(events) if capture_available else None,
        "event_names": dict(names.most_common()),
        "first_event_in_window": iso_local(first),
        "last_event_in_window": iso_local(last),
        "event_span_hours": event_span_hours,
        "capture_span_hours": capture_span_hours,
        "delivered": delivered,
        "blocked": blocked,
    }


def live_snapshot(name: str, sock: str, now: float):
    """`cys queue list --json --socket` — 읽기 전용 RPC. 데몬이 없으면 자동기동 금지(CYS_NO_AUTOSTART=1)."""
    env = dict(os.environ, CYS_NO_AUTOSTART="1", PATH=os.environ.get("PATH", "") + f":{HOME}/.cargo/bin:/usr/local/bin")
    res = {"socket": sock, "ok": False, "measured_at": iso_local(now)}
    try:
        cp = subprocess.run(["cys", "queue", "list", "--json", "--socket", sock], env=env,
                            capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired) as ex:
        res["error"] = f"{type(ex).__name__}: {ex}"
        return res
    if cp.returncode != 0:
        res["error"] = f"rc={cp.returncode} {cp.stderr.strip()[:300]}"
        return res
    try:
        entries = json.loads(cp.stdout)
    except json.JSONDecodeError:
        res["error"] = f"non-json stdout: {cp.stdout[:200]!r}"
        return res
    if not isinstance(entries, list):
        res["error"] = "unexpected shape"
        return res
    res["ok"] = True
    res["pending_entries"] = len(entries)
    res["restored_entries"] = sum(1 for e in entries if e.get("restored"))
    by_code = Counter(reason_code(e.get("blocked_by")) for e in entries)
    res["blocked_by_code_entries"] = dict(by_code.most_common())
    tot = sum(by_code.values())
    res["prompt_unknown_share_pct"] = round(100.0 * by_code.get("prompt_unknown", 0) / tot, 1) if tot else None
    surfaces = defaultdict(lambda: {"entries": 0, "blocked_by": None, "blocked_since": None, "oldest_age_secs": 0, "role_hint": None})
    for e in entries:
        k = e.get("surface_ref") or f"surface:{e.get('surface_id')}"
        s = surfaces[k]
        s["entries"] += 1
        s["blocked_by"] = e.get("blocked_by")
        s["blocked_since"] = iso_local(e["blocked_since"]) if isinstance(e.get("blocked_since"), (int, float)) else None
        s["oldest_age_secs"] = max(s["oldest_age_secs"], int(e.get("age_secs") or 0))
    res["surfaces"] = dict(surfaces)
    ages = sorted(int(e.get("age_secs") or 0) for e in entries)
    res["pending_age_secs"] = {"max": ages[-1] if ages else None, "p50": nearest_rank(ages, 0.5),
                               "over_600s": sum(1 for a in ages if a > 600)}
    res["unknown_strings"] = sorted({e.get("blocked_by") for e in entries
                                     if e.get("blocked_by") and reason_code(e["blocked_by"]) not in BLOCKED_REASONS})
    res["nonexact_strings"] = sorted({e.get("blocked_by") for e in entries
                                      if e.get("blocked_by") and e["blocked_by"] not in EXACT_BLOCKED_STRINGS})
    return res


def ledger_counts(name: str, ledger_dir: str, since: float, until: float | None):
    """원장 origin=queue 계수 — wait_secs 는 없다(WP-5 L 이 추가 예정). 배달 계수의 교차 확인용."""
    out = {"files": [], "missing_files": [], "queue_origin_records": 0, "all_records": 0, "part_records": 0, "last_ts": None}
    last = None
    for fn in LEDGERS.get(name, []):
        p = os.path.join(ledger_dir, fn)
        if not os.path.exists(p):
            out["missing_files"].append(p)
            continue
        cnt = 0
        with open(p, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                try:
                    r = json.loads(line)
                except json.JSONDecodeError:
                    continue
                ts = r.get("ts_epoch")
                if not isinstance(ts, (int, float)):
                    continue
                if last is None or ts > last:
                    last = ts
                if ts < since or (until is not None and ts > until):
                    continue
                cnt += 1
                if "part" in r:  # 조각 행(parent sha 로 묶인 분할 제출) — 배달 1건이 아니다
                    out["part_records"] += 1
                    continue
                out["all_records"] += 1
                if r.get("origin") == "queue":
                    out["queue_origin_records"] += 1
        out["files"].append({"path": p, "records_in_window": cnt})
    out["last_ts"] = iso_local(last)
    if not out["files"]:  # 원장이 하나도 없으면 계수는 결측
        out["queue_origin_records"] = out["all_records"] = out["part_records"] = None
    return out


def fmt(v):
    return "-" if v is None else (f"{v:.0f}" if isinstance(v, float) and v == int(v) else str(v))


def render_md(report: dict) -> str:
    L = []
    L.append(f"# 큐 재측정 (WP-5 0단계) — 측정 {report['measured_at']}")
    L.append("")
    L.append(f"- 창: {report['window']['since']} → {report['window']['until'] or '측정 시각'} · 데몬 버전 {report['daemon_version']}")
    L.append(f"- 기준선(0.14.29 · 본부): p90 {BASELINE_0_14_29['p90_secs']}s · 10분 초과 {BASELINE_0_14_29['over_600s_pct']}%  ({BASELINE_0_14_29['note']})")
    L.append("- 백분위 nearest-rank · 결측은 '-'(0 아님) · 보류 사유 표본은 depth_high/starved 이벤트(5분 쿨다운) — 틱 전수 아님")
    L.append("")
    L.append("## 1. 데이터 존재량(캡처 커버리지)")
    L.append("")
    L.append("| 데몬 | 캡처 | 창 내 이벤트 | 첫 이벤트 | 마지막 이벤트 | 이벤트 스팬(h) | 캡처 스팬(h) | 소스(존재/창내 이벤트) |")
    L.append("|---|---|---|---|---|---|---|---|")
    for d in report["daemons"]:
        src = "; ".join(f"{os.path.basename(m['path'])}({'있음' if m['exists'] else '없음'}/{m['events_in_window']})" for m in d["sources"])
        L.append(f"| {d['daemon']} | {'있음' if d['capture_available'] else '**없음(결측)**'} | {fmt(d['events_in_window'])} | {d['first_event_in_window'] or '-'} | {d['last_event_in_window'] or '-'} | {fmt(d['event_span_hours'])} | {fmt(d['capture_span_hours'])} | {src} |")
    L.append("")
    L.append("## 2. queue.delivered wait_secs 분포")
    L.append("")
    L.append("| 데몬 | n(배달) | wait 결측 | p50 | p90 | p99 | max | >60s | >600s | >600s % | overdue | forced | 병합배달 |")
    L.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for d in report["daemons"]:
        v = d["delivered"]
        L.append(f"| {d['daemon']} | {fmt(v['count'])} | {v['wait_secs_missing']} | {fmt(v['p50'])} | {fmt(v['p90'])} | {fmt(v['p99'])} | {fmt(v['max'])} | {v['over_60s']} | {v['over_600s']} | {fmt(v['over_600s_pct'])} | {v['overdue']} | {v['forced']} | {v['merged_multi']} |")
    L.append("")
    L.append("### 기준선 대비")
    L.append("")
    for d in report["daemons"]:
        c = report["baseline_compare"][d["daemon"]]
        L.append(f"- {d['daemon']}: {c}")
    L.append("")
    L.append("## 3. 보류 사유(blocked_by) 분포 — 이벤트 표본")
    L.append("")
    L.append("| 데몬 | 표본 | 사유별(코드: 건) | prompt_unknown % | starved 발행 | 미지/비정확 문자열 |")
    L.append("|---|---|---|---|---|---|")
    for d in report["daemons"]:
        b = d["blocked"]
        codes = ", ".join(f"{k}: {v}" for k, v in b["by_code"].items()) or "-"
        odd = ", ".join(sorted(set(b["unknown_strings"]) | set(b["nonexact_strings"]))) or "-"
        L.append(f"| {d['daemon']} | {b['samples'] if d['capture_available'] else '-'} | {codes} | {fmt(b['prompt_unknown_share_pct'])} | {b['starved_count']} | {odd} |")
    L.append("")
    if report.get("live"):
        L.append(f"## 4. 라이브 스냅샷 `cys queue list --json` (측정 시각 {report['measured_at']} · 미배달 항목의 현재 blocked_by · 표본 아님)")
        L.append("")
        L.append("| 데몬 | 미배달 | restored | 사유별(코드: 건) | prompt_unknown % | 최고령(s) | >600s | surface별 |")
        L.append("|---|---|---|---|---|---|---|---|")
        for name, s in report["live"].items():
            if not s.get("ok"):
                L.append(f"| {name} | 오류 | | {s.get('error')} | | | | |")
                continue
            codes = ", ".join(f"{k}: {v}" for k, v in s["blocked_by_code_entries"].items()) or "-"
            surf = "; ".join(f"{k}:{v['entries']}건/{reason_code(v['blocked_by'])}/{v['oldest_age_secs']}s" for k, v in s["surfaces"].items()) or "-"
            a = s["pending_age_secs"]
            L.append(f"| {name} | {s['pending_entries']} | {s['restored_entries']} | {codes} | {fmt(s['prompt_unknown_share_pct'])} | {fmt(a['max'])} | {a['over_600s']} | {surf} |")
        L.append("")
    if report.get("ledger"):
        L.append("## 5. 원장 교차 확인(origin=queue 레코드 계수 · wait_secs 없음)")
        L.append("")
        L.append("| 데몬 | 창 내 배달(조각 제외) | origin=queue | 조각 행 | 원장 마지막 ts | 결측 파일 |")
        L.append("|---|---|---|---|---|---|")
        for name, lg in report["ledger"].items():
            L.append(f"| {name} | {fmt(lg['all_records'])} | {fmt(lg['queue_origin_records'])} | {fmt(lg['part_records'])} | {lg['last_ts'] or '-'} | {len(lg['missing_files'])} |")
        L.append("")
    L.append("## 6. 데몬이 내는 보류 사유 정확 문자열 (0.14.30 bc01f43 · mark_queue_blocked)")
    L.append("")
    L.append("| 코드 | 정확 문자열 | 소스 | 뜻 |")
    L.append("|---|---|---|---|")
    for code, (full, src, mean) in BLOCKED_REASONS.items():
        L.append(f"| `{code}` | `{full}` | {src} | {mean} |")
    L.append("")
    L.append(f"- 강제배달(queue.deliver RPC) 거부 코드(별 체계): {', '.join(FORCE_DENY_CODES)}")
    L.append("- `queue.blocked` 라는 **이벤트는 존재하지 않는다** — 사유는 surface 상태(`queue_blocked`)로만 남고 `queue.list` 가 읽으며, 이벤트로는 `queue.depth_high`/`queue.starved` 의 `blocked_by` 에만 실린다.")
    L.append("")
    if report.get("notes"):
        L.append("## 7. 비고")
        L.append("")
        for n in report["notes"]:
            L.append(f"- {n}")
        L.append("")
    return "\n".join(L)


def compare_baseline(d: dict) -> str:
    v = d["delivered"]
    if not d["capture_available"]:
        return "비교 불능 — 캡처 없음(결측)"
    if v["count"] == 0:
        return f"비교 불능 — 캡처 {fmt(d['capture_span_hours'])}h 동안 queue.delivered 0건(실측 0 · 배달 전무)"
    if v["p90"] is None:
        return "비교 불능 — 배달은 있으나 wait_secs 결측"
    parts = []
    p90 = v["p90"]
    parts.append(f"p90 {fmt(p90)}s vs 기준 {BASELINE_0_14_29['p90_secs']}s ({'개선' if p90 < BASELINE_0_14_29['p90_secs'] else '악화/동일'})")
    pct = v["over_600s_pct"]
    parts.append(f">600s {fmt(pct)}% vs 기준 {BASELINE_0_14_29['over_600s_pct']}% ({'개선' if pct < BASELINE_0_14_29['over_600s_pct'] else '악화/동일'})")
    parts.append(f"수용기준(p90<60s·>600s 0건): {'충족' if (p90 < 60 and v['over_600s'] == 0) else '미충족'}")
    parts.append(f"n={v['count']} · 이벤트 스팬 {fmt(d['event_span_hours'])}h · 캡처 스팬 {fmt(d['capture_span_hours'])}h")
    return " · ".join(parts)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--since", required=True, help="ISO(naive=로컬 KST · 'Z'/오프셋 허용)")
    ap.add_argument("--until", default=None)
    ap.add_argument("--daemon", action="append", default=[], metavar="NAME=PATH",
                    help="캡처 파일(반복 가능 · 같은 NAME 여러 파일은 seq 로 dedup). 미지정 시 DEFAULT_SOURCES")
    ap.add_argument("--live", action="store_true", help="cys queue list --json 스냅샷(읽기 전용 RPC)")
    ap.add_argument("--ledger-dir", default=None, help="예: ~/.cys/state — origin=queue 계수 교차 확인")
    ap.add_argument("--daemon-version", default=None, help="보고서 표기용(예: 0.14.30)")
    ap.add_argument("--out-json", required=True)
    ap.add_argument("--out-md", required=True)
    ap.add_argument("--note", action="append", default=[])
    a = ap.parse_args()

    assert_writable_outside_protected(a.out_json)
    assert_writable_outside_protected(a.out_md)
    since = parse_iso(a.since)
    until = parse_iso(a.until)
    now = time.time()
    sources: dict[str, list[str]] = defaultdict(list)
    if a.daemon:
        for spec in a.daemon:
            if "=" not in spec:
                ap.error(f"--daemon 형식 NAME=PATH: {spec}")
            k, p = spec.split("=", 1)
            sources[k].append(os.path.expanduser(p))
    else:
        for k, v in DEFAULT_SOURCES.items():
            sources[k] = list(v)
    order = [d for d in DAEMONS if d in sources] + [d for d in sources if d not in DAEMONS]

    version = a.daemon_version
    if not version:
        try:
            version = subprocess.run(["cys", "--version"], capture_output=True, text=True, timeout=10,
                                     env=dict(os.environ, PATH=os.environ.get("PATH", "") + ":/usr/local/bin")).stdout.strip()
        except (OSError, subprocess.TimeoutExpired):
            version = "unknown"

    daemons = [summarize_daemon(d, sources[d], since, until) for d in order]
    report = {
        "tool": "queue_remeasure.py",
        "measured_at": iso_local(now),
        "measured_at_epoch": now,
        "window": {"since": iso_local(since), "until": iso_local(until), "since_epoch": since, "until_epoch": until},
        "daemon_version": version,
        "baseline_0_14_29": BASELINE_0_14_29,
        "blocked_reasons_exact": {k: {"string": v[0], "source": v[1], "meaning": v[2]} for k, v in BLOCKED_REASONS.items()},
        "force_deliver_deny_codes": FORCE_DENY_CODES,
        "daemons": daemons,
        "baseline_compare": {d["daemon"]: compare_baseline(d) for d in daemons},
        "notes": list(a.note),
    }
    if a.live:
        report["live"] = {d: live_snapshot(d, SOCKETS[d], now) for d in order if d in SOCKETS}
    if a.ledger_dir:
        report["ledger"] = {d: ledger_counts(d, os.path.expanduser(a.ledger_dir), since, until) for d in order if d in LEDGERS}

    os.makedirs(os.path.dirname(os.path.abspath(a.out_json)) or ".", exist_ok=True)
    with open(a.out_json, "w", encoding="utf-8") as fh:
        json.dump(report, fh, ensure_ascii=False, indent=1, default=str)
    md = render_md(report)
    with open(a.out_md, "w", encoding="utf-8") as fh:
        fh.write(md)
    # write-then-verify
    with open(a.out_json, encoding="utf-8") as fh:
        back = json.load(fh)
    if back.get("measured_at") != report["measured_at"] or len(back.get("daemons", [])) != len(daemons):
        print("VERIFY FAIL: json readback mismatch", file=sys.stderr)
        return 2
    if open(a.out_md, encoding="utf-8").read() != md:
        print("VERIFY FAIL: md readback mismatch", file=sys.stderr)
        return 2
    for d in daemons:
        v = d["delivered"]
        print(f"{d['daemon']}: capture={'Y' if d['capture_available'] else 'N'} span={fmt(d['event_span_hours'])}h/{fmt(d['capture_span_hours'])}h delivered n={fmt(v['count'])} p50={fmt(v['p50'])} p90={fmt(v['p90'])} "
              f"p99={fmt(v['p99'])} max={fmt(v['max'])} >60s={v['over_60s']} >600s={v['over_600s']}({fmt(v['over_600s_pct'])}%) "
              f"blocked_samples={d['blocked']['samples']} {d['blocked']['by_code']}")
    print(f"wrote {a.out_json} ({os.path.getsize(a.out_json)}B) {a.out_md} ({os.path.getsize(a.out_md)}B) measured_at={report['measured_at']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
