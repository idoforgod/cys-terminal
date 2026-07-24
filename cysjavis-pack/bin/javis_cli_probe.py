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
  python3 javis_cli_probe.py <name>   존재 시 경로 stdout·exit 0 / 미발견 exit 1.

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


def main(argv):
    if len(argv) < 2 or argv[1] in ("-h", "--help"):
        sys.stderr.write("usage: javis_cli_probe.py <cli-name>  "
                         "(경로 stdout·exit 0=발견 / exit 1=미발견)\n")
        return 2
    path = resolve_cli(argv[1])
    if path:
        print(path)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
