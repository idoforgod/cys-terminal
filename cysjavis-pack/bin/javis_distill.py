#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_distill — 증류 수명주기(Distillation Lifecycle) 결정론 도구 (설계 v3.2 §C8).

증류의 수명주기 계약을 LLM 자연어 추론에서 분리해 결정론으로 집행한다.
확인된 근본원인 → regression test → candidate(rule_id 발급) → holdout 재발 검증 →
active 승격 / supersede / retire. 삭제는 없다 — status 전이만(회계 감사 가능).

3저장소의 지위(§C8.2 · SOT 분기 차단):
  · canonical 규칙 md(canonical_locator가 지정하는 단일 경로) = canonical SOT — rule_id·본문·status의 유일 정본.
  · 자비스 memory(feedback) = derivative index — 교차프로젝트 회상용. canonical의 파생물(독자 개정 금지).
  · 커밋 trailer(.vibecoding/receipts.jsonl 미러) = append-only receipt — 증류 이벤트 영수증(개정 불가).

C8.2-A immutable canonical_locator: rule 생성 시 프로젝트 설정의 기본 locator를 상속해
immutable 필드로 단일 기록한다(생성 후 변경 불가). canonical_locator가 아닌 경로에서 동일
rule_id가 active로 발견되면 scan-dual-active가 fail-closed(exit 2)로 거부한다.

C8.4 동기화 책임자: master가 단일 sync owner다 — canonical 변경 시 memory 미러·receipt 발행을
같은 작업 단위에서 집행한다. 워커는 candidate 제안(propose)까지만. promote/supersede/retire는
--master 플래그 필수.

명령:
    propose        candidate 생성(rule_id 자동발급) — 워커 권한
    promote        active 승격 — holdout 검증 ref 필수 + --master (master 전용)
    supersede      구 rule을 신 rule로 대체(superseded_by 링크·양방향 추적) — --master
    retire         rule 폐기(status=retired) — --master
    sync-check     3저장소 대조(canonical vs memory 색인 vs receipt) — FX-1·2·3 검출·복구(--fix)
    scan-dual-active  C8.2-A: 비-locator 경로의 active rule_id 발견 시 exit 2 fail-closed + 충돌 로그

종료 코드: 0 성공/정합 · 1 드리프트 검출(sync-check) 또는 일반 실패 · 2 fail-closed 충돌/인자·권한 거부
의존성: 파이썬 표준 라이브러리만 (네트워크·LLM 호출 없음).
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time

CONFIG_REL = os.path.join(".vibecoding", "distill.json")
RECEIPTS_REL = os.path.join(".vibecoding", "receipts.jsonl")
SYNC_LOG_REL = os.path.join(".vibecoding", "sync.log")

VALID_STATUS = ("candidate", "active", "superseded", "retired")
RULE_ID_RE = re.compile(r"^VR-(\d{3,})$")
# canonical md 안의 rule 블록: 기계 헤더(JSON, 사람에게 숨김) + 사람 가독 본문 + 종료 마커.
BLOCK_RE = re.compile(
    r"<!--\s*vibe-rule\s+(?P<meta>\{.*?\})\s*-->\n(?P<body>.*?)\n<!--\s*/vibe-rule\s*-->",
    re.S)
CANONICAL_HEADER = "# Vibe Rules — canonical SOT (javis_distill 관리 · 손편집 주의)\n\n"
# rule record의 메타 필드(본문 body 제외) — canonical_locator는 생성 후 immutable.
META_FIELDS = ("rule_id", "status", "canonical_locator",
               "regression_test_ref", "root_cause", "superseded_by")


def fail(code, msg):
    print(json.dumps({"error": msg}, ensure_ascii=False), file=sys.stderr)
    return code


# ── 프로젝트 설정 ──────────────────────────────────────────────

def load_config(project):
    """.vibecoding/distill.json 로드 — 기본 canonical_locator 1개 선언(C8.2-A)."""
    path = os.path.join(project, CONFIG_REL)
    if not os.path.isfile(path):
        return None, "프로젝트 설정 없음: %s (canonical_locator 선언 필요)" % path
    try:
        cfg = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError) as e:
        return None, "설정 파싱 실패: %s" % e
    loc = (cfg.get("canonical_locator") or "").strip()
    if not loc:
        return None, "설정에 canonical_locator가 비어있다: %s" % path
    cfg["canonical_locator"] = os.path.normpath(loc)
    return cfg, None


# ── canonical md 직렬화/파싱 ───────────────────────────────────

def serialize_rule(rule):
    meta = {k: rule.get(k) for k in META_FIELDS}
    return "<!-- vibe-rule %s -->\n%s\n<!-- /vibe-rule -->\n" % (
        json.dumps(meta, ensure_ascii=False, sort_keys=True), (rule.get("body") or "").strip())


def parse_rules(text):
    """md 텍스트에서 vibe-rule 블록을 순서 보존해 파싱한다."""
    rules = []
    for m in BLOCK_RE.finditer(text):
        try:
            meta = json.loads(m.group("meta"))
        except ValueError:
            continue
        rec = {k: meta.get(k) for k in META_FIELDS}
        rec["body"] = m.group("body").strip()
        rules.append(rec)
    return rules


def render_canonical(rules):
    return CANONICAL_HEADER + "\n".join(serialize_rule(r) for r in rules) + ("\n" if rules else "")


def write_text_atomic(path, text):
    d = os.path.dirname(path) or "."
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-", suffix=".swap")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def canonical_path(project, cfg):
    return os.path.join(project, cfg["canonical_locator"])


def read_canonical(project, cfg):
    path = canonical_path(project, cfg)
    if not os.path.isfile(path):
        return []
    return parse_rules(open(path, encoding="utf-8", errors="replace").read())


def write_canonical(project, cfg, rules):
    write_text_atomic(canonical_path(project, cfg), render_canonical(rules))


# ── receipt(append-only) ───────────────────────────────────────

def append_receipt(project, record):
    """증류 이벤트 영수증을 append-only jsonl에 기록(커밋 trailer의 기계 미러)."""
    path = os.path.join(project, RECEIPTS_REL)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    record = dict(record)
    record.setdefault("ts", round(time.time(), 3))
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")


def read_receipts(project):
    """receipt 목록(jsonl) + (git repo면) 커밋 trailer의 Vibe-Rule: 영수증 병합."""
    out = []
    path = os.path.join(project, RECEIPTS_REL)
    if os.path.isfile(path):
        for line in open(path, encoding="utf-8", errors="replace"):
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except ValueError:
                continue
    out.extend(_git_trailer_receipts(project))
    return out


def _git_trailer_receipts(project):
    """커밋 trailer 'Vibe-Rule: VR-NNN <event>' 파싱(방어적 — 실패 시 빈 목록)."""
    if not os.path.isdir(os.path.join(project, ".git")):
        return []
    try:
        r = subprocess.run(["git", "-C", project, "log", "--format=%(trailers:key=Vibe-Rule,valueonly)"],
                           capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.SubprocessError):
        return []
    recs = []
    for line in r.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if RULE_ID_RE.match(parts[0]):
            recs.append({"rule_id": parts[0], "event": (parts[1] if len(parts) > 1 else "trailer"),
                         "source": "git-trailer"})
    return recs


def latest_receipt_body(receipts, rule_id):
    """해당 rule_id의 가장 최근 body를 가진 receipt 본문(FX-2 candidate 복원 재료)."""
    body = None
    for rec in receipts:
        if rec.get("rule_id") == rule_id and rec.get("body"):
            body = rec["body"]
    return body


# ── memory 미러(derivative index — 읽기 전용 접근이 기본) ────────

# ★팩 경로 env 키의 우선순위 목록(W14 S19 · P3). Rust 정본 `src/pack.rs::PACK_DIR_ENV_KEYS`와
#   목록·순서가 기계 대조된다(bin/tests/test_todo_shared_constants.py).
PACK_DIR_ENV_KEYS = ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR")


def default_memory_dir():
    """memory 미러 위치 — src/pack.rs pack_dir() 폴백(PACK_DIR_ENV_KEYS) 미러링."""
    for key in PACK_DIR_ENV_KEYS:
        v = os.environ.get(key, "")
        if v:
            return os.path.join(v, "memory")
    return os.path.join(os.path.expanduser("~"), ".cys/pack", "memory")


def memory_filename(rule_id):
    return "feedback_vibe-rule-%s.md" % rule_id.lower()


def parse_memory_mirror(text):
    """memory 미러 파일에서 (vibe_rule_id, body) 추출 — derivative index 읽기."""
    rid, body = None, ""
    if text.startswith("---"):
        end = text.find("\n---", 3)
        if end >= 0:
            head = text[3:end]
            for line in head.splitlines():
                s = line.strip()
                if s.startswith("vibe_rule_id:"):
                    rid = s[len("vibe_rule_id:"):].strip() or None
            body = text[end + 4:].strip()
    return rid, body


def memory_map(mdir):
    """{rule_id: body} — memory 색인을 derivative로만 읽는다."""
    out = {}
    if not os.path.isdir(mdir):
        return out
    for fn in sorted(os.listdir(mdir)):
        if not fn.endswith(".md") or fn == "MEMORY.md" or fn.startswith("."):
            continue
        rid, body = parse_memory_mirror(
            open(os.path.join(mdir, fn), encoding="utf-8", errors="replace").read())
        if rid:
            out[rid] = body
    return out


def write_memory_mirror(mdir, rule):
    """canonical(SOT)에서 memory 미러를 파생 생성/덮어쓰기 — sync owner의 파생 집행."""
    os.makedirs(mdir, exist_ok=True)
    body = (rule.get("body") or "").strip()
    desc = body.splitlines()[0] if body else rule["rule_id"]
    content = ("---\n"
               "name: vibe-rule-%s\n"
               "description: %s\n"
               "metadata:\n"
               "  type: feedback\n"
               "  vibe_rule_id: %s\n"
               "---\n\n%s\n" % (rule["rule_id"].lower(), desc, rule["rule_id"], body))
    write_text_atomic(os.path.join(mdir, memory_filename(rule["rule_id"])), content)


# ── rule_id 발급 ───────────────────────────────────────────────

def next_rule_id(rules, receipts):
    hi = 0
    for src in (rules, receipts):
        for r in src:
            m = RULE_ID_RE.match(r.get("rule_id") or "")
            if m:
                hi = max(hi, int(m.group(1)))
    return "VR-%03d" % (hi + 1)


def find_rule(rules, rule_id):
    for r in rules:
        if r["rule_id"] == rule_id:
            return r
    return None


# ── 명령: propose (워커 권한) ──────────────────────────────────

def cmd_propose(args):
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    if not (args.body or "").strip():
        return fail(2, "--body(규칙 본문)는 비울 수 없다")
    if not (args.root_cause or "").strip():
        return fail(2, "--root-cause(확인된 근본원인)는 필수다 (C8.1 입장 게이트)")
    if not (args.regression_test_ref or "").strip():
        return fail(2, "--regression-test-ref(재발 방지 테스트 참조)는 필수다 (C8.1 candidate 선행)")
    rules = read_canonical(args.project, cfg)
    receipts = read_receipts(args.project)
    rid = args.rule_id or next_rule_id(rules, receipts)
    if not RULE_ID_RE.match(rid):
        return fail(2, "rule_id 형식은 VR-NNN 이어야 한다: %r" % rid)
    if find_rule(rules, rid):
        return fail(2, "rule_id 중복: %s (이미 canonical에 존재)" % rid)
    rec = {"rule_id": rid, "status": "candidate",
           "canonical_locator": cfg["canonical_locator"],  # 상속 → immutable
           "regression_test_ref": args.regression_test_ref.strip(),
           "root_cause": args.root_cause.strip(), "superseded_by": None,
           "body": args.body.strip()}
    rules.append(rec)
    write_canonical(args.project, cfg, rules)
    append_receipt(args.project, {"event": "propose", "rule_id": rid, "status": "candidate",
                                  "body": rec["body"], "root_cause": rec["root_cause"],
                                  "regression_test_ref": rec["regression_test_ref"]})
    print(json.dumps({"proposed": rid, "status": "candidate",
                      "canonical_locator": cfg["canonical_locator"]}, ensure_ascii=False))
    return 0


# ── 명령: promote / supersede / retire (master 전용) ───────────

def _require_master(args):
    if not getattr(args, "master", False):
        return fail(2, "promote/supersede/retire는 master 전용이다 — --master 필수 (C8.4 워커는 propose까지만)")
    return None


def cmd_promote(args):
    guard = _require_master(args)
    if guard is not None:
        return guard
    if not (args.holdout_evidence or "").strip():
        return fail(2, "--holdout-evidence(holdout 재발 검증 ref)는 active 승격 필수다 (C8.1)")
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    rules = read_canonical(args.project, cfg)
    rule = find_rule(rules, args.rule_id)
    if rule is None:
        return fail(2, "rule 없음: %s" % args.rule_id)
    if rule["status"] != "candidate":
        return fail(2, "candidate만 promote 가능 — 현재 status=%s" % rule["status"])
    rule["status"] = "active"
    write_canonical(args.project, cfg, rules)
    mdir = args.memory_dir or default_memory_dir()
    write_memory_mirror(mdir, rule)  # sync owner: canonical→memory 파생을 같은 단위에서
    append_receipt(args.project, {"event": "promote", "rule_id": args.rule_id, "status": "active",
                                  "holdout_evidence": args.holdout_evidence.strip(),
                                  "body": rule["body"]})
    print(json.dumps({"promoted": args.rule_id, "status": "active", "memory_dir": mdir},
                     ensure_ascii=False))
    return 0


def cmd_supersede(args):
    guard = _require_master(args)
    if guard is not None:
        return guard
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    rules = read_canonical(args.project, cfg)
    old = find_rule(rules, args.rule_id)
    new = find_rule(rules, args.by)
    if old is None:
        return fail(2, "구 rule 없음: %s" % args.rule_id)
    if new is None:
        return fail(2, "신 rule 없음: %s (먼저 propose/promote)" % args.by)
    if new["status"] != "active":
        return fail(2, "대체 rule은 active여야 한다 — %s status=%s" % (args.by, new["status"]))
    old["status"] = "superseded"
    old["superseded_by"] = args.by  # 양방향 추적
    write_canonical(args.project, cfg, rules)
    append_receipt(args.project, {"event": "supersede", "rule_id": args.rule_id,
                                  "status": "superseded", "superseded_by": args.by})
    print(json.dumps({"superseded": args.rule_id, "superseded_by": args.by}, ensure_ascii=False))
    return 0


def cmd_retire(args):
    guard = _require_master(args)
    if guard is not None:
        return guard
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    rules = read_canonical(args.project, cfg)
    rule = find_rule(rules, args.rule_id)
    if rule is None:
        return fail(2, "rule 없음: %s" % args.rule_id)
    rule["status"] = "retired"
    write_canonical(args.project, cfg, rules)
    append_receipt(args.project, {"event": "retire", "rule_id": args.rule_id, "status": "retired"})
    print(json.dumps({"retired": args.rule_id}, ensure_ascii=False))
    return 0


# ── 명령: sync-check (3저장소 대조 · FX-1·2·3) ─────────────────

def cmd_sync_check(args):
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    mdir = args.memory_dir or default_memory_dir()
    rules = read_canonical(args.project, cfg)
    by_id = {r["rule_id"]: r for r in rules}
    active = {rid: r for rid, r in by_id.items() if r["status"] == "active"}
    mem = memory_map(mdir)
    receipts = read_receipts(args.project)

    findings = []

    # FX-1: canonical active인데 memory 색인에 없음 → sync 복구 절차 발동.
    for rid, rule in sorted(active.items()):
        if rid not in mem:
            findings.append({"type": "MISSING_IN_MEMORY", "rule_id": rid,
                             "recovery": "canonical(SOT)에서 memory 미러 재생성",
                             "fixed": False})

    # FX-3: memory 본문이 canonical과 다름 → canonical 우선 덮어쓰기 + 불일치 로그.
    for rid, rule in sorted(active.items()):
        if rid in mem and mem[rid].strip() != (rule["body"] or "").strip():
            findings.append({"type": "MEMORY_BODY_DIVERGENT", "rule_id": rid,
                             "recovery": "canonical 우선 — memory 본문 덮어쓰기 + 불일치 로그",
                             "fixed": False})

    # FX-2: receipt(trailer)엔 있으나 canonical엔 없음 → candidate 등재(자동 active 아님).
    receipt_ids = {r.get("rule_id") for r in receipts if r.get("rule_id")}
    for rid in sorted(x for x in receipt_ids if x):
        if rid not in by_id:
            findings.append({"type": "TRAILER_ONLY", "rule_id": rid,
                             "recovery": "candidate로 등재(자동 active 승격 아님)",
                             "fixed": False})

    if args.fix and findings:
        _apply_sync_fix(args.project, cfg, rules, by_id, mdir, mem, receipts, findings)

    ok = not findings
    out = {"ok": ok, "canonical_locator": cfg["canonical_locator"], "memory_dir": mdir,
           "fixed": bool(args.fix), "findings": findings}
    if args.json:
        print(json.dumps(out, ensure_ascii=False, indent=2))
    else:
        print("sync-check: %s — 발견 %d건 (canonical=%s)"
              % ("OK" if ok else "DRIFT", len(findings), cfg["canonical_locator"]))
        for f in findings:
            print("  [%s] %s → %s%s"
                  % (f["type"], f["rule_id"], f["recovery"], " (복구됨)" if f.get("fixed") else ""))
        if findings and not args.fix:
            print("복구는 --fix (master가 단일 sync owner — C8.4). 이 출력 외 추론으로 정합 선언 금지.")
    # 복구 후에는 정합으로 보고(0), 미복구 드리프트는 1.
    return 0 if (ok or (args.fix and all(f.get("fixed") for f in findings))) else 1


def _apply_sync_fix(project, cfg, rules, by_id, mdir, mem, receipts, findings):
    """canonical을 SOT로 삼아 파생(memory)·candidate 등재를 집행하고 findings에 fixed 마킹."""
    logf = os.path.join(project, SYNC_LOG_REL)
    os.makedirs(os.path.dirname(logf), exist_ok=True)
    log_lines = []
    canonical_changed = False
    for f in findings:
        rid = f["rule_id"]
        if f["type"] == "MISSING_IN_MEMORY":
            write_memory_mirror(mdir, by_id[rid])
            f["fixed"] = True
            log_lines.append("MISSING_IN_MEMORY %s → memory 미러 재생성(canonical SOT)" % rid)
        elif f["type"] == "MEMORY_BODY_DIVERGENT":
            log_lines.append("MEMORY_BODY_DIVERGENT %s → canonical 우선 덮어쓰기 (memory 본문 폐기)" % rid)
            write_memory_mirror(mdir, by_id[rid])
            f["fixed"] = True
        elif f["type"] == "TRAILER_ONLY":
            body = latest_receipt_body(receipts, rid) or "(receipt에 본문 없음 — 수동 확인 필요)"
            rec = {"rule_id": rid, "status": "candidate",
                   "canonical_locator": cfg["canonical_locator"],
                   "regression_test_ref": None, "root_cause": None,
                   "superseded_by": None, "body": body}
            rules.append(rec)
            by_id[rid] = rec
            canonical_changed = True
            f["fixed"] = True
            log_lines.append("TRAILER_ONLY %s → candidate 등재(자동 active 아님)" % rid)
    if canonical_changed:
        write_canonical(project, cfg, rules)
    if log_lines:
        with open(logf, "a", encoding="utf-8") as fh:
            stamp = time.strftime("%Y-%m-%dT%H:%M:%S")
            for ln in log_lines:
                fh.write("%s %s\n" % (stamp, ln))


# ── 명령: scan-dual-active (C8.2-A fail-closed · FX-4) ─────────

# M-4: dot-directory도 반드시 탐색한다 — 숨긴 경로(.hidden/ 등)에 비-locator active 사본을
# 두어 이중 canonical 충돌을 은폐하는 우회를 차단한다. 아래 명시 제외 목록(VCS 내부·기계 생성
# 캐시 — 정본 규칙이 살 수 없는 곳)만 건너뛴다. 임의의 dot-dir(.config/.rules 등)은 스캔 대상.
_SKIP_DIRS = {".git", ".hg", ".svn", ".bzr", "node_modules",
              "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache", ".tox"}


def _iter_md_files(project):
    for root, dirs, files in os.walk(project):
        dirs[:] = [d for d in dirs if d not in _SKIP_DIRS]  # dot-dir는 제외목록에 한해서만 skip(M-4)
        for fn in files:
            if fn.endswith(".md"):
                yield os.path.join(root, fn)


def cmd_scan_dual_active(args):
    cfg, err = load_config(args.project)
    if err:
        return fail(2, err)
    locator = cfg["canonical_locator"]
    conflicts = []
    for path in _iter_md_files(args.project):
        rel = os.path.normpath(os.path.relpath(path, args.project))
        if rel == locator:
            continue  # 정본 경로는 허용
        for rule in parse_rules(open(path, encoding="utf-8", errors="replace").read()):
            if rule["status"] == "active":
                conflicts.append({"rule_id": rule["rule_id"], "found_at": rel,
                                  "authoritative_path": locator,
                                  "verdict": "REJECTED_NON_LOCATOR"})
    conflicts.sort(key=lambda c: (c["rule_id"], c["found_at"]))
    out = {"ok": not conflicts, "canonical_locator": locator, "conflicts": conflicts}
    if args.json:
        print(json.dumps(out, ensure_ascii=False, indent=2))
    else:
        if not conflicts:
            print("scan-dual-active: OK — 비-locator 경로에 active rule 없음 (canonical=%s)" % locator)
        else:
            print("scan-dual-active: FAIL-CLOSED — 비-locator 경로 active %d건 (C8.2-A 거부)" % len(conflicts))
            for c in conflicts:
                print("  [DUAL-ACTIVE] %s active @ %s → 거부(정본=%s만 유효)"
                      % (c["rule_id"], c["found_at"], c["authoritative_path"]))
    return 0 if not conflicts else 2


# ── self-test (결정론 스모크) ──────────────────────────────────

def self_test():
    import contextlib
    import io
    failures = []
    with tempfile.TemporaryDirectory(prefix="javis-distill-selftest-") as proj:
        os.makedirs(os.path.join(proj, ".vibecoding"))
        json.dump({"canonical_locator": "docs/rules/vibe-rules.md"},
                  open(os.path.join(proj, CONFIG_REL), "w", encoding="utf-8"))
        mdir = os.path.join(proj, "memory")
        sink = io.StringIO()

        def call(fn, ns):
            with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
                return fn(ns)

        base = dict(project=proj, memory_dir=mdir)
        rc = call(cmd_propose, argparse.Namespace(
            rule_id=None, body="null 방어", root_cause="외부 API null 역참조",
            regression_test_ref="tests/x.py::t", **base))
        if rc != 0:
            failures.append("propose 실패")
        rules = read_canonical(proj, load_config(proj)[0])
        if not rules or rules[0]["status"] != "candidate":
            failures.append("candidate 미등재")
        rid = rules[0]["rule_id"]
        # 워커 권한(--master 부재) promote 거부
        if call(cmd_promote, argparse.Namespace(rule_id=rid, holdout_evidence="h", master=False, **base)) != 2:
            failures.append("master 없는 promote가 거부되지 않음")
        # holdout 없는 promote 거부
        if call(cmd_promote, argparse.Namespace(rule_id=rid, holdout_evidence="", master=True, **base)) != 2:
            failures.append("holdout 없는 promote가 거부되지 않음")
        # 정상 promote
        if call(cmd_promote, argparse.Namespace(rid_placeholder=None, rule_id=rid,
                                                holdout_evidence="run#42 재발0", master=True, **base)) != 0:
            failures.append("promote 실패")
        if rid not in memory_map(mdir):
            failures.append("promote가 memory 미러를 만들지 않음")
        # 정합 sync-check → 0
        if call(cmd_sync_check, argparse.Namespace(fix=False, json=True, **base)) != 0:
            failures.append("정합인데 sync-check가 드리프트 보고")
        # FX-1 유사: memory 미러 삭제 → 드리프트(1)
        os.unlink(os.path.join(mdir, memory_filename(rid)))
        if call(cmd_sync_check, argparse.Namespace(fix=False, json=True, **base)) != 1:
            failures.append("MISSING_IN_MEMORY 드리프트를 검출 못함")
        if call(cmd_sync_check, argparse.Namespace(fix=True, json=True, **base)) != 0 or rid not in memory_map(mdir):
            failures.append("--fix가 memory 미러를 복구 못함")
        # scan-dual-active: 정상 0
        if call(cmd_scan_dual_active, argparse.Namespace(project=proj, json=True)) != 0:
            failures.append("정상인데 dual-active가 충돌 보고")
        # 비-locator active 사본 → exit 2
        stray = os.path.join(proj, "CLAUDE.md")
        write_text_atomic(stray, render_canonical([{
            "rule_id": rid, "status": "active", "canonical_locator": "docs/rules/vibe-rules.md",
            "regression_test_ref": None, "root_cause": None, "superseded_by": None, "body": "위조 사본"}]))
        if call(cmd_scan_dual_active, argparse.Namespace(project=proj, json=True)) != 2:
            failures.append("비-locator active를 fail-closed 하지 못함")
    print(json.dumps({"self_test": "ok" if not failures else "fail", "failures": failures},
                     ensure_ascii=False, indent=2))
    return 0 if not failures else 1


# ── CLI ────────────────────────────────────────────────────────

def main():
    ap = argparse.ArgumentParser(description="증류 수명주기 결정론 도구 (§C8)")
    ap.add_argument("--self-test", action="store_true", help="결정론 자기검증")
    sub = ap.add_subparsers(dest="cmd")

    def add_project(p):
        p.add_argument("--project", required=True, help="프로젝트 루트(.vibecoding/distill.json 포함)")

    p = sub.add_parser("propose", help="candidate 생성(rule_id 자동발급) — 워커 권한")
    add_project(p)
    p.add_argument("--body", required=True, help="규칙 본문")
    p.add_argument("--root-cause", dest="root_cause", required=True, help="확인된 근본원인(C8.1 입장 게이트)")
    p.add_argument("--regression-test-ref", dest="regression_test_ref", required=True,
                   help="재발 방지 테스트 참조")
    p.add_argument("--rule-id", dest="rule_id", default=None, help="(선택) 명시 rule_id — 기본 자동발급")

    p = sub.add_parser("promote", help="active 승격 — holdout ref + --master 필수")
    add_project(p)
    p.add_argument("--rule-id", dest="rule_id", required=True)
    p.add_argument("--holdout-evidence", dest="holdout_evidence", required=True,
                   help="holdout 재발 검증 ref (C8.1)")
    p.add_argument("--master", action="store_true", help="master 전용 승격 게이트")
    p.add_argument("--memory-dir", dest="memory_dir", default=None, help="derivative index 디렉터리")

    p = sub.add_parser("supersede", help="구 rule을 신 rule로 대체 — --master")
    add_project(p)
    p.add_argument("--rule-id", dest="rule_id", required=True, help="대체될 구 rule_id")
    p.add_argument("--by", required=True, help="대체하는 신 rule_id(active)")
    p.add_argument("--master", action="store_true")

    p = sub.add_parser("retire", help="rule 폐기(status=retired) — --master")
    add_project(p)
    p.add_argument("--rule-id", dest="rule_id", required=True)
    p.add_argument("--master", action="store_true")

    p = sub.add_parser("sync-check", help="3저장소 대조(canonical vs memory vs receipt) — FX-1·2·3")
    add_project(p)
    p.add_argument("--memory-dir", dest="memory_dir", default=None)
    p.add_argument("--fix", action="store_true", help="canonical을 SOT로 파생 복구(master sync owner)")
    p.add_argument("--json", action="store_true")

    p = sub.add_parser("scan-dual-active", help="C8.2-A: 비-locator active rule → exit 2 fail-closed")
    add_project(p)
    p.add_argument("--json", action="store_true")

    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.cmd == "propose":
        return cmd_propose(args)
    if args.cmd == "promote":
        return cmd_promote(args)
    if args.cmd == "supersede":
        return cmd_supersede(args)
    if args.cmd == "retire":
        return cmd_retire(args)
    if args.cmd == "sync-check":
        return cmd_sync_check(args)
    if args.cmd == "scan-dual-active":
        return cmd_scan_dual_active(args)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
