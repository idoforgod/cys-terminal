#!/usr/bin/env python3
"""C8 dormancy census for 0.14.52 (HEAD-RULING-C8-SCOPE-0.14.52): compiler-verified production references.

Predicate: a non-test item of the cysd/cys binaries that stops compiling when the subject is made #[cfg(test)]
(`cargo check --offline [--target T] --bin cysd --bin cys`). Subjects: the C8 modules, the Surface::receiver_proof field,
the Surface constructor, the role-change finalize step, and the partial landings D3 decline_unstarted and D4
Plan::append_owned_submit. Each subject is gated on its own line (line numbers unchanged), checked, and restored
byte-identically. Output is the population file that handlers::tests::c8_dormancy_* reads (one list, two instruments).

Usage (from the repository root; uses the caller's cargo env, including CARGO_TARGET_DIR):
  python3 scripts/c8_dormancy_census.py [--write | --check] [--target <triple>]
  python3 scripts/c8_dormancy_census.py --self-test        (field_use classification only; no cargo)
The script never edits the tree it is started in (R-C8DORM-1 #4): it clones HEAD into a temporary directory
(`git clone --shared`), gates and checks there, and removes the clone. Uncommitted changes are refused, because they
would not be in the clone. --write puts the population into this tree; --check compares the body with the committed
population and exits 1 on any difference (the "measured at" line is not compared).
Field-use classes: a struct-literal init to Mutex::new(None) is init-none; any other init, struct-field shorthand, an
assignment, a mutating call (.replace .take .insert .get_or_insert[_with] .swap .set) or std::mem::replace/swap/take on
the field is a write; everything else is a read (R-C8DORM-1-G #2). A write through a binding of the guard
(`let mut g = s.receiver_proof.lock().unwrap(); *g = ..`) is classed read here; the lexical pin reports any new statement.
Not covered: references produced by macros from strings, build-script output, or code outside the two binaries; code
under a cfg that is off for the checked target (the default is the host - run again with --target for another
platform; the lexical pin c8_dormancy_receiver_proof_has_only_the_known_production_shapes reads every cfg).
"""
import re, shutil, subprocess, sys, tempfile
from pathlib import Path
sys.dont_write_bytecode = True
ROOT = Path.cwd(); POPULATION = "src/bin/cysd/c8_dormancy_population.txt"
MODULES = ["receiver", "native_publication", "pending_requests", "policy_loader", "rpc_receipt", "writer_gate"]
SUBJECTS = [  # kind, file, exact line prefix that is gated
    ("field", "src/bin/cysd/state.rs", "    pub receiver_proof: Mutex<Option<Arc<dyn crate::agent_write::Proof>>>,"),
    ("constructor", "src/bin/cysd/state.rs", "    pub fn create_surface_with_env("),
    ("role_finalize", "src/bin/cysd/handlers.rs", "fn finalize_agent_meta_after_role_change("),
    ("partial decline_unstarted", "src/bin/cysd/agent_write.rs", "pub(crate) fn decline_unstarted("),
    ("partial append_owned_submit", "src/bin/cysd/agent_write.rs", "    pub(crate) fn append_owned_submit("),
] + [("module " + m, "src/bin/cysd/main.rs", "mod %s;" % m) for m in MODULES]
LIMITS = [  # written into the population header (R-C8DORM-1 #7)
    "# limit: cargo check ran for {target}; code under a cfg that is off there was not compiled (the lexical pin reads every cfg)",
    "# limit: a static file - it pins the census at the commit above; a later change to these sites needs this script re-run",
    "# limit: CI does not re-run the census; CI runs the PassThrough test and the receiver_proof lexical pin",
    "# limit: references built by macros from strings, build-script output and code outside the cysd/cys binaries are not seen",
    "# limit: the positive control covered the classes it planted (constructor init, finalize assignment, a partial item, a C8 module)",
]
MUTATING = re.compile(r"\.\s*(?:replace|take|insert|get_or_insert(?:_with)?|swap|set)\s*\(")
MEM_CALL = re.compile(r"mem\s*::\s*(?:replace|swap|take)\s*\(")

def enclosing_fn(lines, index):
    for i in range(index, -1, -1):
        m = re.match(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)", lines[i])
        if m:
            return m.group(1)
    return "-"

def field_use(lines, index):
    line = lines[index]; at = line.find("receiver_proof"); text = line[at:]
    if re.match(r"receiver_proof\s*:(?!:)", text):
        return "init-none" if re.match(r"receiver_proof\s*:\s*Mutex::new\(None\)", text) else "write"
    if re.match(r"receiver_proof\s*[,}]", text):
        return "write"                                          # struct-field shorthand: the value is a binding
    statement = " ".join([text] + lines[index + 1:index + 6]).split(";")[0]
    if re.search(r"(?<![=!<>])=(?![=>])", statement) or MUTATING.search(statement) or MEM_CALL.search(line[:at]):
        return "write"
    return "read"

def self_test():
    cases = [
        ("        let proof_class = if s.receiver_proof.lock().unwrap().is_some() { \"bound\" } else { \"unbound\" };", "read"),
        ("        self.receiver_proof", "read"),
        ("            receiver_proof: Mutex::new(None),", "init-none"),
        ("            receiver_proof: Mutex::new(Some(p)),", "write"),
        ("            receiver_proof,", "write"),
        ("    *s.receiver_proof.lock().unwrap() = Some(p);", "write"),
        ("    s.receiver_proof.lock().unwrap().replace(p);", "write"),
        ("    let old = s.receiver_proof.lock().unwrap().take();", "write"),
        ("    std::mem::replace(&mut *s.receiver_proof.lock().unwrap(), Some(p));", "write"),
        ("    if s.receiver_proof.lock().unwrap().is_some() == expected { }", "read"),
    ]
    bad = [(src, want, field_use([src, ";"], 0)) for src, want in cases if field_use([src, ";"], 0) != want]
    for src, want, got in bad:
        print("SELF-TEST FAIL want %s got %s: %s" % (want, got, src.strip()))
    print("self-test: %d cases, %d failed" % (len(cases), len(bad)))
    return 1 if bad else 0

def census(tree, kind, rel, prefix, target):
    path = tree / rel; original = path.read_bytes(); lines = original.decode().split("\n")
    hits = [i for i, l in enumerate(lines) if l.startswith(prefix)]
    if len(hits) != 1:
        raise SystemExit("census subject %r: expected one line starting %r in %s, found %d" % (kind, prefix, rel, len(hits)))
    lines[hits[0]] = "#[cfg(test)] " + lines[hits[0]].lstrip() if kind.startswith("module") else \
        lines[hits[0]][:len(lines[hits[0]]) - len(lines[hits[0]].lstrip())] + "#[cfg(test)] " + lines[hits[0]].lstrip()
    cmd = ["cargo", "check", "--offline"] + (["--target", target] if target else []) + \
          ["--bin", "cysd", "--bin", "cys", "--message-format=short"]
    try:
        path.write_text("\n".join(lines))
        run = subprocess.run(cmd, cwd=tree, capture_output=True, text=True)
    finally:
        path.write_bytes(original)
    if path.read_bytes() != original:
        raise SystemExit("restore failed: " + rel)
    text = run.stdout + run.stderr
    if run.returncode == 0:
        return []
    sites = sorted(set(re.findall(r"^(src/[^:\s]+):(\d+):\d+: error", text, re.M)))
    if not sites:
        raise SystemExit("census subject %r: cargo failed with no attributable error site:\n%s" % (kind, text[-2000:]))
    rows = []
    for f, n in sites:
        src = (tree / f).read_text().split("\n"); i = int(n) - 1
        use = field_use(src, i) if kind == "field" else "ref"
        rows.append("%s %s %s:%s %s" % (kind, use, f, n, enclosing_fn(src, i)))
    return rows

def git(*args, cwd=ROOT):
    return subprocess.run(["git"] + list(args), cwd=cwd, capture_output=True, text=True)

def main():
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    target = args[args.index("--target") + 1] if "--target" in args else None
    if git("status", "--porcelain", "--untracked-files=no").stdout.strip():
        raise SystemExit("refused: uncommitted changes in %s would not be measured (the census runs on a clone of HEAD)" % ROOT)
    pin = git("rev-parse", "HEAD").stdout.strip()
    scratch = Path(tempfile.mkdtemp(prefix="c8-dormancy-census-")); clone = scratch / "tree"
    try:
        for step in (["clone", "--quiet", "--shared", str(ROOT), str(clone)], ["-C", str(clone), "checkout", "--quiet", "--detach", pin]):
            run = git(*step)
            if run.returncode != 0:
                raise SystemExit("refused: could not prepare the census clone: %s" % run.stderr.strip())
        rows = ["# C8 dormancy census population - generated by scripts/c8_dormancy_census.py; do not edit by hand",
                "# measured at commit %s (the committing change is not included); predicate: see the script docstring" % pin]
        rows += [l.format(target=target or "the host") for l in LIMITS]
        for kind, rel, prefix in SUBJECTS:
            found = census(clone, kind, rel, prefix, target)
            rows += found or ["%s none" % kind]
    finally:
        if scratch.parent == Path(tempfile.gettempdir()) and scratch.name.startswith("c8-dormancy-census-"):
            shutil.rmtree(scratch)
    text = "\n".join(rows) + "\n"
    if "--check" in args:
        body = lambda t: [l for l in t.splitlines() if not l.startswith("# measured at commit")]
        committed = (ROOT / POPULATION).read_text()
        if body(committed) != body(text):
            sys.stdout.write(text)
            print("CHECK FAILED: the census output differs from %s" % POPULATION, file=sys.stderr)
            return 1
        print("CHECK OK: census output equals %s (measured-at line excluded)" % POPULATION)
        return 0
    if "--write" in args:
        (ROOT / POPULATION).write_text(text)
    sys.stdout.write(text)
    return 0

if __name__ == "__main__":
    sys.exit(main())
