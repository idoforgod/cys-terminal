#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""nsis-hook placement state-machine model — exhaustive branch exploration (W7).

WHAT THIS IS
  A faithful, executable Python mirror of the PLACEMENT state machine in
  src-tauri/nsis-hooks.nsh (the W4 lock-tolerant placement redesign), explored
  exhaustively over injected filesystem failures and interruptions, with the
  hook's frozen invariants asserted mechanically at every terminal state.

  There is no Windows machine (and no Rosetta/Wine substrate) on the dev Mac, so
  the real installer cannot be EXECUTED here — only compiled
  (scripts/tests/nsis-hook-compile/).  This model raises the evidence level for
  the hook's *logic*: every branch of every placement macro is walked, including
  the deep failure lanes no manual test reaches.

FIDELITY CONTRACT (read this before editing the hook)
  * Each mirror function below cites the hook ANCHORS it models (macro names and
    label families such as cys_pl_need_ / cys_ld_bad_ — never line numbers; the
    same rule as harness N6).  If you change a macro body, change its mirror in
    the same commit.
  * A guard at startup (`hook_guard()`) re-derives the hook's macro/anchor/token
    census PLUS a normalized (comment/whitespace-stripped) hash of every modeled
    macro body, the rescue callbacks, and the POSTINSTALL !insertmacro order,
    and compares it to the PIN below.  If the hook drifts anywhere — even a
    body-only edit that keeps every label — and the model was not touched, this
    script FAILS (exit 2).  Re-pin only after updating the mirrors:
    CYS_MODEL_REPIN=1 python3 model.py
  * Frozen vocabulary (NSIS-CONTRACT.md §3/§4/§5) is asserted verbatim.

WHAT IS MODELED / NOT MODELED
  modeled:   PREINSTALL taskkill(GUI)+unlock-sweep, template File extraction
             (SetOverwrite try semantics incl. truncate-write tearing),
             POSTINSTALL 3-binary transaction (CYS_PLACE ①..⑧, CYS_RESTORE_SLOT,
             CYS_UNPLACE, CYS_SLOT_CLEANUP, CYS_LASTDITCH), failure classification
             (exit 3/4 + 4-token failure file), Abort→.onInstFailed, user cancel
             →MUI .onUserAbort → cys_on_user_abort (CYS_ABORT_RESCUE), hard kill (no callback), NTFS lock
             classes (running image: rename OK/delete·overwrite DENIED — L2;
             share-read handle: read OK/rename·delete·write DENIED — CONTRACT P6;
             share-none: everything incl. probes DENIED — CONTRACT §9-6).
  not modeled: the installer-singleton mutex (single instance assumed; exit 5 is
             untouched-quit by construction), PREUNINSTALL, desktop shortcut,
             registry, the cysd boot guard (its INPUTS — on-disk material — are
             what the invariants protect), and compile-time gates (S5 "sidecar
             without VERSIONINFO" is a build failure by !error; the runtime path
             is unreachable — proven by harness negative control N1).

INVARIANTS (mechanical form of hook R1/R3/R4 with the honest scopes of
NSIS-CONTRACT §9 — the model refuses to over-claim):
  I1  No terminal state loses a canonical silently:
      - exit 0  ⇒ every canonical exists and IS this build (kind NEW).
      - canonical absent ⇒ only on exit 3 (named in `unrecoverable:`) or on
        cancel/kill terminals; and if that binary started with a working build,
        a working copy (OLD or NEW) still exists on disk in its name family
        (boot-guard material — the "무음 소실 경로 없음" clause of §9-1).
      - canonical truncated ⇒ never on exit 0/4; on exit 3 it is named loudly;
        on cancel/kill it is the deliberate loud Hold state (R2-r2: a truncated
        canonical is left for the boot guard rather than silently vacated).
  I3  Placement prefix (hook scope — CONTRACT §2 스코프 정정):
      - the set of binaries the TRANSACTION placed and kept is always a prefix
        of [cys, cysd, cys-app] on completed runs;
      - ANY old/new generation split among canonicals (template- or
        emergency-promotion-made — §9-3/§9-5) is LOUD: never on exit 0.
  I4  Every refusal is loud and the old build survives:
      - completed failing runs write the failure file with the frozen 4-token
        schema and exit 3/4 per the frozen classification (`unrecoverable:` or
        `rolled-back-to-previous:` nonempty ⇒ 3, else 4);
      - every `placement-refused:` reason is from the frozen reason-code set;
      - for every refused binary that started with a working old build, an OLD
        copy still exists on disk;
      - the success marker is written only on exit 0.

RUN
  python3 scripts/tests/nsis-hook-model/model.py            # exit 0, prints census
  CYS_MODEL_FAULTS=4 python3 .../model.py                   # deeper fault budget
  env: CYS_MODEL_FAULTS (default 3)  fault-injection budget per run
       CYS_MODEL_INT_FAULT_CAP (default 3)  offer interrupts only while
                                            faults-used <= cap (bounds the tree)
       CYS_MODEL_REPIN=1  print the new guard pin instead of failing
"""

import hashlib
import os
import re
import sys

BINS = ("cys", "cysd", "cys-app")  # fixed transaction order — hook R3
SIZE_FLOOR = 65536                  # the hook's working-executable floor (R1-r1)

# frozen reason codes — NSIS-CONTRACT §3 (placement-refused line)
REASONS = ("stale-new-locked", "extract-failed", "new-bad",
           "vacate-locked", "fill-failed", "reverify-failed")

FAULT_BUDGET = int(os.environ.get("CYS_MODEL_FAULTS", "4"))
INT_FAULT_CAP = int(os.environ.get("CYS_MODEL_INT_FAULT_CAP", "3"))

# ═════════════════════════════════════════════════════════════════════════════
# hook drift guard — the macro/anchor/token census of src-tauri/nsis-hooks.nsh
# ═════════════════════════════════════════════════════════════════════════════

EXPECTED_MACROS = {
    "CYS_RENAME_RETRY", "CYS_ORACLE", "CYS_RESTORE_SLOT", "CYS_PLACE",
    "CYS_UNPLACE", "CYS_SLOT_CLEANUP", "CYS_LASTDITCH", "CYS_ABORT_RESCUE",
    "NSIS_HOOK_PREINSTALL", "NSIS_HOOK_POSTINSTALL", "NSIS_HOOK_PREUNINSTALL",
}
# sha256 over the sorted census the mirrors depend on: anchors/tokens + the
# normalized BODY hashes of the modeled macros/callbacks + POSTINSTALL order
# (see hook_guard)
GUARD_PIN = "a0f620836f70fb30e6a2065f6b7afb9d74f3247562d470df37c95085572cc5be"


def hook_path():
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.normpath(os.path.join(here, "..", "..", "..",
                                         "src-tauri", "nsis-hooks.nsh"))


def hook_guard():
    p = hook_path()
    try:
        with open(p, "r", encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        print("FAIL[guard]: cannot read the hook at %s: %s" % (p, e), file=sys.stderr)
        sys.exit(2)

    macros = set(re.findall(r"!macro\s+([A-Z_][A-Z0-9_]+)", text))
    if macros != EXPECTED_MACROS:
        print("FAIL[guard]: hook macro set changed.\n"
              "  new since pin : %s\n  gone since pin: %s\n"
              "  The model mirrors these macros branch-by-branch — update the "
              "mirrors in scripts/tests/nsis-hook-model/model.py in the SAME "
              "commit, then re-pin (CYS_MODEL_REPIN=1)."
              % (sorted(macros - EXPECTED_MACROS), sorted(EXPECTED_MACROS - macros)),
              file=sys.stderr)
        sys.exit(2)

    # census: label families + frozen vocabulary the mirrors rely on
    census = set()
    census.update("macro:" + m for m in macros)
    census.update("label:" + l for l in re.findall(
        r"\b(cys_(?:rr|or|rs|pl|up|sc|ld|ar|pre|txn|post)_[a-z0-9]+)_\$?\{?", text))
    census.update("reason:" + r for r in REASONS if ("(%s)" % r) in text or r in text)
    for tok in ("unrecoverable:", "rolled-back-to-previous:", "not-updated:",
                "placement-refused:", ".onInstFailed", "cys_on_user_abort",
                "MUI_CUSTOMFUNCTION_ABORT",
                "SetErrorLevel 3", "SetErrorLevel 4", "SetErrorLevel 5",
                "cys-install-failure.txt", "cys-installed-version.txt"):
        if tok in text:
            census.add("tok:" + tok)

    # body bind (W4 review): names/anchors above cannot see a body edit that
    # keeps every label — e.g. dropping the CYS_UNPLACE undo calls from
    # cys_txn_undo, or adding a premature Delete at the CYS_PLACE ⑧ commit.
    # Pin a normalized (comment/whitespace-stripped) hash of every modeled
    # macro body + the rescue callbacks, and the POSTINSTALL !insertmacro
    # order, so ANY body edit forces a conscious mirror update + re-pin.
    def norm(block):
        out = []
        for raw in block.splitlines():
            cut, q = len(raw), False
            for i, c in enumerate(raw):
                if c == '"':
                    q = not q
                elif c in ";#" and not q:
                    cut = i
                    break
            line = " ".join(raw[:cut].split())
            if line:
                out.append(line)
        return "\n".join(out)

    bodies = dict(re.findall(r"(?ms)^!macro[ \t]+(\S+)[^\n]*\n(.*?)^!macroend",
                             text))
    bodies.update(re.findall(
        r"(?ms)^Function[ \t]+(\S+)[^\n]*\n(.*?)^FunctionEnd", text))
    for name in sorted((EXPECTED_MACROS - {"NSIS_HOOK_PREUNINSTALL"})
                       | {".onInstFailed", "cys_on_user_abort"}):
        if name not in bodies:
            print("FAIL[guard]: cannot extract the body of %s from the hook "
                  "— the body bind needs it (block moved or renamed?)" % name,
                  file=sys.stderr)
            sys.exit(2)
        census.add("body:%s:%s" % (
            name, hashlib.sha256(norm(bodies[name]).encode()).hexdigest()))
    census.add("seq:POSTINSTALL:" + ">".join(re.findall(
        r"!insertmacro[ \t]+(\S+)", norm(bodies["NSIS_HOOK_POSTINSTALL"]))))

    digest = hashlib.sha256("\n".join(sorted(census)).encode()).hexdigest()
    if os.environ.get("CYS_MODEL_REPIN") == "1":
        print("GUARD_PIN = \"%s\"" % digest)
        sys.exit(0)
    if digest != GUARD_PIN:
        print("FAIL[guard]: hook census drifted (pin mismatch) — anchors, "
              "tokens, a modeled macro/callback BODY, or the POSTINSTALL "
              "!insertmacro order changed.\n"
              "  pinned  : %s\n  computed: %s\n"
              "  A hook edit touched material the model mirrors. Update the "
              "mirrors here in the SAME commit, then re-pin with "
              "CYS_MODEL_REPIN=1 python3 model.py" % (GUARD_PIN, digest),
              file=sys.stderr)
        sys.exit(2)


# ═════════════════════════════════════════════════════════════════════════════
# exhaustive DFS chooser (stateless model checking over decision sequences)
# ═════════════════════════════════════════════════════════════════════════════

class Interrupted(Exception):
    def __init__(self, mode):  # "cancel" | "kill"
        self.mode = mode


class Chooser:
    def __init__(self, prefix):
        self.prefix = prefix
        self.trail = []  # (n_options, chosen)

    def choose(self, n):
        i = len(self.trail)
        c = self.prefix[i] if i < len(self.prefix) else 0
        self.trail.append((n, c))
        return c


def explore(run_fn):
    """Enumerate the whole decision tree; yields one terminal per run."""
    stack = [[]]
    while stack:
        prefix = stack.pop()
        ch = Chooser(prefix)
        yield run_fn(ch), ch
        for i in range(len(ch.trail) - 1, len(prefix) - 1, -1):
            n, c = ch.trail[i]
            for alt in range(c + 1, n):
                stack.append([t[1] for t in ch.trail[:i]] + [alt])


# ═════════════════════════════════════════════════════════════════════════════
# filesystem + lock semantics (NTFS via the hook's L2 + CONTRACT P6/§9-6)
# ═════════════════════════════════════════════════════════════════════════════
# entity: {"kind": OLD|NEW|NEWBAD|TRUNC, "lock": None|image|shareread|sharenone,
#          "proc": None|gui|..., "arrival": str}
#   OLD    previous working build (>=64KiB; VERSIONINFO may be absent — oracle
#          reads it as notfresh, never fresh)
#   NEW    this build (oracle DWORDs match the compile constants)
#   NEWBAD size-passing torn/corrupt claim of this build (oracle mismatch)
#   TRUNC  truncated junk below the 64KiB floor
# lock classes:  image      rename OK / delete·overwrite DENIED / read OK   (L2)
#                shareread  rename·delete·overwrite DENIED / read OK       (P6)
#                sharenone  everything DENIED incl. open-for-read       (§9-6)


class Run:
    def __init__(self, scen, chooser):
        self.scen = scen
        self.ch = chooser
        self.files = {}
        for name, kind, lock, proc in scen["files"]:
            self.files[name] = {"kind": kind, "lock": lock, "proc": proc,
                                "arrival": "start"}
        self.faults_left = FAULT_BUDGET
        self.faults_used = 0
        self.int_left = 1
        self.sticky = set()      # fault labels that stay failed for the run
        self.R = {"refused": [], "unrec": [], "rolled": [], "notupd": []}
        self.reason = {}         # bin -> refusal reason code
        self.slots = {b: None for b in BINS}
        self.txn_placed = {b: False for b in BINS}   # ⑧ commit reached
        self.attempted = {b: False for b in BINS}    # oracle gate said 'needs placement'
        self.undone = {b: False for b in BINS}       # UNPLACE restored old
        self.torn_tpl = set()    # canonicals torn by an interrupted template write
        self.failfile = False
        self.marker = False
        self.exit = None         # 0|3|4|"cancel"|"kill"
        self.events = []

    # ── decision helpers ────────────────────────────────────────────────────
    def fault(self, label):
        if label in self.sticky:
            return True
        if self.faults_left <= 0:
            return False
        if self.ch.choose(2) == 1:
            self.faults_left -= 1
            self.faults_used += 1
            self.sticky.add(label)
            self.events.append("FAULT " + label)
            return True
        return False

    def fault_variant(self, label, n_bad):
        """0 = ok; 1..n_bad = distinct failure flavors (each costs budget)."""
        if self.faults_left <= 0:
            return 0
        v = self.ch.choose(n_bad + 1)
        if v:
            self.faults_left -= 1
            self.faults_used += 1
            self.events.append("FAULT %s#%d" % (label, v))
        return v

    def int_point(self, label):
        if self.int_left <= 0 or self.faults_used > INT_FAULT_CAP:
            return
        c = self.ch.choose(3)  # 0 continue, 1 cancel (.onUserAbort), 2 kill
        if c:
            self.int_left -= 1
            self.events.append(("CANCEL@" if c == 1 else "KILL@") + label)
            raise Interrupted("cancel" if c == 1 else "kill")

    # ── primitive ops (NSIS semantics × lock classes) ───────────────────────
    def op_delete(self, n, site):
        e = self.files.get(n)
        if e is None:
            return True                      # NSIS Delete of absent: no error
        if e["lock"] is not None:
            return False                     # in use — Delete sets the error flag
        if self.fault(site):
            return False
        del self.files[n]
        return True

    def op_rename(self, src, dst, site):
        e = self.files.get(src)
        if e is None or dst in self.files:
            return False                     # NSIS Rename: needs src, free dst
        if e["lock"] in ("shareread", "sharenone"):
            return False                     # no FILE_SHARE_DELETE → no rename
        if self.fault(site):                 # transient AV blip (RENAME_RETRY ×5 exhausted)
            return False
        del self.files[src]
        self.files[dst] = e
        return True

    def op_copy(self, src, dst, site):
        s = self.files.get(src)
        if s is None or s["lock"] == "sharenone":
            return False                     # cmd copy must read the source
        d = self.files.get(dst)
        if d is not None and d["lock"] is not None:
            return False                     # cannot overwrite a locked target
        if self.fault(site):
            return False
        self.files[dst] = {"kind": s["kind"], "lock": None, "proc": None,
                           "arrival": "copy"}
        return True

    def write(self, n, kind, arrival):
        e = self.files.get(n)
        if e is not None and e["lock"] is not None:
            return False
        self.files[n] = {"kind": kind, "lock": None, "proc": None,
                         "arrival": arrival}
        return True

    def size_probe(self, n):
        """FileOpen r + FileSeek END.  None = cannot even probe (absent or
        share-none); True/False = size floor verdict."""
        e = self.files.get(n)
        if e is None or e["lock"] == "sharenone":
            return None
        return e["kind"] != "TRUNC"

    def oracle(self, b, site):
        """CYS_ORACLE — absolute freshness oracle (hook R2, fail-closed).
        anchors: cys_or_*  ("fresh"/"notfresh"/"absent")."""
        n = b + ".exe"
        e = self.files.get(n)
        if e is None:
            return "absent"
        if e["lock"] == "sharenone":
            return "notfresh"                # FileOpen fails → fail-closed
        if e["kind"] == "TRUNC":
            return "notfresh"                # < 64KiB floor
        if self.fault(site):
            return "notfresh"                # GetDLLVersion read blip → fail-closed
        return "fresh" if e["kind"] == "NEW" else "notfresh"

    def refuse(self, b, reason):
        assert reason in REASONS, reason
        self.R["refused"].append(b)
        self.reason[b] = reason
        self.events.append("REFUSE %s(%s)" % (b, reason))

    def family(self, b):
        pref = b + "."
        return {n: e for n, e in self.files.items()
                if n == b + ".exe" or n.startswith(pref)}


# ═════════════════════════════════════════════════════════════════════════════
# mirrors of the hook macros (anchors cited per function)
# ═════════════════════════════════════════════════════════════════════════════

def preinstall(run):
    """NSIS_HOOK_PREINSTALL — mirrors: singleton (assumed won), GUI-only
    taskkill (no /T — L1), unlock-sweep (L4/L5: root level, skips the three
    canonicals and *.prev*, moves only LOCKED PE files aside)."""
    for e in run.files.values():             # taskkill /F /IM cys-app.exe
        if e.get("proc") == "gui":
            e["lock"] = None
            e["proc"] = None
    run.int_point("after-taskkill")
    canon = {b + ".exe" for b in BINS}
    for name in sorted(run.files):
        if name in canon or ".prev" in name:
            continue                         # L5: canonicals untouched; leftovers skipped
        if not name.endswith((".exe", ".dll", ".pyd", ".node")):
            continue
        e = run.files[name]
        if e["lock"] is None:
            continue                         # open-rw probe succeeded → not locked
        # locked → try Move to <name>.prev<rand>; image class allows rename,
        # shareread/sharenone deny it (catch {} — silent, stays put)
        run.op_rename(name, name + ".prev4213", "sweep:" + name)


def template_extract(run):
    """The template Section's File extraction under `SetOverwrite try`:
    a locked target is skipped SILENTLY; an unlocked target is truncate-written
    in place (the tear window R2-r2 documents — interruption leaves TRUNC)."""
    for b in BINS:
        n = b + ".exe"
        run.int_point("tpl-pre:" + b)
        e = run.files.get(n)
        if e is not None and e["lock"] is not None:
            continue                         # locked → extraction silently skipped
        if run.fault("tpl-skip:" + b):
            continue                         # write error under `try` → continue
        if run.int_left > 0 and run.faults_used <= INT_FAULT_CAP:
            c = run.ch.choose(3)             # tear the truncate-write mid-flight
            if c:
                run.int_left -= 1
                run.write(n, "TRUNC", "tpl-tear")
                run.torn_tpl.add(b)
                run.events.append(("CANCEL@" if c == 1 else "KILL@") + "tpl-tear:" + b)
                raise Interrupted("cancel" if c == 1 else "kill")
        run.write(n, "NEW", "tpl-extract")


def restore_slot(run, b, slot):
    """CYS_RESTORE_SLOT — prev-slot old build back to the canonical name.
    anchors: cys_rs_*  (rename-retry → cmd copy fallback → delete+rename).
    The copy path RESTORES BUT LEAVES the slot file (copy ≠ move).
    SLOTVAR is cleared in every branch (the hook does the same)."""
    n, s = b + ".exe", "%s.%s.exe" % (b, slot)
    run.slots[b] = None
    if run.op_rename(s, n, "restore-ren:" + b):
        run.files[n]["arrival"] = "restore-old"
        return True
    if run.op_copy(s, n, "restore-copy:" + b):
        run.files[n]["arrival"] = "restore-old"
        return True
    # partial copy cleaned, then one more rename — same underlying blocker as
    # the first (sticky label): the full cascade fails as one lane
    if run.op_rename(s, n, "restore-ren:" + b):
        run.files[n]["arrival"] = "restore-old"
        return True
    run.events.append("RESTORE-FATAL " + b)
    return False


def place(run, b, src_ok=True):
    """CYS_PLACE — lock-tolerant placement, 8 steps (IMPL-SPEC §W4-B).
    anchors: cys_pl_need_ / cys_pl_extract_ / cys_pl_exfail_ / cys_pl_newbad_ /
    cys_pl_vacate_ (s2/s3/stick) / cys_pl_fill_ / cys_pl_rv_ / cys_pl_rvstuck_ /
    cys_pl_commit_.  Returns "ok"/"fail"."""
    n, new = b + ".exe", b + ".new.exe"
    run.slots[b] = None

    def done(res):
        """⑨ 임시 신본(.new.exe) 중앙 정리 — 훅 cys_pl_done_ 의 미러.
        ★조건부: 정식이 존재하고 열리며 크기 하한을 넘을 때만 지운다. 경로마다 정식
        상태가 달라 일괄 삭제는 "빈손 삭제"(유일한 복구 재료 소실)가 될 수 있기 때문이며,
        W1 데몬 스윕의 `.new.exe` 규칙(정식 존재 시에만 삭제)과 같은 자를 쓴다."""
        if new in run.files:
            c = run.files.get(n)
            if c is not None and c["lock"] != "sharenone" and c["kind"] != "TRUNC":
                run.op_delete(new, "d-done:" + b)   # best-effort
        return res
    # ① oracle short-circuit — same-version reinstall / already-extracted path
    if run.oracle(b, "or1:" + b) == "fresh":
        run.op_delete(new, "d-shortcut:" + b)   # best-effort, errors cleared
        return done("ok")
    run.attempted[b] = True
    run.int_point("place:" + b)
    # ② stale .new must be FULLY removed (downgrade trap — see hook comment)
    if not run.op_delete(new, "d-stale:" + b):
        run.refuse(b, "stale-new-locked")
        return done("fail")
    # ③ extract this build to the side name (canonical untouched)
    v = run.fault_variant("extract:" + b, 3)  # 1 write-error, 2 torn<64KiB, 3 torn>=64KiB
    if v == 1:
        run.op_delete(new, "d-exfail:" + b)
        run.refuse(b, "extract-failed")
        return done("fail")
    run.write(new, "NEW" if v == 0 else ("TRUNC" if v == 2 else "NEWBAD"),
              "hook-extract")
    # ④ verify the extract: exists + size floor + oracle DWORDs (fail-closed)
    e = run.files.get(new)
    bad = (e is None or e["lock"] == "sharenone" or e["kind"] != "NEW"
           or run.fault("newprobe:" + b))
    if bad:
        run.op_delete(new, "d-newbad:" + b)   # failure ignored by the hook
        run.refuse(b, "new-bad")
        return done("fail")
    # ⑤ vacate the canonical (old build) to a prev slot — absent → straight to ⑥
    slot = None
    if n in run.files:
        for cand in ("prev", "prev2", "prev3"):
            s = "%s.%s.exe" % (b, cand)
            if not run.op_delete(s, "d-slot:" + s):
                continue                      # slot held by a lame-duck → next
            run.int_point("vacate:" + b + ":" + cand)
            if run.op_rename(n, s, "vacate:" + s):
                slot = cand
                break
        else:
            t = "prev71001"                   # tick slot — always-fresh name
            while ("%s.%s.exe" % (b, t)) in run.files:
                t += "1"
            if run.op_rename(n, "%s.%s.exe" % (b, t), "vacate-tick:" + b):
                slot = t
            else:
                run.refuse(b, "vacate-locked")  # canonical untouched (old intact)
                return done("fail")
        run.slots[b] = slot
    # ⑥ fill: the verified build takes the canonical name (the ms window)
    run.int_point("fill:" + b)
    if not run.op_rename(new, n, "fill:" + b):
        if run.slots[b]:
            restore_slot(run, b, run.slots[b])
        run.refuse(b, "fill-failed")          # .new stays on disk (hook leaves it)
        return done("fail")
    run.files[n]["arrival"] = "txn-fill"
    run.int_point("reverify:" + b)
    # ⑦ re-verify — AV quarantine / fs anomaly window (recheck-fail class)
    if run.fault("corrupt-after-fill:" + b):
        run.files[n]["kind"] = "NEWBAD"       # torn between fill and re-check
    if run.oracle(b, "or2:" + b) != "fresh":
        if not run.op_rename(n, new, "rvv:" + b):
            run.refuse(b, "reverify-failed")  # rvstuck: canonical keeps 'something' (R1)
            return done("fail")
        if run.slots[b]:
            restore_slot(run, b, run.slots[b])
        run.refuse(b, "reverify-failed")
        return done("fail")
    # ⑧ commit — .new already gone via rename; Delete is the belt
    run.op_delete(new, "d-commit:" + b)
    run.txn_placed[b] = True
    return done("ok")


def unplace(run, b):
    """CYS_UNPLACE — transactional undo of one placed binary.
    anchors: cys_up_stuck_ / cys_up_norestore_ / cys_up_keepnew_.
    Returns "ok"/"undofail" (caller stops the undo chain on undofail — prefix)."""
    slot = run.slots[b]
    if not slot:
        return "ok"
    n, new = b + ".exe", b + ".new.exe"
    run.op_delete(new, "up-clean:" + b)       # belt; errors cleared
    if not run.op_rename(n, new, "up-vacate:" + b):
        run.events.append("UNDO-STUCK " + b)  # new build kept, loud WARNING
        return "undofail"
    if restore_slot(run, b, slot):
        run.R["notupd"].append(b)
        run.undone[b] = True
        return "ok"
    # old restore failed and the canonical is empty — re-seat the new build (R1)
    if run.op_rename(new, n, "up-reseat:" + b):
        run.files[n]["arrival"] = "unplace-reseat"
        run.events.append("UNDO-KEEPNEW " + b)
        return "undofail"
    run.events.append("UNDO-LOST-BOTH " + b)  # LASTDITCH retries from .new
    return "undofail"


def slot_cleanup(run, b):
    """CYS_SLOT_CLEANUP — best-effort, lame-duck slot files simply survive."""
    if run.slots[b]:
        run.op_delete("%s.%s.exe" % (b, run.slots[b]), "sc:" + b)


def lastditch(run, b):
    """CYS_LASTDITCH — the final floor check (R1 enforcement, both paths).
    anchors: cys_ld_bad_ / cys_ld_nsus_ / cys_ld_nbad_ / cys_ld_p1..p3_ /
    cys_ld_mark_ / cys_ld_rolled_ / cys_ld_fatal_.
    Size-only probes on the old-build lanes; size+oracle on the .new lane
    (CONTRACT §9-3 — the honest probe-strength split)."""
    n, new = b + ".exe", b + ".new.exe"
    if run.size_probe(n) is True:
        return                                 # working floor met — done
    run.events.append("EMERGENCY " + b)
    run.op_delete(n, "ld-clear:" + b)          # may fail (locked) — flag ignored
    # `.new` promotion lane — this-build claim ⇒ size AND oracle re-check
    if new in run.files and run.op_rename(new, n, "ld-new:" + b):
        run.files[n]["arrival"] = "ld-new"
        sp = run.size_probe(n)
        if sp is True and run.files[n]["kind"] == "NEW" \
                and not run.fault("ld-oracle:" + b):
            return                             # recovered to THIS build — not reported
        # nsus: dispose of the suspect ONLY holding old material (fixed 3 slots
        # — NOT the tick wildcard; the hook checks prev/prev2/prev3 by name)
        if sp is not False:                    # size passed but oracle refused
            if not any(("%s.%s.exe" % (b, s)) in run.files
                       for s in ("prev", "prev2", "prev3")):
                run.events.append("LD-KEEP-SUSPECT " + b)
                return                         # empty-handed: keep, stay loud later
        run.op_delete(n, "ld-nbad:" + b)
    # prev chain: copy (leaves the slot) → on failure delete junk + rename
    for s in ("prev", "prev2", "prev3"):
        sn = "%s.%s.exe" % (b, s)
        if sn not in run.files:
            continue
        if run.op_copy(sn, n, "ld-slot:" + b + ":" + s):
            pass
        else:
            run.op_delete(n, "ld-junk:" + b)
            if not run.op_rename(sn, n, "ld-slot:" + b + ":" + s):
                continue
        if run.size_probe(n) is True:
            run.files[n]["arrival"] = "ld-prev"
            run.R["rolled"].append(b)          # rolled-back-to-previous: loud
            return
        run.R["unrec"].append(b)               # mark-probe failed — fatal
        return
    run.R["unrec"].append(b)                   # no material worked — fatal, loud


def abort_rescue(run, b):
    """CYS_ABORT_RESCUE — the interruption callback (single-shot renames).
    anchors: cys_ar_clean_ / cys_ar_mat_ / cys_ar_try_ / cys_ar_p1..p3_.
    R1-r1 size probe on the canonical; R2-r2 empty-handed rule (a truncated
    canonical with NO material is left untouched — the loud Hold state);
    material probe uses the .prev* WILDCARD (tick slots count), while the
    promotion chain itself is .new → prev → prev2 → prev3 only."""
    n, new = b + ".exe", b + ".new.exe"
    if n in run.files:
        sp = run.size_probe(n)
        if sp is None:
            return                             # cannot probe — no touch
        if sp:
            run.op_delete(new, "cb-clean:" + b)  # working canonical: clear staging
            return
        pref = b + ".prev"
        mats = new in run.files or any(
            m.startswith(pref) and m.endswith(".exe") for m in run.files)
        if not mats:
            return                             # empty-handed: leave the torn canonical
        if not run.op_delete(n, "cb-vacate:" + b):
            return                             # cannot vacate — no touch
    for cand, arr in ((new, "cb-new"),
                      (b + ".prev.exe", "cb-prev"),
                      (b + ".prev2.exe", "cb-prev"),
                      (b + ".prev3.exe", "cb-prev")):
        if cand in run.files and run.op_rename(cand, n, "cb-ren:" + cand):
            run.files[n]["arrival"] = arr
            return


def callback(run):
    for b in BINS:                             # .onInstFailed / .onUserAbort order
        abort_rescue(run, b)


def postinstall(run):
    """NSIS_HOOK_POSTINSTALL driver — transaction, reverse undo with
    stop-at-undofail, commit cleanup, LASTDITCH ×3, frozen classification.
    anchors: cys_txn_undo / cys_txn_commit / cys_txn_after / cys_post_fail /
    cys_post_lvl / cys_post_ok."""
    failed = False
    for b in BINS:
        if place(run, b) == "fail":
            failed = True
            break
    if failed:
        for b in reversed(BINS):
            if unplace(run, b) == "undofail":
                break                          # prefix preservation (R3)
    else:
        for b in BINS:
            slot_cleanup(run, b)
    run.int_point("before-lastditch")
    for b in BINS:
        lastditch(run, b)
    R = run.R
    if R["unrec"] or R["rolled"] or R["notupd"] or R["refused"]:
        lvl = 3 if (R["unrec"] or R["rolled"]) else 4
        run.failfile = True                    # frozen 4-token schema, ASCII
        run.exit = lvl
        callback(run)                          # Abort → .onInstFailed fires
        return
    run.failfile = False                       # success deletes the failure file
    run.marker = True                          # version marker only after all gates
    run.exit = 0


def run_one_factory(scen):
    def run_one(ch):
        run = Run(scen, ch)
        try:
            preinstall(run)
            template_extract(run)
            postinstall(run)
        except Interrupted as it:
            if it.mode == "cancel":
                callback(run)                  # .onUserAbort
            run.exit = it.mode
        return run
    return run_one


# ═════════════════════════════════════════════════════════════════════════════
# invariants
# ═════════════════════════════════════════════════════════════════════════════

class Violation(AssertionError):
    pass


def check(run):
    scen = run.scen
    start = {b: k for b, k in scen["start_kind"].items()}
    fam = {b: run.family(b) for b in BINS}

    def working_copy_somewhere(b, kinds=("OLD", "NEW")):
        return any(e["kind"] in kinds for e in fam[b].values())

    def old_somewhere(b):
        return any(e["kind"] == "OLD" for e in fam[b].values())

    completed = run.exit in (0, 3, 4)

    # exit classification self-check (frozen §4)
    if completed:
        want = 0
        if run.R["unrec"] or run.R["rolled"]:
            want = 3
        elif run.R["refused"] or run.R["notupd"]:
            want = 4
        if run.exit != want:
            raise Violation("exit classification drifted: exit=%r tokens=%r"
                            % (run.exit, run.R))

    for b in BINS:
        n = b + ".exe"
        e = run.files.get(n)
        # ── I1 ──────────────────────────────────────────────────────────────
        if run.exit == 0:
            if e is None or e["kind"] != "NEW":
                raise Violation("I1: exit 0 but %s is %r (must be this build)"
                                % (n, e and e["kind"]))
        elif e is None:                        # canonical ABSENT
            if run.exit == 4:
                raise Violation("I1: exit 4 with %s absent (LASTDITCH must "
                                "have escalated to 3)" % n)
            if run.exit == 3 and b not in run.R["unrec"]:
                raise Violation("I1: %s absent on exit 3 but not named in "
                                "unrecoverable: — silent loss" % n)
            if start[b] in ("OLD", "NEW") and not working_copy_somewhere(b):
                raise Violation("I1: %s absent and NO working copy left in its "
                                "family — silent total loss (started %s)"
                                % (n, start[b]))
        elif e["kind"] == "TRUNC":             # canonical present but truncated
            if run.exit in (0, 4):
                raise Violation("I1: exit %r with truncated %s" % (run.exit, n))
            if run.exit == 3 and b not in run.R["unrec"]:
                raise Violation("I1: truncated %s on exit 3 but not named in "
                                "unrecoverable:" % n)
            # cancel/kill: the deliberate loud Hold state (R2-r2) — allowed;
            # silent-loss guard: a torn canonical that DESTROYED the old build
            # is only reachable through the template's own truncate-write tear
            if run.exit in ("cancel", "kill") and start[b] == "OLD" \
                    and not working_copy_somewhere(b) and b not in run.torn_tpl:
                raise Violation("I1: %s truncated, old build gone, and not a "
                                "template tear — hook-made silent loss" % n)

    # ── I3 ──────────────────────────────────────────────────────────────────
    # The phoenix hazard is a NEW build at a LATER canonical while an EARLIER
    # canonical still runs the OLD build ("새 cysd + 구 cys" — hook R3 header).
    # Hook scope (CONTRACT §2 scope correction + §9-3/§9-5): such splits are
    # reachable through template/emergency combinations, but must NEVER be
    # silent — impossible on exit 0, and on completed runs the OLD-side binary
    # is always named in a token (it got OLD back through a refusal/undo/
    # rollback, each of which reports it).
    kinds = [run.files.get(b + ".exe", {}).get("kind") for b in BINS]
    named = set(run.R["refused"]) | set(run.R["unrec"]) \
        | set(run.R["rolled"]) | set(run.R["notupd"])
    for i in range(3):
        for j in range(i + 1, 3):
            if kinds[i] == "OLD" and kinds[j] == "NEW" and run.exit == 0:
                raise Violation("I3: exit 0 with generation split (%s OLD / "
                                "%s NEW)" % (BINS[i], BINS[j]))
    # per-binary loudness: any binary the transaction ATTEMPTED that ends on
    # the OLD build is named in a token (refused / not-updated / rolled-back /
    # unrecoverable).  Binaries the transaction never reached are legitimately
    # unnamed — frozen by CONTRACT §3 ("시도조차 못 한 바이너리는 목록에 없다");
    # the RUN is still loud (exit≠0 + failure file, asserted in I4).
    if completed:
        for b in BINS:
            if run.attempted[b] and run.files.get(b + ".exe", {}).get("kind") \
                    == "OLD" and b not in named:
                raise Violation("I3: %s was attempted, ends on the OLD build, "
                                "and is named in no token" % b)

    # ── I4 ──────────────────────────────────────────────────────────────────
    if completed:
        for b in BINS:
            e = run.files.get(b + ".exe")
            if e is not None and e["kind"] == "NEWBAD" and b not in (
                    set(run.R["refused"]) | set(run.R["unrec"])
                    | set(run.R["rolled"]) | set(run.R["notupd"])):
                # §9-3: a size-passing torn canonical is invisible to the
                # LASTDITCH size probe BY DESIGN (old builds may lack
                # VERSIONINFO) — but every lane that can produce one must have
                # named the binary on the way (reverify-failed etc.)
                raise Violation("I4: torn size-passing %s.exe with no token — "
                                "silent corruption" % b)
    for b in run.R["refused"]:
        if run.reason[b] not in REASONS:
            raise Violation("I4: refusal reason %r outside the frozen set"
                            % run.reason[b])
        e = run.files.get(b + ".exe")
        if start[b] == "OLD" and not old_somewhere(b) \
                and not (e is not None and e["kind"] == "NEW") \
                and not working_copy_somewhere(b):
            # CONTRACT §4 exit-4 row + §9-5/§9-3 scopes: the old build survives
            # UNLESS (a) the template already delivered THIS build in place
            # (old bytes legitimately consumed), or (b) the §9-3 torn-canonical
            # window — size-passing junk at the canonical, ALWAYS loud, with a
            # working build left in the family as boot-guard material
            # (the cysd PE-structure probe heals it next boot).
            raise Violation("I4: %s refused with NO working build anywhere in "
                            "its family — unrecoverable silent loss" % b)
    if completed and run.exit in (3, 4):
        if not run.failfile:
            raise Violation("I4: failing exit %r without the failure file"
                            % run.exit)
        if run.marker:
            raise Violation("I4: version marker written on failing exit %r"
                            % run.exit)
    if run.exit == 0 and (run.failfile or not run.marker):
        raise Violation("I4: exit 0 bookkeeping wrong (failfile=%r marker=%r)"
                        % (run.failfile, run.marker))
    if run.exit in ("cancel", "kill") and run.marker:
        raise Violation("I4: version marker written on an interrupted run")


# ═════════════════════════════════════════════════════════════════════════════
# scenarios (task S1..S6 mapping noted; S5 is a compile gate — harness N1)
# ═════════════════════════════════════════════════════════════════════════════

def scen(name, note, files, expect):
    start_kind = {b: None for b in BINS}
    for n, kind, lock, proc in files:
        for b in BINS:
            if n == b + ".exe":
                start_kind[b] = kind
    return {"name": name, "note": note, "files": files,
            "start_kind": start_kind, "expect": expect}


def O(n, kind, lock=None, proc=None):
    return (n, kind, lock, proc)


SCENARIOS = [
    scen("S1-fresh", "fresh install, nothing present",
         [], {"exit": 0}),
    scen("S2a-upgrade-unlocked", "old build present, nothing running "
         "(template overwrites in place; oracle short-circuits; no prev)",
         [O("cys.exe", "OLD"), O("cysd.exe", "OLD"), O("cys-app.exe", "OLD")],
         {"exit": 0, "no_prev": True}),
    scen("S2b-upgrade-running", "the real in-place update: CLI+daemon keep "
         "their image locks, GUI lock released by taskkill (task S2)",
         [O("cys.exe", "OLD", "image"), O("cysd.exe", "OLD", "image"),
          O("cys-app.exe", "OLD", "image", "gui")],
         {"exit": 0, "prev": ("cys", "cysd")}),
    scen("S3-samever", "canonical already IS this build (task S3): oracle "
         "short-circuit, no swap, no prev, no failure file",
         [O("cys.exe", "NEW"), O("cysd.exe", "NEW"), O("cys-app.exe", "NEW")],
         {"exit": 0, "no_prev": True}),
    scen("S3b-samever-running", "same-version reinstall while running",
         [O("cys.exe", "NEW", "image"), O("cysd.exe", "NEW", "image"),
          O("cys-app.exe", "NEW", "image", "gui")],
         {"exit": 0, "no_prev": True}),
    scen("S4a-trunc-unlocked", "truncated canonical, unlocked (task S4): "
         "template heals it in place",
         [O("cys.exe", "TRUNC"), O("cysd.exe", "OLD"), O("cys-app.exe", "OLD")],
         {"exit": 0}),
    scen("S4b-trunc-avlocked", "truncated canonical held by a share-read "
         "handle: refused loudly, never left absent (task S4 hard case)",
         [O("cys.exe", "TRUNC", "shareread"), O("cysd.exe", "OLD", "image"),
          O("cys-app.exe", "OLD")],
         {"exit": 3, "unrec": ("cys",)}),
    scen("S6-shareread", "CONTRACT P6: share-read handle on cys (task S6) — "
         "exit 4, vacate-locked, old intact; §9-5 split via template siblings",
         [O("cys.exe", "OLD", "shareread"), O("cysd.exe", "OLD"),
          O("cys-app.exe", "OLD")],
         {"exit": 4, "refused": {"cys": "vacate-locked"}}),
    scen("S6b-sharenone", "CONTRACT §9-6: share-NONE class — diagnosed as "
         "exit 3 misclassification, but lossless (old build untouched)",
         [O("cys.exe", "OLD", "image"), O("cysd.exe", "OLD", "sharenone"),
          O("cys-app.exe", "OLD", "image")],
         {"exit": 3}),
    scen("S7a-stale-new-locked", "stale OLD-content .new held by a share-read "
         "handle: the downgrade trap — placement refused (hook ② comment)",
         [O("cys.exe", "OLD", "image"), O("cysd.exe", "OLD", "image"),
          O("cys-app.exe", "OLD", "image"),
          O("cys.new.exe", "OLD", "shareread")],
         {"exit": 4, "refused": {"cys": "stale-new-locked"}}),
    scen("S7b-stale-new-swept", "stale .new under an IMAGE lock: the "
         "unlock-sweep renames it aside, placement proceeds clean",
         [O("cys.exe", "OLD", "image"), O("cysd.exe", "OLD", "image"),
          O("cys-app.exe", "OLD", "image"),
          O("cysd.new.exe", "OLD", "image")],
         {"exit": 0}),
    scen("S8-slots-full", "all three fixed prev slots held by lame-ducks: "
         "the tick-slot lane places anyway",
         [O("cys.exe", "OLD", "image"), O("cysd.exe", "OLD", "image"),
          O("cys-app.exe", "OLD", "image"),
          O("cysd.prev.exe", "OLD", "image"), O("cysd.prev2.exe", "OLD", "image"),
          O("cysd.prev3.exe", "OLD", "image")],
         {"exit": 0}),
    scen("S9-absent-mixed", "cys canonical deleted by hand, siblings locked: "
         "the vacate-skip (absent) fill lane",
         [O("cysd.exe", "OLD", "image"), O("cys-app.exe", "OLD", "image", None)],
         {"exit": 0}),
]


def check_expect(run):
    """The scenario's headline expectation, asserted on the CLEAN path only
    (the all-default DFS run: no faults, no interruption)."""
    exp = run.scen["expect"]
    if run.exit != exp["exit"]:
        raise Violation("clean-path exit %r != expected %r" % (run.exit, exp["exit"]))
    if exp.get("no_prev"):
        for n in run.files:
            if ".prev" in n:
                raise Violation("clean path created a prev slot %r" % n)
    for b in exp.get("prev", ()):
        if not any(n.startswith(b + ".prev") for n in run.files):
            raise Violation("clean path: expected a prev slot for %s" % b)
    for b, why in exp.get("refused", {}).items():
        if run.reason.get(b) != why:
            raise Violation("clean path: expected %s refusal %r, got %r"
                            % (b, why, run.reason.get(b)))
    for b in exp.get("unrec", ()):
        if b not in run.R["unrec"]:
            raise Violation("clean path: expected %s in unrecoverable" % b)


# ═════════════════════════════════════════════════════════════════════════════
# main
# ═════════════════════════════════════════════════════════════════════════════

def coverage_of(run):
    """Deep-lane markers this run touched — the anti-atrophy pins."""
    hit = set()
    for why in run.reason.values():
        hit.add("refusal:" + why)
    for tok, key in (("unrec", "token:unrecoverable"),
                     ("rolled", "token:rolled-back"),
                     ("notupd", "token:not-updated"),
                     ("refused", "token:placement-refused")):
        if run.R[tok]:
            hit.add(key)
    for ev in run.events:
        for tag in ("UNDO-STUCK", "UNDO-KEEPNEW", "UNDO-LOST-BOTH",
                    "LD-KEEP-SUSPECT", "RESTORE-FATAL", "EMERGENCY"):
            if ev.startswith(tag):
                hit.add("lane:" + tag)
    for b in BINS:
        e = run.files.get(b + ".exe")
        if e is None:
            continue
        arr = e.get("arrival")
        if arr in ("ld-new", "ld-prev", "cb-new", "cb-prev", "unplace-reseat",
                   "restore-old", "txn-fill", "tpl-extract"):
            hit.add("arrival:" + arr)
        if e["kind"] == "NEWBAD":
            hit.add("lane:torn-size-passing-canonical")
        if e["kind"] == "TRUNC" and run.exit in ("cancel", "kill"):
            hit.add("lane:hold-truncated-canonical")
    if any(".prev7" in n for n in run.files):
        hit.add("lane:tick-slot")
    if run.torn_tpl:
        hit.add("lane:template-tear")
    kinds = [run.files.get(b + ".exe", {}).get("kind") for b in BINS]
    if any(kinds[i] == "OLD" and kinds[j] == "NEW"
           for i in range(3) for j in range(i + 1, 3)):
        hit.add("lane:generation-split-loud")
    # D11 doc-claim pins (2026-08-29): NSIS-CONTRACT §9-3 / checklist exit-code
    # box corrections stay asserted against the code, not just written down —
    #   torn-canonical-exit4: an exit-4 terminal CAN hold a size-passing torn
    #     canonical (the failure file's frozen Note prose overstates there;
    #     token naming is what I4 asserts),
    #   ld-split-exit4: the LASTDITCH `.new` promotion lane CAN leave an
    #     old/new generation split on the exit-4 lane (loudness is I3/I4's job;
    #     these pins keep the REACHABILITY claims honest).
    # If a hook redesign (e.g. D9-b PREINSTALL .old evacuation, Release B)
    # removes these lanes, the pins go red — update NSIS-CONTRACT §9-3 and the
    # checklist exit-code box in the SAME commit.
    if run.exit == 4:
        if any(run.files.get(b + ".exe", {}).get("kind") == "NEWBAD"
               for b in BINS):
            hit.add("lane:D11-torn-canonical-exit4")
        if any(kinds[i] == "OLD" and kinds[j] == "NEW"
               and run.files[BINS[j] + ".exe"].get("arrival") == "ld-new"
               for i in range(3) for j in range(i + 1, 3)):
            hit.add("lane:D11-ld-split-exit4")
    hit.add("exit:%s" % run.exit)
    return hit


# every deep lane the mirrors must keep reaching at the default budgets —
# if one of these stops being explored, a mirror lost a decision point
COVERAGE_PINS = (
    ["refusal:" + r for r in REASONS]
    + ["token:unrecoverable", "token:rolled-back", "token:not-updated",
       "token:placement-refused",
       "lane:UNDO-STUCK", "lane:UNDO-KEEPNEW", "lane:UNDO-LOST-BOTH",
       "lane:LD-KEEP-SUSPECT", "lane:RESTORE-FATAL", "lane:EMERGENCY",
       "lane:torn-size-passing-canonical", "lane:hold-truncated-canonical",
       "lane:tick-slot", "lane:template-tear", "lane:generation-split-loud",
       "lane:D11-torn-canonical-exit4", "lane:D11-ld-split-exit4",
       "arrival:ld-new", "arrival:ld-prev", "arrival:cb-new", "arrival:cb-prev",
       "arrival:unplace-reseat", "arrival:restore-old", "arrival:txn-fill",
       "arrival:tpl-extract",
       "exit:0", "exit:3", "exit:4", "exit:cancel", "exit:kill"])

DEFAULT_BUDGETS = (FAULT_BUDGET == 4 and INT_FAULT_CAP == 3
                   and "CYS_MODEL_FAULTS" not in os.environ
                   and "CYS_MODEL_INT_FAULT_CAP" not in os.environ
                   or (FAULT_BUDGET >= 4 and INT_FAULT_CAP >= 3))


def main():
    hook_guard()
    total = 0
    cov = {}
    print("nsis-hook-model: faults<=%d per run, interrupts<=1 "
          "(offered while faults<=%d)" % (FAULT_BUDGET, INT_FAULT_CAP))
    for sc in SCENARIOS:
        census = {}
        n_runs = 0
        first = True
        for run, ch in explore(run_one_factory(sc)):
            n_runs += 1
            try:
                if first:
                    check_expect(run)
                    first = False
                check(run)
            except Violation as v:
                print("\nINVARIANT VIOLATION in %s: %s" % (sc["name"], v),
                      file=sys.stderr)
                print("  decision trail: %r" % [t[1] for t in ch.trail],
                      file=sys.stderr)
                print("  events: %s" % "; ".join(run.events), file=sys.stderr)
                print("  terminal files:", file=sys.stderr)
                for n in sorted(run.files):
                    print("    %-24s %s" % (n, run.files[n]["kind"]),
                          file=sys.stderr)
                print("  exit=%r tokens=%r" % (run.exit, run.R), file=sys.stderr)
                sys.exit(1)
            for h in coverage_of(run):
                cov[h] = cov.get(h, 0) + 1
            key = (run.exit,
                   bool(run.R["unrec"]), bool(run.R["rolled"]),
                   bool(run.R["notupd"]), bool(run.R["refused"]))
            census[key] = census.get(key, 0) + 1
        total += n_runs
        parts = []
        for key in sorted(census, key=repr):
            ex, u, r, nu, rf = key
            toks = "".join(t for t, on in
                           zip(("U", "R", "N", "P"), (u, r, nu, rf)) if on)
            parts.append("%s%s:%d" % (ex, ("+" + toks) if toks else "", census[key]))
        print("  %-22s %7d states  [%s]" % (sc["name"], n_runs, " ".join(parts)))
    print("nsis-hook-model: deep-lane coverage:")
    for k in sorted(cov):
        print("    %-38s %7d" % (k, cov[k]))
    if DEFAULT_BUDGETS:
        missing = [pin for pin in COVERAGE_PINS if not cov.get(pin)]
        if missing:
            print("nsis-hook-model: FAIL — pinned deep lanes no longer "
                  "explored: %s\n  (a mirror lost a decision point, or a "
                  "scenario was dropped — fix the model, do not delete pins)"
                  % ", ".join(missing), file=sys.stderr)
            sys.exit(1)
        if total < 30000:
            print("nsis-hook-model: FAIL — exploration collapsed (%d states; "
                  ">=30000 expected at the default budgets)." % total,
                  file=sys.stderr)
            sys.exit(1)
    print("nsis-hook-model: OK — %d terminal states explored, "
          "invariants I1/I3/I4 held in all of them, %d/%d deep-lane pins hit"
          % (total, len([p for p in COVERAGE_PINS if cov.get(p)]),
             len(COVERAGE_PINS)))


if __name__ == "__main__":
    main()
