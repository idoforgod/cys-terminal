#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""LEARN GAPS(C1~C11) 검증 fixture — LEARN_GAPS_CONTRACT.md C13 목록 전체를 서브프로세스
exit code로 검증한다(자기완결·임시 디렉토리·실 상태 오염 금지).

커버리지(C13):
  구 페이로드 호환(파이썬 측=checkpoint_payload 5키 유지+v2 3키 — Rust apply fixture는 Rust 트랙) /
  v2 store 필드 누락 거부(C8) / 마커 누락 거부(C3) / challenge 프리스크린·쿨다운(C4) /
  freeze 부재 evaluate 거부·master-freeze 무서명 거부(C6) / candidates 필드·해시 누락 거부(C7) /
  시도 4회 ESCALATE(C10) / lapse 강등·provisional 만기 tombstone·reval enqueue(C11·C1) /
  evaluator manifest 해시 변동 검출(C5) / conflictscan 정면충돌 fixture 차단(C9).

실행: python3 docs/learn_gaps_tests.py   (종료 0=전 PASS · 1=실패 존재)
"""
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from datetime import date, timedelta

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LEARN = os.path.join(ROOT, "cysjavis-pack", "bin", "javis_learn.py")
GATE = os.path.join(ROOT, "cysjavis-pack", "bin", "rsi-gate.sh")
FAIL = []
SHA_A = "a" * 64


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


def make_env(d):
    """격리 git repo + pack 골격(실 상태 오염 금지 — CYS_PACK_DIR·CYS_ROUND_DIR·JAVIS_ROOT 전부
    격리 + no-op cys 스텁으로 _push_checkpoint의 실 데몬 push 차단)."""
    git(["init", "-q"], d)
    git(["config", "user.email", "t@t"], d)
    git(["config", "user.name", "t"], d)
    open(os.path.join(d, "seed"), "w").write("x")
    git(["add", "-A"], d)
    git(["commit", "-qm", "seed"], d)
    os.makedirs(os.path.join(d, "pack", "memory"), exist_ok=True)
    open(os.path.join(d, "pack", "memory", "MEMORY.md"), "w", encoding="utf-8").write(
        "# MEMORY.md\n\n## 색인\n\n")
    stub_dir = os.path.join(d, "stub-bin")
    os.makedirs(stub_dir, exist_ok=True)
    stub = os.path.join(stub_dir, "cys")
    open(stub, "w").write("#!/bin/sh\nexit 0\n")
    os.chmod(stub, 0o755)


def run_learn(args, cwd, stdin=None):
    env = dict(os.environ, CYS_ROUND_DIR=os.path.join(cwd, "_round"),
               CYS_PACK_DIR=os.path.join(cwd, "pack"), JAVIS_ROOT=cwd,
               PATH=os.path.join(cwd, "stub-bin") + os.pathsep + os.environ.get("PATH", ""))
    return subprocess.run([sys.executable, LEARN] + args, cwd=cwd, capture_output=True,
                          text=True, env=env, input=stdin)


def write(path, obj):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False)


def state_path(d):
    return os.path.join(d, "_round", "learn", "learn_state.json")


def load_state(d):
    return json.load(open(state_path(d), encoding="utf-8"))


def save_state(d, st):
    write(state_path(d), st)


def find_item(st, iid):
    for r in st.get("rounds", {}).values():
        for s in r.get("stored", []) + r.get("harness", []):
            if s.get("id") == iid or s.get("name") == iid or s.get("harness_ref") == iid:
                return s
    return None


def v2_candidates():
    kf = [{"source_url": "https://ex.org/postmortem", "snapshot_sha256": SHA_A, "summary": "실패 사례"}]
    return [{"source_url": "https://w3.org/spec", "claim": "X", "retrieved_at": "2026-07-17",
             "canonical": True, "first_seen": "2024-05-01", "adoption_evidence": "3개 조직 채택",
             "known_failures": kf, "counterquery_log": ["X problems 역질의 2026-07-17"]},
            {"source_url": "https://developer.mozilla.org/y", "claim": "Y", "retrieved_at": "2026-07-17",
             "canonical": True, "first_seen": "2024-06-01", "adoption_evidence": "표준 편입",
             "known_failures": [], "counterquery_log": ["Y criticism 역질의 2026-07-17"]}]


def v1_pattern():
    return {"domain": "d", "condition": "c", "action": "a", "rationale": "r",
            "evidence_ref": "https://w3.org/spec"}


def v2_pattern():
    p = v1_pattern()
    p.update({"behavioral_claim": "polling interval increase reduces idle load",
              "falsifier": "load unchanged after interval increase",
              # F58 §4.2: a URL anywhere but the citation is outside the closed shape (staged), so the
              #   stored v2 fixture carries an empty known_failures list (allowed: "no failure found").
              "maturity": {"first_seen": "2024-05-01", "adoption_evidence": "3개 조직 채택",
                           "known_failures": []}})
    return p


VERDICTS = [{"dimension": "fact_check", "model_family": "gemini", "verdict": "PASS"},
            {"dimension": "logic", "model_family": "codex", "verdict": "PASS"}]


def gi_prov_bundle():
    return {"human_signed": False, "producer_model_family": "claude",
            "target_paths": [], "operations": [],
            "dimensions": {"source": {"fetch_log": True, "canonical": False, "distinct_sources": 1},
                           "fact_check": {"cross_checked": True},
                           "evidence": {"quote": "", "context_entailment": "support"},
                           "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                           "quality": {"eval_improved": True}},
            "verdicts": VERDICTS}


def gi_conf_bundle(snap, sha):
    return {"human_signed": False, "producer_model_family": "claude",
            "target_paths": [], "operations": [],
            "snapshot": {"path": snap, "sha256_expected": sha},
            # ★P0-1: confirmed는 후보 수 무관 conflict_audit(reviewer2) 무조건 필수.
            "conflict_audit": {"reviewer": "reviewer2", "verdict": "PASS", "note": "의미 감사 통과"},
            "dimensions": {"source": {"fetch_log": True, "canonical": True, "distinct_sources": 2,
                                      "source_urls": ["https://w3.org/a", "https://developer.mozilla.org/b"]},
                           "fact_check": {"cross_checked": True},
                           "evidence": {"quote": "quote-here", "snapshot_path": snap,
                                        "context_entailment": "support"},
                           "logic": {"verdict_json": "{\"verdict\":\"PASS\"}"},
                           "quality": {"eval_improved": True}},
            "verdicts": VERDICTS}


def improved_round(d, rid):
    run_learn(["evaluate", "--round", rid, "--score", "0.90", "--baseline"], d)
    return run_learn(["evaluate", "--round", rid, "--score", "0.95"], d)


# ───────────────────────── 순수 로직(모듈 핀) ─────────────────────────

def pure_tests():
    m = load_module()

    # C2: checkpoint_payload — 기존 5키 유지 + v2 신규 3키(구 페이로드 호환의 파이썬 측 절반).
    st = {"rounds": {"R": {"round": "R", "verdict": "improved",
                           "stored": [{"name": "n", "type": "reference", "state": "provisional",
                                       "expires": "2026-10-15"}],
                           "harness": [{"harness_ref": "h", "state": "provisional",
                                        "expires": "2026-10-15"}],
                           "evaluator_hash": "e" * 64}},
          "discovery": {"capability": 1, "perspective": 0, "knowledge": 0}}
    pl = m.checkpoint_payload(st, "R")
    check("C2 payload 기존 5키 유지", all(k in pl for k in ("round", "verdict", "stored", "harness", "discovery")))
    check("C2 payload v2 신규 키(items·evaluator_hash·schema)",
          pl.get("schema") == "v2" and pl.get("evaluator_hash") == "e" * 64 and len(pl.get("items", [])) == 2,
          str(pl))
    check("C2 items 스키마 {name,type,state,expires}",
          pl["items"][0] == {"name": "n", "type": "reference", "state": "provisional", "expires": "2026-10-15"}
          and pl["items"][1]["type"] == "harness", str(pl["items"]))
    # 구 레코드(신규 필드 부재) 관용 — items가 None 필드로도 산출.
    st_old = {"rounds": {"R": {"round": "R", "verdict": "improved",
                               "stored": [{"name": "n", "type": "reference", "state": "provisional"}],
                               "harness": []}}, "discovery": {}}
    pl_old = m.checkpoint_payload(st_old, "R")
    check("C2 구 레코드 관용(expires=None)", pl_old["items"][0]["expires"] is None)
    # ★P2-2 evaluator_hash=None이면 payload 키 생략(Rust Some(Null) 덮어쓰기 방지).
    check("P2-2 evaluator_hash None=키 생략", "evaluator_hash" not in pl_old)

    # ★P0-2 canonical union — 사설 stale + 데몬 신규 라운드 병합·사설 lifecycle 권위 유지.
    with tempfile.TemporaryDirectory(prefix="cys-p02-") as d:
        learn = os.path.join(d, "learn"); os.makedirs(learn)
        old_env = os.environ.get("CYS_ROUND_DIR")
        os.environ["CYS_ROUND_DIR"] = d
        old_canon = m._is_canonical
        m._is_canonical = lambda: True
        try:
            write(os.path.join(learn, "learn_state.json"),
                  {"rounds": {"R1": {"round": "R1", "stored": [{"name": "a", "state": "provisional"}]}},
                   "discovery": {"capability": 1, "perspective": 0, "knowledge": 0}})
            write(os.path.join(learn, "state.json"),
                  {"rounds": {"R1": {"round": "R1", "stored": [{"name": "a", "state": "tombstone"}]},
                              "R2": {"round": "R2", "stored": [{"name": "b", "state": "confirmed"}]}},
                   "discovery": {"capability": 3, "perspective": 0, "knowledge": 0}})
            st2 = m._load_state()
            check("P0-2 신규 데몬 라운드 가시(stale 분기 소멸)", "R2" in st2["rounds"])
            check("P0-2 사설 lifecycle 권위 유지", st2["rounds"]["R1"]["stored"][0]["state"] == "provisional")
            check("P0-2 discovery max 병합", st2["discovery"]["capability"] == 3)
            before = open(os.path.join(learn, "state.json")).read()
            st2["rounds"]["R1"]["stored"][0]["state"] = "challenged"
            m._save_state(st2)
            check("P0-2 canonical 저장이 데몬 state.json 미변경(단일 writer)",
                  before == open(os.path.join(learn, "state.json")).read())
        finally:
            m._is_canonical = old_canon
            if old_env is None:
                os.environ.pop("CYS_ROUND_DIR", None)
            else:
                os.environ["CYS_ROUND_DIR"] = old_env

    # C1: TTL 자동 계산 prov=+90d·conf=+180d.
    t = date(2026, 7, 17)
    f_prov = m.new_item_fields("provisional", [], today=t)
    f_conf = m.new_item_fields("confirmed", ["a.md"], today=t)
    check("C1 prov expires=+90d", f_prov["expires"] == (t + timedelta(days=90)).isoformat())
    check("C1 conf expires=+180d", f_conf["expires"] == (t + timedelta(days=180)).isoformat())
    check("C1 v2 필드 전부 존재",
          all(k in f_prov for k in ("state", "expires", "review_due", "reval_count", "refs",
                                    "effect_log", "challenge")) and f_prov["challenge"] is None)

    # C7: v2 후보 검증 + normalized 전 필드 보존(4키 소거 함정 수리).
    ok = m.validate_candidates(v2_candidates())
    check("C7 v2 후보 정상", ok["ok"], str(ok["errors"]))
    check("C7 normalized 전 필드 보존",
          ok["normalized"][0].get("first_seen") == "2024-05-01"
          and ok["normalized"][0].get("counterquery_log")
          and ok["normalized"][0].get("known_failures") is not None, str(ok["normalized"][0]))
    bad = v2_candidates()
    bad[0]["known_failures"] = [{"source_url": "https://ex.org/p", "summary": "해시 없음"}]
    check("C7 known_failures 해시 누락 거부", not m.validate_candidates(bad)["ok"])
    bad = v2_candidates()
    del bad[0]["counterquery_log"]
    check("C7 counterquery_log 부재 거부", not m.validate_candidates(bad)["ok"])
    bad = v2_candidates()
    del bad[0]["first_seen"]
    check("C7 first_seen 부재 거부", not m.validate_candidates(bad)["ok"])
    v1 = [{"source_url": "https://w3.org/spec", "claim": "X", "retrieved_at": "2026-07-17"}]
    check("C7 v1 후보 관용(후방 호환)", m.validate_candidates(v1)["ok"])

    # C8: pattern v2.
    check("C8 v1 pattern 관용", m.validate_pattern_v2(v1_pattern())["ok"])
    check("C8 v2 pattern 정상", m.validate_pattern_v2(v2_pattern())["ok"])
    p = v2_pattern(); del p["falsifier"]
    check("C8 falsifier 누락 거부", not m.validate_pattern_v2(p)["ok"])
    p = v2_pattern(); p["maturity"] = {"first_seen": "2024"}
    check("C8 maturity 불완전 거부", not m.validate_pattern_v2(p)["ok"])

    # C3: 마커 타입별 문법(순수 파일 검사).
    with tempfile.TemporaryDirectory(prefix="cys-marker-") as d:
        md = os.path.join(d, "x.md"); open(md, "w").write("본문\n<!-- learn:idA -->\n")
        py = os.path.join(d, "x.py"); open(py, "w").write("# learn:idA\ncode=1\n")
        js = os.path.join(d, "x.json"); write(js, {"_learn_refs": ["idA"], "k": 1})
        check("C3 md 마커", m.marker_present(md, "idA") and not m.marker_present(md, "idB"))
        check("C3 py 마커", m.marker_present(py, "idA") and not m.marker_present(py, "idB"))
        check("C3 json 마커", m.marker_present(js, "idA") and not m.marker_present(js, "idB"))
        check("C3 부재 파일=False", not m.marker_present(os.path.join(d, "nope.md"), "idA"))

    # C5: 트리 해시 — 파일 1바이트 변경=다른 해시·부재=fail.
    with tempfile.TemporaryDirectory(prefix="cys-evh-") as d:
        c1 = os.path.join(d, "launcher.sh"); open(c1, "w").write("run v1\n")
        c2 = os.path.join(d, "prompt.md"); open(c2, "w").write("prompt v1\n")
        h1, miss = m.evaluator_tree_hash([c1, c2])
        check("C5 트리 해시 산출", h1 and miss is None)
        check("C5 순서 무관(정렬 연접)", m.evaluator_tree_hash([c2, c1])[0] == h1)
        open(c2, "a").write("!")
        h2, _ = m.evaluator_tree_hash([c1, c2])
        check("C5 1바이트 변경=다른 해시", h2 != h1)
        check("C5 component 부재=fail", m.evaluator_tree_hash([c1, os.path.join(d, "gone")])[0] is None)

    # C9: sample_audit 시드 결정론(재현 가능).
    s1 = m.sample_audit_flag("R/x")
    check("C9 시드 결정론", s1 == m.sample_audit_flag("R/x") and isinstance(s1[1], bool))

    # C6: content_sha256 결정론.
    led = {"tasks": ["t"], "success_criteria": "s", "aux_metrics_protocol": {"분모": "d"}}
    check("C6 content_sha256 결정론",
          m.freeze_content_sha(led) == m.freeze_content_sha(dict(led, extra="무시")))


# ───────────────────────── ENV A: store/refs/conflict/challenge/audit ─────────────────────────

def env_a():
    today = date.today()
    with tempfile.TemporaryDirectory(prefix="cys-gaps-a-") as d:
        make_env(d)
        cand = os.path.join(d, "cands.json"); write(cand, v2_candidates())
        patv1 = os.path.join(d, "pat_v1.json"); write(patv1, v1_pattern())
        patv2 = os.path.join(d, "pat_v2.json"); write(patv2, v2_pattern())
        gi_prov = os.path.join(d, "gi_prov.json"); write(gi_prov, gi_prov_bundle())
        snap = os.path.join(d, "snapshot.txt")
        open(snap, "w", encoding="utf-8").write("hello canonical world quote-here")
        sha = hashlib.sha256(open(snap, "rb").read()).hexdigest()
        # gi_conf=conflict_audit 제거(부재 차단 검증용) · gi_conf_audit=포함(통과 검증용).
        gi_conf = os.path.join(d, "gi_conf.json")
        b_noaudit = gi_conf_bundle(snap, sha); b_noaudit.pop("conflict_audit", None)
        write(gi_conf, b_noaudit)
        gi_conf_audit = os.path.join(d, "gi_conf_audit.json"); write(gi_conf_audit, gi_conf_bundle(snap, sha))

        # C7 search 서브프로세스 exit code.
        r = run_learn(["search", "--topic", "T", "--candidates", cand, "--json"], d)
        check("C7 search v2 정상(rc0)", r.returncode == 0, r.stderr)
        check("C7 search normalized 전 필드 보존(stdout)", '"first_seen"' in r.stdout, r.stdout[:300])
        bad = os.path.join(d, "bad.json")
        b2 = v2_candidates(); b2[0]["known_failures"] = [{"source_url": "https://e/p", "summary": "s"}]
        write(bad, b2)
        r = run_learn(["search", "--topic", "T", "--candidates", bad], d)
        check("C7 search 해시 누락 거부(rc2)", r.returncode == 2, f"rc={r.returncode}")
        b2 = v2_candidates(); del b2[1]["adoption_evidence"]
        write(bad, b2)
        r = run_learn(["search", "--topic", "T", "--candidates", bad], d)
        check("C7 search adoption_evidence 부재 거부(rc2)", r.returncode == 2, f"rc={r.returncode}")

        r = improved_round(d, "R1")
        check("evaluate improved 준비(rc0)", r.returncode == 0, r.stderr)

        # C8 v2 store 필드 누락 거부.
        p = v2_pattern(); del p["maturity"]
        pbad = os.path.join(d, "pat_bad.json"); write(pbad, p)
        r = run_learn(["store", "--round", "R1", "--pattern", pbad, "--type", "reference",
                       "--gate-input", gi_prov, "--name", "gapx-badv2"], d)
        check("C8 v2 store 필드 누락 거부(rc2)", r.returncode == 2, f"rc={r.returncode} {r.stderr}")

        # C3 마커 — 3타입 실존 검증 + 부재=exit 3.
        md = os.path.join(d, "impl.md"); open(md, "w").write("반영\n<!-- learn:gapx-refs -->\n")
        py = os.path.join(d, "impl.py"); open(py, "w").write("# learn:gapx-refs\nx=1\n")
        js = os.path.join(d, "impl.json"); write(js, {"_learn_refs": ["gapx-refs"]})
        nomark = os.path.join(d, "nomark.md"); open(nomark, "w").write("마커 없음\n")
        r = run_learn(["store", "--round", "R1", "--pattern", patv2, "--type", "reference",
                       "--gate-input", gi_prov, "--name", "gapx-refs",
                       "--refs", f"{md},{nomark}"], d)
        check("C3 store 마커 누락 거부(rc3)", r.returncode == 3, f"rc={r.returncode} {r.stderr}")
        r = run_learn(["store", "--round", "R1", "--pattern", patv2, "--type", "reference",
                       "--gate-input", gi_prov, "--name", "gapx-refs",
                       "--refs", f"{md},{py},{js}", "--json"], d)
        check("C3 store 마커 3타입 실존 통과(rc0)", r.returncode == 0, r.stdout + r.stderr)
        st = load_state(d)
        item = find_item(st, "gapx-refs")
        check("C1 신규 레코드 v2 필드 기록",
              item and item.get("expires") == (today + timedelta(days=90)).isoformat()
              and item.get("reval_count") == 0 and item.get("refs") == [md, py, js]
              and item.get("effect_log") == [] and item.get("challenge") is None, str(item))

        # C3 harness 마커 누락 거부 + C1 harness 레코드.
        r = run_learn(["harness", "--round", "R1", "--pattern", patv1,
                       "--evolve", "gapx-harness", "--refs", nomark], d)
        check("C3 harness 마커 누락 거부(rc3)", r.returncode == 3, f"rc={r.returncode}")
        hm = os.path.join(d, "hmark.md"); open(hm, "w").write("<!-- learn:gapx-harness -->\n")
        r = run_learn(["harness", "--round", "R1", "--pattern", patv1, "--evolve", "gapx-harness",
                       "--refs", hm, "--gate-input", gi_prov, "--json"], d)
        check("C3 harness 마커 통과(rc0)", r.returncode == 0, r.stdout + r.stderr)
        item = find_item(load_state(d), "gapx-harness")
        check("C1 harness 레코드 v2 필드", item and item.get("expires") and item.get("state") == "provisional",
              str(item))

        # C9 conflictscan — 0건=시드 기록 sample_audit 플래그.
        r = run_learn(["conflictscan", "--pattern", patv1, "--round", "R1", "--name", "n0", "--json"], d)
        out = json.loads(r.stdout)
        check("C9 conflictscan 0건 sample_audit 플래그(rc0)",
              r.returncode == 0 and out["count"] == 0 and "sample_audit" in out
              and "sample_audit_seed" in out, r.stdout[:300])

        # C9 정면충돌 fixture — 규범 코퍼스(_round/*.md)에 충돌 라인 심기.
        norms = os.path.join(d, "_round", "NORMS.md")
        open(norms, "w", encoding="utf-8").write(
            "# 규범\n\npolling interval increase 금지 — idle load 무관 절대 불가\n")
        r = run_learn(["conflictscan", "--pattern", patv2, "--json"], d)
        out = json.loads(r.stdout)
        check("C9 정면충돌 후보 검출", r.returncode == 0 and out["count"] >= 1, r.stdout[:300])
        # confirmed 승격: conflict_audit verdict 부재=차단 · 존재=통과.
        r = run_learn(["store", "--round", "R1", "--pattern", patv2, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf,
                       "--name", "gapx-conf-noaudit"], d)
        check("C9 정면충돌+confirmed conflict_audit 부재 차단(rc2)", r.returncode == 2,
              f"rc={r.returncode} {r.stderr}")
        r = run_learn(["store", "--round", "R1", "--pattern", patv2, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf_audit,
                       "--name", "gapx-conf-audited", "--json"], d)
        # F58 §3.4: conflict_audit 를 통과한 confirmed 도 자동 채택하지 않는다 — 보류(staged · rc8 · 기억 무기록).
        check("F58 conflict_audit 동봉 confirmed=보류(rc8·기억 무기록)",
              r.returncode == 8 and '"staged"' in r.stdout
              and not os.path.exists(os.path.join(d, "pack", "memory", "reference_gapx-conf-audited.md")),
              r.stdout + r.stderr)
        check("C1 confirmed expires=+180d(순수 new_item_fields)",
              load_module().new_item_fields("confirmed", [], today)["expires"]
              == (today + timedelta(days=180)).isoformat())
        # 아래 C11(effect_log·레짐) 시험은 confirmed 레코드 하나를 전제한다 — 0.14.57 이전에 저장된 레코드처럼 심는다.
        st = load_state(d)
        rec = {"name": "gapx-conf-audited", "id": "gapx-conf-audited", "type": "reference", "ts": 0}
        rec.update(load_module().new_item_fields("confirmed", [], today))
        st["rounds"]["R1"].setdefault("stored", []).append(rec)
        save_state(d, st)

        # C4 challenge — 프리스크린(exit 4)·쿨다운(exit 5)·효력 유지·upheld=tombstone+스윕.
        # ★P0-4 위조 봉쇄 — snapshot_path 실 파일 해시 대조 + quote substring 정박.
        snap = os.path.join(d, "snap.txt")
        open(snap, "w", encoding="utf-8").write("반증 근거 본문: polling 무관 사례 관측됨 q\n")
        snap_hash = hashlib.sha256(open(snap, "rb").read()).hexdigest()
        ev_bad = os.path.join(d, "ev_bad.json")
        write(ev_bad, {"id": "gapx-refs", "reason": "반증",
                       "evidence": [{"source_url": "https://e/x", "quote": "q"}]})  # 해시 부재
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_bad], d)
        check("C4 프리스크린 거부(rc4)", r.returncode == 4, f"rc={r.returncode}")
        # ★P0-4 위조 해시(형식만 맞고 실 파일 불일치)=거부(rc4).
        ev_forge = os.path.join(d, "ev_forge.json")
        write(ev_forge, {"id": "gapx-refs", "reason": "위조 시도",
                         "evidence": [{"source_url": "https://e/x", "snapshot_path": snap,
                                       "snapshot_sha256": SHA_A, "quote": "q"}]})
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_forge], d)
        check("P0-4 위조 해시 거부(rc4)", r.returncode == 4, f"rc={r.returncode} {r.stderr}")
        ev_ok = os.path.join(d, "ev_ok.json")
        write(ev_ok, {"id": "gapx-refs", "reason": "반증 근거 확보",
                      "evidence": [{"source_url": "https://e/x", "snapshot_path": snap,
                                    "snapshot_sha256": snap_hash, "quote": "q"}]})
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_ok, "--json"], d)
        check("C4 challenge open(rc0)", r.returncode == 0, r.stdout + r.stderr)
        item = find_item(load_state(d), "gapx-refs")
        check("C4 challenged 상태 전이(효력 유지 명문)",
              item.get("state") == "challenged" and item["challenge"]["status"] == "open", str(item))
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_ok], d)
        check("C4 open 중복 거부(rc5)", r.returncode == 5, f"rc={r.returncode}")
        r = run_learn(["challenge", "--id", "gapx-refs", "--resolve", "rejected", "--json"], d)
        check("C4 resolve rejected(rc0·이전 상태 복귀)",
              r.returncode == 0 and find_item(load_state(d), "gapx-refs").get("state") == "provisional",
              r.stdout + r.stderr)
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_ok], d)
        check("C4 쿨다운 14d 거부(rc5)", r.returncode == 5, f"rc={r.returncode}")
        # 쿨다운 경과 시뮬레이션(직전 challenge 일자 20d 전으로) → 재탄핵 → upheld.
        st = load_state(d)
        find_item(st, "gapx-refs")["challenge"]["date"] = (today - timedelta(days=20)).isoformat()
        save_state(d, st)
        r = run_learn(["challenge", "--id", "gapx-refs", "--evidence", ev_ok], d)
        check("C4 쿨다운 경과 후 재탄핵(rc0)", r.returncode == 0, r.stderr)
        # ★P0-4 파괴 출구 fail-closed — upheld는 --approved 없으면 거부(rc2).
        r = run_learn(["challenge", "--id", "gapx-refs", "--resolve", "upheld"], d)
        check("P0-4 upheld --approved 부재 거부(rc2)", r.returncode == 2, f"rc={r.returncode}")
        r = run_learn(["challenge", "--id", "gapx-refs", "--resolve", "upheld", "--approved", "--json"], d)
        out = json.loads(r.stdout)
        item = find_item(load_state(d), "gapx-refs")
        check("C4 upheld=tombstone(soft)+refs 스윕 출력",
              r.returncode == 0 and item.get("state") == "tombstone"
              and sorted(out.get("refs_sweep", [])) == sorted([md, py, js]), r.stdout[:400])
        check("C4 미존재 id 거부(rc2)",
              run_learn(["challenge", "--id", "ghost", "--evidence", ev_ok], d).returncode == 2)

        # C11 ③ refs 양방향 — 레코드에 있는데 마커 없음=hard-fail(exit 1).
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        check("C11 refs 정합 시 hard-fail 없음",
              not [x for x in rep["refs_hard_fail"] if x["id"] == "gapx-harness"], r.stdout[:300])
        open(hm, "w").write("마커 삭제됨(스윕/실수)\n")
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        check("C11 refs 마커 소실=hard-fail(rc1)",
              r.returncode == 1 and any(x["id"] == "gapx-harness" for x in rep["refs_hard_fail"]),
              r.stdout[:400])

        # C11 ④ effect_log none 2연속 보고 + ⑤ 체인 대조(직전 승·최초 대비 하락).
        st = load_state(d)
        find_item(st, "gapx-conf-audited")["effect_log"] = [
            {"date": "2026-07-01", "effect": "none"}, {"date": "2026-07-10", "effect": "none"}]
        save_state(d, st)
        # ★P1-2: confirmed 존재 후 레짐 활성 — 새 라운드 RC는 freeze 선행 필수.
        benchc = os.path.join(d, "benchc.json")
        write(benchc, {"tasks": ["t"], "success_criteria": "s", "aux_metrics_protocol": {"m": "def"}})
        run_learn(["freeze", "--round", "RC", "--benchmarks", benchc, "--proposer", "worker",
                   "--auditor-sig", "reviewer2:s"], d)
        run_learn(["evaluate", "--round", "RC", "--score", "0.90", "--baseline"], d)
        run_learn(["evaluate", "--round", "RC", "--score", "0.70"], d)
        run_learn(["evaluate", "--round", "RC", "--score", "0.75"], d)
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        check("C11 effect_log none 2연속 강등 사유 보고",
              any(x["id"] == "gapx-conf-audited" for x in rep["effect_none_streak"]), r.stdout[:400])
        check("C11 체인 대조 hard-fail(직전 승·최초 대비 하락·rc1)",
              r.returncode == 1 and any(x["round"] == "RC" for x in rep["chain_hard_fail"]),
              r.stdout[:400])
        # ★P1-5 sample_audit 소비 배선 — 20% 플래그는 시드 결정론이라 직접 ledger에 심어 소비 검증.
        with open(os.path.join(d, "_round", "learn", "ledger.jsonl"), "a", encoding="utf-8") as lf:
            lf.write(json.dumps({"event": "conflictscan", "round": "RC", "name": "sampx",
                                 "count": 0, "sample_audit": True, "sample_audit_seed": "seed1"}) + "\n")
        rep3 = json.loads(run_learn(["audit", "--json"], d).stdout)
        check("P1-5 sample_audit 소비(audit 표면화·dead signal 해소)",
              any(x.get("name") == "sampx" for x in rep3.get("sample_audits_pending", [])),
              str(rep3.get("sample_audits_pending")))


# ───────────────────────── ENV B: evaluator/attempts/freeze ─────────────────────────

def env_b():
    with tempfile.TemporaryDirectory(prefix="cys-gaps-b-") as d:
        make_env(d)
        # C5 evaluator manifest.
        c1 = os.path.join(d, "launcher.sh"); open(c1, "w").write("run v1\n")
        c2 = os.path.join(d, "prompt.md"); open(c2, "w").write("prompt v1\n")
        man = os.path.join(d, "manifest.json")
        write(man, {"components": [c1, c2], "model_id": "claude-x", "params": {"temperature": 0.0}})
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.9", "--baseline",
                       "--evaluator-manifest", man, "--json"], d)
        check("C5 manifest evaluate(rc0)", r.returncode == 0, r.stderr)
        h1 = load_state(d)["rounds"]["E1"]["evaluator_hash"]
        check("C5 라운드 레코드에 evaluator_hash·manifest 기록",
              h1 and load_state(d)["rounds"]["E1"]["evaluator_manifest"]["model_id"] == "claude-x")
        r = run_learn(["status", "--evaluator-hash", h1, "--json"], d)
        check("C5 status --evaluator-hash 질의", r.returncode == 0 and "E1" in r.stdout, r.stdout[:200])
        open(c2, "a").write("!")  # 프롬프트 1바이트 변경 → 다른 심판
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.95",
                       "--evaluator-manifest", man], d)
        h2 = load_state(d)["rounds"]["E1"]["evaluator_hash"]
        check("C5 해시 변동 검출(1바이트 변경)", r.returncode == 0 and h2 != h1, f"{h1[:8]} vs {h2[:8]}")
        write(man, {"components": [c1, os.path.join(d, "gone.md")], "model_id": "m", "params": {}})
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.96", "--evaluator-manifest", man], d)
        check("C5 component 부재=rc6(fail-closed)", r.returncode == 6, f"rc={r.returncode}")

        # C10 시도 상한 — E1은 이미 2회 → 3회째 정상 → 4회째 ESCALATE.
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.96"], d)
        check("C10 3회째 정상(rc0)", r.returncode == 0, r.stderr)
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.97"], d)
        check("C10 4회째 ESCALATE(rc9)", r.returncode == 9, f"rc={r.returncode}")
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.97", "--owner-approved"], d)
        check("C10 owner-approved인데 responds-to 부재=무응답 재제출 거부(rc2)",
              r.returncode == 2, f"rc={r.returncode}")
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.97", "--owner-approved",
                       "--responds-to", "REVISE-evidence-r3", "--json"], d)
        check("C10 owner-approved+responds-to 속행(rc0)", r.returncode == 0, r.stderr)
        # ★P0-5 시도 상한이 라운드 레코드 attempts에 원자 기록 — ledger 삭제로 우회 불가.
        cap = load_module().EVALUATE_ATTEMPT_CAP
        check("P0-5 라운드 레코드 attempts 카운터",
              (load_state(d)["rounds"]["E1"].get("attempts") or 0) >= cap)
        open(os.path.join(d, "_round", "learn", "ledger.jsonl"), "w").close()  # ledger 전삭(조작)
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.98"], d)
        check("P0-5 ledger 삭제해도 상한 유지(rc9·조작 우회 봉쇄)", r.returncode == 9, f"rc={r.returncode}")

        # C6 freeze — master 무서명=rc7 · 서명=rc0 · ledger 무결 · 신규 라운드 fail-closed.
        bench = os.path.join(d, "bench.json")
        write(bench, {"tasks": ["t1"], "success_criteria": "기준",
                      "aux_metrics_protocol": {"결함발견률": "분모=검증건·분자=결함·수집=라운드 종료"}})
        r = run_learn(["freeze", "--round", "FZ", "--benchmarks", bench, "--proposer", "master"], d)
        check("C6 master-freeze 무서명 거부(rc7)", r.returncode == 7, f"rc={r.returncode}")
        fz_path = os.path.join(d, "_round", "learn", "benchmarks", "FZ.json")
        check("C6 무서명 거부 시 ledger 미생성", not os.path.exists(fz_path))
        r = run_learn(["freeze", "--round", "FZ", "--benchmarks", bench, "--proposer", "master",
                       "--auditor-sig", "reviewer2:sig-abc", "--json"], d)
        led = json.load(open(fz_path))
        check("C6 freeze ledger 기록(content_sha256 포함)",
              r.returncode == 0 and led.get("content_sha256") and led.get("auditor_sig"),
              r.stdout + r.stderr)
        r = run_learn(["freeze", "--round", "FZ", "--benchmarks", bench, "--proposer", "worker",
                       "--auditor-sig", "reviewer2:s"], d)
        check("C6 재동결 금지(rc2)", r.returncode == 2, f"rc={r.returncode}")
        # ★P1-1 자기신고 회피 봉쇄 — worker 신고여도 auditor_sig 무조건 필수(무서명=rc7).
        r = run_learn(["freeze", "--round", "FZ3", "--benchmarks", bench, "--proposer", "worker"], d)
        check("P1-1 proposer 무관 무서명 거부(rc7)", r.returncode == 7, f"rc={r.returncode}")
        r = run_learn(["freeze", "--round", "FZ3", "--benchmarks", bench, "--proposer", "worker",
                       "--auditor-sig", "reviewer2:s3"], d)
        check("C6 auditor_sig 동봉 freeze 통과(rc0)", r.returncode == 0, r.stderr)
        # 레짐 활성 후: freeze 있는 신규 라운드=통과 · 없는 신규 라운드=rc8 · 구 라운드=면제.
        r = run_learn(["evaluate", "--round", "FZ", "--score", "0.8", "--baseline"], d)
        check("C6 freeze 존재 신규 라운드 evaluate(rc0)", r.returncode == 0, r.stderr)
        r = run_learn(["evaluate", "--round", "NEWR", "--score", "0.8", "--baseline"], d)
        check("C6 freeze 부재 신규 라운드 evaluate 거부(rc8)", r.returncode == 8, f"rc={r.returncode}")
        r = run_learn(["evaluate", "--round", "E1", "--score", "0.98", "--owner-approved",
                       "--responds-to", "REVISE-evidence-r4"], d)
        check("C6 구 라운드(레짐 이전) 면제(rc0)", r.returncode == 0, r.stderr)
        led["tasks"] = ["t1", "몰래 추가"]  # 사후 변조 — content_sha256 불일치.
        write(fz_path, led)
        r = run_learn(["evaluate", "--round", "FZ", "--score", "0.85"], d)
        check("C6 freeze 해시 무결 위반 거부(rc8)", r.returncode == 8, f"rc={r.returncode}")


# ───────────────────────── ENV C: TTL audit(C11·C1) ─────────────────────────

def env_c():
    today = date.today()
    with tempfile.TemporaryDirectory(prefix="cys-gaps-c-") as d:
        make_env(d)
        patv1 = os.path.join(d, "pat.json"); write(patv1, v1_pattern())
        patv2 = os.path.join(d, "patv2.json"); write(patv2, v2_pattern())
        gi_prov = os.path.join(d, "gi_prov.json"); write(gi_prov, gi_prov_bundle())
        snap = os.path.join(d, "snapshot.txt")
        open(snap, "w", encoding="utf-8").write("hello canonical world quote-here")
        sha = hashlib.sha256(open(snap, "rb").read()).hexdigest()
        gi_conf = os.path.join(d, "gi_conf.json"); write(gi_conf, gi_conf_bundle(snap, sha))
        improved_round(d, "R1")
        r = run_learn(["store", "--round", "R1", "--pattern", patv1, "--type", "reference",
                       "--gate-input", gi_prov, "--name", "gapc-prov"], d)
        check("ENV C prov store(rc0)", r.returncode == 0, r.stderr)
        # ★P0-1 오답 핀 반전 — v1 pattern의 confirmed 승격은 거부(provisional 상한).
        r = run_learn(["store", "--round", "R1", "--pattern", patv1, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf,
                       "--name", "gapc-conf-v1"], d)
        check("P0-1 v1 pattern confirmed 승격 거부(rc2)", r.returncode == 2, f"rc={r.returncode} {r.stderr}")
        # confirmed는 v2 pattern + conflict_audit 무조건 필수.
        r = run_learn(["store", "--round", "R1", "--pattern", patv2, "--type", "reference",
                       "--state", "confirmed", "--gate-input", gi_conf,
                       "--name", "gapc-conf"], d)
        check("F58 ENV C conf store=보류(rc8 · confirmed 는 자동 채택 안 함)", r.returncode == 8, r.stdout + r.stderr)
        # C11 수명(만기·reval·lapse)은 0.14.57 이전에 저장된 confirmed 레코드에도 계속 적용된다 —
        #   그 레코드를 learn_state 에 심어 같은 수명 경로를 계속 시험한다(기억 파일은 수명 판정에 쓰이지 않는다).
        st = load_state(d)
        rec = {"name": "gapc-conf", "id": "gapc-conf", "type": "reference", "ts": 0}
        rec.update(load_module().new_item_fields("confirmed", [], today))
        st["rounds"]["R1"].setdefault("stored", []).append(rec)
        save_state(d, st)

        # C11 ① prov 만기=tombstone · conf 만기(유예 내)=wakeup enqueue.
        st = load_state(d)
        find_item(st, "gapc-prov")["expires"] = (today - timedelta(days=1)).isoformat()
        find_item(st, "gapc-conf")["expires"] = (today - timedelta(days=10)).isoformat()
        save_state(d, st)
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        st = load_state(d)
        check("C11 provisional 만기=tombstone(rc0)",
              r.returncode == 0 and find_item(st, "gapc-prov")["state"] == "tombstone"
              and any(x["id"] == "gapc-prov" for x in rep["expired_tombstoned"]), r.stdout[:400])
        check("C11 confirmed 만기(유예 내)=reval enqueue",
              any(x["id"] == "gapc-conf" and x["enqueue_rc"] == 0 for x in rep["reval_enqueued"]),
              r.stdout[:400])
        pend = os.path.join(d, "_round", "wakeups", "pending", "master__learn-reval-gapc-conf.json")
        check("C11 wakeup pending 파일 생성(javis_wakeup 실배선)", os.path.exists(pend), pend)
        check("C11 confirmed 유예 내 강등 없음", find_item(st, "gapc-conf")["state"] == "confirmed")

        # G1 full-recheck 판정 — reval_count=0(2의 배수)=full 의무를 reason으로 결정론 전달.
        check("C11 full-recheck 의무 reason(reval_count=0)",
              any(x["id"] == "gapc-conf" and x.get("reason") == "ttl-expired-full-recheck"
                  and x.get("full_recheck") is True for x in rep["reval_enqueued"]), r.stdout[:400])
        r = run_learn(["audit", "--mark-revaled", "gapc-conf"], d)
        check("C11 full 의무 회차 --full 부재=rc2(의무 회피 불가)", r.returncode == 2, f"rc={r.returncode}")
        r = run_learn(["audit", "--mark-revaled", "gapc-conf", "--full", "--json"], d)
        it = find_item(load_state(d), "gapc-conf")
        check("C11 mark-revaled --full: reval_count+1·expires +180d 재계산",
              r.returncode == 0 and it["reval_count"] == 1 and it["review_due"] == it["expires"]
              and it["expires"] == (today + timedelta(days=180)).isoformat(), r.stdout + r.stderr)
        st = load_state(d)
        find_item(st, "gapc-conf")["expires"] = (today - timedelta(days=5)).isoformat()
        save_state(d, st)
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        check("C11 경량 회차 reason=ttl-expired(reval_count=1 홀수)",
              any(x["id"] == "gapc-conf" and x.get("reason") == "ttl-expired"
                  and x.get("full_recheck") is False for x in rep["reval_enqueued"]), r.stdout[:400])
        r = run_learn(["audit", "--mark-revaled", "gapc-conf"], d)
        check("C11 경량 회차 mark-revaled(--full 불요·rc0)",
              r.returncode == 0 and find_item(load_state(d), "gapc-conf")["reval_count"] == 2, r.stderr)
        check("C11 mark-revaled 미존재 id=rc2",
              run_learn(["audit", "--mark-revaled", "ghost"], d).returncode == 2)
        check("C11 mark-revaled 비confirmed=rc2",
              run_learn(["audit", "--mark-revaled", "gapc-prov"], d).returncode == 2)

        # ★P1-4 effect_log writer — 기록 수단(audit --record-effect) 신설(dead consumer 해소).
        r = run_learn(["audit", "--record-effect", "gapc-conf", "--effect", "none", "--json"], d)
        check("P1-4 record-effect 기록(rc0)", r.returncode == 0, r.stderr)
        r = run_learn(["audit", "--record-effect", "gapc-conf", "--effect", "none"], d)
        el = find_item(load_state(d), "gapc-conf").get("effect_log") or []
        check("P1-4 effect_log 2건 축적", len(el) == 2 and el[-1]["effect"] == "none", str(el))
        r = run_learn(["audit", "--json"], d)
        rep2 = json.loads(r.stdout)
        check("P1-4 effect none 2연속=강등 사유 보고(ROI 축)",
              any(x["id"] == "gapc-conf" for x in rep2["effect_none_streak"]), r.stdout[:400])

        # C11 ② lapse — conf 만기+30d 초과=자동 provisional 강등(보수 방향).
        st = load_state(d)
        find_item(st, "gapc-conf")["expires"] = (today - timedelta(days=40)).isoformat()
        save_state(d, st)
        r = run_learn(["audit", "--json"], d)
        rep = json.loads(r.stdout)
        st = load_state(d)
        check("C11 lapse 강등(rc0·confirmed→provisional)",
              r.returncode == 0 and find_item(st, "gapc-conf")["state"] == "provisional"
              and any(x["id"] == "gapc-conf" for x in rep["lapsed"]), r.stdout[:400])
        # 다음 실행에서 강등된 provisional의 만기 처리(전이 1회/실행 — 보수 캐스케이드).
        r = run_learn(["audit", "--json"], d)
        check("C11 강등 후 다음 실행에서 tombstone(캐스케이드)",
              r.returncode == 0 and find_item(load_state(d), "gapc-conf")["state"] == "tombstone",
              r.stdout[:400])


# ───────────────────────── G5: fleet_report 학습 결과 지표 1행 ─────────────────────────

def fleet_tests():
    fleet_py = os.path.join(ROOT, "cysjavis-pack", "bin", "javis_fleet_report.py")
    spec = importlib.util.spec_from_file_location("fleetrep", fleet_py)
    fr = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fr)
    with tempfile.TemporaryDirectory(prefix="cys-fleet-") as d:
        rd = os.path.join(d, "_round")
        os.makedirs(os.path.join(rd, "learn"), exist_ok=True)
        st = {"rounds": {"R1": {
            "stored": [
                {"name": "a", "state": "confirmed", "ts": 1752700000.0,
                 "effect_log": [{"effect": "improved"}, {"effect": "none"}]},
                {"name": "b", "state": "provisional", "ts": 1752710000.0, "effect_log": []},
                {"name": "c", "state": "tombstone", "ts": 1752720000.0}],
            "harness": [{"harness_ref": "h", "state": "challenged", "ts": 1752730000.0}]}}}
        write(os.path.join(rd, "learn", "learn_state.json"), st)
        ls = fr.learn_summary(rd)
        check("G5 learn_summary 결과 계수(채택 3·tombstone 1·effects)",
              ls and ls["adopted"] == 3 and ls["by_state"]["tombstone"] == 1
              and ls["effects"] == {"improved": 1, "none": 1} and ls["last_episode"], str(ls))
        rep = {"days_window": 7, "dbs_scanned": [], "dbs_skipped": [],
               "fleet": {"tools": [], "subagents": [], "cost_usd_window": 0.0,
                         "cache_hit_ratio_pct": 0.0, "cache_read": 0,
                         "missed_savings": [], "pack_version_advisory": None, "learn": ls}}
        txt = fr.render_digest(rep)
        check("G5 digest 학습 1행(결과 지표)",
              "학습(RSI·결과 지표)" in txt and "tombstone 1" in txt and "confirmed 1" in txt, txt)
        check("G5 활동량 지표 금지('추천' 미노출)", "추천" not in txt, txt)
        check("G5 채택>0=재조정 문구 없음", "게이트 비용 재조정" not in txt)
        st0 = {"rounds": {"R1": {"stored": [{"name": "c", "state": "tombstone", "ts": 1.0}],
                                 "harness": []}}}
        write(os.path.join(rd, "learn", "learn_state.json"), st0)
        ls0 = fr.learn_summary(rd)
        rep["fleet"]["learn"] = ls0
        txt0 = fr.render_digest(rep)
        check("G5 채택 0건='게이트 비용 재조정 검토' 트리거 문구",
              ls0 and ls0["gate_cost_review"] and "게이트 비용 재조정 검토" in txt0, txt0)
        check("G5 learn 상태 부재=None(행 생략 관용)",
              fr.learn_summary(os.path.join(d, "nope")) is None)
        rep["fleet"]["learn"] = None
        check("G5 learn=None digest 행 생략", "학습(RSI" not in fr.render_digest(rep))


def main():
    pure_tests()
    env_a()
    env_b()
    env_c()
    fleet_tests()
    print()
    if FAIL:
        print(f"❌ {len(FAIL)} FAIL: {FAIL}")
        return 1
    print("✅ 전 항목 PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
