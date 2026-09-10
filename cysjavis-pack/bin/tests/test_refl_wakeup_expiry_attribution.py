#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_wakeup_expiry_attribution.py — 성찰 R4 N8·N9: 만료 사건의 귀속과 at-least-once 무장.

무엇을 막는가
  ① N8 [major] wakeup 식별자 GC 가 **만료 사건 귀속**을 끊었다.
     사슬(실측 시간축 그대로): 5분 보고 주기 · seen TTL 30분 · 데몬 큐 TTL 6시간. 승인 대기
     6시간 동안 30분마다 `seen_gc` 가 레코드를 지우고 재선점이 새 wakeup ID 를 만든다 → 6시간
     뒤 도착한 **최초 항목**의 `queue.expired` 가 현재 레코드의 ID 와 달라 아무것도 종결시키지
     못한다 → 최초 사건이 영영 열린 채로 남아 같은 사건이 TTL 마다 반복 재enqueue 된다.
     수정: **재시도 억제 TTL** 과 **미완료 사건의 ID 보존**을 분리한다(`pending` 목록 · 보존
     8시간 = 데몬 큐 TTL 6시간 + 여유). 과거 ID 의 종결도 원 사건에 연결된다.
  ② N9 [major] 만료가 at-least-once 무장을 **배달과 동일하게** 풀었다(`edge_fire`).
     귀결 셋: ⓐ 재통보 간격이 seen TTL 1800s → 쿨다운 7200s 로 4배 · ⓑ `cooldown` 기본값 0 인
     트리거는 **영구 침묵**(재무장은 '이번 주기에 없는 키' 에만 일어나는데 조건이 지속되면 키는
     계속 있다) · ⓒ 만료 갈래가 badge·evt·ledger 어느 것도 남기지 않아 "critical 이 아무도 읽지
     않은 채 폐기됐다" 를 함대가 관측할 수 없다(배달 계수는 그대로라 대장만 보면 정상이다).
     수정: 만료는 **종결로 기록하되 무장은 풀지 않는다**(억제 상한은 seen TTL 이 이미 준다) ·
     `expired_undelivered` 계수와 전용 배지를 낸다.

밀폐: `FakeRunner`(외부 명령 0 · 데몬 0)와 임시 state_dir 만 쓴다 — 기존 `test_report_gate.py` 의
      대역 하네스를 그대로 재사용한다(같은 계약을 두 벌로 적지 않는다).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-WAKEUP-EXPIRY-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_wakeup_expiry_attribution.py
"""
import os
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
sys.path.insert(0, SELF)

import javis_report_gate as G                              # noqa: E402
from test_report_gate import FakeRunner, gate, report, badges, ledger_entries  # noqa: E402

fails = []
T0 = 1_000_000.0
QUEUE_TTL = 6 * 3600.0        # 데몬 기본 큐 TTL(설계 · `queue.expired` 가 이 뒤에 온다)


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def load_rec(t, key):
    import json
    with open(G.seen_path(t, key), encoding="utf-8") as f:
        return json.load(f)


# ── ① N8: 6시간 큐 TTL 을 넘긴 최초 항목의 만료가 원 사건에 귀속된다 ────────────────────
with tempfile.TemporaryDirectory() as t:
    key = G.seen_key("stall_confirmed", "s:worker", G.SEV_CRIT)
    G.seen_claim(t, key, G.SEV_CRIT, T0)
    G.seen_mark(t, key, T0, state=G.SEEN_STATE_INFLIGHT, wakeup_id="W-first")
    G.seen_pending_add(t, key, "W-first", T0)

    # 승인 대기 6시간 — 30분(seen TTL)마다 GC + 재선점이 일어난다(실측 시간축).
    cycles = int(QUEUE_TTL // G.SEEN_TTL_SECS)            # 12회
    now = T0
    last_wid = "W-first"
    for i in range(1, cycles + 1):
        now = T0 + i * (G.SEEN_TTL_SECS + 1)
        G.seen_gc(t, now, G.SEEN_TTL_SECS)
        claimed, _r = G.seen_claim(t, key, G.SEV_CRIT, now)
        if not claimed:
            check("1x %d회차 재선점이 열리지 않았다(억제 TTL 이 GC 로 늘어났다)" % i, False)
            break
        last_wid = "W-%02d" % i
        G.seen_mark(t, key, now, state=G.SEEN_STATE_INFLIGHT, wakeup_id=last_wid)
        G.seen_pending_add(t, key, last_wid, now)
    rec = load_rec(t, key)
    check("1a 레코드가 GC 를 넘어 살아남는다(미종결 id 가 있으므로)", os.path.exists(G.seen_path(t, key)))
    ids = [it["id"] for it in (rec.get("pending") or [])]
    check("1b ★최초 wakeup id 가 6시간 뒤에도 기억된다", "W-first" in ids, repr(ids))
    # ★계측 타당성: 한 칸짜리 `wakeup_id` 로는 이 귀속이 **불가능**했다는 증거.
    check("1c 계측 타당성 — 한 칸 `wakeup_id` 는 이미 최신 id 로 덮여 있다",
          rec.get("wakeup_id") == last_wid and rec.get("wakeup_id") != "W-first",
          repr(rec.get("wakeup_id")))

    r = FakeRunner(events=[{"name": "queue.expired", "payload": {"entry_ids": ["W-first"]}}])
    g = gate(t, r)
    g._poll_once()
    counters = {}
    g._reconcile_inflight(counters, now + 60)
    rec = load_rec(t, key)
    check("1d ★최초 항목의 만료가 원 사건을 종결시킨다", rec["state"] == G.SEEN_STATE_EXPIRED,
          repr(rec["state"]))
    ids = [it["id"] for it in (rec.get("pending") or [])]
    check("1e 종결된 id 는 미종결 목록에서 빠진다", "W-first" not in ids, repr(ids))
    check("1f 아직 종결되지 않은 id 들은 남는다", last_wid in ids, repr(ids))
    check("1g 만료가 계수로 남는다", counters.get("expired_undelivered") == 1,
          repr(counters.get("expired_undelivered")))
    # 음성 대조: 남의 id 만 만료되면 내 사건은 열린 채다(귀속이 실제로 id 로 걸린다).
    with tempfile.TemporaryDirectory() as t2:
        k2 = G.seen_key("stall_confirmed", "s:other", G.SEV_CRIT)
        G.seen_claim(t2, k2, G.SEV_CRIT, T0)
        G.seen_mark(t2, k2, T0, state=G.SEEN_STATE_INFLIGHT, wakeup_id="W-mine")
        G.seen_pending_add(t2, k2, "W-mine", T0)
        r2 = FakeRunner(events=[{"name": "queue.expired", "payload": {"entry_ids": ["W-nope"]}}])
        g2 = gate(t2, r2)
        g2._poll_once()
        c2 = {}
        g2._reconcile_inflight(c2, T0 + 60)
        check("1h 음성 대조 — 남의 만료는 내 사건을 종결시키지 않는다",
              load_rec(t2, k2)["state"] == G.SEEN_STATE_INFLIGHT and not c2.get("expired_undelivered"))

# ── ② N8: 미종결이 없으면 GC 는 종전대로 지운다(레코드가 영원히 쌓이지 않는다) ──────────
with tempfile.TemporaryDirectory() as t:
    key = G.seen_key("stall_confirmed", "s:done", G.SEV_CRIT)
    G.seen_claim(t, key, G.SEV_CRIT, T0)
    G.seen_mark(t, key, T0, state=G.SEEN_STATE_DELIVERED)
    removed = G.seen_gc(t, T0 + G.SEEN_TTL_SECS + 1, G.SEEN_TTL_SECS)
    check("2a 미종결이 없으면 만료 레코드는 삭제된다(무한 성장 차단)",
          removed == 1 and not os.path.exists(G.seen_path(t, key)), "removed=%r" % removed)
    # 보존 상한을 넘긴 미종결도 솎인다
    key2 = G.seen_key("stall_confirmed", "s:stale", G.SEV_CRIT)
    G.seen_claim(t, key2, G.SEV_CRIT, T0)
    G.seen_pending_add(t, key2, "W-old", T0)
    removed = G.seen_gc(t, T0 + G.SEEN_PENDING_KEEP_SECS + 1, G.SEEN_TTL_SECS)
    check("2b 보존 상한(8h)을 넘긴 미종결 id 는 레코드째 정리된다",
          removed == 1 and not os.path.exists(G.seen_path(t, key2)), "removed=%r" % removed)
    check("2c 보존 상한이 데몬 큐 TTL(6h)보다 길다(만료 통지가 도착할 시간)",
          G.SEEN_PENDING_KEEP_SECS > QUEUE_TTL,
          "keep=%r queue_ttl=%r" % (G.SEEN_PENDING_KEEP_SECS, QUEUE_TTL))

# ── ③ N9: 만료는 무장을 풀지 않는다 — 조건이 지속되면 seen TTL 안에 재통보된다 ──────────
with tempfile.TemporaryDirectory() as t:
    r = FakeRunner()
    g = gate(t, r)
    w = {"trigger": "death", "severity": G.SEV_CRIT, "idem": "death:worker",
         "task": "gate-death", "cooldown": 0,          # ★쿨다운 0 = 종전 판본의 영구 침묵 조건
         "avoid_role": "worker", "wake_body": "worker 좌석 사망 확증", "stamp": {}}
    counters, reasons = {}, []
    # ★영수증 회수가 **이번 실행에서 성립**해야 critical 이 at-least-once(inflight)로 남는다 —
    #   폴링 없이 부르면 `ack_unavailable_downgrade` 로 at-most-once 강등되어 이 축을 못 잰다.
    g._poll_once()
    target = g._push(w, counters, T0, reasons, ["cso", "master"])
    check("3a 첫 push 가 큐에 실린다", target == "cso" and len(r.enqueues) == 1, repr(r.enqueues))
    key = G.seen_key("death", "death:worker", G.SEV_CRIT)
    wid = r.enqueues and ("W-%010x" % 1)
    r.events = [{"name": "queue.expired", "payload": {"entry_ids": [wid]}}]
    g._poll_once()
    g._reconcile_inflight(counters, T0 + 60)
    check("3b 만료가 종결로 기록된다", load_rec(t, key)["state"] == G.SEEN_STATE_EXPIRED)
    check("3c ★엣지 무장이 풀리지 않는다",
          G.edge_state(counters, "push_edge", key).get("armed", True) is True,
          repr(counters.get("push_edge")))
    check("3d 만료가 계수로 남는다", counters.get("expired_undelivered") == 1)
    # 억제 상한은 그대로 seen TTL 이다 — TTL 안에는 재통보되지 않는다(과교정 아님)
    reasons2 = []
    t_in = T0 + G.SEEN_TTL_SECS - 10
    check("3e 음성 대조 — seen TTL 안에서는 재통보되지 않는다",
          g._push(w, counters, t_in, reasons2, ["cso", "master"]) is None
          and len(r.enqueues) == 1, "%r / %r" % (reasons2, r.enqueues))
    # TTL 이 지나면 **한 번은 반드시** 다시 나간다(종전 판본은 쿨다운 0 이라 영영 안 나갔다)
    reasons3 = []
    t_out = T0 + G.SEEN_TTL_SECS + 10
    target2 = g._push(w, counters, t_out, reasons3, ["cso", "master"])
    check("3f ★seen TTL 이 지나면 재통보가 나간다(영구 침묵 금지)",
          target2 == "cso" and len(r.enqueues) == 2, "%r / %r / %r" % (target2, r.enqueues, reasons3))

# ── ④ N9: 만료가 배지·대장으로 관측된다(엔드투엔드 한 주기) ────────────────────────────
with tempfile.TemporaryDirectory() as t:
    key = G.seen_key("stall_confirmed", "s:obs", G.SEV_CRIT)
    G.seen_claim(t, key, G.SEV_CRIT, T0)
    G.seen_mark(t, key, T0, state=G.SEEN_STATE_INFLIGHT, wakeup_id="W-obs")
    G.seen_pending_add(t, key, "W-obs", T0)
    r = FakeRunner(rep=report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}]),
                   events=[{"name": "queue.expired", "payload": {"entry_ids": ["W-obs"]}}])
    rc = gate(t, r).run()
    check("4a 게이트는 정상 종료한다", rc == 0, "rc=%r" % rc)
    bs = badges(t)
    check("4b ★만료가 배지로 드러난다", "queue-expired-undelivered" in bs,
          repr(sorted(bs)))
    det = (bs.get("queue-expired-undelivered") or {}).get("detail") or {}
    check("4c 배지가 어느 사건인지 짚는다", det.get("wakeup_id") == "W-obs" and det.get("key") == key,
          repr(det))
    ent = ledger_entries(t)[-1]
    check("4d 대장 사유에도 남는다",
          any(str(x).startswith("expired_undelivered:") for x in (ent.get("reasons") or [])),
          repr(ent.get("reasons")))
    check("4e ★배달로 세지 않는다(그 주기에 push 0)", r.enqueues == [], repr(r.enqueues))

print("")
if fails:
    print("FAILED %d: %s" % (len(fails), " · ".join(fails)))
    sys.exit(1)
print("ALL PASS")
print("REFL-WAKEUP-EXPIRY-OK")
