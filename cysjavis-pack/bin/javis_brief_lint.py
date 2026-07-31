#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_brief_lint.py — 위임 브리프 결정론 린트 (Phase 1 T1 · DESIGN-DECISIONS §1-3).

"검증 수단 없는 위임은 접수 거부"의 입구 게이트. 브리프(BRIEF_CONTRACT 5필드:
task/stretch/guardrails/exit_criteria/verify[])를 기계 검사한다. LLM 0 · 네트워크 0 ·
표준 라이브러리만. 산문 의미 판정은 하지 않는다 — 형식·결정론 규칙만(over-claim 금지).

검사 4종(설계 §1-3):
  ① verify[] 형식 스키마 — verify 항목이 '실행형'(실행 가능 명령/locked-eval ref/exit code
     산출형)인지. '스스로 재검토하라'류 자기검토 문구는 검증 수단으로 불인정(연구 §수정방향 ④
     — 산문 감지가 아니라 형식 스키마로 결정론화). 부재·빈 목록·비실행형 항목 = hard.
  ② stretch×verify 짝 — stretch(난이도 상향)는 캘리브레이션된 검증([calibrated] 태그 또는
     JSON 항목 calibrated:true) 동봉 시에만 허용. 위반 = 경고(fail-open).
  ③ 마이크로매니징 감지 — 명령형 마커(MUST/CRITICAL/절대/반드시/무조건) 카운트·번호 스텝 밀도
     임계 초과 시 '상위 수준 과제 서술로 재작성' 경고(fail-open).
  ④ 8KB 초과 경고(조건 12 — 파일 참조 위임에서도 브리프 비대는 워커 baseline 상승) — 경고.

exit 계약(T1 브리프 고정):
  0 = 통과
  1 = 경고만(fail-open 경로 — 소비측은 차단 금지, 경고 주입만)
  2 = hard 거부(검사 ① 위반·브리프 파일 읽기 실패) — fail-closed 소비는
      javis_task checkout --brief 경로**만**(조건 05①·21①·28). 인세션 Agent/Task 경고
      훅(brief-lint-warn.sh)은 이 exit 를 차단으로 쓰지 않는다(fail-open).

브리프 형식(BRIEF_CONTRACT — _work/phase1-impl-package/contracts/BRIEF_CONTRACT.md):
  · JSON: {"task": "...", "stretch": ..., "guardrails": ..., "exit_criteria": ...,
           "verify": ["python3 x.py self-test → exit 0", {"cmd": "...", "calibrated": true}]}
  · Markdown: 필드 = 헤딩(#..######) 제목 또는 행머리 라벨('verify:')로 표기.
    별칭: task|과제 · stretch|스트레치 · guardrails|가드레일 · exit_criteria|완료 기준|출구 기준 ·
    verify|검증. verify 항목 = 해당 섹션의 리스트 항목(-·*·+·숫자.) 1줄 1항목.

사용:
    python3 javis_brief_lint.py lint <FILE> [--json]
    python3 javis_brief_lint.py --self-test          # 밀폐 자기검증(무 네트워크·tmp 만)
검사는 FLOOR 다 — 통과는 필요조건이지 충분조건이 아니다(리뷰어 라운드·4자 수렴 대체 불가 ·
조건 30①). 이 게이트는 브리프의 '형식'만 잠근다.
"""

import argparse
import json
import os
import re
import sys

EXIT_OK, EXIT_WARN, EXIT_HARD = 0, 1, 2

FIELDS = ("task", "stretch", "guardrails", "exit_criteria", "verify")

# 필드 별칭(마크다운 헤딩·라벨 탐지) — 우선순위 순서: 한 제목이 여러 별칭을 담으면 앞선 필드 승리.
_FIELD_ALIASES = (
    ("verify", ("verify", "검증")),
    ("stretch", ("stretch", "스트레치")),
    ("exit_criteria", ("exit_criteria", "exit criteria", "완료 기준", "완료기준", "출구 기준")),
    ("guardrails", ("guardrails", "가드레일")),
    ("task", ("task", "과제")),
)

_HEADING_RE = re.compile(r"^ {0,3}#{1,6}\s+(.*)$")
_LABEL_RE = re.compile(r"^(task|stretch|guardrails|exit[_ ]criteria|verify|과제|스트레치|가드레일|완료\s*기준|출구\s*기준|검증)\s*[:：]\s*(.*)$", re.I)
_LIST_ITEM_RE = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+(.+)$")

# ① 실행형 판정 — 명령 러너 토큰·상대경로 실행·locked-eval ref 중 하나는 있어야 한다.
#   A2: make·go 는 산문 충돌 토큰이라 일반 목록에서 분리 — 아래 _PROSE_CMD_RE 참조.
_COMMAND_RE = re.compile(
    r"(^|[\s`\(\[])((python3?|pytest|sh|bash|zsh|npm|npx|node|cargo|git|cys|pip3?|ruby|perl|rustc)\b|\./)",
    re.I)
# A2: make·go 명령 위치 휴리스틱 — 토큰 뒤 '실인자'(타깃·서브커맨드·플래그) 요구 +
#   "make sure"/"go over" 류 자연어 후속 명시 제외. go 는 서브커맨드 닫힌 집합(결정론),
#   make 는 산문 후속 stoplist 제외 후 타깃 토큰 1개 요구.
_PROSE_CMD_RE = re.compile(
    r"(^|[\s`\(\[])(?:"
    r"go\s+(?:test|build|run|vet|mod|fmt|generate|tool|install|version|env)\b"
    r"|make\s+(?!(?:sure|certain|it|this|that|these|those|the|a|an|and|or|no|not|do|be|use|"
    r"sense|room|time|way|good|up|out|over|your|our|my|their|his|her|its)\b)[A-Za-z0-9_.-]"
    r")", re.I)
_LOCKED_EVAL_RE = re.compile(r"locked[-_ ]?eval|eval[-_ ]ref", re.I)
# 자기검토 문구 — 실행형 근거 없이 이것만 있으면 검증 수단 불인정(Huang et al. ICLR 2024 근거).
_SELF_REVIEW_RE = re.compile(
    r"스스로|자기\s*검토|자체\s*검토|재검토|double[-\s]?check|self[-\s]?review|review\s+it\s+yourself", re.I)
# calibrated 표식 — 문자열 항목은 [calibrated] 태그, JSON dict 항목은 calibrated:true.
_CALIBRATED_TAG_RE = re.compile(r"\[calibrated\]", re.I)

# ③ 마이크로매니징 임계(결정론 상수 — BRIEF_CONTRACT 에 병기).
_IMPERATIVE_RE = re.compile(r"\bMUST\b|\bCRITICAL\b|반드시|무조건|절대")
IMPERATIVE_WARN_COUNT = 12
_STEP_LINE_RE = re.compile(r"^\s*\d+[.)]\s")
STEP_LINES_WARN_COUNT = 25

# ④ 8KB 경고(조건 12).
BRIEF_SIZE_WARN_BYTES = 8192


def _match_field(title):
    """제목/라벨 문자열 → 필드명 또는 None. 별칭 부분일치·우선순위 순."""
    t = " ".join(title.strip().lower().split())
    for field, aliases in _FIELD_ALIASES:
        for al in aliases:
            if al in t:
                return field
    return None


def _parse_markdown(text):
    """마크다운 브리프 → {field: [내용 줄...], "verify": [항목...]} (결정론 파싱)."""
    fields = {}
    cur = None
    for line in text.splitlines():
        m = _HEADING_RE.match(line)
        if m:
            f = _match_field(m.group(1))
            cur = f  # 필드 아닌 헤딩은 섹션 이탈(cur=None) — 무관 섹션 내용 오귀속 차단
            if f:
                fields.setdefault(f, [])
            continue
        m = _LABEL_RE.match(line)
        if m:
            f = _match_field(m.group(1))
            if f:
                cur = f
                fields.setdefault(f, [])
                rest = m.group(2).strip()
                if rest:
                    fields[f].append(rest)
            continue
        if cur:
            fields[cur].append(line)
    out = {}
    for f, lines in fields.items():
        if f == "verify":
            items = [m.group(1).strip() for ln in lines for m in [_LIST_ITEM_RE.match(ln)] if m]
            # 리스트 항목이 하나도 없으면 비어있지 않은 본문 줄을 항목으로 승격(라벨 1줄 형식 허용)
            if not items:
                items = [ln.strip() for ln in lines if ln.strip()]
            out[f] = items
        else:
            body = "\n".join(ln for ln in lines).strip()
            out[f] = body
    return out


def _parse_brief(text):
    """브리프 텍스트 → 필드 dict. JSON 객체면 JSON 모드, 아니면 마크다운 모드."""
    try:
        obj = json.loads(text.lstrip("﻿"))
        if isinstance(obj, dict):
            return obj
    except ValueError:
        pass
    return _parse_markdown(text)


def _entry_text(entry):
    """verify 항목 → 판정용 문자열(dict 항목은 cmd 필드)."""
    if isinstance(entry, dict):
        return str(entry.get("cmd") or "")
    return str(entry or "")


def _entry_executable(entry):
    """검사 ①: verify 항목이 실행형인가 → (ok, 사유)."""
    txt = _entry_text(entry)
    if not txt.strip():
        return False, "빈 항목(실행 명령 없음)"
    has_cmd = (bool(_COMMAND_RE.search(txt)) or bool(_PROSE_CMD_RE.search(txt))
               or bool(_LOCKED_EVAL_RE.search(txt)))
    if has_cmd:
        return True, None
    if _SELF_REVIEW_RE.search(txt):
        return False, "자기검토 문구 — 외부 실행 검증 아님(불인정): %r" % txt[:60]
    return False, "실행형 아님(러너 토큰·./경로·locked-eval ref 없음): %r" % txt[:60]


def _entry_calibrated(entry):
    if isinstance(entry, dict):
        return entry.get("calibrated") is True
    return bool(_CALIBRATED_TAG_RE.search(str(entry or "")))


def _stretch_present(fields):
    v = fields.get("stretch")
    if isinstance(v, str):
        return bool(v.strip())
    if isinstance(v, (list, dict)):
        return len(v) > 0
    return v is not None and v is not False


def lint_text(text, size_bytes=None):
    """순수 CHECK — 브리프 원문 → (exit_code, findings[]). I/O·전역상태 무접촉(밀폐).

    findings 각 줄은 '[HARD] …' 또는 '[WARN] …'. exit: 0 통과 · 1 경고만 · 2 hard(검사 ①).
    """
    findings = []
    hard = False
    fields = _parse_brief(text)

    # ① verify[] 형식 스키마(hard)
    verify = fields.get("verify")
    if isinstance(verify, str):
        verify = [verify] if verify.strip() else []
    if not isinstance(verify, list) or not verify:
        hard = True
        findings.append("[HARD] verify[] 부재·빈 목록 — 검증 수단 없는 위임은 접수 거부"
                        "(실행형 verify 항목 ≥1 필수 · BRIEF_CONTRACT)")
    else:
        for i, entry in enumerate(verify):
            ok, why = _entry_executable(entry)
            if not ok:
                hard = True
                findings.append("[HARD] verify[%d] %s" % (i, why))

    # ② stretch×verify 짝(경고)
    if _stretch_present(fields):
        entries = verify if isinstance(verify, list) else []
        if not any(_entry_calibrated(e) for e in entries):
            findings.append("[WARN] stretch 동봉인데 캘리브레이션된 verify 없음 — 난이도 상향은 "
                            "[calibrated] 태그(또는 JSON calibrated:true) 검증 동봉 시에만(설계 §1-3 ②)")

    # ③ 마이크로매니징(경고)
    imp = len(_IMPERATIVE_RE.findall(text))
    steps = sum(1 for ln in text.splitlines() if _STEP_LINE_RE.match(ln))
    if imp >= IMPERATIVE_WARN_COUNT or steps >= STEP_LINES_WARN_COUNT:
        findings.append("[WARN] 마이크로매니징 의심(명령형 마커 %d≥%d 또는 번호 스텝 %d≥%d) — "
                        "상위 수준 과제 서술로 재작성 권고(경고만·차단 아님)"
                        % (imp, IMPERATIVE_WARN_COUNT, steps, STEP_LINES_WARN_COUNT))

    # ④ 8KB 경고(조건 12)
    nbytes = size_bytes if size_bytes is not None else len(text.encode("utf-8", "replace"))
    if nbytes > BRIEF_SIZE_WARN_BYTES:
        findings.append("[WARN] 브리프 %d바이트 > %d(8KB) — 파일 참조 위임이라도 비대 브리프는 "
                        "워커 시작 baseline 상승(조건 12) · 분할 권고" % (nbytes, BRIEF_SIZE_WARN_BYTES))

    if hard:
        return EXIT_HARD, findings
    if findings:
        return EXIT_WARN, findings
    return EXIT_OK, findings


def lint_file(path):
    """파일 경유 린트 → (exit_code, findings[]). 읽기 실패 = hard(2) — 이 fail-closed 는
    checkout --brief 경로 소비 전제다(경고 훅 경로는 자체 fail-open 래핑이 흡수)."""
    try:
        size = os.path.getsize(path)
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError as e:
        return EXIT_HARD, ["[HARD] 브리프 파일 읽기 실패: %s (%s)" % (path, e)]
    return lint_text(text, size_bytes=size)


def cmd_lint(path, as_json):
    code, findings = lint_file(path)
    if as_json:
        print(json.dumps({"ok": code == EXIT_OK, "exit": code, "file": path,
                          "findings": findings}, ensure_ascii=False, indent=2))
    else:
        for ln in findings:
            print(ln)
        verdict = {EXIT_OK: "PASS", EXIT_WARN: "WARN(fail-open)", EXIT_HARD: "REJECT(hard)"}[code]
        print("brief-lint: %s — %s (exit %d)" % (verdict, path, code))
        if code == EXIT_HARD:
            print("hard 거부 소비는 checkout --brief 경로만 — 인세션 경고 훅은 차단 금지(조건 05③).")
    return code


def self_test():
    """밀폐 배터리 — 각 검사·경계가 정확한 exit 로 잡히는지(무 네트워크·tmp 파일만)."""
    failures = []

    def chk(name, text, want_code, want_substr=None):
        code, findings = lint_text(text)
        if code != want_code:
            failures.append("%s: exit=%d 기대=%d (findings=%s)" % (name, code, want_code, findings))
        elif want_substr and not any(want_substr in f for f in findings):
            failures.append("%s: findings 에 %r 없음 — %s" % (name, want_substr, findings))

    MD_OK = ("# task\n계약 구현\n\n# 완료 기준\n- 게이트 통과\n\n# verify\n"
             "- python3 bin/javis_task.py self-test → exit 0\n"
             "- `pytest tests/ -q` PASS\n")
    chk("md-happy", MD_OK, EXIT_OK)
    # JSON 모드 — 문자열·dict 항목 혼합
    chk("json-happy", json.dumps({"task": "t", "verify": [
        "python3 x.py --self-test", {"cmd": "bash run.sh", "calibrated": True}]}), EXIT_OK)
    # ① hard: verify 부재
    chk("no-verify", "# task\n일해라\n", EXIT_HARD, "verify[] 부재")
    chk("empty-verify", "# task\nx\n# verify\n", EXIT_HARD, "verify[] 부재")
    # ① hard: 자기검토 문구 불인정
    chk("self-review", "# verify\n- 스스로 재검토하라\n", EXIT_HARD, "자기검토")
    chk("self-review-en", "# verify\n- double-check your work\n", EXIT_HARD, "자기검토")
    # ① hard: 비실행형(러너 토큰 없음)
    chk("prose-verify", "# verify\n- 결과가 좋아야 한다\n", EXIT_HARD, "실행형 아님")
    # ① A2 회귀: make·go 산문 우회 — 자연어 후속(sure/over)은 verify 불인정
    chk("prose-make", "# verify\n- make sure the results look good\n", EXIT_HARD, "실행형 아님")
    chk("prose-go", "# verify\n- go over the checklist carefully\n", EXIT_HARD, "실행형 아님")
    # ① A2: 명령 위치의 make·go(실인자·서브커맨드)는 실행형 인정 유지
    chk("make-cmd", "# verify\n- make test → exit 0\n", EXIT_OK)
    chk("go-cmd", "# verify\n- go test ./... → PASS\n", EXIT_OK)
    # ① 실행형 인정: ./경로·locked-eval ref
    chk("dot-slash", "# verify\n- ./run_tests.sh → exit 0\n", EXIT_OK)
    chk("locked-eval", "# verify\n- locked-eval ref: evals/T1.lock\n", EXIT_OK)
    # ② stretch 짝 — calibrated 없으면 WARN, 있으면 PASS
    chk("stretch-nocal", "# stretch\n최고 품질로\n# verify\n- python3 t.py\n",
        EXIT_WARN, "stretch")
    chk("stretch-cal", "# stretch\n상향\n# verify\n- [calibrated] python3 t.py → exit 0\n", EXIT_OK)
    chk("stretch-cal-json", json.dumps({"stretch": "상향", "verify": [
        {"cmd": "python3 t.py", "calibrated": True}]}), EXIT_OK)
    # ③ 마이크로매니징 — 명령형 마커 임계
    micro = "# verify\n- python3 t.py\n" + ("반드시 이렇게 해라. 절대 다르게 하지 마라.\n" * 8)
    chk("micromanage", micro, EXIT_WARN, "마이크로매니징")
    # ③ 번호 스텝 임계(스텝은 verify 밖 섹션 — 오귀속 없이 밀도만 측정)
    steps = ("# verify\n- python3 t.py\n# 절차\n"
             + "".join("%d. 스텝\n" % i for i in range(30)))
    chk("step-density", steps, EXIT_WARN, "마이크로매니징")
    # ④ 8KB — size_bytes 주입 경로
    code, findings = lint_text(MD_OK, size_bytes=BRIEF_SIZE_WARN_BYTES + 1)
    if code != EXIT_WARN or not any("8KB" in f for f in findings):
        failures.append("8kb: exit=%d findings=%s" % (code, findings))
    # 라벨 형식(헤딩 없이 'verify:' 라벨)
    chk("label-form", "task: 구현\nverify:\n- python3 t.py self-test\n", EXIT_OK)
    # 무관 헤딩 아래 리스트가 verify 로 오귀속되지 않음(섹션 이탈)
    chk("section-exit", "# verify\n- python3 t.py\n# 참고\n- 산문 항목\n", EXIT_OK)
    # hard + warn 혼재 → hard(2) 우선
    chk("hard-precedence", "# stretch\n상향\n# verify\n- 좋게 하라\n", EXIT_HARD)

    # 파일 경유 — 읽기 실패 = hard(2)
    import tempfile
    with tempfile.TemporaryDirectory(prefix="javis-brief-lint-st-") as td:
        good = os.path.join(td, "b.md")
        with open(good, "w", encoding="utf-8") as f:
            f.write(MD_OK)
        code, _ = lint_file(good)
        if code != EXIT_OK:
            failures.append("file-happy: exit=%d" % code)
        code, findings = lint_file(os.path.join(td, "nope.md"))
        if code != EXIT_HARD or not any("읽기 실패" in f for f in findings):
            failures.append("file-missing: exit=%d findings=%s" % (code, findings))

    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "cases": 24, "failures": failures}, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description="위임 브리프 결정론 린트 (0=통과 1=경고 2=hard)")
    ap.add_argument("--self-test", action="store_true", help="밀폐 자기검증")
    sub = ap.add_subparsers(dest="cmd")
    c = sub.add_parser("lint", help="브리프 1건 린트")
    c.add_argument("file")
    c.add_argument("--json", action="store_true")

    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.cmd == "lint":
        return cmd_lint(args.file, args.json)
    ap.print_help()
    return EXIT_WARN


if __name__ == "__main__":
    sys.exit(main())
