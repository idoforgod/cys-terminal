#!/usr/bin/env python3
"""test_parity_paths.py — C3 두 경로 동등성 e2e 계약 핀 (T0 RED-first · 설계 §5·DD-3).

버튼 경로(cys-dept allocate→formation ensure)와 수동 경로("너는 마스터다"→javis_bootstrap→
동일 ensure)가 **동일 로스터 + 동일 주입 디렉티브 해시 + C03 동일** 을 내는지 mock-agent 로 검증.

현 코드에는 `javis_formation.py`(두 경로가 수렴하는 ensure)가 없으므로 → RED. T2/T5 가 GREEN 화.
품질 앵커(R2-4): 노드 수가 아니라 **주입된 지침의 온전함(해시 대조)** 이 판정 기준이다.

계약(구현 후 통과 기준):
  1) 두 경로 모두 formation.ensure 로 수렴(동일 함수) — 구조적 동등성.
  2) 두 경로의 결과 로스터(역할 집합)가 동일.
  3) 두 경로가 각 역할 pane 에 주입한 디렉티브의 sha256 이 동일(mock-agent 가 기록).
  4) 두 경로 모두 C03(표준 핀 잔존) PASS.

실행: python3 test_parity_paths.py   (exit 0=PASS / 1=RED)
"""
import hashlib
import importlib.util
import os
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_formation.py"))
FIXTURE = os.path.join(SELF, "fixtures", "mock-agent.sh")
# ★밀폐(hermetic) 고정 — load() 전에 설정해야 모듈 import 시점 PACK_DIR 이 리포 팩으로 잡힌다.
# 비밀폐 결함 수리(master 재검증): CYS_PACK_DIR 미설정 환경에서 javis_formation._directives_dir()
# 가 라이브 팩(~/.cys/pack — 현재 반쪽마스터 스텁)으로 폴백해 C03 이 FAIL 했다(4/5). 테스트는
# 환경을 상속하지 않고 결정론적으로 리포 팩(테스트 파일 상대 경로)만 읽는다 = 라이브 무접촉.
os.environ["CYS_PACK_DIR"] = os.path.normpath(os.path.join(SELF, "..", ".."))  # cysjavis-pack/
fails = []
_total = [0]


def check(name, cond, detail=""):
    _total[0] += 1
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def load():
    if not os.path.exists(MODULE):
        return None
    spec = importlib.util.spec_from_file_location("javis_formation", MODULE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def _py_sha256(path):
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def _e2e_supported():
    """실 mock-agent 주입-해시 e2e 가능 여부: POSIX sh + 독립 해셔(shasum|sha256sum)."""
    if os.name == "nt":
        return False, "Windows(POSIX sh 부재) — 주입 e2e 는 posix 전용(스킵)"
    import shutil
    if not (os.path.exists("/bin/sh") or shutil.which("sh")):
        return False, "sh 부재(스킵)"
    if not (shutil.which("shasum") or shutil.which("sha256sum")):
        return False, "독립 해셔(shasum/sha256sum) 부재(스킵)"
    return True, ""


def _run_mock_agent(role, directive_path):
    """mock-agent.sh 를 실제 서브프로세스로 실행 → 그 프로세스가 **독립 해셔(shasum/sha256sum)**로
    주입 디렉티브를 해싱해 남긴 AWAKE 라인의 directive_sha 를 회수한다. 데몬 없는 밀폐 실행 —
    편성 노드가 각성 시 주입 디렉티브를 실제로 손에 쥐는지(=주입 파손 탐지)를 검증한다.
    반환: 캡처된 directive_sha 문자열(주입 부재/파손 시 'none')."""
    log = tempfile.NamedTemporaryFile(prefix="mockawake-", suffix=".log", delete=False)
    log.close()
    env = dict(os.environ)
    env["MOCK_AGENT_LOG"] = log.name
    env["MOCK_AGENT_ONESHOT"] = "1"
    env["CYS_ROLE"] = role
    env["CYS_SURFACE_ID"] = "surface:test-%s" % role
    if directive_path is not None:
        env["CYS_INJECT_DIRECTIVE"] = directive_path
    else:
        env.pop("CYS_INJECT_DIRECTIVE", None)
    try:
        subprocess.run(["/bin/sh", FIXTURE], env=env, timeout=20,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        line = ""
        with open(log.name, encoding="utf-8") as f:
            for ln in f:
                if ln.startswith("AWAKE "):
                    line = ln.strip()
        sha = "none"
        for tok in line.split():
            if tok.startswith("directive_sha="):
                sha = tok.split("=", 1)[1]
        return sha
    finally:
        try:
            os.remove(log.name)
        except OSError:
            pass


def inject_e2e(m):
    """T2 R2 인계 실물화: 노드 수가 아니라 **주입된 지침의 온전함**을 실 mock-agent 실행으로 검증.
    tautological(파일 재읽기 해시 비교) 아님:
      (1) 별도 프로세스(mock-agent)가 (2) 독립 해셔(shasum/sha256sum)로 실제 주입 디렉티브를 다시 해싱
      (3) 그 전달 해시를 button·manual **두 RosterPlan 이 각자 독립 산출한 claim(python hashlib)** 과
          대조(=두 경로 파리티를 '실 전달'로 확증) (4) 음성 대조군으로 주입 파손 탐지를 증명."""
    ok, why = _e2e_supported()
    if not ok:
        check("5 주입-해시 e2e (환경 스킵)", True, why)
        return

    role_file = getattr(m, "_ROLE_DIRECTIVE_FILE", None)
    ddir = m._directives_dir() if hasattr(m, "_directives_dir") else None
    if not role_file or not ddir:
        check("5 주입 조립 계약(_ROLE_DIRECTIVE_FILE·_directives_dir)", False,
              "주입 조립 심볼 부재")
        return

    # 두 경로의 계획(RosterPlan)을 각자 유도 — 각 plan.directive_hashes 는 그 경로가 **독립적으로**
    # 산출한 role→주입 디렉티브 해시(claim)다(REVISE1: 동일 path 2회 호출 tautology 제거).
    button_plan = m.plan_roster("button")
    manual_plan = m.plan_roster("manual")
    bh = getattr(button_plan, "directive_hashes", {}) or {}
    mh = getattr(manual_plan, "directive_hashes", {}) or {}

    delivered = {}          # role → mock-agent(독립 해셔)가 실제 전달받아 캡처한 해시
    all_present = True
    hasher_agrees = True
    two_path_parity = True
    for role, fn in role_file.items():
        path = os.path.join(ddir, fn)
        expect = _py_sha256(path)          # python hashlib(조립부 _sha256_file 과 동일 계약)
        cap = _run_mock_agent(role, path)  # 실 mock-agent 1회 실행 — 독립 해셔로 전달분 재해싱
        delivered[role] = cap
        if cap == "none":
            all_present = False
        # (a) 전달 무결: 독립 해셔(shell) == 조립부(python hashlib) 재해싱.
        if expect is None or cap != expect:
            hasher_agrees = False
        # (b) 두 경로 파리티(실 전달로): 전달 해시가 button·manual **양 plan 의 독립 claim** 과 일치.
        if cap == "none" or bh.get(role) != cap or mh.get(role) != cap:
            two_path_parity = False

    check("5 주입 e2e: 전 역할 디렉티브 실제 전달(none 0 — '편성됐지만 미주입' 탐지)",
          all_present, "delivered=%r" % delivered)
    check("6 주입 e2e: 독립 해셔(shasum)↔조립부(hashlib) 해시 일치(실 전달 무결)",
          hasher_agrees, "delivered=%r" % delivered)
    check("7 주입 e2e: 전달 해시가 button·manual 두 plan 의 독립 claim 과 일치(두 경로 파리티·실 전달)",
          two_path_parity, "delivered=%r button_claim=%r manual_claim=%r" % (delivered, bh, mh))

    # ★음성 대조군(non-tautology 증명): 주입 소스가 깨지면(존재하지 않는 경로) mock-agent 가
    #   directive_sha=none 을 내야 한다 = 이 e2e 가 '주입 파손'을 실제로 탐지함을 보증한다.
    broken = os.path.join(ddir, "____NO_SUCH_DIRECTIVE____.md")
    neg = _run_mock_agent("master", broken)
    check("8 주입 e2e 대조군: 파손된 주입은 none 으로 검출(테스트가 파손을 잡음을 증명)",
          neg == "none", "broken capture=%r (none 이어야 함)" % neg)


def main():
    check("0 mock-agent 픽스처 존재", os.path.exists(FIXTURE), FIXTURE)

    m = load()
    if m is None or not callable(getattr(m, "ensure", None)):
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "javis_formation.ensure 미구현 — 두 경로 수렴점 부재(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    # 두 경로 시뮬레이션 API 계약 — plan_roster(path) 가 경로별 계획 로스터를 반환한다고 가정.
    plan = getattr(m, "plan_roster", None)
    if not callable(plan):
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "plan_roster(path) 계약 미구현(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    try:
        button = plan(path="button")
        manual = plan(path="manual")
        check("1 두 경로 formation.ensure 수렴",
              getattr(button, "ensure_fn", None) == getattr(manual, "ensure_fn", None),
              "수렴 함수 불일치")
        check("2 로스터 동일",
              set(button.roles) == set(manual.roles), "%r vs %r" % (button.roles, manual.roles))
        check("3 주입 디렉티브 해시 동일",
              button.directive_hashes == manual.directive_hashes,
              "해시 불일치(노드 수 아닌 지침 온전함)")
        check("4 두 경로 C03 PASS",
              button.c03_pass and manual.c03_pass, "C03 불통")
    except Exception as e:
        for n in ("1 두 경로 formation.ensure 수렴",
                  "2 로스터 동일", "3 주입 디렉티브 해시 동일", "4 두 경로 C03 PASS"):
            check(n, False, "동등성 API 호출 실패: %s" % e)

    # ── D1(T5): 실 mock-agent 주입-해시 e2e (T2 R2 인계 실물화 — 밀폐·데몬 없음) ──
    inject_e2e(m)

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
