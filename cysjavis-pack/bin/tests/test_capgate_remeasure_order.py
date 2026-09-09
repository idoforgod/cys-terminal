#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_capgate_remeasure_order.py — 능력 게이트 표적 재측정의 **국면**(성찰 P7)과 미해소 표식
판독의 **정본·3값 의미**(성찰 P16). 2026-09-10.

## 무엇을 막는가
P7: 재측정(`preflight --fix --only C28.self-correction`)이 부트 체인 **단계 ①** 에서 돌았다.
`unknown` 의 지배적 원인은 `cys status --json` 무응답인데, ① 은 ② 데몬 생존 확인보다 **앞**이다 —
같은 국면·같은 입력을 다시 재므로 재시도가 새 정보를 얻지 못한다. 매 부트가
`unknown → 표식 → 다음 부트 ① 에서 다시 unknown` 을 반복하고 게이트는 영원히 미등록인데
부트 로그는 매번 rc 0(초록)이다(봉인표 ③ 의 다른 얼굴).

P16: 표식 경로가 부트 체인엔 **리터럴**, preflight 엔 API 로 두 벌이었다. ⓐ 파일명을 옮기면
부트 체인이 경고 0 으로 "fast path 로 preflight 영구 생략" 으로 되돌아가고 ⓑ `capgate_unresolved()`
가 문서화한 '판독 실패는 미해소' 3값 의미가 `os.path.exists` 한 값으로 접혀, **있는데 못 보는**
표식이 '해소됨' 이 된다.

## 이 파일이 못박는 것
  1) 단계 레지스트리에서 `②′preflight-capgate` 가 `②ping` **뒤**, `③claim-role` **앞**이다
     (이 파일의 계약: 선언 순서 = 실행 순서).
  2) `_cmd_run_chain` 소스에서 `--only C28.self-correction` 호출이 ② ping 판정 **뒤**에 있고,
     ① 갈래는 재측정을 예약만 한다(`_cap_remeasure`).
  3) 부트 체인은 리터럴이 아니라 `javis_preflight.capgate_unresolved()` 를 소비한다.
  4) 조회 실패(EACCES)는 '해소됨' 이 아니다 — 표식이 있는데 못 보면 **미해소**로 읽는다.
  5) 음성 대조(검출력): 같은 경로에서 종전 술어 `os.path.exists` 는 False(= '해소됨')를 낸다.

## 재지 않는 것(정직)
①무응답 · ②응답 **라이브 2국면 픽스처**에서 같은 부트 안에 게이트가 실제로 등록되는지는 여기서
재지 않는다 — 데몬·좌석을 띄워야 하고 이 검체는 라이브 무접촉이다. 여기서 재는 것은 **국면의
순서**(재측정이 데몬 생존 확인 뒤에 온다)와 **표식 판독의 방향**이다.

실행: python3 bin/tests/test_capgate_remeasure_order.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 CAPGATE-REMEASURE-OK.
"""
import inspect
import os
import shutil
import sys
import tempfile

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)
sys.dont_write_bytecode = True

import javis_bootstrap as bs  # noqa: E402
import javis_preflight as pf  # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── 1. 단계 레지스트리 순서 ─────────────────────────────────────────────────────
check("1 ②′preflight-capgate 는 ②ping 뒤 · ③claim-role 앞(선언 순서 = 실행 순서)",
      bs.STEP_INDEX[bs.STEP.PING] < bs.STEP_INDEX[bs.STEP.PREFLIGHT_CAPGATE]
      < bs.STEP_INDEX[bs.STEP.CLAIM_ROLE],
      "%s < %s < %s" % (bs.STEP_INDEX[bs.STEP.PING],
                        bs.STEP_INDEX[bs.STEP.PREFLIGHT_CAPGATE],
                        bs.STEP_INDEX[bs.STEP.CLAIM_ROLE]))

# ── 2. 호출 위치: ② 판정 뒤 · ① 은 예약만 ──────────────────────────────────────
chain = inspect.getsource(bs._cmd_run_chain)
i_only = chain.find('"--only", "C28.self-correction"')
i_pingfail = chain.find("cys ping 유계 재시도 소진")
i_pre_branch = chain.find("_cap_remeasure = preflight")
check("2 재측정 호출이 ② ping 판정 **뒤**에 있다(종전엔 ① 안이었다)",
      i_only > 0 and i_pingfail > 0 and i_only > i_pingfail,
      "only=%d pingfail=%d" % (i_only, i_pingfail))
check("2b ① 갈래는 재측정을 **예약만** 한다(그 자리에서 실행 0)",
      i_pre_branch > 0 and i_pre_branch < i_pingfail
      and chain.count('"--only", "C28.self-correction"') == 1)
check("2c 재측정 기록은 전용 단계(STEP.PREFLIGHT_CAPGATE)로 남는다(①preflight 와 동명이의 0)",
      "STEP.PREFLIGHT_CAPGATE" in chain)

# ── 3. 판독 정본은 형제 모듈 API ────────────────────────────────────────────────
state_src = inspect.getsource(bs._capgate_unresolved_state)
check("3 부트 체인은 capgate_unresolved()/capgate_unresolved_path() 를 소비한다(리터럴 아님)",
      "capgate_unresolved(pack)" in state_src and "capgate_unresolved_path(pack)" in state_src
      and "_capgate_unresolved_state(PACK)" in chain
      and 'os.path.join(PACK, "state", "capgate-unresolved.json")' not in chain)

# ── 4·5. 조회 실패는 '해소됨' 이 아니다 + 음성 대조 ─────────────────────────────
d = tempfile.mkdtemp(prefix="capgate-marker-")
try:
    pack = os.path.join(d, "pack")
    state = os.path.join(pack, "state")
    os.makedirs(state)
    mark = pf.capgate_unresolved_path(pack)
    with open(mark, "w", encoding="utf-8") as f:
        f.write('{"state":"unknown"}')
    check("4a 정상 표식 → 미해소", pf.capgate_unresolved(pack)[0] is True)
    os.remove(mark)
    check("4b 증명된 부재 → 해소됨", pf.capgate_unresolved(pack)[0] is False)
    with open(mark, "w", encoding="utf-8") as f:
        f.write("{손상")
    check("4c 손상 표식 → 미해소(문서는 None)", pf.capgate_unresolved(pack) == (True, None))

    if os.geteuid() != 0:
        os.chmod(state, 0o000)
        try:
            unresolved = pf.capgate_unresolved(pack)[0]
            legacy = os.path.exists(mark)
            boot_unresolved, _p = bs._capgate_unresolved_state(pack)
        finally:
            os.chmod(state, 0o700)
        check("4d 조회 실패(EACCES)는 미해소다 — 있는데 못 보는 표식을 '해소됨' 으로 읽지 않는다",
              unresolved is True)
        check("4e 부트 체인도 같은 방향(래퍼 경유)", boot_unresolved is True)
        check("5 검출력: 같은 경로에서 종전 술어 os.path.exists 는 False(= '해소됨')를 낸다",
              legacy is False)
    else:
        print("SKIP 4d/4e/5 — root 로 실행 중이라 EACCES 를 만들 수 없다(측정 불가 · 통과 아님)")
finally:
    shutil.rmtree(d, ignore_errors=True)

print("\n=== FAIL %d ===" % len(fails))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("CAPGATE-REMEASURE-OK")
