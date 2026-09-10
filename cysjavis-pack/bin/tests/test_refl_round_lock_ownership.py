#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_round_lock_ownership.py — 성찰 R4 N6·N13·N17: 잠금 소유권 게시·반납·복제 패리티.

무엇을 막는가
  ① N6 — 빈 잠금 회수가 **살아 있는 소유자의 잠금**을 제거했다. 사슬: A 가 `os.mkdir` 성공 뒤
     owner 를 쓰기 **전에** 상한을 넘겨 지연 → B 가 '빈 + 나이 초과' 로 회수·재획득·게시 →
     A 가 재개해 **B 의 잠금 안에** 자기 owner 를 덮어쓰고 되읽기까지 성공 → 둘이 동시에
     임계구간에 들고 A 의 반납이 B 의 잠금을 지운다.
     수리: 소유권 게시를 `O_EXCL` 로 원자화(성공한 하나만 소유자) · 되읽기가 **남의 토큰**이면
     그것도 소유권 상실 · 반납은 경로 unlink 가 아니라 **원자 청구 후 바이트 확인** · 세대
     `(st_dev, st_ino)` 는 삭제의 **보조 거부**로만(신원 증명으로 쓰지 않는다).
  ② N13 — 잠금 프로토콜 ≈110행이 `javis_orchestra` ↔ `javis_rsi` 에 바이트 동형 복제인데
     패리티 장치가 없었다(행동 검체도 orchestra 판만). AST 해시 대조 + 두 판본 각각의 행동 검체.
  ③ N17 — `<장부>.lock` 자리의 **일반 파일**·**미지 내용물**은 어느 회수 경로도 손대지 않는데
     안내는 "300초 뒤 자동 회수" 라고 말했다(거짓 · 조작자는 영원히 기다린다). 문면을 가른다.

배리어: 잠자기로 순서를 가정하지 않는다 — 자식(A)이 `os.mkdir` **직후** 표식 파일을 만들고
멈추면, 부모(B)가 그것을 보고 나이를 조작(`os.utime`)한 뒤 잠금을 쥐고, 그때서야 A 를 깨운다.
타임아웃은 **중단용**이지 순서의 증거가 아니다.

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-LOCK-OWNERSHIP-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_round_lock_ownership.py
"""
import ast
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
sys.path.insert(0, BIN)
PY = sys.executable
BARRIER_TIMEOUT = 60.0

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def wait_for(path, what):
    """배리어 — 표식이 설 때까지 기다린다. 타임아웃은 **중단**이지 순서 증거가 아니다."""
    t0 = time.monotonic()
    while not os.path.exists(path):
        if time.monotonic() - t0 > BARRIER_TIMEOUT:
            raise RuntimeError("배리어 시간초과: %s" % what)
        time.sleep(0.01)


import javis_orchestra as ORC          # noqa: E402
import javis_rsi as RSI                # noqa: E402

MODS = (("javis_orchestra", ORC), ("javis_rsi", RSI))

# ── ② N13: 두 판본 동형 패리티 ───────────────────────────────────────────────
def class_node(path, name="_best_effort_lock"):
    tree = ast.parse(open(path, encoding="utf-8").read())
    for n in tree.body:
        if isinstance(n, ast.ClassDef) and n.name == name:
            return n
    return None


_orc_cls = class_node(os.path.join(BIN, "javis_orchestra.py"))
_rsi_cls = class_node(os.path.join(BIN, "javis_rsi.py"))
check("2a 두 모듈 모두 잠금 클래스를 갖는다", _orc_cls is not None and _rsi_cls is not None)
if _orc_cls is not None and _rsi_cls is not None:
    # ★위치 정보만 지운 AST 덤프를 해시로 대조한다(codex (e)): 문자열·상수·조건식은 정규화로
    #   지우지 않는다 — 그것들이 갈리는 것이 바로 이 패리티가 잡아야 하는 사고다.
    ha = hashlib.sha256(ast.dump(_orc_cls).encode("utf-8")).hexdigest()
    hb = hashlib.sha256(ast.dump(_rsi_cls).encode("utf-8")).hexdigest()
    check("2b 잠금 클래스 AST 해시 동형", ha == hb, "%s vs %s" % (ha[:16], hb[:16]))
    ma = sorted(n.name for n in _orc_cls.body if isinstance(n, ast.FunctionDef))
    mb = sorted(n.name for n in _rsi_cls.body if isinstance(n, ast.FunctionDef))
    check("2c 메서드 집합 동형", ma == mb, repr((ma, mb)))
    check("2d 신설 규율이 두 판본에 다 있다",
          {"_publish_owner", "_gen_ok", "_stat_gen", "_release", "_claim_and_unlink",
           "obstruction"} <= set(ma), repr(ma))
# 기본 인자는 **이름**만 같아선 안 된다 — 해소된 값도 같아야 한다(codex: 외부 기본값 따로 비교).
check("2e 대기·상한 상수 값 동형",
      (ORC.WP6_LOCK_WAIT, ORC.WP6_LOCK_STALE) == (RSI.WP6_LOCK_WAIT, RSI.WP6_LOCK_STALE),
      repr(((ORC.WP6_LOCK_WAIT, ORC.WP6_LOCK_STALE), (RSI.WP6_LOCK_WAIT, RSI.WP6_LOCK_STALE))))


# ── ① N6: 배리어 2프로세스 — 빈 잠금 회수가 살아 있는 소유자를 만들지 않는다 ──
CHILD = r'''
import json, os, sys, time
BIN, base, sig, resume, out, modname = sys.argv[1:7]
sys.path.insert(0, BIN)
mod = __import__(modname)
lock = base + ".lock"
real_mkdir = os.mkdir

def barrier_mkdir(p, *a, **kw):
    r = real_mkdir(p, *a, **kw)
    if str(p) == lock:
        # ★정지점: mkdir 은 **성공**했고 소유권 게시는 아직이다(N6 의 그 순간).
        open(sig, "w").close()
        t0 = time.monotonic()
        while not os.path.exists(resume):
            if time.monotonic() - t0 > 60:
                raise SystemExit("child barrier timeout")
            time.sleep(0.01)
    return r

os.mkdir = barrier_mkdir
res = {}
with mod._best_effort_lock(base, wait=0.5, stale=300.0) as lk:
    res = {"held": bool(lk.held), "blocked": bool(lk.blocked),
           "owner_state": lk.owner_state, "wrote_owner": bool(lk.wrote_owner)}
    if lk.held:                       # 임계구간 진입의 **관측 가능한 흔적**
        with open(base, "a", encoding="utf-8") as f:
            f.write("A\n")
# ★A2(2026-09-11 · 부하 아래 실측 적색 1회): 부모의 배리어는 **파일 존재**를 기다린다 —
#   `open(out,"w")` 는 내용을 쓰기 **전에** 빈 파일을 만들므로, 느린 기계에서 부모가 그 틈을 보고
#   빈 문자열을 json.loads 해 `JSONDecodeError` 로 죽었다(순서 경합 · 제품 결함 아님).
#   원자 교환으로 닫는다: 존재 = 완결이다(대기 상한을 늘려서는 못 고치는 자리).
open(out + ".tmp", "w", encoding="utf-8").write(json.dumps(res))
os.replace(out + ".tmp", out)
'''

for modname, mod in MODS:
    root = tempfile.mkdtemp()
    try:
        base = os.path.join(root, "LEDGER.md")
        lock = base + ".lock"
        sig = os.path.join(root, "sig")
        resume = os.path.join(root, "resume")
        outp = os.path.join(root, "out.json")
        script = os.path.join(root, "child.py")
        open(script, "w", encoding="utf-8").write(CHILD)
        child = subprocess.Popen([PY, script, BIN, base, sig, resume, outp, modname])
        try:
            wait_for(sig, "A 의 mkdir 완료(%s)" % modname)
            # A 는 게시 전에 멈춰 있다 — 그 사이 잠금은 상한을 넘긴 **빈** 잠금이 된다.
            old = time.time() - 4000
            os.utime(lock, (old, old))
            with mod._best_effort_lock(base, wait=5.0, stale=300.0) as b:
                check("1a[%s] B 가 빈 고아 잠금을 회수·획득" % modname, b.held and not b.blocked,
                      "held=%s blocked=%s state=%r" % (b.held, b.blocked, b.owner_state))
                check("1b[%s] B 의 소유권이 게시됐다" % modname, b.owner_state == "published")
                open(resume, "w").close()            # 이제 A 를 깨운다(B 는 계속 쥔 채)
                wait_for(outp, "A 의 종료(%s)" % modname)
                res = json.loads(open(outp, encoding="utf-8").read())
                owner_mid = b._owner()
                check("1c[%s] A 는 임계구간에 들지 않는다" % modname,
                      not res["held"], repr(res))
                check("1d[%s] A 는 소유권 상실을 안다(owner_state=lost)" % modname,
                      res["owner_state"] == "lost", repr(res))
                check("1e[%s] A 가 B 의 owner 를 덮어쓰지 않았다" % modname,
                      owner_mid == b.token, "%r vs %r" % (owner_mid, b.token))
            check("1f[%s] B 의 반납 후 잠금이 정상 해제" % modname, not os.path.exists(lock))
            body = open(base, encoding="utf-8").read() if os.path.isfile(base) else ""
            check("1g[%s] 동시 기록 0(A 의 흔적 없음)" % modname, "A\n" not in body, repr(body))
        finally:
            open(resume, "w").close()                # 실패 경로에서도 자식을 풀어 준다
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
    finally:
        shutil.rmtree(root, ignore_errors=True)


# ── ① N6 잔여 반례(codex): 배타 생성 **직후·쓰기 전**에 회수당하면 되읽기가 남의 토큰이다 ──
for modname, mod in MODS:
    root = tempfile.mkdtemp()
    try:
        base = os.path.join(root, "L2.md")
        lock = base + ".lock"
        real_write, done = os.write, []

        def racing_write(fd, data, *a, **kw):
            if not done and data == getattr(racing_write, "tok", None):
                done.append(True)
                # B 가 빈 owner 를 회수하고 재획득·게시한다(A 의 fd 는 이미 unlink 된 파일).
                shutil.rmtree(lock, ignore_errors=True)
                os.mkdir(lock)
                with open(os.path.join(lock, "owner"), "wb") as f:
                    f.write(b"B-LIVE-OWNER")
            return real_write(fd, data, *a, **kw)

        lk = mod._best_effort_lock(base, wait=0.3, stale=300.0)
        racing_write.tok = lk.token
        os.write = racing_write
        try:
            with lk:
                held = bool(lk.held)
                state = lk.owner_state
        finally:
            os.write = real_write
        check("1h[%s] 전제 성립(게시 도중 회수 재현)" % modname, bool(done))
        check("1i[%s] 게시 도중 회수 → 진입 거부" % modname, not held,
              "held=%s state=%r" % (held, state))
        check("1j[%s] 상실을 lost 로 가른다" % modname, state == "lost", repr(state))
        try:
            with open(os.path.join(lock, "owner"), "rb") as f:
                survived = f.read()
        except OSError as e:
            survived = b"<%s>" % str(e).encode()
        check("1k[%s] B 의 owner 가 살아남았다" % modname, survived == b"B-LIVE-OWNER",
              repr(survived))
    finally:
        shutil.rmtree(root, ignore_errors=True)


# ── ① N6 반납 TOCTOU: 확인 뒤 소유자가 바뀌어도 남의 잠금을 지우지 않는다 ──
for modname, mod in MODS:
    root = tempfile.mkdtemp()
    try:
        base = os.path.join(root, "L3.md")
        lock = base + ".lock"
        os.mkdir(lock)
        with open(os.path.join(lock, "owner"), "wb") as f:
            f.write(b"B-NEW-OWNER")
        stale_lk = mod._best_effort_lock(base)      # 고아로 오인돼 회수된 옛 소유자
        stale_lk.held, stale_lk.wrote_owner, stale_lk.owner_state = True, True, "published"
        stale_lk.gen = None                          # 세대를 못 얻은 형상(가장 불리한 쪽)
        stale_lk.__exit__()
        try:
            with open(os.path.join(lock, "owner"), "rb") as f:
                after = f.read()
        except OSError as e:
            after = b"<%s>" % str(e).encode()
        check("1l[%s] 옛 소유자의 반납이 새 소유자 owner 를 지우지 않는다" % modname,
              after == b"B-NEW-OWNER", repr(after))
        check("1m[%s] 잠금 디렉터리도 남는다" % modname, os.path.isdir(lock))

        # 음성 대조 — **내** 잠금은 정상 반납된다(과보호로 잠금을 남기면 300초 교착이다).
        base2 = os.path.join(root, "L4.md")
        with mod._best_effort_lock(base2) as ok_lk:
            assert ok_lk.held
        check("1n[%s] 정상 소유자는 반납된다(음성 대조)" % modname,
              not os.path.exists(base2 + ".lock"))
    finally:
        shutil.rmtree(root, ignore_errors=True)


# ── ③ N17: 회수 불가 형상은 안내 문면이 갈린다 ───────────────────────────────
root = tempfile.mkdtemp()
try:
    # (a) 잠금 자리에 **일반 파일**
    b1 = os.path.join(root, "T1.md")
    open(b1 + ".lock", "w").close()
    lk1 = ORC._best_effort_lock(b1, wait=0.1, stale=300.0)
    with lk1:
        pass
    check("3a 일반 파일 형상은 blocked", lk1.blocked)
    check("3b 일반 파일 형상 판별", lk1.obstruction() == "file", repr(lk1.obstruction()))
    m1 = ORC.lock_blocked_reason(lk1, "해악.")
    check("3c 일반 파일 안내에 '자동 회수' 약속 없음", "자동 회수되지 않는다" in m1, m1)

    # (b) 잠금 디렉터리에 **미지 내용물**
    b2 = os.path.join(root, "T2.md")
    os.mkdir(b2 + ".lock")
    open(os.path.join(b2 + ".lock", "junk"), "w").close()
    old = time.time() - 4000
    os.utime(b2 + ".lock", (old, old))
    lk2 = ORC._best_effort_lock(b2, wait=0.1, stale=300.0)
    with lk2:
        pass
    check("3d 미지 내용물 형상은 blocked", lk2.blocked)
    check("3e 미지 내용물 형상 판별", lk2.obstruction() == "contents", repr(lk2.obstruction()))
    m2 = ORC.lock_blocked_reason(lk2, "해악.")
    check("3f 미지 내용물 안내에 '자동 회수' 약속 없음", "자동 회수되지 않는다" in m2, m2)
    check("3g 두 문면이 서로 다르다", m1 != m2)

    # (c) ★음성 대조 — 살아 있는 소유자(회수 가능 형상)에는 종전 안내가 그대로 나온다
    b3 = os.path.join(root, "T3.md")
    os.mkdir(b3 + ".lock")
    with open(os.path.join(b3 + ".lock", "owner"), "wb") as f:
        f.write(b"live-owner")
    lk3 = ORC._best_effort_lock(b3, wait=0.1, stale=300.0)
    with lk3:
        pass
    check("3h 살아 있는 소유자는 회수 가능 형상", lk3.obstruction() == "")
    m3 = ORC.lock_blocked_reason(lk3, "해악.")
    check("3i 회수 가능 형상 안내는 종전대로 '자동 회수'", "초 뒤 자동 회수" in m3, m3)

    # (d) 실제 명령 문면 — round-log 가 그 문면을 그대로 낸다(일반 파일 자리)
    pack = os.path.join(root, "pack")
    os.makedirs(os.path.join(pack, "round"))
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = pack
    for k in ("CYS_ROUND_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        env.pop(k, None)
    _saved_pack = os.environ.get("CYS_PACK_DIR")
    os.environ["CYS_PACK_DIR"] = pack
    try:
        ledger = ORC.round_path("REFL-N17")
    finally:
        if _saved_pack is None:
            os.environ.pop("CYS_PACK_DIR", None)
        else:
            os.environ["CYS_PACK_DIR"] = _saved_pack
    open(ledger + ".lock", "w").close()          # 잠금 자리의 **일반 파일**(실측 재현 형상)
    r = subprocess.run([PY, os.path.join(BIN, "javis_orchestra.py"), "round-log",
                        "--task", "REFL-N17", "--round", "1",
                        "--evaluator", "worker", "--verdict", "ACCEPT"],
                       capture_output=True, text=True, env=env)
    blob = (r.stdout or "") + (r.stderr or "")
    check("3j round-log 가 갈린 문면을 낸다", "자동 회수되지 않는다" in blob, blob[-400:])
    check("3k round-log 는 rc≠0(기록 거부)", r.returncode != 0, str(r.returncode))
    check("3l round-log 가 거짓 약속을 하지 않는다", "초 뒤 자동 회수" not in blob, blob[-300:])
finally:
    shutil.rmtree(root, ignore_errors=True)

print("REFL-LOCK-OWNERSHIP-OK" if not fails else "FAILED: %s" % ", ".join(fails))
sys.exit(1 if fails else 0)
