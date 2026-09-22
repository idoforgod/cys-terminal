#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_lane_redirect.py — WP-B-hooks 레인 위임·공통 탐지 RED 검체.

무엇을 막는가: 개인 프로필의 본부 훅이 부서 레인에서 rc 0·stdout/stderr 0 으로
사라져 능력 deny·grill 차단·SessionStart 주입까지 무관측으로 꺼지는 결함.
  ① R-1 실제 reviewer Write deny 보존 + 같은 팩/reviewer·master 대조
  ② R-2 부서 스텁 실행 흔적 + 인자·stdin 그대로 전달
  ③ R-3 grill-gate/grill-stop exit 2·stderr 보존
  ④ R-4 대응 훅 부재 → exit 0·stdout 0·표식 생성/덮어쓰기
  ⑤ R-5 같은 팩·성공 위임에서는 표식 무생성
  ⑥ R-6 표식 쓰기 불가에서도 exit 0·stdout 0
  ⑦ R-7 이미 위임됨·심링크 변형에서 무한 위임 차단 · 판독 불가 대상 → 표식+exit 0
  ⑧ R-8 전 프리루드 경유 훅의 redirect 호출 위치 census(런처·하위 훅 포함)
  ⑨ R-9 혼재 프로필: base→dept 1회 + dept 직접 1회 = 카운터 2.
     위임은 1:1이다. 프로필의 이중 실행 방지 정본은 C56 불변식(글로벌 프로필에
     dept 훅 등록 0)이며, 여기서 두 등록을 임의로 dedup하지 않는다.
  ⑩ P-1 읽기 전용 C83: 표식 나이·실사용 SessionStart 설정·fix 무변경·run 배선
  ⑪ P-2 공용 판독기 + bootstrap/mission hooks_effective additive 필드·exit 불변

밀폐: tempfile 아래 base/dept에 hooks 전체와 bin/*.py만 복사한다(tests 제외).
os.environ을 상속하지 않는 env에 HOME/CYS_PACK_DIR/TMPDIR/state/socket을 가둔다.
PATH에는 지정 Python 심링크와 /usr/bin:/bin만 둔다(cys/cysd 없음).
임시 쓰기는 tempfile 루트 안에서만 한다. 제품 원본은 읽기만 하며 데몬·네트워크는 쓰지 않는다.
출력: check(name, cond, detail)의 PASS/FAIL 행 · 실패 수가 있으면 exit 1.
전부 통과한 경우에만 종료 토큰 LANE-REDIRECT-OK.
실행 규약: CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_lane_redirect.py
"""

import ast
from contextlib import contextmanager
import hashlib
import importlib.util
import inspect
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

PACK = Path(__file__).resolve().parents[2]
REDIRECT = 'command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"'
MARKER = Path("state/lane-guard-tripped")
fails = []
sys.dont_write_bytecode = True


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name,
                         (" — " + str(detail)) if detail else ""))
    if not cond:
        fails.append(name)


def attempt(name, action):
    """없는 신규 API·timeout도 해당 항목 FAIL로 접고 다음 검체를 계속한다."""
    try:
        action()
    except Exception as exc:
        check(name, False, "%s: %s" % (type(exc).__name__, exc))


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def read(path):
    return path.read_text(encoding="utf-8") if path.is_file() else ""


class Lab:
    def __init__(self, root):
        self.root = root
        for name in ("home", "bin", "tmp", "state", "work"):
            (root / name).mkdir(parents=True)
        (root / "bin/python3").symlink_to(sys.executable)
        self.base = root / "base"
        self.dept = root / "dept"
        for pack in (self.base, self.dept):
            shutil.copytree(PACK / "hooks", pack / "hooks")
            (pack / "bin").mkdir()
            for source in (PACK / "bin").glob("*.py"):
                shutil.copy2(source, pack / "bin" / source.name)
            (pack / "state").mkdir()
        self.env = {
            "HOME": str(root / "home"),
            "PATH": str(root / "bin") + ":/usr/bin:/bin",
            "TMPDIR": str(root / "tmp"),
            "LANG": "en_US.UTF-8",
            "CYS_SURFACE_ID": "surface:99",
            "CYS_SOCKET": str(root / "no-such.sock"),
            "CYS_NO_AUTOSTART": "1",
            "CYS_STATE_DIR": str(root / "state"),
            "CYS_PACK_DIR": str(self.dept),
            "PYTHONDONTWRITEBYTECODE": "1",
        }
        for binary in ("cys", "cysd"):
            if shutil.which(binary, path=self.env["PATH"]):
                raise RuntimeError("밀폐 PATH에 %s가 있다" % binary)
        # ★Write 대상은 tmp 밖의 경로여야 한다 — 능력 게이트는 `/tmp/`·`/var/folders/` 등
        #   (ALLOW_PATH_PREFIXES) 아래 쓰기를 reviewer 에게도 허용하므로, 실험실(tempfile)
        #   안의 경로를 주면 같은 팩에서도 deny 가 나지 않아 계측 대조(R-1a)가 무너진다.
        #   존재하지 않는 경로라도 판정은 경로 규칙만 본다(실측).
        self.payload = json.dumps({
            "session_id": "t", "transcript_path": str(root / "t.jsonl"),
            "cwd": str(root / "work"), "hook_event_name": "PreToolUse",
            "tool_name": "Write", "tool_input": {
                "file_path": "/opt/lane-redirect-probe/src/x.txt", "content": "x"},
        })
        write(root / "t.jsonl", "")

    def run(self, hook, pack=None, source=None, extra=None, args=(), shell_args=()):
        env = {**self.env, "CYS_PACK_DIR": str(pack or self.dept), **(extra or {})}
        return subprocess.run(
            ["sh", *shell_args, str((source or self.base) / "hooks" / hook), *args],
            input=self.payload, env=env, cwd=self.root / "work",
            capture_output=True, text=True, timeout=60)


def result(r):
    return "rc=%d stdout=%r stderr=%r" % (r.returncode, r.stdout, r.stderr)


def denied(r):
    try:
        return (r.returncode == 0 and
                json.loads(r.stdout)["hookSpecificOutput"]["permissionDecision"] == "deny")
    except (ValueError, KeyError, TypeError):
        return False


def r1_r5(lab):
    hook = "role-capability-gate.sh"
    reviewer = {"CYS_ROLE": "reviewer-codex"}
    same = lab.run(hook, pack=lab.base, extra=reviewer)
    check("R-1a 같은 팩 reviewer Write deny(계측 대조)", denied(same), result(same))
    delegated = lab.run(hook, extra=reviewer)
    check("R-1b 타 레인 reviewer Write deny 보존", denied(delegated), result(delegated))
    for tag, pack in (("same", lab.base), ("redirect", lab.dept)):
        master = lab.run(hook, pack=pack, extra={"CYS_ROLE": "master"})
        check("R-1c %s master deny 없음" % tag,
              master.returncode == 0 and not denied(master), result(master))
    check("R-5a 같은 팩 표식 무생성", not (lab.base / MARKER).exists())
    # 위임 성공 여부는 R-1b에서 별도로 단언한다. 무생성만으로 성공을 주장하지 않는다.
    check("R-5b 위임 경로 표식 무생성", not (lab.dept / MARKER).exists())


def r2(lab):
    stub = ('#!/bin/sh\n'
            'printf \'%s\\n\' \'{"hookSpecificOutput":{"hookEventName":"PreToolUse",'
            '"permissionDecision":"deny","permissionDecisionReason":"DEPT-STUB-RAN"}}\'\n'
            'exit 0\n')
    target = lab.dept / "hooks/role-capability-gate.sh"
    write(target, stub)
    ran = lab.run(target.name)
    check("R-2a 실제 dept 스텁 실행", denied(ran) and "DEPT-STUB-RAN" in ran.stdout,
          result(ran))
    # cat >/dev/null은 입력 내용을 증명하지 못하므로 먼저 바이트 사본도 남긴다.
    write(target, stub.replace("exit 0\n", 'cat > "$PROBE_STDIN"\n'
                              'cat < "$PROBE_STDIN" >/dev/null\n'
                              'printf \'%s\' "$1" >&2\nexit 0\n'))
    received = lab.root / "received.json"
    probe = lab.run(target.name, args=("--probe",), extra={"PROBE_STDIN": str(received)})
    check("R-2b 인자·stdin·stderr 그대로 전달",
          probe.returncode == 0 and probe.stderr == "--probe"
          and read(received) == lab.payload, result(probe) + " stdin_equal=%s" %
          (read(received) == lab.payload))
    # hooks/<sub>/<basename> 경로도 평탄화하지 않고 그대로 위임한다.
    subhook = "fullauto/owner-active.sh"
    write(lab.dept / "hooks" / subhook, '#!/bin/sh\nprintf \'%s\' SUB-DEPT-RAN\n')
    sub = lab.run(subhook)
    check("R-2c 하위 디렉터리 대응 훅 위임", sub.returncode == 0 and
          sub.stdout == "SUB-DEPT-RAN", result(sub))


def r3(lab):
    for hook in ("grill-gate.sh", "grill-stop.sh"):
        write(lab.dept / "hooks" / hook, '#!/bin/sh\nprintf \'%s\\n\' DEPT-BLOCK >&2\nexit 2\n')
        r = lab.run(hook)
        check("R-3 %s exit 2·stderr 보존" % hook,
              r.returncode == 2 and "DEPT-BLOCK" in r.stderr, result(r))


def r4(lab):
    hook = "grill-arm.sh"
    (lab.dept / "hooks" / hook).unlink()
    marker = lab.dept / MARKER
    first = lab.run(hook)
    first_text = read(marker)
    expected = ["script=" + hook, "hook_root=" + str(lab.base.resolve()),
                "lane_root=" + str(lab.dept.resolve()), "surface=surface:99"]
    fields = first_text.splitlines()
    check("R-4a 대응 훅 부재 exit 0·stdout 0·표식 필드",
          first.returncode == 0 and first.stdout == "" and marker.is_file()
          and all(field in fields for field in expected)
          and any(re.fullmatch(r"ts=\d+", field) for field in fields),
          result(first) + " marker=%r" % first_text)
    # 다른 surface로 두 번째 실행: 같은 줄 수만 재면 '아무것도 안 쓴' 구현도 통과한다.
    second = lab.run(hook, extra={"CYS_SURFACE_ID": "surface:100"}, shell_args=("-u",))
    second_text = read(marker)
    check("R-4b 표식 1개·덮어쓰기·set -u 안전",
          second.returncode == 0 and second.stdout == ""
          and len(list((lab.dept / "state").glob("lane-guard-tripped*"))) == 1
          and len(second_text.splitlines()) == len(fields) > 0
          and "surface=surface:100" in second_text.splitlines()
          and "surface=surface:99" not in second_text.splitlines(),
          result(second) + " lines=%d→%d" % (len(fields), len(second_text.splitlines())))


def r6(lab):
    (lab.dept / "hooks/grill-arm.sh").unlink()
    state = lab.dept / "state"
    state.rmdir()
    write(state, "not a directory\n")
    r = lab.run("grill-arm.sh", shell_args=("-u",))
    check("R-6a state가 파일이어도 exit 0·stdout 0",
          r.returncode == 0 and r.stdout == "", result(r))
    state.unlink()
    state.mkdir(mode=0o500)
    try:
        if hasattr(os, "geteuid") and os.geteuid() == 0:
            print("SKIP R-6b chmod 쓰기 불가 — root는 권한 비트를 우회한다")
        else:
            r = lab.run("grill-arm.sh", shell_args=("-u",))
            check("R-6b state 0500에서도 exit 0·stdout 0",
                  r.returncode == 0 and r.stdout == "", result(r))
    finally:
        state.chmod(0o700)


def r7(lab):
    target = lab.dept / "hooks/grill-arm.sh"
    write(target, '#!/bin/sh\nprintf \'%s\' MUST-NOT-REDIRECT\n')
    r = lab.run(target.name, extra={"CYS_LANE_REDIRECTED": "1"})
    check("R-7a 이미 위임됨 → 재위임 0·exit 0·표식",
          r.returncode == 0 and r.stdout == "" and (lab.dept / MARKER).is_file(),
          result(r) + " marker=%s" % (lab.dept / MARKER).is_file())
    # 프리루드 파일이 base를 가리켜도 $0의 dept 경로로 끝나야 한다.
    shutil.copy2(lab.base / "hooks/role-capability-gate.sh",
                 lab.dept / "hooks/role-capability-gate.sh")
    lib = lab.dept / "hooks/_lib.sh"
    lib.unlink()
    lib.symlink_to(lab.base / "hooks/_lib.sh")
    attempt("R-7b 프리루드 심링크에서도 60s 안에 종료", lambda: _symlink_run(lab))
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        print("SKIP R-7c chmod 판독 불가 — root는 권한 비트를 우회한다")
    else:
        marker = lab.dept / MARKER
        marker.unlink(missing_ok=True)
        try:
            target.chmod(0o000)
            r = lab.run(target.name)
            check("R-7c 판독 불가 대상 → 표식+exit 0·stdout 0",
                  r.returncode == 0 and r.stdout == "" and marker.is_file(),
                  result(r) + " marker=%s" % marker.is_file())
        finally:
            target.chmod(0o755)


def _symlink_run(lab):
    r = lab.run("role-capability-gate.sh", extra={"CYS_ROLE": "reviewer-codex"})
    check("R-7b 프리루드 심링크에서도 60s 안에 종료", r.returncode == 0, result(r))


def r8():
    missing = []
    found = []
    for path in sorted((PACK / "hooks").rglob("*.sh")):
        if path.name == "_lib.sh":
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        if path.name == "role-bootstrap.sh":
            start = next((i for i, line in enumerate(lines)
                          if line.strip() == 'if [ -r "$_CYS_PRELUDE" ]; then'), None)
            end = next((i for i in range(start + 1, len(lines))
                        if lines[i].strip() == "fi"), None) if start is not None else None
        else:
            start = next((i for i, line in enumerate(lines)
                          if re.match(r"\s*(?:\.|source)\s", line) and "_lib.sh" in line), None)
            if start is None:
                continue
            end = next((i for i in range(start, len(lines))
                        if re.search(r"exit 0;\s*}\s*$", lines[i])), None)
        rel = path.relative_to(PACK / "hooks").as_posix()
        found.append(rel)
        following = next((line for line in lines[end + 1:] if line.strip()), "") if end is not None else ""
        if following != REDIRECT:
            missing.append(rel)
    required = {"test_pre_dispatch.sh", "role-bootstrap.sh", "role-bootstrap-legacy.sh"}
    check("R-8 프리루드 직후 redirect census", bool(found) and required.issubset(found)
          and not missing, "scanned=%d missing=%s" % (len(found), ", ".join(missing)))


def r9(lab):
    hook = "role-capability-gate.sh"
    write(lab.dept / "hooks" / hook, '#!/bin/sh\ncat >/dev/null\nprintf x >> "$COUNT"\n')
    count = lab.root / "count"
    extra = {"COUNT": str(count)}
    first = lab.run(hook, extra=extra)
    one = read(count)
    second = lab.run(hook, source=lab.dept, extra=extra)
    two = read(count)
    check("R-9 혼재 등록은 위임 1+직접 1=2회",
          first.returncode == second.returncode == 0 and one == "x" and two == "xx",
          "after_base=%r after_dept=%r rc=(%d,%d)" %
          (one, two, first.returncode, second.returncode))


@contextmanager
def sealed_process(env, cwd):
    saved_env, saved_cwd, saved_path = dict(os.environ), Path.cwd(), list(sys.path)
    os.environ.clear()
    os.environ.update(env)
    os.chdir(cwd)
    sys.path.insert(0, str(Path(env["CYS_PACK_DIR"]) / "bin"))
    try:
        yield
    finally:
        sys.path[:] = saved_path
        os.chdir(saved_cwd)
        os.environ.clear()
        os.environ.update(saved_env)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def snapshot(root):
    """읽기 축의 쓰기 0: 파일 내용·mtime·권한과 디렉터리 생성/삭제를 함께 비교."""
    return {str(p.relative_to(root)): (p.stat().st_mode, p.stat().st_mtime_ns,
            hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else None)
            for p in root.rglob("*")}


def seed_marker(lab, age=0):
    marker = lab.dept / MARKER
    write(marker, "hook_root=%s\nlane_root=%s\nscript=grill-arm.sh\n"
          "surface=surface:99\nts=%d\n" % (lab.base.resolve(), lab.dept.resolve(), time.time()))
    stamp = time.time() - age
    os.utime(marker, (stamp, stamp))
    return marker


def c83(pf, lab, name, status, contains=(), fix=False, detail_test=None):
    def action():
        before = snapshot(lab.root)
        probe = pf.Preflight(fix=fix, skips=[], mode="fix" if fix else "report")
        try:
            probe.c83_lane_guard_tripped()
        finally:
            if snapshot(lab.root) != before:
                raise AssertionError("C83가 파일을 변경했다(report/fix 쓰기 0 위반)")
        rows = probe.results
        ok = (len(rows) == 1 and rows[0]["id"] == "C83.lane-guard-tripped"
              and rows[0]["status"] == status
              and all(value in rows[0]["detail"] for value in contains)
              and (detail_test is None or detail_test(rows[0]["detail"])))
        check(name, ok, repr(rows))
    attempt(name, action)


def profile(lab, paths, cfg=None):
    cfg = cfg or lab.root / "cfg"
    settings = cfg / "settings.json"
    write(settings, json.dumps({"hooks": {"SessionStart": [
        {"hooks": [{"type": "command", "command": 'sh "%s"' % path}]}
        for path in paths]}}, ensure_ascii=False, indent=2) + "\n")
    return settings


def p1(pf, lab):
    c83(pf, lab, "P-1a 표식 없음 → PASS", pf.PASS)
    marker = seed_marker(lab)
    c83(pf, lab, "P-1b 최근 표식 → FAIL·표식 인용·처방", pf.FAIL,
        ("script=", "CLAUDE_CONFIG_DIR"))
    old = time.time() - 25 * 3600
    os.utime(marker, (old, old))
    c83(pf, lab, "P-1c 25h 표식 → PASS·나이 상세", pf.PASS,
        detail_test=lambda detail: bool(re.search(
            r"(?:\d+(?:\.\d+)?\s*(?:초|시간|일|[smhd]\b)|age(?:_s)?\s*[=:]\s*\d)",
            detail, re.IGNORECASE)))
    marker.unlink()
    os.environ.update(CLAUDECODE="1", CLAUDE_CONFIG_DIR=str(lab.root / "cfg"))
    foreign = lab.base / "hooks/inject-context.sh"
    own = lab.dept / "hooks/inject-context.sh"
    settings = profile(lab, [foreign])
    c83(pf, lab, "P-1d 타 레인 SessionStart만 → WARN·처방", pf.WARN,
        ("CLAUDE_CONFIG_DIR",))
    profile(lab, [foreign, own])
    c83(pf, lab, "P-1d 자기 레인 포함 → PASS", pf.PASS)
    alias = lab.root / "dept-alias"
    alias.symlink_to(lab.dept, target_is_directory=True)
    profile(lab, [alias / "hooks/inject-context.sh"])
    c83(pf, lab, "P-1d realpath 자기 레인 → PASS", pf.PASS)
    profile(lab, [])
    c83(pf, lab, "P-1d SessionStart 0건 → WARN·C08 소관", pf.WARN, ("C08",))
    # CLAUDE_CONFIG_DIR가 없으면 밀폐 HOME/.claude를 읽어야 한다.
    os.environ.pop("CLAUDE_CONFIG_DIR")
    profile(lab, [own], lab.root / "home/.claude")
    c83(pf, lab, "P-1d 기본 설정 폴더 → PASS", pf.PASS)
    os.environ["CLAUDE_CONFIG_DIR"] = str(settings.parent)
    profile(lab, [foreign])
    seed_marker(lab)
    original = settings.read_bytes()
    c83(pf, lab, "P-1e fix도 최근 표식 FAIL·쓰기 0", pf.FAIL,
        ("script=", "CLAUDE_CONFIG_DIR"), fix=True)
    check("P-1e fix settings.json 바이트 불변", settings.read_bytes() == original)
    # 표식 FAIL 조기 반환이 설정축의 쓰기를 가리지 않도록 설정축도 따로 잰다.
    marker.unlink()
    c83(pf, lab, "P-1e fix도 타 레인 WARN·쓰기 0", pf.WARN,
        ("CLAUDE_CONFIG_DIR",), fix=True)
    profile(lab, [])
    c83(pf, lab, "P-1e fix도 미배선 WARN·쓰기 0", pf.WARN, ("C08",), fix=True)
    src = inspect.getsource(pf.Preflight.run)
    check("P-1f run C83가 C62 앞에 배선",
          "c83_lane_guard_tripped" in src and "c62_pack_heal_ledger" in src
          and src.index("c83_lane_guard_tripped") < src.index("c62_pack_heal_ledger"))


def p2(pf, lab):
    marker = lab.dept / MARKER
    marker.unlink(missing_ok=True)

    def reader(name, recent, info_expected):
        def action():
            before = snapshot(lab.root)
            try:
                explicit = pf.lane_guard_tripped(str(lab.dept))
                default = pf.lane_guard_tripped()
            finally:
                if snapshot(lab.root) != before:
                    raise AssertionError("공용 판독기가 파일을 변경했다(읽기 전용 위반)")
            value, info = explicit
            valid = explicit == (False, None) if not info_expected else (
                value is recent and isinstance(info, dict)
                and info.get("script") == "grill-arm.sh"
                and info.get("path") == str(marker)
                and isinstance(info.get("age_s"), (float, int))
                and (info["age_s"] >= 24 * 3600 if not recent else 0 <= info["age_s"] < 24 * 3600))
            # 두 호출 사이 시간이 흐르므로 age_s의 숫자 자체를 동일시하지 않는다.
            valid = valid and default[0] is recent and (default[1] is None if not info_expected
                                                       else default[1].get("script") == "grill-arm.sh")
            check(name, valid, repr(explicit))
        attempt(name, action)

    reader("P-2a 판독기 표식 없음 → (False, None)", False, False)
    seed_marker(lab)
    reader("P-2b 판독기 최근 → (True, info)", True, True)
    seed_marker(lab, age=25 * 3600)
    reader("P-2c 판독기 오래됨 → (False, info)", False, True)

    source = read(lab.dept / "bin/javis_bootstrap.py")
    tree = ast.parse(source)
    summaries = [node for node in ast.walk(tree) if isinstance(node, ast.Assign)
                 and any(isinstance(t, ast.Name) and t.id == "summary" for t in node.targets)
                 and isinstance(node.value, ast.Dict)
                 and {"ok", "steps", "boot_last"}.issubset({k.value for k in node.value.keys
                     if isinstance(k, ast.Constant) and isinstance(k.value, str)})]
    # literal dict뿐 아니라 출력 직전 summary["hooks_effective"] 할당도 additive다.
    assemblies = [re.split(r"print\(\s*json\.dumps\(summary", "\n".join(
        source.splitlines()[node.lineno - 1:]), maxsplit=1)[0] for node in summaries]
    check("P-2d bootstrap 최종 summary hooks_effective 텍스트 핀",
          bool(assemblies) and any('"hooks_effective"' in part or "'hooks_effective'" in part
                                   for part in assemblies))
    source = read(lab.dept / "bin/javis_mission.py")
    status = next(node for node in ast.parse(source).body
                  if isinstance(node, ast.FunctionDef) and node.name == "cmd_status")
    check("P-2e mission cmd_status hooks_effective 텍스트 핀",
          "hooks_effective" in ast.get_source_segment(source, status))
    for label, age, expected in (("없음", None, True), ("최근", 0, False),
                                 ("오래됨", 25 * 3600, True)):
        if age is None:
            marker.unlink(missing_ok=True)
        else:
            seed_marker(lab, age=age)
        env = {**lab.env, "CYS_STATE_DIR": str(lab.root)}  # CYS_MISSION 없음
        r = subprocess.run([sys.executable, str(lab.dept / "bin/javis_mission.py"),
                            "status", "--json"], env=env, cwd=lab.root / "work",
                           capture_output=True, text=True, timeout=60)
        try:
            doc = json.loads(r.stdout)
        except ValueError:
            doc = {}
        check("P-2f mission 표식 %s rc 불변·hooks_effective=%s" % (label, expected),
              r.returncode in (1, 2) and doc.get("hooks_effective") is expected, result(r))


def preflight_cases(lab):
    with sealed_process(lab.env, lab.root / "work"):
        pf = load(lab.dept / "bin/javis_preflight.py", "_pf_lane_redirect")
        attempt("P-1 C83 검체", lambda: p1(pf, lab))
        attempt("P-2 공용 판독기·summary 검체", lambda: p2(pf, lab))


def main():
    # 임시 루트는 시스템 tmp(다른 test_*.py 와 같은 관례) — 저장소 안에 잔재를 남기지 않는다.
    with tempfile.TemporaryDirectory(prefix="lane-redirect-") as tmp:
        root = Path(tmp)
        for name, action in (("R-1/R-5", r1_r5), ("R-2", r2), ("R-3", r3),
                             ("R-4", r4), ("R-6", r6), ("R-7", r7),
                             ("R-9", r9), ("P-1/P-2", preflight_cases)):
            attempt(name, lambda name=name, action=action:
                    action(Lab(root / name.replace("/", "-"))))
        attempt("R-8 census", r8)
    if fails:
        print("\n%d FAIL" % len(fails))
        return 1
    print("\nALL PASS\nLANE-REDIRECT-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
