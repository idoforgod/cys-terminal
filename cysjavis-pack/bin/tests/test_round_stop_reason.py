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


def _verdict_no_fix():
    """fix 가 빈 BLOCK — `javis_verdict` R2 강등 대상(→ INVESTIGATE). 재작성 0(§8)."""
    return {
        "verdict": "BLOCK",
        "justification": "교정안 없는 반려(검체)",
        "evidence": [{"claim": "c", "ref": "f.py:1", "verified": True}],
        "issues": [{"severity": "blocking", "where": "f.py:1", "what": "지적", "fix": ""}],
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

    def write_verdict_at(self, path, severities=("minor",), verdict="ACCEPT"):
        """임의 경로에 verdict JSON — **다른 경로로** 재평가하는 갈래를 보기 위한 헬퍼."""
        with open(path, "w", encoding="utf-8") as f:
            json.dump(_verdict(severities, verdict), f, ensure_ascii=False)
        return path

    def log_investigate(self, rnd, ev="codex"):
        """fix 없는 BLOCK = javis_verdict R2 강등 → INVESTIGATE 행(승인 아님)."""
        p = os.path.join(self.pack, "round", "_reviews", "inv-r%d-%s.json" % (rnd, ev))
        with open(p, "w", encoding="utf-8") as f:
            json.dump(_verdict_no_fix(), f, ensure_ascii=False)
        return self.orc("round-log", "--task", TASK, "--round", str(rnd),
                        "--evaluator", ev, "--verdict-json", p)

    def sidecar_damage(self):
        """사이드카에서 JSON 으로 읽히지 않는 줄 수(찢김·깨진 UTF-8 포함)."""
        if not os.path.isfile(self.sidecar):
            return 0
        bad = 0
        for chunk in open(self.sidecar, "rb").read().split(b"\n"):
            if not chunk.strip():
                continue
            try:
                json.loads(chunk.decode("utf-8"))
            except (ValueError, UnicodeDecodeError):
                bad += 1
        return bad

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

    def test_binding_without_hash_is_not_evidence(self):
        """해시 없는 결속은 증거가 아니다 — 경로만 가리키는 결속으로는 종결하지 못한다
        (손편집·기록 시점 읽기 실패로 sha256 이 null/빈 값이 된 경우 · codex 위임 검체 발견)."""
        self.seed_two_minor_rounds()
        for blank in (None, ""):
            evs = self.events()
            for e in evs:
                if e.get("event") == "verdict_src" and e.get("round") == 2:
                    e["sha256"] = blank
            with open(self.sidecar, "w", encoding="utf-8") as f:
                for e in evs:
                    f.write(json.dumps(e, ensure_ascii=False) + "\n")
            out = self.status().stdout
            self.assertIn("stop_reason=open", out)
            self.assertIn("결속이 불완전", out)

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
        """★두 호출의 **exit·문면까지** 본다(codex R1 검체 지적): 종전엔 반환을 안 봐서
        실패·무동작 핸들러도 '읽기 전용'으로 통과했다(공허한 단언)."""
        self.seed_two_minor_rounds()
        before = (open(self.ledger, "rb").read(), open(self.sidecar, "rb").read())
        r1 = self.status()
        r2 = self.status("--verdict-json", "2:codex:%s" %
                         os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json"))
        for r in (r1, r2):
            self.assertEqual(r.returncode, 0, (r.returncode, r.stdout, r.stderr))
            self.assertIn("stop_reason=stopped_stagnation", r.stdout)
        after = (open(self.ledger, "rb").read(), open(self.sidecar, "rb").read())
        self.assertEqual(before, after, "round-status 가 장부·사이드카를 건드렸다")

    # ── ⑨ 리뷰 반영 R1 — 기록되지 않은 호출은 아무것도 소진하지 않는다 ────
    def test_rejected_call_does_not_consume_override(self):
        """스키마 거부(exit 2)로 **행 0건**인 호출이 끈끈한 종결을 풀면 안 된다(claude R1 major-1).

        재현: R3 machine 행은 `--from-cmd` 없이 기록 불가(exit 2)인데, 종전엔 게이트가 그보다
        **앞서** override 를 커밋해 다음 호출이 `--override` 없이 통과했다."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        before = len(self.rows())
        r = self.orc("round-log", "--task", TASK, "--round", "3",
                     "--evaluator", "machine", "--override", "핑계")
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertEqual(len(self.rows()), before, "거부인데 행이 기록됐다")
        self.assertFalse([e for e in self.events() if e.get("event") == "override"],
                         "행 0건 호출이 재개를 소진했다")
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3,
                         "거부된 호출의 --override 로 종결이 풀렸다")

    def test_reviewer_axis_cannot_be_recorded_with_from_cmd(self):
        """리뷰어 축은 `--from-cmd` 로 기록되지 않는다(codex R1 추가 발견) — 종전엔
        `--evaluator codex --from-cmd "exit 0"` 이 JSON 검증 없이 PASS 를 남겨, 마지막-승
        규칙 때문에 앞선 BLOCK 이 승인으로 뒤집혔다."""
        self.log_reviewer(1, "codex", ("blocking",), "BLOCK")
        r = self.orc("round-log", "--task", TASK, "--round", "1",
                     "--evaluator", "codex", "--from-cmd", "exit 0")
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertIn("--verdict-json", r.stderr)
        self.assertEqual(len([x for x in self.rows() if "codex" in x]), 1, self.rows())

    def test_binding_failure_refuses_the_row(self):
        """결속이 디스크에 남지 못하면 **행도 남기지 않는다**(codex R1 blocking-1).
        행만 남으면 status 가 낡은 결속으로 정체를 선언한다. 사이드카 경로를 디렉터리로 만들어
        append 를 실패시킨다(권한 변경보다 이식적)."""
        self.seed_two_minor_rounds()
        os.remove(self.sidecar)
        os.makedirs(self.sidecar)
        before = len(self.rows())
        r = self.log_reviewer(2, "codex", ("major",))
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertIn("결속", r.stderr)
        self.assertEqual(len(self.rows()), before, "결속 없이 행이 기록됐다")

    def test_ledger_failure_refuses_and_says_so(self):
        """장부 append 실패는 exit 2 — 기록되지 않은 것을 기록됐다고 말하지 않는다."""
        os.makedirs(self.ledger)           # 장부 경로를 디렉터리로 → append 실패
        r = self.log_reviewer(1, "gemini")
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertIn("장부 기록 실패", r.stderr)

    def test_separate_path_reevaluation_wins(self):
        """재평가를 **다른 경로**의 파일로 해도 마지막 결속이 이긴다 — 종전 검체는 같은 경로를
        덮어써서 이 갈래(낡은 결속 재사용)를 못 봤다(codex R1 검체 지적)."""
        self.seed_two_minor_rounds()
        self.assertIn("stop_reason=stopped_stagnation", self.status().stdout)
        p2 = self.write_verdict_at(os.path.join(self.root, "reeval-r2-codex.json"), ("major",))
        r = self.orc("round-log", "--task", TASK, "--round", "2", "--evaluator", "codex",
                     "--verdict-json", p2)
        self.assertEqual(r.returncode, 0, r.stderr)
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)
        self.assertIn("severity major", out)

    def test_torn_last_event_does_not_resurrect_older_binding(self):
        """마지막 결속 줄이 **찢어지면** 더 낡은 결속으로 되돌아가 종결하지 않는다
        (codex R1 blocking-2 — 통째 손상만 보던 검체의 사각)."""
        self.seed_two_minor_rounds()
        p2 = self.write_verdict_at(os.path.join(self.root, "reeval-r2-codex.json"), ("major",))
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "2",
                                  "--evaluator", "codex", "--verdict-json", p2).returncode, 0)
        lines = [ln for ln in open(self.sidecar, "rb").read().split(b"\n") if ln.strip()]
        lines[-1] = lines[-1][:max(1, len(lines[-1]) // 2)]        # 부분 append(찢김) 재현
        with open(self.sidecar, "wb") as f:
            f.write(b"\n".join(lines))
        self.assertEqual(self.sidecar_damage(), 1, "찢김을 만들지 못했다(검체 무효)")
        r = self.status()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("stop_reason=open", r.stdout)
        self.assertIn("손상", r.stderr)

    def test_damaged_utf8_sidecar_does_not_crash(self):
        """깨진 UTF-8 이 두 명령을 죽이지 않는다(codex R1 major-9) — 죽으면 **완결 경로까지** 막힌다."""
        self.seed_two_minor_rounds()
        with open(self.sidecar, "ab") as f:
            f.write(b'{"event": "verdict_src", "round": 2, "evaluator": "\xed\x95"}\n')
        r = self.status()
        self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))
        self.assertIn("stop_reason=", r.stdout)
        self.assertNotIn("Traceback", r.stderr)
        r2 = self.orc("round-log", "--task", TASK, "--round", "2",
                      "--evaluator", "master", "--verdict", "approve")
        self.assertEqual(r2.returncode, 0, r2.stderr)      # 같은 라운드 완결은 계속 가능
        self.assertNotIn("Traceback", r2.stderr)

    def test_admitted_round_completes_after_concurrent_stop(self):
        """게이트를 통과해 **개시된 라운드**는 그 사이 종결이 기록돼도 완결된다
        (codex R1 blocking-4 — 좌초 방지). 새 라운드는 여전히 막힌다."""
        self.seed_two_minor_rounds()
        r = self.log_reviewer(3, "gemini", extra=("--override", "오너 지시: 보안축 재확인"))
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(self.sidecar, "a", encoding="utf-8") as f:      # 동시 writer 의 종결 주입
            f.write(json.dumps({"event": "stagnation_stop", "round": 2, "task": TASK,
                                "ts": 1.0}, ensure_ascii=False) + "\n")
        self.assertEqual(self.log_reviewer(3, "codex").returncode, 0,
                         "개시된 라운드가 좌초했다(같은 라운드 완결 불가)")
        self.assertEqual(self.log_reviewer(4, "gemini").returncode, 3,
                         "새 라운드가 종결을 무시하고 열렸다")

    def test_damaged_history_blocks_new_rounds_until_override(self):
        """손상된 이력은 '종결 없음'의 증거가 아니다(codex R1 D5) — 찢긴 줄이 종결일 수 있으니
        **새 라운드만** 막고 명시 재개를 요구한다(완결 경로는 그대로 열려 있다)."""
        for ev in ("gemini", "codex"):
            self.assertEqual(self.log_reviewer(1, ev).returncode, 0)
        self.assertEqual(self.log_machine(1).returncode, 0)
        with open(self.sidecar, "ab") as f:
            f.write(b'{"event": "stagnation_stop", "roun\n')     # 찢긴 append
        r = self.log_reviewer(2, "gemini")
        self.assertEqual(r.returncode, 3, (r.returncode, r.stdout, r.stderr))
        self.assertIn("손상", r.stderr)
        st = self.status()          # status 문면도 집행과 같은 말을 해야 한다(갈리지 않게)
        self.assertIn("손상", st.stdout)
        self.assertNotIn("진행 가능", st.stdout)
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "1",
                                  "--evaluator", "master", "--verdict", "approve").returncode, 0,
                         "손상이 **완결 경로**까지 막았다(교착)")
        r2 = self.log_reviewer(2, "gemini", extra=("--override", "손상 확인 후 재개"))
        self.assertEqual(r2.returncode, 0, r2.stderr)

    def test_skipping_round_numbers_does_not_bypass_the_stop(self):
        """번호를 건너뛴 새 라운드(R9)도 막힌다 — 완결권은 '최대 번호'가 아니라 **개시된 라운드**
        (장부에 행이 있는 라운드)에만 있다(codex R1 D2 반례)."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        self.assertEqual(self.log_reviewer(9, "codex").returncode, 3,
                         "번호를 건너뛰면 종결이 우회된다")

    def test_resolved_investigation_does_not_hold_the_round(self):
        """INVESTIGATE 를 재평가로 해소하면 조사 요구가 남지 않는다(codex R1 major-6 —
        `gate_verdicts` 와 같은 마지막-승 규칙)."""
        self.assertEqual(self.log_investigate(1, "codex").returncode, 0)
        out = self.status().stdout
        self.assertIn("stop_reason=needs_investigation", out)     # 양성 대조
        self.assertEqual(self.log_reviewer(1, "codex").returncode, 0)
        self.log_reviewer(1, "gemini")
        self.log_machine(1)
        out2 = self.status().stdout
        self.assertIn("stop_reason=open", out2)
        self.assertNotIn("조사 필요", out2)

    def test_sticky_stop_never_prints_advance_permission(self):
        """끈끈한 종결이 집행 중이면 "다음 라운드 진행 가능"을 내지 않는다(codex R1 major-7) —
        같은 소비자에게 상반된 지시를 주지 않는다."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        with open(os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json"),
                  "w", encoding="utf-8") as f:
            json.dump(_verdict(("major",)), f, ensure_ascii=False)   # 증거 무효화
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)                       # 계산은 정직하게 open
        self.assertNotIn("다음 라운드 3 진행 가능", out)
        self.assertIn("--override", out)
        self.assertIn("완결·재평가 기록은 막히지 않는다", out)

    def test_relocated_binding_is_seen_by_both_commands(self):
        """증거를 옮기면 `round-relocate` 로 결속을 옮겨야 status(판정)와 round-log(집행)가
        같은 증거를 본다(codex R1 major-8)."""
        self.seed_two_minor_rounds()
        src = os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json")
        dst = os.path.join(self.root, "moved-r2-codex.json")
        shutil.move(src, dst)
        out = self.status().stdout
        self.assertIn("stop_reason=open", out)
        self.assertIn("파일 부재", out)
        bad = self.write_verdict_at(os.path.join(self.root, "other.json"), ("major",))
        r = self.orc("round-relocate", "--task", TASK, "--round", "2",
                     "--evaluator", "codex", "--path", bad)
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertIn("다른 파일", r.stderr)
        r = self.orc("round-relocate", "--task", TASK, "--round", "2",
                     "--evaluator", "codex", "--path", dst)
        self.assertEqual(r.returncode, 0, (r.returncode, r.stdout, r.stderr))
        self.assertIn("stop_reason=stopped_stagnation", self.status().stdout)
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3,
                         "재배치 뒤에도 집행(round-log)이 같은 증거를 보지 못한다")

    def test_verdict_json_without_binding_is_noted(self):
        """결속 없는 `--verdict-json` 지정은 **무고지 폐기**가 아니라 note 1줄이다(claude R1 minor-3)."""
        self.seed_two_minor_rounds()
        p = self.write_verdict(2, "codex")
        r = self.status("--verdict-json", "2:master:%s" % p)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("무시", r.stderr)
        self.assertIn("결속 없음", r.stderr)
        r2 = self.status("--verdict-json", "9:codex:%s" % p)
        self.assertIn("정체 판정 대상", r2.stderr)

    def test_rejected_call_leaves_no_empty_ledger(self):
        """거부된 호출은 **빈 장부조차** 남기지 않는다(codex R1 위임 검체 발견) — 오타 난 task
        이름으로 machine 행을 쓰려다 거부돼도 팩에 장부 파일이 생기지 않는다."""
        r = self.orc("round-log", "--task", TASK, "--round", "1", "--evaluator", "machine")
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertFalse(os.path.exists(self.ledger), "거부인데 빈 장부가 생겼다")
        self.assertFalse(os.path.exists(self.sidecar))
        # 정상 기록에서는 장부가 헤더까지 갖춰 생성된다(지연 생성이 헤더를 빠뜨리지 않는다)
        self.assertEqual(self.log_machine(1).returncode, 0)
        text = open(self.ledger, encoding="utf-8").read()
        self.assertIn("| 라운드 | 평가자 | 기록값 | 판정 |", text)
        self.assertIn("| 1 | machine |", text)

    def test_malformed_stop_round_does_not_release_the_stop(self):
        """`stop_round` 키가 **있는데 값이 정수가 아니면** 손편집·손상이다 — '구 기록 호환'으로
        읽어 아무 종결이나 푸는 만능 재개가 되면 안 된다(codex R1 위임 검체 발견)."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)   # 종결 기록
        # 증거를 무효화해 **계산은 open** 으로 만든다 — 그래야 남은 차단이 오직 '끈끈한 종결'이고,
        # override 가 그것을 풀었는지 아닌지가 exit 코드로 갈린다(양성 대조가 성립한다).
        with open(os.path.join(self.pack, "round", "_reviews", "WP6_정체-r2-codex.json"),
                  "w", encoding="utf-8") as f:
            json.dump(_verdict(("major",)), f, ensure_ascii=False)
        for bad in (None, "bad", [], {}, True):
            evs = [e for e in self.events() if e.get("event") != "override"]
            evs.append({"event": "override", "round": 3, "stop_round": bad, "task": TASK,
                        "ts": 2.0, "reason": "손편집"})
            with open(self.sidecar, "w", encoding="utf-8") as f:
                for e in evs:
                    f.write(json.dumps(e, ensure_ascii=False) + "\n")
            self.assertEqual(self.log_reviewer(4, "gemini").returncode, 3,
                             "stop_round=%r 인 override 가 종결을 풀었다" % (bad,))
        # 대조: 올바른 stop_round 는 푼다(과잉 차단이 아니다)
        evs = [e for e in self.events() if e.get("event") != "override"]
        evs.append({"event": "override", "round": 4, "stop_round": 2, "task": TASK,
                    "ts": 3.0, "reason": "정상 재개"})
        with open(self.sidecar, "w", encoding="utf-8") as f:
            for e in evs:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")
        self.assertEqual(self.log_reviewer(4, "gemini").returncode, 0)

    # ── ⑩ round-status 진행 지시 4분기 문면(지침 소비자가 읽는 줄) ────────
    def test_status_directive_accepted(self):
        self.seed_two_minor_rounds()
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "2", "--evaluator",
                                  "master", "--verdict", "approve").returncode, 0)
        out = self.status().stdout
        self.assertIn("stop_reason=accepted", out)
        self.assertIn("합격(4자 수렴)", out)
        self.assertNotIn("진행 가능", out)

    def test_status_directive_stopped_budget(self):
        for rnd in (9, 10):
            self.log_reviewer(rnd, "gemini")
            self.log_reviewer(rnd, "codex")
            self.log_machine(rnd)
        out = self.status().stdout
        self.assertIn("stop_reason=stopped_budget", out)
        self.assertIn("상한 도달", out)
        self.assertNotIn("진행 가능", out)

    def test_status_directive_needs_investigation(self):
        self.assertEqual(self.log_investigate(1, "codex").returncode, 0)
        out = self.status().stdout
        self.assertIn("stop_reason=needs_investigation", out)
        self.assertIn("조사 필요", out)
        self.assertNotIn("진행 가능", out)

    # ── ⑪ 동시 writer — 장부 절단 0 · 사이드카 손상 0 ─────────────────────
    def test_concurrent_writers_do_not_tear_the_files(self):
        """4명이 동시에 라운드 1을 기록해도 장부가 잘리거나(초기 생성 경쟁) 사이드카가
        찢기지 않는다. 정합의 근거는 잠금이 아니라 결속·서수지만, 이 검체는 **동시 기록이
        서로를 지우지 않는다**를 본다."""
        vg = self.write_verdict(1, "gemini")
        vc = self.write_verdict(1, "codex")
        cmds = [
            [PY, ORC, "round-log", "--task", TASK, "--round", "1", "--evaluator", "gemini",
             "--verdict-json", vg],
            [PY, ORC, "round-log", "--task", TASK, "--round", "1", "--evaluator", "codex",
             "--verdict-json", vc],
            [PY, ORC, "round-log", "--task", TASK, "--round", "1", "--evaluator", "machine",
             "--from-cmd", "exit 0"],
            [PY, ORC, "round-log", "--task", TASK, "--round", "1", "--evaluator", "master",
             "--verdict", "approve"],
        ]
        procs = [subprocess.Popen(c, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  env=self.env) for c in cmds]
        for p in procs:
            self.assertEqual(p.wait(timeout=120), 0, p.stderr.read().decode("utf-8", "replace"))
        self.assertEqual(len(self.rows()), 4, self.rows())
        self.assertEqual(self.sidecar_damage(), 0, "동시 append 로 사이드카가 찢겼다")
        text = open(self.ledger, encoding="utf-8").read()
        self.assertEqual(text.count("| 라운드 | 평가자 | 기록값 | 판정 |"), 1,
                         "장부 헤더가 중복·절단됐다(초기 생성 경쟁)")


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

    def queue_recs(self):
        q = os.path.join(self.root, "_round", "learn", "digest_queue.jsonl")
        if not os.path.isfile(q):
            return []
        with open(q, encoding="utf-8") as f:
            return [json.loads(ln) for ln in f if ln.strip()]

    def flat_round(self, n=3):
        for _ in range(n):
            r = self.rsi("progress", "--round", "r1", "--score", "1.0")
            self.assertEqual(r.returncode, 0, r.stderr)

    def test_ceiling_latch_survives_state_loss(self):
        """추천 래치는 state.json **밖에도** 있어야 한다(codex R1 major-10): state 를 지우고
        같은 라운드를 다시 굴려도 큐는 1건이다(멱등키가 큐 레코드에 남는다)."""
        self.env["CYS_RSI_MAX_ROUNDS"] = "99"        # 예산이 아니라 래치를 본다
        self.assertEqual(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").returncode, 0)
        self.flat_round()
        self.assertEqual(len(self.queue_recs()), 1, self.queue_recs())
        self.assertEqual(self.queue_recs()[0]["key"], "rsi.ceiling:r1", self.queue_recs())
        os.remove(self.state)                        # state 소실(복구·되돌리기)
        self.assertEqual(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").returncode, 0)
        self.flat_round()
        self.assertEqual(len(self.queue_recs()), 1,
                         "state 소실 후 같은 라운드의 추천이 두 번 적재됐다: %r" % self.queue_recs())

    def test_ceiling_latch_survives_queue_rotation(self):
        """큐가 주간 다이제스트로 **소비·정리**된 뒤에도 ledger 이벤트가 재추천을 막는다."""
        self.env["CYS_RSI_MAX_ROUNDS"] = "99"
        self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
        self.flat_round()
        self.assertEqual(len(self.queue_recs()), 1)
        os.remove(os.path.join(self.root, "_round", "learn", "digest_queue.jsonl"))
        os.remove(self.state)
        self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
        self.flat_round()
        self.assertEqual(self.queue_recs(), [],
                         "큐 회전 뒤 같은 라운드가 다시 추천됐다: %r" % self.queue_recs())

    def test_env_knob_is_positive_int_only(self):
        self.env["CYS_RSI_MAX_ROUNDS"] = "0"            # 게이트를 끄는 노브는 없다
        for _ in range(4):
            e = json.loads(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").stdout)
        self.assertEqual(e["max_rounds"], 3, "잘못된 노브 값이 기본값으로 접히지 않았다")
        self.assertEqual(e["stop_reason"], "stopped_budget")


if __name__ == "__main__":
    unittest.main(verbosity=2)
