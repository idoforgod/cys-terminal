#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""javis_selfservice — the self-service case tool (F58; DESIGN-F58-final §2, r2 §2 and §8.2, r1 C3/C4).

The user's Jarvis fixes its own problems in user space. This tool keeps the case record of one such repair
and is the only shipped route to a vendor report. It never sends anything.

  open     --symptom <text> [--surface N]          -> opened; prints the case id
  triage   --case <id> [--entry <entry id>]         -> triaged (or exhausted when no lever applies); 8 NO_MATCH
  consent  --case <id> (--lever <L> | --report) (--yes | --no)
  attempt  --case <id> --lever <L> --action <text> --evidence <file>
  verify   --case <id>                              -> fixed | mitigated_with_residual; 1 not fixed
  retriage --case <id>                              -> product_defect | unresolved (from exhausted)
  recheck  [--case <id>]                            -> after an update; 1 if any case regressed
  report   --case <id> [--hypothesis <t>] [--observed <t>]   -> the report gate (final §2.4)
  status   [--case <id>]

Exit codes: 0 ok; 1 not fixed / symptom gone at retriage / state has no such step; 2 usage; 3 no such case;
4 REDIRECT (wrong state, next lever, nothing to report); 5 NO_ATTEMPT; 6 NEEDS_USER_YES; 7 USER_DECLINED;
8 NO_MATCH.

Files: ~/.cys/local/self-service/<UTC id>/case.json, attempts.json, obs/<seq>-<probe>.out, vendor-report.txt,
report-evidence/. Every string written is scrubbed (HOME -> ~, values of secret-named env vars, approval
signatures); no environment dump; evidence files are recorded by name, sha256 and size only.
Probe values are pass | fail | unknown, written only by this tool; unknown is never success; verify, recheck,
report and retriage re-run the probes at that moment and never decide from a stored value.
Lever table: <pack>/round/self_service_levers.json (pack = CYS_PACK_DIR, else this script's pack root).
"""
import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

HERE = os.path.dirname(os.path.abspath(__file__))

RC_OK, RC_NOT_FIXED, RC_USAGE, RC_NO_CASE, RC_REDIRECT, RC_NO_ATTEMPT, RC_NEEDS_YES, RC_DECLINED, RC_NO_MATCH = range(9)

SELF_FIXABLE, SR, R_NO_LEVER, OWNER = ("self_fixable", "self_mitigable_product_defect", "product_defect_no_lever",
                                       "owner_needed")
VERDICTS = (SELF_FIXABLE, SR, R_NO_LEVER, OWNER)
DEFECT_VERDICTS = (SR, R_NO_LEVER)
REPORT_STATES = ("mitigated_with_residual", "product_defect", "unresolved")
RECHECK_STATES = ("fixed", "mitigated_with_residual", "reported", "product_defect", "unresolved")
TRIAGE_FROM = ("opened", "regressed", "product_defect", "unresolved")
CASE_ID_RE = re.compile(r"^\d{8}T\d{6}Z-[0-9a-f]{6}$")
SECRET_NAME_RE = re.compile(r"(?i)(token|secret|key|password|passwd|sig|auth|cookie|credential)")
SIGNATURE_RE = re.compile(r"(?i)(signature|sig|hmac)([\"'\s:=]+)([A-Za-z0-9+/=_-]{16,})")
FIXED_RECHECK_DAYS = 30

SR_QUESTION = ("지금 쓰시는 데는 문제없게 막았습니다. 남는 원인은 제품 안에 있어 보입니다 — "
               "고친 기록을 붙여 제조사 보고 초안을 만들어 둘까요?")
DEFECT_QUESTION = ("이 기기에서 할 수 있는 조치로는 증상이 남아 있습니다 — "
                   "시도한 기록을 붙여 제조사 보고 초안을 만들어 둘까요? (보내지는 않습니다)")
GUI_SENTENCE = ("REDIRECT: the user declined every fix, so there is no self-fix evidence, and a vendor report needs "
                "it (owner boundary, F58). The fixes can be offered again (triage --case %s re-plans them). The "
                "user can still send feedback through the app's 「피드백 보내기」 window, which never refuses.")


# ───────────────────────── paths, time, scrubbing, atomic writes ─────────────────────────

def pack_dir():
    return os.environ.get("CYS_PACK_DIR") or os.path.dirname(HERE)


def cases_root():
    return Path.home() / ".cys" / "local" / "self-service"


def _now():
    return datetime.now(timezone.utc)


def _iso(t=None):
    return (t or _now()).strftime("%Y-%m-%dT%H:%M:%SZ")


def _parse_iso(s):
    try:
        return datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except (TypeError, ValueError):
        return None


def _sha(b):
    return hashlib.sha256(b).hexdigest()


class Scrubber(object):
    """HOME -> ~; values (>= 8 chars) of env vars whose NAME looks secret -> <redacted>; approval signatures."""

    def __init__(self, environ=None):
        env = os.environ if environ is None else environ
        vals = set()
        for k, v in env.items():
            if SECRET_NAME_RE.search(k) and isinstance(v, str) and len(v) >= 8:
                vals.add(v)
        self.secrets = sorted(vals, key=len, reverse=True)
        homes = set()
        for h in (env.get("HOME"), env.get("USERPROFILE"), str(Path.home())):
            if not h:
                continue
            for x in (h, os.path.realpath(h)):
                x = x.rstrip("/\\")
                if len(x) >= 2:
                    homes.add(x)
        self.homes = sorted(homes, key=len, reverse=True)

    def text(self, s):
        for v in self.secrets:
            s = s.replace(v, "<redacted>")
        for h in self.homes:
            s = s.replace(h, "~")
        return SIGNATURE_RE.sub(lambda m: m.group(1) + m.group(2) + "<redacted>", s)

    def obj(self, o):
        if isinstance(o, str):
            return self.text(o)
        if isinstance(o, list):
            return [self.obj(x) for x in o]
        if isinstance(o, dict):
            return {k: self.obj(v) for k, v in o.items()}
        return o


_SCRUB = []


def scrubber():
    if not _SCRUB:
        _SCRUB.append(Scrubber())
    return _SCRUB[0]


def _write_bytes_atomic(path, data):
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix="." + os.path.basename(path) + ".", suffix=".part")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _write_json(path, obj):
    text = json.dumps(scrubber().obj(obj), ensure_ascii=False, indent=1) + "\n"
    _write_bytes_atomic(path, text.encode("utf-8"))


def _read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


# ───────────────────────── lever table, matching, probes ─────────────────────────

class TableError(Exception):
    pass


def load_table(pack=None):
    p = os.path.join(pack or pack_dir(), "round", "self_service_levers.json")
    try:
        t = _read_json(p)
    except (OSError, ValueError) as e:
        raise TableError("lever table unreadable: %s (%s)" % (p, type(e).__name__))
    if not isinstance(t, dict) or not isinstance(t.get("entries"), list):
        raise TableError("lever table has no entries list: %s" % p)
    return t


def match_entry(table, symptom):
    """D-U3-3: score = number of an entry's `match` substrings in the symptom (case-insensitive); the best
    score >= 1 wins, ties go to table order; None when nothing matches."""
    s = (symptom or "").lower()
    best, best_score = None, 0
    for e in table.get("entries", []):
        score = sum(1 for m in e.get("match", []) if isinstance(m, str) and m and m.lower() in s)
        if score > best_score:
            best, best_score = e, score
    return best


def predicate_holds(pred, out):
    """True / False, or None when the predicate cannot be evaluated on this output (-> unknown)."""
    if pred is None:
        return True
    if not isinstance(pred, dict):
        return None
    kind = pred.get("kind", "none")
    if kind == "none":
        return True
    if kind == "contains":
        return str(pred.get("text", "")) in out
    if kind == "not_contains":
        return str(pred.get("text", "")) not in out
    if kind == "json_path_eq":
        try:
            cur = json.loads(out)
        except ValueError:
            return None
        for part in str(pred.get("path", "")).split("."):
            if part == "":
                continue
            if isinstance(cur, dict) and part in cur:
                cur = cur[part]
            elif isinstance(cur, list) and part.isdigit() and int(part) < len(cur):
                cur = cur[int(part)]
            else:
                return None
        return cur == pred.get("value")
    return None


def _safe_name(pid):
    return re.sub(r"[^A-Za-z0-9._-]", "_", str(pid))[:60] or "probe"


def run_probe(table, pid, case_dir, seq, role, command, pack):
    """Run one probe; the value is pass | fail | unknown and nothing else. Output is stored scrubbed and
    hashed as stored."""
    obs = {"seq": seq, "role": role, "probe": pid or "none", "command": command, "value": "unknown",
           "exit": None, "argv": None, "out_sha256": None, "out_size": 0, "out_file": None, "at": _iso(),
           "note": None}
    if not pid or pid == "none":
        obs["note"] = "no probe for this step"
        return obs
    spec = (table.get("probes") or {}).get(pid)
    if not isinstance(spec, dict) or not isinstance(spec.get("argv"), list) or not spec["argv"]:
        obs["note"] = "probe id not in the lever table"
        return obs
    argv = [str(t).replace("{python}", sys.executable).replace("{pack}", pack) for t in spec["argv"]]
    if spec["argv"][0] == "{python}":
        argv.insert(1, "-B")
    obs["argv"] = argv
    timeout = spec.get("timeout_s", 20)
    raw_out, raw_err, rc = b"", b"", None
    try:
        r = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout)
        raw_out, raw_err, rc = r.stdout or b"", r.stderr or b"", r.returncode
    except subprocess.TimeoutExpired as e:
        raw_out, raw_err = e.stdout or b"", e.stderr or b""
        obs["note"] = "timeout after %s s" % timeout
    except (OSError, ValueError) as e:
        obs["note"] = "not runnable: %s" % type(e).__name__
    if rc is not None:
        obs["exit"] = rc
        held = predicate_holds(spec.get("predicate"), raw_out.decode("utf-8", "replace"))
        if held is None:
            obs["note"] = "the predicate could not read the output"
        elif rc == spec.get("ok_exit", 0) and held:
            obs["value"] = "pass"
        else:
            obs["value"] = "fail"
    stored = raw_out + ((b"\n[stderr]\n" + raw_err) if raw_err else b"")
    if rc is not None or stored:
        data = scrubber().text(stored.decode("utf-8", "replace")).encode("utf-8")
        name = "%03d-%s.out" % (seq, _safe_name(pid))
        _write_bytes_atomic(os.path.join(case_dir, "obs", name), data)
        obs.update({"out_file": "obs/" + name, "out_sha256": _sha(data), "out_size": len(data)})
    return obs


# ───────────────────────── case records ─────────────────────────

def case_dir_of(cid):
    return str(cases_root() / cid)


def load_case(cid):
    if not isinstance(cid, str) or not CASE_ID_RE.match(cid):
        return None
    p = os.path.join(case_dir_of(cid), "case.json")
    if not os.path.isfile(p):
        return None
    try:
        c = _read_json(p)
    except (OSError, ValueError):
        return None
    return c if isinstance(c, dict) else None


def save_case(case):
    case["updated_at"] = _iso()
    _write_json(os.path.join(case_dir_of(case["id"]), "case.json"), case)


def load_attempts(cid):
    p = os.path.join(case_dir_of(cid), "attempts.json")
    if not os.path.isfile(p):
        return []
    doc = _read_json(p)
    lst = doc.get("attempts") if isinstance(doc, dict) else doc
    return lst if isinstance(lst, list) else []


def save_attempts(cid, attempts):
    _write_json(os.path.join(case_dir_of(cid), "attempts.json"), {"schema": 1, "case": cid, "attempts": attempts})


def transition(case, to, command, hashes=(), rule=None):
    case.setdefault("history", []).append({"at": _iso(), "command": command, "from": case.get("state"), "to": to,
                                           "evidence_hashes": [h for h in hashes if h]})
    case["state"] = to
    if rule is not None:
        case["rule"] = rule


def record(case, obs):
    case.setdefault("observations", []).append(obs)
    return obs


def next_seq(case):
    return len(case.get("observations", [])) + 1


def probe_now(case, table, pid, role, command):
    return record(case, run_probe(table, pid, case_dir_of(case["id"]), next_seq(case), role, command, pack_dir()))


def symptom_pid(case):
    return (case.get("symptom_probe") or {}).get("id") or "none"


def defect_pid(case):
    return (case.get("defect_probe") or {}).get("id") or "none"


def plan_levers(case):
    return ((case.get("plan") or {}).get("levers")) or []


def find_lever(case, lid):
    return next((lv for lv in plan_levers(case) if lv.get("id") == lid), None)


def plan_attempts(case, attempts):
    seq = (case.get("plan") or {}).get("seq")
    return [a for a in attempts if a.get("plan") == seq]


def exhausted_now(case, attempts):
    """r2 §2.2 / final §2.3: every applicable lever has an attempt (this plan) whose verify is fail or unknown,
    or is declined."""
    cur = plan_attempts(case, attempts)
    for lv in plan_levers(case):
        if not lv.get("applicable"):
            continue
        if lv.get("status") == "declined":
            continue
        if any(a.get("lever") == lv["id"] and a.get("verify") in ("fail", "unknown") for a in cur):
            continue
        return False
    return True


def valid_report_consent(case):
    """The report answer counts only until the next state change (final §2.3: the Jarvis does not ask again
    unless the user raises it or the state changes)."""
    if case.get("report_consent_epoch") != len(case.get("history", [])):
        return None
    return case.get("report_consent")


def must_case(cid):
    case = load_case(cid)
    if case is None:
        print("no such case: %s (cases live in %s)" % (cid, cases_root()))
    return case


def wrong_state(case, verb, rc=RC_REDIRECT):
    print("%s: not in state %s (case %s)" % (verb, case.get("state"), case.get("id")))
    return rc


def obs_line(o):
    if not o:
        return "none"
    return "%s value %s sha256 %s at %s" % (o.get("probe"), o.get("value"), o.get("out_sha256") or "-",
                                            o.get("at"))


# ───────────────────────── verbs ─────────────────────────

def _cys_version():
    exe = shutil.which("cys")
    if not exe:
        return "unknown"
    try:
        r = subprocess.run([exe, "--version"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=10)
    except (OSError, ValueError, subprocess.SubprocessError):
        return "unknown"
    out = (r.stdout or b"").decode("utf-8", "replace").strip()
    return out.splitlines()[0].strip() if r.returncode == 0 and out else "unknown"


def _pack_version():
    try:
        with open(os.path.join(pack_dir(), ".pack-version"), encoding="utf-8") as f:
            v = f.read().strip()
    except (OSError, ValueError):
        return "unknown"
    return v.splitlines()[0][:200] if v else "unknown"


def cmd_open(a):
    try:
        table = load_table()
    except TableError as e:
        table, note = {"entries": []}, str(e)
    else:
        note = None
    root = cases_root()
    while True:
        cid = _now().strftime("%Y%m%dT%H%M%SZ") + "-" + os.urandom(3).hex()
        d = root / cid
        try:
            os.makedirs(str(d))
            break
        except FileExistsError:
            continue
    case = {"schema": 1, "id": cid, "created_at": _iso(), "updated_at": None, "symptom": a.symptom,
            "surface": a.surface,
            "versions": {"cys": _cys_version(), "pack": _pack_version(),
                         "os": (platform.system() + " " + platform.release()).strip() or "unknown",
                         "arch": platform.machine() or "unknown"},
            "state": None, "entry": None, "entry_source": None, "entry_title": None, "verdict": None,
            "symptom_probe": None, "defect_probe": None, "plan": None, "residual": None, "reason": None,
            "rule": None, "report_consent": None, "report_consent_at": None, "report_consent_epoch": None,
            "report_declined_at": None, "report": None, "from_state": None, "observations": [], "history": []}
    e = match_entry(table, a.symptom)
    # r2 §2.2: a `before` observation is taken at open — the matched entry's symptom probe (none -> unknown).
    obs = probe_now(case, table, e.get("symptom_probe", "none") if e else "none", "symptom", "open")
    obs["entry"] = e.get("id") if e else None
    if note:
        obs["note"] = note
    transition(case, "opened", "open", [obs.get("out_sha256")])
    save_case(case)
    print(cid)
    print("state: opened")
    print("next: triage --case %s" % cid)
    return RC_OK


def _list_entries(table):
    for e in table.get("entries", []):
        print("%s  %s" % (e.get("id"), e.get("title", "")))


def cmd_triage(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    if case.get("state") not in TRIAGE_FROM:
        return wrong_state(case, "triage")
    table = load_table()
    if a.entry:
        e = next((x for x in table["entries"] if x.get("id") == a.entry), None)
        if e is None:
            print("unknown entry id: %s; entries:" % a.entry)
            _list_entries(table)
            return RC_USAGE
        source = "chosen"
    else:
        e = match_entry(table, case.get("symptom"))
        if e is None:
            print("NO_MATCH: no lever-table entry matches the symptom; choose one with --entry <id>:")
            _list_entries(table)
            return RC_NO_MATCH
        source = "matched"
    if e.get("verdict") not in VERDICTS:
        print("lever table entry %s has an unknown verdict %r" % (e.get("id"), e.get("verdict")))
        return RC_USAGE
    sym = probe_now(case, table, e.get("symptom_probe", "none"), "symptom", "triage")
    defect = None
    if e["verdict"] in DEFECT_VERDICTS:
        dpid = e.get("defect_probe", "none")
        dobs = probe_now(case, table, dpid, "defect", "triage")
        spec = (table.get("probes") or {}).get(dpid) if dpid != "none" else None
        defect = {"id": dpid, "reason": e.get("defect_probe_reason"), "argv": dobs.get("argv"),
                  "predicate": spec.get("predicate") if isinstance(spec, dict) else None,
                  "value": dobs["value"], "out_sha256": dobs.get("out_sha256"), "at": dobs["at"]}
    prev = case.get("plan") or {}
    levers = []
    for lv in e.get("levers", []):
        levers.append({"id": lv.get("id"), "class": lv.get("class"), "action": lv.get("action"),
                       "risky": bool(lv.get("risky")), "ask_once": bool(lv.get("risky")),
                       "applicable": bool(lv.get("applicable")),
                       "not_applicable_reason": lv.get("not_applicable_reason"),
                       "readback_probe": lv.get("readback_probe") or "none",
                       "status": "pending", "consent": None, "consent_at": None})
    case.update({"entry": e.get("id"), "entry_source": source, "entry_title": e.get("title"),
                 "verdict": e["verdict"],
                 "symptom_probe": {"id": e.get("symptom_probe", "none"), "reason": e.get("symptom_probe_reason")},
                 "defect_probe": defect, "plan": {"seq": int(prev.get("seq") or 0) + 1, "at": _iso(),
                                                  "levers": levers},
                 "residual": None, "reason": None})
    hashes = [sym.get("out_sha256"), (defect or {}).get("out_sha256")]
    transition(case, "triaged", "triage", hashes, rule=None)
    if not any(lv["applicable"] for lv in levers):
        transition(case, "exhausted", "triage", hashes, rule="triage: the plan has no applicable lever (r2 2.2)")
    save_case(case)
    print("case %s: %s (entry %s, %s)" % (case["id"], case["state"], e.get("id"), source))
    print("verdict: %s" % e["verdict"])
    print("symptom probe: %s" % sym["value"])
    if defect:
        print("defect probe: %s" % defect["value"])
    _print_plan(case)
    return RC_OK


def _print_plan(case):
    plan = case.get("plan") or {}
    if not plan:
        return
    print("plan %s:" % plan.get("seq"))
    for lv in plan.get("levers", []):
        print("  %s [%s] %s%s%s — %s" % (lv.get("id"), lv.get("class"), lv.get("status"),
                                        " ask_once" if lv.get("ask_once") else "",
                                        "" if lv.get("applicable") else
                                        " not_applicable (%s)" % lv.get("not_applicable_reason"),
                                        lv.get("action")))


def cmd_consent(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    answer = "yes" if a.yes else "no"
    if a.report:
        if case.get("state") not in REPORT_STATES:
            return wrong_state(case, "consent --report")
        case.update({"report_consent": answer, "report_consent_at": _iso(),
                     "report_consent_epoch": len(case.get("history", []))})
        if answer == "no":
            case["report_declined_at"] = case["report_consent_at"]
        save_case(case)
        print("report consent: %s (case %s)" % (answer, case["id"]))
        return RC_OK
    if case.get("state") not in ("triaged", "attempting"):
        return wrong_state(case, "consent --lever")
    lv = find_lever(case, a.lever)
    if lv is None:
        print("lever %s is not in the plan" % a.lever)
        return RC_USAGE
    if not lv.get("ask_once"):
        print("lever %s is not ask_once: it needs no consent and cannot be declined; attempt it" % a.lever)
        return RC_USAGE
    if not lv.get("applicable"):
        print("lever %s is not applicable: %s" % (a.lever, lv.get("not_applicable_reason")))
        return RC_REDIRECT
    lv.update({"consent": answer, "consent_at": _iso()})
    if answer == "no":
        lv["status"] = "declined"
    elif lv.get("status") == "declined":
        lv["status"] = "pending"
    if answer == "no" and exhausted_now(case, load_attempts(case["id"])):
        transition(case, "exhausted", "consent %s no" % a.lever,
                   rule="consent: every applicable lever attempted with fail/unknown or declined (final 2.3)")
    save_case(case)
    print("lever %s consent: %s; state %s" % (a.lever, answer, case["state"]))
    return RC_OK


def _file_sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def cmd_attempt(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    if case.get("state") not in ("triaged", "attempting"):
        return wrong_state(case, "attempt")
    lv = find_lever(case, a.lever)
    if lv is None:
        print("lever %s is not in the plan" % a.lever)
        return RC_USAGE
    if not lv.get("applicable") or lv.get("status") == "declined":
        print("REDIRECT: lever %s is %s" % (a.lever, "declined" if lv.get("applicable") else
                                            "not applicable: %s" % lv.get("not_applicable_reason")))
        return RC_REDIRECT
    if lv.get("ask_once") and lv.get("consent") != "yes":
        print("NEEDS_USER_YES: lever %s is a risky step; ask the user once, then record the answer with "
              "consent --case %s --lever %s --yes|--no" % (a.lever, case["id"], a.lever))
        return RC_NEEDS_YES
    if not os.path.isfile(a.evidence):
        print("evidence file not found: %s" % a.evidence)
        return RC_USAGE
    table = load_table()
    ev_path = os.path.abspath(a.evidence)
    evidence = {"name": ev_path, "sha256": _file_sha(ev_path), "size": os.path.getsize(ev_path)}
    before = next((o for o in reversed(case.get("observations", [])) if o.get("role") == "symptom"), None)
    after = probe_now(case, table, symptom_pid(case), "symptom", "attempt")
    readback = probe_now(case, table, lv.get("readback_probe") or "none", "readback", "attempt")
    attempts = load_attempts(case["id"])
    attempts.append({"seq": len(attempts) + 1, "plan": case["plan"]["seq"], "lever": lv["id"], "action": a.action,
                     "evidence": evidence, "before": before, "after": after, "readback": readback,
                     "verify": after["value"], "at": _iso()})
    save_attempts(case["id"], attempts)
    lv["status"] = "attempted"
    hashes = [evidence["sha256"], after.get("out_sha256"), readback.get("out_sha256")]
    if case["state"] != "attempting":
        transition(case, "attempting", "attempt %s" % lv["id"], hashes)
    if exhausted_now(case, attempts):
        transition(case, "exhausted", "attempt %s" % lv["id"], hashes,
                   rule="attempt: every applicable lever attempted with fail/unknown or declined (r2 2.2)")
    save_case(case)
    print("attempt %d on %s: before %s, after %s, readback %s; state %s"
          % (len(attempts), lv["id"], (before or {}).get("value", "none"), after["value"], readback["value"],
             case["state"]))
    if after["value"] == "pass" and case["state"] == "attempting":
        print("next: verify --case %s" % case["id"])
    return RC_OK


def cmd_verify(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    st = case.get("state")
    if st in ("opened", "triaged"):
        print("NO_ATTEMPT: no attempt yet in case %s (state %s)" % (case["id"], st))
        return RC_NO_ATTEMPT
    if st not in ("attempting", "exhausted"):
        return wrong_state(case, "verify", RC_NOT_FIXED)
    table = load_table()
    sym = probe_now(case, table, symptom_pid(case), "symptom", "verify")
    if sym["value"] != "pass":
        save_case(case)
        print("symptom probe %s: not fixed; state %s" % (sym["value"], st))
        return RC_NOT_FIXED
    if case.get("verdict") not in DEFECT_VERDICTS:
        transition(case, "fixed", "verify", [sym.get("out_sha256")],
                   rule="verify: symptom probe pass, verdict %s (final 2.3)" % case.get("verdict"))
    else:
        dobs = probe_now(case, table, defect_pid(case), "defect", "verify")
        hashes = [sym.get("out_sha256"), dobs.get("out_sha256")]
        if dobs["value"] == "pass":
            transition(case, "fixed", "verify", hashes,
                       rule="verify: symptom probe pass and defect probe pass (final 2.3)")
        else:
            case["residual"] = {"probe": dobs["probe"], "value": dobs["value"],
                                "out_sha256": dobs.get("out_sha256"), "at": dobs["at"]}
            transition(case, "mitigated_with_residual", "verify", hashes,
                       rule="verify: symptom probe pass, defect probe %s (final 2.3)" % dobs["value"])
    save_case(case)
    print("state: %s" % case["state"])
    if case["state"] == "mitigated_with_residual":
        print("ask the user once: %s" % SR_QUESTION)
        print("then: consent --case %s --report --yes|--no" % case["id"])
    return RC_OK


def cmd_retriage(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    if case.get("state") != "exhausted":
        return wrong_state(case, "retriage")
    attempts = plan_attempts(case, load_attempts(case["id"]))
    applicable = [lv for lv in plan_levers(case) if lv.get("applicable")]
    tried = {x.get("lever") for x in attempts}
    if applicable and all(lv.get("status") == "declined" for lv in applicable) \
            and not any(lv["id"] in tried for lv in applicable):
        case["reason"] = "all_declined"
        transition(case, "unresolved", "retriage",
                   rule="retriage: every applicable lever declined and none attempted (final 2.3)")
        save_case(case)
        print("state: unresolved (all_declined)")
        return RC_OK
    table = load_table()
    sym = probe_now(case, table, symptom_pid(case), "symptom", "retriage")
    if sym["value"] == "pass":
        save_case(case)
        print("symptom not observed now: run verify --case %s" % case["id"])
        return RC_NOT_FIXED
    if sym["value"] == "fail":
        for lv in applicable:
            if lv["id"] not in tried or (lv.get("readback_probe") or "none") == "none":
                continue
            rb = probe_now(case, table, lv["readback_probe"], "readback", "retriage")
            if rb["value"] == "pass":
                case["reason"] = None
                transition(case, "product_defect", "retriage", [sym.get("out_sha256"), rb.get("out_sha256")],
                           rule="retriage: symptom probe fail now and lever %s reads back changed now (r2 2.2)"
                                % lv["id"])
                save_case(case)
                print("state: product_defect")
                return RC_OK
    if not applicable:
        reason = "no_lever"
    elif sym["value"] == "unknown":
        reason = "unknown_probe"
    elif any(lv.get("status") == "declined" for lv in applicable):
        reason = "declined"
    else:
        reason = "no_effect_shown"
    case["reason"] = reason
    transition(case, "unresolved", "retriage", [sym.get("out_sha256")],
               rule="retriage: symptom probe %s now, %s (r2 2.2)" % (sym["value"], reason))
    save_case(case)
    print("state: unresolved (%s)" % reason)
    return RC_OK


def _recheck_one(case, table):
    """Returns True when the case regressed."""
    st = case["state"]
    base = case.get("from_state") if st == "reported" else st
    sym = probe_now(case, table, symptom_pid(case), "symptom", "recheck")
    if base == "fixed":
        if sym["value"] != "pass":
            transition(case, "regressed", "recheck", [sym.get("out_sha256")],
                       rule="recheck: symptom probe %s after fixed (r2 2.2)" % sym["value"])
            return True
        return False
    if base == "mitigated_with_residual":
        dobs = probe_now(case, table, defect_pid(case), "defect", "recheck")
        hashes = [sym.get("out_sha256"), dobs.get("out_sha256")]
        if sym["value"] != "pass":
            transition(case, "regressed", "recheck", hashes,
                       rule="recheck: symptom probe %s after the mitigation (final 2.3)" % sym["value"])
            return True
        if dobs["value"] == "pass":
            transition(case, "fixed", "recheck", hashes,
                       rule="recheck: symptom probe pass and defect probe pass (final 2.3)")
            return False
        case["residual"] = {"probe": dobs["probe"], "value": dobs["value"], "out_sha256": dobs.get("out_sha256"),
                            "at": dobs["at"]}
        return False
    return False  # product_defect / unresolved (or reported from them): the value is recorded, no change


def _fixed_recently(case):
    at = next((h.get("at") for h in reversed(case.get("history", [])) if h.get("to") == "fixed"), None)
    t = _parse_iso(at)
    return t is not None and (_now() - t).total_seconds() <= FIXED_RECHECK_DAYS * 86400


def cmd_recheck(a):
    if a.case:
        case = must_case(a.case)
        if case is None:
            return RC_NO_CASE
        if case.get("state") not in RECHECK_STATES:
            return wrong_state(case, "recheck")
        targets = [case]
    else:
        targets = []
        root = cases_root()
        names = sorted(os.listdir(str(root))) if root.is_dir() else []
        for n in names:
            c = load_case(n)
            if c is None or c.get("state") not in RECHECK_STATES:
                continue
            if c["state"] == "fixed" and not _fixed_recently(c):
                continue
            targets.append(c)
    table = load_table() if targets else {"entries": []}
    regressed = 0
    for c in targets:
        before = c["state"]
        if _recheck_one(c, table):
            regressed += 1
        save_case(c)
        print("%s: %s -> %s" % (c["id"], before, c["state"]))
    print("rechecked %d case(s), regressed %d" % (len(targets), regressed))
    return RC_NOT_FIXED if regressed else RC_OK


def _report_text(case, attempts, sym, dobs, a):
    v = case.get("versions") or {}
    lines = ["vendor report draft — written by javis_selfservice.py; nothing was sent. The user attaches this file "
             "and report-evidence/ in 「피드백 보내기」.",
             "case: %s" % case["id"], "written_at: %s" % _iso(), "symptom: %s" % case.get("symptom"),
             "versions: cys=%s pack=%s os=%s arch=%s" % (v.get("cys"), v.get("pack"), v.get("os"), v.get("arch")),
             "entry: %s (%s) %s" % (case.get("entry"), case.get("entry_source"), case.get("entry_title")),
             "verdict: %s" % case.get("verdict"),
             "state: %s" % case.get("state"),
             "rule: %s" % case.get("rule"),
             "reason: %s" % (case.get("reason") or "-"),
             "root_cause: %s" % (("observed: %s (given by the Jarvis with --observed; the tool does not infer it)"
                                  % a.observed) if a.observed else "unproven"),
             "jarvis_hypothesis (a hypothesis, not a finding): %s" % (a.hypothesis or "none"), ""]
    tri_sym = next((o for o in reversed(case.get("observations", [])) if o.get("role") == "symptom"
                    and o.get("command") == "triage"), None)
    lines.append("symptom probe at triage: %s" % obs_line(tri_sym))
    dp = case.get("defect_probe")
    if dp:
        lines.append("defect probe at triage: %s value %s sha256 %s at %s"
                     % (dp.get("id"), dp.get("value"), dp.get("out_sha256") or "-", dp.get("at")))
    lines.append("levers (plan %s):" % (case.get("plan") or {}).get("seq"))
    for lv in plan_levers(case):
        lines.append("  %s [%s] %s%s — %s" % (lv.get("id"), lv.get("class"), lv.get("status"),
                                              " ask_once" if lv.get("ask_once") else "", lv.get("action")))
    lines.append("attempts:")
    if not attempts:
        lines.append("  none")
    for x in attempts:
        ev = x.get("evidence") or {}
        lines += ["  #%s lever %s (plan %s) at %s" % (x.get("seq"), x.get("lever"), x.get("plan"), x.get("at")),
                  "     action: %s" % x.get("action"),
                  "     evidence: %s sha256 %s size %s" % (ev.get("name"), ev.get("sha256"), ev.get("size")),
                  "     before: %s" % obs_line(x.get("before")),
                  "     after: %s" % obs_line(x.get("after")),
                  "     readback: %s" % obs_line(x.get("readback")),
                  "     verify: %s" % x.get("verify")]
    if case.get("state") == "mitigated_with_residual":
        mit = [x for x in plan_attempts(case, attempts) if (x.get("after") or {}).get("value") == "pass"]
        lines.append("mitigation that restored the symptom: %s" % (
            "; ".join("lever %s: %s (attempt #%s, after sha256 %s)" % (
                x.get("lever"), x.get("action"), x.get("seq"), (x.get("after") or {}).get("out_sha256"))
                for x in mit) or "the symptom probe passed at verify"))
        r = case.get("residual") or {}
        lines.append("residual at verify: defect probe %s value %s sha256 %s at %s"
                     % (r.get("probe"), r.get("value"), r.get("out_sha256") or "-", r.get("at")))
        if dobs is not None:
            lines.append("defect probe now: %s command %s value %s sha256 %s at %s"
                         % (dobs.get("probe"), json.dumps(dobs.get("argv"), ensure_ascii=False), dobs.get("value"),
                            dobs.get("out_sha256") or "-", dobs.get("at")))
    lines.append("symptom probe now (re-run for this report): %s" % obs_line(sym))
    lines.append("files: report-evidence/attempts.json and report-evidence/obs/ (copies frozen at %s)" % _iso())
    return "\n".join(lines) + "\n"


def _write_report(case, a):
    table = load_table()
    cid = case["id"]
    d = case_dir_of(cid)
    sym = probe_now(case, table, symptom_pid(case), "symptom", "report")
    dobs = None
    if case["state"] == "mitigated_with_residual":
        dobs = probe_now(case, table, defect_pid(case), "defect", "report")
    attempts = load_attempts(cid)
    data = scrubber().text(_report_text(case, attempts, sym, dobs, a)).encode("utf-8")
    ev_dir = os.path.join(d, "report-evidence")
    src = os.path.join(d, "attempts.json")
    if os.path.isfile(src):
        with open(src, "rb") as f:
            _write_bytes_atomic(os.path.join(ev_dir, "attempts.json"), f.read())
    else:
        _write_json(os.path.join(ev_dir, "attempts.json"), {"schema": 1, "case": cid, "attempts": []})
    obs_dir = os.path.join(d, "obs")
    copied = []
    for n in sorted(os.listdir(obs_dir)) if os.path.isdir(obs_dir) else []:
        p = os.path.join(obs_dir, n)
        if os.path.isfile(p) and n.endswith(".out"):
            with open(p, "rb") as f:
                _write_bytes_atomic(os.path.join(ev_dir, "obs", n), f.read())
            copied.append("obs/" + n)
    out = os.path.join(d, "vendor-report.txt")
    _write_bytes_atomic(out, data)
    case["report"] = {"path": "vendor-report.txt", "sha256": _sha(data), "at": _iso(),
                      "evidence_dir": "report-evidence", "copied": ["attempts.json"] + copied}
    case["from_state"] = case["state"]
    transition(case, "reported", "report", [_sha(data), sym.get("out_sha256"), (dobs or {}).get("out_sha256")])
    save_case(case)
    print("report: %s" % out)
    print("written, not sent: attach vendor-report.txt and report-evidence/ in 「피드백 보내기」")
    return RC_OK


def cmd_report(a):
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    st = case.get("state")
    cid = case["id"]
    if st == "opened":
        print("NO_ATTEMPT: triage first: triage --case %s" % cid)
        return RC_NO_ATTEMPT
    if st in ("triaged", "attempting"):
        nxt = next((lv for lv in plan_levers(case) if lv.get("applicable") and lv.get("status") == "pending"), None)
        if nxt is not None:
            print("REDIRECT: try the next lever first: %s [%s] %s" % (nxt["id"], nxt.get("class"), nxt.get("action")))
        else:
            print("REDIRECT: every lever was tried; run verify --case %s" % cid)
        return RC_REDIRECT
    if st == "exhausted":
        print("NO_ATTEMPT: retriage first: retriage --case %s" % cid)
        return RC_NO_ATTEMPT
    if st == "regressed":
        print("NO_ATTEMPT: the case regressed; triage first (re-plan): triage --case %s" % cid)
        return RC_NO_ATTEMPT
    if st == "fixed":
        print("REDIRECT: fixed — the symptom is gone and no defect probe fails; nothing to report. "
              "The app's 「피드백 보내기」 window never refuses the user.")
        return RC_REDIRECT
    if st == "reported":
        print("report: %s" % os.path.join(case_dir_of(cid), (case.get("report") or {}).get("path",
                                                                                          "vendor-report.txt")))
        print("already reported: nothing written")
        return RC_OK
    if st == "unresolved" and case.get("reason") == "all_declined":
        print(GUI_SENTENCE % cid)
        _print_plan(case)
        return RC_REDIRECT
    if st in REPORT_STATES:
        consent = valid_report_consent(case)
        if consent is None:
            print("NEEDS_USER_YES: ask the user this one question:")
            print(SR_QUESTION if st == "mitigated_with_residual" else DEFECT_QUESTION)
            print("then record the answer: consent --case %s --report --yes|--no" % cid)
            return RC_NEEDS_YES
        if consent == "no":
            print("USER_DECLINED: the user declined the report at %s; nothing written. Do not ask again unless "
                  "the user raises it or the case state changes." % case.get("report_consent_at"))
            return RC_DECLINED
        return _write_report(case, a)
    return wrong_state(case, "report", RC_NOT_FIXED)


def cmd_status(a):
    if not a.case:
        root = cases_root()
        names = sorted(os.listdir(str(root))) if root.is_dir() else []
        n = 0
        for name in names:
            c = load_case(name)
            if c is None:
                continue
            n += 1
            print("%s  %s  %s  %s" % (c["id"], c.get("state"), c.get("verdict") or "-",
                                      (c.get("symptom") or "")[:60].replace("\n", " ")))
        print("%d case(s) in %s" % (n, root))
        return RC_OK
    case = must_case(a.case)
    if case is None:
        return RC_NO_CASE
    print("case: %s" % case["id"])
    print("state: %s" % case.get("state"))
    print("verdict: %s" % (case.get("verdict") or "-"))
    print("entry: %s (%s) %s" % (case.get("entry") or "-", case.get("entry_source") or "-",
                                 case.get("entry_title") or ""))
    print("symptom: %s" % case.get("symptom"))
    if case.get("reason"):
        print("reason: %s" % case["reason"])
    if case.get("residual"):
        print("residual: %s" % case["residual"].get("value"))
    _print_plan(case)
    return RC_OK


# ───────────────────────── CLI ─────────────────────────

def build_parser():
    p = argparse.ArgumentParser(prog="javis_selfservice.py",
                                description="F58 self-service case tool (case record, report gate; sends nothing)")
    sub = p.add_subparsers(dest="cmd")
    s = sub.add_parser("open", help="open a case for one symptom")
    s.add_argument("--symptom", required=True)
    s.add_argument("--surface")
    s = sub.add_parser("triage", help="match the lever table, run the probes, plan the levers")
    s.add_argument("--case", required=True)
    s.add_argument("--entry")
    s = sub.add_parser("consent", help="record the user's one answer for a risky lever or the report")
    s.add_argument("--case", required=True)
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--lever")
    g.add_argument("--report", action="store_true")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--yes", action="store_true")
    g.add_argument("--no", action="store_true")
    s = sub.add_parser("attempt", help="record one lever attempt; the tool runs the probes")
    s.add_argument("--case", required=True)
    s.add_argument("--lever", required=True)
    s.add_argument("--action", required=True)
    s.add_argument("--evidence", required=True)
    for verb in ("verify", "retriage"):
        s = sub.add_parser(verb)
        s.add_argument("--case", required=True)
    s = sub.add_parser("recheck", help="re-run the probes of settled cases (after an update)")
    s.add_argument("--case")
    s = sub.add_parser("report", help="the report gate: writes vendor-report.txt only with evidence and consent")
    s.add_argument("--case", required=True)
    s.add_argument("--hypothesis")
    s.add_argument("--observed")
    s = sub.add_parser("status")
    s.add_argument("--case")
    # `corrections --role <ROLE>` (U2: vendor-default corrections for this install) is added here; not in U3.
    return p


HANDLERS = {"open": cmd_open, "triage": cmd_triage, "consent": cmd_consent, "attempt": cmd_attempt,
            "verify": cmd_verify, "retriage": cmd_retriage, "recheck": cmd_recheck, "report": cmd_report,
            "status": cmd_status}


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    p = build_parser()
    a = p.parse_args(argv)
    if not a.cmd:
        p.print_help()
        return RC_USAGE
    try:
        return HANDLERS[a.cmd](a)
    except TableError as e:
        print(str(e))
        return RC_USAGE


if __name__ == "__main__":
    sys.exit(main())
