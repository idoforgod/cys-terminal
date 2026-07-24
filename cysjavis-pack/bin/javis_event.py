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
import subprocess
import sys
import time

import javis_scrub  # ★G2: 기록·전파 직전 비밀 마스킹(같은 폴더 형제 모듈 — 부재 시 즉시 실패=fail-closed)

EXIT_OK, EXIT_USAGE, EXIT_INVALID = 0, 2, 6
EXIT_DEGRADED = 4  # 해법④(2026-07-14 오너 승인): 미지 타입=부패 아닌 '방출자가 더 새로움' — 무음 드롭 금지

WIRE_PREFIX = "[EVT v2]"
WIRE_RE = re.compile(r"^\[EVT v[12]\]\s+(?P<type>[a-z_.]+)\s+(?P<json>\{.*\})\s*$")

# spool 귀속 key(--surface): bare surface:N 또는 정식 부서 키 <slug>@surface:N (검증 확장·§4d).
SURFACE_KEY_RE = re.compile(r"^(?:[a-z0-9_-]{1,32}@)?surface:\d{1,8}$")
CYS_BIN = os.environ.get("CYS_BIN", "cys")


def _resolve_slug_from_socket():
    """CYS_SOCKET env → 부서 slug (P2-4). 부재=본부 main. depts.json socket 매칭.

    매칭 실패 시 None(미귀속 폴백 — fail-open 금지: main 으로 오귀속하지 않는다).
    """
    sock = os.environ.get("CYS_SOCKET")
    if not sock:
        return "main"   # 본부 기본(CYS_SOCKET 미설정 노드 = 본부)
    reg = os.environ.get("CYS_DEPTS_JSON") or os.path.expanduser("~/.cys/depts.json")
    try:
        with open(reg) as f:
            depts = (json.load(f) or {}).get("depts") or {}
    except (OSError, ValueError):
        return None
    for name, meta in depts.items():
        if isinstance(meta, dict) and meta.get("socket") == sock:
            return name
    return None   # 매칭 실패 → 미귀속


def _resolve_surface_ref():
    """`cys identify` → caller.surface_ref (P2-4). 실패·surface 밖이면 None."""
    try:
        out = subprocess.run([CYS_BIN, "identify"], capture_output=True, text=True, timeout=5)
    except Exception:
        return None
    if out.returncode != 0:
        return None
    try:
        d = json.loads(out.stdout)
    except (ValueError, TypeError):
        return None
    ref = ((d or {}).get("caller") or {}).get("surface_ref")
    return ref if isinstance(ref, str) and ref else None


def resolve_auto_surface():
    """--surface auto → 방출 노드의 정식 키 <slug>@surface:N (P2-4).

    slug(CYS_SOCKET→depts.json)·surface_ref(cys identify) 중 하나라도 해석 실패 시 None
    (미귀속 폴백 — fail-open 금지). 순수 조립부만 분리해 테스트에서 두 해석기를 주입한다.
    """
    slug = _resolve_slug_from_socket()
    if slug is None:
        return None
    ref = _resolve_surface_ref()
    if ref is None:
        return None
    return f"{slug}@{ref}"

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
    # ★W3(능력파리티 T3 · 2026-07-24): 편성 상태 수명 이벤트(음성/HUD 트랙). UI 배너 소멸/갱신은
    # 데몬 feed 버스(feed.item.created, payload.kind=formation-*)가 담당하는 별개 층이다
    # (handlers.rs:2814 층 분리 주석 참조). 계약 SOT=_round/EVENT_CONTRACT.md(이 리포 체크아웃엔
    # 부재 — 이 SCHEMA/SPEAK 항목과 아래 주석이 리포 내 소스 계약). 방출자=javis_formation._emit_evt.
    "boot.formation.complete": ["socket", "state", "kind"],
    "boot.formation.partial": ["socket", "state", "kind"],
    "boot.formation.pending": ["socket", "state", "kind"],
    "boot.formation.failed": ["socket", "state", "kind"],
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
    "boot.formation.complete": "팀 편성이 완결됐습니다(마스터+4종 의무 노드).",
    "boot.formation.partial": "팀이 일부만 편성됐습니다({state}). 나머지 CLI 설치 시 자동 완결됩니다.",
    "boot.formation.pending": "팀 편성이 대기 중입니다({state}).",
    "boot.formation.failed": "팀 편성에 실패했습니다({state}).",
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


def parse_wire(line, lenient_unknown=False):
    """(evt_type, payload) 또는 ValueError. lenient_unknown=미지 타입만 통과(해법④)."""
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
        # 해법④: lenient_unknown 시 '미지 타입'만 통과시켜 호출부가 강등 표시(EXIT_DEGRADED)로
        # 처리하게 한다 — 신형 방출자의 이벤트가 구형 소비자에서 부패(invalid)로 오분류되어
        # 무음 소실되는 경로 차단. 형식·JSON·필수키 오류는 종전대로 거부(계약 불변).
        if lenient_unknown and evt_type not in SCHEMA:
            return evt_type, payload
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


def _spool_path():
    """HUD spool 경로 — $HUD_STATE_DIR 우선, 없으면 <pack>/state/ (브리지 tailer와 동일 SOT)."""
    base = os.environ.get("HUD_STATE_DIR")
    if not base:
        base = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "state")
    return os.path.join(base, "evt_spool.jsonl")


def _spool_append(evt_type, payload, surface):
    """검증 통과 이벤트를 HUD spool에 O_APPEND 단일 write로 원자 append (동시 방출 안전)."""
    path = _spool_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    entry = {"ts": time.time(), "type": evt_type, "payload": javis_scrub.scrub_obj(payload)}
    if surface:
        entry["key"] = surface
    line = (json.dumps(entry, ensure_ascii=False) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


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
    surface = getattr(a, "surface", None)
    if surface == "auto":   # P2-4: 방출 노드 정식 키 자동 해석(실패=미귀속 폴백·fail-open 금지)
        surface = resolve_auto_surface()
    elif surface is not None and not SURFACE_KEY_RE.match(surface):
        print(f"invalid: bad --surface key: {surface} "
              f"(surface:N · <slug>@surface:N · auto)", file=sys.stderr)
        return EXIT_INVALID
    print(to_wire(a.type, payload))
    if getattr(a, "spool", False):
        try:  # spool 기록 실패는 wire 방출을 막지 않는다 (best-effort 수송로)
            _spool_append(a.type, payload, surface)   # auto 해석·검증 통과한 key(또는 None)
        except OSError:
            pass
    return EXIT_OK


def cmd_parse(a):
    line = a.line if a.line else sys.stdin.readline()
    try:
        evt_type, payload = parse_wire(line, lenient_unknown=True)
    except ValueError as e:
        print(f"invalid: {e}", file=sys.stderr)
        return EXIT_INVALID
    if evt_type not in SCHEMA:
        print(json.dumps({"type": evt_type, "payload": payload, "degraded": "unknown-type",
                          "hint": "소비자 스키마보다 새 이벤트 — 스키마 갱신 전까지 강등 표시(무음 드롭 금지)"},
                         ensure_ascii=False))
        print(f"degraded: unknown type {evt_type} — 소비자 스키마 갱신 필요(EVENT_CONTRACT)", file=sys.stderr)
        return EXIT_DEGRADED
    print(json.dumps({"type": evt_type, "payload": payload}, ensure_ascii=False))
    return EXIT_OK


def cmd_speak(a):
    line = a.line if a.line else sys.stdin.readline()
    try:
        evt_type, payload = parse_wire(line, lenient_unknown=True)
    except ValueError as e:
        print(f"invalid: {e}", file=sys.stderr)
        return EXIT_INVALID
    if evt_type not in SCHEMA:
        print(f"새 유형의 이벤트({evt_type})를 수신했습니다 — 소비자 스키마 갱신 필요.")
        return EXIT_DEGRADED
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
    c.add_argument("--spool", action="store_true", help="wire 방출에 더해 HUD spool에 append")
    c.add_argument("--surface",
                   help="spool 노드 귀속 key — auto(표준·자동 정식 키 해석) · dept-1@surface:12 · surface:12")
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
