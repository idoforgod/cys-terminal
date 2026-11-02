#!/usr/bin/env python3
"""W3 손상 내성(C2 폴백 체인·C3 설명-가능-축소·C5 구조화 liveness) 테스트(리포 커밋).
데몬 불요: CYS 를 더미로 두거나 m.cys/헬퍼를 몽키패치해 파일·순수 로직만 검증한다(라이브 무접촉).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_w3_corruption.py  (0=전건 PASS)
"""
import calendar, hashlib, importlib.util, json, os, shutil, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []
def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))


def _write(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        if isinstance(obj, str):
            f.write(obj)
        else:
            json.dump(obj, f, ensure_ascii=False)


def _ph_home(td, name):
    sd = os.path.join(td, name)
    home = os.path.join(sd, "phoenix")
    os.makedirs(home, exist_ok=True)
    return os.path.join(sd, "cys.sock"), home


# ─────────────────────────── C2: 폴백 체인 ───────────────────────────

def c2_bak_fallback(td):
    """desired 손상 + 유효 .bak(죽은 역할 worker) → 격리 + .bak 복원 → DEGRADED(exit 3·부활 보류)."""
    sock, home = _ph_home(td, "c2bak")
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, "{ corrupt desired ]]]")
    _write(dp + ".bak", {"roster": {"worker": {"role": "worker"}}, "tombstones": []})
    m.CYS = os.path.join(td, "nonexistent-cys")  # cys 전부 rc127 강등 → live 없음
    res = m.run_restore(sock, ticket="c2bak", stub=True, print_result=False)
    check("C2.bak → DEGRADED", res.get("phoenix_restore") == "DEGRADED", "verdict=%s" % res.get("phoenix_restore"))
    check("C2.bak → exit 3", m.restore_exit_code(res) == 3, "exit=%s" % m.restore_exit_code(res))
    check("C2.bak → source=bak", any(d.get("source") == "bak" for d in (res.get("degraded") or [])),
          "%s" % res.get("degraded"))
    check("C2.bak → 부활 보류 held worker", "worker" in (res.get("held_roles") or []),
          "held=%s" % res.get("held_roles"))
    # 격리본(.corrupt-*) 이 생겼고, desired 는 .bak 내용으로 복원됨(이후 observe 가 정상 재기록)
    corr = [f for f in os.listdir(home) if f.startswith("desired_roster.json.corrupt-")]
    check("C2.bak → 손상 원본 .corrupt 격리", len(corr) >= 1, "corr=%s" % corr)
    check("C2.bak → desired 유효 복원", m._roster_file_status(dp) == "valid", m._roster_file_status(dp))


def c2_snapshot_fallback(td):
    """desired 손상 + .bak 부재 + 세대 스냅샷 존재 → 스냅샷 복원 → DEGRADED(source=snapshot).
    LIVE_STATE/HOME 를 격리 temp 로 몽키패치해 스냅샷 경로를 유효화한다."""
    live = os.path.realpath(os.path.join(td, "live"))
    home = os.path.join(live, "phoenix")
    os.makedirs(home, exist_ok=True)
    sock = os.path.join(live, "cys.sock")
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, "]]] corrupt no bak")
    gen = os.path.join(td, ".cys", "state-generations", "20260706T120000Z")
    _write(os.path.join(gen, "topology.json"),
           {"entries": [{"role": "worker"}], "tombstones": []})
    with open(os.path.join(gen, "topology.json"), "rb") as _f:   # a generation with no manifest is not read (0.14.57 P0)
        _raw = _f.read()
    _write(os.path.join(gen, "manifest.json"),
           {"created_at": calendar.timegm(time.strptime("20260706T120000Z", "%Y%m%dT%H%M%SZ")) + 0.25,
            "files": [{"source": os.path.join(live, "topology.json"), "stored_as": "topology.json",
                       "sha256": hashlib.sha256(_raw).hexdigest(), "size": len(_raw)}]})
    _live0, _home0 = m.LIVE_STATE, m.HOME
    try:
        m.LIVE_STATE = live
        m.HOME = td
        m.CYS = os.path.join(td, "nonexistent-cys")
        rec = m._recover_retention_file(sock, dp, "desired_roster")
        check("C2.snap → degraded", rec.get("status") == "degraded", "%s" % rec)
        check("C2.snap → source=snapshot", rec.get("source") == "snapshot", "%s" % rec)
        # 복원된 desired 에 스냅샷 역할 worker 반영
        rr, _t = m.load_desired_roster(sock)
        check("C2.snap → worker 복원", "worker" in rr, "roster=%s" % list(rr))
    finally:
        m.LIVE_STATE, m.HOME = _live0, _home0


def c2_unrecoverable(td):
    """desired 손상 + .bak/스냅샷 전무(비 LIVE) → unrecoverable → CORRUPT(exit 6)."""
    sock, home = _ph_home(td, "c2none")
    _write(os.path.join(home, "desired_roster.json"), "{ corrupt nothing ]]]")
    m.CYS = os.path.join(td, "nonexistent-cys")
    res = m.run_restore(sock, ticket="c2none", stub=True, print_result=False)
    check("C2.none → CORRUPT", res.get("phoenix_restore") == "CORRUPT", "verdict=%s" % res.get("phoenix_restore"))
    check("C2.none → exit 6", m.restore_exit_code(res) == 6, "exit=%s" % m.restore_exit_code(res))
    check("C2.none → isolated 명시", bool(res.get("isolated")), "%s" % res.get("isolated"))


def c2_corrupt_prune(td):
    """.corrupt-<ts> 격리본은 최근 3개만 유지(초과 prune·inode DoS 차단)."""
    d = os.path.join(td, "prune")
    os.makedirs(d, exist_ok=True)
    target = os.path.join(d, "desired_roster.json")
    made = []
    for i in range(5):
        _write(target, "corrupt %d" % i)
        iso = m._isolate_corrupt(target)  # 매번 격리 → prune 이 최근 3개로 수렴
        if iso:
            made.append(iso)
    remain = sorted(f for f in os.listdir(d) if f.startswith("desired_roster.json.corrupt-"))
    check("C2.prune → 최근 3개만 유지", len(remain) == 3, "remain=%d(%s)" % (len(remain), remain))


def c2_missing_vs_corrupt(td):
    """corrupt 와 missing 은 다른 exit/event — missing=정상 빈 부팅(NOOP·exit0), corrupt=이벤트+degraded/corrupt."""
    m.CYS = os.path.join(td, "nonexistent-cys")
    sock_missing, _ = _ph_home(td, "missing")  # 상태파일 전무
    res_missing = m.run_restore(sock_missing, ticket="miss", stub=True, print_result=False)
    check("missing → NOOP exit0",
          res_missing.get("phoenix_restore") == "NOOP" and m.restore_exit_code(res_missing) == 0,
          "verdict=%s exit=%s" % (res_missing.get("phoenix_restore"), m.restore_exit_code(res_missing)))
    sock_corrupt, home_c = _ph_home(td, "corruptvs")
    _write(os.path.join(home_c, "desired_roster.json"), "]]] corrupt")
    res_corrupt = m.run_restore(sock_corrupt, ticket="corr", stub=True, print_result=False)
    check("corrupt → exit≠0(missing 과 상이)",
          m.restore_exit_code(res_corrupt) != 0
          and res_corrupt.get("phoenix_restore") in ("CORRUPT", "DEGRADED"),
          "verdict=%s exit=%s" % (res_corrupt.get("phoenix_restore"), m.restore_exit_code(res_corrupt)))


def c2_breaker_corrupt(td):
    """보조상태 breaker.json 손상 → hard-fail 아님: 격리+경고 후 빈 카운트 재시작(크래시 없음)."""
    sock, home = _ph_home(td, "breaker")
    bp = os.path.join(home, "breaker.json")
    _write(bp, "{ corrupt breaker ]]]")
    m.CYS = os.path.join(td, "nonexistent-cys")
    try:
        opened, attempts = m.breaker_check_and_record(sock)
        ok = (opened is False) and (len(attempts) == 1)
    except Exception as e:
        ok = False
        attempts = "EXC:%s" % e
    check("C2.breaker → 손상에도 크래시 없이 리셋", ok, "attempts=%s" % attempts)
    corr = [f for f in os.listdir(home) if f.startswith("breaker.json.corrupt-")]
    check("C2.breaker → 손상 격리(.corrupt·침묵 아님)", len(corr) >= 1, "corr=%s" % corr)


# ─────────────────────────── C3: 설명-가능-축소 ───────────────────────────

def _observe_isolated(td, name, prev_obj, load_ret, rebase=False):
    """observe_and_persist_roster 를 격리 환경에서 호출하되 병합 소스를 전부 비활성화하고
    load_desired_roster 를 주입해 '설명 여부'만 검증한다(순수 가드 로직)."""
    sock, home = _ph_home(td, name)
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, prev_obj)  # prev(직전 영속본)
    m.CYS = os.path.join(td, "nonexistent-cys")
    saved = {k: getattr(m, k) for k in
             ("load_desired_roster", "_snapshot_roster_entries", "read_topology",
              "live_role_surfaces", "_read_tombstone_intents", "_resync_intents_if_daemon_up",
              "get_boot_epoch")}
    m.load_desired_roster = lambda s: (dict(load_ret[0]), set(load_ret[1]))
    m._snapshot_roster_entries = lambda s: {}
    m.read_topology = lambda s: {"entries": [], "updated_at": 0}
    m.live_role_surfaces = lambda s: {}
    m._read_tombstone_intents = lambda s: []
    m._resync_intents_if_daemon_up = lambda s, i: False
    m.get_boot_epoch = lambda s: None
    try:
        roster, tombs = m.observe_and_persist_roster(sock, rebase=rebase)
    finally:
        for k, v in saved.items():
            setattr(m, k, v)
    on_disk = json.load(open(dp))
    return roster, tombs, on_disk


def c3_unexplained_refused(td):
    """직전 {a,b} 대비 b 소실(묘비·ephemeral 아님) → write 1회 거부(직전 상태 보존·b 유지)."""
    roster, tombs, disk = _observe_isolated(
        td, "c3ref",
        prev_obj={"roster": {"a": {"role": "a"}, "b": {"role": "b"}}, "tombstones": []},
        load_ret=({"a": {"role": "a"}}, set()))  # load 가 b 를 잃음(부분/버그 시뮬)
    check("C3.refuse → 반환 roster 에 b 보존", "b" in roster, "roster=%s" % list(roster))
    check("C3.refuse → 디스크 미축소(b 유지)", "b" in (disk.get("roster") or {}),
          "disk=%s" % list((disk.get("roster") or {})))


def c3_explained_tombstone_ok(td):
    """직전 {a,b} 대비 b 소실이 tombstone 으로 설명 → write 진행(정당 스케일다운)."""
    roster, tombs, disk = _observe_isolated(
        td, "c3tomb",
        prev_obj={"roster": {"a": {"role": "a"}, "b": {"role": "b"}}, "tombstones": ["b"]},
        load_ret=({"a": {"role": "a"}}, {"b"}))
    check("C3.tomb → 설명된 축소 write 진행(디스크 roster=={a})",
          set((disk.get("roster") or {}).keys()) == {"a"}, "disk=%s" % list((disk.get("roster") or {})))


def c3_rebase_forces(td):
    """설명불가 축소라도 rebase=True 면 강제 수용(운영자 명시 재기반)."""
    roster, tombs, disk = _observe_isolated(
        td, "c3rebase",
        prev_obj={"roster": {"a": {"role": "a"}, "b": {"role": "b"}}, "tombstones": []},
        load_ret=({"a": {"role": "a"}}, set()), rebase=True)
    check("C3.rebase → 강제 수용(디스크 roster=={a})",
          set((disk.get("roster") or {}).keys()) == {"a"}, "disk=%s" % list((disk.get("roster") or {})))


def c3_write_failure_no_silent(td):
    """원자쓰기 실패는 침묵(except:pass) 금지 — 예외를 삼켜 크래시하지 않고 roster 를 반환한다(log/EVT 는 부수)."""
    sock, home = _ph_home(td, "c3wf")
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, {"roster": {"a": {"role": "a"}}, "tombstones": []})
    m.CYS = os.path.join(td, "nonexistent-cys")
    saved_w = m._atomic_write_json
    saved_snap = m._snapshot_roster_entries
    saved_topo = m.read_topology
    saved_live = m.live_role_surfaces
    saved_intents = m._read_tombstone_intents
    saved_resync = m._resync_intents_if_daemon_up
    def _boom(*a, **k):
        raise OSError("disk full (simulated)")
    m._atomic_write_json = _boom
    m._snapshot_roster_entries = lambda s: {}
    m.read_topology = lambda s: {"entries": [], "updated_at": 0}
    m.live_role_surfaces = lambda s: {}
    m._read_tombstone_intents = lambda s: []
    m._resync_intents_if_daemon_up = lambda s, i: False
    try:
        roster, tombs = m.observe_and_persist_roster(sock)
        ok = "a" in roster  # 크래시 없이 반환
    except Exception as e:
        ok = False
        roster = "EXC:%s" % e
    finally:
        m._atomic_write_json = saved_w
        m._snapshot_roster_entries = saved_snap
        m.read_topology = saved_topo
        m.live_role_surfaces = saved_live
        m._read_tombstone_intents = saved_intents
        m._resync_intents_if_daemon_up = saved_resync
    check("C3.writefail → 침묵 크래시 없이 roster 반환", ok, "%s" % roster)


# ─────────────────────────── C5: 구조화 liveness ───────────────────────────

class _FakeR:
    def __init__(self, rc, out=""):
        self.returncode = rc
        self.stdout = out
        self.stderr = ""


def c5_liveness_from_status_json(td):
    """live_role_surfaces 가 `cys status --json`.surfaces(구조화)에서 role/exited 를 읽는다."""
    saved = m.cys
    status = {"surfaces": [
        {"surface_ref": "surface:10", "role": "worker", "exited": False, "agent_alive": True},
        {"surface_ref": "surface:11", "role": "master", "exited": True, "agent_alive": False},
        {"surface_ref": "surface:12", "role": None, "exited": True, "agent_alive": None},  # 미claim 잔재
    ]}
    def fake_cys(*args, **kw):
        if args[:2] == ("status", "--json"):
            return _FakeR(0, json.dumps(status))
        return _FakeR(127, "")  # list 폴백은 오지 않아야(status 성공)
    m.cys = fake_cys
    try:
        out = m.live_role_surfaces("sock")
    finally:
        m.cys = saved
    check("C5.status → worker liveness 구조화 파싱",
          out.get("worker") and out["worker"][0]["exited"] is False, "%s" % out.get("worker"))
    check("C5.status → master exited 반영",
          out.get("master") and out["master"][0]["exited"] is True, "%s" % out.get("master"))
    check("C5.status → agent_alive 노출(readiness 신호)",
          out["worker"][0].get("agent_alive") is True, "%s" % out.get("worker"))
    check("C5.status → 미claim(role=null) 잔재는 '-'로 보존(P1-6 회귀 잠금·C6 회수용)",
          out.get("-") and out["-"][0]["exited"] is True and out["-"][0]["surface"] == "surface:12",
          "%s" % out.get("-"))


def c5_liveness_fallback_to_list(td):
    """status --json 미도달(rc≠0) 시 `cys list` 정규식 폴백(가용성 하한)."""
    saved = m.cys
    def fake_cys(*args, **kw):
        if args[:2] == ("status", "--json"):
            return _FakeR(1, "")  # 미지원/미도달
        if args[:1] == ("list",):
            return _FakeR(0, "surface:20\trole=worker\tpid=999\texited=false\t·\t·")
        return _FakeR(127, "")
    m.cys = fake_cys
    try:
        out = m.live_role_surfaces("sock")
    finally:
        m.cys = saved
    check("C5.fallback → list 폴백 파싱",
          out.get("worker") and out["worker"][0]["exited"] is False, "%s" % out.get("worker"))


def c5_readiness_structured_ack(td):
    """stage_ready(prod) 가 배너 이전에 구조화 ack(agent_alive)로 ready 판정."""
    saved = m._status_json
    m._status_json = lambda s: {"surfaces": [{"surface_ref": "surface:30", "role": "worker",
                                              "exited": False, "agent_alive": True}]}
    try:
        alive = m._surface_agent_alive("sock", "surface:30")
        ready, detail = m.stage_ready("sock", "worker", "surface:30", stub=False)
    finally:
        m._status_json = saved
    check("C5.ack → _surface_agent_alive True", alive is True, "%s" % alive)
    check("C5.ack → stage_ready 구조화 ready(배너 무의존)",
          ready is True and "structured ack" in detail, "ready=%s detail=%s" % (ready, detail))


def c5_harness_allow_live_gate():
    """하네스 guard_isolation: LIVE 타깃은 CYS_PHOENIX_ALLOW_LIVE=1 없으면 거부, 있으면 통과."""
    HPH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix_harness.py"))
    hspec = importlib.util.spec_from_file_location("javis_phoenix_harness", HPH)
    h = importlib.util.module_from_spec(hspec)
    hspec.loader.exec_module(h)
    saved_hd, saved_sock = h.HARN_DIR, h.HARN_SOCK
    saved_env = os.environ.get("CYS_PHOENIX_ALLOW_LIVE")
    try:
        h.HARN_DIR = h.LIVE_STATE          # 강제 LIVE 타깃(사고/오설정 시뮬)
        h.HARN_SOCK = h.LIVE_SOCK
        os.environ.pop("CYS_PHOENIX_ALLOW_LIVE", None)
        refused = False
        try:
            h.guard_isolation()
        except SystemExit:
            refused = True
        check("C5.harness → opt-in 없으면 LIVE write 거부", refused)
        os.environ["CYS_PHOENIX_ALLOW_LIVE"] = "1"
        allowed = True
        try:
            h.guard_isolation()  # opt-in → 통과(die 없음)
        except SystemExit:
            allowed = False
        check("C5.harness → opt-in(=1) 있으면 통과", allowed)
    finally:
        h.HARN_DIR, h.HARN_SOCK = saved_hd, saved_sock
        if saved_env is None:
            os.environ.pop("CYS_PHOENIX_ALLOW_LIVE", None)
        else:
            os.environ["CYS_PHOENIX_ALLOW_LIVE"] = saved_env


# ─────────────────────────── deploy 중첩 lease(게이트 행) ───────────────────────────

def deploy_nested_lease(td):
    """restore lease 보유 중 재진입 restore 는 LEASE_HELD(멱등 skip·exit0) — deploy 내부 restore 가 콜드부트
    auto 와 경합해도 FAILED 오판이 아니라 정직 skip. lease 해제 후 재진입은 정상 진행."""
    sock, home = _ph_home(td, "lease")
    _write(os.path.join(home, "desired_roster.json"), {"roster": {}, "tombstones": []})
    m.CYS = os.path.join(td, "nonexistent-cys")
    ok, handle = m._acquire_restore_lease(sock)
    check("lease → 최초 획득", ok is True, "ok=%s" % ok)
    res = m.run_restore(sock, ticket="held", stub=True, print_result=False)
    check("lease → 보유 중 재진입 LEASE_HELD",
          res.get("phoenix_restore") == "LEASE_HELD", "verdict=%s" % res.get("phoenix_restore"))
    check("lease → LEASE_HELD exit 0(멱등·FAILED 아님)", m.restore_exit_code(res) == 0,
          "exit=%s" % m.restore_exit_code(res))
    m._release_lease(handle)
    res2 = m.run_restore(sock, ticket="held2", stub=True, print_result=False)
    check("lease → 해제 후 재진입 정상(NOOP)", res2.get("phoenix_restore") == "NOOP",
          "verdict=%s" % res2.get("phoenix_restore"))


# ─────────────────────────── P1-7 RMW flock · 보조상태 journal 손상 ───────────────────────────

def p17_rmw_lock(td):
    """desired/dept RMW 직렬화 lock(roster.lock) — 보유 중 외부 NB 실패·해제 후 성공·observe 후 자동 해제."""
    if m.IS_WINDOWS:
        check("P1-7 (Windows msvcrt=W5·skip)", True)
        return
    import fcntl
    sock, home = _ph_home(td, "p17")
    p = os.path.join(home, "roster.lock")
    h = m._acquire_roster_lock(sock, "roster")
    check("P1-7 lock 확보", h is not None)
    f2 = open(p, "w"); held = False
    try:
        fcntl.flock(f2.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        held = True
    f2.close()
    check("P1-7 보유 중 외부 NB 실패(직렬화)", held)
    m._release_lease(h)
    f3 = open(p, "w"); reok = False
    try:
        fcntl.flock(f3.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB); reok = True
    except OSError:
        pass
    f3.close()
    check("P1-7 해제 후 재획득 성공", reok)
    # observe_and_persist_roster 는 RMW 후 lock 을 해제한다
    m.CYS = os.path.join(td, "nonexistent-cys")
    _write(os.path.join(home, "desired_roster.json"), {"roster": {}, "tombstones": []})
    m.observe_and_persist_roster(sock)
    f4 = open(p, "w"); freed = False
    try:
        fcntl.flock(f4.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB); freed = True
    except OSError:
        pass
    f4.close()
    check("P1-7 observe 후 lock 자동 해제", freed)


def aux_journal_corrupt(td):
    """보조상태 journal 손상 → hard-fail 아님: fresh dict 반환(크래시 없음) + 격리(.corrupt·침묵 아님)."""
    sock, home = _ph_home(td, "jrnl")
    jp = m.journal_path(sock, "t1")
    _write(jp, "{ corrupt journal ]]]")
    try:
        j = m.load_journal(sock, "t1")
        ok = isinstance(j, dict) and j.get("roles") == {} and j.get("events") == []
    except Exception as e:
        ok = False; j = "EXC:%s" % e
    check("aux.journal 손상 → fresh dict(크래시 없음)", ok, "%s" % j)
    corr = [f for f in os.listdir(home) if f.startswith(os.path.basename(jp) + ".corrupt-")]
    check("aux.journal 손상 격리(.corrupt·침묵 아님)", len(corr) >= 1, "%s" % corr)


# ─────────── codex W3 BLOCKING 수정 게이트: provenance 영속·해제·unknown-liveness·dept 매트릭스 ───────────

def c2_degraded_persists_across_retry(td):
    """★codex BLOCKING(1): 손상 복구→DEGRADED 후, cysd auto-retry 상당 2차 실행에서도 보류 유지(휘발 금지).
    lease 는 별도 프로세스 재시도를 모사하려 fail-open 으로 우회(동일 프로세스 재호출 LEASE_HELD 회피)."""
    sock, home = _ph_home(td, "persist")
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, "{ corrupt ]]]")
    _write(dp + ".bak", {"roster": {"worker": {"role": "worker"}}, "tombstones": []})
    m.CYS = os.path.join(td, "nonexistent-cys")
    saved_lease = m._acquire_restore_lease
    m._acquire_restore_lease = lambda s: (True, None)  # 재시도=별도 프로세스 모사(lease 자유)
    try:
        r1 = m.run_restore(sock, ticket="p1", stub=True, print_result=False)
        # 복구본에 provenance 영속 확인
        prov = m._recovered_provenance(dp)
        r2 = m.run_restore(sock, ticket="p1", stub=True, print_result=False)  # auto-retry 2차
    finally:
        m._acquire_restore_lease = saved_lease
    check("persist ① attempt1 DEGRADED", r1.get("phoenix_restore") == "DEGRADED", "%s" % r1.get("phoenix_restore"))
    check("persist ② 복구본에 recovered_from 영속", isinstance(prov, dict) and prov.get("source") == "bak", "%s" % prov)
    check("persist ③ attempt2(재시도)도 DEGRADED 유지(휘발 아님)",
          r2.get("phoenix_restore") == "DEGRADED", "%s" % r2.get("phoenix_restore"))
    check("persist ④ attempt2 held worker(부활 보류 지속)", "worker" in (r2.get("held_roles") or []),
          "held=%s" % r2.get("held_roles"))


def c2_provenance_release(td):
    """★codex BLOCKING(1) 해제: ①검증된-건강 topology replace(rev 마커) ②운영자 --rebase 시 provenance 제거."""
    # ① healthy topology replace → provenance 자동 해제
    sock, home = _ph_home(td, "release_healthy")
    dp = os.path.join(home, "desired_roster.json")
    _write(dp, {"roster": {"worker": {"role": "worker"}}, "tombstones": [],
                "recovered_from": {"source": "bak", "ts": 1}})
    # state_dir(=dirname(sock))에 건강 topology(schema_version+rev) 배치
    _write(os.path.join(os.path.dirname(sock), "topology.json"),
           {"schema_version": 1, "tombstones_rev": 7, "tombstones": [], "entries": [{"role": "worker"}]})
    m.CYS = os.path.join(td, "nonexistent-cys")
    m.observe_and_persist_roster(sock)
    check("release ① 건강 replace → provenance 제거", m._recovered_provenance(dp) is None,
          "%s" % m._recovered_provenance(dp))
    # ② --rebase → provenance 제거(topology 없어도)
    sock2, home2 = _ph_home(td, "release_rebase")
    dp2 = os.path.join(home2, "desired_roster.json")
    _write(dp2, {"roster": {"worker": {"role": "worker"}}, "tombstones": [],
                 "recovered_from": {"source": "snapshot", "ts": 1}})
    m.observe_and_persist_roster(sock2, rebase=True)
    check("release ② --rebase → provenance 제거", m._recovered_provenance(dp2) is None,
          "%s" % m._recovered_provenance(dp2))
    # ③ 건강 replace 아니고 rebase 아니면 provenance 유지(보류 지속)
    sock3, home3 = _ph_home(td, "release_hold")
    dp3 = os.path.join(home3, "desired_roster.json")
    _write(dp3, {"roster": {"worker": {"role": "worker"}}, "tombstones": [],
                 "recovered_from": {"source": "bak", "ts": 1}})
    m.observe_and_persist_roster(sock3)  # topology 없음(불건강) → 유지
    check("release ③ 불건강·비rebase → provenance 유지(보류 지속)",
          m._recovered_provenance(dp3) is not None, "%s" % m._recovered_provenance(dp3))


def c5_malformed_list_holds(td):
    """★codex BLOCKING(3): status --json 실패 + list 형식 드리프트(비어있지 않은데 0건/부분 미매칭) →
    liveness=unknown → 전원 사망 추정 금지·부활 보류(대량 오스폰 0)."""
    saved = m.cys
    def fake_cys(*args, **kw):
        if args[:2] == ("status", "--json"):
            return _FakeR(1, "")                       # status 미도달
        if args[:1] == ("list",):
            return _FakeR(0, "surface:5 BROKEN FORMAT no fields here\nsurface:6 also drifted")
        return _FakeR(1, "")
    m.cys = fake_cys
    try:
        out, known = m._live_role_surfaces_checked("sock")
        # run_restore 로 대량 스폰 차단 확인(desired 에 죽은 역할 존재)
        sock, home = _ph_home(td, "c5malformed")
        _write(os.path.join(home, "desired_roster.json"),
               {"roster": {"worker": {"role": "worker"}}, "tombstones": []})
        saved_lease = m._acquire_restore_lease
        m._acquire_restore_lease = lambda s: (True, None)
        try:
            res = m.run_restore(sock, ticket="c5m", stub=True, print_result=False)
        finally:
            m._acquire_restore_lease = saved_lease
    finally:
        m.cys = saved
    check("C5.malformed → known=False(구조 드리프트)", known is False, "known=%s" % known)
    check("C5.malformed → run_restore DEGRADED(unknown_liveness)",
          res.get("phoenix_restore") == "DEGRADED" and res.get("degraded_reason") == "unknown_liveness",
          "%s/%s" % (res.get("phoenix_restore"), res.get("degraded_reason")))
    check("C5.malformed → 대량 스폰 0(target 미진입)", not res.get("held_roles") and "spawned" not in res,
          "%s" % res)


def c5_empty_list_is_known(td):
    """대조군: rc≠0·빈 출력(데몬 미도달/콜드부트)은 known=True(살아있는 surface 0 = 정당 관측·부활 정상)."""
    saved = m.cys
    def fake_cys(*args, **kw):
        return _FakeR(127, "")  # status·list 모두 실패·빈 출력
    m.cys = fake_cys
    try:
        out, known = m._live_role_surfaces_checked("sock")
    finally:
        m.cys = saved
    check("C5.empty → known=True(빈=surface 0·부활 정상)", known is True and out == {}, "known=%s out=%s" % (known, out))


def dept_corruption_matrix(td):
    """★codex minor(4): dept_roster × 손상유형 매트릭스 — .bak 복원·missing·repeated retry 지속(desired 대칭)."""
    # ── dept corrupt + .bak(유효) → degraded(source=bak)·provenance 영속 ──
    sock, home = _ph_home(td, "deptbak")
    depp = os.path.join(home, "dept_roster.json")
    _write(depp, "]]] corrupt dept")
    _write(depp + ".bak", {"roster": {"dept-1": {}}, "tombstones": ["dept-9"]})
    m.CYS = os.path.join(td, "nonexistent-cys")
    rec = m._recover_retention_file(sock, depp, "dept_roster")
    check("dept.bak → degraded source=bak", rec.get("status") == "degraded" and rec.get("source") == "bak", "%s" % rec)
    check("dept.bak → provenance 영속", isinstance(m._recovered_provenance(depp), dict))
    # ── dept missing → status missing(부활 정상·degrade 아님) ──
    sock_m, home_m = _ph_home(td, "deptmissing")
    rec_m = m._recover_retention_file(sock_m, os.path.join(home_m, "dept_roster.json"), "dept_roster")
    check("dept.missing → status missing(정상)", rec_m.get("status") == "missing", "%s" % rec_m)
    # ── dept corrupt + no bak → discovery degraded(재발견 경로·unrecoverable 아님) ──
    sock_d, home_d = _ph_home(td, "deptdisc")
    ddp = os.path.join(home_d, "dept_roster.json")
    _write(ddp, "{ corrupt no bak ]]]")
    rec_d = m._recover_retention_file(sock_d, ddp, "dept_roster")
    check("dept.corrupt+nobak → discovery degraded(재발견)", rec_d.get("status") == "degraded" and rec_d.get("source") == "discovery", "%s" % rec_d)
    # ── dept degraded 반복 retry 지속(observe_and_persist_depts 가 provenance 이월) ──
    m.observe_and_persist_depts(sock)  # topology 무관(dept=glob) → provenance 유지
    check("dept.retry → provenance 지속(보류 유지)", isinstance(m._recovered_provenance(depp), dict),
          "%s" % m._recovered_provenance(depp))
    # ── dept --rebase → provenance 해제 ──
    m.observe_and_persist_depts(sock, rebase=True)
    check("dept.rebase → provenance 해제", m._recovered_provenance(depp) is None,
          "%s" % m._recovered_provenance(depp))


def main():
    td = os.path.realpath(tempfile.mkdtemp(prefix="phoenix-w3-"))
    # The generation reader is no longer guarded by lane, so these fixtures must not reach the real store. Each axis, set and asserted.
    cwd0, home0 = os.getcwd(), m.HOME
    os.chdir(td)
    m.HOME = td
    for k in list(os.environ):
        if k.startswith(("CYS_", "JAVIS_", "AITERM_")) or k == "CLAUDE_CONFIG_DIR":
            del os.environ[k]
    assert os.path.realpath(os.getcwd()) == td, "cwd is outside the rig"
    assert os.path.join(m.HOME, ".cys", "state-generations").startswith(td + os.sep), "the generation store is outside the rig"
    try:
        c2_bak_fallback(td)
        c2_snapshot_fallback(td)
        c2_unrecoverable(td)
        c2_corrupt_prune(td)
        c2_missing_vs_corrupt(td)
        c2_breaker_corrupt(td)
        c3_unexplained_refused(td)
        c3_explained_tombstone_ok(td)
        c3_rebase_forces(td)
        c3_write_failure_no_silent(td)
        c5_liveness_from_status_json(td)
        c5_liveness_fallback_to_list(td)
        c5_readiness_structured_ack(td)
        c5_harness_allow_live_gate()
        p17_rmw_lock(td)
        aux_journal_corrupt(td)
        deploy_nested_lease(td)
        c2_degraded_persists_across_retry(td)
        c2_provenance_release(td)
        c5_malformed_list_holds(td)
        c5_empty_list_is_known(td)
        dept_corruption_matrix(td)
    finally:
        os.chdir(cwd0)
        m.HOME = home0
        shutil.rmtree(td, ignore_errors=True)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
