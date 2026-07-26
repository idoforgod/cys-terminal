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
check("A4 데몬 묘비 병행 기록(D-IMPL-2)", "tombstone dept-3 --dept" in calls_a, calls_a[-120:])
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
check("W6 데몬 묘비 set 병행", '"$CYS" tombstone "$1" --dept' in src)
check("W7 데몬 묘비 remove 병행", '"$CYS" tombstone "$1" --dept --remove' in src)
# ★R7(적대검증 W1): down/down-sock 모두 묘비가 reg_remove보다 선행(set -e abort 시 등재+미묘비 창 봉쇄)
# ★W8 앵커 수리(CU-5A 라운드): 종전 `split(";;")`은 down 아암을 **첫 `;;`까지**로 잘랐다. down에
# 인자 파싱 하드닝(`for _a; case "$_a" in --purge-state) … ;;`)이 들어오며 그 내부 `;;`가 먼저
# 걸려 슬라이스가 묘비 줄에 닿지 못했고 `.index()`가 ValueError로 스크립트를 통째로 중단시켰다
# (기준선 42/43 FAIL의 정체). 검사 의도(묘비 선기록 순서)는 그대로 두고, 아암 경계를 **다음 아암
# 라벨**(`\n  down-sock)`)로 잡아 내부 case의 `;;`에 면역시킨다. 아래 W9도 같은 위험(현재는
# 내부 case 부재로 통과 중)이라 동일 규약으로 정박한다.
_down = src.split("\n  down)", 1)[1].split("\n  down-sock)", 1)[0]
check("W8 down: 묘비 선기록", _down.index('dept_tombstone "$name"') < _down.index('reg_remove "$name"'))
_ds = src.split("\n  down-sock)", 1)[1].split("\n  rotate)", 1)[0]
check("W9 down-sock: 묘비 선기록(실행문 정박 — 주석 오매치 방지)",
      _ds.index('dept_tombstone "$name"') < _ds.index('reg_remove "$name"'))
check("W10 ★R11 해소 실패 WARN 가시화", "데몬 묘비 해소 미확정" in src)

# ── CU-5A 정적 트립와이어(설계 DESIGN_scope-first-class.md §4 CU-5A "테스트") ──
# ★번호 주의: 설계는 이 3핀을 W10·W11·W12로 부르나 W10은 위 R11 핀이 **선점**(LOCKED 기준선의
#   이름을 바꾸면 기존 증거·CI 로그와 어긋난다) → stdout 오염 핀만 `W10a`로 두고 나머지는 설계 번호
#   그대로 쓴다. AC-9 "보호 핀 4종"의 W10 = 아래 W10a(stdout 계약)를 가리킨다.
def _arm(start, end):
    """case 아암 슬라이스 — 아암 라벨 경계로 정박(내부 case의 `;;` 면역 · W8 수리와 같은 규약)."""
    return src.split(start, 1)[1].split(end, 1)[0]


def _strip_comment(line):
    """따옴표 밖 `#`부터 잘라낸다 — 주석 안의 `>&2` 문구가 리다이렉션으로 오인되지 않게."""
    q = None
    for i, ch in enumerate(line):
        if q:
            if ch == q:
                q = None
        elif ch in "\"'":
            q = ch
        elif ch == "#":
            return line[:i]
    return line


_launch = _arm("\n  launch)", "\n  allocate)")
_alloc = _arm("\n  allocate)", "\n  create)")
# ★W10a(E15 계약 · AC-9 보호 핀): allocate stdout 마지막 줄은 확정 name(Tauri 파싱·871행 주석),
#   launch stdout은 사람용 상태줄뿐이다. 두 아암에 **새 stdout echo가 하나라도 늘면 FAIL** —
#   진단·경고는 전부 >&2 여야 한다(GUI 부서장 버튼 파손 회귀 봉쇄). 허용 목록=CU-5A 이전 원본 3종.
_ALLOWED_STDOUT = ("이미 가동 중 — 재사용", "가동 완료 (sock=", "ERROR: $name 데몬 기동 실패",
                   'echo "$name"')
_leaks = []
for _blk, _tag in ((_launch, "launch"), (_alloc, "allocate")):
    for _ln in _blk.splitlines():
        _s = _strip_comment(_ln).strip()
        if "echo " not in _s or _s.startswith("#"):
            continue
        if ">&2" in _s:
            continue
        if not any(a in _s for a in _ALLOWED_STDOUT):
            _leaks.append("%s: %s" % (_tag, _s[:90]))
check("W10a allocate/launch 신규 echo 전부 >&2 (stdout 오염 0)", not _leaks, "; ".join(_leaks))
check("W10a-2 allocate stdout 마지막 줄 = 확정 name",
      _strip_comment([l for l in _alloc.splitlines()
                      if "echo " in _strip_comment(l) and ">&2" not in _strip_comment(l)][-1]).strip()
      == 'echo "$name"')
# ★W11: launch spawn에 CYS_ACCOUNT_DIR 주입 — rotate(재기동)가 launch를 재사용하므로 이 배선이
#   빠지면 세대교체마다 부서 데몬이 계정 격리를 잃는다(AC-3 env 3종 생존).
check("W11 launch 블록 CYS_ACCOUNT_DIR 주입",
      'CYS_ACCOUNT_DIR="$acctdir"' in _launch and 'dept_account_dir "$name"' in _launch)
# ★W12: 값이 있는데 실물이 없으면 조용한 무주입이 아니라 fail-loud(exit 5) — 원사고(비격리 spawn) 형태 봉쇄.
check("W12 dept_account_dir fail-loud 분기", "return 5" in src.split("dept_account_dir(){", 1)[1]
      .split("\n}", 1)[0] and '[ "$acctdir_rc" = 5 ] && exit 5' in _launch)
# ★W12b: allocate의 account_dir 기록(create 예약 python `e['account_dir']=acctdir`과 대칭) —
#   미기록이면 allocate-born 부서가 계정 미상으로 남아 이후 launch/rotate가 복구에만 의존한다.
check("W12b allocate account_dir 레지스트리 기록(create 대칭)",
      'reg_set_account_dir "$name" "$acctdir"' in _alloc and "e['account_dir']=acctdir" in src)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
