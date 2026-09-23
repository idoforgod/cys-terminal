#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_inject_context_role_seat.py — WP-7 O: 동일 cwd 세션 카운트의 역할 좌석 강등 핀 (0.14.31).

무엇을 막는가: 데몬은 좌석을 전부 같은 cwd 로 띄운다(실측 2026-09-06 본부 12좌석 cwd=오너 홈).
그 형상에 race 경고를 물리면 경고가 **상시 참**이 되어 아무도 읽지 않는다(경보 피로 · 버그리포트 B1).
0.14.31 부터 역할 좌석은 경고 대신 사실 1줄을 받고, 무역할 세션만 종전 경고를 받는다.

밀폐: PATH 스텁 `ps`/`lsof`/`cys` 로 세션 카운트 형상을 만든다 — 라이브 프로세스·데몬 무접촉.
      cwd 는 임시 디렉터리, 팩도 임시(체크리스트·게이트 경로 부재로 그 분기는 안 탄다).

핀 목록
  ① 역할 좌석(데몬 rc0 + 역할) → 정보 1줄("역할 좌석 포함") · 경고 문구 0 · exit 0
  ② ★확정 무역할(데몬 rc0 + 빈 줄)은 CYS_ROLE env 가 있어도 **경고**다
     — 역할이 풀린 좌석의 env 잔재가 경고를 영영 끄면 안 된다(plan §8: env 는 권위가 아니다)
  ③ 판정 불가(데몬 rc≠0) → env 폴백으로 역할 좌석 취급(정보 1줄)
  ④ ★CR 오염(Windows 네이티브 cys 의 `\r\n`)이 무역할을 역할 좌석으로 둔갑시키지 않는다
  ⑤ cys 부재 + env 있음 → 정보 1줄 · cys 부재 + env 없음 → 경고
  ⑥ ★부트 폭주 봉인: 조회에 CYS_NO_AUTOSTART=1 이 걸려 있다(SessionStart 가 데몬을 낳지 않는다)
  ⑦ ★음성 대조: 세션 1개(SHARE<2)면 정보도 경고도 없다 — 그리고 **lead 좌석(env master·cso*)은**
     그때 조회 자체를 안 한다(0.14.41 U13 개정: 역할별 복원 신호가 역할을 알아야 하므로 그 밖의
     좌석은 훅 1회당 **최대 1회** 조회한다 — 정본 캐시·백오프·2s 상한 그대로 · 아래 ⑬)
  ⑧ ★R1(리뷰 minor): **모든** 역할 경로가 한 줄로 잘린다 — 데몬 응답만 `head -n1` 하고 env 폴백을
     안 자르면 여러 줄 CYS_ROLE 이 정보 1줄을 여러 줄로 부풀려 SessionStart 컨텍스트에 들어간다
  ⑨~⑮ ★U13 착수 게이트(0.14.41 · 오너 지시 2026-09-23 · WP-C1) — 복원 신호 줄의 역할 분기
     ⑨ member(역할이 확정된 비-master·비-cso 좌석) = '이어서 진행'·'미해결 게이트부터 재개' 0 ·
        착수 게이트 문안(단일 원본 `_lib.sh:cys_start_gate_note`) 1줄
     ⑩ 부서 레인 member = 부서장 SESSION_STATE **본문 0** · 'master 소관' 1줄 + 경로 · 체크리스트 미실행
     ⑪ 역할 미상(판별 실패) = 중립 문안 · 그러나 작업기억 본문은 **유지**(그 좌석이 부서장 자신일 수
        있다 — 작업기억은 복원 생명선이다)
     ⑫ ★lead(master·cso*) = 종전 출력 **바이트 동일**(문자열 핀 상시 + 기준 커밋 훅과의 전문 대조 ·
        git 기준 커밋이 없으면 전문 대조만 정직 SKIP)
     ⑬ 비용 — lead env 는 조회 0 · 그 밖은 훅 1회당 조회 ≤1 · 신원(surface id) 없으면 0
     ⑭ compact(자동 압축 직후)는 역할 무관 종전 문안(진행 중 턴의 연속이지 새 착수가 아니다)
     ⑮ 단일 원본 문안 계약 — 비어 있지 않음 · 백슬래시 0(`printf '%b'` 소비) · `set-status`·`--ack`·
        `CYS_BOOT_NONCE`·'부트 브리지' 낱말 0(session-start.sh 소비처 계약 · WP-E 가 참조한다)
     ⑮b ★부트 체인 안전(속행 · WP-C1 471d08b2 직후 발견): 운영 절차 예외 목록에 `javis_boot_node.py
        awaken_message`("즉시 각성하라")도 들어있다 — 없으면 신규 좌석 부트 주입이 착수 게이트에
        '배경 참고'로 읽혀 ①claim-role ②set-status ③TODO 확인 ④각성 보고가 멈출 수 있다(온보딩 위험)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 INJECT-CONTEXT-ROLE-SEAT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_inject_context_role_seat.py
해석기: 훅의 shebang(`#!/bin/bash`)을 따라 **bash** 로 부른다 — Ubuntu 의 `/bin/sh`(dash)는 훅 :34 의
       here-string 을 받지 못한다(실측 rc 2). POSIX 전용 검체이므로 Windows 레그에는 등재하지 않는다.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.normpath(os.path.join(SELF, "..", "..", "hooks", "inject-context.sh"))
# ★R2(수렴 · 리뷰 minor "Ubuntu 레인에서 이 검체가 적색"): 훅은 `#!/bin/bash` 이고 :34 에
#   **bash 전용 here-string**(`<<< "$_PARSED"`)을 쓴다. macOS 의 `sh` 는 posix 모드 bash 라 그것을
#   받지만 Ubuntu 의 `/bin/sh` 는 dash 이고 받지 못한다 — 실측: `dash -n hooks/inject-context.sh`
#   → rc 2 `Syntax error: redirection unexpected`. 이 검체가 등재된 두 레인(release
#   `pack-artifacts` · pack-release `pack-only`)은 **ubuntu-latest** 라, `sh` 로 부르면 훅의 결함이
#   아니라 **호출 규약의 오류**로 레인이 적색이 되고 서명 전에 팩 발행이 멈춘다.
#   저장소 규약과도 어긋났다 — `run_bootstrap_health.py` 는 같은 훅을 `[BASH, hook]` 으로 부른다.
#   해석기는 훅의 shebang 이 정하고, 이 검체는 그 shebang 을 따른다.
BASH = shutil.which("bash") or "/bin/bash"
INFO_MARK = "역할 좌석 포함"
WARN_MARK = "동시에 도는 claude 세션이"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _write_exec(path, body):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, 0o755)


def run_hook(tmp, seats, cys_mode, role_env=None, cys_present=True):
    """seats = 같은 cwd 로 잡히는 claude 세션 수(스텁 lsof 가 그만큼 `n<cwd>` 를 낸다).
    cys_mode: role|none|unjudged|cr  (스텁 `cys surface-role` 의 응답 형상)."""
    bindir = os.path.join(tmp, "stubbin")
    pack = os.path.join(tmp, "pack")
    cwd = os.path.join(tmp, "work")
    for d in (bindir, pack, cwd):
        os.makedirs(d, exist_ok=True)
    # 스텁 ps — awk 가 comm=claude 로 잡는 형상(실측 ⓐ 런처 실행)
    rows = "".join("echo '%d claude /Users/x/.local/bin/claude --x'\n" % (9000 + i)
                   for i in range(max(seats, 1)))
    _write_exec(os.path.join(bindir, "ps"), "#!/bin/sh\n" + rows)
    # 스텁 lsof — 요청한 pid 집합에 대해 cwd 를 seats 번 낸다
    _write_exec(os.path.join(bindir, "lsof"),
                "#!/bin/sh\n" + "".join("printf 'n%s\\n'\n" % cwd for _ in range(seats)))
    if cys_present:
        body = {"role": 'printf "cso\\n"; exit 0',
                "none": 'exit 0',
                "unjudged": 'exit 3',
                "cr": 'printf "\\r\\n"; exit 0'}[cys_mode]
        _write_exec(os.path.join(bindir, "cys"),
                    "#!/bin/sh\n"
                    'printf "%%s\\n" "${CYS_NO_AUTOSTART:-<unset>}" >> "%s/autostart.log"\n'
                    'case "$1" in surface-role) %s ;; esac\nexit 0\n' % (tmp, body))
    env = {k: v for k, v in os.environ.items()
           if k not in ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SOCKET",
                        "CYS_GATE_LANE_SOCKET", "CYS_SOUL")}
    # PATH 는 스텁만 + 최소 시스템(awk·grep·sed·printf 해소용)
    env["PATH"] = bindir + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
    env["CYS_PACK_DIR"] = pack
    env["CYS_ROOT"] = tmp
    # ★R3(codex · CI 3레인 등재 전제): **홈 무접촉**. 빈 임시 팩에는 soul 이 없어서 훅의 soul 해소가
    #   `$HOME/.claude/soul.md` → `$HOME/.cys/pack/soul.md` 로 폴백한다 — 러너/오너 홈의 내용이
    #   판정에 섞이면 초록이 근거가 되지 못한다. `HOME` 을 임시로 고정하고 `CYS_SOUL` 상속도 끊는다.
    env["HOME"] = tmp
    # ★성찰 R4 N5 이후: 훅은 역할 해소를 정본 `_lib.sh:cys_resolve_role` 에 위임한다. 정본의
    #   **신원 전제** — 숫자 surface id 가 없으면 데몬에게 '나' 를 묻지 않는다(주소가 없다는 사실이
    #   '역할 없음' 판정으로 승격되면 정상 위임 경로가 죽는다) — 때문에 실좌석이 언제나 갖는 이
    #   변수를 픽스처도 갖춰야 한다(데몬이 좌석에 주입한다). 핀의 **단언은 하나도 바뀌지 않았다**.
    env["CYS_SURFACE_ID"] = "7"
    # 정본은 60s 디스크 캐시·30s 실패 백오프를 `$TMPDIR` 아래 uid 전용 디렉터리에 둔다 —
    # 케이스마다 격리하지 않으면 앞 케이스의 캐시가 뒤 케이스의 판정을 정한다.
    env["TMPDIR"] = tmp
    if role_env:
        env["CYS_ROLE"] = role_env
    payload = json.dumps({"source": "startup", "cwd": cwd})
    r = subprocess.run([BASH, HOOK], input=payload, capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r


# ─────────────────── ⑨~⑮ U13 착수 게이트 하네스(0.14.41 · WP-C1) ───────────────────
# 종전 복원 신호(lead 좌석의 바이트 동일 기준 — 문자열 그대로 핀한다).
LEGACY_CLEAR = "▶ 작업 계속(source=clear): 위 작업기억 이어서 진행.\n"
LEGACY_STARTUP = ("▶ 복원 모드(source=startup): RECOVERY.md 절차 실행 → G2 실측 대조(git·pane·server) → "
                  "배달 원장 다이제스트(BOOT_SNAPSHOT.md 있으면 그것 · 귀속 판별은 MASTER_DIRECTIVE '귀속 판별' "
                  "절(절이 없으면 constitution 병합 대기 — cys pack-merge 승인 필요)) → 미해결 게이트부터 재개.\n")
LEGACY_COMPACT = "▶ 압축 직후(source=compact): 작업기억 보충 완료. 진행 중 작업 계속.\n"
SELF_START = ("이어서 진행", "미해결 게이트부터 재개")
GATE_MARK = "착수 게이트"
LIB = os.path.join(os.path.dirname(HOOK), "_lib.sh")
REPO = os.path.normpath(os.path.join(SELF, "..", "..", ".."))
BASE_REF = "126cfdd0"          # v0.14.40 — U13 이전 트리(lead 바이트 동일의 기준)
# ★리뷰1 I-5: 12c 는 BASE_REF 와 **영구** 대조한다(만료 조건 없음). ci-branch job1 은 얕은
#   체크아웃이라 `git show BASE_REF:...` 가 실패해 12c 가 SKIP 되고(§402 — 정직한 출력이지만
#   그 레인에서는 집행되지 않는다), 로컬에서는 이후 lead(master·cso*) 출력을 **정당하게**
#   바꾸는 모든 변경(통합 때 형제 WP 포함)에서 FAIL 한다.
#   갱신 규칙: lead 출력을 의도적으로 바꾸면(즉 12a 의 LEGACY_CLEAR/LEGACY_STARTUP 문자열
#   핀도 같이 고치는 변경이면) BASE_REF 를 그 변경이 들어간 커밋으로 올린다 — 문자열 핀(12a)
#   과 바이트 대조(12c)가 같은 순간에 갱신돼야 한다. 통합 단계에서 형제 WP(C3·E 등)와
#   교차 실행해 이 트리에서 SKIP 이 아니라 실제로 도는지 확인할 것(로컬 전용 집행 — 남은 문제).
_STRIP = ("CYS_ROLE", "CYS_SURFACE_ROLE", "CYS_SOCKET", "CYS_BIN", "CYS_GATE_LANE_SOCKET", "CYS_SOUL",
          "CYS_ROLE_UID", "CYS_SURFACE_ID", "JAVIS_SURFACE_ID", "AITERM_SURFACE_ID", "JAVIS_SOCKET",
          "AITERM_SOCKET", "CYS_ROOT", "CYS_STATE_DIR", "CYS_MISSION")


def gate_note():
    """단일 원본 문안 — 프리루드를 source 해서 함수 출력을 그대로 받는다(사본 금지)."""
    home = tempfile.mkdtemp(prefix="ic-gate-note-")
    try:
        env = {k: v for k, v in os.environ.items() if k not in _STRIP}
        env["HOME"] = home
        env["CYS_PACK_DIR"] = home
        r = subprocess.run([BASH, "-c", '. "$1" >/dev/null 2>&1; '
                            'command -v cys_start_gate_note >/dev/null 2>&1 && cys_start_gate_note', "_", LIB],
                           capture_output=True, text=True, encoding="utf-8", env=env, timeout=30)
        return r.stdout
    finally:
        shutil.rmtree(home, ignore_errors=True)


def gate_fixture(root, dept=False, state=True, seats=1, daemon="unjudged", checklist=True):
    """U13 좌석 픽스처. daemon: None=cys 부재 · 'unjudged'=rc3(판정 불가) · ''=rc0 빈 줄(권위 무역할) ·
    그 밖=rc0 + 그 역할. 체크리스트 스텁은 실행되면 checklist.log 에 1줄을 남긴다(미실행 = 파일 부재)."""
    fx = {"root": root, "bin": os.path.join(root, "stubbin"), "home": os.path.join(root, "home"),
          "cwd": os.path.join(root, "work"), "calls": os.path.join(root, "calls.log"),
          "chk": os.path.join(root, "checklist.log"),
          "pack": os.path.join(root, "pack-dept-t1" if dept else "pack")}
    for d in (fx["bin"], fx["home"], fx["cwd"], fx["pack"]):
        os.makedirs(d, exist_ok=True)
    rows = "".join("echo '%d claude /Users/x/.local/bin/claude --x'\n" % (9100 + i)
                   for i in range(max(seats, 1)))
    _write_exec(os.path.join(fx["bin"], "ps"), "#!/bin/sh\n" + rows)
    _write_exec(os.path.join(fx["bin"], "lsof"),
                "#!/bin/sh\n" + "".join("printf 'n%s\\n'\n" % fx["cwd"] for _ in range(seats)))
    if daemon is not None:
        body = {"unjudged": "exit 3", "": "exit 0"}.get(daemon, 'printf "%s\\n"; exit 0' % daemon)
        _write_exec(os.path.join(fx["bin"], "cys"),
                    "#!/bin/sh\n"
                    'printf "%%s\\n" "$1" >> "%s"\n'
                    'case "$1" in surface-role) %s ;; esac\nexit 0\n' % (fx["calls"], body))
    if dept:
        os.makedirs(os.path.join(fx["pack"], "round"), exist_ok=True)
        with open(os.path.join(fx["pack"], "round", "SESSION_STATE.md"), "w", encoding="utf-8") as f:
            f.write("# S\nDEPT-SS-MARKER\n다음 액션: r09 착수\n")
    elif state:
        os.makedirs(os.path.join(fx["cwd"], "_round"), exist_ok=True)
        with open(os.path.join(fx["cwd"], "_round", "SESSION_STATE.md"), "w", encoding="utf-8") as f:
            f.write("# S\nHUB-SS-MARKER\n")
    if checklist:
        os.makedirs(os.path.join(fx["pack"], "bin"), exist_ok=True)
        with open(os.path.join(fx["pack"], "bin", "javis_checklist.py"), "w", encoding="utf-8") as f:
            f.write("import sys\nopen(%r, 'a').write('ran\\n')\nprint('CHECKLIST-RAN')\n" % fx["chk"])
    return fx


def gate_run(fx, role_env=None, source="clear", surface=True, hook=None):
    """훅 1회 — 실행마다 새 TMPDIR(정본 역할 캐시 격리 · 앞 실행의 캐시가 뒤 판정을 정하지 않게)."""
    env = {k: v for k, v in os.environ.items() if k not in _STRIP}
    env["PATH"] = fx["bin"] + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
    env["CYS_PACK_DIR"] = fx["pack"]
    env["CYS_ROOT"] = fx["root"]
    env["HOME"] = fx["home"]
    env["TMPDIR"] = tempfile.mkdtemp(prefix="t", dir=fx["root"])
    if surface:
        env["CYS_SURFACE_ID"] = "7"
    if role_env:
        env["CYS_ROLE"] = role_env
    payload = json.dumps({"source": source, "cwd": fx["cwd"]})
    return subprocess.run([BASH, hook or HOOK], input=payload, capture_output=True, text=True,
                          encoding="utf-8", env=env, timeout=90)


def gate_queries(fx):
    if not os.path.isfile(fx["calls"]):
        return 0
    return sum(1 for ln in open(fx["calls"], encoding="utf-8") if ln.strip() == "surface-role")


def old_hooks(root):
    """기준 커밋(U13 이전)의 훅 3파일을 임시 디렉터리에 복원 — git 부재·shallow 이면 None(정직 SKIP)."""
    d = os.path.join(root, "oldhooks")
    os.makedirs(d, exist_ok=True)
    for name in ("inject-context.sh", "_lib.sh", "inject_gate.py"):
        try:
            r = subprocess.run(["git", "-C", REPO, "show", "%s:cysjavis-pack/hooks/%s" % (BASE_REF, name)],
                               capture_output=True, timeout=30)
        except Exception:      # noqa: BLE001 — git 부재는 정상 갈래(SKIP)
            return None
        if r.returncode != 0 or not r.stdout:
            return None
        with open(os.path.join(d, name), "wb") as f:
            f.write(r.stdout)
        os.chmod(os.path.join(d, name), 0o755)
    return os.path.join(d, "inject-context.sh")


tmp = tempfile.mkdtemp(prefix="ic-roleseat-")
try:
    # ① 역할 좌석 — 정보 1줄, 경고 0
    r = run_hook(os.path.join(tmp, "c1"), seats=3, cys_mode="role")
    check("1a exit 0", r.returncode == 0, r.stderr[-200:])
    check("1b 정보 1줄(역할 좌석 포함)", INFO_MARK in r.stdout, r.stdout[-300:])
    check("1c 세션 수 표기", "3개(역할 좌석 포함)" in r.stdout, r.stdout[-300:])
    check("1d ★역할 좌석 경고 0(수용 기준)", WARN_MARK not in r.stdout)
    check("1e 미검증 문구('레인 격리') 0", "레인 격리" not in r.stdout)

    # ② 확정 무역할(rc0+빈 줄)은 env 가 있어도 경고 — env 잔재가 경고를 끄지 않는다
    r = run_hook(os.path.join(tmp, "c2"), seats=2, cys_mode="none", role_env="cso")
    check("2a 확정 무역할은 경고", WARN_MARK in r.stdout, r.stdout[-300:])
    check("2b 확정 무역할에 정보줄 없음", INFO_MARK not in r.stdout)

    # ③ 판정 불가(rc≠0) → env 폴백
    r = run_hook(os.path.join(tmp, "c3"), seats=2, cys_mode="unjudged", role_env="worker-2")
    check("3a 판정 불가 + env → 정보줄", INFO_MARK in r.stdout, r.stdout[-300:])
    check("3b 판정 불가 + env → 경고 0", WARN_MARK not in r.stdout)
    r = run_hook(os.path.join(tmp, "c3b"), seats=2, cys_mode="unjudged")
    check("3c 판정 불가 + env 없음 → 종전 경고", WARN_MARK in r.stdout, r.stdout[-300:])

    # ④ CR 오염이 무역할을 역할 좌석으로 둔갑시키지 않는다
    r = run_hook(os.path.join(tmp, "c4"), seats=2, cys_mode="cr")
    check("4 CR 응답은 무역할(경고 유지)", WARN_MARK in r.stdout and INFO_MARK not in r.stdout,
          repr(r.stdout[-300:]))

    # ⑤ cys 부재
    r = run_hook(os.path.join(tmp, "c5"), seats=2, cys_mode="role", role_env="cso",
                 cys_present=False)
    check("5a cys 부재 + env → 정보줄", INFO_MARK in r.stdout, r.stdout[-300:])
    r = run_hook(os.path.join(tmp, "c5b"), seats=2, cys_mode="role", cys_present=False)
    check("5b cys 부재 + env 없음 → 경고", WARN_MARK in r.stdout)

    # ⑥ 부트 폭주 봉인 — 조회는 CYS_NO_AUTOSTART=1 로만 나간다
    d6 = os.path.join(tmp, "c6")
    run_hook(d6, seats=2, cys_mode="role")
    log = os.path.join(d6, "autostart.log")
    seen = open(log, encoding="utf-8").read().split() if os.path.exists(log) else []
    check("6a 조회가 실제로 일어났다(계측 타당성)", seen != [], repr(seen))
    check("6b ★모든 조회에 CYS_NO_AUTOSTART=1", seen and all(s == "1" for s in seen), repr(seen))

    # ⑦ 음성 대조 — 세션 1개면 두 줄 다 없다. ★0.14.41 U13 개정: 조회 0 은 **lead env 좌석**의 계약이다.
    #    종전 7b("단일 세션이면 조회 자체가 없다")는 역할과 무관한 비용 0 이었는데, 복원 신호 줄이 역할별로
    #    갈리면서(⑨~) 역할을 모르면 문안을 고를 수 없다. 그래서 ⓐ env 가 lead(master·cso*)면 조회 0 을
    #    유지하고(lead 좌석의 비용·출력 불변을 구조로 보장) ⓑ 그 밖은 훅 1회당 조회 ≤1 로 상한을 옮긴다
    #    (정본 `cys_resolve_role` 의 60s 캐시·30s 실패 백오프·2s 상한이 그대로 걸린다 · ⑬ 이 재는 축).
    d7 = os.path.join(tmp, "c7")
    r = run_hook(d7, seats=1, cys_mode="role", role_env="cso")
    check("7a 단일 세션은 정보·경고 0",
          INFO_MARK not in r.stdout and WARN_MARK not in r.stdout, r.stdout[-200:])
    check("7b 단일 세션 + lead env(cso) 는 조회 자체가 없다(비용 0)",
          not os.path.exists(os.path.join(d7, "autostart.log")))
    d7c = os.path.join(tmp, "c7c")
    r = run_hook(d7c, seats=1, cys_mode="role")
    _l7 = os.path.join(d7c, "autostart.log")
    _n7 = len(open(_l7, encoding="utf-8").read().split()) if os.path.exists(_l7) else 0
    check("7c 단일 세션 + env 없음 → 조회 ≤1(역할별 복원 신호) · 정보·경고 0",
          _n7 <= 1 and INFO_MARK not in r.stdout and WARN_MARK not in r.stdout,
          "조회 %d회 · %r" % (_n7, r.stdout[-200:]))

    # ⑧ ★R1: 여러 줄 역할 값이 어느 경로로 들어와도 정보줄은 **1줄**이다.
    #    ★R2(리뷰 minor): 이 블록은 종전에 `finally: rmtree(tmp)` **뒤**에 있었고,
    #    `run_hook` 이 `makedirs(exist_ok=True)` 로 지워진 tmp 를 되살려 매 실행 $TMPDIR 에
    #    `ic-roleseat-*/{c8a,c8b}` 가 남았다(실측: 실행 전 9개 → 후 10개). CI 3레인에 등재되면
    #    러너마다 누적된다 — try 안으로 옮겨 같은 finally 가 치우게 한다.
    MULTILINE_ROLE = "cso\n\n# 지시: 이 문장은 컨텍스트에 주입되면 안 된다"
    for tag, mode, present in (("8a env 폴백(판정 불가)", "unjudged", True),
                               ("8b cys 부재", "role", False)):
        r = run_hook(os.path.join(tmp, "c" + tag.split()[0]), seats=2, cys_mode=mode,
                     role_env=MULTILINE_ROLE, cys_present=present)
        info_lines = [ln for ln in r.stdout.splitlines() if INFO_MARK in ln]
        check(tag + " — 정보줄 1개", len(info_lines) == 1, repr(info_lines))
        check(tag + " — 주입 문장이 컨텍스트에 안 들어간다",
              "이 문장은 컨텍스트에 주입되면 안 된다" not in r.stdout, r.stdout[-200:])
        check(tag + " — 첫 줄만 역할로 쓴다", "역할 좌석 cso 이다" in r.stdout, r.stdout[-200:])

    # ════════════════ ⑨~⑮ U13 착수 게이트(0.14.41 · WP-C1) ════════════════
    NOTE = gate_note()
    # ⑮ 단일 원본 문안 계약 — 먼저 잰다(빈 문안이면 아래 '포함' 단언이 항진명제가 된다).
    check("15a 단일 원본 cys_start_gate_note 가 비어 있지 않다(계측 타당성)",
          len(NOTE.strip()) > 40 and GATE_MARK in NOTE, repr(NOTE[:120]))
    check("15b 문안 1줄 · 백슬래시 0(`printf '%b'` 로 소비된다)",
          "\n" not in NOTE and "\\" not in NOTE, repr(NOTE[:120]))
    check("15c session-start 소비처 금지 낱말 0(set-status·--ack·CYS_BOOT_NONCE·부트 브리지)",
          all(w not in NOTE for w in ("set-status", "--ack", "CYS_BOOT_NONCE", "부트 브리지")), NOTE)
    check("15d 문안이 착수 규칙 핵심을 싣는다([RESUME]·[RESTORE]·운영 절차 예외 [CYCLE]·[CYCLE-VERIFY]·[DRAIN])",
          all(t in NOTE for t in ("[RESUME]", "[RESTORE]", "[CYCLE]", "[CYCLE-VERIFY]", "[DRAIN]", "대기")),
          NOTE)
    # ★리뷰1 I-2: 예외 목록은 이 9개 낱말 **전수**다 — 15d 가 그중 5개만 쟀다(MU9b 가 나머지
    #   4개+DRAIN-VERIFY 를 지워도 전부 통과했다). 핑·ACK 가 빠지면 member 가 `reinject --check`·
    #   부트 확인에 답하지 않아 ③자가치유·온보딩 회귀가 된다 — 전수로 넓힌다.
    check("15d2 ★회귀 핀(리뷰1 I-2) — 예외 목록 나머지도 전수([CYCLE-PRE]·[DRAIN-VERIFY]·"
          "지침 각성 확인 핑·각성 ACK·승인)",
          all(t in NOTE for t in ("[CYCLE-PRE]", "[DRAIN-VERIFY]", "지침 각성 확인 핑",
                                   "각성 ACK", "승인")),
          NOTE)
    # ★리뷰1 I-3: 설계 §3 U13 은 데몬 라벨 메시지([schedule …]/[wakeup]/[heartbeat])를 '지시'로
    #   명시했다(WORKER §0·REVIEWER §1-2 에는 있다 — test_bootv2_doc_contract 는 그 두 문서만 잰다).
    #   기존 설치는 지침 `.new` 미병합이면 이 훅 문안**만**을 규칙으로 받으므로, fresh 스케줄 좌석
    #   (member 로 분류)이 그 정의를 놓치지 않게 여기도 싣고 핀으로 고정한다.
    check("15d3 ★회귀 핀(리뷰1 I-3) — 데몬 라벨 메시지가 지시 열거에 있다"
          "([schedule …]·[wakeup]·[heartbeat])",
          all(t in NOTE for t in ("[schedule", "[wakeup]", "[heartbeat]")), NOTE)
    check("15e 문안 자체가 자율 착수 낱말을 싣지 않는다", all(w not in NOTE for w in SELF_START), NOTE)
    check("15f ★부트 체인 안전 — 운영 절차 예외에 '각성 메시지'가 있다"
          "(javis_boot_node.awaken_message '즉시 각성하라'가 착수 게이트에 막혀 신규 좌석"
          " claim-role·set-status·TODO 확인·각성 보고가 멎는 온보딩 회귀를 막는다)",
          "각성 메시지" in NOTE, NOTE)

    # ⑨ member — 본부 레인(작업기억 있음) · clear / startup · 비표준 역할
    for tag, role, daemon, src, lead_line in (
            ("9a worker clear", "worker", "worker", "clear", "▶ 컨텍스트 순환 직후(source=clear): "),
            ("9b reviewer-codex startup(데몬 판정 불가 → env)", "reviewer-codex", "unjudged", "startup",
             "▶ 복원 모드(source=startup): "),
            ("9c worker-2 resume", "worker-2", "worker-2", "resume", "▶ 복원 모드(source=resume): "),
            ("9d 비표준 역할 planner clear", "planner", "planner", "clear",
             "▶ 컨텍스트 순환 직후(source=clear): ")):
        fx = gate_fixture(os.path.join(tmp, "g9-" + tag.split()[0]), daemon=daemon)
        r = gate_run(fx, role_env=role, source=src)
        out = r.stdout
        check(tag + " — exit 0", r.returncode == 0, r.stderr[-200:])
        check(tag + " — 자율 착수 문안 0", all(w not in out for w in SELF_START), out[-400:])
        check(tag + " — 착수 게이트 1줄(단일 원본 그대로)",
              bool(NOTE) and (lead_line + NOTE + "\n") in out, out[-500:])
        check(tag + " — 본부 작업기억 본문은 그대로(설계: 본문 대체는 부서 레인만)",
              "HUB-SS-MARKER" in out and "배경 컨텍스트" in out, out[:300])

    # ⑩ 부서 레인 member — 부서장 SESSION_STATE 본문 0 · master 소관 1줄 + 경로 · 체크리스트 미실행
    fx = gate_fixture(os.path.join(tmp, "g10"), dept=True, daemon="worker")
    r = gate_run(fx, role_env="worker", source="clear")
    out = r.stdout
    ss = os.path.join(fx["pack"], "round", "SESSION_STATE.md")
    check("10a 부서 member — 부서장 작업기억 본문·다음 액션 0",
          "DEPT-SS-MARKER" not in out and "r09 착수" not in out, out[:400])
    check("10b 부서 member — 'master 소관' 1줄 + 경로",
          "master 소관" in out and ss in out
          and len([ln for ln in out.splitlines() if "master 소관" in ln]) == 1, out[:400])
    check("10c 부서 member — 실측 체크리스트(부서장 작업기억 대조) 미실행",
          not os.path.exists(fx["chk"]) and "CHECKLIST-RAN" not in out, out[-300:])
    check("10d 부서 member — '작업기억 미발견'·'부서 SESSION_STATE 부재' 오고지 0",
          "작업기억 미발견" not in out and "SESSION_STATE 부재" not in out, out[:400])
    check("10e 부서 member — 착수 게이트 1줄", bool(NOTE) and NOTE in out, out[-300:])
    # 음성 대조: 같은 부서 픽스처의 부서장(master)·CSO 는 본문·체크리스트를 그대로 받는다.
    for role in ("master", "cso"):
        fxl = gate_fixture(os.path.join(tmp, "g10-" + role), dept=True, daemon=role)
        rl = gate_run(fxl, role_env=role, source="clear")
        check("10f 부서 %s — 본문·체크리스트 그대로(음성 대조)" % role,
              "DEPT-SS-MARKER" in rl.stdout and os.path.exists(fxl["chk"])
              and "master 소관" not in rl.stdout, rl.stdout[:300])

    # ⑪ 역할 미상 — 중립 문안 · 본문 유지(복원 생명선)
    fx = gate_fixture(os.path.join(tmp, "g11a"), daemon=None)
    r = gate_run(fx, role_env=None, source="clear", surface=False)       # 신원 없음 + env 없음
    check("11a 미상(신원·env 없음) — 중립 문안 · 자율 착수 0",
          bool(NOTE) and NOTE in r.stdout and all(w not in r.stdout for w in SELF_START), r.stdout[-300:])
    check("11b 미상 — 본부 작업기억 본문 유지", "HUB-SS-MARKER" in r.stdout, r.stdout[:300])
    fx = gate_fixture(os.path.join(tmp, "g11c"), dept=True, daemon="")      # 권위 무역할
    r = gate_run(fx, role_env=None, source="startup")
    check("11c 부서 미상(권위 무역할) — 본문 유지(부서장일 수 있다) · 중립 문안",
          "DEPT-SS-MARKER" in r.stdout and bool(NOTE) and NOTE in r.stdout
          and all(w not in r.stdout for w in SELF_START), r.stdout[-300:])

    # ⑫ lead — 종전 출력 바이트 동일
    for role in ("master", "cso", "cso-1"):
        fx = gate_fixture(os.path.join(tmp, "g12-" + role), seats=2, daemon=role)
        rc_ = gate_run(fx, role_env=role, source="clear")
        rs_ = gate_run(fx, role_env=role, source="startup")
        check("12a %s — 종전 clear·startup 문안 그대로 · 착수 게이트 0" % role,
              LEGACY_CLEAR in rc_.stdout and LEGACY_STARTUP in rs_.stdout
              and GATE_MARK not in rc_.stdout + rs_.stdout, rc_.stdout[-300:])
    # 데몬이 lead 라고 답하면(env 가 낡은 worker 여도) 종전 문안 — 권위는 데몬이다.
    fx = gate_fixture(os.path.join(tmp, "g12b"), daemon="master")
    r = gate_run(fx, role_env="worker", source="clear")
    check("12b 데몬 lead(env 는 낡은 worker) → 종전 문안(승계 좌석)",
          LEGACY_CLEAR in r.stdout and GATE_MARK not in r.stdout, r.stdout[-300:])
    old = old_hooks(os.path.join(tmp, "g12-old"))
    if old is None:
        print("SKIP 12c lead 전문 바이트 대조 — 기준 커밋 %s 부재(shallow/비레포) · 문자열 핀(12a)만 유효" % BASE_REF)
    else:
        diffs = []
        for role in ("master", "cso-1"):
            for dept in (False, True):
                for src in ("startup", "clear", "compact", "resume"):
                    fx = gate_fixture(os.path.join(tmp, "g12c-%s-%d-%s" % (role, dept, src)),
                                      dept=dept, seats=2, daemon=role)
                    a = gate_run(fx, role_env=role, source=src, hook=old)
                    b = gate_run(fx, role_env=role, source=src)
                    if a.stdout != b.stdout or a.returncode != b.returncode:
                        diffs.append("%s/%s/%s" % (role, "dept" if dept else "hub", src))
        # 데몬 권위로만 lead 가 확정되는 좌석(env 없음)도 같다.
        fx = gate_fixture(os.path.join(tmp, "g12c-daemon"), seats=2, daemon="master")
        a = gate_run(fx, source="clear", hook=old)
        b = gate_run(fx, source="clear")
        if a.stdout != b.stdout:
            diffs.append("daemon-master/hub/clear")
        check("12c ★lead 전문 바이트 동일(기준 커밋 %s 훅 대조 · 17케이스)" % BASE_REF, not diffs,
              "갈린 케이스: %s" % diffs)

    # ⑬ 비용
    fx = gate_fixture(os.path.join(tmp, "g13a"), seats=1, daemon="master")
    gate_run(fx, role_env="master", source="clear")
    check("13a lead env — 조회 0", gate_queries(fx) == 0, "조회 %d" % gate_queries(fx))
    fx = gate_fixture(os.path.join(tmp, "g13b"), seats=2, daemon="worker")
    r = gate_run(fx, role_env="worker", source="clear")
    check("13b member + 다중 세션(정보줄도 역할 필요) — 조회 정확히 1(중복 0)",
          gate_queries(fx) == 1 and "역할 좌석 worker 이다" in r.stdout,
          "조회 %d · %r" % (gate_queries(fx), r.stdout[-200:]))
    fx = gate_fixture(os.path.join(tmp, "g13c"), seats=1, daemon="worker")
    gate_run(fx, role_env=None, source="clear", surface=False)
    check("13c 신원(surface id) 없음 — 조회 0(주소 없는 질의 금지)", gate_queries(fx) == 0,
          "조회 %d" % gate_queries(fx))

    # ⑭ compact — 역할 무관 종전 문안
    fx = gate_fixture(os.path.join(tmp, "g14"), daemon="worker")
    r = gate_run(fx, role_env="worker", source="compact")
    check("14 member compact — 종전 문안(진행 중 턴의 연속) · 착수 게이트 0",
          LEGACY_COMPACT in r.stdout and GATE_MARK not in r.stdout, r.stdout[-300:])
finally:
    shutil.rmtree(tmp, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("INJECT-CONTEXT-ROLE-SEAT-OK")
sys.exit(0)
