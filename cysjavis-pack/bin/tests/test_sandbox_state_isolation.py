#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_sandbox_state_isolation.py — 샌드박스 데몬이 **본 레인 원장**을 오염시키지 않는다 (0.14.30 W-A · master 봉쇄 지시 2026-09-04).

## 무엇이 일어났나 (실사고 · 2026-09-04 21:34:47)

시험용 데몬 하나가 소켓만 격리한 채(`~/.cys/state-harness/cys.sock`) 떠서, **본 레인의**
`~/.cys/state/delivery-base.epoch.json` 을 자기 인스턴스 표식으로 덮어썼다. 결과:
`javis_mission.py status` 가 **데몬 재기동**으로 오탐해 master 임무 게이트가 닫히고
본부 자율주행이 정지했다. 원장(`delivery-base.jsonl`)에도 시험 좌석 레코드가 섞였다.

## 왜 조용히 일어나는가 — 격리는 **두 변수**를 요구하고 한쪽 누락이 무증상이다

  · `CYS_SOCKET`  — 어느 소켓으로 말하는가(누락하면 라이브 데몬에 붙어 즉시 티가 난다)
  · `CYS_STATE_DIR` — 원장·표식을 **어디에 쓰는가**(누락하면 `default_state_root()` =
    `$HOME/.cys/state` 로 조용히 떨어진다)

게다가 레인 판정(`delivery.rs::socket_is_base`)은 **파일명만** 본다 — basename 이 `cys.sock` 이고
경로에 `cys-dept-` 성분이 없으면 전부 **base 레인**이다. 즉 `/tmp/aaa/cys.sock` 로 띄운
샌드박스 데몬도 레인 키가 `base` 라, `CYS_STATE_DIR` 을 빠뜨리는 순간 본 레인 파일
`delivery-base.*` 를 정확히 겨냥해 덮어쓴다. **소켓 격리만으로는 원장이 격리되지 않는다.**

## 이 검체가 재는 것

  1 **양성**: 두 변수를 모두 격리하면 원장·표식이 **격리 디렉터리에만** 생긴다.
  2 **본 레인 불변**: 위 기동 전후로 실제 `~/.cys/state/delivery-base.*` 의 mtime·sha 가
    **바뀌지 않는다**(읽기만 — 이 검체는 본 레인을 절대 쓰지 않는다).
  3 **음성 대조(결측형)**: `CYS_STATE_DIR` 을 **빼고** 띄우면 오염이 **재현된다**
    (`$HOME/.cys/state/delivery-base.epoch.json` 이 생긴다). ★HOME 을 스크래치로 돌려 재현하므로
    실제 본 레인은 건드리지 않는다 — 재현하되 오염시키지 않는다.
    이 축이 없으면 1·2 의 초록은 "격리가 통했다"가 아니라 "아무것도 안 잰다"일 수 있다.
  4 **레인 판정 핀**: 샌드박스 소켓(`cys.sock`)이 `base` 로 접히는 성질을 소스로 못박는다 —
    이 성질이 바뀌면(레인 분리) 위 위험 자체가 사라지므로 검체도 함께 개정돼야 한다.

출력: PASS/FAIL 행 · 실패 시 exit 1 · 종료 토큰 SANDBOX-STATE-ISOLATION-OK.
cysd 바이너리가 없으면(팩 단독 배포·CI 팩 레인) 1·3 은 SKIP 하고 2·4 만 잰다.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_sandbox_state_isolation.py
"""
import hashlib
import io
import os
import shutil
import subprocess
import sys
import tempfile
import time

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _live_state_dir():
    return os.path.join(os.path.expanduser("~"), ".cys", "state")


def _epoch_fp(d):
    """본 레인 **표식**(epoch)의 (mtime, sha256) — 읽기 전용.

    ★이것이 사고의 급소다: 표식이 덮이면 `javis_mission.py status` 가 **데몬 재기동**으로
      오탐해 master 임무 게이트가 닫힌다(2026-09-04 실사고). 그래서 표식은 **바이트 불변**을
      요구한다 — 살아있는 데몬도 기동 시 1회만 쓰므로 시험 중에는 바뀌지 않는 것이 정상이다."""
    p = os.path.join(d, "delivery-base.epoch.json")
    if not os.path.isfile(p):
        return None
    try:
        with open(p, "rb") as f:
            return (os.stat(p).st_mtime, hashlib.sha256(f.read()).hexdigest())
    except OSError as e:
        return ("unreadable", str(e))


def _jsonl_fp(d):
    """본 레인 **원장**의 (크기, 앞 4KB sha256) — 읽기 전용.

    ★원장은 살아있는 데몬이 **정상적으로 계속 append** 한다(내가 cys send 를 할 때마다 는다).
      그래서 바이트 불변을 요구하면 정상 운영을 적색으로 만든다. 여기서 요구하는 것은
      **잘림·재작성 없음**(크기 비감소 + 앞부분 불변)이다.
    ★정직한 한계: 이 지표는 샌드박스가 **끼워 넣은 append** 를 라이브 append 와 구별하지
      못한다. 그 축은 위 1·1b(격리 디렉터리에만 생성)와 3(결측 시 재현)이 담당한다."""
    p = os.path.join(d, "delivery-base.jsonl")
    if not os.path.isfile(p):
        return None
    try:
        with open(p, "rb") as f:
            head = f.read(4096)
        return (os.path.getsize(p), hashlib.sha256(head).hexdigest())
    except OSError as e:
        return ("unreadable", str(e))


def _find_cysd():
    """cysd 실행 파일. `CYS_TEST_CYSD` 로 명시 지정 가능(러너·다른 워크트리 빌드 대응).
    ★못 찾으면 **SKIP** 이지 PASS 가 아니다 — 미측정을 통과로 접지 않는다."""
    env = os.environ.get("CYS_TEST_CYSD", "").strip()
    if env and os.path.isfile(env) and os.access(env, os.X_OK):
        return env
    for c in (os.path.join(REPO, "target", "debug", "cysd"),
              os.path.join(REPO, "target", "release", "cysd")):
        if os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


def _spawn(cysd, sb, isolate_state):
    """샌드박스 데몬 1대. isolate_state=False 면 **CYS_STATE_DIR 미설정**(결측형 대조)."""
    home = os.path.join(sb, "home")
    os.makedirs(home, exist_ok=True)
    env = dict(os.environ, HOME=home,                   # ★HOME 도 스크래치 — 본 레인 불가침
               CYS_SOCKET=os.path.join(sb, "cys.sock"),
               CYS_PACK_DIR=os.path.join(sb, "pack"),
               CYS_CONFIG_DIR=os.path.join(sb, "config"),
               CYS_PACK_CAPTURES_DIR=os.path.join(sb, "captures"),
               CYS_NO_PERSONAL_HOOK_MERGE="1")
    for k in ("CYS_SURFACE_ID", "CYS_SEAT_TOKEN", "CYS_SURFACE_REF", "CYS_ROLE"):
        env.pop(k, None)
    if isolate_state:
        env["CYS_STATE_DIR"] = os.path.join(sb, "state")
        os.makedirs(env["CYS_STATE_DIR"], exist_ok=True)
    else:
        env.pop("CYS_STATE_DIR", None)
    log = open(os.path.join(sb, "cysd.log"), "wb")
    p = subprocess.Popen([cysd], env=env, stdout=log, stderr=log)
    sock = env["CYS_SOCKET"]
    # ★디버그 빌드 냉시작은 소켓 개방까지 **십수 초** 걸린다(실측 16s — phoenix 잡 보장·
    #   auto-restore 가 선행한다). 창을 짧게 잡으면 "표식 0" 을 격리 성공으로 오독하고,
    #   음성 대조까지 초록이 되어 **검체가 아무것도 재지 못한다**(그리고 하네스는 데몬을
    #   유출한다 — 2026-09-04 실사고).
    deadline = time.time() + 60.0
    while time.time() < deadline and not os.path.exists(sock):
        if p.poll() is not None:                         # 조기 종료 = 미측정(통과 아님)
            return p, home
        time.sleep(0.2)
    time.sleep(1.5)                                      # 기동 시 표식 1회 쓰기 여유
    return p, home


def _stop(p):
    if p and p.poll() is None:
        p.terminate()
        try:
            p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            p.kill()


# ── 2. 본 레인 불변 (전 구간 감시) ────────────────────────────────────────────
live = _live_state_dir()
before_epoch, before_jsonl = _epoch_fp(live), _jsonl_fp(live)
print("본 레인 baseline: epoch=%r · jsonl=%r" % (before_epoch, before_jsonl))

cysd = _find_cysd()
if cysd is None:
    print("SKIP 1 격리 기동(cysd 바이너리 부재 — 팩 단독 배포)")
    print("SKIP 3 음성 대조(cysd 바이너리 부재)")
else:
    # ── 1. 양성: 두 변수 격리 → 원장·표식이 격리 디렉터리에만 ──────────────────
    sb1 = tempfile.mkdtemp(prefix="iso-yes-")
    p1 = None
    try:
        p1, home1 = _spawn(cysd, sb1, isolate_state=True)
        st = os.path.join(sb1, "state")
        made = sorted(f for f in os.listdir(st) if f.startswith("delivery-"))
        check("1 격리 기동: 표식·원장이 CYS_STATE_DIR 에만 생긴다", bool(made), repr(made))
        # HOME 폴백 자리에는 아무것도 없어야 한다
        leak = os.path.join(home1, ".cys", "state")
        leaked = sorted(os.listdir(leak)) if os.path.isdir(leak) else []
        check("1b 격리 기동: HOME 폴백 자리 오염 0", not leaked, repr(leaked))
    finally:
        _stop(p1)
        shutil.rmtree(sb1, ignore_errors=True)

    # ── 3. 음성 대조(결측형): CYS_STATE_DIR 누락 → 오염이 재현돼야 한다 ────────
    #    ★HOME 을 스크래치로 돌렸으므로 재현 대상은 **가짜 본 레인**이다(실제는 불가침).
    sb2 = tempfile.mkdtemp(prefix="iso-no-")
    p2 = None
    try:
        p2, home2 = _spawn(cysd, sb2, isolate_state=False)
        fake_live = os.path.join(home2, ".cys", "state")
        polluted = sorted(f for f in os.listdir(fake_live)
                          if f.startswith("delivery-")) if os.path.isdir(fake_live) else []
        check("3 음성 대조: CYS_STATE_DIR 누락이 '본 레인'을 오염시킨다(검출력 확인)",
              bool(polluted), repr(polluted) or "오염 0 — 이 검체는 아무것도 재지 못한다")
        # ★3b 개정(P0 레인 규칙 수리 반영): 기대값이 뒤집혔다. 수리 전에는 관례 파일명
        #   (`cys.sock`)을 유지한 격리 소켓이 basename 만으로 **base 로 접혀**
        #   `delivery-base.*` 를 정확히 겨냥했다(그것이 실사고의 급소였다). 수리 후에는
        #   부모 디렉터리까지 보므로 **자기 레인**으로 떨어진다.
        #   그래서 이 축이 지키는 것은 "base 를 겨냥하지 않는다"(2층 방어)로 바뀐다 —
        #   "라이브 루트 안에 쓴다"는 여전한 사실이고 그것은 위 3 이 잡는다.
        check("3b 레인 규칙 수리 후: 오염 파일이 base 레인을 겨냥하지 않는다(2층 방어)",
              polluted and not any(f.startswith("delivery-base.") for f in polluted),
              repr(polluted))
    finally:
        _stop(p2)
        shutil.rmtree(sb2, ignore_errors=True)

after_epoch, after_jsonl = _epoch_fp(live), _jsonl_fp(live)
check("2 본 레인 **표식**(epoch) 바이트 불변 — 임무 게이트의 급소",
      before_epoch == after_epoch, "before=%r after=%r" % (before_epoch, after_epoch))
if before_jsonl is None or after_jsonl is None:
    check("2b 본 레인 원장 무잘림·무재작성", before_jsonl == after_jsonl,
          "before=%r after=%r" % (before_jsonl, after_jsonl))
else:
    grew = after_jsonl[0] >= before_jsonl[0]
    same_head = after_jsonl[1] == before_jsonl[1]
    check("2b 본 레인 원장 무잘림·무재작성(라이브 append 는 정상)", grew and same_head,
          "크기 %d→%d · 앞4KB %s" % (before_jsonl[0], after_jsonl[0],
                                     "동일" if same_head else "변경됨"))

# ── 4. 레인 판정 핀 — Rust 판정부가 **현 규칙**을 쓰는가 ─────────────────────
#    ★2026-09-05 수리: 이 절은 `src/bin/cysd/delivery.rs` 한 곳만 보고 있었는데, W-B 의
#      `161d306` 이 레인 판정을 **`src/lane.rs` 단일 소유자로 승격**하면서 그 파일에서 함수가
#      사라졌다. 종전 코드는 미발견 시 `body` 가 **빈 문자열**이 되어 뒤 단언이 '없다'를 참으로
#      돌려주는 자리였다 — 통합 트리에서 세 축이 동시에 죽었고, 그건 결함이 아니라 **핀이 재는
#      자리를 잃은 것**이다(IG-19·H-META-PIN 과 같은 계급). 그래서 셋을 고친다:
#      ⓐ 소유 후보를 순서대로 찾고 **어디서도 못 찾으면 명시적 실패**다(빈 본문 단언 금지).
#      ⓑ 단언을 **현 규칙**(부모 디렉터리가 정확히 `cys`)으로 갱신하고, 그 단언이 **구 구현에서는
#         걸리지 않는다**는 음성 대조를 함께 둔다 — 구 구현에서도 통과하는 단언은 규칙을 재지 않는다.
#      ⓒ 승격 전 트리(그 함수가 아직 delivery.rs 에 있고 구 규칙인 트리)는 **PEND**로 크게 적는다.
#         조용히 통과시키지 않으며, 그 경우에도 '구 규칙이 실재한다'를 단언해 축을 비우지 않는다.
_LANE_OWNERS = (os.path.join("src", "lane.rs"),                       # 161d306 승격 후 소유자
                os.path.join("src", "bin", "cysd", "delivery.rs"))    # 승격 전 소유자
_CUR_RULE = 'it.next() == Some("cys")'      # 부모 디렉터리까지 본다(715ccb9 · W-B Rust 파리티)
_OLD_RULE = 'last == "cys" || last == "cys.sock"'
_OLD_IMPL_SNIPPET = (                        # 음성 대조용 — 승격 전 구현의 판정 꼬리(사본)
    'fn socket_is_base(sock: &str) -> bool {\n'
    '    let norm = sock.replace(\'\\\\\', "/");\n'
    '    let last = norm.rsplit(\'/\').next().unwrap_or("");\n'
    '    last == "cys" || last == "cys.sock"\n}')
_owner, _body = None, ""
if not os.path.isdir(os.path.join(REPO, "src")):
    print("SKIP 4 레인 판정 핀(src 부재 — 팩 단독 배포)")
else:
    for _rel in _LANE_OWNERS:
        _fp = os.path.join(REPO, _rel)
        if not os.path.isfile(_fp):
            continue
        with io.open(_fp, encoding="utf-8", errors="replace") as f:
            _src = f.read()
        _i = _src.find("fn socket_is_base(")
        if _i > 0:
            _owner, _body = _rel, _src[_i:_src.find("\n}\n", _i)]
            break
    # ⓐ 미발견은 조용한 통과가 아니라 **실패**다(승격 경로가 또 바뀌면 여기서 운다).
    check("4 레인 판정 함수 소유자를 찾았다(미발견=적색 · 빈 본문 단언 금지)",
          bool(_owner) and bool(_body), "후보=%r" % (_LANE_OWNERS,))
    # ⓑ 음성 대조 먼저 — 현 규칙 핀이 **구 구현에서는 걸리지 않아야** 한다.
    check("4a′ 음성 대조: 현 규칙 핀이 구 구현에는 걸리지 않는다(핀의 판별력)",
          _CUR_RULE not in _OLD_IMPL_SNIPPET and _OLD_RULE in _OLD_IMPL_SNIPPET)
    if _CUR_RULE in _body:
        check("4b 판정이 **부모 디렉터리까지** 본다(승격 후 현 규칙 · 샌드박스 cys.sock 은 비-base)",
              True, "소유자=%s" % _owner)
    else:
        # ⓒ 승격 전 트리 — 크게 적고, 축을 비우지 않기 위해 구 규칙 실재를 단언한다.
        print("PEND 4b 현 규칙 핀 미발효 — 이 트리는 레인 판정 승격(161d306) 전이다"
              "(소유자=%s · C4 유입 시 자동 발효)" % _owner)
        check("4b(승격 전) 구 규칙이 실재한다 — 축을 비우지 않는다",
              _OLD_RULE in _body, "소유자=%s" % _owner)
    check("4c 부서 레인만 분리된다(cys-dept- 성분 · 승격 전후 공통 규칙)",
          'part.starts_with("cys-dept-")' in _body)

# ── 5. 팩 내장 하네스(javis_phoenix_harness) 격리 — 실사고의 진짜 오염자 ──────────
#    2026-09-04: 오염자는 손으로 만든 샌드박스가 아니라 **팩이 스스로 돌리는 시험 하네스**였다.
#    `_daemon_env()` 가 `CYS_SOCKET` 만 주고 `CYS_STATE_DIR` 을 주지 않아, 레인 키가 base 인
#    하네스 데몬이 본 레인 `delivery-base.*` 를 덮어썼다(`test_seat_revival` 이 그 경로로 돈다).
sys.path.insert(0, BIN)
try:
    import javis_phoenix_harness as HARN
except Exception as e:                      # 스큐·의존 부재는 SKIP(미측정을 통과로 접지 않는다)
    print("SKIP 5 팩 내장 하네스 격리(import 실패: %s)" % e)
    HARN = None

if HARN is not None:
    live_root = os.path.realpath(os.path.join(os.path.expanduser("~"), ".cys", "state"))
    henv = HARN._daemon_env()
    hstate = henv.get("CYS_STATE_DIR", "")
    check("5 하네스가 CYS_STATE_DIR 을 **반드시** 설정한다", bool(hstate), repr(hstate))
    hreal = os.path.realpath(hstate) if hstate else ""
    check("5b 그 경로가 라이브 원장 루트 밖이다",
          bool(hreal) and hreal != live_root and not hreal.startswith(live_root + os.sep),
          "state=%r live=%r" % (hreal, live_root))
    check("5c 하네스 상태가 HARN_DIR 아래에 있다(격리 자리 고정)",
          bool(hreal) and hreal.startswith(os.path.realpath(HARN.HARN_DIR)),
          "%r ⊄ %r" % (hreal, HARN.HARN_DIR))

    # ★음성 대조(결측형): **종전 하네스**(CYS_STATE_DIR 미설정)를 그대로 재현하면 원장이
    #   라이브 루트로 떨어진다는 것을 **경로 계산으로** 증명한다(데몬 미기동 = 재오염 0).
    try:
        import javis_lane as LANE
        prev = os.environ.get("CYS_STATE_DIR")
        os.environ.pop("CYS_STATE_DIR", None)     # 종전 하네스 상태 모사
        try:
            old_ledger = os.path.realpath(LANE.lane_state_path("delivery", HARN.HARN_SOCK))
            old_epoch = os.path.realpath(LANE.lane_state_path("delivery_epoch", HARN.HARN_SOCK))
        finally:
            if prev is None:
                os.environ.pop("CYS_STATE_DIR", None)
            else:
                os.environ["CYS_STATE_DIR"] = prev
        check("5d 음성 대조: 종전 하네스(CYS_STATE_DIR 미설정)는 **라이브 원장**을 겨냥했다",
              old_ledger.startswith(live_root + os.sep) and old_epoch.startswith(live_root + os.sep),
              "ledger=%r epoch=%r" % (old_ledger, old_epoch))
        # ★5e 개정(P0 레인 규칙 수리 반영 · 2026-09-04): **이 축의 기대값이 뒤집혔다.**
        #   수리 전에는 `state-harness/cys.sock` 이 basename 만으로 base 로 접혀 표적이
        #   `delivery-base.epoch.json` 이었고, 그래서 본 레인 표식이 덮여 임무 게이트가 닫혔다.
        #   수리 후에는 부모 디렉터리(`cys`)까지 보므로 **자기 레인**으로 떨어진다 — 즉
        #   CYS_STATE_DIR 를 빠뜨려도 최소한 **base 파일은 더 이상 겨냥하지 않는다**(2층 방어).
        #   ※ 그래도 라이브 원장 **루트 안**에 쓰는 것은 여전하다 — 그 축은 위 5d 가 지킨다.
        check("5e 레인 규칙 수리 후: 격리 소켓이 base 로 접히지 않는다(2층 방어)",
              not os.path.basename(old_epoch).startswith("delivery-base."),
              os.path.basename(old_epoch))
        try:
            sys.path.insert(0, BIN)
            import javis_lane as _L
            check("5e2 python 판정도 같은 답(공유 코퍼스 계약)",
                  _L.socket_is_base(HARN.HARN_SOCK) is False,
                  "socket_is_base(%r)=%r" % (HARN.HARN_SOCK, _L.socket_is_base(HARN.HARN_SOCK)))
        except Exception as _e:
            check("5e2 python 판정도 같은 답(공유 코퍼스 계약)", False, "javis_lane 소비 실패: %s" % _e)
    except Exception as e:
        print("SKIP 5d/5e 음성 대조(javis_lane 미사용: %s)" % e)

    # C5 가드가 원장 경로를 실제로 본다 — 라이브 루트를 겨냥하면 거부해야 한다.
    try:
        import contextlib
        _save_state, _save_sock = HARN.HARN_STATE, HARN.HARN_SOCK
        HARN.HARN_STATE = os.path.join(live_root, "state-probe")
        raised = False
        try:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                HARN.guard_isolation()
        except SystemExit:
            raised = True
        finally:
            HARN.HARN_STATE, HARN.HARN_SOCK = _save_state, _save_sock
        check("5f C5 가드가 **원장 경로**로 라이브를 잡는다(종전엔 소켓 자리만 봤다)", raised,
              "라이브 루트를 겨냥하자 거부(SystemExit)" if raised
              else "guard 가 통과시켰다 — 원장 축이 여전히 사각")
    except Exception as e:
        print("SKIP 5f C5 가드 확장(%s)" % e)

    # 하네스를 import 하는 검체들이 이 격리를 **상속**한다.
    #   ★오라클 주의: "파일에 CYS_STATE_DIR 문자열이 있는가"로 재면 오탐이다 —
    #     `run_bootstrap_health.py` 는 자기 격리 샌드박스에 그것을 정당하게 쓰고,
    #     이 검체 자신도 이름을 언급한다. 재는 것은 **하네스의 격리 상수를 덮어쓰는가** 다
    #     (덮어쓰면 상속이 끊겨 그 검체만 라이브로 샐 수 있다).
    importers, breakers = [], []
    for f in sorted(os.listdir(SELF)):
        if not f.endswith(".py") or f == os.path.basename(__file__):
            continue
        try:
            with io.open(os.path.join(SELF, f), encoding="utf-8", errors="replace") as fh:
                body = fh.read()
        except OSError:
            continue
        if "javis_phoenix_harness" not in body:
            continue
        importers.append(f)
        # 격리 상수 재정의 = 상속 파기 — **단, 가드를 시험하려고 복원과 함께 하는 것은 정당**하다
        #   (`test_phoenix_w3_corruption` 이 라이브를 겨냥해 `guard_isolation()` 이 거부하는지
        #    재고 finally 로 되돌린다 · 데몬은 띄우지 않는다). 그것까지 적색으로 만들면
        #   **가드를 시험하는 행위 자체가 금지**되어 오라클이 목적을 배반한다.
        reassigns = any(tok in body for tok in (".HARN_STATE =", ".HARN_DIR =", ".HARN_SOCK =",
                                                "_daemon_env ="))
        exercises_guard = "guard_isolation" in body and "finally:" in body
        if reassigns and not exercises_guard:
            breakers.append(f)
    check("5g 하네스 소비 검체가 격리를 상속한다(격리 상수 재정의 0)",
          not breakers, "소비=%r · 파기=%r" % (importers, breakers))
    check("5h 하네스 소비 검체가 실재한다(무측정 방지)", bool(importers), repr(importers))

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("SANDBOX-STATE-ISOLATION-OK")
sys.exit(0)
