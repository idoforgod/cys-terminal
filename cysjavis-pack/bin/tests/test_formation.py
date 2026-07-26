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

    # 9~11. ★ensure 판정 순서·표면화 계약(2026-07-26 배너 불멸 수리 회귀 핀).
    ensure_order_gate(m)

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


# ── 9~11. ensure 판정 순서 + 표면화 스팸 억제 (밀폐 — 라이브 데몬 무접촉) ──
#   배경(라이브 실측 2026-07-26): ensure 가 자원 게이트를 로스터 판정보다 **먼저** 봐서, 5역할이
#   전원 생존(classify=complete)인데도 _resource_ok=False 한 번에 pending-resource 로 조기 반환했다.
#   UI 배너(bootbanner.ts)의 유일한 소멸 신호는 feed kind `formation-complete` 하나뿐이라 그 신호가
#   영영 발행되지 않아 "팀 기동 경고" 배너가 불멸했다.
#   ★쌍 게이트(reward-hack 차단): (a)만 통과하는 구현은 "배너를 그냥 지우는 것"과 구별 불가하므로
#   (b) 로스터가 complete 가 **아닐 때는 formation-complete 를 절대 발행하지 않는다(INV-1)" 를
#   함께 못박는다 — pending-cli 의 설치 안내(기능1 온보딩) 경로 보존까지 실증한다.
def _ensure_harness(m, live, installed, resource_ok):
    """ensure 의 외부 접촉(cys list·자원 게이트·boot_node·feed·EVT)을 전부 스텁으로 대체.
    반환: feed 호출 기록 리스트(kind, title, body) — 라이브 `cys feed push` 는 절대 실행되지 않는다."""
    feeds = []
    m.gate_check = lambda: True
    m._installed_clis = lambda: set(installed)
    m._live_roles = lambda socket=None: (set(live) if live is not None else None)
    m._resource_ok = lambda socket=None: resource_ok
    m._boot_node = lambda role, socket, cwd=None, timeout=200: (True, "stub")
    m._ensure_master_seat = lambda socket, cwd: (True, "stub")
    m._feed = lambda title, body, kind="formation": feeds.append((kind, title, body))
    m._emit_evt = lambda evt, fields: None
    return feeds


def ensure_order_gate(m):
    import shutil
    import tempfile
    if not callable(getattr(m, "ensure", None)):
        check("9a 로스터 complete + 자원 hard → complete", False, "ensure 미구현")
        return
    saved = {k: getattr(m, k) for k in
             ("gate_check", "_installed_clis", "_live_roles", "_resource_ok",
              "_boot_node", "_ensure_master_seat", "_feed", "_emit_evt")}
    saved_state = os.environ.get("CYS_STATE_DIR")
    td = tempfile.mkdtemp(prefix="fmens-")
    os.environ["CYS_STATE_DIR"] = td
    all_clis = {"claude", "agy", "codex"}
    try:
        # (a) 로스터 complete + 자원 hard(_resource_ok=False) → complete + formation-complete 표면화.
        feeds = _ensure_harness(m, live=REQUIRED, installed=all_clis, resource_ok=False)
        state, detail = m.ensure(socket="/tmp/a.sock")
        check("9a 로스터 complete + 자원 hard → complete(자원 게이트보다 로스터 우선)",
              state == "complete", "state=%r detail=%r" % (state, detail))
        check("9a2 formation-complete 표면화(배너 소멸 신호 발행)",
              [f for f in feeds if f[0] == "formation-complete"], "feeds=%r" % (feeds,))

        # (b1) 로스터 partial(2/5) → complete 발행 금지(배너 유지).
        feeds = _ensure_harness(m, live={"master", "cso"}, installed=all_clis, resource_ok=True)
        state, _d = m.ensure(socket="/tmp/b1.sock")
        check("9b1 로스터 partial → complete 아님(배너 유지)",
              m.state_kind(state) == "partial", "state=%r" % (state,))
        check("9b2 partial 경로는 formation-complete 무발행(INV-1)",
              not [f for f in feeds if f[0] == "formation-complete"]
              and [f for f in feeds if f[0] == "formation-partial"], "feeds=%r" % (feeds,))

        # (b2) CLI 전무 → pending-cli 유지 + 설치 안내(기능1 온보딩) 경로 보존 + complete 무발행.
        feeds = _ensure_harness(m, live=set(), installed=set(), resource_ok=True)
        state, _d = m.ensure(socket="/tmp/b2.sock")
        check("9b3 CLI 전무 → pending-cli 유지", m.state_kind(state) == "pending-cli",
              "state=%r" % (state,))
        pend = [f for f in feeds if f[0] == "formation-pending"]
        check("9b4 pending-cli 경로 complete 무발행 + 설치 안내 보존(기능1)",
              not [f for f in feeds if f[0] == "formation-complete"]
              and pend and "claude.ai/install.sh" in pend[0][2], "feeds=%r" % (feeds,))

        # (b3) 로스터 complete 인데 CLI 부분 설치 → complete 로 승격 금지(부분 설치 오판 차단).
        feeds = _ensure_harness(m, live=REQUIRED, installed={"claude"}, resource_ok=False)
        state, _d = m.ensure(socket="/tmp/b3.sock")
        check("9b5 부분 CLI 는 로스터 전원이어도 complete 아님(INV-1)",
              state != "complete" and not [f for f in feeds if f[0] == "formation-complete"],
              "state=%r feeds=%r" % (state, feeds))

        # (c) 동일 kind 연속 10회 ensure → 표면화(feed) 는 1회만(주기 심박 토스트 스팸 차단).
        feeds = _ensure_harness(m, live=REQUIRED, installed=all_clis, resource_ok=False)
        for _ in range(10):
            m.ensure(socket="/tmp/c.sock")
        check("10 동일 kind 10회 ensure → 표면화 1회(주기 실행 토스트 스팸 0)",
              len(feeds) == 1 and feeds[0][0] == "formation-complete",
              "feeds=%r" % (feeds,))

        # (c2) ★편성 실행(final) 경로도 동일 — 여기가 구 코드에서 _feed_for_state 를 **무조건** 부르던
        #      자리다(주기 10분 잡이 붙으면 매 틱 토스트). partial 로스터로 10회 반복 → 표면화 1회.
        feeds = _ensure_harness(m, live={"master", "cso"}, installed=all_clis, resource_ok=True)
        for _ in range(10):
            m.ensure(socket="/tmp/c2.sock")
        check("10b 편성 실행 경로 10회 → 표면화 1회(final 경로 스팸 게이트 = _surface 교체)",
              len(feeds) == 1 and feeds[0][0] == "formation-partial", "feeds=%r" % (feeds,))

        # (d) kind 전이 → 표면화 재발화(침묵 금지 C6 보존 — 스팸 억제가 침묵으로 퇴화하지 않음).
        m._live_roles = lambda socket=None: {"master", "cso"}
        m._resource_ok = lambda socket=None: True
        m.ensure(socket="/tmp/c.sock")
        check("11 kind 전이(complete→partial) → 표면화 재발화",
              len(feeds) == 2 and feeds[1][0] == "formation-partial", "feeds=%r" % (feeds,))
    finally:
        for k, v in saved.items():
            setattr(m, k, v)
        if saved_state is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved_state
        shutil.rmtree(td, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
