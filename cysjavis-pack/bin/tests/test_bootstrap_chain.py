#!/usr/bin/env python3
"""test_bootstrap_chain.py — javis_bootstrap.py 스텁 결정론 CI (BOOTSTRAP_HARDENING T0).

실 데몬·실 Claude 없이: 임시 HOME + 가짜 팩(스텁 preflight/orchestra/cys-dept) + PATH 선두
`cys` 스텁으로 부트 체인의 exit-code 계약을 핀한다. 설계 v1.1 검증 핀:
  ⓐ 부서 소켓 컨텍스트에서 전 단계 성공해도 base 마커 미생성(소켓 격리)
  ⓑ check 스텁 "N회 실패 후 성공" 시퀀스에서 부트 성공 / 전부 실패 시 상한 내 종료(retry)
  ⓒ (hook 3상태는 test_session_start_hook.py — 본 파일 아님)
  ⓓ ⑦ 호출이 promote-if-pending --request-only(비대기 인자)로 기록됨
+ 기본 매트릭스: happy path·preflight/ping/boot/claim 실패·assert-ready 3상태·롤백 불변식
  (마커·상태 파일 삭제 = 게이트 전부 현행 거동 복귀 — 부재=제약 없음).
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

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def make_env(tmp, *, claim_exit=0, ping_exit=0, boot_exit=0, preflight_exit=0,
             check_fail_times=0, check_final=0, socket="", check_needs_reviewers=False,
             br_exit=0, pack_dept=None, claim_reject_msg=None, list_rows=None):
    """임시 HOME + 가짜 팩 + 스텁 생성 → 환경 dict 반환.
    pack_dept=<부서명>: 팩을 ~/.cys/pack-dept-<부서명>에 만들고 CYS_PACK_DIR로 지정
    (증분1 레인↔팩 정합 가드 — 부서 소켓 레인은 같은 부서 팩 페어링 필수·불일치=exit 8).
    claim_reject_msg=<str>: claim-role 거부 시 데몬이 stderr로 내는 사유 문자열(step ③ v3 tie-break용).
      기본값=일반 좌석 경합 메시지. list_rows=<str>: 스텁 `cys list` 가 낼 탭 구분 행(없으면 빈 출력)."""
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys",
                        "pack-dept-%s" % pack_dept if pack_dept else "pack")
    bindir = os.path.join(tmp, "stubbin")
    for d in (os.path.join(pack, "bin"), bindir):
        os.makedirs(d, exist_ok=True)

    def w(path, body, mode=0o755):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        os.chmod(path, mode)

    # step ③ v3 tie-break 재현: 거부 사유·list 출력을 파일로 주입(셸 escaping 회피)
    w(os.path.join(tmp, "claim_reject.msg"),
      (claim_reject_msg or "claim_denied: privileged role held by live surface") + "\n", 0o644)
    if list_rows is not None:
        w(os.path.join(tmp, "list.out"), list_rows if list_rows.endswith("\n") else list_rows + "\n", 0o644)

    # 스텁 cys — 서브커맨드별 exit·호출 기록(calls.log). claim 거부문·list 출력은 파일에서 읽는다.
    w(os.path.join(bindir, "cys"), (
        "#!/bin/sh\n"
        "echo \"cys $@\" >> \"%s/calls.log\"\n"
        "case \"$1\" in\n"
        "  ping) exit %d;;\n"
        "  claim-role) [ %d -ne 0 ] && cat \"%s/claim_reject.msg\" >&2; exit %d;;\n"
        "  boot) exit %d;;\n"
        "  list) [ -f \"%s/list.out\" ] && cat \"%s/list.out\"; exit 0;;\n"
        "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
        "esac\nexit 0\n") % (tmp, ping_exit, claim_exit, tmp, claim_exit, boot_exit, tmp, tmp))
    # 스텁 preflight
    w(os.path.join(pack, "bin", "javis_preflight.py"),
      "import sys; sys.exit(%d)\n" % preflight_exit, 0o644)
    # 스텁 orchestra — 서브커맨드 분기: boot-reviewers=마커 생성(④-b 재현), check=카운터
    # (+needs_reviewers면 마커 없을 때 실패 — "cys boot만으론 리뷰어 0" 시나리오).
    w(os.path.join(pack, "bin", "javis_orchestra.py"), (
        "import os,sys\n"
        "mode=sys.argv[1] if len(sys.argv)>1 else ''\n"
        "open('%s/orch.log','a').write(mode+'\\n')\n"
        "if mode=='boot-reviewers':\n"
        "    open('%s/reviewers.flag','w').write('1')\n"
        "    sys.exit(%d)\n"
        "if mode!='check': sys.exit(0)\n"
        "c='%s/check.count'\n"
        "n=int(open(c).read()) if os.path.exists(c) else 0\n"
        "open(c,'w').write(str(n+1))\n"
        "if %d and not os.path.exists('%s/reviewers.flag'): sys.exit(1)\n"
        "sys.exit(1 if n < %d else %d)\n")
      % (tmp, tmp, br_exit, tmp, 1 if check_needs_reviewers else 0, tmp,
         check_fail_times, check_final), 0o644)
    # 스텁 cys-dept — 인자 기록
    w(os.path.join(pack, "bin", "cys-dept"),
      "#!/bin/sh\necho \"cys-dept $@\" >> \"%s/calls.log\"\nexit 0\n" % tmp)

    env = dict(os.environ)
    # ★_surface_id_env 우선순위(AITERM_→JAVIS_→CYS_)에서 CYS_SURFACE_ID=7이 권위를 갖도록 상위 키 제거
    env.pop("AITERM_SURFACE_ID", None)
    env.pop("JAVIS_SURFACE_ID", None)
    env.update({"HOME": home, "PATH": bindir + os.pathsep + env.get("PATH", ""),
                "CYS_BOOT_CHECK_RETRIES": "4", "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                "CYS_SURFACE_ID": "7"})
    env.pop("CYS_PACK_DIR", None)
    if pack_dept:
        env["CYS_PACK_DIR"] = pack
    env.pop("CYS_BOOT_GATE", None)
    if socket:
        env["CYS_SOCKET"] = socket
    else:
        env.pop("CYS_SOCKET", None)
    return env, home


def run(env, *args):
    r = subprocess.run([PY, SCRIPT] + list(args), capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stdout, r.stderr


def marker_path(home):
    return os.path.join(home, ".cys", ".master-bootstrapped")


def calls(tmp):
    p = os.path.join(tmp, "calls.log")
    return open(p, encoding="utf-8").read() if os.path.exists(p) else ""


# ── 1. happy path: exit 0 · 마커 생성 · ⑧ JSON · ⑦ --request-only(ⓓ) ──
tmp = tempfile.mkdtemp(prefix="boot-t1-")
env, home = make_env(tmp)
code, out, err = run(env)
check("1a happy exit 0", code == 0, "exit=%d err=%s" % (code, err[-200:]))
check("1b 마커 생성", os.path.exists(marker_path(home)))
m = json.load(open(marker_path(home), encoding="utf-8")) if os.path.exists(marker_path(home)) else {}
check("1c 마커에 orchestra 증거", m.get("orchestra_check") == "exit 0")
try:
    summary = json.loads(out.strip().splitlines()[-1])
except Exception:
    summary = {}
check("1d ⑧ 기계 요약 JSON(ok)", summary.get("ok") is True)
check("1e ⓓ ⑦ 비대기 인자", "promote-if-pending --request-only" in calls(tmp))
check("1f boot-last 단계 누적",
      bool((json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"),
                           encoding="utf-8")) or {}).get("steps")))
check("1g ★R12 진행 신호(침묵 창 방지 — 단계 시작 stderr)",
      "[bootstrap] ①" in err and "[bootstrap] ⑤" in err, err[:200])
src_boot = open(SCRIPT, encoding="utf-8").read()
check("1h ★R6 ④-b timeout≥320(2슬롯×130s 순차 — 스텁은 즉시 반환이라 정적 핀·실기 미검증 정직 표기)",
      "timeout=320" in src_boot)
shutil.rmtree(tmp)

# ── 2. ⓐ 부서 소켓 컨텍스트: 성공해도 base 마커 미생성·⑦ 생략 ──
# 현행 부서 규약(증분1): 소켓은 디렉토리형 <state>/cys-dept-<name>/cys.sock + 같은 부서 팩
# (pack-dept-<name>) 페어링 — 구식 평면 소켓/메인 팩 조합은 레인↔팩 가드 exit 8
# (그 가드 자체는 test_lane_isolation_v1.py t3가 핀). 신교리(증분2): 부서 레인 팀 기동은
# CEO 티켓 필수 — "부서 부트 성공" 핀을 두 경로로 분해한다.
# 2-i) 티켓 부재 → 부서장 단독 각성 강등(exit 0·팀 기동 ④⑤ 생략·마커 무접촉·⑦ 생략)
tmp = tempfile.mkdtemp(prefix="boot-t2-")
dept_sock = os.path.join(tmp, "state", "cys-dept-dept-3", "cys.sock")
env, home = make_env(tmp, socket=dept_sock, pack_dept="dept-3")
code, out, err = run(env)
check("2a-i 티켓 부재 부서 부트 exit 0(단독 각성 강등)", code == 0,
      "exit=%d err=%s" % (code, err[-200:]))
try:
    summary2 = json.loads(out.strip().splitlines()[-1])
except Exception:
    summary2 = {}
check("2a-ii 단독 각성 명시(solo_awakening·dept)",
      summary2.get("solo_awakening") is True and summary2.get("dept") == "dept-3")
check("2a-iii 팀 기동 생략(cys boot 미호출)", "cys boot" not in calls(tmp))
check("2b ⓐ base 마커 미생성(소켓 격리)", not os.path.exists(marker_path(home)))
check("2c 부서 컨텍스트 ⑦ 생략", "promote-if-pending" not in calls(tmp))
shutil.rmtree(tmp)

# 2-ii) 티켓 발급(base 레인 issue-ticket) → 부서 팀 기동 경로: ④⑤ 수행·exit 0·마커는 여전히 무접촉
tmp = tempfile.mkdtemp(prefix="boot-t2t-")
dept_sock = os.path.join(tmp, "state", "cys-dept-dept-3", "cys.sock")
env, home = make_env(tmp, socket=dept_sock, pack_dept="dept-3")
env_base = dict(env)
env_base.pop("CYS_SOCKET", None)  # 발급은 base 레인에서만 허용
code, out, err = run(env_base, "issue-ticket", "--dept", "dept-3")
check("2d 티켓 발급 exit 0(base 레인)", code == 0, "exit=%d err=%s" % (code, err[-200:]))
code, out, err = run(env)
check("2e 티켓 有 부서 팀 기동 exit 0", code == 0, "exit=%d err=%s" % (code, err[-200:]))
check("2f 팀 기동 수행(cys boot 호출)", "cys boot" in calls(tmp))
check("2g 티켓 1회성 소비(.used rename)",
      os.path.exists(os.path.join(home, ".cys", "state", "dept-boot-tickets",
                                  "dept-3.ticket.used")))
check("2h 팀 기동이어도 base 마커 무접촉", not os.path.exists(marker_path(home)))
check("2i 부서 컨텍스트 ⑦ 생략", "promote-if-pending" not in calls(tmp))
shutil.rmtree(tmp)

# ── 2w. windows pipe 이름도 base로 인정 ──
tmp = tempfile.mkdtemp(prefix="boot-t2w-")
env, home = make_env(tmp, socket=r"\\.\pipe\cys")
code, out, err = run(env)
check("2w pipe cys=base(마커 생성)", code == 0 and os.path.exists(marker_path(home)))
shutil.rmtree(tmp)

# ── 3. ⓑ check 3회 실패 후 성공 → 부트 성공(retry) ──
tmp = tempfile.mkdtemp(prefix="boot-t3-")
env, home = make_env(tmp, check_fail_times=3, check_final=0)
code, out, err = run(env)
check("3a ⓑ retry 후 성공", code == 0, "exit=%d" % code)
check("3b 마커 생성", os.path.exists(marker_path(home)))
shutil.rmtree(tmp)

# ── 4. ⓑ check 전부 실패 → exit 6·마커 무·시도수=상한 ──
tmp = tempfile.mkdtemp(prefix="boot-t4-")
env, home = make_env(tmp, check_fail_times=99)
code, out, err = run(env)
check("4a check 최종 실패 exit 6", code == 6, "exit=%d" % code)
check("4b 마커 미생성", not os.path.exists(marker_path(home)))
attempts = int(open(os.path.join(tmp, "check.count"), encoding="utf-8").read())
check("4c 시도수=상한(4)", attempts == 4, "attempts=%d" % attempts)
shutil.rmtree(tmp)

# ── 5. claim 거부(살아있는 보유자) → exit 7·boot 미호출·마커 무 (v3 계약) ──
# 스텁 cys: (a) claim-role을 "...is held by live surface 9"로 거부(데몬 권위 거부 신호),
#          (b) list가 살아있는 master 행(surface:9·탭 구분)을 냄 → v3-a/S5 둘 다 exit7 수렴.
tmp = tempfile.mkdtemp(prefix="boot-t5-")
env, home = make_env(
    tmp, claim_exit=1,
    claim_reject_msg="claim_role denied: privileged role 'master' is held by live surface 9",
    list_rows="surface:9\trole=master\tpid=999\texited=false\t제목\t/cwd")
code, out, err = run(env)
check("5a claim 거부(live-holder) exit 7", code == 7, "exit=%d err=%s" % (code, err[-200:]))
check("5b 마커 미생성", not os.path.exists(marker_path(home)))
check("5c 거부 후 boot 미호출", "cys boot" not in calls(tmp))
check("5d 인계 지시 출력", "인계" in err)
shutil.rmtree(tmp)

# ── 5i. 신원 해석 실패 거부("surface None") + list 빈 출력 → exit 10 + "재선언" 안내 (v3-e) ──
tmp = tempfile.mkdtemp(prefix="boot-t5i-")
env, home = make_env(
    tmp, claim_exit=1,
    claim_reject_msg="claim_denied: caller (surface None) may only claim its own surface, not 6")
code, out, err = run(env)
check("5i-a 신원 거부+빈 list → exit 10", code == 10, "exit=%d err=%s" % (code, err[-200:]))
check("5i-b 마커 미생성", not os.path.exists(marker_path(home)))
check("5i-c 재선언 안내 출력(거짓 인계 아님)", "재선언" in err, err[-300:])
shutil.rmtree(tmp)

# ── 5ii. 미지 사유 거부 + list 빈 출력 → L2 fallback 경유 exit 7(구본 보수 기본값) (v3-f) ──
# 원사고의 거울상 봉인 검증: post-list가 비어도 거짓 exit10(master 부재)로 새지 않고, live/신원 힌트가
# 모두 없는 미지 사유는 검증된 L2 방어선으로 강등 → L2가 신원 힌트 없음을 확인하고 보수적 exit 7.
tmp = tempfile.mkdtemp(prefix="boot-t5ii-")
env, home = make_env(
    tmp, claim_exit=1, claim_reject_msg="claim_denied: some unknown daemon condition")
code, out, err = run(env)
check("5ii-a 미지 사유 거부 → L2 경유 exit 7", code == 7, "exit=%d err=%s" % (code, err[-200:]))
check("5ii-b 마커 미생성", not os.path.exists(marker_path(home)))
check("5ii-c 거부 후 boot 미호출", "cys boot" not in calls(tmp))
shutil.rmtree(tmp)

# ── 6. 선행 단계 실패 exit 매핑: ping=3 · boot=4 (부팅-치명 전제 위반) ──
# ★preflight는 제외(오너 2026-07-15 적대검증 adv#1): preflight FAIL은 팀 부팅을 abort하지 않는다
# (60+ 체크 중 하나만 FAIL이어도 팀 0개였던 "100% 완료" 위반 수리). 진짜 게이트는 ⑤ check. → 6b 참조.
for name, kw, want in (("ping", {"ping_exit": 1}, 3),
                       ("boot", {"boot_exit": 1}, 4)):
    tmp = tempfile.mkdtemp(prefix="boot-t6-")
    env, home = make_env(tmp, **kw)
    code, out, err = run(env)
    check("6 %s 실패 exit %d" % (name, want), code == want, "exit=%d" % code)
    check("6 %s 실패 시 마커 무" % name, not os.path.exists(marker_path(home)))
    shutil.rmtree(tmp)

# ── 6b. preflight 비치명 계약(adv#1 전사): preflight FAIL이어도 이후 단계가 green이면 부트 완료 ──
tmp = tempfile.mkdtemp(prefix="boot-t6b-")
env, home = make_env(tmp, preflight_exit=1)   # preflight만 실패, ping/claim/boot/check는 green
code, out, err = run(env)
check("6b preflight FAIL 비치명 — 체인 계속 exit 0", code == 0, "exit=%d" % code)
check("6b preflight FAIL이어도 부트 완료 마커 생성", os.path.exists(marker_path(home)))
shutil.rmtree(tmp)

# ── 7. assert-ready: 부재=5 · warn=0 · off=0 · 버전 불일치=5 · 일치=0 ──
tmp = tempfile.mkdtemp(prefix="boot-t7-")
env, home = make_env(tmp)
code, _, _ = run(env, "assert-ready")
check("7a 마커 부재 exit 5", code == 5)
env2 = dict(env); env2["CYS_BOOT_GATE"] = "warn"
check("7b 밸브 warn=0", run(env2, "assert-ready")[0] == 0)
env3 = dict(env); env3["CYS_BOOT_GATE"] = "off"
check("7c 밸브 off=0", run(env3, "assert-ready")[0] == 0)
code, _, _ = run(env)  # 정상 부트로 마커 생성(.pack-version 부재 → 'unknown' 일치)
check("7d 부트 후 assert-ready=0", code == 0 and run(env, "assert-ready")[0] == 0)
with open(os.path.join(home, ".cys", ".pack-version"), "w", encoding="utf-8") as f:
    f.write("9.9.9")  # 현재 pack_version만 전진 → 마커 stale
check("7e 버전 불일치 exit 5", run(env, "assert-ready")[0] == 5)
shutil.rmtree(tmp)

# ── 9. ④-b 리뷰어 폴백 (D-IMPL-1 재현 핀 · 산문 §0 ④-b 전사) ──
# 9a: check가 리뷰어 폴백 마커를 요구(=agy/codex 부재 기계) → ④-b가 체인에 있어야만 부트 성공.
tmp = tempfile.mkdtemp(prefix="boot-t9a-")
env, home = make_env(tmp, check_needs_reviewers=True)
code, out, err = run(env)
check("9a ④-b 폴백으로 부트 성공(agy/codex 부재 기계)", code == 0, "exit=%d" % code)
orch = open(os.path.join(tmp, "orch.log"), encoding="utf-8").read().split() if \
    os.path.exists(os.path.join(tmp, "orch.log")) else []
check("9b ④-b가 check보다 선행", orch[:1] == ["boot-reviewers"], "order=%s" % orch[:3])
shutil.rmtree(tmp)
# 9c: ④-b 자체 실패는 비중단(best-effort — 최종 게이트는 ⑤ check).
tmp = tempfile.mkdtemp(prefix="boot-t9c-")
env, home = make_env(tmp, br_exit=1)
code, out, err = run(env)
check("9c ④-b 실패해도 체인 계속(check green이면 부트 성공)", code == 0, "exit=%d" % code)
shutil.rmtree(tmp)

# ── 8. 롤백 불변식: 마커·상태 삭제 = 부재 상태로 완전 복귀(재부트로 재생성 가능) ──
tmp = tempfile.mkdtemp(prefix="boot-t8-")
env, home = make_env(tmp)
run(env)
os.remove(marker_path(home))
shutil.rmtree(os.path.join(home, ".cys", "state"))
check("8a 삭제 후 assert-ready=5(게이트=순수 추가 제약)", run(env, "assert-ready")[0] == 5)
check("8b 재부트로 재생성", run(env)[0] == 0 and os.path.exists(marker_path(home)))
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
