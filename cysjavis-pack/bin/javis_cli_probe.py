#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_cli_probe.py — DD-2 CLI 감지 단일 함수(로그인셸 프로브).

모든 CLI 감지 경로(cys-dept 편성·javis_bootstrap·javis_formation·배너 원인 판별)가
이 모듈의 `probe_cli`/`resolve_cli` 만 쓴다 — GUI(launchd 빈곤 PATH) 프로세스에서 호출돼도
로그인셸 rc(PATH 재구성 = 기능2 우산) 를 거쳐 해석하므로 오판 0.

계약(test_cli_probe.py):
  probe_cli(name) -> bool        존재=True / 미존재=False (엄격 bool).
  resolve_cli(name) -> str|None  해석된 절대경로(또는 None) — 편성 seat(--cmd) 용.
  PROBE_READONLY = True          설치·수정 부작용 0(온보딩 무간섭 표식).

CLI:
  python3 javis_cli_probe.py <name>       존재 시 경로 stdout·exit 0 / 미발견 exit 1.
  python3 javis_cli_probe.py --self-test  내부 판정 로직 밀폐 배터리(팩 관례 —
                                          javis_detect·javis_lock 과 동형: exit 0=OK / 1=FAIL).

주의: probe_cli 는 엄격 bool 을 반환한다(RED 계약이 `is True`/`is False` 로 핀). 해석된
경로가 필요한 호출부는 resolve_cli(또는 CLI stdout)를 쓴다 — 층위 분리(DD-2 · ADR-D2).
"""
import os
import shlex
import subprocess
import sys

# read-only 계약 표식(온보딩 무간섭 — 설치·수정 부작용 0). test_cli_probe #4 가 이 표식을 검증한다.
PROBE_READONLY = True

_PROBE_TIMEOUT = float(os.environ.get("CYS_PROBE_TIMEOUT_SECS", "15"))


def _login_shell():
    """로그인셸 절대경로 해석 — 빈곤 PATH(빈 PATH env)에서도 exec 가능하도록 절대경로를 쓴다.
    macOS 기본 zsh 우선, 부재 시 bash. (절대경로 argv[0] 은 PATH 무관 execve 가능.)"""
    for sh in ("/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh",
               "/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"):
        if os.path.exists(sh):
            return sh
    # 최후 폴백: PATH 로 탐색(빈곤 PATH 가 아닐 때만 유효 — 위 절대경로가 우선이라 실효 낮음)
    for name in ("zsh", "bash", "sh"):
        for d in (os.environ.get("PATH") or os.defpath).split(os.pathsep):
            if d and os.path.exists(os.path.join(d, name)):
                return os.path.join(d, name)
    return "/bin/sh"


def _resolve_posix(name):
    """로그인셸 rc(PATH 재구성) 경유로 name 을 해석 → 절대경로 문자열(또는 None).
    핵심(기능2 우산): 부모 PATH 가 비어도(GUI launchd 빈곤 PATH) 로그인셸이 /etc/zprofile
    (path_helper) 등 rc 를 소싱해 PATH 를 복원하므로, 그 안에서 `command -v` 가 해석한다."""
    sh = _login_shell()
    cmd = "command -v -- %s" % shlex.quote(name)
    try:
        r = subprocess.run([sh, "-lc", cmd], capture_output=True, text=True,
                           timeout=_PROBE_TIMEOUT)
    except Exception:
        return None
    if r.returncode != 0:
        return None
    lines = [ln.strip() for ln in (r.stdout or "").splitlines() if ln.strip()]
    if not lines:
        return None
    path = lines[0]
    # `command -v` 는 함수/별칭 이름을 그대로 뱉을 수 있다 — 절대경로만 신뢰(감지 = 실행가능 바이너리).
    if not path.startswith("/"):
        return None
    return path


def _resolve_windows(name):
    """Windows: 동봉 bash(-lc) 경유 우선, 실패 시 `where` 폴백."""
    import shutil
    bash = shutil.which("bash")
    if bash:
        try:
            r = subprocess.run([bash, "-lc", "command -v -- %s" % shlex.quote(name)],
                               capture_output=True, text=True, timeout=_PROBE_TIMEOUT)
            if r.returncode == 0:
                lines = [ln.strip() for ln in (r.stdout or "").splitlines() if ln.strip()]
                if lines:
                    return lines[0]
        except Exception:
            pass
    # where 폴백
    try:
        r = subprocess.run(["where", name], capture_output=True, text=True,
                           timeout=_PROBE_TIMEOUT)
        if r.returncode == 0:
            lines = [ln.strip() for ln in (r.stdout or "").splitlines() if ln.strip()]
            if lines:
                return lines[0]
    except Exception:
        pass
    # 최후: PATH 상 직접 탐색
    w = shutil.which(name)
    return w if w else None


def resolve_cli(name):
    """name → 해석된 절대경로(str) 또는 None. 로그인셸 PATH 우산(기능2). read-only."""
    if not name:
        return None
    if os.name == "nt":
        return _resolve_windows(name)
    return _resolve_posix(name)


def probe_cli(name):
    """name 이 로그인셸 PATH 상 실행가능하면 True, 아니면 False(엄격 bool)."""
    return resolve_cli(name) is not None


# ── self-test (밀폐 배터리 · 결정론 · 외부 설치 의존 0) ───────────────────────
def _self_test():
    """내부 판정 로직 배터리 — 팩 관례(javis_detect·javis_lock 과 동형)의 진입점.

    실패는 stderr 에 FAIL 행 + exit 1 / 성공은 stdout 한 줄 + exit 0 (preflight·CI 는 exit
    code 만 읽는다). 배터리 자체가 PROBE_READONLY 계약을 지킨다 — 설치·수정 부작용 0이고,
    빈곤 PATH 케이스가 건드리는 os.environ["PATH"] 는 finally 로 원상 복구한다.

    posix 4건(로그인셸 절대경로·절대경로 해소·빌트인 배제·빈곤 PATH 우산)은 macOS/리눅스
    고유 술어라 Windows 에서는 스킵한다 — Windows 경로는 `_resolve_windows`(bash -lc → where
    → shutil.which)라 `_login_shell()` 자체를 타지 않고, 빈곤 PATH 복원 술어도 성립하지
    않는다. test_cli_probe.py 케이스 3 의 스킵 근거와 같다(플랫폼 의미론 차이 · 코드 결함 아님)."""
    fails = []
    saved_path = os.environ.get("PATH")
    saved_cwd = os.getcwd()
    core = "sh" if os.name != "nt" else "cmd"
    bogus = "definitely-not-a-real-cli-xyz-123"
    posix = os.name != "nt"
    try:
        # ① read-only 표식(온보딩 무간섭) — test_cli_probe #4 와 같은 계약을 자체 확인.
        if PROBE_READONLY is not True:
            fails.append("PROBE_READONLY 표식이 True 가 아니다(read-only 계약 이탈)")
        # ② 빈 입력 가드 — None·빈 문자열은 셸을 띄우지 않고 즉시 None/False.
        if resolve_cli(None) is not None or resolve_cli("") is not None:
            fails.append("빈 이름(None·'')이 None 으로 차단되지 않는다")
        if probe_cli("") is not False:
            fails.append("probe_cli('') 이 엄격 False 가 아니다")
        # ③ 엄격 bool 계약(RED 가 `is True`/`is False` 로 핀) — 존재 유틸 / 미존재 이름.
        if probe_cli(core) is not True:
            fails.append("probe_cli(%r) 이 엄격 True 가 아니다" % core)
        if probe_cli(bogus) is not False:
            fails.append("probe_cli(%r) 이 엄격 False 가 아니다" % bogus)
        if posix:
            # ④ 로그인셸은 **실재하는 절대경로**여야 한다(빈곤 PATH 에서도 execve 가능해야
            #    하므로 — posix 해소 경로 전용 술어. Windows 는 _login_shell 을 타지 않는다).
            sh = _login_shell()
            if not (isinstance(sh, str) and sh.startswith("/") and os.path.exists(sh)):
                fails.append("_login_shell() 이 실재 절대경로가 아니다: %r" % sh)
            # ⑤ resolve_cli 는 **절대경로**를 돌려준다(편성 seat 의 --cmd 가 그대로 소비한다).
            p = resolve_cli(core)
            if not (isinstance(p, str) and p.startswith("/")):
                fails.append("resolve_cli(%r) 이 절대경로가 아니다: %r" % (core, p))
            # ⑥ `command -v` 가 그대로 뱉는 **비절대 이름**(셸 빌트인)은 감지가 아니다 —
            #    감지 = 실행가능 바이너리. 이 분기가 죽으면 빌트인이 CLI 로 오계상된다.
            if resolve_cli("cd") is not None or probe_cli("cd") is not False:
                fails.append("셸 빌트인(cd)이 감지로 계상됐다(절대경로만 신뢰 분기 이탈)")
            # ⑦ 기능2 우산 — 빈곤 PATH(GUI launchd)여도 로그인셸 rc 가 PATH 를 복원해 해석한다.
            os.environ["PATH"] = ""
            hit = probe_cli(core)
            if saved_path is not None:
                os.environ["PATH"] = saved_path
            else:
                os.environ.pop("PATH", None)
            if hit is not True:
                fails.append("빈곤 PATH 에서 %r 미해석 — GUI launchd 오판 재현(회귀)" % core)
        # ⑧ 부작용 0 — 배터리 전후 cwd·PATH 동일(프로브가 프로세스 상태를 바꾸지 않는다).
        if os.getcwd() != saved_cwd:
            fails.append("self-test 가 cwd 를 바꿨다(부작용 0 이탈)")
        if os.environ.get("PATH") != saved_path:
            fails.append("self-test 가 PATH 를 복원하지 못했다(부작용 0 이탈)")
    finally:
        if saved_path is not None:
            os.environ["PATH"] = saved_path
        else:
            os.environ.pop("PATH", None)
    if fails:
        sys.stderr.write("javis_cli_probe self-test FAIL (%d):\n" % len(fails))
        for f in fails:
            sys.stderr.write("  - %s\n" % f)
        return 1
    print("javis_cli_probe self-test OK — %d 케이스(read-only 표식·빈 입력 가드·엄격 True·"
          "엄격 False·부작용 0%s)"
          % (9 if posix else 5,
             " + posix 4(로그인셸 절대경로·절대경로 해소·빌트인 배제·빈곤 PATH 우산)" if posix
             else " · posix 전용 4건은 Windows 스킵"))
    return 0


def main(argv):
    # ★팩 관례(javis_detect.main·javis_lock __main__): `--self-test` 를 위치인자 해석보다
    #   먼저 가로챈다. 인자 없음·-h/--help·<cli name> 경로의 exit code 는 그대로다.
    if "--self-test" in argv[1:]:
        return _self_test()
    if len(argv) < 2 or argv[1] in ("-h", "--help"):
        sys.stderr.write("usage: javis_cli_probe.py <cli-name> | --self-test  "
                         "(경로 stdout·exit 0=발견 / exit 1=미발견 / --self-test 0=OK·1=FAIL)\n")
        return 2
    path = resolve_cli(argv[1])
    if path:
        print(path)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
