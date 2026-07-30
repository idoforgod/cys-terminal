#!/usr/bin/env python3
"""run_bootstrap_health.py — 부트스트랩 상시 건강성 1커맨드 게이트 (T-0147-7 재감사 §5).

    python3 <pack>/bin/tests/run_bootstrap_health.py            # exit 0 = GREEN
    python3 <pack>/bin/tests/run_bootstrap_health.py --json     # 기계 판독
    python3 <pack>/bin/tests/run_bootstrap_health.py --list      # 검체 대장(발효 웨이브 포함)
    python3 <pack>/bin/tests/run_bootstrap_health.py --only H-WIN-1,H-CONC-6
    python3 <pack>/bin/tests/run_bootstrap_health.py --wave W1a  # 특정 웨이브 발효분만

설계(재감사 §5):
  · **발효 시점표**: 검체마다 발효 웨이브 태그가 붙는다. 아직 안 착지한 웨이브의 검체는
    `pending`(회색·게이트 비산입)이고, **발효된 검체의 미구현·실패는 hard fail** 이다.
    '1커맨드 게이트'의 정의 = "현재 발효분 전량 GREEN". 이 규약이 없으면 러너가 W4 완료까지
    상시 적색이어서 릴리스 게이트로 성립하지 않는다.
  · 발효 웨이브 집합은 `LANDED_WAVES` 단일 상수다 — 웨이브가 착지하면 여기만 늘린다.
  · **플랫폼 분기는 스텁 목**(cygpath 목·백슬래시 fixture·드라이브 경로 fixture·lsof 목)으로
    macOS에서도 결정론 실행한다. Windows 실기 재실행은 H-WIN-11(W4·CI 잡) 소속.
  · 각 검체는 커버하는 **원 결함 ID** 를 동봉한다 — 회귀 시 결함 대장으로 즉시 역추적.
  · **계측기 자기검증**(MEMORY '디버깅 계측 타당성 게이트'): 정적 술어 검체는 가능하면
    `git show <기준커밋>:<파일>` 에 같은 탐지기를 돌려 **구 코드에서 FIRE 하는지** 확인한다.
    탐지기가 구 결함을 못 잡으면 신 코드의 PASS 는 아무 의미가 없다.

stdlib만 사용. 네트워크 0. 어떤 검체도 사용자 HOME·실 데몬을 건드리지 않는다(전부 격리 tmp).
"""
import argparse
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time

# ── 경로 해소 ────────────────────────────────────────────────────────────────
TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.dirname(TESTS_DIR)
PACK_DIR = os.path.dirname(BIN_DIR)
HOOKS_DIR = os.path.join(PACK_DIR, "hooks")
REPO_DIR = os.path.dirname(PACK_DIR)          # 레포 체크아웃일 때만 유효(배포 팩은 무의미)
PY = sys.executable or "python3"

# 기준 커밋 핀(재감사 헤더) — 계측기 자기검증(구 코드 FIRE 확인)의 대조 트리.
# W0 착지 커밋을 쓴다: W1a 이전 상태이면서 W0 지혈이 반영된 트리.
CALIBRATION_REF = os.environ.get("CYS_HEALTH_CALIB_REF", "a96d8b1")

# ★발효 웨이브 — 착지한 웨이브만 넣는다. 미발효 검체는 pending(게이트 비산입).
LANDED_WAVES = ("W0", "W1a")

_REG = []          # [(id, wave, title, defects, fn|None)]


def specimen(sid, wave, title, defects):
    def deco(fn):
        _REG.append((sid, wave, title, defects, fn))
        return fn
    return deco


def pending(sid, wave, title, defects):
    """미발효(또는 미구현) 검체를 대장에만 등록한다 — 커버리지 공백을 숨기지 않는다."""
    _REG.append((sid, wave, title, defects, None))


class Fail(AssertionError):
    pass


class Skip(Exception):
    """검체 적용 불가(배포 팩에 Rust 소스 부재 등) — fail 아님."""


def need(cond, msg):
    if not cond:
        raise Fail(msg)


# ── 공용 하네스 ──────────────────────────────────────────────────────────────
def _w(path, body, mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def _run(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    kw.setdefault("encoding", "utf-8")
    kw.setdefault("errors", "replace")
    kw.setdefault("timeout", 120)
    return subprocess.run(cmd, **kw)


def _read(path, limit=400000):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read(limit)
    except OSError:
        return ""


def _hook(name):
    p = os.path.join(HOOKS_DIR, name)
    need(os.path.isfile(p), "훅 부재: %s" % p)
    return p


def _shell_hooks():
    """훅 .sh 전수 — `_lib.sh`(프리루드)는 훅이 아니라 라이브러리이므로 제외한다.
    (G-SYNTAX 는 프리루드를 별도로 명시 추가해 문법 검사한다.)"""
    out = []
    for root, _dirs, files in os.walk(HOOKS_DIR):
        for f in sorted(files):
            if f.endswith(".sh") and f != "_lib.sh":
                out.append(os.path.relpath(os.path.join(root, f), HOOKS_DIR))
    return sorted(out)


def _git_show(relpath):
    """기준 커밋의 파일 내용(계측기 자기검증용). 레포가 아니거나 실패면 None."""
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        return None
    r = _run(["git", "-C", REPO_DIR, "show", "%s:%s" % (CALIBRATION_REF, relpath)], timeout=30)
    return r.stdout if r.returncode == 0 else None


def _mock_cys(bindir, tmp, extra_case=""):
    """호출을 calls.log 에 적재하는 스텁 cys. extra_case = 추가 서브커맨드 분기 sh 코드."""
    _w(os.path.join(bindir, "cys"),
       "#!/bin/sh\n"
       'printf "%s\\n" "cys $*" >> "{log}"\n'
       "{extra}\n"
       "exit 0\n".format(log=os.path.join(tmp, "calls.log"), extra=extra_case))


def _calls(tmp):
    return _read(os.path.join(tmp, "calls.log"))


def _base_env(extra=None, drop=()):
    env = dict(os.environ)
    for k in ("CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_SOCKET", "CYS_PACK_DIR",
              "CYS_ROLE", "CYS_STATE_DIR", "CYS_LOCK_BACKEND", "CYS_ROOT", "CYS_SOUL"):
        env.pop(k, None)
    for k in drop:
        env.pop(k, None)
    if extra:
        env.update(extra)
    return env


def _rb_sandbox(tmp, *, boot_body=None, surface=True, mock_py=None, pack_has_boot=True):
    """role-bootstrap.sh 격리 실행 환경. 반환: (env, home, pack, bindir, state_dir)."""
    home = os.path.join(tmp, "home")
    pack = os.path.join(tmp, "pack")
    bindir = os.path.join(tmp, "bin")
    state = os.path.join(home, ".cys", "state")
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    if pack_has_boot:
        _w(os.path.join(pack, "bin", "javis_bootstrap.py"),
           boot_body if boot_body is not None else "print('MOCK-BOOT')\n", 0o644)
    _mock_cys(bindir, tmp, 'case "$1" in surface-role) echo ""; exit 0;; esac')
    if mock_py:
        _w(os.path.join(bindir, "python3"), mock_py)
    env = _base_env({"HOME": home, "CYS_PACK_DIR": pack,
                     "PATH": bindir + os.pathsep + os.environ.get("PATH", "")})
    if surface:
        env["CYS_SURFACE_ID"] = "7"
    return env, home, pack, bindir, state


def _run_rb(env, prompt="너는 마스터다"):
    return _run(["bash", _hook("role-bootstrap.sh")], input=json.dumps({"prompt": prompt}), env=env)


def _run_ledgers(state, timeout=8.0):
    """런별 발화 로그 목록(백그라운드 기록 완료까지 bounded 대기)."""
    end = time.time() + timeout
    while time.time() < end:
        names = sorted(n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)) \
            if os.path.isdir(state) else []
        if names and all(os.path.getsize(os.path.join(state, n)) > 0 for n in names):
            return names
        time.sleep(0.1)
    return sorted(n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)) \
        if os.path.isdir(state) else []


# ═══════════════════════════════════════════════════════════════════════════
# 0. 베이스라인 (결정론 회귀 — 재감사 부채 V3)
# ═══════════════════════════════════════════════════════════════════════════
def _baseline(cmd, label):
    r = _run(cmd, timeout=300)
    need(r.returncode == 0,
         "%s 비0 종료(exit=%d)\n--- stdout ---\n%s\n--- stderr ---\n%s"
         % (label, r.returncode, r.stdout[-1500:], r.stderr[-1500:]))
    return (r.stdout.strip().splitlines() or [""])[-1][:200]


@specimen("B-1", "W0", "베이스라인 test_role_bootstrap_hook", ["A2", "A4", "A3=B7"])
def b1():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_role_bootstrap_hook.py")], "test_role_bootstrap_hook")


@specimen("B-2", "W0", "베이스라인 test_bootstrap_chain", ["부트 체인 exit 계약"])
def b2():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_bootstrap_chain.py")], "test_bootstrap_chain")


@specimen("B-3", "W0", "베이스라인 test_import_guard", ["형제 모듈 import 가드"])
def b3():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_import_guard.py")], "test_import_guard")


@specimen("B-4", "W0", "베이스라인 javis_bootstrap --self-test", ["레인 격리·결손 판정"])
def b4():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_bootstrap.py"), "--self-test"], "bootstrap --self-test")


@specimen("B-5", "W0", "베이스라인 javis_boot_node --self-test", ["생존 술어·회수 판정"])
def b5():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_boot_node.py"), "--self-test"], "boot_node --self-test")


@specimen("B-6", "W0", "베이스라인 test_dept_doctrine_v1", ["부서 교리 v1"])
def b6():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_dept_doctrine_v1.py")], "test_dept_doctrine_v1")


@specimen("B-7", "W1a", "javis_lock --self-test (공용 락·원자쓰기)", ["A8py", "R1"])
def b7():
    p = os.path.join(BIN_DIR, "javis_lock.py")
    need(os.path.isfile(p), "javis_lock.py 부재 — A8py 미착지")
    return _baseline([PY, p, "--self-test"], "javis_lock --self-test")


# ═══════════════════════════════════════════════════════════════════════════
# 1. 러너 게이트 검체 (G-* — §5 ID 공간이 아닌 러너 로컬 게이트)
# ═══════════════════════════════════════════════════════════════════════════
@specimen("G-SYNTAX", "W1a", "전 훅 + 프리루드 `sh -n`·`bash -n` 무오류", ["CS-4① ⓐ POSIX sh"])
def g_syntax():
    targets = ["_lib.sh"] + _shell_hooks()
    bad = []
    for rel in targets:
        p = os.path.join(HOOKS_DIR, rel)
        for sh in ("sh", "bash"):
            r = _run([sh, "-n", p], timeout=30)
            if r.returncode != 0:
                bad.append("%s(%s): %s" % (rel, sh, r.stderr.strip()[:200]))
    need(not bad, "구문 오류:\n  " + "\n  ".join(bad))
    return "%d 파일 × sh/bash 문법 OK" % len(targets)


@specimen("G-PRELUDE", "W1a", "프리루드 계약 — 함수 표면·stdout 무출력·0 종료·loud-skip", ["CS-4① ⓑⓒⓓⓔ"])
def g_prelude():
    lib = os.path.join(HOOKS_DIR, "_lib.sh")
    need(os.path.isfile(lib), "_lib.sh 부재 — 프리루드 미착지")
    fns = ["cys_require_surface", "cys_have_surface", "cys_norm_path", "cys_is_abs",
           "cys_norm_cwd", "cys_path_has_prefix", "cys_native_path", "cys_shquote",
           "cys_resolve_py", "cys_fix_locale"]
    body = _read(lib)
    missing = [f for f in fns if ("%s()" % f) not in body]
    need(not missing, "프리루드 함수 누락: %s" % missing)
    # bashism 금지(ⓐ): 배열·[[·${x:0:n}·local·+= 는 sh 에서 깨진다.
    # ★주석 제외 — 문서화 목적으로 금지 토큰을 **인용**한 줄까지 잡으면 계측기 오탐이다.
    code = "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))
    for pat, why in ((r"\[\[", "[[ (bash 전용 조건)"), (r"\blocal\s", "local (POSIX 미정의)"),
                     (r"\$\{[A-Za-z_][A-Za-z0-9_]*:\d+:", "${x:0:n} 슬라이스"),
                     (r"\)\s*=\s*\(", "배열 대입")):
        need(not re.search(pat, code), "프리루드 bashism 발견: %s" % why)
    # stdout 무출력 + set -u 안전 + 0 종료 (ⓑⓒⓓ)
    with tempfile.TemporaryDirectory() as tmp:
        probe = os.path.join(tmp, "probe.sh")
        _w(probe, '#!/bin/sh\nset -u\n. "%s"\nprintf "RC=%%s\\n" "$?" >&2\n' % lib)
        r = _run(["sh", probe], env=_base_env())
        need(r.stdout == "", "프리루드가 stdout에 출력했다(모델 컨텍스트 오염): %r" % r.stdout[:200])
        need("RC=0" in r.stderr, "프리루드 source 가 0으로 끝나지 않는다: %r" % r.stderr[:200])
        # ── 2단 해소: 훅이 팩 밖으로 복사돼 실행돼도 CYS_PACK_DIR 경로에서 프리루드를 집는다 ──
        # ★이 폴백이 없으면 배선 하네스·테스트 스텁·local/hooks 오버레이가 전면 강등된다
        #   (실측 회귀: test_pre_dispatch.sh 56/56 → 10/56). G-DISPATCH 가 그 표면을 지킨다.
        fake = os.path.join(tmp, "fakehooks")
        os.makedirs(fake, exist_ok=True)
        shutil.copy2(_hook("save-state.sh"), os.path.join(fake, "save-state.sh"))
        fakepack = os.path.join(tmp, "fakepack")
        os.makedirs(os.path.join(fakepack, "hooks"), exist_ok=True)
        shutil.copy2(lib, os.path.join(fakepack, "hooks", "_lib.sh"))
        proj2 = os.path.join(tmp, "p2")
        os.makedirs(os.path.join(proj2, "_round"), exist_ok=True)
        r1 = _run(["bash", os.path.join(fake, "save-state.sh")],
                  input=json.dumps({"cwd": proj2, "hook_event_name": "Stop"}),
                  env=_base_env({"HOME": os.path.join(tmp, "h2"), "CYS_PACK_DIR": fakepack}))
        need(r1.returncode == 0, "2단 폴백 경로에서 훅이 비0 종료: %d" % r1.returncode)
        need("_lib.sh 소실" not in r1.stderr,
             "팩 경로에 프리루드가 있는데도 loud-skip 했다(2단 폴백 미작동): %r" % r1.stderr[:300])
        need("Stop" in _read(os.path.join(proj2, "_round", ".state_log")),
             "2단 폴백 경로에서 훅 본체가 동작하지 않았다")
        # loud-skip 규약: **양 단계 모두** 실패 → stderr 1줄 + exit 0 + stdout 무오염
        r2 = _run(["bash", os.path.join(fake, "save-state.sh")], input="{}",
                  env=_base_env({"HOME": os.path.join(tmp, "h3"),
                                 "CYS_PACK_DIR": os.path.join(tmp, "nowhere")}))
        need(r2.returncode == 0, "loud-skip 이 exit 0 아님: %d" % r2.returncode)
        need("_lib.sh 소실" in r2.stderr, "loud-skip stderr 문구 부재: %r" % r2.stderr[:200])
        need(r2.stdout == "", "loud-skip 이 stdout 오염: %r" % r2.stdout[:200])
        # 전 훅이 2단 해소 규약을 쓰는가(1단만 남은 훅 = 복사 배선에서 조용히 강등)
        no2 = [h for h in _shell_hooks()
               if "_lib.sh" in _read(os.path.join(HOOKS_DIR, h))
               and 'CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh' not in _read(os.path.join(HOOKS_DIR, h))]
        need(not no2, "2단 폴백 미적용 훅(복사 배선에서 전면 강등): %s" % no2)
    # ── 프리루드 요구 범위 = **규약을 소비하는 훅 전수** ──
    # G22 소인 범위는 "python 인터프리터를 쓰거나 경로/surface 술어를 쓰는 훅"이다. 전 .sh 파일을
    # 무조건 요구하면 **의존이 0인 훅**(cys-hook.sh·cys-statusline.sh — "cys 자기완결·python 등
    # 외부 의존 없음"이 명문 계약)까지 새 실패 모드를 심는 과확장이 된다. 두 훅은 의도적 제외이며,
    # 나중에 python/경로 술어를 쓰게 되면 아래 술어가 자동으로 계약 안으로 끌어들인다.
    consumers, nosrc = [], []
    markers = ("CYS_PY", "PYBIN", "python3", "cys_norm_", "cys_is_abs", "cys_native_path",
               "cys_require_surface", "cys_have_surface", "cys_path_has_prefix", "cys_shquote")
    for h in _shell_hooks():
        body_h = _read(os.path.join(HOOKS_DIR, h))
        if any(m in body_h for m in markers):
            consumers.append(h)
            if "_lib.sh" not in body_h:
                nosrc.append(h)
    need(not nosrc, "규약 소비 훅인데 프리루드 미적용: %s" % nosrc)
    need(len(consumers) >= 28,
         "규약 소비 훅 인벤토리가 28 미달(%d) — G22 전수 범위 축소 회귀" % len(consumers))
    return ("함수 %d종 · stdout 무출력 · loud-skip · 규약 소비 훅 %d개 전수 source(의존 0 훅 %d개 제외)"
            % (len(fns), len(consumers), len(_shell_hooks()) - len(consumers)))


@specimen("G-DISPATCH", "W1a", "pre-dispatch 서브훅 배선 회귀(팩 밖 복사 실행 표면)",
          ["CS-4① ⓔ 2단 해소", "pre-dispatch 계약"])
def g_dispatch():
    """★이 게이트가 존재하는 이유(실사고): 프리루드 1단 해소만 두었을 때, 훅을 팩 밖으로
    **복사해 실행하는** 배선(테스트 스텁·local/hooks 오버레이·배선 하네스)이 전면 강등돼
    56/56 → 10/56 로 무너졌다. 그 표면을 지키는 유일한 검체다."""
    h = os.path.join(HOOKS_DIR, "test_pre_dispatch.sh")
    if not os.path.isfile(h):
        raise Skip("test_pre_dispatch.sh 부재")
    env = _base_env({"REAL_GUARD": _hook("guard.sh"), "REAL_HOOKS": HOOKS_DIR})
    r = _run(["sh", h], env=env, timeout=300)
    m = re.search(r"결과: PASS=(\d+) FAIL=(\d+)", r.stdout)
    need(m, "하네스 결과 라인 부재(FATAL?): %r" % (r.stdout[-500:] or r.stderr[-500:]))
    p, f = int(m.group(1)), int(m.group(2))
    need(f == 0, "서브훅 배선 회귀: PASS=%d FAIL=%d\n%s" % (p, f, "\n".join(
        l for l in r.stdout.splitlines() if "FAIL" in l)[:2000]))
    need(p >= 50, "하네스 케이스 수 급감(%d) — 조기 강등 의심" % p)
    return "pre-dispatch 하네스 %d/%d PASS(guard·cys-hook·appbuild·grill 배선 무회귀)" % (p, p + f)


@specimen("G-SMOKE", "W1a", "회귀 스모크 — 대표 3훅 정상 경로 종전 거동 유지",
          ["inject-context", "save-state", "guard"])
def g_smoke():
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"),
           "# S\n> 최종 갱신: old\nSMOKE-STATE-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        payload = json.dumps({"source": "clear", "cwd": proj, "hook_event_name": "PreCompact"})
        # ① inject-context: 작업기억 주입 + exit 0
        r = _run(["bash", _hook("inject-context.sh")], input=payload, env=env)
        need(r.returncode == 0, "inject-context exit=%d" % r.returncode)
        need("SMOKE-STATE-MARKER" in r.stdout, "inject-context 가 작업기억을 주입하지 않았다")
        notes.append("inject-context OK")
        # ② save-state: .state_log append + 타임스탬프 갱신
        r = _run(["bash", _hook("save-state.sh")], input=payload, env=env)
        need(r.returncode == 0, "save-state exit=%d" % r.returncode)
        log = _read(os.path.join(proj, "_round", ".state_log"))
        need("PreCompact" in log, "save-state 가 .state_log 를 남기지 않았다: %r" % log[:200])
        need("auto write-ahead" in _read(os.path.join(proj, "_round", "SESSION_STATE.md")),
             "save-state 가 '최종 갱신' 타임스탬프를 갱신하지 않았다")
        notes.append("save-state OK")
        # ③ guard: 무해 명령 통과 / 헌법파일 차단
        r = _run(["bash", _hook("guard.sh")],
                 input=json.dumps({"tool_name": "Bash", "tool_input": {"command": "ls -la"}}), env=env)
        need(r.returncode == 0, "guard 가 무해 명령을 차단(exit=%d)" % r.returncode)
        r = _run(["bash", _hook("guard.sh")],
                 input=json.dumps({"tool_name": "Write",
                                   "tool_input": {"file_path": "/x/.claude/soul.md"}}), env=env)
        need(r.returncode == 2, "guard 가 헌법파일 쓰기를 통과(exit=%d)" % r.returncode)
        notes.append("guard OK")
    return " · ".join(notes)


# ═══════════════════════════════════════════════════════════════════════════
# 2. H-DETECT (훅 발화 계층)
# ═══════════════════════════════════════════════════════════════════════════
pending("H-DETECT-1", "W1b", "혼합의도 FIRE corpus(절 경계 스코프)", ["A4"])
pending("H-DETECT-2", "W1b", "'네가/니가/당신이 마스터다' FIRE + 인접 의문 SKIP", ["P3-A-NEGA"])
pending("H-DETECT-3", "W1b", "filler 경계 15자=발화/16자=미발화", ["P3-A-FILLER"])
pending("H-DETECT-4", "W1b", "LC_ALL=C 파리티(UTF-8과 동일 판정)", ["G9"])
pending("H-DETECT-5", "W1b", "200자 창 python측 문자 단위 슬라이스", ["G25"])


@specimen("H-DETECT-6", "W1a", "무-surface env → 무발화·무부작용", ["A2"])
def h_detect_6():
    with tempfile.TemporaryDirectory() as tmp:
        env, home, _pack, _bind, state = _rb_sandbox(tmp, surface=False)
        r = _run_rb(env)
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need(r.stdout.strip() == "", "무-surface 에서 컨텍스트를 주입했다: %r" % r.stdout[:300])
        need("발화됨" not in r.stdout, "무-surface 에서 발화 보고")
        # 무부작용: cys 왕복 0(surface-role 조회조차 없음)·상태 디렉터리 무생성
        need(_calls(tmp) == "", "무-surface 에서 cys 를 호출했다(데몬 autostart 표면): %r" % _calls(tmp))
        need(not os.path.isdir(state) or not os.listdir(state),
             "무-surface 에서 상태 디렉터리를 건드렸다: %s" % (os.listdir(state) if os.path.isdir(state) else ""))
        # 대조군: surface 있으면 발화(게이트가 과차단이 아님을 증명)
        env2, _h2, _p2, _b2, _s2 = _rb_sandbox(os.path.join(tmp, "b"), surface=True)
        r2 = _run_rb(env2)
        need("발화됨" in r2.stdout, "surface 있는 정상 경로가 발화하지 않았다(과차단): %r" % r2.stdout[:300])
    # 계측기 자기검증: 구 코드(기준 커밋)에는 게이트가 없어야 한다
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need("cys_require_surface" not in old,
             "계측 타당성 실패: 구 코드에 이미 게이트가 있다면 이 검체는 아무것도 증명하지 않는다")
        calib = "구 코드 게이트 부재 확인"
    return "무발화·cys왕복0·상태무접촉 + 대조군 발화 · 계측검증=%s" % calib


pending("H-DETECT-7", "W1b", "role 게이트 allowlist 행렬(worker-2·cso-1·미지 role)", ["A3=B7"])
pending("H-DETECT-8", "W1b", "surface-role 판정불가 → fail-closed+loud", ["A5"])
pending("H-DETECT-9", "W1b", "python 부재 → 정적 loud(cannot-judge 분리)", ["A22"])


@specimen("H-DETECT-10", "W1a", "pre-exec 사망 목 → '발화 실패' 상태파생 보고(허위 '발화됨' 금지)", ["A6"])
def h_detect_10():
    real = shutil.which("python3") or PY
    # 목 인터프리터: `-c`(프롬프트 파싱·NOTE 생성)는 정상 위임, **스크립트 실행은 exec 실패(127)**.
    mock = ("#!/bin/sh\n"
            'case "$1" in\n'
            '  -c) exec "%s" "$@" ;;\n'
            "esac\n"
            "exit 127\n" % real)
    with tempfile.TemporaryDirectory() as tmp:
        env, _home, _pack, _bind, state = _rb_sandbox(tmp, mock_py=mock)
        r = _run_rb(env)
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need("발화됨" not in r.stdout, "발화가 죽었는데 '발화됨' 을 보고했다(허위 보고): %r" % r.stdout[:400])
        need("발화 실패" in r.stdout, "'발화 실패' 상태파생 보고가 없다: %r" % r.stdout[:400])
        need("role-bootstrap-" in r.stdout, "실패 보고에 런 로그 경로 안내가 없다: %r" % r.stdout[:400])
        need("exit 127" in r.stdout, "실패 사유(exit code)가 보고에 없다: %r" % r.stdout[:400])
    # 계측기 자기검증: 구 코드는 같은 목에서 '발화됨' 을 말해야 한다(결함 재현)
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    if old is not None:
        with tempfile.TemporaryDirectory() as tmp2:
            oldhook = os.path.join(tmp2, "hooks", "role-bootstrap.sh")
            _w(oldhook, old)
            _w(os.path.join(tmp2, "hooks", "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            env2, _h, _p, _b, _s = _rb_sandbox(os.path.join(tmp2, "sb"), mock_py=mock)
            r2 = _run(["bash", oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=env2)
            need("발화됨" in r2.stdout,
                 "계측 타당성 실패: 구 코드가 이 목에서 허위 '발화됨' 을 내지 않는다 — 목이 결함을 재현하지 못함")
            calib = "구 코드 허위 '발화됨' 재현 확인"
    return "발화 실패 보고·로그 경로 동봉 · 계측검증=%s" % calib


@specimen("H-DETECT-11", "W1a", "동시/연속 발화에서 선행 런 로그 무-truncate(런별 파일+상한)", ["A16", "R3"])
def h_detect_11():
    with tempfile.TemporaryDirectory() as tmp:
        boot = ("import os,sys\n"
                "c=os.path.join(os.environ['HOME'],'.cys','state','n')\n"
                "n=int(open(c).read()) if os.path.exists(c) else 0\n"
                "open(c,'w').write(str(n+1))\n"
                "sys.stdout.write('RUN-MARKER-%d\\n' % n)\n")
        env, home, _pack, _bind, state = _rb_sandbox(tmp, boot_body=boot)
        os.makedirs(state, exist_ok=True)
        for i in (1, 2):
            r = _run_rb(env)
            need("발화됨" in r.stdout, "%d회차 발화 실패: %r" % (i, r.stdout[:200]))
            time.sleep(0.2)
        names = _run_ledgers(state)
        need(len(names) >= 2, "런별 로그가 분리되지 않았다(truncate 회귀): %s" % names)
        bodies = [_read(os.path.join(state, n)) for n in names]
        markers = sorted(set(re.findall(r"RUN-MARKER-\d+", "\n".join(bodies))))
        need(len(markers) >= 2, "선행 런 내용이 소실됐다(상호 truncate): %s / %s" % (names, bodies))
        need(os.path.exists(os.path.join(state, "role-bootstrap-latest.log")),
             "latest 포인터가 없다")
        need(not os.path.exists(os.path.join(state, "role-bootstrap.log")),
             "구 단일 truncate 로그가 여전히 생성된다(A16 미수리)")
        # 개수 상한(최근 10개 유지) — 더미 12개 선주입 후 1회 발화
        for k in range(12):
            _w(os.path.join(state, "role-bootstrap-100000%02d-999.log" % k), "old-%d\n" % k, 0o644)
        _run_rb(env)
        left = [n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)]
        need(len(left) <= 10, "개수 상한 미작동(%d개 잔여)" % len(left))
    return "런별 %d파일 분리·마커 %d종 보존·latest 포인터·상한 10 준수" % (len(names), len(markers))


# ═══════════════════════════════════════════════════════════════════════════
# 3. H-EXIT (종료·판정 계약) — W1a 발효분은 A15/R2 하나
# ═══════════════════════════════════════════════════════════════════════════
pending("H-EXIT-1", "W1b", "cmd_run 종료 불변식(stdout JSON / stderr verdict·run_id)", ["A7", "A19"])
pending("H-EXIT-2", "W2", "boot Busy → busy 판정 구분 + CEO 티켓 보존", ["G11"])
pending("H-EXIT-3", "W2", "claim-role 타입드 exit 표(0/7/3/2)", ["A20"])
pending("H-EXIT-4", "W2", "boot --json 스키마(mandatory·outcome·install_hint)", ["G29", "B15", "B16", "R5"])
pending("H-EXIT-5", "W2", "미지 subcommand EX_USAGE + 게이트 exit 2 충돌 해소", ["A13", "A14"])
pending("H-EXIT-6", "W2", "orchestra 2/127=영구실패 · 124=재시도 분류", ["A12"])
pending("H-EXIT-7", "W2", "check exit 2(데몬 소실) 별도 분기", ["G32"])
pending("H-EXIT-8", "W2", "discover sentinel(격리 팩 vs 미발견 분리)", ["G1"])
pending("H-EXIT-9", "W1b", "싱글플라이트 패자 → 비-master skip verdict(즉시 반환)", ["G17"])


@specimen("H-EXIT-10", "W1a", "②ping 실패에도 알림 시도 발생(카브아웃 제거·notifier 단일화)", ["A15", "R2"])
def h_exit_10():
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = os.path.join(home, ".cys", "pack")
        bindir = os.path.join(tmp, "stubbin")
        os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
        os.makedirs(bindir, exist_ok=True)
        # 스텁 cys: ping 실패(exit 1) · 그 외(feed push·send)는 성공 + 호출 적재
        _mock_cys(bindir, tmp, 'case "$1" in ping) exit 1;; --version) echo "cys 0.0.0-stub"; exit 0;; esac')
        _w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
        _w(os.path.join(pack, "bin", "javis_orchestra.py"), "import sys; sys.exit(0)\n", 0o644)
        env = _base_env({"HOME": home, "PATH": bindir + os.pathsep + os.environ.get("PATH", ""),
                         "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "1",
                         "CYS_BOOT_CHECK_INTERVAL_S": "0.05"})
        r = _run([PY, os.path.join(BIN_DIR, "javis_bootstrap.py")], env=env, timeout=180)
        need(r.returncode == 3, "②ping 실패의 exit 계약(3) 이탈: %d\n%s" % (r.returncode, r.stderr[-800:]))
        calls = _calls(tmp)
        need("feed push" in calls or "send --queued" in calls,
             "②ping 실패에서 알림 시도가 전무하다(카브아웃 잔존): %r" % calls[-600:])
        bl = json.loads(_read(os.path.join(home, ".cys", "state", "boot-last.json")) or "{}")
        res = bl.get("result") or {}
        need(res.get("failed_step") == "②ping", "boot-last 실패 단계 기록 이상: %r" % res)
        need((res.get("notify") or {}).get("attempted") is True,
             "알림 시도가 상태로 기록되지 않았다(보고=실측 위반): %r" % res)
        chan = (res.get("notify") or {}).get("channel")
    # 계측기 자기검증: 구 코드는 ②ping 카브아웃으로 알림을 건너뛴다
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    if old is not None:
        need('if name != "②ping"' in old,
             "계측 타당성 실패: 구 코드에 ②ping 카브아웃이 없으면 이 검체는 아무것도 증명하지 않는다")
        calib = "구 코드 카브아웃 존재 확인"
    return "exit 3 · 알림 채널=%s · boot-last notify 기록 · 계측검증=%s" % (chan, calib)


# ═══════════════════════════════════════════════════════════════════════════
# 4. H-CONC (동시성)
# ═══════════════════════════════════════════════════════════════════════════
_LOCK_PROBE = (
    "import sys, os\n"
    "sys.path.insert(0, sys.argv[1])\n"
    "import javis_lock as L\n"
    "lk = L.FileLock(sys.argv[2], owner=sys.argv[3])\n"
    "st = lk.acquire()\n"
    "print('%s|%s|%s' % (st, lk.backend, lk.reclaimed_stale))\n"
    "sys.stdout.flush()\n"
    "if st == L.ACQUIRED and len(sys.argv) > 4:\n"
    "    import time; time.sleep(float(sys.argv[4]))\n"
)


@specimen("H-CONC-1", "W1a", "동시 2+발화 → 정확히 1 실행(flock/msvcrt 헬퍼 양 분기)", ["A8", "A8py", "G17"])
def h_conc_1():
    lockmod = os.path.join(BIN_DIR, "javis_lock.py")
    need(os.path.isfile(lockmod), "javis_lock.py 부재")
    results = {}
    with tempfile.TemporaryDirectory() as tmp:
        lp = os.path.join(tmp, "sf.lock")
        holder = subprocess.Popen([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "holder", "2.5"],
                                  stdout=subprocess.PIPE, text=True)
        first = holder.stdout.readline().strip()
        need(first.startswith("acquired"), "선점자가 락을 못 잡았다: %r" % first)
        backend = first.split("|")[1]
        # 4 도전자 동시 → 전원 busy
        chal = [subprocess.Popen([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "chal%d" % i],
                                 stdout=subprocess.PIPE, text=True) for i in range(4)]
        outs = [c.communicate(timeout=60)[0].strip() for c in chal]
        holder.kill(); holder.wait(timeout=30)
        need(all(o.startswith("busy") for o in outs), "동시 발화가 직렬화되지 않았다: %r" % outs)
        results["race"] = "1 acquired / %d busy (backend=%s)" % (len(outs), backend)

        # 부트스트랩이 실제로 이 헬퍼를 소비하는가 (A8py — Windows 실효화의 전부)
        src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
        need("import javis_lock" in src, "javis_bootstrap 이 javis_lock 을 import 하지 않는다")
        need("_lock.FileLock" in src, "_acquire_singleflight 가 FileLock 을 소비하지 않는다")
        need(not re.search(r"^\s*import fcntl", src, re.M),
             "javis_bootstrap 에 fcntl 직접 사용이 남았다(posix 전용 사본)")
        # 함수 **본문만** 잘라 검사한다(다음 top-level def 까지) — 인접 함수 오탐 차단.
        m = re.search(r"^def _acquire_singleflight\(\):\n(.*?)(?=^def |\Z)", src, re.M | re.S)
        need(m, "_acquire_singleflight 정의를 찾지 못했다")
        # docstring·주석 제외 — 제거한 결함을 **문서화한** 문장까지 잡으면 계측기 오탐이다.
        sf = re.sub(r'"""(?:.|\n)*?"""', "", m.group(1))
        sf = "\n".join(l for l in sf.splitlines() if not l.strip().startswith("#"))
        need("os.name" not in sf,
             "싱글플라이트 본문에 posix 전용 가드가 남았다(Windows 무락 회귀)")

        # 부트스트랩 싱글플라이트 실동작: 보유 중 2번째 프로세스는 no-op(None)
        home = os.path.join(tmp, "home")
        os.makedirs(os.path.join(home, ".cys", "state"), exist_ok=True)
        code = ("import sys, os; sys.path.insert(0, %r); import javis_bootstrap as B\n"
                "r = B._acquire_singleflight()\n"
                "print('A' if r else 'NONE')\n"
                "sys.stdout.flush()\n"
                "import time; time.sleep(float(sys.argv[1]))\n" % BIN_DIR)
        env = _base_env({"HOME": home})
        p1 = subprocess.Popen([PY, "-c", code, "2.5"], stdout=subprocess.PIPE, text=True, env=env)
        need(p1.stdout.readline().strip() == "A", "1번째 부트스트랩이 락을 못 잡았다")
        p2 = _run([PY, "-c", code, "0"], env=env, timeout=60)
        p1.kill(); p1.wait(timeout=30)
        need(p2.stdout.strip() == "NONE",
             "2번째 부트스트랩이 no-op 으로 접히지 않았다(중복 부트): %r" % p2.stdout)
        results["bootstrap"] = "2번째 호출 no-op 확인"
    return " · ".join("%s: %s" % kv for kv in results.items())


pending("H-CONC-2", "W2", "훅+GUI boot 중첩 → 중복 스폰 0(락 확장)", ["G12"])
pending("H-CONC-3", "W3", "settings.json 3-writer 경합 무파손(mkstemp+공용 락)", ["G16", "A8"])
pending("H-CONC-4", "W2", "좌석 승계 임계영역 재검증(프로브 후 점유 → 승계 취소)", ["G13", "G14"])
pending("H-CONC-5", "W1b", "하네스 pgid 실측(결정 실험 — 처방 아닌 측정)", ["A18"])


@specimen("H-CONC-6", "W1a", "스테일 락(보유 pid 사망) 회수 — 유한 거부 창 상한", ["R1", "A2"])
def h_conc_6():
    def dead_pid():
        p = subprocess.Popen([PY, "-c", "pass"])
        p.wait()
        return p.pid
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        env = _base_env({"CYS_LOCK_BACKEND": "pidfile"})
        lp = os.path.join(tmp, "stale.lock")
        _w(lp, json.dumps({"pid": dead_pid(), "started": 0, "owner": "ghost"}), 0o644)
        r = _run([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "probe"], env=env, timeout=60)
        need(r.stdout.strip().startswith("acquired"), "스테일 락을 회수하지 못했다: %r" % r.stdout)
        need(r.stdout.strip().endswith("True"), "회수 흔적(reclaimed_stale)이 없다: %r" % r.stdout)
        notes.append("pidfile 스테일 회수 OK")
        # 살아있는 보유자는 회수 금지(중복 스폰 방지 — 반대 방향 결함)
        lp2 = os.path.join(tmp, "alive.lock")
        _w(lp2, json.dumps({"pid": os.getpid(), "started": time.time(), "owner": "me"}), 0o644)
        r2 = _run([PY, "-c", _LOCK_PROBE, BIN_DIR, lp2, "probe"], env=env, timeout=60)
        need(r2.stdout.strip().startswith("busy"), "살아있는 보유자의 락을 오회수했다: %r" % r2.stdout)
        notes.append("생존 보유자 보호 OK")
        # 부트스트랩 경로에서도 스테일 회수가 신규 부트를 허용하는가
        home = os.path.join(tmp, "home")
        st = os.path.join(home, ".cys", "state")
        os.makedirs(st, exist_ok=True)
        _w(os.path.join(st, "bootstrap-base.lock"),
           json.dumps({"pid": dead_pid(), "started": 0, "owner": "ghost"}), 0o644)
        code = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as B\n"
                "print('A' if B._acquire_singleflight() else 'NONE')\n" % BIN_DIR)
        r3 = _run([PY, "-c", code], env=_base_env({"HOME": home, "CYS_LOCK_BACKEND": "pidfile"}),
                  timeout=60)
        need(r3.stdout.strip() == "A",
             "스테일 부트 락이 신규 부트를 계속 거부한다(~330s 창 잔존): %r %s" % (r3.stdout, r3.stderr[-300:]))
        notes.append("부트스트랩 스테일 락 회수 OK")
    return " · ".join(notes)


# ═══════════════════════════════════════════════════════════════════════════
# 5. H-WIN (크로스플랫폼 — 스텁 목으로 macOS 결정론 실행)
# ═══════════════════════════════════════════════════════════════════════════
@specimen("H-WIN-1", "W1a", "백슬래시 file_path 로 헌법파일 쓰기 → 차단(guard 정규화)", ["G18"])
def h_win_1():
    g = _hook("guard.sh")
    env = _base_env()
    cases = [
        ("C:\\Users\\me\\.claude\\soul.md", 2, "드라이브 백슬래시 soul.md"),
        ("C:\\x\\CLAUDE.md", 2, "백슬래시 CLAUDE.md"),
        ("C:\\x\\WORKER_DIRECTIVE.md", 2, "백슬래시 *_DIRECTIVE.md"),
        ("/Users/me/.claude/soul.md", 2, "POSIX soul.md(회귀 대조)"),
        ("C:\\x\\notes.md", 0, "무해 파일(과차단 대조)"),
    ]
    for fp, want, label in cases:
        r = _run(["bash", g], env=env,
                 input=json.dumps({"tool_name": "Write", "tool_input": {"file_path": fp}}))
        need(r.returncode == want, "%s: exit=%d(기대 %d)" % (label, r.returncode, want))
    # Bash 경로(LOOSE)도 같은 정규화가 걸리는가
    r = _run(["bash", g], env=env, input=json.dumps(
        {"tool_name": "Bash", "tool_input": {"command": "echo x | tee C:\\Users\\me\\soul.md"}}))
    need(r.returncode == 2, "LOOSE bash 백슬래시 경로 우회(exit=%d)" % r.returncode)
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/guard.sh")
    if old is not None:
        with tempfile.TemporaryDirectory() as tmp:
            og = os.path.join(tmp, "guard.sh")
            _w(og, old)
            _w(os.path.join(tmp, "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            r2 = _run(["bash", og], env=env, input=json.dumps(
                {"tool_name": "Write", "tool_input": {"file_path": "C:\\Users\\me\\.claude\\soul.md"}}))
            need(r2.returncode == 0,
                 "계측 타당성 실패: 구 guard 가 백슬래시 경로를 이미 차단한다면 이 검체는 무의미")
            calib = "구 코드 무음 우회 재현 확인"
    return "차단 4 / 통과 1 (Write+Bash 양 경로) · 계측검증=%s" % calib


@specimen("H-WIN-2", "W1a", "드라이브 cwd(C:\\·C:/) → 상향탐색 성공(cwd 게이트 4훅)", ["G19"])
def h_win_2():
    files = ["inject-context.sh", "save-state.sh", "reflect-scan.sh", "vibecoding/vibe-regression.sh"]
    for f in files:
        body = _read(os.path.join(HOOKS_DIR, f))
        need("cys_is_abs" in body, "%s 가 cys_is_abs 를 쓰지 않는다" % f)
        # 주석 제외 — 결함 설명으로 구 패턴을 인용한 줄까지 잡으면 계측기 오탐이다.
        code = "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))
        need(not re.search(r'case\s+"\$CWD"\s+in\s+/\*\)', code),
             "%s 에 구 `/*` 전용 게이트가 남았다" % f)
    notes = ["정적: 4훅 전수 cys_is_abs"]
    with tempfile.TemporaryDirectory() as tmp:
        # $TMP 안에 'C:' 디렉터리를 만들고 cwd를 상대 해석시켜 드라이브 경로를 macOS에서 재현.
        drive = os.path.join(tmp, "C:")
        os.makedirs(os.path.join(drive, "proj", "_round"), exist_ok=True)
        os.makedirs(os.path.join(drive, "proj", "tests"), exist_ok=True)
        _w(os.path.join(drive, "proj", "_round", "SESSION_STATE.md"), "WIN-STATE-MARKER\n", 0o644)
        pack = os.path.join(tmp, "pack")
        _w(os.path.join(pack, "bin", "javis_memory.py"), "import sys; sys.exit(1)\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": pack})
        for cwd in ("C:/proj/sub", "C:\\proj\\sub"):
            payload = json.dumps({"source": "clear", "cwd": cwd, "hook_event_name": "Stop",
                                  "transcript_path": ""})
            r = _run(["bash", _hook("inject-context.sh")], input=payload, env=env, cwd=tmp)
            need("WIN-STATE-MARKER" in r.stdout,
                 "inject-context: cwd=%r 에서 작업기억 미발견" % cwd)
            r = _run(["bash", _hook("save-state.sh")], input=payload, env=env, cwd=tmp)
            need("Stop" in _read(os.path.join(drive, "proj", "_round", ".state_log")),
                 "save-state: cwd=%r 에서 write-ahead 미기록" % cwd)
            os.remove(os.path.join(drive, "proj", "_round", ".state_log"))
            r = _run(["bash", _hook("reflect-scan.sh")], input=payload, env=env, cwd=tmp)
            need("WARN:memory" in _read(os.path.join(drive, "proj", "_round", ".state_log")),
                 "reflect-scan: cwd=%r 에서 _round 해소 실패" % cwd)
            os.remove(os.path.join(drive, "proj", "_round", ".state_log"))
            # vibe-regression 은 상향탐색이 아니라 cwd 를 **직접** 쓴다 → 루트 표기로 대입.
            r = _run(["bash", os.path.join(HOOKS_DIR, "vibecoding", "vibe-regression.sh")],
                     env=env, cwd=tmp, input=json.dumps(
                         {"tool_input": {"command": "javis_task.py set-status T done"},
                          "cwd": cwd.rsplit("/", 1)[0].rsplit("\\", 1)[0]}))
            need("테스트 스위트를 done 직전에 실행" in r.stderr,
                 "vibe-regression: cwd=%r 가 $PWD 로 오폴백(스위트 오탐)" % cwd)
        notes.append("기능: 4훅 × 2표기(C:/·C:\\) 해소 확인")
    return " · ".join(notes)


@specimen("H-WIN-3", "W1a", "명명 부서 + named pipe 소켓 → DEPT_CTX 판정(python 술어 일치)", ["G20", "G4"])
def h_win_3():
    with tempfile.TemporaryDirectory() as tmp:
        # ① 명명 부서 팩(pack-dept-sales) → 부서 round 정본만 주입
        dept = os.path.join(tmp, "pack-dept-sales")
        os.makedirs(os.path.join(dept, "round"), exist_ok=True)
        _w(os.path.join(dept, "round", "SESSION_STATE.md"), "DEPT-ROUND-MARKER\n", 0o644)
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "MAIN-LANE-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": dept})
        r = _run(["bash", _hook("inject-context.sh")], env=env,
                 input=json.dumps({"source": "clear", "cwd": proj}))
        need("DEPT-ROUND-MARKER" in r.stdout, "명명 부서 팩이 부서 정본을 주입하지 않았다(G4)")
        need("MAIN-LANE-MARKER" not in r.stdout, "명명 부서 레인에 메인 레인 작업기억이 오주입됐다(격리 파괴)")
        # ② Windows named pipe 소켓(백슬래시) → 부서 컨텍스트로 인식
        env2 = _base_env({"HOME": os.path.join(tmp, "home"),
                          "CYS_PACK_DIR": os.path.join(tmp, "pack"),
                          "CYS_SOCKET": r"\\.\pipe\cys-dept-sales"})
        r2 = _run(["bash", _hook("inject-context.sh")], env=env2,
                  input=json.dumps({"source": "clear", "cwd": proj}))
        need("부서 pack round SESSION_STATE 부재" in r2.stdout,
             "named pipe 부서 소켓이 부서 컨텍스트로 인식되지 않았다(G20): %r" % r2.stdout[:300])
        need("MAIN-LANE-MARKER" not in r2.stdout, "부서 소켓 레인에 메인 작업기억이 오주입됐다")
    # ③ 셸 ↔ python 술어 parity (판정 SOT 일치)
    code = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as B\n"
            "print(B._socket_dept(r'\\\\\\\\.\\\\pipe\\\\cys-dept-sales'), "
            "B._socket_dept('/x/cys-dept-sales/cys.sock'), B._pack_dept('/y/pack-dept-sales'))\n" % BIN_DIR)
    r3 = _run([PY, "-c", code], env=_base_env(), timeout=60)
    need(r3.stdout.split() == ["sales", "sales", "sales"],
         "python 술어 parity 실패: %r %s" % (r3.stdout, r3.stderr[-300:]))
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/inject-context.sh")
    if old is not None:
        need("pack-dept-dept-" in old,
             "계측 타당성 실패: 구 코드에 dept-N 한정 글롭이 없으면 이 검체는 무의미")
        calib = "구 코드 dept-N 한정 글롭 확인"
    return "명명 부서·named pipe 인식 + 셸↔python 술어 일치 · 계측검증=%s" % calib


@specimen("H-WIN-4", "W1a", "taskkill 감지 + rc=2 인프라/판정 분리(fail-open 정책 복원)", ["G21"])
def h_win_4():
    gate = _hook("actprobe-kill-gate.sh")
    with tempfile.TemporaryDirectory() as tmp:
        _w(os.path.join(tmp, "verdict.py"),
           "import sys\nprint('[FAIL] kill-preflight pid: 활성 노드')\nsys.exit(2)\n", 0o644)
        _w(os.path.join(tmp, "usage.py"),
           "import sys\nsys.stderr.write('actprobe: argument error: bad\\n')\nsys.exit(2)\n", 0o644)
        _w(os.path.join(tmp, "indet_usage.py"),
           "import sys\nsys.stderr.write('usage: actprobe ...\\n')\nsys.exit(3)\n", 0o644)
        def run(cmd, probe):
            return _run(["sh", gate], input=json.dumps(
                {"tool_name": "Bash", "tool_input": {"command": cmd}}),
                env=_base_env({"CYS_ACTPROBE": os.path.join(tmp, probe)}))
        need(run("taskkill /F /PID 4242", "verdict.py").returncode == 2,
             "taskkill + verdict FAIL 이 차단되지 않았다(G21 동사 누락)")
        r = run("taskkill /F /PID 4242", "usage.py")
        need(r.returncode == 0, "rc=2 사용오류가 차단으로 승격됐다(fail-open 정책 역전 잔존)")
        need("verdict 형상 아님" in r.stderr, "인프라 실패 WARN 문구 부재: %r" % r.stderr[:300])
        r = run("taskkill /F /PID 4242", "indet_usage.py")
        need(r.returncode == 0, "rc=3 인자 오류가 차단으로 승격됐다")
        need(run("taskkill /IM claude.exe /F", "verdict.py").returncode == 2,
             "이름 기반 taskkill 이 fail-closed 로 차단되지 않았다")
        need(run("ls -la", "verdict.py").returncode == 0, "비-kill 명령에 개입했다(과차단)")
        need(run("kill -9 4242", "verdict.py").returncode == 2, "POSIX kill 회귀")
    body = _read(os.path.join(HOOKS_DIR, "actprobe-kill-gate.sh"))
    need('"taskkill"' in body, "KILL 동사 집합에 taskkill 이 없다")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/actprobe-kill-gate.sh")
    if old is not None:
        need('"taskkill"' not in old, "계측 타당성 실패: 구 코드에 이미 taskkill 이 있다")
        calib = "구 코드 taskkill 부재 확인"
    return "taskkill 차단·rc2/rc3 형상 분리·POSIX 회귀 0 · 계측검증=%s" % calib


@specimen("H-WIN-5", "W1a", "python3 경성 참조 전수 소멸 + CYS_PY 폴백 실동작", ["G22"])
def h_win_5():
    # ── 탐지기: 비주석 텍스트에서 `python3` 이 **명령어 위치**에 오는 경우만 위반 ──
    # 허용: 주석 / `command -v python3` / 후보 목록 / `${CYS_PY:-python3}`(계약된 명시 폴백) /
    #       python 소스·메시지 문자열 안의 언급.
    # `python3` 다음 문자까지 요구해 산문 언급(`(python3/python/py)` 같은 열거)을 배제한다 —
    # 명령어라면 뒤에 공백·인용·구분자·행끝이 온다.
    cmdpos = re.compile(r"(?:(?<=^)|(?<=\|)|(?<=;)|(?<=&)|(?<=\()|(?<=`)|(?<=\bexec\s)|"
                        r"(?<=\bthen\s)|(?<=\bdo\s)|(?<=\belse\s))\s*python3(?=$|[\s;|&)\"'])")

    def violations(body):
        out = []
        for i, line in enumerate(body.splitlines(), 1):
            s = line.strip()
            if not s or s.startswith("#"):
                continue
            if "command -v python3" in s or re.search(r"for\s+c\s+in\b", s):
                continue
            probe = s.replace("${CYS_PY:-python3}", "${CYS_PY}")
            if cmdpos.search(probe):
                out.append((i, s[:110]))
        return out

    bad = {}
    for rel in _shell_hooks():
        v = violations(_read(os.path.join(HOOKS_DIR, rel)))
        if v:
            bad[rel] = v
    need(not bad, "python3 경성 호출 잔존:\n" + "\n".join(
        "  %s:%s %s" % (k, l, t) for k, vs in bad.items() for l, t in vs))

    # ── 계측기 자기검증: 같은 탐지기가 구 코드에서는 FIRE 해야 한다 ──
    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        r = _run(["git", "-C", REPO_DIR, "grep", "-ln", "python3", CALIBRATION_REF,
                  "--", "cysjavis-pack/hooks/"], timeout=60)
        old_files = [l.split(":", 1)[1] for l in r.stdout.splitlines() if ":" in l]
        old_sh = [f for f in old_files if f.endswith(".sh")]
        fired = sum(1 for f in old_sh if violations(_git_show(f) or ""))
        need(len(old_sh) >= 20, "구 인벤토리 산출 실패(%d) — 대조 불가" % len(old_sh))
        need(fired >= 10, "계측 타당성 실패: 구 코드에서 탐지기가 %d개만 FIRE(결함 재현 불충분)" % fired)
        calib = "구 인벤토리 %d훅 중 %d훅 FIRE" % (len(old_sh), fired)

    # ── 기능: python3 부재·python 만 있는 PATH 에서 대표 훅이 정상 동작 ──
    real = shutil.which("python3") or PY
    with tempfile.TemporaryDirectory() as tmp:
        binp = os.path.join(tmp, "bin")
        _w(os.path.join(binp, "python"), '#!/bin/sh\nexec "%s" "$@"\n' % real)
        for tool in ("sh", "bash", "env", "sed", "grep", "date", "wc", "tr", "awk", "head",
                     "tail", "cat", "cut", "sort", "dirname", "basename", "ls", "rm", "mkdir",
                     "sleep", "printf", "git", "locale", "stat", "uname", "mktemp", "chmod"):
            src = shutil.which(tool)
            if src and not os.path.exists(os.path.join(binp, tool)):
                os.symlink(src, os.path.join(binp, tool))
        BASH = shutil.which("bash") or "/bin/bash"
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "NOPY3-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "PATH": binp,
                         "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        need(shutil.which("python3", path=binp) is None, "격리 PATH 에 python3 가 남아 있다")
        payload = json.dumps({"source": "clear", "cwd": proj, "hook_event_name": "PreCompact"})
        r = _run([BASH, _hook("inject-context.sh")], input=payload, env=env)
        need(r.returncode == 0 and "NOPY3-MARKER" in r.stdout,
             "python3 부재 PATH 에서 inject-context 실패: exit=%d %r" % (r.returncode, r.stderr[-300:]))
        r = _run([BASH, _hook("save-state.sh")], input=payload, env=env)
        need(r.returncode == 0 and "PreCompact" in _read(os.path.join(proj, "_round", ".state_log")),
             "python3 부재 PATH 에서 save-state 실패")
        r = _run([BASH, _hook("pack-guard.sh")], input=json.dumps(
            {"tool_input": {"file_path": "/x/y.sh"}, "session_id": "s"}), env=env)
        need(r.returncode == 0, "python3 부재 PATH 에서 pack-guard 비0 종료")
    return "훅 %d개 경성 참조 0 · 계측검증=%s · python-only PATH 3훅 정상" % (
        len(_shell_hooks()), calib)


@specimen("H-WIN-6", "W1a", "pack-guard 백슬래시 접두 매칭(vendor 경고 무음 해소)", ["G23"])
def h_win_6():
    with tempfile.TemporaryDirectory() as tmp:
        binp = os.path.join(tmp, "bin")
        _mock_cys(binp, tmp, 'case "$1" in pack-ownership) echo system; exit 0;; esac')
        env = _base_env({"HOME": os.path.join(tmp, "home"),
                         "PATH": binp + os.pathsep + os.environ.get("PATH", ""),
                         "CYS_PACK_DIR": "C:/Users/me/.cys/pack",
                         "TMPDIR": os.path.join(tmp, "stamps")})
        r = _run(["bash", _hook("pack-guard.sh")], env=env, input=json.dumps(
            {"tool_input": {"file_path": "C:\\Users\\me\\.cys\\pack\\hooks\\guard.sh"},
             "session_id": "s1"}))
        need(r.returncode == 0, "pack-guard 비0 종료(%d)" % r.returncode)
        need("pack-ownership" in _calls(tmp),
             "백슬래시 경로가 팩 접두로 인식되지 않았다(경고 무음 잔존): %r" % _calls(tmp))
        need("pack-guard" in r.stdout and "hooks/guard.sh" in r.stdout,
             "REL 산출·경고 문안 이상: %r" % r.stdout[:400])
        # 과차단 대조: 팩 밖 파일은 무동작
        env["TMPDIR"] = os.path.join(tmp, "stamps2")
        r2 = _run(["bash", _hook("pack-guard.sh")], env=env, input=json.dumps(
            {"tool_input": {"file_path": "C:\\other\\x.sh"}, "session_id": "s2"}))
        need(r2.stdout.strip() == "", "팩 밖 파일에 경고를 냈다(과탐): %r" % r2.stdout[:200])
    return "백슬래시 팩 접두 매칭 + 팩 밖 무동작"


@specimen("H-WIN-7", "W1a", "부트 브리지 안내 문자열 cygpath·인용 적용", ["G8"])
def h_win_7():
    with tempfile.TemporaryDirectory() as tmp:
        pack = os.path.join(tmp, "pack")
        os.makedirs(os.path.join(pack, "directives"), exist_ok=True)
        _w(os.path.join(pack, "bin", "javis_bootstrap.py"), "# stub\n", 0o644)
        _w(os.path.join(pack, "directives", "MASTER_DIRECTIVE.md"), "MASTER-BODY\n", 0o644)
        binp = os.path.join(tmp, "bin")
        _mock_cys(binp, tmp)
        # cygpath 목: -w 로 공백 포함 네이티브 경로를 돌려준다(인용 필요 상황 재현)
        _w(os.path.join(binp, "cygpath"),
           '#!/bin/sh\n[ "$1" = "-w" ] && { printf "X:\\\\Prog Files\\\\javis_bootstrap.py\\n"; exit 0; }\nexit 1\n')
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": pack,
                         "CYS_SURFACE_ID": "3",
                         "PATH": binp + os.pathsep + os.environ.get("PATH", "")})
        want = '"X:\\\\Prog Files\\\\javis_bootstrap.py"'
        r = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        need(want in r.stdout,
             "role-less 안내가 네이티브 경로+인용을 쓰지 않는다: %r" % r.stdout[:600])
        env["CYS_ROLE"] = "master"
        r2 = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        need("부트 브리지" in r2.stdout and want in r2.stdout,
             "master 부트 브리지가 네이티브 경로+인용을 쓰지 않는다: %r" % r2.stdout[-600:])
        # 인용이 실제로 sh 에서 원 경로로 복원되는가(eval 왕복)
        line = [l for l in r2.stdout.splitlines() if want in l][0].strip()
        rt = os.path.join(tmp, "roundtrip.sh")
        _w(rt, "eval 'set -- %s'\nprintf '%%s' \"$2\"\n" % line, 0o644)
        chk = _run(["sh", rt], env=_base_env())
        need(chk.stdout == "X:\\Prog Files\\javis_bootstrap.py",
             "인용 왕복 실패(복사 실행 불가): %r (line=%r)" % (chk.stdout, line))
        # unix(cygpath 부재) 무변경 대조
        env.pop("CYS_ROLE")
        env["PATH"] = os.environ.get("PATH", "")
        r3 = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        need(os.path.join(pack, "bin", "javis_bootstrap.py") in r3.stdout,
             "cygpath 부재 환경에서 경로가 변형됐다(unix 회귀): %r" % r3.stdout[:500])
    return "cygpath 변환·인용 왕복 검증 · unix 무변경"


@specimen("H-WIN-8", "W1a", "spawn_env_pairs 공용 소비(pane·schedule 2경로 · GUI=W4)", ["A17"])
def h_win_8():
    lib = os.path.join(REPO_DIR, "src", "lib.rs")
    if not os.path.isfile(lib):
        raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
    body = _read(lib)
    need(body.count("pub fn spawn_env_pairs(") == 1, "lib.rs 에 spawn_env_pairs 단일 정의가 아니다")
    need("pub fn spawn_env_pairs_from_process(" in body, "프로세스 env 래퍼 부재")
    dups = []
    for root, _d, files in os.walk(os.path.join(REPO_DIR, "src")):
        for f in files:
            if not f.endswith(".rs"):
                continue
            p = os.path.join(root, f)
            if os.path.abspath(p) == os.path.abspath(lib):
                continue
            if re.search(r"^\s*(pub\s+)?fn spawn_env_pairs\s*\(", _read(p), re.M):
                dups.append(os.path.relpath(p, REPO_DIR))
    need(not dups, "spawn_env_pairs 사본 정의 잔존: %s" % dups)
    for rel in ("src/bin/cysd/state.rs", "src/bin/cysd/schedule.rs"):
        need("spawn_env_pairs" in _read(os.path.join(REPO_DIR, rel)),
             "%s 가 공용 spawn_env_pairs 를 소비하지 않는다" % rel)
    st = _read(os.path.join(REPO_DIR, "src/bin/cysd/state.rs"))
    need("spawn_env_pairs_from_process" in st, "pane 스폰이 HOME backfill 규약을 소비하지 않는다")
    calib = "skip(no-git)"
    old = _git_show("src/bin/cysd/schedule.rs")
    if old is not None:
        need(re.search(r"^\s*fn spawn_env_pairs\s*\(", old, re.M),
             "계측 타당성 실패: 구 코드에 private 사본이 없으면 승격 검증이 무의미")
        old_state = _git_show("src/bin/cysd/state.rs") or ""
        need("spawn_env_pairs" not in old_state,
             "계측 타당성 실패: 구 pane 스폰이 이미 규약을 소비했다면 A17 은 결함이 아니다")
        calib = "구 코드 사본·pane 미소비 확인"
    return "lib 단일 정의·사본 0·pane/schedule 소비 · 계측검증=%s" % calib


pending("H-WIN-9", "W4", "어댑터 OS 후보 해소(agy/codex Windows 경로) — 커버 결함 B8 = W4 소속",
        ["B8"])
pending("H-WIN-10", "W4", "Windows 1차 종료 단계 유효성(CTRL_BREAK 대체·명시 로그) — 커버 결함 G24 = W4 소속",
        ["G24"])
pending("H-WIN-11", "W4", "Windows CI 실기 재실행(부채 V4 해소)", ["A6", "A8", "B13"])


# ═══════════════════════════════════════════════════════════════════════════
# 6. H-PRED / H-TIME / H-DOC / H-SEED / H-LIFE / H-OBS
# ═══════════════════════════════════════════════════════════════════════════
pending("H-PRED-1", "W2", "결손 판정↔check verdict 공유 fixture 기계 차분", ["A1", "G26"])
pending("H-PRED-2", "W2", "생존 술어 parity(boot 스킵·wakeup zombie·reclaim)", ["B3", "B13", "G27"])
pending("H-PRED-3", "W2", "awakened_at 래치(부재=legacy-presumed·NOT-awake 단정 금지)", ["B6"])
pending("H-PRED-4", "W2", "슬롯 충족 parity(native/substitute/이중충전)", ["B2", "B12"])
pending("H-PRED-5", "W2", "role_family 전수 해소(데몬 발권 전 role × 소비처 전수)", ["A3=B7", "B10", "G2"])
pending("H-PRED-6", "W2", "규약 상수 grep 전수 계약(env 4키·CYS_PY·부서 판정 정규화)", ["A11", "G4", "G22"])
pending("H-PRED-7", "W2", "PLAN 정책 열↔effective_required_roles↔결손 구성 3자 대조", ["B1"])
pending("H-PRED-8", "W2", "readiness 델타 매칭(개수 비교 회귀 차단)", ["B4"])
pending("H-PRED-9", "W4", "trust 패턴·마커 SOT=코드/vendor + user override 계층", ["P3-B19"])
pending("H-PRED-10", "W4", "TCC 탐침 대상이 실자원(cwd+PACK)에서 파생", ["P3-A-TCC"])
pending("H-TIME-1", "W2", "예산 parity Σ(하위 최악치)≤상위 + 냉시작 하한 fixture", ["B9"])
pending("H-TIME-2", "W2", "문서·훅 안내 숫자=BUDGET 상수 파생(하드코딩 grep 0)", ["P3-A-120S"])
pending("H-TIME-3", "W2", "카운트 회계 금지(Instant 데드라인 단언)", ["B17"])
pending("H-DOC-1", "W1b", "§0↔훅 note↔template 문안 정합", ["A10", "P3-A-TEMPLATE"])
pending("H-DOC-2", "W4", "'5노드'=REQUIRED_ROLES+1 파생", ["B18"])
pending("H-DOC-3", "W4", "CEO_TEMPLATE 동사↔cys-dept 가드 허용 집합 대조", ["G6"])
pending("H-DOC-4", "W4", "헤더 exit 표↔코드 상수 기계 대조 (결함 G31·G32 문구는 W0에서 수정 — 기계 대조 assert 는 CS-6 문서 정합 테스트=W4 산출)", ["G31", "G32"])
pending("H-DOC-5", "W4", "generic reviewer 안내 문구 금지 (결함 G30 문구는 W0에서 수정 — assert 는 W4)", ["G30"])
pending("H-DOC-6", "W4", "§0 ③ 경로가 CYS_PACK_DIR 파생 (결함 G5 는 W0에서 수정 — assert 는 W4)", ["G5"])
pending("H-DOC-7", "W4", "agents 스키마 완결성(vendor/user 계층)", ["B20"])
pending("H-DOC-8", "W4", "팀 부트 진입점 전수 단일 계약", ["B5"])
pending("H-SEED-1", "W3", "소망 훅 집합 동등성(Rust 시드/init-pack == SELFCORR_HOOKS)", ["A9"])
pending("H-SEED-2", "W3", "실사용 config dir 등록 hard 검증(CLAUDE_CONFIG_DIR 최우선)", ["A21", "R4"])
pending("H-SEED-3", "W3", "settings.json 없는 프로필 디렉토리 후보화", ["G7"])
pending("H-SEED-4", "W3", "cys-dept launch/rotate → CYS_ACCOUNT_DIR 복원 주입", ["G3"])
pending("H-SEED-5", "W3", "외부 동명 훅 보존(_prune 소유 술어)", ["G10"])
pending("H-LIFE-1", "W3", "레인별 marker/boot-last 분리 + run 귀속 교차 단언", ["G15", "P3-A-DEPT-LANE"])
pending("H-LIFE-2", "W1b", "step id enum 유일성·기록 순서=실행 순서", ["P3-A-STEP-NAME"])
pending("H-OBS-1", "W2", "배달 3분기 VERIFY(pending/dropped/delivered-무ack)", ["B11"])
pending("H-OBS-2", "W2", "주입 검증 실패 → directive_verified:false 상태화", ["B14"])



@specimen("H-OBS-3", "W1a", "다중 세션 카운트가 comm=claude 프로세스를 계수", ["G33"])
def h_obs_3():
    body = _read(os.path.join(HOOKS_DIR, "inject-context.sh"))
    need("-c claude" in body, "lsof 계측이 claude 프로세스를 세지 않는다(계측기 타당성)")
    with tempfile.TemporaryDirectory() as tmp:
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "S\n", 0o644)
        binp = os.path.join(tmp, "bin")
        # 목 lsof: `-c claude` 가 인자에 있을 때만 cwd 행 2개를 낸다(=네이티브 claude 2세션)
        _w(os.path.join(binp, "lsof"),
           '#!/bin/sh\nfor a in "$@"; do [ "$a" = claude ] && '
           '{ printf "n%%s\\nn%%s\\n" "%s" "%s"; exit 0; }; done\nexit 0\n' % (proj, proj))
        for tool in ("sed", "grep", "date", "wc", "tr", "awk", "head", "tail", "cat",
                     "dirname", "basename", "ls", "rm", "mkdir", "python3", "sh", "bash"):
            src = shutil.which(tool)
            if src and not os.path.exists(os.path.join(binp, tool)):
                os.symlink(src, os.path.join(binp, tool))
        env = _base_env({"HOME": os.path.join(tmp, "home"), "PATH": binp,
                         "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        r = _run(["bash", _hook("inject-context.sh")], env=env,
                 input=json.dumps({"source": "clear", "cwd": proj}))
        need("동시에 도는 claude 세션이 2개 감지됨" in r.stdout,
             "claude 세션 2개를 계수하지 못했다: %r" % r.stdout[-500:])
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/inject-context.sh")
    if old is not None:
        need("-c claude" not in old, "계측 타당성 실패: 구 코드가 이미 claude 를 셌다")
        calib = "구 코드 node 전용 계측 확인"
    return "목 lsof 로 claude 2세션 계수 확인 · 계측검증=%s" % calib


# ═══════════════════════════════════════════════════════════════════════════
# 러너
# ═══════════════════════════════════════════════════════════════════════════
def main(argv=None):
    ap = argparse.ArgumentParser(description="부트스트랩 상시 건강성 1커맨드 게이트")
    ap.add_argument("--json", action="store_true", help="기계 판독 결과(JSON)")
    ap.add_argument("--list", action="store_true", help="검체 대장만 출력")
    ap.add_argument("--only", default="", help="검체 ID 콤마 목록만 실행")
    ap.add_argument("--wave", default="", help="해당 웨이브 태그 검체만 실행")
    ap.add_argument("--include-pending", action="store_true",
                    help="미발효 검체도 실행 시도(개발용 — 게이트 판정에는 산입하지 않는다)")
    args = ap.parse_args(argv)

    if args.list:
        for sid, wave, title, defects, fn in _REG:
            print("%-12s %-4s %-9s %s  [%s]" % (
                sid, wave, "effective" if (fn and wave in LANDED_WAVES) else "pending",
                title, ",".join(defects)))
        return 0

    only = {s.strip() for s in args.only.split(",") if s.strip()}
    rows = []
    t0 = time.time()
    for sid, wave, title, defects, fn in _REG:
        if only and sid not in only:
            continue
        if args.wave and wave != args.wave:
            continue
        row = {"id": sid, "wave": wave, "title": title, "defects": defects}
        effective = bool(fn) and wave in LANDED_WAVES
        if not effective:
            row["status"] = "pending"
            row["reason"] = ("발효 웨이브 %s 미착지(현재 착지: %s)" % (wave, ",".join(LANDED_WAVES))
                             if wave not in LANDED_WAVES else "구현 대기(발효 웨이브 착지 — 구현 필요)")
            if wave in LANDED_WAVES and not fn:
                row["status"] = "fail"
                row["reason"] = "발효 웨이브인데 검체 미구현 — 측정 실패는 hard fail"
            if not (args.include_pending and fn):
                rows.append(row)
                continue
        s = time.time()
        try:
            detail = fn()
            row["status"] = "pass"
            row["detail"] = detail or ""
        except Skip as e:
            row["status"] = "skip"
            row["reason"] = str(e)
        except Fail as e:
            row["status"] = "fail"
            row["reason"] = str(e)
        except Exception as e:  # 검체 자체 크래시도 fail(측정 실패=hard fail)
            row["status"] = "fail"
            row["reason"] = "검체 크래시 %s: %s" % (type(e).__name__, e)
        row["secs"] = round(time.time() - s, 2)
        if not effective:
            row["counted"] = False
        rows.append(row)

    counted = [r for r in rows if r.get("counted", True)]
    failed = [r for r in counted if r["status"] == "fail"]
    passed = [r for r in counted if r["status"] == "pass"]
    skipped = [r for r in counted if r["status"] == "skip"]
    pend = [r for r in counted if r["status"] == "pending"]
    verdict = "GREEN" if not failed else "RED"
    summary = {"verdict": verdict, "landed_waves": list(LANDED_WAVES),
               "pass": len(passed), "fail": len(failed), "skip": len(skipped),
               "pending": len(pend), "total": len(rows),
               "elapsed_secs": round(time.time() - t0, 1),
               "calibration_ref": CALIBRATION_REF}

    if args.json:
        print(json.dumps({"summary": summary, "specimens": rows}, ensure_ascii=False, indent=1))
    else:
        for r in rows:
            mark = {"pass": "PASS", "fail": "FAIL", "skip": "SKIP", "pending": "PEND"}[r["status"]]
            line = "%s %-12s %-4s %s" % (mark, r["id"], r["wave"], r["title"])
            if r["status"] == "pass" and r.get("detail"):
                line += "\n       └ %s" % r["detail"]
            if r["status"] in ("fail", "skip", "pending") and r.get("reason"):
                line += "\n       └ %s" % r["reason"]
            print(line)
        print("\n%s — 발효 %d PASS / %d FAIL / %d SKIP · 미발효 %d PEND · %.1fs (발효 웨이브 %s)"
              % (verdict, len(passed), len(failed), len(skipped), len(pend),
                 summary["elapsed_secs"], ",".join(LANDED_WAVES)))
        if failed:
            print("\n적색 검체 → 원 결함 ID로 역추적:")
            for r in failed:
                print("  - %s [%s]: %s" % (r["id"], ",".join(r["defects"]), r.get("reason", "")[:400]))
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
