#!/usr/bin/env python3
"""test_bootstrap_chain.py — javis_bootstrap.py 스텁 결정론 CI (BOOTSTRAP_HARDENING T0).

실 데몬·실 Claude 없이: 임시 HOME + 가짜 팩(스텁 preflight/orchestra/cys-dept) + PATH 선두
`cys` 스텁으로 부트 체인의 exit-code 계약을 핀한다. 설계 v1.1 검증 핀:
  ⓐ 부서 소켓 컨텍스트에서 전 단계 성공해도 base 마커 미생성(소켓 격리)
  ⓑ check 스텁 "N회 실패 후 성공" 시퀀스에서 부트 성공 / 전부 실패 시 상한 내 종료(retry)
  ⓒ (hook 3상태는 test_session_start_hook.py — 본 파일 아님)
  ⓓ ⑦ 호출이 promote-if-pending --request-only(비대기 인자)로 기록됨
  ⓔ ★T10: 부서 레인 ⑦도 신호 발사 — base 팩 cys-dept 대상 + 스크럽 env(CYS_SOCKET·
     CYS_PACK_DIR·CYS_ACCOUNT_DIR 제거·CYS_NO_AUTOSTART=1) — 2s 절이 핀
+ 기본 매트릭스: happy path·preflight/ping/boot/claim 실패·assert-ready 3상태·롤백 불변식
  (마커·상태 파일 삭제 = 게이트 전부 현행 거동 복귀 — 부재=제약 없음).
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

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
             br_exit=0, pack_dept=None, boot_stdout="", boot_stderr="", status_json=None,
             records_lines=None, boot_records_out=None, notice_exit=0, boot_json_unsupported=False,
             boot_deferral_line=False):
    """임시 HOME + 가짜 팩 + 스텁 생성 → 환경 dict 반환.
    pack_dept=<부서명>: 팩을 ~/.cys/pack-dept-<부서명>에 만들고 CYS_PACK_DIR로 지정
    (증분1 레인↔팩 정합 가드 — 부서 소켓 레인은 같은 부서 팩 페어링 필수·불일치=exit 8).
    ★(B-360 R3 · option 2 · additive — every default reproduces the old stub byte-for-byte in effect):
      boot_stdout / boot_stderr = what the stub `cys boot` prints on each stream (stdout first);
      status_json = what the stub `cys status --json` prints (None = nothing, as before);
      records_lines = pre-seeded lines of ~/.cys/.kept-pane-records.jsonl (the pack state base);
      boot_records_out = the JSON text the stub `cys boot` writes to $CYS_KEPT_PANE_RECORDS_OUT (if set);
      notice_exit = the stub `cys kept-pane-notice` exit; boot_json_unsupported = the stub `cys boot --json` fails
      like an old binary's clap (exit 2, "unexpected argument") and only bare `cys boot` works. The stub logs each
      boot's deferral env to boot-env.log and each notice's stdin to notice-in.<n>.json.
      ★(REFLECTION-2 R2-P-1) boot_deferral_line = the stub `cys boot` behaves like the real binary under the deferral
      pair: it writes the out file (`[]` unless boot_records_out) and ends stderr with the one line the real Drop prints,
      `[kept-pane] notice deferred to the caller (N record(s) written to <out>)`. Off by default (the old stub)."""
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

    # 스텁 cys — 서브커맨드별 exit·호출 기록(calls.log)
    # ★(B-360 R3) boot/status/kept-pane-notice read their payloads from files in tmp (no shell quoting of JSON);
    #   an absent file means "print nothing", which is the old stub's behaviour.
    # ★(B-360 R3 · worker-7's no-JSON ruling (b)) a real `cys boot --json` that exits 0 ALWAYS prints its JSON line
    #   (cys.rs run_boot); exit 0 with an unreadable stdout now withholds the ready marker as "readiness unmeasured". So
    #   the stub's default for an exit-0 boot is the minimal real report (no roles), not silence — on af41d65c that is
    #   the same verdict as silence (not fatal, marker written), which keeps sections 1-12 meaning what they meant.
    if not boot_stdout and boot_exit == 0:
        boot_stdout = json.dumps({"roles": [], "summary": {"lock": "acquired", "fatal_failed": 0}}) + "\n"
    for name, body in (("boot.stdout", boot_stdout), ("boot.stderr", boot_stderr),
                       ("status.json", status_json), ("boot.records", boot_records_out)):
        if body:
            with open(os.path.join(tmp, name), "w", encoding="utf-8", newline="\n") as f:
                f.write(body)
    if records_lines:
        os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
        with open(os.path.join(home, ".cys", ".kept-pane-records.jsonl"), "w", encoding="utf-8",
                  newline="\n") as f:
            f.write("".join(l if l.endswith("\n") else l + "\n" for l in records_lines))
    w(os.path.join(bindir, "cys"), (
        "#!/bin/sh\n"
        "T='%(t)s'\n"
        "echo \"cys $@\" >> \"$T/calls.log\"\n"
        "case \"$1\" in\n"
        "  ping) exit %(ping)d;;\n"
        "  claim-role) [ %(claim)d -ne 0 ] && echo 'claim_denied: privileged role held by live surface' >&2; exit %(claim)d;;\n"
        "  boot)\n"
        "    if [ %(nojson)d -eq 1 ] && [ \"$2\" = '--json' ]; then\n"
        "      echo \"error: unexpected argument '--json' found\" >&2; exit 2\n"
        "    fi\n"
        "    echo \"notice=${CYS_KEPT_PANE_NOTICE-unset} out=${CYS_KEPT_PANE_RECORDS_OUT-unset}\" >> \"$T/boot-env.log\"\n"
        "    [ -f \"$T/boot.stdout\" ] && cat \"$T/boot.stdout\"\n"
        "    [ -f \"$T/boot.stderr\" ] && cat \"$T/boot.stderr\" >&2\n"
        "    if [ -n \"${CYS_KEPT_PANE_RECORDS_OUT:-}\" ] && [ -f \"$T/boot.records\" ]; then\n"
        "      cp \"$T/boot.records\" \"$CYS_KEPT_PANE_RECORDS_OUT\"\n"
        "    fi\n"
        "    if [ %(deferral)d -eq 1 ] && [ \"${CYS_KEPT_PANE_NOTICE:-}\" = caller ] && [ -n \"${CYS_KEPT_PANE_RECORDS_OUT:-}\" ]; then\n"
        "      [ -f \"$CYS_KEPT_PANE_RECORDS_OUT\" ] || printf '[]' > \"$CYS_KEPT_PANE_RECORDS_OUT\"\n"
        "      echo \"[kept-pane] notice deferred to the caller (0 record(s) written to $CYS_KEPT_PANE_RECORDS_OUT)\" >&2\n"
        "    fi\n"
        "    exit %(boot)d;;\n"
        "  status) [ -f \"$T/status.json\" ] && cat \"$T/status.json\"; exit 0;;\n"
        "  kept-pane-notice)\n"
        "    n=$(ls \"$T\" | grep -c '^notice-in\\.')\n"
        "    cat > \"$T/notice-in.$n.json\"\n"
        "    exit %(notice)d;;\n"
        "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
        "esac\nexit 0\n") % {"t": tmp, "ping": ping_exit, "claim": claim_exit, "boot": boot_exit,
                              "notice": notice_exit, "nojson": 1 if boot_json_unsupported else 0,
                              "deferral": 1 if boot_deferral_line else 0})
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


def called(tmp, prefix):
    """호출 로그에 `prefix` 로 **시작하는 줄**이 있는가 — 부분문자열 대조 금지.
    알림 본문·안내문에 명령 문자열이 들어가면 부분문자열 판정은 '호출됨'을 오보한다."""
    return any(line.startswith(prefix) for line in calls(tmp).splitlines())


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
# ★T-0147-7 W2(B9) 갱신: ④-b 외부 상한은 **하드코딩 320 이 아니라 javis_budget 파생값**이다.
#   종전 리터럴 핀은 예산 역전(외부 320 < 내부 2슬롯×2폴백×130=520)을 박제하고 있었다 —
#   테스트 갱신이 수리의 일부다. 새 핀: ①파생 소비(리터럴 0) ②파생값이 구 하한 320 **이상**
#   (내부 감액 금지 방향 — 값이 내려가면 조기실패 부활).
_ok_derived = '_budget_derived("boot_reviewers_outer_s"' in src_boot and "timeout=320" not in src_boot
_derived_val = None
try:
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    import javis_budget as _BU
    _derived_val = int(round(_BU.boot_reviewers_outer_s()))
except Exception as _e:                                    # 예산 모듈 부재 = 파생 계약 미성립
    _derived_val = None
check("1h ★R6/W2 ④-b timeout=예산 파생(≥320 — 내부 감액 금지 방향·리터럴 핀 폐기)",
      _ok_derived and _derived_val is not None and _derived_val >= 320,
      "derived=%r literal320=%r" % (_derived_val, "timeout=320" in src_boot))
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
# ★T10 계약 전환: 부서 레인 ⑦은 이제 '무조건 생략'이 아니라 **base 팩 cys-dept 로의 신호
#   발사**다(request-only) — 이 픽스처는 base 팩(~/.cys/pack)이 없으므로 무발사(부재 생략)가
#   정답이고, 신호 발사·env 스크럽 계약은 2s 절이 base 팩 스텁으로 핀한다.
check("2i 부서 컨텍스트 ⑦ base 팩 부재 — 신호 무발사(부재 생략)",
      "promote-if-pending" not in calls(tmp))
shutil.rmtree(tmp)

# ── 2s. ★T10(P3-2·R3-P03-2): 부서 레인 ⑦ 신호 발사 — base 팩 cys-dept + 스크럽 env ──
# 계약: 대상=base 팩($HOME/.cys/pack/bin/cys-dept · 부서 팩 PACK 아님) · 인자=promote-if-pending
# --request-only(무변조 신호) · env=CYS_SOCKET/CYS_PACK_DIR/CYS_ACCOUNT_DIR 스크럽 +
# CYS_NO_AUTOSTART=1(base 데몬 사망 창의 autostart 가 부서 env 를 상속해 base 소켓에 부서 팩
# 데몬을 띄우는 교차 오염 봉쇄 — R3-P03-2 치명 결함).
tmp = tempfile.mkdtemp(prefix="boot-t2s-")
dept_sock = os.path.join(tmp, "state", "cys-dept-dept-3", "cys.sock")
env, home = make_env(tmp, socket=dept_sock, pack_dept="dept-3")
env["CYS_ACCOUNT_DIR"] = "/should/be/scrubbed"   # 스크럽 검증용 오염값
base_bin = os.path.join(home, ".cys", "pack", "bin")
os.makedirs(base_bin, exist_ok=True)
with open(os.path.join(base_bin, "cys-dept"), "w", encoding="utf-8", newline="\n") as f:
    f.write("#!/bin/sh\n"
            "echo \"base-dept $* sock=${CYS_SOCKET-unset} pack=${CYS_PACK_DIR-unset}"
            " acct=${CYS_ACCOUNT_DIR-unset} noauto=${CYS_NO_AUTOSTART-unset}\""
            " >> \"%s/calls.log\"\nexit 0\n" % tmp)
os.chmod(os.path.join(base_bin, "cys-dept"), 0o755)
env_base = dict(env)
env_base.pop("CYS_SOCKET", None)
run(env_base, "issue-ticket", "--dept", "dept-3")   # 팀 기동 경로 진입(⑦ 도달)용 티켓
code, out, err = run(env)
check("2s-a 티켓 有 부서 부트 exit 0", code == 0, "exit=%d err=%s" % (code, err[-200:]))
sig = [l for l in calls(tmp).splitlines() if l.startswith("base-dept ")]
check("2s-b ⑦ 신호 발사(base 팩 cys-dept · request-only)",
      len(sig) == 1 and "promote-if-pending --request-only" in sig[0],
      "sig=%r" % sig)
line = sig[0] if sig else ""
check("2s-c 스크럽: CYS_SOCKET 제거", "sock=unset" in line, line)
check("2s-d 스크럽: CYS_PACK_DIR 제거", "pack=unset" in line, line)
check("2s-e 스크럽: CYS_ACCOUNT_DIR 제거", "acct=unset" in line, line)
check("2s-f autostart 봉쇄: CYS_NO_AUTOSTART=1", "noauto=1" in line, line)
check("2s-g base 마커 무접촉 유지", not os.path.exists(marker_path(home)))
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

# ── 5. claim 거부(폴백 비활성 CYS_DEPT_FALLBACK=0) → 구계약: exit 7·boot 미호출·마커 무 ──
# ★2026-08 위계 폴백(D1ⓐ) 도입 후 이 케이스는 '킬스위치 off 시 구계약 보존'의 핀이다.
tmp = tempfile.mkdtemp(prefix="boot-t5-")
env, home = make_env(tmp, claim_exit=1)
env["CYS_DEPT_FALLBACK"] = "0"
code, out, err = run(env)
check("5a claim 거부(폴백 off) exit 7", code == 7, "exit=%d" % code)
check("5b 마커 미생성", not os.path.exists(marker_path(home)))
check("5c 거부 후 boot 미호출", "cys boot" not in calls(tmp))
check("5d 인계 지시 출력", "인계" in err)
shutil.rmtree(tmp)

# ── 5-fb. claim 거부 + base 레인(unix) → 위계 폴백: 부서 자동 생성·부서장·팀 기동 (D1ⓐ·D2·D3) ──
# 스텁 계약: cys-dept allocate → dept-7 · `<name> --` env 주입 실행 · launch-agent 는 master.flag
# 를 만들어 이후 `cys list` 가 살아있는 master 를 보고하게 한다(멱등 재선언 검증 재료).
def make_dept_fb_stubs(tmp, env, home):
    bindir = os.path.join(tmp, "stubbin")
    pack = os.path.join(home, ".cys", "pack")
    dept_pack = os.path.join(home, ".cys", "pack-dept-dept-7")
    dept_sock = os.path.join(tmp, "cys-dept-dept-7.sock")
    os.makedirs(os.path.join(dept_pack, "bin"), exist_ok=True)
    def w(path, body, mode=0o755):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        os.chmod(path, mode)
    # 부서 팩: 준비 폴링 대상 + 팀 단계(orchestra) 스텁
    w(os.path.join(dept_pack, "bin", "javis_bootstrap.py"), "# dept pack ready\n", 0o644)
    w(os.path.join(dept_pack, "bin", "javis_orchestra.py"),
      "import sys\nopen('%s/orch-dept.log','a').write(' '.join(sys.argv[1:])+'\\n')\nsys.exit(0)\n" % tmp,
      0o644)
    # cys-dept 스텁: allocate=이름 발급 · sock=소켓 경로 · `<name> -- <cmd>`=env 주입 exec
    w(os.path.join(pack, "bin", "cys-dept"), (
        "#!/bin/sh\n"
        "echo \"cys-dept $@\" >> \"%(t)s/calls.log\"\n"
        "case \"$1\" in\n"
        "  allocate) echo '[cys-dept] spawn...' >&2; echo dept-7; exit 0;;\n"
        "  sock) echo '%(s)s'; exit 0;;\n"
        "  dept-7) shift; [ \"$1\" = '--' ] && shift; CYS_SOCKET='%(s)s' CYS_PACK_DIR='%(p)s' \"$@\"; exit $?;;\n"
        "esac\nexit 0\n") % {"t": tmp, "s": dept_sock, "p": dept_pack})
    # cys 스텁 확장: launch-agent → surface:42 + master.flag · status --json → 부서장 판정 소스
    # (★P0 교정 대응: _dept_master_alive 는 이제 status --json 의 agent_alive/seat 를 본다.
    #  master.flag=agent 살아있는 부서장 · master-shell.flag=allocate 직후의 role=master 빈 셸)
    w(os.path.join(bindir, "cys"), (
        "#!/bin/sh\n"
        "echo \"cys $@ [sock=${CYS_SOCKET:-base}]\" >> \"%(t)s/calls.log\"\n"
        "case \"$1\" in\n"
        "  ping) exit 0;;\n"
        "  claim-role) echo 'claim_denied: privileged role held by live surface' >&2; exit 7;;\n"
        "  launch-agent) echo 'surface:42'; touch \"%(t)s/master.flag\"; exit 0;;\n"
        "  status)\n"
        "    if [ -z \"${CYS_SOCKET:-}\" ]; then\n"
        # ★base 레인(2026-08-16 L3 가드 대응): 위계 폴백의 **전제**는 '살아있는 master 보유자가
        #   이미 있다'이다. 종전 스텁은 claim 거부만 흉내내고 그 전제는 모사하지 않아, 전제 없이
        #   부서가 만들어지는 실제 결함(role=- 인 채 dept 증식)을 재현조차 못 했다. 이제 base
        #   레인 status 는 그 전제를 사실로 제공한다(전제 부재 시나리오의 핀은 5fb-p).
        "      echo '{\"surfaces\":[{\"surface_id\":9,\"role\":\"master\",\"exited\":false,\"agent\":\"claude\",\"agent_alive\":true,\"seat\":\"occupied\"}]}'\n"
        "    elif [ -f \"%(t)s/master.flag\" ]; then\n"
        "      echo '{\"surfaces\":[{\"surface_id\":42,\"role\":\"master\",\"exited\":false,\"agent\":\"claude\",\"agent_alive\":true,\"seat\":\"occupied\"}]}'\n"
        "    elif [ -f \"%(t)s/master-shell.flag\" ]; then\n"
        "      echo '{\"surfaces\":[{\"surface_id\":41,\"role\":\"master\",\"exited\":false,\"agent\":null,\"agent_alive\":null,\"seat\":\"empty\"}]}'\n"
        "    else\n"
        "      echo '{\"surfaces\":[]}'\n"
        "    fi\n"
        "    exit 0;;\n"
        "  list) [ -f \"%(t)s/master.flag\" ] && echo 'surface:42	role=master	pid=1	exited=false	x'; exit 0;;\n"
        "  boot) exit 0;;\n"
        "  send) exit 0;;\n"
        "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
        "esac\nexit 0\n") % {"t": tmp})
    return dept_sock, dept_pack

tmp = tempfile.mkdtemp(prefix="boot-t5fb-")
env, home = make_env(tmp, claim_exit=1)
# ★폭주 봉인 ⓑ 실강제(2026-08-12) 대응: 폴백은 훅이 human 판정 직후 스폰에 싣는 마커를
# 요구한다 — 이 시나리오는 훅 발화 경로를 모사한다(마커 부재 경로의 핀은 5fb-n).
env["CYS_DECL_ORIGIN"] = "hook-human"
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
code, out, err = run(env)
check("5fb-a 폴백 성공 exit 0", code == 0, "exit=%d err=%s" % (code, err[-300:]))
try:
    summary = json.loads(out.strip().splitlines()[-1])
except Exception:
    summary = {}
check("5fb-b 최종 JSON state=dept_fallback", summary.get("state") == "dept_fallback", out[-300:])
check("5fb-c 부서명 dept-7", summary.get("dept") == "dept-7")
c = calls(tmp)
check("5fb-d allocate 호출", "cys-dept allocate" in c)
check("5fb-e launch-agent master(부서 소켓)", "launch-agent --role master --agent claude [sock=%s]" % dept_sock in c)
check("5fb-f 팀 boot(부서 소켓)", "boot --json [sock=%s]" % dept_sock in c)
ticket = os.path.join(home, ".cys", "state", "dept-boot-tickets", "dept-7.ticket")
check("5fb-g 티켓 발급(D3)·미소비", os.path.exists(ticket), ticket)
check("5fb-h base 마커 미생성(부서 폴백은 base 부트가 아니다)", not os.path.exists(marker_path(home)))
bl = json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"), encoding="utf-8"))
check("5fb-i boot-last state=dept_fallback·ok=null",
      bl.get("result", {}).get("state") == "dept_fallback" and bl.get("result", {}).get("ok") is None,
      json.dumps(bl.get("result", {}), ensure_ascii=False)[:200])
# 멱등 재선언: 같은 surface 재실행 → allocate 재호출 없음·launch-agent 재호출 없음(생존 master)
code2, out2, err2 = run(env)
c2 = calls(tmp)
check("5fb-j 재선언 exit 0(멱등)", code2 == 0, "exit=%d" % code2)
check("5fb-k allocate 1회(재생성 없음)", c2.count("cys-dept allocate") == 1, "count=%d" % c2.count("cys-dept allocate"))
check("5fb-l launch-agent 1회(생존 master 존중)",
      c2.count("launch-agent --role master") == 1, "count=%d" % c2.count("launch-agent --role master"))
shutil.rmtree(tmp)

# ── 5-fb-m. ★P0 회귀 핀(2026-08-12 R2 확정): allocate 가 만든 role=master '빈 셸'은
#    살아있는 부서장이 아니다 — 빈 셸을 생존으로 오판해 launch-agent 를 생략하면 신규 부서의
#    부서장(claude)이 영영 안 뜬다. 빈 셸(agent 없음·seat empty)만 있는 부서에서 launch-agent
#    가 반드시 발화해야 한다. ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbm-")
env, home = make_env(tmp, claim_exit=1)
env["CYS_DECL_ORIGIN"] = "hook-human"
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
open(os.path.join(tmp, "master-shell.flag"), "w").close()  # allocate 직후의 빈 셸 상태 모사
code, out, err = run(env)
check("5fb-m1 빈 셸 부서 폴백 성공 exit 0", code == 0, "exit=%d err=%s" % (code, err[-300:]))
check("5fb-m2 빈 셸 위에 launch-agent 발화(P0 핀 — 생략 금지)",
      "launch-agent --role master" in calls(tmp), calls(tmp)[-400:])
shutil.rmtree(tmp)

# ── 5-fb-n. ★폭주 봉인 ⓑ 실강제 핀(2026-08-12): 직접 실행(CYS_DECL_ORIGIN 마커 무 — CLAUDE.md
#    §0 폴백·기계 배달 경로)은 부서 자동 생성으로 이어지지 않는다(구계약 exit 7·부서 미생성). ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbn-")
env, home = make_env(tmp, claim_exit=1)
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
env.pop("CYS_DECL_ORIGIN", None)
code, out, err = run(env)
check("5fb-n1 마커 무 → 폴백 비적용 exit 7", code == 7, "exit=%d" % code)
check("5fb-n2 부서 생성 미시도(폭주 봉인)", "cys-dept allocate" not in calls(tmp))
shutil.rmtree(tmp)

# ── 5-fb-o. ★오염 차단 핀(2026-08-12 R2 확정): 폴백 실패는 base 건강 기록(boot-last)을
#    ok:false 로 덮지 않는다 — base master 는 건강하다(그래서 정당거부가 났다). 실패는
#    ok=null·state=dept_fallback_failed 로 귀속만 남긴다(§0 재부트 churn 차단). ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbo-")
env, home = make_env(tmp, claim_exit=1)
env["CYS_DECL_ORIGIN"] = "hook-human"
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
_cd = os.path.join(home, ".cys", "pack", "bin", "cys-dept")
with open(_cd, "w", encoding="utf-8", newline="\n") as f:
    f.write("#!/bin/sh\necho 'allocate 실패 모사' >&2\nexit 1\n")
os.chmod(_cd, 0o755)
code, out, err = run(env)
check("5fb-o1 allocate 실패 → exit 4(EXIT_BOOT)", code == 4, "exit=%d" % code)
bl = json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"), encoding="utf-8"))
check("5fb-o2 boot-last ok=null·state=dept_fallback_failed(오염 차단)",
      bl.get("result", {}).get("ok") is None
      and bl.get("result", {}).get("state") == "dept_fallback_failed",
      json.dumps(bl.get("result", {}), ensure_ascii=False)[:200])
shutil.rmtree(tmp)

# ── 5-fb-p. ★L3 실측 가드 핀(2026-08-16 현장 결함 — "없는 master 를 있다고 믿고 부서를 만든다") ──
#    실사고: 훅이 부트를 세션 분리로 발화 → 부트가 재부모화돼 조상 체인 단절 → 데몬이 claim 을
#    '발신 pane 미해석'으로 거부 → 종전 데몬은 그 거부를 '살아있는 보유자 있음'과 **같은 코드**로
#    냈고 → 부트가 정당거부로 읽어 **부서를 자동 생성**했다(role=- 인 채 dept-N 증식).
#    L1(데몬 코드 분리)이 그 오역을 닫았지만, 이 가드는 **독립적으로** 폴백의 전제를 직접 잰다 —
#    구 데몬 × 신 팩 스큐에서도, 앞으로 rc 7 로 접히는 다른 경로가 생겨도 전제 없는 스폰을 막는다.
tmp = tempfile.mkdtemp(prefix="boot-t5fbp-")
env, home = make_env(tmp, claim_exit=1)
env["CYS_DECL_ORIGIN"] = "hook-human"
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
# base 레인 status 가 **살아있는 master 보유자 없음**을 보고하게 덮어쓴다(현장 결함 재현).
_cys_stub = os.path.join(tmp, "stubbin", "cys")
with open(_cys_stub, "w", encoding="utf-8", newline="\n") as f:
    f.write("#!/bin/sh\n"
            "echo \"cys $@ [sock=${CYS_SOCKET:-base}]\" >> \"%s/calls.log\"\n"
            "case \"$1\" in\n"
            "  ping) exit 0;;\n"
            "  claim-role) echo 'claim_denied: privileged role held by live surface' >&2; exit 7;;\n"
            "  status) echo '{\"surfaces\":[]}'; exit 0;;\n"
            "  list) exit 0;;\n"
            "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
            "esac\nexit 0\n" % tmp)
os.chmod(_cys_stub, 0o755)
code, out, err = run(env)
check("5fb-p1 전제 미확인 → 부서 자동 생성 미진입(exit 10 세션 배선 오류)", code == 10, "exit=%d" % code)
check("5fb-p2 allocate 미호출(없는 master 로 부서 만들지 않는다)",
      not called(tmp, "cys-dept allocate"), calls(tmp)[-300:])
bl = json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"), encoding="utf-8"))
check("5fb-p3 boot-last ok=null·state=session_error(공유 기록 오염 0)",
      bl.get("result", {}).get("ok") is None
      and bl.get("result", {}).get("state") == "session_error",
      json.dumps(bl.get("result", {}), ensure_ascii=False)[:200])
shutil.rmtree(tmp)

# ── 5-fb-q/r. ★L2 선행 claim 소비 핀(2026-08-16): 훅이 **조상 체인이 온전한 시점에** claim 을
#    끝내고 판정을 env(CYS_CLAIM_RC)로 넘긴다. 부트는 그것을 소비하고 claim 을 다시 치지 않는다
#    (분리된 이 프로세스가 다시 claim 하면 언제나 신원 미해석으로 거부된다 — 결함의 본체).
def bind_claim(env, rc, out="(선행 claim 출력)"):
    """훅이 싣는 **결박된** 선행 claim 판정(rc + surface 귀속 + 신선도).
    결박 없는 rc 는 부트가 무시해야 한다(아래 5fb-s 핀)."""
    env["CYS_CLAIM_RC"] = str(rc)
    env["CYS_CLAIM_OUT"] = out
    env["CYS_CLAIM_SID"] = env.get("CYS_SURFACE_ID", "7")
    env["CYS_CLAIM_AT"] = str(int(time.time()))
    return env


tmp = tempfile.mkdtemp(prefix="boot-t5fbq-")
env, home = make_env(tmp)
bind_claim(env, 0, "registered: master → surface:7")
code, out, err = run(env)
check("5fb-q1 선행 claim rc0 소비 → 체인 계속 exit 0", code == 0, "exit=%d err=%s" % (code, err[-300:]))
check("5fb-q2 claim-role 재호출 없음(중복 claim 금지)",
      "cys claim-role" not in calls(tmp), calls(tmp)[-300:])
shutil.rmtree(tmp)

tmp = tempfile.mkdtemp(prefix="boot-t5fbr-")
env, home = make_env(tmp)
env["CYS_DECL_ORIGIN"] = "hook-human"
dept_sock, dept_pack = make_dept_fb_stubs(tmp, env, home)
bind_claim(env, 6)   # 발신 신원 미확정 — '다른 pane 이 master' 가 아니다
code, out, err = run(env)
check("5fb-r1 선행 claim rc6 → 세션 배선 오류 exit 10", code == 10, "exit=%d" % code)
check("5fb-r2 rc6 은 부서 자동 생성으로 이어지지 않는다",
      not called(tmp, "cys-dept allocate"), calls(tmp)[-300:])
shutil.rmtree(tmp)

# ── 5-fb-s. ★판정 결박 핀(2026-08-16 · L2 가 만든 신규 실패 클래스 봉인): 정수처럼 보이는 env
#    하나로 claim 을 건너뛰면, 사용자 셸·래퍼에 남은 값이 **치지도 않은 claim 을 '실측'으로**
#    boot-last 에 적는다(CS-3 보고=실측 위반). 결박(같은 surface·300s 신선도)이 없거나 어긋나면
#    무시하고 직접 claim 해야 한다 — 하위호환이 곧 안전한 기본값이다.
for _tag, _mut, _why in (
    ("s1 결박 무(sid·at 부재)", lambda e: e.update({"CYS_CLAIM_RC": "0"}), "구 훅·수동 export"),
    ("s2 타 surface 판정", lambda e: (bind_claim(e, 0), e.update({"CYS_CLAIM_SID": "99"})),
     "남의 pane 판정 유입"),
    ("s3 신선도 초과(1h 전)", lambda e: (bind_claim(e, 0), e.update(
        {"CYS_CLAIM_AT": str(int(time.time()) - 3600)})), "낡은 판정 재사용"),
    # s4: MAX<=0 가드 판별 검체 — CLAIM_AT 를 미래(60s)로 밀어 나이를 음수 skew 허용창 안에
    #     넣는다. 가드 없으면 -120<=age<0 이 참이 되어 결박(=직접 claim 부재)로 적색이 난다 —
    #     '0=소비 차단' 수동 튜닝 관용이 유계 음수 허용에 조용히 깨지지 않음을 핀한다.
    ("s4 MAX_AGE=0 소비 차단(음수 skew 창)", lambda e: (bind_claim(e, 0), e.update(
        {"CYS_CLAIM_MAX_AGE_S": "0", "CYS_CLAIM_AT": str(int(time.time()) + 60)})),
     "'0=차단' 관용 보존"),
):
    tmp = tempfile.mkdtemp(prefix="boot-t5fbs-")
    env, home = make_env(tmp)
    _mut(env)
    code, out, err = run(env)
    check("5fb-%s → 무시하고 직접 claim(%s)" % (_tag, _why),
          "cys claim-role" in calls(tmp), "calls=%s" % calls(tmp)[-200:])
    shutil.rmtree(tmp)

# ── 5-fb-t. ★P0-1 결박 신선도=런 시작 기준 핀(CLM-2 라이브락 절단): ①preflight 가 신선도 창을
#    넘게 오래 걸려도, 훅 스탬프가 **런 시작 시점**에 신선했으면 결박은 소비돼야 한다.
#    소비 시각(time.time()) 기준이던 구 계약은 이 시나리오에서 만료→직접 claim→재부모화 신원
#    미해석(rc6)→session_error 였다 — preflight sleep(12s) > CYS_CLAIM_MAX_AGE_S(10s) 로 그 경합을
#    재현하고, 신 계약(_RUN_T0 기준·나이≈스폰 지연 수백 ms)에서 결박 유지를 핀한다.
#    ★검체 전제: bind_claim 스탬프→부트 모듈 로드(_RUN_T0 캡처)가 10s 이내여야 한다(정상
#    수백 ms — 극단 콜드스타트 CI 여유분으로 창을 3s→10s 로 넓혔다). 전제 초과 시 실패 방향은
#    적색(5fb-t2 가 직접 claim 재호출을 검출 — 가짜 green 없음)이고 calls 로그로 즉시 진단된다.
tmp = tempfile.mkdtemp(prefix="boot-t5fbt-")
env, home = make_env(tmp)
# 스텁 preflight 를 '창보다 긴 지연'으로 교체 — in-run 소요 시뮬레이션(fresh 마커 없음 → 실행됨).
with open(os.path.join(home, ".cys", "pack", "bin", "javis_preflight.py"),
          "w", encoding="utf-8", newline="\n") as f:
    f.write("import sys,time; time.sleep(12); sys.exit(0)\n")
bind_claim(env, 0, "registered: master → surface:7")
env["CYS_CLAIM_MAX_AGE_S"] = "10"  # 창(10s) < preflight 소요(12s) — 구 계약이면 여기서 만료된다
code, out, err = run(env)
check("5fb-t1 preflight 지연(창 초과)에도 결박 소비 유지 → 체인 계속 exit 0",
      code == 0, "exit=%d err=%s" % (code, err[-300:]))
check("5fb-t2 claim-role 재호출 없음(런 시작 기준 나이 — in-run 소요와 무관)",
      "cys claim-role" not in calls(tmp), "calls=%s" % calls(tmp)[-200:])
shutil.rmtree(tmp)

# ── 5-fb-u. ★P0-1 유계 음수 스큐 핀: 훅 스탬프가 _RUN_T0 보다 미래(스탬프↔기동 사이 NTP 후퇴)
#    여도 허용창(-120s) 안이면 결박을 유지한다 — 음수를 미결박으로 접으면 정확히 rc6 재생산.
#    진단 1줄(stderr)은 남긴다(시계가 움직인 사실은 침묵하지 않는다).
tmp = tempfile.mkdtemp(prefix="boot-t5fbu-")
env, home = make_env(tmp)
bind_claim(env, 0, "registered: master → surface:7")
env["CYS_CLAIM_AT"] = str(int(time.time()) + 60)   # 60s 미래 — 허용창(120s) 안의 시계 후퇴
code, out, err = run(env)
check("5fb-u1 음수 skew(창 내) 결박 소비 유지 → 체인 계속 exit 0",
      code == 0, "exit=%d err=%s" % (code, err[-300:]))
check("5fb-u2 claim-role 재호출 없음(유계 음수 허용)",
      "cys claim-role" not in calls(tmp), "calls=%s" % calls(tmp)[-200:])
check("5fb-u3 시계 후퇴 stderr 진단 1줄", "시계 후퇴" in err, "err=%s" % err[-300:])
shutil.rmtree(tmp)


# ── 5-fb-v. ★P0-3 재시도 래치(retry_eligible) 핀: session_error(exit 10) 기록이 boot-last
#    **최상위** "retry" per-surface 맵({sid:{count,at}})을 기계 갱신하고, result.retry_eligible 이
#    §0-A session_error 행의 유일한 재실행 근거가 된다. 오너 결정 ⑬Y: 최초 실패(자기 이력 0)는
#    count=1=true(1회 재실행 허용) · 같은 surface 연속 2회째는 count=2=false(소진). 래치는
#    cmd_run 이 _Log 선기록 **이전에** 스냅샷하는 carry-forward 라, 타 surface 완주 런(정당거부
#    declined 포함)이 본체 result 를 덮어도 불소실이어야 한다(R3-P03-1 음성 독해 — 소진 증거의
#    외래 쓰기 소실 = 래치 재무장 = 기계 유계 붕괴의 봉합 검체). ──
def _bl(home):
    return json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"),
                          encoding="utf-8"))


tmp = tempfile.mkdtemp(prefix="boot-t5fbv-")
env, home = make_env(tmp, claim_exit=1)   # 스텁 claim=거부 마커+rc1 — v3 개입 런(직접 claim)용
env["CYS_DEPT_FALLBACK"] = "0"            # 개입 런을 declined 구계약(exit 7)으로 고정
bind_claim(env, 6)                        # 자기 surface(7)의 선행 claim rc6 → session_error 확정
code, out, err = run(env)
bl = _bl(home)
check("5fb-v1 최초 session_error exit 10", code == 10, "exit=%d" % code)
check("5fb-v2 ⑬Y 최초 실패 retry_eligible=true(count=1)",
      bl.get("result", {}).get("retry_eligible") is True
      and bl.get("retry", {}).get("7", {}).get("count") == 1,
      json.dumps({"result": bl.get("result", {}), "retry": bl.get("retry")},
                 ensure_ascii=False)[:300])
# 타 surface(8)의 정당거부(declined) 개입 — 본체 result 는 덮이지만 래치 맵은 이월돼야 한다
env8 = dict(env)
for _k in ("CYS_CLAIM_RC", "CYS_CLAIM_OUT", "CYS_CLAIM_SID", "CYS_CLAIM_AT"):
    env8.pop(_k, None)
env8["CYS_SURFACE_ID"] = "8"
code8, _, _ = run(env8)
bl = _bl(home)
check("5fb-v3 타 surface 개입 런=declined(검체 전제 — 본체 result 를 실제로 덮는다)",
      code8 == 7 and bl.get("result", {}).get("state") == "declined"
      and bl.get("result", {}).get("surface") == "8",
      "exit=%d result=%s" % (code8, json.dumps(bl.get("result", {}), ensure_ascii=False)[:200]))
check("5fb-v4 개입 후에도 래치 carry-forward 생존(retry['7'] 불소실)",
      bl.get("retry", {}).get("7", {}).get("count") == 1,
      json.dumps(bl.get("retry"), ensure_ascii=False)[:200])
# 같은 surface(7) 2회째 session_error → count=2 → retry_eligible=false(소진 — 기계 유계)
code, out, err = run(env)
bl = _bl(home)
check("5fb-v5 동일 surface 연속 2회째 session_error retry_eligible=false(count=2 소진)",
      code == 10 and bl.get("result", {}).get("retry_eligible") is False
      and bl.get("retry", {}).get("7", {}).get("count") == 2,
      json.dumps({"result": bl.get("result", {}), "retry": bl.get("retry")},
                 ensure_ascii=False)[:300])
shutil.rmtree(tmp)

# ── 5-fb-v′. ★R3-ATTR-3 귀속 정규화 핀: `CYS_SURFACE_ID` 의 형식은 진입점마다 다르다 —
#    데몬 pane 주입·감독자는 `"7"`, GUI '마스터 시작'(spawn_orchestra_boot)은 회수한 참조
#    `"surface:7"` 을 그대로 싣는다. 래치 키·`result.surface` 귀속 필드가 원문이면 ⓐ같은 물리
#    좌석이 두 슬롯으로 갈려 §0-A 합산 상한(≈5)이 그 좌석에서 6이 되고 ⓑ '자기 surface' 대조가
#    GUI 기동 런에 대해 실패해 자가치유가 GUI 인구에게만 조용히 꺼진다. 두 표기가 **같은 슬롯·
#    같은 귀속 필드**를 산출함을 못박는다(정규화 = 숫자부 · my_surface_key 단일 소유). ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbv2-")
env, home = make_env(tmp)
bind_claim(env, 6)                        # 정수 표기 런: 자기 surface(7) rc6 → session_error
code, out, err = run(env)
bl = _bl(home)
check("5fb-v′1 정수 표기 런 최초 session_error(count=1·true)",
      code == 10 and bl.get("retry", {}).get("7", {}).get("count") == 1
      and bl.get("result", {}).get("surface") == "7",
      json.dumps({"result": bl.get("result", {}), "retry": bl.get("retry")},
                 ensure_ascii=False)[:300])
# 같은 좌석을 GUI 표기(`surface:7`)로 재기동 — 정규화가 없으면 새 슬롯("surface:7")이 생겨
# count=1(=재무장)이 되고 귀속 필드도 "surface:7" 로 갈린다.
env_gui = dict(env)
env_gui["CYS_SURFACE_ID"] = "surface:7"
bind_claim(env_gui, 6)                    # CYS_CLAIM_SID 도 같은 표기로 실린다(GUI 형상 동형)
code2, _, err2 = run(env_gui)
bl = _bl(home)
check("5fb-v′2 GUI 표기(surface:7)도 같은 래치 슬롯 — 소진 승계(count=2·false)",
      code2 == 10 and bl.get("retry", {}).get("7", {}).get("count") == 2
      and "surface:7" not in json.dumps(bl.get("retry"), ensure_ascii=False)
      and bl.get("result", {}).get("retry_eligible") is False,
      json.dumps({"result": bl.get("result", {}), "retry": bl.get("retry")},
                 ensure_ascii=False)[:300])
check("5fb-v′3 귀속 필드도 정규화 — 자기 surface 대조가 표기 차이로 깨지지 않는다",
      bl.get("result", {}).get("surface") == "7",
      json.dumps(bl.get("result", {}), ensure_ascii=False)[:200])
shutil.rmtree(tmp)

# ── 5-fb-w. ★P0-3 래치 TTL(24h) 회수 핀: at 이 24h 지난 항목은 새 런의 carry-forward 에서
#    회수된다(surface 는 단명이라 맵의 무한 증식 차단 — 항목 유계 2종의 하나). 회수 뒤의
#    session_error 는 다시 '최초 실패'(count=1·true)로 판정된다. ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbw-")
env, home = make_env(tmp)
bind_claim(env, 6)
code, out, err = run(env)
blp = os.path.join(home, ".cys", "state", "boot-last.json")
bl = json.load(open(blp, encoding="utf-8"))
check("5fb-w1 래치 선행 적재(count=1 — 검체 전제)",
      bl.get("retry", {}).get("7", {}).get("count") == 1,
      json.dumps(bl.get("retry"), ensure_ascii=False)[:200])
bl["retry"]["7"]["at"] = time.time() - (24 * 3600 + 60)   # 24h+ 경과로 밀어 회수 대상화
with open(blp, "w", encoding="utf-8") as f:
    json.dump(bl, f, ensure_ascii=False)
bind_claim(env, 6)                                        # 신선한 스탬프로 재결박
code, out, err = run(env)
bl = json.load(open(blp, encoding="utf-8"))
check("5fb-w2 24h 경과 항목 회수 → 최초 실패로 재판정(count=1·retry_eligible=true)",
      code == 10 and bl.get("result", {}).get("retry_eligible") is True
      and bl.get("retry", {}).get("7", {}).get("count") == 1,
      json.dumps({"result": bl.get("result", {}), "retry": bl.get("retry")},
                 ensure_ascii=False)[:300])
shutil.rmtree(tmp)

# ── 5-fb-x. ★P0-3 래치 리셋 핀: 자기 surface 의 정상 완주(ok:true)가 자기 항목을 제거한다 —
#    수리된 세션의 다음 session_error 는 다시 '최초 실패'로 판정돼야 한다(리셋 없는 래치는
#    수리 후에도 영구 소진 = 자가치유 경로 재봉인). declined·session_error 등 ok:null 완주는
#    리셋이 아니다(5fb-v4 가 그 방향을 함께 핀한다). ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbx-")
env, home = make_env(tmp)
bind_claim(env, 6)
run(env)                                   # count=1 적재
env_ok = dict(env)
for _k in ("CYS_CLAIM_RC", "CYS_CLAIM_OUT", "CYS_CLAIM_SID", "CYS_CLAIM_AT"):
    env_ok.pop(_k, None)
code, out, err = run(env_ok)               # 스텁 직접 claim rc0 → 정상 완주(ok:true)
bl = _bl(home)
check("5fb-x1 정상 완주 exit 0(검체 전제)", code == 0, "exit=%d err=%s" % (code, err[-200:]))
check("5fb-x2 ok:true 완주가 자기 래치 항목 제거(리셋)",
      "7" not in (bl.get("retry") or {}),
      json.dumps(bl.get("retry"), ensure_ascii=False)[:200])
shutil.rmtree(tmp)

# ── 5-fb-y. ★R2-3 지속성 교차 핀(2026-08-26 적대검증): boot-last **쓰기가 실패**하면 그 런의
#    session_error 는 디스크에 반영되지 않는다 — 그런데도 §0-A 가 읽는 유일 채널에는 직전의
#    ok:true 가 남아 master 가 '기동 완료'를 보고하고, 매 런이 자기 이력 0 으로 재판정돼
#    retry_eligible 이 항상 true 가 된다(기계 유계 무효화). 수리 계약 셋을 함께 잰다:
#      ⓐ 쓰기 실패 관측 시 retry_eligible=false + retry_eligible_unknown=true(측정 불능은 통과가
#        아니다 = 재실행 허가도 아니다)
#      ⓑ stdout 에 `boot-last-mirror` 1줄(디스크가 죽어도 모델이 인용할 채널 — A7 동형)
#      ⓒ 정상 경로 무회귀(대조군: 잠금 없이 3연속 → true,false,false)
#    ★디렉터리 권한(0o500)으로 쓰기를 막는 방식이라 root 나 비-POSIX 파일시스템에서는 계측이
#      성립하지 않는다 — 그때는 **통과가 아니라 skip 을 명시**한다(측정 불능 은폐 금지).
tmp = tempfile.mkdtemp(prefix="boot-t5fby-")
env, home = make_env(tmp)
bind_claim(env, 6)
_stdir = os.path.join(home, ".cys", "state")
os.makedirs(_stdir, exist_ok=True)
_blp = os.path.join(_stdir, "boot-last.json")
with open(_blp, "w", encoding="utf-8") as f:                       # 직전 런의 '완주 성공'
    json.dump({"result": {"ok": True, "state": "completed", "surface": "7", "run_id": "old"}}, f)
os.chmod(_stdir, 0o500)
_probe = None
try:
    with open(os.path.join(_stdir, ".probe"), "w") as f:            # 계측 타당성 확인
        f.write("x")
    _probe = "writable"                                             # 잠금이 안 걸렸다
    os.remove(os.path.join(_stdir, ".probe"))
except OSError:
    _probe = "locked"
if _probe != "locked":
    print("  SKIP 5fb-y: state 디렉터리 쓰기 잠금 불성립(%s) — 계측 불능(통과 아님)" % _probe)
else:
    code, out, err = run(env)
    os.chmod(_stdir, 0o700)
    _disk = json.load(open(_blp, encoding="utf-8"))
    _mirror = None
    for _ln in (out or "").splitlines():
        _ln = _ln.strip()
        if _ln.startswith("{") and "boot-last-mirror" in _ln:
            try:
                _mirror = json.loads(_ln)
            except ValueError:
                pass
    check("5fb-y1 쓰기 실패 런도 exit 10(본체는 계측 실패로 죽지 않는다)", code == 10,
          "exit=%d err=%s" % (code, err[-200:]))
    check("5fb-y2 디스크는 남의 완주 성공 그대로(검체 전제 — 쓰기가 실제로 막혔다)",
          _disk.get("result", {}).get("state") == "completed",
          json.dumps(_disk.get("result"), ensure_ascii=False)[:200])
    check("5fb-y3 stdout 미러 1줄 존재(디스크가 죽어도 인용할 채널)",
          isinstance(_mirror, dict) and _mirror.get("state") == "session_error",
          "out=%s" % (out or "")[-300:])
    check("5fb-y4 측정 불능은 재실행 허가가 아니다(retry_eligible=false + unknown 표기)",
          isinstance(_mirror, dict) and _mirror.get("retry_eligible") is False
          and _mirror.get("retry_eligible_unknown") is True
          and (_mirror.get("log_write_failures") or 0) > 0,
          json.dumps(_mirror, ensure_ascii=False)[:300])
try:
    os.chmod(_stdir, 0o700)
except OSError:
    pass
shutil.rmtree(tmp)

# ── 5-fb-win/dept. 게이트 확인: 부서 레인에서는 폴백 미발동(부서 안에 부서 금지) ──
# ★마커를 실어 레인 게이트(_is_base_socket)를 실검증한다(2026-08-12 재검증 지적): 마커 없이
#   돌리면 origin 게이트가 먼저 None 을 반환해 이 핀이 엉뚱한 게이트로 green — 레인 게이트가
#   무핀 상태가 된다(마커 보유 부서 pane 의 부서-안-부서 스폰을 막는 유일한 중화 장치).
tmp = tempfile.mkdtemp(prefix="boot-t5fbd-")
env, home = make_env(tmp, claim_exit=1, socket="/x/cys-dept-alpha/cys.sock", pack_dept="alpha")
env["CYS_DECL_ORIGIN"] = "hook-human"
code, out, err = run(env)
check("5fbd-a 부서 레인 claim 거부 → 폴백 없이 exit 7", code == 7, "exit=%d" % code)
check("5fbd-b 부서 생성 미시도", "cys-dept allocate" not in calls(tmp))
shutil.rmtree(tmp)

# ── 6. 선행 단계 실패 exit 매핑: ping=3 · boot=4 (부팅-치명 전제 위반) ──
# ★preflight는 제외(2026-07-15 적대검증 adv#1): preflight FAIL은 팀 부팅을 abort하지 않는다
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

# ── 10. ★W-A3 ②ping 유계 재시도: 일시 실패 후 성공 → 체인 계속(단발 ping 시대 회귀 핀) ──
# 구 계약(단발 15s 1회)은 데몬 콜드스타트·Defender 첫 스캔 창의 첫 실패 하나로 선언 전체를
# EXIT_PING(3)으로 폐기했다 — 몇 초 뒤 살아날 데몬인데 체인이 통째로 접혔다. 신 계약은 벽시계
# 총예산(CYS_BOOT_PING_RETRY_TOTAL_S) 창 안에서 간격(CYS_BOOT_PING_RETRY_INTERVAL_S) 재시도한다.
# 스텁: 카운터 파일 기반 — 첫 2회 exit 1(무응답) → 3회째부터 0(콜드스타트 회복 모사).
tmp = tempfile.mkdtemp(prefix="boot-t10-")
env, home = make_env(tmp)
_cys_stub = os.path.join(tmp, "stubbin", "cys")
with open(_cys_stub, "w", encoding="utf-8", newline="\n") as f:
    f.write("#!/bin/sh\n"
            "echo \"cys $@\" >> \"%(t)s/calls.log\"\n"
            "case \"$1\" in\n"
            "  ping)\n"
            "    n=$(cat \"%(t)s/ping.count\" 2>/dev/null || echo 0)\n"
            "    n=$((n+1)); printf %%s \"$n\" > \"%(t)s/ping.count\"\n"
            "    [ \"$n\" -le 2 ] && exit 1\n"
            "    exit 0;;\n"
            # ★(B-360 R3) fixture realism (worker-7 approved): a real `cys boot --json` exit 0 prints its JSON line
            "  boot) echo '{\"roles\": [], \"summary\": {\"lock\": \"acquired\", \"fatal_failed\": 0}}'; exit 0;;\n"
            "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
            "esac\nexit 0\n" % {"t": tmp})
os.chmod(_cys_stub, 0o755)
# 하네스 전용 창 오버라이드(CHECK_* 와 동일 규약) — 재시도 2회에 넉넉한 창 + 짧은 간격.
env["CYS_BOOT_PING_RETRY_TOTAL_S"] = "30"
env["CYS_BOOT_PING_RETRY_INTERVAL_S"] = "0.1"
code, out, err = run(env)
check("10a ②ping 일시 실패(2회) 후 성공 — 체인 계속 exit 0", code == 0,
      "exit=%d err=%s" % (code, err[-300:]))
check("10b 부트 완주(마커 생성)", os.path.exists(marker_path(home)))
_pings = int(open(os.path.join(tmp, "ping.count"), encoding="utf-8").read() or "0")
check("10c ping 실측 3회(실패 2 + 성공 1 — 성공 즉시 진행·과재시도 없음)", _pings == 3,
      "pings=%d" % _pings)
bl = json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"), encoding="utf-8"))
_steps10 = [s["step"] for s in bl.get("steps", [])]
check("10d 재시도가 boot-last 에 실측 기록(②ping + ②ping#2 + ②ping#3)",
      "②ping" in _steps10 and "②ping#2" in _steps10 and "②ping#3" in _steps10,
      "steps=%r" % _steps10[:8])
shutil.rmtree(tmp)

# ── 11. ★W-A3 ⑤ exit 2 정밀 분기(축약판 — t1/t2/t3 3분기 전체 핀은 run_bootstrap_health
# H-EXIT-7 소유·여기는 t2 만): exit 2 + `cys ping` 생존이면 '데몬 소실'이 아니다 — 별도 상한
# (CHECK_UNJUDGEABLE_RETRIES·budget 키 부재 시 3)으로 유계 재시도 후 '팩 결손 가능성' 진단으로
# 실패한다. 구 계약(무조건 즉시 이탈)은 orchestra 스크립트 부재(python 자신이 rc 2)까지 '데몬
# 소실'로 오진해 처방(`cys ping`·데몬 기동)을 뒤집었다. ──
tmp = tempfile.mkdtemp(prefix="boot-t11-")
env, home = make_env(tmp, check_fail_times=0, check_final=2)   # check 항상 exit 2 · ping 은 0
code, out, err = run(env)
check("11a exit 2 반복 + 데몬 생존 → 최종 exit 6", code == 6, "exit=%d" % code)
try:
    _cap = max(1, int(_BU.leaf("CHECK_UNJUDGEABLE_RETRIES")))   # bootstrap 과 동일 산식
except Exception:
    _cap = 3                                                    # budget 키 부재 폴백(현행 실효값)
_attempts11 = int(open(os.path.join(tmp, "check.count"), encoding="utf-8").read())
check("11b 시도수=별도 상한(유계 — 1 초과·창 상한 4 미소진)",
      1 < _attempts11 <= _cap and _attempts11 < 4,
      "attempts=%d cap=%d" % (_attempts11, _cap))
bl = json.load(open(os.path.join(home, ".cys", "state", "boot-last.json"), encoding="utf-8"))
_unj11 = [s for s in bl.get("steps", []) if s.get("step") == "⑤check-unjudgeable"]
check("11c 진단='팩 결손 가능성'(데몬 소실 처방으로의 반전 금지)",
      bool(_unj11) and "팩 결손 가능성" in _unj11[-1].get("detail", "")
      and "데몬을 확인·기동하라" not in _unj11[-1].get("detail", ""),
      (_unj11[-1].get("detail", "")[:200] if _unj11 else "unjudgeable 단계 없음"))
shutil.rmtree(tmp)

# ── 12. ★W-A3 ③ _Log 쓰기 best-effort: boot-last 기록 실패가 부트 본체를 죽이지 않는다 ──
# 유도: boot-last.json 자리에 **디렉터리**를 만들어 _atomic_write_json 의 os.replace 를 전량
# 실패시킨다(Windows 공유 위반 실사고의 POSIX 결정론 재현). 구 계약은 _atomic_write_json
# 직호출이라 ⓐ step()/result() 경유 즉사 ⓑ finally→finish() 재예외가 정상 완주 exit 까지
# 삼켰다(exit 0 완주가 uncaught 크래시로 뒤집힘). 신 계약: 비크래시 + stderr 1줄(침묵 금지).
tmp = tempfile.mkdtemp(prefix="boot-t12-")
env, home = make_env(tmp)
os.makedirs(os.path.join(home, ".cys", "state", "boot-last.json"))   # 파일 자리의 디렉터리
code, out, err = run(env)
check("12a 계측 쓰기 전량 실패에도 부트 비크래시(체인 완주 exit 0)", code == 0,
      "exit=%d err=%s" % (code, err[-300:]))
check("12b 실패 사실 stderr 1줄+(조용한 삼킴 금지)", "boot-last 기록 실패" in err, err[-300:])
try:
    _summary12 = json.loads(out.strip().splitlines()[-1])
except Exception:
    _summary12 = {}
check("12c 완주 계약 보존(stdout 최종 JSON ok:true)", _summary12.get("ok") is True, out[-200:])
check("12d 본체 산출물 무손상(마커 생성)", os.path.exists(marker_path(home)))
shutil.rmtree(tmp)

# ══ 13+. ★(B-360 R3 · option 2) the ready marker is WITHHELD on an unknown-class signal ══════════════════
# FIX-SPEC-option2-v2 §2-§4 + §7 A1-A5, worker-7's veto of §1e (this run's records come back through an OUT file
# named by CYS_KEPT_PANE_RECORDS_OUT, the boot JSON is untouched) and the decision-point ruling (spec §10: the
# withhold is APPLIED right after ④ — a stale lane marker removed, the sidecar written — BEFORE the run's one
# kept-pane notice; ⑥ only skips writing). Prerequisite asserts first, then the discriminating one ("★").
# On af41d65c the marker is written whatever `cys boot` says, so every "★marker ABSENT" row is RED there.
import importlib.util
import traceback

R3_GUARD_SENTENCE = ("completion-guard Stop verification is SKIPPED (SKIPPED_GRACE) on every pane armed with "
                     "CYS_COMPLETION_GUARD=1, in every lane (the guard reads the base marker)")
R3_BASE_CONSEQUENCES = [R3_GUARD_SENTENCE, "CEO promotion is held on a never-promoted base",
                        "javis_bootstrap.py assert-ready refuses (exit 5)"]
R3_DEPT_CONSEQUENCES = ["javis_bootstrap.py assert-ready refuses (exit 5) for this lane",
                        "the base marker and the completion guard are not affected"]
R3_BASE_SOCK = "/u/.local/state/cys/cys.sock"            # the base lane by shape (javis_lane.socket_is_base)
R3_FOREIGN_SOCK = "/u/.local/state/cys-dept-zz/cys.sock"  # another lane
R3_DAEMON = 2000.5                                         # the stub daemon instance (`daemon.started_at`)


def r3_section(name, fn):
    """One section; a crash inside it is a FAIL row, never an abort of the rest of this file."""
    try:
        fn()
    except Exception as e:      # noqa: BLE001 — a test harness boundary
        check("%s (raised %s)" % (name, type(e).__name__), False, traceback.format_exc()[-600:])


def r3_row(role, outcome, mandatory=False, **kw):
    r = {"role": role, "agent": "claude", "outcome": outcome, "mandatory": mandatory}
    r.update(kw)
    return r


def r3_boot_stdout(*rows):
    """What `cys boot --json` prints on stdout: progress prose, then the one JSON line (cys.rs run_boot)."""
    return ("· claude: 기동 시작 (role=x)…\nboot 완료: (stub)\n"
            + json.dumps({"roles": list(rows), "summary": {"lock": "acquired", "fatal_failed": 0}}) + "\n")


def r3_seat(sid, role, session=None, created=10.0):
    return {"surface_id": sid, "surface_ref": "surface:%d" % sid, "role": role, "agent": "claude",
            "exited": False, "awakened_at": 1.0, "agent_alive": True, "seat": "occupied",
            "registered_session_id": session, "created_at": created}


def r3_status(worker_sid=7, worker_session="S-w", worker_created=10.0, started=R3_DAEMON):
    """`cys status --json` of a COMPLETE team (no deficit → `cys boot` is not called), every possible reviewer slot
    seated (native or Claude substitute, so the roster detection of the machine does not matter)."""
    seats = [r3_seat(3, "cso", "S-cso"), r3_seat(worker_sid, "worker", worker_session, worker_created),
             r3_seat(4, "reviewer-gemini"), r3_seat(5, "reviewer-codex"),
             r3_seat(8, "reviewer-claude-1", "S-c1"), r3_seat(9, "reviewer-claude-2", "S-c2")]
    daemon = {"version": "0.0.0-stub"}
    if started is not None:
        daemon["started_at"] = started
    return json.dumps({"daemon": daemon, "surfaces": seats})


def r3_record(session="S-w", sid=7, created=10.0, started=R3_DAEMON, socket=R3_BASE_SOCK, at=1999,
              withholds=True, owner=True, write="adoption", outcome="unknown"):
    """A kept-pane record line as cys writes it (schema 2, FIX-SPEC §1c); `owner=False` / `started=None` give the
    legacy / identity-unmeasured shapes."""
    rec = {"at": at, "command": "boot", "surface_id": sid, "role": "worker", "agent": "claude",
           "outcome": outcome, "write": write, "request_id": "w1-2", "schema": 2,
           "withholds_ready": withholds, "session_id": session, "surface_created_at": created,
           "identity_unmeasured": started is None}
    if owner:
        rec["owner"] = {"writer": "cys", "pid": 4242, "socket": socket, "daemon_started_at": started}
    return json.dumps(rec)


def r3_run(pre=None, keep=False, args=(), **kw):
    """One bootstrap run in a fresh scratch HOME (make_env + the new stub kwargs). `pre(tmp, env, home)` runs before
    it. Returns a dict of the observable facts; the scratch dir is removed unless `keep`."""
    tmp = tempfile.mkdtemp(prefix="boot-r3-")
    env, home = make_env(tmp, **kw)
    if pre is not None:
        pre(tmp, env, home)
    code, out, err = run(env, *args)
    res = r3_collect(tmp, home, code, out, err)
    res["env"] = env
    if keep:
        res["tmp"] = tmp
    else:
        shutil.rmtree(tmp, ignore_errors=True)
    return res


def r3_collect(tmp, home, code, out, err):
    try:
        summary = json.loads(out.strip().splitlines()[-1])
    except Exception:
        summary = {}
    state = os.path.join(home, ".cys", "state")
    sidecars = sorted(n for n in (os.listdir(state) if os.path.isdir(state) else []) if n.startswith("ready-withheld"))
    sidecar = None
    if sidecars:
        try:
            sidecar = json.load(open(os.path.join(state, sidecars[0]), encoding="utf-8"))
        except Exception:
            sidecar = {"unreadable": sidecars[0]}
    notices = []
    for n in sorted(x for x in os.listdir(tmp) if x.startswith("notice-in.")):
        raw = open(os.path.join(tmp, n), encoding="utf-8").read()
        try:
            notices.append(json.loads(raw))
        except ValueError:
            notices.append({"unparseable": raw})
    env_log = os.path.join(tmp, "boot-env.log")
    boot_env = open(env_log, encoding="utf-8").read().splitlines() if os.path.exists(env_log) else []
    bl_path = os.path.join(state, "boot-last.json")
    try:
        steps = json.load(open(bl_path, encoding="utf-8")).get("steps", [])
    except Exception:
        steps = []
    orch = open(os.path.join(tmp, "orch.log"), encoding="utf-8").read().split() \
        if os.path.exists(os.path.join(tmp, "orch.log")) else []
    return {"code": code, "out": out, "err": err, "summary": summary, "marker": os.path.exists(marker_path(home)),
            "marker_isdir": os.path.isdir(marker_path(home)), "sidecar": sidecar, "sidecars": sidecars,
            "notices": notices, "boot_env": boot_env, "steps": steps, "calls": calls(tmp),
            "check_called": "check" in orch, "promote_called": "promote-if-pending" in calls(tmp),
            "boot_called": called(tmp, "cys boot"), "home": home}


def r3_rows(r):
    """The withheld rows the run's sidecar names (the identity of a withhold)."""
    return (r["sidecar"] or {}).get("rows") or []


def r3_is_row(x, source, basis, **want):
    return (isinstance(x, dict) and x.get("source") == source and x.get("basis") == basis
            and all(x.get(k) == v for k, v in want.items()))


def r3_brief(r):
    return "exit=%s marker=%s sidecar=%s notices=%d boot=%s err=%s" % (
        r["code"], r["marker"], (r["sidecar"] or {}).get("state"), len(r["notices"]), r["boot_called"],
        r["err"][-300:].replace("\n", " | "))


def r3_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


R3_REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
R3_OLD = "af41d65c"


def r3_old_bootstrap():
    """javis_bootstrap.py as it was at af41d65c (git), or None when the history is not reachable (copy / shallow)."""
    try:
        src = subprocess.run(["git", "-C", R3_REPO, "show", "%s:cysjavis-pack/bin/javis_bootstrap.py" % R3_OLD],
                             capture_output=True, timeout=30)
    except Exception:
        return None
    if src.returncode != 0 or not src.stdout:
        return None
    d = tempfile.mkdtemp(prefix="boot-r3-old-")
    p = os.path.join(d, "javis_bootstrap_%s.py" % R3_OLD)
    with open(p, "wb") as f:
        f.write(src.stdout)
    sys.path.insert(0, os.path.dirname(SCRIPT))       # its siblings (javis_lane, javis_budget, …)
    try:
        return r3_module(p, "javis_bootstrap_%s" % R3_OLD)
    finally:
        shutil.rmtree(d, ignore_errors=True)


# ── 13. S1 — each unknown-class variant in its own run withholds the marker (RED) ──
def r3_s1():
    for name, row in (
            ("directive_unknown", r3_row("worker", "directive_unknown", True, surface_ref="surface:7")),
            ("recover_unknown", r3_row("worker", "recover_unknown", True, surface_ref="surface:7")),
            ("recover_unrecognised", r3_row("worker", "recover_unrecognised", True, surface_ref="surface:7")),
            ("failed+kept_pane=launch_unknown (non-mandatory)",
             r3_row("reviewer-codex", "failed", False, kept_pane="launch_unknown", surface_ref="surface:9")),
            ("failed+kept_pane=directive_unknown (non-mandatory)",
             r3_row("reviewer-codex", "failed", False, kept_pane="directive_unknown", surface_ref="surface:9"))):
        r = r3_run(boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True), row))
        check("13-S1 %s: prerequisite exit 0 · ⑤check called · ⑦ called" % name,
              r["code"] == 0 and r["check_called"] and r["promote_called"], r3_brief(r))
        check("13-S1 %s: ★marker ABSENT" % name, not r["marker"], r3_brief(r))
        # ★identity (ruling (b) widened the safe side: an unreadable JSON withholds too) — it is THIS boot row
        check("13-S1 %s: ★the one withheld row is this boot row (source boot, basis unknown)" % name,
              len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "boot", "unknown", outcome=row["outcome"],
                                                 kept_pane=row.get("kept_pane")),
              json.dumps(r3_rows(r), ensure_ascii=False)[:300])
        check("13-S1 %s: summary ready:false" % name, r["summary"].get("ready") is False,
              json.dumps(r["summary"], ensure_ascii=False)[:300])
        check("13-S1 %s: sidecar state=withheld (A3a)" % name, (r["sidecar"] or {}).get("state") == "withheld",
              "sidecars=%r" % r["sidecars"])


# ── 14. S2 — CONTROL: already_alive, no records file → marker written, ready:true, no notice, no sidecar ──
def r3_s2():
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True), r3_row("worker", "already_alive", True)))
    check("14-S2 control: exit 0 · ⑤ called", r["code"] == 0 and r["check_called"], r3_brief(r))
    check("14-S2 control: marker WRITTEN", r["marker"], r3_brief(r))
    check("14-S2 control: no kept-pane-notice call", not called_notice(r), r["calls"][-300:])
    check("14-S2 control: no sidecar", not r["sidecars"], "%r" % r["sidecars"])
    check("14-S2 summary ready:true (new key — absent on af41d65c)", r["summary"].get("ready") is True,
          json.dumps(r["summary"], ensure_ascii=False)[:300])


def called_notice(r):
    return any(l.startswith("cys kept-pane-notice") for l in r["calls"].splitlines())


# ── 15. S3 — an outcome no table knows (catch-all on the SAFE side) → withheld (RED) ──
def r3_s3():
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True),
                                          r3_row("worker", "brand_new_state", True)))
    check("15-S3 prerequisite exit 0", r["code"] == 0, r3_brief(r))
    check("15-S3 ★unknown outcome 'brand_new_state' → marker ABSENT", not r["marker"], r3_brief(r))
    check("15-S3 ★identity: the withheld row is the brand_new_state boot row (basis unknown)",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "boot", "unknown", outcome="brand_new_state"),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True),
                                          r3_row("reviewer-codex", "failed", False, kept_pane="brand_new_label")))
    check("15-S3 ★unknown kept_pane label → marker ABSENT", not r["marker"], r3_brief(r))
    check("15-S3 ★identity: the withheld row is the failed row with kept_pane brand_new_label (basis unknown)",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "boot", "unknown", outcome="failed",
                                             kept_pane="brand_new_label"),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])


# ── 16. S4 — a stale marker from an earlier run is REMOVED on the signal (RED) ──
def r3_s4():
    def stale(tmp, env, home):
        os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
        with open(marker_path(home), "w", encoding="utf-8") as f:
            f.write('{"pack_version": "unknown", "orchestra_check": "exit 0"}\n')
    r = r3_run(pre=stale, boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True)))
    check("16-S4 prerequisite exit 0", r["code"] == 0, r3_brief(r))
    check("16-S4 ★stale marker removed", not r["marker"], r3_brief(r))


# ── 17. S5 — Y2 byte identity (head condition 3): a boot that adopts UNKNOWN AND launches a pane (stderr after the
#    JSON) vs the same boot without the launch line ──
def r3_s5():
    so = r3_boot_stdout(r3_row("worker", "directive_unknown", True, surface_ref="surface:7"),
                        r3_row("reviewer-codex", "launched", False))
    old = r3_old_bootstrap()
    new = r3_module(SCRIPT, "javis_bootstrap_r3_new")
    for name, se in (("with launch", "[launch-agent] surface:9 created (role=reviewer-codex)\n"),
                     ("control without launch", "")):
        r = r3_run(boot_stdout=so, boot_stderr=se)
        step = [s for s in r["steps"] if s.get("step") == "④boot"]
        check("17-S5 %s (i) the ④boot step detail an OLD consumer gets == (so+se).strip()[-2000:]" % name,
              len(step) == 1 and step[0].get("detail") == (so + se).strip()[-2000:],
              "steps=%r" % [s.get("step") for s in r["steps"]])
        if old is None:
            print("  SKIP 17-S5 %s (i) verdicts vs %s: git history not reachable — measurement unavailable "
                  "(not a pass)" % (name, R3_OLD))
        else:
            for fn in ("_boot_fatal_verdict", "_boot_was_busy", "_boot_gate_pending_verdict"):
                check("17-S5 %s (i) %s(0, so+se) is the same on this tree and on %s" % (name, fn, R3_OLD),
                      getattr(new, fn)(0, so + se) == getattr(old, fn)(0, so + se))
        check("17-S5 %s (ii) ★marker ABSENT (the reader parses STDOUT only)" % name, not r["marker"], r3_brief(r))
        # ★(B-360 R3) mutant M2 (parse the merged text) survived the line above once ruling (b) made an unreadable JSON
        #   withhold too ("readiness unmeasured"): the absence alone does not prove the reader READ stdout. It did iff
        #   the withheld row is the boot row itself — directive_unknown, basis "unknown" — and nothing is "unmeasured".
        rows = (r["sidecar"] or {}).get("rows") or []
        check("17-S5 %s (ii) ★the withheld row is the boot row (directive_unknown, basis unknown) — stdout was READ"
              % name, len(rows) == 1 and rows[0].get("outcome") == "directive_unknown"
              and rows[0].get("basis") == "unknown" and (r["sidecar"] or {}).get("bases") == ["unknown"],
              json.dumps(rows, ensure_ascii=False)[:300])


# ── 18. S6 — P2' durable, NO-DEFICIT run (`cys boot` not called): a v2 record whose pane is still live (same daemon
#    instance, same number, same created_at, same session) withholds (the head's P1 costume case) (RED) ──
def r3_s6():
    r = r3_run(status_json=r3_status(), records_lines=[r3_record()])
    check("18-S6 prerequisite: no deficit → cys boot NOT called · exit 0 · ⑤ called",
          not r["boot_called"] and r["code"] == 0 and r["check_called"], r3_brief(r))
    check("18-S6 ★durable live record → marker ABSENT", not r["marker"], r3_brief(r))
    check("18-S6 ★identity: the withheld row is the durable record of surface:7 (source record, basis unknown)",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "record", "unknown", surface_id=7),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])
    check("18-S6 sidecar withheld + one notice carrying ready_withheld",
          (r["sidecar"] or {}).get("state") == "withheld" and len(r["notices"]) == 1
          and isinstance(r["notices"][0].get("ready_withheld"), dict), r3_brief(r))


# ── 19. S7 — P2' clears when the pane is gone (no surface 7 any more; worker sits on 12) → marker written ──
def r3_s7():
    r = r3_run(status_json=r3_status(worker_sid=12), records_lines=[r3_record()])
    check("19-S7 prerequisite: no deficit · exit 0", not r["boot_called"] and r["code"] == 0, r3_brief(r))
    check("19-S7 ★pane gone → marker WRITTEN · no sidecar · no notice",
          r["marker"] and not r["sidecars"] and not r["notices"], r3_brief(r))


# ── 20. S8 — A2 one lifetime for every seat kind (daemon instance always in the key; session only an extra guard) ──
def r3_s8():
    for name, kw, rec, want_marker in (
            ("(a) claude row, NEW daemon instance", {}, r3_record(started=1000.0), True),
            ("(b) same daemon + same session", {}, r3_record(), False),
            ("(c) same daemon + DIFFERENT session", {"worker_session": "S-new"}, r3_record(), True),
            ("(d) unmeasured row, daemon started after at+1", {}, r3_record(started=None, at=100), True),
            ("(e) unmeasured row, daemon older than at", {}, r3_record(started=None, at=3000), False),
            ("(f) codex row (session null), same daemon + same created_at", {"worker_session": None},
             r3_record(session=None), False),
            ("(g) codex row (session null), same daemon, created_at differs", {"worker_session": None,
                                                                              "worker_created": 55.0},
             r3_record(session=None), True)):
        r = r3_run(status_json=r3_status(**kw), records_lines=[rec])
        check("20-S8 %s: prerequisite no deficit · exit 0" % name, not r["boot_called"] and r["code"] == 0,
              r3_brief(r))
        check("20-S8 %s: ★marker %s" % (name, "WRITTEN (gone)" if want_marker else "ABSENT (live)"),
              r["marker"] is want_marker, r3_brief(r))
        if not want_marker:
            check("20-S8 %s: ★identity: the withheld row is the durable record (source record, basis unknown)" % name,
                  len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "record", "unknown", surface_id=7),
                  json.dumps(r3_rows(r), ensure_ascii=False)[:300])


# ── 21. S9 — P2' lane filter: another lane's record is skipped; a record with no owner is KEPT (safe side) ──
def r3_s9():
    r = r3_run(status_json=r3_status(), records_lines=[r3_record(socket=R3_FOREIGN_SOCK)])
    check("21-S9 ★another lane's owner socket → marker WRITTEN", r["marker"] and not r["boot_called"], r3_brief(r))
    legacy = json.dumps({"at": 3000, "command": "boot", "surface_id": 7, "role": "worker", "agent": "claude",
                         "outcome": "unknown", "write": "adoption", "request_id": "w1-2"})
    r = r3_run(status_json=r3_status(), records_lines=[legacy])
    check("21-S9 ★an R2 record with no owner (and no withholds_ready) → kept → marker ABSENT",
          not r["marker"] and not r["boot_called"], r3_brief(r))
    check("21-S9 ★identity: the R2 record withholds as UNMEASURED (no withholds_ready), source record",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "record", "unmeasured", surface_id=7),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])
    # the bootstrap's own lane-key resolver on the three socket forms (unix base · unix dept · Windows pipe)
    b = r3_module(SCRIPT, "javis_bootstrap_r3_lane")
    rows = (("", R3_BASE_SOCK, False), ("", R3_FOREIGN_SOCK, True), ("", r"\\.\pipe\cys", False),
            ("", r"\\.\pipe\cys-dept-x", True), (r"\\.\pipe\cys-dept-x", r"\\.\pipe\cys-dept-x", False),
            (r"\\.\pipe\cys-dept-x", r"\\.\pipe\cys-dept-y", True), (R3_FOREIGN_SOCK, R3_FOREIGN_SOCK, False))
    for here, owner_sock, foreign in rows:
        got = b._kept_record_is_foreign_lane({"owner": {"socket": owner_sock}}, here_sock=here)
        check("21-S9 lane filter here=%r owner=%r → foreign=%s" % (here or "<unset: base>", owner_sock, foreign),
              got is foreign, "got %r" % got)
    check("21-S9 missing owner / missing socket → never foreign (safe side)",
          b._kept_record_is_foreign_lane({}, here_sock="") is False
          and b._kept_record_is_foreign_lane({"owner": {"pid": 1}}, here_sock="") is False)


# ── 22. S10 — status unreadable + a candidate record → withheld ("liveness unmeasured") (RED) ──
def r3_s10():
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True), r3_row("worker", "already_alive", True)),
               records_lines=[r3_record()])
    check("22-S10 prerequisite: status unreadable → boot runs (deficit assumed) · exit 0",
          r["boot_called"] and r["code"] == 0, r3_brief(r))
    check("22-S10 ★status unreadable + candidate → marker ABSENT", not r["marker"], r3_brief(r))
    check("22-S10 ★identity: the withheld row is the record, basis UNMEASURED (liveness unmeasured)",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "record", "unmeasured", surface_id=7)
          and "liveness" in str(r3_rows(r)[0].get("why", "")),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])


# ── 23. S11 — loudness (head condition 4): ONE notice per run, naming the base marker and the guard skip; the boot
#    child deferred its own notice (both vars); the notice carries THIS run's records from the out file (RED) ──
R3_ADOPT_REC = json.loads(r3_record())


def r3_s11():
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True, surface_ref="surface:7")),
               boot_records_out=json.dumps([R3_ADOPT_REC]))
    check("23-S11 prerequisite exit 0 · marker absent", r["code"] == 0 and not r["marker"], r3_brief(r))
    env_ok = (len(r["boot_env"]) == 1 and r["boot_env"][0].startswith("notice=caller out=/")
              and r["boot_env"][0] != "notice=caller out=unset")
    check("23-S11 the cys boot child got CYS_KEPT_PANE_NOTICE=caller + an absolute CYS_KEPT_PANE_RECORDS_OUT",
          env_ok, "%r" % r["boot_env"])
    check("23-S11 ★exactly ONE kept-pane-notice call", len(r["notices"]) == 1, r3_brief(r))
    n = r["notices"][0] if r["notices"] else {}
    rw = n.get("ready_withheld") or {}
    check("23-S11 ★ready_withheld.marker == the base marker · lane base · consequences = the base list (guard first)",
          rw.get("marker") == marker_path(r["home"]) and rw.get("lane") == "base"
          and rw.get("consequences") == R3_BASE_CONSEQUENCES and rw.get("remove_error") is None,
          json.dumps(n, ensure_ascii=False)[:500])
    check("23-S11 ★identity: an UNKNOWN-only withhold names nothing as unmeasured (ready_withheld.unmeasured == [])",
          rw.get("unmeasured") == [], json.dumps(rw, ensure_ascii=False)[:300])
    check("23-S11 the notice carries THIS run's records (from the out file) · command bootstrap",
          n.get("command") == "bootstrap" and R3_ADOPT_REC in (n.get("records") or []),
          json.dumps(n, ensure_ascii=False)[:500])


# ── 24. S12 — dept lane + signal: the LANE marker is withheld, a pre-existing base marker is byte-identical, the
#    consequences are the dept list (RED) ──
def r3_s12():
    tmp = tempfile.mkdtemp(prefix="boot-r3-dept-")
    try:
        dept_sock = os.path.join(tmp, "state", "cys-dept-dept-3", "cys.sock")
        env, home = make_env(tmp, socket=dept_sock, pack_dept="dept-3",
                             boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True)))
        env_base = dict(env)
        env_base.pop("CYS_SOCKET", None)
        run(env_base, "issue-ticket", "--dept", "dept-3")     # the dept team path needs a ticket
        os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
        base_bytes = b'{"pack_version": "base-run", "orchestra_check": "exit 0"}\n'
        with open(marker_path(home), "wb") as f:
            f.write(base_bytes)
        code, out, err = run(env)
        r = r3_collect(tmp, home, code, out, err)
        lane_marker = [n for n in os.listdir(os.path.join(home, ".cys")) if n.startswith(".master-bootstrapped-")]
        check("24-S12 prerequisite: dept team path ran boot · exit 0", r["boot_called"] and code == 0, r3_brief(r))
        check("24-S12 ★the dept lane marker is ABSENT", not lane_marker, "%r" % lane_marker)
        check("24-S12 ★identity: the withheld row is the directive_unknown boot row (basis unknown)",
              len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "boot", "unknown", outcome="directive_unknown"),
              json.dumps(r3_rows(r), ensure_ascii=False)[:300])
        check("24-S12 the base marker is byte-identical", open(marker_path(home), "rb").read() == base_bytes)
        rw = (r["notices"][0] if r["notices"] else {}).get("ready_withheld") or {}
        check("24-S12 consequences = the dept list · lane dept · marker = this lane's path",
              rw.get("consequences") == R3_DEPT_CONSEQUENCES and rw.get("lane") == "dept"
              and os.path.basename(rw.get("marker", "")).startswith(".master-bootstrapped-"),
              json.dumps(r["notices"], ensure_ascii=False)[:400])
        check("24-S12 the sidecar is the dept lane's (suffixed) file",
              r["sidecars"] and r["sidecars"][0] != "ready-withheld.json", "%r" % r["sidecars"])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# ── 25. S13 — CONTROL: the fatal column is unchanged (not ready ≠ fatal) ──
def r3_s13():
    r = r3_run(boot_exit=1, boot_stdout=r3_boot_stdout(
        r3_row("worker", "failed", True, kept_pane="directive_unknown", surface_ref="surface:9")))
    check("25-S13 control: mandatory failed+kept_pane=directive_unknown, boot exit 1 → bootstrap exit 4 (as today)",
          r["code"] == 4, r3_brief(r))
    b = r3_module(SCRIPT, "javis_bootstrap_r3_fatal")
    check("25-S13 control: BOOT_FATAL_OUTCOMES == ('failed', 'missing')", b.BOOT_FATAL_OUTCOMES == ("failed", "missing"))


# ── 26. S14 — CONTROL: the completion guard reads the BASE marker by default (the consequence list depends on it;
#    if the guard's source moves, the list must be revisited) ──
def r3_s14():
    tmp = tempfile.mkdtemp(prefix="boot-r3-guard-")
    try:
        env = dict(os.environ)
        env["HOME"] = os.path.join(tmp, "home")
        env.pop("CYS_GUARD_BOOT_MARKER", None)
        probe = ("import sys; sys.path.insert(0, %r); import javis_completion_guard as g, javis_bootstrap as b; "
                 "print(g._boot_marker_path()); print(b.MARKER)" % os.path.dirname(SCRIPT))
        p = subprocess.run([PY, "-c", probe], capture_output=True, text=True, env=env, timeout=60)
        lines = p.stdout.strip().splitlines()
        check("26-S14 control: guard _boot_marker_path() (CYS_GUARD_BOOT_MARKER unset) == bootstrap base MARKER",
              p.returncode == 0 and len(lines) == 2 and lines[0] == lines[1]
              and lines[0] == os.path.join(env["HOME"], ".cys", ".master-bootstrapped"),
              "rc=%s out=%r err=%s" % (p.returncode, lines, p.stderr[-300:]))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# ── 27. S15 — worker-7's veto: the out file is read on EVERY exit path of the boot call, even a fatal exit 1, and the
#    PLAIN notice still goes out for refused-only records (no withheld clause); the out file is removed (RED) ──
R3_REFUSED_REC = json.loads(r3_record(outcome="refused", write="directive", withholds=False))


def r3_s15():
    r = r3_run(boot_exit=1, boot_stdout=r3_boot_stdout(
        r3_row("worker", "failed", True, kept_pane="directive_refused", surface_ref="surface:9")),
        boot_records_out=json.dumps([R3_REFUSED_REC]))
    check("27-S15 prerequisite: fatal boot → bootstrap exit 4", r["code"] == 4, r3_brief(r))
    n = r["notices"][0] if r["notices"] else {}
    check("27-S15 ★one PLAIN notice carrying this run's refused record (ready_withheld null)",
          len(r["notices"]) == 1 and n.get("records") == [R3_REFUSED_REC] and n.get("ready_withheld") is None,
          json.dumps(r["notices"], ensure_ascii=False)[:400])
    outp = r["boot_env"][0].split(" out=", 1)[1] if r["boot_env"] and " out=" in r["boot_env"][0] else ""
    check("27-S15 the out file is removed after reading", outp.startswith("/") and not os.path.exists(outp),
          "out=%r" % outp)


# ── 28. S16/S17 — spec §10 decision point: the withhold is APPLIED right after ④, so the notice's clause is true on
#    every exit path: fatal exit 4 and check-fail exit 6 also leave the stale marker removed + the sidecar (RED) ──
def r3_stale(tmp, env, home):
    os.makedirs(os.path.join(home, ".cys"), exist_ok=True)
    with open(marker_path(home), "w", encoding="utf-8") as f:
        f.write('{"pack_version": "unknown"}\n')


def r3_s16_s17():
    for name, kw, want in (
            ("fatal exit 4", {"boot_exit": 1, "boot_stdout": r3_boot_stdout(
                r3_row("cso", "failed", True), r3_row("worker", "directive_unknown", True))}, 4),
            ("check-fail exit 6", {"check_fail_times": 99, "boot_stdout": r3_boot_stdout(
                r3_row("worker", "directive_unknown", True))}, 6)):
        r = r3_run(pre=r3_stale, **kw)
        check("28-S16/17 %s: prerequisite exit %d" % (name, want), r["code"] == want, r3_brief(r))
        check("28-S16/17 %s: ★stale marker ABSENT" % name, not r["marker"], r3_brief(r))
        check("28-S16/17 %s: ★identity: the withheld row is the directive_unknown boot row (basis unknown)" % name,
              any(r3_is_row(x, "boot", "unknown", outcome="directive_unknown") for x in r3_rows(r))
              and all(x.get("basis") == "unknown" for x in r3_rows(r)),
              json.dumps(r3_rows(r), ensure_ascii=False)[:300])
        rw = (r["notices"][0] if r["notices"] else {}).get("ready_withheld") or {}
        check("28-S16/17 %s: sidecar withheld + the notice's withheld clause (no remove_error)" % name,
              (r["sidecar"] or {}).get("state") == "withheld" and rw.get("marker") == marker_path(r["home"])
              and rw.get("remove_error") is None, r3_brief(r))


# ── 29. S18 — spec §10: the stale marker CANNOT be removed → the notice says so (remove_error), never a false
#    "withheld"; ⑧ carries stale_marker_remains; ⑥ does not claim "withheld" (RED) ──
def r3_s18():
    def undeletable(tmp, env, home):
        os.makedirs(os.path.join(marker_path(home), "pinned"), exist_ok=True)   # a non-empty directory: unlink fails
    r = r3_run(pre=undeletable, boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True)))
    check("29-S18 prerequisite: exit 0 · the marker path still exists (unlink failed)",
          r["code"] == 0 and r["marker"], r3_brief(r))
    rw = (r["notices"][0] if r["notices"] else {}).get("ready_withheld") or {}
    check("29-S18 ★identity: the withheld row is the directive_unknown boot row (basis unknown)",
          len(r3_rows(r)) == 1 and r3_is_row(r3_rows(r)[0], "boot", "unknown", outcome="directive_unknown"),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])
    check("29-S18 ★the notice carries remove_error (Rust then says COULD NOT BE REMOVED, never WITHHELD)",
          len(r["notices"]) == 1 and isinstance(rw.get("remove_error"), str) and rw["remove_error"],
          json.dumps(r["notices"], ensure_ascii=False)[:400])
    check("29-S18 ⑧ stale_marker_remains:true · ready:false", r["summary"].get("stale_marker_remains") is True
          and r["summary"].get("ready") is False, json.dumps(r["summary"], ensure_ascii=False)[:300])
    m6 = [s for s in r["steps"] if s.get("step") == "⑥marker"]
    check("29-S18 the ⑥ step does not claim the marker is withheld",
          m6 and "could not be removed" in m6[-1].get("detail", "") and "marker withheld" not in m6[-1].get("detail", ""),
          "%r" % [s.get("detail", "")[:160] for s in m6])


# ── 30. A3a — the sidecar is cleared by a later run that writes the marker; `status` prints it (RED) ──
def r3_a3a():
    tmp = tempfile.mkdtemp(prefix="boot-r3-a3a-")
    try:
        env, home = make_env(tmp, status_json=r3_status(), records_lines=[r3_record()])
        code, out, err = run(env)
        r1 = r3_collect(tmp, home, code, out, err)
        check("30-A3a run 1 (live record): marker absent · sidecar withheld",
              not r1["marker"] and (r1["sidecar"] or {}).get("state") == "withheld", r3_brief(r1))
        sc, so, se = run(env, "status")
        try:
            st = json.loads(so)
        except ValueError:
            st = {}
        check("30-A3a `javis_bootstrap.py status` prints the sidecar (additive field)",
              (st.get("ready_withheld") or {}).get("state") == "withheld" and st.get("ready_withheld_path"),
              so[-300:])
        with open(os.path.join(tmp, "status.json"), "w", encoding="utf-8") as f:
            f.write(r3_status(worker_sid=12))          # the pane is gone
        code, out, err = run(env)
        r2 = r3_collect(tmp, home, code, out, err)
        check("30-A3a run 2 (pane gone): ★marker WRITTEN and the sidecar REMOVED",
              r2["marker"] and not r2["sidecars"], r3_brief(r2))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# ── 31. F4 parity — the reader's record path is Rust's `kept_pane_records_path(&pack_state_base())`: the PARENT of the
#    pack dir (shared by every lane), with Path::parent's trailing-separator semantics ──
def r3_f4():
    tmp = tempfile.mkdtemp(prefix="boot-r3-f4-")
    try:
        for pack_dir, want in (("/a/b/pack", "/a/b/.kept-pane-records.jsonl"),
                               ("/a/b/pack-dept-x/", "/a/b/.kept-pane-records.jsonl")):
            env = dict(os.environ)
            env["HOME"] = os.path.join(tmp, "home")
            env["CYS_PACK_DIR"] = pack_dir
            probe = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as b; "
                     "print(b._kept_pane_records_path())" % os.path.dirname(SCRIPT))
            p = subprocess.run([PY, "-c", probe], capture_output=True, text=True, env=env, timeout=60)
            check("31-F4 CYS_PACK_DIR=%r → records path %s (Rust pack_state_base parity)" % (pack_dir, want),
                  p.returncode == 0 and p.stdout.strip() == want, "rc=%s out=%r err=%s"
                  % (p.returncode, p.stdout.strip(), p.stderr[-200:]))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


for _name, _fn in (("13-S1", r3_s1), ("14-S2", r3_s2), ("15-S3", r3_s3), ("16-S4", r3_s4), ("17-S5", r3_s5),
                   ("18-S6", r3_s6), ("19-S7", r3_s7), ("20-S8", r3_s8), ("21-S9", r3_s9), ("22-S10", r3_s10),
                   ("23-S11", r3_s11), ("24-S12", r3_s12), ("25-S13", r3_s13), ("26-S14", r3_s14),
                   ("27-S15", r3_s15), ("28-S16/17", r3_s16_s17), ("29-S18", r3_s18), ("30-A3a", r3_a3a),
                   ("31-F4", r3_f4)):
    if os.environ.get("BOOT_R3_ONLY") and _name not in os.environ["BOOT_R3_ONLY"].split(","):
        continue
    r3_section(_name, _fn)


# ── 32. worker-7's busy ruling (narrowed): a REAL busy report (summary.lock=="busy" AND every role "busy", the literal
#    of src/bin/cys.rs:13451@af41d65c) is the busy column — not withheld; a lock=busy JSON with ONE non-busy row is
#    classified normally (directive_unknown → withheld) ──
def r3_busy():
    busy_rows = [r3_row(role, "busy", True, reason="boot 락 보유자 존재(중복 스폰 방지)") for role in ("cso", "worker")]
    real = json.dumps({"roles": busy_rows, "summary": {"busy": 2, "lock": "busy", "fatal_failed": 0}}) + "\n"
    r = r3_run(boot_exit=75, boot_stdout="cys boot — 다른 boot 진행 중(락 보유)\n" + real)
    check("32-busy control: a real busy boot (exit 75) → marker WRITTEN (busy column, nothing adopted by this run)",
          r["code"] == 0 and r["marker"] and not r["sidecars"], r3_brief(r))
    mixed = json.dumps({"roles": [busy_rows[0], r3_row("worker", "directive_unknown", True)],
                        "summary": {"busy": 1, "lock": "busy", "fatal_failed": 0}}) + "\n"
    r = r3_run(boot_exit=75, boot_stdout=mixed)
    check("32-busy ★lock=busy with one directive_unknown row → classified → marker ABSENT", not r["marker"], r3_brief(r))
    # (corrected after 08981f1f: per worker-7's busy ruling a mixed report is classified row by row, so its "busy" row is
    #  a value no table knows and withholds through the catch-all too — two rows, both basis unknown)
    check("32-busy ★identity: the directive_unknown row withholds (basis unknown) and the mixed report's busy row is "
          "classified by the tables (catch-all → withheld, basis unknown)",
          len(r3_rows(r)) == 2
          and any(r3_is_row(x, "boot", "unknown", outcome="directive_unknown") for x in r3_rows(r))
          and any(r3_is_row(x, "boot", "unknown", outcome="busy") for x in r3_rows(r)),
          json.dumps(r3_rows(r), ensure_ascii=False)[:300])


# ── 33. worker-7's no-JSON ruling (split): (b) `cys boot --json` exit 0 with NO parseable JSON on stdout (prose only)
#    → WITHHELD, "readiness unmeasured: boot --json stdout unreadable"; (a) the BOOT_SKEW bare fallback (old binary:
#    no JSON by design, cannot emit the unknown signals) → no rows → written (control) ──
def r3_nojson():
    r = r3_run(boot_stdout="boot 완료: 신규 기동 0 · (no json line)\n")
    rows = r["summary"].get("ready_withheld") or (r["sidecar"] or {}).get("rows") or []
    check("33-(b) prerequisite exit 0", r["code"] == 0, r3_brief(r))
    check("33-(b) ★exit 0 + no parseable JSON → marker ABSENT with the 'readiness unmeasured' reason",
          not r["marker"] and any("readiness unmeasured: boot --json stdout unreadable" in str(x.get("why", ""))
                                  for x in rows if isinstance(x, dict)),
          "%s rows=%s" % (r3_brief(r), json.dumps(rows, ensure_ascii=False)[:300]))
    r = r3_run(boot_json_unsupported=True)
    skew = [s for s in r["steps"] if s.get("step") == "④boot-skew"]
    check("33-(a) control: the BOOT_SKEW bare fallback ran · exit 0 · marker WRITTEN (no rows)",
          skew and r["code"] == 0 and r["marker"], r3_brief(r))


for _name, _fn in (("32-busy", r3_busy), ("33-nojson", r3_nojson)):
    if os.environ.get("BOOT_R3_ONLY") and _name not in os.environ["BOOT_R3_ONLY"].split(","):
        continue
    r3_section(_name, _fn)


# ── 34. the head's note: READINESS UNMEASURED is not a value. Every withheld row and the sidecar carry a `basis` —
#    "unknown" (an UNKNOWN-class signal: a boot row or a durable record) or "unmeasured" (an unreadable boot JSON,
#    status or record line) — and the sidecar lists the bases present. Plus worker-7's exit-124 ruling: a `--json` boot
#    that exits non-zero (not 75/78) with no parseable JSON is unmeasured, so the decision point removes a stale marker
#    there too; the fatal column is unchanged (exit 4). The control proves the removal comes from "unmeasured", not
#    from "fatal": exit 1 WITH a parseable JSON and no unknown row leaves the stale marker in place. ──
def r3_basis():
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True, surface_ref="surface:7")))
    sc = r["sidecar"] or {}
    rows = sc.get("rows") or []
    check("34 ★an UNKNOWN-class row → basis 'unknown' (sidecar rows + ⑧ ready_withheld) · sidecar bases ['unknown']",
          bool(rows) and all(x.get("basis") == "unknown" for x in rows) and sc.get("bases") == ["unknown"]
          and all(x.get("basis") == "unknown" for x in (r["summary"].get("ready_withheld") or [{}])),
          json.dumps(sc, ensure_ascii=False)[:400])
    r = r3_run(boot_stdout="boot 완료: (no json line)\n")
    sc = r["sidecar"] or {}
    rows = sc.get("rows") or []
    rw = (r["notices"][0] if r["notices"] else {}).get("ready_withheld") or {}
    check("34 ★(b) unreadable boot JSON → basis 'unmeasured' · bases ['unmeasured'] · the notice names it unmeasured",
          bool(rows) and all(x.get("basis") == "unmeasured" for x in rows) and sc.get("bases") == ["unmeasured"]
          and any("boot --json stdout unreadable" in u for u in (rw.get("unmeasured") or [])),
          "%s · notice %s" % (json.dumps(sc, ensure_ascii=False)[:300], json.dumps(rw, ensure_ascii=False)[:300]))
    m6 = [x for x in r["steps"] if x.get("step") == "⑥marker"]
    check("34 (b) the ⑥ step says 'readiness unmeasured (…)', never 'not ready' / 'UNKNOWN'",
          bool(m6) and "readiness unmeasured (" in m6[-1].get("detail", "")
          and "not ready" not in m6[-1].get("detail", "") and "UNKNOWN" not in m6[-1].get("detail", ""),
          "%r" % [x.get("detail", "")[:200] for x in m6])
    r = r3_run(pre=r3_stale, boot_exit=124)                     # killed: no stdout at all
    sc = r["sidecar"] or {}
    rw = (r["notices"][0] if r["notices"] else {}).get("ready_withheld") or {}
    check("34 exit 124 without JSON: prerequisite bootstrap exit 4 (fatal by the old contract)", r["code"] == 4,
          r3_brief(r))
    check("34 ★exit 124 without JSON → stale marker ABSENT · sidecar withheld · basis unmeasured naming exit 124 · ONE notice",
          not r["marker"] and sc.get("state") == "withheld" and sc.get("bases") == ["unmeasured"]
          and any("exit 124" in str(x.get("why", "")) for x in (sc.get("rows") or []))
          and len(r["notices"]) == 1 and any("exit 124" in u for u in (rw.get("unmeasured") or [])),
          "%s · sidecar %s" % (r3_brief(r), json.dumps(sc, ensure_ascii=False)[:300]))
    r = r3_run(pre=r3_stale, boot_exit=1, boot_stdout=r3_boot_stdout(r3_row("cso", "failed", True)))
    check("34 control: exit 1 WITH a parseable JSON and no unknown row → exit 4 · the stale marker is STILL PRESENT",
          r["code"] == 4 and r["marker"] and not r["sidecars"], r3_brief(r))


for _name, _fn in (("34-basis", r3_basis),):
    if os.environ.get("BOOT_R3_ONLY") and _name not in os.environ["BOOT_R3_ONLY"].split(","):
        continue
    r3_section(_name, _fn)


# ── 35. REFLECTION-1 fixes (R1-P-1, R1-P-2, R1-P3-1, R1-P-3) ──
def r3_r1():
    # R1-P-1 · pack skew: an OLDER javis_lane.py (af41d65c's — no `ready_withheld` kind) beside this bootstrap must not
    #   crash the chain (the bootstrap tolerates that module's skew). Cannot be red on af41d65c (that bootstrap never asks
    #   for the kind): it pins the R3 code itself; its evidence is the revert check.
    old_lane = None
    try:
        p = subprocess.run(["git", "-C", R3_REPO, "show", "%s:cysjavis-pack/bin/javis_lane.py" % R3_OLD],
                           capture_output=True, timeout=30)
        old_lane = p.stdout if p.returncode == 0 and p.stdout else None
    except Exception:
        old_lane = None
    if old_lane is None:
        print("  SKIP 35-R1-P-1: git history not reachable — measurement unavailable (not a pass)")
    else:
        binc = os.path.join(tempfile.mkdtemp(prefix="boot-r3-skew-"), "bin")
        try:
            shutil.copytree(os.path.dirname(SCRIPT), binc, ignore=shutil.ignore_patterns("tests", "__pycache__"))
            with open(os.path.join(binc, "javis_lane.py"), "wb") as f:
                f.write(old_lane)
            for name, kw, want_marker in (
                    ("a signal run", {"boot_stdout": r3_boot_stdout(r3_row("worker", "directive_unknown", True))}, False),
                    ("a normal run", {}, True)):
                tmp = tempfile.mkdtemp(prefix="boot-r3-skewrun-")
                try:
                    env, home = make_env(tmp, **kw)
                    rr = subprocess.run([PY, os.path.join(binc, "javis_bootstrap.py")], capture_output=True, text=True,
                                        encoding="utf-8", env=env, timeout=120)
                    res = r3_collect(tmp, home, rr.returncode, rr.stdout, rr.stderr)
                    check("35-R1-P-1 older javis_lane (no ready_withheld kind), %s: ★the chain completes (exit 0), marker %s"
                          % (name, "written" if want_marker else "withheld (sidecar at the derived name)"),
                          rr.returncode == 0 and res["marker"] is want_marker
                          and (want_marker or res["sidecars"] == ["ready-withheld.json"]), r3_brief(res))
                finally:
                    shutil.rmtree(tmp, ignore_errors=True)
        finally:
            shutil.rmtree(os.path.dirname(binc), ignore_errors=True)
    # R1-P-2 · a readable status WITHOUT a surfaces list says nothing about any pane: unreadable → the live record
    #   withholds (basis unmeasured), never "every pane is gone" (a false ready)
    r = r3_run(status_json=json.dumps({"daemon": {"started_at": R3_DAEMON}}), records_lines=[r3_record()],
               boot_stdout=r3_boot_stdout(r3_row("cso", "already_alive", True), r3_row("worker", "already_alive", True)))
    rows = (r["sidecar"] or {}).get("rows") or []
    check("35-R1-P-2 prerequisite: exit 0 (no surfaces → deficit → boot ran)", r["code"] == 0 and r["boot_called"],
          r3_brief(r))
    check("35-R1-P-2 ★a status without a surfaces list is UNREADABLE → marker ABSENT, the record row is basis unmeasured",
          not r["marker"] and any(x.get("source") == "record" and x.get("basis") == "unmeasured"
                                  and "no surfaces list" in str(x.get("why", "")) for x in rows),
          "%s rows=%s" % (r3_brief(r), json.dumps(rows, ensure_ascii=False)[:300]))
    # R1-P3-1 · ⑧ names the basis (unmeasured is not a value)
    r = r3_run(boot_stdout="boot 완료: (no json line)\n")
    m = r["summary"].get("marker") or ""
    check("35-R1-P3-1 ★⑧ unmeasured: ready_bases ['unmeasured'] · the marker note says 'readiness unmeasured (' and "
          "neither 'not ready' nor 'UNKNOWN'",
          r["summary"].get("ready_bases") == ["unmeasured"] and "readiness unmeasured (" in m
          and "not ready" not in m and "UNKNOWN" not in m, json.dumps(r["summary"], ensure_ascii=False)[:300])
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True, surface_ref="surface:7")))
    m = r["summary"].get("marker") or ""
    check("35-R1-P3-1 ⑧ UNKNOWN-class: ready_bases ['unknown'] · the marker note names the pane",
          r["summary"].get("ready_bases") == ["unknown"] and "UNKNOWN" in m and "surface:7" in m,
          json.dumps(r["summary"], ensure_ascii=False)[:300])
    # R1-P-3 · a notice that is not sent (exit 3: coalesced / no master) is not a failed step
    r = r3_run(boot_stdout=r3_boot_stdout(r3_row("worker", "directive_unknown", True)), notice_exit=3)
    st = [x for x in r["steps"] if x.get("step") == "④boot-ready"]
    check("35-R1-P-3 ★a not-sent notice (exit 3): the ④boot-ready step has exit 0 and its detail names the rc",
          len(st) == 1 and st[0].get("exit") == 0 and "kept-pane notice exit 3" in st[0].get("detail", ""),
          "%r" % [(x.get("step"), x.get("exit"), x.get("detail", "")[:120]) for x in st])


for _name, _fn in (("35-R1", r3_r1),):
    if os.environ.get("BOOT_R3_ONLY") and _name not in os.environ["BOOT_R3_ONLY"].split(","):
        continue
    r3_section(_name, _fn)


# ── 36. REFLECTION-2 fix R2-P-1: the deferral's own stderr line never reaches the OLD consumers ──
#    The real `cys boot` under the deferral pair ALWAYS ends stderr with `[kept-pane] notice deferred to the caller (N
#    record(s) written to <out>)` — also with 0 records; a boot run without the pair never printed it. The old consumers
#    (`_boot_fatal_verdict` / `_boot_was_busy` / `_boot_gate_pending_verdict` / the ④boot step) parse the merged text up to
#    its END, so that line alone made a no-launch boot's JSON unreadable to them: same verdicts, but the fatal reason fell
#    back to "--json 계약 소비 불가" and a gate-pending boot lost its role list. The stub prints the real line only with
#    `boot_deferral_line=True`; each "control" row runs the same boot without it (it passes before and after the fix).
def r3_r2():
    missing = r3_row("worker", "missing", True, install_hint="install the worker CLI (stub hint)")
    for name, kw in (("with the real deferral line", {"boot_deferral_line": True}),
                     ("control: no deferral line", {})):
        r = r3_run(boot_exit=1, boot_stdout=r3_boot_stdout(missing), **kw)
        check("36-R2-P-1 %s: prerequisite: mandatory missing, boot exit 1 → bootstrap exit 4 (verdict)" % name,
              r["code"] == 4, r3_brief(r))
        check("36-R2-P-1 %s: ★the fatal reason is the structured one (worker=missing [hint]), never the '--json 계약 "
              "소비 불가' fallback" % name,
              "의무(Fatal) 역할 기동 실패: worker=missing [install the worker CLI (stub hint)]" in r["err"]
              and "--json 계약 소비 불가" not in r["err"], r["err"][-400:].replace("\n", " | "))
    gated = r3_row("worker", "gate_pending", True, reason="first-run gate (stub)")
    for name, kw in (("with the real deferral line", {"boot_deferral_line": True}),
                     ("control: no deferral line", {})):
        r = r3_run(boot_exit=78, boot_stdout=r3_boot_stdout(gated), **kw)
        st = [x for x in r["steps"] if x.get("step") == "④boot-gate-pending"]
        check("36-R2-P-1 %s: ★a gate-pending boot (exit 78) names its role in the ④boot-gate-pending step, never "
              "'(--json 소비 불가'" % name,
              len(st) == 1 and "worker=gate_pending" in st[0].get("detail", "")
              and "(--json 소비 불가" not in st[0].get("detail", ""),
              "%r" % [(x.get("step"), x.get("detail", "")[:160]) for x in st])
    so = r3_boot_stdout(r3_row("cso", "already_alive", True), r3_row("worker", "already_alive", True))
    r = r3_run(boot_stdout=so, boot_deferral_line=True)
    st = [x for x in r["steps"] if x.get("step") == "④boot"]
    check("36-R2-P-1 ★a clean no-launch boot: the ④boot detail an OLD consumer gets is the boot's text WITHOUT the "
          "deferral line (== so.strip()[-2000:])",
          len(st) == 1 and st[0].get("detail") == so.strip()[-2000:], "%r" % [x.get("detail", "")[-200:] for x in st])
    rd = [x for x in r["steps"] if x.get("step") == "④boot-ready"]
    check("36-R2-P-1 ★the deferral line is not dropped: the ④boot-ready detail says what cys boot said",
          len(rd) == 1 and "cys boot said: [kept-pane] notice deferred to the caller (0 record(s) written to "
          in rd[0].get("detail", ""), "%r" % [x.get("detail", "")[-300:] for x in rd])
    check("36-R2-P-1 prerequisite: that clean run completes (exit 0) with the marker written", r["code"] == 0
          and r["marker"], r3_brief(r))


for _name, _fn in (("36-R2", r3_r2),):
    if os.environ.get("BOOT_R3_ONLY") and _name not in os.environ["BOOT_R3_ONLY"].split(","):
        continue
    r3_section(_name, _fn)


print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
