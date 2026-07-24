#!/usr/bin/env python3
"""test_budget.py — W6 곱셈 자원 예산 계약 핀 (T0 RED-first · 설계 DD-3/W6).

`javis_resource_gate.py` 는 존재하나 **곱셈 편성 예산(부서 수 × 편성 크기)** API 가 없다 → RED.
T4 가 `CYS_FORMATION_BUDGET` 곱셈 검사를 추가해 GREEN 화.

계약(구현 후 통과 기준):
  1) formation_budget_check(live_agents, formation_size, dept_count, budget) 존재.
  2) projected = live + formation_size(또는 dept_count×formation_size) 가 budget 초과 → 차단(pending-resource).
     차단은 역할 중립(CEO·부서장 동일 물리법칙) · hard-fail 아님(대기·자동 재시도).
  3) budget 이하 → 통과.
  4) CYS_FORMATION_BUDGET env(또는 인자)로 한도 주입 가능.

실행: python3 test_budget.py   (exit 0=PASS / 1=RED)
"""
import importlib.util
import os
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_resource_gate.py"))
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
    spec = importlib.util.spec_from_file_location("javis_resource_gate", MODULE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main():
    m = load()
    if m is None:
        check("0 javis_resource_gate 로드", False, "모듈 부재(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    fbc = getattr(m, "formation_budget_check", None)
    if not callable(fbc):
        for n in ("1 formation_budget_check 존재",
                  "2 초과→차단(pending-resource·역할중립)", "3 이하→통과",
                  "4 CYS_FORMATION_BUDGET 주입"):
            check(n, False, "곱셈 편성 예산 API 미구현 — T4 대상(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    check("1 formation_budget_check 존재", True)
    try:
        # 8부서 × 5노드 = 40, 한도 20 → 차단
        over = fbc(live_agents=5, formation_size=5, dept_count=8, budget=20)
        check("2 초과→차단(pending-resource)",
              (getattr(over, "blocked", None) is True) or ("pending" in str(over).lower()),
              "got=%r" % (over,))
        under = fbc(live_agents=2, formation_size=5, dept_count=1, budget=20)
        check("3 이하→통과",
              (getattr(under, "blocked", None) is False) or ("ok" in str(under).lower()
                                                             or "pass" in str(under).lower()),
              "got=%r" % (under,))
    except Exception as e:
        check("2 초과→차단(pending-resource)", False, "호출 실패: %s" % e)
        check("3 이하→통과", False, "호출 실패: %s" % e)

    check("4 CYS_FORMATION_BUDGET 주입 경로",
          hasattr(m, "FORMATION_BUDGET_ENV") or "CYS_FORMATION_BUDGET" in (open(MODULE, encoding="utf-8").read()),
          "env 배선 미구현(RED)")

    # 6. ★리뷰어1 fix#4: CYS_FORMATION_BUDGET env 음수/0/비숫자/공백 각 거동(오타 env=기본 20 폴백·
    #    조직 기동 우발 전면차단 방지). 유효 양수만 수용. 각 케이스 실제 재현.
    val = getattr(m, "_formation_budget_value", None)
    env_name = getattr(m, "FORMATION_BUDGET_ENV", "CYS_FORMATION_BUDGET")
    default = getattr(m, "FORMATION_BUDGET_DEFAULT", 20)
    if callable(val):
        import os as _os
        saved = _os.environ.get(env_name)

        def _with(raw):
            if raw is None:
                _os.environ.pop(env_name, None)
            else:
                _os.environ[env_name] = raw
            try:
                return val()   # explicit=None → env 경로
            finally:
                if saved is None:
                    _os.environ.pop(env_name, None)
                else:
                    _os.environ[env_name] = saved

        cases = {"음수": ("-5", default), "0": ("0", default),
                 "비숫자": ("abc", default), "공백": ("   ", default),
                 "유효양수(공백포함)": (" 30 ", 30)}
        allok = True
        detail = []
        for label, (raw, want) in cases.items():
            got = _with(raw)
            ok = (got == want)
            allok = allok and ok
            detail.append("%s=%r→%s(want %s)%s" % (label, raw, got, want, "" if ok else "✗"))
        check("6 budget env 음수/0/비숫자/공백/유효 각 거동(폴백 20)", allok, " ".join(detail))
    else:
        check("6 budget env 음수/0/비숫자/공백/유효 각 거동", False, "_formation_budget_value 미노출")

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
