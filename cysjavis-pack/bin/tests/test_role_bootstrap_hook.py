#!/usr/bin/env python3
"""test_role_bootstrap_hook.py — role-bootstrap.sh(UserPromptSubmit 결정론 부트 발화) 트리거 검증.

오너 절대요구(2026-07-15): "너는 마스터다" 입력 → 부트스트랩 100% 발화. 2회 성찰(적대검증+아키텍트)이
지적한 트리거 미검증(arch#2)·감지 오류(adv#2)·role-blind(arch#1)·인용 오발화(adv#8)·부정 오억제(adv#7)를
회귀로 핀한다. 훅 '발화 판정'만 검증(실제 부트는 목 javis_bootstrap.py로 대체 — 노드 스폰 없음).

관측 기법: 격리 HOME + 빈 팩(목 javis_bootstrap.py) + PATH 앞 목 cys(surface-role 반환값 주입)로
훅을 실행하고, stdout에 "발화됨" NOTE가 있으면 발화, 없으면 무시로 판정.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile

HOOK = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__)))), "hooks", "role-bootstrap.sh")


def _run_hook(prompt, surface_role="", surface_env=True):
    """훅을 격리 실행. surface_role = 목 cys surface-role 반환값(빈=미claim). 반환: 발화 여부(bool).

    surface_env=False 는 **cys 밖**(VS Code 등 임의 claude 세션)을 재현한다 — A2 게이트 핀.
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
        f.write('#!/bin/bash\n[ "$1" = surface-role ] && { echo "%s"; exit 0; }\nexit 0\n' % surface_role)
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
    if surface_env:
        env["CYS_SURFACE_ID"] = "7"
    else:
        env.pop("CYS_SURFACE_ID", None)
    try:
        r = subprocess.run(["bash", HOOK], input=json.dumps({"prompt": prompt}),
                           capture_output=True, text=True, timeout=20, env=env)
    except Exception as e:
        return False, "exec 실패: %s" % e
    return ("발화됨" in r.stdout), r.stdout


FIRE = [
    "너는 마스터다", "너는 이제 마스터다", "너는 지금부터 마스터다", "너가 마스터야",
    "당신은 우리의 마스터입니다", "너는 마스터로 각성하라", "you are the master",
    "지금부터 너는 마스터가 된다",
]
SKIP = [
    "너는 마스터가 아니다", "'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
    "오늘 작업 지시해줘", "너는 워커다", "마스터 브랜치를 확인해줘", "너는 오늘 마스터 브랜치 봐",
]


def main():
    fails = []
    # 1. 감지 행렬 — 발화해야 함
    for p in FIRE:
        fired, _ = _run_hook(p)
        if not fired:
            fails.append("FALSE-NEGATIVE(발화 안 됨): %r" % p)
    # 2. 감지 행렬 — 무시해야 함
    for p in SKIP:
        fired, _ = _run_hook(p)
        if fired:
            fails.append("FALSE-POSITIVE(오발화): %r" % p)
    # 3. role-aware 게이트 — 비-마스터 pane은 오발화 금지(arch#1)
    for role in ("worker", "cso", "reviewer-gemini", "reviewer-codex"):
        fired, _ = _run_hook("너는 마스터다", surface_role=role)
        if fired:
            fails.append("ROLE-BLIND(%s pane에서 마스터 부트 오발화): arch#1 회귀" % role)
    # 4. role-aware — master·미claim은 발화 허용
    for role in ("master", ""):
        fired, _ = _run_hook("너는 마스터다", surface_role=role)
        if not fired:
            fails.append("role='%s'에서 발화 안 됨(정상 마스터 선언 차단)" % (role or "미claim"))

    # 5. ★A2 surface 이중 게이트 — cys 밖(CYS_SURFACE_ID·AITERM_SURFACE_ID 둘 다 부재)은 무발화
    fired, _ = _run_hook("너는 마스터다", surface_env=False)
    if fired:
        fails.append("A2 회귀: surface env 부재(비-cys 터미널)에서 마스터 부트 오발화")

    if fails:
        print("FAIL (%d):" % len(fails))
        for f in fails:
            print("  -", f)
        sys.exit(1)
    print("PASS: 감지 %d발화/%d무시 + role-aware 4skip/2fire + A2 surface 게이트 1skip — 전건 통과"
          % (len(FIRE), len(SKIP)))


if __name__ == "__main__":
    main()
