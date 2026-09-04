#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_rsi — RSI(재귀적 자기개선) 라운드 무결성의 결정론 도구 (T7 E7).

soul/CLAUDE의 ★eval-driven 원칙(producer≠evaluator·측정실패 hard fail·삭제 reward-hack 차단)을
기계로 박제한다. master가 "좋아진 것 같다"고 LLM 추론하면 환각 — 진척은 **외부에서 주입된 score**의
산술 비교로만 판정한다(이 스크립트가 유일한 사실). 점수 산출은 이 도구가 하지 않는다(기록·비교만).

명령:
  checkpoint --round <id> [--score F] [--note S]
      라운드 시작 HEAD SHA·기준 score를 _round/rsi/state.json + ledger.jsonl에 기록하고,
      복구 anchor로 refs/rsi/ckpt/<id>를 현재 HEAD에 만든다(비파괴 — git read + update-ref).
  progress --round <id> --score F [--note S]
      score를 그 라운드 checkpoint 기준과 비교 → delta·verdict(improved/regressed/flat) 기록.
  markers [--json]
      git log에서 커밋 trailer `iter-id: N`을 파싱해 RSI 반복 이력을 낸다(read-only).
  rollback --round <id> [--execute] [--force]
      ★기본 dry-run: 버려질 커밋(ckpt..HEAD)·복구 백업 브랜치명·정확한 명령만 출력(실행 0).
      --execute: 먼저 `rsi-abandoned-<id>-<ts>` 브랜치에 현재 HEAD를 박제(retention — 비가역 삭제 차단)
      한 뒤에만 `git reset --hard <ckpt>`. 더티 트리·ckpt가 조상 아님 → --force 없이는 거부.
  status [--json]

★불변: 점수 자체 생성 금지(주입만)·rollback은 백업 ref 없이는 절대 reset 안 함·--execute 없으면 무실행.
사용: python3 javis_rsi.py <cmd> ... · 의존성: 표준 라이브러리 + PATH의 git.
"""
import argparse
import json
import os
import subprocess
import sys
import time

EPS = 1e-9


def rsi_dir():
    root = os.environ.get("CYS_ROUND_DIR")
    if root:
        return os.path.join(root, "rsi")
    # 기본: cwd의 _round/rsi
    return os.path.join(os.getcwd(), "_round", "rsi")


def _git(args, cwd=None, check=True):
    """git 호출 — (rc, stdout). check=True면 실패 시 RuntimeError."""
    r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} 실패: {r.stderr.strip()}")
    return r.returncode, r.stdout.strip()


# ───────────────────────── 순수 로직(테스트 핀) ─────────────────────────

def verdict(delta, eps=EPS):
    """score delta → 판정. eps 이내는 flat(노이즈)."""
    if delta > eps:
        return "improved"
    if delta < -eps:
        return "regressed"
    return "flat"


def parse_markers(log_text):
    """`<sha>\\x1f<subject>\\x1f<body>\\x1e` 레코드에서 trailer `iter-id: N` 파싱(read-only).
    반환: [{sha, iter_id(int|None), subject}] (최신순 — git log 순서 보존)."""
    out = []
    for rec in log_text.split("\x1e"):
        rec = rec.strip("\n")
        if not rec:
            continue
        parts = rec.split("\x1f")
        if len(parts) < 2:
            continue
        sha, subject = parts[0].strip(), parts[1].strip()
        body = parts[2] if len(parts) > 2 else ""
        iter_id = None
        for line in body.splitlines():
            s = line.strip().lower()
            if s.startswith("iter-id:"):
                tok = line.split(":", 1)[1].strip()
                try:
                    iter_id = int(tok)
                except ValueError:
                    iter_id = None
                break
        out.append({"sha": sha[:12], "iter_id": iter_id, "subject": subject})
    return out


def rollback_plan(round_id, ckpt_sha, head_sha, discarded, dirty, is_ancestor):
    """rollback 사전 계획(순수) — 무엇을 버리고 어떻게 복구하는지. 실행 0."""
    return {
        "round": round_id,
        "target_sha": ckpt_sha,
        "head_sha": head_sha,
        "discarded_commits": discarded,
        "discarded_count": len(discarded),
        "working_tree_dirty": dirty,
        "target_is_ancestor": is_ancestor,
        "safe": (not dirty) and is_ancestor,
        "blockers": (
            (["working tree dirty (--force 필요)"] if dirty else [])
            + ([] if is_ancestor else ["checkpoint이 HEAD 조상 아님 (--force 필요)"])
        ),
    }


# ───────────────────────── 상태 파일 I/O ─────────────────────────

def _load_state():
    p = os.path.join(rsi_dir(), "state.json")
    try:
        return json.load(open(p, encoding="utf-8"))
    except (OSError, ValueError):
        return {"rounds": {}}


def _learn_state_dir():
    """cysd learn.status가 읽는 위치(handlers.rs learn_state_dir와 동일 규칙) —
    CC 학습 탭 미러링용(B-10: 프로젝트 _round/rsi와 데몬 읽기 경로의 단절 해소)."""
    root = os.environ.get("CYS_ROUND_DIR")
    if root:
        return os.path.join(root, "learn")
    pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
    return os.path.join(pack, "round", "learn")


def _mirror_learn_state(state):
    """rounds/discovery를 데몬 가독 위치로 미러(best-effort) — 실패는 RSI 판정에 불간섭."""
    try:
        d = _learn_state_dir()
        os.makedirs(d, exist_ok=True)
        payload = {
            "rounds": state.get("rounds", {}),
            "discovery": state.get("discovery", {"capability": 0, "perspective": 0, "knowledge": 0}),
        }
        p = os.path.join(d, "state.json")
        tmp = p + ".tmp"
        open(tmp, "w", encoding="utf-8").write(json.dumps(payload, ensure_ascii=False, indent=2))
        os.replace(tmp, p)
    except Exception:
        pass


def _save_state(state):
    d = rsi_dir()
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, "state.json")
    tmp = p + ".tmp"
    open(tmp, "w", encoding="utf-8").write(json.dumps(state, ensure_ascii=False, indent=2))
    os.replace(tmp, p)
    _mirror_learn_state(state)  # CC 학습 탭 배선(B-10)


def _append_ledger(entry):
    d = rsi_dir()
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "ledger.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


# ── RSI 학습 자율추천의 배달 채널: feed(건별 승인 요청) → 주간 다이제스트 큐 ──
# ★오너 승인 개정(2026-09-04 전면 감사): 자동 트리거(라운드 종료·eval ceiling)는 더 이상
#   `cys feed push --kind learn_proposal` 로 **건별 승인 요청**을 발행하지 않는다. 승인권은
#   오너뿐인데 자동 생성분 6건(최고령 19h48m)이 적체해 **실제 승인 요청**(SSH 프로브 9.5h)을
#   덮었다. RSI_LEARNING_DIRECTIVE §7-4 "접점 신설 시 다이제스트가 기본값"이 이미 정본이므로
#   코드를 정본에 맞춘 정합 수정이다(§3 도 같은 날 함께 개정).
# ★큐 경로·레코드 모양은 javis_orchestra.py 의 동명 함수와 **동형**이어야 한다(같은 큐에 쓴다).
#   패리티는 bin/tests/test_learn_digest_queue.py 가 기계 검증한다.
#   경로 해소는 이 모듈의 기존 규약 `_learn_state_dir()` 을 그대로 쓴다 — 생산 형상에서는
#   `<팩>/round/learn` 으로 orchestra 와 같고, `CYS_ROUND_DIR` 이 설정된 데몬 미러 형상에서만
#   그 값을 따른다(이 모듈이 원래 갖고 있던 오버라이드 — 새로 만든 갈래가 아니다).
LEARN_DIGEST_QUEUE = "digest_queue.jsonl"


def learn_digest_queue_path():
    """주간 다이제스트 큐 파일 — `<팩>/round/learn/digest_queue.jsonl`."""
    return os.path.join(_learn_state_dir(), LEARN_DIGEST_QUEUE)


def enqueue_learn_digest(reason, topic, source):
    """추천 1건을 다이제스트 큐에 적재(best-effort). feed 는 **쏘지 않는다**. 반환: 적재 여부."""
    try:
        path = learn_digest_queue_path()
        os.makedirs(os.path.dirname(path), exist_ok=True)
        rec = {"ts": time.time(), "reason": reason, "topic": topic, "source": source,
               "status": "queued_for_weekly_digest"}
        with open(path, "a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        return True
    except Exception:
        return False


def _recommend_learn(reason, topic):
    """RSI 학습 자율추천(best-effort) — **다이제스트 큐 적재**만 한다(feed 발행 0).
    추천까지만 자율·착수는 사람 승인이라는 directive §4 계약은 그대로이고, 바뀐 것은 **배달
    채널**뿐이다. 오류는 무시한다(추천은 비핵심 부가 신호 — 핵심 판정 불간섭)."""
    enqueue_learn_digest(reason, topic, "rsi.ceiling")


# ───────────────────────── 명령 ─────────────────────────

def cmd_checkpoint(a):
    _, head = _git(["rev-parse", "HEAD"])
    ts = time.time()
    ref = f"refs/rsi/ckpt/{a.round}"
    _git(["update-ref", ref, head])  # 복구 anchor (비파괴)
    state = _load_state()
    state["rounds"][a.round] = {
        "round": a.round, "checkpoint_sha": head, "ref": ref,
        "baseline_score": a.score, "started_at": ts, "note": a.note or "",
        "progress": [],
    }
    state["current_round"] = a.round
    _save_state(state)
    entry = {"event": "checkpoint", "round": a.round, "sha": head[:12],
             "score": a.score, "ts": ts, "ref": ref}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    return 0


def cmd_progress(a):
    state = _load_state()
    r = state["rounds"].get(a.round)
    if not r:
        print(f"error: 라운드 '{a.round}' checkpoint 없음 — 먼저 checkpoint 하라", file=sys.stderr)
        return 2
    base = r.get("baseline_score")
    # 직전 progress가 있으면 그것과 비교(라운드 내 단조), 없으면 baseline.
    prev = r["progress"][-1]["score"] if r.get("progress") else base
    if prev is None:
        print("error: 기준 score 없음 — checkpoint에 --score 주거나 직전 progress 필요", file=sys.stderr)
        return 2
    delta = a.score - prev
    v = verdict(delta)               # ★verdict는 순수 delta 산술 — tokens_saved 절대 미접촉(injected-only)
    ts = time.time()
    rec = {"score": a.score, "prev": prev, "delta": round(delta, 6), "verdict": v,
           "ts": ts, "note": a.note or ""}
    # U4 rider: tokens_saved는 score 옆 공동기록만(verdict/delta/flat_streak 불변). 미지정=키 생략.
    if getattr(a, "tokens_saved", None) is not None:
        rec["tokens_saved"] = a.tokens_saved
    r["progress"].append(rec)
    # (RSI 자율추천 iii) ceiling — flat N연속 = 점수 정체 → 학습 추천(추천만·사람 승인).
    r["flat_streak"] = (r.get("flat_streak", 0) + 1) if v == "flat" else 0
    _save_state(state)
    entry = {"event": "progress", "round": a.round, **rec}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    if r["flat_streak"] >= int(os.environ.get("CYS_RSI_CEILING_FLATS", "3")):
        _recommend_learn("ceiling", "%s 정체(ceiling) 돌파 방법론" % a.round)
    return 0


def cmd_markers(a):
    _, log = _git(["log", "-n", "300", "--format=%H%x1f%s%x1f%b%x1e"], check=False)
    markers = parse_markers(log)
    with_id = [m for m in markers if m["iter_id"] is not None]
    if a.json:
        print(json.dumps({"markers": with_id, "total_scanned": len(markers)}, ensure_ascii=False))
    else:
        if not with_id:
            print("iter-id trailer를 가진 커밋 없음 (RSI 라운드 커밋에 'iter-id: N' trailer를 달면 추적됨)")
        for m in with_id:
            print(f"  iter-{m['iter_id']:<4} {m['sha']}  {m['subject']}")
    return 0


def cmd_rollback(a):
    state = _load_state()
    r = state["rounds"].get(a.round)
    if not r:
        print(f"error: 라운드 '{a.round}' checkpoint 없음", file=sys.stderr)
        return 2
    ckpt = r["checkpoint_sha"]
    _, head = _git(["rev-parse", "HEAD"])
    # 더티 트리? — untracked는 reset --hard가 보존하므로 제외(추적 변경만 retention 위험).
    _, st = _git(["status", "--porcelain", "--untracked-files=no"], check=False)
    dirty = bool(st.strip())
    # ckpt가 HEAD 조상인가?
    rc, _ = _git(["merge-base", "--is-ancestor", ckpt, head], check=False)
    is_ancestor = rc == 0
    # 버려질 커밋 목록
    _, disc = _git(["log", "--oneline", f"{ckpt}..HEAD"], check=False)
    discarded = [ln for ln in disc.splitlines() if ln.strip()]
    plan = rollback_plan(a.round, ckpt[:12], head[:12], discarded, dirty, is_ancestor)

    if not a.execute:
        backup = f"rsi-abandoned-{a.round}-<ts>"
        plan["dry_run"] = True
        plan["recovery_branch_would_be"] = backup
        plan["commands_if_executed"] = [
            f"git branch {backup} {head[:12]}",
            f"git reset --hard {ckpt[:12]}",
        ]
        print(json.dumps(plan, ensure_ascii=False, indent=2))
        print("\n※ dry-run — 아무것도 실행하지 않았다. 실제 실행은 --execute (현재 HEAD는 백업 브랜치로 보존됨).", file=sys.stderr)
        return 0

    # --execute: 차단 조건
    if plan["blockers"] and not a.force:
        print("error: rollback 거부 — " + "; ".join(plan["blockers"]), file=sys.stderr)
        return 3
    # ★retention: 현재 HEAD를 백업 브랜치에 먼저 박제(비가역 삭제 차단)
    backup = f"rsi-abandoned-{a.round}-{int(time.time())}"
    _git(["branch", backup, head])
    # 그 다음에만 reset
    _git(["reset", "--hard", ckpt])
    entry = {"event": "rollback", "round": a.round, "from": head[:12], "to": ckpt[:12],
             "recovery_branch": backup, "discarded_count": len(discarded), "ts": time.time()}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    print(f"\n✅ rollback 완료. 버려진 {len(discarded)}커밋은 '{backup}' 브랜치에 보존 — 복구: git checkout {backup}", file=sys.stderr)
    return 0


def cmd_status(a):
    state = _load_state()
    cur = state.get("current_round")
    if a.json:
        print(json.dumps(state, ensure_ascii=False, indent=2))
        return 0
    if not cur:
        print("RSI 라운드 기록 없음 (checkpoint --round <id> 로 시작)")
        return 0
    r = state["rounds"].get(cur, {})
    print(f"현재 라운드: {cur} · checkpoint {r.get('checkpoint_sha','?')[:12]} · 기준점수 {r.get('baseline_score')}")
    for p in r.get("progress", []):
        print(f"  score {p['score']} (Δ{p['delta']:+}) → {p['verdict']}")
    return 0


def main():
    ap = argparse.ArgumentParser(description="RSI 라운드 무결성 결정론 도구 (eval-driven)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("checkpoint"); c.add_argument("--round", required=True); c.add_argument("--score", type=float); c.add_argument("--note")
    p = sub.add_parser("progress"); p.add_argument("--round", required=True); p.add_argument("--score", type=float, required=True); p.add_argument("--note")
    p.add_argument("--tokens-saved", type=float, default=None, help="U4 비-verdict rider — 원장에 공동기록만, verdict()/delta 미접촉(injected-only 불변)")
    m = sub.add_parser("markers"); m.add_argument("--json", action="store_true")
    rb = sub.add_parser("rollback"); rb.add_argument("--round", required=True); rb.add_argument("--execute", action="store_true"); rb.add_argument("--force", action="store_true")
    s = sub.add_parser("status"); s.add_argument("--json", action="store_true")
    a = ap.parse_args()
    return {"checkpoint": cmd_checkpoint, "progress": cmd_progress, "markers": cmd_markers,
            "rollback": cmd_rollback, "status": cmd_status}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
