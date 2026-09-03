#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_review_prompt_verdict_path.py — review-prompt 가 verdict JSON **정본 경로·저장 지시**를 강제하는가 (0.14.30 A8).

무엇을 막는가(dept-1 실측 2026-09-04 · CEO 상신): reviewer-codex R3 verdict(7,581B)가 `_reviews/` 에
0건이었고 **유일한 완전본이 부서장 좌석 큐 안에만** 있었다 — 큐 배달 전에 좌석이 정리됐다면 판정
근거가 통째로 휘발됐을 것이다(화면 캡처는 pane 폭 때문에 1,533B 부분본). `round-log` 는 리뷰어 행에
`--verdict-json <파일>` 을 **요구**하는데 의뢰문은 그 파일을 어디에 쓰라고 말하지 않았다 — 요구와
지시가 갈린 자리다.

밀폐: `CYS_PACK_DIR`(레인)·`JAVIS_ROOT`(handoff 소스)를 임시 디렉터리로 덮어 라이브 팩·라이브
`_round` 무접촉. 서브프로세스는 review-prompt 생성뿐이고 파일 쓰기는 0이다(생성기는 순수 stdout).

  ① 생성문에 **저장 경로**가 실재(레인 팩 파생 · `<팩>/round/_reviews/<task>-r<N>-<evaluator>.json`)
  ② 저장 지시 토큰 실재(정본 저장 · 스키마 계약 · round-log 짝 · mkdir 안내)
  ③ 성공 기준에 **파일 실재**가 들어간다(파일 없으면 미제출)
  ④ 레인별 유일 — 서로 다른 CYS_PACK_DIR 은 서로 다른 경로를 내고 각자 자기 팩 아래다
  ⑤ (round·evaluator)별 유일 — r1↔r3, codex↔gemini 가 서로 다른 파일명
  ⑥ 미해소 플레이스홀더 0(`<task>`·`<N>`·`<evaluator>`) · `--reviewer` 미지정도 **구체 경로**
  ⑦ round-log 요구와 같은 파일을 가리킨다(짝 결속 — 소스에서 요구 문구 실재 확인)
  ⑧ 경로 탈출 0 — task 에 `../`·`/` 가 있어도 basename 에 분리자가 남지 않는다
  ⑨ ★음성 대조(계측 타당성) — 주입 블록을 제거한 **변조 사본**은 위 토큰·경로를 내지 않는다
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REVIEW-PROMPT-VERDICT-PATH-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_review_prompt_verdict_path.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
ORC = os.path.join(BIN, "javis_orchestra.py")
PY = sys.executable or "python3"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def run_rp(pack, task="R3 게이트", reviewer="codex", rnd=3, script=None):
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = pack
    for k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        env.pop(k, None)
    env["JAVIS_ROOT"] = pack                     # handoff·불변식 소스 격리(라이브 _round 무접촉)
    env["PYTHONPATH"] = BIN + os.pathsep + env.get("PYTHONPATH", "")
    cmd = [PY, script or ORC, "review-prompt", "--task", task, "--scope", "bin/x.py",
           "--round", str(rnd)]
    if reviewer:
        cmd += ["--reviewer", reviewer]
    return subprocess.run(cmd, capture_output=True, text=True, timeout=90, env=env)


root = tempfile.mkdtemp()
try:
    pack_a = os.path.join(root, "pack-a")
    pack_b = os.path.join(root, "pack-dept-sales")
    os.makedirs(pack_a); os.makedirs(pack_b)

    r = run_rp(pack_a)
    out = r.stdout
    check("0 생성 성공", r.returncode == 0, repr(r.stderr[-200:]))
    expect = os.path.join(pack_a, "round", "_reviews", "R3_게이트-r3-codex.json")
    check("1 저장 경로 실재(레인 팩 파생)", expect in out, repr(expect))
    for tok, label in (("verdict 정본 저장", "정본 저장 지시"),
                       ("REVIEWER_VERDICT_CONTRACT", "스키마 계약"),
                       ("--verdict-json", "round-log 짝"),
                       ("mkdir -p", "디렉터리 안내")):
        check("2 저장 지시 토큰: %s" % label, tok in out, tok)
    check("3 성공 기준에 파일 실재",
          "완료 기준(파일 실재)" in out and "미제출" in out, repr(out[-400:]))

    rb = run_rp(pack_b)
    expect_b = os.path.join(pack_b, "round", "_reviews", "R3_게이트-r3-codex.json")
    check("4 레인별 유일", expect_b in rb.stdout and expect not in rb.stdout, repr(expect_b))

    r1 = run_rp(pack_a, rnd=1)
    rg = run_rp(pack_a, reviewer="gemini")
    check("5a 라운드별 유일",
          os.path.join(pack_a, "round", "_reviews", "R3_게이트-r1-codex.json") in r1.stdout
          and expect not in r1.stdout)
    check("5b 평가자별 유일",
          os.path.join(pack_a, "round", "_reviews", "R3_게이트-r3-gemini.json") in rg.stdout
          and expect not in rg.stdout)

    rn = run_rp(pack_a, reviewer=None)
    check("6a --reviewer 미지정도 구체 경로(앵커 역할로 접음)",
          os.path.join(pack_a, "round", "_reviews", "R3_게이트-r3-reviewer1.json") in rn.stdout,
          repr(rn.stdout[-300:]))
    check("6b 미해소 플레이스홀더 0",
          not any(t in out for t in ("<task>", "<N>", "<evaluator>")), repr(out[-300:]))

    with open(ORC, encoding="utf-8") as f:
        src = f.read()
    check("7 round-log 가 리뷰어 행에 --verdict-json 을 요구한다(짝 결속)",
          "--verdict-json <파일> 필수" in src)

    revil = run_rp(pack_a, task="../../etc/passwd")
    line = [l for l in revil.stdout.splitlines() if "_reviews" in l]
    check("8a 경로 탈출 0(_reviews 밖으로 못 나간다)",
          bool(line) and all(os.path.dirname(l.strip()).endswith(os.path.join("round", "_reviews"))
                             for l in line if l.strip().endswith(".json")),
          repr(line))
    check("8b basename 에 분리자 없음",
          bool(line) and all("/" not in os.path.basename(l.strip())
                             for l in line if l.strip().endswith(".json")), repr(line))

    # ⑨ ★음성 대조 — 주입 블록을 제거한 사본은 아무 토큰도 내지 않는다
    lines = src.split("\n")
    try:
        i0 = next(i for i, l in enumerate(lines) if l.strip().startswith("# ★A8("))
        i1 = next(i for i, l in enumerate(lines) if i > i0 and 'lines.append("회신:' in l)
    except StopIteration:
        check("9 변조 앵커 실재", False, "A8 블록 앵커를 못 찾았다(핀 무효)")
    else:
        mut = os.path.join(root, "javis_orchestra.py")
        with open(mut, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines[:i0] + lines[i1:]))
        rm = run_rp(pack_a, script=mut)
        check("9 변조본은 저장 지시·경로 0(계측 타당성)",
              rm.returncode == 0 and "verdict 정본 저장" not in rm.stdout
              and "_reviews" not in rm.stdout,
              repr((rm.returncode, rm.stdout[-200:], rm.stderr[-200:])))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("REVIEW-PROMPT-VERDICT-PATH-OK")
sys.exit(0)
