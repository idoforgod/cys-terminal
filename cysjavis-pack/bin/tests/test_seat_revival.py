#!/usr/bin/env python3
"""★SEAT 좌석 부활 무결성 E2E (2026-07-17 실사고 회귀 핀 · 격리 cysd 실측).

재현 대상(전부 2026-07-17 dept-2 저널·transcript 실측에서 온 것):
  E1 빈 좌석(role=master 를 쥔 agent 없는 셸)이 있으면 `cys restore` 가 "이미 가동 중"으로
     건너뛰어 master 를 **영영 부활시키지 못한다**.
  E2 그 좌석 앞으로 온 --queued 메시지가 zsh 프롬프트에 문자로 타이핑돼 **보고가 증발한다**.
  E4 (대조군) 빈 좌석이 없으면 정상 부활한다 — 무회귀 핀.
  SEAT-GATE 승계는 opt-in 이며, agent 가 붙은 정당한 좌석은 여전히 보호된다(오탈취 0).

수리 후 기대: seat=empty 로 판정 → 큐 배달 보류(유실 0) → 승계/부활 후 이관돼 배달.
바이너리 미발견 시 skip(exit 0). 라이브 무접촉(격리 하니스 소켓만).

실행: python3 cysjavis-pack/bin/tests/test_seat_revival.py
"""
import importlib.util, json, os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
DBG = os.path.join(REPO, "target", "debug")


def _find(name):
    env = os.environ.get("PHOENIX_HARNESS_" + name.upper())
    if env and os.path.exists(env):
        return env
    c = os.path.join(DBG, name)
    return c if os.path.exists(c) else shutil.which(name)


CYSD, CYS = _find("cysd"), _find("cys")
if not (CYSD and CYS):
    print("SKIP: cysd/cys 미발견(빌드 필요) — CI 게이트는 빌드 후 실행. skip(exit 0).")
    sys.exit(0)

os.environ["PHOENIX_HARNESS_CYSD"] = CYSD
os.environ["PATH"] = DBG + ":" + os.environ.get("PATH", "")
os.environ["PHOENIX_CYS"] = CYS
spec = importlib.util.spec_from_file_location("h", os.path.join(HERE, "..", "javis_phoenix_harness.py"))
h = importlib.util.module_from_spec(spec); spec.loader.exec_module(h)
h.CYS = CYS; h.CYSD = CYSD
h.guard_isolation()

results = []


def check(n, c, d=""):
    results.append(bool(c))
    print(("PASS " if c else "FAIL ") + n + (" | " + d if d else ""))


def _ref(out):
    m = re.search(r"(surface:\d+)", out or "")
    return m.group(1) if m else None


def _status():
    r = h.cys("status", "--json")
    try:
        return json.loads(r.stdout or "{}")
    except Exception:
        return {}


def _seat_of(ref):
    for s in _status().get("surfaces", []):
        if s.get("surface_ref") == ref:
            return s.get("seat")
    return None


def _wait_seat(ref, want, timeout=20.0):
    """watchdog 틱이 좌석 캐시를 갱신할 때까지 대기(캐시는 단일 writer=틱)."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        if _seat_of(ref) == want:
            return True
        time.sleep(1.0)
    return False


def main():
    h.teardown(verbose=False); h._fresh_harness()
    shutil.rmtree(os.path.join(h.HARN_DIR, "phoenix"), ignore_errors=True)
    # ★v0.13.17 C1: 큐 WAL v2 신규 파일도 함께 제거(구 파일만 지우면 큐가 되살아난다).
    for f in ("topology.json", "desired_roster.json", "queue-state.json", "queue-state-v2.json"):
        try: os.remove(os.path.join(h.HARN_DIR, f))
        except OSError: pass
    h.start_daemon(wait=12.0)
    try:
        # ── 셋업: cys-dept 가 만드는 것과 동일한 '빈 셸 좌석'(role=master·agent 없음) ──
        seat = _ref(h.cys("new-surface", "--role", "master").stdout)
        check("셋업: role=master 빈 좌석 생성", bool(seat), "seat=%s" % seat)
        if not seat:
            return

        # ── SEAT-1: 커널 사실 판정 — 셸 단독이면 empty ──
        check("SEAT-1 빈 셸 좌석 = seat:empty", _wait_seat(seat, "empty"),
              "seat=%s" % _seat_of(seat))

        # ── E2(red→green): 좌석 앞 큐 메시지는 배달되지 않고 보류된다(유실 0) ──
        # 수리 전이면 quiet 3초 뒤 zsh 에 타이핑돼 scrollback 에 나타난다.
        marker = "SEAT_E2_PROBE_MARKER"
        h.cys("send", "--surface", seat, "--queued", marker)
        time.sleep(12.0)  # quiet 임계(3s)+틱 여유 — 수리 전이라면 이 사이에 배달됐다
        scr = (h.cys("read-screen", "--surface", seat).stdout or "")
        check("E2 빈 좌석에 큐 배달 안 됨(zsh 타이핑 0)", marker not in scr,
              "screen=%r" % scr[-120:])
        depth = next((s.get("queue_depth") for s in _status().get("surfaces", [])
                      if s.get("surface_ref") == seat), None)
        check("E2 메시지는 큐에 보존됨(유실 0)", depth == 1, "queue_depth=%s" % depth)

        # ── SEAT-GATE: 승계는 opt-in — 플래그 없으면 종전대로 거부 ──
        # ★claim-role 은 **그 pane 안에서** 실행돼야 한다(타 surface claim 금지 게이트 — handlers.rs).
        #   실제 부트 체인(javis_bootstrap ③)도 자기 pane 에서 부른다. 그래서 pane 에 명령을 주입해
        #   결과를 마커로 회수한다(pane 밖 호출은 다른 게이트에 막혀 이 게이트를 검증하지 못한다).
        other = _ref(h.cys("new-surface").stdout)

        def _claim_in_pane(ref, role_name, takeover, tag):
            """pane 안에서 claim-role 실행 → 종료코드를 마커로 회수(RC_<tag>_<code>).
            ★바이너리 프로버넌스 고정: pane 은 **로그인 셸**이라 ~/.zshrc 가 PATH 를 재정렬해
            `cys` 가 설치본(구버전)으로 풀린다 — 실측이 낡은 바이너리를 측정하는 고전적 오염
            (신 플래그 미지원 → clap exit 2). 반드시 방금 빌드한 절대경로로 호출한다."""
            flag = " --takeover-empty-seat" if takeover else ""
            h.cys("send", "--surface", ref,
                  "%s claim-role %s%s >/tmp/seat_%s.out 2>&1; echo RC_%s_$?"
                  % (CYS, role_name, flag, tag, tag))
            h.cys("send-key", "--surface", ref, "Return")
            t0 = time.time()
            while time.time() - t0 < 25.0:
                scr = h.cys("read-screen", "--surface", ref).stdout or ""
                m = re.search(r"RC_%s_(\d+)" % tag, scr)
                if m:
                    return int(m.group(1))
                time.sleep(1.0)
            return None

        rc_noflag = _claim_in_pane(other, "master", False, "noflag")
        check("SEAT-GATE opt-in: 플래그 없으면 거부(현행 유지·deny-by-default)",
              rc_noflag not in (0, None), "rc=%s" % rc_noflag)

        # ── SEAT-2: 빈 좌석이면 명시 승계 성공 + role 해제 + 큐 이관 ──
        rc_take = _claim_in_pane(other, "master", True, "take")
        check("SEAT-2 빈 좌석 승계 성공", rc_take == 0, "rc=%s" % rc_take)
        st = _status()
        roles = {s.get("surface_ref"): s.get("role") for s in st.get("surfaces", [])}
        check("SEAT-2 구 좌석 role 해제(stale role 0)", roles.get(seat) is None,
              "seat role=%s" % roles.get(seat))
        check("SEAT-2 신 좌석이 master", roles.get(other) == "master",
              "other role=%s" % roles.get(other))
        depths = {s.get("surface_ref"): s.get("queue_depth") for s in st.get("surfaces", [])}
        check("SEAT-2 큐 이관됨(구 좌석 0 · 신 좌석 1 — 보고 유실 0)",
              depths.get(seat) == 0 and depths.get(other) == 1,
              "prev=%s next=%s" % (depths.get(seat), depths.get(other)))

        # ── SEAT-3(오탈취 0): 자손 프로세스가 있는 좌석은 Occupied — 승계 거부 ──
        busy = _ref(h.cys("new-surface", "--role", "cso").stdout)
        h.cys("send", "--surface", busy, "sleep 3600"); h.cys("send-key", "--surface", busy, "Return")
        check("SEAT-3 자손 있는 좌석 = seat:occupied", _wait_seat(busy, "occupied"),
              "seat=%s" % _seat_of(busy))
        third = _ref(h.cys("new-surface").stdout)
        rc_busy = _claim_in_pane(third, "cso", True, "busy")
        check("SEAT-3 점유 좌석은 승계 거부(오탈취 0)", rc_busy not in (0, None),
              "rc=%s" % rc_busy)

        # ── E4(대조군·무회귀): 점유 좌석은 배달이 정상 진행 ──
        marker2 = "SEAT_E4_DELIVER_MARKER"
        h.cys("send", "--surface", busy, "--queued", marker2)
        t0, delivered = time.time(), False
        while time.time() - t0 < 25.0:
            if marker2 in (h.cys("read-screen", "--surface", busy).stdout or ""):
                delivered = True
                break
            time.sleep(1.0)
        check("E4 점유 좌석엔 큐 배달 정상(무회귀)", delivered)

        # ── E1(red→green): 빈 좌석이 phoenix/restore 부활을 잠그지 않는다 ──
        # 2026-07-17 실사고의 핵심 체인: 빈 좌석이 '생존'으로 잡혀 ①topology live 에 들고
        # ②`cys restore` 가 "이미 가동 중 — 건너뜀" ③phoenix 대상역할에서 master 탈락
        # ④검증조차 없이 COMPLETE(침묵 성공). 수리 후엔 seat=empty 가 이 사슬을 끊는다.
        e1_seat = _ref(h.cys("new-surface", "--role", "worker").stdout)
        check("E1 셋업: role=worker 빈 좌석", _wait_seat(e1_seat, "empty"), "seat=%s" % _seat_of(e1_seat))
        topo = {}
        try:
            topo = json.loads((h.cys("--socket", h.HARN_SOCK, "identify").stdout or "{}"))
        except Exception:
            pass
        # topology live 엔트리에 seat 가 실렸는가(= restore/phoenix 가 소비할 근거).
        st2 = _status()
        seat_field_present = any("seat" in s for s in st2.get("surfaces", []))
        check("E1 status --json 이 seat 사실을 노출(소비 근거·판정 이원화 금지)", seat_field_present)
        # `cys restore` 는 빈 좌석 역할을 '가동 중'으로 건너뛰면 안 된다. agent 미상 pane 이므로
        # 실제 기동은 안 되지만(무엇을 띄울지 모름) **skip 사유가 '이미 가동 중'이 아니어야** 한다.
        rr = h.cys("restore")
        rr_out = (rr.stdout or "") + (rr.stderr or "")
        check("E1 빈 좌석을 '이미 가동 중'으로 건너뛰지 않음(부활 잠김 해소)",
              "worker: 이미 가동 중" not in rr_out, "restore_out=%r" % rr_out[-160:])
    finally:
        h.teardown(verbose=False)

    ok = all(results)
    print("\n%s: %d/%d" % ("ALL PASS" if ok else "FAIL", sum(1 for r in results if r), len(results)))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
