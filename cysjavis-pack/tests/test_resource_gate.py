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
import argparse
import os
import time
import tempfile
import socket
import shutil
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
        # ★A3-b 밀폐: measure() 는 servers 를 **원장**(`cys ps`)에서 먼저 읽는다. 기본값을
        #   '빈 원장' 으로 주입해 단위 테스트가 실행 기계의 라이브 데몬에 의존하지 않게 한다
        #   (라이브 의존 = 기계마다 다른 결과 = 결정론 파괴). 폴백 경로를 재는 테스트는
        #   _ledger_servers 를 명시 패치한다.
        servers_ledger_override={"lane": "(ledger empty)", "depts": {}},
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
        # ★A3-c 축 분리: 리스너 프로브(_socket_listening)는 **별도 축**이며 TestSocketProbe 가
        #   실소켓 픽스처로 잠근다. 여기서 재는 것은 '데몬 응답 → 좌석 계상' 이므로 프로브는
        #   통과로 고정한다(가짜 경로에 실소켓이 없어 프로브가 먼저 죽으면 이 축을 못 잰다).
        with mock.patch.object(G.glob, "glob", return_value=socks), \
                mock.patch.object(G, "_socket_listening", return_value=True), \
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
                mock.patch.object(G, "_socket_listening", return_value=True), \
                mock.patch.object(G.subprocess, "run", side_effect=self._run_side_effect):
            m = G.measure(a)
        self.assertEqual(m["active_depts"], 1)
        self.assertEqual(m["dept_seats"], 3)
        self.assertEqual(m["depts"], [{"name": "dept-1", "seats": 3}])
        self.assertEqual(m["measure_errors"], ["dept(rc1)"])
        self.assertEqual(m["nodes_hard_effective"], G.NODES_HARD_DEFAULT)
        self.assertEqual(G.evaluate(m, a)[0], "soft")



class TestSocketProbe(unittest.TestCase):
    """④ A3-c 리스너 프로브 — stale 소켓에서 5초를 태우지 않기 위한 축(실측 근거: 프로브 이전
    stale 소켓 1개당 게이트 왕복 5.11s → 이후 0.07s). **판정은 바뀌지 않고 시간만 줄어든다** —
    죽은 소켓은 종전에도 `dept(<이름>)` 오류로 계상됐다.

    ★설계 원칙: '확실한 죽음'만 False 다. 판정 불가(권한·미지원·그 밖의 OSError)는 True 로 접어
    종전 경로(`cys status` 왕복)로 보낸다 — 프로브가 판정기가 되면 안 된다(측정 불능은 통과가
    아니라 '종전 경로로 진행'이다)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="probe-")
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_plain_file_is_dead(self):
        # 잔재가 소켓이 아닌 일반 파일인 형상 — macOS 실측 errno 38(ENOTSOCK)
        p = os.path.join(self.tmp, "plain.sock")
        open(p, "w").close()
        self.assertFalse(G._socket_listening(p))

    def test_missing_path_is_dead(self):
        # glob 과 프로브 사이 레이스로 경로가 사라진 형상 — ENOENT
        self.assertFalse(G._socket_listening(os.path.join(self.tmp, "nope.sock")))

    def test_bound_but_not_listening_is_dead(self):
        # ★가장 현실적인 잔재: 데몬이 비정상 종료해 소켓 inode 만 남은 형상 — ECONNREFUSED
        p = os.path.join(self.tmp, "dead.sock")
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.bind(p)
        s.close()
        self.assertTrue(os.path.exists(p), "픽스처가 소켓 파일을 남기지 못했다")
        self.assertFalse(G._socket_listening(p))

    def test_listening_socket_is_alive(self):
        p = os.path.join(self.tmp, "live.sock")
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(p)
        srv.listen(1)
        self.addCleanup(srv.close)
        self.assertTrue(G._socket_listening(p))

    def test_unknown_oserror_is_not_a_verdict(self):
        # 권한 오류(EACCES) 등 '판정 불가'는 True — 종전 경로로 보내고 프로브가 결론내지 않는다.
        with mock.patch.object(G.socket, "socket",
                               side_effect=PermissionError(13, "Permission denied")):
            self.assertTrue(G._socket_listening("/whatever.sock"))

    def test_timeout_is_not_a_verdict(self):
        # 연결이 매달리는 형상(socket.timeout ⊂ OSError · errno 없음) → 판정 불가 → True
        class _S:
            def settimeout(self, _t):
                pass

            def connect(self, _p):
                raise socket.timeout("timed out")

            def close(self):
                pass
        with mock.patch.object(G.socket, "socket", return_value=_S()):
            self.assertTrue(G._socket_listening("/slow.sock"))

    def test_windows_skips_probe(self):
        # Windows 부서 소켓은 named pipe — AF_UNIX 프로브 대상이 아니다(분기 보존).
        with mock.patch.object(G.os, "name", "nt"):
            self.assertTrue(G._socket_listening("\\\\.\\pipe\\cys-dept-x"))

    def test_probe_timeout_constant_is_subsecond(self):
        # 프로브가 초 단위면 목적(5s 절감)을 잃는다 — 상한을 상수로 못박는다.
        self.assertLessEqual(G.SOCKET_PROBE_TIMEOUT, 1.0)

    def test_stale_socket_roster_is_fast_and_counted_as_error(self):
        """★A3-c 목적 자체의 핀: stale 소켓이 있어도 로스터 산출이 **1초 안에** 끝나고
        그 부서는 오류로 계상된다(판정 무변경 · 시간만 감소). 프로브 이전에는 여기서
        DEPT_STATUS_TIMEOUT(5s)이 통째로 소요됐다."""
        p = os.path.join(self.tmp, "stale.sock")
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.bind(p)
        s.close()
        calls = []

        def _never(argv, **kw):
            calls.append(argv)
            raise AssertionError("stale 소켓에 cys status 를 스폰했다(프로브 미발화)")

        t0 = time.monotonic()
        with mock.patch.object(G.glob, "glob", return_value=[p]), \
                mock.patch.object(G.subprocess, "run", side_effect=_never):
            roster = G._dept_roster()
        elapsed = time.monotonic() - t0
        self.assertEqual(roster["active"], 0)
        self.assertEqual(roster["seats"], 0)
        self.assertEqual(len(roster["errors"]), 1, roster)
        self.assertFalse(calls, "스폰 0 계약 위반")
        self.assertLess(elapsed, 1.0, "stale 소켓에서 %0.2fs 소요 — 프로브가 무효" % elapsed)



class TestServersLedger(unittest.TestCase):
    """⑤ A3-b servers 축 — 논리 서버는 **프로세스 원장**(`cys ps`) 항목 수다.

    근거(dept-1 실측 22:05 · impl/live-evidence/dept1-queue-starvation-2205.txt:56): 논리 서버
    1개가 래퍼 체인(`cys run -- npm exec vite` → `npm exec vite` → `node …/vite`) 때문에 ps
    패턴에서 3으로 세어져 servers hard(3)에 걸렸다 — 아무 서버도 새로 띄우지 않은 레인이
    '서버 누적' 판정으로 착수를 거부당했다. 임계는 그대로 두고(근본은 계수) 계수를 고친다."""

    LEDGER = ("pid=101\tpgid=101\tscoped=true\tsurface=3\tcys run -- npm exec vite\n"
              "pid=202\tpgid=202\tscoped=true\tsurface=4\tcys events --category queue --reconnect\n")

    def test_ledger_counts_only_server_entries(self):
        # 원장에 서버 1 + 비서버 1 → 1 (원장은 서버 전용이 아니다 — dept-1 실측 형상)
        n, errs = G._ledger_servers({"lane": self.LEDGER, "depts": {}})
        self.assertEqual((n, errs), (1, []))

    def test_empty_ledger_is_zero(self):
        self.assertEqual(G._ledger_servers({"lane": "(ledger empty)", "depts": {}}), (0, []))

    def test_dept_ledgers_are_summed_and_pid_deduped(self):
        dept = ("pid=101\tpgid=101\tscoped=true\tsurface=9\tcys run -- npm exec vite\n"
                "pid=303\tpgid=303\tscoped=true\tsurface=9\tnode /srv/server.js\n")
        n, errs = G._ledger_servers({"lane": self.LEDGER, "depts": {"/x/cys.sock": dept}})
        self.assertEqual((n, errs), (2, []), "pid 101 중복 계상 또는 부서 합산 누락")

    def test_chain_of_three_processes_is_one_logical_server(self):
        """★재현 픽스처: 같은 체인 3프로세스 → 패턴 폴백 계수 1. 음성 대조로 **구 계수식**
        (_count_matching)이 같은 입력에서 3 을 내는 것까지 확인한다 — 픽스처가 결함을
        실제로 재현한다는 증거가 없으면 이 테스트는 아무것도 증명하지 못한다."""
        lines = ["  101 cys run -- npm exec vite",
                 "  102 npm exec vite",
                 "  103 node /Users/x/proj/node_modules/.bin/vite --port 5173"]
        ppid = {101: 1, 102: 101, 103: 102}
        self.assertEqual(G._count_matching(lines, G.SERVER_PATTERNS, G.SERVER_EXCLUDE_PATTERNS), 3,
                         "음성 대조 실패 — 픽스처가 구 계수식에서 3 을 재현하지 못한다")
        with mock.patch.object(G, "_ppid_map", return_value=ppid):
            roots = G._server_procs(lines)
        self.assertEqual([p for p, _c in roots], [101], "체인 루트 접기 실패")

    def test_two_independent_servers_stay_two(self):
        # 접기가 서로 다른 트리까지 합치면 실서버 누적을 놓친다(반대 방향 회귀 차단).
        lines = ["  101 cys run -- npm exec vite", "  201 uvicorn app:main"]
        with mock.patch.object(G, "_ppid_map", return_value={101: 1, 201: 1}):
            roots = G._server_procs(lines)
        self.assertEqual(sorted(p for p, _c in roots), [101, 201])

    def test_ppid_unavailable_keeps_conservative_count(self):
        # 체인 판정 불가 → 접지 않는다(과대계수는 보수적 방향 — 조용한 과소계수 금지)
        lines = ["  101 cys run -- npm exec vite", "  102 npm exec vite"]
        with mock.patch.object(G, "_ppid_map", return_value=None):
            self.assertEqual(len(G._server_procs(lines)), 2)

    def test_lane_ledger_failure_falls_back_to_pattern_with_error(self):
        a = make_args(nodes_override=0, load_override=0.0, servers_ledger_override=None)
        lines = ["  101 cys run -- npm exec vite", "  102 npm exec vite"]
        with mock.patch.object(G, "_ledger_servers", return_value=(None, ["servers(ledger)"])), \
                mock.patch.object(G, "_ps_lines", return_value=lines), \
                mock.patch.object(G, "_ppid_map", return_value={101: 1, 102: 101}), \
                mock.patch.object(G, "_dept_roster", return_value={
                    "active": 0, "seats": 0, "errors": [], "depts": []}):
            m = G.measure(a)
        self.assertEqual(m["servers"], 1, "폴백에서도 체인은 1개로 세어야 한다")
        self.assertIn("servers(ledger)", m["measure_errors"])
        self.assertEqual(G.evaluate(m, a)[0], "soft", "원장 실패는 최소 soft 로 신호해야 한다")

    def test_dept_ledger_failure_is_partial_not_total(self):
        # 부서 원장 실패는 그 부서만 빠지고 전면 폴백이 아니다(오류만 남는다).
        n, errs = G._ledger_servers({"lane": self.LEDGER, "depts": {}})
        self.assertEqual(n, 1)
        with mock.patch.object(G.glob, "glob", return_value=["/h/.local/state/cys-dept-x/cys.sock"]), \
                mock.patch.object(G, "_socket_listening", return_value=False), \
                mock.patch.object(G.subprocess, "run",
                                  return_value=_cp(["cys", "ps"], 0, self.LEDGER)):
            n2, errs2 = G._ledger_servers()
        self.assertEqual(n2, 1, "현재 레인 원장은 그대로 세어야 한다")
        self.assertEqual(errs2, ["servers-ledger(x)"])

    def test_threshold_unchanged(self):
        """★임계 상향 금지(CEO 지시 — 근본은 계수다). **프로덕션 기본값**으로 잰다.

        주의: 이 파일의 `make_args` 대역은 servers_hard=4 로 프로덕션 argparse 기본값(3)과
        다르다(선행 코드의 대역 값 — 이 커밋에서 건드리지 않는다). 임계 계약은 대역이 아니라
        실제 CLI 기본값으로 확인해야 의미가 있으므로 main() 경로를 그대로 탄다."""
        import contextlib
        import io
        argv = ["check", "--json", "--nodes-override", "0", "--load-override", "0.0",
                "--dept-roster-override", '{"active":0,"seats":0,"errors":[],"depts":[]}',
                "--servers-ledger-override", '{"lane":"(ledger empty)","depts":{}}']
        with contextlib.redirect_stdout(io.StringIO()):
            rc_soft = G.main(argv + ["--servers-override", "2"])
            rc_hard = G.main(argv + ["--servers-override", "3"])
        self.assertEqual(rc_soft, G.EXIT_SOFT, "servers 2 가 soft 가 아니다 — 임계가 움직였다")
        self.assertEqual(rc_hard, G.EXIT_HARD, "servers 3 이 hard 가 아니다 — 임계가 움직였다")
        self.assertEqual(G.NODES_HARD_DEFAULT, 18)


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
