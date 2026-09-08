#!/usr/bin/env python3
"""test_session_start_hook.py — session-start.sh 3상태 재대조·안내문 계약 핀 (WP-1·핀ⓒ).

가짜 JARVIS_DIR(디렉티브 파일)+PATH 스텁 cys로 hook을 sh 실행:
  ⓐ claim 성공 → 디렉티브 주입(현행)
  ⓑ 명시적 거부(claim_denied) → 디렉티브 대신 self-demote 지시·exit 0
  ⓒ 데몬-불가(스텁이 비0+무패턴/응답없음) → fail-open: 디렉티브 주입+고지 1줄
+ 안내문(role-less)이 javis_bootstrap.py 단일 진입점·exit 7 인계·인용 의무를 담는지
+ worker role은 재대조 미적용(무왕복) 핀.

★(0.14.31 · WP-4) 역할 **자동 복구** 확장:
  ⓓ 무역할 + `cys reclaim-role --auto` 가 역할을 돌려주면 → 그 역할 지침 주입 + 고지
  ⓔ 무역할 + `role=`(무결합) → 종전 안내문(무회귀). 두 왕복(surface-role·reclaim-role)은 실제로 났다
  ⓕ `cys surface-role` exit 2(판정 불가) → **reclaim 왕복 0**(모르는 상태에서 역할을 옮기지 않는다)
  ⓖ 역할명 형식 가드 — 데몬 유래 값이라도 [a-zA-Z0-9_-] 밖이면 채택하지 않는다
"""
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.join(SELF, "..", "..", "hooks", "session-start.sh")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def setup(tmp, claim_mode, reclaim_mode="none"):
    """claim_mode: ok | denied | dead(비0 무패턴) | silent(무한대기→timeout)

    reclaim_mode(★0.14.31 WP-4): 스텁 `cys` 의 surface-role·reclaim-role 응답 조합.
      reclaim-role 의 stdout 계약은 **4줄**이다: `role=` · `reason=` · `env_role=` · `detail=`
      (넷째는 진단 축 — 사유 어휘를 늘리지 않고 처방만 가른다 · 빈 값·부재 모두 정상).
      none        surface-role exit 0 · `role=` / no_candidate / env_role=unknown  (무결합)
      found       surface-role exit 0 · `role=cso` / bound / env_role=unknown      (자동 복구)
      undecidable surface-role **exit 2** · (호출되면 안 된다)
      bogus       surface-role exit 0 · 형식 위반 역할명
      taken       surface-role exit 0 · `role=` / no_candidate / **env_role=other_live**
                  (★강등 — 그 역할을 지금 다른 산 좌석이 쥐었다)
      mine        surface-role exit 0 · `role=worker` / already_roled / **env_role=self**
                  (내가 그 역할이다 — 강등 없음)
    """
    pack = os.path.join(tmp, "pack")
    bindir = os.path.join(tmp, "stubbin")
    os.makedirs(os.path.join(pack, "directives"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    for d in ("MASTER", "WORKER", "CSO", "REVIEWER"):
        with open(os.path.join(pack, "directives", "%s_DIRECTIVE.md" % d), "w",
                  encoding="utf-8") as f:
            f.write("DIRECTIVE-BODY-%s\n" % d)
    body = {"ok": "exit 0",
            "denied": "echo 'claim_denied: privileged role held by live surface' >&2; exit 1",
            "dead": "echo 'connect error' >&2; exit 1",
            "silent": "sleep 10"}[claim_mode]
    def _rc(role, reason, env_role, detail=""):
        # ★(수렴 R2) 계약은 4줄이다 — 넷째 `detail=` 은 사유 코드가 아니라 진단 축이고 빈 값이
        #   정상이다(구 바이너리는 이 줄이 없다 → 훅은 종전대로 동작해야 한다).
        return ("printf 'role=%s\\nreason=%s\\nenv_role=%s\\ndetail=%s\\n' "
                "'{r}' '{s}' '{e}' '{d}'; exit 0"
                .format(r=role, s=reason, e=env_role, d=detail))
    sr_body, rc_body = {
        "none":        ("exit 0", _rc("", "no_candidate", "unknown")),
        "found":       ("exit 0", _rc("cso", "bound", "unknown")),
        "undecidable": ("exit 2", _rc("cso", "bound", "unknown")),
        "bogus":       ("exit 0", _rc("cso; rm -rf /", "bound", "unknown")),
        "taken":       ("exit 0", _rc("", "no_candidate", "other_live")),
        "mine":        ("exit 0", _rc("worker", "already_roled", "self")),
        # ★(R2) 데몬이 **결합을 커밋**했는데 env 는 다른 값 — 훅은 데몬 답을 채택해야 한다.
        "bound_other":  ("exit 0", _rc("cso", "bound", "vacant")),
        # ★(R2) worker 중복제거 실전형: 이 좌석은 정당하게 worker-2 를 쥐었고 env 는 stale
        #   `worker` 이며, 그 `worker` 는 **다른 산 좌석**이 쥐었다(other_live).
        #   종전 규칙은 이 좌석을 **강등**해 지침 0 으로 만들었다(두 리뷰어 공통 blocking).
        "dedup_live":   ("exit 0", _rc("worker-2", "already_roled", "other_live")),
        # ★(R2) 특권 빈 좌석이 있으나 자동 경로는 열지 않는다 — 처방을 고지해야 한다.
        "priv_optin":   ("exit 0", _rc("", "privileged_needs_optin", "unknown")),
        # ★(R2) 데몬이 이 좌석의 축을 모른다 — 무결합 + 처방 고지.
        "axes_unknown": ("exit 0", _rc("", "caller_axes_unknown", "unknown")),
        # ★(독립 재유도) 신고 $PWD 와 좌석의 실제 cwd 가 다른 폴더다 — 축 미확정과 **처방이
        #   다른** 무결합이므로 사유를 나눠 말한다(신고로 다른 폴더의 역할을 가져오지 않는다).
        "cwd_conflict": ("exit 0", _rc("", "no_candidate", "unknown", "reported_cwd_conflict")),
        # ★(R2) 데몬이 **판정해서** 무역할이라고 답했고 env 역할은 주인이 없다(미등록).
        "unregistered":  ("exit 0", _rc("", "no_candidate", "vacant")),
        # ★(수렴 R2) 구 바이너리 — 넷째 줄(`detail=`)이 **없는** 3줄 응답. 훅은 종전대로 돈다.
        "legacy3":      ("exit 0",
                         "printf 'role=\\nreason=no_candidate\\nenv_role=unknown\\n'; exit 0"),
        # ★독립 재유도(triage · codex major #7): **손상된 응답**을 '정상적인 빈 역할 답변'으로
        #   읽으면 안 된다. ⓐ 형식 가드에 걸린 역할명(첫 줄이 `role=` 이지만 채택 불가) ·
        #   ⓑ 첫 줄이 계약 형식이 아니고 명령 자체가 실패(exit 1). 둘 다 CYS_RECLAIMED 가
        #   빈 문자열이 되어 `env_role=other_live` 와 만나면 **강등**으로 떨어진다.
        "bogus_live":   ("exit 0", _rc("cso; rm -rf /", "bound", "other_live")),
        "garbled_live": ("exit 0",
                         "printf 'cys: connection reset by peer\\nreason=bound\\n"
                         "env_role=other_live\\n'; exit 1"),
    }[reclaim_mode]
    with open(os.path.join(bindir, "cys"), "w", encoding="utf-8", newline="\n") as f:
        f.write("#!/bin/sh\necho \"cys $@\" >> \"%s/calls.log\"\n"
                "case \"$1\" in\n"
                "  claim-role) %s;;\n"
                "  surface-role) %s;;\n"
                "  reclaim-role) %s;;\n"
                "esac\nexit 0\n" % (tmp, body, sr_body, rc_body))
    os.chmod(os.path.join(bindir, "cys"), 0o755)
    env = dict(os.environ)
    env.update({"CYS_PACK_DIR": pack, "CYS_SURFACE_ID": "3",
                "PATH": bindir + os.pathsep + env.get("PATH", "")})
    env.pop("CYS_ROLE", None)
    return env


def run_hook(env, role=None):
    e = dict(env)
    if role:
        e["CYS_ROLE"] = role
    r = subprocess.run(["sh", HOOK], capture_output=True, text=True, encoding="utf-8",
                       env=e, stdin=subprocess.DEVNULL, timeout=30)
    return r.returncode, r.stdout, r.stderr


# ── 1. role-less 안내문 계약 ──
tmp = tempfile.mkdtemp(prefix="hook-t1-")
env = setup(tmp, "ok")
code, out, _ = run_hook(env)
check("1a 안내문 exit 0", code == 0)
check("1b 단일 진입점 스크립트", "javis_bootstrap.py" in out)
check("1c exit 7 인계 분기", "exit 7" in out and "인계" in out)
check("1d 인용 의무 명문", "인용" in out)
check("1e 산문 부트 지시 제거", "preflight.py --fix" not in out.replace("javis_preflight", ""))
shutil.rmtree(tmp)

# ── 2. ⓐ master claim 성공 → 디렉티브 주입 ──
tmp = tempfile.mkdtemp(prefix="hook-t2-")
env = setup(tmp, "ok")
code, out, _ = run_hook(env, role="master")
check("2a ⓐ성공: 디렉티브 주입", "DIRECTIVE-BODY-MASTER" in out)
check("2b ⓐ성공: self-demote 없음", "역할 주소 상실" not in out)
# ★R13 부트 브리지: 구 산문 §0만 아는 디렉티브 기계에도(hook=system층 전파) 스크립트 경로 고지
check("2c ★R13 부트 브리지 주입(javis_bootstrap 부재 시 생략)",
      "부트 브리지" not in out)  # 가짜 팩엔 bin/javis_bootstrap.py 없음 → 브리지 미주입(조건부 확인)

shutil.rmtree(tmp)

# ── 2x. ★R13 브리지 존재 케이스: 팩에 javis_bootstrap.py 있으면 master 주입에 브리지 동봉 ──
tmp = tempfile.mkdtemp(prefix="hook-t2x-")
env = setup(tmp, "ok")
_bs = os.path.join(env["CYS_PACK_DIR"], "bin")
os.makedirs(_bs, exist_ok=True)
open(os.path.join(_bs, "javis_bootstrap.py"), "w", encoding="utf-8").write("# stub\n")
code, out, _ = run_hook(env, role="master")
check("2x1 ★R13 브리지 주입", "부트 브리지" in out and "javis_bootstrap.py" in out)
check("2x2 브리지는 worker엔 미주입", "부트 브리지" not in run_hook(env, role="worker")[1])
shutil.rmtree(tmp)

# ── 3. ⓑ 명시적 거부 → self-demote·디렉티브 미주입 ──
tmp = tempfile.mkdtemp(prefix="hook-t3-")
env = setup(tmp, "denied")
code, out, _ = run_hook(env, role="master")
check("3a ⓑ거부: self-demote 지시", "역할 주소 상실" in out and "인계" in out)
check("3b ⓑ거부: 디렉티브 미주입", "DIRECTIVE-BODY-MASTER" not in out)
check("3c ⓑ거부: exit 0(hook 무해)", code == 0)
shutil.rmtree(tmp)

# ── 4. ⓒ 데몬-불가(비0·무패턴) → fail-open: 디렉티브 주입+고지 ──
tmp = tempfile.mkdtemp(prefix="hook-t4-")
env = setup(tmp, "dead")
code, out, _ = run_hook(env, role="master")
check("4a ⓒ불가: 디렉티브 주입(fail-open)", "DIRECTIVE-BODY-MASTER" in out)
check("4b ⓒ불가: 고지 1줄", "역할 재확인 불가" in out)
check("4c ⓒ불가: self-demote 없음", "역할 주소 상실" not in out)
shutil.rmtree(tmp)

# ── 5. ⓒ 무응답(timeout 상한) → fail-open ──
tmp = tempfile.mkdtemp(prefix="hook-t5-")
env = setup(tmp, "silent")
code, out, _ = run_hook(env, role="master")
check("5a ⓒ무응답: 디렉티브 주입(fail-open)", "DIRECTIVE-BODY-MASTER" in out)
shutil.rmtree(tmp)

# ── 6. worker는 재대조 미적용(권한 role 아님 — claim 왕복 0) ──
tmp = tempfile.mkdtemp(prefix="hook-t6-")
env = setup(tmp, "denied")
code, out, _ = run_hook(env, role="worker")
check("6a worker 디렉티브 주입", "DIRECTIVE-BODY-WORKER" in out)
calls = ""
if os.path.exists(os.path.join(tmp, "calls.log")):
    calls = open(os.path.join(tmp, "calls.log"), encoding="utf-8").read()
# ★(R1) 문자열 정밀화: `reclaim-role` 이 부분문자열로 `claim-role` 을 포함한다 — 종전 검사는
#   그 두 개를 구별하지 못했다. 이 핀의 뜻은 "worker 는 **특권 재대조**(`cys claim-role <role>`)를
#   하지 않는다"이고, 그 뜻 그대로 **명령 토큰 단위**로 잰다(느슨해진 것이 아니라 정확해졌다).
check("6b worker claim 왕복 0(특권 재대조 없음)",
      not any(l.startswith("cys claim-role") for l in calls.splitlines()))
shutil.rmtree(tmp)


# ══════════════════════════════════════════════════════════════════════════════
# ★(0.14.31 · WP-4) 무역할 좌석의 역할 자동 복구
# ══════════════════════════════════════════════════════════════════════════════

def calls_of(tmp):
    p = os.path.join(tmp, "calls.log")
    return open(p, encoding="utf-8").read() if os.path.exists(p) else ""


# ── 7. ⓓ 자동 복구 성공 → 그 역할 지침 주입 + 고지(안내문 대신) ──
tmp = tempfile.mkdtemp(prefix="hook-t7-")
env = setup(tmp, "ok", reclaim_mode="found")
code, out, _ = run_hook(env)
check("7a ⓓ복구: 역할 지침 주입", "DIRECTIVE-BODY-CSO" in out)
check("7b ⓓ복구: 고지 1줄", "역할 자동 복구" in out and "cso" in out)
check("7c ⓓ복구: 무역할 안내문 대신 지침", "javis_bootstrap.py" not in out)
check("7d ⓓ복구: exit 0", code == 0)
_c = calls_of(tmp)
check("7e ⓓ복구: 인자 계약(--auto --config --cwd)",
      "reclaim-role --auto --config" in _c and "--cwd" in _c, _c.strip().replace("\n", " | "))
shutil.rmtree(tmp)

# ── 8. ⓔ 무결합(role=) → 종전 안내문 그대로(무회귀) + 두 왕복은 실제로 났다 ──
tmp = tempfile.mkdtemp(prefix="hook-t8-")
env = setup(tmp, "ok", reclaim_mode="none")
code, out, _ = run_hook(env)
check("8a ⓔ무결합: 종전 안내문 유지", "javis_bootstrap.py" in out and code == 0)
check("8b ⓔ무결합: 역할 지침 미주입", "DIRECTIVE-BODY-" not in out)
check("8c ⓔ무결합: 고지 없음", "역할 자동 복구" not in out)
_c = calls_of(tmp)
check("8d ⓔ무결합: surface-role·reclaim-role 왕복 각 1",
      "surface-role" in _c and "reclaim-role" in _c)
shutil.rmtree(tmp)

# ── 9. ⓕ 판정 불가(surface-role exit 2) → reclaim 왕복 0 ──
tmp = tempfile.mkdtemp(prefix="hook-t9-")
env = setup(tmp, "ok", reclaim_mode="undecidable")
code, out, _ = run_hook(env)
_c = calls_of(tmp)
check("9a ⓕ판정불가: reclaim 왕복 0", "reclaim-role" not in _c, _c.strip().replace("\n", " | "))
check("9b ⓕ판정불가: 고지 1줄", "역할 판정 불가" in out)
check("9c ⓕ판정불가: 역할 지침 미주입(안내문 유지)",
      "DIRECTIVE-BODY-" not in out and "javis_bootstrap.py" in out)
check("9d ⓕ판정불가: exit 0", code == 0)
shutil.rmtree(tmp)

# ── 10. ⓖ 형식 가드 — 역할명이 [a-zA-Z0-9_-] 밖이면 채택하지 않는다 ──
tmp = tempfile.mkdtemp(prefix="hook-t10-")
env = setup(tmp, "ok", reclaim_mode="bogus")
code, out, _ = run_hook(env)
check("10a ⓖ형식가드: 미채택(안내문 유지)", "javis_bootstrap.py" in out)
check("10b ⓖ형식가드: 지침 미주입", "DIRECTIVE-BODY-" not in out)
check("10c ⓖ형식가드: 고지 없음", "역할 자동 복구" not in out)
shutil.rmtree(tmp)

# ── 11. ★재핀(R1): 역할이 **있어도** 데몬에 묻는다 — 그러나 답이 `self` 면 종전 그대로다 ──
#   종전 핀은 "역할 보유 시 왕복 0"이었다. 그 규칙은 승계 뒤 전임자 셸의 stale `CYS_ROLE` 을
#   영원히 교정하지 못하게 만들어, 두 세션이 같은 역할로 행동하는 상태를 방치했다(적대검증
#   R1 major). 이제는 묻되 **강등은 `env_role=other_live` 에서만** 한다. (이 핀은 같은 WP-4
#   커밋에서 내가 세운 것이고 선행 릴리스의 계약이 아니다 — 반례가 아니라 재핀이다.)
tmp = tempfile.mkdtemp(prefix="hook-t11-")
env = setup(tmp, "ok", reclaim_mode="mine")
code, out, _ = run_hook(env, role="worker")
_c = calls_of(tmp)
check("11a 역할 보유해도 데몬 권위 조회는 한다", "reclaim-role" in _c and "surface-role" in _c)
check("11b env_role=self 면 종전 지침 주입(강등 없음)",
      "DIRECTIVE-BODY-WORKER" in out and "역할 주소 상실" not in out)
check("11c 역할 보유 시 --env-role 로 현재 역할을 신고", "--env-role worker" in _c,
      _c.strip().replace("\n", " | "))
shutil.rmtree(tmp)

# ── 11x. ★(R2 재핀 · 두 리뷰어 공통 blocking) **데몬의 답이 이긴다** ──
#   종전 규칙은 `role=` 을 `CYS_ROLE` 이 **빌 때만** 채택했다. 그 규칙에서는
#     ① 데몬이 `role=cso/bound` 로 이미 결합을 커밋했는데 훅이 그 답을 버리고 stale env 의
#        worker 지침을 주입했고(데몬과 세션이 서로 다른 역할을 믿는다),
#     ② worker 중복제거로 이 좌석이 정당하게 `worker-2` 를 쥔 경우 `env_role=other_live` 만
#        보고 **강등**해 살아 있는 역할 좌석을 지침 0 으로 만들었다(치명위험 ③).
#   이제 규칙은 하나다: `role=` 이 비어 있지 않으면 그것이 이 좌석의 역할이다.
tmp = tempfile.mkdtemp(prefix="hook-t11x-")
env = setup(tmp, "ok", reclaim_mode="bound_other")
code, out, _ = run_hook(env, role="worker")
check("11x-a ①커밋된 권위 역할을 채택한다(stale env 를 이긴다)", "DIRECTIVE-BODY-CSO" in out)
check("11x-b ①stale env 지침은 주입하지 않는다", "DIRECTIVE-BODY-WORKER" not in out)
check("11x-c ①교정 사실을 고지한다", "역할 교정" in out and "cso" in out)
shutil.rmtree(tmp)

tmp = tempfile.mkdtemp(prefix="hook-t11y-")
env = setup(tmp, "ok", reclaim_mode="dedup_live")
code, out, _ = run_hook(env, role="worker")
check("11y-a ②정당한 worker-2 좌석이 강등되지 않는다", "역할 주소 상실" not in out)
check("11y-b ②권위 역할(worker-2)의 지침을 받는다", "DIRECTIVE-BODY-WORKER" in out)
check("11y-c ②exit 0", code == 0)
shutil.rmtree(tmp)

# ── 11z. ★(R2 · codex minor) 처방이 있는 무결합 사유는 **세션에 보인다** ──
#   종전에는 둘째 줄(`reason=`)을 읽지 않고 stderr 도 버려서 "왜 안 붙었는지"도
#   "무엇을 하면 되는지"도 어디에도 남지 않았다.
tmp = tempfile.mkdtemp(prefix="hook-t11z-")
env = setup(tmp, "ok", reclaim_mode="priv_optin")
code, out, _ = run_hook(env)
check("11z-a 특권 opt-in 처방 고지", "--takeover-empty-seat" in out)
check("11z-b 자동 경로가 특권을 옮기지 않는다는 사실 명시", "특권 역할" in out)
shutil.rmtree(tmp)

tmp = tempfile.mkdtemp(prefix="hook-t11w-")
env = setup(tmp, "ok", reclaim_mode="axes_unknown")
code, out, _ = run_hook(env)
check("11w-a 축 미확정 고지 + 수동 처방", "cys claim-role" in out and "확정하지 못" in out)
shutil.rmtree(tmp)

# ── 11t. ★(독립 재유도) cwd 충돌은 축 미확정과 **다른 처방**을 낸다 ──
#   신고한 폴더와 좌석의 실제 폴더가 다르면 두 조건을 함께 만족하는 후보가 없다. 그 사실과
#   "신고로 남의 폴더 역할을 가져오지 않는다"를 말해야 사람이 무엇을 할지 안다.
tmp = tempfile.mkdtemp(prefix="hook-t11t-")
env = setup(tmp, "ok", reclaim_mode="cwd_conflict")
code, out, _ = run_hook(env)
check("11t-a cwd 충돌 사유가 그대로 고지된다", "실제 작업 폴더가 달라" in out)
check("11t-b 처방이 붙는다(해당 폴더에서 시작 · claim-role)",
      "cys claim-role" in out and "폴더에서 세션을 시작" in out)
check("11t-c 강등이 아니다", "역할 주소 상실" not in out)
check("11t-d 사유 코드는 종전 어휘 그대로다(처방만 진단 축으로 가른다)",
      "reported_cwd_conflict" not in out)
shutil.rmtree(tmp)

# ── 11s. ★(수렴 R2) **구 바이너리 3줄 응답**(넷째 줄 없음)에서도 종전대로 동작한다 ──
#   `detail=` 은 추가 줄이다. 없으면 빈 값이고, 없는 것을 충돌로 읽으면 정상 좌석에 엉뚱한
#   처방이 붙는다(그리고 있는 줄을 못 읽으면 처방이 사라진다). 양방향을 여기서 함께 잰다.
tmp = tempfile.mkdtemp(prefix="hook-t11s-")
env = setup(tmp, "ok", reclaim_mode="legacy3")
code, out, _ = run_hook(env)
check("11s-a 넷째 줄이 없어도 훅이 산다", code == 0)
check("11s-b 없는 진단 축을 충돌로 읽지 않는다", "실제 작업 폴더가 달라" not in out)
shutil.rmtree(tmp)

# ── 11v. ★(R2 · codex major) 판정된 '무역할' + 주인 없는 env 역할 → 지침은 주되 **미등록 고지** ──
#   지침을 끊으면 지침 없는 좌석을 새로 만든다(치명위험 ③). 그러나 등록되지 않았다는 사실을
#   숨기면 그 좌석은 자기 `cys` 명령이 왜 거부되는지 모른 채 역할처럼 행동한다.
tmp = tempfile.mkdtemp(prefix="hook-t11v-")
env = setup(tmp, "ok", reclaim_mode="unregistered")
code, out, _ = run_hook(env, role="worker")
check("11v-a 미등록 고지가 뜬다", "역할 등록이 없다" in out)
check("11v-b 처방(claim-role)이 붙는다", "cys claim-role worker" in out)
check("11v-c 지침은 그대로 주입한다(지침 없는 좌석을 만들지 않는다)", "DIRECTIVE-BODY-WORKER" in out)
check("11v-d 강등이 아니다", "역할 주소 상실" not in out)
shutil.rmtree(tmp)

# ── 11u. ★음성 대조: master|cso 는 아래 재대조가 그 자리에서 등록하므로 이 고지 대상이 아니다 ──
tmp = tempfile.mkdtemp(prefix="hook-t11u-")
env = setup(tmp, "ok", reclaim_mode="unregistered")
code, out, _ = run_hook(env, role="cso")
check("11u-a cso 에는 미등록 고지 없음(재대조가 등록한다)", "역할 등록이 없다" not in out)
check("11u-b cso 지침 주입 유지", "DIRECTIVE-BODY-CSO" in out)
shutil.rmtree(tmp)

# ── 13. ★강등: `env_role=other_live` — 그 역할을 지금 다른 산 좌석이 쥐었다 ──
tmp = tempfile.mkdtemp(prefix="hook-t13-")
env = setup(tmp, "ok", reclaim_mode="taken")
code, out, _ = run_hook(env, role="reviewer-codex")
check("13a 강등: 역할 지침 미주입", "DIRECTIVE-BODY-REVIEWER" not in out)
check("13b 강등: 인계 안내", "역할 주소 상실" in out and "reviewer-codex" in out)
check("13c 강등: 복구 경로 명시(claim-role)", "cys claim-role reviewer-codex" in out)
check("13e 강등은 `role=` 이 빈 경우에만(권위 역할이 오면 채택이 이긴다 — 11y 가 반례)",
      "역할 교정" not in out)
check("13d 강등: exit 0(좌석을 죽이지 않는다)", code == 0)
shutil.rmtree(tmp)

# ── 14. ★음성 대조: 같은 무결합이라도 `env_role` 이 other_live 가 **아니면** 강등하지 않는다 ──
#   "호출자에게 역할이 없다"로 내리면 경합 한 번에 살아 있는 좌석이 지침을 잃는다(치명위험 ③).
tmp = tempfile.mkdtemp(prefix="hook-t14-")
env = setup(tmp, "ok", reclaim_mode="none")  # role= · no_candidate · env_role=unknown
code, out, _ = run_hook(env, role="cso")
check("14a 무결합+env_role=unknown 은 강등 아님", "역할 주소 상실" not in out)
check("14b 종전 지침 주입 유지(fail-open)", "DIRECTIVE-BODY-CSO" in out)
shutil.rmtree(tmp)

# ── 15. ★판정 불가(surface-role exit 2)면 역할 보유 세션도 건드리지 않는다 ──
tmp = tempfile.mkdtemp(prefix="hook-t15-")
env = setup(tmp, "ok", reclaim_mode="undecidable")
code, out, _ = run_hook(env, role="worker")
_c = calls_of(tmp)
check("15a 판정 불가: reclaim 왕복 0", "reclaim-role" not in _c)
check("15b 판정 불가: 지침 유지·강등 없음",
      "DIRECTIVE-BODY-WORKER" in out and "역할 주소 상실" not in out)
check("15c 판정 불가: 역할 보유 세션엔 '판정 불가' 고지 없음(무역할 전용 문안)",
      "역할 판정 불가" not in out)
shutil.rmtree(tmp)

# ── 16. ★Windows 경로 표기: 두 인자가 cys_native_path 를 거쳐 나간다(소스 핀 + 행위 핀) ──
#   MSYS `$PWD`(/c/…)를 원문으로 넘기면 데몬의 네이티브 표기와 **항상** 어긋나 Windows 의
#   모든 호출이 무결합이 된다(WP-4 가 그 플랫폼에 배포되지 않는다).
tmp = tempfile.mkdtemp(prefix="hook-t16-")
env = setup(tmp, "ok", reclaim_mode="none")
code, out, _ = run_hook(env)
_c = calls_of(tmp)
check("16a --config·--cwd 가 실제로 실려 나간다", "--config" in _c and "--cwd" in _c)
_src = open(HOOK, encoding="utf-8").read()
check("16b 두 인자가 cys_native_path 를 경유(원문 전달 금지)",
      'cys_native_path "${CLAUDE_CONFIG_DIR:-}"' in _src and 'cys_native_path "$PWD"' in _src)
shutil.rmtree(tmp)

# ── 17. ★독립 재유도(triage · codex major #7): 손상된 응답은 강등 근거가 아니다 ──
#   `role=` 이 비어 있다는 사실은 **데몬이 판정해서 '이 좌석은 무역할'이라고 답했을 때만**
#   성립한다. 형식 위반·실패한 명령의 출력은 '판정 없음'이고, 그것으로 강등하면 데몬이 방금
#   결합해 준(reason=bound) 좌석까지 지침 0 으로 만든다(치명위험 ③ 바보 좌석).
tmp = tempfile.mkdtemp(prefix="hook-t17a-")
env = setup(tmp, "ok", reclaim_mode="bogus_live")
code, out, _ = run_hook(env, role="reviewer-codex")
check("17a 형식 위반 역할명 + other_live 에서 강등하지 않는다", "역할 주소 상실" not in out)
check("17b 형식 위반이어도 좌석은 지침을 받는다(무채택·무강등)",
      "DIRECTIVE-BODY-REVIEWER" in out)
shutil.rmtree(tmp)

tmp = tempfile.mkdtemp(prefix="hook-t17b-")
env = setup(tmp, "ok", reclaim_mode="garbled_live")
code, out, _ = run_hook(env, role="reviewer-codex")
check("17c 손상된 응답(첫 줄 비계약·exit 1)에서 강등하지 않는다", "역할 주소 상실" not in out)
check("17d 손상된 응답에서도 지침은 주입된다", "DIRECTIVE-BODY-REVIEWER" in out)
shutil.rmtree(tmp)

# ── 12. Windows 함정 회귀 — 재대조가 검증되지 않은 `timeout` 을 직접 부르지 않는다 ──
#   PortableGit 의 System32 timeout.exe 는 인자를 받으면 즉시 rc=1 이라, 재대조가 실행조차
#   되지 않은 채 '데몬 미응답'으로 접힌다. 판정은 소스에 있고 행위로는 mac 에서 못 재므로 소스 핀.
_hook_src = open(HOOK, encoding="utf-8").read()
_code_lines = [l for l in _hook_src.splitlines() if not l.lstrip().startswith("#")]
_code = "\n".join(_code_lines)
check("12a 훅이 bare `timeout N cys` 를 쓰지 않는다(cys_timeout_run 경유)",
      "timeout 2 cys" not in _code and "command -v timeout" not in _code)
#   ★R1: 종전 12b 는 헬퍼 **등장 횟수**만 셌다 — 한 호출이 bare 로 바뀌고 다른 곳에 헬퍼가
#   하나 더 생기면 그대로 통과한다. 이제 **각 호출의 실제 형태**를 본다.
check("12b-1 surface-role 이 cys_timeout_run 경유",
      "cys_timeout_run 5 cys surface-role" in _code)
check("12b-2 reclaim-role 이 cys_timeout_run 경유",
      "cys_timeout_run 12 cys reclaim-role --auto" in _code)
check("12b-3 claim-role 재대조가 cys_timeout_run 경유",
      "cys_timeout_run 2 cys claim-role" in _code)
check("12b-4 bare `cys surface-role`·`cys reclaim-role` 직접 호출 0",
      not any(l.strip().startswith(("cys surface-role", "cys reclaim-role"))
              for l in _code.splitlines()))
check("12c 훅에 ps·flock 없음(Windows 안전)",
      " ps " not in _code and "flock" not in _code)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
