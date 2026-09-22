#!/usr/bin/env python3
"""javis_purge_verify.py — 기능2 부서 완전 폐역 사후 결정론 검증기.

부서 state 격리(cys-dept down --purge-state / javis_org destroy --purge-state) 뒤
①해당 부서 재등재 0 ②묘비 생존 ③형제 부서 무오염 을 결정론(exit code)으로 판정한다.
'검증기'이므로 고의로 깨진 입력(불완전 purge·묘비 소실)에 반드시 non-zero FAIL을 낸다
(negative fixture로 자기검증 — happy path만 통과시키는 위양성 검증기 금지).

재발견 판정은 javis_phoenix.discover_depts() 를 그대로 재실행한다(glob `state_root/cys-dept-*`
∪ depts.json). 격리 위치(`cys-trash/`)는 glob 무매치라 재발견되지 않아야 정상이다.

exit: 0=전량 통과 · 2=검증 실패(FAIL 사유 출력) · 3=입력 오류.
"""
import argparse, json, os, sys
sys.dont_write_bytecode = True  # SEAL-1 층4: 호출자 env 와 무관하게 형제 import 의 __pycache__ 기록 차단(D-pyc 2026-09-21)

def _load_dept_roster_tombstones(path):
    """phoenix dept_roster.json 의 tombstones 집합. 파일 부재/손상=빈 집합(호출자가 필요성 판단)."""
    if not path or not os.path.exists(path):
        return None
    try:
        d = json.load(open(path, encoding="utf-8"))
        return set(d.get("tombstones", []))
    except Exception:
        return None

def verify(dept, state_root, depts_json=None, dept_roster=None,
           siblings=(), require_tombstone=True):
    """반환 (ok:bool, report:dict). 순수 함수(예외 없이 사유를 report에 담는다)."""
    fails = []
    checks = {}

    # ── ① 원위치 소멸: state_root/cys-dept-<dept> 잔존 = 불완전 purge FAIL ──
    src = os.path.join(state_root, f"cys-dept-{dept}")
    src_gone = not os.path.exists(src)
    checks["state_dir_gone"] = src_gone
    if not src_gone:
        fails.append(f"불완전 purge: state 디렉토리 잔존({src})")

    # ── ② discover_depts 재실행: 해당 부서 재등재 0 ──
    # ★phoenix discover_depts 를 그대로 재사용(glob ∪ registry) — 검증기 독자 구현 아님(계약 일치).
    _bindir = os.path.dirname(os.path.abspath(__file__))
    if _bindir not in sys.path:
        sys.path.insert(0, _bindir)
    os.environ["PHOENIX_DEPT_STATE_ROOT"] = state_root
    os.environ["PHOENIX_DEPTS_JSON"] = depts_json or os.path.join(state_root, "__no_registry__.json")
    try:
        import javis_phoenix as _ph
        discovered = _ph.discover_depts()
    except Exception as e:  # 검증기는 크래시 금지 — 조회 불가는 명시 FAIL
        return False, {"error": f"discover_depts 재실행 실패: {type(e).__name__}: {e}"}
    dept_rediscovered = dept in discovered
    checks["dept_not_rediscovered"] = not dept_rediscovered
    if dept_rediscovered:
        fails.append(f"재등재 발생: discover_depts 가 폐역 부서 '{dept}' 를 다시 발견({discovered[dept]})")

    # ── ③ 형제 부서 무오염: 기대 형제는 여전히 발견돼야 한다 ──
    sib_report = {}
    for s in siblings:
        present = s in discovered
        sib_report[s] = present
        if not present:
            fails.append(f"형제 오염: 기대 형제 부서 '{s}' 가 발견에서 사라짐(무관 부서 손상)")
    checks["siblings_survive"] = sib_report

    # ── ④ 묘비 생존: dept_roster tombstones 에 폐역 부서 존재 ──
    if require_tombstone:
        tombs = _load_dept_roster_tombstones(dept_roster)
        if tombs is None:
            checks["tombstone_present"] = False
            fails.append(f"묘비 소실: dept_roster 부재/손상({dept_roster}) — 묘비 확인 불가")
        else:
            present = dept in tombs
            checks["tombstone_present"] = present
            # 형제가 묘비에 잘못 들어가지 않았는지(메인/형제 roster 무오염)
            wrong = [s for s in siblings if s in tombs]
            if wrong:
                fails.append(f"묘비 오염: 형제 부서가 묘비에 잘못 편입됨{wrong}")
            if not present:
                fails.append(f"묘비 소실: 폐역 부서 '{dept}' 가 dept_roster tombstones 에 없음{sorted(tombs)}")

    report = {"dept": dept, "checks": checks, "fails": fails,
              "verdict": "PASS" if not fails else "FAIL"}
    return (not fails), report

def main():
    ap = argparse.ArgumentParser(description="기능2 부서 완전 폐역 사후 검증기")
    ap.add_argument("--dept", required=True)
    ap.add_argument("--state-root", required=True)
    ap.add_argument("--depts-json")
    ap.add_argument("--dept-roster", help="phoenix dept_roster.json 경로(묘비 확인)")
    ap.add_argument("--expect-sibling", action="append", default=[],
                    help="여전히 발견돼야 할 형제 부서(반복 지정)")
    ap.add_argument("--no-tombstone", action="store_true", help="묘비 확인 생략")
    args = ap.parse_args()
    if not os.path.isdir(args.state_root):
        # 0-부서/부재 state-root: 우아 처리 — 재등재 0·형제 없음이면 통과(크래시 금지).
        os.makedirs(args.state_root, exist_ok=True)
    ok, report = verify(args.dept, args.state_root, args.depts_json, args.dept_roster,
                        siblings=args.expect_sibling, require_tombstone=not args.no_tombstone)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if ok else 2

if __name__ == "__main__":
    sys.exit(main())
