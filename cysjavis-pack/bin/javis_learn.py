#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_learn — RSI 학습 루프(5단계)의 결정론 엔진 (directive RSI_LEARNING §1·§5 · DESIGN §4).

오너 5단계(①검색·탐색 ②패턴·철학 추출 ③객관·근거 평가 ④문서·지침 저장 ⑤skill/harness
제작·발전)를 결정론으로 박제한다. ★할루시네이션 원천 봉쇄(제품 절대명제): 환각 자료가
학습에 침투하면 재귀 증폭으로 전 시스템이 붕괴하므로 입구를 전면 차단한다(부분 통과 = 전체 중단).

이 도구는 **계약 강제·검증·위임자**다 — 점수를 자체 생성하지 않고(③→javis_rsi 위임),
기억을 직접 쓰지 않으며(④→javis_memory 위임), 실제 WebSearch는 에이전트가 수행하고 이 도구는
그 산출(candidates/pattern JSON)의 계약(citation·스키마·정박)을 결정론으로 검증한다(네트워크·LLM
호출 없음). 의미·논리의 독립 모델 검증과 5차원 봉쇄 집행은 rsi-gate.sh가 담당한다.

명령(DESIGN §4 계약 + LEARN_GAPS_CONTRACT C1~C11):
  propose  --reason <stuck|gate|ceiling> --topic <S> [--json]
      트리거 신호 → 학습 후보·근거 payload 산출(승인 요청용). 승인 전 검색·저장·채택 무실행.
  search   --topic <S> --candidates <path|-> [--json]
      ① 후보 JSON 검증 게이트. source_url·claim·retrieved_at 필수·citation 0이면 hard fail.
      ★C7 v2: 후보가 v2 필드(first_seen·adoption_evidence·known_failures·counterquery_log)를
      하나라도 선언하면 전 v2 필드 강제(known_failures 각 항목 source_url+snapshot_sha256+summary).
      normalized는 전 필드 보존. 구(v1) 후보=관용(후방 호환).
  extract  --from <candidates.json|-> --pattern <pattern.json|-> [--json]
      ② pattern 스키마 검증 + evidence_ref가 후보 출처에 정박했는지 대조. 미충족 거부.
  evaluate --round <id> --score F [--baseline] [--note S] [--evaluator-manifest <path>]
           [--owner-approved] [--responds-to <ref>] [--json]
      ③ javis_rsi에 위임(첫 회=checkpoint·이후=progress). ★score는 주입만(자체생성 금지).
      C5: --evaluator-manifest=components 실측 sha256 트리 해시(부재=exit 6·자기신고 금지).
      C6: freeze 레짐 활성 시 신규 라운드는 freeze 존재+해시 무결 요구(exit 8·구 라운드 면제).
      C10: 라운드별 시도 수 ledger 계수 — 4회째=exit 9(ESCALATE)·--owner-approved로만 속행,
      속행(재제출)은 --responds-to <직전 REVISE evidence 참조> 필수(무응답 재제출 거부).
  store    --round <id> --pattern <pattern.json|-> --type <feedback|reference|project>
           [--approved] [--state <provisional|confirmed>] [--fallback] [--refs <경로,…>]
           [--name S] [--desc S] [--json]
      ④ verdict=improved AND --approved일 때만 javis_memory add 위임. fallback 모드 confirmed 차단.
      C1: 항목 레코드 v2(state/expires(prov+90d·conf+180d)/review_due/reval_count/refs/
      effect_log/challenge). C3: --refs 각 경로 마커 실존 검증(부재=exit 3). C8: pattern이
      v2 필드를 선언하면 behavioral_claim·falsifier·maturity 전부 필수(v1=관용).
      C9: store 전 conflictscan 자동 — confirmed는 (v2 pattern 또는 충돌 후보>0) 시
      gate-input 번들 conflict_audit verdict 필수.
  harness  --round <id> --pattern <pattern.json|-> [--evolve <skill>] [--refs <경로,…>] [--json]
      ⑤ retention. 라운드 verdict=regressed면 javis_rsi rollback(dry-run) 권고. C1·C3·C8 동일.
  challenge --id <id> --evidence <path|-> [--resolve <upheld|rejected>] [--json]
      C4: 탄핵 — 증거 번들 프리스크린(부재/형식위반=exit 4)·동일 id 쿨다운 14d(exit 5)·
      challenged 중 효력 유지·upheld=tombstone(soft)+refs 스윕 출력.
  freeze   --round <id> --benchmarks <path|-> --proposer <role> [--auditor-sig <sig>] [--json]
      C6: benchmark 사전 등록 ledger(_round/learn/benchmarks/<round>.json·content_sha256).
      proposer=master면 auditor_sig 필수(부재=exit 7).
  conflictscan --pattern <pattern.json|-> [--round <id>] [--name <id>] [--json]
      C9: 코퍼스(pack directives 읽기전용·_round/*.md·skills SKILL.md·MEMORY.md+confirmed
      memory 본문) 키워드 대조 → conflict_candidates. 0건=시드 기록 20% sample_audit 플래그.
      한계 명문: 후보 추출기이지 증명기 아님.
  audit    [--json] [--mark-revaled <id> [--full]]
      C11: ①만기 스캔(prov 만기=tombstone·conf 만기=wakeup enqueue — reval_count가 2의
      배수인 회차는 --reason ttl-expired-full-recheck로 5차원 full recheck 의무 결정론 전달,
      G1 래칫 봉쇄) ②lapse(conf 만기+30d=provisional 자동 강등) ③refs 양방향 대조
      ④effect_log none 2연속 보고 ⑤C_n vs C_0 체인 대조("직전 승·최초 대비 하락"=hard-fail).
      hard-fail 존재=exit 1. --mark-revaled=reval 완료 기록(reval_count+1·expires +180d 재계산
      — full 의무 회차에 --full 부재=exit 2·경량 연장으로 의무 회피 불가).
  status   [--json] [--evaluator-hash <h>]
      UI 데이터원 — 라운드·verdict·채택/rollback·발견 누적. C5: --evaluator-hash=해당 해시가
      채점한 라운드 목록(폭발 반경).

종료 코드: 0 성공 · 1 audit hard-fail 존재 · 2 인자/계약 위반(hard fail) · 3 위임 도구 실패
  또는 refs 마커 부재(C3) · 4 challenge 프리스크린 거부 · 5 challenge 쿨다운 · 6 evaluator
  manifest component 부재 · 7 master freeze 무서명 · 8 freeze 부재/해시 무결 위반 ·
  9 evaluate 시도 상한 ESCALATE.
의존성: 파이썬 표준 라이브러리 + 같은 bin의 javis_rsi.py·javis_memory.py·javis_wakeup.py.
네트워크·LLM 호출 없음.

상태 파일: 이 도구의 진실은 `_round/learn/learn_state.json`(사설)이다 — javis_rsi의
_mirror_learn_state가 `_round/learn/state.json`을 rsi 스키마로 덮어쓰는 경로 충돌(stored/
harness 소실 실측) 때문에 분리했다. `state.json`은 데몬 가독 미러로 best-effort 병행 기록,
로드는 사설 우선·부재 시 legacy state.json 관용 폴백(후방 호환).
"""
import argparse
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from datetime import date, timedelta
from urllib.parse import urlparse

HERE = os.path.dirname(os.path.abspath(__file__))
RSI = os.path.join(HERE, "javis_rsi.py")
MEM = os.path.join(HERE, "javis_memory.py")
GATE = os.path.join(HERE, "rsi-gate.sh")
WAKEUP = os.path.join(HERE, "javis_wakeup.py")

PATTERN_FIELDS = ("domain", "condition", "action", "rationale", "evidence_ref")
VALID_REASONS = ("stuck", "gate", "ceiling")
VALID_TYPES = ("feedback", "reference", "project")

# C7 후보 v2 필드(하나라도 선언=전부 강제) · C8 pattern v2 필드(동일 규칙)
CANDIDATE_V2_FIELDS = ("first_seen", "adoption_evidence", "known_failures", "counterquery_log")
PATTERN_V2_FIELDS = ("behavioral_claim", "falsifier", "maturity")
MATURITY_KEYS = ("first_seen", "adoption_evidence", "known_failures")

# C1 TTL — store 시 자동 계산(구 레코드 부재=관용)
TTL_DAYS = {"provisional": 90, "confirmed": 180}
CHALLENGE_COOLDOWN_DAYS = 14   # C4
LAPSE_GRACE_DAYS = 30          # C11 lapse rule
EVALUATE_ATTEMPT_CAP = 3       # C10: 4회째(기존 3회 초과)=ESCALATE

SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")


def learn_dir():
    root = os.environ.get("CYS_ROUND_DIR")
    if root:
        return os.path.join(root, "learn")
    return os.path.join(os.getcwd(), "_round", "learn")


def fail(code, msg):
    print(f"error: {msg}", file=sys.stderr)
    return code


def _read_json_arg(val):
    """'-'=stdin · 그 외=파일 경로. 반환: 파싱된 객체 (실패 시 ValueError)."""
    if val == "-":
        return json.loads(sys.stdin.read())
    with open(val, encoding="utf-8") as f:
        return json.load(f)


# ───────────────────────── 순수 로직(테스트 핀) ─────────────────────────

def domain_of(url):
    try:
        return (urlparse(url).netloc or "").lower()
    except ValueError:
        return ""


def slugify(s, prefix="rsi-"):
    """javis_memory의 kebab-case 슬러그([a-z0-9-]·영숫자 시작) 계약에 맞는 이름 생성."""
    base = "".join(ch if (ch.isascii() and ch.isalnum()) else "-" for ch in str(s).lower())
    base = "-".join(filter(None, base.split("-")))
    out = (prefix + base)[:48].strip("-")
    return out or (prefix.strip("-") or "rsi")


def _valid_known_failures(kf):
    """C7: known_failures 각 항목 {source_url, snapshot_sha256, summary} — 해시 없는 비판=거부.
    반환: 오류 문자열 목록(빈 목록=통과). 빈 배열은 허용('역질의 후 실패 사례 미발견' 기록)."""
    if not isinstance(kf, list):
        return ["known_failures가 배열 아님"]
    errs = []
    for j, item in enumerate(kf):
        if not isinstance(item, dict):
            errs.append(f"known_failures[{j}] 객체 아님")
            continue
        u = str(item.get("source_url", "")).strip()
        if not (u.startswith("http://") or u.startswith("https://")):
            errs.append(f"known_failures[{j}].source_url 없음/비URL")
        if not SHA256_RE.match(str(item.get("snapshot_sha256", ""))):
            errs.append(f"known_failures[{j}].snapshot_sha256 없음/64hex 아님(해시 없는 비판=지어낸 비판 봉쇄)")
        if not str(item.get("summary", "")).strip():
            errs.append(f"known_failures[{j}].summary 비어있음")
    return errs


def is_v2_candidate(c):
    """C7: v2 필드 하나라도 선언하면 v2(전 필드 강제). 선언 0=v1 관용(후방 호환)."""
    return isinstance(c, dict) and any(k in c for k in CANDIDATE_V2_FIELDS)


def validate_candidates(cands):
    """① 후보 검증(순수). citation 필수·필드 정박. 반환 {ok, errors, normalized, distinct_sources}.
    C7: v2 후보(v2 필드 선언)는 first_seen·adoption_evidence·known_failures(각 항목 해시)·
    counterquery_log(역질의 기록 1개+)·canonical 명시까지 전부 필수. normalized는 전 필드 보존."""
    if not isinstance(cands, list) or not cands:
        return {"ok": False, "errors": ["후보 0건 — 학습지식 단독 금지(citation 필수·hard fail)"],
                "normalized": [], "distinct_sources": 0}
    require_v2 = any(is_v2_candidate(c) for c in cands)
    errors, norm, domains = [], [], set()
    for i, c in enumerate(cands):
        if not isinstance(c, dict):
            errors.append(f"[{i}] 객체 아님")
            continue
        url = str(c.get("source_url", "")).strip()
        claim = str(c.get("claim", "")).strip()
        ra = str(c.get("retrieved_at", "")).strip()
        if not (url.startswith("http://") or url.startswith("https://")):
            errors.append(f"[{i}] source_url 없음/비URL")
        if not claim:
            errors.append(f"[{i}] claim 비어있음")
        if not ra:
            errors.append(f"[{i}] retrieved_at 없음(실호출 로그 정박 필요)")
        if require_v2:
            if "canonical" not in c:
                errors.append(f"[{i}] canonical 명시 없음(v2 필수)")
            if not str(c.get("first_seen", "")).strip():
                errors.append(f"[{i}] first_seen 없음(v2 필수 — 성숙도 정박)")
            if not str(c.get("adoption_evidence", "")).strip():
                errors.append(f"[{i}] adoption_evidence 없음(v2 필수)")
            if "known_failures" not in c:
                errors.append(f"[{i}] known_failures 없음(v2 필수)")
            else:
                errors.extend(f"[{i}] {e}" for e in _valid_known_failures(c.get("known_failures")))
            cq = c.get("counterquery_log")
            if not (isinstance(cq, list) and len(cq) >= 1 and all(str(q).strip() for q in cq)):
                errors.append(f"[{i}] counterquery_log 없음/빈 항목(역질의 실행 기록 1개+ 필수)")
        if url:
            domains.add(domain_of(url))
        # ★C7 정규화 보존: 전 필드 보존(기존 4키만 보존하던 소거 함정 수리) — 필수 키는 정규화 덮어쓰기.
        entry = dict(c)
        entry.update({"source_url": url, "claim": claim, "retrieved_at": ra,
                      "canonical": bool(c.get("canonical", False))})
        norm.append(entry)
    return {"ok": not errors, "errors": errors, "normalized": norm,
            "distinct_sources": len(domains)}


def validate_pattern(pattern, candidate_urls=None):
    """② pattern 스키마 + evidence_ref 정박 검증(순수). 반환 {ok, errors}."""
    if not isinstance(pattern, dict):
        return {"ok": False, "errors": ["pattern 객체 아님"]}
    errors = [f"필드 '{f}' 비어있음" for f in PATTERN_FIELDS if not str(pattern.get(f, "")).strip()]
    ev = str(pattern.get("evidence_ref", "")).strip()
    if ev and candidate_urls is not None and ev not in set(candidate_urls):
        errors.append(f"evidence_ref가 후보 출처에 정박 안 됨: {ev}")
    return {"ok": not errors, "errors": errors}


def is_v2_pattern(pattern):
    """C8: v2 필드(behavioral_claim·falsifier·maturity) 하나라도 선언=v2. v1 로드=관용."""
    return isinstance(pattern, dict) and any(k in pattern for k in PATTERN_V2_FIELDS)


def validate_pattern_v2(pattern):
    """C8 pattern v2 검증(순수) — v2 선언 시 behavioral_claim(관찰 가능 행동)·falsifier(반증
    관측 조건)·maturity(C7 3필드 요약: first_seen·adoption_evidence·known_failures) 전부 필수.
    v1(무선언)=통과(관용). 반환 {ok, errors, v2}."""
    if not is_v2_pattern(pattern):
        return {"ok": True, "errors": [], "v2": False}
    errors = []
    if not str(pattern.get("behavioral_claim", "")).strip():
        errors.append("behavioral_claim 비어있음(v2 필수 — 관찰 가능 행동 서술)")
    if not str(pattern.get("falsifier", "")).strip():
        errors.append("falsifier 비어있음(v2 필수 — 반증 관측 조건)")
    mat = pattern.get("maturity")
    if not isinstance(mat, dict):
        errors.append("maturity 없음/객체 아님(v2 필수 — C7 3필드 요약)")
    else:
        for k in ("first_seen", "adoption_evidence"):
            if not str(mat.get(k, "")).strip():
                errors.append(f"maturity.{k} 비어있음")
        if "known_failures" not in mat:
            errors.append("maturity.known_failures 없음")
        else:
            errors.extend(f"maturity: {e}" for e in _valid_known_failures(mat.get("known_failures")))
    return {"ok": not errors, "errors": errors, "v2": True}


# ── C3 역참조 마커(타입별 문법) ──

def marker_present(path, item_id):
    """C3: 경로에 learn:<id> 마커 실존 검증(순수 파일 검사). md/html=<!-- learn:<id> --> ·
    json=최상위 "_learn_refs" 배열 · 그 외(py/sh/# 주석 언어)=# learn:<id>. 부재/읽기실패=False."""
    ext = os.path.splitext(str(path))[1].lower()
    try:
        if ext == ".json":
            obj = json.load(open(path, encoding="utf-8"))
            refs = obj.get("_learn_refs") if isinstance(obj, dict) else None
            return isinstance(refs, list) and item_id in refs
        text = open(path, encoding="utf-8", errors="replace").read()
    except (OSError, ValueError):
        return False
    if ext in (".md", ".html", ".htm"):
        return f"<!-- learn:{item_id} -->" in text
    return f"# learn:{item_id}" in text


def missing_markers(refs, item_id):
    """refs 경로 목록 중 마커 부재 경로 반환(빈 목록=전부 실존)."""
    return [p for p in refs if not marker_present(p, item_id)]


def _parse_refs(refs_arg):
    return [p.strip() for p in str(refs_arg or "").split(",") if p.strip()]


# ── C1 TTL·날짜 ──

def _today():
    return date.today()


def _iso_after(base, days):
    return (base + timedelta(days=days)).isoformat()


def _parse_iso(s):
    try:
        return date.fromisoformat(str(s))
    except (TypeError, ValueError):
        return None


def new_item_fields(state_name, refs, today=None):
    """C1: 신규 store/harness 항목 레코드 v2 필드(순수). prov=+90d·conf=+180d."""
    t = today or _today()
    exp = _iso_after(t, TTL_DAYS.get(state_name, TTL_DAYS["provisional"]))
    return {"state": state_name, "expires": exp, "review_due": exp, "reval_count": 0,
            "refs": list(refs), "effect_log": [], "challenge": None}


# ── C5 evaluator 매니페스트 트리 해시 ──

def evaluator_tree_hash(components):
    """components 각 파일 실측 sha256 → 정렬 연접 → 트리 해시(순수). 파일 부재=(None, 경로)."""
    hashes = []
    for p in components:
        try:
            hashes.append(hashlib.sha256(open(p, "rb").read()).hexdigest())
        except OSError:
            return None, p
    return hashlib.sha256("".join(sorted(hashes)).encode("ascii")).hexdigest(), None


# ── C2 체크포인트 페이로드 v2 ──

def checkpoint_payload(state, rid):
    """기존 5키(round·verdict·stored·harness·discovery) 유지 + v2 신규 키
    items(C1 항목 상태 배열)·evaluator_hash(C5)·schema:"v2". 라운드 부재=None."""
    r = state.get("rounds", {}).get(rid)
    if not r:
        return None
    items = [{"name": s.get("name"), "type": s.get("type"),
              "state": s.get("state"), "expires": s.get("expires")}
             for s in r.get("stored", [])]
    items += [{"name": h.get("harness_ref"), "type": "harness",
               "state": h.get("state"), "expires": h.get("expires")}
              for h in r.get("harness", [])]
    payload = {"round": rid, "verdict": r.get("verdict"),
               "stored": r.get("stored", []), "harness": r.get("harness", []),
               "discovery": state.get("discovery"),
               "items": items, "schema": "v2"}
    # ★P2-2 — evaluator_hash가 None이면 키 생략(Rust 병합이 기존 값을 Null로 덮어쓰는 것 방지).
    if r.get("evaluator_hash"):
        payload["evaluator_hash"] = r["evaluator_hash"]
    return payload


def confidence_of(distinct_sources):
    """독립 출처 수 → confidence. 2개 미만=low(단일 출처 confirmed 불가)."""
    return "low" if distinct_sources < 2 else "med"


def promotion_allowed(verdict, approved, fallback_mode, state):
    """④ 저장 승격 가부(순수). 반환 (allowed, reason)."""
    if state == "confirmed" and fallback_mode:
        return False, "fallback 모드(단일 모델 변형·공통모드 방어 약화)는 confirmed 승격 불가 — provisional만(codex R3)"
    if verdict != "improved":
        return False, f"verdict={verdict} — improved 아니면 저장 거부(측정 우위 없음)"
    if not approved:
        return False, "사람 승인(--approved) 없음 — ④저장·⑤채택은 사람 승인(directive §4)"
    return True, "ok"


# ───────────────────────── 상태 파일 I/O ─────────────────────────
# ★수리: javis_rsi._mirror_learn_state가 _round/learn/state.json을 rsi 스키마 rounds로
# 덮어써 javis_learn의 stored/harness가 소실되는 경로 충돌 실측(learn_e2e 5건 FAIL 재현).
# javis_learn의 진실은 사설 learn_state.json — 로드는 사설 우선·legacy state.json 관용 폴백,
# 저장은 사설 + state.json 미러(best-effort·데몬 가독 표면 유지).

def _state_path():
    return os.path.join(learn_dir(), "learn_state.json")


def _is_canonical():
    """★P0-2: learn_dir()이 데몬 canonical(~/.cys/state/learn)과 동일하면 canonical 모드.
    이 모드에선 state.json이 데몬 단일 writer 소유 — 미러 쓰기 금지·로드는 데몬 라운드 병합."""
    root = os.environ.get("CYS_ROUND_DIR")
    if not root:
        return False
    try:
        canon = os.path.realpath(os.path.expanduser("~/.cys/state"))
        return os.path.realpath(root) == canon
    except OSError:
        return False


def _read_json_file(p):
    try:
        st = json.load(open(p, encoding="utf-8"))
        return st if isinstance(st, dict) else None
    except (OSError, ValueError):
        return None


def _load_state():
    priv = _read_json_file(_state_path())
    if _is_canonical():
        # ★P0-2 stale 분기 봉쇄 — 데몬 state.json(다른 세션이 checkpoint로 병합한 신규 라운드)을
        #   사설 진실에 라운드 단위 union. 사설이 아는 라운드는 lifecycle 권위 유지(tombstone·attempts),
        #   사설이 모르는 라운드만 데몬에서 편입(신규 라운드 미가시 봉쇄).
        daemon = _read_json_file(os.path.join(learn_dir(), "state.json"))
        base = priv if priv is not None else {}
        if daemon:
            dr = daemon.get("rounds", {})
            br = base.setdefault("rounds", {})
            for rid, rec in (dr.items() if isinstance(dr, dict) else []):
                if rid not in br:
                    br[rid] = rec
            for k, v in (daemon.get("discovery", {}) or {}).items():
                cur = base.setdefault("discovery", {})
                if isinstance(v, (int, float)):
                    cur[k] = max(cur.get(k, 0), v)
        st = base if (priv is not None or daemon) else None
    else:
        st = priv if priv is not None else _read_json_file(os.path.join(learn_dir(), "state.json"))
    if isinstance(st, dict):
        st.setdefault("rounds", {})
        st.setdefault("discovery", {"capability": 0, "perspective": 0, "knowledge": 0})
        return st
    return {"rounds": {}, "discovery": {"capability": 0, "perspective": 0, "knowledge": 0}}


def _save_state(state):
    d = learn_dir()
    os.makedirs(d, exist_ok=True)
    p = _state_path()
    tmp = p + ".tmp"
    open(tmp, "w", encoding="utf-8").write(json.dumps(state, ensure_ascii=False, indent=2))
    os.replace(tmp, p)
    if _is_canonical():
        return  # ★P0-2 canonical 모드 — state.json 미러 금지(데몬 단일 writer·전파는 _push_checkpoint)
    try:  # 세션 모드 데몬 가독 미러(기존 표면) — 실패해도 사설 진실은 이미 기록됨.
        mp = os.path.join(d, "state.json")
        mtmp = mp + ".tmp"
        open(mtmp, "w", encoding="utf-8").write(json.dumps(state, ensure_ascii=False, indent=2))
        os.replace(mtmp, mp)
    except OSError:
        pass


def _append_ledger(entry):
    d = learn_dir()
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "ledger.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _round_rec(state, rid):
    r = state.setdefault("rounds", {}).setdefault(
        rid, {"round": rid, "verdict": None, "stored": [], "harness": [], "created_at": time.time()})
    # legacy state.json 폴백으로 rsi 스키마 레코드가 섞여 들어온 경우 관용 백필.
    r.setdefault("stored", [])
    r.setdefault("harness", [])
    return r


def _run(tool, args):
    """위임 도구 호출 — (rc, stdout, stderr). 환경(CYS_ROUND_DIR 등) 승계."""
    r = subprocess.run([sys.executable, tool] + args, capture_output=True, text=True, env=dict(os.environ))
    return r.returncode, r.stdout.strip(), r.stderr.strip()


def _push_checkpoint(state, rid):
    """라운드 기록을 데몬 canonical(~/.cys/state/learn)에 best-effort push — CC 학습 탭 데이터원.
    데몬이 canonical의 단일 writer이고 로컬 state.json이 진실이다. 이 push는 순수 부가 동기화이며
    모든 실패(cys 부재=FileNotFoundError·timeout·비0 exit)는 조용히 무시한다 — push 실패가 로컬
    학습 기록을 절대 막지 않는다(비0 exit는 check=False라 예외를 던지지 않아 자연 무시된다)."""
    payload = checkpoint_payload(state, rid)  # C2 v2: 기존 5키 + items·evaluator_hash·schema
    if payload is None:
        return
    try:
        subprocess.run(["cys", "learn-checkpoint"], input=json.dumps(payload, ensure_ascii=False),
                       text=True, timeout=5, capture_output=True)
    except (OSError, subprocess.SubprocessError):
        pass


def _read_gate_input(gate_input_arg):
    """--gate-input 1회 파싱(stdin '-' 이중 읽기 방지). 반환 (gi|None, err|None)."""
    if not gate_input_arg:
        return None, None
    try:
        gi = _read_json_arg(gate_input_arg)
    except (OSError, ValueError) as e:
        return None, f"gate-input 읽기/파싱 실패: {e}"
    if not isinstance(gi, dict):
        return None, "gate-input 객체 아님"
    return gi, None


def _enforce_gate(gi, step, state, fallback, extra=None):
    """★rsi-gate.sh 강제 호출(통합 — 봉쇄 우회 차단). 반환 (ok, msg).

    gi=파싱된 gate-input(검증 증거 번들). step·target_state·fallback_mode(+extra: pattern·
    conflict_candidates·evaluator_hash 등 javis_learn 실측치)를 권위적으로 주입한 뒤
    rsi-gate.sh를 호출한다. gate가 DENY(exit≠0)면 ok=False. gate-input 부재/불량도 fail-closed."""
    if gi is None:
        return False, "rsi-gate 통합: --gate-input 필수(검증 증거 번들 없이는 봉쇄 통과 증명 불가)"
    if not isinstance(gi, dict):
        return False, "gate-input 객체 아님"
    gi = dict(gi)
    gi["step"] = step
    gi["target_state"] = state
    gi["fallback_mode"] = bool(fallback)
    if extra:
        gi.update(extra)
    r = subprocess.run(["bash", GATE], input=json.dumps(gi, ensure_ascii=False),
                       capture_output=True, text=True, env=dict(os.environ))
    if r.returncode != 0:
        return False, "rsi-gate DENY(봉쇄 미통과): " + (r.stderr.strip() or r.stdout.strip())
    return True, "gate allow"


# ───────────────────────── C9 conflictscan · C6 freeze · 항목 탐색 ─────────────────────────

def _pack_dir():
    return os.environ.get("CYS_PACK_DIR") or os.path.expanduser(os.path.join("~", ".cys", "pack"))


def _conflict_corpus():
    """C9 코퍼스: pack directives(읽기전용) · _round/*.md · skills SKILL.md · MEMORY.md +
    confirmed memory 본문(provisional 표기 본문은 제외). 정렬=결정론."""
    pack = _pack_dir()
    round_dir = os.path.dirname(learn_dir())
    files = sorted(glob.glob(os.path.join(pack, "directives", "*.md")))
    files += sorted(glob.glob(os.path.join(round_dir, "*.md")))
    files += sorted(glob.glob(os.path.join(pack, "skills", "*", "SKILL.md")))
    mem_files = sorted(glob.glob(os.path.join(pack, "memory", "*.md")))
    for p in mem_files:
        if os.path.basename(p) == "MEMORY.md":
            files.append(p)
            continue
        try:
            body = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        if '"state": "provisional"' not in body:  # confirmed(또는 무표기 legacy) 본문만
            files.append(p)
    return files


_TOKEN_RE = re.compile(r"[0-9A-Za-z가-힣_]+")


def _keywords(text):
    """키워드 추출(순수): ascii 4자+·비ascii 2자+ 토큰. 한계 명문: 어휘 회피·교차 언어에 무력."""
    out = set()
    for t in _TOKEN_RE.findall(str(text).lower()):
        if len(t) >= (2 if any(ord(ch) > 127 for ch in t) else 4):
            out.add(t)
    return out


def conflict_scan(pattern, corpus_files=None, min_overlap=2, cap=50):
    """C9 결정론부: pattern claim 키워드 ↔ 코퍼스 라인 대조 → conflict_candidates.
    후보 추출기이지 증명기 아님(계약 명문). 반환 {candidates, keywords, corpus_size}."""
    claim_text = " ".join(str(pattern.get(k, "")) for k in
                          ("domain", "condition", "action", "behavioral_claim", "falsifier"))
    kws = _keywords(claim_text)
    cands = []
    files = _conflict_corpus() if corpus_files is None else corpus_files
    if kws:
        for fp in files:
            try:
                lines = open(fp, encoding="utf-8", errors="replace").read().splitlines()
            except OSError:
                continue
            for ln, line in enumerate(lines, 1):
                hit = kws & _keywords(line)
                if len(hit) >= min_overlap:
                    cands.append({"file": fp, "line": ln, "text": line.strip()[:200],
                                  "keywords": sorted(hit)})
                    if len(cands) >= cap:
                        return {"candidates": cands, "keywords": sorted(kws),
                                "corpus_size": len(files), "capped": True}
    return {"candidates": cands, "keywords": sorted(kws), "corpus_size": len(files)}


def sample_audit_flag(key):
    """C9: 후보 0건=시드 기록된 20% 샘플 감사 플래그(결정론·재현 가능). 반환 (seed_hex, bool)."""
    seed = hashlib.sha256(str(key).encode("utf-8")).hexdigest()
    return seed, int(seed, 16) % 5 == 0


def _bench_dir():
    return os.path.join(learn_dir(), "benchmarks")


def _bench_path(rid):
    safe = re.sub(r"[^A-Za-z0-9._-]", "_", str(rid))[:80]
    return os.path.join(_bench_dir(), f"{safe}.json")


def freeze_content_sha(ledger):
    """C6: content_sha256 = {tasks, success_criteria, aux_metrics_protocol} canonical JSON 해시."""
    content = {k: ledger.get(k) for k in ("tasks", "success_criteria", "aux_metrics_protocol")}
    return hashlib.sha256(json.dumps(content, ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest()


def _freeze_check(rid, state):
    """C6 evaluate 게이트: freeze 레코드 존재 시 해시 무결 검증(불일치=위반). freeze 레짐
    활성(벤치마크 ledger 1개+ 존재) 상태에서 '신규 라운드'(state에 기록 없음)가 freeze 없이
    evaluate=위반. 레짐 비활성(freeze 이전 환경)·구 라운드=면제(후방 호환). 반환 (ok, msg)."""
    fp = _bench_path(rid)
    if os.path.isfile(fp):
        try:
            led = json.load(open(fp, encoding="utf-8"))
        except (OSError, ValueError) as e:
            return False, f"freeze ledger 읽기/파싱 실패(fail-closed): {e}"
        if not isinstance(led, dict) or led.get("content_sha256") != freeze_content_sha(led):
            return False, "freeze ledger 해시 무결 위반(사후 변조 의심) — 기준 사전 등록 오염"
        return True, "freeze 무결"
    # ★P1-2 냉시작 봉쇄 — 레짐 활성 판정을 '벤치마크 파일 존재'(자발적 opt-in·냉시작 우회)에서
    #   '첫 confirmed 승격 이후 freeze 의무'로 강화. confirmed가 하나라도 존재하면 레짐 활성.
    regime_active = (bool(glob.glob(os.path.join(_bench_dir(), "*.json")))
                     or state.get("confirmed_ever") is True
                     or any(it.get("state") == "confirmed"
                            for _, _, it, _ in _iter_items(state)))
    if regime_active and rid not in state.get("rounds", {}):
        return False, f"신규 라운드 '{rid}' freeze 레코드 부재 — 기준 사전 등록(freeze) 없이 evaluate 불가(fail-closed·레짐 활성)"
    return True, "freeze 면제(레짐 이전 환경 또는 구 라운드)"


def _ledger_evaluate_count(rid):
    """C10: 라운드별 evaluate 시도 수 — ledger.jsonl 계수(append-only 진실)."""
    p = os.path.join(learn_dir(), "ledger.jsonl")
    n = 0
    try:
        for ln in open(p, encoding="utf-8"):
            try:
                e = json.loads(ln)
            except ValueError:
                continue
            if e.get("event") == "evaluate" and e.get("round") == rid:
                n += 1
    except OSError:
        pass
    return n


def _iter_items(state):
    """전 라운드 stored/harness 항목 순회 — (rid, kind, item_dict, item_id) yield."""
    for rid, r in state.get("rounds", {}).items():
        if not isinstance(r, dict):
            continue
        for s in r.get("stored", []) or []:
            yield rid, "stored", s, s.get("id") or s.get("name")
        for h in r.get("harness", []) or []:
            yield rid, "harness", h, h.get("id") or h.get("harness_ref")


def _find_item(state, item_id):
    """id로 항목 탐색(최신 우선 — 마지막 일치 반환). 반환 (rid, kind, item) 또는 (None,)*3."""
    found = (None, None, None)
    for rid, kind, item, iid in _iter_items(state):
        if iid == item_id:
            found = (rid, kind, item)
    return found


# ───────────────────────── 명령 ─────────────────────────

def cmd_propose(a):
    if a.reason not in VALID_REASONS:
        return fail(2, f"--reason은 {VALID_REASONS} 중 하나")
    payload = {"event": "propose", "topic": a.topic, "reason": a.reason,
               "evidence": [], "status": "awaiting_approval", "ts": time.time(),
               "note": "pending feed approval item 등록 → 사람이 'cys feed reply <id> allow'(또는 feed 패널)로 승인할 때만 ①~⑤ 착수. 거부=무실행."}
    _append_ledger(payload)
    print(json.dumps(payload, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_search(a):
    try:
        cands = _read_json_arg(a.candidates)
    except (OSError, ValueError) as e:
        return fail(2, f"candidates 읽기/파싱 실패: {e}")
    res = validate_candidates(cands)
    if not res["ok"]:
        return fail(2, "① 검색 게이트 거부(citation/정박) — " + "; ".join(res["errors"]))
    out = {"event": "search", "topic": a.topic, "candidates": res["normalized"],
           "distinct_sources": res["distinct_sources"],
           "confidence": confidence_of(res["distinct_sources"]), "ts": time.time()}
    d = learn_dir()
    os.makedirs(d, exist_ok=True)
    slug = "".join(ch if ch.isalnum() else "-" for ch in a.topic.lower())[:48].strip("-") or "topic"
    path = os.path.join(d, f"candidates_{slug}.json")
    tmp = path + ".tmp"
    open(tmp, "w", encoding="utf-8").write(json.dumps(out["candidates"], ensure_ascii=False, indent=2))
    os.replace(tmp, path)
    out["candidates_path"] = path
    _append_ledger({k: out[k] for k in ("event", "topic", "distinct_sources", "confidence", "ts")})
    print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_extract(a):
    try:
        cands = _read_json_arg(getattr(a, "from"))
        pattern = _read_json_arg(a.pattern)
    except (OSError, ValueError) as e:
        return fail(2, f"from/pattern 읽기/파싱 실패: {e}")
    urls = [c.get("source_url", "") for c in cands] if isinstance(cands, list) else []
    res = validate_pattern(pattern, urls)
    if not res["ok"]:
        return fail(2, "② 추출 게이트 거부(스키마/정박) — " + "; ".join(res["errors"]))
    out = {"event": "extract", "pattern": pattern, "ts": time.time()}
    _append_ledger({"event": "extract", "domain": pattern.get("domain"),
                    "evidence_ref": pattern.get("evidence_ref"), "ts": out["ts"]})
    print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_evaluate(a):
    state = _load_state()
    # ★P0-5 judge shopping 상한 — 라운드 레코드 attempts 원자 카운터(state=writer 통제).
    #   ledger 계수는 append-only라 라인 삭제로 우회됐음(레드팀 F3). state.json은 canonical
    #   모드에서 데몬 단일 writer라 조작 불가. ledger 계수는 감사 로그·큰 값 우선(방어적).
    _existing = state.get("rounds", {}).get(a.round) or {}
    attempts = max(int(_existing.get("attempts", 0) or 0), _ledger_evaluate_count(a.round))
    if attempts >= EVALUATE_ATTEMPT_CAP:
        if not a.owner_approved:
            return fail(9, f"라운드 '{a.round}' evaluate {attempts}회 기록 — 4회째=ESCALATE"
                           "(judge shopping 상한). --owner-approved로만 속행")
        if not a.responds_to:
            return fail(2, "상한 초과 재제출은 --responds-to <직전 REVISE evidence 참조> 필수"
                           "(무응답 재제출 거부)")
    # ★C5 evaluator 계보 — 매니페스트 components 실측 sha256 트리 해시(자기신고 금지).
    evaluator_hash = None
    manifest_rec = None
    if a.evaluator_manifest:
        try:
            manifest = _read_json_arg(a.evaluator_manifest)
        except OSError:
            return fail(6, f"evaluator manifest 파일 부재/읽기 실패: {a.evaluator_manifest}")
        except ValueError as e:
            return fail(2, f"evaluator manifest 파싱 실패: {e}")
        comps = manifest.get("components") if isinstance(manifest, dict) else None
        if not (isinstance(comps, list) and comps and all(str(c).strip() for c in comps)):
            return fail(2, "evaluator manifest.components 목록 필수(launcher·프롬프트/의존·benchmark suite)")
        evaluator_hash, missing = evaluator_tree_hash([str(c) for c in comps])
        if evaluator_hash is None:
            return fail(6, f"evaluator manifest component 파일 부재(실측 해시 불가·fail-closed): {missing}")
        manifest_rec = {"components": [str(c) for c in comps],
                        "model_id": manifest.get("model_id"), "params": manifest.get("params")}
    # ★C6 freeze 사전 등록 게이트 — 레짐 활성 시 신규 라운드는 freeze 존재+해시 무결 필수.
    ok, fmsg = _freeze_check(a.round, state)
    if not ok:
        return fail(8, "③ 평가 거부(freeze) — " + fmsg)
    # ③ javis_rsi 위임 — 첫 회(또는 --baseline)=checkpoint, 이후=progress. score는 주입만.
    sub = "checkpoint" if a.baseline else "progress"
    args = [sub, "--round", a.round, "--score", repr(a.score)]
    if a.note:
        args += ["--note", a.note]
    rc, out, err = _run(RSI, args)
    if rc != 0:
        # checkpoint 없는데 progress면 baseline부터 — 재시도(checkpoint).
        if sub == "progress" and "checkpoint 없음" in err:
            rc, out, err = _run(RSI, ["checkpoint", "--round", a.round, "--score", repr(a.score)]
                                + (["--note", a.note] if a.note else []))
        if rc != 0:
            return fail(12, f"javis_rsi 위임 실패(일시적·재시도 가능): {err or out}")  # ★P2-1 위임 실패=12(계약 위반 exit 2/3과 분리)
    try:
        rsi_res = json.loads(out)
    except ValueError:
        rsi_res = {"raw": out}
    verdict = rsi_res.get("verdict", "baseline" if a.baseline else None)
    r = _round_rec(state, a.round)
    r["verdict"] = verdict
    r["last_score"] = a.score
    r["attempts"] = attempts + 1  # ★P0-5 원자 증가(state=writer 통제·ledger 삭제 무효화)
    r.setdefault("scores", []).append(a.score)  # C11 ⑤ C_n vs C_0 체인 대조용 이력
    if evaluator_hash:
        r["evaluator_hash"] = evaluator_hash    # C5 라운드 레코드 기록(실측)
        r["evaluator_manifest"] = manifest_rec
    _save_state(state)
    entry = {"event": "evaluate", "round": a.round, "score": a.score, "verdict": verdict,
             "attempt": attempts + 1, "ts": time.time()}
    if evaluator_hash:
        entry["evaluator_hash"] = evaluator_hash
    if a.responds_to:
        entry["responds_to"] = a.responds_to
    if a.owner_approved:
        entry["owner_approved"] = True
    _append_ledger(entry)
    print(json.dumps({**entry, "rsi": rsi_res}, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_store(a):
    state = _load_state()
    r = state.get("rounds", {}).get(a.round)
    verdict = (r or {}).get("verdict")
    if verdict is None:
        return fail(2, f"라운드 '{a.round}' 평가 기록 없음 — 먼저 evaluate 하라(verdict 필요)")
    allowed, reason = promotion_allowed(verdict, a.approved, a.fallback, a.state)
    if not allowed:
        return fail(2, "④ 저장 거부 — " + reason)
    if a.type not in VALID_TYPES:
        return fail(2, f"--type은 {VALID_TYPES} 중 하나")
    try:
        pattern = _read_json_arg(a.pattern)
    except (OSError, ValueError) as e:
        return fail(2, f"pattern 읽기/파싱 실패: {e}")
    pv = validate_pattern(pattern)
    if not pv["ok"]:
        return fail(2, "④ 저장 거부(pattern 스키마) — " + "; ".join(pv["errors"]))
    # ★C8 pattern v2 — v2 선언 시 behavioral_claim·falsifier·maturity 전부 필수(v1=관용).
    pv2 = validate_pattern_v2(pattern)
    if not pv2["ok"]:
        return fail(2, "④ 저장 거부(pattern v2) — " + "; ".join(pv2["errors"]))
    # ★P0-1 v2 자기선언 opt-out 봉쇄(레드팀 F1+감사 C8/C9 CRITICAL) — confirmed 승격은
    #   pattern v2(behavioral_claim·falsifier·maturity+known_failures 해시)를 무조건 필수.
    #   v1 pattern의 confirmed 승격 금지(v1=provisional 상한). "부분 통과=전체 중단" 준수.
    if a.state == "confirmed" and not pv2["v2"]:
        return fail(2, "④ 저장 거부(P0-1) — v1 pattern은 confirmed 승격 불가(provisional 상한). "
                       "confirmed는 behavioral_claim·falsifier·maturity(known_failures 해시 정박) 무조건 필수")
    name = a.name or slugify(pattern.get("domain", "learn"))
    # ★C3 역참조 마커 — --refs 각 경로에 learn:<id> 마커 실존 검증(부재=exit 3·fail-closed).
    refs = _parse_refs(a.refs)
    if refs:
        miss = missing_markers(refs, name)
        if miss:
            return fail(3, f"④ 저장 거부(역참조 마커 부재) — learn:{name} 마커 없는 경로: {miss}"
                           " (무강제 규약 금지 — 반영처에 마커 먼저)")
    # ★C9 conflictscan(store 전 자동) — 후보 추출기이지 증명기 아님(계약 명문 한계).
    scan = conflict_scan(pattern)
    conflict_candidates = scan["candidates"]
    scan_entry = {"event": "conflictscan", "round": a.round, "name": name,
                  "count": len(conflict_candidates), "ts": time.time()}
    if not conflict_candidates:
        seed, flag = sample_audit_flag(f"{a.round}/{name}")
        scan_entry["sample_audit"] = flag
        scan_entry["sample_audit_seed"] = seed
    _append_ledger(scan_entry)
    gi, gerr = _read_gate_input(a.gate_input)
    if gerr:
        return fail(2, "④ 저장 거부 — " + gerr)
    # ★C9 confirmed 승격 — P0-1: 후보 수 무관 conflict_audit verdict(reviewer2) 무조건 필수.
    #   (어휘 회피로 후보 0건→무감사 confirmed 우회 봉쇄. v1 confirmed는 위에서 이미 차단.)
    if a.state == "confirmed":
        ca = (gi or {}).get("conflict_audit")
        if not (isinstance(ca, dict) and str(ca.get("verdict", "")).strip().upper() in ("PASS", "ACCEPT")):
            return fail(2, "④ 저장 거부(C9) — confirmed 승격은 gate-input 번들에 conflict_audit "
                           f"verdict(reviewer2 의미 감사·PASS|ACCEPT) 무조건 필수 (충돌 후보 {len(conflict_candidates)}건)")
    # ★rsi-gate 강제 통합(codex BLOCK 보정) — 봉쇄 통과 증명 없이는 저장 불가(존재≠강제 결함 해소).
    #   pattern·conflict_candidates(+라운드 evaluator_hash)는 javis_learn 실측치를 권위 주입.
    extra = {"pattern": pattern, "conflict_candidates": conflict_candidates}
    if (r or {}).get("evaluator_hash"):
        extra["evaluator_hash"] = r["evaluator_hash"]
    ok, gmsg = _enforce_gate(gi, "store", a.state, a.fallback, extra)
    if not ok:
        return fail(2, "④ 저장 거부 — " + gmsg)
    desc = a.desc or f"[{a.state}] {pattern.get('domain')}: {pattern.get('action')}"[:200]
    body = json.dumps({"pattern": pattern, "state": a.state, "round": a.round,
                       "verdict": verdict, "evidence_ref": pattern.get("evidence_ref")},
                      ensure_ascii=False, indent=2)
    rc, out, err = _run(MEM, ["add", "--type", a.type, "--name", name, "--desc", desc, "--body", body])
    if rc != 0:
        return fail(12, f"javis_memory 위임 실패(일시적·재시도 가능): {err or out}")  # ★P2-1 위임 실패=12
    rec = {"name": name, "id": name, "type": a.type, "ts": time.time()}
    rec.update(new_item_fields(a.state, refs))  # C1 항목 레코드 v2
    r.setdefault("stored", []).append(rec)
    _save_state(state)
    _push_checkpoint(state, a.round)
    entry = {"event": "store", "round": a.round, "name": name, "state": a.state,
             "type": a.type, "verdict": verdict, "expires": rec["expires"],
             "refs": refs, "conflict_candidates": len(conflict_candidates), "ts": time.time()}
    _append_ledger(entry)
    print(json.dumps({**entry, "memory": out}, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_harness(a):
    state = _load_state()
    r = state.get("rounds", {}).get(a.round)
    verdict = (r or {}).get("verdict")
    try:
        pattern = _read_json_arg(a.pattern)
    except (OSError, ValueError) as e:
        return fail(2, f"pattern 읽기/파싱 실패: {e}")
    # ★C8 pattern v2 — store와 동일 규칙(v2 선언 시 전부 필수·v1=관용).
    pv2 = validate_pattern_v2(pattern)
    if not pv2["ok"]:
        return fail(2, "⑤ 채택 거부(pattern v2) — " + "; ".join(pv2["errors"]))
    # ★P0-1 — confirmed harness keep도 v1 승격 금지(store와 대칭·부분 통과=전체 중단).
    if a.state == "confirmed" and not pv2["v2"]:
        return fail(2, "⑤ 채택 거부(P0-1) — v1 pattern은 confirmed harness 채택 불가(provisional 상한)")
    harness_ref = a.evolve or slugify(pattern.get("domain", "learn"), prefix="rsi-harness-")
    # ★C3 역참조 마커 — --refs 각 경로 learn:<id> 실존 검증(부재=exit 3).
    refs = _parse_refs(a.refs)
    if refs:
        miss = missing_markers(refs, harness_ref)
        if miss:
            return fail(3, f"⑤ 채택 거부(역참조 마커 부재) — learn:{harness_ref} 마커 없는 경로: {miss}")
    retention = "keep" if verdict == "improved" else "rollback_recommended"
    # ★rsi-gate 강제 통합 — 채택(keep)은 봉쇄 통과 증명 필수. 폐기(rollback)는 게이트 무관.
    gate_passed = None
    if retention == "keep":
        gi, gerr = _read_gate_input(a.gate_input)
        if gerr:
            return fail(2, "⑤ 채택 거부 — " + gerr)
        ok, gmsg = _enforce_gate(gi, "harness", a.state, a.fallback, {"pattern": pattern})
        if not ok:
            return fail(2, "⑤ 채택 거부 — " + gmsg)
        gate_passed = True
    out = {"event": "harness", "round": a.round, "harness_ref": harness_ref,
           "evolve": a.evolve, "verdict": verdict, "retention": retention,
           "state": a.state, "fallback": bool(a.fallback), "gate_passed": gate_passed,
           "ts": time.time()}
    if retention == "rollback_recommended":
        rc, ro, re_ = _run(RSI, ["rollback", "--round", a.round])  # dry-run(기본·무실행)
        out["rollback_dry_run"] = ro or re_
    if r is not None:
        hrec = {"harness_ref": harness_ref, "id": harness_ref, "retention": retention,
                "fallback": bool(a.fallback), "gate_passed": gate_passed, "ts": out["ts"]}
        hrec.update(new_item_fields(a.state, refs))  # C1 항목 레코드 v2
        r.setdefault("harness", []).append(hrec)
        _save_state(state)
        _push_checkpoint(state, a.round)
    # ledger에 채택 요약(state·fallback·gate 통과) 기록 — codex minor(감사 추적성).
    _append_ledger({k: out[k] for k in
                    ("event", "round", "harness_ref", "retention", "state", "fallback", "gate_passed", "ts")})
    print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_challenge(a):
    """C4 탄핵 — open: 증거 번들 프리스크린(exit 4)·쿨다운 14d(exit 5)·challenged 중 효력 유지.
    resolve: upheld=tombstone(soft·물리 삭제 금지)+refs 스윕 출력 / rejected=이전 상태 복귀."""
    state = _load_state()
    rid, kind, item = _find_item(state, a.id)
    if item is None:
        return fail(2, f"학습물 id '{a.id}' 없음(stored/harness 레코드 미발견)")
    if a.resolve:
        ch = item.get("challenge")
        if not (isinstance(ch, dict) and ch.get("status") == "open" and item.get("state") == "challenged"):
            return fail(2, f"resolve 대상 아님 — '{a.id}'에 open challenge 없음")
        # ★P0-4 파괴 출구 fail-closed — upheld(tombstone)는 사람 승인 필수(위조 근거 자동 파괴 봉쇄).
        if a.resolve == "upheld" and not a.approved:
            return fail(2, "C4 resolve upheld 거부 — tombstone(파괴)은 --approved(사람 서명) 필수 "
                           "(위조 근거 tombstone DoS 봉쇄·입구와 동일 fail-closed)")
        ch["status"] = a.resolve
        ch["resolved_at"] = _today().isoformat()
        out = {"event": "challenge_resolve", "id": a.id, "round": rid, "kind": kind,
               "resolution": a.resolve, "ts": time.time()}
        if a.resolve == "upheld":
            item["state"] = "tombstone"  # soft — 물리 삭제 금지(레코드·본문 보존)
            out["refs_sweep"] = [p for p in (item.get("refs") or []) if marker_present(p, a.id)]
            out["note"] = "refs_sweep 경로의 learn 마커·반영분을 사람이 스윕하라(자동 삭제 없음)"
        else:
            item["state"] = ch.get("prior_state") or "provisional"
        _save_state(state)
        _push_checkpoint(state, rid)
        _append_ledger({k: out[k] for k in ("event", "id", "round", "resolution", "ts")})
        print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
        return 0
    if not a.evidence:
        return fail(2, "challenge open은 --evidence <path|-> 필수")
    try:
        bundle = _read_json_arg(a.evidence)
    except (OSError, ValueError) as e:
        return fail(4, f"C4 프리스크린 거부 — 증거 번들 읽기/파싱 실패: {e}")
    # ── 결정론 프리스크린: {id, reason, evidence[{source_url, snapshot_sha256, quote}]} 전 필드.
    errs = []
    if not isinstance(bundle, dict):
        errs.append("번들 객체 아님")
    else:
        if str(bundle.get("id", "")).strip() != a.id:
            errs.append(f"번들 id 부재/불일치(--id {a.id})")
        if not str(bundle.get("reason", "")).strip():
            errs.append("reason 비어있음")
        evs = bundle.get("evidence")
        if not (isinstance(evs, list) and evs):
            errs.append("evidence 배열 부재/0건")
        else:
            for j, e in enumerate(evs):
                if not isinstance(e, dict):
                    errs.append(f"evidence[{j}] 객체 아님")
                    continue
                u = str(e.get("source_url", "")).strip()
                if not (u.startswith("http://") or u.startswith("https://")):
                    errs.append(f"evidence[{j}].source_url 없음/비URL")
                decl = str(e.get("snapshot_sha256", ""))
                if not SHA256_RE.match(decl):
                    errs.append(f"evidence[{j}].snapshot_sha256 없음/64hex 아님")
                q = str(e.get("quote", "")).strip()
                if not q:
                    errs.append(f"evidence[{j}].quote 비어있음")
                # ★P0-4 위조 해시 봉쇄 — snapshot_path 제공 시 실 파일 해시 대조 + quote substring 정박.
                #   (형식 64hex만 검사 → 실측 대조로 강화. 파괴는 입구와 동일 무결성 요구.)
                sp = str(e.get("snapshot_path", "")).strip()
                if sp:
                    try:
                        raw = open(sp, "rb").read()
                    except OSError as ex:
                        errs.append(f"evidence[{j}].snapshot_path 읽기 실패: {ex}")
                    else:
                        actual = hashlib.sha256(raw).hexdigest()
                        if SHA256_RE.match(decl) and actual != decl:
                            errs.append(f"evidence[{j}] 해시 불일치(위조) — 선언 {decl[:12]}… ≠ 실측 {actual[:12]}…")
                        elif q and q not in raw.decode("utf-8", "replace"):
                            errs.append(f"evidence[{j}].quote가 스냅샷 본문에 없음(out-of-context/위조)")
                else:
                    errs.append(f"evidence[{j}].snapshot_path 없음 — 위조 방지 위해 실 파일 해시 대조 필수")
    if errs:
        return fail(4, "C4 프리스크린 거부(필드 부재/형식 위반) — " + "; ".join(errs))
    # ── 쿨다운 14d(스팸 DoS 차단) — 직전 challenge 일자 대조. open 중복도 거부.
    prev = item.get("challenge")
    if isinstance(prev, dict):
        if prev.get("status") == "open":
            return fail(5, f"'{a.id}' open challenge 진행 중 — 중복 탄핵 불가")
        pd = _parse_iso(prev.get("date"))
        if pd and (_today() - pd).days < CHALLENGE_COOLDOWN_DAYS:
            return fail(5, f"쿨다운 {CHALLENGE_COOLDOWN_DAYS}d 미경과(직전 {prev.get('date')})")
    prior = item.get("state") or "provisional"
    item["challenge"] = {"date": _today().isoformat(),
                         "evidence": a.evidence if a.evidence != "-" else "stdin",
                         "status": "open", "reason": bundle.get("reason"), "prior_state": prior}
    item["state"] = "challenged"  # ★효력 유지 명문 — 재검증 확정 전 정지 없음
    _save_state(state)
    _push_checkpoint(state, rid)
    out = {"event": "challenge", "id": a.id, "round": rid, "kind": kind,
           "status": "open", "prior_state": prior,
           "note": "challenged 상태 중 효력 유지(정지 없음) — resolve upheld|rejected로 확정",
           "ts": time.time()}
    _append_ledger({k: out[k] for k in ("event", "id", "round", "status", "ts")})
    print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_freeze(a):
    """C6 benchmark 사전 등록 — ledger=_round/learn/benchmarks/<round>.json(content_sha256).
    proposer=master(추천자=기준 저자 이해충돌)면 auditor_sig(reviewer2) 필수(부재=exit 7)."""
    try:
        bench = _read_json_arg(a.benchmarks)
    except (OSError, ValueError) as e:
        return fail(2, f"benchmarks 읽기/파싱 실패: {e}")
    if not isinstance(bench, dict):
        return fail(2, "benchmarks 객체 아님")
    missing = [k for k in ("tasks", "success_criteria", "aux_metrics_protocol") if k not in bench]
    if missing:
        return fail(2, f"benchmarks 필수 키 부재: {missing} (aux_metrics_protocol=보조지표 분모·분자·수집시점 — Goodhart 봉쇄)")
    # ★P1-1 자기신고 회피 봉쇄 — proposer는 검증 없는 자유 문자열이므로 'master 아님' 거짓 신고로
    #   auditor_sig를 우회할 수 있다. 이해충돌 판정을 proposer 신고에 맡기지 않고 auditor_sig를
    #   무조건 요구(reviewer2 감사 서명이 freeze의 결정론 정박 — 저위험 위장 우회 차단).
    if not a.auditor_sig:
        return fail(7, "freeze는 auditor_sig(reviewer2 감사 서명) 무조건 필수 "
                       "(proposer 자기신고로 이해충돌 게이트 우회 봉쇄·P1-1)")
    fp = _bench_path(a.round)
    if os.path.exists(fp):
        return fail(2, f"라운드 '{a.round}' 이미 동결됨 — 사전 등록 재동결 금지(변조 방지): {fp}")
    led = {"round": a.round, "tasks": bench["tasks"], "success_criteria": bench["success_criteria"],
           "aux_metrics_protocol": bench["aux_metrics_protocol"],
           "frozen_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
           "proposer": a.proposer, "auditor_sig": a.auditor_sig}
    led["content_sha256"] = freeze_content_sha(led)
    os.makedirs(_bench_dir(), exist_ok=True)
    tmp = fp + ".tmp"
    open(tmp, "w", encoding="utf-8").write(json.dumps(led, ensure_ascii=False, indent=2))
    os.replace(tmp, fp)
    entry = {"event": "freeze", "round": a.round, "proposer": a.proposer,
             "auditor_sig": bool(a.auditor_sig), "content_sha256": led["content_sha256"],
             "path": fp, "ts": time.time()}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def cmd_conflictscan(a):
    """C9 모순 탐지(결정론부) — 후보 추출기이지 증명기 아님(계약 명문 한계)."""
    try:
        pattern = _read_json_arg(a.pattern)
    except (OSError, ValueError) as e:
        return fail(2, f"pattern 읽기/파싱 실패: {e}")
    if not isinstance(pattern, dict):
        return fail(2, "pattern 객체 아님")
    res = conflict_scan(pattern)
    out = {"event": "conflictscan", "round": a.round, "name": a.name,
           "count": len(res["candidates"]), "candidates": res["candidates"],
           "keywords": res["keywords"], "corpus_size": res["corpus_size"], "ts": time.time()}
    if not res["candidates"]:
        seed, flag = sample_audit_flag(f"{a.round or ''}/{a.name or pattern.get('domain', '')}")
        out["sample_audit"] = flag       # C9: 0건=시드 기록된 20% 샘플 의미 감사 플래그
        out["sample_audit_seed"] = seed
    led_entry = {k: out[k] for k in ("event", "round", "name", "count", "ts")}
    if "sample_audit" in out:
        led_entry["sample_audit"] = out["sample_audit"]
        led_entry["sample_audit_seed"] = out["sample_audit_seed"]
    _append_ledger(led_entry)
    print(json.dumps(out, ensure_ascii=False, indent=2 if not a.json else None))
    return 0


def _reval_full_due(item):
    """G1 래칫 봉쇄 판정: reval_count가 2의 배수인 회차=5차원 full recheck 의무
    (연장 2회마다 1회 — 유지 게이트가 승격 게이트보다 영구히 약해지는 래칫 차단)."""
    return (item.get("reval_count") or 0) % 2 == 0


def cmd_audit(a):
    """C11 — ①만기 스캔(prov=tombstone·conf=wakeup enqueue·full-recheck 의무 판정) ②lapse
    (만기+30d=provisional 강등) ③refs 양방향 대조 ④effect_log none 2연속 보고 ⑤C_n vs C_0
    체인 대조. 항목당 전이는 1회/실행(보수 방향 — 강등된 항목의 후속 만기 처리는 다음 실행).
    hard-fail 존재=exit 1. --mark-revaled=reval 완료 기록(별도 흐름·스캔 미수행)."""
    state = _load_state()
    today = _today()
    if getattr(a, "record_effect", None):
        # ★P1-4 effect_log writer — C1 정의·C11④·§7-4가 소비하나 기록 수단 부재였음(dead consumer).
        rid, kind, item = _find_item(state, a.record_effect)
        if item is None:
            return fail(2, f"학습물 id '{a.record_effect}' 없음")
        if a.effect not in ("improved", "none"):
            return fail(2, "--effect는 improved|none")
        item.setdefault("effect_log", []).append(
            {"date": today.isoformat(), "effect": a.effect,
             "metrics": (json.loads(a.metrics) if a.metrics else {})})
        _save_state(state)
        _push_checkpoint(state, rid)
        entry = {"event": "record_effect", "id": a.record_effect, "round": rid,
                 "effect": a.effect, "ts": time.time()}
        _append_ledger(entry)
        print(json.dumps(entry, ensure_ascii=False, indent=2 if not a.json else None))
        return 0
    if getattr(a, "mark_revaled", None):
        # ── reval 완료 기록(최소 인터페이스): reval_count+1 · expires/review_due=+180d 재계산.
        rid, kind, item = _find_item(state, a.mark_revaled)
        if item is None:
            return fail(2, f"학습물 id '{a.mark_revaled}' 없음(stored/harness 레코드 미발견)")
        if item.get("state") != "confirmed":
            return fail(2, f"reval 완료 기록은 confirmed 항목만(현재 state={item.get('state')}) — "
                           "강등·묘비·탄핵 상태는 별도 절차(store 재승격/challenge resolve)")
        if _reval_full_due(item) and not a.full:
            return fail(2, f"reval_count={item.get('reval_count') or 0}(2의 배수) — 5차원 full "
                           "recheck 의무 회차: --full 필수(경량 연장으로 의무 회피 불가·래칫 봉쇄)")
        item["reval_count"] = (item.get("reval_count") or 0) + 1
        exp = _iso_after(today, TTL_DAYS["confirmed"])
        item["expires"] = exp
        item["review_due"] = exp
        item["last_reval"] = {"date": today.isoformat(), "full": bool(a.full)}
        _save_state(state)
        _push_checkpoint(state, rid)
        entry = {"event": "reval", "id": a.mark_revaled, "round": rid, "full": bool(a.full),
                 "reval_count": item["reval_count"], "expires": exp, "ts": time.time()}
        _append_ledger(entry)
        print(json.dumps(entry, ensure_ascii=False, indent=2 if not a.json else None))
        return 0
    report = {"expired_tombstoned": [], "reval_enqueued": [], "lapsed": [],
              "refs_hard_fail": [], "orphan_markers": [], "effect_none_streak": [],
              "chain_hard_fail": [], "sample_audits_pending": [], "ts": time.time()}
    # ★P1-5 sample_audit 소비 배선(레드팀 F7 dead signal 해소) — conflictscan이 발행한 20%
    #   샘플 감사 플래그를 audit가 읽어 미소비 목록으로 표면화(reviewer2 의미 감사 대상 큐).
    lp = os.path.join(learn_dir(), "ledger.jsonl")
    try:
        for ln in open(lp, encoding="utf-8"):
            try:
                e = json.loads(ln)
            except ValueError:
                continue
            if e.get("event") == "conflictscan" and e.get("sample_audit") is True:
                report["sample_audits_pending"].append(
                    {"round": e.get("round"), "name": e.get("name"),
                     "seed": e.get("sample_audit_seed"),
                     "note": "conflictscan 0건 20% 샘플 — reviewer2 의미 감사 대상"})
    except OSError:
        pass
    changed_rounds = set()
    known_ids = {iid for _, _, _, iid in _iter_items(state) if iid}
    for rid, kind, item, iid in _iter_items(state):
        st = item.get("state")
        exp = _parse_iso(item.get("expires"))
        # ①·② 만기 — 구 레코드(expires 부재)=면제·challenged/tombstone=전이 제외(관용).
        if exp and exp < today and st in ("provisional", "confirmed"):
            if st == "provisional":
                item["state"] = "tombstone"  # soft — 부트 격리·유지비 0(물리 삭제 없음)
                item["tombstoned_at"] = today.isoformat()
                report["expired_tombstoned"].append({"id": iid, "round": rid, "kind": kind})
                changed_rounds.add(rid)
            elif (today - exp).days > LAPSE_GRACE_DAYS:
                item["state"] = "provisional"  # lapse — 침묵 효력 연장 봉쇄(보수 방향=자동)
                item["lapsed_at"] = today.isoformat()
                report["lapsed"].append({"id": iid, "round": rid, "kind": kind,
                                         "reason": f"만기+{LAPSE_GRACE_DAYS}d 초과 미재검증"})
                changed_rounds.add(rid)
            else:
                # G1 래칫 봉쇄: reval_count 2의 배수 회차=full recheck 의무를 reason으로 결정론 전달.
                full = _reval_full_due(item)
                reason = "ttl-expired-full-recheck" if full else "ttl-expired"
                rc, _o, _e = _run(WAKEUP, ["enqueue", "--to", "master",
                                           "--task", f"learn-reval-{iid}",
                                           "--reason", reason,
                                           "--idempotency-key", f"learn-reval-{iid}"])
                ent = {"id": iid, "round": rid, "enqueue_rc": rc,
                       "reason": reason, "full_recheck": full}
                report["reval_enqueued"].append(ent)
                # ★P0-3 무음 유실 봉쇄 — enqueue 실패(launchd cwd=/ 등)를 hard-fail로 승격.
                #   재검 통지가 매일 조용히 사라지는 것을 exit 1로 loud화(드레인 주체=master 큐).
                if rc != 0:
                    report.setdefault("enqueue_failed", []).append(
                        {"id": iid, "round": rid, "enqueue_rc": rc,
                         "reason": "wakeup enqueue 실패 — 재검 통지 유실 위험(무음 금지)",
                         "stderr": (_e or "")[:200]})
        # ③ 정방향: 레코드 refs에 있는데 마커 없음=hard-fail 항목.
        for p in item.get("refs") or []:
            if not marker_present(p, iid):
                report["refs_hard_fail"].append({"id": iid, "path": p,
                                                 "reason": "레코드 refs에 있으나 파일 마커 부재('완결처럼 보이는 부분 목록' 차단)"})
        # ④ effect_log "none" 2연속 = 강등 사유 보고(ROI 축 — 루프도 eval 면제 불가).
        el = item.get("effect_log") or []
        if len(el) >= 2 and all(isinstance(e, dict) and e.get("effect") == "none" for e in el[-2:]):
            report["effect_none_streak"].append({"id": iid, "round": rid,
                                                 "reason": "효과 무 2연속 — 강등 사유"})
    # ③ 역방향: 기록된 refs 파일 합집합에서 미지 learn:<id> 마커=orphan 보고.
    #   한계 명문: 스캔 범위=레코드에 기록된 refs 파일(전역 grep은 비경계·2차 전파는 못 잡음).
    marker_re = re.compile(r"learn:([A-Za-z0-9._-]+)")
    ref_files = sorted({p for _, _, item, _ in _iter_items(state) for p in (item.get("refs") or [])})
    for p in ref_files:
        try:
            if p.lower().endswith(".json"):
                obj = json.load(open(p, encoding="utf-8"))
                ids = obj.get("_learn_refs") if isinstance(obj, dict) else []
                ids = [str(x) for x in ids] if isinstance(ids, list) else []
            else:
                ids = marker_re.findall(open(p, encoding="utf-8", errors="replace").read())
        except (OSError, ValueError):
            continue
        for mid in ids:
            if mid not in known_ids:
                report["orphan_markers"].append({"path": p, "id": mid})
    # ⑤ C_n vs C_0 체인 대조 — "직전 대비 승 + 최초 대비 하락"=누적 드리프트 hard-fail.
    for rid, r in state.get("rounds", {}).items():
        scores = (r or {}).get("scores") or []
        if len(scores) >= 3 and scores[-1] > scores[-2] and scores[-1] < scores[0]:
            report["chain_hard_fail"].append({"round": rid, "first": scores[0],
                                              "prev": scores[-2], "last": scores[-1],
                                              "reason": "직전 연승·최초 대비 하락(C_n<C_0)"})
    if changed_rounds:
        _save_state(state)
        for rid in sorted(changed_rounds):
            _push_checkpoint(state, rid)
    hard_fail = bool(report["refs_hard_fail"] or report["chain_hard_fail"]
                     or report.get("enqueue_failed"))  # ★P0-3 enqueue 실패=hard-fail(무음 금지)
    report["hard_fail"] = hard_fail
    _append_ledger({"event": "audit", "hard_fail": hard_fail,
                    **{k: len(report[k]) for k in ("expired_tombstoned", "reval_enqueued", "lapsed",
                                                   "refs_hard_fail", "orphan_markers",
                                                   "effect_none_streak", "chain_hard_fail")},
                    "ts": report["ts"]})
    print(json.dumps(report, ensure_ascii=False, indent=2 if not a.json else None))
    return 1 if hard_fail else 0


def cmd_status(a):
    state = _load_state()
    if getattr(a, "evaluator_hash", None):
        # C5 계보 질의 — 해당 심판(트리 해시)이 채점한 전 라운드(폭발 반경).
        rounds = sorted(rid for rid, r in state.get("rounds", {}).items()
                        if isinstance(r, dict) and r.get("evaluator_hash") == a.evaluator_hash)
        print(json.dumps({"evaluator_hash": a.evaluator_hash, "rounds": rounds},
                         ensure_ascii=False, indent=2 if not a.json else None))
        return 0
    if a.json:
        print(json.dumps(state, ensure_ascii=False, indent=2))
        return 0
    rounds = state.get("rounds", {})
    if not rounds:
        print("학습 라운드 기록 없음 (propose/search 로 시작)")
        return 0
    for rid, r in rounds.items():
        st = ", ".join(f"{s['name']}({s['state']})" for s in r.get("stored", [])) or "-"
        print(f"라운드 {rid}: verdict={r.get('verdict')} · 저장[{st}] · harness {len(r.get('harness', []))}")
    disc = state.get("discovery", {})
    print(f"발견 누적: 기능 {disc.get('capability', 0)} · 관점 {disc.get('perspective', 0)} · 지식 {disc.get('knowledge', 0)}")
    return 0


def main():
    ap = argparse.ArgumentParser(description="RSI 학습 루프(5단계) 결정론 엔진")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("propose"); p.add_argument("--reason", required=True); p.add_argument("--topic", required=True); p.add_argument("--json", action="store_true")
    s = sub.add_parser("search"); s.add_argument("--topic", required=True); s.add_argument("--candidates", required=True); s.add_argument("--json", action="store_true")
    e = sub.add_parser("extract"); e.add_argument("--from", required=True, dest="from"); e.add_argument("--pattern", required=True); e.add_argument("--json", action="store_true")
    ev = sub.add_parser("evaluate"); ev.add_argument("--round", required=True); ev.add_argument("--score", type=float, required=True); ev.add_argument("--baseline", action="store_true"); ev.add_argument("--note"); ev.add_argument("--evaluator-manifest", dest="evaluator_manifest", help="C5 evaluator 매니페스트(path|-) — components 실측 트리 해시"); ev.add_argument("--owner-approved", dest="owner_approved", action="store_true", help="C10 시도 상한(4회째 ESCALATE) 속행 승인"); ev.add_argument("--responds-to", dest="responds_to", help="C10 재제출 — 직전 REVISE evidence 참조"); ev.add_argument("--json", action="store_true")
    st = sub.add_parser("store"); st.add_argument("--round", required=True); st.add_argument("--pattern", required=True); st.add_argument("--type", required=True); st.add_argument("--approved", action="store_true"); st.add_argument("--state", default="provisional", choices=["provisional", "confirmed"]); st.add_argument("--fallback", action="store_true"); st.add_argument("--gate-input", dest="gate_input", help="rsi-gate 검증 증거 번들(path|-) — 강제 봉쇄 통과"); st.add_argument("--refs", help="C3 반영처 경로(콤마 구분) — 각 경로 learn:<id> 마커 실존 필수"); st.add_argument("--name"); st.add_argument("--desc"); st.add_argument("--json", action="store_true")
    h = sub.add_parser("harness"); h.add_argument("--round", required=True); h.add_argument("--pattern", required=True); h.add_argument("--evolve"); h.add_argument("--state", default="provisional", choices=["provisional", "confirmed"]); h.add_argument("--fallback", action="store_true"); h.add_argument("--gate-input", dest="gate_input", help="rsi-gate 검증 증거 번들(path|-) — 채택 시 강제"); h.add_argument("--refs", help="C3 반영처 경로(콤마 구분)"); h.add_argument("--json", action="store_true")
    ch = sub.add_parser("challenge"); ch.add_argument("--id", required=True); ch.add_argument("--evidence", help="C4 증거 번들(path|-) — {id, reason, evidence[{source_url, snapshot_path, snapshot_sha256, quote}]}"); ch.add_argument("--resolve", choices=["upheld", "rejected"]); ch.add_argument("--approved", action="store_true", help="P0-4 upheld(tombstone) 사람 서명"); ch.add_argument("--json", action="store_true")
    fz = sub.add_parser("freeze"); fz.add_argument("--round", required=True); fz.add_argument("--benchmarks", required=True, help="C6 {tasks, success_criteria, aux_metrics_protocol}(path|-)"); fz.add_argument("--proposer", required=True); fz.add_argument("--auditor-sig", dest="auditor_sig", help="reviewer2 감사 서명 — proposer=master면 필수"); fz.add_argument("--json", action="store_true")
    cs = sub.add_parser("conflictscan"); cs.add_argument("--pattern", required=True); cs.add_argument("--round"); cs.add_argument("--name"); cs.add_argument("--json", action="store_true")
    au = sub.add_parser("audit"); au.add_argument("--json", action="store_true"); au.add_argument("--mark-revaled", dest="mark_revaled", help="reval 완료 기록 — reval_count+1·expires +180d 재계산"); au.add_argument("--full", action="store_true", help="5차원 full recheck 수행 표기(의무 회차엔 필수)"); au.add_argument("--record-effect", dest="record_effect", help="P1-4 effect_log 기록 대상 id"); au.add_argument("--effect", choices=["improved", "none"], help="P1-4 채택 후 사후 효과"); au.add_argument("--metrics", help="P1-4 효과 지표 JSON(선택)")
    stt = sub.add_parser("status"); stt.add_argument("--json", action="store_true"); stt.add_argument("--evaluator-hash", dest="evaluator_hash", help="C5 계보 질의 — 해당 해시가 채점한 라운드 목록")

    a = ap.parse_args()
    return {"propose": cmd_propose, "search": cmd_search, "extract": cmd_extract,
            "evaluate": cmd_evaluate, "store": cmd_store, "harness": cmd_harness,
            "challenge": cmd_challenge, "freeze": cmd_freeze, "conflictscan": cmd_conflictscan,
            "audit": cmd_audit, "status": cmd_status}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
