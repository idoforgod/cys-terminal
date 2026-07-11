#!/usr/bin/env python3
"""javis_wakeup.py — P0-2 wakeup 큐: 코얼레싱 + 멱등키 + zombie 가드 (클린룸 파일 포트)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-2):
- enqueue가 몰려도 같은 (target, task_key)의 pending은 1건으로 병합(coalesced_count 증가,
  reason은 최신으로 갱신, payload는 얕은 병합 — Paperclip mergeCoalescedContextSnapshot의 축소판).
- idempotency_key가 같은 요청은 중복 삽입하지 않는다(suppressed로 기록).
- zombie 가드: 배달(drain) 시 대상 생존을 확인하고, 죽은 대상에는 배달하지 않고 skipped 처리
  ("죽은 런에 병합하면 불멸화" 함정의 배달 측 방어).
- 원장은 append-only: 모든 사건(queued/coalesced/suppressed/delivered/skipped)을 queue.jsonl에 기록.

상태 저장: $JAVIS_ROOT/_round/wakeups/pending/<safe(target)>__<safe(task_key)>.json
원장:      $JAVIS_ROOT/_round/wakeups/queue.jsonl (append-only)
enqueue 직렬화: pending 파일별 read-modify-write를 .lock 디렉터리(mkdir 원자성)로 보호.

대상 생존 판정(zombie 가드) 우선순위:
  1) 테스트/강제 주입: JAVIS_WAKEUP_LIVENESS=alive|dead
  2) `cys list` 출력에 target 문자열 존재 여부 (cys 없으면 unknown→배달 보류 아닌 경고 배달)
배달은 기본 드라이런(cys send --queued 명령을 출력만). --deliver 시 실제 실행.

exit codes: 0 ok · 2 usage · 5 nothing-to-do
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # safe_path(-P) guard for sibling imports (same idiom as javis_replay.py)
import javis_scrub  # ★G2: 원장 기록 직전 비밀 마스킹(같은 폴더 형제 모듈 — 부재 시 즉시 실패=fail-closed)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
WK_DIR = os.path.join(ROOT, "_round", "wakeups")
PENDING_DIR = os.path.join(WK_DIR, "pending")
LEDGER = os.path.join(WK_DIR, "queue.jsonl")

EXIT_OK, EXIT_USAGE, EXIT_EMPTY = 0, 2, 5


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", s)[:80]


def _pending_path(target, task_key):
    return os.path.join(PENDING_DIR, f"{_safe(target)}__{_safe(task_key)}.json")


def _ledger_append(event):
    os.makedirs(WK_DIR, exist_ok=True)
    event["ts"] = _now()
    # ★G2(cokacdir 성찰 2026-07-04): 원장(queue.jsonl) 기록 직전 비밀 마스킹 — 값 단위 재귀.
    event = javis_scrub.scrub_obj(event)
    with open(LEDGER, "a", encoding="utf-8") as f:
        f.write(json.dumps(event, ensure_ascii=False) + "\n")


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


class _FileLock:
    """mkdir 원자성 기반 락. stale(30초+)은 rename으로 원자적 회수."""

    def __init__(self, path, timeout=5.0, stale_sec=30.0):
        self.path, self.timeout, self.stale_sec = path, timeout, stale_sec

    def __enter__(self):
        deadline = time.time() + self.timeout
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        while True:
            try:
                os.mkdir(self.path)
                return self
            except FileExistsError:
                try:
                    if time.time() - os.stat(self.path).st_mtime > self.stale_sec:
                        os.rename(self.path, f"{self.path}.stale.{time.time_ns()}")
                        continue
                except OSError:
                    pass
                if time.time() > deadline:
                    raise TimeoutError(f"lock timeout: {self.path}")
                time.sleep(0.02)

    def __exit__(self, *exc):
        try:
            os.rmdir(self.path)
        except OSError:
            pass


def _target_alive(target):
    """zombie 가드. 반환 'alive'|'dead'|'unknown'."""
    override = os.environ.get("JAVIS_WAKEUP_LIVENESS")
    if override in ("alive", "dead"):
        return override
    cys = shutil.which("cys")
    if not cys:
        return "unknown"
    try:
        out = subprocess.run([cys, "list"], capture_output=True, text=True, timeout=10).stdout
        # ★WP-8(P-ORCH-4): 부분일치 금지 — target을 식별자 경계로 정확일치. 'worker'가
        #   'worker-2'·'coworker' 안에 부분매칭돼 죽은/다른 노드를 alive로 오판하던 경로 차단.
        #   경계 = 앞뒤로 식별자문자([A-Za-z0-9._-])가 아닌 곳(공백·줄·구분자).
        pat = re.compile(r"(?<![A-Za-z0-9._-])%s(?![A-Za-z0-9._-])" % re.escape(target))
        return "alive" if pat.search(out) else "dead"
    except (subprocess.SubprocessError, OSError):
        return "unknown"


def _load_pending(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def cmd_enqueue(a):
    payload = json.loads(a.payload) if a.payload else {}
    path = _pending_path(a.to, a.task)
    lock = _FileLock(path + ".lock")
    with lock:
        cur = _load_pending(path)
        if cur:
            # 멱등키: 같은 키의 요청은 중복 없이 억제
            if a.idempotency_key and a.idempotency_key in cur.get("idempotency_keys", []):
                _ledger_append({"event": "suppressed", "target": a.to, "task_key": a.task,
                                "idempotency_key": a.idempotency_key, "wakeup_id": cur["id"]})
                print(json.dumps({"result": "suppressed", "id": cur["id"]}, ensure_ascii=False))
                return EXIT_OK
            # 코얼레싱: 최신 reason으로 갱신, payload 얕은 병합, count 증가
            cur["coalesced_count"] = cur.get("coalesced_count", 0) + 1
            cur["reason"] = a.reason
            cur["payload"] = {**cur.get("payload", {}), **payload}
            if a.idempotency_key:
                cur.setdefault("idempotency_keys", []).append(a.idempotency_key)
            cur["updated_at"] = _now()
            _write_json_atomic(path, cur)
            _ledger_append({"event": "coalesced", "target": a.to, "task_key": a.task,
                            "wakeup_id": cur["id"], "coalesced_count": cur["coalesced_count"]})
            print(json.dumps({"result": "coalesced", "id": cur["id"],
                              "coalesced_count": cur["coalesced_count"]}, ensure_ascii=False))
            return EXIT_OK
        rec = {
            "id": f"W-{uuid.uuid4().hex[:10]}",
            "target": a.to,
            "task_key": a.task,
            "reason": a.reason,
            "payload": payload,
            "idempotency_keys": [a.idempotency_key] if a.idempotency_key else [],
            "coalesced_count": 0,
            "queued_at": _now(),
            "updated_at": _now(),
        }
        _write_json_atomic(path, rec)
        _ledger_append({"event": "queued", "target": a.to, "task_key": a.task,
                        "wakeup_id": rec["id"], "reason": a.reason})
        print(json.dumps({"result": "queued", "id": rec["id"]}, ensure_ascii=False))
        return EXIT_OK


def _iter_pending():
    if not os.path.isdir(PENDING_DIR):
        return []
    out = []
    for name in sorted(os.listdir(PENDING_DIR)):
        if name.endswith(".json"):
            rec = _load_pending(os.path.join(PENDING_DIR, name))
            if rec:
                out.append((os.path.join(PENDING_DIR, name), rec))
    return out


def cmd_list(a):
    rows = [rec for _, rec in _iter_pending()]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def _build_send_message(rec):
    n = rec.get("coalesced_count", 0)
    merged = f" (병합 {n}건)" if n else ""
    return (f"[wakeup {rec['id']}] task={rec['task_key']} reason={rec['reason']}{merged} "
            f"payload={json.dumps(rec.get('payload', {}), ensure_ascii=False)}")


# ★G13(cokacdir 성찰 2026-07-04): 연속 배달실패 카운터 + fast-fail — hang/유령 대상 무한
#   재시도('idle 5분' 산문 규칙뿐이던 갭)를 결정론으로 차단. 임계 도달 시 pending 종결 +
#   카운터 리셋(이후 재큐잉은 깨끗한 재시도 — 자가 회복·수동 리셋 불요).
FAILCOUNT = os.path.join(WK_DIR, "failcount.json")


def _load_failcount():
    try:
        with open(FAILCOUNT, encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def _bump_failcount(target, reset=False):
    with _FileLock(FAILCOUNT + ".lock"):
        fc = _load_failcount()
        fc[target] = 0 if reset else fc.get(target, 0) + 1
        _write_json_atomic(FAILCOUNT, fc)
        return fc[target]


def cmd_drain(a):
    fastfail_max = int(os.environ.get("JAVIS_FASTFAIL_MAX", "3"))
    pending = _iter_pending()
    if a.target:
        pending = [(p, r) for p, r in pending if r["target"] == a.target]
    if not pending:
        print("nothing pending")
        return EXIT_EMPTY
    delivered = skipped = 0
    for path, rec in pending:
        alive = _target_alive(rec["target"])
        if alive == "dead":
            # zombie 가드: 죽은 대상에 배달/병합 유지 금지 → skipped로 종결(pending 제거)
            os.remove(path)
            _ledger_append({"event": "skipped", "target": rec["target"], "wakeup_id": rec["id"],
                            "why": "target_dead(zombie guard)"})
            print(f"skipped (대상 사망): {rec['id']} → {rec['target']}")
            skipped += 1
            continue
        msg = _build_send_message(rec)
        cmd = ["cys", "send", "--queued", "--to", rec["target"], msg]
        if a.deliver:
            if alive == "unknown":
                print(f"warn: {rec['target']} 생존 미확인 상태로 배달 시도", file=sys.stderr)
            try:
                subprocess.run(cmd, check=True, timeout=15)
                if _load_failcount().get(rec["target"]):
                    _bump_failcount(rec["target"], reset=True)  # 성공 = 연속실패 해소
            except (subprocess.SubprocessError, OSError, FileNotFoundError) as e:
                streak = _bump_failcount(rec["target"])
                if streak >= fastfail_max:
                    # ★G13 fast-fail: 임계 도달 — pending 종결(무한 재시도 차단)·카운터 리셋
                    os.remove(path)
                    _ledger_append({"event": "skipped", "target": rec["target"],
                                    "wakeup_id": rec["id"],
                                    "why": f"fast-fail(연속 {streak}회 배달실패)"})
                    _bump_failcount(rec["target"], reset=True)
                    print(f"skipped (fast-fail {streak}회): {rec['id']} → {rec['target']}",
                          file=sys.stderr)
                    skipped += 1
                    continue
                _ledger_append({"event": "deliver_failed", "target": rec["target"],
                                "wakeup_id": rec["id"], "why": str(e), "fail_streak": streak})
                print(f"deliver failed (pending 유지·연속 {streak}회): {rec['id']} — {e}",
                      file=sys.stderr)
                continue
        else:
            print("DRYRUN:", " ".join(cmd))
        os.remove(path)
        _ledger_append({"event": "delivered" if a.deliver else "delivered_dryrun",
                        "target": rec["target"], "wakeup_id": rec["id"]})
        delivered += 1
    print(json.dumps({"delivered": delivered, "skipped": skipped}, ensure_ascii=False))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="wakeup 큐 — 코얼레싱·멱등·zombie 가드 (P0-2)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("enqueue")
    c.add_argument("--to", required=True, help="대상(역할/노드 이름)")
    c.add_argument("--task", required=True, help="task_key(병합 단위)")
    c.add_argument("--reason", required=True)
    c.add_argument("--payload", help="JSON 문자열")
    c.add_argument("--idempotency-key", dest="idempotency_key")
    c.set_defaults(fn=cmd_enqueue)

    c = sub.add_parser("list")
    c.set_defaults(fn=cmd_list)

    c = sub.add_parser("drain")
    c.add_argument("--target", help="특정 대상만")
    c.add_argument("--deliver", action="store_true", help="실제 cys send 실행(기본 드라이런)")
    c.set_defaults(fn=cmd_drain)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
