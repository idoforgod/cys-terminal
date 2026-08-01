#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_reap_exited.py — exited surface 결정론 자동 회수(reap) 도구.

CSO_DIRECTIVE [절대규칙 — exited surface 자동 reap](제품 기본 절차 · 즉시성 포함)의
산문 조항을 코드 불변식으로 격상한다. CSO는 능동 모니터링 사이클/이벤트 수신마다 이 스크립트를
1콜 실행하며, 판단은 이 스크립트의 exit code·stdout JSON만이 사실이다(LLM 자연어 재추론 금지).

동작:
  1. `cys status --json` → surfaces[].exited == true 필터 (데몬 권위 판정.
     ★화면 파싱·`cys list` 텍스트 파싱 금지 — JSON 계약만 사용.)
  2. 각 대상: `cys read-screen --surface <ref>` 스냅샷 → <pack>/round/reap_log/
     (사후 부검 증거 보존. 실패 시 사유만 기록하고 reap은 계속 — 잔재 회수가 1목적.)
  3. `cys close-surface <ref> --reap` (묘비 미생성·부활 대상 유지 = 죽은 잔재 회수 모드).

★안전 불변식(코드 강제·deny-by-default):
  - close-surface 는 오직 exited==true 로 수집된 ref 에만 호출된다. live(exited=false)
    surface 는 어떤 인자·경로로도 대상이 되지 않는다(_plan 이 구조적으로 필터).
  - 상태 조회 실패 = 아무것도 하지 않는다(fail-open: 오폭보다 미집행이 안전측).

exit: 0=정상(0건 포함·전건 성공) / 1=부분 실패(일부 close 실패) / 2=상태 조회 불가(미집행).
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

TIMEOUT = 20


def _pack_dir():
    return os.environ.get("CYS_PACK_DIR", "").strip() or os.path.join(
        os.path.expanduser("~"), ".cys", "pack")


def _log_dir():
    return os.path.join(_pack_dir(), "round", "reap_log")


def _run(cmd, timeout=TIMEOUT):
    """subprocess 러너 — (rc, stdout, stderr). 예외도 rc!=0로 정규화(fail-soft)."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout or "", p.stderr or ""
    except Exception as e:
        return 127, "", "runner error: %s" % e


def fetch_surfaces(runner=_run):
    """cys status --json → surfaces 리스트. 조회 불가 시 None(fail-open 신호)."""
    rc, out, err = runner(["cys", "status", "--json"])
    if rc != 0:
        return None
    try:
        d = json.loads(out)
        s = d.get("surfaces")
        return s if isinstance(s, list) else None
    except (ValueError, AttributeError):
        return None


def plan_reaps(surfaces):
    """★불변식 지점: exited==True(bool 엄격 비교)인 row의 surface_ref만 통과.
    truthy 오염(문자열 'false' 등)·ref 부재 row는 제외 — live 오폭 구조 차단."""
    targets = []
    for s in surfaces or []:
        if not isinstance(s, dict):
            continue
        if s.get("exited") is not True:
            continue
        ref = s.get("surface_ref")
        if isinstance(ref, str) and ref.strip():
            targets.append({"surface_ref": ref.strip(),
                            "role": s.get("role"), "title": s.get("title")})
    return targets


def snapshot(ref, runner=_run, log_dir=None):
    """read-screen 스냅샷 → reap_log. 실패해도 reap 은 진행(경로 or None 반환)."""
    d = log_dir or _log_dir()
    rc, out, err = runner(["cys", "read-screen", "--surface", ref])
    if rc != 0 or not out.strip():
        return None
    try:
        os.makedirs(d, exist_ok=True)
        safe = re.sub(r"[^0-9A-Za-z_-]", "_", ref)
        path = os.path.join(d, "%s-%s.txt" % (
            time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()), safe))
        with open(path, "w", encoding="utf-8") as f:
            f.write(out)
        return path
    except OSError:
        return None


def _still_present(ref, runner):
    """close 실패 후 재조회 — 대상이 이미 소멸했으면 실패가 아니라 성공(무해 race)."""
    surfaces = fetch_surfaces(runner=runner)
    if surfaces is None:
        return True   # 판정 불가 → 보수적으로 '존재'로 보고 실패 유지(허위 성공 금지)
    return any(isinstance(s, dict) and s.get("surface_ref") == ref for s in surfaces)


def reap(targets, runner=_run, dry_run=False, log_dir=None):
    results = []
    for t in targets:
        ref = t["surface_ref"]
        snap = None if dry_run else snapshot(ref, runner=runner, log_dir=log_dir)
        if dry_run:
            results.append(dict(t, action="dry-run", snapshot=None, ok=True))
            continue
        rc, out, err = runner(["cys", "close-surface", ref, "--reap"])
        if rc != 0 and not _still_present(ref, runner):
            # ★실측된 race(E2E 2026-07-16): fetch↔close 사이 타 주체(데몬·CSO)가 선회수.
            #   대상 소멸 = 목적 달성 — partial-failure 허위 경보를 내지 않는다.
            results.append(dict(t, action="already-gone", snapshot=snap, ok=True,
                                detail=""))
            continue
        results.append(dict(t, action="reaped" if rc == 0 else "close-failed",
                            snapshot=snap, ok=(rc == 0),
                            detail=(err.strip()[:200] if rc != 0 else "")))
    return results


def self_test():
    """fixture 기반(라이브 데몬 불요) — 불변식·degrade·필터 검증."""
    fails = []
    fx = [
        {"surface_ref": "surface:1", "exited": False, "role": "master"},
        {"surface_ref": "surface:2", "exited": True, "role": "worker"},
        {"surface_ref": "surface:3", "exited": "true", "role": "cso"},   # 문자열 오염
        {"surface_ref": "", "exited": True},                              # ref 부재
        {"exited": True},                                                  # ref 키 없음
        "garbage",                                                         # row 오염
    ]
    t = plan_reaps(fx)
    if [x["surface_ref"] for x in t] != ["surface:2"]:
        fails.append("①불변식: exited==True(bool)·유효 ref만 통과해야 함 → %s" % t)
    if plan_reaps(None) != [] or plan_reaps([]) != []:
        fails.append("②빈 입력이 빈 계획이 아님")

    calls = []
    def stub_ok(cmd, timeout=TIMEOUT):
        calls.append(cmd)
        if cmd[:2] == ["cys", "read-screen"]:
            return 0, "final screen text", ""
        return 0, "", ""
    import tempfile, shutil
    td = tempfile.mkdtemp(prefix="reap-test-")
    try:
        r = reap(t, runner=stub_ok, log_dir=td)
        if not (len(r) == 1 and r[0]["ok"] and r[0]["action"] == "reaped"):
            fails.append("③정상 reap 실패: %s" % r)
        if not (r[0]["snapshot"] and os.path.exists(r[0]["snapshot"])):
            fails.append("③스냅샷 파일 미생성")
        closes = [c for c in calls if c[:2] == ["cys", "close-surface"]]
        if closes != [["cys", "close-surface", "surface:2", "--reap"]]:
            fails.append("④close-surface 호출이 계획과 불일치(live 오폭 위험): %s" % closes)

        def stub_snap_fail(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "read-screen"]:
                return 1, "", "exited pane unreadable"
            return 0, "", ""
        r2 = reap(t, runner=stub_snap_fail, log_dir=td)
        if not (r2[0]["ok"] and r2[0]["snapshot"] is None):
            fails.append("⑤스냅샷 실패 시 degrade(계속 reap) 위반: %s" % r2)

        def stub_close_fail(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "close-surface"]:
                return 3, "", "boom"
            if cmd[:3] == ["cys", "status", "--json"]:
                # 재조회 시 대상이 '아직 존재' → 진짜 실패로 판정되어야
                return 0, json.dumps({"surfaces": [
                    {"surface_ref": "surface:2", "exited": True}]}), ""
            return 0, "x", ""
        r3 = reap(t, runner=stub_close_fail, log_dir=td)
        if r3[0]["ok"] or r3[0]["action"] != "close-failed":
            fails.append("⑥close 실패가 ok로 위장됨(도구-증명 위반)")

        def stub_race_gone(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "close-surface"]:
                return 3, "", "no such surface"
            if cmd[:3] == ["cys", "status", "--json"]:
                return 0, json.dumps({"surfaces": []}), ""   # 재조회: 이미 소멸
            return 0, "x", ""
        r3b = reap(t, runner=stub_race_gone, log_dir=td)
        if not (r3b[0]["ok"] and r3b[0]["action"] == "already-gone"):
            fails.append("⑥b 선회수 race가 허위 실패로 보고됨: %s" % r3b)

        def stub_status_fail(cmd, timeout=TIMEOUT):
            return 1, "", "daemon down"
        if fetch_surfaces(runner=stub_status_fail) is not None:
            fails.append("⑦상태 조회 실패가 None(미집행 신호)이 아님")
        r4 = reap(t, runner=stub_ok, dry_run=True, log_dir=td)
        if r4[0]["action"] != "dry-run" or [c for c in calls if c[1] == "close-surface" and "surface:2" in c].__len__() > 1:
            fails.append("⑧dry-run이 close를 호출함")
    finally:
        shutil.rmtree(td, ignore_errors=True)

    if fails:
        sys.stderr.write("\n".join(fails) + "\n")
        return 1
    print(json.dumps({"self_test": "ok", "cases": 9,
                      "covers": "불변식(bool엄격·ref검증)·빈입력·정상reap·스냅샷·"
                                "degrade·close실패보고·선회수race·조회불가미집행·dry-run"},
                     ensure_ascii=False))
    return 0


def main():
    ap = argparse.ArgumentParser(description="exited surface 결정론 자동 reap")
    ap.add_argument("--dry-run", action="store_true", help="계획만 출력(집행 없음)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        sys.exit(self_test())

    surfaces = fetch_surfaces()
    if surfaces is None:
        print(json.dumps({"status": "status-unavailable", "reaped": [],
                          "note": "데몬 상태 조회 불가 — 미집행(fail-open)"},
                         ensure_ascii=False))
        sys.exit(2)
    targets = plan_reaps(surfaces)
    results = reap(targets, dry_run=args.dry_run)
    ok = all(r["ok"] for r in results)
    print(json.dumps({"status": "ok" if ok else "partial-failure",
                      "exited_found": len(targets), "results": results},
                     ensure_ascii=False))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
