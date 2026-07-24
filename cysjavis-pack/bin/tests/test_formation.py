#!/usr/bin/env python3
"""test_formation.py — DD-3 편성 상태 기계 계약 핀 (T0 RED-first).

현 코드에는 `javis_formation.py` 가 **없다** → import 실패 → RED. T2 가 GREEN 화.

계약(구현 후 통과 기준):
  1) 상태 enum: complete / partial / pending-cli / pending-resource / failed.
  2) complete = master + 4종 의무 노드(cso·worker·reviewer-gemini·reviewer-codex) 전부일 때만
     (부분 설치를 complete 로 오판 금지 — Sim S2-5).
  3) 일부 CLI 만 설치 → partial:{missing} (설치된 부분 기동 + 부족 목록).
  4) CLI 전무 → pending-cli:{roles} (빈 셸 유지·온보딩 보존).
  5) 자원 초과 → pending-resource.
  6) ensure() 는 멱등 + 소켓키별 싱글플라이트 락(동시 2회 = 1회만 편성).
  7) kill-switch(paused) 중 ensure → pending 유지(gate-check 선행·Sim S2-4).

실행: python3 test_formation.py   (exit 0=PASS / 1=RED)
"""
import importlib.util
import os
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_formation.py"))
# ★밀폐(hermetic) 고정 — javis_formation 을 로드하는 테스트는 라이브 팩(~/.cys/pack) 상속을 금지하고
# 리포 팩만 읽는다(환경 무관 결정론·라이브 무접촉). classify 는 순수라 현 검사엔 팩 읽기가 없으나,
# 동일 모듈을 쓰는 테스트의 비밀폐 방어(parity_paths 결함과 동류) 차원의 선제 고정.
os.environ["CYS_PACK_DIR"] = os.path.normpath(os.path.join(SELF, "..", ".."))  # cysjavis-pack/
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


REQUIRED = {"master", "cso", "worker", "reviewer-gemini", "reviewer-codex"}


def main():
    m = load()
    if m is None:
        for n in ("1 상태 enum 5종", "2 complete=5종 전부", "3 partial:{missing}",
                  "4 pending-cli:{roles}", "5 pending-resource",
                  "6 ensure 멱등·싱글플라이트 락", "7 paused→pending 유지"):
            check(n, False, "javis_formation.py 미구현 — T2 대상(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    # 1. 상태 상수/enum
    states = getattr(m, "STATES", None) or getattr(m, "FormationState", None)
    names = set()
    if states is not None:
        try:
            names = {str(s).lower().split(".")[-1].replace("_", "-") for s in
                     (states if hasattr(states, "__iter__") else states.__members__)}
        except Exception:
            names = set(str(states).lower().replace("_", "-").split())
    for want in ("complete", "partial", "pending-cli", "pending-resource", "failed"):
        check("1 상태 enum 포함: %s" % want, any(want in n for n in names) or want in str(states).lower(),
              "선언된=%r" % names)

    # 2~5. classify(installed_clis, live_roles, resource_ok) → state
    classify = getattr(m, "classify", None)
    if callable(classify):
        all_clis = {"claude", "agy", "codex"}
        try:
            st_full = classify(installed=all_clis, live=REQUIRED, resource_ok=True)
            check("2 complete=master+4종 전부", "complete" in str(st_full).lower(),
                  "got=%r" % (st_full,))
            st_part = classify(installed={"claude"}, live={"master", "cso"}, resource_ok=True)
            check("3 partial:{missing}", "partial" in str(st_part).lower(), "got=%r" % (st_part,))
            st_none = classify(installed=set(), live=set(), resource_ok=True)
            check("4 pending-cli", "pending" in str(st_none).lower() and "cli" in str(st_none).lower(),
                  "got=%r" % (st_none,))
            st_res = classify(installed=all_clis, live={"master"}, resource_ok=False)
            check("5 pending-resource", "resource" in str(st_res).lower(), "got=%r" % (st_res,))
        except Exception as e:
            for n in ("2 complete=master+4종 전부", "3 partial:{missing}",
                      "4 pending-cli", "5 pending-resource"):
                check(n, False, "classify 호출 실패: %s" % e)
    else:
        for n in ("2 complete=master+4종 전부", "3 partial:{missing}",
                  "4 pending-cli", "5 pending-resource"):
            check(n, False, "classify() 미구현(RED)")

    # 6. ensure 멱등·싱글플라이트
    check("6 ensure()·싱글플라이트 락 API 존재",
          callable(getattr(m, "ensure", None)) and
          (callable(getattr(m, "_singleflight", None)) or callable(getattr(m, "acquire_lock", None))),
          "ensure/락 API 미구현(RED)")

    # 7. paused → pending (gate-check 선행)
    check("7 paused→pending(gate-check 선행) API 존재",
          callable(getattr(m, "gate_check", None)) or hasattr(m, "PAUSE_HONORED"),
          "kill-switch 존중 훅 실측 미구현(RED)")

    # 8. ★싱글플라이트 락 **실행 스모크** (REVISE2 — 크로스플랫폼 락 헬퍼 실경로 커버).
    #    _singleflight.__enter__ 은 posix=fcntl.flock / Windows=msvcrt.locking 을 실제로 호출한다.
    #    이 스모크는 windows-pack 잡에서 **msvcrt.locking 분기를 실행**시켜(어느 CI 에서도 안 돌던
    #    Windows 락 경로) 획득·해제·상호배제를 실측한다. env -i 밀폐(CYS_STATE_DIR temp).
    _singleflight = getattr(m, "_singleflight", None)
    if callable(_singleflight):
        import tempfile
        saved_state = os.environ.get("CYS_STATE_DIR")
        td = tempfile.mkdtemp(prefix="fmlk-")
        os.environ["CYS_STATE_DIR"] = td
        try:
            key = "lock-smoke"
            with _singleflight(key) as a:
                # 1차 획득 성공 = _lk(fcntl.flock/msvcrt.locking)가 예외 없이 실행됨(플랫폼 락 경로 실행 증명).
                check("8a 싱글플라이트 락 획득(락 헬퍼 실행 — Windows=msvcrt.locking)",
                      a.acquired is True, "acquired=%r" % a.acquired)
                # 2차(같은 키) 획득 시도 = 상호배제로 실패해야(동시 2회 편성 1회로 억제 계약).
                with _singleflight(key) as b:
                    check("8b 같은 키 재획득 차단(상호배제 — 동시 편성 억제)",
                          b.acquired is False, "second acquired=%r (False 여야 함)" % b.acquired)
            # 해제(__exit__=_ulk 실행) 후 재획득 가능해야(락 정상 반납 — msvcrt 언락 경로도 실행).
            with _singleflight(key) as c:
                check("8c 해제 후 재획득 가능(_ulk 반납 실행)",
                      c.acquired is True, "reacquired=%r" % c.acquired)
        finally:
            if saved_state is None:
                os.environ.pop("CYS_STATE_DIR", None)
            else:
                os.environ["CYS_STATE_DIR"] = saved_state
            import shutil
            shutil.rmtree(td, ignore_errors=True)
    else:
        check("8a 싱글플라이트 락 실행 스모크", False, "_singleflight 미구현")

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
