"""H-10 (0.14.55-P8) — 릴리스 게이트의 모집단 판정자와 그 결합 가드를 **함께** 잰다.

왜 함께인가: 게이트는 태그 레인을 **이름**으로 가른다(`event==push ∧ head_branch~v*`). 그 판정이
정확한 것은 `windows-health.yml` 의 브랜치 트리거가 `v` 로 시작하는 이름을 만들 수 없기 때문이고,
그 전제가 깨지면 게이트는 **조용히** 틀린다. 판정자와 가드를 한 파일에서 등록해 둘의 드리프트가
검사 없이 지나가지 못하게 한다(설계 R2 §5-3 · R3 §3-5).

판정자는 `release.yml` 의 게이트 스텝 안에 히어독으로 산다(그 잡에는 checkout 이 없다). 여기서는
그 블록을 **워크플로 파일에서 그대로 떼어** 돌린다 — 저장소에 사본을 두지 않으므로 둘이 갈릴 수 없다.
PyYAML 없이 돈다(CI 의 파이썬에 없다).

픽스처 6개(설계 §5-9):
  F1 접미 태그(`v0.14.55-rc.1` · `+build.1` · `v1`)가 전부 태그 레인으로 제외된다
  F2 미완 런이 섞이면 통과하지 않는다(대기)
  F3 태그 레인만 있으면 fail-closed
  F4 원시 결과가 페이지 상한이면 **필터 전에** fail-closed
  F5 브랜치 성공 + 태그 실패 ⇒ 브랜치 레인만으로 판정해 통과
  F6 결합 가드: 핀에서 PASS · 충돌/미정 가설에서 FAIL/REFUSED

    python3 scripts/tests/test_release_gate_pop.py
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
RELEASE_YML = os.environ.get("H10_RELEASE_YML_UNDER_TEST") or os.path.join(
    ROOT, ".github", "workflows", "release.yml")
GUARD = os.environ.get("H10_GUARD_UNDER_TEST") or os.path.join(ROOT, "scripts", "coupling_guard.py")
WINDOWS_HEALTH = os.path.join(ROOT, ".github", "workflows", "windows-health.yml")
HEREDOC_START = 'cat > "$GATE_PY" <<\'PY\''
SHA = "a" * 40
PASS, FAIL, WAIT = 0, 1, 2
REFUSED = 3


def extract_gate_py(path):
    with open(path, encoding="utf-8") as fh:
        lines = fh.read().replace("\r\n", "\n").split("\n")
    start = None
    for i, ln in enumerate(lines):
        if ln.strip() == HEREDOC_START:
            start, indent = i, len(ln) - len(ln.lstrip())
            break
    if start is None:
        raise AssertionError("게이트 판정자 히어독을 찾지 못했다: %s" % HEREDOC_START)
    body = []
    for ln in lines[start + 1:]:
        if ln.strip() == "PY":
            return "\n".join(body) + "\n"
        body.append(ln[indent:] if ln.startswith(" " * indent) else ln.lstrip())
    raise AssertionError("히어독의 끝(PY)을 찾지 못했다")


def run_gate(src, runs, event="push", sha=SHA):
    with tempfile.TemporaryDirectory() as d:
        gp = os.path.join(d, "gate_pop.py")
        rj = os.path.join(d, "runs.json")
        with open(gp, "w", encoding="utf-8") as fh:
            fh.write(src)
        with open(rj, "w", encoding="utf-8") as fh:
            json.dump(runs, fh)
        r = subprocess.run([sys.executable, "-I", "-B", gp, rj, sha, event],
                           capture_output=True, text=True, cwd=d)
        return r.returncode, r.stdout + r.stderr


def run(idx, status="completed", conclusion="success", event="push",
        head_branch="fix/0.14.55-x", sha=SHA, attempt=1):
    return {"id": 1000 + idx, "status": status, "conclusion": conclusion,
            "url": "https://example.invalid/run/%d" % (1000 + idx), "event": event,
            "head_branch": head_branch, "head_sha": sha, "run_attempt": attempt}


class H10GatePopulation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src = extract_gate_py(RELEASE_YML)

    def test_extracted_predicate_is_the_workflow_one(self):
        for token in ("def is_tag_lane(", "RAW_CAP = 30", "head_sha", "run_attempt"):
            self.assertIn(token, self.src, "떼어 낸 본문이 게이트 판정자가 아니다")

    # ── F1 ────────────────────────────────────────────────────────────────
    def test_f1_suffix_tags_are_all_classified_tag_lane(self):
        tags = ["v0.14.54", "v0.14.55-rc.1", "v0.14.55+build.1", "v1"]
        runs = [run(i, head_branch=t) for i, t in enumerate(tags)]
        runs.append(run(9, head_branch="fix/0.14.55-x"))      # 브랜치 레인 1건(성공)
        rc, out = run_gate(self.src, runs)
        self.assertEqual(rc, PASS, out)
        self.assertIn("태그 레인 %d 런은 모집단에서 제외" % len(tags), out)
        self.assertIn("브랜치 레인 1 런", out)

    def test_f1b_a_workflow_dispatch_named_like_a_tag_is_not_a_tag_lane(self):
        # 트리거가 push 일 때만 태그 레인이다 — `vnext` 를 workflow_dispatch 로 돌린 경우.
        rc, out = run_gate(self.src, [run(0, event="workflow_dispatch", head_branch="vnext")])
        self.assertEqual(rc, PASS, out)
        self.assertIn("브랜치 레인 1 런", out)

    # ── F2 ────────────────────────────────────────────────────────────────
    def test_f2_pending_run_does_not_pass(self):
        runs = [run(0), run(1, status="in_progress", conclusion=None)]
        rc, out = run_gate(self.src, runs)
        self.assertEqual(rc, WAIT, out)
        self.assertIn("미완 1", out)

    def test_f2b_zero_completed_waits_rather_than_passing(self):
        rc, out = run_gate(self.src, [run(0, status="queued", conclusion=None)])
        self.assertEqual(rc, WAIT, out)

    # ── F3 ────────────────────────────────────────────────────────────────
    def test_f3_only_tag_lane_runs_is_fail_closed(self):
        rc, out = run_gate(self.src, [run(0, head_branch="v0.14.55")])
        self.assertEqual(rc, FAIL, out)
        self.assertIn("전부 태그 레인", out)

    def test_f3b_no_runs_at_all_keeps_the_pinned_two_arms(self):
        rc, out = run_gate(self.src, [], event="push")
        self.assertEqual(rc, WAIT, out)                     # push: 아직 안 생겼을 수 있다
        rc, out = run_gate(self.src, [], event="workflow_dispatch")
        self.assertEqual(rc, FAIL, out)                     # 수동 경로: 종전 그대로 즉시 닫는다
        self.assertIn("gh workflow run windows-health.yml", out)

    # ── F4 ────────────────────────────────────────────────────────────────
    def test_f4_page_saturation_fails_before_filtering(self):
        # 30건 전부 태그 레인 ⇒ 필터를 **먼저** 했다면 POP 이 0이 되어 다른 메시지가 났을 것이다.
        runs = [run(i, head_branch="v0.14.55") for i in range(30)]
        rc, out = run_gate(self.src, runs)
        self.assertEqual(rc, FAIL, out)
        self.assertIn("페이지 상한", out)
        self.assertNotIn("전부 태그 레인", out)
        # 29건이면 상한에 닿지 않는다(경계).
        rc29, _ = run_gate(self.src, [run(i, head_branch="v0.14.55") for i in range(29)])
        self.assertEqual(rc29, FAIL)                        # 태그 레인뿐이라 여전히 닫히지만
        self.assertIn("전부 태그 레인", run_gate(
            self.src, [run(i, head_branch="v0.14.55") for i in range(29)])[1])

    # ── F5 ────────────────────────────────────────────────────────────────
    def test_f5_branch_success_beside_tag_failure_passes_on_the_branch_lane(self):
        runs = [run(0, head_branch="fix/0.14.55-x", conclusion="success"),
                run(1, head_branch="v0.14.55", conclusion="failure")]
        rc, out = run_gate(self.src, runs)
        self.assertEqual(rc, PASS, out)

    def test_f5b_a_failed_branch_lane_run_closes_the_gate(self):
        runs = [run(0, conclusion="success"), run(1, conclusion="failure")]
        rc, out = run_gate(self.src, runs)
        self.assertEqual(rc, FAIL, out)
        self.assertIn("conclusion=failure", out)

    def test_f5c_runs_at_another_sha_are_not_in_the_population(self):
        rc, out = run_gate(self.src, [run(0, sha="b" * 40)], event="push")
        self.assertEqual(rc, WAIT, out)

    # ── G-5 ───────────────────────────────────────────────────────────────
    def test_g5_success_log_names_attempts_and_what_it_does_not_testify_to(self):
        rc, out = run_gate(self.src, [run(0, attempt=3)])
        self.assertEqual(rc, PASS, out)
        self.assertIn("attempt=3", out)
        self.assertIn("continue-on-error", out)

    # ── F6 ────────────────────────────────────────────────────────────────
    def test_f6_coupling_guard_passes_at_the_pin(self):
        r = subprocess.run([sys.executable, "-I", "-B", GUARD, WINDOWS_HEALTH],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("PASS", r.stdout)

    def test_f6b_coupling_guard_asserts_all_three_verdicts_on_every_run(self):
        r = subprocess.run([sys.executable, "-I", "-B", GUARD, WINDOWS_HEALTH],
                           capture_output=True, text=True)
        self.assertIn("self-test", r.stdout)
        for want in ("기대=0", "기대=1", "기대=3"):
            self.assertIn(want, r.stdout, "가드의 자기검사가 세 갈래를 모두 단언하지 않는다")

    def test_f6c_coupling_guard_fails_and_refuses_on_hypotheticals(self):
        cases = [("    branches: ['feat/**', 'v*']\n    tags: ['v*']\n", 1),
                 ('    branches: ["feat/**", "v*"]\n    tags: ["v*"]\n', 1),
                 ("    branches:\n      - feat/**\n      - v*\n    tags:\n      - v*\n", 1),
                 ("    branches: ['feat/**', 'v[ab]z']\n    tags: ['v*']\n", 3),
                 ("    branches: []\n    tags: ['v*']\n", 3)]
        with tempfile.TemporaryDirectory() as d:
            for n, (body, want) in enumerate(cases):
                p = os.path.join(d, "wf%d.yml" % n)
                with open(p, "w", encoding="utf-8") as fh:
                    fh.write("name: t\non:\n  push:\n" + body +
                             "jobs:\n  x:\n    runs-on: ubuntu-latest\n")
                r = subprocess.run([sys.executable, "-I", "-B", GUARD, p],
                                   capture_output=True, text=True)
                self.assertEqual(r.returncode, want, "%s\n%s" % (body, r.stdout + r.stderr))

    # ── 메타: 투영이 필요한 필드를 지키는가(G-0) ─────────────────────────────
    def test_g0_projection_keeps_the_fields_the_predicate_needs(self):
        with open(RELEASE_YML, encoding="utf-8") as fh:
            text = fh.read()
        jq_lines = [l for l in text.split("\n") if ".workflow_runs[]" in l]
        self.assertEqual(len(jq_lines), 1, "게이트의 투영 줄이 하나가 아니다: %r" % jq_lines)
        for field in ("event", "head_branch", "head_sha", "run_attempt",
                      "id", "status", "conclusion"):
            self.assertIn(field, jq_lines[0],
                          "투영이 %s 를 버린다 — G-1/G-5 의 전제가 깨진다" % field)


if __name__ == "__main__":
    unittest.main(verbosity=2)
