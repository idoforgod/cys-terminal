#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_intake_exit_contract.py — javis_intake.py(접수-지불 대장) exit 계약 회귀 (B-328 #8 · 2026-10-20).

## 무엇을 못박는가
이 도구의 사실은 exit code 다(도구 머리말 【결정론 계약】). 실제 CLI 를 서브프로세스로 돌려 각 갈래를 한 번씩 낸다.
  E0 PAID      open → pay(내구 artifact + 증거 40자) → status = 0 · 정당한 defer = 0 · audit 은 모든 대장이 지불일 때만 0
  E1 OPEN      항목이 열려 있으면 1 · 짧은 증거·부재 artifact·임시 경로만인 pay·조건 없는 defer 는 REFUSED = 1
               · 지불 뒤 증거 바이트가 바뀌면(덧붙임 포함) 다시 1 · 지불된 대장에 add-item 하면 다시 1
               · 정규 파일의 지문을 얻지 못한 새 지불(디렉터리·읽기 불가·FIFO·지불 시점 부재)은 REFUSED = 1 이고
                 그 자리에 뒤늦게 파일이 생겨도 1 — 지문 면제는 지문 칸 이전의 맨 문자열 기록뿐이다(그쪽은 EXEMPT 10)
  E2 NO_DENOM  항목 0건 대장 = 2 — ★통과(0)가 아니다
  E3 NO_LEDGER 빈 팩·없는 --id·대상 부재 pay/add-item·빈 루트 audit·다른 좌석의 대장만 있음 = 3
               · 좌석 권위(스텁 cys · POSIX): identify 가 env 를 이긴다 · 재번호된 같은 역할은 자기 대장(0)
  E4 DRIFT     기록된 지시 원문을 사후 변조 = 4 · 판독 불가 대장 = 4
  E5 USAGE     사용법 오류는 argparse 기본 2 가 아니라 5 — 2(NO_DENOM)와 같은 숫자가 두 사실을 뜻하지 않는다
  E7 UNATTRIB  좌석 귀속이 없는 대장은 좌석 범위 status 에서 판정하지 않는다 = 7 · stamp 의 0 은 명령 성공일 뿐(그 뒤 status 1)
  E8 UNREADABLE ★있는데 읽히지 않는 artifact(권한 000 · 실측 기제는 macOS dataless)는 「바뀌었다」가 아니다 = 8 —
               errno 와 (darwin 이면) st_flags 를 찍고 「지불은 무효가 아니다」를 말하며 ⛔재지불 처방을 찍지 않는다.
               크기 일치는 정황일 뿐 신원이 아니다(읽을 수 있는 sha 가 필요하다). 열린 항목 1건은 판독 불가 전부를 이긴다(1 > 8).
  S  selftest  = 0 · 자기 재기동 argv 에 -I 와 함께 -B (번들 python 이 캐시를 번들 안에 쓰지 않게)
  W  Windows   임시 루트(TMPDIR·TEMP·TMP·LOCALAPPDATA\\Temp · 대소문자 무시 · DOS 장치 경로 \\\\?\\ \\\\.\\ 접두 벗김)
               — 도구 모듈의 os 를 대역으로 바꿔 이 Mac 에서도 Windows 판정을 잰다(%TEMP% 만인 artifact 는 지불이 아니다)

## 밀폐
- 팩·HOME·cwd 는 케이스마다 새 임시 디렉터리다. 라이브 팩(~/.cys/pack*)은 읽지도 쓰지도 않는다.
- 체크아웃에는 쓰지 않는다(읽기 전용 체크아웃에서도 돈다). 내구 artifact 가 필요한 갈래는 DurableRig 가 리그 안의
  work/ 를 쓰고, 자식 안에서만 임시 루트를 리그의 eph/ 로 바꿔 work/ 를 내구 자리로 읽게 한다.
- 자식 PATH 는 빈 디렉터리다 ⇒ 도구의 `cys identify`·`cys status --json` 호출은 실행 파일을 못 찾고(도구가
  예외를 삼킨다) **라이브 데몬에 닿지 않는다**. 좌석 귀속은 env(CYS_ROLE·CYS_SURFACE_ID — 도구의 비권위
  폴백 경로)로 준다. 부모의 CLAUDE_CONFIG_DIR·CYS_* 는 자식에 넘기지 않는다.
  E3SeatAuthority 만 예외로 PATH 에 스텁 cys(임시 폴더의 sh 스크립트 · 정해 둔 JSON 만 출력)를 둔다.
- PYTHONUTF8=1 — 운영 pane 이 모든 좌석에 주입하는 값(src/bin/cysd/state.rs pane 스폰 · src/lib.rs
  spawn_env_pairs)을 그대로 따른다.

## 한계
- Windows 실측은 없다. W 는 대역 os 로 Windows 판정 논리를 재지만 Windows 위에서 돈 적은 없다.
  스텁 cys(sh 스크립트) 갈래와 POSIX 규칙 고정 케이스는 Windows 에서 건너뛰고, 건너뜀은 사유와 함께 찍힌다.
- W·S 의 일부는 도구를 같은 프로세스로 읽는다 — 그때 캐시(.pyc)를 체크아웃에 남기지 않는다(load_tool).

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_intake_exit_contract.py
"""
import contextlib
import importlib.util
import io
import json
import ntpath
import os
import shutil
import stat
import copy
import hashlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
TOOL = os.path.join(BIN, "javis_intake.py")
# 내구 artifact — 체크아웃 안의 이 파일 자신(임시 경로가 아니다).
DURABLE = os.path.abspath(__file__)

EVIDENCE = "검증 로그를 디스크에 남겼고 기대값과 실측값이 일치함을 확인했다 — exit 0 실측"
REASON = "상위 결정(헤드 판정)이 아직 내려지지 않아 착수할 수 없다"
SEAT_A = {"CYS_ROLE": "worker-a", "CYS_SURFACE_ID": "901"}
SEAT_B = {"CYS_ROLE": "worker-b", "CYS_SURFACE_ID": "902"}

EXIT_PAID, EXIT_OPEN, EXIT_NO_DENOM, EXIT_NO_LEDGER, EXIT_DRIFT, EXIT_USAGE = 0, 1, 2, 3, 4, 5
EXIT_UNATTRIB = 7
EXIT_UNREADABLE = 8


class Rig(unittest.TestCase):
    """임시 팩 1개 + 빈 PATH + 좌석 env — 케이스마다 새로 만든다."""

    def setUp(self):
        self.assertTrue(os.path.isfile(TOOL), "도구 부재: %s" % TOOL)
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="intake-exit-", dir=os.environ.get("P5N_FIXTURE_ROOT")))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.pack = os.path.join(self.tmp, "pack")
        self.home = os.path.join(self.tmp, "home")
        self.nobin = os.path.join(self.tmp, "nobin")
        for d in (self.pack, self.home, self.nobin):
            os.makedirs(d)
        # 밀폐 전제: 자식 PATH 로는 cys 를 찾을 수 없다.
        self.assertIsNone(shutil.which("cys", path=self.nobin))
        self.instr = os.path.join(self.tmp, "instruction.txt")
        with open(self.instr, "w", encoding="utf-8") as fh:
            fh.write("오너 지시 원문 — 두 항목을 하나하나 끝내라.\n")

    def env(self, seat):
        e = {k: v for k, v in os.environ.items()
             if not k.startswith("CYS_") and k != "CLAUDE_CONFIG_DIR"}
        e.update({"CYS_PACK_DIR": self.pack, "HOME": self.home, "PATH": self.nobin,
                  "PYTHONUTF8": "1"})
        e.update(seat or {})
        return e

    def run_tool(self, *argv, seat=SEAT_A):
        return subprocess.run([sys.executable, "-I", TOOL] + list(argv), cwd=self.tmp,
                              env=self.env(seat), capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=60)

    def rc(self, want, r, what):
        self.assertEqual(r.returncode, want, "%s: rc=%d (기대 %d)\n--stdout--\n%s\n--stderr--\n%s"
                         % (what, r.returncode, want, r.stdout, r.stderr))

    def ledger_path(self, lid):
        return os.path.join(self.pack, "round", "_intake", "%s.json" % lid)

    def load_ledger(self, lid):
        with open(self.ledger_path(lid), encoding="utf-8") as fh:
            return json.load(fh)

    def open_ledger(self, lid, items, seat=SEAT_A):
        argv = ["open", "--id", lid, "--instruction-file", self.instr]
        for it in items:
            argv += ["--item", it]
        return self.run_tool(*argv, seat=seat)


class E0Paid(Rig):
    def test_pay_with_durable_artifact_is_paid(self):
        self.rc(EXIT_OPEN, self.open_ledger("L0", ["항목 하나"]), "open(항목 1)")
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "L0", "--item-no", "1",
                                         "--evidence", EVIDENCE, "--artifact", DURABLE), "pay")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "L0"), "status --id")
        self.rc(EXIT_PAID, self.run_tool("status"), "status(좌석 범위)")
        r = self.run_tool("status", "--json")
        self.rc(EXIT_PAID, r, "status --json")
        self.assertEqual(json.loads(r.stdout)["worst_exit"], EXIT_PAID)
        led = self.load_ledger("L0")
        # 밀폐 증인: cys 를 못 찾았으므로 귀속은 env 값이다(라이브 데몬이 답했다면 다른 surface).
        self.assertEqual(led["opened_by"]["surface"], "901")
        self.assertEqual(led["opened_by"]["role"], "worker-a")

    def test_defer_with_release_condition_is_paid(self):
        self.open_ledger("D0", ["보류할 항목"])
        self.rc(EXIT_PAID, self.run_tool("defer", "--id", "D0", "--item-no", "1", "--reason", REASON,
                                         "--release-condition", "헤드 판정 도착"), "defer")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "D0"), "status --id")

    def test_audit_is_0_only_when_every_ledger_is_paid(self):
        # audit 의 0 = 모든 루트의 모든 대장이 지불됐다(머리말 exit 계약) — 열린 대장이 하나라도 있으면 1.
        self.open_ledger("A0", ["항목"])
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "A0", "--item-no", "1", "--evidence", EVIDENCE,
                                         "--artifact", DURABLE), "pay")
        self.rc(EXIT_PAID, self.run_tool("audit", "--roots", self.pack), "audit(전부 지불)")
        self.open_ledger("A1", ["열린 항목"])
        self.rc(EXIT_OPEN, self.run_tool("audit", "--roots", self.pack), "audit(열린 대장 1)")


class E1Open(Rig):
    def test_open_item_is_open_and_listed(self):
        self.rc(EXIT_OPEN, self.open_ledger("L1", ["지불 전 항목", "두번째 미지불"]), "open(항목 2)")
        r = self.run_tool("status", "--id", "L1")
        self.rc(EXIT_OPEN, r, "status --id")
        self.assertIn("두번째 미지불", r.stdout, "열린 항목 본문이 출력되지 않았다")

    def test_one_open_among_paid_is_open(self):
        self.open_ledger("L1b", ["지불할 항목", "남는 항목"])
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "L1b", "--item-no", "1",
                                         "--evidence", EVIDENCE, "--artifact", DURABLE), "pay #1")
        self.rc(EXIT_OPEN, self.run_tool("status", "--id", "L1b"), "status(1 지불 · 1 열림)")

    def test_short_evidence_is_refused_and_not_recorded(self):
        self.open_ledger("L1c", ["항목"])
        before = self.load_ledger("L1c")
        r = self.run_tool("pay", "--id", "L1c", "--item-no", "1", "--evidence", "했다",
                          "--artifact", DURABLE)
        self.rc(EXIT_OPEN, r, "pay(증거 2자)")
        self.assertIn("REFUSED", r.stderr)
        self.assertIn("evidence 2자", r.stderr)
        led = self.load_ledger("L1c")
        self.assertEqual(led, before, "거부는 대장을 쓰지 않는다")
        self.rc(EXIT_OPEN, self.run_tool("status", "--id", "L1c"), "status(거부 뒤)")

    def test_missing_artifact_is_refused(self):
        self.open_ledger("L1d", ["항목"])
        # 내구 자리(체크아웃 안)의 부재 경로 — 임시 경로였다면 '임시 경로만' 사유로 거부돼 부재 판정을 재지 못한다.
        gone = os.path.join(TESTS_DIR, "no-such-artifact-intake.log")
        self.assertFalse(os.path.exists(gone))
        r = self.run_tool("pay", "--id", "L1d", "--item-no", "1", "--evidence", EVIDENCE,
                          "--artifact", gone)
        self.rc(EXIT_OPEN, r, "pay(부재)")
        self.assertIn("디스크 부재", r.stderr)

    def test_ephemeral_only_artifact_is_refused(self):
        self.open_ledger("L1e", ["항목"])
        fd, tmpart = tempfile.mkstemp(prefix="intake-eph-", dir=tempfile.gettempdir())
        os.close(fd)
        self.addCleanup(os.unlink, tmpart)
        r = self.run_tool("pay", "--id", "L1e", "--item-no", "1", "--evidence", EVIDENCE,
                          "--artifact", tmpart)
        self.rc(EXIT_OPEN, r, "pay(임시 경로만)")
        self.assertIn("임시 경로", r.stderr)

    def test_add_item_reopens_a_paid_ledger(self):
        self.open_ledger("L1h", ["항목"])
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "L1h", "--item-no", "1", "--evidence", EVIDENCE,
                                         "--artifact", DURABLE), "pay")
        self.rc(EXIT_PAID, self.run_tool("add-item", "--id", "L1h", "--item", "뒤늦게 받은 항목"), "add-item")
        r = self.run_tool("status", "--id", "L1h")
        self.rc(EXIT_OPEN, r, "status(항목 추가 뒤)")
        self.assertIn("뒤늦게 받은 항목", r.stdout)

    def test_defer_without_release_condition_is_refused(self):
        self.open_ledger("L1f", ["항목"])
        r = self.run_tool("defer", "--id", "L1f", "--item-no", "1", "--reason", REASON,
                          "--release-condition", "")
        self.rc(EXIT_OPEN, r, "defer(조건 공백)")
        self.assertIn("release_condition", r.stderr)


# 임시 루트 주입 실행기 — 자식 안에서 도구를 읽어 _EPHEMERAL_ROOTS 만 argv[2] 로 바꾸고 main 을 돈다(-B: 체크아웃에 캐시 없음).
LAUNCH = ("import importlib.util,sys;"
          "s=importlib.util.spec_from_file_location('javis_intake',sys.argv[1]);"
          "m=importlib.util.module_from_spec(s);s.loader.exec_module(m);"
          "m._EPHEMERAL_ROOTS=(sys.argv[2],);"
          "sys.exit(m.main(sys.argv[3:]))")


class DurableRig(Rig):
    """리그 폴더 안의 work/ 를 내구 자리로 읽게 임시 루트를 주입한다 — 내구 artifact 갈래를 체크아웃에 쓰지 않고 잰다
    (읽기 전용 체크아웃에서도 돈다). 주입은 자식 안에서만 일어나고 분류 함수(_ephemeral)는 진짜다: POSIX 는 도구의
    _EPHEMERAL_ROOTS 를, Windows 는 임시 루트 env 를 리그 안의 eph/ 로 바꾼다. eph/ 가 여전히 임시로 읽히는지는
    test_injected_root_is_still_ephemeral 이 잰다(주입이 검사를 끄지 않았다는 양성 통제)."""

    def setUp(self):
        super().setUp()
        self.eph = os.path.join(self.tmp, "eph")
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.eph)
        os.makedirs(self.work)

    def env(self, seat):
        e = super().env(seat)
        if os.name == "nt":
            e.update({"TMPDIR": self.eph, "TEMP": self.eph, "TMP": self.eph,
                      "LOCALAPPDATA": os.path.join(self.tmp, "lad")})
        return e

    def run_tool(self, *argv, seat=SEAT_A, timeout=60):
        return subprocess.run([sys.executable, "-I", "-B", "-c", LAUNCH, TOOL, self.eph + os.sep] + list(argv),
                              cwd=self.tmp, env=self.env(seat), capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=timeout)

    def put(self, name, text="검증 로그 원본\n"):
        p = os.path.join(self.work, name)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(text)
        return p

    def pay(self, lid, *arts, timeout=60):
        argv = ["pay", "--id", lid, "--item-no", "1", "--evidence", EVIDENCE]
        for a in arts:
            argv += ["--artifact", a]
        return self.run_tool(*argv, timeout=timeout)

    def test_injected_root_is_still_ephemeral(self):
        art = os.path.join(self.eph, "proof.log")
        with open(art, "w", encoding="utf-8") as fh:
            fh.write("x\n")
        self.open_ledger("R0", ["항목"])
        r = self.pay("R0", art)
        self.rc(EXIT_OPEN, r, "pay(주입한 임시 루트 안)")
        self.assertIn("임시 경로", r.stderr)

    def test_regular_file_in_work_is_paid(self):
        self.open_ledger("R1", ["항목"])
        self.rc(EXIT_PAID, self.pay("R1", self.put("proof.log")), "pay(work 의 정규 파일)")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "R1"), "status")

    def test_changed_artifact_reopens(self):
        # 지불 뒤 증거 바이트가 바뀌면(덧붙임 포함) 다시 열린다 — 지문 대조.
        art = self.put("proof.log")
        self.open_ledger("L1g", ["항목"])
        self.rc(EXIT_PAID, self.pay("L1g", art), "pay")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "L1g"), "status(변경 전)")
        with open(art, "a", encoding="utf-8") as fh:
            fh.write("덧붙인 줄\n")
        r = self.run_tool("status", "--id", "L1g")
        self.rc(EXIT_OPEN, r, "status(증거 덧붙임 뒤)")
        self.assertIn("증거 바이트가 변경됐다", r.stdout)

    # ★지문 없는 새 지불은 지불이 아니다 — 지문 칸이 생긴 뒤의 기록(dict)이 정규 파일의 지문을 얻지 못했으면(디렉터리·
    #   읽기 불가·FIFO·지불 시점 부재) 그 artifact 는 지불을 싣지 않는다. 지문 면제는 그 전의 맨 문자열 기록뿐이다.
    def test_new_payment_with_directory_artifact_is_refused(self):
        d = os.path.join(self.work, "a-directory")
        os.makedirs(d)
        self.open_ledger("F1", ["항목"])
        r = self.pay("F1", d)
        self.rc(EXIT_OPEN, r, "pay(디렉터리)")
        self.assertIn("지문을 얻지 못했다", r.stderr)
        self.rc(EXIT_OPEN, self.run_tool("status", "--id", "F1"), "status(디렉터리 지불 뒤)")

    @unittest.skipIf(os.name == "nt", "chmod 000 은 Windows 에서 읽기를 막지 않는다")
    def test_new_payment_with_unreadable_file_is_refused(self):
        art = self.put("locked.log")
        os.chmod(art, 0)
        self.addCleanup(os.chmod, art, 0o600)
        if os.access(art, os.R_OK):
            self.skipTest("이 계정은 권한 000 파일을 읽는다(root 등) — 읽기 불가를 만들 수 없다")
        self.open_ledger("F2", ["항목"])
        r = self.pay("F2", art)
        self.rc(EXIT_OPEN, r, "pay(읽기 불가 파일)")
        self.assertIn("지문을 얻지 못했다", r.stderr)

    @unittest.skipIf(not hasattr(os, "mkfifo"), "FIFO 가 없는 OS")
    def test_new_payment_with_fifo_is_refused_without_blocking(self):
        # 정규 파일이 아니면 열지 않는다 — FIFO 를 열면 쓰는 쪽이 올 때까지 막힌다(수리 전 바이트는 여기서 시간 초과).
        fifo = os.path.join(self.work, "a-fifo")
        os.mkfifo(fifo)
        self.open_ledger("F3", ["항목"])
        r = self.pay("F3", fifo, timeout=20)
        self.rc(EXIT_OPEN, r, "pay(FIFO)")
        self.assertIn("지문을 얻지 못했다", r.stderr)

    def test_refused_absent_artifact_stays_open_when_it_appears(self):
        # 지불 시점에 없던 경로는 지문이 없다 — 거부된 뒤 그 자리에 파일이 생겨도 재지불 없이 PAID 가 되지 않는다.
        later = os.path.join(self.work, "later.log")
        self.open_ledger("F4", ["항목"])
        self.rc(EXIT_OPEN, self.pay("F4", later), "pay(부재)")
        self.put("later.log")
        r = self.run_tool("status", "--id", "F4")
        self.rc(EXIT_OPEN, r, "status(부재였던 경로가 생긴 뒤)")
        self.assertIn("state=open", r.stdout)

    def test_fingerprinted_file_beside_a_directory_is_refused(self):
        # 선언한 디렉터리 증거는 지문이 없으므로 다른 정규 파일이 있어도 거부한다.
        d = os.path.join(self.work, "a-directory")
        os.makedirs(d)
        self.open_ledger("F5", ["항목"])
        self.rc(EXIT_OPEN, self.pay("F5", d, self.put("proof.log")), "pay(디렉터리 + 정규 파일)")
        self.rc(EXIT_OPEN, self.run_tool("status", "--id", "F5"), "status")

    def test_legacy_bare_string_record_is_exempt_not_paid(self):
        # 맨 문자열 기록은 존재 면제일 뿐 신원이 증명된 PAID 가 아니다.
        art = self.put("legacy.log")
        self.open_ledger("F6", ["항목"])
        led = self.load_ledger("F6")
        led["items"][0].update({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        with open(self.ledger_path("F6"), "w", encoding="utf-8") as fh:
            json.dump(led, fh, ensure_ascii=False)
        r = self.run_tool("status", "--id", "F6")
        self.rc(10, r, "status(맨 문자열 기록)")
        self.assertIn("지문 없음", r.stdout)


class E2NoDenominator(Rig):
    def test_ledger_without_items_is_not_a_pass(self):
        r = self.open_ledger("L2", [])
        self.rc(EXIT_NO_DENOM, r, "open(항목 0)")
        self.assertIn("NO_DENOM", r.stdout)
        self.rc(EXIT_NO_DENOM, self.run_tool("status", "--id", "L2"), "status --id")
        self.rc(EXIT_NO_DENOM, self.run_tool("status"), "status(좌석 범위)")


class E3NoLedger(Rig):
    def test_empty_pack(self):
        r = self.run_tool("status")
        self.rc(EXIT_NO_LEDGER, r, "status(빈 팩)")
        self.assertIn("NO_LEDGER", r.stdout)

    def test_unknown_id_and_missing_targets(self):
        self.rc(EXIT_NO_LEDGER, self.run_tool("status", "--id", "nope"), "status --id 부재")
        self.rc(EXIT_NO_LEDGER, self.run_tool("pay", "--id", "nope", "--item-no", "1",
                                              "--evidence", EVIDENCE, "--artifact", DURABLE), "pay 부재")
        self.rc(EXIT_NO_LEDGER, self.run_tool("add-item", "--id", "nope", "--item", "x"), "add-item 부재")

    def test_audit_root_without_ledgers(self):
        self.rc(EXIT_NO_LEDGER, self.run_tool("audit", "--roots", self.pack), "audit(빈 루트)")

    def test_other_seats_ledger_does_not_count(self):
        self.open_ledger("LA", ["A 의 항목"], seat=SEAT_A)
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "LA", "--item-no", "1", "--evidence", EVIDENCE,
                                         "--artifact", DURABLE, seat=SEAT_A), "A 지불")
        r = self.run_tool("status", seat=SEAT_B)
        self.rc(EXIT_NO_LEDGER, r, "status(좌석 B · A 의 지불만 있음)")
        self.assertIn("OTHER_SEAT", r.stdout)


class E4Drift(Rig):
    def test_tampered_instruction_text(self):
        self.open_ledger("L4", ["항목"])
        p = self.ledger_path("L4")
        led = self.load_ledger("L4")
        led["instruction"]["text"] += "(사후 덧붙임)"
        with open(p, "w", encoding="utf-8") as fh:
            json.dump(led, fh, ensure_ascii=False)
        r = self.run_tool("status", "--id", "L4")
        self.rc(EXIT_DRIFT, r, "status(변조)")
        self.assertIn("DRIFT", r.stdout)

    def test_unreadable_ledger(self):
        d = os.path.join(self.pack, "round", "_intake")
        os.makedirs(d)
        with open(os.path.join(d, "broken.json"), "w", encoding="utf-8") as fh:
            fh.write("{")
        self.rc(EXIT_DRIFT, self.run_tool("status"), "status(판독 불가)")


class E5Usage(Rig):
    def test_usage_errors_are_5_not_2(self):
        self.rc(EXIT_USAGE, self.run_tool("audit"), "audit --roots 누락")
        self.rc(EXIT_USAGE, self.run_tool("nosuchcmd"), "미지 서브커맨드")
        self.rc(EXIT_USAGE, self.run_tool(), "서브커맨드 없음")

    def test_tool_level_usage(self):
        self.open_ledger("L5", ["항목"])
        self.rc(EXIT_USAGE, self.open_ledger("L5", ["항목"]), "open(이미 있음)")
        self.rc(EXIT_USAGE, self.run_tool("pay", "--id", "L5", "--item-no", "9",
                                          "--evidence", EVIDENCE, "--artifact", DURABLE), "pay(범위 밖)")


class E7Unattributed(Rig):
    def test_ledger_without_seat_is_not_judged(self):
        self.open_ledger("L7", ["항목"], seat={})
        r = self.run_tool("status", seat=SEAT_A)
        self.rc(EXIT_UNATTRIB, r, "status(귀속 없는 대장)")
        self.assertIn("UNATTRIBUTED", r.stdout)

    def test_stamp_is_command_success_not_completion(self):
        # stamp 의 0 = 귀속을 기입했다는 뜻뿐이다 — 그 뒤 좌석 범위 status 는 그 대장을 판정하고 열린 항목으로 1.
        self.open_ledger("L7s", ["항목"], seat={})
        self.rc(EXIT_PAID, self.run_tool("stamp", "--id", "L7s", seat=SEAT_A), "stamp")
        self.rc(EXIT_OPEN, self.run_tool("status", seat=SEAT_A), "status(stamp 뒤)")


STUB_CYS = """#!/bin/sh
d=${0%/*}
case "$1" in
  identify) exec /bin/cat "$d/identify.json" ;;
  status) exec /bin/cat "$d/status.json" ;;
esac
exit 2
"""


@unittest.skipIf(os.name == "nt", "스텁 cys 가 sh 스크립트라 Windows CreateProcess 로는 실행되지 않는다")
class E3SeatAuthority(Rig):
    """스텁 cys(임시 PATH 의 sh 스크립트 — 라이브 데몬 무접촉)로 데몬 권위 갈래를 잰다.
    ⓐ `cys identify` 의 surface 가 env CYS_SURFACE_ID 를 이긴다.
    ⓑ 함대 재번호로 기록된 surface 가 살아있는 목록에서 사라지면 역할로 대조한다 — 같은 역할이면 자기 대장(0),
       역할이 다르거나 기록된 surface 가 아직 살아 있으면 남의 대장(3)."""

    def setUp(self):
        super().setUp()
        self.stub = os.path.join(self.tmp, "stubbin")
        os.makedirs(self.stub)
        p = os.path.join(self.stub, "cys")
        with open(p, "w", encoding="utf-8") as fh:
            fh.write(STUB_CYS)
        os.chmod(p, 0o755)

    def answer(self, surface, live):
        with open(os.path.join(self.stub, "identify.json"), "w", encoding="utf-8") as fh:
            json.dump({"caller": {"surface_id": surface, "surface_ref": "surface:%d" % surface}}, fh)
        with open(os.path.join(self.stub, "status.json"), "w", encoding="utf-8") as fh:
            json.dump({"surfaces": [{"surface_id": s} for s in live]}, fh)

    def env(self, seat):
        e = super().env(seat)
        e["PATH"] = self.stub
        return e

    def test_identify_wins_over_env(self):
        self.answer(107, [107])
        self.open_ledger("S1", ["항목"], seat={"CYS_ROLE": "worker-a", "CYS_SURFACE_ID": "999"})
        self.assertEqual(self.load_ledger("S1")["opened_by"]["surface"], "107")

    def renumbered(self, role_now, live_after):
        self.answer(94, [94])
        self.open_ledger("S2", ["항목"], seat={"CYS_ROLE": "worker-a"})
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "S2", "--item-no", "1", "--evidence", EVIDENCE,
                                         "--artifact", DURABLE, seat={"CYS_ROLE": "worker-a"}), "pay")
        self.answer(107, live_after)
        return self.run_tool("status", seat={"CYS_ROLE": role_now})

    def test_renumbered_seat_keeps_its_ledger(self):
        self.rc(EXIT_PAID, self.renumbered("worker-a", [107, 200]), "status(재번호 · 같은 역할)")

    def test_renumbered_other_role_is_other_seat(self):
        self.rc(EXIT_NO_LEDGER, self.renumbered("worker-b", [107, 200]), "status(재번호 · 다른 역할)")

    def test_live_recorded_surface_is_other_seat(self):
        self.rc(EXIT_NO_LEDGER, self.renumbered("worker-a", [94, 107]), "status(기록 surface 생존)")


class SelfTest(Rig):
    def test_selftest_passes(self):
        r = self.run_tool("selftest")
        self.rc(EXIT_PAID, r, "selftest")
        self.assertIn("SELFTEST OK", r.stdout)


def load_tool():
    """도구를 같은 프로세스로 읽는다 — 캐시(.pyc)를 체크아웃에 남기지 않는다."""
    spec = importlib.util.spec_from_file_location("javis_intake_under_test", TOOL)
    mod = importlib.util.module_from_spec(spec)
    prev = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.dont_write_bytecode = prev
    return mod


class _NtPath:
    """ntpath 에 '존재' 판정만 대역으로 얹는다 — 이 Mac 에 없는 Windows 경로를 실존으로 본다."""

    def __init__(self, existing):
        self._existing = {ntpath.normcase(p) for p in existing}

    def exists(self, p):
        return ntpath.normcase(p) in self._existing

    def __getattr__(self, name):
        return getattr(ntpath, name)


class _NtOs:
    """도구 모듈의 os 대역 — name·path·environ 만 Windows 처럼 바꾸고 나머지는 진짜 os 로 넘긴다."""

    def __init__(self, env, existing=()):
        self.name, self.path, self.environ = "nt", _NtPath(existing), env

    def stat(self, path):
        if not self.path.exists(path):
            raise FileNotFoundError(path)
        return type('Regular', (), dict(st_mode=stat.S_IFREG | 0o600, st_size=1))()

    def __getattr__(self, name):
        return getattr(os, name)


WIN_TEMP = r"C:\Users\x\AppData\Local\Temp"


class WindowsTempRoots(unittest.TestCase):
    """대역 os 로 운영 경로(`os.name`·`os.environ`·`os.path.exists`)를 그대로 태운다 — 그래서 수리 전 바이트에도
    같은 검체가 돌고, 수리 전에는 첫 케이스가 거짓 PAID 로 붉다."""

    @classmethod
    def setUpClass(cls):
        cls.m = load_tool()

    @contextlib.contextmanager
    def as_windows(self, env, existing=()):
        fake = _NtOs(env, existing)
        with mock.patch.object(self.m, "os", fake), mock.patch.object(self.m, "_STAT", fake.stat):
            yield

    def test_temp_only_artifact_is_not_a_payment(self):
        art = WIN_TEMP + r"\proof.log"
        with self.as_windows({"TEMP": WIN_TEMP}, [art]):
            st, why = self.m._item_state({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        self.assertEqual(st, "OPEN", why)
        self.assertIn("임시 경로", why)

    def test_extended_length_temp_artifact_is_not_a_payment(self):
        # \\?\ 를 붙인 같은 %TEMP% 파일 — 접두를 벗기지 않으면 어휘 비교가 빗나가 거짓 PAID 다.
        art = "\\\\?\\" + WIN_TEMP + r"\proof.log"
        with self.as_windows({"TEMP": WIN_TEMP}, [art]):
            st, why = self.m._item_state({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        self.assertEqual(st, "OPEN", why)
        self.assertIn("임시 경로", why)

    def test_durable_bare_artifact_is_exempt(self):
        art = r"C:\Users\x\Documents\proof.log"
        with self.as_windows({"TEMP": WIN_TEMP}, [art]):
            st, why = self.m._item_state({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        self.assertEqual(st, "EXEMPT", why)

    def test_root_resolution(self):
        cases = [  # (env, 경로, 임시인가)
            ({"TEMP": WIN_TEMP}, r"c:\users\X\appdata\local\temp\a.log", True),        # 대소문자
            ({"TEMP": WIN_TEMP}, "C:/Users/x/AppData/Local/Temp/sub/a.log", True),      # 슬래시
            ({"TMP": r"D:\Scratch"}, r"d:\scratch\a.log", True),
            ({"TMPDIR": r"E:\t"}, r"E:\T\a.log", True),
            ({"LOCALAPPDATA": r"C:\Users\x\AppData\Local"}, WIN_TEMP + r"\a.log", True),
            # 8.3 짧은 이름 TEMP + 긴 이름 경로 — LOCALAPPDATA\Temp 가 긴 이름 형태를 덮는다
            ({"TEMP": r"D:\SHORTD~1\Temp", "LOCALAPPDATA": r"D:\ShortDirLong"}, r"D:\ShortDirLong\Temp\a.log", True),
            ({"TEMP": r"C:\Temp"}, r"C:\Temp2\a.log", False),                           # 접두 함정
            ({"TEMP": WIN_TEMP}, r"C:\Users\x\Documents\a.log", False),
            ({}, WIN_TEMP + r"\a.log", False),                                          # 루트를 대는 env 없음
            # DOS 장치 경로 — 같은 파일의 다른 이름(\\?\X:\ · \\?\UNC\ · \\.\ 도 같다). 접두를 벗기고 비교한다.
            ({"TEMP": r"C:\Temp"}, r"\\?\C:\Temp\a.log", True),
            ({"TEMP": r"\\server\share\temp"}, r"\\?\UNC\server\share\temp\a.log", True),
            ({"TEMP": r"\\server\share\temp"}, r"\\?\unc\SERVER\share\Temp\a.log", True),   # UNC 낱말 대소문자
            ({"TEMP": r"C:\Temp"}, "//?/C:/Temp/a.log", True),                          # 슬래시 형태
            ({"TEMP": r"C:\Temp"}, r"\\.\C:\Temp\a.log", True),
            ({"TEMP": r"\\server\share\temp"}, r"\\.\UNC\server\share\temp\a.log", True),
            ({"TEMP": r"\\?\C:\Temp"}, r"C:\Temp\a.log", True),                         # 루트 쪽이 장치 경로
            ({"TEMP": r"C:\Temp"}, r"\\?\C:\Temp2\a.log", False),                       # 접두 함정은 벗긴 뒤에도
            ({"TEMP": r"\\server\share\temp"}, r"\\server\share\temp2\a.log", False),
        ]
        for env, path, want in cases:
            with self.subTest(env=env, path=path):
                with self.as_windows(env):
                    self.assertEqual(self.m._ephemeral(path), want)

    @unittest.skipIf(os.name == "nt", "POSIX 규칙은 POSIX 호스트에서만 성립한다(Windows abspath 는 드라이브 문자가 붙는다)")
    def test_posix_rule_unchanged(self):
        self.assertTrue(self.m._ephemeral("/tmp/a.log", os_name="posix"))
        self.assertTrue(self.m._ephemeral("/var/folders/x/a.log", os_name="posix"))
        self.assertFalse(self.m._ephemeral(DURABLE, os_name="posix"))


class SelftestRelaunch(unittest.TestCase):
    """selftest 의 자기 재기동은 -I 와 함께 -B 를 단다 — -I 는 환경변수의 캐시 금지 봉인을 무시하므로 번들 python 이면
    자식이 .pyc 를 번들 안에 쓸 수 있다. 재기동을 가로채 argv 를 본다(자식은 실제로 뜨지 않는다)."""

    def test_relaunch_carries_dash_b(self):
        m = load_tool()
        seen = []

        def fake_run(argv, *a, **k):
            seen.append(list(argv))
            return subprocess.CompletedProcess(argv, EXIT_USAGE, b"", b"")

        with mock.patch("subprocess.run", fake_run), contextlib.redirect_stdout(io.StringIO()):
            rc = m.cmd_selftest(None)
        relaunch = [a for a in seen if a and a[0] == sys.executable]
        self.assertGreaterEqual(len(relaunch), 2, seen)
        for a in relaunch:
            self.assertIn("-I", a)
            self.assertIn("-B", a)
        self.assertEqual(rc, EXIT_PAID)


# ─── E8 UNREADABLE — 「읽지 못했다」는 「바뀌었다」가 아니다 (CEO 지시 2026-10-23 · WP10) ────────────────
#   CEO 실측: 기록된 크기 그대로 present 인 파일의 모든 읽기가 OSError errno 11(EDEADLK) 로 막혔고(macOS
#   dataless/compressed) 이 도구는 그것을 「증거 바이트가 변경됐다 … 그 절만 떠낸 새 파일로 재지불하라」로 찍었다 —
#   ⛔읽을 수 없는 증거 대신 **새 증거를 지어내라**는 지시다. 아래 검체는 그 구별을 못박는다.
SF_DATALESS = 0x40000000      # macOS sys/stat.h — 내용이 클라우드로 비워졌다
UF_COMPRESSED = 0x20          # macOS — 압축 저장


def _raises(no):
    """읽기만 막는 대역 opener — iCloud 도, 그런 파일도 필요하지 않다(U4: 검체는 재현 가능해야 한다)."""
    def _op(*_a, **_k):
        raise OSError(no, os.strerror(no))
    return _op


class _FakeStat:
    """os.stat 의 대역 결과 — 정규 파일 · 크기 · st_flags. 이 Mac 에 dataless 파일을 만들지 않고 그 갈래를 잰다."""

    def __init__(self, size, flags, mode=0o100644):
        self.st_mode, self.st_size, self.st_flags = mode, size, flags


class _DatalessOs:
    """도구 모듈의 os 대역 — 지정한 한 경로의 stat 만 가로채고 나머지는 진짜 os 로 넘긴다(기존 _NtOs 와 같은 수법)."""

    def __init__(self, path, size, flags):
        self._p, self._st = os.path.realpath(path), _FakeStat(size, flags)

    def stat(self, p, *a, **k):
        if os.path.realpath(p) == self._p:
            return self._st
        return os.stat(p, *a, **k)

    def __getattr__(self, name):
        return getattr(os, name)



class StatBeforeOpen(unittest.TestCase):
    """Inject stat flags only; never create an actual dataless file."""

    def setUp(self):
        self.m = load_tool()
        with open(DURABLE, "rb") as fh:
            self.data = fh.read()
        self.sha = self.m._digest(self.data)[:16]

    def probe(self, flags, platform):
        opener = mock.Mock(wraps=open)
        with mock.patch.object(self.m, "_STAT", return_value=_FakeStat(len(self.data), flags), create=True), \
             mock.patch.object(self.m, "_OPEN", opener), \
             mock.patch.object(self.m.sys, "platform", platform):
            result = self.m._probe(DURABLE)
        print(json.dumps({"platform": platform, "flags": flags, "probe": result,
                          "open_calls": opener.call_count}))
        return result, opener.call_count

    def test_darwin_dataless_never_requests_bytes(self):
        result, calls = self.probe(SF_DATALESS | UF_COMPRESSED, "darwin")
        self.assertEqual(result, (self.m.PROBE_UNREADABLE, None, len(self.data), None,
                                  SF_DATALESS | UF_COMPRESSED))
        self.assertEqual(calls, 0)

    def test_regular_file_still_opens_and_hashes(self):
        result, calls = self.probe(0, "darwin")
        self.assertEqual(result, (self.m.PROBE_OK, self.sha, len(self.data), None, 0))
        self.assertEqual(calls, 1)

    def test_non_darwin_same_bit_still_opens(self):
        for platform in ("linux", "win32"):
            with self.subTest(platform=platform):
                result, calls = self.probe(SF_DATALESS, platform)
                self.assertEqual(result, (self.m.PROBE_OK, self.sha, len(self.data), None, SF_DATALESS))
                self.assertEqual(calls, 1)

    def test_fp_dataless_cannot_create_payment_fingerprint(self):
        opener = mock.Mock(wraps=open)
        with mock.patch.object(self.m, "_STAT", return_value=_FakeStat(len(self.data), SF_DATALESS), create=True), \
             mock.patch.object(self.m, "_OPEN", opener), \
             mock.patch.object(self.m.sys, "platform", "darwin"):
            result = self.m._fp(DURABLE)
        print(json.dumps({"case": "fp", "fingerprint": result, "open_calls": opener.call_count}))
        self.assertEqual(result, (None, None))
        self.assertEqual(opener.call_count, 0)

    def test_no_open_note_has_no_fabricated_errno_or_repayment(self):
        opener = mock.Mock(wraps=open)
        item = {"state": "done", "evidence": EVIDENCE,
                "artifacts": [{"path": DURABLE, "sha256_16": self.sha, "size": len(self.data)}]}
        with mock.patch.object(self.m, "_STAT", return_value=_FakeStat(len(self.data), SF_DATALESS), create=True), \
             mock.patch.object(self.m, "_OPEN", opener), \
             mock.patch.object(self.m.sys, "platform", "darwin"):
            state, why = self.m._item_state(item)
        print(json.dumps({"case": "note", "state": state, "why": why,
                          "open_calls": opener.call_count}, ensure_ascii=False))
        self.assertEqual(state, "UNREADABLE")
        self.assertIn("열지 않았다", why)
        self.assertIn("바이트를 요청하지 않았다", why)
        self.assertNotIn("errno 11", why)
        self.assertNotIn("재지불", why)
        self.assertEqual(opener.call_count, 0)

    def test_non_regular_is_rejected_before_dataless(self):
        opener = mock.Mock(wraps=open)
        with mock.patch.object(self.m, "_STAT", return_value=_FakeStat(0, SF_DATALESS, mode=0o040755), create=True), \
             mock.patch.object(self.m, "_OPEN", opener), \
             mock.patch.object(self.m.sys, "platform", "darwin"):
            result = self.m._probe(DURABLE)
        self.assertEqual(result[0], self.m.PROBE_NOT_REGULAR)
        self.assertEqual(opener.call_count, 0)


class UnreadableIsNotChanged(unittest.TestCase):
    """도구를 같은 프로세스로 읽고 운영 경로(_item_state·_evaluate)를 그대로 태운다. 대역은 `create=True` 로 심는다 —
    ★그래서 수리 전 바이트에서도 이 검체가 돌고, 거기서는 붉다(대역을 쓰지 않는 옛 _fp 가 파일을 읽어 PAID 를 낸다)."""

    @classmethod
    def setUpClass(cls):
        cls.m = load_tool()

    def setUp(self):
        # Keep existing os-proxy fixtures (including the protected size pin) on the new stat seam.
        patcher = mock.patch.object(self.m, "_STAT", side_effect=lambda *a, **k: self.m.os.stat(*a, **k))
        patcher.start()
        self.addCleanup(patcher.stop)

    def item(self, sha, size, path=DURABLE):
        return {"state": "done", "evidence": EVIDENCE,
                "artifacts": [{"path": path, "sha256_16": sha, "size": size}]}

    def test_unreadable_artifact_is_not_a_change(self):
        sha, size = self.m._fp(DURABLE)
        with mock.patch.object(self.m, "_OPEN", _raises(11), create=True):
            st, why = self.m._item_state(self.item(sha, size))
        self.assertEqual(st, "UNREADABLE", why)
        self.assertIn("11", why, why)
        self.assertNotIn("재지불", why)          # ⛔증거를 지어내라는 처방이 없어야 한다(blocking 조항)
        self.assertNotIn("변경됐다", why)
        self.assertIn("무효가 아니다", why)
        self.assertIn("정황", why)                # 크기 일치는 정황이지 신원이 아니다

    def test_real_change_is_still_changed_with_its_remedy(self):
        # ★음성 대조 — 읽히는 파일의 진짜 지문 불일치는 종전대로 OPEN + 재지불이다(판독 불가가 그것을 삼키지 않았다).
        st, why = self.m._item_state(self.item("0" * 16, 1))
        self.assertEqual(st, "OPEN", why)
        self.assertIn("변경됐다", why)
        self.assertIn("재지불", why)

    def test_absent_artifact_is_absent_not_changed(self):
        gone = os.path.join(TESTS_DIR, "no-such-artifact-unreadable.log")
        self.assertFalse(os.path.exists(gone))
        st, why = self.m._item_state(self.item("0" * 16, 1, path=gone))
        self.assertEqual(st, "OPEN", why)
        self.assertIn("부재", why)
        self.assertNotIn("변경됐다", why)

    def test_dataless_line_reports_no_open_and_the_flag(self):
        sha, size = self.m._fp(DURABLE)
        with mock.patch.object(self.m, "os", _DatalessOs(DURABLE, size, SF_DATALESS | UF_COMPRESSED)), \
             mock.patch.object(self.m, "_OPEN", side_effect=_raises(11)) as opener, \
             mock.patch.object(self.m.sys, "platform", "darwin"):
            st, why = self.m._item_state(self.item(sha, size))
        self.assertEqual(st, "UNREADABLE", why)
        self.assertIn("열지 않았다", why)
        self.assertNotIn("errno 11", why)
        self.assertIn("st_flags=0x", why)
        self.assertEqual(opener.call_count, 0)

    def test_flag_names_are_darwin_only(self):
        # U5: 다른 OS 에서도 errno 로 분류는 되고, darwin 전용 비트 해석만 꺼진다.
        self.assertIn("dataless", self.m._flags_note(SF_DATALESS, plat="darwin"))
        self.assertNotIn("dataless", self.m._flags_note(SF_DATALESS, plat="win32"))
        self.assertIn("0x", self.m._flags_note(SF_DATALESS, plat="win32"))
        self.assertEqual(self.m._flags_note(None), "")

    def test_size_mismatch_under_unreadable_is_said_plainly(self):
        # 읽지 못했으면 변경을 **확정하지 못한다** — 크기 차이는 정황으로 찍고 재지불은 요구하지 않는다(판정은 사람 몫).
        sha, size = self.m._fp(DURABLE)
        with mock.patch.object(self.m, "os", _DatalessOs(DURABLE, size + 7, 0)), \
             mock.patch.object(self.m, "_OPEN", _raises(11), create=True):
            st, why = self.m._item_state(self.item(sha, size))
        self.assertEqual(st, "UNREADABLE", why)
        self.assertIn("다르다", why)
        self.assertNotIn("재지불", why)

    def test_unreadable_only_ledger_is_exit_8(self):
        sha, size = self.m._fp(DURABLE)
        led = {"id": "t", "instruction": {"text": "x", "sha256": self.m._digest(b"x")},
               "items": [self.item(sha, size)]}
        with mock.patch.object(self.m, "_OPEN", _raises(11), create=True):
            rc, lines = self.m._evaluate(led)
        self.assertEqual(rc, EXIT_UNREADABLE, lines)
        self.assertTrue(any("판독 불가" in l for l in lines), lines)

    def test_an_open_item_outranks_unreadable(self):
        # ★심각도는 숫자 크기가 아니다(_SEVERITY) — 8 이 1 을 가리면 열린 항목이 집계에서 사라진다.
        sha, size = self.m._fp(DURABLE)
        led = {"id": "t", "instruction": {"text": "x", "sha256": self.m._digest(b"x")},
               "items": [self.item(sha, size), {"state": "open", "text": "미지불"}]}
        with mock.patch.object(self.m, "_OPEN", _raises(11), create=True):
            rc, lines = self.m._evaluate(led)
        self.assertEqual(rc, EXIT_OPEN, lines)
        self.assertTrue(any("미지불" in l for l in lines), lines)
        self.assertEqual(self.m._worse(EXIT_UNREADABLE, EXIT_OPEN), EXIT_OPEN)
        self.assertEqual(self.m._worse(EXIT_PAID, EXIT_UNREADABLE), EXIT_UNREADABLE)
        self.assertEqual(self.m._worse(EXIT_OPEN, EXIT_DRIFT), EXIT_DRIFT)

    def test_a_non_oserror_read_failure_does_not_kill_the_judgement(self):
        # ★R1: `_probe` 는 OSError 만 분류한다 — 거대 artifact 의 MemoryError 같은 예외가 `_item_state` 밖으로
        #   터지면 대장 하나가 status·audit **전체**를 죽인다(수리 직후 바이트의 실측 결함). 종전 결말로 떨어뜨린다.
        def _boom(*_a, **_k):
            raise MemoryError("artifact too large to read into memory")
        sha, size = self.m._fp(DURABLE)
        with mock.patch.object(self.m, "_OPEN", _boom, create=True):
            st, why = self.m._item_state(self.item(sha, size))
        self.assertEqual(st, "OPEN", why)
        self.assertIn("변경됐다", why)

    def test_pay_time_unreadable_is_still_refused(self):
        # 지불 **시점**에 읽히지 않는 artifact 는 지문이 없다 ⇒ 종전대로 지불이 아니다(이 수리가 그 문을 열지 않았다).
        with mock.patch.object(self.m, "_OPEN", _raises(13), create=True):
            sha, size = self.m._fp(DURABLE)
        self.assertIsNone(sha)
        st, why = self.m._item_state({"state": "done", "evidence": EVIDENCE,
                                      "artifacts": [{"path": DURABLE, "sha256_16": None, "size": None}]})
        self.assertEqual(st, "OPEN", why)
        self.assertIn("지문을 얻지 못했다", why)


class E8UnreadableExit(DurableRig):
    """CLI 의 exit 계약 — 진짜 파일을 권한 000 으로 막아 잰다(iCloud 무접촉 · POSIX 권한만). 수리 전에는 1 + 재지불 처방이다."""

    def _make_unreadable(self, art):
        os.chmod(art, 0)
        self.addCleanup(os.chmod, art, 0o600)
        if os.access(art, os.R_OK):
            self.skipTest("이 계정은 권한 000 파일을 읽는다(root 등) — 읽기 불가를 만들 수 없다")

    @unittest.skipIf(os.name == "nt", "chmod 000 은 Windows 에서 읽기를 막지 않는다")
    def test_status_and_audit_report_unreadable_without_a_repay_remedy(self):
        art = self.put("evicted.log")
        self.open_ledger("U8", ["항목"])
        self.rc(EXIT_PAID, self.pay("U8", art), "pay(읽을 수 있을 때)")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "U8"), "status(읽을 수 있을 때)")
        self._make_unreadable(art)
        r = self.run_tool("status", "--id", "U8")
        self.rc(EXIT_UNREADABLE, r, "status(읽기 불가)")
        self.assertIn("판독 불가", r.stdout)
        self.assertIn("무효가 아니다", r.stdout)
        self.assertNotIn("재지불", r.stdout)
        self.assertNotIn("증거 바이트가 변경됐다", r.stdout)
        ra = self.run_tool("audit", "--roots", self.pack)
        self.rc(EXIT_UNREADABLE, ra, "audit(읽기 불가)")
        self.assertIn("UNREADABLE", ra.stdout)
        self.assertIn("판독 불가", ra.stdout)
        self.assertNotIn("재지불", ra.stdout)

    @unittest.skipIf(os.name == "nt", "chmod 000 은 Windows 에서 읽기를 막지 않는다")
    def test_an_open_item_still_wins_over_unreadable_in_audit(self):
        art = self.put("evicted2.log")
        self.open_ledger("U9", ["항목"])
        self.rc(EXIT_PAID, self.pay("U9", art), "pay")
        self.rc(EXIT_PAID, self.run_tool("add-item", "--id", "U9", "--item", "뒤늦게 받은 항목"), "add-item")
        self._make_unreadable(art)
        r = self.run_tool("audit", "--roots", self.pack)
        self.rc(EXIT_OPEN, r, "audit(열림 1 + 판독 불가 1)")
        self.assertIn("뒤늦게 받은 항목", r.stdout)



class P5NIntegrity(Rig):
    """Real reader/writer regressions; no live ledger or daemon access."""
    def setUp(self):
        super().setUp()
        self.m = load_tool()
        self.open_ledger("N", ["one", "two"])
        self.args = type("Args", (), dict(root=self.pack, id="N", item_no=1,
                    evidence=EVIDENCE, artifact=[DURABLE], reason=REASON,
                    release_condition="decision"))()

    def bytes(self):
        with open(self.ledger_path("N"), "rb") as f:
            return f.read()

    def event(self):
        sha, size = self.m._fp(DURABLE)
        return dict(kind="pay", state="done", evidence=EVIDENCE, paid_at="t",
                    artifacts=[dict(path=DURABLE, sha256_16=sha, size=size, ephemeral=False)])

    def test_p5n_refusals_preserve_previous_payment_bytes(self):
        self.assertEqual(self.m.cmd_pay(self.args), 0)
        before = self.bytes()
        for changes, command in ((dict(evidence="short"), self.m.cmd_pay),
                                 (dict(evidence=EVIDENCE, artifact=[DURABLE+".absent"]), self.m.cmd_pay),
                                 (dict(reason="short"), self.m.cmd_defer)):
            self.args.__dict__.update(changes)
            with self.subTest(changes=changes), mock.patch.object(self.m, "_write_atomic", wraps=self.m._write_atomic) as write:
                self.assertEqual(command(self.args), 1)
                self.assertEqual(self.bytes(), before)
                write.assert_not_called()

    def test_p5n_append_snapshot_lazy_and_conflict_output(self):
        original = self.load_ledger("N")
        original['items'][0]['unknown'] = {'verbatim': [1, 'two']}
        with open(self.ledger_path("N"), 'w') as f:
            json.dump(original, f)
        self.assertEqual(self.m.cmd_pay(self.args), 0)
        first = self.load_ledger("N")
        item = first['items'][0]
        self.assertIn('payments', item)
        self.assertEqual(item['payments'][0]['kind'], 'legacy_snapshot')
        self.assertEqual(item['payments'][0]['payload'], original['items'][0])
        self.assertEqual(first['items'][1], original['items'][1])
        # A v1 writer's mirror is undecidable even if its current proof looks valid.
        item.update(dict(state='done', evidence='mirror '+EVIDENCE, artifacts=[DURABLE], paid_at='v1'))
        with open(self.ledger_path("N"), 'w') as f:
            json.dump(first, f)
        st, why = self.m._item_state(item)
        self.assertEqual(st, 'CONFLICT')
        self.assertIn('mirror '+EVIDENCE, why)
        self.assertIn('paid_at', why)
        self.assertIn('journal', why)
        snap = {k:v for k,v in item.items() if k != 'payments'}
        self.assertEqual(self.m.cmd_pay(self.args), 0)
        final = self.load_ledger("N")['items'][0]
        self.assertEqual([x['kind'] for x in final['payments']], ['legacy_snapshot','pay','legacy_snapshot','pay'])
        self.assertEqual(final['payments'][-2]['payload'], snap)
        self.assertEqual(final['payments'][-2]['provenance'], 'v1 fields, acceptance unproven')
        self.assertNotIn('state', final)

    def test_p5n_journal_validation_is_in_reader(self):
        ev = self.event()
        variants = [None, {}, dict(ev, state='defer'), dict(ev, evidence=12),
                    dict(ev, artifacts=[DURABLE]), dict(ev, artifacts=[{}]),
                    dict(ev, paid_at=''), dict(kind='unknown')]
        for val in (None, 0, 1, 'false'):
            variants.append(dict(ev, artifacts=[dict(ev['artifacts'][0], ephemeral=val)]))
        art = dict(ev['artifacts'][0]); art.pop('ephemeral')
        variants.append(dict(ev, artifacts=[art]))
        for bad in variants:
            for entries in ([ev,bad], [bad,ev]):
                with self.subTest(entries=entries):
                    self.assertEqual(self.m._item_state({'payments':entries})[0], 'DRIFT')
        self.assertEqual(self.m._item_state({'payments':[],'state':'done'})[0], 'CONFLICT')
        self.assertEqual(self.m._item_state({'payments':None})[0], 'DRIFT')
        self.assertEqual(self.m._item_state({'payments':[dict(kind='legacy_snapshot', payload=ev,
                         at='t', provenance='v1 fields, acceptance unproven')]})[0], 'OPEN')

    def test_p5n_bare_is_stat_only_exempt(self):
        with mock.patch.object(self.m, '_OPEN', side_effect=AssertionError('bare opened')) as op, \
                mock.patch.object(self.m, '_STAT', wraps=self.m._STAT) as stat_call:
            st, why = self.m._item_state(dict(state='done',evidence=EVIDENCE,artifacts=[DURABLE]))
            self.assertEqual(st, 'EXEMPT')
            self.assertEqual(stat_call.call_count, 1)
            op.assert_not_called()
        with mock.patch.object(self.m, '_STAT', side_effect=PermissionError(13, 'denied')):
            self.assertEqual(self.m._item_state(dict(state='done',evidence=EVIDENCE,artifacts=[DURABLE]))[0], 'UNREADABLE')

    def test_p5n_recorded_ephemeral_reaches_fold(self):
        ev = self.event()
        vanished = dict(path=DURABLE+'.gone',sha256_16='a'*16,size=1,ephemeral=True)
        ev['artifacts'].append(vanished)
        self.assertEqual(self.m._item_state({'payments':[ev]})[0], 'PAID')
        vanished['ephemeral'] = False
        self.assertEqual(self.m._item_state({'payments':[ev]})[0], 'OPEN')
        self.assertEqual(self.m.cmd_pay(self.args), 0)
        item = self.load_ledger('N')['items'][0]
        self.assertIs(item['payments'][-1]['artifacts'][0]['ephemeral'], False)

    def test_p5n_foreign_paths_never_probe(self):
        paths = [r'C:\proof.txt', r'\\server\share\proof', '//server/share/proof', 'relative.txt']
        with mock.patch.object(self.m, '_STAT', side_effect=AssertionError('foreign stat')):
            for p in paths:
                with self.subTest(path=p):
                    self.assertEqual(self.m._item_state(dict(state='done',evidence=EVIDENCE,artifacts=[p]))[0], 'UNRESOLVABLE')

    def test_p5n_atomic_write_syncs_and_retains_preimage(self):
        path = self.ledger_path('N'); before = self.bytes()
        with mock.patch.object(self.m.os, 'fsync', wraps=os.fsync) as fsync:
            rc = self.m._write_atomic(path, self.load_ledger('N'))
            self.assertEqual(rc, 0)
            self.assertGreaterEqual(fsync.call_count, 5)
        backups = os.listdir(os.path.join(os.path.dirname(path), '.preimage'))
        self.assertEqual(len(backups), 1)
        with open(os.path.join(os.path.dirname(path), '.preimage', backups[0]), 'rb') as f:
            self.assertEqual(f.read(), before)


    def test_p5n_durability_outcomes_and_preimage_mismatch(self):
        path = self.ledger_path('N')
        original = self.bytes()
        candidate = self.load_ledger('N'); candidate['test_marker'] = 'new'
        for mode, expected, committed in [('ok',0,True),('early-unsupported',13,True),
                                         ('backup-unsupported',13,True),('backup-fail',11,False),
                                         ('final-fail',12,True),('final-unsupported',13,True),
                                         ('windows',0,True)]:
            with self.subTest(mode=mode):
                with open(path,'wb') as f: f.write(original)
                published = [False]; seen = []
                replace = os.replace
                def publish(src, dst):
                    replace(src,dst)
                    if dst == path: published[0] = True
                def sync(d):
                    seen.append(d)
                    if mode == 'windows': return 'SYNC_NONPOSIX'
                    if mode == 'early-unsupported' and len(seen)==1: return 'SYNC_UNSUPPORTED'
                    if d.endswith('.preimage'):
                        if mode=='backup-unsupported': return 'SYNC_UNSUPPORTED'
                        if mode=='backup-fail': return 'SYNC_FAILED'
                    if published[0]:
                        if mode=='final-fail': return 'SYNC_FAILED'
                        if mode=='final-unsupported': return 'SYNC_UNSUPPORTED'
                    return 'SYNC_OK'
                with mock.patch.object(self.m,'_sync_dir',side_effect=sync,create=True), \
                        mock.patch.object(self.m.os,'replace',side_effect=publish), contextlib.redirect_stderr(io.StringIO()) as err:
                    rc = self.m._write_atomic(path,candidate)
                self.assertEqual(rc,expected)
                self.assertEqual(published[0],committed)
                self.assertEqual(self.bytes()!=original,committed)
                if mode=='windows':
                    self.assertIn('COMMITTED/DURABILITY_UNVERIFIED (platform has no directory sync)',err.getvalue())
        with open(path,'wb') as f:f.write(original)
        builtin_open = open
        def corrupt_read(p,*args,**kwargs):
            if '.preimage' in str(p) and args and args[0]=='rb': return io.BytesIO(b'corrupt')
            return builtin_open(p,*args,**kwargs)
        with mock.patch.object(self.m,'open',side_effect=corrupt_read,create=True):
            self.assertEqual(self.m._write_atomic(path,candidate),11)
        self.assertEqual(self.bytes(),original)

    def test_p5n_each_published_file_is_flushed_before_replace(self):
        synced = set(); published = []
        real_sync, real_replace = os.fsync, os.replace
        def sync(fd):
            st = os.fstat(fd)
            if stat.S_ISREG(st.st_mode): synced.add((st.st_dev, st.st_ino))
            return real_sync(fd)
        def replace(src,dst):
            st = os.stat(src)
            self.assertIn((st.st_dev,st.st_ino),synced,'published unsynced file')
            published.append(dst)
            return real_replace(src,dst)
        with mock.patch.object(self.m.os,'fsync',side_effect=sync), mock.patch.object(self.m.os,'replace',side_effect=replace):
            self.assertEqual(self.m._write_atomic(self.ledger_path('N'),self.load_ledger('N')),0)
        self.assertEqual(len(published),2)
        self.assertIn('.preimage',published[0])
        self.assertEqual(published[1],self.ledger_path('N'))

    def test_p5n_file_sync_and_replace_fail_before_commit(self):
        path=self.ledger_path('N'); before=self.bytes(); candidate=self.load_ledger('N')
        for fail in ('fsync','replace'):
            with self.subTest(fail=fail), mock.patch.object(self.m.os,fail,side_effect=OSError(5,'injected')):
                self.assertEqual(self.m._write_atomic(path,candidate),11)
            self.assertEqual(self.bytes(),before)

    def test_p5n_sync_classification_and_parent_entry_order(self):
        self.assertTrue(hasattr(self.m,'_sync_dir'))
        with mock.patch.object(self.m.os,'fsync',side_effect=OSError(22,'unsupported')):
            self.assertEqual(self.m._sync_dir(self.tmp),'SYNC_UNSUPPORTED')
        with mock.patch.object(self.m.os,'open',side_effect=OSError(13,'denied')):
            self.assertEqual(self.m._sync_dir(self.tmp),'SYNC_FAILED')
        with mock.patch.object(self.m,'os',_NtOs({})), mock.patch.object(self.m.os,'open',side_effect=AssertionError('directory opened')):
            self.assertEqual(self.m._sync_dir(self.tmp),'SYNC_NONPOSIX')
        path=os.path.join(self.tmp,'fresh','round','_intake','L.json'); synced=[]; observed=[]
        replace=os.replace
        def publish(src,dst):
            observed.append((dst,list(synced)));return replace(src,dst)
        with mock.patch.object(self.m,'_sync_dir',side_effect=lambda d:(synced.append(d) or 'SYNC_OK')), \
                mock.patch.object(self.m.os,'replace',side_effect=publish):
            self.assertEqual(self.m._write_atomic(path,{'items':[]}),0)
        before=next(seen for dst,seen in observed if dst==path)
        for d in (self.tmp,os.path.join(self.tmp,'fresh'),os.path.join(self.tmp,'fresh','round')):
            self.assertIn(d,before)
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(path),'.preimage')))

    def test_p5n_isolated_import_does_not_write_sibling_bytecode(self):
        bundle = os.path.join(self.tmp, 'bundle')
        os.mkdir(bundle)
        for name in ('javis_intake.py','javis_lock.py'):
            shutil.copyfile(os.path.join(BIN,name),os.path.join(bundle,name))
        with mock.patch.dict(globals(), TOOL=os.path.join(bundle,'javis_intake.py')):
            self.rc(0,self.run_tool('add-item','--id','N','--item','isolated'),'isolated -I without -B')
        self.assertFalse(os.path.exists(os.path.join(bundle,'__pycache__')))

    def test_p5n_lock_is_nonblocking_and_released(self):
        spec=importlib.util.spec_from_file_location('lock_for_intake_test',os.path.join(BIN,'javis_lock.py'))
        lockmod=importlib.util.module_from_spec(spec);spec.loader.exec_module(lockmod)
        with lockmod.FileLock(self.ledger_path('N')+'.write-lock',owner='test-holder'):
            for argv in [('add-item','--id','N','--item','blocked'),
                         ('pay','--id','N','--item-no','1','--evidence',EVIDENCE,'--artifact',DURABLE),
                         ('defer','--id','N','--item-no','1','--reason',REASON,'--release-condition','later'),
                         ('stamp','--id','N'),('open','--id','N','--instruction-file',self.instr)]:
                with self.subTest(command=argv[0]):
                    r=self.run_tool(*argv)
                    self.rc(5,r,'writer held')
                    self.assertIn('holder=',r.stderr)
                    self.assertEqual(len(self.load_ledger('N')['items']),2)
        self.rc(0,self.run_tool('add-item','--id','N','--item','after release'),'released')
        self.assertEqual(len(self.load_ledger('N')['items']),3)

    def test_p5n_residue_old_accepted_and_refused_writers_lose_new_commit(self):
        """RESIDUE, deliberately still true: v1 never takes the new writer lock."""
        fixture=os.path.join(TESTS_DIR,'fixtures','intake-v1-9dbea792.txt')
        with open(fixture,'rb') as f: raw=f.read()
        self.assertEqual(hashlib.sha256(raw).hexdigest()[:16],'9e312adebd6b3b20')
        scope={'__name__':'pinned_v1','__file__':fixture}
        exec(compile(raw,fixture,'exec'),scope)
        for accepted in (True,False):
            with self.subTest(old_branch='accepted' if accepted else 'refused'):
                ev=self.event(); ev['evidence']='A '+EVIDENCE
                led=self.load_ledger('N');led['items'][0]={'text':'one','payments':[ev]}
                with open(self.ledger_path('N'),'w') as f:json.dump(led,f)
                classify=scope['_item_state'];published=[]
                def interleave(it,root_for_rel=None):
                    self.args.evidence='B '+EVIDENCE
                    self.assertEqual(self.m.cmd_pay(self.args),0)
                    published.append(self.load_ledger('N')['items'][0]['payments'][-1]['evidence'])
                    return classify(it,root_for_rel)
                scope['_item_state']=interleave
                args=copy.copy(self.args);args.evidence=EVIDENCE if accepted else 'no'
                try: rc=scope['cmd_pay'](args)
                finally:scope['_item_state']=classify
                self.assertEqual(rc,0 if accepted else 1)
                self.assertEqual(published,['B '+EVIDENCE])
                remaining=self.load_ledger('N')['items'][0]['payments']
                self.assertEqual(len(remaining),1,'residue unexpectedly changed: inspect old writer')
                self.assertEqual(remaining[0]['evidence'],'A '+EVIDENCE)

    def test_p5n_conflict_temporal_cases_and_severity(self):
        ev=self.event()
        for outcome in ('ABSENT','UNREADABLE','OK'):
            mirror=dict(ev);mirror['payments']=[ev]
            with mock.patch.object(self.m,'_probe',return_value=(outcome,None,None,13,None)):
                st,why=self.m._item_state(mirror)
            self.assertEqual(st,'CONFLICT',why)
            self.assertEqual(json.loads(why)['legacy']['paid_at'],'t')
            self.assertEqual(json.loads(why)['journal'],ev)
        codes=[0,10,8,14,9,1,2,3,4,7]
        for i,a in enumerate(codes):
            for b in codes[i:]:
                self.assertEqual(self.m._worse(a,b),b)
                self.assertEqual(self.m._worse(b,a),b)
        led=self.load_ledger('N');led['items']=[dict(ev,payments=[])];
        with open(self.ledger_path('N'),'w') as f:json.dump(led,f)
        for argv in [('status','--id','N'),('audit','--roots',self.pack)]:
            result=self.run_tool(*argv);self.rc(14,result,'conflict '+argv[0])
            for word in ('legacy','journal',EVIDENCE):self.assertIn(word,result.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
