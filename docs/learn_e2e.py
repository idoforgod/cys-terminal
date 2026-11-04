#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""RSI 학습 루프 E2E — javis_learn.py + rsi-gate.sh를 격리 환경에서 실측한다.

(1) 순수 로직(validate_candidates·validate_pattern·promotion_allowed·confidence_of·slugify),
(2) 정상 경로(propose→search→extract→evaluate[→javis_rsi]→store[→javis_memory]→harness→status),
(3) ★봉쇄 거부 케이스 — 오너 절대명제(부분 통과 = 전체 중단)를 코드로 확인:
    출처0 hard fail · pattern 정박 실패 · store 무승인/verdict非improved/fallback confirmed ·
    rsi-gate: 복구수단 불변 · 고위험 무서명 · fallback confirmed · 출처 fetch_log0 ·
    스냅샷 해시 위변조 · quote 부재(out-of-context) · 논리 JSON 파싱실패=FAIL/verdict FAIL ·
    내용우수성 미충족 · 공통모드(동일 모델)/독립 verdict 누락.

실행: python3 docs/learn_e2e.py   (종료 0=전 PASS · 1=실패 존재)
"""
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEARN = os.path.join(ROOT, "cysjavis-pack", "bin", "javis_learn.py")
GATE = os.path.join(ROOT, "cysjavis-pack", "bin", "rsi-gate.sh")
FAIL = []


def check(name, cond, detail=""):
    print(f"[{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        FAIL.append(name)


def load_module():
    spec = importlib.util.spec_from_file_location("learn", LEARN)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def git(args, cwd):
    return subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)


def run_learn(args, cwd, env_extra=None, stdin=None):
    env = dict(os.environ, CYS_ROUND_DIR=os.path.join(cwd, "_round"),
               CYS_PACK_DIR=os.path.join(cwd, "pack"))
    if env_extra:
        env.update(env_extra)
    return subprocess.run([sys.executable, LEARN] + args, cwd=cwd, capture_output=True,
                          text=True, env=env, input=stdin)


def run_gate(payload, cwd):
    return subprocess.run(["bash", GATE], cwd=cwd, capture_output=True, text=True,
                          input=json.dumps(payload))


def write(path, obj):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False)


def main():
    m = load_module()

    # ── (1) 순수 로직 ──
    good = [{"source_url": "https://w3.org/spec", "claim": "X", "retrieved_at": "2026-06-18"},
            {"source_url": "https://developer.mozilla.org/y", "claim": "Y", "retrieved_at": "2026-06-18"}]
    r = m.validate_candidates(good)
    check("validate_candidates 정상(2 출처)", r["ok"] and r["distinct_sources"] == 2, str(r))
    check("validate_candidates 빈 목록 hard fail", not m.validate_candidates([])["ok"])
    check("validate_candidates citation 누락 거부",
          not m.validate_candidates([{"claim": "no url"}])["ok"])
    check("confidence_of 단일출처=low", m.confidence_of(1) == "low" and m.confidence_of(2) == "med")

    pat = {"domain": "d", "condition": "c", "action": "a", "rationale": "r",
           "evidence_ref": "https://w3.org/spec"}
    check("validate_pattern 정상+정박", m.validate_pattern(pat, ["https://w3.org/spec"])["ok"])
    check("validate_pattern evidence_ref 미정박 거부",
          not m.validate_pattern(pat, ["https://other.com"])["ok"])
    check("validate_pattern 필드 누락 거부",
          not m.validate_pattern({"domain": "d"}, None)["ok"])

    # F58: promotion_allowed 는 승인 플래그를 읽지 않는다(플래그가 없다) — 무엇을 쓸지는 닫힌 모양 검출기가 정한다.
    ok1, _ = m.promotion_allowed("improved", False, "confirmed")
    ok2, _ = m.promotion_allowed("improved", True, "confirmed")
    ok3, _ = m.promotion_allowed("flat", False, "provisional")
    check("promotion_allowed improved allow(승인 플래그 없음)", ok1)
    check("promotion_allowed fallback+confirmed 차단", not ok2)
    check("promotion_allowed verdict非improved 차단", not ok3)
    check("F58 닫힌 모양 검출기: 명령 낱말이면 보류 이유",
          bool(m.string_problems("curl the archive")) and not m.string_problems("keep the cache folder"))
    check("slugify 슬러그 안전화", m.slugify("CSS @layer 전파!") .replace("-", "").isalnum())

    # ── (2) 정상 경로 (격리 git repo) ──
    with tempfile.TemporaryDirectory(prefix="cys-learn-") as d:
        git(["init", "-q"], d)
        git(["config", "user.email", "t@t"], d)
        git(["config", "user.name", "t"], d)
        open(os.path.join(d, "seed"), "w").write("x")
        git(["add", "-A"], d)
        git(["commit", "-qm", "seed"], d)
        os.makedirs(os.path.join(d, "pack", "memory"), exist_ok=True)
        open(os.path.join(d, "pack", "memory", "MEMORY.md"), "w", encoding="utf-8").write(
            "# MEMORY.md\n\n## 색인\n\n")

        cand_path = os.path.join(d, "cands.json")
        write(cand_path, good)
        pat_path = os.path.join(d, "pat.json")
        write(pat_path, pat)

        # 검증 증거 번들(gate-input) — 에이전트가 search/extract/factcheck/logic-review로 생성한 것을 모사.
        snap = os.path.join(d, "snapshot.txt")
        open(snap, "w", encoding="utf-8").write("hello canonical world quote-here")
        sha = hashlib.sha256(open(snap, "rb").read()).hexdigest()
        verdicts = [{"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
                    {"dimension": "logic", "model_family": "codex", "verdict": "PASS"}]
        gi_prov = os.path.join(d, "gi_prov.json")
        write(gi_prov, {
            "human_signed": False, "producer_model_family": "claude",
            "target_paths": [], "operations": [],
            "dimensions": {"source": {"fetch_log": True, "canonical": False, "distinct_sources": 1},
                           "fact_check": {"cross_checked": True},
                           "evidence": {"quote": "", "context_entailment": "support"},
                           "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                           "quality": {"eval_improved": True}},
            "verdicts": verdicts})
        gi_conf = os.path.join(d, "gi_conf.json")
        write(gi_conf, {
            "human_signed": False, "producer_model_family": "claude",
            "target_paths": ["docs/x.md"], "operations": [],
            "snapshot": {"path": snap, "sha256_expected": sha},
            # ★P0-1: confirmed는 conflict_audit(reviewer2) 무조건 필수.
            "conflict_audit": {"reviewer": "reviewer2", "verdict": "PASS"},
            "dimensions": {"source": {"fetch_log": True, "canonical": True, "distinct_sources": 2,
                                      "source_urls": ["https://w3.org/a", "https://developer.mozilla.org/b"]},
                           "fact_check": {"cross_checked": True},
                           "evidence": {"quote": "quote-here", "snapshot_path": snap, "context_entailment": "support"},
                           "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                           "quality": {"eval_improved": True}},
            "verdicts": verdicts})
        # ★P0-1: confirmed 승격용 v2 pattern(behavioral_claim·falsifier·maturity).
        pat_v2_path = os.path.join(d, "pat_v2.json")
        write(pat_v2_path, {**pat, "behavioral_claim": "관찰 가능 행동 서술",
                            "falsifier": "반증 관측 조건",
                            "maturity": {"first_seen": "2024-05", "adoption_evidence": "3개 조직",
                                         "known_failures": [{"source_url": "https://ex.org/pm",
                                                             "snapshot_sha256": sha, "summary": "실패 사례"}]}})

        r = run_learn(["propose", "--reason", "ceiling", "--topic", "T"], d)
        check("propose 0(F58: status recorded · 승인 대기 없음)", r.returncode == 0 and '"recorded"' in r.stdout
              and "awaiting_approval" not in r.stdout, r.stderr)

        r = run_learn(["search", "--topic", "T", "--candidates", cand_path, "--json"], d)
        check("search 정상 0", r.returncode == 0, r.stderr)

        r = run_learn(["extract", "--from", cand_path, "--pattern", pat_path, "--json"], d)
        check("extract 정상 0", r.returncode == 0, r.stderr)

        r = run_learn(["evaluate", "--round", "R1", "--score", "0.90", "--baseline", "--json"], d)
        check("evaluate baseline(→javis_rsi checkpoint) 0", r.returncode == 0, r.stderr)
        r = run_learn(["evaluate", "--round", "R1", "--score", "0.95", "--json"], d)
        improved = r.returncode == 0 and '"verdict": "improved"' in r.stdout
        check("evaluate progress improved(→javis_rsi) ", improved, r.stdout + r.stderr)

        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "provisional", "--gate-input", gi_prov,
                       "--name", "e2e-x", "--json"], d)
        check("store provisional+gate통과(→javis_memory) 0", r.returncode == 0, r.stdout + r.stderr)
        check("store가 memory 파일 생성",
              os.path.exists(os.path.join(d, "pack", "memory", "reference_e2e-x.md")), r.stderr)

        r = run_learn(["store", "--round", "R1", "--pattern", pat_v2_path, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf,
                       "--name", "e2e-conf", "--json"], d)
        check("F58 store confirmed=보류(rc8 · 게이트 미실행 · 기억 무기록)",
              r.returncode == 8 and '"not_run"' in r.stdout
              and not os.path.exists(os.path.join(d, "pack", "memory", "reference_e2e-conf.md")), r.stdout + r.stderr)
        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--approved", "--gate-input", gi_prov, "--name", "e2e-fake"], d)
        check("F58 store --approved=argparse 오류(rc2 · 승인 위조 불가)",
              r.returncode == 2 and "--approved" in r.stderr, r.stderr)

        r = run_learn(["harness", "--round", "R1", "--pattern", pat_path, "--gate-input", gi_prov, "--json"], d)
        check("harness keep(improved)+gate통과 0", r.returncode == 0 and '"retention": "keep"' in r.stdout, r.stderr)
        # (codex minor b) harness keep ledger에 state/fallback/gate 통과 요약 기록
        led = os.path.join(d, "_round", "learn", "ledger.jsonl")
        hk = [json.loads(ln) for ln in open(led, encoding="utf-8") if ln.strip()]
        hk = [e for e in hk if e.get("event") == "harness" and e.get("retention") == "keep"]
        check("harness keep ledger에 state/fallback/gate_passed 기록",
              bool(hk) and hk[-1].get("gate_passed") is True and "state" in hk[-1] and "fallback" in hk[-1],
              str(hk[-1]) if hk else "no harness keep ledger")

        r = run_learn(["status", "--json"], d)
        check("status 0 + R1 기록", r.returncode == 0 and "R1" in r.stdout, r.stderr)

        # ── (3a) javis_learn 봉쇄 거부 ──
        bad = os.path.join(d, "bad.json")
        write(bad, [])
        r = run_learn(["search", "--topic", "T", "--candidates", bad], d)
        check("search 출처0 hard fail(rc2)", r.returncode == 2, r.stdout)

        write(bad, [{"claim": "no url", "retrieved_at": "2026"}])
        r = run_learn(["search", "--topic", "T", "--candidates", bad], d)
        check("search citation 누락 거부(rc2)", r.returncode == 2, r.stdout)

        notanchor = os.path.join(d, "na.json")
        write(notanchor, {"domain": "d", "condition": "c", "action": "a", "rationale": "r",
                          "evidence_ref": "https://unlisted.example/z"})
        r = run_learn(["extract", "--from", cand_path, "--pattern", notanchor], d)
        check("extract 정박 실패 거부(rc2)", r.returncode == 2, r.stdout)

        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "provisional"], d)  # gate-input 없음
        check("store gate-input 없이 거부(rc2)", r.returncode == 2, r.stdout)

        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "confirmed", "--fallback"], d)
        check("store fallback+confirmed 차단(rc2)", r.returncode == 2, r.stdout)

        # verdict非improved 라운드
        run_learn(["evaluate", "--round", "R2", "--score", "0.90", "--baseline"], d)
        run_learn(["evaluate", "--round", "R2", "--score", "0.80"], d)  # regressed
        r = run_learn(["store", "--round", "R2", "--pattern", pat_path, "--type", "reference",
                       "--gate-input", gi_prov], d)
        check("store verdict非improved 거부(rc2)", r.returncode == 2, r.stdout)

        # ★통합 우회 차단(codex BLOCK 핵심): gate 없이/미통과 confirmed store·harness 불가.
        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "confirmed", "--name", "rsi-nogate"], d)  # gate-input 없음
        check("★store confirmed gate-input 없이 거부(우회차단·rc2)", r.returncode == 2, r.stdout + r.stderr)

        gi_conf_nosnap = os.path.join(d, "gi_conf_nosnap.json")
        bundle = json.load(open(gi_conf)); bundle.pop("snapshot")
        write(gi_conf_nosnap, bundle)
        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf_nosnap,
                       "--name", "rsi-nosnap"], d)
        check("★store confirmed 미통과 gate(snapshot 누락) 거부(rc2)", r.returncode == 2, r.stdout + r.stderr)

        gi_commonmode = os.path.join(d, "gi_cm.json")
        bundle = json.load(open(gi_conf))
        bundle["verdicts"] = [{"dimension": "fact_check", "model_family": "claude", "verdict": "PASS"},
                              {"dimension": "logic", "model_family": "claude", "verdict": "PASS"}]
        write(gi_commonmode, bundle)
        r = run_learn(["store", "--round", "R1", "--pattern", pat_path, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_commonmode,
                       "--name", "rsi-cm"], d)
        check("★store 통합: gate DENY(공통모드) 시 store 거부(rc2)", r.returncode == 2, r.stdout + r.stderr)

        r = run_learn(["harness", "--round", "R1", "--pattern", pat_path], d)  # keep인데 gate-input 없음
        check("★harness keep gate-input 없이 거부(우회차단·rc2)", r.returncode == 2, r.stdout + r.stderr)

        # ── (3b) rsi-gate.sh 봉쇄 (실파일 스냅샷·해시) ──
        snap = os.path.join(d, "snapshot.txt")
        open(snap, "w", encoding="utf-8").write("hello canonical world quote-here")
        sha = hashlib.sha256(open(snap, "rb").read()).hexdigest()

        base = {
            "step": "store", "target_state": "confirmed", "human_signed": False,
            "fallback_mode": False, "producer_model_family": "claude",
            "target_paths": ["docs/x.md"], "operations": [],
            "snapshot": {"path": snap, "sha256_expected": sha},
            "dimensions": {
                "source": {"fetch_log": True, "canonical": True, "distinct_sources": 2,
                           "source_urls": ["https://w3.org/a", "https://developer.mozilla.org/b"]},
                "fact_check": {"cross_checked": True},
                "evidence": {"quote": "quote-here", "snapshot_path": snap, "context_entailment": "support"},
                "logic": {"verdict_json": "{\"verdict\":\"PASS\",\"reason\":\"ok\"}"},
                "quality": {"eval_improved": True},
            },
            "verdicts": [
                {"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
                {"dimension": "logic", "model_family": "codex", "verdict": "PASS"},
            ],
        }
        import copy
        r = run_gate(copy.deepcopy(base), d)
        check("gate 정상경로 allow(0)", r.returncode == 0, r.stdout + r.stderr)

        def deny(name, mut):
            inp = copy.deepcopy(base); mut(inp)
            rr = run_gate(inp, d)
            check(name, rr.returncode == 1, f"rc={rr.returncode} {rr.stderr}")

        deny("gate 복구수단 불변 DENY(1)", lambda i: i.__setitem__("target_paths", ["refs/rsi/ckpt"]))
        deny("gate 고위험 무서명 DENY(1)", lambda i: i.__setitem__("target_paths", ["cysjavis-pack/bin/javis_rsi.py"]))
        deny("gate fallback+confirmed DENY(1)", lambda i: i.__setitem__("fallback_mode", True))
        deny("gate 출처 fetch_log0 DENY(1)", lambda i: i["dimensions"]["source"].__setitem__("fetch_log", False))
        deny("gate 스냅샷 해시 위변조 DENY(1)", lambda i: i["snapshot"].__setitem__("sha256_expected", "deadbeef" * 8))
        deny("gate quote 부재(out-of-context) DENY(1)", lambda i: i["dimensions"]["evidence"].__setitem__("quote", "NOT-IN-SNAPSHOT"))
        deny("gate 논리 JSON 파싱실패=FAIL DENY(1)", lambda i: i["dimensions"]["logic"].__setitem__("verdict_json", "{broken"))
        deny("gate 논리 verdict FAIL DENY(1)", lambda i: i["dimensions"]["logic"].__setitem__("verdict_json", "{\"verdict\":\"FAIL\"}"))
        deny("gate 내용우수성 미충족 DENY(1)", lambda i: i["dimensions"]["quality"].__setitem__("eval_improved", False))
        deny("gate 공통모드(동일 모델) DENY(1)", lambda i: i.__setitem__("verdicts", [
            {"dimension": "fact_check", "model_family": "claude", "verdict": "PASS"},
            {"dimension": "logic", "model_family": "claude", "verdict": "PASS"}]))
        deny("gate 독립 verdict 누락 DENY(1)", lambda i: i.__setitem__("verdicts", [
            {"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"}]))

        # ★confirmed 필수 필드 누락 = DENY (gemini R3 보정 · 선택적 필드 생략 우회 차단)
        deny("gate confirmed snapshot 누락 DENY(1)", lambda i: i.pop("snapshot"))
        deny("gate confirmed quote 빈문자열 DENY(1)", lambda i: i["dimensions"]["evidence"].__setitem__("quote", ""))
        deny("gate confirmed entailment≠support DENY(1)", lambda i: i["dimensions"]["evidence"].__setitem__("context_entailment", "neutral"))
        deny("gate confirmed verdict_json 누락 DENY(1)", lambda i: i["dimensions"]["logic"].__setitem__("verdict_json", None))
        deny("gate confirmed fact_check 누락 DENY(1)", lambda i: i["dimensions"].__setitem__("fact_check", {}))
        # (codex minor a) evidence.snapshot_path ≠ snapshot.path = DENY(해시 잠금 외 파일 대조 우회 차단)
        deny("gate snapshot_path≠evidence.snapshot_path DENY(1)",
             lambda i: i["dimensions"]["evidence"].__setitem__("snapshot_path", os.path.join(d, "other.txt")))

        # 고위험 + 인간서명 → allow
        signed = copy.deepcopy(base)
        signed["target_paths"] = ["cysjavis-pack/bin/javis_rsi.py"]
        signed["human_signed"] = True
        r = run_gate(signed, d)
        check("gate 고위험 인간서명 allow(0)", r.returncode == 0, r.stdout + r.stderr)

        # gate 빈 입력 fail-closed
        r = subprocess.run(["bash", GATE], cwd=d, capture_output=True, text=True, input="")
        check("gate 빈 입력 fail-closed DENY(1)", r.returncode == 1, r.stdout)

    # ── (4) LEARN GAPS v2 경로 스모크 — 상세 계약 검증은 docs/learn_gaps_tests.py ──
    with tempfile.TemporaryDirectory(prefix="cys-learn-v2-") as d:
        git(["init", "-q"], d)
        git(["config", "user.email", "t@t"], d)
        git(["config", "user.name", "t"], d)
        open(os.path.join(d, "seed"), "w").write("x")
        git(["add", "-A"], d)
        git(["commit", "-qm", "seed"], d)
        os.makedirs(os.path.join(d, "pack", "memory"), exist_ok=True)
        open(os.path.join(d, "pack", "memory", "MEMORY.md"), "w", encoding="utf-8").write(
            "# MEMORY.md\n\n## 색인\n\n")

        sha_a = "a" * 64
        cand_v2 = [{"source_url": "https://w3.org/spec", "claim": "X", "retrieved_at": "2026-07-17",
                    "canonical": True, "first_seen": "2024-05-01", "adoption_evidence": "3개 조직",
                    "known_failures": [{"source_url": "https://ex.org/pm", "snapshot_sha256": sha_a,
                                        "summary": "실패 사례"}],
                    "counterquery_log": ["X problems 역질의"]}]
        cv2_path = os.path.join(d, "cands_v2.json")
        write(cv2_path, cand_v2)
        r = run_learn(["search", "--topic", "V2", "--candidates", cv2_path, "--json"], d)
        check("v2 search 정상 + normalized 전 필드 보존",
              r.returncode == 0 and '"first_seen"' in r.stdout, r.stdout[:300] + r.stderr)

        pat_v2 = {"domain": "d", "condition": "c", "action": "a", "rationale": "r",
                  "evidence_ref": "https://w3.org/spec",
                  "behavioral_claim": "관찰 가능 행동", "falsifier": "반증 관측 조건",
                  "maturity": {"first_seen": "2024-05-01", "adoption_evidence": "3개 조직",
                               "known_failures": []}}  # F58 §4.2: URL 은 인용 자리 밖이면 보류 → 빈 목록
        pv2_path = os.path.join(d, "pat_v2.json")
        write(pv2_path, pat_v2)
        gi_prov2 = os.path.join(d, "gi_prov2.json")
        write(gi_prov2, {
            "human_signed": False, "producer_model_family": "claude",
            "target_paths": [], "operations": [],
            "dimensions": {"source": {"fetch_log": True, "canonical": False, "distinct_sources": 1},
                           "fact_check": {"cross_checked": True},
                           "evidence": {"quote": "", "context_entailment": "support"},
                           "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                           "quality": {"eval_improved": True}},
            "verdicts": [{"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
                         {"dimension": "logic", "model_family": "codex", "verdict": "PASS"}]})

        run_learn(["evaluate", "--round", "V2R", "--score", "0.90", "--baseline"], d)
        run_learn(["evaluate", "--round", "V2R", "--score", "0.95"], d)
        mark = os.path.join(d, "impl.md")
        open(mark, "w", encoding="utf-8").write("반영 문서\n<!-- learn:v2-smoke -->\n")
        r = run_learn(["store", "--round", "V2R", "--pattern", pv2_path, "--type", "reference",
                       "--gate-input", gi_prov2, "--refs", mark,
                       "--name", "v2-smoke", "--json"], d)
        check("v2 store(pattern v2+refs 마커) 정상", r.returncode == 0, r.stdout + r.stderr)
        check("v2 store 레코드 TTL(expires) 기록", '"expires"' in r.stdout, r.stdout[:300])

        bad_ev = os.path.join(d, "bad_ev.json")
        write(bad_ev, {"id": "v2-smoke", "reason": "r",
                       "evidence": [{"source_url": "https://e/x", "quote": "q"}]})  # 해시 부재
        r = run_learn(["challenge", "--id", "v2-smoke", "--evidence", bad_ev], d)
        check("v2 challenge 프리스크린 거부(rc4)", r.returncode == 4, f"rc={r.returncode}")

        r = run_learn(["audit", "--json"], d)
        check("v2 audit 정상(rc0·hard-fail 없음)", r.returncode == 0 and '"hard_fail": false' in r.stdout,
              r.stdout[:300] + r.stderr)

    print()
    if FAIL:
        print(f"❌ {len(FAIL)} FAIL: {FAIL}")
        return 1
    print("✅ 전 항목 PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
