#!/usr/bin/env python3
"""test_parity_paths.py — C3 두 경로 동등성 e2e 계약 핀 (T0 RED-first · 설계 §5·DD-3).

버튼 경로(cys-dept allocate→formation ensure)와 수동 경로("너는 마스터다"→javis_bootstrap→
동일 ensure)가 **동일 로스터 + 동일 주입 디렉티브 해시 + C03 동일** 을 내는지 mock-agent 로 검증.

현 코드에는 `javis_formation.py`(두 경로가 수렴하는 ensure)가 없으므로 → RED. T2/T5 가 GREEN 화.
품질 앵커(R2-4): 노드 수가 아니라 **주입된 지침의 온전함(해시 대조)** 이 판정 기준이다.

계약(구현 후 통과 기준):
  1) 두 경로 모두 formation.ensure 로 수렴(동일 함수) — 구조적 동등성.
  2) 두 경로의 결과 로스터(역할 집합)가 동일.
  3) 두 경로가 각 역할 pane 에 주입한 디렉티브의 sha256 이 동일(mock-agent 가 기록).
  4) 두 경로 모두 C03(표준 핀 잔존) PASS.

실행: python3 test_parity_paths.py   (exit 0=PASS / 1=RED)
"""
import importlib.util
import os
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_formation.py"))
FIXTURE = os.path.join(SELF, "fixtures", "mock-agent.sh")
fails = []
_total = [0]


def check(name, cond, detail=""):
    _total[0] += 1
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def load():
    if not os.path.exists(MODULE):
        return None
    spec = importlib.util.spec_from_file_location("javis_formation", MODULE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main():
    check("0 mock-agent 픽스처 존재", os.path.exists(FIXTURE), FIXTURE)

    m = load()
    if m is None or not callable(getattr(m, "ensure", None)):
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "javis_formation.ensure 미구현 — 두 경로 수렴점 부재(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    # 두 경로 시뮬레이션 API 계약 — plan_roster(path) 가 경로별 계획 로스터를 반환한다고 가정.
    plan = getattr(m, "plan_roster", None)
    if not callable(plan):
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "plan_roster(path) 계약 미구현(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    try:
        button = plan(path="button")
        manual = plan(path="manual")
        check("1 두 경로 formation.ensure 수렴",
              getattr(button, "ensure_fn", None) == getattr(manual, "ensure_fn", None),
              "수렴 함수 불일치")
        check("2 로스터 동일",
              set(button.roles) == set(manual.roles), "%r vs %r" % (button.roles, manual.roles))
        check("3 주입 디렉티브 해시 동일",
              button.directive_hashes == manual.directive_hashes,
              "해시 불일치(노드 수 아닌 지침 온전함)")
        check("4 두 경로 C03 PASS",
              button.c03_pass and manual.c03_pass, "C03 불통")
    except Exception as e:
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "동등성 API 호출 실패: %s" % e)

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
