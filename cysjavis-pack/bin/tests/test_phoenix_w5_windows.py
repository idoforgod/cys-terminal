#!/usr/bin/env python3
"""W5 축D Windows 격납 — 플랫폼 무관 로직 게이트(리포 커밋). Windows Job Object kill-on-close 실거동·msvcrt
이중스폰 차단 실거동·CI 실 cysd.exe 부활은 **Windows 러너에서만 검증 가능**(설계 D4/E1) — 이 파일은 mac/Linux
에서도 돌릴 수 있는 계약(_try_lock_nb 상호배제·lease 통합·open 모드)만 결정론 검증한다. Windows 런타임 게이트는
.github/workflows(windows-build.yml T5 real-path)·decide_auto_restore Rust 단위테스트가 담당.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_w5_windows.py  (0=전건 PASS)
"""
import importlib.util, os, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []
def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))


def main():
    td = tempfile.mkdtemp(prefix="phoenix-w5-")

    # ── D2 통합 락 헬퍼 _try_lock_nb: 상호배제 계약(unix flock·Windows msvcrt 공통) ──
    #   같은 파일을 두 핸들로 열어 첫 핸들이 락을 잡으면 둘째는 False(경합)여야 한다. 해제(close) 후 재획득 True.
    lockp = os.path.join(td, "x.lock")
    f1 = open(lockp, "a+")
    r1 = m._try_lock_nb(f1)
    check("D2 _try_lock_nb 최초 획득 → True", r1 is True, "r1=%s" % r1)
    f2 = open(lockp, "a+")
    r2 = m._try_lock_nb(f2)
    check("D2 _try_lock_nb 보유 중 둘째 → False(상호배제)", r2 is False, "r2=%s" % r2)
    f2.close()
    f1.close()
    f3 = open(lockp, "a+")
    r3 = m._try_lock_nb(f3)
    check("D2 _try_lock_nb 해제(close) 후 재획득 → True", r3 is True, "r3=%s" % r3)
    f3.close()

    # ── D2 restore lease: 통합 헬퍼 경유 이중 스폰 차단(보유 중 재획득 실패) ──
    home = os.path.join(td, "state", "phoenix")
    os.makedirs(home, exist_ok=True)
    sock = os.path.join(td, "state", "cys.sock")
    m.CYS = os.path.join(td, "nonexistent-cys")
    ok1, h1 = m._acquire_restore_lease(sock)
    check("D2 restore lease 최초 획득", ok1 is True and h1 is not None, "ok=%s" % ok1)
    ok2, h2 = m._acquire_restore_lease(sock)
    check("D2 restore lease 보유 중 재획득 → False(이중 스폰 차단·Windows fail-open 제거)",
          ok2 is False, "ok2=%s" % ok2)
    m._release_lease(h1)
    ok3, h3 = m._acquire_restore_lease(sock)
    check("D2 restore lease 해제 후 재획득 → True", ok3 is True, "ok3=%s" % ok3)
    m._release_lease(h3)

    # ── ★(0.14.31 · WP-4 R2 · 리뷰 minor) lease 경합은 **유계 재시도** 뒤에 skip 한다 ──
    #   0.14.31 부터 이 lease 를 다투는 것이 restore 프로세스만이 아니다: 일상적인 SessionStart
    #   훅의 역할 재결합이 커밋 동안 같은 파일을 수십 ms 잡는다. 한 번 실패에 회차를 통째로
    #   버리면 콜드부트 자가치유가 그 짧은 창 때문에 통째로 건너뛴다.
    #   결정론 검증: 락 시도를 가로채 처음 두 번은 경합(False), 세 번째에 성공하게 만들고
    #   `time.sleep` 을 세어, **재시도가 실제로 일어나고 획득에 성공**하는지 잰다.
    _orig_try = m._try_lock_nb
    _orig_sleep = m.time.sleep
    calls = {"n": 0, "slept": []}
    def _flaky(f):
        calls["n"] += 1
        return True if calls["n"] >= 3 else False
    m._try_lock_nb = _flaky
    m.time.sleep = lambda d: calls["slept"].append(d)
    try:
        okr, hr = m._acquire_restore_lease(sock)
    finally:
        m._try_lock_nb = _orig_try
        m.time.sleep = _orig_sleep
    check("R2 lease 경합은 유계 재시도 후 획득한다(회차 통째 skip 금지)",
          okr is True and calls["n"] == 3, "시도=%s ok=%s" % (calls["n"], okr))
    check("R2 재시도 대기가 유계다(총 1.5초 이하)",
          0 < sum(calls["slept"]) <= 1.5, "sleep=%s" % calls["slept"])
    if hr is not None:
        m._release_lease(hr)
    # 음성 대조: 끝까지 경합하면 종전 계약대로 skip 한다(배타 자체는 넓히지 않았다).
    calls2 = {"n": 0, "slept": []}
    m._try_lock_nb = lambda f: False
    m.time.sleep = lambda d: calls2["slept"].append(d)
    try:
        okn, _hn = m._acquire_restore_lease(sock)
    finally:
        m._try_lock_nb = _orig_try
        m.time.sleep = _orig_sleep
    check("R2 끝까지 경합하면 종전대로 skip(False)", okn is False, "ok=%s" % okn)
    check("R2 무한 재시도가 아니다(시도 횟수 유계)", len(calls2["slept"]) <= 4,
          "sleep 횟수=%s" % len(calls2["slept"]))

    # ── D2 roster/dept 락: 통합 헬퍼 경유(P1-8 Windows fail-open 제거) ──
    hl = m._acquire_roster_lock(sock, "roster")
    check("D2 roster lock 획득(핸들 반환)", hl is not None)
    # 보유 중 외부 핸들 획득 실패(상호배제)
    ext = open(os.path.join(home, "roster.lock"), "a+")
    check("D2 roster lock 보유 중 외부 → False", m._try_lock_nb(ext) is False)
    ext.close()
    m._release_lease(hl)

    # ── open 모드 계약: lease/lock 파일은 truncate 금지(a+)여야 Windows msvcrt byte0 영역이 일치한다 ──
    #   (회귀 핀: 과거 "w" 는 truncate 로 동시 open 시 msvcrt 영역/락이 어긋날 수 있었다 — 소스 문자열 검증.)
    src = open(PH, encoding="utf-8").read()
    check("D2 restore.lease open 모드 a+(무truncate)", 'open(lease_path, "a+")' in src)
    check("D2 roster/dept lock open 모드 a+(무truncate)", 'open(p, "a+")' in src)
    check("D2 _try_lock_nb 에 msvcrt.locking(LK_NBLCK) 배선", "msvcrt.locking" in src and "LK_NBLCK" in src)

    import shutil
    shutil.rmtree(td, ignore_errors=True)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
