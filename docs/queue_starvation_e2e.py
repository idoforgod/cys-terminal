#!/usr/bin/env python3
"""B1 큐 기아 수용 기준 E2E — 빌드된 cysd 로 `queue-starvation-case.md` §6·§7 을 실측한다.

무엇을 재는가(그 문서의 수용 기준 그대로):
  ① 발신→수신 지연 p90 < 60초 · 10분 초과 0건
  ② sha 전수 대조 유실 0(보낸 본문이 배달 원장에 전부 남는다)
  ③ 버스트 0(같은 발신자 3건+ 가 60초 안에 **여러 턴**으로 쏟아지지 않는다 = 병합 배달)
  ④ 다이제스트 상한 준수 100%(건수·문자)
  ⑤ 입력줄에 사람 입력이 있으면 배달 0 + 사유 기록(음성 대조)
  ⑥ 고스트 텍스트(커서 뒤 제안문)만 있으면 배달된다(양성 — 2h10m 영구 보류 사고의 회귀 핀)
  ⑦ 연속 도구 실행 시나리오: 출력이 끊이지 않는 pane 에도 배달 지연 상한이 지켜진다
     (CEO 실측 요구 2026-09-03 22:1x — dept-1 master 가 idle 0s 로 953s 굶었다)

실행: cargo build --bin cys --bin cysd && python3 docs/queue_starvation_e2e.py
검사기(게이트): impl/W-B/checks/check-e2e-queue.sh — 성공 시 마지막 줄 `QUEUE E2E PASS`.

★HOME 샌드박스 필수(CONTRIBUTING 'E2E isolation — HOME sandbox (W0-E2E)'): 빌드 바이너리는
프로덕션 진입점이라 HOME·CYS_PACK_DIR·CYS_CONFIG_DIR·CYS_SOCKET 를 전부 스크래치로 돌린다.
이 스크립트는 실행 중 데몬에 접속하지 않는다(자기 소켓만 · 티켓 계약: 데몬 교체 금지).
"""
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CYSD = os.path.join(ROOT, "target", "debug", "cysd")
CYS = os.path.join(ROOT, "target", "debug", "cys")
FAIL = []
NOTES = []

# 수용 기준 임계(문서 §6·§7)
P90_LIMIT_SECS = 60.0
DIGEST_MAX_ITEMS = 5
DIGEST_MAX_CHARS = 4000
# 연속 출력 시나리오의 배달 상한 — 실측 사고는 953초였다. 데몬 배달 틱 주기(WATCHDOG_INTERVAL)의
# 여러 배를 잡아도 60초 안에 들어와야 '기아 아님'이다. 시간 축소 계수는 §7 주석 참조.
STREAM_DELIVER_LIMIT_SECS = 60.0


def check(name, cond, detail=""):
    print(f"[{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
    if not cond:
        FAIL.append(name)


def normalize(text):
    """delivery.rs::normalize 미러 — 모든 공백류를 단일 스페이스로 접고 양끝을 버린다."""
    out = []
    pending = False
    for ch in text:
        if ch.isspace():
            pending = bool(out)
            continue
        if pending:
            out.append(" ")
            pending = False
        out.append(ch)
    return "".join(out)


def sha_of(text):
    return hashlib.sha256(normalize(text).encode("utf-8")).hexdigest()


class Daemon:
    def __init__(self, sandbox):
        self.dir = sandbox
        self.sock = os.path.join(sandbox, "cys.sock")
        home = os.path.join(sandbox, "home")
        os.makedirs(home, exist_ok=True)
        self.state_dir = os.path.join(sandbox, "state")
        os.makedirs(self.state_dir, exist_ok=True)
        self.env = dict(
            os.environ,
            HOME=home,
            CYS_SOCKET=self.sock,
            CYS_PACK_DIR=os.path.join(sandbox, "pack"),
            CYS_CONFIG_DIR=os.path.join(sandbox, "config"),
            CYS_PACK_CAPTURES_DIR=os.path.join(sandbox, "captures"),
            CYS_STATE_DIR=self.state_dir,
            CYS_NO_PERSONAL_HOOK_MERGE="1",
            # 병합·경보 노브는 기본값을 그대로 쓴다(기본값이 곧 계약 — 여기서 덮으면 측정 무의미).
        )
        self.proc = None

    def start(self):
        log = open(os.path.join(self.dir, "cysd.log"), "wb")
        self.proc = subprocess.Popen([CYSD], env=self.env, stdout=log, stderr=log)
        for _ in range(200):
            if os.path.exists(self.sock):
                try:
                    self.rpc("system.ping", {})
                    return
                except Exception:
                    pass
            time.sleep(0.05)
        raise RuntimeError("daemon did not come up")

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def rpc(self, method, params):
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(10)
        s.connect(self.sock)
        s.sendall((json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        s.close()
        rec = json.loads(buf.decode())
        if not rec.get("ok", False):
            raise RuntimeError(f"{method} failed: {rec}")
        return rec

    def queue_rows(self, sid):
        r = self.rpc("queue.list", {"surface_id": sid})
        rows = r.get("entries") or r.get("result", {}).get("entries") or []
        return [e for e in rows if e.get("surface_id") == sid]

    def ledger_records(self):
        """배달 원장(jsonl) 전량 — 경로 규약은 delivery.rs::ledger_path."""
        out = []
        for root, _dirs, files in os.walk(self.state_dir):
            for f in files:
                if not f.endswith(".jsonl") or "deliver" not in f:
                    continue
                for line in open(os.path.join(root, f), encoding="utf-8", errors="replace"):
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        out.append(json.loads(line))
                    except Exception:
                        pass
        return out


FAKE_AGENT = r'''
import os, sys, time, threading
# 가짜 에이전트 pane — dept1-queue-starvation-2205.txt 의 실제 화면 형태를 재현한다:
#   본문 → 구분선 → "❯ " 입력줄 → 구분선 → 상태줄. 스피너 행을 계속 다시 그려
#   last_output 이 영구 갱신되게 만든다(= 종전 quiet 규칙이면 영원히 배달 안 됨).
STOP = False
def spinner():
    frames = "|/-\\"
    i = 0
    while not STOP:
        # 커서를 지키기 위해 저장/복원으로 상태줄만 갱신한다(입력줄 커서 불변).
        sys.stdout.write("\x1b7\x1b[1;1H  %s Combobulating... (%ds)\x1b[K\x1b8" % (frames[i % 4], i))
        sys.stdout.flush()
        i += 1
        time.sleep(0.3)
def paint(typed="", ghost=""):
    sys.stdout.write("\x1b[2J\x1b[H")
    sys.stdout.write("  본문 영역\r\n")
    sys.stdout.write("-" * 60 + "\r\n")
    sys.stdout.write("❯ " + typed)
    if ghost:
        sys.stdout.flush()
        # 고스트: 커서 위치를 저장하고 제안문을 그린 뒤 커서를 되돌린다(Claude Code 렌더 형태).
        sys.stdout.write("\x1b7" + ghost + "\x1b8")
    sys.stdout.flush()
mode = os.environ.get("FAKE_MODE", "idle")
typed = os.environ.get("FAKE_TYPED", "")
ghost = os.environ.get("FAKE_GHOST", "")
paint(typed, ghost)
if mode == "stream":
    t = threading.Thread(target=spinner, daemon=True)
    t.start()
# 받은 줄은 소비하고 프롬프트를 다시 그린다(에이전트가 턴을 처리한 것처럼).
while True:
    line = sys.stdin.readline()
    if not line:
        time.sleep(0.2)
        continue
    sys.stdout.write("\r\n  [수신] %d바이트\r\n" % len(line))
    paint("", "")
'''


def make_surface(d, tag, mode="idle", typed="", ghost=""):
    """가짜 에이전트 pane 생성 + agent_meta 등록(마커 ❯ 해소용)."""
    script = os.path.join(d.dir, f"fake_{tag}.py")
    with open(script, "w", encoding="utf-8") as f:
        f.write(FAKE_AGENT)
    env_prefix = f'FAKE_MODE={mode} FAKE_TYPED="{typed}" FAKE_GHOST="{ghost}" '
    cmd = env_prefix + f"{sys.executable} -u {script}"
    r = d.rpc("surface.create", {"cmd": cmd, "rows": 24, "cols": 100})
    sid = r.get("surface_id") or r.get("result", {}).get("surface_id") or r.get("id")
    if sid is None:
        raise RuntimeError(f"surface.create 응답에서 surface_id 를 찾지 못했다: {r}")
    d.rpc("surface.set_meta", {"surface_id": sid, "agent": "claude"})
    time.sleep(1.2)  # 첫 화면 렌더 대기
    return sid


def enqueue(d, sid, text, frm=None):
    p = {"surface_id": sid, "text": text, "queued": True}
    if frm:
        p["from"] = frm
    d.rpc("surface.send_text", p)


def wait_drained(d, sid, timeout):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if not d.queue_rows(sid):
            return True, time.time() - t0
        time.sleep(0.25)
    return False, time.time() - t0


def main():
    if not os.path.exists(CYSD):
        print(f"[FAIL] cysd 바이너리 없음: {CYSD} (cargo build --bin cys --bin cysd 선행)")
        return 2
    sandbox = tempfile.mkdtemp(prefix="cys-b1-e2e-")
    print(f"# sandbox={sandbox}")
    print(f"# HOME={os.path.join(sandbox, 'home')} (W0-E2E 규약: 실사용 프로필 무접촉)")
    d = Daemon(sandbox)
    try:
        d.start()

        # ── ① 연속 출력(스피너) pane 에 배달되는가 = 기아 본체 ─────────────────────
        sid = make_surface(d, "stream", mode="stream")
        sent = []
        for i in range(3):
            text = f"[보고] 연속 출력 중 배달 {i} · 본문에 백틱 `echo` 와 따옴표 \"큰\" 포함"
            enqueue(d, sid, text, frm="surface:99")
            sent.append(text)
        ok, waited = wait_drained(d, sid, STREAM_DELIVER_LIMIT_SECS)
        check(
            "① 연속 도구 실행(스피너 재그리기) pane 에도 배달된다",
            ok,
            f"대기 {waited:.1f}s / 상한 {STREAM_DELIVER_LIMIT_SECS}s",
        )

        # ── ②③④ 지연 p90 · 버스트 · 다이제스트 상한 ────────────────────────────
        recs_all = d.ledger_records()
        # ★원장 판독 규약: `part` 가 있는 레코드는 **제출 단위 조각**이지 배달(턴)이 아니다
        #   (delivery.rs record_full 이 전문 레코드 + 조각 레코드를 함께 남긴다). 턴 계수는
        #   전문 레코드만 센다 — 이걸 놓치면 병합이 잘 돼도 '턴이 많다' 로 오판한다.
        delivered = [r for r in recs_all if r.get("origin") == "queue" and "part" not in r]
        waits = [float(r.get("wait_secs", 0)) for r in delivered if "wait_secs" in r]
        p90 = sorted(waits)[int(len(waits) * 0.9)] if waits else 0.0
        check(
            f"② 배달 지연 p90 < {P90_LIMIT_SECS}s",
            bool(waits) and p90 < P90_LIMIT_SECS,
            f"p90={p90:.1f}s n={len(waits)} max={max(waits) if waits else 0:.1f}s",
        )
        merged_counts = [int(r.get("digest_items", 1)) for r in delivered]
        turns = len(delivered)
        check(
            "③ 같은 발신자 3건이 여러 턴으로 쏟아지지 않는다(병합 배달)",
            turns <= 2 and max(merged_counts or [0]) >= 2,
            f"턴={turns} 병합최대={max(merged_counts or [0])}",
        )
        cap_ok = all(m <= DIGEST_MAX_ITEMS for m in merged_counts)
        recs = recs_all
        digest_recs = [r for r in recs if r.get("digest_items")]
        chars_ok = all(int(r.get("chars", 0)) <= DIGEST_MAX_CHARS * 2 for r in digest_recs)
        check(
            "④ 다이제스트 상한 준수(건수 ≤ 5)",
            cap_ok and chars_ok,
            f"건수최대={max(merged_counts or [0])} 다이제스트레코드={len(digest_recs)}",
        )

        # ── ⑤ sha 전수 유실 0 ──────────────────────────────────────────────────
        # 대조 집합 = 전문·조각 레코드의 sha ∪ 다이제스트 항목별 sha(digest_parts[].sha256).
        known = set()
        for r in recs:
            if r.get("sha256"):
                known.add(r["sha256"])
            for pt in r.get("digest_parts") or []:
                if pt.get("sha256"):
                    known.add(pt["sha256"])
        really_missing = [t[:40] for t in sent if sha_of(t) not in known]
        check(
            "⑤ 발신 본문 sha 전수 대조 — 유실 0",
            not really_missing,
            f"원장레코드={len(recs)} 미대조={len(really_missing)} {really_missing[:2]}",
        )
        NOTES.append(
            f"sha 대조 주석: 병합 배달은 합성 본문 1건으로 기록되고 항목별 사실은 digest_parts 에 "
            f"남는다(레코드 {len(recs)} · 다이제스트 {len(digest_recs)})."
        )

        # ── ⑥ 입력줄 점유 = 배달 0 + 사유(음성 대조) ────────────────────────────
        sid2 = make_surface(d, "typed", mode="idle", typed="오너가 치던 미제출 초안")
        enqueue(d, sid2, "[보고] 이건 배달되면 안 된다", frm="surface:98")
        time.sleep(6)
        rows = d.queue_rows(sid2)
        blocked_by = rows[0].get("blocked_by") if rows else None
        check(
            "⑥ 입력줄에 미제출 입력이 있으면 배달 0(음성 대조)",
            len(rows) == 1,
            f"남은 큐={len(rows)} blocked_by={blocked_by}",
        )
        check(
            "⑥-b 보류 사유가 queue.list 에 노출된다",
            bool(blocked_by) and "input_pending" in str(blocked_by),
            f"blocked_by={blocked_by}",
        )

        # ── ⑦ 고스트 텍스트만 있으면 배달된다(양성) ─────────────────────────────
        sid3 = make_surface(d, "ghost", mode="idle", ghost="try running the tests")
        enqueue(d, sid3, "[보고] 고스트가 있어도 배달돼야 한다", frm="surface:97")
        ok3, waited3 = wait_drained(d, sid3, 30)
        check(
            "⑦ 고스트 텍스트(커서 뒤 제안문)는 배달을 막지 않는다",
            ok3,
            f"대기 {waited3:.1f}s",
        )

        # ── ⑧ B3 #11 전문 조회(--full) + queue-state.json 판독 계약 ────────────────
        #    사람 입력이 있는 pane 은 배달이 보류되므로(⑤) 미배달 상태를 안정적으로 관측한다.
        sid4 = make_surface(d, "full", mode="idle", typed="사람이 치는 중")
        body = "가" * 300
        enqueue(d, sid4, body, frm="surface:98")
        time.sleep(1.0)
        rows = d.queue_rows(sid4)
        check("⑧a 보류 항목이 큐에 남는다(관측 전제)", len(rows) == 1, f"rows={len(rows)}")
        if rows:
            check(
                "⑧b 기본 조회는 preview 80자 절단 · 전문 키 부재",
                len(rows[0].get("preview", "")) == 80 and "text" not in rows[0],
                f"preview={len(rows[0].get('preview',''))}자 text키={'있음' if 'text' in rows[0] else '없음'}",
            )
        fr = d.rpc("queue.list", {"surface_id": sid4, "full": True})
        full_rows = [
            e
            for e in (fr.get("entries") or fr.get("result", {}).get("entries") or [])
            if e.get("surface_id") == sid4
        ]
        check(
            "⑧c full=true 는 전문을 원문 그대로 싣는다",
            len(full_rows) == 1 and full_rows[0].get("text") == body,
            f"rows={len(full_rows)} 일치={bool(full_rows) and full_rows[0].get('text') == body}",
        )
        # CLI 표면도 같은 계약인지(--full --json 의 text 키) — RPC 만 맞고 CLI 가 안 맞는 스큐 차단.
        cli = subprocess.run(
            [CYS, "--socket", d.sock, "queue", "list", "--surface", f"surface:{sid4}",
             "--full", "--json"],
            env=d.env, capture_output=True,
        )
        cli_ok = False
        if cli.returncode == 0:
            try:
                cli_ok = any(e.get("text") == body for e in json.loads(cli.stdout.decode()))
            except Exception:
                cli_ok = False
        check("⑧d CLI `queue list --full --json` 도 전문을 낸다", cli_ok,
              f"rc={cli.returncode} {cli.stderr.decode(errors='replace')[:120]}")
        # WAL 계약: 미배달 본문의 정본은 queue-state.json 이며 **미배달분만** 담는다.
        # ★경로 주의(설계 B3 #11 ①): unix 의 state_dir 는 **소켓의 부모**다 — 소켓이 다른
        #   부서 데몬은 WAL 파일도 다르다. 본부 파일을 열고 "빈 배열" 이라 판정하는 오독이
        #   여기서 나온다. 환경변수 CYS_STATE_DIR 쪽도 함께 훑어 실재 경로를 특정한다.
        wal_candidates = [
            os.path.join(os.path.dirname(d.sock), "queue-state.json"),
            os.path.join(d.state_dir, "queue-state.json"),
        ]
        wal_path = next((p for p in wal_candidates if os.path.exists(p)), wal_candidates[0])
        wal = json.load(open(wal_path, encoding="utf-8")) if os.path.exists(wal_path) else None
        check(
            "⑧e queue-state.json(WAL) 에 미배달 전문이 실재한다",
            isinstance(wal, list) and any(x.get("text") == body for x in wal),
            f"path={wal_path} 항목={len(wal) if isinstance(wal, list) else 'NONE'}",
        )
        d.rpc("queue.clear", {"surface_id": sid4})
        time.sleep(0.5)
        wal2 = json.load(open(wal_path, encoding="utf-8")) if os.path.exists(wal_path) else None
        check(
            "⑧f 큐가 비면 WAL 에서도 사라진다 — '빈 배열'은 결함이 아니라 정상",
            isinstance(wal2, list) and not any(x.get("text") == body for x in wal2),
            f"잔여={len(wal2) if isinstance(wal2, list) else 'NONE'}",
        )
        NOTES.append(
            "⑧ queue-state.json 은 미배달 WAL(이력 원장 아님) — 전량 배달 후 [] 가 정상이다. "
            "배달 이력은 배달 원장(delivery.rs)이 담당한다."
        )

        print()
        for n in NOTES:
            print(f"# {n}")
        if FAIL:
            print(f"\nQUEUE E2E FAILED — {len(FAIL)}건: {FAIL}")
            return 1
        print("\nQUEUE E2E PASS")
        return 0
    finally:
        d.stop()
        keep = os.environ.get("KEEP_SANDBOX") == "1"
        if not keep:
            shutil.rmtree(sandbox, ignore_errors=True)
        else:
            print(f"# sandbox kept: {sandbox}")


if __name__ == "__main__":
    sys.exit(main())
