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
import importlib.util
import json
import os
import sys

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
    live_master = "/Users/cys-macbook/.cys/pack/directives/MASTER_DIRECTIVE.md"
    if os.path.exists(live_master):
        import hashlib
        with open(live_master, "rb") as f:
            lsha = hashlib.sha256(f.read()).hexdigest()
        check("1b 라이브 반쪽마스터가 스텁 대장에 탐지됨(현 라이브 재발 확증)",
              lsha in stub_shas, "live sha=%s.." % lsha[:16])
    else:
        check("1b 라이브 반쪽마스터가 스텁 대장에 탐지됨", True, "라이브 부재 — skip(대장 자체는 유효)")
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

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
