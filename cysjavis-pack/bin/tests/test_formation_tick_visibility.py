#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""WP6-5 — 실제 락 경합·최후 예외의 심박 흔적과 정상 기록 바이트 규약.

실패 방향: 관측 기록 실패는 복구 반환을 바꾸지 않는다. 상태가 없으면 만들지 않는다.
정상 기록의 write-lock 대기는 유계다 — 보유자가 안 놓으면 STATE_WRITE_LOCK_WAIT_SEC 안에 loud WARN 으로
return 하고(무기한 대기 금지 · 판정 1회 미영속), 보유자가 놓으면 다시 쓴다(4g·4h).
리포 팩·임시 상태로 밀폐하고 subprocess를 차단한다(cys·cysd 실행 0).
"""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
from unittest import mock

SELF = Path(__file__).resolve().parent
os.environ["CYS_PACK_DIR"] = str(SELF.parent.parent)
os.environ.pop("CYS_FORMATION_EXTERNAL_ROLES", None)
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def main():
    spec = importlib.util.spec_from_file_location("javis_formation", SELF.parent / "javis_formation.py")
    jf = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(jf)
    tick_keys = {"last_tick", "last_tick_epoch", "last_tick_note"}

    def without_tick(obj):
        return {k: v for k, v in obj.items() if k not in tick_keys}

    with tempfile.TemporaryDirectory(prefix="formation-tick-") as root, \
            mock.patch.dict(os.environ, {"CYS_STATE_DIR": root}), \
            mock.patch.object(jf.subprocess, "run", side_effect=AssertionError("subprocess 금지")), \
            mock.patch.object(jf, "_feed"), mock.patch.object(jf, "_emit_evt"):
        sock = str(Path(root) / "department.sock")
        path = Path(jf._write_state(sock, "partial:booting", "이전 판정", ["master"]))
        before = jf._read_state_obj(sock)
        owner = {}

        def owner_writes_then_allows():
            # ensure가 읽은 prev_obj보다 최신인 락 보유자의 기록을 재현한다.
            jf._write_state(sock, "pending-resource", "락 보유자의 최신 판정", ["master", "cso"],
                            attempts={"worker": {"count": 3, "last_at": 123.0}},
                            gate={"verdict": "hard", "trips": [{"metric": "nodes"}]},
                            held=["worker"])
            owner.update(jf._read_state_obj(sock))
            return True

        # ① 실제 동일 소켓 락 경합 — 이전 이월본이 최신 판정·시도 원장을 덮으면 실패한다.
        with jf._singleflight(sock) as lock, \
                mock.patch.object(jf, "gate_check", side_effect=owner_writes_then_allows):
            check("1a 선행 싱글플라이트 락 획득", lock.acquired)
            state, detail = jf.ensure(socket=sock)
        after = jf._read_state_obj(sock)
        check("1b inflight 반환은 그대로", state == "partial:inflight")
        check("1c 심박 도달 시각과 사유 기록",
              tick_keys <= after.keys()
              and after.get("last_tick_epoch") != before.get("last_tick_epoch")
              and after.get("last_tick_note") == "inflight: " + detail)
        check("1d 최신 판정·시도·보류·게이트·갱신 시각 전부 불변",
              without_tick(after) == owner)

        # ② 최후 예외 — failed 반환·기존 판정 보존·사유 200자 한계를 함께 잰다.
        message = "예" * 250
        output = io.StringIO()
        with mock.patch.object(jf, "ensure", side_effect=ValueError(message)), \
                contextlib.redirect_stdout(output):
            rc = jf._cmd_ensure(["--socket", sock, "--json"])
        failed = jf._read_state_obj(sock)
        summary = json.loads(output.getvalue())
        check("2a failed는 exit 1과 원래 JSON 예외를 반환",
              rc == 1 and summary["state"] == "failed:ValueError"
              and summary["kind"] == "failed" and not summary["ok"] and summary["detail"] == message)
        check("2b 예외 흔적 기록·사유 200자 제한",
              failed.get("last_tick_note") == "failed:ValueError — " + message[:200]
              and failed.get("last_tick_epoch") != after.get("last_tick_epoch"))
        check("2c 예외도 판정 키·갱신 시각 불변", without_tick(failed) == owner)

        # ③ 실제 정상 complete 경로 — 시각을 고정해 종전 JSON 바이트와 직접 비교한다.
        roles = set(jf.REQUIRED_ROLES)
        with mock.patch.object(jf, "gate_check", return_value=True), \
                mock.patch.object(jf, "_installed_clis", return_value=set(jf.REQUIRED_CLIS)), \
                mock.patch.object(jf, "_live_roles", return_value=roles), \
                mock.patch.object(jf.time, "strftime", return_value="2001-02-03T04:05:06"), \
                mock.patch.object(jf.time, "time", return_value=1000.0):
            state, detail = jf.ensure(socket=sock)
        normal = jf._read_state_obj(sock)
        expected = {
            "_doc": "파생 캐시 — 복원 정본 아님(정본=depts.json)",
            "state": "complete", "kind": "complete", "socket": sock,
            "detail": "라이브 로스터 이미 완결(5역할 생존) — 신규 기동 0 · 자원 게이트 무관",
            "roles_booted": sorted(roles), "updated_at": "2001-02-03T04:05:06", "updated_epoch": 1000.0,
        }
        check("3a 정상 complete는 last_tick·빈 선택 키를 넣지 않는다",
              state == "complete" and set(normal) == set(expected) and not tick_keys.intersection(normal))
        expected_bytes = (json.dumps(expected, ensure_ascii=False, indent=1) + "\n").encode("utf-8")
        check("3b 정상 경로의 종전 JSON 바이트 보존", path.read_bytes() == expected_bytes)

        # 기록 불능은 복구 반환에 영향 0 — 실제 os.replace 실패를 주입한다.
        stderr, stdout = io.StringIO(), io.StringIO()
        with mock.patch.object(jf.os, "replace", side_effect=OSError("disk unavailable")), \
                mock.patch.object(jf, "gate_check", return_value=True), \
                contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            with jf._singleflight(sock):
                inflight_state, _ = jf.ensure(socket=sock)
            with mock.patch.object(jf, "ensure", side_effect=RuntimeError("ensure failed")):
                failed_rc = jf._cmd_ensure(["--socket", sock, "--json"])
        check("4a 기록 실패도 inflight·failed 반환 유지",
              inflight_state == "partial:inflight" and failed_rc == 1)
        check("4b 기록 실패는 조용히 무시·원본 보존·임시파일 회수",
              stderr.getvalue() == "" and path.read_bytes() == expected_bytes
              and not list(path.parent.glob("*.tmp")))

        with jf._state_write_lock(str(path)):
            skipped = jf._touch_state(sock, "writer busy")
        check("4c 정상 writer가 쓰는 동안 심박 기록은 즉시 생략",
              skipped is None and path.read_bytes() == expected_bytes)

        # read→replace 사이 정상 writer를 진입시킨다. 잠금 제거 대조군은 실제로 덮어써야 한다.
        def overlapping_writers(lock_enabled):
            jf._write_state(sock, "partial:booting", "old", [])
            tick_ready, release_tick = threading.Event(), threading.Event()
            owner_started, owner_done = threading.Event(), threading.Event()
            errors = []
            replace = jf._replace_state_obj

            def pause_tick(target, obj):
                if "last_tick_note" in obj:
                    tick_ready.set()
                    if not release_tick.wait(3):
                        raise RuntimeError("tick release timeout")
                replace(target, obj)

            def write_owner():
                owner_started.set()
                try:
                    jf._write_state(sock, "complete", "owner latest", roles)
                except Exception as e:
                    errors.append(str(e))
                finally:
                    owner_done.set()

            lock_control = (contextlib.nullcontext() if lock_enabled else
                            mock.patch.object(jf, "_state_write_lock",
                                              side_effect=lambda *a, **kw: contextlib.nullcontext()))
            with lock_control, mock.patch.object(jf, "_replace_state_obj", side_effect=pause_tick):
                tick = threading.Thread(target=jf._touch_state, args=(sock, "overlap"), daemon=True)
                owner_thread = threading.Thread(target=write_owner, daemon=True)
                tick.start()
                try:
                    ready = tick_ready.wait(2)
                    owner_thread.start()
                    started = owner_started.wait(2)
                    finished_early = owner_done.wait(0.1 if lock_enabled else 2)
                finally:
                    release_tick.set()
                    tick.join(2)
                    if owner_thread.ident is not None:
                        owner_thread.join(2)
            return (ready and started and not errors and not tick.is_alive()
                    and not owner_thread.is_alive(), finished_early, jf._read_state_obj(sock))

        ok, finished, raced = overlapping_writers(False)
        check("4d 잠금 제거 대조군은 최신 판정 덮어쓰기를 검출",
              ok and finished and raced.get("state") == "partial:booting")
        ok, finished, serialized = overlapping_writers(True)
        check("4e 심박 read→replace 동안 정상 writer의 쓰기를 직렬화", ok and not finished)
        check("4f 경쟁 쓰기 뒤 최신 판정 보존",
              serialized.get("state") == "complete" and serialized.get("detail") == "owner latest"
              and not tick_keys.intersection(serialized))

        # ★WP6 리뷰(동시성): 다른 스레드가 write-lock 을 쥔 채 놓지 않으면 정상 기록은 상한 안에 loud WARN 으로
        #   끝나야 한다(무기한 대기 = 싱글플라이트를 쥔 채 편성 자가치유 영구 정지). 실제 상수·실제 락으로 잰다.
        cap = jf.STATE_WRITE_LOCK_WAIT_SEC
        held_bytes = path.read_bytes()
        holder_ready, holder_release = threading.Event(), threading.Event()

        def hold_write_lock():
            with jf._state_write_lock(str(path)):
                holder_ready.set()
                holder_release.wait(cap + 5)

        holder = threading.Thread(target=hold_write_lock, daemon=True)
        holder.start()
        stderr = io.StringIO()
        try:
            check("4g0 보유자 스레드 write-lock 선점", holder_ready.wait(2))
            t0 = time.monotonic()
            with contextlib.redirect_stderr(stderr):
                stuck = jf._write_state(sock, "partial:booting", "stuck holder", roles)
            elapsed = time.monotonic() - t0
        finally:
            holder_release.set()
            holder.join(2)
        warn = stderr.getvalue()
        check("4g 보유자 미해제 시 상한 안에 None 반환(대기는 상한까지 실제로 했다)",
              stuck is None and cap * 0.9 <= elapsed < cap + 1.0,
              "elapsed=%.2fs cap=%.1fs" % (elapsed, cap))
        check("4h 판정 미영속은 loud WARN 1줄(버린 판정 명시)·원본 보존·임시파일 0",
              warn.count("[formation] WARN:") == 1 and "write-lock 대기 상한" in warn
              and "판정 partial:booting 미영속" in warn and path.read_bytes() == held_bytes
              and not list(path.parent.glob("*.tmp")) and not holder.is_alive(),
              warn.strip()[:160])
        # 대조군(무너지는 방향): 보유자가 놓으면 같은 호출이 즉시 쓴다 — 상한은 보유자에 대한 것이지 락 고장이 아니다.
        t0 = time.monotonic()
        with contextlib.redirect_stderr(io.StringIO()):
            resumed = jf._write_state(sock, "complete", "owner latest", roles)
        check("4i 보유자 해제 뒤 정상 기록 즉시 재개",
              resumed is not None and time.monotonic() - t0 < 1.0
              and jf._read_state_obj(sock).get("detail") == "owner latest")

        # 상태가 없거나 손상됐으면 임의 상태를 만들어 판정하지 않는다.
        missing_root = str(Path(root) / "never-written")
        with mock.patch.dict(os.environ, {"CYS_STATE_DIR": missing_root}):
            result = jf._touch_state(sock, "inflight")
            with mock.patch.object(jf, "ensure", side_effect=RuntimeError("missing")), \
                    contextlib.redirect_stdout(io.StringIO()):
                missing_rc = jf._cmd_ensure(["--socket", sock, "--json"])
            check("5a 상태 부재는 None·생성 금지·예외 exit 유지",
                  result is None and missing_rc == 1 and not Path(missing_root).exists())
        path.write_bytes(b"{broken")
        check("5b 손상 상태도 덮어쓰지 않는다",
              jf._touch_state(sock, "inflight") is None and path.read_bytes() == b"{broken")

    if fails:
        print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
        return 1
    print("\nALL PASS")
    print("FORMATION-TICK-VISIBILITY-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
