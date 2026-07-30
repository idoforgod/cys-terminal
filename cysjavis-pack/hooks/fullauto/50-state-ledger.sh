#!/bin/sh
# 50-state-ledger.sh — PostToolUse.d 오버레이 (설계 §2 컴포넌트 3 배선 1·2·3 · 구현물 4a)
#
# 설치 위치: ~/.cys/local/hooks/PostToolUse.d/50-state-ledger.sh
#   → 팩 cys-hook.sh(17-28행)가 hook_event_name 으로 디렉터리를 고르고 `sh "$f"` 로 후행 실행한다.
#     stdin = 훅 JSON 원문, stdout 은 러너가 /dev/null 로 폐기, exit code 는 무시(|| true).
#
# ★불변 상속(OBSERVABILITY 클래스 — cys-hook.sh 6-12행):
#   이 훅은 절대 에이전트를 막지 않는다. 모든 실패를 무해히 흘리고 항상 exit 0, 1초 내 종료.
#   판정 실패·도구 부재·git 부재는 전부 '기록 안 함'으로 강등된다(fail-quiet).
#
# 감지 3종:
#   ① Bash · git commit 성공(--dry-run 제외)  → type=commit  {repo, hash, subject, path}
#   ② Bash · javis_task.py set-status … done   → type=task_done {id, cwd}
#   ③ Write · _round/handoffs/*.md             → type=handoff  {name, path}
#
# 비활성 스위치: CYS_STATE_LEDGER_DISABLE=1 (설치 롤백 전 즉시 무력화용)
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/../_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(50-state-ledger)" >&2; exit 0; }
set +e
[ -n "$CYS_STATE_LEDGER_DISABLE" ] && exit 0

IN=$(cat 2>/dev/null)
[ -n "$IN" ] || exit 0

# G22: 인터프리터 해소는 프리루드 단일 소유(python3→python→py). 사본 재구현 금지(RC4).
PY="${CYS_PY:-}"
[ -n "$PY" ] || exit 0
# 스크립트 해소(팩 갱신 면역): 명시 env > 팩 bin > 사용자 로컬 bin. 셋 다 없으면 무동작.
BIN="${CYS_STATE_LEDGER_BIN:-}"
if [ -z "$BIN" ] || [ ! -f "$BIN" ]; then
  BIN="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_state_ledger.py"
  [ -f "$BIN" ] || BIN="${CYS_LOCAL_DIR:-$HOME/.cys/local}/bin/javis_state_ledger.py"
fi
[ -f "$BIN" ] || exit 0

# 하드 타임아웃(있으면 사용 — macOS 기본 셸에는 timeout 이 없어 대개 빈 값이다).
TO=""
if command -v timeout >/dev/null 2>&1; then TO="timeout 2"
elif command -v gtimeout >/dev/null 2>&1; then TO="gtimeout 2"; fi

# ── 분류는 python 1스폰이 전담한다 ────────────────────────────────────────────
# 셸로 원시 command 를 흘리지 않는다(다중행·따옴표·백틱이 셸 재해석되는 사고면 봉쇄).
# 출력 계약: 1행 KIND · 2행 CWD · 3행 EXTRA · 4행 __CYS_END__ (전부 개행·탭 제거된 단일행)
PARSED=$(printf '%s' "$IN" | $TO "$PY" -c '
import json, re, sys

def out(kind, cwd="", extra=""):
    for v in (kind, cwd, extra):
        sys.stdout.write(str(v).replace("\n", " ").replace("\t", " ") + "\n")
    sys.stdout.write("__CYS_END__\n")
    sys.exit(0)

try:
    d = json.load(sys.stdin)
except Exception:
    out("none")
if not isinstance(d, dict):
    out("none")
tool = d.get("tool_name") or ""
ti = d.get("tool_input") or {}
if not isinstance(ti, dict):
    ti = {}
cwd = d.get("cwd") or ""
if not isinstance(cwd, str) or not cwd.startswith("/"):
    cwd = ""

# ③ handoff — Write 도구의 file_path 가 _round/handoffs/ 매칭
if tool == "Write":
    fp = ti.get("file_path") or ti.get("path") or ""
    if isinstance(fp, str) and ("/_round/handoffs/" in fp or fp.startswith("_round/handoffs/")):
        out("handoff", cwd, fp)
    out("none")

if tool != "Bash":
    out("none")
cmd = ti.get("command") or ""
if not isinstance(cmd, str) or not cmd:
    out("none")

# ② task done — javis_task.py + set-status + 독립 토큰 done 동시 포함
if "javis_task.py" in cmd and "set-status" in cmd and re.search(r"(?<![\w-])done(?![\w-])", cmd):
    try:
        import shlex
        toks = shlex.split(cmd)
    except Exception:
        toks = cmd.split()
    tid = ""
    if "set-status" in toks:
        j = toks.index("set-status") + 1
        VALUED = ("--evidence", "--evidence-artifact", "--skip-reason", "--settled-override")
        while j < len(toks):
            t = toks[j]
            if t.startswith("-"):
                j += 2 if (t in VALUED) else 1
                continue
            tid = t
            break
    out("task_done", cwd, tid)

# ① git commit(생성형) — 조회계열(log/show/status/diff)·commit-msg 하이픈형 제외, --dry-run 제외
if not re.search(r"(?:^|[;&|(\s])git\s+(?:-[-\w]+(?:=\S+)?\s+|-C\s+\S+\s+)*commit(?:\s|$)", cmd):
    out("none")
if "--dry-run" in cmd:
    out("none")
hint = ""
m = re.search(r"git\s+-C\s+(\S+)\s", cmd)
if m:
    hint = m.group(1).strip("\x27\"")
if not hint:
    m = re.match(r"\s*cd\s+(\"[^\"]+\"|\x27[^\x27]+\x27|\S+)\s*&&", cmd)
    if m:
        hint = m.group(1).strip("\x27\"")
out("commit", cwd, hint)
' 2>/dev/null)

[ -n "$PARSED" ] || exit 0
KIND=$(printf '%s\n' "$PARSED" | sed -n '1p')
CWD=$(printf '%s\n' "$PARSED" | sed -n '2p')
EXTRA=$(printf '%s\n' "$PARSED" | sed -n '3p')
[ "$(printf '%s\n' "$PARSED" | sed -n '4p')" = "__CYS_END__" ] || exit 0

MDIR="${CYS_STATE_DIR:-$HOME/.cys/state}/state_ledger"

case "$KIND" in
  commit)
    command -v git >/dev/null 2>&1 || exit 0
    RD="$EXTRA"
    # 힌트가 미확장 변수/틸드로 온 경우 최소 확장($HOME·~ 접두만 — eval 금지)
    case "$RD" in
      '$HOME'/*) RD="$HOME${RD#\$HOME}" ;;
      '~'/*)     RD="$HOME${RD#\~}" ;;
    esac
    [ -n "$RD" ] && [ -d "$RD" ] || RD="$CWD"
    [ -n "$RD" ] && [ -d "$RD" ] || exit 0
    TOP=$($TO git -C "$RD" rev-parse --show-toplevel 2>/dev/null)
    [ -n "$TOP" ] || exit 0
    # 성공 판정(exit code 는 훅 입력에 없다): HEAD 가 '방금' 생긴 커밋일 때만 기록한다.
    #   실패한 commit 은 HEAD 를 갱신하지 않으므로 오래된 %ct 로 걸러지고,
    #   동일 HEAD 재기록은 아래 마커가 막는다(재실행·중복 훅 대비).
    INFO=$($TO git -C "$TOP" log -1 --format='%H%n%ct%n%s' 2>/dev/null)
    [ -n "$INFO" ] || exit 0
    HASH=$(printf '%s\n' "$INFO" | sed -n '1p')
    CT=$(printf '%s\n' "$INFO" | sed -n '2p')
    SUBJ=$(printf '%s\n' "$INFO" | sed -n '3p')
    case "$CT" in ''|*[!0-9]*) exit 0 ;; esac
    NOW=$(date +%s 2>/dev/null)
    case "$NOW" in ''|*[!0-9]*) exit 0 ;; esac
    [ $((NOW - CT)) -le 300 ] || exit 0
    mkdir -p "$MDIR" 2>/dev/null
    KEY=$(printf '%s' "$TOP" | tr '/ .' '___')
    MARK="$MDIR/lastcommit-$KEY"
    [ "$(cat "$MARK" 2>/dev/null)" = "$HASH" ] && exit 0
    REPO=$(basename "$TOP")
    SHORT=$(printf '%s' "$HASH" | cut -c1-12)
    if $TO "$PY" "$BIN" append --type commit --actor post-tool-hook \
        --field "repo=$REPO" --field "hash=$SHORT" --field "subject=$SUBJ" \
        --field "path=$TOP" >/dev/null 2>&1; then
      printf '%s' "$HASH" > "$MARK" 2>/dev/null
    fi
    ;;
  task_done)
    [ -n "$EXTRA" ] || exit 0
    $TO "$PY" "$BIN" append --type task_done --actor post-tool-hook \
      --field "id=$EXTRA" --field "cwd=$CWD" >/dev/null 2>&1
    ;;
  handoff)
    [ -n "$EXTRA" ] || exit 0
    NAME=$(basename "$EXTRA")
    $TO "$PY" "$BIN" append --type handoff --actor post-tool-hook \
      --field "name=$NAME" --field "path=$EXTRA" >/dev/null 2>&1
    ;;
esac
exit 0
