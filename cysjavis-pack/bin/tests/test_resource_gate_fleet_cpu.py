#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_resource_gate_fleet_cpu.py — WP-7 N 소비자 무회귀 핀 (0.14.31).

무엇을 막는가: `javis_resource_gate` 에 신설 축(`fleet_cpu_ratio`)과 신설 라벨(`unavailable`)이
들어가면서 **판정 JSON 의 모양**이 바뀌었다. 그 JSON 을 읽는 소비자는 셋이다 —
`javis_bootstrap._resource_gate_decision`(exit 2 + nodes 단독 hard 예외) ·
`javis_formation._hard_trips/_axes_text/_resource_detail`(pending-resource 라벨) ·
`javis_completion_guard._soft_kind/_pick_trip`(warnings 완전일치). 이 검체는 **게이트가 실제로 낸
JSON**(픽스처 손작성 아님)을 그 소비자들에 먹여 판정이 안 바뀌었음을 증명한다.

밀폐: 게이트는 override 로만 부른다(라이브 ps·부서 소켓·boot-epoch 무접촉 · 데몬 0).
      `CYS_STATE_DIR`/`CYS_PACK_DIR` 은 임시 디렉터리로 고정.

핀 목록
  ① 게이트 종단 JSON 모양 — fleet_cpu_ratio 축 존재 · load_ratio 의 hard 는 None
  ② bootstrap: fleet_cpu 단독 hard 는 **hard-block**(nodes 과계수 예외로 새지 않는다)
  ③ bootstrap: nodes 단독 hard 의 과계수 예외는 **그대로 살아 있다**(회귀 핀)
  ④ bootstrap: nodes+fleet 동시 hard → other_hard 존재 → hard-block(예외 미적용)
  ⑤ formation: fleet_cpu hard 가 축 표기·detail 에 그대로 나온다(축 미상 아님)
  ⑥ ★unavailable 라벨은 축이 아니다 — trips 무오염 · detail 은 '축 미상(hard trips 없음)'
  ⑦ completion_guard: warnings 완전일치 계약 불변(신설 축이 warnings 를 늘리지 않는다)
  ⑧ ★음성 대조(계측 타당성): 좌석 전멸 형상(함대 CPU 0) 에서는 이 축이 복구를 막지 않는다
  ⑨ ★R1(봉인표 ③): 연속 hard 보류 상한이 넘으면 **소비자 종단에서** 차단이 풀린다
     (bootstrap hard-block → soft · formation pending-resource → proceed)
  ⑩ ★R1(비밀값): allow 에서는 `fleet_cpu_top` 이 비어 있고, hard 에서도 원시 명령줄이 없다 —
     `_gate_compact` 가 상태파일로 나르는 measured 에 인자가 실리지 않는다
  ⑪ ★R1(nan): 신설 실수 인자의 nan 은 EX_USAGE(64) 이고, 소비자는 그것을 hard 로 읽지 않는다
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 RESOURCE-GATE-FLEET-CPU-OK.
실행 규약(CI 동형):
  CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" python3 bin/tests/test_resource_gate_fleet_cpu.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
GATE = os.path.join(BIN, "javis_resource_gate.py")

_ROOT = tempfile.mkdtemp(prefix="fleetcpu-")
os.environ["CYS_STATE_DIR"] = os.path.join(_ROOT, "state")
os.environ.setdefault("CYS_PACK_DIR", os.path.join(_ROOT, "pack"))
os.environ.pop("CYS_SOCKET", None)          # boot-epoch 경로가 라이브를 가리키지 않게
sys.path.insert(0, BIN)

import javis_bootstrap as B      # noqa: E402
import javis_formation as F      # noqa: E402

fails = []
ROSTER = '{"active":0,"seats":0,"errors":[],"depts":[]}'


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def gate(*extra):
    """게이트를 override 만으로 호출 → (rc, json). 계약 채널은 stdout 하나다."""
    argv = [sys.executable, GATE, "check", "--json",
            "--dept-roster-override", ROSTER,
            "--boot-elapsed-override", "99999"] + list(extra)
    r = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=60)
    try:
        return r.returncode, json.loads(r.stdout.strip())
    except ValueError as e:
        fails.append("gate JSON 파싱 실패(%r): %s / %s" % (extra, e, r.stderr[-200:]))
        return r.returncode, {}


def axis(doc, metric):
    return next((c for c in (doc.get("checks") or []) if c.get("metric") == metric), {})


try:
    base = ["--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0"]

    # ① 종단 JSON 모양
    rc, ok_doc = gate(*(base + ["--fleet-cpu-override", "0.0"]))
    check("1a allow 경로 rc 0", rc == 0, repr(rc))
    check("1b fleet_cpu_ratio 축 존재", axis(ok_doc, "fleet_cpu_ratio").get("level") == "ok",
          repr(axis(ok_doc, "fleet_cpu_ratio")))
    check("1c load_ratio 는 soft 전용(hard None)",
          axis(ok_doc, "load_ratio").get("hard") is None, repr(axis(ok_doc, "load_ratio")))

    # ② fleet 단독 hard → bootstrap 은 hard-block. 라이브 노드 수가 아무리 작아도 예외 아님.
    rc, fleet_hard = gate(*(base + ["--fleet-cpu-override", "1.2"]))
    check("2a fleet 단독 hard rc 2", rc == 2, repr(rc))
    v, why = B._resource_gate_decision(2, fleet_hard, 0)
    check("2b bootstrap hard-block(과계수 예외로 새지 않음)", v == "hard-block", "%s / %s" % (v, why))
    check("2c 사유에 축 이름이 남는다", "fleet_cpu_ratio" in why, why)

    # ③ nodes 단독 hard 의 과계수 예외는 그대로(회귀 핀)
    rc, nodes_hard = gate(*["--servers-override", "0", "--nodes-override", "99",
                            "--load-override", "0.0", "--fleet-cpu-override", "0.0"])
    check("3a nodes 단독 hard rc 2", rc == 2, repr(rc))
    v, why = B._resource_gate_decision(2, nodes_hard, 1)
    check("3b nodes 단독 + 라이브 1 → hard-overcount 유지", v == "hard-overcount",
          "%s / %s" % (v, why))

    # ④ nodes + fleet 동시 hard → other_hard 존재 → 예외 미적용
    rc, both = gate(*["--servers-override", "0", "--nodes-override", "99",
                      "--load-override", "0.0", "--fleet-cpu-override", "1.2"])
    v, why = B._resource_gate_decision(2, both, 1)
    check("4 nodes+fleet 동시 hard → hard-block", v == "hard-block", "%s / %s" % (v, why))

    # ⑤ formation 라벨 — 축 미상으로 접히지 않는다
    trips = F._hard_trips(fleet_hard)
    check("5a _hard_trips 가 fleet 축을 집는다",
          [t["metric"] for t in trips] == ["fleet_cpu_ratio"], repr(trips))
    check("5b _axes_text 표기", F._axes_text(fleet_hard) == "fleet_cpu_ratio 1.2/1.0",
          repr(F._axes_text(fleet_hard)))
    det = F._resource_detail(fleet_hard)
    check("5c _resource_detail 에 축이 그대로", det == "자원 게이트 hard — fleet_cpu_ratio 1.2/1.0 · 편성 대기",
          repr(det))
    check("5d 피드 본문도 같은 축", F._resource_feed_body(fleet_hard).startswith(
        "자원 게이트 hard(fleet_cpu_ratio 1.2/1.0)"), repr(F._resource_feed_body(fleet_hard)))
    check("5e _gate_compact 는 measured 를 보존(신설 키 포함)",
          "fleet_cpu_ratio" in ((F._gate_compact(fleet_hard) or {}).get("measured") or {}),
          repr(sorted((F._gate_compact(fleet_hard) or {}).get("measured") or {})))
    check("5f exit 분기 불변", (F._resource_branch(2), F._resource_branch(1),
                             F._resource_branch(64)) == ("block", "proceed", "retry"))

    # ⑥ unavailable 라벨은 축이 아니다 — ps 부재 형상을 게이트 내부에서 만들 수 없으므로
    #    게이트가 낸 실제 dict 를 그대로 쓰되 fleet 축만 unavailable 로 치환해 소비자에 먹인다.
    unavail = json.loads(json.dumps(ok_doc))
    for c in unavail["checks"]:
        if c["metric"] == "fleet_cpu_ratio":
            c.update({"value": None, "level": "unavailable", "reason": "absent"})
    unavail["trips"] = [c for c in unavail["checks"] if c["level"] in ("soft", "hard")]
    check("6a unavailable 은 trips 에 없다", unavail["trips"] == [], repr(unavail["trips"]))
    check("6b formation 은 축 미상으로 정직 표기",
          "축 미상(hard trips 없음)" in F._resource_detail(unavail), F._resource_detail(unavail))
    check("6c bootstrap 은 unavailable 을 hard 로 읽지 않는다",
          B._resource_gate_decision(0, unavail, 0)[0] == "allow")

    # ⑦ completion_guard warnings 완전일치 계약 — 신설 축이 warnings 를 늘리지 않는다
    import javis_completion_guard as G       # noqa: E402
    check("7a allow 경로 warnings 는 종전 그대로",
          ok_doc.get("warnings") == ["context_unmeasured"], repr(ok_doc.get("warnings")))
    check("7b _soft_kind 분기 불변(proceed_unmeasured)",
          G._soft_kind(None, ok_doc.get("warnings")) == "proceed_unmeasured")
    check("7c _pick_trip 은 unavailable 을 고르지 않는다",
          G._pick_trip(unavail["trips"], "hard") is None)

    # ⑧ ★음성 대조(봉인표 ③): 좌석이 전멸하면 그 좌석의 CPU 는 0 이다 — 그 형상에서 이 축은
    #    복구를 막지 않는다. 이 축이 hard 이려면 '살아 있는 다른 함대 프로세스가 전 코어를
    #    태우는 중' 이라는 사실이 참이어야 한다(막는 근거가 실재해야 한다).
    rc, wiped = gate(*(base + ["--fleet-cpu-override", "0.0"]))
    check("8a 좌석 전멸(함대 CPU 0) 형상은 allow", rc == 0, repr(rc))
    check("8b 그 형상에서 formation 은 pending-resource 가 아니다",
          F._resource_branch(rc) == "proceed", F._resource_branch(rc))

    # ⑨ ★봉인표 ③ 종단: 같은 hard 값이라도 **연속 보류가 상한을 넘으면** 차단이 풀린다.
    #    이 축은 호스트 전체 함대 CPU 를 재면서 각 부서의 복구 허가에 쓰이므로, 상한이 없으면
    #    다른 부서의 부하가 전멸 부서의 복구를 무기한 막는다(리뷰 blocking).
    rc_hold0, doc_hold0 = gate(*(base + ["--fleet-cpu-override", "1.5",
                                         "--fleet-cpu-hold-override", "0"]))
    check("9a 보류 시작 시점에는 여전히 hard(축이 사문화되지 않았다)", rc_hold0 == 2, repr(rc_hold0))
    check("9b 그때 bootstrap 은 hard-block",
          B._resource_gate_decision(2, doc_hold0, 0)[0] == "hard-block")
    rc_hold1, doc_hold1 = gate(*(base + ["--fleet-cpu-override", "1.5",
                                         "--fleet-cpu-hold-override", "100000"]))
    check("9c 상한 초과 후에는 soft(exit 1) — 무기한 보류 없음", rc_hold1 == 1, repr(rc_hold1))
    check("9d bootstrap 은 그것을 soft 로 읽는다",
          B._resource_gate_decision(rc_hold1, doc_hold1, 0)[0] == "soft")
    check("9e formation 은 진행한다(pending-resource 아님)",
          F._resource_branch(rc_hold1) == "proceed", F._resource_branch(rc_hold1))
    check("9f 완화 사실이 축에 남는다(조용한 완화 금지)",
          axis(doc_hold1, "fleet_cpu_ratio").get("hold_expired") is True,
          repr(axis(doc_hold1, "fleet_cpu_ratio")))
    check("9g 완화가 warnings 를 늘리지 않는다(completion_guard 완전일치 계약)",
          doc_hold1.get("warnings") == ["context_unmeasured"], repr(doc_hold1.get("warnings")))

    # ⑩ ★비밀값 전파: 정상 판정의 measured 에 진단 행이 아예 없다(boot-last.json·상태파일 영속 차단).
    check("10a allow 의 fleet_cpu_top 은 빈 목록",
          (ok_doc.get("measured") or {}).get("fleet_cpu_top") == [],
          repr((ok_doc.get("measured") or {}).get("fleet_cpu_top")))
    compact = F._gate_compact(fleet_hard) or {}
    check("10b _gate_compact 가 나르는 measured 에 원시 명령줄 키가 없다",
          all("cmd" not in (r or {}) for r in
              ((compact.get("measured") or {}).get("fleet_cpu_top") or [])),
          repr((compact.get("measured") or {}).get("fleet_cpu_top")))

    # ⑪ ★nan: 조용한 allow 도, 비표준 JSON 도 아니고 **사용오류**다.
    r_nan = subprocess.run([sys.executable, GATE, "check", "--json",
                            "--dept-roster-override", ROSTER,
                            "--boot-elapsed-override", "99999"] + base
                           + ["--fleet-cpu-override", "nan"],
                           capture_output=True, text=True, encoding="utf-8", timeout=60)
    check("11a --fleet-cpu-override nan 은 EX_USAGE(64)", r_nan.returncode == 64,
          repr(r_nan.returncode))
    check("11b 계약 채널(stdout)에 비표준 JSON 토큰이 안 나간다",
          "NaN" not in r_nan.stdout and "Infinity" not in r_nan.stdout, repr(r_nan.stdout[:120]))
    check("11c bootstrap 은 64 를 hard 가 아니라 usage-error 로 읽는다",
          B._resource_gate_decision(64, None, 0)[0] == "usage-error")
finally:
    shutil.rmtree(_ROOT, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("RESOURCE-GATE-FLEET-CPU-OK")
sys.exit(0)
