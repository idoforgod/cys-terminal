#!/usr/bin/env python3
"""javis_resource_gate 단위 테스트 — stdlib unittest만 (신규 의존성 0).

대상: 치유 원복 사고로 소실됐다 vendor 흡수된 승인 수정 2건의 회귀 핀.
  ① codex 이중계수 제외(2026-07-11 CSO·CEO B승인) — NODE_EXCLUDE_PATTERNS
  ② nodes hard 동적 부서가산(2026-07-06 CSO 위임·master 승인) — nodes_hard_effective
     ★2026-09-03 A3(SURVEY A4·B6-2 · PREP #8): '부서당 +5' → 'max(18, 12 + Σ데몬 응답 좌석)' 치환.
       활성 부서 = `cys status --json --socket <sock>` 응답 · 좌석 = 비-exited surfaces · 실패 = soft.
+ evaluate가 동적 임계(m["nodes_hard_effective"])를 실제로 소비하는지, 측정 실패
  soft 격상(P-ORCH-1)이 유지되는지.
+ ③ A3 라이브 경로(_dept_roster) — glob·subprocess 대역으로 응답/실패 4형상 밀폐 재현.
+ ④ A1 픽스처(SURVEY A5 표 · F-a 4 · DESIGN A1): claude argv 2형태의 NODE_PATTERNS 계수 특성화 핀.
"""
import json
import os
import subprocess
import sys
import types
import unittest
from unittest import mock

BIN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bin")
if BIN not in sys.path:
    sys.path.insert(0, BIN)

import javis_resource_gate as G  # noqa: E402


def make_args(**over):
    """cmd_check가 받는 argparse Namespace의 테스트 대역 — 기본값은 argparse 정의와 동일."""
    a = types.SimpleNamespace(
        servers_override=None, nodes_override=None, load_override=None,
        context=None, nodes_soft=12, nodes_hard=G.NODES_HARD_DEFAULT,
        servers_soft=2, servers_hard=4,
        load_soft_ratio=1.0, load_hard_ratio=2.0,
        context_soft=50, context_hard=60,
        dept_roster_override=None,
    )
    for k, v in over.items():
        setattr(a, k, v)
    return a


def roster(*seats, errors=()):
    """_dept_roster 대역 — 좌석 목록으로 응답 로스터를 만든다(부서 이름 d1, d2, …)."""
    return {"active": len(seats), "seats": sum(seats), "errors": list(errors),
            "depts": [{"name": "d%d" % (i + 1), "seats": s} for i, s in enumerate(seats)]}


class TestCodexDoubleCountExclusion(unittest.TestCase):
    """① codex 노드 1개 = wrapper + darwin-arm64 native 2프로세스 → wrapper만 계수."""

    def test_native_vendor_binary_excluded(self):
        lines = [
            "  101 node /usr/local/bin/codex serve",                 # wrapper — 계수
            "  102 /Users/x/.codex/bin/codex-darwin-arm64 --child",  # native — 제외
        ]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 1)

    def test_exclusion_off_reproduces_inflation(self):
        # 제외 패턴이 사라지면 이중계수(2)로 회귀 — 수정 소실을 검출하는 음성 대조군.
        lines = [
            "  101 node /usr/local/bin/codex serve",
            "  102 /Users/x/.codex/bin/codex-darwin-arm64 --child",
        ]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, ()), 2)

    def test_self_and_nonnode_not_counted(self):
        lines = [
            "  201 python3 bin/javis_resource_gate.py check",  # 자기 자신 — 제외
            "  202 vim notes.md",                              # 노드 아님
            "  203 claude --dangerously-skip-permissions",     # 계수
        ]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 1)


class TestNodesHardDynamic(unittest.TestCase):
    """② nodes hard = max(정적 floor 18, 12 + Σ데몬 응답 좌석) · --nodes-hard 명시 시 그 값 우선."""

    def _measure(self, r, **arg_over):
        a = make_args(servers_override=0, nodes_override=0, load_override=0.0, **arg_over)
        with mock.patch.object(G, "_dept_roster", return_value=r):
            return G.measure(a)

    def test_static_floor_when_no_depts(self):
        m = self._measure(roster())
        self.assertEqual(m["nodes_hard_effective"], G.NODES_HARD_DEFAULT)
        self.assertEqual((m["active_depts"], m["dept_seats"], m["depts"]), (0, 0, []))

    def test_dynamic_overtakes_floor_from_two_depts(self):
        # 종전 의도 보존: 2부서 × 5좌석 = 10 → 22 > floor 18 (구 '부서당 +5' 와 같은 값이 나오는 형상).
        m = self._measure(roster(5, 5))
        self.assertEqual(m["nodes_hard_effective"], G.NODES_HARD_BASE + 10)  # 22 > floor 18
        self.assertEqual(m["active_depts"], 2)
        self.assertEqual(m["dept_seats"], 10)
        self.assertEqual(m["depts"], [{"name": "d1", "seats": 5}, {"name": "d2", "seats": 5}])

    def test_seat_sum_not_dept_count_drives_hard(self):
        # 같은 좌석 합 10 이면 부서 수(1 vs 2)와 무관하게 22 — 좌석 합산 규칙의 핵심(DESIGN A3: 4+6=10 → 22).
        self.assertEqual(self._measure(roster(10))["nodes_hard_effective"], 22)
        self.assertEqual(self._measure(roster(4, 6))["nodes_hard_effective"], 22)

    def test_real_roster_dept1_nine_seats(self):
        # 실데이터(SURVEY A4 · evidence G3-dept1-status-raw.json): dept-1 비-exited 9좌석 → 21 = 12+9.
        # 구 규칙은 1부서 → max(18, 12+5) = 18 이라 본부 5 + 부서 9 = 14~17 이 hard 턱밑이었다(CSO 22:05).
        self.assertEqual(self._measure(roster(9))["nodes_hard_effective"], 21)

    def test_small_seats_keep_floor_negative_control(self):
        # 음성 대조: 2부서 × 2좌석 = 4 → 12+4 = 16 < 18 → floor 18. 구 '+5/부서' 규칙이면 22 가 나와야
        # 했다 — 이 핀이 22 로 바뀌면 부서당 가산이 되살아난 것이다.
        self.assertEqual(self._measure(roster(2, 2))["nodes_hard_effective"], G.NODES_HARD_DEFAULT)

    def test_explicit_nodes_hard_wins_over_dynamic(self):
        # 테스트 주입 등 명시 지정(기본값과 다름)이면 동적 계산 생략 — 그 값 그대로.
        self.assertEqual(self._measure(roster(25), nodes_hard=7)["nodes_hard_effective"], 7)

    def test_roster_errors_join_measure_errors_and_are_not_counted(self):
        m = self._measure(roster(errors=["dept(x)"]))
        self.assertIn("dept(x)", m["measure_errors"])
        self.assertEqual(m["nodes_hard_effective"], G.NODES_HARD_DEFAULT)  # 실패 부서는 미계상
        self.assertEqual((m["active_depts"], m["dept_seats"]), (0, 0))


class TestEvaluateConsumesDynamicHard(unittest.TestCase):
    def _eval(self, nodes, r):
        a = make_args(servers_override=0, nodes_override=nodes, load_override=0.0)
        with mock.patch.object(G, "_dept_roster", return_value=r):
            m = G.measure(a)
        return G.evaluate(m, a)

    def test_old_static_hard_value_is_now_soft(self):
        # 구 임계(12)라면 hard_block 오탐이던 값이, 동적 임계(2부서×5좌석 → 22)에서는 soft.
        worst, checks = self._eval(nodes=13, r=roster(5, 5))
        self.assertEqual(worst, "soft")
        node_check = next(c for c in checks if c["metric"] == "nodes")
        self.assertEqual(node_check["hard"], 22)

    def test_hard_still_trips_beyond_dynamic_ceiling(self):
        worst, _ = self._eval(nodes=22, r=roster(5, 5))
        self.assertEqual(worst, "hard")

    def test_survey_sweep_two_depts_nine_seats_no_longer_hard(self):
        # SURVEY B6-2 좌석 스윕 행: 본부 5 + 2부서×9 = 23 노드 · 구 임계 max(18, 12+2×5)=22 → hard 오탐.
        # 신 규칙: 12 + 18 = 30 → 23 < 30 → soft. 음성 대조: 30 이상이면 여전히 hard(상한은 살아 있다).
        worst, checks = self._eval(nodes=23, r=roster(9, 9))
        self.assertEqual(worst, "soft")
        self.assertEqual(next(c for c in checks if c["metric"] == "nodes")["hard"], 30)
        self.assertEqual(self._eval(nodes=30, r=roster(9, 9))[0], "hard")

    def test_measure_errors_escalate_to_soft(self):
        # P-ORCH-1: 측정 실패는 조용한 allow 금지 — 최소 soft 격상 유지 회귀 핀.
        a = make_args(load_override=0.0)
        with mock.patch.object(G, "_dept_roster", return_value=roster()), \
                mock.patch.object(G, "_ps_lines", return_value=None):
            m = G.measure(a)
        self.assertIn("nodes(ps)", m["measure_errors"])
        worst, _ = G.evaluate(m, a)
        self.assertEqual(worst, "soft")

    def test_dept_response_failure_escalates_to_soft(self):
        # A3: 부서 데몬 무응답은 '조용한 0 좌석' 이 아니라 soft — 트립이 아니라 측정 실패 격상이라
        # checks 는 전부 ok. 음성 대조: 같은 형상에서 errors 만 비우면 ok.
        worst, checks = self._eval(nodes=0, r=roster(errors=["dept(stale)"]))
        self.assertEqual(worst, "soft")
        self.assertTrue(all(c["level"] == "ok" for c in checks))
        self.assertEqual(self._eval(nodes=0, r=roster())[0], "ok")


def _cp(argv, rc, stdout=""):
    return subprocess.CompletedProcess(argv, rc, stdout=stdout, stderr="")


class TestDeptRosterLive(unittest.TestCase):
    """③ A3 라이브 경로 — glob·subprocess 대역으로 데몬 응답/실패 형상을 밀폐 재현(실소켓 무접촉)."""

    SOCKS = {
        "ok": "/h/.local/state/cys-dept-dept-1/cys.sock",
        "rc1": "/h/.local/state/cys-dept-rc1/cys.sock",
        "timeout": "/h/.local/state/cys-dept-slow/cys.sock",
        "nocys": "/h/.local/state/cys-dept-nocys/cys.sock",
        "badjson": "/h/.local/state/cys-dept-badjson/cys.sock",
        "nosurf": "/h/.local/state/cys-dept-nosurf/cys.sock",
    }
    OK_JSON = json.dumps({"surfaces": [
        {"role": "master", "agent_alive": True, "exited": False},
        {"role": "worker", "agent_alive": True, "exited": False},
        {"role": "worker-2", "agent_alive": False, "exited": False},  # agent 죽어도 좌석 점유 → 계수(PREP #19)
        {"role": "worker-3", "agent_alive": False, "exited": True},   # exited → 제외
    ]})

    def _run_side_effect(self, argv, **kw):
        self.calls.append((argv, kw))
        sock = argv[-1]
        if sock == self.SOCKS["ok"]:
            return _cp(argv, 0, self.OK_JSON)
        if sock == self.SOCKS["rc1"]:
            return _cp(argv, 1)
        if sock == self.SOCKS["timeout"]:
            raise subprocess.TimeoutExpired(argv, kw.get("timeout"))
        if sock == self.SOCKS["nocys"]:
            raise FileNotFoundError("cys")
        if sock == self.SOCKS["badjson"]:
            return _cp(argv, 0, "not json")
        if sock == self.SOCKS["nosurf"]:
            return _cp(argv, 0, json.dumps({"daemon": {}}))
        raise AssertionError("unexpected sock %r" % sock)

    def _roster_for(self, *keys):
        self.calls = []
        socks = [self.SOCKS[k] for k in keys]
        with mock.patch.object(G.glob, "glob", return_value=socks), \
                mock.patch.object(G.subprocess, "run", side_effect=self._run_side_effect):
            return G._dept_roster()

    def test_rc0_json_counts_non_exited_seats(self):
        r = self._roster_for("ok")
        self.assertEqual(r, {"active": 1, "seats": 3, "errors": [],
                             "depts": [{"name": "dept-1", "seats": 3}]})

    def test_argv_shape_is_existing_surface_list_no_shell(self):
        # 데몬 신규 표면 0 · Windows/포터블 호환 계약: list argv · shell 미사용 · timeout 상수.
        self._roster_for("ok")
        argv, kw = self.calls[0]
        self.assertEqual(argv, ["cys", "status", "--json", "--socket", self.SOCKS["ok"]])
        self.assertIsNot(kw.get("shell"), True)
        self.assertEqual(kw.get("timeout"), G.DEPT_STATUS_TIMEOUT)

    def test_rc1_is_error_not_counted(self):
        r = self._roster_for("rc1")
        self.assertEqual(r, {"active": 0, "seats": 0, "errors": ["dept(rc1)"], "depts": []})

    def test_timeout_is_error(self):
        r = self._roster_for("timeout")
        self.assertEqual(r["errors"], ["dept(slow)"])
        self.assertEqual((r["active"], r["seats"]), (0, 0))

    def test_cys_missing_is_error(self):
        r = self._roster_for("nocys")
        self.assertEqual(r["errors"], ["dept(nocys)"])
        self.assertEqual((r["active"], r["seats"]), (0, 0))

    def test_bad_json_is_error(self):
        r = self._roster_for("badjson")
        self.assertEqual(r["errors"], ["dept(badjson)"])

    def test_json_without_surfaces_is_error(self):
        r = self._roster_for("nosurf")
        self.assertEqual(r["errors"], ["dept(nosurf)"])

    def test_mixed_two_sockets_partial_success(self):
        # 1 응답 + 1 실패: 응답 부서만 활성/좌석 계상, 실패는 errors — 부분 실패가 전체를 0 으로 접지 않는다.
        r = self._roster_for("rc1", "ok")
        self.assertEqual(r["active"], 1)
        self.assertEqual(r["seats"], 3)
        self.assertEqual(r["errors"], ["dept(rc1)"])
        self.assertEqual(r["depts"], [{"name": "dept-1", "seats": 3}])
        self.assertEqual(len(self.calls), 2)

    def test_no_sockets_means_zero_depts_no_calls(self):
        # Windows(named pipe = 파일 아님) 및 부서 없는 머신: glob 무매치 → 0 · subprocess 0회(종전 동일).
        r = self._roster_for()
        self.assertEqual(r, {"active": 0, "seats": 0, "errors": [], "depts": []})
        self.assertEqual(self.calls, [])

    def test_override_short_circuits_live_lookup(self):
        with mock.patch.object(G.glob, "glob", return_value=[self.SOCKS["ok"]]), \
                mock.patch.object(G.subprocess, "run", side_effect=AssertionError("live call")) as run:
            r = G._dept_roster({"active": 1, "seats": 9, "errors": [],
                                "depts": [{"name": "dept-1", "seats": 9}]})
        self.assertEqual((r["active"], r["seats"]), (1, 9))
        run.assert_not_called()

    def test_override_partial_dict_normalized(self):
        with mock.patch.object(G.subprocess, "run", side_effect=AssertionError("live call")):
            self.assertEqual(G._dept_roster({"seats": 10}),
                             {"active": 0, "seats": 10, "errors": [], "depts": []})

    def test_active_dept_count_compat_wrapper_counts_socket_files(self):
        # 호환 래퍼는 종전 정의(소켓 파일 수) 그대로 — measure 는 더 이상 이 값을 쓰지 않는다.
        with mock.patch.object(G.glob, "glob", return_value=[self.SOCKS["ok"], self.SOCKS["rc1"]]):
            self.assertEqual(G._active_dept_count(), 2)

    def test_measure_end_to_end_with_live_double(self):
        # measure() → _dept_roster() 실경로(대역): dept-1 3좌석 → hard 18 유지(12+3<18) · rc1 실패 → soft.
        a = make_args(servers_override=0, nodes_override=0, load_override=0.0)
        self.calls = []
        with mock.patch.object(G.glob, "glob", return_value=[self.SOCKS["ok"], self.SOCKS["rc1"]]), \
                mock.patch.object(G.subprocess, "run", side_effect=self._run_side_effect):
            m = G.measure(a)
        self.assertEqual(m["active_depts"], 1)
        self.assertEqual(m["dept_seats"], 3)
        self.assertEqual(m["depts"], [{"name": "dept-1", "seats": 3}])
        self.assertEqual(m["measure_errors"], ["dept(rc1)"])
        self.assertEqual(m["nodes_hard_effective"], G.NODES_HARD_DEFAULT)
        self.assertEqual(G.evaluate(m, a)[0], "soft")


class TestClaudeArgvForms(unittest.TestCase):
    """④ A1 픽스처(SURVEY A5 표 형태1/형태3 · DESIGN A1): claude argv 2형태의 NODE_PATTERNS 계수."""

    def test_native_path_claude_with_flag_counted(self):
        # 형태1: 절대경로 argv0 + 플래그 — `claude(\s|$)` 가 'claude ' 에 매칭 → 계수 1.
        lines = ["  301 /Users/x/.local/bin/claude --flag"]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 1)

    def test_versioned_binary_path_not_counted_characterization(self):
        # ★특성화(characterization) 핀 — 의도 선언이 아니라 현행 동작의 기록.
        # 형태3: argv0 이 버전 경로(`.../claude/versions/2.1.259 -p`) — 'claude' 뒤가 '/' 라
        # 현행 NODE_PATTERNS[0] `claude(\s|$)` 에 매칭되지 않아 **계수 0** 이다.
        # SURVEY F-a 4: 이 설치 형태는 실물 부재로 라이브 판정 불능(측정불능) · SURVEY B6-2/DESIGN A1:
        # NODE_PATTERNS 는 이번 라운드 **무수정 결정**(라이브 과대계수 0 실측 · 패턴 확장은 오너 결정).
        # 장래 패턴을 바꿔 이 형태를 계수하게 되면 이 핀을 **의도적으로** 1 로 갱신해야 한다(무언 변경 차단).
        lines = ["  302 /Users/x/.local/share/claude/versions/2.1.259 -p"]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 0)

    def test_bare_claude_positive_control(self):
        lines = ["  303 claude --dangerously-skip-permissions"]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 1)

    def test_three_forms_together(self):
        # 형태1 + 형태3 + 맨 claude → 2 (형태3 만 빠짐 — 위 특성화 핀과 정합).
        lines = ["  301 /Users/x/.local/bin/claude --flag",
                 "  302 /Users/x/.local/share/claude/versions/2.1.259 -p",
                 "  303 claude --dangerously-skip-permissions"]
        self.assertEqual(G._count_matching(lines, G.NODE_PATTERNS, G.NODE_EXCLUDE_PATTERNS), 2)


if __name__ == "__main__":
    unittest.main()
