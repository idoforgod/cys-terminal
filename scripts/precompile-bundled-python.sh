#!/usr/bin/env bash
# precompile-bundled-python.sh — 동봉 런타임의 모든 Python 소스를 **코드서명 전에** 바이트컴파일해
# `.pyc` 를 서명 봉인(CodeResources)에 포함시킨다. (SEAL-2 · 2026-08-01 실사고 구조적 봉인)
#
# ★왜 필요한가 — SEAL-1(env) 이 못 덮는 구멍
#   SEAL-1(`src/lib.rs` `ENV_PY_NO_BYTECODE`)은 **우리가 스폰하는** python 에만
#   `PYTHONDONTWRITEBYTECODE=1` 을 얹는다. 사용자·에이전트·외부 도구가 번들 python 을
#   **절대경로로 직접** 부르면 그 env 를 못 타고, CPython 이 `.../__pycache__/*.pyc` 를
#   번들 안에 새로 써서 봉인이 스스로 깨진다 → quarantine 이 붙은 사본은 첫 실행 시
#   Gatekeeper 전체 재검증에서 "손상되었기 때문에 열 수 없습니다"로 차단된다.
#   `scripts/verify-gatekeeper-user-path.sh` ⑥-A 가 정확히 이 불변식을 측정하며,
#   그 검사는 **완화가 아니라 이 패키징 단계로만** 통과한다.
#
#   원리: 모든 `.pyc` 를 서명 **전에** 만들어 봉인에 넣으면, 이후 누가 python 을 부르든
#   이미 유효한 캐시가 있어 **새로 쓸 것이 없다** → 봉인 불변.
#
# ★왜 `--invalidation-mode unchecked-hash` 인가 (기본 timestamp 를 쓰면 안 되는 이유)
#   · timestamp: `.pyc` 헤더에 **소스 mtime·크기**를 박고 import 때마다 대조한다. 번들은
#     tar→Tauri 복사→DMG(hdiutil)→ditto 설치→업데이터 tar 를 거치며 mtime 이 얼마든지
#     달라질 수 있고, 1초라도 어긋나면 CPython 이 **재컴파일해서 다시 쓴다** → 갭 부활.
#     설치 후 mtime 변동에 봉인 안전성을 걸 수는 없다.
#   · checked-hash: mtime 대신 소스 **해시**를 대조 — 이식성은 얻지만 import 마다 전체 소스를
#     읽어 해시해야 해 기동이 느려진다(우리는 훅 hot path 가 있다).
#   · unchecked-hash: 헤더에 해시를 넣되 **검사하지 않는다**(`check_source=0`). 소스가 뭐로
#     보이든 `.pyc` 를 그대로 쓴다 → **어떤 경우에도 재컴파일·재기록이 없다**. 번들은 서명으로
#     봉인된 읽기전용 산출물이라 "소스가 몰래 바뀌었을 때 자동 재컴파일" 이라는 timestamp/
#     checked 의 유일한 이점이 여기선 무의미하다(소스가 바뀌면 봉인이 먼저 깨져 앱이 차단된다).
#
# ★왜 `-o 0 -o 1 -o 2` 3종을 **전부** 만들어야 하는가 — opt 레벨은 별도 캐시 파일이다
#   CPython 의 캐시 경로에는 최적화 레벨이 **파일명으로** 박힌다:
#     opt-0 → `__pycache__/x.cpython-312.pyc` · opt-1 → `…opt-1.pyc` · opt-2 → `…opt-2.pyc`
#   즉 opt-0 만 만들어 두면 **`-O`/`-OO`/`PYTHONOPTIMIZE` 로 부르는 순간 캐시 미스**가 나고,
#   CPython 이 번들 안에 `…opt-1.pyc` 를 새로 써서 봉인이 그대로 깨진다. unchecked-hash 도
#   이걸 막지 못한다 — 다른 레벨의 `.pyc` 는 **존재 자체가 없으니** 재사용할 대상이 아니다.
#   실측(2026-08-01 · 이 스크립트의 opt-0 전용 산출물 + 무거운 import 부하):
#     `python3 -O`  → `.pyc` 1140 → 1544 (**+404 opt-1**) · codesign --verify rc=1 (봉인 파손)
#     `python3 -OO` → 1544 → 1948 (**+404 opt-2**) · `PYTHONOPTIMIZE=1/2` 동일 경로
#   SEAL-1(env)도 여기선 무력하다: `PYTHONDONTWRITEBYTECODE` 를 안 타는 절대경로 직접 호출이
#   바로 이 갭의 전제이기 때문이다. 우리가 `-O` 로 안 부른다는 것은 방어가 아니다 —
#   사용자·에이전트·외부 도구(`PYTHONOPTIMIZE` 를 export 한 셸 포함)가 부를 수 있다.
#   비용 실측: 3종 생성 시 `.pyc` 1140 → 3420개 · 트리 +57MB · 소요 1.43초(opt-0 전용 1.18초와 사실상 동일).
#   배포 크기: 같은 번들을 UDZO DMG 로 말면 **+20.5MB**(102,180,464 → 123,644,389 B) — 상세는
#   build-macos-signed.sh 의 DMG 생성부 주석. 봉인 파손(=앱이 아예 안 열림)과 바꿀 값이 아니다.
#
# ★왜 `.app` **밖**(= `src-tauri/runtime/`)에서 돌려야 하는가 — 실측 근거 (2026-08-01, macOS 25.5.0)
#   `.app` 번들 안의 바이너리를 한 번이라도 exec 하면 macOS 가 그 번들에 앱 번들 보호를 건다.
#   측정: 조립 중인 `.app` 안의 동봉 python 을 exec → **SIGKILL(rc=137)**, 그 뒤 같은 번들에
#   `codesign --force` → **"Operation not permitted"**. 즉 `.app` 생성 후 선컴파일은 불가능하다.
#   → 선컴파일은 **Tauri 가 resources 를 복사하기 전** 런타임 트리에서 끝나 있어야 한다.
#
# 사용: scripts/precompile-bundled-python.sh [runtime_dir]   (기본 src-tauri/runtime)
# 종료 코드: 0=선컴파일 + 검증 통과 / 1=컴파일 실패·커버리지 결손·무효 invalidation-mode
#            2=대상·인터프리터 부재(판정 불가 — 통과가 아니다)
set -euo pipefail

RT="${1:-src-tauri/runtime}"
[ -d "$RT" ] || { echo "✗ 런타임 디렉터리 없음: $RT" >&2; exit 2; }
PY="$RT/python/bin/python3"
[ -x "$PY" ] || { echo "✗ 동봉 python 없음/실행 불가: $PY" >&2; exit 2; }

# ★컴파일러는 반드시 **동봉 인터프리터 자신**이어야 한다 — 호스트 python3(버전 다름)로 만들면
#   magic number 가 어긋나 동봉 python 이 그 `.pyc` 를 무시하고 새로 쓴다(= 갭 그대로).
echo "== 동봉 Python 선컴파일 (unchecked-hash · 서명 전) =="
echo "   대상: $RT   인터프리터: $("$PY" -c 'import sys;print(sys.version.split()[0])')"
before_pyc=$({ find "$RT" -name '*.pyc' 2>/dev/null || true; } | wc -l | tr -d ' ')
before_kb=$(du -sk "$RT" | cut -f1)

# 런타임 **트리 전체**를 대상으로 한 번에 돈다 — python stdlib·site-packages(pip) 뿐 아니라
# node 가 품은 python(npm→node-gyp→gyp/pylib, 실측 59개)까지 같은 결함 표면이기 때문이다.
# (실측: gyp 모듈을 import 만 해도 `.pyc` 10개가 번들 안에 생긴다.)
# -q 는 compileall 자신의 진행 로그만 줄인다 — 컴파일 **오류는 그대로 출력**되고 rc≠0 이 된다.
# `-o 0 -o 1 -o 2` = 최적화 레벨 3종을 **한 번의 순회로 전부** 생성(머리 주석의 opt 레벨 항 참조).
# compileall 은 `-o 0` 을 태그 없는 `.pyc`(= CPython 이 기본 레벨에서 찾는 그 파일)로 매핑하고,
# 1·2 는 `.opt-1.pyc`·`.opt-2.pyc` 로 낸다 → 세 레벨 어느 쪽으로 불려도 캐시 미스가 없다.
if ! "$PY" -m compileall -q -f -j0 -o 0 -o 1 -o 2 --invalidation-mode unchecked-hash "$RT"; then
  echo "✗ compileall 실패 — 위 오류의 파일을 해결해야 한다(무시하면 그 모듈이 런타임에 .pyc 를 쓴다)" >&2
  exit 1
fi

# ── fail-closed 검증: 커버리지 100%(opt-0/1/2 **3종 전부**) + 전량 unchecked-hash ──
# 어느 레벨 하나라도 빠지거나 timestamp 모드면, 그 레벨로 부르는 순간 그 파일이 런타임에
# `.pyc` 를 새로 써서 봉인을 깬다. ★이 검사는 게이트가 요구하는 불변식 자체다 —
# 느슨하게 만들지 말 것(완화 = 사고 재발). 특히 "opt-0 만 보면 된다"로 되돌리지 말 것:
# 그 축소가 정확히 2026-08-01 에 적발된 잔여 갭이다.
"$PY" - "$RT" <<'PYCHECK'
import importlib.util, os, re, struct, sys

root = sys.argv[1]
# compileall `-o 0 -o 1 -o 2` 가 만드는 3종. ""(빈 문자열)=태그 없는 opt-0 캐시.
LEVELS = ("", "1", "2")
OPT_TAG = re.compile(r"\.opt-(\d+)$")

missing, badmode, n_py = [], [], 0
n_pyc = {"opt-0": 0, "opt-1": 0, "opt-2": 0}
n_pyc_other = 0

for dirpath, dirnames, filenames in os.walk(root):
    if os.path.basename(dirpath) == "__pycache__":
        for f in filenames:
            if not f.endswith(".pyc"):
                continue
            m = OPT_TAG.search(f[:-4])
            key = f"opt-{m.group(1)}" if m else "opt-0"
            if key in n_pyc:
                n_pyc[key] += 1
            else:
                n_pyc_other += 1
            with open(os.path.join(dirpath, f), "rb") as fh:
                head = fh.read(8)
            if len(head) < 8 or head[:4] != importlib.util.MAGIC_NUMBER:
                badmode.append((os.path.join(dirpath, f), "magic 불일치"))
                continue
            flags = struct.unpack("<I", head[4:8])[0]
            # bit0=hash-based, bit1=check_source. unchecked-hash == 1.
            if flags != 1:
                kind = "timestamp" if not (flags & 1) else "checked-hash"
                badmode.append((os.path.join(dirpath, f), f"{kind}(flags={flags})"))
        continue
    for f in filenames:
        if not f.endswith(".py"):
            continue
        n_py += 1
        src = os.path.join(dirpath, f)
        # ★레벨별로 **각각** 존재해야 한다 — 캐시 경로에 레벨이 파일명으로 박히므로
        #   opt-0 이 있어도 `-O` 호출은 opt-1 캐시를 못 찾아 새로 쓴다.
        lack = [lv for lv in LEVELS
                if not os.path.exists(importlib.util.cache_from_source(src, optimization=lv))]
        if lack:
            missing.append((src, lack))

print(f"   .py {n_py}개 · .pyc {sum(n_pyc.values()) + n_pyc_other}개"
      f" (opt-0 {n_pyc['opt-0']} · opt-1 {n_pyc['opt-1']} · opt-2 {n_pyc['opt-2']}"
      + (f" · 기타 {n_pyc_other}" if n_pyc_other else "") + ")")
rc = 0
if missing:
    n_lack = sum(len(lv) for _, lv in missing)
    print(f"✗ 선컴파일 누락: 소스 {len(missing)}개 · 캐시 {n_lack}건 —"
          f" 해당 레벨로 부르면 런타임에 .pyc 를 써서 봉인을 깬다:", file=sys.stderr)
    for p, lv in missing[:20]:
        levels = ", ".join("opt-" + (x or "0") for x in lv)
        print(f"   - {os.path.relpath(p, root)}: {levels} 없음", file=sys.stderr)
    if len(missing) > 20:
        print(f"   … 외 {len(missing) - 20}개", file=sys.stderr)
    rc = 1
if badmode:
    print(f"✗ invalidation-mode 위반 {len(badmode)}개 — unchecked-hash 가 아니면 재컴파일·재기록이 살아난다:", file=sys.stderr)
    for p, why in badmode[:20]:
        print(f"   - {os.path.relpath(p, root)}: {why}", file=sys.stderr)
    if len(badmode) > 20:
        print(f"   … 외 {len(badmode) - 20}개", file=sys.stderr)
    rc = 1
sys.exit(rc)
PYCHECK

after_pyc=$({ find "$RT" -name '*.pyc' 2>/dev/null || true; } | wc -l | tr -d ' ')
after_kb=$(du -sk "$RT" | cut -f1)
echo "   ✓ .pyc ${before_pyc} → ${after_pyc} · 트리 $((before_kb/1024))MB → $((after_kb/1024))MB (+$(((after_kb-before_kb)/1024))MB)"
