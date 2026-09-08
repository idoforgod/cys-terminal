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

    r = run("enqueue", "--to", "master", "--task", "t1", "--reason", "보관 실패 재현")
    assert r.returncode == 0, f"enqueue 실패: {r.stderr}"
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

    if fails:
        print("FAIL " + " | ".join(fails))
        print("stdout1=", r1.stdout.strip()[-300:])
        print("stdout2=", r2.stdout.strip()[-300:])
        return 1
    print("PASS 보관 실패는 drain 을 죽이지 않고 · 재전송하지 않으며 · park_failed 로 갈라 기록된다")
    print("WAKEUP-PARK-FAILURE-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
