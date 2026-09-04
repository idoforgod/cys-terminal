#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_bootlast_golden_lock_parity.py — boot-last 골든 파리티 + 레인 락 원시 파리티 (0.14.30 W-A A4).

세 축을 한 파일에 담는다(명세 §3-1·§4 · 시뮬레이션 T1-9·T1-10·T2-10 · BUILD_PLAN A4):
  · **러너 exit 파리티**  `cys boot-run`(Rust)의 종료 대수 13·14·15 가 python 종료 공간을 침범하지
                        않고, 공유 값(0·7·9·10·11)의 뜻이 갈리지 않는가 — 그리고 python 이 러너
                        전용 값으로 **종료하지 않는가**(소스 실측).
  · **H-BOOTLAST-GOLDEN** python `_Log` 가 쓰는 boot-last 가 상태 전이별 골든과 **문서 동일**한가
                        (휘발 필드만 정규화) · session_error 의 stdout 미러 1줄 형상 ·
                        `boot-skip-<lane>.json`(exit 11)의 boot-last 무접촉.
  · **T1-10 레인 락 원시** 락 파일 경로·바이트 범위·모드가 **기계 판독 가능한 선언**으로 노출되고,
                        두 python 프로세스가 같은 락을 다툴 때 후자가 **비차단으로 실패**하는가.

무엇을 막는가:
 ① 러너(B4)가 boot-last writer 를 새로 구현하면서 python 의 progressive 의미론(running 선기록 →
    steps append → fail/result → finish)을 **일부만** 옮기는 것. 골든이 없으면 그 차이는 사고 당일
    boot-last 를 들여다볼 때에야 드러난다.
 ② `_Log` 자신이 조용히 바뀌는 것 — 키가 하나 사라지면 §0-A 판독 규약(자기 surface 의 최신 완주
    런)·훅 처방(`retry_eligible`)·GUI 진단이 동시에 갈린다.
 ③ 상태 대수가 늘어나는데 골든이 따라오지 않는 것(`team_complete` 가 실제로 그렇게 들어왔다) —
    ②절이 소스의 `state=` 리터럴을 전수 수집해 골든·`expected.json` 과 1:1 을 강제한다.
 ④ 락 원시가 "산문으로만" 합의돼 있는 것. Rust 가 다른 원시를 쓰면 **Windows 에서만** 상호 배제가
    깨지고 증상은 중복 부트다 — 시뮬레이션 §5 가 "실측 검체로만 닫힌다"고 못박은 자리다.

기대값은 **테스트 코드가 아니라 `fixtures/boot-last-golden/expected.json` 에서 읽는다**
(test_todo_decl.py:5-9 와 같은 이유: 기대값이 python 안에 있으면 Rust 파리티가 껍데기가 된다).
python 이 갖는 것은 '어떤 kwargs 로 `_Log` 를 모는가'라는 **드라이버 레시피**뿐이고, 그 레시피가
생산 호출부와 갈리지 않는다는 보증은 ④절의 소스 핀이 진다.

밀폐: `tempfile.mkdtemp()` + `HOME`·`CYS_STATE_DIR`·`CYS_PACK_DIR`·`CYS_SOCKET`·`CYS_SURFACE_ID`·
`CYS_ROLE` 덮어쓰기 + `PATH` 를 **가짜 cys(즉시 실패) 1개**만 있는 디렉터리로 교체 — 라이브 팩·
라이브 데몬·라이브 boot-last 무접촉이고 노드 스폰 0이다. 변조 사본은 임시 bin 디렉터리에만 쓴다.

★음성 대조(계측 타당성): ⑨절이 구현을 무력화한 **변조 사본** 6종을 만들어 이 검체가 실제로
  FAIL 하는지 실증한다(running 선기록 제거 · crashed/aborted 융합 · 미러 봉인 제거 · 러너 exit
  상수 이탈 · 락 비차단 실패 제거 · 골든 없는 새 state 추가). 음성 대조 없는 검체는 계측 타당성
  미증명이다.

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 BOOTLAST-GOLDEN-LOCK-PARITY-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_bootlast_golden_lock_parity.py
골든 재생성(계약이 **실제로** 바뀌었을 때만): `--regen`
  — 재생성은 실행 산출을 그대로 굳히므로 '초록으로 만들기' 수단이 아니다. 재생성 전후 diff 를
    리뷰에 붙이지 않으면 골든은 계약이 아니라 스냅샷이 된다(⑨ 음성 대조가 그 구분을 강제한다).
"""
import ast
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True     # 팩 봉인(SEAL-1) 정신 — 검체가 bin/ 에 캐시를 남기지 않는다

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
FIXTURES = os.path.join(SELF, "fixtures", "boot-last-golden")
EXPECTED_PATH = os.path.join(FIXTURES, "expected.json")
BOOTSTRAP_SRC_PATH = os.path.join(BIN, "javis_bootstrap.py")
LOCK_SRC_PATH = os.path.join(BIN, "javis_lock.py")
PY = sys.executable or "python3"
REGEN = "--regen" in sys.argv[1:]
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _read(path):
    with io.open(path, encoding="utf-8") as f:
        return f.read()


def _write_json(path, obj):
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.write("\n")


def _load(path):
    try:
        return json.loads(_read(path))
    except (OSError, ValueError):
        return None


# ─────────────────────────────────────────────────────────────────────────────
# 드라이버 — `_Log` 를 **생산 호출부와 같은 kwargs 로** 몰아 산출을 그대로 덤프한다.
#   ★왜 인프로세스인가: 상태 12종 중 dept_fallback 계열·crashed·aborted 는 블랙박스 실행으로
#     재현하려면 cys-dept·launch-agent 다중 스텁과 SystemExit 주입이 필요하다. 특히 `aborted` 는
#     현행 체인의 **모든 return 이 result 를 먼저 남기므로 블랙박스로는 도달 불가**다(방어 분기).
#     writer 의미론(=명세 §3-1 이 러너에 요구하는 것)은 `_Log` 가 전부 소유하므로 그 표면을
#     직접 재는 것이 더 정확하고 밀폐도 강하다.
#   ★kwargs 를 여기서 지어내지 않는다는 보증은 ④절의 **소스 호출부 핀**이 진다 — 생산 코드에
#     그 `state="…"` 리터럴이 실재하지 않으면 FAIL 이다.
# ─────────────────────────────────────────────────────────────────────────────
_DRIVER = r'''
import io, json, os, sys
sys.dont_write_bytecode = True          # 팩 봉인(SEAL-1) 정신 — bin/ 에 캐시를 남기지 않는다
bindir, case = sys.argv[1], sys.argv[2]
sys.path.insert(0, bindir)
import javis_bootstrap as B

_real = sys.stdout


class _Cap(object):
    def __init__(self):
        self.buf = []

    def write(self, s):
        self.buf.append(s)
        return len(s)

    def flush(self):
        pass


cap = _Cap()
sys.stdout = cap


POST_CLAIM = __POST_CLAIM__


def drive(case, lg):
    if case in POST_CLAIM:
        lg.data["role_claimed"] = "master"       # 생산 호출부 javis_bootstrap.py 와 동형
    if case == "running":
        return
    if case == "completed":
        lg.step(B.STEP.PREFLIGHT, 0, "detail")
        lg.result(ok=True, state="completed", exit=B.EXIT_OK)
        lg.finish(B.EXIT_OK)
        return
    if case == "team_complete":
        lg.step(B.STEP.CEO_TICKET_REQUEST, 1, "detail")
        lg.result(ok=True, state="team_complete", solo_awakening=False, team_complete=True,
                  reason="detail", ticket_requested=False, exit=B.EXIT_OK)
        lg.finish(B.EXIT_OK)
        return
    if case == "solo_awakening":
        lg.step(B.STEP.CEO_TICKET_SOLO, 0, "detail")
        lg.result(ok=True, state="solo_awakening", solo_awakening=True, reason="detail",
                  ticket_requested=True, exit=B.EXIT_OK)
        lg.finish(B.EXIT_OK)
        return
    if case == "dept_fallback":
        lg.step(B.STEP.DEPT_FB_CHECK, 0, "detail")
        lg.result(ok=None, state="dept_fallback", dept="dept-1", exit=B.EXIT_OK)
        lg.finish(B.EXIT_OK)
        return
    if case == "failed-ping":
        lg.fail(B.STEP.PING, 1, "detail", B.EXIT_PING)
        lg.finish(B.EXIT_PING)
        return
    if case == "failed-check":
        lg.fail(B.STEP.CHECK, 1, "detail", B.EXIT_CHECK)
        lg.finish(B.EXIT_CHECK)
        return
    if case == "failed-lane-pack":
        lg.step(B.STEP.LANE_PACK, 1, "detail")
        lg.result(ok=False, state="failed", failed_step="lane-pack", exit=B.EXIT_LANE_PACK)
        lg.finish(B.EXIT_LANE_PACK)
        return
    if case == "failed-resource-gate":
        lg.step(B.STEP.RESOURCE_GATE_NOTIFY, 0, "detail")
        lg.result(ok=False, state="failed", failed_step="resource-gate",
                  exit=B.EXIT_RESOURCE_HARD)
        lg.finish(B.EXIT_RESOURCE_HARD)
        return
    if case == "declined":
        lg.fail(B.STEP.CLAIM_ROLE, 1, "detail", B.EXIT_CLAIM_DENIED, ok=None, state="declined")
        lg.finish(B.EXIT_CLAIM_DENIED)
        return
    if case in ("session_error", "session_error_latched"):
        lg.fail(B.STEP.CLAIM_ROLE_CONTEXT, 6, "detail", B.EXIT_SESSION_CONTEXT,
                ok=None, state="session_error")
        lg.finish(B.EXIT_SESSION_CONTEXT)
        return
    if case == "dept_fallback_failed":
        lg.fail(B.STEP.DEPT_FB_ALLOC, 1, "detail", B.EXIT_BOOT, ok=None,
                state="dept_fallback_failed")
        lg.finish(B.EXIT_BOOT)
        return
    if case == "dept_fallback_gate_pending":
        lg.fail(B.STEP.DEPT_FB_MASTER, 78, "detail", B.EXIT_BOOT, ok=None,
                state="dept_fallback_gate_pending")
        lg.finish(B.EXIT_BOOT)
        return
    if case == "crashed":
        lg.finish(1, exc="RuntimeError: detail")
        return
    if case == "aborted":
        lg.finish(B.EXIT_RESOURCE_HARD)
        return
    raise SystemExit("미지 케이스: %r" % case)


out = {"case": case}
if case == "skip":
    # exit 11 — boot-last **무접촉**이 계약이다. _Log 를 만들지 않는다.
    out["exit"] = B._emit_skip_verdict()
    out["boot_last_exists"] = os.path.exists(B.lane_state_path("boot_last"))
    try:
        with io.open(B._skip_record_path(), encoding="utf-8") as f:
            out["skip"] = json.load(f)
    except Exception as e:
        out["skip"] = {"_read_error": "%s: %s" % (type(e).__name__, e)}
else:
    lg = B._Log(B._load_retry_carry(B.lane_state_path("boot_last")))
    drive(case, lg)
    out["write_failures"] = lg.write_failures()
    try:
        with io.open(B.lane_state_path("boot_last"), encoding="utf-8") as f:
            out["doc"] = json.load(f)
    except Exception as e:
        out["doc"] = {"_read_error": "%s: %s" % (type(e).__name__, e)}
out["stdout"] = "".join(cap.buf)
sys.stdout = _real
json.dump(out, _real, ensure_ascii=False)
'''

# ③claim-role 성공 **뒤**에 도달하는 상태들 — 생산 코드가 `log.data["role_claimed"] = "master"`
# 를 심으므로 boot-last 최상위에 그 키가 **있다**. 거부(declined)·거부 이후 폴백(dept_fallback*)·
# ②ping·⓪레인 가드에는 **없다**. 이 조건부 키의 분포 자체가 계약이다 — 있어야 할 곳에 없으면
# §0-A 의 '자기 surface 의 최신 완주 런' 판독이 갈린다(정찰 traps 의 조건부 키 함정).
POST_CLAIM = ("completed", "team_complete", "solo_awakening", "failed-check",
              "failed-resource-gate")

# 드라이버가 아는 케이스 id(= 골든 파일 1개씩). 기대값(state/ok/exit/note)은 expected.json 소유.
CASE_IDS = [
    "running", "completed", "team_complete", "solo_awakening", "dept_fallback",
    "failed-ping", "failed-check", "failed-lane-pack", "failed-resource-gate",
    "declined", "session_error", "session_error_latched",
    "dept_fallback_failed", "dept_fallback_gate_pending", "crashed", "aborted",
]
CASE_NOTES = {
    "running": "선기록만 — 이 시점 이후 어떤 경로로 죽어도 '진행 중'이 남는다(A19)",
    "completed": "정상 완주(⑤check 통과 → ⑥마커 → ⑧요약 JSON)",
    "team_complete": "부서 재기동 · 결손 0 → 팀 기동 불요(0.14.30 A4 #13 신설 · 명세 미등재 상태)",
    "solo_awakening": "부서장 단독 각성(CEO 티켓 부재 — 실패 아님)",
    "dept_fallback": "③ 정당거부 → 부서 자동 생성 폴백 성공(ok=None)",
    "failed-ping": "②ping 실패 — fail() 경로(step + notify 동반)",
    "failed-check": "⑤check 최종 실패 — fail() 경로",
    "failed-lane-pack": "레인↔팩 불일치 — result() 직접 경로(notify 없음)",
    "failed-resource-gate": "자원 hard_block — result() 직접 경로",
    "declined": "③ 정당거부(ok=None — 건강한 master 의 ok:true 를 덮지 않는다)",
    "session_error": "③ 비거부 실패 — 래치 첫 기록(retry_eligible=true) + stdout 미러 1줄",
    "session_error_latched": "같은 surface 연속 2회째 — 래치 소진(retry_eligible=false)",
    "dept_fallback_failed": "부서 자동 생성 실패",
    "dept_fallback_gate_pending": "부서장 기동이 첫기동 관문에 갇힘(exit 78 수신)",
    "crashed": "미포착 예외 — finish(exc) 가 running 을 crashed 로 전이",
    "aborted": "result 없이 종결 — finish(exc 없음)가 running 을 aborted 로 전이. ★현행 체인은 "
               "모든 return 이 result 를 먼저 남기므로 **블랙박스 도달 불가**인 방어 분기다. "
               "이 골든은 _Log 직접 구동으로만 만들어진다 — 그 사실을 숨기지 않는다",
}

# 골든에서 **계약이 아닌** 필드(값이 매 런 달라진다) → 정규화 치환값. Rust(B4)가 같은 규칙을
# 구현하면 같은 골든을 본다.
NORMALIZED = {
    "started": "<ts>", "ended": "<ts>", "run_id": "<run_id>", "pid": 0,
    "boot_last_path": "<boot_last>", "boot_last": "<boot_last>",
    "detail": "<detail>", "channel": "<channel>", "last": "<prose>",
    "at": 0, "lock": "<lock>", "record": "<record>",
    "reason": "<prose>", "self_check": "<prose>",
    "surface_role": "<observed>", "is_master": "<observed>",
}

EXPECTED_DOC = {
    "_contract": (
        "골든 픽스처 = boot-last writer 의미론(명세 §3-1)의 SOT. python `_Log`(javis_bootstrap.py) "
        "와 Rust 러너(`cys boot-run` · B4)가 **같은 상태 전이에서 같은 문서**를 써야 한다. 절차: "
        "상태별로 writer 를 몰아 boot-last 를 만들고 → `_normalized` 규칙으로 휘발 필드를 치환한 뒤 "
        "→ 이 디렉터리의 `state-<case>.json` 과 **문서 전체를 동일 비교**한다."),
    "_contract_surface": (
        "대조 대상은 정규화 후 **문서 전체**다(키 집합만이 아니다). 최상위 12필드 + 조건부 "
        "role_claimed·ended·exit·exc·log_write_failures, `result` 하위 전 필드, `steps[]` 의 "
        "step/exit/order, 최상위 `retry` 래치 맵이 전부 계약이다. 상태별 기대값(state·ok·exit)은 "
        "`cases` 가 들고 있으며 테스트 코드에 하드코딩하지 않는다."),
    "_not_contract": (
        "★사람이 읽는 한국어 진단 문구·타임스탬프·pid·절대 경로는 계약이 아니다. 그래서 "
        "`_normalized` 가 그것들을 치환한다 — 다시 실제 값으로 되돌리지 마라. 문구를 계약에 "
        "넣으면 문구를 다듬는 순간 파리티가 결함을 오보한다(todo-decl 픽스처와 같은 교리). "
        "`notify.channel` 도 런타임 가용성 파생이라 계약이 아니다."),
    "_normalized": NORMALIZED,
    "_aux_files": {
        "expected.json": "이 파일(SOT).",
        "mirror-session_error.json": "§3-1 stdout 미러 1줄 — session_error(exit 10) 전용 채널. "
                                     "훅·MASTER_DIRECTIVE·CEO_TEMPLATE 가 키 이름을 문자열로 인용한다.",
        "mirror-session_error-persist_failed.json": "디스크 쓰기가 전량 실패했을 때의 미러 — "
                                                    "retry_eligible=false + retry_eligible_unknown=true"
                                                    "(측정 불능은 재실행 허가가 아니다).",
        "skip-verdict.json": "exit 11(skipped_inflight)의 `boot-skip-<lane>.json`. **boot-last 가 "
                             "아니다** — 러너가 terminal 로 전사할 때 자리가 다르다.",
        "lock-primitive.json": "레인 락 원시 계약(T1-10) — Rust 러너가 include_str! 로 같은 표를 본다.",
    },
    "_state_note": (
        "`skipped_inflight` 는 boot-last 의 state 가 아니라 별도 파일의 `verdict` 다(정찰 실측). "
        "명세 §3-2 는 이것을 terminal.kind 목록에 넣었지만 boot-last 파일 안에는 자리가 없다."),
    "cases": {},
}

LOCK_FIXTURE_DOC = [
    "레인 락 원시 계약(시뮬레이션 T1-10 · T2-10 · 명세 §2-7) — 소유자는 javis_bootstrap.LANE_LOCK_PRIMITIVE.",
    "이 파일은 그 상수의 사본이며 Rust 러너(W-B B4)가 include_str! 로 읽어 같은 표를 보게 하는 것이 목적이다.",
    "불변식: 같은 파일(path_template) · 같은 바이트 범위([byte_offset, byte_offset+byte_length)) · 같은 모드(mode).",
    "posix flock 은 범위 인자가 없는 파일 전체 advisory 락이라 [0,1)을 포함한다 — 범위가 의미를 갖는 축은 windows 뿐이다.",
    "확장 규칙: 필드는 추가만 한다(K3). 값을 바꾸면 python self-test 와 이 검체가 동시에 운다.",
]
LOCK_RUST_STATUS = ("not_tested: Rust boot-run 미착지(W-B B4) — 이 fixture 는 python 축만 실측으로 "
                    "닫는다. Rust 가 착지하면 같은 파일을 읽어 교차 배제(python 보유 중 Rust 획득 "
                    "실패 · 그 역)를 실증해야 계약이 닫힌다.")


def normalize(obj):
    if isinstance(obj, dict):
        return {k: (NORMALIZED[k] if (k in NORMALIZED and not isinstance(v, (dict, list)))
                    else normalize(v)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [normalize(v) for v in obj]
    return obj


def golden_path(case):
    return os.path.join(FIXTURES, "state-%s.json" % case)


root = tempfile.mkdtemp(prefix="a4-golden-")
try:
    # ── 밀폐 env: 라이브 HOME·state·팩·데몬 무접촉 ────────────────────────────
    home = os.path.join(root, "home")
    state = os.path.join(home, ".cys", "state")
    nobin = os.path.join(root, "nobin")
    os.makedirs(state)
    os.makedirs(nobin)
    fake_cys = os.path.join(nobin, "cys")
    with io.open(fake_cys, "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\nexit 1\n")     # _notify_loud 두 채널 모두 실패 → 'none(...)' 결정론
    os.chmod(fake_cys, 0o755)

    def base_env(**over):
        e = dict(os.environ)
        for k in ("CYS_LOCK_BACKEND", "CYS_BOOT_LANE_LEGACY", "AITERM_SURFACE_ID",
                  "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
            e.pop(k, None)
        # ★바이트코드 캐시 억제는 **env 가 아니라 자식 스크립트의 `sys.dont_write_bytecode`** 로
        #   건다 — 봉인 인구조사(test_pyseal_census ⓑ)가 그 env 이름을 언급하는 파일 집합을
        #   동결하고 있어서, 검체가 문자열만으로 그 집합에 끼면 인구조사가 결함을 오보한다.
        e.update({"HOME": home, "CYS_STATE_DIR": state,
                  "CYS_PACK_DIR": os.path.join(root, "pack"),
                  "CYS_SOCKET": "", "CYS_SURFACE_ID": "7", "CYS_ROLE": "master",
                  "PATH": nobin + os.pathsep + "/usr/bin" + os.pathsep + "/bin"})
        e.update(over)
        return e

    driver_src = _DRIVER.replace("__POST_CLAIM__", repr(POST_CLAIM))   # 목록 단일 소유(사본 금지)

    def drive(case, bindir=BIN, env=None):
        r = subprocess.run([PY, "-c", driver_src, bindir, case], capture_output=True, text=True,
                           env=env or base_env(), timeout=90)
        if r.returncode != 0 or not r.stdout.strip():
            return {"_driver_error": "rc=%d stderr=%s" % (r.returncode, r.stderr[-400:])}
        return json.loads(r.stdout)

    def fresh():
        """레인 상태 초기화 — 케이스 간 carry-forward 오염 차단."""
        for name in os.listdir(state):
            p = os.path.join(state, name)
            if os.path.isdir(p):
                shutil.rmtree(p, ignore_errors=True)
            else:
                os.unlink(p)

    src_bs = _read(BOOTSTRAP_SRC_PATH)
    src_lock = _read(LOCK_SRC_PATH)
    sys.path.insert(0, BIN)
    import javis_bootstrap as B          # noqa: E402  (상수 판독 전용)
    import javis_lane as LANE            # noqa: E402

    if REGEN and not os.path.isdir(FIXTURES):
        os.makedirs(FIXTURES)
    if REGEN:
        print("!! --regen: 골든을 실행 산출로 덮어쓴다 — diff 를 반드시 리뷰에 붙여라 !!")

    # ═══════════ ① 러너 exit 파리티 ═══════════
    check("1a 러너 전용 상수 3종 실재·값 고정(명세 §4)",
          (B.RUNNER_EXIT_ABORTED, B.RUNNER_EXIT_COMPLETED_DEGRADED, B.RUNNER_EXIT_CRASHED)
          == (13, 14, 15),
          repr((B.RUNNER_EXIT_ABORTED, B.RUNNER_EXIT_COMPLETED_DEGRADED, B.RUNNER_EXIT_CRASHED)))
    py_exits = {int(v) for v in re.findall(r"^EXIT_[A-Z_]+\s*=\s*(\d+)", src_bs, re.M)}
    check("1b python 종료 공간 11종 추출(추출기 파손 시 fail-closed)", len(py_exits) == 11,
          repr(sorted(py_exits)))
    runner_only = {B.RUNNER_EXIT_ABORTED, B.RUNNER_EXIT_COMPLETED_DEGRADED, B.RUNNER_EXIT_CRASHED}
    check("1c 러너 전용 값이 python 종료 공간을 침범하지 않는다",
          not (runner_only & py_exits), repr(sorted(runner_only & py_exits)))
    check("1d 러너 exit 표 정의역 = 명세 §4 의 8종",
          set(B.RUNNER_EXIT_TERMINAL) == {0, 7, 9, 10, 11, 13, 14, 15},
          repr(sorted(B.RUNNER_EXIT_TERMINAL)))
    check("1e 공유 값의 terminal 의미가 갈리지 않는다(파리티의 실체)",
          [B.RUNNER_EXIT_TERMINAL[c] for c in (0, 7, 9, 10, 11)]
          == ["completed", "declined", "aborted", "session_error", "skipped_inflight"],
          repr({c: B.RUNNER_EXIT_TERMINAL[c] for c in (0, 7, 9, 10, 11)}))
    check("1f check 의 ack_pending(12)과 값 충돌 없음", 12 not in B.RUNNER_EXIT_TERMINAL)
    check("1g 부서 exec 전사표 = 시뮬 §3 회전3 13번 행(4·5·6·8 → aborted)",
          B.DEPT_EXEC_TERMINAL == {4: ("aborted", "boot_failed"), 5: ("aborted", "assert_ready"),
                                   6: ("aborted", "check_failed"), 8: ("aborted", "lane_pack")},
          repr(B.DEPT_EXEC_TERMINAL))
    # ★주석 줄은 제외한다 — 이 파일의 상수 주석이 "`return RUNNER_EXIT_ABORTED` 같은 코드는
    #   존재해서는 안 된다"고 **경고를 적어 두었기 때문**이다. 경고문을 금지 대상으로 세면
    #   계측기가 자기 문서를 결함으로 오보한다.
    bad_return = [ln.strip() for ln in src_bs.splitlines()
                  if not ln.lstrip().startswith("#")
                  and re.search(r"return\s+RUNNER_EXIT_[A-Z_]+", ln)]
    check("1h python 체인이 러너 전용 exit 로 return 하는 경로 0건", not bad_return, repr(bad_return))
    check("1i 러너 전용 값의 리터럴 종료 경로도 0건(13·14·15 로 나가지 않는다)",
          not [n for n in re.findall(r"^\s+return (\d{1,3})\s*$", src_bs, re.M)
               if int(n) in runner_only])
    check("1j `--self-test` 가 이 파리티를 상주 배터리로 갖는다",
          "러너 aborted exit 상수 이탈" in src_bs and "러너 exit 표가 명세 §4" in src_bs)

    # ═══════════ ② 상태 대수 전수성 — 소스 ↔ 골든 ↔ expected.json 1:1 ═══════════
    log_body = src_bs[src_bs.index("class _Log:"):src_bs.index("def _observe_surface_role")]
    inner = set()
    for m in re.findall(r'"state":\s*"([a-z_]+)"(?:\s*if exc else\s*"([a-z_]+)")?', log_body):
        inner |= {s for s in m if s}
    # `_Log` 스스로 만드는 state 는 이 셋뿐이다(선기록 + 종결 전이 2분기). 여기에 새 값이 생기면
    # 아래 src_states 수집(=`state=` kwargs)이 그것을 못 보므로 전수성이 조용히 거짓이 된다.
    check("2a `_Log` 내부 state 리터럴 = {running, crashed, aborted}",
          inner == {"running", "crashed", "aborted"}, repr(sorted(inner)))
    # ★AST 로 센다(정규식 아님): `log.result(state=…)`·`log.fail(state=…)` 호출의 키워드만 세야
    #   self-test 안의 status 픽스처(`{"state": "working"}`)나 지역 변수 대입을 유령으로 오보하지
    #   않는다. `fail()` 은 state 를 안 넘기는 호출이 있으므로 **시그니처 기본값**도 함께 센다.
    tree = ast.parse(src_bs)
    call_states = set()
    fail_default = None
    for node in ast.walk(tree):
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                and node.func.attr in ("result", "fail")):
            for kw in node.keywords:
                if (kw.arg == "state" and isinstance(kw.value, ast.Constant)
                        and isinstance(kw.value.value, str)):
                    call_states.add(kw.value.value)
        if isinstance(node, ast.FunctionDef) and node.name == "fail":
            args = node.args.args[1:]                       # self 제외
            offset = len(args) - len(node.args.defaults)
            for i, a in enumerate(args):
                if a.arg == "state" and i >= offset:
                    d = node.args.defaults[i - offset]
                    if isinstance(d, ast.Constant) and isinstance(d.value, str):
                        fail_default = d.value
    check("2b `_Log.fail` 의 state 기본값이 실재(state 미지정 호출의 귀착지)",
          fail_default == "failed", repr(fail_default))
    src_states = call_states | ({fail_default} if fail_default else set()) | inner
    check("2b′ state 리터럴 전수 수집(추출기 파손 시 fail-closed)", len(src_states) >= 12,
          repr(sorted(src_states)))

    produced = {}
    for case in CASE_IDS:
        fresh()
        if case == "session_error_latched":
            drive("session_error")            # 1회차로 래치를 채운 뒤 2회차를 잰다
        produced[case] = drive(case)

    if REGEN:
        cases = {}
        for case in CASE_IDS:
            res = ((produced[case].get("doc") or {}).get("result") or {})
            cases[case] = {"note": CASE_NOTES[case], "golden": "state-%s.json" % case,
                           "state": res.get("state"), "ok": res.get("ok"), "exit": res.get("exit")}
        EXPECTED_DOC["cases"] = cases
        _write_json(EXPECTED_PATH, EXPECTED_DOC)
    SPEC = _load(EXPECTED_PATH) or {}
    check("2c expected.json(SOT) 판독", bool(SPEC.get("cases")), EXPECTED_PATH)
    spec_cases = SPEC.get("cases") or {}
    check("2d 드라이버 케이스 ↔ expected.json 1:1", set(CASE_IDS) == set(spec_cases),
          "차분: %s" % sorted(set(CASE_IDS) ^ set(spec_cases)))
    golden_states = {v.get("state") for v in spec_cases.values()}
    check("2e 골든이 소스의 state 대수를 전수 덮는다(누락 0)", src_states <= golden_states,
          "미커버: %s" % sorted(src_states - golden_states))
    check("2f 골든에 소스에 없는 유령 state 가 없다", golden_states <= src_states,
          "유령: %s" % sorted(golden_states - src_states))
    check("2g 신설 상태 team_complete 가 골든에 있다(명세·시뮬 미등재 — 실측이 정본)",
          "team_complete" in golden_states)
    check("2h `_normalized`(비계약 필드 규칙)가 SOT 에 실려 있다", SPEC.get("_normalized") == NORMALIZED)
    # 2i(디스크 fixture ↔ SOT 1:1)는 ⑧절 끝에 있다 — `--regen` 이 ③~⑧에서 파일을 쓰므로 여기서
    # 세면 재생성 첫 런이 아직 안 써진 파일을 결손으로 오보한다.

    # ═══════════ ③ 상태 전이별 골든 파리티 ═══════════
    for case in CASE_IDS:
        got = produced[case]
        exp = spec_cases.get(case) or {}
        if "_driver_error" in got:
            check("3-%s 드라이버 구동" % case, False, got["_driver_error"])
            continue
        doc = got.get("doc") or {}
        res = doc.get("result") or {}
        check("3-%s state/ok/exit 3자 정합(SOT 대조)" % case,
              (res.get("state"), res.get("ok"), res.get("exit"))
              == (exp.get("state"), exp.get("ok"), exp.get("exit")),
              "got=%r want=%r" % ((res.get("state"), res.get("ok"), res.get("exit")),
                                  (exp.get("state"), exp.get("ok"), exp.get("exit"))))
        check("3-%s 디스크 반영 실측(log_write_failures 0 — 측정 불능은 통과가 아니다)" % case,
              got.get("write_failures") == 0, repr(got.get("write_failures")))
        norm = normalize(doc)
        gp = golden_path(case)
        if REGEN:
            _write_json(gp, norm)
        gold = _load(gp)
        check("3-%s 골든 문서 동일(휘발 필드만 정규화)" % case, gold == norm,
              "" if gold == norm else "키 차분=%s" % sorted(set(gold or {}) ^ set(norm)))

    step_order_ok = True
    for case in CASE_IDS:
        last = -1
        for rec in ((produced[case].get("doc") or {}).get("steps") or []):
            if rec.get("step") not in B.STEP_ORDER or "order" not in rec:
                step_order_ok = False
            if rec.get("order", -1) < last or "order_violation" in rec or "step_unregistered" in rec:
                step_order_ok = False
            last = rec.get("order", last)
    check("3z 골든의 steps 가 STEP 레지스트리 소속 + order 단조(계측기 자기고장 0)", step_order_ok)

    # ═══════════ ④ 생산 호출부 핀 — 드라이버가 kwargs 를 지어내지 않았다 ═══════════
    for lit, why in (
        ('log.result(ok=True, state="completed", exit=EXIT_OK)', "완주"),
        ('state="team_complete"', "부서 재기동 결손 0"),
        ('state="solo_awakening"', "단독 각성"),
        ('log.result(ok=None, state="dept_fallback", dept=name, exit=EXIT_OK)', "부서 폴백"),
        ('ok=None, state="declined")', "정당거부"),
        ('ok=None, state="session_error")', "세션 컨텍스트 오류"),
        ('ok=None, state="dept_fallback_failed")', "부서 생성 실패"),
        ('ok=None, state="dept_fallback_gate_pending")', "부서장 관문 보류"),
        ('log.result(ok=False, state="failed", failed_step="lane-pack", exit=EXIT_LANE_PACK)',
         "레인↔팩"),
        ('state="failed", failed_step="resource-gate"', "자원 hard"),
        ('"result": {"ok": None, "state": "running"', "running 선기록"),
        ('"state": "crashed" if exc else "aborted"', "종결 전이"),
        ('log.fail(STEP.PING, code,', "②ping 실패"),
        ('return log.fail(STEP.CHECK, code,', "⑤check 실패"),
        ('log.data["role_claimed"] = "master"', "③ 성공 후 조건부 키"),
    ):
        check("4-%s 생산 호출부 실재" % why, lit in src_bs, lit[:60])
    # 조건부 키의 **분포**가 계약이다 — ③ 성공 뒤에만 있고, 거부·거부 이후 폴백에는 없다.
    have_rc = {c for c in CASE_IDS if "role_claimed" in ((produced[c].get("doc") or {}))}
    check("4-role_claimed 분포: ③ 성공 계열에만 실린다", have_rc == set(POST_CLAIM),
          "got=%s want=%s" % (sorted(have_rc), sorted(POST_CLAIM)))

    # ═══════════ ⑤ stdout 미러 1줄(§3-1) ═══════════
    mirror_raw = (produced.get("session_error") or {}).get("stdout", "").strip()
    mirror = None
    try:
        mirror = json.loads(mirror_raw)
    except ValueError:
        pass
    check("5a session_error 는 stdout 미러 1줄을 낸다",
          isinstance(mirror, dict) and len(mirror_raw.splitlines()) == 1, repr(mirror_raw[:120]))
    if isinstance(mirror, dict):
        check("5b 미러 키 10종 고정(훅·MASTER_DIRECTIVE·CEO_TEMPLATE 가 문자열로 인용)",
              sorted(mirror) == sorted(["channel", "state", "exit", "run_id", "surface", "lane",
                                        "boot_last", "retry_eligible", "retry_eligible_unknown",
                                        "log_write_failures"]), repr(sorted(mirror)))
        check("5c 미러 channel 문자열 고정", mirror.get("channel") == "boot-last-mirror")
        check("5d 첫 실패는 재실행 가능(래치 count 1)", mirror.get("retry_eligible") is True)
        mp = os.path.join(FIXTURES, "mirror-session_error.json")
        nm = normalize(mirror)
        if REGEN:
            _write_json(mp, nm)
        check("5e 미러 골든 동일", _load(mp) == nm, repr(nm)[:140])
    try:
        lm = json.loads((produced.get("session_error_latched") or {}).get("stdout", "").strip())
    except ValueError:
        lm = {}
    check("5f 같은 surface 2회째는 래치 소진(retry_eligible=false)",
          lm.get("retry_eligible") is False, repr(lm.get("retry_eligible")))
    check("5g 미러는 session_error 전용 — 다른 실패는 stdout 무접촉(A7 성공 전용 채널)",
          not (produced.get("failed-ping") or {}).get("stdout", "").strip()
          and not (produced.get("declined") or {}).get("stdout", "").strip()
          and not (produced.get("completed") or {}).get("stdout", "").strip())

    # ═══════════ ⑥ 쓰기 불능(persist 실패) — 측정 불능은 통과가 아니다 ═══════════
    fresh()
    os.makedirs(os.path.join(state, "boot-last.json"))   # 디스크 쓰기를 결정론으로 봉인
    dead = drive("session_error")
    fresh()
    try:
        dm = json.loads(dead.get("stdout", "").strip())
    except ValueError:
        dm = None
    check("6a 디스크가 죽어도 미러 1줄은 산다(§3-1 stdout 채널)", isinstance(dm, dict), repr(dead)[:200])
    if isinstance(dm, dict):
        check("6b 쓰기 실패는 재실행 허가가 아니다(retry_eligible=false)",
              dm.get("retry_eligible") is False, repr(dm.get("retry_eligible")))
        check("6c 측정 불능이 명시된다(retry_eligible_unknown=true)",
              dm.get("retry_eligible_unknown") is True)
        check("6d 쓰기 실패 횟수가 실려 있다",
              isinstance(dm.get("log_write_failures"), int) and dm["log_write_failures"] > 0,
              repr(dm.get("log_write_failures")))
        dp = os.path.join(FIXTURES, "mirror-session_error-persist_failed.json")
        nd = normalize(dm)
        nd["log_write_failures"] = "<count>"
        if REGEN:
            _write_json(dp, nd)
        check("6e 쓰기불능 미러 골든 동일", _load(dp) == nd, repr(nd)[:140])

    # ═══════════ ⑦ skipped_inflight — boot-last 무접촉(별도 파일·별도 스키마) ═══════════
    fresh()
    sk = drive("skip")
    check("7a skip 은 exit 11 로 종결", sk.get("exit") == 11, repr(sk.get("exit")))
    check("7b skip 은 boot-last 를 만들지 않는다(단일-writer 불변식)",
          sk.get("boot_last_exists") is False, repr(sk.get("boot_last_exists")))
    check("7c skip 은 stdout 무접촉(완료 선언 채널 오염 0)", not sk.get("stdout", "").strip())
    skd = sk.get("skip") or {}
    check("7d skip verdict 키 14종 고정",
          sorted(skd) == sorted(["verdict", "ok", "exit", "reason", "run_id", "pid", "surface",
                                 "surface_role", "is_master", "self_check", "lock", "waited",
                                 "boot_last_untouched", "record"]), repr(sorted(skd)))
    check("7e verdict/boot_last_untouched 값 고정",
          skd.get("verdict") == "skipped_inflight" and skd.get("boot_last_untouched") is True)
    check("7f skipped_inflight 은 boot-last state 가 아니다(러너 전사 시 자리가 다르다)",
          "skipped_inflight" not in golden_states)
    spath = os.path.join(FIXTURES, "skip-verdict.json")
    nsk = normalize(skd)
    if REGEN:
        _write_json(spath, nsk)
    check("7g skip verdict 골든 동일", _load(spath) == nsk, repr(nsk)[:140])

    # ═══════════ ⑧ T1-10 레인 락 원시 파리티 ═══════════
    P = json.loads(json.dumps(B.LANE_LOCK_PRIMITIVE))     # JSON 왕복(fixture 와 같은 표현으로)
    lockp = os.path.join(FIXTURES, "lock-primitive.json")
    if REGEN:
        _write_json(lockp, {"$doc": LOCK_FIXTURE_DOC, "rust_parity_status": LOCK_RUST_STATUS,
                            "primitive": P})
    lf = _load(lockp) or {}
    check("8a 락 원시 선언이 fixture 로도 노출된다(Rust include_str! 대조면)",
          lf.get("primitive") == P,
          "" if lf.get("primitive") == P else ("fixture ≠ 상수" if lf else "fixture 부재"))
    check("8a′ fixture 가 Rust 파리티 미착지를 정직 고지한다",
          str(lf.get("rust_parity_status", "")).startswith("not_tested:"),
          repr(lf.get("rust_parity_status"))[:90])
    base_sock = "/Users/x/.local/state/cys/cys.sock"
    dept_sock = "/Users/x/.local/state/cys-dept-d1/cys.sock"
    check("8b 선언 stem·ext 가 실제 경로 규약과 일치(base 도 접미)",
          os.path.basename(LANE.lane_state_path("lock", base_sock))
          == P["path_stem"] + "-base" + P["path_ext"], LANE.lane_state_path("lock", base_sock))
    check("8c 레인마다 다른 락 파일(교차 배제가 레인을 넘지 않는다)",
          LANE.lane_state_path("lock", base_sock) != LANE.lane_state_path("lock", dept_sock))
    check("8d 바이트 범위 [0,1) · 비차단 배타 모드 선언",
          (P["byte_offset"], P["byte_length"], P["mode"]) == (0, 1, "exclusive-nonblocking"),
          repr((P["byte_offset"], P["byte_length"], P["mode"])))
    check("8e posix 범위 개념 비대칭이 정직하게 고지된다(flock=파일 전체)",
          P["posix_scope"] == "whole-file" and P["range_is_windows_axis"] is True)
    for lit, why in (
        ("fd = os.open(self.path, os.O_CREAT | os.O_RDWR, 0o644)", "열기 플래그·모드"),
        ("_fcntl.flock(fd, _fcntl.LOCK_EX | _fcntl.LOCK_NB)", "posix 비차단 배타"),
        ("_msvcrt.locking(fd, _msvcrt.LK_NBLCK, 1)", "windows 1바이트 비차단"),
        ("os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o644)", "pidfile 폴백"),
    ):
        check("8f 락 원시 구현 핀(%s)" % why, lit in src_lock, lit[:50])
    os_lock_body = src_lock[src_lock.index("def _try_os_lock"):src_lock.index("def _try_pidfile")]
    check("8g windows 획득 경로에 lseek 이 없다 = 오프셋 0 이 전제(선언과 정합)",
          "os.lseek" not in os_lock_body)
    check("8h 락 파일 본문 키 4종(보유자 신원 — 스테일 회수의 근거)",
          P["holder_blob_keys"] == ["pid", "started", "owner", "host"]
          and '"pid": os.getpid(), "started": time.time()' in src_lock)
    check("8i bootstrap 소비부가 이 owner 로 잠근다",
          'FileLock(lock_path, owner="%s", blocking=False)' % P["holder_owner"] in src_bs)

    PROBE = (
        "import sys, os, time\n"
        "sys.dont_write_bytecode = True\n"
        "sys.path.insert(0, sys.argv[1])\n"
        "import javis_lock as L\n"
        "lk = L.FileLock(sys.argv[2], owner='javis_bootstrap')\n"
        "t0 = time.time()\n"
        "st = lk.acquire()\n"
        "print('%s|%s|%.3f' % (st, lk.backend, time.time() - t0)); sys.stdout.flush()\n"
        "if st == L.ACQUIRED and len(sys.argv) > 3: time.sleep(float(sys.argv[3]))\n"
    )
    for backend in ("fcntl", "pidfile"):
        lpath = os.path.join(state, "bootstrap-base-%s.lock" % backend)
        env = base_env(CYS_LOCK_BACKEND=backend)
        holder = subprocess.Popen([PY, "-c", PROBE, BIN, lpath, "3"], env=env,
                                  stdout=subprocess.PIPE, text=True)
        first = (holder.stdout.readline() or "").strip()
        t0 = time.time()
        rival = subprocess.run([PY, "-c", PROBE, BIN, lpath], env=env, capture_output=True,
                               text=True, timeout=30)
        waited = time.time() - t0
        holder.kill()
        holder.wait(timeout=10)
        rv = (rival.stdout or "").strip().split("|")
        check("8j[%s] 선행 프로세스가 락을 잡는다" % backend,
              first.startswith("acquired|" + backend), first)
        check("8k[%s] 후속 프로세스는 busy 로 **실패**한다(상호 배제 실측)" % backend,
              len(rv) > 1 and rv[0] == "busy", (rival.stdout or rival.stderr)[-160:])
        check("8l[%s] 그리고 **기다리지 않는다**(비차단 — 부트 훅 블록 금지)" % backend,
              waited < 2.5 and float(rv[2] if len(rv) > 2 else 9) < 1.0,
              "elapsed=%.2fs probe=%s" % (waited, rv[2] if len(rv) > 2 else "?"))
        after = subprocess.run([PY, "-c", PROBE, BIN, lpath], env=env, capture_output=True,
                               text=True, timeout=30)
        check("8m[%s] 보유자 종료 후에는 획득된다(영구 거부 창 0)" % backend,
              (after.stdout or "").strip().startswith("acquired|"), (after.stdout or "")[-120:])
    SF = ("import sys\nsys.dont_write_bytecode = True\nsys.path.insert(0, sys.argv[1])\n"
          "import javis_bootstrap as B\n"
          "r = B._acquire_singleflight()\n"
          "print('%s|%s' % (r, B._singleflight_path())); sys.stdout.flush()\n"
          "if r: import time; time.sleep(float(sys.argv[2]))\n")
    fresh()
    h2 = subprocess.Popen([PY, "-c", SF, BIN, "3"], env=base_env(), stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, text=True)
    l1 = (h2.stdout.readline() or "").strip()
    r2 = subprocess.run([PY, "-c", SF, BIN, "0"], env=base_env(), capture_output=True,
                        text=True, timeout=30)
    h2.kill()
    h2.wait(timeout=10)
    l2 = (r2.stdout or "").strip()
    check("8n 생산 진입점: 1번째 _acquire_singleflight 는 획득", l1.startswith("True|"), l1)
    check("8o 생산 진입점: 2번째는 None(no-op → exit 11 경로)", l2.startswith("None|"),
          l2 or r2.stderr[-160:])
    check("8p 둘이 **같은 파일**을 다툰다 = 선언된 경로",
          l1.split("|", 1)[-1] == l2.split("|", 1)[-1] == os.path.join(state,
                                                                       "bootstrap-base.lock"),
          l1.split("|", 1)[-1])
    check("8q 락 파일 생성 모드 8진 병기가 실제 값과 일치(fixture 독자 오독 차단)",
          int(P["create_mode_octal"], 8) == P["create_mode"],
          repr((P["create_mode_octal"], P["create_mode"])))

    # ── ②i(자리 이동): 디스크 fixture ↔ SOT 1:1 — 모든 fixture 쓰기가 끝난 뒤에 센다 ──
    on_disk = {f for f in os.listdir(FIXTURES) if f.endswith(".json")}
    want_disk = {v["golden"] for v in spec_cases.values()} | set(SPEC.get("_aux_files") or {})
    check("2i 디스크 fixture ↔ SOT 1:1(고아·누락 0)", on_disk == want_disk,
          "차분: %s" % sorted(on_disk ^ want_disk))

    # ═══════════ ⑨ ★음성 대조 — 변조 사본이 이 검체를 통과하지 못한다 ═══════════
    mut_dir = os.path.join(root, "mutbin")
    os.makedirs(mut_dir)
    for name in ("javis_lane.py", "javis_lock.py"):
        shutil.copy2(os.path.join(BIN, name), os.path.join(mut_dir, name))

    def mutate_bootstrap(old, new):
        body = src_bs.replace(old, new, 1)
        with io.open(os.path.join(mut_dir, "javis_bootstrap.py"), "w",
                     encoding="utf-8", newline="\n") as f:
            f.write(body)
        return body != src_bs

    ok = mutate_bootstrap('"result": {"ok": None, "state": "running", "run_id": self.run_id,',
                          '"result": {"ok": None, "state": "started", "run_id": self.run_id,')
    check("9a 변조① 앵커 실재(running 선기록)", ok)
    fresh()
    m = drive("running", bindir=mut_dir)
    check("9b 변조①은 running 골든을 통과하지 못한다",
          normalize(m.get("doc") or {}) != _load(golden_path("running")),
          repr((m.get("doc") or {}).get("result"))[:120])

    ok = mutate_bootstrap('{"ok": None, "state": "crashed" if exc else "aborted",',
                          '{"ok": None, "state": "crashed",')
    check("9c 변조② 앵커 실재(종결 전이 융합)", ok)
    fresh()
    m = drive("aborted", bindir=mut_dir)
    check("9d 변조②는 aborted 골든을 통과하지 못한다",
          normalize(m.get("doc") or {}) != _load(golden_path("aborted")),
          repr((m.get("doc") or {}).get("result"))[:120])

    ok = mutate_bootstrap('        if state == "session_error":\n'
                          '            self._seal_session_error(exit_code)\n',
                          '        if False:\n'
                          '            self._seal_session_error(exit_code)\n')
    check("9e 변조③ 앵커 실재(미러 봉인)", ok)
    fresh()
    m = drive("session_error", bindir=mut_dir)
    check("9f 변조③은 stdout 미러를 못 낸다 → ⑤절이 FAIL 한다",
          not (m.get("stdout") or "").strip(), repr((m.get("stdout") or "")[:120]))

    ok = mutate_bootstrap("RUNNER_EXIT_ABORTED = 13 ", "RUNNER_EXIT_ABORTED = 12 ")
    check("9g 변조④ 앵커 실재(러너 exit 상수)", ok)
    rmut = subprocess.run([PY, os.path.join(mut_dir, "javis_bootstrap.py"), "--self-test"],
                          capture_output=True, text=True, env=base_env(), timeout=120)
    check("9h 변조④는 self-test 를 통과하지 못한다",
          rmut.returncode != 0 and "러너 aborted exit 상수 이탈" in (rmut.stdout + rmut.stderr),
          "rc=%d %s" % (rmut.returncode, (rmut.stdout + rmut.stderr)[-140:]))

    shutil.copy2(BOOTSTRAP_SRC_PATH, os.path.join(mut_dir, "javis_bootstrap.py"))
    mlock = src_lock.replace("                _fcntl.flock(fd, _fcntl.LOCK_EX | _fcntl.LOCK_NB)\n",
                             "                pass\n", 1)
    check("9i 변조⑤ 앵커 실재(락 비차단 실패)", mlock != src_lock)
    with io.open(os.path.join(mut_dir, "javis_lock.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write(mlock)
    lpath = os.path.join(state, "bootstrap-mut.lock")
    env = base_env(CYS_LOCK_BACKEND="fcntl")
    h3 = subprocess.Popen([PY, "-c", PROBE, mut_dir, lpath, "3"], env=env,
                          stdout=subprocess.PIPE, text=True)
    h3.stdout.readline()
    r3 = subprocess.run([PY, "-c", PROBE, mut_dir, lpath], env=env, capture_output=True,
                        text=True, timeout=30)
    h3.kill()
    h3.wait(timeout=10)
    check("9j 변조⑤는 상호 배제를 잃는다 → ⑧k 가 FAIL 한다",
          (r3.stdout or "").strip().startswith("acquired|"), (r3.stdout or r3.stderr)[-140:])

    # 변조 ⑥: 생산에 **새 state 를 추가**한다 → 전수성 축(②e)이 울어야 한다. `team_complete` 가
    #         실제로 그렇게 들어왔고(명세·시뮬 미등재), 그때 아무 검체도 울지 않았다. 이 대조가
    #         없으면 "골든이 전수를 덮는다"는 문장은 검증되지 않은 주장이다.
    src_new_state = src_bs.replace('log.result(ok=True, state="completed", exit=EXIT_OK)',
                                   'log.result(ok=True, state="brand_new", exit=EXIT_OK)', 1)
    check("9k 변조⑥ 앵커 실재(새 state 추가)", src_new_state != src_bs)
    mut_states = set()
    for node in ast.walk(ast.parse(src_new_state)):
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                and node.func.attr in ("result", "fail")):
            for kw in node.keywords:
                if (kw.arg == "state" and isinstance(kw.value, ast.Constant)
                        and isinstance(kw.value.value, str)):
                    mut_states.add(kw.value.value)
    check("9l 변조⑥은 전수성 축(②e)을 통과하지 못한다 — 골든 없는 새 state 를 잡는다",
          not (mut_states <= golden_states) and "brand_new" in (mut_states - golden_states),
          repr(sorted(mut_states - golden_states)))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("BOOTLAST-GOLDEN-LOCK-PARITY-OK")
sys.exit(0)
