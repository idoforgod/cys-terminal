#!/usr/bin/env python3
"""★R3-2(0.14.42 · S27b H5): phoenix 가 `cys restore` 에 씌우는 상한이 **기동 단위 수 비례**인가(데몬 불요).

결함: `cys restore`(cys.rs run_restore)는 죽은 역할을 순차로 세운다. phoenix 는 그것을 **고정 90s** 로 감쌌다 →
좌석 8 · 기동 15s 에서 역할당 ~23s × 8 ≈ 184s 인 일이 90s 에 잘려 뒤 4역할이 유실(2/2회). 5번째 역할은 부트
도중 SIGKILL(반쪽 좌석)이고, 뒤 재시도는 침식된 topology 를 읽어 헛돌아 독약 fresh 강등(대화 유실)으로 끝났다.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_r32_restore_budget.py   (0 = 전건 PASS)
실패 방향:
  · 상한이 단위 수를 모르면(고정) 적색 · 0/1단위가 종전 90 과 다르면 적색(소규모 회귀)
  · 단위를 phoenix 의 target/need 로 세면 적색(`cys restore` 에는 --roles 필터가 없다 — 명시 --roles 가 절단 재현)
  · 앞선 회차가 상한에 걸렸는데(rc=124) 재시도가 같은 큰 상한을 받으면 적색(멈춤 꼬리 증폭)
  · 롤백 노브가 크래시하거나(inf·1e999) 90 미만을 받으면 적색
  · 단위 최악치가 cys.rs 의 구조(락 대기 TICK×4 · 틱 넘침 · in-seat→fresh 2회 · 선별 규칙)와 어긋나면 적색
    (⑦ 은 레포 체크아웃에서만 돈다 — 배포 팩에는 cys.rs 가 없다)
"""
import importlib.util
import json
import os
import re
import shutil
import sys
import tempfile
import time
from types import SimpleNamespace

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(HERE, ".."))
REPO = os.path.normpath(os.path.join(BIN, "..", ".."))
PH = os.path.join(BIN, "javis_phoenix.py")
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
sys.path.insert(0, BIN)
import javis_budget as B  # noqa: E402

_results = []


def check(name, cond):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name)


def _clean_env():
    for k in list(os.environ):
        if k.startswith("CYS_BUDGET_") or k == "PHOENIX_RESTORE_TIMEOUT_S":
            del os.environ[k]


def entry(role, agent="claude"):
    return {"role": role, "agent": agent, "session_id": "s-" + role, "cwd": "/tmp"}


def row(seat="occupied", exited=False):
    return {"surface": "surface:1", "exited": exited, "seat": seat}


ROLES8 = ["w%d" % i for i in range(1, 9)]


# ── ① 단위 계상 = run_restore 의 선별 ─────────────────────────────────────────
def t_units():
    U = m.restore_workload_units
    topo = {"entries": [entry(r) for r in ROLES8], "tombstones": []}
    check("① 8 죽은 역할 → 8단위", U(topo, {}) == 8)
    check("① 묘비 역할은 세지 않는다", U({"entries": topo["entries"], "tombstones": ["w1"]}, {}) == 7)
    tm = {"entries": topo["entries"] + [entry("master")], "tombstones": []}
    check("① master 는 include_master 일 때만", U(tm, {}) == 8 and U(tm, {}, include_master=True) == 9)
    check("① 가동 좌석(비빈)·구 데몬(seat 키 부재)은 건너뜀",
          U(topo, {"w1": [row()], "w2": [{"surface": "s", "exited": False}]}) == 6)
    check("① 빈 좌석 역할 = 2단위(in-seat 실패 → fresh 폴백)", U(topo, {"w3": [row("empty")]}) == 9)
    check("① exited 행은 죽음으로 본다", U(topo, {"w1": [row(exited=True)]}) == 8)
    check("① agent 미상 항목은 건너뜀(Rust 와 같다)",
          U({"entries": topo["entries"] + [{"role": "x", "agent": None}], "tombstones": []}, {}) == 8)
    check("① 손상 입력 무크래시", U({"entries": [None, 3, {"role": ""}, {"agent": "c"}]}, {"w1": None}) == 0)


# ── ② 상한 파생 ──────────────────────────────────────────────────────────────
def t_budget():
    T = lambda u, **kw: m.restore_spawn_timeout_s(u, **kw)[0]
    t0, t1, t4, t8 = T(0), T(1), T(4), T(8)
    check("② 8단위 상한 > 종전 90 ∧ ≥ 8 × 단위 최악치", t8 > 90 and t8 >= 8 * B.restore_unit_worst_s())
    check("② 8단위 상한 == javis_budget 파생값", t8 == B.cys_restore_outer_s(8))
    check("② 단조(1<4<8)", t1 < t4 < t8)
    check("② 0·1단위 == 종전 90(소규모 무회귀)", t0 == 90 and t1 == 90)
    check("② 행 의심(rc=124 이력) → 8단위여도 종전 90", T(8, hang_suspected=True) == 90)


# ── ③ 롤백 노브(env > 파일) ─────────────────────────────────────────────────
def t_knob():
    t8 = B.cys_restore_outer_s(8)
    T = lambda: m.restore_spawn_timeout_s(8)[0]
    home0 = m.HOME
    td = tempfile.mkdtemp(prefix="r32-knob-")
    try:
        m.HOME = td                       # 파일 노브 격리(실 ~/.cys 무접촉)
        cases = [("90", 90), ("30", 90), ("100000", t8), ("1e300", t8), ("200", 200)]
        for raw, want in cases:
            os.environ["PHOENIX_RESTORE_TIMEOUT_S"] = raw
            try:
                check("③ env %s → %s([90, 파생] clamp)" % (raw, want), T() == want)
            finally:
                del os.environ["PHOENIX_RESTORE_TIMEOUT_S"]
        for raw in ("inf", "1e999", "-inf", "nan", "-5", "0", "abc", ""):
            os.environ["PHOENIX_RESTORE_TIMEOUT_S"] = raw
            try:
                try:
                    got = T()
                except Exception as e:           # OverflowError 등 — 새 크래시 지점
                    got = "%s: %s" % (type(e).__name__, e)
                check("③ env %r → 무크래시·파생 유지" % raw, got == t8)
            finally:
                del os.environ["PHOENIX_RESTORE_TIMEOUT_S"]
        os.makedirs(os.path.join(td, ".cys"))
        kf = os.path.join(td, ".cys", m.RESTORE_TIMEOUT_KNOB_FILE)
        with open(kf, "w") as f:
            f.write("90\n")
        check("③ 파일 노브 90 → 90(데몬 재기동 없이 종전 거동)", T() == 90)
        os.environ["PHOENIX_RESTORE_TIMEOUT_S"] = "200"
        try:
            check("③ env 가 파일보다 우선", T() == 200)
        finally:
            del os.environ["PHOENIX_RESTORE_TIMEOUT_S"]
        with open(kf, "w") as f:
            f.write("1e999")
        check("③ 파일 손상값(inf) → 파생 유지", T() == t8)
    finally:
        m.HOME = home0
        shutil.rmtree(td, ignore_errors=True)


# ── ④ javis_budget 결손·스큐 → 종전 90(산식 사본 없음) ────────────────────────
def t_budget_missing():
    saved = m._BUDGET_MOD
    try:
        m._BUDGET_MOD = False
        check("④ 모듈 결손 → 8단위도 종전 90", m.restore_spawn_timeout_s(8)[0] == 90)
        m._BUDGET_MOD = SimpleNamespace(cys_restore_outer_s=lambda n: float("inf"))
        check("④ 비유한 파생값 → 종전 90(무크래시)", m.restore_spawn_timeout_s(8)[0] == 90)
        m._BUDGET_MOD = SimpleNamespace()
        check("④ 함수 부재(구 팩 스큐) → 종전 90", m.restore_spawn_timeout_s(8)[0] == 90)
    finally:
        m._BUDGET_MOD = saved


# ── ⑤ spawn_production 이 그 상한을 cys() 에 싣고 하트비트를 멈춘다 ─────────────
def t_spawn_and_heartbeat():
    seen = {}
    orig_cys, orig_log = m.cys, m.log

    def fake(*args, **kw):
        seen["timeout"] = kw.get("timeout")
        return SimpleNamespace(returncode=0, stdout="restore 완료", stderr="", stderr_raw="")
    try:
        m.cys = fake
        m.log = lambda s: None
        r = m.spawn_production("s", ["w1"], units=8)
        check("⑤ cys() timeout == 8단위 파생 · 반환에 실림", seen["timeout"] == B.cys_restore_outer_s(8)
              and r["timeout_s"] == seen["timeout"] and r["units"] == 8)
        m.spawn_production("s", ["w1"], units=8, hang_suspected=True)
        check("⑤ 행 의심 → cys() timeout 90", seen["timeout"] == 90)
        m.spawn_production("s", ROLES8)
        check("⑤ units 미지정(구 호출부) → pending 길이로", seen["timeout"] == B.cys_restore_outer_s(8))
        # 하트비트: 느린 cys 동안 주기 줄 · 반환 뒤엔 멈춘다
        lines = []
        m.log = lambda s: lines.append(s)
        saved_mod, saved_iv = m._BUDGET_MOD, m.SPAWN_HEARTBEAT_FALLBACK_S
        try:
            m._BUDGET_MOD, m.SPAWN_HEARTBEAT_FALLBACK_S = False, 0.2

            def slow(*a, **kw):
                time.sleep(0.9)
                return SimpleNamespace(returncode=0, stdout="", stderr="", stderr_raw="")
            m.cys = slow
            m.spawn_production("s", ["w1"], units=1)
            beats = [s for s in lines if s.startswith("spawn 진행 중")]
            n_after = len(beats)
            time.sleep(0.6)
            beats2 = [s for s in lines if s.startswith("spawn 진행 중")]
            check("⑤ 하트비트가 대기 중 주기 기록(≥2줄)", n_after >= 2)
            check("⑤ 하트비트는 반환과 함께 멈춘다", len(beats2) == n_after)
            check("⑤ 예산 줄이 상한·사유를 밝힌다", any(s.startswith("spawn 예산: cys restore 1단위 → 상한 90s")
                                                   for s in lines))
        finally:
            m._BUDGET_MOD, m.SPAWN_HEARTBEAT_FALLBACK_S = saved_mod, saved_iv
    finally:
        m.cys, m.log = orig_cys, orig_log


# ── ⑥ 실 run_restore 상태머신(데몬 불요 · CLI 대본) ──────────────────────────
class _Stop(Exception):
    pass


def run_scenario(name, restore_script, status_rows0=None, roles=None, topo_entries=None):
    """run_restore 를 실제로 돌리고 `cys restore` 호출마다 받은 timeout 을 모은다. 스폰 단계 끝(공유 lease
    해제 지점)에서 멈춘다 — 이 검체의 관심은 스폰 상한뿐이다."""
    td = tempfile.mkdtemp(prefix="r32-run-%s-" % name)
    sock = os.path.join(td, "cys.sock")
    saved = (m.cys, m.time.sleep, m._emit_evt, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
             m._release_shared_restore_lease, m.log)
    state = {"rows": dict(status_rows0 or {}), "restore_calls": [], "launch_calls": 0}
    topo_path = os.path.join(td, "topology.json")

    def write_topo(ents):
        with open(topo_path, "w") as f:
            json.dump({"schema_version": 1, "tombstones": [], "tombstones_rev": 0, "updated_at": 1,
                       "entries": ents}, f)
    write_topo(topo_entries if topo_entries is not None else [entry(r) for r in ROLES8])

    def fake_cys(*args, socket=None, timeout=25):
        verb = args[0]
        if verb == "status":
            rows = []
            for role, rs in state["rows"].items():
                for s in rs:
                    rows.append({"surface_ref": s.get("surface", "surface:9"), "role": role,
                                 "exited": s.get("exited", False), "seat": s.get("seat"),
                                 "agent_alive": True})
            return SimpleNamespace(returncode=0, stdout=json.dumps({"daemon": {"started_at": 1},
                                                                    "surfaces": rows}), stderr="")
        if verb == "restore":
            state["restore_calls"].append(timeout)
            rc = restore_script(len(state["restore_calls"]), state, write_topo)
            out = "TIMEOUT %ss" % timeout if rc == 124 else "restore 완료"
            return SimpleNamespace(returncode=rc, stdout="", stderr=out, stderr_raw="")
        if verb == "launch-agent":
            state["launch_calls"] += 1
            return SimpleNamespace(returncode=1, stdout="", stderr="fake fresh fail")
        if verb in ("list", "tombstone"):
            return SimpleNamespace(returncode=0, stdout="", stderr="")
        raise AssertionError("unexpected cys call %r" % (args,))

    def rel(reason):
        if str(reason).startswith("spawn 완료"):
            raise _Stop()
        return saved[5](reason)
    try:
        m.cys = fake_cys
        m.time.sleep = lambda s: None
        m._emit_evt = lambda *a, **k: None
        m._release_shared_restore_lease = rel
        m.log = lambda s: None
        try:
            m.run_restore(sock, ticket="r32-" + name, stub=False, no_breaker=True, roles=roles,
                          print_result=False)
        except _Stop:
            pass
        return state
    finally:
        (m.cys, m.time.sleep, m._emit_evt, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
         m._release_shared_restore_lease, m.log) = saved
        shutil.rmtree(td, ignore_errors=True)


def t_run_restore():
    o = B.cys_restore_outer_s

    def all_up(k, st, wt):
        for r in ROLES8:
            st["rows"][r] = [row()]
        return 0
    s = run_scenario("roles", all_up, roles=["w1"])
    check("⑥ 명시 --roles w1 · 8역할 사망 → 첫 상한은 8단위(%ds) — target 1개(90s) 아님" % o(8),
          s["restore_calls"][:1] == [o(8)])

    def hang(k, st, wt):
        return 124                      # 멈춤 — 아무것도 서지 않는다(침식도 없다)
    s = run_scenario("hang", hang)
    check("⑥ 멈춤: 첫 회 %ds → 재시도 3회는 종전 90s(꼬리 증폭 0)" % o(8),
          s["restore_calls"] == [o(8), 90, 90, 90])
    check("⑥ 멈춤 뒤 독약 강등 경로는 불변(8역할 fresh 시도)", s["launch_calls"] == 8)

    def partial(k, st, wt):
        if k == 1:                      # w1..w6 가동 · w7 빈 좌석 · w8 무 — topology 는 살아 있는 surface 로 침식
            for r in ROLES8[:6]:
                st["rows"][r] = [row()]
            st["rows"]["w7"] = [row("empty")]
            wt([entry(r) for r in ROLES8[:7]])
            return 1
        st["rows"]["w8"] = [row()]
        return 0
    s = run_scenario("partial", partial)
    check("⑥ 비멈춤 재시도는 침식된 topology 로 다시 센다(8단위 → w7 빈 좌석 2단위 = %ds)" % o(2),
          s["restore_calls"] == [o(8), o(2)])

    def up(k, st, wt):
        for r in ROLES8:
            st["rows"][r] = [row()]
        return 0
    s = run_scenario("seat", up, status_rows0={"w3": [row("empty")]})
    check("⑥ 첫 회 빈 좌석은 2단위로 센다(8역할 + 1 = %ds)" % o(9), s["restore_calls"] == [o(9)])
    s = run_scenario("master", up, topo_entries=[entry(r) for r in ROLES8[:2]] + [entry("master")])
    check("⑥ master 는 비 include 실행에서 단위에 들지 않는다(첫 회 %ds)" % o(2), s["restore_calls"][:1] == [o(2)])


# ── ⑦ cys.rs 구조 핀(반증 가능 — 파이썬 단위 최악치가 기대는 Rust 사실) ───────────
def _slice(src, start, end):
    i = src.find(start)
    if i < 0:
        return None
    j = src.find(end, i + len(start))
    return src[i:j if j > 0 else len(src)]


def t_rust_pins():
    rs = os.path.join(REPO, "src", "bin", "cys.rs")
    hd = os.path.join(REPO, "src", "bin", "cysd", "handlers.rs")
    gv = os.path.join(REPO, "src", "bin", "cysd", "governance.rs")
    if not all(os.path.isfile(p) for p in (rs, hd, gv)):
        print("SKIP ⑦ cys.rs 구조 핀 — 레포 체크아웃 밖(배포 팩에는 Rust 소스가 없다 · 검사 수에 넣지 않음)")
        return
    src = open(rs, encoding="utf-8").read()
    const = {k: int(v) for k, v in re.findall(r"const (BUDGET_[A-Z_]+): u64 = (\d+);", src)}
    tick = const["BUDGET_TICK_MS"] / 1000.0
    lock = _slice(src, "fn acquire_launch_lock() -> Option<LockHold> {", "\n}\n") or ""
    check("⑦ 락 대기 = BUDGET_TICK_MS × 4 ↔ python launch_lock_wait_max_s",
          "from_millis(BUDGET_TICK_MS * 4)" in lock and B.launch_lock_wait_max_s() == 4 * tick)
    want = (const["BUDGET_RESTORE_CAP_SECS"] + tick + const["BUDGET_TRUST_SETTLE_SECS"]
            + const["BUDGET_POST_MARKER_SETTLE_SECS"] + const["BUDGET_ACK_WAIT_SECS"] + tick
            + B.leaf("RPC_SLACK_S"))
    check("⑦ 기동 1회 최악치 = Rust 상수 합(캡+틱+신뢰+안착+ack+틱) + RPC 여유 (%.1f)" % want,
          abs(B.restore_boot_once_worst_s() - want) < 1e-9)
    # 단위 = 락 대기 + 기동 1회. 락 항이 빠져도 위 두 핀(락 함수값·기동 1회값)은 그대로 초록이라 단위 자체를 Rust
    #   상수에서 다시 계산해 대조한다(구현 단계 변이 M5 '락 항 삭제' 가 이 한 줄 없이는 생존했다).
    check("⑦ 기동 1단위 = 락 대기(TICK×4) + 기동 1회 — Rust 상수로 다시 계산한 값 (%.1f)" % (4 * tick + want),
          abs(B.restore_unit_worst_s() - (4 * tick + want)) < 1e-9)
    boot = _slice(src, "fn boot_agent_on_surface(", "\nfn inject_directive_after_ready(") or ""
    inj = _slice(src, "fn inject_directive_after_ready(", "\n}\n") or ""
    rd = re.search(r"while std::time::Instant::now\(\) < deadline \{\s*\n\s*std::thread::sleep\("
                   r"std::time::Duration::from_millis\(BUDGET_TICK_MS\)\);", boot)
    ak = re.search(r"while std::time::Instant::now\(\) < ack_deadline \{\s*\n\s*std::thread::sleep\("
                   r"std::time::Duration::from_millis\(BUDGET_TICK_MS\)\);", inj)
    check("⑦ readiness·ack 루프 = 데드라인 검사 후 틱 1회 수면(넘침 ≤ 틱 1 — python 이 틱 2개를 진다)",
          bool(rd) and bool(ak))
    rr = _slice(src, "fn run_restore(", "\n}\n") or ""
    check("⑦ run_restore 에 역할 필터가 없다(phoenix 는 target 이 아니라 topology 로 센다)",
          rr.startswith("fn run_restore(cwd: Option<String>, include_master: bool, no_resume: bool) -> i32"))
    code = "\n".join(ln for ln in rr.splitlines() if not ln.strip().startswith("//"))   # 주석 = 이력 서술
    check("⑦ 역할당 기동 지점 = in-seat 1 + fresh 1(빈 좌석 = 2단위)",
          code.count("acquire_launch_lock()") == 1 and code.count("boot_agent_on_surface(") == 1
          and code.count("run_launch_agent_opts(") == 1)
    filt = ['let saved = topo["saved"]', "tombstones.contains(role)", 'role == "master" && !include_master',
            "live.contains(role)", 'let Some(agent) = entry["agent"].as_str() else',
            '.filter(|e| e["seat"].as_str() != Some("empty"))']
    check("⑦ run_restore 선별 규칙 = python restore_workload_units 의 거울(묘비·master·생존·agent·빈 좌석)",
          all(f in rr for f in filt))
    hsrc, gsrc = open(hd, encoding="utf-8").read(), open(gv, encoding="utf-8").read()
    topo_h = _slice(hsrc, '"system.topology" => {', "Reply::Single") or ""
    ld = _slice(gsrc, "pub fn load_topology(", "\n}\n") or ""
    check("⑦ system.topology.saved = 디스크 topology.json(phoenix read_topology 와 같은 파일)",
          "let saved = crate::governance::load_topology(daemon);" in topo_h
          and 'dir.join("topology.json")' in ld and '["entries"]' in ld)


# ── ⑧ 소스·예산 핀 ──────────────────────────────────────────────────────────
def t_source_pins():
    src = open(PH, encoding="utf-8").read()
    body = src[src.index("def spawn_production("):src.index("def spawn_fresh_production(")]
    check("⑧ spawn_production 에 하드코딩 timeout=90 없음", re.search(r"timeout\s*=\s*90\b", body) is None)
    check("⑧ 호출부가 회차마다 단위를 다시 세고 행 의심을 넘긴다",
          "_restore_units_now(socket, _units_view, include_master, entries, _tombstones)" in src
          and "hang_suspected=_restore_hang" in src
          and '_restore_hang = _restore_hang or res.get("rc") == 124' in src)
    labels = [p[0] for p in B.parity_pairs()]
    check("⑧ javis_budget 파리티에 restore 참조 쌍", any("cys restore" in x for x in labels))
    d = B.table()["derived"]
    check("⑧ 표에 로스터 총합 단일값(오독 소지) 없음 · 참조값은 REF 로 표기",
          "CYS_RESTORE_OUTER_S" not in d and d.get("CYS_RESTORE_OUTER_REF_S") == B.cys_restore_outer_s(8)
          and d.get("RESTORE_UNIT_WORST_S") == B.restore_unit_worst_s())


def main():
    _results.clear()
    _clean_env()
    # ★파일 노브 격리(전 검사): 롤백 노브는 env 뿐 아니라 ~/.cys/phoenix-restore-timeout-s 도 읽는다. 오너가
    #   롤백(파일에 90)해 둔 기계에서 이 검체를 돌리면 ②⑤⑥ 이 호스트 파일을 읽어 거짓 적색이 된다(실측 10 FAIL).
    #   모듈의 HOME 을 빈 임시 디렉터리로 돌려 호스트 ~/.cys 를 읽지도 쓰지도 않는다(t_knob 은 자체 격리를 덧댄다).
    home0, iso_home = m.HOME, tempfile.mkdtemp(prefix="r32-home-")
    m.HOME = iso_home
    try:
        for t in (t_units, t_budget, t_knob, t_budget_missing, t_spawn_and_heartbeat, t_run_restore,
                  t_rust_pins, t_source_pins):
            try:
                t()
            except Exception as e:  # 검체 자체의 예외도 적색으로 센다(조용한 통과 금지)
                check("%s 예외 없이 완주 (%s: %s)" % (t.__name__, type(e).__name__, e), False)
    finally:
        m.HOME = home0
        shutil.rmtree(iso_home, ignore_errors=True)
    ok = all(_results)
    print("=== %d checks · FAIL %d ===" % (len(_results), _results.count(False)))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
