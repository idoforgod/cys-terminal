#!/usr/bin/env python3
"""★U8 P0-M2(0.14.41): phoenix G2 는 ACK **확인 전용**이고, 재주입 판정 문면을 정직하게 분류한다(데몬·네트워크 불요).

배경(반박 검증 M2 · 09-23 실측): 각성 핑이 Claude 큐에 회색으로 머물면 `cys reinject --check` 가 'ACK 없음' 을
드리프트로 읽고 58KB 디렉티브 전문을 다시 넣었다. phoenix 는 stage_reinject(timeout 6) 뒤 stage_g2_ack(timeout 4)
에서 **같은 명령**을 한 번 더 불러, 핑→전문→핑→전문 순으로 한 좌석에 2회(CEO 3회) 재주입됐고 워커는 부트 5분 뒤
ctx 64% 로 강제 clear 됐다(폭주 ① + 무clear ② 결합).

이 검체가 재는 것:
  A. stage_g2_ack 는 `--ack-only` 를 붙여 부른다(재주입 없는 ACK 확인) — 인자 전량을 대조한다.
  B. G2 의 ACK 판정은 CLI 의 **줄 단위 ACK 문면**(분류기 `ack`)이다 — 종전 `"각성" in stdout` 은 실제 ACK 줄
     ("디렉티브 생존 확인 (ACK 수신)")을 한 번도 인정하지 못했다(항상 degraded · 매 restore 재핑).
  C. 분류기는 새 보류 문면을 성공 증거로 오인하지 않는다: 바쁨(`busy`) · 판정 불가/멱등 소진/ack 전용(`held`).
     둘 다 F-1 인정 종류(ack·injected) 밖이다(주입하지 않은 것을 주입 증거로 세지 않는다).
  D. 구 cys(플래그 미지원 → clap rc 2)는 ACK 아님(degraded)으로 접히고 재주입은 일어나지 않는다(버전 스큐 안전).
  E. 소스 핀: CLI 의 안정 문면이 실제로 cys.rs 에 있다(설치 팩에는 Rust 소스가 없을 수 있다 → SKIP).

실행: python3 cysjavis-pack/bin/tests/test_phoenix_g2_ack_only.py (0=전건 PASS)
"""
import importlib.util, os, sys
from types import SimpleNamespace

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
PH = os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py"))
spec = importlib.util.spec_from_file_location("javis_phoenix", PH)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

_results = []
def check(name, cond):
    _results.append(cond); print(("PASS " if cond else "FAIL ") + name)


ACK = "디렉티브 생존 확인 (ACK 수신) — 재주입 불필요"
BUSY = ("재주입 보류(핑 전달됨·대상 바쁨 · queued_in_agent) — surface:7 (worker-1) · "
        "에이전트 큐에 대기 중인 핑을 '미배달'로 읽지 않는다(폭주 차단)")
HELD_UNKNOWN = "재주입 보류(핑 배달 판정 불가: 세션 기록에 핑 흔적 없음) — surface:7 (worker-1)"
HELD_ONCE = "재주입 보류(이 세션 check 재주입 1회 소진 · 좌석당 멱등) — surface:7 (worker-1)"
ACK_ONLY_MISS = "재주입 생략(ack-only · ACK 미수신: 핑 전달됨·대상 바쁨) — surface:7 (worker-1)"
INJECTED = "reinjected 58588 bytes → surface:7 (worker-1)"


def run_g2(returncode, stdout, stderr=""):
    calls = []
    orig = (m.cys, m._surface_agent_present)
    def fake_cys(*args, socket=None, timeout=None):
        calls.append(args)
        return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)
    try:
        m.cys = fake_cys
        m._surface_agent_present = lambda socket, surface: True
        ok, ev = m.stage_g2_ack("sock", "worker-1", "surface:7", False)
    finally:
        m.cys, m._surface_agent_present = orig
    return ok, ev, calls


def main():
    _results.clear()
    # A: 인자 전량 — ACK 전용 플래그가 붙고 역할·좌석·짧은 창은 종전 그대로다.
    ok, ev, calls = run_g2(0, ACK)
    check("g2 calls reinject exactly once", len(calls) == 1)
    check("g2 passes --ack-only", calls and calls[0] == ("reinject", "--check", "--role", "worker-1",
                                                         "--surface", "surface:7", "--timeout", "4", "--ack-only"))
    # B: 실제 ACK 줄은 ACK 다(종전 판독은 이것을 degraded 로 읽었다).
    check("g2 acks on the CLI ACK line", ok is True and "ack=True" in ev)
    # B': ACK 아닌 결과는 전부 degraded — 재주입 줄이 와도(구 cys 가 플래그를 무시한 가상) ACK 가 아니다.
    for name, rc, out, err in (
        ("busy", 0, BUSY, "[reinject] ACK 없음 (4s) — 핑 배달 판정(세션 기록)"),
        ("ack-only miss", 0, ACK_ONLY_MISS, ""),
        ("injected", 0, INJECTED, "[reinject] ACK 없음 (4s)"),
        ("old cys rejects flag", 2, "", "error: unexpected argument '--ack-only' found"),
        ("contradiction", 0, ACK, "[reinject] ACK 없음 (4s)"),
        ("empty", 0, "", ""),
    ):
        ok, ev, _ = run_g2(rc, out, err)
        check("g2 degraded on " + name, ok is False and "ack=False" in ev)
    # C: 분류기 — 새 보류 문면은 성공 증거가 아니다.
    table = (
        ("ack", 0, ACK, "", "ack"),
        ("busy", 0, BUSY, "[reinject] ACK 없음 (6s) — 핑 배달 판정(세션 기록)", "busy"),
        ("held unknown", 0, HELD_UNKNOWN, "[reinject] ACK 없음 (6s)", "held"),
        ("held once", 0, HELD_ONCE, "[reinject] ACK 없음 (6s)", "held"),
        ("ack-only miss", 0, ACK_ONLY_MISS, "", "held"),
        ("injected", 0, INJECTED, "[reinject] ACK 없음 (6s)", "injected"),
        ("rc fail beats busy", 1, BUSY, "", "fail"),
        ("queued beats busy", 0, BUSY, "[inject] 사람 입력 감지 — (--queued 1회 전환)", "queued"),
        # 줄 머리 앵커: 다른 줄 **안**의 문면은 보류 종류가 아니다.
        ("mid-line busy is not busy", 0, "note: 재주입 보류(핑 전달됨·대상 바쁨", "", "unknown"),
    )
    for name, rc, out, err, want in table:
        got = m.classify_reinject_result(rc, out, err)
        check("classify %s -> %s (got %s)" % (name, want, got), got == want)
    for kind in ("busy", "held"):
        check("F-1 does not accept " + kind, kind not in m.F1_ACCEPTED_REINJECT_KINDS)
    check("kind regex reads busy", m._reinject_kind("reinject rc=0 kind=busy " + BUSY) == "busy")
    check("kind regex reads held", m._reinject_kind("reinject rc=0 kind=held " + HELD_ONCE) == "held")

    # E: 소스 핀(설치 팩에는 Rust 소스가 없을 수 있다).
    repo = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
    try:
        with open(os.path.join(repo, "src", "bin", "cys.rs"), encoding="utf-8") as f:
            source = f.read()
    except FileNotFoundError:
        print("SKIP source pin src/bin/cys.rs (repo source absent)")
    else:
        for literal in ("재주입 보류(핑 전달됨·대상 바쁨 · queued_in_agent)", "재주입 보류(핑 배달 판정 불가: ",
                        "재주입 보류(이 세션 check 재주입 1회 소진 · 좌석당 멱등)", "재주입 생략(ack-only · ACK 미수신: ",
                        "디렉티브 생존 확인 (ACK 수신)", "[reinject] ACK 없음"):
            check("source pin " + literal, literal in source)

    npass = sum(1 for c in _results if c)
    print("\n=== %d/%d PASS ===" % (npass, len(_results)))
    return 0 if npass == len(_results) else 1


if __name__ == "__main__":
    sys.exit(main())
