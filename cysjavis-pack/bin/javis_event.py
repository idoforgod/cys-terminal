#!/usr/bin/env python3
"""javis_event.py — P0-4 이벤트 타입 enum 계약 검증기/방출기 (EVENT_CONTRACT v2 구현)

계약 SOT: _round/EVENT_CONTRACT.md
- 미지 타입은 deny-by-default로 거부. 필수 payload 키 누락 거부. 추가 키 허용(전방 호환).
- wire: `[EVT v2] <type> <json>` 한 줄 (파서는 v1 라인도 수용 — 전환기 하위호환).
- speak: 이벤트 → 한국어 한 문장(TTS 대본 토대 — 배달·억제는 음성 트랙 몫).

exit codes: 0 ok · 2 usage · 6 invalid(타입/스키마 위반)
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # safe_path(-P) guard for sibling imports (same idiom as javis_replay.py)
import javis_scrub  # ★G2: 기록·전파 직전 비밀 마스킹(같은 폴더 형제 모듈 — 부재 시 즉시 실패=fail-closed)

EXIT_OK, EXIT_USAGE, EXIT_INVALID = 0, 2, 6

WIRE_PREFIX = "[EVT v2]"
WIRE_RE = re.compile(r"^\[EVT v[12]\]\s+(?P<type>[a-z_.]+)\s+(?P<json>\{.*\})\s*$")

# type → 필수 payload 키 (EVENT_CONTRACT.md 표와 1:1)
SCHEMA = {
    "run.queued": ["agent", "task"],
    "run.started": ["agent", "task"],
    "run.succeeded": ["agent", "task", "summary"],
    "run.failed": ["agent", "task", "summary"],
    "agent.error": ["agent", "summary"],
    "agent.silent": ["agent", "silent_minutes", "level"],
    "approval.needed": ["agent", "task", "summary"],
    "resource.soft": ["metric", "value", "threshold"],
    "resource.hard": ["metric", "value", "threshold"],
    "task.blocked": ["task", "blocked_by"],
    "task.unblocked": ["task"],
    "briefing": ["counts"],
    "task_progress": ["task", "stage"],  # v2(ViMax OPP-14): 작업 내부 스테이지 — pct·detail·cost_usd_cum 선택
}

SPEAK = {
    "run.queued": "{agent}의 {task} 작업이 대기열에 들어갔습니다.",
    "run.started": "{agent}가 {task} 작업을 시작했습니다.",
    "run.succeeded": "{agent}의 {task} 작업이 완료됐습니다. {summary}",
    "run.failed": "{agent}의 {task} 작업이 실패했습니다. {summary}",
    "agent.error": "{agent} 노드에 오류가 발생했습니다. {summary}",
    "agent.silent": "{agent}가 {silent_minutes}분째 응답이 없습니다({level}).",
    "approval.needed": "승인이 필요합니다. {agent}의 {task}: {summary}",
    "resource.soft": "자원 경고: {metric}가 {value}로 임계 {threshold}에 도달했습니다.",
    "resource.hard": "자원 초과로 착수를 차단했습니다: {metric} {value} (임계 {threshold})",
    "task.blocked": "{task}가 대기 중입니다. 선행 작업: {blocked_by}",
    "task.unblocked": "{task}의 선행 작업이 모두 끝나 재개 가능합니다.",
    "briefing": ("가동 {running}건, 처리할 일 {inbox}건, "
                 "승인대기 {approvals}건, 경보 {alerts}건입니다."),
    "task_progress": "{task} 작업이 {stage} 단계입니다.",
}


def validate(evt_type, payload):
    """(ok:bool, error:str|None) — deny-by-default."""
    if evt_type not in SCHEMA:
        return False, f"unknown event type: {evt_type} (deny-by-default)"
    if not isinstance(payload, dict):
        return False, "payload must be a JSON object"
    missing = [k for k in SCHEMA[evt_type] if k not in payload]
    if missing:
        return False, f"missing required keys for {evt_type}: {missing}"
    if evt_type == "agent.silent" and payload.get("level") not in ("suspicious", "critical"):
        return False, "agent.silent.level must be suspicious|critical"
    if evt_type == "briefing":
        counts = payload.get("counts")
        need = ["running", "inbox", "approvals", "alerts"]
        if not isinstance(counts, dict) or any(k not in counts for k in need):
            return False, f"briefing.counts must contain {need}"
    return True, None


def to_wire(evt_type, payload):
    # ★G2(cokacdir 성찰 2026-07-04): 와이어 기록 직전 비밀 마스킹 — 값 단위 재귀
    #   (직렬화 '후' 마스킹은 JSON 구조 훼손 위험이라 scrub_obj로 str 값만 교체).
    payload = javis_scrub.scrub_obj(payload)
    return f"{WIRE_PREFIX} {evt_type} {json.dumps(payload, ensure_ascii=False)}"


def parse_wire(line):
    """(evt_type, payload) 또는 ValueError."""
    m = WIRE_RE.match(line.strip())
    if not m:
        raise ValueError("not an EVT v1/v2 line")
    evt_type = m.group("type")
    try:
        payload = json.loads(m.group("json"))
    except json.JSONDecodeError as e:
        raise ValueError(f"bad payload json: {e}") from e
    ok, err = validate(evt_type, payload)
    if not ok:
        raise ValueError(err)
    return evt_type, payload


def speak(evt_type, payload):
    tpl = SPEAK[evt_type]
    if evt_type == "briefing":
        return tpl.format(**payload["counts"])
    safe = {k: (json.dumps(v, ensure_ascii=False) if isinstance(v, (list, dict)) else v)
            for k, v in payload.items()}
    class _D(dict):
        def __missing__(self, k):
            return f"<{k}?>"
    return tpl.format_map(_D(safe))


def _parse_fields(fields):
    payload = {}
    for f in fields or []:
        if "=" not in f:
            raise ValueError(f"--field must be key=value: {f}")
        k, v = f.split("=", 1)
        try:
            payload[k] = json.loads(v)  # 숫자/객체/배열 자동 인식
        except json.JSONDecodeError:
            payload[k] = v
    return payload


def cmd_emit(a):
    try:
        payload = json.loads(a.payload) if a.payload else _parse_fields(a.field)
    except ValueError as e:
        print(f"invalid: {e}", file=sys.stderr)
        return EXIT_INVALID
    ok, err = validate(a.type, payload)
    if not ok:
        print(f"invalid: {err}", file=sys.stderr)
        return EXIT_INVALID
    print(to_wire(a.type, payload))
    return EXIT_OK


def cmd_parse(a):
    line = a.line if a.line else sys.stdin.readline()
    try:
        evt_type, payload = parse_wire(line)
    except ValueError as e:
        print(f"invalid: {e}", file=sys.stderr)
        return EXIT_INVALID
    print(json.dumps({"type": evt_type, "payload": payload}, ensure_ascii=False))
    return EXIT_OK


def cmd_speak(a):
    line = a.line if a.line else sys.stdin.readline()
    try:
        evt_type, payload = parse_wire(line)
    except ValueError as e:
        print(f"invalid: {e}", file=sys.stderr)
        return EXIT_INVALID
    print(speak(evt_type, payload))
    return EXIT_OK


def cmd_types(a):
    print(json.dumps({t: SCHEMA[t] for t in SCHEMA}, ensure_ascii=False, indent=1))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="이벤트 enum 계약 v1 (P0-4)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("emit")
    c.add_argument("type")
    c.add_argument("--field", action="append", help="key=value (반복 가능)")
    c.add_argument("--payload", help="JSON 문자열(--field 대신)")
    c.set_defaults(fn=cmd_emit)

    c = sub.add_parser("parse")
    c.add_argument("line", nargs="?", help="생략 시 stdin")
    c.set_defaults(fn=cmd_parse)

    c = sub.add_parser("speak")
    c.add_argument("line", nargs="?", help="생략 시 stdin")
    c.set_defaults(fn=cmd_speak)

    c = sub.add_parser("types")
    c.set_defaults(fn=cmd_types)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
