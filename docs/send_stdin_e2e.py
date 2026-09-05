#!/usr/bin/env python3
"""B3 #4 `cys send --stdin/--file` E2E — 백틱 셸 치환 사고를 실측 재현하고 봉인을 확인한다.

무엇을 재는가:
  ① `--stdin` 전문 채널: 백틱·`$(...)`·`$VAR`·따옴표가 화면에 **원문 그대로** 도착한다.
  ② 음성 대조(사고 재현): 같은 본문을 zsh 의 argv 경로(큰따옴표)로 보내면 셸이 먼저 실행해
     `PWNED` 가 화면에 나타난다 — ①의 통과가 '아무 경로나 안전' 이 아님을 증명한다.
  ③ `--file` 채널도 ①과 동일하게 원문을 보존한다.
  ④ 여러 줄 본문의 내부 개행이 보존되고 말미 개행 1개만 벗겨진다.
  ⑤ 사용 오류: 본문 채널을 둘 이상 주면 clap 이 거부한다(exit != 0).
  ⑥ 위험 문안 코퍼스(작은따옴표·백틱·$·큰따옴표·% 혼합) 왕복 무결성: 원본 sha / 파일 sha /
     왕복 sha 3자 동일(CSO 실측 형식 — handover/B3-4-shell-quoting-reference.md §2·§5).
  ⑦ 음성 대조(필수): 같은 코퍼스를 구 경로(argv·셸 인용)로 보내면 깨지거나 변조된다.
     ★단일따옴표는 작은따옴표 없는 본문을 보존한다 — 즉 argv 의 안전은 본문 내용에 의존하며
     어떤 단일 인용 방식도 코퍼스 전량을 보존하지 못한다(그것이 이 수정의 근거다).

배경(실사고): 2026-09-03 13:31 dept-1 에서 push 본문의 `` `cys claim-role worker` `` 가
발신 zsh 에서 실행돼 CSO 좌석이 worker 로 강등됐다(같은 기제 3건 · dept-1 2회 추가 실측).
본문 채널이 argv 하나뿐이면 CLI 에는 방어 지점이 없다 — 셸을 거치지 않는 채널이 해법이다.

실행: cargo build --bin cys --bin cysd && python3 docs/send_stdin_e2e.py
로그 규약: 성공 시 마지막 줄 `SEND STDIN E2E PASS`.

★HOME 샌드박스 필수(CONTRIBUTING 'E2E isolation — HOME sandbox (W0-E2E)'): 실사용 데몬·
소켓·프로필을 건드리지 않는다. 이 스크립트는 자기 소켓에만 접속한다.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CYSD = os.path.join(ROOT, "target", "debug", "cysd")
CYS = os.path.join(ROOT, "target", "debug", "cys")
FAIL = []

# 사고 본문 — 백틱 실행·명령치환·변수전개·따옴표를 한 문장에 모은다.
PAYLOAD = "[보고] `echo PWNED` $(echo PWNED2) $HOME \"큰따옴표\" '작은따옴표'"


def check(name, cond, detail=""):
    print(f"[{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
    if not cond:
        FAIL.append(name)


class Daemon:
    """샌드박스 전용 cysd — HOME·소켓·팩·설정 전부 스크래치."""

    def __init__(self, sandbox):
        self.dir = sandbox
        self.sock = os.path.join(sandbox, "cys.sock")
        self.home = os.path.join(sandbox, "home")
        os.makedirs(self.home, exist_ok=True)
        self.env = dict(
            os.environ,
            HOME=self.home,
            CYS_SOCKET=self.sock,
            CYS_PACK_DIR=os.path.join(sandbox, "pack"),
            CYS_CONFIG_DIR=os.path.join(sandbox, "config"),
            CYS_PACK_CAPTURES_DIR=os.path.join(sandbox, "captures"),
            CYS_STATE_DIR=os.path.join(sandbox, "state"),
            CYS_NO_PERSONAL_HOOK_MERGE="1",
        )
        self.env.pop("CYS_SURFACE_ID", None)  # 발신자 상속 차단(호출 셸의 좌석과 무관하게)
        self.proc = None

    def start(self):
        log = open(os.path.join(self.dir, "cysd.log"), "wb")
        self.proc = subprocess.Popen([CYSD], env=self.env, stdout=log, stderr=log)
        for _ in range(200):
            if os.path.exists(self.sock):
                r = self.cys("ping")
                if r.returncode == 0:
                    return
            time.sleep(0.05)
        raise RuntimeError("daemon did not come up")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def cys(self, *args, stdin_bytes=None, shell_cmd=None):
        """샌드박스 소켓에 붙는 cys 호출. shell_cmd 면 zsh 를 한 겹 태운다(음성 대조용)."""
        if shell_cmd is not None:
            cmd = ["zsh", "-c", shell_cmd]
        else:
            cmd = [CYS, "--socket", self.sock, *args]
        return subprocess.run(
            cmd,
            env=self.env,
            input=stdin_bytes,
            capture_output=True,
        )


def make_echo_surface(d):
    """받은 바이트를 그대로 화면에 되쓰는 pane(cat). 본문 훼손 여부를 눈으로 잰다."""
    r = d.cys("new-surface", "--cmd", "cat", "--rows", "24", "--cols", "200")
    if r.returncode != 0:
        raise RuntimeError(f"new-surface 실패: {r.stderr.decode(errors='replace')}")
    ref = r.stdout.decode().strip().split()[-1]
    time.sleep(0.6)
    return ref


def screen(d, ref):
    r = d.cys("read-screen", "--surface", ref)
    return r.stdout.decode(errors="replace")


# 위험 문안 코퍼스 — 작은따옴표·백틱·$·큰따옴표·% 를 섞는다(CSO 실측 문안을 1번으로 고정).
# 한 줄이어야 한다: 왕복 채널이 pty 정규모드(줄 단위)라 개행은 본문 경계가 된다.
CORPUS = [
    "재개: it's a `date` test with $HOME and \"quotes\" · 100% 그대로여야 한다",
    "[보고] `cys claim-role worker` 완료 — 좌석 강등 실사고 문안",
    "100% $(id -u) 'single' \"double\" `tick` %s %d",
    "mixed: $HOME ${PATH} $(echo x) `echo y` * ? [a-z] | ; & > <",
    "상태: 8/8 통과 · 유실 0 · sha=abc \"확인\" 'ok'",
]


def sha(text):
    import hashlib

    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def read_lines(path):
    """**완결된** 줄만 돌려준다(개행으로 닫힌 것). 미완결 꼬리는 아직 배달 중이므로 세지 않는다 —
    빈 파일을 `[""]` 로 세면 배달 전에 빈 줄을 읽고 통과해 버린다(측정이 값을 앞지르는 오류)."""
    if not os.path.exists(path):
        return []
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        return f.read().split("\n")[:-1]


def make_sink(d, sandbox, tag):
    """받은 바이트를 파일에 그대로 적는 pane — 화면 렌더(줄바꿈·폭)를 거치지 않는 왕복 채널."""
    out = os.path.join(sandbox, f"sink-{tag}.txt")
    r = d.cys("new-surface", "--cmd", f"cat > {out}", "--rows", "24", "--cols", "200")
    if r.returncode != 0:
        raise RuntimeError(f"sink pane 생성 실패: {r.stderr.decode(errors='replace')}")
    ref = r.stdout.decode().strip().split()[-1]
    time.sleep(0.6)
    return ref, out


def deliver_and_read(d, ref, out, before, sender):
    """sender() 로 본문을 넣고 Return 으로 줄을 확정한 뒤 새로 적힌 줄을 돌려준다."""
    rc = sender()
    d.cys("send-key", "--surface", ref, "Return")
    for _ in range(40):
        lines = read_lines(out)
        if len(lines) > before:
            return rc, lines[before]
        time.sleep(0.1)
    return rc, None


def corpus_roundtrip(d, sandbox):
    # ⑥ 신 경로(--stdin·--file): 원본·파일·왕복 sha 3자 동일.
    ref_new, sink_new = make_sink(d, sandbox, "new")
    new_ok = 0
    for i, item in enumerate(CORPUS):
        fpath = os.path.join(sandbox, f"corpus-{i}.txt")
        with open(fpath, "w", encoding="utf-8") as f:
            f.write(item + "\n")  # 말미 개행 1개는 벗겨지는 것이 계약이다
        file_sha = sha(open(fpath, encoding="utf-8").read().rstrip("\n"))

        before = len(read_lines(sink_new))
        _, got_stdin = deliver_and_read(
            d,
            ref_new,
            sink_new,
            before,
            lambda: d.cys(
                "send", "--surface", ref_new, "--stdin", stdin_bytes=item.encode() + b"\n"
            ),
        )
        before = len(read_lines(sink_new))
        _, got_file = deliver_and_read(
            d,
            ref_new,
            sink_new,
            before,
            lambda: d.cys("send", "--surface", ref_new, "--file", fpath),
        )
        same = (
            got_stdin is not None
            and got_file is not None
            and sha(got_stdin) == sha(item) == file_sha == sha(got_file)
        )
        if same:
            new_ok += 1
        check(
            f"⑥[{i}] 원본·파일·왕복 sha 3자 동일 (--stdin·--file)",
            same,
            f"orig={sha(item)[:12]} file={file_sha[:12]} "
            f"stdin={(sha(got_stdin)[:12] if got_stdin is not None else 'MISSING')} "
            f"file_ch={(sha(got_file)[:12] if got_file is not None else 'MISSING')}",
        )

    # ⑦ 음성 대조(필수): 같은 코퍼스를 구 경로(argv · 셸 인용)로 보내면 깨지거나 변조된다.
    #    이 실패가 없으면 ⑥ 의 통과가 무엇을 잡는지 증명되지 않는다(vacuous 게이트 거부).
    #
    #    ★실측이 가르쳐 준 정확한 명제(2026-09-04): "argv 경로는 항상 깨진다" 는 **거짓**이다.
    #    작은따옴표로 감싸면 작은따옴표가 없는 본문은 원문 그대로 간다. 즉 argv 의 안전은
    #    **본문 내용에 의존**하며, 어떤 단일 인용 방식도 코퍼스 전량을 보존하지 못한다 —
    #    그것이 곧 "안전을 호출자의 기억에 의존시킨다" 는 뿌리다(CEO 지침 ②).
    ref_old, sink_old = make_sink(d, sandbox, "old")
    matrix = {}
    for i, item in enumerate(CORPUS):
        for form, quoted in (("이중따옴표", f'"{item}"'), ("단일따옴표", f"'{item}'")):
            before = len(read_lines(sink_old))
            shell_cmd = f"{CYS} --socket {d.sock} send --surface {ref_old} {quoted}"
            rc, got = deliver_and_read(
                d, ref_old, sink_old, before, lambda: d.cys(shell_cmd=shell_cmd)
            )
            if rc.returncode != 0:
                verdict = f"셸오류(rc{rc.returncode})"
            elif got is None:
                verdict = "미배달"
            elif sha(got) != sha(item):
                verdict = "변조"
            else:
                verdict = "원문보존"
            matrix[(i, form)] = verdict

    for i in range(len(CORPUS)):
        dq = matrix[(i, "이중따옴표")]
        check(
            f"⑦a[{i}] 구 경로 이중따옴표는 이 문안을 보존하지 못한다",
            dq != "원문보존",
            f"이중따옴표={dq} · 단일따옴표={matrix[(i, '단일따옴표')]}",
        )
    sq_broken = [i for i in range(len(CORPUS)) if matrix[(i, "단일따옴표")] != "원문보존"]
    check(
        "⑦b 구 경로 단일따옴표도 코퍼스 전량을 보존하지 못한다",
        len(sq_broken) > 0,
        f"깨진 항목 {sq_broken} (작은따옴표를 품은 본문에서 인용이 닫힌다)",
    )
    check(
        "⑦c 어떤 단일 인용 방식도 코퍼스 전량을 보존하지 못한다 — argv 안전은 본문 내용 의존",
        all(
            any(matrix[(i, f)] != "원문보존" for i in range(len(CORPUS)))
            for f in ("이중따옴표", "단일따옴표")
        ),
        "전량 보존하는 인용 방식이 하나라도 있으면 '채널을 바꿔야 한다' 는 근거가 약해진다",
    )
    print(
        "# 구 경로 양태표: "
        + " | ".join(
            f"[{i}] 이중={matrix[(i, '이중따옴표')]} 단일={matrix[(i, '단일따옴표')]}"
            for i in range(len(CORPUS))
        )
    )
    print(f"# 신 경로 3자 sha 동일: {new_ok}/{len(CORPUS)}")


def main():
    for path in (CYSD, CYS):
        if not os.path.exists(path):
            print(f"[FAIL] 바이너리 없음: {path} (cargo build --bin cys --bin cysd 선행)")
            return 2
    sandbox = tempfile.mkdtemp(prefix="cys-b34-e2e-")
    print(f"# sandbox={sandbox}")
    print(f"# HOME={os.path.join(sandbox, 'home')} (W0-E2E 규약: 실사용 프로필 무접촉)")
    d = Daemon(sandbox)
    try:
        d.start()

        # ── ① --stdin 전문 채널: 원문 보존 ─────────────────────────────────────
        ref = make_echo_surface(d)
        r = d.cys("send", "--surface", ref, "--stdin", stdin_bytes=PAYLOAD.encode() + b"\n")
        time.sleep(0.8)
        s1 = screen(d, ref)
        check("① --stdin exit 0", r.returncode == 0, r.stderr.decode(errors="replace")[:200])
        check("① 백틱이 원문 그대로 도착", "`echo PWNED`" in s1, repr(s1[-160:]))
        check("① 명령치환 원문 보존", "$(echo PWNED2)" in s1)
        check("① 변수 전개 안 됨", "$HOME" in s1 and d.home not in s1)
        check("① 따옴표 보존", '"큰따옴표"' in s1 and "'작은따옴표'" in s1)
        check("① 치환 결과가 화면에 없다", "PWNED\n" not in s1 and "PWNED " not in s1)

        # ── ② 음성 대조: argv 경로(zsh 큰따옴표)는 사고를 재현한다 ────────────────
        ref2 = make_echo_surface(d)
        shell_cmd = f'{CYS} --socket {d.sock} send --surface {ref2} "{PAYLOAD}"'
        r2 = d.cys(shell_cmd=shell_cmd)
        time.sleep(0.8)
        s2 = screen(d, ref2)
        check("② argv 경로 exit 0", r2.returncode == 0, r2.stderr.decode(errors="replace")[:200])
        check(
            "② 사고 재현 — 셸이 백틱·$( ) 를 먼저 실행해 치환 결과가 도착한다",
            "PWNED" in s2 and "`echo PWNED`" not in s2,
            repr(s2[-160:]),
        )
        check("② 사고 재현 — $HOME 이 전개돼 도착한다", d.home in s2)

        # ── ③ --file 채널도 동일하게 보존 ───────────────────────────────────────
        ref3 = make_echo_surface(d)
        body_path = os.path.join(sandbox, "body.txt")
        with open(body_path, "w", encoding="utf-8") as f:
            f.write(PAYLOAD + "\n")
        r3 = d.cys("send", "--surface", ref3, "--file", body_path)
        time.sleep(0.8)
        s3 = screen(d, ref3)
        check("③ --file exit 0", r3.returncode == 0, r3.stderr.decode(errors="replace")[:200])
        check("③ --file 원문 보존", "`echo PWNED`" in s3 and "$HOME" in s3)

        # ── ④ 여러 줄: 내부 개행 보존 · 말미 개행 1개만 제거 ─────────────────────
        ref4 = make_echo_surface(d)
        r4 = d.cys(
            "send", "--surface", ref4, "--stdin", stdin_bytes="첫째줄\n둘째줄\n".encode()
        )
        time.sleep(0.8)
        s4 = screen(d, ref4)
        check("④ --stdin 여러 줄 exit 0", r4.returncode == 0)
        check("④ 내부 개행 보존(두 줄 모두 도착)", "첫째줄" in s4 and "둘째줄" in s4)

        # ── ⑤ 사용 오류: 본문 채널 중복은 거부 ──────────────────────────────────
        r5 = d.cys("send", "--surface", ref, "--stdin", "hello", stdin_bytes=b"x\n")
        check("⑤ --stdin + argv 동시 지정 거부", r5.returncode != 0, f"rc={r5.returncode}")
        r6 = d.cys("send", "--surface", ref, "--stdin", "--file", body_path, stdin_bytes=b"x\n")
        check("⑤ --stdin + --file 동시 지정 거부", r6.returncode != 0, f"rc={r6.returncode}")

        # ── ⑥⑦ 위험 문안 코퍼스 왕복 sha 대조 + 음성 대조 ─────────────────────
        #    CSO 실측 형식(handover/B3-4-shell-quoting-reference.md §2·§5)을 그대로 따른다:
        #    원본 sha / 파일 sha / 왕복 sha 3자 동일 + 구 경로가 실제로 깨지는 것을 함께 남긴다.
        corpus_roundtrip(d, sandbox)

        if FAIL:
            print(f"\nSEND STDIN E2E FAILED — {len(FAIL)}건: {FAIL}")
            return 1
        print("\nSEND STDIN E2E PASS")
        return 0
    finally:
        d.stop()
        if os.environ.get("KEEP_SANDBOX") == "1":
            print(f"# sandbox kept: {sandbox}")
        else:
            shutil.rmtree(sandbox, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
