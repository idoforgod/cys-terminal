#!/usr/bin/env python3
"""CU-6B 죽은-에이전트 페인 **관찰자** 테스트(리포 커밋·데몬 불요·라이브 무접촉).

검증 대상은 설계 정본 §4 CU-6B 의 술어 5종과 ADR-5(회수기=순수 관찰자·스폰 권위 phoenix 단독)다.
전부 tempdir 에서 순수 함수와 합성 `cys status --json` 픽스처로만 돈다 — 실 데몬·실 팩 무접촉.

  ① 합성 status 6케이스: null / false-무이력 / false-유이력 / grace내 / exited / 정상
  ② 원장 손상 격리(_isolate_corrupt 재사용·침묵 리셋 아님)
  ③ 원장 upsert 왕복(first_seen 고정·last_seen 갱신·저장→로드 동일)
  ④ ADR-5 자기증명: 탐지 경로가 부르는 cys 서브커맨드는 `status --json` **하나뿐**
     (close-surface·restore·node-recover 호출 0 — 관찰자가 행동하지 않음을 기계로 못박는다)

실행: python3 cysjavis-pack/bin/tests/test_dead_agent_detect.py   (0=전건 PASS)
"""
import importlib.util, json, os, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []


def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))


def _ph_home(td, name):
    """tempdir 안에 격리 소켓 경로 + phoenix home 을 만든다(라이브 상태 디렉터리 무접촉)."""
    sd = os.path.join(td, name)
    home = os.path.join(sd, "phoenix")
    os.makedirs(home, exist_ok=True)
    return os.path.join(sd, "cys.sock"), home


NOW = 1_000_000  # 고정 기준 시각(테스트 결정론 — 실시간 의존 0)
GRACE = m.DEADAGENT_GRACE_DEFAULT  # 300s


def _surface(ref, **over):
    """org.status surfaces 엔트리 최소 모사(실 데몬 handlers.rs 필드명 기준)."""
    s = {"surface_ref": ref, "role": "worker", "agent": "claude",
         "agent_alive": True, "exited": False, "created_at": NOW - 10_000}
    s.update(over)
    return s


# ─────────────────────── ① 합성 status 6케이스(순수 판정) ───────────────────────

def case_six_fixtures():
    """술어 5종을 한 번에 태운다 — 대상은 'false + 이력 + grace 경과 + agent 등록 + 미종료' 하나뿐."""
    entries = [
        _surface("surface:1", agent_alive=None),                       # ⓐ null(미상) — 절대 비대상
        _surface("surface:2", agent_alive=False),                      # ⓑ false·이력 없음 — 비대상
        _surface("surface:3", agent_alive=False),                      # ⓒ false·이력 있음 — ★대상
        _surface("surface:4", agent_alive=False,
                 created_at=NOW - (GRACE // 2)),                       # ⓓ grace 내(부팅 창) — 비대상
        _surface("surface:5", agent_alive=False, exited=True),         # ⓔ exited 잔재(C6 소관) — 비대상
        _surface("surface:6"),                                          # ⓕ 정상 생존 — 비대상
    ]
    ledger = {
        "surface:3": {"first_seen": NOW - 9_000, "last_seen": NOW - 400},
        "surface:4": {"first_seen": NOW - 100, "last_seen": NOW - 100},
        "surface:5": {"first_seen": NOW - 9_000, "last_seen": NOW - 400},
        "surface:6": {"first_seen": NOW - 9_000, "last_seen": NOW},
    }
    got = m.detect_dead_agent_entries(entries, ledger, now=NOW, grace=GRACE)
    refs = [d["surface"] for d in got]
    check("6케이스: 대상은 false+이력+grace경과 하나뿐", refs == ["surface:3"], "detected=%s" % refs)
    check("6케이스: null(미상) 비대상(오살 차단)", "surface:1" not in refs)
    check("6케이스: false-무이력 비대상(부팅 pane 보호)", "surface:2" not in refs)
    check("6케이스: grace 내 비대상", "surface:4" not in refs)
    check("6케이스: exited 비대상(C6 stale 과 이중취급 금지)", "surface:5" not in refs)
    check("6케이스: 정상 생존 비대상", "surface:6" not in refs)
    if refs == ["surface:3"]:
        d = got[0]
        check("탐지 레코드에 관측 근거 동봉", d.get("age_secs") == 10_000 and d.get("agent") == "claude"
              and d.get("agent_seen_first") == NOW - 9_000, "rec=%s" % d)


def case_extra_predicates():
    """경계 술어 — agent 미등록(빈 셸)·created_at 부재·grace 경계(동일 시각)."""
    bare = _surface("surface:10", agent_alive=False, agent=None)
    got = m.detect_dead_agent_entries([bare], {"surface:10": {"first_seen": 1, "last_seen": 2}},
                                      now=NOW, grace=GRACE)
    check("agent 미등록(빈 셸) 비대상", got == [], "got=%s" % got)

    nocreate = _surface("surface:11", agent_alive=False)
    nocreate.pop("created_at")
    got = m.detect_dead_agent_entries([nocreate], {"surface:11": {"first_seen": 1, "last_seen": 2}},
                                      now=NOW, grace=GRACE)
    check("created_at 부재 비대상(모르면 안 센다)", got == [], "got=%s" % got)

    edge = _surface("surface:12", agent_alive=False, created_at=NOW - GRACE)
    got = m.detect_dead_agent_entries([edge], {"surface:12": {"first_seen": 1, "last_seen": 2}},
                                      now=NOW, grace=GRACE)
    check("grace 경계(정확히 300s) 비대상(초과일 때만 대상)", got == [], "got=%s" % got)

    edge2 = _surface("surface:13", agent_alive=False, created_at=NOW - GRACE - 1)
    got = m.detect_dead_agent_entries([edge2], {"surface:13": {"first_seen": 1, "last_seen": 2}},
                                      now=NOW, grace=GRACE)
    check("grace 초과 1초 = 대상", [d["surface"] for d in got] == ["surface:13"], "got=%s" % got)

    # 원장 이력이 first_seen=None 인 반쪽 레코드는 이력 없음으로 본다(보수적).
    got = m.detect_dead_agent_entries([_surface("surface:14", agent_alive=False)],
                                      {"surface:14": {"first_seen": None, "last_seen": 5}},
                                      now=NOW, grace=GRACE)
    check("반쪽 원장 레코드(first_seen=None) 비대상", got == [], "got=%s" % got)


def case_grace_env():
    """env CYS_DEADAGENT_GRACE_SECS 우선·부적격 값은 기본값 폴백(침묵 무시 아님)."""
    old = os.environ.get("CYS_DEADAGENT_GRACE_SECS")
    try:
        os.environ["CYS_DEADAGENT_GRACE_SECS"] = "60"
        check("env grace 반영(60s)", m.deadagent_grace_secs() == 60, str(m.deadagent_grace_secs()))
        os.environ["CYS_DEADAGENT_GRACE_SECS"] = "그냥문자"
        check("env grace 비정수 → 기본 300s", m.deadagent_grace_secs() == m.DEADAGENT_GRACE_DEFAULT)
        os.environ["CYS_DEADAGENT_GRACE_SECS"] = "-5"
        check("env grace 음수 → 기본 300s(보호창 무력화 금지)",
              m.deadagent_grace_secs() == m.DEADAGENT_GRACE_DEFAULT)
        del os.environ["CYS_DEADAGENT_GRACE_SECS"]
        check("env 부재 → 기본 300s", m.deadagent_grace_secs() == m.DEADAGENT_GRACE_DEFAULT)
    finally:
        if old is None:
            os.environ.pop("CYS_DEADAGENT_GRACE_SECS", None)
        else:
            os.environ["CYS_DEADAGENT_GRACE_SECS"] = old


# ─────────────────────── ② 원장 손상 격리 / ③ upsert 왕복 ───────────────────────

def case_ledger_roundtrip(td):
    sock, home = _ph_home(td, "ledger")
    path = m.reap_ledger_path(sock)
    check("원장 경로 = phoenix home 하위", os.path.dirname(path) == os.path.realpath(home),
          "%s vs %s" % (os.path.dirname(path), home))
    check("부재 원장 로드 = 빈 dict(fresh 정상)", m.load_reap_ledger(path) == {})

    # upsert 1회차 — 살아있는 것만 기록된다.
    entries = [_surface("surface:1"), _surface("surface:2", agent_alive=False),
               _surface("surface:3", agent_alive=None)]
    l1 = m.ledger_observe_alive({}, entries, now=NOW)
    check("upsert: agent_alive=True 만 기록", sorted(l1) == ["surface:1"], "ledger=%s" % sorted(l1))
    check("upsert: first_seen=last_seen(최초)", l1["surface:1"] == {"first_seen": NOW, "last_seen": NOW})

    # 2회차 — first_seen 고정·last_seen 갱신, 입력 원장은 불변(순수).
    l2 = m.ledger_observe_alive(l1, entries, now=NOW + 500)
    check("upsert: first_seen 고정·last_seen 갱신",
          l2["surface:1"] == {"first_seen": NOW, "last_seen": NOW + 500}, "%s" % l2["surface:1"])
    check("upsert: 입력 원장 불변(순수 함수)", l1["surface:1"]["last_seen"] == NOW, "%s" % l1)

    # 저장→로드 왕복 동일.
    m.save_reap_ledger(path, l2)
    check("왕복: 저장→로드 동일", m.load_reap_ledger(path) == l2, "%s" % m.load_reap_ledger(path))
    saved = json.load(open(path))
    check("원장 봉투(surfaces/updated_at)", "surfaces" in saved and "updated_at" in saved,
          "keys=%s" % sorted(saved))


def case_ledger_corrupt(td):
    sock, home = _ph_home(td, "corrupt")
    path = m.reap_ledger_path(sock)
    with open(path, "w", encoding="utf-8") as f:
        f.write("{ 손상된 원장 ]]]")
    got = m.load_reap_ledger(path)
    check("손상 원장 → 빈 dict 로 강등(크래시 아님)", got == {}, "got=%s" % got)
    corr = [f for f in os.listdir(home) if f.startswith(m.REAP_LEDGER_NAME + ".corrupt-")]
    check("손상 원장 → .corrupt-* 격리(_isolate_corrupt 재사용)", len(corr) == 1, "corr=%s" % corr)
    check("손상 원장 → 원본 자리 비움(침묵 덮어쓰기 아님)", not os.path.exists(path))

    # 최상위가 object 가 아닌 유효 JSON(배열)도 손상 취급.
    with open(path, "w", encoding="utf-8") as f:
        f.write("[1,2,3]")
    check("배열 원장(스키마 위반) → 빈 dict", m.load_reap_ledger(path) == {})
    corr2 = [f for f in os.listdir(home) if f.startswith(m.REAP_LEDGER_NAME + ".corrupt-")]
    check("스키마 위반도 격리", len(corr2) == 2, "corr=%s" % corr2)


# ─────────────────────── ④ ADR-5: 관찰자 자기증명(행동 0) ───────────────────────

def case_observer_no_action(td):
    """c6_detect_dead_agent_surfaces 가 실제로 부르는 cys 서브커맨드를 전수 기록해 대조한다.
    close-surface·restore·node-recover 가 한 번이라도 불리면 ADR-5 위반이므로 즉시 FAIL 한다."""
    sock, home = _ph_home(td, "observer")
    calls = []
    live = [_surface("surface:1"), _surface("surface:2", agent_alive=False)]
    dead = [_surface("surface:1", agent_alive=False), _surface("surface:2", agent_alive=False)]
    payload = {"surfaces": live}

    def fake_cys(*args, **kw):
        calls.append(tuple(args))
        return m._CapR(returncode=0, stdout=json.dumps(payload))

    orig = m.cys
    m.cys = fake_cys
    try:
        # 1틱: surface:1 이 살아있는 것을 관측 → 원장에 이력이 생긴다. 탐지 0.
        got1 = m.c6_detect_dead_agent_surfaces(sock, now=NOW)
        check("1틱: 살아있는 동안 탐지 0", got1 == [], "got=%s" % got1)
        led = m.load_reap_ledger(m.reap_ledger_path(sock))
        check("1틱: 살아있는 pane 만 원장 등재", sorted(led) == ["surface:1"], "ledger=%s" % sorted(led))

        # 2틱: 같은 pane 의 agent 가 죽었다(셸은 생존) → 이력이 있으므로 탐지 대상.
        payload["surfaces"] = dead
        got2 = m.c6_detect_dead_agent_surfaces(sock, now=NOW + 1_000)
        refs = [d["surface"] for d in got2]
        check("2틱: 이력 있는 pane 만 죽음 판정", refs == ["surface:1"], "detected=%s" % refs)

        # ADR-5 자기증명 — 부른 서브커맨드가 status --json 뿐인가.
        subs = sorted({c[0] for c in calls if c})
        check("ADR-5: 호출 서브커맨드 = status 뿐", subs == ["status"], "subs=%s calls=%s" % (subs, calls))
        forbidden = [c for c in calls if c and c[0] in ("close-surface", "restore", "node-recover",
                                                        "new-surface", "launch-agent")]
        check("ADR-5: 스폰·close 계열 호출 0", forbidden == [], "forbidden=%s" % forbidden)

        # status 불신(rc≠0) = 빈 목록(fail-safe — 전원 사망 추정 금지).
        def bad_cys(*args, **kw):
            calls.append(tuple(args))
            return m._CapR(returncode=1, stdout="")

        m.cys = bad_cys
        check("status 불신 → 탐지 0(fail-safe)", m.c6_detect_dead_agent_surfaces(sock, now=NOW) == [])
    finally:
        m.cys = orig


def main():
    with tempfile.TemporaryDirectory() as td:
        case_six_fixtures()
        case_extra_predicates()
        case_grace_env()
        case_ledger_roundtrip(td)
        case_ledger_corrupt(td)
        case_observer_no_action(td)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
