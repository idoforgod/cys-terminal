#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_silentpass_pack_c2.py — U4 '조용한 통과' 팩 런타임 드러내기(0.14.41 WP-C2 ②③④⑤).

설계 정본(수정설계-0.14.41 §3 U4 · C2 목록)과 반박(U4c.refute D1·D2·M6·R11) 반영 결정:
  ② preflight C56·C57: settings.json **판독·파싱 실패**는 '누수 0 PASS' 가 아니라 WARN.
     단 **파일 부재(ENOENT)는 PASS 유지** — discover_claude_settings 는 G7 규약으로 settings.json
     이 아직 없는 프로필 디렉터리도 대상에 넣는다(부재 = 훅 0개가 정답 · 새 오경보 금지).
  ③ preflight 요약 줄 **안에** '미측정 k'(판정 불가 SKIP 수). 요약 줄은 **항상 마지막 줄**이다 —
     javis_checklist 가 preflight 출력의 마지막 비어 있지 않은 줄을 SessionStart 컨텍스트에 싣는다
     (반박 M6). 그래서 id 목록·FAIL 안내는 요약 줄 **앞**에 찍는다. exit code 불변. JSON 은 추가만.
  ④ javis_bootstrap: preflight 스크립트 부재 → boot-last `preflight_state: "absent"` + stderr ⚠
     (rc 0 비치명 계약 불변).
  ⑤ cys-dept promote-if-pending --request-only: feed push 실패를 '발행' 으로 적지 않는다 —
     실패면 stderr 에 rc 와 함께 고지(exit 0 유지 · 부트 ⑦ 무차단).

밀폐: 임시 HOME · 스텁 cys($HOME/.local/bin — cys-dept PATH prepend 1순위) · 라이브 데몬·팩 무접촉.
실행(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_silentpass_pack_c2.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 SILENTPASS-PACK-C2-OK.
"""
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
PREFLIGHT = os.path.join(BIN, "javis_preflight.py")
BOOTSTRAP = os.path.join(BIN, "javis_bootstrap.py")
DEPT = os.path.join(BIN, "cys-dept")
PY = sys.executable or "python3"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _w(path, body, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def _clean_env(extra=None):
    env = {k: v for k, v in os.environ.items()
           if not (k.startswith("CYS_") or k.startswith("JAVIS_") or k.startswith("AITERM_"))}
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env.update(extra or {})
    return env


# ═══════════════════════ ② C56·C57 판독 실패 → WARN (ENOENT 는 PASS) ═══════════════════════
sys.path.insert(0, BIN)
import javis_preflight as pf        # noqa: E402

LEAK_HOOK = {"hooks": {"SessionStart": [{"hooks": [
    {"type": "command", "command": "sh /Users/x/.cys/pack-dept-sales/hooks/session-start.sh"}]}]}}
CLEAN = {"hooks": {"SessionStart": [{"hooks": [
    {"type": "command", "command": "sh /Users/x/.cys/pack/hooks/session-start.sh"}]}]}}


def _c5657(paths, fix=False):
    """discover_claude_settings 를 주어진 경로로 고정하고 C56·C57 만 돌린다 → {id: (status, detail)}."""
    saved = pf.discover_claude_settings
    saved_env = {k: os.environ.get(k) for k in ("CYS_PACK_DIR", "CYS_ACCOUNT_DIR")}
    try:
        pf.discover_claude_settings = lambda: list(paths)
        os.environ["CYS_PACK_DIR"] = "/nonexistent/.cys/pack"   # 비부서 팩 컨텍스트(C56 진입)
        os.environ.pop("CYS_ACCOUNT_DIR", None)
        p = pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report")
        out = {}
        for fn, cid in ((p.c56_dept_hook_leak, "C56.dept-hook-leak"),
                        (p.c57_temp_hook_leak, "C57.temp-hook-leak")):
            try:
                fn()
            except Exception as e:           # noqa: BLE001 — 크래시도 관측 결과다(PASS 아님)
                out[cid] = ("EXC", "%s: %s" % (type(e).__name__, e))
        out.update({r["id"]: (r["status"], r["detail"]) for r in p.results})
        return out
    finally:
        pf.discover_claude_settings = saved
        for k, v in saved_env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


tmp = tempfile.mkdtemp(prefix="c2-c5657-")
try:
    absent = os.path.join(tmp, ".claude-2", "settings.json")          # 디렉터리만 있고 파일 없음(G7)
    os.makedirs(os.path.dirname(absent), exist_ok=True)
    broken = os.path.join(tmp, ".claude", "settings.json")
    _w(broken, '{"hooks": {"SessionStart": [ ')                         # 찢긴 쓰기
    clean = os.path.join(tmp, ".claude-3", "settings.json")
    _w(clean, json.dumps(CLEAN))
    leak = os.path.join(tmp, ".claude-4", "settings.json")
    _w(leak, json.dumps(LEAK_HOOK))
    listy = os.path.join(tmp, ".claude-5", "settings.json")
    _w(listy, "[1, 2]")                                                 # 최상위가 객체가 아님

    r = _c5657([absent])
    check("2a ENOENT(프로필 디렉터리만) → C56 PASS 유지(새 오경보 0)",
          r.get("C56.dept-hook-leak", ("?",))[0] == pf.PASS, repr(r.get("C56.dept-hook-leak")))
    check("2b ENOENT → C57 PASS 유지", r.get("C57.temp-hook-leak", ("?",))[0] == pf.PASS,
          repr(r.get("C57.temp-hook-leak")))
    r = _c5657([clean])
    check("2c 정상 settings → C56·C57 PASS(무회귀)",
          r.get("C56.dept-hook-leak", ("?",))[0] == pf.PASS
          and r.get("C57.temp-hook-leak", ("?",))[0] == pf.PASS, repr(r))
    r = _c5657([broken, clean])
    c56 = r.get("C56.dept-hook-leak", ("?", ""))
    c57 = r.get("C57.temp-hook-leak", ("?", ""))
    check("2d 깨진 JSON 섞임 → C56 WARN(PASS 아님)", c56[0] == pf.WARN, repr(c56))
    check("2e 깨진 JSON 섞임 → C57 WARN(PASS 아님)", c57[0] == pf.WARN, repr(c57))
    check("2f WARN 문안이 '판독 불가'·'PASS 아님'을 말한다",
          "판독 불가" in c56[1] and "PASS 아님" in c56[1], c56[1][:200])
    r = _c5657([listy])
    check("2g 최상위가 객체 아닌 JSON → C56 WARN(크래시·PASS 아님)",
          r.get("C56.dept-hook-leak", ("?",))[0] == pf.WARN, repr(r.get("C56.dept-hook-leak")))
    r = _c5657([broken, leak])
    c56 = r.get("C56.dept-hook-leak", ("?", ""))
    check("2h 누수 + 판독 불가 → FAIL(누수) 유지 + 판독 불가 병기",
          c56[0] == pf.FAIL and "판독 불가" in c56[1], repr(c56))
    if hasattr(os, "geteuid") and os.geteuid() != 0 and os.name == "posix":
        denied = os.path.join(tmp, ".claude-6", "settings.json")
        _w(denied, json.dumps(CLEAN))
        os.chmod(denied, 0)
        try:
            r = _c5657([denied])
            check("2i 권한 거부 → C56 WARN", r.get("C56.dept-hook-leak", ("?",))[0] == pf.WARN,
                  repr(r.get("C56.dept-hook-leak")))
        finally:
            os.chmod(denied, stat.S_IRUSR | stat.S_IWUSR)
    else:
        print("SKIP 2i 권한 거부 — root 또는 비POSIX(chmod 0 재현 불가 · 통과 아님)")
finally:
    shutil.rmtree(tmp, ignore_errors=True)


# ═══════════════════════ ③ 요약 줄 '미측정 k' · 요약은 항상 마지막 줄 ═══════════════════════
def _preflight(args, env_extra=None):
    env = _clean_env(env_extra)
    return subprocess.run([PY, PREFLIGHT] + list(args), capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=120, env=env)


def _last_line(text):
    last = ""
    for ln in (text or "").splitlines():
        if ln.strip():
            last = ln.strip()
    return last


tmp = tempfile.mkdtemp(prefix="c2-summary-")
try:
    home = os.path.join(tmp, "home")
    os.makedirs(home)
    nopath = os.path.join(tmp, "empty-bin")          # cys 가 없는 PATH → C12 판정 불가
    os.makedirs(nopath)
    base = {"HOME": home, "PATH": nopath + os.pathsep + "/usr/bin" + os.pathsep + "/bin",
            "CYS_PACK_DIR": os.path.join(tmp, "no-such-pack")}
    r = _preflight(["--only", "C12"], base)
    last = _last_line(r.stdout)
    check("3a 미측정만(C12 cys 부재) → rc 0 불변", r.returncode == 0, "rc=%s" % r.returncode)
    check("3b 요약 줄에 '미측정 1' 이 들어 있다", "미측정 1" in last, repr(last))
    check("3c 요약 줄 형식 유지(preflight: READY … FAIL 0 · WARN n · 미측정 k · 검사 N)",
          last.startswith("preflight: READY") and "FAIL 0" in last and "검사 1" in last, repr(last))
    check("3d 미측정 id 목록은 요약 줄 앞에(마지막 줄 아님)", "C12.daemon" in r.stdout
          and "C12.daemon" not in last, r.stdout[-300:])
    r = _preflight(["--only", "C12", "--json"], base)
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        doc = {}
    check("3e JSON unmeasured 목록(추가 필드) = ['C12.daemon']", doc.get("unmeasured") == ["C12.daemon"],
          repr(doc.get("unmeasured")))
    check("3f JSON 기존 키 불변(ok·fails·warns·checks)", all(k in doc for k in ("ok", "fails", "warns",
                                                                                 "checks")), repr(sorted(doc)))
    # FAIL 이 있어도 마지막 줄은 요약이다(javis_checklist 소비 — 종전엔 'FAIL 항목을 수리하고…' 였다)
    r = _preflight(["--only", "C01", "--only", "C12"], base)
    last = _last_line(r.stdout)
    check("3g FAIL 있음 → rc 1 불변", r.returncode == 1, "rc=%s" % r.returncode)
    check("3h FAIL 있어도 마지막 줄 = 요약(NOT READY · 미측정 1)",
          last.startswith("preflight: NOT READY") and "미측정 1" in last and "FAIL 1" in last,
          repr(last))
    check("3i FAIL 안내 문장은 요약 앞에 남아 있다", "FAIL 항목을 수리하고" in r.stdout, r.stdout[-400:])
    # javis_checklist 가 싣는 줄이 요약이다(소비자 실측)
    import javis_checklist as jc     # noqa: E402
    cmd = "%s %s --only C01 --only C12" % (json.dumps(PY), json.dumps(PREFLIGHT))
    saved = {k: os.environ.get(k) for k in ("HOME", "PATH", "CYS_PACK_DIR")}
    try:
        os.environ.update(base)
        code, cl_last = jc.run_preflight(cmd, timeout=120)
    finally:
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
    check("3j javis_checklist 가 싣는 마지막 줄 = 요약(판정·미측정 보존)",
          (cl_last or "").startswith("preflight: NOT READY") and "미측정" in (cl_last or ""),
          repr(cl_last))
    # 적용 불가 SKIP 만(C11b Windows 아님 등)은 미측정이 아니다 — 0 이어야 한다
    r = _preflight(["--only", "C01"], dict(base, CYS_PACK_DIR=tmp))
    last = _last_line(r.stdout)
    check("3k 미측정 0 이면 '미측정 0'(형식 일관 · 경보 피로 없음)", "미측정 0" in last, repr(last))
finally:
    shutil.rmtree(tmp, ignore_errors=True)


# ═══════════════════════ ④ bootstrap preflight 부재 → preflight_state='absent' ═══════════════════════
def _boot(with_preflight):
    home = tempfile.mkdtemp(prefix="c2-boot-")
    pack = os.path.join(home, ".cys", "pack")
    mockbin = os.path.join(home, "mockbin")
    log = os.path.join(home, "cys.log")
    lst = os.path.join(home, "list.txt")
    roles = ["cso", "worker", "reviewer-gemini", "reviewer-codex"]
    with open(lst, "w") as f:                          # 결손 0 → 게이트·④ 생략(빠른 완주)
        f.write("".join("surface:%d\trole=%s\tpid=%d\texited=false\t\t\n" % (i, r, 100 + i)
                        for i, r in enumerate(roles)))
    _w(os.path.join(mockbin, "cys"),
       '#!/bin/bash\necho "$@" >> "%s"\ncase "$1" in\n  list) cat "%s"; exit 0;;\n'
       '  *) exit 0;;\nesac\n' % (log, lst), 0o755)
    _w(os.path.join(pack, "bin", "javis_orchestra.py"), "#!/usr/bin/env python3\nimport sys\nsys.exit(0)\n", 0o755)
    if with_preflight:
        _w(os.path.join(pack, "bin", "javis_preflight.py"),
           "#!/usr/bin/env python3\nprint('preflight: READY — FAIL 0 · WARN 0 · 미측정 0 · 검사 0')\n", 0o755)
    with open(os.path.join(pack, "agents.json"), "w", encoding="utf-8") as f:
        json.dump({a: {"cmd": PY} for a in ("claude", "gemini", "codex")}, f)
    env = _clean_env({"HOME": home, "CYS_PACK_DIR": pack,
                      "PATH": mockbin + os.pathsep + os.environ.get("PATH", ""),
                      "CYS_BOOT_CHECK_RETRIES": "1", "CYS_BOOT_CHECK_INTERVAL_S": "0"})
    r = subprocess.run([PY, BOOTSTRAP, "run"], capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=180, env=env)
    data = None
    state = os.path.join(home, ".cys", "state")
    for cand in sorted(os.listdir(state) if os.path.isdir(state) else []):
        if cand.startswith("boot-last") and cand.endswith(".json"):
            data = json.load(open(os.path.join(state, cand), encoding="utf-8"))
            break
    shutil.rmtree(home, ignore_errors=True)
    return r, data or {}


r, data = _boot(with_preflight=False)
check("4a preflight 부재 → 부트 비치명(rc 0 불변)", r.returncode == 0, "rc=%s stderr=%r" % (r.returncode, r.stderr[-300:]))
check("4b boot-last preflight_state='absent'", data.get("preflight_state") == "absent",
      repr(data.get("preflight_state")))
check("4c stderr ⚠ 고지(팩 불완전 가능)", "preflight 스크립트 부재" in r.stderr, r.stderr[-300:])
pf_steps = [s for s in data.get("steps", []) if s.get("step") == "①preflight"]
check("4d ① 단계 rc 0 기록 유지(비치명 계약)", bool(pf_steps) and pf_steps[0].get("exit") == 0,
      repr(pf_steps))
r, data = _boot(with_preflight=True)
check("4e 대조: preflight 있으면 preflight_state 부재(absent 아님)",
      data.get("preflight_state") != "absent" and r.returncode == 0,
      "rc=%s state=%r" % (r.returncode, data.get("preflight_state")))


# ═══════════════════════ ⑤ cys-dept promote-if-pending --request-only feed push 실패 고지 ═══════════════════════
def _dept(push_rc):
    tmp = tempfile.mkdtemp(prefix="c2-dept-")
    home = os.path.join(tmp, "home")
    bindir = os.path.join(home, ".local", "bin")          # cys-dept PATH prepend 1순위
    os.makedirs(os.path.join(home, ".cys", "state"), exist_ok=True)
    reg = os.path.join(home, ".cys", "depts.json")
    with open(reg, "w", encoding="utf-8") as f:
        json.dump({"depts": {"d0": {}}}, f)
    open(os.path.join(home, ".cys", ".master-bootstrapped"), "w").close()
    open(os.path.join(home, ".cys", "state", "ceo-pending"), "w").close()
    _w(os.path.join(bindir, "cys"),
       '#!/bin/sh\necho "cys $@" >> "%s/calls.log"\n'
       'case "$1 $2" in "feed push") exit %d;; "feed list") exit 0;; esac\n'
       'case "$1" in status) exit 1;; esac\nexit 0\n' % (tmp, push_rc), 0o755)
    _w(os.path.join(bindir, "cysd"), "#!/bin/sh\nexit 1\n", 0o755)   # 실 cysd 해소 차단(밀폐)
    env = _clean_env({"HOME": home, "CYS_DEPTS_JSON": reg,
                      "PATH": bindir + os.pathsep + os.environ.get("PATH", "")})
    r = subprocess.run(["bash", DEPT, "promote-if-pending", "--request-only"], capture_output=True,
                       text=True, encoding="utf-8", errors="replace", timeout=60, env=env)
    calls = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read() \
        if os.path.exists(os.path.join(tmp, "calls.log")) else ""
    shutil.rmtree(tmp, ignore_errors=True)
    return r, calls


r, calls = _dept(push_rc=3)
check("5a feed push 실패여도 exit 0 유지(부트 ⑦ 무차단)", r.returncode == 0,
      "rc=%s err=%r" % (r.returncode, r.stderr[-300:]))
check("5b push 가 실제로 시도됐다(계측 가능)", "cys feed push" in calls, repr(calls[-200:]))
check("5c 실패를 '발행' 으로 적지 않는다", "알림 발행(request-only)" not in r.stdout, r.stdout[-300:])
check("5d stderr 에 실패·rc 고지", "발행 실패" in r.stderr and "rc=3" in r.stderr, r.stderr[-300:])
r, calls = _dept(push_rc=0)
check("5e 대조: push 성공 → '발행(request-only)' · stderr 실패 고지 없음",
      "알림 발행(request-only)" in r.stdout and "발행 실패" not in r.stderr and r.returncode == 0,
      "out=%r err=%r" % (r.stdout[-200:], r.stderr[-200:]))

print("\n%d FAIL" % len(fails) if fails else "\nSILENTPASS-PACK-C2-OK")
sys.exit(1 if fails else 0)
