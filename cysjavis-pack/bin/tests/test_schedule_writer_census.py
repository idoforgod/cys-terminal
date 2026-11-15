#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_schedule_writer_census.py — CONTRACTS C7 H6(2) 의 `schedule.json` 작성자 census(0.15.0 · T2 소유).

계약(RULINGS H6(2) · CONTRACTS C7 표, 태그 기준 작성자 11): 팩의 `schedule.json` 을 쓰는 제품 코드는 표에 적힌
자리뿐이고, 잠금 없이 **제자리 절단**(`open(p, "w").write(...)`)으로 쓰던 유일한 작성자(preflight C16 두 자리)는
같은 디렉터리 임시 파일 + 원자 교체(LF)로 바뀌었다. 새 작성자가 생기면 이 census 가 멈춘다 — 그때 그 작성자가
다른 작성자의 항목을 지우지 않는지(설계 §8-1 D㉘) 검토하고 표를 함께 고친다(조용한 통과 금지).

판정 방식(정직): 소스 텍스트 스캔이다. `schedule.json` 이라는 이름을 담은 운영 소스 파일의 **집합**을 고정하고,
preflight 안에서는 쓰기 형태를 고정한다. 동적 경로 조립(이름을 쪼개 붙이기)으로 숨긴 작성자는 잡지 못한다.
"""
import os
import re
import subprocess
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
REPO = os.path.dirname(os.path.dirname(BIN))

# C7 표의 '이름이 나오는 운영 소스' 집합(시험·문서 제외). 이 밖의 파일이 이름을 쓰기 시작하면 검토 대상이다.
CENSUS_FILES = {
    "src/bin/cysd/schedule.rs",          # 작성자 1–5(데몬 · ScheduleFileLock)
    "src/bin/cysd/main.rs",              # 주석만(부팅 보장 호출)
    "src/bin/cys.rs",                    # 작성자 6(cys schedule add/remove · 잠금) — 그 밖은 읽기·안내
    "src/pack.rs",                       # 작성자 7(설치기 · write_atomic) · 9(저널 롤백)
    "src/pack_prev_vendor.rs",           # 해시 표(쓰기 아님)
    "cysjavis-pack/bin/javis_preflight.py",     # 작성자 11(C16 — 임시 + 원자 교체 · LF)
    "cysjavis-pack/bin/javis_dept_schedule.py", # 작성자 10(부서 심기 · mkstemp + os.replace · LF)
    "cysjavis-pack/bin/javis_report_gate.py",   # 읽기만
    "cysjavis-pack/bin/cys-dept",               # 작성자 10 호출
    # 0.15.0 T6 등재(검토 결과): 수리 엔진은 schedule.json 을 대상으로 하는 승인된 수리·M10 되돌리기를
    # 규칙 1(파일 전체 맞바꾸기)로 쓴다 — 팩 적용 잠금 아래, 살아 있는 바이트가 0단계에 기록된 기대
    # 상태와 같을 때만(다르면 거절·대체됨) 바꾸므로 그 뒤에 다른 작성자가 넣은 항목은 지우지 않는다(D㉘).
    "src/repair/apply.rs",               # 작성자 12(수리 엔진 · 규칙 1) — 이름은 H6(1) 자체 검사 주석
    "src/bin/cysd/repair_job.rs",        # 읽기만 — H6(1) 자체 검사(스케줄러 검증)를 수리 엔진에 넘김
    "scripts/windows-measure/tw1_probes.py",    # NOT a writer of the product (C7 table unchanged): the CI-only Windows
                                                # measurement driver (0.15.0 TW1, RULINGS FC3) edits the schedule.json
                                                # of the installation UNDER TEST on a CI runner - the scenario's "the
                                                # user removed a built-in job" - and reads it back
}


def _tracked_files():
    try:
        out = subprocess.run(["git", "-C", REPO, "ls-files"], capture_output=True, text=True,
                             timeout=60, check=True).stdout.splitlines()
    except Exception:
        out = []
        for d, _ds, fs in os.walk(REPO):
            if "/.git" in d or "/target" in d or "node_modules" in d:
                continue
            for n in fs:
                out.append(os.path.relpath(os.path.join(d, n), REPO))
    return out


def _is_prod_source(rel):
    if "/tests/" in rel or rel.startswith("tests/") or "/test_" in rel or rel.endswith("_tests.rs"):
        return False
    if rel.startswith(("docs/", "impl/", ".github/")) or rel.endswith((".md", ".txt", ".json")):
        return False
    return rel.endswith((".rs", ".py", ".sh", ".ts", ".js")) or rel in ("cysjavis-pack/bin/cys-dept",)


def _prod_text(rel):
    with open(os.path.join(REPO, rel), encoding="utf-8", errors="replace") as f:
        text = f.read()
    if rel.endswith(".rs"):
        cut = text.find("#[cfg(test)]\nmod tests")
        if cut >= 0:
            text = text[:cut]
    return text


class ScheduleWriterCensus(unittest.TestCase):
    def test_files_naming_schedule_json_are_the_census_set(self):
        found = set()
        for rel in _tracked_files():
            if not _is_prod_source(rel):
                continue
            try:
                text = _prod_text(rel)
            except OSError:
                continue
            if "schedule.json" in text:
                found.add(rel)
        self.assertTrue({"src/bin/cysd/schedule.rs", "cysjavis-pack/bin/javis_preflight.py"} <= found,
                        "census 가 눈멀었다(대표 작성자 파일을 못 봤다): %r" % sorted(found))
        unexpected = sorted(found - CENSUS_FILES)
        self.assertEqual(unexpected, [],
                         "schedule.json 을 다루는 새 운영 소스 — C7 작성자 표에 넣고 다른 작성자의 항목을 "
                         "지우지 않는지 검토하라(설계 §8-1 D㉘ · RULINGS H6): %r" % unexpected)

    def test_preflight_has_no_truncate_in_place_schedule_writer(self):
        text = _prod_text("cysjavis-pack/bin/javis_preflight.py")
        start = text.index("def c16_report_schedule(self):")
        end = text.index("\n    def ", start + 10)
        c16 = text[start:end]
        self.assertEqual(c16.count("_write_json_lf_atomic(p, data)"), 2,
                         "C16 의 두 쓰기 자리는 임시 파일 + 원자 교체(LF)여야 한다(H6(2))")
        self.assertNotRegex(c16, r'open\(\s*p\s*,\s*["\']w["\']',
                            "C16 이 다시 제자리 절단으로 쓴다 — 잠금 없는 작성자가 데몬의 항목을 지우는 창(H6(2))")
        helper = text[text.index("def _write_json_lf_atomic("):]
        helper = helper[:helper.index("\n\n\n")]
        self.assertIn('newline="\\n"', helper, "LF 강제(윈도우 텍스트 모드 CRLF 금지)")
        self.assertIn("os.replace(tmp, path)", helper, "원자 교체")
        self.assertNotIn("FileLock", helper, "잠금은 더하지 않는다(H6(2) 'no lock added')")

    def test_dept_seed_writes_lf_atomically(self):
        text = _prod_text("cysjavis-pack/bin/javis_dept_schedule.py")
        self.assertRegex(text, r'os\.fdopen\(fd, "w", encoding="utf-8", newline="\\n"\)')
        self.assertIn("os.replace(tmp, path)", text)

    def test_daemon_writers_keep_their_lock(self):
        text = _prod_text("src/bin/cysd/schedule.rs")
        self.assertIn("ScheduleFileLock", text, "데몬 작성자 1–5 의 잠금(변경 없음)")
        self.assertTrue(re.search(r"fn write_schedule_atomic", text), "데몬 원자 쓰기(변경 없음)")


if __name__ == "__main__":
    unittest.main(verbosity=2)
