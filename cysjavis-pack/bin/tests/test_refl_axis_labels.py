#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_axis_labels.py — 성찰 R4 N16·N19: `ps` 거부 문면 분류와 hard 축 이름 문면.

무엇을 막는가
  ① N16 — BusyBox 는 `ps: bad -o argument 'pcpu'` 로 플래그를 거부하는데 종전 패턴은
     `bad option` 만 알아서 그 문구가 `"failed"`(측정 실패)로 접혔다. 그러면 Alpine/BusyBox 에서
     **매 호출** `measure_errors: fleet_cpu(ps)` → 최소 soft → `javis_completion_guard._soft_kind`
     가 `skip_soft`(SKIPPED_RESOURCE) = **완료 검증 영구 skip**. 거부 문면은 '이 ps 로는 이 축을
     못 잰다'는 플랫폼 능력 사실이지 측정 실패가 아니다 → `unsupported` 로 접혀야 한다.
  ② N19 — `javis_formation.py` 문면이 `load_ratio` 를 hard 축으로, `fleet_cpu_ratio` 를 없는 축으로
     적었다. 0.14.31 에서 `load_ratio` 는 hard 가 제거됐고 새 hard 축은 `fleet_cpu_ratio` 다.
     문면은 상태파일 `gate` 키·피드를 읽는 사람이 원인 축을 짚는 자리라 **존재하지 않는 축**을
     가리키면 안 된다. 이 검체는 문자열을 손으로 적지 않고 **게이트가 실제로 hard 를 낼 수 있는
     축 목록**을 `evaluate()` 에서 뽑아 문면과 대조한다(문서-코드 결박).

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-AXIS-LABELS-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_axis_labels.py
"""
import os
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


import javis_resource_gate as G          # noqa: E402
import javis_formation as F              # noqa: E402

# ── ① N16: 거부 문면 분류 ────────────────────────────────────────────────────
# 기존 4문구(회귀 핀 — 하나라도 빠지면 그 플랫폼이 상시 soft 로 돌아간다)
LEGACY = [
    "ps: illegal option -- a",                       # macOS/BSD
    "ps: unknown option -- axo",                     # 일부 procps
    "ps: invalid option -- 'x'",                     # GNU procps
    "ps: unrecognized option '--sort'",              # GNU long
    "ps: bad option",                                # 구 BusyBox 축약
]
for phrase in LEGACY:
    check("1a 기존 거부 문면 유지 %r" % phrase,
          bool(G._PS_FLAG_REJECT_RE.search(phrase)))

# ★신설 양성군 — BusyBox 의 **실제** 문구(플래그 이름이 사이에 낀다)
BUSYBOX = [
    "ps: bad -o argument 'pcpu'",
    "ps: bad -o argument",
    "ps: bad --sort argument 'x'",
]
for phrase in BUSYBOX:
    check("1b BusyBox 실제 문구가 unsupported 로 분류 %r" % phrase,
          bool(G._PS_FLAG_REJECT_RE.search(phrase)))

# ★음성 대조 — '측정 실패'는 여전히 측정 실패다(전부 unsupported 로 접으면 조용한 allow 가 된다)
NEGATIVE = [
    "ps: cannot open /proc: Permission denied",
    "ps: error: could not read procfs",
    "Killed",
    "",
]
for phrase in NEGATIVE:
    check("1c 측정 실패 문구는 unsupported 아님 %r" % phrase,
          not G._PS_FLAG_REJECT_RE.search(phrase))

# ★통합 — PATH 선두의 목 `ps` 가 BusyBox 처럼 거부하면 `_ps_cpu_lines` 가 unsupported 를 낸다.
root = tempfile.mkdtemp()
mockbin = os.path.join(root, "mockbin")
os.makedirs(mockbin)
mp = os.path.join(mockbin, "ps")
with open(mp, "w", encoding="utf-8", newline="\n") as f:
    f.write("#!/bin/sh\nprintf \"ps: bad -o argument 'pcpu'\\n\" >&2\nexit 1\n")
os.chmod(mp, 0o755)
_saved_path = os.environ.get("PATH", "")
os.environ["PATH"] = mockbin + os.pathsep + _saved_path
try:
    lines, reason = G._ps_cpu_lines()
    check("1d 목 BusyBox ps → reason=unsupported", reason == "unsupported",
          repr((lines, reason)))
finally:
    os.environ["PATH"] = _saved_path

# `unsupported` 는 measure_errors 로 가지 않는다(exit 계약 불변) — 그 규칙의 위치 핀.
src = open(os.path.join(BIN, "javis_resource_gate.py"), encoding="utf-8").read()
check("1e unsupported 는 measure_errors 제외 목록에 있다",
      '"ok", "override", "absent", "unsupported"' in src)


# ── ② N19: hard 축 이름 문면 ─────────────────────────────────────────────────
class _A(object):
    """evaluate 가 읽는 인자 표면만 갖춘 최소 네임스페이스."""
    servers_soft, servers_hard = 1, 2
    nodes_soft = 1
    load_soft_ratio = 1.0
    context_soft, context_hard = 50, 80
    fleet_cpu_soft, fleet_cpu_hard = 0.5, 0.8
    rate_check = False
    rate_override = None
    formation_size = None


_m = {"servers": 0, "nodes": 0, "nodes_hard_effective": 9, "fleet_cpu_ratio": 0.1,
      "load_ratio": 0.1, "context_pct": 1, "measure_errors": [], "active_depts": 0}
_worst, _checks = G.evaluate(_m, _A())
HARD_AXES = sorted(c["metric"] for c in _checks if c.get("hard") is not None)
SOFT_ONLY = sorted(c["metric"] for c in _checks if c.get("hard") is None)

check("2a 게이트의 hard 축 집합", HARD_AXES == ["context_pct", "fleet_cpu_ratio", "nodes", "servers"],
      repr(HARD_AXES))
check("2b load_ratio 는 soft 전용", SOFT_ONLY == ["load_ratio"], repr(SOFT_ONLY))

fsrc = open(os.path.join(BIN, "javis_formation.py"), encoding="utf-8").read()
# 문면 세 자리(모듈 docstring · ④ 절 주석 · `_resource_gate_json` docstring)를 한꺼번에 본다:
#   존재하지 않는 축을 hard 원인으로 적지 않고, 실제 hard 축(fleet_cpu_ratio)은 적혀 있어야 한다.
BAD = "servers/nodes/load_ratio/context_pct"
check("2c 존재하지 않는 hard 축 목록이 문면에서 사라졌다", BAD not in fsrc,
      "여전히 %r 를 hard 축으로 적는다" % BAD)
check("2d fleet_cpu_ratio 가 문면에 있다", fsrc.count("fleet_cpu_ratio") >= 3,
      "등장 %d회" % fsrc.count("fleet_cpu_ratio"))
check("2e load_ratio 언급은 'soft 전용' 이라는 사실과 함께만",
      all("soft 전용" in ln for ln in fsrc.splitlines() if "load_ratio" in ln),
      repr([ln.strip() for ln in fsrc.splitlines() if "load_ratio" in ln]))
# 축 이름 자체가 게이트의 것과 같은지(오타 결박) — formation 이 적은 이름이 전부 실존해야 한다.
_known = set(HARD_AXES) | set(SOFT_ONLY) | {"formation_budget"}
for axis in ("servers", "nodes", "fleet_cpu_ratio", "context_pct", "formation_budget"):
    check("2f 문면 축 %s 는 실존" % axis, axis in _known)

print("REFL-AXIS-LABELS-OK" if not fails else "FAILED: %s" % ", ".join(fails))
sys.exit(1 if fails else 0)
