#!/usr/bin/env python3
"""javis_replay.py — 이벤트 원장 리플레이 타임라인 (읽기 전용)

원장: $JAVIS_ROOT/_round/events/events-YYYYMMDD.jsonl (+ .1 로테이트본)
javis_event.py `emit --log`가 남긴 JSONL({"ts","wire"})을 시간순으로 읽어
사람 가독 1줄/이벤트(`HH:MM:SS <type> <핵심 payload 요약 ≤80자>`)로 출력한다.
어떤 파일도 쓰지 않는다(분석 전용).

exit codes: 0 ok · 2 usage
"""
import argparse
import datetime
import json
import os
import sys
sys.dont_write_bytecode = True  # SEAL-1 층4: 호출자 env 와 무관하게 형제 import 의 __pycache__ 기록 차단(D-pyc 2026-09-21)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import javis_event  # 동일 bin — parse_wire(타입·스키마 검증 포함) 재사용

EXIT_OK, EXIT_USAGE = 0, 2


def _ledger_root():
    # JAVIS_ROOT env 또는 CWD (javis_task.py·javis_event.py와 동일 규약)
    return os.environ.get("JAVIS_ROOT") or os.getcwd()


def _ledger_paths(date):
    """.1(로테이트=더 오래됨)을 먼저, 그다음 본 파일 — 시간순 토대."""
    d = os.path.join(_ledger_root(), "_round", "events")
    base = os.path.join(d, "events-%s.jsonl" % date)
    return [base + ".1", base]


def _fmt_time(ts):
    try:
        return datetime.datetime.fromisoformat(ts).strftime("%H:%M:%S")
    except (ValueError, TypeError):
        return "??:??:??"


def _summarize(payload):
    s = json.dumps(payload, ensure_ascii=False)
    return s if len(s) <= 80 else s[:79] + "…"


def cmd_timeline(a):
    date = a.date or datetime.datetime.now().strftime("%Y%m%d")
    paths = [p for p in _ledger_paths(date) if os.path.exists(p)]
    if not paths:
        print("원장 없음")
        return EXIT_OK

    rows = []       # (ts, type, payload)
    skipped = 0
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)                      # 손상 줄 = JSON 파싱 실패
                    evt_type, payload = javis_event.parse_wire(rec["wire"])
                except (ValueError, KeyError, TypeError):
                    skipped += 1
                    continue
                rows.append((rec.get("ts", ""), evt_type, payload))

    rows.sort(key=lambda r: r[0])  # 시간순(안정 정렬 — 동초 이벤트는 원장 삽입순 유지)

    if a.grep:
        rows = [r for r in rows
                if a.grep in "%s %s" % (r[1], json.dumps(r[2], ensure_ascii=False))]

    if a.limit is not None and a.limit >= 0:
        rows = rows[-a.limit:] if a.limit else []  # 최근 N건(시간순 유지)

    for ts, evt_type, payload in rows:
        print("%s %s %s" % (_fmt_time(ts), evt_type, _summarize(payload)))

    if skipped:
        print("skipped %d" % skipped)
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="이벤트 원장 리플레이(읽기 전용)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("timeline")
    c.add_argument("--date", help="YYYYMMDD (기본=오늘)")
    c.add_argument("--grep", help="부분 문자열 필터(type+payload 대상)")
    c.add_argument("--limit", type=int, help="최근 N건만 출력")
    c.set_defaults(fn=cmd_timeline)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
