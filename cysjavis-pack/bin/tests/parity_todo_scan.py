#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""parity_todo_scan.py — todo **스캔 집합**의 2언어 차등 하네스(설계 §14 S18 · W14).

Python 소비자 C1(`javis_report.discover_todo_files`)과 Rust 소비자 C2(`cys::todo_scan` →
`cysd` 워치독)가 **같은 임시 트리**에서 **같은 파일 집합**을 보는지 기계 비교한다.
불일치가 1건이라도 있으면 exit 1이다.

왜 이게 있어야 하나
    두 소비자의 배제 **정책**은 이미 같았다(`todo_is_countable` ↔ `classify_files`). 그런데
    **스캔 루트**가 달랐다 — C1은 `pack/round`를 보고 C2는 안 봤다. 그리고 이 조직의 정본 todo
    위치가 바로 `${CYS_PACK_DIR}/round/` 다. 그래서 데몬에 배선한 선언 판정·유령 배제·
    verdict/owner payload가 **정본 todo에는 한 번도 적용되지 않았다**(S18). 정책 리뷰로는 절대
    못 잡는다 — "무엇을 보는가"를 같은 입력으로 대조하는 기계만이 잡는다.

계약 표면 — 대조하는 것은 정확히 둘이다
    ① **스캔 루트 집합**(공통 3종: `pack/round` · 각 surface `cwd/_round` · `CYS_TODO_DIRS`)
    ② 그 루트들에서 발견되는 **파일 집합**(정규경로 기준 · 비재귀 · 일반 파일만)

정직한 비대칭 1건(대조 대상에서 제외 — 숨기지 않고 명시한다)
    Python 보고기는 `cys status --json`의 `live_cwd`(현재 cd 위치)와 spawn-time `cwd`를 **둘 다**
    루트로 넣는다. 데몬은 `live_cwd`를 상태로 보관하지 않으므로(요청 시점 sysinfo 조회) surface
    루트는 spawn-time cwd뿐이다 = Python이 상위집합이다. 또 `--extra-dir`는 CLI 전용 입력이라
    데몬에 대응물이 없다. 이 하네스는 **양쪽이 공통으로 책임지는 3종 루트**만 투입한다.

실행
    python3 cysjavis-pack/bin/tests/parity_todo_scan.py
    python3 cysjavis-pack/bin/tests/parity_todo_scan.py --keep   # 임시 트리 보존(디버그)
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))                 # …/cysjavis-pack/bin/tests
BIN = os.path.dirname(SELF)                                       # …/cysjavis-pack/bin
REPO = os.path.dirname(os.path.dirname(BIN))                      # 저장소 루트
RUST_SRC = os.path.join(REPO, "src", "todo_scan.rs")
DUMPER_SRC = os.path.join(REPO, "scripts", "todo_scan_dump.rs")

sys.path.insert(0, BIN)


def die(msg):
    """hard fail. 조용한 skip은 게이트를 껍데기로 만든다(파리티 하네스 공통 규약)."""
    sys.stderr.write("parity_todo_scan: %s\n" % msg)
    raise SystemExit(1)


def find_rustc():
    cand = os.environ.get("CYS_RUSTC") or shutil.which("rustc") \
        or os.path.expanduser("~/.cargo/bin/rustc")
    if not (os.path.exists(cand) if cand and os.path.sep in str(cand) else shutil.which(cand or "")):
        die("rustc 를 찾을 수 없다(PATH · ~/.cargo/bin · CYS_RUSTC). "
            "Rust 측을 못 돌리면 파리티는 성립하지 않는다")
    return cand


def build_dumper(workdir):
    for p in (RUST_SRC, DUMPER_SRC):
        if not os.path.isfile(p):
            die("Rust 소스가 없다: %s" % p)
    binpath = os.path.join(workdir, "todo_scan_dump")
    proc = subprocess.run([find_rustc(), "--edition", "2021", "-C", "debuginfo=0",
                           DUMPER_SRC, "-o", binpath], capture_output=True, text=True)
    if proc.returncode != 0:
        die("Rust 덤퍼 컴파일 실패:\n%s%s" % (proc.stdout, proc.stderr))
    return binpath


def build_tree(root):
    """양쪽에 그대로 먹일 임시 트리. 각 자리에 '보여야 하는 것'과 '보이면 안 되는 것'을 함께 둔다.

    반환 = (pack_dir, [surface cwd …], CYS_TODO_DIRS 값, 기대 파일 basename 집합)
    """
    pack = os.path.join(root, ".cys", "pack")
    pack_round = os.path.join(pack, "round")
    cwd_a = os.path.join(root, "work", "proj-a")
    cwd_b = os.path.join(root, "work", "proj-b")
    extra = os.path.join(root, "extra-dir")
    for d in (pack_round, os.path.join(cwd_a, "_round"),
              os.path.join(cwd_b, "_round"), extra):
        os.makedirs(d, exist_ok=True)

    expect = set()

    def todo(d, name):
        with open(os.path.join(d, name), "w", encoding="utf-8") as f:
            f.write("# %s\n- [ ] a\n" % name)
        expect.add(name)

    # ① 정본 위치 — S18의 정확한 자리다.
    todo(pack_round, "WORKER_TODO.md")
    todo(pack_round, "MASTER_TODO.md")
    # ② surface cwd/_round
    todo(os.path.join(cwd_a, "_round"), "CSO_TODO.md")
    todo(os.path.join(cwd_b, "_round"), "REVIEWER-GEMINI_TODO.md")
    # ③ CYS_TODO_DIRS(디렉터리 자체가 루트다 — `_round`를 덧붙이지 않는다)
    todo(extra, "WORKER_2_TODO.md")

    # 보이면 안 되는 것들 — 양쪽이 **똑같이** 걸러야 한다.
    with open(os.path.join(pack_round, "NOTES.md"), "w") as f:
        f.write("todo 가 아니다\n")                       # 접미 불일치
    os.makedirs(os.path.join(pack_round, "DIR_TODO.md"), exist_ok=True)  # 디렉터리
    os.makedirs(os.path.join(pack_round, "nested"), exist_ok=True)
    with open(os.path.join(pack_round, "nested", "DEEP_TODO.md"), "w") as f:
        f.write("- [ ] a\n")                              # 비재귀 경계
    os.symlink(os.path.join(root, "does-not-exist"),
               os.path.join(cwd_a, "_round", "BROKEN_TODO.md"))          # 깨진 심링크
    # 같은 실체 파일이 두 루트로 보이는 경우 — 정규경로 기준 1건이어야 한다.
    os.symlink(os.path.join(pack_round, "WORKER_TODO.md"),
               os.path.join(extra, "WORKER_TODO.md"))

    return pack, [cwd_a, cwd_b], extra, expect


def python_side(pack, cwds, todo_dirs):
    """C1 정본으로 스캔 집합을 낸다. env·status 는 데몬 쪽 입력과 동일 의미로 맞춘다."""
    os.environ["CYS_PACK_DIR"] = pack
    os.environ["CYS_TODO_DIRS"] = todo_dirs
    for stale in ("JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        os.environ.pop(stale, None)
    import javis_report as RP                                     # noqa: E402
    # 데몬이 보관하는 것은 spawn-time cwd 뿐이므로 live_cwd 는 넣지 않는다(위 '정직한 비대칭').
    status = {"surfaces": [{"role": "r%d" % i, "cwd": c} for i, c in enumerate(cwds)]}
    return set(RP.discover_todo_files(status, []))


def rust_side(binpath, pack, cwds, todo_dirs):
    env = dict(os.environ)
    env["CYS_TODO_DIRS"] = todo_dirs
    cmd = [binpath, "--pack", pack]
    for c in cwds:
        cmd += ["--cwd", c]
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if proc.returncode != 0:
        die("Rust 덤퍼 실행 실패(exit=%d):\n%s" % (proc.returncode, proc.stderr))
    roots, files = set(), set()
    for line in proc.stdout.splitlines():
        kind, _, value = line.partition("\t")
        if kind == "root":
            roots.add(value)
        elif kind == "file":
            files.add(value)
        elif line:
            die("덤퍼 출력 형식 위반: %r" % line)
    return roots, files


def report(label, py, rs):
    only_py, only_rs = sorted(py - rs), sorted(rs - py)
    if not only_py and not only_rs:
        return 0
    sys.stderr.write("불일치 [%s]\n" % label)
    for p in only_py:
        sys.stderr.write("    py에만: %s\n" % p)
    for p in only_rs:
        sys.stderr.write("    rs에만: %s\n" % p)
    return len(only_py) + len(only_rs)


def main():
    ap = argparse.ArgumentParser(description="todo 스캔 집합 2언어 차등 하네스")
    ap.add_argument("--keep", action="store_true", help="임시 트리를 지우지 않는다(디버그)")
    args = ap.parse_args()

    workdir = tempfile.mkdtemp(prefix="todo-scan-parity-")
    try:
        binpath = build_dumper(workdir)
        tree = os.path.join(workdir, "tree")
        os.makedirs(tree)
        pack, cwds, extra, expect = build_tree(tree)

        py_files = python_side(pack, cwds, extra)
        rs_roots, rs_files = rust_side(binpath, pack, cwds, extra)

        bad = report("파일 집합", py_files, rs_files)

        # 루트 대조 — 파일 집합이 우연히 같아도 루트 규칙이 갈려 있으면 다음 배치에서 터진다.
        py_roots = {os.path.join(pack, "round")}
        py_roots |= {os.path.join(c, "_round") for c in cwds}
        py_roots |= {extra}
        bad += report("스캔 루트", py_roots, rs_roots)

        # 절대 기준(SOT) — 두 언어가 똑같이 틀리면 상호 대조만으로는 통과한다.
        got = {os.path.basename(p) for p in py_files}
        if got != expect:
            sys.stderr.write("불일치 [기대 집합] py=%s expect=%s\n"
                             % (sorted(got), sorted(expect)))
            bad += 1
        got_rs = {os.path.basename(p) for p in rs_files}
        if got_rs != expect:
            sys.stderr.write("불일치 [기대 집합] rs=%s expect=%s\n"
                             % (sorted(got_rs), sorted(expect)))
            bad += 1

        if bad:
            sys.stderr.write("\nFAIL 스캔 집합 파리티 불일치 — %d건\n" % bad)
            return 1
        print("OK 스캔 집합 일치 — 루트 %d종 · 파일 %d종 (%s)"
              % (len(rs_roots), len(rs_files),
                 ", ".join(sorted(os.path.basename(p) for p in rs_files))))
        return 0
    finally:
        if args.keep:
            sys.stderr.write("임시 트리 보존: %s\n" % workdir)
        else:
            shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
