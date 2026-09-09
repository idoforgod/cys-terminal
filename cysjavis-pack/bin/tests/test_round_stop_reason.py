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
import time
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
ORC = os.path.join(BIN, "javis_orchestra.py")
RSI = os.path.join(BIN, "javis_rsi.py")
LEARN = os.path.join(BIN, "javis_learn.py")
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
        # ★기록값 칸은 이제 **증거 식별자**(`vj:<sha 앞 32자>`)다(R2 blocking-1 수리) — 등급이
        #   아니라 "이 행이 어느 verdict 파일로 났는가" 다. 등급·점수는 여전히 0이다.
        text = open(self.ledger, encoding="utf-8").read()
        row = [l for l in text.splitlines() if l.startswith("| 2 | gemini |")]
        self.assertEqual(len(row), 1, text)
        self.assertRegex(row[0], r"^\| 2 \| gemini \| vj:[0-9a-f]{32} \| BLOCK \|$")

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
        # ★R2 수리: 슬러그가 겹치는 **다른 task 는 이 장부에 아예 쓸 수 없다**(장부 귀속).
        #   종전엔 rc=0 으로 행이 붙었고, 그 행이 원 task 의 완결권·`gate_verdicts` 에 섞여
        #   정체 게이트가 **표기를 바꾸는 것만으로 우회**됐다(codex R2 blocking-7).
        twin = "WP6/정체"          # 같은 슬러그(WP6_정체) → 같은 장부·사이드카
        r = subprocess.run([PY, ORC, "round-log", "--task", twin, "--round", "9",
                            "--evaluator", "master", "--verdict", "approve"],
                           capture_output=True, text=True, timeout=120, env=self.env)
        self.assertEqual(r.returncode, 2, (r.returncode, r.stderr))
        self.assertIn("다른 task", r.stderr)
        self.assertNotIn("| 9 | master", open(self.ledger, encoding="utf-8").read())

    def test_colliding_task_cannot_manufacture_completion_rights(self):
        """슬러그 충돌 task 가 **완결권을 제조**하지 못한다(codex R2 blocking-7).

        시나리오: 원 task 가 R3 에서 정체 종결(exit 3) → 다른 표기(`WP6/정체`)로 R9 를 먼저
        기록 → 다시 원 task 로 R9 요청. 종전엔 두 번째가 rc=0(이미 행이 있는 라운드 = 완결권)
        이었다. 이제 충돌 task 의 쓰기 자체가 거부되므로 그 행이 존재할 수 없고, 원 task 의
        R9 는 여전히 exit 3 이다.
        """
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
        twin = "WP6/정체"
        r = subprocess.run([PY, ORC, "round-log", "--task", twin, "--round", "9",
                            "--evaluator", "machine", "--from-cmd", "exit 0"],
                           capture_output=True, text=True, timeout=120, env=self.env)
        self.assertEqual(r.returncode, 2, (r.returncode, r.stderr))
        r2 = self.log_machine(9)
        self.assertEqual(r2.returncode, 3, (r2.returncode, r2.stderr))

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
        os.makedirs(self.ledger)           # 장부 경로를 디렉터리로 → append 불가(결정론)
        r = self.log_reviewer(1, "gemini")
        self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
        self.assertIn("일반 파일이 아니다", r.stderr)
        self.assertIn("아무것도 기록되지 않았다", r.stderr)

    def test_ledger_creation_failure_is_a_refusal_not_a_traceback(self):
        """장부 **생성** 실패는 traceback + exit 1 이 아니라 exit 2 거부다(claude R2 minor).

        ★왜: 'exit 1 = 행은 기록됐고 기계검증 실패' 라는 계약과 충돌한다 — rc 단일 경로로
          판정하는 소비자가 '행이 남았다'로 오독한다.
        """
        rd = os.path.join(self.pack, "round")
        os.chmod(rd, 0o500)                # 생성 불가(읽기·실행만)
        probe = os.path.join(rd, ".probe")
        try:                               # Windows 는 디렉터리 chmod 가 무효 — 실효 없으면 SKIP
            open(probe, "w").close()
            os.remove(probe)
            os.chmod(rd, 0o700)
            self.skipTest("이 플랫폼에서 디렉터리 chmod 가 생성을 막지 못한다(Windows)")
        except OSError:
            pass
        try:
            r = self.orc("round-init", "--task", TASK)
            self.assertEqual(r.returncode, 2, (r.returncode, r.stdout, r.stderr))
            self.assertNotIn("Traceback", r.stderr)
            r2 = self.log_machine(1)
            self.assertEqual(r2.returncode, 2, (r2.returncode, r2.stdout, r2.stderr))
            self.assertNotIn("Traceback", r2.stderr)
        finally:
            os.chmod(rd, 0o700)

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


    # ── ⑨ 리뷰 반영 R2 반례 — 결속-행 세대·손상·귀속·잠금·불확정 ──────────────
    def test_row_ordinal_reuse_cannot_bind_minor_evidence_to_another_row(self):
        """행 서수 재사용으로 **남의 행에 내 증거**를 묶지 못한다(codex R2 blocking-1).

        시나리오: (R2,codex) 축에 minor 행 1개가 있다. 그 뒤 **major** 이슈를 가진 재평가 행이
        서수 2로 커밋된다. 뒤늦게 도착한 writer 가 minor 파일을 가리키는 결속을 서수 2로
        남긴다(마지막-승). 종전엔 서수만 맞으면 통과해 **major 행이 minor 증거로 판정**됐다.
        이제 행 스스로가 어느 증거로 났는지(기록값 칸의 `vj:` 식별자) 말하므로 짝이 어긋난다.
        """
        self.seed_two_minor_rounds()
        minor_b = [e for e in self.events()
                   if e.get("event") == "verdict_src" and e.get("round") == 2
                   and e.get("evaluator") == "codex"][-1]
        r = self.log_reviewer(2, "codex", ("major",))     # 서수 2 = major 행
        self.assertEqual(r.returncode, 0, r.stderr)
        forged = dict(minor_b)                            # 낡은(minor) 결속을 서수 2로 재기록
        forged.pop("_line", None)
        forged["row_ordinal"] = 2
        with open(self.sidecar, "a", encoding="utf-8") as f:
            f.write(json.dumps(forged, ensure_ascii=False) + "\n")
        out = self.status().stdout
        self.assertNotIn("stop_reason=stopped_stagnation", out)
        self.assertIn("증거 불일치", out + self.status().stderr)

    def test_damaged_row_does_not_resurrect_older_approval(self):
        """장부의 **읽히지 않는 거절 행**이 낡은 승인을 부활시키지 못한다(codex R2 blocking-5).

        두 변형을 모두 본다: ⓐ깨진 UTF-8 바이트 ⓑ유효 UTF-8인데 구조가 깨진 행(첫 `|`→`!`).
        둘 다 종전엔 정규식에 안 걸려 **행이 사라졌고**, 마지막-승 규칙이 직전 PASS 를 되살려
        정체가 성립했다.
        """
        for mutate in ("utf8", "struct"):
            self.setUp()
            self.seed_two_minor_rounds()
            self.assertIn("stop_reason=stopped_stagnation", self.status().stdout)
            raw = open(self.ledger, "rb").read()
            bad = b"| 2 | machine | - | FAIL(exit 1) |\n"
            open(self.ledger, "wb").write(raw + bad)
            self.assertIn("stop_reason=open", self.status().stdout, "정상 FAIL 행이 안 읽힌다")
            raw2 = open(self.ledger, "rb").read()
            if mutate == "utf8":
                broken = raw2.replace(b"| 2 | machine | - | FAIL(exit 1) |",
                                      b"| 2 | mach\xffne | - | FAIL(exit 1) |")
            else:
                broken = raw2.replace(b"| 2 | machine | - | FAIL(exit 1) |",
                                      b"! 2 | machine | - | FAIL(exit 1) |")
            open(self.ledger, "wb").write(broken)
            out = self.status().stdout
            self.assertNotIn("stop_reason=stopped_stagnation", out, mutate)
            self.assertNotIn("stop_reason=accepted", out, mutate)
            g = self.orc("gate-status", "--task", TASK)
            self.assertNotEqual(g.returncode, 0, "손상 장부에서 수렴(자동 착수)이 열렸다")

    def test_ledger_damage_is_recoverable_by_rerecording(self):
        """손상은 **영구 불통**이 아니다 — 손상 줄 뒤에 다시 기록하면 판정이 회복된다.

        (codex R2 major-8: 옛 설명문 한 바이트가 이후 모든 라운드의 승인을 영원히 막으면
        복구 경로가 '역사 삭제' 뿐이 된다.)
        ★재기록 대상에 **행이 없던 축(master)** 도 포함된다(X-3 R2 재수리 · 의도적 변경):
          회복 경계를 넘는 유일한 수단이 '손상 뒤의 행'인데 행이 없는 축은 그것을 만들 수
          없다. 종전 판은 그 축을 면제했고, 그래서 **사라진 master 반려 위에서** 종결이
          선언됐다(X-3). 여기서 확인하는 것은 그 면제가 사라진 뒤에도 **회복이 실재한다**는
          것이다 — master 를 명시 기록(`SKIPPED: <사유>`)하면 정체 판정이 되돌아온다.
        """
        self.seed_two_minor_rounds()
        raw = open(self.ledger, "rb").read()
        open(self.ledger, "wb").write(raw.replace(b"| 1 | machine", b"| 1 | mach\xffne", 1))
        self.assertNotIn("stop_reason=stopped_stagnation", self.status().stdout)
        for rnd in (1, 2):                      # 손상 줄 **뒤에** 축을 다시 기록
            self.assertEqual(self.log_reviewer(rnd, "gemini").returncode, 0)
            self.assertEqual(self.log_reviewer(rnd, "codex").returncode, 0)
            self.assertEqual(self.log_machine(rnd).returncode, 0)
        self.assertNotIn("stop_reason=stopped_stagnation", self.status().stdout,
                         "행이 하나도 없는 master 축이 회복 경계를 면제받았다(X-3 재발)")
        for rnd in (1, 2):                      # 사라졌을 수 있는 축도 명시 기록해야 회복된다
            self.assertEqual(self.orc("round-log", "--task", TASK, "--round", str(rnd),
                                      "--evaluator", "master",
                                      "--verdict", "SKIPPED: 손상 확인 후 유보").returncode, 0)
        self.assertIn("stop_reason=stopped_stagnation", self.status().stdout,
                      "재기록으로 회복되지 않는다(회복 경계 없음)")

    def test_damage_does_not_permanently_block_approval(self):
        """손상 뒤 **네 축을 다시 기록**하면 합격(자동 착수)이 열린다 — 전면 차단 아님.

        (codex R2 major-8 의 방향을 R2 재수리가 되살리지 않았다는 양성 대조: '행이 없는 축은
        회복 경계를 넘을 수 없다'가 **승인의 영구 불통**을 만들지 않는다. 합격은 어차피 네 축이
        모두 그 라운드에 기록돼야 성립하므로, 손상 뒤 재기록이면 그대로 열린다.)
        """
        for ev in ("gemini", "codex"):
            self.assertEqual(self.log_reviewer(1, ev).returncode, 0)
        self.assertEqual(self.log_machine(1).returncode, 0)
        raw = open(self.ledger, "rb").read()
        open(self.ledger, "wb").write(raw.replace(b"| 1 | gemini", b"| 1 | gem\xffni", 1))
        self.assertIn("stop_reason=open", self.status().stdout, "전제 불성립(손상 상태)")
        for ev in ("gemini", "codex"):
            self.assertEqual(self.log_reviewer(1, ev).returncode, 0)
        self.assertEqual(self.log_machine(1).returncode, 0)
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "1", "--evaluator",
                                  "master", "--verdict", "approve").returncode, 0)
        self.assertIn("stop_reason=accepted", self.status().stdout,
                      "손상이 **승인**을 영구히 막았다(codex R2 major-8 방향 재발)")
        g = self.orc("gate-status", "--task", TASK)
        self.assertIn(g.returncode, (0, 4),          # 4 = 수렴했으나 임무 미지정(수렴 자체는 성립)
                      "재기록으로 회복됐는데 수렴이 열리지 않는다(%s · %s)"
                      % (g.returncode, g.stderr.strip()))

    def test_surviving_earlier_row_does_not_hide_a_vanished_master_rejection(self):
        """손상 줄 **앞의** 살아남은 행(R0 승자 기록)이 사라진 master 반려를 가리면 안 된다.

        (reviewer-claude·reviewer-codex 잔여 major · X-3 PARTIAL) 종전 수리는 손상이 **장부
        전체의 머리**에 있을 때만 '행 없는 축'을 stale 로 뒀다(`worst < min(known)`). 그런데
        그 '앞 행'은 예외가 아니라 **기본값**이다 — `javis_compete` 는 승자를 `--round 0` 으로
        장부 머리에 기록한다. 그래서 R0 행 하나만 있어도 head_damage=False 가 되어, 유일한
        master BLOCK 이 손상으로 사라진 장부 위에서 `stopped_stagnation`(종결·정지)이 그대로
        선언됐다 — triage 가 major 로 판정한 '끝내는 쪽의 오답'이다.
        """
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "0", "--evaluator",
                                  "machine", "--from-cmd", "exit 0").returncode, 0)
        self.assertEqual(self.orc("round-log", "--task", TASK, "--round", "1", "--evaluator",
                                  "master", "--verdict", "BLOCK").returncode, 0)
        raw = open(self.ledger, "rb").read()
        block = b"| 1 | master | - | BLOCK |"
        self.assertIn(block, raw, "전제 불성립: master 반려 행이 기록되지 않았다")
        open(self.ledger, "wb").write(raw.replace(block, b"| 1 | mast\xffr | - | BLOCK |", 1))
        for rnd in (1, 2):                      # 손상 **뒤에** 세 축을 기록(회복 경계 통과)
            for ev in ("gemini", "codex"):
                self.assertEqual(self.log_reviewer(rnd, ev).returncode, 0)
            self.assertEqual(self.log_machine(rnd).returncode, 0)
        out = self.status().stdout
        self.assertNotIn("stop_reason=stopped_stagnation", out,
                         "사라진 master 반려 위에서 종결(정체)을 선언했다:\n%s" % out)
        self.assertNotIn("stop_reason=accepted", out, "사라진 반려 위에서 합격을 선언했다")
        self.assertNotEqual(self.orc("gate-status", "--task", TASK).returncode, 0,
                            "사라진 master 반려 위에서 자동 착수가 열렸다")

    def test_empty_orphan_lock_dir_does_not_refuse_recording_forever(self):
        """**빈** 잠금 디렉터리(mkdir 직후 사망)가 영구 교착이 되면 안 된다.

        (reviewer-claude·reviewer-codex 잔여 major · X-1 회귀) rename 청구는 owner 파일이,
        잔재 회수는 청구 파일이 있을 때만 동작한다 — 둘 다 없는 **빈** 잠금은 어느 경로에도
        걸리지 않아 `_reclaim_orphan` 이 항상 False 를 냈다. 그러면 `round-log` 가 매번
        `거부: 다른 writer 가 …(고아 잠금은 300초 뒤 자동 회수)` rc=2 를 내는데 그 안내가
        **영원히** 거짓말이 된다(사람이 잠금을 지울 때까지 그 task 의 장부가 안 쓰인다).
        """
        self.assertEqual(self.log_reviewer(1, "gemini").returncode, 0)
        lock = self.ledger + ".lock"
        os.mkdir(lock)                                   # owner 를 쓰기 전에 죽은 형상
        old = time.time() - 10 * 3600
        os.utime(lock, (old, old))
        r = self.log_reviewer(1, "codex")
        self.assertEqual(r.returncode, 0,
                         "빈 고아 잠금이 기록을 영구 거부했다(rc=%s · %s)" % (r.returncode, r.stderr))
        self.assertNotIn("| 1 | codex", "".join(self.rows()[:1]))
        self.assertTrue(any("| 1 | codex" in ln for ln in self.rows()),
                        "거부는 안 했는데 행이 남지 않았다")

    def test_unreadable_sidecar_is_not_proof_of_empty_history(self):
        """사이드카를 **읽을 수 없는 것**은 '이력 없음'이 아니다(codex R2 blocking-6)."""
        self.seed_two_minor_rounds()
        self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)   # 끈끈한 종결 기록
        os.remove(self.sidecar)
        os.makedirs(self.sidecar)              # 판독 불가(디렉터리 — OS 중립)
        r = self.log_machine(9)
        self.assertEqual(r.returncode, 3, (r.returncode, r.stderr))
        self.assertIn("읽을 수 없다", r.stderr)
        r2 = self.log_machine(9, extra=("--override", "판독 불가 확인 후 재개"))
        self.assertIn(r2.returncode, (0, 1), (r2.returncode, r2.stderr))

    def test_malformed_numeric_stop_round_does_not_release_the_stop(self):
        """`stop_round: 2.9`·`Infinity` 는 종결을 풀지 않는다(codex R2 blocking-8).

        종전엔 `int(2.9)==2` 가 라운드 2 의 종결을 풀었고 `Infinity` 는 **잡히지 않는
        OverflowError** 였다. 또 `round: Infinity` 는 판정기 자체를 죽였다.
        """
        for payload in ('{"event":"override","task":"%s","round":9,"stop_round":2.9,'
                        '"reason":"x"}' % TASK,
                        '{"event":"override","task":"%s","round":9,"stop_round":Infinity,'
                        '"reason":"x"}' % TASK,
                        '{"event":"override","task":"%s","round":Infinity,"stop_round":2,'
                        '"reason":"x"}' % TASK):
            self.setUp()
            self.seed_two_minor_rounds()
            self.assertEqual(self.log_reviewer(3, "gemini").returncode, 3)
            with open(self.sidecar, "a", encoding="utf-8") as f:
                f.write(payload + "\n")
            r = self.log_machine(9)
            self.assertNotIn("Traceback", r.stderr, payload)
            self.assertEqual(r.returncode, 3, (payload, r.returncode, r.stderr))

    def test_null_byte_in_binding_path_does_not_crash_judgment(self):
        """사이드카의 널 바이트 경로가 판정기를 죽이지 않는다(codex R2 major)."""
        self.seed_two_minor_rounds()
        evs = self.events()
        for e in evs:
            if e.get("event") == "verdict_src" and e.get("round") == 2:
                e["path"] = "x\u0000y"
        with open(self.sidecar, "w", encoding="utf-8") as f:
            for e in evs:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")
        r = self.status()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("Traceback", r.stderr)
        self.assertNotIn("stop_reason=stopped_stagnation", r.stdout)
        r2 = self.log_machine(9)
        self.assertNotIn("Traceback", r2.stderr)

    def test_lock_contention_refuses_instead_of_racing(self):
        """잠금 경합이면 **쓰지 않고 거부**한다(codex R2 blocking-1·3) — init 도 같은 뮤텍스."""
        lock = self.ledger + ".lock"
        os.makedirs(lock)
        with open(os.path.join(lock, "owner"), "wb") as f:
            f.write(b"other-writer")
        try:
            r = self.orc("round-init", "--task", TASK)
            self.assertEqual(r.returncode, 2, (r.returncode, r.stderr))
            self.assertIn("잠금", r.stderr)
            self.assertFalse(os.path.exists(self.ledger), "경합 중에 장부가 생겼다")
            r2 = self.log_machine(1)
            self.assertEqual(r2.returncode, 2, (r2.returncode, r2.stderr))
            self.assertFalse(os.path.exists(self.ledger))
        finally:
            shutil.rmtree(lock, ignore_errors=True)

    def test_readonly_ledger_refuses_and_unwritable_is_unknown(self):
        """쓰기 실패의 두 갈래를 **다른 값**으로 말한다: 되읽기 성공=exit 2(기록 없음) ·
        되읽기 실패=exit 5(불확정). 둘을 한 값으로 접으면 어느 쪽이든 거짓말이다."""
        self.assertEqual(self.log_machine(1).returncode, 0)
        os.chmod(self.ledger, 0o400)
        try:
            with open(self.ledger, "a", encoding="utf-8"):
                pass
            os.chmod(self.ledger, 0o600)
            self.skipTest("이 플랫폼에서 파일 chmod 가 쓰기를 막지 못한다")
        except OSError:
            pass
        try:
            r = self.orc("round-log", "--task", TASK, "--round", "1",
                         "--evaluator", "machine", "--from-cmd", "exit 0")
            self.assertEqual(r.returncode, 2, (r.returncode, r.stderr))
            self.assertIn("장부 기록 실패", r.stderr)
        finally:
            os.chmod(self.ledger, 0o600)
        os.chmod(self.ledger, 0o000)
        try:
            readable = True
            try:
                open(self.ledger, "rb").close()
            except OSError:
                readable = False
            r2 = self.orc("round-log", "--task", TASK, "--round", "1",
                          "--evaluator", "machine", "--from-cmd", "exit 0")
            self.assertNotIn("Traceback", r2.stderr)
            self.assertEqual(r2.returncode, 2 if readable else 5, (r2.returncode, r2.stderr))
            if not readable:
                self.assertIn("불확정", r2.stderr)
        finally:
            os.chmod(self.ledger, 0o600)

    def test_relocate_does_not_launder_pre_damage_binding(self):
        """재배치는 **손상 이전** 결속을 새 줄 번호로 세탁하지 못한다(codex R2 blocking-4)."""
        self.seed_two_minor_rounds()
        paths = {}
        for rnd in (1, 2):
            for ev in ("gemini", "codex"):
                paths[(rnd, ev)] = os.path.join(
                    self.pack, "round", "_reviews", "WP6_정체-r%d-%s.json" % (rnd, ev))
        with open(self.sidecar, "ab") as f:
            f.write(b'{"event":"verdict_src","round":2,')      # 찢긴 줄
        self.assertNotIn("stop_reason=stopped_stagnation", self.status().stdout)
        for (rnd, ev), path in sorted(paths.items()):
            r = self.orc("round-relocate", "--task", TASK, "--round", str(rnd),
                         "--evaluator", ev, "--path", path)
            self.assertEqual(r.returncode, 2, (rnd, ev, r.returncode, r.stderr))
            self.assertIn("손상", r.stderr)
        self.assertNotIn("stop_reason=stopped_stagnation", self.status().stdout,
                         "재배치가 손상 이전 증거를 세탁했다")

    def test_ignored_override_is_announced(self):
        """게이트 미발동 호출의 `--override` 는 **조용히 사라지지 않는다**(claude R2 minor)."""
        r = self.log_machine(1, extra=("--override", "재개 사유"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("적용되지 않았다", r.stderr)
        self.assertEqual([e for e in self.events() if e.get("event") == "override"], [])

    # ── ⑧ 독립 재유도(triage) 반례 — 3R 잔여 지적의 재현 핀 (2026-09-08) ────────
    # 판정자는 산출자도 직전 리뷰어도 아니다(CONTRACTS §E-1). 아래 검체는 각 지적을
    # **현재 HEAD 에서 실패하는 최소 반례**로 고정한 것이며, 수리 방식은 규정하지 않는다.

    def test_triage_unreadable_ledger_is_not_an_empty_history(self):
        """**판독 불가** 장부를 '기록 없음'으로 접고 "다음 라운드 진행 가능" 을 내면 안 된다.

        (reviewer-claude 잔여 major: 수리 축 ⑥ '판독 불가 ≠ 이력 없음' 이 사이드카
        (`read_round_events`·`history_unknown`)에만 적용됐다. 장부 쪽은 `unreadable` 을
        stderr 주의로만 흘리고, `round_stop_reason` 은 rows 가 비었다는 이유로
        `last<=0 → open · "기록된 라운드 없음"` 으로 접어 `damage_blocks_round` 를 아예
        보지 않는다 — 소비자가 읽는 stdout 은 **빈 장부와 구별되지 않는다**.)
        장부 자리를 디렉터리로 만드는 형상은 chmod 없이도 읽기를 실패시킨다(Windows 포함).
        """
        os.makedirs(self.ledger)                     # 파일 자리 = 디렉터리 → open 실패(OSError)
        r = self.status()
        self.assertNotIn("Traceback", r.stderr)
        self.assertNotIn("진행 가능", r.stdout,
                         "판독 불가 장부에서 새 라운드 진행을 지시했다:\n%s" % r.stdout)
        self.assertNotIn("기록된 라운드 없음", r.stdout,
                         "판독 불가를 '이력 없음'으로 보고했다:\n%s" % r.stdout)

    def test_triage_negative_round_does_not_damage_the_ledger(self):
        """`--round -1` 은 도구가 **스스로 판독 불가 줄**을 써 넣어 수렴을 뒤집는다(rc=0 보고).

        (reviewer-claude 잔여 major) `LEDGER_ROW_RE` 는 라운드를 `[0-9]+` 로만 읽으므로 `| -1 | … |` 를 행으로
        읽지 못하고, `_row_candidate` 는 그 줄을 **손상**으로 센다. 그 손상 줄은 파일 끝에
        붙으므로 `damage_blocks_round` 의 회복 경계에서 **모든 축이 stale** 이 되어 이미
        `accepted` 인 라운드가 `open` 으로 되돌아간다. 게이트는 음수 라운드를 통과시키고
        (`args.round <= 0` 은 R0 취급) 호출은 rc=0 "기록:" 을 낸다.
        수리 방향은 자유다(거부하거나, 행이 되게 쓰거나) — 이 핀은 **결과**만 못박는다:
        성공을 보고한 호출이 자기 장부를 판독 불가로 만들지 않는다.
        """
        for ev in ("gemini", "codex"):
            self.assertEqual(self.log_reviewer(1, ev).returncode, 0)
        self.assertEqual(self.log_machine(1).returncode, 0)
        r = self.orc("round-log", "--task", TASK, "--round", "1",
                     "--evaluator", "master", "--verdict", "ACCEPT")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("stop_reason=accepted", self.status().stdout, "전제 불성립(수렴 상태)")
        neg = self.orc("round-log", "--task", TASK, "--round", "-1",
                       "--evaluator", "master", "--verdict", "ACCEPT")
        st = self.status()
        self.assertNotIn("판독 불가", st.stderr,
                         "도구가 자기 손으로 손상 줄을 만들었다(rc=%s · %s)"
                         % (neg.returncode, neg.stdout.strip()))
        self.assertIn("stop_reason=accepted", st.stdout,
                      "음수 라운드 한 줄이 수렴 판정을 뒤집었다:\n%s" % st.stdout)

    def test_triage_stale_stop_does_not_reclose_an_approved_resume(self):
        """잠금 **밖에서** 계산한 종결이 성공한 override 뒤에 다시 기록되면 안 된다.

        (reviewer-codex 잔여 major) `cmd_round_log` 는 rows·events 를 맨 위에서 읽고
        `stagnation_gate(holding=False)` 가 그 **스냅샷으로** `stagnation_stop` 을 append 한다.
        append 는 잠금을 쥐지만 rows/events 를 **재조회하지 않는다** — 그 사이(사이드카 잠금
        대기는 최대 5s) 다른 writer 가 종결+명시 override 를 기록하면, 뒤늦은 append 가
        승인된 재개를 다시 닫는다. 여기서는 A 의 스냅샷을 실제로 먼저 떠서 그 순서를 고정한다.
        """
        import argparse
        sys.path.insert(0, BIN)
        import javis_orchestra as orc
        prev = os.environ.get("CYS_PACK_DIR")
        os.environ["CYS_PACK_DIR"] = self.pack
        try:
            self.seed_two_minor_rounds()
            rows, rdmg = orc.parse_rounds(self.ledger, with_damage=True)     # A 의 스냅샷
            events, dmg = orc.read_round_events(TASK, with_damage=True)      # (종결 기록 전)
            r = self.orc("round-log", "--task", TASK, "--round", "3",
                         "--evaluator", "master", "--verdict", "ACCEPT")
            self.assertEqual(r.returncode, 3, (r.returncode, r.stderr))      # B: 종결 기록
            r = self.orc("round-log", "--task", TASK, "--round", "3", "--evaluator", "master",
                         "--verdict", "ACCEPT", "--override", "오너 승인 재개")
            self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))      # B: 명시 재개
            self.assertIsNone(orc.stagnation_block_round(orc.read_round_events(TASK), TASK),
                              "전제 불성립: override 가 종결을 풀지 못했다")
            args = argparse.Namespace(task=TASK, round=4, override=None)
            code, _pend = orc.stagnation_gate(args, self.ledger, rows, events, dmg, rdmg)
            self.assertEqual(code, 3, "전제 불성립: 낡은 스냅샷이 종결로 계산되지 않았다")
            self.assertIsNone(orc.stagnation_block_round(orc.read_round_events(TASK), TASK),
                              "승인된 재개가 **낡은 스냅샷의 종결 재기록**으로 다시 닫혔다: %r"
                              % (self.events(),))
        finally:
            if prev is None:
                os.environ.pop("CYS_PACK_DIR", None)
            else:
                os.environ["CYS_PACK_DIR"] = prev


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

    def test_damaged_ledger_line_neither_crashes_nor_burns_other_rounds(self):
        """깨진 ledger 1줄이 ⓐ도구를 죽이지 않고 ⓑ **남의 라운드 예산을 태우지 않는다**.

        (claude R2 major-2 = 영구 크래시 · codex R2 major-10 = 손상 줄을 시도수에 더하면
        신규 라운드가 첫 호출부터 `stopped_budget` 이 된다.)
        """
        self.assertEqual(self.rsi("checkpoint", "--round", "rA", "--score", "1.0").returncode, 0)
        led = os.path.join(self.root, "_round", "rsi", "ledger.jsonl")
        with open(led, "ab") as f:
            for _ in range(3):
                f.write(b'{"event":"progress","round":"rA","note":"\xed\x95"}\n')
        r = self.rsi("checkpoint", "--round", "rB", "--score", "1.0")
        self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))
        self.assertNotIn("Traceback", r.stderr)
        e = json.loads(r.stdout)
        self.assertEqual(e["attempts"], 1, "남의 손상 줄이 신규 라운드 예산을 태웠다")
        self.assertEqual(e["stop_reason"], "open")
        self.assertEqual(e.get("ledger_damaged"), 3)
        self.assertTrue(e.get("budget_unknown"), "손상을 감췄다(불확정 표기 없음)")
        r2 = self.rsi("progress", "--round", "rB", "--score", "2.0")
        self.assertEqual(r2.returncode, 0, r2.stderr)

    def test_rsi_attempts_do_not_leak_into_learn_judge_shopping_cap(self):
        """RSI 시도수가 미러를 타고 `javis_learn` 의 judge-shopping 상한으로 **새지 않는다**.

        (claude R2 major-1 실측 재현: rsi checkpoint + progress×3 뒤 첫 `learn evaluate` 가
        'evaluate 4회 기록 — 4회째=ESCALATE'(fail 9)로 막혔다 — learn 평가는 0회인데.)
        """
        self.assertEqual(self.rsi("checkpoint", "--round", "r1", "--score", "10").returncode, 0)
        for _ in range(3):
            self.rsi("progress", "--round", "r1", "--score", "10")
        mirror = json.load(open(os.path.join(self.root, "_round", "learn", "state.json"),
                                encoding="utf-8"))
        rec = mirror["rounds"]["r1"]
        self.assertNotIn("attempts", rec, "learn 이 읽는 키 이름 그대로 미러됐다")
        self.assertEqual(rec.get("rsi_attempts"), 4)
        r = subprocess.run([PY, LEARN, "evaluate", "--round", "r1", "--score", "20",
                            "--baseline"], cwd=self.root, capture_output=True, text=True,
                           timeout=120, env=self.env)
        self.assertEqual(r.returncode, 0, (r.returncode, r.stdout, r.stderr))
        self.assertEqual(json.loads(r.stdout)["attempt"], 1, r.stdout)

    def test_ceiling_latch_backfills_the_ledger_leg(self):
        """큐에만 남은 래치는 **ledger 다리를 메운다**(codex R2 major-11): 큐 회전 + state 소실
        뒤에도 같은 추천이 다시 나가지 않는다."""
        self.env["CYS_RSI_CEILING_FLATS"] = "2"
        self.env["CYS_RSI_MAX_ROUNDS"] = "99"
        self.rsi("checkpoint", "--round", "r1", "--score", "5.0")
        for _ in range(2):
            self.rsi("progress", "--round", "r1", "--score", "5.0")
        led = os.path.join(self.root, "_round", "rsi", "ledger.jsonl")
        keep = [l for l in open(led, encoding="utf-8")
                if json.loads(l).get("event") != "ceiling_recommend"]
        open(led, "w", encoding="utf-8").writelines(keep)      # ledger 다리만 제거
        self.rsi("progress", "--round", "r1", "--score", "5.0")   # 복구 호출
        evs = [json.loads(l) for l in open(led, encoding="utf-8")]
        self.assertTrue(any(e.get("event") == "ceiling_recommend" and e.get("backfilled")
                            for e in evs), "ledger 래치가 메워지지 않았다")
        q = os.path.join(self.root, "_round", "learn", "digest_queue.jsonl")
        open(q, "w", encoding="utf-8").write("")                  # 큐 회전
        os.remove(os.path.join(self.root, "_round", "rsi", "state.json"))
        self.rsi("checkpoint", "--round", "r1", "--score", "5.0")
        self.rsi("progress", "--round", "r1", "--score", "5.0")
        lines = [l for l in open(q, encoding="utf-8") if l.strip()]
        self.assertEqual(len(lines), 0, ("세 근거가 남아 있는데 재추천됐다", lines))

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
        # ★재핀(독립 재유도 X-5 · 의도적 키 변경): 멱등키에 **프로젝트 신원**이 들어간다. 종전
        #   `rsi.ceiling:<라운드>` 는 팩을 공유하는 다른 프로젝트의 같은 라운드 추천을 영구
        #   억제했다(라운드 id 는 `r1` 처럼 짧아 충돌이 기본값이다). 여기서는 **형태**만 핀한다.
        key = self.queue_recs()[0]["key"]
        self.assertTrue(key.startswith("rsi.ceiling:") and key.endswith(":r1")
                        and len(key.split(":")) == 3 and key.split(":")[1],
                        "멱등키에 프로젝트 신원이 없다: %r" % key)
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


    def test_triage_unreadable_ledger_is_not_an_empty_history(self):
        """**읽을 수 없는** ledger 를 '이력 없음'으로 접으면 라운드 예산이 조용히 리셋된다.

        (reviewer-codex 잔여 major) `_read_ledger` 의 `except OSError: return recs, damaged`
        는 파일 **부재**와 권한·I/O 실패를 구분하지 않는다(`_load_state` 도 같다). 그래서
        state 가 소실·복원된 형상에서 이력이 통째로 안 읽히면 `attempts=1 · stop_reason=open`
        을 **확정으로** 보고한다 — 같은 모듈이 손상 줄 1개에는 `budget_unknown` 을 붙여
        "모름을 모른다" 고 말하는데(§8-1 M5), 전면 판독 불가에는 그 표기가 없다.
        """
        self.assertEqual(self.rsi("checkpoint", "--round", "r1", "--score", "1.0").returncode, 0)
        for _ in range(2):
            self.assertEqual(self.rsi("progress", "--round", "r1", "--score", "1.0").returncode, 0)
        led = os.path.join(self.root, "_round", "rsi", "ledger.jsonl")
        os.remove(self.state)                       # state 소실(재시작·복원)
        os.chmod(led, 0o222)                        # append 는 되고 읽기는 안 되는 형상
        try:
            open(led, "rb").close()
            os.chmod(led, 0o600)
            self.skipTest("이 플랫폼에서 chmod 가 읽기를 막지 못한다")
        except OSError:
            pass
        try:
            r = self.rsi("checkpoint", "--round", "r1", "--score", "1.0")
            self.assertEqual(r.returncode, 0, (r.returncode, r.stderr))
            self.assertNotIn("Traceback", r.stderr)
            e = json.loads(r.stdout)
            self.assertTrue(e.get("budget_unknown"),
                            "판독 불가 ledger 를 빈 이력으로 접고 attempts=%s 를 확정으로 "
                            "보고했다: %s" % (e.get("attempts"), r.stdout.strip()))
        finally:
            os.chmod(led, 0o600)


class Wp6TriageLockOwnership(unittest.TestCase):
    """고아 잠금 회수는 **새 소유자의 잠금**을 지우지 않는다 (독립 재유도 · 2026-09-08).

    (reviewer-codex 잔여 blocking) `_best_effort_lock.__enter__` 의 회수는 토큰·mtime 확인과
    `unlink`·`rmdir` 가 원자적이지 않다. 마지막 확인 **뒤** 소유자가 바뀌면 남의 잠금을 지우고
    자기가 쥔다 — 그러면 두 writer 가 동시에 임계구간에 들어가고, `write_ledger_header` 의
    `os.replace` 가 먼저 커밋된 장부 행을 덮을 수 있다(기록 유실).
    검체는 그 순간을 결정론으로 고정한다: A 가 "고아다" 를 확인한 직후(첫 `os.unlink` 호출
    경계에서) B 가 회수를 마치고 **살아 있는** 새 잠금을 쥔다.
    """

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.base = os.path.join(self.root, "ORCHESTRATION-t.md")
        self.lock = self.base + ".lock"
        sys.path.insert(0, BIN)
        import javis_orchestra
        self.orc = javis_orchestra

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_orphan_reclaim_does_not_delete_a_new_owners_lock(self):
        import time as _time
        os.mkdir(self.lock)
        with open(os.path.join(self.lock, "owner"), "wb") as f:
            f.write(b"DEAD-OWNER")
        old = _time.time() - 10 * 3600
        os.utime(self.lock, (old, old))

        real_unlink, raced = os.unlink, []

        def racing_unlink(p, *a, **kw):
            # A 가 고아 판정을 마치고 **지우기 직전**에 멈춘 사이 B 가 회수하고 새로 쥔다.
            if not raced and os.path.dirname(str(p)) == self.lock:
                raced.append(True)
                shutil.rmtree(self.lock, ignore_errors=True)
                os.mkdir(self.lock)
                with open(os.path.join(self.lock, "owner"), "wb") as f:
                    f.write(b"NEW-LIVE-OWNER")
            return real_unlink(p, *a, **kw)

        os.unlink = racing_unlink
        try:
            with self.orc._best_effort_lock(self.base, wait=2.0, stale=1.0) as lk:
                held = bool(lk.held)
                try:
                    with open(os.path.join(self.lock, "owner"), "rb") as f:
                        owner = f.read()
                except OSError as e:
                    owner = b"<%s>" % str(e).encode("utf-8", "replace")
        finally:
            os.unlink = real_unlink
        self.assertTrue(raced, "전제 불성립: 고아 회수 경로에 도달하지 못했다")
        self.assertEqual(owner, b"NEW-LIVE-OWNER",
                         "회수가 **새 소유자의 잠금**을 지웠다(owner=%r · held=%s)" % (owner, held))
        self.assertFalse(held, "남의 잠금을 지우고 자기가 쥐었다 — 동시 임계구간")


class Wp6TriageDamageRecovery(unittest.TestCase):
    """손상 회복 경계 — **사라진 축**은 '손상 없음'이 아니다 (독립 재유도 · 2026-09-08)."""

    def setUp(self):
        sys.path.insert(0, BIN)
        import javis_orchestra
        self.orc = javis_orchestra

    def test_vanished_master_rejection_is_not_stagnation(self):
        """유일한 master 반려 행이 손상으로 **사라지면** 정체를 선언해서는 안 된다.

        (reviewer-codex 잔여 major) `damage_blocks_round` 는 `if seen and …` 로 **현재 파싱된
        행이 있는 축만** 검사한다. 손상 줄의 귀속은 알 수 없는데, 그 줄이 유일한 master BLOCK
        이었으면 master 는 검사 대상에서 아예 빠지고 `gate_verdicts` 에서도 `None`(미기록)이라
        정체 판정의 ③(미승인 0)을 통과한다 — 해소되지 않은 반려 위에서 `stopped_stagnation`
        (=종결·정지)이 선언된다. 다른 세 축은 손상 **뒤에** 재기록돼 회복 경계를 통과한다.
        """
        o = self.orc
        rows, ln = [], 21                      # 20행 = 손상(사라진 master BLOCK)
        for rnd in (1, 2):
            for ev in ("gemini", "codex", "machine"):
                rows.append({"round": rnd, "evaluator": ev, "score": "-",
                             "verdict": "PASS(exit 0)" if ev == "machine" else "ACCEPT",
                             "_line": ln})
                ln += 1
        dmg = {"damaged": 1, "lines": [20], "last_damaged_line": 20, "unreadable": ""}
        ok = {"ok": True, "verdict": "ACCEPT", "severities": ["minor"]}
        evidence = {(r, e): ok for r in (1, 2) for e in ("gemini", "codex")}
        # 양성 대조: 손상이 없으면 이 상태는 정체다(검체가 무언가를 재고 있다는 증거).
        self.assertEqual(o.round_stop_reason(rows, evidence)[0], "stopped_stagnation")
        reason, why = o.round_stop_reason(rows, evidence, row_damage=dmg)
        self.assertNotEqual(reason, "stopped_stagnation",
                            "귀속을 모르는 손상 줄 위에서 종결을 선언했다: %r" % (why,))


    def test_one_surviving_row_before_the_damage_does_not_exempt_a_vanished_axis(self):
        """(X-3 PARTIAL · reviewer-codex) 손상 줄 앞의 **살아남은 행 하나**가 '행 없는 축'을
        면제하면 안 된다. 두 변형을 다 본다: ⓐ앞 행이 R0 master 승인 ⓑ앞 행이 나중에
        재기록된 같은 축(R1 gemini). 둘 다 종전 판에서 `stopped_stagnation` 이 나왔다.
        """
        o = self.orc
        base, ln = [], 21
        for rnd in (1, 2):
            for ev in ("gemini", "codex", "machine"):
                base.append({"round": rnd, "evaluator": ev, "score": "-",
                             "verdict": "PASS(exit 0)" if ev == "machine" else "ACCEPT",
                             "_line": ln})
                ln += 1
        dmg = {"damaged": 1, "lines": [20], "last_damaged_line": 20, "unreadable": ""}
        ok = {"ok": True, "verdict": "ACCEPT", "severities": ["minor"]}
        evidence = {(r, e): ok for r in (1, 2) for e in ("gemini", "codex")}
        for label, early in (
                ("R0 master 승인", {"round": 0, "evaluator": "master", "score": "-",
                                    "verdict": "approve", "_line": 19}),
                ("재기록된 R1 gemini", {"round": 1, "evaluator": "gemini", "score": "-",
                                        "verdict": "ACCEPT", "_line": 19})):
            rows = [early] + base
            self.assertEqual(o.round_stop_reason(rows, evidence)[0], "stopped_stagnation",
                             "양성 대조 불성립(%s)" % label)
            reason, why = o.round_stop_reason(rows, evidence, row_damage=dmg)
            self.assertNotEqual(reason, "stopped_stagnation",
                                "%s 한 줄이 사라진 master 축을 면제했다: %r" % (label, why))
            self.assertTrue(o.damage_blocks_round(rows, dmg, 1),
                            "%s: 라운드 1 의 사라진 축을 통과시켰다" % label)


class Wp6TriageEmptyLockReclaim(unittest.TestCase):
    """**빈** 고아 잠금은 회수된다 — X-1 수리가 만든 영구 교착 회귀 (2026-09-08).

    `javis_orchestra` · `javis_rsi` 의 동명 클래스는 의도적 중복이므로 **둘 다** 잰다.
    """

    def setUp(self):
        self.root = tempfile.mkdtemp()
        sys.path.insert(0, BIN)
        import javis_orchestra
        import javis_rsi
        self.mods = (javis_orchestra, javis_rsi)

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_aged_empty_lock_dir_is_reclaimed(self):
        for mod in self.mods:
            base = os.path.join(self.root, "%s-base" % mod.__name__)
            lock = base + ".lock"
            os.mkdir(lock)
            old = time.time() - 4000
            os.utime(lock, (old, old))
            with mod._best_effort_lock(base, wait=2.0, stale=300.0) as lk:
                self.assertTrue(lk.held, "%s: 빈 고아 잠금을 회수하지 못했다" % mod.__name__)
                self.assertFalse(lk.blocked, "%s: 영구 교착(blocked)" % mod.__name__)

    def test_fresh_empty_lock_dir_is_still_respected(self):
        """음성 대조: **갓 만들어진** 빈 잠금은 owner 를 쓰는 중인 살아 있는 소유자다."""
        for mod in self.mods:
            base = os.path.join(self.root, "%s-fresh" % mod.__name__)
            os.mkdir(base + ".lock")
            with mod._best_effort_lock(base, wait=0.3, stale=300.0) as lk:
                self.assertFalse(lk.held, "%s: 살아 있는 소유자의 잠금을 강탈했다" % mod.__name__)
                self.assertTrue(lk.blocked, "%s: 경합을 blocked 로 표기하지 않았다" % mod.__name__)

    def test_ctrl_c_between_mkdir_and_owner_leaves_a_reclaimable_lock(self):
        """도달 경로: `KeyboardInterrupt` 는 `except OSError` 에 걸리지 않아 `__enter__` 밖으로
        빠져나가고 `__exit__` 도 돌지 않는다 — **빈** 잠금이 남는다.

        ★주입 지점은 `os.open`(소유권 배타 게시 · 성찰 R4 N6)이다. 종전에는 `builtins.open` 을
        가로챘는데, 게시가 `O_EXCL` 로 바뀌면서 그 지점은 **되읽기**가 되어 owner 파일이 이미
        생긴 뒤였다(빈 잠금이 아니라 owner 있는 잠금이 남아 전제가 깨졌다)."""
        mod = self.mods[0]
        base = os.path.join(self.root, "ctrlc")
        lock = base + ".lock"
        real_os_open, hit = os.open, []

        def interrupting_open(f, *a, **kw):
            if not hit and str(f) == os.path.join(lock, "owner"):
                hit.append(True)
                raise KeyboardInterrupt("Ctrl-C")
            return real_os_open(f, *a, **kw)

        os.open = interrupting_open
        try:
            with self.assertRaises(KeyboardInterrupt):
                with mod._best_effort_lock(base, wait=1.0, stale=300.0):
                    pass
        finally:
            os.open = real_os_open
        self.assertTrue(hit, "전제 불성립: owner 게시 지점에 도달하지 못했다")
        self.assertTrue(os.path.isdir(lock) and not os.listdir(lock),
                        "전제 불성립: 빈 잠금이 남지 않았다")
        old = time.time() - 4000
        os.utime(lock, (old, old))
        with mod._best_effort_lock(base, wait=2.0, stale=300.0) as lk:
            self.assertTrue(lk.held, "Ctrl-C 가 남긴 빈 잠금이 영구 교착이 됐다")


class Wp6TriageRsiProjectScope(unittest.TestCase):
    """프로젝트별 RSI 라운드가 **공용 다이제스트 키**로 뭉치지 않는다 (독립 재유도)."""

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.pack = os.path.join(self.root, "pack")
        os.makedirs(self.pack)
        self.env = dict(os.environ)
        self.env["CYS_PACK_DIR"] = self.pack
        for k in ("CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
            self.env.pop(k, None)
        self.env["CYS_RSI_CEILING_FLATS"] = "2"
        self.env["CYS_RSI_MAX_ROUNDS"] = "99"
        self.proj = {}
        for name in ("A", "B"):
            d = os.path.join(self.root, name)
            os.makedirs(d)
            subprocess.run(["git", "init", "-q", "."], cwd=d, check=True)
            for kv in (("user.email", "t@example.invalid"), ("user.name", "t")):
                subprocess.run(["git", "config"] + list(kv), cwd=d, check=True)
            open(os.path.join(d, "a"), "w").write("x\n")
            subprocess.run(["git", "add", "a"], cwd=d, check=True)
            subprocess.run(["git", "commit", "-qm", "init"], cwd=d, check=True)
            self.proj[name] = d

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def queue(self):
        p = os.path.join(self.pack, "round", "learn", "digest_queue.jsonl")
        if not os.path.isfile(p):
            return []
        with open(p, encoding="utf-8") as f:
            return [json.loads(l) for l in f if l.strip()]

    def hit_ceiling(self, name):
        d = self.proj[name]
        for args in (["checkpoint", "--round", "r1", "--score", "5.0"],
                     ["progress", "--round", "r1", "--score", "5.0"],
                     ["progress", "--round", "r1", "--score", "5.0"]):
            r = subprocess.run([PY, RSI] + args, cwd=d, capture_output=True, text=True,
                               timeout=120, env=self.env)
            self.assertEqual(r.returncode, 0, (name, args, r.stderr))

    def test_other_projects_round_does_not_suppress_this_ones_recommendation(self):
        """(reviewer-codex 잔여 major) RSI 이력은 프로젝트별(`cwd/_round/rsi`)인데 다이제스트
        큐는 공용 팩(`<팩>/round/learn`)이고 멱등키는 `rsi.ceiling:<round>` 로 **프로젝트
        신원이 없다**. 그래서 프로젝트 A 의 `r1` 추천이 큐에 남아 있으면 프로젝트 B 의 `r1`
        추천이 '이미 나갔다'로 눌리고, B 의 ledger 에는 나간 적 없는 추천의 `backfilled` 래치가
        영속한다(큐가 비워져도 재추천되지 않는다).
        """
        self.hit_ceiling("A")
        self.assertEqual(len(self.queue()), 1, "전제 불성립: A 의 추천이 적재되지 않았다")
        self.hit_ceiling("B")
        self.assertEqual(len(self.queue()), 2,
                         "프로젝트 B 의 추천이 A 의 공용 키에 눌렸다: %r" % (self.queue(),))
        led = os.path.join(self.proj["B"], "_round", "rsi", "ledger.jsonl")
        with open(led, encoding="utf-8") as f:
            recs = [json.loads(l) for l in f if l.strip()]
        self.assertFalse([e for e in recs if e.get("event") == "ceiling_recommend"
                          and e.get("backfilled")],
                         "나간 적 없는 추천을 B 의 ledger 에 backfill 했다: %r" % recs)


if __name__ == "__main__":
    unittest.main(verbosity=2)
