#!/usr/bin/env python3
"""javis_hud_bridge.py — 메타버스 오피스 수집·정규화 브리지 (DESIGN.md §3)

cys 데몬의 이벤트 스트림(`cys events`)과 스냅샷(`cys fleet/status --json`)을
WorldState(§4)로 정규화해 127.0.0.1 전용 HTTP(+SSE)로 내보낸다.

원칙:
  · 파이썬 stdlib만 사용 (외부 의존 0)
  · 읽기 전용 — 쓰기 API 없음, cys에 대한 호출도 관측 명령만 (R4)
  · 소음 필터·코얼레싱 (§6.3) — watchdog 무시, fx 초당 상한, 알림류 중복 병합
  · 이벤트 유실 방어 — --cursor-file 로 시퀀스 영속, 재시작 시 gap 없이 재개

기동:  cys run -- python3 bin/javis_hud_bridge.py
접속:  http://127.0.0.1:8765
"""
import glob
import json
import os
import re
import secrets
import subprocess
import sys
import threading
import time
from collections import defaultdict, deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Empty, Full, Queue

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WEB_DIR = os.path.join(ROOT, "web")
STATE_DIR = os.environ.get("HUD_STATE_DIR") or os.path.join(ROOT, "state")  # 팩 편입 시 외부 지정
BIND = "127.0.0.1"
PORT = int(os.environ.get("HUD_PORT", "8642"))  # 8765는 이 장비에서 선점 실측 → 회피
FLEET_POLL_SECS = float(os.environ.get("HUD_POLL", "2.0"))
CYS = os.environ.get("HUD_CYS_BIN", "cys")
# P2-1 부서 이벤트 멀티 구독 슈퍼바이저
SUB_CAP = int(os.environ.get("HUD_SUB_CAP", "12"))     # 동시 구독 상한(런어웨이 방지)
SUB_BACKOFF_SECS = 2.0                                   # 구독 재수립 백오프
SUB_RECONCILE_SECS = 2.0                                 # 타깃 reconcile 주기
# Windows: 이 브리지는 콘솔 없는 cysd가 NO_WINDOW로 띄운다 — 콘솔 자식(cys.exe)을 그냥
# 스폰하면 새 콘솔 창이 할당된다(상주 events 자식 = AppData 경로 제목의 검은 WT 탭 실사고
# 2026-07-11). 이 파일의 모든 subprocess 호출에 **NOWIN 을 전개해 숨긴다. 타 OS 무동작.
NOWIN = {"creationflags": 0x08000000} if os.name == "nt" else {}
BACKLOG_FX_SECS = 90.0   # 이보다 오래된 이벤트는 상태만 반영, fx(연출) 억제 — 콜드스타트 폭주 방지
CMD_MAX_LEN = 2000       # 조작 지시 텍스트 상한

# 오피스 디테일 v1.1 (§DESIGN-office-detail-v11) — EVT spool·칸반·verdict·전광판 데이터
JAVIS_ROOT = os.environ.get("JAVIS_ROOT") or os.path.expanduser("~/Desktop/CYSjavis")
TASKS_DIR = os.path.join(JAVIS_ROOT, "_round", "tasks")
ROUND_DIR = os.path.join(JAVIS_ROOT, "_round")
TRANSCRIPTS_DB = os.path.expanduser("~/.cys/transcripts.db")
EVT_SPOOL_PATH = os.path.join(STATE_DIR, "evt_spool.jsonl")
EVT_OFFSET_PATH = os.path.join(STATE_DIR, "evt_spool.offset")
PRESENCE_HEAT_PATH = os.path.join(STATE_DIR, "presence_heat.json")
SPOOL_POLL_SECS = 1.0
SPOOL_ROTATE_BYTES = 4 * 1024 * 1024   # spool 무제한 성장 봉인 — 드레인 상태에서만 로테이션
KANBAN_POLL_SECS = 5.0
VERDICT_POLL_SECS = 5.0
BOARD_PERSIST_SECS = 60.0   # 히트 영속·비용 캐시 갱신 주기
REVIEW_KEEP = 10            # world.review.items 최신 유지 건수

# ---------------------------------------------------------------- 판정 (§5)
HOOK_ACTIVE_WINDOW = 30.0      # 최근 30s 내 도구 훅 → active
SELF_REPORT_FRESH = 300.0      # 자기보고 신선 기준
IDLE_WAITING = 300
IDLE_DROWSY = 3600
STATE_MAP = {"working": "active", "waiting": "waiting", "quiescing": "quiescing"}


def judge_presence(node, now, last_hook_ts):
    """(presence, confidence) 판정 — DESIGN.md §5 우선순위 표 그대로.

    node: fleet surface dict. last_hook_ts: 해당 surface의 마지막 hook 시각(없으면 None).
    """
    if node.get("exited") or node.get("agent_alive") is False:
        return "dead", 1.0
    if last_hook_ts is not None and (now - last_hook_ts) <= HOOK_ACTIVE_WINDOW:
        return "active", 0.95
    st = node.get("status") or {}
    state = st.get("state")
    age = st.get("age_secs")
    if state in STATE_MAP and age is not None and age < SELF_REPORT_FRESH:
        return STATE_MAP[state], 0.9
    idle = node.get("idle_secs")
    stale_penalty = 0.15 if (state and age is not None and age >= SELF_REPORT_FRESH) else 0.0
    if idle is None:
        return "unknown", 0.3
    if idle < IDLE_WAITING:
        return "waiting", 0.7 - stale_penalty
    if idle < IDLE_DROWSY:
        return "drowsy", 0.7 - stale_penalty
    return "sleeping", 0.7 - stale_penalty


def compute_activity(hook_count_60s, lines_per_sec):
    """활동 강도 0..1 (§7-3): hook 빈도와 출력량 중 큰 쪽. 상한 정규화."""
    h = min(hook_count_60s / 20.0, 1.0)
    l = min(max(lines_per_sec, 0.0) / 15.0, 1.0)
    return round(max(h, l), 3)


def pick_ctx(node):
    """ctx% 선택: 실측(usage.ctx_pct·statusline) > 자기보고(status.context_pct)."""
    u = node.get("usage") or {}
    if isinstance(u.get("ctx_pct"), (int, float)):
        return u["ctx_pct"]
    st = node.get("status") or {}
    if isinstance(st.get("context_pct"), (int, float)):
        return st["context_pct"]
    return None


# ------------------------------------------------------------ 코얼레싱 (§6.3)
# watchdog.* 는 route_event 가 startswith 로 선차단·dog_fx 가 kill/alert 변환 — NOISE_NAMES 는
# 그 외 소음용(현재 비어 있음). "watchdog.proc_count" 는 실이벤트명이 proc_count_high 라 도달불가
# 死엔트리였으므로 제거(reviewer1 minor).
NOISE_NAMES = set()
ALERT_COALESCE = {  # (이벤트명 → 동일 surface 재발화 억제 윈도 s)
    "health.alert": 30.0,
    "master.deadman": 60.0,
    "master.idle": 120.0,     # G2 W3-B: 정보성 침묵 신호 — pane.idle 과 동률(120s) 코얼레스
    "pane.idle": 120.0,
    "schedule.fired": 10.0,
    "schedule.error": 30.0,
    "watchdog.dog.kill": 10.0,    # D4 강아지 fx kill ≤1건/10s (kind별 분리 — kill 이 alert 에 안 밀림)
    "watchdog.dog.alert": 10.0,   # D4 강아지 fx alert ≤1건/10s
}
FX_BUDGET_PER_SEC = 20


class Coalescer:
    """알림류 (name,surface) 윈도 병합 + fx 전역 초당 예산. 상태(patch)는 여기 안 거침."""

    def __init__(self, budget=FX_BUDGET_PER_SEC):
        self.last_seen = {}
        self.budget = budget
        self.bucket = budget
        self.bucket_ts = None  # 첫 allow의 now 기준으로 지연 초기화 (시간축 혼선 방지)
        self._lock = threading.Lock()   # events_loop·spool_loop 공유 → allow() 스레드 안전

    def allow(self, name, surface_id, now=None):
        now = time.monotonic() if now is None else now
        with self._lock:
            if self.bucket_ts is None:
                self.bucket_ts = now
            # 전역 예산 (초당 리필 · 음수 경과 가드)
            self.bucket = min(self.budget,
                              self.bucket + max(0.0, now - self.bucket_ts) * self.budget)
            self.bucket_ts = now
            if self.bucket < 1:
                return False
            win = ALERT_COALESCE.get(name)
            if win is not None:
                key = (name, surface_id)
                last = self.last_seen.get(key)
                if last is not None and (now - last) < win:
                    return False
                self.last_seen[key] = now
            self.bucket -= 1
            return True


# ------------------------------------------------------- 조작 게이트 (T3 · §R4 해제)
# 위협모델: 실경계 = 브라우저 안 원격 웹페이지의 localhost CSRF (로컬 프로세스는
#   이미 cys를 직접 호출 가능하므로 경계 밖 — 이 게이트의 목적이 아니다).
# 방어: 기동 시 랜덤 토큰(HTML 주입) + 커스텀 헤더 X-HUD-Token(=preflight 강제,
#   OPTIONS는 무조건 deny) + Origin allowlist + 대상 surface allowlist(실존만)
#   + 제어문자·개행 제거 + 길이 상한 + fail-closed + 감사 로그.
# 근본한계(명문화 · 원칙5):
#   ① 대상 surface가 shell 프롬프트 상태면 Return 주입 = 즉시 실행이다. 이는 기능
#      그 자체이며 신뢰 주체(오너)의 실수 방지를 위해 개행 제거·1줄 강제만 한다.
#   ② GET / 를 읽을 수 있는 로컬 주체는 토큰을 얻는다 — localhost 바인딩이 경계.
# v2 부서 한정 키(DESIGN-dept-qualified-keys-v2 §3·C2): 정식 노드 키는 <slug>@surface:N.
# 구분자 @ (v2.0의 # 교체 — # 은 URL fragment 라 /peek GET 에서 서버에 키가 잘려 도달).
CMD_KEY_RE = re.compile(r"^[a-z0-9_-]{1,32}@surface:\d{1,8}$")
CTRL_CHARS = re.compile(r"[\x00-\x1f\x7f]")


def allowed_origins(port):
    return {f"http://127.0.0.1:{port}", f"http://localhost:{port}",
            "tauri://localhost", "http://tauri.localhost"}


def cors_header_for(method, origin, origins):
    """GET 읽기 응답에 붙일 CORS 허용 origin 판정 (순수 함수 — 자기공격 테스트 대상).

    CC(tauri webview·교차출처)의 /world 프로브 fetch가 응답을 읽을 수 있게
    allowlist(조작 게이트와 동일 SOT=allowed_origins) 정확 일치 + GET 한정으로만
    Access-Control-Allow-Origin을 에코한다. 그 외(POST·미지 origin·"null"·부재)는
    전부 None(무헤더 = deny-by-default) — 원격 웹페이지의 localhost 읽기 차단 불변.
    POST /command 방어(커스텀 헤더=preflight 강제·OPTIONS 무조건 deny)도 불변.
    """
    if method == "GET" and origin is not None and origin in origins:
        return origin
    return None


def gate_command(body, token_header, expected_token, origin, origins, known_keys):
    """조작 요청 판정 (순수 함수 — 자기공격 테스트 대상).

    반환 (ok, err, cleaned): ok=False면 err=거부 사유. deny-by-default —
    모든 검사를 통과한 요청만 allow, 판정 불능은 전부 거부(fail-closed).
    """
    if not expected_token or not token_header or \
       not secrets.compare_digest(str(token_header), str(expected_token)):
        return False, "bad_token", None
    if origin is not None and origin not in origins:
        return False, "bad_origin", None            # 교차출처 브라우저 확정 신호
    if not isinstance(body, dict):
        return False, "bad_body", None
    key = body.get("key")
    if not isinstance(key, str) or not CMD_KEY_RE.match(key):
        return False, "bad_key_format", None
    if key not in known_keys:
        return False, "unknown_target", None        # 실존 fleet allowlist 밖 → deny
    text = body.get("text")
    if not isinstance(text, str):
        return False, "bad_text", None
    text = CTRL_CHARS.sub(" ", text).strip()        # 제어문자·개행 → 공백 (1줄 강제)
    if not text or len(text) > CMD_MAX_LEN:
        return False, "bad_text_len", None
    return True, None, {"key": key, "text": text}


# ---------------------------------------------- v2 부서 한정 키 (§3 · §4b)
SLUG_RE = re.compile(r"^[a-z0-9_-]{1,32}$")


def normalize_slug(raw):
    """레지스트리 키 → 계약 문자셋(^[a-z0-9_-]{1,32}$) 정규화. fail-open 금지.

    소문자화 → 허용 외 문자 '-' 치환 → 32자 절단. 충돌 해소(-N 서픽스)는 호출자(부서 루프) 몫.
    빈 결과는 'dept' 로 대체(정식 키 생성 보장).
    """
    s = re.sub(r"[^a-z0-9_-]", "-", (raw or "").strip().lower())[:32]
    return s or "dept"


def full_key(slug, surface_ref):
    """정식 노드 키 <slug>@surface:N. surface_ref 부재 시 None (귀속 불능 노드)."""
    if not surface_ref:
        return None
    return "%s@%s" % (slug, surface_ref)


def node_key(s):
    """노드 정식 키 — merge_fleet 가 주석한 _full_key 우선.

    미주석(부트 전·구 테스트 fixture) 시 bare surface_ref 폴백 — 하위호환 경로(계약 §5).
    실운영에선 merge_fleet 가 매 스냅샷에서 _full_key 를 부여하므로 항상 정식 키가 반환된다.
    """
    return s.get("_full_key") or s.get("surface_ref")


# ------------------------------------------------------------------ 월드
# ★W14 S16 — 파일명 정규식이 **숫자·하이픈을 통째로 탈락**시켰다. 종전 `[A-Z_]+`는
# `WORKER_2_TODO.md`·`REVIEWER-GEMINI_TODO.md`를 아예 매치하지 못해 HUD에서 **표시 자체가
# 사라졌다**. 둘 다 실재 형태다 — `cys todo-path`가 role `worker-2`에 대해 실제로 만드는
# 이름이고, 하이픈판은 손기동 산출물로 실측된다. 문자집합을 role 라벨공간에 맞춘다.
TODO_ROLE = re.compile(r"(?:^|/)(?P<r>[A-Za-z0-9_-]+)_TODO\.md$")
# 파일명 → role 라벨 정규화는 **소문자화 + 언더스코어→하이픈** 한 규칙이다
# (`javis_report.node_label`·`javis_todo_stamp.owner_from_filename`과 **같은 규칙**).
# 종전의 하드코딩 dict는 이 규칙과 결과가 동일하면서 새 역할이 생길 때마다 조용히
# 누락되는 표면만 늘렸다 — 규칙 하나로 수렴시킨다.
TODO_ROLE_MAP = {"MASTER": "master", "WORKER": "worker", "CSO": "cso",
                 "REVIEWER_GEMINI": "reviewer-gemini", "REVIEWER_AGY": "reviewer-agy",
                 "REVIEWER_CODEX": "reviewer-codex"}


def normalize_role_label(raw):
    """role 라벨공간(소문자·하이픈)으로 수렴시킨다. 빈 값이면 None.

    ⚠라벨은 게이트·HUD의 **조인 키**다. 한 글자만 어긋나도 조인이 조용히 전패한다
    (`javis_report.decl_label`이 같은 이유로 같은 정규화를 한다).
    """
    s = (raw or "").strip().lower().replace("_", "-")
    return s or None


def todo_label(path, entry):
    """이 todo 항목의 라벨 — **선언 `owner`가 1순위**, 없으면 파일명 폴백(D3 해소).

    ★W14 S16 — 데몬은 `todo.updated`에 owner를 실으면서 `org.status`에는 싣지 않았고,
    브리지는 **어느 쪽도 읽지 않았다**. 그래서 선언이 소거하려던 파일명→역할 추론(D3)이
    HUD(C4)에 그대로 생존했다 — 실측: 선언 `owner=cso`인데 HUD 라벨은 `worker`.
    선언이 유일한 진실이라는 ADR-1이 마지막 소비자에서 지켜지지 않았다.

    ⚠스큐 안전(ADR-2): 구버전 데몬은 `owner`를 싣지 않는다 → 파일명 폴백 = 종전 동작 그대로.
    """
    owner = normalize_role_label((entry or {}).get("owner"))
    if owner:
        return owner
    m = TODO_ROLE.search(path or "")
    if not m:
        return None
    raw = m.group("r")
    return normalize_role_label(TODO_ROLE_MAP.get(raw, raw))

# C4: 데몬(C2)이 실은 선언 판정 → HUD 구분 표시 라벨.
# 집계에 남아 있는 비-counted 판정은 이 둘뿐이다 — retired·foreign-scope 는 데몬이 애초에
# 등재하지 않는다(설계 §4-5). `counted` 와 미지 판정은 라벨을 달지 않는다.
TODO_FLAG_BY_VERDICT = {
    "unclaimed": "미선언",      # 선언 없음·깨짐 — 주인을 기계가 확정하지 못한 파일
    "orphan-scope": "고아",     # 실재하지 않는 팩을 가리킴(부서 teardown·개명 흔적)
}


def todo_flag(entry):
    """todo 항목(또는 todo.updated payload)의 판정 라벨. 표시할 것이 없으면 None.

    ⚠**스큐 안전(ADR-2)**: 팩은 pack 채널로, 데몬은 앱 릴리스로 배송돼 **스큐가 정상 상태**다.
    구버전 데몬은 `verdict` 를 싣지 않으므로 `.get()` 기본값으로 None → 표시·프레임이 종전과
    완전히 동일하다. 양측은 서로를 전제하지 않는다.

    ⚠**온보딩 방어(설계 §6)**: 미선언은 **경고가 아니라 정보**다. 신규 사용자가 손으로 쓴 todo 에
    경고가 쏟아지면 첫 경험이 훼손된다 — 라벨만 달고 진행률(done/total)은 그대로 보여준다.
    """
    return TODO_FLAG_BY_VERDICT.get((entry or {}).get("verdict"))


def with_todo_flag(entry):
    """항목에 표시 라벨(`flag`)을 얹은 사본. 달 라벨이 없으면 **원본 그대로** 돌려준다
    (구버전 데몬·정상 파일에서 프레임이 한 바이트도 달라지지 않게 하는 스큐 안전 경로)."""
    flag = todo_flag(entry)
    if flag is None:
        return entry
    out = dict(entry)
    out["flag"] = flag
    return out


class World:
    """WorldState 보유·병합·직렬화 (계약 v1 · §4)."""

    def __init__(self):
        self.lock = threading.Lock()
        self.departments = []          # fleet 원본 (surfaces 포함)
        self.daemon = {}
        self.todo = {}                 # path → {done,total,age_secs}
        self.seq = 0
        # P2-3(잠복 결함 2호): 4맵을 sid 단독 키 → 정식 노드 키(<slug>@surface:N)로 전환.
        # sid 는 부서 데몬별이라 비유일 — sid 키 유지 시 동번호 부서가 hook·activity·flag 상호 오염.
        self.hooks = defaultdict(deque)      # node_key → 최근 hook ts deque
        self.line_hist = {}            # node_key → (line_count, ts)
        self.line_rate = {}            # node_key → lines/s
        self.flags = defaultdict(dict)  # node_key → {flag: expiry_ts}
        self.server_room = {}          # pid → {cmd, scoped}
        self.prev_nodes = {}           # key → 직전 노드 스냅샷 (diff용)
        # v2 부서 한정 키(§4b) — 소켓 캐리·지연 히트 마이그레이션
        self.sockets = {}              # full_key → socket 경로|None(본부) (M1 스냅샷 캐리)
        self.pending_heat = {}         # 부팅 시 보존한 bare 키 히트 (첫 merge 후 승격·M2)
        self.heat_migrated = False     # bare→정식 히트 승격 1회 게이트
        self._dept_fallback_warned = False  # dept 필드 부재 폴백 1회 경고
        # 오피스 디테일 v1.1 — 상단 필드·노드 확장 필드
        self.progress = {}             # node key → {task,stage,pct,detail,ts}  (기능 6)
        self.run = {}                  # node key → {queued,active,done_today,failed_today} (기능 7)
        self.run_date = None           # done/failed_today 로컬 날짜 롤오버 기준
        self.blocked = []              # [{task,blocked_by,key,ts}] (기능 8)
        self.kanban = {"ts": 0, "tasks": []}    # (기능 9)
        self.review = {"ts": 0, "items": []}    # (기능 10)
        self.heat_acc = {}             # node key → {"active":[24],"total":[24]} (기능 12)
        self.heat_hour = None          # 링 버킷 롤오버 기준 시각(hour)
        self.cost_cache = {"ts": 0.0, "value": {"usd": None, "tokens": None}}
        self.prev_top = {}             # patch_top 직전값 비교 (field → value)

    # --- 수집 반영 ---
    def note_hook(self, key, ts):
        dq = self.hooks[key]
        dq.append(ts)
        while dq and ts - dq[0] > 60.0:
            dq.popleft()

    def set_flag(self, key, flag, ttl):
        self.flags[key][flag] = time.time() + ttl

    def live_flags(self, key, now):
        f = self.flags.get(key) or {}
        return sorted(k for k, exp in f.items() if exp > now)

    def apply_usage(self, surface_id, payload, slug="main"):
        """usage.updated → 해당 노드 usage 즉시 갱신. 성공 시 노드 정식 key 반환.

        P2-2: 이벤트가 도착한 구독의 slug 스코프로 조회한다(C3 본부 한정을 일반화 — 각 부서
        구독은 자기 데몬 surface_id 만 발급하므로 slug 로 스코프해 동번호 부서 오귀속·순서 의존 소거).
        기본값 "main" 은 단일 구독(P1) 하위호환.
        """
        with self.lock:
            for d in self.departments:
                if d.get("_slug") != slug:
                    continue
                for s in d.get("surfaces", []):
                    if s.get("surface_id") == surface_id:
                        s["usage"] = payload
                        return node_key(s)
        return None

    def socket_for(self, full_key):
        """정식 키 → (found, socket|None). 스냅샷(M1)에 없으면 (False, None) = fail-closed.

        found=True·socket=None 은 본부 노드(기본 소켓 사용) — /command·/peek 가 --socket 을 생략한다.
        found=False 는 미지 키 → 조작 거부.
        """
        with self.lock:
            if full_key in self.sockets:
                return True, self.sockets[full_key]
        return False, None

    def has_full_key(self, full_key):
        """현재 fleet 스냅샷에 정식 키가 실존하는지 (귀속 사다리 ① 존재 검증)."""
        with self.lock:
            return any(node_key(s) == full_key for d in self.departments
                       for s in d.get("surfaces", []))

    def unique_full_key_for_ref(self, ref):
        """bare surface_ref 가 fleet 에서 정확히 1개 surface 에만 있으면 그 정식 키 (귀속 사다리 ②)."""
        if not ref:
            return None
        with self.lock:
            hits = [node_key(s) for d in self.departments for s in d.get("surfaces", [])
                    if s.get("surface_ref") == ref]
        return hits[0] if len(hits) == 1 else None

    def dept_targets(self):
        """현재 fleet 의 구독 타깃 {slug: socket|None} (P2-1 슈퍼바이저 reconcile 입력).

        각 부서의 socket 은 merge_fleet 가 주석한 dept["socket"](본부=None). 첫 등장 slug 만 채택.
        """
        with self.lock:
            out = {}
            for d in self.departments:
                slug = d.get("_slug")
                if slug and slug not in out:
                    out[slug] = d.get("socket")
            return out

    @staticmethod
    def _todo_rank(path, entry):
        """정본 선출 정렬 키(작을수록 우선). 상세 근거는 `snapshot`의 주석."""
        e = entry or {}
        done, total = e.get("done") or 0, e.get("total") or 0
        pending = total > 0 and done < total
        return (0 if e.get("owner") else 1,      # ① 선언 owner 보유
                0 if pending else 1,             # ② 미완 우선(완료가 미완을 덮지 않는다)
                e.get("age_secs") or 0,          # ③ 최신
                path or "")                      # ④ 결정론 tie-break

    def apply_todo(self, path, done, total, verdict=None, owner=None):
        with self.lock:
            ent = self.todo.setdefault(path, {})
            ent["done"], ent["total"], ent["age_secs"] = done, total, 0
            # owner 는 신버전 데몬만 싣는 선택 필드다(ADR-2 스큐 안전 — verdict 와 같은 규칙).
            # 부재면 기존 값을 **지우지 않는다**: 구버전 이벤트 한 건이 스냅샷의 라벨 진실을
            # 파일명 추론으로 되돌리면 안 된다.
            if owner:
                ent["owner"] = owner
            # verdict 는 신버전 데몬만 싣는 선택 필드다(ADR-2 스큐 안전). 부재면 기존 항목의
            # 판정을 지우지 않고 그대로 둔다 — 구버전 이벤트가 스냅샷의 라벨을 지워버리면
            # 미선언·고아가 무음으로 되돌아간다.
            if verdict is not None:
                ent["verdict"] = verdict
                flag = todo_flag(ent)
                if flag is None:
                    ent.pop("flag", None)   # counted 로 전이 = 라벨 해제
                else:
                    ent["flag"] = flag

    def apply_ledger(self, name, payload):
        if name == "ledger.registered":
            self.server_room[payload.get("pid")] = {
                "cmd": (payload.get("cmd") or "")[:80], "scoped": payload.get("scoped")}
        elif name == "ledger.killed":
            self.server_room.pop(payload.get("pid"), None)

    def _annotate_depts(self, new_deps):
        """부서별 slug 정규화(충돌 -N 서픽스)·정식 키·dept_label·socket 을 surface 에 주석.

        self.sockets 스냅샷(full_key → socket|None) 재구성(M1). (lock 보유 상태 호출 — 재진입 금지.)
        dept 필드 부재(구 cys 조합) 시 display_name 정규화 폴백 + 1회 경고(계약 §4b·§5).
        """
        used = set()
        sockets = {}
        for d in new_deps:
            raw_dept = d.get("dept")
            if raw_dept is None:   # 구 cys: dept 필드 없음 → display_name 정규화 폴백 + 1회 경고
                raw_dept = d.get("department")
                if not self._dept_fallback_warned:
                    sys.stderr.write(
                        "[hud-bridge] warn: fleet dept 필드 부재 — display_name 정규화 폴백\n")
                    self._dept_fallback_warned = True
            base = normalize_slug(raw_dept)
            slug = base
            n = 2
            while slug in used:   # 충돌 해소: base-2, base-3 … (32자 상한 유지)
                slug = "%s-%d" % (base[:29], n)
                n += 1
            used.add(slug)
            label = d.get("department") or slug
            socket = d.get("socket")   # 문자열 경로 또는 None(본부)
            d["_slug"] = slug
            d["_dept_label"] = label
            for s in d.get("surfaces", []):
                fk = full_key(slug, s.get("surface_ref"))
                s["_full_key"] = fk
                s["_dept_label"] = label
                s["_dept"] = slug
                if fk is not None:
                    sockets[fk] = socket
        self.sockets = sockets

    def _migrate_pending_heat(self):
        """부팅 시 보존한 bare 키 히트를 첫 fleet 기준 정식 키로 1회 승격 (M2 · lock 보유 호출).

        부팅 순서상 load_heat(main 1333) < 첫 merge_fleet(스레드 기동 후) 이라 즉시 승격 불가 —
        유일 매칭(bare surface_ref 가 정확히 1개 surface)만 정식 키로 승격, 실패분은 폐기한다.
        """
        for bare, v in self.pending_heat.items():
            hits = [node_key(s) for d in self.departments for s in d.get("surfaces", [])
                    if s.get("surface_ref") == bare]
            if len(hits) == 1 and hits[0] and hits[0] not in self.heat_acc:
                self.heat_acc[hits[0]] = v
        self.pending_heat = {}
        self.heat_migrated = True

    def merge_fleet(self, fleet, status):
        """스냅샷 병합 → (patch 프레임 목록, 구조변화 여부). 부재 시 이전 유지."""
        with self.lock:
            if status:
                self.daemon = status.get("daemon") or self.daemon
                self.daemon["paused"] = status.get("paused", False)
                for p, t in (status.get("todo") or {}).items():
                    # C4: 데몬이 실은 선언 판정을 표시 라벨로 승격(미선언/고아 구분).
                    # 플래그 부재(구버전 데몬)면 t를 그대로 둔다 — 종전과 동일 동작.
                    self.todo[p] = with_todo_flag(t)
                self.seq = self.daemon.get("latest_seq", self.seq)
            structural = False
            if fleet:
                new_deps = fleet.get("departments") or []
                self._annotate_depts(new_deps)   # 정식 키·socket 주석 (구조 비교 전 선행)
                # 정식 키 기준 구조 비교 — 동번호 부서 surface_ref 충돌로 인한 오탐 제거
                old_keys = {node_key(s) for d in self.departments
                            for s in d.get("surfaces", [])}
                new_keys = {node_key(s) for d in new_deps
                            for s in d.get("surfaces", [])}
                # D5: (slug, label) 튜플 비교 — display_name 개명만 바뀌어도 structural=True
                # → 전체 재빌드 유발(층 라벨 재시작 없이 반영). 개명은 드문 이벤트, 재빌드 비용 수용.
                old_shape = [(d.get("_slug"), d.get("_dept_label")) for d in self.departments]
                new_shape = [(d.get("_slug"), d.get("_dept_label")) for d in new_deps]
                structural = (old_keys != new_keys) or (old_shape != new_shape)
                # 출력량 변화율 (line_count 델타)
                now = time.time()
                for d in new_deps:
                    for s in d.get("surfaces", []):
                        # P2-3: 정식 키로 line 이력 추적 — 동번호 부서 간 activity 오염 제거
                        nk = node_key(s)
                        lc = s.get("line_count") or 0
                        prev = self.line_hist.get(nk)
                        if prev and now > prev[1]:
                            self.line_rate[nk] = max(0.0, (lc - prev[0]) / (now - prev[1]))
                        self.line_hist[nk] = (lc, now)
                self.departments = new_deps
                if not self.heat_migrated:   # 첫 실 fleet 병합 시점에 히트 지연 승격(M2)
                    self._migrate_pending_heat()
            patches = self._diff_patches()
            return patches, structural

    def _node_view(self, s, now):
        """fleet surface → 계약 노드 뷰."""
        sid = s.get("surface_id")
        nkey = node_key(s)   # P2-3: hook·activity·flag 조회를 정식 키로 (sid 비유일 오염 제거)
        dq = self.hooks.get(nkey) or ()
        last_hook = dq[-1] if dq else None
        presence, conf = judge_presence(s, now, last_hook)
        st = s.get("status") or {}
        ctx = pick_ctx(s)
        u = s.get("usage") or {}
        flags = self.live_flags(nkey, now)
        if ctx is not None and ctx >= 90:
            flags = flags + ["ctx_critical"]
        return {
            "key": nkey, "surface_id": sid,
            "dept_label": s.get("_dept_label"),   # 표시 전용(패널 타이틀) — raw 키 노출 제거(§3)
            "role": s.get("role") or "무소속",
            "agent": s.get("agent") or u.get("agent"),
            "presence": presence, "presence_conf": round(conf, 2),
            "task": (st.get("task") or "")[:80],
            # 60s 버킷 양자화 — 매초 변하는 idle이 patch를 스팸하지 않도록 (§6.3)
            "idle_secs": (s.get("idle_secs") // 60) * 60
                         if isinstance(s.get("idle_secs"), int) else s.get("idle_secs"),
            "ctx": {"pct": ctx, "window": u.get("ctx_window")},
            "rate": [{"label": r.get("label"), "used_pct": r.get("used_pct"),
                      "resets_at": r.get("resets_at")}
                     for r in (u.get("rate") or [])],
            "activity": compute_activity(len(dq), self.line_rate.get(nkey, 0.0)),
            "flags": flags,
            # 오피스 디테일 v1.1 — spool 귀속 노드 상태 (없으면 null)
            "progress": self.progress.get(nkey),
            "run": self.run.get(nkey),
        }

    def _diff_patches(self):
        """노드 뷰 diff → patch 프레임 목록. (lock 보유 상태에서 호출)"""
        now = time.time()
        patches = []
        seen = {}
        for d in self.departments:
            for s in d.get("surfaces", []):
                v = self._node_view(s, now)
                seen[v["key"]] = v
                old = self.prev_nodes.get(v["key"])
                if old != v:
                    patches.append({"t": "patch", "key": v["key"], "node": v})
        self.prev_nodes = seen
        return patches

    def known_keys(self):
        """현재 fleet에 실존하는 정식 노드 키 집합 (조작 대상 allowlist)."""
        with self.lock:
            return {node_key(s) for d in self.departments
                    for s in d.get("surfaces", []) if node_key(s)}

    # --- 오피스 디테일 v1.1 ---
    def ref_count(self, key):
        """fleet에서 surface_ref == key 인 surface 수 (락 내 카운트).

        surface 번호는 부서 데몬별이라 전역 유일이 아니다(실측 다수 충돌) — 귀속 전 유일성 판정용.
        """
        if not key:
            return 0
        with self.lock:
            return sum(1 for d in self.departments for s in d.get("surfaces", [])
                       if s.get("surface_ref") == key)

    def role_key(self, role):
        """role 문자열이 fleet에서 유일한 surface에 매칭되면 그 정식 키, 아니면 None (귀속 사다리 ③)."""
        if not role:
            return None
        with self.lock:
            hits = [node_key(s) for d in self.departments
                    for s in d.get("surfaces", []) if s.get("role") == role]
        return hits[0] if len(hits) == 1 else None

    def roll_run_date(self, now):
        """로컬 날짜 롤오버 시 전 노드 done/failed_today 리셋 (queued/active는 라이브 카운터라 보존)."""
        today = time.strftime("%Y-%m-%d", time.localtime(now))
        if self.run_date != today:
            self.run_date = today
            for r in self.run.values():
                r["done_today"] = 0
                r["failed_today"] = 0

    def accumulate_heat(self, now):
        """스냅샷 틱마다 노드별 presence=='active' 시간을 로컬 24버킷 링에 누적 (기능 12)."""
        with self.lock:
            hour = time.localtime(now).tm_hour
            rolled = hour != self.heat_hour
            self.heat_hour = hour
            for d in self.departments:
                for s in d.get("surfaces", []):
                    key = node_key(s)
                    if not key:
                        continue
                    dq = self.hooks.get(key) or ()   # P2-3: 정식 키로 hook 조회
                    presence, _ = judge_presence(s, now, dq[-1] if dq else None)
                    acc = self.heat_acc.setdefault(
                        key, {"active": [0] * 24, "total": [0] * 24})
                    if rolled:  # 이 시각 버킷은 24h 전 값 → 링 재사용 위해 리셋
                        acc["active"][hour] = 0
                        acc["total"][hour] = 0
                    acc["total"][hour] += 1
                    if presence == "active":
                        acc["active"][hour] += 1

    def _board_value(self):
        """전광판 값 구성 — 호출자가 self.lock을 보유한 상태에서만 호출(락프리·재진입 방지)."""
        return {"heat": {k: [round(a / t, 3) if t else 0.0
                             for a, t in zip(v["active"], v["total"])]
                         for k, v in self.heat_acc.items()},
                "cost_today": dict(self.cost_cache["value"])}

    def board_snapshot(self, now=None):
        """전광판 데이터 (기능 12) — 24h 히트 비율 + 오늘 비용(캐시)."""
        with self.lock:
            return self._board_value()

    def top_frame(self, field, now=None):
        """상단 필드가 직전값과 다르면 patch_top 프레임, 같으면 None.

        값 구성·prev_top 비교·갱신을 **단일 self.lock 안**에서 수행한다 — "스레드별 상이 field"
        암묵 규약에 의존하지 않도록 불변식을 제거(동일 field 동시호출도 안전). board 값은 락 재획득
        없이 _board_value()로 인라인(RLock 미사용)해 이중 획득을 피한다.
        """
        with self.lock:
            if field == "blocked":
                val = [dict(b) for b in self.blocked]
            elif field == "kanban":
                val = json.loads(json.dumps(self.kanban))
            elif field == "review":
                val = json.loads(json.dumps(self.review))
            elif field == "board":
                val = self._board_value()
            else:
                return None
            if self.prev_top.get(field) == val:
                return None
            self.prev_top[field] = val
        return {"t": "patch_top", "field": field, "value": val}

    def snapshot(self):
        """전체 WorldState (계약 v1)."""
        with self.lock:
            now = time.time()
            deps = []
            unassigned = []
            for i, d in enumerate(self.departments):
                nodes = []
                for s in d.get("surfaces", []):
                    v = self._node_view(s, now)
                    if v["role"] == "무소속":
                        unassigned.append(v)
                    else:
                        nodes.append(v)
                deps.append({
                    "id": d.get("department"),
                    "floor": len(self.departments) - i,   # 첫 부서(본부)가 최상층
                    "nodes": nodes,
                })
            # ★W14 S16 — 라벨당 **정본 1개 선출**. 종전에는 dict 대입이라 나중에 순회된
            # 항목이 앞의 것을 조용히 덮었고, **완료 5/5가 미완 0/2를 덮는** 것이 실측으로
            # 재현됐다. 그 상태는 사람이 보는 화면에서 살아있는 작업을 지운다.
            # 정렬은 `javis_report`의 정본 선출 키에서 **미완 우선**을 이식한 것이다
            # (거기서는 `shadowed` 선출, 여기서는 표시 선출 — 같은 원리를 같은 순서로).
            #   ① 선언 owner 보유(= 기계가 주인을 확정한 파일)
            #   ② **미완 우선** — 살아있는 작업이 완료된 파일에 밀리지 않는다
            #   ③ 최신(age_secs 작은 쪽) ④ 경로순(결정론 tie-break)
            # ※ `javis_report`의 '정본위치(pack/round)' 키는 여기 대응물이 없다 — 브리지는
            #   팩 경로를 모른다. 경로 패턴으로 추측하는 것은 ADR-1이 없애려던 그 추론이므로
            #   넣지 않는다(누락이 아니라 의도적 부재다).
            todo_named = {}
            for p, t in sorted(self.todo.items(), key=lambda kv: self._todo_rank(*kv)):
                label = todo_label(p, t)
                if label and label not in todo_named:
                    todo_named[label] = t
            return {
                "v": 2, "ts": now, "seq": self.seq,   # v2: 정식 노드 키(<slug>@surface:N)·dept_label
                "daemon": {"version": self.daemon.get("version"),
                           "paused": bool(self.daemon.get("paused"))},
                "departments": deps,
                "todo": todo_named,
                "lobby": {"unassigned": unassigned},
                "server_room": [dict(pid=k, **v) for k, v in sorted(self.server_room.items())],
                # 오피스 디테일 v1.1 (전부 additive)
                "blocked": [dict(b) for b in self.blocked],
                "kanban": self.kanban,
                "review": self.review,
                "board": self._board_value(),   # snapshot은 이미 self.lock 보유
            }


# ------------------------------------------------------------- 브로드캐스트
class Hub:
    """SSE 클라이언트 큐 관리. 큐 포화 시 world 재동기화로 강등(상태 유실 방지)."""

    def __init__(self):
        self.lock = threading.Lock()
        self.clients = set()

    def attach(self):
        q = Queue(maxsize=500)
        with self.lock:
            self.clients.add(q)
        return q

    def detach(self, q):
        with self.lock:
            self.clients.discard(q)

    def publish(self, frame):
        data = json.dumps(frame, ensure_ascii=False)
        with self.lock:
            clients = list(self.clients)
        for q in clients:
            try:
                q.put_nowait(data)
            except Full:
                try:  # 밀린 클라이언트: 비우고 재동기화 표식
                    while True:
                        q.get_nowait()
                except Empty:
                    pass
                try:
                    q.put_nowait(json.dumps({"t": "resync"}))
                except Full:
                    pass


# ---------------------------------------------------------- 이벤트 → 프레임
TOOL_HOOKS = {"agent.hook.PreToolUse": "pre", "agent.hook.PostToolUse": "post",
              "agent.hook.PermissionRequest": "perm"}


def dog_fx(name, ev, coal, now):
    """watchdog kill/alert 이벤트 → 강아지 fx 프레임 (D4). 그 외 watchdog.* 는 차단(틱 피드 유지).

    · watchdog.duplicate_procs → {t:'dog', kind:'kill', pid:<pids 첫번째 or None>, count:N}
    · watchdog.proc_count_high → {t:'dog', kind:'alert', sid:<surface id>, count:N}
    코얼레싱: kind별 독립 10s 창(watchdog.dog.kill / watchdog.dog.alert) — 초과분 폐기.
    kill(실 프로세스 강제종료 사건)은 alert 창에 밀리지 않는다(각 kind 자기 창만 소비).
    백로그(BACKLOG_FX_SECS 초과 과거 이벤트)는 억제. t:'dog' 라 구 프론트는 무시(additive·v2 유지).
    """
    p = ev.get("payload") or {}
    if name == "watchdog.duplicate_procs":
        pids = p.get("pids") if isinstance(p.get("pids"), list) else []
        frame = {"t": "dog", "kind": "kill", "pid": pids[0] if pids else None,
                 "count": p.get("count")}
    elif name == "watchdog.proc_count_high":
        frame = {"t": "dog", "kind": "alert", "sid": ev.get("surface_id"),
                 "count": p.get("count")}
    else:
        return []   # tick_panic·load_high·duplicates_killed 등 그 외 watchdog.* 차단
    ts = ev.get("timestamp") or now
    if (now - ts) > BACKLOG_FX_SECS:
        return []   # 과거 이벤트: 연출 억제 (콜드스타트 폭주 방지) — 백로그 판정은 epoch(now vs ts)
    # coal.allow 는 now 를 넘기지 않는다: Coalescer 버킷·창은 monotonic 시계축이라 epoch now 를
    # 주입하면 (epoch-monotonic ≈ +1.78e9) 매 dog 이벤트가 전역 fx 예산을 풀리필해 flood 보호가
    # 무력화된다(reviewer1 실측). 나머지 9개 호출과 동일하게 monotonic 기본에 정렬한다.
    if not coal.allow("watchdog.dog." + frame["kind"], "dog"):
        return []   # kind별 코얼레싱 창 내 초과분 폐기
    return [frame]


def route_event(ev, world, coal, slug="main", now=None):
    """데몬 이벤트 1건 → 방출 프레임 목록 (+월드 반영). 순수 로직 (테스트 대상).

    P2-2: slug = 이 이벤트가 도착한 구독의 부서 slug. 노드 키·apply_usage·hook·flag·coalesce
    를 전부 f"{slug}@surface:{sid}" 정식 키로 스코프한다(main@ 고정 일반화). 기본값 "main" 은
    단일 구독(P1) 하위호환. coalesce·hook·flag 키를 정식 키로 둬 동번호 부서 상호 억제/오염 차단.
    반환: (frames, poke_fleet) — poke_fleet=True면 즉시 스냅샷 재폴링 필요.
    백로그 게이트(§7): 이벤트가 BACKLOG_FX_SECS보다 과거면 상태 반영만 하고
    fx 프레임은 억제한다 — "켜는 순간"의 과거 연출 폭주 방지, 상태는 손실 0.
    """
    name = ev.get("name") or ""
    now = time.time() if now is None else now
    if name.startswith("watchdog."):
        return dog_fx(name, ev, coal, now), False   # kill/alert 만 강아지 fx, 그 외 차단
    if name in NOISE_NAMES:
        return [], False
    ts = ev.get("timestamp") or now
    backlog = (now - ts) > BACKLOG_FX_SECS
    sid = ev.get("surface_id")
    p = ev.get("payload") or {}
    # P2-2: 구독 부서 slug 로 정식 노드 키 조립.
    key = f"{slug}@surface:{sid}" if sid is not None else None
    frames = []
    poke = False

    if name in TOOL_HOOKS:
        now = ev.get("timestamp") or time.time()
        if key is not None:
            world.note_hook(key, now)
        phase = TOOL_HOOKS[name]
        if phase == "perm" and not p.get("is_actionable"):
            phase = "pre"  # 비실행성 권한훅은 도구 연출로 강등
        if coal.allow(name, key):
            frames.append({"t": "fx", "kind": "tool", "key": key, "phase": phase,
                           "tool": p.get("tool_name"), "role": p.get("role"),
                           "ok": (p.get("exit_code") in (0, None))})
    elif name == "usage.updated":
        k = world.apply_usage(sid, p, slug)
        if k:
            frames.append({"t": "fx", "kind": "usage", "key": k, "pct": p.get("ctx_pct")})
    elif name == "todo.updated":
        # C4: verdict·owner 는 신버전 데몬만 싣는다(부재=구버전 → 종전 동작 · ADR-2 스큐 안전).
        world.apply_todo(p.get("path"), p.get("done"), p.get("total"),
                         p.get("verdict"), p.get("owner"))
        fx = {"t": "fx", "kind": "todo", "path": p.get("path"),
              "done": p.get("done"), "total": p.get("total")}
        flag = todo_flag(p)
        if flag is not None:
            fx["flag"] = flag   # 달 라벨이 없으면 키 자체를 만들지 않는다(구버전 프레임과 동형)
        frames.append(fx)
    elif name == "surface.input_injected":
        # from 은 동일 데몬 내 소스 surface → 같은 slug 로 정식화.
        frames.append({"t": "fx", "kind": "doc", "to": key,
                       "from": f"{slug}@surface:{p.get('from')}" if p.get("from") is not None
                               else None,
                       "bytes": p.get("bytes")})
    elif name == "queue.enqueued":
        frames.append({"t": "fx", "kind": "queue", "to": key, "depth": p.get("depth")})
    elif name == "queue.delivered":
        frames.append({"t": "fx", "kind": "doc", "to": key, "from": None,
                       "bytes": p.get("bytes")})
    elif name in ("surface.created", "surface.closed", "surface.exited"):
        poke = True
        # C4: sid 부재 시 payload.surface_ref 에서 번호 재조립 → 구독 slug 정식 키로 생성.
        frames.append({"t": "fx", "kind": "spawn" if name == "surface.created" else "despawn",
                       "key": key or f"{slug}@surface:{(p.get('surface_ref') or '').split(':')[-1]}"})
    elif name == "health.alert":
        rule = p.get("rule") or "alert"
        if key is not None and rule == "rate_limited":
            world.set_flag(key, "rate_limited", 90)
        if coal.allow(name, key):
            frames.append({"t": "fx", "kind": "alert", "key": key, "rule": rule,
                           "line": (p.get("line") or "")[:120]})
    elif name == "master.deadman":
        if coal.allow(name, key):
            frames.append({"t": "fx", "kind": "deadman", "key": key,
                           "idle_secs": p.get("idle_secs")})
    elif name in ("schedule.fired", "schedule.error"):
        if coal.allow(name, p.get("job_id")):
            frames.append({"t": "fx", "kind": "bell", "job": p.get("job_id"),
                           "error": name.endswith("error")})
    elif name in ("ledger.registered", "ledger.killed"):
        world.apply_ledger(name, p)
        frames.append({"t": "fx", "kind": "rack", "on": name == "ledger.registered",
                       "pid": p.get("pid")})
    elif name == "pane.idle":
        if coal.allow(name, key):
            frames.append({"t": "fx", "kind": "idle", "key": key,
                           "idle_secs": p.get("idle_seconds")})
    elif name == "master.idle":
        # G2 W3-B: v2 데몬의 master 침묵(정보성·category=info) — fx kind 는 pane.idle 과 같은
        # "idle" 을 공유한다(겹침은 확인된 의도 — 같은 유휴 연출·확정 결정 MINOR). role 을
        # additive 로 실어 구분하고, 코얼레스 키는 (name,surface)라 pane.idle 과 상호 억제 없음.
        # payload 필드명 주의: 데몬은 idle_secs 를 싣는다(pane.idle 의 idle_seconds 와 다름).
        if coal.allow(name, key):
            frames.append({"t": "fx", "kind": "idle", "key": key,
                           "role": p.get("role"), "idle_secs": p.get("idle_secs")})
    if backlog:  # 과거 이벤트: 상태 반영은 위에서 완료, 연출만 억제
        frames = [fr for fr in frames if fr.get("t") != "fx"]
    return frames, poke


# --------------------------------------------------- EVT spool → 프레임 (§2·§3)
def attribute_spool(entry, world):
    """spool 항목 노드 귀속 — v2 귀속 사다리 (계약 §4b).

    ① 정식 키(<slug>@surface:N) 명시 → 스냅샷 존재 검증 후 즉시 귀속.
    ② bare surface:N → 전역 유일 게이트(하위호환) → 그 surface 의 정식 키로 승격.
    ③ payload.agent==role 유일 일치 → 그 정식 키.
    ④ 미귀속(None). 틀린 귀속(오정보)보다 미귀속(정직한 공백)이 낫다.
    """
    key = entry.get("key")
    if isinstance(key, str) and key:
        if "@" in key:                       # ① 정식 키 명시
            if world.has_full_key(key):
                return key
            # 스냅샷에 없는 정식 키 → 신뢰하지 않고 role 폴백으로 강등(③)
        else:                                # ② bare surface:N (하위호환)
            fk = world.unique_full_key_for_ref(key)
            if fk is not None:
                return fk
    return world.role_key((entry.get("payload") or {}).get("agent"))


def route_spool(entry, world, coal, now=None):
    """EVT spool 항목 1건 → 방출 프레임 목록 (+월드 반영). 순수 로직 (테스트 대상).

    route_event와 동형 시그니처. 반환 (frames, poke). 상태 반영은 항상 수행하고
    fx 프레임만 Coalescer 예산·백로그 게이트(BACKLOG_FX_SECS)로 억제한다(계약 §2).
    """
    now = time.time() if now is None else now
    typ = entry.get("type")
    p = entry.get("payload") or {}
    ts = entry.get("ts") or now
    backlog = (now - ts) > BACKLOG_FX_SECS
    key = attribute_spool(entry, world)
    frames = []

    if typ == "task_progress":
        task, stage = p.get("task"), p.get("stage")
        pct = p.get("pct")
        if key is not None:
            with world.lock:
                old = world.progress.get(key)
                # 동일 (task,stage) pct 미증가 재방출 무시 (계약 재방출 정책 미러)
                stale = (old and old.get("task") == task and old.get("stage") == stage
                         and isinstance(pct, (int, float))
                         and isinstance(old.get("pct"), (int, float))
                         and pct <= old["pct"])
                if not stale:
                    world.progress[key] = {"task": task, "stage": stage, "pct": pct,
                                           "detail": p.get("detail"), "ts": ts}
            if not stale and coal.allow(typ, key):
                frames.append({"t": "fx", "kind": "progress", "key": key,
                               "task": task, "stage": stage, "pct": pct})
    elif typ in ("run.queued", "run.started", "run.succeeded", "run.failed"):
        phase = typ.split(".", 1)[1]
        if key is not None:
            with world.lock:
                world.roll_run_date(now)
                r = world.run.setdefault(
                    key, {"queued": 0, "active": None, "done_today": 0, "failed_today": 0})
                if phase == "queued":
                    r["queued"] += 1
                elif phase == "started":
                    r["queued"] = max(0, r["queued"] - 1)
                    r["active"] = {"task": p.get("task"), "started": ts}
                elif phase == "succeeded":
                    r["active"] = None
                    r["done_today"] += 1
                elif phase == "failed":
                    r["active"] = None
                    r["failed_today"] += 1
        if coal.allow(typ, key):
            frames.append({"t": "fx", "kind": "runcard", "key": key, "phase": phase,
                           "task": p.get("task"), "summary": p.get("summary")})
    elif typ == "task.blocked":
        task = p.get("task")
        bb = p.get("blocked_by") if isinstance(p.get("blocked_by"), list) else []
        with world.lock:
            world.blocked = [b for b in world.blocked if b.get("task") != task]
            world.blocked.append({"task": task, "blocked_by": bb, "key": key, "ts": ts})
        if coal.allow(typ, key):
            frames.append({"t": "fx", "kind": "blocked", "task": task,
                           "blocked_by": bb, "key": key})
    elif typ == "task.unblocked":
        task = p.get("task")
        with world.lock:
            world.blocked = [b for b in world.blocked if b.get("task") != task]
        if coal.allow(typ, key):
            frames.append({"t": "fx", "kind": "unblocked", "task": task, "key": key})

    if backlog:  # 과거 항목: 상태 반영은 위에서 완료, 연출만 억제
        frames = [fr for fr in frames if fr.get("t") != "fx"]
    return frames, False


def tail_spool(path, offset):
    """spool 파일에서 offset 이후 완결 줄만 파싱 → (entries, new_offset). 순수 로직.

    파일 축소/truncate 감지 시 0부터. 미완결 마지막 줄(쓰는 중)은 다음 폴에 넘긴다.
    깨진 JSON 줄은 skip (음성 케이스).
    """
    try:
        size = os.path.getsize(path)
    except OSError:
        return [], 0
    if size < offset:          # truncate/축소 → 처음부터
        offset = 0
    if size <= offset:
        return [], offset
    try:
        with open(path, "rb") as f:
            f.seek(offset)
            data = f.read()
    except OSError:
        return [], offset
    nl = data.rfind(b"\n")
    if nl == -1:               # 아직 완결 줄 없음
        return [], offset
    chunk = data[:nl + 1]
    entries = []
    for raw in chunk.split(b"\n"):
        if not raw.strip():
            continue
        try:
            entries.append(json.loads(raw.decode("utf-8")))
        except (ValueError, UnicodeDecodeError):
            continue           # 깨진 줄 skip
    return entries, offset + len(chunk)


def maybe_rotate_spool(path, offset):
    """드레인 상태에서 offset이 상한 초과 시 spool 로테이션. 순수 로직 (tmpdir 테스트 대상).

    반환 (rotated:bool, leftover_entries:list, new_offset:int). 호출자는 이번 폴에서 신규 항목
    0(드레인)일 때만 호출한다. 방출자(javis_event._spool_append)는 open→write(1줄 원자)→close
    **단발**이라 os.replace(rename) 이후의 append는 새 파일(O_CREAT)에 착지한다 — 이 가정 위에서
    교체 직전 창(마지막 tail~rename 사이)에 착지한 완결 줄은 .old를 기존 offset부터 한 번 더
    tail해 회수한다(무유실). 이후 .old 삭제·offset=0.
    """
    if offset <= SPOOL_ROTATE_BYTES:
        return False, [], offset
    old = path + ".old"
    try:
        os.replace(path, old)
    except OSError:
        return False, [], offset
    leftover, _ = tail_spool(old, offset)   # rename 직전 착지분 회수
    try:
        os.remove(old)
    except OSError:
        pass
    return True, leftover, 0


# ---------------------------------------------------- 칸반 스캐너 (기능 9 · §2)
_KANBAN_STATUS = {
    "done": "done", "complete": "done", "completed": "done", "closed": "done",
    "in_progress": "doing", "in-progress": "doing", "doing": "doing",
    "active": "doing", "working": "doing", "running": "doing",
    "todo": "todo", "pending": "todo", "new": "todo", "open": "todo", "queued": "todo",
    "blocked": "blocked", "waiting": "blocked",
}


def normalize_status(raw, blocked_by):
    """실물 status 필드 → todo|doing|done|blocked 정규화. 미완료+선행 미해소는 blocked."""
    norm = _KANBAN_STATUS.get((raw or "").strip().lower(), "todo")
    if norm != "done" and blocked_by:
        return "blocked"
    return norm


def scan_kanban(tasks_dir, now=None):
    """`$JAVIS_ROOT/_round/tasks/*.json`을 읽어 칸반 뷰로 정규화 (읽기 전용)."""
    now = time.time() if now is None else now
    tasks = []
    try:
        names = sorted(os.listdir(tasks_dir))
    except OSError:
        return {"ts": now, "tasks": []}
    for fn in names:
        if not fn.endswith(".json"):
            continue
        try:
            with open(os.path.join(tasks_dir, fn)) as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue
        if not isinstance(d, dict):
            continue
        bb = d.get("blocked_by") if isinstance(d.get("blocked_by"), list) else []
        tasks.append({
            "id": d.get("id") or fn[:-5],
            "title": (d.get("title") or "")[:120],
            "status": normalize_status(d.get("status"), bb),
            "owner": d.get("owner"),
            "blocked_by": bb,
        })
    return {"ts": now, "tasks": tasks}


def scan_dir_mtime(path, suffix):
    """디렉터리+매칭 파일들의 최대 mtime (스캔 게이트용). 부재/오류 시 0."""
    latest = 0.0
    try:
        latest = os.path.getmtime(path)
        for fn in os.listdir(path):
            if fn.endswith(suffix):
                try:
                    latest = max(latest, os.path.getmtime(os.path.join(path, fn)))
                except OSError:
                    pass
    except OSError:
        return 0.0
    return latest


# --------------------------------------------------- verdict 워처 (기능 10 · §2)
VERDICT_RE = re.compile(r'verdict"?\s*[:=]\s*"?(ACCEPT|REVISE|BLOCK|ESCALATE)', re.I)
REVIEWER_TOKENS = [("cso", "CSO"), ("agy", "agy"), ("codex", "codex"), ("gemini", "gemini")]


def parse_verdict(content, filename):
    """verdict md 파싱 → {reviewer,verdict,target} 또는 None. 두 실물 형식(`= X`·`: "X"`) 수용.

    reviewer는 파일명 우선·본문 보조에서 추출, target은 파일명에서 reviewer/verdict 토큰 제거 잔여.
    """
    m = VERDICT_RE.search(content or "")
    if not m:
        return None
    verdict = m.group(1).upper()
    low_name = (filename or "").lower()
    reviewer = None
    for tok, label in REVIEWER_TOKENS:
        if tok in low_name:
            reviewer = label
            break
    if reviewer is None:
        low_body = (content or "").lower()
        for tok, label in REVIEWER_TOKENS:
            if tok in low_body:
                reviewer = label
                break
    base = re.sub(r"\.(md|json)$", "", filename or "", flags=re.I)
    tgt = re.sub(r"(?i)reviewer|verdict|_?(cso|agy|codex|gemini)", "", base)
    tgt = tgt.strip("_-. ") or None
    return {"reviewer": reviewer, "verdict": verdict, "target": tgt}


def scan_verdicts(round_dir, keep=REVIEW_KEEP):
    """`*VERDICT*.md` mtime 워치 → 최신 keep건 review 항목 (계약 §2)."""
    items = []
    try:
        names = os.listdir(round_dir)
    except OSError:
        return {"ts": time.time(), "items": []}
    for fn in names:
        if not (fn.endswith(".md") and "VERDICT" in fn):
            continue
        fp = os.path.join(round_dir, fn)
        try:
            mtime = os.path.getmtime(fp)
            with open(fp, encoding="utf-8", errors="replace") as f:
                content = f.read()
        except OSError:
            continue
        parsed = parse_verdict(content, fn)
        if parsed:
            parsed["ts"] = mtime
            items.append(parsed)
    items.sort(key=lambda x: x["ts"], reverse=True)
    return {"ts": time.time(), "items": items[:keep]}


def verdict_fx(items, seen, now=None):
    """새 verdict 항목의 fx 프레임 목록. 순수 로직 (테스트 대상).

    route_event·route_spool과 동일한 백로그 정책: 과거(now-ts > BACKLOG_FX_SECS) 항목은
    seen 등록만 하고 연출을 억제한다 — (재)기동 시 seen이 비어 기존 verdict가 전부 신규로
    판정돼 fx가 폭주하는 것 방지("켜는 순간의 과거 연출 폭주 방지, 상태 손실 0"). seen은
    호출자 보유 set으로 in-place 갱신(억제해도 등록해 이후 중복 재방출 차단).
    """
    now = time.time() if now is None else now
    frames = []
    for it in items:
        sig = (it.get("reviewer"), it.get("verdict"), it.get("target"), it.get("ts"))
        if sig in seen:
            continue
        seen.add(sig)
        if (now - (it.get("ts") or now)) <= BACKLOG_FX_SECS:
            frames.append({"t": "fx", "kind": "verdict", "reviewer": it.get("reviewer"),
                           "verdict": it.get("verdict"), "target": it.get("target")})
    return frames


# ------------------------------------------------- 전광판 비용 (기능 12 · §5)
def read_cost_today(db_path, now=None):
    """transcripts.db를 read-only로 열어 오늘 비용/토큰 best-effort 산출. 불가·오류 시 null.

    스키마를 런타임 조사 — 비용/토큰 컬럼이 있으면 오늘자 합산, 없으면 {usd:null,tokens:null}.
    이 기능이 브리지를 죽이면 안 되므로 전체를 try/except로 감싼다.
    """
    import sqlite3
    now = time.time() if now is None else now
    out = {"usd": None, "tokens": None}
    con = None
    try:
        con = sqlite3.connect("file:%s?mode=ro" % db_path, uri=True, timeout=2.0)
        cols = {r[1].lower(): r[1] for r in con.execute("PRAGMA table_info(lines)")}
        # 오늘 로컬 자정 epoch
        lt = time.localtime(now)
        midnight = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 0, 0, 0, 0, 0, -1))
        has_ts = "ts" in cols
        cost_col = next((cols[c] for c in ("cost_usd", "usd", "cost") if c in cols), None)
        tok_col = next((cols[c] for c in ("tokens", "total_tokens", "token_count") if c in cols),
                       None)
        where = "WHERE ts >= ?" if has_ts else ""
        args = (midnight,) if has_ts else ()
        if cost_col:
            v = con.execute("SELECT SUM(%s) FROM lines %s" % (cost_col, where), args).fetchone()
            out["usd"] = round(v[0], 4) if v and v[0] is not None else None
        if tok_col:
            v = con.execute("SELECT SUM(%s) FROM lines %s" % (tok_col, where), args).fetchone()
            out["tokens"] = int(v[0]) if v and v[0] is not None else None
    except Exception:
        return {"usd": None, "tokens": None}
    finally:
        if con is not None:
            try:
                con.close()
            except Exception:
                pass
    return out


# ------------------------------------------------------------------ 수집 루프
ARCHIVE_PATH = os.path.join(STATE_DIR, "fx_archive.jsonl")
ARCHIVE_CAP = 20 * 1024 * 1024   # 20MB 넘으면 뒤 절반 유지


def archive_fx(ts, frame):
    """타임머신(#4) 토대 — 연출 프레임을 시각과 함께 영속 (연출 리플레이 전용)."""
    try:
        os.makedirs(STATE_DIR, exist_ok=True)
        with open(ARCHIVE_PATH, "a") as f:
            f.write(json.dumps({"ts": ts or time.time(), "fx": frame},
                               ensure_ascii=False) + "\n")
        if os.path.getsize(ARCHIVE_PATH) > ARCHIVE_CAP:
            with open(ARCHIVE_PATH) as f:
                lines = f.readlines()
            with open(ARCHIVE_PATH, "w") as f:
                f.writelines(lines[len(lines)//2:])
    except OSError:
        pass


def read_history(after_ts, limit=6000):
    out = []
    try:
        with open(ARCHIVE_PATH) as f:
            for line in f:
                try:
                    e = json.loads(line)
                except ValueError:
                    continue
                if e.get("ts", 0) > after_ts:
                    out.append(e)
    except OSError:
        pass
    return out[-limit:]


# -------------------------------------------------- 스킬 카탈로그 (D6 · GET /skills)
def skill_sources(home=None):
    """스킬 스캔 소스 [(디렉토리, 계정 라벨)] 동적 탐색.

    <ROOT>/skills(pack) + ~/.claude/skills(claude) + glob ~/.claude-*/skills
    (라벨 = 디렉토리명 'claude-' 뒤 접미사). 계정 프로필을 하드코딩하지 않아 임의 계정을
    자동 지원하며(기능 확장), 소스에 개인 핸들 리터럴이 남지 않는다(라벨은 런타임 파생).
    """
    home = home or os.path.expanduser("~")
    srcs = [(os.path.join(ROOT, "skills"), "pack"),
            (os.path.join(home, ".claude", "skills"), "claude")]
    prefix = ".claude-"
    for d in sorted(glob.glob(os.path.join(home, prefix + "*", "skills"))):
        suffix = os.path.basename(os.path.dirname(d))[len(prefix):]   # .claude-<라벨>
        srcs.append((d, suffix or "claude"))
    return srcs


SKILLS_CACHE_SECS = 60.0
SKILL_DESC_MAX = 200
_skills_cache = {"ts": 0.0, "data": None}
_skills_lock = threading.Lock()
_SKILL_FM_RE = re.compile(r"^\s*(name|description)\s*:\s*(.*)$")


def parse_skill_md(path, dirname):
    """SKILL.md → (name, description). frontmatter name/description 우선, 없으면 디렉토리명·첫 문단.

    description 은 SKILL_DESC_MAX(200) 자 절단. 파일 부재·오류 시 (dirname, "").
    """
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError:
        return dirname, ""
    name = desc = None
    body = text
    if text.startswith("---"):
        end = text.find("\n---", 3)
        if end != -1:
            body = text[end + 4:]
            for line in text[3:end].splitlines():
                m = _SKILL_FM_RE.match(line)
                if not m:
                    continue
                k, v = m.group(1), m.group(2).strip()
                if len(v) >= 2 and v[0] in "\"'" and v[-1] == v[0]:
                    v = v[1:-1]
                if k == "name" and v:
                    name = v
                elif k == "description" and v:
                    desc = v
    if not name:
        name = dirname
    if not desc:
        for line in body.splitlines():   # 첫 문단(비어있지·헤딩·구분선 아닌 첫 줄)
            s = line.strip()
            if s and not s.startswith("#") and not s.startswith("---"):
                desc = s
                break
        desc = desc or ""
    return name, desc[:SKILL_DESC_MAX]


def scan_skills(now=None, sources=None):
    """4개 소스의 SKILL.md 스캔 → 병합 스킬 목록 {"skills":[{name,description,accounts}]}.

    동일 name 은 accounts 병합(마스터·워커 계정 스킬 편차 표기). 60s 캐시(시각 비교) —
    sources 명시(테스트) 시 캐시 우회. 각 소스는 하위 디렉토리의 SKILL.md 만 채택('_'·'.' 접두 skip).
    """
    now = time.time() if now is None else now
    use_cache = sources is None
    srcs = skill_sources() if sources is None else sources
    if use_cache:
        with _skills_lock:
            if _skills_cache["data"] is not None \
               and (now - _skills_cache["ts"]) < SKILLS_CACHE_SECS:
                return _skills_cache["data"]
    merged = {}   # name → {"name","description","accounts":[...]}
    for base, account in srcs:
        try:
            entries = sorted(os.listdir(base))
        except OSError:
            continue
        for dirname in entries:
            if dirname.startswith("_") or dirname.startswith("."):
                continue
            sk = os.path.join(base, dirname, "SKILL.md")
            if not os.path.isfile(sk):
                continue
            nm, desc = parse_skill_md(sk, dirname)
            ent = merged.get(nm)
            if ent is None:
                merged[nm] = {"name": nm, "description": desc, "accounts": [account]}
            else:
                if account not in ent["accounts"]:
                    ent["accounts"].append(account)
                if not ent["description"] and desc:   # 앞 소스가 빈 설명이면 뒤 소스로 보강
                    ent["description"] = desc
    data = {"skills": sorted(merged.values(), key=lambda e: e["name"])}
    if use_cache:
        with _skills_lock:
            _skills_cache["ts"] = now
            _skills_cache["data"] = data
    return data


ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07")


def peek_surface(key, socket=None, lines=12):
    """직원 응답 회수(#2 대화) — read-screen 단발 조회 (상시 폴링 아님·관측 전용).

    M1: key 는 정식 키(<slug>@surface:N) — @ 뒤 bare surface:N 만 --surface 로 넘기고,
    부서 소켓(socket 인자)이 있으면 `cys --socket <path>` 전역 옵션으로 해당 데몬에 라우팅한다.
    """
    bare = key.split("@", 1)[1] if "@" in key else key
    args = [CYS]
    if socket:
        args += ["--socket", socket]
    args += ["read-screen", "--surface", bare, "--lines", "40"]
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=10, **NOWIN)
        if r.returncode != 0:
            return None
        txt = ANSI_RE.sub("", r.stdout)
        rows = [ln.rstrip() for ln in txt.splitlines() if ln.strip()]
        return rows[-lines:]
    except Exception:
        return None


def run_json(args, timeout=10):
    try:
        out = subprocess.run([CYS] + args, capture_output=True, text=True, timeout=timeout, **NOWIN)
        if out.returncode != 0:
            return None
        return json.loads(out.stdout)
    except Exception:
        return None


def fleet_loop(world, hub, poke):
    while True:
        fleet = run_json(["fleet", "--json"])
        status = run_json(["status", "--json"])
        patches, structural = world.merge_fleet(fleet, status)
        world.accumulate_heat(time.time())   # 전광판 히트 링 누적 (기능 12)
        if structural:
            hub.publish({"t": "world", "world": world.snapshot()})
        else:
            for fr in patches:
                hub.publish(fr)
        poke.wait(FLEET_POLL_SECS)
        poke.clear()


def reconcile_targets(desired, active, cap=SUB_CAP):
    """구독 타깃 조정 (P2-1 · 순수 로직 — 테스트 대상).

    desired: {slug: socket|None} (main 포함). active: 현재 구독 slug 집합/딕셔너리.
    반환 (to_spawn:{slug:socket}, to_reap:set) — 상한 cap 은 main 우선 + slug 정렬로 결정론적
    절단(런어웨이 방지). desired 에서 사라진 slug 은 reap, 신규 slug 은 spawn.
    """
    ordered = (["main"] if "main" in desired else []) + \
              sorted(s for s in desired if s != "main")
    capped = {s: desired[s] for s in ordered[:cap]}
    to_spawn = {s: sock for s, sock in capped.items() if s not in active}
    to_reap = set(active) - set(capped)
    return to_spawn, to_reap


class SubscriptionSupervisor:
    """부서별 (slug,socket) 이벤트 구독 fan-out 슈퍼바이저 (P2-1).

    fleet 폴 주기마다 타깃({main:None} ∪ fleet dept/socket)과 실구독을 reconcile —
    신규 부서 spawn·소멸 부서 reap(proc terminate). 공유 Hub·공유 Coalescer 경유,
    cursor 는 slug 별 cursor-<slug>.seq. 각 구독은 독립 리더 스레드 + `cys [--socket S] events`.
    """

    def __init__(self, world, hub, coal, poke, state_dir):
        self.world = world
        self.hub = hub
        self.coal = coal
        self.poke = poke
        self.state_dir = state_dir
        self.subs = {}   # slug → {"stop":Event, "proc":[Popen|None], "thread":Thread, "socket":..}

    def _reader(self, slug, socket, stop, holder):
        cursor = os.path.join(self.state_dir, f"cursor-{slug}.seq")
        args_base = [CYS] + (["--socket", socket] if socket else []) + \
                    ["events", "--reconnect", "--cursor-file", cursor]
        while not stop.is_set():
            proc = subprocess.Popen(args_base, stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, text=True, bufsize=1, **NOWIN)
            holder[0] = proc
            try:
                for line in proc.stdout:
                    if stop.is_set():
                        break
                    try:
                        ev = json.loads(line)
                    except ValueError:
                        continue
                    if ev.get("type") != "event":
                        continue
                    self.world.seq = max(self.world.seq, ev.get("seq") or 0)
                    frames, want_poke = route_event(ev, self.world, self.coal, slug)
                    for fr in frames:
                        self.hub.publish(fr)
                        if fr.get("t") in ("fx", "dog"):   # D4 강아지 fx 도 /history 리플레이 포함
                            archive_fx(ev.get("timestamp"), fr)
                    if want_poke:
                        self.poke.set()
            finally:
                try:
                    proc.terminate()
                except Exception:
                    pass
            if stop.is_set():
                break
            time.sleep(SUB_BACKOFF_SECS)   # 구독 재수립 백오프

    def _spawn(self, slug, socket):
        stop = threading.Event()
        holder = [None]
        t = threading.Thread(target=self._reader, args=(slug, socket, stop, holder), daemon=True)
        self.subs[slug] = {"stop": stop, "proc": holder, "thread": t, "socket": socket}
        t.start()

    def _reap(self, slug):
        sub = self.subs.pop(slug, None)
        if not sub:
            return
        sub["stop"].set()
        proc = sub["proc"][0]
        if proc is not None:   # stdout 블로킹 리더를 깨우려면 proc 을 외부에서 종료해야 한다
            try:
                proc.terminate()
            except Exception:
                pass

    def reconcile_once(self):
        desired = {"main": None}
        desired.update(self.world.dept_targets())
        to_spawn, to_reap = reconcile_targets(desired, self.subs)
        for slug in to_reap:
            self._reap(slug)
        for slug, sock in to_spawn.items():
            self._spawn(slug, sock)

    def run(self):
        os.makedirs(self.state_dir, exist_ok=True)
        while True:
            try:
                self.reconcile_once()
            except Exception:
                pass   # reconcile 실패가 슈퍼바이저를 죽이지 않게(다음 주기 재시도)
            time.sleep(SUB_RECONCILE_SECS)


# --------------------------------------------- 오피스 디테일 v1.1 수집 루프
def _load_offset(path):
    try:
        with open(path) as f:
            return int(f.read().strip() or 0)
    except (OSError, ValueError):
        return 0


def _save_offset(path, offset):
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(str(offset))
    except OSError:
        pass


def load_heat(world):
    """부팅 시 히트 링 복원 (브리지 재시작 생존 · §2).

    M2: 부팅 직후는 fleet 미수집이라 bare 키(surface:N)를 정식 키로 매핑할 수 없다 —
    정식 키(@ 포함)는 heat_acc 로 즉시 복원하고, bare 키는 pending_heat 로 **보존만** 한 뒤
    첫 merge_fleet 완료 시점에 1회 기회적 승격한다(_migrate_pending_heat).
    """
    try:
        with open(PRESENCE_HEAT_PATH) as f:
            d = json.load(f)
    except (OSError, ValueError):
        return
    acc = d.get("acc")
    if isinstance(acc, dict):
        for k, v in acc.items():
            a = v.get("active"); t = v.get("total")
            if isinstance(a, list) and len(a) == 24 and isinstance(t, list) and len(t) == 24:
                entry = {"active": a, "total": t}
                if isinstance(k, str) and "@" in k:
                    world.heat_acc[k] = entry          # 정식 키 → 즉시 복원
                else:
                    world.pending_heat[k] = entry      # bare 키 → 지연 승격 대기
    if isinstance(d.get("hour"), int):
        world.heat_hour = d["hour"]


def persist_heat(world):
    try:
        os.makedirs(STATE_DIR, exist_ok=True)
        with world.lock:
            payload = {"v": 2, "hour": world.heat_hour,   # v2: acc 키 = 정식 노드 키
                       "acc": {k: dict(v) for k, v in world.heat_acc.items()}}
        tmp = PRESENCE_HEAT_PATH + ".tmp"
        with open(tmp, "w") as f:
            json.dump(payload, f, ensure_ascii=False)
        os.replace(tmp, PRESENCE_HEAT_PATH)
    except OSError:
        pass


def spool_loop(world, hub, coal):
    """EVT spool tail → route_spool → SSE (1s 폴링·오프셋 영속·truncate 감지·상한 로테이션)."""
    offset = _load_offset(EVT_OFFSET_PATH)
    while True:
        entries, offset = tail_spool(EVT_SPOOL_PATH, offset)
        if not entries:   # 드레인 상태에서만 로테이션 시도 (무제한 성장 봉인)
            rotated, entries, offset = maybe_rotate_spool(EVT_SPOOL_PATH, offset)
            if rotated:
                _save_offset(EVT_OFFSET_PATH, offset)   # offset=0 즉시 영속
        if entries:
            for entry in entries:
                frames, _ = route_spool(entry, world, coal)
                for fr in frames:
                    hub.publish(fr)
                    if fr.get("t") == "fx":
                        archive_fx(entry.get("ts"), fr)
            _save_offset(EVT_OFFSET_PATH, offset)
            fr = world.top_frame("blocked")  # blocked[] 변경 시 상단 갱신
            if fr:
                hub.publish(fr)
        time.sleep(SPOOL_POLL_SECS)


def kanban_loop(world, hub):
    """`_round/tasks/*.json` mtime 게이트 스캔 → world.kanban + patch_top (기능 9)."""
    last = -1.0
    while True:
        mt = scan_dir_mtime(TASKS_DIR, ".json")
        if mt != last:
            last = mt
            kb = scan_kanban(TASKS_DIR)
            with world.lock:
                world.kanban = kb
            fr = world.top_frame("kanban")
            if fr:
                hub.publish(fr)
        time.sleep(KANBAN_POLL_SECS)


def verdict_loop(world, hub):
    """`*VERDICT*.md` mtime 워치 → world.review + fx verdict + patch_top (기능 10)."""
    last = -1.0
    seen = set()
    while True:
        mt = scan_dir_mtime(ROUND_DIR, ".md")
        if mt != last:
            last = mt
            rv = scan_verdicts(ROUND_DIR)
            with world.lock:
                world.review = rv     # 상태 반영은 무조건 (백로그 무관·손실 0)
            for fr in verdict_fx(rv["items"], seen):   # 신선 항목만 fx (과거 억제)
                hub.publish(fr)
            fr = world.top_frame("review")
            if fr:
                hub.publish(fr)
        time.sleep(VERDICT_POLL_SECS)


def board_loop(world, hub):
    """히트 링 60s 영속 + 비용 캐시 갱신 + 전광판 patch_top (기능 12)."""
    while True:
        time.sleep(BOARD_PERSIST_SECS)
        world.cost_cache = {"ts": time.time(),
                            "value": read_cost_today(TRANSCRIPTS_DB)}
        persist_heat(world)
        fr = world.top_frame("board")
        if fr:
            hub.publish(fr)


# ------------------------------------------------------------------ HTTP
class Handler(BaseHTTPRequestHandler):
    world = None
    hub = None
    routes = {}
    token = None
    origins = set()

    def log_message(self, *a):  # 조용히
        pass

    def _send(self, code, ctype, body, cache=False):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        allow = cors_header_for(self.command, self.headers.get("Origin"), self.origins)
        if allow:
            self.send_header("Access-Control-Allow-Origin", allow)
            self.send_header("Vary", "Origin")
        if cache:
            self.send_header("Cache-Control", "max-age=86400")
        else:
            # HTML·API는 항상 최신 강제 — WKWebView(iframe)가 무헤더 응답을 휴리스틱
            # 캐시해 구버전 HUD가 재시작 후에도 잔존하는 것 차단(벤더 JS만 위 max-age).
            self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/world":
            body = json.dumps(self.world.snapshot(), ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path == "/stream":
            return self._sse()
        if path == "/history":
            try:
                after = float((self.path.split("after=")[1].split("&")[0])
                              if "after=" in self.path else 0)
            except ValueError:
                after = 0.0
            body = json.dumps({"events": read_history(after)}, ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path == "/skills":   # D6: 카페 팝업스토어 진열용 스킬 카탈로그 (127.0.0.1·60s 캐시)
            body = json.dumps(scan_skills(), ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path == "/peek":
            tok = self.headers.get("X-HUD-Token")
            if not self.token or not tok or not secrets.compare_digest(str(tok), str(self.token)):
                return self._send(403, "application/json", b'{"ok": false, "error": "bad_token"}')
            key = (self.path.split("key=")[1].split("&")[0]) if "key=" in self.path else ""
            if not CMD_KEY_RE.match(key) or key not in self.world.known_keys():
                return self._send(403, "application/json", b'{"ok": false, "error": "bad_key"}')
            found, socket = self.world.socket_for(key)   # M1: 스냅샷 socket 캐리(재독 금지)
            if not found:                                 # 미지 키 → fail-closed
                return self._send(403, "application/json", b'{"ok": false, "error": "bad_key"}')
            rows = peek_surface(key, socket)
            body = json.dumps({"ok": rows is not None, "lines": rows or []},
                              ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path in self.routes:
            fp, ctype, cache = self.routes[path]
            try:
                with open(fp, "rb") as f:
                    body = f.read()
                if ctype.startswith("text/html"):   # 조작 토큰 주입 (동일 페이지 한정)
                    body = body.replace(b"__HUD_TOKEN__", (self.token or "").encode())
                    # W1-c 조용한 실패 배너 부트 가드 주입 — __HUD_TOKEN__ 치환과 별개
                    # 앵커(</head> 직전, 실측 존재). office3d.html 본문 무접촉 원칙 준수.
                    body = body.replace(
                        b"</head>",
                        b'<script src="/office-boot.js"></script>\n</head>', 1)
                    if b"/office-boot.js" not in body:  # 주입 self-check (침묵 실패 방지)
                        sys.stderr.write(
                            "[hud_bridge] WARN: office-boot.js 주입 실패 — "
                            "%s 에 </head> 앵커 부재\n" % fp)
                        sys.stderr.flush()
                return self._send(200, ctype, body, cache)
            except OSError:
                return self._send(404, "text/plain", b"missing asset")
        if path == "/favicon.ico":
            return self._send(204, "text/plain", b"")
        return self._send(404, "text/plain", b"not found")

    def do_OPTIONS(self):  # preflight 무조건 deny → 교차출처 커스텀헤더 요청 차단
        self._send(403, "text/plain", b"denied")

    def do_POST(self):
        if self.path.split("?")[0] != "/command":
            return self._send(405, "text/plain", b"unsupported")
        try:
            n = int(self.headers.get("Content-Length") or 0)
            if n <= 0 or n > 65536:
                raise ValueError
            body = json.loads(self.rfile.read(n))
        except Exception:
            return self._cmd_result(400, False, "bad_json", None)   # fail-closed
        ok, err, cleaned = gate_command(
            body, self.headers.get("X-HUD-Token"), self.token,
            self.headers.get("Origin"), self.origins, self.world.known_keys())
        if not ok:
            return self._cmd_result(403, False, err, body if isinstance(body, dict) else None)
        key, text = cleaned["key"], cleaned["text"]
        # M1 소켓 캐리: 정식 키(<slug>@surface:N)에서 스냅샷 socket 을 조회(depts.json 재독 금지).
        # 미지 키는 fail-closed 거부. @ 뒤 bare surface:N 만 --surface 로, 부서 소켓은 전역 옵션
        # `cys --socket <path>` 로 해당 데몬에 라우팅 — 본부(socket=None)는 --socket 생략.
        found, socket = self.world.socket_for(key)
        if not found:
            return self._cmd_result(403, False, "unknown_target", cleaned)
        bare = key.split("@", 1)[1]
        pre = [CYS] + (["--socket", socket] if socket else [])
        # 동일화 경로: cys send(타이핑) → send-key Return(확정). argv 배열 직접
        # 전달이라 shell 해석 표면 없음. 주입은 surface.input_injected 이벤트로
        # 데몬→브리지→SSE로 되돌아와 화면에 배달 연출로 확인된다(동시성).
        r1 = subprocess.run(pre + ["send", "--surface", bare, "--", text],
                            capture_output=True, text=True, timeout=10, **NOWIN)
        if r1.returncode != 0:
            return self._cmd_result(502, False,
                                    "send_failed: " + (r1.stderr or "")[:120], cleaned)
        r2 = subprocess.run(pre + ["send-key", "--surface", bare, "Return"],
                            capture_output=True, text=True, timeout=10, **NOWIN)
        if r2.returncode != 0:
            return self._cmd_result(502, False,
                                    "return_failed: " + (r2.stderr or "")[:120], cleaned)
        return self._cmd_result(200, True, None, cleaned)

    def _cmd_result(self, code, ok, err, req):
        try:  # 감사 로그 (원칙: 모든 조작 시도 기록 — allow·deny 불문)
            os.makedirs(STATE_DIR, exist_ok=True)
            with open(os.path.join(STATE_DIR, "command_audit.jsonl"), "a") as f:
                f.write(json.dumps({
                    "ts": time.time(), "ok": ok, "err": err,
                    "origin": self.headers.get("Origin"),
                    "key": (req or {}).get("key"),
                    "text": (req or {}).get("text"),
                }, ensure_ascii=False) + "\n")
        except OSError:
            pass
        body = json.dumps({"ok": ok, "error": err}).encode()
        self._send(code, "application/json; charset=utf-8", body)

    do_PUT = do_DELETE = do_PATCH = lambda self: self._send(405, "text/plain", b"unsupported")

    def _sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        q = self.hub.attach()
        try:
            first = json.dumps({"t": "world", "world": self.world.snapshot()},
                               ensure_ascii=False)
            self.wfile.write(f"data: {first}\n\n".encode())
            self.wfile.flush()
            while True:
                try:
                    data = q.get(timeout=15.0)
                    self.wfile.write(f"data: {data}\n\n".encode())
                except Empty:
                    self.wfile.write(b": keepalive\n\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass
        finally:
            self.hub.detach(q)


def main():
    world = World()
    hub = Hub()
    poke = threading.Event()
    os.makedirs(STATE_DIR, exist_ok=True)
    token = secrets.token_hex(16)          # 세션 토큰 — 기동마다 회전
    tk_path = os.path.join(STATE_DIR, "token")
    with open(tk_path, "w") as f:
        f.write(token)
    os.chmod(tk_path, 0o600)               # state 내부 신규 파일 권한 (게이트 자기보호)
    Handler.world = world
    Handler.hub = hub
    Handler.token = token
    Handler.origins = allowed_origins(PORT)
    Handler.routes = {
        "/": (os.path.join(WEB_DIR, "office3d.html"), "text/html; charset=utf-8", False),
        "/vendor/three.module.js":
            (os.path.join(WEB_DIR, "vendor", "three.module.js"),
             "text/javascript; charset=utf-8", True),
        # W1-c 부트 가드 — 침묵 실패 복구 안내. cache=False(no-store): 보안·복구
        # 자산이 WKWebView 휴리스틱 캐시로 구버전 잔존하면 안 됨(HTML과 동일 근거).
        "/office-boot.js":
            (os.path.join(WEB_DIR, "office-boot.js"),
             "text/javascript; charset=utf-8", False),
    }
    load_heat(world)                       # 히트 링 복원 (재시작 생존)
    world.cost_cache = {"ts": time.time(), "value": read_cost_today(TRANSCRIPTS_DB)}
    coal = Coalescer()                     # fx 전역 초당 예산 = 단일 인스턴스 (events·spool 공유)
    threading.Thread(target=fleet_loop, args=(world, hub, poke), daemon=True).start()
    # P2-1: 단일 events_loop → 부서별 멀티 구독 슈퍼바이저(공유 hub·coal·poke)
    sup = SubscriptionSupervisor(world, hub, coal, poke, STATE_DIR)
    threading.Thread(target=sup.run, daemon=True).start()
    threading.Thread(target=spool_loop, args=(world, hub, coal), daemon=True).start()
    threading.Thread(target=kanban_loop, args=(world, hub), daemon=True).start()
    threading.Thread(target=verdict_loop, args=(world, hub), daemon=True).start()
    threading.Thread(target=board_loop, args=(world, hub), daemon=True).start()
    srv = ThreadingHTTPServer((BIND, PORT), Handler)
    print(f"[hud-bridge] http://{BIND}:{PORT}  (읽기 전용 · 127.0.0.1 한정)", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
