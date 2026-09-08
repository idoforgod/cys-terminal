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
import hashlib
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

def _read_state_file():
    """state.json → (상태, 판독 불가 사유). **부재만** 빈 상태다(독립 재유도 X-4).

    ★"읽을 수 없다"는 "없다"가 아니다: 권한·I/O 실패·깨진 JSON 을 빈 상태로 접으면 라운드
      예산(attempts)이 조용히 리셋되고, 그 리셋이 **확정**으로 보고된다. 부재(첫 라운드)만
      정상이고 나머지는 불확정으로 올려 `budget_unknown` 어휘에 싣는다.
    """
    p = os.path.join(rsi_dir(), "state.json")
    try:
        with open(p, encoding="utf-8") as f:
            return json.load(f), ""
    except FileNotFoundError:
        return {"rounds": {}}, ""
    except (OSError, ValueError) as e:
        return {"rounds": {}}, "%s" % e


def _load_state():
    """호환 진입점 — 상태만 돌려준다(불확정 여부가 필요하면 `_read_state_file`)."""
    return _read_state_file()[0]


def _learn_state_dir():
    """cysd learn.status가 읽는 위치(handlers.rs learn_state_dir와 동일 규칙) —
    CC 학습 탭 미러링용(B-10: 프로젝트 _round/rsi와 데몬 읽기 경로의 단절 해소)."""
    root = os.environ.get("CYS_ROUND_DIR")
    if root:
        return os.path.join(root, "learn")
    pack = os.environ.get("CYS_PACK_DIR") or os.path.expanduser("~/.cys/pack")
    return os.path.join(pack, "round", "learn")


# ★미러에서 **이름을 바꿔야 하는** 키(claude R2 major-1): `javis_learn.cmd_evaluate` 는 라운드
#   레코드의 `attempts` 를 **자기 judge-shopping 시도수**로 읽는다(:724 `max(_existing.get("attempts")…)`
#   → `EVALUATE_ATTEMPT_CAP=3` 초과면 fail(9) ESCALATE). 그런데 이 미러는 `<CYS_ROUND_DIR>/learn/state.json`
#   에 쓰고, learn 은 사설 `learn_state.json` 이 없거나(첫 evaluate) canonical union 경로에서 그 파일을
#   폴백으로 읽는다. 즉 RSI 의 시도수가 learn 의 상한으로 **새어** 첫 평가가 즉시 막혔다(실측 재현:
#   rsi checkpoint+progress×3 → learn evaluate = "evaluate 4회 기록 — 4회째=ESCALATE", learn 평가 0회).
#   자가치유 루프 정지 방향(§7 위험 ③)이므로 미러에서는 **다른 이름**으로 내보낸다. 데몬은 라운드
#   레코드를 그대로 전달만 하고(handlers.rs learn.status), 병합은 화이트리스트(verdict·stored·harness·
#   items·evaluator_hash·schema)라 이 개명에 무영향이다.
MIRROR_RENAME_KEYS = {"attempts": "rsi_attempts"}


def _mirror_round_rec(rec):
    """미러용 라운드 레코드 — learn 의 lifecycle 키와 **이름이 겹치지 않게** 개명(순수 함수)."""
    if not isinstance(rec, dict):
        return rec
    out = {}
    for k, v in rec.items():
        out[MIRROR_RENAME_KEYS.get(k, k)] = v
    return out


def _mirror_learn_state(state):
    """rounds/discovery를 데몬 가독 위치로 미러(best-effort) — 실패는 RSI 판정에 불간섭."""
    try:
        d = _learn_state_dir()
        os.makedirs(d, exist_ok=True)
        rounds = state.get("rounds", {})
        payload = {
            "rounds": {k: _mirror_round_rec(v) for k, v in rounds.items()}
                      if isinstance(rounds, dict) else {},
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


def _read_ledger():
    """ledger.jsonl → 레코드 목록. **바이트로 읽고 줄마다 strict 디코드**한다.

    반환 `(레코드 목록, 손상 줄 수, 판독 불가 사유)`.

    ★깨진 1바이트로 도구가 죽지 않는다(리뷰 R2 blocking · claude major-2): 종전엔 텍스트 모드
      반복이라 `\xed\x95` 같은 줄 하나가 `UnicodeDecodeError` 를 던졌고, 핸들러는 `OSError` 만
      잡아 **checkpoint·progress 가 rc=1 로 죽었다**. 소비자(`javis_learn.py:766`)는 그 rc≠0 을
      "일시적·재시도 가능"(fail 12)으로 올리는데 손상 줄은 남으므로 **영구 정지**였다
      (§7 위험 ③ 자가치유 전멸 방향). 이제 손상은 예외가 아니라 **세어서 돌려주는 값**이다.
    ★`errors="replace"` 를 쓰지 않는 이유는 orchestra 사이드카와 같다: 문자열 **안**의 깨진
      바이트가 유효 JSON 으로 통과해 `round`·`event` 가 조용히 바뀔 수 있다(거짓 계수).
    """
    recs, damaged, unreadable = [], 0, ""
    try:
        with open(os.path.join(rsi_dir(), "ledger.jsonl"), "rb") as f:
            raw = f.read()
    except FileNotFoundError:
        return recs, damaged, unreadable          # 없음 = 이력 없음(정상 · 첫 라운드)
    except OSError as e:
        # ★"읽을 수 없다"는 "없다"가 아니다(독립 재유도 X-4 · orchestra 사이드카와 같은 규율):
        #   권한·I/O 실패를 빈 이력으로 접으면 예산이 조용히 리셋되는데, 같은 모듈이 손상 줄
        #   1개에는 `budget_unknown` 을 붙여 "모름을 모른다"고 말한다(§8-1 M5). 비대칭을 없앤다.
        return recs, damaged, "%s" % e
    for chunk in raw.split(b"\n"):
        if not chunk.strip():
            continue
        try:
            e = json.loads(chunk.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            damaged += 1
            continue
        if isinstance(e, dict):
            recs.append(e)
        else:
            damaged += 1
    return recs, damaged, unreadable


def _safe_append_ledger(entry):
    """보조 기록용 append — 실패를 **삼킨다**(주 평가의 rc 를 바꾸지 않는다). 반환 성공 여부."""
    try:
        _append_ledger(entry)
        return True
    except Exception:
        return False


def _ledger_attempt_count(rid):
    """라운드별 평가 시도 수 — ledger.jsonl 계수(append-only 진실 · 선례 javis_learn :625).
    state.json 을 지우거나 되돌려도 이 값이 남아 상한을 되살린다. 반환 (계수, 손상 줄 수).

    ★손상 줄은 **어느 라운드의 것인지 알 수 없다**. 그래서 계수에 더하지 않는다(codex R2
      major-10 반례: 라운드 A 의 깨진 3줄이 신규 라운드 B·C·D 의 첫 checkpoint 를 곧바로
      `stopped_budget` 으로 만들었다 — 남의 예산을 태우는 거짓 정밀도다). 대신 **불확정**으로
      따로 돌려주고(`budget_unknown`) 고지한다: 계수는 확인된 것만, 모르는 것은 모른다고.
    ★그럼 예산이 되돌아가지 않는가? 되돌아갈 수 있는 경로는 'state 소실 + 그 라운드의 ledger
      줄 손상' 동시 발생뿐이고, 그때 stop_reason 은 `budget_unknown` 을 달고 나간다(§5 한계).
    """
    recs, damaged, unreadable = _read_ledger()
    n = 0
    for e in recs:
        if e.get("event") in RSI_ATTEMPT_EVENTS and e.get("round") == rid:
            n += 1
    return n, damaged, unreadable


def _next_attempt(state, rid):
    """이번 호출의 시도 번호 — max(state, ledger) + 1(양쪽 되돌리기 방어).
    반환 (시도번호, 직전 라운드 레코드, 손상 줄 수, ledger 판독 불가 사유)."""
    prev = state.get("rounds", {}).get(rid)
    prev = prev if isinstance(prev, dict) else {}
    led, damaged, unreadable = _ledger_attempt_count(rid)
    return max(_as_int(prev.get("attempts"), 0), led) + 1, prev, damaged, unreadable


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


def rsi_project_identity():
    """다이제스트 멱등키에 넣을 **프로젝트 신원** — RSI 이력 디렉터리의 실경로 sha256.

    ★왜 필요한가(독립 재유도 X-5): RSI 이력은 프로젝트별(`cwd/_round/rsi`)인데 다이제스트 큐는
      **공용 팩**(`<팩>/round/learn`)이고, 종전 멱등키는 `rsi.ceiling:<round>` 로 신원이 없었다.
      라운드 id 는 `r1`·`r2` 처럼 짧아 팩을 공유하는 다중 프로젝트에서 충돌이 예외가 아니라
      **기본값**이다 — A 의 r1 추천이 큐에 있으면 B 의 r1 추천이 '이미 나갔다'로 눌리고,
      B 의 ledger 에는 나간 적 없는 추천의 래치가 영속해 큐가 비워져도 **영구 억제**된다.
    ★한계(정직한 표기): 이것은 프로젝트의 신원이 아니라 **이력 저장 경로의 이름**이다.
      별칭·이동은 같은 이력을 다른 키로 만들어 **추천 1건 중복**을 낼 수 있고, `CYS_ROUND_DIR`
      을 여러 프로젝트가 공유하면 이력 자체가 공유되므로 키도 같아진다. 두 오답 중 중복은
      다이제스트 한 줄이고 억제는 자가치유 신호의 영구 유실이다 — 중복 쪽으로 튼다.
    ★자르지 않는다(codex 지적): 12자(48비트)는 불필요한 충돌 가능성을 더한다. 전체 sha256 을
      쓰는 비용은 큐 한 줄의 길이뿐이다.
    """
    try:
        real = os.path.realpath(rsi_dir())
    except OSError:
        real = os.path.abspath(rsi_dir())
    return hashlib.sha256(real.encode("utf-8", "replace")).hexdigest()


def ceiling_digest_key(rid):
    """ceiling 추천의 멱등키 — `rsi.ceiling:<프로젝트 신원>:<라운드>`(순수 함수는 아니다:
    경로를 읽는다). 구 키(`rsi.ceiling:<라운드>`)는 조회 대상이 아니다."""
    return "rsi.ceiling:%s:%s" % (rsi_project_identity(), rid)


class _best_effort_lock(object):
    """`os.mkdir` 원자성만 쓰는 최선노력 상호배제 — fcntl·msvcrt 무의존(Windows 안전).

    ★javis_orchestra.py 의 동명 클래스와 **의도적 중복**이다: `javis_rsi` 는 독립 실행 도구라
      orchestra(대형 모듈)를 import 하지 않는다. 여기서도 정합의 근거가 아니라 **완충**이며,
      대기 상한(5s) 뒤에는 그냥 진행한다 — 무한대기는 부트체인 ④ 방향이다. 중복 방지의 정본은
      큐에 남는 **멱등키**다(잠금이 실패해도 키 검사가 다음 호출에서 잡는다).
    """

    def __init__(self, path, wait=5.0, stale=300.0):
        self.path, self.wait, self.stale, self.held = path + ".lock", wait, stale, False
        self.blocked, self.unsupported = False, ""
        # 소유자 토큰 — **내 잠금일 때만** 지운다(orchestra 와 같은 규율: 느린 소유자가 고아로
        # 오인돼 회수된 뒤 그대로 rmdir 하면 다음 소유자의 잠금을 지워 둘이 함께 들어간다).
        self.wrote_owner = False
        # 회수는 `__enter__` 당 한 번만(orchestra 와 같은 규율 — 연쇄 강탈 차단).
        self.reclaim_tried = False
        self.token = ("%d-%d-%s" % (os.getpid(), int(time.time() * 1000),
                                    hashlib.sha256(os.urandom(16)).hexdigest()[:16])).encode("ascii")

    def _owner_file(self):
        return os.path.join(self.path, "owner")

    def _owner(self):
        try:
            with open(self._owner_file(), "rb") as f:
                return f.read(200)
        except OSError:
            return b""

    def _claim_name(self):
        """회수 청구 파일 — 잠금 디렉터리 **안**의 `owner.stale-<내토큰>`(청구자마다 유일)."""
        return self._owner_file() + ".stale-" + self.token.decode("ascii", "replace")

    def _abandoned_claim(self):
        """중단된 회수가 남긴 청구 파일 하나 — owner 가 없고 나이가 상한을 넘은 것만
        (rename 성공과 unlink 사이의 사망이 잠금을 영구 교착으로 만들지 않게 한다)."""
        try:
            names = sorted(os.listdir(self.path))
        except OSError:
            return None
        if "owner" in names:
            return None
        for n in names:
            if not n.startswith("owner.stale-"):
                return None          # 모르는 내용물 — 손대지 않는다(보류 방향)
        for n in names:
            q = os.path.join(self.path, n)
            try:
                if time.time() - os.path.getmtime(q) > self.stale:
                    return q
            except OSError:
                return None
        return None

    def _reclaim_orphan(self, seen):
        """고아 잠금 회수 — **원자적 청구(rename)에 성공한 하나만** 회수자다(javis_orchestra
        동명 메서드와 같은 규율 · 독립 재유도 X-1). 확인과 삭제가 원자적이지 않으면 마지막
        확인 **뒤** 들어온 새 소유자의 잠금을 지워 두 writer 가 동시에 임계구간에 든다."""
        claim, src = self._claim_name(), self._owner_file()
        if not os.path.exists(src):
            src = self._abandoned_claim()
            if not src:
                return False
            seen = None
        try:
            os.rename(src, claim)              # ★원자적 청구
        except OSError:
            return False
        if seen is not None:
            try:
                with open(claim, "rb") as f:
                    got = f.read(200)
            except OSError:
                got = None
            if got != seen:                    # 내가 본 그 고아가 아니다 — 원상복구
                try:
                    os.rename(claim, self._owner_file())
                except OSError:
                    pass
                return False
        try:
            os.unlink(claim)
        except FileNotFoundError:
            return False
        except OSError:
            try:
                os.rename(claim, self._owner_file())
            except OSError:
                pass
            return False
        try:
            os.rmdir(self.path)                # 비어 있을 때만 성공한다
        except OSError:
            return False
        return True

    def __enter__(self):
        deadline = time.monotonic() + self.wait      # 단조 시계 — 시스템 시각 되감기 방어
        while True:
            try:
                os.mkdir(self.path)
                self.held = True
                try:
                    with open(self._owner_file(), "wb") as f:
                        f.write(self.token)
                    self.wrote_owner = self._owner() == self.token     # 되읽어 확인한다
                except OSError:
                    pass
                return self
            except FileExistsError:
                pass
            except OSError as e:
                self.unsupported = "%s" % e
                return self
            if time.monotonic() >= deadline:
                self.blocked = True
                return self
            try:
                if not self.reclaim_tried and \
                        time.time() - os.path.getmtime(self.path) > self.stale:
                    seen = self._owner()
                    time.sleep(0.2)
                    if seen == self._owner() and \
                            time.time() - os.path.getmtime(self.path) > self.stale:
                        self.reclaim_tried = True
                        self._reclaim_orphan(seen)
                    continue
            except OSError:
                pass
            time.sleep(0.05)

    def __exit__(self, *exc):
        if self.held:
            # 토큰이 비어 있으면(owner 쓰기 실패) 우리가 만든 것 — 반납하지 않으면 아무도 못 푼다.
            # ★단 내 토큰이 되읽기로 **확인**된 경우엔 빈 owner 를 내 것으로 보지 않는다: 새
            #   소유자가 mkdir 뒤 owner 를 쓰기 전의 빈 상태를 옛 소유자가 오인하면 안 된다.
            own = self._owner()
            if own == self.token or (own == b"" and not self.wrote_owner):
                try:
                    os.unlink(self._owner_file())
                except OSError:
                    pass
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
        with _best_effort_lock(path) as lk:
            # ★잠금을 못 쥐면 **쓰지 않는다**(codex R2 major-6 과 같은 규율): 경합 중에 검사+적재를
            #   하면 같은 키가 두 줄 쌓인다. 적재하지 않으면 래치도 서지 않으므로 추천은 다음
            #   호출에서 다시 시도된다(영구 유실 0).
            if lk.blocked:
                return False
            if key and digest_queue_has_key(path, key):
                return False
            with open(path, "a", encoding="utf-8") as f:
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        return True
    except Exception:
        return False


def _ledger_has_event(kind, rid, key=None):
    """append-only ledger 에 그 라운드의 이벤트가 있는가 — 래치의 두 번째 내구 근거.
    `key` 를 주면 **그 멱등키로 기록된 것만** 인정한다(구 키의 래치를 승계하지 않는다).
    (큐가 주간 다이제스트로 **소비·정리**된 뒤에도 남는다. `RSI_ATTEMPT_EVENTS` 밖의 종류라
     시도 계수에는 잡히지 않는다.)"""
    recs, _damaged, _unreadable = _read_ledger()   # 판독 규약은 `_ledger_attempt_count` 와 같다
    for e in recs:
        if e.get("event") != kind or e.get("round") != rid:
            continue
        if key is not None and e.get("key") != key:
            # ★신원 없는 **구 키**의 래치는 승계하지 않는다(독립 재유도 X-5): 프로젝트 신원이
            #   없던 시절의 키로 눌린 기록(특히 남의 추천을 보고 메운 `backfilled`)이 이 
            #   프로젝트의 추천을 영구히 억제한다. 출처를 증명할 수 없으면 **중복 1회**가
            #   영구 유실보다 안전하다.
            continue
        return True
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
    state, state_unreadable = _read_state_file()
    # ★WP-6: 재checkpoint 는 라운드 레코드를 새로 쓰지만 **시도 이력은 이월한다**.
    #   (종전엔 통째 덮어쓰기라 같은 라운드를 다시 시작하면 상한이 리셋됐다 —
    #    ceiling 신호(flat_streak)도 같은 이유로 이월한다: 재시작이 정체를 지우면 안 된다.)
    attempts, prev, damaged, unreadable = _next_attempt(state, a.round)
    flat = _as_int(prev.get("flat_streak"), 0)
    stop_reason = rsi_stop_reason(attempts, flat, rsi_max_rounds(), rsi_ceiling_flats())
    rec = {
        "round": a.round, "checkpoint_sha": head, "ref": ref,
        "baseline_score": a.score, "started_at": ts, "note": a.note or "",
        "progress": [], "attempts": attempts, "flat_streak": flat,
        "stop_reason": stop_reason,
        "ceiling_recommended": bool(prev.get("ceiling_recommended")),
    }
    if prev.get("ceiling_recommended_key"):
        rec["ceiling_recommended_key"] = prev["ceiling_recommended_key"]
    if damaged or unreadable or state_unreadable:
        rec["budget_unknown"] = True      # 영속 state 에도 싣는다(다음 호출이 이어 읽는다)
    state.setdefault("rounds", {})[a.round] = rec
    state["current_round"] = a.round
    _save_state(state)
    entry = {"event": "checkpoint", "round": a.round, "sha": head[:12],
             "score": a.score, "ts": ts, "ref": ref,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _mark_unknown(entry, damaged, unreadable, state_unreadable)
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    _warn_stop(stop_reason, a.round, attempts, damaged, unreadable, state_unreadable)
    return 0


def _mark_unknown(entry, damaged, unreadable="", state_unreadable=""):
    """예산 불확정 표기를 stdout JSON 레코드에 싣는다 — **기존 어휘 재사용**(소비자 개정 0).

    `budget_unknown` 은 이미 손상 줄 1개에 붙던 표기다(§8-1 M5). 전면 판독 불가(권한·I/O)와
    state 판독 불가도 같은 축의 사건이므로 같은 키에 싣고, 원인만 별도 필드로 남긴다.
    """
    if damaged:
        entry["ledger_damaged"], entry["budget_unknown"] = damaged, True
    if unreadable:
        entry["ledger_unreadable"], entry["budget_unknown"] = unreadable, True
    if state_unreadable:
        entry["state_unreadable"], entry["budget_unknown"] = state_unreadable, True
    return entry


def _warn_damaged(damaged, unreadable="", state_unreadable=""):
    """ledger 손상 고지(stderr) — 손상을 조용히 건너뛰지 않는다. exit code 는 바꾸지 않는다."""
    if unreadable:
        print("[rsi] 주의: ledger.jsonl 을 **읽을 수 없다**(%s) — 읽을 수 없는 것은 '이력 없음'이 "
              "아니다. 이 라운드의 시도수는 확인된 것만 센 값이며 실제보다 작을 수 있다"
              "(budget_unknown). 파일: %s"
              % (unreadable, os.path.join(rsi_dir(), "ledger.jsonl")), file=sys.stderr)
    if state_unreadable:
        print("[rsi] 주의: state.json 을 **읽을 수 없다**(%s) — 예산 이력의 한쪽 다리가 빠졌다"
              "(budget_unknown). 파일: %s"
              % (state_unreadable, os.path.join(rsi_dir(), "state.json")), file=sys.stderr)
    if not damaged:
        return
    print("[rsi] 주의: ledger.jsonl 에 판독 불가 %d줄(깨진 UTF-8·JSON 아님) — **어느 라운드의 "
          "시도인지 알 수 없어** 계수에 넣지 않았다(budget_unknown). 즉 이 라운드의 시도수는 "
          "확인된 것만 센 값이며, 손상 줄이 이 라운드의 시도였다면 실제보다 작을 수 있다. "
          "파일: %s — 손상 줄을 고치거나 걷어내면 계수가 정확해진다."
          % (damaged, os.path.join(rsi_dir(), "ledger.jsonl")), file=sys.stderr)


def _warn_stop(stop_reason, rid, attempts, damaged=0, unreadable="", state_unreadable=""):
    """종료 사유 고지(stderr) — exit code 는 바꾸지 않는다(소비자 루프를 세우지 않는다)."""
    _warn_damaged(damaged, unreadable, state_unreadable)
    if stop_reason == "stopped_budget":
        print("[rsi] stop_reason=stopped_budget — 라운드 '%s' 시도 %d회 > 상한 %d "
              "(CYS_RSI_MAX_ROUNDS). 라운드를 잇지 말고 격차를 보고하라(기록은 남았다)."
              % (rid, attempts, rsi_max_rounds()), file=sys.stderr)
    elif stop_reason == "stopped_stagnation":
        print("[rsi] stop_reason=stopped_stagnation — flat 연속 %d회 이상(ceiling). 같은 방법의 "
              "반복은 점수를 올리지 못한다: 방법을 바꾸거나 종결하라." % rsi_ceiling_flats(),
              file=sys.stderr)


def cmd_progress(a):
    state, state_unreadable = _read_state_file()
    r = state.get("rounds", {}).get(a.round)
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
    attempts, _prev, damaged, unreadable = _next_attempt(state, a.round)
    r["attempts"] = attempts
    stop_reason = rsi_stop_reason(attempts, r["flat_streak"], rsi_max_rounds(),
                                  rsi_ceiling_flats())
    r["stop_reason"] = stop_reason
    if damaged or unreadable or state_unreadable:
        r["budget_unknown"] = True
    _save_state(state)
    entry = {"event": "progress", "round": a.round, **rec,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _mark_unknown(entry, damaged, unreadable, state_unreadable)
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    _warn_stop(stop_reason, a.round, attempts, damaged, unreadable, state_unreadable)
    # 추천은 **라운드당 1회**다(다이제스트 1줄 계약). 종전엔 ceiling 이상인 매 progress 마다
    # 적재해 같은 사유가 큐에 쌓였다 — 배달 채널은 그대로(feed 0 · 주간 다이제스트).
    # ★래치는 state.json 밖에도 있어야 한다(codex R1 major-10): state 를 지우거나 되돌리면
    #   같은 라운드의 같은 사유가 두 번 적재됐다. 그래서 **라운드별 멱등키**를 ①큐 레코드와
    #   ②append-only ledger 양쪽에서 조회한다(큐가 소비돼도 ledger 가 남고, ledger 를 잃어도
    #   큐가 남는다). 적재 실패면 래치를 걸지 않는다 — 추천을 영구히 잃지 않기 위해서다.
    key = ceiling_digest_key(a.round)
    if r["flat_streak"] >= rsi_ceiling_flats():
        # ★내구 다리 셋을 **매번 함께** 본다(codex R2 major-11 B): 종전엔 state 래치가 참이면
        #   분기 자체에 들어오지 않아, "큐에서 복구해 래치만 세운" 프로세스가 ledger 를 영영
        #   비워 뒀다. 그 뒤 큐 회전 + state 소실이면 같은 추천이 다시 나간다.
        in_queue = digest_queue_has_key(learn_digest_queue_path(), key)
        in_ledger = _ledger_has_event("ceiling_recommend", a.round, key)
        # ★state 래치도 **키로 범위를 좁힌다**(독립 재유도 X-5): 신원 없는 구 키로 눌려 세워진
        #   래치(남의 추천을 보고 세운 것)를 그대로 인정하면 키를 고쳐도 억제가 풀리지 않는다.
        latched = bool(r.get("ceiling_recommended")) and r.get("ceiling_recommended_key") == key
        if latched or in_queue or in_ledger:
            if not in_ledger:
                # ★보조 기록의 실패가 **주 평가의 rc 를 바꾸지 않는다**(codex R2 major-11 A):
                #   `_append_ledger` 가 ENOSPC 로 던지면 progress 전체가 rc=1 이 되고
                #   `javis_learn.py:766` 이 그것을 fail(12)(일시적·재시도 가능)로 올려 이미
                #   끝난 평가가 재시도로 되돌아간다(§7 위험 ③ 방향). 실패는 삼키고 래치는
                #   세우지 않는다 — 다음 호출이 다시 메울 단서를 남긴다.
                if _safe_append_ledger({"event": "ceiling_recommend", "round": a.round,
                                        "key": key, "ts": time.time(), "backfilled": True}):
                    in_ledger = True
                else:
                    print("[rsi] 주의: ceiling 추천의 ledger 래치를 메우지 못했다 — 추천은 이미 "
                          "나갔고(큐 또는 state), 다음 호출이 다시 시도한다.", file=sys.stderr)
            if not latched and (in_queue or in_ledger):
                r["ceiling_recommended"] = True   # 이미 추천됨(다른 경로에서) — 래치 복원
                r["ceiling_recommended_key"] = key
                _save_state(state)
        elif _recommend_learn("ceiling", "%s 정체(ceiling) 돌파 방법론" % a.round, key):
            if _safe_append_ledger({"event": "ceiling_recommend", "round": a.round,
                                    "key": key, "ts": time.time()}):
                r["ceiling_recommended"] = True
                r["ceiling_recommended_key"] = key
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
