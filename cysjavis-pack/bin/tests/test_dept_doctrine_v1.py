#!/usr/bin/env python3
"""test_dept_doctrine_v1.py — 증분2 '부서 교리 기계'(D1 옵션 1') 통합(subprocess) 회귀.

순수 로직(티켓 TTL·자원 게이트 결정)은 javis_bootstrap `--self-test`가 밀폐 검증한다. 이 파일은
프로세스 경계가 필요한 부트 흐름을 핀한다(모듈 전역이 import 시 HOME/PACK로 고정):
  (h) 부서 레인 + 티켓 부재 → 팀 기동(④) 생략·exit 0·단독 각성 메시지(cys boot 미호출).
  (i) 부서 레인 + 유효 티켓 + 결손>0 → 팀 기동 경로 진입(cys boot 관측)·티켓 .used 소비.
  (j) 결손 0(구성 충족 재선언) → 자원 게이트·cys boot 미호출(스폰 없음)·티켓 미소비.
  (j2) 반쪽 팀(reviewer만 4 — 구 총수 비교의 오판 케이스) → 결손 판정·팀 기동 진입(네거티브).
  (k) 자원 게이트 hard(servers) 목 → 팀 기동 0·exit 9·escalation 흔적(cys boot 미호출).
  (l) issue-ticket base 레인 전용 가드(부서 레인 거부).
  (m) javis_dept_migrate 멱등(2회 --fix 동일 결과·백업 생성).
  (n) 자원 게이트 soft → 디바운스 없음: 2연속 부트 모두 경고 경로 진입·레인별 상태파일 0.
  (o) javis_dept_migrate 스테일 교리 백필 — §3 픽스처 dry-run 감지 → --fix 후 §3 부재·
      ④-c 정합·백업 존재·멱등(2회차 무변화).

관측 기법(증분1 test_lane_isolation_v1 계승): 격리 HOME + PATH 앞 목 cys(호출 로깅) + 목 팩
(mock orchestra·resource_gate)로 실행. exit code·boot-last.json·목 로그로 판정. 라이브 데몬 무접촉.
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
BOOTSTRAP = os.path.join(BIN, "javis_bootstrap.py")
MIGRATE = os.path.join(BIN, "javis_dept_migrate.py")


def _write_exec(path, content):
    with open(path, "w") as f:
        f.write(content)
    os.chmod(path, 0o755)


def _mock_cys(mockbin, cys_log, list_file):
    """목 cys — 모든 서브명령 exit 0(데몬 생존 재현) + 호출 args를 cys_log에 append.
    list는 list_file(존재 시)을 cat(결손 산출 제어 — cys list 라이브 노드 수), 그 외는 무출력."""
    _write_exec(os.path.join(mockbin, "cys"),
                '#!/bin/bash\n'
                'echo "$@" >> "%s" 2>/dev/null\n'
                'case "$1" in\n'
                '  surface-role) echo ""; exit 0;;\n'
                '  list) [ -f "%s" ] && cat "%s"; exit 0;;\n'
                '  *) exit 0;;\n'
                'esac\n' % (cys_log, list_file, list_file))


def _node_list(n):
    """cys list 형식(ref\\trole=..\\tpid=..\\texited=false\\t..) 노드 surface n줄 생성(결손 0 재현)."""
    roles = ["cso", "worker", "reviewer-gemini", "reviewer-codex", "reviewer-grok"]
    return "".join("surface:%d\trole=%s\tpid=%d\texited=false\t\t\n"
                   % (i, roles[i % len(roles)], 100 + i) for i in range(n))


def _mock_pack(pack, orch_check_exit=1, gate_exit=0, gate_json="",
               orch_log=None, gate_log=None):
    """목 팩(bin/javis_orchestra.py·javis_resource_gate.py) — exit·출력을 인자로 고정."""
    binp = os.path.join(pack, "bin")
    os.makedirs(binp, exist_ok=True)
    _write_exec(os.path.join(binp, "javis_orchestra.py"),
                '#!/usr/bin/env python3\n'
                'import os,sys\n'
                'cmd = sys.argv[1] if len(sys.argv)>1 else ""\n'
                'log = %r\n'
                'if log: open(log,"a").write(cmd+"\\n")\n'
                'if cmd=="check": sys.exit(%d)\n'
                'sys.exit(0)\n' % (orch_log, orch_check_exit))
    _write_exec(os.path.join(binp, "javis_resource_gate.py"),
                '#!/usr/bin/env python3\n'
                'import sys\n'
                'log = %r\n'
                'if log: open(log,"a").write(" ".join(sys.argv[1:])+"\\n")\n'
                'out = %r\n'
                'if out: print(out)\n'
                'sys.exit(%d)\n' % (gate_log, gate_json, gate_exit))
    # ★agents.json 시드 필수(W0 P0 지혈 이후): 결손 판정이 ⑤check와 **같은 소스**
    # (javis_orchestra.effective_required_roles)에서 의무 역할 목록을 받으므로, 팩에 agents.json이
    # 없으면 리뷰어 감지가 전부 미설치로 떨어져 로스터가 Claude 대체(reviewer-claude-1/2)로 치환된다
    # — 실 레인에는 preflight가 agents.json을 시드하므로 그 상태가 정상이고, 시드 없는 목 팩은
    # 픽스처 결함이었다. cmd를 sys.executable(절대경로·실행가능 확정)로 두어 PATH·호스트 설치
    # 상태와 무관하게 **네이티브 로스터(reviewer-gemini·reviewer-codex)** 로 결정론 고정한다.
    with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
        json.dump({a: {"cmd": sys.executable} for a in ("claude", "gemini", "codex")}, f)


def _run_bootstrap(home, pack, mockbin, socket=None):
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_PACK_DIR"] = pack
    env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
    if socket is None:
        env.pop("CYS_SOCKET", None)
    else:
        env["CYS_SOCKET"] = socket
    # 부트 check 재시도 상한을 짧게(테스트 속도·⑤가 목 orchestra라 즉시 확정)
    env["CYS_BOOT_CHECK_RETRIES"] = "1"
    env["CYS_BOOT_CHECK_INTERVAL_S"] = "0"
    r = subprocess.run([sys.executable, BOOTSTRAP, "run"],
                       capture_output=True, text=True, timeout=60, env=env)
    bl = os.path.join(home, ".cys", "state", "boot-last.json")
    data = json.load(open(bl, encoding="utf-8")) if os.path.isfile(bl) else None
    return r.returncode, data, r.stderr


def _steps(data):
    return [s["step"] for s in (data or {}).get("steps", [])]


class DeptTicketGate(unittest.TestCase):
    def _setup(self, name="dept-1", orch_check_exit=0, gate_exit=0, gate_json="",
               ticket=None, live_nodes=0):
        home = tempfile.mkdtemp()
        pack = os.path.join(home, ".cys", "pack-dept-%s" % name)  # 레인↔팩 정합(부서)
        mockbin = tempfile.mkdtemp()
        self.cys_log = os.path.join(home, "cys-calls.log")
        self.gate_log = os.path.join(home, "gate-calls.log")
        self.list_file = os.path.join(home, "cys-list.txt")
        with open(self.list_file, "w") as f:
            f.write(_node_list(live_nodes))   # live_nodes>=4 → 결손 0
        _mock_cys(mockbin, self.cys_log, self.list_file)
        _mock_pack(pack, orch_check_exit=orch_check_exit, gate_exit=gate_exit,
                   gate_json=gate_json, gate_log=self.gate_log)
        if ticket is not None:
            tdir = os.path.join(home, ".cys", "state", "dept-boot-tickets")
            os.makedirs(tdir, exist_ok=True)
            with open(os.path.join(tdir, "%s.ticket" % name), "w") as f:
                json.dump(ticket, f)
        sock = "%s/.local/state/cys-dept-%s/cys.sock" % (home, name)
        self.home, self.pack, self.mockbin, self.sock, self.name = home, pack, mockbin, sock, name
        return home, pack, mockbin, sock

    def _cys_called(self, sub):
        if not os.path.isfile(self.cys_log):
            return False
        return any(line.split()[0:1] == [sub] for line in open(self.cys_log))

    # (h) 부서 레인 + 티켓 부재 → 단독 각성(팀 기동 생략·exit 0)
    def test_h_dept_no_ticket_solo_awakening(self):
        home, pack, mockbin, sock = self._setup(ticket=None)
        rc, data, _ = _run_bootstrap(home, pack, mockbin, socket=sock)
        self.assertEqual(rc, 0, "티켓 부재 부서 레인인데 exit≠0(단독 각성 강등 실패)")
        self.assertIn("③″ceo-ticket", _steps(data))
        self.assertNotIn("④boot", _steps(data), "티켓 부재인데 팀 기동 단계 진입")
        self.assertTrue((data.get("result") or {}).get("solo_awakening"),
                        "solo_awakening 미표기")
        self.assertFalse(self._cys_called("boot"), "티켓 부재인데 cys boot 호출됨")

    # (i) 부서 레인 + 유효 티켓 + 결손>0 → 팀 기동 진입·티켓 소비
    def test_i_dept_valid_ticket_boots_and_consumes(self):
        ticket = {"dept": "dept-1", "issued_at": time.time(), "issuer": "base-master"}
        # live_nodes=0 → 결손>0 → 게이트 발동(gate_exit=0 allow) → ④boot → 티켓 소비(성공 직후·
        # R2-LOW-C "1회성 티켓 ⟺ 실스폰") → ⑤(orch 0) 통과.
        home, pack, mockbin, sock = self._setup(orch_check_exit=0, gate_exit=0,
                                                ticket=ticket, live_nodes=0)
        rc, data, _ = _run_bootstrap(home, pack, mockbin, socket=sock)
        self.assertIn("③″ceo-ticket-consume", _steps(data), "티켓 소비 단계 부재(팀 기동 미진입)")
        self.assertTrue(self._cys_called("boot"), "유효 티켓인데 cys boot 미호출(팀 기동 미진입)")
        tdir = os.path.join(home, ".cys", "state", "dept-boot-tickets")
        self.assertFalse(os.path.exists(os.path.join(tdir, "dept-1.ticket")), "티켓 미소비(잔존)")
        self.assertTrue(os.path.exists(os.path.join(tdir, "dept-1.ticket.used")), ".used 미생성")

    # (j) 결손 0(구성 충족 재선언) → 자원 게이트 미호출·cys boot 미호출(스폰 없음)·티켓 미소비
    def test_j_no_deficit_skips_gate_and_keeps_ticket(self):
        ticket = {"dept": "dept-1", "issued_at": time.time(), "issuer": "base-master"}
        # live_nodes=4 → roles 순환이 cso·worker·reviewer-gemini·reviewer-codex → 구성 충족(결손 0)
        # → 게이트 생략 + ④ cys boot 호출 생략(R1-MED-1 — "결손 0=스폰 없음" 결정론화)·티켓 미소비.
        home, pack, mockbin, sock = self._setup(orch_check_exit=0, ticket=ticket, live_nodes=4)
        rc, data, _ = _run_bootstrap(home, pack, mockbin, socket=sock)
        self.assertEqual(rc, 0, "결손 0 재선언인데 exit≠0")
        self.assertFalse(os.path.isfile(self.gate_log), "결손 0인데 자원 게이트 호출됨(오탐 위험)")
        gate_step = [s for s in (data or {}).get("steps", []) if s["step"] == "④′resource-gate"]
        self.assertTrue(gate_step and "결손 0" in gate_step[0]["detail"], "게이트 생략 흔적 부재")
        self.assertFalse(self._cys_called("boot"),
                         "결손 0인데 cys boot 호출됨(스폰 경로 진입 — 구동작 잔재)")
        self.assertIn("④boot-skip", _steps(data), "④ 생략 흔적(④boot-skip 단계) 부재")
        self.assertNotIn("④boot", _steps(data), "결손 0인데 ④boot 단계 기록")
        tdir = os.path.join(home, ".cys", "state", "dept-boot-tickets")
        self.assertTrue(os.path.exists(os.path.join(tdir, "dept-1.ticket")),
                        "결손 0 재선언인데 티켓 소비됨(재사용 불가)")
        self.assertNotIn("③″ceo-ticket-consume", _steps(data), "결손 0인데 티켓 소비 단계 발생")

    # (j2) ★네거티브(R1-MED-1 원 결함): reviewer만 4(총수 4) — 구 총수 비교는 결손 0으로 오판해
    # cso/worker 사망을 방치했다. 신 구성 판정은 결손으로 보고 팀 기동 경로에 진입해야 한다.
    def test_j2_half_team_reviewers_only_is_deficit(self):
        ticket = {"dept": "dept-1", "issued_at": time.time(), "issuer": "base-master"}
        home, pack, mockbin, sock = self._setup(orch_check_exit=0, gate_exit=0,
                                                ticket=ticket, live_nodes=0)
        with open(self.list_file, "w") as f:
            f.write("".join("surface:%d\trole=reviewer-claude-%d\tpid=%d\texited=false\t\t\n"
                            % (i, i, 100 + i) for i in range(4)))
        rc, data, _ = _run_bootstrap(home, pack, mockbin, socket=sock)
        self.assertEqual(rc, 0, "반쪽 팀 부트인데 exit≠0")
        self.assertTrue(os.path.isfile(self.gate_log),
                        "반쪽 팀(reviewer만 4)인데 자원 게이트 미호출(총수 비교 오판 잔재)")
        self.assertTrue(self._cys_called("boot"), "반쪽 팀인데 cys boot 미호출(결손 미판정)")
        self.assertIn("③″ceo-ticket-consume", _steps(data), "실스폰인데 티켓 미소비")

    # (k) 자원 게이트 hard(servers) → 팀 기동 0·exit 9·escalation
    def test_k_resource_hard_blocks_boot_exit9(self):
        gate_json = json.dumps({"verdict": "hard_block",
                                "trips": [{"metric": "servers", "level": "hard", "value": 5}],
                                "measured": {"nodes_hard_effective": 18}})
        # base 레인으로 격리(티켓 게이트 무관) — 결손>0(라이브 노드 0) → 게이트 hard(gate exit 2)
        home = tempfile.mkdtemp()
        pack = os.path.join(home, ".cys", "pack")  # base 레인 ↔ 메인 팩(정합)
        mockbin = tempfile.mkdtemp()
        self.cys_log = os.path.join(home, "cys-calls.log")
        self.gate_log = os.path.join(home, "gate-calls.log")
        list_file = os.path.join(home, "cys-list.txt")
        open(list_file, "w").close()   # 빈 목록 → 라이브 노드 0 → 결손>0
        _mock_cys(mockbin, self.cys_log, list_file)
        _mock_pack(pack, orch_check_exit=1, gate_exit=2, gate_json=gate_json,
                   gate_log=self.gate_log)
        rc, data, _ = _run_bootstrap(home, pack, mockbin, socket=None)  # base
        self.assertEqual(rc, 9, "자원 hard인데 exit≠9(hard-block 미발동)")
        self.assertEqual((data.get("result") or {}).get("failed_step"), "resource-gate")
        self.assertNotIn("④boot", _steps(data), "hard-block 후에도 팀 기동으로 진행")
        self.assertIn("④′resource-gate-notify", _steps(data), "CEO escalation 알림 흔적 부재")
        self.assertFalse(any(line.split()[0:1] == ["boot"] for line in open(self.cys_log)),
                         "hard-block인데 cys boot 호출됨")


class SoftGateNoDebounce(unittest.TestCase):
    # (n) 자원 게이트 soft → 디바운스 없음: 2연속 부트 모두 경고 경로 진입·레인별 상태파일 0
    def test_n_soft_warns_every_run_no_state_file(self):
        # base 레인 · 결손>0(빈 목록) · gate exit 1(soft) → 매 부트 경고 후 진행(exit 0).
        home = tempfile.mkdtemp()
        pack = os.path.join(home, ".cys", "pack")  # base 레인 ↔ 메인 팩(정합)
        mockbin = tempfile.mkdtemp()
        cys_log = os.path.join(home, "cys-calls.log")
        list_file = os.path.join(home, "cys-list.txt")
        open(list_file, "w").close()   # 빈 목록 → 라이브 노드 0 → 결손>0 → 게이트 발동
        _mock_cys(mockbin, cys_log, list_file)
        _mock_pack(pack, orch_check_exit=0, gate_exit=1)
        for run in (1, 2):
            rc, data, stderr = _run_bootstrap(home, pack, mockbin, socket=None)
            self.assertEqual(rc, 0, "%d차: soft인데 부트 미진행(exit=%d)" % (run, rc))
            self.assertIn("soft_warn", stderr,
                          "%d차: soft 경고 경로 미진입(디바운스 잔재 의심)" % run)
            gate_steps = [s for s in (data or {}).get("steps", [])
                          if s["step"] == "④′resource-gate"]
            self.assertTrue(gate_steps and "verdict=soft" in gate_steps[0]["detail"],
                            "%d차: 게이트 soft 판정 흔적 부재" % run)
        state_dir = os.path.join(home, ".cys", "state")
        leftovers = [f for f in os.listdir(state_dir) if f.startswith("resource-gate-level-")]
        self.assertEqual(leftovers, [], "레인별 디바운스 상태파일 잔재: %s" % leftovers)


class IssueTicketGuard(unittest.TestCase):
    # (l) issue-ticket base 전용 가드
    def test_l_issue_ticket_base_only(self):
        home = tempfile.mkdtemp()
        env = dict(os.environ)
        env["HOME"] = home
        # base 레인: 발급 성공
        env.pop("CYS_SOCKET", None)
        r = subprocess.run([sys.executable, BOOTSTRAP, "issue-ticket", "--dept", "dept-1"],
                           capture_output=True, text=True, timeout=30, env=env)
        self.assertEqual(r.returncode, 0, "base 레인 발급 실패:\n%s" % r.stderr)
        tpath = os.path.join(home, ".cys", "state", "dept-boot-tickets", "dept-1.ticket")
        self.assertTrue(os.path.isfile(tpath), "티켓 파일 미생성")
        # 부서 레인: 발급 거부
        env["CYS_SOCKET"] = "%s/.local/state/cys-dept-dept-2/cys.sock" % home
        r2 = subprocess.run([sys.executable, BOOTSTRAP, "issue-ticket", "--dept", "dept-2"],
                            capture_output=True, text=True, timeout=30, env=env)
        self.assertEqual(r2.returncode, 2, "부서 레인 발급이 거부되지 않음")
        self.assertFalse(os.path.exists(os.path.join(
            home, ".cys", "state", "dept-boot-tickets", "dept-2.ticket")), "거부인데 티켓 생성됨")


class MigrateIdempotent(unittest.TestCase):
    # (m) javis_dept_migrate 멱등(2회 --fix 동일 결과·백업 생성)
    def test_m_migrate_idempotent(self):
        home = tempfile.mkdtemp()
        # 메인 팩(소스) — 훅·부트스트랩 실체
        main_pack = os.path.join(home, ".cys", "pack")
        os.makedirs(os.path.join(main_pack, "hooks"), exist_ok=True)
        os.makedirs(os.path.join(main_pack, "bin"), exist_ok=True)
        _write_exec(os.path.join(main_pack, "hooks", "role-bootstrap.sh"), "#!/bin/bash\nexit 0\n")
        _write_exec(os.path.join(main_pack, "bin", "javis_bootstrap.py"), "#!/usr/bin/env python3\n")
        # 기존 부서 config(UserPromptSubmit 부재 — 풀 세트 중 SessionStart만 있는 상태 재현)
        acctdir = os.path.join(home, ".cys", "claude-default-dept-1")
        os.makedirs(acctdir, exist_ok=True)
        settings = os.path.join(acctdir, "settings.json")
        with open(settings, "w") as f:
            json.dump({"hooks": {"SessionStart": [
                {"hooks": [{"type": "command", "command": "sh /x/hooks/session-start.sh"}]}]}}, f)
        # 부서 팩(훅 부재 — 복사 대상)
        dept_pack = os.path.join(home, ".cys", "pack-dept-dept-1")
        os.makedirs(dept_pack, exist_ok=True)

        env = dict(os.environ)
        env["HOME"] = home
        env["CYS_PACK_DIR"] = main_pack

        def run_fix():
            return subprocess.run([sys.executable, MIGRATE, "--fix", "--json"],
                                  capture_output=True, text=True, timeout=30, env=env)

        r1 = run_fix()
        self.assertEqual(r1.returncode, 0, "1차 --fix 실패:\n%s" % r1.stderr)
        content1 = open(settings, encoding="utf-8").read()
        self.assertIn("UserPromptSubmit", content1, "UserPromptSubmit 미등록")
        self.assertIn("role-bootstrap.sh", content1, "role-bootstrap.sh 명령 미등록")
        self.assertTrue(os.path.isfile(settings + ".bak-migrate"), "백업 미생성")
        self.assertTrue(os.path.isfile(os.path.join(dept_pack, "hooks", "role-bootstrap.sh")),
                        "부서 팩에 훅 미복사")
        self.assertTrue(os.path.isfile(os.path.join(dept_pack, "bin", "javis_bootstrap.py")),
                        "부서 팩에 부트스트랩 미복사")

        r2 = run_fix()
        self.assertEqual(r2.returncode, 0, "2차 --fix 실패")
        content2 = open(settings, encoding="utf-8").read()
        self.assertEqual(content1, content2, "멱등 위반 — 2차 실행이 settings.json을 변경(중복 등록 등)")
        # 2차 dry-run: 모두 ok(이미 등록·존재)
        r3 = subprocess.run([sys.executable, MIGRATE, "--json"],
                            capture_output=True, text=True, timeout=30, env=env)
        rep = json.loads(r3.stdout)
        d = rep["depts"][0]
        self.assertEqual(d["hook"]["action"], "ok", "멱등 상태인데 hook action≠ok: %s" % d["hook"])
        self.assertTrue(all(pf["action"] == "ok" for pf in d["pack_files"]),
                        "멱등 상태인데 pack_files action≠ok")


STALE_DIRECTIVE = """# MASTER ABSOLUTE DIRECTIVE — 지휘 노드 절대지침

## [부서장 스코프 절대규칙] (2026-07-11 박사님 승인·dept-recovery §3)

- 너는 부서장(dept-master)이다 — 메인 CEO가 아니다.
- 내부 노드는 CEO의 명시적 작업배정 티켓이 있을 때만 기동한다. 각성 기본값=부서장 단독 대기.

> 너는 이 cys 터미널 워크스페이스의 **master**다. (④-c 이전 구본 본문)
"""

CURRENT_DIRECTIVE = """# MASTER ABSOLUTE DIRECTIVE — 지휘 노드 절대지침

> 너는 이 cys 터미널 워크스페이스의 **master**다. (현행 본문)
   ④-c **부서 레인 분기 (CEO 티켓 게이트 · D1 옵션 1')**: 팀 기동은 CEO 발급 티켓 +
   자원 게이트 통과 시에만 자동 수행된다. 단독 대기는 각성 기본값이 아니라 티켓 부재 시의 강등 상태다.
"""


class MigrateDirectiveBackfill(unittest.TestCase):
    # (o) 스테일 교리 백필 — dry-run 감지 → --fix 교체(§3 부재·④-c 정합·백업) → 멱등
    def _setup(self):
        home = tempfile.mkdtemp()
        main_pack = os.path.join(home, ".cys", "pack")
        os.makedirs(os.path.join(main_pack, "hooks"), exist_ok=True)
        os.makedirs(os.path.join(main_pack, "bin"), exist_ok=True)
        os.makedirs(os.path.join(main_pack, "directives"), exist_ok=True)
        _write_exec(os.path.join(main_pack, "hooks", "role-bootstrap.sh"), "#!/bin/bash\nexit 0\n")
        _write_exec(os.path.join(main_pack, "bin", "javis_bootstrap.py"), "#!/usr/bin/env python3\n")
        with open(os.path.join(main_pack, "directives", "MASTER_DIRECTIVE.md"), "w",
                  encoding="utf-8") as f:
            f.write(CURRENT_DIRECTIVE)
        os.makedirs(os.path.join(home, ".cys", "claude-default-dept-1"), exist_ok=True)
        dept_dir = os.path.join(home, ".cys", "pack-dept-dept-1", "directives")
        os.makedirs(dept_dir, exist_ok=True)
        self.dept_directive = os.path.join(dept_dir, "MASTER_DIRECTIVE.md")
        with open(self.dept_directive, "w", encoding="utf-8") as f:
            f.write(STALE_DIRECTIVE)
        env = dict(os.environ)
        env["HOME"] = home
        env["CYS_PACK_DIR"] = main_pack
        return home, env

    def _run(self, env, *args):
        r = subprocess.run([sys.executable, MIGRATE, "--json"] + list(args),
                           capture_output=True, text=True, timeout=30, env=env)
        self.assertEqual(r.returncode, 0, "migrate 실패(%s):\n%s%s" % (args, r.stdout, r.stderr))
        return json.loads(r.stdout)["depts"][0]["directive"]

    def test_o_stale_doctrine_backfill(self):
        home, env = self._setup()
        # ① dry-run: 스테일 감지·교체 예정(파일 무변화)
        d = self._run(env)
        self.assertEqual(d["action"], "would", "dry-run이 스테일 §3을 감지하지 못함: %s" % d)
        content = open(self.dept_directive, encoding="utf-8").read()
        self.assertIn("[부서장 스코프 절대규칙]", content, "dry-run이 파일을 변경(파괴)함")
        # ② --fix: §3 부재 + ④-c 정합 + 백업 존재
        d = self._run(env, "--fix")
        self.assertEqual(d["action"], "fixed", "--fix가 교체하지 않음: %s" % d)
        fixed1 = open(self.dept_directive, encoding="utf-8").read()
        self.assertNotIn("[부서장 스코프 절대규칙]", fixed1, "--fix 후에도 스테일 §3 잔존")
        self.assertIn("④-c", fixed1, "--fix 후 ④-c 분기 부재(현행 정합 실패)")
        self.assertEqual(fixed1, CURRENT_DIRECTIVE, "교체본이 메인 팩 소스와 불일치")
        backup = self.dept_directive + ".bak-migrate"
        self.assertTrue(os.path.isfile(backup), "백업 미생성")
        self.assertIn("[부서장 스코프 절대규칙]",
                      open(backup, encoding="utf-8").read(), "백업이 원본(스테일)을 보존하지 않음")
        # ③ 멱등: 2회차 --fix 무변화·ok
        d = self._run(env, "--fix")
        self.assertEqual(d["action"], "ok", "멱등 위반 — 2회차가 ok가 아님: %s" % d)
        fixed2 = open(self.dept_directive, encoding="utf-8").read()
        self.assertEqual(fixed1, fixed2, "멱등 위반 — 2회차 실행이 파일을 변경")

    def test_o2_symlink_refused(self):
        home, env = self._setup()
        os.unlink(self.dept_directive)
        os.symlink(os.path.join(home, "elsewhere.md"), self.dept_directive)
        d = self._run(env, "--fix")
        self.assertEqual(d["action"], "skip", "symlink가 거부되지 않음: %s" % d)
        self.assertTrue(os.path.islink(self.dept_directive), "symlink가 파괴됨")

    def test_o3_stale_main_source_refused(self):
        # 메인 팩 소스 자체가 ④-c 부재(구본)면 스테일로 스테일을 덮지 않도록 error 보류
        home, env = self._setup()
        with open(os.path.join(home, ".cys", "pack", "directives", "MASTER_DIRECTIVE.md"),
                  "w", encoding="utf-8") as f:
            f.write("# MASTER ABSOLUTE DIRECTIVE\n\n> 구본(현행 분기 부재)\n")
        r = subprocess.run([sys.executable, MIGRATE, "--json", "--fix"],
                           capture_output=True, text=True, timeout=30, env=env)
        self.assertEqual(r.returncode, 2, "구본 소스인데 exit≠2(오류 미보고)")
        d = json.loads(r.stdout)["depts"][0]["directive"]
        self.assertEqual(d["action"], "error", "구본 소스 교체가 보류되지 않음: %s" % d)
        content = open(self.dept_directive, encoding="utf-8").read()
        self.assertEqual(content, STALE_DIRECTIVE, "보류인데 파일이 변경됨")


if __name__ == "__main__":
    unittest.main()
