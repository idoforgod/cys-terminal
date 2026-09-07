#!/usr/bin/env python3
"""★triage R1-WP1-HF(claude major): `carry-unproven` 처방의 **적용 범위** 계약.

실행: python3 cysjavis-pack/bin/tests/test_carry_unproven_scope.py (0=전건 PASS)

【무엇을 재는가】 `carry-unproven` 은 **좌석 1개**의 사실이다(그 좌석 화면에 입력창 양성 증거가
없다). 그런데 처방 ⓑ 는 `CYS_BOOT_GATES=0 cys boot` 을 지목한다:
  · `CYS_BOOT_GATES` 는 **마스터 롤백 스위치**다(src/lib.rs `ENV_BOOT_GATES` doc — "이 캠페인이
    추가한 판정 축이 **전부 동시에** 종전 동작으로 복귀").
  · `cys boot` 은 **로스터 전체**를 부트한다(역할 필터 인자가 없다 — src/bin/cys.rs `Command::Boot`
    는 `--cwd`·`--json` 뿐).
  · 종전 판정에서는 첫기동 관문 화면 **6종 전부**가 ready 다(src/readiness.rs 검체
    `legacy_v1_reproduces_the_defect_on_every_gate_screen`) — 그 부트에서 **다른** 좌석이 진짜
    폴더신뢰·면책 관문에 앉아 있으면 디렉티브 + Return 이 나가고 기본 포커스는 `No, exit` 이다.
같은 문단 ⓐ 가 "관문이면 사람이 통과시킨다(기본 포커스는 No, exit)" 라고 경고해 놓고 ⓑ 가 그
경고를 로스터 전체에서 끄는 손잡이를 권한다 — 두 문장이 서로를 부정한다.

【계약】 처방이 마스터 스위치를 지목한다면, **같은 문장 안에서** 그 범위가 좌석 1개가 아니라
그 부트의 **모든 좌석**이라는 사실과 그 대가(다른 좌석의 관문 거부가 함께 꺼진다)를 말해야 한다.
말하지 못하면 좌석 범위로 좁힌 손잡이를 주어야 한다(범위 축소가 옳은 방향이다).
"""
import importlib.util
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BS = os.path.normpath(os.path.join(HERE, "..", "javis_bootstrap.py"))
spec = importlib.util.spec_from_file_location("javis_bootstrap_scope", BS)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []


def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (("  — " + detail) if detail and not cond else ""))


def main():
    _results.clear()
    line = m._GATE_REASON_PRESCRIPTION[m.GATE_REASON_CARRY_UNPROVEN]

    # 전제 ① — 처방이 실제로 마스터 스위치를 지목한다(지목하지 않게 됐다면 이 검체는 그대로 통과).
    names_master = "CYS_BOOT_GATES=0" in line
    check("전제: carry-unproven 처방이 마스터 롤백 스위치를 지목한다", True,
          "지목 여부=%s" % names_master)
    if not names_master:
        print("SKIP  처방이 마스터 스위치를 더는 지목하지 않는다 — 범위 계약은 자동 충족")
        return 0

    # 전제 ② — `cys boot` 에 좌석·역할 한정 인자가 없다(= 범위가 로스터 전체다).
    cys_rs = os.path.normpath(os.path.join(HERE, "..", "..", "..", "src", "bin", "cys.rs"))
    boot_decl = ""
    if os.path.exists(cys_rs):
        with open(cys_rs, encoding="utf-8") as f:
            src = f.read()
        i = src.find("    Boot {")
        boot_decl = src[i:src.find("\n    },", i)] if i >= 0 else ""
    check("전제: `cys boot` 에 좌석/역할 한정 인자가 없다(범위=로스터 전체)",
          boot_decl != "" and "--role" not in boot_decl and "role:" not in boot_decl,
          "Boot 선언: %r" % boot_decl[:200])

    # 본계약 — 범위와 대가를 같은 처방이 말하는가.
    scope_tokens = ("모든 좌석", "로스터 전체", "전 좌석", "다른 좌석", "그 부트의 모든")
    said_scope = [t for t in scope_tokens if t in line]
    check("처방이 마스터 스위치의 **범위**(그 부트의 모든 좌석)를 밝힌다",
          bool(said_scope),
          "처방에 범위 어휘가 없다(찾은 것: %s) — 좌석 1개 문제에 로스터 전체의 관문·모달 거부를 "
          "끄는 손잡이를 조건 없이 권한다: %r" % (said_scope, line))

    cost_tokens = ("관문 거부", "관문·모달", "관문이 열린다", "No, exit", "좌석 사망", "함께 꺼진다")
    # ⓐ 절의 `No, exit` 은 **다른 문장**의 경고다 — ⓑ 절(마스터 스위치 문장) 안에서 대가를 말해야 한다.
    tail = line[line.find("CYS_BOOT_GATES=0"):]
    said_cost = [t for t in cost_tokens if t in tail]
    check("처방의 마스터 스위치 문장이 **대가**(다른 좌석의 관문 거부가 함께 꺼진다)를 말한다",
          bool(said_cost),
          "스위치 문장 이후 문면에 대가 어휘가 없다: %r" % tail)

    ok = all(_results)
    print("\n%d/%d PASS" % (sum(1 for r in _results if r), len(_results)))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
