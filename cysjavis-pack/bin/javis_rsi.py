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


def _as_int_ok(v, default=0):
    """느슨한 정수 해석 → `(값, 해석됨?)`. 결측(None)은 '값이 아니다' 라 `(default, True)` 다 —
    '없음' 과 '읽었는데 쓸 수 없음' 을 가른다(후자만 예산 불확정으로 올린다).

    ★성찰 R4 N14: 종전엔 `OverflowError` 를 잡지 않았다. `state.json` 의 `attempts: 1e999` 는
      `json.load` 가 `inf`(float)로 읽고 `int(inf)` 가 OverflowError 를 던진다 — 그 예외가
      `cmd_checkpoint`/`cmd_progress` 를 통째로 죽여 **복구·완료 경로가 traceback 으로 끝났다**
      (`javis_learn.py:766` 이 rc≠0 을 fail(12)=일시 실패로 올려 끝난 평가가 재시도로 되돌아간다 ·
      §7 위험 ③ 방향). 이제 해석 불가는 기본값 + `ok=False` 이고, 호출부가 `budget_unknown` 으로
      **정직하게 표기**한다(모르는 것은 모른다 — 0 으로 조용히 접지 않는다)."""
    if v is None:
        return default, True
    try:
        return int(v), True
    except (TypeError, ValueError, OverflowError):
        return default, False


def _as_int(v, default=0):
    """느슨한 정수 해석 — 손상된 state 값이 판정을 죽이지 않게(결측은 값이 아니다)."""
    return _as_int_ok(v, default)[0]


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


# ★미러가 **건드리면 안 되는** 키(성찰 R4 N1 · codex 설계 비평 2): 이 파일의 라운드 레코드는
#   두 주인이 나눠 갖는다 — lifecycle(평가 결과)은 `javis_learn`/데몬 소유, RSI 계측은 RSI 소유다.
#   `learn evaluate` 가 RSI 에 **같은 `--round`** 를 넘기므로 두 쪽의 라운드 id 공간은 실제로 겹친다.
#   그래서 rid 단위 병합만으로는 부족하고, RSI 는 남의 필드를 **덮지도 지우지도 않는다**.
LEARN_OWNED_ROUND_KEYS = frozenset((
    "verdict", "stored", "harness", "items", "evaluator_hash", "schema",
    "attempts",            # learn 의 judge-shopping 시도수(RSI 것은 `rsi_attempts` 로 개명된다)
    "created_at",
))
LEARN_CANONICAL_STATE = "~/.cys/state"


def _mirror_daemon_owned(d):
    """미러 대상 디렉터리 `d` 가 **데몬 canonical 학습 디렉터리**인가(실경로 비교).

    ★왜 `javis_learn._is_canonical` 의 '루트 비교' 를 그대로 베끼지 않는가(codex 설계 비평 1):
      루트만 보면 **비canonical 루트 아래의 `learn` 만 canonical 로 심링크된** 형상을 통과시켜
      정본을 그대로 덮는다. 그리고 `CYS_ROUND_DIR` 이 없을 때의 `<팩>/round/learn` 별칭도 놓친다.
      여기서 지켜야 할 것은 '환경변수의 모양' 이 아니라 **쓰려는 자리가 데몬 것인가** 이므로
      대상 경로의 실경로를 canonical 의 실경로와 비교한다(루트 비교를 포함한다).
    ★판정 불가(OSError)는 **소유로 본다** — 데몬 파일일 수 있는 자리에 쓰지 않는 쪽이 안전 방향이다
      (자가치유 상태 전손이 §7 위험 ③ 이고, 미러 누락은 CC 학습 탭의 가시성 손실뿐이다)."""
    try:
        canon = os.path.realpath(os.path.expanduser(
            os.path.join(LEARN_CANONICAL_STATE, "learn")))
        return os.path.realpath(d) == canon
    except OSError:
        return True


def _mirror_learn_state(state):
    """rounds 를 데몬 가독 위치로 미러(best-effort) — 실패는 RSI 판정에 불간섭.

    ★성찰 R4 N1(blocking): 종전엔 payload 를 만들어 `os.replace` 로 **통째 치환**했다.
      · 상시 잡이 `CYS_ROUND_DIR="${CYS_ROUND_DIR:-$HOME/.cys/state}"` 를 박으면 이 미러의 대상이
        **데몬 canonical**(`~/.cys/state/learn/state.json`)이 된다. `javis_learn` 은 그 형상에서
        미러 쓰기를 끊는데(`_is_canonical`) RSI 는 같은 프로세스 트리에서 덮었고, 데몬이
        `daemon.learn_write` 락으로 지키던 lifecycle 라운드와 discovery 계수가 **전손**됐다
        (실측: `rounds.L1`(verdict·stored·harness·evaluator_hash) 소멸 · discovery 전부 0).
        프로세스 밖 `os.replace` 는 데몬 락이 막지 못한다.
      · canonical 이 아니어도 이 파일에는 **남의 라운드**가 있다(`javis_learn` 세션 모드 미러가
        같은 자리를 쓴다). 치환은 그것도 함께 지웠다.
      수정: ⓐ 대상이 데몬 것이면 **쓰지 않는다**(전파는 데몬 RPC 소관) ⓑ 아니면 rid 단위 병합에
      **필드 소유권**을 얹는다(위 `LEARN_OWNED_ROUND_KEYS`) ⓒ `discovery` 는 **보내지 않는다** —
      RSI 는 그 값을 갱신하는 코드가 없어 늘 기본 0 이었고, 그 0 이 데몬이 세운 계수를 덮었다
      (codex 설계 비평 2: 보존이 맞다 · 합은 재미러마다 중복 · 최대는 하향 정정을 봉쇄)
      ⓓ 기존 파일을 **읽지 못하면 쓰지 않는다** — 판독 실패를 빈 상태로 접고 저장하면 그 자체가
      새 전손 경로다(codex 설계 비평 4)."""
    try:
        d = _learn_state_dir()
        if _mirror_daemon_owned(d):
            return                       # 데몬 단일 writer 소유 — 미러 쓰기 금지(N1 ⓐ)
        mp = os.path.join(d, "state.json")
        cur = {}
        if os.path.exists(mp):
            try:
                with open(mp, encoding="utf-8") as f:
                    cur = json.load(f)
            except (OSError, ValueError) as e:
                print("[rsi] 주의: 학습 미러를 읽지 못해 이번 미러를 건너뛴다(%s) — 덮어쓰면 "
                      "남의 라운드가 사라진다." % e, file=sys.stderr)
                return                   # N1 ⓓ
            if not isinstance(cur, dict):
                print("[rsi] 주의: 학습 미러가 객체가 아니다 — 이번 미러를 건너뛴다.",
                      file=sys.stderr)
                return
        out = dict(cur)
        rounds_out = dict(out.get("rounds") or {}) if isinstance(out.get("rounds"), dict) else {}
        mine = state.get("rounds", {})
        for rid, rec in (mine.items() if isinstance(mine, dict) else []):
            base = rounds_out.get(rid)
            base = dict(base) if isinstance(base, dict) else {}
            for k, v in (_mirror_round_rec(rec) or {}).items():
                if k in LEARN_OWNED_ROUND_KEYS:
                    continue             # 남의 필드는 덮지 않는다(N1 ⓑ)
                base[k] = v
            rounds_out[rid] = base
        out["rounds"] = rounds_out
        # ★discovery 는 **손대지 않는다**(N1 ⓒ): `out` 은 기존 파일의 사본이므로 있던 값이 그대로
        #   남고, 없으면 만들지 않는다. RSI 가 기본 0 을 실어 데몬 계수를 0 으로 되돌리던 길이 닫힌다.
        os.makedirs(d, exist_ok=True)
        tmp = mp + ".tmp"
        open(tmp, "w", encoding="utf-8").write(json.dumps(out, ensure_ascii=False, indent=2))
        os.replace(tmp, mp)
    except Exception:                    # noqa: BLE001 — 미러 실패가 RSI 판정을 죽이지 않는다
        pass


def _save_state(state):
    d = rsi_dir()
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, "state.json")
    tmp = p + ".tmp"
    open(tmp, "w", encoding="utf-8").write(json.dumps(state, ensure_ascii=False, indent=2))
    os.replace(tmp, p)
    _mirror_learn_state(state)  # CC 학습 탭 배선(B-10)


# ★(0.14.31 · 성찰 확인 · major · 계획 N7) state.json **세 writer 공용** 잠금과 경합 rc.
#   writer 는 checkpoint · progress · ceiling 래치 셋이고, 셋이 **같은 잠금**을 써야만 상호배제가
#   성립한다(하나만 쥐면 그 사이로 나머지가 끼어든다 — 잠금이 있다는 착각이 더 나쁘다).
#   잠금 자체는 최선노력이다(`_best_effort_lock` doc): 파일계가 잠금을 지원하지 않으면
#   종전대로 진행하고(`unsupported`), 남이 쥐고 있으면(`blocked`) **쓰지 않는다**.
RSI_RC_BUSY = 4                           # 경합으로 아무것도 쓰지 않았다(일시 실패 · 재시도 가능)


def _state_lock():
    """state.json 트랜잭션 잠금 — 세 writer 가 이 한 자리를 공유한다."""
    d = rsi_dir()
    try:
        os.makedirs(d, exist_ok=True)
    except OSError:
        pass
    return _best_effort_lock(os.path.join(d, "state.json"))


def _latch_ceiling_recommended(rid, key):
    """추천 래치 **두 필드만** 최신 상태 위에 얹는다(보조 저장) → 성공 여부.

    ★성찰 R4 N7(major): 종전엔 이 자리에서 `_save_state(state)` 로 **함수 진입 때 읽은 상태 전체**
      를 다시 썼다. A 가 progress 로 H1 을 읽고 → 그 사이 B 가 H2 checkpoint 를 성공시켜 저장 →
      A 가 래치 두 필드를 세우려고 H1 전체를 재저장하면 `checkpoint_sha`·`ref`·attempts 가 H1 로
      **후퇴한다**. rollback 앵커가 잘못된 커밋을 가리키는 것은 되돌리기 사고 방향이다(§7 위험 ③).
      다른 rid·새 progress·`budget_unknown` 도 함께 잃었다(codex 설계 비평 3).
    ★순서는 그대로 둔다(codex 설계 비평 3): 주 저장 → 주 ledger → 추천/보조 ledger → **이 래치**.
      재읽기를 주 저장 앞으로 옮기면 메모리에만 있던 progress 가 단독 실행에서도 빠진다.
    ★판독 실패면 **쓰지 않는다**(codex 설계 비평 4): `_read_state_file` 의 사유를 버리고 빈 상태로
      접어 저장하면 그것이 새 전손 경로다. 래치는 편의(캐시)이고 내구 근거는 큐와 ledger 다.
    ★어떤 실패도 예외로 올리지 않는다(N14): 보조 저장의 ENOSPC 가 주 평가의 rc 를 바꾸면
      `javis_learn.py` 가 끝난 평가를 fail(12)(일시 실패)로 올려 재시도로 되돌린다."""
    try:
        # ★(성찰 확인 · N7) 재읽기만으로는 부족하다 — 읽기와 쓰기 **사이**에 남의 checkpoint 가
        #   저장되면 그것이 그대로 지워진다(래치는 성공을 반환해 아무도 모른다). 주 writer 와
        #   **같은 잠금** 안에서 읽고 쓴다. 못 쥐면 래치를 세우지 않는다 — 래치는 편의(캐시)이고
        #   내구 근거는 큐와 ledger 라, 다음 호출이 다시 메운다(§N7 실패 방향).
        with _state_lock() as lk:
            if lk.blocked:
                print("[rsi] 주의: 상태 잠금 경합으로 ceiling 래치를 남기지 못했다 — 추천은 이미 "
                      "나갔고 큐·ledger 가 중복을 막는다.", file=sys.stderr)
                return False
            cur, unreadable = _read_state_file()
            if unreadable:
                print("[rsi] 주의: 상태 판독 실패로 ceiling 래치를 남기지 못했다(%s) — 추천은 이미 "
                      "나갔고 큐·ledger 가 중복을 막는다." % unreadable, file=sys.stderr)
                return False
            r = cur.get("rounds", {}).get(rid)
            if not isinstance(r, dict):
                return False             # 그 라운드가 최신 상태에 없다 — 얹을 자리가 없다
            r["ceiling_recommended"] = True
            r["ceiling_recommended_key"] = key
            _save_state(cur)
        return True
    except Exception as e:               # noqa: BLE001 — 보조 저장은 주 평가의 rc 를 바꾸지 않는다
        print("[rsi] 주의: ceiling 래치 저장 실패(%s) — 큐·ledger 가 중복을 막는다." % e,
              file=sys.stderr)
        return False


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
    반환 (시도번호, 직전 라운드 레코드, 손상 줄 수, ledger 판독 불가 사유, 저장값 해석 불가?).

    ★성찰 R4 N14: 저장된 `attempts` 가 **해석 불가**(비유한 수·문자열)면 ledger 계수만으로 세되
      그 사실을 다섯째 값으로 올린다 — 호출부가 `budget_unknown` 을 세운다. 종전엔 그 값이
      예외(OverflowError)로 터지거나 조용히 0 으로 접혀 예산이 되돌려졌다."""
    prev = state.get("rounds", {}).get(rid)
    prev = prev if isinstance(prev, dict) else {}
    led, damaged, unreadable = _ledger_attempt_count(rid)
    stored, ok = _as_int_ok(prev.get("attempts"), 0)
    return max(stored, led) + 1, prev, damaged, unreadable, (not ok)


# ── RSI 학습 자율추천의 배달 채널: feed(건별 승인 요청) → 주간 다이제스트 큐 ──
# ★오너 승인 개정(2026-09-04 전면 감사): 자동 트리거(라운드 종료·eval ceiling)는 더 이상
#   `cys feed push --kind learn_proposal` 로 **건별 승인 요청**을 발행하지 않는다. 승인권은
#   오너뿐인데 자동 생성분 6건(최고령 19h48m)이 적체해 **실제 승인 요청**(SSH 프로브 9.5h)을
#   덮었다. RSI_LEARNING_DIRECTIVE §7-4 "접점 신설 시 다이제스트가 기본값"이 이미 정본이므로
#   코드를 정본에 맞춘 정합 수정이다(§3 도 같은 날 함께 개정).
# ★큐 경로·레코드 모양은 javis_orchestra.py 의 동명 함수와 **동형**이어야 한다(같은 큐에 쓴다).
#   패리티는 bin/tests/test_learn_digest_queue.py 가 기계 검증한다.
# ★성찰 R4 N10 — 경로 해소를 `_learn_state_dir()`(= `CYS_ROUND_DIR/learn` 우선)에서 **팩 고정**으로
#   되돌린다. 종전 주석은 "생산 형상에서는 orchestra 와 같다" 고 단언했지만 사실이 아니었다:
#     · 상시 잡이 `CYS_ROUND_DIR="${CYS_ROUND_DIR:-$HOME/.cys/state}"` 를 박는 형상이 **생산**이다.
#       그때 orchestra 는 `<팩>/round/learn/…` 에 적재하고 rsi 는 `<CYS_ROUND_DIR>/learn/…` 를 본다
#       → RSI ceiling 추천이 **아무도 읽지 않는 파일**에 쌓이고, 멱등 조회(`digest_queue_has_key`)도
#       그 파일만 봐서 같은 사유가 두 큐에 따로 쌓인다.
#     · `_learn_state_dir()` 은 팩 env 를 `CYS_PACK_DIR` **한 키만** 본다 — 레거시 env 기계
#       (`AITERM_PACK_DIR` 등)에서는 `CYS_ROUND_DIR` 이 없어도 두 모듈이 갈렸다(S19 계열).
#   그래서 큐 경로는 orchestra 와 **같은 규칙**(`pack_dir()` = `PACK_DIR_ENV_KEYS` 4키)으로 고정한다.
#   데몬 미러(`_mirror_learn_state`)는 여전히 `_learn_state_dir()` 이다 — 그쪽은 데몬이 읽는 자리이고
#   이 큐와 목적이 다르다.
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")


def pack_dir():
    """팩 경로 — 키 목록·순서는 `PACK_DIR_ENV_KEYS`(정본 `src/pack.rs` · 파리티는
    `bin/tests/test_todo_shared_constants.py` S19 가 기계 대조한다)."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


LEARN_DIGEST_QUEUE = "digest_queue.jsonl"


def learn_digest_queue_path():
    """주간 다이제스트 큐 파일 — `<팩>/round/learn/digest_queue.jsonl`(레인별 유일).
    ★`javis_orchestra.learn_digest_queue_path` 와 **같은 규칙**이어야 한다(같은 큐다 · N10)."""
    return os.path.join(pack_dir(), "round", "learn", LEARN_DIGEST_QUEUE)


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


# ★잠금 상수도 `javis_orchestra.py` 와 **같은 이름·같은 값**이다(성찰 R4 N13): 패리티 검체가
#   두 판본의 클래스를 AST 해시로 대조하는데, 기본 인자가 한쪽만 리터럴이면 같은 규율이 다른
#   소스로 보여 대조가 성립하지 않는다(그리고 상한을 한쪽만 고치는 길이 열린다).
WP6_LOCK_WAIT = 5.0                       # 최선노력 잠금 대기 상한(초) — 무한대기 금지
WP6_LOCK_STALE = 300.0                    # 고아 잠금으로 보는 나이(초) — 임계구간은 초 단위다


class _best_effort_lock(object):
    """`os.mkdir` 원자성만 쓰는 **최선노력** 상호배제 — fcntl·msvcrt 무의존(Windows 안전).

    ★이름 그대로다: 이것은 정합의 근거가 **아니다**(codex R1 D6 반례 — 대기 상한 뒤 진행하면
      동시 writer 가 되고, 고아 판정을 시간으로만 하므로 느린 소유자를 강탈할 수도 있다).
      정합은 ①결속 sha ②행 서수(`row_ordinal`) ③손상 감지 셋이 **fail-closed** 로 담당하고,
      이 잠금은 찢긴 append 의 **빈도를 줄이는 완충**일 뿐이다. 그래서 어떤 실패 경로에서도
      명령을 세우지 않는다 — 무한대기는 부트체인 ④(전 pane 사망) 방향이다.
    ★잠금 파일은 **task 장부 하나**로 통일한다(사이드카·장부 공용) — 서로 다른 두 잠금을 쓰면
      한쪽만 쥔 writer 가 그 사이로 끼어든다.
    ★이 클래스는 `javis_orchestra.py` 와 `javis_rsi.py` 에 **바이트 동형으로 복제**돼 있다
      (`javis_rsi` 는 독립 실행 도구라 orchestra 대형 모듈을 import 하지 않는다 — 배포 누락·검색
      경로라는 새 의존을 늘리지 않는 쪽을 골랐다). 복제가 갈리면 이번 라운드에 네 번 고쳐진 규율이
      한쪽에만 남으므로, 동형은 `bin/tests/test_refl_round_lock_ownership.py` 의 **AST 해시 패리티**
      가 기계로 잡고 **행동 검체는 두 판본에 각각** 돌린다(같은 결함은 패리티를 통과한다).
    """

    def __init__(self, path, wait=WP6_LOCK_WAIT, stale=WP6_LOCK_STALE):
        self.path, self.wait, self.stale, self.held = path + ".lock", wait, stale, False
        # ★두 실패를 **구분**한다(codex R2 blocking-1·3): `blocked` = 다른 writer 가 쥐고 있다
        #   (= 직렬화가 필요한 임계구간은 진행하면 안 된다) · `unsupported` = 이 파일계에서
        #   잠금 자체가 불가(권한·경로 길이 등 — 아무도 못 쥐므로 종전대로 완충 없이 진행하되
        #   그 사실을 호출자가 고지한다). 둘을 뭉치면 Windows 경로 한계 같은 형상에서 기록이
        #   전면 거부돼 부트체인 ④ 방향이 된다.
        self.blocked, self.unsupported = False, ""
        # ★내 토큰을 **실제로 남겼는가**(주인님 규율 "쓰기 후 되읽기"): owner 쓰기가 성공했다면
        #   빈 owner 는 내 잠금이 아니다(회수 중이거나 남의 새 잠금이다) — 반납 대상이 아니다.
        self.wrote_owner = False
        # ★게시 결과를 **네 상태로 가른다**(성찰 R4 N6 · codex (c) "서로 다른 상태를 False 하나로
        #   합치지 마라"): "published"(내 토큰이 되읽기로 확인됨) · "unwritten"(파일을 못 만들었거나
        #   못 썼다 — 이 파일계의 한계) · "lost"(**남의 토큰이 보인다 = 소유권 상실의 증거**) ·
        #   ""(아직 시도하지 않음). `lost` 는 파일계 한계가 아니라 경합의 결과이므로 진입을 막는다.
        self.owner_state = ""
        # ★세대(보조 거부 전용 · codex (d)): `mkdir` 직후의 `(st_dev, st_ino)`. **신원 증명이
        #   아니다** — `st_ino` 가 0 이거나 재사용되는 파일계가 있으므로 '같다'는 아무것도 증명하지
        #   않는다. 유효한 값이 **다를 때만** '이 디렉터리는 내 것이 아니다' 로 읽어 삭제를 거부한다.
        #   ctime 은 쓰지 않는다: owner 생성이 디렉터리 ctime 을 바꾸는 파일계에서 정상 소유자가
        #   자기 잠금을 영영 반납하지 못한다(= 회수 불능 잔재 = 부트체인 ④ 방향).
        self.gen = None
        # ★회수는 `__enter__` 당 **한 번**만 시도한다(독립 재유도 X-1): 회수 뒤에도 잠금이 살아
        #   있으면 그것은 고아가 아니라 **새 소유자**다. 연쇄 회수를 허용하면 대기 상한이 상한을
        #   넘긴 나이의 새 잠금까지 강탈해 두 writer 를 만든다(수리 전 실측 형상).
        self.reclaim_tried = False
        # ★청구 파일명은 **짧게**(reviewer-codex Windows 노트): 종전 `owner.stale-<pid>-<ms>-
        #   <16hex>` 는 약 50자라 MAX_PATH 제약 경로에서 `os.rename` 이 실패해 고아가 회수
        #   불능이 될 수 있었다(같은 방향의 교착). 파일명은 `owner.<8hex>`(14자)로 두고,
        #   소유자 식별에 쓰는 긴 토큰은 **파일 내용**에 남긴다(경로 길이와 무관).
        self.tag = hashlib.sha256(os.urandom(16)).hexdigest()[:8]
        self.token = ("%d-%d-%s" % (os.getpid(), int(time.time() * 1000),
                                    self.tag)).encode("ascii")

    def _owner_file(self):
        return os.path.join(self.path, "owner")

    def _owner(self):
        try:
            with open(self._owner_file(), "rb") as f:
                return f.read(200)
        except OSError:
            return b""

    def _claim_name(self):
        """회수 청구 파일 — 잠금 디렉터리 **안**의 `owner.<8hex>`(청구자마다 유일 · 짧다)."""
        return self._owner_file() + "." + self.tag

    @staticmethod
    def _is_claim(name):
        """청구 파일 이름인가 — 신형 `owner.<8hex>` · 구형 `owner.stale-<토큰>`(호환).

        구형을 계속 인정하지 않으면 이전 판이 남긴 잔재가 '모르는 내용물'이 되어 그 잠금이
        영구 교착으로 남는다(회수 불능 = 모든 기록 거부 · 부트체인 ④ 방향).
        """
        if not name.startswith("owner."):
            return False
        rest = name[len("owner."):]
        if rest.startswith("stale-"):
            return True
        return len(rest) == 8 and all(c in "0123456789abcdef" for c in rest)

    def obstruction(self):
        """이 자리가 **자동 회수될 수 없는 형상**인가 → `"file"` · `"contents"` · `""`.

        ★왜(성찰 R4 N17): 안내문은 "고아 잠금은 300초 뒤 자동 회수" 라고 말하는데, `<장부>.lock`
          자리에 **일반 파일**이 있거나 잠금 디렉터리에 **미지 내용물**이 있으면 두 회수 경로가
          모두 손대지 않는다(보류 방향 — 옳다). 그러면 그 task 의 라운드 기록은 사람이 지울
          때까지 영구 거부인데 조작자는 안내를 믿고 기다린다. 문면을 가르기 위한 판별이다.
          · "file"     = 잠금 자리에 일반 파일(또는 비디렉터리)이 있다 — `mkdir` 이 영영 실패한다
          · "contents" = 잠금 디렉터리에 청구 파일이 아닌 내용물이 있다 — 회수가 손대지 않는다
          · ""         = 회수 가능한 형상(살아 있는 소유자 · 고아 · 빈 잠금)이거나 판독 불가
        """
        try:
            names = os.listdir(self.path)
        except NotADirectoryError:
            return "file"
        except OSError:
            try:
                if os.path.exists(self.path) and not os.path.isdir(self.path):
                    return "file"
            except OSError:
                pass
            return ""
        for n in names:
            if n != "owner" and not self._is_claim(n):
                return "contents"
        return ""

    def _stat_gen(self):
        """이 잠금 디렉터리의 세대 후보 `(st_dev, st_ino)` — 유효하지 않으면 None(순수하지 않음).
        0 은 신원이 아니다(Windows·일부 파일계가 `st_ino` 를 0 으로 낸다)."""
        try:
            st = os.stat(self.path)
        except OSError:
            return None
        dev, ino = getattr(st, "st_dev", 0), getattr(st, "st_ino", 0)
        return (dev, ino) if (dev and ino) else None

    def _gen_ok(self):
        """세대 **보조 거부**(codex (d)) — 유효한 신원이 **다르면** 남의 디렉터리다.
        같거나 신원을 못 얻으면 여기서는 아무것도 증명하지 않는다(증거는 토큰이다)."""
        if self.gen is None:
            return True
        cur = self._stat_gen()
        return cur is None or cur == self.gen

    def _publish_owner(self):
        """소유권 **원자 게시** → `"published"` · `"lost"` · `"unwritten"`.

        ★왜 `O_EXCL` 인가(성찰 R4 N6): 종전은 `mkdir` 직후 owner 를 **덮어썼다**. 그래서 A 가
          `mkdir` 성공 뒤 owner 를 쓰기 전에 상한을 넘겨 지연되면 → B 가 빈 잠금을 회수·재획득해
          자기 owner 를 게시하고 → 재개한 A 가 **B 의 잠금 안에** 자기 토큰을 덮어써서 되읽기까지
          성공했다. 둘이 동시에 임계구간에 들고, A 의 반납이 B 의 잠금을 지웠다.
          생성 배타(`O_EXCL`)는 그 창을 닫는다 — 게시에 성공한 **하나만** 소유자다.
        ★되읽기가 **남의 토큰**이면 그것도 상실이다(codex (c) 반례): `O_EXCL` 에 성공하고 쓰기
          전에 지연되면 B 가 그 빈 owner 를 회수해 갈 수 있다. 그때 A 의 쓰기는 이미 unlink 된
          fd 로 가고 경로 되읽기에는 B 의 토큰이 보인다 — 진입하면 안 된다.
        ★`unwritten` 은 파일계 한계다(만들 수도 쓸 수도 없다). 그 경우는 종전 표면을 유지한다 —
          여기서 기록을 전면 거부하면 owner 파일을 못 만드는 파일계에서 라운드 기록이 영구
          불능이 된다(부트체인 ④ 방향).
        """
        try:
            fd = os.open(self._owner_file(), os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        except FileExistsError:
            return "lost"
        except OSError:
            return "unwritten"
        try:
            os.write(fd, self.token)
        except OSError:
            return "unwritten"
        finally:
            try:
                os.close(fd)
            except OSError:
                pass
        got = self._owner()
        if got == self.token:
            return "published"
        if got == b"":
            return "unwritten"
        return "lost"

    def _abandoned_claim(self):
        """중단된 회수가 남긴 청구 파일 하나 — owner 가 없고 나이가 상한을 넘은 것만.

        ★rename 성공과 unlink 사이에서 프로세스가 죽으면 owner 가 사라진 잠금이 남는다.
          그 상태는 아무도 청구할 수 없어 **영구 교착**이 된다(모든 기록이 거부된다 —
          자율 루프를 세우는 방향). 청구 파일은 회수자가 1~2 syscall 만 쥐는 임시물이므로
          나이가 상한을 넘은 것은 버려진 것으로 본다.
        """
        try:
            names = sorted(os.listdir(self.path))
        except OSError:
            return None
        if "owner" in names:
            return None
        for n in names:
            if not self._is_claim(n):
                return None          # 모르는 내용물이 있다 — 손대지 않는다(보류 방향)
        for n in names:
            p = os.path.join(self.path, n)
            try:
                if time.time() - os.path.getmtime(p) > self.stale:
                    return p
            except OSError:
                return None
        return None

    def _reclaim_empty(self):
        """owner 도 청구 잔재도 없는 **빈 잠금 디렉터리**를 회수한다 — 반환 회수 여부.

        ★X-1 수리의 회귀를 닫는다(reviewer-claude·reviewer-codex 잔여 major): rename 청구는
          owner 파일이 있을 때만 동작하고 잔재 회수는 청구 파일이 있을 때만 동작한다. 그런데
          `__enter__` 는 `os.mkdir` 성공 **직후**에 owner 를 쓰므로 그 사이의 SIGKILL·전원
          장애·페인 종료는 물론 **KeyboardInterrupt(Ctrl-C)** — `except OSError` 에 걸리지
          않아 `__enter__` 밖으로 빠져나가고 `__exit__` 도 돌지 않는다 — 가 **빈** 잠금
          디렉터리를 남긴다. 그 형상은 두 회수 경로 모두에 걸리지 않아 **영구 교착**이 됐다:
          이후 모든 `round-log`·다이제스트 큐 기록이 사람이 `rm -rf <잠금>` 을 할 때까지
          거부된다(부트체인 ④ 방향 · 안내문 "300초 뒤 자동 회수" 가 거짓말이 된다).
        ★`os.rmdir` 는 디렉터리가 **비었을 때만** 성공하므로 원자성은 그대로다: 그 사이 누군가
          owner 를 썼다면 실패하고 우리는 아무것도 지우지 않는다. 나이도 다시 확인한다 —
          갓 만들어진 빈 잠금은 owner 를 쓰는 중인 **살아 있는** 소유자다.
        ★그래도 '살아 있는 소유자를 회수해 버릴' 창은 남는다(codex: 회수 전 stat 은 삭제 권한을
          예약하지 않는다). 그 잔여 창은 **피해자 쪽에서** 닫는다 — `_publish_owner` 의 배타
          생성이 실패하면 그 프로세스는 자기가 잠금을 잃었음을 알고 임계구간에 들지 않는다.
        """
        try:
            if os.listdir(self.path):
                return False                   # 내용이 있다 — 여기서 다룰 형상이 아니다
            if time.time() - os.path.getmtime(self.path) <= self.stale:
                return False                   # 살아 있는 소유자가 owner 를 쓰는 중일 수 있다
            os.rmdir(self.path)                # 비어 있을 때만 성공한다
        except OSError:
            return False
        return True

    def _claim_and_unlink(self, src, expect):
        """`src` 를 **원자적으로 청구(rename)** 하고, 옮긴 그 바이트가 `expect` 일 때만 지운다.
        반환 True = 지웠다. `expect is None` 이면 내용을 확인하지 않는다(이미 죽은 소유자의 잔재).

        ★읽는 바이트와 지우는 바이트를 같게 만드는 것이 이 함수의 존재 이유다: 경로 기반
          `확인 → unlink` 는 확인 **뒤** 바뀐 소유자의 owner 를 지운다(codex "반납 TOCTOU").
          내 것이 아니면 **되돌린다**.
        """
        claim = self._claim_name()
        try:
            os.rename(src, claim)
        except OSError:
            return False                       # 남이 먼저 청구했거나 소유자가 바뀌었다
        if expect is not None:
            try:
                with open(claim, "rb") as f:
                    got = f.read(200)
            except OSError:
                got = None
            if got != expect:
                try:
                    os.rename(claim, src)
                except OSError:
                    pass
                return False
        try:
            os.unlink(claim)                   # 청구한 바이트만 지운다
        except FileNotFoundError:
            return False                       # 디렉터리가 통째로 바뀌었다 — 손대지 않는다
        except OSError:
            try:
                os.rename(claim, src)
            except OSError:
                pass
            return False
        return True

    def _reclaim_orphan(self, seen):
        """고아 잠금 회수 — **원자적 청구(rename)에 성공한 하나만** 회수자다. 반환 회수 여부.

        ★확인과 삭제가 원자적이지 않으면 확인 **뒤** 소유자가 바뀔 수 있다(독립 재유도 X-1):
          종전엔 토큰을 두 번 읽고 지웠는데, 마지막 확인 뒤 새 소유자가 들어오면 그 잠금을
          지우고 자기가 쥐었다 — 두 writer 가 동시에 임계구간에 들고, 최초 장부 생성 창에서는
          뒤늦은 `os.replace` 가 먼저 커밋된 행을 **흔적 없이** 덮는다. 재확인을 늘리는 것은
          창을 좁힐 뿐 닫지 못한다.
        ★그래서 회수 권한을 **rename 으로 청구**한다(`_claim_and_unlink`): owner 파일을 옮기는 데
          성공한 프로세스만 회수자이고, **옮긴 그 파일**에서 소유자를 확인한다(읽는 바이트와
          지우는 바이트가 같다). 내가 본 고아가 아니면 되돌린다.
        ★마지막 `rmdir` 는 디렉터리가 **비었을 때만** 성공한다 — 그 사이 새 소유자가 들어와
          owner 를 썼다면 실패하고, 우리는 아무것도 지우지 않는다.
        ★남는 창(정직한 표기): 새 소유자가 `mkdir` 에 성공하고 owner 를 **게시하기 전**의 순간에
          우리 `rmdir` 가 들어가면 여전히 겹칠 수 있다. 그 창은 이 완충으로 닫히지 않고, 피해자
          쪽의 배타 게시 실패(`_publish_owner` → `lost`)가 진입을 막는 것으로 막는다.
        """
        src = self._owner_file()
        if not os.path.exists(src):
            src = self._abandoned_claim()      # 중단된 회수의 잔재 — 교착을 푸는 유일한 길
            if not src:
                return self._reclaim_empty()   # owner 도 잔재도 없는 **빈** 잠금(영구 교착)
            seen = None                        # 잔재의 내용은 이미 죽은 소유자의 토큰이다
        if not self._claim_and_unlink(src, seen):
            return False
        try:
            os.rmdir(self.path)                # 비어 있을 때만 성공한다
        except OSError:
            return False
        return True

    def __enter__(self):
        # ★단조 시계(codex R2 major-7 C): `time.time()` 은 시스템 시각이 뒤로 밀리면 대기 상한이
        #   늘어난다("5초 상한" 이 깨진다). 대기·나이 판정 모두 monotonic 을 쓴다.
        deadline = time.monotonic() + self.wait
        while True:
            made = False
            try:
                os.mkdir(self.path)
                made = True
            except FileExistsError:
                pass
            except OSError as e:
                self.unsupported = "%s" % e
                return self                    # 잠글 수 없는 파일계 — 완충 없이 진행
            if made:
                # 세대는 **삭제 거부**에만 쓴다(보조) · 소유권은 배타 게시가 증명한다.
                self.gen = self._stat_gen()
                self.owner_state = self._publish_owner()
                self.wrote_owner = self.owner_state == "published"
                if self.owner_state != "lost":
                    self.held = True
                    return self
                # 게시 경쟁에서 졌다 — 이 자리는 남의 잠금이다. 아무것도 지우지 않고 다시 기다린다.
                self.gen = None
            if time.monotonic() >= deadline:
                self.blocked = True            # 다른 writer 가 쥐고 있다(대기 상한)
                return self                    # 멈추지 않는다 — 판단은 호출자 몫
            # 고아 회수는 나이가 상한을 넘고 그 사이 소유자 토큰이 바뀌지 않았을 때만 **청구**
            # 한다(살아 있는 소유자를 즉시 강탈하는 폭을 좁힌다). 실제 삭제는 `_reclaim_orphan`
            # 이 **원자적 청구에 성공했을 때만** 한다 — 재확인만으로는 창이 닫히지 않는다.
            try:
                if not self.reclaim_tried and \
                        time.time() - os.path.getmtime(self.path) > self.stale:
                    seen = self._owner()
                    time.sleep(0.2)
                    if seen == self._owner() and \
                            time.time() - os.path.getmtime(self.path) > self.stale:
                        self.reclaim_tried = True
                        self._reclaim_orphan(seen)   # 고아 회수 — 임계구간은 초 단위다
                    continue
            except OSError:
                pass
            time.sleep(0.05)

    def __exit__(self, *exc):
        if self.held:
            self._release()
            self.held = False
        return False

    def _release(self):
        """반납 — **내 잠금일 때만** 지운다(codex R2 major-6).

        느린 소유자가 고아로 오인돼 회수된 뒤 그대로 rmdir 하면 **다음 소유자의 잠금**을 지워
        둘이 동시에 임계구간에 든다. 그래서 ①세대가 유효하게 다르면 손대지 않고 ②owner 삭제는
        경로 unlink 가 아니라 **원자 청구 후 바이트 확인**으로 한다(확인과 삭제 사이에 소유자가
        바뀌는 창을 닫는다 — codex "반납 TOCTOU").
        토큰이 **비어 있으면**(owner 게시 실패) 우리가 만든 것이므로 반납한다 — 아니면 아무도
        못 푸는 잠금이 상한만큼 남아 모든 기록을 막는다(부트체인 ④ 방향). 단 내 토큰이
        **게시 확인**된 경우에는 빈 owner 를 내 것으로 보지 않는다(codex 잔여 지적): 새 소유자가
        mkdir 하고 owner 를 쓰기 **전**의 빈 상태를 옛 소유자가 자기 것으로 오인하면 남의 잠금을
        지운다.
        """
        if not self._gen_ok():
            return                             # 세대가 다르다 — 이 디렉터리는 내 잠금이 아니다
        own = self._owner()
        if own == self.token:
            if not self._claim_and_unlink(self._owner_file(), self.token):
                return
        elif own == b"" and self.owner_state != "published":
            try:
                os.unlink(self._owner_file())
            except OSError:
                pass
        else:
            return                             # 남의 토큰 — 손대지 않는다
        try:
            os.rmdir(self.path)
        except OSError:
            pass


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
    # ★(0.14.31 · 성찰 확인 · major · 계획 N7) **읽기→변이→쓰기는 공용 잠금 안에서 한다.**
    #   state.json 의 writer 는 셋이다(checkpoint · progress · ceiling 래치). 셋 다 잠금 없이
    #   read-modify-write 를 했으므로 두 프로세스가 겹치면 **나중 쓰기가 먼저 쓰기를 통째로
    #   되돌린다**: A 가 H1 을 읽는 사이 B 가 H2·attempts 2 를 저장하면, A 의 저장이 그것을 H1·1 로
    #   내려앉히고 그 뒤 rollback 이 **틀린 커밋**을 앵커로 잡는다(되돌리기 사고 방향 · §7 위험 ③).
    #   N7 이 요구한 '세 writer 를 덮는 공유 트랜잭션 잠금' 이 이것이다.
    # ★잠금을 못 쥐면 **쓰지 않는다**(enqueue_learn_digest 와 같은 규율) — 경합 중 쓰기는 곧
    #   남의 기록 소실이다. 호출자(javis_learn)는 비-0 을 일시 실패(12)로 받아 재시도한다.
    with _state_lock() as lk:
        if lk.blocked:
            print("error: state.json 잠금 경합 — 다른 rsi writer 가 쥐고 있다. 아무것도 쓰지 "
                  "않았다(재시도 가능).", file=sys.stderr)
            return RSI_RC_BUSY
        state, state_unreadable = _read_state_file()
        # ★WP-6: 재checkpoint 는 라운드 레코드를 새로 쓰지만 **시도 이력은 이월한다**.
        #   (종전엔 통째 덮어쓰기라 같은 라운드를 다시 시작하면 상한이 리셋됐다 —
        #    ceiling 신호(flat_streak)도 같은 이유로 이월한다: 재시작이 정체를 지우면 안 된다.)
        attempts, prev, damaged, unreadable, budget_bad = _next_attempt(state, a.round)
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
        if damaged or unreadable or state_unreadable or budget_bad:
            # ★N14: `budget_bad` = 저장된 `attempts` 를 **읽었는데 쓸 수 없었다**(비유한 수 등).
            #   그것을 0 으로 접으면 예산이 조용히 되돌아가므로 불확정으로 표기한다.
            rec["budget_unknown"] = True      # 영속 state 에도 싣는다(다음 호출이 이어 읽는다)
        state.setdefault("rounds", {})[a.round] = rec
        state["current_round"] = a.round
        _save_state(state)
    entry = {"event": "checkpoint", "round": a.round, "sha": head[:12],
             "score": a.score, "ts": ts, "ref": ref,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _mark_unknown(entry, damaged, unreadable, state_unreadable, budget_bad)
    _append_ledger(entry)
    print(json.dumps(entry, ensure_ascii=False))
    _warn_stop(stop_reason, a.round, attempts, damaged, unreadable, state_unreadable)
    return 0


def _mark_unknown(entry, damaged, unreadable="", state_unreadable="", budget_bad=False):
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
    if budget_bad:
        # ★성찰 R4 N14 — 저장된 `attempts` 를 읽었는데 쓸 수 없었다(비유한 수·문자열). 종전엔
        #   `int(inf)` 가 OverflowError 로 명령 전체를 죽였다. 이제 사유를 남기고 계속 간다.
        entry["state_attempts_unusable"], entry["budget_unknown"] = True, True
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
    # ★(0.14.31 · 성찰 확인 · major · 계획 N7) **읽기→변이→쓰기는 공용 잠금 안에서 한다.**
    #   state.json 의 writer 는 셋이다(checkpoint · progress · ceiling 래치). 셋 다 잠금 없이
    #   read-modify-write 를 했으므로 두 프로세스가 겹치면 **나중 쓰기가 먼저 쓰기를 통째로
    #   되돌린다**: A 가 H1 을 읽는 사이 B 가 H2·attempts 2 를 저장하면, A 의 저장이 그것을 H1·1 로
    #   내려앉히고 그 뒤 rollback 이 **틀린 커밋**을 앵커로 잡는다(되돌리기 사고 방향 · §7 위험 ③).
    #   N7 이 요구한 '세 writer 를 덮는 공유 트랜잭션 잠금' 이 이것이다.
    # ★잠금을 못 쥐면 **쓰지 않는다**(enqueue_learn_digest 와 같은 규율) — 경합 중 쓰기는 곧
    #   남의 기록 소실이다. 호출자(javis_learn)는 비-0 을 일시 실패(12)로 받아 재시도한다.
    with _state_lock() as lk:
        if lk.blocked:
            print("error: state.json 잠금 경합 — 다른 rsi writer 가 쥐고 있다. 아무것도 쓰지 "
                  "않았다(재시도 가능).", file=sys.stderr)
            return RSI_RC_BUSY
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
        attempts, _prev, damaged, unreadable, budget_bad = _next_attempt(state, a.round)
        r["attempts"] = attempts
        stop_reason = rsi_stop_reason(attempts, r["flat_streak"], rsi_max_rounds(),
                                      rsi_ceiling_flats())
        r["stop_reason"] = stop_reason
        if damaged or unreadable or state_unreadable or budget_bad:
            r["budget_unknown"] = True        # ★N14 — 해석 불가한 저장 예산도 불확정이다
        _save_state(state)
    entry = {"event": "progress", "round": a.round, **rec,
             "attempts": attempts, "max_rounds": rsi_max_rounds(),
             "stop_reason": stop_reason}
    _mark_unknown(entry, damaged, unreadable, state_unreadable, budget_bad)
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
                # 이미 추천됨(다른 경로에서) — 래치 복원. ★N7: 여기서 `state` 전체를 다시 쓰면
                #   그 사이 남이 저장한 checkpoint 가 후퇴한다. 두 필드만 최신 상태에 얹는다.
                if _latch_ceiling_recommended(a.round, key):
                    r["ceiling_recommended"] = True
                    r["ceiling_recommended_key"] = key
        elif _recommend_learn("ceiling", "%s 정체(ceiling) 돌파 방법론" % a.round, key):
            if _safe_append_ledger({"event": "ceiling_recommend", "round": a.round,
                                    "key": key, "ts": time.time()}):
                # ★N7: 두 필드만 최신 상태에 얹는다(전체 재저장 금지 — 위 헬퍼 주석 참조).
                if _latch_ceiling_recommended(a.round, key):
                    r["ceiling_recommended"] = True
                    r["ceiling_recommended_key"] = key
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
