#!/usr/bin/env python3
"""가짜 에이전트 TUI (V3a 검체) — Claude Code 의 입력 거동을 최소 재현한다.

  · raw 모드(커널 에코 없음) — 포커스 보고 ESC[I / ESC[O 는 **입력줄에 아무것도 남기지 않는다**
    (Claude Code 와 동형: 포커스 이벤트는 앱이 소비한다). cooked 모드였다면 tty 가 `^[[I` 를
    에코해 화면 축이 정당하게 Occupied 가 돼 버리므로 D-01 측정이 오염된다.
  · 기동 시 ESC[?1004h(포커스 보고 on) · ESC[?2004h(괄호 붙여넣기 on)를 낸다.
  · 화면 형태: 본문 → 괘선(U+2500) → "❯ <입력>" → 괘선 → 상태줄("? for shortcuts").
  · 받은 바이트는 전부 RX 로그(hex)에 남긴다 → "PTY 에 도달했는가" 의 1차 증거.
  · CR = 제출(본문에 [SUBMIT] 줄을 찍고 입력줄을 비운다) · Ctrl-U/Ctrl-C = 줄 비움 · DEL = 1자 삭제.

env:
  FAKE_MODE   idle | stream     (stream = 0.3s 마다 상단 스피너 행 재그리기 · 'esc to interrupt' 없음)
  FAKE_TYPED  기동 시 입력줄에 미리 놓을 초안(데몬 계수 0 · 화면 축 전용 검체)
  FAKE_GHOST  커서 **뒤**에 그릴 고스트 제안문(Claude Code prompt suggestion 형태)
  FAKE_HOME_CURSOR  "1" 이면 초안을 그린 뒤 커서를 마커 바로 뒤(줄 머리)로 옮긴다
  FAKE_RXLOG  RX 로그 경로
"""
import os
import sys
import termios
import threading
import time
import tty

MODE = os.environ.get("FAKE_MODE", "idle")
GHOST = os.environ.get("FAKE_GHOST", "")
HOME_CURSOR = os.environ.get("FAKE_HOME_CURSOR", "") == "1"
RXLOG = os.environ.get("FAKE_RXLOG", "")
RULE = "─" * 60
lock = threading.Lock()
line = os.environ.get("FAKE_TYPED", "")
body = []          # 제출 기록(본문 영역)
focus_events = 0
stop = False


def out(s):
    sys.stdout.write(s)
    sys.stdout.flush()


def rx(b):
    if RXLOG:
        with open(RXLOG, "a", encoding="utf-8") as f:
            f.write("%.3f %s\n" % (time.time(), b.hex()))


def paint():
    """전체 재그리기. 커서는 입력줄의 (초안 끝 | HOME_CURSOR 면 마커 뒤)에 둔다."""
    with lock:
        s = "\x1b[2J\x1b[H"
        s += "\r\n"                       # 1행 = 스피너 자리(stream 모드)
        s += "  fake-agent body\r\n"
        for b in body[-6:]:
            s += "  " + b + "\r\n"
        s += RULE + "\r\n"
        s += "❯ " + line             # ❯
        if GHOST and not line:
            s += "\x1b7\x1b[2m" + GHOST + "\x1b[0m\x1b8"   # 커서 저장 → 고스트 → 커서 복원
        s += "\x1b7"                      # 입력줄 커서 위치 저장
        s += "\r\n" + RULE + "\r\n" + "  ? for shortcuts"
        s += "\x1b8"                      # 커서 복원(입력줄)
        if HOME_CURSOR and line:
            s += "\r\x1b[2C"              # 줄 머리 + 2칸(마커+공백 뒤)
        out(s)


def spinner():
    frames = "|/-\\"
    i = 0
    while not stop:
        with lock:
            out("\x1b7\x1b[1;1H  %s Combobulating... (%ds)\x1b[K\x1b8" % (frames[i % 4], i))
        i += 1
        time.sleep(0.3)


def main():
    global line, focus_events
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    tty.setraw(fd)
    try:
        out("\x1b[?1004h\x1b[?2004h")
        paint()
        if MODE == "stream":
            threading.Thread(target=spinner, daemon=True).start()
        buf = b""
        in_paste = False
        while True:
            chunk = os.read(fd, 4096)
            if not chunk:
                time.sleep(0.1)
                continue
            rx(chunk)
            buf += chunk
            changed = False
            while buf:
                if in_paste:
                    end = buf.find(b"\x1b[201~")
                    if end < 0:
                        break                       # 봉투 닫힘 대기
                    line += buf[:end].decode("utf-8", "replace").replace("\r", " ").replace("\n", " ")
                    buf = buf[end + 6:]
                    in_paste = False
                    changed = True
                    continue
                if buf.startswith(b"\x1b[200~"):
                    buf = buf[6:]
                    in_paste = True
                    continue
                if buf.startswith(b"\x1b[I") or buf.startswith(b"\x1b[O"):
                    focus_events += 1               # 포커스 이벤트 — 입력줄 불변·출력 0
                    buf = buf[3:]
                    continue
                if buf[:1] == b"\x1b":
                    if len(buf) == 1:
                        break                       # 시퀀스 나머지 대기
                    if buf[1:2] == b"[":
                        j = 2
                        while j < len(buf) and not (0x40 <= buf[j] <= 0x7e):
                            j += 1
                        if j >= len(buf):
                            break
                        buf = buf[j + 1:]           # 그 밖의 CSI 는 버린다
                        continue
                    buf = buf[2:]                   # ESC+1바이트(Alt 조합 등) 버림
                    continue
                c = buf[:1]
                if c in (b"\r", b"\n"):
                    body.append("[SUBMIT] %r" % line)
                    line = ""
                    buf = buf[1:]
                    changed = True
                    continue
                if c in (b"\x15", b"\x03"):
                    line = ""
                    buf = buf[1:]
                    changed = True
                    continue
                if c == b"\x7f":
                    line = line[:-1]
                    buf = buf[1:]
                    changed = True
                    continue
                if c[0] < 0x20:
                    buf = buf[1:]                   # 그 밖의 제어문자 무시(Ctrl-A 등)
                    continue
                # UTF-8 한 글자
                n = 1
                b0 = c[0]
                if b0 >= 0xf0:
                    n = 4
                elif b0 >= 0xe0:
                    n = 3
                elif b0 >= 0xc0:
                    n = 2
                if len(buf) < n:
                    break
                line += buf[:n].decode("utf-8", "replace")
                buf = buf[n:]
                changed = True
            if changed:
                paint()
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)


if __name__ == "__main__":
    main()
