#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_round_stop_reason.py — 라운드 **종료 사유(stop_reason)** 와 정체 종결 게이트 (0.14.31 WP-6).

무엇을 막는가: 라운드 루프가 "미수렴/수렴" 두 값만 알던 시절엔, 리뷰어가 계속 ACCEPT 를 내면서
minor 만 덧붙이는 상태를 **종결로 읽을 어휘가 없었다**(실측: SURVEY R1~R7 중 R3~R7 이 문구
미세조정에 소모). 이 검체는 새 어휘가 ①정확히 5값이고 ②종결을 **증거로만** 선언하며
③BLOCK·FAIL·SKIP·major 를 종결로 삼키지 않고 ④종결 이후 새 라운드를 exit 3 으로 막되
`--override "<사유>"` 로만 재개되며 그 재개가 **기록**되는지를 CLI 종단으로 못박는다.

★순수 판정 배터리는 `javis_orchestra.py --self-test` 에 있다(그쪽이 소스-오브-레코드).
  여기서는 **배선**을 본다 — help 토큰(지침의 '도구 선행 확인' 조항이 그 토큰으로 휴면을 푼다) ·
  stdout 문면 · exit 코드 · 거부 시 `--from-cmd` 미실행 · 감사 기록 · sha 결속 · 읽기 전용성.

밀폐: `CYS_PACK_DIR`·`JAVIS_ROOT` 를 임시 디렉터리로 덮어 라이브 팩·라이브 `_round` 무접촉.
데몬 왕복 0(round-log·round-status 는 ACK 게이트 소비자가 아니다).

    CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" \
        python3 cysjavis-pack/bin/tests/test_round_stop_reason.py
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
ORC = os.path.join(BIN, "javis_orchestra.py")
RSI = os.path.join(BIN, "javis_rsi.py")
PY = sys.executable or "python3"
TASK = "WP6 정체"


def _verdict(severities=("minor",), verdict="ACCEPT"):
    return {
        "verdict": verdict,
        "justification": "근거 충분(검체)",
        "evidence": [{"claim": "c", "ref": "f.py:1", "verified": True}],
        "issues": [{"severity": s, "where": "f.py:%d" % (i + 1), "what": "지적",
                    "fix": "교정안"} for i, s in enumerate(severities)],
    }


class RoundStopReason(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.pack = os.path.join(self.root, "pack")
        os.makedirs(os.path.join(self.pack, "round", "_reviews"))
        self.env = dict(os.environ)
        self.env["CYS_PACK_DIR"] = self.pack
        self.env["JAVIS_ROOT"] = os.path.join(self.root, "javis")
        for k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR", "CYS_ROUND_DIR"):
            self.env.pop(k, None)
        self.ledger = os.path.join(self.pack, "round", "ORCHESTRATION-WP6_정체.md")
        self.sidecar = self.ledger[:-3] + ".wp6.jsonl"

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    # ── 실행 헬퍼 ──────────────────────────────────────────────────────────
    def orc(self, *args):
        return subprocess.run([PY, ORC] + list(args), capture_output=True, text=True,
                              timeout=120, env=self.env)

    def write_verdict(self, rnd, ev, severities=("minor",), verdict="ACCEPT"):
        p = os.path.join(self.pack, "round", "_reviews", "WP6_정체-r%d-%s.json" % (rnd, ev))
        with open(p, "w", encoding="utf-8") as f:
            json.dump(_verdict(severities, verdict), f, ensure_ascii=False)
        return p

    def log_reviewer(self, rnd, ev, severities=("minor",), verdict="ACCEPT", extra=()):
        p = self.write_verdict(rnd, ev, severities, verdict)
        return self.orc("round-log", "--task", TASK, "--round", str(rnd),
                        "--evaluator", ev, "--verdict-json", p, *extra)

    def log_machine(self, rnd, cmd="exit 0", extra=()):
        return self.orc("round-log", "--task", TASK, "--round", str(rnd),
                        "--evaluator", "machine", "--from-cmd", cmd, *extra)

    def seed_two_minor_rounds(self):
        """R1·R2 = gemini·codex ACCEPT(minor only) + machine PASS → 정체 성립 상태."""
        for rnd in (1, 2):
            for ev in ("gemini", "codex"):
                r = self.log_reviewer(rnd, ev)
                self.assertEqual(r.returncode, 0, r.stderr)
            r = self.log_machine(rnd)
            self.assertEqual(r.returncode, 0, r.stderr)

    def status(self, *extra):
        return self.orc("round-status", "--task", TASK, *extra)

    def rows(self):
        with open(self.ledger, encoding="utf-8") as f:
            return [ln for ln in f if ln.lstrip().startswith("|") and "라운드 |" not in ln
                    and not set(ln.strip()) <= set("|- ")]

    def events(self):
        if not os.path.isfile(self.sidecar):
            return []
        with open(self.sidecar, encoding="utf-8") as f:
            return [json.loads(ln) for ln in f if ln.strip()]

    # ── ① 지침의 '도구 선행 확인' 토큰 (help 계약) ────────────────────────
    def test_help_contract_tokens(self):
        """MASTER/CEO 지침은 `round-status --help` 의 `stop_reason` 과 `round-log` 의
        `--override` 로 이 조항의 휴면을 푼다 — 토큰이 없으면 지침이 조용히 잠든다."""
        rs = self.orc("round-status", "--help")
        self.assertEqual(rs.returncode, 0, rs.stderr)
        self.assertIn("stop_reason", rs.stdout)
        for v in ("accepted", "stopped_budget", "stopped_stagnation",
                  "needs_investigation", "open"):
            self.assertIn(v, rs.stdout, "help 에 stop_reason 값 %r 누락" % v)
        rl = self.orc("round-log", "--help")
        self.assertEqual(rl.returncode, 0, rl.stderr)
        self.assertIn("--override", rl.stdout)

    # ── ② 정체 성립 · 장부만으로는 성립 불가 ──────────────────────────────
    def test_two_rounds_minor_only_accept_is_stagnation(self):
        self.seed_two_minor_rounds()
        r = self.status()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("stop_reason=stopped_stagnation", r.stdout)
        self.assertIn("백로그", r.stdout)          # 지침 문면과 같은 처방
        self.assertNotIn("다음 라운드 3 진행 가능", r.stdout)   # 상반된 지시 동시 출력 금지

    def test_ledger_alone_never_stagnates(self):
        """결속(verdict_src)을 지운 장부 = 구 장부 — 어떤 경우에도 종결하지 않는다(휴면)."""
        self.seed_two_minor_rounds()
        os.remove(self.sidecar)
        r = self.status()
        self.assertIn("stop_reason=open", r.stdout)
        self.assertIn("결속 없음", r.stdout)

    def test_sha_mismatch_invalidates_evidence(self):
        """기록 이후 verdict 파일이 바뀌면 그 증거는 무효다(낡은/덮인 파일로 거짓 종결 차단)."""
        self.seed_two_minor_rounds()
        p = os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json")
        with open(p, "w", encoding="utf-8") as f:
            json.dump(_verdict(("major",)), f, ensure_ascii=False)   # 덮어쓰기
        r = self.status()
        self.assertIn("stop_reason=open", r.stdout)
        self.assertIn("sha256 불일치", r.stdout)

    # ── ③ 종결로 삼키면 안 되는 것들 ──────────────────────────────────────
    def test_reviewer_block_stays_open(self):
        """reviewer1 BLOCK / reviewer2 ACCEPT → open (BLOCK 보존 · §8)."""
        self.log_reviewer(1, "gemini")
        self.log_reviewer(1, "codex")
        self.log_machine(1)
        r = self.log_reviewer(2, "gemini", ("blocking",), "BLOCK")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.log_reviewer(2, "codex")
        self.log_machine(2)
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)
        self.assertIn("| 2 | gemini | - | BLOCK |", open(self.ledger, encoding="utf-8").read())

    def test_machine_fail_stays_open(self):
        """machine FAIL / 리뷰 ACCEPT → open (기계검증 실패를 종결로 접지 않는다)."""
        for rnd in (1, 2):
            self.log_reviewer(rnd, "gemini")
            self.log_reviewer(rnd, "codex")
        self.log_machine(1)
        r = self.log_machine(2, "exit 1")
        self.assertEqual(r.returncode, 1, "machine 실패는 exit 1 로 남아야 한다")
        self.assertIn("stop_reason=open", self.status().stdout)

    def test_major_issue_stays_open(self):
        """ACCEPT 라도 major 이슈가 열려 있으면 정체가 아니라 **미완**이다."""
        for rnd in (1, 2):
            self.log_reviewer(rnd, "gemini", ("minor", "major") if rnd == 2 else ("minor",))
            self.log_reviewer(rnd, "codex")
            self.log_machine(rnd)
        self.assertIn("stop_reason=open", self.status().stdout)

    # ── ④ 정체 이후 라운드 발행 게이트 ────────────────────────────────────
    def test_new_round_refused_with_exit_3(self):
        self.seed_two_minor_rounds()
        before = len(self.rows())
        r = self.log_reviewer(3, "gemini")
        self.assertEqual(r.returncode, 3, (r.returncode, r.stdout, r.stderr))
        self.assertIn("stopped_stagnation", r.stderr)
        self.assertIn("--override", r.stderr)
        self.assertEqual(len(self.rows()), before, "거부인데 행이 기록됐다")
        self.assertTrue(any(e.get("event") == "stagnation_stop" for e in self.events()),
                        "종결이 감사 기록에 남지 않았다")

    def test_refusal_does_not_run_from_cmd(self):
        """거부는 **부수효과 전**에 일어난다 — 종결 후 빌드/테스트를 돌리고 버리지 않는다."""
        self.seed_two_minor_rounds()
        flag = os.path.join(self.root, "ran.flag")
        cmd = '%s -c "open(r\'%s\', \'w\').close()"' % (PY, flag)
        r = self.log_machine(3, cmd)
        self.assertEqual(r.returncode, 3, (r.returncode, r.stderr))
        self.assertFalse(os.path.exists(flag), "거부인데 --from-cmd 가 실행됐다(1800s 낭비·진단 유실)")

    def test_same_round_completion_allowed_but_new_round_still_sticky(self):
        """정체를 부른 라운드의 완결·재평가는 막지 않되, 재개 권한은 override 까지 유지된다."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        r = self.orc("round-log", "--task", TASK, "--round", "2",
                     "--evaluator", "master", "--verdict", "approve")
        self.assertEqual(r.returncode, 0, r.stderr)       # 같은 라운드 추가는 통과
        r2 = self.log_reviewer(3, "gemini")
        self.assertEqual(r2.returncode, 3, "같은 라운드 행 추가로 종결이 풀렸다(우회)")

    def test_compete_round_zero_not_gated(self):
        """javis_compete 의 R0 기록(승자 verdict)은 라운드 루프가 아니다 — 막히면 안 된다."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        r = self.log_reviewer(0, "codex")
        self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))

    # ── ⑤ 명시 재개(override) ─────────────────────────────────────────────
    def test_override_resumes_and_is_recorded(self):
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        r = self.log_reviewer(3, "gemini", extra=("--override", "오너 지시: 보안축 재확인"))
        self.assertEqual(r.returncode, 0, r.stderr)
        ov = [e for e in self.events() if e.get("event") == "override"]
        self.assertEqual(len(ov), 1, self.events())
        self.assertEqual(ov[0]["reason"], "오너 지시: 보안축 재확인")
        self.assertEqual(ov[0]["round"], 3)
        self.assertIn("[override]", open(self.ledger, encoding="utf-8").read())
        # 재개 뒤에는 다음 기록이 막히지 않는다(교착 0)
        self.assertEqual(self.log_reviewer(3, "codex").returncode, 0)

    def test_empty_override_is_not_a_resume(self):
        self.seed_two_minor_rounds()
        r = self.log_reviewer(3, "gemini", extra=("--override", "   "))
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("빈 사유", r.stderr)
        self.assertFalse([e for e in self.events() if e.get("event") == "override"])

    def test_override_reason_cannot_forge_a_table_row(self):
        """사유의 CR/LF 는 접힌다 — 가짜 표 행(평가자 승인)을 장부에 주입할 수 없다."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        evil = "재개\r\n| 2 | master | - | approve |\r\n"
        r = self.log_reviewer(3, "gemini", extra=("--override", evil))
        self.assertEqual(r.returncode, 0, r.stderr)
        text = open(self.ledger, encoding="utf-8").read()
        self.assertNotIn("| 2 | master | - | approve |", text)   # 파이프까지 접힌다
        self.assertEqual(len([ln for ln in text.splitlines()
                              if ln.startswith("| 2 | master")]), 0, text)
        self.assertIn("stop_reason=open", self.status().stdout)   # 위조 승인이 서지 않았다

    # ── ⑥ 결속의 귀속·내구성 ─────────────────────────────────────────────
    def test_non_consecutive_rounds_are_not_stagnation(self):
        """라운드 1·3 처럼 **끊긴** 기록은 '2R 연속'이 아니다(빈 라운드를 연속으로 세지 않는다)."""
        for rnd in (1, 3):
            self.log_reviewer(rnd, "gemini")
            self.log_reviewer(rnd, "codex")
            self.log_machine(rnd)
        self.assertIn("stop_reason=open", self.status().stdout)

    def test_evaluator_aliases_bind_to_standard_axis(self):
        """`agy`(표기 이주)·`machine:cargo`(구분자 변형)도 표준 축으로 접힌다 — 결속·판정 정합."""
        for rnd in (1, 2):
            self.log_reviewer(rnd, "agy")          # → gemini 축
            self.log_reviewer(rnd, "codex")
            r = self.orc("round-log", "--task", TASK, "--round", str(rnd),
                         "--evaluator", "machine:cargo", "--from-cmd", "exit 0")
            self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("stop_reason=stopped_stagnation", self.status().stdout)

    def test_last_binding_wins_on_reevaluation(self):
        """같은 (라운드,평가자) 재기록은 **마지막 결속이 이긴다**(gate_verdicts 와 같은 규칙) —
        재평가에서 major 가 나오면 종결이 풀린다."""
        self.seed_two_minor_rounds()
        self.assertIn("stop_reason=stopped_stagnation", self.status().stdout)
        r = self.log_reviewer(2, "codex", ("major",))
        self.assertEqual(r.returncode, 0, r.stderr)
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)
        self.assertIn("severity major", out)

    def test_corrupt_sidecar_is_fail_closed(self):
        """사이드카가 통째로 깨지면 결속이 사라진다 — 종결이 아니라 open(휴면)."""
        self.seed_two_minor_rounds()
        with open(self.sidecar, "w", encoding="utf-8") as f:
            f.write("garbage\n")
        self.assertIn("stop_reason=open", self.status().stdout)

    def test_missing_verdict_file_is_not_evidence(self):
        """결속은 있는데 파일이 사라지면 증거가 아니다(경로만 남은 종결 금지)."""
        self.seed_two_minor_rounds()
        os.remove(os.path.join(self.pack, "round", "_reviews", "WP6_정체-r1-gemini.json"))
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)
        self.assertIn("파일 부재", out)

    def test_slug_collision_does_not_inherit_foreign_stop(self):
        """슬러그가 충돌하는 다른 task 의 **종결을 물려받지 않는다** — 장부·사이드카는 공유되지만
        끈끈한 종결은 task 귀속으로 걸러진다(codex 위임 검체 발견)."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)   # 종결 기록됨
        stops = [e for e in self.events() if e.get("event") == "stagnation_stop"]
        self.assertEqual(len(stops), 1)
        self.assertEqual(stops[0]["task"], TASK)
        twin = "WP6/정체"          # 같은 슬러그(WP6_정체) → 같은 장부·사이드카
        r = subprocess.run([PY, ORC, "round-log", "--task", twin, "--round", "9",
                            "--evaluator", "master", "--verdict", "approve"],
                           capture_output=True, text=True, timeout=120, env=self.env)
        self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))

    # ── ⑧ round-status 는 읽기 전용 ───────────────────────────────────────
    def test_status_is_read_only(self):
        self.seed_two_minor_rounds()
        before = (open(self.ledger, "rb").read(), open(self.sidecar, "rb").read())
        self.status()
        self.status("--verdict-json", "2:codex:%s" %
                    os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json"))
        after = (open(self.ledger, "rb").read(), open(self.sidecar, "rb").read())
        self.assertEqual(before, after, "round-status 가 장부·사이드카를 건드렸다")


class RsiRoundBudget(unittest.TestCase):
    """RSI 라운드 예산 — `attempts` 는 재checkpoint·재시작·ledger 삭제를 넘어 영속한다."""
    maxDiff = None

    def setUp(self):
        self.root = tempfile.mkdtemp()
        subprocess.run(["git", "init", "-q", "."], cwd=self.root, check=True)
        for kv in (("user.email", "t@example.invalid"), ("user.name", "t")):
            subprocess.run(["git", "config"] + list(kv), cwd=self.root, check=True)
        open(os.path.join(self.root, "a"), "w").write("x\n")
        subprocess.run(["git", "add", "a"], cwd=self.root, check=True)
        subprocess.run(["git", "commit", "-qm", "init"], cwd=self.root, check=True)
        self.env = dict(os.environ)
        self.env["CYS_ROUND_DIR"] = os.path.join(self.root, "_round")
        self.env.pop("CYS_RSI_MAX_ROUNDS", None)
        self.env.pop("CYS_RSI_CEILING_FLATS", None)
        self.state = os.path.join(self.root, "_round", "rsi", "state.json")

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def rsi(self, *args):
        return subprocess.run([PY, RSI] + list(args), cwd=self.root, capture_output=True,
                              text=True, timeout=120, env=self.env)

    def test_recheckpoint_keeps_the_cap(self):
        seen = []
        for _ in range(4):
            r = self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
            self.assertEqual(r.returncode, 0, r.stderr)
            seen.append(json.loads(r.stdout))
        self.assertEqual([e["attempts"] for e in seen], [1, 2, 3, 4],
                         "재checkpoint 가 시도 이력을 지웠다(상한 리셋)")
        self.assertEqual([e["stop_reason"] for e in seen],
                         ["open", "open", "open", "stopped_budget"])
        self.assertIn("stopped_budget", seen[-1]["stop_reason"])

    def test_attempts_survive_state_loss(self):
        """state.json 을 지워도 append-only ledger 재계수가 상한을 되살린다."""
        for _ in range(3):
            self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
        os.remove(self.state)
        e = json.loads(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").stdout)
        self.assertEqual(e["attempts"], 4)
        self.assertEqual(e["stop_reason"], "stopped_budget")

    def test_progress_counts_as_attempt(self):
        """checkpoint 만 세면 상한이 무력하다 — progress(점수 주입)도 시도다."""
        self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
        got = [json.loads(self.rsi("progress", "--round", "r1", "--score",
                                   str(1.0 + i)).stdout) for i in (1, 2, 3)]
        self.assertEqual([g["attempts"] for g in got], [2, 3, 4])
        self.assertEqual(got[-1]["stop_reason"], "stopped_budget")
        self.assertEqual([g["verdict"] for g in got], ["improved"] * 3,
                         "verdict(주입 점수 산술)는 그대로다")

    def test_ceiling_is_stagnation_and_digest_is_one_line(self):
        """flat 연속 = stopped_stagnation · 추천은 주간 다이제스트 큐 **1건**(feed 0)."""
        self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
        self.env["CYS_RSI_MAX_ROUNDS"] = "99"           # 예산이 아니라 정체를 본다
        last = None
        for _ in range(4):
            last = json.loads(self.rsi("progress", "--round", "r1", "--score", "1.0").stdout)
        self.assertEqual(last["stop_reason"], "stopped_stagnation", last)
        q = os.path.join(self.root, "_round", "learn", "digest_queue.jsonl")
        with open(q, encoding="utf-8") as f:
            recs = [json.loads(ln) for ln in f if ln.strip()]
        self.assertEqual(len(recs), 1, "정체 추천이 progress 마다 중복 적재됐다: %r" % recs)
        self.assertEqual(recs[0]["status"], "queued_for_weekly_digest")
        self.assertEqual(recs[0]["source"], "rsi.ceiling")

    def test_env_knob_is_positive_int_only(self):
        self.env["CYS_RSI_MAX_ROUNDS"] = "0"            # 게이트를 끄는 노브는 없다
        for _ in range(4):
            e = json.loads(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").stdout)
        self.assertEqual(e["max_rounds"], 3, "잘못된 노브 값이 기본값으로 접히지 않았다")
        self.assertEqual(e["stop_reason"], "stopped_budget")


if __name__ == "__main__":
    unittest.main(verbosity=2)
