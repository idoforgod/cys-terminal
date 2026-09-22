#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_compete — 레버① COMPETE(경쟁 생성 프로토콜)의 결정론 지점.

QUALITY_LEVERS_DESIGN §1 구현. COMPETE는 기존 품질 체계의 **대체가 아니라 앞 단계(R0)**다
— 리뷰 라운드에 들어갈 "초안"을 1개가 아니라 N개 중 최선으로 만든다(§1.0). 이 스크립트는
그 흐름(§1.3) 중 **결정론 지점(§1.4)**만 담당한다: 다양화 격리 공간 생성·익명 심판 판정·
격리 공간 청소. 생성·심판 자체(LLM 노동)는 master/워커/리뷰어가 수행한다.

핵심 불변식:
- **분산 강제**: 후보별 접근축(--approach)이 N개 미만이거나 서로 같으면 init 거부(exit 2).
  분산 없는 N배 생성은 같은 답 N벌 — 경쟁이 허상이다(설계 공격 #1 해소).
- **점수 채널 원천 부재**: score/grade/rating 어떤 수치 등급도 만들지 않는다. 판정은
  관문 탈락 유무 → 반박 생존 우열(질적 서열) → 불일치 시 재유도(exit 코드)로만 가린다.
  javis_verdict.py 가 어느 깊이든 점수를 거부하는 것과 정합(§1.4).
- **익명 심판**: 심판은 실명 아닌 라벨(cand_A/cand_B)만 본다. 라벨↔실경로↔접근축 매핑은
  MAPPING.json(master 전용)으로 분리하고, MANIFEST.json 에는 심판에게 무해한 정보(라벨·
  트랙·criteria 해시)만 남긴다(§1.3-J4 심판 편향 완화). 단 후보와 심판이 같은 파일시스템을
  공유하므로 완전한 코드적 격리는 불가하다 — 이 분리는 우발 노출 완화(defense-in-depth)이지
  봉인이 아니다.
- **책임 경계**: 발동 게이트 4종 중 넓은 솔루션공간 판단·비용 승인·리뷰어 부하는 master
  책임(이 스크립트 밖)이다. 이 스크립트는 CRITERIA 잠금·다양성만 결정론으로 강제한다.
- **부트 무접촉**: preflight 미등록. 기존 파일(orchestra·preflight·verdict) 무수정 —
  round-log 는 subprocess 외부 호출로만 건드린다.

서브커맨드:
  init  : 관문 파일 검증·sha256 잠금 + 다양성 게이트 + 격리 공간 N개 생성 + MANIFEST 산출.
          CODE 트랙=git worktree / TEXT 트랙=candidates 디렉토리(git 불요).
  judge : verdicts/ 의 익명 verdict 를 javis_verdict 로 일괄 검증 → 판정 규칙(순수 함수):
          ①BLOCK=관문 탈락 ②생존 1개=승자 ③복수 생존=반박 생존 서열(질적) ④서열 불능
          =exit 4(독립 재유도 필요) ⑤재유도 후 동급=exit 5(master 전략 결정). 승자 확정 시
          RESULT.md + round-log 외부 호출(장부는 best-effort).
  clean : 격리 공간·브랜치 폐기(수명주기 완결). RESULT.md·MANIFEST.json 은 보존(기록).

사용:
    python3 javis_compete.py init --task T --n 2 --criteria C.md --track code \\
            --approach "정공법·검증우선" --approach "역발상·사용자우선" --repo R [--setup CMD]
    python3 javis_compete.py judge --task T [--reelicited]
    python3 javis_compete.py clean --task T [--force]
    python3 javis_compete.py --self-test          # 밀폐 자기검증(git·전역 무접촉)

종료 코드:
  init  : 0 성공 · 2 게이트 위반(관문 부재·approach 부족/중복)·인자/IO 오류
  judge : 0 승자 확정 · 2 verdict 계약 위반·후보 verdict 누락·IO 오류 ·
          3 전 후보 관문 탈락(재생성 필요) · 4 서열 불능(독립 재유도 필요) ·
          5 재유도 후에도 동급(master 전략 결정 요청)
  clean : 0 성공 · 2 비승자 태스크 --force 없이·격리 공간 제거 실패(고아 잔존)·IO 오류
의존성: 파이썬 표준 라이브러리 + (judge 시) 형제 javis_verdict 모듈. 네트워크·LLM 없음.
"""

import argparse
import hashlib
import json
import os
import re
import string
import subprocess
import sys
sys.dont_write_bytecode = True  # SEAL-1 층4: 호출자 env 와 무관하게 형제 import 의 __pycache__ 기록 차단(D-pyc 2026-09-21)
import time

LABELS = string.ascii_uppercase  # cand_A, cand_B, cand_C ...
ORCHESTRA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_orchestra.py")


# ── 경로(grill_gate.py 하우스 스타일 준수 — CYS_ROOT 오버라이드 가능) ──────────
def _cys_root():
    v = os.environ.get("CYS_ROOT", "").strip()
    if v:
        return v
    return os.path.join(os.path.expanduser("~"), "Desktop", "CYSjavis")


def _safe(task):
    """task id → 파일시스템 안전 토큰.

    치환(안전문자 외 → '_')·절단(>80)이 발생하면 서로 다른 task 가 같은 토큰으로 뭉개져
    같은 디렉토리를 공유할 수 있다(F5 클로버). 이 경우 원본 task 의 sha256 8자를 접미로
    붙여 유일성을 보장한다 — 무손실 task 는 접미 없이 그대로 둔다."""
    raw = str(task)
    tok = re.sub(r"[^0-9A-Za-z가-힣_.-]", "_", raw)
    safe = tok[:80]
    if safe != raw:  # 치환 또는 절단 발생 — 원본 해시 접미로 충돌 방지
        suffix = "_" + hashlib.sha256(raw.encode("utf-8")).hexdigest()[:8]
        safe = tok[:80 - len(suffix)] + suffix
    return safe


def compete_dir(task):
    return os.path.join(_cys_root(), "_round", "compete", _safe(task))


def manifest_path(task):
    return os.path.join(compete_dir(task), "MANIFEST.json")


def mapping_path(task):
    return os.path.join(compete_dir(task), "MAPPING.json")


def verdicts_dir(task):
    return os.path.join(compete_dir(task), "verdicts")


def result_path(task):
    return os.path.join(compete_dir(task), "RESULT.md")


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


# ── 다양성 게이트(순수 함수 — self-test 박제) ──────────────────────────────
def check_approaches(approaches, n):
    """(ok, reason). 후보별 접근축이 N개 이상이고 서로 다를 때만 통과.

    분산 없는 N배 생성은 토큰 낭비(설계 공격 #1)이므로 결정론으로 강제한다.
    정규화(공백 접기·소문자·구두점 제거) 후 중복을 판정 — 표면 차이 위장 차단."""
    approaches = list(approaches or [])
    if len(approaches) < n:
        return False, ("접근축 %d개 < 후보 %d개 — 후보마다 서로 다른 공략축을 "
                       "--approach 로 N개 지정하라(다양성 강제·§1.3-S2)"
                       % (len(approaches), n))
    norm = [re.sub(r"[\s\W_]+", "", a.strip().lower()) for a in approaches[:n]]
    if any(not x for x in norm):
        return False, "빈 접근축이 있다 — 각 후보의 공략축을 서술하라"
    if len(set(norm)) < n:
        return False, ("접근축이 서로 동일하다(정규화 후 %d종) — 같은 목표·같은 관문, "
                       "다른 공략축이어야 경쟁이 실효(§1.1)" % len(set(norm)))
    return True, "ok"


# ── 판정 규칙(순수 함수 — self-test 박제·점수 채널 원천 부재) ───────────────
# 생존 후보 서열용 질적 티어(낮을수록 우수). BLOCK 은 관문 탈락이라 여기 없음.
# 개수 세기·합산 아님 — 후보가 받은 verdict 중 '가장 나쁜 것'으로 티어를 정하는 ordinal 선택.
_SURVIVOR_TIER = {"ACCEPT": 0, "REVISE": 1, "INVESTIGATE": 1, "ESCALATE": 2}


def judge_candidates(cand_verdicts, reelicited=False):
    """후보별 verdict 쌍 목록 → 판정. (outcome, payload, exit_code, detail) 반환.

    cand_verdicts: {label: [(orig, eff), ...]}
      orig = validate_verdict 통과 후 **강등 전 원본** enum(관문 탈락 판정용)
      eff  = INVESTIGATE 강등까지 반영한 **유효** enum(생존 서열 티어용)
    규칙(§1.4):
      ① 어느 후보든 **원본** BLOCK verdict 보유 = 관문 탈락(그 후보 제거).
         COMPETE 의 탈락은 후보 폐기라 fix 가 무의미한 유일한 경우 — javis_verdict 의
         fix-없는-BLOCK→INVESTIGATE 강등을 탈락 계산에서 우회한다(F3·master 설계 결정).
      ② 생존 후보 0 → 전 후보 탈락(exit 3, 재생성 필요).
      ③ 생존 후보 1 → 승자 확정(exit 0).
      ④ 복수 생존 → 반박 생존 서열(각 후보의 최악 유효 verdict 티어) 최상위가 유일하면 승자.
      ⑤ 최상위 티어 동급(복수) → exit 4(독립 재유도) · reelicited면 exit 5(master 결정).
    """
    eliminated = {lab: any(orig == "BLOCK" for orig, _eff in vs)
                  for lab, vs in cand_verdicts.items()}
    survivors = [lab for lab in cand_verdicts if not eliminated[lab]]
    detail = {"eliminated": sorted(l for l, e in eliminated.items() if e),
              "survivors": sorted(survivors)}
    if not survivors:
        return "ALL_ELIMINATED", None, 3, detail
    if len(survivors) == 1:
        return "WINNER", survivors[0], 0, detail
    # 복수 생존 — 후보별 최악 티어(질적 서열·유효 verdict 기준)
    worst = {lab: max(_SURVIVOR_TIER.get(eff, 2) for _orig, eff in cand_verdicts[lab])
             for lab in survivors}
    best_tier = min(worst.values())
    best = sorted(lab for lab in survivors if worst[lab] == best_tier)
    detail["tiers"] = worst
    detail["best_tier"] = best_tier
    if len(best) == 1:
        return "WINNER", best[0], 0, detail
    if reelicited:
        return "TIE_STRATEGIC", best, 5, detail
    return "TIE_REELICIT", best, 4, detail


# ── init ───────────────────────────────────────────────────────────────────
def _run(cmd, cwd=None):
    """(rc, tail). 예외는 rc=-1 로 정규화."""
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=600)
        tail = (r.stdout or "") + (r.stderr or "")
        return r.returncode, tail.strip()
    except (OSError, subprocess.SubprocessError) as e:
        return -1, str(e)


def cmd_init(args):
    task, n, track = args.task, args.n, args.track
    if n not in (2, 3):
        print("[compete-init] 거부: --n 은 2 또는 3 (설계 pass@N)", file=sys.stderr)
        return 2
    # 관문 게이트: criteria 실존
    if not args.criteria or not os.path.isfile(args.criteria):
        print("[compete-init] 거부: CRITERIA 파일 부재(%r) — 최소 관문 선작성·해시 잠금이 "
              "발동 전제(§1.2)" % args.criteria, file=sys.stderr)
        return 2
    # 다양성 게이트
    ok, reason = check_approaches(args.approach, n)
    if not ok:
        print("[compete-init] 거부: %s" % reason, file=sys.stderr)
        return 2
    if track == "code" and not args.repo:
        print("[compete-init] 거부: CODE 트랙은 --repo 필수(worktree 격리)", file=sys.stderr)
        return 2
    # 심판 익명성 — producer 는 MAPPING 전용, MANIFEST/RESULT/stdout 출력 금지 (v3.1 Δ1).
    # 미제공 시 기존 동작 완전 동일(하위 호환 — 기록용 선택 인자, 게이트 아님).
    producers = list(getattr(args, "producer", []) or [])
    if producers and len(producers) != n:
        print("[compete-init] 거부: --producer 개수(%d)가 --n(%d)과 불일치"
              % (len(producers), n), file=sys.stderr)
        return 2
    if producers and any(not p.strip() for p in producers):
        print("[compete-init] 거부: 빈 producer 가 있다 — 각 후보의 생산자를 서술하라",
              file=sys.stderr)
        return 2

    # F5② 클로버 차단: 같은 디렉토리에 다른 task 의 MANIFEST 가 이미 있으면 거부
    mpath = manifest_path(task)
    if os.path.isfile(mpath):
        try:
            prior = json.load(open(mpath, encoding="utf-8"))
        except (OSError, ValueError):
            prior = {}
        if prior.get("task") not in (None, task):
            print("[compete-init] 거부: 기존 MANIFEST 의 task(%r)가 현재 task(%r)와 다르다 "
                  "— 격리 공간 클로버 위험(F5)" % (prior.get("task"), task), file=sys.stderr)
            return 2

    cdir = compete_dir(task)
    os.makedirs(verdicts_dir(task), exist_ok=True)
    criteria_abs = os.path.abspath(args.criteria)
    criteria_sha = _sha256(criteria_abs)
    # MAPPING(master 전용): 라벨↔실경로↔접근축·repo·criteria 실경로 — 심판 편향원.
    mapping = {
        "task": task,
        "track": track,
        "n": n,
        "criteria_file": criteria_abs,
        "created_at": time.time(),
        "repo": os.path.abspath(args.repo) if args.repo else None,
        "producer_note": getattr(args, "producer_note", None),  # 이질화 예외 사유 — MAPPING 전용
        "candidates": [],
    }
    # MANIFEST(심판 무해): 라벨·트랙·criteria 해시만 — 실경로·접근축 미노출.
    manifest = {
        "task": task,
        "track": track,
        "n": n,
        "criteria_sha256": criteria_sha,
        "created_at": mapping["created_at"],
        "candidates": [],
    }
    approaches = list(args.approach)[:n]
    created = []  # 롤백용
    try:
        for i in range(n):
            label = "cand_%s" % LABELS[i]
            wn = "w%d" % (i + 1)
            entry = {"label": label, "approach": approaches[i], "slot": wn}
            if producers:
                entry["producer"] = producers[i]  # MAPPING 전용 — MANIFEST/RESULT/stdout 출력 금지
            if track == "code":
                repo = os.path.abspath(args.repo)
                wt = os.path.abspath(os.path.join(repo, "..", "_worktrees",
                                                  "%s-%s" % (_safe(task), wn)))
                branch = "compete/%s/%s" % (_safe(task), wn)
                rc, tail = _run(["git", "-C", repo, "worktree", "add", "-b", branch, wt, "HEAD"])
                if rc != 0:
                    print("[compete-init] worktree 생성 실패(%s): %s" % (label, tail),
                          file=sys.stderr)
                    raise RuntimeError("worktree add 실패")
                created.append(("worktree", repo, wt, branch))
                entry["path"] = wt
                entry["branch"] = branch
                if args.setup:
                    src, stail = _run(args.setup if isinstance(args.setup, list)
                                      else ["/bin/sh", "-c", args.setup], cwd=wt)
                    entry["setup_rc"] = src
                    if src != 0:
                        print("[compete-init] 경고: --setup 훅 non-zero(%s rc=%s): %s"
                              % (label, src, stail[:200]), file=sys.stderr)
            else:  # text
                cand = os.path.join(cdir, "candidates", wn)
                os.makedirs(cand, exist_ok=True)
                created.append(("dir", cand))
                entry["path"] = cand
            mapping["candidates"].append(entry)
            manifest["candidates"].append({"label": label})  # 심판 무해 — 라벨만
    except Exception as e:
        # 부분 생성 롤백(고아 격리 공간 0 — §1.5 성공 기준)
        _rollback(created)
        print("[compete-init] 롤백 완료(부분 생성 제거): %s" % e, file=sys.stderr)
        return 2

    with open(manifest_path(task), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
    with open(mapping_path(task), "w", encoding="utf-8") as f:
        json.dump(mapping, f, ensure_ascii=False, indent=2)

    # 후보별 위임 브리프 스텁 N장(stdout — 차등 접근 포함, 실경로는 master만 봄)
    print("=" * 64)
    print("COMPETE R0 격리 공간 %d개 생성 · 트랙=%s · task=%s" % (n, track, task))
    print("MANIFEST(심판 무해): %s" % manifest_path(task))
    print("MAPPING(master 전용): %s" % mapping_path(task))
    print("관문(CRITERIA): %s  (sha256 %s)" % (criteria_abs, criteria_sha[:16]))
    print("심판 익명화: verdict 는 라벨명으로만 → %s/<라벨>__<평가자>.json" % verdicts_dir(task))
    for e in mapping["candidates"]:
        print("-" * 64)
        print("[위임 브리프 스텁] %s (%s)" % (e["label"], e["slot"]))
        print("  접근축(차등 의무): %s" % e["approach"])
        print("  작업 공간: %s" % e["path"])
        if "branch" in e:
            print("  브랜치: %s" % e["branch"])
        print("  격리 규칙: 다른 후보 참조 금지(오염 방지). 같은 목표·같은 관문, 다른 공략축.")
        print("  심판 제출: verdict 를 %s/%s__<평가자>.json 로(익명 라벨 유지)."
              % (verdicts_dir(task), e["label"]))
    print("=" * 64)
    return 0


def _destroy_worktree(repo, wt, branch):
    """worktree·브랜치 제거 — rc 확인 후 실패 항목 목록 반환(F4·빈 목록=전부 성공)."""
    failed = []
    if wt:
        rc, tail = _run(["git", "-C", repo, "worktree", "remove", "--force", wt])
        if rc != 0:
            failed.append("worktree remove %s (rc=%s: %s)" % (wt, rc, tail[:120]))
    if branch:
        rc, tail = _run(["git", "-C", repo, "branch", "-D", branch])
        if rc != 0:
            failed.append("branch -D %s (rc=%s: %s)" % (branch, rc, tail[:120]))
    return failed


def _destroy_dir(p):
    """디렉토리 제거 — 실패·잔존 확인 후 실패 항목 목록 반환(F4·빈 목록=성공)."""
    import shutil
    try:
        shutil.rmtree(p)
    except OSError as e:
        return ["rmtree %s (%s)" % (p, e)]
    if os.path.exists(p):
        return ["rmtree %s (경로 잔존)" % p]
    return []


def _rollback(created):
    """부분 생성 롤백 — 제거 rc 확인. 실패 항목은 stderr 로 정직 보고(F4·고아 경고)."""
    failed = []
    for item in reversed(created):
        if item[0] == "worktree":
            _, repo, wt, branch = item
            failed += _destroy_worktree(repo, wt, branch)
        elif item[0] == "dir":
            failed += _destroy_dir(item[1])
    if failed:
        print("[compete-init] 경고: 롤백 중 %d건 제거 실패(고아 잔존 가능):" % len(failed),
              file=sys.stderr)
        for f in failed:
            print("  - %s" % f, file=sys.stderr)


# ── judge ──────────────────────────────────────────────────────────────────
def _parse_verdict_filename(fn):
    """<label>__<evaluator>.json → (label, evaluator). evaluator 없으면 None.
    label 은 반드시 cand_<라벨> 형태여야 인정(익명 규약 강제)."""
    base = fn[:-5] if fn.endswith(".json") else fn
    parts = base.split("__", 1)
    label = parts[0]
    evaluator = parts[1] if len(parts) == 2 and parts[1] else None
    if not re.fullmatch(r"cand_[0-9A-Za-z]+", label):
        return None, None
    return label, evaluator


def cmd_judge(args):
    task = args.task
    mpath = manifest_path(task)
    if not os.path.isfile(mpath):
        print("[compete-judge] 거부: MANIFEST 부재(%s) — init 선행 필요" % mpath, file=sys.stderr)
        return 2
    try:
        manifest = json.load(open(mpath, encoding="utf-8"))
    except (OSError, ValueError) as e:
        print("[compete-judge] 거부: MANIFEST 로드 실패: %s" % e, file=sys.stderr)
        return 2

    vdir = verdicts_dir(task)
    files = sorted(f for f in os.listdir(vdir)) if os.path.isdir(vdir) else []
    files = [f for f in files if f.endswith(".json")]
    if not files:
        print("[compete-judge] 거부: verdict 없음(%s) — 심판 산출물을 라벨명으로 넣어라"
              % vdir, file=sys.stderr)
        return 2

    # javis_verdict 모듈 일괄 검증(형제 모듈)
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    try:
        import javis_verdict
    except Exception as e:
        print("[compete-judge] 거부: javis_verdict 모듈 로드 실패: %s" % e, file=sys.stderr)
        return 2

    cand_verdicts = {}          # label -> [(orig, eff), ...]  (원본·유효 쌍)
    winner_files = {}           # label -> [(evaluator, filepath), ...]
    contract_errors = []
    for fn in files:
        label, evaluator = _parse_verdict_filename(fn)
        if label is None:
            contract_errors.append("%s: 파일명이 익명 라벨(cand_X[__평가자]) 규약 위반" % fn)
            continue
        fp = os.path.join(vdir, fn)
        try:
            obj = json.load(open(fp, encoding="utf-8"))
        except (OSError, ValueError) as e:
            contract_errors.append("%s: JSON 로드 실패(%s)" % (fn, e))
            continue
        schema_errors, _lint, verdict_out = javis_verdict.validate_verdict(obj)
        if schema_errors:
            contract_errors.append("%s: verdict 계약 위반 — %s" % (fn, "; ".join(schema_errors)))
            continue
        # 원본 verdict(강등 전)은 관문 탈락 판정용, verdict_out(유효)은 생존 서열용(F3)
        cand_verdicts.setdefault(label, []).append((obj.get("verdict"), verdict_out))
        winner_files.setdefault(label, []).append((evaluator, fp))

    if contract_errors:
        print("[compete-judge] 거부: verdict 계약 위반 %d건(fail-closed):" % len(contract_errors),
              file=sys.stderr)
        for e in contract_errors:
            print("  - %s" % e, file=sys.stderr)
        return 2

    # F1 팬텀 라벨 차단: 수집 라벨이 MANIFEST 라벨의 부분집합이 아니면 거부(위조 승자 주입 차단)
    manifest_labels = {c["label"] for c in manifest.get("candidates", [])}
    phantom = sorted(set(cand_verdicts) - manifest_labels)
    if phantom:
        print("[compete-judge] 거부: MANIFEST 외 미지 라벨 %s — 팬텀 후보 주입(위조 승자) "
              "차단(fail-closed)" % phantom, file=sys.stderr)
        return 2
    # 모든 후보(MANIFEST 라벨)가 verdict 를 가졌는지 — 미심판 후보는 판정 불가
    unjudged = sorted(manifest_labels - set(cand_verdicts))
    if unjudged:
        print("[compete-judge] 거부: 미심판 후보 %s — 전 후보 verdict 필요" % unjudged,
              file=sys.stderr)
        return 2

    outcome, payload, code, detail = judge_candidates(cand_verdicts, args.reelicited)
    # 접근축은 MAPPING(master 전용)에서만 — RESULT 는 master-facing 이라 병기 허용(best-effort)
    approaches = _load_approaches(task)
    _write_result(task, outcome, payload, detail, cand_verdicts, manifest, approaches)

    if outcome == "WINNER":
        winner = payload
        print("[compete-judge] 승자 확정: %s (생존 %s · 탈락 %s)"
              % (winner, detail["survivors"], detail["eliminated"]))
        print("[compete-judge] ★승자는 종결 아님 — 정련 라운드(R8) 진입 필수·라운드 생략 금지(§1.0)")
        # round-log 외부 호출 — orchestra 무접촉(subprocess). 실패는 경고 후 진행(장부는 best-effort).
        _round_log_winner(task, winner, winner_files.get(winner, []))
        return 0

    if outcome == "ALL_ELIMINATED":
        print("[compete-judge] 전 후보 관문 탈락 — R0 재생성 필요(탈락 %s)" % detail["eliminated"],
              file=sys.stderr)
        return 3
    if outcome == "TIE_REELICIT":
        print("[compete-judge] 서열 불능(동급 %s) — ★독립 재유도 필요(신규 심판 1회 후 "
              "--reelicited 로 재판정)" % payload, file=sys.stderr)
        return 4
    if outcome == "TIE_STRATEGIC":
        print("[compete-judge] 재유도 후에도 동급(%s) — master 전략 적합성(로드맵·일관성) "
              "결정 요청(품질 채점 아님·§1.3-A5)" % payload, file=sys.stderr)
        return 5
    print("[compete-judge] 내부 오류: 미지 outcome %s" % outcome, file=sys.stderr)
    return 2


def _round_log_winner(task, winner, eval_files):
    """승자 verdict 를 라운드 장부(R0)에도 기록 — orchestra round-log CLI 외부 호출.
    실패해도 판정은 이미 확정이므로 경고만 남기고 진행(장부 기록=best-effort·§1.4)."""
    if not os.path.isfile(ORCHESTRA):
        print("[compete-judge] 경고: orchestra 부재 — round-log 생략(판정은 확정)",
              file=sys.stderr)
        return
    evaluator, fp = None, None
    for ev, path in eval_files:
        if ev:  # 평가자명이 파일명에 있는 것을 우선(리뷰어 행 검증 경로)
            evaluator, fp = ev, path
            break
    if fp is None and eval_files:
        evaluator, fp = (eval_files[0][0] or "compete"), eval_files[0][1]
    if fp is None:
        print("[compete-judge] 경고: 승자 verdict 파일 없음 — round-log 생략", file=sys.stderr)
        return
    cmd = [sys.executable, ORCHESTRA, "round-log", "--task", task, "--round", "0",
           "--evaluator", evaluator or "compete", "--verdict-json", fp]
    rc, tail = _run(cmd)
    if rc == 0:
        print("[compete-judge] round-log 기록(R0): 평가자 %s" % evaluator)
    else:
        print("[compete-judge] 경고: round-log 실패(rc=%s) — 판정은 확정, 장부만 미기록: %s"
              % (rc, tail[:200]), file=sys.stderr)


def _load_approaches(task):
    """MAPPING.json(master 전용)에서 라벨→접근축 — 부재/손상 시 빈 dict(best-effort)."""
    mp = mapping_path(task)
    if not os.path.isfile(mp):
        return {}
    try:
        m = json.load(open(mp, encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return {c["label"]: c.get("approach", "") for c in m.get("candidates", []) if "label" in c}


def _fmt_verdicts(vs):
    """[(orig, eff), ...] → 'ACCEPT, BLOCK→INVESTIGATE(강등)' 병기 문자열(F3 원본·강등값)."""
    out = []
    for orig, eff in vs:
        out.append(orig if orig == eff else "%s→%s(강등)" % (orig, eff))
    return ", ".join(out) or "-"


def _write_result(task, outcome, payload, detail, cand_verdicts, manifest, approaches=None):
    approaches = approaches or {}
    lines = ["# COMPETE R0 판정 결과 — %s" % task, "",
             "- 생성: javis_compete.py judge · %s" % time.strftime("%Y-%m-%d %H:%M:%S"),
             "- 트랙: %s · 후보 %d" % (manifest.get("track"), manifest.get("n")),
             "- 관문 sha256: %s" % manifest.get("criteria_sha256", "")[:16],
             "- 판정: **%s**" % outcome, ""]
    if outcome == "WINNER":
        lines.append("## 승자: %s" % payload)
        lines.append("")
        lines.append("> ★승자는 종결 아님 — 정련 라운드(R8) 진입 필수·라운드 생략 금지(§1.0).")
    elif outcome == "ALL_ELIMINATED":
        lines.append("## 전 후보 관문 탈락 — 재생성 필요")
    elif outcome == "TIE_REELICIT":
        lines.append("## 서열 불능(동급): %s — 독립 재유도 필요" % ", ".join(payload))
    elif outcome == "TIE_STRATEGIC":
        lines.append("## 재유도 후 동급: %s — master 전략 결정 요청" % ", ".join(payload))
    lines += ["", "## 후보 verdict(익명 라벨 · 원본→강등값 병기)"]
    for c in manifest.get("candidates", []):
        lab = c["label"]
        vs = cand_verdicts.get(lab, [])
        mark = "탈락" if lab in detail["eliminated"] else ("생존" if lab in detail["survivors"] else "?")
        lines.append("- %s [%s]: %s  (접근축: %s)"
                     % (lab, mark, _fmt_verdicts(vs), approaches.get(lab, "")))
    lines += ["", "> 점수 없음 — 관문 탈락·반박 생존 서열만(§1.4 · javis_verdict score 금지 정합).",
              "> 탈락 판정은 강등 전 원본 verdict 로 수행(F3) — fix 무의미한 후보 폐기라 우회.", ""]
    os.makedirs(os.path.dirname(result_path(task)), exist_ok=True)
    with open(result_path(task), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


# ── clean ──────────────────────────────────────────────────────────────────
def _parse_outcome_text(text):
    """RESULT.md 본문 → '- 판정: **<OUTCOME>**' 판정값(순수 함수 — self-test 박제)."""
    m = re.search(r"^- 판정:\s*\*\*([A-Z_]+)\*\*", text or "", re.MULTILINE)
    return m.group(1) if m else None


def _result_outcome(task):
    """RESULT.md 의 판정값 추출 — 부재/불명 시 None."""
    rp = result_path(task)
    if not os.path.isfile(rp):
        return None
    try:
        return _parse_outcome_text(open(rp, encoding="utf-8").read())
    except OSError:
        return None


def cmd_clean(args):
    task = args.task
    mpath = manifest_path(task)
    if not os.path.isfile(mpath):
        print("[compete-clean] MANIFEST 부재(%s) — 청소할 격리 공간 없음" % mpath, file=sys.stderr)
        return 2
    # F2 가드: RESULT 존재만으로 부족(TIE/전탈락도 RESULT 기록됨). 판정==WINNER 일 때만
    # --force 없이 청소 허용. WINNER 아니면 후보 보존이 목적이라 --force 강제.
    outcome = _result_outcome(task)
    if outcome != "WINNER" and not args.force:
        why = "미판정(RESULT.md 없음)" if outcome is None else "판정=%s(승자 아님)" % outcome
        print("[compete-clean] 경고: %s — 청소는 후보/산출물 소실 위험(TIE_REELICIT 은 재유도 "
              "위해 후보 보존이 목적). 강행하려면 --force." % why, file=sys.stderr)
        return 2
    # 경로·브랜치·트랙은 MAPPING(master 전용)에서 — 부재 시 MANIFEST fallback(구 포맷 호환)
    src = mapping_path(task) if os.path.isfile(mapping_path(task)) else mpath
    try:
        meta = json.load(open(src, encoding="utf-8"))
    except (OSError, ValueError) as e:
        print("[compete-clean] 매핑 로드 실패(%s): %s" % (src, e), file=sys.stderr)
        return 2
    track = meta.get("track")
    repo = meta.get("repo")
    removed, failed = [], []
    for c in meta.get("candidates", []):
        if track == "code" and repo:
            wt, branch = c.get("path"), c.get("branch")
            fs = _destroy_worktree(repo, wt, branch)
            if fs:
                failed += fs
            elif wt:
                removed.append(wt)
        elif track == "text":
            p = c.get("path")
            if p and os.path.isdir(p):
                fs = _destroy_dir(p)
                if fs:
                    failed += fs
                else:
                    removed.append(p)
    # candidates/ 상위 껍데기도 정리(text) — RESULT.md·MANIFEST.json·MAPPING.json 은 보존
    cand_root = os.path.join(compete_dir(task), "candidates")
    if track == "text" and os.path.isdir(cand_root) and not os.listdir(cand_root):
        os.rmdir(cand_root)
    print("[compete-clean] 격리 공간 %d개 폐기(기록 보존: MANIFEST.json·MAPPING.json·RESULT.md)"
          % len(removed))
    for r in removed:
        print("  - %s" % r)
    if failed:
        print("[compete-clean] ★제거 실패 %d건(고아 격리 공간 잔존 — '고아 0' 검증 거짓 방지):"
              % len(failed), file=sys.stderr)
        for f in failed:
            print("  - %s" % f, file=sys.stderr)
        return 2
    return 0


# ── self-test(밀폐 — 실제 git·전역 상태 무접촉) ─────────────────────────────
def self_test():
    fails = []

    def want(name, cond):
        if not cond:
            fails.append(name)

    # ── 다양성 게이트 ──
    ok, _ = check_approaches(["a", "b"], 2)
    want("다양성:정상 2종 통과", ok)
    ok, _ = check_approaches(["same", "same"], 2)
    want("다양성:동일 approach 거부", not ok)
    ok, _ = check_approaches(["  Same-Approach ", "same approach"], 2)
    want("다양성:정규화 후 동일 거부(표면 위장 차단)", not ok)
    ok, _ = check_approaches(["a"], 2)
    want("다양성:개수 부족 거부", not ok)
    ok, _ = check_approaches(["a", "", "c"], 3)
    want("다양성:빈 축 거부", not ok)
    ok, _ = check_approaches(["a", "b", "c"], 3)
    want("다양성:N=3 정상 통과", ok)

    # ── 판정 규칙(순수 함수 · verdict 는 (원본, 유효) 쌍) ──
    def V(*vs):  # 원본==유효인 정상 verdict 목록(강등 없음)
        return [(v, v) for v in vs]

    # ① BLOCK = 관문 탈락 → 생존 1 → 승자
    o, w, c, d = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": V("BLOCK")})
    want("판정:BLOCK 탈락→단독 생존 승자", o == "WINNER" and w == "cand_A" and c == 0)
    want("판정:탈락 목록 정확", d["eliminated"] == ["cand_B"])
    # ② 생존 1개(명시) → 승자
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT")})
    want("판정:단일 후보 승자", o == "WINNER" and w == "cand_A" and c == 0)
    # ③ 전 후보 탈락 → exit 3
    o, w, c, _ = judge_candidates({"cand_A": V("BLOCK"), "cand_B": V("BLOCK")})
    want("판정:전 후보 탈락 exit 3", o == "ALL_ELIMINATED" and c == 3)
    # ④ 복수 생존·티어 차이 → 우수 티어 유일 승자(ACCEPT > REVISE)
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": V("REVISE")})
    want("판정:티어 우열 승자(ACCEPT>REVISE)", o == "WINNER" and w == "cand_A" and c == 0)
    # ④b ESCALATE 는 REVISE 보다 하위 — REVISE 후보가 승자
    o, w, c, _ = judge_candidates({"cand_A": V("ESCALATE"), "cand_B": V("REVISE")})
    want("판정:REVISE>ESCALATE 승자", o == "WINNER" and w == "cand_B" and c == 0)
    # ④c 최악 verdict 로 티어 — A(ACCEPT+REVISE=최악 REVISE) vs B(ACCEPT) → B 승
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT", "REVISE"), "cand_B": V("ACCEPT")})
    want("판정:최악 verdict 로 티어(보수)", o == "WINNER" and w == "cand_B" and c == 0)
    # ⑤ 동급(둘 다 ACCEPT) → exit 4 재유도
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": V("ACCEPT")})
    want("판정:동급 exit 4(재유도)", o == "TIE_REELICIT" and c == 4 and set(w) == {"cand_A", "cand_B"})
    # ⑤b 재유도 후에도 동급 → exit 5 전략 결정
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": V("ACCEPT")}, reelicited=True)
    want("판정:재유도 후 동급 exit 5", o == "TIE_STRATEGIC" and c == 5)

    # ── F3: 원본 BLOCK 탈락(강등값 INVESTIGATE 로 우회 못함) ──
    # cand_B 원본 BLOCK 이 fix 없어 INVESTIGATE 로 강등돼도 탈락은 원본 기준 — cand_A 승자
    o, w, c, d = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": [("BLOCK", "INVESTIGATE")]})
    want("F3:원본 BLOCK 탈락(강등 우회 차단)",
         o == "WINNER" and w == "cand_A" and c == 0 and d["eliminated"] == ["cand_B"])
    # 유효 verdict(강등값)은 생존 서열 티어에만 반영 — 원본 REVISE→INVESTIGATE 동일 티어
    o, w, c, _ = judge_candidates({"cand_A": V("ACCEPT"), "cand_B": [("REVISE", "INVESTIGATE")]})
    want("F3:강등값은 서열 티어에만(REVISE~INVESTIGATE 동급 티어)",
         o == "WINNER" and w == "cand_A" and c == 0)

    # ── F5: _safe 충돌 방지(치환·절단 시 원본 해시 접미) ──
    want("F5:무손실 task 접미 없음", _safe("simple-task_1") == "simple-task_1")
    want("F5:치환 발생 시 원본 구분", _safe("a/b") != _safe("a_b"))
    want("F5:치환 발생 시 접미 부착", _safe("a/b") != "a_b" and _safe("a/b").startswith("a_b_"))
    long_a, long_b = "x" * 85 + "A", "x" * 85 + "B"  # 앞 80자 동일 → 절단 시 같은 토큰(충돌)
    want("F5:절단 충돌 원본 해시로 구분", _safe(long_a) != _safe(long_b))
    want("F5:토큰 길이 80 이하 유지", len(_safe(long_a)) <= 80)

    # ── F1: 팬텀 라벨 집합차 규칙(수집 라벨 ⊄ MANIFEST 라벨 → 거부) ──
    collected, manifest_labels = {"cand_A", "cand_Z"}, {"cand_A", "cand_B"}
    want("F1:팬텀 라벨 집합차 감지", sorted(collected - manifest_labels) == ["cand_Z"])
    want("F1:정상 라벨은 부분집합", not (({"cand_A"} - {"cand_A", "cand_B"})))

    # ── F2: clean 가드 판정값 파싱(WINNER 만 --force 없이 청소 허용) ──
    want("F2:WINNER 판정 파싱", _parse_outcome_text("- 판정: **WINNER**\n") == "WINNER")
    want("F2:TIE 판정 파싱", _parse_outcome_text("- 판정: **TIE_REELICIT**\n") == "TIE_REELICIT")
    want("F2:판정 줄 없으면 None", _parse_outcome_text("# 제목\n본문") is None)

    # ── 파일명 익명 규약 파서 ──
    want("파일명:라벨+평가자", _parse_verdict_filename("cand_A__codex.json") == ("cand_A", "codex"))
    want("파일명:라벨만", _parse_verdict_filename("cand_B.json") == ("cand_B", None))
    want("파일명:비규약 거부", _parse_verdict_filename("winner__codex.json") == (None, None))

    if fails:
        print(json.dumps({"self_test": "fail", "failures": fails}, ensure_ascii=False, indent=2))
        return 1
    print(json.dumps({"self_test": "ok",
                      "covers": "다양성게이트(정상·동일·정규화위장·개수부족·빈축·N3)·"
                                "판정(BLOCK탈락·단일·전탈락·티어우열·ESCALATE하위·"
                                "최악티어·동급재유도·재유도후전략)·"
                                "F3(원본BLOCK탈락·강등값서열한정)·"
                                "F5(무손실·치환·절단충돌·길이)·F1(팬텀집합차)·"
                                "F2(WINNER파싱·TIE·None)·파일명익명규약"},
                     ensure_ascii=False, indent=2))
    return 0


def main():
    ap = argparse.ArgumentParser(description="레버① COMPETE 결정론 지점(init/judge/clean)")
    ap.add_argument("--self-test", action="store_true", help="밀폐 자기검증(git·전역 무접촉)")
    sub = ap.add_subparsers(dest="cmd")

    pi = sub.add_parser("init", help="관문·다양성 게이트 + 격리 공간 N개 생성 + MANIFEST")
    pi.add_argument("--task", required=True)
    pi.add_argument("--n", type=int, required=True, help="후보 수(2 또는 3)")
    pi.add_argument("--criteria", required=True, help="CRITERIA 관문 파일(실존·해시 잠금)")
    pi.add_argument("--track", required=True, choices=["code", "text"])
    pi.add_argument("--approach", action="append", default=[],
                    help="후보별 차등 접근축 — N회 반복(서로 달라야 함·다양성 강제)")
    pi.add_argument("--repo", default=None, help="CODE 트랙: worktree 격리 대상 git repo")
    pi.add_argument("--setup", default=None, help="CODE 트랙: worktree 생성 후 실행할 준비 훅")
    # 심판 익명성 — producer 는 MAPPING 전용, MANIFEST/RESULT/stdout 출력 금지 (v3.1 Δ1)
    pi.add_argument("--producer", action="append", default=[],
                    help="후보별 생산자 기록(선택·반복 — 예: worker:claude) — 기록용, 게이트 아님")
    pi.add_argument("--producer-note", default=None,
                    help="이질화 예외 사유(선택) — MAPPING 전용 기록")

    pj = sub.add_parser("judge", help="익명 verdict 일괄 검증 + 판정(점수 없음)")
    pj.add_argument("--task", required=True)
    pj.add_argument("--reelicited", action="store_true",
                    help="독립 재유도 후 재판정 — 동급이면 exit 5(전략 결정)")

    pc = sub.add_parser("clean", help="격리 공간·브랜치 폐기(MANIFEST·RESULT 보존)")
    pc.add_argument("--task", required=True)
    pc.add_argument("--force", action="store_true", help="비승자(미판정·TIE·전탈락) 태스크도 강제 청소")

    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.cmd == "init":
        return cmd_init(args)
    if args.cmd == "judge":
        return cmd_judge(args)
    if args.cmd == "clean":
        return cmd_clean(args)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
