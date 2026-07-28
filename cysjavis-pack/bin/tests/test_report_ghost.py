#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_report_ghost.py — 유령 todo 결함 수정(Phase 2) + 선언 소비자 C1(M1) 회귀.

이 스위트가 지키는 것
  ① Phase 2 핫픽스 — 라벨공간 정규화(D3)·배제 분류기(D1·D2)·배제 전건 노출·stall 쿨다운(D4)
  ② 적대검증 회귀 핀 — A2 자해(체크박스 뒤 문구는 선언이 아니다)·A3 false QUIET(신선 고아는
     집계에 남는다)·SIM-2 전방호환(차기 선언 v1 status=retired를 지금 소비자가 인식한다)
  ③ 적대 입력 내성 — 신규 설치·디렉터리/깨진 심링크/무권한 파일·환경변수 오염·CLI exit code
  ④ **선언 경로(C1)** — 선언 5분기(counted/retired/foreign-scope/orphan-scope/unclaimed),
     미선언 폴백이 진행률을 빼앗지 않을 것, `unclaimed[]` 미러가 `nodes[]`를 오염시키지 않을 것,
     온보딩 방어 3규칙(정보 표기·진행률 보존·todo 0개 무소음)

원본은 master의 로컬 검증 자산(`_round/ghost-todo-fix/test_ghost_todo_fix.py` 26항목 +
`test_hostile.py` 10항목)이며, 여기서 저장소 관례(unittest·파일 직접 실행·절대경로 하드코딩
제거·mkdtemp)로 이관하고 선언 경로 케이스를 신설했다.

실행: python3 test_report_ghost.py   (unittest·표준 러너 — CI가 파일 직접 실행하는 관례 준거)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(SELF)                                              # cysjavis-pack/bin
sys.path.insert(0, BIN)
import javis_report as RP                                                # noqa: E402
import javis_report_gate as RG                                           # noqa: E402

REPORT_PY = os.path.join(BIN, "javis_report.py")


# ─────────────────────────── 공용 헬퍼 ───────────────────────────

def boxes(done, open_):
    """체크박스 본문 — done개 완료 + open_개 미완."""
    return "\n".join(["- [x] d"] * done + ["- [ ] o"] * open_) + "\n"


def status_with(roles):
    """`cys status --json` 최소 모사(귀속 판정에 필요한 role 집합만)."""
    return {"surfaces": [{"role": r, "cwd": None, "idle_secs": 0, "agent_alive": True,
                          "status": {}} for r in roles],
            "feed": {"pending": 0}, "paused": False}


class TempPackCase(unittest.TestCase):
    """임시 팩·환경변수 격리 기반 클래스 — 부작용 0(실제 홈 디렉터리 무접촉)."""

    ENV_KEYS = ("CYS_PACK_DIR", "CYS_TODO_STALE_DAYS", "CYS_TODO_DIRS",
                "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR")

    def setUp(self):
        self._env_backup = {k: os.environ.get(k) for k in self.ENV_KEYS}
        for k in self.ENV_KEYS:
            os.environ.pop(k, None)
        self.tmp = tempfile.mkdtemp(prefix="report-ghost-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.addCleanup(self._restore_env)
        self.pack = os.path.join(self.tmp, "pack")                # my_scope = "pack"
        self.pack_round = os.path.join(self.pack, "round")
        os.makedirs(self.pack_round)
        os.environ["CYS_PACK_DIR"] = self.pack

    def _restore_env(self):
        for k, v in self._env_backup.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def write(self, path, body, age_days=0):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(body)
        if age_days:
            t = time.time() - age_days * 86400
            os.utime(path, (t, t))
        return path

    def proj(self, name="proj"):
        """비정본 위치(프로젝트 `_round`) 디렉터리를 만들고 경로를 돌려준다."""
        d = os.path.join(self.tmp, name, "_round")
        os.makedirs(d, exist_ok=True)
        return d

    @staticmethod
    def labels(rep):
        return [n["node"] for n in rep["nodes"]]

    @staticmethod
    def exc_by_node(rep):
        return {e["node"]: e["excluded"] for e in rep["excluded"]}


# ─────────────────────── ① 라벨공간 정규화 (D3) ───────────────────────

class TestLabelSpace(unittest.TestCase):
    """T1·T2 — 파일명 라벨을 role 라벨공간(`-`)으로 역정규화한다.

    소문자화만 하면 `reviewer_gemini` ≠ role `reviewer-gemini`가 되어 report_gate의
    pending/todo_labels/node_is_idle 조인이 **조용히 전패**한다(배정 노드를 '무배정'으로
    오분류하고 stall 담당노드 생사를 '미지'로 보수 승격 → 영구 오발화).
    """

    def test_t1_underscore_label_normalized_to_role_space(self):
        self.assertEqual(RP.node_label("REVIEWER_GEMINI_TODO.md"), "reviewer-gemini")

    def test_t2_hyphen_variant_converges_to_same_label(self):
        self.assertEqual(RP.node_label("REVIEWER-GEMINI_TODO.md"),
                         RP.node_label("REVIEWER_GEMINI_TODO.md"))


# ─────────────────── ② 유령 배제 분류기 (사고 현장 재현) ───────────────────

class TestGhostExclusion(TempPackCase):
    """T3~T9 — 07-26 사고 현장(공유 폴더 유산 4파일)을 그대로 재현해 배제를 핀한다."""

    def setUp(self):
        super().setUp()
        p = self.proj()
        self.p = p
        # 정본(pack/round)
        self.write(os.path.join(self.pack_round, "MASTER_TODO.md"), boxes(1, 0))
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"), boxes(0, 2))
        self.write(os.path.join(self.pack_round, "CSO_TODO.md"), boxes(3, 1), age_days=40)
        # 현재 편대의 정상 작업(비정본 위치)
        self.write(os.path.join(p, "WORKER_2_TODO.md"), boxes(4, 1))
        # 유령들
        self.write(os.path.join(p, "MASTER_TODO.md"), boxes(79, 38), age_days=8)
        self.write(os.path.join(p, "REVIEWER_GEMINI_TODO.md"),
                   "<!-- ★ STALE 무효화 (teardown 유산) -->\n" + boxes(142, 2), age_days=14)
        self.write(os.path.join(p, "WORKER_PURGEFIX_TODO.md"), boxes(11, 3), age_days=9)
        self.write(os.path.join(p, "REVIEWER_CODEX_TODO.md"), boxes(20, 6), age_days=30)
        self.write(os.path.join(p, "WORKER_GHOSTFRESH_TODO.md"), boxes(2, 1))

        self.status = status_with(["master", "worker", "worker-2", "cso", "reviewer-codex"])
        self.extra = [os.path.dirname(p)]
        self.rep = RP.build_report(self.status, self.extra)
        self.exc = {os.path.basename(e["path"]): e["excluded"] for e in self.rep["excluded"]
                    if os.path.dirname(e["path"]) == os.path.realpath(p)}

    def test_t3_retire_marker_excluded(self):
        self.assertEqual(self.exc.get("REVIEWER_GEMINI_TODO.md"), "retired", str(self.exc))

    def test_t4_duplicate_label_keeps_canonical_only(self):
        self.assertEqual(self.exc.get("MASTER_TODO.md"), "shadowed", str(self.exc))
        self.assertEqual(self.labels(self.rep).count("master"), 1)

    def test_t5_orphan_label_past_threshold_excluded(self):
        self.assertEqual(self.exc.get("WORKER_PURGEFIX_TODO.md"), "orphan", str(self.exc))

    def test_t5b_fresh_orphan_stays_counted(self):
        """A3 회귀 핀 — 신선 고아를 배제하면 죽은 워커의 미완 작업이 사라지고 게이트가
        false QUIET/park에 빠진다(유령을 막으려다 진짜 사고를 숨기는 방향 오류)."""
        self.assertIn("worker-ghostfresh", self.labels(self.rep))
        self.assertNotIn("WORKER_GHOSTFRESH_TODO.md", self.exc)

    def test_t6_stale_legacy_excluded(self):
        self.assertEqual(self.exc.get("REVIEWER_CODEX_TODO.md"), "stale", str(self.exc))

    def test_t7_canonical_location_exempt_from_age(self):
        """정본 위치의 미완 작업을 침묵시켜 시야에서 잃는 쪽이 유령보다 위험하다(보수적 설계)."""
        self.assertIn("cso", self.labels(self.rep))

    def test_t7b_current_fleet_work_counted(self):
        self.assertIn("worker-2", self.labels(self.rep))

    def test_t9_aggregate_uncontaminated(self):
        """master 1/1 + worker 0/2 + cso 3/4 + worker-2 4/5 + ghostfresh 2/3 = 10/15."""
        self.assertEqual((self.rep["overall_done"], self.rep["overall_total"]), (10, 15))

    def test_t9b_all_exclusions_exposed(self):
        self.assertEqual(len(self.rep["excluded"]), 4, str(self.rep["excluded"]))

    def test_t9c_exclusion_carries_reason_path_counts(self):
        for e in self.rep["excluded"]:
            self.assertTrue(e.get("excluded") and e.get("path"))
            self.assertIn("done", e)
            self.assertIn(e.get("source"), ("decl", "heuristic"), str(e))

    def test_t9d_text_render_shows_exclusions(self):
        self.assertIn("집계 제외 4건", RP.render_text(self.rep))

    def test_t8_without_status_orphan_rule_disabled(self):
        """status 미수집 = 귀속 판정 불가 → 보수적으로 집계 유지."""
        rep = RP.build_report(None, self.extra)
        self.assertIn("worker-ghostfresh", self.labels(rep))

    def test_t8b_without_status_retired_and_stale_still_excluded(self):
        rep = RP.build_report(None, self.extra)
        self.assertEqual({e["excluded"] for e in rep["excluded"]},
                         {"retired", "shadowed", "stale"})

    def test_t12_clean_path_no_regression(self):
        """유령 없는 순수 정본 입력의 집계는 종전과 동일하고 배제는 0건이다."""
        rep = RP.build_report(self.status, [])
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (4, 7))
        self.assertEqual(rep["excluded"], [])


# ─────────────── ③ 무효화 선언의 위치 계약 (A2 자해 회귀 핀) ───────────────

class TestRetireDeclarationPosition(TempPackCase):
    """T13·T14 — 무효화 문구를 머리말 밖에서도 인정하면 **살아있는 작업이 자해**한다.

    실측: "- [ ] STALE 무효화 마커 기계 판독 구현"이라는 워커의 todo 항목이 그 워커의 파일
    전체를 배제시켰다. 그래서 선언은 첫 체크박스 이전 머리말에서만 유효하다.
    """

    def setUp(self):
        super().setUp()
        d = self.proj("sw")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "# WORKER TODO — 유령 결함 수정 중\n\n"
                   "- [x] 규명\n- [ ] STALE 무효화 마커 기계 판독 구현\n")
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- ★ STALE 무효화 (레인 종결) -->\n\n- [x] a\n- [ ] b\n")
        self.rep = RP.build_report(status_with(["worker", "cso"]), [os.path.dirname(d)])

    def test_t13_marker_after_checkbox_is_not_a_declaration(self):
        self.assertIn("worker", self.labels(self.rep))
        self.assertNotEqual(self.exc_by_node(self.rep).get("worker"), "retired")

    def test_t14_header_marker_still_retires(self):
        self.assertEqual(self.exc_by_node(self.rep).get("cso"), "retired")


# ────────────── ④ 전방호환 — 차기 선언 v1 (SIM-2 스큐 봉쇄) ──────────────

class TestForwardCompatDeclaration(TempPackCase):
    """T15·T15b — 읽는 쪽을 쓰는 쪽보다 먼저 배포한다(ADR-2 따름정리 0).

    이 핀이 없으면 마이그레이션 기간 내내 구버전 소비자가 은퇴한 todo를 계속 집계한다.
    """

    def setUp(self):
        super().setUp()
        d = self.proj("fc")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n# T\n"
                   + boxes(5, 5))
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack status=active -->\n\n# T\n"
                   + boxes(3, 1))
        self.rep = RP.build_report(status_with(["worker", "cso"]), [os.path.dirname(d)])

    def test_t15_v1_retired_recognized(self):
        self.assertEqual(self.exc_by_node(self.rep).get("worker"), "retired")

    def test_t15b_v1_active_still_counted(self):
        self.assertIn("cso", self.labels(self.rep))


# ────── ④-b 휴리스틱 폴백의 머리말 정의 (W12 — 파서와 소비자 경계 사이의 구멍) ──────

class TestHeuristicRetireHeaderMasking(TempPackCase):
    """W12 — 문서용 예시 선언이 **휴리스틱 폴백 경로**에서 살아있는 파일을 은퇴시켰다.

    master 실측 재현(2026-07-26): 머리말에 코드펜스로 감싼 예시 선언(`status=retired`)을 적고
    미완 2건을 가진 파일이 `excluded=[('worker','retired','heuristic')]`로 통째로 소실됐다.
    같은 파일을 **파서**(`javis_todo_decl`)는 이미 `unclaimed`로 옳게 판정하고 있었다 —
    W9가 파서에 G12 머리말 마스킹을 넣었는데 `javis_report.is_retired`의 레거시 휴리스틱만
    옛 정의("첫 체크박스 이전 **전량**")로 남아 경계 사이로 샜다.

    근본 원인은 설계자(master)가 `RE_RETIRED`에 심은 차기 선언 전방호환 패턴
    (`javis:todo v\\d+ … status=retired`)이다 — SIM-2 스큐 봉쇄 목적으로는 옳지만, 그 패턴은
    **선언을 설명하는 문서**에 정확히 걸린다. 패턴이 아니라 **적용 범위**가 결함이었다.

    ★★이 오분류가 유독 위험한 이유(다음 사람에게 남기는 메모):
      `retired`는 "주인이 끝났다고 명시 선언한 것"이라 마지막 방어선인 구조적 불변식
      `pending_outside_nodes`(교정 2-b)의 **면제 대상**이다. 따라서 여기서 오분류되면
      미완 작업이 있어도 `pending_outside_nodes`가 **빈 채로 남고**, 게이트의 QUIET/park
      분기가 그대로 오발동한다 — 즉 이 버킷만은 불변식이 구제해 주지 않는다.
      다른 버킷(shadowed·orphan·stale)으로 잘못 빠졌다면 미완 작업이 불변식에 걸려 살아났다.
      그래서 아래 테스트는 `nodes[]` 생존과 `pending_outside_nodes` 공집합을 **함께** 핀한다:
      전자가 깨지면 후자는 경보를 울리지 못한다는 사실 자체가 이 스위트의 요점이다.

    반대 방향(진짜 머리말 선언은 여전히 은퇴)도 같은 클래스에서 핀한다 — 마스킹을 과하게
    적용해 정당한 은퇴 선언을 놓치면 07-26 유령 집계가 그대로 재발하기 때문이다.
    """

    FENCED = ("# WORKER TODO — 선언 블록 도입 작업\n\n"
              "```markdown\n"
              "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n"
              "```\n\n"
              "- [ ] 선언 생산자 배선\n- [ ] 회귀 테스트\n")
    QUOTED = ("# WORKER TODO\n\n"
              "> 예시: <!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n"
              "- [ ] 선언 생산자 배선\n- [ ] 회귀 테스트\n")
    INDENTED = ("# WORKER TODO\n\n"
                "    <!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n"
                "- [ ] 선언 생산자 배선\n- [ ] 회귀 테스트\n")
    # 선언이 아니라 **레거시 문구만** 펜스 안에 있는 경우. 선언 경로(파서)는 이 파일을
    # `unclaimed`로 두고 지나가므로, 방어는 오직 휴리스틱 쪽에만 있다 — 여기가 뚫리면
    # "STALE 무효화 규약을 설명하는 문서"가 자기 자신을 은퇴시킨다(A2의 재판).
    LEGACY_FENCED = ("# WORKER TODO — 무효화 규약 문서\n\n"
                     "```text\n"
                     "<!-- ★ STALE 무효화 (레인 종결) -->\n"
                     "```\n\n"
                     "- [ ] 규약 문서화\n- [ ] 회귀 테스트\n")

    def _report(self, body, name="WORKER_TODO.md"):
        d = self.proj("hm")
        self.write(os.path.join(d, name), body)
        return RP.build_report(status_with(["worker"]), [os.path.dirname(d)])

    def _assert_alive(self, body, label):
        rep = self._report(body)
        self.assertIn("worker", self.labels(rep), "%s: nodes[]에서 소실" % label)
        self.assertNotIn("worker", self.exc_by_node(rep), "%s: 자해 배제" % label)
        node = [n for n in rep["nodes"] if n["node"] == "worker"][0]
        self.assertEqual((node["done"], node["total"]), (0, 2), "%s: 미완 2건 소실" % label)
        # ★불변식이 이 케이스를 못 막는다는 사실의 기계 증명 — 위 assert가 깨진 상태에서도
        #   이 목록은 비어 있으므로 QUIET/park는 조용히 오발동한다.
        self.assertEqual(rep["pending_outside_nodes"], [],
                         "%s: 살아있으면 집계 안이므로 불변식 목록은 비어야 한다" % label)

    def test_w12_fenced_example_declaration_stays_alive(self):
        self._assert_alive(self.FENCED, "펜스 안 예시 선언")

    def test_w12_blockquote_example_declaration_stays_alive(self):
        self._assert_alive(self.QUOTED, "인용문 안 예시 선언")

    def test_w12_indented_example_declaration_stays_alive(self):
        self._assert_alive(self.INDENTED, "들여쓴(4칸) 예시 선언")

    def test_w12_fenced_legacy_marker_stays_alive(self):
        self._assert_alive(self.LEGACY_FENCED, "펜스 안 레거시 문구")

    def test_w12_is_retired_agrees_with_parser_verdict(self):
        """휴리스틱과 파서가 **같은 머리말 정의**를 쓰는지 직접 핀한다(두 기준 금지)."""
        import javis_todo_decl as decl
        d = self.proj("hm2")
        for i, body in enumerate((self.FENCED, self.QUOTED, self.INDENTED)):
            p = self.write(os.path.join(d, "W%d_TODO.md" % i), body)
            self.assertFalse(RP.is_retired(p), "휴리스틱이 예시 선언을 은퇴로 읽었다")
            self.assertEqual(
                decl.classify(decl.parse(decl.read_head(p))[0], "pack", lambda s: True),
                "unclaimed", "파서 기대값이 바뀌었다(픽스처 계약 확인 필요)")

    # ── 반대 방향 회귀: 마스킹이 정당한 은퇴 선언까지 삼키면 유령이 되돌아온다 ──

    def test_w12_real_header_declaration_still_retired(self):
        rep = self._report("# WORKER TODO\n\n"
                           "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n"
                           "- [ ] 잔여\n- [ ] 잔여2\n")
        self.assertEqual(self.exc_by_node(rep).get("worker"), "retired")
        self.assertNotIn("worker", self.labels(rep))

    def test_w12_real_header_legacy_marker_still_retired(self):
        rep = self._report("<!-- ★ STALE 무효화 (레인 종결) -->\n\n# WORKER TODO\n\n"
                           "- [ ] 잔여\n- [ ] 잔여2\n")
        self.assertEqual(self.exc_by_node(rep).get("worker"), "retired")

    def test_w12_fallback_without_parser_keeps_old_definition(self):
        """ADR-2 스큐(파서 부재) — 폴백은 옛 정의로 동작하고 **예외를 던지지 않는다**.

        여기서 죽으면 팩 부분갱신 한 번에 진행% 보고 전체가 멈춘다. 대가로 그 구간에서는
        자해 위험이 남으며, 그 사실은 `is_retired` 주석에 명시돼 있다(교정 지시 1).
        """
        d = self.proj("hm3")
        p = self.write(os.path.join(d, "WORKER_TODO.md"), self.FENCED)
        saved = RP._decl
        RP._decl = None
        try:
            self.assertTrue(RP.is_retired(p))          # 옛 정의 = 펜스를 모른다(문서화된 한계)
            self.assertEqual(RP._retire_scan_lines("a\n- [ ] b\nc\n"), ["a"])
        finally:
            RP._decl = saved


# ─────────────── ⑤ 선언 소비자 C1 (M1 병행) — 신규 케이스 ───────────────

class TestDeclaredStateConsumer(TempPackCase):
    """선언이 1순위이고, 미선언은 폴백으로 **집계를 유지**한다(설계 §4-2·§4-5·§6).

    선언 5분기를 소비자 경계에서 전부 밟는다 — 파서 단위 테스트(test_todo_decl.py)는
    판정 문자열까지만 책임지고, 그 판정이 report의 어느 버킷으로 가는지는 여기가 책임진다.
    """

    def setUp(self):
        super().setUp()
        # 형제 팩 실재 여부가 foreign-scope ↔ orphan-scope를 가른다(디스크 존재 = 결정론 입력).
        os.makedirs(os.path.join(self.tmp, "pack-dept-dept-1", "round"))
        self.d = self.proj("decl")

    def build(self, roles=("master", "worker", "cso", "reviewer-gemini")):
        return RP.build_report(status_with(list(roles)), [os.path.dirname(self.d)])

    def test_decl_counted_enters_nodes(self):
        """선언 counted — scope가 내 팩이면 위치·mtime과 무관하게 집계된다."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "# 제목이 먼저 와도 된다(G1 완화)\n"
                   "<!-- javis:todo v1 owner=worker scope=pack lane=x status=active -->\n\n"
                   + boxes(3, 1), age_days=400)      # 400일 경과여도 선언이 이긴다
        rep = self.build()
        self.assertIn("worker", self.labels(rep))
        self.assertEqual(rep["excluded"], [], str(rep["excluded"]))
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (3, 4))
        self.assertEqual(rep["decl_stats"], {"total": 1, "declared": 1, "unclaimed_ratio": 0.0})

    def test_decl_retired_excluded_with_decl_source(self):
        """선언 retired — 배제하되 `source=decl`로 판정 출처를 드러낸다."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n"
                   + boxes(9, 1))
        rep = self.build()
        e = rep["excluded"]
        self.assertEqual(len(e), 1, str(e))
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("retired", "decl"))
        self.assertEqual(rep["overall_total"], 0)

    def test_decl_orphan_scope_reported_loudly(self):
        """선언 orphan-scope — 실재하지 않는 팩을 가리키는 선언은 **시끄럽게** 보고한다.

        조용히 지우면 부서 teardown·팩 개명 때 살아있는 파일이 통째로 사라져 07-11 사고를
        거울상으로 재현한다(R2 교정).

        ★2026-07-26 master 심판(교정 1): "시끄럽게"의 정의가 바뀌었다 — 텍스트에만 적는 것은
        기계 소비자에겐 침묵이다. `nodes[]`에 **남긴 채** 플래그를 단다.
        """
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack-dept-dept-99 status=active -->\n"
                   + boxes(2, 2))
        rep = self.build()
        self.assertEqual(rep["excluded"], [], str(rep["excluded"]))   # 조용한 배제 금지
        n = [x for x in rep["nodes"] if x["node"] == "worker"]
        self.assertEqual(len(n), 1, str(rep["nodes"]))
        self.assertEqual((n[0]["flag"], n[0]["source"]), ("orphan-scope", "decl"))
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (2, 4))
        self.assertIn("귀속 팩 부재(선언)", RP.render_text(rep))

    def test_decl_foreign_scope_is_quiet_but_listed(self):
        """선언 foreign-scope — 실재하는 남의 팩이면 정상(조용한 배제)이되 목록에는 남는다."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack-dept-dept-1 status=active -->\n"
                   + boxes(2, 2))
        rep = self.build()
        e = rep["excluded"]
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("foreign-scope", "decl"))
        self.assertIn("남의 팩 소유(선언)", RP.render_text(rep))

    def test_scope_exists_uses_pack_siblings_not_hardcoding(self):
        """`my_scope`는 pack_dir의 basename, `scope_exists`는 그 형제 디렉터리 존재다."""
        self.assertEqual(RP.my_scope(), "pack")
        self.assertTrue(RP.scope_exists("pack-dept-dept-1"))
        self.assertFalse(RP.scope_exists("pack-dept-dept-99"))
        for hostile in ("", None, ".", "..", "a/b"):
            self.assertFalse(RP.scope_exists(hostile), repr(hostile))

    def test_unclaimed_keeps_progress_and_mirrors_nodes(self):
        """M1의 핵심 — 미선언 파일은 **집계에 그대로 남고** 관측 버킷에 미러링된다.

        폴백이 없으면 실측상 63/63 미선언이라 진행%가 그 즉시 0/0이 된다(SIM-1).
        """
        self.write(os.path.join(self.d, "WORKER_TODO.md"), boxes(3, 1))
        rep = self.build()
        self.assertIn("worker", self.labels(rep))
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (3, 1 + 3))
        self.assertEqual(len(rep["unclaimed"]), 1)
        u = rep["unclaimed"][0]
        self.assertEqual((u["node"], u["done"], u["total"]), ("worker", 3, 1 + 3))
        self.assertEqual(u["diag"], "선언 없음")
        self.assertTrue(u["path"].endswith("WORKER_TODO.md"))

    def test_unclaimed_never_leaks_into_nodes_as_extra_record(self):
        """⚠ report_gate 무회귀 — `unclaimed[]`가 `nodes[]`에 **별도 레코드**로 섞이면
        라벨이 중복돼 stall/idle 판정이 오발화한다. 미러는 부분집합이어야 한다."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"), boxes(1, 1))
        self.write(os.path.join(self.d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack status=active -->\n" + boxes(1, 0))
        rep = self.build()
        node_paths = [n["path"] for n in rep["nodes"]]
        self.assertEqual(len(node_paths), len(set(node_paths)))
        self.assertEqual(sorted(self.labels(rep)), ["cso", "worker"])
        for u in rep["unclaimed"]:
            self.assertIn(u["path"], node_paths)      # 부분집합 미러

    def test_decl_stats_ratio_is_measurable(self):
        """P4-1 관측 지표 — 미선언 비율 < 10% 도달이 M3 전환의 게이트다."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(1, 0))
        self.write(os.path.join(self.d, "CSO_TODO.md"), boxes(1, 0))
        rep = self.build()
        self.assertEqual(rep["decl_stats"]["total"], 2)
        self.assertEqual(rep["decl_stats"]["declared"], 1)
        self.assertEqual(rep["decl_stats"]["unclaimed_ratio"], 0.5)

    def test_future_version_retired_still_excluded_by_fallback(self):
        """구→신 반대 방향 스큐 — 파서가 모르는 미래 버전(v2+)은 미선언으로 떨어지지만,
        폴백 휴리스틱의 무효화 마커 인식이 `status=retired`를 붙잡는다.

        폴백을 지금 지우면(M3 조기 전환) 이 경로가 통째로 뚫린다 — 그래서 M1은 병행이다.
        """
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v2 owner=worker scope=pack status=retired -->\n"
                   + boxes(4, 4))
        rep = self.build()
        e = rep["excluded"]
        self.assertEqual(len(e), 1, str(e))
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("retired", "heuristic"))

    def test_broken_declaration_falls_back_with_diagnosis(self):
        """G9 — 깨진 선언은 미선언으로 떨어지되 **무엇이 틀렸는지**를 함께 알려준다.

        조용한 실패가 채택을 막는 진짜 원인이다(SIM-3).
        """
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   '<!-- javis:todo v1 owner="worker" scope=pack status=active -->\n'
                   + boxes(1, 1))
        rep = self.build()
        self.assertIn("worker", self.labels(rep))            # 집계는 유지(fail-open)
        self.assertEqual(len(rep["unclaimed"]), 1)
        self.assertTrue(rep["unclaimed"][0]["diag"], "진단 사유가 비었다")


class TestOnboardingDefense(TempPackCase):
    """§6 온보딩 방어 3규칙 — 신규 사용자의 첫 경험을 지키는 설계 불변식."""

    def test_rule3_zero_todo_prints_no_unclaimed_section(self):
        """③ todo가 0개면 미선언 섹션 자체를 출력하지 않는다(신규 설치 무소음)."""
        rep = RP.build_report(None, [])
        self.assertEqual(rep["unclaimed"], [])
        self.assertEqual(rep["decl_stats"], {"total": 0, "declared": 0, "unclaimed_ratio": 0.0})
        text = RP.render_text(rep)
        self.assertNotIn("미선언", text)
        self.assertNotIn("선언 미보유", text)

    def test_rule1_unclaimed_is_info_not_warning(self):
        """① 정보(ℹ)이지 경고(⚠)가 아니다.

        `⚠`는 javis_gate_check의 WARNING_KEYWORDS 원소라 쓰는 순간 게이트 WARN으로 승격된다
        — 온보딩 사용자에게 그건 신호가 아니라 결함이다.
        """
        d = self.proj("ob")
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(1, 1))
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        line = [ln for ln in RP.render_text(rep).splitlines() if "선언 미보유" in ln]
        self.assertEqual(len(line), 1, RP.render_text(rep))
        self.assertIn("ℹ", line[0])
        self.assertNotIn("⚠", line[0])
        for kw in ("idle", "stall", "실패", "승인", "컨텍스트"):
            self.assertNotIn(kw, line[0])

    def test_rule2_unclaimed_progress_not_taken_away(self):
        """② 미선언이라는 이유로 사용자의 진행률을 빼앗지 않는다."""
        d = self.proj("ob2")
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(7, 3))
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (7, 10))
        self.assertIn("70%", RP.render_text(rep))

    def test_long_unclaimed_list_discloses_truncation(self):
        """무언의 절삭 금지 — 요약했다는 사실을 명시하고 전량은 --json에 담는다."""
        d = self.proj("ob3")
        roles = []
        for i in range(9):
            self.write(os.path.join(d, "WORKER_%d_TODO.md" % i), boxes(1, 1))
            roles.append("worker-%d" % i)
        rep = RP.build_report(status_with(roles), [os.path.dirname(d)])
        self.assertEqual(len(rep["unclaimed"]), 9)                 # JSON에는 전량
        text = RP.render_text(rep)
        self.assertIn("외 4건 생략", text)
        self.assertIn("--json", text)


# ─────────────────── ⑥ stall 쿨다운 (D4 무한 발화 수정) ───────────────────

class TestStallCooldown(unittest.TestCase):
    """T10~T11 — 승격 조건이 한 번 성립하면 매 5분 영구 발화하던 결함을 상한으로 억제한다.

    실측: 유산 todo가 140주기(약 11.6시간) 연속 5분마다 stall 승격했다.
    """

    GREP = {"nodes": [{"node": "worker", "done": 1, "total": 5}],
            "live_nodes": [{"role": "worker", "idle_secs": 999,
                            "agent_alive": True, "context_pct": 5}],
            "idle_nodes": [{"role": "worker", "idle_secs": 999,
                            "agent_alive": True, "context_pct": 5}]}
    T0 = 1_000_000.0

    def fire_cycles(self, n=20, now_epoch=True):
        counters = {"nodes": {}}
        fired = []
        for i in range(n):
            epoch = self.T0 + i * 300 if now_epoch else 0
            if RG.build_stall_warnings(counters, self.GREP, 5, 6, "ts", epoch):
                fired.append(i)
        return counters, fired

    def test_t10_stall_promotes_at_least_once(self):
        _, fired = self.fire_cycles()
        self.assertGreaterEqual(len(fired), 1)

    def test_t10b_refire_suppressed_within_cooldown(self):
        _, fired = self.fire_cycles()
        self.assertLessEqual(len(fired), 2, "발화 %s" % fired)

    def test_t10c_refires_after_cooldown_window(self):
        _, fired = self.fire_cycles()
        self.assertEqual(len(fired), 2, str(fired))
        self.assertGreaterEqual(fired[1] - fired[0],
                                RG.STALL_COOLDOWN_SECS // (5 * 60), str(fired))

    def test_t10d_vendor_behaviour_reproduced_without_epoch(self):
        """대조군 — `now_epoch` 미전달(구 호출 규약)이면 쿨다운이 꺼져 vendor의 무한 발화가
        그대로 재현된다. 이 대비가 없으면 "고쳤다"는 주장에 반증 가능성이 없다."""
        _, fired_old = self.fire_cycles(now_epoch=False)
        _, fired_new = self.fire_cycles()
        self.assertGreaterEqual(len(fired_old), 13,
                                "vendor %d회 / 수정본 %d회" % (len(fired_old), len(fired_new)))
        self.assertLess(len(fired_new), len(fired_old))

    def test_t11_progress_resumption_resets_counter_and_cooldown(self):
        counters = {"nodes": {}}
        for i in range(8):
            RG.build_stall_warnings(counters, self.GREP, 5, 6, "ts", self.T0 + i * 300)
        moved = {"nodes": [{"node": "worker", "done": 2, "total": 5}],
                 "live_nodes": self.GREP["live_nodes"], "idle_nodes": self.GREP["idle_nodes"]}
        RG.build_stall_warnings(counters, moved, 5, 6, "ts", self.T0 + 8 * 300)
        st = counters["nodes"]["worker"]
        self.assertEqual((st["count"], st["last_stall_fired"]), (0, 0), str(st))


# ───────────────────────── ⑦ 적대 입력 내성 ─────────────────────────

class TestHostileInputs(TempPackCase):
    """H1~H5 — 신규 설치·적대 파일·환경변수 오염에서 예외 없이 exit 0."""

    def test_h1_fresh_install_no_round_dir(self):
        fresh = os.path.join(self.tmp, "freshpack")
        os.makedirs(fresh)
        os.environ["CYS_PACK_DIR"] = fresh
        rep = RP.build_report(None, [])
        self.assertEqual(rep["overall_total"], 0)
        self.assertEqual(rep["excluded"], [])
        self.assertEqual(rep["unclaimed"], [])

    def _hostile_pack(self):
        rd = self.pack_round
        os.makedirs(os.path.join(rd, "DIR_TODO.md"))                  # 이름만 todo인 디렉터리
        os.symlink(os.path.join(self.tmp, "nonexistent"),
                   os.path.join(rd, "BROKEN_TODO.md"))                # 깨진 심링크
        self.write(os.path.join(rd, "OK_TODO.md"), boxes(1, 1))
        noperm = self.write(os.path.join(rd, "NOPERM_TODO.md"), boxes(0, 1))
        os.chmod(noperm, 0o000)
        self.addCleanup(os.chmod, noperm, 0o600)
        return rd

    def test_h2_hostile_files_do_not_raise(self):
        self._hostile_pack()
        rep = RP.build_report(None, [])
        self.assertIsInstance(rep["nodes"], list)

    def test_h2b_valid_file_still_counted_among_hostile(self):
        self._hostile_pack()
        rep = RP.build_report(None, [])
        self.assertIn("ok", self.labels(rep))

    def test_h3_round_path_is_a_file(self):
        proj = os.path.join(self.tmp, "projfile")
        os.makedirs(proj)
        self.write(os.path.join(proj, "_round"), "x")
        rep = RP.build_report(None, [proj])                            # 예외 없이 통과해야 한다
        self.assertIsInstance(rep["nodes"], list)

    def _cli(self, extra_env=None, args=()):
        """서브프로세스 엔드투엔드 — PATH에서 `cys`를 제거해 데몬 의존을 끊는다(결정론)."""
        emptybin = os.path.join(self.tmp, "emptybin")
        os.makedirs(emptybin, exist_ok=True)
        env = dict(os.environ, CYS_PACK_DIR=self.pack, PATH=emptybin)
        env.update(extra_env or {})
        return subprocess.run([sys.executable, REPORT_PY] + list(args),
                              capture_output=True, text=True, env=env, timeout=60)

    def test_h4_cli_exit_zero_both_modes(self):
        self._hostile_pack()
        for args in ((), ("--json",)):
            r = self._cli(args=args)
            self.assertEqual(r.returncode, 0, (args, r.stderr[-300:]))
        payload = json.loads(self._cli(args=("--json",)).stdout)
        for key in ("nodes", "excluded", "unclaimed", "pending_outside_nodes", "decl_stats"):
            self.assertIn(key, payload)

    def test_h5_polluted_stale_days_env(self):
        self._hostile_pack()
        for v in ("abc", "-5", "", "999999"):
            r = self._cli(extra_env={"CYS_TODO_STALE_DAYS": v}, args=("--json",))
            self.assertEqual(r.returncode, 0, (v, r.stderr[-300:]))

    def test_h7_non_utf8_todo_is_counted_lossy(self):
        """비UTF-8 todo의 2언어 정합 핀(교정 6) — 데몬 `governance.rs::check_todo`가 같은
        바이트열을 `String::from_utf8_lossy`로 읽어 **같은 (1,2)** 를 집계한다.

        종전 데몬은 `read_to_string` 실패로 `continue`(등재 0·캐시 갱신 0)였고 Python만
        집계했다 — 같은 파일을 두고 데몬은 "없음", 팩은 "있음"이라 말하는 조용한 갈림이었다.
        Rust 측 핀은 `governance.rs::non_utf8_todo_is_lossy_decoded_like_python`.
        """
        d = self.proj("nonutf8")
        p = os.path.join(d, "WORKER_TODO.md")
        with open(p, "wb") as f:
            f.write("<!-- javis:todo v1 owner=worker scope=pack status=active -->\n"
                    .encode("utf-8"))
            f.write(b"\n# \xff\xfe\x80 (\xeb\x81\xa8\xec\xa7\x84 UTF-8)\n")
            f.write(b"- [x] \xff\xfe\n- [ ] \x80\n")
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        self.assertEqual([(n["node"], n["done"], n["total"]) for n in rep["nodes"]],
                         [("worker", 1, 2)])

    def test_h6_missing_parser_module_degrades_to_heuristic(self):
        """ADR-2 스큐 안전 — 파서가 없는 구버전 팩에서도 소비자는 계속 돌아야 한다."""
        d = self.proj("skew")
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(2, 2))
        saved = RP._decl
        RP._decl = None
        try:
            rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        finally:
            RP._decl = saved
        self.assertIn("worker", self.labels(rep))
        self.assertEqual((rep["overall_done"], rep["overall_total"]), (2, 4))
        self.assertEqual(rep["decl_stats"]["declared"], 0)


# ══════════════════════ ⑧ reviewer1 BLOCK 회귀 (치명 1·2·3) ══════════════════════
#
# 아래 3클래스는 reviewer1이 **재현 가능한 결함**으로 상신하고 master가 교정을 심판한 건이다.
# 각 테스트는 reviewer1의 재현 시나리오를 그대로 코드화한다 — 실패하면 그 사고가 그대로
# 되돌아온 것이다.

class TestReviewer1Critical1OrphanScopeStaysCounted(TempPackCase):
    """【치명 1】 `orphan-scope`를 `nodes[]`에서 빼면 게이트가 false QUIET/park로 간다.

    사슬: 조용한 배제 → `nodes[]` 이탈 → `in_progress_tasks` 공집합 → QUIET 성립 → 세션 주차.
    "시끄러운 보고"가 사람이 읽는 텍스트에만 있고 기계 소비자에겐 완전 침묵이었다(A3 재발).
    도달 조건은 scope 오타 1글자·부서 팩 개명·teardown = 이 조직에서 실제로 일어난 사건이다.

    master 심판: `nodes[]`에 남기고 플래그를 단다. 조용한 배제는 `retired`(명시 은퇴)와
    `foreign-scope`(실재하는 다른 팩이 주인) 둘뿐 — Rust `todo_is_countable`과 동일 정책.
    """

    def setUp(self):
        super().setUp()
        self.d = self.proj("c1")
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack-dept-dept-99 status=active -->\n"
                   + boxes(0, 2))
        self.rep = RP.build_report(
            {"surfaces": [{"role": "worker", "cwd": None, "idle_secs": 999,
                           "agent_alive": True, "status": {}}],
             "feed": {"pending": 0}, "paused": False},
            [os.path.dirname(self.d)])

    def test_orphan_scope_remains_in_nodes_with_flag(self):
        self.assertEqual(self.rep["excluded"], [], str(self.rep["excluded"]))
        self.assertEqual([n["node"] for n in self.rep["nodes"]], ["worker"])
        self.assertEqual(self.rep["nodes"][0]["flag"], "orphan-scope")

    def test_gate_sees_the_pending_work(self):
        """기계 소비자에게 보이는가 — 이것이 결함의 본질이었다."""
        self.assertTrue(RG.in_progress_tasks(self.rep))

    def test_gate_does_not_go_quiet(self):
        self.assertFalse(RG.quiet_branch_holds(self.rep))

    def test_foreign_scope_and_retired_remain_the_only_quiet_exclusions(self):
        """조용한 배제는 정확히 둘뿐이라는 정책을 핀한다(경계가 흐려지면 치명 1이 재발한다)."""
        os.makedirs(os.path.join(self.tmp, "pack-dept-dept-1", "round"), exist_ok=True)
        d = self.proj("c1b")
        self.write(os.path.join(d, "MASTER_TODO.md"),
                   "<!-- javis:todo v1 owner=master scope=pack status=retired -->\n" + boxes(0, 2))
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack-dept-dept-1 status=active -->\n"
                   + boxes(0, 2))
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack-dept-dept-99 status=active -->\n"
                   + boxes(0, 2))
        rep = RP.build_report(status_with(["master", "cso", "worker"]), [os.path.dirname(d)])
        self.assertEqual({e["node"]: e["excluded"] for e in rep["excluded"]},
                         {"master": "retired", "cso": "foreign-scope"})
        self.assertEqual([n["node"] for n in rep["nodes"]], ["worker"])


class TestReviewer1Critical2FalseQuietInvariant(TempPackCase):
    """【치명 2】 `shadowed` 배제의 두 번째 false QUIET 경로 + 구조적 불변식.

    reviewer1 재현: 완료된 선언 파일(1/1)이 미완의 미선언 파일(0/2)을 정본 선출에서 밀어내
    `nodes: [worker 1/1]`만 남고 QUIET이 성립했다. 정렬키가 **미완 여부를 보지 않았다**.
    도달성: 실측 63/63 미선언 상태에서 마이그레이션 중 **정상 상태**다.

    master 심판은 두 겹이다 — (a) 정렬키에 미완 우선 삽입, (b) 구조적 불변식.
    (b)가 핵심이다: 버킷 분류 실수가 **다시 나도** park 오발동으로 번지지 않게 한다.
    """

    IDLE = {"surfaces": [{"role": "worker", "cwd": None, "idle_secs": 999,
                          "agent_alive": True, "status": {}}],
            "feed": {"pending": 0}, "paused": False}

    def test_a_sort_key_prefers_pending_over_completed(self):
        """(a) 동급 경쟁에서 **살아있는 작업이 완료된 파일에 밀리지 않는다**."""
        d1, d2 = self.proj("c2a1"), self.proj("c2a2")
        p = self.write(os.path.join(d1, "WORKER_TODO.md"), boxes(0, 2))
        os.utime(p, (time.time() - 3600, time.time() - 3600))    # 미완 쪽이 **더 오래된** mtime
        self.write(os.path.join(d2, "WORKER_TODO.md"), boxes(1, 0))
        rep = RP.build_report(status_with(["worker"]),
                              [os.path.dirname(d1), os.path.dirname(d2)])
        self.assertEqual([(n["node"], n["done"], n["total"]) for n in rep["nodes"]],
                         [("worker", 0, 2)])
        self.assertEqual(self.exc_by_node(rep), {"worker": "shadowed"})

    def test_b_invariant_blocks_quiet_when_shadowed_work_is_pending(self):
        """(b) 정렬 상위 키가 달라 shadow가 그대로 나는 조합에서도 QUIET은 성립하지 않는다.

        reviewer1의 원 재현 그대로 — 정본위치의 **선언된 완료 파일**이 비정본의 미선언
        미완 파일을 이긴다(선언 보유 > 정본위치가 상위 키이므로 (a)로는 안 뒤집힌다).
        """
        d = self.proj("c2b")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(1, 0))
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(0, 2))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        self.assertEqual([(n["node"], n["done"], n["total"]) for n in rep["nodes"]],
                         [("worker", 1, 1)])                    # shadow 자체는 그대로 난다
        self.assertEqual(RG.in_progress_tasks(rep), [])         # ①은 여전히 공집합
        self.assertTrue(RG.all_nodes_idle(rep))                 # ③도 성립
        po = RG.pending_outside_nodes(rep)                      # ②가 막는다
        self.assertEqual([(x["node"], x["bucket"], x["done"], x["total"]) for x in po],
                         [("worker", "shadowed", 0, 2)])
        self.assertFalse(RG.quiet_branch_holds(rep), "false QUIET이 재발했다")

    def test_b_invariant_exempts_only_owner_declared_dispositions(self):
        """불변식의 적용 범위 — 면제는 **주인이 처분을 명시한 것**뿐이다(master 심판).

        면제  `retired`(끝났다고 선언) · `foreign-scope`(실재하는 남의 팩이 주인이라고 선언)
        비면제 `shadowed`·`orphan`·`stale`(우리 추론) · `orphan-scope`·`unclaimed`(주인 불명)
        """
        os.makedirs(os.path.join(self.tmp, "pack-dept-dept-1", "round"), exist_ok=True)
        d = self.proj("c2c")
        self.write(os.path.join(d, "MASTER_TODO.md"),           # retired = 면제
                   "<!-- javis:todo v1 owner=master scope=pack status=retired -->\n" + boxes(0, 9))
        self.write(os.path.join(d, "AGY_TODO.md"),              # foreign-scope = 면제
                   "<!-- javis:todo v1 owner=agy scope=pack-dept-dept-1 status=active -->\n"
                   + boxes(0, 4))
        self.write(os.path.join(d, "CSO_TODO.md"),              # stale = 미면제
                   boxes(0, 3), age_days=40)
        rep = RP.build_report(status_with(["master", "agy", "cso"]), [os.path.dirname(d)])
        buckets = {(x["node"], x["bucket"]) for x in RG.pending_outside_nodes(rep)}
        self.assertEqual(buckets, {("cso", "stale")}, str(rep["pending_outside_nodes"]))

    def test_b_foreign_scope_pending_alone_still_allows_quiet(self):
        """★foreign-scope 면제(master 심판 2026-07-26) — 남의 레인 미완이 우리를 영구 주차시키지 않는다.

        내부 정합성이 근거다: 우리는 이 파일을 "남의 것"이라며 `nodes[]` 집계에서 이미 조용히
        뺐다. 그런데 같은 파일이 QUIET은 막는다면 "진행률에는 안 세지만 영구히 주차는 못 하게
        한다"는 모순이 된다. 그 파일은 **실재하는** 그 팩의 게이트가 본다.
        """
        os.makedirs(os.path.join(self.tmp, "pack-dept-dept-1", "round"), exist_ok=True)
        d = self.proj("c2e")
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack-dept-dept-1 status=active -->\n"
                   + boxes(0, 3))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        self.assertEqual(self.exc_by_node(rep), {"cso": "foreign-scope"})   # 조용한 배제는 유지
        self.assertEqual(rep["pending_outside_nodes"], [],
                         "남의 팩 소유 선언이 우리 QUIET을 막고 있다(배제/면제 근거 불일치)")
        self.assertTrue(RG.quiet_branch_holds(rep))

    def test_b_orphan_scope_pending_still_blocks_quiet(self):
        """★대칭 핀 — `orphan-scope`는 면제가 **아니다**(주인 불명 = 우리가 마지막 관측자).

        `foreign-scope`와 한 글자 차이지만 의미가 정반대다: 가리키는 팩이 **실재하지 않으므로**
        그 미완 작업을 볼 주체가 우리 말고 없다. 우리가 조용해지면 세상에서 사라진다.
        면제 기준이 "선언이 있는가"로 느슨해지면 이 방어가 통째로 뚫린다.
        """
        d = self.proj("c2f")
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack-dept-dept-99 status=active -->\n"
                   + boxes(0, 3))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        # orphan-scope는 애초에 nodes[]에 남으므로 ①에서 잡힌다(교정 1) — 이중 방어의 첫 겹.
        self.assertEqual([n["node"] for n in rep["nodes"]], ["cso"])
        self.assertTrue(RG.in_progress_tasks(rep))
        self.assertFalse(RG.quiet_branch_holds(rep))

        # 둘째 겹: nodes[]에서 밀려나도(같은 라벨 정본에 shadow) 면제되지 않는다.
        self.write(os.path.join(self.pack_round, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack status=active -->\n" + boxes(2, 0))
        rep2 = RP.build_report(self.IDLE, [os.path.dirname(d)])
        self.assertEqual(RG.in_progress_tasks(rep2), [])
        self.assertEqual([(x["node"], x["bucket"]) for x in RG.pending_outside_nodes(rep2)],
                         [("cso", "shadowed")])
        self.assertFalse(RG.quiet_branch_holds(rep2))

    def test_b_heuristic_retired_with_pending_blocks_quiet(self):
        """★W13 교정 1(b) 뒤집기 — 면제는 **파서가 확정한 판정**(`source=decl`)에만 준다.

        종전 논거는 "판정 주체는 휴리스틱이어도 판정 **근거**는 주인이 적은 명시 선언이다"였다.
        그러나 면제는 *"주인이 명시했다"*의 신뢰도에 붙는 특권인데 이 경로는 파서가 확정하지
        않은 **추론**이다 — 실제로 치명 A의 자해(산문 한 줄이 파일을 은퇴시킴)가 정확히 이
        경로로 들어와, 미완 2건을 가진 살아있는 파일이 마지막 방어선인 `pending_outside_nodes`
        에조차 뜨지 않았다. 추론에 특권을 주면 불변식 자체가 무력해진다.

        여기서는 **미완이 남은 채 은퇴로 읽힌** 파일을 쓴다 — 이제 시끄러워져야 하는 상태다.
        """
        d = self.proj("c2g")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v2 owner=worker scope=pack status=retired -->\n" + boxes(0, 5))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        e = rep["excluded"]
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("retired", "heuristic"))
        self.assertEqual([(x["node"], x["bucket"], x["source"])
                          for x in rep["pending_outside_nodes"]],
                         [("worker", "retired", "heuristic")])
        self.assertFalse(RG.quiet_branch_holds(rep))

    def test_b_heuristic_retired_without_pending_still_allows_quiet(self):
        """대칭 핀 — 면제를 잃어도 **진짜 은퇴 파일은 영향이 없다**(master 심판의 근거).

        은퇴한 파일에는 미완 항목이 남아 있지 않으므로 `pending_outside_nodes`(미완 전건 목록)에
        애초에 뜨지 않는다. 이 테스트가 없으면 교정 1(b)가 "스큐 구간의 정상 은퇴까지 주차를
        막는다"는 반론에 반증을 제시하지 못한다.
        """
        d = self.proj("c2g2")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v2 owner=worker scope=pack status=retired -->\n" + boxes(5, 0))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        e = rep["excluded"]
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("retired", "heuristic"))
        self.assertEqual(rep["pending_outside_nodes"], [])
        self.assertTrue(RG.quiet_branch_holds(rep))

    def test_b_parser_confirmed_retired_stays_exempt(self):
        """면제의 잔존 범위 — 파서가 확정한 은퇴(`source=decl`)는 미완이 남아도 면제다.

        주인이 v1 문법으로 명시 선언했고 파서가 그 선언을 **확정**했다. 여기까지가 특권의 경계다.
        """
        d = self.proj("c2g3")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n" + boxes(0, 5))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        e = rep["excluded"]
        self.assertEqual((e[0]["excluded"], e[0]["source"]), ("retired", "decl"))
        self.assertEqual(rep["pending_outside_nodes"], [])
        self.assertTrue(RG.quiet_branch_holds(rep))

    def test_b_invariant_is_skew_safe_for_old_reports(self):
        """구버전 보고기(필드 부재)에서는 종전 동작 — 양측 상호 전제 금지(ADR-2)."""
        old = {"nodes": [], "live_nodes": [{"role": "worker", "idle_secs": 999,
                                            "agent_alive": True}]}
        self.assertEqual(RG.pending_outside_nodes(old), [])
        self.assertTrue(RG.quiet_branch_holds(old))

    def test_b_invariant_does_not_suppress_normal_quiet(self):
        """정상 상태(미완 0)에서는 QUIET이 그대로 성립해야 한다 — 불변식이 주차를 죽이면 안 된다."""
        d = self.proj("c2d")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(3, 0))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        self.assertEqual(rep["pending_outside_nodes"], [])
        self.assertTrue(RG.quiet_branch_holds(rep))


class TestReviewer1Critical3OwnerIsTheLabel(TempPackCase):
    """【치명 3】 `owner`를 아무도 소비하지 않았다 — 라벨이 여전히 파일명 파생이었다.

    G5가 `owner`를 필수로 강제하면서 값의 정합성은 검증도 사용도 하지 않았다.
    결과: `owner=master`인 `WORKER_TODO.md`를 아무도 잡지 못한다(D3 미해소).

    master 심판: 선언이 유효하면 라벨은 `decl.owner`. 없으면 `node_label(path)` 폴백.
    """

    def setUp(self):
        super().setUp()
        self.d = self.proj("c3")

    def test_owner_overrides_filename_derived_label(self):
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=master scope=pack status=active -->\n" + boxes(0, 2))
        rep = RP.build_report(status_with(["master", "worker"]), [os.path.dirname(self.d)])
        self.assertEqual([n["node"] for n in rep["nodes"]], ["master"])

    def test_owner_label_lands_in_role_label_space(self):
        """⚠ 라벨은 report_gate의 pending·stall·idle **조인 키**다.

        role 라벨공간은 하이픈(`reviewer-gemini`)이다. owner를 그대로 쓰면 언더스코어 표기가
        영원히 조인되지 않는다 — `node_label`과 **같은 정규화**로 수렴함을 핀한다.
        """
        self.assertEqual(RP.decl_label({"owner": "REVIEWER_GEMINI"}),
                         RP.node_label("REVIEWER_GEMINI_TODO.md"))
        self.assertEqual(RP.decl_label({"owner": "reviewer-gemini"}), "reviewer-gemini")

    def test_owner_label_joins_with_gate_stall_and_idle(self):
        """조인 실증 — owner 라벨로 stall 승격과 idle 판정이 실제로 붙는가."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=reviewer-gemini scope=pack status=active -->\n"
                   + boxes(0, 2))
        status = {"surfaces": [{"role": "reviewer-gemini", "cwd": None, "idle_secs": 999,
                                "agent_alive": True, "status": {}}],
                  "feed": {"pending": 0}, "paused": False}
        rep = RP.build_report(status, [os.path.dirname(self.d)])
        self.assertEqual(RG.node_is_idle(rep, "reviewer-gemini"), True)
        counters, fired = {"nodes": {}}, []
        for i in range(8):                                   # stall 임계(6주기) 초과
            fired += RG.build_stall_warnings(counters, rep, 5, 6, "ts",
                                             1_700_000_000 + i * 300)
        self.assertTrue(fired, "owner 라벨이 stall 조인에 붙지 않았다")
        self.assertEqual(fired[0]["evt_fields"]["agent"], "reviewer-gemini")
        # 파일명 파생 라벨(`worker`)로는 조인이 전패했을 것 — 그것이 D3의 정체였다.
        self.assertNotIn("worker", [w["evt_fields"]["agent"] for w in fired])

    def test_sentinel_and_missing_owner_fall_back_to_filename(self):
        """ADR-4 C-3 센티널(`"?"` = 레거시 은퇴, 주인 미상)과 미선언은 파일명 폴백."""
        self.assertIsNone(RP.decl_label({"owner": "?", "scope": "?", "status": "retired"}))
        self.assertIsNone(RP.decl_label(None))
        self.assertIsNone(RP.decl_label({"owner": ""}))
        self.write(os.path.join(self.d, "CSO_TODO.md"),
                   "<!-- ★ STALE 무효화 (레인 종결) -->\n" + boxes(0, 1))
        rep = RP.build_report(status_with(["cso"]), [os.path.dirname(self.d)])
        self.assertEqual(self.exc_by_node(rep), {"cso": "retired"})   # 파일명 폴백으로 라벨 유지

    def test_broken_declaration_falls_back_to_filename_label(self):
        """선언이 깨졌으면 owner를 신뢰할 수 없다 — 파일명 폴백(fail-open)."""
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   '<!-- javis:todo v1 owner="master" scope=pack status=active -->\n'
                   + boxes(0, 2))
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(self.d)])
        self.assertEqual([n["node"] for n in rep["nodes"]], ["worker"])


# ══════════════ ⑨ reviewer1 2차 BLOCK 회귀 (W13 치명 A·B · 중대 C·D · 경미 E) ══════════════
#
# reviewer1이 **재현 가능한 결함**으로 재상신하고 master가 전건 심판한 5건이다. 지난 라운드에서
# 파서(W9)와 소비자(W10)를 나눠 고쳤더니 **경계 사이로 자해가 새어나갔다** — 각자 자기 범위는
# 정확히 고쳤는데 인계점이 비었다. 그래서 아래 테스트는 전부 **양쪽 판정을 함께** 확인한다.

class TestW13CriticalARetireMarkerAnchor(TempPackCase):
    """【치명 A】 무앵커 부분일치가 **산문 한 줄로 파일을 지운다**.

    G12 마스킹은 펜스·인용·들여쓰기만 덮으므로 **평범한 머리말 산문**이 그대로 자해했다.
    ★가장 아픈 대목: 그 문구의 출처가 우리 자신이다 — 이번 브랜치가 디렉티브·템플릿에
    *"레인이 끝나면 status=retired 로 갱신하라"* 를 넣었고, **워커가 그 지침을 자기 todo
    머리말에 적으면 자해**했다.

    master 심판 2건을 여기서 함께 핀한다.
      (a) 은퇴 마커 판정을 **줄 전체 앵커**로 좁힌다(Python·Rust 동일 규칙).
      (b) `_owner_declared` 면제를 **파서가 확정한 판정**에만 준다(휴리스틱 경로에서 제거).
    """

    # reviewer1 재현 입력 2종 — 앞은 휴리스틱 경로, 뒤는 **파서 확정 판정**(Rust 데몬 동조).
    PROSE_RULE = ("# WORKER TODO — 선언 도입\n\n"
                  "규약: 레인이 끝나면 javis:todo v1 선언의 status=retired 로 바꾼다.\n\n"
                  "- [ ] 미완1\n- [ ] 미완2\n")
    PROSE_STALE = ("# WORKER TODO\n\n"
                   "이번 작업 목표: STALE 무효화 마커를 기계가 읽도록 구현한다.\n\n"
                   "- [ ] 미완1\n- [ ] 미완2\n")

    IDLE = {"surfaces": [{"role": "worker", "cwd": None, "idle_secs": 999,
                          "agent_alive": True, "status": {}}],
            "feed": {"pending": 0}, "paused": False}

    def _report(self, body):
        d = self.proj("w13a")
        self.p = self.write(os.path.join(d, "WORKER_TODO.md"), body)
        return RP.build_report(self.IDLE, [os.path.dirname(d)])

    def _assert_survives(self, body, label):
        rep = self._report(body)
        self.assertEqual([(n["node"], n["done"], n["total"]) for n in rep["nodes"]],
                         [("worker", 0, 2)], "%s: nodes[]에서 소실" % label)
        self.assertEqual(rep["excluded"], [], "%s: 자해 배제" % label)
        self.assertFalse(RG.quiet_branch_holds(rep), "%s: false QUIET" % label)
        # 소비자 3면이 **같은 판정**인가 — 이번 결함의 근본이 판정 기준의 갈림이었다.
        self.assertFalse(RP.is_retired(self.p), "%s: 휴리스틱이 은퇴로 읽었다" % label)
        import javis_todo_decl as decl
        self.assertEqual(
            decl.classify(decl.parse(decl.read_head(self.p))[0], "pack", lambda s: True),
            "unclaimed", "%s: 파서가 은퇴로 읽었다(= Rust 데몬도 같이 지운다)" % label)

    def test_a_prose_mentioning_status_retired_does_not_retire(self):
        self._assert_survives(self.PROSE_RULE, "규약 산문(휴리스틱 경로)")

    def test_a_prose_mentioning_stale_marker_does_not_retire(self):
        self._assert_survives(self.PROSE_STALE, "STALE 무효화 산문(파서 확정 경로)")

    def test_a_directive_text_itself_is_safe_in_a_worker_todo(self):
        """발현원 봉쇄 — 우리 디렉티브 문구를 그대로 머리말에 붙여도 자해하지 않는다."""
        self._assert_survives(
            "# WORKER TODO\n\n"
            "> 절대지침: 레인이 끝나면 `javis:todo v1` 선언의 status=retired 로 갱신하라.\n"
            "지침 원문: 레인이 끝나면 status=retired 로 갱신하라(STALE 무효화 마커와 동치).\n\n"
            "- [ ] 미완1\n- [ ] 미완2\n", "디렉티브 문구")

    def test_a_real_header_marker_still_retires(self):
        """반대 방향 — 진짜 마커 줄은 여전히 은퇴다(과잉 앵커는 07-26 유령 집계를 되살린다)."""
        for marker in ("<!-- ★ STALE 무효화 (레인 종결) -->",
                       "<!-- javis:todo-retired -->",
                       "<!-- RETIRED 2026-07-11 -->",
                       # 실측 정본 — 여러 줄 주석의 **개시 줄**(같은 줄에 `-->`가 없다)
                       "<!-- ★★★★ STALE 무효화 (2026-07-11 teardown 삽입) ★★★★"):
            with self.subTest(marker=marker):
                rep = self._report("%s\n# WORKER TODO\n- [x] 완료\n" % marker)
                self.assertEqual(self.exc_by_node(rep).get("worker"), "retired", marker)
                self.assertTrue(RP.is_retired(self.p), marker)

    def test_b_heuristic_retired_loses_the_invariant_exemption(self):
        """(b) 휴리스틱 은퇴는 면제가 아니다 — 오분류가 나도 불변식이 잡아낸다.

        파서가 없는 스큐 구간에서 산문 자해가 다시 나더라도, 미완 작업은 이제
        `pending_outside_nodes`에 남아 park 오발동을 막는다(마지막 방어선 복원).
        """
        d = self.proj("w13a2")
        p = self.write(os.path.join(d, "WORKER_TODO.md"),
                       "<!-- javis:todo v2 owner=worker scope=pack status=retired -->\n"
                       "- [ ] 미완1\n- [ ] 미완2\n")
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        self.assertTrue(RP.is_retired(p))
        self.assertEqual([(x["node"], x["source"]) for x in rep["pending_outside_nodes"]],
                         [("worker", "heuristic")])
        self.assertFalse(RG.quiet_branch_holds(rep))


class TestW13CriticalBSingleBudget(TempPackCase):
    """【치명 B】 파서 예산(1 KiB)과 휴리스틱 예산(8 KiB)의 이원화 = 2소비자 불일치.

    `is_retired`는 머리말 **정의**만 파서에서 빌리고 **예산**은 빌리지 않았다. 그래서 1 KiB 밖·
    8 KiB 안의 은퇴 선언을 팩(휴리스틱)만 보고 Rust 데몬은 못 봤다 — 같은 파일을 두고 팩은
    "은퇴", 데몬은 "집계 중"이라 말했다.

    master 심판: `RETIRE_SCAN_BYTES`를 **폐기하고 `HEAD_BYTES`로 수렴**한다.
    """

    def setUp(self):
        super().setUp()
        self.d = self.proj("w13b")
        filler = "".join("설명 줄 %02d — 이 파일이 왜 은퇴하는지 길게 적는다.\n" % i
                         for i in range(60))
        self.body = ("# WORKER TODO\n\n" + filler +
                     "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n\n"
                     "- [ ] 미완1\n- [ ] 미완2\n")
        self.p = self.write(os.path.join(self.d, "WORKER_TODO.md"), self.body)

    def test_second_budget_constant_is_gone(self):
        """두 번째 기준을 남기지 않는다 — 상수 자체가 사라져야 재도입이 눈에 띈다."""
        self.assertFalse(hasattr(RP, "RETIRE_SCAN_BYTES"),
                         "예산 상수가 되살아났다(두 기준 = 다음 drift)")
        import javis_todo_decl as decl
        self.assertEqual(RP.HEAD_BYTES, decl.HEAD_BYTES, "예산이 파서와 갈렸다")

    def test_out_of_budget_declaration_is_invisible_to_both_consumers(self):
        """예산 밖 선언은 **양쪽 모두에게** 보이지 않는다(불일치 해소가 계약이다)."""
        import javis_todo_decl as decl
        self.assertGreater(self.body.encode("utf-8").index(b"<!-- javis:todo"),
                           decl.HEAD_BYTES, "재현 전제(예산 밖 배치)가 깨졌다")
        self.assertFalse(RP.is_retired(self.p))                       # 소비자 A(팩)
        self.assertEqual(                                             # 소비자 B(파서=Rust 데몬)
            decl.classify(decl.parse(decl.read_head(self.p))[0], "pack", lambda s: True),
            "unclaimed")
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(self.d)])
        self.assertIn("worker", self.labels(rep))     # 조용히 사라지지 않는다(fail-open)

    def test_in_budget_declaration_is_seen_by_both(self):
        """대조군 — 예산 안이면 양쪽 다 은퇴로 본다(수렴이 '전부 못 본다'가 아님을 증명)."""
        import javis_todo_decl as decl
        p = self.write(os.path.join(self.d, "CSO_TODO.md"),
                       "<!-- javis:todo v1 owner=cso scope=pack status=retired -->\n"
                       "- [ ] 미완1\n")
        self.assertEqual(
            decl.classify(decl.parse(decl.read_head(p))[0], "pack", lambda s: True), "retired")
        rep = RP.build_report(status_with(["worker", "cso"]), [os.path.dirname(self.d)])
        self.assertEqual(self.exc_by_node(rep).get("cso"), "retired")


class TestW13MajorCOwnerRoleValidation(TempPackCase):
    """【중대 C】 `owner` 라벨이 role 실재를 검증하지 않아 게이트 조인이 다시 깨진다.

    선언 owner가 라벨공간을 벗어나면 배정된 노드가 **'무배정'으로 오분류**돼, 매 주기 WARN
    (`idle_5min:<role>`) 대신 엣지 1회 + 쿨다운(`idle_edge:<role>`)으로 강등된다 = **2시간 침묵**.
    ★발현원이 우리 문서다 — 설계 §4-1 정본 예시가 `owner=worker-2`였다(이번에 문서도 교정).

    master 심판: role에 없으면 owner를 채택하지 말고 `node_label` 폴백 + 진단 노출.
    status 미수집이면 검증 불가이므로 owner 채택 유지(보수적).
    """

    IDLE = {"surfaces": [{"role": "worker", "cwd": None, "idle_secs": 999,
                          "agent_alive": True, "status": {}}],
            "feed": {"pending": 0}, "paused": False}

    def setUp(self):
        super().setUp()
        self.d = self.proj("w13c")
        self.write(os.path.join(self.d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker-2 scope=pack status=active -->\n"
                   "- [ ] 미완1\n- [ ] 미완2\n")
        self.rep = RP.build_report(self.IDLE, [os.path.dirname(self.d)])

    def test_label_falls_back_when_owner_is_not_a_live_role(self):
        self.assertEqual([n["node"] for n in self.rep["nodes"]], ["worker"])

    def test_gate_emits_per_cycle_warn_not_edge_once(self):
        """결함의 본질 — 강등되면 쿨다운(기본 2h)에 갇혀 침묵한다."""
        warns = RG.extract_warnings(self.rep, {"idle_edge": {}}, 1_700_000_000, 7200)
        self.assertEqual([w["reason"] for w in warns], ["idle_5min:worker"])

    def test_diagnosis_is_exposed_not_silently_swallowed(self):
        """조용한 폴백은 "조인이 왜 안 붙는지"를 다시 감춘다 — 그게 D3의 정체였다."""
        self.assertEqual(self.rep["nodes"][0]["owner_unresolved"], "worker-2")
        self.assertIn("선언 owner가 현재 role에 없음", RP.render_text(self.rep))

    def test_matching_owner_still_wins_over_filename(self):
        """무회귀 — role에 실재하는 owner는 종전대로 파일명을 이긴다(치명 3 교정 보존)."""
        d = self.proj("w13c2")
        self.write(os.path.join(d, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=reviewer-gemini scope=pack status=active -->\n"
                   + boxes(0, 2))
        rep = RP.build_report(status_with(["reviewer-gemini"]), [os.path.dirname(d)])
        self.assertEqual([n["node"] for n in rep["nodes"]], ["reviewer-gemini"])
        self.assertNotIn("owner_unresolved", rep["nodes"][0])

    def test_without_status_owner_is_kept_conservatively(self):
        """status 미수집 = "role이 없다"가 아니라 "알 수 없다" — 선언을 추론으로 덮지 않는다."""
        rep = RP.build_report(None, [os.path.dirname(self.d)])
        self.assertEqual([n["node"] for n in rep["nodes"]], ["worker-2"])


class TestW13MajorDUnclosedFenceRecovery(TempPackCase):
    """【중대 D】 미닫힘 펜스가 **정당한 선언을 삼킨다**(마스킹 과잉).

    M1에서는 휴리스틱 폴백이 덮어 무해해 보이지만 M3에서 폴백이 삭제되면 집계에서 사라지고,
    지금도 `decl_stats.unclaimed_ratio`(M3 전환 판단 근거)를 왜곡한다. 반대 방향도 위험하다 —
    같은 형태로 `status=retired`를 삼키면 **은퇴시켰다고 믿은 파일이 계속 집계된다**.

    master 심판: 머리말이 끝날 때까지 닫히지 않은 펜스는 **없었던 것으로 재판정**(양 언어 동일).
    """

    def _decl_verdict(self, path):
        import javis_todo_decl as decl
        return decl.classify(decl.parse(decl.read_head(path))[0], "pack", lambda s: True)

    def test_active_declaration_below_inline_fence_is_counted(self):
        d = self.proj("w13d")
        p = self.write(os.path.join(d, "WORKER_TODO.md"),
                       "# WORKER TODO\n\n```\n"
                       "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n\n"
                       "- [ ] 미완1\n- [ ] 미완2\n")
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        self.assertEqual(self._decl_verdict(p), "counted")
        self.assertIn("worker", self.labels(rep))
        # ★M3 전환 판단 근거가 왜곡되지 않는가 — 이것이 이 결함의 조용한 2차 피해였다.
        self.assertEqual(rep["decl_stats"], {"total": 1, "declared": 1, "unclaimed_ratio": 0.0})

    def test_retired_declaration_below_inline_fence_is_retired(self):
        """반대 방향 — 은퇴 선언도 대칭으로 회수된다(믿고 은퇴시킨 파일이 계속 세지면 안 된다)."""
        d = self.proj("w13d2")
        p = self.write(os.path.join(d, "CSO_TODO.md"),
                       "# CSO TODO\n\n```\n"
                       "<!-- javis:todo v1 owner=cso scope=pack status=retired -->\n\n"
                       "- [ ] 미완1\n- [ ] 미완2\n")
        rep = RP.build_report(status_with(["cso"]), [os.path.dirname(d)])
        self.assertEqual(self._decl_verdict(p), "retired")
        self.assertEqual(self.exc_by_node(rep), {"cso": "retired"})

    def test_closed_fence_example_is_still_masked(self):
        """무회귀 — G12 ①의 본령(닫힌 펜스 안 예시 선언 보호)은 그대로다(W9·W12 교정 보존)."""
        d = self.proj("w13d3")
        p = self.write(os.path.join(d, "WORKER_TODO.md"),
                       "# WORKER TODO — 선언 블록 도입 작업\n\n```markdown\n"
                       "<!-- javis:todo v1 owner=worker scope=pack status=retired -->\n"
                       "```\n\n- [ ] 미완1\n- [ ] 미완2\n")
        rep = RP.build_report(status_with(["worker"]), [os.path.dirname(d)])
        self.assertEqual(self._decl_verdict(p), "unclaimed")
        self.assertFalse(RP.is_retired(p))
        self.assertEqual([(n["node"], n["done"], n["total"]) for n in rep["nodes"]],
                         [("worker", 0, 2)])


class TestW13MinorEProgressDoesNotHidePending(TempPackCase):
    """【경미 E】 진행%가 거짓말한다 — `nodes=[('worker','1/1')]`이면 종합 100%인데 미완 2건.

    master 심판: 종합 진행률이 미완을 숨기지 않게 하되 방식은 구현자 재량. **택한 계약**은
    ①모수는 `nodes[]` 불변(유령 재유입 금지) ②`overall_complete`·`hidden_pending` 신설
    ③텍스트는 완료를 주장하지 않는다 — 백분율에 "집계 기준" 한정어와 반증을 함께 낸다.
    (기각한 대안: 모수에 `pending_outside_nodes` 편입 → 유령 79/117이 진행률로 되돌아온다.)
    """

    IDLE = {"surfaces": [{"role": "worker", "cwd": None, "idle_secs": 999,
                          "agent_alive": True, "status": {}}],
            "feed": {"pending": 0}, "paused": False}

    def setUp(self):
        super().setUp()
        d = self.proj("w13e")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(1, 0))
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(0, 2))
        self.rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

    def test_nodes_shape_is_unchanged(self):
        """게이트 계약 — `nodes[]` 구조는 손대지 않는다(master 금지 조항)."""
        self.assertEqual([(n["node"], n["done"], n["total"]) for n in self.rep["nodes"]],
                         [("worker", 1, 1)])
        self.assertEqual((self.rep["overall_done"], self.rep["overall_total"]), (1, 1))

    def test_completion_is_not_claimed_while_pending_hides_outside(self):
        self.assertFalse(self.rep["overall_complete"])
        # ★W15 교정 3 — 합계는 종전 계약 그대로 유지하고, 갈래별 숫자를 **덧붙인다**.
        self.assertEqual(self.rep["hidden_pending"], {
            "files": 1, "open": 2,
            "unresolved": {"files": 1, "open": 2},     # shadowed = 주인 불명 → park 차단
            "stale_ghost": {"files": 0, "open": 0},
        })

    def test_text_output_states_the_contract(self):
        line = [ln for ln in RP.render_text(self.rep).splitlines() if "전체 진행" in ln]
        self.assertEqual(len(line), 1, RP.render_text(self.rep))
        self.assertIn("집계 기준", line[0])          # 백분율의 모수를 명시
        self.assertIn("완료 아님", line[0])          # 완료 주장 철회
        self.assertIn("집계 밖 미완 2건", line[0])   # 반증을 같은 줄에

    def test_aggregate_is_not_contaminated_by_ghosts(self):
        """기각한 대안의 반증 — 유령의 체크박스가 진행률 모수로 되돌아오지 않는다.

        모수 편입안을 택했다면 밀려난 파일의 0/2가 합산돼 `overall_total`이 3이 됐을 것이다.
        그 방향은 `test_t9_aggregate_uncontaminated`(유령 4파일 차단)와 정면으로 충돌한다.
        """
        self.assertEqual((self.rep["overall_done"], self.rep["overall_total"]), (1, 1))
        self.assertEqual(self.rep["overall_pct"], 100)

    def test_genuine_completion_still_claims_complete(self):
        """정상 완료 상태에서는 완료를 주장한다 — 계약이 주차를 죽이면 안 된다."""
        d = self.proj("w13e2")
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v1 owner=cso scope=pack status=active -->\n" + boxes(3, 0))
        rep = RP.build_report(
            {"surfaces": [{"role": "cso", "cwd": None, "idle_secs": 999,
                           "agent_alive": True, "status": {}}],
             "feed": {"pending": 0}, "paused": False}, [os.path.dirname(d)])
        self.assertTrue(rep["overall_complete"])
        self.assertEqual(rep["hidden_pending"], {
            "files": 0, "open": 0,
            "unresolved": {"files": 0, "open": 0},
            "stale_ghost": {"files": 0, "open": 0},
        })
        text = [ln for ln in RP.render_text(rep).splitlines() if "전체 진행" in ln][0]
        self.assertNotIn("완료 아님", text)
        self.assertTrue(RG.quiet_branch_holds(rep))


class TestW15Critical3GhostDoesNotBlockParkForever(TempPackCase):
    """【W15 중대 ③】 유령의 미완이 park를 **영구** 차단한다 — 두 갈래로 분리(master 심판).

    reviewer1 실측: `pending_outside_nodes`가 `orphan`·`stale`을 비면제로 포함해, 07-26
    형태의 유령이 존재하는 동안 `overall_complete`가 영원히 False이고 `quiet_branch_holds`가
    영원히 False였다. **유령을 성공적으로 배제한 바로 그 사실이 세션 주차를 영구 차단**한
    것이고, 유령은 정의상 오래된 파일이라 시간이 풀어주지도 않는다.

    심판:
      · `stale_ghost`(휴리스틱 `orphan` · ★W18에서 `stale` 제외) → park를 막지 않는다.
        보고에는 계속 노출.
      · `unresolved`(주인 불명 `unclaimed`·`orphan-scope`·`shadowed` · ★W18에서 `stale` 편입)
        → park 차단 유지.
      · 동반 — 운영자가 유령을 처분할 경로(`javis_todo_stamp.py --promote-retire`)를 연다.

    ★W18 교정 1(master 심판 2026-07-26 · reviewer1 자기반박 채택) — 면제는 `orphan`에만 준다.
    `orphan`과 `stale`을 가르는 것은 **담당 role의 생사**뿐인데, `stale`은 담당 role이 **살아
    있는** 파일이다. 아래 test_a/test_g/test_h가 세 조합을 양방향으로 핀한다.
    """

    IDLE = {"surfaces": [{"role": r, "cwd": None, "idle_secs": 999,
                          "agent_alive": True, "status": {}} for r in ("worker", "cso")],
            "feed": {"pending": 0}, "paused": False}

    def test_a_orphan_ghost_alone_does_not_block_park(self):
        """★조합 ② — 07-26 유령 형태(종결 레인 · **담당 role 부재**)만 남으면 주차 가능하다.

        07-26 사고의 형태 그대로다: 8일 지난 비정본 위치 파일이 79/117을 들고 있고, 그 라벨
        (`reviewer-gemini`)은 현재 편대에 없다. W15가 푼 livelock이 W18 교정 1로 **회귀하지
        않았음**을 핀한다 — 실제 유령 4파일은 전부 이 형태였다.
        """
        d = self.proj("w15a")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(2, 0))
        self.write(os.path.join(d, "REVIEWER_GEMINI_TODO.md"), boxes(79, 38), age_days=8)
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        po = rep["pending_outside_nodes"]
        self.assertEqual([(r["bucket"], r["kind"]) for r in po], [("orphan", "stale_ghost")])
        self.assertEqual(rep["hidden_pending"]["unresolved"], {"files": 0, "open": 0})
        self.assertEqual(rep["hidden_pending"]["stale_ghost"], {"files": 1, "open": 38})
        self.assertEqual(RG.unresolved_pending_nodes(rep), [])
        self.assertTrue(RG.quiet_branch_holds(rep))          # ★핵심 — 주차 가능
        # 보고에서 사라지지는 않는다(park 조건에서만 뺐다).
        self.assertIn("고아 추론", RP.render_text(rep))

    def test_b_orphan_label_ghost_is_also_exempt(self):
        """라벨이 죽은 role이면 미완 규모와 무관하게 같은 갈래다(작은 유령도 면제)."""
        d = self.proj("w15b")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(2, 0))
        self.write(os.path.join(d, "REVIEWER_GEMINI_TODO.md"), boxes(1, 3), age_days=9)
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        po = rep["pending_outside_nodes"]
        self.assertEqual([(r["bucket"], r["kind"]) for r in po], [("orphan", "stale_ghost")])
        self.assertTrue(RG.quiet_branch_holds(rep))

    def test_g_live_role_stale_blocks_park(self):
        """★W18 교정 1 · 조합 ① — **담당 role이 살아 있는** 9일 미수정 미완은 주차를 막는다.

        reviewer1 재현 그대로다: 살아있는 role `cso`의 9일 미수정 todo(미완 2건)가 W15에서는
        `stale_ghost`로 분류돼 `quiet_branch_holds=True`(주차 허용)였다.

        면제 근거는 "우리 추론이지만 담당자가 이미 없다"였는데, `stale`은 담당자가 **있는**
        상태다. 살아있는 담당자의 미완을 mtime 추론만으로 침묵시키는 것은 ADR-3("놓치는 것보다
        시끄러운 편이 안전")과 A3 교훈에 정면으로 어긋난다. M1 구간에는 todo가 전부 미선언이라
        현존 파일 전량이 이 휴리스틱의 사정권에 있고, 며칠 단위 조사 레인에서 todo를 임계일
        동안 안 건드리는 일은 이 조직에서 실제로 일어난다.
        """
        d = self.proj("w18g")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(2, 0))
        self.write(os.path.join(d, "CSO_TODO.md"), boxes(3, 2), age_days=9)   # cso = 생존 role
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        po = rep["pending_outside_nodes"]
        self.assertEqual([(r["bucket"], r["kind"]) for r in po], [("stale", "unresolved")])
        self.assertEqual(rep["hidden_pending"]["unresolved"], {"files": 1, "open": 2})
        self.assertEqual(rep["hidden_pending"]["stale_ghost"], {"files": 0, "open": 0})
        self.assertEqual(len(RG.unresolved_pending_nodes(rep)), 1)
        self.assertFalse(RG.quiet_branch_holds(rep))         # ★핵심 — 주차 금지
        self.assertIn("주인 불명", RP.render_text(rep))

    def test_h_orphan_and_stale_together_block_park(self):
        """★조합 ③ — 면제 대상(`orphan`)이 함께 있어도 `stale` 하나가 주차를 막는다.

        면제가 "목록에 면제 항목이 있으면 통과"로 느슨해지면 이 핀이 깨진다. 판정은 항목별이다.
        """
        d = self.proj("w18h")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(2, 0))
        self.write(os.path.join(d, "REVIEWER_GEMINI_TODO.md"), boxes(79, 38), age_days=8)
        self.write(os.path.join(d, "CSO_TODO.md"), boxes(3, 2), age_days=9)
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])

        self.assertEqual({(r["bucket"], r["kind"]) for r in rep["pending_outside_nodes"]},
                         {("orphan", "stale_ghost"), ("stale", "unresolved")})
        self.assertEqual(rep["hidden_pending"]["unresolved"], {"files": 1, "open": 2})
        self.assertEqual(rep["hidden_pending"]["stale_ghost"], {"files": 1, "open": 38})
        self.assertFalse(RG.quiet_branch_holds(rep))
        # 사람 대면 렌더도 두 갈래를 **따로** 보여준다(왜 안 조용해지는지 + 왜 조용해져도 되는지).
        txt = RP.render_text(rep)
        self.assertIn("주인 불명", txt)
        self.assertIn("고아 추론", txt)

    def test_i_status_unavailable_gives_no_park_exemption(self):
        """★status 미수집이면 어떤 파일도 면제받지 못한다 — 편대를 모르는 채 주차하지 않는다.

        `roles is None`이면 `classify_files`의 분기가 전부 `stale`로 떨어진다. W15에서는 그
        전부가 면제였다(= 편대 현황을 모르는 상태에서 유령 추정만으로 주차 허용). W18에서는
        전부 `unresolved`가 되어 보수적 방향으로 닫힌다.
        """
        d = self.proj("w18i")
        self.write(os.path.join(d, "REVIEWER_GEMINI_TODO.md"), boxes(1, 3), age_days=9)
        rep = RP.build_report(None, [os.path.dirname(d)])     # status 미수집
        self.assertEqual([(r["bucket"], r["kind"]) for r in rep["pending_outside_nodes"]],
                         [("stale", "unresolved")])
        self.assertFalse(RG.quiet_branch_holds(rep))

    def test_c_undeclared_pending_still_blocks_park(self):
        """반대 방향 — **미선언 미완**(주인 불명)은 여전히 주차를 막는다.

        이것이 분리의 경계다. 유령은 우리가 근거를 갖고 배제한 것이고, 미선언은 우리가
        마지막 관측자인 상태다. 이 핀이 무너지면 W13 치명 2(false QUIET)가 되살아난다.
        """
        d = self.proj("w15c")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(1, 0))
        self.write(os.path.join(d, "WORKER_TODO.md"), boxes(0, 2))   # 신선 = shadowed
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        po = rep["pending_outside_nodes"]
        self.assertEqual([(r["bucket"], r["kind"]) for r in po],
                         [("shadowed", "unresolved")])
        self.assertFalse(RG.quiet_branch_holds(rep))
        self.assertIn("주인 불명", RP.render_text(rep))

    def test_d_heuristic_retired_with_pending_is_not_a_ghost(self):
        """휴리스틱 `retired`는 `stale_ghost`가 **아니다** — W13 교정 1(b)의 근거가 유효하다.

        그 판정은 관측 사실이 아니라 문자열 매칭이고, 치명 A의 자해가 정확히 그 경로로
        들어와 마지막 방어선까지 통과했다. 오탐의 성질이 달라 같은 특권을 줄 수 없다.
        """
        d = self.proj("w15d")
        self.write(os.path.join(self.pack_round, "WORKER_TODO.md"),
                   "<!-- javis:todo v1 owner=worker scope=pack status=active -->\n" + boxes(2, 0))
        # 파서가 모르는 v2 선언 = `is_retired` 휴리스틱만 걸리는 경로(W13 테스트와 같은 입력).
        self.write(os.path.join(d, "CSO_TODO.md"),
                   "<!-- javis:todo v2 owner=cso scope=pack status=retired -->\n" + boxes(0, 2))
        rep = RP.build_report(self.IDLE, [os.path.dirname(d)])
        po = rep["pending_outside_nodes"]
        self.assertEqual([(r["bucket"], r["source"], r["kind"]) for r in po],
                         [("retired", "heuristic", "unresolved")])
        self.assertFalse(RG.quiet_branch_holds(rep))

    def test_e_gate_treats_missing_kind_as_unresolved(self):
        """ADR-2 스큐 — 구버전 보고기(`kind` 없음)에서는 **종전과 똑같이** park를 막는다."""
        legacy = {"nodes": [], "pending_outside_nodes": [{"node": "w", "path": "/x",
                                                          "done": 0, "total": 2,
                                                          "bucket": "stale"}],
                  "live_nodes": [{"role": "w", "idle_secs": 999, "agent_alive": True}]}
        self.assertEqual(len(RG.unresolved_pending_nodes(legacy)), 1)
        self.assertFalse(RG.quiet_branch_holds(legacy))

    def test_f_pending_kind_is_one_rule(self):
        """갈래 판정은 산출기의 단일 함수다 — 게이트가 두 번째 기준을 만들지 않는다."""
        self.assertEqual(RP.pending_kind("orphan", "heuristic"), "stale_ghost")
        # ★W18 교정 1 — `stale`(담당 role 생존)은 면제가 아니다. 면제는 `orphan` 하나뿐이다.
        self.assertEqual(RP.pending_kind("stale", "heuristic"), "unresolved")
        self.assertEqual(RP.PENDING_STALE_GHOST_BUCKETS, ("orphan",))
        self.assertEqual(RP.pending_kind("shadowed", "heuristic"), "unresolved")
        self.assertEqual(RP.pending_kind("retired", "heuristic"), "unresolved")
        self.assertEqual(RP.pending_kind("unclaimed", None), "unresolved")
        self.assertEqual(RP.pending_kind("orphan-scope", "decl"), "unresolved")
        # 같은 이름의 버킷이 **선언 경로**에서 생기면 면제가 아니다(주인이 말한 것이므로).
        self.assertEqual(RP.pending_kind("orphan", "decl"), "unresolved")
        self.assertEqual(RG.PENDING_KIND_STALE_GHOST, RP.PENDING_KIND_STALE_GHOST)


if __name__ == "__main__":
    unittest.main()
