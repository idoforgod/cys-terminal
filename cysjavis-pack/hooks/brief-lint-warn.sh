#!/bin/sh
# brief-lint-warn.sh — 인세션 위임(Task|Agent) 브리프 경고 훅 (Phase 1 T1 · DESIGN-DECISIONS §1-3)
#
# 계급 OBSERVABILITY — **어떤 오류도 exit 0**(fail-open · 조건 05③·21①). 차단 exit 절대 금지:
# hard(fail-closed) 경로는 javis_task checkout --brief 뿐이다(비대칭 원칙 — 조건 28).
# 등록 대상: PostToolUse(Task|Agent) — W0-2 verify-reminder.sh 라인의 승계·흡수. 라이브 등록은
# [OT-2] 오너 승인 후 개별 항목 등록(디스패처 편입 단독은 무음 비활성 — D22). ★등록 대상표는
# completion-guard 와 분리 독립(설계 §1-3): 경고 훅 = master `~/.claude` **포함** / guard = 워커
# 2프로필(`~/.claude-cysinsight`+`~/.cys/claude`) — C74(B2) 판정 기준이 이 표를 핀한다.
#
# ★조건 11 흡수 선언(설계 §1-3): javis_verify_reminder.py 는 descope(동일 기능 이중 구현 금지) —
#   그 역할(위임 브리프 verify 부재 경고 주입)을 이 훅이 승계한다. 승계 계약 2:
#   ① 위임당 1회 마커 억제 — $JAVIS_ROOT/_round/tasks/.brief-warned/<key> (key = surface/session
#     + 브리프 경로(무브리프면 프롬프트 head) 해시 — 같은 위임의 반복 경고 차단)
#   ② 주입 텍스트 300자 캡 — master 컨텍스트 팽창 차단.
# 동작: stdin(PostToolUse JSON)에서 Task|Agent 위임의 브리프 파일 참조를 찾는다.
#   · 브리프 미동봉 → 파일 참조 위임(BRIEF_CONTRACT) 권고 1회 주입(additionalContext · 비차단)
#   · 브리프 동봉 + lint 비0 → 상위 finding 요약 1회 주입(비차단)
#   · 그 외(정상 브리프·파싱 실패·무관 도구) → 무출력 exit 0

# ── 공용 프리루드(CS-4①) — loud-skip: 소실 시 조용히 꺼지지 않고 stderr 1줄 후 강등 ──
. "$(dirname "$0")/_lib.sh" 2>/dev/null \
  || . "${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" 2>/dev/null \
  || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(brief-lint-warn)" >&2; exit 0; }

[ -n "${CYS_PY:-}" ] || exit 0        # G22 fail-open: python(3) 해소 불가
HOOK_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || exit 0
LINT_PY="$HOOK_DIR/../bin/javis_brief_lint.py"
[ -f "$LINT_PY" ] || exit 0           # 대상 부재 = 조용히 통과(H-HOOK-2)

IN=$(cat 2>/dev/null) || exit 0

printf '%s' "$IN" | "$CYS_PY" -c '
import hashlib, json, os, re, sys, time
try:
    data = json.loads(sys.stdin.read())
    tin = data.get("tool_input") or {}
    tname = str(data.get("tool_name") or "")
    if tname not in ("Task", "Agent"):
        sys.exit(0)
    prompt = ""
    for k in ("prompt", "description", "input"):
        v = tin.get(k)
        if isinstance(v, str):
            prompt += "\n" + v
    m = re.search(r"(?:--brief\s+)?(\S*briefs?/\S+\.md|\S+-brief\.md)", prompt)
    brief_path = m.group(1) if m else None
    root = os.environ.get("JAVIS_ROOT") or os.getcwd()
    # B10①: 매치된 브리프 경로가 cwd 기준 미해소면 JAVIS_ROOT 기준 재시도(상대경로 위임 관행)
    if brief_path and not os.path.isfile(brief_path):
        alt = os.path.join(root, brief_path)
        if os.path.isfile(alt):
            brief_path = alt
    mark_dir = os.path.join(root, "_round", "tasks", ".brief-warned")
    # B10②: 마커 디렉토리 진입 시 mtime 7일+ 파일 정리(스테일 마커의 영구 억제 차단)
    for fn in (os.listdir(mark_dir) if os.path.isdir(mark_dir) else []):
        fp = os.path.join(mark_dir, fn)
        try:
            if time.time() - os.path.getmtime(fp) > 7 * 86400:
                os.remove(fp)
        except OSError:
            pass
    sid = (os.environ.get("CYS_SURFACE_ID") or os.environ.get("AITERM_SURFACE_ID")
           or str(data.get("session_id") or ""))
    key_src = brief_path or prompt[:200] or tname
    key = hashlib.sha256((sid + "\x00" + key_src).encode("utf-8", "replace")).hexdigest()[:16]
    mark = os.path.join(mark_dir, key)
    if os.path.exists(mark):
        sys.exit(0)  # 위임당 1회 억제(조건 11 승계)
    msg = None
    if brief_path and os.path.isfile(brief_path):
        sys.path.append(os.path.dirname(sys.argv[1]))
        import javis_brief_lint as bl
        code, findings = bl.lint_file(brief_path)
        if code != 0:
            msg = "[brief-lint] %s: %s" % (os.path.basename(brief_path),
                                           "; ".join(findings[:2]) or "형식 경고")
    elif brief_path:
        # B10①: JAVIS_ROOT 재시도 후에도 부재 — 경고 주입(fail-open 유지·비차단)
        msg = "[brief-lint] 참조 브리프 파일 부재: %s — 경로 확인 후 재위임 권고(비차단)" % brief_path
    else:
        msg = ("[brief-lint] 브리프 미동봉 위임 감지 — 파일 참조 위임(BRIEF_CONTRACT: "
               "_round/briefs/<task>.md 경로+1줄 요약 push) + 실행형 verify[] 동봉 권고. "
               "비차단 경고(fail-open)·위임당 1회.")
    if msg:
        os.makedirs(mark_dir, exist_ok=True)
        with open(mark, "w", encoding="utf-8") as f:
            f.write("")
        msg = msg[:300]  # 주입 300자 캡(조건 11 승계)
        print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse",
                                                 "additionalContext": msg}},
                         ensure_ascii=False))
    sys.exit(0)
except SystemExit:
    raise
except Exception:
    sys.exit(0)  # fail-open — 어떤 내부 오류도 무해 통과
' "$LINT_PY" 2>/dev/null || exit 0
exit 0
