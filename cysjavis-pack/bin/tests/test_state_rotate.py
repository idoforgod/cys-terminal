#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_state_rotate.py — 상태 파일 회전기 회귀 (결재 10).

무엇을 막는가: 손 이관이 만든 **사본-원본 경쟁 상태**(2026-09-17 20:08 · 사본 64,666B 대 원본 64,886B).
회전기가 ①사본 무결 ②자르기 직전 재확인(경쟁 시 무변경 exit 9) ③축소 보장 ④머리글·체크박스 보존을
지키는지 고정하고, 회전 결과가 **상한 게이트(CSO_TODO_CAP 64KiB) 아래**로 내려오는지 함께 본다.
"""
import os, subprocess, sys, tempfile

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOD = os.path.join(BIN, "javis_state_rotate.py")
sys.path.insert(0, BIN)
import javis_state_rotate as R  # noqa: E402

CAP = 64 * 1024
fails = []


def ck(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


r = subprocess.run([sys.executable, MOD, "--self-test"], capture_output=True, text=True)
ck("모듈 self-test 8종 통과", r.returncode == 0, r.stdout[-200:])

d = tempfile.mkdtemp(prefix="rotgate-")
p = os.path.join(d, "CSO_TODO.md")
head = "<!-- javis:todo v1 owner=cso -->\n# CSO_TODO\n- [ ] 살아있는 항목\n"
open(p, "w", encoding="utf-8").write(head + "".join("기록 줄 %d %s\n" % (i, "가" * 60) for i in range(900)))
before = os.path.getsize(p)
ck("픽스처가 상한을 넘는다(회전 대상)", before > CAP, "%dB" % before)
rc, msg, info = R.rotate(p, target=48 * 1024)
after = os.path.getsize(p)
ck("회전 exit 0", rc == 0, msg)
ck("★회전 결과가 상한 게이트 아래(64KiB)", after < CAP, "%dB → %dB" % (before, after))
ck("★체크박스 보존(진행% 집계 불변)", "- [ ] 살아있는 항목" in open(p, encoding="utf-8").read())
ck("사본이 원본 전체 바이트 보존", os.path.getsize(info["archive"]) == before)
ck("재실행은 회전 불요(exit 2 · 멱등)", R.rotate(p, target=48 * 1024)[0] == 2)

print("\n=== %d/%d PASS ===" % (7 - len(fails), 7))
sys.exit(1 if fails else 0)
