#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_queue_drill.py — T9 혼잡 드릴 (v0.13.17 큐 역압 3층 방어 E2E)

설계 근거: `_round/queue-backpressure/DESIGN-response-storm-fix-v1.md` §7 T9
목적: enforce/log 두 정책 모드에서 "수신자 busy 고정 + 다발 발신" 실사고형 혼잡을 격리
      데몬에 재현해, ①소프트캡 결정론 거부 ②멱등 병합 ③depth 수렴 ④무키 무병합
      ⑤log 모드 관측 정확성 ⑥TTL 적체 상한 ⑦System-origin **등급 분리**를 **결정론(exit code)**
      으로 판정한다. 자기채점 산문이 아니라 exit code 와 stdout JSON 이 사실이다.

★⑦ 재작성(2026-07-27 적대리뷰 N-6): 종전 7c "System-origin TTL 면제"는 **오라벨·공허 통과**였다.
  드릴이 `CYS_QUEUE_SYSTEM_TTL_SECS` 를 설정하지 않아 라이브 기본 14400s(4h)가 걸린 채로
  "TTL 창 경과 후에도 잔존"을 물었으니, 어떤 대기시간에도 참인 명제였다(무엇도 검증 못 함).
  게다가 B5 이후 System 은 **면제가 아니라 별도의 긴 등급**이다. 그래서 지금은 System TTL 을
  드릴 창(TTL_SYSTEM)에 맞춰 명시하고 **3등급 분리**를 각각 실검증한다:
    7c Agent TTL 창을 지난 나이인데도 System 은 잔존(= 더 긴 별도 등급)
    7d System TTL 창까지 가면 만기 이관(= 무한 면제 아님 · 무한 누적 재발 차단)
    7e 사람의 GUI 큐잉(System 라벨 `gui`)은 그 창을 지나도 잔존(= **더 긴 전용 등급**)
    7f 그 GUI 도 자기 등급(TTL_GUI) 창까지 가면 만기 이관(= **면제가 아니다**)

★⑦e/f 재작성(2026-07-27 최종 재검증 E4): 종전 7e 는 "GUI = TTL 면제"를 시연했는데, 그 면제는
  클라이언트 자기신고(`human:true`)에 걸려 있어 pane 밖 detach 프로세스가 한 줄로 **무기한 TTL 을
  위조 획득**하는 구멍이었다. 면제를 철회하고 GUI 전용 장주기 등급(라이브 기본 24h ·
  `CYS_QUEUE_GUI_TTL_SECS`)으로 바꿨으므로, 드릴도 "면제 시연"이 아니라 **등급 시연**을 한다 —
  GUI TTL 을 드릴 창(TTL_GUI)에 명시해 ①System 보다 길고 ②그래도 상한이 있음을 함께 못박는다
  (명시하지 않으면 라이브 기본 86400s 가 걸려 7f 가 영원히 공허 통과한다 — N-6 과 같은 형태).

격리 계약(javis_phoenix_harness 와 동일 — 라이브 무접촉):
  · 데몬은 ~/.cys/state-harness/cys.sock 에만 bind 하고 상태·dead-letter 도 그 디렉터리로 격리된다.
  · 라이브 소켓(~/.local/state/cys/cys.sock)·라이브 state 는 읽지도 쓰지도 않는다
    (harness.guard_isolation() 가 하드 가드).
  · 드릴 종료 시 harness.teardown() 이 격리 surface·데몬을 전멸시킨다(잔여 0).

Agent-origin 재현(위장이 아니라 실 경로):
  정책의 origin 판별(queue_policy::derive_origin)은 **자기신고 from 을 쓰지 않고** 커널 peer pid →
  조상 추적(resolve_caller_surface)으로 얻은 발신 surface 가 agent_meta 를 보유하는지로만 판정한다.
  따라서 드릴은
    ⓐ 격리 surface 를 하나 만들고 `surface.set_meta` 로 에이전트 등록(정당 경로 ② — 대상
      agent_meta 가 None 인 갓 만든 자식 surface 초기화. 오케스트레이터가 하는 것과 동일 RPC)
    ⓑ **그 pane 안에서** `cys send --queued` 를 실행
  하여 진짜 Agent-origin 발신을 만든다. 프로토콜 위장·필드 조작은 하지 않는다.
  반대로 System-origin 은 이 스크립트(pane 밖 프로세스)가 직접 RPC/CLI 로 보내면 그대로 성립한다
  (조상 추적이 어떤 surface 에도 닿지 않음 → QueueOrigin::System).

시간 압축: 격리 데몬 env 로 TTL·소프트캡·depth 경보를 짧게 잡는다(라이브 기본값 불변).
  · enforce 위상: SOFTCAP=5 · AGENT_TTL=600 · SYSTEM_TTL=600 · GUI_TTL=600
                  (TTL 이 소프트캡/병합 판정을 교란하지 않게 길게)
  · log     위상: SOFTCAP=5 · AGENT_TTL=6 · SYSTEM_TTL=45 · GUI_TTL=120
                  (적체 상한 + **3등급 분리** 실증이 주제)
  watchdog 틱은 5초 고정이라 TTL 실측 상한은 TTL+틱 ≈ 11초다.
  등급 비(6:45:120)는 라이브 비(3600:14400:86400 = 1:4:24)보다 짧은 쪽을 크게 벌렸다 —
  틱 해상도(5초)가 인접 등급의 sweep 창을 섞지 않게 하려면 짧은 쪽 TTL 대비 충분한 간격이 필요하다.

사용:
  python3 cysjavis-pack/bin/javis_queue_drill.py            # 전 판정 실행
  python3 cysjavis-pack/bin/javis_queue_drill.py --keep     # 실패 진단용: 격리 데몬 존치
exit code: 0=전 판정 PASS · 1=판정 실패 · 2=환경 부재(바이너리 미발견 등 실행 불가)
stdout: 마지막 줄부터 JSON 요약(판정별 pass/기대/실측). CI 비편입(로컬·릴리스 게이트 전용).
"""

import argparse
import importlib.util
import json
import os
import re
import socket
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
DBG = os.path.join(REPO, "target", "debug")

# ── 정책 파라미터(격리 데몬 env 로 주입) ──────────────────────────────────────
SOFTCAP = 5
TTL_LONG = 600      # enforce 위상 — TTL 이 소프트캡/병합 판정을 교란하지 않게
TTL_SHORT = 6       # log 위상 — 적체 상한 실증(Agent 등급)
# ★N-6: System 등급 TTL 을 **드릴 창에 맞춰 명시**한다. 미설정 시 라이브 기본 14400s 가 걸려
#   "System 은 잔존한다"가 무조건 참이 되는 공허 판정이 된다(종전 7c 결함).
TTL_SYSTEM = 45     # log 위상 — Agent(6s)보다 길고 드릴 안에서 반드시 도달하는 별도 등급
# ★E4 재수정: GUI 등급도 같은 이유로 **드릴 창에 명시**한다. 미설정이면 라이브 기본 86400s(24h)가
#   걸려 "GUI 는 잔존한다"가 무조건 참이 되는 공허 판정으로 되돌아간다(종전 7e 결함과 같은 형태).
#   System(45s)의 약 2.7배 — 7e(System 창 통과 시점) 관측과 7f(GUI 창 도달) 사이에 앞단계 지연을
#   흡수할 여유를 두려면 두 창 사이가 넉넉해야 한다.
TTL_GUI = 120       # log 위상 — System(45s)보다 길고 드릴 안에서 반드시 도달하는 GUI 전용 등급
DEPTH_ALERT = 2
TICK = 5.0          # governance.rs WATCHDOG_INTERVAL_SECS (상수)
HARD_CAP = 100      # queue_policy.rs QUEUE_HARD_CAP (상수)


def _find(name):
    env = os.environ.get("PHOENIX_HARNESS_" + name.upper())
    if env and os.path.exists(env):
        return env
    c = os.path.join(DBG, name)
    if os.path.exists(c):
        return c
    import shutil
    return shutil.which(name)


CYSD, CYS = _find("cysd"), _find("cys")
if not (CYSD and CYS):
    print("SKIP: cysd/cys 미발견(빌드 필요: cargo build -p cys-terminal). exit 2.")
    sys.exit(2)

os.environ["PHOENIX_HARNESS_CYSD"] = CYSD
os.environ["PHOENIX_CYS"] = CYS
_spec = importlib.util.spec_from_file_location(
    "phoenix_harness", os.path.join(HERE, "javis_phoenix_harness.py"))
h = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(h)
h.CYS, h.CYSD = CYS, CYSD
h.guard_isolation()

DRILL_DIR = os.path.join(h.HARN_DIR, "queue_drill")
DEAD_LETTERS = os.path.join(h.HARN_DIR, "dead-letters.jsonl")
EVENTS_LOG = os.path.join(DRILL_DIR, "events.jsonl")

RESULTS = []          # [(id, name, ok, detail dict)]
_events_proc = None


def log(m):
    print("[drill] %s" % m, flush=True)


def judge(jid, name, ok, detail):
    RESULTS.append({"id": jid, "name": name, "pass": bool(ok), "detail": detail})
    print(("[drill] %s %s — %s" % ("PASS" if ok else "FAIL", jid, name)), flush=True)
    if not ok:
        print("        실측: %s" % json.dumps(detail, ensure_ascii=False), flush=True)


# ── 격리 데몬 RPC (newline-delimited JSON · cys.rs request() 와 동일 프레이밍) ──
def rpc(method, params=None, timeout=10.0):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect(h.HARN_SOCK)
        s.sendall((json.dumps({"id": 1, "method": method, "params": params or {}}) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        return json.loads(buf.split(b"\n")[0].decode() or "{}")
    finally:
        s.close()


def queue_entries(sid):
    """대상 surface 의 큐 항목 전량(구조 그대로 — origin_class·idem_key·merged_count 포함)."""
    r = rpc("queue.list", {"surface_id": sid})
    return [e for e in (r.get("result", {}).get("entries") or []) if e.get("surface_id") == sid]


def depth(sid):
    return len(queue_entries(sid))


def oldest_age(sid):
    """대상 큐 최고령 항목의 나이(초). 비었으면 None.
    ★TTL 등급 판정은 '얼마나 기다렸나'(벽시계)가 아니라 '항목이 몇 초 묵었나'(enqueued_at)로
    해야 한다 — 드릴 앞단계가 느려져도 판정이 흔들리지 않는다."""
    now = time.time()
    ages = [now - e["enqueued_at"] for e in queue_entries(sid) if "enqueued_at" in e]
    return max(ages) if ages else None


def entries_matching(sid, tag):
    """preview(80자 절단)에 태그가 든 큐 항목만. 같은 surface 안의 레인 분리 확인용."""
    return [e for e in queue_entries(sid) if tag in (e.get("preview") or "")]


def dead_letters(reason=None, tag=None):
    """dead-letter 원장 조회. 원장이 SOT 이며 이벤트(링·유실 가능)에 의존하지 않는다."""
    rows = []
    try:
        with open(DEAD_LETTERS, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    j = json.loads(line)
                except ValueError:
                    continue
                if reason and j.get("reason") != reason:
                    continue
                if tag and tag not in (j.get("text") or ""):
                    continue
                rows.append(j)
    except OSError:
        pass
    return rows


def events(name=None, sid=None):
    """구독 스트림에서 큐 이벤트를 회수한다. ★페이로드 키는 `payload` 다(cys events 와이어 실측 —
    `data` 로 읽으면 전 필터가 조용히 0을 돌려주는 계측기 결함이 된다)."""
    rows = []
    try:
        with open(EVENTS_LOG, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line.startswith("{"):
                    continue
                try:
                    j = json.loads(line)
                except ValueError:
                    continue
                if j.get("type") != "event":
                    continue
                if name and j.get("name") != name:
                    continue
                if sid is not None and j.get("surface_id") != sid:
                    continue
                rows.append(j)
    except OSError:
        pass
    return rows


# ── 격리 환경 조립 ────────────────────────────────────────────────────────────
def boot(mode, ttl, system_ttl, gui_ttl):
    """정책 env 를 건 격리 데몬을 hermetic 하게 재기동한다(이전 위상 상태 전소거).

    ★system_ttl·gui_ttl 은 **필수 인자**다(N-6 · E4 재수정): 미설정이면 라이브 기본
    (14400s·86400s)이 걸려 해당 등급 판정이 공허 통과한다. 위상마다 명시하게 강제해
    그 회귀를 타입 수준에서 막는다."""
    global _events_proc
    stop_events()
    h.teardown()
    h._wipe_daemon_state()
    for p in (DEAD_LETTERS,):
        try:
            os.remove(p)
        except OSError:
            pass
    os.makedirs(DRILL_DIR, exist_ok=True)
    os.environ["CYS_QUEUE_POLICY_MODE"] = mode
    os.environ["CYS_QUEUE_AGENT_SOFTCAP"] = str(SOFTCAP)
    os.environ["CYS_QUEUE_TTL_SECS"] = str(ttl)
    os.environ["CYS_QUEUE_SYSTEM_TTL_SECS"] = str(system_ttl)
    os.environ["CYS_QUEUE_GUI_TTL_SECS"] = str(gui_ttl)
    os.environ["CYS_QUEUE_DEPTH_ALERT"] = str(DEPTH_ALERT)
    if not h.start_daemon() or not h.harness_ping():
        raise SystemExit("격리 데몬 기동 실패 (%s)" % h.DAEMON_LOG)
    log("격리 데몬 기동 — mode=%s softcap=%d agent_ttl=%ss system_ttl=%ss gui_ttl=%ss depth_alert=%d"
        % (mode, SOFTCAP, ttl, system_ttl, gui_ttl, DEPTH_ALERT))
    start_events()


def start_events():
    global _events_proc
    open(EVENTS_LOG, "w").close()
    _events_proc = subprocess.Popen(
        [CYS, "--socket", h.HARN_SOCK, "events", "--category", "queue"],
        stdout=open(EVENTS_LOG, "ab"), stderr=subprocess.DEVNULL)
    time.sleep(0.6)   # 구독 성립 대기


def stop_events():
    global _events_proc
    if _events_proc is not None:
        try:
            _events_proc.kill()
            _events_proc.wait(timeout=3)
        except Exception:
            pass
        _events_proc = None


def mk_surface():
    r = h.cys("new-surface", timeout=15)
    m = re.search(r"surface:(\d+)", (r.stdout or ""))
    if not m:
        raise SystemExit("격리 surface 생성 실패: %s" % (r.stderr or r.stdout))
    return "surface:" + m.group(1), int(m.group(1))


def run_in_pane(ref, cmd):
    """pane 안에서 셸 명령을 실행시킨다(직접 주입 → Return)."""
    h.cys("send", "--surface", ref, cmd, timeout=15)
    h.cys("send-key", "--surface", ref, "Return", timeout=15)


def make_busy(ref):
    """수신자 busy 고정 — 지속 출력으로 배달기의 quiet 게이트(기본 3초)를 계속 막는다."""
    run_in_pane(ref, "while true; do echo drill-tick; sleep 0.2; done")


def busy_confirmed(ref):
    r = h.cys("read-screen", "--surface", ref, timeout=10)
    return "drill-tick" in (r.stdout or "")


def register_agent(sid):
    """정당 경로 ②(대상 agent_meta=None 인 갓 만든 자식 surface 초기화)로 에이전트 등록."""
    r = rpc("surface.set_meta", {"surface_id": sid, "agent": "claude", "agent_bin": "claude"})
    return r.get("ok") is True


BURST_SH = r"""#!/bin/sh
# 인자: CYS SOCK TARGET N OUT TAG KEY
CYS="$1"; SOCK="$2"; TARGET="$3"; N="$4"; OUT="$5"; TAG="$6"; KEY="$7"
i=1
while [ "$i" -le "$N" ]; do
  if [ -n "$KEY" ]; then
    "$CYS" --socket "$SOCK" send --queued --idempotency-key "$KEY" --surface "$TARGET" "$TAG-$i" >>"$OUT" 2>&1
  else
    "$CYS" --socket "$SOCK" send --queued --surface "$TARGET" "$TAG-$i" >>"$OUT" 2>&1
  fi
  echo "RC $i $?" >> "$OUT"
  i=$((i+1))
done
echo "DRILL_BURST_DONE" >> "$OUT"
"""


def agent_burst(sender_ref, target_ref, n, tag, key=None, wait=60.0):
    """Agent-origin 다발 발신 — 발신 pane 안에서 실행되므로 커널 조상 추적이 그 surface 로 귀결된다.
    반환: 발신별 exit code 리스트(원문 로그 포함)."""
    script = os.path.join(DRILL_DIR, "burst.sh")
    with open(script, "w", encoding="utf-8") as f:
        f.write(BURST_SH)
    out = os.path.join(DRILL_DIR, "burst-%s.log" % tag)
    open(out, "w").close()
    run_in_pane(sender_ref, "/bin/sh %s %s %s %s %d %s %s %s" % (
        script, CYS, h.HARN_SOCK, target_ref, n, out, tag, key or ""))
    t0 = time.time()
    while time.time() - t0 < wait:
        try:
            body = open(out, "r", encoding="utf-8", errors="replace").read()
        except OSError:
            body = ""
        if "DRILL_BURST_DONE" in body:
            break
        time.sleep(0.2)
    body = open(out, "r", encoding="utf-8", errors="replace").read()
    rcs = [int(m.group(2)) for m in re.finditer(r"^RC (\d+) (\d+)$", body, re.M)]
    return rcs, body


def system_burst(target_ref, n, tag):
    """System-origin 발신 — 이 스크립트는 pane 밖 프로세스라 조상 추적이 어떤 surface 에도 닿지 않는다."""
    rcs = []
    for i in range(1, n + 1):
        r = h.cys("send", "--queued", "--surface", target_ref, "%s-%d" % (tag, i), timeout=15)
        rcs.append(r.returncode)
    return rcs


def gui_burst(target_sid, n, tag):
    """사람의 **GUI 큐잉** 재현 — 앱(Tauri)이 쓰는 RPC 그대로 `surface.send_text{queued,human}`.

    위장이 아니다: 이 스크립트는 pane 밖 프로세스라 조상추적이 어떤 surface 에도 닿지 않아
    origin 은 그대로 System 이고(derive_origin), `human` 플래그는 **분류를 바꾸지 않고**
    System 라벨만 `gui` 로 세분한다(handlers.rs "GUI 발신은 분류를 바꾸지 않고 System 라벨만
    gui 로 세분화"). 정책 등급이 아니라 라벨만 다른 두 레인을 만들어 **TTL 등급 분기**를 실검증한다
    (E4 재수정 이후 이 라벨이 주는 것은 면제가 아니라 GUI 전용 장주기 등급이다 — 7e/7f 참조).
    CLI 에는 이 플래그가 없어(cys send 에 --human 없음) RPC 로 직접 부른다.
    반환: 발신별 성공 여부(0=성공)."""
    rcs = []
    for i in range(1, n + 1):
        r = rpc("surface.send_text", {"surface_id": target_sid, "text": "%s-%d" % (tag, i),
                                      "queued": True, "human": True})
        rcs.append(0 if r.get("result") is not None and not r.get("error") else 1)
    return rcs


def wait_until(pred, timeout, period=0.5):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if pred():
            return True
        time.sleep(period)
    return pred()


# ── 위상 1: enforce ───────────────────────────────────────────────────────────
def phase_enforce():
    # enforce 위상은 TTL 이 소프트캡·병합 판정을 교란하지 않게 두 등급 모두 길게 잡는다.
    boot("enforce", TTL_LONG, TTL_LONG, TTL_LONG)
    r1_ref, r1 = mk_surface()
    r2_ref, r2 = mk_surface()
    a_ref, a = mk_surface()
    make_busy(r1_ref)
    make_busy(r2_ref)
    time.sleep(1.2)
    judge("E0", "수신자 busy 고정(quiet 게이트 차단 성립)",
          busy_confirmed(r1_ref) and busy_confirmed(r2_ref),
          {"r1": r1_ref, "r2": r2_ref})
    judge("E0b", "발신 surface Agent 등록(set_meta)", register_agent(a), {"sender": a_ref})

    # ── ① keyless 다발 → 소프트캡 도달 후 결정론 거부 + dead-letter + 소실 0 ──
    n1, tag1 = 16, "DRILLENF"
    rcs, body = agent_burst(a_ref, r1_ref, n1, tag1)
    admitted = sum(1 for c in rcs if c == 0)
    rejected = sum(1 for c in rcs if c != 0)
    ents = queue_entries(r1)
    remaining = len(ents)
    delivered = len(events("queue.delivered", r1))
    dl_soft = dead_letters("softcap_rejected", tag1)
    ev_rej = [e for e in events("queue.rejected", r1)
              if (e.get("payload") or {}).get("mode") == "enforce"]
    origin_all_agent = all(e.get("origin_class") == "agent" for e in ents)

    judge("1a", "Agent-origin 판별(큐 항목 전량 origin=agent)", origin_all_agent and remaining > 0,
          {"entries": [e.get("origin_class") for e in ents]})
    judge("1b", "소프트캡 도달 후 결정론 거부(적재=softcap, 나머지 전량 거부)",
          admitted == SOFTCAP and rejected == n1 - SOFTCAP,
          {"sent": n1, "admitted": admitted, "rejected": rejected, "softcap": SOFTCAP,
           "rc_series": rcs})
    judge("1c", "거부 오류코드 queue_softcap_exceeded + 재전송금지 지시 전달",
          "queue_softcap_exceeded" in body and "재전송하지 마라" in body,
          {"has_err": "queue_softcap_exceeded" in body,
           "has_directive": "재전송하지 마라" in body})
    judge("1d", "dead-letter(softcap_rejected) 행 수 = 거부 수",
          len(dl_soft) == rejected,
          {"dead_letter_rows": len(dl_soft), "rejected": rejected})
    judge("1e", "소실 0 (발신 총량 = 배달 + 큐잔량 + dead-letter)",
          n1 == delivered + remaining + len(dl_soft),
          {"sent": n1, "delivered": delivered, "remaining": remaining,
           "dead_letter": len(dl_soft)})
    judge("1f", "queue.rejected{mode:enforce} 이벤트 수 = 거부 수",
          len(ev_rej) == rejected, {"events": len(ev_rej), "rejected": rejected})

    # ── ③ depth 수렴(하드캡 불도달) ──
    judge("3", "depth 수렴 — 소프트캡에서 정지, 하드캡(100) 불도달",
          remaining <= SOFTCAP and remaining < HARD_CAP,
          {"depth": remaining, "softcap": SOFTCAP, "hard_cap": HARD_CAP})

    # ── ④ 무키 병합 0건 ──
    merged_ev = events("queue.merged", r1)
    dl_sup1 = dead_letters("superseded", tag1)
    judge("4", "무키 발신 병합 0건(동일 패턴 재전송 보존)",
          len(merged_ev) == 0 and len(dl_sup1) == 0
          and all(e.get("merged_count") == 0 for e in ents),
          {"merged_events": len(merged_ev), "superseded_rows": len(dl_sup1),
           "merged_counts": [e.get("merged_count") for e in ents]})

    # ── ② keyed 반복 → 큐 내 1건 유지 · merged_count 증가 · superseded 행 존재 ──
    n2, tag2, key2 = 8, "DRILLKEY", "drill-idem-1"
    rcs2, body2 = agent_burst(a_ref, r2_ref, n2, tag2, key=key2)
    ents2 = queue_entries(r2)
    dl_sup2 = dead_letters("superseded", tag2)
    merged_ev2 = events("queue.merged", r2)
    judge("2a", "keyed 반복 → 큐 내 1건 유지(제자리 교체)",
          len(ents2) == 1 and ents2[0].get("idem_key") == key2,
          {"depth": len(ents2), "idem_key": ents2[0].get("idem_key") if ents2 else None,
           "rc_series": rcs2})
    judge("2b", "merged_count 증가(= 병합 횟수)",
          bool(ents2) and ents2[0].get("merged_count") == n2 - 1,
          {"merged_count": ents2[0].get("merged_count") if ents2 else None,
           "expected": n2 - 1})
    judge("2c", "superseded dead-letter 행 존재(병합조차 정보 미폐기)",
          len(dl_sup2) == n2 - 1 and len(merged_ev2) == n2 - 1,
          {"superseded_rows": len(dl_sup2), "merged_events": len(merged_ev2),
           "expected": n2 - 1})
    judge("2d", "keyed 소실 0 (발신 총량 = 큐잔량 + superseded + 배달)",
          n2 == len(ents2) + len(dl_sup2) + len(events("queue.delivered", r2)),
          {"sent": n2, "remaining": len(ents2), "superseded": len(dl_sup2)})

    # depth_high 는 watchdog 틱(5초)의 배달 루프에서 평가된다 — 발신 직후 즉시 보면 아직 0이다.
    # 틱 2회분을 기다려 관측한다(발화 여부만 판정 · OOB 실주입은 best-effort 경계).
    wait_until(lambda: len(events("queue.depth_high")) > 0, 2 * TICK + 3)
    dh = events("queue.depth_high")
    judge("E9", "depth_high 통지 발화(OOB 하드 통지 토대 · best-effort 관측)",
          len(dh) > 0, {"depth_high_events": len(dh), "threshold": DEPTH_ALERT,
                        "surfaces": sorted({e.get("surface_id") for e in dh})})
    return {"enforce_sent": n1 + n2}


# ── 위상 2: log ───────────────────────────────────────────────────────────────
def phase_log():
    boot("log", TTL_SHORT, TTL_SYSTEM, TTL_GUI)
    r3_ref, r3 = mk_surface()   # ⑤⑥a keyless 적재 → TTL 만료
    r4_ref, r4 = mk_surface()   # ⑥b 지속 유입 적체 상한
    r5_ref, r5 = mk_surface()   # ⑦a~d System-origin(익명) 등급
    r6_ref, r6 = mk_surface()   # ⑦e/f 사람 GUI 큐잉(System 라벨 gui) — 전용 장주기 등급 레인
    b_ref, b = mk_surface()
    for ref in (r3_ref, r4_ref, r5_ref, r6_ref):
        make_busy(ref)
    time.sleep(1.2)
    judge("L0", "수신자 busy 고정(log 위상)",
          all(busy_confirmed(x) for x in (r3_ref, r4_ref, r5_ref, r6_ref)),
          {"receivers": [r3_ref, r4_ref, r5_ref, r6_ref]})
    judge("L0b", "발신 surface Agent 등록(set_meta)", register_agent(b), {"sender": b_ref})

    # ── ⑤ log 모드: 거부 없음 + rejected{mode:log} 관측 정확 ──
    n5, tag5 = 12, "DRILLLOG"
    rcs5, body5 = agent_burst(b_ref, r3_ref, n5, tag5)
    ents5 = queue_entries(r3)
    ev_log = [e for e in events("queue.rejected", r3)
              if (e.get("payload") or {}).get("mode") == "log"]
    ev_log_admitted = [e for e in ev_log if (e.get("payload") or {}).get("admitted") is True]
    dl_soft5 = dead_letters("softcap_rejected", tag5)
    judge("5a", "log 모드 — 소프트캡 초과분도 적재 허용(거부 0)",
          all(c == 0 for c in rcs5) and len(dl_soft5) == 0,
          {"rc_series": rcs5, "softcap_dead_letters": len(dl_soft5)})
    judge("5b", "queue.rejected{mode:log, admitted:true} 이벤트 = 소프트캡 초과 발신 수",
          len(ev_log) == n5 - SOFTCAP and len(ev_log_admitted) == n5 - SOFTCAP,
          {"log_events": len(ev_log), "admitted_flag": len(ev_log_admitted),
           "expected": n5 - SOFTCAP})
    judge("5c", "log 모드 소실 0 (발신 총량 = 큐잔량 + dead-letter + 배달)",
          n5 == len(ents5) + len(dead_letters(tag=tag5)) + len(events("queue.delivered", r3)),
          {"sent": n5, "remaining": len(ents5),
           "dead_letter": len(dead_letters(tag=tag5))})

    # ── ⑦ System 등급 2레인 적재(익명 System · 사람 GUI 큐잉) — TTL 창 대기 전에 적재 ──
    n7, tag7 = 8, "DRILLSYS"
    rcs7 = system_burst(r5_ref, n7, tag7)
    ents7 = queue_entries(r5)
    ev_rej7 = events("queue.rejected", r5)
    n7g, tag7g = 4, "DRILLGUI"
    rcs7g = gui_burst(r6, n7g, tag7g)
    ents7g = queue_entries(r6)
    judge("7a", "System-origin(익명 발신) 소프트캡 미적용 — 전량 적재",
          all(c == 0 for c in rcs7) and len(ents7) == n7 and len(ev_rej7) == 0,
          {"rc_series": rcs7, "depth": len(ents7), "sent": n7,
           "rejected_events": len(ev_rej7)})
    judge("7b", "System-origin 판별 정확(origin_class=system)",
          all(e.get("origin_class") == "system" for e in ents7),
          {"origins": [e.get("origin_class") for e in ents7]})
    # GUI 큐잉도 **분류는 System 그대로**여야 한다(라벨만 gui) — 분류가 바뀌면 소프트캡 정책이
    # 통째로 우회되므로(자기신고 human 으로 Agent 를 탈출하던 R1 결함) 그 자리를 못박는다.
    judge("7b2", "사람 GUI 큐잉 적재 — 분류는 System 유지(라벨만 gui · 정책 등급 불변)",
          all(c == 0 for c in rcs7g) and len(ents7g) == n7g
          and all(e.get("origin_class") == "system" for e in ents7g),
          {"rc_series": rcs7g, "depth": len(ents7g), "sent": n7g,
           "origins": [e.get("origin_class") for e in ents7g]})

    # ── ⑥a TTL 만료 → dead-letter(ttl) 이관 ──
    ttl_budget = TTL_SHORT + 3 * TICK + 4
    drained = wait_until(lambda: depth(r3) == 0, ttl_budget)
    ev_exp = events("queue.expired", r3)
    dl_ttl5 = dead_letters("ttl", tag5)
    judge("6a", "TTL 만료 → 폐기 아닌 dead-letter(ttl) 전량 이관",
          drained and len(dl_ttl5) == n5 and len(ev_exp) > 0,
          {"depth_after": depth(r3), "ttl_rows": len(dl_ttl5), "sent": n5,
           "expired_events": len(ev_exp), "budget_secs": ttl_budget})
    judge("6a2", "TTL 이관에서도 소실 0 (발신 총량 = dead-letter(ttl) + 잔량 + 배달)",
          n5 == len(dl_ttl5) + depth(r3) + len(events("queue.delivered", r3)),
          {"sent": n5, "ttl_rows": len(dl_ttl5), "remaining": depth(r3)})

    # ── ⑦c System 등급 분리: Agent TTL 창을 지난 나이인데도 System 은 잔존 ──
    #   판정 기준을 벽시계가 아니라 **항목의 나이**로 잡는다(oldest_age) — 앞단계가 느려져도
    #   "Agent 등급이었으면 이미 만기됐을 나이"라는 명제 자체는 흔들리지 않는다.
    aged_past_agent = wait_until(lambda: (oldest_age(r5) or 0) > TTL_SHORT + TICK,
                                 TTL_SHORT + 3 * TICK + 6)
    age7c = oldest_age(r5) or 0
    ents7c = queue_entries(r5)
    judge("7c", "System 등급 분리 — Agent TTL(%ss)+sweep 창을 넘긴 나이인데도 System 전량 잔존"
          % TTL_SHORT,
          aged_past_agent and len(ents7c) == n7 and len(dead_letters("ttl", tag7)) == 0,
          {"oldest_age_secs": round(age7c, 1), "agent_ttl": TTL_SHORT, "system_ttl": TTL_SYSTEM,
           "sweep_window": TTL_SHORT + TICK, "depth": len(ents7c), "sent": n7,
           "ttl_rows": len(dead_letters("ttl", tag7))})

    # ── ⑥b 적체 상한 = TTL 창 (지속 유입에도 depth 가 총 발신량으로 자라지 않음) ──
    rounds, batch, period, tag6 = 6, 3, 4.0, "DRILLTRK"
    series = []
    for k in range(rounds):
        agent_burst(b_ref, r4_ref, batch, "%s%d" % (tag6, k), wait=30.0)
        series.append(depth(r4))
        time.sleep(period)
    total6 = rounds * batch
    # 이론 상한: 한 항목의 최대 체류 = TTL + 틱(sweep 주기). 그 창 안에 들어오는 발신량 + 여유 1배치.
    bound = int(((TTL_SHORT + TICK) / period + 1) * batch) + batch
    steady = series[-1] <= max(series[:max(1, rounds // 2)]) + batch
    judge("6b", "적체 상한 = TTL 창 — 지속 유입에도 depth 가 총 발신량으로 자라지 않음",
          max(series) < total6 and max(series) <= bound and steady,
          {"depth_series": series, "total_sent": total6, "theoretical_bound": bound,
           "steady_state": steady, "ttl": TTL_SHORT, "tick": TICK})
    final_drained = wait_until(lambda: depth(r4) == 0, TTL_SHORT + 3 * TICK + 4)
    judge("6c", "유입 정지 후 TTL 창 내 전량 배수(적체 영구화 불가)",
          final_drained and len(dead_letters("ttl")) >= total6,
          {"depth_final": depth(r4), "ttl_rows_total": len(dead_letters("ttl"))})

    # ── ⑦d System 은 **무한 면제가 아니다** — 자기 등급 TTL 창에 닿으면 만기 이관 ──
    #   종전(B5 이전)에는 System 이 소프트캡·TTL 양쪽에서 면제라, 배달이 봉쇄된 pane 앞에서
    #   무한 누적됐다(라이브 원장 1h+ 실증). 그 회귀를 여기서 못박는다.
    sys_drained = wait_until(lambda: depth(r5) == 0, TTL_SYSTEM + 3 * TICK + 6)
    dl_ttl7 = dead_letters("ttl", tag7)
    ev_exp7 = events("queue.expired", r5)
    judge("7d", "System 별도 장주기 TTL(%ss) 도달 → 만기 이관(무한 면제·무한 누적 아님)"
          % TTL_SYSTEM,
          sys_drained and len(dl_ttl7) == n7 and len(ev_exp7) > 0,
          {"depth_after": depth(r5), "ttl_rows": len(dl_ttl7), "sent": n7,
           "expired_events": len(ev_exp7), "system_ttl": TTL_SYSTEM,
           "budget_secs": TTL_SYSTEM + 3 * TICK + 6})
    judge("7d2", "System 만기에서도 소실 0 (발신 총량 = dead-letter(ttl) + 잔량 + 배달)",
          n7 == len(dl_ttl7) + depth(r5) + len(events("queue.delivered", r5)),
          {"sent": n7, "ttl_rows": len(dl_ttl7), "remaining": depth(r5)})

    # ── ⑦e 사람의 GUI 큐잉은 **System 보다 긴 전용 등급**이다(System 라벨 gui) ──
    #   같은 System 분류인데도 익명 레인(r5)은 만기됐고 이 레인은 잔존해야 한다 = 라벨 분기 실증.
    #
    #   ★sweep 창을 반드시 넘겨서 본다(N-6 재발 방지): 만기는 5초 틱의 sweep 이 집행하므로
    #   `age > TTL` 만으로는 "아직 안 쓸린 것"과 "등급이 다른 것"을 구별할 수 없다 — 실제로 초안은
    #   age=47.1s(TTL 45s)에서 통과했는데 그건 등급 분기가 아니라 **틱 사이에 본 것**이었다.
    #   sweep 을 최소 2회 확실히 지난 나이(System TTL + 2틱 + 여유)까지 기다린 뒤에만 판정한다.
    #
    #   ★E4 재수정: 여기서 묻는 것은 "면제"가 **아니다**. 면제라면 어떤 나이에서도 참이라
    #   상한 철회를 못 잡는다(그게 종전 7e 의 결함이자, 자기신고 위조로 무기한을 얻던 구멍이다).
    #   지금은 "GUI 창(TTL_GUI) **안에서만** 잔존"을 묻고, 그 창을 넘긴 뒤는 7f 가 만기를 묻는다.
    gui_hold = TTL_SYSTEM + 2 * TICK + 4
    wait_until(lambda: (oldest_age(r6) or 0) > gui_hold, gui_hold + 3 * TICK)
    ents7e = entries_matching(r6, tag7g)
    age7e = oldest_age(r6) or 0
    judge("7e", "GUI 전용 장주기 등급 — System TTL(%ss)+sweep 2틱을 넘겨도 GUI 창(%ss) 안이면 전량 잔존"
          % (TTL_SYSTEM, TTL_GUI),
          len(ents7e) == n7g and len(dead_letters("ttl", tag7g)) == 0
          and gui_hold < age7e < TTL_GUI,
          {"depth": len(ents7e), "sent": n7g, "oldest_age_secs": round(age7e, 1),
           "system_ttl": TTL_SYSTEM, "gui_ttl": TTL_GUI, "required_age": gui_hold, "tick": TICK,
           "ttl_rows": len(dead_letters("ttl", tag7g)),
           "note": "잔존 실패 = state.rs effective_ttl 이 System 라벨 gui 를 분기하지 않는다는 뜻. "
                   "age >= gui_ttl 로 실패했다면 앞단계 지연으로 관측 창을 놓친 것이니 "
                   "TTL_GUI 를 늘려 재실행하라(판정 의미 자체는 유효)."})

    # ── ⑦f 그 GUI 도 **면제가 아니다** — 자기 등급 창(TTL_GUI)에 닿으면 만기 이관 ──
    #   ★이 판정이 E4 재수정의 본핀이다. GUI 라벨은 클라이언트 자기신고(human:true)로 붙으므로,
    #   등급이 무기한이면 pane 밖 detach 프로세스가 한 줄로 무기한 TTL 을 위조 획득한다.
    #   유계 등급이면 위조해도 얻는 게 TTL_GUI(라이브 24h) 뿐이라 위조할 값어치가 없다.
    #   만기는 폐기가 아니라 이관이므로 전문은 원장에 남는다(무손실) — 그 둘을 함께 못박는다.
    gui_drained = wait_until(lambda: len(entries_matching(r6, tag7g)) == 0,
                             TTL_GUI + 3 * TICK + 6)
    dl_ttl7g = dead_letters("ttl", tag7g)
    ev_exp7g = events("queue.expired", r6)
    judge("7f", "GUI 등급도 상한이 있다 — GUI TTL(%ss) 도달 → 만기 이관(무기한 면제 아님)"
          % TTL_GUI,
          gui_drained and len(dl_ttl7g) == n7g and len(ev_exp7g) > 0,
          {"depth_after": len(entries_matching(r6, tag7g)), "ttl_rows": len(dl_ttl7g),
           "sent": n7g, "expired_events": len(ev_exp7g), "gui_ttl": TTL_GUI,
           "budget_secs": TTL_GUI + 3 * TICK + 6,
           "note": "FAIL 이면 GUI 가 다시 무기한 면제라는 뜻 — 자기신고 human:true 로 "
                   "무기한 TTL 을 위조 획득하는 경로가 열려 있다"})
    judge("7f2", "GUI 만기에서도 소실 0 (발신 총량 = dead-letter(ttl) + 잔량 + 배달)",
          n7g == len(dl_ttl7g) + len(entries_matching(r6, tag7g))
          + len(events("queue.delivered", r6)),
          {"sent": n7g, "ttl_rows": len(dl_ttl7g),
           "remaining": len(entries_matching(r6, tag7g)),
           "delivered": len(events("queue.delivered", r6))})
    return {"log_sent": n5 + n7 + n7g + total6}


def main():
    ap = argparse.ArgumentParser(description="T9 큐 혼잡 드릴(격리 · 결정론)")
    ap.add_argument("--keep", action="store_true", help="실패 진단용: 격리 데몬을 종료하지 않는다")
    args = ap.parse_args()

    t0 = time.time()
    live_before = len(h.live_surfaces())
    summary = {}
    try:
        summary.update(phase_enforce())
        summary.update(phase_log())
    finally:
        stop_events()
        if not args.keep:
            residual = h.teardown()
        else:
            residual = h.harness_daemon_pids()
    live_after = len(h.live_surfaces())
    judge("Z1", "라이브 데몬 무접촉(surface 수 불변)", live_before == live_after,
          {"before": live_before, "after": live_after})
    if not args.keep:
        judge("Z2", "격리 데몬 잔여 0", not residual, {"residual_pids": residual})

    elapsed = round(time.time() - t0, 1)
    failed = [r for r in RESULTS if not r["pass"]]
    out = {
        "drill": "queue-backpressure T9",
        "elapsed_secs": elapsed,
        "params": {"softcap": SOFTCAP, "ttl_enforce": TTL_LONG, "ttl_log": TTL_SHORT,
                   "ttl_system_log": TTL_SYSTEM,
                   "depth_alert": DEPTH_ALERT, "watchdog_tick": TICK, "hard_cap": HARD_CAP},
        "socket": h.HARN_SOCK,
        "dead_letters": DEAD_LETTERS,
        "total": len(RESULTS), "passed": len(RESULTS) - len(failed), "failed": len(failed),
        "verdict": "PASS" if not failed else "FAIL",
        "judgements": RESULTS,
    }
    summary.update(out)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
