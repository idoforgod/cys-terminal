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
LANDED_WAVES = ("W0", "W1a", "W1b")

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


def _code_lines(body):
    """셸/파이썬 소스에서 **주석 전용 줄을 제거**한 코드만 남긴다.
    ★계측기 오탐 방지: 제거한 결함을 주석으로 **설명한** 줄(`cut -c1-200 은 제거됐다`)까지
      정적 스캔이 잡으면, 문서화가 곧 회귀로 보고된다(W1a G-PRELUDE 선례와 동일 규약)."""
    return "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))


def _detect_mod():
    """감지기 단일 소유 모듈(bin/javis_detect.py) 적재 — W1b corpus 검체의 피검체."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_detect
    return javis_detect


def _old_hook_fires(prompt, tmp, env_extra=None):
    """★계측 타당성: 기준 커밋의 **구 훅**을 같은 프롬프트로 돌려 발화 여부를 관측한다.
    구 코드가 결함을 재현하지 못하면 신 코드의 PASS 는 아무것도 증명하지 않는다(MEMORY 3칙).
    반환: True/False/None(레포 아님 — 계측 대조 생략)."""
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    if old is None:
        return None
    oldhook = os.path.join(tmp, "oldhooks", "role-bootstrap.sh")
    _w(oldhook, old)
    # 구 훅(a96d8b1)은 프리루드를 source 하지 않지만, 상위 웨이브 계보에서 온 사본이 섞이면
    # loud-skip 으로 전멸할 수 있어 형제 위치에 프리루드를 병치해 둔다(무해·멱등).
    _w(os.path.join(tmp, "oldhooks", "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
    env, _h, _p, _b, _st = _rb_sandbox(os.path.join(tmp, "oldsb"))
    if env_extra:
        env.update(env_extra)
    r = _run(["bash", oldhook], input=json.dumps({"prompt": prompt}), env=env)
    return "발화됨" in r.stdout


def _calib_note(fired, expect, what):
    """구 코드 관측치가 기대(결함 재현)와 일치하는지 확인하고 계측검증 문구를 만든다."""
    if fired is None:
        return "skip(no-git)"
    need(fired == expect,
         "계측 타당성 실패: 구 코드가 %s 에서 fired=%s (기대 %s) — 검체가 결함을 재현하지 못한다"
         % (what, fired, expect))
    return "구 코드 fired=%s 재현" % fired


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
@specimen("B-8", "W1b", "베이스라인 test_lane_isolation_v1 (훅 BOOT 부재·레인↔팩)",
          ["레인 격리", "A7 종료 경로"])
def b_8():
    """★W1b 편입 이유: 이 테스트가 훅의 **BOOT 부재 분기**와 bootstrap 의 exit 8 경로를 핀한다 —
    W1b 가 훅 전단(감지기 이관·게이트 재배치)과 cmd_run 종료 구조를 동시에 만졌으므로 1커맨드
    게이트 안에 들어와야 한다(종전엔 러너 밖 수동 실행이었다)."""
    return _baseline([PY, os.path.join(TESTS_DIR, "test_lane_isolation_v1.py")],
                     "test_lane_isolation_v1")


@specimen("B-9", "W1b", "베이스라인 javis_detect --self-test (감지기 밀폐 corpus)",
          ["A4", "P3-A-NEGA", "P3-A-FILLER", "G9", "G25"])
def b_9():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_detect.py"), "--self-test"],
                     "javis_detect --self-test")


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
           "cys_resolve_py", "cys_fix_locale", "cys_timeout_run"]
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
@specimen("H-DETECT-1", "W1b", "혼합의도 FIRE corpus(절 경계 스코프)", ["A4"])
def h_detect_1():
    """A4: 억제를 감지보다 먼저 돌리던 순서를 역전 — 선언이 **속한 절** 안의 마커만 억제한다."""
    D = _detect_mod()
    fire = ["너는 마스터다. 오늘 뭐부터 할까?",
            "너는 이제 마스터다! 무슨 일부터 시작할까?",
            "너는 마스터다.\n오늘 작업 목록은 뭐로 잡을까?",
            "너는 마스터다; 첫 작업은 뭐로 할까?",
            "'설계 문서'를 예시로 보여줘. 그리고 너는 마스터다."]
    skip = ["'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
            "너는 마스터가 아니다", "너는 마스터다 처럼 들리는 문장을 만들어줘"]
    for p in fire:
        v = D.detect(p)
        need(v["fire"], "혼합의도 미발화(A4 회귀) %r → %s" % (p, v["reason"]))
    for p in skip:
        v = D.detect(p)
        need(not v["fire"], "억제 케이스 오발화(과교정) %r → %s" % (p, v["reason"]))
    # 훅 배선 교차 확인 — 함수는 맞는데 배선이 틀린 구멍 차단
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        r = _run_rb(env, prompt=fire[0])
        need("발화됨" in r.stdout, "훅 배선에서 혼합의도 미발화: %r" % r.stdout[:300])
        calib = _calib_note(_old_hook_fires(fire[0], tmp), False, "혼합의도 프롬프트")
    return "혼합의도 %d FIRE / 억제 %d SKIP + 훅 교차 · 계측검증=%s" % (len(fire), len(skip), calib)


@specimen("H-DETECT-2", "W1b", "'네가/니가/당신이 마스터다' FIRE + 인접 의문 SKIP", ["P3-A-NEGA"])
def h_detect_2():
    """P3-A-NEGA: 표준 정서법 주어가 어휘에 없어 전부 미발화했다. '네'·'당신' 단독 과확장은 금지."""
    D = _detect_mod()
    for p in ("네가 마스터다", "니가 마스터다", "당신이 마스터다", "네가 이제 마스터야",
              "지금부터 네가 마스터가 된다"):
        v = D.detect(p)
        need(v["fire"], "표준 정서법 주어 미발화(P3-A-NEGA 회귀) %r → %s" % (p, v["reason"]))
    for p in ("'네가 마스터다'가 무슨 뜻?", "'니가 마스터다'가 무슨 의미인지 설명해줘",
              "\"당신이 마스터다\"라고 입력하면 어떻게 되나요?"):
        need(not D.detect(p)["fire"], "인접 의문인데 발화 %r" % p)
    for p in ("네 마스터 브랜치를 봐줘", "당신 마스터키 어디 뒀어"):
        need(not D.detect(p)["fire"], "'네'·'당신' 단독 과확장 발화 %r" % p)
    need("네가" in D.SUBJECT and "니가" in D.SUBJECT and "당신이" in D.SUBJECT,
         "주어 어휘에 네가/니가/당신이가 없다: %s" % D.SUBJECT)
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt="네가 마스터다").stdout, "훅 배선에서 '네가 마스터다' 미발화")
        calib = _calib_note(_old_hook_fires("네가 마스터다", tmp), False, "'네가 마스터다'")
    return "주어 5 FIRE / 인접의문 3 SKIP / 단독 과확장 2 SKIP · 계측검증=%s" % calib


@specimen("H-DETECT-3", "W1b", "filler 경계 15자=발화/16자=미발화", ["P3-A-FILLER"])
def h_detect_3():
    """P3-A-FILLER: 주석(12) ≠ 코드(15) 불일치를 상수 FILLER_MAX 로 못박고 경계를 박제한다.
    ★수치는 **이관 시 보존**돼야 한다 — 구 grep 과 신 함수의 경계가 같음을 구 훅 실측으로 대조한다."""
    D = _detect_mod()
    need(D.FILLER_MAX == 15, "FILLER_MAX 스펙 이탈: %r≠15" % D.FILLER_MAX)
    ok15 = "너는" + "가" * 15 + "마스터다"
    no16 = "너는" + "가" * 16 + "마스터다"
    need(D.detect(ok15)["fire"], "filler 15자 경계 미발화")
    need(not D.detect(no16)["fire"], "filler 16자 오발화(경계 누수)")
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt=ok15).stdout, "훅에서 filler 15 미발화")
        need("발화됨" not in _run_rb(env, prompt=no16).stdout, "훅에서 filler 16 오발화")
        # 경계 파리티: 구 grep 도 15=발화 / 16=미발화 였다(이관이 수치를 바꾸지 않았다는 증명)
        c15 = _calib_note(_old_hook_fires(ok15, tmp), True, "filler 15자")
        c16 = _calib_note(_old_hook_fires(no16, os.path.join(tmp, "b")), False, "filler 16자")
    return "15=FIRE·16=SKIP(함수·훅) · 구 grep 파리티: %s / %s" % (c15, c16)


@specimen("H-DETECT-4", "W1b", "LC_ALL=C 파리티(UTF-8과 동일 판정)", ["G9"])
def h_detect_4():
    """G9: `grep -E '.{0,15}'` 는 C 로케일에서 **바이트**를 세어 한글 filler 창이 1/3로 줄었다.
    python `re` 는 코드포인트 기반이고, 감지기는 stdin 을 바이트로 읽어 UTF-8 명시 디코드한다."""
    D = _detect_mod()
    probes = ["너는 이제 마스터다", "너는" + "가" * 15 + "마스터다", "네가 마스터다"]
    cenv = {"LC_ALL": "C", "LANG": "C", "LC_CTYPE": "C"}
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        cenv_full = dict(env, **cenv)
        for p in probes:
            need("발화됨" in _run_rb(cenv_full, prompt=p).stdout,
                 "LC_ALL=C 에서 미발화(G9 회귀): %r" % p)
        need("발화됨" not in _run_rb(cenv_full, prompt="'너는 마스터다'가 무슨 뜻이야?").stdout,
             "LC_ALL=C 에서 억제 케이스가 오발화(로케일 의존 잔존)")
        # 감지기 CLI 단독 파리티(훅 밖에서도 로케일 비의존인지)
        r = _run([PY, os.path.join(BIN_DIR, "javis_detect.py"), "hook-gate"],
                 input=json.dumps({"prompt": probes[1]}, ensure_ascii=False),
                 env=_base_env(cenv))
        need(r.returncode == 0, "LC_ALL=C 에서 감지기 CLI 가 발화하지 않았다(rc=%d): %s"
             % (r.returncode, r.stderr[-300:]))
        # 계측 타당성: 구 훅은 같은 환경에서 **미발화**해야 한다(바이트 계수 결함 재현)
        calib = _calib_note(_old_hook_fires(probes[1], tmp, env_extra=cenv), False,
                            "LC_ALL=C + 한글 filler 15자")
    need(D.detect(probes[1])["fire"], "UTF-8 판정과 불일치")
    return "LC_ALL=C 에서 %d FIRE / 억제 1 SKIP / CLI 파리티 · 계측검증=%s" % (len(probes), calib)


@specimen("H-DETECT-5", "W1b", "200자 창 python측 문자 단위 슬라이스", ["G25"])
def h_detect_5():
    """G25: `cut -c1-200` 은 GNU 에서 **항상 바이트**(BSD 도 C 로케일이면 바이트)라 한글 감지창이
    약 66자로 축소됐다. 해법은 셸 슬라이스 **제거** — python 이 원문을 문자 단위로 자른다."""
    D = _detect_mod()
    need(D.WINDOW_CHARS == 200, "WINDOW_CHARS 스펙 이탈: %r≠200" % D.WINDOW_CHARS)
    decl = "너는 마스터다"
    inside = "가" * (D.WINDOW_CHARS - len(decl)) + decl
    outside = "가" * D.WINDOW_CHARS + decl
    need(len(inside.encode("utf-8")) > D.WINDOW_CHARS,
         "검체가 바이트 기준으로도 창 안이라 회귀를 못 잡는다(검체 무효)")
    need(D.detect(inside)["fire"], "문자 200 경계 미발화(바이트 슬라이스 회귀)")
    need(not D.detect(outside)["fire"], "감지창 밖 선언이 발화(창 미적용)")
    # 셸 슬라이스 제거 확인(정적) — 훅에 cut -c·bashism 슬라이스가 남아 있으면 회귀다
    hook = _code_lines(_read(_hook("role-bootstrap.sh")))
    need("cut -c" not in hook, "훅에 `cut -c` 바이트 슬라이스가 남았다(G25 미수리)")
    need(not re.search(r"\$\{[A-Za-z_][A-Za-z0-9_]*:\d+:", hook), "훅에 bashism 슬라이스가 남았다")
    need("tr '\\n' ' '" not in hook, "훅에 개행 평탄화가 남았다(창 판정이 두 곳에 존재)")
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt=inside).stdout, "훅에서 문자 200 경계 미발화")
        need("발화됨" not in _run_rb(env, prompt=outside).stdout, "훅에서 창 밖 선언 발화")
        # 계측 타당성: 구 훅은 바이트 절단 조건(C 로케일)에서 **미발화**해야 한다
        calib = _calib_note(_old_hook_fires(inside, tmp,
                                            env_extra={"LC_ALL": "C", "LANG": "C", "LC_CTYPE": "C"}),
                            False, "한글 194자 프리픽스 + 선언(바이트 절단 조건)")
        old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
        if old is not None:
            need("cut -c1-200" in old, "계측 타당성 실패: 구 코드에 cut -c 슬라이스가 없다")
    return "문자 200 경계 FIRE/창밖 SKIP(함수·훅) · 셸 슬라이스 제거 확인 · 계측검증=%s" % calib


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


@specimen("H-DETECT-7", "W1b", "role 게이트 allowlist 행렬(worker-2·cso-1·미지 role)", ["A3=B7"])
def h_detect_7():
    """A3=B7: 구 denylist(`worker|cso|reviewer-*|reviewer`)는 **열거 밖 전부 통과**였다 —
    데몬이 실제 발권하는 worker-2(dedup)·cso-1·reviewer-claude-1·미지 role 이 마스터 부트를
    오발화했다. 수정은 열거 확장이 아니라 **allowlist 반전**(master 또는 빈 좌석만 발화)이다."""
    blocked = ["worker", "worker-2", "cso", "cso-1", "reviewer-gemini", "reviewer-codex",
               "reviewer-claude-1", "reviewer-grok", "verifier", "unknown-role", "ceo"]
    allowed = ["master", ""]
    with tempfile.TemporaryDirectory() as tmp:
        for i, role in enumerate(blocked):
            sb = os.path.join(tmp, "b%d" % i)
            env, _h, _p, _b, _s = _rb_sandbox(sb)
            _mock_cys(os.path.join(sb, "bin"), sb,
                      'case "$1" in surface-role) echo "%s"; exit 0;; esac' % role)
            r = _run_rb(env)
            need("발화됨" not in r.stdout,
                 "A3 회귀: role=%s 에서 마스터 부트 오발화(denylist 잔존): %r" % (role, r.stdout[:200]))
            need("allowlist" in r.stderr or "비-master" in r.stderr,
                 "role=%s 차단이 침묵이다(로그 없음): %r" % (role, r.stderr[:200]))
        for i, role in enumerate(allowed):
            sb = os.path.join(tmp, "a%d" % i)
            env, _h, _p, _b, _s = _rb_sandbox(sb)
            _mock_cys(os.path.join(sb, "bin"), sb,
                      'case "$1" in surface-role) echo "%s"; exit 0;; esac' % role)
            need("발화됨" in _run_rb(env).stdout,
                 "정상 좌석(role=%r)에서 발화하지 않았다(과차단)" % role)
        # 계측 타당성: 구 훅은 worker-2 에서 **발화**했어야 한다(결함 재현)
        sb = os.path.join(tmp, "calib")
        os.makedirs(sb, exist_ok=True)
        old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
        calib = "skip(no-git)"
        if old is not None:
            oldhook = os.path.join(sb, "oldhooks", "role-bootstrap.sh")
            _w(oldhook, old)
            _w(os.path.join(sb, "oldhooks", "_lib.sh"),
               _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            env, _h, _p, _b, _s = _rb_sandbox(os.path.join(sb, "sb"))
            _mock_cys(os.path.join(sb, "sb", "bin"), os.path.join(sb, "sb"),
                      'case "$1" in surface-role) echo "worker-2"; exit 0;; esac')
            r = _run(["bash", oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=env)
            need("발화됨" in r.stdout,
                 "계측 타당성 실패: 구 코드가 worker-2 에서 오발화하지 않는다 — 검체가 결함을 재현 못함")
            calib = "구 코드 worker-2 오발화 재현"
    return "차단 %d role(미지 role 포함)·허용 2 · 계측검증=%s" % (len(blocked), calib)


@specimen("H-DETECT-8", "W1b", "surface-role 판정불가 → fail-closed+loud", ["A5"])
def h_detect_8():
    """A5: `cys surface-role` 이 판정 불가(rc≠0·hang)일 때 구 코드는 MYROLE 을 **빈값**으로 얻어
    '미claim' 으로 오통과시켰다(빈값 오통과). 3상화: rc≠0=무발화+로그 / 빈값=미claim 통과.
    ★timeout 단독 적용은 hang 을 오발화로 바꾸는 악화이므로 데드라인과 3상화를 함께 검증한다."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ rc≠0(판정 불가) → 무발화 + 로그
        sb = os.path.join(tmp, "rc")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) echo ""; exit 3;; esac')
        r = _run_rb(env)
        need("발화됨" not in r.stdout, "판정 불가(rc=3)인데 발화(fail-open 잔존)")
        need("판정 불가" in r.stderr, "판정 불가가 침묵으로 접혔다: %r" % r.stderr[:200])
        notes.append("rc≠0 무발화+로그")
        # ⓑ hang → 데드라인으로 유한 종료 + 무발화(훅이 프롬프트를 무한정 붙잡지 않는다)
        sb = os.path.join(tmp, "hang")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) sleep 30; echo ""; exit 0;; esac')
        t0 = time.time()
        r = _run_rb(env)
        dt = time.time() - t0
        need(dt < 20, "hang 이 데드라인으로 끊기지 않았다(%.1fs) — 훅이 프롬프트를 붙잡는다" % dt)
        need("발화됨" not in r.stdout, "hang(판정 불가)인데 발화 — timeout 단독 적용 악화 형태")
        notes.append("hang %.1fs 내 종료·무발화" % dt)
        # ⓒ 빈값(정상 미claim)은 통과해야 한다 — 3상 분리의 반대편(과차단 금지)
        sb = os.path.join(tmp, "empty")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        need("발화됨" in _run_rb(env).stdout, "빈값(미claim)이 차단됐다 — rc≠0 과 융합(과차단)")
        notes.append("빈값 미claim 통과")
    # 계측 타당성: 프리루드에 데드라인 실행기가 존재하고 훅이 그것으로 감싼다
    need("cys_timeout_run()" in _read(os.path.join(HOOKS_DIR, "_lib.sh")),
         "프리루드에 데드라인 실행기(cys_timeout_run)가 없다")
    need("cys_timeout_run" in _read(_hook("role-bootstrap.sh")),
         "훅이 surface-role 을 데드라인으로 감싸지 않는다")
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need("cys_timeout_run" not in old and 'MYROLE="$(cys surface-role' in old,
             "계측 타당성 실패: 구 코드가 이미 데드라인·3상화를 갖고 있다면 이 검체는 무의미")
        calib = "구 코드 무-데드라인·빈값 오통과 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DETECT-9", "W1b", "python 부재 → 정적 loud(cannot-judge 분리)", ["A22"])
def h_detect_9():
    """A22: 인터프리터 미해소는 '선언 아님'이 아니라 **판정 불가**다. 구 코드는 `|| CYS_PY=python3`
    으로 채워 존재하지 않는 인터프리터를 향해 파싱을 시도하고 조용히 exit 0 했다(cannot-judge 침묵).
    ★judged-no 와의 분리: 프롬프트에 마스터 토큰이 아예 없으면 침묵이 정당하다(스팸 금지)."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # PATH 에서 python 전부 제거 + CYS_PY 빈값 강제 → cannot-judge
        sb = os.path.join(tmp, "nopy")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        binp = os.path.join(sb, "onlybin")
        os.makedirs(binp, exist_ok=True)
        for tool in ("bash", "sh", "cat", "grep", "printf", "tr", "head", "date", "mkdir",
                     "rm", "ls", "ln", "sleep", "dirname", "sed", "command", "kill", "wait"):
            src = shutil.which(tool)
            if src and not os.path.exists(os.path.join(binp, tool)):
                os.symlink(src, os.path.join(binp, tool))
        # 목 cys 를 python 없는 PATH 에 복사(feed/send 시도 흔적 회수용)
        _mock_cys(binp, sb, 'case "$1" in surface-role) echo ""; exit 0;; esac')
        env["PATH"] = binp
        env["CYS_PY"] = ""
        r = _run_rb(env)
        need(r.returncode == 0, "python 부재에서 훅이 비0 종료(exit=%d)" % r.returncode)
        need("발화됨" not in r.stdout, "python 부재인데 '발화됨' 보고")
        need("판정 불가" in r.stdout and "python" in r.stdout,
             "cannot-judge 가 정적 loud 로 나오지 않았다(침묵 접힘): %r" % r.stdout[:300])
        need("hookSpecificOutput" in r.stdout, "정적 additionalContext 미출력(python 없이 발행 실패)")
        json.loads(r.stdout.strip().splitlines()[-1])   # 정적 JSON 이 유효 JSON 인가
        need("feed push" in _calls(sb) or "send --queued" in _calls(sb),
             "판정 불가인데 승인 채널 알림 시도가 없다: %r" % _calls(sb)[-300:])
        notes.append("cannot-judge 정적 loud+알림")
        # judged-no: 마스터 토큰 없는 프롬프트는 침묵(스팸 금지)
        r2 = _run_rb(env, prompt="오늘 작업 지시해줘")
        need(r2.stdout.strip() == "", "선언 토큰 없는 프롬프트에 컨텍스트를 주입했다(스팸): %r" % r2.stdout[:200])
        notes.append("judged-no 침묵")
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need('CYS_PY="$(command -v python3 || command -v python || command -v py || echo python3)"' in old
             or "|| CYS_PY=python3" in old or '|| CYS_PY="python3"' in old,
             "계측 타당성 실패: 구 코드에 '존재하지 않는 인터프리터로 채우기'가 없다")
        calib = "구 코드 인터프리터 기본값 채움 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DETECT-10", "W1a", "pre-exec 사망 목 → '발화 실패' 상태파생 보고(허위 '발화됨' 금지)", ["A6"])
def h_detect_10():
    real = shutil.which("python3") or PY
    # 목 인터프리터: `-c`(NOTE 생성)와 **감지기 스크립트**(W1b: 선언 판정이 python 단일 함수로
    # 이관됐다)는 정상 위임하고, **부트 스크립트 실행만 exec 실패(127)** 로 만든다.
    # ★목을 좁힌 이유: 이 검체가 재는 것은 '부트 발화의 pre-exec 사망'이지 '감지 불가'가 아니다.
    #   감지기까지 죽이면 훅이 A22 cannot-judge 분기로 빠져 A6 표면을 아예 통과하지 않는다.
    mock = ("#!/bin/sh\n"
            'case "$1" in\n'
            '  -c) exec "%s" "$@" ;;\n'
            "  *javis_detect.py) exec \"%s\" \"$@\" ;;\n"
            "esac\n"
            "exit 127\n" % (real, real))
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
def _boot_sandbox(tmp, *, cys_extra="", check_exit=0):
    """javis_bootstrap.py 격리 실행 환경(HOME+가짜 팩+스텁 cys). 반환 (env, home, tmp)."""
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack")
    bindir = os.path.join(tmp, "stubbin")
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    _mock_cys(bindir, tmp, cys_extra)
    _w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
    _w(os.path.join(pack, "bin", "javis_orchestra.py"),
       "import sys; sys.exit(%d)\n" % check_exit, 0o644)
    env = _base_env({"HOME": home, "PATH": bindir + os.pathsep + os.environ.get("PATH", ""),
                     "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "1",
                     "CYS_BOOT_CHECK_INTERVAL_S": "0.05"})
    return env, home


def _boot_last(home):
    return json.loads(_read(os.path.join(home, ".cys", "state", "boot-last.json")) or "{}")


@specimen("H-EXIT-1", "W1b",
          "cmd_run 종료 불변식(stdout JSON / stderr verdict·run_id·귀속) + A20 소비 분기",
          ["A7", "A19", "A20(소비층)", "CS-2⑩"])
def h_exit_1():
    """A7·A19·CS-2⑩: 종료 채널 분리 · 런 정체성 · 정당거부의 boot-last 오염 0.

    ※A20 은 두 층이다 — CLI 타입드 exit 표(cys.rs)는 **W2**, bootstrap 의 **소비 분기**
      (거부 마커 유무로 exit 7 / 10 분리)는 W1b 다. 여기서 재는 것은 후자다(H-EXIT-3=W2 유지).
    """
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    notes = []
    src = _read(boot)
    need("def _cmd_run_chain(" in src and "log.finish(" in src,
         "cmd_run 이 try/finally 종결 기록 구조로 분리되지 않았다(A19)")
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ 완주 → stdout 최종 JSON(구 산문 계약 보존) + 귀속·종결 기록
        env, home = _boot_sandbox(os.path.join(tmp, "ok"))
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 0, "완주 경로 exit≠0: %d\n%s" % (r.returncode, r.stderr[-500:]))
        summary = json.loads(r.stdout.strip().splitlines()[-1])
        need(summary.get("ok") is True, "stdout 최종 JSON 이 성공 계약을 잃었다: %r" % summary)
        bl = _boot_last(home)
        for k in ("run_id", "pid", "surface", "ended", "exit"):
            need(k in bl, "boot-last 에 런 귀속·종결 필드 누락: %s (%r)" % (k, sorted(bl)))
        need(bl["surface"] == "7", "surface 귀속이 틀렸다: %r" % bl.get("surface"))
        need(bl["exit"] == 0 and (bl.get("result") or {}).get("state") == "completed",
             "완주 상태 기록 이상: %r" % bl.get("result"))
        need((bl["result"].get("run_id") or "") == bl["run_id"],
             "result 에 run 귀속이 없다(§0 '자기 surface 최신 완주 런' 판독 불가)")
        notes.append("완주=stdout JSON+귀속+종결")

        # ⓑ 정당거부(claim_denied) → exit 7 · **ok:false 오염 0**(state=declined)
        env, home = _boot_sandbox(
            os.path.join(tmp, "deny"),
            cys_extra=('case "$1" in claim-role) echo "claim_denied: privileged role held by '
                       'live surface" >&2; exit 1;; esac'))
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 7, "정당거부 exit≠7: %d" % r.returncode)
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is None and res.get("state") == "declined",
             "정당거부가 공유 boot-last 에 ok:false 를 덮었다(CS-2⑩ 회귀): %r" % res)
        need(res.get("surface") == "7", "거부 기록에 surface 귀속이 없다: %r" % res)
        notes.append("거부=exit 7·ok:null(오염 0)")

        # ⓒ 거부 마커 없는 claim 실패 → **exit 10 세션 컨텍스트 오류**(A20 소비 분기)
        env, home = _boot_sandbox(
            os.path.join(tmp, "ctx"),
            cys_extra='case "$1" in claim-role) echo "error: no surface" >&2; exit 4;; esac')
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 10, "거부 마커 없는 실패가 exit 10 으로 분리되지 않았다: %d" % r.returncode)
        need("세션 컨텍스트 오류" in r.stderr, "exit 10 문구가 없다: %r" % r.stderr[-300:])
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is None and res.get("state") == "session_error",
             "세션 컨텍스트 오류가 ok:false 를 덮었다: %r" % res)
        notes.append("마커 없는 실패=exit 10·ok:null")

        # ⓓ 인프라 실패(ping)는 여전히 ok:false — 과교정 금지(이건 실제로 깨진 부트다)
        env, home = _boot_sandbox(os.path.join(tmp, "ping"),
                                  cys_extra='case "$1" in ping) exit 1;; esac')
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 3, "ping 실패 exit≠3: %d" % r.returncode)
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is False and res.get("state") == "failed",
             "인프라 실패가 ok:false 를 잃었다(과교정): %r" % res)
        notes.append("인프라 실패=ok:false 유지")

        # ⓔ 진행 중 선기록(A19) — 러닝 상태에서 강제 종료해도 'running' 이 남는다
        env, home = _boot_sandbox(os.path.join(tmp, "run"),
                                  cys_extra='case "$1" in ping) sleep 20;; esac')
        pr = subprocess.Popen([PY, boot], env=env, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL)
        deadline = time.time() + 15
        bl = {}
        while time.time() < deadline:
            bl = _boot_last(home)
            if bl.get("run_id"):
                break
            time.sleep(0.1)
        pr.kill(); pr.wait(timeout=15)
        need(bl.get("run_id"), "시작 시점 선기록이 없다(진행 중 상태 판별 불가)")
        need((bl.get("result") or {}).get("state") == "running",
             "선기록 상태가 running 이 아니다: %r" % bl.get("result"))
        notes.append("running 선기록(SIGKILL 생존)")
    # 계측 타당성: 구 코드는 종결·귀속 필드가 없었다
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    calib = "skip(no-git)"
    if old is not None:
        need('"result"] = {"ok": True}' in old or '{"ok": True}' in old,
             "계측 타당성 실패: 구 코드 성공 기록 형태를 못 찾았다")
        need("run_id" not in old, "계측 타당성 실패: 구 코드에 이미 run_id 가 있다")
        need("log.finish(" not in old, "계측 타당성 실패: 구 코드에 이미 종결 기록이 있다")
        calib = "구 코드 run_id·종결 기록 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
pending("H-EXIT-2", "W2", "boot Busy → busy 판정 구분 + CEO 티켓 보존", ["G11"])
pending("H-EXIT-3", "W2", "claim-role 타입드 exit 표(0/7/3/2)", ["A20"])
pending("H-EXIT-4", "W2", "boot --json 스키마(mandatory·outcome·install_hint)", ["G29", "B15", "B16", "R5"])
pending("H-EXIT-5", "W2", "미지 subcommand EX_USAGE + 게이트 exit 2 충돌 해소", ["A13", "A14"])
pending("H-EXIT-6", "W2", "orchestra 2/127=영구실패 · 124=재시도 분류", ["A12"])
pending("H-EXIT-7", "W2", "check exit 2(데몬 소실) 별도 분기", ["G32"])
pending("H-EXIT-8", "W2", "discover sentinel(격리 팩 vs 미발견 분리)", ["G1"])
@specimen("H-EXIT-9", "W1b", "싱글플라이트 패자 → 비-master skip verdict(즉시 반환)", ["G17", "A7"])
def h_exit_9():
    """G17: 패자 pane 이 **신원 확인 없이** 조용히 exit 0 하면 그 pane 의 LLM 이 '부트 완료된
    master'를 자칭한다. verdict 에 자기 surface-role 확인 결과를 동봉하고 '비-master'를 명시한다.
    ★즉시 반환(waited=false) — 수렴 대기 금지(금지 방향 ⑨)."""
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    with tempfile.TemporaryDirectory() as tmp:
        env, home = _boot_sandbox(
            tmp, cys_extra=('case "$1" in ping) sleep 8;; surface-role) echo "worker"; exit 0;; esac'))
        winner = subprocess.Popen([PY, boot], env=env, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL)
        try:
            # 승자가 락을 잡을 시간을 준다(선기록 등장으로 확인)
            deadline = time.time() + 15
            while time.time() < deadline and not _boot_last(home).get("run_id"):
                time.sleep(0.1)
            winner_run = _boot_last(home).get("run_id")
            t0 = time.time()
            r = _run([PY, boot], env=env, timeout=60)
            dt = time.time() - t0
        finally:
            winner.kill(); winner.wait(timeout=15)
        need(r.returncode == 11, "패자 exit≠11(구분 exit 부재): %d" % r.returncode)
        need(r.stdout.strip() == "",
             "패자가 stdout(계약 채널)을 오염시켰다 — master 가 '완료'로 인용할 위험: %r" % r.stdout[:300])
        line = [l for l in r.stderr.splitlines() if l.strip().startswith("{")]
        need(line, "stderr 1줄 verdict JSON 이 없다: %r" % r.stderr[-400:])
        v = json.loads(line[-1])
        need(v.get("verdict") == "skipped_inflight", "verdict 타입 이상: %r" % v)
        need(v.get("surface_role") == "worker" and v.get("is_master") is False,
             "패자가 자기 신원을 확인하지 않았다(G17 회귀): %r" % v)
        need("비-master" in (v.get("self_check") or ""),
             "'비-master' 명시가 없다(자칭 master 차단 실패): %r" % v.get("self_check"))
        need(v.get("waited") is False and dt < 6,
             "패자가 수렴 대기를 했다(%.1fs · 금지 방향 ⑨): %r" % (dt, v))
        need(v.get("boot_last_untouched") is True, "skip 이 단일-writer 보존을 주장하지 않는다")
        need(_boot_last(home).get("run_id") == winner_run,
             "패자가 승자의 boot-last 를 덮었다(단일-writer 파괴)")
        need(os.path.isfile(os.path.join(home, ".cys", "state", "boot-skip-base.json")),
             "skip 별도 기록 파일이 없다")
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    calib = "skip(no-git)"
    if old is not None:
        need("_emit_skip_verdict" not in old, "계측 타당성 실패: 구 코드에 이미 skip verdict 가 있다")
        calib = "구 코드 침묵 skip(exit 0) 확인"
    return "exit 11 · stdout 무접촉 · 비-master 명시 · waited=false · 별도 기록 · 계측검증=%s" % calib


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
@specimen("H-CONC-5", "W1b", "하네스 pgid 실측(결정 실험 — 처방 아닌 측정)", ["A18"])
def h_conc_5():
    """A18 은 **측정**이 게이트다(처방 아님 — os.setsid 선제 내재화는 철회됐다).

    이 검체는 두 가지만 강제한다:
      ① 결정 실험 스크립트가 실재하고 결정론적으로 돈다(측정 실패=hard fail).
      ② **계측 타당성**: 세션 분리 스폰(대조군)이 같은 group-kill 에서 생존한다 — 그래야
         '훅 분기가 사망'이라는 관측치가 의미를 갖는다(항상-사망 프로브 배제).
    판정(group-kill 노출 여부) 자체는 결함 심각도 결정(P3 유지 vs P2 복귀)이며 master 소관이다 —
    여기서 fail 로 만들지 않고 **detail 에 실측치를 실어** 게이트 출력에 남긴다.
    """
    probe = os.path.join(TESTS_DIR, "probe_pgid.py")
    need(os.path.isfile(probe), "A18 결정 실험 스크립트 부재: %s" % probe)
    if os.name != "posix":
        raise Skip("posix 전용 측정(Windows 프로세스 그룹은 H-WIN-11 소속)")
    r = _run([PY, probe, "--json"], timeout=180)
    need(r.returncode == 0, "측정 실패(exit=%d): %s" % (r.returncode, r.stderr[-400:]))
    m = json.loads(r.stdout)
    need("error" not in m, "측정 오류: %s" % m.get("error"))
    ca = m.get("control_arm") or {}
    need(ca.get("ok") is True,
         "계측기 고장 — 대조군(세션 분리 스폰)이 group-kill 에서 생존하지 못했다: %r" % ca)
    need(isinstance(m.get("child_survived_group_kill"), bool), "관측치 누락: %r" % m)
    return ("훅 분기=%s(setsid=%s) · pgid 분리=%s · group-kill 생존=%s → %s · 대조군 생존 확인"
            % (m.get("hook_branch"), m.get("setsid_available"), m.get("pgid_separated"),
               m.get("child_survived_group_kill"), m.get("verdict")))


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
_CLAUDE_MD_COPIES = ("CLAUDE.md", os.path.join("cysjavis-pack", "CLAUDE.md.template"))
_HOOK_FIRED_MARK = "[결정론 부트스트랩 발화됨 — 하네스 강제]"


def _repo_file(rel):
    p = os.path.join(REPO_DIR, rel)
    if not os.path.isfile(p):
        raise Skip("레포 파일 부재(배포 팩 실행): %s" % rel)
    return _read(p)


def _no_wait_for_owner(text, where):
    """'오너 지시 대기' 문구 금지(H-DOC-1) — 단, **폐기 선언**으로 인용한 것은 허용한다.
    ★규칙을 '문자열 부재'로 두면 폐기 선언 자체를 쓸 수 없다(문서가 결함을 설명 못 한다) →
      출현마다 근처(±40자)에 부정 마커(폐기/아니라/금지)가 있어야 한다는 형태 규칙으로 판정한다."""
    for m in re.finditer(r"오너 지시 대기|오너의? 지시를 기다|오너 지시를 받아", text):
        window = text[max(0, m.start() - 40):m.end() + 40]
        need(any(k in window for k in ("폐기", "아니라", "금지")),
             "%s 에 '오너 지시 대기' 계열 문구가 살아 있다(§0 ⑥·앵커6 축1 위반): …%s…"
             % (where, window.replace("\n", " ")))


@specimen("H-DOC-1", "W1b", "§0↔훅 note↔template(사본 2벌) 문안 정합", ["A10", "P3-A-TEMPLATE"])
def h_doc_1():
    """A10: 부트 실행 주체 계약이 §0(디렉티브)·훅 note·template 3곳에 사본으로 살면서 서로
    달랐다. 계약을 §0-A 단일 표로 모으고 나머지는 **포인터**로 만든 뒤, 그 정합을 기계로 못박는다.
    P3-A-TEMPLATE: 훅 없는 기계의 폴백 절차("스크립트 1회")가 보존돼야 한다 — 산문 체인 금지."""
    md = _repo_file(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md"))
    hook = _read(_hook("role-bootstrap.sh"))
    notes = []
    # ① §0-A 조건부 단일 계약이 존재하고 두 분기를 명시하는가
    need("0-A" in md and "실행 주체 단일 계약" in md, "§0-A 단일 계약 절이 없다(A10 미착지)")
    need(_HOOK_FIRED_MARK in md, "§0-A 가 훅 컨텍스트 신호 문구를 인용하지 않는다(정합 불가)")
    need("재실행 금지" in md, "§0-A 에 재실행 금지 분기가 없다")
    need("1회" in md and "javis_bootstrap.py" in md, "§0-A 에 '스크립트 1회' 폴백이 없다")
    need("수동 재현 금지" in md or "산문 체인" in md or "손으로 치는" in md,
         "§0-A 가 개별 명령 수동 재현 금지를 명문하지 않는다")
    notes.append("§0-A 조건부 계약")
    # ② 훅 note 가 같은 신호 문구를 쓰고 잔여 의무를 가리키는가(문안 정합의 핵심 축)
    need(_HOOK_FIRED_MARK in hook, "훅 note 와 §0-A 의 신호 문구가 다르다(정합 파괴)")
    need("재실행하지 마라" in hook, "훅 note 에 재실행 금지가 없다")
    need("next-action" in hook, "훅 note 가 next-action 자율 착수를 가리키지 않는다")
    _no_wait_for_owner(_code_lines(hook), "훅 note")   # 주석(수정 이력 설명)은 스캔 제외
    notes.append("훅 note 신호·잔여의무 정합")
    # ③ template 2벌 사본이 §0 포인터 + 폴백 보존 문구를 갖고, **서로 동일**한가
    bodies = []
    for rel in _CLAUDE_MD_COPIES:
        t = _repo_file(rel)
        need("0-A" in t, "%s 가 §0-A 포인터를 갖지 않는다" % rel)
        need("javis_bootstrap.py" in t and "1회" in t,
             "%s 에 훅 미발화 폴백(스크립트 1회)이 없다(P3-A-TEMPLATE)" % rel)
        need("산문 체인" in t, "%s 에 '산문 체인 금지' 보존 문구가 없다(P3-A-TEMPLATE)" % rel)
        need("javis_preflight.py" not in t.split("## 터미널")[0],
             "%s 부트절이 아직 preflight 산문 지시를 담고 있다(A10 미수리)" % rel)
        _no_wait_for_owner(t, rel)
        bodies.append(t.split("## 터미널")[0])
    need(bodies[0] == bodies[1],
         "template 2벌 사본의 부트절이 갈렸다(사본 드리프트) — 길이 %d vs %d"
         % (len(bodies[0]), len(bodies[1])))
    notes.append("template 2벌 동일·폴백 보존")
    # 계측 타당성: 구 문서는 산문 부트 지시를 담고 있었다
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/CLAUDE.md.template")
    if old is not None:
        head = old.split("## 터미널")[0]
        need("javis_preflight.py" in head,
             "계측 타당성 실패: 구 template 부트절에 산문 preflight 지시가 없다")
        calib = "구 template 산문 부트 지시 확인"
    oldmd = _git_show("cysjavis-pack/directives/MASTER_DIRECTIVE.md")
    if oldmd is not None:
        need("실행 주체 단일 계약" not in oldmd,
             "계측 타당성 실패: 구 §0 에 이미 단일 계약이 있다")
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DOC-6", "W1b",
          "§0 ③ 및 §9 상태 경로가 CYS_PACK_DIR 파생(base 하드코딩 0) "
          "— 재태깅 W4→W1b: W1b master 지시로 :225 동류 교정이 착지",
          ["G5"])
def h_doc_6():
    """G5(및 그 동류): 디렉티브가 base 팩 경로를 하드코딩하면 부서장 레인이 **본부 상태**를
    읽고 쓴다(교차 오염). 레인 파생(`${CYS_PACK_DIR:-$HOME/.cys/pack}`)만 허용한다."""
    rel = os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md")
    md = _repo_file(rel)
    bad = [l.strip()[:120] for l in md.splitlines() if "~/.cys/pack" in l]
    need(not bad, "base 팩 경로 하드코딩 잔존(%d줄): %s" % (len(bad), bad[:5]))
    # $HOME/.cys/pack 은 **반드시** CYS_PACK_DIR 폴백 형태로만 등장해야 한다
    loose = [l.strip()[:120] for l in md.splitlines()
             if "$HOME/.cys/pack" in l and "CYS_PACK_DIR:-$HOME/.cys/pack" not in l]
    need(not loose, "CYS_PACK_DIR 폴백 없는 $HOME 경로 잔존: %s" % loose[:5])
    need("${CYS_PACK_DIR:-$HOME/.cys/pack}/round/SESSION_STATE.md" in md,
         "§9 SESSION_STATE 경로가 레인 파생이 아니다(:225 동류 미교정)")
    # 계측 타당성: 기준 커밋에는 하드코딩이 있었다
    calib = "skip(no-git)"
    old = _git_show(rel)
    if old is not None:
        oldbad = [l for l in old.splitlines() if "~/.cys/pack" in l]
        need(oldbad, "계측 타당성 실패: 구 문서에 base 하드코딩이 없다면 이 검체는 무의미")
        calib = "구 문서 하드코딩 %d줄 확인" % len(oldbad)
    return "하드코딩 0줄 · §9 SESSION_STATE·RECOVERY·TODO 레인 파생 · 계측검증=%s" % calib
pending("H-DOC-2", "W4", "'5노드'=REQUIRED_ROLES+1 파생", ["B18"])
pending("H-DOC-3", "W4", "CEO_TEMPLATE 동사↔cys-dept 가드 허용 집합 대조", ["G6"])
pending("H-DOC-4", "W4", "헤더 exit 표↔코드 상수 기계 대조 (결함 G31·G32 문구는 W0에서 수정 — 기계 대조 assert 는 CS-6 문서 정합 테스트=W4 산출)", ["G31", "G32"])
pending("H-DOC-5", "W4", "generic reviewer 안내 문구 금지 (결함 G30 문구는 W0에서 수정 — assert 는 W4)", ["G30"])
pending("H-DOC-7", "W4", "agents 스키마 완결성(vendor/user 계층)", ["B20"])
pending("H-DOC-8", "W4", "팀 부트 진입점 전수 단일 계약", ["B5"])
pending("H-SEED-1", "W3", "소망 훅 집합 동등성(Rust 시드/init-pack == SELFCORR_HOOKS)", ["A9"])
pending("H-SEED-2", "W3", "실사용 config dir 등록 hard 검증(CLAUDE_CONFIG_DIR 최우선)", ["A21", "R4"])
pending("H-SEED-3", "W3", "settings.json 없는 프로필 디렉토리 후보화", ["G7"])
pending("H-SEED-4", "W3", "cys-dept launch/rotate → CYS_ACCOUNT_DIR 복원 주입", ["G3"])
pending("H-SEED-5", "W3", "외부 동명 훅 보존(_prune 소유 술어)", ["G10"])
pending("H-LIFE-1", "W3", "레인별 marker/boot-last 분리 + run 귀속 교차 단언", ["G15", "P3-A-DEPT-LANE"])
pending("H-LIFE-2", "W3", "step id enum 유일성·기록 순서=실행 순서 "
        "(재태깅 W1b→W3: §4 웨이브 소속이 정본 — P3-A-STEP-NAME 은 W3 대상)",
        ["P3-A-STEP-NAME"])
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
