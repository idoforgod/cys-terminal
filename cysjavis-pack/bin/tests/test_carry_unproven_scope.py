#!/usr/bin/env python3
"""★triage R1-WP1-HF(claude major): `carry-unproven` 처방의 **적용 범위** 계약.

실행: python3 cysjavis-pack/bin/tests/test_carry_unproven_scope.py (0=전건 PASS)

【무엇을 재는가】 `carry-unproven` 은 **좌석 1개**의 사실이다(그 좌석 화면에 입력창 양성 증거가
없다). 그런데 처방 ⓑ 는 `CYS_BOOT_GATES=0 cys boot` 을 지목한다:
  · `CYS_BOOT_GATES` 는 **마스터 롤백 스위치**다(src/lib.rs `ENV_BOOT_GATES` doc — "이 캠페인이
    추가한 판정 축이 **전부 동시에** 종전 동작으로 복귀").
  · `cys boot` 은 **로스터 전체**를 부트한다(역할 필터 인자가 없다 — src/bin/cys.rs `Command::Boot`
    는 `--cwd`·`--json` 뿐).
  · 종전 판정에서는 첫기동 관문 화면 **6종 전부**가 ready 다(src/readiness.rs 검체
    `legacy_v1_reproduces_the_defect_on_every_gate_screen`) — 그 부트에서 **다른** 좌석이 진짜
    폴더신뢰·면책 관문에 앉아 있으면 디렉티브 + Return 이 나가고 기본 포커스는 `No, exit` 이다.
같은 문단 ⓐ 가 "관문이면 사람이 통과시킨다(기본 포커스는 No, exit)" 라고 경고해 놓고 ⓑ 가 그
경고를 로스터 전체에서 끄는 손잡이를 권한다 — 두 문장이 서로를 부정한다.

【계약】 처방이 마스터 스위치를 지목한다면, **같은 문장 안에서** 그 범위가 좌석 1개가 아니라
그 부트의 **모든 좌석**이라는 사실과 그 대가(다른 좌석의 관문 거부가 함께 꺼진다)를 말해야 한다.
말하지 못하면 좌석 범위로 좁힌 손잡이를 주어야 한다(범위 축소가 옳은 방향이다).

【수렴 R2 · 두 채널】 이 계약은 **처방이 나가는 채널마다** 성립해야 한다 — python
`javis_bootstrap._GATE_REASON_PRESCRIPTION[carry-unproven]`(부트 요약을 읽는 쪽)과 Rust
`src/bin/cys.rs::CARRY_UNPROVEN_HINT`(`cys boot --json` 의 `hint` 를 읽는 쪽·CLI 안내). R1 판은
python 사전만 쟀고, 헬스체크 H-BOOT-GATE-78 은 Rust 쪽에서 `"CYS_BOOT_GATES=0 cys boot" in rs`
**부분문자열 존재**만 본다(범위·대가 문안은 재지 않는다 — run_bootstrap_health.py:12617~12620).
그래서 Rust 문안이 다음 편집에서 조용히 A-M3 이전으로 되돌아가도 두 채널 다 초록이었다.
지금은 **같은 토큰 집합**을 두 채널에 적용한다(파리티의 실체는 이 검체다).

【배포 팩에서의 거동】 이 파일은 `cysjavis-pack/bin/tests/` 안에 있어 팩과 함께 배송된다. 설치본에는
`src/bin/cys.rs` 가 없으므로 Rust 축은 **SKIP**(NOTE 로 적는다)이고 python 계약 2건은 그대로 잰다 —
계약 위반이 아닌 **경로 부재**로 적색을 내지 않는다(하네스 관용구 = run_bootstrap_health.py 의
`if os.path.isfile(lib)` → "…(python 측만 — 배포 팩)").
회귀 방향 확인용으로 `CYS_REPO_CYS_RS=<경로>` 가 Rust 원본 위치를 덮어쓴다(probe 전용 · 미설정이
정상 경로다).
"""
import importlib.util
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BS = os.path.normpath(os.path.join(HERE, "..", "javis_bootstrap.py"))
spec = importlib.util.spec_from_file_location("javis_bootstrap_scope", BS)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

_results = []
_notes = []


def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (("  — " + detail) if detail and not cond else ""))


def note(msg):
    """계약을 재지 못한 사실을 **적색이 아니라 기록**으로 남긴다(전제 부재 ≠ 계약 위반)."""
    _notes.append(msg)
    print("SKIP  " + msg)


# 범위·대가 어휘 — 두 채널에 **같은 집합**을 적용한다(파리티의 실체).
SCOPE_TOKENS = ("모든 좌석", "로스터 전체", "전 좌석", "다른 좌석", "그 부트의 모든")
COST_TOKENS = ("관문 거부", "관문·모달", "관문이 열린다", "No, exit", "좌석 사망", "함께 꺼진다")
SWITCH = "CYS_BOOT_GATES=0"


def check_prescription(channel, line):
    """한 채널의 처방 문안이 범위와 대가를 말하는가(마스터 스위치를 지목할 때만 의무)."""
    if SWITCH not in line:
        note("%s: 처방이 마스터 스위치를 더는 지목하지 않는다 — 범위 계약은 자동 충족" % channel)
        return
    said_scope = [t for t in SCOPE_TOKENS if t in line]
    check("[%s] 처방이 마스터 스위치의 **범위**(그 부트의 모든 좌석)를 밝힌다" % channel,
          bool(said_scope),
          "처방에 범위 어휘가 없다(찾은 것: %s) — 좌석 1개 문제에 로스터 전체의 관문·모달 거부를 "
          "끄는 손잡이를 조건 없이 권한다: %r" % (said_scope, line))
    # ⓐ 절의 `No, exit` 은 **다른 문장**의 경고다 — ⓑ 절(마스터 스위치 문장) 안에서 대가를 말해야 한다.
    tail = line[line.find(SWITCH):]
    said_cost = [t for t in COST_TOKENS if t in tail]
    check("[%s] 처방의 마스터 스위치 문장이 **대가**(다른 좌석의 관문 거부가 함께 꺼진다)를 말한다"
          % channel,
          bool(said_cost),
          "스위치 문장 이후 문면에 대가 어휘가 없다: %r" % tail)


def rust_string_literal(src, decl):
    """`const <NAME>: &str = "..."` 의 리터럴을 뽑는다(이스케이프·행이음 인식). 없으면 None."""
    i = src.find(decl)
    if i < 0:
        return None
    j = src.find('"', i + len(decl))
    if j < 0:
        return None
    out, k = [], j + 1
    while k < len(src):
        c = src[k]
        if c == '\\':
            nxt = src[k + 1] if k + 1 < len(src) else ""
            if nxt == "\n":  # Rust 행이음 — 개행과 뒤따르는 들여쓰기를 먹는다
                k += 2
                while k < len(src) and src[k] in " \t":
                    k += 1
                continue
            out.append(nxt)
            k += 2
            continue
        if c == '"':
            return "".join(out)
        out.append(c)
        k += 1
    return None


def main():
    _results.clear()
    del _notes[:]
    line = m._GATE_REASON_PRESCRIPTION[m.GATE_REASON_CARRY_UNPROVEN]

    # 전제 ① — 처방이 실제로 마스터 스위치를 지목한다. 지목하지 않게 됐다면 계약은 자동 충족이고
    #   `check_prescription` 이 SKIP 으로 접는다(항진 PASS 를 세지 않는다 — 재지 않은 것은 초록이 아니다).
    print("NOTE  python 처방이 마스터 스위치를 지목한다=%s" % (SWITCH in line))

    # ── 채널 1: python 사전(부트 요약 소비자) ──────────────────────────────────
    check_prescription("python", line)

    # ── 채널 2: Rust `CARRY_UNPROVEN_HINT`(`cys boot --json` hint · CLI 안내) ──
    # ★(수렴 R2 · 리뷰어 minor) 리포 체크아웃에서만 잴 수 있는 축이다. 배포 팩에는 원본이 없으므로
    #   **전제 부재는 SKIP** 이다(하드 실패 금지 — 계약 위반과 경로 부재를 구별한다).
    cys_rs = os.environ.get("CYS_REPO_CYS_RS") or os.path.normpath(
        os.path.join(HERE, "..", "..", "..", "src", "bin", "cys.rs"))
    if not os.path.isfile(cys_rs):
        note("Rust 채널 축(범위·대가 파리티 · `cys boot` 인자)을 재지 못했다(python 측만 — "
             "배포 팩): %s 없음" % cys_rs)
    else:
        with open(cys_rs, encoding="utf-8") as f:
            src = f.read()
        # 전제 ② — `cys boot` 에 좌석·역할 한정 인자가 없다(= 범위가 로스터 전체다).
        i = src.find("    Boot {")
        boot_decl = src[i:src.find("\n    },", i)] if i >= 0 else ""
        check("전제: `cys boot` 에 좌석/역할 한정 인자가 없다(범위=로스터 전체)",
              boot_decl != "" and "--role" not in boot_decl and "role:" not in boot_decl,
              "Boot 선언: %r" % boot_decl[:200])
        rs_hint = rust_string_literal(src, 'const CARRY_UNPROVEN_HINT: &str =')
        if rs_hint is None:
            check("Rust 처방 상수 `CARRY_UNPROVEN_HINT` 가 있다", False,
                  "상수를 찾지 못했다 — Rust 채널의 처방이 사라졌거나 이름이 갈렸다(파리티 붕괴)")
        else:
            check_prescription("rust", rs_hint)

    ok = all(_results)
    print("\n%d/%d PASS%s" % (sum(1 for r in _results if r), len(_results),
                              ("  · SKIP %d" % len(_notes)) if _notes else ""))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
