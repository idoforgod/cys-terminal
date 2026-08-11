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
             br_exit=0, pack_dept=None):
    """임시 HOME + 가짜 팩 + 스텁 생성 → 환경 dict 반환.
    pack_dept=<부서명>: 팩을 ~/.cys/pack-dept-<부서명>에 만들고 CYS_PACK_DIR로 지정
    (증분1 레인↔팩 정합 가드 — 부서 소켓 레인은 같은 부서 팩 페어링 필수·불일치=exit 8)."""
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
    w(os.path.join(bindir, "cys"), (
        "#!/bin/sh\n"
        "echo \"cys $@\" >> \"%s/calls.log\"\n"
        "case \"$1\" in\n"
        "  ping) exit %d;;\n"
        "  claim-role) [ %d -ne 0 ] && echo 'claim_denied: privileged role held by live surface' >&2; exit %d;;\n"
        "  boot) exit %d;;\n"
        "  --version) echo 'cys 0.0.0-stub'; exit 0;;\n"
        "esac\nexit 0\n") % (tmp, ping_exit, claim_exit, claim_exit, boot_exit))
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
        "    if [ -f \"%(t)s/master.flag\" ]; then\n"
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

# ── 5-fb-win/dept. 게이트 확인: 부서 레인에서는 폴백 미발동(부서 안에 부서 금지) ──
tmp = tempfile.mkdtemp(prefix="boot-t5fbd-")
env, home = make_env(tmp, claim_exit=1, socket="/x/cys-dept-alpha/cys.sock", pack_dept="alpha")
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

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
