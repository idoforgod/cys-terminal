#!/usr/bin/env python3
"""test_dept_teardown_atomicity.py — down-sock D8 봉쇄·묘비 배선 핀 (WP-3/T5).

기능시험(가짜 HOME·스텁 cys·스텁 phoenix — 실 데몬/실 phoenix 무접촉):
  A) 정상 down-sock: 역인덱스 성공 → reg_remove + dept 묘비 기록
  B) D8: 역인덱스 실패(빈 레지스트리)여도 소켓 슬러그에서 name 파생 → 묘비 기록(무음 구멍 봉쇄)
  Bw) Windows named pipe 문자열에서도 파생
  C) 비표준 소켓 → 파생 실패 시 보수적 skip(종전 거동)
정적 트립와이어 핀(배선 소실 검출 — 기능시험이 무거운 생성 경로용):
  launch/allocate/create의 dept_tombstone_remove 배선 · rotate의 CYS_DEPT_ROTATE=1 가드 ·
  helper의 --remove 플래그·rotate 가드. (launch/allocate/create 기능시험은 실 cysd 필요 — CI 통합 영역.)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
DEPT = os.path.join(SELF, "..", "cys-dept")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def setup(tmp, reg_depts):
    home = os.path.join(tmp, "home")
    bindir = os.path.join(home, ".local", "bin")
    fakepack = os.path.join(tmp, "fakepack", "bin")
    os.makedirs(bindir, exist_ok=True)
    os.makedirs(fakepack, exist_ok=True)
    reg = os.path.join(home, ".cys", "depts.json")
    os.makedirs(os.path.dirname(reg), exist_ok=True)
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": reg_depts}, f)
    # 스텁 cys: identify 실패(pid 빈값 → kill 생략)·기타 성공·전 호출 기록(A4 데몬 묘비 핀용)
    with open(os.path.join(bindir, "cys"), "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\necho \"cys $@\" >> \"%s/calls.log\"\n"
                "case \"$1\" in identify) exit 1;; esac\nexit 0\n" % tmp)
    os.chmod(os.path.join(bindir, "cys"), 0o755)
    # 스텁 phoenix: 인자 기록만
    with open(os.path.join(fakepack, "javis_phoenix.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write("import sys\nopen(%r, 'a').write(' '.join(sys.argv[1:]) + '\\n')\n"
                % os.path.join(tmp, "phoenix.log"))
    env = dict(os.environ)
    env.update({"HOME": home, "CYS_DEPTS_JSON": reg,
                "CYS_PACK_DIR": os.path.join(tmp, "fakepack"),
                "PATH": bindir + os.pathsep + env.get("PATH", "")})
    for k in ("CYS_ROLE", "CYS_SOCKET"):
        env.pop(k, None)
    return env, home, reg


def phoenix_log(tmp):
    p = os.path.join(tmp, "phoenix.log")
    return open(p, encoding="utf-8").read() if os.path.exists(p) else ""


def run(env, *args):
    r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stdout + r.stderr


# ── A. 정상 down-sock: 역인덱스 성공 → reg_remove + 묘비 ──
tmp = tempfile.mkdtemp(prefix="dt-a-")
sockA = "/tmp/x/cys-dept-dept-3/cys.sock"
env, home, reg = setup(tmp, {"dept-3": {"socket": sockA}})
code, out = run(env, "down-sock", sockA)
check("A1 down-sock exit 0", code == 0, out[-150:])
check("A2 reg_remove(레지스트리 비움)",
      json.load(open(reg, encoding="utf-8"))["depts"] == {})
check("A3 묘비 기록(tombstone dept-3 --dept)", "tombstone dept-3 --dept" in phoenix_log(tmp))
calls_a = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read() if \
    os.path.exists(os.path.join(tmp, "calls.log")) else ""
check("A4 데몬 묘비 병행 기록(D-IMPL-2)", "tombstone --dept -- dept-3" in calls_a, calls_a[-120:])
# ↑ D3(i) 인자 위생: cys 호출은 `--dept [--remove] -- "$name"`(clap `--` 종단) 형식 — 핀 의도
#   (데몬 묘비 병행 set)는 불변, 기대 형식만 현행 소스와 정합(v4 A14 기대값 갱신).
shutil.rmtree(tmp)

# ── B. D8: 역인덱스 실패여도 슬러그 파생 → 묘비 기록 ──
tmp = tempfile.mkdtemp(prefix="dt-b-")
env, home, reg = setup(tmp, {})  # 빈 레지스트리 = 역인덱스 실패
code, out = run(env, "down-sock", "/tmp/x/cys-dept-dept-7/cys.sock")
check("B1 exit 0", code == 0)
check("B2 D8 파생(name=dept-7) 고지", "파생(dept-7)" in out, out[-200:])
check("B3 파생 name으로 묘비 기록", "tombstone dept-7 --dept" in phoenix_log(tmp))
shutil.rmtree(tmp)

# ── Bw. Windows named pipe 문자열 파생 ──
tmp = tempfile.mkdtemp(prefix="dt-bw-")
env, home, reg = setup(tmp, {})
code, out = run(env, "down-sock", r"\\.\pipe\cys-dept-dept-9")
check("Bw1 pipe 파생 묘비", "tombstone dept-9 --dept" in phoenix_log(tmp), out[-200:])
shutil.rmtree(tmp)

# ── C. 비표준 소켓 → 보수적 skip(묘비 없음·exit 0) ──
tmp = tempfile.mkdtemp(prefix="dt-c-")
env, home, reg = setup(tmp, {})
code, out = run(env, "down-sock", "/tmp/custom-daemon.sock")
check("C1 비표준 exit 0", code == 0)
check("C2 묘비 미기록(보수)", "tombstone" not in phoenix_log(tmp))
shutil.rmtree(tmp)

# ── 정적 트립와이어: 배선 소실 검출 ──
src = open(DEPT, encoding="utf-8").read()
check("W1 launch 배선", src.count("dept_tombstone_remove \"$name\"") >= 3,
      "count=%d(launch/allocate/create)" % src.count("dept_tombstone_remove \"$name\""))
check("W2 rotate 가드(export)", "CYS_DEPT_ROTATE=1 bash \"$0\" launch" in src)
check("W3 helper rotate 가드", '[ "${CYS_DEPT_ROTATE:-}" = "1" ] && return 0' in src)
check("W4 helper --remove", "--dept --remove" in src)
check("W5 D8 파생 로직", "cys-dept-[^/]*" in src)
# ★D-IMPL-2 대칭 핀: phoenix 묘비와 데몬 묘비는 set/remove가 항상 쌍으로 — 한쪽만 있으면
# "삭제→재생성→재시작 시 새 부서 살해"(데몬 묘비 잔존) 또는 부활 구멍(데몬 묘비 미기록).
# D3(i) 인자 위생 형식(`--dept [--remove] -- "$1"`)으로 기대 갱신 — 핀 의도(set/remove 쌍 배선)는 불변.
check("W6 데몬 묘비 set 병행", '"$CYS" tombstone --dept -- "$1"' in src)
check("W7 데몬 묘비 remove 병행", '"$CYS" tombstone --dept --remove -- "$1"' in src)


# ★파서 견고화(v4 A14): 종전 `split("  down)")[1].split(";;")[0]`는 down 플래그 파서(기능2 하드닝)의
#   내부 case문 인라인 ';;'(`--purge-state) purge_state=1 ;;`)에서 블록을 조기 절단해 W8이
#   ValueError crash — 분기 종결자는 '공백+";;"뿐인 독립 줄'로만 인식한다(인라인 ';;'는 통과).
def case_arm(source, label):
    body = source.split("\n  %s)" % label, 1)[1]
    kept = []
    for ln in body.split("\n"):
        if ln.strip() == ";;":
            break
        kept.append(ln)
    return "\n".join(kept)


def precedes(block, first, second):
    """두 실행문이 모두 실재 ∧ first 선행이면 True — 부재는 FAIL로 보고(crash 금지)."""
    i, j = block.find(first), block.find(second)
    return 0 <= i < j


# ★R7(적대검증 W1): down/down-sock 모두 묘비가 reg_remove보다 선행(set -e abort 시 등재+미묘비 창 봉쇄)
_down = case_arm(src, "down")
check("W8 down: 묘비 선기록", precedes(_down, 'dept_tombstone "$name"', 'reg_remove "$name"'))
_ds = case_arm(src, "down-sock")
check("W9 down-sock: 묘비 선기록(실행문 정박 — 주석 오매치 방지)",
      precedes(_ds, 'dept_tombstone "$name"', 'reg_remove "$name"'))
check("W10 ★R11 해소 실패 WARN 가시화", "데몬 묘비 해소 미확정" in src)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
