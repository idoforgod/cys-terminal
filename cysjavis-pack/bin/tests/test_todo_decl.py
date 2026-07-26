#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_todo_decl.py — javis_todo_decl.py(선언 블록 v1 파서·분류기) 골든 픽스처 회귀.

DESIGN_declared-state.md §4-1(G1'~G10)·§4-2(판정 5분기)의 계약을 **픽스처가 SOT**로 핀한다
(ADR-2). 이 파일은 픽스처를 해석하는 Python 측 러너일 뿐이고, 같은 픽스처를 Rust
`src/todo_decl.rs`가 읽어 동일 결과를 내는지는 파리티 CI 잡(P3-4)이 강제한다 — 그래서
기대값을 테스트 코드에 하드코딩하지 않고 전부 `fixtures/todo-decl/expected.json`에서 읽는다.
기대값이 테스트 코드 안에 있으면 언어 중립이 아니게 되고 파리티 CI가 껍데기가 된다.

실행: python3 test_todo_decl.py   (unittest·표준 러너 — CI가 파일 직접 실행하는 관례 준거)
"""
import json
import os
import sys
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
FIXTURES = os.path.join(SELF, "fixtures", "todo-decl")
sys.path.insert(0, BIN)
import javis_todo_decl as T                                              # noqa: E402

with open(os.path.join(FIXTURES, "expected.json"), encoding="utf-8") as _f:
    SPEC = json.load(_f)

EXISTING = set(SPEC["existing_scopes"])

# 정상 선언 한 줄(문법 세부 테스트의 기준선).
GOOD = "<!-- javis:todo v1 owner=worker-2 scope=pack status=active -->"


def scope_exists(scope):
    """픽스처가 자체 선언한 가상 팩 목록으로 판정 — 실제 파일시스템에 의존하면 안 된다."""
    return scope in EXISTING


class TestFixtureParity(unittest.TestCase):
    """픽스처 전건 대조 — 케이스마다 subTest로 갈라 첫 실패에서 멈추지 않게 한다."""

    def test_all_cases(self):
        """계약 표면 3종만 대조한다 — classify · diag_code · decl(ADR-4 C-2).

        한국어 진단 문구는 **의도적으로 대조하지 않는다.** 문구를 계약에 넣으면 문구를 다듬는
        순간 파리티가 결함을 오보한다.
        """
        for name, exp in sorted(SPEC["cases"].items()):
            with self.subTest(case=name, note=exp["note"]):
                head = T.read_head(os.path.join(FIXTURES, name), SPEC["head_bytes"])
                decl, diag = T.parse(head)
                got = T.classify(decl, SPEC["my_scope"], scope_exists)
                self.assertEqual(got, exp["classify"],
                                 "%s: 판정 불일치 (진단=%r)" % (name, diag))
                code = diag.code if diag is not None else None
                self.assertEqual(code, exp["diag_code"], "%s: 진단 코드 불일치" % name)
                self.assertEqual(decl, exp["decl"], "%s: 선언 내용 불일치" % name)

    def test_case_coverage_is_exhaustive(self):
        """픽스처 파일과 expected.json 항목이 1:1 — 어느 쪽이 늘어도 조용히 새지 않는다."""
        on_disk = {f for f in os.listdir(FIXTURES) if f.endswith(".md")}
        self.assertEqual(on_disk, set(SPEC["cases"]),
                         "픽스처 파일과 expected.json 불일치(누락/고아)")
        self.assertGreaterEqual(len(on_disk), 15, "설계 P3-1이 요구한 최소 15케이스 미달")

    def test_expected_uses_only_known_diag_codes(self):
        """픽스처의 diag_code는 전부 DIAG_CODES 안에 있어야 한다(오타 코드 = 조용한 계약 이탈)."""
        for name, exp in sorted(SPEC["cases"].items()):
            with self.subTest(case=name):
                code = exp["diag_code"]
                if code is None:
                    continue
                self.assertIn(code, T.DIAG_CODES, "%s: 미지 diag_code" % name)

    def test_contract_file_declares_the_code_set(self):
        """계약 파일이 스스로 코드 집합을 싣고 있고, 구현과 일치한다(양 언어 공통 목록)."""
        self.assertEqual(list(SPEC["_diag_codes"]), list(T.DIAG_CODES))
        self.assertEqual(sorted(T.DIAG_CODES), sorted(T.DIAG_TEMPLATES),
                         "코드 목록과 문구 템플릿 키가 어긋난다")

    def test_human_message_is_not_part_of_the_contract(self):
        """★ADR-4 C-2 핀 — 한국어 문구가 계약 파일에 **다시 들어오는 것**을 막는다.

        미래의 수정자가 "문구도 고정하면 더 엄격하지 않나"라며 되돌리기 쉬운 지점이라, 계약
        파일에 문구 필드가 없음을 테스트로 못 박는다(계약 파일의 `_not_contract` 고지 참조).
        문구를 계약에 넣는 순간, 문구를 다듬는 커밋마다 파리티 CI가 결함을 오보한다.
        """
        for name, exp in sorted(SPEC["cases"].items()):
            with self.subTest(case=name):
                self.assertNotIn("diag", exp,
                                 "%s: 한국어 진단 문구는 계약이 아니다(ADR-4 C-2)" % name)
        self.assertIn("_not_contract", SPEC, "계약 경계 고지문이 사라졌다")

    def test_diag_carries_code_and_human_message(self):
        """Diag는 str이면서 code를 나른다 — 기존 호출부 무회귀 + 코드 노출을 동시에 만족."""
        decl, diag = T.parse("<!-- javis:todo v1 owner=w status=active -->\n")
        self.assertIsNone(decl)
        self.assertIsInstance(diag, str)                     # 기존 호출부 계약(문자열)
        self.assertEqual(diag.code, "missing-keys")          # 신규 계약(언어중립 코드)
        self.assertEqual(diag, "필수 키 누락: scope")        # 문구는 UX일 뿐(계약 아님)

    def test_every_code_is_reachable(self):
        """7종 코드가 전부 실제 입력으로 도달 가능해야 한다 — 죽은 코드는 계약이 아니다."""
        samples = {
            "no-decl": "# T\n- [ ] a\n",
            "duplicate": "%s\n%s\n" % (GOOD, GOOD),
            "syntax": "%s x\n" % GOOD,
            "unknown-version": "<!-- javis:todo v9 owner=w scope=pack status=active -->\n",
            "bad-token": '<!-- javis:todo v1 owner="w" scope=pack status=active -->\n',
            "missing-keys": "<!-- javis:todo v1 owner=w scope=pack -->\n",
            "bad-status": "<!-- javis:todo v1 owner=w scope=pack status=done -->\n",
        }
        self.assertEqual(sorted(samples), sorted(T.DIAG_CODES), "샘플이 코드 집합과 어긋남")
        for code, head in sorted(samples.items()):
            with self.subTest(code=code):
                decl, diag = T.parse(head)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, code)


class TestVerdictContract(unittest.TestCase):
    """판정 5분기의 직렬화 계약 — 비교 가능한 표현이 없으면 파리티 CI가 껍데기가 된다."""

    VERDICTS = ("counted", "retired", "foreign-scope", "orphan-scope", "unclaimed")

    def test_only_contract_strings_are_emitted(self):
        for name in SPEC["cases"]:
            head = T.read_head(os.path.join(FIXTURES, name), SPEC["head_bytes"])
            got = T.classify(T.parse(head)[0], SPEC["my_scope"], scope_exists)
            self.assertIn(got, self.VERDICTS, "%s: 계약 밖 판정 문자열" % name)

    def test_all_five_branches_are_exercised(self):
        """5분기 전부가 픽스처에 실재해야 한다 — 안 도는 분기는 검증되지 않은 분기다."""
        seen = {exp["classify"] for exp in SPEC["cases"].values()}
        self.assertEqual(seen, set(self.VERDICTS), "미검증 분기 존재: %s"
                         % (set(self.VERDICTS) - seen))

    def test_retired_precedes_scope(self):
        """분기 순서 고정 — 은퇴는 scope보다 먼저다(내 팩이든 남의 팩이든 은퇴는 은퇴)."""
        for scope in ("pack", "pack-dept-dept-1", "pack-dept-dept-9"):
            decl = {"owner": "worker-2", "scope": scope, "status": "retired"}
            self.assertEqual(T.classify(decl, "pack", scope_exists), "retired")

    def test_scope_exists_is_injected_not_probed(self):
        """실재 판정은 주입된 콜러블로만 이뤄진다(파서가 디스크를 만지지 않는다는 계약)."""
        calls = []

        def probe(scope):
            calls.append(scope)
            return False

        decl = {"owner": "w", "scope": "pack-somewhere-else", "status": "active"}
        self.assertEqual(T.classify(decl, "pack", probe), "orphan-scope")
        self.assertEqual(calls, ["pack-somewhere-else"])


class TestLegacySentinel(unittest.TestCase):
    """★ADR-4 C-3 — 레거시 은퇴 선언의 표현은 센티널 `"?"` 로 양 언어 통일.

    Rust `Decl`의 owner/scope가 비-Option `String`이라 값을 채워야 하고(Option화는 파급이 큼),
    Python이 키를 아예 빼면 두 언어의 선언 표현이 갈린다. 센티널은 "모른다"를 명시적으로 말한다.
    대가는 하나 — **센티널이 scope 판정에 새면 안 된다.** 그 불변식을 여기서 핀한다.
    """

    LEGACY = ("<!-- javis:todo-retired -->", "<!-- ★ STALE 무효화 -->",
              "<!-- RETIRED 2026-07-11 -->")

    def test_legacy_marker_yields_sentinel_declaration(self):
        for line in self.LEGACY:
            with self.subTest(marker=line):
                decl, diag = T.parse("%s\n# T\n- [ ] a\n" % line)
                self.assertIsNone(diag)
                self.assertEqual(decl, {"owner": "?", "scope": "?",
                                        "status": "retired", "_legacy": True})

    def test_retired_branch_never_probes_scope(self):
        """★불변식 — retired 분기는 `scope_exists`를 **호출하지 않는다**.

        호출되면 센티널 `"?"`가 팩 이름으로 조회돼(당연히 부재) 은퇴 파일이 orphan-scope로
        시끄럽게 쏟아진다. 콜백이 불리면 즉시 터지게 해서 회귀를 조용히 넘기지 않는다
        (Rust 측은 같은 계약을 패닉 콜백으로 핀한다 — `classify_none_is_unclaimed…`).
        """
        def explode(scope):
            raise AssertionError("retired 분기가 scope 실재를 조회했다: %r" % scope)

        for line in self.LEGACY:
            with self.subTest(marker=line):
                decl, _ = T.parse("%s\n# T\n- [ ] a\n" % line)
                self.assertEqual(T.classify(decl, "pack", explode), "retired")
        # 정규 은퇴 선언도 동일(레거시 전용 특례가 아니라 분기 순서의 성질이다).
        for scope in ("pack", "pack-dept-dept-1", "pack-dept-dept-9", "?"):
            decl = {"owner": "w", "scope": scope, "status": "retired"}
            self.assertEqual(T.classify(decl, "pack", explode), "retired")

    def test_sentinel_is_not_a_real_scope(self):
        """센티널이 살아있는 선언에 쓰이면 orphan으로 잡힌다 — 조용히 counted 되지 않는다."""
        decl = {"owner": "?", "scope": "?", "status": "active"}
        self.assertEqual(T.classify(decl, "pack", scope_exists), "orphan-scope")


class TestHeadBudget(unittest.TestCase):
    """G3 — 파싱 예산은 **바이트** 기준이다(문자 기준이면 한글에서 Rust와 갈린다)."""

    def test_head_bytes_constant(self):
        self.assertEqual(T.HEAD_BYTES, 1024)

    def test_read_head_truncates_by_bytes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            p = os.path.join(td, "K_TODO.md")
            with open(p, "wb") as f:
                f.write(("가" * 100).encode("utf-8"))          # 300 바이트 = 100 문자
            self.assertEqual(len(T.read_head(p, 30)), 10)      # 바이트 절단이면 10 문자
            self.assertEqual(len(T.read_head(p, 300)), 100)

    def test_read_head_missing_file_is_fail_open(self):
        self.assertEqual(T.read_head(os.path.join(FIXTURES, "없는파일_TODO.md")), "")

    def test_split_multibyte_becomes_replacement_char(self):
        """경계에서 잘린 다바이트 문자는 U+FFFD — Rust from_utf8_lossy와 같은 결과."""
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            p = os.path.join(td, "K_TODO.md")
            with open(p, "wb") as f:
                f.write("가나".encode("utf-8"))
            self.assertEqual(T.read_head(p, 4), "가�")


class TestSelfHarmRegression(unittest.TestCase):
    """★A2 자해 회귀 핀 — 본문 체크박스 뒤 선언은 절대 인정되지 않는다."""

    DECL = "<!-- javis:todo v1 owner=worker-2 scope=pack status=retired -->"

    def test_declaration_after_first_checkbox_is_ignored(self):
        decl, diag = T.parse("# T\n- [ ] 미완\n%s\n" % self.DECL)
        self.assertIsNone(decl)
        self.assertEqual(diag, T.DIAG_TEMPLATES["no-decl"])

    def test_declaration_inside_checkbox_text_is_ignored(self):
        decl, diag = T.parse("# T\n- [x] 완료\n- [ ] %s\n" % self.DECL)
        self.assertIsNone(decl)
        self.assertEqual(T.classify(decl, "pack", scope_exists), "unclaimed")

    def test_legacy_marker_after_checkbox_is_ignored(self):
        """G10 레거시 마커도 같은 위치 계약을 받는다(예외를 두면 자해 통로가 다시 열린다)."""
        decl, _ = T.parse("# T\n- [ ] 미완\n<!-- ★ STALE 무효화 -->\n")
        self.assertIsNone(decl)


class TestHeaderMasking(unittest.TestCase):
    """★G12 머리말 마스킹(W9 교정 1) — 문서용 예시 선언이 살아있는 파일을 은퇴시키면 안 된다.

    G1'(첫 체크박스 이전)만으로는 부족했다: 선언 문법을 **설명하는** todo(펜스 안 예시)가
    자기 자신을 은퇴시켰고, 이 프로젝트에서 가장 흔한 종류의 todo가 정확히 그 피해자였다.
    Rust `src/todo_decl.rs`가 같은 이름의 테스트로 같은 계약을 핀한다.
    """

    RET = "<!-- javis:todo v1 owner=worker-2 scope=pack status=retired -->"

    def verdict(self, text):
        return T.classify(T.parse(text)[0], "pack", scope_exists)

    def test_fenced_example_declaration_does_not_retire_the_file(self):
        for fence in ("```", "~~~", "````", "```markdown"):
            close = fence[0] * 4 if fence.startswith("`" * 4) else fence[0] * 3
            with self.subTest(fence=fence):
                text = "%s\n\n# 안내\n\n%s\n%s\n%s\n\n- [ ] a\n- [ ] b\n" % (
                    GOOD, fence, self.RET, close)
                self.assertEqual(self.verdict(text), "counted")

    def test_fenced_example_alone_is_undeclared_not_retired(self):
        text = "# 선언 문법 안내\n\n```\n%s\n```\n\n- [ ] a\n- [ ] b\n" % self.RET
        decl, diag = T.parse(text)
        self.assertIsNone(decl)
        self.assertEqual(diag.code, "no-decl")

    def test_fence_closes_only_with_same_char_and_length(self):
        for close in ("~~~", "``", "```` x"):
            with self.subTest(close=close):
                text = "# T\n````\n%s\n%s\n%s\n- [ ] a\n" % (self.RET, close, self.RET)
                self.assertEqual(self.verdict(text), "unclaimed")
        # 같은 문자로 더 길게 닫는 것은 허용(CommonMark).
        self.assertEqual(
            self.verdict("# T\n```\n예시\n`````\n%s\n- [ ] a\n" % GOOD), "counted")

    def test_unclosed_fence_is_recovered_at_end_of_header(self):
        """★W13 중대 D — 미닫힘 펜스는 머리말 끝에서 **회수**한다(G12 ⑤ · master 심판).

        종전 규칙("펜스가 열리면 첫 체크박스까지 무조건 마스킹")은 인라인 삼중백틱 한 줄로
        그 아래 **정당한 선언을 삼켰다**. M1에서는 폴백이 덮어 무해해 보이지만 M3에서 폴백이
        삭제되면 그 파일이 집계에서 사라지고, 지금도 `unclaimed_ratio`를 왜곡한다.
        반대 방향(은퇴 선언이 삼켜져 은퇴한 파일이 계속 집계됨)도 같은 무게로 핀한다.
        """
        # ① 살아있는 선언은 삼켜지지 않는다.
        self.assertEqual(self.verdict("# T\n```\n%s\n\n- [ ] a\n" % GOOD), "counted")
        # ② 은퇴 선언도 대칭으로 회수된다.
        self.assertEqual(self.verdict("# T\n```\n%s\n\n- [ ] a\n" % self.RET), "retired")
        # ③ 레거시 마커 줄도 같다.
        self.assertEqual(
            self.verdict("# T\n```\n<!-- ★ STALE 무효화 -->\n\n- [ ] a\n"), "retired")
        # ④ 회수해도 ②인용·③들여쓰기 마스킹은 그대로다.
        for masked in ("> %s" % GOOD, "    %s" % GOOD, "\t%s" % GOOD):
            with self.subTest(masked=masked):
                self.assertEqual(
                    self.verdict("# T\n```\n%s\n- [ ] a\n" % masked), "unclaimed")
        # ⑤ **닫힌** 펜스는 종전대로 마스킹된다(G12 ①의 본령은 그대로다).
        self.assertEqual(
            self.verdict("# T\n```\n%s\n```\n- [ ] a\n" % self.RET), "unclaimed")
        # ⑥ ★W15 중대 ① — **정상 후보가 있으면 회수를 취소한다**(회귀 핀).
        #   W13 직후 이 자리는 `unclaimed`(G7 duplicate)였다. 그것이 회귀였다 — 회수 도입
        #   **전에는** 펜스 안 예시가 마스킹돼 `counted`였고, 회수가 정당한 선언을 죽인 것이다.
        #   선언 도입 작업의 todo(자기 선언 + 문법 설명 펜스)가 정확히 이 형태다.
        self.assertEqual(
            self.verdict("%s\n```\n%s\n- [ ] a\n" % (GOOD, self.RET)), "counted")

    def test_unclosed_fence_recovery_yields_to_existing_candidate(self):
        """★W15 중대 ① — 회수는 '선언이 없을 때의 구제책'이지 '선언을 늘리는 장치'가 아니다.

        회수 구간과 정상 구간에 **둘 다** 후보가 있으면 회수를 취소한다(정상 후보 우선).
        후보에는 v1 선언 후보와 **레거시 은퇴 마커 줄**이 모두 포함된다 — 한쪽만 세면 같은
        충돌이 다른 토큰으로 재발한다.
        """
        # ① 진짜 선언(정상) vs 회수될 예시 선언 → 진짜 선언이 이긴다.
        self.assertEqual(
            self.verdict("%s\n\n# 안내\n```\n%s\n\n- [ ] a\n" % (GOOD, self.RET)), "counted")
        # ② 은퇴 선언(정상) vs 회수될 active 예시 → 은퇴가 이긴다(반대 방향).
        self.assertEqual(
            self.verdict("%s\n\n# 안내\n```\n%s\n\n- [ ] a\n" % (self.RET, GOOD)), "retired")
        # ③ 정상 구간의 **레거시 마커** vs 회수될 선언 → 마커가 이긴다(교차 축).
        self.assertEqual(
            self.verdict("<!-- ★ STALE 무효화 -->\n# T\n```\n%s\n\n- [ ] a\n" % GOOD), "retired")
        # ④ 정상 구간에 후보가 없으면 회수는 종전대로 발동한다(W13 계약 불변).
        self.assertEqual(self.verdict("# T\n```\n%s\n\n- [ ] a\n" % GOOD), "counted")
        # ⑤ 회수 구간 **안에서만** 후보가 2개면 종전대로 G7 모호성 거부다.
        self.assertEqual(
            self.verdict("# T\n```\n%s\n%s\n- [ ] a\n" % (GOOD, self.RET)), "unclaimed")

    def test_retire_marker_line_is_whole_line_anchored(self):
        """★W13 치명 A — 은퇴 마커는 **그 줄이 마커 줄일 때만** 인정한다(부분일치 금지).

        reviewer1 실측 2종이 이 핀의 이유다. 무앵커 부분일치이던 종전 판정에서는 아래 산문
        한 줄이 미완 2건짜리 살아있는 파일을 통째로 은퇴시켰고, 두 번째 문장은 **파서까지
        확정 판정**을 내려 Rust 데몬도 같이 지웠다. 문구의 출처가 우리 자신의 디렉티브
        (*"레인이 끝나면 status=retired 로 갱신하라"*)라 워커가 지침을 옮겨 적으면 자해했다.
        """
        prose = (
            "규약: 레인이 끝나면 javis:todo v1 선언의 status=retired 로 바꾼다.",
            "이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.",
            "STALE 무효화 마커를 기계가 읽도록 구현한다.",     # 줄 선두여도 산문은 산문
            "이 파일을 javis:todo-retired 로 표시하는 방법을 설명한다.",
            "★ STALE  무효화",                                # 주석도 아니고 마커 전용 줄도 아니다
            "★ javis:todo-retired",
            "- 종결 시 <!-- RETIRED --> 를 넣는다",            # 줄이 `<!--`로 시작하지 않는다
            "<!-- 참고: retired 로 바꾸려면 -->",              # bare `retired`는 `<!--` 직후만
        )
        for line in prose:
            with self.subTest(line=line):
                self.assertFalse(T.is_retire_marker_line(line))
                decl, diag = T.parse("# WORKER TODO\n%s\n- [ ] a\n- [ ] b\n" % line)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "no-decl")

    def test_marker_token_is_anchored_inside_comments_too(self):
        """★W15 중대 ② — **부정문·설명문이 파일을 은퇴시키면 안 된다**(주석 안 앵커).

        W13은 산문(비주석)만 앵커하고 주석 안은 `RETIRED`만 앵커했다. `javis:todo-retired`·
        `stale 무효화`는 주석 안 **어디든** 부분일치라, 아래 두 줄이 미완 2건짜리 살아있는
        파일을 `retired`로 확정시켰다(reviewer1 3차 실측 재현). 자해의 형태는 W13이 막은 것과
        똑같고 **자리만 주석 안으로 옮겼을 뿐**이다.

        교정: `<!--` 와 마커 토큰 사이에는 장식 문자(`DECOR_CHARS`)만 허용한다. 한글·라틴
        문장 문자가 하나라도 오면 그 줄은 마커가 아니다.
        """
        prose = (
            "<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->",
            "<!-- 은퇴시키려면 STALE 무효화 라고 적는다 -->",
            "<!-- TODO: STALE 무효화 마커를 기계가 읽도록 구현한다 -->",
            "<!-- 이 파일을 javis:todo-retired 로 표시하는 방법 -->",
            "<!-- note: javis:todo-retired 는 레거시 토큰이다 -->",
            "<!-- 종결 시 RETIRED 를 넣는다 -->",
        )
        for line in prose:
            with self.subTest(line=line):
                self.assertFalse(T.is_retire_marker_line(line))
                decl, diag = T.parse("# WORKER TODO\n%s\n- [ ] a\n- [ ] b\n" % line)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "no-decl")

    def test_real_marker_forms_are_still_honored(self):
        """반대 방향 — 진짜 마커 줄은 여전히 은퇴다(과잉 앵커는 07-26 유령 집계를 되살린다).

        마지막 항목이 이 조직의 **유일한 실물 마커**다(07-11 teardown이 삽입한 여러 줄 주석의
        개시 줄). 같은 줄의 `-->`를 요구하면 그 파일이 즉시 `unclaimed`로 되살아난다.
        """
        for line in ("<!-- javis:todo-retired -->", "<!-- JAVIS:TODO-RETIRED -->",
                     "<!-- ★ STALE  무효화 (레인 종결) -->", "<!-- RETIRED 2026-07-11 -->",
                     "<!--retired-->", "<!--  RETIRED  -->",
                     "javis:todo-retired", "stale 무효화", "  STALE\t무효화  ",
                     # ★W15 — 장식 접두는 허용한다(실물 마커 10개가 전부 이 형태다).
                     "<!-- ==== RETIRED ==== -->", "<!-- ###javis:todo-retired -->",
                     "<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입) ★★★★"):
            with self.subTest(line=line):
                self.assertTrue(T.is_retire_marker_line(line))
                self.assertEqual(self.verdict("%s\n# T\n- [ ] a\n" % line), "retired")

    def test_documented_defense_boundary_is_exactly_token_position(self):
        """★W18 교정 2 — 방어 범위를 **문서가 주장하는 그대로** 기계로 핀한다.

        정본 문장(코드 주석·설계 문서·릴리스 노트에 같은 문구로 적혀 있다):
          "문장 문자가 **토큰 앞**에 오면 마커가 아니다. 토큰 **뒤**의 꼬리 텍스트는 마커를
           무력화하지 못한다(실물 마커가 꼬리를 요구하므로 불가피). 따라서 마커 토큰으로
           시작하는 설명·부정문은 은퇴로 읽힌다 — 마커를 설명하려면 토큰을 문장 뒤에 두어라."

        이 테스트가 없으면 문서의 주장과 코드가 갈릴 수 있고, 갈린 문서는 **없는 방어를
        있다고 믿게 만든다** — 이 프로젝트가 반복해서 당한 사고의 형태다(W15 정본 블록의
        ⚠주석이 정확히 그 자리에서 틀려 있었다).

        실물 코퍼스에 아래 형태는 0건이고, 꼬리를 금지하면 실물 마커 10개가 죽는다.
        결함이 아니라 **계약의 경계**이므로 여기서 계약으로 못 박는다.
        """
        # 토큰이 **앞** → 은퇴로 읽힌다(설명·부정문이어도).
        for line in ("<!-- **STALE 무효화** 는 이 파일에 해당하지 않는다 -->",
                     "<!-- javis:todo-retired 마커는 레거시 표기다 -->",
                     "<!-- RETIRED 는 은퇴 토큰이다 -->"):
            with self.subTest(token_first=line):
                self.assertTrue(T.is_retire_marker_line(line))
                self.assertEqual(self.verdict("%s\n# T\n- [ ] a\n" % line), "retired")
        # 문장 문자가 **토큰 앞** → 마커가 아니다(권장 표기).
        for line in ("<!-- 이 파일은 STALE 무효화 대상이 아니다 -->",
                     "<!-- 레거시 표기는 javis:todo-retired 다 -->",
                     "<!-- 은퇴 토큰은 RETIRED 다 -->"):
            with self.subTest(prose_first=line):
                self.assertFalse(T.is_retire_marker_line(line))

    def test_decor_charset_contains_no_sentence_characters(self):
        """★W15 중대 ② — `DECOR_CHARS`의 **유일한 계약**: 문장 문자가 하나도 없다.

        이 집합에 한글·라틴 문자가 한 글자라도 들어가는 순간 그 글자로 시작하는 문장이
        마커 관문을 통과하고, 부정문 은퇴 결함이 곧바로 되살아난다. Rust 리터럴과의 동일성은
        `test_todo_shared_constants.py`가 따로 대조한다(여기서는 집합 자체의 성질만 핀한다).
        """
        for ch in T.DECOR_CHARS:
            with self.subTest(ch=ch):
                self.assertFalse(ch.isalnum(), "장식 집합에 문장 문자가 있다: %r" % ch)
        self.assertIn(" ", T.DECOR_CHARS)     # 실물 마커 `<!-- ★★★★ …` 의 공백
        self.assertIn("★", T.DECOR_CHARS)     # 실물 마커의 장식

    def test_quoted_and_indented_declarations_are_not_candidates(self):
        for masked in ("> %s" % self.RET, ">%s" % self.RET, "   > %s" % self.RET,
                       "    %s" % self.RET, "\t%s" % self.RET,
                       "        %s" % self.RET):
            with self.subTest(masked=masked):
                # 자기 선언과 함께면 자기 선언만 남는다(중복 판정이 아니다).
                self.assertEqual(
                    self.verdict("%s\n%s\n- [ ] a\n" % (GOOD, masked)), "counted")
                decl, diag = T.parse("# T\n%s\n- [ ] a\n" % masked)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "no-decl")

    def test_masked_legacy_retire_marker_is_not_honored(self):
        """레거시 마커도 같은 마스킹 — 마커를 설명하는 문서가 자기를 은퇴시키면 안 된다."""
        for masked in ("```\n<!-- ★ STALE 무효화 -->\n```",
                       "> <!-- javis:todo-retired -->",
                       "    <!-- RETIRED 2026-07-11 -->"):
            with self.subTest(masked=masked):
                decl, diag = T.parse("# T\n%s\n- [ ] a\n" % masked)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "no-decl")

    def test_indent_up_to_three_spaces_is_still_a_declaration(self):
        """완화 유지 — 구멍만 막는다(선두 공백 3개까지는 선언 줄)."""
        self.assertEqual(self.verdict("   %s\n- [ ] a\n" % GOOD), "counted")


class TestSeparatorConvergence(unittest.TestCase):
    """★G11 개행·공백 수렴(W9 교정 2) — 좁은 쪽(Rust)으로 맞춘다.

    `str.splitlines()`와 정규식 `\\s`는 유니코드 전량을 개행·공백으로 보고, Rust는 `\\n\\r`만
    개행·`White_Space`만 공백으로 본다. 실측 13건이 갈렸고 다수가 **Python은 배제하는데
    데몬은 집계**하는 위험한 방향이었다. 좁은 쪽으로 맞추면 그런 줄은 양 언어에서 똑같이
    문법 위반으로 떨어진다.
    """

    # Python `str.splitlines()` 가 개행으로 보지만 G11 계약은 보지 않는 문자들.
    EXTRA_NEWLINES = ("\x0b", "\x0c", "\x1c", "\x1d", "\x1e", "\x85",
                      "\u2028", "\u2029")
    # `\s`/`str.split()` 이 공백으로 보지만 G11 계약은 보지 않는 문자들.
    EXTRA_SPACES = ("\x1c", "\x1f", "\xa0", "\u2028", "\u3000")

    def test_only_lf_crlf_cr_split_lines(self):
        self.assertEqual(T._split_lines("a\nb\r\nc\rd"), ["a", "b", "c", "d"])
        self.assertEqual(T._split_lines("a\n"), ["a"])       # 말미 개행은 빈 줄을 만들지 않는다
        for c in self.EXTRA_NEWLINES:
            with self.subTest(char=hex(ord(c))):
                self.assertEqual(T._split_lines("a%sb" % c), ["a%sb" % c])

    def test_token_separator_is_space_and_tab_only(self):
        for c in set(self.EXTRA_NEWLINES) | set(self.EXTRA_SPACES):
            with self.subTest(char=hex(ord(c))):
                # 키 사이 — 토큰이 갈리지 않아 문자 클래스 위반이다.
                decl, diag = T.parse(
                    "<!-- javis:todo v1 owner=w%sscope=pack status=active -->\n- [ ] a\n" % c)
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "bad-token")
                # 줄 꼬리 — `-->` 앵커가 깨진다(후행 공백으로 흡수되지 않는다).
                decl, diag = T.parse("%s%s\n- [ ] a\n" % (GOOD, c))
                self.assertIsNone(decl)
                self.assertEqual(diag.code, "syntax")

    def test_space_and_tab_are_valid_separators(self):
        decl, diag = T.parse(
            "<!-- javis:todo v1 owner=w\tscope=pack \t status=active -->\n- [ ] a\n")
        self.assertIsNone(diag)
        self.assertEqual(T.classify(decl, "pack", scope_exists), "counted")


class TestForwardCompat(unittest.TestCase):
    """G6 — 모르는 키는 무시(전방호환), 모르는 버전은 미선언(스큐 안전)."""

    def test_unknown_key_does_not_break_verdict(self):
        decl, diag = T.parse("<!-- javis:todo v1 owner=w scope=pack status=active"
                             " future_field=x -->\n")
        self.assertIsNone(diag)
        self.assertEqual(T.classify(decl, "pack", scope_exists), "counted")

    def test_unknown_version_is_undeclared(self):
        for ver in ("v0", "v2", "v99"):
            decl, diag = T.parse("<!-- javis:todo %s owner=w scope=pack status=active -->\n"
                                 % ver)
            self.assertIsNone(decl)
            self.assertEqual(diag, "미지 버전 %s" % ver)


if __name__ == "__main__":
    unittest.main()
