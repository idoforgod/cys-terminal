#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_memory_desc_drift.py — 색인 훅↔description 대조 축과 원자적 update 핀 (0.14.30 A7).

무엇을 막는가(dept-1 실측 + 본부 재현 2026-09-04): `verify` 가 '색인↔파일 정합'을 자칭하면서
**색인 줄의 훅 문장과 파일 frontmatter 의 description** 은 대조하지 않았고, 도구에 수정 명령이
없어 갱신은 늘 손편집이었다 — 한쪽만 고쳐도 아무도 잡지 못했다. 색인은 **매 세션 전 노드에
자동 주입**되므로 색인이 철회된 처방을 계속 전파한다(dept-1: 60건 중 5건 드리프트 · 2건은 방향
정반대).

밀폐: 전부 임시 memory 디렉터리에서 돈다 — **라이브 `~/.cys/pack/memory` 무접촉**(읽지도 않는다).

  ① 정합 기준선 — add 직후 드리프트 0
  ② 색인만 변조 → desc_drift 검출(파일은 그대로)
  ③ 파일 description 만 변조 → desc_drift 검출(색인은 그대로)
  ④ **요약형 색인**(훅 ⊂ description)은 정합 — 현행 관행을 깨지 않는다(과잉 차단 0)
  ⑤ 표기 차이(마크다운 강조·전각·제로폭·공백·양끝 구두점)는 드리프트가 아니다(정규화 축)
  ⑥ **극성 반전**(미부여↔부여)은 containment 를 통과해도 desc_polarity 로 잡는다
  ⑦ ★exit 코드 계약 — 기본 verify 는 드리프트가 있어도 **rc 0**(preflight C18 이 이 코드를
     소비한다). `--desc-strict` 로 옵트인할 때만 problems 에 합류해 rc 1.
  ⑧ update — 파일 description·본문과 색인 훅이 **한 번에** 갱신되고 이후 드리프트 0
  ⑨ update 거부 — 미등재 슬러그·중복 슬러그·빈 값·필드 미지정(전부 exit 2)
  ⑩ ★중간 실패 주입 — 색인 쓰기가 실패하면 **파일이 원상 복구**된다(부분 갱신 0)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 MEMORY-DESC-DRIFT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_memory_desc_drift.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
MEM = os.path.join(BIN, "javis_memory.py")
PY = sys.executable or "python3"
sys.path.insert(0, BIN)
import javis_memory as M  # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def run(mdir, *argv):
    r = subprocess.run([PY, MEM, "--dir", mdir] + list(argv),
                       capture_output=True, text=True, timeout=60)
    return r


def seed(mdir, name="alpha-rule", desc="알파 규칙 — 기본 미부여", body="본문."):
    os.makedirs(mdir, exist_ok=True)
    open(os.path.join(mdir, "MEMORY.md"), "w", encoding="utf-8").write(
        "# MEMORY.md — 테스트 골격\n\n```markdown\n- [예시](type_example.md) — 코드펜스 예시\n```\n\n")
    r = run(mdir, "add", "--type", "feedback", "--name", name, "--desc", desc, "--body", body)
    assert r.returncode == 0, r.stderr
    return os.path.join(mdir, "feedback_%s.md" % name)


def desc_problems(mdir):
    r = run(mdir, "verify", "--json")
    return json.loads(r.stdout)["desc_problems"], r.returncode


def write(p, t):
    open(p, "w", encoding="utf-8", newline="\n").write(t)


def read(p):
    return open(p, encoding="utf-8").read()


root = tempfile.mkdtemp()
try:
    # ① 기준선
    m1 = os.path.join(root, "m1")
    f1 = seed(m1)
    dp, rc = desc_problems(m1)
    check("1 add 직후 드리프트 0", dp == [] and rc == 0, repr((dp, rc)))

    # ② 색인만 변조
    idx = os.path.join(m1, "MEMORY.md")
    write(idx, read(idx).replace("알파 규칙 — 기본 미부여", "전혀 다른 문장이 색인에만 남았다"))
    dp, rc = desc_problems(m1)
    check("2 색인만 변조 → desc_drift", len(dp) == 1 and "desc_drift" in dp[0], repr(dp))
    check("2b 기본 verify 는 여전히 rc 0(보고용)", rc == 0, str(rc))

    # ③ 파일 description 만 변조
    m2 = os.path.join(root, "m2")
    f2 = seed(m2)
    write(f2, read(f2).replace("description: 알파 규칙 — 기본 미부여",
                               "description: 파일에만 남은 다른 서술"))
    dp, _ = desc_problems(m2)
    check("3 파일만 변조 → desc_drift", len(dp) == 1 and "desc_drift" in dp[0], repr(dp))

    # ④ 요약형 색인(훅 ⊂ description) = 정합
    m3 = os.path.join(root, "m3")
    f3 = seed(m3, desc="알파 규칙 — 기본 미부여이며 오너가 직접 부여할 때만 발효한다")
    idx3 = os.path.join(m3, "MEMORY.md")
    write(idx3, read(idx3).replace("알파 규칙 — 기본 미부여이며 오너가 직접 부여할 때만 발효한다",
                                   "알파 규칙 — 기본 미부여"))
    dp, _ = desc_problems(m3)
    check("4 요약형 색인은 정합(과잉 차단 0)", dp == [], repr(dp))

    # ⑤ 표기 차이(강조·제로폭·공백·양끝 구두점)는 드리프트가 아니다
    m4 = os.path.join(root, "m4")
    f4 = seed(m4, desc="알파 규칙 — 기본 미부여")
    idx4 = os.path.join(m4, "MEMORY.md")
    write(idx4, read(idx4).replace("알파 규칙 — 기본 미부여",
                                   "**알파 규칙**  —  기본​ 미부여."))
    dp, _ = desc_problems(m4)
    check("5 표기 차이는 드리프트 아님(정규화)", dp == [], repr(dp))

    # ⑥ 극성 축 — containment 를 **통과하는데도** 방향이 갈리는 경우.
    #    (색인이 파일에 없는 방향 주장을 덧붙인 형상 — dept-1 의 '정본 2건이 방향 정반대'가 이 축이다.
    #     containment 자체가 깨지는 반전은 ②③ 이 desc_drift 로 이미 잡는다.)
    m5 = os.path.join(root, "m5")
    f5 = seed(m5, desc="자율 진행 권한 기본 미부여")
    idx5 = os.path.join(m5, "MEMORY.md")
    write(idx5, read(idx5).replace("자율 진행 권한 기본 미부여",
                                   "자율 진행 권한 기본 미부여 — 오너 승인 시 허용"))
    dp, _ = desc_problems(m5)
    check("6a containment 통과 + 방향 상이 → desc_polarity",
          len(dp) == 1 and "desc_polarity" in dp[0], repr(dp))
    # 음성 대조: 방향 토큰이 같으면(덧붙은 말이 극성이 아니면) 정합이다 — 과잉 차단 0
    m5b = os.path.join(root, "m5b")
    f5b = seed(m5b, desc="자율 진행 권한 기본 미부여")
    idx5b = os.path.join(m5b, "MEMORY.md")
    write(idx5b, read(idx5b).replace("자율 진행 권한 기본 미부여",
                                     "자율 진행 권한 기본 미부여 — 3축 계약"))
    dp, _ = desc_problems(m5b)
    check("6b 극성이 같으면 요약 확장도 정합", dp == [], repr(dp))

    # ⑦ exit 코드 계약
    r_default = run(m1, "verify")
    r_strict = run(m1, "verify", "--desc-strict")
    r_report = run(m1, "verify", "--desc-report-only")
    check("7a 기본 verify rc 0(preflight C18 무영향)", r_default.returncode == 0,
          repr(r_default.stdout[-200:]))
    check("7b --desc-strict 는 rc 1", r_strict.returncode == 1, repr(r_strict.stdout[-200:]))
    check("7c --desc-report-only 는 기본과 같다", r_report.returncode == 0)
    check("7d 기본 출력에 [DESC] 보고행", "[DESC]" in r_default.stdout, repr(r_default.stdout[:200]))

    # ⑧ update — 파일 + 색인 동시 갱신
    m6 = os.path.join(root, "m6")
    f6 = seed(m6, name="beta-rule", desc="원래 설명")
    ru = run(m6, "update", "--name", "beta-rule", "--desc", "새 설명 — 갱신됨",
             "--body", "새 본문")
    check("8a update rc 0", ru.returncode == 0, repr((ru.stdout, ru.stderr[-200:])))
    body6, idx6 = read(f6), read(os.path.join(m6, "MEMORY.md"))
    check("8b 파일 description 갱신", "description: 새 설명 — 갱신됨" in body6, repr(body6[:200]))
    check("8c 파일 본문 갱신", "새 본문" in body6 and "원래 설명" not in body6)
    check("8d 색인 훅 갱신", "새 설명 — 갱신됨" in idx6 and "원래 설명" not in idx6, repr(idx6[-200:]))
    dp, rc6 = desc_problems(m6)
    check("8e update 후 드리프트 0", dp == [] and rc6 == 0, repr(dp))
    check("8f 코드펜스 예시는 무접촉", "- [예시](type_example.md) — 코드펜스 예시" in idx6)

    # ⑨ 거부 경로
    check("9a 미등재 슬러그 거부", run(m6, "update", "--name", "없는-슬러그",
                                       "--desc", "x").returncode == 2)
    check("9b 필드 미지정 거부", run(m6, "update", "--name", "beta-rule").returncode == 2)
    check("9c 빈 값 거부", run(m6, "update", "--name", "beta-rule", "--desc", "   ").returncode == 2)
    m7 = os.path.join(root, "m7")
    f7 = seed(m7, name="dup-rule", desc="설명")
    shutil.copy(f7, os.path.join(m7, "project_dup-rule.md"))   # 같은 name 을 가진 두 파일
    check("9d 중복 슬러그 거부", run(m7, "update", "--name", "dup-rule",
                                     "--desc", "x").returncode == 2)

    # ⑩ ★중간 실패 주입 — 색인 쓰기 실패 시 파일 원상 복구
    m8 = os.path.join(root, "m8")
    f8 = seed(m8, name="rollback-rule", desc="원본 설명", body="원본 본문")
    before_file, before_index = read(f8), read(os.path.join(m8, "MEMORY.md"))
    real_write = M._write_text_atomic

    def boom(path, text):
        if os.path.basename(path) == M.INDEX_FILE:
            raise OSError("주입된 색인 쓰기 실패")
        return real_write(path, text)

    class _Args:
        name, desc, body = "rollback-rule", "덮어써질 설명", "덮어써질 본문"

    M._write_text_atomic = boom
    try:
        rc10 = M.cmd_update(m8, _Args())
    finally:
        M._write_text_atomic = real_write
    check("10a 실패는 exit 3", rc10 == 3, str(rc10))
    check("10b 파일이 원상 복구", read(f8) == before_file, repr(read(f8)[:160]))
    check("10c 색인 무변경", read(os.path.join(m8, "MEMORY.md")) == before_index)
    dp, rc8 = desc_problems(m8)
    check("10d 롤백 후 정합", dp == [] and rc8 == 0, repr(dp))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("MEMORY-DESC-DRIFT-OK")
sys.exit(0)
