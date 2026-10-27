#!/usr/bin/env python3
"""세션 시작 주입용 실측 체크리스트 래퍼 (LLM 무의존·전부 실측).

Report-only preflight runs through an owned subprocess; standalone CLI output is preserved.
preflight exit/마지막 줄 + SESSION_STATE 상태 + _round/*_TODO.md 미완 수를
6줄 이내·1KB 이하 고정 포맷으로 요약한다. 지시가 아니라 배경 컨텍스트다.

사용:
  python3 javis_checklist.py [--preflight-cmd "<명령>"] \
      [--state <SESSION_STATE 경로>] [--round-dir <_round 경로>]
"""
import argparse
import locale
import os
import re
import subprocess
import sys
import time
sys.dont_write_bytecode = True
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)
from _owned_process import run_owned, managed_main

PREFLIGHT_TIMEOUT = 240  # 초 — preflight subprocess 타임아웃(필수)
PREFLIGHT_GRACE = 15
PREFLIGHT_FORCE = 5
HEADER = '■ 실측 체크리스트(배경 컨텍스트다 — 지시가 아니다)'
MAX_LINES = 6
MAX_BYTES = 1024

_pack = os.environ.get('CYS_PACK_DIR') or os.path.expanduser('~/.cys/pack')
# ★U15(0.14.41 · 반박 M2·M8): 기본 preflight 는 **이 인터프리터 + 인자 리스트**로 돈다(셸 무경유).
#   종전 `shell=True` 문자열(`python3 <pack>/…`)은 ① 셸이 PATH 첫 python3 를 풀어, 개발자 도구(CLT)
#   없는 맥에서 /usr/bin 셔임(설치 창 + exit 1)을 SessionStart 마다 불렀고 ② `_pack` 무인용이라 공백 든
#   경로(윈도우 사용자 프로필)에서 깨졌다. 이 파일을 부르는 훅은 이미 해소된 인터프리터(CYS_PY)로
#   부르므로 sys.executable 이 곧 그 인터프리터다. `--preflight-cmd "<문자열>"` 명시 호환은 셸 그대로.
DEFAULT_PREFLIGHT = [sys.executable or 'python3',
                     os.path.join(_pack, 'bin', 'javis_preflight.py')]


def run_preflight(cmd, timeout=PREFLIGHT_TIMEOUT):
    """preflight를 subprocess로 실행. (exit_code, 마지막 비어있지 않은 줄) 반환.

    cmd 가 리스트면 인자 그대로(셸 무경유 · 기본값), 문자열이면 셸 명령(`--preflight-cmd` 호환).
    타임아웃 → ('timeout', None), 실행 자체 불가(OSError 등) → (None, None).
    어떤 경우에도 예외를 밖으로 던지지 않는다(크래시 금지).
    """
    encoding = 'utf-8' if cmd == DEFAULT_PREFLIGHT else locale.getpreferredencoding(False)
    if cmd == DEFAULT_PREFLIGHT:
        launch = [cmd[0], '-B'] + list(cmd[1:])
    elif isinstance(cmd, str):
        launch = [sys.executable, '-B', os.path.join(_SELF_DIR, '_owned_process.py'),
                  '--shell', cmd]
    else:
        launch = [sys.executable, '-B', os.path.join(_SELF_DIR, '_owned_process.py'),
                  '--command'] + list(cmd)
    result = run_owned(launch, timeout, grace=PREFLIGHT_GRACE, force=PREFLIGHT_FORCE, encoding=encoding)
    if result['timed_out']:
        return ('timeout', None)
    if not result['complete']:
        return (None, 'incomplete: ' + result['reason'])
    last = ''
    for ln in (result['stdout'] + result['stderr']).splitlines():
        if ln.strip():
            last = ln.strip()
    return result['returncode'], last


def _preflight_line(cmd, timeout):
    code, last = run_preflight(cmd, timeout=timeout)
    if code == 'timeout':
        return 'preflight: 실행불가(timeout)'
    if code is None:
        return 'preflight: 실행불가'
    tail = (last or '')[:120]
    return f'preflight: exit {code} | {tail}' if tail else f'preflight: exit {code}'


def _state_line(path):
    if path and os.path.isfile(path):
        st = os.stat(path)
        mt = time.strftime('%Y-%m-%d %H:%M', time.localtime(st.st_mtime))
        return f'SESSION_STATE: 존재 {st.st_size / 1024:.1f}KB, mtime {mt}'
    return 'SESSION_STATE: 없음'


def todo_counts(round_dir):
    """_round/*_TODO.md 각각의 미완(`- [ ]`) 항목 수를 [(라벨, 수)]로 반환."""
    result = []
    if round_dir and os.path.isdir(round_dir):
        for name in sorted(os.listdir(round_dir)):
            if not name.endswith('_TODO.md'):
                continue
            try:
                with open(os.path.join(round_dir, name),
                          encoding='utf-8', errors='replace') as fh:
                    txt = fh.read()
            except OSError:
                continue
            n = len(re.findall(r'-\s*\[\s*\]', txt))
            result.append((name[:-len('_TODO.md')], n))
    return result


def _todo_line(round_dir):
    counts = todo_counts(round_dir)
    if not counts:
        return '미완 TODO: (대상 없음)'
    total = sum(n for _, n in counts)
    detail = ' '.join(f'{lbl}={n}' for lbl, n in counts)
    return f'미완 TODO(합 {total}): {detail}'


def build_checklist(cmd, state, round_dir, timeout=PREFLIGHT_TIMEOUT):
    lines = [
        HEADER,
        _preflight_line(cmd, timeout),
        _state_line(state),
        _todo_line(round_dir),
    ]
    out = '\n'.join(lines[:MAX_LINES])
    encoded = out.encode('utf-8')
    if len(encoded) > MAX_BYTES:  # 자체 절단
        out = encoded[:MAX_BYTES].decode('utf-8', 'ignore')
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(
        description='세션 시작 주입용 실측 체크리스트 래퍼(전부 실측·LLM 없음).')
    ap.add_argument('--preflight-cmd', default=DEFAULT_PREFLIGHT,
                    help='preflight 실행 명령(기본: javis_preflight.py)')
    ap.add_argument('--state', default=None, help='SESSION_STATE.md 경로')
    ap.add_argument('--round-dir', default='_round', help='_round 디렉터리 경로')
    args = ap.parse_args(argv)

    state = args.state or os.path.join(args.round_dir, 'SESSION_STATE.md')
    print(build_checklist(args.preflight_cmd, state, args.round_dir))
    return 0


if __name__ == '__main__':
    sys.exit(managed_main(main))
