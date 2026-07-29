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

exit codes: 0 ok · 2 usage · 3 not found · 4 blocked · 5 no evidence(W0-3·E1 artifact) · 8 duplicate origin · 9 conflict(409)

T0a(2026-07-13 · attention-p0 승인): W0-3 evidence 게이트·W2-1 전이표·W1-2 handoff 리마인더를
omc-w2 라인에서 이식 복원 — 문서화된 운영 계약(CLAUDE.md)과 라이브 코드의 분기 봉합.
버전CAS·lease renew·서킷 등 W2 심층 기능 재통합은 T0b(후속 티켓) 범위.
"""
import argparse
import contextlib
import fcntl
import getpass
import hashlib
import json
import os
import re
import socket
import sys
import time
import uuid

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   ._pth 는 표준 경로 계산을 우회해 **스크립트 폴더를 sys.path 에 넣지 않는다**
#   (2026-07-29 Windows 0.14.4 실측: `ModuleNotFoundError: No module named 'javis_scrub'`).
#   unix/mac 은 스크립트 폴더가 이미 sys.path[0] 이라 이 블록은 무동작(멱등).
#   ★append 인 이유는 MAJ#1 과 동일 — **발견이 목적이지 기존 항목의 precedence 를 강등하지 않는다**
#   (bin/ 을 stdlib 앞에 놓지 않아 미래의 이름충돌 shadowing 을 원천 차단).
#   선례(append 형태): javis_report.py:33-34.
#   (hooks/inject_gate.py:22 는 insert(0) + CYS_PACK_DIR 기반 경로 — 형태가 다르므로 선례 아님)
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")

STATUSES = ["backlog", "todo", "in_progress", "in_review", "done", "blocked", "cancelled"]
OPEN_STATUSES = ["backlog", "todo", "in_progress", "in_review", "blocked"]
TERMINAL_STATUSES = ["done", "cancelled"]
TERMINAL_FROM = ("in_progress", "in_review")  # W2-1 전이표: 터미널 전이 허용 출발 상태(T0a 이식)

EXIT_OK, EXIT_USAGE, EXIT_NOTFOUND, EXIT_BLOCKED, EXIT_DUP, EXIT_CONFLICT = 0, 2, 3, 4, 8, 9
EXIT_NO_EVIDENCE = 5  # W0-3: done 전이에 증거 부재(T0a 이식 · 기존 exit 체계와 무충돌)


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


# ─────────────────────────────────────────────────────────────────────────────
# E1(증거의 기계화 · 설계 DESIGN_LAZYCODEX_DISTILLATION §E1): evidence artifact 게이트.
#   done 전이 시 --evidence-artifact 경로를 기계 검사(실존·비어있지않음·신선도)한다.
#   신선도 앵커 = owner.json acquired_at (release 전 판독 — R2), 부재 시 created_at 폴백.
# ─────────────────────────────────────────────────────────────────────────────
def _iso_to_epoch(iso):
    """_now() 형식('%Y-%m-%dT%H:%M:%S%z')의 ISO 문자열 → epoch 초. 파싱 실패 시 None."""
    if not iso:
        return None
    from datetime import datetime
    try:
        return datetime.strptime(iso, "%Y-%m-%dT%H:%M:%S%z").timestamp()
    except (ValueError, TypeError):
        return None


def _sha256_8(path):
    """파일 내용 sha256의 앞 8 hex — 증거 지문(감사 대조용, 무결성 강제 아님)."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()[:8]


def _skip_audit_path():
    return os.path.join(ROOT, "_round", "evidence", "skip_audit.jsonl")


def _append_jsonl(path, rec):
    """append-only(O_APPEND) JSONL 1줄 기록 — 감사 원장(수정·삭제 금지 · R3)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    line = (json.dumps(rec, ensure_ascii=False) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def _check_evidence_artifacts(task_id, task, paths):
    """--evidence-artifact 경로들을 기계 검사(실존·비어있지않음·신선도).
    신선도 앵커 = owner.json acquired_at(release 전 판독 · R2), 부재 시 created_at 폴백.
    반환 (True, records) | (False, 오류문자열). records엔 anchor 출처를 명시(감사 가시화)."""
    owner = _read_owner(task_id)
    if owner and owner.get("acquired_at"):
        anchor_iso, anchor_src = owner["acquired_at"], "acquired_at"
    else:
        anchor_iso, anchor_src = task.get("created_at"), "created_at-fallback"
    anchor_ts = _iso_to_epoch(anchor_iso)
    records = []
    for p in paths:
        if not os.path.isfile(p):                               # ① 실존
            return False, "부재 경로: %s" % p
        st = os.stat(p)
        if st.st_size <= 0:                                     # ② 비어있지않음
            return False, "빈 파일(0바이트): %s" % p
        if anchor_ts is not None and st.st_mtime < anchor_ts:   # ③ 신선도
            return False, ("신선도 실패(mtime<%s): %s — 태스크 시작 전 파일은 증거 아님"
                           % (anchor_src, p))
        records.append({"path": p, "size": st.st_size,
                        "mtime": round(st.st_mtime, 3),
                        "sha256_8": _sha256_8(p), "anchor": anchor_src})
    return True, records


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


# ─────────────────────────────────────────────────────────────────────────────
# P1b(설계 §2.2·§2.1b · R1 리플레이 해소): probe 실행 영수증 대조 게이트. E1(산출물 파일
#   검증)과 상보적 — E1=검증 산출물 파일의 실존/신선도, 여기=probe 실행이 남긴 append-only
#   영수증의 최근성·exit0·대상/task 일치. 둘 다 통과해야 done. 영수증 스키마·경로는 actprobe
#   와 공유 계약이며, ts 파싱은 E1 의 _iso_to_epoch 를 재사용한다(중복 정의 금지).
# ─────────────────────────────────────────────────────────────────────────────
SCHEMA_VERSION = 1
_PROBE_TOKEN_RE = re.compile(r"probe:([a-z-]+)")
# relaxed 3종: target 이 surface ref·pid 라 부분일치가 무의미 → 대조 축을 task 로 교체(§2.1b).
#   이 3종은 현재 태스크로 바인딩된(--task) 영수증만 인정(무-task·타-task = 리플레이 거부).
_RELAXED_TARGET_PROBES = {"submit", "ctx-compare", "kill-preflight"}


def _rec_task(rec):
    """영수증 선택 필드 task(§2.1b) — 문자열이고 비어있지 않으면 반환, 아니면 None."""
    v = rec.get("task")
    return v if isinstance(v, str) and v != "" else None


def _probe_runs_path():
    """영수증 경로: env CYS_PROBE_RUNS > <pack>/state/probe_runs.jsonl (actprobe 와 동일)."""
    env = os.environ.get("CYS_PROBE_RUNS")
    if env:
        return env
    pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
    return os.path.join(pack, "state", "probe_runs.jsonl")


def _load_receipts(path):
    """probe_runs.jsonl 파싱 → (rows, file_exists). 깨진 행은 fail-soft로 건너뛴다."""
    if not os.path.exists(path):
        return [], False
    rows = []
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except ValueError:
                    continue  # 깨진 행 fail-soft(§2.2-3)
                if isinstance(rec, dict):
                    rows.append(rec)
    except OSError:
        return [], False
    return rows, True


def _verify_probe_receipts(task, ev_text, tid):
    """evidence 의 probe:<name> 토큰을 영수증과 기계 대조 → (ok, reason).

    토큰이 없으면 (True, None) — 하위호환. 토큰이 있으면 각 probe:
      · 영수증 파일 부재 = probe 미실행 → 거부(§2.2-3)
      · (probe명 ∧ exit0 ∧ 최근 K분) 영수증 최소 1건 필요(없으면 태만/스테일 거부)
      · relaxed 3종(§2.1b): task 필드가 현재 태스크(tid)와 일치하는 영수증만 인정 —
        무-task·타-task 영수증은 리플레이로 거부(target 대조 무의미 → 축을 task 로 교체)
      · artifact·verdict-match: target 이 evidence/goal/why 에 부분일치 + (task 필드가 있으면
        현재 태스크와 일치 — 타 태스크 리플레이 차단, 무-task 는 target 만으로 인정)

    ★위협모델 한계(설계 §0 v3): 동일 OS 사용자의 가짜 영수증 수기 작성은 이 게이트가 차단
    못 한다 — 감사층(원장·mine) 탐지 대상이며 출처증명은 cysd attestation 로드맵. 본 게이트는
    실측 지배 실패 모드(태만·스테일·대상/task 불일치)를 결정론으로 닫는다.
    """
    tokens = _PROBE_TOKEN_RE.findall(ev_text)
    if not tokens:
        return True, None
    path = _probe_runs_path()
    rows, exists = _load_receipts(path)
    if not exists:
        return False, ("probe 토큰 %s 영수증 미대조: 영수증 파일 부재(%s) — probe 미실행 증거"
                       % (sorted(set(tokens)), path))
    window_min = float(os.environ.get("CYS_PROBE_RECEIPT_WINDOW_MIN", "240"))
    now = time.time()
    # target 부분일치 대상 텍스트(대소문자 무시): evidence + 태스크 goal/why.
    haystack = " ".join([ev_text, task.get("goal") or "", task.get("why") or ""]).lower()
    for name in dict.fromkeys(tokens):  # 순서 보존 dedup
        recent0 = []
        for rec in rows:
            if rec.get("probe") != name or rec.get("exit") != 0:
                continue
            ep = _iso_to_epoch(rec.get("ts"))  # E1 헬퍼 재사용(중복 정의 금지)
            if ep is None or (now - ep) > window_min * 60:
                continue
            recent0.append(rec)
        if not recent0:
            return False, ("probe:%s 미대조 — exit0·최근 %d분 이내 영수증 없음"
                           % (name, int(window_min)))
        if name in _RELAXED_TARGET_PROBES:
            # 대조 축=task(§2.1b): 현재 태스크로 바인딩된 영수증만 인정.
            if any(_rec_task(r) == tid for r in recent0):
                continue
            if any(_rec_task(r) is not None for r in recent0):
                return False, ("probe:%s task 바인딩 불일치 — 타 태스크 영수증(리플레이 의심)"
                               % name)
            return False, ("probe:%s 무-task 영수증만 존재 — relaxed probe 는 --task 동반 "
                           "영수증만 인정(리플레이 차단)" % name)
        # artifact·verdict-match: target 부분일치 필요.
        tmatch = [r for r in recent0
                  if (r.get("target") or "").strip()
                  and (r.get("target") or "").strip().lower() in haystack]
        if not tmatch:
            return False, ("probe:%s 영수증은 있으나 target 불일치 — 영수증 target 이 "
                           "evidence/goal/why 텍스트에 없음" % name)
        # task 필드가 있으면 현재 태스크와 일치해야 함(리플레이 차단). 무-task 는 target 만으로 인정.
        if not any(_rec_task(r) is None or _rec_task(r) == tid for r in tmatch):
            return False, ("probe:%s target 은 맞으나 task 바인딩 불일치 — 타 태스크 영수증"
                           "(리플레이 의심)" % name)
    return True, None


def _append_receipt_line(path, rec):
    """actprobe._append_receipt 규약 복제(append-only·끝개행 보정·flock 단일 writer).
    E1 _append_jsonl 은 별도 파일(skip_audit.jsonl)용이므로 여기선 actprobe 와 같은 flock
    규약을 쓴다(공유 계약 정합) — 코드 결합 없이 규약만 소형 복제."""
    line = json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n"
    d = os.path.dirname(path) or "."
    os.makedirs(d, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        size = os.fstat(fd).st_size
        if size:
            os.lseek(fd, size - 1, os.SEEK_SET)
            if os.read(fd, 1) != b"\n":
                os.write(fd, b"\n")
        os.write(fd, line.encode("utf-8"))
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def _resolve_caller():
    """caller: env CYS_ACTPROBE_CALLER > cys identify surface_ref > OS 사용자."""
    env = os.environ.get("CYS_ACTPROBE_CALLER")
    if env:
        return env
    try:
        import subprocess  # E1 self-test 관례(지역 import) — 모듈 상단 결합 회피
        out = subprocess.run([os.environ.get("CYS_BIN", "cys"), "identify"],
                             capture_output=True, text=True, timeout=5)
        if out.returncode == 0:
            ref = ((json.loads(out.stdout) or {}).get("caller") or {}).get("surface_ref")
            if isinstance(ref, str) and ref:
                return ref
    except Exception:
        pass
    try:
        return getpass.getuser()
    except Exception:
        return "unknown"


def _append_audit(task_id, audit_kind):
    """우회(skip-reason·gate-off) 가시화 감사 행 → probe_runs.jsonl(§2.2-2). best-effort·비차단.
    E1 skip_audit.jsonl 과 상보적 — 이쪽은 probe 원장에 남겨 §3 mine 이 반복 우회를 포착한다."""
    rec = {
        "schema_version": SCHEMA_VERSION,
        "ts": _now(),
        "probe": "task-audit",
        "target": task_id,
        "exit": 0,
        "argv_digest": hashlib.sha256(
            "\x00".join(sys.argv[1:]).encode("utf-8")).hexdigest()[:16],
        "caller": _resolve_caller(),
        "audit": audit_kind,
    }
    try:
        _append_receipt_line(_probe_runs_path(), rec)
    except OSError as e:
        print("task: WARN audit receipt append failed: %s" % e, file=sys.stderr)


def cmd_set_status(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    # ★G6: done은 완료 하한선 통과 후에만 — 오종결이 의존자 unblock(:308)을 조기 발화하는
    #   경로를 닫는다. owner 없는(체크아웃 이력 없는) 태스크는 안착 대상이 없어 게이트 생략.
    #   ★WP-8(P-ORCH): 최대 settle sleep 은 wlock '밖'에서 — 락 장기점유·타임아웃을 피한다.
    # W0-3 evidence 게이트(T0a 이식 — omc-w2 라인): done 전이는 --evidence 또는 --skip-reason 필수.
    # 인자만 보는 즉시 검사라 settle 프로브(sleep 동반)보다 먼저 — cheap-first(성찰 D3).
    # 기본 strict, CYS_TASK_EVIDENCE_GATE=warn|off 안전밸브.
    ev_text = (getattr(a, "evidence", None) or "").strip()
    skip_text = (getattr(a, "skip_reason", None) or "").strip()
    art_paths = list(getattr(a, "evidence_artifact", None) or [])
    artifact_records = None  # done 게이트 통과 시 채워져 wlock 안에서 task.evidence.artifacts로 저장
    skip_audit_pending = False  # skip 감사는 실제 done 진행(전이·settle 게이트 통과) 후에만 기록
    if a.status == "done":
        # ── E1 evidence-artifact 게이트(증거의 기계화 · 설계 §E1) — cheap-first(settle sleep 전) ──
        #   신선도 앵커 = owner.json acquired_at을 release(:하단) '전'에 판독한다(R2).
        #   기계 검사 실패(실존·비어있지않음·신선도)는 우회 불가 — strict 거부/warn 경고.
        art_mode = os.environ.get("CYS_TASK_EVIDENCE_ARTIFACT_GATE", "strict")
        art_valid = False
        if art_mode != "off" and art_paths:
            ok, res = _check_evidence_artifacts(a.id, task, art_paths)
            if ok:
                artifact_records = res
                art_valid = True
            else:
                if art_mode == "strict":
                    print("evidence-artifact required(5): %s "
                          "(기계 검사 3종 — 실존·비어있지않음·신선도 mtime≥앵커)" % res, file=sys.stderr)
                    return EXIT_NO_EVIDENCE
                print("evidence-artifact warn: %s" % res, file=sys.stderr)
        # 기존 텍스트 evidence 게이트(W0-3) — 유효 artifact는 이를 충족으로 인정(증거 상위호환)
        mode = os.environ.get("CYS_TASK_EVIDENCE_GATE", "strict")
        has_evidence = len(ev_text) >= 8 or bool(skip_text) or art_valid  # 품질 하한: evidence 최소 8자
        if mode != "off" and not has_evidence:
            if mode == "strict":
                print('evidence required(5): done 전이는 --evidence·--evidence-artifact 또는 --skip-reason 필수 '
                      '(예: --evidence "pytest → 31/31 PASS" · evidence 최소 8자)', file=sys.stderr)
                return EXIT_NO_EVIDENCE
            print("evidence warn: done 전이에 검증 증거 없음 — --evidence·--evidence-artifact 또는 "
                  "--skip-reason 권장", file=sys.stderr)
        # E1 artifact 게이트(R1 day-1 strict): 유효 artifact ≥1 또는 --skip-reason
        if art_mode != "off" and not art_valid and not skip_text:
            if art_mode == "strict":
                print("evidence-artifact required(5): done 전이는 유효 --evidence-artifact ≥1 또는 "
                      "--skip-reason 필수 (증거의 기계화 — 텍스트 단정만으로는 불충분)", file=sys.stderr)
                return EXIT_NO_EVIDENCE
            print("evidence-artifact warn: 유효 --evidence-artifact 없음 — "
                  "--evidence-artifact 또는 --skip-reason 권장", file=sys.stderr)
        # ── P1b probe 영수증 대조(설계 §2.2) — E1 게이트 '뒤'·settle sleep '전'(cheap-first). evidence
        #    의 probe:<name> 토큰을 actprobe 영수증과 기계 대조(최근성·exit0·target/task 바인딩).
        #    E1(산출물 파일)과 상보적이며 둘 다 통과해야 done. W0-3 텍스트 게이트와 동일 안전밸브
        #    (mode=CYS_TASK_EVIDENCE_GATE) 공유 — strict 거부(5)/warn 경고/off 생략.
        if mode != "off":
            pok, pwhy = _verify_probe_receipts(task, ev_text, a.id)
            if not pok:
                if mode == "strict":
                    print("probe receipt mismatch(5): " + pwhy, file=sys.stderr)
                    return EXIT_NO_EVIDENCE
                print("probe receipt warn: " + pwhy, file=sys.stderr)
        # skip 감사(R3): skip-reason이 done 통과의 부담 근거(유효 artifact 부재)이면 예약 — art_mode와
        #   무관하다(★어태커 결함1: off 밸브에서도 --skip-reason은 텍스트 게이트 통과 근거로 쓰이므로
        #   기록해야 원장이 완전하다). 실제 기록은 전이·settle 게이트를 통과해 done이 확정된 뒤
        #   (wlock 안)에만 — 조기 기록은 거부된 재진입에도 감사가 오염된다.
        skip_audit_pending = (bool(skip_text) and not art_valid)
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
        # W2-1 전이표(T0a 이식): 터미널 전이는 in_progress|in_review에서만. 락 안 재독 상태 기준 —
        # 어떤 변이보다 먼저 검사(거부 시 무변이 반환). CYS_TASK_TRANSITION_GATE=warn|off 안전밸브.
        if a.status in TERMINAL_STATUSES and task.get("status") not in TERMINAL_FROM:
            tmode = os.environ.get("CYS_TASK_TRANSITION_GATE", "strict")
            if tmode == "strict":
                print(f"transition denied(2): {task.get('status')}→{a.status} — "
                      f"터미널 전이는 {'|'.join(TERMINAL_FROM)}에서만", file=sys.stderr)
                return EXIT_USAGE
            if tmode != "off":
                print(f"transition warn: {task.get('status')}→{a.status} — "
                      "터미널 전이는 in_progress|in_review에서 권장(전이표 W2-1)", file=sys.stderr)
        if skip_audit_pending:  # E1 R3: 전이 게이트 통과·done 확정 시점에만 append(재진입 오염 차단)
            _append_jsonl(_skip_audit_path(), {"ts": _now(), "task": a.id, "reason": skip_text})
        if ev_text or skip_text:
            task["evidence"] = {"type": "evidence" if ev_text else "skip",
                                "text": ev_text or skip_text, "at": _now()}
        if artifact_records:  # E1: 비파괴 확장 — 기존 evidence(text/skip)와 공존
            task.setdefault("evidence", {})["artifacts"] = artifact_records
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
        if task.get("evidence"):
            result["evidence"] = task["evidence"]
        if a.status == "done":
            result["unblocked"] = _find_unblocked_dependents(a.id)  # → javis_wakeup enqueue 대상
            # W1-2 리마인더 + T2 전사통계 배선 — 비차단(exit 불변·연성 의존: 도구 부재 시 그냥 생략)
            print("handoff: _round/handoffs/%s-done.md 5필드 기록 권장(HANDOFF_CONTRACT)" % a.id,
                  file=sys.stderr)
            print("evidence에 전사 통계 첨부 권장: javis_transcript_stats.py --latest --oneline "
                  "(도구 부재 시 생략 — 비차단)", file=sys.stderr)
    # P1b 우회 가시화(§2.2-2): done 확정 후에만 probe 원장에 감사 행(skip-reason·gate-off) append.
    #   E1 skip_audit.jsonl(위)과 상보적 — 비차단·wlock 밖. mode 는 done 분기에서 정의된다.
    if a.status == "done":
        if skip_text:
            _append_audit(a.id, "skip-reason")
        if mode != "strict":
            _append_audit(a.id, "gate-off")
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


def cmd_self_test(args):
    """E1 evidence-artifact 게이트 밀폐 자기검증 — subprocess로 실제 CLI(exit code)를 구동한다.
    ★결함11호: 모든 호출은 tmpdir + JAVIS_ROOT 주입으로 밀폐 — 실장부(_round/tasks/) 접촉 금지.
    커버리지(설계 §E1 시험 목록): 부정 4·긍정 1·폴백 1·skip 감사 1·재진입·warn/off 경계·하위호환."""
    import subprocess
    import tempfile

    self_path = os.path.abspath(__file__)

    def run(root, argv, env_extra=None):
        env = dict(os.environ)
        env["JAVIS_ROOT"] = root
        env.setdefault("JAVIS_TASK_LIVENESS", "alive")  # 체크아웃 락 생존 결정론
        # P1b 밀폐: probe 원장 감사 행(skip-reason·gate-off)이 라이브 pack/state 로 새지 않도록
        #   영수증 경로·caller 를 tmpdir 로 고정(라이브 미접촉 · cys identify 서브프로세스 회피).
        env.setdefault("CYS_PROBE_RUNS", os.path.join(root, "probe_runs.jsonl"))
        env.setdefault("CYS_ACTPROBE_CALLER", "self-test")
        if env_extra:
            env.update(env_extra)
        r = subprocess.run([sys.executable, self_path] + argv,
                           capture_output=True, text=True, env=env)
        return r.returncode, r.stdout, r.stderr

    def read_task(root, tid):
        with open(os.path.join(root, "_round", "tasks", tid + ".json"), encoding="utf-8") as f:
            return json.load(f)

    def mkfile(root, name, content="ok evidence\n"):
        p = os.path.join(root, name)
        with open(p, "w", encoding="utf-8") as f:
            f.write(content)
        return p

    def setup_checked_out(root, tid):
        """create + checkout → owner.json(acquired_at) 생성·status in_progress."""
        rc, _, e = run(root, ["create", tid, "--id", tid])
        assert rc == 0, "create 실패(%s): %s" % (tid, e)
        rc, _, e = run(root, ["checkout", tid, "--owner", "w1"])
        assert rc == 0, "checkout 실패(%s): %s" % (tid, e)

    OV = ["--settled-override", "self-test"]  # 체크아웃된 태스크의 완료 하한선(settle) 우회
    try:
        with tempfile.TemporaryDirectory(prefix="javis-task-e1-") as root:
            # ── 부정 1: 부재 경로 → exit 5 ──
            setup_checked_out(root, "Tneg-missing")
            rc, _, _ = run(root, ["set-status", "Tneg-missing", "done",
                                  "--evidence-artifact", os.path.join(root, "nope.txt")] + OV)
            assert rc == EXIT_NO_EVIDENCE, "부재 경로가 5를 안 냄: %s" % rc

            # ── 부정 2: 빈 파일 → exit 5 ──
            setup_checked_out(root, "Tneg-empty")
            empty = mkfile(root, "empty.txt", "")
            rc, _, _ = run(root, ["set-status", "Tneg-empty", "done",
                                  "--evidence-artifact", empty] + OV)
            assert rc == EXIT_NO_EVIDENCE, "빈 파일이 5를 안 냄: %s" % rc

            # ── 부정 3: acquired_at 이전 mtime(신선도 실패) → exit 5 ──
            setup_checked_out(root, "Tneg-stale")
            stale = mkfile(root, "stale.txt", "old\n")
            os.utime(stale, (time.time() - 3600, time.time() - 3600))  # 체크아웃보다 1h 전
            rc, _, err = run(root, ["set-status", "Tneg-stale", "done",
                                    "--evidence-artifact", stale] + OV)
            assert rc == EXIT_NO_EVIDENCE, "신선도 실패가 5를 안 냄: %s (%s)" % (rc, err)
            assert "신선도" in err, "신선도 사유 메시지 누락: %s" % err

            # ── 부정 4: strict에서 artifact·skip 모두 부재(텍스트 evidence만) → exit 5 ──
            setup_checked_out(root, "Tneg-noart")
            rc, _, _ = run(root, ["set-status", "Tneg-noart", "done",
                                  "--evidence", "pytest 31/31 PASS"] + OV)
            assert rc == EXIT_NO_EVIDENCE, "텍스트만(artifact/skip 부재)이 5를 안 냄: %s" % rc

            # ── 긍정: 유효 artifact → exit 0 + artifacts 기록(sha·anchor) ──
            setup_checked_out(root, "Tpos")
            good = mkfile(root, "good.txt", "verified ok\n")
            rc, out, e = run(root, ["set-status", "Tpos", "done",
                                    "--evidence-artifact", good] + OV)
            assert rc == EXIT_OK, "유효 artifact가 0을 안 냄: %s (%s)" % (rc, e)
            arts = read_task(root, "Tpos")["evidence"]["artifacts"]
            assert len(arts) == 1 and arts[0]["size"] > 0, "artifacts 기록 누락: %s" % arts
            assert len(arts[0]["sha256_8"]) == 8, "sha256_8 형식 오류: %s" % arts[0]
            assert arts[0]["anchor"] == "acquired_at", "체크아웃 태스크 anchor≠acquired_at: %s" % arts[0]

            # ── 폴백: owner 없는(체크아웃 없는) 태스크 + created_at 이후 파일 → 0 + anchor 폴백 ──
            rc, _, e = run(root, ["create", "Tfb", "--id", "Tfb", "--status", "in_progress"])
            assert rc == 0, "폴백 create 실패: %s" % e
            fb = mkfile(root, "fb.txt", "fallback ok\n")
            rc, out, e = run(root, ["set-status", "Tfb", "done", "--evidence-artifact", fb])
            assert rc == EXIT_OK, "폴백 done이 0을 안 냄: %s (%s)" % (rc, e)
            fbart = read_task(root, "Tfb")["evidence"]["artifacts"][0]
            assert fbart["anchor"] == "created_at-fallback", "폴백 anchor 오기록: %s" % fbart

            # ── skip 감사: --skip-reason → 0 + skip_audit.jsonl 1줄 append ──
            rc, _, e = run(root, ["create", "Tskip", "--id", "Tskip", "--status", "in_progress"])
            assert rc == 0, "skip create 실패: %s" % e
            rc, _, e = run(root, ["set-status", "Tskip", "done",
                                  "--skip-reason", "CI에서 재현 불가"])
            assert rc == EXIT_OK, "skip done이 0을 안 냄: %s (%s)" % (rc, e)
            audit = os.path.join(root, "_round", "evidence", "skip_audit.jsonl")
            with open(audit, encoding="utf-8") as f:
                lines = [ln for ln in f.read().splitlines() if ln.strip()]
            assert len(lines) == 1, "skip_audit 줄 수≠1: %s" % lines
            rec = json.loads(lines[0])
            assert rec["task"] == "Tskip" and rec["reason"] == "CI에서 재현 불가", "skip 감사 내용 오류: %s" % rec

            # ── 재진입: done 성공 후 재시도는 터미널 전이 게이트로 안전 거부(exit 2)·기록 무중복 ──
            rc, _, _ = run(root, ["set-status", "Tskip", "done",
                                  "--skip-reason", "재시도"])
            assert rc == EXIT_USAGE, "터미널 재진입이 2를 안 냄: %s" % rc
            with open(audit, encoding="utf-8") as f:
                lines2 = [ln for ln in f.read().splitlines() if ln.strip()]
            assert len(lines2) == 1, "재진입이 skip 감사에 중복 기록: %s" % lines2

            # ── 경계 warn: 증거 전무라도 warn 모드는 통과(exit 0) + 경고 출력 ──
            rc, _, e = run(root, ["create", "Twarn", "--id", "Twarn", "--status", "in_progress"])
            assert rc == 0
            rc, _, err = run(root, ["set-status", "Twarn", "done"],
                             {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "warn",
                              "CYS_TASK_EVIDENCE_GATE": "warn"})
            assert rc == EXIT_OK, "warn 모드가 0을 안 냄: %s (%s)" % (rc, err)
            assert "warn" in err, "warn 경고 메시지 누락: %s" % err

            # ── 경계 off: 게이트 완전 생략(exit 0)·artifacts 미기록 ──
            rc, _, e = run(root, ["create", "Toff", "--id", "Toff", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Toff", "done"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off",
                            "CYS_TASK_EVIDENCE_GATE": "off"})
            assert rc == EXIT_OK, "off 모드가 0을 안 냄: %s (%s)" % (rc, e)
            assert "artifacts" not in read_task(root, "Toff").get("evidence", {}), "off인데 artifacts 기록됨"

            # ── off 밸브 skip 감사(어태커 결함1): artifact 게이트 off라도 --skip-reason이 done 통과
            #    근거이면 skip_audit.jsonl에 기록돼 원장이 완전해야 한다 ──
            rc, _, e = run(root, ["create", "Toffskip", "--id", "Toffskip", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Toffskip", "done", "--skip-reason", "off 밸브 skip"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off"})
            assert rc == EXIT_OK, "off+skip done이 0을 안 냄: %s (%s)" % (rc, e)
            with open(audit, encoding="utf-8") as f:
                offlines = [json.loads(ln) for ln in f.read().splitlines() if ln.strip()]
            assert any(r["task"] == "Toffskip" and r["reason"] == "off 밸브 skip" for r in offlines), \
                "off 밸브에서 skip 감사가 누락됨(원장 불완전): %s" % offlines

            # ── 하위호환 A: --evidence 텍스트만(artifact 게이트 off) → 무파손(evidence.text 보존) ──
            rc, _, e = run(root, ["create", "Tbc1", "--id", "Tbc1", "--status", "in_progress"])
            assert rc == 0
            rc, _, e = run(root, ["set-status", "Tbc1", "done", "--evidence", "구형 텍스트 증거 12345"],
                           {"CYS_TASK_EVIDENCE_ARTIFACT_GATE": "off"})
            assert rc == EXIT_OK, "하위호환(텍스트만) 무파손 실패: %s (%s)" % (rc, e)
            ev = read_task(root, "Tbc1")["evidence"]
            assert ev["text"] == "구형 텍스트 증거 12345" and "artifacts" not in ev, "구형 evidence 필드 손상: %s" % ev

            # ── 하위호환 B: --evidence 텍스트 + --evidence-artifact 공존(strict) → 둘 다 기록 ──
            setup_checked_out(root, "Tbc2")
            g2 = mkfile(root, "bc2.txt", "both ok\n")
            rc, _, e = run(root, ["set-status", "Tbc2", "done",
                                  "--evidence", "pytest 12/12 PASS",
                                  "--evidence-artifact", g2] + OV)
            assert rc == EXIT_OK, "텍스트+artifact 공존 실패: %s (%s)" % (rc, e)
            ev2 = read_task(root, "Tbc2")["evidence"]
            assert ev2.get("text") == "pytest 12/12 PASS" and len(ev2.get("artifacts", [])) == 1, \
                "텍스트+artifact 비파괴 공존 실패: %s" % ev2
    except AssertionError as ex:
        print("javis_task self-test FAIL: %s" % ex, file=sys.stderr)
        return 1
    print("javis_task self-test OK (E1 evidence-artifact 게이트 — 부정4·긍정·폴백·skip감사·"
          "재진입·warn/off 경계·off밸브 skip감사·하위호환 A/B · 밀폐 tmpdir+JAVIS_ROOT)")
    return EXIT_OK


def main(argv=None):
    # preflight 호환: `--self-test`는 subcommand 없이도 동작해야 한다(orchestra 관례 준용·가로채기).
    if "--self-test" in (sys.argv if argv is None else argv):
        return cmd_self_test(None)
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
    c.add_argument("--evidence", default=None,
                   help="W0-3: 검증 증거 '<검증명령 → 결과>' (최소 8자) — done 전이 필수")
    c.add_argument("--evidence-artifact", dest="evidence_artifact", action="append",
                   metavar="PATH",
                   help="E1: 검증 증거 파일 경로(반복 가능) — 기계 검사(실존·비어있지않음·신선도) 후 "
                        "task.evidence.artifacts 기록. done 전이 필수(strict) 또는 --skip-reason")
    c.add_argument("--skip-reason", dest="skip_reason", default=None,
                   help="W0-3·E1: 검증 불가 사유 — evidence/artifact 대체(사용 시 skip_audit.jsonl 감사 기록)")
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

    sub.add_parser("self-test", help="E1 게이트 밀폐 자기검증(tmpdir + JAVIS_ROOT 주입)"
                   ).set_defaults(fn=cmd_self_test)

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
