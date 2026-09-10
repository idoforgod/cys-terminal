#!/usr/bin/env python3
"""★F-1(0.14.31): fresh 예상·주입 증거·검증·저널 이관 계약(데몬·네트워크 불요).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_fresh_honest.py (0=전건 PASS)
"""
import copy
import importlib.util, json, os, shutil, sys, tempfile

# 기존 시나리오 하네스처럼 모듈만 적재(바이트코드 파일 생성 방지).
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

_results = []
def check(name, cond):
    _results.append(cond); print(("PASS " if cond else "FAIL ") + name)


def captured_path(entry):
    """Rust format 문자열을 정규화 없이 관측하고 exists 패치를 반드시 복원한다."""
    paths = []
    original = m.os.path.exists
    def exists(path):
        paths.append(path)
        return False
    try:
        m.os.path.exists = exists
        result = m.fresh_expected(entry)
    finally:
        m.os.path.exists = original
    return result, paths


def main():
    _results.clear()
    td = tempfile.mkdtemp(prefix="phoenix-fresh-")
    try:
        # A: Rust claude_project_component의 ASCII 치환.
        for name, cwd, expected in (
            ("Users example", "/Users/x/Desktop/ProjX", "-Users-x-Desktop-ProjX"),
            ("tmp example", "/tmp/a.b_c", "-tmp-a-b-c"),
            ("empty", "", ""),
            ("None", None, ""),
            ("ASCII only", "A-z09/한é_", "A-z09----"),
        ):
            check("munge " + name, m._claude_project_component(cwd) == expected)

        # B: run_restore skips agentless entries; resolve_resume_suffix trims only
        # the blank-ID test, uses Path::exists, and preserves empty config/cwd.
        check("agent missing", m.fresh_expected({}) == (False, ""))
        for agent in ("gemini", "codex"):
            check(agent + " empty sid", m.fresh_expected({"agent": agent, "session_id": ""}) == (False, ""))
        for name, sid in (("empty", ""), ("blank", " \t ")):
            check("claude " + name + " sid", m.fresh_expected({"agent": "claude", "session_id": sid}) == (True, "no_session"))
        entry = {"agent": "claude", "session_id": "sid", "claude_config_dir": td, "cwd": "/tmp/a.b_c"}
        check("claude missing session file", m.fresh_expected(entry) == (True, "no_session_file"))
        path = os.path.join(td, "projects", "-tmp-a-b-c", "sid.jsonl")
        os.makedirs(os.path.dirname(path))
        with open(path, "w", encoding="utf-8") as f:
            f.write("{}\n")
        check("claude existing session file", m.fresh_expected(entry) == (False, ""))
        os.remove(path)
        os.mkdir(path)
        check("claude session directory exists", m.fresh_expected(entry) == (False, ""))
        os.rmdir(path)
        if os.name != "nt":
            result, paths = captured_path(dict(entry, session_id=" sid "))
            check("claude untrimmed sid path", result == (True, "no_session_file")
                  and len(paths) == 1 and paths[0].endswith("/ sid .jsonl"))

        original_account_dir = os.environ.get("CYS_ACCOUNT_DIR")
        try:
            os.environ["CYS_ACCOUNT_DIR"] = td
            check("default config honors CYS_ACCOUNT_DIR", m._default_claude_config_dir() == td)
            no_cfg = {k: v for k, v in entry.items() if k != "claude_config_dir"}
            check("missing cfg env session missing", m.fresh_expected(no_cfg) == (True, "no_session_file"))
            with open(path, "w", encoding="utf-8") as f:
                f.write("{}\n")
            check("missing cfg env session present", m.fresh_expected(no_cfg) == (False, ""))
            default = os.path.join(os.path.expanduser("~"), ".cys", "claude")
            os.environ.pop("CYS_ACCOUNT_DIR", None)
            check("default config env unset", m._default_claude_config_dir() == default)
            os.environ["CYS_ACCOUNT_DIR"] = ""
            check("default config env empty", m._default_claude_config_dir() == default)
            # 순수 문자열 비교(exists 패치 · Rust format 그대로) — 플랫폼 무관이라 nt 가드를 두지 않는다.
            result, paths = captured_path(dict(entry, claude_config_dir=""))
            check("empty cfg keeps empty prefix", result == (True, "no_session_file")
                  and paths == ["/projects/-tmp-a-b-c/sid.jsonl"])
        finally:
            if original_account_dir is None:
                os.environ.pop("CYS_ACCOUNT_DIR", None)
            else:
                os.environ["CYS_ACCOUNT_DIR"] = original_account_dir
        for name, candidate in (
            ("missing", {k: v for k, v in entry.items() if k != "cwd"}),
            ("empty", dict(entry, cwd="")),
        ):
            result, paths = captured_path(candidate)
            check(name + " cwd keeps double slash", result == (True, "no_session_file")
                  and paths == ["%s/projects//%s.jsonl" % (td, "sid")])

        # C/D: CLI 안정 토큰 및 저널 증거 파싱.
        injected = "reinjected 123 bytes → surface:3 (worker)"
        ack = "디렉티브 생존 확인 (ACK 수신)"
        backend = "reinjected 123 bytes → surface:3 (BACKEND)"
        for name, rc, stdout, stderr, expected in (
            ("nonzero", 1, "x", "", "fail"),
            ("ACK", 0, "디렉티브 생존 확인 (ACK 수신) — 재주입 불필요", "", "ack"),
            ("direct injection", 0, injected, "", "injected"),
            ("queued injection", 0, injected, "[inject] 사람 입력 감지 — 입력을 멈추면 큐가 배달합니다(--queued 1회 전환, surface:3)", "queued"),
            ("empty shell", 0, "빈 셸(라이브 에이전트 부재) — check reinject skip (surface:3)", "", "skip"),
            ("empty output", 0, "", "", "unknown"),
            ("None output", 0, None, None, "unknown"),
            ("BACKEND queued", 0, backend, "--queued", "queued"),
            ("BACKEND injected", 0, backend, "", "injected"),
            ("ACK queued", 0, ack, "--queued", "queued"),
            ("ACK contradiction", 0, ack, "[reinject] ACK 없음 (6s)", "unknown"),
            ("ACK not line start", 0, "x " + ack, "", "unknown"),
            ("injected not line start", 0, "x " + injected, "", "unknown"),
            ("ACKNOWLEDGED", 0, "ACKNOWLEDGED reinjected", "", "unknown"),
            ("failed ACK", 1, ack, "", "fail"),
            ("failed queued ACK", 1, ack, "--queued", "fail"),
            ("ACK later line", 0, "header\n" + ack, "", "ack"),
            ("injected later line", 0, "header\n" + injected, "", "injected"),
            ("ACK before injected", 0, injected + "\n" + ack, "", "ack"),
            ("injected before skip", 0, "check reinject skip\n" + injected, "", "injected"),
            ("contradiction before injected", 0, ack + "\n" + injected, "[reinject] ACK 없음", "unknown"),
        ):
            check("classify " + name, m.classify_reinject_result(rc, stdout, stderr) == expected)
        for name, evidence, expected in (
            ("ack", "reinject rc=0 kind=ack 디렉티브 생존 확인", "ack"),
            ("legacy", "reinject rc=0 something", "unknown"),
            ("None", None, "unknown"),
            ("skip colon", "reinject skip kind=skip: empty shell", "skip"),
            ("negative rc", "reinject rc=-1 kind=fail", "fail"),
            ("ack suffix", "reinject rc=0 kind=ack_bad", "unknown"),
            ("ack digit suffix", "reinject rc=0 kind=ack1", "unknown"),
            ("intervening text", "reinject rc=0 something kind=ack", "unknown"),
            ("prefix text", "foo reinject rc=0 kind=ack", "unknown"),
        ):
            check("reinject kind " + name, m._reinject_kind(evidence) == expected)

        # E: 매 케이스는 독립 복사로 단 하나의 증거 조건만 바꾼다.
        rr = {"stages": {"reinject": {"done": True, "evidence": "reinject rc=0 kind=ack …"},
                         "g2_ack": {"done": False}},
              "observed_sid": "new1", "observed_sid_source": "registered",
              # ★리뷰 R3: 증거는 주입 **직전·직후** 모두 그 세션이 결속돼 있을 때만 귀속된다.
              "reinject_sid": "new1", "reinject_sid_before": "new1",
              "fresh_pre_sids": ["old"], "fresh_pre_dir": td}
        session_file_exists = lambda p: p.endswith("new1.jsonl")
        row = {"exited": False, "agent_alive": True, "gate_pending": None, "registered_session_id": "new1"}
        outcome, missing, ev = m.f1_fresh_verify(rr, row, session_file_exists)
        check("verify full ACK evidence", outcome == "fresh" and missing is None
              and ev["gate_cleared"] is True and ev["reinject_kind"] == "ack" and ev["g2_ack"] is False
              and ev["provenance"] == "new" and ev["observed_sid"] == "new1"
              and ev["observed_sid_source"] == "registered"
              and ev["registered_now"] == "new1" and ev["reinject_sid"] == "new1"
              and ev["reinject_sid_before"] == "new1")
        for kind in ("injected", "queued", "skip", "unknown"):
            candidate = copy.deepcopy(rr)
            candidate["stages"]["reinject"]["evidence"] = "reinject rc=0 kind=" + kind
            outcome, missing, ev = m.f1_fresh_verify(candidate, row, session_file_exists)
            if kind == "injected":
                check("verify injected", outcome == "fresh" and missing is None and ev["reinject_kind"] == kind)
            else:
                check("verify rejects " + kind, outcome == "unverified" and "주입 증거" in (missing or "")
                      and ev["reinject_kind"] == kind)
        for name, kind in (("absent", "missing"), ("not done", "fail")):
            candidate = copy.deepcopy(rr)
            if kind == "missing":
                candidate["stages"].pop("reinject")
            else:
                candidate["stages"]["reinject"]["done"] = False
            outcome, missing, ev = m.f1_fresh_verify(candidate, row, session_file_exists)
            check("verify reinject " + name, outcome == "unverified" and ev["reinject_kind"] == kind
                  and "주입 증거" in (missing or ""))
        for name, candidate, reason, gate_cleared in (
            ("row None", None, "status 행 부재", None),
            ("gate key absent", {k: v for k, v in row.items() if k != "gate_pending"}, "키 부재", None),
            ("folder trust", dict(row, gate_pending={"gate": "folder-trust"}), "관문 보류", False),
            ("stale gate", dict(row, gate_pending={"gate": "gate_pending_stale"}), "관문 보류", False),
            ("agent dead", dict(row, agent_alive=False), "agent_alive≠true", True),
            ("agent unknown", dict(row, agent_alive=None), "agent_alive≠true", True),
            ("exited", dict(row, exited=True), "exited≠false", True),
            ("exited key absent", {k: v for k, v in row.items() if k != "exited"}, "exited≠false", True),
        ):
            outcome, missing, ev = m.f1_fresh_verify(rr, candidate, session_file_exists)
            check("verify rejects " + name, outcome == "unverified" and reason in (missing or "")
                  and ev["gate_cleared"] is gate_cleared)
        candidate = copy.deepcopy(rr)
        candidate["stages"]["g2_ack"]["done"] = True
        outcome, missing, ev = m.f1_fresh_verify(candidate, row, session_file_exists)
        check("verify reflects G2 ACK", outcome == "fresh" and missing is None and ev["g2_ack"] is True)

        for name, key, value, provenance in (
            ("inventory absent", "fresh_pre_sids", None, "no_inventory"),
            ("inventory not list", "fresh_pre_sids", ("old",), "no_inventory"),
            ("directory absent", "fresh_pre_dir", None, "no_inventory"),
            ("directory not string", "fresh_pre_dir", 1, "no_inventory"),
            ("sid unobserved", "observed_sid", None, "unobserved"),
            ("sid empty", "observed_sid", "", "unobserved"),
            ("source absent", "observed_sid_source", None, "unbound"),
            ("source heuristic", "observed_sid_source", "topology", "unbound"),
            ("sid separator", "observed_sid", "a/b", "invalid_id"),
            ("sid space", "observed_sid", "a b", "invalid_id"),
            ("sid too long", "observed_sid", "a" * 129, "invalid_id"),
            ("pre_existing beats awakening", "fresh_pre_sids", ["old", "new1"], "pre_existing"),
        ):
            candidate = copy.deepcopy(rr)
            candidate[key] = value
            outcome, missing, ev = m.f1_fresh_verify(candidate, row, session_file_exists)
            check("verify provenance " + name, outcome == "unverified" and bool(missing)
                  and ev["provenance"] == provenance and ev["gate_cleared"] is True
                  and ev["reinject_kind"] == "ack"
                  and ev["observed_sid"] == candidate["observed_sid"]
                  and ev["observed_sid_source"] == candidate["observed_sid_source"]
                  and (provenance != "pre_existing" or "fork 의심" in missing))
        for name, rr_changes, current_row, provenance in (
            ("registration key absent", {}, {k: v for k, v in row.items() if k != "registered_session_id"}, "unbound_now"),
            ("row not dict", {}, None, "unbound_now"),
            ("registration changed", {}, dict(row, registered_session_id="other"), "changed"),
            ("reinject None", {"reinject_sid": None}, row, "evidence_unbound"),
            ("reinject other", {"reinject_sid": "other"}, row, "evidence_unbound"),
            ("changed beats pre_existing", {"fresh_pre_sids": ["new1"]}, dict(row, registered_session_id="other"), "changed"),
            ("pre_existing beats evidence_unbound", {"fresh_pre_sids": ["new1"], "reinject_sid": None}, row, "pre_existing"),
            ("evidence_unbound beats file_missing", {"reinject_sid": None}, row, "evidence_unbound"),
            # ★리뷰 R3(codex major): 주입 **직전** 결속이 미관측/다름이면 그 ACK 는 어느 세션의 것도 아니다.
            #   "before absent" 가 정확히 44713ff 저널 형식이다(등록 전 주입 → 다른 세션 등록 경쟁의 산물).
            ("before None", {"reinject_sid_before": None}, row, "evidence_unbound"),
            ("before other", {"reinject_sid_before": "other"}, row, "evidence_unbound"),
        ):
            candidate = copy.deepcopy(rr)
            candidate.update(rr_changes)
            paths = []
            outcome, missing, ev = m.f1_fresh_verify(candidate, current_row, lambda p: paths.append(p) or False)
            check("verify R2 " + name, outcome == "unverified" and bool(missing)
                  and ev["provenance"] == provenance and paths == []
                  and ev["registered_now"] == (current_row.get("registered_session_id") if current_row else None)
                  and ev["reinject_sid"] == candidate.get("reinject_sid")
                  and (current_row is None or ev["gate_cleared"] is True)
                  and (provenance != "changed" or ("new1" in missing and "other" in missing)))
        # ★리뷰 R3: 키 자체가 없는 구(44713ff) 레코드도 귀속 미확정이다 — 검증기만 고치면 캐시가 살아남는다.
        legacy_bound = copy.deepcopy(rr)
        legacy_bound.pop("reinject_sid_before")
        outcome, missing, ev = m.f1_fresh_verify(legacy_bound, row, lambda p: False)
        check("verify R2 before absent", outcome == "unverified" and ev["provenance"] == "evidence_unbound"
              and ev["reinject_sid_before"] is None and "주입 직전 결속" in (missing or ""))
        outcome, missing, ev = m.f1_fresh_verify(rr, row, lambda p: False)
        check("verify provenance file missing", outcome == "unverified" and bool(missing)
              and ev["provenance"] == "file_missing")
        paths = []
        outcome, missing, ev = m.f1_fresh_verify(rr, row, lambda p: paths.append(p) or True)
        check("verify session file path", outcome == "fresh" and missing is None
              and paths == [os.path.join(td, "new1.jsonl")])
        with open(os.path.join(td, "new1.jsonl"), "w", encoding="utf-8") as f:
            f.write("{}\n")
        outcome, missing, ev = m.f1_fresh_verify(rr, row)
        check("verify default real session file", outcome == "fresh" and missing is None
              and ev["provenance"] == "new")

        # F: 예상 이관은 독약 기록과 증거 있는 verify를 훼손하지 않는다.
        j = {"roles": {
            "w": {"fresh_fallback": True, "fresh_reason": "no_session", "stages": {"verify": {"done": True}}},
            "p": {"fresh_fallback": True, "fresh_reason": "poison", "stages": {"verify": {"done": True}}},
            "e": {"fresh_fallback": True, "fresh_reason": "no_session_file", "fresh_evidence": {"x": 1},
                  "stages": {"verify": {"done": True}}},
            "n": {"stages": {}},
        }}
        before = copy.deepcopy(j)
        moved = m.migrate_f1_journal(j)
        check("migration moved roles", sorted(moved) == ["e", "w"])
        w, p, e, n = (j["roles"][role] for role in ("w", "p", "e", "n"))
        check("migration resets unproven verify", w.get("fresh_expected") is True
              and "fresh_fallback" not in w and w["stages"]["verify"]["done"] is False)
        check("migration leaves poison untouched", p == before["roles"]["p"])
        check("migration preserves proven verify", e.get("fresh_expected") is True
              and "fresh_fallback" not in e and e["stages"]["verify"]["done"] is True
              and e["fresh_evidence"] == {"x": 1})
        check("migration leaves normal role untouched", n == before["roles"]["n"])
        check("migration empty journal", m.migrate_f1_journal({}) == [])
        malformed = {"roles": {"bad": None}}
        try:
            moved = m.migrate_f1_journal(malformed)
        except Exception as exc:
            check("migration skips non-dict role (%r)" % exc, False)
        else:
            check("migration skips non-dict role", moved == [] and malformed == {"roles": {"bad": None}})

        j = {"roles": {
            "legacy": {"fresh_expected": True, "outcome": "fresh",
                       "fresh_evidence": {"gate_cleared": True}, "stages": {"verify": {"done": True}}},
            "proven": {"fresh_expected": True, "outcome": "fresh",
                       "observed_sid": "new1", "reinject_sid": "new1", "reinject_sid_before": "new1",
                       "fresh_evidence": {"provenance": "new"}, "stages": {"verify": {"done": True}}},
            "unverified": {"fresh_expected": True, "outcome": "unverified",
                           "stages": {"verify": {"done": True}}},
        }}
        before = copy.deepcopy(j)
        moved = m.migrate_f1_journal(j)
        check("migration dc3a158 resets awakening-only fresh", moved == ["legacy"]
              and j["roles"]["legacy"]["stages"]["verify"]["done"] is False)
        check("migration dc3a158 preserves new provenance", "proven" not in moved
              and j["roles"]["proven"] == before["roles"]["proven"])
        check("migration dc3a158 preserves unverified", "unverified" not in moved
              and j["roles"]["unverified"] == before["roles"]["unverified"])

        for name, changes in (("absent", {}), ("None", {"reinject_sid": None}),
                              ("different", {"reinject_sid": "other"})):
            cached = copy.deepcopy(before["roles"]["proven"])
            cached.pop("reinject_sid")
            cached.update(changes)
            journal = {"roles": {"cached": cached}}
            check("migration ef1d3e4 reinject " + name, m.migrate_f1_journal(journal) == ["cached"]
                  and cached["stages"]["verify"]["done"] is False)
        # ★리뷰 R3(codex major): 44713ff 형식(직전 결속 없음/다름)도 캐시 성공을 무효화한다 — 그 레코드는
        #   지금 고치는 경쟁(등록 전 주입 → 다른 세션 등록)의 산물일 수 있다. 귀결은 재검증(스폰·파괴 0).
        for name, changes in (("absent", {}), ("None", {"reinject_sid_before": None}),
                              ("different", {"reinject_sid_before": "other"})):
            cached = copy.deepcopy(before["roles"]["proven"])
            cached.pop("reinject_sid_before")
            cached.update(changes)
            journal = {"roles": {"cached": cached}}
            check("migration 44713ff before " + name, m.migrate_f1_journal(journal) == ["cached"]
                  and cached["stages"]["verify"]["done"] is False)

        # G: 명시적 계약 상수.
        check("fresh reasons", m.F1_FRESH_REASONS == ("no_session", "no_session_file", "no_resume_arg", "cli_fresh"))
        check("accepted reinject kinds", set(m.F1_ACCEPTED_REINJECT_KINDS) == {"ack", "injected"})
        check("registered grace tries", m.F1_REGISTERED_GRACE_TRIES >= 3)
        check("reverify passes", m.F1_REVERIFY_PASSES == 2)
        check("reobservable provenance", m.F1_REOBSERVABLE ==
              ("unobserved", "file_missing", "unbound_now", "changed", "evidence_unbound", "pre_existing"))
        uuid = "11111111-2222-3333-4444-555555555555"
        for name, sid, accepted in (
            ("uuid", uuid, True), ("empty", "", False), ("separator", "a/b", False),
            ("space", "a b", False), ("129 chars", "a" * 129, False),
            ("128 chars", "a" * 128, True), ("allowed punctuation", "a._-9", True),
        ):
            check("SID regex " + name, bool(m.F1_SID_RE.match(sid)) == accepted)

        # H: 공유 프로젝트 경로와 스폰 전 재고(오류는 빈 재고와 구별).
        check("session project directory", m._session_project_dir(entry) == "%s/projects/-tmp-a-b-c" % td)
        original_account_dir = os.environ.get("CYS_ACCOUNT_DIR")
        try:
            os.environ["CYS_ACCOUNT_DIR"] = td
            for name, candidate in (
                ("missing cfg", {"cwd": "/tmp/a.b_c"}),
                ("None cfg", {"claude_config_dir": None, "cwd": "/tmp/a.b_c"}),
                ("non-string cfg", {"claude_config_dir": 1, "cwd": "/tmp/a.b_c"}),
            ):
                check("session project " + name, m._session_project_dir(candidate) == "%s/projects/-tmp-a-b-c" % td)
        finally:
            if original_account_dir is None:
                os.environ.pop("CYS_ACCOUNT_DIR", None)
            else:
                os.environ["CYS_ACCOUNT_DIR"] = original_account_dir
        # 순수 문자열(파일시스템 미접촉) — 플랫폼 무관.
        check("session project empty cfg", m._session_project_dir(dict(entry, claude_config_dir=""))
              == "/projects/-tmp-a-b-c")
        for name, cwd in (("empty", ""), ("None", None)):
            check("session project " + name + " cwd", m._session_project_dir(dict(entry, cwd=cwd))
                  == "%s/projects/" % td)

        inventory_entry = dict(entry, cwd="/inventory")
        project_dir = "%s/projects/-inventory" % td
        check("inventory absent directory", m.session_inventory(inventory_entry) == (project_dir, []))
        os.makedirs(project_dir)
        for name in ("old.jsonl", "notes.txt", "b.jsonl"):
            with open(os.path.join(project_dir, name), "w", encoding="utf-8") as f:
                f.write("{}\n")
        check("inventory sorted jsonl stems", m.session_inventory(inventory_entry) == (project_dir, ["b", "old"]))
        original_listdir = m.os.listdir
        def denied_listdir(path):
            raise PermissionError("inventory unavailable")
        try:
            m.os.listdir = denied_listdir
            check("inventory OSError unknown", m.session_inventory(inventory_entry) == (project_dir, None))
        finally:
            m.os.listdir = original_listdir

        # I: 등록 신원 관측과 grace(데몬 호출·실제 sleep 없음).
        original_status_row, original_sleep = m._surface_status_row, m.time.sleep
        try:
            for name, rows, tries, expected_sid, fragment, expected_calls, expected_sleeps, pre_sids in (
                ("row absent", [None], 2, None, "미관측", 2, 1, None),
                ("old daemon", [{}], 2, None, "구 데몬", 2, 1, None),
                ("registration pending", [{"registered_session_id": None}], 2, None, "미관측", 2, 1, None),
                ("valid uuid", [{"registered_session_id": uuid}], 2, uuid, "registered(", 1, 0, None),
                ("invalid separator", [{"registered_session_id": "a/b"}], 2, None, "미관측", 2, 1, None),
                ("grace second attempt", [None, {"registered_session_id": uuid}], 3, uuid, "registered(", 2, 1, None),
                ("stale then new", [{"registered_session_id": "A"}, {"registered_session_id": "A"},
                                    {"registered_session_id": "B"}], 3, "B", "registered(", 3, 2, ["A"]),
                ("stale through grace", [{"registered_session_id": "A"}], 2, "A", "스폰 전 재고", 2, 1, ["A"]),
                ("pre_sids None", [{"registered_session_id": "A"}], 3, "A", "attempt 1", 1, 0, None),
            ):
                calls, sleeps = [], []
                def status_row(socket, surface):
                    calls.append((socket, surface))
                    return rows[min(len(calls) - 1, len(rows) - 1)]
                m._surface_status_row = status_row
                m.time.sleep = lambda seconds: sleeps.append(seconds)
                sid, evidence = m.stage_observe_registered_session("test-socket", "surface:3", tries=tries, pre_sids=pre_sids)
                check("observe registered " + name, sid == expected_sid and fragment in evidence
                      and (expected_sid is None or evidence.startswith("registered("))
                      and calls == [("test-socket", "surface:3")] * expected_calls
                      and len(sleeps) == expected_sleeps)
        finally:
            m._surface_status_row, m.time.sleep = original_status_row, original_sleep

        # J: 설치 pack에는 Rust 소스가 없을 수 있다.
        repo = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
        for relative, literals in (
            (os.path.join("src", "bin", "cys.rs"), (
                "디렉티브 생존 확인 (ACK 수신)", "reinjected {} bytes → surface:{sid}",
                "check reinject skip", "[reinject] ACK 없음",
                # ★리뷰 R3: 실제 기동 모드가 phoenix 에 닿는 유일한 문면(`cli_fresh_roles` 가 파싱한다).
                "[launch-agent] fresh 각성(role={role} · agent={agent})")),
            (os.path.join("src", "bin", "cysd", "handlers.rs"), ('"registered_session_id"',)),
        ):
            try:
                with open(os.path.join(repo, relative), encoding="utf-8") as f:
                    source = f.read()
            except FileNotFoundError:
                print("SKIP source pin %s (repo source absent)" % relative)
            else:
                for literal in literals:
                    check("source pin " + literal, literal in source)

        # L: ★리뷰 R3(codex major) — 어댑터의 resume_arg 효력(예상)과 CLI 관측 줄(실제 기동 모드).
        #    Rust `fill_missing_fields` 가 계층으로 채우는 키는 ready_marker·approval_patterns·first_run_gates
        #    셋뿐이라 `resume_arg` 는 디스크 선언이 전부다 — 부재/공백/비문자열은 전부 fresh 기동이다.
        pack = os.path.join(td, "packL")
        os.makedirs(pack)
        original_pack_env = {k: os.environ.get(k) for k in m.PACK_DIR_ENV_KEYS}
        try:
            for k in m.PACK_DIR_ENV_KEYS:
                os.environ.pop(k, None)
            os.environ["CYS_PACK_DIR"] = pack
            agents_path = os.path.join(pack, "agents.json")
            def write_agents(obj, stamp):
                with open(agents_path, "w", encoding="utf-8") as f:
                    json.dump(obj, f)
                # 캐시 키는 (경로, mtime_ns, 크기) — 같은 크기로 덮어써도 갈리도록 mtime 을 벌린다.
                os.utime(agents_path, (stamp, stamp))
            check("pack dir env precedence", m._pack_dir() == pack)
            check("resume effect agents absent", m.resume_arg_effect("claude") == ("unknown", "agents_unreadable"))
            for i, (name, spec, expected) in enumerate((
                ("real arg", {"cmd": "claude", "resume_arg": "--resume {session_id}"}, ("effective", "")),
                ("absent", {"cmd": "claude"}, ("no_effect", "resume_arg_absent")),
                ("empty", {"cmd": "claude", "resume_arg": ""}, ("no_effect", "resume_arg_blank")),
                ("whitespace", {"cmd": "claude", "resume_arg": " \t "}, ("no_effect", "resume_arg_blank")),
                ("null", {"cmd": "claude", "resume_arg": None}, ("no_effect", "resume_arg_blank")),
                ("non-string", {"cmd": "claude", "resume_arg": ["--resume"]}, ("no_effect", "resume_arg_blank")),
                ("adapter absent", {}, ("unknown", "adapter_absent")),
            )):
                payload = {"claude": spec} if spec else {"gemini": {"cmd": "gemini"}}
                write_agents(payload, 1_600_000_000 + i * 60)
                check("resume effect " + name, m.resume_arg_effect("claude") == expected)
            # fresh_expected 는 세션 파일이 **있어도** 효력 없는 접미면 fresh 를 예상한다(거짓 verified 차단).
            cfg = os.path.join(td, "acctL")
            proj = os.path.join(cfg, "projects", m._claude_project_component("/tmp/L"))
            os.makedirs(proj)
            with open(os.path.join(proj, "s1.jsonl"), "w", encoding="utf-8") as f:
                f.write("{}\n")
            e = {"agent": "claude", "session_id": "s1", "claude_config_dir": cfg, "cwd": "/tmp/L"}
            write_agents({"claude": {"cmd": "claude", "resume_arg": ""}}, 1_700_000_000)
            check("fresh_expected empty resume arg", m.fresh_expected(e) == (True, "no_resume_arg"))
            write_agents({"claude": {"cmd": "claude"}}, 1_700_000_060)
            check("fresh_expected absent resume arg", m.fresh_expected(e) == (True, "no_resume_arg"))
            write_agents({"claude": {"cmd": "claude", "resume_arg": "--resume {session_id}"}}, 1_700_000_120)
            check("fresh_expected real resume arg", m.fresh_expected(e) == (False, ""))
            # 다른 어댑터는 F-1 범위 밖 — 어떤 선언이어도 예상은 False.
            check("fresh_expected other adapter", m.fresh_expected(dict(e, agent="codex")) == (False, ""))
            os.remove(agents_path)
            check("fresh_expected unknown keeps prediction", m.fresh_expected(e) == (False, ""))
        finally:
            for k, v in original_pack_env.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
        # 출하 팩의 claude 어댑터가 실 `resume_arg` 를 선언한다 — "어댑터 부재 → 임베드 통본" 접기의 전제.
        shipped = os.path.join(os.path.abspath(os.path.join(HERE, "..", "..")), "agents.json")
        try:
            with open(shipped, encoding="utf-8") as f:
                declared = (json.load(f).get("claude") or {}).get("resume_arg")
        except (OSError, ValueError):
            print("SKIP shipped adapter pin (pack agents.json absent)")
        else:
            check("shipped claude adapter declares a non-empty resume_arg",
                  isinstance(declared, str) and bool(declared.strip()))
        # CLI 관측 줄 — 역할만 뽑고, 다른 줄·부분 문자열에 위조되지 않는다.
        observed = m.cli_fresh_roles(
            "[launch-agent] fresh 각성(role=worker-1 · agent=claude): resume 이 요청됐으나 …\n"
            "  [launch-agent] fresh 각성(role=indented · agent=claude)\n"        # 줄머리 아님 → 무시
            "[launch-agent] resume 생략: session_id 없음 — fresh 로 기동한다\n"  # 다른 줄 → 무시
            "[launch-agent] fresh 각성(role=cso · agent=claude): …\n")
        check("cli fresh roles line anchored", observed == {"worker-1", "cso"})
        check("cli fresh roles empty", m.cli_fresh_roles("") == set() and m.cli_fresh_roles(None) == set())

        # M: ★리뷰 R3b — resume 미상은 핀 일치로 자기채점하지 않고, 타임아웃 원문은 관측만 살린다.
        # M1: 비-F1 topology 진리표 — outcome과 사유를 함께 고정한다(미상은 일치만 강등).
        for name, exp, obs, mode, expected, fragment in (
            ("match default", "s1", "s1", None, "verified", "세션 일치"),
            ("match effective", "s1", "s1", "effective", "verified", "세션 일치"),
            ("match no_effect", "s1", "s1", "no_effect", "verified", "세션 일치"),
            ("match empty mode", "s1", "s1", "", "verified", "세션 일치"),
            ("match unknown", "s1", "s1", "unknown", "unverified", "resume 모드 미상"),
            ("unobserved unknown", "s1", None, "unknown", "unverified", "transient"),
            ("empty observed unknown", "s1", "", "unknown", "unverified", "transient"),
            ("fork", "s1", "s2", None, "unverified", "fork"),
            ("fork unknown", "s1", "s2", "unknown", "unverified", "fork"),
            ("pin absent", "", "s1", None, "unverified", "핀 부재"),
            ("both absent", None, None, None, "unverified", "transient"),
        ):
            outcome, reason = m.legacy_verify_outcome(exp, obs, mode)
            check("legacy verify " + name, outcome == expected and fragment in reason
                  and (name != "match unknown" or obs in reason))
        check("legacy verify unknown preserves fork",
              m.legacy_verify_outcome("s1", "s2", "unknown") == m.legacy_verify_outcome("s1", "s2"))

        # M2: 실제 cys 호출 없이 세 스트림을 각각 관측한다 — 분류용 TIMEOUT 문안은 그대로다.
        fresh_line = "[launch-agent] fresh 각성(role=worker-1 · agent=claude): resume 이 요청됐으나 …"
        class CaptureStub:
            returncode = 124
            stdout = ""
            stderr = "TIMEOUT 90s"
        original_cys = m.cys
        try:
            result = CaptureStub()
            result.stderr_raw = fresh_line + "\n관측과 무관한 진단 줄\n"
            m.cys = lambda *args, **kwargs: result
            spawned = m.spawn_production("test-socket", ["worker-1"])
            check("spawn timeout rc", spawned["rc"] == 124)
            check("spawn timeout classifier out unchanged", spawned["out"] == "TIMEOUT 90s")
            check("spawn timeout raw fresh observed", spawned["fresh_observed"] == ["worker-1"])
            # 구 반환형은 속성 자체가 없다 — getattr 기본값으로 관측 없음, 예외 없음.
            result = CaptureStub()
            spawned = m.spawn_production("test-socket", ["worker-1"])
            check("spawn old capture shape", not hasattr(result, "stderr_raw")
                  and spawned["fresh_observed"] == [])
            for stream in ("stdout", "stderr"):
                result = CaptureStub()
                setattr(result, stream, fresh_line)
                spawned = m.spawn_production("test-socket", ["worker-1"])
                check("spawn fresh only in " + stream, spawned["fresh_observed"] == ["worker-1"])
        finally:
            m.cys = original_cys

        # M3: Windows 캡처 대역도 같은 원문 필드를 가진다 — 플랫폼 분기·파일 쓰기 불요.
        check("capture raw stderr default", m._CapR().stderr_raw == "")
        check("capture raw stderr explicit", m._CapR(stderr_raw="x").stderr_raw == "x")

        # N: ★리뷰 R4(codex major) — **진짜 cys() 예외 경로**로 잘린 멀티바이트를 흘린다.
        #    종전 검체는 이미 디코드된 스텁을 m.cys 에 꽂아 프로덕션의 strict decode 를 한 번도 밟지 않았다.
        #    여기서는 subprocess.run 을 몽키패치해 TimeoutExpired(바이트 캡처)를 던지게 하고, cys() 가
        #    예외 없이 구조화 결과를 내는지 · 관측(fresh 각성 줄)이 보존되는지를 잰다.
        fresh_bytes = fresh_line.encode("utf-8")
        original_run = m.subprocess.run
        original_win = m.IS_WINDOWS
        try:
            m.IS_WINDOWS = False  # 이 검체는 POSIX 분기(진짜 예외 경로)를 잰다 — Windows CI 에서도 같은 것을 잰다.
            for name, out_b, err_b, want_fresh in (
                ("cut stderr", b"", fresh_bytes + b"\n\xe2", True),          # 완전한 줄 + 잘린 멀티바이트
                ("cut stdout", fresh_bytes + b"\n\xed\x95", b"", True),      # 양쪽 다 잘림 가능
                ("both cut", fresh_bytes + b"\xe2", fresh_bytes + b"\n\xf0\x9f", True),
                ("clean utf8", b"", fresh_bytes.decode().encode("utf-8") + b"\n", True),  # 대조군(정상)
                ("str capture", "", fresh_line + "\n", True),                # text=True 경로(bytes 아님)
                ("none capture", None, None, False),                          # 캡처 자체가 없음
            ):
                def _raise(*a, **kw):
                    raise m.subprocess.TimeoutExpired(cmd=["cys"], timeout=90, output=out_b, stderr=err_b)
                m.subprocess.run = _raise
                try:
                    r = m.cys("restore", "--role", "worker-1", timeout=90)
                    ok = True
                except Exception as exc:                                       # noqa: BLE001 — 이 검체의 대상
                    r, ok = None, "raised %s" % type(exc).__name__
                check("cys timeout no raise " + name, ok is True)
                if ok is not True:
                    continue
                check("cys timeout rc " + name, r.returncode == 124)
                check("cys timeout classifier " + name, r.stderr == "TIMEOUT 90s")
                observed = m.cli_fresh_roles(getattr(r, "stderr_raw", "")) | m.cli_fresh_roles(r.stdout)
                check("cys timeout keeps observation " + name,
                      ("worker-1" in observed) == want_fresh)
                # 손실은 대체문자로 흡수될 뿐, 예외도 잘림도 관측을 지우지 않는다.
                check("cys timeout decoded str " + name,
                      isinstance(r.stdout, str) and isinstance(getattr(r, "stderr_raw", ""), str))
            # 순수부 직접 대조 — 바이트·문자열·None 세 형태.
            check("decode captured replaces", m._decode_captured(b"ok\xe2").endswith("\ufffd"))
            check("decode captured str passthrough", m._decode_captured("ok") == "ok")
            check("decode captured none", m._decode_captured(None) == "" and m._decode_captured(b"") == "")
        finally:
            m.subprocess.run = original_run
            m.IS_WINDOWS = original_win

        # K: 성공 fresh만 target 순서로 분할; 기본 사유는 구 저널의 poison.
        targets = ["f1", "pending", "poison", "verified"]
        outcomes = {"poison": "fresh", "verified": "verified", "pending": "unverified", "f1": "fresh"}
        roles = {"poison": {}, "f1": {"fresh_reason": "no_session_file"},
                 "pending": {"fresh_reason": "no_session"}, "verified": {}}
        check("result partition exact target order", m.f1_result_partition(targets, outcomes, roles) ==
              (["f1", "poison"], {"f1": "no_session_file", "poison": "poison"}, ["poison"], ["f1"]))
    finally:
        shutil.rmtree(td)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
