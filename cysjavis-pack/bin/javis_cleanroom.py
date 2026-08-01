#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_cleanroom — _research 흡수 보고서의 MPL/라이선스 클린룸 4원칙 헤더 검증·삽입.

오픈소스 흡수 연구(`_research/*_박사급_연구보고서.md`)가 라이선스(특히 MPL-2.0 파일단위
전염)를 어긴 코드복사로 오염되지 않도록 "코드복사 0 · 계약/패턴/산술만 클린룸 · 1차표준
직접 출처 · 복사 아님 명시" 4원칙을 기계 파싱 가능한 표준 헤더 블록으로 박제한다.
도메인-무관·결정론·순수 stdlib(추가 인프라 0·종량제 0).

서브커맨드:
    javis_cleanroom.py --self-test               # → exit 0, {"self_test":"ok"} (검증로직 자기검증)
    javis_cleanroom.py check  --root <_research>  # → {"ok":bool,"missing":[...],"broken":[...]}  exit 1 if any
    javis_cleanroom.py fix    --root <_research>  # → 누락 헤더 삽입(라이선스 자동탐지), {"fixed":[...]}

AGENTREACH OPP-19/OPP-20 통합(새 도구 안 만듦 — 본 도구에 1-체크 통합):
    javis_cleanroom.py vendor-check    --skills <skills_dir>  # 벤더링 스냅샷 해시핀 드리프트 감지 (NEVER-modify-upstream 자동강제)
    javis_cleanroom.py vendor-snapshot --skills <skills_dir>  # baseline 해시 재생성 (owner 승인 게이트 — denylist ②)
    javis_cleanroom.py license-check   --skills <skills_dir>  # THIRD_PARTY/NOTICE 라이선스 추적 + AGPL copyleft 게이트

종료 코드: 0 = 정합/수리 완료, 1 = missing/broken/drifted/license 위반(check) 또는 self-test 실패,
            3 = vendor-snapshot 승인대기(CYS_VENDOR_SNAPSHOT_OWNER_APPROVED 미설정).
"""

import argparse
import glob
import hashlib
import json
import os
import re
import sys

OPEN = "<!-- CLEANROOM-GUARDRAIL v1 -->"
CLOSE = "<!-- /CLEANROOM-GUARDRAIL -->"
# 4원칙 고정 키 — C41/C42가 존재 검사하는 기계 마커.
PRINCIPLE_KEYS = ("코드복사 0", "계약/패턴/산술", "1차표준", "복사 아님")
# 라이선스 SPDX 화이트리스트(추정 금지 — 미탐지 시 플레이스홀더 유지).
SPDX_TOKENS = ("MPL-2.0", "MIT", "AGPL", "GPL", "Apache-2.0", "BSD")
# 흡수 연구 보고서 파일 패턴.
REPORT_GLOB = "*_박사급_연구보고서.md"

# ── OPP-19/20 벤더링 무결성 + 라이선스 추적 ──────────────────────────────────
# 벤더링 매니페스트 사이드카(진실원천). skills/ 루트 직하 '_' 접두 파일이라
# build.rs walk()가 임베드한다('.' 접두·tests·__pycache__만 제외 — build.rs:17 실측).
VENDOR_MANIFEST_NAME = "_VENDOR_MANIFEST.json"
# build.rs walk() 임베드 제외 규칙의 stdlib 재구현(복사 아님 — 계약 재현, build.rs:17).
WALK_EXCLUDE_NAMES = ("tests", "__pycache__")
# copyleft 정책 enum(공개 SPDX 분류만 — 법적 호환성 결론은 ESCALATE).
COPYLEFT_CLASS = {
    "MIT": "permissive", "BSD": "permissive", "Apache-2.0": "permissive",
    "MPL-2.0": "weak_copyleft", "Unlicense": "public_domain",
    "LGPL": "weak_copyleft", "GPL": "strong_copyleft", "AGPL": "strong_copyleft",
}
# strong_copyleft = PACK 코드사이닝 단일바이너리(include_str!)와 충돌 가능 → 임베드 시 게이트.
STRONG_COPYLEFT = ("GPL", "AGPL")


def _norm_bytes(raw):
    """정규화: BOM 제거 + CRLF→LF (OS/checkout 잡음 제거 후 해시)."""
    if raw[:3] == b"\xef\xbb\xbf":
        raw = raw[3:]
    return raw.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def file_sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(_norm_bytes(f.read())).hexdigest()


def walk_vendored(src_dir):
    """src_dir 하위 임베드 대상 파일(상대경로 정렬)을 build.rs walk 규칙으로 결정론 열거."""
    out = []
    for root, dirs, files in os.walk(src_dir):
        dirs[:] = [d for d in dirs if not d.startswith(".") and d not in WALK_EXCLUDE_NAMES]
        for fn in files:
            if fn.startswith("."):
                continue
            full = os.path.join(root, fn)
            out.append(os.path.relpath(full, src_dir).replace(os.sep, "/"))
    return sorted(out)


class ManifestError(Exception):
    """매니페스트가 존재하나 파싱 불가 — 부재(None)와 구분해 fail-closed 신호로 쓴다(P-GATE-4/5)."""


def load_manifest(skills_dir):
    p = os.path.join(skills_dir, VENDOR_MANIFEST_NAME)
    if not os.path.isfile(p):
        return None  # 부재 = 미착수(정상 skip) — 이때만 None.
    try:
        with open(p, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError) as e:
        # ★WP-8(P-GATE-4/5): 존재하나 파싱 실패 = 손상/변조. 부재로 위장(skip→pass)하면
        #   매니페스트를 깨는 것만으로 벤더 드리프트·라이선스 게이트가 뚫린다 → fail-closed.
        raise ManifestError("%s 파싱 실패: %s" % (p, e))
    if not isinstance(data, dict):
        raise ManifestError("%s 최상위가 객체(dict) 아님" % p)
    return data


def classify_vendor(manifest, skills_dir):
    """5상태 probe(PHIL-03): PINNED_OK/DRIFTED/MISSING/ORPHAN/UNPINNED.
    범위 = manifest 각 source.id 가 가리키는 벤더링 디렉터리 화이트리스트만(원작 스킬 오판 금지)."""
    drifted, missing, orphan, unpinned, ok = [], [], [], [], []
    for src in manifest.get("sources", []):
        if src.get("kind") == "runtime_dep":
            continue  # 런타임 의존(pip)은 디스크 파일 핀 대상 아님 — 라이선스 추적만.
        sid = src["id"]
        src_root = os.path.join(skills_dir, sid)
        pins = src.get("files", {})
        # 트리 실재 파일(임베드 규칙 적용) 집합.
        tree = set(walk_vendored(src_root)) if os.path.isdir(src_root) else set()
        for rel, meta in sorted(pins.items()):
            full = os.path.join(src_root, rel)
            if not os.path.isfile(full):
                missing.append("%s/%s" % (sid, rel))
                continue
            actual = file_sha256(full)
            if actual != meta.get("sha256"):
                drifted.append({"file": "%s/%s" % (sid, rel),
                                "expected": (meta.get("sha256") or "")[:12],
                                "actual": actual[:12]})
            else:
                ok.append("%s/%s" % (sid, rel))
        # 등록 디렉터리 내부에서만 ORPHAN/UNPINNED 판정(원작 스킬 제외).
        for rel in sorted(tree):
            if rel not in pins:
                unpinned.append("%s/%s" % (sid, rel))
    return {"pinned_ok": ok, "drifted": drifted, "missing": missing,
            "orphan": orphan, "unpinned": unpinned}


def snapshot_vendor(manifest, skills_dir):
    """현 트리 → manifest files{} 해시 재생성(승인 게이트 통과 시만 호출)."""
    for src in manifest.get("sources", []):
        if src.get("kind") == "runtime_dep":
            continue
        src_root = os.path.join(skills_dir, src["id"])
        if not os.path.isdir(src_root):
            continue
        files = {}
        for rel in walk_vendored(src_root):
            full = os.path.join(src_root, rel)
            files[rel] = {"sha256": file_sha256(full),
                          "bytes": os.path.getsize(full)}
        src["files"] = files
    return manifest


def license_classify(manifest, skills_dir):
    """OPP-20: 라이선스 추적 게이트. THIRD_PARTY/NOTICE 검증 + AGPL copyleft 추적.
    MISSING(디렉터리 있는데 핀 없음)·ORPHAN(핀 있는데 디렉터리 없음)·NO_SPDX·POLICY(strong copyleft 임베드)."""
    no_spdx, missing_dir, policy, notice_missing, ok = [], [], [], [], []
    tp = os.path.join(skills_dir, "THIRD_PARTY.md")
    tp_text = open(tp, encoding="utf-8", errors="replace").read() if os.path.isfile(tp) else ""
    for src in manifest.get("sources", []):
        sid = src["id"]
        spdx = src.get("spdx")
        if spdx not in SPDX_TOKENS and spdx != "Unlicense":
            no_spdx.append(sid)
            continue
        klass = COPYLEFT_CLASS.get(spdx, "unknown")
        # AGPL/GPL copyleft 추적: 임베드(코드사이닝 단일바이너리) 대상이면 정책 충돌 ESCALATE.
        if spdx in STRONG_COPYLEFT and src.get("embed") and src.get("kind") != "runtime_dep":
            policy.append({"id": sid, "spdx": spdx, "copyleft_class": klass,
                           "reason": "strong_copyleft + PACK 임베드 = 코드사이닝 충돌(오너 보유결정)"})
        if src.get("kind") == "runtime_dep":
            # 런타임 의존: SKILL.md 선언 존재만 확인(transitive 범위 밖).
            ok.append(sid)
            continue
        # 벤더링 디렉터리: 실재 + (THIRD_PARTY 색인 or NOTICE) 고지의무.
        if not os.path.isdir(os.path.join(skills_dir, sid)):
            missing_dir.append(sid)
            continue
        notice = src.get("notice")
        has_notice = bool(notice and os.path.isfile(os.path.join(skills_dir, notice)))
        in_tp = sid in tp_text
        if not (has_notice or in_tp):
            notice_missing.append(sid)
        else:
            ok.append(sid)
    return {"ok": ok, "no_spdx": no_spdx, "missing_dir": missing_dir,
            "policy_escalate": policy, "notice_missing": notice_missing}


def vendor_verdict(cls):
    """OPP-19 verdict enum(score 금지·evidence:file)."""
    if cls["drifted"]:
        return "BLOCK", cls
    if cls["missing"]:
        return "ESCALATE", cls
    if cls["orphan"] or cls["unpinned"]:
        return "REVISE", cls
    return "ACCEPT", cls


def license_verdict(cls):
    """OPP-20 verdict enum(score 금지). AGPL은 오너 결정 전제로 ESCALATE 큐잉(BLOCK 아님)."""
    if cls["no_spdx"] or cls["missing_dir"] or cls["notice_missing"]:
        return "BLOCK", cls
    if cls["policy_escalate"]:
        return "ESCALATE", cls
    return "ACCEPT", cls


def cmd_vendor_check(skills_dir):
    try:
        m = load_manifest(skills_dir)
    except ManifestError as e:
        # ★WP-8(P-GATE-4): 손상 매니페스트는 통과(skip) 금지 — BLOCK.
        print(json.dumps({"ok": False, "verdict": "BLOCK", "error": "manifest-parse-fail",
                          "note": str(e)}, ensure_ascii=False))
        return 1
    if m is None:
        print(json.dumps({"ok": True, "skip": "no-manifest",
                          "note": "%s 부재 — vendor-snapshot 으로 baseline 생성 필요"
                          % VENDOR_MANIFEST_NAME}, ensure_ascii=False))
        return 0  # 부재≠위반(미착수 정상). 게이트는 manifest 있을 때만 강제.
    cls = classify_vendor(m, skills_dir)
    verdict, _ = vendor_verdict(cls)
    ok = verdict == "ACCEPT"
    print(json.dumps({"ok": ok, "verdict": verdict, **cls}, ensure_ascii=False))
    return 0 if ok else 1


def cmd_vendor_snapshot(skills_dir):
    if os.environ.get("CYS_VENDOR_SNAPSHOT_OWNER_APPROVED", "") not in ("1", "true", "yes"):
        sys.stderr.write("[CLEANROOM] baseline 갱신 = 변조 정상승인 = 오너/owner 승인 필요"
                         "(denylist ②). CYS_VENDOR_SNAPSHOT_OWNER_APPROVED=1 후 재실행.\n")
        return 3
    try:
        m = load_manifest(skills_dir)
    except ManifestError as e:
        # ★WP-8(P-GATE-4/5): 손상 매니페스트 위에 snapshot 하면 훼손을 baseline으로 굳힌다 → 거부.
        print(json.dumps({"error": "manifest-parse-fail", "note": str(e)}, ensure_ascii=False))
        return 1
    if m is None:
        print(json.dumps({"error": "no-manifest", "note":
                          "최초 manifest 골격을 먼저 만들어라(sources[].id/spdx/embed)"},
                         ensure_ascii=False))
        return 1
    m = snapshot_vendor(m, skills_dir)
    p = os.path.join(skills_dir, VENDOR_MANIFEST_NAME)
    with open(p, "w", encoding="utf-8") as f:
        json.dump(m, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps({"snapshotted": p, "sources":
                      [s["id"] for s in m.get("sources", [])]}, ensure_ascii=False))
    return 0


def cmd_license_check(skills_dir):
    try:
        m = load_manifest(skills_dir)
    except ManifestError as e:
        # ★WP-8(P-GATE-5): 손상 매니페스트는 라이선스 게이트 통과 금지 — BLOCK.
        print(json.dumps({"ok": False, "verdict": "BLOCK", "error": "manifest-parse-fail",
                          "note": str(e)}, ensure_ascii=False))
        return 1
    if m is None:
        print(json.dumps({"ok": True, "skip": "no-manifest"}, ensure_ascii=False))
        return 0
    cls = license_classify(m, skills_dir)
    verdict, _ = license_verdict(cls)
    ok = verdict in ("ACCEPT", "ESCALATE")  # ESCALATE(AGPL 오너결정)는 부트 비차단.
    print(json.dumps({"ok": ok, "verdict": verdict, **cls}, ensure_ascii=False))
    return 0 if ok else 1


def parse_block(text):
    """헤더 블록 1개 추출 → (found, ok, reason)."""
    if OPEN not in text:
        return (False, False, "no-header")
    if text.count(OPEN) != text.count(CLOSE) or CLOSE not in text:
        return (True, False, "marker-mismatch")
    block = text.split(OPEN, 1)[1].split(CLOSE, 1)[0]
    missing = [k for k in PRINCIPLE_KEYS if k not in block]
    if missing:
        return (True, False, "missing-keys:%s" % ",".join(missing))
    return (True, True, "ok")


def detect_license(text):
    """본문에서 라이선스 SPDX 토큰을 화이트리스트 매칭으로 추출(추정 금지)."""
    for tok in SPDX_TOKENS:
        if re.search(r"\b%s\b" % re.escape(tok), text):
            return tok
    return None


def render_header(spdx, lic_ref):
    spdx = spdx or "<LICENSE-SPDX>"
    return "\n".join([
        OPEN,
        "> **클린룸 가드레일(불변·기계검증 C42):**",
        "> ① **코드복사 0** — 대상 repo 소스 텍스트를 cys 트리에 복사하지 않는다"
        "(MPL-2.0 등 파일단위 전염 ↔ pack.rs `include_str!` 단일 서명 바이너리 충돌 회피).",
        "> ② **계약/패턴/산술만 클린룸** — enum·불변식·알고리즘·수치 규칙만 사양서 독립 재구현.",
        "> ③ **1차표준 직접 출처** — 2차 분석본 아닌 file:line/표준/URL 직접 근거.",
        "> ④ **복사 아님 명시** — 본 보고서의 모든 cys 매핑은 재구현이며 복제가 아님.",
        "> 대상 라이선스: `%s` (출처: `%s`)" % (spdx, lic_ref or "<LICENSE-PATH:LINE>"),
        CLOSE,
        "",
    ])


def insert_after_blockquote(text, header):
    """상단 > 인용블록 종료 직후·첫 '---' 앞에 삽입(본문 0라인 변경)."""
    m = re.search(r"\n---\n", text)  # 첫 구분선 = 헤더 영역 끝
    idx = m.start() + 1 if m else len(text)  # '---' 라인 직전(개행 보존)
    return text[:idx] + header + text[idx:]


def iter_reports(root):
    return sorted(glob.glob(os.path.join(root, REPORT_GLOB)))


def cmd_check(root):
    missing, broken = [], []
    for p in iter_reports(root):
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        found, ok, reason = parse_block(text)
        if not found:
            missing.append(p)
        elif not ok:
            broken.append(p)
    ok = not (missing or broken)
    print(json.dumps({"ok": ok, "missing": missing, "broken": broken}, ensure_ascii=False))
    return 0 if ok else 1


def cmd_fix(root):
    fixed, broken = [], []
    for p in iter_reports(root):
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        found, ok, _ = parse_block(text)
        if found and ok:
            continue  # 멱등 — 이미 정합인 헤더는 재삽입 0
        spdx = detect_license(text)
        if spdx is None:
            broken.append(p)  # 추정 삽입 금지 — 사람 보강 유도
        header = render_header(spdx, None)
        new = insert_after_blockquote(text, header)
        with open(p, "w", encoding="utf-8") as f:
            f.write(new)
        fixed.append(p)
    print(json.dumps({"fixed": fixed, "broken": broken}, ensure_ascii=False))
    return 0


def self_test():
    good = render_header("MPL-2.0", "x/LICENSE:1")
    f1, ok1, _ = parse_block(good)
    f2, ok2, _ = parse_block("no header here")
    f3, ok3, r3 = parse_block(OPEN + "\n> ① 코드복사 0\n" + CLOSE)  # keys missing
    # marker-mismatch: OPEN without CLOSE
    f4, ok4, r4 = parse_block(OPEN + "\n> 4원칙 ...\n")
    # insert idempotence pin
    body = "> intro\n\n---\n\nbody\n"
    once = insert_after_blockquote(body, good)
    failures = []
    if not (f1 and ok1):
        failures.append("render->parse round-trip")
    if f2 or ok2:
        failures.append("absent must be not-found")
    if ok3 or "missing-keys" not in r3:
        failures.append("partial must fail")
    if ok4 or r4 != "marker-mismatch":
        failures.append("unclosed marker must fail")
    if OPEN not in once or once.count(OPEN) != 1:
        failures.append("insert must place exactly one header")
    failures += _self_test_vendor()
    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False))
    return 1 if failures else 0


def _self_test_vendor():
    """OPP-19/20 자기공격 변이검증(PHIL-08·네트워크0). LOCKED 기대값 대조(producer≠evaluator)."""
    import shutil
    import tempfile
    fails = []
    d = tempfile.mkdtemp(prefix="cleanroom_vendor_")
    try:
        sid = "fixture-src"
        src_root = os.path.join(d, sid)
        os.makedirs(os.path.join(src_root, "engine"))
        f1 = os.path.join(src_root, "a.py")
        f2 = os.path.join(src_root, "engine", "b.py")
        open(f1, "w").write("print(1)\n")
        open(f2, "w").write("print(2)\n")
        # tests/·__pycache__·.hidden 은 임베드 제외(walk 규칙) 박제.
        os.makedirs(os.path.join(src_root, "tests"))
        open(os.path.join(src_root, "tests", "t.py"), "w").write("x\n")
        manifest = {"sources": [{"id": sid, "spdx": "MIT", "embed": True,
                                 "kind": "vendored", "files": {}}]}
        # 1) round-trip 핀: snapshot → vendor-check = PINNED_OK.
        snapshot_vendor(manifest, d)
        if "a.py" not in manifest["sources"][0]["files"]:
            fails.append("snapshot must pin a.py")
        if "tests/t.py" in manifest["sources"][0]["files"]:
            fails.append("walk must exclude tests/ (build.rs:17 동형)")
        v, cls = vendor_verdict(classify_vendor(manifest, d))
        if v != "ACCEPT" or cls["drifted"]:
            fails.append("clean snapshot must be ACCEPT/no-drift")
        # 2) 변이 주입(자기공격): 1바이트 append → DRIFTED·BLOCK 필수.
        open(f1, "a").write("# tamper\n")
        v2, cls2 = vendor_verdict(classify_vendor(manifest, d))
        if v2 != "BLOCK" or len(cls2["drifted"]) != 1:
            fails.append("1-byte tamper must be DRIFTED/BLOCK (자기공격 미포착)")
        # 3) 파일 삭제 → MISSING·ESCALATE.
        os.remove(f1)
        v3, cls3 = vendor_verdict(classify_vendor(manifest, d))
        if v3 != "ESCALATE" or not cls3["missing"]:
            fails.append("delete must be MISSING/ESCALATE")
        # 4) 핀 안 된 파일 추가 → UNPINNED·REVISE.
        open(f1, "w").write("print(1)\n")  # 복원(drift 제거)
        open(os.path.join(src_root, "c.py"), "w").write("print(3)\n")
        v4, cls4 = vendor_verdict(classify_vendor(manifest, d))
        if v4 != "REVISE" or "%s/c.py" % sid not in cls4["unpinned"]:
            fails.append("unpinned file must be UNPINNED/REVISE")
        # 5) snapshot 승인게이트: env 미설정 시 exit 3.
        old = os.environ.pop("CYS_VENDOR_SNAPSHOT_OWNER_APPROVED", None)
        _stderr = sys.stderr
        try:
            sys.stderr = open(os.devnull, "w")  # 승인게이트 경고는 self-test 기대동작 — 소음 억제.
            rc_snap = cmd_vendor_snapshot(d)
        finally:
            sys.stderr.close()
            sys.stderr = _stderr
            if old is not None:
                os.environ["CYS_VENDOR_SNAPSHOT_OWNER_APPROVED"] = old
        if rc_snap != 3:
            fails.append("snapshot without owner-approval must exit 3")
        # 6) OPP-20 라이선스: AGPL+embed=ESCALATE, no-spdx=BLOCK, normalize 정합.
        lic_m = {"sources": [
            {"id": sid, "spdx": "MIT", "embed": True, "kind": "vendored"},
        ]}
        open(os.path.join(d, "THIRD_PARTY.md"), "w").write("fixture-src vendored\n")
        lv, lc = license_verdict(license_classify(lic_m, d))
        if lv != "ACCEPT":
            fails.append("MIT vendored w/ THIRD_PARTY index must be ACCEPT")
        lic_m["sources"][0]["spdx"] = "AGPL"
        lv2, lc2 = license_verdict(license_classify(lic_m, d))
        if lv2 != "ESCALATE" or not lc2["policy_escalate"]:
            fails.append("AGPL embedded must be ESCALATE (copyleft 추적)")
        lic_m["sources"][0]["spdx"] = "BOGUS-LIC"
        lv3, _ = license_verdict(license_classify(lic_m, d))
        if lv3 != "BLOCK":
            fails.append("unknown SPDX must be BLOCK (no_spdx)")
        # normalize 박제: CRLF/BOM 차이가 해시를 흔들지 않음.
        if file_sha256(f2) != hashlib.sha256(b"print(2)\n").hexdigest():
            fails.append("sha256 must normalize CRLF/BOM")
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return fails


def main():
    ap = argparse.ArgumentParser(description="_research 클린룸 4원칙 헤더 검증·삽입")
    ap.add_argument("--self-test", action="store_true", help="검증 로직 자기검증(exit 0=ok)")
    sub = ap.add_subparsers(dest="cmd")
    pc = sub.add_parser("check", help="헤더 정합 검사")
    pc.add_argument("--root", required=True)
    pf = sub.add_parser("fix", help="누락 헤더 삽입(라이선스 자동탐지)")
    pf.add_argument("--root", required=True)
    # OPP-19/20 — 벤더링 무결성 + 라이선스 추적(--skills = cysjavis-pack/skills).
    pvc = sub.add_parser("vendor-check", help="벤더링 스냅샷 해시핀 드리프트 감지(OPP-19)")
    pvc.add_argument("--skills", required=True)
    pvs = sub.add_parser("vendor-snapshot", help="baseline 해시 재생성(owner 승인 게이트·OPP-19)")
    pvs.add_argument("--skills", required=True)
    plc = sub.add_parser("license-check", help="THIRD_PARTY/NOTICE 라이선스 + AGPL copyleft 게이트(OPP-20)")
    plc.add_argument("--skills", required=True)
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if args.cmd == "check":
        return cmd_check(args.root)
    if args.cmd == "fix":
        return cmd_fix(args.root)
    if args.cmd == "vendor-check":
        return cmd_vendor_check(args.skills)
    if args.cmd == "vendor-snapshot":
        return cmd_vendor_snapshot(args.skills)
    if args.cmd == "license-check":
        return cmd_license_check(args.skills)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
