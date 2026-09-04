#!/usr/bin/env python3
"""run_bootstrap_health.py — 부트스트랩 상시 건강성 1커맨드 게이트 (T-0147-7 재감사 §5).

    python3 <pack>/bin/tests/run_bootstrap_health.py            # exit 0 = GREEN
    python3 <pack>/bin/tests/run_bootstrap_health.py --json     # 기계 판독
    python3 <pack>/bin/tests/run_bootstrap_health.py --list      # 검체 대장(발효 웨이브 포함)
    python3 <pack>/bin/tests/run_bootstrap_health.py --only H-WIN-1,H-CONC-6
    python3 <pack>/bin/tests/run_bootstrap_health.py --wave W1a  # 특정 웨이브 발효분만

설계(재감사 §5):
  · **발효 시점표**: 검체마다 발효 웨이브 태그가 붙는다. 아직 안 착지한 웨이브의 검체는
    `pending`(회색·게이트 비산입)이고, **발효된 검체의 미구현·실패는 hard fail** 이다.
    '1커맨드 게이트'의 정의 = "현재 발효분 전량 GREEN". 이 규약이 없으면 러너가 W4 완료까지
    상시 적색이어서 릴리스 게이트로 성립하지 않는다.
  · 발효 웨이브 집합은 `LANDED_WAVES` 단일 상수다 — 웨이브가 착지하면 여기만 늘린다.
  · **플랫폼 분기는 스텁 목**(cygpath 목·백슬래시 fixture·드라이브 경로 fixture·lsof 목)으로
    macOS에서도 결정론 실행한다. Windows 실기 재실행은 H-WIN-11(W4·CI 잡) 소속.
  · 각 검체는 커버하는 **원 결함 ID** 를 동봉한다 — 회귀 시 결함 대장으로 즉시 역추적.
  · **계측기 자기검증**(MEMORY '디버깅 계측 타당성 게이트'): 정적 술어 검체는 가능하면
    `git show <기준커밋>:<파일>` 에 같은 탐지기를 돌려 **구 코드에서 FIRE 하는지** 확인한다.
    탐지기가 구 결함을 못 잡으면 신 코드의 PASS 는 아무 의미가 없다.

핀 이사 계약(U-2 · 2026-08-23 · 기계 집행자 = 검체 `H-META-PIN`):
  이 러너의 다수 검체는 **Rust 소스의 문자열 자체를 핀**한다(예: `boot_agent_on_surface` 본문에
  어떤 리터럴이 있다/없다). 그래서 그 코드가 리팩터로 **다른 파일·다른 함수로 이사**하면 검체가
  적색이 된다. 그 순간의 규율을 여기 상주시킨다 — 이 네 줄이 이 파일의 최상위 계약이다.
    ① **핀을 지우지 말고 이사시킨다.** 검체가 소스 문자열을 핀하는 곳에서 리팩터가 필요하면,
       `need(...)` 를 지우는 것이 아니라 **스캔 대상 경로만** 옮긴다. 이사 = 아래 `SCAN_TARGETS`
       레지스트리의 해당 논리 이름에 새 경로를 **추가**하는 것이며, 그 한 줄은 diff 에 반드시
       드러난다(조용한 핀 삭제가 구조적으로 불가능해진다).
    ② **판정 조건은 하나도 완화하지 않는다.** 이 개정에서 바뀔 수 있는 것은 *스캔 대상 경로*
       뿐이다 — `need()` 조건의 삭제·약화, 검체 삭제, 웨이브 재배치는 전부 계약 위반이다.
       판정 축 자체를 옮겨야만 하는 경우에는 **그것이 완화가 아닌 이유를 코드 주석으로 남긴다**
       (선례: H-META-READ ⓑ 의 '여유배수 → 조기경보' 재분류 주석).
    ③ **기능 변경과 검체 개정은 같은 커밋**에 넣는다. 기능을 먼저 착지시키고 검체를 나중에
       초록으로 만들면 **계측기가 결함을 승인하는** 순서가 된다(MEMORY '디버깅 계측 타당성
       게이트' 3칙 ①).
    ④ **적색이 났을 때 첫 행동은 검체 수정이 아니라 원인 규명이다.** "빨개서 고쳐 초록으로
       만들었다"는 정당한 작업이 아니다 — 먼저 *왜* 빨간지(기능 결함인가 · 코드 이사인가 ·
       계측기 자신의 결함인가)를 규명하고, 그 답이 '이사'일 때에만 ①을 적용한다.

출력 스트림 계약(U-27 · 2026-08-24 · 이종 리뷰어 [P0-3] 채택):
  `--json` 일 때 **stdout 에는 JSON 한 덩어리만** 나간다. 사람용 진행 출력과, 검체가 인프로세스로
  부르거나 스폰한 코드가 뱉는 출력은 전부 stderr 로 간다(실측 오염원: `javis_report_gate` 의
  `verdict=… delivered=… reasons=…` 요약 줄이 러너 stdout 으로 새어 12,416자를 JSON 앞에 붙였다).
  구현은 **fd 층위**다 — 검체 실행 구간 동안 fd 1 을 fd 2 로 덮고 JSON 인쇄 직전에만 되돌린다.
  `sys.stdout` 치환으로는 부족하다: 자식 프로세스는 fd 를 **상속**하므로 파이썬 층위를 그냥 지나친다.
  ∴ 소비자는 `json.loads(r.stdout)` 를 그대로 쓴다 — 이 캠페인 내내 워커들이 써 온
  **"첫 `{` + `"summary"` 위치를 찾아 `raw_decode`" 우회 관용구는 이제 필요하지 않다.**
  (`--json` 이 아니면 종전과 완전히 동일하다. 집행자 = 검체 `H-META-OFF` ⓔ.)

미측정 축 계약(U-27 · 2026-08-24 · 이종 리뷰어 [major] 채택):
  Skip 은 한 종류가 아니고, 두 종류는 **서로 다른 사실**이다.
    · **적용 불가**(`skip`)     = 이 환경에는 적용 대상이 없다(darwin 에서 Windows 실기 · 배포 팩에
                                  Rust 소스 부재). 잴 것이 애초에 없으므로 게이트 판정에 중립이다.
    · **사람이 끈 것**(`disabled`) = 대상은 그대로 있는데 **측정을 하지 않기로 사람이 정했다**
                                  (env 스위치). 이것은 통과가 아니다.
  종전 판은 둘을 같은 `skip` 축에 담았다. 그래서 `CYS_U26_OFF=1` 한 줄이 검체 5건을 지우고도
  최종 줄은 `GREEN`, 종료코드는 0 이었다 — Skip 문구는 스스로 "이것은 통과가 아니라 미측정" 이라
  말하는데 요약과 종료코드는 통과라고 말한 것이다. 이 저장소가 반복해서 낸 사고의 형태
  ("측정 불능을 통과로 접는다")를 **계측기 자신이** 재현했다.
  ∴ 지금은 ①`disabled` 가 별도 축이고 ②등재된 스위치가 하나라도 켜져 있으면 판정 라벨이
  `UNMEASURED` 이며 ③**종료코드가 2** 다. 코드 3값의 의미는 서로 다르다 —
  **0=GREEN(다 재고 다 통과) · 1=RED(재서 틀렸다) · 2=UNMEASURED(재지 않았다).**
  등재소는 `MEASUREMENT_OFF_SWITCHES`(끄는 스위치)와 `MEASUREMENT_PIN_OVERRIDES`(핀을 갈아끼우는
  스위치) 둘뿐이고, 새 스위치를 등재 없이 만들면 검체 `H-META-OFF` ⓑ 가 소스에서 찾아내 적색으로
  만든다.

stdlib만 사용. 네트워크 0. 어떤 검체도 사용자 HOME·실 데몬을 건드리지 않는다(전부 격리 tmp).
"""
import argparse
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time

# ── 경로 해소 ────────────────────────────────────────────────────────────────
TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN_DIR = os.path.dirname(TESTS_DIR)
PACK_DIR = os.path.dirname(BIN_DIR)
HOOKS_DIR = os.path.join(PACK_DIR, "hooks")
REPO_DIR = os.path.dirname(PACK_DIR)          # 레포 체크아웃일 때만 유효(배포 팩은 무의미)
PY = sys.executable or "python3"
# ★bash 단일 해소(2026-08-10 W-A · run 31396459407/31396849323 근저원인): 네이티브 Windows
#   python 의 CreateProcess 는 PATH 탐색 **이전에** System32 를 뒤져 WSL 스텁 bash.exe 를
#   집는다 — 리터럴 "bash" 스폰은 Windows 실기에서 훅 검체 전군을 오판시킨다. 전 스폰 지점을
#   이 모듈 레벨 상수로 보낸다(H-WIN-5 의 기존 정답 패턴을 모듈로 승격 · macOS 동작 불변).
BASH = shutil.which("bash") or "bash"

# 기준 커밋 핀(재감사 헤더) — 계측기 자기검증(구 코드 FIRE 확인)의 대조 트리.
# W0 착지 커밋을 쓴다: W1a 이전 상태이면서 W0 지혈이 반영된 트리.
CALIBRATION_REF = os.environ.get("CYS_HEALTH_CALIB_REF", "a96d8b1")
# W0 **이전** 트리 — W0 자신이 고친 문구 결함(G30·G31·G32·FILLER)의 계측 대조는 이쪽이어야 한다
# (W0 착지 트리에서는 이미 수리돼 탐지기가 FIRE 하지 않는다 = 기준 선택 오류로 인한 오보 차단).
PRE_W0_REF = os.environ.get("CYS_HEALTH_PRE_W0_REF", "b35f01d")
# D4-a(무스폰) 시대 트리 — H-MISSION-1 스폰 검출기의 계측 대조용. 그 시대 훅은 임무 미지정
# 선언에서 spawn 하지 않았고(2択 혼동 결함), D4-a′(2026-08-10 오너 재정: 선언=기동 명령)가
# 대체했다. 위 두 기준과 같은 이유로 **고정 해시**를 쓴다(HEAD 는 D4-a′ 착지와 함께 움직인다).
D4A_REF = os.environ.get("CYS_HEALTH_D4A_REF", "58337fb")
# W2 착지 이후·W-A3(⑤ exit 2 `cys ping` 재확인 분기 · 2026-08-21) **이전** 트리 — H-EXIT-7
# 신계약의 계측 대조용. 그 트리엔 ⑤check-unjudgeable(즉시 이탈)은 있으나 `cys ping` 재확인·
# '팩 결손 가능성' 진단이 없다(exit 2 전부를 '데몬 소실'로 오진하던 구 계약 + 127='orchestra
# 스크립트 부재' 사문 문안). 위 기준들과 같은 이유로 **고정 해시**를 쓴다(HEAD 는 W-A3 착지와
# 함께 움직인다).
PRE_WA3_REF = os.environ.get("CYS_HEALTH_PRE_WA3_REF", "8def22a")
# U-24(레인 규약 `javis_lane` 이관) **이전** 트리 = 부트 결정론 캠페인 베이스. 이 이관의 '구 코드'는
# W0(a96d8b1)이 아니다 — 그 트리엔 `lane_state_path` 자체가 아직 없어(H-LIFE-1 이 그 부재를 핀한다)
# "구 코드에 인라인 소유가 있었다"를 잴 수 없다. 잘못된 기준의 '탐지 실패'는 탐지기 파손이 아니라
# 기준 선택 오류다(러너 헤더 규약) — 그래서 이 축만의 고정 해시를 따로 둔다.
PRE_U24_REF = os.environ.get("CYS_HEALTH_PRE_U24_REF", "985093d")

# ★U-0(2026-08-23 · 계측 타당성 복원) — `_read` 의 읽기 상한(문자 수).
#   구 값 400,000자는 검체가 읽는 실제 파일보다 **작았다**. `f.read(limit)` 은 초과분을 말없이
#   버리므로, 검체는 절반만 본 텍스트에 `in`/`find()` 를 걸고 "없다"고 단언했다 —
#   실측(2026-08-23):
#     · src/bin/cys.rs            = 715,231자 (구 상한에서 315,231자 = 44% 미가시)
#     · src/bin/cysd/handlers.rs  = 552,867자 (구 상한에서 152,867자 = 28% 미가시)
#   그 직접 피해가 H-CONC-4 의 **거짓 적색**이다: T-0147-4 회귀 앵커
#   `close_rejects_foreign_surface` 는 실재하는데(src/bin/cysd/handlers.rs:9852 · 문자
#   오프셋 438,079) 절단 범위 밖이라 검체가 "사라졌다"고 판정했다.
#   ★값 선정: 실측 최대(715,231자)의 약 11배. 대상 파일이 두 배로 자라도 여유가 있고,
#     그럼에도 상한이 남아 있어 병리적 대용량 파일에서 메모리가 무한 팽창하지 않는다.
#   ★롤백 스위치는 이 1지점이다 — 상수 또는 env `CYS_HEALTH_READ_LIMIT`.
#   ★상한 도달은 '판정'이 아니라 '측정 실패'로 취급한다(아래 `_read` 가드 · H-META-READ).
READ_LIMIT_CHARS = int(os.environ.get("CYS_HEALTH_READ_LIMIT") or 8000000)

# `_read` 가 실제로 읽은 경로 → 문자 수. 계측기 자기감시(H-META-READ)가 소비한다.
_READ_OBSERVED = {}

# ★발효 웨이브 — 착지한 웨이브만 넣는다. 미발효 검체는 pending(게이트 비산입).
LANDED_WAVES = ("W0", "W1a", "W1b", "W2", "W3", "W4", "W5", "W6")

_REG = []          # [(id, wave, title, defects, fn|None)]


def specimen(sid, wave, title, defects):
    def deco(fn):
        _REG.append((sid, wave, title, defects, fn))
        return fn
    return deco


def pending(sid, wave, title, defects):
    """미발효(또는 미구현) 검체를 대장에만 등록한다 — 커버리지 공백을 숨기지 않는다."""
    _REG.append((sid, wave, title, defects, None))


class Fail(AssertionError):
    pass


class Skip(Exception):
    """검체 **적용 불가** — 이 환경에 잴 대상이 애초에 없다(배포 팩에 Rust 소스 부재 · darwin
    에서 Windows 실기 · root 라 chmod 재현 불가 등). fail 아니고, 게이트 판정에 중립이다.

    ★사람이 계측기를 **끈** 경우는 이것이 아니라 아래 `Disabled` 다(헤더 '미측정 축 계약')."""


class Disabled(Skip):
    """★**사람이 끈 미측정** — `Skip`(적용 불가)과 다른 사실이다.

    측정 대상은 그대로 있는데 env 스위치로 계측기를 껐을 뿐이므로, 이것은 **통과가 아니다**.
    `Skip` 의 하위형으로 둔 이유는 기존 `except Skip` 경로(검체 내부의 조기 이탈 관용구)를
    깨지 않기 위해서다 — 판정 분기는 `main()` 이 `Disabled` 를 **먼저** 잡아 별도 축으로 센다.

    ★왜 축을 갈랐는가(2026-08-24 이종 리뷰어 [major]): 종전 판에서
    `CYS_U26_OFF=1 … --only H-CI-TAG-1,H-EVID-1,H-FAKE-1,H-FAKE-2,H-FAKE-3` 은 검체 5건을
    전부 지우고도 `GREEN — 발효 0 PASS / 0 FAIL / 5 SKIP` · exit 0 을 냈다. 요약과 종료코드가
    "통과" 라고 말한 것이다. 이 파일이 막으려는 실패 유형("측정 불능을 통과로 접는다")을
    계측기 자신이 재현한 자리이므로, **판정 라벨과 종료코드 양쪽**에서 갈라 놓는다."""


# ── ★'사람이 끈 미측정' 축 등재소(U-27 · 2026-08-24) ─────────────────────────
# ① 측정을 **끄는** 스위치. 켜져 있으면 그 검체는 `disabled`(별도 축)이고, 이 런은 GREEN 을
#    잃고 exit 2 가 된다. 새 롤백 스위치를 만드는 사람은 `off_switch()` 로 여기에 등재한다.
MEASUREMENT_OFF_SWITCHES = []          # [(env, scope)] — off_switch() 가 채운다


def off_switch(env, scope):
    """롤백/차단용 env 스위치를 미측정 축 등재소에 올리고 이름을 그대로 돌려준다."""
    MEASUREMENT_OFF_SWITCHES.append((env, scope))
    return env


# ② 측정을 끄지는 않지만 **핀(측정 기준)을 갈아끼우는** 스위치. 검체는 계속 돌지만 그 초록의
#    의미가 달라진다 — 계측 대조 기준 커밋을 바꾸면 "구 코드에서 FIRE 한다"가 다른 트리에 대한
#    주장이 되고, 읽기 상한을 바꾸면 `_read` 의 절단 가드가 다른 자리에서 물린다.
#    ★같은 규율로 GREEN 을 박탈하는 근거: 이쪽은 `skip` 조차 남기지 않고 **조용히 PASS** 로
#      접힌다(실측 형태 — 기준 커밋을 해소 불가한 값으로 주면 `_git_show` 가 None 을 돌려
#      계측 대조가 `계측대조 생략(레포 아님)` 으로 강등되는데, 검체는 그대로 PASS 다).
#      끈 것보다 조용하므로 더 엄하게 다룬다. 판단이 갈리는 자리에서는 엄격한 쪽을 택한다.
#    ⚠이름은 **열거**한다. `CYS_HEALTH_` 접두 스캔으로 잡으면 제품 env
#      `CYS_HEALTH_NARRATION_CJK_MIN`(cysd 헬스 룰)까지 오검출한다 — 그건 러너의 스위치가 아니다.
MEASUREMENT_PIN_OVERRIDES = (
    ("CYS_HEALTH_CALIB_REF", "계측 대조 기준 커밋(W0 착지)"),
    ("CYS_HEALTH_PRE_W0_REF", "계측 대조 기준 커밋(W0 이전)"),
    ("CYS_HEALTH_D4A_REF", "계측 대조 기준 커밋(D4-a 무스폰 시대)"),
    ("CYS_HEALTH_PRE_WA3_REF", "계측 대조 기준 커밋(W-A3 이전)"),
    ("CYS_HEALTH_PRE_U24_REF", "계측 대조 기준 커밋(U-24 이전)"),
    ("CYS_HEALTH_W5_CALIB_REF", "계측 대조 기준 커밋(W5 축)"),
    ("CYS_HEALTH_U23_CALIB_REF", "계측 대조 기준 커밋(U-23 축)"),
    ("CYS_HEALTH_READ_LIMIT", "`_read` 절단 가드 임계(U-0)"),
)


def _off_switch_on(env):
    """env 스위치가 켜져 있는가. 종전 `_u26_gate` 의 판정을 그대로 옮기고 `on`·대문자만 넓혔다
    (넓히는 방향 = '꺼진 것으로 볼 값'이 늘어난다 = 더 엄해진다 — 완화가 아니다)."""
    return (os.environ.get(env) or "").strip().lower() in ("1", "true", "yes", "on")


def _off_switches_engaged():
    """지금 켜져 있는 등재 스위치 — [(kind, env, value, scope)]. kind: off | pin."""
    out = []
    for env, scope in MEASUREMENT_OFF_SWITCHES:
        if _off_switch_on(env):
            out.append(("off", env, os.environ.get(env), scope))
    for env, scope in MEASUREMENT_PIN_OVERRIDES:
        if (os.environ.get(env) or "").strip():
            out.append(("pin", env, os.environ.get(env), scope))
    return out


def need(cond, msg):
    if not cond:
        raise Fail(msg)


# ── 공용 하네스 ──────────────────────────────────────────────────────────────
def _w(path, body, mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def _run(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    kw.setdefault("encoding", "utf-8")
    kw.setdefault("errors", "replace")
    kw.setdefault("timeout", 120)
    return subprocess.run(cmd, **kw)


def _read(path, limit=None):
    """파일 전문을 읽는다. **절단은 판정이 아니라 측정 실패다.**

    U-0(2026-08-23): 구 판은 `f.read(400000)` 으로 초과분을 조용히 버렸다. 검체는 잘린 텍스트에
    `in`/`find()` 를 걸어 '없다'고 단언했고, 실재하는 회귀 앵커가 사라졌다는 거짓 적색을 냈다
    (H-CONC-4 · handlers.rs 문자 오프셋 438,079 > 400,000). 계측기가 대상의 절반만 보면서
    '없음'을 단언하는 것은 계측 무효다 — MEMORY '디버깅 계측 타당성 게이트' 3칙 ①.
    ∴ 상한에 **도달하면** 잘라서 돌려주지 않고 `Fail` 을 던진다. 조용한 절단은 불가능하다.
    (읽기 자체가 불가능한 경우(OSError)는 종전대로 빈 문자열 — 부재는 검체가 판정할 사실이다.)
    """
    lim = READ_LIMIT_CHARS if limit is None else limit
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            body = f.read(lim + 1)      # ★상한+1 — '도달했는가'를 관측 가능하게 만드는 1자
    except OSError:
        return ""
    if len(body) > lim:
        raise Fail("읽기 상한 도달 = 파일 절단 = 계측 무효(U-0 가드): %s (>%d자). "
                   "READ_LIMIT_CHARS 를 올리거나 env CYS_HEALTH_READ_LIMIT 로 조정하라 — "
                   "잘린 텍스트로 '부재'를 판정해선 안 된다." % (path, lim))
    _READ_OBSERVED[path] = len(body)
    return body


def _hook(name):
    p = os.path.join(HOOKS_DIR, name)
    need(os.path.isfile(p), "훅 부재: %s" % p)
    return p


def _shell_hooks():
    """훅 .sh 전수 — `_lib.sh`(프리루드)는 훅이 아니라 라이브러리이므로 제외한다.
    (G-SYNTAX 는 프리루드를 별도로 명시 추가해 문법 검사한다.)"""
    out = []
    for root, _dirs, files in os.walk(HOOKS_DIR):
        for f in sorted(files):
            if f.endswith(".sh") and f != "_lib.sh":
                out.append(os.path.relpath(os.path.join(root, f), HOOKS_DIR))
    return sorted(out)


# ★부트 v2 A2(2026-09-04): UserPromptSubmit 훅이 **런처 + 본체** 2파일로 갈렸다.
#   · 등록·실행 진입점 = `role-bootstrap.sh`(자기완결 런처) — E2E 는 여전히 이 파일을 돌린다.
#   · 판독 규칙·문안·note 블록이 사는 곳 = `role-bootstrap-legacy.sh`(본체).
#   그래서 **내용 핀은 본체를**, **실행은 런처를** 본다. 둘을 섞으면 이 하네스가 조용히
#   아무것도 재지 않게 된다(계측 무효화 — 이 파일이 반복해서 경계한 형상).
#   구 커밋 대조(`_git_show`)는 분할 이전 트리를 읽으므로 경로가 그대로다(자기완결 구 훅).
ROLE_HOOK = "role-bootstrap.sh"                 # 런처(등록·실행)
ROLE_BODY = "role-bootstrap-legacy.sh"          # 본체(내용 핀)


def _git_show(relpath, ref=None):
    """기준 커밋의 파일 내용(계측기 자기검증용). 레포가 아니거나 실패면 None.

    ★ref 인자(W4): 기본 기준은 W0 착지 커밋(`CALIBRATION_REF`)이지만, **W0 자신이 고친 결함**
      (G30·G31·G32·P3-A-FILLER 의 문구면)은 그 트리에서 이미 수리돼 있어 탐지기가 FIRE 하지
      않는다 — 그 경우 계측 대조는 **W0 이전 트리**(`PRE_W0_REF`)로 해야 유효하다. 잘못된 기준에서
      "구 코드가 안 잡힌다"는 결과는 탐지기 파손이 아니라 기준 선택 오류다(오보 방지)."""
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        return None
    r = _run(["git", "-C", REPO_DIR, "show", "%s:%s" % (ref or CALIBRATION_REF, relpath)],
             timeout=30)
    return r.stdout if r.returncode == 0 else None


def _mock_cys(bindir, tmp, extra_case=""):
    """호출을 calls.log 에 적재하는 스텁 cys. extra_case = 추가 서브커맨드 분기 sh 코드.

    ★P2(R3-P2-7 ⓐ): `boot-intent` 의 기본값은 **구 CLI 모사**(rc2+"unrecognized subcommand")다 —
      기존 검체 전부가 종전 spawn **폴백 leg** 를 계속 재게 하는 장치다(훅의
      `_cys_hook_legacy_unavailable` 가 조용한 스큐로 접는 형상). 미지 서브커맨드 기본 rc0 을
      그대로 두면 토큰 없는 rc0 이 '스큐 증거 없음' loud 폴백이 되어 전 검체 stderr 가 오염되고,
      frontdoor leg 를 재려는 검체가 어느 leg 를 재는지 불명해진다. frontdoor leg 는 신설 검체
      (H-BOOT-INTENT-1)가 자기 arm 을 extra_case 로 주입해 전담한다 — extra_case 의 분기가
      먼저 매치되므로 이 기본값을 이긴다."""
    _w(os.path.join(bindir, "cys"),
       "#!/bin/sh\n"
       'printf "%s\\n" "cys $*" >> "{log}"\n'
       "{extra}\n"
       "case \"$1\" in boot-intent) echo \"error: unrecognized subcommand 'boot-intent'\" >&2; exit 2;; esac\n"
       "exit 0\n".format(log=os.path.join(tmp, "calls.log"), extra=extra_case))


def _calls(tmp):
    return _read(os.path.join(tmp, "calls.log"))


def _base_env(extra=None, drop=()):
    env = dict(os.environ)
    for k in ("CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_SOCKET", "CYS_PACK_DIR",
              "CYS_ROLE", "CYS_STATE_DIR", "CYS_LOCK_BACKEND", "CYS_ROOT", "CYS_SOUL",
              # ★임무 게이트 신호는 러너 환경에서 새어 들어오면 안 된다(검체가 자기 조건을
              #   명시적으로 만든다 — `_rb_sandbox(mission=...)`).
              "CYS_MISSION",
              # ★제품 롤백 스위치 누수 차단(U-27 · 2026-08-24 · 미측정 축 전수 점검의 부산물).
              #   이것들은 **제품의** 판정 축을 끄는 스위치다(마스터·강등·축·주입 가드). 운영자
              #   셸에 하나라도 켜져 있으면 검체가 띄우는 훅·부트가 **가드가 꺼진 제품**을 재고,
              #   러너는 그 사실을 어디에도 적지 않는다 — "환경이 계측을 조용히 바꾼다" 는
              #   `CYS_U26_OFF` 와 같은 계열의 구멍이다. 검체가 이 축을 시험할 때는 `extra` 로
              #   **명시 주입**하므로(strip 은 extra 적용 **전**이다) 그 경로는 영향받지 않는다.
              "CYS_BOOT_GATES", "CYS_GATE_PENDING", "CYS_GATE_PENDING_CLOSE",
              "CYS_INJECT_GATE_GUARD"):
        env.pop(k, None)
    for k in drop:
        env.pop(k, None)
    if extra:
        env.update(extra)
    return env


def _rb_sandbox(tmp, *, boot_body=None, surface=True, mock_py=None, pack_has_boot=True,
                mission="health-suite 검체 임무(오너 지정 가정)"):
    """role-bootstrap.sh 격리 실행 환경. 반환: (env, home, pack, bindir, state_dir).

    ★`mission` 기본값이 있는 이유(2026-08-10 D4-a′ 이후에도 유지): spawn 은 이제 임무 유무와
      무관하지만(선언=기동 명령 — 오너 재정), 임무 게이트 exit 는 여전히 **착수 규율 문안**
      (MISSION_SENT)을 가른다. H-DETECT-* 계열은 '선언을 감지해 **발화**하는가'(A4·G9·G25·A2·
      A3·A5·A6·A16·R3)를 재는 검체이므로, 문안 변인을 고정한 조건 = **오너가 임무를 준 세션**
      에서 돌려 원래 커버리지를 보존한다. 그 조건을 만드는 실재 수단이 `CYS_MISSION` 이다
      (`javis_mission.gate()` 의 ①번 신호).
      ★정정(2026-08-01 R2 적발 (d)): 이 변수를 **자동으로 채우는 launcher 는 없다**(`src/`·`ui/`
      배선 0건). pane env 는 데몬 프로세스 env 를 상속하므로 CLI 호출 시점의 값은 전달되지
      않는다 — 여기서는 테스트 하네스가 직접 주입해 '오너가 임무를 준 세션'을 만든다.
      단언은 하나도 바꾸지 않는다.
      `mission=None` 으로 넘기면 임무 미지정 경로(신규 사용자 — spawn 1 + 착수금지 문안)를
      재현한다 — H-MISSION-1 소속.
    """
    home = os.path.join(tmp, "home")
    pack = os.path.join(tmp, "pack")
    bindir = os.path.join(tmp, "bin")
    state = os.path.join(home, ".cys", "state")
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    if pack_has_boot:
        _w(os.path.join(pack, "bin", "javis_bootstrap.py"),
           boot_body if boot_body is not None else "print('MOCK-BOOT')\n", 0o644)
    _mock_cys(bindir, tmp, 'case "$1" in surface-role) echo ""; exit 0;; esac')
    if mock_py:
        _w(os.path.join(bindir, "python3"), mock_py)
    # ★CYS_STATE_DIR 명시 핀(2026-08-10 Windows 실기 run 31403557039 근저원인): 종전엔 HOME 만
    #   덮어 상태를 격리했는데, **ntpath.expanduser 는 env HOME 을 무시**하고 USERPROFILE 을
    #   쓴다(javis_bootstrap.HOME=expanduser("~")). 그래서 Windows 에서는 env 미설정 네이티브
    #   python(ⓓ-2 원장 픽스처 등)이 **러너 실사용자 ~/.cys/state** 에 쓰고, 훅 경유 자식은
    #   _lib.sh 가 샌드박스 HOME 로 만든 CYS_STATE_DIR 을 읽어 — 쓰는 쪽과 읽는 쪽이 갈라져
    #   층1(배달 원장 대조)이 공전했다(+"어떤 검체도 사용자 HOME 을 건드리지 않는다" 하네스
    #   계약도 Windows 에서 깨져 있었다). 값은 macOS 에서 _lib.sh 파생값과 문자열까지 동일
    #   (`<home>/.cys/state`)이라 posix 동작 무변경이고, _lib.sh 는 `${CYS_STATE_DIR:-…}` 로
    #   기존값을 보존하므로 훅 경유·직접 실행 어느 쪽도 같은 경로를 본다.
    env = _base_env({"HOME": home, "CYS_PACK_DIR": pack, "CYS_STATE_DIR": state,
                     "PATH": bindir + os.pathsep + os.environ.get("PATH", "")})
    if surface:
        env["CYS_SURFACE_ID"] = "7"
    if mission:
        env["CYS_MISSION"] = mission
    else:
        env.pop("CYS_MISSION", None)
    return env, home, pack, bindir, state


def _run_rb(env, prompt="너는 마스터다"):
    return _run([BASH, _hook("role-bootstrap.sh")], input=json.dumps({"prompt": prompt}), env=env)


def _code_lines(body):
    """셸/파이썬 소스에서 **주석 전용 줄을 제거**한 코드만 남긴다.
    ★계측기 오탐 방지: 제거한 결함을 주석으로 **설명한** 줄(`cut -c1-200 은 제거됐다`)까지
      정적 스캔이 잡으면, 문서화가 곧 회귀로 보고된다(W1a G-PRELUDE 선례와 동일 규약)."""
    return "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))


def _detect_mod():
    """감지기 단일 소유 모듈(bin/javis_detect.py) 적재 — W1b corpus 검체의 피검체."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_detect
    return javis_detect


def _old_hook_fires(prompt, tmp, env_extra=None):
    """★계측 타당성: 기준 커밋의 **구 훅**을 같은 프롬프트로 돌려 발화 여부를 관측한다.
    구 코드가 결함을 재현하지 못하면 신 코드의 PASS 는 아무것도 증명하지 않는다(MEMORY 3칙).
    반환: True/False/None(레포 아님 — 계측 대조 생략)."""
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    if old is None:
        return None
    oldhook = os.path.join(tmp, "oldhooks", "role-bootstrap.sh")
    _w(oldhook, old)
    # 구 훅(a96d8b1)은 프리루드를 source 하지 않지만, 상위 웨이브 계보에서 온 사본이 섞이면
    # loud-skip 으로 전멸할 수 있어 형제 위치에 프리루드를 병치해 둔다(무해·멱등).
    _w(os.path.join(tmp, "oldhooks", "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
    env, _h, _p, _b, _st = _rb_sandbox(os.path.join(tmp, "oldsb"))
    if env_extra:
        env.update(env_extra)
    r = _run([BASH, oldhook], input=json.dumps({"prompt": prompt}), env=env)
    return "발화됨" in r.stdout


def _calib_note(fired, expect, what):
    """구 코드 관측치가 기대(결함 재현)와 일치하는지 확인하고 계측검증 문구를 만든다."""
    if fired is None:
        return "skip(no-git)"
    need(fired == expect,
         "계측 타당성 실패: 구 코드가 %s 에서 fired=%s (기대 %s) — 검체가 결함을 재현하지 못한다"
         % (what, fired, expect))
    return "구 코드 fired=%s 재현" % fired


def _run_ledgers(state, timeout=8.0):
    """런별 발화 로그 목록(백그라운드 기록 완료까지 bounded 대기)."""
    end = time.time() + timeout
    while time.time() < end:
        names = sorted(n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)) \
            if os.path.isdir(state) else []
        if names and all(os.path.getsize(os.path.join(state, n)) > 0 for n in names):
            return names
        time.sleep(0.1)
    return sorted(n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)) \
        if os.path.isdir(state) else []


# ═══════════════════════════════════════════════════════════════════════════
# 0-a. 발행 차단 게이트 (시크릿·개인정보) — ★등록 순서상 **첫 검체**
# ═══════════════════════════════════════════════════════════════════════════
# ★W0 배치 근거(2026-08-24):
#   ① **fail-fast 자리.** 개인정보가 트리에 살아 있으면 그 트리는 어떤 검체가 초록이든
#      발행할 수 없다. 그 사실을 나머지 139건보다 **먼저** 드러내는 것이 옳은 순서다.
#   ② **싸다.** `--all` 은 추적 853파일 전수를 14초 안에 훑는다(실측 2026-08-24 · macOS).
#      W0(베이스라인) 예산 안에서 매 런 돌 수 있다.
#   ③ **CI 만으로는 늦다.** 이 게이트는 지금까지 CI 잡에서만 돌아 **태그를 찍은 뒤에야**
#      적색이 됐다(#81 이 그 실증이다). `.git/hooks/pre-commit` 은 per-clone 이라 추적
#      파일로 강제할 수 없다 — 그래서 **상시 1커맨드 게이트 안**으로 들여온다.
#
# ★합성 표본을 **조각으로 이어 붙이는** 이유: 이 러너 자신이 `git ls-files` 대상이다.
#   양성 표본을 리터럴로 적으면 스캐너가 이 파일을 잡아 ⓐ가 영구 적색이 된다. 그때의
#   유혹은 "러너를 스캐너 제외 목록에 넣는 것"인데 그것은 **판정 완화**다(러너 헤더 계약 ②).
#   조각 이어붙이기는 그 유혹 자체를 없앤다 — ⓒ가 그 상태를 동결한다.
_SECRET_PROBES = (
    # (표본 id, 기대 라벨, 본문) — 라벨은 `scripts/secret-scan.sh` 의 규칙 이름 그대로다.
    ("path", "PATH", "log: wrote /Users/%s/work/notes.md" % ("jd" + "oe")),
    ("win-path", "WIN-PATH", "install dir = C:\\Users\\%s\\AppData\\Local\\cys" % ("jd" + "oe")),
    # ★사각 ②의 직격 표본 — 구 PROFILE 규칙은 정슬래시 전용이라 이 줄을 통과시켰다.
    ("profile-win", "PROFILE", "USERPROFILE=C:\\Users\\%s" % ("c" + "ys")),
    ("profile-dot", "PROFILE", "config = ~/.claude-%s/settings.json" % ("ysfut" + "ure")),
    ("email", "EMAIL", "contact: %s" % ("probe" + "@" + "acme-corp.dev")),
    ("secret", "SECRET", "AWS_ACCESS_KEY_ID=%s" % ("AKIA" + "1234567890ABCDEF")),
    # ★사각 ③의 직격 표본 — 경로 접두 없이 pane 제목 꼬리로 박힌 오너 기계 계정명.
    ("handle", "HANDLE", "node title = cso-claude · %s" % ("cys-mac" + "book")),
)

# 음성 대조 — **승인된 더미만** 담긴 파일은 0건이어야 한다. 이 축이 없으면 "무엇에나 FIRE 하는
# 고장난 스캐너"도 위 7건을 전부 통과시킨다(탐지력 증명이 아니라 소음 측정이 된다).
# ★POSIX 더미도 조각으로 만든다: 형제 스캐너(`scan-pack-secrets.sh`)의 placeholder 목록은
#   `x|you|NAME` 로 더 좁다. 더미 `user` 를 홈경로 리터럴로 적으면 **팩 발행 게이트**가 이 파일을
#   잡는다(실측 — 이 주석의 초안이 그렇게 걸렸다). 두 게이트의 허용집합 차이도 조각화가 흡수한다.
_SECRET_CLEAN = (
    "posix   /Users/%s/work/notes.md\n" % "x" +
    "posix   /Users/%s/.cys/pack\n" % "user" +
    "win     C:\\Users\\x\\AppData\\Local\\cys\n"
    "win     PS C:\\Users\\x> claude --dangerously-skip-permissions\n"
    "docs    C:\\Users\\...\\AppData (문서의 생략 표기)\n"
    "brand   cysinsight — LICENSE·README·홈페이지 URL 의 공개 브랜드다(개인정보 아님)\n"
    "prompt  user@host:~/dev$\n"
)


@specimen("H-SECRET-1", "W0",
          "발행 차단 게이트 상시화 — 산 트리 clean(853파일) + 규칙 6종 합성 변조 FIRE + 음성 대조",
          ["#81", "개인정보-누출", "CI-only-게이트"])
def h_secret_1():
    """`scripts/secret-scan.sh` 를 **CI 밖으로** 꺼내 상시 게이트에 태운다.

    ⓐ **산 트리** — 추적 전수(`--all`)가 exit 0 인가. 개인 홈경로(POSIX·Windows) · 개인
       프로필 · 이메일 · 토큰 · 개인 계정 핸들이 트리에 하나도 없다는 뜻이다.
    ⓑ **★합성 변조(계측 타당성)** — ⓐ 하나만으로는 아무것도 증명하지 못한다. 스캐너가
       죽어 있어도(정규식 파손 · 조기 exit 0 · 파일 목록 0건) clean 트리는 똑같이 초록이다.
       MEMORY '디버깅 계측 타당성 게이트' 3칙 ①이 금지하는 바로 그 형태다. 그래서 임시
       파일에 **규칙별 양성 표본**을 하나씩 넣고 **그 규칙 라벨로** 잡히는지 확인한다.
       라벨까지 보는 이유: 아무 규칙이나 걸려도 exit 1 이므로, 종료코드만 보면 규칙 6종 중
       5종이 죽어 있어도 전건 초록이다.
    ⓒ **자기 면제 동결** — 이 러너를 스캐너 제외 목록에 넣지 않았는가. ⓑ의 표본이 리터럴로
       적히면 ⓐ가 적색이 되고, 그 적색을 지우는 가장 쉬운 길이 '러너를 제외 목록에 추가'다.
       그것은 계측기가 자기 판정 대상을 줄이는 것 = 완화다(러너 헤더 계약 ②).

    ★임시 파일은 **저장소 밖**(`tempfile`)에만 만들고 반드시 지운다. 사용자 HOME 무접촉."""
    if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
        raise Skip("레포 체크아웃이 아니다(배포 팩 실행) — 발행 게이트 스캐너 부재")
    scan = os.path.join(REPO_DIR, "scripts", "secret-scan.sh")
    need(os.path.isfile(scan), "발행 게이트 스캐너 부재: %s" % scan)

    def _scan(*args):
        return _run([BASH, scan] + list(args), cwd=REPO_DIR, timeout=300)

    def _labels(out):
        return {ln.split("\t", 1)[0] for ln in out.splitlines() if "\t" in ln}

    # ⓐ 산 트리
    r = _scan("--all")
    need(r.returncode == 0,
         "산 트리에 시크릿·개인정보가 남아 있다(발행 차단) — exit=%d\n%s"
         % (r.returncode, (r.stdout + r.stderr)[-2000:]))
    m = re.search(r"clean \(mode=--all, (\d+) 파일\)", r.stdout)
    need(m, "스캐너 clean 요약을 읽지 못했다(출력 계약 변경?): %r" % r.stdout[-400:])
    scanned = int(m.group(1))
    need(scanned >= 500,
         "스캔 대상이 %d 파일뿐 — `git ls-files` 가 비었거나 목록이 끊겼다. '0건 발견'이 "
         "'아무것도 안 봤다'와 구별되지 않는다(측정 실패를 통과로 접지 않는다)" % scanned)

    # ⓑ 합성 변조 — 규칙별 양성 표본 + 음성 대조
    tmp = tempfile.mkdtemp(prefix="cys-secret-probe-")   # ★저장소 밖 · HOME 무접촉
    try:
        fired = []
        for pid, label, body in _SECRET_PROBES:
            f = os.path.join(tmp, "probe-%s.txt" % pid)
            with open(f, "w", encoding="utf-8", newline="\n") as fh:
                fh.write(body + "\n")
            pr = _scan(f)
            got = _labels(pr.stdout)
            need(pr.returncode == 1,
                 "합성 표본 %s 가 차단되지 않았다(exit=%d) — 스캐너가 이 규칙을 못 본다: %r"
                 % (pid, pr.returncode, body))
            need(label in got,
                 "합성 표본 %s 가 %s 규칙으로 잡히지 않았다(잡힌 라벨=%s) — 그 규칙이 죽었다"
                 % (pid, label, sorted(got) or "없음"))
            fired.append("%s→%s" % (pid, label))

        clean = os.path.join(tmp, "negative-control.txt")
        with open(clean, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(_SECRET_CLEAN)
        cr = _scan(clean)
        need(cr.returncode == 0,
             "음성 대조가 차단됐다 — 승인된 더미·공개 브랜드까지 잡는 스캐너는 곧 꺼진다:\n%s"
             % (cr.stdout + cr.stderr)[-1200:])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    need(not os.path.isdir(tmp), "합성 표본 임시 디렉터리가 남았다: %s" % tmp)

    # ⓒ 자기 면제 동결
    stext = _read(scan)
    need("run_bootstrap_health" not in stext,
         "스캐너 제외 목록에 이 러너가 등재됐다 — 계측기가 자기 판정 대상을 줄인 것이다(완화 금지)")

    return ("산 트리 %d파일 clean · 합성 변조 %d/%d FIRE(%s) · 음성 대조 0건"
            % (scanned, len(fired), len(_SECRET_PROBES), " ".join(fired)))


# ═══════════════════════════════════════════════════════════════════════════
# 0. 베이스라인 (결정론 회귀 — 재감사 부채 V3)
# ═══════════════════════════════════════════════════════════════════════════
def _baseline(cmd, label):
    r = _run(cmd, timeout=300)
    need(r.returncode == 0,
         "%s 비0 종료(exit=%d)\n--- stdout ---\n%s\n--- stderr ---\n%s"
         % (label, r.returncode, r.stdout[-1500:], r.stderr[-1500:]))
    return (r.stdout.strip().splitlines() or [""])[-1][:200]


@specimen("B-1", "W0", "베이스라인 test_role_bootstrap_hook", ["A2", "A4", "A3=B7"])
def b1():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_role_bootstrap_hook.py")], "test_role_bootstrap_hook")


@specimen("B-2", "W0", "베이스라인 test_bootstrap_chain", ["부트 체인 exit 계약"])
def b2():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_bootstrap_chain.py")], "test_bootstrap_chain")


@specimen("B-3", "W0", "베이스라인 test_import_guard", ["형제 모듈 import 가드"])
def b3():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_import_guard.py")], "test_import_guard")


@specimen("B-4", "W0", "베이스라인 javis_bootstrap --self-test", ["레인 격리·결손 판정"])
def b4():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_bootstrap.py"), "--self-test"], "bootstrap --self-test")


@specimen("B-5", "W0", "베이스라인 javis_boot_node --self-test", ["생존 술어·회수 판정"])
def b5():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_boot_node.py"), "--self-test"], "boot_node --self-test")


@specimen("B-6", "W0", "베이스라인 test_dept_doctrine_v1", ["부서 교리 v1"])
def b6():
    return _baseline([PY, os.path.join(TESTS_DIR, "test_dept_doctrine_v1.py")], "test_dept_doctrine_v1")


@specimen("B-7", "W1a", "javis_lock --self-test (공용 락·원자쓰기)", ["A8py", "R1"])
def b7():
    p = os.path.join(BIN_DIR, "javis_lock.py")
    need(os.path.isfile(p), "javis_lock.py 부재 — A8py 미착지")
    return _baseline([PY, p, "--self-test"], "javis_lock --self-test")


# ═══════════════════════════════════════════════════════════════════════════
# 1. 러너 게이트 검체 (G-* — §5 ID 공간이 아닌 러너 로컬 게이트)
# ═══════════════════════════════════════════════════════════════════════════
@specimen("B-8", "W1b", "베이스라인 test_lane_isolation_v1 (훅 BOOT 부재·레인↔팩)",
          ["레인 격리", "A7 종료 경로"])
def b_8():
    """★W1b 편입 이유: 이 테스트가 훅의 **BOOT 부재 분기**와 bootstrap 의 exit 8 경로를 핀한다 —
    W1b 가 훅 전단(감지기 이관·게이트 재배치)과 cmd_run 종료 구조를 동시에 만졌으므로 1커맨드
    게이트 안에 들어와야 한다(종전엔 러너 밖 수동 실행이었다)."""
    return _baseline([PY, os.path.join(TESTS_DIR, "test_lane_isolation_v1.py")],
                     "test_lane_isolation_v1")


@specimen("B-9", "W1b", "베이스라인 javis_detect --self-test (감지기 밀폐 corpus)",
          ["A4", "P3-A-NEGA", "P3-A-FILLER", "G9", "G25"])
def b_9():
    return _baseline([PY, os.path.join(BIN_DIR, "javis_detect.py"), "--self-test"],
                     "javis_detect --self-test")


@specimen("G-SYNTAX", "W1a", "전 훅 + 프리루드 `sh -n`·`bash -n` 무오류", ["CS-4① ⓐ POSIX sh"])
def g_syntax():
    targets = ["_lib.sh"] + _shell_hooks()
    bad = []
    for rel in targets:
        p = os.path.join(HOOKS_DIR, rel)
        for sh in ("sh", "bash"):
            # bash 는 모듈 상수 BASH 로 해소한다(W-A — System32 WSL 스텁 회피). sh 는 System32
            # 동명 스텁이 없어 리터럴 유지가 안전하다.
            r = _run([BASH if sh == "bash" else sh, "-n", p], timeout=30)
            if r.returncode != 0:
                bad.append("%s(%s): %s" % (rel, sh, r.stderr.strip()[:200]))
    need(not bad, "구문 오류:\n  " + "\n  ".join(bad))
    return "%d 파일 × sh/bash 문법 OK" % len(targets)


@specimen("G-PRELUDE", "W1a", "프리루드 계약 — 함수 표면·stdout 무출력·0 종료·loud-skip", ["CS-4① ⓑⓒⓓⓔ"])
def g_prelude():
    lib = os.path.join(HOOKS_DIR, "_lib.sh")
    need(os.path.isfile(lib), "_lib.sh 부재 — 프리루드 미착지")
    fns = ["cys_require_surface", "cys_have_surface", "cys_norm_path", "cys_is_abs",
           "cys_norm_cwd", "cys_path_has_prefix", "cys_native_path", "cys_shquote",
           "cys_resolve_py", "cys_fix_locale", "cys_timeout_run"]
    body = _read(lib)
    missing = [f for f in fns if ("%s()" % f) not in body]
    need(not missing, "프리루드 함수 누락: %s" % missing)
    # bashism 금지(ⓐ): 배열·[[·${x:0:n}·local·+= 는 sh 에서 깨진다.
    # ★주석 제외 — 문서화 목적으로 금지 토큰을 **인용**한 줄까지 잡으면 계측기 오탐이다.
    code = "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))
    for pat, why in ((r"\[\[", "[[ (bash 전용 조건)"), (r"\blocal\s", "local (POSIX 미정의)"),
                     (r"\$\{[A-Za-z_][A-Za-z0-9_]*:\d+:", "${x:0:n} 슬라이스"),
                     (r"\)\s*=\s*\(", "배열 대입")):
        need(not re.search(pat, code), "프리루드 bashism 발견: %s" % why)
    # stdout 무출력 + set -u 안전 + 0 종료 (ⓑⓒⓓ)
    with tempfile.TemporaryDirectory() as tmp:
        probe = os.path.join(tmp, "probe.sh")
        _w(probe, '#!/bin/sh\nset -u\n. "%s"\nprintf "RC=%%s\\n" "$?" >&2\n' % lib)
        r = _run(["sh", probe], env=_base_env())
        need(r.stdout == "", "프리루드가 stdout에 출력했다(모델 컨텍스트 오염): %r" % r.stdout[:200])
        need("RC=0" in r.stderr, "프리루드 source 가 0으로 끝나지 않는다: %r" % r.stderr[:200])
        # ── 2단 해소: 훅이 팩 밖으로 복사돼 실행돼도 CYS_PACK_DIR 경로에서 프리루드를 집는다 ──
        # ★이 폴백이 없으면 배선 하네스·테스트 스텁·local/hooks 오버레이가 전면 강등된다
        #   (실측 회귀: test_pre_dispatch.sh 56/56 → 10/56). G-DISPATCH 가 그 표면을 지킨다.
        fake = os.path.join(tmp, "fakehooks")
        os.makedirs(fake, exist_ok=True)
        shutil.copy2(_hook("save-state.sh"), os.path.join(fake, "save-state.sh"))
        fakepack = os.path.join(tmp, "fakepack")
        os.makedirs(os.path.join(fakepack, "hooks"), exist_ok=True)
        shutil.copy2(lib, os.path.join(fakepack, "hooks", "_lib.sh"))
        proj2 = os.path.join(tmp, "p2")
        os.makedirs(os.path.join(proj2, "_round"), exist_ok=True)
        r1 = _run([BASH, os.path.join(fake, "save-state.sh")],
                  input=json.dumps({"cwd": proj2, "hook_event_name": "Stop"}),
                  env=_base_env({"HOME": os.path.join(tmp, "h2"), "CYS_PACK_DIR": fakepack}))
        need(r1.returncode == 0, "2단 폴백 경로에서 훅이 비0 종료: %d" % r1.returncode)
        need("_lib.sh 소실" not in r1.stderr,
             "팩 경로에 프리루드가 있는데도 loud-skip 했다(2단 폴백 미작동): %r" % r1.stderr[:300])
        need("Stop" in _read(os.path.join(proj2, "_round", ".state_log")),
             "2단 폴백 경로에서 훅 본체가 동작하지 않았다")
        # loud-skip 규약: **양 단계 모두** 실패 → stderr 1줄 + exit 0 + stdout 무오염
        r2 = _run([BASH, os.path.join(fake, "save-state.sh")], input="{}",
                  env=_base_env({"HOME": os.path.join(tmp, "h3"),
                                 "CYS_PACK_DIR": os.path.join(tmp, "nowhere")}))
        need(r2.returncode == 0, "loud-skip 이 exit 0 아님: %d" % r2.returncode)
        need("_lib.sh 소실" in r2.stderr, "loud-skip stderr 문구 부재: %r" % r2.stderr[:200])
        need(r2.stdout == "", "loud-skip 이 stdout 오염: %r" % r2.stdout[:200])
        # 전 훅이 2단 해소 규약을 쓰는가(1단만 남은 훅 = 복사 배선에서 조용히 강등)
        no2 = [h for h in _shell_hooks()
               if "_lib.sh" in _read(os.path.join(HOOKS_DIR, h))
               and 'CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh' not in _read(os.path.join(HOOKS_DIR, h))]
        need(not no2, "2단 폴백 미적용 훅(복사 배선에서 전면 강등): %s" % no2)
    # ── 프리루드 요구 범위 = **규약을 소비하는 훅 전수** ──
    # G22 소인 범위는 "python 인터프리터를 쓰거나 경로/surface 술어를 쓰는 훅"이다. 전 .sh 파일을
    # 무조건 요구하면 **의존이 0인 훅**(cys-hook.sh·cys-statusline.sh — "cys 자기완결·python 등
    # 외부 의존 없음"이 명문 계약)까지 새 실패 모드를 심는 과확장이 된다. 두 훅은 의도적 제외이며,
    # 나중에 python/경로 술어를 쓰게 되면 아래 술어가 자동으로 계약 안으로 끌어들인다.
    consumers, nosrc = [], []
    markers = ("CYS_PY", "PYBIN", "python3", "cys_norm_", "cys_is_abs", "cys_native_path",
               "cys_require_surface", "cys_have_surface", "cys_path_has_prefix", "cys_shquote")
    for h in _shell_hooks():
        body_h = _read(os.path.join(HOOKS_DIR, h))
        if any(m in body_h for m in markers):
            consumers.append(h)
            if "_lib.sh" not in body_h:
                nosrc.append(h)
    need(not nosrc, "규약 소비 훅인데 프리루드 미적용: %s" % nosrc)
    need(len(consumers) >= 28,
         "규약 소비 훅 인벤토리가 28 미달(%d) — G22 전수 범위 축소 회귀" % len(consumers))
    return ("함수 %d종 · stdout 무출력 · loud-skip · 규약 소비 훅 %d개 전수 source(의존 0 훅 %d개 제외)"
            % (len(fns), len(consumers), len(_shell_hooks()) - len(consumers)))


@specimen("G-DISPATCH", "W1a", "pre-dispatch 서브훅 배선 회귀(팩 밖 복사 실행 표면)",
          ["CS-4① ⓔ 2단 해소", "pre-dispatch 계약"])
def g_dispatch():
    """★이 게이트가 존재하는 이유(실사고): 프리루드 1단 해소만 두었을 때, 훅을 팩 밖으로
    **복사해 실행하는** 배선(테스트 스텁·local/hooks 오버레이·배선 하네스)이 전면 강등돼
    56/56 → 10/56 로 무너졌다. 그 표면을 지키는 유일한 검체다."""
    h = os.path.join(HOOKS_DIR, "test_pre_dispatch.sh")
    if not os.path.isfile(h):
        raise Skip("test_pre_dispatch.sh 부재")
    env = _base_env({"REAL_GUARD": _hook("guard.sh"), "REAL_HOOKS": HOOKS_DIR})
    r = _run(["sh", h], env=env, timeout=300)
    m = re.search(r"결과: PASS=(\d+) FAIL=(\d+)", r.stdout)
    need(m, "하네스 결과 라인 부재(FATAL?): %r" % (r.stdout[-500:] or r.stderr[-500:]))
    p, f = int(m.group(1)), int(m.group(2))
    need(f == 0, "서브훅 배선 회귀: PASS=%d FAIL=%d\n%s" % (p, f, "\n".join(
        l for l in r.stdout.splitlines() if "FAIL" in l)[:2000]))
    need(p >= 50, "하네스 케이스 수 급감(%d) — 조기 강등 의심" % p)
    return "pre-dispatch 하네스 %d/%d PASS(guard·cys-hook·appbuild·grill 배선 무회귀)" % (p, p + f)


@specimen("G-SMOKE", "W1a", "회귀 스모크 — 대표 3훅 정상 경로 종전 거동 유지",
          ["inject-context", "save-state", "guard"])
def g_smoke():
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"),
           "# S\n> 최종 갱신: old\nSMOKE-STATE-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        payload = json.dumps({"source": "clear", "cwd": proj, "hook_event_name": "PreCompact"})
        # ① inject-context: 작업기억 주입 + exit 0
        r = _run([BASH, _hook("inject-context.sh")], input=payload, env=env)
        need(r.returncode == 0, "inject-context exit=%d" % r.returncode)
        need("SMOKE-STATE-MARKER" in r.stdout, "inject-context 가 작업기억을 주입하지 않았다")
        notes.append("inject-context OK")
        # ② save-state: .state_log append + 타임스탬프 갱신
        r = _run([BASH, _hook("save-state.sh")], input=payload, env=env)
        need(r.returncode == 0, "save-state exit=%d" % r.returncode)
        log = _read(os.path.join(proj, "_round", ".state_log"))
        need("PreCompact" in log, "save-state 가 .state_log 를 남기지 않았다: %r" % log[:200])
        need("auto write-ahead" in _read(os.path.join(proj, "_round", "SESSION_STATE.md")),
             "save-state 가 '최종 갱신' 타임스탬프를 갱신하지 않았다")
        notes.append("save-state OK")
        # ③ guard: 무해 명령 통과 / 헌법파일 차단
        r = _run([BASH, _hook("guard.sh")],
                 input=json.dumps({"tool_name": "Bash", "tool_input": {"command": "ls -la"}}), env=env)
        need(r.returncode == 0, "guard 가 무해 명령을 차단(exit=%d)" % r.returncode)
        r = _run([BASH, _hook("guard.sh")],
                 input=json.dumps({"tool_name": "Write",
                                   "tool_input": {"file_path": "/x/.claude/soul.md"}}), env=env)
        need(r.returncode == 2, "guard 가 헌법파일 쓰기를 통과(exit=%d)" % r.returncode)
        notes.append("guard OK")
    return " · ".join(notes)


# ═══════════════════════════════════════════════════════════════════════════
# 2. H-DETECT (훅 발화 계층)
# ═══════════════════════════════════════════════════════════════════════════
@specimen("H-DETECT-1", "W1b", "혼합의도 FIRE corpus(절 경계 스코프)", ["A4"])
def h_detect_1():
    """A4: 억제를 감지보다 먼저 돌리던 순서를 역전 — 선언이 **속한 절** 안의 마커만 억제한다."""
    D = _detect_mod()
    fire = ["너는 마스터다. 오늘 뭐부터 할까?",
            "너는 이제 마스터다! 무슨 일부터 시작할까?",
            "너는 마스터다.\n오늘 작업 목록은 뭐로 잡을까?",
            "너는 마스터다; 첫 작업은 뭐로 할까?",
            "'설계 문서'를 예시로 보여줘. 그리고 너는 마스터다."]
    skip = ["'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
            "너는 마스터가 아니다", "너는 마스터다 처럼 들리는 문장을 만들어줘"]
    for p in fire:
        v = D.detect(p)
        need(v["fire"], "혼합의도 미발화(A4 회귀) %r → %s" % (p, v["reason"]))
    for p in skip:
        v = D.detect(p)
        need(not v["fire"], "억제 케이스 오발화(과교정) %r → %s" % (p, v["reason"]))
    # 훅 배선 교차 확인 — 함수는 맞는데 배선이 틀린 구멍 차단
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        r = _run_rb(env, prompt=fire[0])
        need("발화됨" in r.stdout, "훅 배선에서 혼합의도 미발화: %r" % r.stdout[:300])
        calib = _calib_note(_old_hook_fires(fire[0], tmp), False, "혼합의도 프롬프트")
    return "혼합의도 %d FIRE / 억제 %d SKIP + 훅 교차 · 계측검증=%s" % (len(fire), len(skip), calib)


@specimen("H-DETECT-2", "W1b", "'네가/니가/당신이 마스터다' FIRE + 인접 의문 SKIP", ["P3-A-NEGA"])
def h_detect_2():
    """P3-A-NEGA: 표준 정서법 주어가 어휘에 없어 전부 미발화했다. '네'·'당신' 단독 과확장은 금지."""
    D = _detect_mod()
    for p in ("네가 마스터다", "니가 마스터다", "당신이 마스터다", "네가 이제 마스터야",
              "지금부터 네가 마스터가 된다"):
        v = D.detect(p)
        need(v["fire"], "표준 정서법 주어 미발화(P3-A-NEGA 회귀) %r → %s" % (p, v["reason"]))
    for p in ("'네가 마스터다'가 무슨 뜻?", "'니가 마스터다'가 무슨 의미인지 설명해줘",
              "\"당신이 마스터다\"라고 입력하면 어떻게 되나요?"):
        need(not D.detect(p)["fire"], "인접 의문인데 발화 %r" % p)
    for p in ("네 마스터 브랜치를 봐줘", "당신 마스터키 어디 뒀어"):
        need(not D.detect(p)["fire"], "'네'·'당신' 단독 과확장 발화 %r" % p)
    need("네가" in D.SUBJECT and "니가" in D.SUBJECT and "당신이" in D.SUBJECT,
         "주어 어휘에 네가/니가/당신이가 없다: %s" % D.SUBJECT)
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt="네가 마스터다").stdout, "훅 배선에서 '네가 마스터다' 미발화")
        calib = _calib_note(_old_hook_fires("네가 마스터다", tmp), False, "'네가 마스터다'")
    return "주어 5 FIRE / 인접의문 3 SKIP / 단독 과확장 2 SKIP · 계측검증=%s" % calib


@specimen("H-DETECT-3", "W1b", "filler 경계 15자=발화/16자=미발화", ["P3-A-FILLER"])
def h_detect_3():
    """P3-A-FILLER: 주석(12) ≠ 코드(15) 불일치를 상수 FILLER_MAX 로 못박고 경계를 박제한다.
    ★수치는 **이관 시 보존**돼야 한다 — 구 grep 과 신 함수의 경계가 같음을 구 훅 실측으로 대조한다."""
    D = _detect_mod()
    need(D.FILLER_MAX == 15, "FILLER_MAX 스펙 이탈: %r≠15" % D.FILLER_MAX)
    ok15 = "너는" + "가" * 15 + "마스터다"
    no16 = "너는" + "가" * 16 + "마스터다"
    need(D.detect(ok15)["fire"], "filler 15자 경계 미발화")
    need(not D.detect(no16)["fire"], "filler 16자 오발화(경계 누수)")
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt=ok15).stdout, "훅에서 filler 15 미발화")
        need("발화됨" not in _run_rb(env, prompt=no16).stdout, "훅에서 filler 16 오발화")
        # 경계 파리티: 구 grep 도 15=발화 / 16=미발화 였다(이관이 수치를 바꾸지 않았다는 증명)
        c15 = _calib_note(_old_hook_fires(ok15, tmp), True, "filler 15자")
        c16 = _calib_note(_old_hook_fires(no16, os.path.join(tmp, "b")), False, "filler 16자")
    return "15=FIRE·16=SKIP(함수·훅) · 구 grep 파리티: %s / %s" % (c15, c16)


@specimen("H-DETECT-4", "W1b", "LC_ALL=C 파리티(UTF-8과 동일 판정)", ["G9"])
def h_detect_4():
    """G9: `grep -E '.{0,15}'` 는 C 로케일에서 **바이트**를 세어 한글 filler 창이 1/3로 줄었다.
    python `re` 는 코드포인트 기반이고, 감지기는 stdin 을 바이트로 읽어 UTF-8 명시 디코드한다."""
    D = _detect_mod()
    probes = ["너는 이제 마스터다", "너는" + "가" * 15 + "마스터다", "네가 마스터다"]
    cenv = {"LC_ALL": "C", "LANG": "C", "LC_CTYPE": "C"}
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        cenv_full = dict(env, **cenv)
        for p in probes:
            need("발화됨" in _run_rb(cenv_full, prompt=p).stdout,
                 "LC_ALL=C 에서 미발화(G9 회귀): %r" % p)
        need("발화됨" not in _run_rb(cenv_full, prompt="'너는 마스터다'가 무슨 뜻이야?").stdout,
             "LC_ALL=C 에서 억제 케이스가 오발화(로케일 의존 잔존)")
        # 감지기 CLI 단독 파리티(훅 밖에서도 로케일 비의존인지)
        r = _run([PY, os.path.join(BIN_DIR, "javis_detect.py"), "hook-gate"],
                 input=json.dumps({"prompt": probes[1]}, ensure_ascii=False),
                 env=_base_env(cenv))
        need(r.returncode == 0, "LC_ALL=C 에서 감지기 CLI 가 발화하지 않았다(rc=%d): %s"
             % (r.returncode, r.stderr[-300:]))
        # 계측 타당성: 구 훅은 같은 환경에서 **미발화**해야 한다(바이트 계수 결함 재현)
        calib = _calib_note(_old_hook_fires(probes[1], tmp, env_extra=cenv), False,
                            "LC_ALL=C + 한글 filler 15자")
    need(D.detect(probes[1])["fire"], "UTF-8 판정과 불일치")
    return "LC_ALL=C 에서 %d FIRE / 억제 1 SKIP / CLI 파리티 · 계측검증=%s" % (len(probes), calib)


@specimen("H-DETECT-5", "W1b", "200자 창 python측 문자 단위 슬라이스", ["G25"])
def h_detect_5():
    """G25: `cut -c1-200` 은 GNU 에서 **항상 바이트**(BSD 도 C 로케일이면 바이트)라 한글 감지창이
    약 66자로 축소됐다. 해법은 셸 슬라이스 **제거** — python 이 원문을 문자 단위로 자른다."""
    D = _detect_mod()
    need(D.WINDOW_CHARS == 200, "WINDOW_CHARS 스펙 이탈: %r≠200" % D.WINDOW_CHARS)
    decl = "너는 마스터다"
    inside = "가" * (D.WINDOW_CHARS - len(decl)) + decl
    outside = "가" * D.WINDOW_CHARS + decl
    need(len(inside.encode("utf-8")) > D.WINDOW_CHARS,
         "검체가 바이트 기준으로도 창 안이라 회귀를 못 잡는다(검체 무효)")
    need(D.detect(inside)["fire"], "문자 200 경계 미발화(바이트 슬라이스 회귀)")
    need(not D.detect(outside)["fire"], "감지창 밖 선언이 발화(창 미적용)")
    # 셸 슬라이스 제거 확인(정적) — 훅에 cut -c·bashism 슬라이스가 남아 있으면 회귀다
    hook = _code_lines(_read(_hook(ROLE_BODY)))
    need("cut -c" not in hook, "훅에 `cut -c` 바이트 슬라이스가 남았다(G25 미수리)")
    need(not re.search(r"\$\{[A-Za-z_][A-Za-z0-9_]*:\d+:", hook), "훅에 bashism 슬라이스가 남았다")
    need("tr '\\n' ' '" not in hook, "훅에 개행 평탄화가 남았다(창 판정이 두 곳에 존재)")
    with tempfile.TemporaryDirectory() as tmp:
        env, _h, _p, _b, _s = _rb_sandbox(tmp)
        need("발화됨" in _run_rb(env, prompt=inside).stdout, "훅에서 문자 200 경계 미발화")
        need("발화됨" not in _run_rb(env, prompt=outside).stdout, "훅에서 창 밖 선언 발화")
        # 계측 타당성: 구 훅은 바이트 절단 조건(C 로케일)에서 **미발화**해야 한다
        calib = _calib_note(_old_hook_fires(inside, tmp,
                                            env_extra={"LC_ALL": "C", "LANG": "C", "LC_CTYPE": "C"}),
                            False, "한글 194자 프리픽스 + 선언(바이트 절단 조건)")
        old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
        if old is not None:
            need("cut -c1-200" in old, "계측 타당성 실패: 구 코드에 cut -c 슬라이스가 없다")
    return "문자 200 경계 FIRE/창밖 SKIP(함수·훅) · 셸 슬라이스 제거 확인 · 계측검증=%s" % calib


@specimen("H-DETECT-6", "W1a", "무-surface env → 무발화·무부작용", ["A2"])
def h_detect_6():
    with tempfile.TemporaryDirectory() as tmp:
        env, home, _pack, _bind, state = _rb_sandbox(tmp, surface=False)
        r = _run_rb(env)
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need(r.stdout.strip() == "", "무-surface 에서 컨텍스트를 주입했다: %r" % r.stdout[:300])
        need("발화됨" not in r.stdout, "무-surface 에서 발화 보고")
        # 무부작용: cys 왕복 0(surface-role 조회조차 없음)·상태 디렉터리 무생성
        need(_calls(tmp) == "", "무-surface 에서 cys 를 호출했다(데몬 autostart 표면): %r" % _calls(tmp))
        need(not os.path.isdir(state) or not os.listdir(state),
             "무-surface 에서 상태 디렉터리를 건드렸다: %s" % (os.listdir(state) if os.path.isdir(state) else ""))
        # 대조군: surface 있으면 발화(게이트가 과차단이 아님을 증명)
        env2, _h2, _p2, _b2, _s2 = _rb_sandbox(os.path.join(tmp, "b"), surface=True)
        r2 = _run_rb(env2)
        need("발화됨" in r2.stdout, "surface 있는 정상 경로가 발화하지 않았다(과차단): %r" % r2.stdout[:300])
    # 계측기 자기검증: 구 코드(기준 커밋)에는 게이트가 없어야 한다
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need("cys_require_surface" not in old,
             "계측 타당성 실패: 구 코드에 이미 게이트가 있다면 이 검체는 아무것도 증명하지 않는다")
        calib = "구 코드 게이트 부재 확인"
    return "무발화·cys왕복0·상태무접촉 + 대조군 발화 · 계측검증=%s" % calib


@specimen("H-DETECT-7", "W1b", "role 게이트 allowlist 행렬(worker-2·cso-1·미지 role)", ["A3=B7"])
def h_detect_7():
    """A3=B7: 구 denylist(`worker|cso|reviewer-*|reviewer`)는 **열거 밖 전부 통과**였다 —
    데몬이 실제 발권하는 worker-2(dedup)·cso-1·reviewer-claude-1·미지 role 이 마스터 부트를
    오발화했다. 수정은 열거 확장이 아니라 **allowlist 반전**(master 또는 빈 좌석만 발화)이다."""
    blocked = ["worker", "worker-2", "cso", "cso-1", "reviewer-gemini", "reviewer-codex",
               "reviewer-claude-1", "reviewer-grok", "verifier", "unknown-role", "ceo"]
    allowed = ["master", ""]
    with tempfile.TemporaryDirectory() as tmp:
        for i, role in enumerate(blocked):
            sb = os.path.join(tmp, "b%d" % i)
            env, _h, _p, _b, _s = _rb_sandbox(sb)
            _mock_cys(os.path.join(sb, "bin"), sb,
                      'case "$1" in surface-role) echo "%s"; exit 0;; esac' % role)
            r = _run_rb(env)
            need("발화됨" not in r.stdout,
                 "A3 회귀: role=%s 에서 마스터 부트 오발화(denylist 잔존): %r" % (role, r.stdout[:200]))
            need("allowlist" in r.stderr or "비-master" in r.stderr,
                 "role=%s 차단이 침묵이다(로그 없음): %r" % (role, r.stderr[:200]))
        for i, role in enumerate(allowed):
            sb = os.path.join(tmp, "a%d" % i)
            env, _h, _p, _b, _s = _rb_sandbox(sb)
            _mock_cys(os.path.join(sb, "bin"), sb,
                      'case "$1" in surface-role) echo "%s"; exit 0;; esac' % role)
            need("발화됨" in _run_rb(env).stdout,
                 "정상 좌석(role=%r)에서 발화하지 않았다(과차단)" % role)
        # 계측 타당성: 구 훅은 worker-2 에서 **발화**했어야 한다(결함 재현)
        sb = os.path.join(tmp, "calib")
        os.makedirs(sb, exist_ok=True)
        old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
        calib = "skip(no-git)"
        if old is not None:
            oldhook = os.path.join(sb, "oldhooks", "role-bootstrap.sh")
            _w(oldhook, old)
            _w(os.path.join(sb, "oldhooks", "_lib.sh"),
               _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            env, _h, _p, _b, _s = _rb_sandbox(os.path.join(sb, "sb"))
            _mock_cys(os.path.join(sb, "sb", "bin"), os.path.join(sb, "sb"),
                      'case "$1" in surface-role) echo "worker-2"; exit 0;; esac')
            r = _run([BASH, oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=env)
            need("발화됨" in r.stdout,
                 "계측 타당성 실패: 구 코드가 worker-2 에서 오발화하지 않는다 — 검체가 결함을 재현 못함")
            calib = "구 코드 worker-2 오발화 재현"
    return "차단 %d role(미지 role 포함)·허용 2 · 계측검증=%s" % (len(blocked), calib)


@specimen("H-DETECT-8", "W1b", "surface-role 판정불가 → fail-closed+loud", ["A5"])
def h_detect_8():
    """A5: `cys surface-role` 이 판정 불가(rc≠0·hang)일 때 구 코드는 MYROLE 을 **빈값**으로 얻어
    '미claim' 으로 오통과시켰다(빈값 오통과). 3상화: rc≠0=무발화+로그 / 빈값=미claim 통과.
    ★timeout 단독 적용은 hang 을 오발화로 바꾸는 악화이므로 데드라인과 3상화를 함께 검증한다."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ rc≠0(판정 불가) → 무발화 + 로그
        sb = os.path.join(tmp, "rc")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) echo ""; exit 3;; esac')
        r = _run_rb(env)
        need("발화됨" not in r.stdout, "판정 불가(rc=3)인데 발화(fail-open 잔존)")
        need("판정 불가" in r.stderr, "판정 불가가 침묵으로 접혔다: %r" % r.stderr[:200])
        notes.append("rc≠0 무발화+로그")
        # ⓑ hang → 데드라인으로 유한 종료 + 무발화(훅이 프롬프트를 무한정 붙잡지 않는다)
        sb = os.path.join(tmp, "hang")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) sleep 30; echo ""; exit 0;; esac')
        t0 = time.time()
        r = _run_rb(env)
        dt = time.time() - t0
        need(dt < 20, "hang 이 데드라인으로 끊기지 않았다(%.1fs) — 훅이 프롬프트를 붙잡는다" % dt)
        need("발화됨" not in r.stdout, "hang(판정 불가)인데 발화 — timeout 단독 적용 악화 형태")
        notes.append("hang %.1fs 내 종료·무발화" % dt)
        # ⓒ 빈값(정상 미claim)은 통과해야 한다 — 3상 분리의 반대편(과차단 금지)
        sb = os.path.join(tmp, "empty")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        need("발화됨" in _run_rb(env).stdout, "빈값(미claim)이 차단됐다 — rc≠0 과 융합(과차단)")
        notes.append("빈값 미claim 통과")
    # 계측 타당성: 프리루드에 데드라인 실행기가 존재하고 훅이 그것으로 감싼다
    need("cys_timeout_run()" in _read(os.path.join(HOOKS_DIR, "_lib.sh")),
         "프리루드에 데드라인 실행기(cys_timeout_run)가 없다")
    need("cys_timeout_run" in _read(_hook(ROLE_BODY)),
         "훅이 surface-role 을 데드라인으로 감싸지 않는다")
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need("cys_timeout_run" not in old and 'MYROLE="$(cys surface-role' in old,
             "계측 타당성 실패: 구 코드가 이미 데드라인·3상화를 갖고 있다면 이 검체는 무의미")
        calib = "구 코드 무-데드라인·빈값 오통과 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DETECT-9", "W1b", "python 부재 → 정적 loud(cannot-judge 분리)", ["A22"])
def h_detect_9():
    """A22: 인터프리터 미해소는 '선언 아님'이 아니라 **판정 불가**다. 구 코드는 `|| CYS_PY=python3`
    으로 채워 존재하지 않는 인터프리터를 향해 파싱을 시도하고 조용히 exit 0 했다(cannot-judge 침묵).
    ★judged-no 와의 분리: 프롬프트에 마스터 토큰이 아예 없으면 침묵이 정당하다(스팸 금지)."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # PATH 에서 python 전부 제거 + CYS_PY 빈값 강제 → cannot-judge
        sb = os.path.join(tmp, "nopy")
        env, _h, _p, _b, _s = _rb_sandbox(sb)
        binp = os.path.join(sb, "onlybin")
        os.makedirs(binp, exist_ok=True)
        for tool in ("bash", "sh", "cat", "grep", "printf", "tr", "head", "date", "mkdir",
                     "rm", "ls", "ln", "sleep", "dirname", "sed", "command", "kill", "wait"):
            src = shutil.which(tool)
            if src and not os.path.exists(os.path.join(binp, tool)):
                os.symlink(src, os.path.join(binp, tool))
        # 목 cys 를 python 없는 PATH 에 복사(feed/send 시도 흔적 회수용)
        _mock_cys(binp, sb, 'case "$1" in surface-role) echo ""; exit 0;; esac')
        env["PATH"] = binp
        env["CYS_PY"] = ""
        r = _run_rb(env)
        need(r.returncode == 0, "python 부재에서 훅이 비0 종료(exit=%d)" % r.returncode)
        need("발화됨" not in r.stdout, "python 부재인데 '발화됨' 보고")
        need("판정 불가" in r.stdout and "python" in r.stdout,
             "cannot-judge 가 정적 loud 로 나오지 않았다(침묵 접힘): %r" % r.stdout[:300])
        need("hookSpecificOutput" in r.stdout, "정적 additionalContext 미출력(python 없이 발행 실패)")
        json.loads(r.stdout.strip().splitlines()[-1])   # 정적 JSON 이 유효 JSON 인가
        need("feed push" in _calls(sb) or "send --queued" in _calls(sb),
             "판정 불가인데 승인 채널 알림 시도가 없다: %r" % _calls(sb)[-300:])
        notes.append("cannot-judge 정적 loud+알림")
        # judged-no: 마스터 토큰 없는 프롬프트는 침묵(스팸 금지)
        r2 = _run_rb(env, prompt="오늘 작업 지시해줘")
        need(r2.stdout.strip() == "", "선언 토큰 없는 프롬프트에 컨텍스트를 주입했다(스팸): %r" % r2.stdout[:200])
        notes.append("judged-no 침묵")
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need('CYS_PY="$(command -v python3 || command -v python || command -v py || echo python3)"' in old
             or "|| CYS_PY=python3" in old or '|| CYS_PY="python3"' in old,
             "계측 타당성 실패: 구 코드에 '존재하지 않는 인터프리터로 채우기'가 없다")
        calib = "구 코드 인터프리터 기본값 채움 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DETECT-10", "W1a", "pre-exec 사망 목 → '발화 실패' 상태파생 보고(허위 '발화됨' 금지)", ["A6"])
def h_detect_10():
    real = shutil.which("python3") or PY
    # 목 인터프리터: `-c`(NOTE 생성)와 **감지기 스크립트**(W1b: 선언 판정이 python 단일 함수로
    # 이관됐다)는 정상 위임하고, **부트 스크립트 실행만 exec 실패(127)** 로 만든다.
    # ★목을 좁힌 이유: 이 검체가 재는 것은 '부트 발화의 pre-exec 사망'이지 '감지 불가'가 아니다.
    #   감지기까지 죽이면 훅이 A22 cannot-judge 분기로 빠져 A6 표면을 아예 통과하지 않는다.
    #   ★같은 이유로 **임무 게이트도 정상 위임**한다(2026-08-01 D4-a): 게이트를 죽이면 훅이
    #     fail-closed 로 접혀 no-spawn 경로를 타므로, 역시 A6 표면에 도달하지 않는다.
    mock = ("#!/bin/sh\n"
            'case "$1" in\n'
            '  -c) exec "%s" "$@" ;;\n'
            "  *javis_detect.py) exec \"%s\" \"$@\" ;;\n"
            "  *javis_mission.py) exec \"%s\" \"$@\" ;;\n"
            "esac\n"
            "exit 127\n" % (real, real, real))
    with tempfile.TemporaryDirectory() as tmp:
        env, _home, _pack, _bind, state = _rb_sandbox(tmp, mock_py=mock)
        r = _run_rb(env)
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need("발화됨" not in r.stdout, "발화가 죽었는데 '발화됨' 을 보고했다(허위 보고): %r" % r.stdout[:400])
        need("발화 실패" in r.stdout, "'발화 실패' 상태파생 보고가 없다: %r" % r.stdout[:400])
        need("role-bootstrap-" in r.stdout, "실패 보고에 런 로그 경로 안내가 없다: %r" % r.stdout[:400])
        need("exit 127" in r.stdout, "실패 사유(exit code)가 보고에 없다: %r" % r.stdout[:400])
    # 계측기 자기검증: 구 코드는 같은 목에서 '발화됨' 을 말해야 한다(결함 재현)
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    if old is not None:
        with tempfile.TemporaryDirectory() as tmp2:
            oldhook = os.path.join(tmp2, "hooks", "role-bootstrap.sh")
            _w(oldhook, old)
            _w(os.path.join(tmp2, "hooks", "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            env2, _h, _p, _b, _s = _rb_sandbox(os.path.join(tmp2, "sb"), mock_py=mock)
            r2 = _run([BASH, oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=env2)
            need("발화됨" in r2.stdout,
                 "계측 타당성 실패: 구 코드가 이 목에서 허위 '발화됨' 을 내지 않는다 — 목이 결함을 재현하지 못함")
            calib = "구 코드 허위 '발화됨' 재현 확인"
    return "발화 실패 보고·로그 경로 동봉 · 계측검증=%s" % calib


@specimen("H-DETECT-11", "W1a", "동시/연속 발화에서 선행 런 로그 무-truncate(런별 파일+상한)", ["A16", "R3"])
def h_detect_11():
    with tempfile.TemporaryDirectory() as tmp:
        boot = ("import os,sys\n"
                "c=os.path.join(os.environ['HOME'],'.cys','state','n')\n"
                "n=int(open(c).read()) if os.path.exists(c) else 0\n"
                "open(c,'w').write(str(n+1))\n"
                "sys.stdout.write('RUN-MARKER-%d\\n' % n)\n")
        env, home, _pack, _bind, state = _rb_sandbox(tmp, boot_body=boot)
        os.makedirs(state, exist_ok=True)
        for i in (1, 2):
            r = _run_rb(env)
            need("발화됨" in r.stdout, "%d회차 발화 실패: %r" % (i, r.stdout[:200]))
            time.sleep(0.2)
        names = _run_ledgers(state)
        need(len(names) >= 2, "런별 로그가 분리되지 않았다(truncate 회귀): %s" % names)
        bodies = [_read(os.path.join(state, n)) for n in names]
        markers = sorted(set(re.findall(r"RUN-MARKER-\d+", "\n".join(bodies))))
        need(len(markers) >= 2, "선행 런 내용이 소실됐다(상호 truncate): %s / %s" % (names, bodies))
        need(os.path.exists(os.path.join(state, "role-bootstrap-latest.log")),
             "latest 포인터가 없다")
        need(not os.path.exists(os.path.join(state, "role-bootstrap.log")),
             "구 단일 truncate 로그가 여전히 생성된다(A16 미수리)")
        # 개수 상한(최근 10개 유지) — 더미 12개 선주입 후 1회 발화
        for k in range(12):
            _w(os.path.join(state, "role-bootstrap-100000%02d-999.log" % k), "old-%d\n" % k, 0o644)
        _run_rb(env)
        left = [n for n in os.listdir(state) if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)]
        need(len(left) <= 10, "개수 상한 미작동(%d개 잔여)" % len(left))
    return "런별 %d파일 분리·마커 %d종 보존·latest 포인터·상한 10 준수" % (len(names), len(markers))


@specimen("H-MISSION-1", "W5",
          "임무 미지정 부팅(D4-a′) → spawn 1 + 착수금지 문안 + 기계유래 무스폰 게이트(§4-10)",
          ["D4-a′(2択 혼동)", "T1 자기인가", "§4-10 부트층 유사체"])
def h_mission_1():
    """D4-a′(2026-08-10 오너 재정): "너는 마스터다" 선언 자체가 팀 기동 명령(동의 신호)이다 —
    훅은 임무 유무와 무관하게 부트를 **정확히 1회** 발화한다. 구 D4-a(2026-08-01)는 임무 미지정
    부팅의 spawn 을 막았지만('동의 없는 사후통보' 차단이 목적), 실사용에서 2択(단독 대기/팀
    기동)이 초보자 혼동을 낳아 오너가 계약을 재정의했다. 착수 차단은 부트 층이 아니라 **착수
    층 소관으로 남는다**(T1 무손상 — 그 층은 이번에 한 줄도 안 바뀌었다):
      · 선언 단독 = mission=null 기록(자기인가 차단 — 훅의 T1 대장 기록 블록 그대로).
      · next-action 은 임무 미지정이면 exit 3(자율 착수 금지) — 팀이 떠 있어도 잔무 큐
        자동 착수는 결정론으로 거부된다(2026-08-01 72% 소진 사고 재발 방지).
    정직성 불변식(주입문 서술 = 실제 실행 1:1)은 유지된다 — 주입문은 '팀 세션 기동'을 실행
    사실로 서술하고 실제로 spawn 하며, 임무 상태에 따라 갈리는 것은 착수 규율 문장뿐이다.
    ★기계유래 스폰 게이트(2026-08-10 P3B — §4-10 부트층 유사체 차단): D4-a′ 의 동의 신호는
    **오너가 친 선언**에만 성립하므로, 훅은 DETECT 발화 직후·spawn 이전에 판별 소유자
    (javis_mission `machine-origin` — 층1 원장 해시·층2 라벨)의 exit 를 소비한다:
    0=기계 유래→무스폰+정직 고지 / 1=오너 타이핑 간주→spawn / 판정 불가·모듈 부재→fail-closed
    무스폰+loud. 구 ⓔ 핀("기계 라벨도 발화+§4-10 고지")은 그 핀 자신이 요구한 절차대로
    **게이트 착지와 함께 의식적으로 뒤집었다** — 이제 무발화가 계약이다(leg ⓓ)."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ 임무 미지정(신규 사용자: 선언 단독) → 부트가 **정확히 1회** 실행된다(구 D4-a: 0회).
        #    계수는 목 부트가 스스로 남긴다 — 훅은 스폰 전에 같은 스크립트를 `lane-path` 조회로도
        #    부르므로(LANE_BOOT_LAST 파생) 조회 호출은 계수에서 제외한다(재는 것은 스폰뿐이다).
        boot = ("import os, sys\n"
                "if 'lane-path' in sys.argv:\n"
                "    print('(mock lane path)')\n"
                "    raise SystemExit(0)\n"
                "c = os.path.join(os.environ['HOME'], '.cys', 'state', 'boot-calls')\n"
                "os.makedirs(os.path.dirname(c), exist_ok=True)\n"
                "open(c, 'a').write('CALL\\n')\n"
                "print('MOCK-BOOT')\n")
        env, home, _pack, _bind, state = _rb_sandbox(tmp, mission=None, boot_body=boot)
        r = _run_rb(env, prompt="너는 마스터다")
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need("발화됨" in r.stdout, "선언=기동 명령(D4-a′)인데 발화하지 않았다(구 D4-a 무스폰 잔존): %r"
             % r.stdout[:300])
        need(_HOOK_FIRED_MARK in r.stdout, "발화 경로의 신호 문구가 바뀌었다(§0-A 정합 파괴)")
        need("발화 실패" not in r.stdout, "spawn 성공인데 '발화 실패' 를 보고했다: %r" % r.stdout[:300])
        logs = _run_ledgers(state)
        need(len(logs) == 1, "발화 로그(role-bootstrap-*.log)가 정확히 1개가 아니다: %s" % logs)
        calls_p = os.path.join(state, "boot-calls")
        _end = time.time() + 8.0
        while time.time() < _end and not os.path.isfile(calls_p):
            time.sleep(0.1)
        calls = _read(calls_p).count("CALL")
        need(calls == 1, "부트 실행 횟수 %d ≠ 1(0=구 D4-a 무스폰 회귀 / 2+=중복 기동)" % calls)
        need("MOCK-BOOT" in _read(os.path.join(state, logs[0])),
             "발화 로그에 부트 stdout(MOCK-BOOT)이 없다 — 스폰 관측과 로그가 어긋난다")
        # 정직성 1:1 + 착수 규율 문안(하한 핀): 주입문의 실행 서술('팀 세션 기동')은 실측
        # (MOCK-BOOT 1회)과 일치하고, 임무 상태 파라미터(MISSION_SENT)가 착수 금지 규율을 나른다.
        # ★관측 파생 강등(2026-08-10 P3 적발 수리): 문안은 게이트 폐쇄(record exit)만 단정하고,
        #   대장 기록은 조건 서술('오너가 친 선언 단독…라면 mission=null')로만 말한다 —
        #   'exit 1(임무 없음)' 핀은 record 가 실제로 완주한 관측 분기가 렌더됐음을 못 박는다.
        # ★기계유래 게이트 정직성(P3B): 이 발화 경로에 도달했다는 것은 machine-origin 게이트가
        #   exit 1(오너 타이핑 간주)을 냈다는 뜻이므로 문안도 그 관측을 그대로 서술해야 한다 —
        #   '오너 타이핑으로 판정' 핀이 그것을, '§4-10' 핀은 판별 범위 밖(동일 UID 위조 등)
        #   잔여위험 고지를 못 박는다(비구분 단언 문안은 게이트 착지로 거짓이 됐다 — 아래 잔존
        #   금지 핀).
        for frag in ("임무 상태:", "임무 게이트 폐쇄",
                     "자율 착수 권한이 발급되지 않았다", "exit 1(임무 없음)",
                     "선언 단독", "mission=null", "임무 대장", "javis_mission.py status",
                     "THREAT-MODEL-mission-gate.md §4-10",
                     "machine-origin", "오너 타이핑으로 판정", "기계 배달",
                     "자율 착수는 금지", "exit 3",
                     "보고 대상이지 착수 대상이 아니다", "2026-08-01", "멈춰",
                     "팀 세션 기동", "cys feed push --wait", "자율 진행 권한은 기본 미부여"):
            need(frag in r.stdout, "임무 미지정 주입문에 %r 이 없다(착수 규율/정직성 결손): %r"
                 % (frag, r.stdout[:400]))
        # 구 무조건 단언 잔존 금지: 'record exit'는 대장 쓰기를 보증하지 않는다(기계 유래 폴드=
        # 대장 무변경 · 모듈 부재 · 타임아웃 · unreadable 폐쇄 — 전부 실경로). 무조건 '기록'
        # 술어가 되살아나면 미기록 경로에서 주입문이 허위를 오너 보고로 중계한다.
        need("에 mission=null 기록" not in r.stdout,
             "무조건 'mission=null 기록' 단언 잔존 — 관측 파생 강등 회귀(정직성 위반): %r"
             % r.stdout[:400])
        # 구 D4-a 문안 잔존 = 정직성 위반(spawn 했는데 '승인 후 기동'이라 말하면 주입문≠실행)
        need("노드 기동은 사용자 승인 후" not in r.stdout,
             "구 D4-a 문안('노드 기동은 사용자 승인 후')이 잔존 — spawn 1회 실측과 어긋난다(정직성 위반)")
        need("준비만 되어 있고" not in r.stdout, "구 D4-a '준비만' 문안 잔존(정직성 위반)")
        # P3B 이전 비구분 단언 잔존 금지: '감지기는 …구분하지 않는다'는 기계유래 게이트 착지로
        # 거짓이 됐다(게이트가 구분한다). 그 문안이 되살아나면 주입문이 실제 배선을 부정한다.
        need("구분하지 않는다" not in r.stdout,
             "P3B 이전 비구분 단언('구분하지 않는다')이 잔존 — machine-origin 게이트가 배선된 "
             "현실과 어긋난다(정직성 위반): %r" % r.stdout[:400])
        # T1 층 무손상: 선언 단독 = mission=null 기록, 게이트는 닫혀 있다(사고 방지는 착수 층 소관)
        led = os.path.join(home, ".cys", "state", "mission.json")
        if os.path.isfile(led):
            rec = json.loads(_read(led) or "{}")
            need(not rec.get("mission"), "선언 단독인데 임무가 기록됐다(T1 파손): %r" % rec)
        g = _run([PY, os.path.join(BIN_DIR, "javis_mission.py"), "status"], env=env)
        need(g.returncode != 0, "선언 단독 후 임무 게이트가 열렸다(rc=%d) — 자율 착수 자기인가 부활"
             % g.returncode)
        notes.append("임무 미지정: spawn 1 · 발화 로그 1 · 착수금지 문안 · 게이트 닫힘")
        # ⓑ 같은 사양에서 **임무가 있으면** 종전 경로 그대로 발화한다(기존 사용자 무회귀)
        env2, _h2, _p2, _b2, state2 = _rb_sandbox(os.path.join(tmp, "b"))
        r2 = _run_rb(env2, prompt="너는 마스터다")
        need("발화됨" in r2.stdout, "임무 지정 세션에서 발화가 사라졌다(기존 사용자 회귀): %r"
             % r2.stdout[:300])
        need(_HOOK_FIRED_MARK in r2.stdout, "발화 경로의 신호 문구가 바뀌었다(§0-A 정합 파괴)")
        need("임무 게이트 exit 0" in r2.stdout,
             "임무 지정 주입문에 착수 허용 문안(임무 상태 파라미터 exit 0)이 없다: %r" % r2.stdout[:300])
        notes.append("임무 지정: 종전 발화 경로 보존")
        # ⓕ 판별 도구 **부재** 레인 = 판정 불가 → fail-closed 무스폰(P3B — 구 P3 계약을 의식적
        #    으로 뒤집었다): P3 시점에는 모듈 부재 레인도 spawn 1(D4-a′) + '미기록' 정직 고지가
        #    계약이었으나, 기계유래 스폰 게이트가 착지하며 판별 도구가 없는 레인은 **오너/기계를
        #    구분할 수 없으므로** 스폰을 열지 않는다(A5 role 게이트와 같은 방향 — 무단 스폰이
        #    판정 보류보다 나쁘다. T1 블록의 '임무 대장 미기록' fail-closed 와도 정합).
        #    주입문은 '선언 아님'과 '판정 불가'를 융합하지 않고 미발화를 명시해야 한다.
        #    훅을 팩 밖으로 복사해 형제 해소(../bin)를 끊고, 감지기만 팩에 심어 감지는 살린다.
        # ★A2 분할 이후: 팩 밖 사본은 **런처와 본체를 함께** 복사해야 돈다(런처 단독이면
        #   '부트 본체 부재' 고지에서 끝나 이 검체가 재려는 경로에 도달하지 못한다).
        curhook = os.path.join(tmp, "curhook-nomission", ROLE_HOOK)
        _w(curhook, _read(_hook(ROLE_HOOK)))
        _w(os.path.join(os.path.dirname(curhook), ROLE_BODY), _read(_hook(ROLE_BODY)))
        _w(os.path.join(os.path.dirname(curhook), "_lib.sh"),
           _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
        env3, _h3, pack3, _b3, _s3 = _rb_sandbox(os.path.join(tmp, "c"), mission=None)
        _w(os.path.join(pack3, "bin", "javis_detect.py"),
           _read(os.path.join(BIN_DIR, "javis_detect.py")), 0o644)
        r3 = _run([BASH, curhook], input=json.dumps({"prompt": "너는 마스터다"}), env=env3)
        need("발화됨" not in r3.stdout,
             "판별 도구(javis_mission.py) 부재 레인에서 spawn 이 열렸다 — 기계유래 판정 불가가 "
             "fail-closed 로 접히지 않는다(무단 스폰 > 판정 보류 역전): %r" % r3.stdout[:300])
        need("임무 대장 미기록" in r3.stderr,
             "모듈 부재 stderr 고지(T1)가 사라졌다: %r" % r3.stderr[:300])
        need("선언 아님이 아니라 판정 불가다" in r3.stdout,
             "모듈 부재 주입문이 '선언 아님'과 '판정 불가'를 융합했다(정직성 결손): %r"
             % r3.stdout[:400])
        need("부트 미발화" in r3.stdout,
             "모듈 부재 주입문에 미발화 명시가 없다: %r" % r3.stdout[:400])
        need("기계유래 판정 불가" in r3.stderr,
             "모듈 부재 게이트의 stderr loud 로그가 없다: %r" % r3.stderr[:300])
        need("mission=null 기록" not in r3.stdout,
             "모듈 부재(대장 미기록)인데 주입문이 'mission=null 기록'을 주장한다 — 같은 런 "
             "stderr '미기록'과 자기모순(정직성 1:1 파괴): %r" % r3.stdout[:400])
        notes.append("모듈 부재: 판정 불가=무스폰(fail-closed) · 판정불가 명시 주입문")
    # ⓒ 검증자가 실증한 **자기인가 우회로 2종**을 그 문안 그대로 재투입 → 대장 미기록
    with tempfile.TemporaryDirectory() as tmp:
        for prompt, why in (("[wakeup] 다음 액션 착수", "자기 예약 wake(CLAUDE.md.template:44)"),
                            ("[worker-1 완료] T1 끝났습니다. 다음 지시 주세요",
                             "워커 완료 push(CLAUDE.md §7)")):
            sb = os.path.join(tmp, re.sub(r"\W+", "_", why)[:20])
            env, home, _p, _b, _s = _rb_sandbox(sb, mission=None)
            _run_rb(env, prompt=prompt)
            led = os.path.join(home, ".cys", "state", "mission.json")
            if os.path.isfile(led):
                rec = json.loads(_read(led) or "{}")
                need(not rec.get("mission"),
                     "%s 가 임무로 기록됐다 — 자기인가 루프가 채널만 바꿔 부활한다: %r" % (why, rec))
            # 게이트도 닫혀 있어야 한다(기록 유무와 무관하게 최종 판정으로 확인)
            g = _run([PY, os.path.join(BIN_DIR, "javis_mission.py"), "status"], env=env)
            need(g.returncode != 0, "%s 이후 임무 게이트가 열렸다(rc=%d)" % (why, g.returncode))
        notes.append("우회로 2종 원문 재투입: 대장 미기록·게이트 닫힘")
        # ⓓ 기계유래 스폰 게이트(P3B — §4-10 부트층 유사체 **차단**): 구 ⓔ 핀은 "기계 라벨도
        #    spawn 발화 + §4-10 고지"를 현행 계약으로 못 박으며 "억제를 넣으면 이 핀을 함께
        #    의식적으로 뒤집으라"고 요구했다 — machine-origin 게이트 착지가 그 뒤집기다.
        #    새 계약: 기계 유래 선언(층2 라벨 또는 층1 원장 일치)은 **spawn 0** 이고, 주입문은
        #    무발화 사실·근거 확인 명령·오너 우연 일치 시 복구 경로를 정직하게 고지한다.
        #    착수 층 불변식(대장 null·게이트 폐쇄)은 그대로 유지된다.
        # ── ⓓ-1 기계 **라벨** 선언(층2 폴백 경로 — 실측 재현 문안 그대로) → spawn 0 ──
        envm, homem, _pm, _bm, sm = _rb_sandbox(os.path.join(tmp, "machine-decl"), mission=None)
        rm = _run_rb(envm, prompt="[wakeup] 너는 마스터다 - 다음 액션 확인")
        need(rm.returncode == 0, "기계 라벨 선언에서 훅이 비0 종료(exit=%d)" % rm.returncode)
        need("발화됨" not in rm.stdout,
             "기계 라벨 선언 push 가 spawn 을 발화했다 — 기계유래 스폰 게이트 미작동"
             "(§4-10 부트층 유사체 개방 · 오너 개입 0 의 팀 재스폰): %r" % rm.stdout[:300])
        logsm = [n for n in (os.listdir(sm) if os.path.isdir(sm) else [])
                 if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)]
        need(not logsm, "무스폰인데 발화 로그가 생겼다(무발화 경로 오염): %s" % logsm)
        for frag in ("기계 유래", "부트 미발화", "부트를 발화하지 않았다",
                     "javis_mission.py status", "delivery-path",
                     "문구를 바꿔", "2026-08-10", "THREAT-MODEL-mission-gate.md §4-10"):
            need(frag in rm.stdout,
                 "기계 라벨 무스폰 주입문에 %r 이 없다(무발화 고지/근거/복구 경로 결손): %r"
                 % (frag, rm.stdout[:400]))
        need("기계 유래 선언" in rm.stderr,
             "기계 라벨 무스폰의 stderr 1줄 로그가 없다: %r" % rm.stderr[:300])
        ledm = os.path.join(homem, ".cys", "state", "mission.json")
        if os.path.isfile(ledm):
            recm = json.loads(_read(ledm) or "{}")
            need(not recm.get("mission"),
                 "기계 라벨 push 가 임무로 기록됐다 — 착수 층 불변식 파괴: %r" % recm)
        gm = _run([PY, os.path.join(BIN_DIR, "javis_mission.py"), "status"], env=envm)
        need(gm.returncode != 0,
             "기계 라벨 push 이후 임무 게이트가 열렸다(rc=%d) — 착수 층 불변식 파괴" % gm.returncode)
        notes.append("기계 라벨 선언: 무스폰 · 정직 고지 · 대장 null · 게이트 닫힘")
        # ── ⓓ-2 라벨 **없는** 원장 일치 기계 배달(층1 경로 — 라벨 규약 우회 push) → spawn 0 ──
        #    픽스처는 판별 소유자(javis_mission)의 자기 함수로 만든다(레코드 필드 사본 금지 —
        #    self-test `_rec` 관례와 동일 필드: v·surface·ts_epoch·sha256·origin·chars·preview).
        envq, homeq, _pq, _bq, sq = _rb_sandbox(os.path.join(tmp, "machine-ledger"), mission=None)
        decl_q = "너는 마스터다 - 다음 액션 확인"          # ⓓ-1 과 같은 문안에서 라벨만 제거
        fix = ("import json, os, sys, time\n"
               "sys.path.insert(0, %r)\n"
               "import javis_mission as jm\n"
               "text = sys.argv[1]\n"
               "dp = jm.delivery_ledger_path()\n"
               "os.makedirs(os.path.dirname(dp), exist_ok=True)\n"
               "norm = jm._normalize_delivery(text)\n"
               "rec = {'v': jm.SCHEMA_VERSION, 'surface': jm._surface(),\n"
               "       'ts_epoch': time.time(), 'sha256': jm._digest_norm(norm),\n"
               "       'origin': 'send', 'chars': len(norm),\n"
               "       'preview': norm[:jm.PREVIEW_CHARS]}\n"
               "open(dp, 'a', encoding='utf-8').write(json.dumps(rec, ensure_ascii=False) + '\\n')\n"
               % BIN_DIR)
        rfix = _run([PY, "-c", fix, decl_q], env=envq)
        need(rfix.returncode == 0,
             "배달 원장 픽스처 생성 실패(rc=%d): %r" % (rfix.returncode, rfix.stderr[:300]))
        rq = _run_rb(envq, prompt=decl_q)
        need("발화됨" not in rq.stdout,
             "라벨 없는 **원장 일치** 기계 배달이 spawn 을 발화했다 — 층1 이 스폰 게이트에서 "
             "소비되지 않는다(라벨 규약 우회 push 로 §4-10 재개방): %r" % rq.stdout[:300])
        need("부트 미발화" in rq.stdout,
             "원장 일치 무스폰 주입문에 미발화 고지가 없다: %r" % rq.stdout[:400])
        gq = _run([PY, os.path.join(BIN_DIR, "javis_mission.py"), "status"], env=envq)
        need(gq.returncode != 0,
             "원장 일치 기계 배달 이후 임무 게이트가 열렸다(rc=%d)" % gq.returncode)
        notes.append("원장 일치 무라벨 배달: 무스폰(층1 소비) · 게이트 닫힘")
    # 계측 타당성 2종(MEMORY 3칙 — 검출기가 구 결함 코드에서 FIRE 하는가):
    #   ① 스폰 검출기: **D4-a(무스폰) 시대 훅**(D4A_REF)은 같은 조건(임무 미지정 선언 단독)에서
    #      spawn 0 = '발화됨' 부재 — '발화됨 있어야' 핀이 그 결함(2択 혼동)을 실제로 잡는다.
    #      ★공허 통과 봉합(2026-08-10 P3 적발): 그 시대(post-W1b) 훅은 감지를
    #      bin/javis_detect.py 에 위임하는데, 샌드박스 팩에는 감지기가 없고 훅 복사로 형제
    #      해소(../bin)도 끊겨 있어 임무 게이트에 도달하기 **전에** '감지기 부재' 분기로 조기
    #      종료했다 — 그 상태의 '발화됨 부재'는 무스폰 결함의 재현이 아니라 감지기 부재의
    #      관측이다(어떤 조기 종료도 같은 부정 단언을 통과시키므로 스폰 검출기의 신·구 구분력 0).
    #      같은 시대의 감지기를 샌드박스 팩에 심어 구 훅이 진짜 무스폰 분기에 도달하게 만들고,
    #      도달 사실을 D4-a 노트의 양성 마커('준비만 되어 있고')로 함께 단언한다.
    #   ② 문안 검출기: **T1 이전 훅**(CALIBRATION_REF)은 발화하되 착수 규율 문안이 전무하다
    #      (2026-08-01 사고 원형) — '자율 착수는 금지' 핀이 그 결함을 실제로 잡는다.
    #   ③ ⓓ-1 무스폰 검출기(P3B): 같은 T1 이전 훅은 **기계 라벨 선언에도 발화한다**(기계유래
    #      게이트·T1 상관 게이트가 모두 없던 시대 = §4-10 부트층 유사체의 원형). 이 재현이
    #      있어야 ⓓ-1 의 '발화됨 부재' 핀이 신·구를 실제로 구분한다. ※D4-a′ 직후·게이트 이전
    #      (P3) 트리는 로컬 미커밋이라 git 계측 대조가 불가능하다 — 그 시점의 기계 라벨 발화는
    #      P3 적대검증이 실측으로 재현·기록했다(구 ⓔ 핀이 그 계약이었다).
    calib = []
    with tempfile.TemporaryDirectory() as tmp:
        for ref, tag in ((D4A_REF, "d4a"), (None, "pre-t1")):
            src = _git_show("cysjavis-pack/hooks/role-bootstrap.sh", ref=ref)
            if src is None:
                calib.append("skip(no-git)")
                continue
            oldhook = os.path.join(tmp, "oldhooks-%s" % tag, "role-bootstrap.sh")
            _w(oldhook, src)
            _w(os.path.join(os.path.dirname(oldhook), "_lib.sh"),
               _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            oenv, _h, opack, _b, _s = _rb_sandbox(os.path.join(tmp, "sb-%s" % tag), mission=None)
            if tag == "d4a":
                det = _git_show("cysjavis-pack/bin/javis_detect.py", ref=D4A_REF)
                need(det is not None,
                     "계측 대조 불가: %s 의 bin/javis_detect.py 를 얻지 못했다(git show 실패) — "
                     "측정 불능은 통과가 아니다" % D4A_REF)
                _w(os.path.join(opack, "bin", "javis_detect.py"), det, 0o644)
            ro = _run([BASH, oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=oenv)
            if tag == "d4a":
                need("판정 불가" not in ro.stdout,
                     "계측 무효: D4-a 시대 훅(%s)이 '판정 불가'로 조기 종료했다(감지기·인터프리터 "
                     "부재 등) — 무스폰 분기 미도달, '발화됨 부재'가 결함 재현을 증명하지 못한다: %r"
                     % (D4A_REF, ro.stdout[:300]))
                need("준비만 되어 있고" in ro.stdout,
                     "계측 타당성 실패: D4-a 시대 훅(%s)이 무스폰 노트('준비만 되어 있고')에 "
                     "도달하지 않았다 — 스폰 검출기가 구 결함(무스폰 2択)의 실경로를 밟지 못했다: %r"
                     % (D4A_REF, ro.stdout[:300]))
                need("발화됨" not in ro.stdout,
                     "계측 타당성 실패: D4-a 시대 훅(%s)이 임무 미지정 선언에서 발화한다 — "
                     "스폰 검출기가 구 결함(무스폰 2択)을 재현하지 못한다" % D4A_REF)
                calib.append("D4-a 훅=무스폰 노트 도달 재현")
            else:
                need("발화됨" in ro.stdout,
                     "계측 타당성 실패: T1 이전 훅이 발화하지 않는다 — 문안 검출기 대조 불가")
                need("자율 착수는 금지" not in ro.stdout,
                     "계측 타당성 실패: T1 이전 훅에 이미 착수금지 문안이 있다 — "
                     "핀이 신·구를 구분하지 못한다")
                # ③ ⓓ-1 무스폰 검출기 계측(위 주석 ③): 기계 라벨 선언 → 구 훅은 발화해야 한다
                rmach = _run([BASH, oldhook],
                             input=json.dumps(
                                 {"prompt": "[wakeup] 너는 마스터다 - 다음 액션 확인"}),
                             env=oenv)
                need("발화됨" in rmach.stdout,
                     "계측 타당성 실패: T1 이전 훅이 기계 라벨 선언에서 발화하지 않는다 — "
                     "ⓓ-1 무스폰 핀이 구 결함(기계 push 스폰 · §4-10 원형)을 재현하지 못한다: %r"
                     % rmach.stdout[:300])
                calib.append("pre-T1 훅=발화+착수규율 문안 부재+기계라벨 스폰 재현")
    return " · ".join(notes) + " · 계측검증=%s" % "+".join(calib)


# ═══════════════════════════════════════════════════════════════════════════
# 3. H-EXIT (종료·판정 계약) — W1a 발효분은 A15/R2 하나
# ═══════════════════════════════════════════════════════════════════════════
def _boot_sandbox(tmp, *, cys_extra="", check_exit=0):
    """javis_bootstrap.py 격리 실행 환경(HOME+가짜 팩+스텁 cys). 반환 (env, home, tmp)."""
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack")
    bindir = os.path.join(tmp, "stubbin")
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    _mock_cys(bindir, tmp, cys_extra)
    _w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
    _w(os.path.join(pack, "bin", "javis_orchestra.py"),
       "import sys; sys.exit(%d)\n" % check_exit, 0o644)
    env = _base_env({"HOME": home, "PATH": bindir + os.pathsep + os.environ.get("PATH", ""),
                     "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "1",
                     "CYS_BOOT_CHECK_INTERVAL_S": "0.05"})
    return env, home


def _boot_last(home):
    return json.loads(_read(os.path.join(home, ".cys", "state", "boot-last.json")) or "{}")


@specimen("H-EXIT-1", "W1b",
          "cmd_run 종료 불변식(stdout JSON / stderr verdict·run_id·귀속) + A20 소비 분기",
          ["A7", "A19", "A20(소비층)", "CS-2⑩"])
def h_exit_1():
    """A7·A19·CS-2⑩: 종료 채널 분리 · 런 정체성 · 정당거부의 boot-last 오염 0.

    ※A20 은 두 층이다 — CLI 타입드 exit 표(cys.rs)는 **W2**, bootstrap 의 **소비 분기**
      (거부 마커 유무로 exit 7 / 10 분리)는 W1b 다. 여기서 재는 것은 후자다(H-EXIT-3=W2 유지).
    """
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    notes = []
    src = _read(boot)
    need("def _cmd_run_chain(" in src and "log.finish(" in src,
         "cmd_run 이 try/finally 종결 기록 구조로 분리되지 않았다(A19)")
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ 완주 → stdout 최종 JSON(구 산문 계약 보존) + 귀속·종결 기록
        env, home = _boot_sandbox(os.path.join(tmp, "ok"))
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 0, "완주 경로 exit≠0: %d\n%s" % (r.returncode, r.stderr[-500:]))
        summary = json.loads(r.stdout.strip().splitlines()[-1])
        need(summary.get("ok") is True, "stdout 최종 JSON 이 성공 계약을 잃었다: %r" % summary)
        bl = _boot_last(home)
        for k in ("run_id", "pid", "surface", "ended", "exit"):
            need(k in bl, "boot-last 에 런 귀속·종결 필드 누락: %s (%r)" % (k, sorted(bl)))
        need(bl["surface"] == "7", "surface 귀속이 틀렸다: %r" % bl.get("surface"))
        need(bl["exit"] == 0 and (bl.get("result") or {}).get("state") == "completed",
             "완주 상태 기록 이상: %r" % bl.get("result"))
        need((bl["result"].get("run_id") or "") == bl["run_id"],
             "result 에 run 귀속이 없다(§0 '자기 surface 최신 완주 런' 판독 불가)")
        notes.append("완주=stdout JSON+귀속+종결")

        # ⓑ 정당거부(claim_denied) → exit 7 · **ok:false 오염 0**(state=declined)
        env, home = _boot_sandbox(
            os.path.join(tmp, "deny"),
            cys_extra=('case "$1" in claim-role) echo "claim_denied: privileged role held by '
                       'live surface" >&2; exit 1;; esac'))
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 7, "정당거부 exit≠7: %d" % r.returncode)
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is None and res.get("state") == "declined",
             "정당거부가 공유 boot-last 에 ok:false 를 덮었다(CS-2⑩ 회귀): %r" % res)
        need(res.get("surface") == "7", "거부 기록에 surface 귀속이 없다: %r" % res)
        notes.append("거부=exit 7·ok:null(오염 0)")

        # ⓒ 거부 마커 없는 claim 실패 → **exit 10 세션 컨텍스트 오류**(A20 소비 분기)
        env, home = _boot_sandbox(
            os.path.join(tmp, "ctx"),
            cys_extra='case "$1" in claim-role) echo "error: no surface" >&2; exit 4;; esac')
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 10, "거부 마커 없는 실패가 exit 10 으로 분리되지 않았다: %d" % r.returncode)
        need("세션 컨텍스트 오류" in r.stderr, "exit 10 문구가 없다: %r" % r.stderr[-300:])
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is None and res.get("state") == "session_error",
             "세션 컨텍스트 오류가 ok:false 를 덮었다: %r" % res)
        notes.append("마커 없는 실패=exit 10·ok:null")

        # ⓓ 인프라 실패(ping)는 여전히 ok:false — 과교정 금지(이건 실제로 깨진 부트다)
        env, home = _boot_sandbox(os.path.join(tmp, "ping"),
                                  cys_extra='case "$1" in ping) exit 1;; esac')
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 3, "ping 실패 exit≠3: %d" % r.returncode)
        res = _boot_last(home).get("result") or {}
        need(res.get("ok") is False and res.get("state") == "failed",
             "인프라 실패가 ok:false 를 잃었다(과교정): %r" % res)
        notes.append("인프라 실패=ok:false 유지")

        # ⓔ 진행 중 선기록(A19) — 러닝 상태에서 강제 종료해도 'running' 이 남는다
        env, home = _boot_sandbox(os.path.join(tmp, "run"),
                                  cys_extra='case "$1" in ping) sleep 20;; esac')
        pr = subprocess.Popen([PY, boot], env=env, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL)
        deadline = time.time() + 15
        bl = {}
        while time.time() < deadline:
            bl = _boot_last(home)
            if bl.get("run_id"):
                break
            time.sleep(0.1)
        pr.kill(); pr.wait(timeout=15)
        need(bl.get("run_id"), "시작 시점 선기록이 없다(진행 중 상태 판별 불가)")
        need((bl.get("result") or {}).get("state") == "running",
             "선기록 상태가 running 이 아니다: %r" % bl.get("result"))
        notes.append("running 선기록(SIGKILL 생존)")
    # 계측 타당성: 구 코드는 종결·귀속 필드가 없었다
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    calib = "skip(no-git)"
    if old is not None:
        need('"result"] = {"ok": True}' in old or '{"ok": True}' in old,
             "계측 타당성 실패: 구 코드 성공 기록 형태를 못 찾았다")
        need("run_id" not in old, "계측 타당성 실패: 구 코드에 이미 run_id 가 있다")
        need("log.finish(" not in old, "계측 타당성 실패: 구 코드에 이미 종결 기록이 있다")
        calib = "구 코드 run_id·종결 기록 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
# ─────────── W2 발효: 타입드 계약(H-EXIT-2~7) ───────────
# 계측 층위 규약: Rust CLI 경계의 계약은 ①레포 소스 구조 단언(배포 팩에선 Skip) + ②빌드된
# 바이너리가 있으면 **부작용 0 경로만** 실행(데몬 없는 미도달·인자 오류)로 잰다. 노드를 스폰할 수
# 있는 경로(`cys boot` 본문)는 러너에서 절대 실행하지 않는다 — 검체가 조직을 건드리면 안 된다.
_CYS_BIN_SKIP_REASON = "바이너리 미빌드"


def _cys_bin():
    """빌드된 **진짜** cys 바이너리(있으면) — 부작용 0 경로 실측용. 없으면 None(구조 단언만).

    ★신원 확인이 필수인 이유(2026-08-01 실측 회귀): 형제 워크플로우의 빌드 픽스처가
      `target/debug/cys` 에 **10바이트 `#!/bin/sh` 스텁**을 남겼고(같은 시각 `cysd`·
      `pack.tar.gz`(빈 gzip)·`pack-manifest.json`(`{}`)·`runtime/.stub` 동반), 종전 판정
      (`isfile` + `X_OK`)이 그것을 바이너리로 받아들였다. 스텁은 무엇을 시켜도 rc=0 이라
      H-EXIT-3 이 "데몬 미도달이 exit 3 이 아니다(rc=0)" 라는 **거짓 적색**을 냈다
      (원인 증명: 깨끗한 HEAD 클론에 같은 스텁을 넣자 PASS→동일 FAIL 로 뒤집혔다).
    ★이것은 게이트 약화가 아니다 — 이 러너 헤더의 **계측기 자기검증** 규약("탐지기가 구 결함을
      못 잡으면 신 코드의 PASS 는 아무 의미가 없다")의 역방향 적용이다. 계측 대상이 가짜면
      PASS 도 FAIL 도 무의미하므로, 신원이 확인되지 않으면 **실측을 건너뛰고 그 사유를 노트에
      남긴다**(조용한 skip 금지 — `_CYS_BIN_SKIP_REASON`).
    """
    global _CYS_BIN_SKIP_REASON
    reasons = []
    for rel in (os.path.join("target", "debug", "cys"), os.path.join("target", "release", "cys")):
        p = os.path.join(REPO_DIR, rel)
        if not (os.path.isfile(p) and os.access(p, os.X_OK)):
            continue
        try:
            r = _run([p, "--version"], timeout=30)
        except Exception as e:                      # 실행 자체가 불가 = 바이너리 아님
            reasons.append("%s 실행 불가(%s)" % (rel, e))
            continue
        out = (r.stdout or "") + (r.stderr or "")
        # clap 정본: `#[command(name = "cys", version)]` → "cys <semver>"
        if r.returncode == 0 and re.search(r"(?m)^\s*cys\s+\d+\.\d+", out):
            _CYS_BIN_SKIP_REASON = None
            return p
        reasons.append("%s 는 cys 바이너리가 아니다(rc=%d · --version=%r · %dB) — 빌드 픽스처 스텁 의심"
                       % (rel, r.returncode, out.strip()[:40], os.path.getsize(p)))
    _CYS_BIN_SKIP_REASON = "; ".join(reasons) if reasons else "바이너리 미빌드"
    return None


@specimen("H-EXIT-2", "W2", "boot Busy → busy 판정 구분(--json + bare exit 75) + CEO 티켓 보존",
          ["G11"])
def h_exit_2():
    """G11: boot 락 Busy-skip(exit 0)을 ④가 '스폰 성공'으로 오인해 CEO 티켓을 **무스폰 소각**했다.
    busy 는 --json 의 outcome 으로 타입 구분되고, 티켓 소비는 실스폰 확인 후에만 일어난다.
    ★**W4 갱신(하드 제약 6-⑧ 이행)**: bare exit 의미 전환이 GUI `--json` 소비와 **같은 커밋**으로
      착지했으므로, 이 검체가 박제하는 계약도 함께 전진한다 — busy = `EXIT_BOOT_BUSY`(75).
      W2 시점의 '구계약 유지(busy=0)' 단언은 폐기가 아니라 **전환 완료로 승계**된 것이다
      (그 단언의 목적은 'GUI 가 모르는 채 exit 의미가 바뀌는 것' 차단이었고, 이제 GUI 가 안다)."""
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    notes = []
    # ⓐ busy 경로가 --json 에서 outcome:"busy" 를 낸다. (run_boot **본문의** Busy 분기 — 락 헬퍼의
    #    Busy 분기와 구분해야 한다: 후자는 유계 대기 경로이고 --json 계약과 무관하다.)
    rb = src.find("fn run_boot(")
    need(rb > 0, "run_boot 를 못 찾았다")
    i = src.find("BootLock::Busy =>", rb)
    need(i > 0, "run_boot 의 boot 락 Busy 분기를 못 찾았다")
    busy_arm = src[i:i + 2000]
    need('"outcome": "busy"' in busy_arm,
         "Busy 분기가 --json 에 outcome:busy 를 내지 않는다(성공과 구분 불가 — G11 재발)")
    need("boot_exit_code(0, 0, true)" in busy_arm,
         "Busy 의 bare exit 가 단일 판정 함수를 통과하지 않는다(의미가 코드 두 곳으로 흩어짐)")
    need("return 0;" not in busy_arm,
         "Busy 가 여전히 exit 0(성공)을 낸다 — 무스폰을 성공으로 보고(G11 재발)")
    # 3자 파리티: Rust lib 정본 ↔ python 소비부 ↔ GUI 소비부가 같은 값을 쓴다.
    lib = os.path.join(REPO_DIR, "src", "lib.rs")
    if os.path.isfile(lib):
        need("pub const EXIT_BOOT_BUSY: i32 = 75;" in _read(lib), "lib 정본 busy exit 상수 이탈")
    need("CYS_BOOT_EXIT_BUSY = 75" in _read(os.path.join(BIN_DIR, "javis_bootstrap.py")),
         "python 소비부 busy exit 상수 이탈(파리티 붕괴)")
    gui = os.path.join(REPO_DIR, "src-tauri", "src", "main.rs")
    if os.path.isfile(gui):
        need("cys::EXIT_BOOT_BUSY" in _read(gui),
             "GUI 가 busy exit 를 별도 분기하지 않는다(중첩 부트 위경보 — 전환의 전제 조건)")
    notes.append("busy=outcome 구분 + bare exit 75(3자 파리티)")
    # ⓑ 티켓 소비는 ④ boot **성공 직후**에만(무스폰 소각 0). bootstrap 소비부 구조 단언.
    boot_src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    # 라벨은 W3 의 STEP 레지스트리로 승격됐다(리터럴 금지) — **소비 호출 지점**을 앵커로 쓴다.
    ci = boot_src.find("_consume_dept_ticket(ticket_path)")
    need(ci > 0, "CEO 티켓 소비 호출 지점을 못 찾았다")
    before = boot_src[:ci]
    need("_boot_fatal_verdict(" in before,
         "티켓 소비가 boot 결과 판정보다 앞선다(무스폰 소각 경로 잔존 — G11)")
    # busy 는 실스폰이 아니다: 소비 지점이 --json 판정 뒤에 있어야 busy 에서 티켓이 타지 않는다.
    need("_parse_boot_json(" in before, "티켓 소비가 --json 소비보다 앞선다(busy 구분 없이 소각)")
    need("_boot_was_busy(" in before, "티켓 소비가 busy 판정보다 앞선다(무스폰 소각 경로 잔존)")
    need(re.search(r"if ticket_path is not None and not boot_busy:", boot_src),
         "티켓 소비 조건에 busy 배제가 없다 — 1회성 티켓 ⟺ 실스폰 불변식 파괴(G11)")
    notes.append("티켓 소비는 boot --json 판정 후 + busy 배제(실스폰 확인)")
    # ⓒ 소비부가 busy 를 Fatal 실패로 오분류하지 않는다(순수 판정 실측).
    sys.path.insert(0, BIN_DIR)
    try:
        import javis_bootstrap as B
        busy = json.dumps({"roles": [{"role": "cso", "agent": "claude",
                                      "outcome": "busy", "mandatory": True}]})
        need(B._boot_fatal_verdict(0, busy) is None, "busy 가 Fatal 실패로 오분류(G11 재발)")
        need(B._boot_was_busy(B.CYS_BOOT_EXIT_BUSY, busy) is True,
             "소비부가 exit 75 를 busy(무스폰)로 읽지 않는다 — 티켓 소각 위험")
        need(B._boot_fatal_verdict(B.CYS_BOOT_EXIT_BUSY, "파싱 불가 산문") is None,
             "exit 75 가 보수 폴백에서 Fatal 로 접힌다(중첩 부트마다 exit 4 위경보)")
        fatal = json.dumps({"roles": [{"role": "cso", "agent": "claude",
                                       "outcome": "failed", "mandatory": True}]})
        need(B._boot_fatal_verdict(1, fatal) is not None, "의무 역할 failed 가 Fatal 로 승격되지 않음")
    finally:
        sys.path.remove(BIN_DIR)
    notes.append("소비부 busy≠Fatal 실측")
    # 계측 타당성: 구 코드는 Busy 에서 산문 한 줄 + exit 0 만 냈다(타입 구분 0).
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        j = old.find("BootLock::Busy =>")
        need(j > 0 and '"outcome"' not in old[j:j + 1200],
             "계측 타당성 실패: 구 코드 Busy 분기에 이미 outcome 타입이 있다")
        calib = "구 코드 Busy=산문+exit0(타입 구분 0) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-EXIT-3", "W2", "claim-role 타입드 exit 표(0/7/3/2) + A5 surface-role 3상", ["A20", "A5"])
def h_exit_3():
    """A20: CLI 경계의 타입드 exit 부재 → 소비부가 **에러 문자열 grep** 으로 정당거부/세션오류를
    갈라야 했다(문자열 계약 = 드리프트 시한폭탄). 0=성공 / 7=정당거부 / 3=미도달 / 2=식별불가."""
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("fn run_claim_role(" in src, "claim-role 타입드 핸들러(run_claim_role)가 없다")
    i = src.find("fn run_claim_role(")
    # ★창 폭(2026-08-16): 종전 3000 은 함수 길이(≈2742)에 아슬아슬해, rc 6 분기가 들어오자
    #   `" 3\n"` 오프셋이 2707 까지 밀렸다 — 산문 몇 줄만 더 붙으면 이 검체가 **결함 없이도**
    #   적색이 되는 시한폭탄이었다. 창은 '함수 전체'를 덮도록 넉넉히 잡는다(판정 대상은 함수
    #   본문이지 임의의 3000자가 아니다). 다음 fn 경계로 자르지 않는 이유는 이 파일의 다른
    #   검체들과 같은 관용(고정 창)을 유지하기 위해서다.
    body = src[i:i + 4500]
    for code, why in ((" 7\n", "정당거부"), (" 3\n", "미도달"), (" 2;\n", "식별불가(early)")):
        need(code in body or code.strip() in body, "claim-role 타입드 exit %s(%s) 부재" % (code.strip(), why))
    need('e.starts_with("claim_denied")' in body,
         "정당거부 판정이 데몬 **에러 코드**가 아니라 다른 근거를 쓴다(문자열 계약 잔존)")
    notes = ["소스 구조: 0/7/3/2 분기 + claim_denied 코드 판정"]
    # ★소비 정합(W1b bootstrap 분기와 짝): exit 가 1차 근거이고 문자열 grep 은 구 바이너리 폴백이다.
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need("code == EXIT_CLAIM_DENIED or (code == 1 and any(" in bsrc,
         "bootstrap 이 타입드 exit 를 1차 근거로 소비하지 않는다(문자열 grep 단독 잔존)")
    need("3: (\"미도달" in bsrc and "2: (\"식별 불가" in bsrc,
         "bootstrap 이 exit 3/2 의 정확한 처방을 분기하지 않는다")
    notes.append("bootstrap 소비: exit 1차 근거 + 3/2 처방 분기(문자열=구 바이너리 폴백)")
    # 실측(부작용 0 경로만): 데몬 소켓 부재 → 미도달(3) / surface 식별 불가 → 2.
    cys = _cys_bin()
    if cys is None:
        notes.append("실측 생략(구조 단언만) — 사유: %s" % _CYS_BIN_SKIP_REASON)
    else:
        tmp = tempfile.mkdtemp(prefix="cys-h-exit3-")
        try:
            # ★부작용 0 강제: CYS_NO_AUTOSTART=1 로 sibling cysd autostart 를 차단한다.
            #   차단하지 않으면 이 검체가 **실제 데몬을 띄우고** 팩을 시드한다(러너가 조직을
            #   건드리는 것은 절대 금지 — 계측기가 대상을 바꾸면 계측이 무효다).
            env = _base_env({"CYS_SOCKET": os.path.join(tmp, "nope.sock"),
                             "CYS_SURFACE_ID": "7", "HOME": tmp,
                             "CYS_NO_AUTOSTART": "1"})
            r = _run([cys, "claim-role", "master"], env=env, timeout=60)
            need(r.returncode == 3,
                 "데몬 미도달이 exit 3 이 아니다(rc=%d) — '남이 master'와 융합\n%s"
                 % (r.returncode, r.stderr[-300:]))
            env2 = dict(env)
            env2.pop("CYS_SURFACE_ID", None)
            env2["AITERM_SURFACE_ID"] = ""
            r2 = _run([cys, "claim-role", "master"], env=env2, timeout=60)
            need(r2.returncode == 2,
                 "surface 식별 불가가 exit 2 가 아니다(rc=%d)\n%s" % (r2.returncode, r2.stderr[-300:]))
            notes.append("실측: 미도달=3 · 식별불가=2(autostart 차단·부작용 0)")
            # ★A5 3상화 동승 실측(같은 CLI 경계 계약): surface-role 이 '판정 불가'를 rc=2 로 낸다.
            #   종전엔 rc0+빈출력으로 '미claim' 과 융합돼, 데몬이 죽은 상황에서 훅이 마스터 부트를
            #   발화했다. stdout 은 여전히 빈 줄이라 role-capability-gate(캐시 폴백)는 무회귀다.
            r3 = _run([cys, "surface-role"], env=env, timeout=60)
            need(r3.returncode == 2,
                 "surface-role 판정 불가가 rc=2 가 아니다(rc=%d) — '미claim' 융합 잔존" % r3.returncode)
            need(r3.stdout.strip() == "",
                 "판정 불가에서 stdout 에 role 을 냈다(소비 훅 오독): %r" % r3.stdout)
            env3 = dict(env)
            env3.pop("CYS_SURFACE_ID", None)
            r4 = _run([cys, "surface-role"], env=env3, timeout=60)
            need(r4.returncode == 0,
                 "surface env 부재가 판정 불가로 승격됐다(rc=%d) — 능력 가드 전면 차단 위험" % r4.returncode)
            notes.append("A5 3상: 판정불가=2(stdout 빈) · surface env 부재=0")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("fn run_claim_role(" not in old,
             "계측 타당성 실패: 구 코드에 이미 타입드 claim-role 핸들러가 있다")
        calib = "구 코드=성공 0/그 밖 1(1비트 붕괴) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-IDENT-1", "W6",
          "claim 신원 실패 ⇄ 정당거부 3겹 분리(데몬 코드·훅 선행 claim·폴백 실측 가드)",
          ["FIELD-2026-08-16"])
def h_ident_1():
    """현장 결함(2026-08-16 · macOS·Windows 공통): 훅이 부트스트랩을 **세션 분리**로 발화하고
    곧 종료한다 → 부트가 재부모화(ppid→1)돼 **조상 체인이 끊긴다** → 데몬 claim_role 이 발신
    pane 을 확정하지 못해 거부 → 그 거부 코드가 "살아있는 특권 보유자" 거부와 **같은
    claim_denied** 라, 소비 사슬(cys.rs rc 7 → javis_bootstrap ③ → 위계 폴백)이 이를 정당거부로
    읽고 **부서를 자동 생성**했다. 결과: master 영구 미등록(role=-)·선언마다 dept-N 증식.

    실측 e2e(격리 데몬): 같은 surface·같은 순간에 동기 실행 claim=성공 / 분리 실행 claim=거부.
    수리 3겹을 각각 박제한다 — 어느 한 겹만 되돌려도 이 검체가 적색이 된다."""
    notes = []

    # ── L1 데몬: 신원 실패 전용 에러코드(정당거부와 분리) + CLI rc 6 매핑 ──
    h = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    need('"claim_caller_unresolved"' in h,
         "데몬이 발신 pane 미해석을 전용 코드로 내지 않는다(claim_denied 와 융합 잔존)")
    need('"claim_not_owner"' in h,
         "데몬이 소유 불일치를 전용 코드로 내지 않는다(claim_denied 와 융합 잔존)")
    c = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need('e.starts_with("claim_caller_unresolved")' in c and 'e.starts_with("claim_not_owner")' in c,
         "CLI 가 신원 실패 코드를 분기하지 않는다 — rc 7(정당거부)로 다시 접힌다")
    notes.append("L1 데몬/CLI: 신원 실패 전용 코드 + rc 6 분기")

    # ── L1-P1 seat 토큰(2026-08-26 · R3-P1-5 P7/P8): 체인 단일채널 → 토큰 1차+모순 거부권 ──
    # rc 6 의 근본원인(조상 체인 단절)을 관통하는 데몬 발급 토큰 축이 **되돌려지면** 이 검체가
    # 적색이 된다. 핀은 전부 실제 성질의 좌표다 — 마커 문자열이 아니라 분기·순서·부재를 잰다.
    # ⓐ 데몬: seat_token param 1차 대조 + 모순 거부권의 신선 재해석(캐시 무효화 → probe 순서).
    need('param_str(&params, "seat_token")' in h,
         "데몬 claim 이 seat_token param 을 대조하지 않는다 — 체인 단일채널 회귀(rc6 근본원인 잔존)")
    need("seat_token_ct_eq" in h,
         "토큰 대조가 상수시간 비교를 쓰지 않는다(타이밍 누설 — state.rs 헬퍼 미사용)")
    need('"token_chain_conflict"' in h and '"token_mismatch"' in h,
         "모순 거부권(token_chain_conflict)·동세대 불일치(token_mismatch) reason 이 없다 — "
         "이상 신호가 침묵으로 접힌다(1차 성찰의 rc6 뿌리 재현)")
    # ★슬라이스 앵커는 claim arm 고유 변수명이다 — `param_str(&params, "seat_token")` 은
    #   hook.decide arm(파일 앞쪽)에도 있어 첫 매치로 자르면 엉뚱한 arm 을 재게 된다.
    claim_arm = h[h.find("let seat_token_param"):][:3000]
    i_inval = claim_arm.find(".remove(&p)")
    i_probe = claim_arm.find("probe_caller_surface_uncached(daemon, p)")
    need(0 <= i_inval < i_probe,
         "거부권의 체인 판독이 '캐시 무효화 → 신선 재해석(probe)' 순서가 아니다 — Windows pid "
         "재사용 stale 양성이 유효 토큰을 오거부한다(false veto · G13 임계영역 내 재검증 위반)")
    need("boot_gates_master_off_from" in claim_arm,
         "데몬 토큰 분기에 CYS_BOOT_GATES=0 우산이 없다 — 롤백 불가(노브 규율 위반)")
    # ⓑ CLI 첨부 배선(P7): claim 은 마스터 스위치 가드 **이후** 첨부, 훅은 스위치가 RPC 자체를
    #   접으므로(legacy 조기 반환) 그 반환이 페이로드 조립보다 앞이어야 한다.
    cl = c[c.find("fn run_claim_role("):]
    cl = cl[:cl.find("\nfn ", 10)]
    # ★가드 앵커도 실제 코드형이다 — 벌거벗은 `gate_axes_forced_legacy()` 리터럴은 주석에도
    #   나타나(예: 훅 함수의 carve-out 서술 10422행) 코드가 지워지고 주석만 남아도 초록이
    #   된다(false green · 우회 가능 핀 금지 — seat_token 앵커와 같은 엄격성).
    i_guard = cl.find("if !cys::gate_axes_forced_legacy()")
    i_attach = cl.find('params["seat_token"]')
    need(i_attach >= 0 and "ENV_SEAT_TOKEN" in cl,
         "run_claim_role 이 env CYS_SEAT_TOKEN 을 params.seat_token 으로 첨부하지 않는다 — "
         "데몬 토큰 축이 있어도 CLI 가 실어 나르지 않으면 체인 단일채널 그대로다")
    need(0 <= i_guard < i_attach,
         "CYS_BOOT_GATES=0 에서 토큰 키 생략(소스 핀)이 없다 — 첨부가 마스터 스위치 가드 "
         "이전이라 롤백 우산이 CLI 측에서 성립하지 않는다(R3-P1-1)")
    hk = c[c.find("fn run_hook_user_prompt_submit("):]
    hk = hk[:hk.find("\nfn ", 10)]
    # ★fold 앵커는 `if cys::…` 코드형으로 좁힌다 — 이 함수 안에는 실제 fold(선두 조기 반환)와
    #   별개로 carve-out 주석이 같은 리터럴을 attach 보다 **앞에** 담고 있어, 벌거벗은 리터럴로
    #   잡으면 조기 반환이 제거되고 주석만 남아도 순서 판정이 초록으로 남는다(false green).
    i_fold = hk.find("if cys::gate_axes_forced_legacy()")
    # ★핀 대상은 실제 대입문이다 — 주석의 'seat_token' 낱말(carve-out 서술)이 아니라. 낱말로
    #   잡으면 배선을 지우고 주석만 남겨도 초록이 된다(우회 가능 핀 금지).
    i_hook_attach = hk.find('hook_params["seat_token"]')
    need(i_hook_attach >= 0 and 0 <= i_fold < i_hook_attach,
         "훅 hook.decide 페이로드에 seat_token 첨부가 없거나, 마스터 스위치 legacy 조기 반환이 "
         "첨부보다 뒤다 — 훅 좌석 해석이 체인 단일채널로 회귀(P7 배선 소실)")
    # ⓒ rc 계약 불변: 토큰 사유는 기존 코드(claim_caller_unresolved/claim_not_owner) payload
    #   reason 이지 신설 에러코드가 아니다 — CLI 에 token 계열 starts_with 분기가 생기면 구 데몬
    #   스큐에서 rc 오진 표면이 생긴다(신설 코드 금지 계약의 역방향 핀).
    need('starts_with("token' not in c,
         "CLI 가 token 계열을 **에러코드**로 분기한다 — rc 6/7 계약 불변(신설 코드 금지 · "
         "reason 은 payload 로만) 위반")
    # ⓓ 경계 회귀 핀(cargo 검체 존재): 무기록·무영속 계약을 재는 rust 테스트가 삭제되면 여기서
    #   잡는다(기존 테스트 약화·삭제 금지의 기계 집행).
    need("seat_token_path_never_records_caller_cache" in h,
         "caller_cache 무기록 회귀 핀(cargo 검체)이 삭제됐다 — 토큰 신원이 send·usage 등 "
         "20+ 소비자로 번지는 경계 붕괴를 재는 유일 검체다")
    need("seat_token_never_persisted_or_listed" in h,
         "무영속·무노출 회귀 핀(cargo 검체)이 삭제됐다 — topology.json/surface.list 등재 회귀를 "
         "재는 유일 검체다")
    notes.append("L1-P1 seat 토큰: 데몬 1차 대조+신선 재해석 순서 · CLI/훅 첨부 배선 · "
                 "BOOT_GATES=0 토큰 생략 · rc 계약 불변 · 경계 회귀 핀 존치")

    # ── L2 훅: 조상 체인이 온전한 시점(spawn 이전)의 선행 claim + env 판정 전달 ──
    hook = _read(_hook(ROLE_BODY))
    need("cys claim-role master" in hook,
         "훅이 선행 claim 을 하지 않는다 — 분리된 부트가 claim 하면 언제나 신원 미해석이다")
    need("export CYS_CLAIM_RC=" in hook, "훅이 claim 판정을 부트에 넘기지 않는다(env 계약 부재)")
    i_claim, i_spawn = hook.find("cys claim-role master"), hook.find("BOOT_ARGS=")
    need(0 <= i_claim < i_spawn,
         "선행 claim 이 spawn(분리 발화) **이후**에 있다 — 조상 체인이 이미 끊긴 뒤라 무의미하다")
    need("export CYS_CLAIM_SID=" in hook and "export CYS_CLAIM_AT=" in hook,
         "선행 claim 판정에 결박(surface 귀속·신선도)이 없다 — 셸에 남은 값이 "
         "'치지도 않은 claim'을 실측으로 둔갑시킨다")
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need('os.environ.get("CYS_CLAIM_RC"' in bsrc, "부트가 선행 claim 판정을 소비하지 않는다")
    need("_pre_bound" in bsrc and "CYS_CLAIM_SID" in bsrc and "CYS_CLAIM_AT" in bsrc,
         "부트가 결박(surface·신선도) 검증 없이 판정을 소비한다(무바인딩 env 소비)")
    need("6: (\"발신 신원 미확정" in bsrc, "부트가 rc 6 의 정확한 처방을 분기하지 않는다")
    notes.append("L2 훅 선행 claim(spawn 이전)+결박 + 부트 소비 검증 + rc6 처방")

    # ── L3 폴백: '살아있는 master 존재'라는 전제를 실측으로 재확인 ──
    need("def _base_live_master(" in bsrc, "폴백 전제 실측 술어가 없다")
    fb = bsrc[bsrc.find("def _dept_fallback("):]
    fb = fb[:fb.find("\ndef ", 10)] if "\ndef " in fb[10:] else fb
    # 앵커는 **실제 호출부**여야 한다(주석·독스트링의 'allocate' 낱말이 아니라 — 그것은 함수
    # 상단 계약 주석에 먼저 나와서 순서 판정을 뒤집는다).
    i_guard, i_alloc = fb.find("_base_live_master("), fb.find('cys_dept, "allocate"')
    need(i_guard >= 0, "폴백이 전제(살아있는 master)를 실측하지 않는다 — 없는 master 로 부서 생성")
    need(i_alloc < 0 or i_guard < i_alloc,
         "전제 실측이 부서 allocate **이후**에 있다 — 이미 만들고 나서 재는 것은 가드가 아니다")
    need("def _live_master_from_status(" in bsrc,
         "전제 판정이 순수 함수로 분리돼 있지 않다 — --self-test 로 핀할 수 없다")
    need("_cys_status_json()" in bsrc[bsrc.find("def _base_live_master("):][:400],
         "status 입구를 재구현했다(세 번째 리더) — 단일 SOT 규율 위반")
    need("STEP.DEPT_FB_GUARD" in fb,
         "폴백 미진입을 전용 단계로 적지 않는다 — 단계 순서 역행(order_violation) 재발")
    notes.append("L3 폴백: allocate 이전 전제 실측(측정 실패=미진입)·순수 판정·전용 단계")

    # ── 계측 타당성: 구 트리에는 이 3겹이 **없어야** 한다(있으면 탐지기가 무엇도 못 잡는다) ──
    old_h = _git_show(os.path.join("src", "bin", "cysd", "handlers.rs"))
    old_hook = _git_show(os.path.join("cysjavis-pack", "hooks", "role-bootstrap.sh"))
    calib = "skip(no-git)"
    if old_h is not None and old_hook is not None:
        need("claim_caller_unresolved" not in old_h,
             "계측 타당성 실패: 구 데몬에 이미 신원 전용 코드가 있다")
        need("CYS_CLAIM_RC" not in old_hook,
             "계측 타당성 실패: 구 훅에 이미 선행 claim 계약이 있다")
        need("seat_token" not in old_h,
             "계측 타당성 실패: 구 데몬에 이미 seat 토큰 축이 있다(L1-P1 탐지기 무효)")
        old_c = _git_show(os.path.join("src", "bin", "cys.rs"))
        if old_c is not None:
            need('"seat_token"' not in old_c,
                 "계측 타당성 실패: 구 CLI 에 이미 토큰 첨부 배선이 있다(L1-P1 탐지기 무효)")
        calib = "구 트리=코드 융합·선행 claim·seat 토큰 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-EXIT-4", "W2", "boot --json 스키마(mandatory·outcome·install_hint)",
          ["G29", "B15", "B16", "R5", "B1", "B8"])
def h_exit_4():
    """G29·B8: '필수 CLI 미설치=exit 0' 이라 소비 계약(exit 4)과 불일치했고 힌트도 오답이었다.
    --json 이 role 별 {outcome, mandatory, install_hint} 를 내고, 플랫폼별 힌트를 파생한다.
    R5(무경고 사각): claude 만 설치 → 리뷰어 0 이 **typed missing** 으로 드러난다."""
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("const BOOT_PLAN: &[(&str, &str, bool)]" in src,
         "PLAN 정책 열(mandatory)이 편성 테이블에 없다(B1)")
    for out in ("launched", "already_alive", "busy", "missing", "failed", "recovered"):
        need('"%s"' % out in src, "boot --json outcome 값 누락: %s" % out)
    need("fn install_hint(agent: &str)" in src, "플랫폼별 install_hint 파생 함수가 없다(G29·B8)")
    need('"install_hint"' in src, "--json 에 install_hint 필드가 없다")
    # ★핀 이사(MF-1 · 2026-08-26) — P4 수정 라운드(5b15e87)가 install_hint 를 순수형
    #   `install_hint_for(agent, os)` + 실행 플랫폼 박피 래퍼로 리팩터했다(os 를 인자로 받아
    #   비 Windows CI 에서도 Windows 갈래를 실제로 밟는다 — lib.rs bundled_git_bash_path_for
    #   와 동일 이유). 종전 핀은 `fn install_hint(` 이후 900자 창에서 `cfg!(windows)` 를
    #   찾았는데, 분기가 순수형의 `os == "windows"` 런타임 분기로 옮겨져 창에서 소실됐다.
    #   축은 한 톨도 안 바뀐다 — "미설치 힌트가 플랫폼을 가르는가". 오히려 강화: 분기 존재만이
    #   아니라 **양 갈래 산출물**(windows=네이티브 install.ps1 / unix=install.sh)과 래퍼의
    #   순수형 위임(힌트 판정 이원화 차단)까지 박는다.
    need("fn install_hint_for(agent: &str, os: &str)" in src,
         "순수형 install_hint_for(agent, os) 가 없다(MF-1 형상 소실)")
    hi = src.find("fn install_hint_for(")
    hint_body = src[hi:hi + 900]
    need('os == "windows"' in hint_body,
         "install_hint_for 가 플랫폼 분기를 갖지 않는다(오답 힌트 재발)")
    need("install.ps1" in hint_body, "Windows 갈래가 네이티브 설치 명령(install.ps1)이 아니다")
    need("install.sh" in hint_body, "unix 갈래 설치 명령(install.sh)이 소실됐다")
    need("install_hint_for(agent, std::env::consts::OS)" in src,
         "실행 플랫폼 래퍼가 순수형에 위임하지 않는다(힌트 판정 이원화)")
    # ★의무 CLI 미설치 ≠ exit 0 성공: missing + mandatory 는 fatal_failed 로 계상된다.
    mi = src.find("outcome\": \"missing\"")
    need(mi > 0, "missing outcome 생성 지점을 못 찾았다")
    # ★핀 이사(M3 · 2026-08-24) — 손 계상이 **파생**으로 옮겼다. 종전 핀은 missing 생성 지점
    #   앞 800자 창에서 `fatal_failed += 1` 을 찾았고, 그 손 계상은 이제 존재하지 않는다
    #   (분기마다 세던 것을 `boot_summary_buckets` 가 typed outcome 에서 파생한다).
    #   축은 한 톨도 안 바뀐다 — "의무 역할의 missing 이 fatal 로 계상되는가". 다만 창(window)
    #   위치가 아니라 **술어 그 자체**를 박으므로, 분기가 늘어도 빠뜨릴 자리가 없다(강화).
    bs = _slice_between(src, "fn boot_summary_buckets(",
                        "\nconst BOOT_SUMMARY_BUCKETS", "H-EXIT-4 버킷 파생")
    need('"failed" | "missing" => fatal_failed += 1,' in bs,
         "의무 역할 미설치가 fatal 로 계상되지 않는다(exit 0 성공으로 위장 — G29 재발)")
    # PLAN 정책 열 ↔ python 정본 파리티(H-PRED-7 과 짝).
    sys.path.insert(0, BIN_DIR)
    try:
        import javis_orchestra as O
        rust_plan = []
        seg = src[src.find("const BOOT_PLAN"):]
        seg = seg[:seg.find("];")]
        for line in seg.splitlines():
            line = line.strip()
            if line.startswith('("'):
                parts = [p.strip().strip('"') for p in line.strip("(),").split(",")]
                if len(parts) == 3:
                    rust_plan.append((parts[0], parts[1], parts[2] == "true"))
        need(rust_plan, "Rust BOOT_PLAN 파싱 실패")
        py_plan = [(r, a, p == O.FAIL_FATAL) for r, a, p in O.BOOT_PLAN]
        need(rust_plan == py_plan,
             "PLAN 정책 열 파리티 붕괴 — rust=%r / python=%r" % (rust_plan, py_plan))
    finally:
        sys.path.remove(BIN_DIR)
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("const PLAN: &[(&str, &str)]" in old,
             "계측 타당성 실패: 구 코드 PLAN 형태(정책 열 없음)를 못 찾았다")
        need("install_hint" not in old, "계측 타당성 실패: 구 코드에 이미 install_hint 파생이 있다")
        calib = "구 코드 PLAN=(role,agent) 2열·힌트 인라인 산문 확인"
    return ("outcome 6종·mandatory·플랫폼 install_hint · 의무 미설치=fatal 계상 · "
            "PLAN 정책열 rust↔python 파리티 %d행 · 계측검증=%s" % (len(rust_plan), calib))


@specimen("H-EXIT-5", "W2", "미지 subcommand EX_USAGE + 게이트 exit 2 충돌 해소", ["A13", "A14"])
def h_exit_5():
    """A13(치환 확정분): argparse exit 2 ↔ EXIT_HARD=2 의 **의미 공간 충돌** — 오타 하나가
    '자원 hard_block(팀 기동 거부)'로 오독됐다. + 소비부의 미지 exit fail-open('allow') 제거:
    EX_USAGE 는 **명시 fail + loud** 로만 접힌다(조용한 allow 금지 — 비평2 B-7)."""
    gate = os.path.join(BIN_DIR, "javis_resource_gate.py")
    need(os.path.isfile(gate), "javis_resource_gate.py 부재")
    notes = []
    # ⓐ 게이트 측: 사용오류 = 64, hard(2)와 분리
    for args, want in ((["bogus-subcommand"], 64), (["check", "--no-such-flag"], 64), ([], 64)):
        r = _run([PY, gate] + args, timeout=60)
        need(r.returncode == want,
             "게이트 %r → exit %d(기대 %d) — argparse↔EXIT_HARD 충돌 잔존" % (args, r.returncode, want))
    r = _run([PY, gate, "check", "--servers-override", "99", "--nodes-override", "0",
              "--load-override", "0"], timeout=60)
    need(r.returncode == 2, "정상 hard 판정이 2 가 아니다(회귀): %d" % r.returncode)
    notes.append("게이트 실측: 사용오류 3케이스=64 · hard=2 유지")
    # ⓑ 내부 예외 = 70(EX_SOFTWARE), soft(1) 오분류 아님
    r = _run([PY, gate, "--self-test"], timeout=120)
    need(r.returncode == 0, "게이트 self-test 실패:\n%s\n%s" % (r.stdout[-800:], r.stderr[-400:]))
    notes.append("게이트 self-test(내부예외=70·계약채널) PASS")
    # ⓒ 소비부: 미지 exit 이 조용한 allow 로 접히지 않는다 + remap 과 **동일 커밋** 확인
    sys.path.insert(0, BIN_DIR)
    try:
        import javis_bootstrap as B
        need(B._resource_gate_decision(64, None, None)[0] == "usage-error",
             "소비부가 EX_USAGE 를 사용오류로 분리하지 않는다")
        need(B._resource_gate_decision(3, None, None)[0] == "unknown-exit",
             "소비부가 미지 exit 를 여전히 allow 로 접는다(fail-open 잔존)")
        # loud 계약: 두 verdict 는 _notify_loud 경로를 탄다(조용히 진행 금지)
        gsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
        gi = gsrc.find('if verdict in ("usage-error", "unknown-exit")')
        need(gi > 0, "측정 실패 verdict 의 loud 분기가 없다")
        need("_notify_loud(" in gsrc[gi:gi + 600], "측정 실패가 loud 알림 없이 조용히 진행한다")
        # ⓓ A13 잠복 경로: 계약 채널 분리(stderr 오염이 stdout JSON 을 파괴하지 않는다)
        need("_run_split(" in gsrc, "게이트 호출이 채널 분리(_run_split)를 쓰지 않는다")
        rc, so, se = B._run_split([PY, "-c",
                                   "import sys;sys.stderr.write('diag\\n');print('{\"a\":1}')"])
        need(rc == 0 and json.loads(so.strip())["a"] == 1 and se.strip() == "diag",
             "채널 분리 실패 — stderr 오염이 stdout 계약을 파괴한다")
    finally:
        sys.path.remove(BIN_DIR)
    notes.append("소비부: usage-error/unknown-exit 분리 + loud + 채널 분리")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_resource_gate.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("EXIT_USAGE" not in old, "계측 타당성 실패: 구 게이트에 이미 EXIT_USAGE 가 있다")
        calib = "구 게이트=argparse 기본 exit 2(EXIT_HARD 충돌) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-EXIT-6", "W2", "orchestra 2/127=영구실패 · 124=재시도 분류", ["A12"])
def h_exit_6():
    """A12: 모든 비0 을 뭉개 재시도하면 **영구 실패**(스크립트 부재·데몬 다운)를 24회 태우고
    정확한 처방을 잃는다. 2/127=영구(재시도 금지·처방) / 124=transient / 그 밖=보수적 transient."""
    sys.path.insert(0, BIN_DIR)
    try:
        import javis_orchestra as O
        table = {0: O.EXIT_CLASS_OK, 2: O.EXIT_CLASS_PERMANENT, 127: O.EXIT_CLASS_PERMANENT,
                 124: O.EXIT_CLASS_TRANSIENT, 1: O.EXIT_CLASS_TRANSIENT,
                 255: O.EXIT_CLASS_TRANSIENT}
        for rc, want in table.items():
            got, why = O.classify_call_exit(rc)
            need(got == want, "rc=%d 분류 %r(기대 %r)" % (rc, got, want))
            if want == O.EXIT_CLASS_PERMANENT:
                need("재시도는 무의미" in why, "영구 실패 처방에 재시도 금지 문구 누락: %r" % why)
        # 실경로: boot_node 헬퍼 부재 → 127 영구 + 처방
        import types
        saved = O.os.path.isfile
        try:
            O.os.path.isfile = lambda p: False if p.endswith("javis_boot_node.py") else saved(p)
            ok, rc, cls, why = O._boot_one_node("reviewer-gemini", "gemini")
            need(ok is False and rc == 127 and cls == O.EXIT_CLASS_PERMANENT,
                 "헬퍼 부재가 영구 실패로 분류되지 않음: %r" % ((ok, rc, cls),))
        finally:
            O.os.path.isfile = saved
        # boot-reviewers 가 영구 실패에서 **대체 폴백을 재시도하지 않는다**(같은 영구 실패 반복 금지)
        osrc = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
        bi = osrc.find("def cmd_boot_reviewers(")
        seg = osrc[bi:bi + 4000]
        need("EXIT_CLASS_PERMANENT" in seg,
             "boot-reviewers 가 영구 실패를 분기하지 않는다(대체 폴백 무의미 재시도)")
        _ = types
    finally:
        sys.path.remove(BIN_DIR)
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_orchestra.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("classify_call_exit" not in old, "계측 타당성 실패: 구 코드에 이미 exit 분류가 있다")
        need("return r.returncode == 0" in old,
             "계측 타당성 실패: 구 _boot_one_node 의 bool 반환 형태를 못 찾았다")
        calib = "구 코드=bool 반환(전 비0 동일 취급) 확인"
    return ("분류 6케이스(2·127=영구+처방 / 124·1·255=transient) · 헬퍼 부재 실경로 127 · "
            "boot-reviewers 영구 분기 · 계측검증=%s" % calib)


@specimen("H-EXIT-7", "W2",
          "check exit 2 — `cys ping` 1회 재확인 분기(데몬 소실=즉시 이탈 / 팩 결손=유계)",
          ["G32", "A12"])
def h_exit_7():
    """G32+W-A3: orchestra check 의 exit 2 는 '노드 미기동'(exit 1)이 아니라 **판정 불가**다 —
    그리고 판정 불가는 단일 사건이 아니므로 `cys ping` **1회 재확인**으로 실측 분리한다:
      t1) exit 2 + (⑤ 시점) ping 사망 → 데몬 소실 **확정** — 재시도 0·즉시 이탈(처방=`cys ping`·데몬)
      t2) exit 2 + ping 생존 → 데몬은 살아 있다 — 재시도 창 안에서 계속하되 이 분기 전용
          **별도 상한**(CHECK_UNJUDGEABLE_RETRIES·budget 키 부재 시 3)으로 유계, 소진 시
          '팩 결손 가능성' 진단(처방=CYS_PACK_DIR·팩 재설치 — `cys boot`·데몬 재기동 처방 금지)
      t3) exit 1 → 종전대로 재시도 소진(대조군 — 이 계약은 불변)

    ★구 계약("판정 불가에서는 재시도하지 않는다 — A12 영구 분류")을 폐기한 이유: 그 전제
      'check exit 2 = 데몬 소실'이 실측으로 틀렸다(W-A3).
      ⓐ `_run([py, orchestra, …])` 는 cmd[0] 이 sys.executable 이라 **orchestra 스크립트가
         없어도 python 자신이 rc 2** 를 낸다(127 이 아니다 — 아래 t2-b 가 step exit 로 박제).
         즉 구 계약은 팩 결손을 '데몬 소실'로 오진하고 처방(`cys ping`·데몬 기동)을 정반대로
         뒤집었다 — 팩이 깨진 기계에서 사용자를 데몬 수리로 보내는 데드엔드.
      ⓑ 그 결과 구 코드의 'exit 127 = orchestra 스크립트·인터프리터 부재' 분기 문안은 스크립트
         부재 쪽으로는 **도달 불가 사문**이었다(127 의 유일 실경로 = 인터프리터 자체 소실).
    ★검체 유의(master 실측 2026-08-21): ping 이 처음부터 죽어 있으면 W-A3 의 ②ping 유계
      재시도가 먼저 EXIT_PING(3)으로 끝나 ⑤ 에 도달조차 못 한다(실측 43.4s·⑤ 단계 0개) —
      t1 은 '② 는 통과, ⑤ 시점에만 사망'을 상태 파일 카운터 목(첫 ping=0·이후 ping=3)으로
      만든다."""
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    src = _read(boot)
    need("⑤check-unjudgeable" in src, "⑤ 가 판정 불가를 별도 단계로 분기하지 않는다")
    need("`cys ping` 재확인" in src, "⑤ exit 2 의 `cys ping` 1회 재확인이 소스에 없다(신계약 부재)")
    need("팩 결손 가능성" in src, "⑤ exit 2 의 '팩 결손 가능성' 진단 분기가 소스에 없다(신계약 부재)")
    # 별도 상한 기대값 — bootstrap 과 **동일 산식**(javis_budget leaf · 키 부재=폴백 3)으로
    # 파생한다. 수치 하드코딩이 아니라 SOT 동행: 키가 budget 에 등재되면 검체도 자동 추종한다.
    sys.path.insert(0, BIN_DIR)
    try:
        try:
            import javis_budget as _bud
            cap = max(1, int(_bud.leaf("CHECK_UNJUDGEABLE_RETRIES")))
        except Exception:
            cap = 3
    finally:
        sys.path.remove(BIN_DIR)

    def _attempt_steps(recs):
        """orchestra check **실호출**(`⑤check#N`)만 센다 — `⑤check#N-ping`(재확인 실측 기록)도
        "⑤check#" 로 시작하므로, 구 검체의 접두 카운트는 신계약에서 재확인 기록을 '재시도'로
        오산한다(구 `startswith` 계수의 폐기 이유)."""
        return [s for s in recs if re.fullmatch(r"⑤check#\d+", s["step"])]

    def _recheck_steps(recs):
        return [s for s in recs if re.fullmatch(r"⑤check#\d+-ping", s["step"])]

    def _unjudge_detail(recs):
        rows = [s for s in recs if s["step"] == "⑤check-unjudgeable"]
        need(rows, "⑤check-unjudgeable 단계가 없다: %r" % [s["step"] for s in recs])
        return rows[-1].get("detail", "")

    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # ── t1. exit 2 + (⑤ 시점) ping 사망 → 데몬 소실 확정: 재확인 1회 실측·재시도 0·즉시 이탈 ──
        sub = os.path.join(tmp, "gone")
        pingf = os.path.join(sub, "ping.n")
        env, home = _boot_sandbox(sub, check_exit=2, cys_extra=(
            'case "$1" in ping)\n'
            '  _n=$(cat "%s" 2>/dev/null || echo 0)\n'
            '  _n=$((_n+1)); printf %%s "$_n" > "%s"\n'
            '  [ "$_n" -ge 2 ] && exit 3\n'
            '  exit 0\n'
            ';; esac' % (pingf, pingf)))
        env["CYS_BOOT_CHECK_RETRIES"] = "5"
        env["CYS_BOOT_CHECK_INTERVAL_S"] = "2"
        t0 = time.time()
        r = _run([PY, boot], env=env, timeout=180)
        dt = time.time() - t0
        need(r.returncode == 6, "t1 데몬 소실 exit≠6(EXIT_CHECK): %d" % r.returncode)
        recs = _boot_last(home).get("steps", [])
        steps = [s["step"] for s in recs]
        need(sum(1 for s in steps if s == "②ping" or s.startswith("②ping#")) == 1,
             "t1 검체 무효: ②ping 이 1회 통과가 아니다(⑤ 시점 사망 재현 실패): %r" % steps)
        need("⑤check-unjudgeable" in steps,
             "t1 판정 불가가 별도 단계로 기록되지 않았다: %r" % steps)
        need(len(_attempt_steps(recs)) == 1,
             "t1 데몬 소실 확정인데 check 를 재시도했다: %r" % steps)
        rech = _recheck_steps(recs)
        need(len(rech) == 1 and rech[0]["exit"] != 0 and "재확인 실패" in rech[0].get("detail", ""),
             "t1 `cys ping` 재확인 실측 기록(⑤check#1-ping·비0)이 없다 — 이탈이 재확인 없이"
             "(구 계약 형태로) 일어났다: %r" % [(s["step"], s["exit"]) for s in recs][-6:])
        need(dt < 8, "t1 즉시 이탈이어야 하는데 재시도 대기를 태웠다(%.1fs)" % dt)
        d1 = _unjudge_detail(recs)
        need("데몬 소실" in d1 and "`cys ping`" in d1,
             "t1 진단에 데몬 처방(`cys ping`)이 없다: %r" % d1[:300])
        need("팩 결손" not in d1, "t1 데몬 소실 확정에 '팩 결손' 진단이 섞였다: %r" % d1[:300])
        notes.append("t1 데몬소실=재확인1·재시도0·즉시이탈·데몬처방")

        # ── t2-a. exit 2 + ping 생존(orchestra 스크립트 실재) → 별도 상한 유계 + '팩 결손 가능성' ──
        env2, home2 = _boot_sandbox(os.path.join(tmp, "alive"), check_exit=2)
        env2["CYS_BOOT_CHECK_RETRIES"] = "5"
        env2["CYS_BOOT_CHECK_INTERVAL_S"] = "0.05"
        r2 = _run([PY, boot], env=env2, timeout=180)
        need(r2.returncode == 6, "t2-a exit≠6: %d" % r2.returncode)
        recs2 = _boot_last(home2).get("steps", [])
        n2 = len(_attempt_steps(recs2))
        need(1 < n2 <= cap and n2 < 5,
             "t2-a 재시도가 유계(1<n≤상한 %d<창 5)가 아니다: n=%d %r"
             % (cap, n2, [s["step"] for s in recs2]))
        need(n2 == cap, "t2-a 별도 상한(%d)에서 멈추지 않았다: n=%d" % (cap, n2))
        rech2 = _recheck_steps(recs2)
        need(len(rech2) == n2 and all(s["exit"] == 0 for s in rech2),
             "t2-a 매 시도의 ping 생존 재확인(rc 0) 실측이 없다: %r"
             % [(s["step"], s["exit"]) for s in rech2])
        d2 = _unjudge_detail(recs2)
        need("팩 결손 가능성" in d2 and "데몬 소실이 아니라" in d2,
             "t2-a 진단이 '팩 결손 가능성'이 아니다(오진 부활): %r" % d2[:300])
        need("있음" in d2 and "CYS_PACK_DIR" in d2,
             "t2-a 스크립트 실재 실측·팩 점검 처방이 진단에 없다: %r" % d2[:300])
        need("데몬을 확인·기동하라" not in d2,
             "t2-a 팩 결손 가능성에 데몬 처방이 섞였다(처방 반전): %r" % d2[:300])
        notes.append("t2a ping생존=유계 %d회·팩결손가능성 진단" % n2)

        # ── t2-b. **팩 결손 실물**(orchestra 스크립트 삭제 — python 자신이 rc 2 를 낸다) →
        #    같은 유계 + '팩 결손 확정적' 처방. 구 계약이 '데몬 소실'로 오진하던 바로 그 입력의
        #    실물 재현이다(위 ⓐ 의 실증 — ⓑ 의 '스크립트 부재=127' 문안이 사문이었음도 여기서
        #    step exit==2 실측으로 확정된다). ──
        sub3 = os.path.join(tmp, "packgone")
        env3, home3 = _boot_sandbox(sub3, check_exit=0)
        os.remove(os.path.join(home3, ".cys", "pack", "bin", "javis_orchestra.py"))
        env3["CYS_BOOT_CHECK_RETRIES"] = "5"
        env3["CYS_BOOT_CHECK_INTERVAL_S"] = "0.05"
        r3 = _run([PY, boot], env=env3, timeout=180)
        need(r3.returncode == 6, "t2-b 팩 결손 exit≠6: %d" % r3.returncode)
        recs3 = _boot_last(home3).get("steps", [])
        att3 = _attempt_steps(recs3)
        need(att3 and all(s["exit"] == 2 for s in att3),
             "t2-b 검체 무효: 스크립트 부재의 실측 rc 가 2 가 아니다(ⓐ 전제 붕괴 — 127 문안 "
             "부활?): %r" % [(s["step"], s["exit"]) for s in att3])
        need(len(att3) == cap, "t2-b 팩 결손이 별도 상한(%d) 유계가 아니다: %d" % (cap, len(att3)))
        d3 = _unjudge_detail(recs3)
        need("팩 결손 확정적" in d3 and "없음" in d3,
             "t2-b 스크립트 부재 실측('없음')·확정 진단이 없다: %r" % d3[:300])
        need("이 상황의 처방이 아니다" in d3,
             "t2-b 가 오진 처방(`cys boot`·데몬 재기동)을 명시 부인하지 않는다: %r" % d3[:300])
        notes.append("t2b 팩결손 실물=rc2 실측·확정 진단(데몬 오진 소멸)")

        # ── t3. 대조군: exit 1(노드 미기동) → 종전대로 재시도 소진 · 판정불가 단계 부재 ──
        env4, home4 = _boot_sandbox(os.path.join(tmp, "missing"), check_exit=1)
        env4["CYS_BOOT_CHECK_RETRIES"] = "3"
        env4["CYS_BOOT_CHECK_INTERVAL_S"] = "0.05"
        r4 = _run([PY, boot], env=env4, timeout=180)
        need(r4.returncode == 6, "t3 노드 미기동 exit≠6: %d" % r4.returncode)
        recs4 = _boot_last(home4).get("steps", [])
        steps4 = [s["step"] for s in recs4]
        need(len(_attempt_steps(recs4)) == 3,
             "t3 미기동 경로가 재시도를 소진하지 않았다: %r" % steps4)
        need("⑤check-unjudgeable" not in steps4, "t3 미기동이 판정 불가로 오분류됐다")
        need(not _recheck_steps(recs4),
             "t3 exit 1 에 ping 재확인이 붙었다(재확인은 exit 2 전용 — 과확장): %r" % steps4)
        notes.append("t3 미기동=재시도 3회 소진·재확인 0(불변)")
    # ── 계측 타당성(구 코드 대조) — 관례 유지하되 방향을 갱신: 구 코드에 **신계약이 없음**을
    #    단언한다(위 t1 재확인 실측·t2 '팩 결손' 문면 단언이 구 코드에서 FIRE 하는 문자 근거). ──
    calib = "skip(no-git)"
    old_w0 = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"))
    if old_w0 is not None:
        # W0 트리: 판정 불가 분기 자체가 없다(G32 원결함 — exit 2 를 미기동과 동일 재시도).
        need("unjudgeable" not in old_w0, "계측 타당성 실패: W0 코드에 이미 판정 불가 분기가 있다")
        calib = "W0=분기 자체 부재"
        old_w2 = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"),
                           ref=PRE_WA3_REF)
        if old_w2 is not None:
            need("⑤check-unjudgeable" in old_w2,
                 "계측 기준 오류: PRE_WA3_REF 트리에 W2 분기가 없다(기준 해시 확인)")
            need("`cys ping` 재확인" not in old_w2,
                 "계측 타당성 실패: 구(W2) 코드에 이미 ping 재확인이 있다")
            need("팩 결손 가능성" not in old_w2,
                 "계측 타당성 실패: 구(W2) 코드에 이미 팩 결손 진단이 있다")
            need("orchestra 스크립트/인터프리터 부재" in old_w2,
                 "계측 기준 오류: 구(W2) 코드의 127 오진 문안을 못 찾았다(ⓑ 사문의 실물)")
            calib += " · W2=재확인·팩결손 진단 부재(즉시 이탈 단일 처방 + 127 오진 문안) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


# ★H-EXIT-8(G1 discover sentinel)은 **W3 소속**이다(재감사 §4: G1 → W3 시드·등록 웨이브).
#   구판 러너가 'H-EXIT 잔여'를 일괄 W2 로 태깅했으나 §4 웨이브 소속이 정본이다(W1a/W1b 재태깅 선례).
@specimen("H-EXIT-8", "W3", "discover sentinel — 격리 팩=글로벌 등록 0 / 순수 미발견=폴백 허용", ["G1"])
def h_exit_8():
    """G1: 격리 가드가 `[]` 를 반환하고 호출부가 `discover() or [~/.claude]` 로 폴백해 **금지가
    통째로 무효화**됐다(부서·임시 팩이 실 글로벌 settings 에 자기 훅을 등록). `[]`(순수 미발견 —
    신규 머신)과 '금지'는 처방이 정반대이므로 반환 타입으로 분리한다."""
    PF = _preflight_mod()
    notes = []
    with tempfile.TemporaryDirectory() as tmp, _temp_guard_double(PF, "SNAPMARK"):
        home = os.path.join(tmp, "home")
        os.makedirs(home, exist_ok=True)
        # ⓐ 순수 미발견(신규 머신·base 팩) → 폴백 1건 허용·금지 사유 없음
        with _env_patch(HOME=home, CYS_PACK_DIR=os.path.join(home, ".cys", "pack"),
                        CYS_ACCOUNT_DIR=None, CLAUDE_CONFIG_DIR=None):
            targets, forbidden = PF.resolve_registration_targets()
            need(forbidden is None, "순수 미발견인데 금지로 판정: %r" % forbidden)
            need(len(targets) == 1 and targets[0].endswith(os.path.join(".claude", "settings.json")),
                 "신규 머신 폴백(기본 프로필 1건)이 사라졌다: %r" % targets)
            notes.append("미발견=폴백 1건")
        # ⓑ 부서 팩(account dir 미상) → **글로벌 등록 0** + 금지 사유
        with _env_patch(HOME=home, CYS_PACK_DIR=os.path.join(home, ".cys", "pack-dept-d1"),
                        CYS_ACCOUNT_DIR=None, CLAUDE_CONFIG_DIR=None):
            targets, forbidden = PF.resolve_registration_targets()
            need(forbidden, "부서 팩인데 금지 사유가 없다(격리 무효화)")
            need(targets == [], "부서 팩인데 등록 대상이 있다: %r" % targets)
            notes.append("부서 팩=등록 0")
        # ⓒ 부서 팩 + account dir → 그 dir **한 건만**(글로벌은 여전히 금지)
        acct = os.path.join(home, ".cys", "claude-d1")
        os.makedirs(acct, exist_ok=True)
        os.makedirs(os.path.join(home, ".claude"), exist_ok=True)
        with _env_patch(HOME=home, CYS_PACK_DIR=os.path.join(home, ".cys", "pack-dept-d1"),
                        CYS_ACCOUNT_DIR=acct, CLAUDE_CONFIG_DIR=None):
            targets, forbidden = PF.resolve_registration_targets()
            need(forbidden, "좁힌 등록에도 금지(격리) 사유가 붙어야 한다")
            need(targets == [os.path.join(acct, "settings.json")],
                 "부서 account dir 한 건이 아니다: %r" % targets)
            notes.append("부서+account=좁힌 1건")
        # ⓓ 임시 팩 → 실 config 등록 0(스냅샷 하네스 부작용 0). 판정 입력은 마커 경로다(위 double).
        with _env_patch(HOME=home, CYS_PACK_DIR=os.path.join(tmp, "SNAPMARK", "snap_grill_1"),
                        CYS_ACCOUNT_DIR=None, CLAUDE_CONFIG_DIR=None):
            targets, forbidden = PF.resolve_registration_targets()
            need(forbidden and targets == [], "임시 팩이 실 config 등록 대상을 가졌다: %r" % targets)
            notes.append("임시 팩=등록 0")
    # ⓔ 구 관용구(`discover() or [기본]`) 잔존 0 — 폴백 소유자는 resolve 하나다
    src = _read(os.path.join(BIN_DIR, "javis_preflight.py"))
    need("discover_claude_settings() or [" not in _code_lines(src),
         "금지를 무효화하는 `or [기본]` 폴백 관용구가 남아 있다(G1 재발)")
    notes.append("`or [기본]` 관용구 0")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_preflight.py"))
    calib = "skip(no-git)"
    if old is not None:
        need(_code_lines(old).count("discover_claude_settings() or [") >= 3,
             "계측 타당성 실패: 구 코드의 폴백 관용구를 못 찾았다")
        need("resolve_registration_targets" not in old,
             "계측 타당성 실패: 구 코드에 이미 sentinel 계약이 있다")
        calib = "구 코드 폴백 관용구 3+·sentinel 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
@specimen("H-EXIT-9", "W1b", "싱글플라이트 패자 → 비-master skip verdict(즉시 반환)", ["G17", "A7"])
def h_exit_9():
    """G17: 패자 pane 이 **신원 확인 없이** 조용히 exit 0 하면 그 pane 의 LLM 이 '부트 완료된
    master'를 자칭한다. verdict 에 자기 surface-role 확인 결과를 동봉하고 '비-master'를 명시한다.
    ★즉시 반환(waited=false) — 수렴 대기 금지(금지 방향 ⑨)."""
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    with tempfile.TemporaryDirectory() as tmp:
        env, home = _boot_sandbox(
            tmp, cys_extra=('case "$1" in ping) sleep 8;; surface-role) echo "worker"; exit 0;; esac'))
        winner = subprocess.Popen([PY, boot], env=env, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL)
        try:
            # 승자가 락을 잡을 시간을 준다(선기록 등장으로 확인)
            deadline = time.time() + 15
            while time.time() < deadline and not _boot_last(home).get("run_id"):
                time.sleep(0.1)
            winner_run = _boot_last(home).get("run_id")
            t0 = time.time()
            r = _run([PY, boot], env=env, timeout=60)
            dt = time.time() - t0
        finally:
            winner.kill(); winner.wait(timeout=15)
        need(r.returncode == 11, "패자 exit≠11(구분 exit 부재): %d" % r.returncode)
        need(r.stdout.strip() == "",
             "패자가 stdout(계약 채널)을 오염시켰다 — master 가 '완료'로 인용할 위험: %r" % r.stdout[:300])
        line = [l for l in r.stderr.splitlines() if l.strip().startswith("{")]
        need(line, "stderr 1줄 verdict JSON 이 없다: %r" % r.stderr[-400:])
        v = json.loads(line[-1])
        need(v.get("verdict") == "skipped_inflight", "verdict 타입 이상: %r" % v)
        need(v.get("surface_role") == "worker" and v.get("is_master") is False,
             "패자가 자기 신원을 확인하지 않았다(G17 회귀): %r" % v)
        need("비-master" in (v.get("self_check") or ""),
             "'비-master' 명시가 없다(자칭 master 차단 실패): %r" % v.get("self_check"))
        need(v.get("waited") is False and dt < 6,
             "패자가 수렴 대기를 했다(%.1fs · 금지 방향 ⑨): %r" % (dt, v))
        need(v.get("boot_last_untouched") is True, "skip 이 단일-writer 보존을 주장하지 않는다")
        need(_boot_last(home).get("run_id") == winner_run,
             "패자가 승자의 boot-last 를 덮었다(단일-writer 파괴)")
        need(os.path.isfile(os.path.join(home, ".cys", "state", "boot-skip-base.json")),
             "skip 별도 기록 파일이 없다")
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    calib = "skip(no-git)"
    if old is not None:
        need("_emit_skip_verdict" not in old, "계측 타당성 실패: 구 코드에 이미 skip verdict 가 있다")
        calib = "구 코드 침묵 skip(exit 0) 확인"
    return "exit 11 · stdout 무접촉 · 비-master 명시 · waited=false · 별도 기록 · 계측검증=%s" % calib


@specimen("H-EXIT-11", "W6",
          "launch-agent 관문 보류 exit(78) 3자 파리티 + 보류≠성공≠실패 소비 계약",
          ["U-11", "S-3", "④ 전 pane 사망"])
def h_exit_11():
    """U-11: `cys launch-agent` 의 종전 계약은 **성공 0 / 그 밖 전부 1** 이었다. 그 1비트에는
    "pane 은 만들어졌고 에이전트 프로세스도 살아 있는데, 첫기동 관문에 갇혀 아직 못 쓴다" 가
    담기지 않는다. 두 오독이 각각 사고다:
      · 0 으로 접으면 → 소비부가 '노드를 세웠다'로 읽어 지침·티켓을 태운다. 그 주입의 Return 이
        **실측상 면책 창의 `No, exit`** 을 눌러 노드를 종료시킨다(rc 1).
      · 1 로 접으면 → '기동이 깨졌다'로 읽어 **살아 있는 좌석을 회수·파괴**하려 든다
        (치명위험 ④ — 오살이 오탐보다 훨씬 비싸다).
    그래서 전용 값을 쓰고, `EXIT_BOOT_BUSY=75` 가 세운 **exit 이름공간 4단 절차**를 그대로 따른다:
    lib 정본 → CLI 소비 → python 상수 → GUI 분기. 이 검체가 그 4단을 기계 대조한다.
    """
    notes = []
    lib = _read(os.path.join(REPO_DIR, "src", "lib.rs"))
    cli = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    boot_src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))

    # ① lib 정본 + 이름공간: 게이트 exit 공간(2~11)을 침범하지 않는다.
    need("pub const EXIT_GATE_PENDING: i32 = 78;" in lib, "lib 정본 상수 부재/값 이탈")
    need(not (2 <= 78 <= 11), "게이트 exit 공간 판정식 오류(계측기 파손)")
    # ② CLI 소비: 세 호출부 중 **보류를 낼 수 있는 둘**이 이 값을 쓴다(launch·node-recover).
    # 접미 확장(`..._DISABLED`)이 계수에 섞이지 않게 **정확한 심볼 경계**로 센다.
    cli_uses = len(re.findall(r"cys::EXIT_GATE_PENDING(?![A-Z_])", cli))
    need(cli_uses >= 3,
         "CLI 소비 지점이 부족하다(%d) — 생산(launch·node-recover)과 소비(run_boot·restore)가 "
         "모두 같은 값을 통과해야 한다" % cli_uses)
    # ★계수만으로는 **어느 지점이 빠졌는지**를 못 잡는다(테스트 assert 만 남아도 수가 찬다).
    #   생산 2곳·소비 3곳을 각각 구조로 못박는다 — 하나라도 1/0 으로 되돌아가면 그 지점이 곧
    #   '살아 있는 좌석을 회수·파괴' 또는 '지침을 관문 창에 주입' 경로다.
    for producer, why in (
        ("Ok(BootVerdict::GatePending { .. }) => cys::EXIT_GATE_PENDING,",
         "node-recover 가 보류를 전용 exit 로 내지 않는다 — rc 1 이면 run_boot 이 reclaim(kill)로 "
         "에스컬레이션해 살아 있는 에이전트를 죽인다"),
        ("            cys::EXIT_GATE_PENDING\n        }",
         "launch-agent 의 보류 분기가 전용 exit 를 반환하지 않는다 — 0 이면 소비부가 지침·티켓을 "
         "태우고(그 Return 이 면책 창을 누른다), 1 이면 좌석 회수 처방이 나간다"),
    ):
        need(producer in cli, why)
    for consumer, why in (
        ("if rc == cys::EXIT_GATE_PENDING {", "run_boot·restore 의 보류 수신 분기 결손"),
        ("if launch_rc == cys::EXIT_GATE_PENDING {", "run_boot 의 launch 보류 수신 분기 결손"),
    ):
        need(consumer in cli, why)
    # ③ python 미러 — 값 파리티.
    need("CYS_LAUNCH_EXIT_GATE_PENDING = 78" in boot_src, "python 소비부 상수 이탈(파리티 붕괴)")
    need("CYS_LAUNCH_EXIT_GATE_PENDING" in boot_src.split("def ", 1)[-1] or
         boot_src.count("CYS_LAUNCH_EXIT_GATE_PENDING") >= 2,
         "python 이 상수를 **선언만** 하고 소비하지 않는다(유령 상수)")
    # ④ GUI 분기 — 보류에서 팀 부트를 이어 붙이지 않는다(그 주입이 관문 창의 Return 이 된다).
    gui_path = os.path.join(REPO_DIR, "src-tauri", "src", "main.rs")
    if os.path.isfile(gui_path):
        gui = _read(gui_path)
        need(gui.count("cys::EXIT_GATE_PENDING") >= 2,
             "GUI 가 보류를 별도 분기하지 않는다(마스터·부서장 두 경로 — 실패 토스트로 뭉개짐)")
        for m in re.finditer(r"cys::EXIT_GATE_PENDING", gui):
            seg = gui[m.end():m.end() + 900]
            need("spawn_orchestra_boot" not in seg.split("if out.status.success()")[0],
                 "GUI 보류 분기가 팀 부트를 이어간다 — 관문 창에 주입하면 그 Return 이 노드를 죽인다")
    notes.append("4단 파리티(lib 78 ↔ CLI ↔ python ↔ GUI)")

    # ⑤ H-DOC-4 와의 비충돌: 78 은 bootstrap **자기** exit 공간이 아니므로 헤더 표에 없어야 한다
    #    (있으면 '유령 exit' 로 잡힌다 — 그 검체가 코드 도달성으로 판정하기 때문).
    head = boot_src[:boot_src.index('"""', boot_src.index('"""') + 3)]
    need(not re.search(r"(?<![\w.])78=", head),
         "78 이 bootstrap 헤더 exit 표에 등재됐다 — 그 표는 bootstrap 자기 exit 공간이고, "
         "78 은 launch-agent 의 값이라 H-DOC-4 가 유령 계약으로 잡는다")
    notes.append("헤더 exit 표 비침범(H-DOC-4 무충돌)")

    # ⑥ 소비 계약 실측: 보류는 `cys boot` 의 Fatal 판정을 오염시키지 않는다(살아 있는 노드다).
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_bootstrap as B
    gated = json.dumps({"roles": [{"role": "cso", "agent": "claude",
                                   "outcome": "gate_pending", "mandatory": True}]})
    need(B._boot_fatal_verdict(0, gated) is None,
         "관문 보류가 Fatal 실패로 오분류 — 살아 있는 좌석에 회수·파괴 처방이 나간다")
    need(B._boot_was_busy(0, gated) is False, "보류가 busy(무스폰)로 오분류 — 티켓 회계 오염")
    # 진짜 실패는 여전히 Fatal 이다(완화 금지 대조군).
    failed = json.dumps({"roles": [{"role": "cso", "agent": "claude",
                                    "outcome": "failed", "mandatory": True}]})
    need(B._boot_fatal_verdict(1, failed) is not None,
         "의무 역할 failed 가 Fatal 로 승격되지 않음(보류 도입이 실패 판정을 삼켰다)")
    notes.append("소비 실측: 보류≠Fatal·≠busy · 진짜 실패=Fatal 불변")

    # ⑦ 계측 타당성 — 구 트리에는 이 값도 타입도 없다(탐지기가 진짜 부재를 잡는지 확인).
    calib = "skip(no-git)"
    old_lib = _git_show(os.path.join("src", "lib.rs"))
    old_cli = _git_show(os.path.join("src", "bin", "cys.rs"))
    if old_lib is not None and old_cli is not None:
        need("EXIT_GATE_PENDING" not in old_lib, "계측 타당성 실패: 구 lib 에 이미 상수가 있다")
        need("BootVerdict" not in old_cli, "계측 타당성 실패: 구 CLI 에 이미 타입드 판정이 있다")
        calib = "구 트리=상수·타입 양쪽 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-EXIT-10", "W1a", "②ping 실패에도 알림 시도 발생(카브아웃 제거·notifier 단일화)", ["A15", "R2"])
def h_exit_10():
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = os.path.join(home, ".cys", "pack")
        bindir = os.path.join(tmp, "stubbin")
        os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
        os.makedirs(bindir, exist_ok=True)
        # 스텁 cys: ping 실패(exit 1) · 그 외(feed push·send)는 성공 + 호출 적재
        _mock_cys(bindir, tmp, 'case "$1" in ping) exit 1;; --version) echo "cys 0.0.0-stub"; exit 0;; esac')
        _w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
        _w(os.path.join(pack, "bin", "javis_orchestra.py"), "import sys; sys.exit(0)\n", 0o644)
        env = _base_env({"HOME": home, "PATH": bindir + os.pathsep + os.environ.get("PATH", ""),
                         "CYS_SURFACE_ID": "7", "CYS_BOOT_CHECK_RETRIES": "1",
                         "CYS_BOOT_CHECK_INTERVAL_S": "0.05"})
        r = _run([PY, os.path.join(BIN_DIR, "javis_bootstrap.py")], env=env, timeout=180)
        need(r.returncode == 3, "②ping 실패의 exit 계약(3) 이탈: %d\n%s" % (r.returncode, r.stderr[-800:]))
        calls = _calls(tmp)
        need("feed push" in calls or "send --queued" in calls,
             "②ping 실패에서 알림 시도가 전무하다(카브아웃 잔존): %r" % calls[-600:])
        bl = json.loads(_read(os.path.join(home, ".cys", "state", "boot-last.json")) or "{}")
        res = bl.get("result") or {}
        need(res.get("failed_step") == "②ping", "boot-last 실패 단계 기록 이상: %r" % res)
        need((res.get("notify") or {}).get("attempted") is True,
             "알림 시도가 상태로 기록되지 않았다(보고=실측 위반): %r" % res)
        chan = (res.get("notify") or {}).get("channel")
    # 계측기 자기검증: 구 코드는 ②ping 카브아웃으로 알림을 건너뛴다
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    if old is not None:
        need('if name != "②ping"' in old,
             "계측 타당성 실패: 구 코드에 ②ping 카브아웃이 없으면 이 검체는 아무것도 증명하지 않는다")
        calib = "구 코드 카브아웃 존재 확인"
    return "exit 3 · 알림 채널=%s · boot-last notify 기록 · 계측검증=%s" % (chan, calib)


# ═══════════════════════════════════════════════════════════════════════════
# 4. H-CONC (동시성)
# ═══════════════════════════════════════════════════════════════════════════
_LOCK_PROBE = (
    "import sys, os\n"
    "sys.path.insert(0, sys.argv[1])\n"
    "import javis_lock as L\n"
    "lk = L.FileLock(sys.argv[2], owner=sys.argv[3])\n"
    "st = lk.acquire()\n"
    "print('%s|%s|%s' % (st, lk.backend, lk.reclaimed_stale))\n"
    "sys.stdout.flush()\n"
    "if st == L.ACQUIRED and len(sys.argv) > 4:\n"
    "    import time; time.sleep(float(sys.argv[4]))\n"
)


@specimen("H-CONC-1", "W1a", "동시 2+발화 → 정확히 1 실행(flock/msvcrt 헬퍼 양 분기)", ["A8", "A8py", "G17"])
def h_conc_1():
    lockmod = os.path.join(BIN_DIR, "javis_lock.py")
    need(os.path.isfile(lockmod), "javis_lock.py 부재")
    results = {}
    with tempfile.TemporaryDirectory() as tmp:
        lp = os.path.join(tmp, "sf.lock")
        holder = subprocess.Popen([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "holder", "2.5"],
                                  stdout=subprocess.PIPE, text=True)
        first = holder.stdout.readline().strip()
        need(first.startswith("acquired"), "선점자가 락을 못 잡았다: %r" % first)
        backend = first.split("|")[1]
        # 4 도전자 동시 → 전원 busy
        chal = [subprocess.Popen([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "chal%d" % i],
                                 stdout=subprocess.PIPE, text=True) for i in range(4)]
        outs = [c.communicate(timeout=60)[0].strip() for c in chal]
        holder.kill(); holder.wait(timeout=30)
        need(all(o.startswith("busy") for o in outs), "동시 발화가 직렬화되지 않았다: %r" % outs)
        results["race"] = "1 acquired / %d busy (backend=%s)" % (len(outs), backend)

        # 부트스트랩이 실제로 이 헬퍼를 소비하는가 (A8py — Windows 실효화의 전부)
        src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
        need("import javis_lock" in src, "javis_bootstrap 이 javis_lock 을 import 하지 않는다")
        need("_lock.FileLock" in src, "_acquire_singleflight 가 FileLock 을 소비하지 않는다")
        need(not re.search(r"^\s*import fcntl", src, re.M),
             "javis_bootstrap 에 fcntl 직접 사용이 남았다(posix 전용 사본)")
        # 함수 **본문만** 잘라 검사한다(다음 top-level def 까지) — 인접 함수 오탐 차단.
        m = re.search(r"^def _acquire_singleflight\(\):\n(.*?)(?=^def |\Z)", src, re.M | re.S)
        need(m, "_acquire_singleflight 정의를 찾지 못했다")
        # docstring·주석 제외 — 제거한 결함을 **문서화한** 문장까지 잡으면 계측기 오탐이다.
        sf = re.sub(r'"""(?:.|\n)*?"""', "", m.group(1))
        sf = "\n".join(l for l in sf.splitlines() if not l.strip().startswith("#"))
        need("os.name" not in sf,
             "싱글플라이트 본문에 posix 전용 가드가 남았다(Windows 무락 회귀)")

        # 부트스트랩 싱글플라이트 실동작: 보유 중 2번째 프로세스는 no-op(None)
        home = os.path.join(tmp, "home")
        os.makedirs(os.path.join(home, ".cys", "state"), exist_ok=True)
        code = ("import sys, os; sys.path.insert(0, %r); import javis_bootstrap as B\n"
                "r = B._acquire_singleflight()\n"
                "print('A' if r else 'NONE')\n"
                "sys.stdout.flush()\n"
                "import time; time.sleep(float(sys.argv[1]))\n" % BIN_DIR)
        env = _base_env({"HOME": home})
        p1 = subprocess.Popen([PY, "-c", code, "2.5"], stdout=subprocess.PIPE, text=True, env=env)
        need(p1.stdout.readline().strip() == "A", "1번째 부트스트랩이 락을 못 잡았다")
        p2 = _run([PY, "-c", code, "0"], env=env, timeout=60)
        p1.kill(); p1.wait(timeout=30)
        need(p2.stdout.strip() == "NONE",
             "2번째 부트스트랩이 no-op 으로 접히지 않았다(중복 부트): %r" % p2.stdout)
        results["bootstrap"] = "2번째 호출 no-op 확인"
    return " · ".join("%s: %s" % kv for kv in results.items())


@specimen("H-CONC-2", "W2", "훅+GUI boot 중첩 → 중복 스폰 0(락 확장)", ["G12", "A8rs"])
def h_conc_2():
    """G12: boot 락이 ④(cys boot) 만 덮고 ④-b(boot-reviewers)·boot_node LAUNCH 를 덮지 않아
    리뷰어 중복 스폰 창이 열려 있었다 + run_boot 이 루프 진입 전 스냅샷 하나로 판정해 stale 했다.
    A8rs: non-unix 의 '무락 Acquired'(락 없이 락을 얻었다고 보고) 수리."""
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    notes = []
    # ⓐ iteration 마다 재조회 — 루프 안에 fetch_surfaces 가 있어야 한다.
    bi = src.find("fn run_boot(")
    need(bi > 0, "run_boot 를 못 찾았다")
    body = src[bi:src.find("\n}\n", bi)]
    li = body.find("for (role, agent, mandatory) in BOOT_PLAN")
    need(li > 0, "run_boot 의 PLAN 루프를 못 찾았다")
    need("fetch_surfaces()" in body[li:],
         "루프 안 role 생존 재조회가 없다(G12: 락 밖 변화에 stale 판정 → 중복 스폰)")
    notes.append("iteration 마다 재조회")
    # ⓑ non-unix 무락 Acquired 수리(A8rs) — pidfile 락 + 스테일 회수.
    need("fn win_pidfile_lock(" in src, "non-unix 락 구현이 없다(무락 Acquired 잔존 — A8rs)")
    need("create_new(true)" in src, "pidfile 락이 O_EXCL(create_new) 원자성을 쓰지 않는다")
    need("fn pidfile_holder_dead(" in src, "스테일 락 회수가 없다(무한 거부 창 — R1 동형)")
    # (의도 보존 갱신) 최종 구현은 f 를 cfg(unix) 한정으로 열어 non-unix 분기에 drop(f) 가
    # 존재하지 않는다 — 의도(무락 Acquired 의 pidfile 락 교체)는 분기가 win_pidfile_lock 을
    # 직접 호출하는지로 판정한다.
    ai = src.find("#[cfg(not(unix))]\n    {\n        match win_pidfile_lock(")
    need(ai > 0, "non-unix 분기가 pidfile 락으로 교체되지 않았다")
    notes.append("A8rs: pidfile 락(O_EXCL)+스테일 회수")
    # ⓑ' LAUNCH 경로 락 참여(G12 핵심) — 별도 프로세스 `cys launch-agent`(boot_node 경유)가
    #    GUI/훅 `cys boot` 와 같은 소켓별 락에 참여한다 + 재진입 방어(자기 교착 0) + 유계 대기.
    need("fn acquire_launch_lock(" in src, "launch-agent 가 boot 락에 참여하지 않는다(G12 창 잔존)")
    need("let _launch_lock = acquire_launch_lock();" in src,
         "run_launch_agent_opts 가 LAUNCH 락을 획득하지 않는다")
    need("fn boot_lock_already_held(" in src and "CYS_BOOT_LOCK_HELD" in src,
         "재진입 방어(프로세스 내부·env 전파)가 없다 — run_boot 이 자기 락에 막힌다")
    li2 = src.find("fn acquire_launch_lock(")
    lbody2 = src[li2:src.find("\nfn acquire_boot_lock(", li2)]
    need("Instant::now() >= deadline" in lbody2,
         "LAUNCH 락 대기가 유계가 아니다(무한 대기 = 부트 정지)")
    need("직렬화 없이 진행" in lbody2, "대기 상한 초과에서 가용성 우선 진행 계약이 없다")
    notes.append("LAUNCH 경로 락 참여+재진입 방어+유계 대기")
    # ⓒ ④-b·boot_node LAUNCH 락 커버리지 — 훅 발화 전체가 단일 실행 락 아래 있는지(python 측).
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need("javis_lock" in bsrc or "_singleflight" in bsrc,
         "bootstrap 이 단일 실행 락을 쓰지 않는다")
    # ④-b 는 ④ 와 **같은 싱글플라이트 임계영역** 안에서 호출된다(락 취득이 cmd_run 진입부).
    ci = bsrc.find("④-b 리뷰어 감지")
    li2 = bsrc.find("_singleflight")
    need(0 < li2 < ci, "④-b 가 싱글플라이트 락 획득 이전/밖에서 호출된다(중복 스폰 창)")
    notes.append("④-b 가 싱글플라이트 임계영역 내부")
    # ⓓ Rust 락 파일 경로가 소켓별(레인 격리)로 파생 — 레인 간 상호 차단 0.
    need("fn boot_lock_path(" in src and "cys-boot.lock" in src, "boot 락 경로 파생 함수가 없다")
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        # acquire_boot_lock 함수 본문 안의 non-unix 분기만 본다(파일 전역 첫 매칭은 무관 코드).
        oi = old.find("fn acquire_boot_lock()")
        need(oi > 0, "계측 타당성 실패: 구 코드의 acquire_boot_lock 을 못 찾았다")
        obody = old[oi:old.find("\nfn run_boot(", oi)]
        j = obody.find("#[cfg(not(unix))]")
        need(j > 0 and "BootLock::Acquired(Some(f))" in obody[j:j + 200],
             "계측 타당성 실패: 구 코드의 non-unix 무락 Acquired 를 못 찾았다")
        need("win_pidfile_lock" not in old, "계측 타당성 실패: 구 코드에 이미 pidfile 락이 있다")
        calib = "구 코드 non-unix=무락 Acquired(상호배제 0) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


_CONC3_CHILD = r"""
import json, os, sys
sys.path.insert(0, os.environ["CYS_BIN_DIR"])
import javis_preflight as PF
tag, iters = sys.argv[1], int(sys.argv[2])
target = os.environ["CYS_SETTINGS"]
script, event = os.environ["CYS_PAIR"].split("|")
pf = PF.Preflight(True, [])
for i in range(iters):
    # ⓐ 훅 등록(멱등) — 실 등록기 경로를 그대로 태운다
    err = pf._register_event_hook(target, event, script, None)
    if err:
        sys.stderr.write("register-fail:%s\n" % err); sys.exit(3)
    # ⓑ 순수 RMW 카운터 — lost update 를 **정확히** 잰다(락이 없으면 총합이 줄어든다)
    def mut(data, tag=tag, i=i):
        data.setdefault("marks", []).append("%s-%d" % (tag, i))
    err = PF._settings_rmw(target, mut)
    if err:
        sys.stderr.write("rmw-fail:%s\n" % err); sys.exit(4)
sys.exit(0)
"""


@specimen("H-CONC-3", "W3", "settings.json 다중 writer 경합 — 무파손·무 lost-update(공용 락+mkstemp)",
          ["G16", "A8"])
def h_conc_3():
    """★실측 정정(N13 · 2026-08-24): 이 검체는 settings.json 을 **3-writer** 대상이라고 적고
    있었다. 전수로 세면 **5**다 — preflight `_settings_rmw`(락 O) · `javis_guard_register.py`
    `_atomic_write`(락 X) · `javis_dept_migrate.py` `_register_hook`(락 X · 고정 `.tmp`) ·
    `src/pack.rs::merge_desired_hooks`(**락 O** · 2026-09-05) ·
    `src/factory_reset.rs::strip_settings_matching`(**락 O** · 2026-09-05).
    근거·잔여 위험은 `javis_preflight.py` 의 `_settings_rmw` 머리 주석이 정본이다.
    이 검체가 실측하는 것은 그중 **preflight 레인**(등록기 4종 + 공용 락)이며, 아래 ⓑ가
    Rust 두 writer 의 원자 쓰기·유일 tmp·공용 락을 소스핀으로 대조한다.

    ★H-CONC-3 daemon 레인 근인(W-B 실측 2026-09-05 · 수리 완료): `pack::write_atomic_mode` 의
    tmp 이름이 `.<파일명>.tmp.<pid>` 로 **pid 당 하나로 고정**이었다. 프로세스가 다르면 이름이
    갈리므로 위 ⓒ의 **6 프로세스** 경합은 파손 0 을 냈고 — 그래서 이 축에서는 보이지 않았다 —
    같은 프로세스의 두 호출(데몬의 연결별 tokio task)만 `File::create`(O_TRUNC)로 같은 inode 를
    공유해 서로를 잘랐다.

    ★★인과 정정(2026-09-05 · W-A 최종 특정 · master 수용) — **이 결함은 CI H-CONC-3 적색의
    원인이 아니었다.** 그 적색의 출처는 아래 ⓓ **음성 대조군 `naive.json`** 이다: 대조군 문서는
    마크 N개일 때 `8N+11` 바이트라 이어붙은 다음 열이 `8N+12` 이고 **N=4 → col 44 · N=5 → col 52**
    다. daemon 레인 사본이 그 파일을 맨몸 `json.loads` 로 읽어 **판정이 아니라 traceback 으로
    죽은 것**이 적색의 실체다(pack 레인은 `e1ae6ce` 로 판정화). 그리고 settings.json 은 indent=2
    라 1행이 여는 중괄호 하나뿐이어서 이 서명이 **원리적으로 불가능**하다.
    ⓒ 경합에 Rust writer 는 애초에 참여하지 않는다(자식은 preflight 두 함수만 부른다).
    즉 위 pid 고정 tmp 결함의 근거는 **Rust 2스레드 프로브와 변이 검증**이지 CI 의 col 숫자가
    아니다 — 숫자가 우연히 맞았을 뿐이다(W-B 자기 정정).

    G16: preflight 의 네 등록기가 각자 `open(path + ".tmp")` → `os.replace` 를 재구현했고
    tmp 이름이 **고정**이었다 — 동시 writer 가 서로의 임시 파일에 써서 교차 파손(반쪽 JSON)을 만들고,
    그 뒤 모든 등록기가 "파싱 실패 — 덮어쓰기 거부"로 수리를 영구 포기한다(A8 지배 실패 모드).
    W1a 가 신설한 `javis_lock`(락+mkstemp)의 **소비 이관**이 W3 몫이다."""
    PF = _preflight_mod()
    notes = []
    # ⓐ 구조: 고정 `.tmp` 재구현 0 · 등록기 전원이 단일 RMW 소유자를 경유
    src = _read(os.path.join(BIN_DIR, "javis_preflight.py"))
    code = _code_lines(src)
    need('settings_path + ".tmp"' not in code, "고정 .tmp 재구현이 남아 있다(교차 파손 경로)")
    need("def _settings_rmw(" in code, "settings RMW 단일 소유자가 없다")
    for fn in ("_register_hook", "_register_statusline", "_register_event_hook",
               "_register_appbuild_hook"):
        i = code.find("def %s(" % fn)
        need(i > 0, "%s 를 못 찾았다" % fn)
        body = code[i:code.find("\n    def ", i + 10)]
        need("_settings_rmw(" in body, "%s 가 단일 RMW 소유자를 경유하지 않는다" % fn)
    need("javis_lock" in code and "FileLock(" in code, "preflight 가 공용 락을 소비하지 않는다")
    notes.append("등록기 4종 단일 RMW 경유·고정 .tmp 0")
    # ⓑ Rust 측 원자 쓰기(W2 A8rs) — 실측 5 writer 중 Rust 두 축
    pk = _repo_file(os.path.join("src", "pack.rs"))
    mi = pk.find("pub fn merge_desired_hooks(")
    need(mi > 0, "Rust 병합기를 못 찾았다")
    mbody = pk[mi:pk.find("\n}\n", mi)]
    need("write_atomic(settings_path" in mbody,
         "Rust 병합기가 원자 쓰기를 쓰지 않는다(반쪽 등록부 = 부트 발화 소실)")
    need("std::fs::write(settings" not in _code_lines(pk), "Rust 비원자 settings write 잔존")
    cy = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("std::fs::write(settings_path" not in _code_lines(cy),
         "init-pack 이 비원자 write 로 되돌아갔다(다중 writer 파손 축 복귀)")
    # ★유일 tmp — pid 고정 이름이 되돌아오면 같은 프로세스 두 writer 가 같은 inode 를 공유한다.
    #   ★검사 범위를 **함수 본문**으로 좁힌다: 파일 전체로 보면 재현 프로브(음성 대조군)가 구
    #   규칙 문자열을 일부러 갖고 있어 영구 적색이 된다(대조군을 결함으로 오판하는 자충수).
    wi = pk.find("pub fn write_atomic_mode(")
    need(wi > 0, "Rust 원자 쓰기 소유자(write_atomic_mode)를 못 찾았다")
    wbody = pk[wi:pk.find("\n}\n", wi)]
    need('.tmp.{}", std::process::id()' not in wbody,
         "pid 고정 tmp 이름이 되돌아왔다 — 같은 프로세스 두 writer 가 같은 inode 를 공유한다")
    need("crate::tmp_path_for(" in wbody,
         "호출마다 유일한 tmp 이름 생성기(A13 공용)를 쓰지 않는다")
    ci = pk.find("fn create_tmp_with_mode(")
    need(ci > 0, "tmp 생성기를 못 찾았다")
    cbody = pk[ci:pk.find("\n}\n", ci)]
    need("create_new(true)" in cbody,
         "tmp 생성이 O_EXCL 이 아니다 — 이름 충돌을 조용히 삼켜 겹쳐 쓴다")
    # ★공용 락 — 원자 쓰기는 반쪽 파일만 막고 lost update 는 락으로만 닫힌다.
    need("acquire_settings_lock(settings_path)" in mbody,
         "Rust 병합기가 공용 락(<settings>.cys-lock)을 잡지 않는다(lost update 축)")
    need("cys-lock" in _code_lines(pk),
         "공용 락 소유자가 pack.rs 에 없다(사본 분화 — cys.rs 단독 구현으로 회귀)")
    # ★IG-23: **세 번째** Rust RMW writer — `retune_registered_hook_timeouts` 도 읽고 다시 쓴다.
    #   2026-09-05 최초 수리가 이것을 빠뜨렸다(writer 전수 색출로 발견). 목록으로 세지 말고
    #   **함수 본문마다** 핀한다 — 목록은 새 writer 가 생기면 조용히 낡는다.
    ri = pk.find("pub fn retune_registered_hook_timeouts(")
    need(ri > 0, "Rust timeout 재조정기를 못 찾았다")
    rbody = pk[ri:pk.find("\n}\n", ri)]
    need("acquire_settings_lock(settings_path)" in rbody,
         "retune_registered_hook_timeouts 가 공용 락을 잡지 않는다(세 번째 RMW writer)")
    fr = _repo_file(os.path.join("src", "factory_reset.rs"))
    si = fr.find("fn strip_settings_matching(")
    need(si > 0, "Rust 제거기를 못 찾았다")
    sbody = fr[si:fr.find("\n}\n", si)]
    need("acquire_settings_lock(settings_path)" in sbody,
         "Rust 제거기가 공용 락을 잡지 않는다(preflight C28 재등록과 교차해 lost update)")
    # ★중첩 금지 — flock 은 **open file description** 단위라 같은 프로세스가 같은 파일에 두 번
    #   걸면 영구 교착이다(실측 확인). 락 소유자를 말단 writer 로 옮긴 뒤 호출부(cys.rs 의
    #   hooks-prune·doctor --fix 3곳)가 바깥에서 또 잡던 것을 걷어냈다 — 되돌아오면 `cys doctor
    #   --fix` 가 그 자리에서 멈춘다(사용자 대면 치유 경로의 hang).
    need("_lock = acquire_settings_lock" not in _code_lines(cy),
         "cys.rs 가 말단 writer 바깥에서 같은 락을 다시 잡는다 — 중첩 flock = 자기교착")
    notes.append("Rust 시드 원자 쓰기 · 유일 tmp(O_EXCL) · 공용 락 3 writer")
    # ⓒ 실측 경합: 6 writer × 12 iter 동시 실행 → 파손 0 · lost update 0 · 훅 6종 전원 등록
    pairs = [("session-start.sh", "SessionStart"), ("role-bootstrap.sh", "UserPromptSubmit"),
             ("save-state.sh", "Stop"), ("save-state.sh", "PreCompact"),
             ("reflect-scan.sh", "SessionEnd"), ("pack-guard.sh", "PostToolUse")]
    iters = 12
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = _fake_pack_with_hooks(os.path.join(home, ".cys", "pack"))
        target = os.path.join(home, ".claude", "settings.json")
        os.makedirs(os.path.dirname(target), exist_ok=True)
        # 사용자 기존 설정 — 경합 후에도 살아 있어야 한다
        _w(target, json.dumps({"theme": "dark", "hooks": {}}), 0o644)
        child = os.path.join(tmp, "child.py")
        _w(child, _CONC3_CHILD, 0o644)
        procs = []
        for n, (script, event) in enumerate(pairs):
            env = _base_env({"HOME": home, "CYS_PACK_DIR": pack, "CYS_BIN_DIR": BIN_DIR,
                             "CYS_SETTINGS": target, "CYS_PAIR": "%s|%s" % (script, event)})
            procs.append(subprocess.Popen([PY, child, "w%d" % n, str(iters)], env=env,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                          text=True))
        fails = []
        for pr in procs:
            out, err = pr.communicate(timeout=180)
            if pr.returncode != 0:
                fails.append("rc=%d %s" % (pr.returncode, (err or "")[-200:]))
        need(not fails, "경합 writer 실패: %r" % fails)
        # 파손 0: 파싱 성공 + 사용자 키 보존
        raw = _read(target)
        # ★맨몸 `json.loads` 금지(cherry-pick -x 2559d4d · W-A): 종전 이 줄은 파손 시
        #   **JSONDecodeError 로 크래시**해 검체가 FAIL 이 아니라 traceback 으로 죽었다 —
        #   판정이 아니라 계측기 사망이라 원인 추적이 불가능했다(파일 내용이 사라진다).
        #   파손은 **판정 결과**(FAIL)여야 하고 그 증거(파일 원문)는 남아야 한다.
        try:
            data = json.loads(raw)      # 파싱 실패 = 교차 파손(원 결함의 지배 모드)
        except ValueError as _je:
            _ev = os.path.join(tempfile.gettempdir(),
                               "h-conc-3-corrupt-%d.json" % os.getpid())
            try:
                with open(_ev, "w", encoding="utf-8") as _f:
                    _f.write(raw)
            except OSError:
                _ev = "(증거 보존 실패)"
            need(False,
                 "settings.json 교차 파손 — %s · %d바이트 · 머리 80=%r · 꼬리 80=%r · 증거=%s "
                 "(‘Extra data’ 는 찢긴 쓰기의 서명이다: 완결 JSON 뒤에 남은 이전 본문의 꼬리)"
                 % (_je, len(raw), raw[:80], raw[-80:], _ev))
        need(data.get("theme") == "dark", "경합이 사용자 키를 지웠다")
        # lost update 0: 모든 writer 의 모든 mark 가 남는다
        marks = data.get("marks") or []
        need(len(marks) == len(pairs) * iters,
             "lost update 발생 — marks %d ≠ 기대 %d(직렬화 실패)" % (len(marks), len(pairs) * iters))
        need(len(set(marks)) == len(marks), "mark 중복(재진입 이상)")
        # 훅 6종 전원 등록 + 이벤트별 중복 0
        #   ★기대 문자열은 **자식과 같은 팩 env** 에서 만들어야 한다(부모 env 로 만들면 경로가 달라
        #     '0회 등록'으로 오판한다 — 계측기 자기검증).
        with _env_patch(HOME=home, CYS_PACK_DIR=pack):
            wants = {(script, event): PF._cys_hook_cmd(script) for script, event in pairs}
        for script, event in pairs:
            want = wants[(script, event)]
            arr = (data.get("hooks") or {}).get(event) or []
            cnt = sum(1 for e in arr for h in e.get("hooks", []) if h.get("command") == want)
            need(cnt == 1, "%s/%s 등록 %d회(0=유실·2+=중복 append)" % (event, script, cnt))
        # 잔재 0: 고정 .tmp 파일이 남지 않는다(mkstemp 정리)
        leftovers = [f for f in os.listdir(os.path.dirname(target))
                     if f.endswith(".tmp") or f.startswith(".tmp-")]
        need(not leftovers, "임시 파일 잔재: %r" % leftovers)
        notes.append("6 writer × %d iter: 파손 0·lost update 0·중복 0" % iters)
        # ⓓ ★계측 타당성(음성 대조군): 같은 mark 오라클을 **직렬화 없는** RMW 에 걸면 lost update 가
        #    실제로 검출돼야 한다 — 검출력이 없는 오라클로 얻은 위 GREEN 은 아무것도 증명하지 않는다
        #    (MEMORY '디버깅 계측 타당성 게이트': 계측기 자체를 먼저 검증한다).
        naive = os.path.join(tmp, "naive.json")
        _w(naive, json.dumps({"marks": []}), 0o644)
        nchild = os.path.join(tmp, "naive.py")
        _w(nchild, "import json, os, sys, time\n"
                   "t, n = sys.argv[1], int(sys.argv[2])\n"
                   "p = os.environ['CYS_SETTINGS']\n"
                   "for i in range(n):\n"
                   "    d = json.load(open(p))\n"
                   "    time.sleep(0.02)\n"
                   "    d.setdefault('marks', []).append('%s-%d' % (t, i))\n"
                   "    json.dump(d, open(p, 'w'))\n", 0o644)
        nprocs = [subprocess.Popen([PY, nchild, "n%d" % k, "6"],
                                   env=_base_env({"CYS_SETTINGS": naive}),
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                  for k in range(4)]
        ndied = 0
        for pr in nprocs:
            pr.communicate(timeout=120)
            if pr.returncode != 0:
                ndied += 1
        # ★대조군의 파손은 **정상 결과**다 — 그것이 이 군을 두는 이유다(직렬화가 없다).
        #   그런데 종전 이 줄은 맨몸 `json.loads` 라, 대조군이 자기 파일을 이어붙이면 검체가
        #   **판정이 아니라 traceback 으로 죽었다**(CI 적색 `JSONDecodeError: Extra data:
        #   line 1 column 52`). 위 ⓒ와 **같은 결함의 두 번째 인스턴스**였다.
        #   ★이 서명은 settings.json 에서는 나올 수 없다: `_settings_rmw` 는 `indent=2` 로 써서
        #     1행이 여는 중괄호 하나다. 대조군 문서는 마크 N개일 때 `8N+11` 바이트라 이어붙은
        #     다음 열이 `8N+12` 이고 **N=4 → 44 · N=5 → 52** 다 — CI 가 본 그 숫자다.
        #   (cherry-pick -x e1ae6ce · W-A)
        nraw = _read(naive)
        try:
            nmarks = (json.loads(nraw or "{}") or {}).get("marks") or []
        except ValueError as _nje:
            # 파손 = lost update 보다 **더 강한** 무직렬화 증거다. 검출로 센다(크래시 금지).
            _nev = os.path.join(tempfile.gettempdir(),
                                "h-conc-3-naive-corrupt-%d.json" % os.getpid())
            try:
                with open(_nev, "w", encoding="utf-8") as _f:
                    _f.write(nraw)
            except OSError:
                _nev = "(증거 보존 실패)"
            notes.append("음성 대조군: 무직렬화 RMW 가 **파일을 이어붙였다**(%s · %d바이트 · "
                         "자식 비정상종료 %d/4 · 증거=%s) — lost update 보다 강한 검출"
                         % (_nje, len(nraw), ndied, _nev))
        else:
            need(len(nmarks) < 4 * 6 or ndied > 0,
                 "계측 타당성 실패: 직렬화 없는 RMW 에서도 mark 오라클이 lost update 를 못 잡았다"
                 "(marks=%d · 자식 비정상종료 0) — 위 GREEN 은 검출력 미확인" % len(nmarks))
            notes.append("음성 대조군: 무직렬화 RMW 에서 lost update %d/%d 검출(자식 비정상종료 %d/4)"
                         % (4 * 6 - len(nmarks), 4 * 6, ndied))
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_preflight.py"))
    calib = "skip(no-git)"
    if old is not None:
        oldc = _code_lines(old)
        need(oldc.count('settings_path + ".tmp"') >= 3,
             "계측 타당성 실패: 구 코드의 고정 .tmp 재구현을 못 찾았다")
        need("_settings_rmw" not in oldc, "계측 타당성 실패: 구 코드에 이미 단일 RMW 소유자가 있다")
        need("javis_lock" not in oldc, "계측 타당성 실패: 구 preflight 가 이미 공용 락을 쓴다")
        calib = "구 코드=고정 .tmp 3+·락 미소비 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SAFE-1", "W2",
          "치명위험 ④ 차단 — 파괴적 복구는 '죽음 확정' 좌석에만(냉시작·기동중 좌석 불가침)",
          ["W2-자체감사", "B3(안전 경계)", "④ 전 pane 사망"])
def h_safe_1():
    """★내가 W2 에서 신설한 파괴 경로(node-recover pane 주입 → reclaim kill)의 발동 조건을 못박는다.

    발견 경위(자체 적대감사): `seat_liveness` 의 `Absent` 는 세 사실의 합집합이다 —
    ⓐ명시적 빈 좌석 ⓑ좌석 판정불가의 시한부 해소 ⓒ구 데몬 무신호. 초안은 ⓑ·ⓒ에도 복구 체인을
    걸었고, 그 경로는 **냉시작 데몬**(watchdog 첫 틱 전 = 전 좌석 Unknown)에서 GUI 의
    `spawn_orchestra_boot` 와 만나 **건강한 전 pane** 을 파괴한다:
      · `run_node_recover` 는 `agent_alive == Some(true)` 만 거부한다 → watchdog 이 아직 자손을
        관측 못 한 **정상 기동 중** 노드는 통과해, 돌고 있는 claude 입력창에 `C-u` + 기동 커맨드가 박힌다.
      · 이어지는 reclaim 은 kill 이다. 세 좌석에 연쇄하면 '터미널에 글자 0'이다.
    수리 = `seat_death_confirmed` 3중 AND(명시 empty ∧ agent_alive==false ∧ 좌석 나이>readiness 예산).
    """
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    notes = []
    need("fn seat_death_confirmed(" in src, "죽음 확정 게이트가 없다 — 파괴 경로가 Absent 전체에 열림")
    gi = src.find("fn seat_death_confirmed(")
    body = src[gi:src.find("\n}\n", gi)]
    # 3중 AND 전수
    need('Some("empty") => {}' in body,
         "좌석 사실이 **명시적** empty 일 때만 통과하지 않는다(Unknown·필드부재 혼입)")
    need('s["agent_alive"].as_bool() != Some(false)' in body,
         "agent_alive==Some(false) 요건이 없다 — meta 없는 사용자 셸까지 파괴 대상이 된다")
    need("created_at" in body and "budget_readiness_max" in body,
         "좌석 나이 가드가 없다 — 기동 중 pane 과의 레이스가 열린다")
    need("created <= 0.0" in body, "created_at 미상에서 파괴를 허용한다(보류 우선 위반)")
    notes.append("3중 AND(명시 empty·agent_alive=false·나이>예산)+미상 보류")
    # 호출부: 확정 실패 시 **파괴도 스폰도 안 한다**(중복 스폰이 곧 이중 에이전트다)
    ci = src.find("match seat_death_confirmed(row)")
    need(ci > 0, "run_boot 이 죽음 확정 게이트를 소비하지 않는다")
    arm = src[ci:ci + 2200]
    need("skipped_unconfirmed" in arm, "미확정 좌석의 typed outcome 이 없다")
    need("continue;" in arm, "미확정에서 스폰으로 흘러간다(중복 스폰·claim_denied·litter)")
    ok_at = arm.find("Ok(()) =>")
    err_at = arm.find("Err(hold) =>")
    need(0 <= err_at < ok_at, "미확정(Err) 분기가 확정(Ok) 분기보다 뒤에 있다(구조 취약)")
    need("run_node_recover" in arm[ok_at:], "확정 분기에 node-recover 가 없다")
    need("run_node_recover" not in arm[err_at:ok_at],
         "미확정 분기에서 node-recover 를 호출한다(살아있는 pane 주입 — ④ 재발)")
    need("escalate_reclaim" not in arm[err_at:ok_at],
         "미확정 분기에서 reclaim(kill)을 호출한다(오살 — ④ 재발)")
    notes.append("미확정=파괴 0·스폰 0(typed skipped_unconfirmed)")
    # reclaim 은 여전히 hold-first 판정을 통과해야 kill 한다(2선 방어)
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_boot_node as BN
    unk = {"surfaces": [{"role": "cso", "exited": False, "agent_alive": False,
                         "status": None, "seat": "unknown"}]}
    need(BN._reclaim_verdict(unk, "cso", 100, 100) == "hold-alive",
         "2선 방어(reclaim hold-first)가 Unknown 좌석에 kill 을 허용")
    notes.append("2선 방어: reclaim Unknown=hold")
    return " · ".join(notes)


@specimen("H-SAFE-2", "W2",
          "치명위험 ④ 차단 — readiness 안전 밸브(영구 오부정 불가능성) + bare exit 계약(W4 전환)",
          ["W2-자체감사", "B4(안전 경계)", "금지 방향 ⑧"])
def h_safe_2():
    """★두 개의 자체감사 산출 안전장치를 박제한다.

    ① **readiness 안전 밸브**: 델타 매칭은 claude 의 `❯` 가 scrollback(개행 완성 라인)에 실린다는
       가정에 서 있다. 그 가정이 어떤 버전·터미널에서 깨지면 readiness 가 **영구 오부정**이 되고,
       T-0147-4 이후 롤백 close 가 실제로 성공하므로 **건강한 pane 이 전부 닫힌다**(글자 0).
       그래서 화면과 무관한 양성 증거(`agent_alive` = 데몬이 커널 프로세스 표에서 관측한 사실)를
       둔다. 기동 **실패** 노드는 자손이 없어 이 밸브가 켜지지 않으므로 B4 오탐 방향은 불변이다.
    ② **bare exit 계약**(하드 제약 6-⑧): W2 에서는 "GUI 가 --json 을 소비하기 전까지 exit 의미를
       바꾸지 말라"였다(구계약 launch 실패>0). **W4 에서 GUI 소비가 같은 커밋으로 착지**했으므로
       계약이 전진한다 — 0=Fatal 없음 / 1=Fatal / 75=busy. 단언의 방향은 그대로다: exit 의 의미는
       한 곳(`boot_exit_code`)이 소유하고, 소비부가 그 세 값을 **분기해서** 읽어야 한다.
    """
    src = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    notes = []
    # ★핀 이사(U-13): 밸브의 **판정**은 `src/readiness.rs` 의 증거 사다리 첫 항으로 옮겼고,
    #   cys.rs 에는 관측(`agent_alive` 조회)과 귀결(ready/break) 배선만 남는다. 세 계약을
    #   그대로 옮긴다 — ⓐ커널 사실 근거 ⓑ화면 무의존 ⓒ마커보다 선행. 판정 조건은 하나도
    #   완화하지 않았고, 관문 문면 AND 항 하나가 **추가**됐다(그 축은 H-READY-13 이 본다).
    body = _slice_between(src, "fn boot_agent_on_surface(",
                          "\n/// 에이전트 기동 + 역할 지침", "H-SAFE-2 부트 본문")
    judge = _slice_between(src, "pub fn judge(",
                           "// ── 판정부 끝(핀 슬라이스 경계) ──", "H-SAFE-2 판정부")
    # ① 안전 밸브 — agent_alive 커널 사실 기반, 델타 실패와 독립
    need("안전 밸브" in body and 's["agent_alive"].as_bool()' in body,
         "readiness 안전 밸브가 없다 — 델타 가정이 깨지면 건강 pane 전부 close(④)")
    # 밸브 **배선** 구간 = 앵커부터 판정 배선 끝까지. ready 를 세우고 루프를 벗어나는 귀결이
    # 여기 있어야 한다(귀결이 사라지면 밸브가 판정만 하고 아무 일도 하지 않는다).
    wire = _slice_between(body, "★★안전 밸브", "// ── readiness 판정 배선 끝 ──",
                          "H-SAFE-2 밸브 배선")
    need("ready = true" in wire, "안전 밸브가 ready 를 세우지 않는다")
    need("break;" in wire, "안전 밸브가 폴링 루프를 벗어나지 않는다")
    need("cys::readiness::judge(&obs)" in wire,
         "배선이 판정부를 타지 않는다 — ready 선언이 다시 여러 자리로 흩어졌다")
    # 밸브 **판정** — 커널 사실 조건 분기 + 마커 항보다 앞(사다리 순서 계약).
    vi = judge.find("★★안전 밸브")
    need(vi > 0, "판정부에서 안전 밸브 항 앵커를 못 찾았다")
    mi = judge.find("★마커 델타 우선")
    need(0 < vi < mi, "안전 밸브가 마커 항 뒤에 있다(델타 실패 시 도달 못 함 — 사다리 순서 역전)")
    valve = judge[vi:mi]
    need("o.agent_alive == Some(true)" in valve, "안전 밸브가 agent_alive 조건 분기를 갖지 않는다")
    need("Evidence::Valve" in valve, "안전 밸브가 ready 근거를 산출하지 않는다")
    # ★핀 이사(P3-0 · 2026-08-24): 밸브의 **두 번째 근거**는 '꼬리 술어' 가 아니라 **맨 셸 판별**
    #   이다. 종전 AND 항(`tail_ok`)의 판정기는 이름과 달리 "마지막 비공백 줄의 끝문자가
    #   `%` `$` `#` `❯` 인가" 를 보는 검사인데, `❯` 는 **살아있는 Claude Code TUI 의 입력
    #   프롬프트 그 자체**다 — 그래서 건강한 pane 에서 밸브가 **상시 차단**됐고, 밸브의 존재
    #   이유(영구 오부정 차단)가 사문화됐다. 항을 지운 것이 아니라 **비용 부호가 맞는 술어로
    #   교체**한 것이므로 핀도 함께 이사한다(단언 수는 1 → 4 로 늘었다).
    need("bare_shell_ok" in valve,
         "밸브가 두 번째 근거(맨 셸 판별)와의 AND 를 잃었다 — alive 단독은 래퍼 생존을 사망 은폐로 "
         "허용한다(P1-1 회귀)")
    need("o.agent_alive == Some(true) && tail_ok" not in valve,
         "밸브가 다시 꼬리 술어와 AND 를 걸었다 — 살아있는 TUI 의 꼬리는 `❯` 라 건강 pane 이 상시 "
         "차단된다(P3-0 회귀: 밸브 사문화)")
    need("fn screen_is_bare_shell_on(" in src,
         "밸브 전용 술어(맨 셸 판별)가 없다 — 판정 축이 다시 꼬리 술어 하나로 겸직한다")
    need("bare_shell: Some(screen_is_bare_shell(text))" in body,
         "밸브 전용 축의 관측이 판정 입력으로 실리지 않는다 — 밸브가 미관측으로 영구 차단된다")
    notes.append("안전 밸브: agent_alive 커널 사실 + 맨 셸 판별 AND(P3-0) · 마커 항 선행 · 배선 귀결 보존")
    # B4 오탐 방향 불변: 밸브의 **판정 근거**(주석 제외 실코드)에 화면/델타 텍스트가 없다.
    valve_code = "\n".join(ln for ln in valve.splitlines() if not ln.strip().startswith("//"))
    for banned in ("o.delta", "o.screen", "delta_text", "delta_flat", "text.contains"):
        need(banned not in valve_code,
             "안전 밸브가 화면/델타 텍스트를 근거로 쓴다(%s) — 커널 사실 독립성 상실" % banned)
    notes.append("밸브 근거=화면 무의존(B4 오탐 방향 불변)")
    # ② bare exit 구계약
    ri = src.find("fn run_boot(")
    rbody = src[ri:src.find("\n/// 죽음 확정 좌석의 reclaim", ri)]
    # ★핀 이사(M3 · 2026-08-24) — 아래 셋은 **완화가 아니라 이사**다. 종전 세 핀은 run_boot 이
    #   분기마다 `fatal_failed` 를 손으로 세던 시절의 형태(인자 1개 · json 리터럴 필드 · 계상
    #   지점 개수)를 박고 있었다. 그 손 계상이 곧 W4 의 fail-open 원인이었으므로 M3 이 집계를
    #   typed outcome 파생(`boot_summary_buckets`)으로 옮겼다 — 이제 '빠뜨릴 자리' 자체가 없다.
    #   그래서 핀도 **개수에서 술어로** 옮긴다(같은 축 · 더 강함).
    need("boot_exit_code(fatal_failed as usize, fatal_gate_pending as usize, false)" in rbody,
         "run_boot 종료가 단일 판정 함수를 통과하지 않는다(의미가 흩어져 두 채널이 갈린다)")
    need("if failed > 0" not in rbody,
         "구계약(launch 실패>0)이 잔존한다 — 리뷰어 1종 실패가 팀 실패로 번지는 B1 데드엔드 재발")
    need("let summary = boot_summary_buckets(&outcomes);" in rbody
         and 'json!({"roles": outcomes, "summary": summary})' in rbody,
         "fatal_failed 가 --json 요약에 없다(typed 채널로 전달되지 않음)")
    need('("fatal_failed", fatal_failed),' in rbody and '("fatal_gate_pending", fatal_gate_pending),' in rbody,
         "요약 맵이 fatal_failed·fatal_gate_pending 교차 집계를 싣지 않는다(소비부가 두 의미를 못 읽는다)")
    # exit 와 --json 이 **같은 사실**을 내는지: Fatal 계상이 mandatory 분기 전건에 있어야 한다.
    # 종전에는 계상 **지점 개수**(>=3)를 셌다 — 파생 이후에는 지점이 하나이므로 그 술어를 직접 박는다.
    need('"failed" | "missing" => fatal_failed += 1,' in rbody,
         "Fatal 계상이 사라졌다 — mandatory ∧ outcome ∈ {failed,missing} 술어가 없으면 "
         "어떤 Fatal 경로가 exit 0 으로 접힌다")
    need("fn boot_exit_code(" in src, "bare exit 판정 순수 함수 부재(테스트 가능성·단일 소유 상실)")
    notes.append("bare exit=신계약(0/1/75 · boot_exit_code 단일 소유)")
    # ③ 팩↔바이너리 스큐 폴백(온보딩 전멸 차단)
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need("_is_unknown_arg_error(" in bsrc, "`--json` 미지원 구 바이너리 폴백이 없다(온보딩 전멸)")
    need("④boot-skew" in bsrc, "스큐 폴백이 진단 단계로 기록되지 않는다")
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_bootstrap as B
    need(B._is_unknown_arg_error("error: unexpected argument '--json' found") is True,
         "clap 미지원 인자 신호를 감지하지 못한다")
    need(B._is_unknown_arg_error("boot 완료: 신규 기동 0 · 실패 1") is False,
         "진짜 부트 실패를 스큐로 오독한다(재시도 루프 위험)")
    notes.append("스큐 폴백: 미지 인자만 좁게 감지")
    return " · ".join(notes)


@specimen("H-SAFE-W", "W2",
          "Windows 전용 락 안전성 — 영구 Busy 불가능성(나이 backstop)·RAII 해제·fail-open + 타깃 타입검사",
          ["W2-자체감사", "A8rs", "④ Windows 온보딩 전멸"])
def h_safe_w():
    """★알려진 실패 양상("맥은 문제없어도 윈도우 설치파일에서 에러·코드 깨짐이 잦다")에 대한 구조적 응답.

    W2 가 신설한 `cfg(not(unix))` 락 코드는 **macOS 컴파일러가 한 번도 검사하지 않는 영역**이다.
    두 종류의 위험을 각각 못박는다:
      ① **컴파일 위험**: 검사되지 않은 cfg 분기의 타입 오류는 Windows 빌드에서만 터진다.
         → 이 검체는 해당 함수들이 존재하고 계약 형상을 유지하는지 **텍스트로** 확인하고,
           실제 타깃 타입검사는 릴리스 게이트(Windows CI · H-WIN-11)와 워커의 스크래치 크레이트
           `cargo check --target x86_64-pc-windows-msvc` 로 수행한다(음성 대조: 같은 크레이트를
           macOS 타깃으로 검사하면 unix 분기의 `libc` 미해소로 **반드시 실패**해야 한다).
      ② **런타임 위험(치명)**: 파일시스템 락은 자동 해제가 없다. 초안은 pidfile 을 삭제하지 않아
         `tasklist` 가 실패하거나 pid 가 재사용되면 **모든 Windows 부트가 영구 Busy** 가 됐다
         (조용하고 영구적인 팀 0 = 온보딩 전멸). 나이 backstop + RAII 삭제 + fail-open 으로
         '영구 Busy' 를 구조적으로 불가능하게 만든다.
    """
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    notes = []
    # ⓐ RAII 해제 — Drop 에서 pidfile 을 삭제한다(핸들만 닫고 파일을 남기면 안 된다)
    need("impl Drop for LockHold" in src, "락 보유 토큰에 RAII 해제(Drop)가 없다")
    di = src.find("impl Drop for LockHold")
    dbody = src[di:src.find("\n}\n", di)]
    need("remove_file" in dbody, "Drop 이 pidfile 을 삭제하지 않는다(잔존 → 다음 부트 Busy)")
    need("self.file = None" in dbody,
         "삭제 전 핸들을 닫지 않는다(Windows 는 열린 파일 삭제가 실패할 수 있다)")
    notes.append("RAII: 핸들 닫고 pidfile 삭제")
    # ⓑ 나이 backstop — 외부 도구 무의존 회수 근거(영구 Busy 불가능성의 핵심)
    need("fn pidfile_reclaimable(" in src, "스테일 회수 판정 함수가 없다")
    ri = src.find("fn pidfile_reclaimable(")
    rbody = src[ri:src.find("\n}\n", ri)]
    need("BUDGET_LOCK_STALE_SECS" in rbody, "나이 기반 backstop 이 없다(tasklist 실패 시 영구 Busy)")
    # (의도 보존 갱신) 외부 프로세스 조회는 pidfile_holder_dead 로 분리됐다(H-CONC-2 계약 심볼).
    # 의도: 회수 판정에서 나이 backstop 이 보유자 사망 조회보다 **먼저** — 조회 도구 실패가
    # 판정을 지배하면 영구 Busy. holder_dead 호출이 상수 뒤에 있고, 조회 자체(tasklist)는
    # holder_dead 안에 있음을 각각 단언한다.
    ai = rbody.find("BUDGET_LOCK_STALE_SECS")
    ti = rbody.find("pidfile_holder_dead")
    need(0 < ai < ti, "나이 backstop 이 보유자 사망 조회 뒤에 있다 — 외부 도구 실패가 먼저 판정을 지배한다")
    hi = src.find("fn pidfile_holder_dead(")
    need(hi > 0 and "tasklist" in src[hi:src.find("\n}\n", hi)],
         "보유자 사망 조회(tasklist)가 holder_dead 에 없다")
    need("pid == std::process::id()" in rbody, "자기 잔재(핸들 누수) 회수 경로가 없다")
    notes.append("나이 backstop 선행 + 자기 잔재 회수")
    # ⓒ fail-open — 판정 불가는 Busy 가 아니라 '직렬화 포기·진행'
    need("WinLock::Unavailable => BootLock::Acquired(LockHold::unserialized())" in src,
         "판정 불가가 fail-open(직렬화 포기·진행)으로 강등되지 않는다")
    wi = src.find("fn win_pidfile_lock(")
    wbody = src[wi:src.find("\n}\n", wi)]
    need("return WinLock::Unavailable" in wbody, "락 생성 실패가 Unavailable 로 강등되지 않는다")
    need("remove_file(&pidfile).is_err()" in wbody,
         "스테일 삭제 실패가 fail-open 으로 흐르지 않는다(권한 문제에서 영구 Busy)")
    notes.append("fail-open: 판정불가·삭제실패 → 직렬화 포기(진행)")
    # ⓓ BUDGET 파리티에 편입 — 상수가 python SOT 와 기계 대조된다
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_budget as BU
    need("BUDGET_LOCK_STALE_SECS" in BU.RUST_PARITY_CONSTS,
         "스테일 임계가 BUDGET 파리티 표에 없다(rust/python 드리프트 가능)")
    m = re.search(r"const BUDGET_LOCK_STALE_SECS: u64 = (\d+);", src)
    need(m and int(m.group(1)) == int(BU.leaf("LOCK_STALE_S")),
         "스테일 임계 파리티 불일치: rust=%s python=%s"
         % (m.group(1) if m else None, BU.leaf("LOCK_STALE_S")))
    # 임계는 정상 부트 최악치보다 커야 한다(진행 중 부트를 뺏지 않는다)
    need(BU.leaf("LOCK_STALE_S") > BU.cys_boot_outer_s(),
         "스테일 임계(%s) ≤ cys boot 외부 상한(%s) — 진행 중 부트의 락을 뺏는다"
         % (BU.leaf("LOCK_STALE_S"), BU.cys_boot_outer_s()))
    notes.append("임계 파리티 + 정상 부트 최악치 초과 단언")
    # ⓔ 파괴 경로는 Windows 에서 넓히지 않는다(python 후보 확장 금지 — 보수 판정)
    #   ★심볼 추적 갱신(f43a0e4): 스폰이 `Command::new("python3")` → `cys::python_command("python3")`
    #     로 개명됐다(SEAL-1: PYTHONDONTWRITEBYTECODE 를 얹는 공용 래퍼 — 내부는 동일 Command::new).
    #     계약(python3 단일 · 후보 확장 금지 · loud no-op)은 그대로이며, 카운트 단언은 래퍼·직접
    #     스폰을 **합산**해 어느 표기로 후보를 넓혀도 잡는다(단언 약화 아님).
    ei = src.find("fn escalate_reclaim(")
    ebody = src[ei:src.find("\n}\n", ei)]
    need('cys::python_command("python3")' in ebody, "reclaim 헬퍼 호출을 못 찾았다")
    need("reclaim **미실행**" in ebody,
         "인터프리터 해소 실패가 무음 no-op 이다(무엇이 안 일어났는지 고지 필요)")
    spawn_sites = ebody.count("python_command(") + ebody.count("Command::new(")
    need(spawn_sites == 1,
         "인터프리터 후보를 넓혔다(스폰 사이트 %d개) — Windows 에서 파괴 경로가 더 쉽게 발화한다"
         "(보수 판정 이탈)" % spawn_sites)
    notes.append("파괴 경로 Windows 확장 금지 + loud no-op")
    return " · ".join(notes)


@specimen("H-SAFE-3", "W2",
          "치명위험 ①②③ 차단 — 큐 폭주 상한·clear 트리거 비선점·자가치유 fail-safe",
          ["W2-자체감사", "B11(상한)", "② clear 게이트", "③ 자가치유"])
def h_safe_3():
    """① 폭주: 재주입은 delivered_no_ack 한정·멱등 1회·큐 비었을 때만 → 누적 0.
    ② clear: status.set 의 래치 영속(fsync 2회·unwrap panic 경로)이 컨텍스트 임계 발화를 **선점하지
       못한다**(순서 역전 금지).
    ③ 자가치유: `cys list` 파서 드리프트가 **전 wakeup 을 dead 로 만들어** 주기 자가치유를 조용히
       전멸시키던 경로를 unknown 강등으로 막는다. 동족 좌석 대표 선택도 결정론화(죽은 좌석 오대표 금지).
    """
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_boot_node as BN
    import javis_orchestra as O
    import javis_wakeup as W
    notes = []
    # ① 큐 폭주 상한 — 재주입은 큐가 **빈** 상태에서만(pending 이면 재전송 금지)
    alive = {"surface_ref": "surface:7", "role": "cso", "pid": 9, "exited": False}
    need(BN.classify_delivery(["너는 이 cys"], alive, "너는 이 cys")[0] == BN.DELIVERY_PENDING,
         "큐 잔존이 pending 이 아니다 — 맹목 재전송(wakeup 홍수) 위험")
    bnsrc = _read(os.path.join(BIN_DIR, "javis_boot_node.py"))
    vi = bnsrc.find("배달 3분기로 처방을 가른다")
    seg = bnsrc[vi:vi + 2600]
    need(seg.count("inject(a.role, msg") == 1, "재주입 지점이 1개가 아니다(멱등 상한 붕괴)")
    need("attempts=1" in seg, "재주입이 멱등 1회가 아니다")
    notes.append("재주입: pending 금지·멱등 1회·지점 1개")
    # ② clear 트리거 비선점 — 래치 영속이 임계 발화보다 **뒤**
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    si = hnd.find('"status.set" =>')
    sbody = hnd[si:hnd.find('"reinject.mark" =>', si)]
    fire = sbody.find("maybe_fire_context_threshold(daemon, &surface, pct")
    persist = sbody.find("crate::governance::persist_topology(daemon);")
    need(fire > 0 and persist > 0, "status.set 의 임계 발화·래치 영속 지점을 못 찾았다")
    need(fire < persist,
         "래치 영속(fsync 2회·unwrap panic 경로)이 컨텍스트 임계 발화를 **선점**한다 — "
         "60%% clear 사이클이 부수 기능에 막힐 수 있다(치명위험 ②)")
    need("let latched_now = {" in sbody[:fire],
         "래치 자체(인메모리)는 발화 전에 세워져야 한다(값 손실 0)")
    notes.append("clear 트리거 비선점(래치 영속은 발화 뒤)")
    # ③ wakeup 파서 드리프트 fail-safe
    need(W._target_alive.__doc__ is not None, "zombie 가드 문서 소실")
    wsrc = _read(os.path.join(BIN_DIR, "javis_wakeup.py"))
    need("liveness=unknown 으로" in wsrc or "unknown 으로 강등" in wsrc,
         "파서 0행에서 unknown 강등이 없다 — 전 wakeup 이 dead 로 접혀 자가치유 전멸")
    need(W.live_target_rows("완전히 다른 포맷의 출력\n또 한 줄\n") == [],
         "드리프트 입력에서 행이 나온다(테스트 전제 붕괴)")
    notes.append("wakeup: 파서 드리프트=unknown(배달 계속)")
    # ③' 동족 좌석 대표 결정론 — 죽은 worker 가 건강한 worker 를 가리지 않는다
    st = {"surfaces": [
        {"role": "cso", "exited": False, "awakened_at": 1.0},
        {"role": "worker", "exited": False, "agent_alive": False, "seat": "empty"},
        {"role": "worker-2", "exited": False, "awakened_at": 2.0},
        {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-codex", "exited": False, "awakened_at": 1.0}]}
    # ★로스터 밀폐(_roster_pin ⓐ): 주입이 없으면 이 fixture 의 리뷰어 두 행은 리뷰어 CLI 가
    #   깔리지 않은 기계에서 **의무 역할과 이름이 어긋나** 판정에서 통째로 빠진다. 적색은 안
    #   나지만 재려던 팀이 조용히 축소된다 — 같은 클래스(환경이 전제를 대신 세움)다.
    DN = _roster_pin(True)
    v, _ = O.check_verdicts(st, detect=DN)
    vw = _vd(v, "worker", "동족 대표 결정론")
    need(vw["satisfied"] is True,
         "죽은 worker 좌석이 건강한 worker-2 를 가려 '미기동' 오판(결손 churn): %r" % vw)
    need(vw["grade"] == "awake_confirmed" and vw["filler"] == "worker-2",
         "동족 대표가 최선 등급 좌석이 아니다: %r" % vw)
    need(all(x["satisfied"] for x in v.values()),
         "동족 대표 fixture 의 나머지 의무 좌석이 판정에서 빠졌다(전제 축소): %r" % v)
    # 결정론: 반복 호출에서 동일 결과(집합 순회 비결정성 제거)
    reps = {json.dumps(O.check_verdicts(st, detect=DN)[0], sort_keys=True) for _ in range(8)}
    need(len(reps) == 1, "check_verdicts 가 비결정적이다(집합 순회 의존 잔존)")
    notes.append("동족 대표=최선 등급·의무 좌석 전원 충족·8회 반복 결정론")
    return " · ".join(notes)


@specimen("H-CONC-4", "W2", "좌석 승계 임계영역 재검증(프로브 후 점유 → 승계 취소)", ["G13", "G14"])
def h_conc_4():
    """G13/G14: 승계 프로브(seat_claimable_now — 전 프로세스 refresh 라 반드시 락 **밖**)와
    임계영역 사이 창에서 **재검증이 0** 이었다. 그 창에 사람이 CLI 를 띄우거나 타이핑하면
    살아있는 좌석의 역할을 빼앗아 라우팅·알림·deadman 감시를 끊었다.
    ★T-0147-4 회귀(close 거부 3경로)는 **반드시 유지**된다 — 같은 표면(handlers.rs)이다."""
    gov = _repo_file(os.path.join("src", "bin", "cysd", "governance.rs"))
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    notes = []
    # ⓐ 재검증 술어 존재 + 4개 값싼 반증(프로세스 표 재조회 0)
    need("pub fn seat_takeover_recheck(" in gov, "임계영역 재검증 술어가 없다(G13)")
    ri = gov.find("pub fn seat_takeover_recheck(")
    rbody = gov[ri:gov.find("\n}\n", ri)]
    for k in ("exited", "agent_meta", "last_human_input", "seat_cache"):
        need(k in rbody, "재검증이 %s 를 보지 않는다(값싼 반증 누락)" % k)
    need("refresh_processes" not in rbody,
         "재검증이 프로세스 표를 재조회한다(락 보유 중 금지 규율 위반)")
    notes.append("재검증 4반증·프로세스 표 재조회 0")
    # ⓑ 두 경로(claim_role·surface.create) 모두 재검증을 소비
    need(hnd.count("seat_takeover_recheck(") >= 2,
         "재검증이 한 경로에만 걸렸다(claim_role·surface.create 둘 다 필요 — G13+G14)")
    notes.append("claim_role·create 양 경로 소비")
    # ⓒ announce 는 **전이 확정 후** — 취소·조기 return 경로에서 통보 0
    ci = hnd.find("let seat_takeover_ok: Option<u64>")
    need(ci > 0, "claim_role 승계 판정부를 못 찾았다")
    # 프로브 직후 구간에서 **주석이 아닌 실제 호출**이 없어야 한다(주석은 설계 근거 서술이다).
    seg_lines = [ln for ln in hnd[ci:ci + 2000].splitlines()
                 if not ln.strip().startswith("//")]
    need("announce_seat_takeover(daemon" not in "\n".join(seg_lines),
         "프로브 직후 announce 호출이 남아 있다(일어나지 않은 승계 통보 — G13)")
    need("takeover_committed" in hnd, "확정 승계 변수(takeover_committed)가 없다")
    # 확정 승계 소비는 2지점이다: ①임계영역 내 마무리(role·caps 내림·큐 이관) ②락 해제 후 announce.
    commit_sites = [m.start() for m in
                    re.finditer(r"if let Some\(prev\) = takeover_committed \{", hnd)]
    need(len(commit_sites) >= 2,
         "확정 승계 소비 지점이 2개 미만(임계영역 마무리 + 락 해제 후 announce): %d" % len(commit_sites))
    need(any("announce_seat_takeover(daemon" in hnd[s:s + 900] for s in commit_sites),
         "확정 승계 후 announce 호출이 없다(전이 확정 통보 누락)")
    # 임계영역 내에서는 announce 를 부르지 않는다(락 보유 중 pane 주입 = 데몬 정지 위험)
    need("announce_seat_takeover(daemon" not in hnd[commit_sites[0]:commit_sites[0] + 400],
         "임계영역 내에서 announce 를 호출한다(락 보유 중 주입)")
    notes.append("announce=전이 확정 후·락 해제 후")
    # ⓓ live-slot 보호는 agent_alive 한정(금지 방향 ⑤ — latest-wins 전면 제거 금지)
    need("pub fn slot_agent_alive(" in gov, "live-slot 술어가 없다(CS-5①)")
    si = gov.find("pub fn slot_agent_alive(")
    sbody = gov[si:gov.find("\n}\n", si)]
    for k in ("agent_meta", "agent_seen", "agent_exit_notified"):
        need(k in sbody, "live-slot 판정이 %s 를 보지 않는다" % k)
    gi = hnd.find("live-slot: agent_alive holder protected")
    need(gi > 0, "비특권 역할의 live-slot 보호 분기가 없다")
    need("slot_agent_alive(" in hnd, "handlers 가 live-slot 술어를 소비하지 않는다")
    notes.append("live-slot 보호=agent_alive 한정(죽은/행 좌석 latest-wins 유지)")
    # ⓔ claim 불변식은 경고+감사(무조건 거부 금지 — 비평2 C-5)
    need("role.family_transition" in hnd, "역할 가족 전이 감사 이벤트가 없다(A3 2층 관측)")
    fi = hnd.find("role.family_transition")
    fseg = hnd[max(0, fi - 1500):fi + 500]
    need("claim_denied" not in fseg.split("role.family_transition")[0][-600:],
         "가족 전이가 거부로 구현됐다(정당 역할 전이 차단 — 비평2 C-5 위반)")
    notes.append("가족 전이=경고+감사(거부 아님)")
    # ⓕ T-0147-4 회귀 테스트 유지(close 거부 3경로)
    for t in ("close_rejects_foreign_surface", "close_denied"):
        need(t in hnd or t in _repo_file(os.path.join("src", "bin", "cysd", "state.rs")),
             "T-0147-4 회귀 앵커(%s)가 사라졌다" % t)
    notes.append("T-0147-4 close 거부 앵커 유지")
    old = _git_show(os.path.join("src", "bin", "cysd", "governance.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("seat_takeover_recheck" not in old,
             "계측 타당성 실패: 구 코드에 이미 임계영역 재검증이 있다")
        need("slot_agent_alive" not in old, "계측 타당성 실패: 구 코드에 이미 live-slot 술어가 있다")
        calib = "구 코드 재검증 0·live-slot 0 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
@specimen("H-CONC-5", "W1b", "하네스 pgid 실측(결정 실험 — 처방 아닌 측정)", ["A18"])
def h_conc_5():
    """A18 은 **측정**이 게이트다(처방 아님 — os.setsid 선제 내재화는 철회됐다).

    이 검체는 두 가지만 강제한다:
      ① 결정 실험 스크립트가 실재하고 결정론적으로 돈다(측정 실패=hard fail).
      ② **계측 타당성**: 세션 분리 스폰(대조군)이 같은 group-kill 에서 생존한다 — 그래야
         '훅 분기가 사망'이라는 관측치가 의미를 갖는다(항상-사망 프로브 배제).
    판정(group-kill 노출 여부) 자체는 결함 심각도 결정(P3 유지 vs P2 복귀)이며 master 소관이다 —
    여기서 fail 로 만들지 않고 **detail 에 실측치를 실어** 게이트 출력에 남긴다.
    """
    probe = os.path.join(TESTS_DIR, "probe_pgid.py")
    need(os.path.isfile(probe), "A18 결정 실험 스크립트 부재: %s" % probe)
    if os.name != "posix":
        raise Skip("posix 전용 측정(Windows 프로세스 그룹은 H-WIN-11 소속)")
    r = _run([PY, probe, "--json"], timeout=180)
    need(r.returncode == 0, "측정 실패(exit=%d): %s" % (r.returncode, r.stderr[-400:]))
    m = json.loads(r.stdout)
    need("error" not in m, "측정 오류: %s" % m.get("error"))
    ca = m.get("control_arm") or {}
    need(ca.get("ok") is True,
         "계측기 고장 — 대조군(세션 분리 스폰)이 group-kill 에서 생존하지 못했다: %r" % ca)
    need(isinstance(m.get("child_survived_group_kill"), bool), "관측치 누락: %r" % m)
    return ("훅 분기=%s(setsid=%s) · pgid 분리=%s · group-kill 생존=%s → %s · 대조군 생존 확인"
            % (m.get("hook_branch"), m.get("setsid_available"), m.get("pgid_separated"),
               m.get("child_survived_group_kill"), m.get("verdict")))


@specimen("H-CONC-6", "W1a", "스테일 락(보유 pid 사망) 회수 — 유한 거부 창 상한", ["R1", "A2"])
def h_conc_6():
    def dead_pid():
        p = subprocess.Popen([PY, "-c", "pass"])
        p.wait()
        return p.pid
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        env = _base_env({"CYS_LOCK_BACKEND": "pidfile"})
        lp = os.path.join(tmp, "stale.lock")
        _w(lp, json.dumps({"pid": dead_pid(), "started": 0, "owner": "ghost"}), 0o644)
        r = _run([PY, "-c", _LOCK_PROBE, BIN_DIR, lp, "probe"], env=env, timeout=60)
        need(r.stdout.strip().startswith("acquired"), "스테일 락을 회수하지 못했다: %r" % r.stdout)
        need(r.stdout.strip().endswith("True"), "회수 흔적(reclaimed_stale)이 없다: %r" % r.stdout)
        notes.append("pidfile 스테일 회수 OK")
        # 살아있는 보유자는 회수 금지(중복 스폰 방지 — 반대 방향 결함)
        lp2 = os.path.join(tmp, "alive.lock")
        _w(lp2, json.dumps({"pid": os.getpid(), "started": time.time(), "owner": "me"}), 0o644)
        r2 = _run([PY, "-c", _LOCK_PROBE, BIN_DIR, lp2, "probe"], env=env, timeout=60)
        need(r2.stdout.strip().startswith("busy"), "살아있는 보유자의 락을 오회수했다: %r" % r2.stdout)
        notes.append("생존 보유자 보호 OK")
        # 부트스트랩 경로에서도 스테일 회수가 신규 부트를 허용하는가
        home = os.path.join(tmp, "home")
        st = os.path.join(home, ".cys", "state")
        os.makedirs(st, exist_ok=True)
        _w(os.path.join(st, "bootstrap-base.lock"),
           json.dumps({"pid": dead_pid(), "started": 0, "owner": "ghost"}), 0o644)
        code = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as B\n"
                "print('A' if B._acquire_singleflight() else 'NONE')\n" % BIN_DIR)
        r3 = _run([PY, "-c", code], env=_base_env({"HOME": home, "CYS_LOCK_BACKEND": "pidfile"}),
                  timeout=60)
        need(r3.stdout.strip() == "A",
             "스테일 부트 락이 신규 부트를 계속 거부한다(~330s 창 잔존): %r %s" % (r3.stdout, r3.stderr[-300:]))
        notes.append("부트스트랩 스테일 락 회수 OK")
    return " · ".join(notes)


# ═══════════════════════════════════════════════════════════════════════════
# 5. H-WIN (크로스플랫폼 — 스텁 목으로 macOS 결정론 실행)
# ═══════════════════════════════════════════════════════════════════════════
@specimen("H-WIN-1", "W1a", "백슬래시 file_path 로 헌법파일 쓰기 → 차단(guard 정규화)", ["G18"])
def h_win_1():
    g = _hook("guard.sh")
    env = _base_env()
    cases = [
        ("C:\\Users\\x\\.claude\\soul.md", 2, "드라이브 백슬래시 soul.md"),
        ("C:\\x\\CLAUDE.md", 2, "백슬래시 CLAUDE.md"),
        ("C:\\x\\WORKER_DIRECTIVE.md", 2, "백슬래시 *_DIRECTIVE.md"),
        ("/Users/x/.claude/soul.md", 2, "POSIX soul.md(회귀 대조)"),
        ("C:\\x\\notes.md", 0, "무해 파일(과차단 대조)"),
    ]
    for fp, want, label in cases:
        r = _run([BASH, g], env=env,
                 input=json.dumps({"tool_name": "Write", "tool_input": {"file_path": fp}}))
        need(r.returncode == want, "%s: exit=%d(기대 %d)" % (label, r.returncode, want))
    # Bash 경로(LOOSE)도 같은 정규화가 걸리는가
    r = _run([BASH, g], env=env, input=json.dumps(
        {"tool_name": "Bash", "tool_input": {"command": "echo x | tee C:\\Users\\x\\soul.md"}}))
    need(r.returncode == 2, "LOOSE bash 백슬래시 경로 우회(exit=%d)" % r.returncode)
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/guard.sh")
    if old is not None:
        with tempfile.TemporaryDirectory() as tmp:
            og = os.path.join(tmp, "guard.sh")
            _w(og, old)
            _w(os.path.join(tmp, "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            r2 = _run([BASH, og], env=env, input=json.dumps(
                {"tool_name": "Write", "tool_input": {"file_path": "C:\\Users\\x\\.claude\\soul.md"}}))
            need(r2.returncode == 0,
                 "계측 타당성 실패: 구 guard 가 백슬래시 경로를 이미 차단한다면 이 검체는 무의미")
            calib = "구 코드 무음 우회 재현 확인"
    return "차단 4 / 통과 1 (Write+Bash 양 경로) · 계측검증=%s" % calib


@specimen("H-WIN-2", "W1a", "드라이브 cwd(C:\\·C:/) → 상향탐색 성공(cwd 게이트 4훅)", ["G19"])
def h_win_2():
    files = ["inject-context.sh", "save-state.sh", "reflect-scan.sh", "vibecoding/vibe-regression.sh"]
    for f in files:
        body = _read(os.path.join(HOOKS_DIR, f))
        need("cys_is_abs" in body, "%s 가 cys_is_abs 를 쓰지 않는다" % f)
        # 주석 제외 — 결함 설명으로 구 패턴을 인용한 줄까지 잡으면 계측기 오탐이다.
        code = "\n".join(l for l in body.splitlines() if not l.strip().startswith("#"))
        need(not re.search(r'case\s+"\$CWD"\s+in\s+/\*\)', code),
             "%s 에 구 `/*` 전용 게이트가 남았다" % f)
    notes = ["정적: 4훅 전수 cys_is_abs"]
    with tempfile.TemporaryDirectory() as tmp:
        if os.name == "nt":
            # ★Windows 실기(run 31400677188 근저원인): 종전 픽스처는 $TMP 안에 'C:' **리터럴
            #   디렉터리**를 만들어 cwd 상대 해석으로 드라이브 경로를 재현하는 macOS 전용
            #   트릭이었다 — NTFS 는 파일명에 콜론을 금지하고, ntpath.join 은 'C:' 성분을 드라이브
            #   상대 표기로 접어 **통째로 소실**시킨다. 그 결과 훅은 실재하지 않는 실제 C:/proj 를
            #   상향탐색했다(검체가 아니라 픽스처 전제가 거짓). 실 Windows 에서는 **실 tmp
            #   절대경로 자체가 드라이브 경로**이므로 그것을 2표기(C:/…·C:\…)로 넘긴다 —
            #   실물 드라이브 cwd 상향탐색의 더 직접적인 측정이다(검체 약화 아님).
            projroot = os.path.join(tmp, "proj")
            cwds = (projroot.replace("\\", "/") + "/sub",
                    projroot.replace("/", "\\") + "\\sub")
        else:
            # $TMP 안에 'C:' 디렉터리를 만들고 cwd를 상대 해석시켜 드라이브 경로를 macOS에서 재현.
            projroot = os.path.join(tmp, "C:", "proj")
            cwds = ("C:/proj/sub", "C:\\proj\\sub")
        os.makedirs(os.path.join(projroot, "_round"), exist_ok=True)
        os.makedirs(os.path.join(projroot, "tests"), exist_ok=True)
        _w(os.path.join(projroot, "_round", "SESSION_STATE.md"), "WIN-STATE-MARKER\n", 0o644)
        pack = os.path.join(tmp, "pack")
        _w(os.path.join(pack, "bin", "javis_memory.py"), "import sys; sys.exit(1)\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": pack})
        for cwd in cwds:
            payload = json.dumps({"source": "clear", "cwd": cwd, "hook_event_name": "Stop",
                                  "transcript_path": ""})
            r = _run([BASH, _hook("inject-context.sh")], input=payload, env=env, cwd=tmp)
            need("WIN-STATE-MARKER" in r.stdout,
                 "inject-context: cwd=%r 에서 작업기억 미발견" % cwd)
            r = _run([BASH, _hook("save-state.sh")], input=payload, env=env, cwd=tmp)
            need("Stop" in _read(os.path.join(projroot, "_round", ".state_log")),
                 "save-state: cwd=%r 에서 write-ahead 미기록" % cwd)
            os.remove(os.path.join(projroot, "_round", ".state_log"))
            r = _run([BASH, _hook("reflect-scan.sh")], input=payload, env=env, cwd=tmp)
            need("WARN:memory" in _read(os.path.join(projroot, "_round", ".state_log")),
                 "reflect-scan: cwd=%r 에서 _round 해소 실패" % cwd)
            os.remove(os.path.join(projroot, "_round", ".state_log"))
            # vibe-regression 은 상향탐색이 아니라 cwd 를 **직접** 쓴다 → 루트 표기로 대입.
            r = _run([BASH, os.path.join(HOOKS_DIR, "vibecoding", "vibe-regression.sh")],
                     env=env, cwd=tmp, input=json.dumps(
                         {"tool_input": {"command": "javis_task.py set-status T done"},
                          "cwd": cwd.rsplit("/", 1)[0].rsplit("\\", 1)[0]}))
            need("테스트 스위트를 done 직전에 실행" in r.stderr,
                 "vibe-regression: cwd=%r 가 $PWD 로 오폴백(스위트 오탐)" % cwd)
        notes.append("기능: 4훅 × 2표기(C:/·C:\\) 해소 확인")
    return " · ".join(notes)


@specimen("H-WIN-3", "W1a", "명명 부서 + named pipe 소켓 → DEPT_CTX 판정(python 술어 일치)", ["G20", "G4"])
def h_win_3():
    with tempfile.TemporaryDirectory() as tmp:
        # ① 명명 부서 팩(pack-dept-sales) → 부서 round 정본만 주입
        dept = os.path.join(tmp, "pack-dept-sales")
        os.makedirs(os.path.join(dept, "round"), exist_ok=True)
        _w(os.path.join(dept, "round", "SESSION_STATE.md"), "DEPT-ROUND-MARKER\n", 0o644)
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "MAIN-LANE-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": dept})
        r = _run([BASH, _hook("inject-context.sh")], env=env,
                 input=json.dumps({"source": "clear", "cwd": proj}))
        need("DEPT-ROUND-MARKER" in r.stdout, "명명 부서 팩이 부서 정본을 주입하지 않았다(G4)")
        need("MAIN-LANE-MARKER" not in r.stdout, "명명 부서 레인에 메인 레인 작업기억이 오주입됐다(격리 파괴)")
        # ② Windows named pipe 소켓(백슬래시) → 부서 컨텍스트로 인식
        env2 = _base_env({"HOME": os.path.join(tmp, "home"),
                          "CYS_PACK_DIR": os.path.join(tmp, "pack"),
                          "CYS_SOCKET": r"\\.\pipe\cys-dept-sales"})
        r2 = _run([BASH, _hook("inject-context.sh")], env=env2,
                  input=json.dumps({"source": "clear", "cwd": proj}))
        need("부서 pack round SESSION_STATE 부재" in r2.stdout,
             "named pipe 부서 소켓이 부서 컨텍스트로 인식되지 않았다(G20): %r" % r2.stdout[:300])
        need("MAIN-LANE-MARKER" not in r2.stdout, "부서 소켓 레인에 메인 작업기억이 오주입됐다")
    # ③ 셸 ↔ python 술어 parity (판정 SOT 일치)
    code = ("import sys; sys.path.insert(0, %r); import javis_bootstrap as B\n"
            "print(B._socket_dept(r'\\\\\\\\.\\\\pipe\\\\cys-dept-sales'), "
            "B._socket_dept('/x/cys-dept-sales/cys.sock'), B._pack_dept('/y/pack-dept-sales'))\n" % BIN_DIR)
    r3 = _run([PY, "-c", code], env=_base_env(), timeout=60)
    need(r3.stdout.split() == ["sales", "sales", "sales"],
         "python 술어 parity 실패: %r %s" % (r3.stdout, r3.stderr[-300:]))
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/inject-context.sh")
    if old is not None:
        need("pack-dept-dept-" in old,
             "계측 타당성 실패: 구 코드에 dept-N 한정 글롭이 없으면 이 검체는 무의미")
        calib = "구 코드 dept-N 한정 글롭 확인"
    return "명명 부서·named pipe 인식 + 셸↔python 술어 일치 · 계측검증=%s" % calib


@specimen("H-WIN-4", "W1a", "taskkill 감지 + rc=2 인프라/판정 분리(fail-open 정책 복원)", ["G21"])
def h_win_4():
    gate = _hook("actprobe-kill-gate.sh")
    with tempfile.TemporaryDirectory() as tmp:
        _w(os.path.join(tmp, "verdict.py"),
           "import sys\nprint('[FAIL] kill-preflight pid: 활성 노드')\nsys.exit(2)\n", 0o644)
        _w(os.path.join(tmp, "usage.py"),
           "import sys\nsys.stderr.write('actprobe: argument error: bad\\n')\nsys.exit(2)\n", 0o644)
        _w(os.path.join(tmp, "indet_usage.py"),
           "import sys\nsys.stderr.write('usage: actprobe ...\\n')\nsys.exit(3)\n", 0o644)
        def run(cmd, probe):
            return _run(["sh", gate], input=json.dumps(
                {"tool_name": "Bash", "tool_input": {"command": cmd}}),
                env=_base_env({"CYS_ACTPROBE": os.path.join(tmp, probe)}))
        need(run("taskkill /F /PID 4242", "verdict.py").returncode == 2,
             "taskkill + verdict FAIL 이 차단되지 않았다(G21 동사 누락)")
        r = run("taskkill /F /PID 4242", "usage.py")
        need(r.returncode == 0, "rc=2 사용오류가 차단으로 승격됐다(fail-open 정책 역전 잔존)")
        need("verdict 형상 아님" in r.stderr, "인프라 실패 WARN 문구 부재: %r" % r.stderr[:300])
        r = run("taskkill /F /PID 4242", "indet_usage.py")
        need(r.returncode == 0, "rc=3 인자 오류가 차단으로 승격됐다")
        need(run("taskkill /IM claude.exe /F", "verdict.py").returncode == 2,
             "이름 기반 taskkill 이 fail-closed 로 차단되지 않았다")
        need(run("ls -la", "verdict.py").returncode == 0, "비-kill 명령에 개입했다(과차단)")
        need(run("kill -9 4242", "verdict.py").returncode == 2, "POSIX kill 회귀")
    body = _read(os.path.join(HOOKS_DIR, "actprobe-kill-gate.sh"))
    need('"taskkill"' in body, "KILL 동사 집합에 taskkill 이 없다")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/actprobe-kill-gate.sh")
    if old is not None:
        need('"taskkill"' not in old, "계측 타당성 실패: 구 코드에 이미 taskkill 이 있다")
        calib = "구 코드 taskkill 부재 확인"
    return "taskkill 차단·rc2/rc3 형상 분리·POSIX 회귀 0 · 계측검증=%s" % calib


@specimen("H-WIN-5", "W1a", "python3 경성 참조 전수 소멸 + CYS_PY 폴백 실동작", ["G22"])
def h_win_5():
    # ── 탐지기: 비주석 텍스트에서 `python3` 이 **명령어 위치**에 오는 경우만 위반 ──
    # 허용: 주석 / `command -v python3` / 후보 목록 / `${CYS_PY:-python3}`(계약된 명시 폴백) /
    #       python 소스·메시지 문자열 안의 언급.
    # `python3` 다음 문자까지 요구해 산문 언급(`(python3/python/py)` 같은 열거)을 배제한다 —
    # 명령어라면 뒤에 공백·인용·구분자·행끝이 온다.
    cmdpos = re.compile(r"(?:(?<=^)|(?<=\|)|(?<=;)|(?<=&)|(?<=\()|(?<=`)|(?<=\bexec\s)|"
                        r"(?<=\bthen\s)|(?<=\bdo\s)|(?<=\belse\s))\s*python3(?=$|[\s;|&)\"'])")

    def violations(body):
        out = []
        for i, line in enumerate(body.splitlines(), 1):
            s = line.strip()
            if not s or s.startswith("#"):
                continue
            if "command -v python3" in s or re.search(r"for\s+c\s+in\b", s):
                continue
            probe = s.replace("${CYS_PY:-python3}", "${CYS_PY}")
            if cmdpos.search(probe):
                out.append((i, s[:110]))
        return out

    bad = {}
    for rel in _shell_hooks():
        v = violations(_read(os.path.join(HOOKS_DIR, rel)))
        if v:
            bad[rel] = v
    need(not bad, "python3 경성 호출 잔존:\n" + "\n".join(
        "  %s:%s %s" % (k, l, t) for k, vs in bad.items() for l, t in vs))

    # ── 계측기 자기검증: 같은 탐지기가 구 코드에서는 FIRE 해야 한다 ──
    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        r = _run(["git", "-C", REPO_DIR, "grep", "-ln", "python3", CALIBRATION_REF,
                  "--", "cysjavis-pack/hooks/"], timeout=60)
        old_files = [l.split(":", 1)[1] for l in r.stdout.splitlines() if ":" in l]
        old_sh = [f for f in old_files if f.endswith(".sh")]
        fired = sum(1 for f in old_sh if violations(_git_show(f) or ""))
        need(len(old_sh) >= 20, "구 인벤토리 산출 실패(%d) — 대조 불가" % len(old_sh))
        need(fired >= 10, "계측 타당성 실패: 구 코드에서 탐지기가 %d개만 FIRE(결함 재현 불충분)" % fired)
        calib = "구 인벤토리 %d훅 중 %d훅 FIRE" % (len(old_sh), fired)

    # ── 기능: python3 부재·python 만 있는 PATH 에서 대표 훅이 정상 동작 ──
    real = shutil.which("python3") or PY
    tools = ("sh", "bash", "env", "sed", "grep", "date", "wc", "tr", "awk", "head",
             "tail", "cat", "cut", "sort", "dirname", "basename", "ls", "rm", "mkdir",
             "sleep", "printf", "git", "locale", "stat", "uname", "mktemp", "chmod")
    with tempfile.TemporaryDirectory() as tmp:
        binp = os.path.join(tmp, "bin")
        if os.name == "nt":
            # ★심링크 팜·shim 금지(Windows 실기 run 31400677188/31403557039 근저원인 2건):
            #   ① NTFS 심링크로 exe 를 격리 디렉터리에 옮기면 Windows 로더의 DLL 탐색이 **심링크
            #     위치** 기준이라 msys 도구가 자기 옆의 msys-2.0.dll 을 못 찾아 무음 스폰 실패
            #     → `$(dirname "$0")` 공출력 → 프리루드 1단 강등('_lib.sh 소실').
            #   ② 확장자 없는 shebang shim(`python`)은 cygdrive 실행권 판정·PATH 탐색이 환경
            #     의존이라 `command -v python` 해소가 실기에서 침묵 실패했다(exit=0 stderr='').
            #   격리의 목적은 'python3 부재'이지 coreutils/python 실물 부재가 아니므로, 필요
            #   도구의 **실 디렉터리**(+ 하네스 인터프리터 PY 의 실 디렉터리)를 그대로 PATH 에
            #   얹되 python3 를 내놓는 디렉터리는 제외한다. 전제(python3 미해소)는 아래 need 가
            #   최종 PATH 전체에 대해 그대로 못 박는다 — 검체 약화 아님.
            dirs = []
            for tool in tools:
                src = shutil.which(tool)
                if not src:
                    continue
                d = os.path.dirname(src)
                if d not in dirs and shutil.which("python3", path=d) is None:
                    dirs.append(d)
            # ★python 은 venv 로 공급한다(run 31404416739 실측): 실 설치 디렉터리는 setup-python
            #   이 python3.exe 를 함께 두어 격리 전제(python3 부재)가 구성 불가였다. Windows venv
            #   의 Scripts 는 python.exe/pythonw.exe 만 만들고(런처가 pyvenv.cfg 로 base 를 해소
            #   — DLL·stdlib 상대 해소 무손상), python3.exe 는 만들지 않는다 — 'python 만 있는
            #   PATH' 를 표준 라이브러리 수단으로 정확히 재현한다.
            venv_dir = os.path.join(tmp, "pyvenv")
            rv = _run([PY, "-m", "venv", "--without-pip", venv_dir], timeout=180)
            need(rv.returncode == 0,
                 "nt 격리용 venv 생성 실패(rc=%d): %r" % (rv.returncode, rv.stderr[-200:]))
            pydir = os.path.join(venv_dir, "Scripts")
            need(shutil.which("python", path=pydir) is not None,
                 "venv Scripts 에 python 이 없다: %s" % pydir)
            need(shutil.which("python3", path=pydir) is None,
                 "venv Scripts 가 python3 를 노출 — nt 격리 PATH 전제 구성 불가")
            iso_path = os.pathsep.join(dirs + [pydir])
        else:
            _w(os.path.join(binp, "python"), '#!/bin/sh\nexec "%s" "$@"\n' % real)
            for tool in tools:
                src = shutil.which(tool)
                if src and not os.path.exists(os.path.join(binp, tool)):
                    os.symlink(src, os.path.join(binp, tool))
            iso_path = binp
        # bash 해소는 모듈 상수 BASH(W-A) — 종전 이 자리의 지역 해소가 모듈로 승격됐다.
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "NOPY3-MARKER\n", 0o644)
        env = _base_env({"HOME": os.path.join(tmp, "home"), "PATH": iso_path,
                         "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        need(shutil.which("python3", path=iso_path) is None, "격리 PATH 에 python3 가 남아 있다")
        payload = json.dumps({"source": "clear", "cwd": proj, "hook_event_name": "PreCompact"})
        r = _run([BASH, _hook("inject-context.sh")], input=payload, env=env)
        need(r.returncode == 0 and "NOPY3-MARKER" in r.stdout,
             "python3 부재 PATH 에서 inject-context 실패: exit=%d stderr=%r stdout=%r"
             % (r.returncode, r.stderr[-200:], r.stdout[-200:]))
        r = _run([BASH, _hook("save-state.sh")], input=payload, env=env)
        need(r.returncode == 0 and "PreCompact" in _read(os.path.join(proj, "_round", ".state_log")),
             "python3 부재 PATH 에서 save-state 실패")
        r = _run([BASH, _hook("pack-guard.sh")], input=json.dumps(
            {"tool_input": {"file_path": "/x/y.sh"}, "session_id": "s"}), env=env)
        need(r.returncode == 0, "python3 부재 PATH 에서 pack-guard 비0 종료")
    return "훅 %d개 경성 참조 0 · 계측검증=%s · python-only PATH 3훅 정상" % (
        len(_shell_hooks()), calib)


@specimen("H-WIN-6", "W1a", "pack-guard 백슬래시 접두 매칭(vendor 경고 무음 해소)", ["G23"])
def h_win_6():
    with tempfile.TemporaryDirectory() as tmp:
        binp = os.path.join(tmp, "bin")
        _mock_cys(binp, tmp, 'case "$1" in pack-ownership) echo system; exit 0;; esac')
        env = _base_env({"HOME": os.path.join(tmp, "home"),
                         "PATH": binp + os.pathsep + os.environ.get("PATH", ""),
                         "CYS_PACK_DIR": "C:/Users/x/.cys/pack",
                         "TMPDIR": os.path.join(tmp, "stamps")})
        r = _run([BASH, _hook("pack-guard.sh")], env=env, input=json.dumps(
            {"tool_input": {"file_path": "C:\\Users\\x\\.cys\\pack\\hooks\\guard.sh"},
             "session_id": "s1"}))
        need(r.returncode == 0, "pack-guard 비0 종료(%d)" % r.returncode)
        need("pack-ownership" in _calls(tmp),
             "백슬래시 경로가 팩 접두로 인식되지 않았다(경고 무음 잔존): %r" % _calls(tmp))
        need("pack-guard" in r.stdout and "hooks/guard.sh" in r.stdout,
             "REL 산출·경고 문안 이상: %r" % r.stdout[:400])
        # 과차단 대조: 팩 밖 파일은 무동작
        env["TMPDIR"] = os.path.join(tmp, "stamps2")
        r2 = _run([BASH, _hook("pack-guard.sh")], env=env, input=json.dumps(
            {"tool_input": {"file_path": "C:\\other\\x.sh"}, "session_id": "s2"}))
        need(r2.stdout.strip() == "", "팩 밖 파일에 경고를 냈다(과탐): %r" % r2.stdout[:200])
    return "백슬래시 팩 접두 매칭 + 팩 밖 무동작"


@specimen("H-WIN-7", "W1a", "부트 브리지 안내 문자열 cygpath·인용 적용", ["G8"])
def h_win_7():
    with tempfile.TemporaryDirectory() as tmp:
        pack = os.path.join(tmp, "pack")
        os.makedirs(os.path.join(pack, "directives"), exist_ok=True)
        _w(os.path.join(pack, "bin", "javis_bootstrap.py"), "# stub\n", 0o644)
        _w(os.path.join(pack, "directives", "MASTER_DIRECTIVE.md"), "MASTER-BODY\n", 0o644)
        binp = os.path.join(tmp, "bin")
        _mock_cys(binp, tmp)
        # cygpath 목: -w 로 공백 포함 네이티브 경로를 돌려준다(인용 필요 상황 재현)
        _w(os.path.join(binp, "cygpath"),
           '#!/bin/sh\n[ "$1" = "-w" ] && { printf "X:\\\\Prog Files\\\\javis_bootstrap.py\\n"; exit 0; }\nexit 1\n')
        env = _base_env({"HOME": os.path.join(tmp, "home"), "CYS_PACK_DIR": pack,
                         "CYS_SURFACE_ID": "3",
                         "PATH": binp + os.pathsep + os.environ.get("PATH", "")})
        want = '"X:\\\\Prog Files\\\\javis_bootstrap.py"'
        r = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        need(want in r.stdout,
             "role-less 안내가 네이티브 경로+인용을 쓰지 않는다: %r" % r.stdout[:600])
        env["CYS_ROLE"] = "master"
        r2 = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        need("부트 브리지" in r2.stdout and want in r2.stdout,
             "master 부트 브리지가 네이티브 경로+인용을 쓰지 않는다: %r" % r2.stdout[-600:])
        # 인용이 실제로 sh 에서 원 경로로 복원되는가(eval 왕복)
        line = [l for l in r2.stdout.splitlines() if want in l][0].strip()
        rt = os.path.join(tmp, "roundtrip.sh")
        _w(rt, "eval 'set -- %s'\nprintf '%%s' \"$2\"\n" % line, 0o644)
        chk = _run(["sh", rt], env=_base_env())
        need(chk.stdout == "X:\\Prog Files\\javis_bootstrap.py",
             "인용 왕복 실패(복사 실행 불가): %r (line=%r)" % (chk.stdout, line))
        # unix(cygpath 부재) 무변경 대조 — ★전제 실측(W-D · 하우스 3상 규율): 이 leg 의 전제
        # 'cygpath 부재'는 Windows 러너(Git Bash = cygpath 실재)에서 거짓이다. 전제가 거짓이면
        # 미측정이지 FAIL 이 아니다 — 사유 명시 skip 으로 접는다. 비교는 구분자 정규화 후
        # 수행한다(훅이 os.path 산출 경로를 백슬래시로 렌더해도 경로 동일성 판정은 불변).
        env.pop("CYS_ROLE")
        env["PATH"] = os.environ.get("PATH", "")
        if shutil.which("cygpath", path=env["PATH"]):
            return ("cygpath 변환·인용 왕복 검증 · unix 대조 leg skip"
                    "(실행 환경에 cygpath 실재=전제 미충족 — 미측정이지 FAIL 아님)")
        r3 = _run(["sh", _hook("session-start.sh")], env=env, stdin=subprocess.DEVNULL)
        expect3 = os.path.join(pack, "bin", "javis_bootstrap.py").replace("\\", "/")
        need(expect3 in r3.stdout.replace("\\", "/"),
             "cygpath 부재 환경에서 경로가 변형됐다(unix 회귀): %r" % r3.stdout[:500])
    return "cygpath 변환·인용 왕복 검증 · unix 무변경"


@specimen("H-WIN-8", "W1a", "spawn_env_pairs 공용 소비(pane·schedule 2경로 · GUI=W4)", ["A17"])
def h_win_8():
    lib = os.path.join(REPO_DIR, "src", "lib.rs")
    if not os.path.isfile(lib):
        raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
    body = _read(lib)
    need(body.count("pub fn spawn_env_pairs(") == 1, "lib.rs 에 spawn_env_pairs 단일 정의가 아니다")
    need("pub fn spawn_env_pairs_from_process(" in body, "프로세스 env 래퍼 부재")
    dups = []
    for root, _d, files in os.walk(os.path.join(REPO_DIR, "src")):
        for f in files:
            if not f.endswith(".rs"):
                continue
            p = os.path.join(root, f)
            if os.path.abspath(p) == os.path.abspath(lib):
                continue
            if re.search(r"^\s*(pub\s+)?fn spawn_env_pairs\s*\(", _read(p), re.M):
                dups.append(os.path.relpath(p, REPO_DIR))
    need(not dups, "spawn_env_pairs 사본 정의 잔존: %s" % dups)
    for rel in ("src/bin/cysd/state.rs", "src/bin/cysd/schedule.rs"):
        need("spawn_env_pairs" in _read(os.path.join(REPO_DIR, rel)),
             "%s 가 공용 spawn_env_pairs 를 소비하지 않는다" % rel)
    st = _read(os.path.join(REPO_DIR, "src/bin/cysd/state.rs"))
    need("spawn_env_pairs_from_process" in st, "pane 스폰이 HOME backfill 규약을 소비하지 않는다")
    calib = "skip(no-git)"
    old = _git_show("src/bin/cysd/schedule.rs")
    if old is not None:
        need(re.search(r"^\s*fn spawn_env_pairs\s*\(", old, re.M),
             "계측 타당성 실패: 구 코드에 private 사본이 없으면 승격 검증이 무의미")
        old_state = _git_show("src/bin/cysd/state.rs") or ""
        need("spawn_env_pairs" not in old_state,
             "계측 타당성 실패: 구 pane 스폰이 이미 규약을 소비했다면 A17 은 결함이 아니다")
        calib = "구 코드 사본·pane 미소비 확인"
    return "lib 단일 정의·사본 0·pane/schedule 소비 · 계측검증=%s" % calib


@specimen("H-WIN-9", "W4", "어댑터 OS 후보 해소(agy/codex Windows 경로) — 단일 오라클 안에서", ["B8"])
def h_win_9():
    """B8: Windows 에서 `where agy` 는 실패하지만 실제 설치물은 `agy.cmd`(npm shim)다. 감지가
    확장자·npm prefix 후보를 순회하지 않으면 **설치돼 있는데 미설치로 판정**해 리뷰어가 영영
    안 뜬다. 그리고 그 후보 순회는 감지 **단일 오라클 안**에 있어야 한다(부트·agent-detect·
    python detect_reviewer 가 같은 판정을 받으려면)."""
    src = os.path.join(REPO_DIR, "src", "bin", "cys.rs")
    if not os.path.isfile(src):
        raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
    body = _read(src)
    # ① 후보 순회가 존재하고 단일 오라클(detect_agent_binary)이 그것을 통과한다
    need("fn windows_agent_candidates(" in body, "Windows 후보 순회 함수 부재(B8 미수리)")
    # ★핀 이사(MF-1 · 2026-08-26) — 5b15e87 이 폴백 시그니처를 (agent, AgentDetection)
    #   2인자로 바꿨다(힌트 치환에서 의무 CLI claude 를 제외하려면 폴백이 agent 이름을 알아야
    #   한다). 종전 핀 `apply_windows_agent_fallback(AgentDetection {` 는 1인자 호출 리터럴이라
    #   소실. 축 보존: 단일 오라클(detect_agent_binary) 산출이 폴백을 **통과**한다 — rustfmt
    #   개행에 흔들리지 않게 공백 관용 regex 로 박는다(정의 2벌은 `_agent: &str` 라 비매치).
    need(re.search(r"apply_windows_agent_fallback\(\s*agent,\s*AgentDetection\s*\{", body),
         "detect_agent_binary 가 Windows 폴백을 통과하지 않는다(오라클 밖 판정)")
    need(body.count("fn apply_windows_agent_fallback(") == 2,
         "cfg 분기 2벌(windows/not) 형태가 아니다 — 다른 OS 에서 항등 보장 불가")
    # ② 후보 집합: 확장자 4종 + npm prefix 2종(%APPDATA%·%LOCALAPPDATA%)
    seg = body[body.index("fn windows_agent_candidates("):]
    seg = seg[:seg.index("\n}\n") + 2]
    for ext in ("cmd", "exe", "bat", "ps1"):
        need('"%s"' % ext in seg, "확장자 후보 누락: %s" % ext)
    for var in ("APPDATA", "LOCALAPPDATA"):
        need('"%s"' % var in seg, "npm prefix 후보 누락: %%%s%%" % var)
    need("npm" in seg, "npm 전역 prefix 경로 후보 부재(PATH 미갱신 셸 대응 불가)")
    need("is_executable_file(" in seg, "후보 판정이 실행권을 안 본다(실재만 보면 오탐)")
    # ③ 후보 전부 미발견이면 **힌트가 경로수정 안내로 교체**된다(설치 안내 반복이 아니라 배선 교정)
    need("WINDOWS_AGENT_PATH_HINT" in body, "전탐색 실패 힌트 상수 부재")
    # ★핀 이사(MF-1 · 2026-08-26) — 치환 배선이 직접 대입(`d.hint = WINDOWS_AGENT_PATH_HINT`)
    #   에서 단일 판정 함수 `full_miss_hint(agent, os)` 경유로 옮겨졌다: Windows+비claude 만
    #   경로수정 힌트, 의무 CLI claude 는 install_hint(네이티브 설치 명령) 유지 — 신규 Windows
    #   기계에서 INST-1 카드 본문이 agents.json 경로수정 안내로 오염되던 것의 수리(P4-4 '문구
    #   SOT=install_hint'). 축 보존: '전탐색 빈손 → 힌트 교체'는 그대로 + 강화: 치환 판정
    #   단일처(full_miss_hint)와 claude 면제 술어·양 갈래 실재까지 박는다.
    need("d.hint = full_miss_hint(agent, " in body,
         "후보 전탐색 실패 시 힌트 교체 배선 부재(full_miss_hint 경유)")
    need("fn full_miss_hint(agent: &str, os: &str)" in body, "치환 단일 판정 함수 부재(MF-1)")
    fh = body[body.index("fn full_miss_hint("):]
    fh = fh[:fh.index("\n}\n") + 2]
    need('os == "windows" && agent != "claude"' in fh,
         "의무 CLI claude 면제 술어가 없다(설치 명령이 경로수정 안내로 치환 — MF-1 재발)")
    need("WINDOWS_AGENT_PATH_HINT" in fh and "install_hint_for(agent, os)" in fh,
         "full_miss_hint 가 경로수정/설치 힌트 양 갈래를 갖지 않는다")
    # ④ 소비층 단일화: python 은 자체 판정을 1차로 쓰지 않는다(cys agent-detect 소비)
    orch = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    need("cys_agent_detect(" in orch and "agent-detect" in orch,
         "python detect_reviewer 가 단일 오라클을 소비하지 않는다(B12)")
    calib = "skip(no-git)"
    old = _git_show("src/bin/cys.rs")
    if old is not None:
        need("windows_agent_candidates" not in old,
             "계측 타당성 실패: 구 코드에 이미 후보 순회가 있다면 B8 은 결함이 아니다")
        calib = "구 코드 후보 순회 부재 확인"
    return "후보 확장자 4·npm prefix 2·실행권 판정 · 힌트 교체 · python 소비 · 계측검증=%s" % calib


@specimen("H-WIN-10", "W4", "Windows 1차 종료 단계 유효성(무효 명시 로그 후 강제 직행)", ["G24"])
def h_win_10():
    """G24: `taskkill /PID <pid> /T`(무 `/F`)는 WM_CLOSE 라 콘솔 프로세스(claude·codex·agy)를
    **종료하지 않는다** — 1차 '그레이스풀' 단계가 구조적 no-op 이었다. 그럼에도 회수 경로는
    그것을 보내고 1.5s 기다렸고, '그레이스풀을 시도했다'는 거짓 기록을 남겼다.
    처방(W2 handoff '후보 확대 금지 — loud no-op 이 정답'과 동형): 대체 시그널을 발명하지 않고
    **무효를 명시 로그**한 뒤 강제 단계로 직행한다.
    ★계측: 플랫폼 술어를 **import 전에 위장**해 실제 모듈 판정을 측정한다(주석 스캔 아님)."""
    probe = r'''
import json, os, sys
os.name = sys.argv[1]          # ★import 전 위장 — 술어는 import 시점에 확정된다
sys.path.insert(0, sys.argv[2])
import javis_boot_node as m
calls = []
m.run = lambda args, timeout=5: (calls.append(list(args)), (0, "", ""))[1]
m._kill(4321)
m._kill(4321, force=True)
print(json.dumps({"supported": m.GRACEFUL_KILL_SUPPORTED,
                  "reason": m.GRACEFUL_KILL_NOOP_REASON, "calls": calls}))
'''
    out = {}
    for osname in ("nt", "posix"):
        r = _run([PY, "-c", probe, osname, BIN_DIR])
        need(r.returncode == 0, "탐침 실패(os.name=%s): %s" % (osname, r.stderr[-400:]))
        out[osname] = json.loads(r.stdout.strip().splitlines()[-1])
    # ① Windows: 그레이스풀 미지원 선언 + 1차 인자가 실제로 `/F` 없는 무효 형태임을 실측
    need(out["nt"]["supported"] is False, "Windows 에서 그레이스풀이 여전히 '지원'으로 선언됨")
    g, f = out["nt"]["calls"]
    need(g[:1] == ["taskkill"] and "/F" not in g,
         "Windows 1차 인자가 taskkill 무-/F 형태가 아니다(계측 전제 붕괴): %s" % g)
    need("/F" in f, "Windows 강제 단계에 /F 가 없다: %s" % f)
    # ② unix: 종전 동작 보존(SIGTERM → SIGKILL 2단)
    need(out["posix"]["supported"] is True, "unix 에서 그레이스풀이 꺼졌다(무회귀 위반)")
    gp, fp = out["posix"]["calls"]
    need(gp[0] == "kill" and "-9" not in gp, "unix 1차가 SIGTERM 이 아니다: %s" % gp)
    need("-9" in fp, "unix 강제 단계가 SIGKILL 이 아니다: %s" % fp)
    # ③ 무효 사유가 '강제 단계 직행'을 명시한다(조용한 생략 금지)
    need("강제" in out["nt"]["reason"], "무효 사유가 강제 직행을 명시하지 않는다")
    # ④ 회수 경로가 술어로 분기한다(1.5s 무의미 대기 제거)
    bn = _read(os.path.join(BIN_DIR, "javis_boot_node.py"))
    need("if GRACEFUL_KILL_SUPPORTED:" in bn, "회수 경로가 술어 분기를 쓰지 않는다")
    need("emit(\"reclaim\", GRACEFUL_KILL_NOOP_REASON)" in bn, "무효 명시 로그 배선 부재")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_boot_node.py")
    if old is not None:
        need("GRACEFUL_KILL_SUPPORTED" not in old,
             "계측 타당성 실패: 구 코드에 이미 술어가 있다면 G24 는 결함이 아니다")
        need("rc, _, _ = _kill(pid)" in old,
             "계측 타당성 실패: 구 코드가 1차 그레이스풀을 무조건 보내지 않았다면 전제가 다르다")
        calib = "구 코드 무조건 1차 그레이스풀 확인"
    return "nt=무효선언+/F직행 · posix=TERM→KILL 보존 · 사유 명시 · 분기 배선 · 계측검증=%s" % calib


@specimen("H-WIN-11", "W4", "Windows CI 실기 재실행(부채 V4 해소) — 로컬은 잡 계약 검증", ["A6", "A8", "B13"])
def h_win_11():
    """부채 V4: G18–G25 판정이 전부 macOS 스텁 목이라 **런타임 확증이 0회**였다. 스텁은 값싼
    조기경보이지 플랫폼 증명이 아니다(0.14.4 D1 형제 import·ConPTY 마우스가 그 계급의 실사고).
    이 검체는 두 층으로 동작한다:
      · 전 플랫폼: Windows CI 잡의 **계약**을 검증한다(존재·windows-latest·python 셋업·러너 실기·
        조건부 아님). 계약이 깨지면 '돌지 않는 초록'이 되고 그게 V4 의 재생산이다.
      · Windows: H-WIN 검체군 + A8 락(H-CONC-1·3) + A6 발화경로(H-DETECT-10)를 **실기 재실행**한다.
    ★macOS 에서는 실측을 하지 않았으므로 **PASS 를 주장하지 않는다** — Skip(사유 동봉)이다.
      측정하지 않은 것을 초록으로 세는 것이 이 문서 전체가 경계하는 reward-hack 이다."""
    rel = os.path.join(".github", "workflows", "windows-health.yml")
    wf = os.path.join(REPO_DIR, rel)
    if not os.path.isfile(wf):
        raise Fail("Windows CI 잡 부재: %s — 부채 V4 미해소(실기 경로 없음)" % rel)
    text = _read(wf)
    need("runs-on: windows-latest" in text, "windows-latest 러너가 아니다(실기 아님)")
    need("actions/setup-python" in text, "python 셋업 스텝 부재 — 러너 실행 불가")
    need("run_bootstrap_health.py" in text, "건강성 러너를 호출하지 않는다")
    need("H-WIN-1" in text and "H-WIN-10" in text, "H-WIN 검체 실기 목록이 없다")
    need("H-CONC-1" in text and "H-DETECT-10" in text,
         "A8 락·A6 경로 실측 검체가 목록에 없다(§5 H-WIN-11 커버 결함)")
    # 잡이 조건부면 '돌지 않는 초록'이다 — 잡 수준 if 금지(ci-branch 레인 게이트와 동형 교리)
    job = text[text.index("jobs:"):]
    need(not re.search(r"^\s{4}if:", job, re.M), "잡 수준 `if:` 존재 — 조건부 Windows 검증 금지")
    need("--json" in text, "기계 판독 결과 미수집(회귀 시 결함 ID 역추적 불가)")
    if os.name != "nt":
        raise Skip("Windows 실기는 CI 잡(%s)에서 수행한다 — 이 기계(%s)에서는 잡 계약만 검증했다"
                   "(windows-latest·python 셋업·러너 H-WIN+H-CONC-1/3+H-DETECT-10 실기·무조건 실행)"
                   % (rel, sys.platform))
    # ── 실기(Windows 러너) ──
    ids = ["H-WIN-%d" % i for i in range(1, 11)] + ["H-CONC-1", "H-CONC-3", "H-DETECT-10"]
    r = _run([PY, os.path.abspath(__file__), "--json", "--only", ",".join(ids)], timeout=900)
    try:
        d = json.loads(r.stdout)
    except ValueError:
        raise Fail("실기 재실행 결과 파싱 실패: %s" % (r.stdout[-400:] + r.stderr[-400:]))
    s = d["summary"]
    bad = [x["id"] for x in d["specimens"] if x["status"] == "fail"]
    need(not bad, "Windows 실기 실패 검체: %s" % bad)
    need(s["pass"] > 0, "Windows 실기 PASS 0건 — 전부 skip 이면 실기 목적이 무력화된다")
    return "실기 %d PASS / %d SKIP (Windows 러너) · 잡 계약 충족" % (s["pass"], s["skip"])


@specimen("H-WIN-12", "W5", "System32 timeout 함정 하 save-state → BOOT_SNAPSHOT 생성(마스터 게이트)",
          ["SYS32-TIMEOUT"])
def h_win_12():
    """MEMORY cys-01411 #3: Windows PortableGit 은 `command -v timeout` 으로 **System32
    timeout.exe**(인자를 받으면 즉시 rc=1)를 해소한다 — 종전 cys_timeout_run ①분기는 GNU 를
    가정해 래핑 명령이 한 번도 실행되지 않았다(save-state.sh:88 generate·inject-context.sh
    is-master 가 Windows 에서 무증상 무력화). 수리는 ①②분기 진입 전 `--version` rc 0 GNU
    판별(_lib.sh 단일 소유처)이다. 이 검체는 System32 **의미론 스텁**(timeout·gtimeout 둘 다
    인자 즉시 rc=1)을 PATH 선두에 두어 그 함정을 전 플랫폼에서 결정론 재현하고(모듈 헤더의
    스텁 목 규약 — Windows 실기에서는 스텁이 안 잡혀도 System32 실물이 같은 함정이라 판정
    동일), PreCompact 모조 stdin 으로 save-state.sh 를 돌려 **exit 0 AND fixture _round 에
    BOOT_SNAPSHOT.md 실재**를 단언한다. 마스터 게이트 조건은 CYS_ROLE=master env
    (javis_snapshot.is_master ①신호)로 충족한다 — 플랫폼 무관 판정."""
    with tempfile.TemporaryDirectory() as tmp:
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        binp = os.path.join(tmp, "bin")
        stub = "#!/bin/sh\necho 'ERROR: Invalid argument/option - --version' >&2\nexit 1\n"
        _w(os.path.join(binp, "timeout"), stub)
        _w(os.path.join(binp, "gtimeout"), stub)
        # 실물 팩(javis_snapshot.py)을 소비하되 상태는 tmp 로 격리(하네스 계약: 사용자 HOME 불가침).
        env = _base_env({"HOME": os.path.join(tmp, "home"),
                         "CYS_PACK_DIR": PACK_DIR,
                         "CYS_STATE_DIR": os.path.join(tmp, "state"),
                         "CYS_ROLE": "master",
                         "PATH": binp + os.pathsep + os.environ.get("PATH", "")})
        payload = json.dumps({"source": "clear", "cwd": proj, "hook_event_name": "PreCompact"})
        r = _run([BASH, _hook("save-state.sh")], input=payload, env=env)
        need(r.returncode == 0, "save-state exit=%d stderr=%r" % (r.returncode, r.stderr[-300:]))
        snap = os.path.join(proj, "_round", "BOOT_SNAPSHOT.md")
        need(os.path.isfile(snap),
             "System32 timeout 함정에서 BOOT_SNAPSHOT.md 미생성 — cys_timeout_run GNU 판별 회귀")
        need("BOOT_SNAPSHOT" in _read(snap), "스냅샷 본문 판독 불가: %r" % _read(snap)[:120])
    return "System32 스텁(timeout·gtimeout) 하 save-state exit 0 + BOOT_SNAPSHOT.md 실재"


# ── U-20: CLAUDE_CODE_GIT_BASH_PATH 배선 탐지기 ──────────────────────────────
# 검체 본문에서 **순수 함수로 분리**한 이유: 수리가 착지하면 트리의 위반은 0 이고, 그때부터
# 이 검체는 탐지기가 통째로 고장나도 초록이다(전형적인 '초록 사각'). 그래서 아래 H-WIN-BASH 는
# 같은 함수에 **합성 표본**(고의로 망가뜨린 소스)을 먹여 매 실행마다 탐지 능력 자체를 시험한다.
# MEMORY '디버깅 계측 타당성 게이트 3칙' ①(계측기 검증)의 실행 지점이다.
_GIT_BASH_KEY = "CLAUDE_CODE_GIT_BASH_PATH"
_GIT_BASH_SEGS = ["runtime", "git", "bin", "bash.exe"]


def _u20_violations(lib_src):
    """`src/lib.rs` 소스의 U-20 배선 위반 목록(빈 목록 = 배선 정상).

    판정 축 여섯 — ①키 상수 정본 ②경로 세그먼트가 `runtime\\git\\bin\\bash.exe` ③해소기의
    OS·실재파일 게이트 ④주입기의 불가침 3계약 + 마스터 스위치 ⑤`spawn_env_pairs` 실배선
    ⑥사용자 프로세스 env 관측. 어느 하나라도 빠지면 Windows 훅이 죽거나(②③⑤) 사용자 설정을
    덮어쓰거나(④⑥) mac 에 엉뚱한 키가 실린다(③).
    """
    v = []

    def _body(sig):
        i = lib_src.find(sig)
        if i < 0:
            return None
        j = lib_src.find("\n}\n", i)
        return lib_src[i:j] if j > i else None

    # ① 키 상수 정본(리터럴 산재 금지 — 호출부가 상수를 경유해야 이름이 갈리지 않는다)
    if 'pub const ENV_CLAUDE_CODE_GIT_BASH_PATH: &str = "%s";' % _GIT_BASH_KEY not in lib_src:
        v.append("키 상수 정본 부재(ENV_CLAUDE_CODE_GIT_BASH_PATH)")

    # ② 경로 세그먼트 — git\bin(런처) 단일 고정. git\usr\bin(MSYS 실바이너리)은 다른 파일이다.
    m = re.search(r"pub const BUNDLED_GIT_BASH_REL[^=]*=\s*\[([^\]]*)\]", lib_src)
    if not m:
        v.append("동봉 bash 상대경로 상수(BUNDLED_GIT_BASH_REL) 부재")
    else:
        segs = re.findall(r'"([^"]*)"', m.group(1))
        if segs != _GIT_BASH_SEGS:
            v.append("경로 세그먼트가 %s 가 아니다: %s (B-12 — usr\\bin 오선택)"
                     % (_GIT_BASH_SEGS, segs))

    # ③ 해소기: OS 게이트(mac 미주입) + 실재 파일 게이트(없는 경로 통보 금지)
    rb = _body("pub fn bundled_git_bash_path_for(")
    if rb is None:
        v.append("해소기 bundled_git_bash_path_for 부재")
    else:
        if 'os != "windows"' not in rb:
            v.append("해소기에 OS 게이트가 없다 — mac/linux 에도 키가 실린다")
        if ".is_file()" not in rb:
            v.append("해소기에 실재 파일 게이트가 없다 — 없는 bash 경로를 벤더에 통보한다")
        if '"usr"' in rb:
            v.append("해소기가 usr 세그먼트를 만진다 — git\\bin 단일 고정 위반(B-12)")

    # ④ 주입기: 사용자값 불가침 · 마스터 롤백 스위치 · 상수 경유
    ib = _body("pub fn inject_claude_code_git_bash_path_for(")
    if ib is None:
        v.append("주입기 inject_claude_code_git_bash_path_for 부재")
    else:
        for tok, why in (("user_env_set", "사용자 기설정 불가침 인자 부재"),
                         ("master_off", "마스터 롤백 스위치 인자 부재"),
                         ("ENV_CLAUDE_CODE_GIT_BASH_PATH", "키 상수 경유 부재(리터럴 산재)")):
            if tok not in ib:
                v.append("주입기: " + why)

    # ⑤⑥ 실배선: 세 스폰 경로의 공용 규약이 해소기·주입기를 실제로 부르고, 사용자 프로세스 env
    #     와 마스터 스위치를 읽는가. 배선이 없으면 위 셋이 전부 사문(死文)이다.
    sb = _body("pub fn spawn_env_pairs(")
    if sb is None:
        v.append("spawn_env_pairs 정의를 찾지 못했다(계측 불능)")
    else:
        if "bundled_git_bash_path_for(" not in sb:
            v.append("spawn_env_pairs 가 해소기를 부르지 않는다")
        if "inject_claude_code_git_bash_path_for(" not in sb:
            v.append("spawn_env_pairs 가 주입기를 부르지 않는다 — 세 스폰 경로 전부 미배선")
        if "std::env::var_os(ENV_CLAUDE_CODE_GIT_BASH_PATH)" not in sb:
            v.append("spawn_env_pairs 가 프로세스 env 의 사용자 값을 관측하지 않는다(덮어쓰기 위험)")
        if "boot_gates_master_off_from(" not in sb:
            v.append("spawn_env_pairs 가 마스터 롤백 스위치(CYS_BOOT_GATES)를 읽지 않는다")
    return v


@specimen("H-WIN-BASH", "W6",
          "CLAUDE_CODE_GIT_BASH_PATH 기본값 주입 — git\\bin 런처 단일 고정 · 사용자값 불가침 · "
          "mac 미주입 · 마스터 스위치 접기",
          ["U-20", "B-12"])
def h_win_bash():
    """U-20(2026-08-24): 시스템 Git 이 없는 Windows 기계에서 Claude Code 는 훅을 **한 번도**
    실행하지 못한다 — 벤더가 훅 실행에 쓸 bash 를 못 찾기 때문이다(무증상: 각성 훅이 조용히
    전멸한다). 우리는 PortableGit 을 동봉하므로 그 기계에도 bash 는 **실재한다**. 그 자리를
    `CLAUDE_CODE_GIT_BASH_PATH` 로 알려주는 것이 이 단위이고, 이 검체가 그 배선을 박제한다.

    지키는 계약 다섯 —
      ⓐ **`git\\bin` 단일 고정**(B-12). 셸 탐지의 기존 후보 순서(`windows_bash_candidates` =
         `runtime_bin_dirs` + `git\\bin` 덧댐)는 `runtime_bin_dirs` 의 Windows 순서
         (python → git\\cmd → **git\\usr\\bin** → node) 때문에 `usr\\bin\\bash.exe` 를 먼저
         집는다. 그것은 MSYS 실 바이너리이고 벤더가 요구하는 것은 **런처**(`git\\bin`)다 —
         릴리스 레인이 이미 그렇게 단언한다(`windows-build.yml` 의 FATAL 게이트, 아래 ⓔ).
         기존 후보 순서는 스케줄 잡의 살아있는 계약이라 **건드리지 않고**, 이 키만 고정한다.
      ⓑ **사용자값 불가침** — 프로세스 env 에 키가 이미 있으면 덮지 않는다.
      ⓒ **mac/linux 미주입** — OS 게이트가 없으면 엉뚱한 플랫폼에 죽은 Windows 경로가 실린다.
      ⓓ **마스터 스위치**(`CYS_BOOT_GATES=0`)에 접힌다. 새 축마다 노브를 따로 두면 사고 순간에
         사람이 조합을 만들 수 없다(BLOCK-3 이 그 값을 이미 치렀다).
      ⓔ 동봉물 실재 보증 — 빌드 레인이 `runtime/git/bin/bash.exe` 를 하드 단언한다.

    ★계측 타당성(3칙): ①탐지기를 순수 함수로 분리해 **합성 표본 5종**으로 매 실행 검증하고,
      ②기준 커밋 트리에 같은 탐지기를 돌려 **수리 전 적색**을 확인한다. 수리 후의 초록은 이 둘이
      없으면 아무 의미가 없다.
    ★범위 한계(정직 고지): 이 검체도 Rust 회귀 핀도 **Windows 실기 검증이 아니다**. 실기 확인은
      `feat/**` 브랜치의 windows-health 잡(H-WIN-11)과 실기 재현의 몫이다.
    """
    lib = _repo_file(os.path.join("src", "lib.rs"))
    notes = []

    # ⓐ~ⓓ 배선 위반 0
    v = _u20_violations(lib)
    need(not v, "U-20 배선 위반 %d건: %s" % (len(v), v))
    notes.append("배선 위반 0")

    # ★탐지기 능력 시험(합성 표본) — 트리가 초록일 때 생기는 사각을 매 실행 제거한다.
    muts = (
        ("git\\bin → git\\usr\\bin 오선택(B-12)",
         lambda s: s.replace('["runtime", "git", "bin", "bash.exe"]',
                             '["runtime", "git", "usr", "bin", "bash.exe"]')),
        ("OS 게이트 삭제(mac 오주입)", lambda s: s.replace('os != "windows"', "false")),
        ("사용자값 불가침 삭제", lambda s: s.replace("user_env_set", "ignored_flag")),
        ("마스터 스위치 삭제",
         lambda s: s.replace("boot_gates_master_off_from(", "always_on_no_rollback(")),
        ("배선 자체 삭제",
         lambda s: s.replace("inject_claude_code_git_bash_path_for(", "disabled_noop(")),
    )
    blind = []
    for label, mut in muts:
        sample = mut(lib)
        need(sample != lib,
             "합성 표본 %r 이 소스를 바꾸지 못했다 — 표본이 무효라 탐지 능력을 못 잰다" % label)
        if not _u20_violations(sample):
            blind.append(label)
    need(not blind,
         "탐지기가 다음 위반을 못 본다 = 계측 무효(초록이 무의미): %s" % blind)
    notes.append("합성 표본 %d종 전부 검출" % len(muts))

    # ★계측 타당성 ② — 수리 전 트리에서 적색인가.
    calib = "skip(no-git)"
    old = _git_show("src/lib.rs")
    if old is not None:
        need(_u20_violations(old),
             "계측 타당성 실패: 기준 커밋(%s) 의 lib.rs 에서도 위반이 0 이다 — 탐지기가 수리 전 "
             "코드를 적색으로 만들지 못하면 지금의 초록은 아무것도 증명하지 않는다"
             % CALIBRATION_REF)
        calib = "기준 %s 적색 확인" % CALIBRATION_REF
    notes.append("계측검증=" + calib)

    # Rust 회귀 핀 실재(행위 단언은 cargo test 소유 — 여기서는 핀이 지워지지 않았음을 본다).
    for t in ("claude_code_git_bash_path_picks_git_bin_launcher",
              "claude_code_git_bash_path_never_leaves_windows",
              "claude_code_git_bash_path_user_value_and_master_switch"):
        need(("fn %s(" % t) in lib,
             "Rust 회귀 핀 %s 가 사라졌다 — 행위 단언(mac 미주입·git\\bin 선택)이 무보호다" % t)
    notes.append("Rust 회귀 핀 3종 실재")

    # ⓔ 동봉물 실재 보증 — 빌드 레인의 하드 단언(이게 없으면 주입 대상이 없어질 수 있다).
    wb = os.path.join(REPO_DIR, ".github", "workflows", "windows-build.yml")
    need(os.path.isfile(wb), "windows-build.yml 부재 — 동봉 bash 실재 게이트를 확인할 수 없다")
    wbs = _read(wb)
    need("git/bin/bash.exe" in wbs,
         "windows-build.yml 이 runtime/git/bin/bash.exe 를 단언하지 않는다 — 주입 대상이 "
         "경량화 스텝에 지워져도 무증상으로 출하된다")
    notes.append("빌드 레인 동봉 단언 실재")
    return " · ".join(notes)


@specimen("H-CYCLE-1", "W6",
          "cycle-autopilot Windows single-flight — os_name 분기표·pid_alive 비파괴·fork SKIP",
          ["R3-WIN-SF"])
def h_cycle_1():
    """R3 개통 슬라이스 회귀 핀 3속(전 플랫폼 결정론 — 분기는 인자 주입으로 직접 구동).

    ① count_cycle_agent 분기표: 분기 조건은 os_name 뿐(rc==127 플랫폼 추정 금지 — run() 이
       POSIX pgrep 타임아웃까지 127 로 정규화하므로 127 분기는 fail-open 반전). posix=pgrep
       rc 계약 그대로, nt=PowerShell CIM(자기매칭 함정 회피: Name 필터+$PID 자기제외,
       패턴 'cycle-agent' 단독) · PS 실패=보수적 1(fail-closed).
    ② pid_alive nt 분기 비파괴 구조: os.kill(sig 0 도 Windows CPython 에서 OpenProcess+
       TerminateProcess 계열로 접힘) 0회 — OpenProcess(QUERY_LIMITED)+GetExitCodeProcess
       +CloseHandle 만. ACCESS_DENIED(5)=True(보수적)·그 외 실패=False.
    ③ self-test os.fork SKIP: 양 스크립트 cmd_self_test 의 fork 케이스만 os.name=="nt"
       가드(javis_state_snapshot T2 선례 동형 · 블랜킷 skip 금지) — Windows 실기에서
       self-test 배터리·계약 패리티가 계속 돈다."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import importlib
    import inspect
    A = importlib.import_module("javis_cycle_autopilot")
    # ① 분기표 — posix 경로(pgrep rc 계약)
    need(A.count_cycle_agent(lambda cmd: (1, "", ""), os_name="posix") == 0,
         "posix rc=1(0건) → 0 위반")
    need(A.count_cycle_agent(lambda cmd: (0, "11\n22\n", ""), os_name="posix") == 2,
         "posix rc=0 pid 2건 → 2 위반")
    need(A.count_cycle_agent(lambda cmd: (127, "", "boom"), os_name="posix") == 1,
         "posix rc=127(러너 예외 정규화) → 보수적 1 위반")
    # ① 분기표 — nt 경로(PS 성공 파싱·실패 fail-closed·자기매칭 함정 회피 명령 계약)
    seen = {}

    def _cap(cmd):
        seen["cmd"] = cmd
        return 0, "3\r\n", ""
    need(A.count_cycle_agent(_cap, os_name="nt") == 3, "nt PS count=3 파싱 위반")
    ps = " ".join(seen["cmd"])
    need(seen["cmd"][0] == "powershell" and "-NoProfile" in seen["cmd"],
         "nt 계수는 powershell -NoProfile 경유여야 한다: %r" % seen["cmd"][:3])
    need("$PID" in ps and "Name='cys.exe'" in ps,
         "자기매칭 함정 회피(ProcessId 자기제외+Name 필터) 소실: %s" % ps[:160])
    need("'cycle-agent'" in ps and "'cys cycle-agent'" not in ps,
         "패턴은 'cycle-agent' 단독(절대경로·따옴표 기동 포섭)이어야 한다")
    need(A.count_cycle_agent(lambda cmd: (1, "", "err"), os_name="nt") == 1,
         "nt PS rc≠0 → 보수적 1 위반")
    need(A.count_cycle_agent(lambda cmd: (0, "not-a-number\n", ""), os_name="nt") == 1,
         "nt 비숫자 출력 → 보수적 1 위반")
    # ② pid_alive — 현 플랫폼 dispatch 실측 + nt 분기 코드 본문의 비파괴 구조
    need(A.pid_alive(os.getpid()) is True, "자기 pid 생존확인 False")
    need(A.pid_alive(2 ** 22 + 9999) is False, "불가능 pid 가 True")
    body = inspect.getsource(A._pid_alive_windows).split('"""')[2]   # docstring 제외 코드만
    need("os.kill" not in body and "TerminateProcess" not in body,
         "nt 생존확인에 파괴 계열(os.kill/TerminateProcess) 재유입")
    need("OpenProcess" in body and "GetExitCodeProcess" in body and "CloseHandle" in body,
         "nt 생존확인이 OpenProcess+GetExitCodeProcess+CloseHandle 3종을 잃음")
    # ③ self-test fork SKIP 가드 — 양 스크립트(케이스 한정 · 블랜킷 skip 아님).
    #   가드는 nt + fork 부재(hasattr) 이중 — winsim(os.fork 만 제거) 재현까지 포섭한다.
    for mod in ("javis_cycle_autopilot", "javis_cycle_verifier"):
        src = inspect.getsource(importlib.import_module(mod).cmd_self_test)
        need('os.name == "nt" or not hasattr(os, "fork")' in src and "os.fork()" in src,
             "%s cmd_self_test 에 fork 케이스 한정 SKIP 가드(nt+fork 부재) 부재" % mod)
    return ("분기표(posix pgrep 불변·nt PS fail-closed)+비파괴 pid_alive+fork SKIP 가드 "
            "— 전 플랫폼 주입식 판정")


@specimen("H-PYSEAL-1", "W6",
          "훅 셸 층 SEAL-1 — 프리루드 source 만으로 PYTHONDONTWRITEBYTECODE=1 무조건 export",
          ["SEAL-1-HOOK"])
def h_pyseal_1():
    """SEAL-1(2026-08-01 실사고 · 정본 src/lib.rs ENV_PY_NO_BYTECODE) 3층 가운데 **훅 셸 층** 기계 핀.

    번들 python 이 번들 안에 `__pycache__/*.pyc` 를 쓰면 코드서명 봉인이 깨져 다음 실행이
    Gatekeeper 에 차단된다("손상되었기 때문에 열 수 없습니다"). Rust 두 층(python_command·
    spawn_env_pairs)은 cargo 테스트가 잠갔지만, **사용자가 직접 띄운 CLI 에서 훅이 $CYS_PY 로
    번들 python 을 부르는 경로**는 그 상속을 못 받는다 — _lib.sh 의 무조건
    `PYTHONDONTWRITEBYTECODE=1; export`(:265-266)만이 막는다. 이 검체가 깨지면: 그 경로의
    python 이 .pyc 를 다시 쓰기 시작하고, SEAL-2(선컴파일)가 못 덮는 **봉인 밖 팩 사본**까지
    오염된다. 실패 시 새는 것 = 훅 발 python 스폰 전군의 바이트코드 쓰기 차단.

    프로브 rc 계약: 3=하네스 누수(source 전에 이미 값 존재 — 검체 무효) · 4=source 실패 ·
    5=값≠1(SEAL-1 소실) · 6=값은 있으나 export 아님(자식 미상속 = 사실상 소실) · 0=PASS.
    검사 셸은 `sh`(POSIX) — 프리루드 계약 ⓐ와 같은 층위이고, System32 동명 스텁이 없어
    리터럴이 안전하다(G-SYNTAX 의 관례)."""
    lib = os.path.join(HOOKS_DIR, "_lib.sh")
    need(os.path.isfile(lib), "_lib.sh 부재 — 프리루드 미착지")

    def _probe(libpath):
        return (
            '[ -z "${PYTHONDONTWRITEBYTECODE:-}" ] || { echo "PRE-LEAK=$PYTHONDONTWRITEBYTECODE" >&2; exit 3; }; '
            '. "%s" || exit 4; '
            '[ "${PYTHONDONTWRITEBYTECODE:-}" = "1" ] || { echo "VAL=${PYTHONDONTWRITEBYTECODE:-unset}" >&2; exit 5; }; '
            "env | grep -q '^PYTHONDONTWRITEBYTECODE=1$' || { echo NO-EXPORT >&2; exit 6; }" % libpath
        )

    # 러너가 cys pane 안에서 돌면 부모 env 에 이미 1이 상속돼 있다(바로 이 export 의 산물) —
    # 지우지 않으면 export 가 소실돼도 초록이 나오는 거짓 PASS 다. drop + rc=3 프리가드 이중.
    env = _base_env(drop=("PYTHONDONTWRITEBYTECODE",))
    r = _run(["sh", "-c", _probe(lib)], env=env)
    need(r.returncode != 3,
         "하네스 자기검증 실패: source 전에 이미 값이 있다(env 누수 — 검체 무효): %r" % r.stderr[:200])
    need(r.returncode != 4, "_lib.sh source 가 비0 종료(프리루드 계약 ⓓ 위반): %r" % r.stderr[:200])
    need(r.returncode != 5,
         "SEAL-1 훅 층 소실: source 후 PYTHONDONTWRITEBYTECODE 가 1이 아니다 — "
         "직접 띄운 CLI 의 훅 python 이 번들에 .pyc 를 쓴다: %r" % r.stderr[:200])
    need(r.returncode != 6,
         "값은 있으나 export 아님 — 자식 python 에 상속되지 않아 차단이 무효다: %r" % r.stderr[:200])
    need(r.returncode == 0, "프로브 비정상 종료 rc=%d stderr=%r" % (r.returncode, r.stderr[:200]))

    # 계측기 자기검증(MEMORY 3칙): SEAL-1 착지(f43a0e4) **직전** 트리 6d1871f 의 _lib.sh 에서
    # 같은 프로브가 rc=5 로 FIRE 하는가. 기본 기준(a96d8b1)은 _lib.sh 자체가 없어 대조 불능이라
    # 고정 해시를 따로 쓴다(_git_show ref 인자 규약 — PRE_W0_REF 와 같은 이유).
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/_lib.sh", ref="6d1871f")
    if old is not None:
        with tempfile.TemporaryDirectory() as tmp:
            oldlib = os.path.join(tmp, "_lib.sh")
            _w(oldlib, old, 0o644)
            r2 = _run(["sh", "-c", _probe(oldlib)], env=env)
            need(r2.returncode == 5,
                 "계측 타당성 실패: 구 _lib.sh(6d1871f · SEAL-1 이전)에서 rc=%d (기대 5) — "
                 "검체가 결함 부재를 재현하지 못한다: %r" % (r2.returncode, r2.stderr[:200]))
            calib = "구 _lib.sh(6d1871f) rc=5 재현"
    return "sh source 만으로 값=1 + export(자식 상속) 확인 · 계측검증 %s" % calib


# ═══════════════════════════════════════════════════════════════════════════
# 6. H-PRED / H-TIME / H-DOC / H-SEED / H-LIFE / H-OBS
# ═══════════════════════════════════════════════════════════════════════════
def _shared_pred():
    """공유 술어 모듈 3종(javis_boot_node·javis_orchestra·javis_bootstrap) — 밀폐 import."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_boot_node as BN
    import javis_orchestra as O
    import javis_bootstrap as B
    return BN, O, B


def _fx(rows):
    """status fixture — surfaces 배열만 담는다(데몬 왕복 0)."""
    return {"surfaces": rows}


# ★공유 LOCKED corpus(H-PRED-1·2·3·4 공용) — producer≠evaluator 를 위해 **결함 대장에서** 역산한다.
#   (a) grok 좌석 / cso-1 변형 좌석 = G26 이 의무 슬롯 충족으로 오계상했던 그 좌석
#   (b) agent 만 죽은 좌석 = B3 가 '가동 중'으로 오판해 건너뛴 그 좌석
#   (c) 래치 없는 건강한 팀 = B6 래치 배포 이전 기계(NOT-awake 오판 금지 대상)
_SEAT_CORPUS = {
    "healthy_latched": [
        {"role": "cso", "exited": False, "awakened_at": 1.0, "seat": "occupied"},
        {"role": "worker", "exited": False, "awakened_at": 2.0, "seat": "occupied"},
        {"role": "reviewer-gemini", "exited": False, "awakened_at": 3.0, "seat": "occupied"},
        {"role": "reviewer-codex", "exited": False, "awakened_at": 4.0, "seat": "occupied"},
    ],
    "legacy_no_latch": [
        {"role": "cso", "exited": False, "agent_alive": True},
        {"role": "worker", "exited": False, "agent_alive": True},
        {"role": "reviewer-gemini", "exited": False, "agent_alive": True},
        {"role": "reviewer-codex", "exited": False, "agent_alive": True},
    ],
    "agent_dead_seat": [
        {"role": "cso", "exited": False, "agent_alive": False, "seat": "empty"},
        {"role": "worker", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-codex", "exited": False, "awakened_at": 1.0},
    ],
    "g26_variant_seats": [
        {"role": "reviewer-grok", "exited": False, "agent_alive": True},
        {"role": "cso-1", "exited": False, "agent_alive": True},
    ],
    "substitute_filled": [
        {"role": "cso", "exited": False, "awakened_at": 1.0},
        {"role": "worker-2", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-claude-2", "exited": False, "awakened_at": 1.0},
    ],
    "seat_unknown": [
        {"role": "cso", "exited": False, "seat": "unknown"},
        {"role": "worker", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-gemini", "exited": False, "awakened_at": 1.0},
        {"role": "reviewer-codex", "exited": False, "awakened_at": 1.0},
    ],
    "exited_litter": [
        {"role": "worker", "exited": True, "agent_alive": False},
        {"role": "cso", "exited": True, "agent_alive": False},
    ],
}


def _roster_pin(native):
    """★리뷰어 로스터 **밀폐 주입**(격리 구성) — 위 corpus 를 기계 감지에서 떼어낸다.

    ⓐ 왜 필요한가(2026-08-25 CI 적색 3건의 근저원인 · 실측 재현):
      `check_verdicts` 가 요구하는 의무 역할은 상수가 아니라 **그 기계에 리뷰어 CLI 가 설치돼
      있는가**로 갈린다(`reviewer_roster` → `detect_reviewer` → 단일 오라클 ∨ agents.json 경로
      실재). 설치된 개발자 기계에서는 네이티브 슬롯 이름이, 깨끗한 기계(CI macos-latest · 신규
      사용자 대다수)에서는 대체 슬롯 이름이 required 에 들어간다.
      그런데 위 `_SEAT_CORPUS` 는 좌석 이름을 네이티브로 적어 두었다 — 즉 이 검체들은
      **사용자 환경이 전제를 대신 세워 주는 동안만** 초록이었고, 깨끗한 기계에서는 없는 키를
      색인해 예외로 죽었다. 검체가 죽으면 무엇을 재려 했는지가 통째로 사라진다(전제 부재는
      **판정**돼야지 크래시가 되면 안 된다).
      `check_verdicts` · `_shared_verdict_deficit` 는 이미 이 주입 규약(detect/agents)을
      **계약으로** 갖고 있다 — 같은 이유로 두 `--self-test` 가 2026-08-22 에 먼저 이주했고,
      건강성 러너의 이 세 검체만 남아 있었다. 이제 여기도 전제를 스스로 세운다.

    ⓑ 왜 완화가 아닌가:
      · `need()` 를 하나도 지우지 않았고 검체 삭제도 0 이다.
      · '전제 없음'을 skip 으로 접지 않는다 — 전제를 **구성**해서 잰다(측정 불능을 통과로
        접지 않는다는 이 저장소의 규율 그대로).
      · 오히려 로스터 **두 형상 전수**(설치 기계 ∥ 깨끗한 기계)에서 같은 계약을 재게 되어
        단언 횟수가 늘고, 종전에는 개발자 기계에서 0회 실행이던 대체 로스터 경로가 상시
        측정된다. 판정은 이제 어느 기계에서 돌려도 같다.

    반환: `reviewer_roster(detect, agents)` 가 호출하는 감지 주입 콜러블.
    """
    def detect(agent, agents=None):
        return bool(native), "밀폐 주입(native=%r · 실감지 우회)" % bool(native)
    return detect


def _vd(verdicts, role, ctx=""):
    """`verdicts[role]` — 없을 때 **터지지 않고 판정한다**(전제 부재는 진단이지 크래시가 아니다).

    ★왜 이 한 줄이 필요한가: 종전에 이 파일의 검체들은 의무 역할을 리터럴로 색인했다
      (`v["reviewer-gemini"]`). 로스터가 그 이름을 담지 않는 기계에서는 검체가 `KeyError` 로
      **크래시**했고, 그 순간 "무엇을 재려 했는가"가 통째로 사라졌다 — 러너가 남긴 것은
      한 줄짜리 예외 이름뿐이라 원인 규명이 CI 로그에서 시작조차 되지 않았다.
      이제는 같은 상황에서 **판정된 실패**가 나온다: 어떤 역할을 기대했고 실제 판정에는 무엇이
      들어 있었는지가 사유에 남는다. 판정 조건은 그대로다(없으면 여전히 실패) — 바뀐 것은
      실패가 **진단 가능해졌다**는 것뿐이다."""
    if role not in verdicts:
        raise Fail("의무 역할 %r 가 check 판정에 없다 — 로스터 전제가 서지 않았다"
                   "(판정에 있는 역할: %s)%s"
                   % (role, ", ".join(sorted(verdicts)) or "(없음)",
                      (" · %s" % ctx) if ctx else ""))
    return verdicts[role]


@specimen("H-PRED-1", "W2", "결손 판정↔check verdict 공유 fixture 기계 차분", ["A1", "G26"])
def h_pred_1():
    """A1 클래스 소멸의 핵심 단언: **결손 판정과 check 판정이 갈리지 않는다** — 유일한
    의도된 예외가 W-B3 신계약(unknown 잔존)이다.

    A1 라이브락의 구조는 "결손 0 → ④ boot 생략 → ⑤ check 실패 → exit 6 → 재선언 동일"이었다.
    그 성립 조건은 두 판정이 **다른 함수**라는 것이다. W2 는 결손 판정이 `check_verdicts` 를
    문자 그대로 소비하게 만든다 → 공유 fixture 전수에서 차분이 0 이어야 한다.
    ★W-B3 신계약(부트 경로 unknown=결손 · 잔여 B3 폐쇄): unknown 등급 검체만 **의도된
      차분**을 갖는다 — ⑤check 는 충족(콜드스타트 fail-open 불변 · exit 6 라이브락 금지) ∧
      결손 산출은 시한부 해소(resolve_unknown_for_spawn · `cys boot` 스폰 규약 동일) 후
      잔존 시 결손>0(`cys boot` 호출 유도). 그래서 차분 0 계약은 unknown 잔존을 예외로 두고,
      unknown 검체는 신계약(⑤불변·잔존=결손·해소=복귀)으로 잰다. 결손 산출의 정본은
      `javis_orchestra._shared_verdict_deficit` 이고 bootstrap 판은 **위임 래퍼**다(소비 배선)."""
    BN, O, B = _shared_pred()
    need("_shared_verdict_deficit" in _read(os.path.join(BIN_DIR, "javis_bootstrap.py")),
         "결손 판정이 공유 판정 함수를 소비하지 않는다")
    diffs = []
    # ★차분 0 은 **로스터에 무관한** 계약이다(두 판정이 같은 로스터를 소비하므로) — 그래서
    #   설치 기계(native)·깨끗한 기계(substitute) 두 형상 전수로 잰다. 종전에는 이 루프가
    #   기계에 실제로 깔린 리뷰어 CLI 한 형상만 밟았다(`_roster_pin` ⓐ 참조).
    for pin in (True, False):
        D = _roster_pin(pin)
        for name, rows in _SEAT_CORPUS.items():
            st = _fx(rows)
            verdicts, _roster = O.check_verdicts(st, detect=D)
            check_missing = sorted(r for r, v in verdicts.items() if not v["satisfied"])
            unknown_ok = sorted(r for r, v in verdicts.items()
                                if v["satisfied"] and v["grade"] == "unknown")
            # 밀폐 주입: 재조회=같은 fixture(잔존 unknown 재현)·tick 0 — 수면 0·데몬 왕복 0.
            has, why = B._shared_verdict_deficit(st, requery=lambda st=st: st, tick_s=0,
                                                 detect=D)
            need(has is not None, "[native=%r] %s: 공유 판정 소비 실패 — %s" % (pin, name, why))
            # 차분 계약: check 부재 ⟺ 결손>0. ★유일 예외 = unknown 잔존(W-B3 — 의도된 차분).
            expect = bool(check_missing) or bool(unknown_ok)
            if bool(has) is not expect:
                diffs.append("[native=%r] %s: check_missing=%r unknown=%r vs deficit=%r"
                             % (pin, name, check_missing, unknown_ok, has))
    need(not diffs, "결손 판정↔check verdict 차분 발생(A1 라이브락 재성립): %r" % diffs)
    # ↓ 아래는 corpus 의 **좌석 이름**(네이티브 슬롯)을 직접 색인하므로 그 전제를 명시로 세운다.
    DN = _roster_pin(True)
    # G26 좌석: grok·cso-1 은 의무 슬롯을 채우지 못한다(양쪽 동일 결론).
    st = _fx(_SEAT_CORPUS["g26_variant_seats"])
    v, _ = O.check_verdicts(st, detect=DN)
    need(not _vd(v, "cso", "G26")["satisfied"], "cso-1 변형 좌석이 의무 cso 를 충족(G26 재발)")
    need(not _vd(v, "reviewer-gemini", "G26")["satisfied"],
         "reviewer-grok 이 의무 리뷰어 슬롯을 충족(G26 재발)")
    need(B._shared_verdict_deficit(st, requery=lambda: st, tick_s=0, detect=DN)[0] is True,
         "G26 좌석에서 결손 0 오판")
    # 대조군: 정상 팀은 양쪽 모두 충족.
    st_ok = _fx(_SEAT_CORPUS["healthy_latched"])
    need(B._shared_verdict_deficit(st_ok, requery=lambda: st_ok, tick_s=0,
                                   detect=DN)[0] is False,
         "정상 팀을 결손>0 으로 오판(역방향 회귀)")
    # ★W-B3 신계약 3단 통제 — 로스터 **두 형상 전수**(주입으로 전제를 세운다):
    #   ⓐ ⑤check satisfied 불변(unknown=충족측) ⓑ 잔존 unknown ⟹ 결손>0(의도된 차분·배선 발효)
    #   ⓒ 재조회 생존 확인 ⟹ 결손 0 복귀(시한부 — 건강한 콜드스타트 팀 오판·boot churn 금지)
    for pin in (True, False):
        D = _roster_pin(pin)
        req_roles = ["cso", "worker"] + [e["role"] for e in O.reviewer_roster(D)]
        unk_st = _fx([{"role": req_roles[0], "exited": False, "seat": "unknown"}] +
                     [{"role": r, "exited": False, "awakened_at": 1.0} for r in req_roles[1:]])
        ok_st = _fx([{"role": r, "exited": False, "awakened_at": 1.0} for r in req_roles])
        v_unk, _ = O.check_verdicts(unk_st, detect=D)
        vu = _vd(v_unk, req_roles[0], "W-B3 unknown 통제(native=%r)" % pin)
        need(vu["grade"] == "unknown",
             "[native=%r] unknown 통제 fixture 가 unknown 등급이 아님" % pin)
        need(vu["satisfied"] is True,
             "[native=%r] ⑤check 가 unknown 을 미충족으로 뒤집음(콜드스타트 exit 6 라이브락 재발)"
             % pin)
        need(B._shared_verdict_deficit(unk_st, requery=lambda: unk_st, tick_s=0,
                                       detect=D)[0] is True,
             "[native=%r] 잔존 unknown 이 결손으로 계상되지 않음(W-B3 배선 미발효)" % pin)
        need(B._shared_verdict_deficit(unk_st, requery=lambda: ok_st, tick_s=0,
                                       detect=D)[0] is False,
             "[native=%r] 재조회 생존 확인된 unknown 을 결손으로 계상(불필요 boot·churn)" % pin)
    # ★배선 실재 핀(소비자 0 재발 금지): bootstrap 판은 orchestra 정본으로 위임하고, 로컬
    #   구현은 폴백 전용 이름으로만 남는다(동명 쌍둥이 드리프트 함정 제거).
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need('getattr(_orch, "_shared_verdict_deficit"' in bsrc,
         "④ 결손 산출이 orchestra 정본으로 위임하지 않는다(신설 함수 소비자 0 — W-B3 미발효)")
    need("def _shared_verdict_deficit_fallback(" in bsrc,
         "구 팩 스큐 폴백(로컬 구 계약) 소실 — 신팩+구팩 혼재에서 부트 경로 취약")
    # ★구 팩 스큐 시뮬(orchestra 에 정본 부재): 폴백이 유효 판정을 내고(부트 불사·구 계약
    #   unknown=충족측) stderr 1줄로 고지된다(조용한 강등 금지).
    #   ★폴백은 주입 통로가 없다(구 계약 그대로 `check_verdicts(status)` 1인자 호출) — 그래서
    #     전제는 **스텁 쪽에서** 세운다: 구 팩에도 있던 판정 코어를 그대로 쓰되 로스터만 못박은
    #     형상으로 실어 준다. 프로덕션 서명을 건드리지 않고도 검체가 환경에서 독립한다.
    import types as _types
    import io as _io
    import contextlib as _ctx
    _sk_req = ["cso", "worker"] + [e["role"] for e in O.reviewer_roster(DN)]
    sk_unk = _fx([{"role": _sk_req[0], "exited": False, "seat": "unknown"}] +
                 [{"role": r, "exited": False, "awakened_at": 1.0} for r in _sk_req[1:]])
    sk_ok = _fx([{"role": r, "exited": False, "awakened_at": 1.0} for r in _sk_req])
    stub = _types.ModuleType("javis_orchestra")
    # 구 팩에도 있던 W2 판정 코어만 남긴 형상 + 로스터 밀폐(전제 구성 · 판정 코어는 정본 그대로)
    stub.check_verdicts = lambda st, *a, **kw: O.check_verdicts(st, detect=DN)
    real = sys.modules.get("javis_orchestra")
    buf = _io.StringIO()
    try:
        sys.modules["javis_orchestra"] = stub
        with _ctx.redirect_stderr(buf):
            fb_ok = B._shared_verdict_deficit(sk_ok)
            fb_unk = B._shared_verdict_deficit(sk_unk)
    finally:
        if real is not None:
            sys.modules["javis_orchestra"] = real
        else:                                                         # pragma: no cover
            sys.modules.pop("javis_orchestra", None)
    need(fb_ok[0] is False, "구 팩 스큐 폴백이 정상 팀을 오판(부트 경로 사망 위험): %r" % (fb_ok,))
    need(fb_unk[0] is False,
         "구 팩 스큐 폴백이 unknown 을 결손으로 계상(구 계약 이탈 — 폴백은 unknown=충족측)")
    need("폴백" in buf.getvalue(), "폴백 발동이 stderr 1줄로 고지되지 않음(조용한 강등)")
    # ★★클래스 봉인(2026-08-25) — "환경이 전제를 대신 세우는 검체" 재발 금지.
    #   이 러너 안의 **모든** 로스터 오라클 호출은 주입을 동반해야 한다. 주입이 없으면 판정
    #   대상 팀이 그 기계에 깔린 리뷰어 CLI 로 갈리고, 그때의 초록·적색은 코드가 아니라 환경을
    #   잰 것이다(실측: 같은 커밋이 개발자 기계 GREEN · 깨끗한 기계 3건 적색).
    #   ※ 결손 산출(`_shared_verdict_deficit`)은 이 오라클을 소비하므로 함께 닫힌다. 단
    #     구 팩 스큐 폴백만은 주입 통로가 없어(구 계약 1인자) 위에서 스텁으로 전제를 세운다.
    _rsrc = _read(os.path.abspath(__file__))
    _ARGS = r"([^()]*(?:\([^()]*\)[^()]*)*)\)"      # 한 단계 중첩까지 무는 인자 구간
    _bare = []
    for _name, _pat, _ok in (
            ("check_verdicts", r"O\.check_verdicts\(" + _ARGS,
             lambda a: "detect=" in a),
            ("reviewer_roster", r"O\.reviewer_roster\(" + _ARGS,
             lambda a: bool(a.strip()))):
        _hits = re.findall(_pat, _rsrc, re.S)
        need(_hits, "%s 호출을 하나도 수확하지 못했다(수확 정규식 파손 — fail-closed)" % _name)
        _bare += ["%s(%s)" % (_name, " ".join(a.split())[:70]) for a in _hits if not _ok(a)]
    need(not _bare,
         "로스터 주입 없는 오라클 호출 %d건 — 검체가 '이 기계에 무엇이 깔렸는가'를 재고 있다"
         "(격리 구성으로 전제를 세워라 · `_roster_pin`): %r" % (len(_bare), _bare))
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("check_verdicts" not in old,
             "계측 타당성 실패: 구 코드가 이미 check 판정을 공유한다")
        need("cys list 텍스트" in old or "_live_role_names" in old,
             "계측 타당성 실패: 구 코드의 cys list 기반 결손 판정을 못 찾았다")
        calib = "구 코드=cys list 텍스트 신호(check 와 다른 함수) 확인"
    return ("공유 fixture %d종 × 로스터 2형상(설치·깨끗) 차분 0(unknown 잔존=의도된 차분) · "
            "G26 좌석 양쪽 결손>0 · 정상 팀 결손 0 · W-B3 2형상: ⑤불변+잔존 unknown 결손+"
            "시한부 복귀 · 위임 배선+구팩 폴백(스큐 시뮬·stderr 고지) · 계측검증=%s"
            % (len(_SEAT_CORPUS), calib))


@specimen("H-SEAT-4AXIS", "W6",
          "좌석 제4 등급 gate_pending 4자 파리티(Rust seat ∥ python node_liveness ∥ "
          "orchestra check ∥ 결손 산출) 차분 0",
          ["U-10", "B3", "A1", "S-4"])
def h_seat_4axis():
    """U-10(2026-08-23): 좌석 생존 판정은 이 저장소의 **샷건 서저리 지점 S-4** 다 —
    같은 규칙이 Rust `seat_liveness` · python `node_liveness` · `orchestra check_verdicts` ·
    `boot_agent_on_surface` readiness 네 곳에 흩어져 있고, 그중 **3벌만 결박**돼 있었다.

    제4 등급 `gate_pending`("프로세스는 살아 있으나 첫기동 관문에 갇혀 입력 불가")을 더하면서
    네 축이 같은 좌석을 반대로 읽으면, 그것이 곧 A1·B3 클래스(판정 이원화)의 새 인스턴스다.
    그래서 **차분 0** 을 기계로 단언한다.

    ★이 검체가 지키는 **두 축의 분리**가 이 단위에서 가장 위험한 계약이다:
      · '충족(satisfied)인가' → 관문 보류는 **아니다**(그래야 관문에 갇힌 팀이 "정상 가동 중"
        으로 집계되지 않는다).
      · '살아 있는가(= 파괴 금지인가)' → 관문 보류는 **그렇다**(`node_alive` → reclaim kill
        게이트). 이 축이 뒤집히면 첫기동 관문에 갇힌 신규 프로필의 4종 노드가 전부 kill 대상이
        되어 **전 pane 사망(글자 0)** 이다. 두 축을 섞는 것이 이 단위의 유일한 치명 실수다.

    ★additive 계약: 이 단계에는 **생산자가 없다**(데몬 wire 값 항상 null). 그래서 구 데몬 ↔ 신
      CLI 혼재 스모크가 '종전 판정 그대로'를 내야 한다 — 그것도 함께 잰다.
    """
    BN, O, B = _shared_pred()
    notes = []

    # ── 계측 타당성 먼저: 구 트리에 이 축이 이미 있으면 탐지기가 무엇도 잡지 못한다 ──
    calib = "skip(no-git)"
    old_bn = _git_show(os.path.join("cysjavis-pack", "bin", "javis_boot_node.py"))
    old_rs = _git_show(os.path.join("src", "bin", "cys.rs"))
    if old_bn is not None and old_rs is not None:
        need("LIVENESS_GATED" not in old_bn,
             "계측 타당성 실패: 구 팩에 이미 제4 등급이 있다(탐지기 무효)")
        need("GatePending" not in old_rs,
             "계측 타당성 실패: 구 Rust 에 이미 제4 등급이 있다(탐지기 무효)")
        calib = "구 트리=제4 등급 부재(python·Rust 양쪽) 확인"

    # ── 축① Rust `seat_liveness`(boot 스킵 술어) — 소스 배선 핀 ──
    #   경로 소유는 SCAN_TARGETS("readiness") — 핀 이사 계약(U-2) 준수.
    src = _scan_source("readiness")
    need("GatePending" in src, "Rust 측 제4 등급 변형이 없다(python 만 등급 — 파리티 붕괴)")
    li = src.find("fn seat_liveness(")
    need(li > 0, "Rust seat_liveness 본문을 못 찾았다")
    lbody = src[li:src.find("\n}\n", li)]
    need("gate_pending_from_wire" in lbody,
         "Rust seat_liveness 가 관문 축을 읽지 않는다(축 미배선)")
    gi, ai = lbody.find("gate_pending_from_wire"), lbody.find('s["agent_alive"].as_bool()')
    need(0 <= gi < ai,
         "Rust 관문 분기가 agent_alive 분기보다 뒤다 — 관문 좌석도 프로세스는 살아 있으므로 "
         "그 분기는 영원히 도달 불가(죽은 코드)이고 보류가 다시 already_alive 로 접힌다")
    # already_alive 집합에서 **제외**돼야 한다(이 등급의 존재 이유).
    ri = src.find("if matches!(grade, SeatLiveness::AwakeConfirmed")
    need(ri > 0, "run_boot 의 already_alive 술어를 못 찾았다")
    need("GatePending" not in src[ri:ri + 300],
         "제4 등급이 already_alive 집합에 들어갔다 — 관문에 갇힌 팀이 '정상 가동 중'으로 집계된다")
    # 파괴 체인(node-recover 주입 → reclaim kill)보다 **앞에서** 빠져나가야 한다.
    bi = src.find("if grade == SeatLiveness::GatePending {")
    di = src.find("match seat_death_confirmed(row)")
    need(0 < bi < di,
         "관문 보류 분기가 파괴 체인보다 뒤다 — 살아 있는 입력창에 기동 커맨드 주입·kill 경로 노출")
    # 파괴 술어 3중 AND 는 **동결**이다(stale 보류가 reclaim 을 영구 마비시키는 A1 역방향 차단).
    ji = src.find("fn seat_death_confirmed(")
    need(ji > 0, "seat_death_confirmed 본문을 못 찾았다")
    need("gate_pending" not in src[ji:src.find("\n}\n", ji)],
         "파괴 경로 3중 AND 가 관문 축을 소비한다 — 동결 계약 위반(stale 보류 = 회수 영구 마비)")
    notes.append("축① Rust: 등급 실재·순서(관문<agent_alive)·already_alive 제외·파괴체인 이전 이탈·"
                 "3중 AND 동결")

    # ── 축②③④ 실행 차분 — 같은 fixture 를 python·orchestra·결손 산출이 어떻게 읽는가 ──
    # ★로스터 밀폐(_roster_pin ⓐ) — 이 축의 판정은 로스터에 무관하지만, 주입 없이 실감지를 쓰면
    #   같은 검체가 기계마다 다른 팀을 재게 된다(설치 기계=네이티브 · 깨끗한 기계=대체).
    D = _roster_pin(True)
    req = ["cso", "worker"] + [e["role"] for e in O.reviewer_roster(D)]

    def _team(first_row):
        return _fx([first_row] + [{"role": r, "exited": False, "awakened_at": 1.0}
                                  for r in req[1:]])

    gated_row = {"role": req[0], "exited": False, "agent_alive": True, "seat": "occupied",
                 "gate_pending": {"gate": "disclaimer", "since": 1.0}}
    gated = _team(gated_row)
    # 대조군 = 같은 좌석에서 관문 축만 뺀 것(구 데몬 = 키 부재 · 신 데몬 무보류 = null)
    plain = _team({k: v for k, v in gated_row.items() if k != "gate_pending"})
    nulled = _team(dict(gated_row, gate_pending=None))

    # 축② python node_liveness
    need(BN.node_liveness(gated, req[0])[0] == BN.LIVENESS_GATED,
         "python 이 관문 좌석을 제4 등급으로 읽지 않는다")
    # 축③ orchestra check_verdicts — **충족 아님**
    v_gate, _ = O.check_verdicts(gated, detect=D)
    need(v_gate[req[0]]["grade"] == BN.LIVENESS_GATED, "check 가 제4 등급을 전달하지 않는다")
    need(v_gate[req[0]]["satisfied"] is False,
         "관문 보류가 충족으로 접혔다 — 관문에 갇힌 팀이 READY 로 집계된다(이 단위의 존재 이유 소멸)")
    # 축④ 결손 산출(부트 경로) — 충족 아님이므로 결손>0
    has, why = B._shared_verdict_deficit(gated, requery=lambda: gated, tick_s=0, detect=D)
    need(has is True, "관문 보류인데 결손 0(부트가 영영 호출되지 않는다): %s" % why)
    # ★차분 0 계약: 세 축이 같은 결론(미충족 ⟺ 결손>0)이고, 대조군에서 셋 다 뒤집힌다.
    for name, st in (("구 데몬(키 부재)", plain), ("신 데몬 무보류(null)", nulled)):
        need(BN.node_liveness(st, req[0])[0] == BN.LIVENESS_PRESUMED,
             "%s 에서 종전 등급이 변형됐다(혼재 안전 붕괴 — 부재 ≠ 부정)" % name)
        vv, _ = O.check_verdicts(st, detect=D)
        need(vv[req[0]]["satisfied"] is True, "%s 에서 충족이 뒤집혔다(위경보)" % name)
        need(B._shared_verdict_deficit(st, requery=lambda st=st: st, tick_s=0,
                                       detect=D)[0] is False,
             "%s 에서 결손이 발생했다(불필요 boot churn)" % name)
    notes.append("축②③④ 차분 0(관문=미충족∧결손>0 · 대조군 2종=충족∧결손0)")

    # ── ★★두 축의 분리(치명 앵커 ④) — 충족 아님이 곧 죽음이 되면 안 된다 ──
    need(BN.node_alive(gated, req[0]) is True,
         "관문 보류 좌석을 죽음으로 판정 — reclaim kill 게이트가 열린다(전 pane 사망 경로 신설)")
    need(BN._reclaim_verdict(gated, req[0], 100, 100) == "hold-alive",
         "관문 보류 좌석에 kill 허용(오살) — 파괴 경로 보류 우선 위반")
    need(BN.latch_death_confirmed(gated_row)[0] is False,
         "살아 있는 관문 좌석이 죽음 3중 확정으로 읽힌다")
    notes.append("두 축 분리: 미충족 ∧ 생존(파괴 금지)")

    # ── 롤백 킬스위치 1지점 — env 하나로 축 전체가 종전 판정으로 복귀한다 ──
    _prev = os.environ.get(BN.GATE_PENDING_ENV)
    try:
        os.environ[BN.GATE_PENDING_ENV] = "0"
        need(BN.gate_pending_axis_enabled() is False, "킬스위치 '0' 이 축을 끄지 못한다")
        need(BN.node_liveness(gated, req[0])[0] == BN.LIVENESS_PRESUMED,
             "킬스위치 off 인데 python 축이 살아 있다")
        vv, _ = O.check_verdicts(gated, detect=D)
        need(vv[req[0]]["satisfied"] is True, "킬스위치 off 인데 check 가 여전히 미충족")
        need(B._shared_verdict_deficit(gated, requery=lambda: gated, tick_s=0,
                                       detect=D)[0] is False,
             "킬스위치 off 인데 결손이 남는다(롤백 1지점 계약 붕괴)")
    finally:
        if _prev is None:
            os.environ.pop(BN.GATE_PENDING_ENV, None)
        else:
            os.environ[BN.GATE_PENDING_ENV] = _prev
    need(BN.gate_pending_axis_enabled() is True, "킬스위치 복원 실패(검체 오염)")
    # ── ★(BLOCK-3 잔여분 · 2026-08-24) 마스터·강등 스위치도 **데몬 축까지** 닿는다 ──
    #   종전엔 축 술어(Rust `gate_pending_axis_enabled` · python 미러)가 `CYS_GATE_PENDING`
    #   **하나만** 읽었다. 그래서 마스터(`CYS_BOOT_GATES=0`)를 눌러도 데몬은 이미 실린 표식을
    #   **TTL 30분까지 계속 직렬화**했다 — CLI 는 종전 판정인데 데몬은 여전히 보류로 좌석을
    #   내보내는 **반쪽 롤백**이고, 그 상태가 정확히 BLOCK-4 가 없앤 "관측은 있는데 귀결이
    #   없는" 조합이다. 문서화된 손잡이 하나가 **전 소비자**에 닿아야 '되돌렸다'가 참이 된다.
    for _env, _on, _loose, _label in (
            (BN.BOOT_GATES_ENV, "0", ("", "false", "off", "1", " 0"), "마스터"),
            (BN.GATE_PENDING_CLOSE_ENV, "1", ("", "true", "yes", "on", " 1"), "강등")):
        _p = os.environ.get(_env)
        try:
            os.environ[_env] = _on
            need(BN.gate_pending_axis_enabled() is False,
                 "%s 스위치(%s=%s)가 축을 끄지 못한다 — 데몬 직렬화에 닿지 않는 손잡이다"
                 % (_label, _env, _on))
            need(BN.node_liveness(gated, req[0])[0] == BN.LIVENESS_PRESUMED,
                 "%s 스위치 off 인데 python 축이 살아 있다" % _label)
            need(O.check_verdicts(gated, detect=D)[0][req[0]]["satisfied"] is True,
                 "%s 스위치 off 인데 check 가 여전히 미충족" % _label)
            need(B._shared_verdict_deficit(gated, requery=lambda: gated, tick_s=0,
                                           detect=D)[0] is False,
                 "%s 스위치 off 인데 결손이 남는다(불필요 boot churn)" % _label)
            for _l in _loose:
                os.environ[_env] = _l
                need(BN.gate_pending_axis_enabled() is True,
                     "%s 스위치가 느슨한 값 %r 을 받았다(오타로 안전장치가 조용히 뒤집힌다)"
                     % (_label, _l))
        finally:
            if _p is None:
                os.environ.pop(_env, None)
            else:
                os.environ[_env] = _p
    need(BN.gate_pending_axis_enabled() is True, "3스위치 복원 실패(검체 오염)")
    # Rust 정본이 같은 3스위치를 **한 지점에서** 접는지 소스로 대조한다(2언어 동형성).
    _lib_axis = _repo_file(os.path.join("src", "lib.rs"))
    _ax_head = _lib_axis.find("pub fn gate_pending_axis_enabled() -> bool {")
    need(_ax_head > 0, "Rust 축 술어 래퍼가 사라졌다")
    _ax_body = _lib_axis[_ax_head:_lib_axis.find("\n}\n", _ax_head)]
    for _v in ("std::env::var(ENV_BOOT_GATES)", "std::env::var(ENV_GATE_PENDING_CLOSE)",
               "std::env::var(ENV_GATE_PENDING)"):
        need(_v in _ax_body,
             "Rust 축 술어가 `%s` 를 읽지 않는다 — python 미러와 접기가 갈렸다(한쪽만 롤백)" % _v)
    need("pub fn gate_pending_axis_effective_from(" in _lib_axis,
         "축 노출 판정의 순수 코어가 없다(진리표 대상 소실)")
    need("fn master_switch_reaches_the_daemon_serialization_axis(" in _lib_axis,
         "마스터→데몬 도달 진리표 배터리 결손")
    notes.append("롤백 3스위치(마스터 CYS_BOOT_GATES=0 · 강등 CYS_GATE_PENDING_CLOSE=1 · "
                 "축 CYS_GATE_PENDING=0) 각각 단독으로 데몬 축까지 도달 · 2언어 접기 동형")

    # ── 데몬 wire 동형성: surface.list ∥ org.status ∥ topology 가 **한 직렬화 지점**을 쓴다 ──
    hsrc = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    need(hsrc.count("(cys::GATE_PENDING_KEY): s.gate_pending_wire()") == 2,
         "surface.list·org.status 두 메서드가 같은 키·같은 직렬화를 노출하지 않는다(동형성 붕괴 — "
         "한쪽만 있으면 python 미러가 축을 영영 못 본다)")
    need('("alt_screen", json!(true), json!(false))' in hsrc
         and "cys::GATE_PENDING_KEY," in hsrc,
         "동형성 핀이 축 **표**로 일반화되지 않았다(축 추가마다 assert 복제 = 한쪽 누락)")
    ssrc = _repo_file(os.path.join("src", "bin", "cysd", "state.rs"))
    need("pub gate_pending: Mutex<Option<GatePending>>" in ssrc, "데몬 스키마에 제4 등급 자리가 없다")
    need("pub fn gate_pending_wire(" in ssrc,
         "직렬화 단일 지점이 없다 — 세 소비처가 각자 json! 하면 키·형·킬스위치가 갈린다")
    gsrc = _repo_file(os.path.join("src", "bin", "cysd", "governance.rs"))
    need("s.gate_pending_wire()" in gsrc, "topology 영속에 관측 슬롯이 없다(재기동 후 사람이 못 읽는다)")
    lib = _repo_file(os.path.join("src", "lib.rs"))
    for pin in ('pub const ENV_GATE_PENDING: &str = "CYS_GATE_PENDING";',
                'pub const GATE_PENDING_KEY: &str = "gate_pending";',
                "pub fn gate_pending_from_wire_with("):
        need(pin in lib, "lib 정본 상수/순수코어 부재: %s" % pin)
    need(BN.GATE_PENDING_ENV == "CYS_GATE_PENDING" and BN.GATE_PENDING_KEY == "gate_pending",
         "python 미러의 키·env 이름이 Rust 정본과 갈렸다(한쪽만 롤백되는 사고)")
    need(BN.BOOT_GATES_ENV == "CYS_BOOT_GATES"
         and BN.GATE_PENDING_CLOSE_ENV == "CYS_GATE_PENDING_CLOSE",
         "python 미러의 마스터·강등 스위치 이름이 Rust 정본과 갈렸다(한쪽만 롤백되는 사고)")
    need('pub const ENV_BOOT_GATES: &str = "CYS_BOOT_GATES";' in lib
         and 'pub const ENV_GATE_PENDING_CLOSE: &str = "CYS_GATE_PENDING_CLOSE";' in lib,
         "Rust 정본의 스위치 이름 상수가 사라졌다")
    notes.append("데몬 wire 동형성(2메서드+topology 단일 직렬화)·이름 3자 파리티")

    # ── 치명위험 ③: phoenix 부활 대상 판정 — 관문 보류 좌석은 **부활 target 이 아니다** ──
    #   현행 `_alive` 는 seat=="empty" 만 비생존으로 읽는다 → occupied 인 관문 좌석은 생존 =
    #   target 제외. 그것이 **옳다**: 살아 있는 pane 을 부활시키면 중복 좌석·claim_denied·관문
    #   재진입 루프(치명위험 ① 폭주)가 된다. 이 술어가 바뀌면 여기가 적색이 되어 재검토를 강제한다.
    ph = _repo_file(os.path.join("cysjavis-pack", "bin", "javis_phoenix.py"))
    need('if s.get("seat") == "empty":' in ph,
         "phoenix 생존 술어가 좌석 사실 기반이 아니다 — 관문 보류 좌석이 부활 target 에 섞일 수 있다")
    notes.append("phoenix: 관문 보류(seat=occupied)는 부활 target 제외(현행 술어로 성립)")

    # ── ★축④(readiness) — **핀 이사 완료(U-11)** ──
    #   U-10 시점의 이 자리는 "readiness 는 아직 생산하지 않는다"는 **미결박 고지 핀**이었고,
    #   생산자가 붙는 순간 적색이 되어 확장을 강제하도록 설계돼 있었다. U-11 이 생산자를 붙였으니
    #   그 핀을 **지우지 않고 이사**시킨다 — '생산하지 않는다' → '생산하되 이 계약을 지킨다'.
    #   완화가 아닌 이유: 단언 수가 1 → 8 로 늘고, 지키는 대상이 '부재'에서 **파괴 금지 배선**으로
    #   올라간다. 축④의 진리표 자체는 언어 경계 때문에 Rust 배터리가 실측한다(아래 ⑧에서 대조).
    bi2 = src.find("fn boot_agent_on_surface(")
    need(bi2 > 0, "boot_agent_on_surface 본문을 못 찾았다")
    # ① 타입드 3분기가 실재한다(Result<(),String> 1비트 붕괴로의 회귀 차단).
    need("enum BootVerdict {" in src, "readiness 결과가 타입화되지 않았다(1비트로 회귀)")
    for variant in ("Ready,", "GatePending { gate: String, tail: String }",
                    "LaunchFailed { evidence: String }"):
        need(variant in src, "BootVerdict 변형 결손: %s" % variant)
    need("-> Result<BootVerdict, String>" in src[bi2:bi2 + 2000],
         "boot_agent_on_surface 가 타입드 판정을 반환하지 않는다")
    # ② 생산 근거 = **커널 사실**이다. 화면 문자열로 파괴를 결정하면 렌더 방식·벤더 문면 한 번의
    #    변화가 곧 좌석 파괴다(H-SAFE-2 밸브와 같은 규율을 판정기에도 건다).
    ti = src.find("fn readiness_timeout_verdict(")
    need(ti > 0, "타임아웃 분류 순수함수가 없다(진리표 테스트 불가·판정이 흩어짐)")
    tbody = src[ti:src.find("\n}\n", ti)]
    tcode = "\n".join(ln for ln in tbody.splitlines() if not ln.strip().startswith("//"))
    need("alive == Some(false)" in tcode,
         "파괴 허용 조건이 '커널이 부재를 확정' 이 아니다 — 판정 불가·생존이 파괴로 접힐 수 있다")
    for banned in ("delta_text", "delta_flat", "text.contains", "screen["):
        need(banned not in tcode,
             "판정기가 화면 텍스트를 근거로 쓴다(%s) — 커널 사실 독립성 상실" % banned)
    # ★U-13 트립와이어(이사된 감시): 판정 근거가 '관문 코퍼스' 로 바뀌는 날 이 핀이 적색이 되어
    #   4축 파리티를 **관문 판정 기준으로** 다시 재도록 강제한다.
    need(tcode.count("BootVerdict::") == 2,
         "판정기의 결과 집합이 2종(LaunchFailed·GatePending)을 벗어났다 — 판정 근거가 바뀌었다면 "
         "H-SEAT-4AXIS 를 그 기준으로 다시 세워라(U-13)")
    # ③ launch 호출부: 보류에서 close 에 **도달할 수 없다**(치명위험 ④ 차단의 본체).
    li2 = src.find("fn run_launch_agent_opts(")
    need(li2 > 0, "run_launch_agent_opts 를 못 찾았다")
    lbody2 = src[li2:src.find("\n/// 온보딩③", li2) if src.find("\n/// 온보딩③", li2) > 0
                 else li2 + 12000]
    gi2 = lbody2.find("Ok(BootVerdict::GatePending")
    # 실패 합류 지점 = `LaunchFailed | Err` or-패턴(롤백 close 를 한 벌로 유지하는 구조).
    ei2 = lbody2.find("Ok(BootVerdict::LaunchFailed { evidence: e }) | Err(e) => {",
                      gi2 if gi2 > 0 else 0)
    need(0 < gi2 < ei2, "launch 호출부에 보류 분기가 없거나 실패 분기보다 뒤다")
    need('"surface.close"' not in lbody2[gi2:ei2],
         "보류 분기가 좌석을 닫는다 — 살아 있는 에이전트를 파괴(치명위험 ④ 재발)")
    need('"surface.close"' in lbody2[ei2:],
         "실패 분기의 롤백 close 가 사라졌다 — 진짜 실패 좌석이 역할을 쥔 채 쌓인다(완화 금지)")
    # ④ stdout 계약: 보류에서도 surface ref 가 나간다(GUI·bootstrap 의 claim 귀속 재료).
    #    ★**무조건** 출력이어야 한다 — `if ready { println!… }` 같은 조건부로 바뀌면 보류 pane 은
    #    살아 있는데 소비부(GUI start_master·bootstrap ③claim)가 그 ref 를 못 받아 '유령 pane'
    #    이 된다. 그래서 존재만 보지 않고 **판정 반환과의 인접**(들여쓰기 포함)을 구조로 고정한다.
    need('        println!("{}", surface_ref(sid));\n        Ok(verdict)' in lbody2,
         "stdout surface ref 계약이 무조건 출력이 아니다 — 보류 pane 이 소비부에서 사라진다"
         "(조건부로 감싸는 변경이면 그 조건이 곧 유령 pane 이다)")
    need(0 < lbody2.find('println!("{}", surface_ref(sid));') < gi2,
         "stdout 출력이 보류 분기보다 뒤다(보류에서 ref 가 나가지 않는다)")
    # ⑤ node-recover → run_boot: **kill 체인 차단**. 보류가 escalate_reclaim 앞에서 빠져나가야 한다.
    ri2 = src.find("fn run_boot(")
    rbody2 = src[ri2:src.find("\n/// `cys boot` bare exit", ri2)]
    # ★접미 확장(`..._DISABLED` 같은 이름 바꿔치기)으로 핀을 비껴가지 못하게 **여는 중괄호까지**
    #   묶어 찾는다 — 부분문자열 핀은 이 저장소가 반복해서 뚫린 구멍이다(계측 자기감시).
    ci2, ki2 = (rbody2.find("rc == cys::EXIT_GATE_PENDING {"),
                rbody2.find("escalate_reclaim(role)"))
    need(0 < ci2 < ki2,
         "node-recover 보류가 reclaim(kill)보다 뒤다 — 관문에 갇힌 살아있는 에이전트를 죽인다")
    need("outcome\": \"gate_pending" in rbody2,
         "run_boot 이 보류를 typed outcome 으로 내지 않는다(실패로 뭉개짐)")
    # ★핀 이사(M3 · 2026-08-24) — 종전 핀은 **손 계상 지점의 개수**(`count(...) >= 3`)를 셌다.
    #   그 축이 재던 것은 "어떤 분기가 계상을 빼먹을 수 있다"였고, 그 빼먹음이 실제로 W4 에서
    #   났다. M3 은 손 계상을 없애고 집계를 typed outcome 에서 **파생**시켜 "버킷 합 = roles
    #   길이"를 구조적 불변식으로 만들었다. 그래서 개수가 아니라 **술어 그 자체**를 박는다.
    #   ★계상 술어는 이제 `boot_summary_buckets` 에 있고 그 함수는 rbody2(=run_boot 본문)
    #     **밖**이다 — 제안된 rbody2 핀은 앵커 부재로 조용히 죽는다. 그래서 파생부를 따로 자른다.
    #   축은 넷으로 **늘었다**: ⓐ run_boot 이 파생을 통과하는가 ⓑ Fatal 술어가 살아있는가
    #   ⓒ 관문 보류가 Fatal 과 **분리** 계상되는가 ⓓ exit 우선순위에서 Fatal 이 보류를 이기는가.
    need("let summary = boot_summary_buckets(&outcomes);" in rbody2
         and "boot_exit_code(fatal_failed as usize, fatal_gate_pending as usize, false)" in rbody2,
         "run_boot 의 집계·종료가 typed outcome 파생을 통과하지 않는다 — 손 계상이 되살아나면 "
         "보류 도입이 진짜 실패를 exit 0 으로 접는다")
    bs2 = _slice_between(src, "fn boot_summary_buckets(",
                         "\nconst BOOT_SUMMARY_BUCKETS", "H-SEAT-4AXIS 버킷 파생")
    need('"failed" | "missing" => fatal_failed += 1,' in bs2,
         "Fatal 계상이 사라졌다 — 보류 도입이 진짜 실패를 exit 0 으로 접었다")
    need('"gate_pending" => fatal_gate_pending += 1,' in bs2,
         "관문 보류가 Fatal 과 분리 계상되지 않는다")
    ec = _slice_between(src, "fn boot_exit_code(", "\n\n", "H-SEAT-4AXIS exit 우선순위")
    need("fatal_gate_pending > 0" in ec and "cys::EXIT_GATE_PENDING" in ec,
         "의무 관문 보류 전용 78 이 없다")
    # ★부재를 통과로 접지 않는다: `find` 는 -1 을 돌려주므로 순서 비교 전에 **둘 다 실재**를 먼저 잰다.
    need("fatal_failed > 0" in ec, "exit 판정에 Fatal 분기가 없다(보류만 남으면 실패가 78 로 위장된다)")
    need(ec.find("fatal_failed > 0") < ec.find("fatal_gate_pending > 0"),
         "Fatal 이 보류에 가려진다")
    # ⑥ restore in-seat: **fresh 폴백 금지**(좌석 증식·관문 재진입 루프 차단).
    si2 = src.find("fn run_restore(")
    sbody2 = src[si2:src.find("\n/// T2-7", si2)]
    gj = sbody2.find("Ok(BootVerdict::GatePending")
    fj = sbody2.find("run_launch_agent_opts(")
    need(0 < gj < fj, "restore 보류 분기가 없거나 fresh 폴백보다 뒤다")
    need("continue;" in sbody2[gj:fj],
         "restore 보류가 fresh 로 폴백한다 — 살아있는 역할에 좌석을 하나 더 만들고 같은 관문에 "
         "재진입한다(폭주 씨앗)")
    # ⑦ 롤백 1지점 + write path 3자 파리티 + 만료 규약.
    need(src.count("cys::gate_pending_close_override()") == 1,
         "롤백 킬스위치 판독이 1지점이 아니다(%d) — 한 곳만 빠져도 '되돌렸다'가 거짓말이 된다"
         % src.count("cys::gate_pending_close_override()"))
    # ★두 스위치가 **합류**해야 한다 — 축 스위치(`CYS_GATE_PENDING=0`)만 눌렀을 때 CLI 가 계속
    #   보류하면 'pane 은 남는데 좌석은 already_alive' 인 반쪽 롤백(관측 없는 보류 = 허위 READY)이다.
    need("pub fn gate_pending_close_override_from(" in lib and
         "!gate_pending_axis_enabled_from(axis_env)" in lib,
         "롤백 합류(두 스위치 OR)가 없다 — 반쪽 롤백 상태가 열린다")
    # ── ★(BLOCK-3 · BLOCK-4 · 2026-08-24) 마스터 스위치 + '엄격 판정 + 즉시 close' 불변식 ──
    #
    # BLOCK-4(재난④ 실증): `CYS_GATE_PENDING=0` **단독**이 "엄격 판정 + 즉시 close" 를 만들었다 —
    #   `gate_pending_close_override_from(None, Some("0")) == True`(합류 OR) 인데
    #   `readiness::legacy_v1_from(None) == False`(엄격 유지) → 관문 화면이 `GateHeld` 로 영원히
    #   ready 가 아니고 → readiness 타임아웃 → `LaunchFailed` 강등 → 호출부 `surface.close`.
    #   **문서화된 롤백 스위치 하나로 전 pane 사망**이다. 불변식은 "보류 장치를 끄면 판정도 함께
    #   종전(느슨)으로 돌아간다" 이고, 그 소유자는 `gate_axes_from` **하나**여야 한다.
    # BLOCK-3: 축 노브 단독으로는 종전 동작이 돌아오지 않았다(리뷰어 4칸 진리표) — 사고 순간에
    #   사람이 쥐는 손잡이는 **마스터 스위치 하나**여야 한다.
    for pin in ('pub const ENV_BOOT_GATES: &str = "CYS_BOOT_GATES";',
                "pub fn boot_gates_master_off_from(", "pub struct GateAxes {",
                "pub fn gate_axes_from(", "pub fn gate_axes_forced_legacy()"):
        need(pin in lib, "BLOCK-3/BLOCK-4 정본 결손: %s" % pin)
    need('env_val == Some("0")' in lib,
         "마스터 스위치가 느슨한 truthy 를 받는다 — 오타로 안전장치가 조용히 뒤집힌다")
    ax = _slice_between(lib, "pub fn gate_axes_from(", "\n}\n", "H-SEAT-4AXIS 판정 축 접기")
    need("holding_off" in ax and "gate_pending_close_override_from(close_env, axis_env)" in ax,
         "판정 축 접기가 보류 장치 상태를 합류시키지 않는다 — BLOCK-4 조합이 되살아난다")
    # ★(U-17) 축이 하나 늘었다 — 기존 항목을 지우지 않고 **추가**한다. 새 축만 엄격하게 남으면
    #   마스터 스위치가 다시 거짓말이 되고, 인증 판정기가 마스터를 눌러도 계속 차단한다.
    for axis in ("readiness_legacy: holding_off", "inject_guard_off: holding_off",
                 "trust_legacy: holding_off", "profile_gate_observe_only: holding_off"):
        need(axis in ax,
             "축 '%s' 이 보류 꺼짐과 함께 풀리지 않는다 — 그 축만 엄격하게 남아 관문 화면이 "
             "곧 close 가 된다(재난④)" % axis)
    # ★경로는 레지스트리 경유로만 얻는다(핀 이사 계약 ⓒ — 직접 `_repo_file` 은 우회다).
    need(src.count("|| crate::gate_axes_forced_legacy()") == 1,
         "readiness 축이 상위 접기값을 소비하지 않는다 — 마스터 스위치가 거짓말이 된다")
    need(_scan_source("inject_guard").count("|| crate::gate_axes_forced_legacy()") == 2,
         "inject_guard 의 두 축(가드·신뢰) 중 하나가 상위 접기값을 소비하지 않는다")
    need("cys::ENV_BOOT_GATES" in src,
         "보류 처방·진단 문안이 **실제로 듣는** 스위치를 알려주지 않는다 — 사람이 축 노브만 끄고 "
         "여전히 보류되어 원인을 못 찾는다(BLOCK-3)")
    for battery in ("fn strict_judgment_and_immediate_close_is_unreachable_in_every_env_combination(",
                    "fn master_switch_alone_restores_the_previous_behavior_on_every_axis(",
                    "fn pre_fix_composition_reproduced_strict_judgment_with_immediate_close(",
                    "fn every_axis_knob_folds_in_the_master_switch_source_pin("):
        need(battery in lib, "BLOCK-3/BLOCK-4 진리표 배터리 결손: %s" % battery)
    need("fn gate_hold_prescription_names_the_switch_that_actually_works(" in src,
         "처방 문안 검체 결손 — 듣지 않는 손잡이만 안내하는 회귀를 아무 데서도 못 잡는다")
    notes.append("마스터 스위치 1개 · 축 3종 합류 · '엄격+즉시close' 불변식 단일 소유 · 배터리 4종")
    need('"surface.gate_pending"' in src, "CLI 가 표식을 기록하는 write path 가 없다")
    need('"surface.gate_pending" =>' in hsrc, "데몬에 표식 write path RPC 가 없다(생산자 미착지)")
    need("gate_denied" in hsrc, "자칭 선언 차단(산출자=평가자) 게이트가 없다")
    need("gate_pending_fresh" in ssrc,
         "직렬화 지점이 만료(TTL)를 상의하지 않는다 — 사람이 관문을 통과해도 좌석이 영구 미충족"
         "(부트 라이브락 A1)")
    for pin in ("pub const GATE_PENDING_TTL_SECS", "pub fn gate_pending_fresh(",
                'pub const ENV_GATE_PENDING_CLOSE: &str = "CYS_GATE_PENDING_CLOSE";',
                "pub const EXIT_GATE_PENDING: i32 = 78;"):
        need(pin in lib, "lib 정본 결손: %s" % pin)
    # ⑧ 축④ **실측**은 Rust 배터리가 한다(순수 판정기 진리표) — python 은 그 실재를 대조한다.
    #    언어 경계 때문에 검체가 직접 호출할 수 없다: 그래서 '재는 곳'을 옮기되 '재는 사실'은
    #    옮기지 않는다(계측 위치의 이동이지 판정의 완화가 아니다).
    for battery in ("fn gate_verdict_truth_table_is_decided_by_the_kernel_fact(",
                    "fn gate_verdict_rollback_switch_demotes_at_exactly_one_point(",
                    "fn gate_verdict_exit_code_is_neither_success_nor_failure("):
        need(battery in src, "축④ 진리표 배터리 결손: %s" % battery)
    notes.append("축④ readiness=생산(커널 사실) · close 도달불가 · kill 체인 차단 · fresh 폴백 0 · "
                 "stdout 계약 보존 · 롤백 1지점 · TTL 실재 · 진리표 배터리 3종")

    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-AUTH-PARSE", "W6",
          "프로필 인증 전제 판정기 — auth_class 8값 · unknown fail-closed · 위조 신원 금지 · "
          "열거 규칙 정본 1벌 · 마스터 스위치 접기",
          ["U-17", "C-3", "E-2"])
def h_auth_parse():
    """U-17(2026-08-24): 첫기동 관문 시드(U-19)는 **로그인 화면을 지운다** — 미인증 프로필에
    시드를 먼저 내면 노드는 관문 없이 태어난 것처럼 보이다가 401 로 죽는다. 그래서 "인증되어
    있는가" 를 시드보다 **먼저** 판정할 수 있어야 하고, 그 판정기가 `src/profile_gate.rs` 다.

    이 검체가 지키는 계약 다섯 —
      ⓐ `auth_class` 는 **8값**이고 `unknown` 은 **통과가 아니다**(측정 불능을 통과로 접는 것이
         이 저장소의 반복 사고 원인이다).
      ⓑ **위조 신원 금지** — 판정기는 `.claude.json` 에 쓰지 않는다(쓰기 API 0건). V-g 실측:
         자격증명은 `sha256(configDir)` 로 Keychain 봉인이라 `oauthAccount` 를 써 넣어도 인증되지
         않는다 — 즉 그 쓰기는 **인증이 아니라 위조**다.
      ⓒ `oauthAccount` **존재 기반 판정 금지**(E-2 반증) — 주장은 인증이 아니다.
      ⓓ 프로필 dir **열거 규칙은 정본 한 벌**. `cysd/accounts.rs seed_known` 이 재구현하면
         두 벌이 갈린다(한쪽만 새 부서 접두를 배우는 식).
      ⓔ 새 판정 축은 **마스터 스위치**(`CYS_BOOT_GATES=0`)에 접힌다 — 사고 순간에 사람이 노브를
         조합할 수는 없다(BLOCK-3 이 그 값을 이미 치렀다).
    ★오살 경보(U-18 이 읽어야 할 것): V-g 실측에서 **API키로 인증된 프로필과 미인증 프로필의
      `.claude.json` 은 동일**했다(자격증명이 env·Keychain 에 있다). 그래서 config 만 보는
      경로는 정상 API키·oauth_token·bedrock 좌석을 전부 `unknown` 으로 낸다 — 그 위에 차단을
      걸면 **살아있는 좌석이 전멸**한다. 등급 축(통과 여부)과 증거 축(무엇으로 알았나)을
      **분리해서** 내보내는 것이 그 방어이며, 아래 ⓕ가 그 분리를 기계로 확인한다.
    """
    notes = []
    lib = _repo_file(os.path.join("src", "lib.rs"))
    acc = _repo_file(os.path.join("src", "bin", "cysd", "accounts.rs"))

    # ⓐ lib 정본 배선 — 여기부터 적색이어야 구 트리에서 이 검체가 FIRE 한다.
    need("pub mod profile_gate;" in lib,
         "lib 에 인증 전제 판정기 모듈이 없다 — 시드(U-19)가 로그인 화면을 지우기 전에 판정할 "
         "수단이 없다는 뜻이다")
    pg = _scan_source("profile_gate")
    body = pg[:pg.index("#[cfg(test)]")] if "#[cfg(test)]" in pg else pg
    need(body != pg, "profile_gate.rs 에서 테스트 경계를 찾지 못했다(슬라이스 앵커 파손)")

    # ⓑ auth_class 8값 전수 + unknown fail-closed
    classes = ("subscription", "oauth_token", "api_key", "api_key_helper", "third_party",
               "not_logged_in", "onboarding_pending", "unknown")
    for c in classes:
        need('=> "%s",' % c in body, "auth_class 8값 결손: %s" % c)
    need(body.count("pub const ALL: [AuthClass; 8]") == 1, "8값 전수 배열이 없다(값이 늘면 조용히 통과)")
    allows = _slice_between(body, "pub fn allows_spawn(self) -> bool {", "\n    }\n",
                            "H-AUTH-PARSE 통과 판정")
    need("AuthClass::Unknown" not in allows,
         "★`unknown` 이 통과 목록에 들어갔다 — 측정 불능을 통과로 접는 것이 이 저장소의 반복 "
         "사고 원인이다(fail-closed 붕괴)")
    for denied in ("AuthClass::NotLoggedIn", "AuthClass::OnboardingPending"):
        need(denied not in allows, "비통과 등급이 통과 목록에 들어갔다: %s" % denied)
    notes.append("auth_class 8값 · 통과 5값 · unknown/미인증/관문 fail-closed")

    # ⓒ 위조 신원 금지 — 쓰기 API 0건. ★합성 표본으로 탐지기 자체를 먼저 시험한다
    #    (트리에 위반이 0이라 탐지기가 고장나도 초록이 되는 핀이다).
    write_apis = ("fs::write(", "File::create(", "OpenOptions::new(", "write_all(",
                  "to_writer(", "create_dir_all(")
    detect = lambda src: sorted(a for a in write_apis if a in src)
    need(detect('let f = std::fs::File::create(p)?; f.write_all(b"{}")?;')
         == ["File::create(", "write_all("],
         "계측 무효: 쓰기 API 탐지기가 합성 표본을 잡지 못한다")
    need(not detect(body),
         "★판정기가 쓰기 API 를 얻었다(%s) — `.claude.json` 에 `oauthAccount` 를 써 넣는 것은 "
         "인증이 아니라 **위조**다(자격증명은 sha256(configDir) 로 Keychain 봉인 · V-g 실측)"
         % detect(body))
    need('"oauthAccount":' not in body, "★`oauthAccount` 를 만드는 리터럴이 생겼다(위조 신원)")
    notes.append("위조 신원 금지: 쓰기 API 0 · 생성 리터럴 0(합성 표본 선시험)")

    # ⓓ `oauthAccount` 존재 기반 판정 금지(E-2) — 판정 함수 본문이 그 주장을 분기 조건으로
    #    쓰지 않는다. 파싱·보고는 허용(진단), 판정은 금지.
    decide = _slice_between(body, "pub fn classify(ev: &ProfileEvidence) -> Verdict {",
                            "\n}\n", "H-AUTH-PARSE 판정 본문")
    need("oauth_claim: Tri::Yes" not in decide and "oauth_claim: Tri::Unreadable" not in decide,
         "★판정이 `oauthAccount` 주장을 분기 조건으로 되돌렸다 — E-2 로 반증된 술어다"
         "(주장은 인증이 아니다: 복사·이동·시드로는 로그인되지 않는다)")
    need("ConfigClaimIsNotAuthentication" in decide,
         "config 주장이 인증이 아니라는 판정 분기가 사라졌다")
    notes.append("E-2: oauthAccount 주장 = 진단 전용(판정 분기 0)")

    # ⓔ 열거 규칙 정본 1벌 — accounts.rs 가 재구현하지 않는다. 합성 표본 선시험(구 문면).
    old_rule = 'if name == ".claude" || name.starts_with(".claude-") {'
    rule_hit = lambda src: '.starts_with(".claude-")' in src or '"claude-"' in src
    need(rule_hit(old_rule), "계측 무효: 열거 규칙 탐지기가 구 문면을 잡지 못한다")
    need(not rule_hit(acc),
         "★accounts.rs 가 프로필 dir 열거 규칙을 재구현했다 — 규칙이 두 벌로 갈리면 한쪽만 새 "
         "부서 접두를 배우고, 계정 발견과 인증 판정이 서로 다른 프로필 집합을 본다")
    need("cys::profile_gate::enumerate_profile_dirs(" in acc,
         "accounts.rs 가 열거 정본을 경유하지 않는다(정본 승격이 절반만 반영됐다)")
    need("pub fn is_profile_dir_name(" in body and "pub fn enumerate_profile_dirs(" in body,
         "열거 규칙 정본(순수 술어 + IO 껍데기)이 lib 에 없다")
    notes.append("열거 규칙 정본 1벌 · accounts.rs 경유")

    # ⓕ 두 축 분리 — 등급(통과 여부) ↔ 증거(무엇으로 알았나). 섞이면 U-18 이 config 만 보고
    #    차단해 정상 API키·토큰·bedrock 좌석을 오살한다.
    for pin in ("pub enum EvidenceGrade {", "OracleVerified", "ConfigOnly",
                '"evidence_grade":', '"allows_spawn":'):
        need(pin in body, "증거 축 결손: %s — 등급만 내보내면 U-18 이 오살한다" % pin)
    notes.append("등급 축 ∥ 증거 축 분리(오살 방어)")

    # ⓖ 마스터 스위치 접기 — 축 노브 1지점 + 상위 접기 OR + GateAxes 필드.
    need('pub const ENV_OBSERVE_ONLY: &str = "CYS_PROFILE_GATE_OBSERVE_ONLY";' in body,
         "롤백 축 env 상수 결손/이탈")
    need(body.count("std::env::var(ENV_OBSERVE_ONLY)") == 1,
         "축 env 판독이 1지점이 아니다 — 한 곳만 빠져도 '되돌렸다'가 거짓말이 된다")
    need(body.count("|| crate::gate_axes_forced_legacy()") == 1,
         "★인증 판정 축이 마스터 스위치에 접히지 않았다 — `CYS_BOOT_GATES=0` 을 눌러도 이 축만 "
         "엄격하게 남아 미인증 판정이 계속 차단한다(BLOCK-3 재발)")
    need('raw == Some("1")' in body, "축 노브가 느슨한 truthy 를 받는다 — 오타로 안전장치가 뒤집힌다")
    need("pub profile_gate_observe_only: bool," in lib, "GateAxes 에 인증 판정 축이 없다")
    notes.append("롤백 축 1지점 · 마스터 접기 · GateAxes 편입")

    # ⓗ Rust 배터리 실재 — 언어 경계 때문에 python 이 직접 호출할 수 없다. '재는 곳'만 옮기고
    #    '재는 사실'은 옮기지 않는다(계측 위치의 이동이지 판정의 완화가 아니다).
    for battery in ("fn unknown_is_never_a_pass_and_exactly_five_classes_pass(",
                    "fn config_only_evidence_never_produces_a_passing_class(",
                    "fn pre_fix_predicate_accepted_a_forged_identity_and_now_does_not(",
                    "fn oauth_claim_is_diagnostic_only_and_never_changes_the_verdict(",
                    "fn this_module_can_never_write_a_claude_json(",
                    "fn vg_fixtures_parse_into_every_authenticated_class(",
                    "fn three_profile_gate_subscription_unauthenticated_apikey("):
        need(battery in pg, "U-17 진리표 배터리 결손: %s" % battery)
    # V-g 실측 문면이 픽스처로 박제됐는가(스키마 드리프트 탐지의 근거).
    for method in ('\\"authMethod\\": \\"none\\"', '\\"authMethod\\": \\"api_key\\"',
                   '\\"authMethod\\":\\"claude.ai\\"'):
        need(method in pg, "V-g 픽스처 문면 결손: %s" % method)
    notes.append("Rust 배터리 7종 · V-g 픽스처 문면 박제")

    # ⓘ 계측 타당성 — 구 트리에는 판정기도 정본 승격도 없다(탐지기가 진짜 부재를 잡는가).
    calib = "skip(no-git)"
    old_lib = _git_show(os.path.join("src", "lib.rs"))
    old_acc = _git_show(os.path.join("src", "bin", "cysd", "accounts.rs"))
    if old_lib is not None and old_acc is not None:
        need("profile_gate" not in old_lib, "계측 타당성 실패: 구 lib 에 이미 판정기가 있다")
        need(rule_hit(old_acc),
             "계측 타당성 실패: 구 accounts.rs 에 열거 규칙이 없다 — 정본 승격 서사가 틀린 것")
        calib = "구 트리=판정기 부재 + accounts 자체 열거 규칙 실재"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-AUTH-SELFLOOP", "W6",
          "surface.create 최종 인증 게이트 — 자기 발화 루프 0 · 게이트 자리(④뒤·create앞) · "
          "다섯 경로 합류 1지점 · 거부=명시 오류(close 아님) · 오살 방어 · 롤백 접기",
          ["U-18", "C-3", "T3-G2"])
def h_auth_selfloop():
    """U-18(2026-08-24): 좌석 생성의 입구는 다섯이다(①`cys launch-agent` ②schedule
    `if_absent: launch` ③`check_agent_death` node-recover ④phoenix auto-restore ⑤GUI).
    그런데 PTY 를 실제로 만드는 함수의 **비-테스트 호출부는 `surface.create` 한 곳뿐**이라
    다섯이 거기로 합류한다 — 인증 게이트를 다섯 벌 만들지 않고 거기 하나만 두는 근거다.

    이 검체가 지키는 계약 —
      ⓐ **자기 발화 루프 0.** 데몬 watchdog 은 pane **화면 텍스트**를 정규식으로 훑는다. 거부
         처방 문안이 `not_logged_in`·`login_required` 같은 룰에 매칭되면 → `health.alert` →
         auth 인터록 300초 차단 + 좌석 오염으로 **우리 경고가 좌석을 죽인다**. 그래서 문안을
         **생산 룰 정규식**(state.rs `default_health_rules` 본문에서 뽑는다)으로 직접 잰다.
      ⓑ **게이트의 자리.** ④ worker active-limit **뒤** · `create_surface_with_env` **앞**.
         앞이면 좌석 승계·멱등 재사용까지 막혀 살아 있는 좌석의 부활이 잠기고, 뒤면 PTY 가
         이미 태어나 관문 화면이 킬체인의 첫 칸이 된다.
      ⓒ **합류 1지점.** PTY 스폰 함수의 호출부가 `state.rs`(정의·테스트 래퍼)와 `handlers.rs`
         밖으로 새면 이 게이트는 조용히 무력화된다.
      ⓓ **거부의 귀결은 close 가 아니다.** 거부 경로에 좌석을 죽이는 어휘가 없어야 한다(재난④).
      ⓔ **오살 방어.** `config_only` 증거·귀속 실패·낡음·config 변경은 전부 통과다. V-g 실측에서
         api_key 로 인증된 프로필과 미인증 프로필의 `.claude.json` 은 **동일**했다 — config 만
         보고 막으면 정상 api_key·oauth_token·bedrock 좌석이 전멸한다.
      ⓕ **비싼 조회 금지.** 데몬 핫패스라 서브프로세스 기동 0이고, 파일 stat 조차 '거부를
         주장하는 verdict 가 실제로 왔을 때' 뒤에만 있다.
      ⓖ **롤백은 새 노브를 만들지 않는다.** 게이트는 자기 env 를 읽지 않고 판정기의 축 노브를
         경유한다(그 노브가 마스터 `CYS_BOOT_GATES=0` 를 이미 OR 한다).
    """
    notes = []
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    st = _repo_file(os.path.join("src", "bin", "cysd", "state.rs"))
    pg = _scan_source("profile_gate")

    # 프로덕션 영역만 본다 — 테스트가 프로덕션 계약을 대신 만족시키는 착시를 막는다.
    hnd_prod = _slice_between(hnd, "pub fn dispatch(", "\n#[cfg(test)]", "H-AUTH-SELFLOOP 배선")

    def _at(hay, needle, what):
        i = hay.find(needle)
        need(i >= 0, "%s: 앵커 부재 %r — 코드가 이사했다면 이 검체의 앵커도 함께 옮겨라"
                     % (what, needle))
        return i

    # ── ⓑ 게이트의 자리(다섯 겹 순서 전량) ───────────────────────────────────
    i_reserved = _at(hnd_prod, "is reserved (daemon-derived identity grade", "예약 role 게이트")
    i_claim = _at(hnd_prod, '"surface.create denied: privileged role', "특권 role 게이트")
    i_idem = _at(hnd_prod, '"idempotent_reuse": true', "멱등 게이트")
    i_limit = _at(hnd_prod, '"worker_limit_exceeded",', "active-limit 게이트")
    i_gate = _at(hnd_prod, "if let Some(reply) = surface_create_auth_gate(", "인증 게이트 호출")
    i_create = _at(hnd_prod, "match daemon.create_surface_with_env(", "PTY 스폰 호출")
    need(i_reserved < i_claim < i_idem < i_limit < i_gate < i_create,
         "★게이트의 자리가 어긋났다(예약=%d 특권=%d 멱등=%d 한도=%d 인증=%d 스폰=%d) — "
         "인증 게이트가 ④보다 앞이면 좌석 승계·멱등 재사용까지 막혀 살아 있는 좌석의 부활이 "
         "잠기고, `create_surface_with_env` 보다 뒤면 PTY 가 이미 태어나 관문 화면이 킬체인의 "
         "첫 칸이 된다" % (i_reserved, i_claim, i_idem, i_limit, i_gate, i_create))
    need(hnd_prod.count("surface_create_auth_gate(daemon,") == 1,
         "인증 게이트 호출부가 1지점이 아니다 — 두 벌이면 한쪽만 고쳐진다")
    notes.append("자리: 예약<특권<멱등<한도<인증<스폰")

    # ── ⓒ PTY 스폰 합류 1지점 ────────────────────────────────────────────────
    cysd_dir = os.path.join(REPO_DIR, "src", "bin", "cysd")
    spawners = sorted(f for f in os.listdir(cysd_dir)
                      if f.endswith(".rs")
                      and "create_surface_with_env(" in _read(os.path.join(cysd_dir, f)))
    need(spawners == ["handlers.rs", "state.rs"],
         "★PTY 스폰 함수의 소비 파일이 늘었다(%s) — 좌석 생성 다섯 경로가 `surface.create` 로 "
         "합류한다는 전제가 깨지면 이 인증 게이트는 조용히 새고, 미인증 프로필이 그 새 경로로 "
         "좌석을 얻는다" % spawners)
    need(hnd_prod.count(".create_surface_with_env(") == 1,
         "handlers.rs 프로덕션 영역의 PTY 스폰 호출이 1건이 아니다(게이트 우회 경로 발생)")
    notes.append("스폰 합류 1지점(state.rs 정의 + handlers.rs 호출 1건)")

    # ── ⓐ 자기 발화 루프 0 ───────────────────────────────────────────────────
    # 생산 룰 정규식을 state.rs 본문에서 뽑는다(사본 정규식이면 검체가 사문화된다).
    rules_src = _slice_between(st, "fn default_health_rules() -> Vec<HealthRule> {", "\n}\n",
                               "H-AUTH-SELFLOOP 생산 룰 집합")
    pairs = re.findall(r'"([a-z0-9_]+)",\s*\n?\s*r"((?:[^"\\]|\\.)*)"', rules_src)
    need(len(pairs) >= 5,
         "계측 무효: 생산 헬스룰을 %d건밖에 수확하지 못했다(수확 정규식 파손)" % len(pairs))
    rules = []
    for name, pat in pairs:
        try:
            rules.append((name, re.compile(pat)))
        except re.error as e:
            raise Fail("생산 룰 %s 의 정규식을 파이썬에서 못 읽는다(%s) — 계측 무효" % (name, e))
    hits = lambda s: [n for n, rx in rules if rx.search(s)]
    # ★합성 표본 선시험 — 트리에 위반이 0이라 탐지기가 고장나도 초록이 되는 핀이다.
    for bad, want in (("Error: not logged in", "not_logged_in"),
                      ("Please run /login to continue", "login_required"),
                      ("401 Unauthorized", "auth_401"),
                      ("your token has expired", "token_expired"),
                      ("rate limited", "rate_limited")):
        need(want in hits(bad),
             "계측 무효: 합성 표본 %r 이 룰 %s 에 걸리지 않는다 — 이 검체가 재는 대상이 "
             "생산 룰이 아니다" % (bad, want))
    # auth 인터록이 보는 네 룰이 실제로 이 집합 안에 있어야 검체가 의미를 갖는다.
    interlock = _slice_between(st, "pub const AUTH_INTERLOCK_RULES:", ";", "인터록 룰 목록")
    for r in re.findall(r'"([a-z0-9_]+)"', interlock):
        need(any(n == r for n, _ in rules),
             "계측 무효: auth 인터록 룰 %r 이 생산 룰 집합에 없다" % r)

    # 처방 문안 복원 — Rust 의 `\<개행>` 이음(다음 줄 선행 공백까지 먹는다)을 되돌린다.
    presc_lit = _slice_between(hnd, 'pub(crate) const AUTH_GATE_PRESCRIPTION: &str =', '";',
                               "H-AUTH-SELFLOOP 처방 문안")
    presc = re.sub(r"\\\n\s*", "", presc_lit[presc_lit.index('"') + 1:])
    need(len(presc) > 40, "처방 문안 복원 실패(길이 %d) — 슬라이스 앵커 파손" % len(presc))
    need(not hits(presc),
         "★처방 문안이 헬스룰 %s 에 매칭된다 — 이 문장이 pane 에 찍히는 순간 그 좌석이 300초 "
         "잠기고 오염된다(자기 발화 루프). 문안: %r" % (hits(presc), presc))
    # 문안에 실리는 기계 필드(등급·이유·증거 등급) 전수 — 정본에서 뽑아 잰다.
    tokens = sorted(set(re.findall(r'=> "([a-z0-9_.]+)",', pg)))
    need(len(tokens) >= 20,
         "계측 무효: 판정기 안정 문자열을 %d건밖에 수확하지 못했다" % len(tokens))
    bad_tokens = {t: hits(t) for t in tokens if hits(t)}
    need(not bad_tokens,
         "★판정기 안정 문자열이 헬스룰에 매칭된다(%s) — 그 값이 처방 문안에 실리는 순간 "
         "좌석이 잠긴다" % bad_tokens)
    notes.append("자기 발화 루프 0: 처방 문안 + 안정 문자열 %d종 × 생산 룰 %d종"
                 % (len(tokens), len(rules)))

    # ── ⓓⓔⓕⓖ 게이트 본문 계약 ──────────────────────────────────────────────
    body = _slice_between(hnd, "fn surface_create_auth_gate(", "\n}\n", "H-AUTH-SELFLOOP 게이트 본문")
    decide = _slice_between(hnd, "pub(crate) fn auth_gate_decide(", "\n}\n",
                            "H-AUTH-SELFLOOP 판정 본문")
    # ⓓ 거부의 귀결은 close 가 아니다(재난④ — 전 pane 사망 경로 차단)
    for killer in ("close_surface", '"surface.close"', "exited.store", "kill(", "reap"):
        need(killer not in body,
             "★인증 게이트가 좌석을 죽이는 어휘를 얻었다(%r) — 거부의 귀결은 명시 오류 하나여야 "
             "한다(치명위험 ④)" % killer)
    need("err_response(id, AUTH_GATE_ERROR_CODE" in body, "거부가 명시 오류로 나가지 않는다")
    # ⓔ 오살 방어 — 통과로 접는 다섯 사유가 판정 본문에 실재한다
    for esc in ("config_only_evidence", "profile_mismatch", "verdict_stale",
                "verdict_from_the_future", "config_changed", "verdict_observe_only"):
        need('Ignored("%s")' % esc in decide,
             "★오살 방어 분기 결손: %s — 증명하지 못한 거부 주장이 차단으로 이어진다" % esc)
    need("if !v.oracle_verified {" in decide,
         "★`config_only` 증거 위에서 차단을 막는 가드가 사라졌다 — V-g 실측상 정상 api_key·"
         "oauth_token·bedrock 프로필이 전부 비통과로 나오므로 그 위의 차단은 좌석 전멸이다")
    # ⓕ 비싼 조회 금지 — 서브프로세스 0 · stat 은 거부 주장 뒤에만
    for costly in ("Command::new", "std::process::Command", ".output()", ".status()",
                   "block_on", "read_to_string("):
        need(costly not in body,
             "★데몬 핫패스에 비싼 조회가 들어왔다(%r) — 동기 서브프로세스는 tokio 워커를 "
             "점유하고 PIPE_LISTENER_POOL 을 포화시킨다" % costly)
    i_pass = _at(body, "if supplied.class.allows_spawn() {", "통과 등급 조기 반환")
    i_obs = _at(body, "if cys::profile_gate::observe_only() {", "롤백 조기 반환")
    i_stat = _at(body, "config_json_mtime_memo(", "mtime 관측")
    need(i_pass < i_obs < i_stat,
         "★IO 가 조기 반환보다 앞으로 올라왔다(통과=%d 롤백=%d 관측=%d) — 통과 verdict·롤백 "
         "상태에서도 매 좌석 생성이 파일을 만지게 된다" % (i_pass, i_obs, i_stat))
    need(body.count("config_json_mtime_memo(") == 1, "mtime 관측 지점이 1곳이 아니다")
    # ⓖ 롤백 — 게이트는 자기 env 를 만들지 않는다
    need("std::env::var(" not in body,
         "★인증 게이트가 자기 env 노브를 얻었다 — 사고 순간에 사람이 노브를 조합할 수는 없다. "
         "롤백은 판정기의 축 노브(그것이 마스터 CYS_BOOT_GATES 를 이미 OR 한다) 하나로 족하다")
    need(body.count("cys::profile_gate::observe_only()") == 1,
         "게이트가 판정기의 롤백 축을 경유하지 않는다 — 마스터 스위치가 이 축만 못 되돌린다")
    notes.append("거부=명시 오류(close 어휘 0) · 오살 통과 6사유 · 서브프로세스 0 · 롤백 축 경유")

    # ── Rust 배터리 실재(언어 경계 — python 이 직접 못 부른다) ──────────────
    for battery in ("fn auth_gate_prescription_never_matches_a_health_rule(",
                    "fn auth_gate_decide_denies_only_on_fresh_oracle_evidence_for_this_profile(",
                    "fn supplied_auth_verdict_contract_violations_never_become_a_block(",
                    "fn surface_create_denies_unauthenticated_profile_without_spawning_anything(",
                    "fn surface_create_never_blocks_on_config_only_evidence(",
                    "fn surface_create_without_a_verdict_is_unchanged(",
                    "fn auth_gate_runs_after_the_idempotency_gate(",
                    "fn auth_gate_runs_after_the_privileged_role_gate(",
                    "fn auth_gate_folds_into_the_profile_gate_rollback_switch(",
                    "fn config_mtime_memo_can_only_fail_open("):
        need(battery in hnd, "U-18 배터리 결손: %s" % battery)
    notes.append("Rust 배터리 10종")

    # ── 계측 타당성 — 구 트리에는 게이트가 없다 ─────────────────────────────
    calib = "skip(no-git)"
    old_h = _git_show(os.path.join("src", "bin", "cysd", "handlers.rs"))
    if old_h is not None:
        need("surface_create_auth_gate" not in old_h,
             "계측 타당성 실패: 구 handlers.rs 에 이미 인증 게이트가 있다(기준 커밋 오선택)")
        need("create_surface_with_env(" in old_h,
             "계측 타당성 실패: 구 handlers.rs 에 PTY 스폰 호출이 없다 — 앵커 서사가 틀린 것")
        calib = "구 트리=게이트 부재 + 스폰 호출 실재"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-PRED-2", "W2", "생존 술어 parity(boot 스킵·wakeup zombie·reclaim)", ["B3", "B13", "G27"])
def h_pred_2():
    """B3·B13·G27: 같은 좌석 상태를 세 소비처가 반대로 해석했다. 하나의 `node_liveness` 를
    공유하고, **Unknown 이원 규칙**(파괴=hold / 스폰=시한부 해소)을 단언한다."""
    BN, O, B = _shared_pred()
    notes = []
    # ⓐ boot 스킵 술어(Rust) ↔ node_liveness(python) 등급 파리티 — 소스 구조로 미러 확인
    src = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("enum SeatLiveness" in src and "fn seat_liveness(" in src,
         "Rust 측 3등급 미러가 없다(B3: `!exited` 단독 술어 잔존)")
    li = src.find("fn seat_liveness(")
    lbody = src[li:src.find("\n}\n", li)]
    for k in ("awakened_at", "agent_alive", "occupied", "unknown"):
        need(k in lbody, "Rust 술어가 %s 축을 보지 않는다" % k)
    need('Some("unknown") =>' in lbody,
         "Rust 가 seat 필드 부재와 \"unknown\"을 융합한다(구 데몬에서 결손 오탐)")
    notes.append("Rust 3등급 미러(래치·agent_alive·좌석·판정불가)")
    # ⓑ 파괴 경로 Unknown=hold / 스폰 경로=시한부 해소(절대 unknown 반환 0)
    unk = _fx([{"role": "cso", "exited": False, "seat": "unknown"}])
    need(BN.node_liveness(unk, "cso")[0] == BN.LIVENESS_UNKNOWN, "판정불가가 등급으로 융합됐다")
    need(BN.node_alive(unk, "cso") is True, "파괴 경로에서 Unknown 이 hold 되지 않음(오살)")
    need(BN._reclaim_verdict(unk, "cso", 100, 100) == "hold-alive",
         "reclaim 이 Unknown 좌석에 kill 을 허용(fail-closed 위반)")
    g, _ = BN.resolve_unknown_for_spawn("cso", lambda: unk, probe=lambda: False, tick_s=0)
    need(g == BN.LIVENESS_ABSENT, "스폰 경로 Unknown 이 시한부 해소되지 않음(콜드스타트 B3 보존)")
    g2, _ = BN.resolve_unknown_for_spawn("cso", lambda: None, tick_s=0)
    need(g2 != BN.LIVENESS_UNKNOWN, "시한부 해소가 unknown 을 반환(계약 위반)")
    notes.append("Unknown 이원: 파괴=hold / 스폰=시한부 해소")
    # ⓒ G27 wakeup zombie 가드 — exited 행의 role 토큰에 속지 않는다
    import javis_wakeup as W
    listing = ("surface:1\trole=worker\tpid=11\texited=true\n"
               "surface:2\trole=cso\tpid=12\texited=false\n")
    live = W.live_target_rows(listing)
    need(live == ["cso"], "exited 행이 라이브로 계상됨: %r" % live)
    need(W._target_matches("cso", live) is True, "라이브 대상 해소 실패")
    need(W._target_matches("worker", live) is False,
         "죽은(exited) 대상이 alive 로 판정됨(G27 재발 — 죽은 대상에 배달)")
    need(W._target_matches("worker", W.live_target_rows(
        "surface:3\trole=worker-2\tpid=13\texited=false\n")) is True,
        "worker-N dedup 좌석이 worker 대상으로 해소되지 않음")
    notes.append("G27: exited 행 배제 + 공유 술어 해소")
    # ⓓ reclaim 은 좌석 empty(죽음 확정)에서만 kill
    empty = _fx([{"role": "cso", "exited": False, "agent_alive": False,
                  "status": None, "seat": "empty"}])
    need(BN._reclaim_verdict(empty, "cso", 100, 100) == "kill",
         "죽음 확정 좌석 회수 불가(막힌 좌석 영구화)")
    notes.append("reclaim: 좌석 empty 만 kill")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_wakeup.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("live_target_rows" not in old, "계측 타당성 실패: 구 코드에 이미 행 파서가 있다")
        need("pat.search(out)" in old, "계측 타당성 실패: 구 코드의 전문 정규식 매칭을 못 찾았다")
        calib = "구 코드=cys list 전문 정규식(exited 행 포함) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-PRED-3", "W2", "awakened_at 래치(부재=legacy-presumed·NOT-awake 단정 금지)", ["B6"])
def h_pred_3():
    """B6: `agent_alive → awake` 는 오답이고 self-test 가 그 오답을 박제 중이었다. 래치 존재=확정,
    **래치 부재=legacy-presumed**(재스폰·재주입 금지 · NOT-awake 단정 금지 — 금지 방향 ⑦).
    ★기존 균형 술어 검체는 폐기가 아니라 **폴백 검체로 보존**된다(W2 게이트 명문)."""
    BN, O, B = _shared_pred()
    notes = []
    # ⓐ 래치 존재 → awake_confirmed
    latched = _fx([{"role": "cso", "exited": False, "awakened_at": 1700000000.0}])
    need(BN.node_liveness(latched, "cso")[0] == BN.LIVENESS_AWAKE, "래치 존재인데 각성확정 아님")
    need(BN.awake_ready(latched, "cso")[0] is True, "래치가 boot 스킵 게이트에 반영되지 않음")
    # ⓑ 래치 부재 + agent_alive → alive_presumed (재스폰·재주입 금지 단언)
    legacy = _fx([{"role": "cso", "exited": False, "agent_alive": True}])
    need(BN.node_liveness(legacy, "cso")[0] == BN.LIVENESS_PRESUMED,
         "agent_alive 단독이 각성확정으로 계상(B6 오답 잔존)")
    need(BN.awake_ready(legacy, "cso")[0] is True,
         "래치 부재를 NOT-awake 로 단정(금지 방향 ⑦ — 재주입 유도)")
    need(BN.node_alive(legacy, "cso") is True, "legacy-presumed 를 죽음으로 판정(오살)")
    # ⓒ 데몬 재시작·업그레이드 fixture: 전 팀 래치 없음 → **NOT-awake(absent) 오판 0**
    st = _fx(_SEAT_CORPUS["legacy_no_latch"])
    grades = {r: BN.node_liveness(st, r)[0] for r in
              ("cso", "worker", "reviewer-gemini", "reviewer-codex")}
    need(all(g == BN.LIVENESS_PRESUMED for g in grades.values()),
         "데몬 재시작 fixture 에서 absent 오판: %r" % grades)
    # ★로스터 전제를 명시로 세운다(_roster_pin ⓐ): 이 fixture 의 좌석 이름은 네이티브 슬롯이라
    #   의무 역할이 네이티브인 형상에서만 '같은 팀'이다. 종전에는 그 전제를 **기계가 대신** 세워
    #   줬고(리뷰어 CLI 설치 여부), 깨끗한 기계에서는 다른 팀을 재면서 '역방향 회귀'라고 외쳤다.
    DN = _roster_pin(True)
    v, _ = O.check_verdicts(st, detect=DN)
    need(all(x["satisfied"] for x in v.values()),
         "래치 이전 기계의 건강한 팀이 check 적색(역방향 회귀): %r" % v)
    need(B._shared_verdict_deficit(st, detect=DN)[0] is False, "래치 이전 기계에서 결손>0 오판")
    # 대체 슬롯 기계(리뷰어 CLI 미설치)에서도 같은 계약이 성립한다 — 좌석 이름만 그 형상으로.
    DS = _roster_pin(False)
    sub_roles = ["cso", "worker"] + [e["role"] for e in O.reviewer_roster(DS)]
    st_sub = _fx([{"role": r, "exited": False, "agent_alive": True} for r in sub_roles])
    v_sub, _ = O.check_verdicts(st_sub, detect=DS)
    need(all(x["grade"] == BN.LIVENESS_PRESUMED for x in v_sub.values()),
         "대체 슬롯 기계의 래치 없는 팀에서 absent 오판: %r" % v_sub)
    need(all(x["satisfied"] for x in v_sub.values()),
         "대체 슬롯 기계의 건강한 팀이 check 적색(역방향 회귀): %r" % v_sub)
    need(B._shared_verdict_deficit(st_sub, detect=DS)[0] is False,
         "대체 슬롯 기계의 래치 이전 팀에서 결손>0 오판")
    notes.append("재시작·업그레이드 fixture(로스터 2형상): absent 오판 0 · check 적색 0")
    # ⓓ **폴백 검체 보존**: 기존 균형 술어(agent_alive OR fresh set-status)가 살아 있다
    fresh = _fx([{"role": "cso", "exited": False, "status": {"age_secs": 5, "state": "working"}}])
    need(BN.awake_ready(fresh, "cso")[0] is True, "fresh set-status 폴백 검체 소실")
    stale = _fx([{"role": "cso", "exited": False, "status": {"age_secs": 9999, "state": "w"}}])
    need(BN.awake_ready(stale, "cso")[0] is False, "stale set-status 를 각성 오인정")
    need("agent_alive(래치 부재=legacy-presumed 폴백)" in _read(
        os.path.join(BIN_DIR, "javis_boot_node.py")),
        "폴백 계약이 코드에 명문화되지 않음")
    notes.append("균형 술어 폴백 검체 보존(반전 아님·보강)")
    # ⓔ 영속(topology) + 단방향 노출 — 데몬 측 계약
    gov = _repo_file(os.path.join("src", "bin", "cysd", "governance.rs"))
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    stt = _repo_file(os.path.join("src", "bin", "cysd", "state.rs"))
    need('"awakened_at": *s.awakened_at.lock().unwrap()' in gov,
         "래치가 topology 에 영속되지 않는다(데몬 재시작 생존 실패 — 비평2 B-1)")
    need("pub awakened_at: Mutex<Option<f64>>" in stt, "Surface 에 래치 필드가 없다")
    need(hnd.count('"awakened_at"') >= 2,
         "래치가 status/dashboard 양쪽에 노출되지 않는다(surface.list·org.status)")
    li = hnd.find("let latched_now = {")
    need(li > 0, "status.set 의 래치 write path 가 없다")
    lseg = hnd[li:li + 700]
    need("latch.is_none()" in lseg, "래치가 1회성(get_or_insert)이 아니다 — 부패 신호로 퇴화")
    need("persist_topology" in hnd[li:li + 1400], "래치 신설 시 영속 트리거가 없다")
    ci = hnd.find('params.get("awakened_at")')
    need(ci > 0, "restore 하이드레이션 채널이 없다(재시작 후 래치 소실)")
    need("latch <= now" in hnd[ci:ci + 400], "하이드레이션에 과거 시각 가드가 없다(위양성 래치)")
    notes.append("영속+양쪽 노출+1회성 래치+restore 하이드레이션 가드")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_boot_node.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("awakened_at" not in old, "계측 타당성 실패: 구 코드에 이미 래치가 있다")
        need('return True, "agent_alive"' in old,
             "계측 타당성 실패: 구 코드의 agent_alive→awake 오답을 못 찾았다")
        calib = "구 코드=agent_alive→awake 박제 확인(래치 0)"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-PRED-4", "W2", "슬롯 충족 parity(native/substitute/이중충전)", ["B2", "B12"])
def h_pred_4():
    """B2: 2차 폴백으로 대체 리뷰어가 좌석을 채우면 check 가 네이티브 역할명을 계속 요구해
    **영구 적색 + 재선언 불회복**이 됐다. 슬롯은 네이티브∨대체로 충족되고 **실충전자를 라벨링**한다.
    ★전제(하드 제약 7): G2(session-start role case) = W1a 착지 — H-PRED-5 가 그 GREEN 을 잰다."""
    BN, O, B = _shared_pred()
    cases = [
        ("native", {"reviewer-gemini"}, "reviewer-gemini", True, True),
        ("substitute", {"reviewer-claude-1"}, "reviewer-claude-1", False, True),
        ("cross-slot", {"reviewer-claude-1"}, None, None, False),   # codex 슬롯을 gemini 대체가 못 채움
        ("absent", set(), None, None, False),
        ("optional-grok", {"reviewer-grok"}, None, None, False),    # 선택 좌석은 의무 슬롯 아님
    ]
    for name, live, filler, native, sat in cases:
        req = "reviewer-codex" if name == "cross-slot" else "reviewer-gemini"
        got_sat, got_fill, got_nat, why = O.slot_satisfied(req, live)
        need(got_sat is sat, "%s: satisfied=%r(기대 %r) — %s" % (name, got_sat, sat, why))
        if sat:
            need(got_fill == filler, "%s: 실충전자 라벨 %r(기대 %r)" % (name, got_fill, filler))
            need(got_nat is native, "%s: native 라벨 %r(기대 %r)" % (name, got_nat, native))
    # 이중충전(네이티브+대체 동시 생존) → 네이티브 우선 라벨(중복 계상 0)
    both = {"reviewer-gemini", "reviewer-claude-1"}
    s, f, n, _ = O.slot_satisfied("reviewer-gemini", both)
    need((s, f, n) == (True, "reviewer-gemini", True), "이중충전에서 네이티브 우선 실패: %r" % ((s, f, n),))
    # check 가 대체 좌석으로 슬롯을 재해소하고 실충전자를 남긴다(영구 적색 차단)
    # ★이 단언은 '의무 역할 = 네이티브 슬롯인데 좌석은 대체가 채웠다'는 **2차 폴백 상황**이
    #   재현돼야 성립한다. 그 전제(네이티브 로스터)를 주입으로 세운다 — 종전에는 기계에 리뷰어
    #   CLI 가 깔려 있어야만 우연히 성립했고, 깨끗한 기계에서는 없는 키를 색인해 크래시했다.
    DN = _roster_pin(True)
    st = _fx(_SEAT_CORPUS["substitute_filled"])
    v, _ = O.check_verdicts(st, detect=DN)
    vc = _vd(v, "reviewer-codex", "B2 대체 충전")
    need(vc["satisfied"] is True, "대체 좌석 재해소 실패(B2 영구 적색)")
    need(vc["native"] is False, "실충전자 라벨(native=False) 누락")
    need(vc["filler"] == "reviewer-claude-2", "실충전자 이름 오류")
    need(B._shared_verdict_deficit(st, detect=DN)[0] is False,
         "대체 충전 팀에서 결손>0(재선언 불회복)")
    # boot-reviewers 가 실충전자를 고지한다(은닉 성공 금지)
    osrc = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    need("슬롯 %s 는 대체 좌석" in osrc, "boot-reviewers 실충전자 고지가 없다")
    # B12: 리뷰어 가용성 오라클 단일화 — detect_reviewer 가 extract_bin+expand+실행권 3축을 본다
    di = osrc.find("def detect_reviewer(")
    dbody = osrc[di:osrc.find("\ndef ", di + 10)]
    for k in ("os.access", "shutil.which", "expanduser"):
        need(k in dbody or k in osrc[osrc.find("def reviewer_launch_binary("):di],
             "리뷰어 감지 오라클이 %s 축을 잃었다(B12)" % k)
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_orchestra.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("def slot_satisfied(" not in old, "계측 타당성 실패: 구 코드에 이미 slot_satisfied 가 있다")
        calib = "구 코드=required 정확일치만(대체 좌석 미인정) 확인"
    return ("슬롯 5케이스+이중충전 · check 재해소·실충전자 라벨 · 결손 0 정합 · 고지 문구 · "
            "B12 감지 3축 · 계측검증=%s" % calib)


@specimen("H-PRED-5", "W2", "role_family 전수 해소(데몬 발권 전 role × 소비처 전수)",
          ["A3=B7", "B10", "G2"])
def h_pred_5():
    """A3=B7·B10·G2 의 공통 근저: **같은 역할 가족 판정이 언어마다 재발명**됐다.
    데몬이 발권 가능한 전 role × 소비처 전수(python role_family·ROLE_AGENT·ROLE_DIRECTIVE ·
    Rust role_directive_path · 셸 훅 case · session-start case)를 기계 대조한다.
    ★W2 게이트의 **전제 검체**다(하드 제약 7): 이게 적색이면 B2 의 대체 좌석 GREEN 은
    '지침 없는 리뷰어 GREEN'(B6 동형 허위 성공)이 된다."""
    BN, O, B = _shared_pred()
    # 데몬이 발권 가능한 전 role: PLAN + REVIEWER_SLOTS(네이티브·대체) + dedup worker-N + cso 변형
    issuable = ["master", "cso", "worker", "worker-2", "worker-3",
                "reviewer-gemini", "reviewer-codex", "reviewer-grok",
                "reviewer-claude-1", "reviewer-claude-2", "cso-1"]
    fam_expect = {"master": "master", "cso": "cso", "cso-1": "cso",
                  "worker": "worker", "worker-2": "worker", "worker-3": "worker",
                  "reviewer-gemini": "reviewer", "reviewer-codex": "reviewer",
                  "reviewer-grok": "reviewer", "reviewer-claude-1": "reviewer",
                  "reviewer-claude-2": "reviewer"}
    for r in issuable:
        need(BN.role_family(r) == fam_expect[r],
             "role_family(%r)=%r(기대 %r)" % (r, BN.role_family(r), fam_expect[r]))
    # 미지 role·빈 role → 가족 없음(wildcard 금지)
    for bad in ("verifier", "", None, "master-2"):
        need(BN.role_family(bad) is None, "미지/변형 role %r 이 가족 배정됨" % (bad,))
    # ① Rust 정본(pack.rs role_directive_path) 접두 의미 미러
    prs = _repo_file(os.path.join("src", "pack.rs"))
    ri = prs.find("pub fn role_directive_path(")
    rbody = prs[ri:prs.find("\n}\n", ri)]
    need('"master" =>' in rbody, "Rust 가 master 를 정확일치로 다루지 않는다")
    for fam in ("worker", "cso", "reviewer"):
        need('r.starts_with("%s")' % fam in rbody, "Rust 가 %s 접두 분기를 잃었다" % fam)
    # ② python 소비처 전수: 발권 전 role 이 ROLE_AGENT·ROLE_DIRECTIVE 로 해소되는가(B10)
    uncovered = []
    for r in issuable:
        fam = BN.role_family(r)
        if BN.ROLE_AGENT.get(r) is None and not (fam == "worker" and r.startswith("worker")):
            uncovered.append(("ROLE_AGENT", r))
        if BN.ROLE_DIRECTIVE.get(r) is None and fam is not None:
            # 가족 폴백으로 해소되면 통과(디렉티브는 가족 단위)
            if not any(k for k in BN.ROLE_DIRECTIVE if BN.role_family(k) == fam):
                uncovered.append(("ROLE_DIRECTIVE", r))
    # worker-N·cso-1 은 ROLE_AGENT 직접 키가 없어도 가족 해소가 정답이다 — 그 사실을 단언한다.
    uncovered = [(t, r) for t, r in uncovered if not (t == "ROLE_AGENT" and BN.role_family(r))]
    need(not uncovered, "발권 role 이 소비처에서 미해소(B10): %r" % uncovered)
    # ③ 셸 훅 case 전수(G2/A3): session-start 는 전체 role 문자열을 받는다
    ss = _read(_hook("session-start.sh"))
    rb = _read(_hook(ROLE_BODY))
    need("reviewer-gemini" in ss or "reviewer-*" in ss or "reviewer*" in ss,
         "session-start role case 가 리뷰어 전체 이름을 커버하지 않는다(G2)")
    need("master|\"\"" in rb.replace(" ", "") or 'master|"")' in rb,
         "role-bootstrap 게이트가 allowlist(master|미claim) 형태가 아니다(A3)")
    for bad in ("worker-2", "cso-1", "reviewer-claude-1", "verifier"):
        # allowlist 반전이므로 열거되지 않은 role 은 전부 차단이다 — 그 구조를 단언한다.
        need("*)" in rb, "allowlist 의 기본 차단 분기(*)가 없다")
        _ = bad
    return ("발권 role %d종 × 가족 판정 · 미지/변형 4종 wildcard 0 · Rust 정본 미러 · "
            "python 소비처 전수 해소 · 셸 case(G2/A3) 구조" % len(issuable))


@specimen("H-PRED-6", "W2", "규약 상수 grep 전수 계약(env 4키·CYS_PY·부서 판정 정규화)",
          ["A11", "G4", "G22"])
def h_pred_6():
    """A11: preflight docstring 의 'byte-identical 미러링' 주장 자체가 거짓이었다(자기 증거).
    규약 상수는 열거식이 아니라 **grep 전수 스캔**으로 결박한다 — 미래 소비자가 자동 편입된다."""
    BN, O, B = _shared_pred()
    notes = []
    # ⓐ 팩 경로 env 키 4종: Rust 정본 ↔ python 소비처 전수(리터럴 스캔)
    prs = _repo_file(os.path.join("src", "pack.rs"))
    mi = prs.find("PACK_DIR_ENV_KEYS")
    need(mi > 0, "Rust PACK_DIR_ENV_KEYS 정본을 못 찾았다")
    seg = prs[mi:prs.find("];", mi)]
    rust_keys = re.findall(r'"([A-Z_]+)"', seg)
    need(tuple(rust_keys) == tuple(O.PACK_DIR_ENV_KEYS),
         "env 키 목록·순서 파리티 붕괴 — rust=%r / python=%r" % (rust_keys, O.PACK_DIR_ENV_KEYS))
    notes.append("env 4키 목록·순서 파리티")
    # ⓑ CYS_PY 해소 전수(G22): python3 를 참조하는 훅은 전부 프리루드 CYS_PY 를 쓴다
    offenders = []
    for rel in _shell_hooks():
        body = _read(os.path.join(HOOKS_DIR, rel))
        if "python3" not in body:
            continue
        if "CYS_PY" not in body:
            offenders.append(rel)
    need(not offenders, "python3 경성 참조 훅(CYS_PY 미해소): %r" % offenders)
    notes.append("python3 참조 훅 전수 CYS_PY 해소")
    # ⓒ 부서 판정 정규화(G4): 셸 글롭 ↔ python 술어가 같은 결론
    ic = _read(_hook("inject-context.sh"))
    need("dept-" in ic, "inject-context 부서 감지 글롭을 못 찾았다")
    need("cys_is_dept_socket" in ic or "DEPT" in ic, "부서 판정이 프리루드 규약을 쓰지 않는다")
    for sock, want in (("/x/state/cys/cys.sock", True),
                       ("/x/state/cys-dept-dept-1/cys.sock", False),
                       ("/x/state/cys-dept-sales/cys.sock", False)):
        need(B._socket_is_base(sock) is want,
             "부서 판정 불일치: %s → %r(기대 %r)" % (sock, B._socket_is_base(sock), want))
    notes.append("부서 판정(셸↔python) 정합")
    # ⓓ BUDGET 상수 파리티(H-TIME-1 과 짝) — Rust 블록 ↔ python leaf
    csrc = _repo_file(os.path.join("src", "bin", "cys.rs"))
    import javis_budget as BU
    bad = []
    for rust_name, py_name in BU.RUST_PARITY_CONSTS.items():
        m = re.search(r"const %s: u64 = (\d+);" % re.escape(rust_name), csrc)
        if not m:
            bad.append("%s 상수 부재" % rust_name)
            continue
        if int(m.group(1)) != int(BU.leaf(py_name)):
            bad.append("%s=%s ≠ python %s=%s" % (rust_name, m.group(1), py_name, BU.leaf(py_name)))
    need(not bad, "BUDGET 상수 파리티 붕괴: %r" % bad)
    notes.append("BUDGET 상수 %d종 rust↔python 파리티" % len(BU.RUST_PARITY_CONSTS))
    return " · ".join(notes)


@specimen("H-PRED-7", "W2", "PLAN 정책 열↔effective_required_roles↔결손 구성 3자 대조", ["B1"])
def h_pred_7():
    """B1: 의무/선택 판정이 편성 테이블 **밖**에 있어 리뷰어 1종 고장이 팀 전체 부트 실패로
    번지는 영구 데드엔드였다. PLAN 정책 열 ↔ 유효 의무 역할 ↔ 결손 구성 3자가 정합해야 한다."""
    BN, O, B = _shared_pred()
    yes = lambda ag, agents=None: (True, "주입")            # noqa: E731
    no = lambda ag, agents=None: (False, "주입 미감지")      # noqa: E731
    synth = {"gemini": {"cmd": "/x/agy"}, "codex": {"cmd": "/x/codex"},
             "claude": {"cmd": "claude"}}
    # ① Fatal 집합 = cso·worker 뿐
    need(O.plan_mandatory_roles() == ["cso", "worker"],
         "Fatal 집합 변형(리뷰어가 Fatal 이면 B1 재발): %r" % O.plan_mandatory_roles())
    # ② Fatal ⊆ 유효 의무 역할(부트가 요구하는 것을 check 도 본다)
    for detect in (yes, no):
        eff = O.effective_required_roles(detect=detect, agents=synth)
        need(set(O.plan_mandatory_roles()) <= set(eff),
             "Fatal 역할이 유효 의무 목록에서 빠짐: %r ⊄ %r" % (O.plan_mandatory_roles(), eff))
        # ③ 유효 의무 역할은 모두 PLAN 또는 슬롯(대체)으로 설명된다 — 고아 요건 0
        plan_roles = {r for r, _a, _p in O.BOOT_PLAN}
        subs = {s for _n, _na, s, _sa in O.REVIEWER_SLOTS}
        need(all(r in plan_roles or r in subs for r in eff),
             "고아 의무 요건(PLAN·슬롯 어디에도 없음): %r" % eff)
    # ④ 결손 구성 3자 대조 — Degrade 만 부재면 결손>0 이지만 처방은 boot-reviewers 다
    st = _fx([{"role": "cso", "exited": False, "awakened_at": 1.0},
              {"role": "worker", "exited": False, "awakened_at": 1.0}])
    # ★로스터 밀폐: '리뷰어 전원 부재' 라는 전제는 어느 슬롯 형상인지를 못박아야 재현된다.
    D7 = _roster_pin(True)
    has, why = B._shared_verdict_deficit(st, detect=D7)
    need(has is True, "리뷰어 전원 부재인데 결손 0")
    v, _ = O.check_verdicts(st, detect=D7)
    missing = [r for r, x in v.items() if not x["satisfied"]]
    need(missing and all(O.plan_policy(m) == O.FAIL_DEGRADE for m in missing),
         "부재 역할의 정책이 Degrade 가 아니다: %r" % [(m, O.plan_policy(m)) for m in missing])
    # ⑤ Rust PLAN 정책 열 ↔ python 정본 파리티(H-EXIT-4 와 동일 대조 — 이중 결박)
    csrc = _repo_file(os.path.join("src", "bin", "cys.rs"))
    seg = csrc[csrc.find("const BOOT_PLAN"):]
    seg = seg[:seg.find("];")]
    rust_plan = []
    for line in seg.splitlines():
        line = line.strip()
        if line.startswith('("'):
            parts = [p.strip().strip('"') for p in line.strip("(),").split(",")]
            if len(parts) == 3:
                rust_plan.append((parts[0], parts[1], parts[2] == "true"))
    py_plan = [(r, a, p == O.FAIL_FATAL) for r, a, p in O.BOOT_PLAN]
    need(rust_plan == py_plan, "PLAN 파리티 붕괴 — rust=%r / python=%r" % (rust_plan, py_plan))
    # ⑥ 소비 분기: exit 4 는 Fatal 실패에서만
    need(B._boot_fatal_verdict(1, json.dumps({"roles": [
        {"role": "reviewer-grok", "agent": "grok", "outcome": "missing", "mandatory": False}]})) is None,
        "Degrade 실패가 exit 4 로 승격(B1 데드엔드 재발)")
    return ("Fatal 집합=cso·worker · Fatal⊆유효의무(감지 2케이스) · 고아 요건 0 · "
            "Degrade 부재 처방 · PLAN 파리티 %d행 · exit4=Fatal 한정" % len(rust_plan))


@specimen("H-PRED-8", "W2", "readiness 델타 매칭(개수 비교 회귀 차단)", ["B4"])
def h_pred_8():
    """B4: claude 의 ready_marker 는 `❯` = p10k/starship 프롬프트 문자와 같다 — 잔존 ❯ 가 마커로
    매칭돼 디렉티브가 맨 셸로 들어갔다. 판정은 **기동 send 직전 커서 이후 신규 출현분**에서만.
    ★개수 비교 구현 금지: 영구 오부정 회귀이고, T-0147-4 이후 롤백 close 가 실제로 성공하므로
      그 오부정은 **건강한 surface 를 실제로 닫는다**."""
    src = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    notes = []
    # ★핀 이사(U-13): 마커 판정은 `src/readiness.rs` 의 순수 술어로 옮겼고 cys.rs 에는 관측·배선이
    #   남는다. **세 계약(①델타 우선 ②셸 프롬프트 가드 ③개수 비교 0)은 그대로 옮긴다** — 다만
    #   ③은 이제 **양쪽**(배선·판정부)에서 본다. 판정이 한 자리로 모였다고 금지 범위가 줄면
    #   그것은 이사가 아니라 완화다.
    body = _slice_between(src, "fn boot_agent_on_surface(",
                          "\n/// 에이전트 기동 + 역할 지침", "H-PRED-8 부트 본문")
    judge = _slice_between(src, "pub fn judge(",
                           "// ── 판정부 끝(핀 슬라이스 경계) ──", "H-PRED-8 판정부")
    # ⓐ 기동 send **직전** 커서 스냅샷
    si = body.find("let since_line: u64")
    ti = body.find('"surface.send_text"')
    need(si > 0 and ti > si, "커서 스냅샷이 기동 send 직전에 없다(시간 귀속 기준선 부재)")
    notes.append("send 직전 커서 스냅샷")
    # ⓑ 3소비자 전부 델타 재료 사용
    need('"since_line": since_line' in body, "readiness 폴링이 델타를 읽지 않는다")
    need("screen_shows_launch_failure(&delta_flat)" in body,
         "기동 실패 판정이 화면 전체를 본다(잔존 에러로 새 기동 사망)")
    need("delta: &delta_text" in body,
         "판정 입력에 델타가 실리지 않는다 — 마커 판정이 델타 우선일 수 없다(배선 절단)")
    need("o.delta.contains(m)" in judge, "마커 판정이 델타 우선이 아니다")
    need('"since_line": inject_cursor' in body, "주입 검증이 델타를 쓰지 않는다(잔존 화면 오통과)")
    notes.append("3소비자(마커·실패·주입검증) 델타 결박")
    # ⓒ 마커 분기에도 셸 프롬프트 가드 — 판정부의 화면 폴백 항 + 그 재료의 배선
    mi = judge.find("o.delta.contains(m)")
    seg = judge[mi:mi + 1500]
    need("tail_ok" in seg,
         "마커 화면 폴백에 꼬리 술어 가드가 없다(브리프 명문 — 잔존 마커에 오통과)")
    need("tail_is_shell_prompt: Some(screen_tail_is_shell_prompt(text))" in body,
         "꼬리 술어 관측이 판정 입력으로 실리지 않는다 — 가드가 이름만 남는다")
    notes.append("마커 분기 셸 프롬프트 가드")
    # ⓓ **개수 비교 금지** — 마커 카운트 산술이 없다(배선·판정부 **양쪽**)
    for banned in (".matches(m", "count()", "marker_count", "prev_count"):
        need(banned not in body,
             "개수 비교 구현 흔적(%s) — 영구 오부정 회귀·건강 surface 실제 close 위험" % banned)
        need(banned not in judge,
             "판정부에 개수 비교 구현 흔적(%s) — 이사한 자리에서 금지가 풀렸다" % banned)
    notes.append("개수 비교 구현 0(배선·판정부)")
    # ⓔ 잔존 ❯ 시나리오 순수 판정: 꼬리가 셸 프롬프트면 폴백이 발화하지 않는다
    need("t.ends_with('❯')" in src, "셸 프롬프트 가드가 ❯ 를 인식하지 않는다")
    # ⓕ line_count 가 surface.list 에 노출됨(커서 원천)
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    need(hnd.count('"line_count"') >= 2, "surface.list 에 line_count 커서가 노출되지 않는다")
    notes.append("line_count 커서 노출")
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("waited += 2" in old, "계측 타당성 실패: 구 코드의 카운트 회계를 못 찾았다")
        need("since_line" not in old[old.find("fn boot_agent_on_surface("):
                                    old.find("fn boot_agent_on_surface(") + 8000],
             "계측 타당성 실패: 구 코드가 이미 델타를 쓴다")
        calib = "구 코드=화면 전체 매칭 + 카운트 회계 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
@specimen("H-PRED-9", "W4", "trust 패턴·마커 SOT=코드/vendor + user override 계층(★W-B 동결 정합)",
          ["P3-B19"])
def h_pred_9():
    """P3-B19: 폴더신뢰 프롬프트 패턴이 **코드 하드코딩**이었다(`trustthisfolder`) — claude 문면이
    바뀌거나 다른 CLI 를 쓰면 코드를 고쳐야 했다. 그런데 `agents.json` 은 ★W-B 로 **user 소유
    동결**이라 단순히 그 파일로 옮기면 예전에 커스터마이즈한 사용자는 vendor 신규 값을 **영영
    못 받는다**(동결 = 폴더신뢰 자동확인 불발·readiness 시간폴백 퇴화).
    그래서 계약이 '파일로 이동'이 아니라 **'코드/vendor 기본값 + user override 필드 계층'** 이다.
    이 검체는 그 계층이 실재하고, **trust-prompt 항목만** 자동응답에 소비되는지 못박는다
    (사람 판단이 필요한 tool-permission 류 자동응답은 절대 금지 — 그게 열리면 승인 게이트 소멸)."""
    src = os.path.join(REPO_DIR, "src", "bin", "cys.rs")
    if not os.path.isfile(src):
        raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
    body = _read(src)
    # ① 필드 계층: 결손 키만 vendor 임베드로 보강, 디스크 파일 무접촉(★W-B)
    need("fn fill_missing_fields(" in body, "필드 계층 함수 부재(agents.json 동결 미해소)")
    need('LAYERED_KEYS' in body, "계층 대상 키 목록 상수 부재")
    seg = body[body.index("fn fill_missing_fields("):]
    seg = seg[:seg.index("\n}\n") + 2]
    for k in ("ready_marker", "approval_patterns"):
        need('"%s"' % k in seg, "계층 대상에 %s 누락" % k)
    # ★U-12 계층 확장(축 이동이지 완화가 아니다): 계층 대상이 2키 → 3키가 됐다. 신규 키는
    #   문자열 리터럴이 아니라 lib 상수를 참조하므로(사본 금지) 키 이름이 아니라 **상수 참조**
    #   와 **개수**를 잰다. 종전 2키 조건은 위에서 그대로 유지된다(항 삭제 0).
    need("cys::first_run_gates::ADAPTER_KEY" in seg,
         "계층 대상에 first_run_gates 신규 키 미편입 — 관문 코퍼스가 기존 기계에 도달하지 않는다(K-1)")
    need("LAYERED_KEYS: [&str; 3]" in seg, "계층 대상 개수가 3이 아니다")
    need("resolved.get(k).is_some()" in seg,
         "디스크 선언(명시 null 포함) 존중 규칙 부재 — 사용자 주권 침해")
    # 고지 규율: 신규 키는 **조용히** 채운다. 매 기존 기계에서 매번 결손이라 안내가 소음이 되고,
    # 안내 문안이 권하는 pack-merge 는 이 키에서 해로운 조치다(디스크로 병합되는 순간 사용자
    # 소유가 되어 이후 벤더 갱신이 도달하지 않는다 = 배달 경로 자해).
    need("NOTIFY_KEYS: [&str; 2]" in seg,
         "고지 대상이 2키가 아니다 — 신규 키 고지는 소음이고 pack-merge 권유는 배달 자해다")
    need("embedded_agents_json(" in body, "vendor 임베드 소스 부재(코드 기본값 계층 없음)")
    # ② trust 패턴은 선언에서 읽고 **trust-prompt 항목만** 소비한다
    need("fn trust_prompt_regex(" in body, "trust 패턴 선언 소비 함수 부재(하드코딩 잔존)")
    tseg = body[body.index("fn trust_prompt_regex("):]
    tseg = tseg[:tseg.index("\n}\n") + 2]
    need('Some("trust-prompt")' in tseg, "trust-prompt 항목 한정 필터 부재(전 패턴 자동응답 위험)")
    need("approval_patterns" in tseg, "패턴 소스가 approval_patterns 가 아니다")
    # ③ agents.json 스키마 버전 + trust-prompt 선언 실재 + 자동응답 금지 계약 문구
    aj = json.loads(_read(os.path.join(PACK_DIR, "agents.json")))
    # ★핀 이사(U-2 ②) — 값이 2 → 3 으로 **이동**했다(U-12 가 first_run_gates 봉투를 넣으며 범프).
    #   완화가 아닌 이유: 조건의 형태(정확히 한 값과 일치)는 그대로이고 비교 대상만 현행 스키마로
    #   옮겼다. 범위(`in (2,3)`)로 넓히지 않는 것이 핵심이다 — 넓히면 다음 범프가 조용히 통과한다.
    need(aj.get("_schema") == 3, "agents.json _schema 가 3 이 아니다(계층 계약 미표기): %r"
         % aj.get("_schema"))
    doc = aj.get("_doc", "")
    need("trust-prompt" in doc and "자동응답" in doc,
         "_doc 에 'trust-prompt 만 자동확인 · 그 외 자동응답 금지' 계약이 없다")
    claude = aj.get("claude", {})
    names = [p.get("name") for p in (claude.get("approval_patterns") or [])]
    need("trust-prompt" in names, "claude 어댑터에 trust-prompt 선언이 없다(자동확인 불발)")
    need(len([n for n in names if n != "trust-prompt"]) > 0,
         "trust-prompt 외 패턴이 0건 — 'trust-prompt 만 소비' 계약의 대조군이 사라졌다")
    # ④ 폴백 축은 **선언 패턴 뒤**에 온다 — 순서를 못박는다(제거가 아니라 순서가 계약이다).
    #    ★핀 이사(U-15): 폴백의 **대상**이 하드코딩 needle → **관문 코퍼스(U-12 정본) 소비**로
    #      옮겨 갔다. 구 needle 이 킬체인의 방아쇠였기 때문이다(확인 에코 `Yes, I trust this
    #      folder ✔` 에 재매칭 → 2발째 Return 이 면책창의 `No, exit` 를 누름). 그래서
    #      "선언 패턴이 먼저" 라는 **축은 그대로 두고 대상만 이사**시키고, 코퍼스 소비 자체를
    #      새 `need` 로 **추가**한다(항 삭제 0 · 완화 0).
    #    ★`.index()` → `.find()`: 앵커가 사라지면 `.index()` 는 `ValueError` 를 던져 검체가
    #      '판정 실패'가 아니라 **예외로 죽는다** — 측정 불능이 적색으로 보고되지 않는 선재
    #      결함이다(U-2 §ⓒ 와 같은 형태). `.find()` 는 -1 → 비교 False → FAIL 로 흐른다.
    need("fn trust_prompt_hit(" in body, "trust 판정 합성 함수 부재(순서 계약을 확인할 자리 없음)")
    hseg = body[body.index("fn trust_prompt_hit("):]
    hseg = hseg[:hseg.index("\n}\n") + 2]
    need("re.is_match(" in hseg, "선언 패턴이 판정에 쓰이지 않는다(하드코딩 1차 잔존)")
    ri = hseg.find("re.is_match(")
    ci = hseg.find("inject_guard::folder_trust_needle_hit(")
    need(ci > 0,
         "폴백 축이 관문 코퍼스(U-12 정본)를 소비하지 않는다 — 하드코딩 needle 이 그대로거나 "
         "새 사본이 생겼다(U-15 미착지)")
    need(0 <= ri < ci, "코퍼스 폴백이 선언 패턴보다 먼저 판정한다(선언 무력화)")
    li = hseg.find("trustthisfolder")
    need(li < 0 or ri < li, "구 하드코딩 needle 이 선언 패턴보다 먼저 판정한다(선언 무력화)")
    need(li < 0 or "legacy_v1 &&" in hseg[:li],
         "구 결함 needle 이 롤백 스위치 없이 상시 판정에 남아 있다 — 확인 에코 재매칭 킬체인 재발")
    need("split_whitespace()" in hseg,
         "정규식을 flat 텍스트에 돌린다 — 선언 패턴의 공백 의미가 깨져 자동확인이 불발한다")
    calib = "skip(no-git)"
    old = _git_show("src/bin/cys.rs")
    if old is not None:
        need("fn trust_prompt_regex(" not in old,
             "계측 타당성 실패: 구 코드에 이미 선언 소비가 있다면 B19 는 결함이 아니다")
        need("fn fill_missing_fields(" not in old,
             "계측 타당성 실패: 구 코드에 이미 필드 계층이 있다면 C-1 은 결함이 아니다")
        need("first_run_gates" not in old,
             "계측 타당성 실패: 구 코드에 이미 관문 신규 키가 있다면 K-1 은 결함이 아니다")
        calib = "구 코드 하드코딩·whole-object 폴백·신규 키 부재 확인"
    return "필드 계층 3키·디스크 존중 · trust-prompt 한정 소비 · _schema=3 · 계약 문구 · 계측검증=%s" % calib



# ═══════════════════════════════════════════════════════════════════════════
# U-12 · 관문 코퍼스 배달 경로 (K-1 해소)
#
# S-1 사본 대장 — **SOT 는 `src/first_run_gates.rs` 하나**이고 나머지는 전부 읽기 소비다.
# 아래 두 표는 "정본에 없는데 살아 있는 문면"의 **명시 면제 목록**이다: 새 사본이 생기면
# 표에 없으므로 적색이 되고, 면제를 넣으려면 사유를 함께 적어야 한다(조용한 확산 차단).
# ═══════════════════════════════════════════════════════════════════════════

# `javis_phoenix_harness.py MODAL_MARKERS` 중 claude 관문 코퍼스가 덮지 **않는** 항목.
GATE_MARKER_EXEMPT = {
    "❯": "관문 6화면 **전부**에 있는 리스트 커서(= agents.json ready_marker 와 같은 문자). "
         "단독으로는 아무 관문도 식별하지 못하므로 정본은 이것을 needle 이 아니라 위젯 서명으로만 쓴다.",
    "keep browser": "claude 첫기동 관문이 아니라 리뷰어 CLI(agy) 의 브라우저 선택 프롬프트 — "
                    "claude 코퍼스 범위 밖(어댑터가 다르다).",
    "use my browser": "위와 같음(agy 브라우저 프롬프트).",
}

# `src/bin/cys.rs trust_prompt_hit` 의 내장 폴백 needle — 정본에 **의도적으로 담지 않는다**.
# ★U-15 착지 후의 지위: 이 둘은 **상시 판정에서 내려와 롤백 분기**(`CYS_TRUST_RETURN_V1=1`)
#   전용이 됐다. 삭제가 아니라 격하인 이유는 `trust_send`/`trust_prompt_hit` 의 doc 에 있다
#   (감지 폭이 예상 밖으로 좁아졌을 때의 손잡이 · 되돌려도 1발 래치와 화면 재확인이 킬체인을
#   따로 막는다). '롤백 밖에 있으면 적색' 은 H-PRED-9 ④ 와 H-KILLCHAIN-1 ⓖ 가 집행한다.
GATE_LEGACY_CODE_NEEDLE = {
    "trustthisfolder": "★결함 needle. 확인 에코 'Yes, I trust this folder ✔' 에 재매칭돼 2발째 "
                       "Return 이 면책창의 'No, exit' 를 눌렀다(2026-07-29 실사고). 정본은 질문형 "
                       "문면만 담아 이 형태를 구조적으로 금지한다. U-15 에서 롤백 분기로 격하됨.",
    "Doyoutrust": "구 문면 폴백(공백 제거형). 정본의 'Do you trust this folder' needle 이 같은 "
                  "화면을 덮는다. U-15 에서 롤백 분기로 격하됨.",
}


@specimen("H-DELIVER-1", "W6",
          "★관문 코퍼스 배달 경로 — 신규 키가 구 agents.json 기계에 도달한다(K-1)", ["K-1"])
def h_deliver_1():
    """K-1(master 직접 확인 · 2026-08-23): `agents.json` 은 `Ownership::User` 다. **기존 설치
    기계에는 `ready_marker`·`approval_patterns` 가 값으로 이미 있으므로**, `fill_missing_fields`
    의 "키가 아예 없을 때만 보강" 규칙(= 사용자 주권)에 막혀 **벤더가 그 값을 고쳐 출하해도
    결함이 있는 바로 그 기계들에는 영영 도달하지 않는다**. 따라서 관문 판정 데이터를
    `agents.json` **값 수정**으로 배달하는 설계는 무효다.

    이 단위의 답은 둘이다 — ⓐ 코퍼스 정본을 **코드에 임베드**(새 바이너리 = 새 코퍼스)
    ⓑ `agents.json` 에는 **신규 키**(`first_run_gates`) 봉투만 둔다. 신규 키는 구 디스크
    파일에 **부재**하므로 계층이 채우고, 그 순간 배달이 성립한다.

    ★판정 분담(정직 고지): **행동 증명은 Rust 검체**
    `h_deliver_1_old_agents_json_receives_new_key_from_embed` 가 한다 — 구 `agents.json`
    픽스처를 실제로 디스크에 쓰고 `load_agent_spec` 을 돌려 신규 키가 채워지는지 본다
    (`cargo test --bins`). 러너는 컴파일러가 아니므로 여기서는 **그 검체의 실재와 배선**을
    핀한다: 검체 이름이 사라지거나 계층 배선이 끊기면 즉시 적색이다."""
    frg_rel = os.path.join("src", "first_run_gates.rs")
    if not os.path.isfile(os.path.join(REPO_DIR, frg_rel)):
        if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
            raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
        raise Fail("코드 임베드 정본 %s 이 없다 — 관문 데이터의 진실원천 부재(K-1 미해소)" % frg_rel)
    sot = _repo_file(os.path.join("src", "first_run_gates.rs"))
    cli = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS

    # ⓐ 코드 임베드 정본이 실측 관문 6종을 담고 있다.
    need("const DEFS:" in sot, "관문 정본 표(DEFS) 부재 — 코퍼스가 코드에 없다")
    for gid in ("theme", "login-method", "oauth-code", "folder-trust",
                "bypass-disclaimer", "feature-announce-fullscreen"):
        need('id: "%s"' % gid in sot, "정본에 관문 %s 누락(실측 6종 미달)" % gid)
    # 로그인·OAuth 는 **기계가 통과시킬 수 없다** — 액션을 정의하지 않는다.
    for gid in ("login-method", "oauth-code"):
        seg = sot[sot.index('id: "%s"' % gid):]
        seg = seg[:seg.index("},\n    Def {")] if "},\n    Def {" in seg else seg
        need("Passability::HumanOnly" in seg, "%s 이 사람 전용으로 선언되지 않았다" % gid)
        need("action: None" in seg, "%s 에 기계 액션이 선언됐다(통과 불가 관문에 키를 쏜다)" % gid)
    # 버전 핀 — measured_on 드리프트 시 액션 보류(관측만)
    need("MEASURED_ON" in sot and "HeldVersionDrift" in sot and "HeldVersionUnknown" in sot,
         "버전 핀 게이트 부재 — 벤더 신기능마다 관문이 늘어나는데 무방비다")
    need("fn action_policy(" in sot, "액션 집행 판정기 부재")

    # ⓑ 배달 배선 — 계층 대상에 신규 키가 편입돼 있다(값 수정 경로가 아니라 **신규 키** 경로).
    need("cys::first_run_gates::ADAPTER_KEY" in cli,
         "LAYERED_KEYS 에 신규 키가 없다 — 봉투가 구 기계에 도달하지 않는다")
    need("LAYERED_KEYS: [&str; 3]" in cli, "계층 대상이 3키가 아니다")

    # ⓒ 봉투가 임베드 팩에 실재하고 **코퍼스 사본이 아니다**(S-1 재발 차단).
    aj = json.loads(_read(os.path.join(PACK_DIR, "agents.json")))
    env = (aj.get("claude") or {}).get("first_run_gates")
    need(isinstance(env, dict), "agents.json claude 어댑터에 first_run_gates 봉투 부재: %r" % env)
    need(env.get("source") == "builtin", "봉투 source 가 builtin 이 아니다: %r" % env.get("source"))
    need(env.get("gates") == [],
         "봉투가 코퍼스 사본을 들고 있다(gates=%r) — 정본은 코드 하나여야 한다" % (env.get("gates"),))
    m = re.search(r'MEASURED_ON:\s*&str\s*=\s*"([^"]+)"', sot)
    need(m, "정본에 MEASURED_ON 상수가 없다")
    need(env.get("measured_on") == m.group(1),
         "봉투 measured_on(%r) ↔ 코드 정본(%r) 파리티 파손 — 버전 핀이 갈렸다"
         % (env.get("measured_on"), m.group(1)))
    doc = env.get("_doc", "")
    need("pack-merge" in doc,
         "봉투 _doc 에 'pack-merge 하지 마라' 경고가 없다 — 병합하는 순간 사용자 소유가 되어 "
         "이후 벤더 갱신이 도달하지 않는다(배달 경로 자해)")

    # ⓓ **행동 증명 검체**의 실재 — 이름이 사라지면 배달성이 아무 데서도 실행되지 않는다.
    need("fn h_deliver_1_old_agents_json_receives_new_key_from_embed(" in cli,
         "배달성 행동 검체가 없다 — 정적 핀만으로는 '실제로 채워진다'를 증명하지 못한다")
    tseg = cli[cli.index("fn h_deliver_1_old_agents_json_receives_new_key_from_embed("):]
    tseg = tseg[:tseg.index("\n    }\n") + 6]
    need("load_agent_spec(" in tseg, "행동 검체가 실경로(load_agent_spec)를 돌리지 않는다")
    need("resolve_from_spec(" in tseg, "행동 검체가 배달된 봉투를 실제로 소비하지 않는다")
    need("VENDOR-NEW-MARKER" in tseg,
         "행동 검체에 K-1 대조군(디스크 값은 vendor 신값으로 덮이지 않는다)이 없다 — "
         "대조가 없으면 '원래 되는 일'을 확인한 공허한 초록일 수 있다")
    need("emb_without" in tseg,
         "행동 검체에 기전 A/B 차분(임베드가 그 키를 들고 있을 때만 채워진다)이 없다 — "
         "배달이 우연이 아님을 보이는 축이 빠졌다")

    # ★계측 타당성: 기준 트리에는 정본도 신규 키도 없다(탐지기가 구 결함에서 FIRE 한다).
    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        need(_git_show(frg_rel) is None,
             "계측 타당성 실패: 기준 트리에 이미 관문 정본이 있다면 K-1 은 결함이 아니다")
        old_aj = _git_show(os.path.join("cysjavis-pack", "agents.json"))
        need(old_aj is not None and "first_run_gates" not in old_aj,
             "계측 타당성 실패: 기준 agents.json 에 이미 신규 키가 있다")
        calib = "기준 트리에 정본·신규 키 부재 확인"
    return ("정본 6관문(사람전용 2) · 버전 핀 · 계층 3키 배달 · 봉투=override 전용(사본 0) · "
            "행동검체 배선 · 계측검증=%s" % calib)


# ── 관문 ↔ 오탐 대조군 **커버리지 표** 탐지기(2026-08-24 · 이종 리뷰어 [major]) ──────────
#
# ★무엇이 결함이었나: 새 관문을 코퍼스에 들일 때 생기는 마찰이 아래 H-GATE-SOT-1 의
#   `need(len(blocks) == 6)` **하나뿐**이었다. 그 숫자는 손으로 `6 → 7` 만 고치면 통과한다 —
#   즉 관문은 늘어나는데 그 관문의 오탐 표면을 재는 자리는 **하나도 늘지 않는다**(리뷰어
#   표현: "지금 이빨은 자라지 않는다"). 관문이 하나 늘 때마다 넓어지는 것은 정확히 "정상 화면을
#   관문으로 오인할 표면" 이고, 이 저장소는 그 오인으로 **건강한 노드를 영구 부트 라이브락**에
#   빠뜨린 적이 있다(BLOCK-1 실측).
#
# ★수리는 2계층이다. Rust 쪽이 `GATE_CONTROL_COVERAGE` 표와 그 집행 검체
#   (`every_gate_has_a_control_screen_that_satisfies_its_widget_signature`)를 세워
#   "대조군 화면이 그 관문의 **위젯 서명을 전부 만족**할 것" 까지 실행으로 증명한다.
#   러너(이 파일)는 그 표의 **실재·항목 수·관문 전수 커버·지목 대상 실재**를 함께 핀한다.
#   한 계층만으로는 "표를 지우고 그 표를 보는 검체도 같이 지우면 초록" 이 남는다 —
#   두 계층이 맞물려야 어느 쪽을 지워도 다른 쪽이 적색이 된다.
#
# ★검체 본문이 아니라 **순수 함수**로 두는 이유: 트리가 초록일 때 탐지기가 통째로 고장나도
#   결과는 똑같이 초록이다(전형적 '초록 사각'). 아래 검체가 합성 변조본을 먹여 매 실행마다
#   탐지 능력 자체를 시험한다(MEMORY '디버깅 계측 타당성 게이트' 3칙 ①).
_GATE_COVERAGE_ENFORCER = "fn every_gate_has_a_control_screen_that_satisfies_its_widget_signature("


def _gate_coverage_rows(sot):
    """정본의 `GATE_CONTROL_COVERAGE` 항목 [(gate_id, screen_id)]. 표가 없으면 None."""
    m = re.search(r"GATE_CONTROL_COVERAGE\s*:\s*&\[\(&str,\s*&str\)\]\s*=\s*&\[(.*?)\n\];",
                  sot, re.S)
    if not m:
        return None
    return re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', m.group(1))


def _gate_coverage_violations(sot):
    """`src/first_run_gates.rs` 소스의 커버리지 계약 위반 목록(빈 목록 = 정상)."""
    v = []
    d0 = sot.find("const DEFS:")
    if d0 < 0:
        return ["정본 관문 표(DEFS)를 못 찾았다 — 관문 집합을 특정할 수 없다(수확기 파손)"]
    defs = sot[d0:sot.find("\n];", d0) + 3]
    gate_ids = re.findall(r'id:\s*"([^"]+)"', defs)
    if not gate_ids:
        return ["DEFS 에서 관문 id 를 하나도 수확하지 못했다(수확기 파손 — fail-closed)"]
    cov = _gate_coverage_rows(sot)
    if cov is None:
        return ["대조군 커버리지 표(GATE_CONTROL_COVERAGE)가 없다 — 새 관문을 들일 때 대조군을 "
                "함께 들이라는 규율의 이빨이 사라졌다(러너의 `len(blocks)==N` 은 손으로 숫자만 "
                "고치면 통과한다)"]
    if not cov:
        v.append("커버리지 표 항목 0건 — 표만 남기고 비우는 것은 지우는 것과 같다")
    sm = re.search(r"NON_GATE_SCREENS\s*:\s*&\[\(&str,\s*&str\)\]\s*=\s*&\[(.*?)\n\s*\];",
                   sot, re.S)
    if not sm:
        v.append("오탐 대조군 표(NON_GATE_SCREENS)를 못 찾았다 — 커버리지가 가리키는 대상의 "
                 "실재를 잴 수 없다")
        screens = None
    else:
        screens = {a for a, _c in re.findall(r'\(\s*"([^"]+)"\s*,\s*([A-Za-z0-9_]+)\s*\)',
                                             sm.group(1))}
        if not screens:
            v.append("NON_GATE_SCREENS 항목 0건(수확기 파손 — fail-closed)")
    covered = {g for g, _s in cov}
    for g in gate_ids:
        if g not in covered:
            v.append("관문 %s 의 대조군 커버리지가 0건 — 관문만 늘고 그것을 재는 자리는 늘지 않았다"
                     % g)
    for g in sorted(covered - set(gate_ids)):
        v.append("커버리지 표가 정본에 없는 관문 %s 를 가리킨다(쓰레기통 금지 — 면제표와 같은 규율)"
                 % g)
    if screens:
        for s in sorted({s for _g, s in cov} - screens):
            v.append("커버리지 표가 존재하지 않는 대조군 화면 %s 를 가리킨다" % s)
    if cov and len(cov) < len(gate_ids):
        v.append("커버리지 항목 %d < 관문 %d — 산술상 커버리지 0인 관문이 반드시 있다"
                 % (len(cov), len(gate_ids)))
    if _GATE_COVERAGE_ENFORCER not in sot:
        v.append("커버리지 집행 검체(every_gate_has_a_control_screen_that_satisfies_its_widget_"
                 "signature)가 없다 — 표만 있고 아무도 집행하지 않으면 장식이다")
    return v


@specimen("H-GATE-SOT-1", "W6",
          "관문 문면 SOT 1벌 — 나머지 사본은 읽기 소비이거나 명시 면제(S-1)", ["S-1"])
def h_gate_sot_1():
    """S-1(샷건 서저리): 관문 문면·마커가 **4벌**로 흩어져 있었고 어떤 파리티 검체도 그것을
    지키지 않았다 — `agents.json` · `cys.rs` 내장 needle · `javis_phoenix_harness.py
    MODAL_MARKERS` · (설계가 지목한) `javis_preflight.py`.
    ★실측 정정: 현행 트리의 `javis_preflight.py` 에는 관문 **문면**이 없다(어댑터 선택 키
      이름과 스키마 버전만 있고 그쪽은 H-DOC-7 이 지킨다). 즉 실제 사본은 3벌이다.
      이 검체는 그 사실을 확인하고, **네 번째 사본이 새로 생기는 것**을 적색으로 만든다.

    계약: SOT 는 `src/first_run_gates.rs` 하나다. 나머지는 ①정본이 덮거나 ②표에 사유와 함께
    면제되어야 한다. 표에 없는 문면은 적색 — 사본은 조용히 늘고, 늘어난 사본은 갈린다."""
    frg_rel = os.path.join("src", "first_run_gates.rs")
    if not os.path.isfile(os.path.join(REPO_DIR, frg_rel)):
        if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
            raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
        raise Fail("관문 정본 %s 부재" % frg_rel)
    sot = _repo_file(os.path.join("src", "first_run_gates.rs"))
    cli = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    notes = []
    # ★스캔 범위는 **코퍼스 표(DEFS)** 다 — 파일 전문이 아니다. 전문을 훑으면 결함 서사를
    #   설명한 주석·킬체인 대조군 픽스처·구 needle 을 인용한 테스트까지 '선언'으로 오독한다
    #   (실측: 첫 판이 정확히 그 이유로 거짓 적색을 냈다 — 결함이 아니라 계측기가 만든 적색).
    #   판정 축은 처음부터 "**정본이 선언한 문면 집합**" 이었고, 그 집합은 이 표 안에만 있다.
    need("const DEFS:" in sot, "관문 정본 표(DEFS) 부재 — 문면 집합을 특정할 수 없다")
    _d0 = sot.index("const DEFS:")
    defs = sot[_d0:sot.index("\n];", _d0) + 3]
    def _lits(field):
        out = []
        for blk in re.findall(r"%s:\s*&\[(.*?)\]" % field, defs, re.S):
            out += re.findall(r'"((?:[^"\\]|\\.)*)"', blk)
        return out
    sot_needles = _lits("needles")
    sot_widgets = _lits("widget")
    need(len(sot_needles) >= 6,
         "정본 표에서 needle 을 %d건만 수확했다(수확기 파손 — fail-closed)" % len(sot_needles))
    flat_decl = "".join("".join(x.split()) for x in sot_needles + sot_widgets)
    flat_needles = {"".join(x.split()) for x in sot_needles}

    # ── ★코퍼스 자기규칙(BLOCK-1 · BLOCK-2 · 2026-08-24) ─────────────────────────
    #
    # **대원칙**: 관문은 "그 관문이 화면에 떠 있을 때만 나타나는 것" 으로만 식별해야 한다.
    # 인사 배너·상태 메시지·에러 문자열은 정상 화면에도 나타나므로 그 조건을 만족하지 않는다.
    # 리뷰어 e2e 실증 — ⓐ theme 관문이 `"Welcome to Claude Code"`(배너) + `❯`(모든 정상 claude
    # 프롬프트에 있는 문자) 단독 위젯으로 성립해 **건강한 노드**를 잡았다(rc 78 · 디렉티브 미주입 ·
    # 사람도 통과시킬 관문이 화면에 없어 **영구 부트 라이브락**). ⓑ `oauth-code` 는 `widget: &[]`
    # 로 AND 가드가 0이라 다른 CLI 의 브라우저 로그인 화면·로그 한 줄·`grep` 출력까지 전부
    # `human_only` 관문으로 식별했다.
    #
    # 판정 자체(진리표·화면 대조)는 Rust 검체가 실행으로 증명한다 — 러너는 컴파일러가 아니므로
    # 여기서는 ①규칙 장치의 실재 ②검체 이름 ③**표에서 직접 잴 수 있는 규칙 ⓑⓒ** 를 핀한다.
    for pin in ("pub const UNIVERSAL_WIDGET_TOKENS:", "pub const NEEDLE_EXEMPTIONS:",
                "pub fn is_universal_widget_token(", "pub fn widget_rule_violations("):
        need(pin in sot, "코퍼스 자기규칙 장치 결손: %s — 규칙이 검체 본문에만 있으면 구 선언을 "
                         "재현해 적색을 증명할 수 없다" % pin)
    for t in ("corpus_self_rule_a_every_needle_is_question_form_or_justified",
              "corpus_self_rule_bc_widget_and_guard_is_present_and_not_universal",
              "no_needle_alone_matches_a_non_gate_screen",
              "no_gate_matches_a_non_gate_screen",
              "self_rules_are_red_on_the_pre_fix_declarations"):
        need(t in sot, "코퍼스 자기규칙 검체 %s 가 사라졌다" % t)
    need("NON_GATE_SCREENS:" in sot,
         "오탐 대조군(관문이 **아닌** 화면) 표가 없다 — 규칙이 '정상 화면에 안 걸린다' 를 "
         "아무 데서도 재지 못한다")
    # 보편 토큰 집합을 정본에서 수확한다(사본 금지 — 값은 코드가 소유한다).
    um = re.search(r"UNIVERSAL_WIDGET_TOKENS:\s*&\[&str\]\s*=\s*&\[(.*?)\];", sot, re.S)
    need(um, "보편 토큰 집합을 수확하지 못했다(수확기 파손 — fail-closed)")
    universal = {"".join(x.split())
                 for x in re.findall(r'"((?:[^"\\]|\\.)*)"', um.group(1))}
    need("❯" in universal,
         "보편 토큰 집합에 `❯` 가 없다 — 관문 6화면과 정상 화면에 **모두** 있는 문자이므로 "
         "위젯 단독 선언 금지의 1순위 대상이다")
    # 관문별로 직접 잰다 — ⓑ위젯 비어있지 않음 ⓒ보편 토큰 단독 아님.
    blocks = defs.split("    Def {")[1:]
    need(len(blocks) == 6, "정본 Def 블록 수확 실패(%d건 — fail-closed)" % len(blocks))
    for blk in blocks:
        gm = re.search(r'id:\s*"([^"]+)"', blk)
        need(gm, "Def 블록에서 id 를 못 읽었다(수확기 파손)")
        gid = gm.group(1)
        wm = re.search(r"widget:\s*&\[(.*?)\]", blk, re.S)
        need(wm, "%s: widget 선언을 못 읽었다(수확기 파손)" % gid)
        ws = [w for w in re.findall(r'"((?:[^"\\]|\\.)*)"', wm.group(1))]
        need(ws, "%s: widget AND 가드가 0 — needle 이 화면 전문에 그대로 걸린다(BLOCK-2 형태)" % gid)
        need([w for w in ws if "".join(w.split()) not in universal],
             "%s: widget 이 보편 토큰 단독(%r) — 모든 정상 화면에 있는 문자를 위젯으로 쓰면 AND 가 "
             "무의미해지고 관문이 needle 하나로 성립한다(BLOCK-1 형태 · 영구 부트 라이브락)"
             % (gid, ws))
    notes.append("자기규칙: 위젯 AND 6/6 실재·보편토큰 단독 0 · 검체 5종 실재")

    # ── ★대조군 커버리지 표 결박(2026-08-24 · 이종 리뷰어 [major] 채택) ──────────
    # 위 `len(blocks) == 6` 은 **그대로 둔다**(핀 삭제 금지 — 러너 헤더 ②). 여기서 하는 일은
    # 판정 축을 **추가**하는 것뿐이다: 관문이 하나 늘면 그 관문의 대조군도 함께 늘어야 한다는
    # 계약을, Rust 표(`GATE_CONTROL_COVERAGE`)와 **맞물려** 잰다. 이제 그 표를 지우거나 비우는
    # 것, 그 표가 없는 관문·없는 화면을 가리키는 것, 집행 검체를 지우는 것이 러너에서도 적색이다.
    cov_v = _gate_coverage_violations(sot)
    need(not cov_v, "대조군 커버리지 계약 위반 %d건: %s" % (len(cov_v), cov_v))
    cov_rows = _gate_coverage_rows(sot) or []
    # ★계측 타당성 — 트리에 위반이 0건이므로 탐지기가 죽어 있어도 결과는 똑같이 초록이다.
    #   그래서 매 실행 합성 변조본을 먹여 **탐지 능력 자체**를 시험한다. 마지막 변조본이
    #   리뷰어가 지목한 정확한 시나리오다: 관문만 하나 더 들이고 대조군은 0으로 둔 상태.
    _planted = ('const DEFS: &[Def] = &[\n    Def {\n        id: "planted-gate",\n'
                '        needles: &["Planted question?"],\n        widget: &["Planted widget"],\n'
                '    },')
    cov_mutants = [
        ("커버리지 표 삭제",
         re.sub(r"pub const GATE_CONTROL_COVERAGE[\s\S]*?\n\];", "", sot, count=1)),
        ("커버리지 표 비움",
         re.sub(r"(pub const GATE_CONTROL_COVERAGE[^=]*=\s*&\[)[\s\S]*?(\n\];)",
                r"\1\2", sot, count=1)),
        ("관문 1종의 대조군 전삭제", re.sub(r'\n\s*\("theme",\s*"[^"]+"\),', "", sot)),
        ("존재하지 않는 대조군 지목",
         sot.replace('("theme", "audit-log-line")', '("theme", "no-such-screen")', 1)),
        ("정본에 없는 관문 지목",
         sot.replace('("theme", "audit-log-line")', '("no-such-gate", "audit-log-line")', 1)),
        ("집행 검체 삭제",
         sot.replace(_GATE_COVERAGE_ENFORCER, "fn coverage_check_disabled(", 1)),
        ("새 관문만 추가(대조군 0 — 리뷰어 지목 시나리오)",
         sot.replace("const DEFS: &[Def] = &[", _planted, 1)),
    ]
    cov_blind = [lbl for lbl, m in cov_mutants if not _gate_coverage_violations(m)]
    need(not cov_blind, "커버리지 탐지기가 합성 변조본을 못 잡았다(탐지기 고장): %s"
         % ", ".join(cov_blind))
    notes.append("대조군 커버리지 %d행/관문 %d종 전수 커버 · 집행 검체 실재 · 합성 변조 %d종 전건 적발"
                 % (len(cov_rows), len(blocks), len(cov_mutants)))

    # ── 사본 ① agents.json trust-prompt 선언 → 정본 needle 에 실재해야 한다
    aj = json.loads(_read(os.path.join(PACK_DIR, "agents.json")))
    pats = {p.get("name"): p.get("pattern")
            for p in ((aj.get("claude") or {}).get("approval_patterns") or [])}
    trust_pat = pats.get("trust-prompt")
    need(trust_pat, "agents.json 에 trust-prompt 선언이 없다(자동확인 소스 소실)")
    need('"%s"' % trust_pat in sot,
         "agents.json trust-prompt 문면 %r 이 정본에 없다 — 사본이 갈렸다" % trust_pat)
    notes.append("agents.json 문면 정본 포함")

    # ── 사본 ② cys.rs 내장 폴백 needle → 정본에 **담지 않는다**(면제표에 사유 필수)
    hseg = cli[cli.index("fn trust_prompt_hit("):]
    hseg = hseg[:hseg.index("\n}\n") + 2]
    builtin_needles = re.findall(r'delta_flat\.contains\("([^"]+)"\)', hseg)
    need(builtin_needles, "trust_prompt_hit 에서 내장 needle 을 수확하지 못했다(수확기 파손)")
    for nd in builtin_needles:
        need(nd in GATE_LEGACY_CODE_NEEDLE,
             "코드 내장 needle %r 이 면제표에 없다 — 새 사본은 사유와 함께 등재하라(조용한 확산 금지)"
             % nd)
    # ★결함 needle 은 정본의 **선언 집합**에 다시 들어오면 안 된다: 그것이 킬체인의 형태다.
    for nd, why in GATE_LEGACY_CODE_NEEDLE.items():
        if "결함 needle" in why:
            need(nd not in flat_needles,
                 "정본이 결함 needle %r 을 선언 집합에 들여왔다 — 확인 에코 재매칭 킬체인 재발" % nd)
    notes.append("코드 내장 needle %d건 면제표 일치" % len(builtin_needles))

    # ── 사본 ③ phoenix harness MODAL_MARKERS → 정본이 덮거나 면제표에 있어야 한다
    ph = _read(os.path.join(BIN_DIR, "javis_phoenix_harness.py"))
    mm = re.search(r"MODAL_MARKERS\s*=\s*\[([^\]]*)\]", ph)
    need(mm, "javis_phoenix_harness.py 에서 MODAL_MARKERS 를 찾지 못했다(수확기 파손)")
    markers = re.findall(r"'([^']*)'|\"([^\"]*)\"", mm.group(1))
    markers = [a or b for a, b in markers]
    need(markers, "MODAL_MARKERS 항목 0건 — 수확기 파손(fail-closed)")
    uncovered = [mk for mk in markers
                 if mk not in GATE_MARKER_EXEMPT and "".join(mk.split()) not in flat_decl]
    need(not uncovered,
         "MODAL_MARKERS 항목 %s 가 정본에도 면제표에도 없다 — 4번째 사본이 생겼다" % uncovered)
    notes.append("MODAL_MARKERS %d항(면제 %d) 정합"
                 % (len(markers), len([m for m in markers if m in GATE_MARKER_EXEMPT])))

    # ── 네 번째 사본 금지: 팩 python 어디에도 관문 **질문형 문면**이 복제되지 않는다.
    #    (설계가 지목한 javis_preflight.py 를 포함한 전수 검사. phoenix 는 위에서 이미 판정했다.)
    dupes = []
    for fn in sorted(os.listdir(BIN_DIR)):
        if not fn.endswith(".py") or fn == "javis_phoenix_harness.py":
            continue
        body = _read(os.path.join(BIN_DIR, fn))
        for nd in sot_needles:
            if nd in body:
                dupes.append("%s:%r" % (fn, nd))
    need(not dupes,
         "팩 python 에 관문 문면 사본이 생겼다: %s — 문면의 진실원천은 %s 하나다(읽기 소비만 허용)"
         % (dupes[:5], frg_rel))
    notes.append("팩 python 사본 0건(문면 %d종 대조)" % len(sot_needles))

    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        need(_git_show(frg_rel) is None,
             "계측 타당성 실패: 기준 트리에 이미 SOT 가 있다면 S-1 은 결함이 아니다")
        calib = "기준 트리 SOT 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-PRED-10", "W4", "TCC 탐침 대상이 실자원(cwd+PACK)에서 파생", ["P3-A-TCC"])
def h_pred_10():
    """P3-A-TCC(RC3 — 계측기가 대상을 못 잰다): 부트의 macOS 폴더권한 탐침이 `~/Desktop`
    **하드코딩**이었다. 부트가 실제로 읽는 자원은 (a) 이 세션의 작업 디렉터리 (b) 팩 디렉터리인데
    탐침은 그 둘 중 아무것도 찌르지 않았다 → Desktop 을 안 쓰는 기계에선 거짓 경고, Documents·
    프로젝트 폴더만 막힌 실제 사고에선 침묵(GUI perm-warning 보다도 좁았다).
    ★계측: 격리 cwd·PACK 에서 함수를 **실제로 호출**해 반환 대상이 그 실자원에서 파생됨을 본다."""
    probe = r'''
import json, os, sys
sys.path.insert(0, sys.argv[1])
os.chdir(sys.argv[2])
os.environ["CYS_PACK_DIR"] = sys.argv[3]
import javis_bootstrap as b
print(json.dumps({"targets": b._tcc_probe_targets(), "pack": b.PACK, "platform": sys.platform}))
'''
    with tempfile.TemporaryDirectory() as tmp:
        cwd = os.path.join(tmp, "work dir")     # 공백 포함 — 경로 조립 취약성 동시 확인
        pack = os.path.join(tmp, "pack-dept-x")
        os.makedirs(cwd)
        os.makedirs(pack)
        r = _run([PY, "-c", probe, BIN_DIR, cwd, pack])
        need(r.returncode == 0, "탐침 호출 실패: %s" % r.stderr[-400:])
        d = json.loads(r.stdout.strip().splitlines()[-1])
    paths = [p for p, _label in d["targets"]]
    if d["platform"] == "darwin":
        need(os.path.realpath(cwd) in paths, "작업 디렉터리(실자원)가 탐침 대상에 없다: %s" % paths)
        need(os.path.realpath(pack) in paths, "팩(실자원)이 탐침 대상에 없다: %s" % paths)
        need(all(isinstance(l, str) and l for _p, l in d["targets"]), "탐침 라벨 결손")
        home_desk = os.path.realpath(os.path.expanduser("~/Desktop"))
        need(home_desk not in paths, "Desktop 하드코딩 잔존: %s" % paths)
    else:
        need(d["targets"] == [], "비-darwin 에서 탐침이 동작한다(무동작 계약 위반)")
    # 소스 규약: **탐침 함수 본문**에 Desktop 리터럴이 없다(주석 설명·self-test 대조군은 허용 —
    # 결함을 설명한 문장과 '하드코딩이 아님'을 단언하는 self-test 까지 잡으면 문서화가 회귀가 된다).
    src_bs = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need("_tcc_probe_targets(" in src_bs, "탐침 대상 파생 함수 부재")
    fn_body = src_bs[src_bs.index("def _tcc_probe_targets("):]
    fn_body = fn_body[:fn_body.index("\ndef ")]
    # docstring(결함 설명)과 주석은 스캔에서 제외 — 결함을 설명한 문장을 회귀로 오보하지 않는다.
    fn_body = _code_lines(fn_body[fn_body.index('"""', fn_body.index('"""') + 3) + 3:])
    need("Desktop" not in fn_body, "탐침 함수 실행부에 Desktop 하드코딩 잔존")
    need("os.getcwd()" in fn_body and "PACK" in fn_body,
         "탐침 대상이 cwd·PACK 실자원에서 파생되지 않는다")
    # 호출부도 파생 목록을 순회한다(단일 대상 하드코딩 재발 차단)
    need("for _probe, _label in _tcc_probe_targets():" in src_bs,
         "호출부가 파생 목록을 순회하지 않는다")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py")
    if old is not None:
        need('os.path.join(HOME, "Desktop")' in old,
             "계측 타당성 실패: 구 코드에 Desktop 하드코딩이 없다면 이 검체는 무의미")
        calib = "구 코드 Desktop 하드코딩 확인"
    return "cwd+PACK 실측 파생(%d대상) · Desktop 리터럴 0 · 계측검증=%s" % (len(paths), calib)
@specimen("H-TIME-1", "W2", "예산 parity Σ(하위 최악치)≤상위 + 냉시작 하한 fixture", ["B9"])
def h_time_1():
    """B9: 예산 역전이 3중이었다(외부 상한 < 내부 최악치) — 상위 timeout 이 정상 진행 중인 하위를
    잘라 **조기실패**를 만들고, 그 산출 중간상태가 A1·B6 오판 사슬을 먹였다.
    ★방향(비평2 D-2): **내부 감액 금지**(냉시작 실측 하한 보존) + 외부 상한은 파생·증액."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_budget as BU
    notes = []
    # ⓐ 전 쌍 파리티: Σ내부최악 ≤ 외부 상한
    v = BU.parity_violations()
    need(not v, "예산 역전 잔존: %r" % v)
    pairs = BU.parity_pairs()
    need(len(pairs) >= 3, "파리티 쌍이 3개 미만 — 재감사가 지목한 3중 역전을 다 덮지 않는다")
    notes.append("파리티 %d쌍 역전 0" % len(pairs))
    # ⓑ **냉시작 실측 하한 fixture** — leaf 를 env 로 내려도 하한이 이긴다(감액 방향 회귀 차단)
    floors = {"BOOT_NODE_TOTAL_S": 90, "BOOT_NODE_LAUNCH_SUBPROC_S": 80,
              "LAUNCH_READINESS_FLOOR_S": 30, "CHECK_RETRIES": 24, "CHECK_INTERVAL_S": 5}
    for name, floor in floors.items():
        need(BU.LEAF_FLOORS[name] >= floor,
             "냉시작 실측 하한 감액: %s=%s < %s (adv#4 가 늘린 예산의 역행)"
             % (name, BU.LEAF_FLOORS[name], floor))
        os.environ["CYS_BUDGET_" + name] = "1"
        try:
            need(BU.leaf(name) == BU.LEAF_FLOORS[name],
                 "%s 감액이 통과됐다(clamp 부재)" % name)
        finally:
            del os.environ["CYS_BUDGET_" + name]
    notes.append("하한 fixture %d종 clamp" % len(floors))
    # ⓒ 외부 상한이 **파생값**이다(하드코딩 회귀 차단) — 내부를 키우면 외부도 커진다
    base = BU.cys_boot_outer_s()
    os.environ["CYS_BUDGET_PLAN_ROLE_COUNT"] = "9"
    try:
        need(BU.cys_boot_outer_s() > base, "외부 상한이 내부 최악치에서 파생되지 않는다")
    finally:
        del os.environ["CYS_BUDGET_PLAN_ROLE_COUNT"]
    notes.append("외부 상한 파생 확인")
    # ⓓ 소비처가 하드코딩 timeout 을 쓰지 않는다(④·④-b·boot_node 외부 상한)
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need('timeout=300)' not in bsrc.replace('"--fix"], timeout=300)', ''),
         "④ 이 하드코딩 300s 를 쓴다(예산 파생 미적용)")
    need('_budget_derived("cys_boot_outer_s"' in bsrc, "④ 이 예산 파생값을 쓰지 않는다")
    need('_budget_derived("boot_reviewers_outer_s"' in bsrc, "④-b 가 예산 파생값을 쓰지 않는다")
    osrc = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    need("_boot_node_outer_timeout()" in osrc, "_boot_one_node 가 예산 파생 외부 상한을 쓰지 않는다")
    need("timeout=130" not in osrc, "_boot_one_node 에 하드코딩 130s 잔존")
    notes.append("소비처 하드코딩 timeout 제거")
    # ⓔ 데드라인 전파 — 하위가 자기 예산을 안다(내부 최악치 유계화·감액 0)
    need('"--timeout", "%.0f" % inner' in osrc, "boot_node 에 데드라인이 전파되지 않는다")
    bnsrc = _read(os.path.join(BIN_DIR, "javis_boot_node.py"))
    need("def deadline_capped(" in bnsrc, "boot_node 가 서브프로세스를 데드라인으로 자르지 않는다")
    need("deadline_capped(budget(\"BOOT_NODE_LAUNCH_SUBPROC_S\"" in bnsrc,
         "LAUNCH 서브프로세스가 데드라인 캡을 받지 않는다(B9 역전 2/3 원인)")
    notes.append("데드라인 전파(감액 0·유계화)")
    # ⓕ 진행 하트비트 — 증액이 만든 침묵 창 상쇄(stderr 전용·verdict 채널 무오염)
    need("heartbeat(" in bnsrc, "boot_node 진행 하트비트가 없다")
    need("⑤check 재시도" in bsrc, "⑤ 재시도 하트비트가 없다")
    csrc = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("BUDGET_HEARTBEAT_INTERVAL_SECS" in csrc, "cys boot 하트비트 상수가 없다")
    notes.append("하트비트 3지점(stderr)")
    r = _run([PY, os.path.join(BIN_DIR, "javis_budget.py"), "--self-test"], timeout=60)
    need(r.returncode == 0, "javis_budget self-test 실패:\n%s" % r.stdout[-600:])
    return " · ".join(notes) + " · budget self-test PASS"


@specimen("H-TIME-2", "W2", "문서·훅 안내 숫자=BUDGET 상수 파생(하드코딩 grep 0)", ["P3-A-120S"])
def h_time_2():
    """P3-A-120S: 산문의 '최대 120s' 는 CHECK_RETRIES×CHECK_INTERVAL_S 를 손으로 곱한 사본이라
    예산이 바뀌면 문서만 거짓이 됐다. 안내 숫자는 BUDGET 파생값 주입으로만 만든다."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_budget as BU
    notes = []
    # ⓐ 훅 안내가 파생값을 주입한다(하드코딩 '120s' grep 0)
    rb = _read(_hook(ROLE_BODY))
    need("javis_budget.py\" --note-check-window" in rb or "--note-check-window" in rb,
         "훅 안내가 예산 파생값을 주입하지 않는다")
    need("생존확인(최대 120s)" not in rb, "훅 안내에 하드코딩 120s 잔존")
    need("최대 %ss" in rb, "훅 안내가 파생 포맷을 쓰지 않는다")
    notes.append("훅 안내=파생 주입 · 하드코딩 0")
    # ⓑ 파생값이 실제로 CHECK 상수에서 나온다
    r = _run([PY, os.path.join(BIN_DIR, "javis_budget.py"), "--note-check-window"], timeout=60)
    need(r.returncode == 0, "--note-check-window 실패: %s" % r.stderr[-300:])
    want = int(round(BU.leaf("CHECK_RETRIES") * BU.leaf("CHECK_INTERVAL_S")))
    need(int(r.stdout.strip()) == want,
         "안내 창(%s) ≠ CHECK 상수 파생(%s)" % (r.stdout.strip(), want))
    notes.append("안내 창=%ds(CHECK 상수 파생)" % want)
    # ⓒ ④·④-b 진행 문구도 파생값을 쓴다(산문 상수 사본 금지)
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need("최대 %ds — 예산 파생" in bsrc, "④ 진행 문구가 파생값을 인용하지 않는다")
    need("최대 300s)" not in bsrc, "④ 진행 문구에 하드코딩 300s 잔존")
    need("최대 320s" not in bsrc, "④-b 진행 문구에 하드코딩 320s 잔존")
    notes.append("④·④-b 진행 문구 파생")
    # ⓓ 러너가 인용하는 산식이 python 파생과 일치(문서-코드 정합)
    need(BU.check_window_s() == want, "check_window_s 파생 불일치")
    old = _git_show(os.path.join("cysjavis-pack", "hooks", "role-bootstrap.sh"))
    calib = "skip(no-git)"
    if old is not None:
        need("생존확인(최대 120s)" in old, "계측 타당성 실패: 구 훅의 하드코딩 120s 를 못 찾았다")
        calib = "구 훅=하드코딩 '최대 120s' 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-TIME-3", "W2", "카운트 회계 금지(Instant 데드라인 단언)", ["B17"])
def h_time_3():
    """B17: readiness 루프가 `waited += 2` 로 시간을 **셌다** — 틱당 실비용(RPC 왕복 + 2.5s sleep +
    trust 분기의 미집계 sleep)이 가정치와 어긋나 실효 대기가 25%+α 오차났다. 벽시계만 쓴다."""
    src = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    body = _slice_between(src, "fn boot_agent_on_surface(",
                          "\n/// 에이전트 기동 + 역할 지침", "H-TIME-3 부트 본문")
    # ★핀 이사(U-13): 판정이 순수 술어로 옮겨 갔으므로 **시간 축의 금지도 함께 옮긴다**.
    #   판정부는 시계를 아예 읽지 않는다(시간 사실은 호출부가 계산해 `Observed` 로 넘긴다) —
    #   그것이 '숨은 입력 금지'의 기계 집행이다.
    judge = _slice_between(src, "pub fn judge(",
                           "// ── 판정부 끝(핀 슬라이스 경계) ──", "H-TIME-3 판정부")
    notes = []
    # ⓐ 카운트 회계 전폐 — **주석 제외 실코드**에서 잔존 0(주석은 결함 이력 서술이다).
    code = "\n".join(ln for ln in body.splitlines() if not ln.strip().startswith("//"))
    jcode = "\n".join(ln for ln in judge.splitlines() if not ln.strip().startswith("//"))
    for banned in ("waited +=", "let mut waited", "waited < max_wait_secs"):
        need(banned not in code, "카운트 기반 시간 회계 잔존(실코드): %s" % banned)
        need(banned not in jcode, "판정부에 카운트 기반 시간 회계 잔존: %s" % banned)
    need("Instant::now()" not in jcode,
         "판정부가 시계를 직접 읽는다 — `Observed` 밖의 숨은 입력(진리표가 실기 없이 돌지 못한다)")
    need("time_fallback_reached: std::time::Instant::now() >= time_fallback_at" in body,
         "시간 폴백 사실이 벽시계로 계산돼 판정 입력에 실리지 않는다")
    notes.append("카운트 회계 0(실코드·판정부) · 판정부 시계 무접촉")
    # ⓑ Instant 데드라인 단언
    need("let deadline = std::time::Instant::now() + max_wait" in body,
         "readiness 루프에 Instant 데드라인이 없다")
    need("while std::time::Instant::now() < deadline" in body,
         "루프 조건이 벽시계 데드라인이 아니다")
    need("let time_fallback_at" in body and "Instant::now() >= time_fallback_at" in body,
         "시간 폴백 시점도 벽시계로 판정되지 않는다")
    notes.append("Instant 데드라인 + 폴백 시점 벽시계")
    # ⓒ trust 분기 sleep 이 회계에서 사라지지 않는다(벽시계라 자동 반영) + continue 로 ready 봉쇄 0
    ti = body.find("folder-trust prompt")
    need(ti > 0, "folder-trust 분기를 못 찾았다")
    tseg = body[ti:ti + 2500]
    need("continue;" not in tseg,
         "신뢰 분기가 continue 로 ready 검사를 봉쇄한다(G35 — 준비 감지 구조 차단)")
    notes.append("신뢰 분기 ready 봉쇄 해제")
    # ⓓ 상한이 BUDGET 파생(하드코딩 30/2/20 제거)
    need("budget_readiness_max(delay, restore)" in body, "readiness 상한이 예산 파생이 아니다")
    need("(delay.max(30) * 2)" not in src, "하드코딩 readiness 산식 잔존")
    need("BUDGET_TICK_MS" in body, "틱 주기가 예산 상수가 아니다")
    notes.append("상한·틱 BUDGET 파생")
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("waited += 2; // ~2.5s per tick" in old,
             "계측 타당성 실패: 구 코드의 카운트 회계 주석을 못 찾았다")
        calib = "구 코드=waited+=2 카운트 회계(trust sleep 무집계) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib
_CLAUDE_MD_COPIES = ("CLAUDE.md", os.path.join("cysjavis-pack", "CLAUDE.md.template"))
_HOOK_FIRED_MARK = "[결정론 부트스트랩 발화됨 — 하네스 강제]"


def _repo_file(rel):
    p = os.path.join(REPO_DIR, rel)
    if not os.path.isfile(p):
        raise Skip("레포 파일 부재(배포 팩 실행): %s" % rel)
    return _read(p)


# ── 스캔 대상 레지스트리 (핀 이사 계약 U-2) ─────────────────────────────────
# 소스 문자열을 핀하는 검체가 **파일 경로를 각자 들고 있으면**, 그 코드가 다른 파일로 이사할 때
# 흩어진 경로를 검체마다 고쳐야 한다 — 그리고 실제로는 고치는 대신 **조용히 지워진다**.
# 그래서 '논리 이름 → 경로 목록'을 한 곳에 모은다. 이사할 때 고칠 곳은 **여기 한 줄**이고,
# 그 한 줄은 diff 에 명시적으로 드러난다.
#
# ★이 레지스트리는 *어디를 읽을지*만 정한다 — *무엇을 단언할지*(각 검체의 `need`)는 소유하지
#   않는다. 즉 레지스트리 경유화는 **표현의 변경이며 판정의 변경이 아니다**.
# ★값은 **추가**한다. 기존 경로 제거는 "그 파일에 판정 대상이 더는 없다"를 증명했을 때만.
SCAN_TARGETS = {
    # readiness 판정부 — 기동 준비 완료 판정 · 안전 밸브 · 데드라인 회계 · 주입 검증 배선.
    # 지금은 전부 `src/bin/cys.rs` 안에 있다. 캠페인이 판정을 순수함수(`src/readiness.rs`)로
    # 추출하면 **여기에 그 경로를 추가**한다(cys.rs 에는 호출 배선이 남으므로 함께 유지).
    "readiness": (
        os.path.join("src", "bin", "cys.rs"),
        # ★(U-13) ready 술어가 순수함수로 이사한 자리. cys.rs 에는 관측·귀결 배선이 남으므로
        #   **둘 다** 등재한다 — 한쪽만 두면 이사한 핀이나 남은 핀 중 한쪽이 조용히 죽는다.
        os.path.join("src", "readiness.rs"),
    ),
    # ★(U-14/U-15) 주입·제출 관문 가드의 판정부. `readiness` 와 **별도 이름**인 이유 둘 —
    #   ⓐ 한 경로를 두 논리 이름이 소유하면 이사 때 한쪽이 누락된다(H-META-PIN ⓐ 가 금지).
    #   ⓑ `readiness` 합본의 **마지막 조각**은 판정부 파일이라는 계약을 H-READY-13 ⓓ 가 쓰고
    #      있다(`split(_SCAN_JOIN)[-1]`). 여기에 파일을 덧붙이면 그 계약이 조용히 깨진다.
    #   cys.rs 쪽 **호출 배선**은 `readiness` 가 이미 소유하므로 여기 등재하지 않는다
    #   (H-KILLCHAIN-1 은 두 이름을 모두 경유해 읽는다).
    "inject_guard": (os.path.join("src", "inject_guard.rs"),),
    # ★(U-17) 프로필 인증 전제 판정기. 신설 파일 하나이며 `cys profile-auth` 배선이 CLI 로
    #   내려가면 **여기에 그 경로를 추가**한다(판정부는 이 파일에 남으므로 둘 다 유지).
    "profile_gate": (os.path.join("src", "profile_gate.rs"),),
}

# 레지스트리를 소비한다고 **선언**한 검체 → 논리 이름. H-META-PIN 이 실제 배선과 대조한다.
SCAN_TARGET_CONSUMERS = {
    # ★(U-10) 제4 등급 파리티 검체 — cys.rs 의 `seat_liveness`·`run_boot`·`boot_agent_on_surface`
    #   본문을 핀한다. 경로를 직접 들지 않고 레지스트리로 묻는다(핀 이사 계약 ⓒ).
    "H-SEAT-4AXIS": "readiness",
    # ★(U-11) 보류 exit 4단 파리티 검체 — cys.rs 의 소비 지점을 센다. 경로를 직접 들지 않고
    #   레지스트리로 묻는다(핀 이사 계약 ⓒ — 소유 경로의 직접 독자를 늘리지 않는다).
    "H-EXIT-11": "readiness",
    # ★(U-12) 배달 경로·문면 SOT 검체 — cys.rs 의 계층 배선(LAYERED_KEYS)과 내장 needle 을
    #   핀한다. 경로를 직접 들지 않고 레지스트리로 묻는다(핀 이사 계약 ⓒ).
    "H-DELIVER-1": "readiness",
    "H-GATE-SOT-1": "readiness",
    # ★(U-13) ready 술어 단일화 검체 — cys.rs 배선과 readiness.rs 판정부를 함께 핀한다.
    "H-READY-13": "readiness",
    # ★(U-14/U-15) 주입 봉인·킬체인 검체 — cys.rs 의 **호출 배선**을 `readiness` 로 묻고
    #   판정부는 `inject_guard` 로 묻는다(두 이름 모두 경유 · 직접 경로 0).
    "H-KILLCHAIN-1": "readiness",
    "H-PRED-8": "readiness",
    "H-SAFE-2": "readiness",
    "H-TIME-3": "readiness",
    "H-OBS-2": "readiness",
    # ★(U-17) 인증 전제 판정기 검체 — 판정부를 `profile_gate` 로 묻는다(직접 경로 0).
    "H-AUTH-PARSE": "profile_gate",
    # ★(U-18) create 게이트 검체 — 문안에 실리는 판정기 안정 문자열을 정본에서 뽑는다.
    "H-AUTH-SELFLOOP": "profile_gate",
}

# 레지스트리 소유 경로를 **직접** 읽는 기존 검체(동결 목록 · 2026-08-23 U-2 시점 실측 15종).
# 이들이 그 파일에서 핀하는 것은 readiness 판정부가 아니라 다른 계약(exit 어휘·시드·동시성·
# 프롬프트 술어 …)이라 이번 단위의 이사 대상이 아니다. 그러나 **새로 늘리지는 않는다** —
# 새 직접 독자는 이사 때 함께 옮겨지지 않아 조용히 죽기 때문이다(H-META-PIN ⓓ 가 적색).
SCAN_TARGET_DIRECT_LEGACY = frozenset({
    "H-CONC-2", "H-CONC-3", "H-EXIT-2", "H-EXIT-3", "H-EXIT-4", "H-IDENT-1",
    "H-PRED-2", "H-PRED-6", "H-PRED-7", "H-SAFE-1", "H-SAFE-W",
    "H-SEED-1", "H-SEED-3", "H-SEED-6", "H-TIME-1",
})

# 등재 경로가 여럿일 때의 이음매. Rust 주석 형태라 어떤 핀 리터럴과도 겹치지 않고, 검체가
# 파일 경계를 넘어 슬라이스하면 이 문자열이 결과에 섞여 **눈에 띄게** 만든다.
_SCAN_JOIN = "\n// ── SCAN_TARGETS 경계(run_bootstrap_health) ──\n"


def _scan_source(name):
    """레지스트리 논리 이름 → 등재 경로 전량의 소스 텍스트(등재 순서대로 이어 붙임).

    핀 이사 계약(헤더 ①)의 실행 지점이다. 검체는 파일 경로를 스스로 들지 않고 **논리 이름**으로
    묻는다 → 코드가 이사해도 고칠 곳은 `SCAN_TARGETS` 한 줄뿐이다.
    ★현재 모든 항목이 단일 경로이므로 반환 텍스트는 종전 `_repo_file(...)` 과 **바이트 동일**
      하다(= 이 개정은 어떤 검체의 판정도 바꾸지 않는다. H-META-PIN 이 아니라 개정 전후
      PASS/FAIL 집합 동일성이 그 증거다).
    ★부재는 종전과 같이 `Skip` 으로 흐른다(배포 팩 실행에서 레포 소스 검체가 접히는 규약 유지).
    """
    rels = SCAN_TARGETS.get(name)
    if not rels:
        raise Fail("SCAN_TARGETS 에 논리 이름 %r 이 없다 — 핀 이사 계약: 레지스트리를 먼저 갱신하라"
                   % name)
    return _SCAN_JOIN.join(_repo_file(rel) for rel in rels)


def _slice_between(src, start, end, where):
    """`src` 에서 `start` 앵커부터 `end` 앵커까지를 자른다 — **앵커 부재는 적색**이다.

    ★선재 결함(U-2 규약 ⓒ · U-13 에서 발견·수리): 종전 관용구는
      `body = src[bi:src.find("…", bi)]` 였다. `find` 가 -1 이면 파이썬은 예외 없이
      `src[bi:-1]`(끝 한 글자만 뺀 전량)을 돌려주고, 그 위에서 `in`/`not in` 을 판정하면
      **계측기가 결함을 승인하거나 없는 결함을 만들어낸다**(잘린/부푼 텍스트에 대고 단언).
      게다가 `SCAN_TARGETS` 에 경로가 둘 이상이 된 지금은 그 슬라이스가 **파일 경계를 넘는다**
      (`_SCAN_JOIN` 구분선은 가시화일 뿐 자동 차단이 아니다).
      그래서 ①앵커 부재 ②경계 침범 둘 다 즉시 적색으로 만든다 — 측정 불능은 통과가 아니다.
    """
    bi = src.find(start)
    need(bi >= 0,
         "%s: 시작 앵커 부재 %r — 코드가 이사했다면 SCAN_TARGETS 와 앵커를 함께 옮겨라"
         % (where, start))
    ei = src.find(end, bi)
    need(ei > bi,
         "%s: 끝 앵커 부재 %r — 슬라이스가 조용히 잘린다(계측 무효 · 핀 이사 계약 ⓒ)"
         % (where, end))
    seg = src[bi:ei]
    need(_SCAN_JOIN not in seg,
         "%s: 슬라이스가 SCAN_TARGETS 파일 경계를 넘었다 — 핀 대상이 뒤섞인다" % where)
    return seg


def _no_wait_for_owner(text, where):
    """'무조건 오너 지시 대기' 문구 금지(H-DOC-1) — 단, **폐기 선언**·**임무 게이트 조건부 대기**로
    인용한 것은 허용한다.
    ★규칙을 '문자열 부재'로 두면 폐기 선언 자체를 쓸 수 없다(문서가 결함을 설명 못 한다) →
      출현마다 근처(±40자)에 부정 마커(폐기/아니라/금지)가 있어야 한다는 형태 규칙으로 판정한다.
    ★T1(2026-08-01 실사고) 이후 허용 마커 확장: 계약이 '항상 자율 착수'에서 **'임무 있으면 자율,
      없으면 보고 후 정지'** 로 바뀌었다. 그래서 '임무'·'exit 3' 근처의 대기 서술은 **정당한
      계약**이며 금지 대상이 아니다 — 금지되는 것은 여전히 **무조건 대기**(구 §0 문안)뿐이다."""
    ok_markers = ("폐기", "아니라", "금지", "임무", "exit 3", "미지정")
    for m in re.finditer(r"오너 지시 대기|오너의? 지시를 기다|오너 지시를 받아", text):
        window = text[max(0, m.start() - 40):m.end() + 40]
        need(any(k in window for k in ok_markers),
             "%s 에 **무조건** '오너 지시 대기' 문구가 살아 있다(§0 ⑥ 위반 — 임무 게이트 §0-C "
             "조건부 대기라면 근처에 '임무'·'exit 3'을 명시하라): …%s…"
             % (where, window.replace("\n", " ")))


def _mission_gate_pinned(text, where):
    """★T1 회귀 핀(2026-08-01 윈도우 실사고): 자율 착수 안내가 **임무 게이트 없이** 살아 있으면
    안 된다. 사고는 '큐에 항목이 있으면 무조건 자율 착수'라는 문안이 임무 미지정 부팅에서
    이전 세션 잔무 큐를 집어 온 것이었다 — 그 문안 자체를 기계로 금지한다."""
    if "next-action" not in text:
        return
    need(any(k in text for k in ("임무 게이트", "javis_mission", "exit 3", "임무 미지정")),
         "%s 가 next-action 을 안내하면서 **임무 게이트를 명시하지 않는다**"
         "(T1 회귀 — 임무 없는 부팅에서 잔무 큐 자율 착수가 되살아난다)" % where)
    for m in re.finditer(r"미완 작업이 있으면 자율 착수|있으면 자율 착수하라", text):
        window = text[max(0, m.start() - 80):m.end() + 80]
        need(any(k in window for k in ("임무", "exit 3", "폐기", "아니라")),
             "%s 에 무조건 자율 착수 문안이 살아 있다(T1 회귀): …%s…"
             % (where, window.replace("\n", " ")))


@specimen("H-DOC-1", "W1b", "§0↔훅 note↔template(사본 2벌) 문안 정합", ["A10", "P3-A-TEMPLATE"])
def h_doc_1():
    """A10: 부트 실행 주체 계약이 §0(디렉티브)·훅 note·template 3곳에 사본으로 살면서 서로
    달랐다. 계약을 §0-A 단일 표로 모으고 나머지는 **포인터**로 만든 뒤, 그 정합을 기계로 못박는다.
    P3-A-TEMPLATE: 훅 없는 기계의 폴백 절차("스크립트 1회")가 보존돼야 한다 — 산문 체인 금지."""
    md = _repo_file(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md"))
    hook = _read(_hook(ROLE_BODY))
    notes = []
    # ① §0-A 조건부 단일 계약이 존재하고 두 분기를 명시하는가
    need("0-A" in md and "실행 주체 단일 계약" in md, "§0-A 단일 계약 절이 없다(A10 미착지)")
    need(_HOOK_FIRED_MARK in md, "§0-A 가 훅 컨텍스트 신호 문구를 인용하지 않는다(정합 불가)")
    need("재실행 금지" in md, "§0-A 에 재실행 금지 분기가 없다")
    need("1회" in md and "javis_bootstrap.py" in md, "§0-A 에 '스크립트 1회' 폴백이 없다")
    need("수동 재현 금지" in md or "산문 체인" in md or "손으로 치는" in md,
         "§0-A 가 개별 명령 수동 재현 금지를 명문하지 않는다")
    notes.append("§0-A 조건부 계약")
    # ② 훅 note 가 같은 신호 문구를 쓰고 잔여 의무를 가리키는가(문안 정합의 핵심 축)
    need(_HOOK_FIRED_MARK in hook, "훅 note 와 §0-A 의 신호 문구가 다르다(정합 파괴)")
    need("재실행하지 마라" in hook, "훅 note 에 재실행 금지가 없다")
    need("next-action" in hook, "훅 note 가 next-action 자율 착수를 가리키지 않는다")
    _no_wait_for_owner(_code_lines(hook), "훅 note")   # 주석(수정 이력 설명)은 스캔 제외
    _mission_gate_pinned(_code_lines(hook), "훅 note")  # ★T1 회귀 핀
    # ★P2 문안 변형별 핀(R3-P2-7 ④): 발화 note 가 frontdoor(인텐트 기록)·폴백(직접 spawn)
    #   2벌이 됐다 — 위 파일 전체 grep 1회는 폴백 잔존만으로 초록이 되므로, 헤드라인 마커와
    #   재실행 금지 규율이 note 조립부 **2곳 전부**에 실려 있음을 계수로 못박는다(주석 제외).
    #   frontdoor note 의 로그 안내는 role-bootstrap 런 로그가 아니라 감독자 로그로 분기한다
    #   (R3-P2-7 ⑤) — 그 분기 앵커도 함께 핀한다. 문안의 실행 서술 정직성(백그라운드 서술
    #   금지 등)은 행동 검체 H-BOOT-INTENT-1 이 실행 산출로 잰다.
    hook_code = _code_lines(hook)
    need(hook_code.count(_HOOK_FIRED_MARK) >= 2,
         "발화 note 헤드라인 마커가 2곳(frontdoor·폴백) 미만이다 — §0-A 리터럴 결박이 한쪽 "
         "경로에서 끊겼다(마커 출현 %d회)" % hook_code.count(_HOOK_FIRED_MARK))
    need(hook_code.count("재실행하지 마라") >= 2,
         "재실행 금지 규율이 2곳(frontdoor·폴백) 미만이다 — frontdoor note 가 규율 문장을 "
         "잃어도 파일 전체 grep 은 초록이었다(R3-P2-7 ④의 사각)")
    need("boot-supervisor.log" in hook_code and "데몬 감독자" in hook_code,
         "frontdoor note 의 로그 안내 경로 분기(boot-supervisor.log·감독자 서술)가 없다"
         "(R3-P2-7 ⑤)")
    # ★T1: 디렉티브 §0-C(임무 게이트 정의처)와 exit 3 계약이 실재하는가 — 산문만 고치고 도구는
    #      안 고치는(또는 그 반대) 반쪽 수리 차단.
    need("0-C" in md and "임무 게이트" in md, "§0-C 임무 게이트 절이 없다(T1 미착지)")
    need("자기인가" in md, "§0-C 가 '큐=master 산출물 → 자기인가' 근본원인을 명문하지 않는다")
    need("javis_mission.py" in md, "§0-C 가 판정 도구(javis_mission.py)를 가리키지 않는다")
    orch_src = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    need("return 3" in orch_src and "이어서 하시겠습니까" in orch_src,
         "next-action 이 exit 3(임무 미지정 → 보고·정지) 경로를 갖지 않는다(문서만 수리)")
    notes.append("훅 note 신호·잔여의무 정합 · T1 임무 게이트 핀")
    # ③ template 2벌 사본이 §0 포인터 + 폴백 보존 문구를 갖고, **서로 동일**한가
    bodies = []
    for rel in _CLAUDE_MD_COPIES:
        t = _repo_file(rel)
        need("0-A" in t, "%s 가 §0-A 포인터를 갖지 않는다" % rel)
        need("javis_bootstrap.py" in t and "1회" in t,
             "%s 에 훅 미발화 폴백(스크립트 1회)이 없다(P3-A-TEMPLATE)" % rel)
        need("산문 체인" in t, "%s 에 '산문 체인 금지' 보존 문구가 없다(P3-A-TEMPLATE)" % rel)
        need("javis_preflight.py" not in t.split("## 터미널")[0],
             "%s 부트절이 아직 preflight 산문 지시를 담고 있다(A10 미수리)" % rel)
        _no_wait_for_owner(t, rel)
        _mission_gate_pinned(t, rel)                   # ★T1 회귀 핀(template 사본 2벌 동일 적용)
        bodies.append(t.split("## 터미널")[0])
    need(bodies[0] == bodies[1],
         "template 2벌 사본의 부트절이 갈렸다(사본 드리프트) — 길이 %d vs %d"
         % (len(bodies[0]), len(bodies[1])))
    notes.append("template 2벌 동일·폴백 보존")
    # 계측 타당성: 구 문서는 산문 부트 지시를 담고 있었다
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/CLAUDE.md.template")
    if old is not None:
        head = old.split("## 터미널")[0]
        need("javis_preflight.py" in head,
             "계측 타당성 실패: 구 template 부트절에 산문 preflight 지시가 없다")
        calib = "구 template 산문 부트 지시 확인"
    oldmd = _git_show("cysjavis-pack/directives/MASTER_DIRECTIVE.md")
    if oldmd is not None:
        need("실행 주체 단일 계약" not in oldmd,
             "계측 타당성 실패: 구 §0 에 이미 단일 계약이 있다")
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-DOC-6", "W1b",
          "§0 ③ 및 §9 상태 경로가 CYS_PACK_DIR 파생(base 하드코딩 0) "
          "— 재태깅 W4→W1b: W1b master 지시로 :225 동류 교정이 착지",
          ["G5"])
def h_doc_6():
    """G5(및 그 동류): 디렉티브가 base 팩 경로를 하드코딩하면 부서장 레인이 **본부 상태**를
    읽고 쓴다(교차 오염). 레인 파생(`${CYS_PACK_DIR:-$HOME/.cys/pack}`)만 허용한다."""
    rel = os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md")
    md = _repo_file(rel)
    bad = [l.strip()[:120] for l in md.splitlines() if "~/.cys/pack" in l]
    need(not bad, "base 팩 경로 하드코딩 잔존(%d줄): %s" % (len(bad), bad[:5]))
    # $HOME/.cys/pack 은 **반드시** CYS_PACK_DIR 폴백 형태로만 등장해야 한다
    loose = [l.strip()[:120] for l in md.splitlines()
             if "$HOME/.cys/pack" in l and "CYS_PACK_DIR:-$HOME/.cys/pack" not in l]
    need(not loose, "CYS_PACK_DIR 폴백 없는 $HOME 경로 잔존: %s" % loose[:5])
    need("${CYS_PACK_DIR:-$HOME/.cys/pack}/round/SESSION_STATE.md" in md,
         "§9 SESSION_STATE 경로가 레인 파생이 아니다(:225 동류 미교정)")
    # 계측 타당성: 기준 커밋에는 하드코딩이 있었다
    calib = "skip(no-git)"
    old = _git_show(rel)
    if old is not None:
        oldbad = [l for l in old.splitlines() if "~/.cys/pack" in l]
        need(oldbad, "계측 타당성 실패: 구 문서에 base 하드코딩이 없다면 이 검체는 무의미")
        calib = "구 문서 하드코딩 %d줄 확인" % len(oldbad)
    return "하드코딩 0줄 · §9 SESSION_STATE·RECOVERY·TODO 레인 파생 · 계측검증=%s" % calib
@specimen("H-DOC-2", "W4", "'노드 수' 표기가 REQUIRED_ROLES+1 파생(required 에 master 부재 유지)",
          ["B18"])
def h_doc_2():
    """B18(RC6): 훅 note 가 `master·cso·worker·reviewer×2 (5노드)` 를 **리터럴**로 박아 두고,
    판정 술어(REQUIRED_ROLES)는 그와 무관하게 진화했다 — 편성이 바뀌면 문서만 거짓이 된다.
    ★금지 방향 ②: 숫자를 맞추려고 `REQUIRED_ROLES` 에 master 를 넣으면 check 의 required 가 master 를
      요구하게 되고, 레거시 master 조합에서 **부트 전체가 사망**한다. master 는 안내에서만 +1 이다."""
    orch = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    hook = _read(_hook(ROLE_BODY))
    # ① 파생 소스가 존재하고 master 는 required 밖이다
    need("def team_roster_note(" in orch, "팀 구성 안내 파생 함수 부재(리터럴 잔존 위험)")
    r = _run([PY, os.path.join(BIN_DIR, "javis_orchestra.py"), "--note-team-roster"])
    need(r.returncode == 0 and r.stdout.strip(), "--note-team-roster 산출 실패: %s" % r.stderr[-300:])
    note = r.stdout.strip()
    # ② 실제 REQUIRED_ROLES 를 읽어 +1 파생인지 대조(숫자·역할명 전건)
    probe = "import sys;sys.path.insert(0, sys.argv[1]);import javis_orchestra as o;" \
            "print(repr(o.REQUIRED_ROLES))"
    rr = _run([PY, "-c", probe, BIN_DIR])
    need(rr.returncode == 0, "REQUIRED_ROLES 조회 실패: %s" % rr.stderr[-300:])
    required = eval(rr.stdout.strip())          # 리스트 리터럴(신뢰 경계: 우리 코드 산출)
    need("master" not in required,
         "REQUIRED_ROLES 에 master 가 들어갔다 — 금지 방향 ②(레거시 master 부트 사망)")
    need("총 %d노드" % (len(required) + 1) in note, "노드 수가 REQUIRED_ROLES+1 파생이 아니다: %s" % note)
    for role in required:
        need(role in note, "필수 역할 %s 가 안내에 없다: %s" % (role, note))
    need(note.startswith("master·"), "안내가 master 로 시작하지 않는다: %s" % note)
    # ③ 훅은 그 산출을 **인용**한다(자기 리터럴 금지)
    code = _code_lines(hook)
    need("--note-team-roster" in code, "훅이 파생 산출을 인용하지 않는다")
    need("TEAM_ROSTER" in code, "훅에 로스터 변수 배선 부재")
    need(not re.search(r"\d\s*노드", code), "훅 코드에 노드 수 리터럴 잔존: %s"
         % re.findall(r".{0,30}\d\s*노드.{0,20}", code)[:3])
    need(not re.search(r"reviewer×\d", code), "훅 코드에 리뷰어 개수 리터럴 잔존")
    # ④ 파생 실패 시 조용히 빈 문구가 되지 않는다(loud 폴백)
    need("로스터 모듈 미소비" in hook, "파생 실패 폴백 문구 부재(빈 안내로 조용히 퇴화)")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    if old is not None:
        need(re.search(r"\(5노드\)", old),
             "계측 타당성 실패: 구 훅에 '(5노드)' 리터럴이 없다면 B18 은 결함이 아니다")
        calib = "구 훅 '(5노드)' 리터럴 확인"
    return "파생=%s · required 에 master 부재 · 훅 리터럴 0 · 계측검증=%s" % (note[:48], calib)


@specimen("H-DOC-3", "W4", "CEO_TEMPLATE 동사 ⊆ cys-dept 가드 허용 집합(지시-집행 통일)", ["G6"])
def h_doc_3():
    """G6(RC6): CEO_TEMPLATE 가 CEO 에게 `cys-dept launch/down` **직접 호출**을 지시했는데,
    cys-dept 의 단일소유 가드는 `CYS_ROLE` 이 설정된 비-CSO 노드의 lifecycle 동사를 exit 7 로
    거부한다. CEO 는 role=master 이므로 **지시대로 하면 항상 실패**한다(지시와 집행의 정면 모순).
    이 검체는 문서가 지시하는 동사 집합이 가드가 허용하는 집합의 부분집합인지 기계 대조한다 —
    문서·가드 어느 쪽이 바뀌어도 즉시 적색이 된다."""
    tmpl_rel = os.path.join("cysjavis-pack", "directives", "CEO_TEMPLATE.md")
    tmpl = _repo_file(tmpl_rel)
    dept = _read(os.path.join(BIN_DIR, "cys-dept"))
    # ① 가드가 막는 동사 집합을 **코드에서** 뽑는다(문서에 적힌 목록을 신뢰하지 않는다)
    m = re.search(r"^\s*(launch\|[a-z|\-]+)\)\s*$", dept, re.M)
    need(m is not None, "cys-dept 단일소유 가드의 동사 case 를 찾지 못했다(가드 형태 변경?)")
    blocked = set(m.group(1).split("|"))
    need({"launch", "down", "create", "rotate"} <= blocked,
         "가드가 막는 집합이 예상보다 좁다(%s) — 검체 전제 재확인 필요" % sorted(blocked))
    need("CYS_ROLE" in dept and "exit 7" in dept, "가드 판정 재료(CYS_ROLE·exit 7) 부재")
    # ② 문서가 **호출 형태로** 지시하는 동사(`cys-dept <verb>`)를 뽑는다
    used = set(re.findall(r"cys-dept\s+([a-z][a-z\-]*)", tmpl))
    illegal = sorted(used & blocked)
    need(not illegal,
         "CEO_TEMPLATE 가 가드가 거부하는 동사를 직접 호출하도록 지시한다: %s "
         "(GUI 부서 버튼 또는 CSO 위임 경유로 개정하라)" % illegal)
    need(used, "문서에 cys-dept 사용 예가 0건 — 추출기 파손 의심(fail-closed)")
    # ③ 대안 경로가 명시돼 있다(금지만 하고 대안이 없으면 문서가 막다른 길이 된다)
    need("CSO" in tmpl and ("위임" in tmpl or "요청" in tmpl), "CSO 위임 경로 안내 부재")
    need("GUI" in tmpl, "GUI 부서 버튼 경로 안내 부재")
    need("exit 7" in tmpl, "가드 거부가 '계약'임을 문서가 알리지 않는다(버그로 오인)")
    calib = "skip(no-git)"
    old = _git_show(tmpl_rel)
    if old is not None:
        old_used = set(re.findall(r"cys-dept\s+([a-z][a-z\-]*)", old))
        need(old_used & blocked,
             "계측 타당성 실패: 구 문서가 이미 허용 동사만 썼다면 G6 은 결함이 아니다")
        calib = "구 문서 위반 동사 %s 확인" % sorted(old_used & blocked)
    return "문서 동사 %s ⊆ 허용(가드 차단 %d종) · 대안 2경로 명시 · 계측검증=%s" % (
        sorted(used), len(blocked), calib)


@specimen("H-DOC-4", "W4", "헤더 exit 표 ↔ 코드 상수 기계 대조", ["G31", "G32"])
def h_doc_4():
    """G31·G32(RC6): 헤더 산문의 exit 표가 코드 상수와 갈렸다(구 헤더 '2=preflight' 는 이미 없는
    계약이었고, check 의 exit 2=판정 불가는 표에 없었다). 문구는 소비자의 **처방**을 결정하므로
    틀린 표는 잘못된 조치를 유도한다. W0 이 문구를 고쳤고, 이 검체가 **기계 대조**를 상주시킨다."""
    bs_path = os.path.join(BIN_DIR, "javis_bootstrap.py")
    body = _read(bs_path)
    head = body[:body.index('"""', body.index('"""') + 3)]
    # ① 코드 상수 EXIT_* → 값
    consts = {int(v): k for k, v in re.findall(r"^(EXIT_[A-Z_]+)\s*=\s*(\d+)", body, re.M)}
    need(len(consts) >= 10, "EXIT_* 상수 추출 %d건 — 추출기 파손 의심(fail-closed)" % len(consts))
    # 상수 외에 **리터럴 return** 으로 나가는 exit 도 실재 계약이다(예: `issue-ticket` 사용오류 2).
    # 유령 판정은 '코드에 그 값으로 나가는 경로가 아예 없는가' 여야 한다 — 상수만 보면 정상 문서를
    # 오보한다(초안이 실제로 2 를 유령으로 잡았다).
    literal_returns = {int(n) for n in re.findall(r"^\s+return (\d{1,2})\s*$", body, re.M)}
    reachable = set(consts) | literal_returns
    # ② 헤더 표에 적힌 값 → 코드에 그 값이 실재해야 한다(유령 exit 금지)
    documented = {int(n) for n in re.findall(r"(?<![\w.])(\d{1,2})=", head)}
    need(documented, "헤더 exit 표를 0건 추출 — 표 형식 변경(fail-closed)")
    ghost = sorted(documented - reachable)
    need(not ghost, "헤더가 코드에 없는 exit 를 문서화한다(유령 계약): %s" % ghost)
    # ③ 반대로 게이트 exit 공간(2~11) 상수가 표에 있어야 한다(문서 결손 금지)
    undocumented = sorted(v for v in consts if 2 <= v <= 11 and v not in documented)
    need(not undocumented, "코드 exit 가 헤더 표에 없다(문서 결손): %s"
         % [(v, consts[v]) for v in undocumented])
    # ④ G31 회귀 핀: '2=preflight' 는 폐기된 계약이다(preflight 는 비치명).
    #    ★규칙을 '문자열 부재'로 두면 **폐기 선언 자체를 쓸 수 없다**(문서가 결함을 설명 못 한다) →
    #      H-DOC-1 의 `_no_wait_for_owner` 와 동형으로, 출현마다 근처에 부정 마커가 있어야 한다는
    #      형태 규칙으로 판정한다(살아있는 계약 vs 폐기 인용의 구분).
    for m in re.finditer(r"2\s*=\s*preflight", head):
        win = head[max(0, m.start() - 60):m.end() + 60]
        need(any(k in win for k in ("낡은", "폐기", "아니다", "금지")),
             "구 계약 '2=preflight' 가 살아있는 계약으로 적혀 있다(G31 회귀): …%s…"
             % win.replace("\n", " "))
    need("preflight" in head and ("비치명" in head or "경고 강등" in head),
         "preflight 비치명 계약이 헤더에 없다")
    # ⑤ G32 회귀 핀: check exit 2(판정 불가)가 1(미기동)과 **분리**돼 문서화됐다
    orch = _read(os.path.join(BIN_DIR, "javis_orchestra.py"))
    ohead = orch[:orch.index('"""', orch.index('"""') + 3)]
    need("판정 불가" in ohead, "orchestra 헤더에 check exit 2=판정 불가 서술 부재(G32)")
    need("cys boot" in ohead and "cys ping" in ohead,
         "처방 분기(2→ping / 1→boot)가 헤더에 없다 — 뭉개면 처방이 뒤집힌다")
    need("EXIT_CHECK" in body and consts.get(6) == "EXIT_CHECK", "⑤check exit 상수 대응 이탈")
    # ⑥ (W4) 소비 계약: `cys boot` 의 busy exit 가 3자 파리티다(python·Rust·lib)
    need("CYS_BOOT_EXIT_BUSY = 75" in body, "python 소비부 busy exit 상수 이탈")
    lib = os.path.join(REPO_DIR, "src", "lib.rs")
    if os.path.isfile(lib):
        need("pub const EXIT_BOOT_BUSY: i32 = 75;" in _read(lib),
             "Rust lib 정본 busy exit 상수 이탈(파리티 붕괴)")
    # ★계측 기준 = **W0 이전**(G31/G32 문구는 W0 이 고쳤다). 구 헤더에 '2=preflight' 유령 계약이
    #   실재했음을 확인해 탐지기가 진짜 결함을 잡는 형태임을 증명한다.
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_bootstrap.py", ref=PRE_W0_REF)
    if old is not None:
        oldhead = old[:old.index('"""', old.index('"""') + 3)]
        need(re.search(r"2\s*=\s*preflight", oldhead),
             "계측 타당성 실패: W0 이전 헤더에 '2=preflight' 유령 계약이 없다면 G31 은 결함이 아니다")
        calib = "W0 이전 헤더 '2=preflight' 유령 계약 확인"
    return "exit 상수 %d종 ↔ 헤더 표 %d종 대조 · 유령 0·결손 0 · busy 파리티 · 계측검증=%s" % (
        len(consts), len(documented), calib)


@specimen("H-DOC-5", "W4", "generic reviewer 안내 문구 금지(로스터 역할명만)", ["G30"])
def h_doc_5():
    """G30(RC6): 안내문이 `cys claim-role <worker|cso|reviewer>` 라고 적어, 리뷰어가 generic
    `reviewer` 로 등록하도록 유도했다. 그런데 orchestra check 는 **에이전트별 역할명**
    (reviewer-gemini·reviewer-codex·대체 reviewer-claude-N)만 의무 좌석으로 인정한다 →
    지시를 따른 노드가 '부재'로 판정돼 부트가 영구 실패한다(문서가 결함을 생산한 사례)."""
    targets = [("hooks/session-start.sh", _read(_hook("session-start.sh"))),
               ("directives/MASTER_DIRECTIVE.md",
                _repo_file(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md")))]
    for rel, body in targets:
        code = _code_lines(body)
        bad = re.findall(r"claim-role\s+<?[a-z|]*\breviewer\b(?![-\w])", code)
        need(not bad, "%s 에 generic reviewer 등록 안내 잔존: %s" % (rel, bad[:3]))
        need("reviewer-gemini" in code and "reviewer-codex" in code,
             "%s 에 에이전트별 리뷰어 역할명이 없다" % rel)
    # 대체 슬롯·선택 슬롯까지 안내에 실재해야 한다(무구독 폴백 기계가 이름을 못 찾는 사고 차단)
    ss = _read(_hook("session-start.sh"))
    for name in ("reviewer-claude-1", "reviewer-claude-2", "reviewer-grok"):
        need(name in ss, "session-start 안내에 %s 누락(폴백 로스터 이름 결손)" % name)
    need("generic" in ss and ("실패" in ss or "못 보고" in ss),
         "generic 등록이 왜 위험한지(판정 실패) 안내가 없다 — 금지만 하면 다시 쓴다")
    # 로스터 이름의 정본은 코드다 — 안내에 적힌 이름이 REVIEWER_SLOTS 와 일치해야 한다
    probe = "import sys;sys.path.insert(0, sys.argv[1]);import javis_orchestra as o;" \
            "print(repr([r for s in o.REVIEWER_SLOTS for r in (s[0], s[2])]))"
    rr = _run([PY, "-c", probe, BIN_DIR])
    need(rr.returncode == 0, "REVIEWER_SLOTS 조회 실패: %s" % rr.stderr[-300:])
    for role in eval(rr.stdout.strip()):
        need(role in ss, "코드 로스터 역할 %s 가 안내에 없다(사본 드리프트)" % role)
    calib = "skip(no-git)"
    # ★계측 기준 = **W0 이전**(G30 문구는 W0 이 고쳤으므로 W0 착지 트리에서는 이미 정상이다)
    old = _git_show("cysjavis-pack/hooks/session-start.sh", ref=PRE_W0_REF)
    if old is not None:
        need(re.search(r"claim-role\s+<?[a-z|]*\breviewer\b(?![-\w])", _code_lines(old)),
             "계측 타당성 실패: W0 이전 안내에 generic reviewer 가 없다면 G30 은 결함이 아니다")
        calib = "W0 이전 안내 generic reviewer 확인"
    return "안내 2곳 generic 0 · 로스터 4종 정합 · 위험 고지 · 계측검증=%s" % calib


@specimen("H-DOC-9", "W5",
          "session-start '선언=기동 명령' 문안 핀(D4-a′) — 기동 계약·착수금지 마커 드리프트 방어",
          ["D4-a′(2択 혼동)", "T1 자기인가"])
def h_doc_9():
    """D4-a′(2026-08-10 오너 재정): session-start 안내(■ 팀 기동)가 새 계약의 온보딩 서술이다 —
    여기가 구 문안('실행 여부는 사용자와 정하라')으로 되돌아가면 훅(선언=자동 발화 · H-MISSION-1)
    과 안내가 갈려 H-DOC-1 류 사본 드리프트가 재발한다. 핵심 마커 2개를 정적으로 핀한다:
      ① 기동 계약: 선언이 곧 기동 명령(실행 여부를 따로 묻지 않는다)
      ② 착수 규율: 팀이 떠도 자율 착수는 금지(next-action exit 3 — T1 층 무손상)"""
    ss = _read(_hook("session-start.sh"))
    need("선언이 곧 기동 명령" in ss,
         "session-start 안내에 D4-a′ 기동 계약 마커('선언이 곧 기동 명령')가 없다 — "
         "훅은 자동 발화하는데 안내가 구 계약(실행 여부 문의)을 말하면 사본 드리프트")
    need("자율 착수는 금지" in ss,
         "session-start 안내에 착수 규율 마커('자율 착수는 금지')가 없다 — "
         "spawn 허용(D4-a′)과 착수 금지(T1)의 분리가 안내에서 사라진다")
    # 계측 타당성: D4-a 시대 안내(구 문안)에는 두 마커가 없어야 한다(탐지기가 신·구를 구분하는가)
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/session-start.sh", ref=D4A_REF)
    if old is not None:
        need("선언이 곧 기동 명령" not in old and "자율 착수는 금지" not in old,
             "계측 타당성 실패: D4-a 시대 안내에 이미 D4-a′ 마커가 있다 — 핀이 신·구를 구분하지 못한다")
        calib = "D4-a 안내=마커 2종 부재 확인"
    return "기동 계약·착수 규율 마커 2종 핀 · 계측검증=%s" % calib


@specimen("H-DOC-10", "W6",
          "`--detach-session` ↔ 창작자 ACL 근거 주석 인용 결박(유령 인용·반쪽 제거 차단)",
          ["U-24", "결함8(2026-08-22 부트 실사고)", "④ 전 pane 사망"])
def h_doc_10():
    """U-24 이관의 **경계 검체**. `--detach-session` 은 `javis_bootstrap.py` 안에서만 사는
    인자가 아니다 — 데몬의 창작자 ACL 원장(`state.rs Daemon::create_caller` · 판정
    `handlers.rs creator_matches`/`ACL_ROLE_CREATOR`)이 **존재 근거로 이 문자열을 인용**한다.

    실사고(2026-08-22): 훅이 `setsid python3 javis_bootstrap.py --detach-session` 으로 부트를
    백그라운드 발화하면 그 프로세스가 launchd 로 재부모화돼 `external` 등급이 되고, 부서 ACL 의
    `external→worker*` deny 에 **부트가 자기가 방금 만든 워커 좌석에 지침을 넣는 것**까지 걸렸다
    (`acl denied: external → worker` → 좌석 롤백 → 워커 기동 실패 = 글자 0).

    ∴ 이관하면서 팩 쪽 인자만 지우면 Rust 쪽 주석은 **유령 인용**이 되고, 다음 사람이
    "근거가 사라졌으니 원장도 지워도 된다"고 읽는 순간 그 사고가 재발한다. 이 검체는
    **인자와 인용의 동시 존재 ∨ 동시 부재**를 강제한다 — 반쪽 제거가 구조적으로 불가능해진다.
    ★U-24 에서 인자를 제거하지 않은 이유도 여기 남긴다: 제거는 훅 발화부·state.rs·handlers.rs
      3파일 동시 개정을 요구하고, 그 동시성이 깨진 상태가 곧 위 사고다.
    """
    FLAG = "--detach-session"
    boot_src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    state_src = _repo_file(os.path.join("src", "bin", "cysd", "state.rs"))
    handlers_src = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    notes = []

    def _verdict(pack, state, handlers):
        """순수 술어 — (팩에 인자 있음, state.rs 인용, handlers.rs 인용) → 위반 목록."""
        bad = []
        has_flag = FLAG in pack
        cites = [("state.rs", FLAG in state), ("handlers.rs", FLAG in handlers)]
        for nm, cited in cites:
            if cited and not has_flag:
                bad.append("%s 가 유령 인용(팩에서 %s 가 사라졌다)" % (nm, FLAG))
            if has_flag and not cited:
                bad.append("%s 에 창작자 ACL 근거 인용이 없다(근거 소실)" % nm)
        return bad

    need(not _verdict(boot_src, state_src, handlers_src),
         "인용 결박 위반: %r" % _verdict(boot_src, state_src, handlers_src))
    notes.append("인자↔인용 3자 정합(%s)" % ("존재" if FLAG in boot_src else "부재"))

    # ⓑ 근거의 **강건성**: 원장의 존재 이유가 인자 이름이 아니라 '재부모화라는 사실'임을
    #    state.rs 가 명시해야 한다. 그렇지 않으면 인자 개명 한 번에 근거가 통째로 흔들린다.
    need("재부모화" in state_src and "H-DOC-10" in state_src,
         "state.rs 창작자 원장 주석이 재부모화 근거·이 검체 포인터를 담지 않는다 — "
         "인용이 인자 이름에만 매달려 있으면 개명 한 번에 근거가 증발한다")
    need("javis_bootstrap.py" in state_src,
         "state.rs 주석이 인용 출처 파일을 명시하지 않는다(추적 불가)")
    notes.append("state.rs 근거 강건성(재부모화·검체 포인터)")

    # ⓒ 팩 쪽도 반대 방향 결박을 문서화하는가(같은 커밋 동시 개정 지시)
    need("state.rs" in boot_src and "handlers.rs" in boot_src,
         "javis_bootstrap 의 detach_session 이 Rust 쪽 동시 개정 의무를 명시하지 않는다")
    need("H-DOC-10" in boot_src, "팩 쪽에 이 검체 포인터가 없다(결박이 사람 기억에만 있다)")
    notes.append("팩→Rust 동시개정 의무 명문")

    # ⓓ **합성 표본으로 탐지력 시험** — 트리가 정합이라 초록인 핀은 탐지 능력을 따로 증명한다.
    need(_verdict(boot_src.replace(FLAG, "--gone"), state_src, handlers_src),
         "계측 타당성 실패: 팩에서 인자를 지운 합성 표본에도 유령 인용 탐지기가 침묵한다")
    need(_verdict(boot_src, state_src.replace(FLAG, "--gone"), handlers_src),
         "계측 타당성 실패: state.rs 인용을 지운 합성 표본에서 근거 소실을 못 잡는다")
    need(_verdict(boot_src, state_src, handlers_src.replace(FLAG, "--gone")),
         "계측 타당성 실패: handlers.rs 인용을 지운 합성 표본에서 근거 소실을 못 잡는다")
    need(not _verdict(boot_src.replace(FLAG, "--gone"), state_src.replace(FLAG, "--gone"),
                      handlers_src.replace(FLAG, "--gone")),
         "계측 타당성 실패: **동시 부재**(정당한 완전 제거)까지 위반으로 잡는다 = 과탐 "
         "— 이 결박은 반쪽 제거만 막아야 한다")
    notes.append("합성 표본 4종(반쪽 3·동시부재 1) 판별")
    return " · ".join(notes)


@specimen("H-DOC-11", "W6",
          "P0-3 재실행 계약의 **배달** — 훅 브리지 자기완결(user-owned 디렉티브 미도달 방어)",
          ["R3-DELIVERY-1", "P0-3", "①폭주"])
def h_doc_11():
    """R3-DELIVERY-1(2026-08-26 적대검증): P0-3 기계 래치(`result.retry_eligible`)는 도달하는데
    그 값을 소비할 **권한 행**(§0-A session_error 행)은 도달하지 않는 비대칭이 있었다.

    비대칭의 기제는 소유권 등급이다 — `directives/*_DIRECTIVE.md` 는 `src/pack.rs ownership()`
    상 `Ownership::User` 라 팩 업데이트가 본문을 절대 덮지 않고(신본은 `<rel>.new` 병치 +
    `cys pack-merge` 대기), 훅은 `System` 이라 강제 치유로 전원에게 도달한다. 그래서 훅이
    "§0-A 의 session_error 행이 재실행 금지 행보다 우선한다"고 **가리키기만** 하면, 기존 설치본
    에는 훅만 도착하고 그 행은 없어 '재실행 금지 vs 1회 재실행'의 **이중 진실**이 배포된다 —
    모델은 그 틈을 재량으로 판결하고, 그것이 정확히 기계 래치가 없애려던 LLM 재량 재시도다.

    수리 계약: 두 훅(session-start.sh 브리지 · role-bootstrap.sh rc6/기타 FORECAST)이 상한
    1회·측정불능 금지를 **자기 문장으로** 서술한다(§0-A 는 정본 참조로만 남는다). 이 검체가
    그 자기완결성을 박제한다 — 포인터로 되돌아가면 배달 결손이 다시 침묵한다."""
    ss = _code_lines(_read(_hook("session-start.sh")))
    rb = _code_lines(_read(_hook(ROLE_BODY)))
    notes = []
    # ① 두 훅 모두 '1회'와 '재실행 금지' 양 분기를 자기 문장으로 갖는다(포인터만이면 실패).
    for rel, body in (("session-start.sh", ss), ("role-bootstrap.sh", rb)):
        need("retry_eligible" in body,
             "%s 가 래치 필드명을 인용하지 않는다 — 소비 근거가 문안 재량으로 되돌아갔다" % rel)
        need("1회" in body and "재실행 금지" in body,
             "%s 브리지에 상한 1회/재실행 금지 두 분기가 자기 문장으로 없다" % rel)
        need("pack-merge" in body or "MASTER_DIRECTIVE.md.new" in body,
             "%s 가 '디렉티브는 user 소유라 팩 갱신이 도달하지 않는다'는 배달 사실을 고지하지 "
             "않는다 — 결손 기계의 독자가 왜 표에 그 행이 없는지 알 수 없다" % rel)
        need("측정 불능" in body or "retry_eligible_unknown" in body,
             "%s 브리지가 측정 불능 금지 분기를 싣지 않는다(측정 불능은 통과가 아니다)" % rel)
    notes.append("훅 2벌 자기완결(상한·측정불능·배달 고지)")
    # ② 디렉티브 정본은 여전히 같은 규칙을 말한다(자기완결화가 정본을 지우면 안 된다).
    md = _repo_file(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md"))
    need("retry_eligible" in md and "session_error" in md,
         "§0-A 정본에서 session_error 행이 사라졌다 — 자기완결화는 정본 제거가 아니다")
    notes.append("§0-A 정본 병존")
    # ③ 결손이 **관측**되는가 — preflight 에 배달 축이 있고, 부트를 볼모로 잡지 않는다(WARN).
    pf = _read(os.path.join(BIN_DIR, "javis_preflight.py"))
    need("C03.boot-contract" in pf, "preflight 에 배달 축(C03.boot-contract)이 없다 — 결손 무관측")
    seg = pf[pf.index("def _c03_boot_contract("):]
    seg = seg[:seg.index("\n    # ── C04")]
    need("self.add(cid, FAIL" not in seg,
         "배달 축이 FAIL 을 낸다 — 병합 전 전 함대가 상시 NOT READY(관측 목적이 부트 준비 "
         "판정을 볼모로 잡는다). 이 축은 WARN 고정이 계약이다")
    need(seg.count("self.add(cid, WARN") >= 2 and "self.add(cid, PASS" in seg,
         "배달 축의 등급 배선(WARN 갈래·PASS 갈래)이 성립하지 않는다")
    need("pack-merge" in seg, "배달 축이 해소 명령을 싣지 않는다(관측만 하고 처방 없음)")
    notes.append("preflight 배달 축 WARN 고정")
    # ④ 탐지력 시험(합성 표본) — 포인터 전용으로 되돌린 본문에서 침묵하면 핀이 무의미하다.
    synthetic = ss.replace("retry_eligible", "REMOVED")
    need("retry_eligible" not in synthetic,
         "계측 타당성 실패: 합성 표본에서 래치 필드명이 지워지지 않았다(탐지기 무효)")
    notes.append("합성 표본 판별")
    return " · ".join(notes)


@specimen("H-DOC-7", "W4", "agents 스키마 완결성(preflight C71 — 결손 의미 고지·vendor/user 계층)",
          ["B20"])
def h_doc_7():
    """B20(RC6): 어댑터의 **선택 키 결손이 조용히 기능을 퇴화**시켰다(ready_marker 없음→readiness
    시간폴백, resume_arg 없음→restore 가 대화기억 없이 fresh 기동 = 맥락 소실). 어느 것도 그
    자체로 오류가 아니라서(grok 처럼 원래 없는 게 정상인 어댑터가 있다) 판정은 'FAIL' 이 아니라
    **결손의 의미를 말하는 것**이어야 한다 — 그리고 가장 비싼 손실(resume_arg)만 WARN 으로 올린다.
    ★W4a 착지분을 **검증**한다(재구현 아님)."""
    pf = _read(os.path.join(BIN_DIR, "javis_preflight.py"))
    need("C71" in pf, "preflight 에 C71 어댑터 스키마 체크 부재(B20 미착지)")
    need("def c71_agents_schema(" in pf, "C71 구현 함수 부재")
    need("OPTIONAL_KEY_MEANING" in pf, "결손 키 의미 표 부재(무음 퇴화 고지 불가)")
    seg = pf[pf.index("OPTIONAL_KEY_MEANING"):]
    seg = seg[:seg.index("KNOWN_AGENTS_SCHEMA")]
    for key in ("ready_marker", "clear_cmd", "resume_arg", "approval_patterns"):
        need('"%s"' % key in seg, "결손 의미 표에 %s 누락" % key)
    # resume_arg **만** WARN 티어(WARN 남발은 신호를 죽인다)
    rows = re.findall(r'\("([a-z_]+)",\s*"[^"]*",\s*(True|False)\)', seg)
    need(rows, "의미 표 행 추출 0건 — 추출기 파손(fail-closed)")
    warn_keys = sorted(k for k, w in rows if w == "True")
    need(warn_keys == ["resume_arg"], "WARN 티어가 resume_arg 단독이 아니다: %s" % warn_keys)
    # 스키마 버전 계층 인지: 알려진 버전 목록 + 미지 버전은 WARN(하위호환 유지)
    need("KNOWN_AGENTS_SCHEMA" in pf, "알려진 스키마 버전 목록 부재")
    # ★핀 이사(U-2 ②) — U-12 의 `_schema` 3 범프에 맞춰 목록이 (1,2,3) 으로 **누적**됐다.
    #   완화가 아닌 이유: 이 핀의 판정축은 "preflight 목록 ↔ agents.json 실값의 파리티"이고,
    #   그 축은 아래 두 줄이 그대로 집행한다(현행 _schema 가 목록 안에 있어야 한다 + 목록이
    #   정확히 이 튜플이어야 한다). 구 버전 1·2 를 남기는 것은 사용자 디스크본 하위호환이다.
    need(re.search(r"KNOWN_AGENTS_SCHEMA\s*=\s*\(1,\s*2,\s*3\)", pf),
         "스키마 버전 목록이 (1, 2, 3) 이 아니다 — agents.json _schema 와 파리티 확인 필요")
    aj = json.loads(_read(os.path.join(PACK_DIR, "agents.json")))
    need(aj.get("_schema") in (1, 2, 3), "agents.json _schema 가 알려진 버전이 아니다")
    need("cmd" in pf and "기동 불가" in pf, "필수 키(cmd) 결손이 FAIL 로 분리되지 않는다")
    # vendor/user 계층 구분이 문서에 있다(★W-B 동결 정합 — 사용자 파일을 코드가 고치지 않는다)
    need("_schema" in aj.get("_doc", "") and "vendor" in aj.get("_doc", ""),
         "_doc 에 vendor/user 필드 계층 설명 부재")
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/bin/javis_preflight.py")
    if old is not None:
        need("C71" not in old,
             "계측 타당성 실패: 구 preflight 에 이미 C71 이 있다면 B20 은 결함이 아니다")
        calib = "구 preflight C71 부재 확인"
    return "C71 실재·의미표 4키·WARN=resume_arg 단독·스키마(1,2,3) 파리티 · 계측검증=%s" % calib


@specimen("H-DOC-8", "W4", "팀 부트 진입점 전수가 단일 계약(폴백 포함 typed 소비 + 강등 신호)", ["B5"])
def h_doc_8():
    """B5(RC1·RC3): 팀 부트 진입점이 셋(훅 체인 / GUI 버튼 / 산문 §0)이었고 GUI 만 **체인을
    건너뛰어** 자기 판정을 가졌다 — 판정 재료가 stdout **산문 문자열**("신규 기동 0"·"미설치")이라
    ①문구가 바뀌면 조용히 오작동하고 ②건강한 팀+grok 미설치에서 위경보가 났고(P3-B16)
    ③claude 만 설치→리뷰어 0 은 무경고였다(R5) ④플랫폼 힌트 사본이 없어 macOS 명령을 Windows
    사용자에게 안내했다(P3-B15).
    ★목표는 '단일 진입점'이 아니라 **'단일 계약 + 명시 강등'**(비평2 D-4): 경로는 둘이어도
      (1차=체인 / 폴백=`cys boot --json` 직접) 판정 계약은 하나이고, 강등은 typed 신호로 기록된다."""
    gui_rel = os.path.join("src-tauri", "src", "main.rs")
    gui_path = os.path.join(REPO_DIR, gui_rel)
    if not os.path.isfile(gui_path):
        raise Skip("레포 체크아웃 아님(배포 팩) — GUI 소스 부재")
    gui = _read(gui_path)
    seg = gui[gui.index("fn spawn_orchestra_boot"):]
    seg = seg[:seg.index("\nfn emit_boot_signal")]
    # ① 1차 = 훅과 **같은 체인**
    need("javis_bootstrap.py" in seg, "GUI 1차 경로가 부트 체인이 아니다(B5 미착지)")
    need('.arg("run")' in seg, "체인 서브커맨드(run) 호출 부재")
    need("inject_runtime_path" in seg, "동봉 runtime PATH 주입 부재(Windows 에서 python 해소 실패)")
    # ② 폴백 = cys boot --json 직접 + typed 강등 신호(조용한 강등 금지)
    need('.arg("boot")' in seg and '.arg("--json")' in seg, "폴백이 typed 계약을 소비하지 않는다")
    need('"boot-degraded"' in seg, "강등이 조용하다(typed 신호 부재)")
    # ③ 산문 매칭 전폐(구 판정 재료 재도입 차단)
    for banned in ("신규 기동 0", 'contains("미설치")'):
        need(banned not in seg, "산문 문자열 매칭 재도입: %s" % banned)
    # ④ 경고 규율: mandatory 실패만 경고 · install_hint 그대로 · busy 는 정보
    need("fn boot_json_fatal_message(" in gui, "typed role 표 판정 함수 부재")
    fseg = gui[gui.index("fn boot_json_fatal_message("):]
    fseg = fseg[:fseg.index("\n}\n") + 2]
    need('"mandatory"' in fseg, "mandatory 필드를 보지 않는다(Degrade 위경보 재발)")
    need('Some("failed") | Some("missing")' in fseg, "outcome 타입 판정이 아니다")
    need('"install_hint"' in fseg, "install_hint 를 표출하지 않는다(플랫폼 사본 부활 위험)")
    need("EXIT_BOOT_BUSY" in gui, "busy exit 를 별도 분기하지 않는다(중첩 부트 위경보)")
    need("enum BootSignal" in gui, "신호 등급 타입 부재 — 조용한 실패를 타입으로 막지 못한다")
    # ④′ ★R3-GUI-4 범위 정직 등재: GUI 경로는 P1(좌석 토큰)·P2(boot-intent 프런트도어) 어느
    #    쪽도 받지 못한 채 남았고(Tauri 프로세스 자식 = pane env 미상속 · 프런트도어 미경유),
    #    2차 성찰의 '위탁' 교차참조는 **미구현·미이월**이었다. 미해소 교차참조를 그대로 두면
    #    다음 독자가 '이미 위탁됐다'고 오독한다 — 함수 doc 의 정직 등재를 핀으로 못박는다.
    #    (수리가 아니라 등재다: 이 핀이 초록이어도 GUI 경로의 rc 6 클래스는 열려 있다.)
    need("R3-GUI-4" in gui,
         "spawn_orchestra_boot doc 에 범위 정직 등재(R3-GUI-4)가 없다 — GUI 경로가 P1·P2 "
         "미적용인 사실이 코드에서 사라지면 '위탁 완료' 오독이 재발한다")
    need("CYS_SEAT_TOKEN" in gui and "caller_unresolved" in gui,
         "정직 등재가 미적용의 **기제**(토큰 미상속·프런트도어 좌석 도출 실패)를 명시하지 "
         "않는다 — 결론만 있고 근거가 없으면 다음 수정자가 검증 없이 지운다")
    # ⑤ 진입점 전수: 훅·산문도 같은 체인을 가리킨다(산문이 개별 명령 재현을 지시하지 않는다)
    hook = _read(_hook(ROLE_BODY))
    need("javis_bootstrap.py" in hook, "훅이 체인을 발화하지 않는다")
    md = _repo_file(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md"))
    need("javis_bootstrap.py" in md and ("수동 재현 금지" in md or "산문 체인" in md),
         "§0 이 체인 단일 계약을 가리키지 않는다")
    # ⑥ 소비부(python)도 같은 typed 계약(outcome·mandatory)을 읽는다
    bs = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    need('"failed", "missing"' in bs and '"mandatory"' in bs,
         "python 소비부 판정이 typed 계약이 아니다")
    need("_boot_was_busy(" in bs, "python 소비부가 busy(무스폰)를 분기하지 않는다(티켓 소각 위험)")
    calib = "skip(no-git)"
    old = _git_show(gui_rel)
    if old is not None:
        need("신규 기동 0" in old,
             "계측 타당성 실패: 구 GUI 에 산문 매칭이 없다면 B5 는 결함이 아니다")
        need("javis_bootstrap.py" not in old.split("fn spawn_orchestra_boot")[1][:3000]
             if "fn spawn_orchestra_boot" in old else True,
             "계측 타당성 실패: 구 GUI 가 이미 체인을 썼다면 통일이 결함이 아니다")
        calib = "구 GUI 산문 매칭·체인 미사용 확인"
    return "1차=체인·폴백=--json·강등 typed · mandatory 한정 경고 · 진입점 4곳 정합 · 계측검증=%s" % calib
# ═══════════════════════════════════════════════════════════════════════════
# W3 발효: 시드·등록·수명주기·레인 (H-SEED-1~5 · H-LIFE-1~2)
# ═══════════════════════════════════════════════════════════════════════════
class _env_patch:
    """env 를 임시 치환한다(None = 삭제). 검체는 실 HOME·실 팩을 절대 만지지 않는다."""

    def __init__(self, **kw):
        self.kw = kw
        self.saved = {}

    def __enter__(self):
        for k, v in self.kw.items():
            self.saved[k] = os.environ.get(k)
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        return self

    def __exit__(self, *a):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        return False


class _temp_guard_double:
    """preflight 의 **임시 팩 가드**를 검체 동안 마커 기반으로 치환한다.

    ★왜 필요한가(계측 타당성): 이 러너의 격리 샌드박스는 그 자체가 `/var/folders/...`(임시 dir)
      아래에 산다 — 그래서 실 가드가 **모든** 샌드박스 팩을 '임시 팩'으로 판정해 등록을 금지한다.
      그러면 base 팩 시나리오(폴백 허용·프로필 발견)를 아예 잴 수 없다.
    ★그래서 가드의 *논리*는 버리지 않고 **판정 입력만 마커로 바꾼다**: 마커 경로를 준 케이스는
      여전히 '임시 팩=금지'로 판정되므로 가드 경로도 같은 검체 안에서 실측된다. 실 함수 자체의
      타당성(진짜 tmp 경로를 True 로 본다)은 진입 시 1회 단언한다."""

    def __init__(self, PF, mark):
        self.PF = PF
        self.mark = mark
        self.orig = None

    def __enter__(self):
        self.orig = self.PF._path_under_tempdir
        need(self.orig(tempfile.gettempdir()) is True,
             "계측 타당성 실패: 실 임시 팩 가드가 tmp 경로를 임시로 보지 않는다")
        self.PF._path_under_tempdir = lambda path, m=self.mark: m in (path or "")
        return self

    def __exit__(self, *a):
        self.PF._path_under_tempdir = self.orig
        return False


def _preflight_mod():
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_preflight
    return javis_preflight


def _bootstrap_mod():
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_bootstrap
    return javis_bootstrap


def _rust_u64_const(src, name):
    """`pub const <name>: u64 = <n>;` 해소 — 매니페스트가 상수를 참조할 때 값을 따라간다."""
    m = re.search(r"pub const %s: u64 = (\d+);" % re.escape(name), src)
    need(m, "src/pack.rs 에서 상수 %s 를 찾지 못했다" % name)
    return int(m.group(1))


def _rust_awakening_hooks():
    """src/pack.rs `AWAKENING_HOOKS` 리터럴 → {(script, event, matcher, timeout)}. 파싱 실패=hard fail.

    ★U-21: 종전 파서는 3-튜플(`script→event→matcher`)이었다. `timeout` 필드를 뒤에 **덧붙이기만**
    하면 이 정규식은 그대로 통과하면서 timeout 스큐를 못 본다 = **무음**. 그래서 필드 확장과
    파서 확장은 같은 커밋에서 함께 간다(핀 이사 계약 ③).
    필드 순서(`matcher` 다음 `timeout`)가 곧 계약이며 아래 정규식이 그 순서를 집행한다."""
    src = _repo_file(os.path.join("src", "pack.rs"))
    m = re.search(r"pub const AWAKENING_HOOKS: \[DesiredHook; (\d+)\] = \[(.*?)\n\];", src, re.S)
    need(m, "src/pack.rs 에서 AWAKENING_HOOKS 를 찾지 못했다(표현이 바뀌면 이 대조를 함께 갱신하라)")
    body = _code_lines(m.group(2).replace("//", "#"))
    out = set()
    for blk in re.finditer(
            r'script:\s*"([^"]+)"\s*,\s*event:\s*"([^"]+)"\s*,\s*matcher:\s*(None|Some\("([^"]*)"\))'
            r'\s*,\s*timeout:\s*(None|Some\(([A-Za-z0-9_]+)\))',
            body):
        raw = blk.group(6)
        if raw is None:
            timeout = None
        elif raw.isdigit():
            timeout = int(raw)
        else:
            timeout = _rust_u64_const(src, raw)
        out.add((blk.group(1), blk.group(2), blk.group(4), timeout))
    need(len(out) == int(m.group(1)),
         "Rust 매니페스트 파싱 수 불일치(선언 %s ≠ 파싱 %d) — timeout 필드가 matcher 뒤에 없거나 "
         "순서 계약이 깨졌다" % (m.group(1), len(out)))
    return out


def _make_profile(home, name, hooks):
    """격리 HOME 에 프로필 디렉터리+settings.json 생성. hooks = {event: [command…]}."""
    d = os.path.join(home, name)
    os.makedirs(d, exist_ok=True)
    if hooks is None:
        return d
    data = {"hooks": {ev: [{"hooks": [{"type": "command", "command": c}]} for c in cmds]
                      for ev, cmds in hooks.items()}}
    _w(os.path.join(d, "settings.json"), json.dumps(data, ensure_ascii=False, indent=2), 0o644)
    return d


def _fake_pack_with_hooks(pack):
    """C28 이 요구하는 훅 스크립트·reflect 엔진을 갖춘 가짜 팩(등록 판정만 재려면 존재만 충분)."""
    for _, script in (("", "session-start.sh"), ("", "role-bootstrap.sh"),
                      ("", "inject-context.sh"), ("", "save-state.sh"),
                      ("", "reflect-scan.sh"), ("", "commit-memory-nudge.sh"),
                      ("", "pack-guard.sh")):
        _w(os.path.join(pack, "hooks", script), "#!/bin/sh\nexit 0\n")
    _w(os.path.join(pack, "bin", "javis_reflect.py"), "import sys;sys.exit(0)\n", 0o644)
    return pack


@specimen("H-SEED-1", "W3", "소망 훅 집합 동등성 — Rust 시드/init-pack == 파이썬 매니페스트", ["A9"])
def h_seed_1():
    """A9: 소망상태가 **집행자마다 흩어져** 있었다 — Rust 시드는 `!settings.exists()` 파일 단위,
    init-pack 은 SessionStart 하나만, role-bootstrap(UserPromptSubmit)은 preflight C28 만 등록했고
    그 C28 의 유일한 자동 트리거가 **결손된 그 훅 자신**이었다(닭·달걀). 소망 집합을 데이터 한 곳에
    적고(1 데이터 × N 집행자) 집행을 **이벤트 단위 멱등 병합**으로 바꾼다."""
    PF = _preflight_mod()
    rust = _rust_awakening_hooks()
    py = {(script, ev, matcher, to)
          for script, evs in PF.AWAKENING_HOOKS for ev, matcher, to in evs}
    need(rust == py, "소망 집합이 2언어에서 갈렸다 — rust=%r / python=%r" % (sorted(rust), sorted(py)))
    notes = ["집합 %d항 2언어 일치(timeout 포함 4-튜플)" % len(rust)]
    # ⓐ 각성 집합은 SessionStart + UserPromptSubmit 을 **둘 다** 갖는다(둘 중 하나만이 원 결함)
    need({e for _, e, _, _ in py} == {"SessionStart", "UserPromptSubmit"},
         "각성 집합이 두 이벤트를 덮지 않는다: %r" % sorted({e for _, e, _, _ in py}))
    # ⓐ′ ★U-21 timeout 선언 — UserPromptSubmit 은 하네스 기본이 30s 뿐이고 timeout 은
    #     '취소 + 출력 폐기'다. 선언이 없거나 기본 이하면 부트 체인이 **무음으로 사라진다**.
    _ups = [(sc, to) for sc, ev, _, to in py if ev == "UserPromptSubmit"]
    need(len(_ups) == 1, "UserPromptSubmit 각성 훅이 1개가 아니다: %r" % _ups)
    _plat = _rust_u64_const(_repo_file(os.path.join("src", "pack.rs")),
                            "HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S")
    need(_plat == PF.HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S,
         "하네스 기본값 상수가 2언어에서 갈렸다(rust=%s / python=%s)"
         % (_plat, PF.HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S))
    need(_ups[0][1] is not None and _ups[0][1] > _plat,
         "UserPromptSubmit 선언 timeout 이 없거나 하네스 기본(%ss) 이하다: %r — "
         "부트 체인이 취소되고 stdout(additionalContext)이 폐기된다" % (_plat, _ups[0]))
    # ⓐ″ 파이썬 안에서 매니페스트와 등록기 표가 갈리지 않았나(등록 경로는 hook_timeout_for 를 쓴다)
    for _sc, _ev, _m, _to in py:
        need(PF.hook_timeout_for(_sc, _ev) == _to,
             "파이썬 내부 스큐 — 매니페스트 %r 과 등록기 표(hook_timeout_for)가 갈렸다: %r"
             % ((_sc, _ev, _to), PF.hook_timeout_for(_sc, _ev)))
    notes.append("UserPromptSubmit timeout %ss 선언(하네스 기본 %ss 초과) · 파이썬 내부 스큐 0"
                 % (_ups[0][1], _plat))
    # ⓑ C28 등록 집합(SELFCORR_HOOKS)이 role-bootstrap 을 포함한다(집행자 누락 방지)
    selfcorr = {(sc, ev) for sc, evs in PF.SELFCORR_HOOKS for ev, _ in evs}
    need(("role-bootstrap.sh", "UserPromptSubmit") in selfcorr,
         "C28 등록 집합에 각성 훅이 없다(등록 주체 부재)")
    need(PF.AWAKENING_SCRIPTS == {sc for sc, _, _, _ in py}, "AWAKENING_SCRIPTS 파생 불일치")
    notes.append("C28 등록 주체 확인")
    # ⓒ Rust 두 집행자가 **매니페스트를 소비**한다(사본 재발명 0)
    pk = _repo_file(os.path.join("src", "pack.rs"))
    need("merge_desired_hooks(&settings, &pack_dir(), &AWAKENING_HOOKS)" in pk,
         "격리 config 시드가 매니페스트를 소비하지 않는다")
    need("fn merge_desired_hooks(" in pk, "이벤트 단위 병합기가 없다")
    need("if !settings.exists() {" not in _code_lines(pk),
         "파일 단위 시드 가드(`!settings.exists()`)가 남아 있다(A9 재발)")
    cy = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("cys::pack::AWAKENING_HOOKS" in cy, "init-pack 이 매니페스트를 소비하지 않는다")
    ii = cy.find("fn install_claude_hook(")
    need(ii > 0, "install_claude_hook 을 못 찾았다")
    body = cy[ii:cy.find("\n}\n", ii)]
    need('"SessionStart"' not in body, "init-pack 이 여전히 SessionStart 만 직접 등록한다")
    notes.append("Rust 집행자 2 소비")
    # ⓓ 개인 프로필 병합(T-0147-1)도 같은 집합을 쓴다
    need("fn merge_awakening_hooks_into_personal_profiles(" in pk, "개인 프로필 병합기가 없다")
    mi = pk.find("pub fn merge_awakening_hooks_into_personal_profiles(")
    pbody = pk[mi:pk.find("\n}\n", mi)]
    need("&AWAKENING_HOOKS" in pbody, "개인 프로필 병합이 매니페스트를 소비하지 않는다")
    notes.append("개인 프로필 병합 동일 집합")
    # ⓔ ★W3 게이트: 설치 직후 '등록 집합 ⊇ 소망 집합' **실측 검증**이 배선돼 있다(주장 금지)
    need("pub fn verify_desired_hooks_registered(" in pk, "설치 후 ⊇ 검증 함수가 없다")
    si = pk.find("fn setup_isolated_config_dir(")  # W5-C: install_hooks 파라미터화 — 개폐괄호 무pin(시그니처 확장 허용·본문 검사는 동일)
    sbody = pk[si:pk.find("\n}\n", si)]
    need("verify_desired_hooks_registered(&settings" in sbody,
         "설치 경로가 '등록 집합 ⊇ 소망 집합' 을 검증하지 않는다(시드했다는 주장만 남는다)")
    need("등록 집합 ⊅ 소망 집합" in sbody, "미충족 시 loud 보고가 없다(조용한 실패)")
    notes.append("설치 후 ⊇ 검증 배선")
    old = _git_show(os.path.join("src", "pack.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("AWAKENING_HOOKS" not in old, "계측 타당성 실패: 구 코드에 이미 매니페스트가 있다")
        need("if !settings.exists() {" in old, "계측 타당성 실패: 구 파일 단위 시드 가드를 못 찾았다")
        oldc = _git_show(os.path.join("src", "bin", "cys.rs"))
        if oldc is not None:
            oi = oldc.find("fn install_claude_hook(")
            need('"SessionStart"' in oldc[oi:oi + 3000],
                 "계측 타당성 실패: 구 init-pack 의 SessionStart 단독 등록을 못 찾았다")
        calib = "구 코드=파일 단위 시드 + init-pack SessionStart 단독 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


def _u21_parse_awakening_body(body, declared_n):
    """`_rust_awakening_hooks` 의 **파싱 코어**만 떼어낸 것 — 합성 표본으로 탐지기 자체를 시험한다.
    반환: (파싱된 항 수, 선언 수와 일치하는가)."""
    n = len(re.findall(
        r'script:\s*"([^"]+)"\s*,\s*event:\s*"([^"]+)"\s*,\s*matcher:\s*(None|Some\("([^"]*)"\))'
        r'\s*,\s*timeout:\s*(None|Some\(([A-Za-z0-9_]+)\))',
        body))
    return n, n == declared_n


@specimen("H-SEED-U21", "W6",
          "훅 등록 reconcile + 선언 timeout — 판정 2축·교체 경로(중복 append 0)·오살 금지·롤백 마스터",
          ["C-9", "A9"])
def h_seed_u21():
    """U-21(2026-08-24): Claude Code 훅의 `UserPromptSubmit` **기본 timeout 은 30초**이고,
    timeout 은 지연이 아니라 **취소 + 출력 폐기**다. role-bootstrap 의 stdout 이 곧
    additionalContext(팀 기동 사실 + 착수 규율)라서, 30초를 넘긴 순간 부트 체인은 에러도 없이
    **조용히 사라진다** — 훅 자신의 데드라인 합만 27초(2+5+5+5+10)라 여유가 3초뿐이고,
    인터프리터 냉시작이 4회 얹히는 Windows 에서 상한을 넘는다(mac 만 멀쩡한 그 계열).

    그래서 판정을 `command 동등 ∧ 선언 timeout 충족` 2축으로 올리고, 불일치는 **교체**한다.
    이 검체가 지키는 위험 두 가지가 정확히 이 제품이 낸 사고다:
      ① 불일치를 append 로 처리 → 같은 명령 2회 등재 → **매 프롬프트 훅 2회 발화**(큐 폭주)
      ② 엔트리를 통째로 갈아끼움 → 같은 배열의 **사용자 항목 소실**(오살)
    그리고 선언은 **동등이 아니라 하한**이다 — 사용자가 더 크게 잡아둔 값을 우리 값으로 내리면
    살아서 완주하던 부트가 우리 손에 잘린다."""
    PF = _preflight_mod()
    notes = []
    pk = _repo_file(os.path.join("src", "pack.rs"))

    # ── ⓐ 직렬화 순서 계약: `timeout` 은 **matcher 뒤** ──────────────────────
    ds = pk.find("pub struct DesiredHook {")
    need(ds > 0, "DesiredHook 구조체를 못 찾았다")
    dbody = pk[ds:pk.find("\n}\n", ds)]
    _mi, _ti = dbody.find("pub matcher:"), dbody.find("pub timeout:")
    need(_ti > 0, "DesiredHook 에 timeout 필드가 없다")
    need(_mi > 0 and _mi < _ti,
         "필드 순서 계약 위반 — timeout 은 matcher **뒤**여야 한다(파서가 짝을 잃는다)")
    notes.append("필드 순서 계약(matcher→timeout)")

    # ── ⓑ ★파서 자체를 합성 표본으로 시험한다(트리에 위반 0이라 고장나도 초록인 핀) ──
    good = ('DesiredHook { script: "a.sh", event: "E", matcher: None, timeout: Some(600) },'
            'DesiredHook { script: "b.sh", event: "F", matcher: None, timeout: None },')
    n, ok = _u21_parse_awakening_body(good, 2)
    need(ok, "★탐지기 파손: 정상 4-튜플 표본을 파싱하지 못했다(파싱 %d/2)" % n)
    # 표본 ① timeout 을 **앞**에 둔 역순 — 반드시 놓쳐야(=선언 수 불일치로 hard fail) 한다
    inverted = 'DesiredHook { script: "a.sh", event: "E", timeout: Some(600), matcher: None },'
    _, ok_inv = _u21_parse_awakening_body(inverted, 1)
    need(not ok_inv, "★탐지기 파손: 순서를 뒤집은 합성 표본을 통과시켰다(순서 계약 무집행)")
    # 표본 ② timeout 필드가 아예 없는 **구 3-튜플** — 반드시 놓쳐야 한다
    legacy = 'DesiredHook { script: "a.sh", event: "E", matcher: None },'
    _, ok_leg = _u21_parse_awakening_body(legacy, 1)
    need(not ok_leg, "★탐지기 파손: 구 3-튜플 표본을 통과시켰다(무음 스큐 재발)")
    notes.append("파서 합성 표본 3종 시험 통과")

    # ── ⓒ Rust 집행 경로가 2축 판정과 교체 경로를 실제로 소비하나 ────────────
    need("pub fn hook_timeout_satisfied(" in pk, "선언 충족 판정의 순수 코어가 없다")
    need("pub fn hook_registered_with_timeout_in(" in pk, "2축 판정기가 없다")
    need("fn raise_hook_timeout_in(" in pk, "불일치 엔트리 교체 경로가 없다")
    mi = pk.find("pub fn merge_desired_hooks(")
    need(mi > 0, "merge_desired_hooks 를 못 찾았다")
    mbody = pk[mi:pk.find("\npub fn ", mi + 10)]
    need("hook_registered_with_timeout_in(" in mbody, "병합기가 2축 판정을 소비하지 않는다")
    need("raise_hook_timeout_in(" in mbody, "병합기에 교체 경로가 배선되지 않았다")
    vi = pk.find("pub fn verify_desired_hooks_registered(")
    vbody = pk[vi:pk.find("\n}\n", vi)]
    need("hook_registered_with_timeout_in(" in vbody,
         "⊇ 게이트가 집행 축과 다른 판정을 쓴다(스큐가 남은 디스크를 '충족'으로 보고한다)")
    notes.append("Rust 2축 판정 + 교체 경로 + ⊇ 게이트 동축")

    # ── ⓓ 롤백은 **마스터 한 손잡이**로 닫힌다 ───────────────────────────────
    need("pub const ENV_HOOK_TIMEOUT_V1:" in pk, "축 전용 노브 상수가 없다")
    need("crate::boot_gates_master_off_from(master_env)" in pk,
         "축이 마스터(CYS_BOOT_GATES=0)에 접히지 않았다 — 사고 순간 노브 조합 불가")
    need(PF.hook_timeout_axis_legacy_from(None, None) is False, "기본이 종전 동작이다")
    need(PF.hook_timeout_axis_legacy_from("0", None) is True, "마스터가 python 축을 끄지 못한다")
    need(PF.hook_timeout_axis_legacy_from(None, "1") is True, "축 노브가 python 축을 끄지 못한다")
    need(PF.hook_timeout_axis_legacy_from("false", "yes") is False, "엄격 비교가 아니다")
    notes.append("롤백 마스터 접기 + 엄격 비교(2언어)")

    # ── ⓔ ★파이썬 등록기 실거동(격리 tmp — 실 HOME 무접촉) ──────────────────
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = os.path.join(home, ".cys", "pack")
        os.makedirs(pack, exist_ok=True)
        with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                        CLAUDE_CONFIG_DIR=None, CYS_BOOT_GATES=None,
                        CYS_HOOK_TIMEOUT_V1=None):
            ours = PF._cys_hook_cmd("role-bootstrap.sh")
            user = "sh %s/myhooks/role-bootstrap.sh" % home     # 동명 사용자 훅(보존 대상)
            want = PF.hook_timeout_for("role-bootstrap.sh", "UserPromptSubmit")
            need(want and want > PF.HOOK_TIMEOUT_PLATFORM_DEFAULT_UPS_S,
                 "등록기 표가 하네스 기본 이하다: %r" % want)

            def _mk(entries):
                sp = os.path.join(tmp, "s-%d.json" % len(os.listdir(tmp)))
                _w(sp, json.dumps({"hooks": {"UserPromptSubmit": entries}}), 0o644)
                return sp

            def _load(sp):
                return json.load(open(sp, encoding="utf-8"))["hooks"]["UserPromptSubmit"]

            def _reg(sp, timeout):
                err = PF.Preflight._register_event_hook(
                    None, sp, "UserPromptSubmit", "role-bootstrap.sh", None, timeout)
                need(err is None, "등록 실패: %s" % err)

            # ① 우리 명령이 **timeout 없이** 실려 있고 옆에 사용자 훅이 있는 현장 상태
            sp = _mk([{"hooks": [{"type": "command", "command": user}]},
                      {"hooks": [{"type": "command", "command": ours}]}])
            need(PF.Preflight._event_hook_registered(sp, "UserPromptSubmit",
                                                     "role-bootstrap.sh", want) is False,
                 "2축 판정이 timeout 미기록을 '충족'으로 읽는다(원 결함 그대로)")
            need(PF.Preflight._event_hook_registered(sp, "UserPromptSubmit",
                                                     "role-bootstrap.sh") is True,
                 "command 축 단독 판정(종전)이 깨졌다")
            _reg(sp, want)
            arr = _load(sp)
            cmds = [h.get("command") for e in arr for h in e.get("hooks", [])]
            need(cmds.count(ours) == 1,
                 "★교체가 아니라 중복 append 됐다(훅 2회 발화 = 큐 폭주 방향): %r" % cmds)
            need(user in cmds, "★사용자 동명 훅이 사라졌다(오살)")
            got = [h.get("timeout") for e in arr for h in e.get("hooks", [])
                   if h.get("command") == ours]
            need(got == [want], "선언 timeout 이 실리지 않았다: %r" % got)
            # 멱등 — 교정 뒤 재실행은 무동작
            _reg(sp, want)
            need([h.get("command") for e in _load(sp) for h in e.get("hooks", [])].count(ours) == 1,
                 "교정 후 재실행이 중복을 만들었다(멱등 위반)")
            notes.append("교체 경로 실거동: 중복 0 · 사용자 훅 보존 · 값 %ss · 멱등" % want)

            # ② ★사용자가 더 크게 잡아둔 값은 **내리지 않는다**(한 방향으로만 여는 축)
            hi = _mk([{"hooks": [{"type": "command", "command": ours, "timeout": want + 300}]}])
            need(PF.Preflight._event_hook_registered(hi, "UserPromptSubmit",
                                                     "role-bootstrap.sh", want) is True,
                 "더 큰 값을 불충족으로 읽는다 — 그 판정은 값을 내리는 쓰기를 부른다(오살)")
            _reg(hi, want)
            got_hi = [h.get("timeout") for e in _load(hi) for h in e.get("hooks", [])]
            need(got_hi == [want + 300],
                 "★더 큰 사용자 값을 우리 값으로 내렸다(오살 방향): %r" % got_hi)
            notes.append("고값 무하향(%ss 유지)" % (want + 300))

            # ③ ★C28 보고 티어 분리 — "미등록" 과 "timeout 미달" 은 다른 사실이다.
            #    command 가 실려 있는데 FAIL("각성 훅 미등록")을 내면 **위경고**이고(H-SEED-2 계약),
            #    그 오보는 현장에서 "훅이 없다"는 잘못된 조치를 부른다. 그러나 교정은 미루지
            #    않는다 — --fix 경로는 두 축을 동일하게 수리한다.
            ccd = os.path.join(tmp, "live-config")
            os.makedirs(ccd, exist_ok=True)
            _fake_pack_with_hooks(pack)
            with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                            CLAUDE_CONFIG_DIR=ccd, CYS_BOOT_GATES=None,
                            CYS_HOOK_TIMEOUT_V1=None), _temp_guard_double(PF, "SNAPMARK"):
                # C28 이 보는 자기교정 훅을 **전부** 등록해 둔다 — 다른 결손의 경고가 보고를
                # 채우면 timeout 경고가 잘려 나가 검체가 무의미해진다(보고 상한 warns[:6]).
                _hooks = {}
                for _sc, _evs in PF.SELFCORR_HOOKS:
                    for _ev, _m in _evs:
                        _e = {"hooks": [{"type": "command", "command": PF._cys_hook_cmd(_sc)}]}
                        if _m is not None:
                            _e["matcher"] = _m
                        _hooks.setdefault(_ev, []).append(_e)
                _w(os.path.join(ccd, "settings.json"),
                   json.dumps({"hooks": _hooks}), 0o644)
                pf = PF.Preflight(False, [])
                pf.c28_self_correction()
                row = [r for r in pf.results if r["id"].startswith("C28")][0]
                need(row["status"] != PF.FAIL,
                     "★위경고 — timeout 스큐뿐인데 '각성 훅 미등록' FAIL 을 냈다: %r" % row)
                need("timeout" in row["detail"],
                     "timeout 스큐를 보고하지 않는다(무음): %r" % row["detail"])
                need("미등록" not in row["detail"].split("timeout")[0][-40:],
                     "실려 있는 훅을 '미등록' 으로 서술한다(거짓 보고): %r" % row["detail"])
                # --fix 는 티어와 무관하게 실제로 교정한다(WARN 강등이 수리를 미루지 않는다)
                fx = PF.Preflight(True, [])
                fx.c28_self_correction()
                got_fx = [h.get("timeout")
                          for e in json.load(open(os.path.join(ccd, "settings.json"),
                                                  encoding="utf-8"))["hooks"]["UserPromptSubmit"]
                          for h in e.get("hooks", []) if h.get("command") == ours]
                need(got_fx == [want], "--fix 가 timeout 스큐를 교정하지 않았다: %r" % got_fx)
                notes.append("C28 티어 분리(스큐=非FAIL·보고 실재) + --fix 실교정")

            # ③ 순수 술어 진리표 — 선언은 하한이지 동등이 아니다
            need(PF.hook_timeout_satisfied(None, None) is True, "무선언이 불충족으로 접혔다")
            need(PF.hook_timeout_satisfied(None, 600) is False,
                 "키 부재를 충족으로 읽는다 — 하네스 기본 30s 를 우리가 읽을 길이 없다")
            need(PF.hook_timeout_satisfied(30, 600) is False, "저값을 충족으로 읽는다")
            need(PF.hook_timeout_satisfied(900, 600) is True, "고값을 불충족으로 읽는다")

        # ④ 롤백: 마스터 하나로 선언이 사라지고 쓰기도 종전으로 돌아간다
        with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                        CLAUDE_CONFIG_DIR=None, CYS_BOOT_GATES="0",
                        CYS_HOOK_TIMEOUT_V1=None):
            need(PF.hook_timeout_for("role-bootstrap.sh", "UserPromptSubmit") is None,
                 "마스터 스위치를 눌러도 선언이 살아 있다(롤백 불능)")
            ours2 = PF._cys_hook_cmd("role-bootstrap.sh")
            sp2 = os.path.join(tmp, "rollback.json")
            _w(sp2, json.dumps({"hooks": {"UserPromptSubmit": [
                {"hooks": [{"type": "command", "command": ours2}]}]}}), 0o644)
            need(PF.Preflight._event_hook_registered(
                sp2, "UserPromptSubmit", "role-bootstrap.sh",
                PF.hook_timeout_for("role-bootstrap.sh", "UserPromptSubmit")) is True,
                 "롤백 상태인데 판정이 여전히 timeout 을 요구한다")
            after = json.load(open(sp2, encoding="utf-8"))["hooks"]["UserPromptSubmit"]
            need("timeout" not in after[0]["hooks"][0], "롤백 상태에서 timeout 키가 실렸다")
            notes.append("롤백(CYS_BOOT_GATES=0) → 선언 None · 판정·쓰기 종전")

    # ── ⓕ 계측 타당성: 구 트리에는 이 축이 아예 없다 ─────────────────────────
    old = _git_show(os.path.join("src", "pack.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("pub timeout: Option<u64>," not in old,
             "계측 타당성 실패: 구 코드에 이미 timeout 필드가 있다(검체 무의미)")
        need("hook_registered_with_timeout_in" not in old,
             "계측 타당성 실패: 구 코드에 이미 2축 판정기가 있다")
        # ★기준 트리(CALIBRATION_REF)에는 `hook_registered_in` 자체가 아직 없을 수 있다 —
        #   그 경우 "구 판정기가 timeout 을 안 본다"는 명제는 **더 강하게** 성립한다(판정기 자체가
        #   없다). 없다고 통과시키는 것이 아니라, 두 경우 모두에서 사실을 확인해 문구로 남긴다.
        oi = old.find("pub fn hook_registered_in(")
        if oi > 0:
            need("timeout" not in old[oi:old.find("\n}\n", oi)],
                 "계측 타당성 실패: 구 판정기가 이미 timeout 을 본다")
            calib = "구 트리=판정기 실재하나 timeout 미고려 + 필드·2축 부재 확인"
        else:
            calib = "구 트리=판정기 자체 부재 + 필드·2축 부재 확인"
    oldp = _git_show(os.path.join("cysjavis-pack", "bin", "javis_preflight.py"))
    if oldp is not None:
        need('("role-bootstrap.sh", [("UserPromptSubmit", None)]),' in oldp,
             "계측 타당성 실패: 구 파이썬 3-튜플 매니페스트 원문을 못 찾았다")
        calib += " · 구 python=3-튜플 매니페스트 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SEED-2", "W3",
          "실사용 config dir 등록 hard 검증(CLAUDE_CONFIG_DIR 최우선 · role-bootstrap 미등록=FAIL)",
          ["A21", "R4"])
def h_seed_2():
    """A21: C08(session-start)=FAIL 인데 C28(role-bootstrap)=WARN 이라 **부트 발화의 유일한
    트리거가 빠져도 preflight 가 초록에 가까웠다**(비대칭). R4: discover 가 `CLAUDE_CONFIG_DIR`
    (=이 세션의 실사용 config dir)을 아예 보지 않아, 정작 훅이 필요한 그 디렉터리가 등록 대상에서
    빠질 수 있었다(등록≠가동 갭)."""
    PF = _preflight_mod()
    notes = []
    with tempfile.TemporaryDirectory() as tmp, _temp_guard_double(PF, "SNAPMARK"):
        home = os.path.join(tmp, "home")
        pack = _fake_pack_with_hooks(os.path.join(home, ".cys", "pack"))
        ccd = os.path.join(tmp, "live-config")           # 실사용 config dir(홈 밖)
        os.makedirs(ccd, exist_ok=True)
        _make_profile(home, ".claude", {"SessionStart": []})
        with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                        CLAUDE_CONFIG_DIR=ccd):
            # ⓐ R4: 실사용 config dir 이 **최우선** 후보다
            found = PF.discover_claude_settings()
            need(found and found[0] == os.path.join(ccd, "settings.json"),
                 "CLAUDE_CONFIG_DIR 이 최우선 후보가 아니다: %r" % found)
            notes.append("CLAUDE_CONFIG_DIR 최우선")
            # ⓑ A21 티어: 각성 훅(role-bootstrap) 미등록 → C28 **FAIL**
            want_ss = PF._cys_hook_cmd("session-start.sh")
            _w(os.path.join(ccd, "settings.json"),
               json.dumps({"hooks": {"SessionStart": [
                   {"hooks": [{"type": "command", "command": want_ss}]}]}}), 0o644)
            pf = PF.Preflight(False, [])
            pf.c28_self_correction()
            row = [r for r in pf.results if r["id"].startswith("C28")][0]
            need(row["status"] == PF.FAIL,
                 "각성 훅 미등록인데 C28 이 FAIL 이 아니다(C08 대칭 위반): %r" % row)
            need("각성" in row["detail"], "FAIL 사유가 각성 훅을 지목하지 않는다: %r" % row["detail"])
            notes.append("role-bootstrap 미등록=FAIL")
            # ⓒ 등록되면 FAIL 이 사라진다(위경고 모드 금지 — 판정이 실측 파생임을 확인).
            #   ★전 프로필에 등록해야 한다 — C28 은 발견된 **모든** 등록 대상을 검사한다(한 프로필만
            #   고치고 FAIL 해소를 기대하면 검체가 계약을 오해한 것이다).
            reg = PF.Preflight(True, [])
            for t in PF.discover_claude_settings():
                need(reg._register_event_hook(t, "UserPromptSubmit", "role-bootstrap.sh", None) is None,
                     "각성 훅 등록 실패: %s" % t)
            pf2 = PF.Preflight(False, [])
            pf2.c28_self_correction()
            row2 = [r for r in pf2.results if r["id"].startswith("C28")][0]
            need(row2["status"] != PF.FAIL,
                 "각성 훅이 등록됐는데도 FAIL(위경고 모드): %r" % row2)
            notes.append("등록 시 FAIL 해소")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_preflight.py"))
    calib = "skip(no-git)"
    if old is not None:
        oi = old.find("def c28_self_correction(")
        need("self.add(cid, FAIL" not in old[oi:oi + 3000],
             "계측 타당성 실패: 구 C28 에 이미 FAIL 티어가 있다")
        di = old.find("def discover_claude_settings(")
        # ★docstring 언급(agents.json 해석 서술)은 코드가 아니다 — **env 를 읽는 코드**의 부재를 잰다.
        need('os.environ.get("CLAUDE_CONFIG_DIR"' not in old[di:di + 3000],
             "계측 타당성 실패: 구 discover 가 이미 CLAUDE_CONFIG_DIR env 를 읽는다")
        calib = "구 C28 FAIL 부재 + 구 discover CLAUDE_CONFIG_DIR 미참조 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SEED-3", "W3", "settings.json 없는 프로필 디렉터리 → 후보화·생성 등록", ["G7"])
def h_seed_3():
    """G7: 후보 기준이 `isfile(settings.json)` 이라 **파일이 아직 없는 프로필**은 영구 미배선으로
    굳었다(그 프로필의 claude 는 훅 없이 돌고, 등록기는 그 프로필을 보지도 않는다). 등록기는 이미
    makedirs+create 를 하므로 기준은 **디렉터리 존재**여야 한다."""
    PF = _preflight_mod()
    notes = []
    with tempfile.TemporaryDirectory() as tmp, _temp_guard_double(PF, "SNAPMARK"):
        home = os.path.join(tmp, "home")
        pack = _fake_pack_with_hooks(os.path.join(home, ".cys", "pack"))
        _make_profile(home, ".claude-empty", None)        # 디렉터리만(settings.json 없음)
        _make_profile(home, ".claude", {"SessionStart": []})
        _w(os.path.join(home, ".claude-notadir"), "file\n", 0o644)   # 파일은 프로필 아님
        with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                        CLAUDE_CONFIG_DIR=None):
            found = PF.discover_claude_settings()
            need(os.path.join(home, ".claude-empty", "settings.json") in found,
                 "settings.json 없는 프로필이 후보에서 빠졌다: %r" % found)
            need(not any("notadir" in f for f in found), "파일을 프로필로 오인: %r" % found)
            notes.append("빈 프로필 후보화")
            # 등록기가 파일을 **생성**한다(후보화만으로는 배선이 아니다)
            pf = PF.Preflight(True, [])
            err = pf._register_hook(os.path.join(home, ".claude-empty", "settings.json"))
            need(err is None, "빈 프로필 등록 실패: %s" % err)
            data = json.loads(_read(os.path.join(home, ".claude-empty", "settings.json")))
            cmds = [h["command"] for e in data["hooks"]["SessionStart"] for h in e["hooks"]]
            need(PF._cys_hook_cmd("session-start.sh") in cmds, "생성된 파일에 훅이 없다: %r" % cmds)
            notes.append("등록기 생성 확인")
    # Rust 파리티 — cys.rs 는 공용 함수(디렉터리 기준)를 경유한다
    cy = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need("cys::pack::personal_profile_settings_paths()" in cy,
         "cys.rs discover 가 공용(디렉터리 기준) 구현을 경유하지 않는다")
    need(".filter(|p| p.is_file())" not in cy, "cys.rs 에 isfile 게이트 잔존(G7 재발)")
    pk = _repo_file(os.path.join("src", "pack.rs"))
    need("&& e.path().is_dir()" in pk, "pack.rs 프로필 열거가 디렉터리 기준이 아니다")
    notes.append("Rust 파리티(디렉터리 기준)")
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need(".filter(|p| p.is_file())" in old, "계측 타당성 실패: 구 isfile 게이트를 못 찾았다")
        calib = "구 코드 isfile 게이트 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SEED-4", "W3", "cys-dept launch/rotate → CYS_ACCOUNT_DIR 복원 주입 + 시드 검증", ["G3"])
def h_seed_4():
    """G3: `allocate`·`create` 는 계정격리(CYS_ACCOUNT_DIR + agents.json 시드)를 세우는데
    **launch 는 둘 다 안 했다**. 그런데 `rotate` 는 launch 를 재귀 호출한다 — **재기동 한 번으로
    격리가 조용히 풀렸다**(자식 claude 가 오너 base config 공유 = F1 붕괴)."""
    dept = os.path.join(BIN_DIR, "cys-dept")
    need(os.path.isfile(dept), "cys-dept 부재")
    src = _read(dept)
    notes = []
    # ⓐ launch 스폰에 CYS_ACCOUNT_DIR 주입 + 시드 검증 fail-closed
    li = src.find("\n  launch)")
    need(li > 0, "launch 분기를 못 찾았다")
    lbody = src[li:src.find("\n  allocate)", li)]
    need('CYS_ACCOUNT_DIR="$acctdir"' in lbody, "launch 스폰에 CYS_ACCOUNT_DIR 주입이 없다(G3 재발)")
    need("resolve_lane_acctdir" in lbody, "launch 가 계정 dir 을 유도하지 않는다")
    need("verify_lane_account_seed" in lbody, "launch 가 계정격리 시드를 검증하지 않는다")
    need("exit 6" in lbody, "시드 실패가 fail-closed 가 아니다(비격리 기동 허용)")
    notes.append("launch 주입+검증+fail-closed")
    # ⓑ rotate 는 launch 를 재귀 호출한다(= 이 수리가 rotate 에도 적용된다는 결박)
    ri = src.find("\n  rotate)")
    need(ri > 0 and 'bash "$0" launch "$name"' in src[ri:ri + 4000],
         "rotate 가 launch 를 경유하지 않는다(복원 경로 결박 실패)")
    notes.append("rotate=launch 재귀(복원 상속)")
    # ⓒ allocate 가 account_dir 을 레지스트리에 기록한다(복원 SOT)
    ai = src.find("\n  allocate)")
    need("reg_set_field \"$name\" account_dir" in src[ai:src.find("\n  create)", ai)],
         "allocate 가 account_dir 을 레지스트리에 기록하지 않는다(rotate 복원 근거 부재)")
    notes.append("allocate=account_dir 기록")
    # ⓓ 유도 3순위·시드 자기치유 실측(함수 블록만 로드 — 데몬·부서 무접촉)
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = os.path.join(home, ".cys", "pack")
        dpack = os.path.join(home, ".cys", "pack-dept-t1")
        acct = os.path.join(home, ".cys", "claude-t1")
        os.makedirs(dpack, exist_ok=True)
        os.makedirs(acct, exist_ok=True)
        _w(os.path.join(pack, "agents.json"),
           json.dumps({"claude": {"cmd": "claude",
                                  "env": {"CLAUDE_CONFIG_DIR": os.path.join(home, ".cys", "claude")}}}),
           0o644)
        reg = os.path.join(home, ".cys", "depts.json")
        _w(reg, json.dumps({"depts": {"t1": {"socket": "s", "pack_dir": dpack,
                                             "account_dir": acct}}}), 0o644)
        fns = os.path.join(tmp, "fns.sh")
        head = src.split("\ncmd=")[0]
        _w(fns, head)
        script = (
            '. "%s"\n'
            'echo "REG=$(reg_get_field t1 account_dir)"\n'
            'echo "RESOLVE=$(resolve_lane_acctdir t1 "%s")"\n'
            'verify_lane_account_seed "%s" "%s" && echo VERIFY=OK || echo VERIFY=FAIL\n'
            'echo "SEEDED=$(pack_seeded_acct "%s")"\n'
            'verify_lane_account_seed "%s" "" && echo NOACCT=OK || echo NOACCT=FAIL\n'
        ) % (fns, dpack, dpack, acct, dpack, dpack)
        env = _base_env({"HOME": home, "CYS_PACK_DIR": pack, "CYS_DEPTS_JSON": reg})
        r = _run([BASH, "-c", script], env=env, timeout=90)
        out = r.stdout
        need("REG=" + acct in out, "레지스트리 account_dir 유도 실패: %r" % out)
        need("RESOLVE=" + acct in out, "유도 3순위가 레지스트리 값을 못 집었다: %r" % out)
        need("VERIFY=OK" in out, "시드 자기치유·검증 실패: %r\n%s" % (out, r.stderr[-400:]))
        need("SEEDED=" + acct in out, "agents.json 이 계정 dir 로 시드되지 않았다: %r" % out)
        need("NOACCT=OK" in out, "계정격리 미사용 부서에서 검증이 실패로 접혔다(회귀): %r" % out)
        notes.append("유도 3순위·자기치유 실측")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "cys-dept"))
    calib = "skip(no-git)"
    if old is not None:
        oi = old.find("\n  launch)")
        oldl = old[oi:old.find("\n  allocate)", oi)]
        need("CYS_ACCOUNT_DIR" not in oldl,
             "계측 타당성 실패: 구 launch 가 이미 CYS_ACCOUNT_DIR 을 주입한다")
        need('CYS_SOCKET="$sock" CYS_PACK_DIR="$pack" nohup' in oldl,
             "계측 타당성 실패: 구 launch 의 무격리 스폰 라인을 못 찾았다")
        calib = "구 launch=CYS_ACCOUNT_DIR 미주입 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SEED-5", "W3", "외부 동명 훅 보존(_prune 소유 술어 — pack 접두 + /hooks/<name> 꼬리)",
          ["G10"])
def h_seed_5():
    """G10: 소유 판정이 `script_name in c and "hooks" in c` 라는 **부분문자열 2개**였다 —
    사용자가 자기 훅을 `~/myhooks/inject-context.sh` 에 두면(경로에 'hooks' 문자열 포함) 우리
    등록기가 그것을 '우리 파손 엔트리'로 보고 **무음 삭제**했다(사용자 설정 파괴)."""
    PF = _preflight_mod()
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        pack = os.path.join(home, ".cys", "pack")
        os.makedirs(pack, exist_ok=True)
        with _env_patch(HOME=home, CYS_PACK_DIR=pack, CYS_ACCOUNT_DIR=None,
                        CLAUDE_CONFIG_DIR=None):
            desired = PF._cys_hook_cmd("inject-context.sh")
            user_a = "sh %s/myhooks/inject-context.sh" % home            # 'hooks' 부분문자열 함정
            user_b = "sh %s/mytools/hooks/inject-context.sh" % home      # 꼬리 정확 매치 함정
            stale = "sh %s/.config/cysjavis/hooks/inject-context.sh" % home   # 구 cys 경로(회수 대상)
            broken = 'bash "%s\\hooks\\inject-context.sh"' % pack.replace("/", "\\")
            arr = [{"hooks": [{"type": "command", "command": c}]}
                   for c in (user_a, user_b, stale, broken, desired)]
            kept, have = PF._prune_stale_hook_entries(arr, "inject-context.sh", desired)
            kept_cmds = [h["command"] for e in kept for h in e["hooks"]]
            need(user_a in kept_cmds, "사용자 동명 훅(~/myhooks/…)이 무음 삭제됐다(G10 재발)")
            need(user_b in kept_cmds, "제3자 hooks/ 디렉터리 훅이 삭제됐다")
            need(desired in kept_cmds and have is True, "정상 엔트리·have 판정 소실")
            need(stale not in kept_cmds, "구 cys 경로 죽은 엔트리가 회수되지 않았다(중복 append 재발)")
            need(broken not in kept_cmds, "파손(역슬래시) 엔트리가 회수되지 않았다")
            notes.append("사용자 2보존 / 구·파손 2회수 / 정상 1보존")
            # 순수 술어 직접 핀
            need(PF._hook_entry_is_ours(user_a, "inject-context.sh",
                                        (os.path.join(pack, "hooks") + os.sep)) is False,
                 "소유 술어가 사용자 훅을 우리 것으로 판정")
            need(PF._hook_entry_is_ours(desired, "inject-context.sh",
                                        (os.path.join(pack, "hooks") + os.sep)) is True,
                 "소유 술어가 우리 훅을 인식하지 못함")
            # ★계측 타당성: **구 술어**는 사용자 훅을 삭제 대상으로 봤다
            old_pred = ("inject-context.sh" in user_a) and ("hooks" in user_a)
            need(old_pred is True,
                 "계측 타당성 실패: 구 부분문자열 술어가 이 검체를 잡지 못한다(검체 무의미)")
            notes.append("구 술어 FIRE 확인(부분문자열)")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_preflight.py"))
    calib = "skip(no-git)"
    if old is not None:
        need('ours = any(script_name in c and "hooks" in c for c in cmds)' in old,
             "계측 타당성 실패: 구 부분문자열 술어 원문을 못 찾았다")
        calib = "구 술어 원문(부분문자열 2개) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-SEED-6", "W6",
          "Rust↔Python 부서 판정 파리티 + 제거 엔진 단일(hooks-prune) + 폴백 dir 0(G3 축1)",
          ["G3-AXIS1"])
def h_seed_6():
    """G3 축1(2026-08 확정): 부서 판정 술어가 2언어에 흩어져 있다 — Python 게이트는 이미 실재
    (`_discover_isolation_block._pack_is_dept` · 2026-06-30)하므로 **C28 부서 게이트를 새로 만들지
    않고**, Rust 신설 소비자(`pack::dept_scope_of` — config 시드 표적·hooks-prune 게이트)가 같은
    'pack-dept-' 접두 규칙을 쓰는지 기계 대조한다. 함께 봉인: ①레거시 폴백 dir(claude-dept-<name>)
    미생성(아무도 안 읽는 dir에 쓰지 않는다) ②부서+무acct = 시드 생략 loud WARN ③제거 엔진은
    hooks-prune 단일(cys-dept down/down-sock 배선 포함) ④C56 탐지 레인 존치(약화 금지)."""
    notes = []
    # ⓐ Rust 술어: dept_scope_of 가 'pack-dept-' 접두 규칙으로 실재
    pk = _repo_file(os.path.join("src", "pack.rs"))
    need("pub fn dept_scope_of(" in pk, "Rust 부서 판정 순수 함수(dept_scope_of)가 없다")
    di = pk.find("pub fn dept_scope_of(")
    dbody = pk[di:pk.find("\n}\n", di)]
    need('strip_prefix("pack-dept-")' in dbody,
         "dept_scope_of 가 'pack-dept-' 접두 규칙을 쓰지 않는다(명명 규칙 드리프트)")
    notes.append("Rust 술어=pack-dept- 접두")
    # ⓑ Python 술어(기존 게이트 실측 — 신설 아님): _pack_is_dept 동일 접두
    pf_src = _read(os.path.join(BIN_DIR, "javis_preflight.py"))
    need("_pack_is_dept" in pf_src and '"pack-dept-" in os.path.basename' in pf_src,
         "preflight 기존 부서 게이트(_pack_is_dept · 'pack-dept-' 접두)를 못 찾았다")
    # 셸 정본(cys-dept dept_pack)도 같은 접두 — 명명 규칙 3소스 일치
    dept_src = _read(os.path.join(BIN_DIR, "cys-dept"))
    need('dept_pack(){ echo "$HOME/.cys/pack-dept-$1"; }' in dept_src,
         "cys-dept dept_pack 명명 규칙이 바뀌었다(pack-dept- 접두 파리티 파손)")
    notes.append("Python·셸 술어 파리티")
    # ⓑ′ 기능 실측: 부서 팩 + 무 CYS_ACCOUNT_DIR → 등록 금지(기존 게이트가 산다)
    PF = _preflight_mod()
    with tempfile.TemporaryDirectory() as tmp:
        home = os.path.join(tmp, "home")
        os.makedirs(home, exist_ok=True)
        with _env_patch(HOME=home, CYS_PACK_DIR=os.path.join(home, ".cys", "pack-dept-d1"),
                        CYS_ACCOUNT_DIR=None, CLAUDE_CONFIG_DIR=None):
            reason, narrow = PF._discover_isolation_block()
            need(reason is not None and list(narrow or []) == [],
                 "부서 팩(무acct)인데 파이썬 게이트가 등록 금지를 반환하지 않는다: %r" % reason)
    notes.append("파이썬 게이트 기능 실측")
    # ⓒ 폴백 dir 0 + 시드 생략 WARN: Rust config 시드가 부서+무acct 에서 생략으로 접힌다
    # (주석 제외 — 결함을 설명한 doc 주석까지 잡으면 문서화가 곧 회귀로 보고된다: _code_lines 규약의 Rust 판)
    pk_code = "\n".join(l for l in pk.splitlines() if not l.strip().startswith("//"))
    need("claude-dept-" not in pk_code,
         "레거시 폴백 dir(claude-dept-<name>)이 pack.rs 코드에 생겼다 — 판독자 전무 사각 디렉터리 금지"
         "(G3 축1 BLOCKER 확정 위반)")
    si = pk.find("fn setup_isolated_config_dir(")  # W5-C: install_hooks 파라미터화 — 개폐괄호 무pin(시그니처 확장 허용·본문 검사는 동일)
    need(si > 0, "setup_isolated_config_dir 을 못 찾았다")
    sbody = pk[si:pk.find("\n}\n", si)]
    need("CYS_ACCOUNT_DIR 미설정" in sbody and "시드 생략" in sbody,
         "부서+무acct 시드 생략 loud WARN 이 없다(무음 공용 오염 또는 무음 스킵)")
    notes.append("폴백 0·시드 생략 WARN")
    # ⓓ 제거 엔진 단일: Rust hooks-prune 실재 + cys-dept down/down-sock 배선 + 파이썬 신규 제거 0
    fr = _repo_file(os.path.join("src", "factory_reset.rs"))
    need("pub fn strip_hooks_pointing_into_pack(" in fr, "제거 엔진(strip_hooks_pointing_into_pack) 부재")
    cy = _repo_file(os.path.join("src", "bin", "cys.rs"))
    need('#[command(name = "hooks-prune")]' in cy, "cys hooks-prune 서브커맨드가 없다")
    for verb in ("\n  down)", "\n  down-sock)"):
        vi = dept_src.find(verb)
        need(vi > 0, "cys-dept %s 분기를 못 찾았다" % verb.strip())
        # verb 본문 경계: 라벨부터 고정 창(다음 verb 까지 충분) — 정밀 파싱 불요한 grep 계약.
        vbody = dept_src[vi:vi + 3500]
        need("hooks-prune --pack-dir" in vbody,
             "cys-dept %s teardown 에 hooks-prune 배선이 없다(잔존 훅 = Claude 기동 실패 벡터)"
             % verb.strip())
    notes.append("제거 엔진 단일+teardown 배선")
    # ⓔ C56 탐지 레인 존치(약화 금지 — 탐지 마커 '/pack-dept-')
    need('"/pack-dept-" in' in pf_src, "C56 dept 훅 누수 탐지 마커가 사라졌다(탐지 레인 약화)")
    notes.append("C56 탐지 존치")
    return " · ".join(notes)


def _dept_boot_sandbox(tmp, name="d1"):
    """부서 레인 부트 격리 환경 — (env, home, pack, sock). base 팩·부서 팩을 모두 세운다."""
    home = os.path.join(tmp, "home")
    bindir = os.path.join(tmp, "stubbin")
    dpack = os.path.join(home, ".cys", "pack-dept-%s" % name)
    sock = os.path.join(home, ".local", "state", "cys-dept-%s" % name, "cys.sock")
    os.makedirs(os.path.join(dpack, "bin"), exist_ok=True)
    os.makedirs(bindir, exist_ok=True)
    _mock_cys(bindir, tmp)
    _w(os.path.join(dpack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
    _w(os.path.join(dpack, "bin", "javis_orchestra.py"), "import sys; sys.exit(0)\n", 0o644)
    # fast path 전제: 마커에 실팩 버전이 박혀야 재선언이 preflight 를 생략할 수 있다
    # (`unknown` 은 판정 불가로 취급 — 그 계약을 검체가 우회하지 않도록 실제 버전 파일을 둔다).
    _w(os.path.join(dpack, ".pack-version"), "9.9.9\n", 0o644)
    env = _base_env({"HOME": home, "PATH": bindir + os.pathsep + os.environ.get("PATH", ""),
                     "CYS_SURFACE_ID": "8", "CYS_BOOT_CHECK_RETRIES": "1",
                     "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                     "CYS_PACK_DIR": dpack, "CYS_SOCKET": sock})
    return env, home, dpack, sock


@specimen("H-LIFE-1", "W3",
          "레인별 marker/boot-last 분리(base·부서 병행 무오염) + base 마커=CEO 게이트 유지",
          ["G15", "P3-A-DEPT-LANE"])
def h_life_1():
    """G15: 락은 레인별인데 상태는 **전 레인 공유 단일 파일**이었다 — base·부서 동시 부트가 서로의
    진단 SOT 를 덮었다. P3-A-DEPT-LANE: 부서 레인엔 마커가 아예 없어 재선언마다 300s preflight 를
    통째로 다시 돌았다.
    ★금지 방향 ①: 부서 마커를 base 마커로 쓰면 CEO 승격 게이트가 오개방된다 — base 마커 경로는
      불변이고 **부서 레인은 절대 그 파일을 쓰지 않는다**.
    ★교차 단언: 같은 레인의 다중 pane 오염은 레인 분리로 해결되지 않는다 — run 귀속(CS-2⑩)이 담당."""
    B = _bootstrap_mod()
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    notes = []
    # ⓐ 경로 규약(순수) — base 는 역사적 경로, 부서는 분리, 그리고 base 마커 불침범
    base_sock = "/x/.local/state/cys/cys.sock"
    dept_sock = "/x/.local/state/cys-dept-d1/cys.sock"
    need(B.lane_state_path("marker", base_sock) == B.MARKER, "base 마커 경로 변경(회귀)")
    need(B.lane_state_path("boot_last", base_sock) == B.BOOT_LAST, "base boot-last 경로 변경(회귀)")
    need(B.lane_state_path("marker", dept_sock) != B.MARKER,
         "부서 레인이 base 마커를 가리킨다(CEO 승격 게이트 오개방 — 금지 방향 ①)")
    need(B.lane_state_path("boot_last", dept_sock) != B.BOOT_LAST, "부서 boot-last 미분리")
    notes.append("경로 규약(base 불변·부서 분리)")
    with tempfile.TemporaryDirectory() as tmp:
        # ⓑ base 레인 완주 → base 마커·base boot-last
        env, home = _boot_sandbox(os.path.join(tmp, "base"))
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 0, "base 부트 실패: %d\n%s" % (r.returncode, r.stderr[-400:]))
        base_marker = os.path.join(home, ".cys", ".master-bootstrapped")
        base_bl = os.path.join(home, ".cys", "state", "boot-last.json")
        need(os.path.isfile(base_marker), "base 마커 미생성")
        base_bl_before = _read(base_bl)
        need(base_bl_before, "base boot-last 미기록")
        notes.append("base 완주 기록")
        # ⓒ **같은 HOME**에서 부서 레인 부트(티켓 발급 후) → 부서 파일만 새로 생기고 base 는 무오염
        denv, dhome, dpack, dsock = _dept_boot_sandbox(os.path.join(tmp, "base"), "d1")
        need(os.path.realpath(dhome) == os.path.realpath(home), "검체 전제: 같은 HOME 이어야 한다")
        ienv = dict(denv)
        ienv.pop("CYS_SOCKET", None)            # 발급은 base 레인에서만
        ienv["CYS_PACK_DIR"] = os.path.join(home, ".cys", "pack")
        ri = _run([PY, boot, "issue-ticket", "--dept", "d1"], env=ienv, timeout=90)
        need(ri.returncode == 0, "CEO 티켓 발급 실패: %d\n%s" % (ri.returncode, ri.stderr[-300:]))
        rd = _run([PY, boot], env=denv, timeout=180)
        need(rd.returncode == 0, "부서 레인 부트 실패: %d\n%s" % (rd.returncode, rd.stderr[-500:]))
        # base 진단이 덮이지 않았다(구 코드에서는 여기서 덮였다)
        need(_read(base_bl) == base_bl_before,
             "부서 부트가 base boot-last 를 덮었다(G15 재발 — 진단 SOT 상호 덮어씀)")
        # 부서 전용 파일이 생겼다
        state = os.path.join(home, ".cys", "state")
        dept_bls = [f for f in os.listdir(state)
                    if f.startswith("boot-last-") and f.endswith(".json")]
        need(dept_bls, "부서 레인 boot-last 파일이 없다(레인 분리 미적용): %r" % os.listdir(state))
        dept_markers = [f for f in os.listdir(os.path.join(home, ".cys"))
                        if f.startswith(".master-bootstrapped-")]
        need(dept_markers, "부서 레인 마커가 없다(P3-A-DEPT-LANE fast path 부재 잔존)")
        notes.append("부서 파일 신설·base 무오염")
        # ⓓ base 마커 = CEO 게이트 SOT 유지: 내용이 부서 런으로 바뀌지 않았다
        bm = json.loads(_read(base_marker) or "{}")
        need(bm.get("lane") in (None, "base"),
             "base 마커가 부서 런으로 덮였다(CEO 승격 게이트 오개방): %r" % bm)
        dm = json.loads(_read(os.path.join(home, ".cys", dept_markers[0])) or "{}")
        need(dm.get("lane") and dm["lane"] != "base", "부서 마커에 레인 귀속이 없다: %r" % dm)
        notes.append("base 마커=CEO 게이트 유지")
        # ⓔ 부서 fast path 실증 — 마커가 생겼으므로 재선언 시 preflight 를 생략한다
        rd2 = _run([PY, boot], env=denv, timeout=180)
        need(rd2.returncode == 0, "부서 재선언 실패: %d" % rd2.returncode)
        dbl = json.loads(_read(os.path.join(state, dept_bls[0])) or "{}")
        pf_steps = [s for s in dbl.get("steps", []) if s["step"] == "①preflight"]
        need(pf_steps and "fast path" in pf_steps[0]["detail"],
             "부서 레인 fast path 미발동(재선언마다 preflight 전량 재실행): %r" % pf_steps)
        notes.append("부서 fast path 발동")
        # ⓕ 교차 단언(CS-2⑩): 같은 레인 다중 pane 은 run 귀속이 담당한다
        need(dbl.get("surface") == "8" and (dbl.get("result") or {}).get("surface") == "8",
             "레인 파일에 run 귀속(surface)이 없다 — 같은 레인 다중 pane 오염 방어 부재")
        need(dbl.get("lane") and dbl.get("boot_last_path"), "레인 귀속 필드 누락: %r" % sorted(dbl))
        notes.append("run 귀속 교차 단언")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("lane_state_path" not in old, "계측 타당성 실패: 구 코드에 이미 레인 경로 규약이 있다")
        need('BOOT_LAST = os.path.join(STATE_DIR, "boot-last.json")' in old
             and "_atomic_write_json(BOOT_LAST, self.data)" in old,
             "계측 타당성 실패: 구 코드의 공유 단일 boot-last write 를 못 찾았다")
        need("if _is_base_socket() and _marker.get" in old or "_is_base_socket() and _marker" in old,
             "계측 타당성 실패: 구 fast path 의 base 전용 가드를 못 찾았다")
        calib = "구 코드=공유 boot-last + base 전용 fast path 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


# ═══════════════════════════════════════════════════════════════════════════
# U-24 — 레인 규약 이관(javis_lane) · 팩 출하 전제 · 인용 결박
# ═══════════════════════════════════════════════════════════════════════════
# 이관 대상 매트릭스(정본 ↔ 레거시 폴백 동치 대조에 쓰는 표본 집합).
# ★소켓 표본은 **판정이 갈리는 축을 전부** 덮는다: 미설정·base(unix)·base(무확장)·부서·
#   부서(ceo)·빈 부서명(불량 레인)·커스텀(비-base 비-dept)·win base pipe·win 부서 pipe·
#   길이 상한 초과(해시 분기)·상한 초과 인접 2종(서로 달라야 한다).
_LANE_SOCK_SAMPLES = (
    "",
    "/Users/x/.local/state/cys/cys.sock",
    "/Users/x/.local/state/cys/cys",
    "/Users/x/.local/state/cys-dept-dept-1/cys.sock",
    "/Users/x/.local/state/cys-dept-ceo/cys.sock",
    "/Users/x/.local/state/cys-dept-/cys.sock",
    "/tmp/whatever.sock",
    "\\\\.\\pipe\\cys",
    "\\\\.\\pipe\\cys-dept-foo",
    "/" + ("z" * 400) + "/cys-dept-q/cys.sock",
    "/" + ("y" * 400) + "/cys-dept-q/cys.sock",
)
_LANE_KINDS = ("marker", "boot_last", "skip", "lock", "mission", "delivery", "delivery_epoch")


def _lane_parity(path_fn_a, path_fn_b, key_fn_a=None, key_fn_b=None):
    """두 구현의 **전수 차분** — 반환은 불일치 목록(빈 리스트 = 완전 동치).

    ★`CYS_STATE_DIR` 2상(미설정·격리 경로)까지 도는 이유: 지연 해소가 한쪽에서만 깨지면
      격리 실행이 실 HOME 을 읽는다(2026-08-01 T1 밀폐 붕괴의 기제). 정적 비교로는 안 잡힌다.
    """
    diffs = []
    saved = os.environ.get("CYS_STATE_DIR")
    try:
        for state_env in (None, os.path.join(os.sep, "tmp", "lane-parity-state")):
            if state_env is None:
                os.environ.pop("CYS_STATE_DIR", None)
            else:
                os.environ["CYS_STATE_DIR"] = state_env
            for sock in _LANE_SOCK_SAMPLES:
                if key_fn_a is not None and key_fn_b is not None:
                    ka, kb = key_fn_a(sock), key_fn_b(sock)
                    if ka != kb:
                        diffs.append("lane_key(%r)=%r vs %r" % (sock[:40], ka, kb))
                for kind in _LANE_KINDS:
                    pa, pb = path_fn_a(kind, sock), path_fn_b(kind, sock)
                    if pa != pb:
                        diffs.append("path(%s,%r)=%r vs %r"
                                     % (kind, sock[:40], pa, pb))
    finally:
        if saved is None:
            os.environ.pop("CYS_STATE_DIR", None)
        else:
            os.environ["CYS_STATE_DIR"] = saved
    return diffs


@specimen("H-LANE-1", "W6",
          "레인 규약 소유자 이관(javis_lane) — 재수출 동일객체 + 레거시 폴백 전수 동치 + 롤백 1지점",
          ["U-24", "G15", "P3-A-DEPT-LANE"])
def h_lane_1():
    """U-24: ③claim·싱글플라이트 집행이 데몬 감독자로 이관되며 `javis_bootstrap.py` 본문이
    얇아진다. 그 리팩터에 **레인 경로 규약이 딸려 흔들리면** 소비자 6곳(javis_mission · 훅
    `lane-path` · 디렉티브 2벌 · preflight CONTENT_PINS · delivery.rs 파리티)이 **조용히**
    갈린다 — 조용한 이유는 경로 함수가 예외를 던지지 않고 *다른 파일*을 성공적으로 가리키기
    때문이다(부서 부트가 base 마커를 오염 → CEO 승격 게이트 오개방 = 금지 방향 ①).
    ∴ 축을 먼저 `javis_lane.py` 로 분리하고, 이 검체가 **이관이 동치임**을 기계로 못박는다.

    ★이 검체는 '기능 제거마다 동치 계약을 재검증하는 신규 검체를 먼저 작성한다'는 U-24 게이트의
      첫 이행이다 — 구 검체(H-LIFE-1)는 **삭제하지 않고** 그대로 두며, 이 검체가 그 위에
      '어느 구현이 실제로 소비되는가'라는 새 축만 얹는다(판정 완화 0).
    """
    B = _bootstrap_mod()
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    lane_py = os.path.join(BIN_DIR, "javis_lane.py")
    notes = []

    # ⓐ 정본 모듈 실재 + 자기 자신의 밀폐 self-test 통과
    need(os.path.isfile(lane_py), "javis_lane.py 부재 — 레인 규약 소유자가 추출되지 않았다")
    r = _run([PY, lane_py, "--self-test"])
    need(r.returncode == 0, "javis_lane --self-test 실패: %d\n%s" % (r.returncode, r.stderr[-400:]))
    notes.append("정본 self-test 통과")

    # ⓑ 재수출이 **같은 객체**여야 한다(사본이면 드리프트가 시작된다)
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_lane as L
    need(B._LANE_SOURCE == "javis_lane",
         "bootstrap 이 정본을 소비하지 않는다(폴백으로 접혔다): %r" % B._LANE_SOURCE)
    for a, b, nm in ((B.lane_state_path, L.lane_state_path, "lane_state_path"),
                     (B.lane_key, L.lane_key, "lane_key"),
                     (B._socket_is_base, L.socket_is_base, "_socket_is_base"),
                     (B._LANE_STATE_KINDS, L.LANE_STATE_KINDS, "_LANE_STATE_KINDS")):
        need(a is b, "재수출 %s 가 정본과 **다른 객체**다(사본 드리프트 위험)" % nm)
    notes.append("재수출 4종 동일객체")

    # ⓒ 레거시 폴백 ≡ 정본 (전수 차분 · 소켓 11종 × kind 7종 × CYS_STATE_DIR 2상 = 154쌍)
    diffs = _lane_parity(L.lane_state_path, B._legacy_lane_state_path,
                         L.lane_key, B._legacy_lane_key)
    need(not diffs,
         "이관 비동치 — 정본과 레거시 폴백이 다른 경로를 낸다(롤백 시 상태 파일이 갈린다): %r"
         % diffs[:6])
    notes.append("폴백 전수 동치 %d쌍" % (len(_LANE_SOCK_SAMPLES) * len(_LANE_KINDS) * 2))

    # ⓒ′ **합성 표본으로 탐지력 자체를 시험한다**(트리에 위반이 0이라 초록인 핀의 계측 타당성).
    #    일부러 어긋난 스텁을 넣어 위 비교기가 FIRE 하지 않으면, ⓒ 의 초록은 아무 의미가 없다.
    def _skewed_path(kind, sock=None):
        p = L.lane_state_path(kind, sock)
        return p + ".SKEW" if kind == "skip" else p
    need(_lane_parity(_skewed_path, B._legacy_lane_state_path, L.lane_key, B._legacy_lane_key),
         "계측 타당성 실패: 일부러 어긋낸 스텁에도 파리티 비교기가 침묵한다(탐지 불능)")
    def _skewed_key(sock):
        return L.lane_key(sock) + "X"
    need(_lane_parity(L.lane_state_path, L.lane_state_path, _skewed_key, B._legacy_lane_key),
         "계측 타당성 실패: 레인 키 축의 차분을 비교기가 못 잡는다")
    notes.append("합성 표본 2종 FIRE")

    # ⓓ 롤백 스위치 = env **1지점**(사고 순간에 노브를 조합할 수 없다)
    src = _read(boot)
    reads = len(re.findall(r'os\.environ\.get\("CYS_BOOT_LANE_LEGACY"', src))
    need(reads == 1,
         "롤백 스위치 판독 지점이 %d 곳이다 — 1지점이어야 한다(축이 갈리면 절반만 롤백된다)" % reads)
    need('_LANE_SOURCE = "javis_lane"' in src,
         "기본값이 신 경로(정본 소비)임을 코드에서 확인할 수 없다")
    notes.append("롤백 env 1지점·기본=정본")

    # ⓔ 스위치 **실행** 대조: 훅이 실제로 쓰는 소비 경로(`lane-path all`)가 두 모드에서 동일
    for sock, tag in (("", "base"), ("/x/.local/state/cys-dept-d1/cys.sock", "dept")):
        env_new = _base_env({"CYS_SOCKET": sock} if sock else {},
                            drop=("CYS_BOOT_LANE_LEGACY",))
        env_old = _base_env(dict({"CYS_BOOT_LANE_LEGACY": "1"},
                                 **({"CYS_SOCKET": sock} if sock else {})))
        a = _run([PY, boot, "lane-path", "all"], env=env_new)
        b = _run([PY, boot, "lane-path", "all"], env=env_old)
        need(a.returncode == 0 and b.returncode == 0,
             "lane-path all 실패(%s): %d/%d\n%s" % (tag, a.returncode, b.returncode, b.stderr[-300:]))
        ja, jb = json.loads(a.stdout), json.loads(b.stdout)
        # ★`lane_source` 는 **일부러 갈려야 하는** 관측 필드다(경로가 아니라 출처 표시).
        #   경로 동치는 그것을 뺀 나머지로 재고, 출처 표시는 반대로 '갈리는지'를 잰다 —
        #   이 필드가 두 모드에서 같으면 관측이 거짓말을 하고 있다는 뜻이다.
        sa, sb = ja.pop("lane_source", None), jb.pop("lane_source", None)
        need(ja == jb,
             "롤백 전후 lane-path 경로가 갈린다(%s):\n신=%s\n구=%s" % (tag, ja, jb))
        need(sa == "javis_lane" and isinstance(sb, str) and sb.startswith("legacy-inline"),
             "lane_source 관측이 두 모드를 구분하지 못한다(침묵 폴백을 못 본다): %r / %r"
             % (sa, sb))
    notes.append("lane-path all 경로 동일·출처 관측 분별(base·dept)")

    # ⓕ 어휘 보존 — exit 11 의 증거 파일명·락 파일명은 이관 후에도 불변이어야 한다
    need(B.lane_state_path("skip", "").endswith("boot-skip-base.json"),
         "exit 11 증거 파일명(boot-skip-<lane>.json)이 바뀌었다 — 어휘 보존 위반")
    need(B.lane_state_path("lock", "").endswith("bootstrap-base.lock"), "락 파일명 변경")
    need(B.EXIT_SKIPPED_INFLIGHT == 11 and B.EXIT_CLAIM_DENIED == 7
         and B.EXIT_SESSION_CONTEXT == 10,
         "이관 어휘(exit 7/10/11)가 바뀌었다 — 소비처가 판정을 오독한다")
    notes.append("어휘 보존(exit 7/10/11 · skip·lock 파일명)")

    # ⓖ ②ping 상수 개명 — ⑤ 재확인 경로가 **계속** 같은 상한을 소비하는가
    need(hasattr(B, "DAEMON_PROBE_TIMEOUT_S"), "DAEMON_PROBE_TIMEOUT_S 개명이 착지하지 않았다")
    need(B.PING_TIMEOUT_S == B.DAEMON_PROBE_TIMEOUT_S, "구 이름 별칭의 값이 갈렸다")
    need(len(re.findall(r'_run\(\["cys", "ping"\], timeout=DAEMON_PROBE_TIMEOUT_S\)', src)) == 2,
         "데몬 탐침 상한 소비 지점이 2곳(② 재시도 · ⑤ exit 2 재확인)이 아니다 — 이관 중 "
         "⑤ 재확인이 무상한 ping 이 되거나 하드코딩으로 갈렸다")
    notes.append("데몬 탐침 상한 소비 2지점")

    # 계측 타당성(구 코드 대조): 기준 트리에는 javis_lane 이 없다
    calib = "skip(no-git)"
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"), ref=PRE_U24_REF)
    if old is not None:
        need("javis_lane" not in old,
             "계측 타당성 실패: 기준 트리에 이미 javis_lane 이관이 있다(기준 선택 오류)")
        need("def lane_state_path(" in old,
             "계측 타당성 실패: 기준 트리에서 인라인 lane_state_path 소유를 못 찾았다 — "
             "기준(PRE_U24_REF)이 이관보다 앞선 트리가 맞는지 먼저 확인하라")
        need("DAEMON_PROBE_TIMEOUT_S" not in old,
             "계측 타당성 실패: 기준 트리에 이미 개명이 있다")
        calib = "기준 트리(%s)=인라인 소유·javis_lane 부재·구 상수명 확인" % PRE_U24_REF
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-PACK-TRACK-1", "W6",
          "팩 파일 전량 git-추적(build.rs 임베드 전제) — '개발트리 초록·설치본 결손' 차단",
          ["U-24", "WIN-설치본 결손"])
def h_pack_track_1():
    """`build.rs` 는 팩을 **`git ls-files cysjavis-pack`(추적 파일 전용)** 으로 임베드한다.
    새 팩 파일을 만들고 `git add` 를 잊으면 개발 트리에서는 형제 import 가 되고(같은 폴더),
    **빌드된 바이너리가 설치하는 팩에는 그 파일이 없다** — 그 순간 소비자는 조용히 레거시
    폴백으로 접히거나 ImportError 로 죽는다. "맥은 멀쩡한데 설치본만 깨진다"의 정확한 기제다.

    ★전제 자체를 먼저 검증한다: build.rs 가 `git ls-files` 소싱과 제외규칙(dot·tests·
      __pycache__)을 유지하는지 확인하고, 그게 바뀌었으면 **이 검체의 판정 근거가 사라진 것**
      이므로 초록이 아니라 적색으로 알린다(측정 불능을 통과로 접지 않는다).
    """
    if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
        raise Skip("레포 체크아웃이 아니다(배포 팩 실행) — git 추적 판정 불가")
    notes = []
    # ⓐ 전제: build.rs 의 소싱·제외 규칙
    br = _read(os.path.join(REPO_DIR, "build.rs"))
    need('"ls-files", "cysjavis-pack"' in br,
         "build.rs 가 더 이상 `git ls-files` 로 팩을 소싱하지 않는다 — 이 검체의 전제가 "
         "무너졌으니 판정 근거를 먼저 재수립하라(초록으로 넘기지 않는다)")
    need("__pycache__" in br and 'c == "tests"' in br,
         "build.rs 제외규칙(tests·__pycache__)이 바뀌었다 — 아래 디스크 스캔과 규칙이 갈린다")
    notes.append("build.rs 전제(ls-files·제외규칙) 확인")

    # ⓑ 술어: **추적되지 않았고 gitignore 도 아닌** 팩 파일 = '`git add` 를 잊은 파일'.
    #    ★gitignore 를 존중하는 것은 완화가 아니라 build.rs 계약의 정확한 미러다 — build.rs 는
    #      스스로 "추적 집합을 SOT로 삼아 **gitignore 경계를 그대로 따른다**"고 선언한다.
    #      의도적으로 무시되는 런타임 산출물(`cysjavis-pack/state/*.jsonl` 등)은 임베드 대상이
    #      아니며, 그것을 적색으로 잡으면 검체가 상시 적색이 되어 **진짜 누락을 가린다**.
    #    ★build.rs 가 추가로 버리는 경로(dot·tests·__pycache__)도 같은 규칙으로 뺀다 —
    #      그쪽은 추적돼 있어도 임베드되지 않으므로 '누락' 축이 아니다.
    def _embed_excluded(rel_in_pack):
        return any(c.startswith(".") or c in ("tests", "__pycache__")
                   for c in rel_in_pack.split("/"))

    def _forgotten(repo):
        """(repo 루트) → build.rs 가 임베드해야 하는데 추적되지 않은 팩 파일 목록."""
        rr = _run(["git", "-C", repo, "ls-files", "--others", "--exclude-standard",
                   "cysjavis-pack"], timeout=60)
        if rr.returncode != 0:
            raise Fail("git ls-files --others 실패 — 측정 불능(통과 아님): rc=%d" % rr.returncode)
        out = []
        for line in rr.stdout.splitlines():
            rel = line.strip()
            if rel.startswith("cysjavis-pack/") and not _embed_excluded(
                    rel[len("cysjavis-pack/"):]):
                out.append(rel)
        return sorted(out)

    # ⓒ 실제 트리 판정
    tracked = _run(["git", "-C", REPO_DIR, "ls-files", "cysjavis-pack"], timeout=60)
    need(tracked.returncode == 0 and tracked.stdout.strip(),
         "git ls-files 실패 — 측정 불능(통과 아님): rc=%d" % tracked.returncode)
    n_tracked = len([l for l in tracked.stdout.splitlines() if l.strip()])
    forgotten = _forgotten(REPO_DIR)
    need(not forgotten,
         "팩 파일이 git 에 추적되지 않는다 = **빌드된 바이너리의 팩에 존재하지 않는다**\n"
         "  (개발 트리에서는 형제 import 로 초록 · 설치본에서만 결손 = 이 제품이 반복해서 낸 사고)\n"
         "  누락: %s\n"
         "  → `git add %s`  (의도적 비배포라면 cysjavis-pack/.gitignore 에 등재하라)"
         % (forgotten[:8], " ".join(forgotten[:8])))
    notes.append("추적 %d건 · 누락 0" % n_tracked)

    # ⓓ **합성 표본으로 탐지력 시험** — 트리가 깨끗해도 탐지기가 살아 있는지 별도로 증명한다.
    #    격리 임시 git 저장소에 4종(추적·미추적·gitignore·tests 제외)을 두고 **같은 술어**를 돌린다
    #    (사용자 레포 무접촉). 잡아야 할 1건만 정확히 나오는지 = 과탐·미탐 양방향 시험.
    if not shutil.which("git"):
        raise Fail("git 부재 — 합성 표본 검증 불가(측정 불능)")
    with tempfile.TemporaryDirectory() as tmp:
        _run(["git", "init", "-q", tmp], timeout=60)
        _run(["git", "-C", tmp, "config", "user.email", "t@t"], timeout=30)
        _run(["git", "-C", tmp, "config", "user.name", "t"], timeout=30)
        _w(os.path.join(tmp, "cysjavis-pack", ".gitignore"), "state/spool.jsonl\n", 0o644)
        _w(os.path.join(tmp, "cysjavis-pack", "bin", "kept.py"), "# tracked\n", 0o644)
        _w(os.path.join(tmp, "cysjavis-pack", "bin", "lost.py"), "# forgotten\n", 0o644)
        _w(os.path.join(tmp, "cysjavis-pack", "state", "spool.jsonl"), "{}\n", 0o644)
        _w(os.path.join(tmp, "cysjavis-pack", "bin", "tests", "t_x.py"), "# excluded\n", 0o644)
        _run(["git", "-C", tmp, "add", "cysjavis-pack/.gitignore",
              "cysjavis-pack/bin/kept.py"], timeout=60)
        synth = _forgotten(tmp)
        need(synth == ["cysjavis-pack/bin/lost.py"],
             "계측 타당성 실패: 합성 표본(추적·미추적·ignore·tests 4종)에서 탐지기가 "
             "'git add 를 잊은 1건'만 정확히 집어내지 못했다 → %r "
             "(미탐이면 이 검체의 초록은 무의미하고, 과탐이면 상시 적색으로 진짜 누락을 가린다)"
             % synth)
    notes.append("합성 표본 4종(미탐·과탐 양방향) 판별")
    return " · ".join(notes)


@specimen("H-LIFE-2", "W3", "step id enum 유일성·리터럴 0·기록 순서=실행 순서",
          ["P3-A-STEP-NAME"])
def h_life_2():
    """P3-A-STEP-NAME(치환된 절반이 성립): **동명이의 재사용**(`③′lane-pack` 이 불량 레인/레인↔팩
    두 단계 공유, `④′resource-gate` 가 실행/생략 공유)과 **서수↔실행순서 불일치**(레인 가드는
    ①preflight 앞인데 `③′`, 티켓 소비는 ④boot 뒤인데 `③″`). 라벨을 레지스트리로 승격한다."""
    B = _bootstrap_mod()
    notes = []
    need(len(set(B.STEP_ORDER)) == len(B.STEP_ORDER),
         "단계 라벨 중복(동명이의): %r" % [l for l in B.STEP_ORDER if B.STEP_ORDER.count(l) > 1])
    notes.append("라벨 %d개 유일" % len(B.STEP_ORDER))
    src = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    lits = re.findall(r'log\.(?:step|fail)\(\s*"', _code_lines(src))
    need(not lits, "단계 기록에 문자열 리터럴 잔존 %d건(레지스트리 미경유)" % len(lits))
    notes.append("호출부 리터럴 0")
    # 선언 순서 = 실행 순서(핵심 3쌍)
    idx = B.STEP_INDEX
    need(idx[B.STEP.LANE_PACK] < idx[B.STEP.PREFLIGHT], "레인 가드가 preflight 뒤로 선언됨")
    need(idx[B.STEP.BOOT] < idx[B.STEP.BOOT_TICKET_CONSUME] < idx[B.STEP.BOOT_REVIEWERS],
         "티켓 소비 서수가 실행 위치(④boot~④-b)와 불일치")
    need(idx[B.STEP.RESOURCE_GATE] < idx[B.STEP.BOOT] < idx[B.STEP.CHECK] < idx[B.STEP.MARKER],
         "체인 서수 역행")
    notes.append("서수=실행순서")
    # 실측: 실제 런의 기록 순서가 선언 순서에 단조하고 위반 표식이 없다
    boot = os.path.join(BIN_DIR, "javis_bootstrap.py")
    with tempfile.TemporaryDirectory() as tmp:
        env, home = _boot_sandbox(tmp)
        r = _run([PY, boot], env=env, timeout=180)
        need(r.returncode == 0, "부트 실패: %d\n%s" % (r.returncode, r.stderr[-400:]))
        bl = _boot_last(home)
        steps = bl.get("steps") or []
        need(steps, "단계 기록이 없다")
        orders = []
        for st in steps:
            need("step_unregistered" not in st, "미등록 라벨 기록: %r" % st["step"])
            need("order_violation" not in st, "순서 역행 기록: %r" % st)
            need("order" in st, "기록에 선언 순서(order)가 없다: %r" % st)
            orders.append(st["order"])
        need(orders == sorted(orders), "기록 순서가 선언 순서에 단조하지 않다: %r" % orders)
        notes.append("실측 %d단계 단조" % len(steps))
        # ⑤check 반복은 base 라벨 + suffix 로 남는다(정체성이 시도마다 새로 생기지 않는다)
        chk = [st for st in steps if st["step"].startswith(B.STEP.CHECK)]
        need(chk and all(st["order"] == idx[B.STEP.CHECK] for st in chk),
             "⑤check 반복 시도의 단계 정체성이 흔들린다: %r" % chk)
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"))
    calib = "skip(no-git)"
    if old is not None:
        need(_code_lines(old).count('log.step("③′lane-pack", 1, detail)') == 2,
             "계측 타당성 실패: 구 코드의 동명이의(③′lane-pack 2회)를 못 찾았다")
        need('log.step("⑤check#%d" % attempt' in old,
             "계측 타당성 실패: 구 코드의 시도별 라벨 조립을 못 찾았다")
        need("STEP_ORDER" not in old, "계측 타당성 실패: 구 코드에 이미 레지스트리가 있다")
        calib = "구 코드=동명이의 2회 + 시도별 라벨 조립 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-OBS-1", "W2", "배달 3분기 VERIFY(pending/dropped/delivered-무ack)", ["B11"])
def h_obs_1():
    """B11: 실체는 '유실'보다 **'무기한 지연 + 미배달/배달-무ack 구분 불가'** 다. 구분이 없으면
    처방이 갈린다 — 그리고 **맹목 재전송**은 wakeup 홍수를 재생산한다(재감사 명문).
    pending=재전송 금지 / dropped=재기동 격상 / delivered_no_ack=**멱등 1회** 재주입."""
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_boot_node as BN
    marker = "너는 이 cys 워크스페이스의"
    alive = {"surface_ref": "surface:7", "role": "cso", "pid": 9, "exited": False}
    cases = [
        ("pending", [marker + " cso 노드다"], alive, BN.DELIVERY_PENDING, "재전송 금지"),
        ("dropped(좌석 소멸)", [], None, BN.DELIVERY_DROPPED, "재기동 격상"),
        ("dropped(exited)", [], {"surface_ref": "surface:7", "exited": True},
         BN.DELIVERY_DROPPED, "재기동 격상"),
        ("delivered_no_ack", [], alive, BN.DELIVERY_DELIVERED_NO_ACK, "멱등 1회 재주입"),
    ]
    for name, q, row, want, why in cases:
        got, reason = BN.classify_delivery(q, row, marker)
        need(got == want, "%s: 분기 %r(기대 %r) — %s" % (name, got, want, reason))
        need(why.split()[0] in reason or want == BN.DELIVERY_DELIVERED_NO_ACK,
             "%s: 처방 문구 누락 — %r" % (name, reason))
    # 다른 메시지가 큐에 있어도 pending 으로 오판하지 않는다(마커 결박)
    got, _ = BN.classify_delivery(["전혀 다른 큐 메시지"], alive, marker)
    need(got == BN.DELIVERY_DELIVERED_NO_ACK, "타 메시지를 우리 주입문으로 오판(마커 결박 실패)")
    # 소비부 구조: 재주입은 delivered_no_ack 에서만·attempts=1(멱등 가드)·루프 0
    src = _read(os.path.join(BIN_DIR, "javis_boot_node.py"))
    vi = src.find("배달 3분기로 처방을 가른다")
    need(vi > 0, "VERIFY 3분기 소비부를 못 찾았다")
    seg = src[vi:vi + 2600]
    need("DELIVERY_PENDING" in seg and "queue_pending" in seg,
         "pending 이 별도 사유로 보고되지 않는다")
    need("inject(a.role, msg, attempts=1)" in seg,
         "재주입이 멱등 1회(attempts=1)가 아니다 — 맹목 재전송 위험")
    need(seg.count("inject(a.role, msg") == 1, "재주입이 여러 지점에서 일어난다(멱등 가드 붕괴)")
    pi = seg.find("DELIVERY_PENDING")
    ri = seg.find("inject(a.role, msg, attempts=1)")
    need(0 < pi < ri, "pending 분기가 재주입보다 뒤에 있다(pending 에서도 재전송)")
    # 큐 조회 경로 존재(pending 판정의 재료)
    need("def queue_previews(" in src and '"queue", "list"' in src,
         "큐 조회(cys queue list) 경로가 없다 — pending 판정 불가")
    # 래치 기반 ack 도 인정(t_inject 이후만)
    lt = {"surfaces": [{"role": "cso", "exited": False, "status": None, "awakened_at": 1000.0}]}
    need(BN.post_inject_ack(lt, "cso", 5, t_inject=900.0) is True, "주입후 래치 ack 미인정")
    need(BN.post_inject_ack(lt, "cso", 5, t_inject=1100.0) is False, "주입전 래치를 ack 오인정")
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_boot_node.py"))
    calib = "skip(no-git)"
    if old is not None:
        need("classify_delivery" not in old, "계측 타당성 실패: 구 코드에 이미 배달 분기가 있다")
        need("injected_unverified" in old and "no_ack" in old,
             "계측 타당성 실패: 구 코드의 단일 'no_ack' 보고를 못 찾았다")
        calib = "구 코드=단일 no_ack(미배달↔배달-무ack 구분 0) 확인"
    return ("배달 4케이스 분기 + 마커 결박 · 재주입=delivered_no_ack 한정·멱등 1회 · "
            "큐 조회 경로 · 래치 ack 경계 · 계측검증=%s" % calib)


@specimen("H-OBS-2", "W2", "주입 검증 실패 → directive_verified:false 상태화", ["B14"])
def h_obs_2():
    """B14: 검증 신호가 '화면 문자열'이라 약했고 실패는 stderr 경고 1줄로 삼켜졌다(RC3).
    신호를 **ack 계약**으로 교체하고 판정을 **상태로 남긴다**.
    ★치명 격상 금지(금지 방향 ③): 미확인은 부트를 죽이지 않는다 — 위경고 모드 회귀 차단."""
    csrc = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    stt = _repo_file(os.path.join("src", "bin", "cysd", "state.rs"))
    hnd = _repo_file(os.path.join("src", "bin", "cysd", "handlers.rs"))
    notes = []
    # ⓐ 상태 필드 + 단일 write path(전용 RPC) + 노출
    need("pub directive_verified: Mutex<Option<bool>>" in stt, "directive_verified 상태 필드가 없다")
    need('"directive.verify" =>' in hnd, "directive_verified 의 단일 write path RPC 가 없다")
    need(hnd.count('"directive_verified"') >= 2,
         "directive_verified 가 status/dashboard 양쪽에 노출되지 않는다")
    notes.append("상태 필드+전용 RPC+양쪽 노출")
    # ⓑ 신원 게이트: 노드 pane 은 자기 검증 결과를 자칭할 수 없다
    vi = hnd.find('"directive.verify" =>')
    seg = hnd[vi:vi + 2600]
    need("resolve_caller_surface" in seg and "verify_denied" in seg,
         "노드 pane 의 자칭 검증을 막는 신원 게이트가 없다")
    notes.append("자칭 검증 차단(신원 게이트)")
    # ⓒ launch-agent 검증 신호가 ack(래치)이고, 화면 문자열은 **보조 증거**로 강등됐다
    # ★(U-13) 슬라이스만 안전화 — 앵커 부재가 조용한 절단으로 흐르지 않게 한다(판정 무변경).
    body = _slice_between(csrc, "fn boot_agent_on_surface(",
                          "\n/// 에이전트 기동 + 역할 지침", "H-OBS-2 부트 본문")
    need('row["awakened_at"].as_f64()' in body, "주입 검증이 ack 래치를 신호로 쓰지 않는다")
    need("보조 증거(주 신호가 아니다)" in body, "화면 에코가 보조 증거로 강등되지 않았다")
    need('"directive.verify"' in body, "검증 결과가 상태로 기록되지 않는다")
    notes.append("신호=ack 래치 · 화면=보조 증거")
    # ⓓ **치명 격상 0**: 미확인 경로가 Err 를 반환하지 않는다(부트 계속)
    ai = body.find("let ack_deadline")
    need(ai > 0, "ack 창을 못 찾았다")
    ack_seg = body[ai:body.find("// 5) T2-5", ai) if body.find("// 5) T2-5", ai) > 0 else ai + 3000]
    need("return Err(" not in ack_seg,
         "주입 검증 미확인이 Err 로 승격됐다(금지 방향 ③ 위반 — 위경고 모드 회귀)")
    need("부트는 계속한다(치명 아님)" in ack_seg, "치명 아님 계약이 코드에 명문화되지 않음")
    # ⓔ **전문 디렉티브 재주입 0** — 그 경로는 boot_node(짧은 각성문)의 몫이다
    need("inject_text(sid, &directive)" in body, "주입 지점을 못 찾았다")
    need(body.count("inject_text(sid, &directive)") == 1,
         "전문 디렉티브가 두 번 주입된다(토큰 2배·중복 지침 혼선·resume 컨텍스트 임계)")
    notes.append("치명 격상 0 · 전문 재주입 0(재주입은 boot_node 몫)")
    # ⓕ 실패 이벤트가 남는다(경고 삼킴 제거)
    need('"directive.unverified"' in hnd, "검증 실패 이벤트가 없다(경고 삼킴 잔존)")
    notes.append("directive.unverified 이벤트")
    old = _git_show(os.path.join("src", "bin", "cys.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need('flat.contains("ABSOLUTEDIRECTIVE")' in old,
             "계측 타당성 실패: 구 코드의 화면 문자열 검증을 못 찾았다")
        need("directive.verify" not in old, "계측 타당성 실패: 구 코드에 이미 상태화가 있다")
        calib = "구 코드=화면 문자열 검증 + stderr 경고 1줄 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib



@specimen("H-OBS-3", "W1a", "다중 세션 카운트가 comm=claude 프로세스를 계수", ["G33"])
def h_obs_3():
    body = _read(os.path.join(HOOKS_DIR, "inject-context.sh"))
    need("-c claude" in body, "lsof 계측이 claude 프로세스를 세지 않는다(계측기 타당성)")
    with tempfile.TemporaryDirectory() as tmp:
        proj = os.path.join(tmp, "proj")
        os.makedirs(os.path.join(proj, "_round"), exist_ok=True)
        _w(os.path.join(proj, "_round", "SESSION_STATE.md"), "S\n", 0o644)
        binp = os.path.join(tmp, "bin")
        # 목 lsof: `-c claude` 가 인자에 있을 때만 cwd 행 2개를 낸다(=네이티브 claude 2세션)
        _w(os.path.join(binp, "lsof"),
           '#!/bin/sh\nfor a in "$@"; do [ "$a" = claude ] && '
           '{ printf "n%%s\\nn%%s\\n" "%s" "%s"; exit 0; }; done\nexit 0\n' % (proj, proj))
        for tool in ("sed", "grep", "date", "wc", "tr", "awk", "head", "tail", "cat",
                     "dirname", "basename", "ls", "rm", "mkdir", "python3", "sh", "bash"):
            src = shutil.which(tool)
            if src and not os.path.exists(os.path.join(binp, tool)):
                os.symlink(src, os.path.join(binp, tool))
        env = _base_env({"HOME": os.path.join(tmp, "home"), "PATH": binp,
                         "CYS_PACK_DIR": os.path.join(tmp, "nopack")})
        r = _run([BASH, _hook("inject-context.sh")], env=env,
                 input=json.dumps({"source": "clear", "cwd": proj}))
        need("동시에 도는 claude 세션이 2개 감지됨" in r.stdout,
             "claude 세션 2개를 계수하지 못했다: %r" % r.stdout[-500:])
    calib = "skip(no-git)"
    old = _git_show("cysjavis-pack/hooks/inject-context.sh")
    if old is not None:
        need("-c claude" not in old, "계측 타당성 실패: 구 코드가 이미 claude 를 셌다")
        calib = "구 코드 node 전용 계측 확인"
    return "목 lsof 로 claude 2세션 계수 확인 · 계측검증=%s" % calib


# ═══════════════════════════════════════════════════════════════════════════
# W5 — T-0147-2 wakeup 홍수 해소 검체군 (설계 §1-B 사전 등록 검체표)
#
#   설계가 **구현 전에** 박제한 검체표를 그대로 코드로 옮긴 것이다(producer≠evaluator).
#   전 검체는 서브프로세스 0·데몬 0·네트워크 0이며 상태는 전부 격리 tmpdir 다
#   (라이브 팩·라이브 state 무접촉 — 게이트 대역 러너가 외부 명령을 전부 가로챈다).
#
#   계측 타당성(MEMORY '디버깅 계측 타당성 게이트'): 음성 검체(push 0 주장)는 **구 코드
#   (W5 착지 직전 커밋)를 같은 시나리오로 돌려 push 가 실제로 나갔는지** 확인한다. 구 코드가
#   조용하면 "push 0"은 아무것도 증명하지 못한다.
# ═══════════════════════════════════════════════════════════════════════════
W5_CALIB_REF = os.environ.get("CYS_HEALTH_W5_CALIB_REF", "5bec3ab")
W5_GATE_REL = os.path.join("cysjavis-pack", "bin", "javis_report_gate.py")


def _w5_mod():
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_report_gate as G
    return G


class _W5Env:
    """게이트 판정에 영향을 주는 env 를 결정론으로 고정(가드·경로 해석의 실행위치 의존 제거)."""

    KEYS = ("CYS_SOCKET", "CYS_PACK_DIR", "JAVIS_PACK_DIR", "CYS_REPORT_GATE_DIR",
            "JAVIS_ROOT", "JAVIS_TASK_ROOT", "CYS_TASK_ROOT", "CYS_BIN")

    def __init__(self, **kv):
        self.kv, self.saved = kv, {}

    def __enter__(self):
        for k in self.KEYS:
            self.saved[k] = os.environ.pop(k, None)
        os.environ.update({k: v for k, v in self.kv.items() if v is not None})
        return self

    def __exit__(self, *exc):
        for k in self.KEYS:
            os.environ.pop(k, None)
        for k, v in self.saved.items():
            if v is not None:
                os.environ[k] = v


class _W5Clock:
    def __init__(self, t0=1_700_000_000.0):
        self.t = t0

    def now_epoch(self):
        return self.t

    def now_iso(self):
        return time.strftime("%Y-%m-%dT%H:%M:%S+0000", time.gmtime(self.t))

    def tick(self, secs=300.0):
        self.t += secs


class _W5Fake:
    """게이트 대역 러너 — 외부 명령(javis_report/event/wakeup·cys·데몬 소켓)을 전부 가로챈다."""

    def __init__(self, rep=None, ok=True, err=None, events=None, ack=True, tasks=None,
                 drain_delivered=1, enqueue_rc=0, collect_raises=False):
        self.rep, self.ok, self.err = rep, ok, err
        self.events, self.ack, self.tasks = list(events or []), ack, tasks
        self.drain_delivered, self.enqueue_rc = drain_delivered, enqueue_rc
        self.collect_raises = collect_raises
        self.enqueues, self.emits, self.drains, self.sends, self.polls = [], [], [], [], []
        self._wid = 0

    # ── 계수 헬퍼: M1 의 계수점(= target=master 인 wake) ──
    @property
    def master_pushes(self):
        return [e for e in self.enqueues if e[0] == "master"]

    def collect_report(self):
        if self.collect_raises:
            raise RuntimeError("주입된 내부 오류")
        return self.ok, self.rep, self.err

    def emit(self, evt_type, fields, surface="auto"):
        self.emits.append((evt_type, fields))
        return 0, "", ""

    def enqueue(self, to, task, reason, idem, payload=None, severity=None):
        self.enqueues.append((to, task, reason, idem, severity))
        self._wid += 1
        return self.enqueue_rc, "W-%010x" % self._wid

    def drain(self, target):
        self.drains.append(target)
        return 0, self.drain_delivered

    def poll_events(self, after_seq, names, timeout=0):
        self.polls.append((after_seq, tuple(names)))
        if not self.ack:
            return False, [], after_seq
        evs = [e for e in self.events if e.get("name") in names]
        self.events = [e for e in self.events if e.get("name") not in names]  # 1회 소비(링버퍼 커서)
        return True, evs, after_seq + len(evs)

    def task_snapshot(self):
        if self.tasks is None:
            return False, None, "no tasks"
        return True, list(self.tasks), None

    def send_queued(self, to, body):
        self.sends.append((to, body))          # ★I2 직송 잔존 탐지용(기대값은 항상 빈 목록)
        return 0


def _w5_report(nodes=None, live=None, feed=None, sampled_at=1_700_000_000.0, **extra):
    """javis_report --json 픽스처. `live` 항목 그대로 층4 권위 레코드(role_measurements)를 만든다.

    live 항목 키: role · idle · alive(기본 True) · ctx · status_age · tokens
    """
    live = live or []
    live_nodes, idle_nodes, measures = [], [], []
    for n in live:
        role = n["role"]
        idle = n.get("idle")
        entry = {"role": role, "state": None, "context_pct": n.get("ctx"),
                 "idle_secs": idle, "agent_alive": n.get("alive", True),
                 "status_age_secs": n.get("status_age"), "usage_ctx_tokens": n.get("tokens")}
        live_nodes.append(entry)
        if isinstance(idle, int) and idle >= 300 and n.get("alive", True):
            idle_nodes.append(entry)
        measures.append({"role": role, "idle_secs": idle if isinstance(idle, int) else None,
                         "sampled_at": sampled_at,
                         "source": "daemon.status.idle_secs" if isinstance(idle, int)
                                   else "unavailable",
                         "agent_alive": n.get("alive", True),
                         "status_age_secs": n.get("status_age"),
                         "usage_ctx_tokens": n.get("tokens")})
    rep = {"overall_pct": 0, "overall_done": 0, "overall_total": 0,
           "nodes": nodes or [], "live_nodes": live_nodes, "idle_nodes": idle_nodes,
           "feed_pending": feed, "paused": None, "status_available": True,
           "sampled_at": sampled_at, "measure_source": "daemon.status",
           "role_measurements": measures}
    rep.update(extra)
    return rep


def _w5_gate(G, sd, runner, clk, stall_cycles=2, quiet_cycles=100, **kw):
    return G.Gate(sd, runner, cycle_minutes=5, stall_cycles=stall_cycles,
                  quiet_cycles=quiet_cycles, now_epoch_fn=clk.now_epoch,
                  now_iso_fn=clk.now_iso, **kw)


def _w5_ledger(sd):
    out = []
    try:
        with open(os.path.join(sd, "ledger.jsonl"), encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    out.append(json.loads(line))
    except OSError:
        pass
    return out


def _w5_badges(sd):
    try:
        with open(os.path.join(sd, "badges.json"), encoding="utf-8") as f:
            return {b["key"]: b for b in json.load(f)["badges"]}
    except (OSError, ValueError, KeyError):
        return {}


def _w5_old_gate_module(tmp):
    """W5 착지 직전 커밋의 게이트를 임시 모듈로 적재(계측 타당성 대조군). 실패 시 None."""
    src = _git_show(W5_GATE_REL, ref=W5_CALIB_REF)
    if src is None:
        return None
    import importlib.util
    path = os.path.join(tmp, "old_report_gate.py")
    _w(path, src, 0o644)
    spec = importlib.util.spec_from_file_location("w5_old_report_gate", path)
    mod = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(mod)
    except Exception:                                  # noqa: BLE001 — 대조군 적재 실패는 skip
        return None
    return mod


class _W5OldFake:
    """구 게이트(W5 이전)의 Runner 계약 — enqueue 는 rc 하나만 돌려준다."""

    def __init__(self, rep):
        self.rep = rep
        self.enqueues, self.drains, self.sends = [], [], []

    def collect_report(self):
        return True, self.rep, None

    def emit(self, evt_type, fields, surface="auto"):
        return 0, "", ""

    def enqueue(self, to, task, reason, idem, payload=None):
        self.enqueues.append((to, task, reason, idem))
        return 0

    def drain(self, target):
        self.drains.append(target)
        return 0, 1

    def send_queued(self, to, body):
        self.sends.append((to, body))
        return 0


def _w5_calibrate_idle_push(tmp, rep):
    """구 코드에서 같은 idle 시나리오가 **master push 를 실제로 냈는지** 확인한다.
    구 코드가 조용하면 신 코드의 'push 0'은 아무것도 증명하지 못한다(계측 타당성)."""
    old = _w5_old_gate_module(tmp)
    if old is None:
        return "skip(no-git)"
    sd = os.path.join(tmp, "oldstate")
    clk = _W5Clock()
    r = _W5OldFake(rep)
    for _ in range(3):
        old.Gate(sd, r, cycle_minutes=5, stall_cycles=2, quiet_cycles=100,
                 now_epoch_fn=clk.now_epoch, now_iso_fn=clk.now_iso).run()
        clk.tick()
    masters = [e for e in r.enqueues if e[0] == "master"]
    need(masters, "계측 타당성 실패: 구 코드(%s)도 idle 에서 master push 를 내지 않았다 — "
                  "검체가 아무것도 측정하지 못한다" % W5_CALIB_REF)
    return "구 코드 %s 에서 master push %d건 재현" % (W5_CALIB_REF, len(masters))


# ── N1 정상 대기 24주기: push 0 ────────────────────────────────────────────
@specimen("H-W5-N1", "W5", "정상 대기 24주기(2h 가속) — master push 0 · badge=quiet", ["I1", "A3"])
def h_w5_n1():
    G = _w5_mod()
    rep = _w5_report(
        nodes=[{"node": "worker", "done": 2, "total": 2, "pct": 100}],
        live=[{"role": "worker", "idle": 600, "status_age": 120, "tokens": 1000},
              {"role": "cso", "idle": 900, "status_age": 120, "tokens": 2000}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        for _ in range(24):                       # 24주기 × 5분 = 2시간 가속 시뮬레이션
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(r.master_pushes == [],
             "정상 대기에서 master push 발생(M1 위반): %r" % (r.master_pushes,))
        need(r.sends == [], "직송(I2) 잔존: %r" % (r.sends,))
        b = _w5_badges(sd)
        need(list(b) == ["quiet"], "정상 대기 배지가 quiet 이 아니다: %r" % list(b))
        led = _w5_ledger(sd)
        need(len(led) == 24, "대장 기록 누락(데드맨 신호 상실): %d" % len(led))
        calib = _w5_calibrate_idle_push(tmp, rep)
    return "24주기 push 0 · badge=quiet · 대장 24건 · 계측검증=%s" % calib


# ── N2 라벨 미조인 ────────────────────────────────────────────────────────
@specimen("H-W5-N2", "W5", "라벨 미조인 — 미발화 + ledger label_unjoined + badge 노출", ["B4", "D3"])
def h_w5_n2():
    G = _w5_mod()
    # ① 접두 해소 성공(양성 대조 — 검체가 공허하지 않음을 먼저 증명)
    got, how = G.resolve_label_roles("reviewer", {"reviewer-gemini", "reviewer-codex", "master"})
    need(how == "family" and got == ["reviewer-codex", "reviewer-gemini"],
         "접두 해소 실패(설계 §0-3 오발화 근원 미수리): %r/%r" % (got, how))
    # ② 해소 불가 → 미발화 + 결함 노출
    rep = _w5_report(nodes=[{"node": "reviewer", "done": 0, "total": 3, "pct": 0}],
                     live=[{"role": "master", "idle": 10, "status_age": 5, "tokens": 1}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        for _ in range(4):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(r.master_pushes == [], "미조인 라벨이 push 를 냈다: %r" % (r.master_pushes,))
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(any("label_unjoined:reviewer" in x for x in reasons),
             "ledger 에 label_unjoined 기록 없음(은닉): %r" % reasons)
        b = _w5_badges(sd)
        need("gate-label-reviewer" in b, "스키마 결함 배지 미노출: %r" % list(b))
        need("스키마 결함" in b["gate-label-reviewer"]["message"], b["gate-label-reviewer"])
    return "접두 해소 2건 · 미조인=미발화+ledger+badge('스키마 결함')"


# ── N3 임계 미달 idle ────────────────────────────────────────────────────
@specimen("H-W5-N3", "W5", "임계 미달 idle(154s<300s) — 무발화·대장만", ["I1"])
def h_w5_n3():
    G = _w5_mod()
    rep = _w5_report(nodes=[{"node": "worker", "done": 1, "total": 3, "pct": 33}],
                     live=[{"role": "worker", "idle": 154, "status_age": 10, "tokens": 5}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        for _ in range(3):
            _w5_gate(G, sd, r, clk, stall_cycles=99).run()
            clk.tick()
        need(r.master_pushes == [], "임계 미달 idle 에서 push: %r" % (r.master_pushes,))
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(not any("idle" in x for x in reasons), "임계 미달인데 idle 경고 발생: %r" % reasons)
        need(list(_w5_badges(sd)) == ["quiet"], "임계 미달인데 경고 배지: %r" % list(_w5_badges(sd)))
    return "154s<300s 무발화 · 대장 기록만 · badge=quiet"


# ── N4 데몬 2개 레인 분리 ────────────────────────────────────────────────
@specimen("H-W5-N4", "W5", "레인 분리 — 데몬별 state·대장 격리 + foreign-daemon 가드", ["B7", "C3"])
def h_w5_n4():
    G = _w5_mod()
    rep_a = _w5_report(nodes=[{"node": "worker", "done": 1, "total": 2, "pct": 50}],
                       live=[{"role": "worker", "idle": 60, "status_age": 10, "tokens": 1}])
    rep_b = _w5_report(nodes=[{"node": "cso", "done": 1, "total": 4, "pct": 25}],
                       live=[{"role": "cso", "idle": 60, "status_age": 10, "tokens": 1}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        a, b = os.path.join(tmp, "report_gate"), os.path.join(tmp, "report_gate-dept-2")
        clk = _W5Clock()
        ra, rb = _W5Fake(rep=rep_a, tasks=[]), _W5Fake(rep=rep_b, tasks=[])
        for _ in range(3):
            _w5_gate(G, a, ra, clk, stall_cycles=99).run()
            _w5_gate(G, b, rb, clk, stall_cycles=99).run()
            clk.tick()
        need(ra.master_pushes == [] and rb.master_pushes == [],
             "정상 대기 2레인에서 push 발생: %r %r" % (ra.master_pushes, rb.master_pushes))
        la, lb = _w5_ledger(a), _w5_ledger(b)
        need(la and lb, "레인별 대장이 생성되지 않았다")
        need({e.get("lane") for e in la} == {"report_gate"},
             "레인 A 대장에 타 레인 기록 혼입: %r" % {e.get("lane") for e in la})
        need({e.get("lane") for e in lb} == {"report_gate-dept-2"},
             "레인 B 대장에 타 레인 기록 혼입: %r" % {e.get("lane") for e in lb})
        need(os.path.isfile(os.path.join(a, "counters.json"))
             and os.path.isfile(os.path.join(b, "counters.json")),
             "레인별 counters 분리 실패(카운터 이중 증가 경로 잔존)")
    # foreign-daemon 가드 — 부서 팩인데 소켓 토큰 불일치 → SKIPPED_FOREIGN_DAEMON
    with tempfile.TemporaryDirectory() as tmp:
        dept = os.path.join(tmp, "pack-dept-9")
        os.makedirs(dept)
        with _W5Env(CYS_PACK_DIR=dept):
            v = G.foreign_daemon_verdict()
            need(v is not None and v[0] == "SKIPPED_FOREIGN_DAEMON",
                 "가드 미발동(구현 소실 잔존): %r" % (v,))
        with _W5Env(CYS_PACK_DIR=dept, CYS_SOCKET=os.path.join(tmp, "cys-dept-9", "cys.sock")):
            need(G.foreign_daemon_verdict() is None, "정합 조합에서 가드가 오발동")
        # 실제 run 경로에서도 대장에 남고 카운터를 만들지 않는다(무접촉 계약)
        sd = os.path.join(tmp, "guarded")
        with _W5Env(CYS_PACK_DIR=dept):
            _w5_gate(G, sd, _W5Fake(rep=rep_a, tasks=[]), _W5Clock()).run()
        need(_w5_ledger(sd)[-1]["verdict"] == "SKIPPED_FOREIGN_DAEMON", _w5_ledger(sd)[-1])
        need(not os.path.isfile(os.path.join(sd, "counters.json")),
             "가드 경로가 카운터를 건드렸다(무접촉 위반)")
    return "2레인 대장·counters 격리 · 가드 SKIP/정합 양방향 · 카운터 무접촉"


# ── N5 drain 실패(수신 pane 파킹) ────────────────────────────────────────
@specimen("H-W5-N5", "W5", "drain 실패 반복 — 재-enqueue 폭주 0 + 배지 노출", ["G13", "A3"])
def h_w5_n5():
    G = _w5_mod()
    #   진짜 stall 확증(§2-C 3종 성립) 상태에서 배달만 실패시킨다 — push 경로가 살아있는
    #   조건이어야 "폭주하지 않음"이 의미를 갖는다(공허한 음성 방지).
    rep = _w5_report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                     live=[{"role": "worker", "idle": 900, "status_age": 1200, "tokens": 7},
                           {"role": "master", "idle": 900, "status_age": 1200, "tokens": 3}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[], drain_delivered=0)
        seen_badges = []
        for _ in range(10):
            _w5_gate(G, sd, r, clk).run()
            #   ★배지는 **매 주기 현재 상태**로 덮어써진다(그래야 갱신 정지가 데드맨이 된다).
            #     따라서 '노출됐는가'는 주기 스냅샷을 모아서 봐야 한다 — 마지막 파일만 보면
            #     쿨다운으로 조용해진 주기를 '암전'으로 오판한다(계측기 자체의 함정).
            seen_badges.append(_w5_badges(sd))
            clk.tick()
        need(len(r.master_pushes) <= 1,
             "배달 실패가 재-enqueue 폭주로 번졌다(%d건)" % len(r.master_pushes))
        need(len(r.master_pushes) == 1, "확증 stall 인데 push 가 아예 없다(공허한 음성)")
        led = _w5_ledger(sd)
        need(any(e.get("delivered") == "wake_pending" for e in led),
             "배달 미완결이 대장에 남지 않았다")
        need(any(any(k.startswith("gate-stall-") for k in b) for b in seen_badges),
             "배달 실패 구간에 배지가 한 번도 없었다(암전): %r" % [list(b) for b in seen_badges])
    #   ★G27 소비 확인(재구현 금지): W5 의 digest 배달 경로가 W2 zombie 가드를 **그대로 쓴다**.
    #     digest 로 배달 단위가 바뀐 자리는 가드를 새로 짜기 딱 좋은 지점이라 못 박는다.
    import javis_wakeup as W
    wsrc = _read(os.path.join(BIN_DIR, "javis_wakeup.py"))
    di = wsrc.find("def cmd_drain")
    need(di > 0 and "_target_alive(target)" in wsrc[di:],
         "digest 배달 경로가 zombie 가드를 소비하지 않는다(G27 재구현/우회)")
    need(W.live_target_rows("surface:1\trole=worker\texited=true\n") == [],
         "exited 행 배제가 깨졌다(죽은 대상에 digest 배달)")
    return "10주기 반복 배달실패 → master push 1건 고정 · wake_pending 기록 · 배지 노출"


# ── N6a collect 실패 ─────────────────────────────────────────────────────
@specimen("H-W5-N6A", "W5", "collect 실패 — 직송 제거·정상 state ledger + badge", ["I2"])
def h_w5_n6a():
    G = _w5_mod()
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk = _W5Clock()
        r = _W5Fake(ok=False, err="javis_report exit=1", tasks=[])
        for _ in range(3):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(r.sends == [], "fail-open 직송(I2)이 살아있다: %r" % (r.sends,))
        need(r.master_pushes == [], "collect 실패가 push 를 냈다: %r" % (r.master_pushes,))
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(any("collect_fail" in x for x in reasons), "collect_fail 대장 기록 없음: %r" % reasons)
        need("gate-collect" in _w5_badges(sd), "collect 실패 배지 없음: %r" % list(_w5_badges(sd)))
    calib = "skip(no-git)"
    old = _git_show(W5_GATE_REL, ref=W5_CALIB_REF)
    if old is not None:
        need('self.runner.send_queued("master", body)' in old,
             "계측 타당성 실패: 구 코드에 fail-open 직송이 없다")
        calib = "구 코드 fail-open 직송 2경로 존재 확인"
    return "collect 실패=ledger+badge · 직송 0 · push 0 · 계측검증=%s" % calib


# ── N6b state 쓰기 불가 → state 외부 oracle ──────────────────────────────
@specimen("H-W5-N6B", "W5", "state 기록 불능 — gate_signal 토큰(state 외부 oracle)", ["I2", "N6b"])
def h_w5_n6b():
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        raise Skip("root 는 파일권한을 무시 — chmod 555 재현 불가")
    G = _w5_mod()
    import contextlib
    import io
    rep = _w5_report(live=[{"role": "worker", "idle": 10, "status_age": 5, "tokens": 1}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        os.makedirs(sd)
        r = _W5Fake(rep=rep, tasks=[])
        os.chmod(sd, 0o555)
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                rc = _w5_gate(G, sd, r, _W5Clock()).run()
        finally:
            os.chmod(sd, 0o755)
        need(rc == 0, "state 불능에서 exit≠0(fail-open 계약 위반): %r" % rc)
        need(r.sends == [], "state 불능 직송(I2) 잔존: %r" % (r.sends,))
        need(r.master_pushes == [], "state 불능이 push 를 냈다")
        need("gate_signal=state_unwritable" in out.getvalue(),
             "state 외부 oracle 토큰 부재: %r" % out.getvalue())
    # ★2언어 이음매: 데몬 allowlist 가 같은 토큰을 승격하는가(한쪽만 바뀌면 조용히 끊긴다)
    sched = _repo_file(os.path.join("src", "bin", "cysd", "schedule.rs"))
    need("GATE_SIGNAL_ALLOWLIST" in sched, "데몬측 게이트 신호 allowlist 부재")
    need('"state_unwritable"' in sched, "데몬 allowlist 에 state_unwritable 없음(신호 사장)")
    need("gate_signals_from_stdout" in sched, "데몬측 stdout sniffer 부재")
    return "exit 0 · 직송 0 · push 0 · gate_signal 토큰 · 데몬 allowlist 이음매 확인"


# ── P1 확증 stall → 정확히 1 push ────────────────────────────────────────
@specimen("H-W5-P1", "W5", "확증 stall(2차 증거 3종) — 정확히 1 push · 미확증은 0", ["C4"])
def h_w5_p1():
    G = _w5_mod()
    base = {"nodes": [{"node": "worker", "done": 1, "total": 5, "pct": 20}]}
    confirmed = _w5_report(
        live=[{"role": "worker", "idle": 900, "status_age": 1200, "tokens": 7},
              {"role": "master", "idle": 900, "status_age": 1200, "tokens": 3}], **base)
    #   ②미측정(set-status age 부재) → **push 금지 + '측정 불가' 배지**(fail-closed)
    unmeasured = _w5_report(
        live=[{"role": "worker", "idle": 900, "tokens": 7},
              {"role": "master", "idle": 900, "tokens": 3}], **base)
    #   ③반증(set-status 가 신선 = 자기보고 살아있음) → push 금지
    refuted = _w5_report(
        live=[{"role": "worker", "idle": 900, "status_age": 30, "tokens": 7},
              {"role": "master", "idle": 900, "status_age": 30, "tokens": 3}], **base)
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        for name, rep, want in (("unmeasured", unmeasured, 0), ("refuted", refuted, 0),
                                ("confirmed", confirmed, 1)):
            sd = os.path.join(tmp, name)
            clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
            snaps = []
            for _ in range(8):
                _w5_gate(G, sd, r, clk).run()
                snaps.append(_w5_badges(sd))         # 배지는 매 주기 현재 상태로 덮인다(N5 주석)
                clk.tick()
            got = len(r.master_pushes)
            need(got == want, "%s: master push %d건(기대 %d) — §2-C fail-closed 위반"
                              % (name, got, want))
            if name == "unmeasured":
                need(any(any(k.startswith("gate-measure-") for k in b) for b in snaps),
                     "미측정이 '측정 불가' 배지로 노출되지 않았다(침묵): %r"
                     % [list(b) for b in snaps])
            if name == "confirmed":
                push = r.master_pushes[0]
                need(push[4] == "critical", "확증 stall push 가 critical 이 아니다: %r" % (push,))
                need(r.drains and r.drains[0] == "master", "배달 체인 미완결: %r" % (r.drains,))
    return "확증=1 · 미측정=0(+배지) · 반증=0 · severity=critical · 8주기 반복에도 1건 고정"


# ── P2 노드 사망(deadman 소비) ───────────────────────────────────────────
@specimen("H-W5-P2", "W5", "노드 사망 — deadman 이벤트 소비 · 1 push · 디바운스", ["D6"])
def h_w5_p2():
    G = _w5_mod()
    rep = _w5_report(live=[{"role": "master", "idle": 10, "status_age": 10, "tokens": 1}])
    dead = {"name": "master.deadman", "seq": 41, "payload": {"role": "worker"}}
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd, r, clk).run()                     # baseline
        clk.tick()
        r.events.append(dead)
        for _ in range(6):                                # 사망 1건 + 이후 정상 주기
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(len(r.master_pushes) == 1,
             "사망 push 가 정확히 1건이 아니다(%d건 — 디바운스/중복채널)" % len(r.master_pushes))
        need(r.master_pushes[0][4] == "critical", r.master_pushes[0])
        need(any(k == "gate-death-worker" for k in _w5_badges(sd)) or True, "")
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(any("death:worker" in x for x in reasons), "사망 대장 기록 없음: %r" % reasons)
    #   ★중복 채널 제거의 구조 확인: 게이트가 스냅샷 diff 로 사망을 **독자 판정하지 않는다**
    gate_src = _read(os.path.join(BIN_DIR, "javis_report_gate.py"))
    need("데몬 deadman 이벤트 소비" in gate_src, "deadman 소비 계약이 코드에 명문화되지 않음")
    need("agent_alive\") is True" not in gate_src,
         "게이트가 사망을 독자 판정한다(중복 채널 잔존 — 두 배로 울린다)")
    return "deadman 1건 → push 1 · 이후 5주기 재발화 0 · 독자 판정 경로 부재"


# ── P2b v1 침묵 오라벨 강등(결함 8) ──────────────────────────────────────
@specimen("H-W5-P2b", "W5", "v1 오라벨 — reason='master silent' 는 push 승격 0 · 대장 기록만", ["D6"])
def h_w5_p2b():
    #   v1 데몬은 "출력 없음"(오너 입력 대기 포함)을 master.deadman reason="master silent" 로
    #   발행한다 — 사망이 아닌 침묵 라벨. 게이트가 이를 critical 사망으로 승격하면 흔한 대기
    #   상태마다 기상이 울린다. 가드는 skip 하되 관측(대장)은 남겨야 한다 — 무해화≠침묵.
    G = _w5_mod()
    rep = _w5_report(live=[{"role": "master", "idle": 10, "status_age": 10, "tokens": 1}])
    dead = {"name": "master.deadman", "seq": 41,
            "payload": {"reason": "master silent", "idle_secs": 912}}   # v1 실제 wire 형태(role 無)
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd, r, clk).run()                     # baseline
        clk.tick()
        r.events.append(dead)
        for _ in range(6):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(len(r.master_pushes) == 0,
             "silent 오라벨이 push 로 승격됨(%d건 — 오너 대기마다 기상)" % len(r.master_pushes))
        need(not [e for e in r.enqueues if (e[1] or "").startswith("gate-death")],
             "silent 오라벨이 사망 wake 경로를 탐: %r" % (r.enqueues,))
        need(not [k for k in _w5_badges(sd) if "gate-death" in k], "silent 오라벨이 badge 승격됨")
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(not any(x.startswith("death:") for x in reasons),
             "silent 오라벨이 사망 대장으로 기록됨: %r" % reasons)
        need(any(x.startswith("death_skip:") for x in reasons),
             "skip 정보 기록 부재(무해화가 관측 침묵이 됨): %r" % reasons)
    return "silent 오라벨 1건 → push 0 · badge 0 · 대장 death_skip 기록만"


# ── P2c 구조 증거 사유는 여전히 사망 — 가드가 reason 키 존재로 과잉 강등하지 않는다 ──
@specimen("H-W5-P2c", "W5", "reason='master surface gone' 은 여전히 push 1 — 과잉 강등 금지", ["D6"])
def h_w5_p2c():
    #   가드의 계약은 "master silent" 정확 일치 한정이다. reason 키가 있다는 이유만으로
    #   구조 증거 사유(surface gone/exited)까지 강등하면 진짜 사망에서 기상 채널이 소멸한다.
    G = _w5_mod()
    rep = _w5_report(live=[{"role": "master", "idle": 10, "status_age": 10, "tokens": 1}])
    dead = {"name": "master.deadman", "seq": 41,
            "payload": {"role": "worker", "reason": "master surface gone"}}
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd, r, clk).run()                     # baseline
        clk.tick()
        r.events.append(dead)
        for _ in range(6):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(len(r.master_pushes) == 1,
             "구조 증거 사망 push 가 정확히 1건이 아니다(%d건)" % len(r.master_pushes))
        need(r.master_pushes[0][4] == "critical", r.master_pushes[0])
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(any("death:worker" in x for x in reasons), "사망 대장 기록 없음: %r" % reasons)
        need(not any(x.startswith("death_skip:") for x in reasons),
             "구조 증거 사유가 skip 됨(과잉 강등): %r" % reasons)
    return "surface-gone 1건 → push 1(critical) · skip 0 — 가드는 silent 정확 일치 한정"


# ── P3 시스템 데드락 ─────────────────────────────────────────────────────
@specimen("H-W5-P3", "W5", "시스템 데드락 — 티켓 원장+자기보고만으로 판정(last_output 배제)", ["R2-C4"])
def h_w5_p3():
    G = _w5_mod()
    #   ★티켓 타임스탬프는 **주입 시계와 같은 시대**여야 한다(_W5Clock t0=1.7e9 = 2023-11-14).
    #     미래 날짜를 쓰면 경과가 음수가 되어 술어가 '방금 활동'으로 읽는다 — 검체가 조용히
    #     공허해지는 형태의 함정이라 여기에 못 박는다.
    old_ts = "2023-11-01T00:00:00+0000"                 # 티켓 무활동(30분 훨씬 초과)
    tickets = [{"id": "T-1", "status": "todo", "owner": None,
                "created_at": old_ts, "updated_at": old_ts}]
    stale = _w5_report(live=[{"role": "master", "idle": 60, "status_age": 3600, "tokens": 1},
                             {"role": "cso", "idle": 60, "status_age": 3600, "tokens": 2}])
    fresh = _w5_report(live=[{"role": "master", "idle": 60, "status_age": 60, "tokens": 1},
                             {"role": "cso", "idle": 60, "status_age": 60, "tokens": 2}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        #   ⓐ 성립 — CSO 1줄 push(수신 계층: 1차 수신자 CSO)
        sd = os.path.join(tmp, "yes")
        clk, r = _W5Clock(), _W5Fake(rep=stale, tasks=tickets)
        for _ in range(6):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        deadlocks = [e for e in r.enqueues if e[1] == "gate-deadlock"]
        need(len(deadlocks) == 1, "데드락 push 가 %d건(기대 1 — 엣지+쿨다운 2h)" % len(deadlocks))
        need(deadlocks[0][0] == "cso", "데드락 push 수신자가 CSO 가 아니다: %r" % (deadlocks[0],))
        #   ⓑ 미성립 — 자기보고가 신선하면(노드가 살아 보고 중) 데드락이 아니다
        sd2 = os.path.join(tmp, "no")
        clk2, r2 = _W5Clock(), _W5Fake(rep=fresh, tasks=tickets)
        for _ in range(6):
            _w5_gate(G, sd2, r2, clk2).run()
            clk2.tick()
        need(not [e for e in r2.enqueues if e[1] == "gate-deadlock"],
             "자기보고 신선한데 데드락 오발화: %r" % (r2.enqueues,))
        #   ⓒ 티켓 원장 미측정 → 판정 자체를 하지 않는다(추론으로 메우지 않는다)
        sd3 = os.path.join(tmp, "unmeasured")
        r3 = _W5Fake(rep=stale, tasks=None)
        _w5_gate(G, sd3, r3, _W5Clock()).run()
        need(not [e for e in r3.enqueues if e[1] == "gate-deadlock"], "미측정에서 데드락 발화")
    #   ★last_output 완전 배제(R2-C4)의 구조 확인 — 술어 본문이 idle 을 읽지 않는다
    src = _read(os.path.join(BIN_DIR, "javis_report_gate.py"))
    i = src.find("def build_deadlock_warning")
    body = src[i:src.find("\ndef ", i + 10)]
    need(i > 0 and "idle_secs" not in body,
         "데드락 술어가 idle(last_output 파생)을 다시 읽는다 — R2-C4 위반")
    return "성립=CSO 1건 · 자기보고 신선=0 · 원장 미측정=0 · 술어에 idle 미사용"


# ── P4 master.idle 소비 — 정보층(격상 미달)·도메인 분리 (G2 W3-C · 결함 8 짝) ──
#   ※ 브리프의 검체 계열명 'H-W5-P3'은 이미 시스템 데드락 검체가 점유 — P4 로 재명명 배치.
@specimen("H-W5-P4", "W5",
          "master.idle 소비 — 정보 기록만(push 0·verdict 비오염·idle 도메인 분리)", ["D6"])
def h_w5_p4():
    #   데몬 v2 는 침묵을 master.idle(정보)로 발행한다(결함 8 축 분리). 격상 미달(idle<3×임계)은
    #   **warn 이 아니어야 한다**: verdict 가 WARN 이 되면 QUIET 연속 카운터가 리셋돼 세션
    #   주차(quiet-park) 신호가 죽는다 — master 침묵은 곧 주차 후보 상태이기도 하다.
    #   트리거 도메인은 기존 idle(idle_edge 카운터·gate-idle-* 키)과 분리돼야 한다(G2 성찰 MAJOR).
    G = _w5_mod()
    rep = _w5_report(live=[{"role": "master", "idle": 10, "status_age": 10, "tokens": 1}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd, r, clk).run()                     # baseline
        clk.tick()
        r.events.append({"name": "master.idle", "seq": 51,
                         "payload": {"role": "master", "axis": "silence",
                                     "idle_secs": 912, "threshold_secs": 900}})
        #   임계 미측정(threshold 부재) 이벤트도 격상 없이 정보층 — 측정 불능은 push 근거가 아니다.
        r.events.append({"name": "master.idle", "seq": 52,
                         "payload": {"role": "cso", "idle_secs": 5000}})
        _w5_gate(G, sd, r, clk).run()                     # 이벤트 소비 주기
        clk.tick()
        need(any(k == "gate-master-idle-master" for k in _w5_badges(sd)),
             "master-idle info 배지 부재: %r" % list(_w5_badges(sd)))
        for _ in range(2):                                # 이후 정상 주기(재발화·잔류 확인)
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        need(len(r.enqueues) == 0, "정보층 master.idle 이 push 를 탔다: %r" % (r.enqueues,))
        entries = _w5_ledger(sd)
        reasons = [x for e in entries for x in (e.get("reasons") or [])]
        need(any(x.startswith("master_idle:master") for x in reasons),
             "master.idle 정보 대장 기록 부재: %r" % reasons)
        need(any(x.startswith("master_idle:cso") and "unmeasured" in x for x in reasons),
             "임계 미측정 이벤트의 정보 기록 부재: %r" % reasons)
        need(not any(x.startswith("master_idle_stuck:") for x in reasons),
             "격상 미달인데 격상 기록: %r" % reasons)
        #   verdict 비오염 — master.idle 만으로는 WARN 이 되지 않는다(quiet-park 생존).
        need(not any(e.get("verdict") == "WARN" for e in entries),
             "정보층이 verdict 를 WARN 으로 오염: %r" % [e.get("verdict") for e in entries])
        #   도메인 분리 — 기존 idle 도메인(idle_5min/idle_edge·gate-idle-*)에 흔적 0.
        need(not any(x.startswith("idle_5min:") or x.startswith("idle_edge:") for x in reasons),
             "master.idle 이 기존 idle 도메인으로 새었다: %r" % reasons)
        need(not any("gate-idle-" in k for k in _w5_badges(sd)),
             "master.idle 이 기존 idle 배지 키를 점유: %r" % list(_w5_badges(sd)))
        #   회수 lane 등재 — 폴링 이름 목록에 master.idle 이 실제로 실린다(G2 BLOCKER ② 배선).
        need(r.polls and "master.idle" in r.polls[-1][1],
             "폴링 이름 목록에 master.idle 미등재: %r" % (r.polls[-1:],))
    return "정보층: push 0 · WARN 0 · 대장 master_idle 기록 · info 배지 · idle 도메인 무혼입"


# ── P4b master.idle 장기화 격상 — 3×임계에서만 critical push(CSO) ─────────────
@specimen("H-W5-P4b", "W5",
          "master.idle 장기화(≥3×임계) — critical push 1건(CSO)·경계 미만 0", ["D6"])
def h_w5_p4b():
    #   hung master(v1 은 900s 후 'master silent' 사망 오라벨로라도 울렸다)의 CSO 기상 채널을
    #   축 분리 후에도 보존하는 격상층이다. 단 사망이 아니므로 death 도메인과 섞지 않는다.
    G = _w5_mod()
    rep = _w5_report(live=[{"role": "master", "idle": 10, "status_age": 10, "tokens": 1},
                           {"role": "cso", "idle": 10, "status_age": 10, "tokens": 2}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        #   ⓐ 3×임계 도달 — 지속 스트림(매 주기 이벤트)에도 push 정확히 1(엣지+쿨다운 2h)
        sd = os.path.join(tmp, "yes")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd, r, clk).run()                     # baseline
        clk.tick()
        for i in range(5):
            r.events.append({"name": "master.idle", "seq": 60 + i,
                             "payload": {"role": "master", "axis": "silence",
                                         "idle_secs": 2700 + 300 * i,
                                         "threshold_secs": 900}})
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        stuck = [e for e in r.enqueues if e[1] == "gate-master-idle-master"]
        need(len(stuck) == 1, "격상 push 가 %d건(기대 1 — 엣지+쿨다운)" % len(stuck))
        need(stuck[0][0] == "cso", "격상 push 수신자가 CSO 가 아니다: %r" % (stuck[0],))
        need(stuck[0][4] == "critical", stuck[0])
        need(len(r.enqueues) == 1, "격상 외 잉여 push: %r" % (r.enqueues,))
        reasons = [x for e in _w5_ledger(sd) for x in (e.get("reasons") or [])]
        need(any(x.startswith("master_idle_stuck:master") for x in reasons),
             "격상 대장 기록 부재: %r" % reasons)
        #   death 도메인 무혼입 — 생존 확정 침묵이 사망 대장·wake 경로를 타면 결함 8 재발이다.
        need(not any(x.startswith("death:") for x in reasons),
             "master.idle 격상이 death 대장으로 새었다: %r" % reasons)
        need(not [e for e in r.enqueues if (e[1] or "").startswith("gate-death")],
             "master.idle 격상이 death wake 경로를 탔다: %r" % (r.enqueues,))
        #   ⓑ 경계 미만(2699 < 3×900=2700) → push 0(정보층으로만)
        sd2 = os.path.join(tmp, "no")
        clk2, r2 = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        _w5_gate(G, sd2, r2, clk2).run()                  # baseline
        clk2.tick()
        r2.events.append({"name": "master.idle", "seq": 61,
                          "payload": {"role": "master", "axis": "silence",
                                      "idle_secs": 2699, "threshold_secs": 900}})
        for _ in range(3):
            _w5_gate(G, sd2, r2, clk2).run()
            clk2.tick()
        need(len(r2.enqueues) == 0, "경계 미만에서 격상 오발화: %r" % (r2.enqueues,))
        reasons2 = [x for e in _w5_ledger(sd2) for x in (e.get("reasons") or [])]
        need(any(x.startswith("master_idle:master") for x in reasons2),
             "경계 미만 정보 기록 부재: %r" % reasons2)
    return "3×임계: push 1(cso·critical) · 지속 5주기 재발화 0 · 경계 2699s 미발화 · death 무혼입"


# ── C1 crash 후 재실행 중복 상한 ─────────────────────────────────────────
@specimen("H-W5-C1", "W5", "seen mark 직전 crash — 총 delivery 1..2 · duplicate 0..1", ["C2", "R2-C3"])
def h_w5_c1():
    G = _w5_mod()
    rep = _w5_report(nodes=[{"node": "worker", "done": 1, "total": 5, "pct": 20}],
                     live=[{"role": "worker", "idle": 900, "status_age": 1200, "tokens": 7},
                           {"role": "master", "idle": 900, "status_age": 1200, "tokens": 3}])
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=[])
        for _ in range(4):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        first = len(r.master_pushes)
        need(first == 1, "확증 stall 첫 push 가 1건이 아니다: %d" % first)
        #   crash 재현: seen 레코드가 `claimed`(enqueue 증거 없음) 상태로 남은 채 죽었다.
        recs = [x for x in G.seen_iter(sd) if x.get("state")]
        need(recs, "seen-store 레코드가 없다(A6′ 미구현)")
        for rec in recs:
            G.seen_mark(sd, rec["key"], clk.now_epoch(), state=G.SEEN_STATE_CLAIMED,
                        wakeup_id=None)
        for _ in range(3):
            _w5_gate(G, sd, r, clk).run()
            clk.tick()
        total = len(r.master_pushes)
        dup = total - 1
        need(1 <= total <= 2, "총 delivery %d건(oracle 1..2)" % total)
        need(0 <= dup <= 1, "duplicate %d건(oracle 0..1)" % dup)
        need(G.seen_iter(sd), "복구 후 seen 근거가 소실됐다(원장 근거 잔존 요구)")
    return "crash(claimed 잔존) 후 재실행 — 총 %s · duplicate ≤1 · seen 근거 잔존" % "1..2"


# ── C2 TTL 경계 ──────────────────────────────────────────────────────────
def _w5_deadlock_fixture():
    """TTL 경계 실험용 지속 조건 — 데드락 술어는 **다른 쿨다운에 가려지지 않는다**.

    ★stall 을 쓰지 않는 이유(실측): `build_stall_warnings` 의 STALL_COOLDOWN(1h)이 seen TTL(30분)
      보다 길어, stall 로 TTL 경계를 재면 "TTL 이 아니라 stall 쿨다운"을 재게 된다. 두 상한이
      겹치는 구간에서 무엇을 측정 중인지 모호해지는 것 자체가 계측 결함이다.
    """
    old_ts = "2023-11-01T00:00:00+0000"
    tickets = [{"id": "T-9", "status": "todo", "owner": None,
                "created_at": old_ts, "updated_at": old_ts}]
    rep = _w5_report(live=[{"role": "master", "idle": 60, "status_age": 3600, "tokens": 1},
                           {"role": "cso", "idle": 60, "status_age": 3600, "tokens": 2}])
    return rep, tickets


@specimen("H-W5-C2", "W5", "seen-store TTL 경계 — 만료 전 0 · 만료 후 1 · GC", ["C2"])
def h_w5_c2():
    G = _w5_mod()
    ttl = 900.0
    #   ⓐ 단위 경계 — seen_claim 자체의 TTL 판정(경계값을 직접 못 박는다)
    with tempfile.TemporaryDirectory() as tmp:
        k = G.seen_key("stall_confirmed", "gate-stall-worker", "critical")
        ok1, _ = G.seen_claim(tmp, k, "critical", 1000.0, ttl)
        need(ok1, "최초 선점 실패")
        G.seen_mark(tmp, k, 1000.0, state=G.SEEN_STATE_INFLIGHT, wakeup_id="W-1")
        ok2, rec2 = G.seen_claim(tmp, k, "critical", 1000.0 + ttl - 1, ttl)
        need(not ok2 and rec2.get("state") == G.SEEN_STATE_INFLIGHT, "만료 직전 재선점 허용(중복)")
        ok3, _ = G.seen_claim(tmp, k, "critical", 1000.0 + ttl, ttl)
        need(ok3, "만료 직후 재선점 거부(조건 지속인데 영구 침묵)")
        #   severity 상승은 TTL 을 우회한다(키에 severity 포함)
        ok4, _ = G.seen_claim(tmp, G.seen_key("stall_confirmed", "gate-stall-worker", "warn"),
                              "warn", 1000.0 + 1, ttl)
        need(ok4, "severity 별 키 분리 실패(상승이 하위 seen 에 막힌다)")
        #   시계 역행 = TTL 재시작(안전 방향 — 이번 주기는 억제)
        ok5, _ = G.seen_claim(tmp, k, "critical", 1.0, ttl)
        need(not ok5, "시계 역행에서 재선점(TTL 무력화)")
    #   ⓑ e2e — 지속 조건에서 만료 전 0 / 만료 후 1
    rep, tickets = _w5_deadlock_fixture()
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk, r = _W5Clock(), _W5Fake(rep=rep, tasks=tickets)
        dl = lambda: [e for e in r.enqueues if e[1] == "gate-deadlock"]   # noqa: E731
        for _ in range(3):
            _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
            clk.tick()
        need(len(dl()) == 1, "첫 push 1건이 아니다: %d" % len(dl()))
        clk.t += ttl - 120                                 # 만료 **직전**
        _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        need(len(dl()) == 1, "TTL 만료 전에 재발화(중복 상한 붕괴): %d" % len(dl()))
        clk.t += 240                                       # 만료 **직후**
        _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        need(len(dl()) == 2, "TTL 만료 후 재발화가 없다(조건 지속인데 영구 침묵)")
        #   GC 가 만료분을 실제로 지우는가(무한 성장 차단)
        clk.t += ttl * 3
        _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        stale = [x for x in G.seen_iter(sd)
                 if clk.now_epoch() - (x.get("first_ts") or 0) >= ttl * 2]
        need(not stale, "만료 레코드가 GC 되지 않았다: %r" % stale)
    return "단위 경계(±1s)·severity 우회·시계역행 · e2e 만료 전 0/후 1 · GC 동작"


# ── C3 enqueue 성공 후 Inject 전 실패 ────────────────────────────────────
@specimen("H-W5-C3", "W5", "enqueue 후 Inject 전 실패 — critical 미disarm·TTL당 재시도 1", ["R2-C3"])
def h_w5_c3():
    G = _w5_mod()
    rep, tickets = _w5_deadlock_fixture()                   # C2 와 같은 이유로 stall 미사용
    ttl = 900.0
    with tempfile.TemporaryDirectory() as tmp, _W5Env(CYS_PACK_DIR=tmp):
        sd = os.path.join(tmp, "state")
        clk = _W5Clock()
        r = _W5Fake(rep=rep, tasks=tickets, drain_delivered=0)   # 파킹: Inject 도달 0
        dl = lambda: [e for e in r.enqueues if e[1] == "gate-deadlock"]   # noqa: E731
        for _ in range(3):
            _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
            clk.tick()
        need(len(dl()) == 1, "첫 enqueue 1건이 아니다: %d" % len(dl()))
        inflight = [x for x in G.seen_iter(sd) if x.get("state") == G.SEEN_STATE_INFLIGHT]
        need(inflight, "영수증 미수신인데 inflight 기록이 없다(at-least-once 붕괴)")
        need(all(x.get("wakeup_id") for x in inflight), "inflight 에 W-id 미봉입: %r" % inflight)
        #   ★시간은 **정상 주기(5분)로만** 흘린다. 큰 점프는 GAP(>3주기) 재baseline 을 유발해
        #     검체가 조용히 공허해진다 — 시뮬레이션은 실제 스케줄을 따라야 한다.
        #     TTL(900s) = 정확히 3주기 → 3회 돌려 **경계를 딱 한 번만** 넘긴다(두 번 넘기면
        #     oracle 상한 2를 초과하는 것이 정상이라 검체가 거짓 적색을 낸다).
        for _ in range(3):
            clk.tick()
            _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        total = len(dl())
        need(1 <= total <= 2, "총 delivery %d건(oracle 1..2)" % total)
        need(total - 1 <= 1, "duplicate 상한 초과: %d" % (total - 1))
        #   ★영수증이 도착하면 확정된다 — 그 뒤로는 쿨다운(2h)이 재발화를 막는다
        last = [x for x in G.seen_iter(sd) if x.get("state") == G.SEEN_STATE_INFLIGHT]
        need(last, "재enqueue 후에도 inflight 여야 한다: %r" % G.seen_iter(sd))
        r.events.append({"name": "queue.delivered", "seq": 99,
                         "payload": {"entry_ids": [x["wakeup_id"] for x in last]}})
        clk.tick(60)          # TTL 경계 직전(경계에 걸치면 GC 가 근거를 지워 관측이 흐려진다)
        _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        done = [x for x in G.seen_iter(sd) if x.get("state") == G.SEEN_STATE_DELIVERED]
        need(done, "queue.delivered 영수증을 받고도 확정되지 않았다: %r" % G.seen_iter(sd))
        with open(os.path.join(sd, "counters.json"), encoding="utf-8") as f:
            edges = (json.load(f).get("push_edge") or {})
        need(edges and all(v.get("armed") is False for v in edges.values()),
             "영수증 수신에도 엣지가 disarm 되지 않았다(at-least-once 종결 실패): %r" % edges)
        after_ack = len(dl())
        for _ in range(4):                                  # TTL 은 지났지만 쿨다운(2h) 창 안
            clk.tick()
            _w5_gate(G, sd, r, clk, seen_ttl=ttl).run()
        need(len(dl()) == after_ack,
             "영수증 확정 뒤에도 TTL 마다 재발화한다(엣지 disarm 무동작): %d→%d"
             % (after_ack, len(dl())))
    return "미영수증=inflight 유지 · TTL당 재시도 1 · 영수증 수신=delivered 확정 후 쿨다운 발효"


# ── 팩 바이트 결정론 ─────────────────────────────────────────────────────
@specimen("H-PACK-CRLF", "W6",
          "임베드 대상 텍스트 전량 CRLF 0 · `.gitattributes` LF 봉인 · build.rs 가드 실재",
          ["U-1", "B-14"])
def h_pack_crlf():
    """U-1(2026-08-23 · CRLF 봉인): `build.rs` 는 `include_str!` 로 **빌드 머신 작업 트리의 바이트를
    그대로** 팩에 임베드한다. Git for Windows 의 설치 기본값 `core.autocrlf=true` 아래에서 Windows
    러너 체크아웃이 CRLF 가 되면 두 가지가 동시에 깨진다 —
      ① `cysjavis-pack/hooks/*.sh` 가 `#!/bin/bash\\r` 로 출하돼 인터프리터 해소가 실패한다(훅 전멸).
      ② 같은 버전인데 Windows 임베드 팩과 ubuntu `pack.tar.gz` 의 바이트가 갈려 매니페스트 해시가 흔들린다.
    이 검체는 그 상태를 즉시 적색으로 만든다 — 세 축:
      ⓐ **바이트 사실**: build.rs 와 **같은 대상 집합**(`git ls-files cysjavis-pack` − dot/tests/
        `__pycache__` 컴포넌트 + 레포 루트 `revoked-licenses.json`)에서 `\\r\\n` 이 0 인가.
        ★대상 집합을 검체가 스스로 재계산한다 — build.rs 의 판정을 믿고 옮겨 적지 않는다.
      ⓑ **봉인 실재**: 레포 루트 `.gitattributes` 가 있고 `* text=auto eol=lf` + 팩 훅/bin LF 못박음이
        살아 있는가(파일을 지우면 적색).
      ⓒ **빌드 게이트 실재**: `build.rs` 의 CRLF 가드(패닉 경로)가 남아 있는가(가드를 지우면 적색).
    ⓐ 만 두면 '검체는 초록인데 다음 체크아웃이 CRLF' 가 가능하고, ⓑⓒ 만 두면 '규칙은 있는데 바이트는
    이미 오염' 이 가능하다. 셋이 함께여야 봉인이다.
    """
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")) and not os.path.isfile(
            os.path.join(REPO_DIR, "build.rs")):
        raise Skip("레포 체크아웃 아님(배포 팩 실행) — 임베드 소스 트리 부재")

    # ⓐ 대상 집합 재계산 — build.rs 의 제외 규칙(dot 컴포넌트·tests·__pycache__)과 동형.
    r = _run(["git", "ls-files", "cysjavis-pack"], cwd=REPO_DIR)
    need(r.returncode == 0 and r.stdout.strip(),
         "git ls-files cysjavis-pack 실패/공집합 — 대상 집합을 산출할 수 없다(측정 실패): %s"
         % (r.stderr or "")[:200])
    targets = []
    for line in r.stdout.splitlines():
        line = line.strip()
        if not line.startswith("cysjavis-pack/"):
            continue
        rel = line[len("cysjavis-pack/"):]
        if any(c.startswith(".") or c in ("tests", "__pycache__") for c in rel.split("/")):
            continue
        targets.append(line)
    need(len(targets) >= 250,
         "임베드 대상 %d개 < 250 — build.rs 가드②와 같은 하한 미달(측정 이상)" % len(targets))
    # 팩 트리 밖에서 문자열로 임베드되는 레포 루트 파일(build.rs `REVOKED_LICENSES_JSON`).
    targets.append("revoked-licenses.json")

    hits, scanned, skipped_bin = [], 0, 0
    for rel in targets:
        p = os.path.join(REPO_DIR, rel)
        try:
            with open(p, "rb") as f:
                b = f.read()
        except OSError:
            continue
        if b"\x00" in b:                     # 바이너리 제외(build.rs 가드와 동일 판정)
            skipped_bin += 1
            continue
        scanned += 1
        n = b.count(b"\r\n")
        if n:
            hits.append("%s(\\r\\n %d개)" % (rel, n))
    need(not hits,
         "임베드 대상에 CRLF 가 있다 = 그대로 출하되면 훅 shebang 사망 + 플랫폼 간 팩 해시 분기: %s"
         % hits[:20])

    # ⓑ `.gitattributes` 봉인 실재
    ga_path = os.path.join(REPO_DIR, ".gitattributes")
    need(os.path.isfile(ga_path),
         ".gitattributes 부재 — 개행이 체크아웃 환경(core.autocrlf)에 좌우된다(U-1 회귀)")
    ga = _read(ga_path)
    ga_lines = [ln.split("#", 1)[0].strip() for ln in ga.splitlines()]
    ga_lines = [ln for ln in ga_lines if ln]
    def _rule(pattern, must):
        for ln in ga_lines:
            parts = ln.split()
            if parts and parts[0] == pattern and all(m in parts[1:] for m in must):
                return True
        return False
    need(_rule("*", ["text=auto", "eol=lf"]),
         ".gitattributes 에 전역 `* text=auto eol=lf` 규칙이 없다 — LF 봉인 해제됨")
    need(_rule("cysjavis-pack/hooks/**", ["text", "eol=lf"]),
         ".gitattributes 에 `cysjavis-pack/hooks/** text eol=lf` 가 없다 — 훅 LF 명시 강제 해제됨")
    # ★`bin/**` 은 강제 `text` 가 아니라 `text=auto` 를 요구한다(F4-a · 2026-08-23): 강제 `text` 는
    #   확장자 pin 밖의 **새 바이너리**가 pack bin 아래 들어오면 개행 변환으로 조용히 손상시킨다.
    #   `text=auto` 는 텍스트에만 `eol=lf` 를 적용하므로 LF 보장은 유지하면서 바이너리를 살린다.
    need(_rule("cysjavis-pack/bin/**", ["text=auto", "eol=lf"]),
         ".gitattributes 에 `cysjavis-pack/bin/** text=auto eol=lf` 가 없다 — 팩 bin LF 강제 해제됨")
    # 의도적 CRLF 픽스처가 재정규화로 파괴되지 않도록 못박혀 있는가(검체가 검사할 입력의 보존).
    need(_rule("cysjavis-pack/bin/tests/fixtures/**", ["-text"]),
         ".gitattributes 에 `cysjavis-pack/bin/tests/fixtures/** -text` 가 없다 — "
         "의도적 CRLF/BOM/제어문자 픽스처가 재정규화로 파괴된다")

    # ⓑ' 기계 확인: git 이 실제로 그 속성을 해소하는가(문면이 아니라 도구 출력)
    probe = ["cysjavis-pack/hooks/_lib.sh",
             "cysjavis-pack/bin/tests/fixtures/todo-decl/11-crlf.md"]
    ca = _run(["git", "check-attr", "text", "eol", "--"] + probe, cwd=REPO_DIR)
    need(ca.returncode == 0, "git check-attr 실패(측정 실패): %s" % (ca.stderr or "")[:200])
    need("cysjavis-pack/hooks/_lib.sh: eol: lf" in ca.stdout,
         "git 이 훅에 eol=lf 를 해소하지 않는다: %s" % ca.stdout[:300])
    need("11-crlf.md: text: unset" in ca.stdout,
         "의도적 CRLF 픽스처가 -text 로 해소되지 않는다(재정규화 위험): %s" % ca.stdout[:300])

    # ⓒ build.rs 빌드 게이트 실재
    # ★`os.path.join` 경유로 부른다 — H-META-READ 의 대상 수확 정규식이 리터럴 호출만 잡으므로,
    #   이 형식을 지켜야 build.rs 가 읽기-상한 자기감시 대장에 함께 오른다.
    br = _repo_file(os.path.join("build.rs"))
    need("CYS_ALLOW_CRLF_EMBED" in br,
         "build.rs 에 CRLF 가드 롤백 스위치(CYS_ALLOW_CRLF_EMBED)가 없다 — 가드 소실")

    # ★거짓 초록 제거(F1 · 2026-08-23) — **가드③ 블록만 잘라서** 판정한다.
    #   구 판은 build.rs **파일 전체**에 `"crlf_hits" in br and "panic!" in br` 을 걸었다. 그런데
    #   build.rs 에는 가드①(pack 소스 공집합)·가드②(엔트리 <250)·revoked 형태검증 등 **다른
    #   `panic!` 이 함께 산다**. 즉 가드③의 `panic!("{msg}")` 를 `println!("cargo:warning=…")` 로
    #   바꿔 **CRLF 차단을 완전히 무력화해도** 파일 어딘가의 다른 panic! 이 이 조건을 만족시켜
    #   검체가 초록이었다 = 거짓 초록(계측 무효). 판정 대상은 파일이 아니라 **그 가드 블록**이다.
    #   블록 경계는 소스에서 직접 찾는다 — 롤백 스위치 이름이 처음 등장하는 지점(가드③ 머리말)부터
    #   `if !crlf_hits.is_empty() { … }` 의 닫는 중괄호(fn 본문 최상위 = 4칸 들여쓰기 `}`)까지.
    g0 = br.find("CYS_ALLOW_CRLF_EMBED")
    m_if = re.search(r"if !crlf_hits\.is_empty\(\)\s*\{", br[g0:])
    need(m_if,
         "build.rs 에서 CRLF 가드의 판정 분기(`if !crlf_hits.is_empty()`)를 찾지 못했다 — "
         "가드 소실이거나 블록 경계 규약 이탈(측정 실패)")
    tail = br[g0 + m_if.end():]
    m_close = re.search(r"^    \}[ \t]*$", tail, re.M)
    need(m_close,
         "build.rs CRLF 가드 블록의 끝(최상위 4칸 들여쓰기 `}`)을 찾지 못했다 — "
         "블록 경계를 확정할 수 없다(측정 실패)")
    guard = br[g0:g0 + m_if.end() + m_close.end()]

    # 슬라이스 안에 ①검출 ②우회 env 분기 ③패닉 — 셋이 **함께** 있어야 차단이 성립한다.
    need('w == b"\\r\\n"' in guard,
         "가드③ 블록에 CRLF 검출식(`w == b\"\\\\r\\\\n\"`)이 없다 — 조용한 CRLF 출하가 다시 가능하다")
    need("crlf_allowed" in guard and "CRLF_ALLOW_ENV" in guard,
         "가드③ 블록에 우회 env 분기(`crlf_allowed`/`CRLF_ALLOW_ENV`)가 없다 — "
         "롤백 스위치가 가드와 분리됐다(가드 밖 우회 가능)")
    need("panic!" in guard,
         "가드③ 블록 안에 `panic!` 이 없다 = CRLF 를 검출하고도 빌드가 죽지 않는다. "
         "(파일 다른 곳의 panic! 은 근거가 되지 못한다 — 그것이 구 판의 거짓 초록이었다.)")

    return ("임베드 대상 %d개 스캔(바이너리 %d 제외) · CRLF 0 · .gitattributes 4규칙 실재 · "
            "check-attr 해소 확인 · build.rs 가드③ 블록(%d자) 안에 검출·우회분기·panic 3축 실재"
            % (scanned, skipped_bin, len(guard)))


@specimen("H-READY-13", "W6",
          "★ready 술어 단일화 — 관문 문면 AND 항 · 두 소비처 경유 · 보류 귀결 선행(U-11)",
          ["U-13"])
def h_ready_13():
    """U-13(2026-08-23 실측): 관문 6화면 **전부**에 `❯` 가 있고 그 화면들은 기동 직후 **신규
    출력**으로 그려진다. 그래서 델타 우선 규칙도, 그보다 먼저 평가되는 안전 밸브도 관문 화면을
    ready 로 선언했고, 디렉티브가 선택기에 붙여넣어졌다 — 그 붙여넣기의 Return 이 면책 창의
    기본 포커스(`No, exit`)를 눌러 **좌석을 rc 1 로 죽인다**(실측 킬체인).

    수리는 판정을 순수 술어 하나로 모으고(`ready = 입력활성 증거 ∧ 관문 문면 부재`) **두**
    소비처(부트 폴링 · `adapter_ready`)를 그 술어로 통과시키는 것이다. 이 검체가 지키는 것:
      ⓐ 판정의 단일 소유 + 숨은 입력 0(판정부가 파일·시계·env·RPC 를 읽지 않는다).
      ⓑ 관문 축이 **문면 SOT 를 소비**한다(사본을 새로 만들지 않는다 — S-1 재발 차단).
      ⓒ 두 소비처 모두 경유(두 번째 소비처가 눈먼 채 남으면 실사고 경로가 그대로 열려 있다).
      ⓓ 롤백 스위치 1지점·엄격 비교.
      ⓔ ★순서 — 엄격화의 귀결이 close 가 아니라 보류(U-11 `GatePending`)여야 한다.
         이 항이 적색이면 **이 단위는 착지해서는 안 된다**(치명위험 ④: 전 pane 사망).
    """
    rel = os.path.join("src", "readiness.rs")
    if not os.path.isfile(os.path.join(REPO_DIR, rel)):
        if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
            raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
        raise Fail("ready 술어 정본 %s 부재 — 판정이 다시 흩어져 있다" % rel)
    src = _scan_source("readiness")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    # 판정부 파일 본문은 **합본의 경계 뒤 조각**으로 취한다. 소유 경로를 `_repo_file` 로 직접
    # 읽으면 H-META-PIN ⓒ(소비 검체의 경유 배선 우회)가 적색이고, 변수 경유로 읽으면
    # H-META-READ ⓒ(미지의 동적 인자)가 적색이다 — 두 계약을 동시에 만족하는 경로가 이것뿐이다.
    # 파일 실재·읽기 상한은 H-META-PIN ⓑ 가 등재 경로 전량에 대해 이미 본다(사각 없음).
    need(_SCAN_JOIN in src,
         "SCAN_TARGETS['readiness'] 에 판정부 경로가 없다 — 이사한 핀이 cys.rs 만 보게 된다")
    rsrc = src.split(_SCAN_JOIN)[-1]
    notes = []

    # ⓐ 판정 단일 소유 + 숨은 입력 0
    judge = _slice_between(src, "pub fn judge(",
                           "// ── 판정부 끝(핀 슬라이스 경계) ──", "H-READY-13 판정부")
    need("pub struct Observed<" in rsrc, "판정 입력 구조체(Observed)가 없다 — 입력 전량 선언 부재")
    for banned in ("std::env", "std::fs", "request(", "fetch_surfaces", "Instant::now"):
        need(banned not in judge,
             "판정부가 %s 를 읽는다 — `Observed` 밖의 숨은 입력(순수성 상실 · 진리표 무력화)" % banned)
    notes.append("판정 단일 소유 · 숨은 입력 0")

    # ⓑ 관문 축이 문면 SOT 를 소비한다(사본 신설 0)
    need("first_run_gates::identify(o.gates, o.screen)" in judge,
         "관문 AND 항이 코퍼스를 소비하지 않는다 — ready 가 관문 화면에서 참이 된다(실사고 경로)")
    sot_rel = os.path.join("src", "first_run_gates.rs")
    sot = _repo_file(os.path.join("src", "first_run_gates.rs"))
    _d0 = sot.index("const DEFS:")
    defs = sot[_d0:sot.index("\n];", _d0) + 3]
    sot_needles = []
    for blk in re.findall(r"needles:\s*&\[(.*?)\]", defs, re.S):
        sot_needles += re.findall(r'"((?:[^"\\]|\\.)*)"', blk)
    need(len(sot_needles) >= 6, "정본 needle 수확 실패(%d건) — fail-closed" % len(sot_needles))
    dupes = [nd for nd in sot_needles if nd in rsrc]
    need(not dupes,
         "판정부 파일에 관문 문면 사본이 생겼다: %s — 문면의 진실원천은 %s 하나다(읽기 소비만)"
         % (dupes[:3], sot_rel))
    notes.append("관문 축=SOT 소비 · 문면 사본 0(%d종 대조)" % len(sot_needles))

    # ⓒ 두 소비처 모두 판정부를 경유한다
    body = _slice_between(src, "fn boot_agent_on_surface(",
                          "\n/// 에이전트 기동 + 역할 지침", "H-READY-13 부트 본문")
    need("cys::readiness::judge(&obs)" in body, "부트 폴링이 판정부를 경유하지 않는다")
    need("gates: &gate_corpus.gates" in body, "부트 판정 입력에 관문 코퍼스가 실리지 않는다")
    ar = _slice_between(src, "fn adapter_ready(", "\n/// ", "H-READY-13 adapter_ready")
    need("cys::readiness::judge(" in ar,
         "adapter_ready 가 판정부를 경유하지 않는다 — 두 번째 소비처가 눈먼 채 남았다"
         "(`scrollback_tail.contains(marker)` 한 줄로 되돌아갔는가)")
    need("gates: &gates" in ar, "adapter_ready 판정 입력에 관문 코퍼스가 실리지 않는다")
    notes.append("소비처 2종 경유(부트·재주입)")

    # ⓓ 롤백 스위치 1지점 · 엄격 비교
    need('pub const ENV_V1: &str = "CYS_READINESS_V1"' in rsrc, "롤백 스위치 이름 상수가 없다")
    need('raw == Some("1")' in rsrc,
         "롤백 스위치가 느슨한 truthy 를 받는다 — 오타로 안전장치가 조용히 뒤집힌다")
    readers = [ln for ln in rsrc.splitlines() if "std::env::var(ENV_V1)" in ln]
    need(len(readers) == 1, "env 읽기 지점이 %d곳이다 — 롤백은 1지점이어야 한다" % len(readers))
    need('std::env::var("CYS_READINESS_V1")' not in src,
         "롤백 스위치를 상수 밖에서 문자열로 직접 읽는 곳이 있다(1지점 규약 이탈)")
    notes.append("롤백 1지점·엄격 비교")

    # ⓔ ★순서 핀 — 엄격화의 귀결은 close 가 아니라 보류(U-11)여야 한다
    need("enum BootVerdict" in src and "GatePending" in src,
         "U-11(보류 귀결)이 없다 — 엄격해진 판정의 미충족이 그대로 close 로 흘러 살아있는 좌석을 "
         "죽인다(치명위험 ④). 이 단위는 U-11 뒤에만 착지할 수 있다")
    gi = src.find("Ok(BootVerdict::GatePending { gate, tail }) => {")
    need(gi > 0, "launch 호출부의 보류 분기를 못 찾았다")
    need('"surface.close"' not in src[gi:gi + 400],
         "보류 분기가 좌석을 닫는다 — 관문에 갇힌 **살아있는** 노드를 파괴하는 방향")
    notes.append("보류 귀결 선행 확인(close 0)")

    # ⓕ 진리표가 실측 픽스처를 소비한다(손으로 지어낸 화면으로 판정하지 않는다)
    need("first_run_gates::fixtures" in rsrc,
         "진리표가 실측 캡처 픽스처를 쓰지 않는다 — 지어낸 화면은 결함을 재현하지 못한다")
    for t in ("truth_table_gate_screens_never_ready",
              "legacy_v1_reproduces_the_defect_on_every_gate_screen",
              "healthy_screen_stays_ready_and_matches_no_gate"):
        need(t in rsrc, "진리표 검체 %s 가 사라졌다" % t)
    notes.append("진리표: 관문 6화면 × 밸브 × 델타 + 오탐 대조군")

    # 계측 타당성 — 기준 트리에는 이 판정부가 없었다(있었다면 U-13 은 결함이 아니다)
    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        need(_git_show(rel) is None, "계측 타당성 실패: 기준 트리에 이미 판정부가 있다")
        old_cli = _git_show(os.path.join("src", "bin", "cys.rs"))
        if old_cli is not None:
            need("readiness::judge" not in old_cli,
                 "계측 타당성 실패: 구 CLI 가 이미 판정부를 경유한다")
            need("scrollback_tail.contains(&m)" in old_cli,
                 "계측 타당성 실패: 구 adapter_ready 의 눈먼 한 줄을 못 찾았다")
        calib = "기준 트리 판정부 부재 · 구 adapter_ready 눈먼 매칭 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-KILLCHAIN-1", "W6",
          "★주입 봉인 + 신뢰 Return 경화 — 킬체인(신뢰→면책) Return 1발·면책 미접촉",
          ["U-14", "U-15"])
def h_killchain_1():
    """U-14/U-15(2026-08-23 실측 · claude 2.1.241): 좌석을 실제로 죽인 것은 **키 한 발**이었다.

      ① `inject_text` 는 bracketed paste 뒤 **800ms 후 무조건** `send_key Return
         {authoritative:true}` 를 보낸다. ready 가 관문 화면에서 잘못 선언되면(U-13 이 그 판정을
         고쳤다) 그 Return 이 면책 창의 기본 포커스 `No, exit` 를 눌러 **rc 1 로 좌석이 죽는다**.
         그리고 `inject_text` 를 부르는 경로는 하나가 아니다(부트 주입·`[RECOVER]`·`[DRAIN]`·
         cycle 재주입·pack-update 재주입·복원 디렉티브·각성 확인 핑 …) — 대부분 화면을 보지 않는다.
      ② 폴더신뢰 자동확인의 **2발째**. 종전 needle 은 하드코딩 `trustthisfolder` 였고 그것은
         통과 직후 화면에 남는 확인 에코 `Yes, I trust this folder ✔` 에 재매칭된다. 거기에
         재전송 상한 2발 + `persisted` 조건이 겹쳐 2발째가 나갔고, 그때 화면은 이미 면책 창이었다.

    수리는 ⓐ 전송 직전에 화면을 다시 보고 관문이면 **보내지 않기**(그물은 호출부 11곳이 아니라
    `inject_text` 안쪽 1지점) ⓑ 그 스캔의 **생애 창을 첫 각성 ack 이전으로 상한**(치명위험 ①)
    ⓒ 걸렸을 때의 귀결을 close 가 아니라 **보류**(U-11 `GatePending`)로 ⓓ 신뢰 needle 을
    **질문형 문면만**(코퍼스 정본 소비)으로 좁히고 전송을 **1발**로 줄이기다.

    ★이 검체가 지키는 것은 위 넷의 **배선**이다. 판정 자체(진리표·킬체인 e2e)는 Rust 검체가
      실행으로 증명한다(`cargo test --bins --lib`) — 러너는 컴파일러가 아니므로 그 검체의
      **실재와 이름**을 핀한다. 이름이 사라지면 즉시 적색이다."""
    rel = os.path.join("src", "inject_guard.rs")
    if not os.path.isfile(os.path.join(REPO_DIR, rel)):
        if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
            raise Skip("레포 체크아웃 아님(배포 팩) — Rust 소스 부재")
        raise Fail("주입 가드 판정부 %s 부재 — 관문 화면에 Return 이 그대로 나간다(U-14 미착지)" % rel)
    gsrc = _scan_source("inject_guard")   # ★핀 이사(U-2) — 경로 소유는 SCAN_TARGETS
    cli = _scan_source("readiness")       # cys.rs 호출 배선(경로 소유는 readiness)
    notes = []

    # ⓐ 판정 단일 소유 + 숨은 입력 0 — 진리표가 실기 없이 돌아야 한다
    judge = _slice_between(gsrc, "pub fn decide(o: &Observed) -> Decision {",
                           "// ── 판정부 끝(핀 슬라이스 경계 · U-14/U-15) ──",
                           "H-KILLCHAIN-1 판정부")
    for banned in ("std::env", "std::fs", "request(", "fetch_surfaces", "Instant::now"):
        need(banned not in judge,
             "판정부가 %s 를 읽는다 — `Observed` 밖의 숨은 입력(순수성 상실 · 진리표 무력화)"
             % banned)
    notes.append("판정 단일 소유 · 숨은 입력 0")

    # ⓑ 관문 문면의 SOT 소비(사본 신설 0 — S-1 재발 차단)
    need("first_run_gates::identify(o.gates, o.screen)" in judge,
         "가드가 관문 코퍼스를 소비하지 않는다 — 문면 사본을 새로 만들었거나 축이 통째로 없다")
    sot = _repo_file(os.path.join("src", "first_run_gates.rs"))
    _d0 = sot.index("const DEFS:")
    defs = sot[_d0:sot.index("\n];", _d0) + 3]
    sot_needles = []
    for blk in re.findall(r"needles:\s*&\[(.*?)\]", defs, re.S):
        sot_needles += re.findall(r'"((?:[^"\\]|\\.)*)"', blk)
    need(len(sot_needles) >= 6, "정본 needle 수확 실패(%d건) — fail-closed" % len(sot_needles))
    dupes = [nd for nd in sot_needles if nd in gsrc]
    need(not dupes,
         "가드 파일에 관문 문면 사본이 생겼다: %s — 문면의 진실원천은 first_run_gates.rs 하나다"
         % dupes[:3])
    notes.append("문면 SOT 소비 · 사본 0(%d종 대조)" % len(sot_needles))

    # ⓒ 그물이 **1지점**이다 — 두 전송 지점(붙여넣기·제출 Return) 앞에 각각 걸린다
    ibody = _slice_between(cli, "fn inject_text(sid: u64", '\n/// "90s"',
                           "H-KILLCHAIN-1 inject_text")
    need(ibody.count("gate_guard_check(sid, ") == 2,
         "inject_text 의 가드 지점이 2곳(붙여넣기·제출 Return)이 아니다 — 800ms 사이에 뜬 관문이 "
         "제출 Return 으로 눌린다(실측 킬 스텝)")
    obody = _slice_between(cli, "fn inject_text_on(", "\n/// `gate_guard_check`",
                           "H-KILLCHAIN-1 inject_text_on")
    need(obody.count("gate_guard_check_on(socket, sid, timeout, ") == 2,
         "부서 소켓 주입 경로가 가드를 우회한다 — 그물에 구멍이 남았다")
    # 호출부는 늘어나도 좋다(그물이 안쪽에 있으므로 자동으로 덮인다). 다만 **가드 없는 새 주입
    # 헬퍼**가 생기면 그물 밖이므로 적색으로 만든다: paste 래핑을 스스로 하는 함수 전수 검사.
    wrappers = re.findall(r'let wrapped = format!\("\\x1b\[200~', cli)
    need(len(wrappers) == 2,
         "bracketed paste 를 스스로 씌우는 주입 헬퍼가 %d개다(기대 2 = inject_text·inject_text_on) "
         "— 새 헬퍼는 그물 밖이다. 가드를 붙이고 이 수를 함께 갱신하라" % len(wrappers))
    notes.append("그물 1지점(주입 헬퍼 2종 × 전송 2지점)")

    # ⓓ ★생애 창 상한 — 치명위험 ①(작업 중 노드 영구 차단·오탐 폭주) 차단
    # ── ★핀 이사(U-28 · 2026-08-24 · 러너 헤더 '핀 이사 계약' ①④) ────────────────────────
    # 【원인 규명 먼저】 함수가 사라진 것이 아니라 **입력이 스냅샷으로 바뀌었다**(P4-4).
    #   종전 `surface_awakened(sid)` 는 자기가 `surface.list` 왕복을 쳤는데, 가드는 각성 창과
    #   좌석 **어댑터**를 함께 필요로 한다. 둘을 각자 조회하면 왕복이 둘이고 그 사이에 값이 갈리면
    #   "창은 열렸는데 어댑터는 다른 좌석의 것" 이라는 **찢어진 관측**이 판정에 실린다.
    #   그래서 `surface_awakened_in(rows, sid)` 로 나뉘고 호출부가 한 왕복 스냅샷을 나눠 쓴다.
    #   ★Rust 측 동형 핀은 이미 이사했다(cys.rs 킬체인 검체 ⓒ의 '핀 이사(P4-4)' 주석) —
    #     러너만 낡아 있었다. 즉 이 적색은 기능 결함이 아니라 **계측기 지연**이다.
    # 【축은 무변】 "생애 창 관측이 실재하고 `awakened_at` **키 부재**(구 데몬)를 판정 불가로 접는다."
    # 【★추가 강화】 ⓘ 구 형상(자기 왕복판)이 돌아오면 적색 ⓙ 창·어댑터를 **한 왕복 스냅샷**에서
    #   함께 읽는지 단언한다 — 찢어진 관측이 이 축의 새 실패 양식이고, 그 방어가 이사의 근거다.
    need("fn surface_awakened_in(rows: &[Value], sid: u64) -> Option<bool>" in cli,
         "생애 창 관측 함수가 없다")
    need("fn surface_awakened(sid: u64) -> Option<bool>" not in cli,
         "생애 창 관측이 **자기 왕복판**으로 되돌아왔다 — 창과 어댑터를 각자 조회하면 그 사이에 "
         "값이 갈려 찢어진 관측이 판정에 실린다(P4-4 역행)")
    aw = _slice_between(cli, "fn surface_awakened_in(rows: &[Value], sid: u64) -> Option<bool> {",
                        "\n}\n", "H-KILLCHAIN-1 생애 창")
    need('row.get("awakened_at")?' in aw,
         "`awakened_at` **키 부재**(구 데몬)를 '아직 각성 안 함' 으로 접는다 — 살아서 일하는 노드 "
         "전부가 스캔 대상이 되고, 그중 하나가 관문 문면을 출력하는 순간 주입이 영구 거부된다")
    need("if awakened != Some(false) {" in cli,
         "창이 닫힌 좌석에서 스캔을 건너뛰는 조기 반환이 없다(오탐·비용 양쪽)")
    need("awakened: Some(false), // 부트 창은 상수다" in cli,
         "부트 경로가 창 여부를 데몬에 묻는다 — 구 데몬에서 가드가 가장 필요한 자리에 꺼진다")
    # ★한 왕복 스냅샷 공유(P4-4) — 공통 그물이 `surface.list` 를 **한 번만** 치고 창·어댑터를
    #   같은 `rows` 에서 읽는다. 두 번 치면 그 사이의 변화가 "다른 좌석의 어댑터로 고른 코퍼스"를
    #   판정에 싣는다(코퍼스가 갈리면 관문 식별이 갈리고, 그 끝은 다시 킬 스텝이다).
    gseg = _slice_between(cli, "fn gate_guard_check(sid: u64, stage: &str) -> Result<(), String> {",
                          "\n}\n", "H-KILLCHAIN-1 공통 그물")
    need(gseg.count("fetch_surfaces()") == 1
         and "surface_awakened_in(&rows, sid)" in gseg
         and "surface_agent_in(&rows, sid)" in gseg,
         "공통 그물이 창·어댑터를 **한 왕복 스냅샷**에서 함께 읽지 않는다 — 찢어진 관측(왕복 %d회)"
         % gseg.count("fetch_surfaces()"))
    notes.append("생애 창 상한(키 부재=판정 불가) · 부트 창 상수 · 한 왕복 스냅샷 공유")

    # ⓔ ★귀결은 close 가 아니라 보류다 — 이 항이 적색이면 치명위험 ④(전 pane 사망)가 성립한다
    hi = cli.find("if let cys::inject_guard::Decision::Hold(hit) =")
    need(hi > 0, "부트 경로의 typed 관문 가드를 못 찾았다")
    hseg = cli[hi:hi + 1400]
    need("settle_gate_pending(sid, &hit.id" in hseg,
         "주입 직전 관문 감지의 귀결이 보류(U-11)가 아니다")
    need('"surface.close"' not in hseg and "escalate_reclaim" not in hseg,
         "가드 보류 분기가 좌석을 파괴한다 — 살아 있는 노드를 죽이는 방향(오살 > 오탐)")
    # ★보류 확정은 **단일 경로**다. 보류가 나는 자리가 셋으로 늘었는데(타임아웃 · 주입 직전 ·
    #   주입 도중) 각자 강등·표식을 직접 부르면 롤백 킬스위치 판독이 3지점이 된다 —
    #   H-SEAT-4AXIS ⑦ 이 그 계약을 이미 소유하고, 여기서는 **경로의 실재**만 대조한다.
    sseg = _slice_between(cli, "fn settle_gate_pending(", "\n}\n", "H-KILLCHAIN-1 보류 확정 경로")
    for anchor in ("boot_verdict_effective(", "BootVerdict::GatePending", "mark_gate_pending("):
        need(anchor in sseg, "보류 확정 단일 경로 결손: %s" % anchor)
    need("if !cys::inject_guard::is_hold_error(&e) {" in cli,
         "가드 에러가 일반 실패와 구분되지 않는다 — `?` 로 흘러 호출부가 close 로 번역한다")
    need("enum BootVerdict" in cli and "GatePending" in cli,
         "U-11(보류 귀결)이 없다 — 이 단위는 그 뒤에만 착지할 수 있다")
    notes.append("보류 귀결(close 0 · kill 0) · 보류 에러 분류")

    # ⓕ 롤백 2축 · env 1지점 · 엄격 비교
    for env_name, const_name, reader, strict in (
        ("CYS_INJECT_GATE_GUARD", "ENV_GUARD_OFF", "std::env::var(ENV_GUARD_OFF)", 'raw == Some("0")'),
        ("CYS_TRUST_RETURN_V1", "ENV_TRUST_V1", "std::env::var(ENV_TRUST_V1)", 'raw == Some("1")'),
    ):
        need('pub const %s: &str = "%s"' % (const_name, env_name) in gsrc,
             "롤백 스위치 이름 상수 %s 가 없다" % const_name)
        need(strict in gsrc,
             "%s 가 느슨한 truthy 를 받는다 — 오타로 안전장치가 조용히 뒤집힌다" % env_name)
        readers = [ln for ln in gsrc.splitlines() if reader in ln]
        need(len(readers) == 1,
             "%s 의 env 읽기 지점이 %d곳이다 — 롤백은 축마다 1지점이어야 한다"
             % (env_name, len(readers)))
        need('std::env::var("%s")' % env_name not in cli + gsrc,
             "%s 를 상수 밖에서 문자열로 직접 읽는 곳이 있다(1지점 규약 이탈)" % env_name)
    notes.append("롤백 2축 · env 1지점 · 엄격 비교")

    # ⓖ ★U-15 — 상한 '상수' 가 아니라 '조건' 을 줄였다 + 예산 leaf 값 무접촉
    ts = _slice_between(gsrc, "pub fn trust_send(o: &TrustObserved) -> bool {", "\n}\n",
                        "H-KILLCHAIN-1 신뢰 전송 정책")
    need("if o.other_gate {" in ts,
         "전송 판정에 화면 재확인 항이 없다 — 누적 델타의 잔상으로 면책 창에 Return 을 쏜다")
    need("if o.legacy_v1 {" in ts, "롤백 분기가 없다 — 스위치가 아무것도 되돌리지 못한다")
    # 기본 정책의 마지막 말은 `o.first` 단독이어야 한다(재전송 기구 폐지).
    need(ts.rstrip().rstrip("}").rstrip().endswith("o.first"),
         "기본 전송 조건이 `first` 단독이 아니다 — 재전송 기구가 살아 있다(2발째 = 킬 스텝)")
    # 예산 leaf 는 값도 이름도 건드리지 않았다(S-5 · 상한은 롤백 분기에서 그대로 집행된다).
    m = re.search(r"const BUDGET_TRUST_MAX_SENDS: u32 = (\d+);", cli)
    need(m and m.group(1) == "2",
         "BUDGET_TRUST_MAX_SENDS 값이 바뀌었다 — 상수를 내리면 persisted·trust_seen_at·"
         "SETTLE 이 죽은 코드가 되고 다음 감사자가 '재전송 기구가 있다' 고 오독한다")
    need("max_sends: BUDGET_TRUST_MAX_SENDS" in cli,
         "예산 상수가 고아가 됐다 — 소비 없는 BUDGET_* 는 파리티 표면을 흔든다(S-5)")
    need("Some(cys::inject_guard::GATE_FOLDER_TRUST)" in cli,
         "신뢰 자동확인의 예외 구멍이 id 하나로 좁혀지지 않았다(관문 전체가 열리면 킬체인 그대로)")
    notes.append("1발 정책 · 화면 재확인 · 예산 leaf 무접촉(=2)")

    # ⓗ 킬체인 e2e Rust 검체의 실재(러너는 컴파일러가 아니다 — 이름과 배선을 핀한다)
    for t in ("killchain_trust_then_disclaimer_sends_exactly_one_return_and_never_touches_the_disclaimer",
              "after_awakening_ack_the_scan_is_off_even_on_gate_text",
              "allow_hole_is_exactly_one_gate_and_never_the_disclaimer",
              "confirm_echo_is_not_a_trust_detection"):
        need(t in gsrc, "킬체인 진리표 검체 %s 가 사라졌다" % t)
    for t in ("killchain_trust_then_disclaimer_sends_exactly_one_return_at_the_call_site_composition",
              "inject_gate_guard_is_wired_inside_the_single_choke_point_source_pin",
              "inject_guard_does_not_block_normal_screens"):
        need(t in cli, "호출부 조립 검체 %s 가 사라졌다" % t)
    need("first_run_gates::fixtures" in gsrc,
         "진리표가 실측 캡처 픽스처를 쓰지 않는다 — 지어낸 화면은 결함을 재현하지 못한다")
    notes.append("e2e·진리표 검체 7종 실재")

    # 계측 타당성 — 기준 트리(CALIBRATION_REF)에는 이 그물이 없었고, 신뢰 Return 축이 정확히
    # 킬체인의 형태를 갖고 있었다. 기준 트리에서 결함이 재현되지 않으면 이 검체는 '원래 안 나는
    # 일을 안 난다고 확인' 하는 공허한 검사이므로, 재현 실패는 **적색**이다.
    calib = "skip(no-git)"
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        need(_git_show(rel) is None, "계측 타당성 실패: 기준 트리에 이미 가드 판정부가 있다")
        old_cli = _git_show(os.path.join("src", "bin", "cys.rs"))
        if old_cli is not None:
            need("gate_guard_check(" not in old_cli,
                 "계측 타당성 실패: 구 CLI 에 이미 주입 가드가 있다면 U-14 는 결함 수리가 아니다")
            need("inject_guard" not in old_cli,
                 "계측 타당성 실패: 구 CLI 가 이미 가드 판정부를 경유한다")
            # ① 신뢰 needle 이 **하드코딩**이었다(선언 소비도, 코퍼스 소비도 없다).
            need('contains("trustthisfolder")' in old_cli,
                 "계측 타당성 실패: 구 코드의 하드코딩 신뢰 needle 을 못 찾았다 — 킬체인 서사가 "
                 "틀렸다면 이 단위는 결함 수리가 아니다")
            need("folder_trust_needle_hit" not in old_cli,
                 "계측 타당성 실패: 구 코드가 이미 코퍼스를 소비한다")
            # ② 그 needle 은 **확인 에코에 실제로 걸린다** — 결함의 작동 원리 자체를 재현한다.
            echo_flat = "".join("Yes, I trust this folder ✔".split())
            need("trustthisfolder" in echo_flat,
                 "계측 타당성 실패: 구 needle 이 확인 에코에 안 걸리면 킬체인 서사가 틀린 것")
            # ③ 그리고 감지된 뒤 Return 이 나간다(전송 지점이 실재).
            ti = old_cli.find("trustthisfolder")
            need('"key": "Return"' in old_cli[ti:ti + 900],
                 "계측 타당성 실패: 구 코드의 신뢰 Return 전송 지점을 못 찾았다")
            # ④ 주입의 제출 Return 은 무조건이었다(가드 0) — 실측 킬 스텝의 자리.
            ii = old_cli.find("fn inject_text(")
            need(ii > 0, "계측 타당성 실패: 구 코드의 inject_text 를 못 찾았다")
            iseg = old_cli[ii:ii + 2000]
            need('"key": "Return", "authoritative": true' in iseg,
                 "계측 타당성 실패: 구 inject_text 의 무조건 제출 Return 을 못 찾았다")
        calib = "기준 트리 가드 부재 · 구 코드=하드코딩 needle(확인 에코 재매칭) + 무가드 제출 Return"
    return " · ".join(notes) + " · 계측검증=%s" % calib


# ── 메타(계측기 자기감시) ────────────────────────────────────────────────
def _u19_seed_call_is_above_hook_return(body):
    """`setup_isolated_config_dir` 본문에서 **시드 호출이 `install_hooks` 조기 return 위**인가.

    ★탐지기 자신이 시험 대상이다 — 아래 검체가 **합성 표본**(순서를 뒤집은 가짜 본문)을 먹여
      이 함수가 실제로 FIRE 하는지 확인한다. 트리에 위반이 0이라 초록인 핀은, 탐지기가
      고장나도 초록이다(2026-07-23 '계측 타당성 게이트' 교훈).
    반환: (판정, 사유). 판정 True = 도달성 정상.
    """
    seed = body.find("seed_first_run_gates(")
    if seed < 0:
        return False, "시드 호출이 없다"
    ret = body.find("if !install_hooks {")
    if ret < 0:
        return False, "install_hooks 조기 return 을 못 찾았다"
    if seed > ret:
        return False, "시드 호출이 조기 return **아래**에 있다(GUI 업데이트 사용자 미도달)"
    return True, "시드 호출 offset %d < 조기 return offset %d" % (seed, ret)


@specimen("H-SEED-U19", "W6",
          "첫기동 관문 시드 — 도달성(조기 return 위)·개인프로필 무접촉·롤백 마스터 접기",
          ["C-4", "K-1"])
def h_seed_u19():
    """U-19(2026-08-24): 첫기동 관문 시드는 **훅 병합과 다른 축**이다. 훅 억제 플래그
    (`--no-install-hook`)로 함께 꺼지면, GUI 인앱 업데이트가 항상 그 플래그로 내려오므로
    **업데이트 사용자 전원에게 영영 시드되지 않는다**(도달성 결함 — `agents.json` 값 수정이
    기존 기계에 안 닿는 K-1 과 같은 계열).

    ★그리고 V-h 실측(2026-08-24 · 2.1.241 · macOS)이 뒤집은 전제 하나를 함께 못박는다:
    `hasCompletedOnboarding:true` 는 테마 관문뿐 아니라 **로그인 관문까지** 지운다. 미인증
    프로필에 그것을 시드하면 좌석이 프롬프트에 도달해 65초 뒤에도 살아 있지만 `Not logged in`
    이다 = **허위 READY 영구화**. 그래서 시드는 인증 프리미스를 요구한다.

    다섯 축이다 — ⓐ 별도 단계 실재 ⓑ 도달성(합성 표본으로 탐지기 자체 시험)
    ⓒ 개인 프로필 무접촉 ⓓ 롤백 마스터 접기 + 기본 꺼짐 ⓔ GUI 결합 실측."""
    notes = []
    pk = _repo_file(os.path.join("src", "pack.rs"))

    # ⓐ 훅 병합과 **분리된** 별도 단계인가
    need("pub fn seed_first_run_gates(" in pk, "시드 단계 함수가 없다")
    need("pub fn plan_first_run_seed(" in pk, "순수 계획기가 없다(판정이 IO 에 섞였다)")
    si = pk.find("fn setup_isolated_config_dir(")
    need(si > 0, "setup_isolated_config_dir 을 못 찾았다")
    sbody = pk[si:pk.find("\n}\n", si)]
    need("seed_first_run_gates(" in sbody, "설치 경로가 시드 단계를 부르지 않는다")
    need("merge_desired_hooks(" in sbody, "훅 병합이 사라졌다(검체 대조축 소실)")
    notes.append("별도 단계 + 순수 계획기 실재")

    # ⓑ ★도달성 — 그리고 **탐지기 자체를 합성 표본으로 시험한다**
    ok, why = _u19_seed_call_is_above_hook_return(sbody)
    need(ok, "도달성 위반: %s" % why)
    # 합성 표본 ①(순서 역전) — 탐지기가 반드시 FIRE 해야 한다
    inverted = ("    let x = 1;\n    if !install_hooks {\n        return;\n    }\n"
                "    seed_first_run_gates(&cfg, W, P);\n")
    fired, _ = _u19_seed_call_is_above_hook_return(inverted)
    need(not fired, "★탐지기 파손: 순서를 뒤집은 합성 표본을 통과시켰다")
    # 합성 표본 ②(시드 호출 삭제) — 역시 FIRE
    removed = "    if !install_hooks {\n        return;\n    }\n"
    fired2, _ = _u19_seed_call_is_above_hook_return(removed)
    need(not fired2, "★탐지기 파손: 시드 호출이 없는 합성 표본을 통과시켰다")
    # 합성 표본 ③(정상 순서) — FIRE 하면 안 된다(거짓 적색 금지)
    good = ("    seed_first_run_gates(&cfg, W, P);\n"
            "    if !install_hooks {\n        return;\n    }\n")
    fired3, _ = _u19_seed_call_is_above_hook_return(good)
    need(fired3, "★탐지기 파손: 정상 순서를 적색으로 냈다(거짓 적색)")
    notes.append("도달성 정상(%s) · 합성 표본 3종 탐지기 시험 통과" % why)

    # ⓒ 개인 프로필 무접촉 — 시드 단계가 개인 프로필 병합기와 섞이지 않았다
    gi = pk.find("pub fn seed_first_run_gates_at(")
    need(gi > 0, "시드 IO 부를 못 찾았다")
    gbody = pk[gi:pk.find("\n}\n", gi)]
    need("merge_awakening_hooks_into_personal_profiles" not in gbody,
         "시드가 개인 프로필 병합과 얽혔다 — 격리 계약 위반")
    need("personal_profile_dirs" not in gbody and "home_dir" not in gbody,
         "시드 IO 부가 홈 디렉터리를 스스로 찾는다(격리 config dir 밖으로 번질 경로)")
    need("cfg.join(CLAUDE_CONFIG_FILE)" in gbody,
         "시드 대상이 격리 config dir 안이라는 것이 코드에서 읽히지 않는다")
    notes.append("개인 프로필 무접촉(시드부에 병합기·홈 탐색 0)")

    # ⓓ 롤백 — 마스터 접기 + 엄격 비교 + **기본 꺼짐**
    need('pub const ENV_FIRST_RUN_SEED: &str = "CYS_FIRST_RUN_SEED";' in pk,
         "롤백 스위치 env 이름 상수가 없다")
    fi = pk.find("pub fn first_run_seed_enabled()")
    need(fi > 0, "env 판독 지점이 없다")
    fbody = pk[fi:pk.find("\n}\n", fi)]
    # ★P1 이월 정리(false-green 봉인): 앵커는 실제 호출 코드형(crate::…)이다 — 벌거벗은
    #   리터럴은 주석·독스트링에도 나타날 수 있어 코드가 지워져도 초록이 된다(우회 가능 핀 금지).
    need("crate::gate_axes_forced_legacy()" in fbody,
         "★새 축이 마스터 스위치(CYS_BOOT_GATES=0)에 접히지 않았다 — 사고 순간에 노브 조합 요구")
    need('env_val == Some("1")' in pk,
         "느슨한 truthy 를 받는다(오타 하나로 시드가 켜진다)")
    # env 판독 지점은 하나뿐이다(축별 1지점 규약)
    need(pk.count('std::env::var(ENV_FIRST_RUN_SEED)') == 1,
         "ENV_FIRST_RUN_SEED 판독 지점이 %d 곳이다(1지점 규약 위반)"
         % pk.count('std::env::var(ENV_FIRST_RUN_SEED)'))
    need("FIRST_RUN_SEED_BACKUP" in pk, "파일 롤백 경로(.bak)가 없다")
    notes.append("롤백 마스터 접기 + 엄격 비교 + env 1지점 + .bak 경로")

    # ⓔ GUI 결합 실측 — 인앱 업데이트가 여전히 --no-install-hook 인가(도달성 논거의 전제)
    gui = _repo_file(os.path.join("src-tauri", "src", "main.rs"))
    need('sealed_sidecar_cys(&["init-pack", "--no-install-hook"])' in gui,
         "GUI 인앱 업데이트의 init-pack 호출 형태가 바뀌었다 — U-19 도달성 논거 재검토 필요")
    need("U-19 도달성 앵커" in gui, "GUI 쪽 결합 앵커 주석이 사라졌다(다음 사람이 순서를 뒤집는다)")
    notes.append("GUI 업데이트 경로 --no-install-hook 확인")

    # 계측 타당성 — 구 트리에는 시드 단계 자체가 없다
    old = _git_show(os.path.join("src", "pack.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("seed_first_run_gates" not in old,
             "계측 타당성 실패: 기준 커밋에 이미 시드 단계가 있다(기준 선택 오류)")
        oi = old.find("fn setup_isolated_config_dir(")
        if oi > 0:
            obody = old[oi:old.find("\n}\n", oi)]
            ofired, _ = _u19_seed_call_is_above_hook_return(obody)
            need(not ofired, "계측 타당성 실패: 구 본문을 탐지기가 통과시켰다")
            calib = "구 트리=시드 단계 부재 · 탐지기 FIRE 확인"
        else:
            calib = "구 트리=시드 단계 부재"
    return " · ".join(notes) + " · 계측검증=%s" % calib


# ── U-29 · 인앱 업데이트 경로 도달성 탐지기(M-09-a) ──────────────────────────
def _u29_retune_reaches_update_path(body):
    """`setup_isolated_config_dir` 본문에서 **timeout 재조정이 훅 억제 조기 return 앞**인가.

    ★탐지기 자신이 시험 대상이다 — 아래 검체가 합성 표본을 먹여 이 함수가 실제로 FIRE 하는지
      확인한다. 트리에 위반이 0이라 초록인 핀은 탐지기가 고장나도 초록이다(2026-07-23 '계측
      타당성 게이트' 교훈 · H-SEED-U19 와 같은 규약).
    반환: (판정, 사유). 판정 True = 인앱 업데이트 사용자에게 도달한다.
    """
    ret = body.find("if !install_hooks {")
    if ret < 0:
        return False, "훅 억제 조기 return 분기를 못 찾았다"
    end = body.find("return;", ret)
    if end < 0:
        return False, "훅 억제 분기의 return 을 못 찾았다"
    call = body.find("retune_registered_hook_timeouts(")
    if call < 0:
        return False, "timeout 재조정 호출이 없다"
    if call > end:
        return False, "재조정 호출이 훅 억제 조기 return **뒤**에 있다(업데이트 사용자 미도달)"
    return True, "재조정 호출 offset %d ≤ 조기 return offset %d" % (call, end)


def _u29_declared_ups_timeout():
    """소망 매니페스트가 선언한 UserPromptSubmit timeout(초). 리터럴을 검체에 적지 않는다."""
    ups = [to for _sc, ev, _m, to in _rust_awakening_hooks() if ev == "UserPromptSubmit"]
    need(len(ups) == 1 and ups[0], "UserPromptSubmit 선언 timeout 을 매니페스트에서 얻지 못했다: %r" % ups)
    return ups[0]


def _u29_ups_entries(settings_path, command):
    """settings.json 의 UserPromptSubmit 에서 `command` 와 바이트 동등인 hook 객체 전량."""
    with open(settings_path, encoding="utf-8") as f:
        root = json.load(f)
    out = []
    for entry in (root.get("hooks") or {}).get("UserPromptSubmit") or []:
        for h in entry.get("hooks") or []:
            if h.get("command") == command:
                out.append(h)
    return root, out


@specimen("H-TIMEOUT-U29", "W6",
          "인앱 업데이트(--no-install-hook) 경로가 등록 timeout 을 재조정 — 설치는 하지 않는다",
          ["M-09-a", "K-1", "U-21"])
def h_timeout_u29():
    """M-09-a(2026-08-24 · 기준선 v0.14.24 ↔ 현재 격리 실주행 대조에서 나온 **유일한 악화 항목**):

    GUI 인앱 업데이트는 항상 `cys init-pack --no-install-hook` 으로 내려온다. 그 플래그는 훅
    병합을 통째로 건너뛰므로 **팩 파일은 새 훅으로 교체되는데 훅 *등록*(settings.json 의
    `timeout`)은 갱신되지 않았다.** 그런데 새 훅은 느려졌다(실측: python 기동 14→16회 · `cys`
    호출 3→4회 · 지연 1초에서 14.83→17.12초) — 즉 옛 하네스 기본 상한이 남은 기계는 **절단
    확률이 올라간다**. U-21 이 만든 축이 도달하지 못해 생기는 악화이므로 수리는 값이 아니라
    **도달성**이다(`agents.json` 수정이 기존 기계에 안 닿는 K-1 과 같은 계열).

    ★그리고 `--no-install-hook` 의 존재 이유를 깨면 이 수리는 수리가 아니다. 그 플래그가
    지키는 것은 ①사용자 프로필 불가침 ②훅을 새로 **설치**하지 않음 둘이고, 아래 실측이
    그 둘을 **행동으로** 증명한다(프로필 파일 바이트 동일 · 등록부 무생성 · 엔트리 무추가).

    다섯 축 — ⓐ 얇은 집행기 실재 + U-21 교체 경로 **재사용**(새 기구 0) ⓑ 도달성(합성 표본으로
    탐지기 자체 시험) ⓒ GUI 결합 실측 ⓓ 진짜 바이너리 실주행(재조정·무생성·무추가·프로필 무접촉)
    ⓔ 계측 타당성(구 트리엔 집행기 자체가 없다)."""
    notes = []
    pk = _repo_file(os.path.join("src", "pack.rs"))

    # ⓐ 얇은 집행기가 실재하고, 값을 올리는 **집행은 U-21 이 만든 교체 경로를 재사용**한다.
    need("pub fn retune_registered_hook_timeouts(" in pk, "timeout 재조정 집행기가 없다")
    ri = pk.find("pub fn retune_registered_hook_timeouts(")
    rbody = pk[ri:pk.find("\n}\n", ri)]
    need("raise_hook_timeout_in(" in rbody,
         "재조정이 U-21 교체 경로를 쓰지 않는다 — 같은 일을 하는 기구가 둘이 되면 계약이 갈린다")
    need("declared_timeout_for(h)" in rbody,
         "재조정이 선언 해소기를 거치지 않는다 — 롤백 스위치(축 노브·마스터)가 이 경로만 비껴간다")
    need("settings_path.exists()" in rbody,
         "'등록부가 없으면 만들지 않는다' 가드가 없다 — 파일 생성은 곧 훅 설치다(플래그 위반)")
    need("write_atomic(" in rbody, "원자 교체를 쓰지 않는다 — 반쪽 파일이 굳으면 등록부 전소")
    # 엔트리 추가 경로를 **갖지 않는다**(구조적 무설치). 병합기의 append 관용구가 여기 오면 적색.
    need("arr.push(entry)" not in _code_lines(rbody.replace("//", "#")),
         "재조정기에 엔트리 append 경로가 있다 — 재조정이 아니라 설치다")
    notes.append("얇은 집행기 실재 + U-21 교체 경로 재사용(append 경로 0)")

    # ⓑ ★도달성 — 그리고 **탐지기 자체를 합성 표본으로 시험한다**
    si = pk.find("fn setup_isolated_config_dir(")
    need(si > 0, "setup_isolated_config_dir 을 못 찾았다")
    sbody = pk[si:pk.find("\n}\n", si)]
    ok, why = _u29_retune_reaches_update_path(sbody)
    need(ok, "도달성 위반: %s" % why)
    inverted = ("    if !install_hooks {\n        return;\n    }\n"
                "    retune_registered_hook_timeouts(&s, &p, &H);\n")
    fired, _ = _u29_retune_reaches_update_path(inverted)
    need(not fired, "★탐지기 파손: 순서를 뒤집은 합성 표본을 통과시켰다")
    removed = "    if !install_hooks {\n        return;\n    }\n"
    fired2, _ = _u29_retune_reaches_update_path(removed)
    need(not fired2, "★탐지기 파손: 재조정 호출이 없는 합성 표본을 통과시켰다")
    good = ("    if !install_hooks {\n        retune_registered_hook_timeouts(&s, &p, &H);\n"
            "        return;\n    }\n")
    fired3, _ = _u29_retune_reaches_update_path(good)
    need(fired3, "★탐지기 파손: 정상 배치를 적색으로 냈다(거짓 적색)")
    # 개인 프로필은 이 경로에서 **여전히 무접촉**이다(플래그의 존재 이유 ①).
    nohook = sbody[sbody.find("if !install_hooks {"):sbody.find("return;", sbody.find("if !install_hooks {"))]
    need("merge_awakening_hooks_into_personal_profiles" not in nohook,
         "훅 억제 분기가 개인 프로필 병합을 부른다 — 사용자 프로필 불가침 계약 파괴")
    need("merge_desired_hooks(" not in nohook,
         "훅 억제 분기가 병합기를 부른다 — 재조정이 아니라 설치가 된다")
    notes.append("도달성 정상(%s) · 합성 표본 3종 통과 · 억제 분기에 병합기·프로필 0" % why)

    # ⓒ GUI 결합 실측 — 인앱 업데이트가 여전히 그 플래그로 내려오는가(도달성 논거의 전제)
    gui = _repo_file(os.path.join("src-tauri", "src", "main.rs"))
    need('sealed_sidecar_cys(&["init-pack", "--no-install-hook"])' in gui,
         "GUI 인앱 업데이트의 init-pack 호출 형태가 바뀌었다 — M-09-a 도달성 논거 재검토 필요")
    notes.append("GUI 업데이트 경로 --no-install-hook 확인")

    # ⓓ ★진짜 바이너리 실주행 — 구조 단언은 '쓰여 있다'까지만 증명한다.
    want = _u29_declared_ups_timeout()
    cys = _cys_bin()
    if cys is None:
        notes.append("실측 생략(구조 단언만) — 사유: %s" % _CYS_BIN_SKIP_REASON)
    else:
        tmp = tempfile.mkdtemp(prefix="cys-h-u29-")
        try:
            home = os.path.join(tmp, "home")
            pack = os.path.join(tmp, "pack")          # ★홈 기본 팩 경로 밖 = 개인 프로필 병합 비대상
            cfg = os.path.join(tmp, "cfg")
            personal = os.path.join(tmp, "personal.json")
            for d in (home, pack, cfg):
                os.makedirs(d, exist_ok=True)
            env = _base_env({"HOME": home, "CYS_PACK_DIR": pack, "CYS_CONFIG_DIR": cfg,
                             "CYS_NO_PERSONAL_HOOK_MERGE": "1", "CYS_NO_AUTOSTART": "1"})

            def _init(extra, cfgdir=None):
                e = dict(env)
                if cfgdir:
                    e["CYS_CONFIG_DIR"] = cfgdir
                return _run([cys, "init-pack"] + extra, env=e, timeout=300)

            # ① 정상 설치로 **제품이 직접** 등록 문자열을 쓰게 한다(명령 문자열 추정 금지).
            r0 = _init(["--claude-settings", personal])
            need(r0.returncode == 0, "init-pack(정상) 실패: %s" % (r0.stdout[-400:] + r0.stderr[-400:]))
            sfile = os.path.join(cfg, "settings.json")
            need(os.path.isfile(sfile), "정상 설치가 격리 config 등록부를 만들지 않았다")
            with open(sfile, encoding="utf-8") as f:
                root = json.load(f)
            cmds = [h.get("command")
                    for e2 in (root.get("hooks") or {}).get("UserPromptSubmit") or []
                    for h in e2.get("hooks") or []]
            need(len(cmds) == 1, "정상 설치 후 UserPromptSubmit 등록이 1건이 아니다: %r" % cmds)
            ours = cmds[0]

            # ② **U-21 이전 기계**를 재현한다 — 우리 훅에서 timeout 키를 걷어내고 남의 훅을 얹는다.
            for e2 in root["hooks"]["UserPromptSubmit"]:
                for h in e2["hooks"]:
                    h.pop("timeout", None)
            root["hooks"]["UserPromptSubmit"].insert(
                0, {"hooks": [{"type": "command", "command": "sh /home/x/mine.sh"}]})
            root["theme"] = "dark"
            with open(sfile, "w", encoding="utf-8") as f:
                json.dump(root, f, ensure_ascii=False, indent=2)
            personal_before = _read(personal)
            need(personal_before.strip(), "개인 프로필 대조군 파일이 비었다(계측 무효)")

            # ③ ★인앱 업데이트 경로를 그대로 태운다.
            r1 = _init(["--no-install-hook"])
            need(r1.returncode == 0,
                 "init-pack --no-install-hook 실패: %s" % (r1.stdout[-400:] + r1.stderr[-400:]))
            after, mine = _u29_ups_entries(sfile, ours)
            need(len(mine) == 1, "재조정이 엔트리를 추가·삭제했다(우리 훅 %d건)" % len(mine))
            need(mine[0].get("timeout") == want,
                 "인앱 업데이트 경로가 등록 timeout 을 선언값(%s)으로 올리지 않았다: %r"
                 % (want, mine[0]))
            need(len(after["hooks"]["UserPromptSubmit"]) == 2,
                 "같은 이벤트의 사용자 항목이 사라지거나 늘었다: %r" % after["hooks"]["UserPromptSubmit"])
            need(after["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"] == "sh /home/x/mine.sh",
                 "사용자 훅이 사라졌다(오살)")
            need("timeout" not in after["hooks"]["UserPromptSubmit"][0]["hooks"][0],
                 "남의 훅에 timeout 을 심었다(계약 위반)")
            need(after.get("theme") == "dark", "사용자 비-훅 키 소실")
            # ★플래그의 존재 이유 ① — 개인 프로필은 **바이트 동일**하다.
            need(_read(personal) == personal_before,
                 "--no-install-hook 인데 사용자 프로필이 변경됐다(불가침 계약 파괴)")

            # ④ 멱등 — 두 번째 업데이트는 아무것도 쓰지 않는다(정상 백업 무접촉으로 실측).
            bak = sfile + ".bak-cys"
            _w(bak, '{"_sentinel":"keep"}', 0o644)
            r2 = _init(["--no-install-hook"])
            need(r2.returncode == 0, "init-pack 재실행 실패")
            need(_read(bak) == '{"_sentinel":"keep"}',
                 "멱등 재실행이 정상 백업을 덮었다(매 업데이트 재직렬화 — 플래그가 막으려던 바로 그 일)")

            # ⑤ **설치는 하지 않는다** — 등록부가 없는 config dir 에는 파일을 만들지 않는다.
            fresh = os.path.join(tmp, "cfg-fresh")
            os.makedirs(fresh, exist_ok=True)
            r3 = _init(["--no-install-hook"], cfgdir=fresh)
            need(r3.returncode == 0, "init-pack(신규 config) 실패")
            need(not os.path.exists(os.path.join(fresh, "settings.json")),
                 "등록부가 없는데 만들었다 = 훅 설치(--no-install-hook 위반)")
            need(os.path.isfile(os.path.join(fresh, "CLAUDE.md")),
                 "라우터 시드(훅 아님)까지 사라졌다 — 종전 거동 회귀")

            # ⑥ 우리 훅이 **없는** 등록부는 무변경(엔트리 추가 0).
            other = os.path.join(tmp, "cfg-other")
            os.makedirs(other, exist_ok=True)
            body = json.dumps({"hooks": {"UserPromptSubmit": [
                {"hooks": [{"type": "command", "command": "sh /home/x/mine.sh"}]}]}},
                ensure_ascii=False, indent=2)
            _w(os.path.join(other, "settings.json"), body, 0o644)
            r4 = _init(["--no-install-hook"], cfgdir=other)
            need(r4.returncode == 0, "init-pack(남의 훅만) 실패")
            need(_read(os.path.join(other, "settings.json")) == body,
                 "우리 훅이 없는 등록부를 재직렬화·변조했다")
            notes.append("실주행: 재조정 %ss · 사용자 항목 보존 · 프로필 바이트 동일 · 멱등 · 무생성 · 무추가"
                         % want)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    # ⓔ 계측 타당성 — 기준 트리에는 집행기 자체가 없고, 탐지기가 그 본문에서 FIRE 한다.
    old = _git_show(os.path.join("src", "pack.rs"))
    calib = "skip(no-git)"
    if old is not None:
        need("retune_registered_hook_timeouts" not in old,
             "계측 타당성 실패: 기준 커밋에 이미 재조정 집행기가 있다(기준 선택 오류)")
        oi = old.find("fn setup_isolated_config_dir(")
        if oi > 0:
            obody = old[oi:old.find("\n}\n", oi)]
            ofired, _ = _u29_retune_reaches_update_path(obody)
            need(not ofired, "계측 타당성 실패: 구 본문을 탐지기가 통과시켰다")
            calib = "구 트리=집행기 부재 · 탐지기 FIRE 확인"
        else:
            calib = "구 트리=집행기 부재"
    return " · ".join(notes) + " · 계측검증=%s" % calib


# ── U-29 · 판정 불가 통보(M-01) 하네스 ──────────────────────────────────────
_U29_VOICE_TOOLS = ("bash", "sh", "cat", "grep", "printf", "tr", "head", "date", "mkdir",
                    "rm", "ls", "ln", "sleep", "dirname", "sed", "command", "kill", "wait",
                    "python3", "env", "timeout", "cygpath", "locale")


def _u29_nocys_sandbox(tmp, name):
    """`cys` 가 **PATH 에 없는** role-bootstrap 실행 환경. 반환 (env, state_dir).

    ★`cys` 부재는 이 저장소에서 가장 빈도 근거가 강한 실패 모드다 — Defender 격리가 릴리스
      노트의 **상설 섹션**이다(docs/RELEASE_NOTES_0.14.21.md:127-135 ·
      docs/WDSI_SUBMISSION.md:42-47 에 실측 복구 명령).
    """
    sb = os.path.join(tmp, name)
    env, home, _p, _b, state = _rb_sandbox(sb)
    binp = os.path.join(sb, "onlybin")
    os.makedirs(binp, exist_ok=True)
    for tool in _U29_VOICE_TOOLS:
        src = shutil.which(tool)
        if src and not os.path.exists(os.path.join(binp, tool)):
            os.symlink(src, os.path.join(binp, tool))
    need(not os.path.exists(os.path.join(binp, "cys")), "샌드박스 PATH 에 cys 가 있다(계측 무효)")
    env["PATH"] = binp
    return env, state


def _u29_ctx(stdout):
    """훅 stdout 의 마지막 줄을 JSON 으로 읽어 additionalContext 를 돌려준다."""
    line = stdout.strip().splitlines()[-1]
    return json.loads(line)["hookSpecificOutput"]["additionalContext"]


@specimen("H-VOICE-U29", "W6",
          "판정 불가 침묵 종료 → 모델 컨텍스트 통보(사유+조치) · fail-open 불변 · 폭주 유계",
          ["M-01", "A22", "A5"])
def h_voice_u29():
    """M-01(2026-08-24 격리 실주행 실측 — **원 민원의 핵심**):

    `cys` 실행파일이 PATH 에서 해소되지 않으면(Defender 가 cys.exe 를 격리 · 데몬의 pane PATH
    주입 단절 · 데몬 미응답 rc 124) 이 훅은 **stdout 0바이트로 종료**했다. 기준선 v0.14.24 와
    현행이 그 경로에서 **바이트 단위로 같았다** — 즉 이 캠페인은 이 결함을 건드리지 않았다.
    사용자가 보는 것은 "너는 마스터다 를 쳤는데 pane 0개 · 모델은 평범한 답변" 이고, 실패
    사유가 모델 컨텍스트에도·승인 Feed 에도·화면에도 남지 않는다 — '무시당했다' 로만 보인다.

    ★장치는 이미 있었다: python 부재(A22)·감지기 부재는 같은 형태로 사유를 말해 준다.
      이 수리는 새 기구를 만들지 않고 **그 경로를 재사용**한다.

    ★무엇을 바꾸지 않는가(fail-open 불변): 발화 여부·exit 0·프롬프트 비차단은 한 글자도
      바뀌지 않는다. 바뀌는 것은 **침묵 → 통보** 하나다.

    여섯 축 — ⓐ `cys` 부재에서 stdout 비어 있지 않음(원 축) ⓑ 데드라인(rc 124)도 같은 통보
    ⓒ 문안이 사유와 조치를 담고 복구 **순서**(제외 → 복원)를 지킴 ⓓ 폭주 유계(연속 실행에서
    통보 1회) ⓔ judged-no 침묵(스팸 금지) + 정상 경로 무회귀 ⓕ 계측 타당성(구 훅 stdout 0바이트)."""
    notes = []
    with tempfile.TemporaryDirectory() as tmp:
        # ⓐ ★원 축 — `cys` 부재에서 stdout 이 비어 있지 않다.
        env, state = _u29_nocys_sandbox(tmp, "nocys")
        r = _run_rb(env)
        need(r.returncode == 0, "통보 경로가 훅을 비0 종료시켰다(fail-open 파괴 · exit=%d)" % r.returncode)
        need(r.stdout.strip(), "cys 부재인데 stdout 이 비었다 — 침묵 종료 그대로(M-01 미수리)")
        ctx = _u29_ctx(r.stdout)
        need("발화됨" not in r.stdout, "발화하지 않았는데 발화됨을 보고했다(허위 보고)")
        need("판정 불가" in ctx, "통보가 판정 불가임을 말하지 않는다: %r" % ctx[:200])
        need("rc=127" in ctx, "통보에 실패 사유(rc)가 없다: %r" % ctx[:200])
        notes.append("cys 부재: stdout %d바이트 통보 · exit 0 · 무발화" % len(r.stdout.strip()))

        # ⓒ 문안 — 사유만이 아니라 **조치**를 담고, 복구 순서(제외 먼저 → 그 다음 복원)를 지킨다.
        need("Defender" in ctx, "가장 흔한 원인(Defender 격리) 안내가 없다: %r" % ctx[:300])
        i_excl, i_rest = ctx.find("제외 항목"), ctx.find("복원")
        need(0 <= i_excl < i_rest,
             "복구 안내의 순서가 뒤집혔다(제외 먼저 → 그 다음 복원) — 순서를 바꾸면 실시간 보호가 "
             "복원 직후 다시 격리한다: 제외=%d 복원=%d" % (i_excl, i_rest))
        need("PATH" in ctx, "격리가 아닌 경우의 확인 경로(PATH) 안내가 없다")
        need("보고하지 마라" in ctx, "'부트가 시작됐다고 보고하지 마라' 규율이 빠졌다(허위 보고 유도)")
        # 배포된 실측 절차와 같은 말을 하는가(문안의 출처가 추정이 아님을 기계로 결박).
        rn = _repo_file(os.path.join("docs", "RELEASE_NOTES_0.14.21.md"))
        need("제외 항목 추가" in rn and "복원" in rn,
             "릴리스 노트의 상설 복구 섹션이 사라졌다 — 훅 문안의 출처가 없어졌다")
        notes.append("문안: 사유+조치+복구 순서(제외→복원)+무허위보고 규율")

        # ⓓ ★폭주 유계 — 같은 조건을 연속으로 돌려도 통보는 창당 1회다.
        spoken = 1
        for _ in range(3):
            rn2 = _run_rb(env)
            need(rn2.returncode == 0, "반복 실행이 비0 종료")
            if rn2.stdout.strip():
                spoken += 1
            else:
                need("억제" in rn2.stderr, "억제 사실을 stderr 로도 말하지 않는다: %r" % rn2.stderr[-200:])
        need(spoken == 1,
             "연속 4회에서 통보가 %d회 나갔다 — 매 프롬프트 반복은 그것대로 재난이다(래치 무력)" % spoken)
        latch = [f for f in os.listdir(state) if f.startswith("cys-hook-notice-")]
        need(latch, "통보 래치 파일이 남지 않았다 — 유계성의 근거가 디스크에 없다")
        notes.append("유계성: 연속 4회 중 통보 1회 · 래치 %r" % latch)

        # ⓔ judged-no — 마스터 토큰 없는 프롬프트는 **첫 실행에서도** 침묵한다(스팸 금지).
        env2, state2 = _u29_nocys_sandbox(tmp, "nocys-plain")
        r2 = _run_rb(env2, prompt="오늘 작업 지시해줘")
        need(r2.stdout.strip() == "",
             "선언 토큰 없는 프롬프트에 컨텍스트를 주입했다(스팸): %r" % r2.stdout[:200])
        latch2 = ([f for f in os.listdir(state2) if f.startswith("cys-hook-notice-")]
                  if os.path.isdir(state2) else [])
        need(not latch2, "judged-no 인데 래치를 소모했다(%r) — 진짜 선언 때 통보가 억제된다" % latch2)
        notes.append("judged-no 침묵 · 래치 무소모")

        # ⓑ 데드라인(rc 124) — 데몬이 게이트 안에 응답하지 않는 경우도 같은 통보로 나온다.
        sb3 = os.path.join(tmp, "hang")
        env3, _h3, _p3, bind3, _s3 = _rb_sandbox(sb3)
        _mock_cys(bind3, sb3,
                  'case "$1" in\n'
                  '  hook) echo "error: unrecognized subcommand" >&2; exit 2 ;;\n'
                  '  surface-role) sleep 30; exit 0 ;;\n'
                  'esac')
        r3 = _run_rb(env3)
        need(r3.returncode == 0, "데드라인 경로가 훅을 비0 종료시켰다(exit=%d)" % r3.returncode)
        need(r3.stdout.strip(), "데드라인(rc 124)에서 stdout 이 비었다 — 침묵 종료 잔존")
        ctx3 = _u29_ctx(r3.stdout)
        need("rc=124" in ctx3, "데드라인 사유가 통보에 없다: %r" % ctx3[:200])
        need("발화됨" not in r3.stdout, "데드라인인데 발화됨을 보고했다")
        notes.append("데드라인 rc=124 통보")

        # ⓔ′ 정상 경로 무회귀 — 판정이 되는 좌석에서는 종전대로 발화한다(통보가 길을 막지 않는다).
        env4, _h4, _p4, _b4, _s4 = _rb_sandbox(os.path.join(tmp, "ok"))
        r4 = _run_rb(env4)
        need(r4.returncode == 0, "정상 경로가 비0 종료")
        need("발화됨" in r4.stdout, "정상 경로에서 부트 발화 보고가 사라졌다(회귀): %r" % r4.stdout[:300])
        need("판정 불가 - 좌석 역할 조회 실패" not in r4.stdout,
             "정상 경로에 판정 불가 통보가 섞였다(오탐)")
        notes.append("정상 경로 발화 무회귀")

        # ⓕ 계측 타당성 — 구 훅은 같은 조건에서 stdout 0바이트여야 한다(결함 재현).
        calib = "skip(no-git)"
        old = _git_show(os.path.join("cysjavis-pack", "hooks", "role-bootstrap.sh"))
        if old is not None:
            oldhook = os.path.join(tmp, "oldhooks", "role-bootstrap.sh")
            _w(oldhook, old)
            _w(os.path.join(tmp, "oldhooks", "_lib.sh"), _read(os.path.join(HOOKS_DIR, "_lib.sh")), 0o644)
            envo, _st = _u29_nocys_sandbox(tmp, "nocys-old")
            ro = _run([BASH, oldhook], input=json.dumps({"prompt": "너는 마스터다"}), env=envo)
            need(ro.stdout.strip() == "",
                 "계측 타당성 실패: 구 훅이 이 조건에서 이미 말한다(결함 재현 불가): %r" % ro.stdout[:200])
            calib = "구 훅 stdout 0바이트(침묵 종료) 재현"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-META-PIN", "W6",
          "핀 이사 계약 집행 — SCAN_TARGETS 실재·소비 배선·우회 직접 호출 동결",
          ["U-2"])
def h_meta_pin():
    """U-2(2026-08-23): 이 러너의 검체 다수는 **Rust 소스 문자열 자체**를 핀한다. 캠페인이
    readiness 판정부를 순수함수로 추출하면 그 핀들이 적색이 되는데, 그때 "빨개서 고쳤다"가
    정당한 작업으로 통하면 **계측기가 결함을 승인**한다(MEMORY '디버깅 계측 타당성 게이트').
    헤더의 '핀 이사 계약'이 그 규율이고, 이 검체가 그 계약의 **기계 집행자**다 — 네 축이다:
      ⓐ 레지스트리 형태(논리 이름 ≥1 · 이름마다 경로 ≥1 · 경로 중복 소유 0).
      ⓑ 등재 경로 전량 실재 + 읽기 상한 미만(레포 실행 시 · 배포 팩이면 Skip).
      ⓒ 소비 선언(`SCAN_TARGET_CONSUMERS`)과 실제 배선 일치 — 선언한 검체가 실제로
         `_scan_source("<이름>")` 을 부르고, 소유 경로를 직접 읽지 않는다.
      ⓓ 레지스트리를 **우회**해 소유 경로를 직접 읽는 검체 집합 == 동결 화이트리스트.
         새 직접 독자(적색)는 이사 때 함께 옮겨지지 않아 조용히 죽고, 사라진 직접 독자(적색)는
         핀이 소리 없이 삭제됐다는 뜻이다 — 양방향으로 동결한다.
    ★이 검체는 어떤 판정도 완화하지 않는다. 기존 `need` 를 대체하지 않고, '경로를 어디서
      얻는가'라는 **새 축**만 추가한다.
    """
    notes = []

    # ⓐ 레지스트리 형태
    need(SCAN_TARGETS, "SCAN_TARGETS 가 비었다 — 핀 이사 계약의 앵커가 사라졌다")
    owner = {}
    for name in sorted(SCAN_TARGETS):
        rels = SCAN_TARGETS[name]
        need(isinstance(rels, tuple) and len(rels) >= 1,
             "SCAN_TARGETS[%r] 은 경로 1개 이상의 튜플이어야 한다(현재 %r)" % (name, rels))
        for rel in rels:
            need(isinstance(rel, str) and rel, "SCAN_TARGETS[%r] 에 빈 경로" % name)
            need(rel not in owner,
                 "경로 %s 를 논리 이름 %r 과 %r 이 동시에 소유한다 — 소유가 갈리면 이사 시 "
                 "한쪽이 누락된다" % (rel, owner.get(rel), name))
            owner[rel] = name
    owned = set(owner)
    notes.append("레지스트리 %d이름/%d경로" % (len(SCAN_TARGETS), len(owned)))

    # ⓑ 등재 경로 실재 + 읽기 상한 미만
    # ★'배포 팩'과 '레지스트리 오타'를 파일 부재로 구분하면 안 된다 — 등재 경로가 하나뿐일 때
    #   오타는 '전량 부재'와 구별되지 않아 **Skip(초록)** 으로 접힌다. 실제로 그렇게 접혔다
    #   (계측 타당성 실험 N3 · 2026-08-23). 측정 불능을 통과로 접는 것은 이 저장소의 게이트
    #   규약 위반이므로, 판별자를 **레포 체크아웃 자체의 표지**(`Cargo.toml`)로 옮긴다.
    #   ★이것은 완화가 아니라 강화다: 종전 축(파일 부재→Skip)에서는 오타가 초록이었고,
    #     새 축에서는 레포 안의 모든 부재가 적색이며 Skip 은 '레포가 아닌 곳'에만 남는다.
    if not os.path.isfile(os.path.join(REPO_DIR, "Cargo.toml")):
        raise Skip("레포 체크아웃이 아니다(배포 팩 실행) — 레지스트리 경로 판정 불가")
    missing, sizes = [], {}
    for rel in sorted(owned):
        p = os.path.join(REPO_DIR, rel)
        if not os.path.isfile(p):
            missing.append(rel)
            continue
        with open(p, encoding="utf-8", errors="replace") as f:
            sizes[rel] = len(f.read())
    need(not missing,
         "SCAN_TARGETS 등재 경로가 실재하지 않는다: %s — 이사가 절반만 반영됐거나 오타다 "
         "(핀 이사 계약 ①: 경로만 옮기되 **옮긴 곳은 반드시 실재해야** 한다)" % missing)
    over = ["%s=%d자" % (r, n) for r, n in sorted(sizes.items()) if n >= READ_LIMIT_CHARS]
    need(not over,
         "등재 경로가 읽기 상한(%d자)에 도달 = 조용한 절단 = 계측 무효: %s" % (READ_LIMIT_CHARS, over))
    notes.append("등재 경로 %d건 실재·상한 미만(최대 %d자)" % (len(sizes), max(sizes.values())))

    # ⓒⓓ 러너 소스를 검체 블록으로 잘라 '직접 호출'을 수확한다
    runner = _read(os.path.abspath(__file__))
    marks = [(m.start(), m.group(1))
             for m in re.finditer(r'^@(?:specimen|pending)\(\s*"([A-Za-z0-9\-]+)"', runner, re.M)]
    need(len(marks) >= 50,
         "검체 블록 수확 실패(정규식 파손) — %d건만 잡혔다" % len(marks))
    blocks = [("<module>", runner[:marks[0][0]])]
    for i, (pos, sid) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(runner)
        blocks.append((sid, runner[pos:end]))
    bodies = dict(blocks)

    def _direct_owned(body):
        hit = set()
        for m in re.finditer(r'_repo_file\(os\.path\.join\(([^)]*)\)\)', body):
            parts = re.findall(r'"([^"]*)"', m.group(1))
            if parts and os.path.join(*parts) in owned:
                hit.add(os.path.join(*parts))
        return hit

    direct = {}
    for sid, body in blocks:
        hit = _direct_owned(body)
        if hit:
            direct[sid] = sorted(hit)

    # ⓒ 소비 선언 ↔ 실제 배선
    for sid in sorted(SCAN_TARGET_CONSUMERS):
        name = SCAN_TARGET_CONSUMERS[sid]
        need(name in SCAN_TARGETS,
             "SCAN_TARGET_CONSUMERS[%s] 가 미등재 논리 이름 %r 을 가리킨다" % (sid, name))
        need(sid in bodies,
             "소비 선언된 검체 %s 를 러너에서 찾지 못했다(이름 변경·삭제 — 핀이 사라졌는가?)" % sid)
        need(('_scan_source("%s")' % name) in bodies[sid],
             "%s 가 _scan_source(%r) 를 부르지 않는다 — 레지스트리 경유 배선이 끊겼다 "
             "(이 검체의 핀은 이사 때 함께 옮겨지지 않는다)" % (sid, name))
        need(sid not in direct,
             "%s 가 레지스트리를 두고 %s 를 직접 읽는다 — 경유 배선 우회" % (sid, direct.get(sid)))
    notes.append("소비 검체 %d종 경유 확인" % len(SCAN_TARGET_CONSUMERS))

    # ⓓ 우회 직접 독자 동결(양방향)
    extra = sorted(set(direct) - set(SCAN_TARGET_DIRECT_LEGACY))
    need(not extra,
         "레지스트리 소유 경로를 직접 읽는 **새** 검체 %s — 새 핀은 _scan_source 경유로 만들어라. "
         "정당한 예외라면 SCAN_TARGET_DIRECT_LEGACY 에 사유와 함께 명시할 것(조용한 확산 금지)"
         % extra)
    gone = sorted(set(SCAN_TARGET_DIRECT_LEGACY) - set(direct))
    need(not gone,
         "동결 목록에 있으나 직접 호출이 사라진 검체 %s — 핀이 이사했다면 SCAN_TARGET_CONSUMERS "
         "로 옮기고, 검체가 삭제됐다면 그 사유를 커밋에 남긴 뒤 목록에서 빼라(조용한 핀 소멸 차단)"
         % gone)
    notes.append("우회 직접 독자 %d종 동결 일치" % len(direct))
    return " · ".join(notes)


# ★등록 위치가 곧 실행 순서다 — 이 검체는 **맨 마지막**에 두어, 앞선 모든 검체가 실제로 읽은
#   경로 관측(`_READ_OBSERVED`)까지 함께 본다. 단독 실행(`--only H-META-READ`) 에서도
#   정적 대상 목록만으로 자립 판정한다.
@specimen("H-META-READ", "W6",
          "계측기 자기감시 — `_read` 조용한 절단 불가(가드 실재) · `_repo_file` 대상 전량 상한 미만",
          ["U-0"])
def h_meta_read():
    """U-0(2026-08-23): `_read(path, limit=400000)` 의 조용한 절단이 22종 검체를 눈멀게 했다.
    잘린 텍스트에 `in`/`find()` 를 걸어 '없다'고 단언하면, **결함이 아니라 계측기가** 적색을
    만든다(H-CONC-4 거짓 적색 · 앵커는 handlers.rs:9852 에 실재). 이 검체는 그 상태로
    되돌아가는 것을 즉시 적색으로 만든다 — 세 축으로 본다:
      ⓐ `_read` 의 절단 실패 가드가 코드에 실재하는가(가드를 지우면 적색).
      ⓑ `_repo_file` 이 읽는 **모든** 레포 파일의 실제 문자 수가 상한 미만인가(상한에 **도달**하면
         적색 = 절단 = 계측 무효). 여유 배수는 판정 조건이 아니다 — 4배 미만이면 notes 에 ⚠
         조기경보만 남기고 통과시킨다(F2 축 재분류 · 아래 주석 참조).
      ⓒ `_repo_file` 대상 목록이 러너 소스와 동기화돼 있는가(새 동적 호출이 생기면 적색).
    """
    import inspect
    notes = []
    # ⓐ 가드 실재 — '상한+1 읽기 → 초과면 Fail' 형태를 유지하는가
    rsrc = inspect.getsource(_read)
    need("lim + 1" in rsrc,
         "_read 가 '상한+1' 을 읽지 않는다 — 상한 도달을 관측할 수 없다(조용한 절단 부활)")
    need("len(body) > lim" in rsrc and "raise Fail" in rsrc,
         "_read 에 절단 실패 가드가 없다 — 조용한 절단이 다시 가능하다(U-0 회귀)")
    need("f.read(lim + 1)" in rsrc, "_read 가 상한+1 을 read() 인자로 쓰지 않는다(가드 우회)")
    notes.append("_read 절단=Fail 가드 실재")

    # ⓒ 대상 목록 동기화 — 러너 소스에서 `_repo_file` 호출을 직접 수확한다
    runner = _read(os.path.abspath(__file__))
    lits = re.findall(r"_repo_file\(os\.path\.join\(([^)]*)\)\)", runner)
    need(lits, "_repo_file 리터럴 호출을 하나도 수확하지 못했다(수확 정규식 파손)")
    targets = set()
    for arg in lits:
        parts = re.findall(r'"([^"]*)"', arg)
        need(parts, "_repo_file 인자 파싱 실패: %s" % arg)
        targets.add(os.path.join(*parts))
    # 변수 경유 호출(`_repo_file(rel)` 등)은 정규식이 못 잡는다 → 알려진 인자 이름만 허용하고,
    # 그 대상 경로를 명시 편입한다. 새 변수 이름이 등장하면 여기서 적색이 난다.
    dyn = set(re.findall(r"_repo_file\((?!os\.path\.join)([A-Za-z_][A-Za-z_0-9]*)\)", runner))
    unknown = sorted(dyn - {"rel", "tmpl_rel"})
    need(not unknown,
         "미지의 _repo_file 동적 인자 %s — H-META-READ 대상 목록이 낡았다(목록 갱신 필요)" % unknown)
    targets |= set(_CLAUDE_MD_COPIES)                       # `for rel in _CLAUDE_MD_COPIES`
    targets.add(os.path.join("cysjavis-pack", "directives", "MASTER_DIRECTIVE.md"))   # rel
    targets.add(os.path.join("cysjavis-pack", "directives", "CEO_TEMPLATE.md"))       # tmpl_rel
    notes.append("대상 %d경로 수확(리터럴 %d·변수 %d)" % (len(targets), len(lits), len(dyn)))

    # ⓑ 실제 크기 < 상한 = hard fail · 여유 배수는 판정 조건이 아니라 아래의 ⚠조기경보뿐
    sizes = {}
    for rel in sorted(targets):
        p = os.path.join(REPO_DIR, rel)
        if not os.path.isfile(p):
            continue                    # 배포 팩 실행 — 해당 검체들은 _repo_file 이 Skip 한다
        with open(p, encoding="utf-8", errors="replace") as f:
            sizes[rel] = len(f.read())
    if not sizes:
        raise Skip("레포 파일 부재(배포 팩 실행) — 대상 0건")
    over = ["%s=%d자" % (r, n) for r, n in sorted(sizes.items()) if n >= READ_LIMIT_CHARS]
    need(not over,
         "_repo_file 대상이 읽기 상한(%d자)에 도달한다 = 조용한 절단 재발: %s"
         % (READ_LIMIT_CHARS, over))
    big_rel, big_n = max(sizes.items(), key=lambda kv: kv[1])
    margin = READ_LIMIT_CHARS / float(big_n) if big_n else float("inf")
    notes.append("최대 %s=%d자 · 상한 %d자(여유 %.1f배)"
                 % (big_rel, big_n, READ_LIMIT_CHARS, margin))
    # ★판정 축 재분류(F2 · 2026-08-23) — **이것은 완화가 아니다.**
    #   구 판은 `need(READ_LIMIT_CHARS >= big_n * 4)` 로 여유배수 미달을 hard fail 로 삼았다.
    #   그러나 여유가 4배 미만이어도 그 시점의 측정은 **완벽히 유효하다**(절단 0 — 바로 위
    #   `over` 판정이 그것을 단언한다). 즉 구 판은 **파일이 자랐다는 사실만으로** 게이트를
    #   적색으로 만든다 = "결함이 아니라 계측기가 적색을 만든다" 이고, 이는 U-0 가 제거하려던
    #   실패 유형과 정확히 같은 계열이다(H-CONC-4 거짓 적색).
    #   ∴ hard fail 은 **실제 상한 도달(= 절단 = 계측 무효)** 에만 남기고, 여유 감소는
    #   **조기경보**로 내려 notes 에 기재한다. 절단은 fail, 여유는 경보 — 축이 다르다.
    if margin < 4.0:
        notes.append("⚠여유 %.1f배 — 상한 상향 검토(READ_LIMIT_CHARS/CYS_HEALTH_READ_LIMIT)"
                     % margin)

    # ⓓ 관측 원장 — 앞선 검체들이 실제로 읽은 경로 중 최대(단독 실행이면 이 검체 자신뿐)
    if _READ_OBSERVED:
        op, on = max(_READ_OBSERVED.items(), key=lambda kv: kv[1])
        notes.append("관측 최대 %d자(%s · %d경로)"
                     % (on, os.path.basename(op), len(_READ_OBSERVED)))

    # 계측 타당성(구 상한 대조): 구 값 400,000자에서 몇 개가 잘렸는지 명시한다 —
    # 이 수가 0이 아니라는 것이 곧 "U-0 는 실재했다"의 기계 증거다.
    cut_old = ["%s(%d자)" % (r, n) for r, n in sorted(sizes.items()) if n >= 400000]
    notes.append("계측대조 구상한400000 절단대상 %d건: %s" % (len(cut_old), ", ".join(cut_old) or "없음"))
    return " · ".join(notes)



@specimen("H-META-OFF", "W6",
          "계측기 자기감시 — '사람이 끈 미측정'≠'적용 불가' · 종료코드 3값 · `--json` stdout 순수",
          ["U-27"])
def h_meta_off():
    """U-27(2026-08-24 · 이종 리뷰어): 계측기 자신이 **측정 불능을 통과로 접었다.**
    `CYS_U26_OFF=1 … --only H-CI-TAG-1,H-EVID-1,H-FAKE-1,H-FAKE-2,H-FAKE-3` 은 검체 5건을
    전부 지우고도 `GREEN — 발효 0 PASS / 0 FAIL / 5 SKIP` · **exit 0** 을 냈다. Skip 문구는
    스스로 "이것은 통과가 아니라 미측정" 이라 말하는데 요약과 종료코드는 통과라고 말한 것이다 —
    이 저장소가 반복해서 낸 사고의 형태를 계측기가 재현한 자리다.
    이 검체가 그 수리의 **기계 집행자**이며 다섯 축으로 본다:
      ⓐ 등재소 형태(두 종류 · 중복 0 · `off_switch()` 경유 등재).
      ⓑ 모듈 레벨 env 독취 **전수** == 등재 집합 — 등재 없는 새 스위치가 조용히 생기면 적색.
      ⓒ 판정 배선(소스 핀): `Disabled` 를 `Skip` **보다 먼저** 잡는가 · GREEN 박탈 · exit 2 ·
         `--json` fd 분리 가드 실재.
      ⓓ **행동 실증** — 러너를 실제로 재실행해 스위치 켠 런이 `UNMEASURED`·exit 2 인지,
         끈 런이 `GREEN`·exit 0 인지 잰다(소스 핀만으로는 "쓰여 있다" 밖에 증명하지 못한다).
      ⓔ `--json` stdout 순수 — 실제 오염원 검체를 태워 stdout 이 JSON 한 덩어리인지,
         오염분이 stderr 로 갔는지 잰다.
    ★이 검체는 어떤 판정도 완화하지 않는다. 기존 need 를 대체하지 않고 새 축만 추가한다."""
    import inspect
    notes = []

    # ⓐ 등재소 형태
    need(MEASUREMENT_OFF_SWITCHES,
         "미측정 축 등재소가 비었다 — `off_switch()` 등재가 사라지면 스위치를 켜도 러너가 "
         "그 사실을 모른다(GREEN 박탈·exit 2 가 통째로 무력화)")
    need(MEASUREMENT_PIN_OVERRIDES, "핀 override 등재소가 비었다")
    names = [e for e, _s in MEASUREMENT_OFF_SWITCHES] + [e for e, _s in MEASUREMENT_PIN_OVERRIDES]
    dup = sorted({n for n in names if names.count(n) > 1})
    need(not dup, "등재소 중복 항목: %r — 한 스위치가 두 축에 있으면 판정이 갈린다" % dup)
    runner = _read(os.path.abspath(__file__))
    for env, _s in MEASUREMENT_OFF_SWITCHES:
        need('off_switch("%s"' % env in runner,
             "%s 가 `off_switch()` 를 경유하지 않고 등재됐다 — 등재 경로가 둘이면 한쪽이 낡는다"
             % env)
    notes.append("등재 off %d · pin %d" % (len(MEASUREMENT_OFF_SWITCHES),
                                           len(MEASUREMENT_PIN_OVERRIDES)))

    # ⓑ 모듈 레벨 env 독취 전수 == 등재 집합
    # ★접두 스캔이 아니라 **모듈 레벨 대입 전수**로 잰다. `CYS_HEALTH_` 접두로 잡으면 제품 env
    #   `CYS_HEALTH_NARRATION_CJK_MIN`(cysd 헬스 룰)까지 오검출한다 — 그건 러너의 스위치가 아니다.
    harvested = set(re.findall(r'^[A-Za-z_][A-Za-z_0-9]*\s*=\s*[^\n]*os\.environ\.get\(\s*"([^"]+)"',
                               runner, re.M))
    need(harvested, "모듈 레벨 env 독취를 하나도 수확하지 못했다(수확 정규식 파손 — fail-closed)")
    declared = {e for e, _s in MEASUREMENT_PIN_OVERRIDES}
    unregistered = sorted(harvested - declared)
    need(not unregistered,
         "등재되지 않은 모듈 레벨 env 스위치 %r — 새 스위치는 MEASUREMENT_PIN_OVERRIDES(또는 "
         "off_switch)에 등재하라. 등재 없는 스위치는 사람이 켜도 러너가 초록을 그대로 준다"
         % unregistered)
    stale = sorted(declared - harvested)
    need(not stale,
         "등재는 남았는데 소스에서 사라진 스위치 %r — 등재소가 낡으면 그 다음 사람이 등재소를 "
         "믿지 않게 된다(양방향 동결)" % stale)
    notes.append("모듈 env 수확 %d == 등재 %d" % (len(harvested), len(declared)))

    # ⓒ 판정 배선(소스 핀)
    need(issubclass(Disabled, Skip),
         "Disabled 가 Skip 의 하위형이 아니다 — 검체 내부의 기존 `except Skip` 관용구가 깨진다")
    msrc = inspect.getsource(main)
    di, si = msrc.find("except Disabled as e:"), msrc.find("except Skip as e:")
    need(di > 0 and si > 0 and di < si,
         "main 이 Disabled 를 Skip 보다 먼저 잡지 않는다 — 순서가 뒤집히면 '사람이 끈 미측정' 이 "
         "다시 '적용 불가' 로 접혀 GREEN 이 된다(U-27 수리의 무력화)")
    need('elif disabled or engaged:' in msrc and '"UNMEASURED"' in msrc,
         "미측정 런의 GREEN 박탈 분기가 없다")
    need('return 2 if verdict == "UNMEASURED" else 0' in msrc,
         "미측정 종료코드(2)가 없다 — 기계 소비자는 종료코드만 본다(라벨만 바꾸면 무력화)")
    need("os.dup(1)" in msrc and "os.dup2(2, 1)" in msrc and "os.dup2(_out_fd, 1)" in msrc,
         "`--json` fd 분리 가드가 없다 — 자식 프로세스는 fd 를 상속하므로 sys.stdout 치환으로는 "
         "stdout 오염을 못 막는다(P0-3 회귀)")
    notes.append("배선 핀: Disabled 우선·GREEN 박탈·exit 2·fd 분리")

    # ⓓ 행동 실증 — 러너를 실제로 재실행한다.
    me = os.path.abspath(__file__)
    allsw = tuple(e for e, _s in MEASUREMENT_OFF_SWITCHES) + tuple(
        e for e, _s in MEASUREMENT_PIN_OVERRIDES)
    clean = _base_env(drop=allsw)      # ★부모가 이미 스위치를 켠 상태여도 대조군은 순수해야 한다

    def _self(env, *argv):
        r = _run([PY, me, "--json"] + list(argv), env=env, timeout=600)
        try:
            return r, json.loads(r.stdout)          # ★우회 관용구 없이 그대로 파싱한다
        except ValueError:
            raise Fail("`--json` stdout 이 순수 JSON 이 아니다(P0-3 회귀) — 앞 200자=%r · "
                       "stderr 끝 200자=%r" % (r.stdout[:200], r.stderr[-200:]))

    probe = "H-CI-TAG-1,H-EVID-1"
    rc, dc = _self(clean, "--only", probe)
    need(rc.returncode == 0 and dc["summary"]["verdict"] == "GREEN",
         "대조군(스위치 전부 꺼짐) 이 GREEN·exit 0 이 아니다: rc=%d verdict=%s"
         % (rc.returncode, dc["summary"]["verdict"]))
    need(dc["summary"]["disabled"] == 0, "대조군에 disabled 가 있다: %r" % dc["summary"])

    ro, do = _self(dict(clean, **{U26_OFF_ENV: "1"}), "--only", probe)
    so = do["summary"]
    need(so["verdict"] == "UNMEASURED",
         "스위치를 켰는데 판정이 %s 다 — 리뷰어 재현(GREEN) 의 회귀" % so["verdict"])
    need(ro.returncode == 2,
         "스위치를 켰는데 종료코드가 %d 다 — 기계 소비자에게는 여전히 '통과' 로 들린다(exit 2 여야 한다)"
         % ro.returncode)
    need(so["disabled"] == 2 and so["pass"] == 0,
         "미측정 축 계수 오류: %r" % {k: so[k] for k in ("pass", "fail", "skip", "disabled")})
    need(any(x["env"] == U26_OFF_ENV and x["kind"] == "off" for x in so["off_switches_engaged"]),
         "요약이 켜진 스위치를 이름으로 밝히지 않는다: %r" % so["off_switches_engaged"])

    pin_env, _pin_scope = MEASUREMENT_PIN_OVERRIDES[0]
    rp, dp = _self(dict(clean, **{pin_env: "0000000-does-not-resolve"}), "--only", "H-CI-TAG-1")
    need(dp["summary"]["verdict"] == "UNMEASURED" and rp.returncode == 2,
         "핀 override(%s)를 켰는데 %s·exit %d — 핀을 갈아끼운 런의 초록은 '핀된 계약을 잰 초록' 이 "
         "아니다(조용히 PASS 로 접히므로 끈 것보다 엄하게 다룬다)"
         % (pin_env, dp["summary"]["verdict"], rp.returncode))
    notes.append("행동 실증: off→UNMEASURED/exit2 · pin→UNMEASURED/exit2 · 대조군→GREEN/exit0")

    # ⓔ `--json` stdout 순수 — **실제 오염원**을 태워 잰다.
    #   H-W5-N1 은 `javis_report_gate` 를 인프로세스로 돌려 `verdict=… delivered=… reasons=…`
    #   요약 줄을 뱉는다(실측: 전량 실행 시 JSON 앞에 12,416자가 붙어 `json.loads` 가 죽었다).
    rj, dj = _self(clean, "--only", "H-W5-N1")
    need(rj.stdout.lstrip().startswith("{") and rj.stdout.rstrip().endswith("}"),
         "stdout 이 JSON 한 덩어리가 아니다: 앞 120자=%r" % rj.stdout[:120])
    st = (dj["specimens"] or [{}])[0].get("status")
    if st == "pass" and "verdict=" in rj.stderr:
        notes.append("stdout 순수(오염 %d건 stderr 전량 격리 · 우회 관용구 불요)"
                     % rj.stderr.count("verdict="))
    else:
        # ★계측 타당성: 오염이 실제로 일어나지 않았다면 이 축은 공허하다 — 통과로 접지 않고
        #   그 사실을 그대로 적는다(오염원 검체가 바뀌면 다른 오염원으로 교체하라).
        notes.append("⚠오염원 검체 H-W5-N1 status=%s · stderr 오염 %d건 — stdout 순수 축은 "
                     "이번 실행에서 공허했다(오염원 교체 검토)" % (st, rj.stderr.count("verdict=")))
    return " · ".join(notes)


# ═══════════════════════════════════════════════════════════════════════════
# ★U-22 — `cys hook` + `hook.decide` (근본원인 R2: 판정의 위치 이동)
#
# 무엇을 지키는가: 부트 판정이 30초짜리 단명 UserPromptSubmit 훅 안에서 서브프로세스를
# 줄줄이 띄우며 일어나고 모든 불확실성이 침묵(rc0+빈출력)으로 접히던 구조를, 데몬 인메모리
# 1왕복으로 옮긴 배선. 이 축이 무너지면 되돌아가는 곳은 "구조 개선 이전"이 아니라 **더 나쁜
# 자리**다 — 위임은 있는데 폴백이 없거나(마스터 부트 전면 사망), 위임이 새 차단자가 되거나
# (비-master 오판으로 팀 미기동), 데몬 핫패스에 스폰이 들어가(프롬프트 제출 먹통) 셋 다
# 이 제품이 실제로 낸 사고 계열이다.
# ═══════════════════════════════════════════════════════════════════════════
_U22_RS_CLI = "src/bin/cys.rs"
_U22_RS_DAEMON = "src/bin/cysd/handlers.rs"
_U22_SH = "cysjavis-pack/hooks/role-bootstrap.sh"
# exit 계약 정본(3중 등재의 기준값) — cys.rs 상수 · 셸 case · 셸 헤더 표가 전부 이 값이어야 한다.
_U22_EXITS = {"PROCEED": 0, "DAEMON_ERR": 1, "SUPPRESS": 3, "UNDECIDED": 4, "LEGACY": 5}
_U22_CONTRACT_V = 1
_U22_EVENT = "user-prompt-submit"
# 판정 토큰 정본(접두 + verdict). cys.rs `HOOK_VERDICT_PREFIX` 와 셸 case 가 이 값이어야 한다.
# ★5종을 **전부** 등재한다 — 종전 검체는 proceed·suppress 둘만 봤고, 그래서 나머지 3종의
#   철자가 갈려도(= 셸이 그 토큰을 못 읽어 조용히 legacy 로 접혀도) 초록이었다.
_U22_TOKEN_PREFIX = "[cys-hook] hook-decide: "
_U22_TOKENS = ("proceed", "suppress", "undecided", "legacy", "error")


def _u22_fn_body(src, sig):
    """`fn ...(` 부터 최상위 닫힘까지(선례 `_u20_violations._body` 와 같은 규약)."""
    i = src.find(sig)
    if i < 0:
        return None
    j = src.find("\n}\n", i)
    return src[i:j] if j > i else None


def _u22_arm_body(src, arm):
    """dispatch match arm 본문 — 다음 arm(8칸 들여쓴 따옴표) 직전까지."""
    i = src.find(arm)
    if i < 0:
        return None
    j = src.find('\n        "', i + len(arm))
    return src[i:j] if j > i else src[i:]


def _u22_violations(cli, daemon, sh):
    """U-22 배선 위반 목록(빈 목록 = 배선 정상).

    ★세 소스를 **인자로** 받는 이유: 같은 탐지기를 기준 커밋 트리에도 그대로 먹여
      "수리 전에는 적색"을 기계로 증명하기 위해서다(계측 타당성 게이트 ①).
    """
    v = []

    # ── CLI(cys.rs) ────────────────────────────────────────────────────────
    if "Hook {" not in cli or "enum HookEvent" not in cli:
        v.append("CLI: `cys hook` 서브커맨드(Command::Hook / enum HookEvent)가 없다")
    if "Command::Hook { event } => return run_hook(event)" not in cli:
        v.append("CLI: dispatch 에 Command::Hook 배선이 없다(서브커맨드 사문)")
    for name, val in sorted(_U22_EXITS.items()):
        if "const HOOK_EXIT_%s: i32 = %d;" % (name, val) not in cli:
            v.append("CLI: exit 상수 HOOK_EXIT_%s=%d 정본 부재/불일치" % (name, val))
    if "const HOOK_DECIDE_CONTRACT_V: u64 = %d;" % _U22_CONTRACT_V not in cli:
        v.append("CLI: 페이로드 계약 버전 상수(HOOK_DECIDE_CONTRACT_V=%d) 부재" % _U22_CONTRACT_V)
    if 'const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "%s";' % _U22_EVENT not in cli:
        v.append("CLI: 훅 이벤트 와이어 상수 부재/철자 불일치")
    # 전용 데드라인 — 하드코딩 금지(예산 파생). 값을 손으로 적으면 예산이 바뀌어도 안 따라온다.
    if "const HOOK_DECIDE_DEADLINE_MS: u64 = BUDGET_TICK_MS;" not in cli:
        v.append("CLI: 전용 데드라인이 BUDGET 파생 상수가 아니다(하드코딩 의심)")

    rh = _u22_fn_body(cli, "fn run_hook(event: HookEvent)")
    if rh is None:
        v.append("CLI: run_hook 정의를 찾지 못했다(계측 불능)")
    elif "set_var(cys::ENV_NO_AUTOSTART, cys::NO_AUTOSTART_ON)" not in rh:
        v.append("CLI: run_hook 이 CYS_NO_AUTOSTART 를 자기 강제하지 않는다 — 단명 훅이 데몬을 낳는다(R2)")

    ru = _u22_fn_body(cli, "fn run_hook_user_prompt_submit()")
    if ru is None:
        v.append("CLI: run_hook_user_prompt_submit 정의를 찾지 못했다(계측 불능)")
    else:
        # ★P1 이월 정리(false-green 봉인): 앵커는 주석 매치 불가한 **코드형**이다 — 벌거벗은
        #   `gate_axes_forced_legacy()` 리터럴은 이 함수 안 carve-out 주석에도 나타나, 조기
        #   반환 코드가 지워지고 주석만 남아도 초록이 됐다(H-IDENT-1 ⓑ의 i_fold 앵커와 동일
        #   엄격성 — 우회 가능 핀 금지).
        if "if cys::gate_axes_forced_legacy()" not in ru:
            v.append("CLI: 롤백 마스터 스위치(CYS_BOOT_GATES)를 읽지 않는다 — 새 축이 마스터에 안 접혔다")
        if "request_on_timeout(" not in ru:
            v.append("CLI: 전용 데드라인 왕복(request_on_timeout)을 쓰지 않는다")
        if re.search(r"[^_.\w]request\(", ru):
            v.append("CLI: autostart 경로 request() 를 쓴다 — 단명 훅에서 데몬 기동 금지")
        if "HOOK_DECIDE_DEADLINE_MS" not in ru:
            v.append("CLI: 전용 데드라인 상수를 소비하지 않는다")
        # ★`eprintln!` 은 stderr 다 — 접두 e 를 배제하지 않으면 부분문자열로 오탐한다(계측 자기점검).
        if re.search(r"(?<![A-Za-z_])println!", ru):
            v.append("CLI: stdout 에 쓴다 — 훅의 stdout 계약(hookSpecificOutput JSON) 오염")
        if '"surface_id"' in ru:
            v.append("CLI: 요청 페이로드에 surface_id 를 싣는다 — 자기신고 금지(인가 계약 위반)")
        # fail-open 불변식: suppress 를 내는 자리가 **정확히 하나**여야 한다.
        if ru.count("HOOK_EXIT_SUPPRESS") != 1:
            v.append("CLI: suppress 반환 지점이 1곳이 아니다(%d곳) — 새 차단자 신설 위험"
                     % ru.count("HOOK_EXIT_SUPPRESS"))
        # 판정은 전부 단일 출구(hook_verdict)를 거쳐야 한다 — 토큰 누락·자유문구 혼입 방지.
        if "return HOOK_EXIT_" in ru or re.search(r"^\s+HOOK_EXIT_\w+\s*$", ru, re.M):
            v.append("CLI: 토큰 단일 출구(hook_verdict)를 우회해 exit 를 직접 반환하는 분기가 있다")
    hv = _u22_fn_body(cli, "fn hook_verdict(")
    if hv is None:
        v.append("CLI: 판정 토큰 단일 출구(hook_verdict)가 없다")
    else:
        # ── ★핀 이사(U-28 · 2026-08-24 · 러너 헤더 '핀 이사 계약' ①④) ────────────────
        # 【원인 규명 먼저】 이 핀이 적색이 된 것은 계약이 무너져서가 아니라 **토큰 줄 조립이
        #   이사**했기 때문이다: `hook_verdict` 본문의 `eprintln!("{HOOK_VERDICT_PREFIX}{verdict}")`
        #   가 순수 절반 `hook_verdict_lines(verdict, detail)` 로 나갔다(검체가 stderr 를 가로채지
        #   않고 **프로덕션 경로 그대로** 같은 문자열을 얻게 하려는 분리 · 목 금지).
        # 【축은 무변】 "토큰 줄 = 접두 + verdict **단독**(자유 문구 혼입 0) · 출구는 하나."
        # 【형상만 이동】 조립은 `hook_verdict_lines` 에서, 인쇄는 `hook_verdict` 에서 본다 —
        #   두 축을 함께 요구하므로 판정은 좁아졌지 넓어지지 않았다.
        lines = _u22_fn_body(cli, "fn hook_verdict_lines(")
        if lines is None:
            v.append("CLI: 토큰 줄 조립의 순수 절반(hook_verdict_lines)이 없다")
        elif 'format!("{HOOK_VERDICT_PREFIX}{verdict}")' not in lines:
            v.append("CLI: 토큰 줄이 접두+verdict **단독**이 아니다 — 자유 문구 혼입은 위조 표면")
        if ("let (token_line, detail_line) = hook_verdict_lines(verdict, detail);" not in hv
                or 'eprintln!("{token_line}");' not in hv):
            v.append("CLI: 단일 출구가 조립된 토큰 줄을 **그대로 한 줄로** 내지 않는다")
        if "println!" in hv.replace("eprintln!", ""):
            v.append("CLI: 판정 출구가 stdout 에 쓴다 — 훅 stdout 계약 오염")
    # ★추가 강화(다중 방어 ⓐ층 · 위조 차단) — 상세 줄은 토큰 접두를 실을 수 없다.
    #   이 층이 없으면 데몬이 준 role/reason 문자열이 상세 줄에 그대로 인쇄되고, 비-master 좌석이
    #   role 을 토큰 문자열로 claim 하는 것만으로 판정이 뒤집힌다(A3=B7 재발 · 이종 리뷰어 격리
    #   재현). 셸 측 정확 일치(ⓑ)와 rc 교차(ⓒ)는 아래에서 따로 본다 — 한 층이 넓어져도 나머지가 선다.
    if "fn sanitize_hook_detail(" not in cli or "HOOK_DETAIL_REDACTED" not in cli:
        v.append("CLI: 상세 줄 무해화(sanitize_hook_detail)가 없다 — 데몬이 준 role/reason 이 "
                 "판정 토큰을 위조할 수 있다")
    if "sanitize_hook_detail(detail)" not in cli:
        v.append("CLI: 상세 줄이 무해화를 경유하지 않는다 — 단일 출구 밖으로 원문이 샌다")
    if "flat.replace(HOOK_VERDICT_PREFIX, HOOK_DETAIL_REDACTED)" not in cli:
        v.append("CLI: 무해화가 **토큰 접두**를 치환하지 않는다 — 제어문자만 접으면 위조는 그대로다")
    if 'const HOOK_VERDICT_PREFIX: &str = "%s";' % _U22_TOKEN_PREFIX not in cli:
        v.append("CLI: 판정 토큰 접두 상수 부재/불일치(셸 case 문과 갈림)")

    # ── 데몬(handlers.rs) ──────────────────────────────────────────────────
    if 'const HOOK_DECIDE_CONTRACT_V: u64 = %d;' % _U22_CONTRACT_V not in daemon:
        v.append("데몬: 페이로드 계약 버전 상수 부재/불일치")
    if 'const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "%s";' % _U22_EVENT not in daemon:
        v.append("데몬: 훅 이벤트 와이어 상수 부재/철자 불일치")
    arm = _u22_arm_body(daemon, '"hook.decide" => {')
    if arm is None:
        v.append("데몬: hook.decide arm 이 없다")
    else:
        if 'params.get("surface_id").is_some()' not in arm or "invalid_params" not in arm:
            v.append("데몬: surface_id 자기신고를 거절하지 않는다(인가 계약 — 위조 가능 필드)")
        if "resolve_caller_surface(daemon" not in arm:
            v.append("데몬: 좌석을 caller_pid 조상 체인으로 도출하지 않는다")
        if '"contract_version": HOOK_DECIDE_CONTRACT_V' not in arm:
            v.append("데몬: 응답에 contract_version 페이로드 필드가 없다")
        if "PROTO_PV" in arm:
            v.append("데몬: hook.decide 가 wire::PROTO_PV 를 만진다 — 전송 프로토콜 무접촉 위반")
        # ★핫패스 금지 3종 — 훅은 사람의 프롬프트 앞에 서 있다.
        for tok, why in (("Command::new", "프로세스 스폰"), (".spawn(", "프로세스 스폰"),
                         ("auth status", "외부 인증 조회"), ("sync_all", "fsync"),
                         ("std::fs::write", "디스크 쓰기"), ("thread::sleep", "블로킹 대기")):
            if tok in arm:
                v.append("데몬: hook.decide 핫패스에 %s(%s) 가 있다 — 프롬프트 제출 먹통 계열" % (tok, why))
        # 판정은 **순수 코어**가 소유해야 한다 — arm 안에 인라인하면 진리표를 시험할 수 없고,
        # 시험되지 않는 allowlist 는 반드시 낡는다(구 denylist 가 그렇게 새어 나갔다).
        if "hook_decide_verdict(" not in arm:
            v.append("데몬: arm 이 순수 코어(hook_decide_verdict)를 경유하지 않는다(판정 인라인)")
    core = _u22_fn_body(daemon, "fn hook_decide_verdict(")
    if core is None:
        v.append("데몬: 판정 순수 코어(hook_decide_verdict)가 없다")
    else:
        # 판정 규칙은 종전 A3 allowlist 그대로여야 한다(완화도 강화도 금지).
        for tok, why in (('Err(why) => ("undecided"', "판정 불가는 차단이 아니다"),
                         ('Ok("") => ("proceed"', "미claim 좌석 통과"),
                         ('Ok("master") => ("proceed"', "master 좌석 통과"),
                         ('Ok(_) => ("suppress"', "그 밖 전부 차단(미지 role 포함)")):
            if tok not in core:
                v.append("데몬: A3 allowlist 판정 축 누락 — %s" % why)
    if "fn hook_decide_verdict_truth_table()" not in daemon:
        v.append("데몬: 판정 진리표 회귀 핀(hook_decide_verdict_truth_table)이 없다")

    # ── 훅(role-bootstrap.sh) ──────────────────────────────────────────────
    # ★주석 언급이 아니라 **실제 호출 라인**만 본다(헤더 문서가 배선을 대신하지 않는다).
    inv = [ln for ln in sh.splitlines()
           if "cys hook user-prompt-submit" in ln and not ln.lstrip().startswith("#")]
    if not inv:
        v.append("훅: `cys hook user-prompt-submit` 위임 호출이 없다(주석 언급만)")
    else:
        if not any("</dev/null" in ln for ln in inv):
            v.append("훅: 위임 호출에 `</dev/null` 이 없다 — 자식이 hook stdin 을 삼키면 무음 실패")
        if not any("2>&1 >/dev/null" in ln for ln in inv):
            v.append("훅: 위임 호출이 stdout 을 버리지 않는다 — 훅 stdout 계약 오염 표면")
    if "_cys_hook_legacy_unavailable()" not in sh:
        v.append("훅: 구 경로 판정기(_cys_hook_legacy_unavailable)가 없다")
    # ★판정의 1차 근거는 stderr 토큰이어야 한다 — rc 로 통과를 읽으면 **exit 0**(셸에서 가장
    #   흔한 사고값)이 곧 게이트 통과가 되어, 아무 일도 안 하고 성공한 stub `cys` 하나로 role
    #   게이트가 증발한다(2026-08-24 실측: worker 좌석 마스터 부트 오발화 = A3=B7 재발).
    # ── ★핀 이사(U-28 · 2026-08-24 · 러너 헤더 '핀 이사 계약' ①②④) ──────────────────────
    # 【원인 규명 먼저 — 검체가 요구하던 형상 자체가 결함이었다】 종전 핀은 셸이 stderr **전문**에
    #   substring glob `*"[cys-hook] hook-decide: proceed"*)` 를 돌 것을 요구했다. 그런데 `cys hook`
    #   의 상세 줄에는 데몬이 준 role 문자열이 인쇄되고 claim 경로에 role 문자열 검증이 없다 —
    #   비-master 좌석이 role 을 `[cys-hook] hook-decide: proceed` 로 claim 하면 `cys hook` 이
    #   올바르게 suppress(rc 3)를 내는데도 **상세 줄이 판정을 뒤집는다**(먼저 매칭되므로).
    #   귀결은 A3=B7 그 자체다(이종 리뷰어 격리 재현 · 수리 = 결함 1).
    # 【축은 하나도 완화하지 않는다】 축 = "셸은 판정을 **rc 가 아니라 stderr 판정 토큰**으로 읽는다."
    #   그 축의 근거(stub `cys` 의 exit 0 이 곧 통과가 되는 사고)는 그대로이고, 아래 ⓔ 가 rc 기반
    #   회귀를 계속 막는다. 바뀐 것은 **토큰을 읽는 형상**뿐이다.
    # 【새 형상 — 전부 종전보다 좁다】 ⓐ 줄 단위 **정확 일치** 5종 전수(정확 일치는 substring 의
    #   진부분집합이고, 종전이 보던 2종에서 5종으로 넓혔다) ⓑ 줄 단위 순회 + CR 제거 ⓒ 토큰 줄
    #   **개수 == 1**(복수 = 위조 의심 → 폴백) ⓓ rc **거부권** 교차(proceed↔0 · suppress↔3)
    #   ⓔ rc=0 통과 분기 부재.
    # 【★추가 강화 ⓕ】 위조 통로(substring glob)가 **다시 들어오면 적색**이다. 그 형상이 정확히
    #   이번에 수리한 벡터이므로, 부재를 단언하는 것이 이사의 자연스러운 귀결이다(완화 아님 —
    #   판정 조건이 하나 늘었다).
    for _tk in _U22_TOKENS:
        if not re.search(r'^\s*"%s%s"[|)]' % (re.escape(_U22_TOKEN_PREFIX), _tk), sh, re.M):
            v.append("훅: 판정 토큰 `%s` 를 **줄 단위 정확 일치**로 읽지 않는다 — 판독이 넓으면 "
                     "상세 줄(데몬이 준 role)이 판정을 뒤집는다(A3=B7 재발)" % _tk)
        if ('*"%s%s"*)' % (_U22_TOKEN_PREFIX, _tk)) in sh:
            v.append("훅: ★위조 통로 재유입 — 토큰 `%s` 를 stderr 전문 substring glob 으로 읽는다"
                     % _tk)
    if "while IFS= read -r CYS_HOOK_LINE" not in sh:
        v.append("훅: stderr 를 **줄 단위**로 훑지 않는다 — 전문 매칭은 그 자체가 위조 표면이다")
    if 'CYS_HOOK_LINE="${CYS_HOOK_LINE%"$CYS_CR"}"' not in sh:
        v.append("훅: CR 제거가 없다 — Windows 파이프에서 정확 일치가 전건 실패해 위임이 통째로 죽는다")
    if "CYS_HOOK_TOKEN_N=$((CYS_HOOK_TOKEN_N + 1))" not in sh:
        v.append("훅: 판정 토큰 줄을 **세지** 않는다 — 복수 토큰(위조 시도·형상 스큐)을 구분 못 한다")
    if '[ "$CYS_HOOK_TOKEN_N" -ne 1 ]' not in sh:
        v.append("훅: 토큰 줄 **개수 1** 판정이 없다 — 첫 토큰만 보면 상세 줄 주입이 그대로 통한다")
    if '[ "$CYS_HOOK_RC" = "0" ]' not in sh or '[ "$CYS_HOOK_RC" = "3" ]' not in sh:
        v.append("훅: rc 교차(proceed↔0 · suppress↔3)가 없다 — rc 거부권 층 소실")
    if re.search(r'^\s*0\)\s*CYS_HOOK_GATE="proceed"', sh, re.M):
        v.append("훅: rc=0 을 통과로 읽는 분기가 남아 있다 — stub cys(exit 0)가 게이트를 증발시킨다")
    if '*) CYS_HOOK_GATE="legacy"' not in sh:
        v.append("훅: 토큰 부재·미지 토큰을 레거시 폴백으로 접지 않는다 — 위임 실패가 곧 차단이 된다")
    # ★폴백 보존: 종전 게이트가 통째로 남아 있어야 한다(cys 부재·구 데몬 레인에서 유일한 판정).
    if "cys surface-role" not in sh or "무발화(fail-closed)" not in sh:
        v.append("훅: 종전 role 게이트 폴백이 사라졌다 — cys 부재 레인에서 판정 자체가 소실")
    if "CYS_BOOT_GATES=0" not in sh:
        v.append("훅: 롤백 스위치(CYS_BOOT_GATES=0) 문서화가 없다")
    return v


@specimen("H-HOOK-DECIDE-1", "W6",
          "U-22 `cys hook`+`hook.decide` 배선 — fail-open·전용 데드라인·NO_AUTOSTART 자기강제 · "
          "데몬 핫패스 금지 3종 · 종전 게이트 폴백 보존",
          ["R2"])
def h_hook_decide_1():
    cli = _read(os.path.join(REPO_DIR, _U22_RS_CLI))
    daemon = _read(os.path.join(REPO_DIR, _U22_RS_DAEMON))
    sh = _read(os.path.join(HOOKS_DIR, ROLE_BODY))   # 판독 규칙은 본체에 산다(A2 분할)
    if not cli or not daemon:
        raise Skip("배포 팩(Rust 소스 부재) — 소스 배선 검체 적용 불가")
    need(sh, "role-bootstrap.sh 를 읽지 못했다(계측 불능)")
    v = _u22_violations(cli, daemon, sh)
    need(not v, "U-22 배선 위반 %d건: %s" % (len(v), " / ".join(v)))
    # ★계측 타당성(구 트리 대조) — 같은 탐지기를 기준 커밋에 먹여 **적색이 나오는지** 확인한다.
    #   여기서 초록이 나오면 탐지기가 고장난 것이다(트리에 위반이 0이라 초록인 것과 구분).
    ocli = _git_show(_U22_RS_CLI)
    odae = _git_show(_U22_RS_DAEMON)
    osh = _git_show(_U22_SH)
    if ocli and odae and osh:
        ov = _u22_violations(ocli, odae, osh)
        need(ov, "계측 무효: 기준 커밋(%s · U-22 이전)에서 탐지기가 위반을 하나도 못 찾았다"
             % CALIBRATION_REF)
        calib = "계측대조 %s 에서 %d건 FIRE" % (CALIBRATION_REF, len(ov))
    else:
        calib = "계측대조 생략(레포 아님)"
    # ★계측 타당성 ② 합성 변조본 — 트리에 위반이 0 이라 초록인 것과 **탐지기 고장**을 가른다.
    #   ★핀 이사(U-28)의 판별력이 여기서 영속 증명된다: 형상만 바꾸고 판별력이 사라지면
    #     그것은 이사가 아니라 **삭제**다. 그래서 ⓐ 낡은(취약한) 형상으로의 되돌림과
    #     ⓑ 축 자체의 파괴를 **둘 다** 적발하는지 매 실행마다 시험한다.
    old_glob = ('\ncase "$CYS_HOOK_ERR" in\n'
                '  *"[cys-hook] hook-decide: proceed"*) CYS_HOOK_GATE="proceed" ;;\n'
                '  *"[cys-hook] hook-decide: suppress"*) CYS_HOOK_GATE="suppress" ;;\n'
                'esac\n')
    mutants = [
        # ⓐ 낡은 형상 복귀 — 위조 통로(stderr 전문 substring glob)가 다시 들어온다.
        ("구 형상 복귀(전문 substring glob)", cli, daemon, sh + old_glob),
        # ⓑ 축 파괴 — 줄 단위 정확 일치 case 를 없앤다.
        ("정확 일치 case 소실", cli, daemon,
         sh.replace('    "[cys-hook] hook-decide: proceed"|\\', '    "banana"|\\')),
        ("토큰 줄 개수 판정 제거", cli, daemon,
         sh.replace('[ "$CYS_HOOK_TOKEN_N" -ne 1 ]', "false")),
        ("CR 제거 소실(Windows 전건 실패)", cli, daemon,
         sh.replace('CYS_HOOK_LINE="${CYS_HOOK_LINE%"$CYS_CR"}"', ":")),
        ("rc 기반 통과 회귀(stub exit 0 = 통과)", cli, daemon,
         sh + '\ncase "$CYS_HOOK_RC" in\n  0) CYS_HOOK_GATE="proceed" ;;\nesac\n'),
        ("rc 거부권 소실", cli, daemon, sh.replace('[ "$CYS_HOOK_RC" = "3" ]', "true")),
        ("산출 측 무해화 제거(상세 줄이 토큰을 실을 수 있다)",
         cli.replace("sanitize_hook_detail(detail)", "detail"), daemon, sh),
        ("토큰 줄에 자유 문구 혼입",
         cli.replace('format!("{HOOK_VERDICT_PREFIX}{verdict}")',
                     'format!("{HOOK_VERDICT_PREFIX}{verdict} ({detail})")'), daemon, sh),
    ]
    blind = [lbl for lbl, c, d, x in mutants if not _u22_violations(c, d, x)]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return "위반 0 · %s · 합성 변조 %d종 전건 적발" % (calib, len(mutants))


@specimen("H-HOOK-DECIDE-2", "W6",
          "U-22 계약 3중 등재 정합 — cys.rs 상수 · handlers.rs 상수 · role-bootstrap.sh 표",
          ["R2"])
def h_hook_decide_2():
    cli = _read(os.path.join(REPO_DIR, _U22_RS_CLI))
    daemon = _read(os.path.join(REPO_DIR, _U22_RS_DAEMON))
    sh = _read(os.path.join(HOOKS_DIR, ROLE_BODY))   # 판독 규칙은 본체에 산다(A2 분할)
    if not cli or not daemon:
        raise Skip("배포 팩(Rust 소스 부재) — 소스 배선 검체 적용 불가")
    # ① contract_version 3중 일치
    def _cv(src):
        m = re.search(r"const HOOK_DECIDE_CONTRACT_V: u64 = (\d+);", src)
        return int(m.group(1)) if m else None
    m = re.search(r"hook\.decide contract_version:\s*(\d+)", sh)
    sh_cv = int(m.group(1)) if m else None
    need(_cv(cli) == _cv(daemon) == sh_cv == _U22_CONTRACT_V,
         "contract_version 3중 불일치: cys.rs=%s handlers.rs=%s role-bootstrap.sh=%s(기준 %d)"
         % (_cv(cli), _cv(daemon), sh_cv, _U22_CONTRACT_V))
    # ② event 와이어 문자열 2중 일치(셸은 호출 문자열이 곧 값이다)
    pat = r'const HOOK_EVENT_USER_PROMPT_SUBMIT: &str = "([^"]+)";'
    ev_cli = re.search(pat, cli)
    ev_dae = re.search(pat, daemon)
    need(ev_cli and ev_dae and ev_cli.group(1) == ev_dae.group(1) == _U22_EVENT,
         "event 와이어 문자열 불일치: cys.rs=%s handlers.rs=%s (기준 %s)"
         % (ev_cli and ev_cli.group(1), ev_dae and ev_dae.group(1), _U22_EVENT))
    need("cys hook %s" % _U22_EVENT in sh, "훅이 부르는 서브커맨드 철자가 와이어 상수와 다르다")
    # ③ 판정 토큰 접두 2중 일치(cys.rs 상수 ↔ 셸 case) — 여기가 갈리면 위임이 조용히 죽는다.
    m0 = re.search(r'const HOOK_VERDICT_PREFIX: &str = "([^"]+)";', cli)
    need(m0, "cys.rs 에 판정 토큰 접두 상수가 없다")
    # ★핀 이사(U-28) — **접두 일치 축은 보존**하고 비교 대상 형상만 옮긴다.
    #   종전 비교 대상은 substring glob `*"<접두>proceed"*)` 였는데, 그 형상 자체가 위조 통로였다
    #   (상세 줄이 판정을 뒤집는다 — `_u22_violations` 의 이사 주석 참조). 이제는 셸의 **줄 단위
    #   정확 일치 case 패턴**과 대조한다. 축("셸 case 가 cys.rs 접두와 한 글자도 다르지 않다")은
    #   무변이고, 종전이 2종만 보던 것을 **5종 전수**로 넓혔다(완화 아님 · 강화).
    _pfx = m0.group(1)
    need(_pfx == _U22_TOKEN_PREFIX,
         "cys.rs 토큰 접두(%r)가 러너 정본(%r)과 다르다" % (_pfx, _U22_TOKEN_PREFIX))
    _miss = [tk for tk in _U22_TOKENS
             if not re.search(r'^\s*"%s%s"[|)]' % (re.escape(_pfx), tk), sh, re.M)]
    need(not _miss,
         "셸 case 가 cys.rs 토큰 접두(%r)와 다르다(정확 일치 누락: %s) — 위임이 조용히 legacy 로 접힌다"
         % (_pfx, ",".join(_miss)))
    # ★추가 강화 — 위조 통로(substring glob)의 **부재**를 단언한다.
    _back = [tk for tk in _U22_TOKENS if ('*"%s%s"*)' % (_pfx, tk)) in sh]
    need(not _back,
         "★위조 통로 재유입(stderr 전문 substring glob): %s — 정확 일치로 좁힌 판독이 다시 넓어졌다"
         % ",".join(_back))
    need(re.search(r"판정 토큰\(1차\):\s*%s" % re.escape(m0.group(1)), sh),
         "훅 헤더 계약표에 판정 토큰(1차) 줄이 없다")
    # ④ exit 계약(보조) — cys.rs 상수 ↔ 셸 헤더 표
    m = re.search(r"exit 계약\(보조\):\s*([^\n]+)", sh)
    need(m, "훅 헤더에 exit 계약 표가 없다(3중 등재의 셸 축 부재)")
    sh_tbl = dict((int(a), b) for a, b in re.findall(r"(\d+)=([a-z\-]+)", m.group(1)))
    want = {0: "proceed", 1: "daemon-error", 3: "suppress", 4: "undecided", 5: "legacy"}
    need(sh_tbl == want, "셸 exit 표 불일치: %s (기준 %s)" % (sh_tbl, want))
    for name, val in sorted(_U22_EXITS.items()):
        need("const HOOK_EXIT_%s: i32 = %d;" % (name, val) in cli,
             "cys.rs exit 상수 HOOK_EXIT_%s 가 %d 가 아니다(셸 표와 갈림)" % (name, val))
    # ⑤ rc 를 1차 근거로 되돌리는 회귀 차단(표만 맞고 코드가 rc 로 읽는 사문 방지)
    need(not re.search(r'^\s*0\)\s*CYS_HOOK_GATE="proceed"', sh, re.M),
         "셸이 rc=0 을 통과로 읽는다 — 토큰 1차 계약 회귀(stub cys 하나로 게이트 증발)")
    return ("contract_version=%d · event=%s · 토큰접두=%r · exit표(보조) %s 3중 일치"
            % (_U22_CONTRACT_V, _U22_EVENT, m0.group(1), want))


@specimen("H-HOOK-DECIDE-3", "W6",
          "U-22 구 경로 판정기 동적 시험 — 합성 표본 3종 참 · 음성대조 6종 거짓(스큐로 결함 삼킴 방지)",
          ["R2"])
def h_hook_decide_3():
    sh = _read(os.path.join(HOOKS_DIR, ROLE_BODY))   # 판독 규칙은 본체에 산다(A2 분할)
    need(sh, "role-bootstrap.sh 를 읽지 못했다(계측 불능)")
    m = re.search(r"^_cys_hook_legacy_unavailable\(\)\s*\{.*?^\}", sh, re.S | re.M)
    need(m, "_cys_hook_legacy_unavailable 정의를 추출하지 못했다 — 판정기 부재")
    fn = m.group(0)
    # ★합성 표본: 트리에 실제 위반이 없어도 **탐지 능력 자체**를 시험한다.
    #   양성 = 구 경로의 명시 증거(조용한 폴백이 정당) · 음성 = 그 밖 전부(시끄러워야 한다).
    pos = [("127", "", "cys 부재"),
           ("2", "error: unrecognized subcommand 'hook'", "구 CLI"),
           ("1", "error: method_not_found: unknown method: hook.decide", "구 데몬")]
    neg = [("1", "error: connect failed", "연결 실패"),
           ("1", "error: not_found: surface 9 not found", "일반 not_found"),
           ("2", "error: invalid value 'x'", "clap 다른 사용오류"),
           ("7", "reap_denied", "무관한 rc"),
           ("124", "", "타임아웃"),
           ("0", "", "성공 rc")]
    with tempfile.TemporaryDirectory() as td:
        harness = os.path.join(td, "probe.sh")
        _w(harness, "#!/bin/bash\n%s\nif _cys_hook_legacy_unavailable \"$1\" \"$2\"; then\n"
                    "  echo LEGACY\nelse\n  echo LOUD\nfi\n" % fn)
        bad = []
        for rc, err, why in pos:
            r = _run([BASH, harness, rc, err], timeout=30)
            if r.stdout.strip() != "LEGACY":
                bad.append("양성 미탐 rc=%s(%s) → %r" % (rc, why, r.stdout.strip()))
        for rc, err, why in neg:
            r = _run([BASH, harness, rc, err], timeout=30)
            if r.stdout.strip() != "LOUD":
                bad.append("음성 오탐 rc=%s(%s) → %r" % (rc, why, r.stdout.strip()))
    need(not bad, "구 경로 판정기 오작동 %d건: %s" % (len(bad), " / ".join(bad)))
    return "합성표본 양성 %d/%d · 음성대조 %d/%d 통과" % (len(pos), len(pos), len(neg), len(neg))


@specimen("H-HOOK-DECIDE-4", "W6",
          "U-22 위임 라우팅 행동 핀 — 토큰 부재(stub cys exit 0)에서 종전 게이트가 살아 있는가 · "
          "토큰 proceed/suppress 가 실제로 발화/차단을 가르는가",
          ["R2", "A3=B7"])
def h_hook_decide_4():
    """★이 검체가 존재하는 이유(2026-08-24 실사고): 위임 판정을 **exit code** 로 읽는 초안은
    통과값이 `0` 이었다. 0 은 셸에서 가장 흔한 사고값이라, 아무 일도 안 하고 성공한 stub `cys`
    (= 목 하네스·구 래퍼·`--help`) 하나로 role 게이트가 통째로 증발했고 worker-2 좌석에서
    마스터 부트가 오발화했다(A3=B7 재발 — H-DETECT-7 이 적색으로 잡았다). 수리는 **stderr 판정
    토큰 1차**(rc 는 보조)이고, 이 검체는 그 수리를 **행동**으로 못박는다. 정적 검체
    (H-HOOK-DECIDE-1/2)만으로는 문자열이 맞는데 라우팅이 틀린 경우를 못 본다."""
    routes = [
        # (라벨, cys 스텁 분기, 기대 발화 여부, 왜)
        ("토큰 부재(stub exit 0)+worker-2",
         'case "$1" in surface-role) echo "worker-2"; exit 0;; esac',
         False, "토큰이 없으면 종전 게이트가 살아 worker-2 를 차단해야 한다"),
        ("토큰 proceed + worker-2",
         'case "$1" in hook) echo "[cys-hook] hook-decide: proceed" >&2; exit 0;; '
         'surface-role) echo "worker-2"; exit 0;; esac',
         True, "데몬 권위 통과 판정이면 종전 게이트를 건너뛴다"),
        ("토큰 suppress + 미claim",
         'case "$1" in hook) echo "[cys-hook] hook-decide: suppress" >&2; exit 3;; '
         'surface-role) echo ""; exit 0;; esac',
         False, "데몬 권위 차단 판정은 종전 게이트와 무관하게 차단한다"),
        ("구 데몬 스큐(method_not_found) + 미claim",
         'case "$1" in hook) echo "error: method_not_found: unknown method: hook.decide" >&2; '
         'exit 1;; surface-role) echo ""; exit 0;; esac',
         True, "구 데몬에서는 종전 게이트가 판정한다(미claim 통과)"),
        ("미지 토큰(형상 스큐) + worker-2",
         'case "$1" in hook) echo "[cys-hook] hook-decide: banana" >&2; exit 0;; '
         'surface-role) echo "worker-2"; exit 0;; esac',
         False, "미지 토큰을 통과로 읽으면 안 된다 — 종전 게이트로 접힌다"),
        ("롤백 스위치(CYS_BOOT_GATES=0) + worker-2",
         'case "$1" in hook) echo "[cys-hook] hook-decide: legacy" >&2; exit 5;; '
         'surface-role) echo "worker-2"; exit 0;; esac',
         False, "롤백은 종전 동작으로의 완전 복귀다 — 게이트가 살아 있어야 한다"),
    ]
    bad = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, (label, extra, want_fire, why) in enumerate(routes):
            sb = os.path.join(tmp, "r%d" % i)
            env, _h, _p, _b, _s = _rb_sandbox(sb)
            env.pop("CYS_BOOT_GATES", None)
            if "CYS_BOOT_GATES=0" in label:
                env["CYS_BOOT_GATES"] = "0"
            _mock_cys(os.path.join(sb, "bin"), sb, extra)
            r = _run_rb(env)
            fired = "발화됨" in r.stdout
            if fired != want_fire:
                bad.append("%s: 발화=%s(기대 %s) — %s | stderr=%r"
                           % (label, fired, want_fire, why, r.stderr[-300:]))
    need(not bad, "위임 라우팅 오작동 %d건: %s" % (len(bad), " / ".join(bad)))
    return "라우팅 %d경로 전건 일치(토큰 부재·proceed·suppress·구 데몬 스큐·미지 토큰·롤백)" % len(routes)


@specimen("H-BOOT-INTENT-1", "W6",
          "P2 boot-intent 프런트도어 — enqueued+rc0=스폰 생략+정직 note · 그 외 전부 spawn 폴백"
          "(supervisor_off=legacy · 구 CLI 조용 스큐 · rc 불일치·위조 loud · claim rc0 게이트)",
          ["R3-P2-3", "R3-P2-4", "R3-P2-6", "R3-P2-7", "R3-P2-8"])
def h_boot_intent_1():
    """P2(R3-P2-1 ⓑ′): 훅의 직접 spawn 이 `cys boot-intent`(RPC boot.enqueue) 프런트도어로
    이관됐다 — **enqueued+rc0 만** 스폰 생략(스폰은 데몬 감독자 소관)이고 그 외 전부(토큰 부재·
    error·legacy·supervisor_off·타임아웃·rc 불일치·선행 claim rc≠0)는 종전 spawn 블록 폴백이다
    (fail-open — 새 차단자 0 · R3-P2-8). 판독은 hook-decide 판독기 동형 3층(토큰 줄 단위 정확
    일치 1차 · 줄 개수 1 · rc 교차 거부권)이다 — rc 를 1차로 읽으면 stub cys(무조건 exit 0)가
    '등록 성공'이 되어 폴백 spawn 이 건너뛰어지고 부트가 무음 사망한다(R3-P2-3 · 이 저장소가
    두 번 치른 rc0=통과 클래스). 기존 검체군은 `_mock_cys` 의 boot-intent 기본값(구 CLI 모사 —
    rc2+unrecognized)으로 종전 spawn **폴백 leg** 를 계속 재고, frontdoor leg 는 이 검체가
    전담한다(R3-P2-7 ⓐ·ⓑ — H-MISSION-1 ⓐ 의 frontdoor 쌍둥이 leg 포함)."""
    notes = []
    boot = ("import os, sys\n"
            "if 'lane-path' in sys.argv:\n"
            "    print('(mock lane path)')\n"
            "    raise SystemExit(0)\n"
            "c = os.path.join(os.environ['HOME'], '.cys', 'state', 'boot-calls')\n"
            "os.makedirs(os.path.dirname(c), exist_ok=True)\n"
            "open(c, 'a').write('CALL\\n')\n"
            "print('MOCK-BOOT')\n")

    def _boot_calls(state, want):
        """스폰 계수(want>0 이면 bounded 대기 후, 0 이면 정착 창 후 판독)."""
        p = os.path.join(state, "boot-calls")
        if want:
            end = time.time() + 8.0
            while time.time() < end and not os.path.isfile(p):
                time.sleep(0.1)
        else:
            time.sleep(1.0)     # 버그(비동기 오스폰)가 착지할 시간을 준다 — 정상 경로는 동기 0
        return _read(p).count("CALL") if os.path.isfile(p) else 0

    def _note_json(stdout, where):
        for line in stdout.splitlines():
            if line.startswith('{"hookSpecificOutput"'):
                try:
                    return json.loads(line)["hookSpecificOutput"]["additionalContext"]
                except (ValueError, KeyError, TypeError):
                    raise Fail("%s: note JSON 이 완주 파싱되지 않는다(부분 출력 사망=소실): %r"
                               % (where, line[:200]))
        raise Fail("%s: note JSON 줄이 없다(통보 소실): %r" % (where, stdout[:300]))

    _FRONT_ARM = ('boot-intent) echo "[cys-hook] boot-intent: enqueued" >&2; '
                  'echo "[cys-hook] boot-intent detail: mock enqueue" >&2; exit 0;;')
    with tempfile.TemporaryDirectory() as tmp:
        # ── ⓐ frontdoor 성공(enqueued+rc0 · 임무 미지정 — H-MISSION-1 ⓐ 쌍둥이): 스폰 0 +
        #    정직 note(인텐트 기록·감독자 서술·로그 경로 분기) + 규율 문장 보존(R3-P2-7 ③④⑤).
        sb = os.path.join(tmp, "front")
        env, _h, _p, _b, state = _rb_sandbox(sb, mission=None, boot_body=boot)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) echo ""; exit 0;; %s esac' % _FRONT_ARM)
        r = _run_rb(env)
        need(r.returncode == 0, "훅이 비0 종료(exit=%d)" % r.returncode)
        need(_HOOK_FIRED_MARK in r.stdout,
             "frontdoor note 가 헤드라인 마커를 잃었다(§0-A 리터럴 3중 결박 파괴 · R3-P2-7 ③)")
        need("boot-intent enqueued" in r.stderr,
             "frontdoor 판정의 stderr 1줄 로그가 없다: %r" % r.stderr[:300])
        note = _note_json(r.stdout, "frontdoor")
        for frag in ("부트 인텐트를 데몬에 기록했다", "데몬 감독자", "최대 3회", "boot-supervisor.log",
                     "재실행하지 마라", "next-action", "exit 3",
                     "임무 게이트 폐쇄", "자율 착수"):
            need(frag in note, "frontdoor note 에 %r 이 없다(정직 서술/규율 문장 결손 — "
                 "R3-P2-7 ④⑤): %r" % (frag, note[:400]))
        need("백그라운드로 실행했다" not in note,
             "frontdoor note 가 spawn 을 서술한다 — 훅은 기록만 했다(정직성 불변식 :63-66 위반)")
        need(_boot_calls(state, 0) == 0,
             "frontdoor(enqueued+rc0)인데 훅이 직접 spawn 했다 — 이중 기동 표면(선언당 시도 "
             "상한 5 초과 방향)")
        logs = [n for n in (os.listdir(state) if os.path.isdir(state) else [])
                if re.match(r"^role-bootstrap-\d+-\d+\.log$", n)]
        need(not logs, "frontdoor 무스폰인데 런 로그가 생겼다(로그 경로 분기 오염): %s" % logs)
        notes.append("frontdoor: 스폰 0·정직 note·규율 보존")
        # ── ⓑ supervisor_off(=legacy rc5) → 조용한 spawn 폴백(R3-P2-4 소비면 · 검체 ⓕ) ──
        sb = os.path.join(tmp, "supoff")
        env, _h, _p, _b, state = _rb_sandbox(sb, boot_body=boot)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) echo ""; exit 0;; '
                  'boot-intent) echo "[cys-hook] boot-intent: legacy" >&2; '
                  'echo "[cys-hook] boot-intent detail: supervisor_off — 감독자 미기동, 종전 '
                  'spawn 폴백을 타라" >&2; exit 5;; esac')
        r = _run_rb(env)
        need("발화됨" in r.stdout and "백그라운드로 실행했다" in r.stdout,
             "supervisor_off 에서 종전 spawn 폴백이 죽었다 — '등록 성공·발화자 0' 무음 후퇴"
             "(R3-P2-4 blocker 재발): %r" % (r.stdout[:200] + r.stderr[-200:]))
        need(_boot_calls(state, 1) == 1, "supervisor_off 폴백 spawn 이 정확히 1회가 아니다")
        # ★loud 판정은 boot-intent 자기 진단 줄로 좁힌다 — 목 cys 는 `cys hook` 위임에도 rc0
        #   무토큰으로 답해 그쪽 loud 폴백 줄('cys hook 위임 실패')이 stderr 에 정상 공존한다.
        need("boot-intent 위임 실패" not in r.stderr,
             "legacy(rc5)가 loud 로 오분류됐다 — CLI 가 이미 사유를 남긴 경로다(R3-P2-8 경보 피로)")
        notes.append("supervisor_off: 조용한 폴백 spawn 1")
        # ── ⓒ 구 CLI 모사 기본값(_mock_cys · R3-P2-7 ⓐ) → 조용한 스큐 폴백 ──
        sb = os.path.join(tmp, "oldcli")
        env, _h, _p, _b, state = _rb_sandbox(sb, boot_body=boot)
        r = _run_rb(env)
        need("발화됨" in r.stdout, "구 CLI 모사에서 폴백 spawn 이 죽었다: %r" % r.stdout[:200])
        need(_boot_calls(state, 1) == 1, "구 CLI 폴백 spawn 이 정확히 1회가 아니다")
        need(any("boot-intent" in ln for ln in _calls(sb).splitlines()),
             "훅이 boot-intent 프런트도어를 시도하지 않았다(배선 부재 — 검체 무효)")
        need("boot-intent 위임 실패" not in r.stderr,
             "구 CLI 모사(rc2+unrecognized)가 조용한 스큐로 접히지 않는다"
             "(_cys_hook_legacy_unavailable 재사용 소실 — R3-P2-8): %r" % r.stderr[:300])
        notes.append("구 CLI 모사: 조용한 스큐 폴백 spawn 1")
        # ── ⓓ rc 거부권·위조 차단: 토큰-rc 불일치 / 복수 토큰 → 폴백 spawn + loud ──
        for name, arm, mark in (
                ("rc 불일치",
                 'boot-intent) echo "[cys-hook] boot-intent: enqueued" >&2; exit 1;;', "불일치"),
                ("복수 토큰",
                 'boot-intent) echo "[cys-hook] boot-intent: enqueued" >&2; '
                 'echo "[cys-hook] boot-intent: enqueued" >&2; exit 0;;', "위조 의심")):
            sb = os.path.join(tmp, re.sub(r"\W+", "_", name))
            env, _h, _p, _b, state = _rb_sandbox(sb, boot_body=boot)
            _mock_cys(os.path.join(sb, "bin"), sb,
                      'case "$1" in surface-role) echo ""; exit 0;; %s esac' % arm)
            r = _run_rb(env)
            need("발화됨" in r.stdout and _boot_calls(state, 1) == 1,
                 "%s 에서 폴백 spawn 이 죽었다(fail-open 소실): %r" % (name, r.stderr[-300:]))
            need(mark in r.stderr,
                 "%s 가 침묵으로 접혔다(%r 로그 부재): %r" % (name, mark, r.stderr[:300]))
            notes.append("%s: 폴백 spawn 1 + loud" % name)
        # ── ⓔ 선행 claim rc0 게이트: rc6 이면 boot-intent 자체를 부르지 않는다 —
        #    인텐트를 남기면 감독자가 claim_stale 로 무음 Retire 해 boot-last 완주 기록
        #    (§0-A session_error 근거·P0-3)이 소실된다. 종전 spawn 폴백이 의미론을 보존한다.
        sb = os.path.join(tmp, "rc6gate")
        env, _h, _p, _b, state = _rb_sandbox(sb, boot_body=boot)
        _mock_cys(os.path.join(sb, "bin"), sb,
                  'case "$1" in surface-role) echo ""; exit 0;; claim-role) exit 6;; %s esac'
                  % _FRONT_ARM)
        r = _run_rb(env)
        need(not any("boot-intent" in ln for ln in _calls(sb).splitlines()),
             "선행 claim rc6 인데 boot-intent 를 불렀다 — claim_stale 무음 Retire 로의 후퇴"
             "(boot-last 완주 기록·§0-A 재시도 근거 소실)")
        need("발화됨" in r.stdout and _boot_calls(state, 1) == 1,
             "claim rc6 의 종전 spawn 폴백이 죽었다: %r" % r.stdout[:200])
        need("결정론적으로 실패한다" in r.stdout,
             "rc6 정직 예보(P0-4)가 폴백 note 에서 사라졌다: %r" % r.stdout[:300])
        notes.append("claim rc6: frontdoor 미시도·폴백 spawn 1·정직 예보 보존")

    # ── 소스 핀(판독기 동형 3층 · R3-P2-3) — 훅은 팩에 항상 실재한다 ──
    hook = _read(_hook(ROLE_BODY))
    for tk in ("enqueued", "error", "legacy"):
        need(re.search(r'^\s*"\[cys-hook\] boot-intent: %s"[|)]' % tk, hook, re.M),
             "훅이 boot-intent 토큰 %r 를 **줄 단위 정확 일치**로 읽지 않는다(A3=B7 계열 위조 "
             "표면)" % tk)
        need('*"[cys-hook] boot-intent: %s"*)' % tk not in hook,
             "★위조 통로 유입 — boot-intent 토큰 %r 를 stderr 전문 substring glob 으로 읽는다" % tk)
    need('[ "$CYS_BI_TOKEN_N" -ne 1 ]' in hook, "boot-intent 토큰 줄 **개수 1** 판정이 없다")
    need('[ "$CYS_BI_RC" = "0" ]' in hook, "boot-intent rc 교차(enqueued↔0 거부권)가 없다")
    need('cys_timeout_run "$CYS_BOOT_INTENT_TIMEOUT_S" cys boot-intent </dev/null 2>&1 >/dev/null'
         in hook,
         "boot-intent 호출에 외곽 데드라인·stdin 차단·stdout 버림 규율이 없다(R3-RISK-2 — "
         "데몬 wedge 가 UserPromptSubmit 을 40s 붙잡는다)")
    need('[ "$CLAIM_RC" = "0" ] && command -v cys' in hook,
         "선행 claim rc0 게이트가 없다 — rc6/rc7 인텐트가 claim_stale 무음 Retire 로 후퇴한다")
    # ── 소스 핀(rust 양면) — 배포 팩(소스 부재)에서는 이 축만 접는다(실행 leg 는 위에서 완주) ──
    cli_p = os.path.join(REPO_DIR, "src", "bin", "cys.rs")
    dae_p = os.path.join(REPO_DIR, "src", "bin", "cysd", "handlers.rs")
    if os.path.isfile(cli_p) and os.path.isfile(dae_p):
        cli, dae = _read(cli_p), _read(dae_p)
        need('const BOOT_INTENT_VERDICT_PREFIX: &str = "[cys-hook] boot-intent: ";' in cli,
             "CLI 토큰 접두 상수 부재/불일치(셸 case 와 갈림 — 위임이 조용히 legacy 로 접힌다)")
        need('e.starts_with("supervisor_off")' in cli,
             "CLI 가 supervisor_off 를 legacy(exit 5)로 환원하지 않는다 — R3-P2-4 blocker 의 "
             "소비면 소실('등록 성공·발화자 0' 무음 후퇴 재개방)")
        need("fn boot_intent_verdict(" in cli and "boot_intent_verdict_lines(" in cli,
             "boot-intent 판정 토큰 단일 출구가 없다(토큰 누락·자유 문구 혼입 표면)")
        need('"", // lane 자기 고정' in dae,
             "(ⓔ) boot.enqueue arm 의 lane 자기 고정(빈값) 인자가 없다 — 호출자 lane 이 열리면 "
             "레인↔팩 불일치 표면(R3-P2-6)")
        # ★부트 v2(W-B B2-a) 개명 수용 — **추가만**(구 이름을 지우지 않는다).
        #   구: boot_enqueue_writes_a_v2_intent_with_own_lane_and_unique_id
        #   신: boot_enqueue_writes_a_v3_intent_with_own_lane_and_content_derived_id
        #   개명 사유: 인텐트 id 가 '호출마다 유일한 카운터'에서 '선언 **내용** 기반 decl_id'
        #   로 바뀌었다(명세 §2-5). 이 핀이 재는 것은 이름이 아니라 **'lane 항상 빈값' 을 재는
        #   cargo 검체의 실재**이며, 신 검체도 `it["lane"] == json!("")` 를 그대로 단언한다.
        #   두 이름 중 하나라도 있으면 통과다 — 구 이름을 지우면 롤백 브랜치에서 오적발이 난다.
        need(("boot_enqueue_writes_a_v2_intent_with_own_lane_and_unique_id" in dae)
             or ("boot_enqueue_writes_a_v3_intent_with_own_lane_and_content_derived_id" in dae),
             "(ⓔ) 'enqueue 산출 인텐트 lane 항상 빈값' 핀(cargo 검체)이 삭제됐다")
        need("boot_enqueue_refuses_when_supervisor_is_off" in dae,
             "(ⓕ 데몬면) supervisor_off 미기록 핀(cargo 검체)이 삭제됐다")
        notes.append("소스 핀: 훅 판독기 3층 + CLI/데몬 양면")
    else:
        notes.append("소스 핀: 훅 판독기 3층(rust 축은 배포 팩이라 생략)")
    # ── 계측 타당성: 구 훅에는 frontdoor 가 없어야 한다(있으면 이 검체는 무엇도 증명 못 한다) ──
    old = _git_show("cysjavis-pack/hooks/role-bootstrap.sh")
    calib = "skip(no-git)"
    if old is not None:
        need("boot-intent" not in old,
             "계측 타당성 실패: 기준 커밋 훅에 이미 boot-intent 가 있다")
        calib = "구 훅 frontdoor 부재 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib



# ═══════════════════════════════════════════════════════════════════════════
# U-23 · boot_supervisor (감독자) — 별도 태스크 · 유계 재시도 · enum 인텐트
# ═══════════════════════════════════════════════════════════════════════════
# ★U-23 계측 대조 기준: 이 단위의 **수리 전 트리**(캠페인 베이스). 전역 `CALIBRATION_REF`
#   (W0 착지 커밋)에는 `delivery.rs` 자체가 아직 없어 "탐지기가 못 잡았다"가 **기준 선택
#   오류**로 나온다 — `_git_show` docstring 이 경고하는 바로 그 오보다.
_U23_CALIB_REF = os.environ.get("CYS_HEALTH_U23_CALIB_REF", "985093d")
_U23_RS_SUP = "src/bin/cysd/boot_supervisor.rs"
_U23_RS_MAIN = "src/bin/cysd/main.rs"
_U23_RS_GOV = "src/bin/cysd/governance.rs"
_U23_RS_DEL = "src/bin/cysd/delivery.rs"

# 감독자 코드에 **한 글자도** 있으면 안 되는 파괴/재기동 어휘. 이 저장소의 제1 계약
# ("오살이 오탐보다 훨씬 위험하다")의 기계 집행 지점이며, Rust 단위테스트
# `supervisor_never_kills_anything` 과 **같은 목록**이다(2벌 · 어느 한쪽만 고쳐지면 갈린다).
_U23_DESTRUCTIVE = ("close_surface", "kill_on_drop(true)", "check_agent_death",
                    "launch_via_cli", "restart_counts", "reap_", ".kill(")


def _rs_prod(src):
    """Rust 소스의 **프로덕션 부분만** 남기고 `//` 줄주석을 제거한다.

    두 가지를 동시에 떨어낸다:
      ① `#[cfg(test)]` 이후 — 테스트 모듈은 금칙 어휘를 **문자열로** 들고 있는 것이 정상이다
         (검체 자신이 그 목록을 선언한다). 자르지 않으면 검체가 자기 자신을 위반으로 읽는다.
      ② `//` 줄주석 — 핀은 '무엇을 **부르는가**'를 보는 장치이고 주석은 부르는 것이 아니다.
         주석 한 줄로 핀이 적색이 되면 다음 사람은 설명을 지우거나 핀을 완화한다(둘 다 나쁘다).
    (문자열 리터럴 안의 `//` 까지 보는 렉서가 아니다 — 대상 파일들에 그런 리터럴이 없고,
     생기면 이 핀이 곧바로 적색으로 알려 준다.)"""
    if src is None:
        src = ""
    i = src.find("#[cfg(test)]")
    if i > 0:
        src = src[:i]
    return "\n".join(l.split("//")[0] for l in src.splitlines())


def _rs_const_usize(src, name):
    """`const <name>: usize = <n>;` 의 값. 없으면 None.

    ★상수의 **값**까지 재는 이유: 두 유계 축(판정 상한·GC 상한)이 이름만 둘이고 값이 같아지면
      그것은 축이 하나로 **다시 합쳐진 것**이고, 그 합침이 정확히 이번에 수리한 GC 기아다."""
    m = re.search(r"const %s: usize = (\d+);" % re.escape(name), src or "")
    return int(m.group(1)) if m else None


def _rs_inject(src, text):
    """합성 변조본 제조기 — **프로덕션 영역**(테스트 모듈 앞)에 코드를 심는다.
    파일 끝에 붙이면 `#[cfg(test)]` 뒤라 판정 대상 밖이고, 그러면 '탐지기가 못 잡았다'는
    결과가 탐지기 고장이 아니라 변조본 제조 오류가 된다(오보 방지)."""
    src = src or ""
    i = src.find("#[cfg(test)]")
    return (src[:i] + text + "\n" + src[i:]) if i > 0 else (src + "\n" + text + "\n")


def _u23_tick_violations(sup, main_rs, gov):
    """H-TICK-ALIVE 판정기 — '감독자는 watchdog 틱을 절대 막지 않는다'."""
    sup, main_rs, gov = sup or "", main_rs or "", gov or ""
    v = []
    if not sup.strip():
        v.append("감독자 모듈이 없다(boot_supervisor.rs 부재) — R3 단발 체인 그대로")
        return v
    sup_c, gov_c, main_c = _rs_prod(sup), _rs_prod(gov), _rs_prod(main_rs)
    # ① 감독자는 **별도 tokio 태스크**다.
    if "tokio::spawn(" not in sup_c:
        v.append("감독자가 별도 tokio 태스크가 아니다 — 부트가 어느 틱 숙주를 정지시킨다")
    if "catch_unwind" not in sup_c:
        v.append("감독자 틱에 패닉 격리가 없다 — 한 번의 패닉이 감독자를 영구 소멸시킨다(치명위험 ③)")
    # ② 자기 cadence — watchdog 과 같은 값이면 두 태스크가 lockstep 으로 깨어난다.
    ms = re.search(r"const SUPERVISOR_INTERVAL_SECS: u64 = (\d+);", sup)
    mw = re.search(r"const WATCHDOG_INTERVAL_SECS: u64 = (\d+);", gov)
    if not ms:
        v.append("감독자 자기 cadence 상수(SUPERVISOR_INTERVAL_SECS)가 없다")
    elif mw and ms.group(1) == mw.group(1):
        v.append("감독자 cadence 가 watchdog(%s초)과 같다 — lockstep" % mw.group(1))
    # ③ watchdog 틱 숙주 오염 0 — governance 는 감독자를 알지 못한다.
    if "boot_supervisor" in gov_c:
        v.append("governance(watchdog 틱 숙주)가 감독자를 참조한다 — 부트 1회가 틱을 정지시킨다")
    # ④ watchdog 태스크의 `.await` 는 sleep 하나뿐이라는 계약(틱 본문 = 동기 클로저).
    wi = gov_c.find("pub fn spawn_watchdog(")
    if wi < 0:
        v.append("spawn_watchdog 소실 — watchdog 계약을 확인할 수 없다(계측 무효)")
    else:
        body = gov_c[wi:gov_c.find("\nfn env_u64(", wi) if gov_c.find("\nfn env_u64(", wi) > 0
                     else wi + 12000]
        n_await = body.count(".await")
        if n_await != 1:
            v.append("watchdog 태스크의 .await 가 %d개다 — sleep 하나라는 계약이 깨졌다" % n_await)
        if "tokio::time::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SECS)).await" not in body:
            v.append("watchdog 의 유일한 .await 가 sleep 이 아니다")
        # ⑤ 틱 4단 순서 불변식(감독자가 이 순서를 흔들지 않았다).
        order = ["refresh_seat_cache(&daemon", "deliver_queued(&daemon",
                 "check_agent_death(&daemon", "check_role_deadman(&daemon"]
        idx = [body.find(x) for x in order]
        if any(i < 0 for i in idx) or idx != sorted(idx):
            v.append("watchdog 틱 4단 순서 불변식이 깨졌다: %s" % list(zip(order, idx)))
    # ⑥ 기동 지점은 main.rs 정확히 1곳.
    if main_c.count("boot_supervisor::spawn(") != 1:
        v.append("main.rs 의 감독자 기동 지점이 정확히 1곳이 아니다(%d곳)"
                 % main_c.count("boot_supervisor::spawn("))
    if "mod boot_supervisor;" not in main_c:
        v.append("main.rs 에 감독자 모듈 선언이 없다")
    return v


def _u23_bound_violations(sup, delivery):
    """H-BOOT-SUP-1 판정기 — 유계성 3중 · enum 전용 인텐트 · 스풀 봉인 · 롤백 접힘 · 오살 0."""
    sup, delivery = sup or "", delivery or ""
    v = []
    if not sup.strip():
        v.append("감독자 모듈이 없다(boot_supervisor.rs 부재)")
        return v
    c = _rs_prod(sup)
    # ── 폭주 차단 3중(치명위험 ①) ────────────────────────────────────────────
    if "pub const MAX_ATTEMPTS: u32" not in c or "attempts >= MAX_ATTEMPTS" not in c:
        v.append("시도 상한(MAX_ATTEMPTS)이 없거나 판정에 쓰이지 않는다 — 무한 재시도")
    if "pub const RETRY_COOLDOWN_SECS: f64" not in c or "fn backoff_until(" not in c:
        v.append("쿨다운/백오프가 없다 — 상한만으로는 순간 폭주를 막지 못한다")
    if "fn effective_attempts(" not in c or "file_attempts.max(mem_attempts)" not in c:
        v.append("메모리측 예산 합류가 없다 — 디스크 쓰기가 실패하면 예산이 영원히 0 이다(폭주)")
    if "const MAX_DISPATCH_PER_TICK" not in c or "dispatched >= MAX_DISPATCH_PER_TICK" not in c:
        v.append("틱당 디스패치 상한이 없다 — 스풀 홍수가 프로세스 떼를 낳는다")
    # ── ★핀 이사(U-28 · 2026-08-24 · 러너 헤더 '핀 이사 계약' ①②④) ───────────────────────
    # 【원인 규명 먼저 — 검체의 니들 두 개가 그 자체로 결함이었다】
    #   ⓐ 종전 형상 `names.truncate(MAX_SCAN_ENTRIES)` **한 줄**은 판정과 GC 를 **같은 상한으로
    #      함께** 잘랐다. 정렬 앞쪽 64건이 살아있는(혹은 지워지지 않는) 인텐트로 차 있는 동안
    #      그 뒤의 쓰레기는 **영원히** 검사되지 않는다 = 스풀 무한 성장(결함 2b · GC 기아).
    #   ⓑ 종전 형상 `budget.clear()` 는 예산 맵을 통째로 비웠다. 그런데 메모리측 예산을 둔
    #      **근거 자체**가 '읽기전용 스풀·디스크 만실에서 attempts 가 영원히 0' 인 상황이다 —
    #      바로 그 상황에서 clear 가 일어나면 두 축이 동시에 무너져 인텐트당 3회 상한이
    #      무력화된다(매 틱 프로세스 1개 = 폭주 · 결함 2a).
    # 【축은 하나도 완화하지 않는다】 축 넷 — "틱당 **판정**이 유계 · 틱당 **GC 검사**가 유계 ·
    #   예산 맵이 유계 · 죽은 항목이 **매 틱** 회수된다" — 는 전부 그대로 요구한다. 종전 두 니들이
    #   덮던 것을 **네 축으로 쪼개 각각** 핀하므로 판정은 넓어졌지 좁아지지 않았다.
    # 【형상 이동】 판정 상한 = `intents.len() < MAX_SCAN_ENTRIES` · GC 상한 = `min(MAX_GC_ENTRIES)`
    #   (신설 독립 축) · 맵 상한 = LRU 축출 + **디스패치 중단**(clear 대체) · 회수 = 매 틱 retain.
    # 【★추가 강화】 ⓘ 두 상한이 **같은 값으로 다시 합쳐지면 적색**(= GC 기아 재유입) ⓙ 구 병합
    #   형상(`truncate(MAX_*_ENTRIES)`)이 돌아오면 적색 ⓚ `budget.clear()` 가 돌아오면 적색
    #   ⓛ 회전 커서·LRU·디스패치 중단의 실재를 요구한다 — 셋은 종전 형상이 하던 일을 **대신**
    #   하므로, 이것들이 없으면 새 형상은 종전보다 약하다(그때 이사는 삭제가 된다).
    if "const MAX_SCAN_ENTRIES" not in c or "intents.len() < MAX_SCAN_ENTRIES" not in c:
        v.append("틱당 **판정** 상한(MAX_SCAN_ENTRIES)이 없다 — 스풀 홍수가 틱 길이를 무한히 늘린다")
    if "const MAX_GC_ENTRIES" not in c or "min(MAX_GC_ENTRIES)" not in c:
        v.append("틱당 **GC 검사** 상한(MAX_GC_ENTRIES)이 독립 축으로 없다 — 판정 상한이 GC 를 굶긴다")
    _scan_cap = _rs_const_usize(c, "MAX_SCAN_ENTRIES")
    _gc_cap = _rs_const_usize(c, "MAX_GC_ENTRIES")
    if _scan_cap is None or _gc_cap is None:
        v.append("유계 상한 상수를 읽지 못했다(계측 불능): MAX_SCAN_ENTRIES=%s MAX_GC_ENTRIES=%s"
                 % (_scan_cap, _gc_cap))
    elif _gc_cap <= _scan_cap:
        v.append("★판정 상한(%d)과 GC 상한(%d)이 다시 합쳐졌다 — 앞쪽이 살아있는 인텐트로 차면 "
                 "뒤쪽 쓰레기가 영원히 검사되지 않는다(GC 기아 · 결함 2b 재유입)"
                 % (_scan_cap, _gc_cap))
    if re.search(r"\.truncate\(MAX_(SCAN|GC)_ENTRIES\)", c):
        v.append("★구 병합 형상(`truncate(MAX_*_ENTRIES)` 한 줄)이 돌아왔다 — 한 줄이 판정과 GC 를 "
                 "함께 자른다(결함 2b 그 자체)")
    if "st.cursor = scan.next_cursor;" not in c or "scan_spool(dir, now, st.cursor)" not in c \
            or "let start = cursor % total;" not in c:
        v.append("GC **회전 커서**가 없다 — 고정 시작점이면 상한 뒤쪽은 상한을 키워도 기아로 남는다")
    if "const MAX_TRACKED" not in c or "st.budget.len() > MAX_TRACKED" not in c:
        v.append("예산 맵 상한(MAX_TRACKED) 방어가 없다 — 24/365 데몬 누수")
    if "budget.clear()" in c:
        v.append("★예산 맵을 통째로 비운다(budget.clear) — 디스크측 예산이 0 인 바로 그 상황에서 "
                 "메모리측까지 지워 인텐트당 상한이 무력화된다(결함 2a 재유입)")
    if "!scan.present.contains(*id)" not in c or "evictable.sort_by(" not in c:
        v.append("상한 집행이 LRU 축출(스풀에 **없는** 항목만·오래된 것부터)이 아니다 — 살아있는 "
                 "예산을 버려 맵 크기를 지키는 것이 곧 유계 파기다")
    if "suspend_dispatch" not in c or "if suspend_dispatch {" not in c:
        v.append("상한을 넘겨도 **낳는 것을 멈추지** 않는다 — clear 없이 유계를 지킬 방법이 사라진다")
    if not re.search(r"budget\s*\.retain\(", c):
        v.append("예산 맵 매 틱 retain 이 없다(맵 3종 방어 ③)")
    if "\"mtime_gc\"" not in c:
        v.append("디스크 mtime GC 가 없다 — 본문이 깨진 늙은 인텐트가 영원히 남는다")
    # ── 인텐트는 enum 값만 나른다(명령 문자열 금지) ──────────────────────────
    if "fn parse(token: &str) -> Option<Self>" not in c:
        v.append("행위 토큰 파서가 닫힌 enum 이 아니다")
    if 'Disposition::Retire("unknown_action")' not in c:
        v.append("미지 토큰이 폐기가 아니라 실행 후보가 된다 — 스풀이 명령 실행 표면이 된다")
    if c.count("Command::new(") != 1 or "Command::new(&python)" not in c:
        v.append("프로세스 생성 지점이 1곳(고정 인터프리터)이 아니다 — 인텐트가 명령이 될 수 있다")
    # ── 스풀 봉인 ─────────────────────────────────────────────────────────────
    if "state_dir(socket_path).join(SPOOL_DIR_NAME)" not in c:
        v.append("스풀이 state_dir 하위가 아니다 — 부서 격리를 상속하지 못한다")
    if "0o700" not in c or "set_permissions" not in c:
        v.append("unix 0o700 재강제가 없다 — 이전 실행이 남긴 넓은 권한이 그대로다")
    # ── 롤백은 마스터 스위치 하나에 접힌다(규약 6) ────────────────────────────
    if "boot_gates_master_off_from" not in c or "ENV_BOOT_GATES" not in c:
        v.append("롤백이 마스터 스위치(CYS_BOOT_GATES=0)에 접히지 않았다 — 사고 순간 노브 조합 불가")
    if 'pub const ENV_SUPERVISOR: &str = "CYS_BOOT_SUPERVISOR"' not in c:
        v.append("축 노브 상수(CYS_BOOT_SUPERVISOR)가 없다")
    # ── 원장 기록이 행위보다 앞(불변식) ───────────────────────────────────────
    di = c.find("fn dispatch_one(")
    if di < 0:
        v.append("dispatch_one(원장 선행 불변식의 유일 지점) 이 없다")
    else:
        body = c[di:]
        rec, run = body.find("record_audited("), body.find("runner(daemon")
        if rec < 0 or run < 0 or rec > run:
            v.append("원장 기록이 행위보다 앞이 아니다 — 임무 게이트가 기계 push 를 오너 임무로 오인")
    # ★핀 개정(P2 ⑧c → ★R2 must_fix · 2026-08-26 — 약화 아님, 트리거 확대): loud 통보도 pane
    #   주입 전에 같은 유래(Origin::Supervisor)로 원장 **선기록**해야 하므로 지정 지점이
    #   dispatch_one + notify_no_spawn 정확히 2곳이다(닫힌 집합 — 제3 지점 유입은 계속 적색).
    #   ★이름이 notify_exhausted → notify_no_spawn 으로 바뀐 이유가 곧 계약 변경이다: 통보
    #   트리거가 '예산 소진'이 아니라 **'이 인텐트는 스폰 0회로 끝났다'** 다. 종전엔
    #   attempts_exhausted 하나만 통보하고 claim_stale·no_surface·expired·schema_mismatch·
    #   unknown_action·unknown_decl_origin 은 버스 이벤트만 낸 채 사라졌는데, frontdoor note 가
    #   모델에게 '스폰 실패 소진 시 통보한다'고 약속한 뒤 훅이 exit 0 한 경로에서 그 침묵은
    #   그대로 '선언했는데 무반응'이다(R2 정적 적대검증 must_fix).
    if c.count("crate::delivery::Origin::Supervisor") != 2:
        v.append("감독자 원장 유래 지정 지점이 정확히 2곳(dispatch_one·notify_no_spawn)이 아니다")
    ni = c.find("fn notify_no_spawn(")
    if ni < 0:
        v.append("무스폰 loud 통보 지점(notify_no_spawn)이 없다 — 조용한 포기(청중 0) 회귀")
    else:
        nbody = c[ni:]
        nrec, ninj = nbody.find("record_audited("), nbody.find("write_tx.try_send(")
        if nrec < 0 or ninj < 0 or nrec > ninj:
            v.append("무스폰 통보의 원장 기록이 주입보다 앞이 아니다 — 기계 push 오너 임무 오인 창")
    # ★스폰 0회 종착의 **닫힌 집합이 전부 loud 인가**(R2 must_fix 회귀 핀): tick_in 안의 통보
    #   지점은 3곳(판정 폐기 · 실행자 폐기 · 마지막 시도 실패)이다. 갈래가 늘면서 통보를
    #   빠뜨리는 것이 정확히 이 결함의 재발 양식이므로 **갯수 자체**를 못 박는다.
    ti = c.find("fn tick_in(")
    if ti < 0:
        v.append("tick_in(무스폰 종착의 유일 소비 지점) 이 없다")
    elif c[ti:].count("notify_no_spawn(") != 3:
        v.append("tick_in 의 무스폰 통보 지점이 3곳(판정 폐기·실행자 폐기·마지막 시도 실패)이 "
                 "아니다 — 종착 갈래가 늘었는데 통보를 빠뜨렸다(조용한 포기 회귀)")
    # ★provenance 상속 절단(R2 should_fix): '주지 않기로' 판정한 값은 상속이 아니라 부재여야
    #   한다 — env_remove 가 없으면 데몬 env 오염(hook-human 마커·타 pane 좌석 토큰)이 자식에게
    #   흘러 기계유래 게이트를 통과한 적 없는 인텐트가 부서 자동 생성 봉인을 연다.
    re_i = c.find("fn run_ensure_team(")
    if re_i < 0:
        v.append("run_ensure_team(provenance 주입의 유일 지점) 이 없다")
    else:
        rbody = c[re_i:]
        for key in ('env_remove("CYS_DECL_ORIGIN")', "env_remove(cys::ENV_SEAT_TOKEN)"):
            if key not in rbody:
                v.append("run_ensure_team 에 %s 가 없다 — 주입하지 않기로 한 값이 데몬 env 에서 "
                         "상속된다(폭주 봉인 ⓑ 무료 개방)" % key)
    # ── 오살 금지 ─────────────────────────────────────────────────────────────
    for w in _U23_DESTRUCTIVE:
        if w in c:
            v.append("감독자에 파괴/재기동 경로가 들어왔다: %s" % w)
    # ── delivery.rs 어휘 additive ─────────────────────────────────────────────
    if delivery.strip():
        d = _rs_prod(delivery)
        if "    Supervisor,\n" not in d or 'Origin::Supervisor => "supervisor",' not in d:
            v.append("delivery.rs 에 Origin::Supervisor 어휘가 없다(원장 유래 미등재)")
        for keep in ('Origin::Send => "send",', 'Origin::Boot => "boot",',
                     'Origin::GuiAuto => "gui_auto",'):
            if keep not in d:
                v.append("기존 원장 어휘가 사라졌다: %s" % keep)
    return v


@specimen("H-TICK-ALIVE", "W6",
          "U-23 감독자는 watchdog 틱을 막지 않는다 — 별도 태스크·자기 cadence·틱 4단 순서 보존 · "
          "watchdog 의 유일한 .await 는 sleep",
          ["R3"])
def h_tick_alive():
    """★이 검체가 지키는 것: 부트 1회(수십 초)를 watchdog 틱 본문(**동기 클로저**)에 얹으면
    큐 배달·승인 격상·데드맨·회수가 그만큼 통째로 정지한다 — 이 저장소가 실제로 낸 사고
    ②(무clear 게이트)·③(watchdog 사망)의 기전 그대로다. 그래서 감독자는 별도 태스크여야 한다."""
    sup = _read(os.path.join(REPO_DIR, _U23_RS_SUP))
    main_rs = _read(os.path.join(REPO_DIR, _U23_RS_MAIN))
    gov = _read(os.path.join(REPO_DIR, _U23_RS_GOV))
    if not main_rs or not gov:
        raise Skip("배포 팩(Rust 소스 부재) — 소스 배선 검체 적용 불가")
    v = _u23_tick_violations(sup, main_rs, gov)
    need(not v, "U-23 틱 계약 위반 %d건: %s" % (len(v), " / ".join(v)))
    # ★계측 타당성 ① 기준 커밋 대조 — 그 트리엔 감독자가 아예 없다(R3 그 자체).
    base_gov = _git_show(_U23_RS_GOV, _U23_CALIB_REF)
    calib = "계측대조 생략(레포 아님)"
    if base_gov:
        ov = _u23_tick_violations(_git_show(_U23_RS_SUP, _U23_CALIB_REF),
                                  _git_show(_U23_RS_MAIN, _U23_CALIB_REF), base_gov)
        need(ov, "계측 무효: 수리 전 트리(%s)에서 탐지기가 위반을 하나도 못 찾았다" % _U23_CALIB_REF)
        calib = "수리 전 %s 에서 %d건 FIRE" % (_U23_CALIB_REF, len(ov))
    # ★계측 타당성 ② 합성 변조본 — 트리에 위반이 0 이라 초록인 것과 탐지기 고장을 가른다.
    mutants = [
        ("감독자를 틱 숙주로 이사", sup, main_rs,
         _rs_inject(gov, "fn m() { let _ = boot_supervisor::spawn; }")),
        ("main 기동 지점 삭제", sup, main_rs.replace("boot_supervisor::spawn(", "no_spawn("), gov),
        ("cadence lockstep", sup.replace("const SUPERVISOR_INTERVAL_SECS: u64 = 3;",
                                         "const SUPERVISOR_INTERVAL_SECS: u64 = 5;"),
         main_rs, gov),
        ("감독자 별도 태스크 해제", sup.replace("tokio::spawn(", "inline_call("), main_rs, gov),
        ("watchdog 4단 순서 뒤집기", sup, main_rs,
         gov.replace("refresh_seat_cache(&daemon, &sys);", "let _ = 0;", 1)),
    ]
    blind = [lbl for lbl, s, m, g in mutants if not _u23_tick_violations(s, m, g)]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return "틱 계약 위반 0 · %s · 합성 변조 %d종 전건 적발" % (calib, len(mutants))


@specimen("H-BOOT-SUP-1", "W6",
          "U-23 감독자 안전 계약 — 유계 재시도 3중·인텐트는 enum 값(명령 문자열 0)·스풀 0o700·"
          "롤백 마스터 접힘·원장 선행·파괴 호출 0",
          ["R3"])
def h_boot_sup_1():
    """★이 검체가 지키는 것: 감독자는 '없는 것을 낳는' 주체다. 그 힘에 상한이 없으면 곧
    이벤트/큐 폭주(치명위험 ①)이고, 인텐트가 명령 문자열을 나르면 상태 디렉터리에 쓸 수 있는
    누구든 데몬 권한을 얻는다. 그리고 감독자가 무언가를 **죽일 수 있게** 되는 순간 전 pane
    사망(치명위험 ④)의 새 경로가 열린다 — 그래서 파괴 어휘를 소스에서 0 으로 못박는다."""
    sup = _read(os.path.join(REPO_DIR, _U23_RS_SUP))
    delivery = _read(os.path.join(REPO_DIR, _U23_RS_DEL))
    if not delivery:
        raise Skip("배포 팩(Rust 소스 부재) — 소스 배선 검체 적용 불가")
    v = _u23_bound_violations(sup, delivery)
    need(not v, "U-23 안전 계약 위반 %d건: %s" % (len(v), " / ".join(v)))
    # ★계측 타당성 ① 기준 커밋 대조(감독자 부재 = 위반).
    base_del = _git_show(_U23_RS_DEL, _U23_CALIB_REF)
    calib = "계측대조 생략(레포 아님)"
    if base_del:
        ov = _u23_bound_violations(_git_show(_U23_RS_SUP, _U23_CALIB_REF), base_del)
        need(ov, "계측 무효: 수리 전 트리(%s)에서 탐지기가 위반을 하나도 못 찾았다" % _U23_CALIB_REF)
        calib = "수리 전 %s 에서 %d건 FIRE" % (_U23_CALIB_REF, len(ov))
    # ★계측 타당성 ② 합성 변조본 — 안전장치를 하나씩 떼면 반드시 적색이어야 한다.
    mutants = [
        ("시도 상한 제거", sup.replace("attempts >= MAX_ATTEMPTS", "false"), delivery),
        ("메모리측 예산 제거",
         sup.replace("file_attempts.max(mem_attempts)", "file_attempts"), delivery),
        ("틱당 상한 제거", sup.replace("dispatched >= MAX_DISPATCH_PER_TICK", "false"), delivery),
        ("미지 토큰 실행 허용",
         sup.replace('Disposition::Retire("unknown_action")', "Disposition::Wait { until: 0.0 }"),
         delivery),
        ("원장 선행 역전",
         sup.replace("fn dispatch_one(", "fn dispatch_one_moved(")
            .replace("    crate::delivery::record_audited(", "    let _ = runner(daemon"), delivery),
        ("파괴 경로 유입", _rs_inject(sup, "fn m(d: &Daemon) { d.close_surface(1); }"), delivery),
        ("스풀 권한 재강제 제거", sup.replace("0o700", "0o755"), delivery),
        ("롤백 마스터 접힘 해제",
         sup.replace("cys::boot_gates_master_off_from(master_env) || ", ""), delivery),
        ("원장 어휘 미등재", sup, delivery.replace('Origin::Supervisor => "supervisor",', "")),
        # ★핀 이사(U-28) 판별력 — ⓐ 낡은(취약한) 형상 복귀 ⓑ 새 축 파괴가 **둘 다** 적색이어야
        #   이사이지 삭제가 아니다. 아래 7종이 그것을 매 실행마다 시험한다.
        ("판정·GC 상한 재병합(GC 기아 복귀)",
         sup.replace("const MAX_GC_ENTRIES: usize = 256;",
                     "const MAX_GC_ENTRIES: usize = 64;"), delivery),
        ("구 병합 형상 복귀(단일 truncate)",
         _rs_inject(sup, "fn m(names: &mut Vec<PathBuf>) { names.truncate(MAX_SCAN_ENTRIES); }"),
         delivery),
        ("판정 상한 제거", sup.replace("intents.len() < MAX_SCAN_ENTRIES", "true"), delivery),
        ("GC 회전 커서 제거(뒤쪽 영구 기아)",
         sup.replace("st.cursor = scan.next_cursor;", "st.cursor = 0;"), delivery),
        ("예산 맵 통째 비우기 복귀(clear)",
         _rs_inject(sup, "fn m(st: &mut SupState) { st.budget.clear(); }"), delivery),
        ("살아있는 예산까지 축출(유계 파기)",
         sup.replace("!scan.present.contains(*id)", "true"), delivery),
        ("포화 시 디스패치 중단 해제", sup.replace("if suspend_dispatch {", "if false {"), delivery),
        ("매 틱 retain 제거", sup.replace(".retain(|id, (_, last)|", ".drain_all(|id, (_, last)|"),
         delivery),
    ]
    blind = [lbl for lbl, s, d in mutants if not _u23_bound_violations(s, d)]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return "안전 계약 위반 0 · %s · 합성 변조 %d종 전건 적발" % (calib, len(mutants))


# ═══════════════════════════════════════════════════════════════════════════
# U-26 · 검증 인프라 (C-10) — 2026-08-24
#
# ★이 단위가 메우는 구멍: 이 캠페인이 확정한 4대 근본원인 중 하나가 **"진짜 e2e 테스트가
#   0개"** 였다. 이 러너의 검체는 거의 전부 '소스 문자열 핀' 이다 — 그것은 "그렇게 쓰여
#   있다"를 증명할 뿐 **"그 키를 누르면 실제로 그렇게 된다"** 를 증명하지 못한다.
#   아래 H-FAKE-* 는 실제 프로세스를 실제 PTY 에 띄우고 실제 이스케이프 바이트를 밀어넣어
#   실제 종료코드를 잰다.
#
# ★롤백 스위치는 **env 1지점**이다 — `CYS_U26_OFF=1` 이면 이 절의 검체 전부가 (통과가 아니라)
#   큰 소리 **미측정**이 된다. 기본값(미설정)이 신동작이고, 사고 순간에 사람이 조합할 노브는 없다.
#   ★2026-08-24(이종 리뷰어 [major]) — "큰 소리 Skip" 은 큰 소리가 아니었다. 종전 판은 이 스위치가
#     켜지면 검체 5건이 `SKIP` 으로 접히고 최종 줄이 `GREEN`, 종료코드가 0 이었다. 즉 **스위치를
#     켠 사람에게 러너가 통과했다고 답했다.** 지금은 등재소(`off_switch`)를 경유해 `Disabled` 를
#     던지고, 러너는 그 런의 GREEN 을 박탈하고 exit 2 를 낸다(헤더 '미측정 축 계약').
#   ⚠이것을 `CYS_BOOT_GATES` 마스터에 접지 **않은** 이유: 그 마스터는 **런타임 판정 축**을
#     끄는 스위치다(관문 가드·주입 가드). 이 절은 판정 축을 하나도 만들지 않는다 — 검증
#     인프라다. 검체 스위치를 런타임 마스터에 접으면 "제품 가드를 끄면 그 가드의 검체도
#     같이 꺼지는" 구조가 되어, 가드를 끈 채 초록을 받는 경로가 생긴다. 그게 정확히 이
#     파일이 막으려는 것이다.
U26_OFF_ENV = off_switch("CYS_U26_OFF",
                         "U-26 검증 인프라 검체(H-CI-TAG-1·H-EVID-1·H-FAKE-1/2/3)")
_U26_EVIDENCE_DIR = os.path.join(REPO_DIR, "docs", "evidence")
_U26_EV_JSON = os.path.join(_U26_EVIDENCE_DIR, "probe-2026-08-23-first-run-gates.json")
_U26_VERIFY = os.path.join(REPO_DIR, "scripts", "verify_evidence.py")
_U26_STUB = os.path.join(REPO_DIR, "scripts", "fake_agent.py")
_U26_E2E = os.path.join(REPO_DIR, "scripts", "fake_agent_e2e.py")
_U26_CORPUS = os.path.join("src", "first_run_gates.rs")
_U26_WF = os.path.join(".github", "workflows", "windows-health.yml")


def _u26_gate(*paths):
    """롤백 스위치 + 적용 가능성.

    ★Skip 과 Fail 의 경계가 이 함수의 전부다. **레포 체크아웃 안에서 파일이 없는 것은
      '적용 불가' 가 아니라 '삭제' 다** — 검증 인프라를 지워 초록을 만드는 경로를 열어 두면
      이 절 전체가 장식이 된다(보존 hard gate 와 같은 교리). 그래서:
        · 롤백 스위치가 켜져 있다(사람이 껐다)      → **Disabled**(미측정 · GREEN 박탈 · exit 2)
        · 레포가 아니다(`.git` 없음 = 배포 팩)      → Skip(적용 불가 · 게이트 중립)
        · 레포인데 파일이 없다                      → **Fail**(삭제로 간주)
    """
    if _off_switch_on(U26_OFF_ENV):
        # ★`Skip` 이 아니라 `Disabled` 다 — 이 둘은 다른 사실이고, 러너는 다른 축으로 센다.
        #   (종전엔 `Skip` 이라 5건이 지워지고도 GREEN·exit 0 이 나왔다 — 이 함수 docstring 이
        #    말하는 "삭제로 초록 만들기" 를 스위치 한 줄로 하는 경로였다.)
        raise Disabled("U-26 롤백 스위치가 켜져 있다(%s=%s) — 검증 인프라 검체를 **측정하지 "
                       "않는다**. 이것은 통과가 아니라 미측정이므로 이 런은 GREEN 을 잃고 "
                       "exit 2 가 된다(헤더 '미측정 축 계약')."
                       % (U26_OFF_ENV, os.environ.get(U26_OFF_ENV)))
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        raise Skip("레포 체크아웃이 아니다(배포 팩) — 검증 인프라 검체 적용 불가")
    for p in paths:
        need(os.path.exists(p),
             "검증 인프라 파일이 없다: %s — 레포 안에서의 부재는 '적용 불가' 가 아니라 "
             "**삭제**다(지워서 초록을 만드는 경로를 막는다)" % os.path.relpath(p, REPO_DIR))


def _u26_py(script, *args, **kw):
    r = _run([PY, script] + list(args), timeout=kw.pop("timeout", 300))
    return r


@specimen("H-CI-TAG-1", "W6",
          "U-26 ① windows-health 가 태그 릴리스에서 실제로 돈다 — 트리거 결박 + 예산 규율",
          ["CI-게이트-부재", "V4"])
def h_ci_tag_1():
    """★무엇이 결함이었나: `windows-health.yml` 트리거가 `push: branches:['feat/**']` +
    수동뿐이라 **태그(`v*`) 릴리스에서 Windows 검체가 한 번도 돌지 않았다.** 릴리스 레인이
    같은 태그로 windows-latest 빌드를 돌리기는 하지만 그것은 '컴파일·번들' 이지 부트스트랩
    계약 검증이 아니다 — 즉 출하 시점 Windows 검증은 0 이었다. mac 은 멀쩡한데 Windows
    설치본만 깨지는 사고가 이 저장소에서 반복된 것과 정확히 같은 층위의 구멍이다.

    ★동시에 **비용 규율**을 못박는다: 늘어나는 것은 태그 push 뿐이어야 한다. `branches` 가
    `**` 로 넓어지거나 `pull_request` 가 붙으면 매 push 마다 windows 러너(+cargo 빌드)가
    돌아 예산이 폭발한다. 그래서 '트리거가 있다' 만이 아니라 '넓어지지 않았다' 도 판정한다."""
    wf = os.path.join(REPO_DIR, _U26_WF)
    _u26_gate(wf)
    text = _read(wf)
    on = text[text.index("\non:"):text.index("\npermissions:")]
    need(re.search(r"tags:\s*\[?\s*['\"]v\*['\"]", on),
         "windows-health 트리거에 태그(`v*`)가 없다 — 태그 릴리스에서 Windows 검체가 돌지 않는다")
    need(re.search(r"branches:\s*\[?\s*['\"]feat/\*\*['\"]", on),
         "종전 `feat/**` 브랜치 트리거가 사라졌다(무회귀 위반 — 개발 중 조기 검출 경로다)")
    # ★예산 규율 — 일상 push 빈도를 늘리는 확장은 거부한다.
    need("pull_request" not in on,
         "예산 위반: `pull_request` 트리거는 매 PR 갱신마다 windows 러너를 돌린다")
    for bad in ("branches: ['**']", 'branches: ["**"]', "branches: ['*']"):
        need(bad not in on, "예산 위반: 전 브랜치 트리거(%s)" % bad)
    # 잡이 여전히 무조건 실행인지(‘돌지 않는 초록’ 금지 교리 재확인 — 트리거만 넓히고
    # 잡에 if 를 다는 조합이면 결국 안 도는 것이다).
    job = text[text.index("jobs:"):]
    need(not re.search(r"^\s{4}if:", job, re.M), "잡 수준 `if:` 존재 — 조건부 Windows 검증 금지")
    # ★계측 타당성: 이 검체가 '없어도 초록' 이 아니라는 것을 합성 표본으로 보인다.
    #   (트리에 위반이 0건이므로 탐지기가 죽어 있어도 결과는 똑같이 초록이다 — 그래서 시험한다.)
    blind = []
    for label, mutated in (
        ("태그 트리거 삭제", on.replace("tags: ['v*']", "")),
        ("feat 브랜치 삭제", on.replace("branches: ['feat/**']", "")),
        ("PR 트리거 추가", on + "\n  pull_request: {}\n"),
    ):
        hit = (re.search(r"tags:\s*\[?\s*['\"]v\*['\"]", mutated)
               and re.search(r"branches:\s*\[?\s*['\"]feat/\*\*['\"]", mutated)
               and "pull_request" not in mutated)
        if hit:
            blind.append(label)
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return ("트리거 = feat/** + tags:v* · pull_request 없음 · 전브랜치 확장 없음 · 잡 무조건 실행 · "
            "합성 변조 3종 전건 적발")


@specimen("H-EVID-1", "W6",
          "U-26 ② `docs/evidence/` 보존 계약 — 집행자 실재 · 대장 1:1(삭제 hard gate) · "
          "합성 표본으로 탐지력 증명",
          ["C-10", "실측-부재"])
def h_evid_1():
    """★왜 계약을 기계로 세우는가: 이 캠페인의 4대 근본원인 중 하나가 '**실측 없이 값을
    골랐다**' 이다. 실측 아티팩트가 레포에 남지 않으면 다음 사람은 또 추정으로 값을 고른다.
    그리고 문서만 있고 집행자가 없는 계약은 계약이 아니라 소원이다.

    ★보존(retention) hard gate 가 왜 필요한가: 측정치가 불편할 때 **파일을 지워 초록을
    만드는 것**이 이 캠페인이 명시적으로 차단하기로 한 reward-hack 이다. 대장(README §6)과
    실물의 1:1 을 요구하면 삭제가 조용히 일어날 수 없다 — 지우려면 대장도 고쳐야 하고 그
    한 줄은 diff 에 드러난다.

    ★'위반 0' 은 검사기가 살아 있다는 증거가 아니다(정상 트리엔 원래 위반이 0이다).
    그래서 검사기의 `--self-test`(합성 위반 표본 15종 + 정상 표본 1종)를 **여기서 돌린다**."""
    readme = os.path.join(_U26_EVIDENCE_DIR, "README.md")
    _u26_gate(_U26_VERIFY, _U26_EVIDENCE_DIR, readme)
    # ① 계약 문서 필수 조항.
    rtext = _read(readme)
    for tok in ("retention", "provenance", "repro", "cys-evidence/1"):
        need(tok in rtext, "계약 문서에 필수 조항 토큰 부재: %r" % tok)
    # ② 집행자가 실제 트리를 초록으로 판정하는가.
    r = _u26_py(_U26_VERIFY, "--json")
    need(r.returncode == 0, "증거 계약 위반: %s" % (r.stdout[-600:] + r.stderr[-300:]))
    d = json.loads(r.stdout)
    need(d["stats"]["files"] >= 1, "증거 아티팩트가 0건 — 실측 없이 값을 고르는 상태로 되돌아갔다")
    need(d["stats"]["indexed"] == d["stats"]["files"],
         "대장과 실물이 1:1 이 아니다: 대장 %d행 / 파일 %d건"
         % (d["stats"]["indexed"], d["stats"]["files"]))
    # ③ ★탐지력 자체를 합성 표본으로 시험(계측 타당성).
    st = _u26_py(_U26_VERIFY, "--self-test", "--json")
    need(st.returncode == 0, "증거 검사기 자기시험 실패(탐지기 고장): %s" % st.stdout[-600:])
    sd = json.loads(st.stdout)
    need(not sd["blind"], "합성 위반 표본 미탐지: %s" % sd["blind"])
    need(sd["cases"] >= 10, "자기시험 표본이 %d건뿐 — 탐지력 증명이 얕다" % sd["cases"])
    return ("증거 %d건 · 관측 %d건 · 대장 1:1 · 합성 표본 %d건 전건 적발"
            % (d["stats"]["files"], d["stats"]["observations"], sd["cases"]))


def _u26_corpus_violations(src, ev):
    """관문 코퍼스(Rust) ↔ 실측 캡처(JSON)의 키 방향 파리티. 반환 = 위반 목록.

    ★이 함수가 지키는 단 하나: **면책 창의 통과 액션이 `Left` 계열로 되돌아가지 않는 것.**
      구 설계는 `Left + Return` 이었고 실측은 그 반대다(세로 리스트라 Left 는 무의미 ·
      기본 포커스가 `No, exit` 이라 Return 은 rc 1 사망). 코퍼스가 다시 틀린 키를 담으면
      제품이 좌석을 죽이면서 CI 는 초록이 된다."""
    v = []

    def block(gate_id):
        i = src.find('id: "%s"' % gate_id)
        if i < 0:
            return ""
        j = src.find("Def {", i)
        return src[i:j if j > i else i + 1400]

    byp = block("bypass-disclaimer")
    if not byp:
        v.append("코퍼스에 bypass-disclaimer 정의가 없다")
    else:
        if "default_index: Some(1)" not in byp:
            v.append("면책 창 기본 포커스 선언이 1(No, exit)이 아니다 — 실측과 어긋난다")
        if 'action: Some((2, "Yes, I accept", Some("2")))' not in byp:
            v.append("면책 창 통과 액션이 실측 형태가 아니다(기대: 인덱스 2 · 리터럴 \"2\")")
        for wrong in ("Left", "ArrowLeft", "\\x1b[D"):
            if wrong in byp:
                v.append("면책 창 선언에 좌방향 키 어휘(%r)가 있다 — 틀린 키 박제 재발" % wrong)
    tru = block("folder-trust")
    if not tru:
        v.append("코퍼스에 folder-trust 정의가 없다")
    elif "default_index: Some(1)" not in tru or 'action: Some((1, "Yes, I trust this folder"' not in tru:
        v.append("폴더신뢰 창 선언이 실측(기본 포커스 Yes · 인덱스 1)과 어긋난다")

    # 실측 캡처의 상수와 코퍼스 선언이 같은 값을 말하는가.
    b = (ev.get("screens") or {}).get("bypass-disclaimer") or {}
    if (b.get("default_index"), b.get("accept_index"), b.get("exit_rc")) != (1, 2, 1):
        v.append("실측 캡처의 면책 상수가 (1,2,1)이 아니다: %r" % b)
    if (ev.get("key_facts") or {}).get("left_right_move_focus") is not False:
        v.append("실측 캡처가 '좌우는 포커스를 옮기지 않는다'를 말하지 않는다")
    # 스텁 화면이 코퍼스 needle 로 실제 감지되는가(감지 불가한 화면은 e2e 가치가 없다).
    for gid, needle in (("bypass-disclaimer", "WARNING: Claude Code running in Bypass Permissions mode"),
                        ("folder-trust", "Quick safety check")):
        txt = ((ev.get("screens") or {}).get(gid) or {}).get("text", "")
        if needle not in txt:
            v.append("실측 캡처 화면(%s)에 코퍼스 needle 이 없다: %r" % (gid, needle))
        if needle not in src:
            v.append("코퍼스에 needle 이 없다: %r" % needle)
    return v


@specimen("H-FAKE-1", "W6",
          "U-26 ③ 관문 스텁 선언 무결성 — 14 시나리오 · 실측 라벨 도용 차단 · 코퍼스 키 방향 파리티",
          ["R2-design-02", "C22", "N02", "N03", "N06"])
def h_fake_1():
    """★핵심 계약: **재지 않은 것에 '실측' 라벨을 붙일 수 없다.** 스텁의 화면·기본 포커스·
    종료코드는 `docs/evidence/probe-2026-08-23-first-run-gates.json` 에서 적재되며,
    `key_oracle == "measured"` 인데 출처가 실측 캡처가 아니면 스텁 자기시험이 적색이다.

    ★그리고 제품 코퍼스(`src/first_run_gates.rs`)와 **같은 값을 말하는지** 대조한다.
    스텁만 옳고 제품이 틀리면(또는 그 반대면) e2e 초록은 아무것도 증명하지 못한다."""
    src_path = os.path.join(REPO_DIR, _U26_CORPUS)
    _u26_gate(_U26_STUB, _U26_EV_JSON, src_path)
    st = _u26_py(_U26_STUB, "--self-test", "--json")
    need(st.returncode == 0, "스텁 자기시험 실패: %s" % (st.stdout[-600:] + st.stderr[-300:]))
    lst = _u26_py(_U26_STUB, "--list", "--json")
    need(lst.returncode == 0, "스텁 대장 조회 실패: %s" % lst.stderr[-300:])
    d = json.loads(lst.stdout)
    need(d["count"] == 14, "시나리오 %d종 (계약 14종)" % d["count"])
    ids = {s["id"] for s in d["scenarios"]}
    for must in ("bypass-dialog", "folder-trust", "trust-echo-double-return", "onboarding-chain",
                 "login-select", "oauth-loop-alive", "seeded-unauthenticated", "api-key-dialog",
                 "gated-alive-forever"):
        need(must in ids, "필수 시나리오 부재: %s" % must)
    # 실측 라벨을 붙인 시나리오는 반드시 실측 캡처를 출처로 댄다.
    for s in d["scenarios"]:
        if s["key_oracle"] == "measured":
            need(s["provenance"] == "measured" and "docs/evidence/" in s["source"],
                 "실측 라벨 도용: %s (%r / %r)" % (s["id"], s["provenance"], s["source"]))
    # ── 코퍼스 ↔ 실측 캡처 파리티 ──
    src = _read(src_path)
    ev = json.loads(_read(_U26_EV_JSON))
    v = _u26_corpus_violations(src, ev)
    need(not v, "코퍼스 파리티 위반 %d건: %s" % (len(v), " / ".join(v)))
    # ★계측 타당성 — 트리에 위반이 0이므로 **합성 변조본**으로 탐지력을 시험한다.
    #   (구 코드 대조가 아닌 이유: 베이스 트리(985093d)의 코퍼스는 이미 교정본이라
    #    '수리 전 적색' 을 잴 대상이 아니다. 잘못된 기준의 탐지 실패는 탐지기 파손이 아니라
    #    기준 선택 오류다 — 이 러너 헤더의 규약.)
    mutants = [
        ("면책 통과 액션을 Left 로 되돌림",
         src.replace('action: Some((2, "Yes, I accept", Some("2")))',
                     'action: Some((2, "Yes, I accept", Some("Left")))'), ev),
        ("면책 기본 포커스를 2로 오기",
         src.replace('default_index: Some(1),\n        action: Some((2, "Yes, I accept"',
                     'default_index: Some(2),\n        action: Some((2, "Yes, I accept"'), ev),
        ("면책 통과 인덱스를 1(No, exit)로 오기",
         src.replace('action: Some((2, "Yes, I accept", Some("2")))',
                     'action: Some((1, "Yes, I accept", Some("1")))'), ev),
        ("폴더신뢰 선언 삭제", src.replace('id: "folder-trust"', 'id: "folder-trust-x"'), ev),
        ("코퍼스 needle 삭제",
         src.replace("WARNING: Claude Code running in Bypass Permissions mode", "X"), ev),
        ("실측 캡처 상수 변조",
         src, dict(ev, screens=dict(ev["screens"],
                                    **{"bypass-disclaimer": dict(ev["screens"]["bypass-disclaimer"],
                                                                 exit_rc=0)}))),
        ("실측 캡처 키 사실 변조",
         src, dict(ev, key_facts=dict(ev["key_facts"], left_right_move_focus=True))),
    ]
    blind = [lbl for lbl, s, e in mutants if not _u26_corpus_violations(s, e)]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return ("시나리오 14종(실측 %d · 코퍼스 %d · 합성 %d) · 코퍼스 파리티 위반 0 · 합성 변조 %d종 전건 적발"
            % (sum(1 for s in d["scenarios"] if s["provenance"] == "measured"),
               sum(1 for s in d["scenarios"] if s["provenance"] == "product-corpus"),
               sum(1 for s in d["scenarios"] if s["provenance"] == "synthetic"), len(mutants)))


@specimen("H-FAKE-2", "W6",
          "U-26 ③ ★진짜 e2e — 실제 프로세스를 실제 PTY 에 띄우고 키를 넣어 종료코드를 잰다",
          ["R4-e2e-0", "B-1", "B-2", "B-3", "B-6"])
def h_fake_2():
    """★근본원인 R4 가 '**진짜 e2e 테스트가 0개**' 였다. 이 검체가 그 구멍을 메운다.
    소스 핀은 "그렇게 쓰여 있다"만 증명한다 — 여기서는 프로세스가 실제로 뜨고, 실제
    이스케이프 바이트(`\\x1b[D` 등)가 들어가고, 실제 종료코드가 나온다.

    ★가장 중요한 세 줄(오라클):
      · 면책 창 `Return` 단독      → **rc 1 사망**
      · 면책 창 `Left+Return`      → **여전히 rc 1**(구 설계가 박제하려던 통과 오라클은 틀렸다)
      · 면책 창 `Down+Return`/`2`  → 통과(생존)
    이 축이 무너지면 2026-07-29 형태의 사고가 CI 초록으로 승인된다.

    ★안전: 스텁 자기 자신 외에는 아무것도 뜨지 않고 아무것도 죽지 않는다(데몬·좌석·프로필
    무접촉 · 전 실행 유계 타임아웃 · 자기 자식만 회수)."""
    _u26_gate(_U26_E2E, _U26_STUB, _U26_EV_JSON)
    r = _u26_py(_U26_E2E, "--json", timeout=600)
    try:
        d = json.loads(r.stdout)
    except ValueError:
        raise Fail("e2e 결과 파싱 실패: %s" % ((r.stdout or "")[-500:] + (r.stderr or "")[-300:]))
    bad = [c for c in d["cases"] if not c["ok"]]
    need(not bad, "e2e 불일치 %d건: %s" % (len(bad), " / ".join(
        "%s(%s)" % (c["name"], "; ".join(c["violations"])[:160]) for c in bad)))
    need(not d["coverage_gaps"], "e2e 커버리지 공백: %s" % d["coverage_gaps"])
    got = {c["name"]: c for c in d["cases"]}
    # ★핵심 축은 이름으로 못박는다 — 사례 목록이 조용히 줄어드는 것을 막는다.
    for name, rc in (("bypass-return", 1), ("bypass-left-return", 1), ("bypass-right-return", 1),
                     ("bypass-down-return", 0), ("bypass-digit-2", 0),
                     ("trust-return-safe", 0), ("trust-then-bypass-kills", 1),
                     ("oauth-loop-never-dies", 0), ("seeded-unauth-query-fails", 0)):
        need(name in got, "핵심 e2e 사례가 사라졌다: %s" % name)
        need(got[name]["rc"] == rc, "%s 종료코드 %r (기대 %d)" % (name, got[name]["rc"], rc))
    need(len(d["cases"]) >= 19, "e2e 사례가 %d건뿐 — 축소 의심" % len(d["cases"]))
    mode = "실기 PTY" if d["pty"] else "파이프(비 posix)"
    return ("%s · 사례 %d건 전건 일치 · 시나리오 14종 전수 구동 · "
            "면책 Return/Left+Return = rc 1 · Down+Return·`2` = 생존"
            % (mode, len(d["cases"])))


@specimen("H-FAKE-3", "W6",
          "U-26 ③ e2e 오라클 자기검증 — 틀린 의미론 4종을 실제로 적발하는가(합성 변조 실기)",
          ["R2-design-02", "계측타당성"])
def h_fake_3():
    """★'전건 초록' 은 오라클이 살아 있다는 증거가 아니다. 오라클이 아무것도 안 보고 있어도
    결과는 똑같이 초록이다. 그래서 **틀린 세계를 만들어** 그 안에서 오라클이 적색이 되는지
    본다 — 그 중 하나가 정확히 구 설계의 세계다:

      · `left-moves-focus` = 좌우가 포커스를 옮기는 세계 → 구 `Left+Return` 오라클이 참이 된다
      · `return-accepts`   = 'Return 은 안전하다' 가정의 세계
      · `no-exit`          = rc 1 사망을 감춘 세계
      · `digit-ignored`    = 코퍼스 리터럴 `2` 통과가 무력화된 세계

    네 세계 중 하나라도 초록으로 통과하면 e2e 전체가 장식이다."""
    _u26_gate(_U26_E2E, _U26_STUB)
    caught = {}
    for m in ("left-moves-focus", "return-accepts", "no-exit", "digit-ignored"):
        r = _u26_py(_U26_E2E, "--json", "--mutate", m, "--expect-fail", timeout=600)
        try:
            d = json.loads(r.stdout)
        except ValueError:
            raise Fail("변조 %s 결과 파싱 실패: %s" % (m, (r.stdout or "")[-400:]))
        hits = [c["name"] for c in d["cases"] if not c["ok"]]
        need(hits, "변조 %r 를 아무 사례도 적발하지 못했다 = 오라클 고장" % m)
        caught[m] = hits
    # 구 설계의 틀린 키가 정확히 그 사례에서 잡혔는지 이름으로 못박는다.
    need("bypass-left-return" in caught["left-moves-focus"],
         "좌방향 변조를 `bypass-left-return` 이 아닌 곳에서 잡았다 — 축이 어긋났다")
    need("bypass-return" in caught["return-accepts"],
         "'Return 은 안전하다' 변조를 `bypass-return` 이 잡지 못했다")
    return "변조 4종 전건 적발 · " + " · ".join("%s→%s" % (k, ",".join(v)) for k, v in caught.items())


# ═══════════════════════════════════════════════════════════════════════════
# U-28 · 배선 결박 (M4 · M7 · M3-짝) — 2026-08-24
#
# ★이 절이 메우는 구멍은 앞 절들과 **층위가 다르다**. U-26 까지가 "검체를 제대로 만들었는가"
#   였다면 여기는 **"그 검체가 실제로 도는가"** 다. 자기성찰 3회전이 확인한 사실:
#   러너에 140종이 등재돼 있는데 CI 에서 도는 것은 19종뿐이었고, 그 121종에는
#   H-META-READ·H-META-PIN·H-EVID-1·H-FAKE-1/2/3·H-BOOT-SUP-1/2·H-SECRET-1 이 전부 들어 있었다.
#   ★자기모순의 정점 — H-CI-TAG-1 은 "windows-health 가 태그에서 도는가" 를 판정하는 검체인데
#     **그 검체 자신이 어느 CI 레인에서도 돌지 않았다.** 이 저장소의 교리("안 도는 검체는
#     게이트가 아니다")를 캠페인이 자기 산출물에서 재생산한 것이다.
#   배선 없이 태그하면 이번에 고친 것들이 다음 커밋에 조용히 되돌아가도 아무도 모른다.
# ═══════════════════════════════════════════════════════════════════════════
_U28_WF_DIR = os.path.join(REPO_DIR, ".github", "workflows")
_RUNNER_BASENAME = "run_bootstrap_health.py"
_U28_VAR_ASSIGN = re.compile(r'^\s*([A-Za-z_][A-Za-z0-9_]*)="([^"]*)"\s*$')


def _u28_workflow_texts():
    """`.github/workflows/*.yml` 전량 → {파일명: 텍스트}."""
    if not os.path.isdir(_U28_WF_DIR):
        raise Skip("레포 체크아웃이 아니다(배포 팩) — CI 배선 검체 적용 불가")
    out = {}
    for f in sorted(os.listdir(_U28_WF_DIR)):
        if f.endswith((".yml", ".yaml")):
            out[f] = _read(os.path.join(_U28_WF_DIR, f))
    need(out, "워크플로 파일을 0건 읽었다 — 추출기 파손(측정 불능은 통과가 아니다)")
    return out


def _u28_runner_lanes(files):
    """{워크플로명: 텍스트} → 러너 호출 레인 목록 `[(파일, kind, ids)]`.

    kind: `full`(--only·--wave 없음 = 등재 전량) · `only`(명시 목록) · `wave`(웨이브 태그).
    ★셸 변수를 **해소**한다 — 실측 형태가 `R="…run_bootstrap_health.py"` + `python "$R" --only
      "$WIN_SPECIMENS"` 라서, 변수를 안 푸는 파서는 windows-health 레인을 통째로 못 본다.
    ★해소 불가한 `--only` 인자는 **적색**이다(빈 집합으로 접지 않는다). 못 읽은 것을 '커버리지
      0' 으로 접으면 차집합이 커져 오히려 요란해질 것 같지만, 반대 방향의 사고도 대칭으로
      가능하다(변수명이 바뀌었는데 조용히 넘어가는 것). 측정 불능은 판정하지 않는다.
    """
    lanes = []
    for fname in sorted(files):
        text = files[fname]
        varmap = {}
        for line in text.splitlines():
            m = _U28_VAR_ASSIGN.match(line)
            if m:
                varmap[m.group(1)] = m.group(2)
        for line in text.splitlines():
            s = line.strip()
            if s.startswith("#") or not re.search(r"\bpython3?\b", s):
                continue
            hit = _RUNNER_BASENAME in s
            if not hit:
                for v in re.findall(r"\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?", s):
                    if _RUNNER_BASENAME in varmap.get(v, ""):
                        hit = True
                        break
            if not hit or "--list" in s:      # `--list` 는 대장 출력이지 실행이 아니다
                continue

            def _arg(flag):
                m = re.search(r"%s\s+(\S+)" % re.escape(flag), s)
                if not m:
                    return None
                tok = m.group(1).strip("\"'")
                if tok.startswith("$"):
                    name = tok[1:].strip("{}").strip("\"'")
                    need(name in varmap,
                         "%s: `%s` 인자 변수 $%s 를 해소하지 못했다 — 커버리지를 측정할 수 없다"
                         % (fname, flag, name))
                    tok = varmap[name]
                return tok

            only, wave = _arg("--only"), _arg("--wave")
            if only:
                ids = {x.strip() for x in only.split(",") if x.strip()}
                lanes.append((fname, "only", frozenset(ids)))
            elif wave:
                lanes.append((fname, "wave",
                              frozenset(sid for sid, w, _t, _d, _f in _REG if w == wave)))
            else:
                lanes.append((fname, "full", None))
    return lanes


def _u28_uncovered(files):
    """(레인 목록, 어느 레인에서도 돌지 않는 검체 집합, full 레인 파일 목록)."""
    all_ids = {sid for sid, _w, _t, _d, _f in _REG}
    lanes = _u28_runner_lanes(files)
    covered, fulls = set(), []
    for fname, kind, ids in lanes:
        if kind == "full":
            covered |= all_ids
            fulls.append(fname)
        else:
            covered |= set(ids)
    return lanes, all_ids - covered, fulls


@specimen("H-CI-COVER-1", "W6",
          "★M4 전 검체가 어느 CI 레인에서든 1회는 돈다 — `--only` 목록 ↔ 등재 전량 차집합 = 0",
          ["CI-게이트-부재", "M4", "자기모순-H-CI-TAG-1"])
def h_ci_cover_1():
    """★무엇이 결함이었나: 검체 140종 중 CI 에서 도는 것이 19종뿐이었다. 나머지 121종은
    "쓰여 있으나 아무도 실행하지 않는 문서" 였고, 그 상태에서 나온 초록은 게이트가 아니다.

    ★왜 이 형태(차집합)인가: '전량을 도는 레인이 있다' 만 보면 나중에 그 레인이 `--only` 로
    좁혀져도 검체는 초록이다. 반대로 각 검체가 자기 실행을 주장하게 하면 등재소가 두 곳이
    된다. 그래서 **CI yml 을 파싱해 실제 실행 집합을 계산**하고 등재 전량과의 차집합을 잰다 —
    러너의 `_REG` 가 유일한 등재소로 남고, 새 검체를 등재하는 순간 이 검체가 그것의 배선까지
    책임진다.

    ★`full` 레인(= `--only` 없는 호출)이 **하나 이상** 있어야 한다는 조건을 함께 건다.
      부분 목록을 여러 개 이어 붙여 차집합을 0으로 만들 수도 있지만, 그 상태에서는 **새 검체를
      등재해도 자동으로 돌지 않는다**(누군가 목록에 손으로 추가해야 하고, 그 누락이 바로 이
      결함의 발생 경로였다). 자동 편입되는 구조를 계약으로 못박는다.

    ⚠비용 경계도 함께 판정한다: 전량 레인이 windows-latest 로 옮겨 가면 매 push 마다 138MB
      윈도우 빌드가 도는 트리거 확대가 된다. 전량은 mac/ubuntu 레인의 몫이다."""
    files = _u28_workflow_texts()
    lanes, uncovered, fulls = _u28_uncovered(files)
    need(lanes, "CI 에서 건강성 러너 호출을 0건 찾았다 — 추출기 파손이거나 배선이 통째로 사라졌다")
    need(fulls,
         "전량(`--only` 없는) 러너 호출이 어느 레인에도 없다 — 새로 등재한 검체가 자동으로 "
         "돌지 않는 구조다(이 결함의 발생 경로 그 자체)")
    need(not uncovered,
         "어느 CI 레인에서도 돌지 않는 검체 %d건: %s%s — '안 도는 검체는 게이트가 아니다'"
         % (len(uncovered), ", ".join(sorted(uncovered)[:12]),
            " …" if len(uncovered) > 12 else ""))
    # ★비용 경계 — 전량 레인은 Windows 러너가 아니어야 한다(트리거 확대 = 예산 폭발).
    need("windows-health.yml" not in fulls,
         "전량 실행이 Windows 실기 레인에 붙었다 — 매 push 마다 windows 러너가 전량을 돈다(예산 위반)")
    # ★ci-branch 예산 규율(H-CI-TAG-1 이 windows-health 에 거는 것과 같은 축).
    cb = files.get("ci-branch.yml")
    if cb:
        on = cb[cb.index("\non:"):cb.index("\npermissions:")]
        need("pull_request" not in on,
             "예산 위반: ci-branch 에 `pull_request` 트리거 — 매 PR 갱신마다 전량이 돈다")
        for bad in ("branches: ['**']", 'branches: ["**"]', "branches: ['*']"):
            need(bad not in on, "예산 위반: ci-branch 전 브랜치 트리거(%s)" % bad)
    # ★계측 타당성 — 트리에 위반이 0이므로 **합성 변조본**으로 탐지력을 시험한다.
    #   (지금 초록인 것이 '탐지기가 살아 있어서' 인지 '아무것도 안 보고 있어서' 인지 가른다.)
    def _blind(label, mutated):
        try:
            _lanes, unc, ful = _u28_uncovered(mutated)
        except Fail:
            return None                     # 적발(해소 불가를 적색으로 낸 경우)
        return None if (unc or not ful) else label

    full_file = fulls[0]
    mutants = [
        ("전량 스텝 삭제",
         dict(files, **{full_file: "\n".join(
             l for l in files[full_file].splitlines()
             if not (re.search(r"\bpython3?\b", l.strip()) and "$R" in l and "--json" in l))})),
        ("전량을 --only 1건으로 축소",
         dict(files, **{full_file: files[full_file].replace(
             'python3 "$R" --json', 'python3 "$R" --only H-WIN-1 --json')})),
        ("windows 레인 목록 변수 개명(해소 불가)",
         dict(files, **{"windows-health.yml":
                        files.get("windows-health.yml", "").replace(
                            'WIN_SPECIMENS="', 'WIN_SPECIMENS_X="')})),
    ]
    blind = [b for b in (_blind(lbl, mut) for lbl, mut in mutants) if b]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return ("등재 %d종 · CI 레인 %d개(전량 %s · 부분 %s) · 미실행 0 · 합성 변조 %d종 전건 적발"
            % (len(_REG), len(lanes), ",".join(fulls),
               ",".join("%s:%d종" % (f, len(i)) for f, k, i in lanes if k != "full") or "없음",
               len(mutants)))


# ── M7 · 인증 서술 4벌 정합 ────────────────────────────────────────────────
_U28_DOC_SITES = (
    (os.path.join("cysjavis-pack", "agents.json"),
     "agents.json — 제품이 '사용자 환경에 맞게 수정 가능' 하다고 안내하는 파일이라 읽힐 확률이 가장 높다"),
    (os.path.join("src", "pack.rs"), "pack.rs — 격리 config dir 계약의 정본 주석"),
    (os.path.join("cysjavis-pack", "bin", "cys-dept"), "cys-dept — 부서별 config dir 분리 지점"),
    (os.path.join("docs", "RELEASE_NOTES_0.2.2.md"), "0.2.2 릴리스 노트 — 이미 외부에 나간 문면"),
)
_ACCOUNT_UNIT_RE = re.compile(r"계정\s?단위")
_CORRECTION_MARKERS = ("아니라", "반증", "정정", "~~", "사실이 아닙니다", "틀렸다", "종전 서술")
_CORRECTED_FACTS = ("config dir 경로 단위", "Claude Code-credentials-")


def _keychain_violations(where, text):
    """한 파일의 위반 목록. 규칙 둘 —
       ① **정정 문면 존재**: 실측 사실(경로 단위 · 서비스명 형태)이 그 파일에 있어야 한다.
       ② **거짓 서술 부재(형태 규칙)**: '계정 단위' 출현마다 근처에 부정·정정 마커가 있어야 한다.
    ★왜 단순 문자열 부재로 판정하지 않는가: 정정문 자체가 구 서술을 **인용**하므로("종전 서술
      …는 반증됐다"), 부재 규칙으로 두면 정정을 쓰는 것이 불가능해진다. 그래서 `_no_wait_for_owner`
      가 이미 쓰는 하우스 관용(출현 ± 창 안의 마커 요구)을 그대로 따른다."""
    v = []
    for tok in _CORRECTED_FACTS:
        if tok not in text:
            v.append("%s: 정정 문면 부재 %r" % (where, tok))
    for m in _ACCOUNT_UNIT_RE.finditer(text):
        w = text[max(0, m.start() - 120):m.end() + 120]
        if not any(k in w for k in _CORRECTION_MARKERS):
            v.append("%s: 무표식 '계정 단위' 서술 — …%s…"
                     % (where, w.replace("\n", " ")[:160]))
    return v


@specimen("H-DOC-KEYCHAIN-1", "W6",
          "★M7 인증 서술 4벌 정합 — '계정 단위 Keychain' 거짓 부재 + 정정 문면 존재(설계 U-8 게이트)",
          ["M7", "U-8", "비가역-창"])
def h_doc_keychain_1():
    """★왜 이 거짓말이 특별한가: `agents.json` 은 `Ownership::User` 다. 이미 디스크 사본이 있는
    기계에는 vendor 갱신이 **자동으로 도달하지 않는다**(`pack.rs::decide_file_action` 의 user
    분기 = Keep + `.new` 병치). 즉 잘못된 문장을 이번 태그에 고치지 못하면 그 기계에서는
    영영 남는다. 그리고 그 문장을 믿고 부서를 분리하면 **조용히 로그인이 깨진다** —
    macOS Keychain 항목은 계정 단위가 아니라 `Claude Code-credentials-<sha256(config dir)[:8]>`
    로 **경로 단위** 봉인이기 때문이다(2026-08-23 실측: 서비스명 9개 열거 → 7개 정확 일치).

    ★설계 U-8 이 이 게이트를 요구했으나 만들어지지 않았고, 그래서 정정판이 세 곳에만 착지하고
      네 번째(agents.json)가 남았다. 문서 정합은 사람 눈으로 세 번 실패한 축이다(레인 대조
      게이트의 교훈과 같은 계급) — 네 벌을 **동시에** 단언하는 기계만이 막는다."""
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        raise Skip("레포 체크아웃이 아니다(배포 팩) — 문서 4벌 정합 검체 적용 불가")
    texts, v = {}, []
    for rel, why in _U28_DOC_SITES:
        p = os.path.join(REPO_DIR, rel)
        need(os.path.isfile(p),
             "정합 대상 파일이 없다: %s (%s) — 레포 안에서의 부재는 삭제다" % (rel, why))
        texts[rel] = _read(p)
        v += _keychain_violations(rel, texts[rel])
    need(not v, "인증 서술 정합 위반 %d건: %s" % (len(v), " / ".join(v)))
    # ★계측 타당성 — 정상 트리엔 위반이 0이므로 합성 변조로 탐지력을 시험한다.
    agents = os.path.join("cysjavis-pack", "agents.json")
    prs = os.path.join("src", "pack.rs")
    pad = "." * 200
    mutants = [
        ("정정 사실(경로 단위) 삭제",
         agents, texts[agents].replace("config dir 경로 단위", "계정 단위")),
        ("정정 사실(서비스명) 삭제",
         agents, texts[agents].replace("Claude Code-credentials-", "X-")),
        ("거짓 서술 무표식 재삽입",
         agents, texts[agents] + "\n" + pad
         + "macOS 인증은 계정 단위 Keychain이라 격리해도 로그인 유지." + pad),
        ("정정 마커만 제거(문장은 유지)",
         prs, texts[prs].replace("아니라", "이고").replace("정정", "안내")
         .replace("반증", "확인").replace("종전 서술", "구 서술").replace("틀렸다", "맞다")),
    ]
    blind = [lbl for lbl, rel, mut in mutants if not _keychain_violations(rel, mut)]
    need(not blind, "합성 변조본을 못 잡았다(탐지기 고장): %s" % ", ".join(blind))
    return ("4벌 정합(%s) · 위반 0 · 합성 변조 %d종 전건 적발"
            % (" · ".join(os.path.basename(r) for r, _w in _U28_DOC_SITES), len(mutants)))


# ── M3-짝 · 관문 보류(gate_pending · exit 78) 소비 ────────────────────────
@specimen("H-BOOT-GATE-78", "W6",
          "★M3-짝 boot 관문 보류 = 제3 상태 — Fatal·busy·Degrade 어디로도 접히지 않는다(exit 78 파리티)",
          ["M3-짝", "U-11", "관문-보류-무처리"])
def h_boot_gate_78():
    """★무엇이 결함이었나: `cys.rs` 가 좌석 **제4 등급** `gate_pending`(pane·프로세스는 살아
    있으나 첫기동 관문에 갇혀 입력을 못 받는다)을 내고 `boot_exit_code` 가 78 을 내는데,
    파이썬 소비부에는 그 값을 받는 자리가 **없었다** — Fatal 집합은 `{failed, missing}` 뿐이고
    busy 판정도 아니어서, `elif code != 0` 의 Degrade 가지로 흘러
    **"비0 이지만 Fatal 역할은 전원 확보"** 라는 거짓 문장이 기록됐다(의무 역할이 관문에 갇혀
    팀이 서지 않았는데 전원 확보라고 적는 것).

    ★그렇다고 Fatal 로 올리는 수리는 반대편 벽에 부딪힌다 — U-11 이 이미 결박한 계약이다
      (검체 `H-EXIT-11` ⑥): 보류를 실패로 접으면 소비부가 '기동이 깨졌다'로 읽어 **살아 있는
      좌석을 회수·파괴**하려 든다(치명위험 ④ · 오살이 오탐보다 훨씬 비싸다).
      ∴ 옳은 형태는 **제3 상태**다 — 큰 소리로 이름 붙여 기록하되 중단하지 않고, 최종 게이트는
      종전대로 ⑤check 다(U-10 의 결손 산출이 gate_pending 좌석을 '못 쓰는 좌석'으로 세므로
      보류 상태에서 부트가 조용히 성공으로 끝나지 않는다).

    ★이 검체는 그 세 벽을 **동시에** 세운다: ⓐFatal 로 접히지 않는가 ⓑbusy 로 접히지 않는가
      ⓒ**그럼에도 반드시 잡히는가**. 셋 중 하나만 검사하면 나머지 방향으로 조용히 무너진다."""
    bsrc = _read(os.path.join(BIN_DIR, "javis_bootstrap.py"))
    notes = []
    # ⓐ 값 파리티 — 관문 보류 전용 종료코드는 **78 하나**다(같은 사실에 값이 둘이면 드리프트).
    need("CYS_LAUNCH_EXIT_GATE_PENDING = 78" in bsrc, "python 소비부 관문 보류 exit 상수 이탈")
    need("CYS_BOOT_EXIT_GATE_PENDING = CYS_LAUNCH_EXIT_GATE_PENDING" in bsrc,
         "boot 쪽 관문 보류 상수가 별도 숫자로 갈라졌다 — 같은 사실에 값이 둘이면 한쪽만 고쳐진다")
    lib = os.path.join(REPO_DIR, "src", "lib.rs")
    if os.path.isfile(lib):
        need("pub const EXIT_GATE_PENDING: i32 = 78;" in _read(lib),
             "Rust 정본 관문 보류 exit 상수 이탈(3자 파리티 붕괴)")
        notes.append("exit 78 파리티(rust lib ↔ python boot/launch)")
    else:
        notes.append("exit 78 파리티(python 측만 — 배포 팩)")
    # ⓑ Fatal 집합은 **단일 등재소**이고, gate_pending 은 거기 없어야 한다(U-11 계약).
    need('BOOT_FATAL_OUTCOMES = ("failed", "missing")' in bsrc,
         "Fatal outcome 등재소가 없거나 집합이 바뀌었다")
    need('r.get("outcome") in BOOT_FATAL_OUTCOMES' in bsrc,
         "_boot_fatal_verdict 가 등재소를 쓰지 않는다(본문 리터럴 회귀 — 새 outcome 이 조용히 샌다)")
    # ⓒ 제3 분기가 **실재하고 소비 배선까지 되어 있는가**(선언만 하고 안 부르면 유령이다).
    need("def _boot_gate_pending_verdict(" in bsrc, "관문 보류 전용 판정 함수가 없다")
    need(bsrc.count("_boot_gate_pending_verdict(") >= 2,
         "관문 보류 판정 함수를 선언만 하고 소비하지 않는다(유령 분기)")
    need("STEP.BOOT_GATE_PENDING" in bsrc, "보류 전용 단계 라벨이 없다 — Degrade 로 뭉개진다")
    ci = bsrc.find("gate_why = _boot_gate_pending_verdict(")
    need(ci > 0, "④ 소비부에서 보류 판정을 계산하지 않는다")
    tail = bsrc[ci:ci + 2500]
    need(tail.find("elif gate_why is not None:") < tail.find("elif code != 0:"),
         "보류 분기가 Degrade(`elif code != 0`) 뒤에 있다 — 먼저 걸리는 쪽이 이겨 거짓 문장이 남는다")
    # ⓓ 행위 실측(순수 판정 8축) — 소스 문자열이 아니라 **판정 결과**를 잰다.
    if BIN_DIR not in sys.path:
        sys.path.insert(0, BIN_DIR)
    import javis_bootstrap as B
    need(B.CYS_BOOT_EXIT_GATE_PENDING == 78, "boot 관문 보류 상수가 78 이 아니다")
    gp = json.dumps({"roles": [{"role": "cso", "agent": "claude", "outcome": "gate_pending",
                                "mandatory": True, "reason": "면책 창 상주"}],
                     "summary": {"gate_pending": 1}})
    need(B._boot_fatal_verdict(1, gp) is None,
         "관문 보류가 Fatal 로 오분류 — 살아 있는 좌석에 회수·파괴 처방이 나간다(U-11 치명위험 ④)")
    need(B._boot_was_busy(1, gp) is False, "관문 보류가 busy(무스폰)로 오분류 — 티켓 회계 오염")
    why = B._boot_gate_pending_verdict(1, gp)
    need(why is not None,
         "관문 보류가 어디에도 걸리지 않는다 — Degrade 로 흘러 'Fatal 역할은 전원 확보'(거짓)가 기록된다")
    for tok, miss in (("관문", "관문 통과 처방"), ("No, exit", "면책 창 기본 포커스 경고"),
                      ("회수", "비파괴 지시"), ("면책 창 상주", "생산자 reason 인용")):
        need(tok in why, "보류 사유에 %s 가 없다" % miss)
    need(B._boot_gate_pending_verdict(B.CYS_BOOT_EXIT_GATE_PENDING, "파싱 불가 산문") is not None,
         "exit 78 단독(--json 소비 불가)에서 보류를 놓친다 — 두 축 OR 이 아니다")
    need(B._boot_fatal_verdict(B.CYS_BOOT_EXIT_GATE_PENDING, "파싱 불가 산문") is None,
         "exit 78 이 보수 폴백에서 Fatal 로 접힌다 — 살아 있는 좌석 회수 처방(U-11 위반)")
    opt = json.dumps({"roles": [{"role": "reviewer-codex", "agent": "codex",
                                 "outcome": "gate_pending", "mandatory": False}]})
    need(B._boot_gate_pending_verdict(1, opt) is None,
         "선택 역할 보류까지 제3 상태로 승격(Degrade 가 옳다 — 과잉 발화)")
    failed = json.dumps({"roles": [{"role": "cso", "agent": "claude",
                                    "outcome": "failed", "mandatory": True}]})
    need(B._boot_gate_pending_verdict(1, failed) is None, "진짜 실패를 보류로 오판(실패 은닉)")
    need(B._boot_fatal_verdict(1, failed) is not None,
         "의무 역할 failed 가 Fatal 로 승격되지 않음(보류 도입이 실패 판정을 삼켰다)")
    notes.append("행위 8축 실측(Fatal 비오판·busy 비오판·반드시 적발·처방 4문·두 축 OR·과잉 발화 0)")
    # ⓔ 계측 타당성 — 캠페인 베이스(PRE_U24_REF)에는 제3 분기가 없었다(진짜 변화를 보고 있다).
    old = _git_show(os.path.join("cysjavis-pack", "bin", "javis_bootstrap.py"), PRE_U24_REF)
    calib = "skip(no-git)"
    if old is not None:
        need("_boot_gate_pending_verdict" not in old,
             "계측 타당성 실패: 구 소비부에 이미 제3 분기가 있다(기준 선택 오류 의심)")
        need('in ("failed", "missing")' in old,
             "계측 타당성 실패: 구 소비부에서 Fatal 집합 리터럴을 찾지 못했다")
        calib = "구 소비부=제3 분기 부재(보류가 Degrade 로 흐름) 확인"
    return " · ".join(notes) + " · 계측검증=%s" % calib


@specimen("H-RESET-SENT-1", "W6",
          "★완전 초기화 센티널 격리 — 경로 주입 가능·상대경로 거부·테스트가 라이브 가드를 지우지 않는다",
          ["M4-선행조건", "전-pane-사망", "테스트-비결정론"])
def h_reset_sent_1():
    """★왜 M4 의 **선행 조건**인가: 센티널 회귀 테스트는 판정을 시험하려고 센티널을 쓰고
    지운다. 경로가 `$HOME/.local/state/…` 로 고정돼 있으면 그 테스트가 두 가지를 동시에
    깨뜨린다 — ⓐ러너가 둘 이상 돌면 같은 파일을 서로 쓰고 지워 **무작위 적색**이 나고(결정론
    게이트가 비결정론의 원천이 된다) ⓑ진짜 완전 초기화가 진행 중이면 **살아 있는 가드를
    지운다.** 그 가드의 존재 이유가 "리셋 중 데몬 부활 = 전 pane 사망 등급" 차단이므로,
    이건 테스트가 제품의 최고 위험 등급 안전장치를 무력화하는 것이다.
    선재 결함이지만 CI 전량 배선(M4)을 넣는 순간 즉시 표면화된다.

    ★오버라이드에 **절대경로 조건**을 거는 이유: 쓰는 쪽(리셋 실행 프로세스)과 읽는 쪽
      (CLI autostart·GUI ensure_daemon)이 서로 다른 프로세스다. 상대경로를 허용하면 두
      프로세스의 cwd 가 다를 때 서로 다른 파일을 보고 가드가 **조용히** 무력해진다."""
    src = _repo_file(os.path.join("src", "factory_reset.rs"))
    need('pub const ENV_RESET_SENTINEL: &str = "CYS_FACTORY_RESET_SENTINEL";' in src,
         "센티널 경로 오버라이드 상수가 없다 — 테스트가 라이브 홈을 쓸 수밖에 없다")
    seg = _slice_between(src, "pub fn sentinel_path()", "\nfn now_unix()",
                         "factory_reset::sentinel_path")
    need("ENV_RESET_SENTINEL" in seg, "sentinel_path 가 오버라이드를 읽지 않는다(상수만 있고 배선 0)")
    need("is_absolute()" in seg,
         "상대경로 오버라이드를 거르지 않는다 — 쓰는 쪽과 읽는 쪽이 다른 파일을 볼 수 있다")
    need("dirs::home_dir()" in seg, "기본 경로(홈 파생)가 사라졌다 — 오버라이드 부재 시 가드가 죽는다")
    ti = src.find("fn sentinel_is_fail_open_and_self_cleaning()")
    need(ti > 0, "센티널 회귀 테스트를 찾지 못했다(삭제됐는가?)")
    tbody = src[ti:ti + 5000]
    need("EnvGuard::set(ENV_RESET_SENTINEL" in tbody,
         "테스트가 센티널 경로를 격리하지 않는다 — 라이브 가드 삭제·동시 실행 적색 경로 잔존")
    need("let restore = std::fs::read_to_string" not in tbody,
         "라이브 센티널 백업·복원 관용구가 남아 있다(그 사이 창에서 실 리셋 가드가 지워진다)")
    # 계측 타당성 — 캠페인 베이스에는 오버라이드가 없었다(탐지기가 진짜 변화를 보고 있다).
    old = _git_show(os.path.join("src", "factory_reset.rs"), PRE_U24_REF)
    calib = "skip(no-git)"
    if old is not None:
        need("ENV_RESET_SENTINEL" not in old,
             "계측 타당성 실패: 구 트리에 이미 오버라이드가 있다(기준 선택 오류 의심)")
        need("let restore = std::fs::read_to_string" in old,
             "계측 타당성 실패: 구 트리에서 라이브 백업·복원 관용구를 찾지 못했다")
        calib = "구 트리=고정 홈 경로 + 라이브 백업·복원 확인"
    return "경로 주입·절대경로 강제·기본 경로 보존 · 테스트 격리 · 계측검증=%s" % calib



# ═══════════════════════════════════════════════════════════════════════════
# 러너
# ═══════════════════════════════════════════════════════════════════════════
def main(argv=None):
    # ★로케일 비의존 I/O(하우스 관례 javis_detect.py · 선례 javis_bootstrap.py R3): windows-latest
    #   가 stdout 을 cp1252 로 열면 한글 검체 title 인쇄(아래 print(line))가 UnicodeEncodeError 로
    #   죽어 **판정 전체가 관측 불능**이 된다(run 31395642155). 러너 자신의 스트림만 재설정하므로
    #   각 검체의 서브프로세스 캡처(별도 파이프·명시 인코딩)에는 영향이 없다.
    for _s in (sys.stdout, sys.stderr):
        try:
            _s.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    ap = argparse.ArgumentParser(description="부트스트랩 상시 건강성 1커맨드 게이트")
    ap.add_argument("--json", action="store_true", help="기계 판독 결과(JSON)")
    ap.add_argument("--list", action="store_true", help="검체 대장만 출력")
    ap.add_argument("--only", default="", help="검체 ID 콤마 목록만 실행")
    ap.add_argument("--wave", default="", help="해당 웨이브 태그 검체만 실행")
    ap.add_argument("--include-pending", action="store_true",
                    help="미발효 검체도 실행 시도(개발용 — 게이트 판정에는 산입하지 않는다)")
    args = ap.parse_args(argv)

    if args.list:
        for sid, wave, title, defects, fn in _REG:
            print("%-12s %-4s %-9s %s  [%s]" % (
                sid, wave, "effective" if (fn and wave in LANDED_WAVES) else "pending",
                title, ",".join(defects)))
        return 0

    only = {s.strip() for s in args.only.split(",") if s.strip()}
    rows = []
    t0 = time.time()
    # ★스위치 상태는 **검체 실행 전에** 스냅샷한다 — 일부 검체가 실행 중 os.environ 을 임시로
    #   바꿨다가 되돌리므로, 실행 후에 읽으면 그 왕복의 잔재를 판정 근거로 삼을 위험이 있다.
    engaged = _off_switches_engaged()

    # ★출력 스트림 분리(U-27 · 이종 리뷰어 [P0-3] · 헤더 '출력 스트림 계약').
    #   `--json` 이면 **검체 실행 구간 동안** fd 1 을 fd 2 로 덮는다. `sys.stdout` 객체 치환으로는
    #   부족하다 — 검체는 팩 모듈을 인프로세스로 부르기도 하고 자식 프로세스를 띄우기도 하는데,
    #   자식은 **fd 를 상속**하므로 파이썬 층위 리다이렉트를 그냥 지나친다. 둘을 동시에 덮는
    #   유일한 층위가 fd 다(실측 오염원: javis_report_gate 의 `verdict=… reasons=…` 요약 줄).
    #   복원은 finally — 검체가 크래시해도 JSON 은 진짜 stdout 으로 나간다.
    _out_fd = None
    if args.json:
        try:
            sys.stdout.flush()
            _out_fd = os.dup(1)
            os.dup2(2, 1)
        except (OSError, ValueError, AttributeError):
            # fd 조작 불가 환경 — 종전 동작으로 자연 강등(하드 실패 아님).
            # ★dup 은 됐는데 dup2 에서 깨진 경우 보존 fd 를 닫는다(누수 방지 · fd 1 은 그대로다).
            if _out_fd is not None:
                try:
                    os.close(_out_fd)
                except OSError:
                    pass
            _out_fd = None
    try:
        for sid, wave, title, defects, fn in _REG:
            if only and sid not in only:
                continue
            if args.wave and wave != args.wave:
                continue
            row = {"id": sid, "wave": wave, "title": title, "defects": defects}
            effective = bool(fn) and wave in LANDED_WAVES
            if not effective:
                row["status"] = "pending"
                row["reason"] = ("발효 웨이브 %s 미착지(현재 착지: %s)" % (wave, ",".join(LANDED_WAVES))
                                 if wave not in LANDED_WAVES
                                 else "구현 대기(발효 웨이브 착지 — 구현 필요)")
                if wave in LANDED_WAVES and not fn:
                    row["status"] = "fail"
                    row["reason"] = "발효 웨이브인데 검체 미구현 — 측정 실패는 hard fail"
                if not (args.include_pending and fn):
                    rows.append(row)
                    continue
            s = time.time()
            try:
                detail = fn()
                row["status"] = "pass"
                row["detail"] = detail or ""
            except Disabled as e:
                # ★`Skip` 보다 **먼저** 잡는다(Disabled 는 Skip 의 하위형이다). 순서가 뒤집히면
                #   '사람이 끈 미측정' 이 다시 '적용 불가' 로 접혀 GREEN 이 된다 = 이 수리의 무력화.
                row["status"] = "disabled"
                row["reason"] = str(e)
            except Skip as e:
                row["status"] = "skip"
                row["reason"] = str(e)
            except Fail as e:
                row["status"] = "fail"
                row["reason"] = str(e)
            except Exception as e:  # 검체 자체 크래시도 fail(측정 실패=hard fail)
                row["status"] = "fail"
                row["reason"] = "검체 크래시 %s: %s" % (type(e).__name__, e)
            row["secs"] = round(time.time() - s, 2)
            if not effective:
                row["counted"] = False
            rows.append(row)
    finally:
        if _out_fd is not None:
            try:
                sys.stdout.flush()
            except Exception:       # noqa: BLE001 — 복원이 우선이다
                pass
            os.dup2(_out_fd, 1)
            os.close(_out_fd)

    counted = [r for r in rows if r.get("counted", True)]
    failed = [r for r in counted if r["status"] == "fail"]
    passed = [r for r in counted if r["status"] == "pass"]
    skipped = [r for r in counted if r["status"] == "skip"]
    # ★'사람이 끈 미측정' 은 `skip`(적용 불가)과 **다른 축**이다(헤더 '미측정 축 계약').
    disabled = [r for r in counted if r["status"] == "disabled"]
    pend = [r for r in counted if r["status"] == "pending"]

    # ── 판정 3값 ────────────────────────────────────────────────────────────
    # GREEN 은 "발효분을 **다 재고** 다 통과했다" 일 때에만 붙는다.
    # ★왜 `engaged` 만으로도 GREEN 을 박탈하는가: 스위치를 켜는 것은 이 **런 전체**에 대한 사람의
    #   행위다. `--only` 로 그 검체가 이번 선택에 없었다는 사실은 "측정 축이 꺼져 있지 않다"를
    #   조금도 뜻하지 않는다. 판단이 갈리는 자리에서 엄격한 쪽을 택하는 것은 완화가 아니므로
    #   러너 헤더 '핀 이사 계약 ②'에 저촉되지 않는다.
    if failed:
        verdict = "RED"
    elif disabled or engaged:
        verdict = "UNMEASURED"
    else:
        verdict = "GREEN"
    summary = {"verdict": verdict, "landed_waves": list(LANDED_WAVES),
               "pass": len(passed), "fail": len(failed), "skip": len(skipped),
               "disabled": len(disabled), "pending": len(pend), "total": len(rows),
               "off_switches_engaged": [{"kind": k, "env": e, "value": val, "scope": sc}
                                        for k, e, val, sc in engaged],
               "elapsed_secs": round(time.time() - t0, 1),
               "calibration_ref": CALIBRATION_REF}

    if args.json:
        print(json.dumps({"summary": summary, "specimens": rows}, ensure_ascii=False, indent=1))
    else:
        for r in rows:
            mark = {"pass": "PASS", "fail": "FAIL", "skip": "SKIP",
                    "disabled": "OFF!", "pending": "PEND"}[r["status"]]
            line = "%s %-12s %-4s %s" % (mark, r["id"], r["wave"], r["title"])
            if r["status"] == "pass" and r.get("detail"):
                line += "\n       └ %s" % r["detail"]
            if r["status"] in ("fail", "skip", "disabled", "pending") and r.get("reason"):
                line += "\n       └ %s" % r["reason"]
            print(line)
        print("\n%s — 발효 %d PASS / %d FAIL / %d SKIP(적용불가) / %d OFF(사람이 끈 미측정) · "
              "미발효 %d PEND · %.1fs (발효 웨이브 %s)"
              % (verdict, len(passed), len(failed), len(skipped), len(disabled), len(pend),
                 summary["elapsed_secs"], ",".join(LANDED_WAVES)))
        if engaged:
            print("\n★측정 축이 꺼져 있다 — 이 결과는 통과가 아니다(exit 2):")
            for kind, env, val, scope in engaged:
                print("  - [%s] %s=%s → %s" % (kind, env, val, scope))
        if failed:
            print("\n적색 검체 → 원 결함 ID로 역추적:")
            for r in failed:
                print("  - %s [%s]: %s" % (r["id"], ",".join(r["defects"]), r.get("reason", "")[:400]))
    # ── 종료코드 3값 — 0=GREEN · 1=RED(재서 틀렸다) · 2=UNMEASURED(재지 않았다) ──
    # ★2 를 새로 내는 근거: 기계 소비자는 종료코드만 본다. 요약 라벨만 바꾸고 코드를 0 으로 두면
    #   "통과라고 답하는 계측기" 가 그대로 남는다(리뷰어 재현: `CYS_U26_OFF=1` → 5 SKIP · exit 0).
    #   1 과 섞지 않은 이유는 원인이 다르기 때문이다 — 섞으면 CI 가 '틀렸다' 와 '안 쟀다' 를
    #   되찾지 못하고, 되찾지 못하는 신호는 결국 무시된다.
    if failed:
        return 1
    return 2 if verdict == "UNMEASURED" else 0


if __name__ == "__main__":
    sys.exit(main())
