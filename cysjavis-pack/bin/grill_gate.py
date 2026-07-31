#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""grill_gate.py — grill-me 최소 질문(결정론 floor) 게이트 엔진.

목적(오너 요구 2026-06-27): grill-me 사용 시 "서로 다른 결정 브랜치"를 최소 20개
(아주 복잡한 작업은 30개) 해소하기 전에는 합의 결과물 쓰기(구현)로 넘어가지 못하게
'강제'한다. 순수 프롬프트 지시는 권고일 뿐이라(producer=evaluator 단일체) 결정론
게이트가 필요하다.

★eval-driven 3분리(앵커5-4 무결성):
  producer  = 질문하는 LLM (이 스크립트가 아님)
  evaluator = 이 스크립트(count) — AskUserQuestion PostToolUse hook이 호출하며, 자체
              누적 마커에 distinct decision_axis를 결정론으로 셈(LLM 자기보고 불신)
  gatekeeper= grill-gate.sh(PreToolUse check) — distinct<floor면 Edit/Write를 deny

서브커맨드:
  begin  : 복잡도를 javis_route.py(+보조 토큰)로 판정 → floor 20/30 → 마커 생성.
           ★surface 격리 불가(CYS_SURFACE_ID 부재)면 마커를 만들지 않고 fail-open
           (cross-node 마비 방지 — 적대검증 R: sid 공유 차단).
  count  : AskUserQuestion PostToolUse hook JSON(stdin)에서 header를 decision_axis로
           정규화·누적. ★호출당 distinct 최대 1개(one-at-a-time 강제 — 배치 우회 차단).
           취소·에러 응답은 불인정.
  check  : distinct<floor면 exit 2(차단). 충족이면 status=passed·exit 0.
  end    : floor 충족 시 done(통과). ★미충족이면 거부(exit 2) — 오너 조기중단만
           `end --force`로 abandoned 처리(흔적 남김·감사 가능, 우회 탐지).
  status : 마커 현황 JSON.
  --self-test : 자체 검증(외부 의존 0).

마커: <CYS_ROOT>/_round/.grill_session.<surface>.json (노드별 격리).
      GRILL_MARKER 환경변수로 오버라이드(self-test·격리).

★fail-open 원칙(threat model = 비악의 오작동 방지, 적대 봉쇄 아님):
  마커 부재·TTL 만료·status=passed/done/abandoned·파싱 실패·판단 불가 → 전부 통과측.

★B2 self-cap(Phase 1 Wave B2 · DESIGN-DECISIONS §3 · D2): grill 과 completion-guard 는
  하네스 Stop 블록 카운터(합산 지갑·cap 기본 8)를 공유한다 — grill 블록이 지갑을 선점하면
  guard 의 escalation(N=3) 도달 전 무검증 오버라이드가 가능(T0 §2 실험 4·K≥5 산술).
  self-cap 은 grill 자신의 블록 예산에 상한을 둔다:
    발동 조건 = env CYS_COMPLETION_GUARD=1 인 pane **한정** + GRILL_STOP_SELF_CAP ≥1
    (기본 0 = **비활성** — 현행 동작 무변. guard 미무장 함대엔 부수효과 0).
    카운터 = 마커(grill 사이클) 귀속 필드 stop_self_blocks — 새 사이클(새 마커) = 리셋.
    소진 시 check 는 차단 대신 **자진 통과(exit 0)** 하고 마커 self_cap_yields 에 사유를
    기록한다(FLOOR 미충족 감사 가시화 — 통과가 충족 위장이 되지 않게 status 는 불변).
    grill-stop.sh·grill-gate.sh 는 무변경(구현 소재 = 이 엔진 · 조건 24③ 훅 개정 회피) —
    두 훅이 같은 check 를 부르므로 카운터는 Stop·PreToolUse 블록을 구분 없이 계상한다
    (자인: PreToolUse deny 는 하네스 Stop 지갑을 쓰지 않지만, 무장 pane 의 총 블록 압력
    상한이라는 보수측 해석으로 수용 — 정밀 분리는 훅 개정 없이는 불가).
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time

DEFAULT_FLOOR_SIMPLE = 20
DEFAULT_FLOOR_COMPLEX = 30
DEFAULT_TTL_SECS = 6 * 3600
EDIT_DISTANCE_DUP = 2          # 긴 라벨(둘 다 >4자)에만 적용 — 짧은 한글 축 과다병합 방지
PASS_STATES = ("passed", "done", "abandoned")   # evaluate가 통과시키는 상태
# router fast로 새는 복잡 작업을 floor 30으로 끌어올리는 보조 신호.
# ★복합 구문 위주(단일 일반어 '워커·전면·분산·migration'은 trivial 오타수정까지 오탐 →
#   floor 불필요 상향 fail-closed. 라운드2 NEW-3 교정으로 제거). 명백한 대규모 신호만 둔다.
COMPLEX_TOKENS = (
    "비가역", "irreversible", "여러 서브시스템", "multiple subsystem",
    "multi-subsystem", "전체 시스템 재설계", "아키텍처 재설계", "전면 재설계",
    "system overhaul", "대규모 마이그레이션", "전면 개편", "전면 재작성",
)


# ── 정수 안전 변환(floor null/비수치 크래시 방지 — 적대검증 교정) ──────────
def _int(v, default):
    try:
        return int(v)
    except (TypeError, ValueError):
        return default


# ── 경로 ──────────────────────────────────────────────────────────────────
def _cys_root():
    v = os.environ.get("CYS_ROOT", "").strip()
    if v:
        return v
    return os.path.join(os.path.expanduser("~"), "Desktop", "CYSjavis")


def _surface_id():
    return re.sub(r"[^0-9A-Za-z_-]", "", os.environ.get("CYS_SURFACE_ID", "").strip())


def _marker_path():
    v = os.environ.get("GRILL_MARKER", "").strip()
    if v:
        return v
    # ★노드별 격리: 마커를 surface(PTY 상속 CYS_SURFACE_ID)별로 둔다.
    sid = _surface_id()
    suffix = (".grill_session.%s.json" % sid) if sid else ".grill_session.json"
    return os.path.join(_cys_root(), "_round", suffix)


def _route_py():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_route.py")


# ── 마커 I/O (전부 fail-soft) ────────────────────────────────────────────
def _load_marker():
    p = _marker_path()
    try:
        with open(p, encoding="utf-8") as f:
            d = json.load(f)
        return d if isinstance(d, dict) else None
    except (OSError, ValueError):
        return None


def _save_marker(m):
    p = _marker_path()
    d = os.path.dirname(p)
    if d:
        os.makedirs(d, exist_ok=True)
    tmp = p + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(m, f, ensure_ascii=False, indent=2)
    os.replace(tmp, p)


def _expired(m):
    try:
        started = float(m.get("started_at", 0))
        ttl = float(m.get("ttl_secs", DEFAULT_TTL_SECS))
    except (TypeError, ValueError):
        return False
    return started > 0 and (time.time() - started) > ttl


# ── decision_axis 정규화·중복판정 (reward-hack 차단 핵심) ──────────────────
def _normalize_axis(s):
    """표면 라벨 → 비교용 정규형. 소문자·영숫자/한글만·후행 쪼개기 숫자접미 제거."""
    if not isinstance(s, str):
        return ""
    s = s.strip().lower()
    s = re.sub(r"[^0-9a-z가-힣]+", "", s)
    # 순수 인덱스 라벨(쪼개기 위장: a1·b2·3·x) = 의미 0 → 빈 정규형(불인정)
    if re.fullmatch(r"[a-z]?\d+|[a-z]", s):
        return ""
    # 후행 숫자 접미 거부: authmethod1 / auth1b → 어근
    if len(s) > 3:
        s = re.sub(r"(\d+[a-z]?)$", "", s)
    return s


def _edit_distance(a, b):
    if a == b:
        return 0
    la, lb = len(a), len(b)
    if abs(la - lb) > EDIT_DISTANCE_DUP:
        return EDIT_DISTANCE_DUP + 1
    prev = list(range(lb + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[-1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def _suffix_only(longer, shorter):
    """longer == shorter + (숫자/단일문자 접미)인가 — '롤백'⊄'롤백전략' 정당분리 보존."""
    if not longer.startswith(shorter) or longer == shorter:
        return False
    return bool(re.fullmatch(r"\d+[a-z]?|[a-z]", longer[len(shorter):]))


def _is_duplicate(norm, existing):
    """norm 이 기존 축들과 같은 결정인가(쪼개기·접미 위장 거부, 짧은 축 과다병합 방지)."""
    if not norm:
        return True   # 빈 정규형 = 무의미 → 불인정
    for ex in existing:
        if norm == ex:
            return True
        if _suffix_only(norm, ex) or _suffix_only(ex, norm):
            return True
        # 편집거리는 '둘 다 4자 이상'일 때만 — 2~3자 한글 축의 대량 오합산(fail-closed)은
        # 막되, 4자 한글 근접동의어(인증방식/인증방법)는 병합(라운드2 REVISE-1 잔여 교정).
        if len(norm) >= 4 and len(ex) >= 4 and _edit_distance(norm, ex) <= EDIT_DISTANCE_DUP:
            return True
    return False


def _axis_hash(norm):
    return hashlib.sha256(norm.encode("utf-8")).hexdigest()[:12]


# ── begin: 복잡도 결정론 판정 → floor ─────────────────────────────────────
def decide_floor(request_text):
    """(floor, signals). LLM 자판단 아님 — router 토큰 + 보조 복잡 토큰으로만 결정."""
    signals = []
    floor = DEFAULT_FLOOR_SIMPLE
    rp = _route_py()
    if request_text and os.path.isfile(rp):
        try:
            out = subprocess.run(
                [sys.executable, rp, "--request", request_text],
                capture_output=True, text=True, timeout=15)
            data = json.loads(out.stdout or "{}")
            mode = data.get("mode")
            if mode == "slow":
                floor = DEFAULT_FLOOR_COMPLEX
                signals.append("router:slow(%s)" % data.get("matched_token"))
            elif mode == "deliberate":
                signals.append("router:deliberate")
            else:
                signals.append("router:fast")
        except (OSError, ValueError, subprocess.SubprocessError):
            signals.append("router:unavailable(default-20)")
    else:
        signals.append("router:no-request(default-20)")
    # 보조 신호: router가 fast로 흘린 비가역·다서브시스템·위임형 복잡 작업을 30으로 승격
    if floor < DEFAULT_FLOOR_COMPLEX and request_text:
        low = request_text.lower()
        hits = [t for t in COMPLEX_TOKENS if t.lower() in low]
        if hits:
            floor = DEFAULT_FLOOR_COMPLEX
            signals.append("complex-token:%s" % hits[0])
    return floor, signals


def cmd_begin(request_text, floor_override=0):
    floor, signals = decide_floor(request_text)
    # ★--floor 상향 전용 오버라이드(v2): 오너/티켓의 "복잡" 선언 반영. 하향은 구조적 불가.
    if floor_override and _int(floor_override, 0) > floor:
        floor = _int(floor_override, 0)
        signals = signals + ["owner-floor:%d" % floor]
    has_override = bool(os.environ.get("GRILL_MARKER", "").strip())
    sid = _surface_id()
    # ★격리 불가 시 fail-open: 마커를 만들지 않아 cross-node 마비를 원천 차단.
    if not sid and not has_override:
        print(json.dumps({"gate": "disabled",
                          "reason": "CYS_SURFACE_ID 부재 — surface 격리 불가, "
                                    "cross-node 마비 방지 위해 미발동(fail-open)",
                          "floor": floor, "signals": signals}, ensure_ascii=False))
        return 0
    # ★v2 상향 시맨틱: collecting(미만료) 마커가 있으면 덮어쓰기 금지 — floor 상향만 병합.
    #   axes·raw_count는 어떤 경로로도 리셋되지 않는다(자동무장 후 모델의 원문 begin이
    #   복잡도를 30으로 정밀 교정하는 경로를 열되, 진행 소실·하향 게이밍은 봉쇄).
    old = _load_marker()
    if old and old.get("status") == "collecting" and not _expired(old):
        old_floor = _int(old.get("floor"), DEFAULT_FLOOR_SIMPLE)
        merged = max(old_floor, floor)
        if merged != old_floor:
            old["floor"] = merged
            old["complexity"] = ("complex" if merged >= DEFAULT_FLOOR_COMPLEX
                                 else "simple")
            old["complexity_signals"] = list(old.get("complexity_signals") or []) + signals
            _save_marker(old)
        print(json.dumps({"session_id": old.get("session_id"), "merged": True,
                          "floor": merged, "distinct": len(old.get("axes", [])),
                          "signals": signals}, ensure_ascii=False))
        return 0
    m = {
        "session_id": _axis_hash(str(time.time()) + (request_text or "")),
        "surface": sid or "(override)",
        "floor": floor,
        "complexity": "complex" if floor >= DEFAULT_FLOOR_COMPLEX else "simple",
        "complexity_signals": signals,
        "started_at": time.time(),
        "ttl_secs": DEFAULT_TTL_SECS,
        "status": "collecting",
        "axes": [],
        "raw_count": 0,
        "request_excerpt": (request_text or "")[:200],
    }
    _save_marker(m)
    print(json.dumps({"session_id": m["session_id"], "floor": floor,
                      "complexity": m["complexity"], "signals": signals},
                     ensure_ascii=False))
    return 0


# ── count: AskUserQuestion 응답 1건 → distinct 최대 1 누적 ─────────────────
def _extract_questions(stdin_data):
    """hook JSON에서 (questions, has_valid_response). 취소·에러 응답은 불인정."""
    if not isinstance(stdin_data, dict):
        return [], False
    ti = stdin_data.get("tool_input")
    qs = ti.get("questions") if isinstance(ti, dict) else None
    if not isinstance(qs, list):
        qs = []
    resp = stdin_data.get("tool_response")
    if isinstance(resp, dict):
        # 사용자가 실제로 답해 결정이 해소됨 — 단 취소/에러/중단은 제외
        has_resp = not (resp.get("error") or resp.get("interrupted")
                        or resp.get("cancelled"))
    else:
        has_resp = resp is not None   # 비-dict 응답(문자열 등)은 존재 시 유효로
    return qs, has_resp


def count_axes(m, questions, has_response):
    """(added, raw). ★호출당 새 distinct는 최대 1개(one-at-a-time 강제·배치 우회 차단)."""
    if not has_response:
        return 0, 0
    raw = len(questions)
    m["raw_count"] = m.get("raw_count", 0) + raw
    existing = [a["norm"] for a in m.get("axes", [])]
    for q in questions:
        if not isinstance(q, dict):
            continue
        label = q.get("header") or q.get("question") or ""
        norm = _normalize_axis(label)
        if not _is_duplicate(norm, existing):
            m.setdefault("axes", []).append(
                {"raw": str(label)[:80], "norm": norm, "hash": _axis_hash(norm)})
            return 1, raw   # 호출당 1개만 인정하고 즉시 종료
    return 0, raw


def cmd_count(stdin_text):
    m = _load_marker()
    if not m or m.get("status") in ("done", "abandoned"):
        return 0   # 활성 세션 없음 → 통과(fail-open)
    try:
        data = json.loads(stdin_text or "{}")
    except ValueError:
        return 0
    questions, has_resp = _extract_questions(data)
    added, raw = count_axes(m, questions, has_resp)
    if added or raw:
        _save_marker(m)
    print(json.dumps({"added": added, "raw_added": raw,
                      "distinct": len(m.get("axes", [])), "floor": m.get("floor")},
                     ensure_ascii=False))
    return 0


# ── check: floor 미충족이면 차단(exit 2) ─────────────────────────────────
def evaluate(m):
    """(blocked, reason, distinct, floor)."""
    if not m:
        return False, "no active grill session (fail-open)", 0, 0
    if _expired(m):
        return False, "grill session expired (fail-open)", 0, 0
    if m.get("status") in PASS_STATES:
        return False, "grill floor satisfied/closed (%s)" % m.get("status"), \
            len(m.get("axes", [])), _int(m.get("floor"), 0)
    distinct = len(m.get("axes", []))
    floor = _int(m.get("floor"), DEFAULT_FLOOR_SIMPLE)
    raw = m.get("raw_count", 0)
    if distinct < floor:
        hint = ""
        if raw and raw / max(distinct, 1) > 1.5:
            hint = (" · 질문 %d개 중 distinct %d개 — 같은 결정을 쪼갠/배치한 질문은 "
                    "카운트되지 않는다(AskUserQuestion으로 서로 다른 결정 축을 1개씩 물어라)"
                    ) % (raw, distinct)
        return True, ("grill 하한 미충족: 서로 다른 결정 브랜치 %d/%d개 해소됨. "
                      "AskUserQuestion으로 미해소 차원(데이터·에러·권한·롤백·동시성·"
                      "범위 밖 인접 시스템)을 더 물어 합의에 이른 뒤 구현하라%s"
                      % (distinct, floor, hint)), distinct, floor
    return False, "grill floor satisfied (%d/%d)" % (distinct, floor), distinct, floor


def _self_cap():
    """B2(D2): grill 자기 블록 예산 상한 — CYS_COMPLETION_GUARD=1 pane 한정·기본 0(off).
    반환 cap(int ≥0) · 0 = 비활성(현행 동작 무변)."""
    if os.environ.get("CYS_COMPLETION_GUARD") != "1":
        return 0
    return max(0, _int(os.environ.get("GRILL_STOP_SELF_CAP"), 0))


def cmd_check():
    m = _load_marker()
    blocked, reason, distinct, floor = evaluate(m)
    if blocked:
        # ── B2 self-cap: 무장 pane 에서 자기 블록 예산 소진 시 자진 통과(+마커 사유 기록).
        #    카운터는 이 마커(사이클) 귀속 — 새 사이클(새 마커)은 0에서 시작(리셋). ──
        cap = _self_cap()
        if cap:
            used = _int(m.get("stop_self_blocks"), 0)
            if used >= cap:
                # G7a: 코얼레싱 — 첫 자진 통과만 행 기록, 이후는 count 증가(반복 Stop 이
                # 마커를 행으로 비대화·중복 감사 행 생성하는 것 방지). 비-list 손상은 새
                # 리스트 치환 가드(파싱 불가 잔재가 기록 자체를 죽이지 않게 — fail-soft).
                yields = m.get("self_cap_yields")
                if not isinstance(yields, list):
                    yields = []
                    m["self_cap_yields"] = yields
                if yields and isinstance(yields[-1], dict):
                    yields[-1]["count"] = _int(yields[-1].get("count"), 1) + 1
                    yields[-1]["last_at"] = time.time()
                else:
                    yields.append(
                        {"at": time.time(), "cap": cap, "blocks": used, "count": 1,
                         "reason": "self-cap 소진 자진 통과 — floor 미충족(%d/%d) 잔존"
                                   "(FLOOR 감사 대상 · 하네스 Stop 지갑 보존 D2 · 자인: "
                                   "grill-gate.sh 공유로 PreToolUse 쓰기 게이트 동반 개방)"
                                   % (distinct, floor)})
                try:
                    _save_marker(m)
                except OSError:
                    pass  # 기록 실패도 통과측(fail-open) — 지갑 보존이 우선
                sys.stderr.write(
                    "grill-gate self-cap: 자기 블록 예산 소진(%d/%d) — 자진 통과(exit 0). "
                    "floor 미충족(%d/%d)은 마커 self_cap_yields 에 감사 기록됨.\n"
                    % (used, cap, distinct, floor))
                return 0
            m["stop_self_blocks"] = used + 1
            try:
                _save_marker(m)
            except OSError:
                pass  # 계상 실패는 차단 자체를 막지 않는다(보수측 — 블록은 유지)
        sys.stderr.write("grill-gate BLOCKED: %s\n" % reason)
        return 2
    if m and m.get("status") == "collecting" and floor > 0 and distinct >= floor:
        m["status"] = "passed"
        m["passed_at"] = time.time()
        _save_marker(m)
    return 0


def cmd_end(force=False):
    m = _load_marker()
    if not m:
        print(json.dumps({"status": "no-session"}, ensure_ascii=False))
        return 0
    distinct = len(m.get("axes", []))
    floor = _int(m.get("floor"), DEFAULT_FLOOR_SIMPLE)
    # ★미충족 end는 거부(BLOCK-2 교정) — producer의 0질문 우회 차단.
    if m.get("status") == "collecting" and distinct < floor:
        if not force:
            # ★--force는 producer에게 노출하지 않는다(라운드2 NEW-2 — 손쉬운 탈출구 차단).
            #   force는 오너 조기중단 전용(argparse help에만 기술). 안내는 '더 질문'만.
            sys.stderr.write(
                "grill end 거부: 하한 미충족(%d/%d) — AskUserQuestion으로 미해소 "
                "결정축을 더 물어 floor를 채운 뒤 종료하라.\n" % (distinct, floor))
            return 2
        # 오너 강제 종료 — 통과시키되 흔적을 남겨 사후 감사(우회 탐지)
        m["status"] = "abandoned"
        m["end_forced_unmet"] = True
    else:
        m["status"] = "done"
    m["ended_at"] = time.time()
    _save_marker(m)
    print(json.dumps({"status": m["status"], "distinct": distinct, "floor": floor},
                     ensure_ascii=False))
    return 0


def cmd_status():
    m = _load_marker()
    if not m:
        print(json.dumps({"active": False}, ensure_ascii=False))
        return 0
    blocked, reason, distinct, floor = evaluate(m)
    print(json.dumps({
        "active": m.get("status") not in ("done", "abandoned"),
        "status": m.get("status"), "distinct": distinct, "floor": floor,
        "raw_count": m.get("raw_count", 0), "blocked_now": blocked,
        "expired": _expired(m), "signals": m.get("complexity_signals"),
    }, ensure_ascii=False))
    return 0


# ── self-test (외부 의존 0) ───────────────────────────────────────────────
def self_test():
    import tempfile
    fails = []
    td = tempfile.mkdtemp(prefix="grill-gate-test-")
    os.environ["GRILL_MARKER"] = os.path.join(td, ".grill_session.json")
    # B2 self-cap 밀폐: ambient 무장 env 가 기존 케이스의 차단 산술을 오염시키지 않게 핀.
    os.environ.pop("CYS_COMPLETION_GUARD", None)
    os.environ.pop("GRILL_STOP_SELF_CAP", None)

    def fresh(floor):
        _save_marker({"session_id": "t", "floor": floor, "status": "collecting",
                      "started_at": time.time(), "ttl_secs": DEFAULT_TTL_SECS,
                      "axes": [], "raw_count": 0})

    def one(header):  # 단일 질문 1회 호출(정상 one-at-a-time)
        return json.dumps({"tool_input": {"questions": [{"header": header}]},
                           "tool_response": {"ok": True}})

    def feed(headers):  # 각 header를 개별 호출(정상 인터뷰)
        for h in headers:
            cmd_count(one(h))

    def distinct():
        return len(_load_marker().get("axes", []))

    try:
        # ① 마커 부재 → fail-open
        if os.path.exists(_marker_path()):
            _save_marker({"status": "done"})   # rm 대신 done으로 무력화
        m0 = _load_marker()
        if m0 and m0.get("status") != "done":
            fails.append("setup")
        # 진짜 부재 테스트: 별도 경로
        os.environ["GRILL_MARKER"] = os.path.join(td, "absent.json")
        if cmd_check() != 0:
            fails.append("①마커 부재인데 차단됨(fail-open 위반)")
        os.environ["GRILL_MARKER"] = os.path.join(td, ".grill_session.json")

        # ② floor 미달(3 distinct) → 차단
        fresh(20)
        feed(["Auth", "Rollback", "Schema"])
        if distinct() != 3:
            fails.append("②distinct 3 기대인데 %d" % distinct())
        if cmd_check() != 2:
            fails.append("②floor 20 미달인데 차단 안 됨")

        # ③ floor 충족(5 distinct, 각 1회) → 통과 + passed
        fresh(5)
        feed(["Auth", "Rollback", "Schema", "Concurrency", "Permissions"])
        if distinct() != 5:
            fails.append("③5 distinct 기대인데 %d" % distinct())
        if cmd_check() != 0:
            fails.append("③floor 5 충족인데 차단됨")
        if _load_marker().get("status") != "passed":
            fails.append("③충족 후 status=passed 아님")

        # ④ ★배치 우회 — 1호출에 25 header → distinct 1(one-at-a-time)
        fresh(20)
        cmd_count(json.dumps({"tool_input": {"questions": [
            {"header": "Axis%d" % i} for i in range(25)]},
            "tool_response": {"ok": True}}))
        if distinct() != 1:
            fails.append("④배치 25개가 distinct %d로 샜다(1 기대·one-at-a-time)" % distinct())

        # ⑤ 쪼개기 변형 → distinct ≤2
        fresh(20)
        feed(["Auth", "Auth1", "Auth1b", "Authn"])
        if distinct() > 2:
            fails.append("⑤쪼개기 변형이 distinct %d로 샘" % distinct())

        # ⑥ 무의미 라벨 → 0
        fresh(20)
        feed(["A1", "B2", "C3"])
        if distinct() != 0:
            fails.append("⑥무의미 라벨이 카운트됨(%d)" % distinct())

        # ⑦ ★한글 2~3자 축 5개(각 1회) → distinct 5(over-merge 방지)
        fresh(20)
        feed(["인증", "권한", "롤백", "스키마", "동시성"])
        if distinct() != 5:
            fails.append("⑦한글 짧은 축 5개가 distinct %d로 붕괴(5 기대)" % distinct())

        # ⑦b ★4자 한글 근접동의어 병합(라운드2 REVISE-1: len>=4 경계)
        fresh(20)
        feed(["인증방식", "인증방법"])
        if distinct() != 1:
            fails.append("⑦b 4자 한글 동의어가 distinct %d로 분리(1 기대)" % distinct())

        # ⑧ ★취소/에러 응답 → 불인정
        fresh(20)
        cmd_count(json.dumps({"tool_input": {"questions": [{"header": "Auth"}]},
                              "tool_response": {"error": "User cancelled",
                                                "interrupted": True}}))
        if distinct() != 0:
            fails.append("⑧취소 응답이 카운트됨")

        # ⑨ 무응답(tool_response 부재) → 불인정
        fresh(20)
        cmd_count(json.dumps({"tool_input": {"questions": [{"header": "Auth"}]}}))
        if distinct() != 0:
            fails.append("⑨무응답 질문이 카운트됨")

        # ⑩ ★end 미충족 거부(exit 2) / end --force 통과(abandoned)
        fresh(20)
        feed(["Auth", "Rollback"])
        if cmd_end(force=False) != 2:
            fails.append("⑩미충족 end가 거부되지 않음(우회)")
        if _load_marker().get("status") != "collecting":
            fails.append("⑩거부된 end가 status를 바꿈")
        if cmd_end(force=True) != 0:
            fails.append("⑩end --force가 실패")
        if _load_marker().get("status") != "abandoned":
            fails.append("⑩force end가 abandoned로 기록 안 됨")
        if cmd_check() != 0:
            fails.append("⑩abandoned 후 통과 안 됨(fail-open)")

        # ⑪ 충족 후 end → done
        fresh(2)
        feed(["Auth", "Rollback"])
        if cmd_end() != 0 or _load_marker().get("status") != "done":
            fails.append("⑪충족 end가 done 아님")

        # ⑫ TTL 만료 → fail-open
        fresh(20)
        m = _load_marker(); m["started_at"] = time.time() - (DEFAULT_TTL_SECS + 100)
        _save_marker(m)
        if cmd_check() != 0:
            fails.append("⑫만료 세션이 차단됨")

        # ⑬ ★floor null/비수치 → 크래시 없이 기본값
        _save_marker({"status": "collecting", "floor": None, "axes": [],
                      "started_at": time.time(), "ttl_secs": DEFAULT_TTL_SECS})
        try:
            rc = cmd_check()
            if rc != 2:
                fails.append("⑬floor null인데 차단 안 됨(기본 20 적용 실패)")
        except Exception as e:
            fails.append("⑬floor null에서 크래시: %s" % e)

        # ⑭ decide_floor — fast=20·slow=30·복잡토큰=30
        f1, _ = decide_floor("간단한 오타 한 글자 수정")
        f2, _ = decide_floor("박사급으로 전체 시스템을 워커 위임해 깊이 분석해줘")
        f3, _ = decide_floor("결제 모듈을 비가역적으로 migration 한다")
        if f1 != 20:
            fails.append("⑭fast floor != 20 (%d)" % f1)
        if f2 != 30:
            fails.append("⑭slow floor != 30 (%d)" % f2)
        if f3 != 30:
            fails.append("⑭복잡토큰(비가역) floor != 30 (%d)" % f3)
        # ⑭b ★COMPLEX_TOKENS 오탐 회귀(라운드2 NEW-3): 일반어 포함 trivial은 20 유지
        f4, _ = decide_floor("워커 이름 오타 한 글자 수정")
        f5, _ = decide_floor("README의 migration 철자 교정")
        if f4 != 20:
            fails.append("⑭b COMPLEX 오탐: '워커' 포함 trivial이 floor %d(20 기대)" % f4)
        if f5 != 20:
            fails.append("⑭b COMPLEX 오탐: 'migration' 단독이 floor %d(20 기대)" % f5)

        # ⑯ ★v2 begin 상향 시맨틱: collecting 마커에 재begin — 진행 보존·상향만
        fresh(20)
        _save_marker(dict(_load_marker(), axes=["a1", "a2"], raw_count=2))
        cmd_begin("멀티 서브시스템 아키텍처 재설계 전수조사")   # 복잡 → 30 상향
        m16 = _load_marker()
        if _int(m16.get("floor"), 0) != 30:
            fails.append("⑯재begin 상향 실패 floor=%s(30 기대)" % m16.get("floor"))
        if len(m16.get("axes", [])) != 2 or m16.get("raw_count") != 2:
            fails.append("⑯재begin이 진행(axes/raw)을 리셋함")
        cmd_begin("오타 수정")   # 단순 → 하향 시도: 30 유지되어야
        if _int(_load_marker().get("floor"), 0) != 30:
            fails.append("⑯하향 게이밍 차단 실패(30→%s)" % _load_marker().get("floor"))
        # ⑯b --floor 상향 전용 오버라이드
        fresh(20)
        cmd_begin("간단 작업", floor_override=30)
        if _int(_load_marker().get("floor"), 0) != 30:
            fails.append("⑯b --floor 상향 미반영")
        fresh(30)
        cmd_begin("간단 작업", floor_override=20)   # 하향 시도
        if _int(_load_marker().get("floor"), 0) != 30:
            fails.append("⑯b --floor 하향이 먹힘(게이밍 경로)")

        # ⑰ ★B2 self-cap: 기본(0)=현행 무변 → cap 2 무장 시 3번째 블록 자진 통과+사유 기록
        #    → 새 사이클(새 마커) 리셋 → 비무장(CYS_COMPLETION_GUARD≠1)은 cap 이 있어도 무변
        fresh(20)
        feed(["Auth", "Rollback"])   # distinct 2 < 20 — 차단 상태 조성
        # (a) 기본 0 = off: 무장돼 있어도 현행 동작 무변(반복 차단·카운터 미계상)
        os.environ["CYS_COMPLETION_GUARD"] = "1"
        for _i in range(3):
            if cmd_check() != 2:
                fails.append("⑰a cap 기본 0 인데 차단이 풀림(현행 동작 변경)")
                break
        if "stop_self_blocks" in (_load_marker() or {}):
            fails.append("⑰a cap off 인데 카운터가 계상됨")
        # (b) cap=2: 1·2번째 블록(exit 2) → 3번째 자진 통과(exit 0)+마커 사유 기록·status 불변
        os.environ["GRILL_STOP_SELF_CAP"] = "2"
        if cmd_check() != 2 or cmd_check() != 2:
            fails.append("⑰b cap 2 의 1·2번째가 차단(exit 2) 아님")
        if cmd_check() != 0:
            fails.append("⑰b 3번째 블록이 자진 통과(exit 0) 아님")
        m17 = _load_marker()
        yields = m17.get("self_cap_yields") or []
        if len(yields) != 1 or "floor 미충족" not in yields[0].get("reason", ""):
            fails.append("⑰b 자진 통과 사유 기록 부재/오기록: %s" % yields)
        if "PreToolUse 쓰기 게이트 동반 개방" not in yields[0].get("reason", ""):
            fails.append("⑰b G7a 사유 문면에 PreToolUse 동반 개방 자인 누락: %s" % yields)
        if m17.get("status") != "collecting":
            fails.append("⑰b 자진 통과가 status 를 변조(충족 위장): %s" % m17.get("status"))
        if _int(m17.get("stop_self_blocks"), 0) != 2:
            fails.append("⑰b 카운터 오계상: %s" % m17.get("stop_self_blocks"))
        # (b2) G7a 코얼레싱: 4·5번째 자진 통과는 행 추가 없이 count 증가(1→3)
        if cmd_check() != 0 or cmd_check() != 0:
            fails.append("⑰b2 반복 자진 통과가 exit 0 아님")
        y2 = (_load_marker() or {}).get("self_cap_yields") or []
        if len(y2) != 1 or _int(y2[-1].get("count"), 0) != 3:
            fails.append("⑰b2 코얼레싱 실패(행 %d·count %s — 기대 1행·count 3)"
                         % (len(y2), y2 and y2[-1].get("count")))
        # (b3) G7a 비-list 손상 가드: self_cap_yields 가 str 손상이어도 새 리스트 치환·무크래시
        m_corrupt = _load_marker()
        m_corrupt["self_cap_yields"] = "corrupt"
        _save_marker(m_corrupt)
        if cmd_check() != 0:
            fails.append("⑰b3 손상 마커에서 자진 통과 실패")
        y3 = (_load_marker() or {}).get("self_cap_yields")
        if not isinstance(y3, list) or len(y3) != 1 or _int(y3[-1].get("count"), 0) != 1:
            fails.append("⑰b3 비-list 손상 치환 가드 실패: %r" % (y3,))
        # (c) 새 사이클(새 마커) = 리셋: 다시 cap 까지 차단이 살아난다
        fresh(20)
        feed(["Auth", "Rollback"])
        if cmd_check() != 2:
            fails.append("⑰c 새 사이클에서 카운터 리셋 실패(즉시 통과됨)")
        # (d) 비무장 pane: cap 이 있어도 무변(한정 발동)
        os.environ.pop("CYS_COMPLETION_GUARD", None)
        fresh(20)
        feed(["Auth", "Rollback"])
        for _i in range(4):
            if cmd_check() != 2:
                fails.append("⑰d 비무장 pane 인데 self-cap 이 발동(한정 위반)")
                break
        os.environ.pop("GRILL_STOP_SELF_CAP", None)

        # ⑮ ★surface 부재 시 begin fail-open(마커 미생성)
        os.environ.pop("GRILL_MARKER", None)
        os.environ.pop("CYS_SURFACE_ID", None)
        os.environ["CYS_ROOT"] = os.path.join(td, "noiso")
        cmd_begin("작업")   # sid 없음·override 없음
        mk = os.path.join(td, "noiso", "_round", ".grill_session.json")
        if os.path.exists(mk):
            fails.append("⑮surface 부재인데 마커 생성됨(cross-node 마비 위험)")
        os.environ.pop("CYS_ROOT", None)
    finally:
        import shutil
        shutil.rmtree(td, ignore_errors=True)
        os.environ.pop("GRILL_MARKER", None)
        os.environ.pop("CYS_ROOT", None)
        os.environ.pop("CYS_COMPLETION_GUARD", None)
        os.environ.pop("GRILL_STOP_SELF_CAP", None)

    if fails:
        sys.stderr.write("\n".join(fails) + "\n")
        sys.stderr.write("grill_gate self-test: %d 실패\n" % len(fails))
        return 1
    print(json.dumps({"self_test": "ok", "cases": 18,
                      "covers": "fail-open·차단·충족·배치우회·쪼개기·무의미·한글축·"
                                "취소·무응답·end거부/force·done·만료·floor-null·"
                                "floor20/30/복잡토큰·surface부재·"
                                "v2상향병합/진행보존/하향거부/floor오버라이드·"
                                "B2 self-cap(기본0 무변·cap2 3번째 자진통과+사유기록·"
                                "status 불변·새사이클 리셋·비무장 무변·"
                                "G7a 코얼레싱 count·비-list 손상 치환·PreToolUse 자인 문면)"},
                     ensure_ascii=False))
    return 0


def main():
    ap = argparse.ArgumentParser(description="grill-me 최소 질문 결정론 게이트")
    ap.add_argument("cmd", nargs="?",
                    choices=["begin", "count", "check", "end", "status"])
    ap.add_argument("--request", default="", help="begin: 원 작업 지시(복잡도 판정용)")
    ap.add_argument("--floor", type=int, default=0,
                    help="begin: 상향 전용 floor 오버라이드(오너/티켓 '복잡' 선언 — 하향 불가)")
    ap.add_argument("--force", action="store_true",
                    help="end: 오너 조기중단(미충족이어도 종료·abandoned 기록)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        sys.exit(self_test())
    if args.cmd == "begin":
        sys.exit(cmd_begin(args.request, floor_override=args.floor))
    if args.cmd == "count":
        sys.exit(cmd_count(sys.stdin.read()))
    if args.cmd == "check":
        sys.exit(cmd_check())
    if args.cmd == "end":
        sys.exit(cmd_end(force=args.force))
    if args.cmd == "status":
        sys.exit(cmd_status())
    ap.print_help()
    sys.exit(0)


if __name__ == "__main__":
    main()
