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

    # 7. ★이중계상 제거(2026-07-26) — 라이브 실측 재현.
    #    live_agents(=ps 측정 nodes)는 소켓 스코프가 없어 **이미 기동된 부서의 노드를 전부 포함**한다.
    #    구 산식은 거기에 dept_count×formation_size 를 통째로 더해 같은 노드를 두 번 셌다
    #    (실측 2026-07-26: nodes=21 · depts=2 · size=5 → projected 31 인데 실존 노드는 21).
    #    투영은 "앞으로 **새로** 뜰 노드"만 더해야 한다 → 기편성분(formed_agents)을 차감.
    try:
        before = fbc(live_agents=21, formation_size=5, dept_count=2, budget=20)  # formed=0 = 구 산식
        after = fbc(live_agents=21, formation_size=5, dept_count=2, budget=20, formed_agents=10)
        check("7a 구 산식 재현(기편성 미차감) = 21 + 2×5 = 31",
              getattr(before, "projected", None) == 31, "got=%r" % (before,))
        check("7b 이중계상 제거 후 = 21 + max(0, 10−10) = 21(실존 노드와 일치)",
              getattr(after, "projected", None) == 21, "got=%r" % (after,))
        check("7c 투영은 실측 노드 미만으로 내려가지 않는다(과다차감 하한)",
              fbc(live_agents=21, formation_size=5, dept_count=2, budget=99,
                  formed_agents=999).projected == 21, "하한 붕괴")
        check("7d 미편성 부서는 여전히 곱셈 투영(예산 축 무력화 아님)",
              fbc(live_agents=5, formation_size=5, dept_count=8, budget=20,
                  formed_agents=5).projected == 40 and
              fbc(live_agents=5, formation_size=5, dept_count=8, budget=20,
                  formed_agents=5).blocked is True, "미편성 투영 소실")
    except Exception as e:
        check("7 이중계상 제거 전후 비교", False, "호출 실패: %s" % e)

    # 8. _formed_agent_count — 편성 상태 파일(roles_booted) 합산·유령/base 레인 제외.
    fac = getattr(m, "_formed_agent_count", None)
    if callable(fac):
        import json as _json
        import os as _os
        import shutil as _shutil
        import tempfile as _tempfile
        td = _tempfile.mkdtemp(prefix="fmbud-")
        saved = _os.environ.get("CYS_STATE_DIR")
        _os.environ["CYS_STATE_DIR"] = td
        try:
            fdir = _os.path.join(td, "formation")
            _os.makedirs(fdir)
            live_sock = _os.path.join(td, "dept-1.sock")
            open(live_sock, "w").close()                      # 살아있는 부서 소켓
            dead_sock = _os.path.join(td, "dept-gone.sock")    # 철거된 부서(파일 없음)

            def w(name, obj):
                with open(_os.path.join(fdir, name), "w", encoding="utf-8") as f:
                    _json.dump(obj, f)

            w("live.json", {"socket": live_sock, "roles_booted": ["master", "cso", "worker"]})
            w("dead.json", {"socket": dead_sock, "roles_booted": ["master", "cso"]})
            w("base.json", {"socket": "", "roles_booted": ["master", "cso"]})
            w("broken.json", {"socket": live_sock, "roles_booted": "not-a-list"})
            check("8a 살아있는 부서만 계수(철거·base 레인 제외)", fac(5) == 3, "got=%r" % (fac(5),))
            check("8b 레인당 formation_size 상한(과다차감 방지)", fac(2) == 2, "got=%r" % (fac(2),))
        except Exception as e:
            check("8 _formed_agent_count 상태파일 계수", False, "실패: %s" % e)
        finally:
            if saved is None:
                _os.environ.pop("CYS_STATE_DIR", None)
            else:
                _os.environ["CYS_STATE_DIR"] = saved
            _shutil.rmtree(td, ignore_errors=True)
    else:
        check("8 _formed_agent_count 존재", False, "이중계상 차감 원천 미구현")

    # 9. ★두 임계 정합(2026-07-26) — 편성 예산이 노드 상한과 **동형으로 동적**인가.
    #    배경(자기모순): 같은 파일의 노드 상한은 동적(max(18, 12+부서수×5))인데 편성 예산만 정적 20 이면
    #    부서 2개 머신(nodes_hard=22, 정상 노드 21)에서 **노드 상한은 허용하는데 편성 예산은 금지**한다
    #    → 편성이 pending-resource 로 영구 대기(배너 소멸 신호 미발행). 예산 ≥ 노드 상한이 성립해야 한다.
    bud = getattr(m, "_formation_budget_value", None)
    nhe = getattr(m, "_nodes_hard_effective", None)
    if callable(bud) and callable(nhe):
        import os as _os
        saved = _os.environ.pop(getattr(m, "FORMATION_BUDGET_ENV", "CYS_FORMATION_BUDGET"), None)
        try:
            rows = []
            allok = True
            for d in (0, 1, 2, 3, 8):
                b, h = bud(None, dept_count=d), nhe(d)
                ok = b >= h
                allok = allok and ok
                rows.append("depts=%d budget=%d nodes_hard=%d%s" % (d, b, h, "" if ok else "✗"))
            check("9a 예산 ≥ 노드 상한(부서 0/1/2/3/8 전 구간 정합)", allok, " ".join(rows))
            check("9b 부서 0~1 은 종전 floor 20 유지(무회귀)",
                  bud(None, dept_count=0) == 20 and bud(None, dept_count=1) == 20,
                  "d0=%r d1=%r" % (bud(None, dept_count=0), bud(None, dept_count=1)))
            check("9c 부서 2 에서 동적 상향(20→22 = nodes_hard 와 동일 산식)",
                  bud(None, dept_count=2) == 22 and nhe(2) == 22,
                  "budget=%r nodes_hard=%r" % (bud(None, dept_count=2), nhe(2)))
            # 라이브 실측 재현: nodes=21 · depts=2 · size=5 · 기편성=10 → projected 21 ≤ budget 22 → 통과.
            #   (구 정적 20 이면 21 > 20 으로 차단 = 자기모순. 노드 축은 21 ≤ 22 로 allow 였다.)
            lv = fbc(live_agents=21, formation_size=5, dept_count=2, formed_agents=10)
            check("9d 라이브 실측(노드21·부서2) — 노드 축 allow 인데 예산 축만 차단하던 모순 해소",
                  lv.blocked is False and lv.budget == 22, "got=%r" % (lv,))
            # 우선순위 보존: env > 동적 기본, 명시 인자 > env.
            _os.environ["CYS_FORMATION_BUDGET"] = "30"
            check("9e env 오버라이드 우선순위 보존(동적 기본보다 env 우선)",
                  bud(None, dept_count=2) == 30 and bud(99, dept_count=2) == 99,
                  "env=%r explicit=%r" % (bud(None, dept_count=2), bud(99, dept_count=2)))
            _os.environ.pop("CYS_FORMATION_BUDGET", None)
        finally:
            if saved is not None:
                _os.environ["CYS_FORMATION_BUDGET"] = saved
            else:
                _os.environ.pop("CYS_FORMATION_BUDGET", None)
    else:
        check("9 편성 예산 동적화(노드 상한 정합)", False,
              "_formation_budget_value(dept_count=)·_nodes_hard_effective 미노출")

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
