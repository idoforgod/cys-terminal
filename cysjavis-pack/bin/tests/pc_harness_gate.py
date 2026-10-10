#!/usr/bin/env python3
"""pc_harness_gate.py - TEST HARNESS ONLY (F5 lane B review R1 M1).

Runs the product gate (path in PC_HARNESS_GATE) exactly like `python3 javis_resource_gate.py <args>` does, but replaces the
`product-cpu` helper lookup with a fixture read from PC_HARNESS_LOOKUP (JSON object; keys all optional):
  {"missing": true}                      resolving the lookup raises AttributeError
  {"raise": "OSError"|"RuntimeError"|"ArgumentError"}   every call raises that exception
  {"hang_s": N, "pid_file": PATH}        the first call records this process's pid in PATH, then sleeps N s
  {"map": {"<pid>": <any JSON>}}         answer per helper pid (absent pid -> -1)
  {"trace_file": PATH}                   one line per resolve / call
The product reads none of these variables and writes no file because of them. This harness' own writes are create-only
(O_EXCL): a path that already exists - a verdict file, or an alias of it - is never truncated or appended to.
The child that the text-check tail starts is this wrapper again (the module's __file__ is pointed here), so the fixture
reaches it. Not shipped logic: lives under tests/, imported by nothing in the product."""
import functools
import importlib.util
import json
import os
import sys
import time

SPEC_ENV = "PC_HARNESS_LOOKUP"
GATE_ENV = "PC_HARNESS_GATE"


def _create_only_write(path, data, mode="w"):
    """Write `data` to a file THIS call creates. Existing target (also through a symlink / hard link) -> silently skipped."""
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except OSError:
        return False
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(data)
    return True


def make_resolver(spec_text):
    state = {"trace_id": None}

    def note(spec, msg):
        trace = spec.get("trace_file")
        if not trace:
            return
        if state["trace_id"] is None:
            try:
                fd = os.open(trace, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_APPEND, 0o600)
            except OSError:
                state["trace_id"] = False
                return
            st = os.fstat(fd)
            state["trace_id"] = (st.st_dev, st.st_ino)
            with os.fdopen(fd, "a", encoding="utf-8") as fh:
                fh.write(msg + "\n")
            return
        if state["trace_id"] is False:
            return
        try:
            fd = os.open(trace, os.O_WRONLY | os.O_APPEND)
        except OSError:
            return
        st = os.fstat(fd)
        if (st.st_dev, st.st_ino) == state["trace_id"]:
            with os.fdopen(fd, "a", encoding="utf-8") as fh:
                fh.write(msg + "\n")
        else:
            os.close(fd)

    def resolve():
        spec = json.loads(spec_text)
        if not isinstance(spec, dict):
            raise ValueError("lookup fixture is not a JSON object")
        note(spec, "resolve")
        if spec.get("missing"):
            raise AttributeError("responsibility_get_pid_responsible_for_pid: symbol not found (fixture)")
        n = {"n": 0}

        def fn(pid):
            n["n"] += 1
            note(spec, "call %s" % pid)
            exc = spec.get("raise")
            if exc:
                import ctypes
                raise {"OSError": OSError, "RuntimeError": RuntimeError,
                       "ArgumentError": ctypes.ArgumentError}.get(exc, RuntimeError)("fixture: the lookup failed")
            if spec.get("hang_s") and n["n"] == 1:
                if spec.get("pid_file"):
                    _create_only_write(spec["pid_file"], str(os.getpid()))
                time.sleep(float(spec["hang_s"]))
            return (spec.get("map") or {}).get(str(pid), -1)
        return fn
    return resolve


def main():
    gate = os.environ[GATE_ENV]
    sys.path.insert(0, os.path.dirname(gate))
    spec = importlib.util.spec_from_file_location("g", gate)
    g = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(g)
    text = os.environ.get(SPEC_ENV)
    if text and hasattr(g, "_product_cpu_collect"):
        g._product_cpu_collect = functools.partial(g._product_cpu_collect, resolve=make_resolver(text))
        g.__file__ = os.path.abspath(__file__)          # the report child is this wrapper again
    # The program entry of the gate, reproduced (javis_resource_gate.py, `if __name__ == "__main__"`).
    if "--self-test" in sys.argv[1:]:
        sys.exit(g.self_test())
    rc = g.main()
    if (hasattr(g, "_pc_text_check_argv") and rc in (g.EXIT_ALLOW, g.EXIT_SOFT, g.EXIT_HARD)
            and g._pc_text_check_argv(sys.argv[1:])):
        g._product_cpu_tail()
    sys.exit(rc)


if __name__ == "__main__":
    main()
