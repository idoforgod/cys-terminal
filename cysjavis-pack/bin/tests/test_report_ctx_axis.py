#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_report_ctx_axis.py — 60% 임계는 실측 축으로 판정한다(자기보고는 신선할 때만 보조). ★WP6-2.

무엇을 막는가: 60% 임계를 읽는 자리(텍스트 보고 `render_text` · 게이트 `extract_warnings`)가 자기보고
`context_pct` 만 보거나 결측을 0 으로 접어, 실측이 60% 를 넘어도 자기보고가 낮으면 놓치고 신고 없는
좌석(부팅 직후·죽은 좌석·agy)을 "0%" 로 위장해 목록에서 **조용히 빼던** 결함.

계약(정본 = javis_hud_bridge.pick_ctx 와 같은 규칙 · 정의는 javis_report.pick_node_ctx 한 벌):
  ① 실측(usage_ctx_pct) > 자기보고(context_pct) — 자기보고는 status_age_secs ≤ 300 일 때만
  ② 결측은 None 이지 0 이 아니다 — 못 재면 목록에서 **빠진다**(경보 없음), 0% 로 위장하지 않는다
  ③ live_nodes 엔트리에 실측 축 `usage_ctx_pct` 가 실린다(가산 · 기존 키 무변)
  ④ 두 소비자(render_text · gate)가 같은 헬퍼로 판정하고 경보 문구에 출처(실측/추정)를 찍는다
  ⑤ 게이트 정규화 블랙리스트에 `usage_ctx_pct` 가 있다(시간파생 — 없으면 매 주기 DELTA 폭주)
  ⑥ (0.14.31 후속) 사망 게이트 — 데몬 수집기·워치독은 exited 좌석을 건너뛰어 usage·agent_alive 가 동결된다.
     `exited is True` 또는 `agent_alive is False` 면 (None, "dead")(동결 실측으로 60% 를 울리지 않는다) ·
     None/False/True 의 나머지 조합은 게이트를 열지 않는다 · live_nodes 엔트리에 `exited` 가 가산된다
     (org.status bool 그대로 · 구버전 키 없음 = None)

밀폐: 모듈은 importlib 로 리포 팩에서 직접 로드, 팩 경로·상태 경로는 임시 디렉터리로 고정(라이브
무접촉). status JSON 은 dict 로 **주입**한다 — `cys`·`cysd` 를 부르지 않는다(형제 test_report_ghost 방식).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REPORT-CTX-AXIS-OK.
실행: python3 cysjavis-pack/bin/tests/test_report_ctx_axis.py
"""
import importlib.util
import os
from pathlib import Path
import re
import sys
import tempfile
from unittest.mock import patch

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))                        # …/bin/tests
BIN = os.path.dirname(HERE)                                              # cysjavis-pack/bin

# ── 밀폐 env — 형제 TempPackCase.ENV_KEYS 와 같은 키 집합을 비우고 임시 팩으로 고정 ──
_ROOT = tempfile.mkdtemp(prefix="report-ctx-axis-")
for _k in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR",
           "CYS_TODO_DIRS", "CYS_TODO_STALE_DAYS"):
    os.environ.pop(_k, None)
os.environ["CYS_PACK_DIR"] = os.path.join(_ROOT, "pack")
os.environ["CYS_STATE_DIR"] = os.path.join(_ROOT, "state")
os.makedirs(os.path.join(_ROOT, "pack", "round"))
os.makedirs(os.path.join(_ROOT, "state"))


def _load(name, fname):
    spec = importlib.util.spec_from_file_location(name, os.path.join(BIN, fname))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


RP = _load("javis_report_ctx_axis", "javis_report.py")
RG = _load("javis_report_gate_ctx_axis", "javis_report_gate.py")

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if (detail and not cond) else ""))
    if not cond:
        fails.append(name)


pick_node_ctx = RP.pick_node_ctx

# ── ① 헬퍼 계약(사양 4단언 중 3) ──
check("실측이 자기보고를 덮는다",
      pick_node_ctx({"usage_ctx_pct": 78, "context_pct": 95, "status_age_secs": 1}) == (78, "measured"))
check("신고 부재는 0 이 아니라 None",
      pick_node_ctx({"usage_ctx_pct": None, "context_pct": None, "status_age_secs": None}) == (None, "none"))
check("낡은 자기보고는 판정 불가",
      pick_node_ctx({"usage_ctx_pct": None, "context_pct": 90, "status_age_secs": 301}) == (None, "none"))
check("신선한 자기보고는 보조 축으로 쓴다",
      pick_node_ctx({"usage_ctx_pct": None, "context_pct": 90, "status_age_secs": 300}) == (90, "self"))
check("나이를 모르는 자기보고는 '방금'이 아니라 '모른다'",
      pick_node_ctx({"usage_ctx_pct": None, "context_pct": 90}) == (None, "none"))
check("실측이 float 여도 실측 축이다",
      pick_node_ctx({"usage_ctx_pct": 61.5, "context_pct": 20, "status_age_secs": 1}) == (61.5, "measured"))
check("회귀 박제 — 결측 pct 는 0 과 같지 않다(옛 `?? 0` 형태 금지)",
      pick_node_ctx({}) [0] is None and pick_node_ctx({})[0] != 0)
# ⑥ 사망 게이트 — 두 축 중 하나라도 사망이면 동결 실측은 값이 아니다.
check("pane 종료(exited=True · agent_alive 는 동결 True)의 동결 실측은 판정 불가",
      pick_node_ctx({"usage_ctx_pct": 82, "exited": True, "agent_alive": True}) == (None, "dead"))
check("에이전트 사망(agent_alive=False · pane 은 살아 exited=False)의 동결 실측은 판정 불가",
      pick_node_ctx({"usage_ctx_pct": 82, "exited": False, "agent_alive": False,
                     "context_pct": 70, "status_age_secs": 1}) == (None, "dead"))
check("음성 대조 — None(미관측·구버전 키 없음)은 게이트를 열지 않는다",
      pick_node_ctx({"usage_ctx_pct": 82, "exited": None, "agent_alive": None}) == (82, "measured")
      and pick_node_ctx({"usage_ctx_pct": 82}) == (82, "measured"))
check("음성 대조 — exited=False · agent_alive=True 는 산 좌석이다",
      pick_node_ctx({"usage_ctx_pct": 82, "exited": False, "agent_alive": True}) == (82, "measured"))
REPO = Path(__file__).resolve().parents[3]
age_literals = {}
for relative, pattern in (
        ("src/bin/cys.rs", r"^const CTX_SELF_REPORT_MAX_AGE_SECS:\s*u64\s*=\s*(\d+)\s*;"),
        ("ui/src/ctxpick.ts", r"^export const CTX_SELF_REPORT_MAX_AGE_SECS\s*=\s*(\d+)\s*;"),
        ("cysjavis-pack/bin/javis_report.py", r"^CTX_SELF_REPORT_MAX_AGE_S\s*=\s*(\d+)\s*$"),
        ("cysjavis-pack/bin/javis_hud_bridge.py", r"^CTX_SELF_REPORT_MAX_AGE_S\s*=\s*(\d+)\s*$")):
    matches = re.findall(pattern, (REPO / relative).read_text(encoding="utf-8"), re.MULTILINE)
    check("신선도 상수 리터럴 추출: " + relative, len(matches) == 1, repr(matches))
    if len(matches) == 1:
        age_literals[relative] = int(matches[0])
check("Rust·TS·보고·HUD 신선도 상수가 같다(네 파일 동기 핀)",
      len(age_literals) == 4 and len(set(age_literals.values())) == 1, repr(age_literals))

# ── ② live_nodes 엔트리 — status JSON 주입(cys 실호출 없음) ──
STATUS = {
    "surfaces": [
        # 실측 78 · 자기보고 95(신선) → 실측이 이긴다(78 실측)
        {"role": "worker", "cwd": None, "idle_secs": 0, "agent_alive": True,
         "status": {"state": "working", "context_pct": 95, "task": "t", "age_secs": 1},
         "usage": {"agent": "claude", "ctx_pct": 78, "ctx_tokens": 156000, "ctx_window": 200000}},
        # 실측 없음 · 자기보고 90 이 301초 낡음 → 판정 불가(목록에서 빠짐)
        {"role": "cso", "cwd": None, "idle_secs": 0, "agent_alive": True,
         "status": {"state": "working", "context_pct": 90, "task": None, "age_secs": 301},
         "usage": None},
        # 실측 없음(agy) · 자기보고 없음 → None(0 으로 위장 금지)
        {"role": "agy", "cwd": None, "idle_secs": 0, "agent_alive": True,
         "status": {}, "usage": None},
        # 실측 낡아 None(usage.rs :stale) · 자기보고 65 신선 → 추정 65
        {"role": "reviewer", "cwd": None, "idle_secs": 0, "agent_alive": True,
         "status": {"state": "working", "context_pct": 65, "task": None, "age_secs": 10},
         "usage": {"agent": "claude", "ctx_pct": None, "ctx_tokens": None, "ctx_window": None}},
        # ⑥ pane 종료 — 워치독이 exited 좌석을 건너뛰어 agent_alive 는 True 로 동결, usage 도 82 로 동결
        {"role": "ghost-exited", "cwd": None, "idle_secs": 0, "agent_alive": True, "exited": True,
         "status": {}, "usage": {"agent": "claude", "ctx_pct": 82, "ctx_tokens": 164000, "ctx_window": 200000}},
        # ⑥ 에이전트 사망 — pane 은 살아 exited=False, 워치독이 agent_alive=False 확정, usage 82 동결
        {"role": "ghost-dead", "cwd": None, "idle_secs": 0, "agent_alive": False, "exited": False,
         "status": {}, "usage": {"agent": "claude", "ctx_pct": 82, "ctx_tokens": 164000, "ctx_window": 200000}},
    ],
    "feed": {"pending": 0}, "paused": False,
}
rep = RP.build_report(STATUS, [], now=1_700_000_000.0, sampled_at=1_700_000_000.0)
by_role = {n["role"]: n for n in rep["live_nodes"]}

check("live_nodes 엔트리에 실측 축이 실린다",
      "usage_ctx_pct" in rep["live_nodes"][0], str(rep["live_nodes"][:1]))
check("실측 축 값이 usage.ctx_pct 그대로다",
      by_role["worker"]["usage_ctx_pct"] == 78 and by_role["reviewer"]["usage_ctx_pct"] is None,
      str({r: n.get("usage_ctx_pct") for r, n in by_role.items()}))
check("기존 키는 삭제·변경되지 않았다(context_pct · status_age_secs · usage_ctx_tokens)",
      by_role["worker"]["context_pct"] == 95 and by_role["worker"]["status_age_secs"] == 1
      and by_role["worker"]["usage_ctx_tokens"] == 156000
      and all(k in by_role["agy"] for k in ("role", "state", "context_pct", "idle_secs", "agent_alive",
                                              "status_age_secs", "usage_ctx_tokens")),
      str(by_role["worker"]))
check("미측정은 None 이다(agy — 0 으로 접지 않는다)",
      by_role["agy"]["usage_ctx_pct"] is None and by_role["agy"]["context_pct"] is None
      and pick_node_ctx(by_role["agy"]) == (None, "none"))
# ⑥ live_nodes 에 `exited` 가 가산된다 — org.status bool 그대로, 구버전(키 없음)은 None.
check("live_nodes 엔트리에 exited 가 실린다(bool 그대로 · 키 없음 = None)",
      all("exited" in n for n in rep["live_nodes"])
      and by_role["ghost-exited"]["exited"] is True and by_role["ghost-dead"]["exited"] is False
      and by_role["worker"]["exited"] is None,
      str({r: n.get("exited") for r, n in by_role.items()}))
check("죽은 좌석의 live_nodes 엔트리는 판정 불가(dead) — 동결 실측 82 를 값으로 내지 않는다",
      pick_node_ctx(by_role["ghost-exited"]) == (None, "dead")
      and pick_node_ctx(by_role["ghost-dead"]) == (None, "dead")
      and by_role["ghost-exited"]["usage_ctx_pct"] == 82,
      str((by_role["ghost-exited"], by_role["ghost-dead"])))

# ── ③ 소비자 1 — 텍스트 보고 ──
text = RP.render_text(rep)
check("텍스트 보고: 실측 60%+ 는 자기보고 값이 아니라 실측 값·출처로 찍힌다",
      "worker(78% 실측)" in text and "worker(95%" not in text, text)
check("텍스트 보고: 신선한 자기보고는 '추정' 으로 찍힌다",
      "reviewer(65% 추정)" in text, text)
check("텍스트 보고: 낡은 자기보고·미측정은 60% 목록에 없다",
      "cso(" not in text and "agy(" not in text, text)
check("텍스트 보고: 죽은 좌석의 동결 실측 82 는 60% 목록에 없다",
      "ghost-exited(" not in text and "ghost-dead(" not in text, text)

# ── ④ 소비자 2 — 게이트 ──
check("게이트가 산출기의 헬퍼를 import 했다(중복 정의 아님)",
      RG._REPORT_IMPORT_ERR is None and RG._pick_node_ctx is not None,
      "err=%r" % RG._REPORT_IMPORT_ERR)
warns = RG.extract_warnings(rep)
ctx_warns = [w for w in warns if w.get("task") == "gate-context"]
check("게이트: gate-context 경보 1건", len(ctx_warns) == 1, str(warns))
body = ctx_warns[0]["wake_body"] if ctx_warns else ""
check("게이트: 경보 문구에 실측 값·출처가 찍힌다",
      "worker(78% 실측)" in body and "reviewer(65% 추정)" in body and "worker(95%" not in body, body)
check("게이트: 낡은 자기보고(cso)·미측정(agy)은 경보에 없다",
      "cso(" not in body and "agy(" not in body, body)
check("게이트: 죽은 좌석(exited / agent_alive=False)의 동결 실측 82 는 경보에 없다",
      "ghost-exited(" not in body and "ghost-dead(" not in body, body)
check("게이트: idem 키는 종전 형태(role 나열)를 유지한다",
      ctx_warns and ctx_warns[0]["idem"] == "gate-context-worker,reviewer", str(ctx_warns))
# 옛 형태 회귀 박제 — 자기보고만 60%+ 이고 낡았으면 게이트는 울리지 않는다(경보 감소는 의도).
rep_stale = dict(rep, live_nodes=[by_role["cso"]], role_measurements=[])
check("게이트: 낡은 자기보고 단독으로는 60% 경보가 없다(의도한 감소)",
      not [w for w in RG.extract_warnings(rep_stale) if w.get("task") == "gate-context"])

# 측정 수단 부재도 실제 Gate 주기를 통과해 대장에 남아야 한다(라이브 Runner 금지).
from test_report_gate import FakeRunner, ledger_entries

with patch.object(RG, "_pick_node_ctx", None), \
        patch.object(RG, "_REPORT_IMPORT_ERR", "boom"), \
        patch.object(RG, "foreign_daemon_verdict", return_value=None), \
        tempfile.TemporaryDirectory() as missing_state:
    check("게이트: 보고 모듈 부재면 gate-context 경보 0건",
          not [w for w in RG.extract_warnings(rep) if w.get("task") == "gate-context"])
    missing_gate = RG.Gate(missing_state, FakeRunner(rep=rep), now_epoch_fn=lambda: 1_000_000.0)
    for phase in ("BASELINE", "후속 주기"):
        check("게이트: 보고 모듈 부재 " + phase + "도 exit 0", missing_gate.run() == 0)
        entry = ledger_entries(missing_state)[-1]
        check("게이트: 보고 모듈 부재 " + phase + " reasons에 report_module_missing",
              "report_module_missing" in entry["reasons"], str(entry))

# ── ⑤ 정규화 블랙리스트 — 실측 % 는 시간파생 ──
check("BLACKLIST_KEYS 에 usage_ctx_pct 가 있다(없으면 매 주기 DELTA 폭주)",
      "usage_ctx_pct" in RG.BLACKLIST_KEYS)
rep2 = RP.build_report(STATUS, [], now=1_700_000_000.0, sampled_at=1_700_000_000.0)
rep2["live_nodes"][0]["usage_ctx_pct"] = 79
check("usage_ctx_pct 만 바뀐 보고는 정규화 스냅샷이 같다",
      RG.normalize(rep) == RG.normalize(rep2))

if fails:
    print("FAILED %d: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("REPORT-CTX-AXIS-OK")
