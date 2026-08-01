#!/usr/bin/env bash
# rsi-gate — RSI 학습 봉쇄 강제자 (directive RSI_LEARNING §6 · DESIGN §5·§12 전부 코드화).
#
# 제품 절대명제: "할루시네이션 자료로 학습하면 시스템 전체가 붕괴한다." 학습물은 다음 라운드
# baseline·harness로 재귀 증폭되므로, 봉쇄를 100% 통과하지 못한 입력은 학습을 단 한 발자국도
# 진행시키지 않는다(★부분 통과 = 전체 중단).
#
# Threat model = 환각·우회 침투(워크플로 가드레일 아닌 무결성 경계). 따라서 ★fail-CLOSED:
# - 입력 파싱 실패·python3 부재·판단 불가 → DENY(exit 1). 통과보다 차단이 안전측.
#   (appbuild-gate.sh의 fail-OPEN과 정반대 — 봉쇄는 의심스러우면 막는다.)
# 기존 _round/autopilot/rsi-gate.sh(autopilot EFEC/AMI 게이트)와는 다른 파일·다른 도메인이다.
#
# 입력: gate-input JSON (인자 $1=파일 경로, 없으면 stdin). 스키마는 아래 python 헤더 참조.
# 종료코드: 0=allow · 1=deny(사유 stderr). --self-test=결정론 자기검증(0 통과/1 실패).

# 인터프리터 해소 — Windows는 python3 명령이 없고 python/py만 있는 경우가 흔하다(부트 실패 방지).
CYS_PY="$(command -v python3 || command -v python || command -v py)"
if [ -z "$CYS_PY" ]; then
  echo "deny: python(3) 부재 — fail-closed(봉쇄는 판단 불가 시 차단)" >&2
  exit 1
fi

if [ "${1:-}" = "--self-test" ]; then export RSI_GATE_SELF_TEST=1; INPUT="{}"; else
  if [ -n "${1:-}" ] && [ -f "$1" ]; then INPUT="$(cat "$1")"; else INPUT="$(cat)"; fi
fi
export RSI_GATE_INPUT="$INPUT"

exec "$CYS_PY" - <<'PYEOF'
import hashlib
import json
import os
import sys

# ── gate-input 스키마 (모든 필드 선택적이나, 누락=불충족=DENY 방향) ──
# {
#   "step": "store"|"harness"|...,
#   "fallback_mode": bool,                 # 폐쇄망 로컬모델 차선 여부
#   "target_state": "provisional"|"confirmed",
#   "target_paths": ["docs/x.md", ...],    # 이 학습물이 수정하려는 파일
#   "operations": ["file_write_io"|"network_socket"|"shell_exec"|"infra_change"],
#   "human_signed": bool,                  # 인간(오너) 서명 유무
#   "producer_model_family": "claude",
#   "snapshot": {"path": "...", "sha256_expected": "..."},   # 원문 스냅샷 무결성
#   "evaluator_hash": "<64hex>",           # C5 계보 — 선언 시 형식 검증(fail-CLOSED)
#   "maturity": {"first_seen": "...", "adoption_evidence": "...",   # C7 성숙도 3필드
#                "known_failures": [{"source_url","snapshot_sha256","summary"}, ...]},
#   "pattern": {...},                      # C8 — v2 필드 선언 시 3필드 전부 검증(fail-CLOSED)
#   "dimensions": {
#       "source":     {"fetch_log": bool, "canonical": bool, "distinct_sources": int},
#       "fact_check": {"cross_checked": bool},
#       "evidence":   {"quote": "...", "snapshot_path": "...", "context_entailment": "support"|"contradict"},
#       "logic":      {"verdict_json": "{\"verdict\":\"PASS\",\"reason\":\"...\"}"},
#       "quality":    {"eval_improved": bool},
#       "efficiency": {"tokens_saved": num>=0, "baseline_tokens": num>0,    # 선택(부재/빈=SKIP)
#                      "measured_by": "<실제 토크나이저 id>", "accuracy_retained": true}
#                      # 정확도(quality.eval_improved) 통과 후에만 평가 · 선언 시 malformed=DENY(fail-CLOSED)
#                      # · 크기 채점 안 함(무결성만; 채점=javis_rsi injected-only)
#   },
#   "verdicts": [ {"dimension":"fact_check","model_family":"gemini","verdict":"PASS"}, ... ]
# }

# 복구수단 불변(절대 DENY — 인간서명으로도 불가): 루프가 복구·롤백 기반을 건드리면 안 됨.
ABSOLUTE_PROTECTED = (".git", "refs/rsi", "rollback", "denylist")
# 고위험 인프라(인간서명 필요): 채점·게이트·원장 변경(codex R3 고위험 ④).
HIGHRISK_INFRA = ("evaluator", "benchmark", "ledger", "locked-eval", "rsi-gate", "rsi_rsi", "javis_rsi")
# 고위험 헌법(인간서명 필요): soul/directive 변조(고위험 ③).
HIGHRISK_CONST = ("soul", "claude.md", "/directives/", "directive")


def deny(reason):
    print(f"deny: {reason}", file=sys.stderr)
    sys.exit(1)


def allow(msg="allow"):
    print(json.dumps({"verdict": "allow", "msg": msg}, ensure_ascii=False))
    sys.exit(0)


def _match_any(paths, needles):
    hits = []
    for p in paths:
        lp = str(p).lower()
        for n in needles:
            if n in lp:
                hits.append(p)
                break
    return hits


def _is_sha256(s):
    return isinstance(s, str) and len(s) == 64 and all(c in "0123456789abcdefABCDEF" for c in s)


def _maturity_errors(mat, label):
    """C7 성숙도 3필드 검증 — known_failures 각 항목 source_url+snapshot_sha256+summary
    (해시 없는 비판=지어낸 비판 봉쇄). 반환: 오류 목록(빈=통과)."""
    if not isinstance(mat, dict):
        return [f"[{label}] maturity가 dict 아님 — fail-CLOSED"]
    errs = []
    for k in ("first_seen", "adoption_evidence"):
        if not str(mat.get(k, "")).strip():
            errs.append(f"[{label}] maturity.{k} 누락/빈 — fail-CLOSED")
    kf = mat.get("known_failures")
    if not isinstance(kf, list):
        errs.append(f"[{label}] maturity.known_failures 배열 누락 — fail-CLOSED")
    else:
        for j, item in enumerate(kf):
            if not isinstance(item, dict):
                errs.append(f"[{label}] known_failures[{j}] 객체 아님")
                continue
            u = str(item.get("source_url", ""))
            if not (u.startswith("http://") or u.startswith("https://")):
                errs.append(f"[{label}] known_failures[{j}].source_url 누락/비URL")
            if not _is_sha256(item.get("snapshot_sha256", "")):
                errs.append(f"[{label}] known_failures[{j}].snapshot_sha256 누락/64hex 아님")
            if not str(item.get("summary", "")).strip():
                errs.append(f"[{label}] known_failures[{j}].summary 누락")
    return errs


def gate(inp):
    """봉쇄 집행(순수 판정) — 통과 시 None, 위반 시 deny 사유 문자열. short-circuit 순서."""
    paths = inp.get("target_paths") or []
    ops = inp.get("operations") or []
    signed = bool(inp.get("human_signed"))

    # ① 복구수단 불변 — 무조건 DENY(인간서명으로도 불가).
    abs_hit = _match_any(paths, ABSOLUTE_PROTECTED)
    if abs_hit:
        return f"복구수단 불변 위반(절대 차단): {abs_hit} — 루프는 .git/refs/rsi/rollback/denylist를 수정할 수 없다"

    # ② 고위험 4종 → 인간서명 없으면 DENY.
    hr = []
    if _match_any(paths, HIGHRISK_INFRA) or "infra_change" in ops:
        hr.append("검증·게이트 인프라 변경(evaluator·benchmark·ledger·locked-eval·rsi-gate)")
    if _match_any(paths, HIGHRISK_CONST):
        hr.append("soul/CLAUDE/directive 변조")
    if "file_write_io" in ops:
        hr.append("파일 쓰기/삭제 I/O 권한")
    if "network_socket" in ops or "shell_exec" in ops:
        hr.append("네트워크 소켓·외부 쉘 실행")
    if hr and not signed:
        return "고위험(인간서명 필요) 무서명: " + " / ".join(hr)

    # ③ fallback 모드 + confirmed 승격 → DENY(provisional만 · codex R3).
    if inp.get("fallback_mode") and inp.get("target_state") == "confirmed":
        return "fallback 모드(단일 모델 변형·공통모드 방어 약화)에서 confirmed 승격 금지 — provisional만 허용"

    dims = inp.get("dimensions") or {}

    # ④ 5차원 결정론(기계) 먼저 — short-circuit(실패 시 즉시 DENY·고비용 이종모델 검증 생략).
    src = dims.get("source") or {}
    if not src.get("fetch_log"):
        return "[출처] fetch 실호출 로그 없음(학습지식 단독 금지·hard fail)"
    is_confirmed = inp.get("target_state") == "confirmed"
    # ★P1-3 자기신고 정수 → 실측 대체(레드팀 F6): confirmed는 source_urls 목록을 요구하고
    #   게이트가 distinct 호스트 수를 실측한다(자기신고 distinct_sources 정수 인플레 차단).
    if is_confirmed:
        urls = src.get("source_urls")
        if not (isinstance(urls, list) and urls):
            return "[출처] confirmed 승격에 source_urls 목록 필수(자기신고 정수 금지 — 게이트 실측 대상)"
        hosts = set()
        for u in urls:
            s = str(u)
            if "://" in s:
                s = s.split("://", 1)[1]
            hosts.add(s.split("/", 1)[0].lower())
        if len(hosts) < 2:
            return f"[출처] confirmed 독립 출처 2개+ 실측 필요(distinct host={len(hosts)})"
    if is_confirmed and not src.get("canonical"):
        return "[출처] canonical 미충족 — whitelist 밖은 provisional만(confirmed 불가)"

    # ★confirmed 필수 필드 누락 = DENY(gemini R3 보정 — 선택적 필드 생략으로 검증 건너뛰기 우회 차단).
    if is_confirmed:
        fc = dims.get("fact_check") or {}
        ev_c = dims.get("evidence") or {}
        lg_c = dims.get("logic") or {}
        snap_c = inp.get("snapshot") or {}
        if not fc.get("cross_checked"):
            return "[confirmed] fact_check.cross_checked 누락/거짓 — 교차 출처 대조 필수"
        if not str(ev_c.get("quote", "")).strip():
            return "[confirmed] evidence.quote 누락/빈문자열 — literal quote 정박 필수"
        if ev_c.get("context_entailment") != "support":
            return f"[confirmed] context_entailment != support('{ev_c.get('context_entailment')}') — 문맥 정합 필수"
        if lg_c.get("verdict_json") is None:
            return "[confirmed] logic.verdict_json 누락 — 독립모델 논리판정 필수"
        if not (snap_c.get("path") and snap_c.get("sha256_expected")):
            return "[confirmed] snapshot(path+sha256_expected) 누락 — 스냅샷 무결성 정박 필수"
        # ★evidence.snapshot_path는 해시 잠금된 snapshot.path와 일치해야 한다(codex minor) —
        #   불일치 시 해시 검증 안 된 다른 파일에서 quote 대조하는 우회를 차단.
        if ev_c.get("snapshot_path") is not None and ev_c.get("snapshot_path") != snap_c.get("path"):
            return "[confirmed] evidence.snapshot_path ≠ snapshot.path — 해시 잠금 외 파일 대조 우회 차단"

    # 스냅샷 SHA-256 무결성(위변조 봉쇄) — 검증 전 해시 = 최초 등록 해시.
    snap = inp.get("snapshot") or {}
    spath, sexp = snap.get("path"), snap.get("sha256_expected")
    if spath and sexp:
        try:
            actual = hashlib.sha256(open(spath, "rb").read()).hexdigest()
        except OSError as e:
            return f"[근거] 스냅샷 읽기 실패(무결성 검사 불가): {e}"
        if actual != sexp:
            return f"[근거] 스냅샷 해시 불일치(위변조 의심): expected {sexp[:12]}… actual {actual[:12]}…"

    # 근거 — literal quote가 (해시 잠금) 스냅샷에 실재 + entailment 모순 아님.
    ev = dims.get("evidence") or {}
    quote = ev.get("quote")
    if quote:
        # ★quote 대조는 해시 잠금된 snapshot.path에서만 한다(evidence.snapshot_path 신뢰 금지 ·
        #   codex minor) — 검증 안 된 다른 파일로 대조하는 위변조 우회 차단.
        if not spath:
            return "[근거] quote 정박 불가 — 해시 잠금 snapshot.path 부재(검증 불가)"
        try:
            content = open(spath, encoding="utf-8", errors="replace").read()
        except OSError:
            content = ""
        if quote not in content:
            return "[근거] literal quote가 해시 잠금 스냅샷에 없음(out-of-context/fabrication 의심)"
    if ev.get("context_entailment") == "contradict":
        return "[근거] claim이 인용 주변 문맥과 모순(entailment=contradict) — 폐기"

    # 논리 — 독립모델 판정을 JSON 스키마로 강제. 파싱 실패 = fail-safe FAIL.
    lg = dims.get("logic") or {}
    vj = lg.get("verdict_json")
    if vj is not None:
        try:
            parsed = json.loads(vj) if isinstance(vj, str) else vj
            v = parsed.get("verdict")
        except (ValueError, AttributeError):
            return "[논리] verdict_json 파싱 실패 = fail-safe FAIL(데드락·우회 차단)"
        if v not in ("PASS", "FAIL"):
            return f"[논리] verdict 비정상('{v}') = fail-safe FAIL"
        if v == "FAIL":
            return "[논리] 비형식 오류 발견(독립모델 verdict=FAIL) — 폐기"

    # 내용우수성 — 측정 우위 없으면 환각.
    if not (dims.get("quality") or {}).get("eval_improved"):
        return "[내용우수성] benchmark 실측 우위 없음(측정 없는 '더 나음' = 환각)"

    # 효율(토큰) — **정확도(quality.eval_improved) 통과 후에만** 평가(순서 load-bearing: 정확도
    # 회귀 시 토큰 win 으로 절대 인정 안 됨). 차원 부재/빈=SKIP(효율 주장 없음, 하위호환).
    # 선언 시 fail-CLOSED: 크기 채점이 아니라 주장 shape+accuracy_retained 무결성만 검증
    # (작은 양의 절감도 인정; 크기 채점=javis_rsi injected-only). '내용 삭제로 효율 보이기' 차단.
    eff = dims.get("efficiency")
    if eff:                                  # None·빈 dict = SKIP
        if not isinstance(eff, dict):
            return "[효율] efficiency 차원이 dict 아님 — fail-CLOSED"
        ts = eff.get("tokens_saved")
        bl = eff.get("baseline_tokens")
        mb = eff.get("measured_by")
        ar = eff.get("accuracy_retained")
        if not isinstance(ts, (int, float)) or isinstance(ts, bool) \
                or not isinstance(bl, (int, float)) or isinstance(bl, bool) \
                or not (isinstance(mb, str) and mb.strip()) or not isinstance(ar, bool):
            return ("[효율] 필드 누락/형식오류(tokens_saved 수>=0·baseline_tokens 수>0·"
                    "measured_by 비빈 str·accuracy_retained bool) — fail-CLOSED")
        if bl <= 0:
            return "[효율] baseline_tokens<=0 — 비율 산출 불가"
        if ts < 0:
            return "[효율] tokens_saved<0 — 음수 절감은 효율 학습 아님"
        if ar is not True:
            return "[효율] accuracy_retained=False — 정확도 미보존 절감은 reward-hack(내용 삭제) 차단"

    # ★C5 evaluator 계보 — 선언 시 64hex 형식 강제(부재=SKIP 하위호환·malformed=DENY).
    eh = inp.get("evaluator_hash")
    if eh is not None and not _is_sha256(eh):
        return "[계보] evaluator_hash 형식 위반(sha256 64hex 아님) — fail-CLOSED"

    # ★C7 성숙도 — 번들에 maturity 선언 시 3필드+known_failures 항목별 해시 강제(부재=SKIP).
    mat = inp.get("maturity")
    if mat is not None:
        errs = _maturity_errors(mat, "성숙도")
        if errs:
            return errs[0] + (f" (외 {len(errs)-1}건)" if len(errs) > 1 else "")

    # ★C8 pattern v2 — 번들에 pattern 선언 + v2 필드(behavioral_claim·falsifier·maturity)
    #   하나라도 존재하면 3필드 전부 유효 강제. v1 pattern(무선언)=SKIP(후방 호환·강화만).
    pt = inp.get("pattern")
    if pt is not None:
        if not isinstance(pt, dict):
            return "[패턴] pattern이 dict 아님 — fail-CLOSED"
        if any(k in pt for k in ("behavioral_claim", "falsifier", "maturity")):
            if not str(pt.get("behavioral_claim", "")).strip():
                return "[패턴v2] behavioral_claim 누락/빈(관찰 가능 행동 서술) — fail-CLOSED"
            if not str(pt.get("falsifier", "")).strip():
                return "[패턴v2] falsifier 누락/빈(반증 관측 조건) — fail-CLOSED"
            errs = _maturity_errors(pt.get("maturity"), "패턴v2")
            if errs:
                return errs[0] + (f" (외 {len(errs)-1}건)" if len(errs) > 1 else "")

    # ⑤ 의미·논리 독립 모델 패밀리 verdict 확인(공통모드 차단) — 결정론 통과 후에만.
    producer = str(inp.get("producer_model_family", "")).lower()
    verdicts = inp.get("verdicts") or []
    need = {"fact_check", "logic"}
    seen_ok = set()
    for vd in verdicts:
        fam = str(vd.get("model_family", "")).lower()
        dim = vd.get("dimension")
        if dim in need and vd.get("verdict") == "PASS" and fam and fam != producer:
            seen_ok.add(dim)
    missing = need - seen_ok
    if missing:
        return ("[공통모드] 독립 모델 패밀리(생산자≠팩트체커) PASS verdict 누락/불일치: "
                + ", ".join(sorted(missing)) + " (같은 모델·FAIL·미존재 모두 차단)")

    return None  # ★전 차원 통과(부분 통과=전체 중단의 역 — 전체 통과만 allow)


def self_test():
    # base = provisional 정상(완화 출처·quote/snapshot 선택). confirmed 정상 allow는 실파일 필요 → e2e서 검증.
    base = {
        "step": "store", "target_state": "provisional", "human_signed": False,
        "fallback_mode": False, "producer_model_family": "claude",
        "target_paths": ["docs/x.md"], "operations": [],
        "dimensions": {
            "source": {"fetch_log": True, "canonical": False, "distinct_sources": 1},
            "fact_check": {"cross_checked": True},
            "evidence": {"quote": "", "context_entailment": "support"},
            "logic": {"verdict_json": "{\"verdict\":\"PASS\",\"reason\":\"ok\"}"},
            "quality": {"eval_improved": True},
        },
        "verdicts": [
            {"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
            {"dimension": "logic", "model_family": "codex", "verdict": "PASS"},
        ],
    }
    import copy
    fails = []

    def expect(name, inp, want_allow):
        got = gate(inp)
        ok = (got is None) == want_allow
        print(f"[{'PASS' if ok else 'FAIL'}] {name}" + ("" if ok else f" — got={got!r}"))
        if not ok:
            fails.append(name)

    # confirmed 정상 형태(필수필드 충족·snapshot은 미존재 경로 → 누락 DENY 테스트엔 도달 전 반환).
    def cbase(**_):
        d = copy.deepcopy(base)
        d["target_state"] = "confirmed"
        d["dimensions"]["source"] = {"fetch_log": True, "canonical": True, "distinct_sources": 2,
                                     "source_urls": ["https://w3.org/a", "https://mozilla.org/b"]}
        d["dimensions"]["fact_check"] = {"cross_checked": True}
        d["dimensions"]["evidence"] = {"quote": "q", "context_entailment": "support"}
        d["dimensions"]["logic"] = {"verdict_json": "{\"verdict\":\"PASS\"}"}
        d["snapshot"] = {"path": "/nonexistent-snap", "sha256_expected": "00"}
        return d

    expect("provisional 정상 allow", copy.deepcopy(base), True)

    d = copy.deepcopy(base); d["target_paths"] = ["refs/rsi/ckpt/R1"]
    expect("복구수단 불변 DENY", d, False)

    d = copy.deepcopy(base); d["target_paths"] = ["cysjavis-pack/bin/javis_rsi.py"]
    expect("고위험 인프라 무서명 DENY", d, False)
    d2 = copy.deepcopy(d); d2["human_signed"] = True
    expect("고위험 인프라 인간서명 allow", d2, True)

    d = copy.deepcopy(base); d["fallback_mode"] = True; d["target_state"] = "confirmed"
    expect("fallback+confirmed DENY", d, False)
    d2 = copy.deepcopy(base); d2["fallback_mode"] = True
    expect("fallback+provisional allow", d2, True)

    d = copy.deepcopy(base); d["dimensions"]["source"]["fetch_log"] = False
    expect("출처 fetch_log 0 DENY", d, False)

    d = copy.deepcopy(base); d["target_state"] = "confirmed"
    d["dimensions"]["source"]["distinct_sources"] = 1
    d["dimensions"]["source"]["source_urls"] = ["https://only-one.example/a"]  # 1 host → 실측 DENY
    expect("confirmed 단일출처 DENY(실측)", d, False)
    # ★P1-3 자기신고 인플레 차단 — distinct_sources=9 자기신고여도 실측 host=1이면 DENY.
    d = cbase(); d["dimensions"]["source"]["distinct_sources"] = 9
    d["dimensions"]["source"]["source_urls"] = ["https://same.host/a", "https://same.host/b"]
    expect("confirmed 자기신고 인플레 무효(실측 host=1) DENY", d, False)

    d = copy.deepcopy(base); d["dimensions"]["logic"]["verdict_json"] = "{not valid json"
    expect("논리 JSON 파싱실패=FAIL DENY", d, False)
    d = copy.deepcopy(base); d["dimensions"]["logic"]["verdict_json"] = "{\"verdict\":\"FAIL\"}"
    expect("논리 verdict FAIL DENY", d, False)

    d = copy.deepcopy(base); d["dimensions"]["quality"]["eval_improved"] = False
    expect("내용우수성 미충족 DENY", d, False)

    d = copy.deepcopy(base); d["dimensions"]["evidence"]["context_entailment"] = "contradict"
    expect("entailment contradict DENY", d, False)

    d = copy.deepcopy(base); d["verdicts"] = [{"dimension": "fact_check", "model_family": "claude", "verdict": "PASS"},
                                              {"dimension": "logic", "model_family": "claude", "verdict": "PASS"}]
    expect("공통모드(동일 모델) DENY", d, False)

    d = copy.deepcopy(base); d["verdicts"] = [{"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"}]
    expect("독립 verdict 누락(logic) DENY", d, False)

    # ★confirmed 필수 필드 누락 = DENY (gemini R3 보정 검증)
    d = cbase(); d["dimensions"]["fact_check"] = {}
    expect("confirmed fact_check 누락 DENY", d, False)
    d = cbase(); d["dimensions"]["evidence"]["quote"] = ""
    expect("confirmed quote 누락 DENY", d, False)
    d = cbase(); d["dimensions"]["evidence"]["context_entailment"] = "neutral"
    expect("confirmed entailment≠support DENY", d, False)
    d = cbase(); d["dimensions"]["logic"]["verdict_json"] = None
    expect("confirmed verdict_json 누락 DENY", d, False)
    d = cbase(); d["snapshot"] = {}
    expect("confirmed snapshot 누락 DENY", d, False)
    d = cbase(); d["dimensions"]["evidence"]["snapshot_path"] = "/other-file"
    expect("confirmed snapshot_path≠snapshot.path DENY", d, False)

    # ★U4 효율 차원 (정확도-우선 순서·fail-CLOSED·크기 미채점)
    eff_ok = {"tokens_saved": 1500, "baseline_tokens": 5000,
              "measured_by": "tiktoken/o200k_base", "accuracy_retained": True}
    expect("efficiency 부재 allow(하위호환)", copy.deepcopy(base), True)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = dict(eff_ok)
    expect("efficiency valid allow", d, True)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = dict(eff_ok, tokens_saved=1)
    expect("efficiency 작은 양의 절감도 allow(크기 미채점)", d, True)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = dict(eff_ok, accuracy_retained=False)
    expect("efficiency accuracy_retained=False DENY(reward-hack 차단)", d, False)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = dict(eff_ok, baseline_tokens=0)
    expect("efficiency baseline_tokens=0 DENY", d, False)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = dict(eff_ok, tokens_saved=-5)
    expect("efficiency tokens_saved<0 DENY", d, False)
    d = copy.deepcopy(base); d["dimensions"]["efficiency"] = {"tokens_saved": 100, "baseline_tokens": 5000,
                                                              "measured_by": "", "accuracy_retained": True}
    expect("efficiency measured_by 빈 DENY(fail-CLOSED)", d, False)
    d = copy.deepcopy(base); d["dimensions"]["quality"]["eval_improved"] = False
    d["dimensions"]["efficiency"] = dict(eff_ok)
    expect("quality 미충족 + efficiency present DENY(정확도-우선 순서)", d, False)

    # ★C5 evaluator_hash (선언 시 형식 강제·부재=SKIP)
    d = copy.deepcopy(base); d["evaluator_hash"] = "a" * 64
    expect("evaluator_hash 유효 allow", d, True)
    d = copy.deepcopy(base); d["evaluator_hash"] = "short"
    expect("evaluator_hash malformed DENY(fail-CLOSED)", d, False)

    # ★C7 maturity (선언 시 3필드+known_failures 해시 강제·부재=SKIP)
    kf_ok = [{"source_url": "https://x.example/postmortem", "snapshot_sha256": "b" * 64, "summary": "s"}]
    mat_ok = {"first_seen": "2026-01-01", "adoption_evidence": "3 orgs", "known_failures": kf_ok}
    d = copy.deepcopy(base); d["maturity"] = dict(mat_ok)
    expect("maturity 유효 allow", d, True)
    d = copy.deepcopy(base); d["maturity"] = dict(mat_ok, known_failures=[{"source_url": "https://x.example/p", "summary": "s"}])
    expect("maturity known_failures 해시 누락 DENY", d, False)
    d = copy.deepcopy(base); d["maturity"] = dict(mat_ok); d["maturity"].pop("first_seen")
    expect("maturity first_seen 누락 DENY", d, False)
    d = copy.deepcopy(base); d["maturity"] = "not-a-dict"
    expect("maturity 비dict DENY(fail-CLOSED)", d, False)

    # ★C8 pattern v2 (v2 선언 시 3필드 강제·v1=SKIP 후방 호환)
    pat_v1 = {"domain": "d", "condition": "c", "action": "a", "rationale": "r", "evidence_ref": "https://e"}
    d = copy.deepcopy(base); d["pattern"] = dict(pat_v1)
    expect("pattern v1(무선언) allow(후방 호환)", d, True)
    pat_v2 = dict(pat_v1, behavioral_claim="관찰 행동", falsifier="반증 조건", maturity=dict(mat_ok))
    d = copy.deepcopy(base); d["pattern"] = dict(pat_v2)
    expect("pattern v2 유효 allow", d, True)
    d = copy.deepcopy(base); d["pattern"] = dict(pat_v2); d["pattern"].pop("falsifier")
    expect("pattern v2 falsifier 누락 DENY", d, False)
    d = copy.deepcopy(base); d["pattern"] = dict(pat_v2, maturity={"first_seen": "2026"})
    expect("pattern v2 maturity 불완전 DENY", d, False)

    return 1 if fails else 0


def main():
    if os.environ.get("RSI_GATE_SELF_TEST"):
        sys.exit(self_test())
    raw = os.environ.get("RSI_GATE_INPUT", "")
    try:
        inp = json.loads(raw) if raw.strip() else {}
    except ValueError as e:
        deny(f"gate-input JSON 파싱 실패(fail-closed): {e}")
    if not isinstance(inp, dict) or not inp:
        deny("gate-input 비어있음/객체 아님(fail-closed)")
    reason = gate(inp)
    if reason:
        deny(reason)
    allow()


if __name__ == "__main__":
    main()
PYEOF
