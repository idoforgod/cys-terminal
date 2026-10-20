#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_intake_exit_contract.py — javis_intake.py(접수-지불 대장) exit 계약 회귀 (B-328 #8 · 2026-10-20).

## 무엇을 못박는가
이 도구의 사실은 exit code 다(도구 머리말 【결정론 계약】). 실제 CLI 를 서브프로세스로 돌려 각 갈래를 한 번씩 낸다.
  E0 PAID      open → pay(내구 artifact + 증거 40자) → status = 0 · 정당한 defer = 0
  E1 OPEN      항목이 열려 있으면 1 · 짧은 증거·부재 artifact·임시 경로만인 pay·조건 없는 defer 는 REFUSED = 1
               · 지불 뒤 증거 바이트가 바뀌면(덧붙임 포함) 다시 1 · 지불된 대장에 add-item 하면 다시 1
  E2 NO_DENOM  항목 0건 대장 = 2 — ★통과(0)가 아니다
  E3 NO_LEDGER 빈 팩·없는 --id·대상 부재 pay/add-item·빈 루트 audit·다른 좌석의 대장만 있음 = 3
               · 좌석 권위(스텁 cys · POSIX): identify 가 env 를 이긴다 · 재번호된 같은 역할은 자기 대장(0)
  E4 DRIFT     기록된 지시 원문을 사후 변조 = 4 · 판독 불가 대장 = 4
  E5 USAGE     사용법 오류는 argparse 기본 2 가 아니라 5 — 2(NO_DENOM)와 같은 숫자가 두 사실을 뜻하지 않는다
  E7 UNATTRIB  좌석 귀속이 없는 대장은 좌석 범위 status 에서 판정하지 않는다 = 7
  S  selftest  = 0 (POSIX 한정 — 아래 한계)

## 밀폐
- 팩·HOME·cwd 는 케이스마다 새 임시 디렉터리다. 라이브 팩(~/.cys/pack*)은 읽지도 쓰지도 않는다.
- 자식 PATH 는 빈 디렉터리다 ⇒ 도구의 `cys identify`·`cys status --json` 호출은 실행 파일을 못 찾고(도구가
  예외를 삼킨다) **라이브 데몬에 닿지 않는다**. 좌석 귀속은 env(CYS_ROLE·CYS_SURFACE_ID — 도구의 비권위
  폴백 경로)로 준다. 부모의 CLAUDE_CONFIG_DIR·CYS_* 는 자식에 넘기지 않는다.
  E3SeatAuthority 만 예외로 PATH 에 스텁 cys(임시 폴더의 sh 스크립트 · 정해 둔 JSON 만 출력)를 둔다.
- PYTHONUTF8=1 — 운영 pane 이 모든 좌석에 주입하는 값(src/bin/cysd/state.rs pane 스폰 · src/lib.rs
  spawn_env_pairs)을 그대로 따른다.

## 한계
- Windows: 도구의 임시 경로 판정(`_EPHEMERAL_ROOTS`)은 POSIX 루트만 담는다. 소스로 읽으면 %TEMP% 의 artifact 가
  내구로 읽히고 selftest ⒡' 가 실패(exit 6)한다 — Windows 실측은 없다. 그래서 그 두 케이스는 os.name == "nt"
  에서 건너뛰고, 건너뜀은 사유와 함께 러너 출력에 찍힌다. 스텁 cys(sh 스크립트) 갈래도 Windows 에서 건너뛴다.

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_intake_exit_contract.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
TOOL = os.path.join(BIN, "javis_intake.py")
# 내구 artifact — 체크아웃 안의 이 파일 자신(임시 경로가 아니다).
DURABLE = os.path.abspath(__file__)

EVIDENCE = "검증 로그를 디스크에 남겼고 기대값과 실측값이 일치함을 확인했다 — exit 0 실측"
REASON = "상위 결정(헤드 판정)이 아직 내려지지 않아 착수할 수 없다"
SEAT_A = {"CYS_ROLE": "worker-a", "CYS_SURFACE_ID": "901"}
SEAT_B = {"CYS_ROLE": "worker-b", "CYS_SURFACE_ID": "902"}
NT_SKIP = ("Windows: _EPHEMERAL_ROOTS 가 POSIX 루트만 담아 %TEMP% 를 임시 경로로 보지 않는다"
           " (소스 판독 · Windows 미측정)")

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

    @unittest.skipIf(os.name == "nt", NT_SKIP)
    def test_ephemeral_only_artifact_is_refused(self):
        self.open_ledger("L1e", ["항목"])
        fd, tmpart = tempfile.mkstemp(prefix="intake-eph-", dir=tempfile.gettempdir())
        os.close(fd)
        self.addCleanup(os.unlink, tmpart)
        r = self.run_tool("pay", "--id", "L1e", "--item-no", "1", "--evidence", EVIDENCE,
                          "--artifact", tmpart)
        self.rc(EXIT_OPEN, r, "pay(임시 경로만)")
        self.assertIn("임시 경로", r.stderr)

    def test_changed_artifact_reopens(self):
        # 지불 뒤 증거 바이트가 바뀌면(덧붙임 포함) 다시 열린다 — 지문 대조. 내구 자리가 체크아웃뿐이라
        # 테스트 폴더에 점 접두 임시 파일을 만들고 지운다(점 접두 = build.rs 가 임베드에서 제외).
        fd, art = tempfile.mkstemp(prefix=".intake-fp-", suffix=".log", dir=TESTS_DIR)
        os.close(fd)
        self.addCleanup(os.unlink, art)
        with open(art, "w", encoding="utf-8") as fh:
            fh.write("검증 로그 원본\n")
        self.open_ledger("L1g", ["항목"])
        self.rc(EXIT_PAID, self.run_tool("pay", "--id", "L1g", "--item-no", "1", "--evidence", EVIDENCE,
                                         "--artifact", art), "pay")
        self.rc(EXIT_PAID, self.run_tool("status", "--id", "L1g"), "status(변경 전)")
        with open(art, "a", encoding="utf-8") as fh:
            fh.write("덧붙인 줄\n")
        r = self.run_tool("status", "--id", "L1g")
        self.rc(EXIT_OPEN, r, "status(증거 덧붙임 뒤)")
        self.assertIn("증거 바이트가 변경됐다", r.stdout)

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
    @unittest.skipIf(os.name == "nt", NT_SKIP)
    def test_selftest_passes(self):
        r = self.run_tool("selftest")
        self.rc(EXIT_PAID, r, "selftest")
        self.assertIn("SELFTEST OK", r.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
