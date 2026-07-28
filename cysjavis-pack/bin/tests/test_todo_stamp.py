#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_todo_stamp.py — javis_todo_stamp.py(선언 블록 일괄 스탬프 마이그레이터) 회귀.

이 도구는 **라이브 자산을 직접 고치는** 유일한 도구다(설계 P3-0). 따라서 테스트가 검증해야
하는 것은 "잘 넣는가"보다 **"넣으면 안 되는 곳에 넣지 않는가"** 다 — 자해(G7 선언 2개),
증거 오염(백업 사본 스탬프), 유산 부활(종결 레인), 시계 리셋(mtime 갱신), 원본 파손.

부작용 0: 모든 케이스가 임시 디렉터리 안에서만 돌고 `CYS_PACK_DIR`를 그 안으로 가리킨다.
실제 팩·`_round`는 이 파일이 하는 어떤 동작으로도 건드려지지 않는다.

실행: python3 test_todo_stamp.py   (unittest·표준 러너 — CI가 파일 직접 실행하는 관례 준거)
"""
import contextlib
import io
import os
import shutil
import sys
import tempfile
import time
import unittest
from unittest import mock

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_todo_decl as T                                              # noqa: E402
import javis_todo_stamp as S                                             # noqa: E402

DAY = 86400
BOM = "\ufeff"
BODY = "# WORKER TODO — 테스트 레인\n\n- [x] 완료 항목\n- [ ] 미완 항목\n"


class Base(unittest.TestCase):
    """임시 팩 트리: <tmp>/packs/pack/round · <tmp>/proj/_round(팩 밖)."""

    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="stamp-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        # 임시경로 자체가 백업 표식을 품으면 모든 케이스가 조용히 skip돼 테스트가 껍데기가 된다.
        self.assertFalse(S.is_backup_path(self.tmp), "임시경로가 백업 표식과 충돌")
        self.pack = os.path.join(self.tmp, "packs", "pack")
        self.round = os.path.join(self.pack, "round")
        self.outside = os.path.join(self.tmp, "proj", "_round")
        for d in (self.round, self.outside):
            os.makedirs(d)
        env = mock.patch.dict(os.environ, {"CYS_PACK_DIR": self.pack}, clear=False)
        env.start()
        self.addCleanup(env.stop)
        os.environ.pop("CYS_TODO_STALE_DAYS", None)   # 임계는 인자로만 결정(결정론)

    # ── 헬퍼 ────────────────────────────────────────────────────────────────
    def write(self, path, text=BODY, age_days=0):
        d = os.path.dirname(path)
        if not os.path.isdir(d):
            os.makedirs(d)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        if age_days:
            t = time.time() - age_days * DAY
            os.utime(path, (t, t))
        return path

    def run_tool(self, *argv):
        """(exit_code, stdout) — 인프로세스 실행이라 mock 주입이 가능하다."""
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = S.main(list(argv))
        return code, buf.getvalue()

    def read(self, path):
        with open(path, encoding="utf-8") as f:
            return f.read()

    def decl_candidates(self, path):
        """머리말의 선언 후보 줄 수 — G7 위반(2개 이상) 탐지용."""
        head = T.read_head(path)
        n = 0
        for line in head.splitlines():
            if T.RE_DONE.search(line) or T.RE_OPEN.search(line):
                break
            if T.RE_DECL_CAND.match(line.strip()):
                n += 1
        return n


class TestDryRun(Base):
    def test_dry_run_does_not_touch_file(self):
        """기본이 dry-run — 내용도 mtime도 바뀌지 않고, 삽입될 선언 줄 전문을 보여준다."""
        p = self.write(os.path.join(self.round, "WORKER_2_TODO.md"), age_days=1)
        before, mtime = self.read(p), os.stat(p).st_mtime
        code, out = self.run_tool()                   # --dir 미지정 = 팩 round/ 기본 스캔
        self.assertEqual(code, 0, out)
        self.assertIn("DRY-RUN", out)
        self.assertIn("[stamp]", out)
        self.assertIn("<!-- javis:todo v1 owner=worker-2 scope=pack status=active -->", out)
        self.assertEqual(self.read(p), before, "dry-run이 내용을 변경")
        self.assertEqual(os.stat(p).st_mtime, mtime, "dry-run이 mtime을 변경")

    def test_owner_is_inferred_from_filename(self):
        """WORKER_2_TODO.md → worker-2 (소문자·언더스코어→하이픈 = 소비자 라벨공간)."""
        self.assertEqual(S.owner_from_filename("/Users/x/REVIEWER_GEMINI_TODO.md"),
                         "reviewer-gemini")
        self.assertEqual(S.owner_from_filename("/Users/x/MASTER_TODO.md"), "master")

    def test_owner_override(self):
        p = self.write(os.path.join(self.round, "MASTER_TODO.md"))
        _, out = self.run_tool("--owner", "cso")
        self.assertIn("owner=cso", out)
        self.assertIn(os.path.basename(p), out)


class TestSkipRules(Base):
    def test_already_declared_is_skipped(self):
        """★자해 방어 — 이미 선언이 있는 파일에 하나 더 넣으면 G7로 미선언이 된다."""
        decl = "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n\n"
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), decl + BODY)
        before = self.read(p)
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 이미 유효 선언 보유", out)
        self.assertEqual(self.read(p), before)
        self.assertEqual(self.decl_candidates(p), 1)

    def test_legacy_retire_marker_is_skipped(self):
        """G10 — 레거시 은퇴 마커는 기계가 이미 retired로 판정한다(스탬프 불필요·유해)."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"),
                       "<!-- ★ STALE 무효화 2026-07-20 -->\n" + BODY)
        before = self.read(p)
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 레거시 은퇴 마커", out)
        self.assertEqual(self.read(p), before)

    def test_broken_declaration_is_skipped(self):
        """깨진 선언 후보가 있으면 스탬프 금지 — 넣으면 후보 2개로 영구 duplicate가 된다."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"),
                       "<!-- javis:todo v1 owner=worker status=active -->\n" + BODY)
        before = self.read(p)
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 선언 오작성(missing-keys", out)
        self.assertEqual(self.read(p), before)

    def test_backup_and_generation_paths_are_skipped(self):
        """설계 R9 — 백업·세대 사본에 선언을 박으면 그것이 살아있는 파일로 오인된다."""
        for name in ("state-generations", "backups", "archive",
                     ".pre-migrate", "round.bak-2026"):
            with self.subTest(dir=name):
                d = os.path.join(self.tmp, "packs", "pack", name)
                p = self.write(os.path.join(d, "WORKER_TODO.md"))
                before = self.read(p)
                code, out = self.run_tool("--dir", d, "--apply")
                self.assertEqual(code, 0, out)
                self.assertIn("[skip 백업·세대 경로", out)
                self.assertEqual(self.read(p), before)

    def test_stale_file_skipped_by_default_and_included_by_flag(self):
        """종결된 레인을 되살리지 않는다 — 포함은 명시 플래그로만."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), age_days=30)
        before = self.read(p)
        code, out = self.run_tool("--stale-days", "7", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 유산 의심", out)
        self.assertEqual(self.read(p), before)

        code, out = self.run_tool("--stale-days", "7", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[stamp]", out)
        self.assertNotEqual(self.read(p), before)

    def test_outside_pack_requires_explicit_scope(self):
        """팩 밖 경로는 scope를 추측하지 않는다 — 잘못된 scope는 orphan-scope 오배제를 낳는다."""
        p = self.write(os.path.join(self.outside, "WORKER_TODO.md"))
        before = self.read(p)
        code, out = self.run_tool("--dir", self.outside, "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip scope 미확정", out)
        self.assertEqual(self.read(p), before, "scope 미확정인데 파일이 변경됨")

        code, out = self.run_tool("--dir", self.outside, "--scope", "pack", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("scope=pack", out)
        decl, _ = T.parse(T.read_head(p))
        self.assertEqual(decl["scope"], "pack")

    def test_sibling_pack_scope_is_detected(self):
        """부서 팩(형제 디렉터리)은 경로에서 scope가 산출된다."""
        d = os.path.join(self.tmp, "packs", "pack-dept-dept-2", "round")
        p = self.write(os.path.join(d, "WORKER_TODO.md"))
        code, out = self.run_tool("--dir", d)
        self.assertEqual(code, 0, out)
        self.assertIn("scope=pack-dept-dept-2", out)

    def test_missing_dir_is_usage_error(self):
        code, out = self.run_tool("--dir", os.path.join(self.tmp, "nope"))
        self.assertEqual(code, 2, out)


class TestApply(Base):
    def test_apply_end_to_end_is_counted_by_parser(self):
        """스탬프 → 파서 재검증 `counted`. 본문은 한 글자도 바뀌지 않는다."""
        p = self.write(os.path.join(self.round, "WORKER_2_TODO.md"))
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        text = self.read(p)
        lines = text.splitlines()
        self.assertEqual(lines[0],
                         "<!-- javis:todo v1 owner=worker-2 scope=pack status=active -->")
        self.assertEqual(lines[1], "", "선언 뒤 빈 줄 1개 규약 위반")
        self.assertTrue(text.endswith(BODY), "기존 내용이 변형됨")
        decl, diag = T.parse(T.read_head(p))
        self.assertIsNone(diag)
        self.assertEqual(T.classify(decl, "pack", lambda s: True), "counted")

    def test_stamped_file_has_exactly_one_declaration(self):
        """G7 위반 없음 + 재실행 멱등(두 번 돌려도 후보는 1개)."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"))
        self.run_tool("--apply")
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 이미 유효 선언 보유", out)
        self.assertEqual(self.decl_candidates(p), 1)
        decl, diag = T.parse(T.read_head(p))
        self.assertIsNotNone(decl, "재실행 후 duplicate 자해 발생(%s)" % diag)

    def test_mtime_is_preserved(self):
        """소비자의 stale 시계를 스탬프가 리셋하면 안 된다(기본 켬)."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), age_days=3)
        before = os.stat(p).st_mtime
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertAlmostEqual(os.stat(p).st_mtime, before, delta=1.0,
                               msg="mtime이 갱신됨 — stale 휴리스틱 시계 리셋")

    def test_no_preserve_mtime_flag_updates_clock(self):
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), age_days=3)
        before = os.stat(p).st_mtime
        self.run_tool("--apply", "--no-preserve-mtime")
        self.assertGreater(os.stat(p).st_mtime, before + DAY)

    def test_bom_stays_at_file_head(self):
        """G8 — BOM은 파일 선두에 남는다(본문 중간으로 밀지 않는다)."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), BOM + BODY)
        self.run_tool("--apply")
        text = self.read(p)
        self.assertTrue(text.startswith(BOM + "<!-- javis:todo v1 "))
        decl, _ = T.parse(T.read_head(p))
        self.assertEqual(T.classify(decl, "pack", lambda s: True), "counted")


class TestFailureSafety(Base):
    def test_write_failure_leaves_original_intact(self):
        """원자성 — 임시파일 생성이 실패해도 원본은 무손상이고 잔여물이 없다."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"))
        before = self.read(p)
        with mock.patch.object(S.tempfile, "mkstemp", side_effect=OSError("디스크 없음")):
            code, out = self.run_tool("--apply")
        self.assertEqual(code, 1, out)
        self.assertIn("[FAIL 쓰기 실패", out)
        self.assertEqual(self.read(p), before)
        self.assertEqual(sorted(os.listdir(self.round)), ["WORKER_TODO.md"],
                         "임시파일 잔여물")

    def test_verification_failure_rolls_back(self):
        """사후 파서 재검증이 실패하면 되돌린다 — 깨진 파일을 남기지 않는다."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"))
        before, mtime = self.read(p), os.stat(p).st_mtime
        real_classify = T.classify
        calls = []

        def fake(decl, scope, exists):
            # 1회차 = 생성물 사전검증(build_decl_line) → 정상 통과.
            # 2회차 = 파일 재검증(verify_counted) → 실패시켜 되돌림 경로를 강제한다.
            calls.append(1)
            return real_classify(decl, scope, exists) if len(calls) == 1 else "unclaimed"

        with mock.patch.object(S.todo_decl, "classify", side_effect=fake):
            code, out = self.run_tool("--apply")
        self.assertEqual(code, 1, out)
        self.assertIn("원본 복원 완료", out)
        self.assertEqual(self.read(p), before, "복원 실패 — 깨진 파일이 남음")
        self.assertAlmostEqual(os.stat(p).st_mtime, mtime, delta=1.0)

    def test_oversized_file_is_skipped(self):
        p = os.path.join(self.round, "WORKER_TODO.md")
        with open(p, "w", encoding="utf-8") as f:
            f.write("x" * (S.MAX_FILE_BYTES + 1))
        code, out = self.run_tool("--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 파일 과대", out)


class TestPromoteRetire(Base):
    """★W15 교정 3 동반 — 유령·레거시 마커를 **명시 은퇴 선언으로 승격**한다.

    왜 필요한가: 소비자가 유령(`stale_ghost`)을 park 차단에서 면제하기로 했는데, 운영자가
    그 유령을 처분할 경로가 없었다(이 도구가 무조건 skip). **정책을 지키려면 도구가 그
    정책을 집행할 수 있어야 한다.**

    이 스위트가 지키는 것은 '잘 승격하는가'보다 **'승격하면 안 되는 것을 승격하지 않는가'** 다.
    이 모드가 "아무 파일이나 은퇴시키는 무기"가 되면 그것 자체가 새 유령원이다.
    """

    MARKER = "<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입) ★★★★\n     구 전문 무효 -->\n"

    def test_legacy_marker_is_promoted_to_explicit_declaration(self):
        p = self.write(os.path.join(self.round, "REVIEWER_GEMINI_TODO.md"),
                       self.MARKER + BODY, age_days=9)
        code, out = self.run_tool("--promote-retire", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[promote]", out)
        decl, _ = T.parse(T.read_head(p))
        self.assertEqual((decl["owner"], decl["scope"], decl["status"]),
                         ("reviewer-gemini", "pack", "retired"))
        self.assertFalse(decl.get("_legacy"), "레거시 마커로 계속 읽히면 승격이 아니다")
        self.assertEqual(T.classify(decl, "pack", lambda s: True), "retired")
        # 원문은 한 글자도 잃지 않는다(마커 줄 포함).
        self.assertIn(self.MARKER, self.read(p))
        self.assertEqual(self.decl_candidates(p), 1, "G7 자해(선언 2개)")

    def test_ghost_requires_include_stale(self):
        """유령 승격은 '주인 대신 은퇴 도장을 찍는' 행위라 마찰이 있어야 한다."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), BODY, age_days=9)
        before = self.read(p)
        code, out = self.run_tool("--promote-retire", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("--include-stale 필요", out)
        self.assertEqual(self.read(p), before)

        code, out = self.run_tool("--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        decl, _ = T.parse(T.read_head(p))
        self.assertEqual(decl["status"], "retired")

    def test_fresh_undeclared_file_is_never_promoted(self):
        """★안전 경계 — 신선한 미선언 파일은 살아있는 작업일 수 있다.

        여기서 실수하면 이 프로젝트가 고치려던 사고(살아있는 파일의 조용한 소멸) 그 자체다.
        `--include-stale`을 줘도 신선 파일은 대상이 아니다.
        """
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"), BODY)
        before = self.read(p)
        code, out = self.run_tool("--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("승격 대상 아님(살아있는 작업 보호)", out)
        self.assertEqual(self.read(p), before)

    def test_explicit_declaration_is_left_alone(self):
        """이미 명시 선언이 있으면 건드리지 않는다(활성 선언을 몰래 은퇴시키지 않는다)."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"),
                       "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + BODY,
                       age_days=30)
        before = self.read(p)
        code, out = self.run_tool("--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("승격 불필요", out)
        self.assertEqual(self.read(p), before)

    def test_broken_declaration_is_not_promoted(self):
        """깨진 선언 위에 승격 선언을 얹으면 영구 G7 duplicate가 된다."""
        p = self.write(os.path.join(self.round, "WORKER_TODO.md"),
                       '<!-- javis:todo v1 owner="worker" scope=pack status=active -->\n' + BODY,
                       age_days=30)
        before = self.read(p)
        code, out = self.run_tool("--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("수동 교정 필요", out)
        self.assertEqual(self.read(p), before)

    def test_backup_paths_are_still_excluded(self):
        """증거 보존 규약은 모드와 무관하다(설계 R9)."""
        bk = os.path.join(self.tmp, "state-generations", "round")
        p = self.write(os.path.join(bk, "WORKER_TODO.md"), self.MARKER + BODY, age_days=9)
        before = self.read(p)
        code, out = self.run_tool("--dir", bk, "--scope", "pack",
                                  "--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("[skip 백업·세대 경로", out)
        self.assertEqual(self.read(p), before)

    def test_dry_run_is_the_default(self):
        p = self.write(os.path.join(self.round, "REVIEWER_GEMINI_TODO.md"),
                       self.MARKER + BODY, age_days=9)
        before = self.read(p)
        code, out = self.run_tool("--promote-retire")
        self.assertEqual(code, 0, out)
        self.assertIn("PROMOTE-RETIRE · DRY-RUN", out)
        self.assertIn("status=retired", out)
        self.assertEqual(self.read(p), before, "dry-run이 파일을 건드렸다")

    def test_mtime_is_preserved_and_second_run_is_a_noop(self):
        """시계 리셋 금지 + 멱등 — 승격된 파일은 다음 실행에서 '승격 불필요'다."""
        p = self.write(os.path.join(self.round, "REVIEWER_GEMINI_TODO.md"),
                       self.MARKER + BODY, age_days=9)
        mtime = os.stat(p).st_mtime
        self.run_tool("--promote-retire", "--apply")
        self.assertAlmostEqual(os.stat(p).st_mtime, mtime, delta=1.0)
        code, out = self.run_tool("--promote-retire", "--apply")
        self.assertEqual(code, 0, out)
        self.assertNotIn("[promote]", out)
        self.assertIn("승격 불필요", out)

    def test_roundtrip_verification_failure_rolls_back(self):
        """왕복 검증 실패 시 원본 복원 — 승격 모드도 같은 안전망을 쓴다."""
        p = self.write(os.path.join(self.round, "REVIEWER_GEMINI_TODO.md"),
                       self.MARKER + BODY, age_days=9)
        before, mtime = self.read(p), os.stat(p).st_mtime
        real_classify = T.classify
        calls = []

        def fake(decl, scope, exists):
            calls.append(1)
            return real_classify(decl, scope, exists) if len(calls) == 1 else "unclaimed"

        with mock.patch.object(S.todo_decl, "classify", side_effect=fake):
            code, out = self.run_tool("--promote-retire", "--apply")
        self.assertEqual(code, 1, out)
        self.assertIn("원본 복원 완료", out)
        self.assertEqual(self.read(p), before)
        self.assertAlmostEqual(os.stat(p).st_mtime, mtime, delta=1.0)

    def test_normal_mode_points_at_the_promote_path(self):
        """기본 모드의 skip 사유가 **승격 경로를 안내**한다(막다른 골목 금지)."""
        self.write(os.path.join(self.round, "WORKER_TODO.md"), self.MARKER + BODY)
        code, out = self.run_tool()
        self.assertEqual(code, 0, out)
        self.assertIn("--promote-retire", out)

    def test_promoted_ghost_stops_blocking_the_quiet_gate(self):
        """★end-to-end — 승격의 **목적**이 달성되는가(소비자 관점).

        승격 전: 유령의 미완이 `pending_outside_nodes`에 `stale_ghost`로 뜬다.
        승격 후: 주인이 처분을 명시했으므로 목록에서 **아예 빠진다**(`retired`(source=decl) 면제).
        """
        sys.path.insert(0, BIN)
        import javis_report as RP
        import javis_report_gate as RG

        self.write(os.path.join(self.round, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack status=active -->\n"
                   "# CSO\n- [x] d\n")
        self.write(os.path.join(self.outside, "WORKER_TODO.md"),
                   "# 유령\n- [x] d\n- [ ] o\n- [ ] o2\n", age_days=9)
        status = {"surfaces": [{"role": "cso", "cwd": None, "idle_secs": 999,
                                "agent_alive": True, "status": {}}],
                  "feed": {"pending": 0}, "paused": False}
        outside_parent = os.path.dirname(self.outside)

        rep = RP.build_report(status, [outside_parent])
        self.assertEqual([r["kind"] for r in rep["pending_outside_nodes"]], ["stale_ghost"])

        code, out = self.run_tool("--dir", self.outside, "--scope", "pack",
                                  "--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)

        rep = RP.build_report(status, [outside_parent])
        self.assertEqual(rep["pending_outside_nodes"], [])
        self.assertEqual(rep["hidden_pending"]["stale_ghost"], {"files": 0, "open": 0})
        self.assertTrue(RG.quiet_branch_holds(rep))


class TestW18Minor3ThresholdVisibility(Base):
    """【W18 교정 3】 `--promote-retire`의 **대상 집합**을 보이지 않는 상태가 바꾸지 않게 한다.

    reviewer1 경미 ㉰ 실측: `--include-stale`의 대상 판정이 `CYS_TODO_STALE_DAYS`를 쓰므로
    임계를 낮추면 승격 대상이 넓어진다.
        CYS_TODO_STALE_DAYS=1 javis_todo_stamp.py --promote-retire --include-stale
        #  기본 임계에선 보호되던 2일 된 파일이 [promote] 대상이 된다
    `--apply` 없이는 무해하고 조작이 명시적이라 **결함은 아니다.**

    심판: 임계 자체는 고정하지 않는다(운영자의 의도적 조정 여지를 남긴다). 대신 적용값과
    **출처**를 헤더에 항상 찍고, 기본값과 다르면 경고 한 줄을 띄운다.
    """

    def env(self, value):
        p = mock.patch.dict(os.environ, {S.STALE_DAYS_ENV: value}, clear=False)
        p.start()
        self.addCleanup(p.stop)

    def fresh_undeclared(self, age_days=2):
        return self.write(os.path.join(self.round, "CSO_TODO.md"), BODY, age_days=age_days)

    def test_default_threshold_prints_source_without_warning(self):
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale")
        self.assertEqual(code, 0, out)
        self.assertIn("유산 임계 7일 (출처: 기본값)", out)
        self.assertNotIn("⚠", out)
        self.assertNotIn("[promote]", out)          # 기본 임계에서는 보호된다

    def test_env_lowered_threshold_widens_targets_and_warns(self):
        """★reviewer1 재현 그대로 — 대상이 넓어지는 그 실행에서 경고가 함께 뜬다."""
        self.env("1")
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale")
        self.assertEqual(code, 0, out)
        self.assertIn("유산 임계 1일 (출처: 환경변수 CYS_TODO_STALE_DAYS=1)", out)
        self.assertIn("⚠ 유산 임계가 기본값(7일)과 다르다", out)
        self.assertIn("승격 대상 집합이 넓어졌다", out)
        self.assertIn("[promote]", out)             # 대상이 실제로 넓어졌음을 함께 핀한다

    def test_threshold_is_not_pinned_to_the_default(self):
        """임계를 **고정하지는 않는다** — 경고만 하고 운영자의 조정은 그대로 존중한다."""
        self.env("1")
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale", "--apply")
        self.assertEqual(code, 0, out)
        self.assertIn("status=retired", self.read(os.path.join(self.round, "CSO_TODO.md")))

    def test_unparsable_env_fallback_is_announced(self):
        """설정됐는데 조용히 무시되는 상태도 알린다 — 그 침묵이 곧 오해의 원인이다."""
        self.env("abc")
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale")
        self.assertEqual(code, 0, out)
        self.assertIn("유산 임계 7일 (출처: 기본값)", out)
        self.assertIn("해석 불가", out)

    def test_cli_argument_overrides_env_and_says_so(self):
        self.env("1")
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale", "--stale-days", "3")
        self.assertEqual(code, 0, out)
        self.assertIn("유산 임계 3일 (출처: --stale-days 인자)", out)
        self.assertIn("--stale-days 인자에 덮여 무시됐다", out)

    def test_raised_threshold_reports_narrower_targets(self):
        """방향 표기가 정확해야 한다 — 임계를 올리면 대상은 **좁아진다**(과장 금지)."""
        self.env("30")
        self.fresh_undeclared()
        code, out = self.run_tool("--promote-retire", "--include-stale")
        self.assertEqual(code, 0, out)
        self.assertIn("승격 대상 집합이 좁아졌다", out)
        self.assertNotIn("보호됐을 파일이 포함될 수 있다", out)

    def test_resolve_stale_days_is_the_single_rule(self):
        """헤더 문구가 아니라 **해석 함수**가 계약이다(두 번째 기준 금지)."""
        self.assertEqual(S.resolve_stale_days(None), (7, "기본값", None))
        self.env("2")
        self.assertEqual(S.resolve_stale_days(None),
                         (2, "환경변수 CYS_TODO_STALE_DAYS=2", None))
        days, src, note = S.resolve_stale_days(5)
        self.assertEqual((days, src), (5, "--stale-days 인자"))
        self.assertIn("무시됐다", note)


if __name__ == "__main__":
    unittest.main()
