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


if __name__ == "__main__":
    sys.exit(main())
