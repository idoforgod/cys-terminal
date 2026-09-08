#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_pyseal_negative_specimen.py — SEAL-1 **행동 검체**(음성 검체): 봉인 env 를 실은 인터프리터가
팩 모듈을 import 해도 팩/번들 트리에 `__pycache__`/`.pyc` 를 **한 개도** 만들지 않는다.

★왜 이 파일이 존재하는가(통합 단계 · 오너 참고1 · CONTRACTS §B-11 "음성 검체 1개 추가"):
  SEAL-1(2026-08-01 실사고 — 번들 python 이 번들 안에 `.pyc` 를 써서 코드서명 봉인이 깨지고
  Gatekeeper 가 "손상되었기 때문에 열 수 없습니다"로 차단 · 정본 `src/lib.rs ENV_PY_NO_BYTECODE`)
  의 기존 핀은 **전부 정적**이다 — Rust 는 `Command` 의 env 쌍을 보고(src/lib.rs `python_spawns_
  never_write_bytecode_into_the_bundle` · `bundled_python_spawn_sites_are_enumerated_and_sealed`),
  팩 census(`test_pyseal_census.py`)는 소스 줄을 센다. 둘 다 **"그 env 를 실었다"** 까지만 증명하고
  **"그 env 를 실은 인터프리터가 실제로 아무것도 안 쓴다"** 는 한 번도 실행으로 재지 않았다.
  그 사이에 남는 계급: env 이름/값 규약의 오해(빈 값=끔), 그 env 를 무시하는 인터프리터 빌드,
  캐시가 다른 데로 새는 변형. 이 파일이 그 마지막 한 칸을 **실행**으로 채운다.

★측정 타당성이 이 검체의 절반이다 — "0개 생성"은 아무 일도 안 일어났을 때도 참이다:
  ① import 가 실패했거나 ② 트리가 읽기 전용이거나 ③ **인터프리터가 애초에 in-tree 로 안 쓰는
  종류**이면, 봉인이 깨져 있어도 0이 나온다. ③ 은 가설이 아니라 실측이다 — macOS 의
  `/usr/bin/python3`(Xcode CLT)는 Apple 패치로 `sys.pycache_prefix` 가
  `~/Library/Caches/com.apple.python` 으로 **고정**돼 있어 `PYTHONDONTWRITEBYTECODE` 를 빼도
  in-tree `.pyc` 가 0이다(2026-09-08 실측). 그런 인터프리터에서의 "0개"는 봉인의 증거가 아니라
  **공허**다. 그래서 이 검체는 ⓑ 에서 인터프리터의 캐시 방향을 먼저 재고, 공허한 축은 PASS 가
  아니라 **사유를 밝힌 SKIP** 으로 내리며, 트리 축이 실제로 돈 실행에서는 ⓒ 음성 대조(봉인을
  뺀 env 로 `.pyc` 가 **생기는 것**을 관측)를 **의무**로 요구한다. 결측을 통과로 읽지 않는다.

축
  ⓐ 인터프리터 해소 — 번들 python(`<app>/Contents/Resources/runtime/python/bin/python3` ·
     win `runtime\\python\\python3.exe`) 우선, 로컬에 번들이 없으면 **팩이 실제로 쓰는 런타임
     python**(`CYS_PY` → PATH → 현재 인터프리터)으로 폴백하고 무엇을 썼는지 출력에 명기한다.
  ⓑ 캐시 방향 실측 — 선택한 인터프리터의 `sys.pycache_prefix` 를 잰다. None 이면 in-tree 관측
     가능(트리 축 유효), 값이 있으면 트리 축은 공허하므로 SKIP 사유로 내린다.
  ⓒ ★음성 대조(계측 타당성 · 트리 축이 도는 실행에서 의무) — 봉인을 **뺀** env 로 사본 트리에서
     같은 import → `.pyc` ≥ 1. ★이 축은 **번들 python 을 절대 쓰지 않는다**: 봉인을 뺀 번들
     인터프리터를 돌리는 것은 설치본 서명을 우리 손으로 깨는 짓이다.
  ⓓ0 봉인 env 를 인터프리터가 **실제로 따른다** — 데몬 주입 env 에서 `sys.dont_write_bytecode`
     가 참. 트리와 무관하게 항상 측정 가능한 직접 축이다(공허해지지 않는다).
  ⓓ 봉인 양성(사본 트리) — 데몬이 주입하는 env(`spawn_env_pairs` 무조건 쌍 = SEAL-1 + PYTHONUTF8)
     로 ⓐ 인터프리터가 같은 import → 사본 트리 캐시 신규 **0**.
  ⓔ 봉인 양성(실물 팩 트리) — 같은 env·같은 인터프리터가 **실물 팩 bin** 에서 import 하고 팩
     트리 전체를 실행 전후로 census 해 신규 0. 신규가 나오면 **정확히 그 경로만** 지우고 FAIL 한다
     (검체가 자기 오염을 남기지 않는다).
  ⓕ 봉인 양성(번들 런타임 트리) — ⓐ 가 번들 python 일 때만: stdlib import 로 번들 안에 캐시를
     쓰는 것이 2026-08-01 사고의 원형이므로 `Contents/Resources/runtime` 트리를 같은 실행 창에서
     census 해 신규 0. 번들이 아니면 SKIP 사유를 출력한다.
  ⓖ 정본 대조 — 레포 루트가 보이면 `src/lib.rs` 의 상수 두 줄(이름·켜짐 값)이 이 파일의 고정값과
     같은지 본다. 팩 단독 설치(루트 없음)에서는 SKIP 사유를 출력한다(무음 생략 금지).

검체 모듈은 **실물 팩 모듈**이다(합성 모듈 아님 — import 부작용 0 을 실측한 것만 고른다):
  `javis_runtime_seal.py` · `javis_idempotency.py` · `javis_lock.py`.

종료 코드(run_bootstrap_health·test_pyseal_census 규약과 동형): 0 = 전부 측정·통과(말미
`PYSEAL-SPECIMEN-OK`) · 1 = FAIL 1건 이상 · 2 = UNMEASURED(인터프리터·검체 모듈 해소 실패,
또는 트리 축이 돌았는데 음성 대조를 세울 인터프리터가 없음 = **측정 불능은 통과가 아니다**).
쓰기는 임시 디렉터리 안에서만 한다(실물 팩·번들은 읽기 전용 · ⓔ 의 자기 오염 청소 제외).
주입점(테스트 전용): `CYS_PYSEAL_SPECIMEN_PYTHON`(ⓐ 인터프리터 강제) ·
`CYS_PYSEAL_SPECIMEN_APP`(번들 .app 경로 강제) · `PYSEAL_CENSUS_ROOT`(레포 루트 · census 와 공용).

    python3 cysjavis-pack/bin/tests/test_pyseal_negative_specimen.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

# 로케일 비의존 출력(test_pyseal_census.py 동형): cp949 파이프 캡처에서 한글 진단 즉사 방지.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
PACK_BIN = os.path.dirname(TESTS_DIR)
PACK = os.path.dirname(PACK_BIN)
ROOT = os.path.abspath(os.environ.get("PYSEAL_CENSUS_ROOT") or os.path.dirname(PACK))

# 정본(src/lib.rs ENV_PY_NO_BYTECODE / PY_NO_BYTECODE_ON) — ⓖ 가 대조한다.
SEAL_ENV = "PYTHONDONTWRITEBYTECODE"
SEAL_ON = "1"
# 데몬이 함께 싣는 무조건 쌍(src/lib.rs ENV_PY_UTF8 / PY_UTF8_ON · W-B2) — "데몬이 주입하는 env"
# 를 그대로 재현하기 위한 것이다. 봉인 판정 자체에는 관여하지 않는다.
UTF8_ENV = "PYTHONUTF8"
UTF8_ON = "1"
# 캐시를 트리 밖으로 돌리는 대체 경로. 봉인 판정을 오염시키지 않도록 **양 축 모두에서 제거**한다
# (ⓒ 에 남아 있으면 .pyc 가 다른 데 생겨 음성 대조가 공허해지고, ⓓ 에 남아 있으면 봉인이 아니라
#  이 변수 덕에 0 이 나와 거짓 양성이 된다 — src/lib.rs 가 이 변수를 기각한 근거와 같은 결).
PYCACHE_PREFIX_ENV = "PYTHONPYCACHEPREFIX"

SPECIMEN_MODULES = ("javis_runtime_seal", "javis_idempotency", "javis_lock")

fails = []
unmeasured = []


def check(name, cond, detail="", why=""):
    """detail = 항상 찍는 **측정값**(PASS 도 증적이다) · why = FAIL 일 때만 붙는 파급 설명.

    실패 문면을 PASS 줄에 섞지 않는다 — 초록 로그에 '부재'·'파손' 같은 낱말이 박히면
    라운드 감사에서 사람이 반대로 읽는다(증적 오독 방지)."""
    tail = (" — " + detail) if detail else ""
    if not cond and why:
        tail += (" · " if detail else " — ") + why
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, tail))
    if not cond:
        fails.append(name)


def skip(name, reason):
    print("SKIP %s — %s" % (name, reason))


def mark_unmeasured(name, detail):
    print("UNMEASURED %s — %s" % (name, detail))
    unmeasured.append(name)


def finish():
    print()
    if unmeasured:
        print("UNMEASURED %d건: %s" % (len(unmeasured), unmeasured))
        print("PYSEAL-SPECIMEN-UNMEASURED")
        sys.exit(2)
    if fails:
        print("FAIL %d건: %s" % (len(fails), fails))
        sys.exit(1)
    print("PYSEAL-SPECIMEN-OK")
    sys.exit(0)


def cache_census(tree):
    """트리 안의 `__pycache__` 디렉터리와 `.pyc`/`.pyo` 파일 경로 집합(트리 상대 · 정렬 가능)."""
    hits = set()
    for dp, dns, fns in os.walk(tree):
        for d in dns:
            if d == "__pycache__":
                hits.add(os.path.relpath(os.path.join(dp, d), tree).replace(os.sep, "/"))
        for fn in fns:
            if fn.endswith(".pyc") or fn.endswith(".pyo"):
                hits.add(os.path.relpath(os.path.join(dp, fn), tree).replace(os.sep, "/"))
    return hits


def base_env():
    """부모 env 에서 봉인·캐시 관련 키를 걷어낸 바닥(양 축의 공통 출발점).

    부모에 이미 봉인이나 PYCACHEPREFIX 가 걸려 있으면 두 축이 모두 거짓말을 한다
    (훅·데몬 아래에서 이 테스트를 돌리면 실제로 걸려 있다 — hooks/_lib.sh 프리루드)."""
    e = dict(os.environ)
    for k in (SEAL_ENV, PYCACHE_PREFIX_ENV, "PYTHONSTARTUP"):
        e.pop(k, None)
    return e


def run_py(python, code, env, timeout=120):
    """(rc, stdout+stderr) — 인터프리터 한 번 호출."""
    try:
        p = subprocess.run([python, "-c", code], env=env, timeout=timeout,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError as e:
        return 127, "%s: %s" % (e.__class__.__name__, e)
    except subprocess.TimeoutExpired:
        return 124, "timeout %ds" % timeout
    return p.returncode, p.stdout.decode("utf-8", "replace")


IMPORT_CODE = (
    "import sys\n"
    "sys.path.insert(0, %r)\n"
    "import %s\n"
    "print('SPECIMEN-IMPORT-OK', sys.executable, 'pycache_prefix=', sys.pycache_prefix)\n"
)


def run_import(python, tree, env):
    return run_py(python, IMPORT_CODE % (tree, ", ".join(SPECIMEN_MODULES)), env)


def probe_pycache_prefix(python):
    """(ok, prefix) — prefix None = in-tree 로 쓴다(트리 축 유효) · 문자열 = 캐시를 그리로 돌린다.

    macOS `/usr/bin/python3`(Xcode CLT)는 Apple 패치로 이 값이 고정돼 있다(2026-09-08 실측) —
    그런 인터프리터에서 'in-tree .pyc 0개'는 봉인의 증거가 아니라 공허다."""
    rc, out = run_py(python, "import sys;print('PFX', sys.pycache_prefix)", base_env())
    if rc != 0:
        return False, None
    for line in out.splitlines():
        if line.startswith("PFX "):
            v = line[4:].strip()
            return True, (None if v == "None" else v)
    return False, None


def copy_specimen(dest):
    """검체 모듈을 사본 트리로 복사(원본 무접촉). 부재 모듈 목록을 반환."""
    os.makedirs(dest, exist_ok=True)
    missing = []
    for m in SPECIMEN_MODULES:
        src = os.path.join(PACK_BIN, m + ".py")
        if not os.path.isfile(src):
            missing.append(m + ".py")
            continue
        shutil.copyfile(src, os.path.join(dest, m + ".py"))
    return missing


# ═══════════════════════════════════════════════════════════════════════════
# ⓐ 인터프리터 해소 — 번들 우선 · 폴백은 사유 명기(무음 대체 금지)
# ═══════════════════════════════════════════════════════════════════════════
def resolve_interpreter():
    """(python, kind, why) — kind ∈ {"bundled","pack-runtime"}."""
    forced = os.environ.get("CYS_PYSEAL_SPECIMEN_PYTHON") or ""
    if forced:
        norm = forced.replace(os.sep, "/")
        kind = "bundled" if "/Contents/Resources/runtime/python/" in norm else "pack-runtime"
        return forced, kind, "CYS_PYSEAL_SPECIMEN_PYTHON 주입"

    # 번들 후보: src/app_bundle.rs PYTHON_RUNTIME 과 Windows 배치(runtime\python\python3.exe).
    apps = []
    forced_app = os.environ.get("CYS_PYSEAL_SPECIMEN_APP") or ""
    if forced_app:
        apps.append(forced_app)
    apps += ["/Applications/cys.app", os.path.expanduser("~/Applications/cys.app")]
    for app in apps:
        for rel in ("Contents/Resources/runtime/python/bin/python3",
                    "runtime/python/python3.exe"):
            cand = os.path.join(app, rel.replace("/", os.sep))
            if os.path.isfile(cand) and os.access(cand, os.X_OK):
                return cand, "bundled", app

    # 폴백: 팩이 실제로 쓰는 런타임 python(hooks/_lib.sh 의 CYS_PY 해소와 같은 순서).
    cys_py = os.environ.get("CYS_PY") or ""
    if cys_py and (os.path.isfile(cys_py) or shutil.which(cys_py)):
        return cys_py, "pack-runtime", "CYS_PY"
    for name in ("python3", "python"):
        w = shutil.which(name)
        if w:
            return w, "pack-runtime", "PATH:" + name
    return sys.executable, "pack-runtime", "sys.executable"


def bundle_runtime_dir(python):
    """번들 인터프리터 경로 → `<app>/Contents/Resources/runtime` (없으면 "")."""
    norm = python.replace(os.sep, "/")
    marker = "/Contents/Resources/runtime/"
    if marker in norm:
        d = norm.split(marker)[0] + marker.rstrip("/")
        return d if os.path.isdir(d) else ""
    # Windows 배치(<install>/runtime/python/python3.exe)
    d = os.path.dirname(os.path.dirname(os.path.dirname(python)))
    return d if os.path.isdir(d) else ""


def control_candidates(primary):
    """음성 대조용 인터프리터 후보 — **번들은 절대 넣지 않는다**(봉인 없는 번들 실행 금지)."""
    out = []
    for c in (primary, sys.executable, shutil.which("python3"), shutil.which("python"),
              "/usr/bin/python3"):
        if not c:
            continue
        if "/Contents/Resources/runtime/python/" in c.replace(os.sep, "/"):
            continue  # 번들 인터프리터 배제
        if c not in out and (os.path.isfile(c) or shutil.which(c)):
            out.append(c)
    return out


PY, PY_KIND, PY_WHY = resolve_interpreter()
print("검체 인터프리터: %s (%s · %s)" % (PY, PY_KIND, PY_WHY))
print("팩 트리: %s" % PACK)
print("레포 루트: %s" % ROOT)

if not (os.path.isfile(PY) or shutil.which(PY)):
    mark_unmeasured("ⓐ 인터프리터 해소", "실행 가능한 python 을 못 찾았다: %r" % PY)
    finish()
check("ⓐ 인터프리터 해소(%s)" % PY_KIND, True,
      "%s%s" % (PY, "" if PY_KIND == "bundled" else
                " — 번들 부재 시 팩 런타임 폴백은 계약이다(무엇을 썼는지 명기)"))

# ── ⓑ 캐시 방향 실측 — 트리 축이 유효한가(공허한 PASS 차단) ─────────────────
ok_probe, PY_PFX = probe_pycache_prefix(PY)
if not ok_probe:
    mark_unmeasured("ⓑ 캐시 방향 실측", "인터프리터가 안 돈다: %s" % PY)
    finish()
TREE_AXES_VALID = PY_PFX is None
check("ⓑ 캐시 방향 실측", True,
      "sys.pycache_prefix=%s → 트리 축 %s"
      % (PY_PFX, "유효(in-tree 관측 가능)" if TREE_AXES_VALID else "공허(캐시가 트리 밖으로 간다)"))

WORK = tempfile.mkdtemp(prefix="cys-pyseal-specimen-")
try:
    seal_tree = os.path.join(WORK, "sealed")
    missing = copy_specimen(seal_tree)
    if missing:
        mark_unmeasured("ⓐ 검체 모듈 준비", "팩 bin 에 검체 모듈 부재: %s" % missing)
        finish()

    sealed_env = base_env()
    sealed_env[SEAL_ENV] = SEAL_ON
    sealed_env[UTF8_ENV] = UTF8_ON

    # ═══════════════════════════════════════════════════════════════════════
    # ⓒ ★음성 대조(계측 타당성) — 트리 축이 도는 실행에서만 의무 · 번들 인터프리터 금지
    # ═══════════════════════════════════════════════════════════════════════
    if TREE_AXES_VALID:
        ctl_py = ""
        ctl_rejected = []
        for cand in control_candidates(PY):
            ok, pfx = probe_pycache_prefix(cand)
            if not ok:
                ctl_rejected.append("%s(안 돎)" % cand)
                continue
            if pfx is not None:
                ctl_rejected.append("%s(pycache_prefix=%s)" % (cand, pfx))
                continue
            ctl_py = cand
            break
        if not ctl_py:
            mark_unmeasured(
                "ⓒ 음성 대조 인터프리터 해소",
                "in-tree 로 쓰는 비번들 python 이 없다(기각 %s) — 트리 축이 돌았는데 그것을 "
                "검증할 대조가 없으면 결과는 통과가 아니다" % ctl_rejected)
            finish()
        ctl_tree = os.path.join(WORK, "control")
        copy_specimen(ctl_tree)
        ctl_rc, ctl_out = run_import(ctl_py, ctl_tree, base_env())  # 봉인 없음
        ctl_hits = cache_census(ctl_tree)
        check("ⓒ 음성 대조 import 성공(%s)" % ctl_py, ctl_rc == 0,
              "rc=%d · %s" % (ctl_rc, ctl_out.strip().replace("\n", " ")[-240:]),
              "import 이 안 돌았으면 이어지는 0 생성 관측은 전부 공허하다")
        check("ⓒ ★계측 타당성: 봉인 없는 env 는 .pyc 를 만든다", len(ctl_hits) >= 1,
              "신규 %d개 %s%s" % (len(ctl_hits), sorted(ctl_hits)[:4],
                                 (" · 기각 %s" % ctl_rejected) if ctl_rejected else ""),
              "0이면 이 검체는 아무것도 재지 못한다 — 봉인이 깨져 있어도 ⓓⓔⓕ 가 초록이 된다")
    else:
        skip("ⓒ 음성 대조",
             "선택한 인터프리터가 캐시를 %s 로 돌려 트리 축 자체가 공허하다 — 검증할 대상이 "
             "없으므로 대조도 세우지 않는다(ⓓ0 직접 축은 그대로 돈다)" % PY_PFX)

    # ═══════════════════════════════════════════════════════════════════════
    # ⓓ0 봉인 env 를 인터프리터가 실제로 따른다 — 트리와 무관한 직접 축(공허 불가)
    # ═══════════════════════════════════════════════════════════════════════
    d0_rc, d0_out = run_py(PY, "import sys;print('DWB', sys.dont_write_bytecode)", sealed_env)
    d0_ok = d0_rc == 0 and "DWB True" in d0_out
    check("ⓓ0 봉인 env 를 인터프리터가 따른다(sys.dont_write_bytecode)", d0_ok,
          "rc=%d · %s" % (d0_rc, d0_out.strip().replace("\n", " ")[-160:]),
          "데몬이 %s=%s 를 실어도 이 인터프리터는 무시한다 — SEAL-1 의 전제가 깨진다"
          % (SEAL_ENV, SEAL_ON))

    # ═══════════════════════════════════════════════════════════════════════
    # ⓓ 봉인 양성(사본 트리) — 데몬이 주입하는 env 그대로
    # ═══════════════════════════════════════════════════════════════════════
    seal_rc, seal_out = run_import(PY, seal_tree, sealed_env)
    seal_hits = cache_census(seal_tree)
    check("ⓓ 봉인 import 성공(%s)" % PY_KIND, seal_rc == 0,
          "rc=%d · %s" % (seal_rc, seal_out.strip().replace("\n", " ")[-240:]),
          "import 이 안 돌았으면 아래 0 생성은 봉인의 증거가 아니다")
    if TREE_AXES_VALID:
        check("ⓓ 봉인 env 에서 사본 트리 캐시 신규 0", not seal_hits,
              "신규 %d개 %s" % (len(seal_hits), sorted(seal_hits)),
              "SEAL-1 파손 — 번들 인터프리터였다면 이것이 곧 코드서명 붕괴다")
    else:
        skip("ⓓ 사본 트리 census", "캐시가 %s 로 가므로 0개는 봉인의 증거가 아니다" % PY_PFX)

    # ═══════════════════════════════════════════════════════════════════════
    # ⓔ 봉인 양성(실물 팩 트리) — 실제 배치와 같은 꼴(실물 bin 에서 import)
    #    실행 전후 census diff · 신규가 나오면 정확히 그것만 지우고 FAIL(자기 오염 금지)
    # ═══════════════════════════════════════════════════════════════════════
    before = cache_census(PACK)
    live_rc, live_out = run_import(PY, PACK_BIN, sealed_env)
    after = cache_census(PACK)
    new_in_pack = sorted(after - before)
    check("ⓔ 실물 팩 bin import 성공", live_rc == 0,
          "rc=%d · %s" % (live_rc, live_out.strip().replace("\n", " ")[-240:]),
          "import 이 안 돌았으면 팩 트리 diff 0 은 봉인의 증거가 아니다")
    if TREE_AXES_VALID:
        check("ⓔ 봉인 env 에서 실물 팩 트리 캐시 신규 0", not new_in_pack,
              "실행 전 %d → 후 %d · 신규 %d개 %s"
              % (len(before), len(after), len(new_in_pack), new_in_pack),
              "배포 팩 안에 캐시가 쌓인다(설치본이면 서명 대상 트리 오염)")
    else:
        skip("ⓔ 실물 팩 트리 census", "캐시가 %s 로 가므로 0개는 봉인의 증거가 아니다" % PY_PFX)
    # 자기 오염 청소 — 이 실행이 만든 것만, **깊은 것부터**(부모 __pycache__ 를 먼저 지우면
    # 자식 .pyc 경로가 이미 사라져 FileNotFoundError 소음이 난다) 지운다.
    for rel in sorted(new_in_pack, reverse=True):
        victim = os.path.join(PACK, rel.replace("/", os.sep))
        if not os.path.exists(victim):
            continue  # 부모째 지워져 이미 사라짐(정상)
        try:
            shutil.rmtree(victim) if os.path.isdir(victim) else os.remove(victim)
            print("     청소: %s" % rel)
        except OSError as e:
            print("     청소 실패(%s): %s" % (e.__class__.__name__, rel))

    # ═══════════════════════════════════════════════════════════════════════
    # ⓕ 봉인 양성(번들 런타임 트리) — 2026-08-01 사고의 원형(번들 stdlib 캐시) 직접 관측
    # ═══════════════════════════════════════════════════════════════════════
    bundle_rt = bundle_runtime_dir(PY) if PY_KIND == "bundled" else ""
    if bundle_rt and TREE_AXES_VALID:
        rb = cache_census(bundle_rt)
        rc2, out2 = run_import(PY, seal_tree, sealed_env)
        ra = cache_census(bundle_rt)
        new_in_bundle = sorted(ra - rb)
        check("ⓕ 번들 런타임 import 성공", rc2 == 0,
              "rc=%d · %s" % (rc2, out2.strip().replace("\n", " ")[-240:]),
              "import 이 안 돌았으면 번들 트리 diff 0 은 봉인의 증거가 아니다")
        check("ⓕ 봉인 env 에서 번들 런타임 트리 캐시 신규 0", not new_in_bundle,
              "실행 전 %d → 후 %d · 신규 %d개 @ %s"
              % (len(rb), len(ra), len(new_in_bundle), bundle_rt),
              "신규 %s — 이것이 2026-08-01 사고(codesign 봉인 파손 → Gatekeeper 차단) 그 자체다"
              % new_in_bundle)
    else:
        skip("ⓕ 번들 런타임 트리",
             "인터프리터가 번들이 아니거나(%s) 트리 축이 공허하다 — 로컬에 설치본이 없으면 이 축은 "
             "잴 수 없다. CI/릴리스 경로의 등가 축은 release-gate-gatekeeper.sh ⑤(SEAL-2 전칭)·"
             "verify-gatekeeper-user-path.sh ⑥-B 다" % PY_KIND)
finally:
    shutil.rmtree(WORK, ignore_errors=True)

# ═══════════════════════════════════════════════════════════════════════════
# ⓖ 정본 대조 — 이 파일의 고정값이 src/lib.rs 상수에서 표류하지 않았는가
# ═══════════════════════════════════════════════════════════════════════════
LIB_RS = os.path.join(ROOT, "src", "lib.rs")
if os.path.isfile(LIB_RS):
    with open(LIB_RS, "rb") as fh:
        lib_src = fh.read().decode("utf-8", "replace")
    want_name = 'pub const ENV_PY_NO_BYTECODE: &str = "%s";' % SEAL_ENV
    want_on = 'pub const PY_NO_BYTECODE_ON: &str = "%s";' % SEAL_ON
    check("ⓖ 봉인 env 이름이 정본과 동일", want_name in lib_src, "src/lib.rs 대조 %r" % SEAL_ENV,
          "정본 줄 %r 을 못 찾았다 — 이 검체가 엉뚱한 변수를 재고 있다" % want_name)
    check("ⓖ 봉인 켜짐 값이 정본과 동일", want_on in lib_src, "src/lib.rs 대조 %r" % SEAL_ON,
          "정본 줄 %r 을 못 찾았다 — 빈 값이면 CPython 이 '끔'으로 읽어 3층이 동시에 죽는다" % want_on)
else:
    skip("ⓖ 정본 대조", "레포 루트에 src/lib.rs 없음(팩 단독 설치) — 상수는 이 파일 상단 고정값을 쓴다")

finish()
