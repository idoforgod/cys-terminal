#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_wakeup_park_failure.py — [수렴 R2 · reviewer-codex F2 · reviewer-claude minor]
`cys send --queued` 가 이미 수락한(exit 0 · durable=false) wakeup 을 보관(archival)하지 못했을 때,
그 항목이 **다음 drain 에서 다시 배달되는가**.

무엇이 문제였나: `_park_unconfirmed` 는 ⓐ `os.makedirs` 를 try 밖에 두어 여기서 OSError 가 나면
`cmd_drain` 이 통째로 예외 종료했고(전송은 이미 성공한 뒤 → pending 잔존 → 다음 drain 이 재전송),
ⓑ `os.replace` 실패 시 **원본 pending 경로를 그대로 반환**해 원장에는 `delivered_unconfirmed`
{parked:<pending 경로>} 가 남는데 파일은 여전히 pending 에 있었다. 큐에는 멱등 키가 없으므로
재전송은 곧 **중복 배달**이다(정본 §7 위험 ① 폭주). 이 검체는 보관 이동을 실패시켜 두 축을 잰다:
  ① drain 이 예외 종료하지 않고 exit 0 으로 끝난다(makedirs 실패 포함)
  ② 두 번째 drain 이 같은 wakeup 을 **다시 보내지 않는다**(목 cys 호출 1회)
  ③ 원장이 사실을 갈라 적는다 — 보관 실패는 `delivered_unconfirmed` 가 아니라 `park_failed`

밀폐: JAVIS_ROOT·CYS_PACK_DIR 임시 디렉터리 · PATH 선두에 목 `cys`(durable=false · exit 0) ·
보관 실패는 `unconfirmed` 를 **파일**로 선점해 만든다(makedirs 가 FileExistsError=OSError).
라이브 데몬·라이브 팩 무접촉.
출력: PASS/FAIL · 실패 시 exit 1 · 통과 시 종료 토큰 WAKEUP-PARK-FAILURE-OK.
실행: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" python3 bin/tests/test_wakeup_park_failure.py
"""
import json
import os
import subprocess
import sys
import tempfile

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WAKEUP = os.path.join(BIN, "javis_wakeup.py")

MOCK_CYS = """#!/bin/sh
# 목 cys — enqueue 는 받았으나 데몬이 WAL 저장에 실패했다(durable=false · exit 0).
echo "$@" >> "$MOCK_CALLS"
echo "QUEUED (depth 1) \u00b7 durable=false"
exit 0
"""


def _rows(ledger):
    if not os.path.exists(ledger):
        return []
    with open(ledger, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


def main():
    root = tempfile.mkdtemp(prefix="wakeup-park-")
    binhome = tempfile.mkdtemp(prefix="wakeup-park-bin-")
    calls = os.path.join(binhome, "calls.log")
    mock = os.path.join(binhome, "cys")
    with open(mock, "w", encoding="utf-8") as f:
        f.write(MOCK_CYS)
    os.chmod(mock, 0o755)

    env = dict(os.environ)
    env["JAVIS_ROOT"] = root
    env["CYS_PACK_DIR"] = tempfile.mkdtemp(prefix="wakeup-park-pack-")
    env["JAVIS_WAKEUP_LIVENESS"] = "alive"
    env["MOCK_CALLS"] = calls
    env["PATH"] = binhome + os.pathsep + env.get("PATH", "")

    def run(*args):
        return subprocess.run([sys.executable, WAKEUP, *args], env=env,
                              capture_output=True, text=True, timeout=60)

    r = run("enqueue", "--to", "master", "--task", "t1", "--reason", "보관 실패 재현",
            "--idempotency-key", "k1")
    assert r.returncode == 0, f"enqueue 실패: {r.stderr}"
    first_id = json.loads([l for l in r.stdout.splitlines() if l.startswith("{")][-1])["id"]
    wk_dir = os.path.join(root, "_round", "wakeups")
    pending_dir = os.path.join(wk_dir, "pending")
    assert len(os.listdir(pending_dir)) == 1, "전제: pending 1건"

    # 보관 실패 주입 — `unconfirmed` 를 **파일**로 선점한다(makedirs·replace 둘 다 OSError).
    with open(os.path.join(wk_dir, "unconfirmed"), "w", encoding="utf-8") as f:
        f.write("보관 디렉터리 자리를 파일이 차지하고 있다")

    r1 = run("drain", "--deliver")
    ledger = os.path.join(wk_dir, "queue.jsonl")
    fails = []
    # ① drain 이 예외로 죽지 않는다.
    if r1.returncode != 0:
        fails.append(f"보관 실패가 drain 을 예외 종료시켰다(rc={r1.returncode}) — "
                     f"전송은 이미 성공한 뒤라 다음 drain 이 재전송한다: {r1.stderr.strip()[-400:]}")
    calls1 = len(open(calls, encoding="utf-8").read().splitlines()) if os.path.exists(calls) else 0
    if calls1 != 1:
        fails.append(f"전제 붕괴: 첫 drain 의 배달 횟수가 1이 아니다({calls1})")

    # ② 두 번째 drain 이 같은 wakeup 을 다시 보내지 않는다.
    r2 = run("drain", "--deliver")
    calls2 = len(open(calls, encoding="utf-8").read().splitlines()) if os.path.exists(calls) else 0
    if calls2 > calls1:
        fails.append(f"보관 실패 뒤 다음 drain 이 같은 wakeup 을 재전송했다({calls1}→{calls2}) — "
                     "큐에 멱등 키가 없어 중복 배달이 된다(정본 §7 위험 ①)")

    # ③ 원장이 사실을 갈라 적는다.
    rows = _rows(ledger)
    park_failed = [x for x in rows if x.get("event") == "park_failed"]
    lied = [x for x in rows
            if x.get("event") == "delivered_unconfirmed"
            and str(x.get("parked", "")).startswith(pending_dir)]
    if lied:
        fails.append("원장이 '보관했다'(delivered_unconfirmed)고 적었는데 경로는 pending 이다 — "
                     f"{lied[0].get('parked')}")
    if not park_failed:
        fails.append("보관 실패가 원장에 park_failed 로 남지 않았다(사실이 어디에도 없다)")
    elif park_failed[0].get("will_retransmit") is not False:
        fails.append(f"park_failed 가 재전송 예정으로 남았다: {park_failed[0]}")

    # ★(0.14.31 · 성찰 Q4) 표식 레코드가 **후속 wakeup 을 흡수하지 않는다** — 세대 분리.
    #   종전: 같은 (target, task) 의 새 요청이 표식 레코드에 병합돼 `coalesced` 로 성공 처리됐고
    #   drain 은 표식 레코드를 영구히 건너뛰었다 = 채널 단위 무성 유실.
    # ⓐ 같은 멱등키 = 정말 수락된 그 사건 → 억제(종전과 같다 · 재전송 0).
    r3 = run("enqueue", "--to", "master", "--task", "t1", "--reason", "같은 사건", "--idempotency-key", "k1")
    j3 = json.loads([l for l in r3.stdout.splitlines() if l.startswith("{")][-1]) if r3.returncode == 0 else {}
    if r3.returncode != 0 or j3.get("result") != "suppressed":
        fails.append(f"수락 세대와 같은 멱등키가 억제되지 않았다(rc={r3.returncode} {j3})")
    # ⓑ 다른 멱등키 = 새 사건 → 병합이 아니라 **새 레코드**(수락 세대는 슬롯 밖으로).
    r4 = run("enqueue", "--to", "master", "--task", "t1", "--reason", "후속 사건", "--idempotency-key", "k2")
    j4 = json.loads([l for l in r4.stdout.splitlines() if l.startswith("{")][-1]) if r4.returncode == 0 else {}
    if r4.returncode != 0 or j4.get("result") != "queued" or j4.get("id") == first_id:
        fails.append(f"후속 wakeup 이 수락 세대에 흡수됐다(rc={r4.returncode} {j4} · 첫 id={first_id})")
    rows = _rows(ledger)
    split = [x for x in rows if x.get("event") == "generation_split"]
    if not split or split[-1].get("accepted_wakeup_id") != first_id:
        fails.append("세대 분리가 원장에 generation_split 로 남지 않았다")
    # ⓒ drain: 후속 세대만 배달되고 수락 세대는 재전송되지 않는다(유실 0 · 중복 0).
    r5 = run("drain", "--deliver")
    lines = open(calls, encoding="utf-8").read().splitlines() if os.path.exists(calls) else []
    if len(lines) != calls2 + 1:
        fails.append(f"세대 분리 뒤 drain 의 배달 횟수가 +1 이 아니다({calls2}→{len(lines)})")
    elif j4.get("id") and (j4["id"] not in lines[-1] or first_id in lines[-1]):
        fails.append(f"마지막 배달 본문에 후속 id 가 없거나 수락 세대 id 가 섞였다: {lines[-1][-200:]}")
    # 수락 세대는 여전히 사람이 볼 수 있다(list · 표식 보존).
    r6 = run("list")
    still = [x for x in json.loads(r6.stdout or "[]") if x.get("id") == first_id]
    if not still or not still[0].get("delivered_unconfirmed_at"):
        fails.append("세대 분리가 수락 세대를 잃었다(list 에 표식 레코드가 없다)")
    # ⓓ 슬롯을 비우지 못하면 새 요청을 **성공 처리하지 않는다**(in-process · os.replace 전면 실패).
    fails.extend(_q4_refuse_axis())

    if fails:
        print("FAIL " + " | ".join(fails))
        print("stdout1=", r1.stdout.strip()[-300:])
        print("stdout2=", r2.stdout.strip()[-300:])
        return 1
    print("PASS 보관 실패는 drain 을 죽이지 않고 · 재전송하지 않으며 · park_failed 로 갈라 기록된다"
          " · 표식 레코드는 후속 wakeup 을 흡수하지 않는다(세대 분리 · 유실 0)")
    print("WAKEUP-PARK-FAILURE-OK")
    return 0


def _q4_refuse_axis():
    """★(0.14.31 · 성찰 Q4 ⓓ) 수락 세대를 슬롯 밖으로 옮기는 세 시도(보관소 이동·표식 재기록·
    같은 디렉터리 비켜 세우기)가 **전부** 실패하면 enqueue 는 rc≠0 으로 답하고 아무것도 병합하지
    않는다. 이 축은 in-process 로 잰다(`os.replace` 전면 실패는 파일시스템으로 만들기 어렵다)."""
    import argparse
    root = tempfile.mkdtemp(prefix="wakeup-park-refuse-")
    os.environ["JAVIS_ROOT"] = root
    sys.path.insert(0, BIN)
    import importlib
    jw = importlib.import_module("javis_wakeup")
    jw = importlib.reload(jw)  # ROOT 를 이 검체의 임시 디렉터리로
    fails = []
    ns = argparse.Namespace(to="master", task="t2", reason="첫 사건", payload=None,
                            idempotency_key="a1", severity="warn")
    assert jw.cmd_enqueue(ns) == jw.EXIT_OK, "전제: 첫 enqueue"
    path = jw._pending_path("master", "t2")
    rec = jw._load_pending(path)
    rec[jw.PARK_MARK] = jw._now()
    jw._write_json_atomic(path, rec)  # 수락 세대 표식(보관 이동 실패 뒤의 상태)
    real_replace = os.replace
    def _boom(*_a, **_k):
        raise OSError("replace refused (injected)")
    os.replace = _boom
    try:
        ns2 = argparse.Namespace(to="master", task="t2", reason="후속 사건", payload=None,
                                 idempotency_key="a2", severity="warn")
        rc = jw.cmd_enqueue(ns2)
    finally:
        os.replace = real_replace
    if rc != getattr(jw, "EXIT_PARK_FAILED", -1):
        fails.append(f"슬롯을 비우지 못했는데 enqueue 가 성공으로 답했다(rc={rc})")
    after = jw._load_pending(path)
    if not after or after.get("id") != rec["id"] or after.get("coalesced_count", 0) != 0 \
            or "a2" in (after.get("idempotency_keys") or []):
        fails.append(f"거부하면서 수락 세대에 병합했다: {after}")
    rows = _rows(jw.LEDGER)
    if not any(x.get("event") == "enqueue_refused" and x.get("blocked_by") == rec["id"] for x in rows):
        fails.append("거부가 원장에 enqueue_refused 로 남지 않았다")
    return fails


if __name__ == "__main__":
    sys.exit(main())
