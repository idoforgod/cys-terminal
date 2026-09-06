#!/usr/bin/env python3
"""★F-1(0.14.31): fresh 예상·주입 증거·검증·저널 이관 계약(데몬·네트워크 불요).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_fresh_honest.py (0=전건 PASS)
"""
import copy
import importlib.util, os, shutil, sys, tempfile

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
            ("Users example", "/Users/user/Desktop/ProjX", "-Users-user-Desktop-ProjX"),
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
              "fresh_pre_sids": ["old"], "fresh_pre_dir": td}
        session_file_exists = lambda p: p.endswith("new1.jsonl")
        row = {"exited": False, "agent_alive": True, "gate_pending": None}
        outcome, missing, ev = m.f1_fresh_verify(rr, row, session_file_exists)
        check("verify full ACK evidence", outcome == "fresh" and missing is None
              and ev["gate_cleared"] is True and ev["reinject_kind"] == "ack" and ev["g2_ack"] is False
              and ev["provenance"] == "new" and ev["observed_sid"] == "new1"
              and ev["observed_sid_source"] == "registered")
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

        # G: 명시적 계약 상수.
        check("fresh reasons", m.F1_FRESH_REASONS == ("no_session", "no_session_file"))
        check("accepted reinject kinds", set(m.F1_ACCEPTED_REINJECT_KINDS) == {"ack", "injected"})
        check("registered grace tries", m.F1_REGISTERED_GRACE_TRIES >= 3)
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
            for name, rows, tries, expected_sid, fragment, expected_calls, expected_sleeps in (
                ("row absent", [None], 2, None, "미관측", 2, 1),
                ("old daemon", [{}], 2, None, "구 데몬", 2, 1),
                ("registration pending", [{"registered_session_id": None}], 2, None, "미관측", 2, 1),
                ("valid uuid", [{"registered_session_id": uuid}], 2, uuid, "registered(", 1, 0),
                ("invalid separator", [{"registered_session_id": "a/b"}], 2, None, "미관측", 2, 1),
                ("grace second attempt", [None, {"registered_session_id": uuid}], 3, uuid, "registered(", 2, 1),
            ):
                calls, sleeps = [], []
                def status_row(socket, surface):
                    calls.append((socket, surface))
                    return rows[min(len(calls) - 1, len(rows) - 1)]
                m._surface_status_row = status_row
                m.time.sleep = lambda seconds: sleeps.append(seconds)
                sid, evidence = m.stage_observe_registered_session("test-socket", "surface:3", tries=tries)
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
                "check reinject skip", "[reinject] ACK 없음")),
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
    finally:
        shutil.rmtree(td)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
