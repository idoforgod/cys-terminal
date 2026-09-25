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
# ★밀폐 고정② (역포팅 2026-08-01): CYS_FORMATION_EXTERNAL_ROLES 는 로스터·좌석 생성 경로를 바꾸는
# 신규 주변 입력이다. 이 핀은 **제외 0(기본값)** 계약을 검증하므로 주변 env 를 상속하면 안 된다
# (설정된 셸에서 돌리면 pending-cli 역할 목록이 조용히 달라진다 — 현 단언은 kind 만 보아 통과하지만
# 그 통과는 우연이다). external-roles 자체의 검증은 모듈 self-test 가 인자 주입으로 담당한다.
os.environ.pop("CYS_FORMATION_EXTERNAL_ROLES", None)
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

    # 14. ★편성 결판 알림(0.14.42 RE-R3-01 · RV-ROLE-1 · RV-ROLE-2) — 이벤트 구동·단계별 1회·재기동 재발화 0.
    onboard_gate(m)

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
    # ★N-7: 스텁 시그니처는 실함수(`_live_roles(socket, require_live_agent=True)`)와 일치시킨다.
    #   종전 `lambda socket=None` 은 좌석 관측 호출(`_live_roles(socket, require_live_agent=False)`)이
    #   추가되는 순간 TypeError 로 조용히 깨진다 — 미래 파손의 씨앗이라 실시그니처를 그대로 받는다.
    m._live_roles = lambda socket=None, require_live_agent=True: (
        set(live) if live is not None else None)
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
        # ★INST-4(P4-5·T9 원자 갱신): 설치 안내는 플랫폼 분기(win32=irm|iex · 그 외=curl|bash —
        #   cys.rs install_hint 동문)이고, '설치하면 자동으로 편성이 완결됩니다' 거짓 약속은
        #   중립 문구('설치 후 앱 재시작 또는 부서 재기동')로 강등돼 있어야 한다.
        want_url = "claude.ai/install.ps1" if os.name == "nt" else "claude.ai/install.sh"
        check("9b4 pending-cli 경로 complete 무발행 + 설치 안내 보존(기능1·플랫폼 분기)",
              not [f for f in feeds if f[0] == "formation-complete"]
              and pend and want_url in pend[0][2]
              and "자동으로 편성이 완결" not in pend[0][2]
              and "재시작 또는 부서 재기동" in pend[0][2], "feeds=%r" % (feeds,))

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
        # ★A2(2026-09-03 · SURVEY B3 Q2-①) 이후 feed 는 **두 축**이다 — 상태 kind 전이 축과
        #   시도 원장 보류 축(m.HELD_FEED_TITLE). 종전 `len(feeds) == 1` 은 두 축을 한 통에 세서
        #   원장 축이 생기는 순간 뒤집힌다. 축을 갈라 **각각** 10회에 1회임을 못박는다(총수 검사보다
        #   강한 단언: 어느 한 축이 매 틱 발화하면 그 축의 계수에서 즉시 잡힌다).
        state_feeds = [f for f in feeds if f[1] != m.HELD_FEED_TITLE]
        held_feeds = [f for f in feeds if f[1] == m.HELD_FEED_TITLE]
        check("10b 편성 실행 경로 10회 → 전이 표면화 1회(final 경로 스팸 게이트 = _surface 교체)",
              len(state_feeds) == 1 and state_feeds[0][0] == "formation-partial",
              "feeds=%r" % (feeds,))
        check("10c ★원장 보류 축도 10회 → feed 1회(보류 목록 무변화 시 침묵 · A2)",
              len(held_feeds) == 1 and "cooldown" in held_feeds[0][2],
              "held_feeds=%r" % (held_feeds,))

        # (d) kind 전이 → 표면화 재발화(침묵 금지 C6 보존 — 스팸 억제가 침묵으로 퇴화하지 않음).
        m._live_roles = lambda socket=None, require_live_agent=True: {"master", "cso"}
        m._resource_ok = lambda socket=None: True
        m.ensure(socket="/tmp/c.sock")
        state_feeds = [f for f in feeds if f[1] != m.HELD_FEED_TITLE]   # A2 축 분리(위 10b 주석)
        check("11 kind 전이(complete→partial) → 표면화 재발화",
              len(state_feeds) == 2 and state_feeds[1][0] == "formation-partial",
              "feeds=%r" % (feeds,))

        # ── 12. ★force_surface 양방향 핀(2026-07-26 · 앱 부트 배너 소멸 갭 수리) ──
        #   배경: 표면화를 '전이 시에만'으로 좁히자 **앱 재시작** 구멍이 생겼다 — UI 의 중복 억제
        #   상태(bootbanner.ts lastFormationKind)는 인메모리라 새 앱 세션에서 초기화되는데, 상태
        #   파일이 이미 complete 인 레인은 전이가 없어 formation-complete 를 다시 발행하지 않는다
        #   → 새 세션에서 뜬 boot-warning 배너에 소멸 신호가 영영 오지 않는다.
        #   양방향 핀: (a) force=True 면 동일 kind 라도 매 호출 1회 표면화 (b) 기본(False)은 종전대로
        #   전이 시에만 — (b) 가 없으면 수리가 주기 잡 스팸으로 퇴화한다.
        feeds = _ensure_harness(m, live=REQUIRED, installed=all_clis, resource_ok=False)
        m.ensure(socket="/tmp/d.sock")                       # 최초 관측 → 1회
        m.ensure(socket="/tmp/d.sock")                       # 동일 kind·기본 → 생략
        check("12a 기본(force_surface=False)은 동일 kind 재표면화 생략(스팸 게이트 보존)",
              len(feeds) == 1, "feeds=%r" % (feeds,))
        m.ensure(socket="/tmp/d.sock", force_surface=True)   # 전이 없어도 강제 1회
        check("12b force_surface=True → 동일 kind 라도 표면화 1회 발생(부트 소멸 신호 도달)",
              len(feeds) == 2 and feeds[1][0] == "formation-complete", "feeds=%r" % (feeds,))
        m.ensure(socket="/tmp/d.sock", force_surface=True)   # 호출마다 정확히 1회(누적 아님)
        check("12c force 표면화는 호출당 정확히 1회(중복 발행 0)",
              len(feeds) == 3, "feeds=%r" % (feeds,))
        m.ensure(socket="/tmp/d.sock")                       # 다시 기본 → 생략(계약 복귀)
        check("12d force 해제 시 종전 계약 복귀(전이 없으면 무발행)",
              len(feeds) == 3, "feeds=%r" % (feeds,))

        # (e) ★INV-1 유지: 강제 표면화는 **상태를 바꾸지 않는다** — complete 가 아닌 로스터에서
        #     force 를 켜도 formation-complete 는 절대 발행되지 않는다(배너를 그냥 지우는 구현 차단).
        feeds = _ensure_harness(m, live={"master", "cso"}, installed=all_clis, resource_ok=True)
        state, _d = m.ensure(socket="/tmp/e.sock", force_surface=True)
        check("12e force 여도 complete 아니면 formation-complete 무발행(INV-1)",
              m.state_kind(state) == "partial"
              and not [f for f in feeds if f[0] == "formation-complete"]
              and [f for f in feeds if f[0] == "formation-partial"],
              "state=%r feeds=%r" % (state, feeds))
        # CLI 전무(pending) 경로도 동일 — force 가 상태 승격 수단이 되지 않는다.
        feeds = _ensure_harness(m, live=set(), installed=set(), resource_ok=True)
        state, _d = m.ensure(socket="/tmp/e2.sock", force_surface=True)
        check("12f force + CLI 전무 → pending 유지·complete 무발행(INV-1)",
              m.state_kind(state) == "pending-cli"
              and not [f for f in feeds if f[0] == "formation-complete"],
              "state=%r feeds=%r" % (state, feeds))

        # (f) CLI 배선 핀: `--force-surface` 플래그가 ensure(force_surface=True) 로 연결되는가.
        #     (앱 부트 경로 src-tauri fire_formation_ensure 가 이 플래그로만 켠다.)
        seen = []
        saved_ensure = m.ensure
        m.ensure = lambda socket=None, cwd=None, force_surface=False: (
            seen.append(force_surface) or ("complete", "stub"))
        try:
            m._cmd_ensure(["--socket", "/tmp/f.sock", "--force-surface", "--json"])
            m._cmd_ensure(["--socket", "/tmp/f.sock", "--json"])
        finally:
            m.ensure = saved_ensure
        check("12g CLI --force-surface → ensure(force_surface=True) 배선(미지정=False)",
              seen == [True, False], "seen=%r" % (seen,))

        # ── 13. ★T9(P3-1) 시도 원장 유계 — ensure 레벨 실측(harness 밀폐·라이브 무접촉) ──
        #   폭주 앵커 ①: 비생존 역할의 실스폰 시도는 역할당 MAX(3)·쿨다운으로 유계여야 한다.
        #   (a) 쿨다운 0 강제 + 부트 실패 스텁 → 10회 ensure 에도 역할당 시도 정확히 3회(소진 보류).
        boots = []
        feeds = _ensure_harness(m, live={"master", "cso"}, installed=all_clis, resource_ok=True)
        m._boot_node = lambda role, socket, cwd=None, timeout=200: (
            boots.append(role) or (False, "stub-fail"))
        saved_cd = m.FORMATION_RETRY_COOLDOWN_S
        m.FORMATION_RETRY_COOLDOWN_S = 0.0
        try:
            for _ in range(10):
                m.ensure(socket="/tmp/led-a.sock")
        finally:
            m.FORMATION_RETRY_COOLDOWN_S = saved_cd
        check("13a 시도 원장 유계 — 비생존 역할 스폰 시도 = MAX(3)·소진 후 보류",
              boots.count("worker") == 3 and boots.count("reviewer-gemini") == 3
              and boots.count("reviewer-codex") == 3,
              "boots=%r" % {r: boots.count(r) for r in set(boots)})
        check("13b 생존 좌석(cso)은 원장 무카운트 — 입양·멱등 경로 보존(매 ensure 호출)",
              boots.count("cso") == 10, "cso=%d" % boots.count("cso"))
        #   (c) 기본 쿨다운(30s)에서는 연속 10회 ensure 가 역할당 시도 1회로 접힌다(백오프).
        boots2 = []
        feeds = _ensure_harness(m, live={"master", "cso"}, installed=all_clis, resource_ok=True)
        m._boot_node = lambda role, socket, cwd=None, timeout=200: (
            boots2.append(role) or (False, "stub-fail"))
        for _ in range(10):
            m.ensure(socket="/tmp/led-b.sock")
        check("13c 쿨다운(30s) — 연속 10회 ensure 에 역할당 시도 1회(백오프 유계)",
              boots2.count("worker") == 1 and boots2.count("reviewer-codex") == 1,
              "boots=%r" % {r: boots2.count(r) for r in set(boots2)})
        _ = feeds
    finally:
        for k, v in saved.items():
            setattr(m, k, v)
        if saved_state is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved_state
        shutil.rmtree(td, ignore_errors=True)


# ── 14. ★편성 결판 알림(0.14.42 RE-R3-01 · RV-ROLE-1 · RV-ROLE-2) ──
#   배경: 대표(CEO)가 턴 안에서 편성을 기다리지 않게 바꾼 뒤(50c6fc33), 편성 완료 후의 일(부서장 전원 각성 지시 =
#   오너 ABSOLUTE ANCHOR · 첫 과제 · 오너 1줄 보고)이 대표의 기억과 '다음 깨어남'의 우연에 매달렸다. 편성 도구가
#   결판(완결·부분·보류·실패)을 이미 판정하므로 **그 자리에서** 부서장·대표 좌석에 큐로 단계별 1회 알린다.
#   핀: ①무장(arm)된 새 팀만 ②대상·단계당 정확히 1회 ③재기동(새 프로세스·--force-surface) 뒤 재발화 0 ④부서장 좌석에
#   에이전트가 없으면 각성 지시 보류(빈 셸 금지 · 지침보다 먼저 가지 않게) ⑤전송 실패는 유계 재시도 ⑥늦은 알림 금지(TTL).
def _status(roles, master_seat="occupied", master_alive=True):
    surfs = []
    for i, r in enumerate(sorted(roles)):
        s = {"surface_id": i + 1, "role": r, "exited": False, "agent": "claude", "agent_alive": True,
             "seat": "occupied"}
        if r == "master":
            s["seat"] = master_seat
            s["agent_alive"] = master_alive
            if master_seat == "empty":
                s["agent"] = None
                s["agent_alive"] = None
        surfs.append(s)
    return {"surfaces": surfs}


def _spec_b64(pid="tp-1790300000-ab12", display="일반저술출판부"):
    import base64
    import json as _j
    return base64.urlsafe_b64encode(_j.dumps({"v": 1, "id": pid, "display": display,
                                              "purpose": "책을 쓴다."}, ensure_ascii=False)
                                    .encode("utf-8")).decode("ascii")


def _onboard_harness(m, live, installed, resource_ok, status):
    """ensure 외부 접촉 스텁(_ensure_harness) + 알림 seam 2개(_onboard_status · _queue_push) 스텁.
    반환 (pushes, box): pushes = [(socket|None, text)] · box["status"]/box["ok"] 로 도중에 바꾼다."""
    feeds = _ensure_harness(m, live, installed, resource_ok)
    pushes = []
    box = {"status": status, "ok": True}
    m._onboard_status = lambda socket: box["status"]
    m._queue_push = lambda socket, text: (pushes.append((socket, text)) or box["ok"])
    m._onboard_registered = lambda rec: True   # 팀 명부 대조는 14m 이 실물로 잰다
    return pushes, box, feeds


def onboard_gate(m):
    import shutil
    import tempfile
    need = ("onboard_arm", "_onboard_notify", "_onboard_status", "_queue_push", "AWAKEN_ORDER",
            "FORMATION_PARTIAL_MSG", "_onboard_path")
    if not all(hasattr(m, n) for n in need):
        check("14 편성 결판 알림 API(onboard_arm·_onboard_notify·seam)", False,
              "미구현: %r" % [n for n in need if not hasattr(m, n)])
        return
    saved = {k: getattr(m, k, None) for k in   # 없는 이름(구판)은 None — 개별 핀이 FAIL 로 센다(스위트 중단 아님)
             ("gate_check", "_installed_clis", "_live_roles", "_resource_ok", "_boot_node",
              "_ensure_master_seat", "_feed", "_emit_evt", "_onboard_status", "_queue_push", "ensure",
              "_onboard_registered", "_onboard_watch_limits", "_onboard_settle", "_replace_state_obj")}
    saved_state = os.environ.get("CYS_STATE_DIR")
    saved_watch = os.environ.get("CYS_ONBOARD_WATCH_S")
    os.environ["CYS_ONBOARD_WATCH_S"] = "0"   # 기본은 지켜보기 끔 — (r)~(t) 만 짧은 창으로 켠다
    td = tempfile.mkdtemp(prefix="fmob-")
    os.environ["CYS_STATE_DIR"] = td
    all_clis = {"claude", "agy", "codex"}
    import io as _io0
    import contextlib as _ctx0
    # 오너 원문(ABSOLUTE ANCHOR) — 모듈 상수가 글자 그대로여야 한다.
    owner = "CSO, Worker, 리뷰어 등 모든 노드 전원에게 각자의 지침을 하달하고, 전원 각성 절차를 진행하라."
    try:
        check("14a0 부서장 각성 지시 문안 = 오너 원문 그대로", m.AWAKEN_ORDER == owner, repr(m.AWAKEN_ORDER))
        # §10 문구 단일 출처 대조(javis_teamtoken.OWNER_MESSAGES['formation_partial'])
        tt_path = os.path.normpath(os.path.join(SELF, "..", "javis_teamtoken.py"))
        spec = importlib.util.spec_from_file_location("javis_teamtoken_fm", tt_path)
        tt = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tt)
        check("14a1 부분 편성 오너 문구 = §10 원문(javis_teamtoken formation_partial)",
              m.FORMATION_PARTIAL_MSG == tt.OWNER_MESSAGES["formation_partial"],
              "%r != %r" % (m.FORMATION_PARTIAL_MSG, tt.OWNER_MESSAGES["formation_partial"]))

        # (a) 무장 + 완결(부서장 에이전트 있음) → 부서장 1 · 대표 1 · 부서장이 먼저.
        S = "/tmp/ob-a/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        check("14a2 무장 기록 생성", bool(m.onboard_arm(S, "dept-1", _spec_b64())) and os.path.isfile(m._onboard_path(S)))
        m.ensure(socket=S)
        to_master = [p for p in pushes if p[0] == S]
        to_ceo = [p for p in pushes if p[0] is None]
        check("14a3 완결 → 부서장 좌석 정확히 1회 · 오너 원문 포함",
              len(to_master) == 1 and owner in to_master[0][1] and to_master[0][1].startswith("["),
              "pushes=%r" % pushes)
        check("14a4 완결 → 대표 좌석 정확히 1회 · 편성 완료 라벨 · 첫 과제 명령(부서 소켓 결박) · 오너 1줄 보고",
              len(to_ceo) == 1 and to_ceo[0][1].startswith("[편성 완료") and "첫 과제" in to_ceo[0][1]
              and S in to_ceo[0][1] and "오너" in to_ceo[0][1] and "일반저술출판부" in to_ceo[0][1],
              "ceo=%r" % to_ceo)
        check("14a5 부서장 지시가 대표 알림보다 먼저(각성 → 첫 과제 순서)",
              len(pushes) == 2 and pushes[0][0] == S, "pushes=%r" % pushes)
        # (b) 같은 레인 반복 ensure(심박 10분 흉내) → 추가 0.
        for _ in range(5):
            m.ensure(socket=S)
        m.ensure(socket=S, force_surface=True)
        check("14b 반복 ensure·--force-surface 6회 → 추가 배달 0(단계당 1회)", len(pushes) == 2, "pushes=%r" % pushes)
        # (c) 재기동 흉내 — 새 프로세스(새 모듈 인스턴스)가 같은 상태 루트로 부트·심박 ensure → 재발화 0.
        m2 = load()
        p2, _b2, _f2 = _onboard_harness(m2, REQUIRED, all_clis, True, _status(REQUIRED))
        m2.ensure(socket=S, force_surface=True)
        m2.ensure(socket=S)
        m2._onboard_notify(S, "complete", "직접 호출")
        check("14c 재기동 뒤(새 프로세스 · 부트 --force-surface · 심박 · 직접 발행) 재배달 0", p2 == [], "p2=%r" % p2)

        # (d) 무장 없는 부서(기존 부서·rotate·launch) → 알림 0 — 업그레이드 뒤 기존 부서 전부에 쏟아지는 폭주 차단.
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        m.ensure(socket="/tmp/ob-d/cys.sock")
        m._onboard_notify("/tmp/ob-d/cys.sock", "complete", "")
        check("14d 무장 없는 부서 완결 → 알림 0", pushes == [], "pushes=%r" % pushes)

        # (e) 부분(워커 미기동 · 부서장 있음) → 부서장 1 + 대표(§10 문구·첫 과제) 1 → 뒤에 완결 → 부서장 1 + 대표 1(첫 과제 재지시 금지) → 이후 0.
        S = "/tmp/ob-e/cys.sock"
        part = REQUIRED - {"worker"}
        pushes, box, _f = _onboard_harness(m, part, all_clis, True, _status(part))
        m.onboard_arm(S, "dept-2", _spec_b64("tp-1790300000-e"))
        m._boot_node = lambda role, socket, cwd=None, timeout=200: (False, "stub-fail")
        m.ensure(socket=S)
        ceo = [p[1] for p in pushes if p[0] is None]
        mst = [p[1] for p in pushes if p[0] == S]
        check("14e1 부분 → 부서장 1 · 대표 1(§10 부분 문구 자리 1개 · 첫 과제 지시)",
              len(mst) == 1 and len(ceo) == 1 and ceo[0].startswith("[편성 부분")
              and m.FORMATION_PARTIAL_MSG.replace("{n}", "1") in ceo[0] and "첫 과제" in ceo[0],
              "pushes=%r" % pushes)
        m._live_roles = lambda socket=None, require_live_agent=True: set(REQUIRED)
        box["status"] = _status(REQUIRED)
        m.ensure(socket=S)
        ceo = [p[1] for p in pushes if p[0] is None]
        mst = [p[1] for p in pushes if p[0] == S]
        check("14e2 부분 → 완결 전이 → 부서장 +1 · 대표 +1(첫 과제는 이미 지시 — 다시 보내지 않는다)",
              len(mst) == 2 and len(ceo) == 2 and ceo[1].startswith("[편성 완료")
              and "다시 보내지 않는다" in ceo[1] and "--to master \"[CEO 지시]" not in ceo[1], "pushes=%r" % pushes)
        for _ in range(3):
            m.ensure(socket=S)
        check("14e3 완결 알림 뒤 무장 종결 — 추가 0", len(pushes) == 4, "pushes=%r" % pushes)

        # (f) 완결인데 부서장 좌석에 에이전트 없음(빈 셸) → 부서장 0(빈 셸·지침 역순 금지) · 대표 1(보류 명시)
        #     → 부서장 에이전트가 앉은 뒤 ensure → 부서장 1 + 대표 추가 1(첫 과제) → 이후 0.
        S = "/tmp/ob-f/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED, master_seat="empty"))
        m.onboard_arm(S, "dept-3", _spec_b64("tp-1790300000-f"))
        m.ensure(socket=S)
        check("14f1 부서장 빈 셸 → 각성 지시 보류 · 대표 알림 1(보류 명시 · 첫 과제 명령 없음)",
              [p for p in pushes if p[0] == S] == [] and len(pushes) == 1 and "보류" in pushes[0][1]
              and "--to master \"[CEO 지시]" not in pushes[0][1], "pushes=%r" % pushes)
        box["status"] = _status(REQUIRED)
        m.ensure(socket=S)
        m.ensure(socket=S)
        mst = [p[1] for p in pushes if p[0] == S]
        ceo = [p[1] for p in pushes if p[0] is None]
        check("14f2 부서장 착석 뒤 → 부서장 1 · 대표 +1(첫 과제) · 이후 0",
              len(mst) == 1 and len(ceo) == 2 and "첫 과제" in ceo[1] and "--to master" in ceo[1]
              and ceo[1].startswith("[편성 알림 — 팀") and "완료라 하지 않는다" in ceo[0], "pushes=%r" % pushes)

        # (g) 전송 실패(데몬 무응답) → 대상·단계당 최대 ONBOARD_MAX_TRIES 회 시도 후 포기(폭주 0).
        S = "/tmp/ob-g/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        box["ok"] = False
        m.onboard_arm(S, "dept-4", _spec_b64("tp-1790300000-g"))
        for _ in range(10):
            m.ensure(socket=S)
        mst = [p for p in pushes if p[0] == S]
        ceo = [p for p in pushes if p[0] is None]
        check("14g 전송 실패 10회 ensure → 대상별 시도 = ONBOARD_MAX_TRIES(유계)",
              len(mst) == m.ONBOARD_MAX_TRIES and len(ceo) == m.ONBOARD_MAX_TRIES,
              "master=%d ceo=%d max=%d" % (len(mst), len(ceo), m.ONBOARD_MAX_TRIES))

        # (g2) 시간 초과(미확정 — 큐에 들어갔을 수 있다) → 재시도 0(중복 배달 차단) · 첫 과제 지시도 1회로 친다.
        S = "/tmp/ob-g2/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        box["ok"] = None
        m.onboard_arm(S, "dept-4b", _spec_b64("tp-1790300000-g2"))
        with _ctx0.redirect_stderr(_io0.StringIO()):
            for _ in range(5):
                m.ensure(socket=S)
        check("14g2 시간 초과(미확정) → 대상별 1회로 끝(재시도 0 · 중복 배달 차단)",
              len([p for p in pushes if p[0] == S]) == 1 and len([p for p in pushes if p[0] is None]) == 1,
              "pushes=%r" % pushes)
        # (g3) 부서장 좌석 판정 — unknown 좌석은 데몬이 에이전트 생존을 관측했을 때만 · empty 는 언제나 보류.
        mr = m._master_ready
        check("14g3 부서장 좌석 판정(occupied=예 · empty=아니오 · unknown+생존관측=예 · unknown+미관측=아니오 · 사망=아니오)",
              mr(_status(REQUIRED)) is True and mr(_status(REQUIRED, master_seat="empty")) is False
              and mr({"surfaces": [{"role": "master", "seat": "unknown", "agent_alive": True}]}) is True
              and mr({"surfaces": [{"role": "master", "seat": "unknown", "agent_alive": None}]}) is False
              and mr({"surfaces": [{"role": "master", "seat": "occupied", "agent_alive": False}]}) is False
              and mr({"surfaces": [{"role": "master", "agent_alive": True}]}) is True and mr(None) is False)

        # (g4) 알림 경로의 자식 출력은 바이트로 받아 UTF-8(replace)로 푼다 — 로케일 코덱(한국어 Windows cp949) 해석 예외가
        #      rc 127 '보내지 못함' 오판 → 재시도 → 중복 배달로 번지지 않게. 한글 + 깨진 바이트 출력에도 rc 를 그대로 돌려준다.
        rb = m._cys_bytes([sys.executable, "-c",
                           "import sys; sys.stdout.buffer.write('QUEUED 한글'.encode('utf-8') + b'\\xff\\xfe'); sys.exit(0)"],
                          dict(os.environ))
        check("14g4 알림 자식 출력 판독이 로케일 코덱에 무관(한글·깨진 바이트 → rc 0 그대로 · 예외 0)",
              isinstance(rb, tuple) and rb[0] == 0 and "QUEUED" in rb[1], repr(rb))

        # (h) TTL 지난 무장 → 알림 0(늦은 알림 금지).
        S = "/tmp/ob-h/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        import time as _t
        m.onboard_arm(S, "dept-5", _spec_b64("tp-1790300000-h"), now=_t.time() - m.ONBOARD_TTL_S - 5)
        m.ensure(socket=S)
        check("14h TTL 지난 무장 → 알림 0", pushes == [], "pushes=%r" % pushes)

        # (i) 보류(자원 hard) → 대표 1(보류 · 첫 과제 없음) · 부서장 0 → 완결 → 부서장 1 + 대표 1(첫 과제).
        S = "/tmp/ob-i/cys.sock"
        pushes, box, _f = _onboard_harness(m, {"master"}, all_clis, False, _status({"master"}))
        m.onboard_arm(S, "dept-6", _spec_b64("tp-1790300000-i"))
        m.ensure(socket=S)
        check("14i1 보류 → 대표 1(편성 보류 · 첫 과제 명령 없음) · 부서장 0",
              len(pushes) == 1 and pushes[0][0] is None and pushes[0][1].startswith("[편성 보류")
              and "--to master \"[CEO 지시]" not in pushes[0][1], "pushes=%r" % pushes)
        m._live_roles = lambda socket=None, require_live_agent=True: set(REQUIRED)
        box["status"] = _status(REQUIRED)
        m.ensure(socket=S)
        check("14i2 보류 → 완결 → 부서장 1 · 대표 +1(첫 과제)",
              len([p for p in pushes if p[0] == S]) == 1 and len(pushes) == 3 and "첫 과제" in pushes[2][1],
              "pushes=%r" % pushes)

        # (j) 편성 예외(failed) → 대표 1(실패 · 첫 과제 없음) · 부서장 0.
        S = "/tmp/ob-j/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        m.onboard_arm(S, "dept-7", _spec_b64("tp-1790300000-j"))

        def _boom(socket=None, cwd=None, force_surface=False):
            raise RuntimeError("stub")
        m.ensure = _boom
        import io as _io
        import contextlib as _ctx
        with _ctx.redirect_stdout(_io.StringIO()):
            m._cmd_ensure(["--socket", S, "--json"])
        m.ensure = saved["ensure"]
        check("14j 편성 예외(failed) → 대표 1(편성 실패 · 첫 과제 명령 없음) · 부서장 0",
              len(pushes) == 1 and pushes[0][0] is None and pushes[0][1].startswith("[편성 실패")
              and "--to master \"[CEO 지시]" not in pushes[0][1], "pushes=%r" % pushes)

        # (k) CLI 배선 — ensure --onboard-dept/--onboard-spec-b64 가 무장한다 · 같은 제안 재무장은 1회성을 되돌리지 않는다.
        S = "/tmp/ob-k/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        args = ["--socket", S, "--onboard-dept", "dept-8", "--onboard-spec-b64",
                _spec_b64("tp-1790300000-k"), "--json"]
        with _ctx.redirect_stdout(_io.StringIO()):
            m._cmd_ensure(args)
            m._cmd_ensure(args)
        check("14k CLI --onboard-* 무장 → 완결 알림 대상별 1회 · 같은 제안 재실행 추가 0",
              len([p for p in pushes if p[0] == S]) == 1 and len([p for p in pushes if p[0] is None]) == 1,
              "pushes=%r" % pushes)
        # (m) 팀 명부 대조(실물) — 알림 전에 팀이 지워지고 같은 번호가 제안 없이 되살아나면 옛 팀 알림 0 · 무장 닫힘.
        #     명부 판독 불가면 보내지도 닫지도 않는다(다음 ensure 가 다시 본다).
        S = "/tmp/ob-m/cys.sock"
        pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
        m._onboard_registered = saved["_onboard_registered"]
        reg = os.path.join(td, "depts.json")
        saved_reg = os.environ.get("CYS_DEPTS_JSON")
        os.environ["CYS_DEPTS_JSON"] = reg
        try:
            m.onboard_arm(S, "dept-10", _spec_b64("tp-1790300000-m"))
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)                                   # 명부 없음(판독 불가) → 보류
            held_ok = pushes == [] and not (m._onboard_read(m._onboard_path(S)) or {}).get("done")
            import json as _j
            with open(reg, "w", encoding="utf-8") as f:
                _j.dump({"depts": {"dept-10": {"socket": S}}}, f)   # 제안 없이 되살아난 같은 번호
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
            stale_ok = pushes == [] and (m._onboard_read(m._onboard_path(S)) or {}).get("done") == "stale"
            S2 = "/tmp/ob-m2/cys.sock"
            m.onboard_arm(S2, "dept-11", _spec_b64("tp-1790300000-m2"))
            with open(reg, "w", encoding="utf-8") as f:
                _j.dump({"depts": {"dept-11": {"socket": S2, "team_proposal_id": "tp-1790300000-m2"}}}, f)
            m.ensure(socket=S2)
            owned_ok = len(pushes) == 2
        finally:
            if saved_reg is None:
                os.environ.pop("CYS_DEPTS_JSON", None)
            else:
                os.environ["CYS_DEPTS_JSON"] = saved_reg
        check("14m 팀 명부 대조 — 판독 불가=보류(닫지 않음) · 제안 id 불일치=알림 0·무장 닫힘 · 일치=대상별 1회",
              held_ok and stale_ok and owned_ok, "held=%s stale=%s owned=%s pushes=%r" % (held_ok, stale_ok, owned_ok, pushes))

        # (n) 좌석 관측 불가(cys status 실패) 중 결판 → 이번엔 0건(부서장 '빈 자리' 오안내 금지) · 무장 유지 → 관측되면 대상별 1회.
        try:
            S = "/tmp/ob-n/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, None)
            m.onboard_arm(S, "dept-12", _spec_b64("tp-1790300000-n"))
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
            n_held = pushes == [] and not (m._onboard_read(m._onboard_path(S)) or {}).get("sent")
            box["status"] = _status(REQUIRED)
            m.ensure(socket=S)
            check("14n 좌석 관측 불가 → 0건·무장 유지(오안내 금지) → 관측 복구 뒤 부서장 1 · 대표 1",
                  n_held and len([p for p in pushes if p[0] == S]) == 1 and len([p for p in pushes if p[0] is None]) == 1
                  and "보류" not in pushes[-1][1], "held=%s pushes=%r" % (n_held, pushes))
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14n 사례 예외", False, repr(e))

        # (o) 부서장 빈 셸 보류 알림은 오너가 할 일(팀장 창에서 claude 실행)을 말한다 — '빈 자리' 만 말하고 멈추면 흐름이 선다.
        try:
            S = "/tmp/ob-o/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED, master_seat="empty"))
            m.onboard_arm(S, "dept-13", _spec_b64("tp-1790300000-o"))
            m.ensure(socket=S)
            check("14o 부서장 빈 셸 보류 알림 = 오너 안내(팀장 창에서 claude 실행) 포함 · [편성 알림] 예고",
                  len(pushes) == 1 and m.LEADER_EMPTY_MSG in pushes[0][1] and "claude" in pushes[0][1]
                  and "[편성 알림]" in pushes[0][1], "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14o 사례 예외", False, repr(e))

        # (p) 실패 결판의 N 은 도구가 센다(대표가 list 로 다시 세지 않는다) · 자리가 다 있으면 오너 보고를 미룬다.
        try:
            S = "/tmp/ob-p/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED - {"worker"}, all_clis, True, _status(REQUIRED - {"worker"}))
            m.onboard_arm(S, "dept-14", _spec_b64("tp-1790300000-p"))
            m._onboard_notify(S, "failed:RuntimeError", "stub")
            p1 = list(pushes)
            S2 = "/tmp/ob-p2/cys.sock"
            box["status"] = _status(REQUIRED)
            m.onboard_arm(S2, "dept-15", _spec_b64("tp-1790300000-p2"))
            m._onboard_notify(S2, "failed:RuntimeError", "stub")
            check("14p 실패 결판 — 빈 자리 수를 도구가 채운다(§10 문구 N=1 · list 재조회 지시 없음) · 자리 다 있으면 보고 미룸",
                  len(p1) == 1 and m.FORMATION_PARTIAL_MSG.replace("{n}", "1") in p1[0][1] and "list` 역할 열" not in p1[0][1]
                  and len(pushes) == 2 and "자리는 모두 있다" in pushes[1][1], "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14p 사례 예외", False, repr(e))

        # (q) 보낼 것이 남지 않은 무장(영구 부분 편성 등)은 좌석 관측(cys status) 없이 끝난다 — 심박마다 RPC 증가 0.
        try:
            S = "/tmp/ob-q/cys.sock"
            part = REQUIRED - {"reviewer-codex"}
            pushes, box, _f = _onboard_harness(m, part, all_clis, True, _status(part))
            m._boot_node = lambda role, socket, cwd=None, timeout=200: (False, "stub-fail")
            calls = []
            m._onboard_status = lambda socket: (calls.append(socket) or box["status"])
            m.onboard_arm(S, "dept-16", _spec_b64("tp-1790300000-q"))
            m.ensure(socket=S)
            first = len(calls)
            for _ in range(4):
                m.ensure(socket=S)
            check("14q 부분 편성 알림을 다 보낸 뒤 반복 ensure 4회 → 좌석 관측 추가 0 · 알림 추가 0",
                  first == 1 and len(calls) == first and len(pushes) == 2, "calls=%d pushes=%r" % (len(calls), pushes))
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14q 사례 예외", False, repr(e))

        # (r) 무장한 생성 꼬리 호출: 부서장 빈 셸로 보류 → 유계 지켜보기 중 착석 → 부서장 1 · 대표 [편성 알림] 1(첫 과제) → 끝.
        try:
            S = "/tmp/ob-r/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED, master_seat="empty"))
            seat_after = [3]
            polls = []

            def _seat_later(socket):
                polls.append(socket)
                if len(polls) > seat_after[0]:
                    return _status(REQUIRED)
                return _status(REQUIRED, master_seat="empty")
            m._onboard_status = _seat_later
            m._onboard_watch_limits = lambda: (5.0, 0.01)
            args = ["--socket", S, "--onboard-dept", "dept-17", "--onboard-spec-b64", _spec_b64("tp-1790300000-r"), "--json"]
            with _ctx.redirect_stdout(_io.StringIO()), _ctx.redirect_stderr(_io.StringIO()) as er:
                m._cmd_ensure(args)
            mst = [p[1] for p in pushes if p[0] == S]
            ceo = [p[1] for p in pushes if p[0] is None]
            check("14r 지켜보기: 빈 셸 보류 → 착석 감지 → 부서장 1 · 대표 2(보류 → [편성 알림] 첫 과제) · 무장 종결",
                  len(mst) == 1 and len(ceo) == 2 and ceo[1].startswith("[편성 알림 — 팀") and "첫 과제" in ceo[1]
                  and (m._onboard_read(m._onboard_path(S)) or {}).get("done") and "지켜보기 종료: sent" in er.getvalue(),
                  "pushes=%r err=%r" % (pushes, er.getvalue()[-300:]))
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14r 사례 예외", False, repr(e))

        # (s) 지켜보기 중 kill-switch → 보내지 않고 끝(심박이 해제 뒤 이어받음).
        try:
            S = "/tmp/ob-s/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED, master_seat="empty"))
            m.onboard_arm(S, "dept-18", _spec_b64("tp-1790300000-s"))
            m.ensure(socket=S)
            box["status"] = _status(REQUIRED)
            m.gate_check = lambda: False
            m._onboard_watch_limits = lambda: (5.0, 0.01)
            res_s = m._onboard_watch(S, "complete")
            m.gate_check = lambda: True
            check("14s 지켜보기 중 kill-switch → 'paused' · 부서장 0 · 대표 추가 0",
                  res_s == "paused" and len(pushes) == 1 and pushes[0][0] is None, "res=%r pushes=%r" % (res_s, pushes))
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14s 사례 예외", False, repr(e))

        # (t) 지켜보기 창 만료 → 'timeout' · 보낸 것 0 · 관측 횟수 유계(창/간격) · 무장은 남아 심박이 이어받는다.
        try:
            S = "/tmp/ob-t/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED, master_seat="empty"))
            m.onboard_arm(S, "dept-19", _spec_b64("tp-1790300000-t"))
            m.ensure(socket=S)
            tcalls = []
            m._onboard_status = lambda socket: (tcalls.append(1) or box["status"])
            m._onboard_watch_limits = lambda: (0.3, 0.05)
            res_t = m._onboard_watch(S, "complete")
            check("14t 지켜보기 창 만료 → timeout · 추가 발송 0 · 관측 ≤ 창/간격 · 무장 유지(심박 인계)",
                  res_t == "timeout" and len(pushes) == 1 and 1 <= len(tcalls) <= 6
                  and not (m._onboard_read(m._onboard_path(S)) or {}).get("done"),
                  "res=%r calls=%d pushes=%r" % (res_t, len(tcalls), pushes))
            m._onboard_watch_limits = saved["_onboard_watch_limits"]
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14t 사례 예외", False, repr(e))

        # (u) TTL 은 팀 명부 판독보다 먼저 본다 — 명부가 계속 깨져 있어도 늦은 무장은 만료로 닫힌다(영구 잔류 0).
        try:
            S = "/tmp/ob-u/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            m._onboard_registered = lambda rec: None
            m.onboard_arm(S, "dept-20", _spec_b64("tp-1790300000-u"), now=_t.time() - m.ONBOARD_TTL_S - 5)
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
            check("14u 명부 판독 불가 + TTL 초과 → 만료로 닫힘 · 알림 0",
                  pushes == [] and (m._onboard_read(m._onboard_path(S)) or {}).get("done") == "expired", "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14u 사례 예외", False, repr(e))

        # (v) 발송 확정 기록이 빠져도(교체 실패 등) 이번 호출의 결과를 믿는다 — 부서장 지시가 나갔는데 대표에게 '보류'라 하지 않는다.
        try:
            S = "/tmp/ob-v/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            m._onboard_settle = lambda path, key, ok, task=False: None
            m.onboard_arm(S, "dept-21", _spec_b64("tp-1790300000-v"))
            m.ensure(socket=S)
            m._onboard_settle = saved["_onboard_settle"]
            ceo = [p[1] for p in pushes if p[0] is None]
            check("14v 확정 기록 유실에도 대표 알림은 '각성 지시 보냄 + 첫 과제'(보류 오안내 0)",
                  len([p for p in pushes if p[0] == S]) == 1 and len(ceo) == 1 and "보류" not in ceo[0] and "첫 과제" in ceo[0],
                  "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001 — 사례별 예외는 그 사례의 FAIL 로 센다(구판 대조용)
            check("14v 사례 예외", False, repr(e))

        # ── R1-F1(0.14.42 리뷰) 첫 과제 지시 1회성 ──
        #   결함: 대표 알림이 첫 과제를 실었다는 표지가 발송 **뒤 확정**에만 기록됐다. 그 알림이 선점(claimed)만 된 채 확정 전이거나
        #   확정 기록이 빠지면, 다음 호출의 [편성 알림](ceo:ready)이 그것을 '첫 과제 미지시'로 읽고 첫 과제를 한 번 더 지시했다.
        #   또 부서장 키가 다른 호출에 선점돼 발송 중이면, 경쟁에서 진 호출이 대표에게 '팀장 자리가 비어 있다'(오안내)를 보냈다.
        task_cmd = "--to master \"[CEO 지시]"

        def _task_ceo(ps):
            return [t for s, t in ps if s is None and task_cmd in t]

        def _other_caller(ps, status):
            """같은 원장 파일을 보는 **다른 호출자**(지켜보기·심박) — 새 모듈 인스턴스라 메모리 상태를 공유하지 않는다."""
            o = load()
            o._onboard_status = lambda socket: status
            o._onboard_registered = lambda rec: True
            o._queue_push = lambda socket, text: (ps.append((socket, text)) or True)
            return o

        # (v2) 14v 후속: 확정 기록이 빠진 채 심박 ensure 가 이어져도 첫 과제를 담은 대표 알림은 정확히 1.
        try:
            S = "/tmp/ob-v/cys.sock"
            with _ctx.redirect_stderr(_io.StringIO()):
                for _ in range(3):
                    m.ensure(socket=S)
            check("14v2 확정 기록 유실 뒤 심박 ensure 3회 → 첫 과제 담은 대표 알림 정확히 1 · 부서장 1 · 대표 1",
                  len(_task_ceo(pushes)) == 1 and len([p for p in pushes if p[0] == S]) == 1
                  and len([p for p in pushes if p[0] is None]) == 1, "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001
            check("14v2 사례 예외", False, repr(e))

        # (x) 첫 과제를 담은 대표 알림의 확정 기록 교체만 3연속 실패(Windows 백신 잠금 흉내 — os.replace PermissionError)
        #     → 그 키는 선점(claimed)으로 남는다 → 뒤이은 심박 ensure 3회에서 첫 과제를 다시 지시하지 않는다.
        #     기본 레인 · 외부 부서장 레인(CYS_FORMATION_EXTERNAL_ROLES=master) 둘 다.
        for ext in ("", "master"):
            tag = "ext" if ext else "base"
            try:
                if ext:
                    os.environ["CYS_FORMATION_EXTERNAL_ROLES"] = ext
                S = "/tmp/ob-x-%s/cys.sock" % tag
                live = (REQUIRED - {"master"}) if ext else REQUIRED
                pushes, box, _f = _onboard_harness(m, live, all_clis, True, _status(REQUIRED))
                m.onboard_arm(S, "dept-23", _spec_b64("tp-1790300000-x" + tag))
                real_rep, left = saved["_replace_state_obj"], [3]

                def _flaky(path, obj, _real=real_rep, _left=left, _d=os.sep + "onboard" + os.sep):
                    ent = (obj or {}).get("sent") or {}
                    if _d in path and _left[0] > 0 and any(
                            k.startswith("ceo:") and (v or {}).get("st") == "sent" for k, v in ent.items()):
                        _left[0] -= 1
                        raise PermissionError(13, "모의 백신 잠금")
                    return _real(path, obj)
                m._replace_state_obj = _flaky
                with _ctx.redirect_stderr(_io.StringIO()):
                    m.ensure(socket=S)            # 생성 꼬리 ensure — 대표 알림의 확정 기록만 빠진다
                    m._replace_state_obj = real_rep
                    n1 = len(_task_ceo(pushes))
                    for _ in range(3):            # 뒤이은 심박
                        m.ensure(socket=S)
                n_m = len([p for p in pushes if p[0] == S])
                check("14x-%s 첫 과제 대표 알림의 확정 기록 유실(교체 3연속 실패) 뒤 심박 3회 → 첫 과제 지시 정확히 1 · 대표 1 · 부서장 %d"
                      % (tag, 0 if ext else 1),
                      left[0] == 0 and n1 == 1 and len(_task_ceo(pushes)) == 1
                      and len([p for p in pushes if p[0] is None]) == 1 and n_m == (0 if ext else 1),
                      "n1=%d left=%d pushes=%r" % (n1, left[0], pushes))
            except Exception as e:  # noqa: BLE001
                check("14x-%s 사례 예외" % tag, False, repr(e))
            finally:
                os.environ.pop("CYS_FORMATION_EXTERNAL_ROLES", None)
                m._replace_state_obj = saved["_replace_state_obj"]

        # (y) 교차(결정론 · 두 호출자): 한 호출이 첫 과제를 담은 대표 알림을 큐에 넣는 **도중**(선점 뒤·확정 전)에 다른 호출
        #     (지켜보기·심박 — 새 모듈 인스턴스 · 같은 원장 파일)이 끼어든다 → 끼어든 호출은 보내지 않는다 · 첫 과제 지시 정확히 1.
        try:
            S = "/tmp/ob-y/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            m.onboard_arm(S, "dept-24", _spec_b64("tp-1790300000-y"))
            other, inner = _other_caller(pushes, _status(REQUIRED)), []

            def _qp_y(socket, text):
                pushes.append((socket, text))
                if socket is None and not inner:
                    with _ctx.redirect_stderr(_io.StringIO()):
                        inner.append(other._onboard_notify(S, "complete", "지켜보기"))
                return True
            m._queue_push = _qp_y
            m._onboard_notify(S, "complete", "심박")
            check("14y 첫 과제 대표 알림 선점~확정 사이에 다른 호출이 끼어듦 → 끼어든 호출 발송 0 · 첫 과제 지시 정확히 1 · "
                  "부서장 1 · 대표 1",
                  len(inner) == 1 and len(_task_ceo(pushes)) == 1 and len([p for p in pushes if p[0] == S]) == 1
                  and len([p for p in pushes if p[0] is None]) == 1, "inner=%r pushes=%r" % (inner, pushes))
        except Exception as e:  # noqa: BLE001
            check("14y 사례 예외", False, repr(e))

        # (z) 교차(결정론 · 두 호출자): 부서장 각성 지시를 다른 호출이 선점해 보내는 **도중**에 끼어든 호출은 대표에게
        #     '팀장 자리가 비어 있다'(오안내)를 보내지 않고 물러난다 — 대표 알림(첫 과제)은 선점한 호출이 이어서 1회 보낸다.
        try:
            S = "/tmp/ob-z/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            m.onboard_arm(S, "dept-25", _spec_b64("tp-1790300000-z"))
            other, inner = _other_caller(pushes, _status(REQUIRED)), []

            def _qp_z(socket, text):
                pushes.append((socket, text))
                if socket == S and not inner:
                    with _ctx.redirect_stderr(_io.StringIO()):
                        inner.append(other._onboard_notify(S, "complete", "지켜보기"))
                return True
            m._queue_push = _qp_z
            m._onboard_notify(S, "complete", "심박")
            ceo = [t for s, t in pushes if s is None]
            check("14z 부서장 각성 지시 발송 중 끼어든 호출 → 끼어든 호출 발송 0 · 대표 1(첫 과제) · '팀장 자리 비어 있음' 오안내 0 · "
                  "부서장 1",
                  len(inner) == 1 and len(ceo) == 1 and len(_task_ceo(pushes)) == 1
                  and not [t for t in ceo if m.LEADER_EMPTY_MSG in t or "보류" in t]
                  and len([p for p in pushes if p[0] == S]) == 1, "inner=%r pushes=%r" % (inner, pushes))
        except Exception as e:  # noqa: BLE001
            check("14z 사례 예외", False, repr(e))

        # (z1·z2) 부서장 키가 선점(claimed)으로 남았는데 보내던 호출이 없다(발송 도중 종료) — 선점 직후(아직 발송 중일 수 있는 창
        #     ONBOARD_CLAIM_STALE_S 안)에는 물러나고(발송 0 · 무장 유지), 창이 지나면 '미확정'으로 보고 대표 알림(첫 과제)을
        #     1회 보낸다(영구 침묵 0 · 부서장 재발송 0 · 오안내 0).
        try:
            S = "/tmp/ob-z1/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            path = m.onboard_arm(S, "dept-26", _spec_b64("tp-1790300000-z1"))
            m._onboard_update(path, lambda r: r["sent"].__setitem__("master:complete", {"at": _t.time(), "st": "claimed"}))
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
            r1 = m._onboard_read(path) or {}
            ok1 = pushes == [] and not r1.get("done") and "ceo:complete" not in (r1.get("sent") or {})
            check("14z1 부서장 키 선점 직후(발송 중일 수 있음) → 끼어든 호출 발송 0 · 무장 유지", ok1,
                  "pushes=%r rec=%r" % (pushes, r1))
            stale = float(getattr(m, "ONBOARD_CLAIM_STALE_S", 120.0))
            m._onboard_update(path, lambda r: r["sent"]["master:complete"].__setitem__("at", _t.time() - stale - 5))
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
                m.ensure(socket=S)
            ceo = [t for s, t in pushes if s is None]
            check("14z2 선점 창이 지난 부서장 키(발송 도중 종료) → 부서장 재발송 0 · 대표 1(첫 과제 · 오안내 0) · 이후 0",
                  [p for p in pushes if p[0] == S] == [] and len(ceo) == 1 and len(_task_ceo(pushes)) == 1
                  and m.LEADER_EMPTY_MSG not in ceo[0], "pushes=%r" % pushes)
        except Exception as e:  # noqa: BLE001
            check("14z1·z2 사례 예외", False, repr(e))

        # (z3) 부서장이 앉아 있는데 부서장 키 선점 기록 자체가 실패(Windows 교체 잠금 3연속) → 이번엔 대표에게 '팀장 자리가 비어
        #     있다'(오안내)를 보내지 않는다(발송 0) → 잠금이 풀린 다음 ensure 에서 부서장 1 · 대표 1(첫 과제).
        try:
            S = "/tmp/ob-z3/cys.sock"
            pushes, box, _f = _onboard_harness(m, REQUIRED, all_clis, True, _status(REQUIRED))
            m.onboard_arm(S, "dept-28", _spec_b64("tp-1790300000-z3"))
            real_rep, left = saved["_replace_state_obj"], [3]

            def _lock_claim(path, obj, _real=real_rep, _left=left, _d=os.sep + "onboard" + os.sep):
                ent = ((obj or {}).get("sent") or {}).get("master:complete") or {}
                if _d in path and _left[0] > 0 and ent.get("st") == "claimed":
                    _left[0] -= 1
                    raise PermissionError(13, "모의 백신 잠금")
                return _real(path, obj)
            m._replace_state_obj = _lock_claim
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
            m._replace_state_obj = real_rep
            first = list(pushes)
            with _ctx.redirect_stderr(_io.StringIO()):
                m.ensure(socket=S)
                m.ensure(socket=S)
            ceo = [t for s, t in pushes if s is None]
            check("14z3 부서장 키 선점 기록 실패(교체 잠금) → 그 회차 발송 0(빈 자리 오안내 0) → 다음 ensure 부서장 1 · 대표 1(첫 과제)",
                  left[0] == 0 and first == [] and len([p for p in pushes if p[0] == S]) == 1 and len(ceo) == 1
                  and len(_task_ceo(pushes)) == 1 and m.LEADER_EMPTY_MSG not in ceo[0],
                  "left=%d first=%r pushes=%r" % (left[0], first, pushes))
        except Exception as e:  # noqa: BLE001
            check("14z3 사례 예외", False, repr(e))
        finally:
            m._replace_state_obj = saved["_replace_state_obj"]

        # (y2) 실제 다중 프로세스(POSIX fork · 실파일 잠금): 호출자 6개가 같은 무장에 동시에 결판 알림 → 부서장 1 · 대표 1 ·
        #      첫 과제 1 · 오안내 0. fork 가 없는 플랫폼(Windows)은 (y)(z) 결정론 교차 핀이 같은 경로를 잰다.
        import multiprocessing as _mp
        if "fork" in _mp.get_all_start_methods():
            try:
                S = "/tmp/ob-y2/cys.sock"
                m.onboard_arm(S, "dept-27", _spec_b64("tp-1790300000-y2"))
                logp = os.path.join(td, "y2-pushes.log")

                def _y2_worker(barrier):
                    code = 0
                    try:
                        o = load()
                        o._onboard_status = lambda socket: _status(REQUIRED)
                        o._onboard_registered = lambda rec: True

                        def _qp(socket, text):
                            _t.sleep(0.05)   # cys send 왕복 흉내 — 선점~확정 창을 넓힌다
                            with open(logp, "a", encoding="utf-8") as f:
                                f.write("%s\t%s\n" % ("M" if socket else "C", text.replace("\n", " ")))
                            return True
                        o._queue_push = _qp
                        barrier.wait(30)
                        with _ctx.redirect_stderr(_io.StringIO()):
                            o._onboard_notify(S, "complete", "동시")
                    except BaseException:  # noqa: BLE001 — 자식 실패는 exit 코드로 부모에 알린다
                        code = 3
                    os._exit(code)
                mpc = _mp.get_context("fork")
                bar = mpc.Barrier(6)
                procs = [mpc.Process(target=_y2_worker, args=(bar,)) for _ in range(6)]
                for p in procs:
                    p.start()
                for p in procs:
                    p.join(90)
                hung = [p for p in procs if p.is_alive()]
                for p in hung:
                    p.terminate()   # 이 테스트가 띄운 자식만(핸들로) — 패턴 kill 아님
                lines = []
                if os.path.exists(logp):
                    with open(logp, encoding="utf-8") as f:
                        lines = f.read().splitlines()
                ceo = [ln for ln in lines if ln.startswith("C")]
                check("14y2 6 프로세스 동시 결판 알림(실파일 잠금) → 부서장 1 · 대표 1 · 첫 과제 1 · 오안내 0",
                      not hung and all(p.exitcode == 0 for p in procs)
                      and len([ln for ln in lines if ln.startswith("M")]) == 1 and len(ceo) == 1
                      and task_cmd in ceo[0] and m.LEADER_EMPTY_MSG not in ceo[0],
                      "hung=%d codes=%r lines=%r" % (len(hung), [p.exitcode for p in procs], [ln[:90] for ln in lines]))
            except Exception as e:  # noqa: BLE001
                check("14y2 사례 예외", False, repr(e))
        else:
            print("SKIP 14y2 (fork 없음 — (y)(z) 결정론 교차 핀이 같은 경로를 잰다)")

        # (w) 팀 명부 경로 = 쓰는 쪽(cys-dept `$HOME/.cys/depts.json`)과 같은 HOME 우선 — Windows 파이썬 `~`(USERPROFILE)가
        #     Git Bash HOME 과 갈려도 명부를 읽는다(못 읽으면 알림 0 = ③).
        try:
            import json as _jw
            reg_fn = saved["_onboard_registered"]
            hw = os.path.join(td, "home-w")
            os.makedirs(os.path.join(hw, ".cys"), exist_ok=True)
            with open(os.path.join(hw, ".cys", "depts.json"), "w", encoding="utf-8") as f:
                _jw.dump({"depts": {"dept-22": {"team_proposal_id": "tp-1790300000-w"}}}, f)
            sv_home, sv_reg, sv_exp = os.environ.get("HOME"), os.environ.pop("CYS_DEPTS_JSON", None), m.os.path.expanduser
            os.environ["HOME"] = hw
            m.os.path.expanduser = lambda p: p.replace("~", os.path.join(td, "userprofile-w"), 1)
            try:
                w_ok = reg_fn({"dept": "dept-22", "proposal_id": "tp-1790300000-w"}) is True
                w_neg = reg_fn({"dept": "dept-22", "proposal_id": "tp-other"}) is False
            finally:
                m.os.path.expanduser = sv_exp
                if sv_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = sv_home
                if sv_reg is not None:
                    os.environ["CYS_DEPTS_JSON"] = sv_reg
            check("14w 팀 명부는 HOME(cys-dept 규약) 우선 — ~ 가 다른 곳을 가리켜도 판독 · 제안 id 대조 유지",
                  w_ok and w_neg, "ok=%s neg=%s" % (w_ok, w_neg))
        except Exception as e:  # noqa: BLE001
            check("14w 사례 예외", False, repr(e))

        # (l) 손상 명세 → 무장 0 · 예외 0.
        with _ctx.redirect_stderr(_io.StringIO()):
            r = m.onboard_arm("/tmp/ob-l/cys.sock", "dept-9", "!!!not-b64")
        check("14l 손상 명세 → 무장 0(예외 없이)", not r and not os.path.exists(m._onboard_path("/tmp/ob-l/cys.sock")))
    except Exception as e:  # noqa: BLE001 — 스위트 예외는 FAIL 로 센다
        import traceback
        traceback.print_exc()
        check("14 편성 결판 알림 스위트 예외", False, repr(e))
    finally:
        for k, v in saved.items():
            if v is not None:
                setattr(m, k, v)
        if saved_state is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved_state
        if saved_watch is None:
            os.environ.pop("CYS_ONBOARD_WATCH_S", None)
        else:
            os.environ["CYS_ONBOARD_WATCH_S"] = saved_watch
        shutil.rmtree(td, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
