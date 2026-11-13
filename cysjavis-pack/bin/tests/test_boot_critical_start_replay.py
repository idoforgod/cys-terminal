#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_boot_critical_start_replay.py — 기동 필수 목록의 산출 규칙을 **부트 스크립트 층**에서 잰다
(0.15.0 · T5a · RULINGS AI7 · AL1 · AN2 · AN4 · AN5 · 계약 C4 4.4 · 설계 D:734).

무엇을 재는가: 실 팩(저장소의 cysjavis-pack 사본)을 격리 HOME 에 깔고, PATH 를 **고정 목록**(스텁 폴더 ·
인터프리터 폴더 · /usr/bin:/bin:/usr/sbin:/sbin)으로만 만든 채 **실 `javis_bootstrap.py run`** 을 돌린다.
`cys` · `nlm` · `uv` · `pipx` · `pip` · `pip3` 는 스텁이다(라이브 데몬 무접촉 · 응답은 파일로 준다).
  ① 건강한 팩 → 부트가 끝난다(exit 0 + 완주 표식 `~/.cys/.master-bootstrapped`) · 격리 HOME 의 Claude
     프로필 settings.json 에 제품 훅이 등록된다(AN5 — 이 등록 집합이 '완전'의 기준이다).
  ①′ 결손 장면(스텁 status 가 `cys boot` 전까지 worker 를 비운다) → 부트가 ④ 팀 기동 갈래(자원 게이트 ·
     `cys boot` · ④-b)를 지나 끝난다. 이 장면의 대조군은 자원 게이트가 '허용'을 답하는 사본이다
     (실 게이트는 이 기계의 부하를 재므로 결정론이 아니다).
  ② 고장 두 갈래(AN2): (i) 파일이 돌 수 없다(문법 오류) · (ii) 기동 경로가 **멈춤으로 따르는 답**을 주는
     파일이 그 답을 준다(아래 `STOP_ANSWERS` — 기동 경로를 읽어 찾은 멈춤 갈래). 목록의 갈래 2 행
     (`bootstrap-replay`)은 어느 한 갈래로 이 층에서 부트를 **멈춰야** 한다. 갈래 3 행(`real-level`)은
     견딤이 허용된다(실기 의존 — 5단계 종단 시험 항목).
  ③ 목록에서 뺀 후보(DROPPED_CANDIDATES) 가운데 이 부트가 읽는 것을 **하나씩** 망가뜨린다 → 부트가 끝나고
     훅 등록 집합이 완전해야 한다(AN5). 멈춤 갈래가 있는 후보는 (ii) 로도 잰다.
  ④ 목록 밖 파일을 **전부 한꺼번에** 망가뜨린다(두 장면) → 부트가 끝나고 훅 등록 집합이 완전하다.
  ⑤ 밀폐(AN4): 실행 뒤 스텁 호출 기록에 기계의 절대 경로 `cys` 가 없고, 격리 HOME 의 산출물 어디에도
     기계의 `cys` 위치가 적히지 않았으며, 호출자의 실제 `~/.local/bin` 목록이 그대로다.
목록의 출처: `src/repair/boot_critical.rs` 의 `PINNED`(산출 목록의 고정 입력) · 뺀 후보는
`src/repair/boot_critical_derive.rs` 의 `DROPPED_CANDIDATES`. 빌드된 `cys` 가 있으면 그 `--list --json` 과도 대조한다.
preflight 의 시험 전용 스위치(AN4): `CYS_PREFLIGHT_HERMETIC_TEST=1` — 격리 HOME 표식 `CYS_TEST_ISOLATED_HOME`
이 실제 HOME 과 같을 때만 듣는다(C65 기계 고정 cys 후보 · C11b HOME 밖 링크 정리 · C20 인터프리터 pip 설치 생략).
격리 HOME 은 임시 디렉터리 밖(`$CARGO_TARGET_DIR` 아래)에 둔다 — preflight 는 임시 팩에서는 훅을 등록하지 않는다.
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
DERIVE_RS = REPO / "src" / "repair" / "boot_critical_derive.rs"
sys.dont_write_bytecode = True

FIXED_SYSTEM_PATH = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
# 기계의 cys 가 놓이는 자리 — 격리 HOME 산출물에 이 경로가 적히면 기계의 cys 를 집은 것이다
MACHINE_CYS = ("/Applications/cys.app/Contents/MacOS/cys", "/opt/homebrew/bin/cys", "/usr/local/bin/cys")

# RULINGS AL1 — 갈래 3(`real-level`): 설계가 이름 붙였거나 실기 의존 때문에 기동 필수인 파일. 이 층(스텁 `cys`)이
#   그 의존을 못 보므로 견딤이 허용된다. 실기 증명은 5단계 종단 시험의 몫이다.
REAL_LEVEL = {
    "bin/javis_preflight.py": "부트 ① 은 preflight 실패를 견딘다 — 설계 D:730 이 기동 스크립트로 이름 붙였고 그 fix 단계가 "
                              "부트 때 깨진 설치를 고친다",
    "bin/cys-dept": "부트 ⑦ 은 보내고 잊는다 — 부서의 기동·복구(데몬 내장 작업)가 이 파일로 돈다",
    "agents.json": "좌석 기동은 본체(`cys boot`/launch-agent · 여기서는 스텁)가 읽는다 — load_agent_spec 의 하드 실패(D:731)",
    "bin/javis_boot_node.py": "모든 좌석이 이것으로 붙는다 — 실기에서 좌석이 없으면 부트 ⑤ 가 실패한다",
    "bin/javis_formation.py": "부서 상비편성이 좌석을 붙인다(cys-dept launch → formation → boot_node)",
    "bin/javis_cli_probe.py": "편성(formation ensure)이 설치 CLI 를 이것으로 판정한다 — 모두 '없음'이면 pending-cli(좌석 0), "
                              "문법 오류면 `except ImportError` 가 못 잡아 편성이 죽는다(javis_formation.py:488-496 · "
                              "1670-1678 · 좌석 붙이기 층에서만 닿는 멈춤 갈래 — RULINGS AN2)",
}
# 갈래 2(`bootstrap-replay`): 이 층에서 (i) 또는 (ii) 로 부트가 멈춘다(재는 대상)
BOOTSTRAP_LEVEL = {"bin/javis_bootstrap.py", "bin/javis_orchestra.py", "bin/javis_resource_gate.py",
                   "bin/javis_lock.py", "bin/javis_lane.py", "bin/javis_budget.py"}

# RULINGS AN2 (ii) — 기동 경로를 읽어 찾은 멈춤 갈래: 파일 → (장면, 그 파일이 줄 멈춤 답, 근거). 답은 실 파일에
#   덧붙인 재정의(같은 이름의 뒤 정의가 앞 정의를 이긴다)이거나 그 답만 내는 사본이다.
STOP_ANSWERS = {
    "bin/javis_lock.py": (
        "normal", "append",
        "\n\ndef _t5a_busy(self, *a, **k):\n    return BUSY\n\n\nFileLock.acquire = _t5a_busy\n",
        "FileLock.acquire → BUSY: 부트 싱글플라이트가 패자로 접혀 exit 11(팀 기동 0 · javis_bootstrap.py:1546-1548 · 1447-1449)"),
    "bin/javis_lane.py": (
        "normal", "append",
        "\n\ndef socket_malformed_dept(sock):\n    return True\n",
        "socket_malformed_dept → True: 불량 레인 가드 exit 8(javis_bootstrap.py:3511-3523 · socket_dept 가 팩과 다른 "
        "부서를 답해도 :3528-3539 의 같은 exit 8)"),
    "bin/javis_budget.py": (
        "normal", "append",
        "\n\nfor _t5a_k in list(LEAF_FLOORS):\n    LEAF_FLOORS[_t5a_k] = type(LEAF_FLOORS[_t5a_k])(0)\n",
        "시간 한도 0(모든 leaf): ② ping 이 0 초 한도로 시간 초과 → EXIT_PING(javis_bootstrap.py:235-267 · 3621-3646) — "
        "이 파일의 답은 '얼마나 기다리나'이고 0 은 기동 경로가 따르는 '기다리지 마라'다(해석 — 보고서 §16)"),
    "bin/javis_resource_gate.py": (
        "deficit", "replace",
        "#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps({\"verdict\": \"hard_block\", \"trips\": [{\"metric\": "
        "\"servers\", \"level\": \"hard\", \"value\": 9, \"hard\": 3}], \"measured\": {}}))\nsys.exit(2)\n",
        "check --json → exit 2 + hard servers: 결손이 있을 때 EXIT_RESOURCE_HARD 9(javis_bootstrap.py:3176-3233 · 3893) · "
        "편성도 같은 답에 pending-resource(javis_formation.py:505-523 · 1654-1665)"),
}
GATE_ALLOW = ("#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps({\"verdict\": \"allow\", \"trips\": [], "
              "\"measured\": {}}))\nsys.exit(0)\n")


def pinned_rows():
    """`PINNED` 표(산출 목록의 고정 입력) → [(rel, category)]."""
    text = RS.read_text(encoding="utf-8")
    at = text.index("const PINNED: &[Row] = &[")
    block = text[at:text.index("\n    ];", at)]
    rows = re.findall(r'\(\s*"([^"]+)",\s*C::(\w+),\s*K::(\w+),\s*"([^"]*)",\s*(?:None|Some\(\d+\)),\s*(?:true|false),?\s*\)',
                      block, re.S)
    return [(r[0], r[1]) for r in rows]


def dropped_rows():
    """`DROPPED_CANDIDATES`(측정으로 뺀 후보) → [rel]."""
    text = DERIVE_RS.read_text(encoding="utf-8")
    at = text.index("pub const DROPPED_CANDIDATES: &[(&str, &str)] = &[")
    block = text[at:text.index("\n];", at)]
    return re.findall(r'^\s*\(\s*"([^"]+)",', block, re.M)


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
echo "$0 $*" >> "$T/calls.log"
case "$1" in
  ping) exit 0 ;;
  claim-role) exit 0 ;;
  boot) : > "$T/booted"; cat "$T/boot.stdout"; exit 0 ;;
  status) if [ -f "$T/deficit" ] && [ ! -f "$T/booted" ]; then cat "$T/status-deficit.json"; else cat "$T/status.json"; fi; exit 0 ;;
  agent-detect) cat "$T/detect.json"; exit 0 ;;
  list) if [ -f "$T/deficit" ] && [ ! -f "$T/booted" ]; then cat "$T/list-deficit.txt"; else cat "$T/list.txt"; fi; exit 0 ;;
  --version) echo 'cys 0.0.0-stub'; exit 0 ;;
  *) exit 0 ;;
esac
'''
# 설치 도구 스텁(AN4) — 기록만 하고 실패한다(아무것도 내려받거나 설치하지 않는다)
STUB_TOOL = '#!/bin/sh\necho "$0 $*" >> "$T5A_STUB/calls.log"\nexit 1\n'
STUB_TOOLS = ("nlm", "uv", "pipx", "pip", "pip3")
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
    return "#!/bin/sh\n(\n"


def registered_hooks(home):
    """격리 HOME 의 Claude 프로필 settings.json 에 등록된 (이벤트, 훅 스크립트) 집합 — 판독 불가는 None."""
    p = home / ".claude" / "settings.json"
    try:
        doc = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    out = set()
    for ev, groups in ((doc.get("hooks") or {}) if isinstance(doc, dict) else {}).items():
        for g in groups if isinstance(groups, list) else []:
            for h in (g.get("hooks") or []) if isinstance(g, dict) else []:
                cmd = h.get("command", "") if isinstance(h, dict) else ""
                m = re.search(r"hooks[/\\]([A-Za-z0-9_.-]+)", cmd)
                if m:
                    out.add((ev, m.group(1)))
    return out


def listing(d):
    """디렉터리 목록 + 각 항목의 링크 대상(실제 ~/.local/bin 불변 확인용) — 없으면 None."""
    try:
        names = sorted(os.listdir(d))
    except OSError:
        return None
    out = []
    for n in names:
        p = os.path.join(d, n)
        try:
            out.append((n, os.readlink(p) if os.path.islink(p) else "file"))
        except OSError:
            out.append((n, "?"))
    return out


def replay_base():
    """격리 HOME 의 뿌리 — 임시 디렉터리 **밖**(preflight 는 임시 팩에서 훅을 등록하지 않는다 · AN5)."""
    td = os.environ.get("CARGO_TARGET_DIR")
    base = Path(td) / "t5a-boot-replay" if td else REPO / "target" / "t5a-boot-replay"
    base.mkdir(parents=True, exist_ok=True)
    return Path(os.path.realpath(tempfile.mkdtemp(prefix="r-", dir=str(base))))


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("sh"),
                     "macOS 전용(부트 스크립트의 건강한 기준선은 macOS 에서 쟀다)")
class BootstrapStartReplay(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.root = replay_base()
        self.addCleanup(shutil.rmtree, str(self.root), True)
        self.stub = self.root / "stub"
        self.stub.mkdir()
        (self.stub / "cys").write_text(STUB_CYS, encoding="utf-8")
        (self.stub / "cys").chmod(0o755)
        for t in STUB_TOOLS:
            (self.stub / t).write_text(STUB_TOOL, encoding="utf-8")
            (self.stub / t).chmod(0o755)
        surfaces = [{"role": r, "exited": False, "agent_alive": True,
                     "status": {"state": "working", "age_secs": 5}} for r in ROLES]
        (self.stub / "status.json").write_text(json.dumps({"surfaces": surfaces}), encoding="utf-8")
        (self.stub / "status-deficit.json").write_text(
            json.dumps({"surfaces": [s for s in surfaces if s["role"] != "worker"]}), encoding="utf-8")
        (self.stub / "detect.json").write_text(json.dumps({"agents": {
            "gemini": {"installed": True, "reason": "stub"}, "codex": {"installed": True, "reason": "stub"},
            "grok": {"installed": False, "reason": "stub"}, "claude": {"installed": True, "reason": "stub"}}}),
            encoding="utf-8")
        rows = ["surface:%d\trole=%s\tpid=%d\texited=false\n" % (10 + i, r, 1010 + i) for i, r in enumerate(ROLES)]
        (self.stub / "list.txt").write_text("".join(rows), encoding="utf-8")
        (self.stub / "list-deficit.txt").write_text("".join(r for r in rows if "role=worker" not in r),
                                                    encoding="utf-8")
        (self.stub / "boot.stdout").write_text(
            json.dumps({"roles": [], "summary": {"lock": "acquired", "fatal_failed": 0}}) + "\n", encoding="utf-8")
        self.trip = self.root / "trip.log"
        self.runs = 0
        self.leaks = []
        self.caller_home = os.path.expanduser("~")
        self.caller_local_bin = listing(os.path.join(self.caller_home, ".local", "bin"))

    def fresh_home(self):
        """새 HOME + 실 팩 사본(~/.cys/pack) + Claude 프로필 하나(AN5)."""
        self.runs += 1
        home = self.root / ("h%d" % self.runs)
        pack = home / ".cys" / "pack"
        copy_pack(str(pack))
        (home / ".claude").mkdir(parents=True)
        (home / ".claude" / "settings.json").write_text("{}\n", encoding="utf-8")
        return home, pack

    def boot_env(self, home):
        """고정 PATH(스텁 · 인터프리터 폴더 · 시스템 넷)와 격리 HOME 의 실행 환경(AN4)."""
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("CYS_", "JAVIS_", "AITERM_", "CLAUDE"))}
        tmpd = home / "tmp"
        tmpd.mkdir(exist_ok=True)
        py_dir = os.path.dirname(os.path.realpath(sys.executable))
        path = [str(self.stub), py_dir] + [d for d in FIXED_SYSTEM_PATH if d != py_dir]
        env.update({"HOME": str(home), "TMPDIR": str(tmpd), "T5A_STUB": str(self.stub),
                    "T5A_TRIP_LOG": str(self.trip), "PATH": os.pathsep.join(path),
                    "CYS_BIN": str(self.stub / "cys"),
                    "CYS_PREFLIGHT_HERMETIC_TEST": "1", "CYS_TEST_ISOLATED_HOME": str(home),
                    "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "2", "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                    "CYS_NO_AUTOSTART": "1", "CYS_NO_OFFICE_BRIDGE": "1",
                    "CYS_NO_PERSONAL_HOOK_MERGE": "1", "CYS_NO_AUTORESTORE": "1"})
        return env

    def run_boot(self, home, pack, scene="normal"):
        for f in ("booted", "deficit"):
            try:
                (self.stub / f).unlink()
            except FileNotFoundError:
                pass
        if scene == "deficit":
            (self.stub / "deficit").write_text("", encoding="utf-8")
        env = self.boot_env(home)
        t0 = time.time()
        try:
            r = subprocess.run([sys.executable, str(pack / "bin" / "javis_bootstrap.py"), "run"],
                               capture_output=True, text=True, env=env, cwd=str(home), timeout=900)
            rc, out, err = r.returncode, r.stdout, r.stderr
        except subprocess.TimeoutExpired as e:
            rc, out, err = None, str(e.stdout or ""), str(e.stderr or "")
        ok = rc == 0 and (home / ".cys" / ".master-bootstrapped").exists()
        self.scan_for_machine_cys(home, out + err)
        return ok, rc, time.time() - t0, err[-600:], registered_hooks(home)

    def scan_for_machine_cys(self, home, captured):
        """AN4 — 이 런이 남긴 글(출력 · 격리 HOME 의 텍스트 파일) 어디에도 기계의 cys 위치가 없다."""
        texts = [("output", captured)]
        for root_, dirs, files in os.walk(home):
            dirs[:] = [d for d in dirs if d != "pack"]
            for f in files:
                p = os.path.join(root_, f)
                if os.path.islink(p):          # e.g. ~/.local/bin/cys-dept → the pack's own script
                    continue
                try:
                    if os.path.getsize(p) <= 4_000_000:
                        with open(p, encoding="utf-8", errors="replace") as fh:
                            texts.append((p, fh.read()))
                except OSError:
                    pass
        machine = MACHINE_CYS + (os.path.join(self.caller_home, ".local", "bin", "cys"),)
        for where, t in texts:
            for m in machine:
                if m in t:
                    self.leaks.append("%s mentions %s" % (where, m))

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

    def give_stop_answer(self, pack, rel):
        scene, how, text, _why = STOP_ANSWERS[rel]
        p = pack / rel
        if how == "append":
            p.write_text(p.read_text(encoding="utf-8") + text, encoding="utf-8")
        else:
            p.write_text(text, encoding="utf-8")
        return scene

    def boot_with(self, scene, edit=None):
        home, pack = self.fresh_home()
        if scene == "deficit":
            (pack / "bin" / "javis_resource_gate.py").write_text(GATE_ALLOW, encoding="utf-8")
        if edit:
            edit(pack)
        return self.run_boot(home, pack, scene)

    def test_derivation_rule_at_the_bootstrap_layer(self):
        rows = pinned_rows()
        self.assertGreater(len(rows), 20, "PINNED 표를 읽지 못했다(파서가 표 형식과 어긋난다)")
        dropped = dropped_rows()
        self.assertGreater(len(dropped), 10, "DROPPED_CANDIDATES 표를 읽지 못했다")
        listed = {r for r, _ in rows}
        start_rows = [r for r, c in rows if c in ("StartScript", "StartDep", "Config")]
        self.assertEqual(set(start_rows), set(REAL_LEVEL) | BOOTSTRAP_LEVEL,
                         "기동 행이 RULINGS AL1 · AN2 의 갈래 2·3 과 다르다")
        self.assertFalse(listed & set(dropped), "목록과 뺀 후보가 겹친다")
        built = os.path.join(os.environ.get("CARGO_TARGET_DIR", ""), "debug", "cys")
        if os.environ.get("CARGO_TARGET_DIR") and os.path.isfile(built) and os.path.getsize(built) > 0:
            r = subprocess.run([built, "repair", "boot-critical", "--list", "--json"], capture_output=True,
                               text=True, timeout=30, env=dict(os.environ, CYS_NO_AUTOSTART="1",
                                                                HOME=str(self.root), CYS_SOCKET=str(self.root / "s")))
            if r.returncode == 0:
                doc = json.loads(r.stdout)
                got = {e["rel"] for e in doc["entries"]}
                self.assertEqual(got, listed, "PINNED 과 빌드된 본체의 목록이 다르다")
                real = {e["rel"] for e in doc["entries"] if e.get("proof") == "real-level"}
                boot = {e["rel"] for e in doc["entries"] if e.get("proof") == "bootstrap-replay"}
                self.assertEqual(real, set(REAL_LEVEL), "본체의 `proof: real-level` 행이 갈래 3 과 다르다")
                self.assertEqual(boot, BOOTSTRAP_LEVEL, "본체의 `proof: bootstrap-replay` 행이 갈래 2 와 다르다")
        matrix, problems = [], []

        def verdict(ok):
            return "COMPLETES" if ok else "STOPS"

        # ① 건강한 부트 · 훅 등록 기준선(AN5)
        ok, rc, dt, err, hooks0 = self.boot_with("normal")
        matrix.append("healthy: ok=%s rc=%s %.1fs · registered hooks %d" % (ok, rc, dt, len(hooks0 or ())))
        if not ok:
            problems.append("① 건강한 팩으로 부트가 끝나지 않았다: rc=%s %s" % (rc, err))
        # 기준선: preflight 가 스텁 cys 로도 등록하는 제품 훅 전부(능력 게이트는 데몬의 능력표를 읽은 뒤에만
        #   등록된다 — C28 `unknown` 은 등록도 해제도 하지 않는다 · 그래서 여기 기준선에 없다)
        names0 = {h for _e, h in (hooks0 or ())}
        if "session-start.sh" not in names0 or len(names0) < 10:
            problems.append("① 건강한 부트의 훅 등록이 비었다(AN5 기준선 불성립): %r" % sorted(hooks0 or ()))
        matrix.append("registered: %s" % ", ".join("%s:%s" % p for p in sorted(hooks0 or ())))

        # ①′ 결손 장면의 대조군(자원 게이트가 허용을 답한다)
        ok, rc, dt, err, hooks_d = self.boot_with("deficit")
        calls = (self.stub / "calls.log").read_text(encoding="utf-8") if (self.stub / "calls.log").exists() else ""
        matrix.append("deficit control: ok=%s rc=%s %.1fs · `cys boot` called=%s" % (ok, rc, dt, " boot" in calls))
        if not ok:
            problems.append("①′ 결손 장면(게이트 허용)에서 부트가 끝나지 않았다: rc=%s %s" % (rc, err))
        if " boot" not in calls:
            problems.append("①′ 결손 장면이 ④ 팀 기동 갈래에 닿지 않았다(`cys boot` 호출 없음)")
        if hooks_d != hooks0:
            problems.append("①′ 결손 장면의 훅 등록 집합이 기준선과 다르다: %r" % sorted((hooks_d or set()) ^ (hooks0 or set())))

        # 트립와이어 — 이 부트가 실제로 읽는 목록 파일 · 뺀 후보(두 장면)
        reads = {}
        for scene in ("normal", "deficit"):
            home, pack = self.fresh_home()
            if scene == "deficit":
                (pack / "bin" / "javis_resource_gate.py").write_text(GATE_ALLOW, encoding="utf-8")
            for rel in sorted(listed | set(dropped)):
                if (pack / rel).is_file() and not (scene == "deficit" and rel == "bin/javis_resource_gate.py"):
                    self.tripwire(pack, rel)
            if self.trip.exists():
                self.trip.unlink()
            ok, rc, dt, err, _h = self.run_boot(home, pack, scene)
            reads[scene] = set(self.trip.read_text(encoding="utf-8").split()) if self.trip.exists() else set()
            matrix.append("tripwire %s: ok=%s rc=%s %.1fs · reads %d listed + %d dropped"
                          % (scene, ok, rc, dt, len(reads[scene] & listed), len(reads[scene] & set(dropped))))
            if not ok:
                problems.append("트립와이어 부트(%s)가 끝나지 않았다(트립와이어 자체가 부트를 깼다?): rc=%s %s"
                                % (scene, rc, err))
        reads["deficit"].add("bin/javis_resource_gate.py")   # 결손 장면은 게이트를 돌린다(허용 사본 — 위 ①′)

        def scenes_reading(rel):
            return [s for s in ("normal", "deficit") if rel in reads[s] or rel.endswith(".json")]

        def broken(rel):
            return lambda pack: (pack / rel).write_text(broken_body(rel), encoding="utf-8")

        # ② 목록의 기동 행 — (i) 망가뜨림 · (ii) 멈춤 답
        for rel in sorted(start_rows):
            stopped_by = []
            for scene in scenes_reading(rel) or []:
                ok, rc, dt, err, _h = self.boot_with(scene, broken(rel))
                matrix.append("(i) break %s [%s] -> boot %s (rc=%s %.1fs)" % (rel, scene, verdict(ok), rc, dt))
                if not ok:
                    stopped_by.append("(i)/%s" % scene)
            if rel in STOP_ANSWERS:
                scene = STOP_ANSWERS[rel][0]
                ok, rc, dt, err, _h = self.boot_with(scene, lambda pack, r=rel: self.give_stop_answer(pack, r))
                matrix.append("(ii) %s answers stop [%s] -> boot %s (rc=%s %.1fs) · %s"
                              % (rel, scene, verdict(ok), rc, dt, STOP_ANSWERS[rel][3]))
                if not ok:
                    stopped_by.append("(ii)/%s" % scene)
            if not scenes_reading(rel) and rel not in STOP_ANSWERS:
                matrix.append("not exercised here: %s (5단계 종단 시험)" % rel)
            if rel in BOOTSTRAP_LEVEL and not stopped_by:
                problems.append("② %s 가 어느 갈래로도 이 층에서 부트를 멈추지 않았다 — 갈래 2 행은 멈춰야 한다"
                                "(RULINGS AL1 · AN2)" % rel)
            if rel in REAL_LEVEL and stopped_by:
                matrix.append("  note: %s (real-level) already stops the boot at this layer: %s"
                              % (rel, ", ".join(stopped_by)))

        # ③ 뺀 후보 — 이 부트가 읽는 것을 하나씩 (i), 멈춤 갈래가 있으면 (ii) · 훅 등록 완전(AN5)
        for rel in sorted(dropped):
            sc = scenes_reading(rel)
            if not sc and rel not in STOP_ANSWERS:
                matrix.append("dropped %s: never read by the boot (tripwire, both scenes)" % rel)
                continue
            for scene in sc:
                ok, rc, dt, err, hk = self.boot_with(scene, broken(rel))
                complete = hk == hooks0
                matrix.append("dropped (i) break %s [%s] -> boot %s (rc=%s %.1fs) · hooks %s"
                              % (rel, scene, verdict(ok), rc, dt, "complete" if complete else "INCOMPLETE"))
                if not ok:
                    problems.append("③ 뺀 후보 %s 를 망가뜨렸더니 부트가 멈췄다[%s] — 목록에 들어가야 한다(AN2 (i)): rc=%s %s"
                                    % (rel, scene, rc, err))
                if not complete:
                    problems.append("③ 뺀 후보 %s 를 망가뜨렸더니 훅 등록이 불완전하다[%s] — 목록에 들어가야 한다(AN5): %r"
                                    % (rel, scene, sorted((hk or set()) ^ (hooks0 or set()))))
            if rel in STOP_ANSWERS:
                scene = STOP_ANSWERS[rel][0]
                ok, rc, dt, err, hk = self.boot_with(scene, lambda pack, r=rel: self.give_stop_answer(pack, r))
                matrix.append("dropped (ii) %s [%s] -> boot %s (rc=%s)" % (rel, scene, verdict(ok), rc))
                if not ok:
                    problems.append("③ 뺀 후보 %s 의 멈춤 답에 부트가 멈췄다 — 목록에 들어가야 한다(AN2 (ii))" % rel)

        # ④ 목록 밖 파일 전부를 한꺼번에 망가뜨린다(두 장면) · 훅 등록 완전(AN5)
        for scene in ("normal", "deficit"):
            home, pack = self.fresh_home()
            if scene == "deficit":
                (pack / "bin" / "javis_resource_gate.py").write_text(GATE_ALLOW, encoding="utf-8")
            unlisted = []
            for root_, _dirs, files in os.walk(pack):
                for f in files:
                    rel = os.path.relpath(os.path.join(root_, f), pack).replace(os.sep, "/")
                    if rel not in listed and not (scene == "deficit" and rel == "bin/javis_resource_gate.py"):
                        unlisted.append(rel)
            for rel in unlisted:
                (pack / rel).write_text(broken_body(rel), encoding="utf-8")
            ok, rc, dt, err, hk = self.run_boot(home, pack, scene)
            complete = hk == hooks0
            matrix.append("break all %d unlisted files [%s] -> boot %s (rc=%s %.1fs) · hooks %s"
                          % (len(unlisted), scene, verdict(ok), rc, dt, "complete" if complete else "INCOMPLETE"))
            if not ok:
                problems.append("④ 목록 밖 파일을 망가뜨렸더니 부트가 실패했다[%s] — 목록이 빠뜨린 파일이 있다: rc=%s %s"
                                % (scene, rc, err))
            if not complete:
                problems.append("④ 목록 밖 파일을 망가뜨렸더니 훅 등록이 불완전하다[%s](AN5): %r"
                                % (scene, sorted((hk or set()) ^ (hooks0 or set()))))

        # ⑤ 밀폐(AN4) — C65 는 이 부트에서 cys 를 **절대 경로로 실행하는** 유일한 preflight 검사다: 같은 환경에서
        #   그 검사만 돌려(`--only C65`) 고른 cys 가 스텁인지 본다(스위치 + CYS_BIN — 기계의 /Applications 후보 생략)
        home, pack = self.fresh_home()
        r = subprocess.run([sys.executable, str(pack / "bin" / "javis_preflight.py"), "--only", "C65"],
                           capture_output=True, text=True, env=self.boot_env(home), cwd=str(home), timeout=120)
        c65 = [ln for ln in (r.stdout + r.stderr).splitlines() if "C65" in ln]
        matrix.append("C65 alone: %s" % (" / ".join(c65)[:300] or "(no row)"))
        if not any(str(self.stub / "cys") in ln for ln in c65) or any(m in r.stdout + r.stderr for m in MACHINE_CYS):
            problems.append("⑤ C65 가 스텁이 아닌 cys 를 골랐다(AN4): %r" % c65)
        calls = (self.stub / "calls.log").read_text(encoding="utf-8").splitlines() \
            if (self.stub / "calls.log").exists() else []
        stub_prefix = str(self.stub) + os.sep
        foreign = [c for c in calls if not c.startswith(stub_prefix)]
        matrix.append("stub calls: %d (cys %d · install tools %d) · foreign %d · machine-cys mentions %d"
                      % (len(calls), sum(1 for c in calls if c.startswith(stub_prefix + "cys ")),
                         sum(1 for c in calls if c.split(" ")[0].rsplit("/", 1)[-1] in STUB_TOOLS),
                         len(foreign), len(self.leaks)))
        if foreign:
            problems.append("⑤ 스텁 기록에 스텁 밖 실행 경로가 있다(AN4): %r" % foreign[:5])
        if self.leaks:
            problems.append("⑤ 기계의 cys 위치가 이 런의 글에 나온다(AN4 — 기계의 cys 를 집었다): %r" % self.leaks[:5])
        after = listing(os.path.join(self.caller_home, ".local", "bin"))
        matrix.append("caller ~/.local/bin unchanged: %s" % (after == self.caller_local_bin))
        if after != self.caller_local_bin:
            problems.append("⑤ 호출자의 ~/.local/bin 목록이 바뀌었다(AN4): before=%r after=%r"
                            % (self.caller_local_bin, after))

        report = "\n".join(matrix) + "\nPROBLEMS:\n" + "\n".join(problems) + "\n"
        td = os.environ.get("CARGO_TARGET_DIR")
        if td and os.path.isdir(td):
            Path(td, "t5a-bootstrap-replay.txt").write_text(report, encoding="utf-8")
        print(report)
        self.assertEqual(problems, [], report)


if __name__ == "__main__":
    unittest.main(verbosity=2)
