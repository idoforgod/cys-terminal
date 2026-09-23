#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_notice_lane.py — ★U10(0.14.41 · 켤 때마다 승인 알림 누적) cys-dept 쪽 회귀 핀.

격리 HOME + 스텁 cys/cysd($HOME/.local/bin — cys-dept 의 PATH prepend 1순위) + argv·env 기록 스텁
javis_formation.py 로 실 데몬·실 ~/.cys 무접촉 검증한다.

  1) 부서 편성(launch 꼬리의 formation_ensure_async)은 **부서 소켓일 때 부서 팩**으로 돈다 —
     편성 스텁이 받은 CYS_PACK_DIR == $HOME/.cys/pack-dept-<name>(launch_dept 가 cysd 에 싣는 것과 같은 값) ·
     CYS_STATE_DIR 무변경(설정값 그대로 · 미설정이면 미설정 — 싱글플라이트 락 불변식) · --cwd 계약 유지.
     unix 소켓 경로와 Windows(MSYS uname 목 · named pipe 소켓) 두 갈래.
  1b) 부서 팩이 아직 설치 전(bin/javis_boot_node.py 부재)이면 종전 팩으로 접는다(편성 실패로 번지지 않는다 ·
     자가치유 ③ 보호) — 이때 CYS_PACK_DIR 는 호출자 값 그대로(미설정이면 미설정).
  2) CEO 알림 5종은 kind=ceo-notice — **기존 플래그** `--kind`(구 바이너리에도 있음 → 폴백 폭주 0) · 정적 전수.
  3) 'CEO 승격 보류(부트 필요)' dedupe — 동종 pending(구 kind=permission 포함)이 있으면 재발행 0.
  4) 승격 성공 → pending '보류/대기' 알림을 비허가 결정(ceo-promoted)으로 닫는다 · 무관 항목 무접촉 · allow 0.

    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_dept_notice_lane.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
DEPT = os.path.join(BIN, "cys-dept")

_SCRUB = ("CYS_ROLE", "CYS_SOCKET", "CYS_PACK_DIR", "CYS_ACCOUNT_DIR", "CYS_NO_AUTOSTART",
          "CYS_DEPT_ROTATE", "CYS_DEPT_CATALOG", "CYS_DEPT_DEFAULT_ACCOUNT", "CYS_PRIMARY_ACCOUNT",
          "CYS_DEPT_SEED_CREDS", "CYS_DEPT_CWD", "CLAUDE_CONFIG_DIR", "CYS_SURFACE_ID",
          "CYS_SURFACE_REF", "CYS_SEAT_TOKEN", "LOCALAPPDATA", "XDG_STATE_HOME", "CYS_STATE_DIR",
          "JAVIS_SOCKET", "AITERM_SOCKET")


def _write(path, text, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    if mode is not None:
        os.chmod(path, mode)


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class _Home(unittest.TestCase):
    """격리 HOME · 스텁 cys(호출 기록 · ping=소켓 파일 존재 · feed list=pending.tsv) · 스텁 cysd."""

    NAME = "w1"
    WINDOWS_MOCK = False

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="u10-dept-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.bindir = os.path.join(self.home, ".local", "bin")
        os.makedirs(self.bindir)
        self.calls = os.path.join(self.tmp, "calls.log")
        self.pending = os.path.join(self.tmp, "pending.tsv")
        if self.WINDOWS_MOCK:
            _write(os.path.join(self.bindir, "uname"), '#!/bin/sh\necho "MINGW64_NT-10.0"\n', 0o755)
        _write(os.path.join(self.bindir, "cys"),
               '#!/bin/sh\necho "cys $*" >> "%s"\n'
               'case "$1" in\n'
               '  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;\n'
               '  status) exit 1 ;;\n'
               '  feed) if [ "$2" = list ]; then [ -f "%s" ] && cat "%s"; fi; exit 0 ;;\n'
               'esac\nexit 0\n' % (self.calls, self.pending, self.pending), 0o755)
        _write(os.path.join(self.bindir, "cysd"), "#!/bin/sh\nexit 0\n", 0o755)
        self.hq = os.path.join(self.home, ".cys", "pack")
        os.makedirs(os.path.join(self.hq, "bin"))
        self.depts = os.path.join(self.home, ".cys", "depts.json")

    def env(self, **extra):
        env = dict(os.environ)
        env.update({"HOME": self.home, "USERPROFILE": self.home, "CYS_DEPTS_JSON": self.depts,
                    "PATH": self.bindir + os.pathsep + env.get("PATH", "")})
        for k in _SCRUB:
            env.pop(k, None)
        env.update(extra)
        return env

    def run_dept(self, *args, role=None, **extra):
        e = self.env(**extra)
        if role:
            e["CYS_ROLE"] = role
        return subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                              encoding="utf-8", env=e, cwd=self.tmp, timeout=120)

    def call_lines(self):
        return _read(self.calls).splitlines() if os.path.exists(self.calls) else []


class FormationLane(_Home):
    """1·1b — launch 재사용 경로 완주 뒤 백그라운드 편성 스텁이 받은 env 실측."""

    def setUp(self):
        super().setUp()
        if self.WINDOWS_MOCK:
            self.sock = r"\\.\pipe\cys-dept-%s" % self.NAME
            open(os.path.join(self.tmp, self.sock), "w").close()   # cwd=tmp 기준 상대 파일(ping 스텁용)
        else:
            self.sock = os.path.join(self.home, ".local", "state", "cys-dept-%s" % self.NAME, "cys.sock")
            _write(self.sock, "")
        _write(self.depts, json.dumps({"depts": {self.NAME: {
            "socket": self.sock, "pack_dir": self.dept_pack(), "role": "dept-master"}}}))
        self.fmlog = os.path.join(self.tmp, "formation.env")
        _write(os.path.join(self.hq, "bin", "javis_formation.py"),
               "import json, os, sys\n"
               "open(%r, 'a').write(json.dumps({'argv': sys.argv[1:],"
               " 'pack': os.environ.get('CYS_PACK_DIR', '<unset>'),"
               " 'state': os.environ.get('CYS_STATE_DIR', '<unset>')}) + '\\n')\n"
               "print('{}')\n" % self.fmlog)

    def dept_pack(self):
        return os.path.join(self.home, ".cys", "pack-dept-%s" % self.NAME)

    def install_dept_pack_bin(self):
        _write(os.path.join(self.dept_pack(), "bin", "javis_boot_node.py"), "# stub\n")

    def launch(self, **extra):
        if os.path.exists(self.fmlog):
            os.unlink(self.fmlog)
        r = self.run_dept("launch", self.NAME, **extra)
        for _ in range(100):                      # 편성은 백그라운드 — 기록을 기다린다
            if os.path.exists(self.fmlog):
                break
            time.sleep(0.1)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertTrue(os.path.exists(self.fmlog), "편성 스텁이 호출되지 않았다: %s" % r.stderr[-400:])
        rec = json.loads(_read(self.fmlog).splitlines()[-1])
        self.assertIn("--socket", rec["argv"])
        return r, rec

    @staticmethod
    def _same_path(a, b):
        # MSYS 목에서도 bash 가 만든 문자열 그대로 비교한다(경로 표기 = cys-dept dept_pack 규칙).
        return os.path.normpath(a) == os.path.normpath(b)

    def test_1_dept_socket_formation_runs_in_dept_pack(self):
        self.install_dept_pack_bin()
        r, rec = self.launch()
        self.assertTrue(self._same_path(rec["pack"], self.dept_pack()),
                        "부서 편성이 부서 팩으로 돌지 않았다(본부 지침·MEMORY 가 부서 좌석에 주입됨): %s" % rec)
        self.assertEqual(rec["state"], "<unset>", "CYS_STATE_DIR 를 새로 설정했다(락 불변식 위반)")
        self.assertEqual(rec["argv"][rec["argv"].index("--socket") + 1], self.sock)
        self.assertIn("--cwd", rec["argv"], "--cwd 계약(시드와 같은 cwd)이 깨졌다")
        self.assertIn("부서 팩", r.stderr, "착수 1줄에 레인 팩 고지가 없다")

    def test_1_state_dir_passthrough_unchanged(self):
        self.install_dept_pack_bin()
        st = os.path.join(self.tmp, "state-root")
        _, rec = self.launch(CYS_STATE_DIR=st)
        self.assertEqual(rec["state"], st, "CYS_STATE_DIR 가 바뀌었다(편성 락 루트 분열)")
        self.assertTrue(self._same_path(rec["pack"], self.dept_pack()))

    def test_1b_uninstalled_dept_pack_falls_back_to_caller_pack(self):
        r, rec = self.launch()
        self.assertEqual(rec["pack"], "<unset>",
                         "설치 전 부서 팩을 편성에 실었다(boot_node 부재 = 편성 실패로 번진다): %s" % rec)
        self.assertIn("종전 팩", r.stderr, "폴백을 조용히 삼켰다(드러내기 누락)")
        custom = os.path.join(self.tmp, "custom-pack")
        shutil.copytree(self.hq, custom)
        _, rec2 = self.launch(CYS_PACK_DIR=custom)
        self.assertEqual(rec2["pack"], custom, "폴백이 호출자 팩을 바꿨다")

    def test_1c_caller_pack_kill_switch_keeps_old_env(self):
        # 호출자(본부) 팩의 kill-switch 파일은 종전에 부서 편성도 멈췄다(javis_formation paused_paths =
        # $PACK_DIR/AUTOPILOT_PAUSED) — 부서 팩 전환이 그 정지를 우회하면 안 된다.
        self.install_dept_pack_bin()
        _write(os.path.join(self.hq, "AUTOPILOT_PAUSED"), "")
        r, rec = self.launch()
        self.assertEqual(rec["pack"], "<unset>", "본부 kill-switch 중인데 부서 팩으로 전환했다(정지 우회): %s" % rec)
        self.assertIn("kill-switch", r.stderr)


class FormationLaneWindows(FormationLane):
    WINDOWS_MOCK = True


class CeoNoticeStatic(unittest.TestCase):
    """2 — cys-dept 의 CEO feed push 전수가 --kind ceo-notice 를 싣는다(정적)."""

    def test_2_every_ceo_push_carries_ceo_notice_kind(self):
        src = _read(DEPT)
        joined = re.sub(r"\\\n\s*", " ", src)      # 줄 이음 병합
        pushes = [l for l in joined.splitlines() if re.search(r'"\$CYS" feed push\b', l)]
        ceo = [l for l in pushes if re.search(r'--title "CEO', l)]
        self.assertGreaterEqual(len(ceo), 5, "CEO 알림 발행 지점 수가 줄었다(검체 갱신 필요): %d" % len(ceo))
        for l in ceo:
            self.assertIn("--kind ceo-notice", l, "kind 없는 CEO 알림(기본 permission = 승인 요청으로 쌓임): %s" % l.strip()[:160])
        self.assertNotIn("--coalesce-key", src, "구 바이너리가 모르는 새 플래그 금지(폴백 폭주)")
        self.assertNotIn("feed reply \"$id\" allow", src)


class CeoNoticeRuntime(_Home):
    """3·4 — 실행 확인(test_ceo_pending_gate 동형 픽스처)."""

    MASTER = "STANDARD-MASTER\n"
    CEO = "CEO-HEADER\n---\n" + MASTER

    def setUp(self):
        super().setUp()
        d = os.path.join(self.hq, "directives")
        _write(os.path.join(d, "MASTER_DIRECTIVE.md"), self.MASTER)
        _write(os.path.join(d, "CEO_TEMPLATE.md"), self.CEO)
        _write(self.depts, json.dumps({"depts": {"d0": {}}}))
        self.marker = os.path.join(self.home, ".cys", ".master-bootstrapped")
        self.ceo_pending = os.path.join(self.home, ".cys", "state", "ceo-pending")

    def pushes(self, title):
        return [l for l in self.call_lines() if "feed push" in l and title in l]

    def test_3_boot_hold_notice_kind_and_dedupe(self):
        r = self.run_dept("promote-ceo")
        self.assertEqual(r.returncode, 5, r.stderr[-300:])
        p = self.pushes("CEO 승격 보류(부트 필요)")
        self.assertEqual(len(p), 1, self.call_lines())
        self.assertIn("--kind ceo-notice", p[0])
        # 동종 pending 존재(구 kind=permission 라인 포함) → 재발행 0
        _write(self.pending, "rid-old\t[pending]\tpermission\tCEO 승격 보류(부트 필요)\tdecision=-\n")
        self.run_dept("promote-ceo")
        self.run_dept("promote-ceo")
        self.assertEqual(len(self.pushes("CEO 승격 보류(부트 필요)")), 1, "dedupe 실패 — 켤 때마다 1건씩 쌓인다")
        # pending 이 사라지면(해소·만료) 다시 알린다(재발 신호 보존)
        os.unlink(self.pending)
        self.run_dept("promote-ceo")
        self.assertEqual(len(self.pushes("CEO 승격 보류(부트 필요)")), 2)

    def test_4_promotion_success_closes_hold_and_wait_notices(self):
        os.makedirs(os.path.dirname(self.ceo_pending), exist_ok=True)
        _write(self.ceo_pending, "pending\n")
        _write(self.marker, "{}")
        _write(self.pending,
               "rid-a\t[pending]\tpermission\tCEO 승격 보류(부트 필요)\tdecision=-\n"
               "rid-b\t[pending]\tceo-notice\tCEO 승격 대기\tdecision=-\n"
               "rid-c\t[pending]\tceo-notice\tCEO 승격 보류(템플릿 상위집합 검사 실패)\tdecision=-\n"
               "rid-x\t[pending]\tpermission\t다른 승인 요청\tdecision=-\n")
        r = self.run_dept("promote-if-pending")
        self.assertEqual(r.returncode, 0, r.stderr[-300:])
        self.assertEqual(_read(os.path.join(self.hq, "directives", "MASTER_DIRECTIVE.md")), self.CEO, "승격 미완")
        replies = [l for l in self.call_lines() if "feed reply" in l]
        for rid in ("rid-a", "rid-b", "rid-c"):
            self.assertIn("cys feed reply %s ceo-promoted" % rid, replies, "승격 뒤 '%s' 가 남았다: %s" % (rid, replies))
        self.assertFalse(any("rid-x" in l for l in replies), "무관 항목을 닫았다")
        self.assertFalse(any(l.endswith(" allow") for l in replies), "자동 종결이 allow 를 썼다")
        done = self.pushes("CEO 승격 완료(자동)")
        self.assertEqual(len(done), 1)
        self.assertIn("--kind ceo-notice", done[0])

    def test_4b_request_only_wait_notice_kind(self):
        os.makedirs(os.path.dirname(self.ceo_pending), exist_ok=True)
        _write(self.ceo_pending, "pending\n")
        _write(self.marker, "{}")
        r = self.run_dept("promote-if-pending", "--request-only", role="master")
        self.assertEqual(r.returncode, 0, r.stderr[-300:])
        p = self.pushes("CEO 승격 대기")
        self.assertEqual(len(p), 1, self.call_lines())
        self.assertIn("--kind ceo-notice", p[0])

    def test_4c_close_is_fail_open_when_daemon_unreachable(self):
        # feed list 실패(데몬 미기동) → 닫기 생략 · 승격 자체는 완주(부서 흐름 불파괴)
        _write(os.path.join(self.bindir, "cys"),
               '#!/bin/sh\necho "cys $*" >> "%s"\ncase "$1" in status) exit 1;; feed) [ "$2" = list ] && exit 1;; esac\nexit 0\n'
               % self.calls, 0o755)
        os.makedirs(os.path.dirname(self.ceo_pending), exist_ok=True)
        _write(self.ceo_pending, "pending\n")
        _write(self.marker, "{}")
        r = self.run_dept("promote-if-pending")
        self.assertEqual(r.returncode, 0, r.stderr[-300:])
        self.assertEqual(_read(os.path.join(self.hq, "directives", "MASTER_DIRECTIVE.md")), self.CEO)
        self.assertFalse(any("feed reply" in l for l in self.call_lines()))


if __name__ == "__main__":
    unittest.main(verbosity=2)
