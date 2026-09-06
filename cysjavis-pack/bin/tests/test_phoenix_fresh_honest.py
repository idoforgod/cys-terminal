#!/usr/bin/env python3
"""★F-1(0.14.31): fresh 예상 판정·Rust 경로 치환 패리티(데몬·네트워크 불요).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_fresh_honest.py (0=전건 PASS)
"""
import importlib.util, os, shutil, sys, tempfile

# ★F-1(0.14.31): 기존 시나리오 하네스처럼 모듈만 적재(바이트코드 파일 생성 방지).
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

# ★F-1(0.14.31): 항목별 PASS/FAIL 을 모아 전건 통과일 때만 exit 0.
_results = []
def check(name, cond):
    _results.append(cond); print(("PASS " if cond else "FAIL ") + name)


# ★F-1(0.14.31): 임시 세션 파일로 존재/부재를 대조하고 반드시 정리한다.
def main():
    td = tempfile.mkdtemp(prefix="phoenix-fresh-")
    try:
        check("munge Rust /Users example", m._claude_project_component("/Users/user/Desktop/ProjX") == "-Users-user-Desktop-ProjX")
        check("munge Rust /tmp example", m._claude_project_component("/tmp/a.b_c") == "-tmp-a-b-c")
        check("munge empty/None", m._claude_project_component("") == m._claude_project_component(None) == "")
        check("munge ASCII only", m._claude_project_component("A-z09/한é_") == "A-z09----")
        check("claude empty sid", m.fresh_expected({"agent": "claude", "session_id": ""}) == (True, "no_session"))
        check("gemini empty sid", m.fresh_expected({"agent": "gemini", "session_id": ""}) == (False, ""))
        check("codex empty sid", m.fresh_expected({"agent": "codex", "session_id": ""}) == (False, ""))
        entry = {"agent": "claude", "session_id": "sid", "claude_config_dir": td, "cwd": "/tmp/a.b_c"}
        check("claude missing session file", m.fresh_expected(entry) == (True, "no_session_file"))
        path = os.path.join(td, "projects", "-tmp-a-b-c", "sid.jsonl")
        os.makedirs(os.path.dirname(path))
        with open(path, "w", encoding="utf-8") as f:
            f.write("{}\n")
        check("claude existing session file", m.fresh_expected(entry) == (False, ""))
        check("claude missing cwd", m.fresh_expected({k: v for k, v in entry.items() if k != "cwd"}) == (False, ""))
        check("claude missing cfg", m.fresh_expected({k: v for k, v in entry.items() if k != "claude_config_dir"}) == (False, ""))
        check("claude whitespace sid", m.fresh_expected({"agent": "claude", "session_id": " \t "}) == (True, "no_session"))
        check("default claude", m.fresh_expected({}) == (True, "no_session"))
    finally:
        shutil.rmtree(td)
    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
