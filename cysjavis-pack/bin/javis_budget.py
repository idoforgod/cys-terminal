#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_budget.py — 부트 체인 **시간 예산 단일 소스**(T-0147-7 W2 · B9·B17·P3-A-120S).

## 왜 모듈인가
종전 예산은 상수 곱 산술이 세 언어·다섯 파일에 흩어져 있었고, 그래서 **외부 상한 < 내부 최악치**
역전이 3중으로 존재했다(재감사 B9 실물 앵커):

    외부 300s `cys boot`              ↔ 내부 최악 5노드 × 노드당 최악
    외부 130s `_boot_one_node`        ↔ 내부 최악 boot_node(--timeout 90 + 데드라인 무시 서브프로세스)
    외부 320s `boot-reviewers`        ↔ 내부 최악 2슬롯 × (네이티브 + 대체 2차 폴백)

역전은 "상위가 하위를 죽인다"는 뜻이다 — 하위가 정상 동작 중인데 상위 timeout 이 먼저 터져
**조기실패**를 만들고, 그 조기실패가 A1(라이브락)·B6(허위 성공) 오판 사슬의 산출 중간상태를
공급했다.

## 방향(비평2 D-2 확정 — 이 파일의 불변식)
1. **내부 감액 금지**. 냉시작 실측 하한(leaf 상수)은 내려가지 않는다. adv#4 가 실측 실패로
   예산을 '늘린' 역사가 있고, 감액은 그 역행(조기실패 부활)이다. `LEAF_FLOORS` 가 그 하한을
   박제하고 `assert_floors()` 가 회귀를 hard fail 시킨다.
2. **외부 상한은 증액**한다 — 하위 최악치 합 + 마진에서 **파생**한다(하드코딩 금지).
3. 증액이 만드는 침묵 창은 **진행 하트비트**(stderr)로 상쇄한다. verdict 채널(A7 stdout/stderr
   JSON)과는 분리된 별개 채널이다.
4. **데드라인 전파**가 우선 수단이다. 내부 최악치를 줄이는 정당한 방법은 leaf 감액이 아니라
   `--timeout` 전파로 하위가 **자기 데드라인을 아는 것**이다(감액 0 · 최악치만 유계화).

## 카운트 회계 금지(B17·H-TIME-3)
`waited += 2` 류의 **카운트 기반 시간 회계**는 폐기다. 틱당 실비용(RPC 왕복·sleep·trust 분기
sleep)이 가정치와 어긋나 실효 대기가 25%+α 로 오차났다. 벽시계(Instant/`time.monotonic`)
데드라인만 쓴다.

## 소비자
  python : javis_bootstrap.py(① ④ ④-b ⑤) · javis_orchestra.py(_boot_one_node·boot-reviewers)
           · javis_boot_node.py(--timeout 기본·데드라인 전파)
  rust   : src/bin/cys.rs 의 `BUDGET_*` 상수 블록 — 이 파일과 **기계 대조**된다(H-TIME-1).
  문서/훅 : `--note-check-window` 등 파생 출력만 인용(하드코딩 grep 0 — H-TIME-2).

사용:
  python3 javis_budget.py --json            # 전 예산 표(기계 판독)
  python3 javis_budget.py --parity          # Σ내부최악 ≤ 외부 상한 검사(exit 0=OK / 1=역전)
  python3 javis_budget.py --get CYS_BOOT_OUTER_S
  python3 javis_budget.py --note-check-window   # 훅 안내용 '생존확인 최대 Ns' 숫자만
  python3 javis_budget.py --self-test
"""
import json
import os
import sys

# ─────────────────────────────────────────────────────────────────────────────
# 1) LEAF 상수 — 냉시작 실측 하한. **감액 금지**(내리면 assert_floors 가 hard fail).
#    env override 는 테스트 하네스 전용이며, 하한 미만으로는 내려가지 않는다(clamp).
# ─────────────────────────────────────────────────────────────────────────────
LEAF_FLOORS = {
    # cys RPC 왕복 계열 — 데몬 냉시작·프로세스 표 refresh 를 포함한 실측 하한.
    "CYS_STATUS_TIMEOUT_S": 12,      # javis_boot_node.cys_status / orchestra.cys_status
    "CYS_LIST_TIMEOUT_S": 15,        # javis_bootstrap._live_role_names / boot_node.cys_list_rows
    "CYS_PING_TIMEOUT_S": 15,        # javis_bootstrap ② ping
    "CYS_CLAIM_TIMEOUT_S": 15,       # javis_bootstrap ③ claim-role
    "RPC_SLACK_S": 10,               # 한 계층이 흘리는 비유계 RPC 왕복 여유(잔여 granularity)

    # ② ping 재시도 창(W-A4 선등재 · 소비자는 W-A3 javis_bootstrap ② 재시도 루프).
    #   단발 ping(CYS_PING_TIMEOUT_S=15) 1회로는 '데몬 자동기동+소켓 바인드+프로세스 표
    #   refresh' 최악 냉시작을 놓칠 수 있다 — 창을 두되 무한 대기는 금지한다(자원 거버넌스).
    #   ★값 근거: TOTAL 45 = 시도당 상한 15(위 냉시작 실측 하한) × 3회분 — 하한 계열에서
    #   파생한 크기라 leaf 실측성이 유지된다. INTERVAL 3 = 시도 간 백오프 —
    #   BOOT_NODE_INJECT_BACKOFF_S(2)·CHECK_INTERVAL_S(5) 사이 크기로, 데몬 기동 직후 소켓
    #   바인드(~1s)를 여유 있게 넘기면서 45s 창의 시도 기회를 잠식하지 않는다(즉시 거절되는
    #   fail-fast ping 에서도 최대 ~15회로 유계 — 재시도 폭주 없음).
    #   ★TOTAL 은 **벽시계 데드라인**이다(B17 카운트 회계 금지) — 소비자는 횟수 셈이 아니라
    #   time.monotonic() 데드라인으로 창을 닫아야 한다.
    "CYS_PING_RETRY_TOTAL_S": 45,
    "CYS_PING_RETRY_INTERVAL_S": 3,

    # javis_boot_node 내부
    "BOOT_NODE_TOTAL_S": 90,         # --timeout 기본(전체 데드라인)
    "BOOT_NODE_LAUNCH_SUBPROC_S": 80,  # `cys launch-agent` 서브프로세스 상한
    "BOOT_NODE_INJECT_ATTEMPTS": 4,
    "BOOT_NODE_INJECT_TIMEOUT_S": 12,
    "BOOT_NODE_INJECT_BACKOFF_S": 2,
    "BOOT_NODE_IDLE_SETTLE_S": 4,      # --idle 기본

    # cys launch-agent(rust) readiness — cys.rs BUDGET_* 블록과 기계 대조되는 leaf.
    "LAUNCH_READINESS_FLOOR_S": 30,    # delay.max(FLOOR)
    "LAUNCH_READINESS_MULT": 2,        # … * MULT
    "LAUNCH_RESTORE_CAP_S": 20,        # restore 모드 캡
    "LAUNCH_TICK_MS": 2500,            # readiness 폴링 틱(벽시계 — 카운트 회계 금지)
    "LAUNCH_POST_MARKER_SETTLE_S": 2,  # marker 감지 후 TUI 입력 활성 여유
    "LAUNCH_ACK_WAIT_S": 8,            # B14 주입검증 ack 대기(짧게 — 미확인=상태화, 치명 아님)
    "LAUNCH_TRUST_SETTLE_S": 2,        # G35 폴더신뢰 Return 후 소멸 확인 여유

    # 편성 규모
    "PLAN_ROLE_COUNT": 5,              # cys boot PLAN 행 수(cso·worker·리뷰어3)
    "REVIEWER_SLOT_COUNT": 2,          # REVIEWER_SLOTS 길이
    "REVIEWER_FALLBACK_ATTEMPTS": 2,   # 네이티브 + 대체 2차 폴백

    # ⑤ check 재시도 창
    "CHECK_RETRIES": 24,
    "CHECK_INTERVAL_S": 5,
    "CHECK_SUBPROC_TIMEOUT_S": 60,

    # ① preflight — 내부 최악치 열거 불가(수백 지점) → 외부 상한을 실측 상한으로 고정 유지.
    #   ★유일한 비파생 외부 상한이며 그 사실을 여기 명문화한다(은닉 하드코딩 아님).
    "PREFLIGHT_OUTER_S": 300,

    # 마진 정책
    "MARGIN_RATIO": 0.2,               # 하위 최악치 합의 20%
    "MARGIN_MIN_S": 20,                # 최소 절대 마진
    "HEARTBEAT_INTERVAL_S": 20,        # 침묵 창 상쇄용 stderr 진행 하트비트 주기
    # non-unix(Windows) pidfile 락의 스테일 회수 임계. 파일시스템 락은 자동 해제가 없어
    # 크래시 잔재가 영구히 부트를 막을 수 있다 — 나이 기반 회수가 그 상한이다(영구 Busy 불가).
    "LOCK_STALE_S": 900,
}

_ENV_PREFIX = "CYS_BUDGET_"


def _leaf(name):
    """leaf 상수 해소 — env override 는 **하한 clamp**(감액 금지 불변식의 집행부)."""
    floor = LEAF_FLOORS[name]
    raw = os.environ.get(_ENV_PREFIX + name)
    if raw is None:
        return floor
    try:
        val = type(floor)(float(raw)) if not isinstance(floor, float) else float(raw)
    except (TypeError, ValueError):
        return floor
    # ratio/interval 계열은 상향만 허용해도 의미가 유지된다 — 전 leaf 동일 규약(하한 clamp).
    return val if val >= floor else floor


# 편의 접근자 — 소비 코드가 dict 키 문자열을 흘리지 않게.
def leaf(name):
    return _leaf(name)


def _margin(inner_worst):
    return max(_leaf("MARGIN_MIN_S"), int(round(inner_worst * _leaf("MARGIN_RATIO"))))


# ─────────────────────────────────────────────────────────────────────────────
# 2) 파생 — 내부 최악치(inner worst) → 외부 상한(outer). 하드코딩 0.
# ─────────────────────────────────────────────────────────────────────────────
def boot_node_inject_worst_s():
    """inject() 최악: attempts × 큐 등록 timeout + (attempts-1) × 백오프."""
    n = _leaf("BOOT_NODE_INJECT_ATTEMPTS")
    return n * _leaf("BOOT_NODE_INJECT_TIMEOUT_S") + (n - 1) * _leaf("BOOT_NODE_INJECT_BACKOFF_S")


def boot_node_inner_worst_s():
    """javis_boot_node 1회 실행의 내부 최악치.

    ★데드라인 전파 후의 최악치다(W2): boot_node 는 자기 `--timeout` 을 알고 LAUNCH 서브프로세스·
      inject 큐 등록·폴링을 `min(leaf, remaining())` 으로 자른다. 따라서 최악치는
      '전체 데드라인 + 마지막으로 진입한 비유계 서브프로세스 1개의 granularity' 로 유계화된다.
      leaf(BOOT_NODE_LAUNCH_SUBPROC_S=80 등)는 **감액하지 않았다** — 유계화만 했다.
    """
    granularity = max(_leaf("CYS_STATUS_TIMEOUT_S"), _leaf("CYS_LIST_TIMEOUT_S"))
    return _leaf("BOOT_NODE_TOTAL_S") + granularity


def boot_node_outer_s():
    """orchestra._boot_one_node 가 boot_node 에 씌우는 subprocess timeout."""
    inner = boot_node_inner_worst_s()
    return inner + _margin(inner)


def boot_reviewers_inner_worst_s():
    """슬롯 수 × (네이티브 + 대체 2차 폴백) × boot_node 외부 상한."""
    return (_leaf("REVIEWER_SLOT_COUNT") * _leaf("REVIEWER_FALLBACK_ATTEMPTS")
            * boot_node_outer_s())


def boot_reviewers_outer_s():
    inner = boot_reviewers_inner_worst_s()
    return inner + _margin(inner)


def launch_readiness_max_s(inject_delay_s=None, restore=False):
    """cys launch-agent readiness 폴링 상한(벽시계 데드라인 — 카운트 회계 금지).
    inject_delay_s = agents.json 의 inject_delay_secs(없으면 floor 로 수렴)."""
    delay = inject_delay_s if isinstance(inject_delay_s, (int, float)) else 0
    base = max(delay, _leaf("LAUNCH_READINESS_FLOOR_S")) * _leaf("LAUNCH_READINESS_MULT")
    return min(base, _leaf("LAUNCH_RESTORE_CAP_S")) if restore else base


def launch_per_node_worst_s():
    """cys boot 가 한 role 에 지불하는 최악치 — readiness + 안착 + 주입 ack 검증 + RPC 여유."""
    return (launch_readiness_max_s()
            + _leaf("LAUNCH_POST_MARKER_SETTLE_S")
            + _leaf("LAUNCH_ACK_WAIT_S")
            + _leaf("RPC_SLACK_S"))


def cys_boot_inner_worst_s():
    return _leaf("PLAN_ROLE_COUNT") * launch_per_node_worst_s()


def cys_boot_outer_s():
    inner = cys_boot_inner_worst_s()
    return inner + _margin(inner)


def check_window_s():
    """⑤ check 재시도 창의 총 대기(문서·훅 안내 숫자의 유일한 출처 — H-TIME-2)."""
    return int(round(_leaf("CHECK_RETRIES") * _leaf("CHECK_INTERVAL_S")))


def check_inner_worst_s():
    """⑤ 단계 내부 최악: 재시도마다 (subprocess 상한 + 간격)."""
    return int(round(_leaf("CHECK_RETRIES")
                     * (_leaf("CHECK_SUBPROC_TIMEOUT_S") + _leaf("CHECK_INTERVAL_S"))))


def ping_retry_worst_s():
    """② ping 재시도 루프(W-A3)의 내부 최악치: 벽시계 데드라인 + 마지막 시도 1회의 granularity.

    boot_node_inner_worst_s 의 'TOTAL + granularity' 와 동일한 유계화 패턴 — 데드라인 직전에
    진입한 마지막 ping 이 자기 시도 상한(CYS_PING_TIMEOUT_S)을 다 쓸 수 있으므로 창 총량만
    계상하면 과소다(과소계상 = 이 모듈이 죽이려는 조기실패의 씨앗).
    """
    return _leaf("CYS_PING_RETRY_TOTAL_S") + _leaf("CYS_PING_TIMEOUT_S")


def bootstrap_chain_worst_s():
    """cmd_run 전체 최악치(냉부팅 상한 — 산문 '1555s' 의 파생 대체값).

    ★② 항은 단발 ping timeout 이 아니라 재시도 창 최악치를 계상한다(W-A4): W-A3 가 ② 를
      재시도 루프로 바꾸는데 단발 15s 계상이 남으면 '계상 상한 < 내부 최악치' — 이 모듈의
      존재 이유인 바로 그 역전이 문서면에 재발한다. W-A3 미착륙 상태에서도 이 값은 유효한
      상한이다(현행 단발 실최악 15s ≤ 계상 60s — 증액 방향은 안전하고, 위험한 것은 과소계상
      뿐이다 · 불변식 2). 소비처 실측 0(table 문서면 전용)이라 timeout 집행에는 영향이 없다.
    """
    return int(round(_leaf("PREFLIGHT_OUTER_S")
                     + ping_retry_worst_s()
                     + _leaf("CYS_CLAIM_TIMEOUT_S")
                     + cys_boot_outer_s()
                     + boot_reviewers_outer_s()
                     + check_inner_worst_s()))


# ─────────────────────────────────────────────────────────────────────────────
# 3) parity — Σ내부최악 ≤ 외부 상한 (H-TIME-1)
# ─────────────────────────────────────────────────────────────────────────────
def parity_pairs():
    """(라벨, 내부 최악치, 외부 상한) 목록. 전 쌍에서 내부 ≤ 외부여야 한다."""
    return [
        ("boot_node ⊂ _boot_one_node", boot_node_inner_worst_s(), boot_node_outer_s()),
        ("boot-reviewers 슬롯합 ⊂ ④-b timeout",
         boot_reviewers_inner_worst_s(), boot_reviewers_outer_s()),
        ("cys boot 노드합 ⊂ ④ timeout", cys_boot_inner_worst_s(), cys_boot_outer_s()),
    ]


def parity_violations():
    return [(label, inner, outer) for label, inner, outer in parity_pairs() if inner > outer]


def assert_floors():
    """leaf 상수가 실측 하한 미만으로 내려가지 않았음을 단언(감액 방향 회귀 차단)."""
    bad = []
    for name, floor in LEAF_FLOORS.items():
        if _leaf(name) < floor:
            bad.append("%s=%s < 하한 %s" % (name, _leaf(name), floor))
    if bad:
        raise AssertionError("내부 감액 금지 위반: " + ", ".join(bad))


def table():
    """전 예산 표(기계 판독) — 러너·문서·Rust parity 검체가 이 dict 를 소비한다."""
    return {
        "leaf": {k: _leaf(k) for k in sorted(LEAF_FLOORS)},
        "leaf_floors": dict(LEAF_FLOORS),
        "derived": {
            "BOOT_NODE_INJECT_WORST_S": boot_node_inject_worst_s(),
            "BOOT_NODE_INNER_WORST_S": boot_node_inner_worst_s(),
            "BOOT_NODE_OUTER_S": boot_node_outer_s(),
            "BOOT_REVIEWERS_INNER_WORST_S": boot_reviewers_inner_worst_s(),
            "BOOT_REVIEWERS_OUTER_S": boot_reviewers_outer_s(),
            "LAUNCH_READINESS_MAX_S": launch_readiness_max_s(),
            "LAUNCH_PER_NODE_WORST_S": launch_per_node_worst_s(),
            "CYS_BOOT_INNER_WORST_S": cys_boot_inner_worst_s(),
            "CYS_BOOT_OUTER_S": cys_boot_outer_s(),
            "PING_RETRY_WORST_S": ping_retry_worst_s(),
            "CHECK_WINDOW_S": check_window_s(),
            "CHECK_INNER_WORST_S": check_inner_worst_s(),
            "BOOTSTRAP_CHAIN_WORST_S": bootstrap_chain_worst_s(),
            "PREFLIGHT_OUTER_S": _leaf("PREFLIGHT_OUTER_S"),
            "HEARTBEAT_INTERVAL_S": _leaf("HEARTBEAT_INTERVAL_S"),
        },
        "parity": [{"pair": p, "inner": i, "outer": o, "ok": i <= o}
                   for p, i, o in parity_pairs()],
    }


# ★Rust `src/bin/cys.rs` 의 BUDGET_* 상수와 1:1 대조되는 이름 표(H-TIME-1 grep 검체).
#   Rust 는 python 을 import 할 수 없으므로 파리티는 **기계 대조**로만 보장한다.
# ★cysd 감독자 SOT(2026-09-05 · master 지시 · W-B 조율). **LEAF_FLOORS 에 넣지 않는다** —
#   그 표는 예산 역전 판정(Σ내부최악 ≤ 외부 상한)과 감액 clamp 의미를 지고 있어서, 성격이 다른
#   감독자 임계를 섞으면 엉뚱한 축이 깨진다. 여기는 **값의 소유자**일 뿐이고 대조는 파리티가 한다.
#   목적은 하나다: Rust 감독자 상수가 python SOT 와 갈리면 **기계가 잡는다**(조용한 드리프트 차단).
CYSD_SUPERVISOR_SOT = {
    # ★계보 정직 고지: 명세 §2-6 ⓐ 의 **리터럴**이다("hb 정지 > 90s"). BOOT_NODE_TOTAL_S 도 90 이라
    #   숫자가 같지만 명세에 유도가 없어 **계보는 확인되지 않았다** — 없는 계보를 지어내면 나중에
    #   그 leaf 를 바꾸는 사람이 fence 임계까지 함께 움직이는 줄 모른다(W-B 조율 2026-09-05).
    "BOOT_HB_STALL_S": 90,
    # ★두 계약을 진다(A17 개정 2026-09-05 · master 판정 · W-B 통지): 인텐트 **수명**이자
    #   명세 §2-6 ⓒ 의 **절대 마감**이다. 별도 마감 기구를 두지 않는 이유는 명세가 지목했던
    #   bootstrap_chain_worst_s 가 3020초라 수명(1800초) 안에서 **원리적으로 발효 불가**(죽은 코드)
    #   였기 때문이다. 이 값을 늘리면 절대 마감도 같이 늘어난다 — 모르고 늘리지 마라.
    "BOOT_INTENT_MAX_AGE_S": 1800,      # 인텐트 수명(초) + 절대 마감
    "BOOT_RETRY_COOLDOWN_S": 30,        # 재시도 쿨다운(초)
    "BOOT_MAX_ATTEMPTS": 3,             # 부트런 시도 상한(개수)
    "SUPERVISOR_TICK_S": 3,             # 감독자 틱 간격(초)
}

# Rust 상수 ↔ python SOT 파리티 표. 값은 (python 키, 소스 키, Rust 타입)이다.
#   ★소스 키를 갖는 이유(2026-09-05): 종전 이 표는 `src/bin/cys.rs` 만 대상이라 **cysd 감독자
#     상수는 대조 밖**이었다 — python 이 값을 바꿔도 아무도 모르는 무측정 계급이다.
#   ★타입을 갖는 이유: 감독자 상수는 `f64`(1800.0)·`u32`·`usize` 라 종전의 `u64` 전용 정규식으로는
#     한 건도 못 잡는다(핀이 조용히 공허해진다).
RUST_PARITY_CONSTS = {
    "BUDGET_READINESS_FLOOR_SECS": ("LAUNCH_READINESS_FLOOR_S", "cys", "u64"),
    "BUDGET_READINESS_MULT": ("LAUNCH_READINESS_MULT", "cys", "u64"),
    "BUDGET_RESTORE_CAP_SECS": ("LAUNCH_RESTORE_CAP_S", "cys", "u64"),
    "BUDGET_TICK_MS": ("LAUNCH_TICK_MS", "cys", "u64"),
    "BUDGET_POST_MARKER_SETTLE_SECS": ("LAUNCH_POST_MARKER_SETTLE_S", "cys", "u64"),
    "BUDGET_ACK_WAIT_SECS": ("LAUNCH_ACK_WAIT_S", "cys", "u64"),
    "BUDGET_TRUST_SETTLE_SECS": ("LAUNCH_TRUST_SETTLE_S", "cys", "u64"),
    "BUDGET_HEARTBEAT_INTERVAL_SECS": ("HEARTBEAT_INTERVAL_S", "cys", "u64"),
    "BUDGET_LOCK_STALE_SECS": ("LOCK_STALE_S", "cys", "u64"),
    # ── cysd 감독자(src/bin/cysd/boot_supervisor.rs) ─────────────────────────
    "HB_STALL_SECS": ("BOOT_HB_STALL_S", "cysd_boot_supervisor", "f64"),
    "INTENT_MAX_AGE_SECS": ("BOOT_INTENT_MAX_AGE_S", "cysd_boot_supervisor", "f64"),
    "RETRY_COOLDOWN_SECS": ("BOOT_RETRY_COOLDOWN_S", "cysd_boot_supervisor", "f64"),
    "MAX_ATTEMPTS": ("BOOT_MAX_ATTEMPTS", "cysd_boot_supervisor", "u32"),
    "SUPERVISOR_INTERVAL_SECS": ("SUPERVISOR_TICK_S", "cysd_boot_supervisor", "u64"),
    # ★선등재(2026-09-05 · W-B 확정값 · B3-3 에서 작성 예정): progress_step 정체 fence 임계.
    #   아직 소스에 없으므로 PEND 이고, W-B 가 쓰는 순간 **자동으로 발효**한다 — 핀이 상수보다
    #   먼저 있으면 그 상수는 태어날 때부터 대조를 받는다(드리프트가 생길 창 자체가 없다).
    #   python 대응은 리터럴이 아니라 **파생값** boot_node_outer_s() 다: 한 노드가 자기 outer 예산을
    #   꽉 써도 fence 되면 안 되므로 그 축의 상한을 쓴다(BOOT_NODE_TOTAL_S·inner 로 두면 경계에서
    #   건강한 런을 자른다 · W-B 근거 수용). leaf 가 커지면 이 임계도 함께 커져야 한다.
    "PROGRESS_STALL_SECS": ("BOOT_NODE_OUTER_S", "cysd_boot_supervisor", "f64"),
}

# 이 레인에 **아직 없어도 적색이 아닌** 항목(미발효 PEND). 소스에 나타나는 순간 자동으로 발효한다.
#   ★목록을 정확히 고정하는 것이 요점이다 — 여기 없는 상수가 사라지면 그건 PEND 가 아니라
#     적색이다(부재를 SKIP 으로 접는 무측정 방지).
#   ★사유를 함께 지는 이유(2026-09-05 · master 승인 · W-C 지적): 이름만 찍히면 판독자가
#     "아직 이 레인에 없다(병합으로 해소)" 와 "아직 아무도 안 썼다(구현 대기)" 를 **구별할 수 없다**.
#     둘은 조치가 정반대인데 화면에서는 같아 보인다 — 무측정과 같은 계급(원인 구별 불가)이다.
#   해소 경로 어휘는 둘뿐이다: `merge`(다른 레인에 실재 · C4 유입으로 발효) · `impl`(어느 레인에도
#     없음 · 누군가 구현해야 발효). 사유 없는 PEND 는 **구조적으로 불가능**하다 — 아래 집합이
#     이 표에서 파생되므로, 사유를 안 적으면 애초에 PEND 목록에 오르지 못하고 곧바로 적색이 된다.
RUST_PARITY_PENDING_REASON = {
    "HB_STALL_SECS": ("merge", "daemon boot_supervisor.rs 에 실재 — C4 유입 시 발효"),
    "MAX_LIVE_BOOT_RUNS": ("merge", "daemon 유도 상수 — C4 유입 시 발효"),
    "FENCED_REAP_AGE_SECS": ("merge", "daemon 유도 상수 — C4 유입 시 발효"),
    "PROGRESS_STALL_SECS": ("impl", "B3-3 미구현 — 세 레인 어디에도 Rust 상수 없음(선등재 핀)"),
}
PENDING_KINDS = ("merge", "impl")
RUST_PARITY_PENDING = frozenset(RUST_PARITY_PENDING_REASON)


def pending_note(name):
    """PEND 한 줄 서술 — 판독자가 조치를 바로 고를 수 있게 해소 경로를 함께 적는다."""
    kind, why = RUST_PARITY_PENDING_REASON.get(name, ("?", "사유 미등재"))
    return "%s: %s(%s)" % (name, why, kind)

# 유도 상수는 **값을 python 에 복제하지 않는다**(복제 자체가 새 드리프트면 — W-B 지적 수용).
#   대신 **유도식 자체를 핀**한다: 누가 유도를 리터럴로 굳히면 잡힌다.
CYSD_DERIVED_PINS = {
    "MAX_LIVE_BOOT_RUNS": "MAX_ATTEMPTS as usize + 1",
    "FENCED_REAP_AGE_SECS": "INTENT_MAX_AGE_SECS",
}


# 파생 함수를 SOT 로 쓰는 파리티 항목 — 리터럴을 복제하지 않는다(leaf 가 커지면 함께 커진다).
DERIVED_PARITY_SOURCES = {
    "BOOT_NODE_OUTER_S": lambda: boot_node_outer_s(),
}


def parity_value(py_key):
    """파리티 대조용 python 값 — leaf 예산 · cysd 감독자 SOT · 파생 함수 셋 중 하나에서 나온다."""
    if py_key in DERIVED_PARITY_SOURCES:
        return DERIVED_PARITY_SOURCES[py_key]()
    if py_key in CYSD_SUPERVISOR_SOT:
        return CYSD_SUPERVISOR_SOT[py_key]
    return _leaf(py_key)


def self_test():
    try:
        assert_floors()
        # ① 파리티 — 역전 0(이 파일의 존재 이유).
        v = parity_violations()
        assert not v, "예산 역전 잔존: %s" % v
        # ② 외부 상한이 파생값임을 단언(하드코딩 회귀 차단): 내부를 키우면 외부도 커진다.
        base = cys_boot_outer_s()
        os.environ[_ENV_PREFIX + "PLAN_ROLE_COUNT"] = "9"
        try:
            grown = cys_boot_outer_s()
        finally:
            del os.environ[_ENV_PREFIX + "PLAN_ROLE_COUNT"]
        assert grown > base, "외부 상한이 내부 최악치에서 파생되지 않는다(하드코딩 의심)"
        # ③ 감액 금지 clamp — env 로 leaf 를 내려도 하한이 이긴다.
        os.environ[_ENV_PREFIX + "BOOT_NODE_TOTAL_S"] = "1"
        try:
            assert _leaf("BOOT_NODE_TOTAL_S") == LEAF_FLOORS["BOOT_NODE_TOTAL_S"], \
                "leaf 감액이 통과됐다(냉시작 하한 붕괴)"
        finally:
            del os.environ[_ENV_PREFIX + "BOOT_NODE_TOTAL_S"]
        # ④ 증액은 통과(외부 조정 여지 보존).
        os.environ[_ENV_PREFIX + "BOOT_NODE_TOTAL_S"] = "200"
        try:
            assert _leaf("BOOT_NODE_TOTAL_S") == 200, "leaf 증액이 막혔다(외부 증액 방향 차단)"
        finally:
            del os.environ[_ENV_PREFIX + "BOOT_NODE_TOTAL_S"]
        # ⑤ restore 캡이 일반 상한을 넘지 않는다.
        assert launch_readiness_max_s(restore=True) <= launch_readiness_max_s(), \
            "restore 캡이 일반 readiness 상한 초과"
        # ⑥ 훅 안내 숫자가 CHECK 상수 파생(H-TIME-2 의 전제).
        assert check_window_s() == int(round(LEAF_FLOORS["CHECK_RETRIES"]
                                             * LEAF_FLOORS["CHECK_INTERVAL_S"])), \
            "check 안내 창이 상수 파생이 아니다"
        # ⑦ ② ping 재시도 창 정합(W-A4): 창이 1회 시도 상한보다 작으면 재시도가 구조적으로
        #    0회로 접히고, 간격이 창 이상이면 2회차 진입 전에 창이 닫힌다 — 둘 다 '키만 있고
        #    재시도는 없는' 죽은 예산이므로 회귀를 여기서 hard fail 시킨다.
        assert _leaf("CYS_PING_RETRY_TOTAL_S") >= _leaf("CYS_PING_TIMEOUT_S"), \
            "ping 재시도 창이 1회 시도 상한보다 작다(재시도 무의미)"
        assert _leaf("CYS_PING_RETRY_INTERVAL_S") < _leaf("CYS_PING_RETRY_TOTAL_S"), \
            "ping 재시도 간격이 창을 잠식한다"
    except AssertionError as e:
        print("javis_budget self-test FAIL: %s" % e)
        return 1
    print("javis_budget self-test OK — leaf %d종·파생 %d종·파리티 %d쌍 역전 0"
          % (len(LEAF_FLOORS), len(table()["derived"]), len(parity_pairs())))
    return 0


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if "--self-test" in argv:
        return self_test()
    if "--parity" in argv:
        v = parity_violations()
        for label, inner, outer in parity_pairs():
            print("%-38s 내부최악 %6.0fs %s 외부 %6.0fs"
                  % (label, inner, "<=" if inner <= outer else " >", outer))
        if v:
            sys.stderr.write("[budget] 예산 역전 %d건 — 외부 상한을 파생값으로 올려라\n" % len(v))
            return 1
        return 0
    if "--note-check-window" in argv:
        print(check_window_s())
        return 0
    if "--get" in argv:
        i = argv.index("--get")
        if i + 1 >= len(argv):
            sys.stderr.write("[budget] --get <NAME> 필요\n")
            return 64
        name = argv[i + 1]
        t = table()
        for section in ("derived", "leaf"):
            if name in t[section]:
                print(t[section][name])
                return 0
        sys.stderr.write("[budget] 미지 예산 이름: %s\n" % name)
        return 64
    if "--json" in argv or not argv:
        print(json.dumps(table(), ensure_ascii=False, indent=1))
        return 0
    sys.stderr.write("[budget] 미지 인자: %s\n" % " ".join(argv))
    return 64


if __name__ == "__main__":
    sys.exit(main())
