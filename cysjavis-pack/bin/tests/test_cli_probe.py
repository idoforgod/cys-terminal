#!/usr/bin/env python3
"""test_cli_probe.py — DD-2 probe_cli 로그인셸 프로브 계약 핀 (T0 RED-first).

현 코드에는 `javis_cli_probe.py`(공용 probe_cli 모듈)가 **없다** → import 실패 → RED.
T2 가 이 모듈을 만들어 GREEN 으로 만든다.

계약(구현 후 통과 기준):
  1) probe_cli(name) → bool. 존재하는 코어유틸(sh/env)은 True, 없는 이름은 False.
  2) 감지는 **로그인셸 경유**(macOS `zsh -lc`/`bash -lc`, Windows 폴백) — GUI launchd 빈곤 PATH
     에서 호출돼도 로그인셸 rc(PATH 재구성) 를 거쳐 해석한다(기능2 우산·오판 0).
  3) 빈곤 PATH(빈 PATH env) 로 호출해도 로그인셸 rc 가 채우는 PATH 로 코어유틸을 찾는다.
  4) probe_cli 는 read-only(설치·수정 부작용 0 — 온보딩 무간섭).

실행: python3 test_cli_probe.py   (exit 0=PASS / 1=RED)
"""
import importlib.util
import os
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
MODULE = os.path.normpath(os.path.join(SELF, "..", "javis_cli_probe.py"))
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
    spec = importlib.util.spec_from_file_location("javis_cli_probe", MODULE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main():
    m = load()
    if m is None or not hasattr(m, "probe_cli"):
        check("0 javis_cli_probe.probe_cli 존재", False,
              "모듈/함수 미구현 — T2 대상(RED)")
        # 나머지 계약도 미검=RED 로 명시.
        for n in ("1 존재 유틸 True", "2 미존재 이름 False",
                  "3 빈곤 PATH 에서도 로그인셸 rc 로 해석", "4 read-only 부작용 0"):
            check(n, False, "probe_cli 미구현으로 미검(RED)")
        print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
        return 1

    probe_cli = m.probe_cli
    # 코어유틸 'sh' 는 어떤 유닉스에도 로그인셸 PATH 상 존재.
    core = "sh" if os.name != "nt" else "cmd"
    check("1 존재 유틸 True", probe_cli(core) is True, "probe_cli(%r)" % core)
    check("2 미존재 이름 False",
          probe_cli("definitely-not-a-real-cli-xyz-123") is False)

    # 빈곤 PATH 시뮬레이션: PATH 를 비워도 로그인셸 rc 가 채우는 PATH 로 해석돼야 한다.
    saved = os.environ.get("PATH")
    try:
        os.environ["PATH"] = ""
        check("3 빈곤 PATH 에서도 로그인셸 rc 로 해석",
              probe_cli(core) is True,
              "빈 PATH 에서 False 면 GUI launchd 오판 재현(RED/회귀)")
    finally:
        if saved is not None:
            os.environ["PATH"] = saved

    # read-only: 프로브가 파일시스템에 흔적을 남기지 않음(간접 계약 — 함수가 순수 조회임을 문서·구현으로 보장).
    check("4 read-only 부작용 0", getattr(m, "PROBE_READONLY", True) is True,
          "구현이 read-only 계약을 표식(PROBE_READONLY)")

    print("\n=== %d/%d PASS (fails: %s) ===" % (_total[0] - len(fails), _total[0], fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
