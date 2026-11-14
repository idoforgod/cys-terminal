"""TW1 measurement record: the PURE part (no Windows call, no clock, no file system except load_matrix).

Release 0.15.0 (self-repair) measures its Windows gates on GitHub's hosted Windows runner (impl RULINGS CW7, EQ1-EQ6).
This module holds every rule that decides what a row of the record says, so that the rules can be unit-tested on any
machine (scripts/tests/windows_measure_selftest.py):

  * the matrix (scripts/windows-measure/matrix.json) is loaded and validated: every gate of design 8-2, every CW7 item and
    every row a ruling added exists exactly once;
  * a probe returns OBSERVATIONS (facts) only. The verdict of a row is COMPUTED here from those facts by the row's checks;
    nothing types a verdict;
  * a probe that raised, timed out, was skipped or never started is NOT-MEASURED, never a pass;
  * a control that did not discriminate makes the row NOT-MEASURED, never a pass;
  * a row needing a unit that is absent in the build is NOT-MEASURED "feature absent in this build";
  * a row that cannot be measured on a hosted runner is NOT-MEASURABLE-HERE, its fail-safe probes are sub-rows;
  * an observation that contradicts a stated assumption of the design or of a ruling is a FINDING (RULINGS EQ3): it never
    changes a verdict and never turns the job red, but it is listed first and counted;
  * the job is red when any row or sub-row is MEASURED-FAIL (after every probe ran) or when the set-up / the identity of the
    installation under test failed (RULINGS EQ4, D10);
  * every probe has a time limit; a probe that could not restore the state it promised makes the state "unknown" and every
    later probe needing a known state is not run (its rows are NOT-MEASURED);
  * the record carries no environment value outside an allow-list and no account name or personal path (RULINGS EQ6).

Standard library only.
"""

import json
import re

PASS = "MEASURED-PASS"
FAIL = "MEASURED-FAIL"
NMH = "NOT-MEASURABLE-HERE"
NM = "NOT-MEASURED"
VERDICTS = (PASS, FAIL, NMH, NM)
# worst first: a row is never PASS while any part of it was not measured (plan 3.7 rule 7)
_RANK = {FAIL: 0, NM: 1, NMH: 2, PASS: 3}

CLASSES = ("YES", "PARTLY", "NO", "WAIT")
KINDS = ("assert", "record")
ROLES = ("identity", "complete", "environment", "control", "check", "assumption")
OPS = (
    "eq", "ne", "eq_fact", "ne_fact", "version_gt_fact", "true", "false", "not_false", "in", "not_in",
    "lt", "le", "gt", "ge", "empty", "nonempty", "none", "not_none", "contains", "not_contains",
    "subset_fact", "all_in", "present",
)
STATES = ("clean", "base", "updated", "any", "same", "unknown")

# reasons are fixed sentences: the summary counts them by prefix
R_PROBE_NOT_BUILT = "probe not built yet"
R_PHASE = "phase 2 leg (seat-driven) - not run in phase 1"
R_FEATURE_ABSENT = "feature absent in this build"
R_UNIT_DISAGREE = "unit presence disagrees"
R_UNIT_UNKNOWN = "unit presence unknown"

# the items a ruling made mandatory (design 8-2 gates; RULINGS CW7; RULINGS EQ5)
REQUIRED_ROWS = tuple("G%02d" % i for i in range(1, 17)) + (
    "CW7-UPD", "CW7-NEXT", "CW7-LISTED", "CW7-GENERAL", "CW7-HELD", "CW7-ENC", "CW7-WFAIL", "CW7-INTR",
    "R-T3D-IDENT", "EC2", "EC4", "EC5", "DY1", "DY3", "T7C",
)

# environment variables whose VALUE may appear in the record (RULINGS EQ6); every other name is listed without its value
ENV_VALUE_ALLOW = (
    "OS", "PROCESSOR_ARCHITECTURE", "NUMBER_OF_PROCESSORS", "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion",
    "GITHUB_SHA", "GITHUB_REF_NAME", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "GITHUB_EVENT_NAME", "GITHUB_WORKFLOW",
    "PYTHONUTF8", "PYTHONIOENCODING", "PYTHONDONTWRITEBYTECODE", "TW1_PHASE", "TW1_BUILD_RUN_ID",
    "TW1_BUILD_CONCLUSION", "TW1_BUILD_REASON", "TW1_TNV_RESULT",
)


class MatrixError(ValueError):
    pass


# ───────────────────────────── matrix ─────────────────────────────

def load_matrix(path):
    with open(path, encoding="utf-8") as f:
        m = json.load(f)
    errors = validate_matrix(m)
    if errors:
        raise MatrixError("matrix invalid:\n  " + "\n  ".join(errors))
    return m


def iter_rows(m):
    """Every row and sub-row, parents first: (row, parent_id or None)."""
    for r in m.get("rows", []):
        yield r, None
        for s in r.get("subrows", []) or []:
            yield s, r["id"]


def _validate_checks(where, checks, errors):
    if not isinstance(checks, list):
        errors.append("%s: checks must be a list" % where)
        return
    seen = set()
    for c in checks:
        cid = c.get("id")
        if not cid or cid in seen:
            errors.append("%s: check without id or duplicated id %r" % (where, cid))
        seen.add(cid)
        if c.get("role") not in ROLES:
            errors.append("%s/%s: role %r not in %s" % (where, cid, c.get("role"), ROLES))
        if c.get("op") not in OPS:
            errors.append("%s/%s: op %r not in %s" % (where, cid, c.get("op"), OPS))
        if not c.get("fact"):
            errors.append("%s/%s: no fact" % (where, cid))
        if c.get("op", "").endswith("_fact") and not c.get("other"):
            errors.append("%s/%s: op %s needs 'other'" % (where, cid, c.get("op")))
        if c.get("role") == "environment" and not c.get("reason"):
            errors.append("%s/%s: an environment check needs the reason it cannot be measured" % (where, cid))
        if c.get("role") == "assumption" and not c.get("assumption"):
            errors.append("%s/%s: an assumption check needs the assumption it tests (source)" % (where, cid))


def validate_matrix(m):
    errors = []
    if m.get("schema") != 1:
        errors.append("schema must be 1")
    probes = m.get("probes")
    if not isinstance(probes, dict):
        errors.append("probes must be an object")
        probes = {}
    for pid, p in probes.items():
        for k in ("stage", "time_limit_s", "requires_state", "leaves_state"):
            if k not in p:
                errors.append("probe %s: missing %s" % (pid, k))
        if not isinstance(p.get("time_limit_s"), int) or p.get("time_limit_s", 0) <= 0:
            errors.append("probe %s: time_limit_s must be a positive integer" % pid)
        if p.get("requires_state") not in STATES or p.get("leaves_state") not in STATES:
            errors.append("probe %s: unknown state" % pid)
    ids = set()
    top = 0
    for r, parent in iter_rows(m):
        rid = r.get("id")
        where = rid or "<row without id>"
        if not rid or rid in ids:
            errors.append("row id missing or duplicated: %r" % rid)
        ids.add(rid)
        if parent is None:
            top += 1
            for k in ("title", "source", "group"):
                if not r.get(k):
                    errors.append("%s: missing %s" % (where, k))
        cls = r.get("class")
        if cls not in CLASSES:
            errors.append("%s: class %r not in %s" % (where, cls, CLASSES))
        if r.get("kind", "assert") not in KINDS:
            errors.append("%s: kind not in %s" % (where, KINDS))
        if r.get("phase", 1) not in (1, 2):
            errors.append("%s: phase must be 1 or 2" % where)
        if cls in ("PARTLY", "NO", "WAIT") and not r.get("why"):
            errors.append("%s: class %s needs 'why'" % (where, cls))
        if cls == "NO" and parent is None and not r.get("fail_safe"):
            errors.append("%s: class NO needs 'fail_safe'" % where)
        if cls == "WAIT" and not r.get("needs_units"):
            errors.append("%s: class WAIT needs 'needs_units'" % where)
        probe = r.get("probe")
        if probe is not None and probe not in probes:
            errors.append("%s: probe %r not in probes" % (where, probe))
        checks = r.get("checks", [])
        _validate_checks(where, checks, errors)
        if cls != "NO" and probe is not None and not [c for c in checks if c.get("role") in ("check", "control")]:
            errors.append("%s: a row with a probe needs at least one check or control" % where)
    for req in REQUIRED_ROWS:
        if req not in ids:
            errors.append("required row missing: %s (a row is removed only by a ruling)" % req)
    exp = m.get("expected_top_rows")
    if exp is not None and exp != top:
        errors.append("expected_top_rows=%s but the matrix has %s top-level rows" % (exp, top))
    for s in m.get("setup", []):
        if s.get("probe") not in probes:
            errors.append("setup: probe %r not in probes" % s.get("probe"))
        _validate_checks("setup/" + str(s.get("probe")), s.get("checks", []), errors)
    return errors


# ───────────────────────────── checks ─────────────────────────────

def _vtuple(s):
    if not isinstance(s, str):
        return None
    m = re.fullmatch(r"\s*(\d+)\.(\d+)\.(\d+)(?:\.(\d+))?\s*", s)
    if not m:
        return None
    return tuple(int(x) for x in m.groups(default="0"))


def evaluate_check(check, facts):
    """-> ("met" | "unmet" | "missing", detail). A fact that was not recorded is "missing", never "met"."""
    fact = check["fact"]
    op = check["op"]
    if fact not in facts:
        return "missing", "fact %r not recorded" % fact
    v = facts[fact]
    want = check.get("value")
    if op.endswith("_fact"):
        other = check["other"]
        if other not in facts:
            return "missing", "fact %r not recorded" % other
        o = facts[other]
    try:
        if op == "present":
            ok = True
        elif op == "eq":
            ok = v == want
        elif op == "ne":
            ok = v != want
        elif op == "eq_fact":
            if v is None or o is None:
                return "missing", "%s=%r %s=%r (a None side is not an observation)" % (fact, v, check["other"], o)
            ok = v == o
        elif op == "ne_fact":
            if v is None or o is None:
                return "missing", "%s=%r %s=%r (a None side is not an observation)" % (fact, v, check["other"], o)
            ok = v != o
        elif op == "version_gt_fact":
            a, b = _vtuple(v), _vtuple(o)
            if a is None or b is None:
                return "missing", "not a version: %r / %r" % (v, o)
            ok = a > b
        elif op == "true":
            ok = v is True
        elif op == "false":
            ok = v is False
        elif op == "not_false":
            ok = v is not False
        elif op == "in":
            ok = v in want
        elif op == "not_in":
            ok = v not in want
        elif op in ("lt", "le", "gt", "ge"):
            if not isinstance(v, (int, float)) or isinstance(v, bool):
                return "missing", "%s=%r is not a number" % (fact, v)
            ok = {"lt": v < want, "le": v <= want, "gt": v > want, "ge": v >= want}[op]
        elif op == "empty":
            if v is None:
                return "missing", "%s is None" % fact
            ok = len(v) == 0
        elif op == "nonempty":
            if v is None:
                return "missing", "%s is None" % fact
            ok = len(v) > 0
        elif op == "none":
            ok = v is None
        elif op == "not_none":
            ok = v is not None
        elif op == "contains":
            if v is None:
                return "missing", "%s is None" % fact
            ok = want in v
        elif op == "not_contains":
            if v is None:
                return "missing", "%s is None" % fact
            ok = want not in v
        elif op == "subset_fact":
            if v is None or o is None:
                return "missing", "a None side"
            ok = set(v) <= set(o)
        elif op == "all_in":
            if v is None:
                return "missing", "%s is None" % fact
            ok = all(x in want for x in v)
        else:  # validate_matrix refuses unknown ops; this is the last line of defence
            return "missing", "unknown op %r" % op
    except TypeError as e:
        return "missing", "cannot compare: %s" % e
    detail = "%s=%r" % (fact, v)
    if op.endswith("_fact"):
        detail += " vs %s=%r" % (check["other"], o)
    elif "value" in check:
        detail += " (%s %r)" % (op, want)
    return ("met" if ok else "unmet"), detail


# ───────────────────────────── decisions ─────────────────────────────

def unit_status(unit, units):
    """present | absent | disagree | unknown, from the behavioural marker and the merge log (plan 3.5)."""
    u = (units or {}).get(unit) or {}
    beh = u.get("behaviour")      # True / False / None (no marker, or not yet observed)
    log = u.get("merge_log")      # True / False / None (history unavailable)
    has_marker = u.get("has_marker", False)
    if has_marker and beh is None:
        return "unknown"
    if beh is None:
        if log is None:
            return "unknown"
        return "present" if log else "absent"
    if log is None:
        return "present" if beh else "absent"
    if beh == log:
        return "present" if beh else "absent"
    return "disagree"


def decide_row(row, probes, state, phase, units):
    """Before any probe runs: -> (run: bool, verdict or None, reason or None)."""
    if row.get("class") == "NO":
        return False, NMH, row.get("why") or "not measurable on a hosted runner"
    if row.get("phase", 1) > phase:
        return False, NM, R_PHASE
    probe = row.get("probe")
    needs = row.get("needs_units", []) or []
    # an absent unit is the truer reason even where the probe is not built yet ("feature absent in this build")
    for u in needs:
        st = unit_status(u, units)
        if st == "absent":
            return bool(row.get("run_when_absent")) and probe is not None, NM, "%s: %s" % (R_FEATURE_ABSENT, u)
        if st == "disagree":
            return False, NM, "%s: %s %r" % (R_UNIT_DISAGREE, u, (units or {}).get(u))
    if probe is None:
        return False, NM, R_PROBE_NOT_BUILT
    for u in needs:
        if unit_status(u, units) == "unknown":
            return False, NM, "%s: %s" % (R_UNIT_UNKNOWN, u)
    need = probes[probe]["requires_state"]
    if need != "any" and state != need:
        return False, NM, "state %r required, found %r" % (need, state)
    return True, None, None


def state_after(probe_meta, status, restored, prev_state):
    """The state the next probe starts from (plan 3.7 rule 8)."""
    if status != "ran":
        if status in ("skipped", "not_started"):
            return prev_state
        return prev_state if probe_meta.get("read_only") else "unknown"
    if restored is False:
        return "unknown"
    leaves = probe_meta.get("leaves_state", "same")
    return prev_state if leaves == "same" else leaves


def _outcome_reason(outcome):
    st = outcome.get("status")
    if st == "error":
        return "probe raised: %s" % (outcome.get("reason") or "error")
    if st == "timeout":
        return "probe timed out after %ss" % outcome.get("limit_s", "?")
    if st == "skipped":
        return "probe skipped: %s" % (outcome.get("reason") or "")
    if st == "not_started":
        return "probe never started: %s" % (outcome.get("reason") or "")
    return "probe status %r" % st


def compute_verdict(row, outcome):
    """-> dict(verdict, reason, findings, checks). Only facts decide; a missing fact is never a pass."""
    res = {"verdict": NM, "reason": None, "findings": [], "checks": []}
    if outcome is None:
        res["reason"] = "probe never started"
        return res
    if outcome.get("status") != "ran":
        res["reason"] = _outcome_reason(outcome)
        return res
    facts = outcome.get("facts") or {}
    by_role = {r: [] for r in ROLES}
    for c in row.get("checks", []):
        state, detail = evaluate_check(c, facts)
        rec = {"id": c["id"], "role": c["role"], "state": state, "detail": detail}
        if c.get("text"):
            rec["text"] = c["text"]
        res["checks"].append(rec)
        by_role[c["role"]].append((c, state, detail))
    for c, st, d in by_role["assumption"]:
        if st == "unmet":
            res["findings"].append({"row": row["id"], "check": c["id"], "assumption": c["assumption"], "observed": d})
    for c, st, d in by_role["environment"]:
        if st == "missing":
            res["reason"] = "environment not observed: %s (%s)" % (c["id"], d)
            return res
        if st == "unmet":
            res["verdict"] = NMH
            res["reason"] = "%s (%s)" % (c["reason"], d)
            return res
    for c, st, d in by_role["identity"]:
        if st != "met":
            res["reason"] = "identity of the installation under test not proven: %s (%s)" % (c["id"], d)
            return res
    for c, st, d in by_role["complete"]:
        if st != "met":
            res["reason"] = "record incomplete: %s (%s)" % (c["id"], d)
            return res
    for c, st, d in by_role["control"]:
        if st == "missing":
            res["reason"] = "control not observed: %s (%s)" % (c["id"], d)
            return res
        if st == "unmet":
            res["reason"] = "control did not discriminate: %s (%s)" % (c["id"], d)
            return res
    missing = [(c, d) for c, st, d in by_role["check"] if st == "missing"]
    if missing:
        c, d = missing[0]
        res["reason"] = "observation missing: %s (%s)" % (c["id"], d)
        return res
    unmet = [(c, d) for c, st, d in by_role["check"] if st == "unmet"]
    if unmet:
        res["verdict"] = FAIL
        res["reason"] = "; ".join("%s: %s" % (c["id"], d) for c, d in unmet)
        return res
    res["verdict"] = PASS
    res["reason"] = "all checks met, every control discriminated" if by_role["control"] else "all checks met"
    return res


def worst(verdicts):
    vs = [v for v in verdicts if v is not None]
    if not vs:
        return NM
    return min(vs, key=lambda v: _RANK[v])


def aggregate(row, own, subs):
    """A parent row's verdict: class NO stays NOT-MEASURABLE-HERE (its fail-safe sub-rows are reported, and counted, on
    their own); any other parent takes the worst of its own result (if it has checks) and its sub-rows."""
    if row.get("class") == "NO":
        return NMH
    parts = list(subs)
    if own is not None:
        parts.append(own)
    return worst(parts)


# ───────────────────────────── time limits ─────────────────────────────

class Deadline:
    """A time limit read from an injected clock (seconds, monotonic)."""

    def __init__(self, clock, limit_s):
        self.clock = clock
        self.start = clock()
        self.limit_s = limit_s

    def remaining(self):
        return max(0.0, self.limit_s - (self.clock() - self.start))

    def expired(self):
        return self.clock() - self.start >= self.limit_s

    def elapsed(self):
        return self.clock() - self.start


def wait_until(poll, clock, sleep, limit_s, step_s=0.5):
    """Call poll() until it returns not-None or the limit passes. -> ("done", value, elapsed) | ("timeout", None, elapsed).
    poll, clock and sleep are injected (tests use a fake clock: no real sleep)."""
    d = Deadline(clock, limit_s)
    while True:
        v = poll()
        if v is not None:
            return "done", v, d.elapsed()
        if d.expired():
            return "timeout", None, d.elapsed()
        sleep(min(step_s, max(d.remaining(), 0.0)) or step_s)


def longest_false_run(samples):
    """Length of the longest run of consecutive False samples (checklist P1: 200-ms presence samples)."""
    best = cur = 0
    for s in samples:
        if s is False:
            cur += 1
            best = max(best, cur)
        else:
            cur = 0
    return best


# ───────────────────────────── redaction / environment ─────────────────────────────

def redaction_map(environ):
    """[(needle, replacement)] longest first. Values come from the runner's environment; the record keeps only the
    placeholders (RULINGS EQ6: no account name, no personal path)."""
    pairs = []
    for name in ("USERPROFILE", "LOCALAPPDATA", "APPDATA", "RUNNER_TEMP", "GITHUB_WORKSPACE", "TEMP", "TMP",
                 "RUNNER_TOOL_CACHE", "HOMEPATH"):
        v = environ.get(name)
        if v and len(v) >= 4:
            pairs.append((v, "%" + name + "%"))
    for name, rep in (("GITHUB_REPOSITORY", "<repository>"), ("GITHUB_REPOSITORY_OWNER", "<owner>"),
                      ("GITHUB_ACTOR", "<actor>"), ("GITHUB_TRIGGERING_ACTOR", "<actor>"),
                      ("USERNAME", "%USERNAME%"), ("COMPUTERNAME", "%COMPUTERNAME%")):
        v = environ.get(name)
        if v and len(v) >= 3:
            pairs.append((v, rep))
    pairs.sort(key=lambda p: len(p[0]), reverse=True)
    return pairs


def _needle_patterns(needle):
    forms = {needle, needle.replace("\\", "/")}
    if "\\" in needle:
        forms.add(needle.replace("\\", "\\\\"))  # JSON-escaped form
    return [re.compile(re.escape(f), re.IGNORECASE) for f in sorted(forms, key=len, reverse=True)]


def redact(obj, mapping):
    """Replace every needle (case-insensitive, back- and forward-slash forms) in every string of obj."""
    compiled = [(p, rep) for needle, rep in mapping for p in _needle_patterns(needle)]

    def one(s):
        for p, rep in compiled:
            s = p.sub(lambda _m: rep, s)
        return s

    def walk(o):
        if isinstance(o, str):
            return one(o)
        if isinstance(o, list):
            return [walk(x) for x in o]
        if isinstance(o, tuple):
            return [walk(x) for x in o]
        if isinstance(o, dict):
            return {walk(k): walk(v) for k, v in o.items()}
        return o

    return walk(obj)


def env_snapshot(environ, allow=ENV_VALUE_ALLOW):
    return {
        "values": {k: environ[k] for k in sorted(environ) if k in allow},
        "names_only": sorted(k for k in environ if k not in allow),
    }


# ───────────────────────────── the record ─────────────────────────────

def _count_reason(prefix, results):
    return sum(1 for r in results if r["verdict"] == NM and (r.get("reason") or "").startswith(prefix))


def summarize(rows, subrows, setup_failed, findings):
    def counts(rs):
        c = {v: 0 for v in VERDICTS}
        for r in rs:
            c[r["verdict"]] += 1
        return c

    top = counts(rows)
    sub = counts(subrows)
    allr = list(rows) + list(subrows)
    s = {
        "rows": len(rows),
        "subrows": len(subrows),
        "top": top,
        "sub": sub,
        "feature_absent": _count_reason(R_FEATURE_ABSENT, rows),
        "probe_not_built": _count_reason(R_PROBE_NOT_BUILT, rows),
        "phase2": _count_reason(R_PHASE, rows),
        "findings": len(findings),
        "setup_failed": bool(setup_failed),
        "any_fail": any(r["verdict"] == FAIL for r in allr),
    }
    s["line"] = ("PASS %d · FAIL %d · NOT-MEASURABLE-HERE %d · NOT-MEASURED %d (feature absent %d; probe not built %d; "
                 "phase 2 %d) · findings %d · set-up %s · sub-rows: PASS %d FAIL %d NMH %d NM %d") % (
        top[PASS], top[FAIL], top[NMH], top[NM], s["feature_absent"], s["probe_not_built"], s["phase2"],
        len(findings), "FAILED" if setup_failed else "ok", sub[PASS], sub[FAIL], sub[NMH], sub[NM])
    return s


def job_conclusion(record):
    """-> ("red" | "green", [reasons]). Findings never turn it red (RULINGS EQ3)."""
    if not record or "summary" not in record:
        return "red", ["no record was produced"]
    s = record["summary"]
    reasons = []
    if s.get("any_fail"):
        reasons.append("at least one row or sub-row is MEASURED-FAIL")
    if s.get("setup_failed"):
        reasons.append("the set-up or the identity of the installation under test failed (RULINGS EQ4 D10)")
    return ("red" if reasons else "green"), reasons


def build_record(meta, environment, setup, row_results, sub_results):
    findings = []
    for r in list(row_results) + list(sub_results):
        findings.extend(r.get("findings", []))
    summary = summarize(row_results, sub_results, setup.get("failed"), findings)
    return {
        "schema": 1,
        "meta": meta,
        "environment": environment,     # environment facts first (ticket TW1, "THE RECORD")
        "setup": setup,
        "summary": summary,
        "findings": findings,           # listed first in the Markdown summary (RULINGS EQ3)
        "rows": list(row_results),
        "subrows": list(sub_results),
    }


def _md_cell(s):
    s = "" if s is None else str(s)
    return s.replace("|", "\\|").replace("\n", " ")


def render_markdown(record):
    s = record["summary"]
    out = ["# TW1 Windows measurement record", "", "**%s**" % s["line"], ""]
    concl, why = job_conclusion(record)
    out.append("Job conclusion: **%s**%s" % (concl, (" - " + "; ".join(why)) if why else ""))
    out.append("")
    out.append("## Findings (observations that contradict a stated assumption - each needs a ruling; RULINGS EQ3)")
    if record["findings"]:
        for f in record["findings"]:
            out.append("- **%s / %s** - assumption: %s - observed: %s" % (
                f["row"], f["check"], _md_cell(f["assumption"]), _md_cell(f["observed"])))
    else:
        out.append("- none")
    out.append("")
    out.append("## Environment")
    env = record.get("environment") or {}
    for k in sorted(env):
        v = env[k]
        if isinstance(v, (dict, list)):
            v = json.dumps(v, ensure_ascii=False, sort_keys=True)
        out.append("- `%s`: %s" % (k, _md_cell(v)))
    out.append("")
    out.append("## Set-up")
    st = record.get("setup") or {}
    out.append("- failed: %s" % st.get("failed"))
    for r in st.get("reasons", []):
        out.append("- %s" % _md_cell(r))
    out.append("")
    out.append("## Rows")
    out.append("")
    out.append("| id | title | class | verdict | reason | source | evidence |")
    out.append("|---|---|---|---|---|---|---|")
    for r in record["rows"]:
        out.append("| %s | %s | %s | **%s** | %s | %s | %s |" % (
            r["id"], _md_cell(r.get("title")), r.get("class"), r["verdict"], _md_cell(r.get("reason")),
            _md_cell(r.get("source")), _md_cell(", ".join(r.get("evidence", [])))))
    out.append("")
    out.append("## Sub-rows")
    out.append("")
    out.append("| id | parent | verdict | reason | evidence |")
    out.append("|---|---|---|---|---|")
    for r in record["subrows"]:
        out.append("| %s | %s | **%s** | %s | %s |" % (
            r["id"], r.get("parent"), r["verdict"], _md_cell(r.get("reason")),
            _md_cell(", ".join(r.get("evidence", [])))))
    out.append("")
    return "\n".join(out)


def evaluate_setup(setup_spec, outcomes):
    """The set-up checks (identity of the base and of the installation under test). -> dict(failed, reasons, checks)."""
    reasons, checks = [], []
    for s in setup_spec:
        oc = outcomes.get(s["probe"])
        if oc is None or oc.get("status") != "ran":
            reasons.append("set-up probe %s: %s" % (s["probe"], _outcome_reason(oc or {"status": "not_started"})))
            continue
        facts = oc.get("facts") or {}
        for c in s.get("checks", []):
            st, d = evaluate_check(c, facts)
            checks.append({"probe": s["probe"], "id": c["id"], "state": st, "detail": d})
            if st != "met":
                reasons.append("%s/%s %s: %s" % (s["probe"], c["id"], st, d))
    return {"failed": bool(reasons), "reasons": reasons, "checks": checks}


def row_result(row, parent, decision, outcome):
    """Assemble one row's (or sub-row's) result from the decision and the probe outcome."""
    run, v, reason = decision
    base = {
        "id": row["id"], "title": row.get("title"), "class": row.get("class"), "kind": row.get("kind", "assert"),
        "source": row.get("source"), "expected": row.get("expected"), "why": row.get("why"),
        "fail_safe": row.get("fail_safe"), "needs_units": row.get("needs_units", []), "probe": row.get("probe"),
        "evidence": [], "findings": [], "checks": [],
    }
    if parent is not None:
        base["parent"] = parent
    if v is not None and not (run and row.get("run_when_absent")):
        base.update({"verdict": v, "reason": reason})
        return base
    if v is not None:  # run for information only: the verdict stays the decision's
        info = compute_verdict(row, outcome)
        base.update({"verdict": v, "reason": reason, "info_checks": info["checks"]})
        base["evidence"] = list((outcome or {}).get("evidence", []))
        return base
    res = compute_verdict(row, outcome)
    base.update({"verdict": res["verdict"], "reason": res["reason"], "findings": res["findings"],
                 "checks": res["checks"]})
    base["evidence"] = list((outcome or {}).get("evidence", []))
    return base


# ───────────────────────────── the windows-build run at this commit ─────────────────────────────

def select_build_run(runs, sha):
    """The windows-build run that built THIS commit: head_sha equal, a push run preferred, the newest first.
    -> run dict or None. Pure (the REST listing is passed in)."""
    same = [r for r in (runs or []) if isinstance(r, dict) and r.get("head_sha") == sha]
    if not same:
        return None
    pushes = [r for r in same if r.get("event") == "push"]
    pool = pushes or same
    return max(pool, key=lambda r: (r.get("created_at") or "", r.get("run_number") or 0, r.get("id") or 0))


def build_wait_decision(run):
    """-> "found" (completed), "wait" (queued / in progress) or "absent"."""
    if run is None:
        return "absent"
    return "found" if run.get("status") == "completed" else "wait"


# ───────────────────────────── pack manifests and line endings (run-1 lessons; RULINGS FS, FT) ─────────────────────────────

def eol_normalized(data):
    """The bytes with every CRLF turned into LF (for "content kept modulo line endings")."""
    return data.replace(b"\r\n", b"\n") if data is not None else None


def manifest_delta(prev, cur):
    """Two pack manifests {rel: sha} -> {"missing": [...], "added": [...], "changed": [...]} (sorted). None in -> None out:
    a manifest that was not taken is not an observation of "nothing changed"."""
    if prev is None or cur is None:
        return None
    p, c = set(prev), set(cur)
    return {"missing": sorted(p - c), "added": sorted(c - p),
            "changed": sorted(k for k in p & c if prev[k] != cur[k])}


def ntfs_key(name):
    """NTFS directory order is case-insensitive by its upper-case table; ASCII names: str.upper() reproduces it."""
    return name.upper()


def entries_before(names, held):
    """The directory entries an in-order delete visits before the held entry `held` (it stops there)."""
    return sorted((n for n in names if ntfs_key(n) < ntfs_key(held)), key=ntfs_key)
