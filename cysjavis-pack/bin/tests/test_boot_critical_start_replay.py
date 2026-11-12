#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_boot_critical_start_replay.py — 기동 필수 목록의 산출 규칙을 **부트 스크립트 층**에서 잰다
(0.15.0 · T5a · RULINGS AI7 · 계약 C4 4.4 · 설계 D:734).

무엇을 재는가: 실 팩(저장소의 cysjavis-pack 사본)을 격리 HOME 에 깔고, PATH 맨 앞에 **스텁 `cys`**
(라이브 데몬 무접촉 · 응답은 파일로 준다)를 놓은 채 **실 `javis_bootstrap.py run`** 을 돌린다.
  ① 건강한 팩 → 부트가 끝난다(exit 0 + 완주 표식 `~/.cys/.master-bootstrapped`).
  ② 목록에 든 파일 가운데 이 부트가 실제로 읽는 것(트립와이어로 확인)을 **하나씩** 망가뜨린다 →
     부트가 실패해야 한다 — 단, 부트 스크립트가 **설계상 그 실패를 견디는** 행은 아래
     `TOLERATED_BY_DESIGN` 에 근거와 함께 적혀 있어야 한다(적히지 않은 견딤 = 실패).
  ③ 목록 밖의 파일을 **전부 한꺼번에** 망가뜨린다 → 부트는 여전히 끝난다.
  ④ 이 층에서 닿지 않는 행(데몬·좌석·부서가 있어야 도는 것)은 표에 '여기서 미행사'로 남는다 —
     보고서가 그것을 5단계 종단 시험 항목으로 적는다.
목록의 출처: `src/repair/boot_critical.rs` 의 `PINNED`(산출 목록의 고정 입력 — RULINGS B6 · Rust 시험이
그것을 컴파일된 표와 같다고 고정한다). 빌드된 `cys` 가 있으면(`$CARGO_TARGET_DIR/debug/cys`) 그
`--list --json` 과도 대조한다.
라이브 무접촉: HOME·TMPDIR·팩은 임시 디렉터리, `cys` 를 가진 PATH 항목은 모두 빼고 스텁만 둔다,
CYS_SOCKET 은 지정하지 않고(스텁이 답한다) 자동 기동 노브 넷을 켠다.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

TESTS = Path(__file__).resolve().parent
BIN = TESTS.parent
PACK_SRC = BIN.parent
REPO = PACK_SRC.parent
RS = REPO / "src" / "repair" / "boot_critical.rs"
sys.dont_write_bytecode = True

# 이 층에서 **읽히지만**(트립와이어) 망가뜨려도 부트가 끝나는 행 — 2026-11-13 실측(runs/T5a-ai-20261113-081251-b).
#   목록에서 빼지 않는다(RULINGS AI1: 도달 가능한 파일을 일부러 빼는 것은 마스터 판정 사항) — 보고서의
#   CONTRACT CHANGE REQUEST CCR-2 가 이 집합을 마스터에게 올린다. 여기 **없는** 견딤이 새로 생기면 실패다
#   (산출 규칙이나 부트 스크립트가 바뀌어 목록이 부트를 멈추지 않는 파일을 더 품게 됐다는 뜻).
_PRE = "부트 ① preflight 경로(preflight 와 그것이 import·경로 실행하는 도구) — 부트는 preflight 실패를 견딘다(test_bootstrap_chain 6b)"
_OPT = "부트 ⑤ orchestra check · 부트 스크립트의 선택 import(실패 시 대체 경로로 진행한다)"
TOLERATED_BY_DESIGN = {
    "bin/javis_preflight.py": _PRE + " — 설계 D:730 이 기동 스크립트로 이름 붙였다",
    "bin/cys-dept": "부트 ⑦ promote-if-pending --request-only 는 보내고 잊는다 — 부서 복구(데몬 내장 작업)는 이 층 밖(5단계)",
    "agents.json": "좌석 기동은 `cys boot`/launch-agent(본체 · 여기서는 스텁)가 읽는다 — load_agent_spec 의 하드 실패는 5단계",
    "bin/javis_boot_node.py": "부서 편성(cys-dept launch → formation)이 좌석을 붙이는 길 — 기본 부트는 `cys boot` 로 붙이고 "
                              "orchestra 의 import 는 선택적이다(5단계)",
    "bin/javis_lane.py": _OPT,
    "bin/javis_channels.py": _OPT,
    "bin/javis_verdict.py": _OPT,
    "bin/javis_mission.py": _OPT,
    "bin/javis_detect.py": _OPT,
    "bin/javis_budget.py": _OPT,
    "bin/javis_cleanroom.py": _PRE,
    "bin/javis_completion_guard.py": _PRE,
    "bin/javis_cycle_autopilot.py": _PRE,
    "bin/javis_mcpgate.py": _PRE,
    "bin/javis_registry.py": _PRE,
    "bin/javis_runtime_seal.py": _PRE,
    "bin/javis_semver.py": _PRE,
    "bin/javis_skillscan.py": _PRE,
    "bin/skillscan_rules.json": _PRE,
    "bin/_owned_process.py": _PRE,
    "bin/grill_gate.py": _PRE,
    "bin/javis_radio.py": _PRE,
    "bin/javis_wakeup.py": _PRE,
    "bin/javis_scrub.py": _PRE,
    "bin/javis_lock.py": _PRE,
    "bin/javis_manifest.py": _PRE,
    "bin/javis_resource_gate.py": _PRE,
    "bin/javis_route.py": _PRE,
    "bin/javis_todo_decl.py": _PRE,
}


def pinned_rows():
    """`PINNED` 표(산출 목록의 고정 입력) → [(rel, category)]."""
    text = RS.read_text(encoding="utf-8")
    at = text.index("const PINNED: &[Row] = &[")
    block = text[at:text.index("\n    ];", at)]
    rows = re.findall(r'\(\s*"([^"]+)",\s*C::(\w+),\s*K::(\w+),\s*"([^"]*)",\s*(?:None|Some\(\d+\)),\s*(?:true|false),?\s*\)',
                      block, re.S)
    return [(r[0], r[1]) for r in rows]


def clean_path():
    out = []
    for d in (os.environ.get("PATH", "") or "").split(os.pathsep):
        if d and not any(os.path.exists(os.path.join(d, n)) for n in ("cys", "cys.exe")):
            out.append(d)
    return out


def copy_pack(dst):
    """build.rs 와 같은 제외 규칙(`.`로 시작하는 성분 · tests · __pycache__)으로 팩 사본을 만든다."""
    for root, dirs, files in os.walk(PACK_SRC):
        rel_root = os.path.relpath(root, PACK_SRC)
        parts = [] if rel_root == "." else rel_root.split(os.sep)
        dirs[:] = [d for d in dirs if not d.startswith(".") and d not in ("tests", "__pycache__")]
        if any(p.startswith(".") or p in ("tests", "__pycache__") for p in parts):
            continue
        for f in files:
            if f.startswith("."):
                continue
            src = os.path.join(root, f)
            rel = f if not parts else "/".join(parts + [f])
            d = os.path.join(dst, rel)
            os.makedirs(os.path.dirname(d), exist_ok=True)
            shutil.copy2(src, d)
    return dst


STUB_CYS = r'''#!/bin/sh
T="$T5A_STUB"
echo "cys $*" >> "$T/calls.log"
case "$1" in
  ping) exit 0 ;;
  claim-role) exit 0 ;;
  boot) cat "$T/boot.stdout"; exit 0 ;;
  status) cat "$T/status.json"; exit 0 ;;
  agent-detect) cat "$T/detect.json"; exit 0 ;;
  list) cat "$T/list.txt"; exit 0 ;;
  --version) echo 'cys 0.0.0-stub'; exit 0 ;;
  *) exit 0 ;;
esac
'''
ROLES = ("cso", "worker", "reviewer-gemini", "reviewer-codex")


def is_python(path):
    if path.endswith(".py"):
        return True
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return "python" in f.readline()
    except OSError:
        return False


def broken_body(path):
    if path.endswith(".json"):
        return "{\n"
    if is_python(path):
        return "def (:\n"
    if path.endswith((".sh", ".md", ".txt", ".yml", ".yaml", ".toml", ".html", ".js")) or "/" not in path:
        return "#!/bin/sh\n(\n"
    return "#!/bin/sh\n(\n"


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("sh"),
                     "macOS 전용(부트 스크립트의 건강한 기준선은 macOS 에서 쟀다)")
class BootstrapStartReplay(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="t5a-boot-")
        self.addCleanup(tmp.cleanup)
        self.root = Path(os.path.realpath(tmp.name))
        self.stub = self.root / "stub"
        self.stub.mkdir()
        (self.stub / "cys").write_text(STUB_CYS, encoding="utf-8")
        (self.stub / "cys").chmod(0o755)
        surfaces = [{"role": r, "exited": False, "agent_alive": True,
                     "status": {"state": "working", "age_secs": 5}} for r in ROLES]
        (self.stub / "status.json").write_text(json.dumps({"surfaces": surfaces}), encoding="utf-8")
        (self.stub / "detect.json").write_text(json.dumps({"agents": {
            "gemini": {"installed": True, "reason": "stub"}, "codex": {"installed": True, "reason": "stub"},
            "grok": {"installed": False, "reason": "stub"}, "claude": {"installed": True, "reason": "stub"}}}),
            encoding="utf-8")
        (self.stub / "list.txt").write_text("".join(
            "surface:%d\trole=%s\tpid=%d\texited=false\n" % (10 + i, r, 1010 + i) for i, r in enumerate(ROLES)),
            encoding="utf-8")
        (self.stub / "boot.stdout").write_text(
            json.dumps({"roles": [], "summary": {"lock": "acquired", "fatal_failed": 0}}) + "\n", encoding="utf-8")
        self.trip = self.root / "trip.log"
        self.runs = 0

    def fresh_home(self):
        """새 HOME + 실 팩 사본(~/.cys/pack)."""
        self.runs += 1
        home = self.root / ("home-%d" % self.runs)
        pack = home / ".cys" / "pack"
        copy_pack(str(pack))
        return home, pack

    def run_boot(self, home, pack):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_", "CLAUDE"))}
        tmpd = home / "tmp"
        tmpd.mkdir(exist_ok=True)
        env.update({"HOME": str(home), "TMPDIR": str(tmpd), "T5A_STUB": str(self.stub),
                    "T5A_TRIP_LOG": str(self.trip),
                    "PATH": os.pathsep.join([str(self.stub)] + clean_path()),
                    "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "2", "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                    "CYS_NO_AUTOSTART": "1", "CYS_NO_OFFICE_BRIDGE": "1",
                    "CYS_NO_PERSONAL_HOOK_MERGE": "1", "CYS_NO_AUTORESTORE": "1"})
        t0 = time.time()
        try:
            r = subprocess.run([sys.executable, str(pack / "bin" / "javis_bootstrap.py"), "run"],
                               capture_output=True, text=True, env=env, cwd=str(home), timeout=900)
            rc, out, err = r.returncode, r.stdout, r.stderr
        except subprocess.TimeoutExpired as e:
            rc, out, err = None, str(e.stdout or ""), str(e.stderr or "")
        ok = rc == 0 and (home / ".cys" / ".master-bootstrapped").exists()
        return ok, rc, time.time() - t0, err[-600:]

    def tripwire(self, pack, rel):
        p = pack / rel
        t = p.read_text(encoding="utf-8", errors="surrogateescape")
        first, rest = ("", t)
        if t.startswith("#!"):
            first, _, rest = t.partition("\n")
            first += "\n"
        if is_python(str(p)):
            line = ('import os as _t5a_os; _t5a_p = _t5a_os.environ.get("T5A_TRIP_LOG"); '
                    '_t5a_p and open(_t5a_p, "a").write("%s\\n")\n' % rel)
        elif rel.endswith(".sh") or t.startswith("#!"):
            line = "printf '%%s\\n' '%s' >> \"${T5A_TRIP_LOG:-/dev/null}\" 2>/dev/null || :\n" % rel
        else:
            return False
        p.write_text(first + line + rest, encoding="utf-8", errors="surrogateescape")
        return True

    def test_derivation_rule_at_the_bootstrap_layer(self):
        rows = pinned_rows()
        self.assertGreater(len(rows), 20, "PINNED 표를 읽지 못했다(파서가 표 형식과 어긋난다)")
        listed = {r for r, _ in rows}
        start_rows = [r for r, c in rows if c in ("StartScript", "StartDep", "HookDep", "Config")
                      and not r.startswith("hooks/")]
        built = os.path.join(os.environ.get("CARGO_TARGET_DIR", ""), "debug", "cys")
        if os.environ.get("CARGO_TARGET_DIR") and os.path.isfile(built) and os.path.getsize(built) > 0:
            r = subprocess.run([built, "repair", "boot-critical", "--list", "--json"], capture_output=True,
                               text=True, timeout=30, env=dict(os.environ, CYS_NO_AUTOSTART="1",
                                                                HOME=str(self.root), CYS_SOCKET=str(self.root / "s")))
            if r.returncode == 0:
                got = {e["rel"] for e in json.loads(r.stdout)["entries"]}
                self.assertEqual(got, listed, "PINNED 과 빌드된 본체의 목록이 다르다")
        matrix, problems = [], []

        # ① 건강한 부트
        home, pack = self.fresh_home()
        ok, rc, dt, err = self.run_boot(home, pack)
        matrix.append("healthy: ok=%s rc=%s %.1fs" % (ok, rc, dt))
        if not ok:
            problems.append("① 건강한 팩으로 부트가 끝나지 않았다: rc=%s %s" % (rc, err))

        # 트립와이어 — 이 부트가 실제로 읽는 목록 파일
        home, pack = self.fresh_home()
        for rel in listed:
            if (pack / rel).is_file():
                self.tripwire(pack, rel)
        if self.trip.exists():
            self.trip.unlink()
        ok, rc, dt, err = self.run_boot(home, pack)
        read = set(self.trip.read_text(encoding="utf-8").split()) if self.trip.exists() else set()
        matrix.append("tripwire run: ok=%s rc=%s %.1fs · reads %d listed files" % (ok, rc, dt, len(read)))
        if not ok:
            problems.append("트립와이어 부트가 끝나지 않았다(트립와이어 자체가 부트를 깼다?): rc=%s %s" % (rc, err))

        # ② 읽는 목록 파일을 하나씩 망가뜨린다
        for rel in sorted(start_rows):
            exercised = rel in read or rel.endswith(".json")
            if not exercised:
                matrix.append("not exercised here: %s (5단계 종단 시험)" % rel)
                continue
            home, pack = self.fresh_home()
            (pack / rel).write_text(broken_body(rel), encoding="utf-8")
            ok, rc, dt, err = self.run_boot(home, pack)
            matrix.append("break %s -> boot %s (rc=%s %.1fs)" % (rel, "COMPLETES" if ok else "fails", rc, dt))
            if ok and rel not in TOLERATED_BY_DESIGN:
                problems.append("② %s 를 망가뜨려도 부트가 끝났다 — 목록에 든 근거가 이 층에 없다"
                                "(TOLERATED_BY_DESIGN 에도 없다)" % rel)
            if not ok and rel in TOLERATED_BY_DESIGN:
                matrix.append("  note: %s now STOPS the boot (no longer tolerated)" % rel)

        # ③ 목록 밖 파일 전부를 한꺼번에 망가뜨린다
        home, pack = self.fresh_home()
        unlisted = []
        for root_, _dirs, files in os.walk(pack):
            for f in files:
                rel = os.path.relpath(os.path.join(root_, f), pack).replace(os.sep, "/")
                if rel not in listed:
                    unlisted.append(rel)
        for rel in unlisted:
            (pack / rel).write_text(broken_body(rel), encoding="utf-8")
        ok, rc, dt, err = self.run_boot(home, pack)
        matrix.append("break all %d unlisted files -> boot %s (rc=%s %.1fs)"
                      % (len(unlisted), "COMPLETES" if ok else "fails", rc, dt))
        if not ok:
            problems.append("③ 목록 밖 파일을 망가뜨렸더니 부트가 실패했다 — 목록이 빠뜨린 파일이 있다: rc=%s %s"
                            % (rc, err))

        calls = (self.stub / "calls.log").read_text(encoding="utf-8").splitlines() \
            if (self.stub / "calls.log").exists() else []
        matrix.append("stub cys calls: %d" % len(calls))
        report = "\n".join(matrix) + "\nPROBLEMS:\n" + "\n".join(problems) + "\n"
        td = os.environ.get("CARGO_TARGET_DIR")
        if td and os.path.isdir(td):
            Path(td, "t5a-bootstrap-replay.txt").write_text(report, encoding="utf-8")
        print(report)
        self.assertEqual(problems, [], report)


if __name__ == "__main__":
    unittest.main(verbosity=2)
