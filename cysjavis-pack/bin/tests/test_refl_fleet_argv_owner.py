#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_fleet_argv_owner.py — 성찰 R4 N12: 정상 런처 옵션이 함대 CPU 계상에서 빠지지 않는다.

무엇을 막는가: `fleet_cpu_ratio` 는 함대 프로세스의 %CPU 합이고, 그 소유권은 ps 명령줄을
언랩해서 정한다. 표에 없는 옵션을 만나면 언랩을 **포기**하는데(그 자체는 옳다 — 미상 옵션의
**값**을 실행 주체로 오인하면 B2 오탐이 재발한다), 포기의 대가는 **미계상**이다: 그 행의 CPU 가
합에서 빠져 축이 조용히 얇아지고 hard 가 걸려야 할 포화에서 걸리지 않는다.
  ⓐ **반복 단축 옵션** — `uv -vv run serena` 는 흔한 정상 형상인데 표는 `-v` 만 알아서 `-vv` 가
     통째로 `unknown` 이었다. 반복은 같은 옵션의 **강도**라 의미가 하나뿐이므로 안전하게 인식할
     수 있다(런처 표는 이름 단위이고 짧은 이름의 의미는 런처마다 달라 클러스터 일반화는 금물이다).
  ⓑ **미등재 불리언** — `node --use-system-ca /x/bin/codex` 처럼 값을 안 먹는 Node 옵션 뒤의 우리
     CLI 가 미계상됐다. `node --help`(v24.20.0) 실측에서 값 표기가 없는 이름만 편입한다.

★이 검체의 절반은 **음성 대조**다(정밀도 우선 · B2 오탐 재발 금지): 옵션 **값에만** CLI 이름이
  있는 형상과, 값을 먹거나 실행 자리를 옮기는 이름은 여전히 미계상이어야 한다.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-FLEET-ARGV-OWNER-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_fleet_argv_owner.py
"""
import os
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)

import javis_resource_gate as G          # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def owner(cmd):
    return G._fleet_owner(cmd)


# ── ① 양성군: 반복 단축 옵션(N12 ⓐ) ─────────────────────────────────────────────────
POS_REPEAT = [
    ("uv -vv run serena", "serena"),
    ("/Users/user/.local/bin/uv -vvv run --active serena start-mcp-server", "serena"),
    ("uv -qq run serena", "serena"),
    ("npx -qq codex", "codex"),
    ("npm -dd exec codex", "codex"),
]
for cmd, want in POS_REPEAT:
    check("1 반복 단축 옵션 뒤의 CLI 가 소유자다 — %r" % cmd, owner(cmd) == want,
          "got=%r want=%r" % (owner(cmd), want))

# 음성 대조: 반복이 아닌 **혼합 클러스터**는 여전히 포기한다(`-p` 는 uv 에서 값을 먹는다).
NEG_CLUSTER = ["uv -pv run serena", "uv -vp run serena", "npx -py codex"]
for cmd in NEG_CLUSTER:
    check("1n 음성 대조 — 혼합 클러스터는 포기한다 %r" % cmd, owner(cmd) is None,
          "got=%r" % (owner(cmd),))
# 음성 대조: 값 옵션 문자의 반복은 불리언이 아니다(`uv -pp …` 의 뒤는 값일 수 있다).
check("1o 음성 대조 — 값 문자의 반복은 불리언이 아니다", owner("uv -pp run serena") is None,
      repr(owner("uv -pp run serena")))

# ── ② 양성군: 새로 편입한 Node 불리언(N12 ⓑ) ────────────────────────────────────────
NEW_BOOLS = ["--use-system-ca", "--trace-promises", "--permission", "--entry-url",
             "--no-experimental-websocket", "--experimental-eventsource", "--v8-options",
             "--report-exclude-env", "--test-randomize", "--interpreted-frames-native-stack"]
for flag in NEW_BOOLS:
    cmd = "node %s /Users/user/.local/bin/codex" % flag
    check("2 미등재 불리언 뒤의 CLI 가 소유자다 — %s" % flag, owner(cmd) == "codex",
          "got=%r" % (owner(cmd),))
check("2b 여러 개 연속도 같다",
      owner("node --trace-promises --permission --use-system-ca "
            "/Users/user/.local/bin/codex") == "codex")

# ── ③ 음성군: 값·모드 이름은 편입하지 않았다(B2 오탐 재발 금지) ────────────────────
NEG_VALUE = [
    # 값 표기가 붙은 별칭 — 편입 대상이 아니다(`--inspect-port=…`)
    "node --debug-port 9229 /Users/user/.local/bin/codex",
    # 뒤 토큰이 **자기 입력 파일**이다(V8 프로파일 로그)
    "node --prof-process /tmp/codex",
    # 값 표기가 붙은 별칭(`--experimental-config-file=…`)
    "node --experimental-default-config-file /Users/user/.local/bin/codex",
]
for cmd in NEG_VALUE:
    check("3 음성 대조 — 값·모드 이름은 여전히 포기 %r" % cmd, owner(cmd) is None,
          "got=%r" % (owner(cmd),))
# ★핵심 음성군(B2 재발 핀): 옵션 **값에만** CLI 이름이 있다 — 실행 주체는 뒤의 스크립트다.
NEG_VALUE_NAME = [
    "node --cpu-prof-dir /tmp/codex /tmp/report.js",
    "node --diagnostic-dir /tmp/codex /tmp/report.js",
    "node --test-reporter-destination /tmp/serena /tmp/report.js",
    "python3 -W ignore /tmp/report.py /tmp/serena",
]
for cmd in NEG_VALUE_NAME:
    check("3n ★값에만 CLI 이름이 있는 형상은 소유자가 아니다 %r" % cmd, owner(cmd) is None,
          "got=%r" % (owner(cmd),))

# ── ④ CPU 합 — 양성군은 합에 들어가고 음성군은 안 들어간다 ──────────────────────────
lines = [
    "  101   12.5 uv -vv run serena",
    "  102    7.5 node --use-system-ca /Users/user/.local/bin/codex",
    "  103   99.0 node --cpu-prof-dir /tmp/codex /tmp/report.js",   # 음성 — 합에서 빠진다
    "  104   50.0 /bin/zsh -l",                                     # 함대 아님
]
total, why = G._fleet_cpu_percent(lines, self_pid=-1)
check("4a 합산이 성공한다", why is None, repr(why))
check("4b ★양성군 두 행만 합에 든다(12.5+7.5=20.0)", total == 20.0, repr(total))
rows, _ = G._fleet_rows(lines, self_pid=-1)
owners = sorted(r["owner"] for r in (rows or []))
check("4c 소유자 집합", owners == ["codex", "serena"], repr(owners))

print("")
if fails:
    print("FAILED %d: %s" % (len(fails), " · ".join(fails)))
    sys.exit(1)
print("ALL PASS")
print("REFL-FLEET-ARGV-OWNER-OK")
