#!/usr/bin/env python3
"""test_role_bootstrap_hook.py — 마스터 선언 발화 계층 회귀(감지기 함수 + 훅 통합).

제품 절대요구(출하 기본 계약): "너는 마스터다" 입력 → 부트스트랩 100% 발화. 2회 성찰(적대검증+아키텍트)이
지적한 트리거 미검증(arch#2)·감지 오류(adv#2)·role-blind(arch#1)·인용 오발화(adv#8)·부정 오억제(adv#7)를
회귀로 핀한다.

★T-0147-7 W1b 구조 변경(CS-8): 감지·억제 판정이 훅의 셸 grep 스택에서 **python 단일 함수**
  (`bin/javis_detect.py::detect`)로 이관됐다. 그래서 이 파일은 두 층을 나눠 검증한다:
    ① **함수 직접 검증**(빠르고 결정론적 · corpus 본진) — 절 경계 스코프(A4)·주어 어휘
       (P3-A-NEGA)·filler 15/16 경계(P3-A-FILLER)·감지창 200 **문자** 경계(G25).
    ② **훅 통합 검증**(프로세스 경계가 필요한 것만) — 기존 발화/무시 행렬 전량 보존 + role
       allowlist 반전(A3)·surface-role 판정불가 fail-closed(A5)·LC_ALL=C 파리티(G9)·
       A2 surface 게이트.
  ①의 케이스를 ②로 중복 실행하지 않는 이유는 비용이다(훅 1회 ≈ 프로세스 3개). 대표 케이스만
  훅으로 교차 확인해 '함수는 맞는데 배선이 틀린' 구멍을 막는다.

관측 기법(②): 격리 HOME + 빈 팩(목 javis_bootstrap.py) + PATH 앞 목 cys(surface-role 반환값·rc 주입)로
훅을 실행하고, stdout에 "발화됨" NOTE가 있으면 발화, 없으면 무시로 판정.
"""
import json
import os
import subprocess
import sys
import tempfile

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))       # cysjavis-pack/bin
HOOK = os.path.join(os.path.dirname(BIN), "hooks", "role-bootstrap.sh")
sys.path.insert(0, BIN)
import javis_detect  # noqa: E402  (형제 모듈 — 감지기 단일 소유)


# ══════════════════════════════════════════════════════════════════════════════
# ① 감지기 함수 직접 검증 — corpus 본진
# ══════════════════════════════════════════════════════════════════════════════
# 기존 훅 시대의 회귀 corpus(전량 보존 — W1a가 결정론화한 그 목록).
LEGACY_FIRE = [
    "너는 마스터다", "너는 이제 마스터다", "너는 지금부터 마스터다", "너가 마스터야",
    "당신은 우리의 마스터입니다", "너는 마스터로 각성하라", "you are the master",
    "지금부터 너는 마스터가 된다",
]
LEGACY_SKIP = [
    "너는 마스터가 아니다", "'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
    "오늘 작업 지시해줘", "너는 워커다", "마스터 브랜치를 확인해줘", "너는 오늘 마스터 브랜치 봐",
]
# W1b 신규 — 재감사가 실측으로 확정한 결함 재현 케이스.
NEW_FIRE = [
    # A4 혼합 의도: 선언 뒤 후속 질문이 붙어도 발화(구 코드는 '?' 하나로 전량 억제 — 실측 미발화)
    "너는 마스터다. 오늘 뭐부터 할까?",
    "너는 이제 마스터다! 무슨 일부터 시작할까?",
    "너는 마스터다.\n오늘 작업 목록은 뭐로 잡을까?",
    "너는 마스터다; 첫 작업은 뭐로 할까?",          # 세미콜론도 절 경계다
    # ※경계 명시(스펙 그대로): 절 경계 **없이** 이어붙인 의문("너는 마스터다 뭐부터 할까?")은
    #   억제가 정답이다 — 재감사 A4의 처방은 "거리 튜닝"이 아니라 "절 경계 판정"이므로, 문장을
    #   끊지 않은 입력에서 선언과 질문은 같은 절이다. 이 방향의 보수성이 오발화보다 안전하다.
    # P3-A-NEGA 표준 정서법·구어 주어(구 어휘엔 없어 전부 미발화 실측)
    "네가 마스터다", "니가 마스터다", "당신이 마스터다", "네가 이제 마스터야",
    "지금부터 네가 마스터가 된다",
    # 억제 마커가 **다른 절**에 있으면 발화(절 경계 스코프의 대칭 확인)
    "'설계 문서'를 예시로 보여줘. 그리고 너는 마스터다.",
]
NEW_SKIP = [
    # A4 반대편: 같은 절 안의 의문·인용은 여전히 억제(과교정 방지)
    "'네가 마스터다'가 무슨 뜻?", "\"당신이 마스터다\"라고 입력하면 어떻게 되나요?",
    "'니가 마스터다'가 무슨 의미인지 설명해줘",
    "너는 마스터다 처럼 들리는 문장을 만들어줘",
    # P3-A-NEGA 과확장 금지: 주어가 아닌 `네`·`당신` 단독은 발화하지 않는다
    "네 마스터 브랜치를 봐줘", "당신 마스터키 어디 뒀어",
]


def fn_matrix(fails):
    for p in LEGACY_FIRE + NEW_FIRE:
        v = javis_detect.detect(p)
        if not v["fire"]:
            fails.append("fn FALSE-NEGATIVE %r → %s" % (p, v["reason"]))
    for p in LEGACY_SKIP + NEW_SKIP:
        v = javis_detect.detect(p)
        if v["fire"]:
            fails.append("fn FALSE-POSITIVE %r → %s" % (p, v["reason"]))

    # ── filler 경계(P3-A-FILLER): 15자=발화 / 16자=미발화. 주석(구 '12자')과 코드의 불일치를
    #    수치 상수(FILLER_MAX)로 못박고 경계를 corpus로 박제한다.
    fmax = javis_detect.FILLER_MAX
    if fmax != 15:
        fails.append("FILLER_MAX 스펙 이탈(%r≠15)" % fmax)
    if not javis_detect.detect("너는" + "가" * fmax + "마스터다")["fire"]:
        fails.append("filler %d자 경계 미발화" % fmax)
    if javis_detect.detect("너는" + "가" * (fmax + 1) + "마스터다")["fire"]:
        fails.append("filler %d자 오발화(경계 누수)" % (fmax + 1))

    # ── 감지창 경계(G25): **문자** 200자. GNU cut -c(바이트)면 한글 약 66자에서 잘렸다.
    w = javis_detect.WINDOW_CHARS
    if w != 200:
        fails.append("WINDOW_CHARS 스펙 이탈(%r≠200)" % w)
    decl = "너는 마스터다"
    inside = "가" * (w - len(decl)) + decl          # 선언이 창 **끝**에 정확히 걸린다
    if not javis_detect.detect(inside)["fire"]:
        fails.append("감지창 %d자 경계(한글)에서 미발화 — 바이트 슬라이스 회귀" % w)
    if len(inside.encode("utf-8")) <= w:
        fails.append("경계 검체가 바이트 관점에서도 창 안이라 회귀를 못 잡는다(검체 무효)")
    if javis_detect.detect("가" * w + decl)["fire"]:
        fails.append("감지창 밖 선언이 발화(창 미적용)")

    # ── 억제/미검출 분리(CS-8⑤ 로그 의무의 전제): verdict 타입이 융합되면 훅이 분기 못 한다
    if javis_detect.detect("'너는 마스터다'가 무슨 뜻?")["verdict"] != "suppressed":
        fails.append("인용·의문 케이스 verdict≠suppressed")
    if javis_detect.detect("오늘 작업 지시해줘")["verdict"] != "no_declaration":
        fails.append("무선언 케이스 verdict≠no_declaration")

    # ── 감지기 CLI 계약(훅이 소비하는 exit 표) ──
    for prompt, want in (("너는 마스터다", 0), ("오늘 작업 지시해줘", 1),
                         ("'너는 마스터다'가 무슨 뜻?", 3)):
        r = subprocess.run([sys.executable, os.path.join(BIN, "javis_detect.py"), "hook-gate"],
                           input=json.dumps({"prompt": prompt}), capture_output=True,
                           text=True, timeout=30)
        if r.returncode != want:
            fails.append("CLI exit 계약 이탈 %r: %d≠%d" % (prompt, r.returncode, want))
        if want == 3 and "SKIP(억제)" not in r.stderr:
            fails.append("억제인데 로그 1줄이 없다(침묵 억제 회귀): %r" % r.stderr[:200])
        if want == 1 and r.stderr.strip():
            fails.append("무선언인데 로그를 냈다(정상 침묵 위반): %r" % r.stderr[:200])
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_detect.py"), "hook-gate"],
                       input="{not json", capture_output=True, text=True, timeout=30)
    if r.returncode != 2:
        fails.append("입력 파싱 불가가 cannot-judge(2)로 분리되지 않음: %d" % r.returncode)


# ══════════════════════════════════════════════════════════════════════════════
# ② 훅 통합 검증
# ══════════════════════════════════════════════════════════════════════════════
def _run_hook(prompt, surface_role="", surface_env=True, role_rc=0, extra_env=None):
    """훅을 격리 실행. surface_role = 목 cys surface-role 반환값(빈=미claim). 반환: (발화 bool, stdout).

    surface_env=False 는 **cys 밖**(VS Code 등 임의 claude 세션)을 재현한다 — A2 게이트 핀.
    role_rc≠0 은 surface-role **판정 불가**(데몬 미응답·hang 등)를 재현한다 — A5 fail-closed 핀.
    """
    home = tempfile.mkdtemp()
    pack = tempfile.mkdtemp()
    mockbin = tempfile.mkdtemp()
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    # 목 javis_bootstrap.py (부트 안 함 — 존재만)
    with open(os.path.join(pack, "bin", "javis_bootstrap.py"), "w") as f:
        f.write("#!/usr/bin/env python3\nprint('MOCK')\n")
    # 목 cys — surface-role만 주입, 나머지 no-op
    cysp = os.path.join(mockbin, "cys")
    with open(cysp, "w") as f:
        f.write('#!/bin/bash\n[ "$1" = surface-role ] && { echo "%s"; exit %d; }\nexit 0\n'
                % (surface_role, role_rc))
    os.chmod(cysp, 0o755)
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_PACK_DIR"] = pack
    env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
    env.pop("CYS_SOCKET", None)
    # ★A2 surface 이중 게이트(T-0147-7 W1a): 훅은 cys pane 안에서만 발화한다. 이 하네스는
    # 'cys pane 안'을 재현하므로 CYS_SURFACE_ID를 **명시** 주입한다 — os.environ 상속에 의존하면
    # 테스트 결과가 실행 위치(cys pane 안/밖)에 따라 흔들린다(결정론 파괴).
    env.pop("AITERM_SURFACE_ID", None)
    # ★임무 게이트(2026-08-01 D4-a): 훅의 **발화(spawn)** 는 이제 "오너가 이 세션에 임무를
    #   지정했는가"로 갈린다 — 임무가 없으면 관측·기록만 하고 노드를 띄우지 않는다.
    #   이 하네스가 재는 것은 **감지·게이트 배선**(A2·A4·A3=B7·G9·G25)이고 그 계약은
    #   '선언을 감지하면 발화한다'이므로, 발화가 계약인 조건 = 임무 지정 세션을 명시 주입한다
    #   (실재 수단: `javis_mission.gate()` ①번 신호 `CYS_MISSION`).
    #   ★정정(2026-08-01 R2 적발 (d)): 이 변수를 자동으로 채우는 launcher 는 **없다** — pane env 는
    #   데몬 프로세스 env 를 상속하므로 `cys launch-agent` 호출 시점의 값은 전달되지 않는다.
    #   여기서는 하네스가 직접 주입한다. 상속에 기대면 실행 위치에 따라 결과가 흔들린다(결정론 파괴).
    #   ※임무 미지정 경로(신규 사용자 · spawn 0)의 계약은 run_bootstrap_health H-MISSION-1 소속.
    env["CYS_MISSION"] = "test_role_bootstrap_hook 검체 임무(오너 지정 가정)"
    if surface_env:
        env["CYS_SURFACE_ID"] = "7"
    else:
        env.pop("CYS_SURFACE_ID", None)
    if extra_env:
        env.update(extra_env)
    try:
        r = subprocess.run(["bash", HOOK], input=json.dumps({"prompt": prompt}),
                           capture_output=True, text=True, timeout=30, env=env)
    except Exception as e:
        return False, "exec 실패: %s" % e
    return ("발화됨" in r.stdout), r.stdout + r.stderr


# 훅 통합 행렬 — 기존 목록 전량 보존(회귀 계약).
FIRE = LEGACY_FIRE
SKIP = LEGACY_SKIP
# 훅으로 교차 확인하는 W1b 대표 케이스(함수 검증과 이중화하지 않되 배선은 확인).
HOOK_NEW_FIRE = ["너는 마스터다. 오늘 뭐부터 할까?", "네가 마스터다", "당신이 마스터다"]
HOOK_NEW_SKIP = ["'네가 마스터다'가 무슨 뜻?"]
# A3 allowlist 반전 — 구 denylist가 통과시켰던 좌석들이 전부 차단돼야 한다.
NON_MASTER_ROLES = ["worker", "cso", "reviewer-gemini", "reviewer-codex",
                    "worker-2", "cso-1", "reviewer-claude-1", "reviewer-grok",
                    "verifier", "unknown-role"]


def main():
    fails = []
    fn_matrix(fails)

    # 1. 감지 행렬 — 발화해야 함
    for p in FIRE + HOOK_NEW_FIRE:
        fired, _ = _run_hook(p)
        if not fired:
            fails.append("FALSE-NEGATIVE(발화 안 됨): %r" % p)
    # 2. 감지 행렬 — 무시해야 함
    for p in SKIP + HOOK_NEW_SKIP:
        fired, _ = _run_hook(p)
        if fired:
            fails.append("FALSE-POSITIVE(오발화): %r" % p)
    # 3. ★A3 role 게이트 allowlist 반전 — 비-master 좌석 전부 차단(미지 role 포함)
    for role in NON_MASTER_ROLES:
        fired, out = _run_hook("너는 마스터다", surface_role=role)
        if fired:
            fails.append("A3 회귀(%s pane에서 마스터 부트 오발화 — denylist 잔존): %r" % (role, out[:200]))
    # 4. role-aware — master·미claim은 발화 허용
    for role in ("master", ""):
        fired, _ = _run_hook("너는 마스터다", surface_role=role)
        if not fired:
            fails.append("role='%s'에서 발화 안 됨(정상 마스터 선언 차단)" % (role or "미claim"))

    # 5. ★A2 surface 이중 게이트 — cys 밖(CYS_SURFACE_ID·AITERM_SURFACE_ID 둘 다 부재)은 무발화
    fired, _ = _run_hook("너는 마스터다", surface_env=False)
    if fired:
        fails.append("A2 회귀: surface env 부재(비-cys 터미널)에서 마스터 부트 오발화")

    # 6. ★A5 surface-role 판정 불가(rc≠0) — 무발화 + 로그(빈값=미claim 과 분리)
    fired, out = _run_hook("너는 마스터다", surface_role="", role_rc=3)
    if fired:
        fails.append("A5 회귀: surface-role 판정 불가인데 발화(fail-open)")
    if "판정 불가" not in out:
        fails.append("A5 회귀: 판정 불가가 침묵으로 접혔다(로그 없음): %r" % out[:200])

    # 7. ★G9 LC_ALL=C 파리티 — 로케일이 감지 결과를 바꾸면 안 된다(구 grep은 바이트를 셌다)
    for p in ("너는 이제 마스터다", "너는" + "가" * javis_detect.FILLER_MAX + "마스터다"):
        fired_c, _ = _run_hook(p, extra_env={"LC_ALL": "C", "LANG": "C"})
        if not fired_c:
            fails.append("G9 회귀: LC_ALL=C 에서 미발화 %r" % p)
    fired_c, _ = _run_hook("'너는 마스터다'가 무슨 뜻이야?", extra_env={"LC_ALL": "C", "LANG": "C"})
    if fired_c:
        fails.append("G9 회귀: LC_ALL=C 에서 억제 케이스가 오발화")

    # 8. ★G25 감지창 200 **문자** — 한글 경계에서 훅 배선까지 문자 단위인지(셸 슬라이스 제거 확인)
    decl = "너는 마스터다"
    fired, _ = _run_hook("가" * (javis_detect.WINDOW_CHARS - len(decl)) + decl)
    if not fired:
        fails.append("G25 회귀: 한글 200자 창 경계 선언이 훅에서 미발화(바이트 슬라이스 잔존)")
    fired, _ = _run_hook("가" * javis_detect.WINDOW_CHARS + decl)
    if fired:
        fails.append("G25 회귀: 감지창 밖 선언이 훅에서 발화")

    if fails:
        print("FAIL (%d):" % len(fails))
        for f in fails:
            print("  -", f)
        sys.exit(1)
    print("PASS: 함수 corpus %d발화/%d무시(+filler·창 경계·CLI exit 계약) + 훅 %d발화/%d무시 + "
          "A3 allowlist %dskip/2fire + A2 1skip + A5 판정불가 1skip + G9 파리티 3 + G25 경계 2"
          % (len(LEGACY_FIRE) + len(NEW_FIRE), len(LEGACY_SKIP) + len(NEW_SKIP),
             len(FIRE) + len(HOOK_NEW_FIRE), len(SKIP) + len(HOOK_NEW_SKIP),
             len(NON_MASTER_ROLES)))


if __name__ == "__main__":
    main()
