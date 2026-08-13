#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_snapshot — BOOT_SNAPSHOT 관측 도구(임무 아님 · 읽기 전용 · W-수리2 2026-08-13).

clear 후 master 가 '기억' 대신 **배달 원장·티켓·큐의 도구 판독**을 귀속·잔무 관측의
근거로 쓰게 한다. 이 도구는 어떤 판정도 소유하지 않는다 — 착수 판정은 javis_mission.py
status, 귀속 판별 절차는 MASTER_DIRECTIVE '귀속 판별' 절이 정본이다.

계약(설계 SOT DESIGN_attribution_wiring.md B-1):
  · 읽기 전용 — 큐 변이·enqueue/drain·데몬 소켓 호출 절대 금지(자기인가 벡터 차단).
  · 원장·대장 판독 규칙은 javis_mission 의 기존 함수를 import 재사용(자체 재구현 금지).
  · stdout 은 ASCII, 산출 파일은 UTF-8 · 원자 쓰기(tmp+rename) · 총 max-bytes 캡.
  · 명령형 문구('착수하라' 류)를 생성하지 않는다 — 전부 관측 서술이다.

CLI:
  javis_snapshot.py generate --round-dir <dir> [--max-bytes 4096]
  javis_snapshot.py is-master        # exit 0=master pane / 1=아님(fail-quiet)
  javis_snapshot.py --self-test
"""
import json
import os
import sys
import time

_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


DEFAULT_MAX_BYTES = 4096
DIGEST_WINDOW_S = 86400          # 다이제스트 창(24h) — 판정 아님·표시 전용
SNAPSHOT_BASENAME = "BOOT_SNAPSHOT.md"
TRUNC_MARKER = "\n…(캡 절단)\n"
SECTION_MARKER = "\n…(섹션 캡)"
# 섹션별 개별 캡(bytes) — 전체 캡과 동시 적용(설계 B-1). 합계가 총캡 미만이 되게 배분.
SECTION_CAPS = {"tasks": 700, "wakeups": 700, "delivery": 1800, "gate": 300}


def _mission_mod():
    """javis_mission import(판독 규칙 단일 소유자). 부재는 관측 생략(graceful)."""
    try:
        import javis_mission
        return javis_mission
    except Exception:
        return None


def _fresh_task_mod():
    """javis_task 재적재 — ROOT 가 import 시점에 얼므로 JAVIS_ROOT 설정 후 새로 적재한다.
    (import 인자는 상수 문자열 — test_import_guard BLOCK-1 정적 증명 가능 형태 준수.)"""
    import importlib
    sys.modules.pop("javis_task", None)
    return importlib.import_module("javis_task")


def _fresh_wakeup_mod():
    """javis_wakeup 재적재 — 사유·형태는 _fresh_task_mod 와 동일."""
    import importlib
    sys.modules.pop("javis_wakeup", None)
    return importlib.import_module("javis_wakeup")


def _a(s):
    """stdout ASCII 계약용 접기 — 동적 값(경로·예외)을 ASCII 로 안전 표기."""
    return str(s).encode("ascii", "backslashreplace").decode("ascii")


def _clip(s, n):
    """파일 내용용 1줄 절단(개행 제거). 파일은 UTF-8 이라 한글 보존."""
    s = " ".join(str("-" if s is None else s).split())
    return s if len(s) <= n else s[: max(0, n - 1)] + "…"


def is_master():
    """(bool, ascii 사유) — generate·is-master 공통 마스터 게이트(설계 B-1).

    env CYS_ROLE=="master" OR (surface id 비어있지 않고 임무 대장 레코드 surface 와 일치).
    surface 판독은 javis_mission._surface(신·구 env 통일 규약), 대장 판독은 read_ledger
    재사용. 대장 부재·판독 불가 = 불통과(fail-quiet)."""
    if (os.environ.get("CYS_ROLE", "") or "").strip().lower() == "master":
        return True, "env CYS_ROLE=master"
    jm = _mission_mod()
    if jm is None:
        return False, "javis_mission unavailable"
    try:
        me = jm._surface()
    except Exception:
        me = ""
    if not me:
        return False, "no surface id in env"
    try:
        rec, err = jm.read_ledger()
    except Exception:
        rec, err = None, "read error"
    if err or not isinstance(rec, dict):
        return False, "mission ledger absent or unreadable"
    if (rec.get("surface", "") or "") == me:
        return True, "surface matches mission ledger"
    return False, "surface mismatch with mission ledger"


def _fmt_epoch(ts):
    try:
        return time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(float(ts)))
    except Exception:
        return "-"


def _delivery_records(jm, now):
    """(recs|None, err) — 원장 '표시용' 열람(24h 창). 파일 판독 규칙(크기 상한·손상 판정)은
    javis_mission._read_ledger_lines, 경로는 delivery_ledger_path, 스키마 필터는
    SCHEMA_VERSION 을 **재사용**한다. 기계/오너 '판정'은 여기서 하지 않는다(read_delivery
    소유) — 이 함수는 레코드를 접거나 버리는 판정 없이 사실을 나열만 한다."""
    p = jm.delivery_ledger_path()
    if not p:
        return None, "ledger path unavailable"
    lines = []
    for cand in (p + ".1", p):           # 회전 세대(.1) → 본 파일 순(read_delivery 동일 규약)
        if os.path.exists(cand) and not os.path.isdir(cand):
            ls, err = jm._read_ledger_lines(cand)
            if err:
                return None, "unreadable"
            lines.extend(ls or [])
    out = []
    for ln in lines:
        ln = ln.strip()
        if not ln:
            continue
        try:
            rec = json.loads(ln)
        except Exception:
            continue                     # 손상 줄 계수·판정은 read_delivery 소유 — 표시만 생략
        if not isinstance(rec, dict) or rec.get("v") != jm.SCHEMA_VERSION:
            continue
        if rec.get("sha256") in (None, "-"):
            continue                     # 기동 표식(sentinel)은 배달이 아니다
        if rec.get("part") is not None:
            continue                     # 조각 레코드 — 전문 레코드로 대표(중복 계수 방지)
        try:
            ts = float(rec.get("ts_epoch"))
        except Exception:
            continue
        if now - ts > DIGEST_WINDOW_S:
            continue
        out.append(rec)
    out.sort(key=lambda r: (r.get("ts_epoch") or 0))
    return out, None


def _section_delivery():
    """기계 배달 다이제스트(핵심) — 귀속 판단을 기억이 아니라 원장 관측으로 시작하게 한다."""
    lines = ["## 기계 배달 다이제스트 (javis_mission 원장 판독 재사용 · 최근 24h · 관측)"]
    jm = _mission_mod()
    if jm is None:
        lines.append("- 판독 불가(javis_mission 부재) — 관측 생략")
        return lines
    now = time.time()
    try:
        matches, status, detail = jm.read_delivery(now)
    except Exception as e:
        lines.append("- 판독 불가(read_delivery %s) — 관측 생략" % e.__class__.__name__)
        return lines
    if status == jm.LEDGER_UNREADABLE:
        lines.append("- 원장 판독 불가(fail-closed): %s" % _clip(detail, 110))
        lines.append("- 귀속 근거 없음 상태 — 판정 불가로 접는다(MASTER_DIRECTIVE '귀속 판별' 절)")
        return lines
    if status == jm.LEDGER_ABSENT:
        lines.append("- 원장 부재(기계 배달 이력 없음 — 정상일 수 있음)")
        return lines
    stale = sum(1 for m in matches.values() if isinstance(m, dict) and m.get("stale"))
    recs, err = _delivery_records(jm, now)
    if recs is None:
        lines.append("- 표시용 열람 실패(%s) · 층1 대조(내 pane 앞): %d건" % (err, len(matches)))
        return lines
    by_origin = {}
    for r in recs:
        k = str(r.get("origin") or "-")
        by_origin[k] = by_origin.get(k, 0) + 1
    lines.append("- 층1 대조(내 pane 앞 배달): %d건(창 밖 %d) · 24h 전 레인: %d건"
                 % (len(matches), stale, len(recs)))
    lines.append("- origin별: %s" % (" · ".join("%s=%d" % (k, by_origin[k])
                                                for k in sorted(by_origin)) or "없음"))
    for r in recs[-5:]:
        lines.append("- %s surface=%s origin=%s from=%s \"%s\""
                     % (_fmt_epoch(r.get("ts_epoch")), _clip(r.get("surface"), 12),
                        _clip(r.get("origin"), 14), _clip(r.get("from"), 12),
                        _clip(r.get("preview"), 40)))
    try:
        me = jm._surface()
    except Exception:
        me = ""
    if me:
        sent = [r for r in recs if str(r.get("from") or "") == me]
        lines.append("- 내 발신(from=%s): %d건" % (_clip(me, 12), len(sent)))
        for r in sent[-3:]:
            lines.append("  - %s -> surface=%s \"%s\"" % (_fmt_epoch(r.get("ts_epoch")),
                         _clip(r.get("surface"), 12), _clip(r.get("preview"), 40)))
    return lines


def _section_tasks(queue_ok):
    """미결 티켓 — javis_task._list_tasks 재사용(코드로 읽기 전용 확인: listdir+json 판독뿐).
    ★택1 근거(설계 B-1): import 우선을 택한다. javis_task 는 최상단 `import fcntl` 이라
    Windows 에서는 in-process/서브커맨드 양쪽 다 같은 이유로 죽는다 — 폴백이 이득이 없어
    graceful 생략이 정답이다. ROOT 가 import 시점에 얼므로 JAVIS_ROOT 설정 후 재적재."""
    lines = ["## 미결 티켓 (javis_task 판독 재사용 · 읽기 전용 · 보고 대상이지 착수 대상 아님)"]
    if not queue_ok:
        lines.append("- round-dir 명이 _round 가 아니라 큐 관측 생략(경로 규약: <ws>/_round)")
        return lines
    try:
        jt = _fresh_task_mod()
        rows = jt._list_tasks()
        open_st = list(getattr(jt, "OPEN_STATUSES",
                               ["backlog", "todo", "in_progress", "in_review", "blocked"]))
    except Exception as e:
        lines.append("- 판독 불가(%s) — 관측 생략" % e.__class__.__name__)
        return lines
    open_rows = [t for t in rows if isinstance(t, dict) and t.get("status") in open_st]
    lines.append("- 미결 %d건 / 전체 %d건" % (len(open_rows), len(rows)))
    recent = sorted(open_rows, key=lambda t: str(t.get("updated_at", "")), reverse=True)
    for t in recent[:5]:
        lines.append("- %s [%s] %s" % (_clip(t.get("id"), 24), _clip(t.get("status"), 12),
                                       _clip(t.get("title"), 60)))
    return lines


def _section_wakeups(queue_ok):
    """대기 wakeup — javis_wakeup._iter_pending 재사용(읽기 전용 확인: listdir+json 판독뿐)."""
    lines = ["## 대기 wakeup (javis_wakeup 판독 재사용 · 읽기 전용 · 배달·취소는 하지 않음)"]
    if not queue_ok:
        lines.append("- round-dir 명이 _round 가 아니라 큐 관측 생략(경로 규약: <ws>/_round)")
        return lines
    try:
        jw = _fresh_wakeup_mod()
        rows = [rec for _p, rec in jw._iter_pending()]
    except Exception as e:
        lines.append("- 판독 불가(%s) — 관측 생략" % e.__class__.__name__)
        return lines
    lines.append("- 대기 %d건" % len(rows))
    recent = sorted(rows, key=lambda r: str(r.get("updated_at", "")), reverse=True)
    for r in recent[:5]:
        lines.append("- %s [%s] target=%s task=%s reason=%s"
                     % (_clip(r.get("id"), 16), _clip(r.get("severity"), 8),
                        _clip(r.get("target"), 16), _clip(r.get("task_key"), 24),
                        _clip(r.get("reason"), 40)))
    return lines


def _section_gate():
    """임무 게이트 상태 1줄 — javis_mission.gate 요약 필드 재사용(exit 판정 아님·표시 전용)."""
    lines = ["## 임무 게이트 (javis_mission status 요약 — 판정 소유자는 그 도구의 exit code)"]
    jm = _mission_mod()
    if jm is None:
        lines.append("- 판독 불가(javis_mission 부재)")
        return lines
    try:
        _rc, v = jm.gate()
        lines.append("- have_mission=%s · %s"
                     % (str(bool(v.get("have_mission"))).lower(),
                        _clip(v.get("mission") or v.get("reason"), 100)))
    except Exception as e:
        lines.append("- 판독 불가(gate %s)" % e.__class__.__name__)
    return lines


def _header(now):
    return ["# BOOT_SNAPSHOT — 도구 산출 관측 (임무 아님)",
            "생성: %s · javis_snapshot.py (관측 전용 · 읽기 전용)" % _fmt_epoch(now),
            "",
            "★이 문서는 관측 보고다. 어떤 항목도 임무·착수 지시가 아니다. "
            "착수 판정의 단일 소유자는 javis_mission.py status다. "
            "이전 세션 잔무는 보고 대상이지 자동 착수 대상이 아니다."]


def _cap_bytes(text, limit, marker):
    """UTF-8 경계 안전 바이트 캡 — 초과 시 절단 후 말미 마커(결과 ≤ limit 보장)."""
    b = text.encode("utf-8")
    if len(b) <= limit:
        return text
    mb = marker.encode("utf-8")
    cut = b[: max(0, limit - len(mb))]
    while cut and (cut[-1] & 0xC0) == 0x80:
        cut = cut[:-1]
    return cut.decode("utf-8", "ignore") + marker


def _safe_section(name, fn, *a):
    try:
        body = "\n".join(fn(*a))
    except Exception as e:
        body = "## (섹션 판독 불가: %s)" % e.__class__.__name__
    return _cap_bytes(body, SECTION_CAPS.get(name, 900), SECTION_MARKER)


def cmd_generate(argv):
    """<round-dir>/BOOT_SNAPSHOT.md 산출. 비마스터=skip(exit 0) · 실패는 ASCII 1줄(훅이 흡수)."""
    round_dir, max_bytes = None, DEFAULT_MAX_BYTES
    i = 0
    while i < len(argv):
        if argv[i] == "--round-dir" and i + 1 < len(argv):
            round_dir = argv[i + 1]
            i += 2
        elif argv[i] == "--max-bytes" and i + 1 < len(argv):
            try:
                max_bytes = max(256, int(argv[i + 1]))
            except ValueError:
                sys.stderr.write("error: --max-bytes must be an integer\n")
                return 2
            i += 2
        else:
            sys.stderr.write("error: unknown arg %s\n" % _a(argv[i]))
            return 2
    if not round_dir:
        sys.stderr.write("usage: javis_snapshot.py generate --round-dir <dir> [--max-bytes N]\n")
        return 2
    ok, why = is_master()
    if not ok:
        print("skip: not-master (%s)" % why)
        return 0
    if not os.path.isdir(round_dir):
        sys.stderr.write("error: round-dir not found: %s\n" % _a(round_dir))
        return 1
    rd = os.path.abspath(os.path.normpath(round_dir))
    queue_ok = os.path.basename(rd) == "_round"
    prev_root = os.environ.get("JAVIS_ROOT")
    os.environ["JAVIS_ROOT"] = os.path.dirname(rd)   # 형제 모듈의 ROOT/_round/* 규약에 정렬
    try:
        now = time.time()
        parts = ["\n".join(_header(now)),
                 _safe_section("tasks", _section_tasks, queue_ok),
                 _safe_section("wakeups", _section_wakeups, queue_ok),
                 _safe_section("delivery", _section_delivery),
                 _safe_section("gate", _section_gate)]
    finally:
        if prev_root is None:
            os.environ.pop("JAVIS_ROOT", None)
        else:
            os.environ["JAVIS_ROOT"] = prev_root
    body = _cap_bytes("\n\n".join(parts) + "\n", max_bytes, TRUNC_MARKER)
    path = os.path.join(rd, SNAPSHOT_BASENAME)
    tmp = path + ".tmp.%d" % os.getpid()
    try:
        with open(tmp, "wb") as f:
            f.write(body.encode("utf-8"))
        os.replace(tmp, path)            # 원자 교체 — 부분 파일 미노출
    except Exception as e:
        try:
            os.path.exists(tmp) and os.remove(tmp)
        except Exception:
            pass
        sys.stderr.write("error: write failed (%s)\n" % _a(e.__class__.__name__))
        return 1
    print("ok: %s (%d bytes)" % (_a(path), len(body.encode("utf-8"))))
    return 0


def cmd_is_master(argv):
    ok, why = is_master()
    print("master (%s)" % why if ok else "not-master (%s)" % why)
    return 0 if ok else 1


def _st_env(extra=None):
    """self-test 밀폐 env — ambient 역할·surface·레인·상태 경로를 전부 걷어낸다."""
    env = dict(os.environ)
    for k in ("CYS_ROLE", "CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_MISSION",
              "CYS_SOCKET", "JAVIS_ROOT", "CYS_STATE_DIR"):
        env.pop(k, None)
    if extra:
        env.update(extra)
    return env


def _st_ws(td):
    """임시 워크스페이스 (ws, rd, state) — 티켓·wakeup·상태 디렉터리 골격."""
    ws = os.path.join(td, "ws")
    rd = os.path.join(ws, "_round")
    state = os.path.join(td, "state")
    os.makedirs(os.path.join(rd, "tasks"))
    os.makedirs(os.path.join(rd, "wakeups", "pending"))
    os.makedirs(state)
    return ws, rd, state


def _run_self_test_cases(run, check, drec):
    import tempfile
    # ── ① 게이트 skip: 역할·surface 전무 → generate 는 파일 없이 skip(exit 0) ──
    with tempfile.TemporaryDirectory() as td:
        _ws, rd, state = _st_ws(td)
        rc, out, _e = run(["generate", "--round-dir", rd], {"CYS_STATE_DIR": state})
        check("gate-skip rc==0", rc == 0, "rc=%d" % rc)
        check("gate-skip stdout", out.startswith("skip: not-master"), out[:60])
        check("gate-skip no file", not os.path.exists(os.path.join(rd, SNAPSHOT_BASENAME)))
    # ── ② is-master exit 코드 4상: role env / 대장 일치 / 불일치 / 대장 부재 ──
    with tempfile.TemporaryDirectory() as td:
        state = os.path.join(td, "state")
        os.makedirs(state)
        rc, _o, _e = run(["is-master"], {"CYS_ROLE": "master", "CYS_STATE_DIR": state})
        check("is-master role env -> 0", rc == 0, "rc=%d" % rc)
        rc, _o, _e = run(["is-master"], {"CYS_STATE_DIR": state, "CYS_SURFACE_ID": "7"})
        check("is-master ledger absent -> 1", rc == 1, "rc=%d" % rc)
        with open(os.path.join(state, "mission.json"), "w", encoding="utf-8") as f:
            json.dump({"schema": 1, "mission": "m", "surface": "7"}, f)
        rc, _o, _e = run(["is-master"], {"CYS_STATE_DIR": state, "CYS_SURFACE_ID": "7"})
        check("is-master surface match -> 0", rc == 0, "rc=%d" % rc)
        rc, _o, _e = run(["is-master"], {"CYS_STATE_DIR": state, "CYS_SURFACE_ID": "8"})
        check("is-master surface mismatch -> 1", rc == 1, "rc=%d" % rc)
        rc, _o, _e = run(["is-master"], {"CYS_STATE_DIR": state})
        check("is-master no surface -> 1", rc == 1, "rc=%d" % rc)
    # ── ③ 정상 생성: 티켓·wakeup·원장 fixture → 내용·캡·원자성·읽기전용 실측 ──
    with tempfile.TemporaryDirectory() as td:
        _ws, rd, state = _st_ws(td)
        with open(os.path.join(rd, "tasks", "T-alpha.json"), "w", encoding="utf-8") as f:
            json.dump({"id": "T-alpha", "title": "티켓 알파", "status": "in_progress",
                       "updated_at": "2026-08-13T00:00:00+0000"}, f, ensure_ascii=False)
        with open(os.path.join(rd, "wakeups", "pending", "m__k.json"), "w",
                  encoding="utf-8") as f:
            json.dump({"id": "W-selftest01", "target": "master", "task_key": "k",
                       "reason": "관측", "severity": "warn",
                       "updated_at": "2026-08-13T00:00:00+0000"}, f, ensure_ascii=False)
        dp = os.path.join(state, "delivery-base.jsonl")
        with open(dp, "w", encoding="utf-8") as f:
            f.write(drec("워커에게 보낸 위임 티켓", surface="2", origin="send", frm="1"))
            f.write(drec("깨어나라 다음 점검", surface="1", origin="queue", frm=None))
        before = {p: open(p, "rb").read() for p in
                  (dp, os.path.join(rd, "tasks", "T-alpha.json"))}
        env = {"CYS_ROLE": "master", "CYS_STATE_DIR": state, "CYS_SURFACE_ID": "1"}
        rc, out, err = run(["generate", "--round-dir", rd], env)
        check("generate rc==0", rc == 0, "rc=%d err=%s" % (rc, err[:80]))
        snap = os.path.join(rd, SNAPSHOT_BASENAME)
        check("snapshot exists", os.path.exists(snap))
        body = open(snap, encoding="utf-8").read()
        check("size<=4096", len(body.encode("utf-8")) <= DEFAULT_MAX_BYTES)
        for needle in ("BOOT_SNAPSHOT", "임무 아님", "T-alpha", "W-selftest01",
                       "origin", "send=1", "queue=1", "내 발신(from=1): 1건"):
            check("content has %s" % needle, needle in body, body[:200])
        for bad in ("착수하라", "실행하라", "진행하라"):
            check("non-imperative %s" % bad, bad not in body)
        check("atomic no tmp", not [n for n in os.listdir(rd) if ".tmp." in n])
        after = {p: open(p, "rb").read() for p in before}
        check("read-only inputs", before == after)
    # ── ④ 캡 절단: 소형 --max-bytes → 크기 준수 + 말미 마커 ──
    with tempfile.TemporaryDirectory() as td:
        _ws, rd, state = _st_ws(td)
        dp = os.path.join(state, "delivery-base.jsonl")
        with open(dp, "w", encoding="utf-8") as f:
            for i in range(30):
                f.write(drec("장문 배달 본문 %d 반복 padding padding" % i))
        rc, _o, _e = run(["generate", "--round-dir", rd, "--max-bytes", "700"],
                         {"CYS_ROLE": "master", "CYS_STATE_DIR": state})
        check("cap rc==0", rc == 0, "rc=%d" % rc)
        raw = open(os.path.join(rd, SNAPSHOT_BASENAME), "rb").read()
        check("cap size<=700", len(raw) <= 700, "size=%d" % len(raw))
        check("cap marker", raw.decode("utf-8").rstrip().endswith("(캡 절단)"))
    # ── ⑤ 입력 부재 graceful: 빈 상태 → 생성 성공 + 부재 고지 ──
    with tempfile.TemporaryDirectory() as td:
        rd = os.path.join(td, "_round")
        os.makedirs(rd)
        state = os.path.join(td, "state")
        os.makedirs(state)
        rc, _o, _e = run(["generate", "--round-dir", rd],
                         {"CYS_ROLE": "master", "CYS_STATE_DIR": state})
        check("absent rc==0", rc == 0, "rc=%d" % rc)
        body = open(os.path.join(rd, SNAPSHOT_BASENAME), encoding="utf-8").read()
        check("absent notes ledger", "원장 부재" in body, body[-200:])
        check("absent notes tasks", "미결 0건" in body, body[:400])
    # ── ⑥ 원장 자리가 디렉터리(판독 불가) → fail-closed 문구·생성은 성공 ──
    with tempfile.TemporaryDirectory() as td:
        _ws, rd, state = _st_ws(td)
        os.makedirs(os.path.join(state, "delivery-base.jsonl"))
        rc, _o, _e = run(["generate", "--round-dir", rd],
                         {"CYS_ROLE": "master", "CYS_STATE_DIR": state})
        check("unreadable rc==0", rc == 0, "rc=%d" % rc)
        body = open(os.path.join(rd, SNAPSHOT_BASENAME), encoding="utf-8").read()
        check("unreadable fail-closed", "판독 불가" in body, body[-300:])


def cmd_self_test():
    """밀폐 자기검증(팩 --self-test 관례) — subprocess 로 실제 CLI(exit code)를 구동한다."""
    import subprocess
    import tempfile
    jm = _mission_mod()
    if jm is None:
        print("javis_snapshot self-test SKIP (javis_mission missing)")
        return 1
    fails = []
    outs = []
    self_path = os.path.abspath(__file__)

    def run(args, extra):
        r = subprocess.run([sys.executable, self_path] + args, capture_output=True,
                           text=True, timeout=60, env=_st_env(extra), cwd=_SELF_DIR)
        outs.append(r.stdout)
        return r.returncode, r.stdout, r.stderr

    def check(name, cond, detail=""):
        if not cond:
            fails.append("%s%s" % (name, (" -- " + detail) if detail else ""))

    def drec(text, surface="2", origin="send", frm="1", ts=None):
        return json.dumps({"v": jm.SCHEMA_VERSION, "surface": surface,
                           "ts_epoch": time.time() if ts is None else ts, "ts": "-",
                           "sha256": jm.delivery_digest(text), "chars": len(text),
                           "preview": text[:32], "origin": origin, "from": frm},
                          ensure_ascii=False) + "\n"

    _run_self_test_cases(run, check, drec)
    if fails:
        print("javis_snapshot self-test FAIL (%d):" % len(fails))
        for f in fails:
            print("  - %s" % _a(f))
        return 1
    for o in outs:
        try:
            o.encode("ascii")
        except UnicodeEncodeError:
            print("javis_snapshot self-test FAIL: non-ascii stdout observed")
            return 1
    print("javis_snapshot self-test PASS (gate-skip / is-master exits / generate / "
          "cap-trunc / graceful-absent / unreadable-dir / atomicity / read-only / "
          "non-imperative / ascii-stdout)")
    return 0


def main(argv):
    if "--self-test" in argv:
        return cmd_self_test()
    cmd = argv[1] if len(argv) > 1 else ""
    if cmd == "generate":
        return cmd_generate(argv[2:])
    if cmd == "is-master":
        return cmd_is_master(argv[2:])
    sys.stderr.write("usage: javis_snapshot.py [generate --round-dir <dir> [--max-bytes N]"
                     " | is-master | --self-test]\n")
    return 64


if __name__ == "__main__":
    sys.exit(main(sys.argv))
