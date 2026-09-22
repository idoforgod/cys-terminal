#!/usr/bin/env python3
"""D-01 v2 · D-03 · D-12 격리 데몬 E2E — 빌드된 cysd/cys 로 미제출 입력 계수와 직접 send 초안 게이트를 실측한다.

기원: bugverify-3problems-20260921 V3a 하네스(설치 바이너리 실측)를 빌드 산출물 기준으로 이관한 것
(0.14.39 · WP-C-input). 격리 세트 = 프로젝트 공식 E2E(docs/queue_starvation_e2e.py:70-91) + 안전 opt-out.
라이브 소켓에는 한 줄도 쓰지 않는다(자기 소켓만). 결과는 전부 OUT 에 남긴다.

무엇을 재는가(수리 전 실측 → 수리 후 기대):
  · D-01 B(유휴 좌석 · 포커스 보고 10회): 큐 배달 12.85s → **≈2~3s** · `queue.input_pending_reset` 이벤트 0
  · D-01 C(출력 계속 좌석 · 포커스 보고 10회): 50s 창 내내 미배달 → **≈2~3s 배달**
  · D-12 F(사람 초안 뒤 3.6s 멈춤 → `cys send` → `send-key Return`): 연접 제출 → **둘 다 QUEUED 폴백 ·
    입력줄에 초안 그대로** (`queue.draft_gate_denied` 2건)
  · D(화면 초안 · 계수 0)·G(사람 초안): 배달 보류(정상) · E(커서 줄 머리): 문서화된 한계(배달됨)
  · P1: GUI 경로 경로 삽입(owner_token · machine_origin) 회귀 대조

실행:
  cargo build --bin cysd --bin cys
  CYS_E2E_OUT=/path/out python3 docs/pending_input_e2e/run_e2e.py /short/symlink/path
  (인자 = 소켓 경로 길이 제한(104) 을 피하는 짧은 심링크 자리 · CYS_E2E_CYSD/CYS_E2E_CYS 로 바이너리 지정 가능)
"""
import json
import os
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
_TARGET = os.environ.get("CARGO_TARGET_DIR", os.path.join(REPO, "target"))
CYSD = os.environ.get("CYS_E2E_CYSD", os.path.join(_TARGET, "debug", "cysd"))
CYS = os.environ.get("CYS_E2E_CYS", os.path.join(_TARGET, "debug", "cys"))
PY = sys.executable
FAKE = os.path.join(HERE, "fake_tui.py")
OUT = os.environ.get("CYS_E2E_OUT", os.path.join(_TARGET, "pending-input-e2e"))
RUN = os.path.join(OUT, "_sandbox", "run-" + time.strftime("%H%M%S"))
SHORT = sys.argv[1]  # 짧은 심링크 경로(→ RUN) — 소켓 경로 104바이트 제한
T0 = time.time()
LOG = []


def log(section, **kw):
    rec = dict(t=round(time.time() - T0, 3), section=section, **kw)
    LOG.append(rec)
    print(json.dumps(rec, ensure_ascii=False), flush=True)


def setup():
    os.makedirs(RUN)
    os.makedirs(OUT, exist_ok=True)
    for d in ("home", "s", "config", "captures", "state", "shim", "rx"):
        os.makedirs(os.path.join(RUN, d))
    shimlog = os.path.join(RUN, "shim-calls.log")
    for name in ("launchctl", "curl", "lsof", "cys"):
        p = os.path.join(RUN, "shim", name)
        with open(p, "w") as f:
            f.write('#!/bin/sh\necho "$(date +%%H:%%M:%%S) %s $*" >> "%s"\nexit 0\n' % (name, shimlog))
        os.chmod(p, 0o755)
    if os.path.islink(SHORT):
        os.unlink(SHORT)
    os.symlink(RUN, SHORT)
    sock = os.path.join(SHORT, "s", "cys.sock")
    assert len(sock.encode()) < 100, len(sock)
    env = {
        "HOME": os.path.join(RUN, "home"),
        "PATH": os.path.join(RUN, "shim") + ":/usr/bin:/bin",
        "LANG": "en_US.UTF-8",
        "CYS_SOCKET": sock,
        "CYS_PACK_DIR": os.path.join(RUN, "pack"),
        "CYS_CONFIG_DIR": os.path.join(RUN, "config"),
        "CYS_PACK_CAPTURES_DIR": os.path.join(RUN, "captures"),
        "CYS_STATE_DIR": os.path.join(RUN, "state"),
        "CYS_NO_PERSONAL_HOOK_MERGE": "1",
        "CYS_NO_OFFICE_BRIDGE": "1",
        "CYS_NO_AUTORESTORE": "1",
        "CYS_BOOT_SUPERVISOR": "0",
        "CYS_ALERT_ROUTE": "0",
        "CYS_NO_AUTOSTART": "1",
        "PHOENIX_LAUNCHCTL": os.path.join(RUN, "shim", "launchctl"),
    }
    return sock, env


class D:
    def __init__(self, sock, env):
        self.sock, self.env, self.proc = sock, env, None

    def start(self):
        lf = open(os.path.join(RUN, "cysd.log"), "wb")
        self.proc = subprocess.Popen([CYSD], env=self.env, stdout=lf, stderr=lf, cwd=RUN,
                                     stdin=subprocess.DEVNULL)
        for _ in range(600):
            if os.path.exists(self.sock):
                try:
                    self.rpc("system.ping", {})
                    return
                except Exception:
                    pass
            if self.proc.poll() is not None:
                raise RuntimeError("cysd exited rc=%s" % self.proc.returncode)
            time.sleep(0.1)
        raise RuntimeError("daemon did not come up")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def rpc(self, method, params):
        """실패해도 예외를 던지지 않고 응답 원문(dict)을 돌려준다."""
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(10)
        s.connect(self.sock)
        s.sendall((json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            c = s.recv(65536)
            if not c:
                break
            buf += c
        s.close()
        return json.loads(buf.decode())

    def token(self):
        p = os.path.join(os.path.dirname(os.path.realpath(self.sock)), "operator.token")
        try:
            return open(p).read().strip()
        except OSError:
            return None


def events_recorder(sock, path, stop):
    s = socket.socket(socket.AF_UNIX)
    s.connect(sock)
    s.sendall((json.dumps({"id": 1, "method": "events.stream", "params": {"after_seq": 0}}) + "\n").encode())
    s.settimeout(1.0)
    with open(path, "wb") as f:
        while not stop.is_set():
            try:
                c = s.recv(65536)
            except socket.timeout:
                continue
            if not c:
                break
            f.write(c)
            f.flush()


def mk(d, tag, mode="idle", typed="", home_cursor=False, multiline=False):
    rxlog = os.path.join(RUN, "rx", tag + ".rx")
    envp = 'FAKE_MODE=%s FAKE_TYPED="%s" FAKE_RXLOG="%s" ' % (mode, typed, rxlog)
    if home_cursor:
        envp += "FAKE_HOME_CURSOR=1 "
    cmd = envp + "%s -u %s" % (PY, FAKE)
    r = d.rpc("surface.create", {"cmd": cmd, "rows": 24, "cols": 100})
    res = r.get("result", r)
    sid = res.get("surface_id") or res.get("id")
    m = d.rpc("surface.set_meta", {"surface_id": sid, "agent": "claude"})
    log("mk", tag=tag, sid=sid, create_ok=r.get("ok"), set_meta_ok=m.get("ok"))
    return sid, rxlog


def human(d, sid, text):
    tok = d.token()
    p = {"surface_id": sid, "text": text, "quiet": True, "human": True, "queued": False,
         "clear_first": False, "machine_origin": False, "owner_token": tok, "operator_token": tok}
    return d.rpc("surface.send_text", p)


def gui_inject(d, sid, text, with_token=True):
    """ui/src/main.ts:4970 injectRawToPane → src-tauri/src/main.rs:515-546 send_input(machineOrigin=true)."""
    p = {"surface_id": sid, "text": text, "quiet": True, "human": True, "queued": False,
         "clear_first": False, "machine_origin": True}
    if with_token:
        p["owner_token"] = d.token()
    return d.rpc("surface.send_text", p)


def machine(d, sid, text):
    return d.rpc("surface.send_text", {"surface_id": sid, "text": text})


def enqueue(d, sid, text):
    return d.rpc("surface.send_text", {"surface_id": sid, "text": text, "queued": True, "from": "surface:99"})


def screen(d, sid):
    r = d.rpc("surface.read_text", {"surface_id": sid})
    res = r.get("result", r)
    t = res.get("text") if isinstance(res, dict) else None
    return t if t is not None else json.dumps(res, ensure_ascii=False)


def prompt_rows(d, sid):
    return [ln for ln in screen(d, sid).splitlines() if ("❯" in ln or "SUBMIT" in ln)]


def rx_bytes(rxlog):
    try:
        return "".join(l.split()[1] for l in open(rxlog) if l.strip())
    except OSError:
        return ""


def short(r):
    if r.get("ok"):
        return {"ok": True, "result": r.get("result")}
    return {"ok": False, "error": r.get("error")}


def cli(d, *args):
    env = dict(d.env)
    p = subprocess.run([CYS, "--socket", d.sock] + list(args), env=env, capture_output=True, text=True,
                       timeout=30, stdin=subprocess.DEVNULL)
    return {"argv": ["cys", "--socket", "<sandbox>"] + list(args), "rc": p.returncode,
            "stdout": p.stdout.strip()[:400], "stderr": p.stderr.strip()[:400]}


def qrows(d, sid):
    r = d.rpc("queue.list", {"surface_id": sid})
    res = r.get("result", r)
    rows = res.get("entries") or []
    return [e for e in rows if e.get("surface_id") == sid]


def main():
    sock, env = setup()
    json.dump({k: v for k, v in env.items()}, open(os.path.join(OUT, "daemon-env.json"), "w"), indent=1)
    d = D(sock, env)
    stop = threading.Event()
    try:
        d.start()
        log("daemon", pid=d.proc.pid, sock=sock, real_state=os.path.realpath(os.path.dirname(sock)))
        th = threading.Thread(target=events_recorder, args=(sock, os.path.join(OUT, "events.jsonl"), stop), daemon=True)
        th.start()
        ident = d.rpc("system.identify", {})
        log("identify", resp=ident.get("result", ident))

        # ───────── 표면 일괄 생성 ─────────
        sP, rxP = mk(d, "P1")
        sB, rxB = mk(d, "D01-B-idle-focus")
        sB0, rxB0 = mk(d, "D01-B0-idle-control")
        sC, rxC = mk(d, "D01-C-stream-focus", mode="stream")
        sC0, rxC0 = mk(d, "D01-C0-stream-control", mode="stream")
        sD, rxD = mk(d, "D12-D-screen-draft", typed="owner draft on screen")
        sE, rxE = mk(d, "D12-E-draft-homecursor", typed="owner draft home cursor", home_cursor=True)
        sF, rxF = mk(d, "D12-F-direct-send")
        sG, rxG = mk(d, "D12-G-queued-vs-human-draft")
        time.sleep(1.5)
        log("screen0", sid=sP, rows=prompt_rows(d, sP))

        # ───────── D-01 / D-12 큐 검체를 먼저 걸어 두고(긴 폴링) 그 사이 P1 을 잰다 ─────────
        for i in range(10):
            r1 = human(d, sB, "\x1b[I"); r2 = human(d, sB, "\x1b[O")
            r3 = human(d, sC, "\x1b[I"); r4 = human(d, sC, "\x1b[O")
            if i == 0:
                log("D01.focus.first_resp", B=[short(r1), short(r2)], C=[short(r3), short(r4)])
        time.sleep(0.5)
        log("D01.after_focus", B_rx_hex=rx_bytes(rxB), B_rows=prompt_rows(d, sB), C_rx_len=len(rx_bytes(rxC)) // 2,
            C_rows=prompt_rows(d, sC))
        r = human(d, sG, "owner typing draft")
        log("D12.G.human_draft", resp=short(r))
        t_enq = time.time()
        for sid, name in ((sB, "B"), (sB0, "B0"), (sC, "C"), (sC0, "C0"), (sD, "D"), (sE, "E"), (sG, "G")):
            r = enqueue(d, sid, "[QUEUED-%s] queued report body" % name)
            log("enqueue", name=name, sid=sid, resp=short(r))

        names = {sB: "B", sB0: "B0", sC: "C", sC0: "C0", sD: "D", sE: "E", sG: "G"}
        timeline = open(os.path.join(OUT, "queue-timeline.jsonl"), "w")
        delivered_at = {}

        def poll_once():
            now = round(time.time() - t_enq, 2)
            for sid, name in names.items():
                rows = qrows(d, sid)
                if rows:
                    rec = {"t": now, "name": name, "depth": len(rows), "blocked_by": rows[0].get("blocked_by"),
                           "age_secs": rows[0].get("age_secs")}
                else:
                    rec = {"t": now, "name": name, "depth": 0}
                    delivered_at.setdefault(name, now)
                timeline.write(json.dumps(rec, ensure_ascii=False) + "\n")
            timeline.flush()

        poll_once()

        # ───────── P1 ─────────
        path_txt = "'/Users/user/Desktop/some dir/file name.txt' "
        r = gui_inject(d, sP, path_txt)
        time.sleep(0.4)
        log("P1.a.idle", resp=short(r), rows=prompt_rows(d, sP), rx_hex_tail=rx_bytes(rxP)[-40:])
        human(d, sP, "\x15"); time.sleep(0.3)
        poll_once()

        r0 = human(d, sP, "a")
        r = gui_inject(d, sP, path_txt)
        time.sleep(0.4)
        log("P1.b.right_after_human_byte", human_resp=short(r0), resp=short(r), rows=prompt_rows(d, sP))
        rm = machine(d, sP, "MACHINE-DIRECT")
        log("P1.b2.control_machine_direct_right_after_human_byte", resp=short(rm))
        human(d, sP, "\x15"); time.sleep(0.3)
        poll_once()

        r0 = human(d, sP, "\x1b[O"); r1 = human(d, sP, "\x1b[I")
        r = gui_inject(d, sP, path_txt)
        time.sleep(0.4)
        log("P1.c.right_after_focus_bytes", focus_resp=[short(r0), short(r1)], resp=short(r), rows=prompt_rows(d, sP))
        human(d, sP, "\x15"); time.sleep(0.3)

        r = gui_inject(d, sP, path_txt, with_token=False)
        time.sleep(0.4)
        log("P1.d.no_owner_token", resp=short(r), rows=prompt_rows(d, sP))
        human(d, sP, "\x15")
        r = gui_inject(d, 9999, path_txt)
        log("P1.e.error_shape.not_found", resp=short(r))
        poll_once()

        # 타이핑 가드 창(3s)을 흘려보낸 뒤: 포커스 바이트만 human 으로 → 곧바로 기계 직접 send (A9 면제 확인)
        time.sleep(3.6)
        human(d, sP, "\x1b[I")
        rm = machine(d, sP, "MACHINE-AFTER-FOCUS-ONLY")
        time.sleep(0.3)
        log("P1.f.control_machine_direct_after_focus_only", resp=short(rm), rows=prompt_rows(d, sP))
        human(d, sP, "\x15")
        poll_once()

        # ───────── D-12 F: 직접 경로(cys send — [CYCLE] 주입기가 쓰는 경로) ─────────
        r = human(d, sF, "owner half sentence ")
        c1 = cli(d, "send", "--surface", "surface:%d" % sF, "[CYCLE] injected within guard")
        log("D12.F1.cli_send_within_typing_guard", human_resp=short(r), cli=c1, rows=prompt_rows(d, sF))
        poll_once()
        time.sleep(3.6)   # 사람이 3초 넘게 멈춤(생각·IME 조합) — 입력줄에는 초안이 그대로
        poll_once()
        c2 = cli(d, "send", "--surface", "surface:%d" % sF, "[CYCLE] injected after 3.6s pause")
        time.sleep(0.5)
        rows_mid = prompt_rows(d, sF)
        c3 = cli(d, "send-key", "--surface", "surface:%d" % sF, "Return")
        time.sleep(0.6)
        log("D12.F2.cli_send_after_pause", cli_send=c2, rows_before_return=rows_mid, cli_send_key=c3,
            rows_after_return=prompt_rows(d, sF), rx_hex=rx_bytes(rxF))
        poll_once()

        # ───────── 폴링 계속(총 ~50s) ─────────
        while time.time() - t_enq < 50:
            poll_once()
            time.sleep(1.0)
        timeline.close()
        final = {}
        for sid, name in names.items():
            rows = qrows(d, sid)
            final[name] = {"depth": len(rows), "blocked_by": rows[0].get("blocked_by") if rows else None,
                           "delivered_at_s": delivered_at.get(name), "prompt_rows": prompt_rows(d, sid)}
        log("queue.final@50s", final=final)
        log("D12.E.rx", rx_hex=rx_bytes(rxE)[:400])
        log("D01.B.rx_total_bytes", n=len(rx_bytes(rxB)) // 2)
        for sid, name in list(names.items()) + [(sP, "P1"), (sF, "F")]:
            open(os.path.join(OUT, "screen-%s.txt" % name), "w").write(screen(d, sid))
    finally:
        stop.set()
        time.sleep(1.2)
        d.stop()
        log("daemon.stopped", rc=d.proc.returncode if d.proc else None)
        json.dump(LOG, open(os.path.join(OUT, "run-log.json"), "w"), ensure_ascii=False, indent=1)
        print("RUN=" + RUN)


if __name__ == "__main__":
    main()
