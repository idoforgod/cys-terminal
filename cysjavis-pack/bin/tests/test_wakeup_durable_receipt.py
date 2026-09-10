#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_wakeup_durable_receipt.py — [triage 2026-09-08 · codex major M10 잔여]
데몬이 `durable:false`(큐 WAL 저장 실패 = 항목이 아직 메모리에만 있다)로 답했는데도
자동 소비자(`javis_wakeup.py drain --deliver`)가 원본 pending 을 지우는가.

무엇이 문제인가: `cys send --queued` 는 `durable=false` 를 **stdout 한 조각 + stderr 경고**로만
알리고 exit 0 이다(운영자 판단을 위한 설계 · 그 자체는 타당). 그런데 소비자는
`subprocess.run(..., check=True)` 만 보고 성공으로 처리해 pending 파일을 즉시 삭제한다
(javis_wakeup.py cmd_drain). 그 뒤 데몬이 WAL 재시도(≤5s) 전에 죽으면 **양쪽 어디에도 없다**
= wakeup 사건 유실. 최소 요구: 미확정을 받았으면 원본을 버리지 않거나(재시도 가능),
적어도 원장에 그 사실을 남겨 추적할 수 있어야 한다.

밀폐: JAVIS_ROOT·CYS_PACK_DIR 임시 디렉터리 · PATH 선두에 **목 `cys`**(durable=false 를 출력하고
exit 0). 라이브 데몬·라이브 팩 무접촉.
출력: PASS/FAIL · 실패 시 exit 1 · 통과 시 종료 토큰 WAKEUP-DURABLE-RECEIPT-OK.
실행: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" python3 bin/tests/test_wakeup_durable_receipt.py
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
echo "QUEUED (depth 1) · durable=false"
echo "[queue] 경고: 데몬이 큐 WAL 저장에 실패했다(durable=false)" >&2
exit 0
"""


def main():
    root = tempfile.mkdtemp(prefix="wakeup-durable-")
    binhome = tempfile.mkdtemp(prefix="wakeup-durable-bin-")
    calls = os.path.join(binhome, "calls.log")
    mock = os.path.join(binhome, "cys")
    with open(mock, "w", encoding="utf-8") as f:
        f.write(MOCK_CYS)
    os.chmod(mock, 0o755)

    env = dict(os.environ)
    env["JAVIS_ROOT"] = root
    env["CYS_PACK_DIR"] = tempfile.mkdtemp(prefix="wakeup-durable-pack-")
    env["JAVIS_WAKEUP_LIVENESS"] = "alive"
    env["MOCK_CALLS"] = calls
    env["PATH"] = binhome + os.pathsep + env.get("PATH", "")

    def run(*args):
        return subprocess.run([sys.executable, WAKEUP, *args], env=env,
                              capture_output=True, text=True, timeout=60)

    r = run("enqueue", "--to", "master", "--task", "t1", "--reason", "삼각 검증")
    assert r.returncode == 0, f"enqueue 실패: {r.stderr}"
    pending_dir = os.path.join(root, "_round", "wakeups", "pending")
    before = sorted(os.listdir(pending_dir))
    assert len(before) == 1, f"전제: pending 1건이어야 한다 — {before}"

    r = run("drain", "--deliver")
    assert r.returncode == 0, f"drain 실패({r.returncode}): {r.stdout}{r.stderr}"
    assert os.path.exists(calls), "전제: 목 cys 가 호출되지 않았다"

    after = sorted(os.listdir(pending_dir))
    ledger = os.path.join(root, "_round", "wakeups", "queue.jsonl")
    rows = []
    if os.path.exists(ledger):
        with open(ledger, encoding="utf-8") as f:
            rows = [json.loads(l) for l in f if l.strip()]
    delivered = [r for r in rows if r.get("event") == "delivered"]
    tracked = [
        r for r in rows
        if any(k in r for k in ("durable", "queue_entry_id", "receipt", "not_durable"))
    ]

    ok = bool(after) or bool(tracked)
    if ok:
        q9 = _q9_console_encoding_axes()
        if q9:
            print("FAIL " + " | ".join(q9))
            return 1
        print("PASS 내구 미확정(durable=false)을 받은 뒤 원본을 버리지 않거나 추적한다"
              " · 콘솔 인코딩 오류(디코딩·표시)가 성공한 enqueue 를 재전송시키지 않는다")
        print("WAKEUP-DURABLE-RECEIPT-OK")
        return 0
    print(
        "FAIL 데몬이 durable=false 로 답했는데 소비자가 원본 pending 을 지웠다 "
        f"(pending {before} → {after} · delivered {len(delivered)}건 · 내구 표식 0) — "
        "데몬이 WAL 재시도 전에 죽으면 그 wakeup 은 양쪽 어디에도 없다"
    )
    return 1


# ★(0.14.31 · 성찰 Q9) 데몬 응답에 로캘 비호환 문자(`·` U+00B7 · 한글 · 잘못된 바이트 0xFF)를 넣는다.
MOCK_CYS_Q9 = """#!/bin/sh
echo "$@" >> "$MOCK_CALLS"
printf 'QUEUED (depth 1) \\302\\267 durable=true \\377\\n'
printf '[queue] \\355\\225\\234\\352\\270\\200 \\302\\267 stderr\\n' >&2
exit 0
"""


def _q9_console_encoding_axes():
    """★(0.14.31 · 성찰 Q9) **성공한 enqueue 가 콘솔 인코딩 오류 때문에 재전송되지 않는다.**

    두 축을 각각 잰다: ⓐ 디코딩 — C 로캘(ASCII)에서 `text=True` 였다면 `UnicodeDecodeError` 가
    `run()` 안에서 났다 ⓑ 표시 — `PYTHONIOENCODING=ascii:strict` 콘솔에 캡처 출력을 되쏘면
    `UnicodeEncodeError` 가 났다. 둘 다 except 절 밖의 예외라 drain 이 죽고 pending 이 남아
    다음 drain 이 같은 wakeup 을 다시 보냈다(Windows cp949 실측 계열). 기대: 각 축에서 rc 0 ·
    목 cys 호출 정확히 1회 · pending 0 · 원장 delivered 1 · 두 번째 drain 은 호출 0.
    """
    fails = []
    axes = {
        "decode(C-locale)": {"PYTHONUTF8": "0", "PYTHONCOERCECLOCALE": "0", "LC_ALL": "C", "LANG": "C"},
        "display(ascii-console)": {"PYTHONIOENCODING": "ascii:strict"},
    }
    for label, extra in axes.items():
        root = tempfile.mkdtemp(prefix="wakeup-q9-")
        binhome = tempfile.mkdtemp(prefix="wakeup-q9-bin-")
        calls = os.path.join(binhome, "calls.log")
        mock = os.path.join(binhome, "cys")
        with open(mock, "w", encoding="utf-8") as f:
            f.write(MOCK_CYS_Q9)
        os.chmod(mock, 0o755)
        env = dict(os.environ)
        env.pop("LC_CTYPE", None)
        env.update({"JAVIS_ROOT": root, "CYS_PACK_DIR": tempfile.mkdtemp(prefix="wakeup-q9-pack-"),
                    "JAVIS_WAKEUP_LIVENESS": "alive", "MOCK_CALLS": calls,
                    "PATH": binhome + os.pathsep + env.get("PATH", "")})
        env.update(extra)

        def run(*args):
            return subprocess.run([sys.executable, WAKEUP, *args], env=env,
                                  capture_output=True, timeout=60)

        r = run("enqueue", "--to", "master", "--task", "q9", "--reason", "encoding probe")
        if r.returncode != 0:
            fails.append(f"[{label}] enqueue 실패 rc={r.returncode}: {r.stderr[-300:]!r}")
            continue
        r1 = run("drain", "--deliver")
        pending_dir = os.path.join(root, "_round", "wakeups", "pending")
        left = [n for n in os.listdir(pending_dir) if n.endswith(".json")] if os.path.isdir(pending_dir) else []
        n1 = len(open(calls, encoding="utf-8").read().splitlines()) if os.path.exists(calls) else 0
        if r1.returncode != 0:
            fails.append(f"[{label}] 표시·디코딩 오류가 drain 을 죽였다 rc={r1.returncode}: {r1.stderr[-300:]!r}")
        if n1 != 1:
            fails.append(f"[{label}] 전제 붕괴: 첫 drain 호출 수 {n1}")
        if left:
            fails.append(f"[{label}] 성공한 enqueue 의 pending 이 남았다(재전송 예약): {left}")
        ledger = os.path.join(root, "_round", "wakeups", "queue.jsonl")
        rows = []
        if os.path.exists(ledger):
            with open(ledger, encoding="utf-8") as f:
                rows = [json.loads(l) for l in f if l.strip()]
        if len([x for x in rows if x.get("event") == "delivered"]) != 1:
            fails.append(f"[{label}] 원장 delivered 가 1 이 아니다: {[x.get('event') for x in rows]}")
        r2 = run("drain", "--deliver")
        n2 = len(open(calls, encoding="utf-8").read().splitlines()) if os.path.exists(calls) else 0
        if n2 != n1:
            fails.append(f"[{label}] 두 번째 drain 이 같은 wakeup 을 재전송했다({n1}→{n2})")
    return fails


if __name__ == "__main__":
    sys.exit(main())
