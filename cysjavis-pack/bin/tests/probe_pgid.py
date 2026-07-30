#!/usr/bin/env python3
"""probe_pgid.py — A18 결정 실험: 훅 발화 부트가 하네스 group-kill 에 노출되는가 (T-0147-7 W1b).

**이것은 처방이 아니라 측정이다.** 재감사 A18 은 PARTIAL(P2→P3, 조건부 복귀 예약)로 핀됐다:
  · 확정: `setsid` 는 macOS 에 없어 role-bootstrap.sh 의 1순위 분기가 사문(死文)이다.
  · 기각: cysd 의 '그룹정리 동사'가 부트를 죽인다는 원 메커니즘(트리워크 개별 kill·재부모화로 이탈).
  · **잔존·미실측**: 포그라운드 pgrp Ctrl-C / **하네스 group-kill**. 이걸 재는 게 이 스크립트다.
  · 처방(os.setsid 선제 내재화)은 철회됐다 — 훅이 이미 `setsid "$CYS_PY" "$BOOT"` 를 부르므로
    세션 리더에서 os.setsid()는 PermissionError 로 훅 경로 전체를 크래시시킨다(비평2 B-6).

측정 방법(실측·격리):
  ① 하네스 모사: `start_new_session=True` 로 sh 를 띄운다 → 그 sh 는 **자기 세션·프로세스그룹의
     리더**다(= claude 가 잡 컨트롤 아래 자기 그룹을 갖는 상황의 모형).
  ② 그 sh 안에서 **role-bootstrap.sh 와 동일한 발화 형태**로 자식(부트 모사)을 스폰한다:
     `setsid <py> child` → 없으면 `nohup <py> child &` → 없으면 `<py> child & disown`.
  ③ 자식은 자기 pid/pgid/sid 를 파일에 적고 대기한다.
  ④ 하네스 **그룹 전체**에 SIGTERM(`kill(-pgid)`)을 보낸다 = 하네스 group-kill.
  ⑤ 자식 생존을 확인한다. **생존=그룹킬 미도달(잔존 위협 없음) / 사망=그룹킬 도달(A18 P2 복귀 근거)**.

출력: stdout 1개 JSON(기계 판독) + 사람이 읽는 요약. **코드는 아무것도 수정하지 않는다.**
사용: python3 probe_pgid.py [--json]
"""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time

CHILD = r'''
import os, sys, time
p = sys.argv[1]
with open(p, "w") as f:
    f.write("%d %d %d\n" % (os.getpid(), os.getpgrp(), os.getsid(0)))
    f.flush()
try:
    time.sleep(60)
except Exception:
    pass
'''


def _read_ids(path, timeout=10.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with open(path) as f:
                parts = f.read().split()
            if len(parts) == 3:
                return [int(x) for x in parts]
        except OSError:
            pass
        time.sleep(0.05)
    return None


def _alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def control_arm():
    """★계측 타당성 게이트(MEMORY '디버깅 계측 타당성 게이트 3칙'): 이 프로브가 '항상 사망'을
    내는 고장 계측기가 아님을 증명한다. `setsid(1)` 이 있는 환경과 **동등한** 스폰(자식이
    `os.setsid()` 로 새 세션 리더가 되는 형태)을 만들어, 같은 group-kill 에서 **생존**하는지 본다.
    생존해야 관측치(nohup 분기 사망)가 의미를 갖는다.
    """
    py = sys.executable or "python3"
    tmp = tempfile.mkdtemp(prefix="pgid-ctrl-")
    childpy = os.path.join(tmp, "child.py")
    ids_file = os.path.join(tmp, "ids.txt")
    with open(childpy, "w") as f:
        f.write("import os\nos.setsid()\n" + CHILD)   # setsid(1) 등가
    script = '"$PY" "$CHILD" "$IDS" >/dev/null 2>&1 &\nsleep 60\n'
    h = subprocess.Popen(["sh", "-c", script],
                         env=dict(os.environ, PY=py, CHILD=childpy, IDS=ids_file),
                         start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        hpgid = os.getpgid(h.pid)
        ids = _read_ids(ids_file)
        if ids is None:
            return {"ok": False, "why": "대조군 자식 식별자 기록 실패"}
        cpid, cpgid, _csid = ids
        os.kill(-hpgid, signal.SIGTERM)
        time.sleep(0.6)
        survived = _alive(cpid)
        try:
            os.kill(cpid, signal.SIGKILL)
        except OSError:
            pass
        return {"ok": bool(survived), "pgid_separated": cpgid != hpgid,
                "why": ("세션 분리 스폰은 group-kill 에서 생존 — 계측기가 차이를 감지한다"
                        if survived else
                        "세션 분리 스폰도 사망 — **계측기 고장**(항상 사망을 내는 프로브의 관측치는 무의미)")}
    finally:
        try:
            os.kill(-os.getpgid(h.pid), signal.SIGKILL)
        except OSError:
            pass
        try:
            h.wait(timeout=10)
        except Exception:
            pass


def probe():
    py = sys.executable or "python3"
    have_setsid = subprocess.run(["sh", "-c", "command -v setsid"],
                                 capture_output=True).returncode == 0
    have_nohup = subprocess.run(["sh", "-c", "command -v nohup"],
                                capture_output=True).returncode == 0
    result = {"platform": sys.platform, "setsid_available": have_setsid,
              "nohup_available": have_nohup,
              "hook_branch": "setsid" if have_setsid else ("nohup" if have_nohup else "bare&")}
    tmp = tempfile.mkdtemp(prefix="pgid-probe-")
    childpy = os.path.join(tmp, "child.py")
    ids_file = os.path.join(tmp, "ids.txt")
    hids_file = os.path.join(tmp, "harness.txt")
    with open(childpy, "w") as f:
        f.write(CHILD)

    # ── 훅과 **동일한 분기 순서**의 발화 스크립트(role-bootstrap.sh:설명 참조) ──
    if have_setsid:
        spawn = 'setsid "$PY" "$CHILD" "$IDS" >/dev/null 2>&1 &'
    elif have_nohup:
        spawn = 'nohup "$PY" "$CHILD" "$IDS" >/dev/null 2>&1 &'
    else:
        spawn = '"$PY" "$CHILD" "$IDS" >/dev/null 2>&1 & disown 2>/dev/null'
    script = (
        'printf "%%s %%s\\n" "$$" "$(ps -o pgid= -p $$ | tr -d \' \')" > "$HIDS"\n'
        + spawn + "\n"
        "sleep 60\n")
    harness = subprocess.Popen(["sh", "-c", script],
                               env=dict(os.environ, PY=py, CHILD=childpy,
                                        IDS=ids_file, HIDS=hids_file),
                               start_new_session=True,          # ① 하네스 = 자기 세션/그룹 리더
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        hpgid = os.getpgid(harness.pid)
        result["harness"] = {"pid": harness.pid, "pgid": hpgid,
                             "is_group_leader": hpgid == harness.pid}
        ids = _read_ids(ids_file)
        if ids is None:
            result["error"] = "자식(부트 모사)이 자기 식별자를 기록하지 못했다 — 측정 실패"
            return result
        cpid, cpgid, csid = ids
        result["child"] = {"pid": cpid, "pgid": cpgid, "sid": csid}
        result["pgid_separated"] = (cpgid != hpgid)
        # ④ 하네스 그룹 전체 SIGTERM
        try:
            os.kill(-hpgid, signal.SIGTERM)
            result["group_kill_sent"] = "SIGTERM to pgid -%d" % hpgid
        except OSError as e:
            result["error"] = "group-kill 전송 실패: %s" % e
            return result
        time.sleep(0.6)
        survived = _alive(cpid)
        result["child_survived_group_kill"] = survived
        result["verdict"] = ("boot_escapes_group_kill" if survived
                             else "boot_killed_by_group_kill")
        result["control_arm"] = control_arm()      # 계측 타당성(항상-사망 프로브 배제)
        result["interpretation"] = (
            "자식(부트)이 하네스 group-kill 을 넘어 생존 — A18 잔존 위협(group-kill) **미성립**. "
            "조건부 내재화(os.setsid) 트리거 불충족 → A18 P3 유지."
            if survived else
            "자식(부트)이 하네스 group-kill 로 사망 — A18 잔존 위협 **성립**. "
            "H-CONC-5 트리거 충족 → A18 P2 복귀 및 조건부 내재화(세션 리더 no-op 가드 필수) 검토 대상.")
        # 정리
        try:
            os.kill(cpid, signal.SIGKILL)
        except OSError:
            pass
        return result
    finally:
        try:
            os.kill(-os.getpgid(harness.pid), signal.SIGKILL)
        except OSError:
            pass
        try:
            harness.wait(timeout=10)
        except Exception:
            pass


def main(argv):
    if os.name != "posix":
        out = {"skipped": "posix 전용 측정(Windows 는 프로세스 그룹 의미론이 다르다 — H-WIN-11 소속)"}
        print(json.dumps(out, ensure_ascii=False, indent=1))
        return 0
    r = probe()
    if "--json" in argv:
        print(json.dumps(r, ensure_ascii=False, indent=1))
        return 0 if "error" not in r else 1
    print(json.dumps(r, ensure_ascii=False, indent=1))
    print()
    print("── A18 측정 요약 ──")
    print("플랫폼: %s · 훅 발화 분기: %s (setsid=%s)"
          % (r["platform"], r.get("hook_branch"), r["setsid_available"]))
    if "error" in r:
        print("측정 실패: %s" % r["error"])
        return 1
    print("하네스 pgid=%s · 자식 pgid=%s → pgid 분리=%s"
          % (r["harness"]["pgid"], r["child"]["pgid"], r["pgid_separated"]))
    print("group-kill 후 자식 생존=%s → 판정: %s"
          % (r["child_survived_group_kill"], r["verdict"]))
    ca = r.get("control_arm") or {}
    print("계측 타당성(대조군 세션분리 스폰): ok=%s — %s" % (ca.get("ok"), ca.get("why")))
    print(r["interpretation"])
    if ca.get("ok") is False:
        print("⚠ 계측기 고장 — 위 판정을 신뢰하지 마라(대조군이 생존하지 못했다).")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
