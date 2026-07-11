#!/usr/bin/env python3
"""javis_task.py — P0-1 원자적 태스크 체크아웃 (Paperclip 계약의 클린룸 파일 포트)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-1):
- 체크아웃은 원자적이다: 락 획득은 POSIX mkdir(원자적)로, 경쟁 시 정확히 1명만 승리.
- 충돌 = 409(exit 9): 살아있는 소유자가 있다는 뜻 — 재시도 금지(Never retry a 409).
- stale 판정은 시간 TTL이 아니라 run-liveness: 소유 pid가 죽었을 때만 락을 회수(adopt).
  adopt도 원자적: 기존 락 디렉터리 rename(원자적, 1명만 성공) 후 새로 획득.
- blocker는 "done"만 해소로 친다(cancelled는 미해소 잔존). 미해소 blocker가 있으면 체크아웃 거부(exit 4).
- 자동생성 중복 차단: 같은 (origin_kind, origin_fingerprint)의 열린 태스크가 있으면 create 거부(exit 8).
- 태스크가 done이 되면, 그 태스크를 기다리던 태스크들 중 blocker가 전부 해소된 것을 보고한다
  (wakeup 큐 연동은 javis_wakeup.py 몫 — 여기선 unblocked 목록 출력만).

상태 저장: $JAVIS_ROOT/_round/tasks/<id>.json (쓰기는 temp+os.replace 원자적)
락:        $JAVIS_ROOT/_round/tasks/<id>.lock/ (디렉터리 = mkdir 원자성) + owner.json

exit codes: 0 ok · 2 usage · 3 not found · 4 blocked · 8 duplicate origin · 9 conflict(409)
"""
import argparse
import contextlib
import getpass
import json
import os
import re
import socket
import sys
import time
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # safe_path(-P) guard for sibling imports (same idiom as javis_replay.py)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")

STATUSES = ["backlog", "todo", "in_progress", "in_review", "done", "blocked", "cancelled"]
OPEN_STATUSES = ["backlog", "todo", "in_progress", "in_review", "blocked"]
TERMINAL_STATUSES = ["done", "cancelled"]

EXIT_OK, EXIT_USAGE, EXIT_NOTFOUND, EXIT_BLOCKED, EXIT_DUP, EXIT_CONFLICT = 0, 2, 3, 4, 8, 9


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


# ★G10(cokacdir 성찰 2026-07-04): task id allowlist — 경로 조합 전 traversal 차단.
#   wakeup._safe(:45)와 동일 문자집합이나 조용한 치환 대신 '거부'(fail-loud) —
#   기존 유효 id의 경로 매핑을 바꾸지 않고, `--id ../../tmp/x` 류 임의 쓰기를 닫는다.
_ID_RE = re.compile(r"^[A-Za-z0-9._-]{1,80}$")


def _task_path(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.json")


def _lock_dir(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.lock")


def _wlock_dir(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.wlock")


@contextlib.contextmanager
def _wlock(task_id, timeout=5.0, stale=30.0):
    """★WP-8(P-ORCH): 태스크 JSON read-modify-write 단일 직렬화 쓰기락(<id>.wlock).

    소유권 락(<id>.lock)과 별개의 '쓰기 직렬화' 뮤텍스다 — set-status·checkout·release·create가
    같은 <id>.json 을 동시에 read-modify-write 하며 갱신을 서로 덮어쓰던 경로(이중 락 비배제)를
    닫는다. 임계구역은 JSON 갱신뿐(수 ms)이라 stale(30s) 회수가 산 소유자를 탈취하지 않는다 —
    긴 완료-하한선 sleep 은 이 락 '밖'에 둔다. 과경합으로 획득 실패 시 하드 실패 대신 경고 후
    무락 진행(기존 exit-code 계약 불변·최악의 경우 현행 무락 동작으로 degrade)."""
    os.makedirs(TASKS_DIR, exist_ok=True)
    path = _wlock_dir(task_id)
    deadline = time.time() + timeout
    acquired = False
    while True:
        try:
            os.mkdir(path)  # ← 원자적: 경쟁 시 1명만 성공
            acquired = True
            break
        except FileExistsError:
            try:  # 죽은 소유자가 남긴 만료 락은 원자적으로 회수(rename→rmdir, 빈 디렉터리)
                if time.time() - os.stat(path).st_mtime > stale:
                    stale_name = f"{path}.stale.{time.time_ns()}"
                    os.rename(path, stale_name)
                    with contextlib.suppress(OSError):
                        os.rmdir(stale_name)
                    continue
            except OSError:
                pass
            if time.time() > deadline:
                print(f"warn: wlock 획득 실패(과경합) — 무락 진행: {task_id}", file=sys.stderr)
                break
            time.sleep(0.02)
    try:
        yield
    finally:
        if acquired:
            with contextlib.suppress(OSError):
                os.rmdir(path)


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)  # 원자적 교체


def _read_task(task_id):
    try:
        with open(_task_path(task_id), encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return None


def _list_tasks():
    if not os.path.isdir(TASKS_DIR):
        return []
    out = []
    for name in sorted(os.listdir(TASKS_DIR)):
        if name.endswith(".json") and not name.startswith("."):
            t = _read_task(name[:-5])
            if t:
                out.append(t)
    return out


def _pid_alive(pid):
    """run-liveness: pid 생존 확인. 테스트 주입용 JAVIS_TASK_LIVENESS=alive|dead 지원."""
    override = os.environ.get("JAVIS_TASK_LIVENESS")
    if override == "alive":
        return True
    if override == "dead":
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, ValueError, TypeError):
        return False
    except PermissionError:
        return True  # 존재하지만 남의 프로세스


def _read_owner(task_id):
    try:
        with open(os.path.join(_lock_dir(task_id), "owner.json"), encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, NotADirectoryError, json.JSONDecodeError):
        return None


# ★WP-8(P-ORCH · lease-adopt 강화): '산 소유자 락 탈취' 창을 닫는다. _acquire_lock 의 adopt 는
#   이미 run-liveness 기반이다 — 읽을 수 있는 owner.json + 살아있는 pid 는 아래 :159 에서 conflict
#   로 보호되고, 죽은 pid 만 즉시 adopt 된다(그 경로는 이 grace 를 '건너뛴다'). 유일한 탈취 경로는
#   owner.json 이 일시 부재/훼손(holder is None)인 상태에서 grace 경과 후 adopt 하는 경로뿐이다.
#   pid 확인 불가(owner.json 없음)라 옵션 (a)'pid 사망 확인 한정'을 순수 적용하면 크래시-복구가
#   영구 deadlock 이 되므로, 옵션 (b) 'LEASE 상향'을 택해 5s→30s 로 올린다 — dead-pid adopt 는
#   무영향, ownerless 창만 길어져 산 소유자 탈취를 실질 0 에 수렴시키되 크래시 복구는 30s 내 보장.
OWNER_WRITE_GRACE_SEC = 30.0  # 락 획득~owner.json 기록 사이의 정당한 lease(과거 5.0 → P-ORCH 강화)


def _write_owner(lock, rec):
    """owner.json을 락 디렉터리 '재생성 없이' 기록. 락이 탈취돼 사라졌으면 False(경쟁 패배)."""
    tmp = os.path.join(TASKS_DIR, f".owner.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    try:
        os.replace(tmp, os.path.join(lock, "owner.json"))
        return True
    except (FileNotFoundError, NotADirectoryError, OSError):
        try:
            os.remove(tmp)
        except OSError:
            pass
        return False


def _acquire_lock(task_id, owner_id, pid=None):
    """mkdir 원자성 기반 CAS. 반환: ('acquired'|'conflict'|'adopted', owner_dict|None)

    pid 기본값은 호출자(부모 프로세스) — CLI 자신의 pid는 즉시 종료되어 liveness 근거가
    못 되기 때문. 장수 프로세스(워커 pane 등)에 묶으려면 --pid로 명시.
    """
    lock = _lock_dir(task_id)
    owner_rec = {
        "owner_id": owner_id,
        "pid": pid if pid is not None else os.getppid(),
        "host": socket.gethostname(),
        "user": getpass.getuser(),
        "acquired_at": _now(),
    }
    try:
        os.mkdir(lock)  # ← 원자적: 경쟁 시 1명만 성공
        if not _write_owner(lock, owner_rec):
            return "conflict", None  # 기록 창에서 락 탈취됨 — 경쟁 패배로 처리
        return "acquired", owner_rec
    except FileExistsError:
        pass
    holder = _read_owner(task_id)
    if holder and holder.get("owner_id") == owner_id:
        return "acquired", holder  # 자기 재진입(멱등)
    if holder and _pid_alive(holder.get("pid")):
        return "conflict", holder  # 살아있는 소유자 = 409
    if holder is None:
        # owner.json 미기록: 방금 획득한 경쟁자의 "기록 창"일 수 있음 — grace 이내면 409
        try:
            age = time.time() - os.stat(lock).st_mtime
        except OSError:
            age = OWNER_WRITE_GRACE_SEC  # 락이 사라짐 = 경쟁 진행 중
        if age < OWNER_WRITE_GRACE_SEC:
            return "conflict", None
    # stale(소유 pid 사망, 또는 grace 지난 owner.json 훼손) → 원자적 adopt: rename은 1명만 성공
    stale_name = f"{lock}.stale.{time.time_ns()}.{uuid.uuid4().hex[:6]}"
    try:
        os.rename(lock, stale_name)
    except (FileNotFoundError, OSError):
        return "conflict", _read_owner(task_id)  # 경쟁자가 먼저 adopt함 — 409로 취급
    try:
        os.mkdir(lock)
    except FileExistsError:
        return "conflict", _read_owner(task_id)
    if not _write_owner(lock, owner_rec):
        return "conflict", None
    # stale 잔해는 감사용으로 보존하지 않고 정리(내용은 owner.json 하나)
    try:
        p = os.path.join(stale_name, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(stale_name)
    except OSError:
        pass
    return "adopted", owner_rec


def _release_lock(task_id, owner_id, force=False):
    lock = _lock_dir(task_id)
    holder = _read_owner(task_id)
    if not os.path.isdir(lock):
        return True
    if not force and holder and holder.get("owner_id") != owner_id:
        return False
    try:
        p = os.path.join(lock, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(lock)
    except OSError:
        return False
    return True


def _unresolved_blockers(task):
    """blocker는 done만 해소(cancelled는 미해소 잔존 — Paperclip 계약 준수)."""
    out = []
    for bid in task.get("blocked_by", []):
        b = _read_task(bid)
        if b is None or b.get("status") != "done":
            out.append(bid)
    return out


def _find_unblocked_dependents(done_task_id):
    """방금 done이 된 태스크로 인해 blocker가 전부 해소된 열린 태스크 목록."""
    out = []
    for t in _list_tasks():
        if t.get("status") in OPEN_STATUSES and done_task_id in t.get("blocked_by", []):
            if not _unresolved_blockers(t):
                out.append(t["id"])
    return out


def cmd_create(a):
    task_id = a.id or f"T{time.strftime('%m%d')}-{uuid.uuid4().hex[:6]}"
    with _wlock(task_id):  # ★WP-8(P-ORCH): dup 검사~쓰기 직렬화 — 동시 create 경합의 lost-write 차단
        if _read_task(task_id):
            print(f"error: task exists: {task_id}", file=sys.stderr)
            return EXIT_DUP
        if a.origin_fingerprint:
            for t in _list_tasks():
                if (t.get("status") in OPEN_STATUSES
                        and t.get("origin_kind") == a.origin_kind
                        and t.get("origin_fingerprint") == a.origin_fingerprint):
                    print(f"duplicate-origin: open task {t['id']} has same "
                          f"({a.origin_kind},{a.origin_fingerprint}) — create 거부", file=sys.stderr)
                    return EXIT_DUP
        task = {
            "id": task_id,
            "title": a.title,
            "status": a.status,
            "why": a.why or "",           # goal 체인: 이 일이 왜 존재하는가
            "goal": a.goal or "",
            "blocked_by": a.blocked_by or [],
            "origin_kind": a.origin_kind,
            "origin_fingerprint": a.origin_fingerprint or "default",
            "owner": None,
            "created_at": _now(),
            "updated_at": _now(),
        }
        _write_json_atomic(_task_path(task_id), task)
    print(task_id)
    return EXIT_OK


def cmd_checkout(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    if task["status"] in TERMINAL_STATUSES:
        print(f"conflict: task is terminal ({task['status']}) — 재시도 금지", file=sys.stderr)
        return EXIT_CONFLICT
    unresolved = _unresolved_blockers(task)
    if unresolved:
        print(f"blocked: 미해소 blocker {unresolved} — 체크아웃 거부", file=sys.stderr)
        return EXIT_BLOCKED
    verdict, holder = _acquire_lock(a.id, a.owner, pid=a.pid)
    if verdict == "conflict":
        who = (holder or {}).get("owner_id", "unknown")
        print(f"conflict(409): 살아있는 소유자 {who} — 재시도 금지, 다른 태스크로 이동", file=sys.stderr)
        return EXIT_CONFLICT
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON 갱신 직렬화 — 동시 set-status 와의 lost-update 차단
        task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        task["status"] = "in_progress"
        task["owner"] = a.owner
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
    print(json.dumps({"checkout": verdict, "id": a.id, "owner": a.owner}, ensure_ascii=False))
    return EXIT_OK


def cmd_release(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ★G11(cokacdir 성찰 2026-07-04): --force 무검증 폐기 — 사유 필수 + 태스크 JSON에
    #   force_history 감사 기록. (암호학 verifier는 다중노드/원격 확장 시 — per-node secret
    #   없는 로컬 단일 시스템에서 해시는 검증이 아니라 장식이다. 성찰 G11 'P2 조건부' 준수.)
    if a.force and not getattr(a, "force_reason", None):
        print("error: --force는 --force-reason '<사유>' 필수 — 무검증 탈취 금지(G11)",
              file=sys.stderr)
        return EXIT_USAGE
    if not _release_lock(a.id, a.owner, force=a.force):
        holder = _read_owner(a.id)
        print(f"conflict(409): 락 소유자 불일치 (holder={(holder or {}).get('owner_id')})", file=sys.stderr)
        return EXIT_CONFLICT
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON 갱신 직렬화
        task = _read_task(a.id)          # 락 안 재독
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        if a.force:
            task.setdefault("force_history", []).append(
                {"at": _now(), "by": a.owner, "reason": a.force_reason})
        if task.get("owner") == a.owner or a.force:
            task["owner"] = None
            if task["status"] == "in_progress":
                task["status"] = "todo"
            task["updated_at"] = _now()
            _write_json_atomic(_task_path(a.id), task)
    print(f"released: {a.id}")
    return EXIT_OK


_PROBE_IDX = 0


def _settle_probe(owner):
    """1회 관측 → (idle_secs|None, todo_mtime|None). JAVIS_SETTLE_PROBE로 결정론 주입 가능
    (형식 "idle:mtime;idle:mtime" — 호출마다 하나 소비 · wakeup JAVIS_WAKEUP_LIVENESS 관례)."""
    global _PROBE_IDX
    seq = os.environ.get("JAVIS_SETTLE_PROBE")
    if seq is not None:
        parts = seq.split(";")
        p = parts[min(_PROBE_IDX, len(parts) - 1)]
        _PROBE_IDX += 1
        try:
            i, m = p.split(":", 1)
            return (float(i) if i else None), (float(m) if m else None)
        except ValueError:
            return None, None
    try:
        import javis_boot_node as _bn  # 형제 모듈(orchestra:194 관례) — 부재 시 관측불가=fail-closed
        st = _bn.cys_status()
        row = _bn.status_surface(st, owner) if st else None
    except Exception:
        row = None
    idle = row.get("idle_secs") if row else None
    todo = os.path.join(ROOT, "_round", "%s_TODO.md" % owner.upper())
    try:
        mt = os.stat(todo).st_mtime
    except OSError:
        mt = None
    return idle, mt


def _completion_settled(owner):
    """★G6(cokacdir 성찰 2026-07-04): 완료 하한선 — 단일 스냅샷 완료 판정 금지.
    owner 노드가 2회 연속(settle 간격) idle 안착 + 그 사이 TODO 갱신(mtime) 정지일 때만
    True(boot_node POLL-IDLE·channel_watch 2-strike 프리미티브 이식). sub-agent 활동은
    같은 surface의 idle_secs에 반영된다는 전제(부정확하면 idle 미안착으로 안전측 거부).
    반환 (ok, 사유)."""
    settle = float(os.environ.get("JAVIS_SETTLE_SEC", "5"))
    idle_min = float(os.environ.get("JAVIS_SETTLE_IDLE_MIN", "4"))
    last_mt = None
    for strike in range(2):
        if strike:
            time.sleep(settle)
        idle, mt = _settle_probe(owner)
        if idle is None:
            return False, "owner '%s' 상태 관측 불가 — 완료 거부(fail-closed)" % owner
        if idle < idle_min:
            return False, ("owner '%s' idle 미안착(idle=%s<%s) — sub-agent/작업 진행 중 추정"
                           % (owner, idle, idle_min))
        if strike and mt != last_mt:
            return False, "owner '%s' TODO 갱신 진행 중(mtime 변동) — 완료 아님" % owner
        last_mt = mt
    return True, "settled(2-strike idle)"


def cmd_set_status(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ★G6: done은 완료 하한선 통과 후에만 — 오종결이 의존자 unblock(:308)을 조기 발화하는
    #   경로를 닫는다. owner 없는(체크아웃 이력 없는) 태스크는 안착 대상이 없어 게이트 생략.
    #   ★WP-8(P-ORCH): 최대 settle sleep 은 wlock '밖'에서 — 락 장기점유·타임아웃을 피한다.
    settle_override_note = None
    if a.status == "done" and task.get("owner"):
        if getattr(a, "settled_override", None):
            settle_override_note = a.settled_override
        else:
            ok, why = _completion_settled(task["owner"])
            if not ok:
                print(json.dumps({"id": a.id, "status_denied": "done", "reason": why,
                                  "hint": "안착 후 재시도 또는 --settled-override '<사유>'"},
                                 ensure_ascii=False), file=sys.stderr)
                return EXIT_BLOCKED
    with _wlock(a.id):  # ★WP-8(P-ORCH): JSON read-modify-write 직렬화 — 동시 mutator lost-update 차단
        task = _read_task(a.id)          # 락 안 재독 — 대기 중 갱신 반영
        if not task:
            print(f"not found: {a.id}", file=sys.stderr)
            return EXIT_NOTFOUND
        if settle_override_note is not None:
            task.setdefault("settle_overrides", []).append(
                {"at": _now(), "reason": settle_override_note})
        task["status"] = a.status
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
        if a.status in TERMINAL_STATUSES:
            _release_lock(a.id, task.get("owner") or "", force=True)
            task["owner"] = None
            _write_json_atomic(_task_path(a.id), task)
        result = {"id": a.id, "status": a.status}
        if a.status == "done":
            result["unblocked"] = _find_unblocked_dependents(a.id)  # → javis_wakeup enqueue 대상
    print(json.dumps(result, ensure_ascii=False))
    return EXIT_OK


def cmd_ready(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    unresolved = _unresolved_blockers(task)
    print(json.dumps({"id": a.id, "ready": not unresolved, "unresolved": unresolved},
                     ensure_ascii=False))
    return EXIT_OK if not unresolved else EXIT_BLOCKED


def cmd_show(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    task["_lock_holder"] = _read_owner(a.id)
    print(json.dumps(task, ensure_ascii=False, indent=1))
    return EXIT_OK


def cmd_list(a):
    rows = _list_tasks()
    if a.open_only:
        rows = [t for t in rows if t.get("status") in OPEN_STATUSES]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="원자적 태스크 체크아웃 (P0-1)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("create")
    c.add_argument("title")
    c.add_argument("--id")
    c.add_argument("--status", default="todo", choices=STATUSES)
    c.add_argument("--why", help="이 일이 왜 존재하는가(goal 체인)")
    c.add_argument("--goal")
    c.add_argument("--blocked-by", nargs="*", dest="blocked_by")
    c.add_argument("--origin-kind", default="manual")
    c.add_argument("--origin-fingerprint")
    c.set_defaults(fn=cmd_create)

    c = sub.add_parser("checkout")
    c.add_argument("id")
    c.add_argument("--owner", required=True, help="워커/세션 식별자")
    c.add_argument("--pid", type=int, help="락 생존 판정에 쓸 장수 프로세스 pid(기본: 호출자)")
    c.set_defaults(fn=cmd_checkout)

    c = sub.add_parser("release")
    c.add_argument("id")
    c.add_argument("--owner", required=True)
    c.add_argument("--force", action="store_true")
    c.add_argument("--force-reason", dest="force_reason", default=None,
                   help="★G11: --force 필수 동반 — 강제 해제 사유(task JSON force_history 감사)")
    c.set_defaults(fn=cmd_release)

    c = sub.add_parser("set-status")
    c.add_argument("id")
    c.add_argument("status", choices=STATUSES)
    c.add_argument("--settled-override", dest="settled_override", default=None,
                   help="★G6: 완료 하한선 우회(사유 필수 기록) — owner 자신이 done을 선언하는 "
                        "등 안착 관측이 불가한 경우만")
    c.set_defaults(fn=cmd_set_status)

    c = sub.add_parser("ready")
    c.add_argument("id")
    c.set_defaults(fn=cmd_ready)

    c = sub.add_parser("show")
    c.add_argument("id")
    c.set_defaults(fn=cmd_show)

    c = sub.add_parser("list")
    c.add_argument("--open-only", action="store_true")
    c.set_defaults(fn=cmd_list)

    a = p.parse_args(argv)
    # ★G10: 모든 진입 id(본 id + create --blocked-by)를 경로 조합 전에 검증 — 단일 집행 지점.
    for tid in [getattr(a, "id", None)] + list(getattr(a, "blocked_by", None) or []):
        if tid is not None and not _ID_RE.match(tid):
            print("error: invalid task id %r — 허용 [A-Za-z0-9._-]{1,80} (G10 traversal 차단)"
                  % tid, file=sys.stderr)
            return EXIT_USAGE
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
