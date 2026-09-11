#!/bin/sh
# win-typecheck.sh — cys-app(src-tauri) 전체를 윈도우 타깃(x86_64-pc-windows-msvc) cfg 로 타입체크·린트한다.
#   맥·리눅스 호스트에서 윈도우 머신 없이 돈다. docs/RELEASE.md §0-C 태그 전 사전 게이트 2번 · ci-branch.yml macOS 잡.
#
# ★왜 존재하는가(2026-09-11 · v0.14.34 윈도우 빌드 파손 재발 방지 P1): src-tauri/src/main.rs 의 유닉스 전용 API
#   (std::os::unix MetadataExt 의 dev/ino)가 cfg 밖에 있어 윈도우에서만 컴파일 오류 3건(E0433·E0599)이 났다.
#   macOS 레인(ci-branch)은 윈도우 cfg 를 컴파일하지 않고, windows-health 는 루트 크레이트만 컴파일해 cys-app
#   파손을 원리적으로 못 본다. 그래서 파손이 태그 뒤 윈도우 릴리스 빌드에서야 드러났다.
#   계측 유효성: 파손 커밋 88c1ca2 에서 CI 와 같은 오류 3건을 재현했고, 수리 뒤 트리에서는 오류 0 이다.
#
# ★증명하는 것 / 못 하는 것(과신 금지):
#   증명 — cys-app 의 bin·bin test 와 그 의존(루트 cys-terminal lib 포함) Rust 코드 전부가 윈도우 cfg 로
#          타입체크·린트를 통과한다.
#   못 함 — 링크·NSIS 번들·런타임 동작. C 코드(ring·sqlite)와 리소스 컴파일은 아래처럼 우회하므로 그건
#          windows-build·release 레인(윈도우 실기)이 담당한다.
#
# 우회는 네이티브 산출물 4곳뿐이다(Rust 타입체크와 무관 · 저장소 파일 무수정 · 부산물은 target/win-typecheck/):
#   ① ring(C 암호)             : 빌드 스크립트 override — links 키는 cargo metadata 에서 도출(버전이 올라가도 따라간다)
#   ② libsqlite3-sys(C sqlite) : 링크 모드(LIBSQLITE3_SYS_USE_PKG_CONFIG=1 + 빈 SQLITE3_LIB_DIR) → 동봉 바인딩만 쓴다
#   ③ tauri-build 리소스(.rc)  : RC_x86_64_pc_windows_msvc = 빈 산출물만 쓰는 check 전용 stub
#   ④ tauri 번들 입력          : TAURI_CONFIG 로 externalBin·resources 를 비우고 frontendDist 를 빈 폴더로
#                                (번들 단계의 입력일 뿐 — 사이드카·동봉 런타임은 bundle-prep 이 윈도우에서 만든다)
#
# 사용: sh scripts/win-typecheck.sh      (리포 안 어디서든)
#   사전: rustup target add x86_64-pc-windows-msvc (자동 설치하지 않는다 — 없으면 exit 2 로 알린다)
#   cargo 메시지 원본(JSON)은 target/win-typecheck/cargo-messages.json 에 남는다.
# exit 0 = 통과(윈도우 컴파일 오류 0) · 1 = 윈도우 컴파일 오류(태그 금지)
#      2 = 판정 불가(도구·타깃 부재 · 우회 실패 · cys-app 이 검사되지 않음) — 통과가 아니다. CI 에서는 잡을 실패시킨다.
set -u
TARGET=x86_64-pc-windows-msvc
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "win-typecheck: 판정 불가 — git 리포 밖에서 실행됐다" >&2; exit 2; }
cd "$ROOT" || exit 2
if [ -d "$HOME/.cargo/bin" ]; then PATH="$HOME/.cargo/bin:$PATH"; export PATH; fi
for t in cargo rustc python3; do
  command -v "$t" >/dev/null 2>&1 || { echo "win-typecheck: 판정 불가 — $t 없음" >&2; exit 2; }
done
LIBDIR=$(rustc --print target-libdir --target "$TARGET" 2>/dev/null)
if [ -z "$LIBDIR" ] || ! ls "$LIBDIR"/libcore-*.rlib >/dev/null 2>&1; then
  echo "win-typecheck: 판정 불가 — $TARGET 표준 라이브러리가 없다(rustup target add $TARGET)" >&2
  exit 2
fi
# 부산물 위치를 고정한다 — 우회 값(경로)이 실행마다 같아야 cargo 가 다시 빌드하지 않는다(웜 실행이 빠르다).
WORK="${CARGO_TARGET_DIR:-$ROOT/target}/win-typecheck"
mkdir -p "$WORK/sqlite-empty" "$WORK/dist" || { echo "win-typecheck: 판정 불가 — $WORK 를 만들지 못했다" >&2; exit 2; }
cat > "$WORK/fake-windres" <<'EOF'
#!/bin/sh
# check 전용 stub(win-typecheck.sh ③): embed-resource 가 '-V' 로 종류를 물으면 windres 라고 답하고
# '--output <f>' 에 빈 파일을 쓴다. 리소스 컴파일·링크 증명이 아니다.
case " $* " in
  *" -V "*) echo "GNU windres (win-typecheck check-only stub)"; exit 0 ;;
esac
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "--output" ]; then out="$2"; shift; fi
  shift
done
[ -n "$out" ] || { echo "fake-windres: --output 없음" >&2; exit 2; }
: > "$out"
EOF
chmod +x "$WORK/fake-windres"
# ① ring links 키 — 윈도우 타깃으로 걸러낸 의존 그래프에서 ring 패키지의 links 값(여러 버전이면 전부).
LINKS=$(cargo metadata --format-version 1 --manifest-path src-tauri/Cargo.toml --filter-platform "$TARGET" 2>"$WORK/metadata.err" \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(" ".join(sorted({p["links"] for p in d["packages"] if p["name"] == "ring" and p.get("links")})))') \
  || { echo "win-typecheck: 판정 불가 — cargo metadata 실패" >&2; cat "$WORK/metadata.err" >&2; exit 2; }
set --
for l in $LINKS; do
  set -- "$@" --config "target.$TARGET.$l.rustc-link-lib=[]"
done
export RC_x86_64_pc_windows_msvc="$WORK/fake-windres"
export LIBSQLITE3_SYS_USE_PKG_CONFIG=1
export SQLITE3_LIB_DIR="$WORK/sqlite-empty"
export TAURI_CONFIG="{\"build\":{\"frontendDist\":\"$WORK/dist\"},\"bundle\":{\"externalBin\":[],\"resources\":[]}}"
JSON="$WORK/cargo-messages.json"
echo "win-typecheck: $TARGET · HEAD $(git rev-parse --short HEAD) · 미커밋 $(git status --porcelain | wc -l | tr -d ' ')건 · ring override: ${LINKS:-없음}"
cargo check --target "$TARGET" --manifest-path src-tauri/Cargo.toml --all-targets "$@" --message-format=json >"$JSON" 2>"$WORK/cargo.stderr"
RC=$?
python3 - "$JSON" "$ROOT" "$RC" "$WORK/cargo.stderr" <<'PY'
import json, os, sys

path, root, rc, errf = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
pre = "path+file://" + root
app = pre + "/src-tauri#"
gha = os.environ.get("GITHUB_ACTIONS") == "true"
arts, msgs = set(), []
app_normal = app_test = False
for line in open(path, encoding="utf-8", errors="replace"):
    try:
        d = json.loads(line)
    except ValueError:
        continue
    pid = d.get("package_id", "")
    if not (pid.startswith(pre + "#") or pid.startswith(pre + "/")):
        continue  # 우리 크레이트(리포 안 경로 패키지)만 센다
    tgt = d.get("target", {})
    name = "%s[%s]" % (tgt.get("name"), ",".join(tgt.get("kind", [])))
    if d.get("reason") == "compiler-artifact":
        prof = "test" if d.get("profile", {}).get("test") else "normal"
        arts.add((name, prof))
        if pid.startswith(app):
            app_normal |= prof == "normal"
            app_test |= prof == "test"
    elif d.get("reason") == "compiler-message":
        m = d.get("message", {})
        lvl, text = m.get("level"), m.get("message", "")
        if lvl not in ("error", "warning") or text.startswith("aborting due to"):
            continue
        sp = next((s for s in m.get("spans", []) if s.get("is_primary")), None)
        f, ln = (sp["file_name"], sp["line_start"]) if sp else ("", 0)
        # 리포 루트 기준 경로로 맞춘다(annotation 이 파일을 가리키게) — rustc 는 이미 루트 기준일 때도 있다.
        if f and os.path.isabs(f) and f.startswith(root + "/"):
            f = f[len(root) + 1:]
        elif f and not os.path.isabs(f) and not os.path.exists(os.path.join(root, f)) and os.path.exists(os.path.join(root, "src-tauri", f)):
            f = "src-tauri/" + f
        code = (m.get("code") or {}).get("code") or ""
        msgs.append((lvl, name, f, ln, code, text))
msgs = list(dict.fromkeys(msgs))  # bin 과 bin test 가 같은 진단을 두 번 낸다 — 한 번만 센다
errors = [x for x in msgs if x[0] == "error"]
warns = [x for x in msgs if x[0] == "warning"]
print("[윈도우 cfg 로 검사된 우리 크레이트 타깃]")
for a in sorted(arts):
    print("  ", a[0], a[1])
print("[오류·경고]")
for x in msgs:
    print("  ", x[0], x[1], "%s:%s" % (x[2], x[3]), x[4], x[5])
print("합계: 오류 %d · 경고 %d · cargo rc=%d" % (len(errors), len(warns), rc))
if gha:
    for x in errors:
        print("::error file=%s,line=%s,title=win-typecheck %s::%s" % (x[2], x[3], x[4], x[5].replace("\n", " ")))
if errors:
    print("판정: 1 — 윈도우 컴파일 오류(태그 금지)")
    sys.exit(1)
if rc != 0:
    print("판정: 2 — cargo 가 실패했는데 우리 코드의 컴파일 오류는 없다(빌드 스크립트·우회 실패 의심 — 통과 아님)")
    for l in open(errf, encoding="utf-8", errors="replace").read().splitlines()[-25:]:
        print("   |", l)
    if gha:
        print("::error title=win-typecheck 판정 불가::cargo rc=%d · 우리 코드 오류 0 — 빌드 스크립트·우회 실패" % rc)
    sys.exit(2)
if not (app_normal and app_test):
    print("판정: 2 — cys-app 의 bin·bin test 가 검사되지 않았다(계측 무효 — 통과 아님)")
    if gha:
        print("::error title=win-typecheck 판정 불가::cys-app 타깃이 검사되지 않았다")
    sys.exit(2)
print("판정: 0 — 통과(윈도우 컴파일 오류 0)")
PY
exit $?
