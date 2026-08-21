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
  3. `cys reap-surface <ref>` (★G4 W4-C: 전용 RPC surface.reap — 권위 role(master/cso)
     게이트·7조건 판정·감사 이벤트 3종·묘비 미생성=부활 대상 유지).
     구 바이너리 폴백: reap-surface 명령 부재(clap 사용오류 + unrecognized subcommand)가
     명시 관측될 때만 기존 `cys close-surface <ref> --reap` 경로를 유지한다(팩 무중단 적용).

★거부 사유별 처리(rc=7 = 게이트 거부 · stderr 사유 코드가 사실):
  - grace_not_elapsed / state_changed → 실패 아님(deferred). grace(기본 60s)는 포렌식·
    노드복구 창이며, 데몬 자동 reap 레인(reap_exited_surfaces · 기본 활성 · 큐 잔존 무관
    회수)이 grace 경과 시 어차피 회수한다 — 다음 사이클 재시도로 수렴.
  - queue_not_empty → `cys queue clear <ref>`(권위 role + exited 예외 = exited_reclaim,
    queue.dropped 감사) 선행 후 reap 1회 재시도하는 2단계. reap 이 큐를 자동 drop 하지
    않는 것은 설계다(인멸을 명시 행위로 강제).
  - 그 외(caller_unresolved·caller_role_forbidden·active_surface·agent_still_alive·
    daemon_ancestor) → 진짜 거부로 보고(ok=False). caller_* 는 이 스크립트를 master/cso
    pane 안에서 실행해야 한다는 신호다(수동 회수는 '누가'가 감사의 핵심 — 익명 금지).

★수동 RPC 의 실효 창(정직 표기): 데몬 자동 reap 레인이 기본 활성이므로, 이 도구의 실효는
  ①grace 경과 후 자동 틱(기본 30s 간격)보다 앞선 즉시 회수 ②자동 레인 비활성
  (CYS_REAP_EXITED=0) 환경 ③큐 잔존 좌석의 명시 2단계 정리 — 셋이다.

★안전 불변식(코드 강제·deny-by-default):
  - reap 은 오직 exited==true 로 수집된 ref 에만 호출된다. live(exited=false) surface 는
    어떤 인자·경로로도 대상이 되지 않는다(_plan 이 구조적으로 필터 + 데몬 게이트가
    active_surface 로 이중 거부 — 치명위험 앵커 ④).
  - 상태 조회 실패 = 아무것도 하지 않는다(fail-open: 오폭보다 미집행이 안전측).

exit: 0=정상(0건·deferred 포함·전건 성공) / 1=부분 실패(일부 거부·실패) / 2=상태 조회 불가(미집행).
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


# ★G4(W4-C): surface.reap 거부 사유 코드 어휘 — 데몬 manual_reap_denial(+caller_unresolved·
# state_changed)과 1:1 계약. CLI 가 rc=7 + stderr 에 사유 코드를 싣는다(cys.rs reap_surface_exit_code).
DENY_REASONS = ("caller_unresolved", "caller_role_forbidden", "active_surface",
                "agent_still_alive", "queue_not_empty", "daemon_ancestor",
                "grace_not_elapsed", "state_changed")


def parse_deny_reason(stderr):
    """rc=7 stderr 에서 사유 코드 추출 — 미지·부재는 None(보수적 = 미분류 처리)."""
    for r in DENY_REASONS:
        if r in (stderr or ""):
            return r
    return None


def _legacy_unavailable(rc, err):
    """구 바이너리 판정: reap-surface 서브커맨드 **부재의 명시 증거**(clap 사용오류 rc=2 +
    unrecognized subcommand)가 있을 때만 True. 그 외 비-0 은 절대 폴백하지 않는다(fail-closed
    — 게이트 거부를 구버전 close-surface 경로로 우회하면 신설 7조건·감사가 무력화된다)."""
    return rc == 2 and "unrecognized subcommand" in (err or "")


def _reap_one(ref, runner):
    """단일 ref 회수 시도 — (action, ok, detail) 반환. 사유별 분기는 여기 한 곳.
    action="unclassified" 는 호출측이 _still_present race 판정으로 마무리한다."""
    rc, _out, err = runner(["cys", "reap-surface", ref])
    if rc == 0:
        return "reaped", True, ""
    if _legacy_unavailable(rc, err):
        # 구 바이너리(reap-surface 부재) — 기존 경로 유지(팩 무중단 적용 · 확정 결정).
        rc2, _o2, err2 = runner(["cys", "close-surface", ref, "--reap"])
        if rc2 == 0:
            return "reaped-legacy", True, ""
        return "unclassified", False, (err2 or "").strip()[:200]
    reason = parse_deny_reason(err) if rc == 7 else None
    if reason in ("grace_not_elapsed", "state_changed"):
        # 실패 아님(deferred): grace 는 포렌식·복구 창이고, 데몬 자동 reap 레인·다음
        # 사이클이 수렴한다. partial-failure 허위 경보를 내지 않는다.
        return "deferred-" + reason, True, ""
    if reason == "queue_not_empty":
        # 2단계: 큐 인멸은 명시 행위(queue.clear exited_reclaim 예외·queue.dropped 감사)
        # 로만 — 정리 후 reap 1회 재시도.
        qrc, _qo, qerr = runner(["cys", "queue", "clear", ref])
        if qrc != 0:
            return "queue-clear-failed", False, (qerr or "").strip()[:200]
        rc3, _o3, err3 = runner(["cys", "reap-surface", ref])
        if rc3 == 0:
            return "reaped-after-queue-clear", True, ""
        return "reap-denied", False, (err3 or "").strip()[:200]
    if reason is not None:
        # caller_unresolved/caller_role_forbidden = master/cso pane 밖 실행 신호,
        # active_surface/agent_still_alive/daemon_ancestor = 진짜 거부 — 그대로 보고.
        return "reap-denied", False, (err or "").strip()[:200]
    return "unclassified", False, (err or "").strip()[:200]


def reap(targets, runner=_run, dry_run=False, log_dir=None):
    results = []
    for t in targets:
        ref = t["surface_ref"]
        snap = None if dry_run else snapshot(ref, runner=runner, log_dir=log_dir)
        if dry_run:
            results.append(dict(t, action="dry-run", snapshot=None, ok=True))
            continue
        action, ok, detail = _reap_one(ref, runner)
        if action == "unclassified" and not _still_present(ref, runner):
            # ★실측된 race(E2E 2026-07-16): fetch↔reap 사이 타 주체(데몬 자동 레인·CSO)가
            #   선회수. 대상 소멸 = 목적 달성 — partial-failure 허위 경보를 내지 않는다.
            results.append(dict(t, action="already-gone", snapshot=snap, ok=True,
                                detail=""))
            continue
        if action == "unclassified":
            action = "close-failed"   # 잔존 + 미분류 실패 — 기존 보고 어휘 유지(소비자 계약)
        results.append(dict(t, action=action, snapshot=snap, ok=ok, detail=detail))
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
        reaps = [c for c in calls if c[:2] == ["cys", "reap-surface"]]
        if reaps != [["cys", "reap-surface", "surface:2"]]:
            fails.append("④reap-surface 호출이 계획과 불일치(live 오폭 위험): %s" % reaps)
        if [c for c in calls if c[:2] == ["cys", "close-surface"]]:
            fails.append("④b 신 경로가 있는데 구 close-surface 를 호출함")

        def stub_snap_fail(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "read-screen"]:
                return 1, "", "exited pane unreadable"
            return 0, "", ""
        r2 = reap(t, runner=stub_snap_fail, log_dir=td)
        if not (r2[0]["ok"] and r2[0]["snapshot"] is None):
            fails.append("⑤스냅샷 실패 시 degrade(계속 reap) 위반: %s" % r2)

        def stub_close_fail(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "reap-surface"]:
                return 3, "", "boom"
            if cmd[:3] == ["cys", "status", "--json"]:
                # 재조회 시 대상이 '아직 존재' → 진짜 실패로 판정되어야
                return 0, json.dumps({"surfaces": [
                    {"surface_ref": "surface:2", "exited": True}]}), ""
            return 0, "x", ""
        r3 = reap(t, runner=stub_close_fail, log_dir=td)
        if r3[0]["ok"] or r3[0]["action"] != "close-failed":
            fails.append("⑥reap 실패가 ok로 위장됨(도구-증명 위반)")

        def stub_race_gone(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "reap-surface"]:
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
        if r4[0]["action"] != "dry-run" or len(
                [c for c in calls if c[:2] == ["cys", "reap-surface"]]) > 1:
            fails.append("⑧dry-run이 reap을 호출함")

        # ⑨ 구 바이너리 폴백: reap-surface 부재의 **명시 증거**(rc=2+unrecognized)에만
        #    기존 close-surface --reap 경로 유지. 일반 오류(rc=1 등)는 절대 폴백 금지.
        legacy_calls = []
        def stub_legacy(cmd, timeout=TIMEOUT):
            legacy_calls.append(cmd)
            if cmd[:2] == ["cys", "reap-surface"]:
                return 2, "", "error: unrecognized subcommand 'reap-surface'"
            if cmd[:2] == ["cys", "read-screen"]:
                return 0, "x", ""
            return 0, "", ""
        r5 = reap(t, runner=stub_legacy, log_dir=td)
        if not (r5[0]["ok"] and r5[0]["action"] == "reaped-legacy"):
            fails.append("⑨구 바이너리 폴백 실패: %s" % r5)
        if [c for c in legacy_calls if c[:2] == ["cys", "close-surface"]] != \
                [["cys", "close-surface", "surface:2", "--reap"]]:
            fails.append("⑨b 폴백 close-surface 호출 불일치: %s" % legacy_calls)
        if _legacy_unavailable(1, "some other error") or _legacy_unavailable(7, "reap_denied"):
            fails.append("⑨c 명시 증거 없는 비-0 이 폴백으로 오판됨(fail-closed 위반)")

        # ⑩ grace 미경과 = deferred(실패 아님) — 데몬 자동 레인·다음 사이클이 수렴.
        def stub_grace(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "reap-surface"]:
                return 7, "", "error: reap_denied: surface.reap denied: grace_not_elapsed"
            if cmd[:2] == ["cys", "read-screen"]:
                return 0, "x", ""
            return 0, "", ""
        r6 = reap(t, runner=stub_grace, log_dir=td)
        if not (r6[0]["ok"] and r6[0]["action"] == "deferred-grace_not_elapsed"):
            fails.append("⑩grace 미경과가 deferred 로 처리되지 않음: %s" % r6)

        # ⑪ queue_not_empty 2단계: queue clear(exited_reclaim) 선행 후 reap 재시도 성공.
        two_calls = []
        def stub_queue(cmd, timeout=TIMEOUT):
            two_calls.append(cmd)
            if cmd[:2] == ["cys", "reap-surface"]:
                n = len([c for c in two_calls if c[:2] == ["cys", "reap-surface"]])
                if n == 1:
                    return 7, "", "error: reap_denied: surface.reap denied: queue_not_empty"
                return 0, "", ""
            if cmd[:2] == ["cys", "read-screen"]:
                return 0, "x", ""
            return 0, "", ""
        r7 = reap(t, runner=stub_queue, log_dir=td)
        if not (r7[0]["ok"] and r7[0]["action"] == "reaped-after-queue-clear"):
            fails.append("⑪queue 2단계 실패: %s" % r7)
        seq = [c for c in two_calls if c[:2] in (["cys", "reap-surface"], ["cys", "queue"])]
        if seq != [["cys", "reap-surface", "surface:2"],
                   ["cys", "queue", "clear", "surface:2"],
                   ["cys", "reap-surface", "surface:2"]]:
            fails.append("⑪b 2단계 호출 순서 불일치: %s" % seq)

        # ⑫ 그 외 게이트 거부(caller_role_forbidden 등)는 진짜 거부로 보고(ok=False).
        def stub_forbidden(cmd, timeout=TIMEOUT):
            if cmd[:2] == ["cys", "reap-surface"]:
                return 7, "", "error: reap_denied: surface.reap denied: caller_role_forbidden"
            if cmd[:2] == ["cys", "read-screen"]:
                return 0, "x", ""
            return 0, "", ""
        r8 = reap(t, runner=stub_forbidden, log_dir=td)
        if r8[0]["ok"] or r8[0]["action"] != "reap-denied":
            fails.append("⑫게이트 거부가 ok 로 위장됨: %s" % r8)
    finally:
        shutil.rmtree(td, ignore_errors=True)

    if fails:
        sys.stderr.write("\n".join(fails) + "\n")
        return 1
    print(json.dumps({"self_test": "ok", "cases": 14,
                      "covers": "불변식(bool엄격·ref검증)·빈입력·정상reap(신RPC)·스냅샷·"
                                "degrade·reap실패보고·선회수race·조회불가미집행·dry-run·"
                                "구바이너리폴백(명시증거한정)·grace-deferred·queue2단계·게이트거부",
                     },
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
