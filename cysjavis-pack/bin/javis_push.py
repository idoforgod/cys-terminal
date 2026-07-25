#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_push.py — 이중 채널 보고: 포인터 1줄(채팅) + 전문 파일(디스크) (C7 · DESIGN §4-C7)

배경(응답 폭주 사고 2026-07-26): 노드들이 진행·상태 보고를 채팅 본문에 통째로 밀어넣어
master 큐가 포화됐다. 해법은 "전문은 파일, 채팅에는 포인터 1줄"의 이중 채널이다.

계약:
- `--to <role> --key <k> --pointer "<1줄>" [--body-file <전문 md>]`
- body-file 지정 시 **실존·비어있지 않음**을 검증한다(실패 exit 2 — 없는 파일을 가리키는
  포인터는 보고가 아니라 소음이다).
- 포인터 문자열에 `· 전문: <절대경로>` 를 자동 첨부한다(수신자가 파일을 열 수 있게).
- 배달은 **재구현하지 않는다** — 기존 `javis_wakeup.py enqueue`(코얼레싱·멱등키·zombie 가드·
  scrub된 원장)에 위임하고, drain은 기존 `cys send --queued` 경로를 그대로 쓴다.
  멱등키 = `--key` 값 → 같은 주제의 연속 보고는 병합/억제되어 큐를 채우지 않는다.

exit codes: 0 ok(queued|coalesced) · 2 usage/검증 실패 · 5 suppressed(멱등 억제 — 무작업)
"""
import argparse
import json
import os
import subprocess
import sys

BIN_DIR = os.path.dirname(os.path.abspath(__file__))
WAKEUP = os.path.join(BIN_DIR, "javis_wakeup.py")

EXIT_OK, EXIT_USAGE, EXIT_SUPPRESSED = 0, 2, 5

POINTER_BODY_SEP = " · 전문: "


def validate_body_file(path):
    """전문 파일 검증. 반환 (절대경로, None) 또는 (None, 사유)."""
    ap = os.path.abspath(os.path.expanduser(path))
    if not os.path.isfile(ap):
        return None, "전문 파일 없음: %s" % ap
    try:
        if os.path.getsize(ap) == 0:
            return None, "전문 파일이 비어있음: %s" % ap
    except OSError as e:
        return None, "전문 파일 stat 실패: %s (%s)" % (ap, e)
    return ap, None


def compose_pointer(pointer, body_abspath=None):
    """포인터 1줄 조립 — 개행은 공백으로 접는다(1줄 계약) + 전문 경로 자동 첨부."""
    one_line = " ".join(str(pointer).split())
    if body_abspath:
        return one_line + POINTER_BODY_SEP + body_abspath
    return one_line


def run_wakeup(argv, timeout=20):
    """javis_wakeup.py 서브프로세스 실행(같은 폴더 형제 모듈 기준). 반환 (rc, out, err).
    테스트는 이 함수만 대체하면 배달 위임을 밀폐 검증할 수 있다."""
    try:
        r = subprocess.run([sys.executable, WAKEUP] + argv,
                           capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout, r.stderr
    except (subprocess.SubprocessError, OSError) as e:
        return 255, "", str(e)


def _wakeup_result(out):
    """wakeup enqueue stdout(JSON 1줄)에서 result를 뽑는다. 파싱 실패는 None."""
    try:
        obj = json.loads((out or "").strip().splitlines()[-1])
    except (ValueError, IndexError):
        return None
    return obj.get("result") if isinstance(obj, dict) else None


def push(to, key, pointer, body_file=None):
    """반환 (exit_code, result_str|None, 오류메시지|None)."""
    body_abs = None
    if body_file:
        body_abs, why = validate_body_file(body_file)
        if why:
            return EXIT_USAGE, None, why
    reason = compose_pointer(pointer, body_abs)
    rc, out, err = run_wakeup(["enqueue", "--to", to, "--task", key,
                               "--reason", reason, "--idempotency-key", key])
    if rc != 0:
        return EXIT_USAGE, None, "wakeup enqueue 실패(rc=%s): %s" % (rc, (err or out or "").strip()[:200])
    result = _wakeup_result(out)
    if result == "suppressed":
        # 멱등 억제 = 이미 같은 키의 보고가 대기 중. 무작업이지 실패가 아니다.
        return EXIT_SUPPRESSED, "suppressed", None
    if result == "queued":
        return EXIT_OK, "enqueued", None
    if result == "coalesced":
        return EXIT_OK, "coalesced", None
    return EXIT_USAGE, None, "wakeup 응답 해석 불가: %r" % (out or "")[:200]


def main(argv=None):
    p = argparse.ArgumentParser(
        description="이중 채널 보고 — 포인터 1줄 push + 전문 파일 (C7)")
    p.add_argument("--to", required=True, help="수신 역할(master·cso·worker …)")
    p.add_argument("--key", required=True,
                   help="보고 키(=멱등키·병합 단위). 같은 주제의 연속 보고는 이 키로 병합된다")
    p.add_argument("--pointer", required=True, help="채팅에 실릴 1줄 요약")
    p.add_argument("--body-file", dest="body_file",
                   help="전문 md 경로(권장 절대경로) — 실존·비어있지않음을 검증한다")
    a = p.parse_args(argv)

    code, result, why = push(a.to, a.key, a.pointer, a.body_file)
    if why:
        print("[javis_push] %s" % why, file=sys.stderr)
        return code
    print(json.dumps({"result": result}, ensure_ascii=False))
    return code


if __name__ == "__main__":
    sys.exit(main())
