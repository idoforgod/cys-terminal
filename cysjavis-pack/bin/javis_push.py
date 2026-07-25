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
- 배달은 **재구현하지 않는다** — 기존 `javis_wakeup.py enqueue`(코얼레싱·zombie 가드·
  scrub된 원장)에 위임하고, drain은 기존 `cys send --queued` 경로를 그대로 쓴다.

★B1(리뷰 수렴 BLOCK 수리) 최신 보고 폐기 근절:
  종전에는 enqueue 에 `--idempotency-key <key>` 를 넘겼다. wakeup 의 멱등키 의미론은
  "같은 키의 요청은 **중복 삽입하지 않는다**(suppressed)" 이므로, 같은 --key 로 낸
  **두 번째 보고가 통째로 버려졌다** — 진행 보고는 본질적으로 "최신 상태"인데
  최신분이 폐기되고 최초분이 남는 정반대 동작이었다(exit 5 를 '정상 무작업'으로
  포장해 무음이었다). D5 의 키 의미론은 "이 주제의 최신 상태"이고, 그것을 구현하는
  것은 wakeup 의 **코얼레스 경로**다(reason 최신 갱신·coalesced_count 증가).
  따라서 enqueue 에서 멱등키를 뗀다 — 같은 (target, task_key) 는 자동 코얼레스된다.
  대신 배달(drain) 쪽 `cys send --queued` 에 `--idempotency-key <task_key>` 를 붙여
  데몬 C4 제자리 병합을 태운다(keyless 통계 오염도 함께 해소).

exit codes: 0 ok(enqueued|coalesced) · 2 usage/검증 실패 · 5 suppressed(호환 잔존 —
  현 경로에서는 발생하지 않는다) · 6 배달 위임 실패(wakeup 호출 자체가 실패)
"""
import argparse
import json
import os
import subprocess
import sys

BIN_DIR = os.path.dirname(os.path.abspath(__file__))
WAKEUP = os.path.join(BIN_DIR, "javis_wakeup.py")

EXIT_OK, EXIT_USAGE, EXIT_SUPPRESSED, EXIT_DELEGATE = 0, 2, 5, 6

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
    # ★B1: 멱등키를 넘기지 않는다 — 같은 --key 의 후속 보고는 **코얼레스**(최신 reason 승리)
    #      되어야 한다. 멱등키를 넘기면 최신 보고가 suppressed 로 폐기됐다(D5 정반대).
    argv = ["enqueue", "--to", to, "--task", key, "--reason", reason]
    rc, out, err = run_wakeup(argv)
    if rc != 0:
        # ★B6④: 위임 실패는 usage 오류가 아니다 — 종료코드를 분리하고, 호출자가 손으로
        #        재현·복구할 수 있게 실행 경로·argv·후속 지시를 함께 준다.
        return EXIT_DELEGATE, None, (
            "배달 위임 실패(rc=%s): %s\n"
            "  wakeup: %s\n"
            "  argv:   %s\n"
            "  후속:   위 명령을 직접 실행해 원인을 확인하라. 계속 실패하면 전문 파일은 이미 "
            "디스크에 있으므로, 포인터 1줄만 `cys send --queued --to %s \"...\"` 로 직접 보내라."
            % (rc, (err or out or "").strip()[:200], WAKEUP, " ".join(argv), to)
        )
    result = _wakeup_result(out)
    if result == "suppressed":
        # 멱등키를 넘기지 않으므로 현 경로에서는 나오지 않는다. 구 wakeup·수동 큐 잔재
        # 대비로 매핑만 유지한다(무작업이지 실패가 아니다).
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
                   help="보고 키(=병합 단위). 같은 주제의 연속 보고는 이 키로 코얼레스되어 "
                        "**최신 보고가 이긴다**(구 보고가 최신을 막지 않는다)")
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
