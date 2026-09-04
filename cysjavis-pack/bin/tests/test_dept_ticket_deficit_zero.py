#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_dept_ticket_deficit_zero.py — 결손 0 부서 재기동의 CEO 티켓 요청 생략 회귀 (0.14.30 A4 #13 · PREP #13).

## 재현 대상 (2026-09-03 실측 3회: 09:28 · 10:04 · 13:10)
부서장이 재기동될 때마다 ③″ 티켓 게이트가 티켓 부재를 감지하고 **CEO 에게 발급 요청 push** 를
쐈다 — 그런데 그 시점에 의무 역할 좌석은 **전원 생존**이었다(CEO 실측: 각 회 9~10좌석 · orchestra
check READY). 티켓의 용도는 '팀 기동 권한'이고 결손 0 이면 기동할 것이 없으므로(④ 도
`if has_deficit:` 로 스폰 경로에 진입하지 않는다) 그 요청은 **CEO 큐에 쌓이는 소음**이자
오판 유인이었다.

## 이 파일이 못박는 것 (실 데몬 무접촉 — 임시 HOME + PATH 선두 `cys` 스텁)
  1) **결손 0** + 티켓 부재 → `cys send` 호출 **0건** · exit 0 · 요약이 사실을 말한다
     (solo_awakening=False · team_complete=True · marker 에 '팀 이미 완결')
  2) 판정 근거가 boot-last 원장(steps)에 남는다 — ③″ceo-ticket-request 단계에 '결손 0' 사유
  3) ★대조군: 의무 역할이 **하나라도 비면** 종전대로 요청 push 가 나간다(요청 경로 무회귀)
  4) ★측정 실패(status 파싱 불가)는 요청 생략의 근거가 되지 않는다 — 보수적으로 요청한다
     (측정 불능은 통과가 아니다)

실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_dept_ticket_deficit_zero.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 TICKET-DEFICIT-ZERO-OK.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(SELF, "..", "javis_bootstrap.py")
PY = sys.executable or "python3"
DEPT = "dept-9"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _roster(roles):
    """의무 역할 좌석 JSON — 전부 살아있는 agent 로 채운다(결손 판정 입력)."""
    return json.dumps({"surfaces": [
        {"surface_id": i + 1, "role": r, "exited": False, "agent": "claude",
         "agent_alive": True, "seat": "occupied"}
        for i, r in enumerate(roles)]}, ensure_ascii=False)


# 스텁 cys — 부서 레인에서 지정 로스터를 보고한다(base 레인은 CEO 1좌석 = 요청 수신자 생존).
_CYS = """#!/bin/sh
echo "cys $@ [sock=${CYS_SOCKET:-base}]" >> "%(t)s/calls.log"
case "$1" in
  ping) exit 0;;
  claim-role) exit 0;;
  boot) exit 0;;
  list) exit 0;;
  --version) echo 'cys 0.0.0-stub'; exit 0;;
  status)
    if [ -z "${CYS_SOCKET:-}" ]; then
      echo '{"surfaces":[{"surface_id":9,"role":"master","exited":false,"agent":"claude","agent_alive":true,"seat":"occupied"}]}'
    else
      %(dept_status)s
    fi
    exit 0;;
  send) exit 0;;
esac
exit 0
"""

# ★의무 리뷰어 **슬롯명은 설치된 CLI 감지 결과에 따라 갈린다** — agy·codex 가 잡히면
#   reviewer-gemini/reviewer-codex, 아니면 대체 슬롯 reviewer-claude-1/2 가 의무가 된다(실측:
#   격리 하네스에서 후자로 해소됐다). 테스트가 실행 기계의 CLI 설치 상태에 좌우되지 않도록
#   **두 명명을 모두** 좌석으로 채운다 — 어느 쪽이 의무로 잡히든 결손 0 이 성립한다.
FULL = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex",
        "reviewer-claude-1", "reviewer-claude-2"]


def make_env(tmp, dept_status_cmd):
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack-dept-%s" % DEPT)
    bindir = os.path.join(tmp, "stubbin")
    for d in (os.path.join(pack, "bin"), bindir):
        os.makedirs(d, exist_ok=True)

    def w(path, body, mode=0o755):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        os.chmod(path, mode)

    w(os.path.join(bindir, "cys"), _CYS % {"t": tmp, "dept_status": dept_status_cmd})
    w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
    w(os.path.join(pack, "bin", "javis_orchestra.py"), "import sys; sys.exit(0)\n", 0o644)
    w(os.path.join(pack, "bin", "cys-dept"), "#!/bin/sh\nexit 0\n")
    env = dict(os.environ)
    env.update({"HOME": home, "PATH": bindir + os.pathsep + env.get("PATH", ""),
                "CYS_PACK_DIR": pack,
                "CYS_SOCKET": os.path.join(tmp, "state", "cys-dept-%s" % DEPT, "cys.sock"),
                "CYS_SURFACE_ID": "7",
                "CYS_BOOT_CHECK_RETRIES": "2", "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                "CYS_DEPT_TICKET_WAIT_S": "3", "CYS_DEPT_TICKET_WAIT_INTERVAL_S": "1"})
    env.pop("CYS_BOOT_GATE", None)
    return env, home


def run_case(tmp, dept_status_cmd):
    env, home = make_env(tmp, dept_status_cmd)
    r = subprocess.run([PY, SCRIPT], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=180)
    log = os.path.join(tmp, "calls.log")
    calls = open(log, encoding="utf-8").read() if os.path.exists(log) else ""
    sends = [l for l in calls.splitlines() if l.startswith("cys send ")]
    try:
        summary = json.loads((r.stdout or "").strip().splitlines()[-1])
    except Exception:
        summary = {}
    boot_last = {}
    state_dir = os.path.join(home, ".cys", "state")
    for name in sorted(os.listdir(state_dir)) if os.path.isdir(state_dir) else []:
        if name.startswith("boot-last") and name.endswith(".json"):
            try:
                with open(os.path.join(state_dir, name), encoding="utf-8") as f:
                    boot_last = json.load(f)
            except Exception:
                pass
    return r, sends, summary, boot_last


# ── 1·2. 결손 0 → 요청 0건 ─────────────────────────────────────────────────────
tmp = tempfile.mkdtemp(prefix="tdz-full-")
try:
    r, sends, summary, boot_last = run_case(tmp, "echo '%s'" % _roster(FULL))
    check("1a exit 0(부트 무중단)", r.returncode == 0, "rc=%s err=%r" % (r.returncode, r.stderr[-200:]))
    check("1b ★결손 0 → cys send 호출 0건(CEO 큐 소음 차단)", not sends, repr(sends))
    check("1c 요약 solo_awakening=False(팀이 살아 있으므로 '단독 각성'이 아니다 — 거짓 플래그 0)",
          summary.get("solo_awakening") is False, json.dumps(summary, ensure_ascii=False)[:200])
    check("1d 요약 team_complete=True", summary.get("team_complete") is True,
          json.dumps(summary, ensure_ascii=False)[:200])
    check("1e marker 가 사실을 말한다('팀 이미 완결')",
          "완결" in (summary.get("marker") or ""), repr(summary.get("marker")))
    check("1f ticket_requested=False", summary.get("ticket_requested") is False)
    steps = json.dumps(boot_last.get("steps") or [], ensure_ascii=False)
    check("2 판정 근거가 boot-last 원장에 남는다(③″ceo-ticket-request 단계 실재)",
          "ceo-ticket-request" in steps, steps[:200])
finally:
    shutil.rmtree(tmp, ignore_errors=True)

# ── 3. ★대조군: 역할이 비면 종전대로 요청 ─────────────────────────────────────
tmp = tempfile.mkdtemp(prefix="tdz-partial-")
try:
    r, sends, summary, _ = run_case(tmp, "echo '%s'" % _roster(["master", "cso"]))
    check("3a exit 0(fail-open 유지)", r.returncode == 0, "rc=%s" % r.returncode)
    check("3b ★결손>0 → 요청 push 발사(요청 경로 무회귀)", bool(sends), repr(sends[:2]))
    check("3c 그 경로는 종전대로 solo_awakening=True",
          summary.get("solo_awakening") is True, json.dumps(summary, ensure_ascii=False)[:200])
finally:
    shutil.rmtree(tmp, ignore_errors=True)

# ── 4. ★측정 실패는 생략 근거가 아니다 ────────────────────────────────────────
tmp = tempfile.mkdtemp(prefix="tdz-unmeasured-")
try:
    r, sends, summary, _ = run_case(tmp, "echo 'not-json'")
    check("4a exit 0(fail-open)", r.returncode == 0, "rc=%s" % r.returncode)
    check("4b ★status 판독 불가 → 보수적으로 요청(측정 불능은 통과가 아니다)",
          bool(sends), repr(sends[:2]))
finally:
    shutil.rmtree(tmp, ignore_errors=True)

total = 11
print("\n=== %d/%d PASS ===" % (total - len(fails), total))
if fails:
    print("FAIL: %s" % fails, file=sys.stderr)
    sys.exit(1)
print("TICKET-DEFICIT-ZERO-OK")
