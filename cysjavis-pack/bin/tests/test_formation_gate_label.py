#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_formation_gate_label.py — pending-resource 축 라벨·상태파일 바이트 규약 핀 (0.14.30 A2 · SURVEY B2·B3).

무엇을 막는가: 종전 ensure ④ 는 게이트 stdout(JSON `trips`)을 버리고 고정 문구
"곱셈 자원 예산 hard — 편성 대기" 를 상태파일·피드에 찍었다. 곱셈 축은 env `CYS_FORMATION_BUDGET`
없이는 무발화라 기본 설치에서 그 라벨은 **항상 거짓**이었고(원인은 servers/nodes/load_ratio/
context_pct 중 하나), 사후에 어느 축이 hard 였는지 알 방법이 없었다.

밀폐: `CYS_STATE_DIR` 을 임시 디렉터리로 고정해 `_state_root()` 를 격리한다(라이브 상태 무접촉).
데몬·서브프로세스 0 — **순수 함수만** 부른다(ensure 경로를 타지 않으므로 스폰 위험 0).

  ① `_gate_json` — 정상 dict · 파손 · 비-dict(list/None) 판정
  ② `_resource_detail` — 단일 축 · 복수 축(" · " 연결) · JSON 부재 · hard trips 부재의 4갈래.
     ★hard 가 아닌 trip(level=warn)은 축에 들어오지 않는다(오라벨 방지).
  ③ `_resource_feed_body` — 같은 축 표기 · 축 미상 시 괄호 없음
  ④ ★음성 대조(계측 타당성) — 모듈 어디에도 종전 고정 문구가 남아 있지 않다
  ⑤ `_gate_compact` — verdict/trips/warnings/measured 만 추리고 checks 는 버린다 · 빈 dict → None
  ⑥ `_write_state` 바이트 규약 — gate/held 가 비면 **키 자체가 없다**(종전 레인 바이트 동일) ·
     주면 그대로 남는다 · held 는 list 로 정규화
  ⑦ `HELD_FEED_TITLE` 유일 등재소 값 핀(사본 드리프트 차단)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 FORMATION-GATE-LABEL-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_formation_gate_label.py
"""
import json
import os
import shutil
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)

_ROOT = tempfile.mkdtemp()
os.environ["CYS_STATE_DIR"] = os.path.join(_ROOT, "state")

import javis_formation as F  # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def read_state(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


GATE_ONE = {"verdict": "hard", "checks": [{"metric": "nodes"}],
            "trips": [{"metric": "nodes", "value": 44, "hard": 42, "level": "hard"}],
            "warnings": [], "measured": {"nodes": 44}}
GATE_TWO = {"verdict": "hard",
            "trips": [{"metric": "nodes", "value": 44, "hard": 42, "level": "hard"},
                      {"metric": "servers", "value": 3, "hard": 3, "level": "hard"}]}
GATE_WARN_ONLY = {"verdict": "soft",
                  "trips": [{"metric": "load_ratio", "value": 2, "hard": 4, "level": "warn"}]}

try:
    # ① _gate_json
    check("1a 정상 JSON dict", F._gate_json('{"verdict":"hard"}') == {"verdict": "hard"})
    check("1b 파손 JSON → None", F._gate_json("{not json") is None)
    check("1c 비-dict(JSON list) → None", F._gate_json("[1,2]") is None)
    check("1d 빈 문자열·None → None", F._gate_json("") is None and F._gate_json(None) is None)

    # ② _resource_detail
    d1 = F._resource_detail(GATE_ONE)
    check("2a 단일 축 표기", d1 == "자원 게이트 hard — nodes 44/42 · 편성 대기", repr(d1))
    d2 = F._resource_detail(GATE_TWO)
    check("2b 복수 축은 ' · ' 연결",
          d2 == "자원 게이트 hard — nodes 44/42 · servers 3/3 · 편성 대기", repr(d2))
    d3 = F._resource_detail(None)
    check("2c JSON 부재는 '축 미상(JSON 없음)'", "축 미상(JSON 없음)" in d3, repr(d3))
    d4 = F._resource_detail({"verdict": "hard"})
    check("2d hard trips 부재는 '축 미상(hard trips 없음)'", "축 미상(hard trips 없음)" in d4, repr(d4))
    d5 = F._resource_detail(GATE_WARN_ONLY)
    check("2e warn trip 은 축이 아니다(오라벨 방지)",
          "축 미상" in d5 and "load_ratio" not in d5, repr(d5))

    # ③ _resource_feed_body
    b1 = F._resource_feed_body(GATE_ONE)
    check("3a 피드도 같은 축 표기", b1.startswith("자원 게이트 hard(nodes 44/42)"), repr(b1))
    b2 = F._resource_feed_body(None)
    check("3b 축 미상이면 괄호 없음", b2.startswith("자원 게이트 hard —") and "(" not in b2, repr(b2))

    # ④ ★음성 대조 — 거짓 라벨의 소멸
    with open(os.path.join(BIN, "javis_formation.py"), encoding="utf-8") as f:
        src = f.read()
    check("4 고정 문구 '곱셈 자원 예산 hard — 편성 대기' 0건",
          "곱셈 자원 예산 hard — 편성 대기" not in src)

    # ⑤ _gate_compact
    c1 = F._gate_compact(GATE_ONE)
    check("5a 네 키만 추린다(checks 제외)",
          set(c1) == {"verdict", "trips", "warnings", "measured"}, repr(c1))
    check("5b 네 키가 전부 없으면 None", F._gate_compact({"checks": []}) is None)
    check("5c 비-dict → None", F._gate_compact(None) is None and F._gate_compact([1]) is None)

    # ⑥ _write_state 바이트 규약
    p0 = F._write_state("sock-a", "pending-resource", "d", [])
    s0 = read_state(p0)
    check("6a gate/held 미지정 → 키 자체가 없다",
          "gate" not in s0 and "held" not in s0, repr(sorted(s0)))
    p1 = F._write_state("sock-b", "pending-resource", "d", [], gate=c1, held=["cso", "worker"])
    s1 = read_state(p1)
    check("6b gate 는 축약본 그대로", s1.get("gate") == c1, repr(s1.get("gate")))
    check("6c held 는 list 로 보존", s1.get("held") == ["cso", "worker"], repr(s1.get("held")))
    p2 = F._write_state("sock-c", "pending-resource", "d", [], gate=None, held=[])
    s2 = read_state(p2)
    check("6d 빈 값(None·[])도 키 없음(바이트 동일 규약)",
          "gate" not in s2 and "held" not in s2, repr(sorted(s2)))

    # ⑦ HELD_FEED_TITLE 유일 등재소
    check("7 held 피드 제목 핀", F.HELD_FEED_TITLE == "부서 팀 편성 보류(시도 원장)",
          repr(F.HELD_FEED_TITLE))
finally:
    shutil.rmtree(_ROOT, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("FORMATION-GATE-LABEL-OK")
sys.exit(0)
