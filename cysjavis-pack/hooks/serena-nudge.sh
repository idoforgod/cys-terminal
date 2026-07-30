#!/usr/bin/env bash
# serena-nudge.sh — PreToolUse reminder-only nudge (S5 Layer 1, 보강).
# 설계서 §S5. Opus 가 code 작업에서 Read/Grep 으로 회귀하는 drift 를 reminder 로 교정한다.
#
# cys 불변(절대 위반 금지):
#   - ALWAYS exit 0 (never exit 2) — cys hook-never-block 불변.
#   - additionalContext 만 출력 — NEVER updatedInput (RTK cys-rewrite.sh 가 PreToolUse:Bash
#     updatedInput 을 소유; 둘이 쓰면 double-rewrite 충돌).
#   - reminder-only — S7 net-win 입증 전 deny 로 승격 금지.
#   - serena MCP tool 사용 중이거나 비코드 작업이면 침묵(산문/SOT/설교/markdown=비코드, Serena 0).
#
# 설치(human-hold/denylist): 노드 settings.json PreToolUse 에 RTK cys-rewrite tuple **뒤** 등록.
#   {"matcher":"","hooks":[{"type":"command","command":"$HOME/.cys/pack/hooks/serena-nudge.sh"}]}
#   worker-only 권장(RTK U0 master-glob gap 회피 · stanza master skip 과 정합).
# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(serena-nudge)" >&2; exit 0; }
# G22: 인터프리터 경성 참조 제거. 미해소 = 넛지 불가 → 무해 통과(never-block 불변 유지).
[ -n "${CYS_PY:-}" ] || exit 0

exec "$CYS_PY" -c '
import sys, json, os

def out_empty():
    sys.exit(0)

try:
    data = json.load(sys.stdin)
except Exception:
    out_empty()

# 유효하지만 비-객체 JSON(list/str/number/null)도 malformed 로 간주 → exit 0(never-block 불변).
if not isinstance(data, dict):
    out_empty()

tool = data.get("tool_name", "") or ""
sid = data.get("session_id") or "default"
sid = "".join(c for c in str(sid) if c.isalnum() or c in "-_")[:64] or "default"
state_dir = os.path.expanduser("~/.cys/state")
sf = os.path.join(state_dir, "serena-nudge-" + sid + ".json")
THRESHOLD = 3

def load():
    try:
        with open(sf, encoding="utf-8") as f:
            d = json.load(f)
        return d if isinstance(d, dict) else {"streak": 0, "nudged": False}
    except Exception:
        return {"streak": 0, "nudged": False}

def save(st):
    try:
        os.makedirs(state_dir, exist_ok=True)
        tmp = sf + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(st, f)
        os.replace(tmp, sf)
    except Exception:
        pass

st = load()

# serena tool 사용 중 -> 카운터 리셋, nudge 없음.
if tool.startswith("mcp__serena"):
    st["streak"] = 0
    st["nudged"] = False
    save(st)
    out_empty()

# code discovery 회귀 추적: Read/Grep/Glob 연속.
if tool in ("Read", "Grep", "Glob"):
    st["streak"] = int(st.get("streak", 0)) + 1
else:
    save(st)
    out_empty()

if st["streak"] >= THRESHOLD and not st.get("nudged"):
    st["nudged"] = True
    save(st)
    msg = ("[serena-nudge] Read/Grep 를 코드에 " + str(st["streak"]) +
           "회 연속 사용했다. Serena MCP tool 이 있으면 get_symbols_overview / find_symbol / "
           "find_referencing_symbols 를 통째-Read·전체-Grep 보다 우선하라(code-nav 토큰 대폭 절감). "
           "Serena tool 이 deferred 면 tool search 로 지금 로드하라. (reminder only · 비코드 파일엔 "
           "Serena 미적용)")
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse",
                                              "additionalContext": msg}}, ensure_ascii=False))
    sys.exit(0)

save(st)
out_empty()
'
