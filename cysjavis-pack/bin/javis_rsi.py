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
      현재 라운드·시도수(attempts)·flat 연속·stop_reason 요약.

★WP-6 라운드 예산: `state["rounds"][id]["attempts"]` 는 checkpoint·progress 를 함께 세며
  재checkpoint·재시작·ledger 라인 삭제를 넘어 영속한다(state 와 ledger 의 큰 값). 상한
  `CYS_RSI_MAX_ROUNDS`(기본 3) 초과 = `stop_reason=stopped_budget`, flat 연속
  `CYS_RSI_CEILING_FLATS`(기본 3) = `stopped_stagnation`. 사유는 **기록·고지**이며 exit code 는
  바꾸지 않는다(소비자 학습 루프를 세우지 않는다 — 하드 상한은 javis_learn 층에 이미 있다).

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

# ── WP-6: RSI 라운드 예산·정체 종료 사유 ──────────────────────────────────────
# ★왜(정본 §4 WP-6): 종전엔 라운드가 몇 번째 시도인지 도구가 몰랐다. `cmd_checkpoint` 가
#   `state["rounds"][id]` 를 **통째로 덮어써서** 재기록·재시작이 이력을 지웠기 때문이다
#   (같은 라운드를 무한히 다시 시작해도 카운터가 늘 1). `attempts` 가 그 이력이다.
# ★attempt 의 정의(codex 적대 검토 blocking-6): "평가 시도 1회" = 이 도구에 **점수가 들어온
#   기록 1건** — 즉 checkpoint(기준선) 와 progress(개선분) 를 함께 센다. checkpoint 만 세면
#   javis_learn 의 정상 호출(첫 회 checkpoint + 이후 progress 반복)에서 상한이 무력해진다.
# ★영속(선례 javis_learn `_ledger_evaluate_count` :625): state 와 **append-only ledger** 의
#   큰 값을 쓴다 — state.json 재기록/삭제도, ledger 라인 삭제도 단독으로는 카운터를 되돌리지
#   못한다. (한계: 잠금은 없다. 동시 호출은 state 증가분을 잃을 수 있으나 ledger 재계수가
#   다음 호출에서 복구한다. Windows 는 fcntl 이 없어 잠금 도입은 이 WP 범위 밖이다.)
RSI_STOP_REASONS = ("open", "stopped_budget", "stopped_stagnation")
RSI_ATTEMPT_EVENTS = ("checkpoint", "progress")


def _as_int(v, default=0):
    """느슨한 정수 해석 — 손상된 state 값이 판정을 죽이지 않게(결측은 값이 아니다)."""
    try:
        return int(v)
    except (TypeError, ValueError):
        return default


def _env_int(key, default):
    """양의 정수 env 노브 — 미설정·비정수·0 이하는 기본값(게이트를 끄는 노브 없음)."""
    n = _as_int(os.environ.get(key), 0)
    return n if n > 0 else default


def rsi_max_rounds():
    """라운드 예산 — `CYS_RSI_MAX_ROUNDS` 기본 3(정본 §4 WP-6)."""
    return _env_int("CYS_RSI_MAX_ROUNDS", 3)


def rsi_ceiling_flats():
    """정체(ceiling) 판정 — flat 연속 `CYS_RSI_CEILING_FLATS` 기본 3."""
    return _env_int("CYS_RSI_CEILING_FLATS", 3)


def rsi_stop_reason(attempts, flat_streak, max_rounds, ceiling):
    """RSI 라운드 종료 사유 — 순수 함수. 예산 초과가 정체보다 강하다(정본 열거 순서).

    ★이 값은 **보고**다: 도구는 exit 0 을 유지하고 기록만 한다. 하드 실패로 올리면
      `javis_learn.py:766` 이 rc≠0 을 fail(12) 로 올려 학습 루프 자체가 서 버린다(§7 위험
      ③ 자가치유 전멸 방향). 반복 평가의 **하드 상한**은 이미 소비자 층에 있다
      (`javis_learn.EVALUATE_ATTEMPT_CAP=3` → 4회째 fail(9)). 여기서는 그 상한과 같은 값을
      기본으로 두고, 사유를 구조화해 소비자·지침이 읽게 한다.
    """
    if attempts > max_rounds:
        return "stopped_budget"
    if flat_streak >= ceiling:
        return "stopped_stagnation"
    return "open"


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


def _ledger_attempt_count(rid):
    """라운드별 평가 시도 수 — ledger.jsonl 계수(append-only 진실 · 선례 javis_learn :625).
    state.json 을 지우거나 되돌려도 이 값이 남아 상한을 되살린다."""
    n = 0
    try:
        with open(os.path.join(rsi_dir(), "ledger.jsonl"), encoding="utf-8") as f:
            for ln in f:
                try:
                    e = json.loads(ln)
                except ValueError:
                    continue
                if isinstance(e, dict) and e.get("event") in RSI_ATTEMPT_EVENTS \
                        and e.get("round") == rid:
                    n += 1
    except OSError:
        pass
    return n


def _next_attempt(state, rid):
    """이번 호출의 시도 번호 — max(state, ledger) + 1(양쪽 되돌리기 방어)."""
    prev = state.get("rounds", {}).get(rid)
    prev = prev if isinstance(prev, dict) else {}
    return max(_as_int(prev.get("attempts"), 0), _ledger_attempt_count(rid)) + 1, prev


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


class _best_effort_lock(object):
    """`os.mkdir` 원자성만 쓰는 최선노력 상호배제 — fcntl·msvcrt 무의존(Windows 안전).

    ★javis_orchestra.py 의 동명 클래스와 **의도적 중복**이다: `javis_rsi` 는 독립 실행 도구라
      orchestra(대형 모듈)를 import 하지 않는다. 여기서도 정합의 근거가 아니라 **완충**이며,
      대기 상한(5s) 뒤에는 그냥 진행한다 — 무한대기는 부트체인 ④ 방향이다. 중복 방지의 정본은
      큐에 남는 **멱등키**다(잠금이 실패해도 키 검사가 다음 호출에서 잡는다).
    """

    def __init__(self, path, wait=5.0, stale=300.0):
        self.path, self.wait, self.stale, self.held = path + ".lock", wait, stale, False

    def __enter__(self):
        deadline = time.time() + self.wait
        while True:
            try:
                os.mkdir(self.path)
                self.held = True
                return self
            except FileExistsError:
                pass
            except OSError:
                return self
            if time.time() >= deadline:
                return self
            try:
                if time.time() - os.path.getmtime(self.path) > self.stale:
                    os.rmdir(self.path)
                    continue
            except OSError:
                pass
            time.sleep(0.05)

    def __exit__(self, *exc):
        if self.held:
            try:
                os.rmdir(self.path)
            except OSError:
                pass
            self.held = False
        return False


def digest_queue_has_key(path, key):
    """다이제스트 큐에 같은 **멱등키**가 이미 있는가(javis_orchestra 동명 함수와 동형)."""
    if not key:
        return False
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except OSError:
        return False
    for chunk in raw.split(b"\n"):
        if not chunk.strip():
            continue
        try:
            rec = json.loads(chunk.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            continue
        if isinstance(rec, dict) and rec.get("key") == key:
            return True
    return False


def enqueue_learn_digest(reason, topic, source, key=None):
    """추천 1건을 다이제스트 큐에 적재(best-effort). feed 는 **쏘지 않는다**. 반환: 적재 여부.
    `key` 를 주면 검사+적재를 한 잠금 안에서 하고 같은 키가 있으면 적재하지 않는다(멱등)."""
    try:
        path = learn_digest_queue_path()
        os.makedirs(os.path.dirname(path), exist_ok=True)
        rec = {"ts": time.time(), "reason": reason, "topic": topic, "source": source,
               "status": "queued_for_weekly_digest", "key": key or ""}
        with _best_effort_lock(path):
            if key and digest_queue_has_key(path, key):
                return False
            with open(path, "a", encoding="utf-8") as f:
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        return True
    except Exception:
        return False


def _ledger_has_event(kind, rid):
    """append-only ledger 에 그 라운드의 이벤트가 있는가 — 래치의 두 번째 내구 근거.
    (큐가 주간 다이제스트로 **소비·정리**된 뒤에도 남는다. `RSI_ATTEMPT_EVENTS` 밖의 종류라
     시도 계수에는 잡히지 않는다.)"""
    try:
        with open(os.path.join(rsi_dir(), "ledger.jsonl"), encoding="utf-8", errors="replace") as f:
            for ln in f:
                try:
                    e = json.loads(ln)
                except ValueError:
                    continue
                if isinstance(e, dict) and e.get("event") == kind and e.get("round") == rid:
                    return True
    except OSError:
        pass
    return False


def _recommend_learn(reason, topic, key=None):
    """RSI 학습 자율추천(best-effort) — **다이제스트 큐 적재**만 한다(feed 발행 0). 반환 적재 여부.
    추천까지만 자율·착수는 사람 승인이라는 directive §4 계약은 그대로이고, 바뀐 것은 **배달
    채널**뿐이다. 오류는 무시한다(추천은 비핵심 부가 신호 — 핵심 판정 불간섭)."""
    return enqueue_learn_digest(reason, topic, "rsi.ceiling", key)


# ───────────────────────── 명령 ─────────────────────────

def cmd_checkpoint(a):
    _, head = _git(["rev-parse", "HEAD"])
    ts = time.time()
    ref = f"refs/rsi/ckpt/{a.round}"
    _git(["update-ref", ref, head])  # 복구 anchor (비파괴)
    state = _load_state()
    # ★WP-6: 재checkpoint 는 라운드 레코드를 새로 쓰지만 **시도 이력은 이월한다**.
    #   (종전엔 통째 덮어쓰기라 같은 라운드를 다시 시작하면 상한이 리셋됐다 —
    #    ceiling 신호(flat_streak)도 같은 이유로 이월한다: 재시작이 정체를 지우면 안 된다.)
    attempts, prev = _next_attempt(state, a.round)
    flat = _as_int(prev.get("flat_streak"), 0)
    stop_reason = rsi_stop_reason(attempts, flat, rsi_max_rounds(), rsi_ceiling_flats())
    state.setdefault("rounds", {})[a.round] = {
        "round": a.round, "checkpoint_sha": head, "ref": ref,
        "baseline_score": a.score, "started_at": ts, "note": a.note or "",
        "progress": [], "attempts": attempts, "flat_streak": flat,
        "stop_reason": stop_reason,
        "ceiling_recommended": bool(prev.get("ceiling_recommended")),
    }
    state["current_round"] = a.round
    _save_state(state)
    entry = {"event": "checkpoint", "round": a.round, "sha": head[:12],
             "score": a.score, "ts": ts, "ref": ref,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    _warn_stop(stop_reason, a.round, attempts)
    return 0


def _warn_stop(stop_reason, rid, attempts):
    """종료 사유 고지(stderr) — exit code 는 바꾸지 않는다(소비자 루프를 세우지 않는다)."""
    if stop_reason == "stopped_budget":
        print("[rsi] stop_reason=stopped_budget — 라운드 '%s' 시도 %d회 > 상한 %d "
              "(CYS_RSI_MAX_ROUNDS). 라운드를 잇지 말고 격차를 보고하라(기록은 남았다)."
              % (rid, attempts, rsi_max_rounds()), file=sys.stderr)
    elif stop_reason == "stopped_stagnation":
        print("[rsi] stop_reason=stopped_stagnation — flat 연속 %d회 이상(ceiling). 같은 방법의 "
              "반복은 점수를 올리지 못한다: 방법을 바꾸거나 종결하라." % rsi_ceiling_flats(),
              file=sys.stderr)


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
    r["flat_streak"] = (_as_int(r.get("flat_streak"), 0) + 1) if v == "flat" else 0
    # ★WP-6: progress 도 **평가 시도**다(점수가 들어온 기록) — checkpoint 만 세면 상한이
    #   무력하다(javis_learn 정상 호출은 checkpoint 1회 + progress 반복).
    attempts, _prev = _next_attempt(state, a.round)
    r["attempts"] = attempts
    stop_reason = rsi_stop_reason(attempts, r["flat_streak"], rsi_max_rounds(),
                                  rsi_ceiling_flats())
    r["stop_reason"] = stop_reason
    _save_state(state)
    entry = {"event": "progress", "round": a.round, **rec,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    _warn_stop(stop_reason, a.round, attempts)
    # 추천은 **라운드당 1회**다(다이제스트 1줄 계약). 종전엔 ceiling 이상인 매 progress 마다
    # 적재해 같은 사유가 큐에 쌓였다 — 배달 채널은 그대로(feed 0 · 주간 다이제스트).
    # ★래치는 state.json 밖에도 있어야 한다(codex R1 major-10): state 를 지우거나 되돌리면
    #   같은 라운드의 같은 사유가 두 번 적재됐다. 그래서 **라운드별 멱등키**를 ①큐 레코드와
    #   ②append-only ledger 양쪽에서 조회한다(큐가 소비돼도 ledger 가 남고, ledger 를 잃어도
    #   큐가 남는다). 적재 실패면 래치를 걸지 않는다 — 추천을 영구히 잃지 않기 위해서다.
    key = "rsi.ceiling:%s" % a.round
    if r["flat_streak"] >= rsi_ceiling_flats() and not r.get("ceiling_recommended"):
        if digest_queue_has_key(learn_digest_queue_path(), key) or \
                _ledger_has_event("ceiling_recommend", a.round):
            r["ceiling_recommended"] = True   # 이미 추천됨(다른 경로에서) — 래치만 복원
            _save_state(state)
        elif _recommend_learn("ceiling", "%s 정체(ceiling) 돌파 방법론" % a.round, key):
            _append_ledger({"event": "ceiling_recommend", "round": a.round,
                            "key": key, "ts": time.time()})
            r["ceiling_recommended"] = True
            _save_state(state)
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
    print(f"  시도 {_as_int(r.get('attempts'), 0)}/{rsi_max_rounds()} · "
          f"flat 연속 {_as_int(r.get('flat_streak'), 0)}/{rsi_ceiling_flats()} · "
          f"stop_reason={r.get('stop_reason') or 'open'}")
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
