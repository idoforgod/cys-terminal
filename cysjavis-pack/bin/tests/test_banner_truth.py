#!/usr/bin/env python3
"""test_banner_truth.py — W3/T3 배너 진실·수명 계약 핀.

계약(구현 후 통과 기준):
  A) javis_formation.feed_kind_for_state: 상태→UI 배너 신호 kind
     (complete=formation-complete·partial=formation-partial·그 외=formation-pending/failed).
  B) javis_formation.evt_type_for_state: 상태→EVT v2 타입(boot.formation.*).
  C) javis_formation._surface: kind 전이 시에만 표면화(심박 스팸 억제 — '전이 시' 계약).
  D) javis_bootstrap._formation_hint: 부트 실패 hint 를 formation 상태로 분기(오진 제거),
     pending-cli 는 기능1 온보딩 UI(설치 안내) 보존, failed 는 상태 파일 포인터.
  E) javis_event: boot.formation.* 4종이 계약에 등재(deny-by-default 통과) + 미지 타입은 여전히 거부.

실행: python3 test_banner_truth.py   (exit 0=PASS / 1=FAIL)
"""
import importlib.util
import os
import subprocess
import sys

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.normpath(os.path.join(SELF, ".."))
sys.path.insert(0, BIN)  # 형제 모듈(javis_scrub·javis_cli_probe) 해석 — 로드 대상의 import 충족
# 밀폐 고정(라이브 팩 상속 금지 — 리포 팩만). test_formation 관례 승계.
os.environ["CYS_PACK_DIR"] = os.path.normpath(os.path.join(SELF, "..", ".."))  # cysjavis-pack/
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _load(mod_name, filename):
    path = os.path.join(BIN, filename)
    if not os.path.exists(path):
        return None
    spec = importlib.util.spec_from_file_location(mod_name, path)
    m = importlib.util.module_from_spec(spec)
    sys.modules[mod_name] = m
    spec.loader.exec_module(m)
    return m


# ── A/B/C: javis_formation ──
fm = _load("javis_formation", "javis_formation.py")
check("A0.formation import", fm is not None)
if fm:
    # A) feed_kind_for_state
    check("A1.complete→formation-complete", fm.feed_kind_for_state("complete") == "formation-complete")
    check("A2.partial→formation-partial", fm.feed_kind_for_state("partial:agy,codex") == "formation-partial")
    check("A3.pending-cli→formation-pending",
          fm.feed_kind_for_state("pending-cli:master,cso") == "formation-pending")
    check("A4.pending-resource→formation-pending",
          fm.feed_kind_for_state("pending-resource") == "formation-pending")
    check("A5.failed→formation-failed", fm.feed_kind_for_state("failed:boom") == "formation-failed")

    # B) evt_type_for_state
    check("B1.complete EVT", fm.evt_type_for_state("complete") == "boot.formation.complete")
    check("B2.partial EVT", fm.evt_type_for_state("partial:agy") == "boot.formation.partial")
    check("B3.pending-cli EVT", fm.evt_type_for_state("pending-cli:x") == "boot.formation.pending")
    check("B4.pending-resource EVT", fm.evt_type_for_state("pending-resource") == "boot.formation.pending")
    check("B5.failed EVT", fm.evt_type_for_state("failed:x") == "boot.formation.failed")

    # C) _surface 전이 게이팅 — _feed_for_state·_emit_evt 를 스파이로 교체
    calls = {"feed": [], "evt": []}
    fm._feed_for_state = lambda state: calls["feed"].append(state)
    fm._emit_evt = lambda evt, fields: calls["evt"].append((evt, fields))

    calls["feed"].clear(); calls["evt"].clear()
    fm._surface(None, None, "complete")  # 최초 관측(prev None) → 발화
    check("C1.최초 관측 발화", len(calls["feed"]) == 1 and calls["evt"] == [("boot.formation.complete",
          {"socket": "", "state": "complete", "kind": "complete"})])

    calls["feed"].clear(); calls["evt"].clear()
    fm._surface(None, "complete", "complete")  # kind 무변화 → 무발화(스팸 억제)
    check("C2.kind 무변화 무발화", calls["feed"] == [] and calls["evt"] == [])

    calls["feed"].clear(); calls["evt"].clear()
    fm._surface("sock", "pending-cli:x", "complete")  # pending→complete 승급 → 발화
    check("C3.pending→complete 승급 발화",
          calls["feed"] == ["complete"] and calls["evt"][0][0] == "boot.formation.complete")

    calls["feed"].clear(); calls["evt"].clear()
    fm._surface(None, "pending-cli:x", "partial:agy")  # pending→partial 승급 → 발화
    check("C4.pending→partial 승급 발화",
          calls["feed"] == ["partial:agy"] and calls["evt"][0][0] == "boot.formation.partial")

    calls["feed"].clear(); calls["evt"].clear()
    fm._surface(None, "partial:agy", "partial:codex")  # 같은 kind(partial) → 무발화
    check("C5.partial 유지 무발화", calls["feed"] == [] and calls["evt"] == [])

    # C6) 소켓 스레딩 — _surface 가 소켓을 EVT fields 로 전달(UI 배너 스코핑의 데이터 원천).
    # (교차소켓 오소멸·edge-race 수렴 자체는 UI 층 bootbanner.ts 순수함수 몫 — bun 테스트에서 검증.)
    calls["feed"].clear(); calls["evt"].clear()
    fm._surface("dept-1.sock", None, "complete")
    check("C6.소켓이 EVT fields 로 스레딩(스코핑 데이터 원천)",
          calls["evt"] and calls["evt"][0][1].get("socket") == "dept-1.sock")

# ── D: javis_bootstrap._formation_hint ──
bs = _load("javis_bootstrap", "javis_bootstrap.py")
check("D0.bootstrap import", bs is not None)
if bs:
    h_cli = bs._formation_hint({"kind": "pending-cli", "state": "pending-cli:master,cso"})
    check("D1.pending-cli 온보딩 UI 보존(설치 안내)",
          h_cli is not None and "claude.ai/install.sh" in h_cli and "단독 사용" in h_cli)
    h_res = bs._formation_hint({"kind": "pending-resource", "state": "pending-resource"})
    check("D2.pending-resource 자원 안내", h_res is not None and "자원" in h_res)
    h_par = bs._formation_hint({"kind": "partial", "state": "partial:agy,codex"})
    check("D3.partial 부족 CLI 목록", h_par is not None and "agy,codex" in h_par)
    h_fail = bs._formation_hint({"kind": "failed", "state": "failed:boom"})
    check("D4.failed 상태 파일 포인터",
          h_fail is not None and "formation" in h_fail and "boot-last.json" in h_fail)
    check("D5.complete/미상 → None(기존 메시지 폴백)",
          bs._formation_hint({"kind": "complete", "state": "complete"}) is None
          and bs._formation_hint({}) is None
          and bs._formation_hint(None) is None)
    # 오진 제거 확증: pending-resource 원인인데 "claude CLI 설치" 로 오진하지 않는다.
    check("D6.오진 제거(자원 원인에 CLI 오안내 금지)", "claude" not in (h_res or ""))

# ── E: javis_event 계약 등재 ──
ev = _load("javis_event", "javis_event.py")
check("E0.event import", ev is not None)
if ev:
    for t in ("boot.formation.complete", "boot.formation.partial",
              "boot.formation.pending", "boot.formation.failed"):
        ok, err = ev.validate(t, {"socket": "s", "state": "complete", "kind": "complete"})
        check("E1.%s 등재·검증 통과" % t, ok, err or "")
        check("E2.%s SPEAK 대본 존재" % t, t in ev.SPEAK)
    # 필수 키 누락 거부
    ok, _ = ev.validate("boot.formation.complete", {"socket": "s"})
    check("E3.필수 키 누락 거부", not ok)
    # 미지 타입은 여전히 deny-by-default
    ok, _ = ev.validate("boot.formation.unknown", {"socket": "s"})
    check("E4.미지 boot.formation.* 거부(deny-by-default 불변)", not ok)
    # speak 렌더 무예외
    try:
        line = ev.speak("boot.formation.partial", {"socket": "s", "state": "partial:agy", "kind": "partial"})
        check("E5.speak 렌더", "partial:agy" in line)
    except Exception as e:
        check("E5.speak 렌더", False, str(e))

# ── F: bash -n 등가(파이썬 컴파일) 스모크 — 방출 CLI 왕복(계약 위반 0) ──
if ev:
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_event.py"), "emit",
                        "boot.formation.complete", "--field", "socket=s",
                        "--field", "state=complete", "--field", "kind=complete"],
                       capture_output=True, text=True)
    check("F1.emit 왕복 exit 0", r.returncode == 0, r.stderr.strip())
    check("F1b.emit wire 라인", "[EVT v2] boot.formation.complete" in r.stdout)

# ── G: pending-resource 배너 문구의 **진실성** — "자동 재시도" 약속에 재시도 주체가 실존하는가 ──
# 배경(2026-07-26): pending-resource 배너는 "자원 정리 후 자동 재시도됩니다"라고 말하는데, 정작
# 편성 ensure 를 주기 실행하는 잡이 schedule.json 에 **0건**이었다(거짓 약속 = 배너 진실 위반).
# 재시도 주체(주기 잡)와 스팸 억제(_surface 전이 게이트)는 **반드시 함께** 있어야 한다 — 잡만 있고
# 억제가 없으면 매 틱 토스트 폭주, 억제만 있고 잡이 없으면 약속이 거짓이다. 둘 다 못박는다.
import json as _json

SCHED = os.path.normpath(os.path.join(SELF, "..", "..", "schedule.json"))
try:
    _jobs = (_json.load(open(SCHED, encoding="utf-8")) or {}).get("jobs", [])
except (OSError, ValueError) as e:
    _jobs = []
    check("G0.schedule.json 파싱", False, str(e))
_fj = [j for j in _jobs if "javis_formation.py" in (j.get("command") or "")
       or "javis_formation.py" in (j.get("text_command") or "")]
check("G1.편성 재시도 주체 실존(schedule.json 에 formation ensure 주기 잡)",
      bool(_fj), "formation 잡 %d건 — 배너의 '자동 재시도' 약속이 거짓이 된다" % len(_fj))
if _fj:
    j = _fj[0]
    check("G2.주기 잡이다(every_minutes 설정·과빈도 아님)",
          isinstance(j.get("every_minutes"), int) and 1 <= j["every_minutes"] <= 60,
          "every_minutes=%r" % j.get("every_minutes"))
    check("G3.ensure 를 호출한다", " ensure" in (j.get("command") or ""), j.get("command", ""))
    check("G4.레인 소켓 규약 준수(부서=--socket · 본부=인자 생략)",
          "${CYS_SOCKET:+--socket" in (j.get("command") or ""),
          "레인별 상태키·락키가 갈라지면 중복 스폰")
if fm:
    # G5) 짝 조건: 주기 잡이 붙어도 표면화는 kind 전이 시에만(위 C2/C5 가 억제를 실증) —
    #     ensure 최종 경로가 무조건 feed 하지 않는지 소스 계약으로 재확인(스팸 폭주 회귀 핀).
    _src = open(os.path.join(BIN, "javis_formation.py"), encoding="utf-8").read()
    _tail = _src[_src.index("def ensure("):_src.index("# ── 두 경로 동등성")]
    check("G5.ensure 어느 경로도 _feed_for_state 를 직접 호출하지 않는다(전부 _surface 경유)",
          "_feed_for_state(" not in _tail,
          "무조건 표면화가 남아 있으면 주기 잡이 매 틱 토스트를 낸다")

# ── H: 부트 레인 강제 표면화 **두 레인 동등성** (2026-07-26 · 배너 소멸 갭 3단계) ──
# 배경: 표면화를 '전이 시에만'으로 좁힌 결과, 상태 파일이 이미 complete 인 레인은 세션이 새로
# 시작돼도(=UI 의 인메모리 lastFormationKind 초기화) formation-complete 를 재발행하지 않아 배너
# 소멸 신호가 유실된다. 해법은 **세션 1회성 부트 경로에서만** 강제 표면화(--force-surface)다.
# 부트 레인은 두 개이며 둘 다 붙어야 한다 — 한쪽만 붙으면 그 경로 사용자에게만 배너가 불멸한다:
#   ① 앱 부트 레인   : src-tauri/src/main.rs fire_formation_ensure
#   ② 부트스트랩 레인 : javis_bootstrap.py _run_formation_ensure ("너는 마스터다" 수동 각성 주경로)
# 짝 조건은 주기 잡 **미부착**이며, 그쪽은 test_phoenix_e2_schedule.py ②′ 가 못박는다(중복 배제).
_bs_src = open(os.path.join(BIN, "javis_bootstrap.py"), encoding="utf-8").read()
_bs_fn = _bs_src[_bs_src.index("def _run_formation_ensure("):_bs_src.index("def _run_heal_preflight(")]
check("H1.부트스트랩 레인이 ensure 를 --force-surface 로 호출(세션 1회성 · 배너 소멸 신호 도달)",
      "--force-surface" in _bs_fn,
      "수동 각성 경로에 없으면 앱 재시작/새 세션에서 boot-warning 배너가 불멸한다")
# 앱 부트 레인(Rust)은 팩 밖 파일이라 리포에서만 검사 가능 — 팩 단독 설치 환경에서는 SKIP.
_MAIN_RS = os.path.normpath(os.path.join(SELF, "..", "..", "..", "src-tauri", "src", "main.rs"))
if os.path.exists(_MAIN_RS):
    _rs = open(_MAIN_RS, encoding="utf-8").read()
    _rs_fn = _rs[_rs.index("fn fire_formation_ensure"):]
    _rs_fn = _rs_fn[:_rs_fn.index("\nfn ") if "\nfn " in _rs_fn else len(_rs_fn)]
    check("H2.앱 부트 레인도 --force-surface(두 레인 동등성 — 한쪽만 고치면 반쪽 수리)",
          '.arg("--force-surface")' in _rs_fn,
          "src-tauri fire_formation_ensure 에서 강제 표면화가 빠졌다")
else:
    print("SKIP H2.앱 부트 레인 소스 부재(팩 단독 설치) — %s" % _MAIN_RS)

print("\n%s: %d fail(s) %s" % ("FAIL" if fails else "PASS", len(fails), fails or ""))
sys.exit(1 if fails else 0)
