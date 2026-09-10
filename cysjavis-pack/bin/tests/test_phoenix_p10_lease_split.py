#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_phoenix_p10_lease_split.py — 성찰 반영(2026-09-10) P10 의 회귀 핀.

P10 [major] F-1 이 restore 1회의 lease 보유 시간을 크게 늘렸는데(`F1_REGISTERED_GRACE_TRIES=6` ×
`PHOENIX_SESSION_GRACE_SLEEP=1.5` × `F1_REVERIFY_PASSES=2` × 역할 수 — 8역할이면 sleep 만 120초)
`src/bin/cysd/reclaim.rs::decide` 는 그 lease 를 후보 계산 **앞에서** 하드 `Defer("restore_lease_held")`
로 본다. 그 창에서 시작한 좌석은 역할 결합 0 = `external:N`(감사 §2 에러 2 의 결과).

수정의 축: 락을 **두 겹**으로 가른다.
  · `restore.run.lock`(신설) = restore ↔ restore 전 구간 배타 — 이중 스폰과 **후행 쓰기 경합**을 막는다.
  · `restore.lease`(기존 · cysd 가독) = 스폰 구간만 — ④ 역할별 하위 단계(관측·재주입·재검증) 진입
    직전에 놓는다. 그 구간은 토폴로지를 만들지 않으므로 "부활 중엔 결정이 낡았다" 가 성립하지 않는다.

여기서 재는 것(라이브 무접촉 — 소켓·상태 dir·팩 전부 임시 디렉터리이고 CLI 는 스크립트다):
  ① 스폰 순간(`cys restore` 호출 중)에는 `restore.lease` 가 **잡혀 있다**(reclaim 이 Defer 하는 것이 옳은 구간).
  ② ④ 진입 뒤(`cys reinject --check` 호출 중)에는 `restore.lease` 가 **비어 있다** — reclaim 대역이 잡는다.
  ③ 같은 순간 `restore.run.lock` 은 **잡혀 있고**, 그 사이 들어온 다른 restore 는 `LEASE_HELD` 로 접힌다
     (이중 스폰·후행 쓰기 경합 0 · 저널 쓰기 0).
  ④ **음성 대조**: 조기 해제를 없앤 종전 형상에서는 ②가 실제로 '잡혀 있음' 으로 재현된다(핀이 죽지 않았다).
  ⑤ 조기 해제는 멱등이고, 해제 뒤에도 atexit 중복 정리가 예외를 내지 않는다.
  ⑥ 순서 계약: run.lock 획득이 lease 획득보다 앞이다(두 restore 가 lease 를 번갈아 쥐는 형상 0).

고지(이 수정이 닫지 못하는 것 · 검체에 남긴다): ②③ 스폰 도중에 **이미 Defer 로 끝난** SessionStart 훅은
조기 해제 뒤에도 스스로 재결합하지 않는다 — reclaim 쪽 유계 재시도(fix-plan P10 ⓑ · `reclaim.rs`)의 몫이다.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_p10_lease_split.py
"""
import copy
import importlib.util
import inspect
import json
import os
import shutil
import sys
import tempfile
from types import SimpleNamespace

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []


def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" — %s" % detail if detail and not cond else ""))


def _probe(path):
    """그 락 파일을 지금 잡을 수 있는가 → 'free' / 'held' / 'unavailable'.
    `reclaim.rs::try_hold_restore_lease` 와 같은 술어다(관측이 아니라 **잡아 본다**). flock/msvcrt 는
    같은 프로세스라도 **다른 fd** 면 경합하므로 이 프로브는 본체가 쥔 락을 정직하게 본다."""
    if not os.path.exists(path):
        return "free"
    try:
        f = open(path, "a+")
    except OSError:
        return "unavailable"
    try:
        r = m._try_lock_nb(f)
        if r is None:
            return "unavailable"
        return "free" if r else "held"
    finally:
        f.close()


def run_scenario(name, disable_early_release=False):
    """실 `run_restore`(production 경로)를 스크립트 CLI 로 돌리고 국면별 락 상태를 기록한다."""
    td = tempfile.mkdtemp(prefix="phoenix-p10-")
    orig = (m.cys, m.time.sleep, m._emit_evt, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
            m._release_shared_restore_lease, m._RESTORE_LEASE_HANDLE)
    try:
        socket_path = os.path.join(td, "cys.sock")
        ticket = "p10-" + name
        entry = {"role": "worker-1", "agent": "claude", "session_id": "",
                 "claude_config_dir": os.path.join(td, "acct"), "cwd": os.path.join(td, "proj"),
                 "surface_id": 7, "surface_ref": "surface:7"}
        os.makedirs(entry["cwd"])
        project_dir = os.path.join(entry["claude_config_dir"], "projects",
                                   m._claude_project_component(entry["cwd"]))
        os.makedirs(project_dir)

        def session_file(sid):
            with open(os.path.join(project_dir, sid + ".jsonl"), "w", encoding="utf-8") as f:
                f.write("{}\n")

        session_file("old")
        now = m._now()
        with open(os.path.join(td, "topology.json"), "w", encoding="utf-8") as f:
            json.dump({"entries": [entry], "updated_at": now, "schema_version": 1,
                       "tombstones": [], "tombstones_rev": 0}, f)
        home = m.phoenix_home(socket_path)
        lease_path = os.path.join(home, "restore.lease")
        run_path = os.path.join(home, "restore.run.lock")
        obs = {"spawn": None, "verify": None, "verify_run": None, "reentrant": None,
               "reentrant_journal_before": None, "reentrant_journal_after": None}
        state = {"alive": False}
        journal_path = m.journal_path(socket_path, ticket)

        def _journal_bytes():
            try:
                with open(journal_path, "rb") as f:
                    return f.read()
            except OSError:
                return None

        def fake_cys(*args, socket=None, timeout=25):
            assert socket == socket_path, "CLI escaped isolated socket"
            verb = args[0]
            stdout = ""
            if verb == "status":
                rows = []
                if state["alive"]:
                    rows = [{"surface_id": 7, "surface_ref": "surface:7", "role": "worker-1",
                             "exited": False, "agent_alive": True, "seat": "occupied",
                             "agent": "claude", "gate_pending": None,
                             "registered_session_id": "new1", "awakened_at": now}]
                stdout = json.dumps({"daemon": {"started_at": now}, "surfaces": rows})
            elif verb == "restore":
                # ① 스폰 구간 — 여기서는 lease 가 잡혀 있어야 한다(reclaim 의 Defer 가 옳은 구간).
                if obs["spawn"] is None:
                    obs["spawn"] = (_probe(lease_path), _probe(run_path))
                state["alive"] = True
                session_file("new1")
                stdout = "restored worker-1 surface:7"
            elif verb == "reinject":
                # ② ④ 구간 — lease 는 비고 run.lock 은 잡혀 있어야 한다.
                if obs["verify"] is None:
                    obs["verify"] = _probe(lease_path)
                    obs["verify_run"] = _probe(run_path)
                    # ③ 그 사이 들어온 다른 restore 는 접힌다 — 저널 바이트가 변하지 않는 것으로 '쓰기 0' 을 잰다.
                    obs["reentrant_journal_before"] = _journal_bytes()
                    obs["reentrant"] = m.run_restore(socket_path, ticket="p10-other-" + name,
                                                     stub=False, no_breaker=True, print_result=False)
                    obs["reentrant_journal_after"] = _journal_bytes()
                stdout = "디렉티브 생존 확인 (ACK 수신)"
            elif verb in ("read-screen", "watch"):
                stdout = "bypass permissions on"
            elif verb == "list":
                stdout = "surface:7 role=worker-1 pid=7 exited=false\n" if state["alive"] else ""
            else:
                raise AssertionError("unexpected cys call: %r" % (args,))
            return SimpleNamespace(returncode=0, stdout=stdout, stderr="")

        m.cys = fake_cys
        m.time.sleep = lambda seconds: None
        m._emit_evt = lambda *a, **k: None
        m._RESTORE_LEASE_HANDLE = None
        if disable_early_release:
            # 음성 대조: 조기 해제가 없던 종전 형상(= 결함)을 그대로 재현한다.
            m._release_shared_restore_lease = lambda why="": False
        result = m.run_restore(socket_path, ticket=ticket, stub=False, no_breaker=True, print_result=False)
        obs["result"] = result
        obs["lease_after"] = _probe(lease_path)
        return obs
    finally:
        (m.cys, m.time.sleep, m._emit_evt, m._ACTIVE_EPOCH, m._LAST_LIVENESS_KNOWN,
         m._release_shared_restore_lease, m._RESTORE_LEASE_HANDLE) = orig
        shutil.rmtree(td, ignore_errors=True)


def main():
    if m.IS_WINDOWS:
        # msvcrt 는 같은 프로세스의 다른 fd 를 경합으로 보지 않을 수 있어 프로브 술어가 성립하지 않는다.
        print("SKIP P10 lease split: Windows byte-range 락은 같은 프로세스 프로브로 잴 수 없다")
        print("PHOENIX-P10-LEASE-SPLIT-SKIP")
        return 0

    fixed = run_scenario("fixed")
    if fixed["spawn"] is None or fixed["verify"] is None:
        print("FAIL 하네스 구동(스폰·재주입 국면에 도달하지 못했다) — %r" % (fixed["result"],))
        return 1

    check("① 스폰 구간에서는 restore.lease 가 잡혀 있다(reclaim Defer 가 옳은 구간)",
          fixed["spawn"][0] == "held", repr(fixed["spawn"]))
    check("① 스폰 구간에서 restore.run.lock 도 잡혀 있다",
          fixed["spawn"][1] == "held", repr(fixed["spawn"]))
    check("② ④ 진입 뒤 restore.lease 는 **비어 있다** — 그 창의 좌석은 reclaim_auto 가 Defer 로 끝나지 않는다",
          fixed["verify"] == "free", repr(fixed["verify"]))
    check("③ 같은 순간 restore.run.lock 은 여전히 잡혀 있다(restore 끼리의 배타는 끊기지 않았다)",
          fixed["verify_run"] == "held", repr(fixed["verify_run"]))
    check("③ 그 창에 들어온 다른 restore 는 LEASE_HELD 로 접힌다(이중 스폰 0)",
          (fixed["reentrant"] or {}).get("phoenix_restore") == "LEASE_HELD", repr(fixed["reentrant"]))
    check("③ 접힌 restore 는 저널을 쓰지 않는다(후행 쓰기 경합 0)",
          fixed["reentrant_journal_before"] == fixed["reentrant_journal_after"])
    check("부활 자체는 정상 완주한다(락 분리가 상태머신을 바꾸지 않는다)",
          (fixed["result"] or {}).get("target_roles") == ["worker-1"]
          and (fixed["result"] or {}).get("backend") == "production(cys restore)",
          repr((fixed["result"] or {}).get("phoenix_restore")))

    # ④ 음성 대조 — 조기 해제를 없애면 ②가 실제로 'held' 로 되돌아간다(핀이 살아 있다).
    old = run_scenario("legacy", disable_early_release=True)
    check("④ 음성 대조: 조기 해제가 없으면 ④ 구간에서도 lease 가 잡혀 있다(종전 결함 재현)",
          old["verify"] == "held", repr(old["verify"]))

    # ⑤ 멱등·이중 정리
    m._RESTORE_LEASE_HANDLE = None
    check("⑤ 조기 해제는 멱등이다(보유 없으면 무동작·예외 0)", m._release_shared_restore_lease("t") is False)
    td = tempfile.mkdtemp(prefix="phoenix-p10-idem-")
    try:
        sock = os.path.join(td, "cys.sock")
        ok, handle = m._acquire_restore_lease(sock)
        m._RESTORE_LEASE_HANDLE = handle
        first = m._release_shared_restore_lease("t")
        second = m._release_shared_restore_lease("t")
        if handle is not None:
            m._release_lease(handle)          # atexit 중복 정리 — 예외 0
        check("⑤ 첫 해제만 참이고 두 번째는 무동작(atexit 중복 정리 안전)", ok and first is True and second is False)
        check("⑤ 해제 뒤 그 파일은 다시 잡힌다", _probe(os.path.join(m.phoenix_home(sock), "restore.lease")) == "free")
    finally:
        m._RESTORE_LEASE_HANDLE = None
        shutil.rmtree(td, ignore_errors=True)

    # ⑦ 같은 프로세스 **순차 재호출** — 두 번 다 진행한다(락이 함수 반환 시점에 풀린다).
    #    회귀 축: run.lock 을 atexit 로만 놓으면 두 번째 호출이 자기 락에 막혀 영구 LEASE_HELD 가 된다
    #    (deploy 의 LEASE_HELD backoff 재시도·auto-retry 2차 실행이 통째로 죽는 자리).
    td2 = tempfile.mkdtemp(prefix="phoenix-p10-seq-")
    orig2 = (m.cys, m.CYS, m._emit_evt, m._RESTORE_LEASE_HANDLE)
    try:
        sock2 = os.path.join(td2, "cys.sock")
        home2 = m.phoenix_home(sock2)
        with open(os.path.join(home2, "desired_roster.json"), "w", encoding="utf-8") as f:
            json.dump({"roster": {}, "tombstones": []}, f)
        m.CYS = os.path.join(td2, "nonexistent-cys")
        m._emit_evt = lambda *a, **k: None
        m._RESTORE_LEASE_HANDLE = None
        r1 = m.run_restore(sock2, ticket="seq1", stub=True, print_result=False)
        r2 = m.run_restore(sock2, ticket="seq2", stub=True, print_result=False)
        check("⑦ 같은 프로세스 순차 재호출이 둘 다 진행한다(LEASE_HELD 0)",
              r1.get("phoenix_restore") != "LEASE_HELD" and r2.get("phoenix_restore") != "LEASE_HELD",
              "%s / %s" % (r1.get("phoenix_restore"), r2.get("phoenix_restore")))
        check("⑦ 반환 뒤에는 두 락이 모두 비어 있다",
              _probe(os.path.join(home2, "restore.lease")) == "free"
              and _probe(os.path.join(home2, "restore.run.lock")) == "free")
    finally:
        (m.cys, m.CYS, m._emit_evt, m._RESTORE_LEASE_HANDLE) = orig2
        shutil.rmtree(td2, ignore_errors=True)

    # ⑥ 구조 계약 — 껍데기(run.lock 전 구간)와 본체(lease · 스폰 구간)의 분업
    wrap = inspect.getsource(m.run_restore)
    body = inspect.getsource(m._run_restore_locked)
    check("⑥ 껍데기가 run.lock 을 먼저 잡고 본체를 부른다(두 restore 가 lease 를 번갈아 쥐는 형상 0)",
          "_acquire_restore_run_lock(socket)" in wrap and "_run_restore_locked(" in wrap
          and wrap.index("_acquire_restore_run_lock(socket)") < wrap.index("_run_restore_locked(")
          and "_acquire_restore_run_lock" not in body)
    check("⑥ 껍데기의 finally 가 두 락을 **함수 반환 시점에** 놓는다(같은 프로세스 재호출이 자기 락에 막히지 않는다)",
          "finally:" in wrap and "_release_shared_restore_lease(" in wrap
          and "_release_lease(_run_handle)" in wrap
          and wrap.index("finally:") < wrap.index("_release_lease(_run_handle)"))
    check("⑥ 조기 해제는 본체의 역할별 하위 단계 루프 **직전** 1지점이다(스폰 뒤 · ④ 앞)",
          body.count("_release_shared_restore_lease(") == 1
          and body.rindex("for role in pending:") > body.index("_release_shared_restore_lease(")
          > body.rindex("need = [r for r in pending"))
    check("⑥ 두 락 파일 이름이 갈려 있다(cysd 가 읽는 축은 restore.lease 하나)",
          '"restore.lease"' in inspect.getsource(m._acquire_restore_lease)
          and '"restore.run.lock"' in inspect.getsource(m._acquire_restore_run_lock))
    check("⑥ 닫지 못한 것을 코드가 말한다 — 스폰 중 Defer 된 훅의 재결합은 reclaim 쪽 몫",
          "reclaim" in inspect.getsource(m._acquire_restore_run_lock) or "reclaim" in body)

    fails = len([r for r in _results if not r])
    print("\n=== %d checks · FAIL %d ===" % (len(_results), fails))
    print("PHOENIX-P10-LEASE-SPLIT-" + ("OK" if fails == 0 else "FAIL"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
