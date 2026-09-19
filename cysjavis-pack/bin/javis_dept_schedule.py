#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_dept_schedule.py — 부서 레인 schedule 시드 필터 (결재 13).

## 무엇이 결함이었나 (실측 2026-09-19)
`cys-dept` 가 부서 데몬 ready **이후** `schedule.json` 을 `{"jobs": []}` 로 덮어썼다. 의도는
owner-progress 보고의 부서 중복 실행 차단이었는데, **레인 지역 기계장치까지 함께 지워졌다**:
`cso-alert-inbox-check-60m` · `cycle-autopilot-tick` · `cycle-verifier-watchdog` ·
`phoenix-snapshot-6h`. 기제는 **순서**다 — cysd 는 부트 때 accept 루프 이전에
`ensure_builtin_jobs()` 로 이 잡들을 넣고, 핫리로드는 ensure 를 다시 돌리지 않는다.
실측 당시 dept-1 `jobs=1`(owner 잡만 잔존) · dept-2 `jobs=0`.

## 처방
**덮어쓰지 않고 거른다.** 데몬이 넣어 둔 잡을 보존하고 **owner/CEO 스코프 id 만** 제외한다.
잡 정의를 복제하지 않으므로 `src/bin/cysd/schedule.rs` 의 `BUILTIN_JOBS_VERSION` 과 드리프트하지 않는다.
`base_only` 잡은 남겨도 데몬의 `base_only_blocked` 관문이 부서 실행을 막으므로 이중 삭제하지 않는다.

사용: `javis_dept_schedule.py <부서팩>/schedule.json [--self-test]`
exit: 0=시드(또는 멱등 무변경) · 1=판독 불가(원본 불변) · 2=쓰기 실패(원본 불변)
"""
import json
import os
import sys
import tempfile

DOC = ("부서 데몬 schedule — owner/CEO 스코프 잡만 제외. "
       "레인 지역 builtin(경보·사이클·스냅샷)은 데몬이 넣은 그대로 보존.")

# 최상위(owner/CEO) 데몬 전용 — 부서마다 돌면 중복 보고·자원 낭비다.
OWNER_SCOPE = frozenset({
    "owner-progress-gate-5min",
    "fleet-adoption-cost-digest",
    "fleet-digest",
    "learn-ttl-audit",
    "content-channel-health-watch",
})

# 레인 지역 기계장치 — 부서마다 자기 것이 돌아야 한다(결재 13 의 보존 대상).
LANE_LOCAL = ("cso-alert-inbox-check-60m", "cycle-autopilot-tick",
              "cycle-verifier-watchdog", "phoenix-snapshot-6h")


def filter_jobs(jobs):
    """(남길 잡, 제외한 id) — 순수. 순서 보존."""
    kept = [j for j in jobs if j.get("id") not in OWNER_SCOPE]
    dropped = [j.get("id") for j in jobs if j.get("id") in OWNER_SCOPE]
    return kept, dropped


def seed(path):
    try:
        d = json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {"jobs": []}
    except (OSError, ValueError) as e:
        sys.stderr.write("[cys-dept] schedule.json 판독 불가(%s) — 원본 불변\n" % e)
        return 1
    jobs = d.get("jobs") or []
    kept, dropped = filter_jobs(jobs)
    if os.path.exists(path) and not dropped and d.get("_doc") == DOC:
        return 0                                    # 멱등 — 쓸 것이 없다
    out = {"_doc": DOC, "jobs": kept}
    dirn = os.path.dirname(os.path.abspath(path)) or "."
    try:
        os.makedirs(dirn, exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=dirn, prefix=".sched.", suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(out, fh, ensure_ascii=False, indent=2)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)                       # 원자 교체
    except OSError as e:
        sys.stderr.write("[cys-dept] schedule 쓰기 실패(%s) — 원본 불변\n" % e)
        return 2
    have = [j.get("id") for j in kept if j.get("id") in LANE_LOCAL]
    sys.stderr.write("[cys-dept] schedule: owner 스코프 %d건 제외(%s) · 보존 %d건 · 레인 지역 %d/%d\n"
                     % (len(dropped), ",".join(d for d in dropped if d) or "-",
                        len(kept), len(have), len(LANE_LOCAL)))
    return 0


def _self_test():
    fails = []

    def ck(name, cond, detail=""):
        print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
        if not cond:
            fails.append(name)

    lane = [{"id": i, "_builtin": "x"} for i in LANE_LOCAL]
    owner = [{"id": "owner-progress-gate-5min"}, {"id": "fleet-digest"}]
    kept, dropped = filter_jobs(owner[:1] + lane + owner[1:])
    ck("① 레인 지역 4종 보존", [j["id"] for j in kept] == list(LANE_LOCAL))
    ck("② owner 스코프만 제외", set(dropped) == {"owner-progress-gate-5min", "fleet-digest"})
    ck("③ base_only 잡은 지우지 않는다(데몬 관문 소관)",
       filter_jobs([{"id": "formation-heartbeat", "base_only": True}])[0] != [])

    d = tempfile.mkdtemp(prefix="deptsched-")
    p = os.path.join(d, "schedule.json")
    json.dump({"jobs": owner[:1] + lane}, open(p, "w", encoding="utf-8"))
    ck("④ 시드 exit 0", seed(p) == 0)
    got = json.load(open(p, encoding="utf-8"))
    ck("⑤ ★레인 지역 4종이 파일에 남는다(종전 결함의 회귀 핀)",
       [j["id"] for j in got["jobs"]] == list(LANE_LOCAL), str([j["id"] for j in got["jobs"]]))
    ck("⑥ owner 잡이 빠졌다", all(j["id"] not in OWNER_SCOPE for j in got["jobs"]))
    before = open(p, encoding="utf-8").read()
    ck("⑦ 재실행 멱등(바이트 동일)", seed(p) == 0 and open(p, encoding="utf-8").read() == before)
    bad = os.path.join(d, "broken.json")
    open(bad, "w", encoding="utf-8").write("{ not json")
    ck("⑧ 판독 불가는 exit 1 · 원본 불변",
       seed(bad) == 1 and open(bad, encoding="utf-8").read() == "{ not json")
    p2 = os.path.join(d, "fresh", "schedule.json")
    ck("⑨ 파일 부재여도 실패하지 않는다(빈 jobs 시드)",
       seed(p2) == 0 and json.load(open(p2, encoding="utf-8"))["jobs"] == [])
    print("\n=== %d/%d PASS ===" % (9 - len(fails), 9))
    return 1 if fails else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        raise SystemExit(_self_test())
    if len(sys.argv) < 2:
        sys.stderr.write("사용: javis_dept_schedule.py <schedule.json> [--self-test]\n")
        raise SystemExit(2)
    raise SystemExit(seed(sys.argv[1]))
