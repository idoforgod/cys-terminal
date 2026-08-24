#!/usr/bin/env python3
"""fake_agent_e2e.py — 관문 스텁을 **실제로 띄워** 키를 넣고 종료코드를 재는 e2e (U-26 ③).

    python3 scripts/fake_agent_e2e.py                  # exit 0 = 전건 일치
    python3 scripts/fake_agent_e2e.py --json
    python3 scripts/fake_agent_e2e.py --case bypass-return
    python3 scripts/fake_agent_e2e.py --mutate left-moves-focus   # ★오라클이 틀린 세계를 잡는가

★왜 소스 핀이 아니라 e2e 인가 — 이 캠페인이 확정한 4대 근본원인 중 하나가 **"진짜 e2e 가 0개"** 다.
  `assert!(소스에 이런 문자열이 있다)` 는 "그렇게 쓰여 있다" 만 증명한다. 이 파일은 다르다:
  진짜 프로세스를 **진짜 PTY**(posix)에 띄우고, 진짜 이스케이프 바이트를 밀어넣고,
  진짜 종료코드를 받는다. 그래서 여기서 초록이면 "그 키를 누르면 실제로 그렇게 된다" 이다.

★★이 파일이 지키는 단 하나의 사실 — **면책 창의 Return 은 좌석을 죽인다**
  구 설계의 `Left + Return` 통과 오라클은 **틀린 키를 박제**한다. 아래 `CASES` 는 그것을
  음성 축으로 못박는다: `Left+Return` 도 `Right+Return` 도 rc 1 이다. 통과는 `Down+Return`
  또는 숫자 `2` 뿐이다. 이 세 줄이 지워지면 2026-07-29 형태의 사고가 CI 초록으로 승인된다.

★안전: 스텁 프로세스 자기 자신 외에는 아무것도 뜨지 않고 아무것도 죽지 않는다.
  cys 데몬·좌석·사용자 프로필·네트워크 무접촉. 모든 실행은 유계 타임아웃을 가진다.
"""
import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
STUB = os.path.join(HERE, "fake_agent.py")
PY = sys.executable or "python3"
HAS_PTY = os.name == "posix"

KEY_BYTES = {
    "Up": b"\x1b[A", "Down": b"\x1b[B", "Right": b"\x1b[C", "Left": b"\x1b[D",
    "Return": b"\r", "Esc": b"\x1b", "Ctrl-D": b"\x04",
}


def _kb(k):
    if k in KEY_BYTES:
        return KEY_BYTES[k]
    return k.encode("utf-8")


# ── 세션 ────────────────────────────────────────────────────────────────────
class Session(object):
    """스텁 1회 실행. posix 는 실제 PTY, 그 외는 파이프(양쪽 다 **실제 프로세스**다)."""

    def __init__(self, scenario, env=None, idle=15.0):
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.q = queue.Queue()
        cmd = [PY, STUB, "--scenario", scenario, "--idle-timeout", str(idle)]
        e = dict(os.environ)
        e["PYTHONUNBUFFERED"] = "1"
        e.pop("CYS_FAKE_AGENT_MUTATE", None)
        if env:
            e.update(env)
        self.pty = HAS_PTY
        if self.pty:
            import pty
            import termios
            import tty
            self.mfd, sfd = pty.openpty()
            # ★에코 끄기: PTY 기본은 입력 바이트를 그대로 되울린다. 그러면 우리가 보낸 키가
            #   전사(transcript)에 섞여 "스텁이 출력한 것" 과 구별되지 않는다 — 관측 오염이다.
            try:
                tty.setraw(sfd, termios.TCSANOW)
            except Exception:
                pass
            self.p = subprocess.Popen(cmd, stdin=sfd, stdout=sfd, stderr=sfd,
                                      close_fds=True, env=e)
            os.close(sfd)
            self.wfd = self.mfd
        else:
            self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                      stderr=subprocess.STDOUT, env=e)
            self.mfd = None
        t = threading.Thread(target=self._pump)
        t.daemon = True
        t.start()
        self.reader = t

    def _pump(self):
        while True:
            try:
                if self.pty:
                    b = os.read(self.mfd, 4096)
                else:
                    b = self.p.stdout.read(1)
            except (OSError, ValueError):
                b = b""
            if not b:
                self.q.put(None)
                return
            with self.lock:
                self.buf.extend(b)
            self.q.put(b)

    def text(self):
        with self.lock:
            return bytes(self.buf).decode("utf-8", "replace").replace("\r\n", "\n")

    def settle(self, quiet=0.20, cap=3.0):
        """출력이 `quiet` 초간 멎을 때까지 기다린다(최대 `cap`). 키 사이의 경합 제거."""
        end = time.time() + cap
        while time.time() < end:
            try:
                if self.q.get(timeout=quiet) is None:
                    return
            except queue.Empty:
                return

    def send(self, key):
        b = _kb(key)
        try:
            if self.pty:
                os.write(self.wfd, b)
            else:
                self.p.stdin.write(b)
                self.p.stdin.flush()
        except (OSError, ValueError):
            return False        # 이미 죽었다 — 그것 자체가 관측 결과다
        return True

    def finish(self, timeout=10.0):
        self.send("Ctrl-D")
        try:
            rc = self.p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.p.kill()                     # ★자기 자식만 죽인다(패턴 kill 없음)
            rc = None
        self.reader.join(timeout=1.0)
        if self.pty:
            try:
                os.close(self.mfd)
            except OSError:
                pass
        else:
            for s in (self.p.stdin, self.p.stdout):
                try:
                    s.close()
                except Exception:
                    pass
        return rc


def drive(scenario, keys, env=None):
    s = Session(scenario, env=env)
    s.settle()
    for k in keys:
        if not s.send(k):
            break
        s.settle()
    rc = s.finish()
    return rc, s.text()


# ── 사례 (오라클) ───────────────────────────────────────────────────────────
# rc: 기대 종료코드 · alive: 최종보고까지 살아남았는가 · has/hasnt: 전사 부분문자열
CASES = [
    # ★면책 — 키 방향 교정의 본체. 이 다섯 줄이 이 파일의 존재 이유다.
    {"name": "bypass-return", "scn": "bypass-dialog", "keys": ["Return"],
     "rc": 1, "alive": False, "has": ["No, exit", "event=rejected", "rc=1"],
     "why": "기본 포커스가 `1. No, exit` — Return 한 발이 좌석을 죽인다(실측 B-1)"},
    {"name": "bypass-left-return", "scn": "bypass-dialog", "keys": ["Left", "Return"],
     "rc": 1, "alive": False, "has": ["event=ignored key=Left", "event=rejected"],
     "why": "★구 설계의 `Left+Return` 통과 오라클은 틀렸다 — 세로 리스트라 Left 는 무의미하고 "
            "뒤따르는 Return 은 여전히 `No, exit` 를 누른다"},
    {"name": "bypass-right-return", "scn": "bypass-dialog", "keys": ["Right", "Return"],
     "rc": 1, "alive": False, "has": ["event=ignored key=Right", "event=rejected"],
     "why": "좌우 어느 쪽도 세로 리스트의 포커스를 옮기지 못한다(대칭 확인)"},
    {"name": "bypass-down-return", "scn": "bypass-dialog", "keys": ["Down", "Return"],
     "rc": 0, "alive": True, "has": ["event=accepted index=2"],
     "why": "실측 통과 경로 ① — 아래방향 1회 + Return"},
    {"name": "bypass-digit-2", "scn": "bypass-dialog", "keys": ["2"],
     "rc": 0, "alive": True, "has": ["event=accepted index=2"],
     "why": "실측 통과 경로 ② — 숫자 2(코퍼스가 선언한 리터럴)"},

    # 폴더 신뢰 — 여기 Return 은 **안전**하다. 사고의 범인이 이 창이 아님을 못박는다.
    {"name": "trust-return-safe", "scn": "folder-trust", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["event=accepted index=1", "Yes, I trust this folder ✔"],
     "why": "기본 포커스가 Yes — Return 이 안전하게 통과시킨다(실측 B-3)"},
    {"name": "trust-then-bypass-kills", "scn": "trust-echo-double-return",
     "keys": ["Return", "Return"], "rc": 1, "alive": False,
     "has": ["Yes, I trust this folder ✔", "event=rejected"],
     "why": "★킬체인 재현: 1발째는 안전, **2발째가 죽인다**. 확인 에코가 화면에 남아 자동응답을 "
            "한 발 더 유발하면 그 두 번째가 면책 창의 `No, exit` 를 누른다"},

    # 로그인·OAuth — 기계가 통과시킬 수 없고, **죽지도 않는다**(허위 READY 의 근원).
    {"name": "login-return-side-effect", "scn": "login-select", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["event=side-effect", "Paste code here"],
     "why": "Return 은 브라우저를 여는 부작용이고 화면은 OAuth 프롬프트로 넘어간다(B-6)"},
    {"name": "oauth-prompt-loops", "scn": "oauth-code-prompt", "keys": ["Return", "Return"],
     "rc": 0, "alive": True, "has": ["event=loop", "Press Enter to retry"],
     "why": "빈 Return 은 무한 재시도 — 좌석은 계속 살아 있다"},
    {"name": "oauth-loop-never-dies", "scn": "oauth-loop-alive",
     "keys": ["Return"] * 5, "rc": 0, "alive": True, "min_events": ("event=loop", 5),
     "why": "5회를 눌러도 rc 가 생기지 않는다 = `seat_death_confirmed` 가 영원히 완료되지 않는다"},

    # 온보딩 — 관문 화면 전부에 `❯` 가 있다(ready_marker 비변별자).
    {"name": "theme-default-is-2", "scn": "theme-select", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["event=accepted index=2"],
     "why": "테마 화면의 기본 포커스는 2번(Dark mode)이다 — 실측 캡처가 그렇게 말한다"},
    {"name": "onboarding-chain-all-carets", "scn": "onboarding-chain",
     "keys": ["Return", "Return"], "rc": 0, "alive": True,
     "has": ["Choose the text style", "Select login method", "Paste code here"],
     "caret_screens": True,
     "why": "★연쇄 전 화면에 `❯` 가 있다 = ready_marker 단독 판정은 관문에서 **반드시 오탐**한다"},

    # 허위 READY 의 마지막 형태 — 화면도 정상, 생존도 정상, 질의만 실패.
    {"name": "seeded-unauth-query-fails", "scn": "seeded-unauthenticated",
     "keys": ["h", "i", "Return"], "rc": 0, "alive": True, "has": ["Not logged in"],
     "why": "관문 화면이 없고 프롬프트도 정상인데 질의만 실패한다 — 화면·생존 어느 축으로도 "
            "잡히지 않는다(실측 V-e E-2)"},
    {"name": "ready-prompt-echoes", "scn": "ready-prompt", "keys": ["h", "i", "Return"],
     "rc": 0, "alive": True, "has": ["(echo) hi"],
     "why": "양성 대조 — 관문이 없는 화면은 아무 일도 일어나지 않는다"},

    # 미지 관문 — 어떤 키로도 나아가지 않고 죽지도 않는다.
    {"name": "unknown-gate-stuck", "scn": "gated-alive-forever",
     "keys": ["Return", "Return", "Return"], "rc": 0, "alive": True,
     "min_events": ("event=ignored", 3),
     "why": "코퍼스에 없는 문면 — deny 목록 단독 판정이 뚫리는 지점(살아 있고, 나아가지 않는다)"},

    # 신기능 안내 — 죽이지는 않지만 **관측 전제를 바꾼다**.
    {"name": "feature-not-now-safe", "scn": "feature-announce", "keys": ["2"],
     "rc": 0, "alive": True, "has": ["event=accepted index=2"], "hasnt": ["[?1049h"],
     "why": "코퍼스 선언대로 `2. Not now` 를 고르면 화면 계약이 유지된다"},
    {"name": "feature-return-changes-contract", "scn": "feature-announce", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["[?1049h"],
     "why": "Return 단독은 기본 포커스(`Yes, try it`)를 눌러 **대체 화면**으로 들어간다 — "
            "죽지는 않지만 화면 판독 전제가 바뀐다(그래서 코퍼스가 기본 포커스를 안 따른다)"},

    # 감지 전용 관문 — 통과 오라클을 만들지 않는다는 사실 자체를 박제한다.
    {"name": "api-key-human-only", "scn": "api-key-dialog", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["event=loop"],
     "why": "자격증명은 사람만 넣는다 — 기계 통과 경로가 없다(감지 전용)"},
    {"name": "platform-unmeasured", "scn": "platform-setup", "keys": ["Return"],
     "rc": 0, "alive": True, "has": ["event=rejected-unmeasured"],
     "why": "★재지 않은 것을 단언하지 않는다 — 미측정 경로는 큰 소리로 미측정이라고 말한다"},
]

# 변조본이 반드시 적발돼야 하는 최소 사례(예산 절약 — 전건을 돌리지 않아도 탐지력은 증명된다).
MUTANT_CASES = ["bypass-return", "bypass-left-return", "bypass-down-return",
                "bypass-digit-2", "trust-return-safe"]


def check(case, rc, text):
    bad = []
    if rc != case["rc"]:
        bad.append("종료코드 %r (기대 %r)" % (rc, case["rc"]))
    alive = "event=final alive=1" in text
    if alive != case["alive"]:
        bad.append("생존 %s (기대 %s)" % (alive, case["alive"]))
    for s in case.get("has", []):
        if s not in text:
            bad.append("전사에 없어야 할 리 없는 문자열 부재: %r" % s)
    for s in case.get("hasnt", []):
        if s in text:
            bad.append("있으면 안 되는 문자열 존재: %r" % s)
    if case.get("min_events"):
        tok, n = case["min_events"]
        got = text.count(tok)
        if got < n:
            bad.append("%r %d회 (기대 ≥%d)" % (tok, got, n))
    if case.get("caret_screens"):
        screens = [b for b in text.split("[fake-agent]") if b.strip()]
        if sum(1 for b in screens if "❯" in b) < 2:
            bad.append("연쇄 화면에서 `❯` 가 2화면 미만 — 비변별자 축이 무너졌다")
    return bad


def run_cases(names=None, mutate=None):
    env = {"CYS_FAKE_AGENT_MUTATE": mutate} if mutate else None
    rows = []
    for c in CASES:
        if names and c["name"] not in names:
            continue
        t0 = time.time()
        rc, text = drive(c["scn"], c["keys"], env=env)
        bad = check(c, rc, text)
        rows.append({"name": c["name"], "scenario": c["scn"],
                     "keys": c["keys"], "rc": rc, "expect_rc": c["rc"],
                     "ok": not bad, "violations": bad, "why": c["why"],
                     "secs": round(time.time() - t0, 2)})
    return rows


def coverage():
    """전 시나리오가 최소 1개 사례로 실제 구동되는가 — 커버리지 공백을 숨기지 않는다."""
    out = subprocess.run([PY, STUB, "--list", "--json"], capture_output=True, text=True,
                         encoding="utf-8", timeout=60)
    if out.returncode != 0:
        return ["스텁 대장 조회 실패: %s" % out.stderr[-300:]], 0
    ids = {s["id"] for s in json.loads(out.stdout)["scenarios"]}
    used = {c["scn"] for c in CASES}
    miss = sorted(ids - used)
    return (["e2e 사례가 없는 시나리오: %s" % ", ".join(miss)] if miss else []), len(ids)


def main(argv=None):
    for _s in (sys.stdout, sys.stderr):
        try:
            _s.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    ap = argparse.ArgumentParser(description="관문 스텁 실기 e2e")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--case", default="", help="쉼표 목록")
    ap.add_argument("--mutate", default="", help="스텁 의미론 변조(계측 타당성 시험)")
    ap.add_argument("--expect-fail", action="store_true",
                    help="변조본 시험 — 하나라도 실패해야 exit 0(오라클이 살아 있다는 증거)")
    a = ap.parse_args(argv)

    names = {x.strip() for x in a.case.split(",") if x.strip()} or None
    if a.mutate and not names:
        names = set(MUTANT_CASES)
    rows = run_cases(names, mutate=a.mutate or None)
    cov, total = ([], 0) if names else coverage()
    failed = [r for r in rows if not r["ok"]]

    if a.expect_fail:
        ok = bool(failed)
        verdict = "GREEN" if ok else "RED"
        detail = ("변조 %r 를 사례 %s 가 적발" % (a.mutate, [r["name"] for r in failed])
                  if ok else "변조 %r 를 아무 사례도 적발하지 못했다 = 오라클 고장" % a.mutate)
    else:
        ok = not failed and not cov
        verdict = "GREEN" if ok else "RED"
        detail = "사례 %d/%d 일치 · 시나리오 커버 %d" % (len(rows) - len(failed), len(rows), total)

    out = {"verdict": verdict, "mode": ("mutant" if a.mutate else "oracle"),
           "mutate": a.mutate or None, "pty": HAS_PTY, "detail": detail,
           "coverage_gaps": cov, "cases": rows}
    if a.json:
        print(json.dumps(out, ensure_ascii=False, indent=1))
    else:
        for r in rows:
            print("%-4s %-28s %-24s rc=%s %s"
                  % ("OK" if r["ok"] else "FAIL", r["name"], r["scenario"], r["rc"],
                     "" if r["ok"] else " / ".join(r["violations"])))
        for g in cov:
            print("COVERAGE %s" % g)
        print("\n%s — %s (pty=%s)" % (verdict, detail, HAS_PTY))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
