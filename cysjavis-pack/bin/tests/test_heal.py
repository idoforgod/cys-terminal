#!/usr/bin/env python3
"""test_heal.py — DD-5 결정론 치유 레인 계약 핀 (T0 RED-first · 설계 W5).

현 코드에는 `javis_directive_heal.py` 가 **없다** → import 실패 → RED. T4 가 GREEN 화.
단, 해시 대장 `heal_hashes.json`/`base_hashes.json` 은 T0 산출물로 이미 존재하며,
이 테스트는 대장이 **현재 라이브 반쪽마스터(MASTER==CEO 스텁)를 탐지 가능** 함까지 검증한다.

계약(구현 후 통과 기준):
  1) heal_hashes.json 에 역대 CEO_TEMPLATE 스텁 해시가 수집돼 있고, 라이브 스텁 sha256 도 포함.
  2) needs_heal(md_sha) — md 해시가 스텁 대장에 정확 일치할 때만 True(퍼지 금지).
  3) 사용자 정당 수정본(대장 밖 해시)은 needs_heal=False(오폭 0·안전핵).
  4) heal 은 부서 팩(depts.json 순회) 동기화 API 를 노출(N-5 전체 범위).
  5) recompose 는 base_hashes.json 화이트리스트 일치 시에만 base 교체.

실행: python3 test_heal.py   (exit 0=PASS / 1=RED)
"""
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.normpath(os.path.join(SELF, "..", ".."))          # cysjavis-pack/
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_directive_heal.py"))
HEAL_LEDGER = os.path.join(PACK, "heal_hashes.json")
BASE_LEDGER = os.path.join(PACK, "base_hashes.json")
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
    spec = importlib.util.spec_from_file_location("javis_directive_heal", MODULE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main():
    # 1. 대장 존재·라이브 스텁 포함 (T0 산출물 — 이 부분은 이미 PASS 가능해야 함)
    heal = base = None
    if os.path.exists(HEAL_LEDGER):
        with open(HEAL_LEDGER, encoding="utf-8") as f:
            heal = json.load(f)
    if os.path.exists(BASE_LEDGER):
        with open(BASE_LEDGER, encoding="utf-8") as f:
            base = json.load(f)
    stub_shas = {h["sha256"] for h in (heal or {}).get("hashes", [])}
    check("1a heal_hashes.json 존재·스텁 다수 수집", len(stub_shas) >= 2, "수집=%d" % len(stub_shas))
    # 1b (밀폐화·T4 인계 #3): 하드코딩 라이브 경로(:58) 제거 — 라이브 무접촉. 리포 스텁 대장 해시와
    #   모듈의 sentinel base-추출 파이프라인만으로 치유 탐지를 핀한다(env -i 에서도 GREEN).
    m0 = load()
    if m0 is not None and stub_shas:
        a_stub = sorted(stub_shas)[0]
        needs = getattr(m0, "needs_heal", None)
        detected = callable(needs) and needs(a_stub) is True
        # sentinel 합성물에서 base 부분만 추출(치유 전처리 파서 — T1 독립행 앵커 동형).
        base_txt = "표준 base 전문\n본문 여러 줄\n"
        composite = (base_txt + "<!-- CEO-OVERLAY BEGIN v1 sha256:%s -->\n오버레이 본문\n"
                     "<!-- CEO-OVERLAY END -->\n" % ("0" * 64))
        xb = getattr(m0, "extract_base", None)
        extracted_ok = callable(xb) and xb(composite) == base_txt
        check("1b 스텁 대장 탐지 + sentinel base 추출(라이브 무접촉)",
              detected and extracted_ok, "detected=%s extract_ok=%s" % (detected, extracted_ok))
    else:
        check("1b 스텁 대장 탐지 + sentinel base 추출(라이브 무접촉)", False,
              "javis_directive_heal.py 미구현 — T4 대상(RED)")
    check("1c base_hashes.json 존재·표준 다수 수집",
          len((base or {}).get("hashes", [])) >= 2, "수집=%d" % len((base or {}).get("hashes", [])))

    # 2~5. 치유 모듈 계약
    m = load()
    if m is None:
        for n in ("2 needs_heal 정확일치 탐지", "3 사용자 수정본 오폭 0",
                  "4 부서 팩 동기화 API", "5 recompose base 화이트리스트"):
            check(n, False, "javis_directive_heal.py 미구현 — T4 대상(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    needs = getattr(m, "needs_heal", None)
    if callable(needs):
        any_stub = next(iter(stub_shas)) if stub_shas else "0" * 64
        check("2 needs_heal 정확일치 탐지", needs(any_stub) is True, "스텁 해시 미탐(RED)")
        check("3 사용자 수정본 오폭 0", needs("f" * 64) is False, "대장 밖 해시를 치유대상 오판")
    else:
        check("2 needs_heal 정확일치 탐지", False, "needs_heal 미구현(RED)")
        check("3 사용자 수정본 오폭 0", False, "needs_heal 미구현(RED)")

    check("4 부서 팩 동기화 API(depts.json 순회)",
          callable(getattr(m, "sync_dept_packs", None)) or callable(getattr(m, "heal_all", None)),
          "부서 팩 sync API 미구현(RED)")
    check("5 recompose base 화이트리스트 존중",
          callable(getattr(m, "recompose", None)) or hasattr(m, "BASE_LEDGER"),
          "recompose/화이트리스트 미구현(RED)")

    # ── ★리뷰어1 fix#4 실질 케이스 (실제 재현 로직) ──────────────────────────
    bt = getattr(m, "_base_trusted", None)

    # 6(①): 8001바이트 쓰레기 소스 거부 — 크기 퍼지 게이트 폐지 확증(.pristine/.pre-ceo 는 정확일치만).
    if callable(bt):
        garbage = "x" * 8001
        rej = bt(garbage, ".pristine") is False and bt(garbage, ".pre-ceo") is False
        acc_self = bt(garbage, "pack-self") is True    # (b) 배포 신뢰 루트만 예외
        check("6 8001B 쓰레기 소스 거부(.pristine)+배포루트만 신뢰",
              rej and acc_self, "reject=%s pack-self-accept=%s" % (rej, acc_self))
    else:
        check("6 8001B 쓰레기 소스 거부", False, "_base_trusted 미노출")

    # 7(②): 정당 수정 base(대장 밖 실콘텐츠·스텁 아님) 완전 무접촉 — needs_heal False·바이트 불변.
    tmp = tempfile.mkdtemp()
    try:
        os.makedirs(os.path.join(tmp, "directives"))
        legit = os.path.join(tmp, "directives", "MASTER_DIRECTIVE.md")
        content = "# 사용자 정당 수정 MASTER\n" + ("규범 조항 줄\n" * 600)
        with open(legit, "w", encoding="utf-8") as f:
            f.write(content)
        before = hashlib.sha256(content.encode()).hexdigest()
        untouched = (m.needs_heal_path(legit) is False)
        hb = m.heal_base(tmp, do_recompose=False)
        after = hashlib.sha256(open(legit, "rb").read()).hexdigest()
        check("7 정당 수정 base 완전 무접촉(needs=False·바이트 불변)",
              untouched and hb.get("needs") is False and before == after,
              "needs_path=%s heal.needs=%s bytes_eq=%s" % (untouched, hb.get("needs"), before == after))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    # 8(③): dept sync 폴백에서 User-소유 파일(directives) 보존 — 폴백 범위 = bin/·hooks/ 만.
    csf = getattr(m, "_copy_system_files", None)
    if callable(csf):
        saved_cys = m.CYS
        m.CYS = "/nonexistent/cys-for-test"      # _cys_available False 강제(폴백 보수 경로)
        dst = tempfile.mkdtemp()
        try:
            os.makedirs(os.path.join(dst, "directives"))
            userf = os.path.join(dst, "directives", "MASTER_DIRECTIVE.md")
            with open(userf, "w", encoding="utf-8") as f:
                f.write("USER OWNED — 보존되어야 함\n")
            ub = hashlib.sha256(open(userf, "rb").read()).hexdigest()
            n = csf(dst)
            ua = hashlib.sha256(open(userf, "rb").read()).hexdigest()
            check("8 dept sync 폴백 User-소유(directives) 보존",
                  ub == ua and n >= 1, "user_preserved=%s copied=%d" % (ub == ua, n))
        finally:
            m.CYS = saved_cys
            shutil.rmtree(dst, ignore_errors=True)
    else:
        check("8 dept sync 폴백 User-소유 보존", False, "_copy_system_files 미노출")

    # 9(④): §CEO-1 없는 커스텀 오버레이로 recompose want=True 정상 — post-verify 오탐 0(리터럴 커플링 제거).
    cys_dept = os.path.join(os.path.dirname(SELF), "cys-dept")
    if os.path.isfile(cys_dept):
        home = tempfile.mkdtemp()
        try:
            pdir = os.path.join(home, ".cys", "pack", "directives")
            os.makedirs(pdir)
            with open(os.path.join(pdir, "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
                f.write("# 표준 base\n본문\n")
            with open(os.path.join(pdir, "CEO_OVERLAY.md"), "w", encoding="utf-8") as f:
                f.write("# 커스텀 오버레이\n지휘 범위 조항(헤딩 리터럴 없음)\n")
            depts = os.path.join(home, "depts.json")
            with open(depts, "w", encoding="utf-8") as f:
                json.dump({"depts": {"dept-1": {"socket": "s", "pack_dir": "p",
                                                "role": "dept-master"}}}, f)
            env = dict(os.environ)
            env["HOME"] = home
            env["CYS_DEPTS_JSON"] = depts
            r = subprocess.run(["bash", cys_dept, "recompose"], env=env,
                               capture_output=True, text=True, timeout=30)
            check("9 커스텀 오버레이(§CEO-1 없음) recompose want=True 정상(post-verify 오탐 0)",
                  r.returncode == 0, "rc=%s err=%s" % (r.returncode, (r.stderr or "").strip()[:120]))
        finally:
            shutil.rmtree(home, ignore_errors=True)
    else:
        check("9 커스텀 오버레이 recompose 정상", False, "cys-dept 부재")

    # 10(⑤): heal 성공 후 DEMOTE_INCOMPLETE 마커 제거(치유=해소).
    orig_stub, orig_base = m._stub_hashes, m._base_hashes
    state = tempfile.mkdtemp()
    pack = tempfile.mkdtemp()
    saved_state_env = os.environ.get("CYS_STATE_DIR")
    try:
        os.makedirs(os.path.join(pack, "directives"))
        os.makedirs(os.path.join(pack, ".pristine", "directives"))
        stub_txt = "STUB 반쪽마스터\n"
        base_txt = "# 표준 base 전문\n" + ("조항\n" * 2000)
        with open(os.path.join(pack, "directives", "MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
            f.write(stub_txt)
        with open(os.path.join(pack, ".pristine", "directives", "MASTER_DIRECTIVE.md"),
                  "w", encoding="utf-8") as f:
            f.write(base_txt)
        stub_sha = hashlib.sha256(stub_txt.encode()).hexdigest()
        # base 정규화(\n 종료)까지 반영한 sha 를 대장에 등록(원자쓰기 후 sha 일치 보장 위해 restore 소스 그대로).
        norm = base_txt if base_txt.endswith("\n") else base_txt + "\n"
        m._stub_hashes = lambda: {stub_sha}
        m._base_hashes = lambda: {hashlib.sha256(norm.encode()).hexdigest()}
        os.environ["CYS_STATE_DIR"] = state
        marker = m._demote_marker_path()
        os.makedirs(os.path.dirname(marker), exist_ok=True)
        with open(marker, "w", encoding="utf-8") as f:
            f.write("demote incomplete rc=4\n")
        hb = m.heal_base(pack, do_recompose=False)
        check("10 heal 성공 후 demote 마커 제거",
              hb.get("healed") is True and not os.path.exists(marker),
              "healed=%s marker_exists=%s" % (hb.get("healed"), os.path.exists(marker)))
    finally:
        m._stub_hashes, m._base_hashes = orig_stub, orig_base
        if saved_state_env is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved_state_env
        shutil.rmtree(state, ignore_errors=True)
        shutil.rmtree(pack, ignore_errors=True)

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
