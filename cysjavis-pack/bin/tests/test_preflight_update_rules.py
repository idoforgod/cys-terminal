#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_update_rules.py — 0.15.0 「스스로 해결」 M11 의 점검 도구 쪽 회귀 하네스.

무엇을 막는가(각 시험은 그 장치를 빼면 적색):
  ① 18개 복구 지점이 묘비(장부에 기록된 삭제)를 '누락'으로 보고 매번 init-pack 을 부르며 FAIL 을 내는 회귀
     (부트 게이트가 영영 READY 가 안 된다 — 설계 §6-11 마).
  ② 훅 묘비를 파일 복구 지점이 '허가된 부재'로 읽는 회귀(종류로 건너뛴다 — RULINGS W3(b)).
  ③ 생명줄 훅 파일(창 시작 훅)이 묘비로 부재 유지되는 회귀(언제나 되살린다 — CONTRACTS 8.7).
  ④ C62 가 user-kept·vendor-first 를 침묵하는 회귀(과제가 남은 상태 — O-C8-1).
  ⑤ 부서당 과제 하나가 실행마다 새 키로 쌓이는 회귀(멱등 키 = 레인 + 보관 집합 지문 — 부록 A-2 #15).
  ⑥ C16 이 schedule.json 을 제자리 덮어쓰기(잠금 없는 절단)로 쓰는 회귀 · 텍스트 모드 CRLF(H6(2) · §4-10).
  ⑦ 부서 일정 심기가 텍스트 모드 CRLF 로 쓰는 회귀(§6-10(6)).

라이브 무접촉: CYS_PACK_DIR·HOME·PATH 는 임시 루트 안 · `cys` 는 스텁 · wakeup 은 팩 안 스텁 스크립트.

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_preflight_update_rules.py
"""
import contextlib
import io
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
sys.path.insert(0, BIN)
sys.dont_write_bytecode = True
import javis_preflight as pf  # noqa: E402
import javis_dept_schedule as ds  # noqa: E402

PASS, WARN, FAIL, FIXED = pf.PASS, pf.WARN, pf.FAIL, pf.FIXED


class Rig(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="pf-m11-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.pack = os.path.join(self.root, "pack")
        self.bin = os.path.join(self.root, "bin")
        for d in (self.pack, self.bin, os.path.join(self.root, "home"),
                  os.path.join(self.root, "local", "repairs", "tombstones")):
            os.makedirs(d, exist_ok=True)
        with open(os.path.join(self.root, "local", "repairs", "tombstones", "base.json"), "w") as f:
            f.write("{}")
        self.calls = os.path.join(self.root, "calls.log")
        self.tombs = os.path.join(self.root, "tombs.json")
        self.life = os.path.join(self.root, "life.json")
        open(self.calls, "w").close()
        with open(self.life, "w", encoding="utf-8") as f:
            json.dump([{"rel": "hooks/session-start.sh", "lifeline": True},
                       {"rel": "hooks/role-bootstrap.sh", "lifeline": True}], f)
        self.set_tombs()
        stub = os.path.join(self.bin, "cys")
        with open(stub, "w", encoding="utf-8") as f:
            f.write("#!/bin/sh\n"
                    "printf '%%s\\n' \"cys $*\" >> %s\n"
                    "case \"$*\" in\n"
                    "  'repair tombstones --json') /bin/cat %s; exit 0 ;;\n"
                    "  'repair boot-critical --list --json') /bin/cat %s; exit 0 ;;\n"
                    "  *) exit 97 ;;\n"
                    "esac\n" % (shlex.quote(self.calls), shlex.quote(self.tombs), shlex.quote(self.life)))
        os.chmod(stub, 0o755)
        env = mock.patch.dict(os.environ, {
            "HOME": os.path.join(self.root, "home"), "PATH": self.bin, "CYS_PACK_DIR": self.pack,
        }, clear=True)
        env.start()
        self.addCleanup(env.stop)
        pf._TOMB_CACHE.clear()
        self.addCleanup(pf._TOMB_CACHE.clear)

    def set_tombs(self, files=(), hooks=()):
        items = [{"rel": r, "kind": "file", "repair": rep} for r, rep in files]
        items += [{"rel": "settings:%s:%s" % (e, s), "kind": {"hook": {"event": e, "script": s}},
                   "repair": rep} for e, s, rep in hooks]
        with open(self.tombs, "w", encoding="utf-8") as f:
            json.dump(items, f)
        pf._TOMB_CACHE.clear()

    def run_check(self, method, prefix, fix=True):
        p = pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report")
        getattr(p, method)()
        rows = [r for r in p.results if r["id"].startswith(prefix)]
        self.assertEqual(len(rows), 1, p.results)
        return rows[0]["status"], rows[0]["detail"]

    def init_pack_called(self):
        with open(self.calls, encoding="utf-8") as f:
            return any("init-pack" in line for line in f)


class RestorePoints(Rig):
    def test_tombstoned_bin_tool_is_reported_not_restored(self):
        self.set_tombs(files=[("bin/javis_report.py", "R-11")])
        st, detail = self.run_check("c15_report_tool", "C15")
        self.assertEqual(st, WARN, detail)
        self.assertIn("R-11", detail)
        self.assertFalse(self.init_pack_called(), "묘비 파일 때문에 init-pack 을 부르지 않는다")

    def test_untombstoned_missing_file_still_fails_and_tries_init_pack(self):
        st, detail = self.run_check("c15_report_tool", "C15")
        self.assertEqual(st, FAIL, detail)
        self.assertTrue(self.init_pack_called(), "기록 없는 부재는 지금처럼 복구를 시도한다")

    def test_tombstoned_config_files_c06(self):
        self.set_tombs(files=[("acl.json", "R-12")])
        with open(os.path.join(self.pack, "schedule.json"), "w", encoding="utf-8") as f:
            f.write('{"jobs": []}')
        st, detail = self.run_check("c06_json_files", "C06")
        self.assertEqual(st, WARN, detail)
        self.assertIn("acl.json", detail)
        self.assertIn("R-12", detail)
        self.assertFalse(self.init_pack_called())

    def test_hook_tombstone_does_not_authorise_a_missing_file(self):
        # pack-guard 의 '등록 해제' 묘비는 hooks/pack-guard.sh 파일의 부재를 허가하지 않는다.
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-13")])
        self.assertIsNone(pf.tombstoned_file("hooks/pack-guard.sh"))

    def test_lifeline_file_never_tombstoned(self):
        self.set_tombs(files=[("hooks/session-start.sh", "R-14"), ("hooks/cys-statusline.sh", "R-14")])
        self.assertIsNone(pf.tombstoned_file("hooks/session-start.sh"), "생명줄 파일 묘비는 무효")
        self.assertIsNone(pf.tombstoned_file("hooks/cys-statusline.sh"), "상태줄(목록 밖 생명줄)도 무효")
        st, _ = self.run_check("c07_hook_script", "C07")
        self.assertEqual(st, FAIL, "생명줄 파일 부재는 지금처럼 FAIL")
        self.assertTrue(self.init_pack_called())

    def test_no_cys_means_todays_behaviour(self):
        os.remove(os.path.join(self.bin, "cys"))
        self.set_tombs(files=[("bin/javis_report.py", "R-15")])
        self.assertIsNone(pf.tombstoned_file("bin/javis_report.py"), "cys 부재 = 묘비 없음(지금 동작)")


class LedgerKinds(Rig):
    def ledger(self, entries):
        with open(os.path.join(self.pack, ".merge-pending.json"), "w", encoding="utf-8") as f:
            json.dump(entries, f)

    def test_c62_lists_user_kept_and_vendor_first(self):
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "reason": "merge-conflict",
                                  "caution": "suspect:Empty", "version": "0.15.0", "ts": 1},
                     "hooks/b.sh": {"kind": "vendor-first", "side": "hooks/b.sh.user", "version": "0.15.0", "ts": 1}})
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(st, WARN)
        self.assertIn("user-kept", detail)
        self.assertIn("bin/a.py", detail)
        self.assertIn("suspect:Empty", detail)
        self.assertIn("vendor-first", detail)
        # ★T2g(RULINGS CN2 · CR3): 두 길과 그 조건 — 글자 그대로.
        self.assertIn("★기동 필수 vendor 우선(vendor-first) 1건 — vendor 본을 먼저 살리고 내 수정본은 <파일>.user 에 보관"
                      "(다시 입히기 = 워커가 승인 절차로: 수리로 고친 파일이면 `cys repair reapply <수리번호>`, 직접 고친 "
                      "파일이면 `cys repair begin --files <파일> --trigger update`): hooks/b.sh", detail)

    # ★T2i(RULINGS EF2 · EF3): 기록 없는 보관 사본은 원장이 없어도 · 원장이 다른 파일만 이름 대도 C62 의 목록에 보인다
    #   (글자 그대로). 원장이 그 사본을 이름 대는 종류(vendor-first)면 그 줄에만 나온다. 본 파일 없는 `.user` · 점 디렉터리
    #   안의 사본 · 링크 디렉터리 너머의 사본은 목록 밖이다.
    def test_c62_lists_a_parked_copy_without_a_record(self):
        os.makedirs(os.path.join(self.pack, "hooks"), exist_ok=True)
        for name, body in (("hooks/guard.sh", "new"), ("hooks/guard.sh.user", "mine"), ("hooks/orphan.sh.user", "x")):
            with open(os.path.join(self.pack, name), "w", encoding="utf-8") as f:
                f.write(body)
        os.makedirs(os.path.join(self.pack, ".pristine", "hooks"), exist_ok=True)
        for name in (".pristine/hooks/guard.sh", ".pristine/hooks/guard.sh.user"):
            with open(os.path.join(self.pack, name), "w", encoding="utf-8") as f:
                f.write("p")
        elsewhere = os.path.join(self.root, "elsewhere")
        os.makedirs(elsewhere)
        for name in ("x.sh", "x.sh.user"):
            with open(os.path.join(elsewhere, name), "w", encoding="utf-8") as f:
                f.write("e")
        os.symlink(elsewhere, os.path.join(self.pack, "linked"))
        # ★T2i(RULINGS FH4 — 기대가 바뀜): 원본이 없는 사본(hooks/orphan.sh.user)도 '원본 없음'으로 보인다.
        want = ("★기록 없는 보관 사본 2건 — 병합 대기 기록 없이 디스크에 남은 내 수정본 <파일>.user(`--take-new` 로 vendor 본 "
                "유지를 확정했거나 기록이 지워짐 · '원본 없음' = 그 파일이 지워졌거나 업데이트에서 빠짐 · 기동 필수·보호 목록 "
                "파일이면 사본이 있는 동안 되돌리기가 그 파일을 건너뜀 · 다시 입히기 = 워커가 승인 절차로: 수리로 고친 파일이면 "
                "`cys repair reapply <수리번호>`, 직접 고친 파일이면 `cys repair begin --files <파일> --trigger update` · 다시 "
                "입히지 않으려면 그 사본을 다른 이름으로 옮긴다(지우지 않는다)): hooks/guard.sh, hooks/orphan.sh(원본 없음)")
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual((st, detail), (WARN, want), "원장 없음")
        self.ledger({"bin/other.py": {"kind": "kept-drift", "side": "bin/other.py", "version": "x", "ts": 0}})
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(st, WARN)
        self.assertIn(want, detail, "원장이 다른 파일만 이름 댐")
        self.ledger({"hooks/guard.sh": {"kind": "vendor-first", "side": "hooks/guard.sh.user", "version": "0.15.0", "ts": 1}})
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(st, WARN)
        self.assertIn("기록 없는 보관 사본 1건", detail, "기록이 이름 대는 사본은 그 줄에만 — 원본 없는 사본은 남는다")
        self.assertNotIn("hooks/guard.sh,", detail)
        self.assertIn("vendor-first) 1건", detail)
        os.remove(os.path.join(self.pack, "hooks", "orphan.sh.user"))
        os.rename(os.path.join(self.pack, "hooks", "guard.sh.user"), os.path.join(self.pack, "hooks", "guard.sh.user.aside"))
        os.remove(os.path.join(self.pack, ".merge-pending.json"))
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual((st, detail), (PASS, "병합 대기 0건 (원장 없음)"), "사본을 옮기면 사라진다")

    # ★T2i(RULINGS FH4): 조사를 끝내지 못하면(읽지 못한 폴더 · 방문 상한) C62 는 '0건'이라고 말하지 않는다 — 원장이 없어도.
    def test_c62_unfinished_scan_never_says_zero(self):
        locked = os.path.join(self.pack, "locked")
        os.makedirs(locked)
        os.chmod(locked, 0)
        self.addCleanup(os.chmod, locked, 0o755)
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(st, WARN)
        self.assertEqual(detail, "★보관 사본 조사 미완(폴더 'locked' 를 읽지 못함(Permission denied)) — 기록 없는 보관 사본이 더 있을 "
                                 "수 있음(이 점검은 '0건'이라고 말하지 않는다)")
        os.chmod(locked, 0o755)
        with open(os.path.join(self.pack, "x.txt"), "w") as f:
            f.write("x")
        with mock.patch.object(pf, "UNRECORDED_SCAN_CAP", 0):
            st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(st, WARN)
        self.assertIn("방문 상한 0개에 닿음", detail)
        st, detail = self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual((st, detail), (PASS, "병합 대기 0건 (원장 없음)"))

    def _wake_rig(self):
        wk = os.path.join(self.pack, "bin")
        os.makedirs(wk, exist_ok=True)
        log = os.path.join(self.root, "wake.log")
        with open(os.path.join(wk, "javis_wakeup.py"), "w", encoding="utf-8") as f:
            f.write("import sys,json\nopen(%r,'a').write(json.dumps(sys.argv[1:])+'\\n')\n" % log)
        ws = os.path.join(self.root, "ws")
        os.makedirs(os.path.join(ws, "_round"), exist_ok=True)
        os.environ["JAVIS_ROOT"] = ws
        return log

    def wakes(self, log):
        if not os.path.exists(log):
            return []
        with open(log, encoding="utf-8") as f:
            return [json.loads(line) for line in f]

    def delivered(self):
        # ★T2i(RULINGS EC6): '보냄' 표지(지문마다 배타 생성) → {과제: 지문}.
        d = os.path.join(self.pack, "state", "notice-marks")
        if not os.path.isdir(d):
            return {}
        out = {}
        for n in os.listdir(d):
            if n.endswith(".sent"):
                task, fp, _s = n.rsplit(".", 2)
                out[task] = fp
        return out

    def test_parked_merge_task_one_key_per_parked_set(self):
        # ★r2(RULINGS AC7): 같은 보관 집합은 **한 번만** 적재한다(배달한 지문을 레인 상태에 둔다) — 종전 r1 은 매 점검마다
        #   적재를 시도하고 큐의 멱등 키에 기댔는데, 그 키는 대기 중일 때만 묶어 배달 뒤에는 같은 과제가 다시 왔다.
        log = self._wake_rig()
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.run_check("c62_pack_heal_ledger", "C62")
        self.run_check("c62_pack_heal_ledger", "C62")
        w = [x for x in self.wakes(log) if "parked-merge" in x]
        self.assertEqual(len(w), 1, "같은 보관 집합은 한 번만(배달 뒤 다시 적재하지 않는다)")
        k = lambda a: a[a.index("--idempotency-key") + 1]  # noqa: E731
        self.assertIn("pack", k(w[0]), "키에 레인(팩 이름)이 들어간다")
        self.assertIn(k(w[0]).rsplit("-", 1)[1], self.delivered().get("parked-merge", ""), "지문이 레인 상태에 기록된다")
        self.assertIn("master", w[0], "master 큐 — master 가 워커에게 위임")
        self.assertTrue(any("직접 병합 금지" in a for a in w[0]))
        # ★T2g(RULINGS CN2 · CR3): vendor-first 의 두 길과 그 조건 — 글자 그대로.
        self.assertTrue(any("vendor-first 는 승인 절차로 다시 입히기(수리로 고친 파일이면 cys repair reapply <수리번호>, "
                            "직접 고친 파일이면 cys repair begin --files <파일> --trigger update · 주인 승인). master 직접 "
                            "병합 금지" in a for a in w[0]), w[0])
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1},
                     "bin/c.py": {"kind": "user-kept", "side": "bin/c.py.new", "version": "0.15.0", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.run_check("c62_pack_heal_ledger", "C62")
        w = [x for x in self.wakes(log) if "parked-merge" in x]
        self.assertEqual(len(w), 2, "집합이 바뀌면 새 과제 한 번")
        self.assertNotEqual(k(w[-1]), k(w[0]))
        # ★T2i(RULINGS FH3 — 기대가 바뀜): 집합이 비어도 표지를 지우지 않는다(다른 지문의 표지를 지우지 않고 나이로만 치운다) —
        #   같은 집합이 '보냄' 뒤 `NOTICE_MARK_MAX_AGE_S` 안에 다시 오면 다시 알리지 않고(C62 WARN 은 매 부트 보인다), 그보다 오래되면
        #   다시 알린다(종전 AC7: 집합이 비면 기록을 지워 곧바로 다시 보였다).
        self.ledger({})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.assertIn("parked-merge", self.delivered(), "표지는 남는다")
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.assertEqual(len([x for x in self.wakes(log) if "parked-merge" in x]), 2, "나이 안에서는 다시 알리지 않는다")
        d = os.path.join(self.pack, "state", "notice-marks")
        for n in os.listdir(d):
            if n.endswith(".sent"):
                p = os.path.join(d, n)
                with open(p, encoding="utf-8") as f:
                    info = json.load(f)
                info["at"] = info["at"] - pf.NOTICE_MARK_MAX_AGE_S - 1
                with open(p, "w", encoding="utf-8") as f:
                    json.dump(info, f)
        self.run_check("c62_pack_heal_ledger", "C62")
        self.assertEqual(len([x for x in self.wakes(log) if "parked-merge" in x]), 3, "나이가 지난 표지는 치워지고 다시 알린다")

    def test_failed_enqueue_is_retried_not_recorded(self):
        log = self._wake_rig()
        wk = os.path.join(self.pack, "bin", "javis_wakeup.py")
        with open(wk, "w", encoding="utf-8") as f:
            f.write("import sys,json\nopen(%r,'a').write(json.dumps(sys.argv[1:])+'\\n')\nsys.exit(1)\n" % log)
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.assertEqual(self.delivered(), {}, "적재 실패는 기록하지 않는다")
        self.run_check("c62_pack_heal_ledger", "C62")
        self.assertEqual(len([x for x in self.wakes(log) if "parked-merge" in x]), 2, "다음 점검이 다시 시도한다")

    # ★T2h(RULINGS CW4) → ★T2i(RULINGS EC6 — 기대가 바뀜): 알림 기록('보내는 중' 표지)을 남길 수 없으면 **적재하지 않는다**
    #   (종전: 큐 옆 표지로 데몬 기동당 한 번 보냈다) — 쓸 수 없는 저장소에서 매 점검 되풀이 없음 · stderr 한 줄.
    def _store_unwritable(self):
        os.makedirs(os.path.join(self.pack, "state"), exist_ok=True)
        with open(os.path.join(self.pack, "state", "notice-marks"), "w") as f:
            f.write("not a directory")

    def test_parked_notice_store_unwritable_enqueues_nothing(self):
        log = self._wake_rig()
        self._store_unwritable()
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1}})
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            for _ in range(3):
                self.run_check("c62_pack_heal_ledger", "C62")
        self.assertEqual([x for x in self.wakes(log) if "parked-merge" in x], [], "기록을 못 남기면 적재하지 않는다")
        self.assertEqual(err.getvalue().count("알림 'parked-merge' 적재하지 않음"), 3, "점검마다 stderr 한 줄")

    def _stale_ledger(self):
        self.ledger({"hooks/b.sh": {"kind": "vendor-first", "side": "hooks/b.sh.user", "version": "0.15.0", "ts": 1}})

    def test_c68_notice_sent_once_per_state(self):
        log = self._wake_rig()
        self._stale_ledger()
        for _ in range(3):
            self.run_check("c68_merge_pending_age", "C68")
        w = [x for x in self.wakes(log) if "merge-review" in x]
        self.assertEqual(len(w), 1, "같은 잔존 상태는 한 번만(배달 뒤 다시 적재하지 않는다)")
        self.assertIn("merge-review", self.delivered(), "지문이 레인 상태에 기록된다")
        st, detail = self.run_check("c68_merge_pending_age", "C68")
        self.assertIn("wakeup 생략(같은 상태를 이미 보냈거나 다른 점검이 지금 보내는 중)", detail)

    def test_c68_notice_store_unwritable_enqueues_nothing(self):
        log = self._wake_rig()
        self._store_unwritable()
        self._stale_ledger()
        with contextlib.redirect_stderr(io.StringIO()):
            for _ in range(3):
                st, detail = self.run_check("c68_merge_pending_age", "C68")
        self.assertEqual([x for x in self.wakes(log) if "merge-review" in x], [], "기록을 못 남기면 적재하지 않는다")
        self.assertIn("wakeup 적재하지 않음(알림 기록을 남길 수 없음 — 기록 없이 보내면 매 점검 되풀이된다)", detail)

    def test_legacy_user_list_shown_once_per_set(self):
        log = self._wake_rig()
        with open(os.path.join(self.pack, "x.md.user"), "w") as f:
            f.write("old fix")
        self.ledger({"x.md": {"kind": "healed", "side": "x.md.user", "version": "0.14.56", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62")
        self.run_check("c62_pack_heal_ledger", "C62")
        w = [x for x in self.wakes(log) if "legacy-user-list" in x]
        self.assertEqual(len(w), 1, "예전 .user 목록은 주인에게 한 번만(RULINGS AC7)")
        self.assertTrue(any("cys pack-adopt" in a for a in w[0]), "목록 = 되살리기 명령의 모의 판정")
        self.assertIn("legacy-user-list", self.delivered())

    def test_report_mode_enqueues_nothing(self):
        log = self._wake_rig()
        self.ledger({"bin/a.py": {"kind": "user-kept", "side": "bin/a.py.new", "version": "0.15.0", "ts": 1}})
        self.run_check("c62_pack_heal_ledger", "C62", fix=False)
        self.assertEqual(self.wakes(log), [], "report 모드는 관찰만")


class NoticeMarksRealQueue(Rig):
    """★T2i(RULINGS EC6): 실제 큐(`javis_wakeup.py` enqueue)와 실제 drain(드라이런 — 배달 명령을 실행하지 않고 대기열을 비운다)으로
    — 끼어들기 지점마다 · 두 점검의 엇갈림 · 쓸 수 없는 저장소. 시계는 `pf._notice_now` 를 바꿔 움직인다(잠 없음)."""

    def setUp(self):
        super().setUp()
        self.ws = os.path.join(self.root, "ws")
        os.makedirs(os.path.join(self.ws, "_round"), exist_ok=True)
        os.environ["JAVIS_ROOT"] = self.ws
        os.environ["JAVIS_WAKEUP_LIVENESS"] = "alive"
        wk = os.path.join(self.pack, "bin")
        os.makedirs(wk, exist_ok=True)
        self.wakeup = os.path.join(wk, "javis_wakeup.py")
        with open(self.wakeup, "w", encoding="utf-8") as f:
            f.write("import runpy, sys\nsys.path.insert(0, %r)\nrunpy.run_path(%r, run_name='__main__')\n"
                    % (BIN, os.path.join(BIN, "javis_wakeup.py")))
        self.clock = [1_000_000.0]
        real_now = pf._notice_now
        pf._notice_now = lambda: self.clock[0]
        self.addCleanup(setattr, pf, "_notice_now", real_now)

    def queued(self):
        d = os.path.join(self.ws, "_round", "wakeups", "pending")
        if not os.path.isdir(d):
            return []
        out = []
        for n in sorted(os.listdir(d)):
            if n.endswith(".json"):
                with open(os.path.join(d, n), encoding="utf-8") as f:
                    out.append(json.load(f))
        return out

    def drain(self):
        r = subprocess.run([sys.executable, self.wakeup, "drain"], capture_output=True, text=True, timeout=60,
                           env=dict(os.environ))
        last = (r.stdout.strip().splitlines() or [""])[-1]
        return 0 if last == "nothing pending" else json.loads(last)["delivered"]

    def enqueue(self, task, fp):
        return lambda: pf._wakeup_enqueue(self.wakeup, task, "ec6 notice", "%s-%s" % (task, fp))

    def mark(self, task, fp, state):
        return os.path.join(self.pack, "state", "notice-marks", "%s.%s.%s" % (task, fp, state))

    def test_normal_once_then_drain(self):
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "already")
        self.assertEqual(len(self.queued()), 1)
        self.assertEqual(self.drain(), 1)
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "already", "배달 뒤에도 다시 적재하지 않는다")
        self.assertEqual(self.drain(), 0)
        self.assertTrue(os.path.exists(self.mark("t", "f1", "pending")) and os.path.exists(self.mark("t", "f1", "sent")))

    def test_interrupted_before_enqueue(self):
        # '보내는 중'만 남기고 멈춘 점검(적재 전) — 방금이면 다른 점검이 보내는 중으로 보고 건드리지 않고, 시간이 지난 뒤의
        # 점검이 같은 키로 적재한다(알림을 잃지 않는다).
        pf._notice_create_mark(self.mark("t", "f1", "pending"))
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "already")
        self.assertEqual(self.queued(), [])
        self.clock[0] += pf.NOTICE_PENDING_STALE_S
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        self.assertEqual(self.drain(), 1)

    def test_interrupted_between_acceptance_and_sent(self):
        # 큐가 받은 뒤 '보냄' 전에 멈춤 → 다음 점검이 같은 키로 다시 적재: 아직 대기 중이면 큐가 묶는다(배달 1회).
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        os.remove(self.mark("t", "f1", "sent"))
        self.clock[0] += pf.NOTICE_PENDING_STALE_S
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        self.assertEqual(len(self.queued()), 1, "대기 중인 같은 키는 묶인다")
        self.assertEqual(self.drain(), 1)
        # 이미 배달된 뒤에 멈춘 경우 → 최악은 한 번 더(그 뒤로는 '보냄'이 있어 되풀이 없음).
        os.remove(self.mark("t", "f1", "sent"))
        self.clock[0] += pf.NOTICE_PENDING_STALE_S
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        self.assertEqual(self.drain(), 1, "한 번 더 배달(최악)")
        for _ in range(3):
            self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "already")
        self.assertEqual(self.drain(), 0, "되풀이 없음")

    def test_two_concurrent_preflights_enqueue_once(self):
        # A 가 '보내는 중'을 만든 뒤 적재하기 직전에 B 가 끝까지 돈다 → B 는 적재하지 않는다(배타 생성이 적재할 쪽을 정한다).
        seen = []

        def a_enqueue():
            seen.append(pf._notice_once("t", "f1", self.enqueue("t", "f1")))
            return self.enqueue("t", "f1")()

        self.assertEqual(pf._notice_once("t", "f1", a_enqueue), "sent")
        self.assertEqual(seen, ["already"])
        # A 가 적재한 뒤 '보냄' 전에 B 가 돈다 → 역시 적재하지 않는다.
        seen.clear()

        def a_enqueue_then_b():
            ok = self.enqueue("t", "f2")()
            seen.append(pf._notice_once("t", "f2", self.enqueue("t", "f2")))
            return ok

        self.assertEqual(pf._notice_once("t", "f2", a_enqueue_then_b), "sent")
        self.assertEqual(seen, ["already"])
        q = self.queued()
        self.assertEqual(len(q), 1, "한 과제(t)의 대기 기록 하나")
        self.assertEqual(q[0]["idempotency_keys"], ["t-f1", "t-f2"], "각 지문은 한 번씩만 적재됐다")
        self.assertEqual(q[0]["coalesced_count"], 1)
        self.assertEqual(self.drain(), 1)

    def test_unwritable_store_enqueues_nothing(self):
        os.makedirs(os.path.join(self.pack, "state"), exist_ok=True)
        with open(os.path.join(self.pack, "state", "notice-marks"), "w") as f:
            f.write("x")
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            for _ in range(3):
                self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "unrecorded")
        self.assertEqual(self.queued(), [])
        self.assertEqual(err.getvalue().count("적재하지 않음"), 3)

    def test_marks_folder_that_is_not_a_folder_is_not_read_as_pending(self):
        # ★T2i(EC6 수정 — 적색 실행이 찾음): 표지 폴더 자리가 **파일**이면 `makedirs` 가 FileExistsError 를 냈고, 그것이 '보내는
        #   중 표지가 이미 있음'으로 읽혀 기록 없이 매 점검 다시 적재했다. 폴더 자리가 파일 · 파일을 가리키는 링크면 '기록 불가'다.
        st = os.path.join(self.pack, "state")
        os.makedirs(st, exist_ok=True)
        marks = os.path.join(st, "notice-marks")
        other = os.path.join(self.root, "a-file")
        with open(other, "w") as f:
            f.write("x")
        for form in ("file", "link-to-file"):
            if os.path.lexists(marks):
                os.remove(marks)
            if form == "file":
                with open(marks, "w") as f:
                    f.write("x")
            else:
                os.symlink(other, marks)
            with self.assertRaises(NotADirectoryError, msg=form):
                pf._notice_create_mark(self.mark("t", "f1", "pending"))
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "unrecorded", form)
            self.assertEqual(self.queued(), [], form)

    # ★T2i(RULINGS FH3): 적재마다 자기 표지 · 다시 적재는 번호로 묶음 — 한 지문의 배달은 저장소가 어떻든 N + 1 번 이하.
    def test_fh3_sent_never_writable_is_bounded(self):
        os.makedirs(self.mark("t", "f1", "sent"))   # '보냄' 자리가 폴더 — 만들 수 없고, '보냄'으로 세지도 않는다
        delivered = 0
        outcomes = []
        with contextlib.redirect_stderr(io.StringIO()):
            for _ in range(pf.NOTICE_RETRY_MAX + 4):
                outcomes.append(pf._notice_once("t", "f1", self.enqueue("t", "f1")))
                delivered += self.drain()
                self.clock[0] += pf.NOTICE_PENDING_STALE_S
        self.assertEqual(delivered, pf.NOTICE_RETRY_MAX + 1, outcomes)
        self.assertEqual(outcomes[:pf.NOTICE_RETRY_MAX + 1], ["sent"] * (pf.NOTICE_RETRY_MAX + 1))
        self.assertEqual(set(outcomes[pf.NOTICE_RETRY_MAX + 1:]), {"exhausted"}, "그 뒤로는 조용하다")

    def test_fh3_folder_turns_read_only_after_the_first_enqueue(self):
        d = os.path.join(self.pack, "state", "notice-marks")

        def enqueue_then_lock():
            ok = self.enqueue("t", "f1")()
            os.chmod(d, 0o555)
            return ok

        self.addCleanup(lambda: os.path.isdir(d) and os.chmod(d, 0o755))
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(pf._notice_once("t", "f1", enqueue_then_lock), "sent")
            self.assertEqual(self.drain(), 1)
            for _ in range(3):
                self.clock[0] += pf.NOTICE_PENDING_STALE_S
                self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "unrecorded")
        self.assertEqual(self.drain(), 0, "다시 적재의 표지를 만들 수 없으면 적재하지 않는다")

    def test_fh3_two_stale_retries_at_once_enqueue_once(self):
        pf._notice_create_mark(self.mark("t", "f1", "pending"), "f1")
        self.clock[0] += pf.NOTICE_PENDING_STALE_S
        seen = []

        def a_enqueue():
            seen.append(pf._notice_once("t", "f1", self.enqueue("t", "f1")))
            return self.enqueue("t", "f1")()

        self.assertEqual(pf._notice_once("t", "f1", a_enqueue), "sent")
        self.assertEqual(seen, ["already"], "B 는 A 의 retry-1 표지를 보고 적재하지 않는다")
        self.assertTrue(os.path.exists(self.mark("t", "f1", "retry-1")))
        self.assertEqual(self.drain(), 1)

    def test_fh3_two_fingerprints_keep_each_others_marks(self):
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        self.assertEqual(pf._notice_once("t", "f2", self.enqueue("t", "f2")), "sent")
        for fp in ("f1", "f2"):
            self.assertTrue(os.path.isfile(self.mark("t", fp, "pending")) and os.path.isfile(self.mark("t", fp, "sent")), fp)
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "already")

    def test_fh3_non_regular_sent_is_not_sent(self):
        os.makedirs(os.path.join(self.pack, "state", "notice-marks"), exist_ok=True)
        os.symlink(os.path.join(self.root, "nowhere"), self.mark("t", "f1", "sent"))   # 끊긴 링크
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent", "한 번도 받지 않은 알림은 보낸다")
        self.assertEqual(len(self.queued()), 1)
        with open(self.mark("t", "f2", "sent"), "w", encoding="utf-8") as f:
            json.dump({"fp": "other"}, f)   # 다른 지문을 담은 '보냄' = 보냄 아님
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(pf._notice_once("t", "f2", lambda: True), "sent")

    def test_fh3_marks_are_removed_by_age_only(self):
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")
        pf._notice_create_mark(self.mark("t", "f9", "pending"), "f9")       # '보냄' 없는 지문
        self.clock[0] += pf.NOTICE_MARK_MAX_AGE_S + 1
        pf._notice_gc()
        self.assertFalse(os.path.exists(self.mark("t", "f1", "sent")), "'보냄'이 오래된 지문의 표지는 치운다")
        self.assertFalse(os.path.exists(self.mark("t", "f1", "pending")))
        self.assertTrue(os.path.exists(self.mark("t", "f9", "pending")), "'보냄' 없는 지문의 표지는 남긴다(횟수 상한)")

    def test_failed_enqueue_leaves_no_mark(self):
        self.assertEqual(pf._notice_once("t", "f1", lambda: False), "failed")
        self.assertFalse(os.path.exists(self.mark("t", "f1", "pending")), "다음 점검이 곧바로 다시 시도한다")
        self.assertEqual(pf._notice_once("t", "f1", self.enqueue("t", "f1")), "sent")

    def test_result_sentences(self):
        self.assertEqual(set(pf.NOTICE_RESULT_SENTENCES), {"sent", "already", "unrecorded", "exhausted", "failed"})
        self.assertEqual(pf.NOTICE_RESULT_SENTENCES["exhausted"],
                         "wakeup 적재하지 않음(같은 알림을 이미 정해진 횟수만큼 적재했으나 '보냄' 기록이 남지 않음 — 더 보내지 않는다)")
        self.assertEqual(pf.NOTICE_RESULT_SENTENCES["unrecorded"],
                         "wakeup 적재하지 않음(알림 기록을 남길 수 없음 — 기록 없이 보내면 매 점검 되풀이된다)")


class ScheduleWriters(Rig):
    def _spy_fdopen(self):
        seen = []
        real = os.fdopen

        def spy(fd, *a, **kw):
            seen.append(kw.get("newline"))
            return real(fd, *a, **kw)
        return seen, spy

    def test_c16_writes_atomically_with_lf_and_keeps_mode(self):
        p = os.path.join(self.pack, "schedule.json")
        with open(p, "w", encoding="utf-8") as f:
            f.write('{"jobs": []}')
        os.chmod(p, 0o640)
        ino = os.stat(p).st_ino
        seen, spy = self._spy_fdopen()
        with mock.patch.object(pf.os, "fdopen", spy):
            st, detail = self.run_check("c16_report_schedule", "C16")
        self.assertEqual(st, FIXED, detail)
        self.assertIn("\n", seen, "임시 파일을 LF(newline='\\n')로 쓴다")
        self.assertNotEqual(os.stat(p).st_ino, ino, "제자리 절단이 아니라 임시 파일 + 원자 교체")
        self.assertEqual(os.stat(p).st_mode & 0o777, 0o640, "원본 권한 유지")
        data = json.load(open(p, encoding="utf-8"))
        self.assertTrue(any(j.get("id") == "owner-progress-gate-5min" for j in data["jobs"]))
        self.assertNotIn(b"\r", open(p, "rb").read())
        leftovers = [n for n in os.listdir(self.pack) if n.startswith(".schedule.")]
        self.assertEqual(leftovers, [], "임시 파일 잔재 없음")

    def test_dept_schedule_seed_writes_lf(self):
        p = os.path.join(self.root, "dept-schedule.json")
        seen, spy = self._spy_fdopen()
        with mock.patch.object(ds.os, "fdopen", spy):
            self.assertEqual(ds.seed(p), 0)
        self.assertEqual(seen, ["\n"], "부서 일정 심기는 LF")
        self.assertNotIn(b"\r", open(p, "rb").read())


if __name__ == "__main__":
    unittest.main(verbosity=2)
