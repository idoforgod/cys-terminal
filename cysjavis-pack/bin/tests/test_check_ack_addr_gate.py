#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_check_ack_addr_gate.py — 부트 v2 §2-9 check 계약 검체 (0.14.30 W-A A1).

세 검체를 한 파일에 담는다(명세 §6):
  · **H-CHECK-2**        exit 2(측정불가) **보존** · `--json` 기존 13필드 **보존**
  · **H-CHECK-12-CONSUMERS**  review-prompt/round-init 은 **12 로 차단** · bootstrap ⑤ 는
                          **0 으로 접음** · gate-status 는 **무접촉**
  · **H-ADDR-1**         역할 주소 resolve 게이트(슬롯은 해소됐는데 그 이름의 비종료 행이 없다)

무엇을 막는가: ①새 exit 12 가 기존 exit 표(0/1/**2**/64)를 잠식하는 것 — 2 는 "재지 못했다"
이고 12 는 "재 보니 ACK 만 없다"라서 처방이 정반대다. ②`ack` 필드 추가가 기존 13필드를 밀어내는
것(K3 계약 추가-only). ③미측정(ack 필드 부재·null)을 '미확인'으로 접어 부트 v2 미배선 기계에서
리뷰 게이트가 전면 차단되는 것. ④12 가 **부트 완주**까지 막는 것(리뷰어 하나의 확률적 각성이
팀 기동을 인질로 잡는 형태) — 막히는 것은 리뷰 게이트뿐이어야 한다.

밀폐: `PATH` 맨 앞에 **가짜 `cys`**(sh 1파일)를 놓아 `cys status --json`·`agent-detect --json`
응답을 시험이 직접 준다 — 라이브 데몬 무접촉·스폰 0. `CYS_PACK_DIR` 는 임시 디렉터리라 라운드
장부도 격리된다. 순수 함수 축(H-ADDR-1)은 import 로 직접 호출한다.

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 CHECK-ACK-ADDR-GATE-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_check_ack_addr_gate.py
"""
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
ORC = os.path.join(BIN, "javis_orchestra.py")
BOOTSTRAP = os.path.join(BIN, "javis_bootstrap.py")
PY = sys.executable or "python3"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── 가짜 cys: status/agent-detect 응답을 파일로 주입 ────────────────────────
FAKE_CYS = '''#!/bin/sh
case "$1" in
  status)
    if [ -f "$FAKE_CYS_STATUS" ]; then cat "$FAKE_CYS_STATUS"; exit 0; fi
    exit 1 ;;
  agent-detect) cat "$FAKE_CYS_DETECT"; exit 0 ;;
  list)
    # ★역할 **주소 레지스트리**(surface.list) — `status`(프로세스 표)와 **다른 자료원**이다.
    # ★★두 실패 갈래를 **rc 로 가른다**(R2 codex #1 blocking). 종전 이 스텁은 파일이 없어도
    #   rc 0 + 무출력을 냈다 — 즉 "재지 못했다"와 "재 보니 비었다"를 **스텁 자신이 이미 섞고
    #   있었고**, 그래서 검체는 소비자의 접힘을 드러낼 수 없었다.
    #     · 파일 미지정/부재 → **rc 1 = 측정 실패**(미측정 · 게이트 불참)
    #     · 빈 파일        → **rc 0 · 행 0 = 측정된 빈 레지스트리**(결손 · 전원 미해소)
    if [ -n "${FAKE_CYS_LIST:-}" ] && [ -f "$FAKE_CYS_LIST" ]; then cat "$FAKE_CYS_LIST"; exit 0; fi
    exit 1 ;;
  ping) exit 0 ;;
  *) exit 0 ;;
esac
'''
DETECT = {"agents": {"gemini": {"installed": True, "reason": "fake"},
                     "codex": {"installed": True, "reason": "fake"},
                     "grok": {"installed": False, "reason": "fake"},
                     "claude": {"installed": True, "reason": "fake"}}}


def surfaces(ack=None, roles=("cso", "worker", "reviewer-gemini", "reviewer-codex")):
    out = []
    for r in roles:
        row = {"role": r, "exited": False, "agent_alive": True,
               "status": {"state": "working", "age_secs": 5}}
        if ack is not None and r.startswith("reviewer-"):
            row["ack_nonce_ok"] = ack
        out.append(row)
    return out


def write_list(path, roles):
    """`cys list` 출력 모사 — `javis_boot_node.cys_list_rows` 가 파싱하는 탭 구분 행."""
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        for i, r in enumerate(roles, start=10):
            f.write("surface:%d\trole=%s\tpid=%d\texited=false\n" % (i, r, 1000 + i))


def write_status(path, payload):
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(payload, f)


def run(args, env, cwd=None, script=None):
    cmd = [PY, script or ORC] + args
    return subprocess.run(cmd, capture_output=True, text=True, timeout=120, env=env, cwd=cwd)


root = tempfile.mkdtemp()
try:
    binp = os.path.join(root, "fakebin")
    pack = os.path.join(root, "pack")
    os.makedirs(binp)
    os.makedirs(pack)
    fake = os.path.join(binp, "cys")
    with io.open(fake, "w", encoding="utf-8", newline="\n") as f:
        f.write(FAKE_CYS)
    os.chmod(fake, 0o755)
    det = os.path.join(root, "detect.json")
    write_status(det, DETECT)
    st = os.path.join(root, "status.json")

    base = dict(os.environ)
    for k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR", "CYS_BOOT_GATES",
              "CYS_AWAKE_AXIS"):
        base.pop(k, None)
    base["PATH"] = binp + os.pathsep + base.get("PATH", "")
    base["CYS_PACK_DIR"] = pack
    base["JAVIS_ROOT"] = pack
    base["PYTHONPATH"] = BIN + os.pathsep + base.get("PYTHONPATH", "")
    base["FAKE_CYS_DETECT"] = det
    base["FAKE_CYS_STATUS"] = st

    # ─────────────── ① H-CHECK-2 — exit 2 보존 · 13필드 보존 ───────────────
    env_nostatus = dict(base)
    env_nostatus["FAKE_CYS_STATUS"] = os.path.join(root, "nope.json")   # 파일 없음 → cys status rc 1
    r = run(["check"], env_nostatus)
    check("1a exit 2 보존(산문 경로)", r.returncode == 2, repr((r.returncode, r.stdout[-160:])))
    r = run(["check", "--json"], env_nostatus)
    check("1b exit 2 보존(--json 경로)", r.returncode == 2, repr(r.returncode))
    try:
        pl2 = json.loads(r.stdout.strip().splitlines()[-1])
    except Exception as e:
        pl2 = {}
        check("1c 판정불가도 JSON 1줄", False, repr(e))
    BASE13 = {"schema", "exit", "axis", "ready", "ready_missing", "gated", "awake",
              "awake_pending", "required", "verdicts", "roster", "optional", "why"}
    check("1c 판정불가 payload 13필드 보존", BASE13 <= set(pl2), repr(sorted(set(pl2))))
    check("1d 판정불가 exit 필드 = 2", pl2.get("exit") == 2, repr(pl2.get("exit")))
    check("1e 판정불가 ack = null(미측정)", "ack" in pl2 and pl2["ack"] is None, repr(pl2.get("ack")))
    check("1f 판정불가는 ready 를 false 로 두지 않는다(측정 없음 ≠ 측정 결과)",
          pl2.get("ready") is None, repr(pl2.get("ready")))
    # ─────────────── ①-b exit 64 = **사용 오류**(측정불가 2 와 분리 · R1 codex #3) ───────────────
    for _args, _label in ((["--no-such-flag"], "최상위 오타"),
                          (["check", "--no-such-flag"], "서브커맨드 오타"),
                          (["nosuchcmd"], "미지 서브커맨드"),
                          (["review-prompt"], "필수 인자 누락")):
        _rr = run(_args, base)
        check("1g %s → exit 64" % _label, _rr.returncode == 64,
              repr((_rr.returncode, _rr.stderr[-120:])))
    check("1h `--help` 는 0 유지(사용 오류가 아니다)", run(["--help"], base).returncode == 0)
    check("1i 64 와 2 는 **다른 값**이다(합치면 오타에 `cys ping` 처방이 붙는다)",
          run(["check"], env_nostatus).returncode == 2
          and run(["nosuchcmd"], base).returncode == 64)


    # ─────────────── ② 정상(구 데몬 = boot_v2_enabled 없음) → exit 0 · 축 off ───────────────
    write_status(st, {"surfaces": surfaces()})
    r = run(["check", "--json"], base)
    pl = json.loads(r.stdout.strip().splitlines()[-1])
    check("2a 구 데몬은 exit 0(12 유출 없음)", r.returncode == 0, repr((r.returncode, r.stderr[-200:])))
    check("2b 13필드 보존 + 추가 필드는 ack 하나뿐",
          BASE13 <= set(pl) and set(pl) - BASE13 == {"ack"}, repr(sorted(set(pl) - BASE13)))
    check("2c 축 off 면 ack 는 전부 null(빈 목록 = 거짓 초록 금지)",
          pl["ack"] == {"axis": False, "pending": None, "ok": None, "unmeasured": None},
          repr(pl["ack"]))
    rp = run(["check"], base)
    check("2d 산문 경로도 exit 0 · READY 문안 보존",
          rp.returncode == 0 and "LLM orchestrating READY" in rp.stdout, repr(rp.stdout[-200:]))
    check("2e 축 off 에서 ACK 관련 산문 무출력(오늘 기계 출력 무변경)",
          "ACK" not in rp.stdout, repr([l for l in rp.stdout.splitlines() if "ACK" in l]))

    # ─────────────── ③ 축 on · ack 필드 부재 → 미측정(12 아님) ───────────────
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces()})
    r = run(["check", "--json"], base)
    pl = json.loads(r.stdout.strip().splitlines()[-1])
    check("3a 부분 배포(ack 필드 부재)는 12 가 아니다", r.returncode == 0, repr(r.returncode))
    check("3b 미측정 역할이 unmeasured 로 분리",
          pl["ack"]["axis"] is True and pl["ack"]["pending"] == []
          and sorted(pl["ack"]["unmeasured"]) == ["reviewer-codex", "reviewer-gemini"],
          repr(pl["ack"]))
    rp = run(["check"], base)
    check("3c 미측정은 고지된다(조용한 초록 금지)", "미측정" in rp.stdout, repr(rp.stdout[-300:]))
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=None)})

    # ─────────────── ④ 축 on · ack=false → exit 12 ───────────────
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=False)})
    r = run(["check", "--json"], base)
    pl = json.loads(r.stdout.strip().splitlines()[-1])
    check("4a check exit 12", r.returncode == 12, repr((r.returncode, r.stderr[-200:])))
    check("4b payload.exit == 12", pl.get("exit") == 12, repr(pl.get("exit")))
    check("4c ack.pending = 리뷰어 2종 · ready 는 그대로 true",
          sorted(pl["ack"]["pending"]) == ["reviewer-codex", "reviewer-gemini"]
          and pl["ready"] is True and pl["ready_missing"] == [], repr((pl["ack"], pl["ready"])))
    rp = run(["check"], base)
    check("4d 처방은 실재 명령(없는 --verify 금지)",
          "--verify" not in rp.stdout and "boot-reviewers" in rp.stdout, repr(rp.stdout[-300:]))
    check("4e 부트 완주는 막지 않는다고 명시", "0 으로 접는다" in rp.stdout or "0 취급" in rp.stdout,
          repr(rp.stdout[-300:]))

    # ⑤ ack=true → 0
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=True)})
    r = run(["check", "--json"], base)
    pl = json.loads(r.stdout.strip().splitlines()[-1])
    check("5 ack 확인되면 exit 0", r.returncode == 0 and pl["ack"]["pending"] == [],
          repr((r.returncode, pl["ack"])))

    # ⑥ 마스터 킬스위치로 되돌아간다
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=False)})
    env_off = dict(base); env_off["CYS_BOOT_GATES"] = "0"
    r = run(["check"], env_off)
    check("6 CYS_BOOT_GATES=0 이면 12 가 사라진다(롤백 1손잡이)", r.returncode == 0,
          repr((r.returncode, r.stdout[-160:])))

    # ─────────────── ⑦ H-CHECK-12-CONSUMERS — 리뷰 게이트 차단 ───────────────
    r = run(["review-prompt", "--task", "T", "--scope", "bin/x.py"], base)
    check("7a review-prompt exit 12", r.returncode == 12, repr((r.returncode, r.stderr[-200:])))
    check("7b 차단 시 stdout 에 의뢰문이 새지 않는다(리뷰어 입력 오염 0)",
          r.stdout.strip() == "", repr(r.stdout[:200]))
    check("7c 처방·근거는 stderr", "각성 ACK 미확인" in r.stderr and "처방" in r.stderr,
          repr(r.stderr[-300:]))
    ledger = os.path.join(pack, "round", "ORCHESTRATION-T.md")
    r = run(["round-init", "--task", "T"], base)
    check("7d round-init exit 12", r.returncode == 12, repr((r.returncode, r.stderr[-160:])))
    check("7e 차단 시 장부 파일 미생성(부작용 0)", not os.path.exists(ledger), ledger)
    # round-log 는 소비자 표에 없다 — 내부 장부 생성까지 막으면 append 가 죽는다
    r = run(["round-log", "--task", "T", "--round", "1", "--evaluator", "master",
             "--verdict", "승인"], base)
    check("7f round-log 는 게이트 대상 아님(장부 생성·기록 성공)",
          r.returncode == 0 and os.path.exists(ledger), repr((r.returncode, r.stderr[-200:])))
    # ack 해제 후에는 통과
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=True)})
    r = run(["review-prompt", "--task", "T", "--scope", "bin/x.py"], base)
    check("7g ACK 확인되면 review-prompt 통과(게이트가 영구 차단이 아니다)",
          r.returncode == 0 and "리뷰 의뢰" in r.stdout, repr((r.returncode, r.stdout[:120])))
    # 데몬 소실(측정 불가)은 차단하지 않는다
    r = run(["review-prompt", "--task", "T", "--scope", "bin/x.py"], env_nostatus)
    check("7h 측정 불가는 차단하지 않는다(새 정지 사유 금지)",
          r.returncode == 0 and "미측정" in r.stderr, repr((r.returncode, r.stderr[-200:])))

    # ─────────────── ⑧ gate-status 무접촉 ───────────────
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=False)})
    with io.open(ledger, "w", encoding="utf-8", newline="\n") as f:
        f.write("# ORCHESTRATION 라운드 장부 — T\n\n"
                "| 라운드 | 평가자 | 기록값 | 판정 |\n|---|---|---|---|\n"
                "| 1 | gemini | - | ACCEPT |\n| 1 | codex | - | ACCEPT |\n"
                "| 1 | master | - | 승인 |\n| 1 | machine | - | PASS(exit 0) |\n")
    r_pending = run(["gate-status", "--task", "T"], base)
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=True)})
    r_ok = run(["gate-status", "--task", "T"], base)
    # ★무접촉의 조작적 정의: ACK 상태가 바뀌어도 **exit 이 한 비트도 안 움직인다**.
    #   (이 기계의 실측 exit 은 4 = "수렴했으나 임무 미지정" — 명세 §4 의 gate-status 불변 대수
    #    0/1/2/4 안이며, 값 자체가 아니라 **불변성**이 이 핀의 명제다.)
    check("8a gate-status exit 이 ACK 상태에 불변",
          r_pending.returncode == r_ok.returncode,
          repr((r_pending.returncode, r_ok.returncode, r_pending.stdout[-160:])))
    check("8b gate-status 가 12 를 내지 않는다(ACK 대수는 그쪽 소관이 아니다)",
          r_pending.returncode != 12 and r_pending.returncode in (0, 1, 2, 4),
          repr(r_pending.returncode))
    src_orc = io.open(ORC, encoding="utf-8").read()
    # 슬라이스 끝은 **다음 최상위 def** 로 잡는다 — 특정 함수명을 앵커로 박으면 그 함수가
    # 개명·이동하는 순간 이 핀이 조용히 다른 범위를 재게 된다(실측으로 겪었다).
    _i0 = src_orc.index("def cmd_gate_status(")
    _i1 = src_orc.index("\ndef ", _i0 + 1) + 1
    gs = src_orc[_i0:_i1]
    _tok = [t for t in ("ack_axis", "ack_gate_precheck", "check_exit_code", "cmd_check",
                        "CHECK_EXIT_ACK_PENDING", "ack[") if t in gs]
    check("8c cmd_gate_status 본문에 ACK·check 심볼 소비 0(무접촉 소스 핀)", not _tok, repr(_tok))
    write_status(st, {"boot_v2_enabled": True, "surfaces": surfaces(ack=False)})

    # ─────────────── ⑨ bootstrap ⑤ 는 12 를 0 으로 접는다 ───────────────
    sys.path.insert(0, BIN)
    import javis_orchestra as orch          # noqa: E402
    src_bs = io.open(BOOTSTRAP, encoding="utf-8").read()
    check("9a bootstrap 이 12 상수를 **정본에서** 가져온다(리터럴 사본 금지)",
          'getattr(_orch_ck, "CHECK_EXIT_ACK_PENDING", 12)' in src_bs)
    check("9b 폴백 리터럴이 정본 값과 같다", orch.CHECK_EXIT_ACK_PENDING == 12,
          repr(orch.CHECK_EXIT_ACK_PENDING))
    check("9c ⑤ 루프가 12 를 0 으로 접고 라벨을 남긴다",
          "if code == CHECK_ACK_PENDING:" in src_bs and "ack_pending = True" in src_bs
          and "0 취급" in src_bs)
    check("9d 접힘 사실이 마커에 기록된다(조용한 강등 금지)",
          '_marker_payload["orchestra_check_ack_pending"] = True' in src_bs)
    check("9e 기존 마커 키·값 무변경(기존 핀 보존)",
          '"orchestra_check": "exit 0"}' in src_bs)

    # ─────────── ⑩ H-ADDR-1 — 역할 주소 resolve 게이트 (★실경로) ───────────
    # ★★R1 codex #1(blocking) 반영: 종전 이 축은 **순수 함수만** 쟀고, 거기 넣던 입력은
    #   실경로에서 만들어질 수 없는 값이었다 — 주소 판정이 생존 판정과 **같은 집합**을
    #   재검사했기 때문이다(게이트 도달 불가 · 검체는 불가능한 중간값으로 공허하게 통과).
    #   이제 주소성은 `cys list`(좌석 레지스트리)에서, 생존은 `cys status`(프로세스 표)에서
    #   온다 — **좌석은 살려 둔 채 레지스트리에서 역할만 지우면** 진짜 갈래가 만들어진다.
    lst = os.path.join(root, "list.txt")
    env_addr = dict(base)
    env_addr["FAKE_CYS_LIST"] = lst
    write_status(st, {"surfaces": surfaces()})                # 좌석 4종 전부 **살아 있다**
    write_list(lst, ["cso", "worker", "reviewer-gemini", "reviewer-codex"])
    r = run(["check"], env_addr)
    check("ADDR-E2E-1 레지스트리가 온전하면 종전대로 exit 0", r.returncode == 0,
          repr((r.returncode, r.stdout[-160:])))
    write_list(lst, ["cso", "worker", "reviewer-codex"])      # ★주소만 사라짐(좌석은 생존)
    r = run(["check"], env_addr)
    check("ADDR-E2E-2 좌석은 살아 있는데 **주소만** 없으면 exit 1(실경로 발화)",
          r.returncode == 1, repr((r.returncode, r.stdout[-260:])))
    check("ADDR-E2E-3 라벨이 '미기동'이 아니라 '주소 미해소'다(거짓 처방 차단)",
          "주소 미해소" in r.stdout and "claim-role" in r.stdout, repr(r.stdout[-320:]))
    rj = run(["check", "--json"], env_addr)
    plj = json.loads(rj.stdout.strip().splitlines()[-1])
    check("ADDR-E2E-4 `--json` 이 사유를 기계 판독 가능하게 싣는다",
          rj.returncode == 1 and plj["exit"] == 1
          and "addr_unresolved" in (plj.get("why") or "")
          and plj["ready_missing"] == ["reviewer-gemini"],
          repr((rj.returncode, plj.get("why"), plj.get("ready_missing"))))
    r = run(["check"], base)                                  # `cys list` rc≠0 = 측정 실패
    check("ADDR-E2E-5 레지스트리 **측정 실패**는 결손이 아니다(exit 0 + 고지)",
          r.returncode == 0 and "레지스트리 미측정" in r.stdout,
          repr((r.returncode, r.stdout[-200:])))

    # ── ★음성 대조 2종(R2 codex #1 blocking) — 성공한 빈 결과 ≠ 측정 실패 ──────────────
    #   종전 구현은 `cys_list_rows()` 의 `[]` 하나로 두 사실을 받았다(그 함수는 rc≠0 에도 []).
    #   그래서 **좌석 레지스트리가 통째로 비어도 check 가 READY/0** 을 냈다 — 게이트가 가장
    #   크게 발화해야 할 상태에서 정확히 침묵했다. 아래 두 케이스가 그 접힘의 유일한 탐지기다.
    empty = os.path.join(root, "list-empty.txt")
    io.open(empty, "w", encoding="utf-8", newline="\n").write("")   # rc 0 · 행 0
    env_empty = dict(base)
    env_empty["FAKE_CYS_LIST"] = empty
    write_status(st, {"surfaces": surfaces()})                # 좌석 4종은 **전부 살아 있다**
    r = run(["check"], env_empty)
    check("ADDR-NEG-1 성공한 **빈 레지스트리**는 결손이다(전원 주소 미해소 → exit 1)",
          r.returncode == 1, repr((r.returncode, r.stdout[-260:])))
    check("ADDR-NEG-2 빈 레지스트리를 '미측정'으로 고지하지 않는다(두 사실 분리)",
          "레지스트리 미측정" not in r.stdout and "주소 미해소" in r.stdout,
          repr(r.stdout[-260:]))
    rj = run(["check", "--json"], env_empty)
    plj = json.loads(rj.stdout.strip().splitlines()[-1])
    check("ADDR-NEG-3 빈 레지스트리에서 satisfied 요건이 **전부** 미해소로 실린다",
          rj.returncode == 1 and "addr_unresolved" in (plj.get("why") or "")
          and set(plj["ready_missing"]) == {"cso", "worker",
                                            "reviewer-gemini", "reviewer-codex"},
          repr((rj.returncode, plj.get("ready_missing"), plj.get("why"))))
    # 대조군: **같은 스텁**에서 rc≠0(파일 미지정)만 다르면 판정이 뒤집힌다 = rc 가 실제로 축이다
    check("ADDR-NEG-4 계측 타당성 — 같은 좌석·같은 스텁에서 rc 만 갈라도 판정이 갈린다",
          run(["check"], base).returncode == 0 and run(["check"], env_empty).returncode == 1,
          "rc 를 안 보면 두 케이스가 같은 값을 낸다(종전 결함의 형상)")
    _src_a = io.open(ORC, encoding="utf-8").read()
    check("ADDR-E2E-6 소스 핀: 주소 판정이 생존 판정과 **같은 자료원**으로 되돌아가지 않았다",
          "def addr_registry_roles(" in _src_a and "cys_list_probe" in _src_a
          and "live_role_names(status or {})" not in _src_a,
          "같은 집합을 재검사하면 게이트가 다시 공허해진다")
    check("ADDR-E2E-7 소스 핀: 성공/실패를 rows 로 되돌려 판정하지 않는다(접힘 회귀 차단)",
          "if not ok:" in _src_a and "if not rows:" not in _src_a,
          "rows 의 진위로 미측정을 판정하면 빈 레지스트리가 다시 초록이 된다")

    # ─────────── ⑩-b 순수 함수 축(입력 = 레지스트리 집합) ───────────
    V = {"cso": {"satisfied": True, "filler": "cso", "why": "x"},
         "reviewer-gemini": {"satisfied": True, "filler": "reviewer-gemini", "why": "x"}}
    check("10a 주소 있으면 미해소 0",
          orch.addr_unresolved_roles({"cso", "reviewer-gemini"}, V) == [])
    check("10b 레지스트리에 role 없음 → 미해소",
          orch.addr_unresolved_roles({"cso"}, V) == ["reviewer-gemini"])
    check("10c 레지스트리 미측정(None)은 결손이 아니다",
          orch.addr_unresolved_roles(None, V) == [])
    check("10d 미충족 좌석은 이 게이트가 다루지 않는다(ready_missing 소관)",
          orch.addr_unresolved_roles({"cso"},
                                     {"reviewer-gemini": {"satisfied": False,
                                                          "filler": None}}) == [])
    M = orch.mark_addr_unresolved(V, ["reviewer-gemini"])
    check("10e 미해소는 ready 미달로 접힌다(exit 1)",
          orch.verdict_axes(M)["ready_missing"] == ["reviewer-gemini"]
          and orch.check_exit_code(orch.verdict_axes(M),
                                   {"pending": ["reviewer-gemini"]}) == 1,
          "축 계급: ready 미달이 12 를 이긴다")
    check("10f 입력 무변조(순수)", V["reviewer-gemini"]["satisfied"] is True)
    check("10g 실충전자(대체 좌석) 이름으로 판정한다 — B2 대체 슬롯 무회귀",
          orch.addr_unresolved_roles(
              {"reviewer-claude-1"},
              {"reviewer-gemini": {"satisfied": True, "filler": "reviewer-claude-1"}}) == [])

    # ─────────────── ⑪ ★음성 대조 — ACK 축을 제거한 변조본은 12 를 못 낸다 ───────────────
    mut = os.path.join(root, "javis_orchestra.py")
    # 변조 ①: exit 단일 정의에서 12 가지를 제거 — 산문·JSON **양 경로가 함께** 0 으로 떨어져야
    #         한다. 한쪽만 떨어지면 규칙 사본이 남아 있다는 뜻이다(이 핀이 실제로 그것을 잡았다).
    mutated = src_orc.replace(
        '    if (ack or {}).get("pending"):\n        return CHECK_EXIT_ACK_PENDING\n', "")
    check("11a 변조 앵커 실재(계측 타당성)", mutated != src_orc)
    with io.open(mut, "w", encoding="utf-8", newline="\n") as f:
        f.write(mutated)
    rmut = run(["check"], base, script=mut)
    rmutj = run(["check", "--json"], base, script=mut)
    check("11b 변조본은 산문 경로에서 12 를 못 낸다",
          rmut.returncode == 0, repr((rmut.returncode, rmut.stdout[-160:])))
    check("11c 변조본은 --json 경로에서도 12 를 못 낸다(exit 단일 정의 증명)",
          rmutj.returncode == 0
          and json.loads(rmutj.stdout.strip().splitlines()[-1])["exit"] == 0,
          repr(rmutj.returncode))
    # 변조 ②: 측정 자체를 죽인다(축을 항상 off) → 12 도 미측정 고지도 사라진다.
    mut2 = os.path.join(root, "javis_orchestra_noaxis.py")
    m2 = src_orc.replace("    if not ack_axis_enabled(status):\n",
                         "    if True:\n", 1)
    check("11d 축 변조 앵커 실재", m2 != src_orc)
    with io.open(mut2, "w", encoding="utf-8", newline="\n") as f:
        f.write(m2)
    rmut2 = run(["check"], base, script=mut2)
    check("11e 축을 죽이면 12 도 죽는다 → 이 검체가 재는 것이 ACK 축이다",
          rmut2.returncode == 0 and "ACK" not in rmut2.stdout,
          repr((rmut2.returncode, rmut2.stdout[-160:])))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("CHECK-ACK-ADDR-GATE-OK")
sys.exit(0)
