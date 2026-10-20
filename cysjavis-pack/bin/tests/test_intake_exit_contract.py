#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_intake_exit_contract.py — javis_intake.py(접수-지불 대장) exit 계약 회귀 (B-328 #8 · 2026-10-20).

## 무엇을 못박는가
이 도구의 사실은 exit code 다(도구 머리말 【결정론 계약】). 실제 CLI 를 서브프로세스로 돌려 각 갈래를 한 번씩 낸다.
  E0 PAID      open → pay(내구 artifact + 증거 40자) → status = 0 · 정당한 defer = 0 · audit 은 모든 대장이 지불일 때만 0
  E1 OPEN      항목이 열려 있으면 1 · 짧은 증거·부재 artifact·임시 경로만인 pay·조건 없는 defer 는 REFUSED = 1
               · 지불 뒤 증거 바이트가 바뀌면(덧붙임 포함) 다시 1 · 지불된 대장에 add-item 하면 다시 1
               · 정규 파일의 지문을 얻지 못한 새 지불(디렉터리·읽기 불가·FIFO·지불 시점 부재)은 REFUSED = 1 이고
                 그 자리에 뒤늦게 파일이 생겨도 1 — 지문 면제는 지문 칸 이전의 맨 문자열 기록뿐이다(그쪽은 0 유지)
  E2 NO_DENOM  항목 0건 대장 = 2 — ★통과(0)가 아니다
  E3 NO_LEDGER 빈 팩·없는 --id·대상 부재 pay/add-item·빈 루트 audit·다른 좌석의 대장만 있음 = 3
               · 좌석 권위(스텁 cys · POSIX): identify 가 env 를 이긴다 · 재번호된 같은 역할은 자기 대장(0)
  E4 DRIFT     기록된 지시 원문을 사후 변조 = 4 · 판독 불가 대장 = 4
  E5 USAGE     사용법 오류는 argparse 기본 2 가 아니라 5 — 2(NO_DENOM)와 같은 숫자가 두 사실을 뜻하지 않는다
  E7 UNATTRIB  좌석 귀속이 없는 대장은 좌석 범위 status 에서 판정하지 않는다 = 7 · stamp 의 0 은 명령 성공일 뿐(그 뒤 status 1)
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


class Rig(unittest.TestCase):
    """임시 팩 1개 + 빈 PATH + 좌석 env — 케이스마다 새로 만든다."""

    def setUp(self):
        self.assertTrue(os.path.isfile(TOOL), "도구 부재: %s" % TOOL)
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="intake-exit-"))
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

    def test_short_evidence_is_refused_and_recorded(self):
        self.open_ledger("L1c", ["항목"])
        r = self.run_tool("pay", "--id", "L1c", "--item-no", "1", "--evidence", "했다",
                          "--artifact", DURABLE)
        self.rc(EXIT_OPEN, r, "pay(증거 2자)")
        self.assertIn("REFUSED", r.stderr)
        self.assertIn("evidence 2자", r.stderr)
        led = self.load_ledger("L1c")
        self.assertEqual(led["items"][0]["state"], "done", "거부도 기록돼야 한다")
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
        self.assertIn("지문을 얻지 못했다", r.stdout)

    def test_fingerprinted_file_beside_a_directory_is_paid(self):
        # 지불은 지문을 얻은 내구 정규 파일 1건이면 성립한다 — 같이 적힌 디렉터리는 지불을 싣지 않을 뿐 막지 않는다.
        d = os.path.join(self.work, "a-directory")
        os.makedirs(d)
        self.open_ledger("F5", ["항목"])
        self.rc(EXIT_PAID, self.pay("F5", d, self.put("proof.log")), "pay(디렉터리 + 정규 파일)")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "F5"), "status")

    def test_legacy_bare_string_record_keeps_its_allowance(self):
        # 지문 칸이 생기기 전의 기록(맨 문자열)은 지문 없이 실존·내구로 지불이 유지된다(소급 검증 불가 표시와 함께).
        art = self.put("legacy.log")
        self.open_ledger("F6", ["항목"])
        led = self.load_ledger("F6")
        led["items"][0].update({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        with open(self.ledger_path("F6"), "w", encoding="utf-8") as fh:
            json.dump(led, fh, ensure_ascii=False)
        r = self.run_tool("status", "--id", "F6")
        self.rc(EXIT_PAID, r, "status(맨 문자열 기록)")
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

    def __getattr__(self, name):
        return getattr(os, name)


WIN_TEMP = r"C:\Users\x\AppData\Local\Temp"


class WindowsTempRoots(unittest.TestCase):
    """대역 os 로 운영 경로(`os.name`·`os.environ`·`os.path.exists`)를 그대로 태운다 — 그래서 수리 전 바이트에도
    같은 검체가 돌고, 수리 전에는 첫 케이스가 거짓 PAID 로 붉다."""

    @classmethod
    def setUpClass(cls):
        cls.m = load_tool()

    def as_windows(self, env, existing=()):
        return mock.patch.object(self.m, "os", _NtOs(env, existing))

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

    def test_durable_artifact_is_still_a_payment(self):
        art = r"C:\Users\x\Documents\proof.log"
        with self.as_windows({"TEMP": WIN_TEMP}, [art]):
            st, why = self.m._item_state({"state": "done", "evidence": EVIDENCE, "artifacts": [art]})
        self.assertEqual(st, "PAID", why)

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


if __name__ == "__main__":
    unittest.main(verbosity=2)
