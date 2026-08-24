#!/usr/bin/env python3
"""javis_lane.py — **레인(lane) 경로 규약의 단일 소유 모듈** (U-24 이관 1단).

`javis_bootstrap.py` 안에 살던 레인 규약 4종을 **먼저 여기로 추출**하고, bootstrap 은
재수출만 한다. 기능 이관(③claim·싱글플라이트 → 데몬 감독자)이 bootstrap 본문을 얇게
만드는 동안, **경로 규약이 그 리팩터에 딸려 흔들리지 않게** 축을 먼저 분리하는 것이
이 파일의 존재 이유 전부다.

이관 대상(= 이 파일이 소유하는 계약):
  · `socket_is_base(sock)`      — 소켓 경로 → base 레인 여부(§4.1 소켓 격리)
  · `lane_key(sock=None)`       — 레인 키('base' 또는 새니타이즈 경로)
  · `LANE_STATE_KINDS`          — 레인 스코프 상태 파일 종류 표
  · `lane_state_path(kind, …)`  — 위 표 + 레인 키 → 실제 경로

★소비자 6곳(하나라도 갈리면 조용히 오작동한다 — 그래서 사본을 금지한다):
  ① `javis_mission.py`         : `javis_bootstrap.lane_state_path("mission"|"delivery"|…)`
  ② 훅 `hooks/role-bootstrap.sh`: `javis_bootstrap.py lane-path boot_last`
  ③ `directives/MASTER_DIRECTIVE.md` §0 · ④ `directives/CEO_TEMPLATE.md`(사본 2벌)
  ⑤ `javis_preflight.py` CONTENT_PINS `("lane-path", …)`
  ⑥ Rust `src/bin/cysd/delivery.rs` `lane_key()` — **교차언어 미러**(파리티 테스트 보유)

★불변식 2개(금지 방향 ① — 절대 완화 금지):
  ⓐ **base 마커 경로는 레인화하지 않는다.** `cys-dept` 의 CEO 승격 게이트가 그 파일의
     '존재'로 열린다 — 부서 마커를 base 경로에 쓰면 게이트가 오개방된다.
  ⓑ base 레인 경로는 **역사적 경로 그대로**다(§0 산문·GUI·테스트 호환·회귀 0).
     접미(`-<lane>`)는 비-base 레인에만 붙는다. 단 `skip`·`lock`·`delivery*` 는
     역사적 무접미 경로가 없어 **항상** 접미가 붙는다(base 예외 없음).

★출하 전제(이 파일이 실제로 배포되는 조건 — 놓치면 조용히 무력화된다):
  `build.rs` 는 `git ls-files cysjavis-pack`(**추적 파일 전용**)로 팩을 임베드한다.
  이 파일이 git 에 추적되지 않으면 **빌드된 바이너리의 팩에 존재하지 않는다** —
  개발 트리에서는 초록인데 설치본에서만 레거시 폴백으로 접히는, 이 제품이 반복해서
  낸 바로 그 사고 형태다. 검체 `H-PACK-TRACK-1` 이 이 조건을 기계 대조한다.

stdlib 만 사용. import 부작용 0(파일 생성·env 변경·프로세스 스폰 전무).
"""
import hashlib
import os
import sys

HOME = os.path.expanduser("~")
CYS_DIR = os.path.join(HOME, ".cys")


def state_dir():
    """상태 파일 루트 — `CYS_STATE_DIR` 우선, 없으면 역사적 기본값 `~/.cys/state`.

    ★env 우선이 계약이다(2026-08-01 T1 봉합): 격리 실행(self-test·테스트 하네스)이
      `CYS_STATE_DIR` 로 밀폐를 걸었는데 한쪽만 env 를 무시하면 **격리해서 돌린 것이
      실 HOME 을 읽는다**. `javis_bootstrap.state_dir` 과 값이 같아야 하며 그 동치는
      검체 `H-LANE-1` 이 매트릭스로 대조한다.
    """
    return os.environ.get("CYS_STATE_DIR") or os.path.join(CYS_DIR, "state")


def socket_is_base(sock):
    """순수 판정: 소켓 경로 문자열 → base 여부(§4.1 소켓 격리). CYS_SOCKET 미설정('')=base.
    ★보수성(아키텍트 성찰): base = (미설정) 또는 (basename이 cys/cys.sock **AND** cys-dept- 성분
    없음). 커스텀 소켓(/tmp/whatever.sock)은 구코드처럼 **비-base·비-dept**다 — base 마커 무접촉·
    issue-ticket 불허·티켓 게이트 비적용(구동작 보존). "cys-dept- 성분 없으면 전부 base"는 미지
    소켓에 base 특권(마커 write·티켓 발급)을 주던 과관용이었다.
    ★경로 기반 dept 판정(basename 아님): 부서 소켓 ~/.local/state/cys-dept-<name>/cys.sock 은
    basename이 본부와 동일한 'cys.sock'이라 basename 단독 판정이 부서를 base로 오판했다
    (마커 오염·ceo_promote 오개방) — cys-dept- 성분이 있으면 무조건 비-base.
    Windows named pipe(백슬래시.백슬래시 pipe 형식)는 성분 분해가 부적합하므로 기존 basename 동작을 보존한다."""
    sock = (sock or "").strip()
    if not sock:
        return True
    norm = sock.replace("\\", "/")
    if sock.startswith("\\\\") or norm.lower().startswith("//./pipe/"):  # win named pipe — 기존 동작 보존
        return os.path.basename(norm) in ("cys", "cys.sock")
    for part in norm.split("/"):
        if part.startswith("cys-dept-"):
            return False
    return os.path.basename(norm) in ("cys", "cys.sock")


def sanitize_sock_key(sock):
    """소켓 전체 경로 → 파일명 안전 락 키(레인마다 유일). 부서 소켓은 basename(cys.sock)이 동일해
    basename 키를 쓰면 모든 레인이 같은 락 파일을 공유했다 — 전체 경로 새니타이즈로 레인 유일화.
    경로 구분자(os.sep·'/'·'\\')·':'를 '_'로 치환. 파일명 길이 상한(255) 여유 — 과길면 앞부분+경로
    해시로 유일성 보존(절단만 하면 서로 다른 긴 경로가 같은 키로 충돌)."""
    raw = (sock or "").strip() or "base"
    for ch in (os.sep, "/", "\\", ":"):
        raw = raw.replace(ch, "_")
    raw = raw.strip("_") or "base"
    if len(raw) > 160:
        raw = raw[:120] + "-" + hashlib.sha1(raw.encode("utf-8")).hexdigest()[:16]
    return raw


def singleflight_key(sock):
    """순수 판정: 소켓 → 싱글플라이트 락 키(R1-LOW-4). base 레인은 env 미설정·base 경로 명시
    어느 쪽이든 단일 'base' 키로 정규화한다 — 같은 base 데몬에 서로 다른 락을 주던 선재결함 교정.
    비-base(부서·커스텀)는 전체 경로 새니타이즈로 레인마다 유일.

    ★U-24: 싱글플라이트 **집행**은 데몬 감독자로 이관되지만 **키 규약**은 여기 남는다 —
      exit 11 과 `boot-skip-<lane>.json` 이 이 키로 이름 붙기 때문이다(어휘 보존)."""
    return "base" if socket_is_base(sock) else sanitize_sock_key(sock)


def lane_key(sock=None):
    """이 부트가 속한 **레인 키** — 'base' 또는 소켓 경로 새니타이즈 값(레인마다 유일).
    락 키와 동일 규약을 쓴다(`singleflight_key`) — 락은 레인별인데 상태 파일은 공유였던
    비대칭(G15·R3)을 없애려면 두 네임스페이스가 **같은 키 함수**를 써야 한다.
    ★Rust 미러: `src/bin/cysd/delivery.rs::lane_key` — 규약이 갈리면 배달 원장이 **조용히**
      무력화된다(양쪽에 교차 기대값 테스트 보유)."""
    return singleflight_key(os.environ.get("CYS_SOCKET", "") if sock is None else sock)


# ★base_dir 자리의 `_STATE` 는 **지연 해소 표식**이다(리터럴 경로 아님) — import 시점에 얼린
#   문자열을 넣으면 `CYS_STATE_DIR` 을 나중에 바꾼 격리 실행(self-test·테스트 하네스)에서
#   경로가 실 HOME 으로 새어 나간다(2026-08-01 T1 밀폐 붕괴의 기제).
_STATE = "\0state_dir"

LANE_STATE_KINDS = {
    "marker": (CYS_DIR, ".master-bootstrapped", ""),
    "boot_last": (_STATE, "boot-last", ".json"),
    # ★U-24: 싱글플라이트 집행이 감독자로 이관돼도 이 두 종류는 **보존**한다 —
    #   exit 11 의 증거 파일(`boot-skip-<lane>.json`)과 락 파일 이름이 곧 어휘다.
    "skip": (_STATE, "boot-skip", ".json"),
    "lock": (_STATE, "bootstrap", ".lock"),
    # ★T1(2026-08-01 윈도우 실사고): 임무 대장 — '이 세션에 오너가 임무를 지정했는가'의 결정론
    #   상태. 소유자는 `javis_mission.py`(판정)이고 **경로 규약만** 여기서 발급한다(사본 금지).
    #   레인별인 이유: 부서 레인의 오너 임무가 base master 의 자율 착수 권한이 되면 안 된다.
    "mission": (_STATE, "mission", ".json"),
    # ★R1(2026-08-01 배달 원장): **데몬(cysd)이 쓰고 훅이 읽는** out-of-band 채널.
    #   - delivery       : pane stdin 주입 직전의 append-only 원장(JSONL) — '이 문장은 기계가
    #                      밀어 넣은 것'의 증거. 문자열 라벨(발신자가 고를 수 있는 값)에
    #                      의존하던 기계/오너 판별을 대체한다.
    #   - delivery_epoch : 데몬 인스턴스 표식 — 임무의 **세션 결박**(과거 임무 무기한 유효 차단).
    #   ★둘 다 **항상 레인 접미**다(base 도 `delivery-base.jsonl`). 역사적 무접미 경로가 없어
    #     base 예외를 둘 이유가 없고, 접미가 항상 있으면 파일명만으로 레인이 결정론이다.
    #   ★생산자는 Rust `src/bin/cysd/delivery.rs`(ledger_path/epoch_path) — 경로 규약이 갈리면
    #     원장이 **조용히** 무력화되므로 양쪽에 교차 테스트를 둔다.
    "delivery": (_STATE, "delivery", ".jsonl"),
    "delivery_epoch": (_STATE, "delivery", ".epoch.json"),
}

# 항상 레인 접미가 붙는 종류(base 예외 없음) — 위 주석의 규약을 코드로 고정한다.
ALWAYS_LANE_SUFFIXED = ("skip", "lock", "delivery", "delivery_epoch")


def lane_state_path(kind, sock=None):
    """레인 스코프 상태 파일 경로.
    kind ∈ marker|boot_last|skip|lock|mission|delivery|delivery_epoch.
    base 레인: 역사적 경로(마커=`~/.cys/.master-bootstrapped` · `boot-last.json`).
    비-base 레인: `-<lane>` 접미(`.master-bootstrapped-<lane>` · `boot-last-<lane>.json`).
    ※ skip·lock·delivery* 는 **항상** 레인별이다 — 규약을 이 함수 하나로 모은다(사본 금지)."""
    try:
        base_dir, stem, ext = LANE_STATE_KINDS[kind]
    except KeyError:
        raise ValueError("미지 레인 상태 종류: %r" % kind)
    if base_dir == _STATE:                       # 지연 해소(위 주석) — 호출 시점의 env 를 본다
        base_dir = state_dir()
    key = lane_key(sock)
    if kind in ALWAYS_LANE_SUFFIXED:
        return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))   # 항상 레인별(구 동작 보존)
    if key == "base":
        return os.path.join(base_dir, stem + ext)
    return os.path.join(base_dir, "%s-%s%s" % (stem, key, ext))


# ── 하위호환 별칭(bootstrap 의 구 사설 이름 그대로) ────────────────────────────
#   bootstrap 이 이 이름들로 재수출하고 그 자신의 `--self-test` 가 사설 이름으로 단언한다.
#   **별칭은 같은 객체다**(사본 아님) — 검체 `H-LANE-1` 이 `is` 동일성으로 못박는다.
_socket_is_base = socket_is_base
_sanitize_sock_key = sanitize_sock_key
_singleflight_key = singleflight_key
_LANE_STATE_KINDS = LANE_STATE_KINDS
_ALWAYS_LANE_SUFFIXED = ALWAYS_LANE_SUFFIXED


def _self_test():
    """밀폐 자기검증 — 부작용 0(파일 write 없음·실 HOME 무접촉)."""
    n = 0
    # ⓐ base 판정(구 bootstrap --self-test 의 배터리를 그대로 이관 — 조건 완화 0)
    assert socket_is_base("") is True, "unset=base"
    assert socket_is_base("/Users/x/.local/state/cys/cys.sock") is True, "unix base"
    assert socket_is_base("/Users/x/.local/state/cys-dept-dept-1/cys.sock") is False, "unix dept"
    assert socket_is_base("/Users/x/.local/state/cys-dept-ceo/cys.sock") is False, "unix dept(ceo)"
    assert socket_is_base("\\\\.\\pipe\\cys") is True, "win base pipe(basename 보존)"
    assert socket_is_base("\\\\.\\pipe\\cys-dept-foo") is False, "win dept pipe"
    assert socket_is_base("/tmp/whatever.sock") is False, "커스텀 소켓은 비-base(과관용 금지)"
    assert socket_is_base("/Users/x/.local/state/cys/cys") is True, "basename cys(무확장)도 base"
    assert socket_is_base("/s/cys-dept-/cys.sock") is False, "빈 부서명이 base로 오판"
    n += 9
    # ⓑ 레인 키 — base 정규화·부서 유일·긴 경로 해시
    assert lane_key("") == "base" and lane_key("/x/.local/state/cys/cys.sock") == "base", "base 키"
    assert lane_key("/x/.local/state/cys-dept-d1/cys.sock") != "base", "부서 키 미분리"
    assert lane_key("/x/cys-dept-a/cys.sock") != lane_key("/x/cys-dept-b/cys.sock"), "부서 키 충돌"
    _long = "/" + ("z" * 400) + "/cys-dept-q/cys.sock"
    assert len(lane_key(_long)) <= 160, "긴 경로 키 상한 위반"
    assert lane_key(_long) != lane_key(_long.replace("z" * 400, "y" * 400)), "긴 경로 키 충돌"
    n += 5
    # ⓒ 경로 규약 — base 무접미 · 부서 접미 · 항상-접미 4종 · 미지 종류 거부
    _b, _d = "/x/.local/state/cys/cys.sock", "/x/.local/state/cys-dept-d1/cys.sock"
    assert lane_state_path("marker", _b) == os.path.join(CYS_DIR, ".master-bootstrapped"), \
        "base 마커 경로 변경(CEO 승격 게이트 SOT — 금지 방향 ①)"
    assert lane_state_path("boot_last", _b) == os.path.join(state_dir(), "boot-last.json"), \
        "base boot-last 경로 변경"
    assert lane_state_path("marker", _d) != lane_state_path("marker", _b), "부서 마커 미분리"
    assert lane_state_path("skip", _b).endswith("boot-skip-base.json"), \
        "exit 11 증거 파일명 변경(어휘 보존 위반)"
    assert lane_state_path("lock", _b).endswith("bootstrap-base.lock"), "base 락 경로 변경"
    assert lane_state_path("delivery", _b).endswith("delivery-base.jsonl"), "배달 원장 경로 변경"
    assert lane_state_path("delivery_epoch", _b).endswith("delivery-base.epoch.json"), \
        "배달 epoch 경로 변경"
    assert os.path.dirname(lane_state_path("delivery", _b)) == state_dir(), "배달 원장 루트 변경"
    try:
        lane_state_path("nope")
    except ValueError:
        pass
    else:
        raise AssertionError("미지 레인 상태 종류가 거부되지 않았다")
    n += 9
    # ⓓ 별칭 동일성 — 사본이 아니라 같은 객체여야 한다
    assert _socket_is_base is socket_is_base and _LANE_STATE_KINDS is LANE_STATE_KINDS, \
        "하위호환 별칭이 사본이다(드리프트 위험)"
    n += 1
    # ⓔ `CYS_STATE_DIR` 지연 해소 — import 시점에 얼면 밀폐가 깨진다
    _saved = os.environ.get("CYS_STATE_DIR")
    try:
        os.environ["CYS_STATE_DIR"] = os.path.join(os.sep, "tmp", "lane-selftest-state")
        assert lane_state_path("boot_last", _b).startswith(os.environ["CYS_STATE_DIR"]), \
            "CYS_STATE_DIR 이 지연 해소되지 않는다(밀폐 붕괴 — T1 기제)"
        assert lane_state_path("marker", _b) == os.path.join(CYS_DIR, ".master-bootstrapped"), \
            "마커는 CYS_STATE_DIR 에 걸리지 않는다(불변식 ⓐ)"
    finally:
        if _saved is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = _saved
    n += 2
    return n


def main(argv):
    if "--self-test" in argv:
        n = _self_test()
        print("javis_lane self-test OK (%d 단언 — base 판정·레인 키·경로 규약·별칭 동일성·"
              "CYS_STATE_DIR 지연 해소)" % n)
        return 0
    if len(argv) > 2 and argv[1] == "path":
        print(lane_state_path(argv[2]))
        return 0
    if len(argv) > 1 and argv[1] == "kinds":
        for k in sorted(LANE_STATE_KINDS):
            print(k)
        return 0
    sys.stderr.write("usage: javis_lane.py [--self-test | path <kind> | kinds]\n"
                     "  (레인 경로 규약 단일 소유 모듈 — 소비자는 import 하거나 "
                     "`javis_bootstrap.py lane-path <kind>` 로 물어본다)\n")
    return 64          # EX_USAGE — bootstrap 의 미지 서브커맨드 계약과 같은 값


if __name__ == "__main__":
    sys.exit(main(sys.argv))
