#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_preflight_hook_body.py — preflight 가 훅 **본체** 실재를 잰다 (0.14.30 W-A · master 지시 ⑥).

무엇을 막는가: 부트 v2 A2 에서 UserPromptSubmit 훅이 **런처**(`role-bootstrap.sh`) + **본체**
(`role-bootstrap-legacy.sh`) 로 갈렸다. 등록되는 것은 런처뿐이고, 본체가 없으면 런처는 고지 1줄을
내고 `exit 0` 한다 — **훅은 정상 종료하는데 부트만 안 난다**. 훅 실패도 아니고 로그도 조용해서
어떤 축도 이 상태를 재지 않았다(부서 팩 복제 목록 결손·부분 배포에서 확정 재현되는 갈래).

C28 은 종전에 `SELFCORR_HOOKS`(=**등록** 집합)에서 실재 목록을 파생했다. 본체를 거기 넣으면
같은 이벤트에 훅이 둘 달려 선언 1건이 두 번 처리되고, 본체가 `$1` 없이 직접 불려 stdin 계약으로
되돌아간다. 그래서 **실재 전용 목록**(`HOOK_BODY_FILES`)을 따로 두는 것이 이 검체가 지키는 설계다.

  ① 본체가 있으면 본체 사유의 FAIL 이 없다
  ② 본체를 지우면 **FAIL**(각성 티어 — 등록 결손과 같은 결과이므로 같은 보고 크기)
  ③ 본체는 settings.json 에 **등록되지 않는다**(실재 전용 — 이중 발화 금지)
  ④ ★음성 대조: `HOOK_BODY_FILES` 를 비운 변조본은 본체를 지워도 FAIL 하지 않는다(계측 타당성)
  ⑤ ★음성 대조: 목록이 `SELFCORR_HOOKS` 와 **겹치지 않는다**(등록 집합 오염 금지)
출력: PASS/FAIL 행 · 실패 시 exit 1 · 종료 토큰 PREFLIGHT-HOOK-BODY-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_preflight_hook_body.py
"""
import importlib.util
import io
import json
import os
import shutil
import stat
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PF_PATH = os.path.join(BIN, "javis_preflight.py")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def w(path, body, mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def build_pack(pack, PF, with_body=True):
    """C28 이 요구하는 훅·엔진을 갖춘 가짜 팩(등록 판정만 재려면 존재만 충분)."""
    for script, _ in PF.SELFCORR_HOOKS:
        w(os.path.join(pack, "hooks", script), "#!/bin/sh\nexit 0\n")
    w(os.path.join(pack, "bin", "javis_reflect.py"), "#!/usr/bin/env python3\n", 0o755)
    if with_body:
        for rel, _owner in PF.HOOK_BODY_FILES:
            w(os.path.join(pack, rel), "#!/bin/bash\nexit 0\n")


def run_c28(PF, pack, home, cfg):
    """C28 만 돌려 (status, detail) 반환. 등록은 하지 않는다(fix=False = report 모드)."""
    prev = {k: os.environ.get(k) for k in
            ("CYS_PACK_DIR", "HOME", "CLAUDE_CONFIG_DIR", "CYS_ACCOUNT_DIR")}
    os.environ["CYS_PACK_DIR"] = pack
    os.environ["HOME"] = home
    os.environ["CLAUDE_CONFIG_DIR"] = cfg
    os.environ.pop("CYS_ACCOUNT_DIR", None)
    try:
        pf = PF.Preflight(False, [])
        pf.c28_self_correction()
        row = [r for r in pf.results if r["id"] == "C28.self-correction"]
        return (row[0]["status"], row[0]["detail"]) if row else (None, "")
    finally:
        for k, v in prev.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


root = tempfile.mkdtemp()
try:
    PF = load(PF_PATH, "_pf_body_live")

    # ⑤ 설계 불변식 — 실재 전용 목록이 등록 집합과 겹치면 안 된다
    reg = {s for s, _ in PF.SELFCORR_HOOKS}
    bodies = [os.path.basename(rel) for rel, _ in PF.HOOK_BODY_FILES]
    check("5a HOOK_BODY_FILES 실재(목록 자체가 있어야 이 검체가 성립)", bool(PF.HOOK_BODY_FILES))
    check("5b 본체는 등록 집합(SELFCORR_HOOKS)과 겹치지 않는다(이중 발화 금지)",
          not (set(bodies) & reg), repr(sorted(set(bodies) & reg)))
    check("5c 본체의 owner 는 각성 훅이다(=FAIL 티어 근거)",
          all(o in PF.AWAKENING_SCRIPTS for _r, o in PF.HOOK_BODY_FILES),
          repr([o for _r, o in PF.HOOK_BODY_FILES]))

    # ① 본체가 있으면 본체 사유 FAIL 없음
    pack = os.path.join(root, "pack-ok")
    home = os.path.join(root, "home-ok")
    cfg = os.path.join(root, "cfg-ok")
    os.makedirs(home); os.makedirs(cfg)
    build_pack(pack, PF, with_body=True)
    st_ok, det_ok = run_c28(PF, pack, home, cfg)
    check("1 본체가 있으면 본체 사유 보고 0",
          "role-bootstrap-legacy.sh" not in det_ok, repr(det_ok[:200]))

    # ② 본체를 지우면 FAIL
    pack2 = os.path.join(root, "pack-nobody")
    home2 = os.path.join(root, "home-nobody")
    cfg2 = os.path.join(root, "cfg-nobody")
    os.makedirs(home2); os.makedirs(cfg2)
    build_pack(pack2, PF, with_body=False)
    st_no, det_no = run_c28(PF, pack2, home2, cfg2)
    check("2a 본체 부재 → C28 FAIL(각성 티어 · 등록 결손과 같은 보고 크기)",
          st_no == PF.FAIL, repr((st_no, det_no[:220])))
    check("2b 사유가 **무엇이 없는지·왜 무관측이었는지**를 말한다",
          "role-bootstrap-legacy.sh" in det_no and "부재" in det_no
          and "부트를 발화하지 않는다" in det_no and "무관측" in det_no,
          repr(det_no[:340]))

    # ③ 본체는 settings.json 에 등록되지 않는다(실재 전용)
    cfgf = os.path.join(cfg, "settings.json")
    reg_txt = io.open(cfgf, encoding="utf-8").read() if os.path.isfile(cfgf) else ""
    check("3 본체가 훅으로 등록되지 않았다(실재 전용 · 이중 발화 0)",
          "role-bootstrap-legacy" not in reg_txt, repr(reg_txt[:200]))

    # ④ ★음성 대조 — 목록을 비운 변조본은 본체를 지워도 FAIL 하지 않는다
    mut_path = os.path.join(root, "javis_preflight_mut.py")
    src = io.open(PF_PATH, encoding="utf-8").read()
    anchor = 'HOOK_BODY_FILES = [\n    (os.path.join("hooks", "role-bootstrap-legacy.sh"), "role-bootstrap.sh"),\n]'
    check("4a 변조 앵커 실재(계측 타당성)", anchor in src)
    io.open(mut_path, "w", encoding="utf-8", newline="\n").write(
        src.replace(anchor, "HOOK_BODY_FILES = []", 1))
    PFM = load(mut_path, "_pf_body_mut")
    pack3 = os.path.join(root, "pack-mut")
    home3 = os.path.join(root, "home-mut")
    cfg3 = os.path.join(root, "cfg-mut")
    os.makedirs(home3); os.makedirs(cfg3)
    build_pack(pack3, PFM, with_body=False)
    st_m, det_m = run_c28(PFM, pack3, home3, cfg3)
    check("4b 목록을 비우면 본체 부재가 **무관측**이 된다 → 이 검체가 재는 것이 그 목록이다",
          "role-bootstrap-legacy.sh" not in det_m, repr(det_m[:200]))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("PREFLIGHT-HOOK-BODY-OK")
sys.exit(0)
