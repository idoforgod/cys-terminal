#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_trust_seed_reflect.py — 성찰 반영(2026-09-10) P2·P6·P12·P18 의 회귀 핀.

라이브 무접촉: 모든 config dir·cwd·레지스트리는 임시 디렉터리다(실 ~/.cys 계정 dir 은 절대 대상이 아니다).

  P2 [blocking] 신뢰 시드 삭제 경로가 원본의 **마지막 사본**을 제거했다 — 세 경로가 삭제 시점의 공용 증명을 공유한다:
     ⓐ 스윕: 계획 계산 **후** 외부가 활성을 `{}` 로 교체 → 잔재(원본+플래그)는 conflict 로 보존(종전: 스윕 시작 memo 와 동등 → 삭제)
     ⓑ 되교환: 활성이 원본을 잃은 형상에서 우리 inode(원본+플래그)는 conflict 로 보존(종전: payload 동등 → 삭제)
        · 활성이 원본을 **담고** 있으면(더 새 문서) 폐기 — 구조 포함 증명
     ⓒ finally: 교체 전 REFUSE(concurrent-change)에서 임시본(원본+플래그)은 conflict 로 보존(종전: 무조건 unlink)
     · 교환 성공 직후 외부가 활성을 비우면 옛 원본(displaced)은 conflict 로 보존
     · 사용자 필드 뒤에서 잘린 임시 JSON 은 보존(종전: '비문서' 로 삭제) · 평범한 부분 기록(접두)은 삭제
     · C58 이 conflict 의 존재·경로를 WARN 으로 안내한다
  P6 [major] 마감 감시가 CLI 진입점에만 있었다 — `seed_trust_bounded` 가 두 호출자를 덮고 C58 `--fix` 는 쌍당 예산 + 총예산
     안에 접히며 **뒤 체크가 실행**된다 · 음성 대조: 무한 대기 형상은 정말로 돌아오지 않는다
  P12 [major] 다른 cwd 의 무손실 크래시 사본이 무한 conflict 로 누적됐다 — A/B cwd 교차 크래시 반복에서 누적 0 ·
     `false`·사용자 필드 차이는 보존 · 같은 바이트의 쌍둥이는 두 번 보존하지 않는다
  P18 [문서] Windows 합성(시드도 심박도 무력 = 자동 복구 0) 문면 핀

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_trust_seed_reflect.py
"""
import hashlib
import inspect
import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
import javis_preflight as pf  # noqa: E402

_HOME_KEYS = ("HOME", "USERPROFILE")
_ISO_KEYS = ("CYS_DEPTS_JSON", "CYS_ACCOUNT_DIR", "CLAUDE_CONFIG_DIR", "CYS_PACK_DIR", "CYS_SOCKET",
             "LOCALAPPDATA", "XDG_STATE_HOME", "CYS_SEED_TRUST_TIMEOUT", "CYS_C58_FIX_BUDGET")

ORIG = {"userID": "u-1", "oauthAccount": {"emailAddress": "a@b"},
        "projects": {"/elsewhere": {"hasTrustDialogAccepted": True, "allowedTools": ["x"]}}}
ORIG_B = json.dumps(ORIG, ensure_ascii=False, indent=2).encode("utf-8")
STUB_ZERO = lambda d: (0, "stub-zero")  # noqa: E731


def _write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def _read(path):
    with open(path, "rb") as f:
        return f.read()


def _sha(b):
    return hashlib.sha256(b).hexdigest()


def _exchange_supported():
    d = tempfile.mkdtemp(prefix="xchg-")
    try:
        a, b = os.path.join(d, "a"), os.path.join(d, "b")
        _write(a, b"a")
        _write(b, b"b")
        try:
            return pf._exchange_paths(a, b) is True
        except OSError:
            return False
    finally:
        shutil.rmtree(d, ignore_errors=True)


class _Box(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trust-refl-")
        self.cfg_dir = os.path.join(self.tmp, "cfg")
        self.wsA = os.path.join(self.tmp, "wsA")
        self.wsB = os.path.join(self.tmp, "wsB")
        for d in (self.cfg_dir, self.wsA, self.wsB):
            os.makedirs(d)
        self.cfg = os.path.join(self.cfg_dir, ".claude.json")
        self.keyA = pf.claude_project_key(self.wsA)
        self.keyB = pf.claude_project_key(self.wsB)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def conflicts(self):
        return sorted(n for n in os.listdir(self.cfg_dir) if n.startswith(pf.SEED_TRUST_CONFLICT_PREFIX))

    def litter_names(self):
        return sorted(n for n in os.listdir(self.cfg_dir) if pf._SEED_TMP_LITTER_RE.match(n))

    def litter(self, name, data):
        p = os.path.join(self.cfg_dir, name)
        _write(p, data)
        return p

    def plan(self, key):
        b = pf._planned_payload_bytes(self.cfg, key)
        self.assertIsNotNone(b)
        return b

    def sweep(self, key):
        note = []
        n = pf._sweep_stale_seed_tmp(self.cfg_dir, note, key)
        return n, note


class P2_SharedProof(_Box):
    """P2: 세 삭제 경로가 **삭제 시점의** 공용 증명을 공유한다."""

    def test_a_sweep_replans_at_judgment_time(self):
        _write(self.cfg, ORIG_B)
        planA = self.plan(self.keyA)
        self.litter(".claude.json.seed-aaaa0001", planA)          # 저널 공개 전 크래시 잔재 = 원본 + 플래그
        _write(self.cfg, b"{}")                                   # 계획 계산 **뒤** 외부가 활성을 비웠다
        n, note = self.sweep(self.keyA)
        self.assertEqual(self.litter_names(), [], note)
        self.assertEqual(len(self.conflicts()), 1, (self.conflicts(), note))
        self.assertEqual(_read(os.path.join(self.cfg_dir, self.conflicts()[0])), planA, "유일한 원본 사본이 conflict 로 보존된다")
        # 음성 대조: 종전 술어(스윕 시작에 memo 한 계획과의 바이트 동등)는 이 잔재의 삭제를 인가했다
        self.assertEqual(planA, _read(os.path.join(self.cfg_dir, self.conflicts()[0])))
        self.assertNotEqual(pf._planned_payload_bytes(self.cfg, self.keyA), planA, "지금 계획은 memo 와 다르다(활성이 바뀌었다)")

    def test_a2_sweep_normal_crash_is_lossless(self):
        _write(self.cfg, ORIG_B)
        self.litter(".claude.json.seed-aaaa0002", self.plan(self.keyA))
        n, note = self.sweep(self.keyA)
        self.assertEqual(n, 1, note)
        self.assertEqual(self.conflicts(), [], "활성이 그대로면 잔재는 재생성 가능 = 삭제(conflict 0)")

    def test_c_finally_preserves_temp_on_refuse(self):
        _write(self.cfg, ORIG_B)
        planA = self.plan(self.keyA)

        def hook():
            _write(self.cfg, b"{}")                                # 대조(⑥) 직전에 외부가 활성을 비웠다
        rc, verdict, reason = pf.seed_trust(self.cfg_dir, self.wsA, proc_counter=STUB_ZERO, _pre_write_hook=hook)
        self.assertEqual(verdict, "REFUSE", reason)
        self.assertIn("concurrent-change", reason)
        self.assertEqual(_read(self.cfg), b"{}", "상대 내용 무접촉")
        self.assertEqual(self.litter_names(), [], "임시 이름은 남지 않는다")
        self.assertEqual(len(self.conflicts()), 1, "finally 가 임시본(원본 + 플래그)을 conflict 로 옮긴다(종전: 무조건 unlink)")
        self.assertEqual(_read(os.path.join(self.cfg_dir, self.conflicts()[0])), planA)

    def test_c2_finally_drops_temp_when_active_intact(self):
        if not _exchange_supported():
            self.skipTest("원자 교환 부재 FS — 성공 경로는 정직 skip")
        _write(self.cfg, ORIG_B)
        rc, verdict, reason = pf.seed_trust(self.cfg_dir, self.wsA, proc_counter=STUB_ZERO)
        self.assertEqual((rc, verdict), (0, "OK"), reason)
        self.assertEqual(self.conflicts(), [], "통상 경로는 conflict 0")
        self.assertEqual(self.litter_names(), [])
        left = sorted(n for n in os.listdir(self.cfg_dir) if n != pf.SEED_TRUST_LOCK_NAME)   # 잠금 파일은 남는 것이 정상
        self.assertEqual(left, [".claude.json"], os.listdir(self.cfg_dir))

    def test_b_swap_back_preserves_when_active_lost_original(self):
        if not _exchange_supported():
            self.skipTest("원자 교환 부재 FS")
        _write(self.cfg, ORIG_B)
        payload = self.plan(self.keyA)
        digests = (_sha(ORIG_B), _sha(payload))
        # 픽스처 = 교환 직후 형상: 활성 = 우리 payload · displaced = 낯선 inode(`{}` — 대조 뒤 외부가 활성을 비웠다)
        _write(self.cfg, payload)
        disp = os.path.join(self.cfg_dir, pf.SEED_TRUST_DISPLACED_PREFIX + digests[1] + "-1")
        _write(disp, b"{}")
        note = []
        got = pf._restore_foreign(disp, self.cfg, payload, note, digests=digests, key=self.keyA)
        self.assertEqual(got[1], "REFUSE", got)
        self.assertEqual(_read(self.cfg), b"{}", "상대 inode 무손실 복원")
        self.assertFalse(os.path.lexists(disp))
        self.assertEqual(len(self.conflicts()), 1, got)
        self.assertEqual(_read(os.path.join(self.cfg_dir, self.conflicts()[0])), payload, "원본의 유일한 사본이 보존된다")
        self.assertIn(self.conflicts()[0], got[2])
        # 음성 대조: 종전 술어 `mine == payload_b` 는 참이었다(= 삭제 인가)
        self.assertEqual(payload, _read(os.path.join(self.cfg_dir, self.conflicts()[0])))

    def test_b2_swap_back_drops_when_active_covers(self):
        if not _exchange_supported():
            self.skipTest("원자 교환 부재 FS")
        _write(self.cfg, ORIG_B)
        payload = self.plan(self.keyA)
        digests = (_sha(ORIG_B), _sha(payload))
        newer = dict(ORIG)
        newer["extra"] = {"added": True}                          # 활성이 원본을 **담은 채** 필드를 더 얻었다
        _write(self.cfg, payload)
        disp = os.path.join(self.cfg_dir, pf.SEED_TRUST_DISPLACED_PREFIX + digests[1] + "-1")
        _write(disp, json.dumps(newer, ensure_ascii=False, indent=2).encode("utf-8"))
        got = pf._restore_foreign(disp, self.cfg, payload, [], digests=digests, key=self.keyA)
        self.assertEqual(got[1], "REFUSE", got)
        self.assertIn("우리 사본 폐기[active-covers", got[2])
        self.assertEqual(self.conflicts(), [], "구조 포함이 서면 conflict 를 만들지 않는다")
        self.assertEqual(json.loads(_read(self.cfg)), newer)

    def test_exchange_success_window_preserves_old_original(self):
        if not _exchange_supported():
            self.skipTest("원자 교환 부재 FS")
        _write(self.cfg, ORIG_B)
        real = pf._exchange_paths

        def racing(a, b):
            r = real(a, b)
            if r is True and os.path.basename(b) == ".claude.json":
                _write(self.cfg, b"{}")                            # 교환 직후 그 마이크로초에 외부가 활성을 비웠다
            return r
        pf._exchange_paths = racing
        try:
            rc, verdict, reason = pf.seed_trust(self.cfg_dir, self.wsA, proc_counter=STUB_ZERO)
        finally:
            pf._exchange_paths = real
        self.assertIn("displaced-preserved(", reason)
        self.assertEqual(len(self.conflicts()), 1, reason)
        self.assertEqual(_read(os.path.join(self.cfg_dir, self.conflicts()[0])), ORIG_B, "옛 원본이 유일한 사본 = 보존")
        self.assertFalse(any(pf._SEED_DISPLACED_RE.match(n) for n in os.listdir(self.cfg_dir)))
        self.assertFalse(any(n.startswith(pf.SEED_TRUST_INTENT_PREFIX) for n in os.listdir(self.cfg_dir)), "저널 회수")

    def test_truncated_temp_with_user_field_is_preserved(self):
        _write(self.cfg, b"{}")
        cut = b'{"userID": "u-1", "projects": {"/x": {"hasTrustDialogAccepted": fals'
        self.litter(".claude.json.seed-bbbb0001", cut)
        self.sweep(self.keyA)
        self.assertEqual(self.litter_names(), [])
        self.assertEqual(len(self.conflicts()), 1, "사용자 필드 뒤에서 잘린 JSON 은 보존(종전: 비문서 = 삭제)")
        self.assertEqual(_read(os.path.join(self.cfg_dir, self.conflicts()[0])), cut)
        # 음성 대조: 종전 술어는 파싱 실패를 '지킬 데이터 0' 으로 읽었다
        self.assertIsNone(pf._json_or_none(cut))
        self.assertTrue(pf._document_carries_user_data(cut))

    def test_truncated_temp_that_is_plan_prefix_is_dropped(self):
        _write(self.cfg, ORIG_B)
        planA = self.plan(self.keyA)
        self.litter(".claude.json.seed-bbbb0002", planA[: len(planA) // 2])   # 쓰다 만 우리 payload
        n, note = self.sweep(self.keyA)
        self.assertEqual(n, 1, note)
        self.assertEqual(self.conflicts(), [], "평범한 부분 기록은 접두 증명으로 삭제(영구 conflict 0)")

    def test_release_uncommitted_and_recovery_use_shared_proof(self):
        src_ruc = inspect.getsource(pf._release_uncommitted_copy)
        src_rec = inspect.getsource(pf._recover_interrupted_seed)
        src_jup = inspect.getsource(pf._judge_unrenamed_payload)
        self.assertIn("_supersedable_file(cfg, dpath, digests, key)", src_ruc)
        self.assertEqual(src_rec.count("_supersedable_file(cfg, dpath, (rec[\"captured_sha256\"], rec[\"payload_sha256\"]), key)"), 2)
        self.assertIn("_supersedable_file(cfg, tpath, (rec[\"captured_sha256\"], rec[\"payload_sha256\"]), key)", src_jup)
        self.assertNotIn("_copy_is_redundant(", src_ruc + src_rec + src_jup)
        # 판독 불가는 보존: 존재하지 않는 사본에 대한 증명은 None 이다(빈 바이트 갈래로 흘리지 않는다)
        self.assertIsNone(pf._supersedable_file(self.cfg, os.path.join(self.cfg_dir, "nope"), (), self.keyA))


class P12_LosslessAccumulation(_Box):
    def test_cross_cwd_crash_copies_do_not_accumulate(self):
        _write(self.cfg, ORIG_B)
        planA = self.plan(self.keyA)
        for i in range(3):
            self.litter(".claude.json.seed-cccc000%d" % i, planA)   # cwd A 의 저널 전 크래시 잔재
            n, note = self.sweep(self.keyB)                           # cwd B 가 정리한다(계획이 다르다)
            self.assertEqual(n, 1, note)
            self.assertEqual(self.conflicts(), [], "무손실 사본(true 플래그 차이만)은 누적 0")
        self.assertEqual(json.loads(_read(self.cfg)), ORIG, "활성 무접촉")

    def test_false_flag_and_user_field_differences_are_preserved_once(self):
        _write(self.cfg, ORIG_B)
        doc_false = json.loads(ORIG_B)
        doc_false["projects"][self.keyA] = {"hasTrustDialogAccepted": False}      # 사람의 명시 거절
        b_false = json.dumps(doc_false, ensure_ascii=False, indent=2).encode("utf-8")
        self.litter(".claude.json.seed-dddd0001", b_false)
        self.sweep(self.keyB)
        self.assertEqual(len(self.conflicts()), 1, "false 는 시더가 쓰지 않는 값 = 보존")
        self.litter(".claude.json.seed-dddd0002", b_false)                          # 같은 바이트가 또 남았다
        self.sweep(self.keyB)
        self.assertEqual(len(self.conflicts()), 1, "쌍둥이가 있으면 다시 보존하지 않는다(누적 상한)")
        self.assertEqual(self.litter_names(), [])
        doc_user = json.loads(ORIG_B)
        doc_user["userID"] = "u-2"
        self.litter(".claude.json.seed-dddd0003", json.dumps(doc_user, ensure_ascii=False, indent=2).encode("utf-8"))
        self.sweep(self.keyB)
        self.assertEqual(len(self.conflicts()), 2, "사용자 필드 차이는 보존")

    def test_type_differences_are_data(self):
        _write(self.cfg, json.dumps({"n": 1, "projects": {}}).encode("utf-8"))
        copy = json.dumps({"n": True, "projects": {}}).encode("utf-8")
        self.assertIsNone(pf._copy_supersedable(self.cfg, copy, key=self.keyA), "1 ≠ true(형 구분)")
        self.assertFalse(pf._active_covers(self.cfg, copy))
        same = json.dumps({"n": 1, "projects": {self.keyB: {"hasTrustDialogAccepted": True}}}).encode("utf-8")
        self.assertIsNotNone(pf._copy_supersedable(self.cfg, same, key=self.keyA), "true 플래그 차이만 = 중복")


class P2_C58Guidance(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trust-refl-c58-")
        self.home = os.path.join(self.tmp, "home")
        self.cfg_dir = os.path.join(self.home, ".cys", "claude-default-dept-1")
        self.ws = os.path.join(self.home, "ws")
        self.pack = os.path.join(self.tmp, "pack")
        for d in (self.cfg_dir, self.ws, self.pack, os.path.join(self.home, ".cys")):
            os.makedirs(d, exist_ok=True)
        self._saved = {k: os.environ.get(k) for k in _HOME_KEYS + _ISO_KEYS}
        for k in _HOME_KEYS:
            os.environ[k] = self.home
        for k in _ISO_KEYS:
            os.environ.pop(k, None)
        os.environ["CYS_DEPTS_JSON"] = os.path.join(self.home, ".cys", "depts.json")
        os.environ["CYS_PACK_DIR"] = self.pack
        self._iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)
        self.key = pf.claude_project_key(self.ws)
        with open(os.environ["CYS_DEPTS_JSON"], "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-1": {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-1", "cys.sock"),
                                            "account_dir": self.cfg_dir, "cwd": self.ws}}}, f)
        _write(os.path.join(self.cfg_dir, ".claude.json"),
               json.dumps({"projects": {self.key: {"hasTrustDialogAccepted": True}}}).encode("utf-8"))

    def tearDown(self):
        pf._discover_isolation_block = self._iso
        for k, v in self._saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _c58(self, fix=False):
        p = pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report", allow_irreversible=False)
        p.c58_trust_harden()
        r = [x for x in p.results if x["id"] == "C58.trust-harden"]
        self.assertEqual(len(r), 1, r)
        return p, r[0]

    def test_conflict_copy_is_reported(self):
        _p, before = self._c58()
        self.assertEqual(before["status"], "PASS", before)
        _write(os.path.join(self.cfg_dir, pf.SEED_TRUST_CONFLICT_PREFIX + "20260910T000000Z-1"), b'{"userID": "u-1"}')
        _p, row = self._c58()
        self.assertEqual(row["status"], "WARN", row)
        self.assertIn("보존 사본 1건", row["detail"])
        self.assertIn(self.cfg_dir, row["detail"])
        self.assertIn("자동 정리 0", row["detail"])
        self.assertTrue(os.path.exists(os.path.join(self.cfg_dir, pf.SEED_TRUST_CONFLICT_PREFIX + "20260910T000000Z-1")),
                        "report 모드는 읽기 전용 · conflict 는 어느 경로에서도 자동 삭제되지 않는다")


class P6_BoundedSeed(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="trust-refl-p6-")
        self.home = os.path.join(self.tmp, "home")
        self.cfg_dir = os.path.join(self.home, ".cys", "claude-default-dept-1")
        self.pack = os.path.join(self.tmp, "pack")
        self.ws = [os.path.join(self.home, "ws%d" % i) for i in range(3)]
        for d in [self.cfg_dir, self.pack, os.path.join(self.home, ".cys")] + self.ws:
            os.makedirs(d, exist_ok=True)
        self._saved = {k: os.environ.get(k) for k in _HOME_KEYS + _ISO_KEYS}
        for k in _HOME_KEYS:
            os.environ[k] = self.home
        for k in _ISO_KEYS:
            os.environ.pop(k, None)
        os.environ["CYS_DEPTS_JSON"] = os.path.join(self.home, ".cys", "depts.json")
        os.environ["CYS_PACK_DIR"] = self.pack
        self._iso = pf._discover_isolation_block
        pf._discover_isolation_block = lambda: (None, None)
        self._seed = pf.seed_trust
        self.release = threading.Event()

    def tearDown(self):
        self.release.set()
        pf.seed_trust = self._seed
        pf._discover_isolation_block = self._iso
        for k, v in self._saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _blocking_lock(self, f):
        self.release.wait()
        return True

    def test_wrapper_returns_within_budget(self):
        t0 = time.monotonic()
        r = pf.seed_trust_bounded(self.cfg_dir, self.ws[0], 0.4, lock_fn=self._blocking_lock, proc_counter=STUB_ZERO)
        dt = time.monotonic() - t0
        self.assertEqual(r[0:2], (pf.SEED_TRUST_REFUSE, "REFUSE"), r)
        self.assertIn("timeout(", r[2])
        self.assertIn("커밋 여부는 이 줄로 단정하지 않는다", r[2])
        self.assertLess(dt, 5.0, "예산 안에 돌아온다")
        # 음성 대조: 상한 없는 형상(secs=None · 종전 라이브러리 경로)은 정말로 돌아오지 않는다
        box = {}
        th = threading.Thread(target=lambda: box.setdefault("r", pf.seed_trust_bounded(
            self.cfg_dir, self.ws[1], None, lock_fn=self._blocking_lock, proc_counter=STUB_ZERO)), daemon=True)
        th.start()
        th.join(0.5)
        self.assertTrue(th.is_alive(), "무한 대기 형상은 0.5s 뒤에도 살아 있다(이것이 종전 C58 --fix 의 형상)")
        self.release.set()
        th.join(5.0)
        self.assertFalse(th.is_alive())

    def test_wrapper_reraises_worker_exception(self):
        def boom(f):
            raise RuntimeError("lock exploded")
        with self.assertRaises(RuntimeError):
            pf.seed_trust_bounded(self.cfg_dir, self.ws[0], 1.0, lock_fn=boom, proc_counter=STUB_ZERO)

    def test_c58_fix_folds_within_budget_and_later_checks_run(self):
        with open(os.environ["CYS_DEPTS_JSON"], "w", encoding="utf-8") as f:
            json.dump({"depts": {"dept-%d" % i: {"socket": os.path.join(self.home, ".local", "state", "cys-dept-dept-%d" % i, "cys.sock"),
                                                 "account_dir": self.cfg_dir, "cwd": w} for i, w in enumerate(self.ws)}}, f)
        calls = []

        def blocking_seed(config_dir, cwd, **kw):
            calls.append(cwd)
            self.release.wait()                                   # 응답 없는 마운트 흉내: 돌아오지 않는다
            return pf.SEED_TRUST_OK, "OK", "late"
        pf.seed_trust = blocking_seed
        os.environ["CYS_SEED_TRUST_TIMEOUT"] = "0.3"
        os.environ["CYS_C58_FIX_BUDGET"] = "0.5"
        p = pf.Preflight(fix=True, skips=[], mode="fix", allow_irreversible=False)
        t0 = time.monotonic()
        p.c58_trust_harden()
        dt = time.monotonic() - t0
        row = [x for x in p.results if x["id"] == "C58.trust-harden"][0]
        self.assertLess(dt, 5.0, "C58 --fix 가 예산 안에 접힌다(종전: 무시간제한)")
        self.assertEqual(row["status"], "WARN", row)
        self.assertIn("timeout(", row["detail"])
        self.assertIn("budget-exhausted(", row["detail"], "총예산 소진 뒤의 쌍은 '보류' 로 보고된다(침묵 0)")
        self.assertLess(len(calls), 3, "예산이 다한 뒤에는 시더를 부르지 않는다")
        # 뒤 체크가 실행된다 — C59 는 격리 팩에 guard.sh 가 없어 FAIL 행이지만 **행이 있다**는 것이 요점이다
        p.c59_guard_wiring()
        self.assertTrue(any(x["id"] == "C59.guard-wiring" for x in p.results), [x["id"] for x in p.results])
        self.release.set()


class P18_WindowsComposite(unittest.TestCase):
    def test_sentence_pinned_in_exchange_unavailable_reason(self):
        src = inspect.getsource(pf.seed_trust)
        self.assertIn("자동 복구 0", src)
        self.assertIn("사람 1회 통과가 유일 경로", src)
        self.assertLess(src.index("exchange-unavailable("), src.index("사람 1회 통과가 유일 경로"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
