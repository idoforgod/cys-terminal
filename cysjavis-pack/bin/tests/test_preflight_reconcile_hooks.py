#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_preflight_reconcile_hooks.py — 0.15.0 「스스로 해결」 M11 마 · RULINGS X1/Y2 회귀 하네스.

무엇을 막는가(각 시험은 그 장치를 빼면 적색):
  ① 묘비(장부에 기록된 해제)가 있는 생명줄이 아닌 훅이 좁은 모드 뒤에도 어느 대상에 남는 회귀.
  ② 묘비를 지웠는데(되돌림) 다시 등록되지 않는 회귀.
  ③ 생명줄 훅(창 시작·팀 기동·상태 주입·상태 저장·상태줄)이 묘비 때문에 빠지는 회귀(F2/F3).
  ④ 좁은 모드가 다른 점검·init-pack·repair.sweep 을 부르는 회귀.
  ⑤ --dry-run 이 쓰는 회귀 · 훅 스크립트 부재를 init-pack 으로 '고치는' 회귀(Y2(d)).
  ⑥ 심볼릭 링크 settings 를 고쳐 쓰는 회귀(refused:<사유> 로 보고해야 한다).
  ⑦ 평소 점검이 잠금을 쥔 사이 데몬이 쓴 묘비를 그 대상의 결정이 못 보는 회귀(Y2(e) — 잠금 안 재읽기).
  ⑧ 출력이 Y2(a) 스키마와 어긋나는 회귀(T6 가 파싱한다).
  ⑨ 평소 C33·C28 이 묘비 훅을 다시 등록하는 회귀.

라이브 무접촉: HOME·PATH·CYS_PACK_DIR·TMPDIR 전부 임시 루트 안이고 `cys` 는 인자 검증 스텁이다(예상 밖 동사 =
exit 97 + 기록). 묘비·생명줄 목록은 스텁이 파일에서 읽어 답한다(CONTRACTS O-C1-8 · O-C8-6 의 로컬 질의 모양).

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_preflight_reconcile_hooks.py
"""
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, BIN)
sys.dont_write_bytecode = True
import javis_preflight as pf  # noqa: E402

PRODUCT_HOOK_SCRIPTS = (
    "session-start.sh", "cys-statusline.sh", "cys-hook.sh", "inject-context.sh", "save-state.sh",
    "reflect-scan.sh", "commit-memory-nudge.sh", "role-bootstrap.sh", "pack-guard.sh",
    "role-capability-gate.sh", "appbuild-gate.sh", "grill-gate.sh", "grill-count.sh",
    "grill-arm.sh", "grill-stop.sh",
)
# 컴파일된 목록이 답할 생명줄(파일 항목의 lifeline 플래그 — 상태줄은 목록 밖: O-C4-4).
LIFELINE_FILES = ("session-start.sh", "role-bootstrap.sh", "inject-context.sh", "save-state.sh",
                  "role-capability-gate.sh")


def _non_temp_base():
    home = os.path.expanduser("~")
    if home and os.path.isdir(home) and not pf._path_under_tempdir(home):
        return home
    return None


class Rig(unittest.TestCase):
    def setUp(self):
        base = _non_temp_base()
        self.root = tempfile.mkdtemp(prefix="pf-reconcile-", dir=base)
        self.addCleanup(shutil.rmtree, self.root, True)
        self.home = os.path.join(self.root, "home")
        self.bin = os.path.join(self.root, "bin")
        self.pack = os.path.join(self.root, "pack")
        for d in (self.home, self.bin, os.path.join(self.pack, "hooks"),
                  os.path.join(self.root, "tmp"),
                  os.path.join(self.root, "local", "repairs", "tombstones"),
                  os.path.join(self.home, ".claude"), os.path.join(self.home, ".claude-2")):
            os.makedirs(d, exist_ok=True)
        for s in PRODUCT_HOOK_SCRIPTS:
            p = os.path.join(self.pack, "hooks", s)
            with open(p, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\nexit 0\n")
            os.chmod(p, 0o755)
        # 묘비 파일의 존재 탐침(_tombstones_may_exist)이 참이 되게 — 내용은 스텁이 답한다.
        with open(os.path.join(self.root, "local", "repairs", "tombstones", "base.json"), "w") as f:
            f.write("{}")
        self.calls = os.path.join(self.root, "calls.log")
        self.tombs = os.path.join(self.root, "tombs.json")
        self.life = os.path.join(self.root, "life.json")
        open(self.calls, "w").close()
        self.set_tombs([])
        with open(self.life, "w", encoding="utf-8") as f:
            json.dump([{"rel": "hooks/" + n, "category": "hook", "lifeline": True} for n in LIFELINE_FILES]
                      + [{"rel": "hooks/pack-guard.sh", "category": "hook", "lifeline": False},
                         {"rel": "bin/javis_bootstrap.py", "category": "start-script", "lifeline": False}], f)
        stub = os.path.join(self.bin, "cys")
        with open(stub, "w", encoding="utf-8") as f:
            f.write("#!/bin/sh\n"
                    "printf '%%s\\n' \"cys $*\" >> %s\n"
                    "case \"$*\" in\n"
                    "  'repair tombstones --json') /bin/cat %s; exit 0 ;;\n"
                    "  'repair boot-critical --list --json') /bin/cat %s; exit 0 ;;\n"
                    "  *) echo 'unexpected stub arguments' >&2; exit 97 ;;\n"
                    "esac\n" % (shlex.quote(self.calls), shlex.quote(self.tombs), shlex.quote(self.life)))
        os.chmod(stub, 0o755)
        os.symlink(sys.executable, os.path.join(self.bin, "python3"))
        env = mock.patch.dict(os.environ, {
            "HOME": self.home, "PATH": self.bin, "CYS_PACK_DIR": self.pack,
            "TMPDIR": os.path.join(self.root, "tmp"),
        }, clear=True)
        env.start()
        self.addCleanup(env.stop)
        if base is None:   # 루트가 임시 디렉터리 아래일 수밖에 없는 실행기 — 임시 팩 금지만 이 시험에서 끈다
            p = mock.patch.object(pf, "_path_under_tempdir", lambda _p: False)
            p.start()
            self.addCleanup(p.stop)
        pf._TOMB_CACHE.clear()
        self.addCleanup(pf._TOMB_CACHE.clear)
        self.targets = [os.path.join(self.home, ".claude", "settings.json"),
                        os.path.join(self.home, ".claude-2", "settings.json")]

    # ── 도우미 ──
    def set_tombs(self, hooks=(), files=()):
        items = [{"rel": "settings:%s:%s" % (ev, sc), "kind": {"hook": {"event": ev, "script": sc}},
                  "repair": rep} for ev, sc, rep in hooks]
        items += [{"rel": rel, "kind": "file", "repair": rep} for rel, rep in files]
        with open(self.tombs, "w", encoding="utf-8") as f:
            json.dump(items, f)
        pf._TOMB_CACHE.pop("tombs", None)

    def pf_fix(self):
        return pf.Preflight(fix=True, skips=[], mode="fix")

    def present(self, t, ev, sc):
        return pf.Preflight._event_hook_present_any(t, ev, sc)

    def reconcile(self, dry_run=False):
        pf._TOMB_CACHE.pop("tombs", None)
        return self.pf_fix().reconcile_hooks(dry_run=dry_run) if not dry_run else \
            pf.Preflight(fix=False, skips=[]).reconcile_hooks(dry_run=True)

    def row(self, rep, ev, sc):
        rows = [h for h in rep["hooks"] if h["event"] == ev and h["script"] == sc]
        self.assertEqual(len(rows), 1, "훅 행 %s/%s 정확히 1개" % (ev, sc))
        return rows[0]

    def calls_seen(self):
        with open(self.calls, encoding="utf-8") as f:
            return f.read().splitlines()


class ReconcileMode(Rig):
    def test_tombstoned_non_lifeline_hook_absent_on_every_target(self):
        p = self.pf_fix()
        for t in self.targets:
            self.assertIsNone(p._register_event_hook(t, "PostToolUse", "pack-guard.sh", "Write|Edit|MultiEdit"))
            self.assertTrue(self.present(t, "PostToolUse", "pack-guard.sh"))
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        rep = self.reconcile()
        r = self.row(rep, "PostToolUse", "pack-guard.sh")
        self.assertFalse(r["lifeline"])
        self.assertEqual(r["desired"], "unregistered")
        self.assertEqual([x["state"] for x in r["targets"]], ["unregistered"] * 2)
        for t in self.targets:
            self.assertFalse(self.present(t, "PostToolUse", "pack-guard.sh"), "묘비 훅이 %s 에 남았다" % t)

    def test_tombstone_removed_then_registered_again(self):
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        self.reconcile()
        for t in self.targets:
            self.assertFalse(self.present(t, "PostToolUse", "pack-guard.sh"))
        self.set_tombs([])   # 되돌림 = 묘비 제거
        rep = self.reconcile()
        r = self.row(rep, "PostToolUse", "pack-guard.sh")
        self.assertEqual(r["desired"], "registered")
        for t in self.targets:
            self.assertTrue(self.present(t, "PostToolUse", "pack-guard.sh"), "되돌린 뒤 다시 등록돼야 한다")

    def test_lifeline_answer_is_unknown_unless_all_five_names_are_present(self):
        # ★RULINGS AC7: 목록이 다섯 생명줄 중 하나라도 생명줄로 답하지 않으면 '모름' = 어떤 훅 묘비도 적용하지 않는다
        #   (그 하나가 묘비로 벗겨지는 일이 없게). 다섯이 다 있으면 다섯을 이름으로 지킨다.
        with open(self.life, "w", encoding="utf-8") as f:
            json.dump([{"rel": "hooks/" + n, "category": "hook", "lifeline": n != "role-capability-gate.sh"}
                       for n in LIFELINE_FILES], f)
        pf._TOMB_CACHE.clear()
        self.assertIsNone(pf.lane_lifeline_scripts(), "다섯 중 하나가 빠지면 모름")
        self.set_tombs(hooks=[("PreToolUse", "role-capability-gate.sh", "R-5"),
                              ("PostToolUse", "pack-guard.sh", "R-6")])
        self.assertIsNone(pf.tombstoned_hook("PreToolUse", "role-capability-gate.sh"), "모름 → 묘비 미적용")
        self.assertIsNone(pf.tombstoned_hook("PostToolUse", "pack-guard.sh"), "모름 → 어떤 훅 묘비도 미적용")
        with open(self.life, "w", encoding="utf-8") as f:
            json.dump([{"rel": "hooks/" + n, "category": "hook", "lifeline": True} for n in LIFELINE_FILES], f)
        pf._TOMB_CACHE.clear()
        self.assertEqual(set(pf.lane_lifeline_scripts()), set(LIFELINE_FILES))
        self.set_tombs(hooks=[("PreToolUse", "role-capability-gate.sh", "R-5"),
                              ("PostToolUse", "pack-guard.sh", "R-6")])
        self.assertIsNone(pf.tombstoned_hook("PreToolUse", "role-capability-gate.sh"), "생명줄은 이름으로 지킨다")
        self.assertEqual(pf.tombstoned_hook("PostToolUse", "pack-guard.sh"), "R-6", "생명줄 아닌 훅 묘비는 적용")

    def test_lifelines_never_removed_whatever_the_tombstones_say(self):
        self.set_tombs(hooks=[("SessionStart", "session-start.sh", "R-1"),
                              ("SessionStart", "inject-context.sh", "R-1"),
                              ("Stop", "save-state.sh", "R-1"),
                              ("UserPromptSubmit", "role-bootstrap.sh", "R-1"),
                              ("statusLine", "cys-statusline.sh", "R-1")])
        rep = self.reconcile()
        for ev, sc in (("SessionStart", "session-start.sh"), ("SessionStart", "inject-context.sh"),
                       ("Stop", "save-state.sh"), ("UserPromptSubmit", "role-bootstrap.sh")):
            r = self.row(rep, ev, sc)
            self.assertTrue(r["lifeline"], "%s 는 생명줄" % sc)
            self.assertEqual(r["desired"], "registered")
            for t in self.targets:
                self.assertTrue(self.present(t, ev, sc), "생명줄 %s 가 %s 에서 빠졌다" % (sc, t))
        sl = self.row(rep, "statusLine", "cys-statusline.sh")
        self.assertTrue(sl["lifeline"])
        self.assertEqual([x["state"] for x in sl["targets"]], ["registered"] * 2)

    def test_mode_runs_no_other_check_no_sweep_no_init_pack(self):
        sched = os.path.join(self.pack, "schedule.json")
        with open(sched, "w", encoding="utf-8") as f:
            f.write('{"jobs": []}')
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        rep = self.reconcile()
        self.assertEqual(set(rep), {"hooks", "dry_run", "pack_dir", "forbidden", "missing_scripts", "errors"})
        calls = self.calls_seen()
        self.assertTrue(calls, "묘비·생명줄은 로컬 질의로만 읽는다")
        for c in calls:
            self.assertIn(c, ("cys repair tombstones --json", "cys repair boot-critical --list --json"),
                          "좁은 모드는 다른 cys 동사를 부르지 않는다(init-pack·repair sweep 금지): %s" % c)
        with open(sched, encoding="utf-8") as f:
            self.assertEqual(f.read(), '{"jobs": []}', "다른 점검(C16 등)은 돌지 않는다")

    def test_dry_run_writes_nothing(self):
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        p = self.pf_fix()
        p._register_event_hook(self.targets[0], "PostToolUse", "pack-guard.sh", "Write|Edit|MultiEdit")
        before = {t: (open(t, "rb").read() if os.path.exists(t) else None) for t in self.targets}
        rep = self.reconcile(dry_run=True)
        self.assertTrue(rep["dry_run"])
        after = {t: (open(t, "rb").read() if os.path.exists(t) else None) for t in self.targets}
        self.assertEqual(before, after, "--dry-run 은 아무것도 쓰지 않는다")
        r = self.row(rep, "PostToolUse", "pack-guard.sh")
        self.assertEqual([x["state"] for x in r["targets"]], ["registered", "unregistered"], "보고는 현재 상태")

    def test_missing_hook_script_reported_no_init_pack(self):
        os.remove(os.path.join(self.pack, "hooks", "commit-memory-nudge.sh"))
        rep = self.reconcile()
        self.assertIn("commit-memory-nudge.sh", rep["missing_scripts"])
        for t in self.targets:
            self.assertFalse(self.present(t, "PostToolUse", "commit-memory-nudge.sh"),
                             "스크립트가 없으면 등록하지 않는다(보고만)")
        self.assertFalse(any("init-pack" in c for c in self.calls_seen()), "init-pack 을 부르지 않는다")
        self.assertFalse(os.path.exists(os.path.join(self.pack, "hooks", "commit-memory-nudge.sh")))

    def test_symlinked_settings_refused(self):
        real = os.path.join(self.root, "real-settings.json")
        with open(real, "w", encoding="utf-8") as f:
            f.write("{}")
        link = self.targets[1]
        os.symlink(real, link)
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        rep = self.reconcile()
        for h in rep["hooks"]:
            st = [x["state"] for x in h["targets"] if x["path"] == link]
            self.assertEqual(st, ["refused:symlink"], "%s/%s" % (h["event"], h["script"]))
        with open(real, encoding="utf-8") as f:
            self.assertEqual(f.read(), "{}", "링크 너머 실파일을 고쳐 쓰지 않는다")

    def test_tombstone_written_while_ordinary_run_holds_lock_for_another_target(self):
        # 평소 C28 이 대상 A 에 등록한 뒤 대상 B 의 잠금을 잡는 순간 데몬이 묘비를 쓴다 → B 는 새 묘비로 결정.
        orig = pf._settings_rmw
        tb = self.targets[1]

        def racing(path, mutate, indent=2):
            if path == tb:
                self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-5")])
            return orig(path, mutate, indent)

        with mock.patch.object(pf, "_settings_rmw", racing):
            self.pf_fix().c28_self_correction()
        self.assertTrue(self.present(self.targets[0], "PostToolUse", "pack-guard.sh"),
                        "묘비 전에 결정된 A 는 등록됨(데몬의 조정이 곧 해제한다)")
        self.assertFalse(self.present(tb, "PostToolUse", "pack-guard.sh"),
                         "B 는 잠금 안에서 새 묘비를 보고 등록하지 않는다(Y2(e))")
        self.reconcile()   # 데몬이 묘비를 쓴 뒤 돌리는 조정 — 어떤 순서로 겹쳐도 끝은 해제
        for t in self.targets:
            self.assertFalse(self.present(t, "PostToolUse", "pack-guard.sh"))

    def test_registrar_rereads_tombstones_inside_the_lock(self):
        # 사전 읽기(캐시)는 '묘비 없음'인데 그 뒤 데몬이 묘비를 쓴다 → 등록기는 잠금 안에서 다시 읽고 등록하지 않는다.
        self.assertEqual(pf.lane_tombstones(), ({}, {}))
        with open(self.tombs, "w", encoding="utf-8") as f:   # 캐시는 그대로(오래된 사전 읽기)
            json.dump([{"rel": "settings:PostToolUse:pack-guard.sh",
                        "kind": {"hook": {"event": "PostToolUse", "script": "pack-guard.sh"}},
                        "repair": "R-9"}], f)
        err = self.pf_fix()._register_event_hook(self.targets[0], "PostToolUse", "pack-guard.sh",
                                                 "Write|Edit|MultiEdit", respect_tombstone=True)
        self.assertEqual(err, pf._REG_TOMBSTONED, "잠금 안 재읽기가 묘비를 보고 등록을 거부해야 한다")
        self.assertFalse(self.present(self.targets[0], "PostToolUse", "pack-guard.sh"))
        # 생명줄은 같은 상황에서도 등록된다.
        with open(self.tombs, "w", encoding="utf-8") as f:
            json.dump([{"rel": "settings:SessionStart:inject-context.sh",
                        "kind": {"hook": {"event": "SessionStart", "script": "inject-context.sh"}},
                        "repair": "R-9"}], f)
        self.assertIsNone(self.pf_fix()._register_event_hook(self.targets[0], "SessionStart", "inject-context.sh",
                                                             None, respect_tombstone=True))
        self.assertTrue(self.present(self.targets[0], "SessionStart", "inject-context.sh"))

    def test_unregister_keeps_entry_if_tombstone_vanished_inside_the_lock(self):
        p = self.pf_fix()
        p._register_event_hook(self.targets[0], "PostToolUse", "pack-guard.sh", "Write|Edit|MultiEdit")
        self.set_tombs([])  # 되돌림이 묘비를 지운 뒤 — 묘비를 근거로 한 해제는 아무것도 빼지 않는다
        self.assertIsNone(p._unregister_event_hook(self.targets[0], "PostToolUse", "pack-guard.sh",
                                                   require_tombstone=True))
        self.assertTrue(self.present(self.targets[0], "PostToolUse", "pack-guard.sh"))

    def test_json_matches_y2a_schema_via_cli(self):
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        env = dict(os.environ)
        r = subprocess.run([sys.executable, os.path.join(BIN, "javis_preflight.py"),
                            "--reconcile-hooks", "--json"],
                           capture_output=True, encoding="utf-8", env=env, timeout=120)
        self.assertEqual(r.returncode, 0, r.stderr)
        rep = json.loads(r.stdout)
        self.assertIsInstance(rep["hooks"], list)
        self.assertTrue(rep["hooks"])
        for h in rep["hooks"]:
            self.assertEqual(set(h), {"event", "script", "lifeline", "desired", "targets"})
            self.assertIsInstance(h["event"], str)
            self.assertIsInstance(h["script"], str)
            self.assertIsInstance(h["lifeline"], bool)
            self.assertIn(h["desired"], ("registered", "unregistered", "unknown"))
            for t in h["targets"]:
                self.assertEqual(set(t), {"path", "state"})
                self.assertTrue(t["state"] in ("registered", "unregistered") or t["state"].startswith("refused:"))
        # 제품이 등록하는 훅 목록 그대로(P1): 각 등록 점검의 훅이 다 있다.
        names = {(h["event"], h["script"]) for h in rep["hooks"]}
        for need in (("SessionStart", "session-start.sh"), ("statusLine", "cys-statusline.sh"),
                     ("PreToolUse", "cys-hook.sh"), ("PostToolUse", "pack-guard.sh"),
                     ("PreToolUse", "role-capability-gate.sh"), ("PreToolUse", "appbuild-gate.sh"),
                     ("PostToolUse", "grill-count.sh")):
            self.assertIn(need, names)

    def test_managed_style_dry_run_flag_parses(self):
        rc = pf._reconcile_hooks_main(["--reconcile-hooks", "--dry-run"])
        self.assertEqual(rc, 0)


class OrdinaryRuns(Rig):
    def test_c33_unregisters_only_the_tombstoned_event_and_never_readds(self):
        p = self.pf_fix()
        for t in self.targets:
            for ev in p.EVENT_HOOK_EVENTS:
                p._register_event_hook(t, ev, p.EVENT_HOOK, matcher="")
        self.set_tombs(hooks=[("PreToolUse", "cys-hook.sh", "R-6")])
        for _ in range(2):
            pf._TOMB_CACHE.clear()
            self.pf_fix().c33_event_hooks()
            for t in self.targets:
                self.assertFalse(self.present(t, "PreToolUse", "cys-hook.sh"))
                self.assertTrue(self.present(t, "PostToolUse", "cys-hook.sh"))
                self.assertTrue(self.present(t, "PermissionRequest", "cys-hook.sh"))

    def test_c28_unregisters_tombstoned_pack_guard_keeps_lifelines(self):
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3"),
                              ("SessionStart", "inject-context.sh", "R-3")])
        for _ in range(2):
            pf._TOMB_CACHE.clear()
            self.pf_fix().c28_self_correction()
            for t in self.targets:
                self.assertFalse(self.present(t, "PostToolUse", "pack-guard.sh"), "묘비 훅 재등록 금지")
                self.assertTrue(self.present(t, "SessionStart", "inject-context.sh"), "생명줄은 묘비 무시")
                self.assertTrue(self.present(t, "PostToolUse", "commit-memory-nudge.sh"), "묘비 없는 훅은 지금처럼")

    def test_no_tombstone_file_means_no_cys_query_and_todays_behaviour(self):
        shutil.rmtree(os.path.join(self.root, "local"))
        pf._TOMB_CACHE.clear()
        self.pf_fix().c28_self_correction()
        self.assertFalse(any(c.startswith("cys repair") for c in self.calls_seen()),
                         "묘비 파일이 없으면 cys 를 띄우지 않는다(수리를 하지 않은 설치는 지금과 같다)")
        for t in self.targets:
            self.assertTrue(self.present(t, "PostToolUse", "pack-guard.sh"))

    def test_unknown_lifelines_apply_no_hook_tombstone(self):
        with open(self.life, "w") as f:
            f.write("not json")
        self.set_tombs(hooks=[("PostToolUse", "pack-guard.sh", "R-3")])
        pf._TOMB_CACHE.clear()
        self.pf_fix().c28_self_correction()
        for t in self.targets:
            self.assertTrue(self.present(t, "PostToolUse", "pack-guard.sh"),
                            "생명줄 목록을 못 읽으면 어떤 훅 묘비도 적용하지 않는다(지금 동작)")
        rep = self.reconcile()
        self.assertEqual(self.row(rep, "PostToolUse", "pack-guard.sh")["desired"], "unknown")


if __name__ == "__main__":
    unittest.main(verbosity=2)
