#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_resource_gate_boot_anchor.py — WP6-6 (1) 부트 유예 앵커: 내용 우선 · mtime 은 표기된 폴백.

무엇을 막는가: `javis_resource_gate._boot_elapsed` 의 종전 근거는 `boot-epoch` 파일의 **mtime 하나**였다.
mtime 은 내용이 아니라 복사·동기화·touch·백업 복원으로 내용과 무관하게 움직이고, 부트 유예는
**완화(fail-open)** 판정이라 mtime 이 현재로 밀리면 없는 부트에 유예가 섰다. 이 검체는 그 발동
조건을 **실제 파일 조작으로 재현**하고(읽어 보니 mtime 을 쓴다 — 가 아니다), 내용 앵커(안 A 데몬
`daemon.started_at` → 안 B nonce 세대 대조)가 그것을 이기며, mtime 을 썼으면 그 사실이
`measured.boot_grace_reason == "mtime_fallback"` 로 남는 것을 증명한다.

밀폐: 게이트는 override 로만 부른다(라이브 ps·부서 소켓·데몬 0 — ⑧만 **가짜 `cys` 스텁 + 실소켓**으로
      `_dept_roster` 의 실제 status 경로를 태운다). `CYS_STATE_DIR`·`HOME`·레인 소켓은 임시 디렉터리.
      실행 규약(격리 env · 데몬 자동기동 금지):
        HOME=<tmp> CYS_STATE_DIR=<tmp> CYS_NO_AUTOSTART=1 PYTHONDONTWRITEBYTECODE=1 \\
          python3 bin/tests/test_resource_gate_boot_anchor.py

핀 목록
  ① ★발동 조건 ⓐ 재현 — 복사·동기화가 mtime 만 현재로 민 형상: 안 A/안 B 는 유예를 주지 않고,
     같은 파일 상태에서 안 B 를 불능으로 만들면(=종전 판정) 폴백이 **표기된 채** 유예 과다를 낸다
  ② 감독자 off 재부팅(epoch 파일 부재·낡음)인데 데몬이 방금 떴다고 말하면 유예가 선다(안 A)
  ③ 폴백을 썼으면 산출물에 남는다 — 상태 I/O 불능 · 내용 비-nonce 두 경로 모두 `mtime_fallback`
  ④ 음성 대조 — 근거가 전혀 없으면 유예 없음(`epoch_missing`) · 판독 불능 · 미래 시각 3종
  ⑤ 세대 교체 — 새 nonce+옛 mtime(백업 복원)=유예 없음 · 새 nonce+갓 쓴 파일(실제 재부팅)=유예 ·
     같은 nonce 에 옛 mtime 이 와도(cp -p) 유예는 유지(내용 앵커가 양방향으로 이긴다) ·
     ★성찰2: 새 nonce+미래 mtime(시계 역행)=유예 없음(clock_backwards)·상태 미기록(다음 호출도 안 열림)
  ⑥ 문서화된 한계(fail-open 성분) — 게이트가 이 세대를 본 적 없고 mtime 이 밀렸으면 첫 관측에
     유예가 선다(그래서 안 A 가 1차다) — 행동을 핀해 두어 바뀌면 보이게 한다
  ⑦ 종단 `--json` 방출 — `--dept-roster-override` 의 `started_at` 주입 → measured/checks/exit 계약
  ⑧ ★왕복 추가 0 — 가짜 `cys` 스텁 + 실제 리스닝 소켓으로 `_dept_roster` 의 status 경로를 태우고,
     그 **한 번의 응답**에서 started_at 이 재사용되며 스폰 횟수가 소켓 수(1)와 같음을 센다
  ⑨ 계약 불변 — `boot_elapsed_override` 우선 · measured 키 3종 · reason 닫힌 집합 · 로스터 모양 불변
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 RESOURCE-GATE-BOOT-ANCHOR-OK.
"""
import json
import os
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import types

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
GATE = os.path.join(BIN, "javis_resource_gate.py")

# ★AF_UNIX 경로 상한(macOS 104B)을 넘지 않게 짧은 루트를 고른다(⑧ 실소켓). /tmp 가 없으면 기본 tmp.
_TMP_BASE = "/tmp" if os.path.isdir("/tmp") and os.access("/tmp", os.W_OK) else None
_ROOT = tempfile.mkdtemp(prefix="ba-", dir=_TMP_BASE)
STATE = os.path.join(_ROOT, "state")
LANE_DIR = os.path.join(_ROOT, "lane")
LANE_SOCK = os.path.join(LANE_DIR, "cys.sock")
EPOCH = os.path.join(LANE_DIR, "boot-epoch")
os.makedirs(STATE)
os.makedirs(LANE_DIR)
os.environ["CYS_STATE_DIR"] = STATE
os.environ.setdefault("CYS_PACK_DIR", os.path.join(_ROOT, "pack"))
os.environ["HOME"] = os.path.join(_ROOT, "home")       # 부서 소켓 glob(~/.local/state/cys-dept-*)이 라이브를 안 보게
os.makedirs(os.environ["HOME"])
os.environ.pop("CYS_SOCKET", None)
os.environ.pop("CYS_GATE_RATE", None)                    # rate 축 opt-in 이면 `cys usage-accounts` 를 스폰한다
os.environ["CYS_GATE_LANE_SOCKET"] = LANE_SOCK           # 레인 = 이 임시 디렉터리(boot-epoch 도 여기)
sys.path.insert(0, BIN)

import javis_resource_gate as G      # noqa: E402

fails = []
seen_reasons = []                    # ⑨ 닫힌 집합 검사용 — 이 검체가 관측한 모든 reason
ROSTER0 = {"active": 0, "seats": 0, "errors": [], "depts": []}


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def write_epoch(nonce, mtime=None):
    """데몬이 내리는 형상 그대로(u64 십진 + 개행). mtime 을 주면 그 시각으로 되돌린다."""
    with open(EPOCH, "w", encoding="utf-8") as f:
        f.write("%s\n" % nonce)
    if mtime is not None:
        os.utime(EPOCH, (mtime, mtime))


def reset_state():
    shutil.rmtree(os.path.join(STATE, "resource-gate"), ignore_errors=True)


def args(started_at=None, roster=None, **over):
    """cmd_check 의 Namespace 대역 — 형제 test_resource_gate.make_args 와 같은 밀폐(ps·원장·래치 무접촉).
    started_at 을 주면 로스터 override 에 안 A 응답 대역 `{LANE_SOCK: epoch}` 을 싣는다."""
    ro = dict(ROSTER0 if roster is None else roster)
    if started_at is not None:
        ro["started_at"] = {LANE_SOCK: started_at}
    a = types.SimpleNamespace(
        servers_override=0, nodes_override=0, load_override=0.0, context=None,
        nodes_soft=12, nodes_hard=G.NODES_HARD_DEFAULT, servers_soft=2, servers_hard=4,
        load_soft_ratio=1.0, load_hard_ratio=None, context_soft=50, context_hard=60,
        dept_roster_override=ro,
        servers_ledger_override={"lane": "(ledger empty)", "depts": {}},
        fleet_cpu_override=1.5, fleet_cpu_hold_override=None,      # 값 주입 = 래치 무접촉
        fleet_cpu_soft=G.FLEET_CPU_SOFT_DEFAULT, fleet_cpu_hard=G.FLEET_CPU_HARD_DEFAULT,
        boot_elapsed_override=None,
    )
    for k, v in over.items():
        setattr(a, k, v)
    return a


def measure(**kw):
    m = G.measure(args(**kw))
    seen_reasons.append(m.get("boot_grace_reason"))
    return m


def blocked_state():
    """안 B 를 **불능**으로 만드는 상태 루트 — 일반 파일 아래의 경로(makedirs·open 이 ENOTDIR)."""
    blocker = os.path.join(_ROOT, "blocker.file")
    if not os.path.exists(blocker):
        open(blocker, "w").close()
    return os.path.join(blocker, "state")


def gate_cli(extra, env_over=None):
    """게이트 종단 호출 → (rc, json). 계약 채널은 stdout 하나다."""
    env = dict(os.environ)
    env.update(env_over or {})
    argv = [sys.executable, GATE, "check", "--json", "--servers-override", "0",
            "--nodes-override", "0", "--load-override", "0.0"] + list(extra)
    r = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=60, env=env)
    try:
        return r.returncode, json.loads(r.stdout.strip())
    except ValueError as e:
        fails.append("gate JSON 파싱 실패(%r): %s / %s" % (extra, e, r.stderr[-300:]))
        return r.returncode, {}


try:
    # ── ① ★발동 조건 ⓐ 재현: 복사·동기화가 mtime 만 현재로 민 형상 ──
    now = time.time()
    reset_state()
    write_epoch("12345", mtime=now - 10_000)           # 실제로는 약 3시간 전에 부팅했다
    m0 = measure(started_at=now - 10_000)              # 데몬도 그렇게 말한다(안 A)
    check("①-0 준비: 3시간 전 부팅 → 유예 없음(안 A)",
          m0["boot_grace"] is False and m0["boot_grace_reason"] == "daemon_started_at"
          and 9_990 <= (m0["boot_elapsed"] or 0) <= 10_100, repr(m0.get("boot_grace_reason")))
    os.utime(EPOCH, (now, now))                        # 백업 복원·rsync 가 mtime 만 현재로 갱신
    check("①-준비 검증: mtime 은 실제로 현재다",
          abs(os.path.getmtime(EPOCH) - now) < 2)
    m1 = measure(started_at=now - 10_000)
    check("① 내용 앵커(안 A)를 쓰면 mtime 이 밀려도 유예를 주지 않는다",
          m1["boot_grace"] is False and m1["boot_grace_reason"] == "daemon_started_at")
    m2 = measure()                                     # 데몬 무응답 → 안 B(같은 nonce · first_seen 유지)
    check("① 데몬 무응답이어도 nonce 세대가 같으면 유예를 다시 열지 않는다(안 B)",
          m2["boot_grace"] is False and m2["boot_grace_reason"] == "nonce"
          and 9_990 <= (m2["boot_elapsed"] or 0) <= 10_100,
          "%r/%r" % (m2.get("boot_grace_reason"), m2.get("boot_elapsed")))
    check("① 무엇을 근거로 썼는지 남는다",
          m1["boot_grace_reason"] in ("daemon_started_at", "nonce")
          and m2["boot_grace_reason"] in ("daemon_started_at", "nonce"))
    # 대조군 — **같은 파일 상태**에서 안 B 를 불능으로 만들면 종전 판정(mtime)이 재현된다: 유예 과다.
    #   이것이 이 검체가 재는 결함이다 — 이제 그 판정은 `mtime_fallback` 으로 **표기되어서만** 난다.
    os.environ["CYS_STATE_DIR"] = blocked_state()
    try:
        m3 = measure()
    finally:
        os.environ["CYS_STATE_DIR"] = STATE
    check("① 대조: 종전 판정(mtime 단독)은 같은 파일에서 유예 과다를 냈다 — 폴백에서만 · 표기된 채",
          m3["boot_grace"] is True and m3["boot_grace_reason"] == "mtime_fallback",
          "%r/%r" % (m3.get("boot_grace"), m3.get("boot_grace_reason")))

    # ── ② 감독자 off 로 실제 재부팅 → epoch 파일 부재/낡음 → 종전엔 유예 부족 ──
    reset_state()
    now = time.time()
    if os.path.exists(EPOCH):
        os.remove(EPOCH)
    m = measure(started_at=now - 10)
    check("② 파일이 없어도 데몬이 방금 떴다고 말하면 유예가 선다(안 A)",
          m["boot_grace"] is True and m["boot_grace_reason"] == "daemon_started_at")
    write_epoch("777", mtime=now - 10_000)             # 낡은 파일(cp -p 보존)이 남아 있는 형상
    m = measure(started_at=now - 10)
    check("② 낡은 epoch 파일이 남아 있어도 데몬의 시각이 이긴다(유예 부족 수리)",
          m["boot_grace"] is True and m["boot_grace_reason"] == "daemon_started_at")
    m = measure()                                      # 데몬 무응답 — 안 A 가 앞당겨 둔 first_seen 으로
    check("② 안 A 의 시각이 안 B 상태에 앞당겨져 데몬 무응답 뒤에도 같은 판정이 난다",
          m["boot_grace"] is True and m["boot_grace_reason"] == "nonce"
          and 9 <= (m["boot_elapsed"] or -1) <= 60, "%r/%r" % (m.get("boot_grace_reason"), m.get("boot_elapsed")))

    # ── ③ 폴백을 썼으면 그 사실이 산출물에 남는다(조용한 완화 금지) ──
    reset_state()
    now = time.time()
    write_epoch("4242", mtime=now - 10_000)
    os.environ["CYS_STATE_DIR"] = blocked_state()      # 상태 I/O 불능 → 안 B 불능
    try:
        m_old = measure()
        os.utime(EPOCH, (now, now))
        m_new = measure()
    finally:
        os.environ["CYS_STATE_DIR"] = STATE
    check("③ 상태 I/O 불능이면 mtime 폴백이 표기된다(옛 mtime → 유예 없음)",
          m_old["boot_grace"] is False and m_old["boot_grace_reason"] == "mtime_fallback")
    check("③ 폴백의 방향은 종전과 같다(mtime 현재 → 유예) — 그래서 표기가 계약이다",
          m_new["boot_grace"] is True and m_new["boot_grace_reason"] == "mtime_fallback")
    with open(EPOCH, "w", encoding="utf-8"):
        pass                                           # 0바이트(persist_epoch 가 막는 형상의 잔재)
    os.utime(EPOCH, (now - 10_000, now - 10_000))
    m = measure()
    check("③ 내용이 nonce 가 아니면(0바이트) 안 B 불능 → mtime 폴백 표기",
          m["boot_grace"] is False and m["boot_grace_reason"] == "mtime_fallback")
    check("③ 폴백은 mtime 값을 그대로 쓴다(경과 ≈ 10000)",
          9_990 <= (m["boot_elapsed"] or 0) <= 10_100, repr(m.get("boot_elapsed")))

    # ── ④ 음성 대조 — 근거가 전혀 없으면 유예 없음(종전 판정 그대로) ──
    reset_state()
    now = time.time()
    os.remove(EPOCH)
    m = measure()
    check("④ 근거 없음 = 유예 없음(epoch_missing)",
          m["boot_grace"] is False and m["boot_elapsed"] is None
          and m["boot_grace_reason"] == "epoch_missing")
    os.makedirs(EPOCH)                                 # 파일 자리에 디렉터리 — 판독 불능
    try:
        m = measure()
    finally:
        os.rmdir(EPOCH)
    check("④ 판독 불능 = 유예 없음(epoch_unreadable)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "epoch_unreadable")
    m = measure(started_at=now + 3600)
    check("④ 미래 started_at(시계 역행) = 유예 없음(clock_backwards)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "clock_backwards")
    m = measure(started_at="not-a-number")
    check("④ 숫자가 아닌 started_at 은 근거가 아니다 → 안 A 무시(파일 부재라 epoch_missing)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "epoch_missing")

    # ── ⑤ 세대 교체 — 내용 앵커는 양방향으로 이긴다 ──
    reset_state()
    now = time.time()
    write_epoch("111", mtime=now - 10_000)             # 게이트가 처음 보는 세대 + 옛 mtime(백업 복원)
    m = measure()
    check("⑤ 새 nonce + 옛 mtime(백업 복원) → first_seen=min(now, mtime) → 유예 없음",
          m["boot_grace"] is False and m["boot_grace_reason"] == "nonce"
          and 9_990 <= (m["boot_elapsed"] or 0) <= 10_100, "%r/%r" % (m.get("boot_grace_reason"), m.get("boot_elapsed")))
    write_epoch("222")                                 # 실제 재부팅: 새 nonce · 갓 쓴 파일
    m = measure()
    check("⑤ 새 nonce + 갓 쓴 파일(실제 재부팅) → 유예",
          m["boot_grace"] is True and m["boot_grace_reason"] == "nonce")
    os.utime(EPOCH, (now - 10_000, now - 10_000))       # cp -p 가 같은 내용에 옛 mtime 을 얹음
    m = measure()
    check("⑤ 같은 nonce 에 옛 mtime 이 와도 유예는 유지(종전엔 유예 부족)",
          m["boot_grace"] is True and m["boot_grace_reason"] == "nonce")
    rec = json.load(open(G._boot_nonce_path(), encoding="utf-8"))
    check("⑤ 게이트 상태 파일 모양 {boot_nonce, first_seen}",
          rec.get("boot_nonce") == "222" and isinstance(rec.get("first_seen"), float), repr(rec))
    # ★성찰2(리뷰 2/3) — 새 nonce + **미래 mtime**(시계 역행 · 허용 오차 1s 초과): 종전 `min(now, mtime)`
    #   은 now 를 골라 경과 0 = '갓 부팅' 으로 유예를 열었다(fdd45f3 의 clock_backwards 에서 fail-open
    #   회귀). 근거 실패 = 유예 없음 · 상태 미기록(다음 호출도 안 열린다) · mtime 이 정상으로 돌아오면 그때 관측.
    write_epoch("444", mtime=now + 3600)
    m = measure()
    check("⑤ 새 nonce + 미래 mtime(시계 역행) → 유예 없음(clock_backwards) — 종전 min(now, mtime) 의 fail-open 수리",
          m["boot_grace"] is False and m["boot_elapsed"] is None
          and m["boot_grace_reason"] == "clock_backwards",
          "%r/%r" % (m.get("boot_grace_reason"), m.get("boot_elapsed")))
    rec = json.load(open(G._boot_nonce_path(), encoding="utf-8"))
    m = measure()
    check("⑤ 미래 mtime 세대는 상태에 남지 않아(직전 세대 222 유지) 다음 호출에도 유예가 안 열린다",
          rec.get("boot_nonce") == "222" and m["boot_grace"] is False
          and m["boot_grace_reason"] == "clock_backwards", "%r/%r" % (rec, m.get("boot_grace_reason")))
    os.utime(EPOCH, (now - 10_000, now - 10_000))       # mtime 이 과거로 돌아오면 그때가 첫 관측이다
    m = measure()
    check("⑤ mtime 이 정상(과거)으로 돌아오면 first_seen=min(now, mtime) 으로 관측(유예 없음 · nonce)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "nonce"
          and 9_990 <= (m["boot_elapsed"] or 0) <= 10_100, "%r/%r" % (m.get("boot_grace_reason"), m.get("boot_elapsed")))

    # ── ⑥ 문서화된 한계(fail-open 성분) — 첫 관측이 늦으면 유예가 선다 ──
    reset_state()                                      # 게이트가 이 세대를 본 적이 없다
    now = time.time()
    write_epoch("333", mtime=now - 10_000)
    os.utime(EPOCH, (now, now))                        # 동기화가 mtime 을 현재로
    m = measure()                                      # 데몬 무응답
    check("⑥ 한계 핀: 첫 관측+밀린 mtime 은 유예를 준다(안 B 의 fail-open 성분 — 그래서 안 A 가 1차)",
          m["boot_grace"] is True and m["boot_grace_reason"] == "nonce")
    m = measure(started_at=now - 10_000)               # 안 A 가 오면 즉시 닫힌다
    check("⑥ 안 A 가 오면 그 창은 닫힌다(first_seen 앞당김)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "daemon_started_at")
    m = measure()
    check("⑥ 닫힌 뒤엔 데몬 무응답이어도 다시 안 열린다",
          m["boot_grace"] is False and m["boot_grace_reason"] == "nonce")

    # ── ⑦ 종단 --json 방출 — measured/checks/exit 계약 ──
    reset_state()
    now = time.time()
    write_epoch("555", mtime=now - 10_000)
    ro_new = dict(ROSTER0, started_at={LANE_SOCK: now - 10})
    rc, doc = gate_cli(["--fleet-cpu-override", "1.5", "--dept-roster-override", json.dumps(ro_new)])
    mm = doc.get("measured") or {}
    ax = next((c for c in (doc.get("checks") or []) if c.get("metric") == "fleet_cpu_ratio"), {})
    check("⑦ started_at 주입(10s 전) → measured.boot_grace/boot_grace_reason 방출 · CPU 축 hard→soft · exit 1",
          rc == G.EXIT_SOFT and mm.get("boot_grace") is True
          and mm.get("boot_grace_reason") == "daemon_started_at" and ax.get("boot_grace") is True
          and ax.get("level") == "soft", "rc=%r measured=%r axis=%r" % (rc, {k: mm.get(k) for k in ("boot_grace", "boot_grace_reason", "boot_elapsed")}, ax))
    seen_reasons.append(mm.get("boot_grace_reason"))
    ro_old = dict(ROSTER0, started_at={LANE_SOCK: now - 10_000})
    rc, doc = gate_cli(["--fleet-cpu-override", "1.5", "--dept-roster-override", json.dumps(ro_old)])
    mm = doc.get("measured") or {}
    check("⑦ started_at 주입(3h 전) → 유예 없음 · exit 2",
          rc == G.EXIT_HARD and mm.get("boot_grace") is False
          and mm.get("boot_grace_reason") == "daemon_started_at")
    seen_reasons.append(mm.get("boot_grace_reason"))
    os.remove(EPOCH)
    rc, doc = gate_cli(["--fleet-cpu-override", "1.5", "--dept-roster-override", json.dumps(ROSTER0)])
    mm = doc.get("measured") or {}
    check("⑦ 근거 없음 → epoch_missing · exit 2",
          rc == G.EXIT_HARD and mm.get("boot_grace") is False
          and mm.get("boot_grace_reason") == "epoch_missing")
    seen_reasons.append(mm.get("boot_grace_reason"))

    # ── ⑧ ★왕복 추가 0 — 가짜 cys 스텁 + 실소켓으로 status 경로를 태운다 ──
    if not hasattr(socket, "AF_UNIX"):
        print("SKIP ⑧ AF_UNIX 없음(Windows) — 부서 소켓은 named pipe 라 이 재현의 대상이 아니다")
    else:
        reset_state()
        now = time.time()
        dept_dir = os.path.join(os.environ["HOME"], ".local", "state", "cys-dept-t")
        os.makedirs(dept_dir)
        dept_sock = os.path.join(dept_dir, "cys.sock")
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(dept_sock)
        srv.listen(1)                                  # 리스너 프로브(_socket_listening)를 통과시킨다
        fakebin = os.path.join(_ROOT, "fakebin")
        os.makedirs(fakebin)
        log = os.path.join(_ROOT, "fake-cys.log")
        stub = os.path.join(fakebin, "cys")
        with open(stub, "w", encoding="utf-8") as f:
            f.write("#!/bin/sh\n"
                    "# 가짜 cys — 실제 바이너리가 아니다. argv 를 기록하고 status 응답 하나를 낸다.\n"
                    "printf '%s\\n' \"$*\" >> \"$FAKE_CYS_LOG\"\n"
                    "printf '{\"daemon\":{\"version\":\"t\",\"started_at\":%s},"
                    "\"surfaces\":[{\"exited\":false},{\"exited\":true}]}\\n' \"$FAKE_STARTED_AT\"\n")
        os.chmod(stub, os.stat(stub).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        saved = {k: os.environ.get(k) for k in ("PATH", "CYS_GATE_LANE_SOCKET", "FAKE_CYS_LOG", "FAKE_STARTED_AT")}
        os.environ["PATH"] = fakebin + os.pathsep + os.environ.get("PATH", "")
        os.environ["CYS_GATE_LANE_SOCKET"] = dept_sock     # 이 레인 = 그 부서(boot-epoch 도 부서 디렉터리)
        os.environ["FAKE_CYS_LOG"] = log
        os.environ["FAKE_STARTED_AT"] = repr(now - 10_000)
        try:
            with open(os.path.join(dept_dir, "boot-epoch"), "w", encoding="utf-8") as f:
                f.write("9001\n")
            os.utime(os.path.join(dept_dir, "boot-epoch"), (now, now))   # mtime 은 밀려 있다
            m = G.measure(args(roster=None, dept_roster_override=None))  # 라이브 로스터 경로
            seen_reasons.append(m.get("boot_grace_reason"))
            calls = open(log, encoding="utf-8").read().splitlines() if os.path.exists(log) else []
        finally:
            srv.close()
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
        check("⑧ 실제 status 경로의 응답에서 started_at 이 재사용된다(부서 1·좌석 1 계상 동일)",
              m["boot_grace"] is False and m["boot_grace_reason"] == "daemon_started_at"
              and m["active_depts"] == 1 and m["dept_seats"] == 1,
              "%r/%r depts=%r errors=%r" % (m.get("boot_grace_reason"), m.get("boot_elapsed"),
                                            m.get("depts"), m.get("measure_errors")))
        check("⑧ 스폰 횟수 = 소켓 수(1) — 부트 앵커를 위한 새 왕복이 없다",
              len(calls) == 1 and calls[0].startswith("status --json --socket "), repr(calls))

    # ── ⑨ 계약 불변 ──
    reset_state()
    write_epoch("1", mtime=time.time() - 5)
    m = measure(boot_elapsed_override=99999.0)
    check("⑨ boot_elapsed_override 가 여전히 이긴다(파일·상태 무접촉)",
          m["boot_grace"] is False and m["boot_grace_reason"] == "override"
          and not os.path.exists(G._boot_nonce_path()))
    check("⑨ measured 키 3종(boot_grace/boot_elapsed/boot_grace_reason) 존재",
          all(k in m for k in ("boot_grace", "boot_elapsed", "boot_grace_reason")))
    bad = [r for r in seen_reasons if r not in G.BOOT_GRACE_REASONS]
    check("⑨ 관측된 reason 전부 닫힌 집합 안(%d건)" % len(seen_reasons), not bad, repr(bad))
    sink = {}
    r = G._dept_roster(dict(ROSTER0, started_at={LANE_SOCK: 1.5, "/x": True, "/y": float("nan")}),
                       status_sink=sink)
    check("⑨ 로스터 반환 모양 불변(4키) · 싱크는 유한 숫자만 받는다(bool·nan 거부)",
          set(r) == {"active", "seats", "errors", "depts"}
          and sink == {G._sock_key(LANE_SOCK): 1.5}, "%r %r" % (r, sink))
finally:
    shutil.rmtree(_ROOT, ignore_errors=True)

if fails:
    print("FAILED: %d — %s" % (len(fails), fails))
    sys.exit(1)
print("ALL PASS")
print("RESOURCE-GATE-BOOT-ANCHOR-OK")
