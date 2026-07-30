#!/usr/bin/env python3
"""test_lane_isolation_v1.py — 증분1 '레인 격리 3종 + 실패 가시화' 통합(subprocess) 회귀.

순수 판정(base/dept·락 키·레인↔팩)은 javis_bootstrap.py 내장 `--self-test`가 밀폐 검증한다.
이 파일은 프로세스 경계가 필요한 두 통합 경로를 핀한다(모듈 전역이 import 시 HOME/PACK로 고정돼
in-process로 격리 불가):
  (t3) 레인↔팩 불일치 가드 발동 — 부서 소켓 + 메인 팩 → 팀 기동 전 exit 8 + **레인** boot-last 흔적.
       ★W3(G15): 진단 SOT 는 레인별 파일이다(base=boot-last.json / 비-base=boot-last-<lane>.json) —
       전 레인 공유 단일 파일이던 종전에는 base·부서 동시 부트가 서로의 진단을 덮었다. 이 테스트는
       부서 레인을 재현하므로 **레인 파일**을 읽는다(경로 판정은 bootstrap 의 lane_state_path 소비).
  (t4) 훅 BOOT 부재 — 빈 팩으로 role-bootstrap.sh 실행 → 행 없이 exit 0 + additionalContext 실패 명시
       (데몬 없는 환경에서 feed push 실패가 훅을 죽이지 않음).
  (t5) 내장 self-test가 subprocess에서도 exit 0(무회귀 게이트).

관측 기법: 격리 HOME + PATH 앞 목 cys(모든 서브명령 실패 — 데몬 부재 재현)로 실행하고 exit code·
boot-last.json·stdout으로 판정. 노드 스폰·라이브 데몬 무접촉.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))          # cysjavis-pack/bin
BOOTSTRAP = os.path.join(BIN, "javis_bootstrap.py")
HOOK = os.path.join(os.path.dirname(BIN), "hooks", "role-bootstrap.sh")     # cysjavis-pack/hooks/...


def _mock_cys(mockbin, surface_role="", exit_code=1):
    """목 cys 설치 — surface-role은 주입값 반환, 그 외 서브명령은 exit_code(데몬 부재 재현)."""
    cysp = os.path.join(mockbin, "cys")
    with open(cysp, "w") as f:
        f.write('#!/bin/bash\n[ "$1" = surface-role ] && { echo "%s"; exit 0; }\nexit %d\n'
                % (surface_role, exit_code))
    os.chmod(cysp, 0o755)


class LanePackGuard(unittest.TestCase):
    """t3 — 부서 소켓 레인이 메인 팩을 쓰면(교차 오염·UT-14) 팀 기동 전 exit 8·시끄러운 실패."""

    def _run_bootstrap(self, sock, pack_dirname):
        home = tempfile.mkdtemp()
        mockbin = tempfile.mkdtemp()
        _mock_cys(mockbin)  # 모든 cys 실패 → _notify_loud 폴백까지 빠르게 소진(데몬 부재)
        pack = os.path.join(home, ".cys", pack_dirname)
        os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
        env = dict(os.environ)
        env["HOME"] = home
        env["CYS_PACK_DIR"] = pack
        env["CYS_SOCKET"] = sock
        env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
        r = subprocess.run([sys.executable, BOOTSTRAP, "run"],
                           capture_output=True, text=True, timeout=60, env=env)
        # ★W3(G15) 레인 스코프 경로 — 경로 규약은 피검체(javis_bootstrap.lane_state_path)에서
        #   파생시킨다(테스트가 규약 사본을 갖지 않는다: 사본은 드리프트의 원천이다).
        state = os.path.join(home, ".cys", "state")
        data = None
        for cand in sorted(os.listdir(state) if os.path.isdir(state) else []):
            if cand.startswith("boot-last") and cand.endswith(".json"):
                with open(os.path.join(state, cand), encoding="utf-8") as f:
                    data = json.load(f)
                break
        return r.returncode, data

    def test_dept_socket_main_pack_exits_8(self):
        # 부서 소켓 + 메인 팩(pack) → 불일치 → exit 8, 팀 기동(cys boot) 도달 전 중단
        rc, data = self._run_bootstrap(
            "/home/x/.local/state/cys-dept-dept-1/cys.sock", "pack")
        self.assertEqual(rc, 8, "부서 소켓+메인 팩인데 exit 8 아님(교차 오염 가드 미발동)")
        self.assertIsNotNone(data, "boot-last.json 미기록")
        self.assertEqual(data["result"]["failed_step"], "lane-pack")
        self.assertEqual(data["result"]["exit"], 8)
        steps = [s["step"] for s in data["steps"]]
        # ★W3(P3-A-STEP-NAME): 레인 가드는 ①preflight **앞**에서 도는데 라벨이 `③′` 였다(서수가
        #   실행순서를 거짓말) + 불량 레인/레인↔팩 두 단계가 **같은 라벨**을 공유했다(동명이의).
        #   라벨을 ⓪lane-pack / ⓪lane-malformed 로 분리했다 — 테스트 갱신은 수리의 일부다.
        self.assertIn("⓪lane-pack", steps, "가드 단계 미기록")
        self.assertIn("⓪lane-pack-notify", steps, "알림 시도 흔적 없음(push 시도 미기록)")
        # 팀 기동 단계(④boot)에 도달하지 않았어야 한다(중단 성공)
        self.assertNotIn("④boot", steps, "가드 이후에도 팀 기동으로 진행됨")

    def test_base_socket_dept_pack_exits_8(self):
        # base 소켓(미설정 아님·본부 소켓) + 부서 팩 → 역방향 불일치도 차단
        rc, data = self._run_bootstrap(
            "/home/x/.local/state/cys/cys.sock", "pack-dept-dept-2")
        self.assertEqual(rc, 8, "base 소켓+부서 팩인데 exit 8 아님")
        self.assertEqual(data["result"]["failed_step"], "lane-pack")

    def test_empty_dept_suffix_exits_8(self):
        # R1-LOW-2: 빈 부서명(cys-dept-/ — suffix 없음)은 base도 부서도 아닌 불량 레인 —
        # 조용한 통과(비-base·dept None으로 어느 게이트에도 안 걸림) 대신 명시 실패(exit 8).
        rc, data = self._run_bootstrap(
            "/home/x/.local/state/cys-dept-/cys.sock", "pack")
        self.assertEqual(rc, 8, "빈 부서명 불량 레인인데 exit 8 아님(침묵 통과)")
        self.assertIsNotNone(data, "boot-last.json 미기록")
        self.assertEqual(data["result"]["failed_step"], "lane-pack")
        steps = [s["step"] for s in data["steps"]]
        self.assertNotIn("④boot", steps, "불량 레인인데 팀 기동으로 진행됨")
        # 동명이의 해소 확인: 불량 레인은 레인↔팩 불일치와 **다른 단계**로 기록된다
        self.assertIn("⓪lane-malformed", steps, "불량 레인 전용 단계 미기록(동명이의 잔존)")
        self.assertNotIn("⓪lane-pack", steps, "불량 레인이 레인↔팩 라벨을 재사용(동명이의 재발)")

    def test_matched_lane_passes_guard(self):
        # dept-1 소켓 + pack-dept-dept-1 팩 → 정합 → 가드 통과(이후 ②ping에서 목 cys 실패로 exit 3).
        # 가드가 통과했음을 exit≠8 + lane-pack 미기록으로 확인(팀 기동 자체는 목 데몬이라 미완이 정상).
        rc, data = self._run_bootstrap(
            "/home/x/.local/state/cys-dept-dept-1/cys.sock", "pack-dept-dept-1")
        self.assertNotEqual(rc, 8, "정합 레인인데 레인↔팩 가드가 오발동(exit 8)")
        if data:
            self.assertNotIn("lane-pack", (data.get("result") or {}).get("failed_step", ""),
                             "정합인데 lane-pack 실패 기록")


class HookBootAbsent(unittest.TestCase):
    """t4 — 빈 팩(javis_bootstrap.py 부재)으로 훅 실행 → 명시 실패·행 없이 exit 0."""

    def test_absent_boot_explicit_failure_exit0(self):
        home = tempfile.mkdtemp()
        pack = tempfile.mkdtemp()  # 빈 팩 — bin/javis_bootstrap.py 없음
        mockbin = tempfile.mkdtemp()
        _mock_cys(mockbin, surface_role="")  # feed/send 모두 실패(데몬 부재) — 훅을 죽이면 안 됨
        env = dict(os.environ)
        env["HOME"] = home
        env["CYS_PACK_DIR"] = pack
        env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
        env.pop("CYS_SOCKET", None)
        # ★A2 surface 이중 게이트(T-0147-7 W1a): 훅은 cys pane 안에서만 발화한다. 이 케이스는
        # 'cys pane 안의 빈 팩'을 재현하므로 CYS_SURFACE_ID를 **명시** 주입한다 — os.environ
        # 상속에 의존하면 결과가 실행 위치(cys pane 안/밖)에 따라 흔들린다(결정론 파괴).
        env["CYS_SURFACE_ID"] = "9"
        env.pop("AITERM_SURFACE_ID", None)
        # timeout=20: 행 걸리면 TimeoutExpired로 테스트 실패(무행 요구 검증)
        r = subprocess.run(["bash", HOOK], input=json.dumps({"prompt": "너는 마스터다"}),
                           capture_output=True, text=True, timeout=20, env=env)
        self.assertEqual(r.returncode, 0, "BOOT 부재인데 훅 exit≠0(feed push 실패가 훅을 죽임)")
        out = r.stdout
        self.assertIn("hookSpecificOutput", out, "additionalContext JSON 미출력")
        payload = json.loads(out)
        ctx = payload["hookSpecificOutput"]["additionalContext"]
        self.assertIn("javis_bootstrap.py가 없어", ctx, "실패 원인이 additionalContext에 명시되지 않음")
        self.assertNotIn("발화됨", ctx, "BOOT 부재인데 '발화됨'으로 잘못 보고")


class EmbeddedSelfTest(unittest.TestCase):
    """t5 — 내장 --self-test 무회귀 게이트(subprocess exit 0)."""

    def test_self_test_passes(self):
        r = subprocess.run([sys.executable, BOOTSTRAP, "--self-test"],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, "내장 self-test 실패:\n%s\n%s" % (r.stdout, r.stderr))
        self.assertIn("self-test OK", r.stdout)


if __name__ == "__main__":
    unittest.main()
