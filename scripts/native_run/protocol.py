"""Fixed approved populations; no executable run authority is granted here."""
import hashlib
import json
import os
from pathlib import Path

COUNTS = {"I": 300, "L": 60, "D": 280, "R": 80, "K": 20}
TRANSPORT = ("own", "raw_cpr", "human", "program", "empty", "partial_error",
             "flush_error", "own_foreign", "foreign_own", "original_proof")
PROTOCOL = ("zero", "multiple", "split", "panic", "fifo_lease", "full", "disconnected", "failed_flush")
RECEIVER = ("baseline", "dsr_outer", "dsr_claim", "foreign_outer", "foreign_claim",
            "raw_cpr", "successor", "visible")
SKIPS = ("hwmon::tests::snapshot_has_all_sections",
         "attach_race_tests::process_and_send_are_atomic_under_parser_lock_no_dup_delivery")
TARGET = "schedule::tests::schedule_writers_serialize_under_the_lock_and_no_add_is_lost"

def plan():
    rows = [{"id": f"I/{i:03}", "arm": "I", "job": "I", "ordinal": i} for i in range(300)]
    for i in range(10):
        for cell in ("w1", "w2"):
            rows.append({"id": f"L/{cell}/{i:02}", "arm": "L", "job": "L", "cell": cell, "rep": i})
    rows += [{"id": f"L/integrity/{i:02}", "arm": "L", "job": "L", "cell": "integrity", "ordinal": i} for i in range(40)]
    for rep in range(10):
        for phase in (("outer", "claim") if rep % 2 == 0 else ("claim", "outer")):
            for cell in TRANSPORT:
                rows.append({"id": f"D/{rep:02}/{phase}/{cell}", "arm": "D", "job": "D", "rep": rep, "phase": phase, "cell": cell})
        for cell in PROTOCOL:
            rows.append({"id": f"D/{rep:02}/protocol/{cell}", "arm": "D", "job": "D", "rep": rep, "phase": "protocol", "cell": cell})
        order = RECEIVER[rep % 8:] + RECEIVER[:rep % 8]
        for cell in order:
            rows.append({"id": f"R/{rep:02}/{cell}", "arm": "R", "job": "R", "rep": rep, "cell": cell})
    for shard in range(1, 11):
        for position in (1, 2):
            rows.append({"id": f"K/{shard:02}/{position}", "arm": "K", "job": f"K{shard:02}", "shard": shard, "position": position})
    assert {a: sum(r["arm"] == a for r in rows) for a in COUNTS} == COUNTS
    return rows

def matrix():
    return ([{"job": a, "arm": a, "minutes": m} for a, m in [("I", 30), ("L", 40), ("D", 60), ("R", 60)]] +
            [{"job": f"K{i:02}", "arm": "K", "minutes": 100} for i in range(1, 11)])

def cost():
    jobs = matrix()
    return {"jobs": len(jobs), "max_parallel": 10, "runner_minutes": sum(j["minutes"] for j in jobs),
            "service_window_minutes_if_10_slots_immediate": 2 * max(j["minutes"] for j in jobs),
            "queue_delay_bound": None, "release_verdict": "NONE"}

def sha(data):
    return hashlib.sha256(data).hexdigest()

def atomic_json(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + f".{os.getpid()}.tmp")
    with temp.open("x", encoding="utf-8") as f:
        json.dump(value, f, ensure_ascii=False, indent=2, allow_nan=False); f.write("\n"); f.flush(); os.fsync(f.fileno())
    os.replace(temp, path)

def decode_json(raw):
    def pairs(values):
        result = {}
        for key, value in values:
            if key in result: raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result
    return json.loads(raw.decode('utf-8'), object_pairs_hook=pairs,
                      parse_constant=lambda x: (_ for _ in ()).throw(ValueError("nonfinite JSON: " + x)))

def read_json(path):
    return decode_json(Path(path).read_bytes())
