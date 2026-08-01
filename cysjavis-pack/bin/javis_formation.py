#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_formation.py — DD-3 조건화 상비 편성 상태 기계.

부서 생성·"너는 마스터다" 수동 경로가 **동일한 ensure 로 수렴**(C3 동등성의 구조적 보장) —
설치된 CLI 기준 즉시 최대 편성 + 편성 대기(pending) + CLI 설치 시 자동 완결.

상태(FormationState): complete / partial:{missing_clis} / pending-cli:{roles} /
                      pending-resource / failed:{cause}.
  complete = master + 4종 의무(cso·worker·reviewer-gemini·reviewer-codex) 전부 기동일 때만
             (리뷰어 없는 부서는 라운드 루프 불능 → complete 오판 금지·Sim S2-5).
  partial  = 설치된 부분만 기동 + 부족 CLI 목록(설치 시 자동 승급).
  pending-cli = CLI 전무 → 빈 셸 유지(온보딩 보존·기능1).
  pending-resource = 곱셈 자원 예산 초과(hard) — 대기·자동 재시도.

ensure(socket): ①cys gate-check(**fail-closed** — exit 0 에서만 진행 · paused·판정 불능 모두
    편성 보류 종료) ②소켓키 싱글플라이트 락
  ③probe_cli(로그인셸 우산) 역할별 CLI 판정 ③′라이브 로스터 관측·분류 — **이미 complete 면 자원
    게이트를 보지 않고 즉시 complete 기록·표면화**(신규 스폰 0 = 자원 소비 0. 구 순서는 complete 레인을
    pending-resource 로 조기 반환해 배너 소멸 신호를 영구 차단했다)
  ④complete 가 아닐 때만 javis_resource_gate.py check(hard→pending-resource)
  ⑤master 자리: 죽었으면 new-surface --role master 재생성 / 빈 셸이면 javis_boot_node 입양경로로
    claude 부착 / 이미 각성이면 no-op ⑥나머지 역할 javis_boot_node 재사용(CSO 먼저) ⑦상태 파일
    갱신 + feed 표면화(침묵 금지) ⑧멱등(이미 기동된 역할 재기동 금지 — boot_node already_up).
  모든 실패는 graceful(비0 exit 이되 예외 스택 아닌 명시 메시지).

외부 역할 제외(env CYS_FORMATION_EXTERNAL_ROLES · 2026-08-01): 콤마 구분 역할 목록에 든 역할은
  **cys 밖 외부 세션이 담당** 한다고 보고 (a)필요 로스터에서 빼고(결원으로 세지 않음)
  (b)좌석 생성·부착을 하지 않는다. 기본 빈값 = 현행 동작 완전 보존(제외 0).
  배경: 실제 master 가 cys 밖 외부 세션인 레인에서 formation 이 master 를 결원으로 오판해
  빈 zsh 좌석을 만들면, 그 유령 좌석이 `--to master` 트래픽을 삼킨다(유령 마스터).
  ★REQUIRED_ROLES 상수 자체는 무수정 — 제외는 호출 시점 파생값이라 env 를 지우면 즉시 원복된다.

저장: <state>/formation/<socket-key>.json  (state = CYS_STATE_DIR or ~/.cys/state — 기존 관례).

CLI:
  python3 javis_formation.py ensure --socket <S> [--cwd D] [--json] [--force-surface]
    --force-surface = 전이 없어도 현재 상태 1회 표면화(앱 부트 레인 전용 · 주기 잡 금지)
  python3 javis_formation.py classify --installed claude,agy --live master,cso [--no-resource]
  python3 javis_formation.py self-test
"""
import json
import os
import subprocess
import sys
import time

SELF_DIR = os.path.dirname(os.path.abspath(__file__))
# ★형제 모듈 경로 가드 — **모듈 레벨**이어야 한다(2026-08-01 적대검증 A3 수리).
#   종전에는 이 가드가 _installed_clis() 안, 그것도 `import javis_cli_probe` 실패 뒤
#   ImportError 핸들러에 있었다 → **첫 해소 시도가 무방비**라 cwd·타 팩의 동명 모듈이
#   먼저 바인딩될 수 있었다(가드는 그 뒤에야 걸리므로 이미 늦다). 가드를 sibling import
#   **앞**으로 올려 첫 시도부터 우리 폴더가 우선하게 만든다.
#   ※형태는 기존 줄(`insert(0, SELF_DIR)`)의 **순수 호이스트**다 — 연산은 그대로 두고
#     실행 시점만 옮긴다. test_import_guard.py §가드 형태 규약은 신규 작성분에 append 를
#     권장하되 기존 insert(0) 선례는 강제 교정하지 않는다(같은 파일 :91).
#   unix/mac 은 스크립트 폴더가 이미 sys.path[0] 이라 이 블록은 무동작(멱등).
if SELF_DIR not in sys.path:
    sys.path.insert(0, SELF_DIR)

PACK_DIR = os.environ.get("CYS_PACK_DIR") or os.path.join(
    os.path.expanduser("~"), ".cys", "pack")

# ── 상태 enum(test_formation #1) ──
STATES = ["complete", "partial", "pending-cli", "pending-resource", "failed"]

# 의무 편성 로스터 + 역할→CLI 바이너리 매핑(probe_cli 대상 = claude/agy/codex).
# ★이 상수는 불변이다 — 외부 세션이 맡는 역할의 제외는 env 파생값(effective_required_roles) 이다.
REQUIRED_ROLES = ("master", "cso", "worker", "reviewer-gemini", "reviewer-codex")
ROLE_CLI = {
    "master": "claude", "cso": "claude", "worker": "claude",
    "reviewer-gemini": "agy", "reviewer-codex": "codex",
}
# 노드 부착 시 javis_boot_node 에 넘길 --agent 이름(gemini 리뷰어의 CLI 바이너리는 agy).
ROLE_AGENT = {
    "master": "claude", "cso": "claude", "worker": "claude",
    "reviewer-gemini": "gemini", "reviewer-codex": "codex",
}
REQUIRED_CLIS = {"claude", "agy", "codex"}

# kill-switch(paused) 존중 훅 표식(test_formation #7 fallback).
PAUSE_HONORED = True


def _state_root():
    """편성 상태·락 저장 루트: <CYS_STATE_DIR or ~/.cys/state>/formation.
    ★불변식(리뷰어1 권고): 소켓키 싱글플라이트 상호배제는 **모든 트리거(cys-dept·bootstrap·schedule
    심박)가 동일한 CYS_STATE_DIR을 볼 때만** 성립한다 — 락 파일 경로가 여기서 파생되므로. 따라서 이
    env를 트리거별로 명시 재설정하지 마라(데몬이 자식 pane에 전파하는 값을 상속만 한다). 테스트는
    CYS_STATE_DIR을 temp로 고정해 격리하되, 프로덕션 트리거는 env를 건드리지 않는다."""
    return os.path.join(
        os.environ.get("CYS_STATE_DIR") or os.path.join(
            os.path.expanduser("~"), ".cys", "state"),
        "formation")


def _sanitize_key(socket):
    """소켓 경로 → 파일명 안전 키(부서 basename 동일 충돌 회피 — 전체 경로 기반)."""
    import hashlib
    s = socket or "base"
    safe = "".join(c if (c.isalnum() or c in "-._") else "_" for c in s)
    if len(safe) > 120:
        safe = safe[:100] + "-" + hashlib.sha256(s.encode("utf-8")).hexdigest()[:16]
    return safe or "base"


def _run(argv, timeout=30):
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout or ""), (r.stderr or "")
    except Exception as e:
        return 127, "", str(e)


# ── ① kill-switch(gate-check) — ★fail-closed 봉인(2026-08-01 F1 · 제품 불변식) ──
PAUSED_BASENAME = "AUTOPILOT_PAUSED"
# 판정 불능일 때만 유효한 **명시 opt-in** 우회 스위치(기본 미설정 = fail-closed).
GATE_BYPASS_ENV = "CYS_FORMATION_IGNORE_GATE"


class _GateVerdict(int):
    """gate_check() 반환값 — bool 처럼 쓰이면서 **보류 사유**를 함께 나른다.

    종전 계약(bool)과 완전 호환이다: 진릿값이 1/0 이라 `if not gate_check():` 가 그대로 돌고,
    True/False 를 돌려주는 기존 스텁(test_formation._ensure_harness)도 무수정 동작한다.
    호출부는 사유를 `getattr(v, "reason", "")` 로 **선택적으로** 읽는다 — 평범한 bool 이 와도
    빈 문자열로 안전하게 흐른다(스텁 호환의 근거).
    ※사유를 반환값에 실은 이유: ensure 가 소비하는 seam 을 gate_check() 하나로 유지하기 위해서다.
      별도 조회 함수를 ensure 가 직접 부르면 gate_check 스텁이 무력화돼 테스트가 라이브 데몬을
      건드리게 된다(밀폐 파괴).
    """

    def __new__(cls, allowed, code=None, reason=""):
        o = int.__new__(cls, 1 if allowed else 0)
        o.allowed = bool(allowed)
        o.code = code
        o.reason = reason
        return o

    def __repr__(self):
        return "<GateVerdict allowed=%r code=%r reason=%r>" % (
            self.allowed, self.code, self.reason)


def _truthy_env(name):
    return (os.environ.get(name) or "").strip().lower() in ("1", "true", "yes", "on")


def paused_paths():
    """kill-switch 파일 2경로(§9-4 2중 기록 규약 — javis_wakeup·javis_cycle_autopilot 와 동일 식).
    하나라도 존재하면 paused 다 — 데몬이 죽어 gate-check 가 응답 못 해도 정지가 관철된다."""
    root = os.environ.get("JAVIS_ROOT") or os.getcwd()
    return [os.path.join(PACK_DIR, PAUSED_BASENAME),
            os.path.join(root, "_round", PAUSED_BASENAME)]


def _paused_file_hit():
    """PAUSED 파일 적중 경로(문자열) 또는 None. 확인 불능(OSError)도 적중으로 본다(fail-closed)."""
    for p in paused_paths():
        try:
            if os.path.exists(p):
                return p
        except OSError as e:
            return "%s (확인 불능: %s)" % (p, e)
    return None


def _gate_verdict(code, err="", paused_file=None, bypass=False):
    """gate 판정 **순수 함수**(subprocess·파일 접촉 0 · self-test 핀 대상).

    계약(2026-08-01 F1 신):
      PAUSED 파일 적중            → False (gate-check 결과와 무관 · 우회 불가)
      exit 0(running)             → True  (**진행은 살아있는 허가가 있을 때만**)
      exit 4(paused)              → False (kill-switch 존중 · 우회 불가)
      그 외(비0·비4·타임아웃·바이너리 부재 127) → False = fail-closed
                                    단 CYS_FORMATION_IGNORE_GATE=1 이면 True(명시 opt-in 우회)

    구(폐기) 계약은 'exit 4 만 False, 그 외 전부 True(보수적 진행)' 였다 — 데몬 부재·조회 실패
    한 번에 **일시정지 여부를 모르는 채** 노드를 기동시켰다(격리 실측: exit 3 → launch-agent 5건).
    측정 불능은 어떤 게이트에서도 통과가 아니다.

    ★우회가 '판정 불능'에만 적용되는 이유: 명시적 kill-switch(exit 4·PAUSED 파일)까지 env 로
      풀어주면 봉인이 아니라 뒷문이 된다. 자기잠금 방지는 '모르는 상태'에서만 필요하다."""
    if paused_file:
        return _GateVerdict(False, 4, "kill-switch(paused) — 편성 보류(PAUSED 파일: %s)"
                            % paused_file)
    if code == 0:
        return _GateVerdict(True, 0, "")
    if code == 4:
        return _GateVerdict(False, 4, "kill-switch(paused) — 편성 보류(gate-check 존중 · exit 4)")
    base = "gate-check 판정 불능(exit %s)" % code
    tail = (err or "").strip().replace("\n", " ")[:120]
    if tail:
        base += " · stderr=%s" % tail
    if bypass:
        return _GateVerdict(True, code, base + " — %s=1 명시 우회로 진행(fail-closed 해제)"
                            % GATE_BYPASS_ENV)
    return _GateVerdict(False, code, base + " — 안전상 편성 보류. 데몬 확인 후 재시도"
                        "(강제 진행: %s=1)" % GATE_BYPASS_ENV)


def gate_check():
    """cys gate-check → 편성 진행 허용 여부(_GateVerdict · bool 호환). 판정식은 _gate_verdict 참조.
    PAUSED 파일이 적중하면 데몬을 조회하지 않는다(파일 존재 = paused · 데몬 무응답에도 정지 관철)."""
    hit = _paused_file_hit()
    if hit:
        return _gate_verdict(4, "", hit, False)
    code, _out, err = _run(["cys", "gate-check"], timeout=10)
    return _gate_verdict(code, err, None, _truthy_env(GATE_BYPASS_ENV))


# ── ② 소켓키 싱글플라이트 락(fcntl/msvcrt — cys-dept reg_upsert 패턴 재사용·Sim S2-3/S2-6) ──
try:
    import fcntl

    def _lk(f):
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def _ulk(f):
        try:
            fcntl.flock(f, fcntl.LOCK_UN)
        except OSError:
            pass
except ImportError:  # Windows: fcntl 부재 → msvcrt 바이트락(파일 닫힘 시 해제)
    try:
        import msvcrt

        def _lk(f):
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)

        def _ulk(f):
            try:
                f.seek(0)
                msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, 1)
            except OSError:
                pass
    except ImportError:
        def _lk(f):
            pass

        def _ulk(f):
            pass


class _singleflight(object):
    """소켓키별 싱글플라이트 락 컨텍스트 — 동시 2회(연타+심박) 진입 시 1회만 편성.
    획득 실패(이미 진행 중) → acquired=False (호출부는 no-op 종료)."""

    def __init__(self, socket):
        root = _state_root()
        os.makedirs(root, exist_ok=True)
        self.path = os.path.join(root, ".lock-" + _sanitize_key(socket))
        self._f = None
        self.acquired = False

    def __enter__(self):
        try:
            self._f = open(self.path, "a+")
            _lk(self._f)
            self.acquired = True
        except (OSError, IOError):
            self.acquired = False
            if self._f:
                try:
                    self._f.close()
                except OSError:
                    pass
                self._f = None
        return self

    def __exit__(self, *exc):
        if self._f:
            _ulk(self._f)
            try:
                self._f.close()
            except OSError:
                pass
            self._f = None
        return False


# 하위호환 별칭(계약 테스트가 acquire_lock 도 허용).
def acquire_lock(socket):
    return _singleflight(socket)


# ── ③ 상태 판정(순수 함수 · test_formation #2~#5) ──
def classify(installed=None, live=None, resource_ok=True, external_roles=None):
    """설치 CLI 집합 · 라이브 역할 집합 · 자원 여유 → 상태 문자열.
      resource_ok False               → pending-resource
      installed 공집합(CLI 전무)        → pending-cli:{roles}
      의무 5역할 전부 live ∧ 필수 CLI 전부 → complete
      그 외(일부 설치·부팅 중)          → partial:{missing_clis|booting}
    complete 는 master+4종 전부일 때만(Sim S2-5).

    external_roles=None → env CYS_FORMATION_EXTERNAL_ROLES 반영(미설정=공집합=현행 동작 동일).
    제외된 역할은 **결원으로 세지 않는다** — 외부 세션이 담당하므로 그 부재가 complete 를 막지 않는다.
    ★REQUIRED_CLIS 는 파생시키지 않는다(무수정): master 제외 시에도 남은 cso·worker 가 claude 를
      쓰므로 필수 CLI 집합은 동일하고, 정적으로 두는 편이 부분 설치 오판(Sim S2-5) 차단에 보수적이다."""
    installed = set(installed or [])
    live = set(live or [])
    if not resource_ok:
        return "pending-resource"
    required = set(effective_required_roles(external_roles))
    missing_clis = sorted(REQUIRED_CLIS - installed)
    if not installed:
        return "pending-cli:" + ",".join(sorted(required))
    if required.issubset(live) and not missing_clis:
        return "complete"
    if missing_clis:
        return "partial:" + ",".join(missing_clis)
    # 필수 CLI 전부 설치·자원 여유이나 아직 전 역할 미기동 → 편성 진행 중.
    return "partial:booting"


def state_kind(state):
    """상태 문자열 → 접두(complete/partial/pending-cli/pending-resource/failed)."""
    return state.split(":", 1)[0]


# ── W3(T3): 배너 수명 신호 매핑 — feed --kind(UI 배너 판정) · EVT 타입(음성/HUD 트랙) ──
# UI 배너 소멸/갱신은 데몬 feed 버스(feed.item.created, payload.kind)가 담당하고(ui bootbanner.ts),
# EVT 는 별개 층(음성·HUD, javis_event.py 계약)이다 — handlers.rs:2814 층 분리 주석과 정합.
_STATE_FEED_KIND = {"complete": "formation-complete", "partial": "formation-partial",
                    "pending-cli": "formation-pending", "pending-resource": "formation-pending",
                    "failed": "formation-failed"}
_STATE_EVT = {"complete": "boot.formation.complete", "partial": "boot.formation.partial",
              "pending-cli": "boot.formation.pending", "pending-resource": "boot.formation.pending",
              "failed": "boot.formation.failed"}


def feed_kind_for_state(state):
    """상태 → cys feed --kind. complete=배너 소멸·partial=배너 갱신·그 외=정보성(순수·테스트 대상)."""
    return _STATE_FEED_KIND.get(state_kind(state), "formation")


def evt_type_for_state(state):
    """상태 → EVT v2 타입(boot.formation.*) 또는 None(순수·테스트 대상)."""
    return _STATE_EVT.get(state_kind(state))


# ── 라이브 로스터 관측(cys status --json · agent 생존 인지) ──
def _canonical_role(role):
    """변형 역할(cso-1·worker-2·reviewer-gemini-x)을 의무 역할 기준으로 귀속(순수·self-test 핀)."""
    if role == "master":
        return "master"
    if role == "cso" or role.startswith("cso-"):
        return "cso"
    if role == "worker" or role.startswith("worker-"):
        return "worker"
    if role.startswith("reviewer-gemini"):
        return "reviewer-gemini"
    if role.startswith("reviewer-codex"):
        return "reviewer-codex"
    return role


# ── 외부(cys 밖) 역할 제외 — env CYS_FORMATION_EXTERNAL_ROLES (2026-08-01) ──
def _external_roles():
    """env CYS_FORMATION_EXTERNAL_ROLES → 편성에서 제외할 역할 집합(콤마 구분·공백 허용).

    미설정/빈값 = 공집합 = **현행 동작 완전 보존**(어떤 판정·생성 경로도 달라지지 않는다).
    값은 `_canonical_role` 로 정규화하므로 `worker-2` 같은 변형 표기도 의무 역할로 귀속된다.
    미지 역할 문자열은 아무것에도 매칭되지 않아 무해(하드 실패 없음 — 편성이 env 오타로 멈추면 안 된다)."""
    raw = os.environ.get("CYS_FORMATION_EXTERNAL_ROLES") or ""
    return frozenset(_canonical_role(x.strip()) for x in raw.split(",") if x.strip())


def effective_required_roles(external=None):
    """제외 후 실제 편성 대상 역할(REQUIRED_ROLES 순서 보존 튜플).

    external=None → env 조회(운영 기본) / external=() → 제외 없음(순수 호출·테스트용)."""
    ext = _external_roles() if external is None else set(external)
    return tuple(r for r in REQUIRED_ROLES if r not in ext)


def _external_note():
    """detail·상태파일용 제외 표기(관측 가능성). 미설정이면 **빈 문자열** = detail 바이트 동일."""
    ext = sorted(_external_roles())
    return (" · external-roles 제외: %s" % ",".join(ext)) if ext else ""


def _roster_from_status(obj, require_live_agent=True):
    """`cys status --json`(org.status) surfaces → 역할 집합(순수·self-test 핀).

    ★agent 사망 인지(2026-07-27 hotfix): 종전 판정은 'pane 에 role 이 있으면 생존'이라, agent 가
      전부 죽고 셸만 남은 부서(라이브 실측: 8노드 agent_alive=false)를 **complete 로 오판**해
      편성 복구가 영영 발화하지 않았다. agent_alive is False = 그 좌석의 agent 사망 → 역할 부재로
      본다(재기동 대상). agent_alive is None 은 '등록된 agent 없음'(수동 new-surface 빈 셸)이라
      종전 의미(좌석 점유=생존)를 그대로 둔다 — 온보딩·입양 경로(boot_node)가 그 위에 서 있다.
    require_live_agent=False 는 좌석 존재만 묻는 관측(=구 동작) — master 좌석 재생성 판단 전용."""
    roles = set()
    for s in (obj or {}).get("surfaces") or []:
        if s.get("exited"):
            continue
        role = s.get("role") or ""
        if not role:
            continue
        if require_live_agent and s.get("agent_alive") is False:
            continue
        roles.add(_canonical_role(role))
    return roles


def _roster_from_list_tsv(text):
    """`cys list` TSV → 역할 집합(순수·구 폴백 경로). agent 생존은 이 출력에 없다(=agent 맹목)."""
    roles = set()
    for line in (text or "").splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited 무시
            continue
        if not role:
            continue
        roles.add(_canonical_role(role))
    return roles


def _live_roles(socket, require_live_agent=True):
    """라이브 역할 집합. 파싱 불가/데몬 부재 → None.

    1차 `cys status --json`(agent_alive 노출 = agent 사망 인지) · 실패 시 `cys list` TSV 폴백
    (구 동작 = agent 맹목). 폴백은 새 결함을 만들지 않는다 — 종전과 같은 판정으로 되돌아갈 뿐이다."""
    env = dict(os.environ)
    if socket:
        env["CYS_SOCKET"] = socket
    try:
        r = subprocess.run(["cys", "status", "--json"], capture_output=True, text=True,
                           timeout=15, env=env)
        if r.returncode == 0:
            return _roster_from_status(json.loads(r.stdout or "{}"), require_live_agent)
    except Exception:
        pass
    try:
        r = subprocess.run(["cys", "list"], capture_output=True, text=True,
                           timeout=15, env=env)
    except Exception:
        return None
    if r.returncode != 0:
        return None
    return _roster_from_list_tsv(r.stdout or "")


def _installed_clis():
    """probe_cli(로그인셸 우산) 로 claude/agy/codex 설치 여부 판정 → 설치된 CLI 집합.
    ★경로 가드는 모듈 최상단(SELF_DIR 직후)에 있다 — 재시도 사다리를 두지 않는다.
      사다리는 '첫 시도는 무방비'를 구조적으로 내포했다(A3). 부재는 그냥 set()."""
    try:
        import javis_cli_probe
    except ImportError:
        return set()
    return {c for c in REQUIRED_CLIS if javis_cli_probe.probe_cli(c)}


# ── ④ 곱셈 자원 예산(W6 접점 — hard=pending-resource) ──
def _resource_ok(socket):
    """javis_resource_gate.py check → 편성 착수 자원 여유(hard=False). 게이트 부재/내부오류=True(보수).
    ★W6 곱셈 편성 예산(T4): --formation-size 로 이 레인 편성 크기를 전달 → 게이트가 투영(측정 노드 +
    부서수×편성크기)을 CYS_FORMATION_BUDGET 과 대조해 초과 시 hard(exit 2)→여기서 False→pending-resource."""
    gate = os.path.join(PACK_DIR, "bin", "javis_resource_gate.py")
    if not os.path.isfile(gate):
        return True
    py = sys.executable or "python3"
    code, _out, _err = _run(
        [py, gate, "check", "--json", "--formation-size", str(len(REQUIRED_ROLES))], timeout=30)
    # exit 2 = hard block(자원 hard 또는 W6 예산 초과). 그 외(0 allow·1 soft·기타 내부오류)=진행.
    return code != 2


# ── ⑤⑥ 노드 부착(전부 javis_boot_node 재사용 — 신규 spawn 로직 0) ──
def _boot_node(role, socket, cwd=None, timeout=200):
    """javis_boot_node 재사용 — 입양 경로(점유만·미각성 seat 에 claude 부착)·already_up 멱등.
    반환: (ok bool, detail)."""
    # ★외부 역할 방어선(2026-08-01): 외부 세션이 담당하는 역할에는 좌석을 만들지도 붙이지도 않는다.
    #   ensure 의 order 필터가 1차 차단이고 이건 다른 호출자까지 덮는 2차 방어선이다(env 미설정=무영향).
    if role in _external_roles():
        return False, "external-role(%s) — 좌석 생성·부착 skip(외부 세션 담당)" % role
    node = os.path.join(PACK_DIR, "bin", "javis_boot_node.py")
    if not os.path.isfile(node):
        return False, "javis_boot_node.py 부재"
    py = sys.executable or "python3"
    argv = [py, node, "--role", role, "--agent", ROLE_AGENT.get(role, "claude"), "--json"]
    if cwd:
        argv += ["--cwd", cwd]
    env = dict(os.environ)
    if socket:
        env["CYS_SOCKET"] = socket
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, env=env)
        return r.returncode == 0, (r.stdout or r.stderr or "").strip()[:200]
    except Exception as e:
        return False, "boot_node 예외: %s" % e


def _ensure_master_seat(socket, cwd):
    """master 자리 확보: seat 닫힘 → new-surface --role master 재생성(node-recover·Sim S2-7).
    빈 셸/미각성 → boot_node 입양 경로가 claude 부착. 이미 각성 → no-op(already_up)."""
    # ★외부 역할 방어선(2026-08-01): master 가 cys 밖 외부 세션이면 **좌석을 만들지 않는다**.
    #   빈 zsh 좌석은 `--to master` 트래픽을 삼키는 유령이 된다(이 잡의 존재 이유). env 미설정=무영향.
    if "master" in _external_roles():
        return False, "external-role(master) — new-surface·입양 skip(외부 세션 담당)"
    # ★좌석 존재만 묻는다(require_live_agent=False): agent 가 죽은 master 좌석은 **재생성 대상이
    # 아니라 입양 대상**이다 — 살아있는 좌석에 new-surface --role master 를 또 쏘면 role 점유 충돌·
    # litter surface 가 된다. 죽은 agent 의 복구는 아래 _boot_node(입양 경로)가 담당한다.
    roles = _live_roles(socket, require_live_agent=False)
    env = dict(os.environ)
    if socket:
        env["CYS_SOCKET"] = socket
    if roles is None or "master" not in roles:
        # seat 부재로 판단 → 재생성(빈 셸). 이후 boot_node 입양이 claude 부착.
        argv = ["cys", "new-surface"]
        if socket:
            argv += ["--socket", socket]
        argv += ["--role", "master"]
        if cwd:
            argv += ["--cwd", cwd]
        _run(argv, timeout=20)  # 실패해도 boot_node 가 후속 판정 — graceful
    # 입양/각성은 boot_node 가 담당(입양 경로: 점유만 됐고 미각성이면 입양→주입).
    return _boot_node("master", socket, cwd)


# ── ⑦ 상태 파일 + feed 표면화(침묵 금지 C6) ──
def _write_state(socket, state, detail, roles_booted):
    root = _state_root()
    try:
        os.makedirs(root, exist_ok=True)
    except OSError:
        return None
    path = os.path.join(root, _sanitize_key(socket) + ".json")
    obj = {"state": state, "kind": state_kind(state), "socket": socket or "",
           "detail": detail, "roles_booted": sorted(roles_booted),
           "updated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
           "updated_epoch": time.time()}
    # 외부 역할 제외의 기계 판독 표기(관측 가능성). ★비어 있으면 키를 넣지 않는다 —
    # env 미설정 레인의 상태파일이 종전과 **바이트 동일** 해야 회귀 판정이 가능하다.
    _ext = sorted(_external_roles())
    if _ext:
        obj["external_roles"] = _ext
    tmp = path + ".tmp"
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as f:
            json.dump(obj, f, ensure_ascii=False, indent=1)
            f.write("\n")
        os.replace(tmp, path)
    except OSError:
        return None
    return path


def _feed(title, body, kind="formation"):
    """편성 상태 표면화(데몬 feed 버스 → UI onDaemonEvent). kind=formation-*(배너 수명 신호).
    데몬 부재 등 실패는 무시(best-effort·부트 무해)."""
    _run(["cys", "feed", "push", "--kind", kind, "--title", title, "--body", body],
         timeout=10)


def _feed_for_state(state):
    kind = state_kind(state)
    fk = feed_kind_for_state(state)   # UI 배너 판정용 --kind(complete=소멸·partial=갱신)
    if kind == "pending-cli":
        _feed("부서 팀 편성 대기(CLI 미설치)",
              "claude CLI 미설치 — 빈 셸 master 자리는 유지됩니다(팀 없이도 마스터 단독 사용 "
              "가능). `curl -fsSL https://claude.ai/install.sh | bash` 로 설치하면 자동으로 "
              "편성이 완결됩니다.", fk)
    elif kind == "partial":
        _feed("부서 팀 부분 편성", "설치된 CLI 로 가능한 노드만 기동했습니다(%s). 나머지 CLI "
              "설치 시 자동으로 편성이 완결됩니다." % state, fk)
    elif kind == "pending-resource":
        _feed("부서 팀 편성 대기(자원)", "곱셈 자원 예산 초과 — 자원 정리 후 자동 재시도됩니다.", fk)
    elif kind == "complete":
        _feed("부서 팀 편성 완결", "master + 4종 의무 노드(cso·worker·reviewer-gemini·"
              "reviewer-codex) 전부 기동 완료.", fk)
    elif kind == "failed":
        _feed("부서 팀 편성 실패", "편성에 실패했습니다(%s) — boot-last.json·formation 상태 "
              "파일에서 원인을 확인하세요." % state, fk)


def _read_state(socket):
    """저장된 이전 상태 문자열(전이 감지용) — 없으면 None. best-effort."""
    path = os.path.join(_state_root(), _sanitize_key(socket) + ".json")
    try:
        with open(path, encoding="utf-8") as f:
            return (json.load(f) or {}).get("state")
    except (OSError, ValueError):
        return None


def _emit_evt(evt_type, fields):
    """javis_event.py emit(EVT v2 · 음성/HUD 트랙 — UI 배너와 별개 층). best-effort·부트 무해(R10)."""
    ev = os.path.join(SELF_DIR, "javis_event.py")
    if not os.path.isfile(ev):
        ev = os.path.join(PACK_DIR, "bin", "javis_event.py")
        if not os.path.isfile(ev):
            return
    py = sys.executable or "python3"
    argv = [py, ev, "emit", evt_type, "--spool"]
    for k, v in fields.items():
        argv += ["--field", "%s=%s" % (k, v)]
    _run(argv, timeout=10)


def _surface(socket, prev_state, state, force=False):
    """상태 표면화 — (1) UI 배너 수명 신호(feed --kind) (2) 음성/HUD EVT. kind 전이 또는 최초
    관측 시에만 발화(심박 스팸 억제·'전이 시' 계약). 전부 best-effort·부트 무해(R10).

    ★force=True(앱 부트 레인 전용·2026-07-26): 전이 여부와 무관하게 현재 상태를 1회 표면화한다.
    UI 의 배너 중복 억제(bootbanner.ts lastFormationKind)는 **인메모리**라 앱 재시작 시 소실되는데,
    상태 파일이 이미 complete 인 레인은 전이가 없어 formation-complete 를 다시 발행하지 않는다 →
    새 앱 세션에서 boot-warning 배너가 뜨면 소멸 신호가 영영 오지 않았다. 부트 1회만 강제 재발행해
    그 갭을 닫는다. ⚠주기 스케줄 잡(10분)에는 절대 붙이지 마라 — 매 틱 토스트 스팸이 되살아난다.
    INV-1 불변: 강제 표면화는 상태를 계산·변경하지 않는다(발행 kind 는 호출자가 준 state 그대로)."""
    if not force and prev_state is not None and state_kind(prev_state) == state_kind(state):
        return  # 상태 kind 무변화 → 재표면화 생략(배너 dismiss 는 멱등이나 스팸 억제)
    _feed_for_state(state)
    evt = evt_type_for_state(state)
    if evt:
        _emit_evt(evt, {"socket": socket or "", "state": state, "kind": state_kind(state)})


# ── ensure: 상태 기계 전이 주체 ──
def ensure(socket=None, cwd=None, force_surface=False):
    """편성 전이(멱등·싱글플라이트). 반환: (state, detail). 모든 실패 graceful.

    force_surface=True 면 kind 전이가 없어도 현재 상태를 1회 표면화한다(_surface 주석 참조 —
    앱 부트 레인 전용. 주기 잡은 기본값 False 로 스팸 억제 계약을 유지한다). 상태 판정·기록에는
    아무 영향이 없다(INV-1: complete 는 여전히 classify()==complete 일 때만)."""
    prev = _read_state(socket)   # 전이 감지용(배너 수명 신호는 전이 시에만 표면화)
    # ① kill-switch — ★fail-closed(2026-08-01 F1): 허용은 exit 0 에서만, 판정 불능도 보류다.
    gate = gate_check()
    if not gate:
        # 보류: 현 관측 기준 상태를 계산해 기록만(신규 기동 0 — 관측·저장·보고는 pause 중 허용 범위).
        installed = _installed_clis()
        live = _live_roles(socket) or set()
        state = classify(installed=installed, live=live, resource_ok=True)
        # exit 4·PAUSED 파일 = 확정 정지(paused) / 그 외 = 판정 불능(gate-unknown) — 상태 문자열로 구분.
        hold = "paused" if getattr(gate, "code", 4) == 4 else "gate-unknown"
        if state == "complete":
            state = "partial:" + hold  # 보류 중엔 complete 로 승격하지 않음
        # ★자기잠금 방지: 무엇에 막혔는지를 상태파일 detail 과 stdout(_cmd_ensure JSON)에 남긴다.
        detail = (getattr(gate, "reason", "")
                  or "kill-switch(paused) — 편성 보류(gate-check 존중)") + _external_note()
        _write_state(socket, state, detail, live)
        return state, detail

    # ② 싱글플라이트 락
    with _singleflight(socket) as lock:
        if not lock.acquired:
            return "partial:inflight", "다른 편성 진행 중(싱글플라이트) — no-op"

        # ③ CLI 가용성(로그인셸 우산 프로브 — 이 ensure 에서 1회만 수행하고 이하 전 경로가 재사용)
        installed = _installed_clis()

        # ③′ ★로스터 우선 판정(자원 게이트보다 먼저 — 2026-07-26 배너 불멸 수리).
        #    이미 complete(5역할 전원 생존)면 **새로 뜰 노드가 0** 이므로 자원 게이트를 볼 이유가 없다.
        #    구 순서(자원 먼저)는 complete 인 레인을 pending-resource 로 조기 반환시켜 배너 소멸 신호
        #    (formation-complete)를 영원히 발행하지 못하게 했다(라이브 실측: classify=complete ∧
        #    _resource_ok=False → pending-resource 기록).
        #    ★불변식 INV-1: complete 는 **오직 classify()==complete** 일 때만 — 아래 어떤 경로도
        #    formation-complete 를 발행하지 않는다(pending-cli·partial 은 기존 kind 유지 = 기능1
        #    인앱 CLI 설치 온보딩 안내 보존).
        live_now = _live_roles(socket)
        if live_now is not None and classify(installed=installed, live=live_now,
                                             resource_ok=True) == "complete":
            state = "complete"
            detail = ("라이브 로스터 이미 완결(5역할 생존) — 신규 기동 0 · 자원 게이트 무관"
                      + _external_note())
            _write_state(socket, state, detail, live_now)
            _surface(socket, prev, state, force=force_surface)
            return state, detail

        # ④ 자원 게이트(complete 가 **아닐 때만** — 실제로 노드를 스폰하는 경로에서만 예산을 본다)
        if not _resource_ok(socket):
            live = live_now or set()   # ③′ 관측 재사용(cys list 중복 호출 0)
            state = "pending-resource"
            detail = "곱셈 자원 예산 hard — 편성 대기" + _external_note()
            _write_state(socket, state, detail, live)
            _surface(socket, prev, state, force=force_surface)
            return state, detail

        # CLI 전무 → pending-cli(빈 셸 유지·온보딩 보존)
        if not installed:
            live = live_now or set()   # ③′ 관측 재사용(cys list 중복 호출 0)
            state = classify(installed=installed, live=live, resource_ok=True)
            detail = "CLI 미설치 — 빈 셸 유지·편성 대기(설치 시 자동 완결)" + _external_note()
            _write_state(socket, state, detail, live)
            _surface(socket, prev, state, force=force_surface)
            return state, detail

        # ⑤⑥ 설치된 CLI 기준 최대 편성. master 먼저(입양 경로) → CSO → 나머지.
        booted = set()
        # ★외부 역할은 order 에서 뺀다(2026-08-01) — 생성 자체를 하지 않는다. env 미설정이면
        #   effective_required_roles() == REQUIRED_ROLES 라 종전 순서·집합과 완전히 동일하다.
        order = list(effective_required_roles())
        for role in order:
            cli = ROLE_CLI[role]
            if cli not in installed:
                continue  # 해당 CLI 미설치 = 이 역할 skip(partial)
            if role == "master":
                ok, _d = _ensure_master_seat(socket, cwd)
            else:
                ok, _d = _boot_node(role, socket, cwd)
            if ok:
                booted.add(role)

        # ⑦ 상태 판정·기록·표면화(라이브 재관측 우선)
        live = _live_roles(socket)
        if live is None:
            live = booted
        state = classify(installed=installed, live=live, resource_ok=True)
        detail = "편성 실행 — 기동=%s · 설치CLI=%s" % (
            ",".join(sorted(booted)) or "없음", ",".join(sorted(installed))) + _external_note()
        _write_state(socket, state, detail, live)
        # ★주기 실행 스팸 차단(2026-07-26): 무조건 _feed_for_state 였던 자리를 _surface(전이 시에만
        # 표면화) 로 교체한다. 편성 ensure 가 스케줄 주기(10분)로 붙으면 매 틱마다 사용자에게 토스트가
        # 갔다. prev 는 ensure 진입부에서 읽은 직전 상태 — 동일 kind 면 _surface 계약대로 생략된다.
        _surface(socket, prev, state, force=force_surface)
        return state, detail


# ── 두 경로 동등성 계획(plan_roster · test_parity_paths) ──
def _directives_dir():
    """계획용 디렉티브 디렉토리 — env/live 팩에 directives 가 있으면 그것, 없으면 모듈 동봉
    (cysjavis-pack/directives). 두 경로가 같은 소스를 읽으므로 해시가 구조적으로 동일."""
    for cand in (os.path.join(PACK_DIR, "directives"),
                 os.path.join(os.path.dirname(SELF_DIR), "directives")):
        if os.path.isdir(cand):
            return cand
    return os.path.join(PACK_DIR, "directives")


_ROLE_DIRECTIVE_FILE = {
    "master": "MASTER_DIRECTIVE.md", "cso": "CSO_DIRECTIVE.md",
    "worker": "WORKER_DIRECTIVE.md", "reviewer-gemini": "REVIEWER_DIRECTIVE.md",
    "reviewer-codex": "REVIEWER_DIRECTIVE.md",
}


def _sha256_file(path):
    import hashlib
    try:
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except OSError:
        return None


def _plan_directive_hashes():
    d = _directives_dir()
    return {role: _sha256_file(os.path.join(d, fn))
            for role, fn in _ROLE_DIRECTIVE_FILE.items()}


def _c03_pass():
    """MASTER_DIRECTIVE 에 표준 핀 조항이 살아있는가(C03.pin.master 취지)."""
    p = os.path.join(_directives_dir(), "MASTER_DIRECTIVE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return False
    # 표준 핀 대표 마커(preflight CONTENT_PINS 부분집합 — 존재로 C03 취지 확인).
    return ("javis_preflight" in text) and ("4종 의무 노드" in text) and ("master" in text)


class RosterPlan(object):
    """두 경로(버튼·수동)가 산출하는 편성 계획 — 동일 ensure 로 수렴(ensure_fn 동일)."""

    def __init__(self, path):
        self.path = path
        self.ensure_fn = ensure  # 두 경로 동일 함수 → 구조적 수렴(C3)
        self.roles = set(REQUIRED_ROLES)
        self.directive_hashes = _plan_directive_hashes()
        self.c03_pass = _c03_pass()


def plan_roster(path="button"):
    """경로별(button/manual) 편성 계획 반환 — 두 경로가 동일 로스터·주입 디렉티브 해시·C03 을 냄.
    품질 앵커(R2-4): 판정 기준은 노드 수가 아니라 주입 지침의 온전함(해시 대조)."""
    return RosterPlan(path)


# ── self-test(순수 판정 회귀) ──
def self_test():
    fails = []

    def ck(cond, msg):
        if not cond:
            fails.append(msg)

    ck(classify(installed={"claude", "agy", "codex"}, live=set(REQUIRED_ROLES),
                resource_ok=True) == "complete", "complete 오판")
    ck(state_kind(classify(installed={"claude"}, live={"master", "cso"},
                           resource_ok=True)) == "partial", "partial 오판")
    ck(state_kind(classify(installed=set(), live=set(),
                           resource_ok=True)) == "pending-cli", "pending-cli 오판")
    ck(classify(installed={"claude", "agy", "codex"}, live={"master"},
                resource_ok=False) == "pending-resource", "pending-resource 오판")
    # 부분 설치를 complete 로 오판 금지(Sim S2-5)
    ck(classify(installed={"claude"}, live=set(REQUIRED_ROLES),
                resource_ok=True) != "complete", "부분 CLI 를 complete 로 오판")
    # 락 키 유일성(부서 basename 동일 → 전체 경로 유일화)
    ck(_sanitize_key("/s/cys-dept-dept-1/cys.sock") !=
       _sanitize_key("/s/cys-dept-dept-2/cys.sock"), "동일 basename 두 부서 키 충돌")
    ck(_sanitize_key("") == _sanitize_key(None) == "base", "미설정=base 키")
    # ★agent 사망 인지(2026-07-27 hotfix): 죽은 agent 좌석은 역할 부재 → 재기동 대상.
    dead = {"surfaces": [
        {"role": "master", "exited": False, "agent_alive": False},
        {"role": "cso", "exited": False, "agent_alive": True},
        {"role": "worker-2", "exited": False, "agent_alive": True},
        {"role": "reviewer-gemini", "exited": False, "agent_alive": None},   # 빈 셸(등록 agent 없음)
        {"role": "reviewer-codex", "exited": True, "agent_alive": True},     # 종료 surface
    ]}
    live = _roster_from_status(dead)
    ck("master" not in live, "agent 사망(agent_alive=false) 좌석을 생존으로 오판")
    ck("cso" in live and "worker" in live, "생존 agent 역할 누락(변형 role 귀속 포함)")
    ck("reviewer-gemini" in live, "빈 셸(agent_alive=None)은 종전대로 좌석 생존")
    ck("reviewer-codex" not in live, "exited surface 를 생존으로 오판")
    # 좌석 관측(require_live_agent=False)은 구 동작 그대로 — master 좌석 재생성 오폭 차단.
    seats = _roster_from_status(dead, require_live_agent=False)
    ck("master" in seats, "좌석 관측이 죽은 agent 좌석을 놓치면 master 를 중복 생성한다")
    ck(classify(installed={"claude", "agy", "codex"}, live=live,
                resource_ok=True) != "complete", "agent 전멸 부서를 complete 로 오판")
    # TSV 폴백 파서(구 경로)는 동일 역할 귀속을 유지한다.
    ck(_roster_from_list_tsv(
        "surface:1\trole=cso-1\tpid=1\texited=false\tt\t/\n"
        "surface:2\trole=worker\tpid=2\texited=true\tt\t/\n") == {"cso"},
       "TSV 폴백 파서 회귀(exited 무시·변형 role 귀속)")
    # ★외부 역할 제외(2026-08-01) — env 미설정 회귀 + 제외 시 로스터/생성 경로 이탈.
    #   env 를 직접 만지지 않고 external_roles 인자로 검증(테스트 격리 · 프로세스 env 오염 0).
    ck(effective_required_roles(external=()) == REQUIRED_ROLES, "제외 0 이면 로스터 불변")
    ck(effective_required_roles(external={"master"}) ==
       ("cso", "worker", "reviewer-gemini", "reviewer-codex"),
       "master 제외 시 의무 4종만 남아야(순서 보존)")
    # master 없는 로스터 + master 제외 → complete(외부 세션 담당이므로 결원 아님)
    ck(classify(installed={"claude", "agy", "codex"},
                live={"cso", "worker", "reviewer-gemini", "reviewer-codex"},
                resource_ok=True, external_roles={"master"}) == "complete",
       "master 외부 제외인데 complete 로 안 올라감")
    # 같은 입력에 제외가 없으면 complete 가 **아니어야** 한다(제외가 판정을 실제로 바꾼다는 증거)
    ck(classify(installed={"claude", "agy", "codex"},
                live={"cso", "worker", "reviewer-gemini", "reviewer-codex"},
                resource_ok=True, external_roles=()) != "complete",
       "제외 없이 master 결원을 complete 로 오판")
    # 의무 4종은 제외해도 결원이다(master 제외가 4종 계약을 갉아먹지 않음)
    ck(classify(installed={"claude", "agy", "codex"}, live={"cso", "worker"},
                resource_ok=True, external_roles={"master"}) != "complete",
       "master 제외가 의무 4종 결원까지 덮음")
    ck(classify(installed=set(), live=set(), resource_ok=True,
                external_roles={"master"}) == "pending-cli:cso,reviewer-codex,reviewer-gemini,worker",
       "pending-cli 역할 목록에서 외부 역할 미제외")
    # ★kill-switch fail-closed 봉인(2026-08-01 F1) — 판정은 순수 함수 _gate_verdict 로 핀한다
    #   (subprocess·파일 접촉 0 = 라이브 데몬 무접촉·환경 무관 결정론).
    ck(bool(_gate_verdict(0)) is True, "exit 0(running) 인데 편성 진행 불허")
    ck(bool(_gate_verdict(4)) is False, "exit 4(paused) 인데 편성 강행 — kill-switch 무력화")
    for bad in (1, 2, 3, 127):
        ck(bool(_gate_verdict(bad)) is False,
           "exit %d(판정 불능) 을 진행 허용으로 오판 — fail-open 회귀" % bad)
    ck(bool(_gate_verdict(3, bypass=True)) is True,
       "%s=1 명시 우회가 동작하지 않음(자기잠금)" % GATE_BYPASS_ENV)
    # 우회는 '판정 불능' 전용 — 확정 정지(exit 4·PAUSED 파일)를 env 로 풀면 봉인이 아니라 뒷문이다.
    ck(bool(_gate_verdict(4, bypass=True)) is False, "우회 env 가 exit 4(paused) 까지 무력화")
    ck(bool(_gate_verdict(0, paused_file="/x/AUTOPILOT_PAUSED", bypass=True)) is False,
       "PAUSED 파일 적중을 gate-check exit 0·우회 env 가 덮음")
    ck("판정 불능" in _gate_verdict(3).reason and "3" in _gate_verdict(3).reason,
       "보류 사유(exit 코드 포함)가 detail 로 노출되지 않음 — 자기잠금 진단 불가")
    ck(len(paused_paths()) == 2, "PAUSED kill-switch 파일 2경로 규약(§9-4) 이탈")
    # 두 경로 동등성(plan_roster)
    b, mn = plan_roster("button"), plan_roster("manual")
    ck(b.ensure_fn is mn.ensure_fn, "두 경로 ensure 수렴 실패")
    ck(b.roles == mn.roles, "두 경로 로스터 불일치")
    ck(b.directive_hashes == mn.directive_hashes, "두 경로 디렉티브 해시 불일치")
    if fails:
        print("javis_formation self-test FAIL: %s" % fails, file=sys.stderr)
        return 1
    print("javis_formation self-test OK (classify 5상태·락 키 유일성·plan_roster 동등성"
          "·external-roles 제외·gate fail-closed)")
    return 0


def _cmd_ensure(argv):
    socket, cwd, as_json, force_surface = None, None, False, False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--socket" and i + 1 < len(argv):
            socket = argv[i + 1]; i += 2; continue
        if a == "--cwd" and i + 1 < len(argv):
            cwd = argv[i + 1]; i += 2; continue
        if a == "--json":
            as_json = True; i += 1; continue
        # ★앱 부트 레인 전용(2026-07-26): UI 배너 억제 상태(lastFormationKind)가 앱 재시작으로
        #   소실돼도 소멸 신호가 도달하도록 현재 상태를 1회 강제 표면화. 주기 잡은 붙이지 않는다.
        if a == "--force-surface":
            force_surface = True; i += 1; continue
        i += 1
    try:
        state, detail = ensure(socket=socket, cwd=cwd, force_surface=force_surface)
    except Exception as e:  # 최후 graceful — 예외 스택 대신 명시 메시지
        state, detail = "failed:%s" % type(e).__name__, str(e)
    summary = {"ok": state_kind(state) in ("complete", "partial"),
               "state": state, "kind": state_kind(state), "detail": detail,
               "socket": socket or ""}
    print(json.dumps(summary, ensure_ascii=False))
    # complete=0 / partial·pending=0(부트 무해·비블록) / failed=1(graceful 비0)
    return 1 if state_kind(state) == "failed" else 0


def _cmd_classify(argv):
    installed, live, resource_ok = set(), set(), True
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--installed" and i + 1 < len(argv):
            installed = set(x for x in argv[i + 1].split(",") if x); i += 2; continue
        if a == "--live" and i + 1 < len(argv):
            live = set(x for x in argv[i + 1].split(",") if x); i += 2; continue
        if a == "--no-resource":
            resource_ok = False; i += 1; continue
        i += 1
    print(classify(installed=installed, live=live, resource_ok=resource_ok))
    return 0


def main(argv):
    cmd = argv[1] if len(argv) > 1 else "ensure"
    if cmd in ("self-test", "--self-test"):
        return self_test()
    if cmd == "classify":
        return _cmd_classify(argv[2:])
    if cmd == "ensure":
        return _cmd_ensure(argv[2:])
    sys.stderr.write("usage: javis_formation.py ensure --socket <S> [--cwd D] [--json] "
                     "[--force-surface] | "
                     "classify --installed a,b --live x,y [--no-resource] | self-test\n"
                     "env: CYS_FORMATION_EXTERNAL_ROLES=<role,role> "
                     "(외부 세션 담당 역할 — 로스터·좌석 생성에서 제외 · 기본 빈값=제외 0)\n"
                     "     CYS_FORMATION_IGNORE_GATE=1 "
                     "(gate-check **판정 불능**일 때만 fail-closed 해제 · 기본 미설정=보류 · "
                     "paused(exit 4)·PAUSED 파일은 우회 불가)\n")
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
