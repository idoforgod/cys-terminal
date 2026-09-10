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
     쓰는 것이 2026-08-01 사고의 원형이다. ★D5(반성 라운드 2026-09-10): 기준선은 **첫 스폰 전**
     (ⓑ probe 전)에 잡고 판정은 **종료 시**(`finish()` — 조기 종료 경로 포함)에 한 번 — 첫 조회부터
     종료까지 신규·**변경**(크기·mtime) 0. 종전엔 ⓑ probe 가 번들 인터프리터를 **봉인 없이** 돌렸고
     기준선은 ⓕ 자기 스폰 직전이라 그 첫-쓰기에 정확히 눈이 멀었다(유효 .pyc 가 없으면 서명된 번들
     안에 `__pycache__` 가 생긴다 = 사고 그 자체). 신규분은 지우고(ⓔ 와 같은 자기 오염 청소 규율)
     변경분은 손대지 않는다(원본을 더 건드리지 않는다). 번들이 아니면 SKIP 사유를 출력한다.
  ⓗ ★합성 번들(D5·D11) — 실제 번들이 없는 레인(3 CI 레인 전부 · CONTRACTS §B-11 의 ⓕ 축이 거기서
     항상 SKIP 이던 격차)에서도 ⓕ 경로를 **실행**한다. 임시 `<tmp>/cys.app/Contents/Resources/
     runtime/python/bin/python3` 는 in-tree 로 쓰는 비번들 python 으로 exec 하는 sh 심(shim)이고
     `…/python/lib/sitecustomize.py` 가 '시작 모듈 캐시가 없는 번들' 을 재현한다(site 가 시작마다
     import 하므로 무봉인 스폰 한 번이면 `lib/__pycache__` 가 생긴다). ⓗ1 계측 타당성(무봉인 심 →
     .pyc ≥1 · 로드된 sitecustomize 가 우리 것) · ⓗ2 이 검체가 번들 인터프리터에 쓰는 스폰 함수
     전부(probe·DWB·import)를 봉인 env 로 돌려 첫 조회부터 변경 0 · ⓗ3 기준선 뒤 의도적 오염을
     가드가 보고하고 신규분만 지우는가(실패 주입 · 가드 감도). **한계(정직)**: 실제 번들 python
     빌드가 봉인을 따르는지는 증명하지 못한다 — 그것은 ⓕ(오너 기계)와 release-gate-gatekeeper.sh
     ⑤(SEAL-2 전칭)의 몫이다. Windows 는 심이 sh 라 SKIP 사유를 출력한다.
  ⓖ 정본 대조 — 레포 루트가 보이면 `src/lib.rs` 의 상수 두 줄(이름·켜짐 값)이 이 파일의 고정값과
     같은지 본다. 팩 단독 설치(루트 없음)에서는 SKIP 사유를 출력한다(무음 생략 금지).

검체 모듈은 **실물 팩 모듈**이다(합성 모듈 아님 — import 부작용 0 을 실측한 것만 고른다):
  `javis_runtime_seal.py` · `javis_idempotency.py` · `javis_lock.py`.

종료 코드(run_bootstrap_health·test_pyseal_census 규약과 동형): 0 = 전부 측정·통과(말미
`PYSEAL-SPECIMEN-OK`) · 1 = FAIL 1건 이상 · 2 = UNMEASURED(인터프리터·검체 모듈 해소 실패,
또는 트리 축이 돌았는데 음성 대조를 세울 인터프리터가 없음 = **측정 불능은 통과가 아니다**).
쓰기는 임시 디렉터리 안에서만 한다(실물 팩·번들은 읽기 전용 · ⓔ·ⓕ 의 **신규분만** 자기 오염 청소 제외).
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
# ★D5: 번들 런타임 트리 가드 — 첫 스폰 전에 기준선을 잡고(main flow) finish() 가 종료 판정을 낸다.
BUNDLE_RT = ""
BUNDLE_GUARD = None
_BUNDLE_VERDICT_DONE = False


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
    _bundle_verdict()          # ★D5: 조기 종료 경로에서도 번들 종료 판정·청소는 반드시 돈다
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


def cache_signature(tree):
    """트리 안 캐시 항목의 서명 {트리 상대 경로: 서명}. 파일 = (크기, mtime_ns) — 경로 집합 차이만 보면
    **기존 .pyc 덮어쓰기**를 못 본다(codex 지적) · 디렉터리 = 상수(내용 변화는 파일 쪽이 잡는다)."""
    sig = {}
    for dp, dns, fns in os.walk(tree):
        for d in dns:
            if d == "__pycache__":
                sig[os.path.relpath(os.path.join(dp, d), tree).replace(os.sep, "/")] = ("dir",)
        for fn in fns:
            if fn.endswith(".pyc") or fn.endswith(".pyo"):
                full = os.path.join(dp, fn)
                try:
                    st = os.stat(full)
                    sig[os.path.relpath(full, tree).replace(os.sep, "/")] = (st.st_size, st.st_mtime_ns)
                except OSError:
                    sig[os.path.relpath(full, tree).replace(os.sep, "/")] = ("unstat",)
    return sig


class TreeGuard:
    """★D5: 기준선(생성 시점)과 대조해 신규·변경분을 보고하고 **신규분만** 지우는 트리 가드.

    기준선은 **첫 스폰 전**에 잡아야 한다 — 스폰 뒤에 잡으면 그 스폰이 남긴 첫-쓰기가 기준선에
    들어가 영원히 보이지 않는다(종전 ⓕ 가 정확히 그렇게 눈이 멀었다)."""

    def __init__(self, tree):
        self.tree = tree
        self.before = cache_signature(tree)

    def changed(self):
        after = cache_signature(self.tree)
        new = sorted(p for p in after if p not in self.before)
        modified = sorted(p for p in after if p in self.before and after[p] != self.before[p])
        return new, modified

    def sweep_new(self, new):
        """신규분만, 깊은 것부터 지운다(부모 __pycache__ 를 먼저 지우면 자식 경로가 사라져 소음)."""
        swept = []
        for rel in sorted(new, reverse=True):
            victim = os.path.join(self.tree, rel.replace("/", os.sep))
            if not os.path.exists(victim):
                continue
            try:
                shutil.rmtree(victim) if os.path.isdir(victim) else os.remove(victim)
                swept.append(rel)
            except OSError as e:
                print("     청소 실패(%s): %s" % (e.__class__.__name__, rel))
        return swept


def _bundle_verdict():
    """ⓕ 종료 판정 — 첫 조회(ⓑ probe)부터 지금까지 번들 런타임 트리의 신규·변경 0. 한 번만 낸다."""
    global _BUNDLE_VERDICT_DONE
    if BUNDLE_GUARD is None or _BUNDLE_VERDICT_DONE:
        return
    _BUNDLE_VERDICT_DONE = True
    new, modified = BUNDLE_GUARD.changed()
    check("ⓕ 봉인 env 에서 번들 런타임 트리 원본 변경 0(첫 조회부터 종료까지 · 신규+변경)",
          not new and not modified,
          "기준선 %d항 · 신규 %d %s · 변경 %d %s @ %s"
          % (len(BUNDLE_GUARD.before), len(new), new[:6], len(modified), modified[:6], BUNDLE_RT),
          "이것이 2026-08-01 사고(codesign 봉인 파손 → Gatekeeper 차단) 그 자체다 — 신규분은 지금 "
          "지운다(변경분은 손대지 않는다 · 원본을 더 건드리지 않는다)")
    for rel in BUNDLE_GUARD.sweep_new(new):
        print("     청소(번들 신규분): %s" % rel)


def base_env():
    """부모 env 에서 봉인·캐시 관련 키를 걷어낸 바닥(양 축의 공통 출발점).

    부모에 이미 봉인이나 PYCACHEPREFIX 가 걸려 있으면 두 축이 모두 거짓말을 한다
    (훅·데몬 아래에서 이 테스트를 돌리면 실제로 걸려 있다 — hooks/_lib.sh 프리루드)."""
    e = dict(os.environ)
    for k in (SEAL_ENV, PYCACHE_PREFIX_ENV, "PYTHONSTARTUP"):
        e.pop(k, None)
    return e


def sealed_env_of():
    """데몬이 주입하는 env 그대로(`spawn_env_pairs` 무조건 쌍 = SEAL-1 + PYTHONUTF8) — ⓓ0·ⓓ·ⓔ·ⓕ·ⓗ 공용."""
    e = base_env()
    e[SEAL_ENV] = SEAL_ON
    e[UTF8_ENV] = UTF8_ON
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
    그런 인터프리터에서 'in-tree .pyc 0개'는 봉인의 증거가 아니라 공허다.

    ★D5(반성 라운드 2026-09-10): 이 조회는 **봉인 env 로** 스폰한다. 종전엔 `base_env()`(봉인 없음)로
    번들 인터프리터를 돌려 — 이 파일의 ⓒ 문서가 금지한 바로 그 실행 — 유효 .pyc 가 없으면 서명된
    번들 안에 시작 모듈 캐시를 남겼다. `sys.pycache_prefix` 값은 봉인과 무관하므로 측정은 같다."""
    rc, out = run_py(python, "import sys;print('PFX', sys.pycache_prefix)", sealed_env_of())
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

# ★D5: 번들 기준선은 **첫 스폰 전**이다 — 바로 아래 ⓑ probe 가 이 검체의 첫 스폰이다.
#   판정은 finish() 의 ⓕ 종료 census(첫 조회부터 종료까지 · 조기 종료 포함)가 낸다.
BUNDLE_RT = bundle_runtime_dir(PY) if PY_KIND == "bundled" else ""
BUNDLE_GUARD = TreeGuard(BUNDLE_RT) if BUNDLE_RT else None
if BUNDLE_GUARD is not None:
    print("번들 런타임 기준선(첫 스폰 전): %s · 캐시 항목 %d" % (BUNDLE_RT, len(BUNDLE_GUARD.before)))

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

    sealed_env = sealed_env_of()

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
    #    ★D5: 기준선은 위(첫 스폰 전) · 판정은 finish() 의 종료 census. 여기서는 사고의 원형인
    #    stdlib-touching import 를 봉인 env 로 한 번 더 돌린다(트리 축의 공허 여부와 무관하게 —
    #    서명 트리의 어떤 변경도 사고다).
    # ═══════════════════════════════════════════════════════════════════════
    if BUNDLE_RT:
        rc2, out2 = run_import(PY, seal_tree, sealed_env)
        check("ⓕ 번들 런타임 import 성공", rc2 == 0,
              "rc=%d · %s" % (rc2, out2.strip().replace("\n", " ")[-240:]),
              "import 이 안 돌았으면 번들 트리 변경 0 은 봉인의 증거가 아니다")
        print("     ⓕ 트리 판정은 종료 census 로 낸다(첫 조회부터 종료까지 · finish())")
    else:
        skip("ⓕ 번들 런타임 트리",
             "인터프리터가 번들이 아니다(%s) — 로컬에 설치본이 없으면 이 축은 잴 수 없다. 같은 경로의 "
             "합성 실행은 아래 ⓗ, CI/릴리스 경로의 등가 축은 release-gate-gatekeeper.sh ⑤(SEAL-2 전칭)·"
             "verify-gatekeeper-user-path.sh ⑥-B 다" % PY_KIND)

    # ═══════════════════════════════════════════════════════════════════════
    # ⓗ ★합성 번들(D5·D11) — ⓕ 경로를 실제 번들 없이 실행 · 이 검체 자신의 스폰 규율 검체
    #    재귀 실행이 아니다(codex): 이 검체가 번들 인터프리터에 쓰는 **스폰 함수 그 자체**
    #    (probe_pycache_prefix · run_py · run_import)와 같은 env 구성(sealed_env_of)을 합성 번들에
    #    건다. 어느 함수가 봉인을 빼면 ⓗ2 가 붉어진다.
    # ═══════════════════════════════════════════════════════════════════════
    if os.name == "nt":
        skip("ⓗ 합성 번들", "심(shim)이 POSIX sh 라 Windows 에서는 세우지 않는다(정직한 결측 — 이 레인은 "
                        "windows-build 부분 레인 계약대로 팩 스위트를 돌리지 않는다)")
    else:
        h_ctl, h_rejected = "", []
        for cand in control_candidates(PY):
            ok, pfx = probe_pycache_prefix(cand)
            if ok and pfx is None:
                h_ctl = cand
                break
            h_rejected.append("%s(%s)" % (cand, "안 돎" if not ok else "pycache_prefix=%s" % pfx))
        if not h_ctl:
            skip("ⓗ 합성 번들", "in-tree 로 쓰는 비번들 python 이 없다(기각 %s) — 합성 번들의 시작 모듈 "
                            "캐시를 관측할 수 없다(Apple python 만 있는 기계 · CI 는 setup-python 이라 돈다)"
                 % h_rejected)
        else:
            def synth_bundle(name):
                """<WORK>/<name>/cys.app — sh 심 + 시작 모듈(sitecustomize) 하나. (심, runtime, lib)"""
                app = os.path.join(WORK, name, "cys.app")
                rt = os.path.join(app, "Contents", "Resources", "runtime")
                lib = os.path.join(rt, "python", "lib")
                bindir = os.path.join(rt, "python", "bin")
                os.makedirs(lib)
                os.makedirs(bindir)
                with open(os.path.join(lib, "sitecustomize.py"), "w", encoding="utf-8") as fh:
                    fh.write("# 합성 번들의 '시작 모듈' — site 가 시작마다 import 한다(캐시 없음 = 첫-쓰기 사례).\n"
                             "SYNTH_BUNDLE_MARK = %r\n" % name)
                shim = os.path.join(bindir, "python3")
                q = lambda s: "'" + s.replace("'", "'\\''") + "'"
                with open(shim, "w", encoding="utf-8") as fh:
                    fh.write("#!/bin/sh\n"
                             "LIB=%s\n"
                             "if [ -n \"${PYTHONPATH:-}\" ]; then PYTHONPATH=\"$LIB:$PYTHONPATH\"; "
                             "else PYTHONPATH=\"$LIB\"; fi\n"
                             "export PYTHONPATH\n"
                             "exec %s \"$@\"\n" % (q(lib), q(h_ctl)))
                os.chmod(shim, 0o755)
                return shim, rt, lib

            SC_CODE = "import sys, sitecustomize; print('SC', sitecustomize.__file__)"

            def loaded_sitecustomize(out):
                for line in out.splitlines():
                    if line.startswith("SC "):
                        return line[3:].strip()
                return ""

            # ⓗ1 계측 타당성 — 무봉인 심 실행이 합성 번들 안에 .pyc 를 만들고, 로드된 sitecustomize 가 우리 것
            #   (이것이 없으면 아래 '변경 0' 은 sitecustomize 미로드·캐시 외부 리디렉션에서도 참이라 공허)
            shim_a, rt_a, lib_a = synth_bundle("synth-a")
            a_rc, a_out = run_py(shim_a, SC_CODE, base_env())           # 봉인 없음 — 합성 번들이라 허용
            a_hits = cache_census(rt_a)
            a_file = loaded_sitecustomize(a_out)
            a_ok = a_rc == 0 and a_file.startswith(lib_a) and len(a_hits) >= 1
            if not a_ok:
                mark_unmeasured("ⓗ1 합성 번들 계측 타당성",
                                "rc=%d · 로드=%r(기대: %s 아래) · 무봉인 .pyc %d개 %s · %s"
                                % (a_rc, a_file, lib_a, len(a_hits), sorted(a_hits)[:3],
                                   a_out.strip().replace("\n", " ")[-160:]))
            else:
                check("ⓗ1 합성 번들 계측 타당성(무봉인 심 → .pyc ≥1 · 로드된 sitecustomize 가 우리 것)", True,
                      "%d개 %s · 로드 %s · 심→%s" % (len(a_hits), sorted(a_hits), a_file, h_ctl))

                # ⓗ2 봉인 규율 — 새 합성 번들에 첫 스폰 전 기준선(ⓕ 와 같은 순서) → 이 검체의 스폰 함수 전부
                shim_b, rt_b, lib_b = synth_bundle("synth-b")
                guard_b = TreeGuard(rt_b)
                ok_b, pfx_b = probe_pycache_prefix(shim_b)               # ① ⓑ 와 같은 함수(봉인 실림)
                b_rc1, b_out1 = run_py(shim_b, "import sys;print('DWB', sys.dont_write_bytecode)",
                                       sealed_env)                        # ② ⓓ0 과 같은 호출
                b_rc2, b_out2 = run_import(shim_b, seal_tree, sealed_env)  # ③ ⓓ/ⓕ 와 같은 호출
                b_rc3, b_out3 = run_py(shim_b, SC_CODE, sealed_env)       # 봉인 실행에서도 로드 확인
                b_new, b_mod = guard_b.changed()
                b_file = loaded_sitecustomize(b_out3)
                check("ⓗ2 봉인 실행이 합성 번들의 시작 모듈을 실제로 로드했다",
                      b_rc3 == 0 and b_file.startswith(lib_b), "로드 %r" % b_file,
                      "우리 sitecustomize 가 안 실렸으면 아래 '변경 0' 은 공허하다")
                check("ⓗ2 합성 번들: 첫 조회(probe)부터 import 까지 봉인 스폰 전부에서 변경 0",
                      ok_b and pfx_b is None and b_rc1 == 0 and "DWB True" in b_out1
                      and b_rc2 == 0 and not b_new and not b_mod,
                      "probe=%s/%s · DWB rc=%d(%s) · import rc=%d · 신규 %d %s · 변경 %d %s"
                      % (ok_b, pfx_b, b_rc1, "DWB True" in b_out1, b_rc2,
                         len(b_new), b_new, len(b_mod), b_mod),
                      "이 검체의 어떤 스폰이 봉인을 빼고 번들 인터프리터를 불렀다 — 실물 번들이면 "
                      "서명 트리 오염(D5 그 자체)")

                # ⓗ3 가드 감도(실패 주입) — 기준선 뒤 의도적 오염(무봉인 심 1회)을 가드가 보고하고, 신규분만 지운다
                c_rc, _c_out = run_py(shim_b, SC_CODE, base_env())
                c_new, c_mod = guard_b.changed()
                check("ⓗ3 가드 감도: 기준선 뒤 오염을 보고한다(실패 주입)", c_rc == 0 and len(c_new) >= 1,
                      "신규 %d %s · 변경 %d" % (len(c_new), c_new, len(c_mod)),
                      "가드가 첫-쓰기를 못 본다 — ⓕ 종료 판정이 공허하다")
                swept = guard_b.sweep_new(c_new)
                check("ⓗ3 신규분만 청소 뒤 잔존 0", not guard_b.changed()[0],
                      "청소 %d %s" % (len(swept), swept))
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

# ★D5 스폰 순서 핀(ⓖ') — 번들 기준선 대입이 이 파일 안에서 첫 스폰(ⓑ probe 호출)보다 **앞**에 있다.
#   ⓗ2 는 "스폰 함수가 봉인을 싣는가" 를 재고, 이 핀은 "기준선이 스폰 뒤로 밀리지 않았는가" 를 잰다
#   (둘은 다른 회귀다 — probe 만 봉인하고 기준선을 뒤로 되돌려도 ⓗ2 는 초록이다 · codex 지적).
with open(os.path.abspath(__file__), "rb") as fh:
    _me = fh.read().decode("utf-8", "replace")
_i_guard = _me.find("BUNDLE_GUARD = TreeGuard(BUNDLE_RT)")
_i_probe = _me.find("ok_probe, PY_PFX = probe_pycache_prefix(PY)")
check("ⓖ' 번들 기준선이 첫 스폰보다 앞(소스 순서 핀)", 0 <= _i_guard < _i_probe,
      "guard@%d · probe@%d" % (_i_guard, _i_probe),
      "기준선이 첫 스폰 뒤로 밀리면 그 스폰의 첫-쓰기에 눈이 먼다(D5)")

finish()
