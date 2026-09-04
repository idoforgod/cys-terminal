#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_preflight_npm_prefix.py — preflight 가 데몬의 번들 오염 판정을 **소비**한다 (0.14.30 W-A · W-B #2).

무엇을 재는가: `npm_config_prefix` 가 설치본 안을 가리키면 npm 전역 설치가 설치본을 바꿔
코드서명·봉인이 깨진다. 판정 주체는 **데몬**이고(`cys status --json` →
`result.daemon.npm_prefix_polluted` · bool · 항상 존재 · 호출마다 재평가) preflight 는 그 bool 을
**소비만** 한다 — 같은 술어를 python 으로 다시 구현하면 경로 정규화·Windows 대소문자·형제 접두
같은 함정이 두 벌이 되고, 두 판정이 갈리는 순간 사용자는 pane 고지와 preflight 에서 **서로 다른
처방**을 받는다.

★이 검체의 심장은 ③이다: **키 부재는 '깨끗함'이 아니다**. 구 데몬·부트 v2 미배선 기계에서 키가
없을 때 그것을 PASS 로 접으면 이 축은 전 기계에서 거짓 초록이 되고, FAIL 로 접으면 스큐가 부트를
막는다. 재지 못한 것은 결손도 통과도 아니다 — SKIP 이다(이 파일이 exit 2 로 지키는 계급 구분).

  ① polluted=true  → WARN + 처방 문안
  ② polluted=false → PASS
  ③ 키 부재        → **SKIP**(거짓 초록 차단)
  ④ rc≠0           → SKIP(미측정 — 데몬 미가동)
  ⑤ stdout 비-JSON → SKIP(판독 불가)
  ⑥ ★음성 대조 — 키 부재를 '깨끗함'으로 접는 변조본은 ③에서 PASS 를 낸다(=③이 재는 것이 그 분기다)
  ⑦ 문안 파리티 — Rust 정본(`npm_prefix_bundle_warning`)이 이 레인에 있으면 핵심 문장을 대조한다
출력: PASS/FAIL 행 · 실패 시 exit 1 · 종료 토큰 PREFLIGHT-NPM-PREFIX-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_preflight_npm_prefix.py
"""
import importlib.util
import io
import json
import os
import shutil
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
REPO = os.path.dirname(os.path.dirname(BIN))
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


def fake_cys(binp, body):
    """가짜 `cys` — `status --json` 응답을 스크립트에 박아 넣는다(데몬 불요)."""
    os.makedirs(binp, exist_ok=True)
    p = os.path.join(binp, "cys")
    with io.open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(p, 0o755)
    return p


def run_c81(PF, binp):
    prev_path = os.environ.get("PATH")
    os.environ["PATH"] = binp + os.pathsep + (prev_path or "")
    try:
        pf = PF.Preflight(False, [])
        pf.c81_npm_prefix_polluted()
        row = [r for r in pf.results if r["id"] == "C81.npm-prefix-polluted"]
        return (row[0]["status"], row[0]["detail"]) if row else (None, "")
    finally:
        if prev_path is None:
            os.environ.pop("PATH", None)
        else:
            os.environ["PATH"] = prev_path


def status_stub(daemon_obj):
    """rc 0 + `result.daemon` 을 담은 status 응답 스텁."""
    payload = json.dumps({"result": {"daemon": daemon_obj}}, ensure_ascii=False)
    return "#!/bin/sh\ncat <<'JSON'\n%s\nJSON\nexit 0\n" % payload


root = tempfile.mkdtemp()
try:
    PF = load(PF_PATH, "_pf_npm_live")

    # ① polluted=true → WARN + 처방
    b1 = os.path.join(root, "b1")
    fake_cys(b1, status_stub({"npm_prefix_polluted": True}))
    st1, d1 = run_c81(PF, b1)
    check("1a polluted=true → WARN(값을 덮지 않는 계약이라 FAIL 아님)", st1 == PF.WARN, repr((st1, d1[:120])))
    check("1b 처방 문안이 실린다(정본 문장 공유)",
          "npm_config_prefix" in d1 and "코드서명" in d1 and "덮지 않습니다" in d1, repr(d1[:200]))

    # ② polluted=false → PASS
    b2 = os.path.join(root, "b2")
    fake_cys(b2, status_stub({"npm_prefix_polluted": False}))
    st2, d2 = run_c81(PF, b2)
    check("2 polluted=false → PASS", st2 == PF.PASS, repr((st2, d2[:120])))

    # ③ ★키 부재 → SKIP (거짓 초록 차단 — 이 검체의 심장)
    b3 = os.path.join(root, "b3")
    fake_cys(b3, status_stub({"pid": 1234}))          # daemon 은 있는데 키가 없다
    st3, d3 = run_c81(PF, b3)
    check("3a 키 부재 → SKIP(미측정 — '깨끗함'으로 접지 않는다)", st3 == PF.SKIP, repr((st3, d3[:160])))
    check("3b 사유가 구 데몬·미배선을 지목한다", "키 부재" in d3 and "깨끗함" in d3, repr(d3[:200]))

    # ④ rc≠0 → SKIP
    b4 = os.path.join(root, "b4")
    fake_cys(b4, "#!/bin/sh\nexit 1\n")
    st4, d4 = run_c81(PF, b4)
    check("4 cys status rc≠0 → SKIP(미측정 · 데몬 미가동)", st4 == PF.SKIP, repr((st4, d4[:120])))

    # ⑤ 비-JSON → SKIP
    b5 = os.path.join(root, "b5")
    fake_cys(b5, "#!/bin/sh\necho 'not json'\nexit 0\n")
    st5, d5 = run_c81(PF, b5)
    check("5 stdout 비-JSON → SKIP(판독 불가)", st5 == PF.SKIP, repr((st5, d5[:120])))

    # ⑥ ★음성 대조 — 키 부재를 '깨끗함'으로 접는 변조본은 ③에서 PASS 를 낸다
    src = io.open(PF_PATH, encoding="utf-8").read()
    anchor = '        if "npm_prefix_polluted" not in daemon:'
    check("6a 변조 앵커 실재(계측 타당성)", anchor in src)
    mut_path = os.path.join(root, "pf_mut.py")
    io.open(mut_path, "w", encoding="utf-8", newline="\n").write(
        src.replace(anchor, '        if False:', 1))
    PFM = load(mut_path, "_pf_npm_mut")
    stm, dm = run_c81(PFM, b3)
    check("6b 키 부재를 접으면 **PASS 로 뒤집힌다** → ③이 재는 것이 그 분기다",
          stm == PFM.PASS and st3 == PF.SKIP, repr((stm, dm[:120])))

    # ⑦ 문안 파리티 — Rust 정본이 이 레인에 있으면 핵심 문장을 대조한다
    lib_rs = os.path.join(REPO, "src", "lib.rs")
    rust = io.open(lib_rs, encoding="utf-8").read() if os.path.isfile(lib_rs) else ""
    if "npm_prefix_bundle_warning" in rust:
        keys = ["npm_config_prefix 가 설치본 안", "코드서명·봉인이 깨져",
                "그래도 값은 덮지 않습니다", "설치본 밖"]
        missing = [k for k in keys if k not in rust or k not in PF.NPM_PREFIX_BUNDLE_WARNING]
        check("7 문안 파리티: Rust 정본과 핵심 문장 일치", not missing, repr(missing))
    else:
        # 정본이 아직 이 레인에 없다 — **없다는 사실을 명시**한다(조용한 통과 금지).
        check("7 문안 파리티 보류: Rust 정본(`npm_prefix_bundle_warning`)이 이 레인에 없다",
              True, "src/lib.rs 미보유 — W-B 착지 후 이 축이 자동으로 켜진다(조건부 대조)")
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("PREFLIGHT-NPM-PREFIX-OK")
sys.exit(0)
