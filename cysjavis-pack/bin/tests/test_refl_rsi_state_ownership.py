#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_rsi_state_ownership.py — 성찰 R4 N1·N7·N10·N14: RSI 상태 쓰기의 소유권·경합·불확정.

무엇을 막는가
  ① N1 [blocking] RSI 미러가 **데몬 canonical 학습 `state.json` 을 통째로 치환**했다.
     상시 잡이 `CYS_ROUND_DIR="${CYS_ROUND_DIR:-$HOME/.cys/state}"` 를 박으면 미러 대상이 데몬
     canonical 이 된다. `javis_learn` 은 그 형상에서 미러 쓰기를 끊는데(`_is_canonical`) RSI 는
     같은 프로세스 트리에서 덮었고, 데몬이 `daemon.learn_write` 락으로 지키던 lifecycle 라운드
     (verdict·stored·harness·evaluator_hash)와 discovery 계수가 **전손**됐다(자가치유 상태 전손 ·
     §7 위험 ③). 프로세스 밖 `os.replace` 는 데몬 락이 막지 못한다.
     비canonical 자리에도 **남의 라운드**가 있다(`javis_learn` 세션 모드 미러가 같은 파일을 쓴다).
  ② N7 [major] ceiling 추천 래치 저장이 **새 checkpoint 를 되돌렸다** — A 가 progress 로 H1 을
     읽고, 그 사이 B 가 H2 checkpoint 를 저장하고, A 가 래치 두 필드를 세우려고 H1 **전체**를
     재저장하면 rollback 앵커가 옛 커밋을 가리킨다(되돌리기 사고 방향).
  ③ N10 [major] 같은 "다이제스트 큐" 를 두 모듈이 다른 규칙으로 해소했다 — orchestra 는
     `pack_dir()/round/learn`, rsi 는 `CYS_ROUND_DIR/learn` 우선(+ 팩 env 를 1키만 인식).
     귀결: RSI ceiling 추천이 **아무도 읽지 않는 파일**에 쌓이고 멱등 조회도 갈린다.
  ④ N14 [minor] 비유한 예산값이 명령을 죽였다(`attempts: 1e999` → `int(inf)` OverflowError) ·
     보조(래치) 저장 실패가 **주 평가의 rc** 를 바꿨다(끝난 평가가 fail(12) 재시도로 되돌아간다).

밀폐: `HOME`·`CYS_ROUND_DIR`·`CYS_PACK_DIR` 을 임시 디렉터리로 고정한다 — 오너 홈의 `~/.cys` 와
      라이브 데몬은 **읽지도 쓰지도 않는다**(canonical 판정도 임시 HOME 아래에서 재현한다).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-RSI-STATE-OWNERSHIP-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_rsi_state_ownership.py
"""
import argparse
import json
import os
import shutil
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)

_ROOT = tempfile.mkdtemp(prefix="refl-rsi-")
os.environ["HOME"] = _ROOT                      # canonical(`~/.cys/state`)을 임시 홈으로 옮긴다
os.environ["CYS_PACK_DIR"] = os.path.join(_ROOT, "pack")
os.environ["CYS_ROUND_DIR"] = os.path.join(_ROOT, "round")
for _k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR", "CYS_SOCKET"):
    os.environ.pop(_k, None)

import javis_rsi as R                    # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def write_json(p, obj):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=2)


def read_json(p):
    with open(p, encoding="utf-8") as f:
        return json.load(f)


# 데몬 canonical 학습 상태의 **실제 모양**(handlers.rs learn_checkpoint_apply 가 쓰는 키들)
def canon_seed():
    return {
        "rounds": {"L1": {"round": "L1", "verdict": "accept",
                          "stored": [{"name": "m1", "state": "kept"}],
                          "harness": ["h1"], "evaluator_hash": "abcd1234",
                          "schema": "v2", "attempts": 2, "created_at": 111.0}},
        "discovery": {"capability": 7, "perspective": 3, "knowledge": 5},
    }


RSI_STATE = {
    "rounds": {"T1": {"round": "T1", "checkpoint_sha": "H1", "ref": "refs/rsi/ckpt/T1",
                      "baseline_score": 1.0, "progress": [], "attempts": 1,
                      "flat_streak": 0, "stop_reason": "open"}},
    "discovery": {"capability": 0, "perspective": 0, "knowledge": 0},
    "current_round": "T1",
}

try:
    # ── ① N1: canonical 자리에는 쓰지 않는다 ─────────────────────────────────────────────
    canon_dir = os.path.join(_ROOT, ".cys", "state", "learn")
    canon_p = os.path.join(canon_dir, "state.json")
    os.environ["CYS_ROUND_DIR"] = os.path.join(_ROOT, ".cys", "state")
    write_json(canon_p, canon_seed())
    before = read_json(canon_p)
    R._mirror_learn_state(RSI_STATE)
    after = read_json(canon_p)
    check("1a canonical 형상에서 미러는 **아무것도 쓰지 않는다**", after == before,
          json.dumps(after, ensure_ascii=False)[:200])
    check("1b L1 lifecycle 레코드 보존(verdict·stored·harness·evaluator_hash)",
          after["rounds"].get("L1", {}).get("verdict") == "accept"
          and after["rounds"]["L1"].get("evaluator_hash") == "abcd1234")
    check("1c discovery 계수 보존(7/3/5)",
          after.get("discovery") == {"capability": 7, "perspective": 3, "knowledge": 5},
          repr(after.get("discovery")))

    # ★codex 설계 비평 1의 반례: **루트는 비canonical인데 `learn` 만 canonical 로 심링크**된 자리.
    #   env 모양(루트)만 보는 가드는 이것을 통과시켜 정본을 그대로 덮는다.
    if hasattr(os, "symlink"):
        alias_root = os.path.join(_ROOT, "alias-round")
        os.makedirs(alias_root, exist_ok=True)
        alias_learn = os.path.join(alias_root, "learn")
        try:
            os.symlink(canon_dir, alias_learn)
            symlink_ok = True
        except (OSError, NotImplementedError):
            symlink_ok = False
        if symlink_ok:
            os.environ["CYS_ROUND_DIR"] = alias_root
            R._mirror_learn_state(RSI_STATE)
            check("1d ★`learn` 만 canonical 로 심링크된 자리도 데몬 것으로 본다(codex 반례)",
                  read_json(canon_p) == before, json.dumps(read_json(canon_p))[:200])
        else:
            check("1d 심링크 반례(이 플랫폼에서 심링크 불가)", True, "SKIP")
    else:
        check("1d 심링크 반례(이 플랫폼에 symlink 없음)", True, "SKIP")

    # ── ② N1: 비canonical 자리는 **병합**이고 남의 필드는 건드리지 않는다 ────────────────
    sess_root = os.path.join(_ROOT, "session")
    os.environ["CYS_ROUND_DIR"] = sess_root
    sess_p = os.path.join(sess_root, "learn", "state.json")
    seed = {"rounds": {"L9": {"round": "L9", "verdict": "reject", "stored": [{"name": "z"}],
                              "harness": ["hh"], "evaluator_hash": "deadbeef", "schema": "v2",
                              "attempts": 3, "created_at": 222.0}},
            "discovery": {"capability": 4, "perspective": 1, "knowledge": 2},
            "extra_top": "보존되어야 한다"}
    write_json(sess_p, seed)
    # RSI 가 **같은 rid** 를 쓰는 형상(learn evaluate 가 같은 --round 를 넘긴다)
    rsi_same = {"rounds": {"L9": {"round": "L9", "checkpoint_sha": "H9",
                                  "ref": "refs/rsi/ckpt/L9", "attempts": 11,
                                  "flat_streak": 2, "progress": [{"score": 1.0}]},
                           "T1": RSI_STATE["rounds"]["T1"]},
                "discovery": {"capability": 0, "perspective": 0, "knowledge": 0}}
    R._mirror_learn_state(rsi_same)
    got = read_json(sess_p)
    l9 = got["rounds"]["L9"]
    check("2a 남의 lifecycle 필드는 그대로다",
          l9.get("verdict") == "reject" and l9.get("evaluator_hash") == "deadbeef"
          and l9.get("harness") == ["hh"] and l9.get("schema") == "v2", json.dumps(l9)[:200])
    check("2b learn 의 `attempts` 를 RSI 시도수가 덮지 않는다(개명은 RSI 입력에만)",
          l9.get("attempts") == 3 and l9.get("rsi_attempts") == 11,
          "attempts=%r rsi_attempts=%r" % (l9.get("attempts"), l9.get("rsi_attempts")))
    check("2c RSI 계측 필드는 같은 레코드에 실린다", l9.get("checkpoint_sha") == "H9")
    check("2d 자기 라운드는 새로 실린다", got["rounds"].get("T1", {}).get("checkpoint_sha") == "H1")
    check("2e ★discovery 는 손대지 않는다(RSI 의 기본 0 이 남의 계수를 덮지 않는다)",
          got.get("discovery") == {"capability": 4, "perspective": 1, "knowledge": 2},
          repr(got.get("discovery")))
    check("2f 미지의 최상위 키도 보존", got.get("extra_top") == "보존되어야 한다")

    # ③ 판독 불가한 미러는 **덮지 않는다**(빈 상태로 접어 저장하면 그것이 새 전손 경로다)
    bad_root = os.path.join(_ROOT, "corrupt")
    os.environ["CYS_ROUND_DIR"] = bad_root
    bad_p = os.path.join(bad_root, "learn", "state.json")
    os.makedirs(os.path.dirname(bad_p), exist_ok=True)
    with open(bad_p, "w", encoding="utf-8") as f:
        f.write("{ 이것은 JSON 이 아니다")
    raw_before = open(bad_p, encoding="utf-8").read()
    R._mirror_learn_state(RSI_STATE)
    check("3a 파손된 미러는 덮지 않는다", open(bad_p, encoding="utf-8").read() == raw_before)
    # 음성 대조: **없던** 자리에는 정상적으로 만든다(미러를 통째로 끈 것이 아니다)
    new_root = os.path.join(_ROOT, "fresh")
    os.environ["CYS_ROUND_DIR"] = new_root
    R._mirror_learn_state(RSI_STATE)
    fresh = read_json(os.path.join(new_root, "learn", "state.json"))
    check("3b 음성 대조 — 빈 자리에는 미러가 정상 생성된다",
          fresh.get("rounds", {}).get("T1", {}).get("checkpoint_sha") == "H1")
    check("3c 음성 대조 — 그 미러에 `discovery` 를 지어내지 않는다", "discovery" not in fresh,
          repr(fresh.get("discovery")))

    # ── ④ N10: 두 모듈의 큐 경로가 **모든 형상에서** 같다 ────────────────────────────────
    import javis_orchestra as O          # noqa: E402
    forms = [
        ("CYS_ROUND_DIR 설정(생산 상시 잡)",
         {"CYS_PACK_DIR": os.path.join(_ROOT, "pack"), "CYS_ROUND_DIR": os.path.join(_ROOT, "st")}),
        ("CYS_ROUND_DIR 미설정", {"CYS_PACK_DIR": os.path.join(_ROOT, "pack")}),
        ("레거시 env 만(AITERM_JARVIS_DIR)", {"AITERM_JARVIS_DIR": os.path.join(_ROOT, "legacy")}),
    ]
    keys = ("CYS_PACK_DIR", "CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR",
            "AITERM_JARVIS_DIR")
    saved = {k: os.environ.get(k) for k in keys}
    try:
        for tag, envs in forms:
            for k in keys:
                os.environ.pop(k, None)
            os.environ.update(envs)
            check("4 큐 경로 동형 — %s" % tag,
                  R.learn_digest_queue_path() == O.learn_digest_queue_path(),
                  "rsi=%s orch=%s" % (R.learn_digest_queue_path(), O.learn_digest_queue_path()))
    finally:
        for k in keys:
            os.environ.pop(k, None)
        for k, v in saved.items():
            if v is not None:
                os.environ[k] = v
    # 기능 핀: orchestra 가 적재한 키를 rsi 가 본다(경로가 같다는 것의 관측 가능한 귀결)
    os.environ["CYS_PACK_DIR"] = os.path.join(_ROOT, "pack")
    os.environ["CYS_ROUND_DIR"] = os.path.join(_ROOT, "st")
    O._recommend_learn_once("gate", "T 라운드 종료", "gate-T-parity")
    # orchestra 는 멱등키에 `orchestra.gate:` 를 접두한다(`_recommend_learn_once`).
    check("4d orchestra 가 적재한 키를 rsi 가 조회한다",
          R.digest_queue_has_key(R.learn_digest_queue_path(), "orchestra.gate:gate-T-parity"),
          R.learn_digest_queue_path())
    check("4e 음성 대조 — 없는 키는 보이지 않는다",
          not R.digest_queue_has_key(R.learn_digest_queue_path(), "orchestra.gate:없는키"))

    # ── ⑤ N7: 추천 래치 저장이 남의 checkpoint 를 되돌리지 않는다 ────────────────────────
    run_root = os.path.join(_ROOT, "run")
    os.environ["CYS_ROUND_DIR"] = run_root
    os.environ["CYS_RSI_CEILING_FLATS"] = "1"       # progress 1회로 ceiling
    os.environ["CYS_RSI_MAX_ROUNDS"] = "99"
    sp = os.path.join(R.rsi_dir(), "state.json")
    write_json(sp, {"rounds": {"T1": {"round": "T1", "checkpoint_sha": "H1",
                                      "ref": "refs/rsi/ckpt/T1", "baseline_score": 1.0,
                                      "progress": [], "attempts": 1, "flat_streak": 0,
                                      "stop_reason": "open"}},
                    "current_round": "T1"})

    # ★결정론 인터리브: 추천(큐 적재)은 **주 저장 뒤 · 래치 저장 앞**에 불린다. 그 자리에서
    #   'B 의 H2 checkpoint 가 도착' 을 재현한다.
    _real_recommend = R._recommend_learn

    def _recommend_then_b(reason, topic, key=None):
        ok = _real_recommend(reason, topic, key)
        cur = read_json(sp)
        cur["rounds"]["T1"]["checkpoint_sha"] = "H2"
        cur["rounds"]["T1"]["ref"] = "refs/rsi/ckpt/T1-b"
        cur["rounds"]["T1"]["attempts"] = 42
        write_json(sp, cur)
        return ok

    R._recommend_learn = _recommend_then_b
    try:
        a = argparse.Namespace(round="T1", score=1.0, note="", tokens_saved=None)
        rc = R.cmd_progress(a)
    finally:
        R._recommend_learn = _real_recommend
    end = read_json(sp)["rounds"]["T1"]
    check("5a progress 는 성공으로 끝난다(주 평가 rc 불변)", rc == 0, "rc=%r" % rc)
    check("5b ★남이 저장한 checkpoint 가 후퇴하지 않는다", end.get("checkpoint_sha") == "H2",
          repr(end.get("checkpoint_sha")))
    check("5c rollback 앵커(ref)도 후퇴하지 않는다", end.get("ref") == "refs/rsi/ckpt/T1-b",
          repr(end.get("ref")))
    check("5d 남의 attempts 도 보존된다", end.get("attempts") == 42, repr(end.get("attempts")))
    check("5e 래치 두 필드는 최신 상태 위에 얹혔다", end.get("ceiling_recommended") is True
          and end.get("ceiling_recommended_key") == R.ceiling_digest_key("T1"),
          repr({k: end.get(k) for k in ("ceiling_recommended", "ceiling_recommended_key")}))

    # ── ⑥ N14: 비유한 예산값 · 보조 저장 실패가 주 평가를 죽이지 않는다 ─────────────────
    v, ok = R._as_int_ok(float("inf"))
    check("6a 비유한 값은 예외가 아니라 '해석 불가'다", (v, ok) == (0, False), repr((v, ok)))
    check("6b 결측은 '해석 불가' 가 아니다(결측은 값이 아니다)", R._as_int_ok(None) == (0, True),
          repr(R._as_int_ok(None)))
    inf_root = os.path.join(_ROOT, "inf")
    os.environ["CYS_ROUND_DIR"] = inf_root
    sp2 = os.path.join(R.rsi_dir(), "state.json")
    os.makedirs(os.path.dirname(sp2), exist_ok=True)
    with open(sp2, "w", encoding="utf-8") as f:      # 1e999 → json 이 inf 로 읽는다
        f.write('{"rounds": {"T2": {"round": "T2", "baseline_score": 1.0, "progress": [], '
                '"attempts": 1e999, "flat_streak": 0}}, "current_round": "T2"}')
    a2 = argparse.Namespace(round="T2", score=1.0, note="", tokens_saved=None)
    try:
        rc2, boom = R.cmd_progress(a2), ""
    except Exception as e:                            # noqa: BLE001
        rc2, boom = None, "%r" % e
    check("6c ★`attempts: 1e999` 가 명령을 죽이지 않는다", rc2 == 0, boom or "rc=%r" % rc2)
    rec2 = read_json(sp2)["rounds"]["T2"]
    check("6d 그 예산은 **불확정**으로 표기된다(0 으로 조용히 접지 않는다)",
          rec2.get("budget_unknown") is True, repr(rec2.get("budget_unknown")))

    aux_root = os.path.join(_ROOT, "aux")
    os.environ["CYS_ROUND_DIR"] = aux_root
    sp3 = os.path.join(R.rsi_dir(), "state.json")
    write_json(sp3, {"rounds": {"T3": {"round": "T3", "checkpoint_sha": "H1",
                                       "baseline_score": 1.0, "progress": [], "attempts": 1,
                                       "flat_streak": 0}}, "current_round": "T3"})
    _real_save = R._save_state
    seen = {"n": 0}

    def _save_then_enospc(st):
        seen["n"] += 1
        if seen["n"] == 1:
            return _real_save(st)                     # 주 저장은 성공
        raise OSError(28, "No space left on device")  # 보조(래치) 저장만 실패

    R._save_state = _save_then_enospc
    try:
        a3 = argparse.Namespace(round="T3", score=1.0, note="", tokens_saved=None)
        rc3 = R.cmd_progress(a3)
    finally:
        R._save_state = _real_save
    check("6e ★보조(래치) 저장 ENOSPC 가 주 평가의 rc 를 바꾸지 않는다", rc3 == 0, "rc=%r" % rc3)
    rec3 = read_json(sp3)["rounds"]["T3"]
    check("6f 주 저장분(progress·attempts)은 그대로 남는다",
          len(rec3.get("progress") or []) == 1 and rec3.get("attempts") == 2, json.dumps(rec3)[:200])
    check("6g 보조 저장이 실패했으므로 래치는 서지 않는다(다음 호출이 다시 시도)",
          rec3.get("ceiling_recommended") is not True, repr(rec3.get("ceiling_recommended")))
finally:
    shutil.rmtree(_ROOT, ignore_errors=True)

print("")
if fails:
    print("FAILED %d: %s" % (len(fails), " · ".join(fails)))
    sys.exit(1)
print("ALL PASS")
print("REFL-RSI-STATE-OWNERSHIP-OK")
