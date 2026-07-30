#!/usr/bin/env bash
# PreToolUse hook (matcher Edit|Write|NotebookEdit): appbuild "코드 선행 금지" 게이트.
# 의도(워크플로 가드레일, 보안 경계 아님): .appbuild 프로젝트에서 완료 게이트(05-gate.md)가
# 파생되기 전에는 본 소스 코드 작성을 막아 "기획·검증부터" 순서를 강제한다.
#
# Threat model = 실수 방지(워커가 기획 건너뛰고 코드부터). 따라서 **fail-OPEN**이 기본:
# - .appbuild 마커가 없으면(=appbuild 프로젝트 아님) 무조건 허용 → cysjavis 자체 개발·무관
#   작업을 절대 막지 않는다.
# - python3 부재·JSON 파싱 실패·판단 불가 → 허용(작업 차단보다 통과가 안전측).
# 막는 경우는 오직: .appbuild 존재 + 05-gate.md 부재 + 대상이 소스 확장자일 때(exit 2).

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(appbuild-gate)" >&2; exit 0; }
# G22: 인터프리터 경성 참조 제거(python3→python→py). 미해소 = 판정 불가 → fail-open(기존 계약).
[ -n "${CYS_PY:-}" ] || exit 0
INPUT="$(cat)" || exit 0
export APPBUILD_HOOK_INPUT="$INPUT"

if [ "${1:-}" = "--self-test" ]; then export APPBUILD_SELF_TEST=1; fi

exec "$CYS_PY" - <<'PYEOF'
import json, os, sys

SRC_EXT = {
    ".js",".jsx",".ts",".tsx",".mjs",".cjs",".vue",".svelte",".py",".rb",".php",
    ".go",".rs",".java",".kt",".swift",".c",".cc",".cpp",".h",".hpp",".cs",
    ".css",".scss",".sass",".less",".html",".htm",".sql",
}

def project_has_appbuild(path):
    """file_path에서 상향 탐색해 .appbuild 디렉토리를 가진 루트를 찾는다. (root|None)."""
    d = os.path.dirname(os.path.abspath(path))
    seen = 0
    while d and d != os.path.dirname(d) and seen < 64:
        if os.path.isdir(os.path.join(d, ".appbuild")):
            return d
        d = os.path.dirname(d); seen += 1
    return None

def decide(file_path):
    """(block: bool, reason)."""
    if not file_path:
        return False, "no file_path"
    root = project_has_appbuild(file_path)
    if not root:
        return False, "not an appbuild project (fail-open)"
    ab = os.path.join(root, ".appbuild")
    absf = os.path.abspath(file_path)
    # .appbuild/ 내부(기획 문서·상태)는 항상 허용
    if absf.startswith(os.path.abspath(ab) + os.sep):
        return False, "writing planning doc under .appbuild/"
    # 게이트가 파생됐으면 빌드 단계 — 허용
    if os.path.isfile(os.path.join(ab, "05-gate.md")):
        return False, "gate derived — build phase"
    # 게이트 전: 소스 확장자만 차단(문서·설정은 허용)
    ext = os.path.splitext(file_path)[1].lower()
    if ext in SRC_EXT:
        return True, "gate not derived — run /appbuild planning+supervisor first"
    return False, "non-source file before gate"

def main():
    try:
        data = json.loads(os.environ.get("APPBUILD_HOOK_INPUT", ""))
    except ValueError:
        sys.exit(0)  # fail-open
    if not isinstance(data, dict):
        sys.exit(0)
    ti = data.get("tool_input")
    fp = ti.get("file_path") if isinstance(ti, dict) else None
    block, reason = decide(fp)
    if block:
        print("appbuild-gate BLOCKED: %s (.appbuild 존재·05-gate.md 부재 — 기획→검증→게이트 "
              "후 빌드)" % reason, file=sys.stderr)
        sys.exit(2)
    sys.exit(0)

def self_test():
    import tempfile, shutil
    fails = []
    base = tempfile.mkdtemp(prefix="appbuild-gate-test-")
    try:
        # ① appbuild 프로젝트·게이트 전·소스 → 차단
        proj = os.path.join(base, "proj"); os.makedirs(os.path.join(proj, ".appbuild"))
        b, _ = decide(os.path.join(proj, "src", "app.tsx"))
        if not b: fails.append("게이트 전 소스가 차단되지 않음")
        # ② 같은 프로젝트·문서는 허용
        b, _ = decide(os.path.join(proj, "README.md"))
        if b: fails.append("문서가 차단됨(허용이어야)")
        # ③ .appbuild 내부 기획 문서 허용
        b, _ = decide(os.path.join(proj, ".appbuild", "01-prd.md"))
        if b: fails.append(".appbuild 내부 문서가 차단됨")
        # ④ 게이트 파생 후 소스 허용
        open(os.path.join(proj, ".appbuild", "05-gate.md"), "w").write("gate")
        b, _ = decide(os.path.join(proj, "src", "app.tsx"))
        if b: fails.append("게이트 후 소스가 차단됨(빌드 단계여야)")
        # ⑤ 비-appbuild 프로젝트는 fail-open(허용) — cysjavis 자체·무관 작업 불간섭
        other = os.path.join(base, "other"); os.makedirs(os.path.join(other, "src"))
        b, _ = decide(os.path.join(other, "src", "x.ts"))
        if b: fails.append("비-appbuild 소스가 차단됨(fail-open 위반)")
        # ⑥ file_path 없음 → 허용
        b, _ = decide(None)
        if b: fails.append("file_path 없음이 차단됨")
    finally:
        shutil.rmtree(base, ignore_errors=True)
    if fails:
        print("\n".join(fails), file=sys.stderr)
        print("self-test: %d 실패" % len(fails), file=sys.stderr); sys.exit(1)
    print("self-test OK: 6 케이스(차단1·허용5) · fail-open 검증"); sys.exit(0)

if os.environ.get("APPBUILD_SELF_TEST"):
    self_test()
else:
    main()
PYEOF
