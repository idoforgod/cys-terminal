#!/usr/bin/env python3
"""★F-1(0.14.31): fresh 예상·주입 증거·검증·저널 이관 계약(데몬·네트워크 불요).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_fresh_honest.py (0=전건 PASS)
"""
import copy
import importlib.util, json, os, shutil, sys, tempfile
from types import SimpleNamespace          # d1: scripted cys replies for the snapshot shapes

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


def bind_rig_state_root(rig_dir, sock):
    """픽스처가 **제품이 읽는 바로 그 자리**에 쓰도록 데몬 상태 루트를 리그 안에 묶는다(codex W1).

    제품 규약은 `javis_phoenix.py:1113-1119` 다 — unix 는 소켓의 부모 디렉터리가 곧 상태 디렉터리이고,
    Windows 는 named pipe 에 파일시스템 부모가 없어 슬러그 매핑
    (`javis_state_snapshot._win_state_dir_for_socket` · `%LOCALAPPDATA%\\cys\\<슬러그>`)으로 파생한다.
    그래서 소켓 부모에 topology 를 쓰면 Windows 에서는 제품이 **다른 자리**를 읽어 부활 대상이 0 이 된다
    (실측 · IS_WINDOWS 대역: 「부활 대상 죽은 역할 0」 → D3-B1 FAIL → `:879` KeyError('evidence')).
    여기서는 `LOCALAPPDATA` 를 리그 안 전용 디렉터리로 돌려 그 매핑이 리그 안에 떨어지게 하고,
    ★**어떤 쓰기보다 먼저** 해소된 디렉터리가 리그 안인지 단언한다(디렉터리는 제품의 해소기로만 구한다 —
    손으로 조립하지 않는다). 호출측이 `LOCALAPPDATA` 사본을 try 앞에서 떠 두고 finally 에서 되돌린다.
    """
    os.environ["LOCALAPPDATA"] = os.path.join(rig_dir, "winroot")
    sd = m.state_dir_for(sock)                       # ★제품의 해소기 그대로
    root, real = os.path.realpath(rig_dir), os.path.realpath(sd)
    assert real == root or real.startswith(root + os.sep), (
        "상태 디렉터리가 리그를 벗어났다: %r (리그 %r · IS_WINDOWS=%r)" % (sd, root, m.IS_WINDOWS))
    os.makedirs(sd, exist_ok=True)
    return sd


def restore_env(saved):
    """bind_rig_state_root 가 건드린 env 를 원래대로 — 없던 키는 지우고 있던 키는 값째 되돌린다."""
    for k, v in saved.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v


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
                "디렉티브 생존 확인 (ACK 수신)", "reinjected {bytes} bytes → surface:{sid}",
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

        # ──────────────────────────────────────────────────────────────────────
        # L: ★D3(0.14.55) fresh 표지의 부팅 범위 한정 — D5H-20261028-PHOENIX-STALE-FRESH-MARKER.
        #    설계 DESIGN.md(d1a4037e)+R2(7a90a1b5)+R3(14f45914) · 판정문 HEAD-RULING-R3-FINAL(14e0cdd2).
        #    낡은 표지가 증거 0으로 'fresh' 를 만들던 자리와, 걷어낸 역할이 핀 동어반복으로 내려가
        #    거짓 VERIFIED 가 되던 자리를 함께 묶는다.
        # ★이 절(L)은 **순수 함수** 계약을 묶는다 — `fresh_epoch`·`fresh_scope_epoch` 를 픽스처에 직접 넣는
        #   줄들은 그 함수의 진리표를 재는 것이고, **제품이 그 값을 쓰는지**는 절 N(실 작성자·실 run_restore)이
        #   따로 묶는다. 두 절을 섞지 마라: 손으로 넣은 값으로만 통과하는 증명은 제품을 재지 않는다.
        E_OLD, E_NOW, E_NEXT = "sa:1793100000.1", "sa:1793200000.9", "sa:1793300000.5"
        SRC = open(m.__file__ if hasattr(m, "__file__") and m.__file__ else PH, encoding="utf-8").read().splitlines()

        def rec(**kw):
            base = {"surface": "surface:7", "expected_sid": "sid-A", "observed_sid": "sid-A",
                    "stages": {s2: {"done": True, "ts": 1, "epoch": E_OLD} for s2 in
                               ("spawn", "ready", "resume", "reinject", "g2_ack", "verify")}}
            base.update(kw)
            return {"ticket_id": "default", "events": [], "created": 1, "roles": {"w": base}}

        def find(pred, lo=0, hi=None):
            hi = len(SRC) if hi is None else hi
            return [i + 1 for i in range(lo, hi) if pred(SRC[i])]

        # D3-1 낡은 표지(세대 스탬프 없음) → 걷어내고 표식을 이번 부팅 이름으로 남긴다 → 가드가 발화한다.
        j = rec(fresh_fallback=True, fresh_reason="poison")
        got = m.scope_fresh_markers(j, E_NOW)
        rr = j["roles"]["w"]
        check("D3-1 stale marker is stripped and flagged for this boot",
              got == [("w", "표지에 세대 없음(이 수리 이전 저널)")]
              and "fresh_fallback" not in rr and "fresh_reason" not in rr
              and rr["fresh_scope_epoch"] == E_NOW
              and m.fresh_scope_guard_fires(rr, E_NOW) is True)

        # D3-2 ★위반 대조: 가드가 없으면 같은 역할이 핀 비교로 내려가 'verified' 가 된다(결함을 계속 보이게 둔다).
        check("D3-2 without the guard the same pair is verified (the defect, kept visible)",
              m.legacy_verify_outcome(rr.get("expected_sid", ""), rr.get("observed_sid"), "effective")
              == ("verified", "세션 일치")
              and m.restore_exit_code({"phoenix_restore": "VERIFIED", "completeness": "COMPLETE"}) == 0)

        # D3-3 / D3-4 좌석 축: 같은 부팅의 표지라도 관문에 갇힌 좌석은 'fresh' 가 아니다.
        trapped = {"exited": False, "agent_alive": True, "gate_pending": {"why": "folder-trust"}}
        healthy = {"exited": False, "agent_alive": True, "gate_pending": None}
        check("D3-3 same-boot marker + gate-trapped seat has no seat evidence",
              m.p11_seat_evidence(trapped) == (False, "gate_pending 보류(관문 보류 · 주입 0)"))
        check("D3-4 same-boot marker + healthy seat has seat evidence",
              m.p11_seat_evidence(healthy) == (True, None))
        check("D3-4b every missing axis is named, not collapsed",
              m.p11_seat_evidence(None)[0] is False
              and m.p11_seat_evidence({"exited": True, "agent_alive": True, "gate_pending": None})[1] == "exited≠false"
              and m.p11_seat_evidence({"exited": False, "agent_alive": None, "gate_pending": None})[1] == "agent_alive≠true"
              and m.p11_seat_evidence({"exited": False, "agent_alive": True})[1].startswith("gate_pending 키 부재"))

        # D3-5 stub 팔: 분지가 stub 일 때 좌석 축을 요구하지 않는다(출하 P11 드릴이 --stub 으로 돈다).
        p11_if = find(lambda l: l.strip() == "if fresh_fb:")
        stub_arm = find(lambda l: "_p11_ok, _p11_why = (True, None) if stub else p11_seat_evidence(_p11_row)" in l)
        check("D3-5 the stub arm exempts the seat axis exactly once",
              len(p11_if) == 1 and len(stub_arm) == 1 and p11_if[0] < stub_arm[0] < p11_if[0] + 6)

        # D3-6 EPOCH_GATE=0 은 stage_done 과 같은 방향(세대 무시) — 표지를 보존한다.
        _gate = m.EPOCH_GATE
        try:
            m.EPOCH_GATE = False
            jg = rec(fresh_fallback=True, fresh_reason="poison")
            check("D3-6 EPOCH_GATE=0 keeps the marker, mirroring stage_done",
                  m.marker_is_current({}, E_NOW) is True
                  and m.scope_fresh_markers(jg, E_NOW) == []
                  and jg["roles"]["w"]["fresh_fallback"] is True)
        finally:
            m.EPOCH_GATE = _gate
        check("D3-6b the mirror restores", m.EPOCH_GATE is _gate and m.marker_is_current({}, E_NOW) is False)

        # D3-7 두 순서: 범위 한정은 migrate **뒤**, dedup(pending)·spawn 재기록 **앞**.
        mig = find(lambda l: "for _r in migrate_f1_journal(j):" in l)
        scope_call = find(lambda l: "_fresh_scope_stripped = dict(scope_fresh_markers(j, _ACTIVE_EPOCH))" in l)
        pend = find(lambda l: 'pending = [r for r in target_roles if not stage_done(j, r, "verify")]' in l)
        remark = find(lambda l: 'j["roles"][role]["fresh_expected"] = True' in l)
        check("D3-7 scope runs after migrate and before pending and the spawn re-mark",
              len(mig) == 1 and len(scope_call) == 1 and len(pend) == 1 and len(remark) == 1
              and mig[0] < scope_call[0] < pend[0] < remark[0])

        # D3-8 가드는 핀 판정 함수보다 **먼저** — 검색 범위를 verify 블록으로 한정한다.
        # ★통합(worker-18 · 결함1 스택 · 헤드 확인됨 2026-10-28): else 팔의 판정 호출이
        #   `legacy_verify_outcome` 에서 결함1의 `_d1_resume_verdict` 로 바뀐다(stub 은 그 함수 **안에서**
        #   종전 진리표를 그대로 쓰므로 `return legacy_verify_outcome(` 는 이 패턴에 걸리지 않는다 ⇒ len==1
        #   유지). 이 행이 지키는 **성질**(가드가 판정보다 먼저)과 세 개수 단언은 그대로다.
        fb_read = find(lambda l: 'fresh_fb = j["roles"][role].get("fresh_fallback", False)' in l)
        verdict_call = find(lambda l: ("outcome, reason = legacy_verify_outcome(exp, obs," in l
                                       or "outcome, reason = _d1_resume_verdict(" in l))
        guard_line = find(lambda l: "elif fresh_scope_guard_fires(" in l, fb_read[0] - 1, verdict_call[0])
        check("D3-8 the guard precedes the verdict call, inside the verify block",
              len(fb_read) == 1 and len(verdict_call) == 1 and len(guard_line) == 1
              and fb_read[0] < guard_line[0] < verdict_call[0])

        # D3-9 F-1 잔여 키도 표지와 함께 걷어낸다.
        j9 = rec(fresh_expected=True, fresh_reason="no_session", fresh_pre_sids=["old"], fresh_pre_dir=td,
                 fresh_evidence={"provenance": "new"}, observed_sid_source="registered",
                 reinject_sid="sid-A", reinject_sid_before="sid-A")
        m.scope_fresh_markers(j9, E_NOW)
        check("D3-9 the F-1 residue keys are stripped with the group",
              not any(k in j9["roles"]["w"] for k in m.FRESH_MARKER_KEYS)
              and set(m.FRESH_MARKER_KEYS) >= {"observed_sid_source", "reinject_sid", "reinject_sid_before"}
              and "expected_sid" in j9["roles"]["w"] and "observed_sid" in j9["roles"]["w"])

        # D3-10 세대 미상·판정 실패는 **걷어내는 쪽**으로 접는다(예외 경로이지 건너뛰기가 아니다).
        j10 = rec(fresh_fallback=True, fresh_reason="poison", fresh_epoch=E_NOW)
        check("D3-10 epoch unknown strips even a stamped marker",
              m.scope_fresh_markers(j10, None) == [("w", "세대 미상(status --json 판독 실패)")]
              and "fresh_fallback" not in j10["roles"]["w"])
        class _Boom(dict):
            def get(self, k, d=None):
                if k == "fresh_epoch":
                    raise RuntimeError("boom")
                return dict.get(self, k, d)
        j11 = {"ticket_id": "default", "events": [], "roles": {"w": _Boom(fresh_fallback=True)}}
        out11 = m.scope_fresh_markers(j11, E_NOW)
        check("D3-10b a raising record is stripped, never kept",
              len(out11) == 1 and "범위 판정 실패" in out11[0][1] and "fresh_fallback" not in j11["roles"]["w"])

        # D3-11 추가 결과 키(관측성) — 결과 dict 에 실린다.
        rkey = find(lambda l: '"fresh_scope_stripped": _fresh_scope_stripped,' in l)
        check("D3-11 the additive result key is in the result dict", len(rkey) == 1)

        # D3-12 migrate_f1_journal 은 손대지 않았다 — poison 레코드 무접촉 계약 유지(출하 계약).
        j12 = {"roles": {"p": {"fresh_fallback": True, "fresh_reason": "poison",
                               "stages": {"verify": {"done": True}}}}}
        before12 = copy.deepcopy(j12["roles"]["p"])
        check("D3-12 migrate_f1_journal still leaves poison untouched",
              m.migrate_f1_journal(j12) == [] and j12["roles"]["p"] == before12)

        # D3-13 / D3-14 ★범위 한정 패스는 적재하중이다 — 조용히 빠질 수 없다.
        check("D3-13 the scope pass has exactly one call site in the restore flow", len(scope_call) == 1)
        j14 = rec(fresh_fallback=True, fresh_reason="poison")
        # 패스를 돌리지 **않으면** 같은 세계에서 좌석이 건강하므로 분지가 'fresh' 를 낸다(fail-open).
        check("D3-14 without the pass a healthy resumed seat still yields fresh (fail-open)",
              j14["roles"]["w"].get("fresh_fallback") is True
              and m.p11_seat_evidence(healthy) == (True, None)
              and m.fresh_scope_guard_fires(j14["roles"]["w"], E_NOW) is False)
        m.scope_fresh_markers(j14, E_NOW)
        check("D3-14b with the pass the same world is unverified through the guard",
              m.fresh_scope_guard_fires(j14["roles"]["w"], E_NOW) is True
              and "fresh_fallback" not in j14["roles"]["w"])

        # D3-15 ★소스 핀: Phase-11 의 좌석 조건 목록과 f1_fresh_verify 의 좌석 조건 목록이 내용·순서로 같다.
        import re as _re
        def conds(fn_name, stop):
            lo = find(lambda l: l.startswith("def %s(" % fn_name))
            assert len(lo) == 1, fn_name
            hi = find(lambda l: stop in l, lo[0], len(SRC))
            out = []
            for l in SRC[lo[0]:hi[0]]:
                mm = _re.match(r'^\s*(?:el)?if (.+?):\s*$', l)
                if mm and ("row" in mm.group(1) or "isinstance" in mm.group(1)):
                    out.append(mm.group(1).strip())
            return out
        ship = conds("f1_fresh_verify", "if kind not in F1_ACCEPTED_REINJECT_KINDS")
        mine = conds("p11_seat_evidence", "return True, None")
        check("D3-15 the two seat condition lists are identical in content and order",
              ship == mine and ship == ['not isinstance(row, dict)', 'row.get("exited") is not False',
                                        'row.get("agent_alive") is not True', '"gate_pending" not in row',
                                        'row.get("gate_pending") is not None'])

        # 판정문 항목 1 — 같은 부팅의 재시도는 표식을 **보존**하고, 새 세대는 지운다(영구 적색 0).
        j16 = rec(fresh_fallback=True, fresh_reason="poison")
        m.scope_fresh_markers(j16, E_NOW)                       # run 1 (this boot)
        again = m.scope_fresh_markers(j16, E_NOW)               # run 2, SAME epoch (daemon's one retry)
        check("ruling-1 the same-boot retry keeps the flag and the guard still fires",
              again == [] and m.fresh_scope_guard_fires(j16["roles"]["w"], E_NOW) is True)
        nxt = m.scope_fresh_markers(j16, E_NEXT)                # a later boot
        check("ruling-1b a new epoch clears the flag — no permanent red",
              nxt == [] and "fresh_scope_stripped" not in j16["roles"]["w"]
              and m.fresh_scope_guard_fires(j16["roles"]["w"], E_NEXT) is False)

        # 판정문 항목 2 — Phase-11 분지가 **자기** 행을 읽는다(elif 안의 row 를 빌리지 않는다).
        own_row = find(lambda l: "_p11_row = None if stub else _surface_status_row(socket, surface)" in l)
        elif_row = find(lambda l: "row = None if stub else _surface_status_row(socket, surface)" in l
                        and "_p11_row" not in l)
        check("ruling-2 the Phase-11 branch reads its own status row before its seat check",
              len(own_row) == 1 and len(elif_row) == 1 and p11_if[0] < own_row[0] < stub_arm[0] < elif_row[0])

        # ──────────────────────────────────────────────────────────────────────
        # M: ★D3 **거동** 핀 — 실 `run_restore` 를 태워 조립된 결과를 본다. L 절은 구조와 순수 함수를
        #    묶고, 이 절은 「그 구조가 실제로 그 판정을 낸다」를 묶는다(가드가 빠지면 여기서 붉어진다).
        def d3_run(seed_stale):
            """resume 역할 1개 · 좌석은 건강하게 올라온다. seed_stale 이면 **지난 부팅의** 독약 표지를 심는다."""
            td2 = tempfile.mkdtemp(prefix="phoenix-d3-run-")
            orig = (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN)
            d3_state_env = {"LOCALAPPDATA": os.environ.get("LOCALAPPDATA")}
            try:
                # ★통합(worker-18 · d3 impl r1 gemini minor · vj:6a2c8071): 이 헬퍼가 CYS_PACK_DIR 을 세우지 않으면
                #   `~/.cys/pack/agents.json` 이 있는 호스트에서 **호스트 팩**을 읽어 resume 모드가 unknown→effective 로
                #   바뀌고 D3-B3 대조가 적색이 된다(실측: 오염된 HOME 에서 318/319 · FAIL D3-B3). 섹션 L 과 같은 규약으로
                #   PACK_DIR_ENV_KEYS 를 비우고 **빈** 임시 팩을 가리킨다(선언 부재 = 이 행들이 통과하던 상태).
                d3_pack = os.path.join(td2, "pack")
                os.makedirs(d3_pack, exist_ok=True)
                d3_pack_env = {k: os.environ.get(k) for k in m.PACK_DIR_ENV_KEYS}
                for _k in m.PACK_DIR_ENV_KEYS:
                    os.environ.pop(_k, None)
                os.environ["CYS_PACK_DIR"] = d3_pack
                m._AGENTS_CACHE.clear()
                sock = os.path.join(td2, "cys.sock")
                # ★codex W1: 제품이 읽는 자리에 쓴다(unix=소켓 부모 · Windows=슬러그 매핑) — 단언이 쓰기보다 먼저.
                state_dir = bind_rig_state_root(td2, sock)
                tick = "d3-" + ("stale" if seed_stale else "clean")
                now = m._now()
                entry = {"role": "worker-1", "agent": "claude", "session_id": "sid-live",
                         "claude_config_dir": os.path.join(td2, "acct"), "cwd": os.path.join(td2, "proj"),
                         "surface_id": 7, "surface_ref": "surface:7"}
                os.makedirs(entry["cwd"])
                proj = os.path.join(entry["claude_config_dir"], "projects",
                                    m._claude_project_component(entry["cwd"]))
                os.makedirs(proj)
                with open(os.path.join(proj, "sid-live.jsonl"), "w", encoding="utf-8") as f:
                    f.write("{}\n")       # 세션 파일이 있으므로 fresh 예상이 아니다 → else(핀) 분지로 간다
                with open(os.path.join(state_dir, "topology.json"), "w", encoding="utf-8") as f:
                    json.dump({"entries": [entry], "updated_at": now, "schema_version": 1,
                               "tombstones": [], "tombstones_rev": 0}, f)
                jp = m.journal_path(sock, tick)
                os.makedirs(os.path.dirname(jp), exist_ok=True)
                if seed_stale:
                    m._atomic_write_json(jp, {"ticket_id": tick, "events": [], "created": now, "roles": {
                        "worker-1": {"surface": "surface:7", "expected_sid": "sid-live",
                                     "observed_sid": "sid-live", "fresh_fallback": True,
                                     "fresh_reason": "poison", "fresh_epoch": "sa:1.0", "outcome": "fresh",
                                     "stages": {"verify": {"done": True, "ts": 1, "epoch": "sa:1.0"}}}}})
                st = {"alive": False}
                def fake_cys(*a, socket=None, timeout=25):
                    assert socket == sock, "CLI escaped the isolated socket"
                    v, out = a[0], ""
                    if v == "status":
                        rows = ([{"surface_id": 7, "surface_ref": "surface:7", "role": "worker-1",
                                  "exited": False, "agent_alive": True, "seat": "occupied", "agent": "claude",
                                  "gate_pending": None, "registered_session_id": "sid-live",
                                  "awakened_at": now}] if st["alive"] else [])
                        out = json.dumps({"daemon": {"started_at": now}, "surfaces": rows})
                    elif v == "restore":
                        st["alive"] = True; out = "restored worker-1 surface:7"
                    elif v == "reinject":
                        out = "디렉티브 생존 확인 (ACK 수신)"
                    elif v in ("read-screen", "watch"):
                        out = "bypass permissions on"
                    elif v == "list":
                        out = "surface:7 role=worker-1 pid=7 exited=false\n" if st["alive"] else ""
                    else:
                        raise AssertionError("unexpected cys call: %r" % (a,))
                    return __import__("types").SimpleNamespace(returncode=0, stdout=out, stderr="")
                m.cys = fake_cys
                m.time.sleep = lambda _s: None
                res = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                with open(jp, encoding="utf-8") as f:
                    jr = json.load(f)
                return res, jr
            finally:
                # ★팩 env 복원(섹션 L 과 같은 규약) — 다음 섹션이 호스트 팩을 물려받지 않게.
                for _k, _v in d3_pack_env.items():
                    if _v is None:
                        os.environ.pop(_k, None)
                    else:
                        os.environ[_k] = _v
                restore_env(d3_state_env)
                m._AGENTS_CACHE.clear()
                (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN) = orig
                shutil.rmtree(td2, ignore_errors=True)

        res_s, jr_s = d3_run(True)
        rr_s = jr_s["roles"]["worker-1"]
        check("D3-B1 a stale marker makes the assembled verdict UNVERIFIED, not VERIFIED_FRESH",
              res_s["phoenix_restore"] == "UNVERIFIED"
              and res_s["per_role_outcome"] == {"worker-1": "unverified"}
              and m.restore_exit_code(res_s) == m.PHOENIX_EXIT_DEGRADED)
        check("D3-B2 the result names the stripped role and the journal records why",
              res_s.get("fresh_scope_stripped", {}).get("worker-1", "").startswith("표지 세대")
              and "fresh_fallback" not in rr_s
              and any(e["stage"] == "verify" and e["status"] == "fresh_scope_stripped"
                      for e in jr_s["events"])
              and "D3" in rr_s["stages"]["verify"]["evidence"]
              and rr_s["stages"]["verify"]["done"] is False)
        res_c, jr_c = d3_run(False)
        ev_c = jr_c["roles"]["worker-1"]["stages"]["verify"]["evidence"]
        # ★CONTROL: 같은 세계에서 낡은 표지만 없으면 이 수리는 **아무것도 하지 않는다** — 역할은 종전처럼
        #   핀 분지로 가고, 그 분지의 종전 결론(이 하네스의 팩에는 agents.json 이 없어 resume 모드 '미상' ⇒
        #   핀 일치를 verified 로 올리지 않는다 · 선재 가드)이 그대로 선다. 같은 'unverified' 이지만 **사유가
        #   다르다**는 것이 요점이다: 내 가드 문면이 아니라 legacy_verify_outcome 의 문면이어야 한다.
        check("D3-B3 CONTROL without a stale marker the verdict comes from the pin branch, not from my guard",
              res_c.get("fresh_scope_stripped") == {}
              and res_c["per_role_outcome"] == {"worker-1": "unverified"}
              and "resume 모드 미상" in ev_c and "D3" not in ev_c
              and m.legacy_verify_outcome("sid-live", "sid-live", "unknown")[0] == "unverified"
              and m.legacy_verify_outcome("sid-live", "sid-live", "effective")[0] == "verified")
        check("D3-B4 CONTROL the two paths are distinguishable in the journal evidence",
              "D3" in jr_s["roles"]["worker-1"]["stages"]["verify"]["evidence"] and "D3" not in ev_c)

        # ──────────────────────────────────────────────────────────────────────
        # N: ★D3 **스탬프**를 제품이 직접 쓰는지 — 손으로 넣지 않는다(부서장 구현 판정 1 B1 · 요구 b).
        #    여기서는 `fresh_epoch` 를 픽스처에 **절대** 적지 않는다. 실 Phase-11 작성자가 쓰고,
        #    실 scope 패스가 읽고, 실 가드가 판정한다. 세 시도(1·2=같은 세대 · 3=새 세대)를 한 저널로 잇는다.
        def d3_stamp_run():
            td3 = tempfile.mkdtemp(prefix="phoenix-d3-stamp-")
            orig = (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
                    m.SPAWN_RETRIES, m.SPAWN_SETTLE, m.SPAWN_BACKOFF, m.F1_REGISTERED_GRACE_TRIES)
            d3_state_env = {"LOCALAPPDATA": os.environ.get("LOCALAPPDATA")}
            try:
                # ★통합(worker-18 · d3 impl r1 gemini minor · vj:6a2c8071): 이 헬퍼가 CYS_PACK_DIR 을 세우지 않으면
                #   `~/.cys/pack/agents.json` 이 있는 호스트에서 **호스트 팩**을 읽어 resume 모드가 unknown→effective 로
                #   바뀌고 D3-B3 대조가 적색이 된다(실측: 오염된 HOME 에서 318/319 · FAIL D3-B3). 섹션 L 과 같은 규약으로
                #   PACK_DIR_ENV_KEYS 를 비우고 **빈** 임시 팩을 가리킨다(선언 부재 = 이 행들이 통과하던 상태).
                d3_pack = os.path.join(td3, "pack")
                os.makedirs(d3_pack, exist_ok=True)
                d3_pack_env = {k: os.environ.get(k) for k in m.PACK_DIR_ENV_KEYS}
                for _k in m.PACK_DIR_ENV_KEYS:
                    os.environ.pop(_k, None)
                os.environ["CYS_PACK_DIR"] = d3_pack
                m._AGENTS_CACHE.clear()
                sock = os.path.join(td3, "cys.sock")
                # ★codex W1: 제품이 읽는 자리에 쓴다 — 단언이 쓰기보다 먼저(M 절과 같은 규약).
                state_dir = bind_rig_state_root(td3, sock)
                tick = "d3-stamp"
                entry = {"role": "worker-1", "agent": "claude", "session_id": "sid-poison",
                         "claude_config_dir": os.path.join(td3, "acct"), "cwd": os.path.join(td3, "proj"),
                         "surface_id": 7, "surface_ref": "surface:7"}
                os.makedirs(entry["cwd"])
                proj = os.path.join(entry["claude_config_dir"], "projects",
                                    m._claude_project_component(entry["cwd"]))
                os.makedirs(proj)
                # 세션 파일이 **있으므로** F-1 fresh 예상이 아니다 → 독약 강등(Phase 11)만이 표지를 쓴다.
                with open(os.path.join(proj, "sid-poison.jsonl"), "w", encoding="utf-8") as f:
                    f.write("{}\n")
                st = {"epoch": 1000.0, "phase": 1, "alive": False, "seat": "empty",
                      "gate": {"gate": "folder-trust"}}
                def write_topology():
                    with open(os.path.join(state_dir, "topology.json"), "w", encoding="utf-8") as f:
                        json.dump({"entries": [entry], "updated_at": m._now(), "schema_version": 1,
                                   "tombstones": [], "tombstones_rev": 0}, f)
                write_topology()
                def fake_cys(*a, socket=None, timeout=25):
                    v, out = a[0], ""
                    if v == "status":
                        rows = []
                        if st["alive"]:
                            rows = [{"surface_id": 7, "surface_ref": "surface:7", "role": "worker-1",
                                     "exited": False, "agent_alive": True, "seat": st["seat"],
                                     "agent": "claude", "gate_pending": st["gate"],
                                     "registered_session_id": "sid-fresh", "awakened_at": m._now()}]
                        out = json.dumps({"daemon": {"started_at": st["epoch"]}, "surfaces": rows})
                    elif v == "restore":
                        # ★독약: cys restore 는 좌석을 띄우지 못한다(resume 불가) → Phase 11 로 내려간다.
                        out = "no surface for worker-1"
                    elif v == "launch-agent":
                        st["alive"] = True          # ★무 resume 재기동이 좌석을 만든다(Phase 11 의 실 경로)
                        out = "launched worker-1 surface:7"
                    elif v == "reinject":
                        out = "디렉티브 생존 확인 (ACK 수신)"
                    elif v in ("read-screen", "watch"):
                        out = "bypass permissions on"
                    elif v == "list":
                        out = "surface:7 role=worker-1 pid=7 exited=false\n" if st["alive"] else ""
                    else:
                        raise AssertionError("unexpected cys call: %r" % (a,))
                    return __import__("types").SimpleNamespace(returncode=0, stdout=out, stderr="")
                m.cys = fake_cys
                m.time.sleep = lambda _s: None
                m.SPAWN_RETRIES, m.SPAWN_SETTLE, m.SPAWN_BACKOFF = 1, 0.0, 0.0
                m.F1_REGISTERED_GRACE_TRIES = 1
                jp = m.journal_path(sock, tick)
                res, jr = {}, {}
                outs = []
                # attempt 1 — 관문에 갇힌 좌석: 실 작성자가 표지를 쓰고, 좌석 축이 막아 unverified.
                r1 = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                j1 = json.load(open(jp, encoding="utf-8"))
                outs.append((r1, j1))
                # attempt 2 — **같은 세대**: 좌석이 건강해졌다(관문 통과). 역할은 여전히 target(빈 좌석)이고
                #   spawn 마킹이 이 세대라 재스폰은 skip → 표지를 **다시 쓰지 않는다** ⇒ 스탬프가 유일한 근거다.
                st["gate"] = None
                r2 = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                outs.append((r2, json.load(open(jp, encoding="utf-8"))))
                # attempt 3 — **새 세대**(데몬 재시작): 같은 저널, 표지는 지난 부팅의 것 ⇒ 걷어내고 가드가 발화.
                st["epoch"] = 2000.0
                r3 = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                outs.append((r3, json.load(open(jp, encoding="utf-8"))))
                return outs
            finally:
                # ★팩 env 복원(섹션 L 과 같은 규약) — 다음 섹션이 호스트 팩을 물려받지 않게.
                for _k, _v in d3_pack_env.items():
                    if _v is None:
                        os.environ.pop(_k, None)
                    else:
                        os.environ[_k] = _v
                restore_env(d3_state_env)
                m._AGENTS_CACHE.clear()
                (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
                 m.SPAWN_RETRIES, m.SPAWN_SETTLE, m.SPAWN_BACKOFF, m.F1_REGISTERED_GRACE_TRIES) = orig
                shutil.rmtree(td3, ignore_errors=True)

        (ra, ja), (rb, jb), (rc_, jc) = d3_stamp_run()
        rra, rrb, rrc = (ja["roles"]["worker-1"], jb["roles"]["worker-1"], jc["roles"]["worker-1"])
        # D3-S1 ★제품이 스탬프를 쓴다 — 저널에서 되읽은 값이 그 실행의 boot_epoch 과 같다.
        check("D3-S1 the product writes fresh_epoch and it equals the run's boot_epoch",
              rra.get("fresh_epoch") is not None and rra["fresh_epoch"] == ra["boot_epoch"]
              and rra.get("fresh_fallback") is True)
        # D3-S2 시도 1 — 관문에 갇힌 좌석이면 이 부팅의 표지라도 'fresh' 가 아니다.
        check("D3-S2 attempt 1: a this-boot marker on a gate-trapped seat is unverified",
              ra["per_role_outcome"] == {"worker-1": "unverified"}
              and ra.get("fresh_scope_stripped") == {}
              and "좌석 증거 미충족" in rra["stages"]["verify"]["evidence"])
        # D3-S3 시도 2 — 같은 세대: 표지가 scope 패스를 **살아서** 지나가고 좌석 증거로 판정된다.
        check("D3-S3 attempt 2 in the SAME epoch: the marker survives the pass and is judged on evidence",
              rb["boot_epoch"] == ra["boot_epoch"]
              and rb.get("fresh_scope_stripped") == {}
              and rrb.get("fresh_fallback") is True
              and rb["per_role_outcome"] == {"worker-1": "fresh"})
        # D3-S4 시도 3 — 새 세대: 걷어내고 가드가 발화한다(핀 비교로 내려가지 않는다).
        check("D3-S4 attempt 3 in a NEW epoch: stripped, the guard fires, unverified",
              rc_["boot_epoch"] != ra["boot_epoch"]
              and rc_.get("fresh_scope_stripped", {}).get("worker-1", "").startswith("표지 세대")
              and "fresh_fallback" not in rrc
              and rc_["per_role_outcome"] == {"worker-1": "unverified"}
              and "D3" in rrc["stages"]["verify"]["evidence"])
        # D3-S5 ★사유 문면이 참이어야 한다 — 이 수리가 쓴 표지를 「이 수리 이전 저널」이라고 적지 않는다.
        check("D3-S5 the strip reason names the epoch mismatch, not a pre-fix journal",
              "이 수리 이전 저널" not in rc_.get("fresh_scope_stripped", {}).get("worker-1", "")
              and "≠ 이번 부팅" in rc_.get("fresh_scope_stripped", {}).get("worker-1", ""))

        # ──────────────────────────────────────────────────────────────────────
        # O: ★D3 스탬프의 **F-1 작성자** 쪽 — 절 N 은 Phase-11 작성자만 태운다. 변이 'F-1 스탬프 제거' 가
        #    절 N 만으로는 **살아남았다**(09-mutations-stamp.log 1회차) ⇒ 그 작성자를 직접 태우는 세계가 필요하다.
        #    세션 파일이 **없는** 역할이면 `fresh_expected(entry)` 가 참이라 F-1 블록이 표지를 쓴다.
        def d3_f1_stamp_run():
            td4 = tempfile.mkdtemp(prefix="phoenix-d3-f1stamp-")
            orig = (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
                    m.SPAWN_SETTLE, m.SPAWN_BACKOFF, m.F1_REGISTERED_GRACE_TRIES)
            d3_state_env = {"LOCALAPPDATA": os.environ.get("LOCALAPPDATA")}
            try:
                # ★통합(worker-18 · d3 impl r1 gemini minor · vj:6a2c8071): 이 헬퍼가 CYS_PACK_DIR 을 세우지 않으면
                #   `~/.cys/pack/agents.json` 이 있는 호스트에서 **호스트 팩**을 읽어 resume 모드가 unknown→effective 로
                #   바뀌고 D3-B3 대조가 적색이 된다(실측: 오염된 HOME 에서 318/319 · FAIL D3-B3). 섹션 L 과 같은 규약으로
                #   PACK_DIR_ENV_KEYS 를 비우고 **빈** 임시 팩을 가리킨다(선언 부재 = 이 행들이 통과하던 상태).
                d3_pack = os.path.join(td4, "pack")
                os.makedirs(d3_pack, exist_ok=True)
                d3_pack_env = {k: os.environ.get(k) for k in m.PACK_DIR_ENV_KEYS}
                for _k in m.PACK_DIR_ENV_KEYS:
                    os.environ.pop(_k, None)
                os.environ["CYS_PACK_DIR"] = d3_pack
                m._AGENTS_CACHE.clear()
                sock = os.path.join(td4, "cys.sock")
                # ★codex W1: 제품이 읽는 자리에 쓴다 — 단언이 쓰기보다 먼저(M 절과 같은 규약).
                state_dir = bind_rig_state_root(td4, sock)
                tick = "d3-f1stamp"
                entry = {"role": "worker-1", "agent": "claude", "session_id": "",
                         "claude_config_dir": os.path.join(td4, "acct"), "cwd": os.path.join(td4, "proj"),
                         "surface_id": 7, "surface_ref": "surface:7"}
                os.makedirs(entry["cwd"])
                proj = os.path.join(entry["claude_config_dir"], "projects",
                                    m._claude_project_component(entry["cwd"]))
                os.makedirs(proj)      # ★세션 파일을 만들지 않는다 → F-1 fresh 예상
                with open(os.path.join(state_dir, "topology.json"), "w", encoding="utf-8") as f:
                    json.dump({"entries": [entry], "updated_at": m._now(), "schema_version": 1,
                               "tombstones": [], "tombstones_rev": 0}, f)
                st = {"epoch": 3000.0, "alive": False}
                def fake_cys(*a, socket=None, timeout=25):
                    v, out = a[0], ""
                    if v == "status":
                        rows = ([{"surface_id": 7, "surface_ref": "surface:7", "role": "worker-1",
                                  "exited": False, "agent_alive": True, "seat": "occupied", "agent": "claude",
                                  "gate_pending": None, "registered_session_id": "new1",
                                  "awakened_at": m._now()}] if st["alive"] else [])
                        out = json.dumps({"daemon": {"started_at": st["epoch"]}, "surfaces": rows})
                    elif v == "restore":
                        st["alive"] = True
                        with open(os.path.join(proj, "new1.jsonl"), "w", encoding="utf-8") as f:
                            f.write("{}\n")
                        out = "restored worker-1 surface:7"
                    elif v == "reinject":
                        out = "디렉티브 생존 확인 (ACK 수신)"
                    elif v in ("read-screen", "watch"):
                        out = "bypass permissions on"
                    elif v == "list":
                        out = "surface:7 role=worker-1 pid=7 exited=false\n" if st["alive"] else ""
                    else:
                        raise AssertionError("unexpected cys call: %r" % (a,))
                    return __import__("types").SimpleNamespace(returncode=0, stdout=out, stderr="")
                m.cys = fake_cys
                m.time.sleep = lambda _s: None
                m.SPAWN_SETTLE, m.SPAWN_BACKOFF = 0.0, 0.0
                m.F1_REGISTERED_GRACE_TRIES = 1
                jp = m.journal_path(sock, tick)
                r1 = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                j1 = json.load(open(jp, encoding="utf-8"))
                st["epoch"] = 4000.0          # 데몬 재시작
                r2 = m.run_restore(sock, ticket=tick, stub=False, no_breaker=True, print_result=False)
                j2 = json.load(open(jp, encoding="utf-8"))
                return (r1, j1), (r2, j2)
            finally:
                # ★팩 env 복원(섹션 L 과 같은 규약) — 다음 섹션이 호스트 팩을 물려받지 않게.
                for _k, _v in d3_pack_env.items():
                    if _v is None:
                        os.environ.pop(_k, None)
                    else:
                        os.environ[_k] = _v
                restore_env(d3_state_env)
                m._AGENTS_CACHE.clear()
                (m.cys, m.time.sleep, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
                 m.SPAWN_SETTLE, m.SPAWN_BACKOFF, m.F1_REGISTERED_GRACE_TRIES) = orig
                shutil.rmtree(td4, ignore_errors=True)

        (rf1, jf1), (rf2, jf2) = d3_f1_stamp_run()
        rrf1 = jf1["roles"]["worker-1"]
        check("D3-F1 the F-1 writer stamps too, and the value equals that run's boot_epoch",
              rrf1.get("fresh_expected") is True
              and rrf1.get("fresh_epoch") is not None
              and rrf1["fresh_epoch"] == rf1["boot_epoch"]
              and rf1.get("fresh_scope_stripped") == {})
        # ★실측으로 정정한 기대: 이 세계의 두 번째 실행에서 역할은 **살아 있다**(좌석 점유) ⇒ 부활 대상이
        #   아니고(`target_roles` 공집합 · NOOP) verify 자체가 돌지 않는다. 그래서 여기서 재는 것은 **스탬프
        #   불일치가 실제로 표지를 걷어내는가**이고, 가드 발화는 역할이 계속 대상으로 남는 절 N(D3-S4)이 묶는다.
        #   스탬프도 표지와 함께 사라져야 한다(읽히지 않는 찌꺼기 금지).
        check("D3-F2 a new epoch strips the F-1 marker group, the stamp with it",
              rf2["boot_epoch"] != rf1["boot_epoch"]
              and rf2.get("fresh_scope_stripped", {}).get("worker-1", "").startswith("표지 세대")
              and not any(k in jf2["roles"]["worker-1"] for k in m.FRESH_MARKER_KEYS)
              and "fresh_epoch" not in jf2["roles"]["worker-1"]
              and rf2["target_roles"] == [] and rf2["per_role_outcome"] == {})
        # D1 (defect `D5H-20261028-PHOENIX-PIN-VERIFY-SAME-SOURCE`): the resumed-role verdict no longer
        # compares topology with itself. Rows are prefixed "d1 " so the Windows step log carries an explicit
        # marker, and the file's own `=== N/N PASS ===` is the positive control (N is computed, never written).
        # Design: DESIGN.md 34f04804 + DESIGN-R2.md 4d7ecb83 + DESIGN-R3.md 30efd72f + HEAD-RULING-R3-FINAL
        # conditions 1-6.
        # ──────────────────────────────────────────────────────────────────────
        check("d1 surface exists", all(hasattr(m, n) for n in (
            "resumed_verify_outcome", "_flat_surface_refs", "restore_final_enum",
            "_last_liveness_provenance", "RESUME_REOBSERVABLE", "RESUME_SEAT_BOUND_AGENTS")))
        check("d1 reobservable legs", m.RESUME_REOBSERVABLE == ("observe", "compare", "file"))
        check("d1 seat-bound agents", m.RESUME_SEAT_BOUND_AGENTS == ("claude",))

        # ★M1: both snapshots are the flat ref set over ALL keys of the role-keyed map — the literal "-"
        #   bucket and exited rows included (javis_phoenix.py:1865/:1869; skipping role=null is a named
        #   P1-6 regression at :1864).
        check("d1 flat refs over all keys incl '-' and exited",
              m._flat_surface_refs({"-": [{"surface": "surface:9", "exited": False}],
                                    "w": [{"surface": "surface:3", "exited": True}]})
              == {"surface:9", "surface:3"})
        check("d1 flat refs of a non-dict is None", m._flat_surface_refs(None) is None)

        d1_live = {"surface_ref": "surface:9", "exited": False, "agent_alive": True,
                   "gate_pending": None, "registered_session_id": "SID-1", "agent": "claude"}
        d1_dir = os.path.join(td, "d1proj")
        os.makedirs(d1_dir, exist_ok=True)
        with open(os.path.join(d1_dir, "SID-1.jsonl"), "w", encoding="utf-8") as f:
            f.write("{}\n")

        def d1_rr(**over):
            base = {"expected_sid": "SID-1", "resume_mode": "effective", "resume_project_dir": d1_dir,
                    "surface_ref": "surface:9",
                    "pre_spawn_snapshot_source": "status_json", "pre_spawn_snapshot_known": True,
                    "pre_spawn_snapshot_enumerated": True,
                    "pre_spawn_surface_refs": ["surface:3"],
                    "post_spawn_surface_refs": ["surface:3", "surface:9"],
                    "registered_before_measured": False, "registered_before": None,
                    "registered_before_surface": None}
            base.update(over)
            return base

        # ★CONDITION 2 — one row per `pre_is_observation` INPUT SHAPE, each produced by the SHIPPED
        #   `_live_surfaces_raw` (not by a hand-set dict), with its expected leg-7 result.
        #   The pane is warm in every row (it is in both snapshots) with a stale registration equal to the
        #   pin, so the ONLY thing that can make leg 7 pass is a genuine "the pane is new" observation.
        def d1_snapshot(status_rc, status_stdout, list_stdout=""):
            """Drive the real liveness parser once and return (flat refs, provenance)."""
            def scripted(*args, socket=None, timeout=25):
                if args[0] == "status":
                    return SimpleNamespace(returncode=status_rc, stdout=status_stdout, stderr="")
                if args[0] == "list":
                    return SimpleNamespace(returncode=0, stdout=list_stdout, stderr="")
                raise AssertionError("unexpected cys call in d1 snapshot: %r" % (args,))
            saved = m.cys
            try:
                m.cys = scripted
                mp = m._live_surfaces_raw("sock-d1")
            finally:
                m.cys = saved
            return m._flat_surface_refs(mp), m._last_liveness_provenance()

        other_rows = [{"surface_id": 3, "surface_ref": "surface:3", "role": "worker-2", "exited": False,
                       "agent_alive": True, "seat": "occupied", "agent": "claude",
                       "gate_pending": None, "registered_session_id": "SID-OTHER"}]
        warm_rows = [{"surface_id": 9, "surface_ref": "surface:9", "role": "worker-1", "exited": False,
                      "agent_alive": True, "seat": "occupied", "agent": "claude",
                      "gate_pending": None, "registered_session_id": "SID-1"}]
        shapes = (
            # name, status rc, status stdout, list stdout, expect enumerated/known/source, expect leg 7 pass
            ("status --json rc!=0 -> list fallback (empty, known=True)",
             1, "", "", {"source": "list_fallback", "known": True, "enumerated": False}, False),
            ("list fallback structure drift (known=False)",
             1, "", "junk line with no surface\n", {"source": "list_fallback", "known": False,
                                                    "enumerated": False}, False),
            ("rc 0 with EMPTY stdout -> {} (no surfaces key)",
             0, "", "", {"source": "status_json", "known": True, "enumerated": False}, False),
            ("rc 0 with JSON lacking 'surfaces'",
             0, '{"daemon": {"started_at": 1.0}}', "", {"source": "status_json", "known": True,
                                                        "enumerated": False}, False),
            ("cold boot: 'surfaces': [] present",
             0, '{"surfaces": []}', "", {"source": "status_json", "known": True, "enumerated": True}, True),
            # populated with OTHER surfaces — the provenance is good and the target pane is genuinely absent.
            ("populated structured snapshot (target pane absent)",
             0, json.dumps({"surfaces": other_rows}), "", {"source": "status_json", "known": True,
                                                           "enumerated": True}, True),
        )
        for name, rc, out, lst, prov_want, leg7_ok in shapes:
            refs, prov = d1_snapshot(rc, out, lst)
            check("d1 provenance: " + name,
                  prov["source"] == prov_want["source"] and prov["known"] == prov_want["known"]
                  and prov["enumerated"] == prov_want["enumerated"])
            # the cold-boot and populated rows are the two that may pass leg 7, and only when the pane is
            # genuinely absent from the pre-snapshot: give each shape a warm pane for the refusal rows and
            # an absent pane for the two that should pass.
            pre = sorted(refs or [])
            rr = d1_rr(pre_spawn_snapshot_source=prov["source"], pre_spawn_snapshot_known=prov["known"],
                       pre_spawn_snapshot_enumerated=prov["enumerated"],
                       pre_spawn_surface_refs=pre if leg7_ok else ["surface:9"],
                       post_spawn_surface_refs=sorted(set(pre) | {"surface:9"}))
            outcome, missing, ev = m.resumed_verify_outcome(rr, d1_live, "claude")
            check("d1 leg7 for shape: " + name,
                  (outcome == "verified") == leg7_ok
                  and (leg7_ok or (ev["leg"] == "attribution" and not ev["pane_is_new"])))

        # ★the violating control: the SAME inputs that the fallback shape refuses become `verified` when the
        #   provenance is forged to claim a structured, known, ENUMERATED snapshot. That is the conjunct the
        #   head's condition 1 adds, measured as the only difference.
        refs_fb, prov_fb = d1_snapshot(1, "", "")
        rr_real = d1_rr(pre_spawn_snapshot_source=prov_fb["source"], pre_spawn_snapshot_known=prov_fb["known"],
                        pre_spawn_snapshot_enumerated=prov_fb["enumerated"],
                        pre_spawn_surface_refs=sorted(refs_fb or []),
                        post_spawn_surface_refs=["surface:9"])
        rr_forged = dict(rr_real, pre_spawn_snapshot_source="status_json",
                         pre_spawn_snapshot_known=True, pre_spawn_snapshot_enumerated=True)
        real_outcome = m.resumed_verify_outcome(rr_real, d1_live, "claude")[0]
        forged_outcome = m.resumed_verify_outcome(rr_forged, d1_live, "claude")[0]
        check("d1 violating control: forging the provenance is what buys 'verified'",
              real_outcome == "unverified" and forged_outcome == "verified")

        # ★the three provenance clauses overlap on the shapes the SHIPPED parser can produce (enumerated is
        #   only ever set in the structured branch, and that branch always ends known=True), so each clause
        #   needs a row where it is the ONLY discriminator — otherwise a mutation that drops it stays green
        #   and the clause is defence in name only. These two forge a CONTRADICTORY provenance (the shape a
        #   future edit to `_live_surfaces_raw` would produce) and one pins the implication itself.
        check("d1 shipped parser never yields enumerated with a non-structured source or known=False",
              all(not prov["enumerated"] or (prov["source"] == "status_json" and prov["known"])
                  for prov in (d1_snapshot(rc, out, lst)[1] for name, rc, out, lst, _w, _ok in shapes)))
        check("d1 contradictory provenance: enumerated but source=list_fallback is refused",
              m.resumed_verify_outcome(
                  d1_rr(pre_spawn_snapshot_source="list_fallback", pre_spawn_snapshot_enumerated=True,
                        pre_spawn_surface_refs=[]), d1_live, "claude")[0] == "unverified")
        check("d1 contradictory provenance: enumerated but known=False is refused",
              m.resumed_verify_outcome(
                  d1_rr(pre_spawn_snapshot_known=False, pre_spawn_snapshot_enumerated=True,
                        pre_spawn_surface_refs=[]), d1_live, "claude")[0] == "unverified")

        # ★B1(c)/R3: a MEASURED pre-read on this surface outranks a snapshot that failed to list it.
        check("d1 measured pre-read == pin on an empty structured snapshot is refused",
              m.resumed_verify_outcome(
                  d1_rr(pre_spawn_surface_refs=[], registered_before_measured=True,
                        registered_before="SID-1", registered_before_surface="surface:9"),
                  d1_live, "claude")[0] == "unverified")
        check("d1 measured transition on this surface is accepted",
              m.resumed_verify_outcome(
                  d1_rr(pre_spawn_surface_refs=["surface:9"], post_spawn_surface_refs=["surface:9"],
                        registered_before_measured=True, registered_before="SID-PREV",
                        registered_before_surface="surface:9"),
                  d1_live, "claude")[0] == "verified")
        check("d1 transition measured on another surface is refused",
              m.resumed_verify_outcome(
                  d1_rr(pre_spawn_surface_refs=["surface:9"], post_spawn_surface_refs=["surface:9"],
                        registered_before_measured=True, registered_before="SID-PREV",
                        registered_before_surface="surface:77"),
                  d1_live, "claude")[0] == "unverified")

        # the strict table: every wrong or unknown input, with the leg that refused it.
        d1_strict = (
            ("pin stale", d1_rr(), dict(d1_live, registered_session_id="OTHER"), "compare"),
            ("not registered yet", d1_rr(), dict(d1_live, registered_session_id=None), "observe"),
            ("registered key absent", d1_rr(),
             {k: v for k, v in d1_live.items() if k != "registered_session_id"}, "channel"),
            ("status row absent", d1_rr(), None, "seat"),
            ("seat exited", d1_rr(), dict(d1_live, exited=True), "seat"),
            ("agent not alive", d1_rr(), dict(d1_live, agent_alive=False), "seat"),
            ("pin empty", d1_rr(expected_sid=""), d1_live, "pin"),
            ("pin not a filename piece", d1_rr(expected_sid="../escape"), d1_live, "pin"),
            ("resume mode unknown", d1_rr(resume_mode="unknown"), d1_live, "guard"),
            ("pre snapshot missing", d1_rr(pre_spawn_surface_refs=None), d1_live, "attribution"),
            ("post snapshot missing", d1_rr(post_spawn_surface_refs=None), d1_live, "attribution"),
            ("pane already in the pre snapshot", d1_rr(pre_spawn_surface_refs=["surface:9"]), d1_live,
             "attribution"),
            ("project dir unknown", d1_rr(resume_project_dir=None), d1_live, "file"),
        )
        for name, rr, row, leg in d1_strict:
            outcome, missing, ev = m.resumed_verify_outcome(rr, row, "claude")
            check("d1 strict %s -> unverified(%s)" % (name, leg),
                  outcome == "unverified" and ev["leg"] == leg and missing)
        check("d1 transcript absent -> unverified(file)",
              m.resumed_verify_outcome(d1_rr(expected_sid="SID-NOFILE"), dict(
                  d1_live, registered_session_id="SID-NOFILE"), "claude")[0:1] == ("unverified",))
        check("d1 no false verified anywhere in the strict table",
              not any(m.resumed_verify_outcome(rr, row, "claude")[0] == "verified"
                      for name, rr, row, leg in d1_strict))
        for agent in ("codex", "gemini", "agy"):
            outcome, missing, ev = m.resumed_verify_outcome(d1_rr(), d1_live, agent)
            check("d1 %s has no seat-bound channel -> seat_only" % agent,
                  outcome == "seat_only" and ev["channel"] == "none" and missing)
        check("d1 happy resume on a new pane -> verified",
              m.resumed_verify_outcome(d1_rr(), d1_live, "claude") [0] == "verified")
        check("d1 project dir derived from the entry when the journal field is absent",
              m.resumed_verify_outcome(
                  d1_rr(resume_project_dir=None), d1_live, "claude",
                  entry={"claude_config_dir": os.path.dirname(d1_dir), "cwd": "/x"}) [0] == "unverified")

        # ★M2: the aggregation keeps the pinned FAILED arm and its order (javis_phoenix.py:3101/:3119-3122).
        d1_enum = (
            ({}, "NOOP"),
            ({"a": "verified"}, "VERIFIED"),
            ({"a": "verified", "b": "fresh"}, "VERIFIED_FRESH"),
            ({"a": "fresh", "b": "seat_only"}, "VERIFIED_FRESH"),
            ({"a": "verified", "b": "seat_only"}, "VERIFIED_SEAT_ONLY"),
            ({"a": "verified", "b": "unverified"}, "UNVERIFIED"),
            ({"a": "seat_only", "b": "unverified"}, "UNVERIFIED"),
            ({"a": "failed"}, "FAILED"),
            ({"a": "failed", "b": "verified"}, "FAILED"),
            ({"a": "failed", "b": "seat_only"}, "FAILED"),
            ({"a": "failed", "b": "unverified"}, "UNVERIFIED"),
        )
        for outcomes_in, want in d1_enum:
            check("d1 enum %s -> %s" % (sorted(set(outcomes_in.values())) or "empty", want),
                  m.restore_final_enum(outcomes_in) == want)
        for verdict, code in (("VERIFIED", 0), ("VERIFIED_FRESH", 0), ("VERIFIED_SEAT_ONLY", 0),
                              ("UNVERIFIED", 3), ("FAILED", 1), ("NOOP", 0)):
            check("d1 exit code %s -> %d" % (verdict, code),
                  m.restore_exit_code({"phoenix_restore": verdict}) == code)
        # the deploy consumer (javis_phoenix.py:4242) must accept the new verdict as COMPLETE/0.
        for verdict, completeness, want in (
                ("VERIFIED", "COMPLETE", ("COMPLETE", 0)),
                ("VERIFIED_FRESH", "COMPLETE", ("COMPLETE", 0)),
                ("VERIFIED_SEAT_ONLY", "COMPLETE", ("COMPLETE", 0)),
                ("UNVERIFIED", "COMPLETE", ("DEGRADED", 3)),
                ("NOOP", "NOOP", ("COMPLETE", 0)),
                ("LEASE_HELD", "COMPLETE", ("LEASE_HELD", 3)),
                ("FAILED", "INCOMPLETE", ("FAILED", 1))):
            check("d1 deploy verdict %s/%s -> %s" % (verdict, completeness, want),
                  m.deploy_verdict_for(verdict, completeness) == want)
    finally:
        shutil.rmtree(td)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
