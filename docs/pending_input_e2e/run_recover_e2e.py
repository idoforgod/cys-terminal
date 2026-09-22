#!/usr/bin/env python3
"""rc 79(EXIT_RECOVER_REFUSED) 격리 데몬 E2E (I-0 인계 ③).

실행 (빌드는 지휘자가 미리 수행; 이 스크립트는 cargo를 실행하지 않는다):
  cargo build --bin cysd --bin cys
  CYS_E2E_OUT=<out> python3 docs/pending_input_e2e/run_recover_e2e.py /tmp/<짧은심링크>

바이너리: $CARGO_TARGET_DIR/debug/{cysd,cys} (기본 REPO/target),
CYS_E2E_CYSD/CYS_E2E_CYS override. 출력 기본값: $CARGO_TARGET_DIR/recover-e2e.
run-log.jsonl/JSON, CLI 원출력, _sandbox/run-*/cysd.log를 보존한다.
종료코드 0은 A의 전제와 rc79·안전 거부 문구·비파괴 5항을 모두 실측한 경우다.
B와 C, 관측 불능(null)은 별도 기록하며 C 실패를 A 실패로 바꾸지 않는다.
"""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time


def load_harness(short):
    """run_e2e.py의 argv 의존성과 __pycache__ 생성을 import 동안만 처리한다."""
    argv, no_bytecode = sys.argv, sys.dont_write_bytecode
    try:
        sys.argv = [__file__, short]
        sys.dont_write_bytecode = True
        spec = importlib.util.spec_from_file_location(
            "pending_input_e2e", Path(__file__).with_name("run_e2e.py"))
        h = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(h)
    finally:
        sys.argv, sys.dont_write_bytecode = argv, no_bytecode
    h.OUT = os.path.abspath(os.environ.get("CYS_E2E_OUT", os.path.join(h._TARGET, "recover-e2e")))
    h.RUN = os.path.join(h.OUT, "_sandbox", time.strftime("run-%Y%m%d-%H%M%S-") + str(os.getpid()))
    h.CYS, h.CYSD = os.path.abspath(h.CYS), os.path.abspath(h.CYSD)
    return h


def process_table():
    """명령행/환경 없이 PID·부모·생성 시각만 관측한다(PID 재사용 방지)."""
    p = subprocess.run(["/bin/ps", "-axo", "pid=,ppid=,lstart="],
                       capture_output=True, text=True, timeout=5)
    if p.returncode:
        raise RuntimeError(p.stderr.strip() or "ps failed")
    rows = {}
    for line in p.stdout.splitlines():
        fields = line.split(None, 2)
        if len(fields) == 3:
            rows[int(fields[0])] = (int(fields[1]), fields[2])
    return rows


def descendants(table, root):
    owned = {root} if root in table else set()
    while True:
        extra = {pid for pid, (ppid, _) in table.items() if ppid in owned} - owned
        if not extra:
            return {pid: table[pid][1] for pid in owned}
        owned.update(extra)


def cli(h, d, label, *args):
    # 출처: run_e2e.py::cli. 원문의 400자 절단을 제거하고 cwd·setsid를 격리한다.
    # setsid + setup()의 새 env에는 pane/surface ID가 없어 authoritative 면제를 받지 않는다.
    argv = [h.CYS, "--socket", d.sock, *args]
    with subprocess.Popen(argv, env=dict(d.env), cwd=h.RUN, start_new_session=True,
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, encoding="utf-8",
                          errors="replace") as p:
        timed_out = False
        try:
            stdout, stderr = p.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(p.pid, signal.SIGKILL)
            stdout, stderr = p.communicate()
        except BaseException:
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGKILL)
            p.communicate()
            raise
    for stream, data in (("stdout", stdout), ("stderr", stderr)):
        Path(h.OUT, label + "." + stream + ".txt").write_text(data, encoding="utf-8")
    result = {"argv": ["cys", "--socket", "<sandbox>", *args],
              "rc": None if timed_out else p.returncode, "process_rc": p.returncode,
              "timed_out": timed_out, "stdout": stdout, "stderr": stderr}
    h.log(label + ".cli", **result)
    return result


def surfaces(h, d, label):
    r = d.rpc("surface.list", {})
    h.log(label, response=r)
    d.observe_children()
    if not r.get("ok"):
        raise RuntimeError("surface.list failed: " + json.dumps(r, ensure_ascii=False))
    result = r.get("result")
    rows = result.get("surfaces") if isinstance(result, dict) else result
    if not isinstance(rows, list):
        raise RuntimeError("surface.list has no surfaces array")
    return rows


def sid_of(row):
    return row.get("id", row.get("surface_id"))


def seat(rows, sid):
    return next((r for r in rows if sid_of(r) == sid), {})


def require_rpc(r):
    if not r.get("ok"):
        raise RuntimeError("RPC failed: " + json.dumps(r, ensure_ascii=False))


def make_seat(h, d, role, label):
    d.create_role = role
    try:
        sid, _ = h.mk(d, label)
    finally:
        d.create_role = None
    r = d.rpc("surface.set_meta", {"surface_id": sid, "agent": "claude", "role": role})
    h.log(label + ".meta", sid=sid, role=role, response=r,
          role_source="surface.create (surface.set_meta currently ignores role)")
    require_rpc(r)
    # fake_tui가 raw 모드·첫 프레임을 준비하기 전에 사람 입력을 보내지 않는다.
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        text = h.screen(d, sid)
        if "fake-agent body" in text and "❯" in text:
            h.log(label + ".screen", sid=sid, text=text)
            return sid
        time.sleep(0.1)
    raise RuntimeError("fake_tui did not render its prompt")


def equal_observed(before, after, key):
    a, b = before.get(key), after.get(key)
    return a == b if a is not None and b is not None else None


def cleanup(h, d):
    observed = dict(getattr(d, "observed_children", {}))
    error, residual = None, None
    if d and d.proc:
        try:
            observed.update(descendants(process_table(), d.proc.pid))
        except Exception as exc:
            error = str(exc)
        # A/C의 관측이 끝난 뒤에만 닫는다. 데몬이 PTY 자식을 kill/wait하도록 한다.
        try:
            if d.proc.poll() is None:
                for row in surfaces(h, d, "cleanup.surfaces"):
                    r = d.rpc("surface.close", {"surface_id": sid_of(row)})
                    h.log("cleanup.close", sid=sid_of(row), response=r)
        except Exception as exc:
            h.log("cleanup.close_error", error=str(exc))
        finally:
            d.stop()
        if observed:
            try:
                for sig in (None, signal.SIGTERM, signal.SIGKILL):
                    deadline = time.monotonic() + 2
                    while True:
                        table = process_table()
                        residual = [pid for pid, born in observed.items()
                                    if pid in table and table[pid][1] == born]
                        if not residual or time.monotonic() >= deadline:
                            break
                        if sig is not None:
                            for pid in residual:
                                try:
                                    os.kill(pid, sig)
                                except ProcessLookupError:
                                    pass
                        time.sleep(0.1)
                    if not residual:
                        break
            except Exception as exc:
                error, residual = str(exc), None
    h.log("cleanup.processes", daemon_rc=d.proc.returncode if d and d.proc else None,
          observed_pids=sorted(observed), residual_pids=residual,
          residual_count=len(residual) if residual is not None else None,
          observation_error=error)
    return len(residual) == 0 if residual is not None else None


def main():
    checks = {"A": dict.fromkeys(("agent_alive_null", "role_matches", "draft_present",
                                  "rc79", "refusal_stderr", "seat_count_unchanged",
                                  "seat_not_exited", "pid_unchanged", "draft_bytes_unchanged",
                                  "no_new_surfaces")), "B_rc_not_79": None,
              "C_recover_refused": None, "children_remaining_zero": None,
              "symlink_removed": None}
    if len(sys.argv) != 2:
        print(json.dumps({"section": "error", "error": "usage: run_recover_e2e.py /tmp/<short-symlink>"}))
        print(json.dumps({"section": "verdict", "pass": False, "checks": checks}))
        return 1
    h = load_harness(os.path.abspath(sys.argv[1]))
    d, log_file = None, None
    original_log = h.log

    def log(section, **kw):
        original_log(section, **kw)
        if log_file:
            log_file.write(json.dumps(h.LOG[-1], ensure_ascii=False) + "\n")
            log_file.flush()

    h.log = log
    try:
        # setup()는 기존 symlink를 지우므로 소유하지 않은 경로는 먼저 거부한다.
        if os.path.lexists(h.SHORT):
            raise RuntimeError("short symlink path already exists; choose an unused path")
        if len(os.fsencode(os.path.join(h.SHORT, "s", "cys.sock"))) >= 100:
            raise RuntimeError("short symlink socket path must be shorter than 100 bytes")
        # live HOME 안의 금지 영역을 출력 경로로도 사용하지 않는다.
        live_home = Path.home().resolve()
        for path in (Path(h.OUT).resolve(), Path(h.SHORT).resolve()):
            if path.is_relative_to(live_home):
                first = path.relative_to(live_home).parts
                if first and (first[0] == ".cys" or first[0].startswith(".claude")):
                    raise RuntimeError("refusing a live HOME path")
        Path(h.OUT).mkdir(parents=True, exist_ok=True)
        log_file = Path(h.OUT, "run-log.jsonl").open("w", encoding="utf-8")
        h.log("setup.paths", out=h.OUT, run=h.RUN, short=h.SHORT, cys=h.CYS, cysd=h.CYSD)
        for binary in (h.CYS, h.CYSD):
            if not os.path.isfile(binary) or not os.access(binary, os.X_OK):
                raise RuntimeError("built executable unavailable: " + binary)
        sock, env = h.setup()
        # node-recover의 C-u 이전 load_agent_spec에는 agents.json이 필수다.
        # 기존 자산 하나만 복사: topology/지침/계정/라이브 팩은 복사하지 않는다.
        Path(env["CYS_PACK_DIR"]).mkdir(exist_ok=True)
        source = Path(h.REPO, "cysjavis-pack", "agents.json")
        shutil.copyfile(source, Path(env["CYS_PACK_DIR"], "agents.json"))
        Path(h.OUT, "daemon-env.json").write_text(json.dumps(env, indent=2), encoding="utf-8")
        h.log("setup.env", env=env, agents_source=str(source),
              claude_on_path=shutil.which("claude", path=env["PATH"]))
        if shutil.which("claude", path=env["PATH"]) is not None:
            raise RuntimeError("negative control requires no claude binary on sandbox PATH")

        class D(h.D):
            create_role = None

            def __init__(self, sock, env):
                super().__init__(sock, env)
                self.observed_children = {}

            def observe_children(self):
                # 뒤에 데몬이 비정상 종료·자식이 reparent돼도 이미 관측한 소유권은 보존한다.
                try:
                    self.observed_children.update(descendants(process_table(), self.proc.pid))
                except Exception:
                    pass  # cleanup에서 관측 실패를 null + 원인으로 기록한다.

            def rpc(self, method, params):
                # mk() 재사용. set_meta는 role을 무시하고 외부 claim_role은 거부하므로
                # 실제 역할은 surface.create에서 지정한다.
                if method == "surface.create" and self.create_role:
                    params = dict(params, role=self.create_role)
                return super().rpc(method, params)

        d = D(sock, env)
        d.start()
        d.observe_children()
        h.log("daemon", pid=d.proc.pid, sock=sock)
        sid = make_seat(h, d, "worker", "A")
        initial = seat(surfaces(h, d, "A.initial"), sid)
        checks["A"]["agent_alive_null"] = (initial["agent_alive"] is None
                                               if "agent_alive" in initial else None)
        checks["A"]["role_matches"] = initial.get("role") == "worker" if initial else None
        if checks["A"]["agent_alive_null"] is not True or checks["A"]["role_matches"] is not True:
            raise RuntimeError("A precondition not observed: agent_alive=null and role=worker")
        r = h.human(d, sid, "사람이 치던 초안")
        h.log("A.human", sid=sid, response=r)
        require_rpc(r)
        before = surfaces(h, d, "A.before")
        old = seat(before, sid)
        checks["A"]["agent_alive_null"] = (old["agent_alive"] is None
                                               if "agent_alive" in old else None)
        count = old.get("pending_input_human_bytes")
        checks["A"]["draft_present"] = count > 0 if isinstance(count, int) else None
        if (checks["A"]["draft_present"] is not True
                or checks["A"]["agent_alive_null"] is not True):
            raise RuntimeError("A precondition changed or human draft counter not observed")
        result = cli(h, d, "A", "node-recover", "--role", "worker")
        after = surfaces(h, d, "A.after")
        new = seat(after, sid)
        checks["A"].update(
            rc79=result["rc"] == 79 if result["rc"] is not None else None,
            refusal_stderr="node-recover 안전 거부" in result["stderr"],
            seat_count_unchanged=len(before) == len(after),
            seat_not_exited=((old["exited"] is False and new["exited"] is False)
                             if isinstance(old.get("exited"), bool)
                             and isinstance(new.get("exited"), bool) else None),
            pid_unchanged=equal_observed(old, new, "pid"),
            draft_bytes_unchanged=equal_observed(old, new, "pending_input_human_bytes"),
            no_new_surfaces=not ({sid_of(r) for r in after} - {sid_of(r) for r in before}))
        h.log("A.nondestructive", sid=sid, before_count=len(before), after_count=len(after),
              new_surfaces=sorted({sid_of(r) for r in after} - {sid_of(r) for r in before}),
              before=old, after=new, checks=checks["A"], screen=h.screen(d, sid))

        # 역할은 유일해야 하므로 대조 좌석에는 worker-control을 사용한다.
        try:
            control = make_seat(h, d, "worker-control", "B")
            row = seat(surfaces(h, d, "B.before"), control)
            ready = (row.get("pending_input_human_bytes") == 0
                     and "agent_alive" in row and row["agent_alive"] is None
                     and row.get("role") == "worker-control")
            h.log("B.precondition", sid=control, no_human_draft=ready, row=row)
            if ready:
                result = cli(h, d, "B", "node-recover", "--role", "worker-control")
                checks["B_rc_not_79"] = result["rc"] != 79 if result["rc"] is not None else None
                surfaces(h, d, "B.after")
        except Exception as exc:
            h.log("B.error", error=str(exc))

        try:
            result = cli(h, d, "C", "boot", "--json")
            lines = result["stdout"].splitlines()
            last = lines[-1] if lines else None
            parsed, parse_error = None, None
            try:
                parsed = json.loads(last) if last is not None else None
            except ValueError as exc:
                parse_error = str(exc)
            roles = parsed.get("roles", []) if isinstance(parsed, dict) else []
            rows = [r for r in roles if isinstance(r, dict)
                    and r.get("surface_ref") == "surface:%s" % sid]
            if rows:
                checks["C_recover_refused"] = any(
                    r.get("outcome") == "skipped_unconfirmed" and r.get("liveness") == "recover_refused"
                    for r in rows)
            h.log("C.boot", last_stdout_line=last, json=parsed, parse_error=parse_error,
                  boot_reached_role=bool(rows), seat_rows=rows,
                  outcome=rows[0].get("outcome") if rows else None,
                  liveness=rows[0].get("liveness") if rows else None)
        except Exception as exc:
            h.log("C.error", error=str(exc), boot_reached_role=None, outcome=None, liveness=None)
    except (Exception, KeyboardInterrupt) as exc:
        h.log("error", error=str(exc), type=type(exc).__name__)
    finally:
        try:
            checks["children_remaining_zero"] = cleanup(h, d)
        except Exception as exc:
            h.log("cleanup.error", error=str(exc))
        try:
            if os.path.islink(h.SHORT) and os.readlink(h.SHORT) == h.RUN:
                os.unlink(h.SHORT)
                checks["symlink_removed"] = not os.path.lexists(h.SHORT)
            h.log("cleanup.symlink", removed=checks["symlink_removed"])
        except OSError as exc:
            h.log("cleanup.symlink_error", error=str(exc))
        passed = all(value is True for value in checks["A"].values())
        h.log("verdict", **{"pass": passed, "checks": checks,
                           "unobserved_A": [k for k, v in checks["A"].items() if v is None]})
        if log_file:
            log_file.close()
            Path(h.OUT, "run-log.json").write_text(json.dumps(h.LOG, ensure_ascii=False, indent=1),
                                                  encoding="utf-8")
    return 0 if passed else 1


if __name__ == "__main__":
    # SIGTERM도 일반 예외 경로로 보내 finally의 데몬/심링크 정리를 실행한다.
    def interrupted(signum, _frame):
        raise InterruptedError("signal %s" % signum)

    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
