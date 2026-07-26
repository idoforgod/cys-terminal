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

ensure(socket): ①cys gate-check(paused→pending 유지 종료) ②소켓키 싱글플라이트 락
  ③probe_cli(로그인셸 우산) 역할별 CLI 판정 ③′라이브 로스터 관측·분류 — **이미 complete 면 자원
    게이트를 보지 않고 즉시 complete 기록·표면화**(신규 스폰 0 = 자원 소비 0. 구 순서는 complete 레인을
    pending-resource 로 조기 반환해 배너 소멸 신호를 영구 차단했다)
  ④complete 가 아닐 때만 javis_resource_gate.py check(hard→pending-resource)
  ⑤master 자리: 죽었으면 new-surface --role master 재생성 / 빈 셸이면 javis_boot_node 입양경로로
    claude 부착 / 이미 각성이면 no-op ⑥나머지 역할 javis_boot_node 재사용(CSO 먼저) ⑦상태 파일
    갱신 + feed 표면화(침묵 금지) ⑧멱등(이미 기동된 역할 재기동 금지 — boot_node already_up).
  모든 실패는 graceful(비0 exit 이되 예외 스택 아닌 명시 메시지).

저장: <state>/formation/<socket-key>.json  (state = CYS_STATE_DIR or ~/.cys/state — 기존 관례).

CLI:
  python3 javis_formation.py ensure --socket <S> [--cwd D] [--json]
  python3 javis_formation.py classify --installed claude,agy --live master,cso [--no-resource]
  python3 javis_formation.py self-test
"""
import json
import os
import subprocess
import sys
import time

SELF_DIR = os.path.dirname(os.path.abspath(__file__))
PACK_DIR = os.environ.get("CYS_PACK_DIR") or os.path.join(
    os.path.expanduser("~"), ".cys", "pack")

# ── 상태 enum(test_formation #1) ──
STATES = ["complete", "partial", "pending-cli", "pending-resource", "failed"]

# 의무 편성 로스터 + 역할→CLI 바이너리 매핑(probe_cli 대상 = claude/agy/codex).
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


# ── ① kill-switch(gate-check) ──
def gate_check():
    """cys gate-check → 편성 진행 허용 여부. paused(exit 4)=False(pending 유지). 데몬 부재 등
    기타 실패는 True(보수적 진행 — 게이트 부재가 편성을 영구 봉쇄하지 않음)."""
    code, _out, _err = _run(["cys", "gate-check"], timeout=10)
    if code == 4:
        return False
    return True


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
def classify(installed=None, live=None, resource_ok=True):
    """설치 CLI 집합 · 라이브 역할 집합 · 자원 여유 → 상태 문자열.
      resource_ok False               → pending-resource
      installed 공집합(CLI 전무)        → pending-cli:{roles}
      의무 5역할 전부 live ∧ 필수 CLI 전부 → complete
      그 외(일부 설치·부팅 중)          → partial:{missing_clis|booting}
    complete 는 master+4종 전부일 때만(Sim S2-5)."""
    installed = set(installed or [])
    live = set(live or [])
    if not resource_ok:
        return "pending-resource"
    missing_clis = sorted(REQUIRED_CLIS - installed)
    if not installed:
        return "pending-cli:" + ",".join(sorted(REQUIRED_ROLES))
    if set(REQUIRED_ROLES).issubset(live) and not missing_clis:
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


# ── 라이브 로스터 관측(cys list) ──
def _live_roles(socket):
    """cys list(소켓 문맥) → 라이브(미exited) 역할 집합. 파싱 불가/데몬 부재 → None."""
    env = dict(os.environ)
    if socket:
        env["CYS_SOCKET"] = socket
    try:
        r = subprocess.run(["cys", "list"], capture_output=True, text=True,
                           timeout=15, env=env)
    except Exception:
        return None
    if r.returncode != 0:
        return None
    roles = set()
    for line in (r.stdout or "").splitlines():
        f = line.rstrip("\n").split("\t")
        if len(f) < 4:
            continue
        role = f[1][5:] if f[1].startswith("role=") else ""
        if f[3].strip().endswith("true"):  # exited 무시
            continue
        if not role:
            continue
        # 변형 역할(cso-1·worker-2·reviewer-*)은 의무 역할 기준으로 귀속.
        if role == "master":
            roles.add("master")
        elif role == "cso" or role.startswith("cso-"):
            roles.add("cso")
        elif role == "worker" or role.startswith("worker-"):
            roles.add("worker")
        elif role.startswith("reviewer-gemini"):
            roles.add("reviewer-gemini")
        elif role.startswith("reviewer-codex"):
            roles.add("reviewer-codex")
        else:
            roles.add(role)
    return roles


def _installed_clis():
    """probe_cli(로그인셸 우산) 로 claude/agy/codex 설치 여부 판정 → 설치된 CLI 집합."""
    try:
        import javis_cli_probe
    except ImportError:
        sys.path.insert(0, SELF_DIR)
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
    roles = _live_roles(socket)
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


def _surface(socket, prev_state, state):
    """상태 표면화 — (1) UI 배너 수명 신호(feed --kind) (2) 음성/HUD EVT. kind 전이 또는 최초
    관측 시에만 발화(심박 스팸 억제·'전이 시' 계약). 전부 best-effort·부트 무해(R10)."""
    if prev_state is not None and state_kind(prev_state) == state_kind(state):
        return  # 상태 kind 무변화 → 재표면화 생략(배너 dismiss 는 멱등이나 스팸 억제)
    _feed_for_state(state)
    evt = evt_type_for_state(state)
    if evt:
        _emit_evt(evt, {"socket": socket or "", "state": state, "kind": state_kind(state)})


# ── ensure: 상태 기계 전이 주체 ──
def ensure(socket=None, cwd=None):
    """편성 전이(멱등·싱글플라이트). 반환: (state, detail). 모든 실패 graceful."""
    prev = _read_state(socket)   # 전이 감지용(배너 수명 신호는 전이 시에만 표면화)
    # ① kill-switch
    if not gate_check():
        # paused: 현 관측 기준 상태를 계산해 기록만(기동 없이 pending 유지).
        installed = _installed_clis()
        live = _live_roles(socket) or set()
        state = classify(installed=installed, live=live, resource_ok=True)
        if state == "complete":
            state = "partial:paused"  # paused 중엔 complete 로 승격하지 않음
        detail = "kill-switch(paused) — 편성 보류(gate-check 존중)"
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
            detail = "라이브 로스터 이미 완결(5역할 생존) — 신규 기동 0 · 자원 게이트 무관"
            _write_state(socket, state, detail, live_now)
            _surface(socket, prev, state)
            return state, detail

        # ④ 자원 게이트(complete 가 **아닐 때만** — 실제로 노드를 스폰하는 경로에서만 예산을 본다)
        if not _resource_ok(socket):
            live = live_now or set()   # ③′ 관측 재사용(cys list 중복 호출 0)
            state = "pending-resource"
            detail = "곱셈 자원 예산 hard — 편성 대기"
            _write_state(socket, state, detail, live)
            _surface(socket, prev, state)
            return state, detail

        # CLI 전무 → pending-cli(빈 셸 유지·온보딩 보존)
        if not installed:
            live = live_now or set()   # ③′ 관측 재사용(cys list 중복 호출 0)
            state = classify(installed=installed, live=live, resource_ok=True)
            detail = "CLI 미설치 — 빈 셸 유지·편성 대기(설치 시 자동 완결)"
            _write_state(socket, state, detail, live)
            _surface(socket, prev, state)
            return state, detail

        # ⑤⑥ 설치된 CLI 기준 최대 편성. master 먼저(입양 경로) → CSO → 나머지.
        booted = set()
        order = ["master", "cso", "worker", "reviewer-gemini", "reviewer-codex"]
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
            ",".join(sorted(booted)) or "없음", ",".join(sorted(installed)))
        _write_state(socket, state, detail, live)
        # ★주기 실행 스팸 차단(2026-07-26): 무조건 _feed_for_state 였던 자리를 _surface(전이 시에만
        # 표면화) 로 교체한다. 편성 ensure 가 스케줄 주기(10분)로 붙으면 매 틱마다 사용자에게 토스트가
        # 갔다. prev 는 ensure 진입부에서 읽은 직전 상태 — 동일 kind 면 _surface 계약대로 생략된다.
        _surface(socket, prev, state)
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
    # 두 경로 동등성(plan_roster)
    b, mn = plan_roster("button"), plan_roster("manual")
    ck(b.ensure_fn is mn.ensure_fn, "두 경로 ensure 수렴 실패")
    ck(b.roles == mn.roles, "두 경로 로스터 불일치")
    ck(b.directive_hashes == mn.directive_hashes, "두 경로 디렉티브 해시 불일치")
    if fails:
        print("javis_formation self-test FAIL: %s" % fails, file=sys.stderr)
        return 1
    print("javis_formation self-test OK (classify 5상태·락 키 유일성·plan_roster 동등성)")
    return 0


def _cmd_ensure(argv):
    socket, cwd, as_json = None, None, False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--socket" and i + 1 < len(argv):
            socket = argv[i + 1]; i += 2; continue
        if a == "--cwd" and i + 1 < len(argv):
            cwd = argv[i + 1]; i += 2; continue
        if a == "--json":
            as_json = True; i += 1; continue
        i += 1
    try:
        state, detail = ensure(socket=socket, cwd=cwd)
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
    sys.stderr.write("usage: javis_formation.py ensure --socket <S> [--cwd D] [--json] | "
                     "classify --installed a,b --live x,y [--no-resource] | self-test\n")
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
