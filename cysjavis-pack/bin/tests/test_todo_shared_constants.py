#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_todo_shared_constants.py — **한 개념이 두 곳에 적힌 자리**의 기계 대조(설계 §14-4 4번 · W14).

> 정의·예산·특권·기본값 — 무엇이든 두 번 적히면 언젠가 갈린다. 재사용하거나, 못 하면 기계
> 대조를 박아라.

이 스위트가 지키는 것(전부 실측 결함에서 왔다)
  ① **S19 팩 경로 env 목록** — 5구현(Python 3 + Rust 2)에 env 목록이 **3종**으로 갈려 있었다.
     `cys todo-path`가 `AITERM_JARVIS_DIR`를 인식하지 못해, 레거시 env 환경에서 **생성 위치와
     스캔 위치가 갈려 파일이 보고기에 영영 보이지 않았다.**
  ② **S20 `STALE_DAYS_DEFAULT`** — 두 벌이고 계약은 "동일 정신"이라는 주석뿐이었다. 갈리면
     스탬프 대상 집합 ≠ 소비자 유산 집합이 된다.
  ③ **G3 파싱 예산(`HEAD_BYTES`)** — 2언어 + 골든 대장 3곳. 갈리면 같은 파일을 한쪽만 짧게 본다.
  ④ **파일명 정규화 규칙** — 라벨은 게이트·HUD의 조인 키다. 한 글자만 어긋나면 조인이 조용히
     전패한다(실측: `reviewer_gemini` ↔ `reviewer-gemini`로 2시간 침묵).

Rust 측은 소스 리터럴을 읽어 대조한다. 컴파일된 바이너리가 그 값을 출력하는 CLI가 없고,
파리티 하네스처럼 rustc를 부르기엔 대상이 상수 하나라 과하다. 대신 **파싱 실패를 hard fail**로
둔다 — Rust 쪽 표현이 바뀌면 조용히 통과하는 대신 여기서 멈춰 사람이 보게 만든다.

실행: python3 test_todo_shared_constants.py
"""
import os
import re
import sys
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
REPO = os.path.dirname(os.path.dirname(BIN))                             # 저장소 루트
sys.path.insert(0, BIN)

import javis_orchestra as ORC                                            # noqa: E402
import javis_report as RP                                                # noqa: E402
import javis_todo_decl as TD                                             # noqa: E402
import javis_todo_stamp as ST                                            # noqa: E402


def rust_source(rel):
    p = os.path.join(REPO, rel)
    if not os.path.isfile(p):
        raise AssertionError("Rust 소스가 없다: %s (기계 대조 불가 = 실패)" % p)
    with open(p, encoding="utf-8") as f:
        return f.read()


def strip_line_comments(src):
    """줄 주석(`//`)을 제거한다 — **코드**를 대조해야 한다.

    주석에 옛 표현을 인용한 설명이 흔하고(이 저장소는 특히 그렇다), 주석까지 대조하면
    "왜 이렇게 고쳤는지"를 적는 순간 테스트가 결함을 오보한다.
    """
    return "\n".join(ln.split("//")[0] for ln in src.splitlines())


class PackDirEnvKeys(unittest.TestCase):
    """S19 — 팩 경로 env 키 목록·순서가 5구현에서 하나여야 한다."""

    def rust_keys(self):
        src = rust_source("src/pack.rs")
        m = re.search(r"pub const PACK_DIR_ENV_KEYS: \[&str; \d+\] = \[(.*?)\];", src, re.S)
        self.assertTrue(m, "src/pack.rs 에서 PACK_DIR_ENV_KEYS 를 찾지 못했다 "
                           "(표현이 바뀌었다면 이 대조를 함께 갱신하라 — 조용한 통과 금지)")
        keys = []
        for tok in strip_line_comments(m.group(1)).split(","):
            tok = tok.strip()
            if not tok:
                continue
            if tok == "ENV_PACK_DIR":                     # 같은 파일의 상수 = "CYS_PACK_DIR"
                mm = re.search(r'pub const ENV_PACK_DIR: &str = "([^"]+)"', src)
                self.assertTrue(mm, "ENV_PACK_DIR 상수를 찾지 못했다")
                keys.append(mm.group(1))
            else:
                mm = re.fullmatch(r'"([^"]+)"', tok)
                self.assertTrue(mm, "예상 밖 항목: %r" % tok)
                keys.append(mm.group(1))
        return keys

    def test_python_implementations_agree(self):
        self.assertEqual(RP.PACK_DIR_ENV_KEYS, ORC.PACK_DIR_ENV_KEYS)
        self.assertEqual(RP.PACK_DIR_ENV_KEYS, ST.PACK_DIR_ENV_KEYS)

    def test_python_matches_rust(self):
        self.assertEqual(list(RP.PACK_DIR_ENV_KEYS), self.rust_keys(),
                         "팩 경로 env 목록·순서가 2언어에서 갈렸다 — 생성 위치와 스캔 위치가 "
                         "갈리는 경로다(S19의 정확한 재현 조건)")

    def test_legacy_key_is_honored_by_every_implementation(self):
        """★S19 재현 축 — `AITERM_JARVIS_DIR`만 설정된 환경에서 **전원이 같은 곳**을 본다."""
        saved = {k: os.environ.get(k) for k in RP.PACK_DIR_ENV_KEYS}
        try:
            for k in RP.PACK_DIR_ENV_KEYS:
                os.environ.pop(k, None)
            os.environ["AITERM_JARVIS_DIR"] = "/tmp/legacy-pack"
            self.assertEqual(RP.pack_dir(), "/tmp/legacy-pack")
            self.assertEqual(ORC.pack_dir(), "/tmp/legacy-pack")
            self.assertEqual(ST.pack_dir(), "/tmp/legacy-pack")
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

    def test_todo_path_cli_goes_through_pack_dir(self):
        """`cys todo-path`가 자체 env 해석을 하지 않는다(S19의 발현 지점).

        종전에는 `env_compat("CYS_PACK_DIR")`(= CYS_/JAVIS_/AITERM_**PACK**_DIR)만 봐서
        `AITERM_JARVIS_DIR`를 놓쳤다. 단일 구현 경유가 계약이다.
        """
        src = rust_source("src/bin/cys.rs")
        m = re.search(r"fn run_todo_path\(.*?\n\}\n", src, re.S)
        self.assertTrue(m, "run_todo_path 를 찾지 못했다")
        body = strip_line_comments(m.group(0))
        self.assertIn("cys::pack::pack_dir()", body,
                      "todo-path 가 팩 경로 단일 구현을 경유하지 않는다")
        self.assertNotIn('env_compat("CYS_PACK_DIR")', body,
                         "todo-path 가 자체 env 해석으로 되돌아갔다(S19 재발)")


class StaleDaysDefault(unittest.TestCase):
    """S20 — 유산 임계 기본값이 생산자(스탬프)와 소비자(보고기)에서 같아야 한다."""

    def test_value_agrees(self):
        self.assertEqual(RP.STALE_DAYS_DEFAULT, ST.STALE_DAYS_DEFAULT,
                         "스탬프 대상 집합과 소비자 유산 집합이 갈린다")

    def test_env_override_agrees(self):
        """같은 knob(`CYS_TODO_STALE_DAYS`)을 같은 규칙으로 해석한다(파싱 실패·음수 포함)."""
        saved = os.environ.get("CYS_TODO_STALE_DAYS")
        try:
            for raw, want in (("", RP.STALE_DAYS_DEFAULT), ("0", 0), ("30", 30),
                              ("bogus", RP.STALE_DAYS_DEFAULT), ("-3", RP.STALE_DAYS_DEFAULT)):
                os.environ["CYS_TODO_STALE_DAYS"] = raw
                self.assertEqual(RP.stale_days(), want, raw)
                self.assertEqual(ST.default_stale_days(), want, raw)
        finally:
            if saved is None:
                os.environ.pop("CYS_TODO_STALE_DAYS", None)
            else:
                os.environ["CYS_TODO_STALE_DAYS"] = saved


class ParsingBudget(unittest.TestCase):
    """G3 파싱 예산은 2언어 + 골든 대장에서 하나다(예산 이원화 = S15의 병)."""

    def test_head_bytes_agrees_across_languages(self):
        src = rust_source("src/todo_decl.rs")
        m = re.search(r"pub const HEAD_BYTES: usize = (\d+);", src)
        self.assertTrue(m, "src/todo_decl.rs 에서 HEAD_BYTES 를 찾지 못했다")
        self.assertEqual(TD.HEAD_BYTES, int(m.group(1)))
        import json
        with open(os.path.join(SELF, "fixtures", "todo-decl", "expected.json"),
                  encoding="utf-8") as f:
            self.assertEqual(json.load(f)["head_bytes"], TD.HEAD_BYTES)

    def test_budget_is_applied_in_exactly_one_place(self):
        """★S15 회귀 핀 — `parse`가 머리말을 **다시 자르지 않는다**.

        예산이 두 곳에서 적용되면 비UTF-8 파일에서 갈린다(lossy 디코드가 1바이트를 3바이트로
        팽창시키므로 두 번째 절단이 상대가 보는 영역을 잘라먹는다). Rust 쪽에 재절단 헬퍼가
        되살아나면 여기서 잡는다 — 소스 수준 핀이지만, 이 결함은 정확히 소스 수준에서 났다.
        """
        src = strip_line_comments(rust_source("src/todo_decl.rs"))
        body = src.split("pub fn parse(")[1].split("\n}\n")[0]
        self.assertNotIn("truncate_head", body,
                         "parse 가 디코드 문자열을 다시 자른다(S15 예산 이원화 재발)")
        # Python 정본도 같은 계약이다.
        import inspect
        self.assertNotIn("[:HEAD_BYTES]", inspect.getsource(TD.parse))

    def test_consumers_go_through_the_contract_reader(self):
        """소비자는 `head_from_bytes`를 **의무 경유**한다 — 자체 절단 재구현 금지.

        종전에는 프로덕션 데몬(C2)과 CLI(C3)가 각자 재구현했고 `head_from_bytes`의 유일한
        호출자가 **테스트 덤퍼**였다. 하네스가 검증하는 읽기 경로 ≠ 프로덕션 읽기 경로 =
        파리티 CI가 초록인 채로 프로덕션만 갈릴 수 있는 상태였다.
        """
        for rel, fn in (("src/bin/cysd/governance.rs", "check_todo_with"),
                        ("src/bin/cys.rs", "todo_decl_excluded")):
            src = rust_source(rel)
            self.assertIn("head_from_bytes", src, "%s 가 계약 정본 읽기를 경유하지 않는다" % rel)
            self.assertIn(fn, src)


class MarkerDecorCharset(unittest.TestCase):
    """★W15 — 은퇴 마커 앞 **장식 문자 집합**이 2언어에서 하나의 리터럴이어야 한다.

    W13은 "장식 집합을 정의하는 순간 2언어가 갈린다"며 집합 정의를 **피했고**, 그 회피가
    reviewer1 3차 중대 ②(주석 안 부분일치 → `<!-- … STALE 무효화 대상이 **아니다** -->` 가
    파일을 은퇴시킴)의 원인이 됐다. 갈릴 여지를 없애는 방법은 집합을 피하는 것이 아니라
    집합을 한 곳에 적고 **기계로 묶는 것**이다 — 이 테스트가 그 묶음이다.
    """

    def rust_decor(self):
        src = rust_source("src/todo_decl.rs")
        m = re.search(r'pub const DECOR_CHARS: &str = "(.*?)";', src)
        self.assertTrue(m, "src/todo_decl.rs 에서 DECOR_CHARS 를 찾지 못했다")
        # Rust 문자열 리터럴의 이스케이프는 `\t` 하나뿐이다(다른 이스케이프가 생기면 여기서 멈춘다).
        raw = m.group(1)
        self.assertNotIn("\\", raw.replace("\\t", ""),
                         "DECOR_CHARS 에 미지원 이스케이프가 있다 — 대조 규칙을 갱신하라")
        return raw.replace("\\t", "\t")

    def test_charset_agrees_across_languages(self):
        self.assertEqual(TD.DECOR_CHARS, self.rust_decor())

    def test_charset_has_no_sentence_characters(self):
        """유일한 계약 — 문장 문자가 하나라도 들어가면 부정문 은퇴가 되살아난다."""
        for ch in TD.DECOR_CHARS:
            self.assertFalse(ch.isalnum(), "장식 집합에 문장 문자가 있다: %r" % ch)

    def test_skew_fallback_regex_agrees_with_the_parser(self):
        """ADR-2 스큐 사본(`javis_report.RE_LEGACY_RETIRE_SKEW`)도 같은 답을 낸다.

        정규식 문자클래스와 문자열 리터럴은 형태가 달라 문자열 대조가 불가능하므로 **행동**으로
        묶는다. 이 사본은 파서가 아예 없는 구간에서만 도는데, 그 구간에서만 부정문이 파일을
        은퇴시키면 결함은 그대로 살아 있는 것이다(발현 빈도가 낮을 뿐).
        """
        cases = (
            # (줄, 마커인가)
            ("<!-- ★★★★ STALE 무효화 (2026-07-11 삽입) ★★★★", True),
            ("<!-- ★ STALE 무효화 -->", True),
            ("<!-- ==== RETIRED ==== -->", True),
            ("<!-- javis:todo-retired -->", True),
            ("javis:todo-retired", True),
            ("stale 무효화", True),
            ("<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->", False),
            ("<!-- 은퇴시키려면 STALE 무효화 라고 적는다 -->", False),
            ("<!-- 이 파일을 javis:todo-retired 로 표시하는 방법 -->", False),
            ("<!-- 참고: retired 로 바꾸려면 -->", False),
            ("이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.", False),
            ("STALE 무효화 (2026-07-11)", False),        # 비주석 + 꼬리 텍스트 = 탈락(\\Z 앵커)
            # ★W18 교정 2 — 방어 범위는 "설명·부정문"이 아니라 **토큰의 위치**에 걸린다.
            # 토큰이 앞에 오면 설명·부정문이어도 은퇴로 읽힌다(계약의 경계 · 실물 코퍼스 0건).
            ("<!-- **STALE 무효화** 는 이 파일에 해당하지 않는다 -->", True),
            ("<!-- javis:todo-retired 마커는 레거시 표기다 -->", True),
            ("<!-- RETIRED 는 은퇴 토큰이다 -->", True),
            # 반대 방향(권장 표기) — 토큰을 문장 뒤에 두면 마커가 아니다.
            ("<!-- 이 파일은 STALE 무효화 대상이 아니다 -->", False),
            ("<!-- 레거시 표기는 javis:todo-retired 다 -->", False),
            ("<!-- 은퇴 토큰은 RETIRED 다 -->", False),
        )
        for line, want in cases:
            with self.subTest(line=line):
                self.assertEqual(TD.is_retire_marker_line(line), want, "파서")
                self.assertEqual(bool(RP.RE_LEGACY_RETIRE_SKEW.match(line)), want, "스큐 사본")

    def test_real_corpus_marker_form_survives(self):
        """실물 마커 형태(장식 접두 + 꼬리 텍스트 + `-->` 부재)는 반드시 은퇴로 남는다.

        엄격 앵커를 택했다면 실측 코퍼스의 마커 10파일이 즉시 `unclaimed`가 되어 07-26
        유령 집계가 부활한다 — 이 핀이 그 방향의 재발을 막는다.
        """
        real = ("<!-- ★★★★ STALE 무효화 (2026-07-11 dept-1 master 삽입 — "
                "이 블록이 아래 전문에 우선한다) ★★★★")
        self.assertTrue(TD.is_retire_marker_line(real))
        self.assertFalse(
            TD.is_retire_marker_line("<!-- 이 파일은 STALE 무효화 대상이 **아니다** -->"))


class LabelNormalization(unittest.TestCase):
    """라벨은 게이트·HUD의 **조인 키**다 — 정규화 규칙이 갈리면 조인이 조용히 전패한다."""

    def test_filename_to_role_is_one_rule(self):
        sys.path.insert(0, BIN)
        import javis_hud_bridge as HB
        for name, want in (("MASTER_TODO.md", "master"),
                           ("REVIEWER_GEMINI_TODO.md", "reviewer-gemini"),
                           ("REVIEWER-GEMINI_TODO.md", "reviewer-gemini"),
                           ("WORKER_2_TODO.md", "worker-2"),
                           ("CSO_TODO.md", "cso")):
            self.assertEqual(RP.node_label("/x/_round/" + name), want, name)
            self.assertEqual(ST.owner_from_filename("/x/_round/" + name), want, name)
            self.assertEqual(HB.todo_label("/x/_round/" + name, {}), want, name)


if __name__ == "__main__":
    unittest.main(verbosity=2)
