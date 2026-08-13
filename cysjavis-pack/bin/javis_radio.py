#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_radio.py — JavisRadio 노드 간 방송 채널의 **기계 게이트** (사양: RADIO_SPEC_v4).

무엇인가: 같은 티켓에 참여한 노드(master·워커·CSO)가 서로에게 사실(FACT)·가설
(HYPOTHESIS)을 방송하고, 그 방송이 **소실되지 않고 정확히 1회 표면화**되도록 잠그는
파일 기반 채널이다. 상주 프로세스도 소켓도 쓰지 않는다 — 모든 상태는 파일이 정본이고
`wait` 는 5초 폴링 워처(watcher)일 뿐이다(크로스플랫폼 · kqueue/inotify 미사용).

왜 파일인가: 트랜스크립트(대화 기록)는 clear·compaction 으로 소멸하지만 파일은 남는다.
"파일이 SOT(단일 진실)이며 트랜스크립트 소실은 radio 상태 소실이 아니다"(§8.3)가 이
도구의 설계 축이다.

이 도구가 기계로 잠그는 것(사양 조문 ↔ 구현 지점):
  §2.3/§2.6/§2.7  seq 할당·5MB 로테이션·반줄 복구·오염 skip·GAP 봉인을 **단일 임계구역**
                  (`.seq-lock-<스레드>`)에서만 수행 — 중복 seq·레코드 표류 원천 차단
  §3.2/§3.6       FACT 의 evidence 를 **원문 대상**으로 기계 검증(파일 실존·라인 실존·
                  스니펫 일치) 한 **뒤에** 마스킹한다. 순서를 뒤집으면 참 FACT 가 부당
                  강등된다. 검증 실패 = HYPOTHESIS + confidence:=UNVERIFIED 자동 강등
  §3.4/§3.10/§3.5 BLOCKER 사유+evidence 필수(exit 5) · 쿨다운 위반 exit 8(시계 미소모) ·
                  분당 12건 차단기(거부 시도도 계수)
  §3.11           stdin 직배달 대장(ENQUEUED/INJECTED/FAILED) — '기배달' 은 INJECTED
                  기록이 있을 때만 참이다(추정 금지 · fail-closed)
  §4.2/§4.7       wake 판정은 '최종 seq > 표면화 커서' 단조 비교 하나 · 커서 2개(표면화·수용)
  §4.3/§4.9/§4.10 등급 우선 절단 + 은닉분 요약 지시자 + read 페이지네이션
  §5.6/§5.2(e)    resolve 기계 레코드가 done 게이트의 유일 판정 입력
  §7.2/§7.5       RETRACT 는 refs **이행적 폐쇄**로 대조(직접 집합만 검사 금지)
  §10.2           close 는 방류→큐 취소→드레인→잔존0 게이트를 통과해야 CLOSE 를 찍는다

명명 규약(자원 거버넌스): 이 파일과 argv 표면에는 서버 계열 문자열을 쓰지 않는다.
  `--self-test` 가 자기 argv 표면을 `javis_resource_gate.SERVER_PATTERNS` · 금지 플래그
  목록과 **기계 대조**해 위반 시 비0 종료한다(사람 눈이 아니라 게이트가 지킨다).

exit 코드(기존 게이트 공간과 비충돌):
  0 정상 · 1 내부 오류 · 2 usage/자원 · 3 능력 게이트 실패 · 4 pause(kill-switch) ·
  5 evidence 부재 · 6 리뷰어 등록 거부 · 7 close 후 send(영구 닫힘) ·
  8 쿨다운·차단기(일시 거부) · 9 락 충돌

의존성: 파이썬 표준 라이브러리 + 팩 형제 모듈(javis_lock·javis_scrub·javis_wakeup)뿐.
네트워크 0 · cys 데몬 비의존(가용하면 stdin 저지연 통로로만 쓴다).
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

# ★Windows 파이프(cp949)에서 한글 출력 UnicodeEncodeError 크래시 방어 —
#   javis_bootstrap.py:107-113 가드와 동형(errors="replace").
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   형태 규약은 javis_wakeup.py:47-57 과 동일하다(append + 중복 검사 · insert(0) 금지 —
#   목적은 발견이지 stdlib precedence 강등이 아니다). bin/tests/test_import_guard.py 가 검증한다.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

import javis_lock    # 잠금·원자 쓰기 정본(3백엔드) — top-level `import fcntl` 금지의 이유
import javis_scrub   # 비밀 마스킹 — 부재 시 즉시 실패(fail-closed · javis_wakeup:59 관례)
import javis_wakeup  # pause 판정·생존 판정·wake 큐 코얼레싱 재사용

# ── 상수 ──────────────────────────────────────────────────────────────────────
SCHEMA_VERSION = 1

EXIT_OK = 0
EXIT_ERROR = 1
EXIT_USAGE = 2
EXIT_CAPABILITY = 3      # AA33(a) 능력 게이트 — '리뷰어 거부 exit 6과 동형·상이 코드'
EXIT_PAUSED = 4          # javis_wakeup.EXIT_PAUSED 와 정렬
EXIT_NO_EVIDENCE = 5     # javis_task W0-3 어휘와 정렬
EXIT_REVIEWER = 6
EXIT_CLOSED = 7          # AA37 — 영구 닫힘
EXIT_THROTTLED = 8       # AA37 — 일시 거부(쿨다운·차단기)
EXIT_CONFLICT = 9        # javis_task checkout 충돌 코드와 정렬

GRADES = ("BLOCKER", "URGENT", "NORMAL", "FYI")
GRADE_RANK = {g: i for i, g in enumerate(GRADES)}      # 작을수록 우선(§4.3 등급 우선 절단)
EPISTEMIC = ("FACT", "HYPOTHESIS")
# §3.9 관리 레코드 — grade:=FYI 고정·도구 자동 부여·wake/델타 제외
MGMT_TYPES = ("ROTATED", "ROTATED_FROZEN", "GAP", "CLOSE")
MSG_TYPE = "MSG"
RETRACT_TYPE = "RETRACT"

# §2.5 스레드 2종 전수 — 'master 스레드' 는 존재하지 않는다(A10(c)).
THREADS = ("main", "review")
DEFAULT_THREAD = "main"

ROTATE_BYTES = 5 * 1024 * 1024      # §2.3(b)
SURFACE_CAP_BYTES = 16 * 1024       # §4.3 — 단일 표면화 폭 제한(총 토큰 절약 아님·A15)
READ_LIMIT_DEFAULT = 40             # §4.10
COMPACT_TEXT_CHARS = 140            # §4.9 하한
POLL_INTERVAL = 5.0                 # §4.2② — 크로스플랫폼 폴링 고정
HEARTBEAT_SEC = 60.0                # §4.5
WATCHER_TTL_SEC = 8 * 3600          # §4.6
BLOCKER_COOLDOWN_SEC = 600.0        # §3.4 — 발신자×티켓 스코프(A35(d))
URGENT_COOLDOWN_SEC = 120.0         # §3.10(a) — 독립 시계
BREAKER_WINDOW_SEC = 60.0
BREAKER_MAX = 12                    # §3.5 — 발신자 **전역** 스코프(A35(d))
DEMOTE_DETECT_SEC = 600.0           # §3.10(b) 강등 탐지 창
GC_RETAIN_DAYS = 14                 # §10.5
HB_STALE_SEC = 180.0                # §4.5 신선도 상한

SENTINEL_CLOSED = "WATCHER CLOSED — 재기동 금지"   # A36(b)①
SENTINEL_EXITED = "WATCHER EXITED — 즉시 재기동"   # A36(b)②

# 명명 절대 제약 — argv 표면에 나타나면 안 되는 플래그(자원 거버넌스 · 상주 서버 오인 차단).
FORBIDDEN_FLAGS = ("--socket", "--port", "--addr", "--bind")


def ROOT():
    """작업 루트 — 개인경로 하드코딩 금지. env 우선, 없으면 CWD(호출 시점 재계산)."""
    return os.environ.get("JAVIS_ROOT") or os.getcwd()


# ── 경로 ─────────────────────────────────────────────────────────────────────
def radio_dir():
    return os.path.join(ROOT(), "_round", "radio")


def archive_dir():
    return os.path.join(radio_dir(), "_archive")


def ticket_dir(ticket):
    return os.path.join(radio_dir(), ticket)


def meta_path(ticket):
    return os.path.join(ticket_dir(ticket), "META.json")


def _tp(ticket, name):
    return os.path.join(ticket_dir(ticket), name)


def thread_path(ticket, thread, meta=None):
    """활성 세그먼트 경로. 통상 `<스레드>.jsonl` 이며, Windows rename 실패 폴백
    (§2.3(c) ROTATED_FROZEN)일 때만 META 의 active_segment 가 이를 대체한다."""
    meta = meta if meta is not None else read_meta(ticket)
    act = (meta.get("active_segment") or {}).get(thread)
    return _tp(ticket, act or ("%s.jsonl" % thread))


def segment_paths(ticket, thread, meta=None):
    """이 스레드에 속한 전 세그먼트(아카이브 + 활성). 레코드는 seq 로 전역 정렬하므로
    파일 순서에 의존하지 않는다 — 로테이션·동결 폴백 어느 쪽이든 판독이 동일하다."""
    d = ticket_dir(ticket)
    out = []
    try:
        names = sorted(os.listdir(d))
    except OSError:
        return out
    pat = re.compile(r"^%s(\.[A-Za-z0-9_-]+)?\.jsonl$" % re.escape(thread))
    for n in names:
        if pat.match(n):
            out.append(os.path.join(d, n))
    active = thread_path(ticket, thread, meta)
    if active not in out and os.path.exists(active):
        out.append(active)
    return out


# ── 공용 소도구 ───────────────────────────────────────────────────────────────
def _now():
    return time.time()


def _sha256(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", str(s))[:80]


def _norm140(text):
    """§3.10(b) 유사 판정용 정규화 — 공백 정규화 후 첫 140자."""
    return re.sub(r"\s+", " ", (text or "").strip())[:COMPACT_TEXT_CHARS]


def _read_json(path, default=None):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def _jsonl_append(path, rec):
    """§2.2 패턴 — O_APPEND + 단일 os.write 로 원자 append.

    ★출처: javis_event.py:227-239 `_spool_append` 의 패턴 복제(동시 방출 안전). 여기서
      복제하는 이유는 javis_event 의 spool 경로 계약(HUD)과 대상 파일이 다르기 때문이다 —
      코드 결합 없이 **규약만** 소형 복제한다(javis_task.py:1278-1281 선례 방식).
    ★말미 개행 보정은 하지 않는다 — 대장 파일은 단일 writer 경로이고, 스레드 파일의
      반줄 복구는 §2.6 임계구역이 전담한다(책임 분리).
    """
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    line = (json.dumps(rec, ensure_ascii=False) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def _jsonl_read(path):
    """(레코드 리스트, 오염 라인번호 리스트). §2.7: 개행 없는 최종 라인은 '기록 진행 중'
    이라 무시하고(오염 아님), JSON 파싱 불능 라인만 오염으로 세고 건너뛴다."""
    recs, bad = [], []
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except OSError:
        return recs, bad
    if not raw:
        return recs, bad
    text = raw.decode("utf-8", "replace")
    lines = text.split("\n")
    trailing_partial = bool(lines and lines[-1] != "")   # 말미 개행 없음 = 기록 진행 중
    body = lines[:-1] if trailing_partial else lines[:-1]
    for i, ln in enumerate(body, start=1):
        if not ln.strip():
            continue
        try:
            obj = json.loads(ln)
        except ValueError:
            bad.append(i)
            continue
        if isinstance(obj, dict):
            recs.append(obj)
        else:
            bad.append(i)
    return recs, bad


# ── META (§2.4 · A11 락+temp-rename 직렬화) ───────────────────────────────────
def read_meta(ticket):
    return _read_json(meta_path(ticket), default={}) or {}


def meta_update(ticket, fn):
    """read-modify-write 를 javis_lock 보유 하에서만 수행한다(A11) — 펜스 필드 소실 차단.
    heartbeat 는 META 에 두지 않는다(§4.5 사이드카 분리)."""
    lp = _tp(ticket, ".meta-lock")
    try:
        with javis_lock.FileLock(lp, owner="radio-meta", blocking=True, timeout=10.0):
            m = read_meta(ticket)
            fn(m)
            javis_lock.atomic_write_json(meta_path(ticket), m)
            return m
    except javis_lock.LockError as e:
        raise RadioConflict("META 락 획득 실패(%s)" % e.status)


class RadioConflict(RuntimeError):
    pass


# ── pause 판정(§9.1) ─────────────────────────────────────────────────────────
def paused_now():
    """kill-switch 2경로 — javis_wakeup.paused_evidence() 정본 판정식을 재사용하고,
    거기에 **호출 시점 ROOT** 경로를 상위집합으로 더한다(javis_wakeup 의 ROOT 는 import
    시점에 고정되므로 테스트·다중 레인에서 갈릴 수 있다). 확인 불능도 정지로 접는다."""
    paused, hits = javis_wakeup.paused_evidence()
    live = os.path.join(ROOT(), "_round", javis_wakeup.PAUSED_BASENAME)
    if live not in hits:
        try:
            if os.path.exists(live):
                hits.append(live)
                paused = True
        except OSError as e:
            hits.append("%s (확인 불능: %s)" % (live, e))
            paused = True
    return paused, hits


# ── cys 통로 (가용할 때만 쓰는 저지연 leg · 부재는 정상 경로) ─────────────────
def _cys_bin():
    if os.environ.get("CYS_RADIO_DISABLE_CYS"):
        return None
    return shutil.which("cys")


def cys_send_queued(target, text):
    """(ok, detail) — `cys send --queued` enqueue 결과의 **동기 판독**.
    §3.7 fire-and-forget 은 '회신 대기 금지'로 유지된다 — enqueue exit 판독은 회신 대기가
    아니다(A30(a))."""
    exe = _cys_bin()
    if not exe:
        return False, "cys 미가용(경로 부재 또는 비활성)"
    try:
        r = subprocess.run([exe, "send", "--queued", "--to", target, text],
                           capture_output=True, timeout=20)
    except Exception as e:
        return False, "cys send 실행 불가: %s" % e
    if r.returncode != 0:
        tail = (r.stderr or r.stdout or b"").decode("utf-8", "replace").strip()
        return False, "cys send exit %d: %s" % (r.returncode, tail[-200:])
    return True, "enqueued"


def probe_injected(target):
    """§3.11(c) 주입 확인 — 관측 표면(`cys status --json`)의 pending 판독.
    조회 불능·pending 미제공이면 **미주입으로 취급**한다(fail-closed → 전문 동승)."""
    exe = _cys_bin()
    if not exe:
        return False
    try:
        r = subprocess.run([exe, "status", "--json"], capture_output=True, timeout=15)
        if r.returncode != 0:
            return False
        data = json.loads((r.stdout or b"").decode("utf-8", "replace"))
    except Exception:
        return False
    pend = _find_pending(data, target)
    if pend is None:
        return False        # pending 여부 미제공 = 미주입
    return pend == 0        # 큐가 비었다 = 주입 완료


def _find_pending(data, target):
    """status JSON 에서 대상 좌석의 pending 수를 찾는다(스키마 관용 탐색 · 미발견 None)."""
    found = []

    def walk(node):
        if isinstance(node, dict):
            role = node.get("role") or node.get("name") or node.get("target")
            if role == target:
                for k in ("pending", "pending_count", "queue", "queued"):
                    if isinstance(node.get(k), int):
                        found.append(node[k])
            for v in node.values():
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    walk(data)
    return found[0] if found else None


def notify_master(ticket, reason, text, urgent=False, idem=None):
    """master 통지. §3.12 — 비긴급 관리 통지는 `cys send` 직접 발신을 금지하고 wakeup 큐의
    멱등 코얼레싱(5분 digest)을 경유한다(pending 큐 캡 포화·크리티컬 tail-drop 차단).
    긴급(BLOCKER 사본·watcher 재기동 실패)만 직접 경로를 쓴다. 어느 쪽도 실패는 삼키지
    않고 폴백 원장에 남긴다 — 통지 소실이 '영구 인지 불능'이 되지 않게."""
    body = "[radio %s] %s: %s" % (ticket, reason, text)
    if urgent:
        ok, detail = cys_send_queued("master", body)
        if ok:
            return True
        _jsonl_append(_tp(ticket, ".notify-fallback.jsonl"),
                      {"ts": _now(), "reason": reason, "urgent": True,
                       "text": body, "error": detail})
        return False
    wk = os.path.join(_SELF_DIR, "javis_wakeup.py")
    key = idem or ("radio-%s-%s" % (ticket, reason))
    if _cys_bin() and os.path.isfile(wk):
        try:
            r = subprocess.run([sys.executable, wk, "enqueue", "--to", "master",
                                "--task", "radio-%s" % _safe(reason),
                                "--reason", body, "--idempotency-key", _safe(key)],
                               capture_output=True, timeout=20)
            if r.returncode == 0:
                return True
        except Exception:
            pass
    _jsonl_append(_tp(ticket, ".notify-fallback.jsonl"),
                  {"ts": _now(), "reason": reason, "urgent": False,
                   "text": body, "idem": key})
    return False


# ── evidence 기계 진위 검증 (§3.2 · §3.6(a) — 마스킹 **전** 원문 대상) ────────
_EV_RE = re.compile(r"^(.+):(\d+):(.*)$", re.S)   # greedy — Windows 'C:\x.py:12:snip' 대응


def parse_evidence(items):
    """`파일:라인:스니펫` 문자열 목록 → dict 목록. 형식 위반은 사유와 함께 반환한다."""
    out, errs = [], []
    for raw in items or []:
        m = _EV_RE.match(raw.strip())
        if not m:
            errs.append("형식 위반(파일:라인:스니펫 필요): %r" % raw[:80])
            continue
        out.append({"file": m.group(1), "line": int(m.group(2)), "snippet": m.group(3)})
    return out, errs


def verify_evidence(evidence):
    """(ok, reason, 보강된 evidence). 3중 기계 검증:
        ①대상 파일 실존 ②그 라인 실존 ③해당 라인에 인용 스니펫 포함(grep 일치)

    ★AA34(b): 통과 시 `line_hash`(대상 파일 현재 해당 라인의 SHA-256)·`snippet_hash`·
      verified:true 를 저장한다. 사후 재검증은 저장 스니펫 재grep 이 아니라 **라인 해시
      대조**로 수행한다 — 마스킹 비의존이며 사후 파일 변조·근거 소멸을 함께 탐지한다.
      (사양은 재검증 대상을 `evidence_hash` 라 부르지만 '스니펫의 해시'와 '현재 라인의
      해시'는 스니펫=전체 라인일 때만 같다 — 둘을 분리 저장해 모순을 제거했다.)
    """
    if not evidence:
        return False, "evidence 없음", []
    enriched = []
    for ev in evidence:
        path, lineno, snip = ev.get("file"), ev.get("line"), ev.get("snippet") or ""
        if not path or not isinstance(lineno, int) or lineno < 1:
            return False, "evidence 필드 불비(file/line)", []
        cand = path if os.path.isabs(path) else os.path.join(ROOT(), path)
        if not os.path.isfile(cand):
            return False, "대상 파일 부재: %s" % path, []
        try:
            with open(cand, encoding="utf-8", errors="replace") as f:
                lines = f.read().split("\n")
        except OSError as e:
            return False, "대상 파일 판독 불가: %s (%s)" % (path, e), []
        if lineno > len(lines):
            return False, "라인 부재: %s:%d (총 %d줄)" % (path, lineno, len(lines)), []
        target = lines[lineno - 1]
        if snip.strip() and snip.strip() not in target:
            return False, "스니펫 불일치: %s:%d" % (path, lineno), []
        e2 = dict(ev)
        e2["line_hash"] = _sha256(target)
        e2["snippet_hash"] = _sha256(snip)
        e2["verified"] = True
        enriched.append(e2)
    return True, "", enriched


def recheck_evidence(evidence):
    """AA34(b) 재검증 — 저장 line_hash 와 대상 파일 **현재** 해당 라인의 해시를 대조한다."""
    for ev in evidence or []:
        path, lineno, want = ev.get("file"), ev.get("line"), ev.get("line_hash")
        if not (path and isinstance(lineno, int) and want):
            return False, "재검증 입력 불비"
        cand = path if os.path.isabs(path) else os.path.join(ROOT(), path)
        try:
            with open(cand, encoding="utf-8", errors="replace") as f:
                lines = f.read().split("\n")
        except OSError:
            return False, "근거 소멸(파일 판독 불가): %s" % path
        if lineno > len(lines):
            return False, "근거 소멸(라인 부재): %s:%d" % (path, lineno)
        if _sha256(lines[lineno - 1]) != want:
            return False, "근거 변조(라인 해시 불일치): %s:%d" % (path, lineno)
    return True, ""


# ── 스레드 판독 (§2.3 로테이션 · §2.7 오염) ──────────────────────────────────
def read_thread(ticket, thread, meta=None):
    """(레코드 seq 정렬 리스트, 진단). 전 세그먼트를 모아 seq 전역 정렬한다 — 로테이션·
    동결 폴백 어느 배치에서도 판독 순서가 동일하다(파일명 의존 제거)."""
    meta = meta if meta is not None else read_meta(ticket)
    recs, corrupt = [], []
    for p in segment_paths(ticket, thread, meta):
        r, bad = _jsonl_read(p)
        recs.extend(r)
        for ln in bad:
            corrupt.append({"file": os.path.basename(p), "line": ln})
    seen, uniq = set(), []
    for r in sorted(recs, key=lambda x: (x.get("seq") if isinstance(x.get("seq"), int) else -1)):
        s = r.get("seq")
        if isinstance(s, int):
            if s in seen:
                continue
            seen.add(s)
        uniq.append(r)
    return uniq, {"corrupt": corrupt}


def sealed_seqs(records):
    """GAP 봉인 구간(§2.7(c)) — 커서 '최대 연속' 판정에서 존재하는 seq 로 취급한다."""
    out = set()
    for r in records:
        if r.get("type") == "GAP":
            a, b = r.get("gap_from"), r.get("gap_to")
            if isinstance(a, int) and isinstance(b, int):
                out.update(range(a, b + 1))
    return out


def final_seq(records):
    return max([r.get("seq") for r in records if isinstance(r.get("seq"), int)] or [0])


def is_mgmt(rec):
    return rec.get("type") in MGMT_TYPES


def is_unknown(rec):
    """AA32(a) 전방호환 — 미지 enum·상한 초과 schema_version 은 **오염이 아니라 미지
    레코드**다. 반영 의무 없이 압축 표기하고 커서는 전진시킨다(불감·커서 정지 금지)."""
    try:
        if int(rec.get("schema_version") or 1) > SCHEMA_VERSION:
            return True
    except (TypeError, ValueError):
        return True
    t = rec.get("type")
    if t is not None and t not in (MGMT_TYPES + (MSG_TYPE, RETRACT_TYPE)):
        return True
    g = rec.get("grade")
    if g is not None and g not in GRADES:
        return True
    e = rec.get("epistemic")
    if e is not None and e not in EPISTEMIC:
        return True
    return False


# ── 임계구역 append (§2.3(a) · §2.6 · A37 TOCTOU) ────────────────────────────
def append_record(ticket, thread, rec, allow_closed=False, owner="radio"):
    """seq 할당 → 말미 무결성 → 로테이션 → append 를 **단일 임계구역**에서 수행한다.
    반환 (레코드, exit코드). exit 7 = close 후 거부 · 9 = 락 충돌."""
    tdir = ticket_dir(ticket)
    os.makedirs(tdir, exist_ok=True)
    lp = _tp(ticket, ".seq-lock-%s" % thread)
    try:
        lk = javis_lock.FileLock(lp, owner=owner, blocking=True, timeout=15.0)
        with lk:
            # ★A37 TOCTOU 봉쇄: close 검사를 임계구역 **안**에서 한다. 밖에서 하면
            #   CLOSE 이후 seq 에 append 가 exit 0 으로 성공하는 유령 레코드가 생긴다.
            meta = read_meta(ticket)
            if meta.get("closed") and not allow_closed:
                return None, EXIT_CLOSED
            recs, _ = read_thread(ticket, thread, meta)
            seq = final_seq(recs) + 1
            path = thread_path(ticket, thread, meta)
            _rotate_if_needed(ticket, thread, path, recs, meta)
            path = thread_path(ticket, thread, read_meta(ticket))
            _heal_trailing_newline(path)
            rec = dict(rec)
            rec["seq"] = seq
            rec.setdefault("schema_version", SCHEMA_VERSION)
            rec.setdefault("ts", _now())
            rec["msg_id"] = "%s:%s:%d" % (ticket, thread, seq)
            _jsonl_append(path, rec)
            return rec, EXIT_OK
    except javis_lock.LockError as e:
        sys.stderr.write("seq 락 획득 실패(%s) — 재시도하지 말고 상위에 보고하라\n" % e.status)
        return None, EXIT_CONFLICT


def _heal_trailing_newline(path):
    """§2.6 — append 직전 파일 말미가 개행이 아니면(선행 기록의 크래시 반줄) 복구 개행
    1바이트를 선기록한다. 반줄과 신규 레코드의 융합(무고한 후속 레코드 소실) 차단."""
    if not os.path.exists(path):
        return
    fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o644)
    try:
        size = os.fstat(fd).st_size
        if not size:
            return
        os.lseek(fd, size - 1, os.SEEK_SET)
        if os.read(fd, 1) != b"\n":
            os.lseek(fd, 0, os.SEEK_END)
            os.write(fd, b"\n")
    finally:
        os.close(fd)


def _rotate_if_needed(ticket, thread, path, recs, meta):
    """§2.3(b)(c) — 5MB 도달 시 rename + tombstone. rename 불가(Windows 공유 위반)면
    구파일에 ROTATED_FROZEN 을 찍고 신규 세그먼트를 즉시 개시한다."""
    try:
        if not os.path.exists(path) or os.path.getsize(path) < ROTATE_BYTES:
            return
    except OSError:
        return
    fseq = final_seq(recs)
    seg_recs, _ = _jsonl_read(path)
    try:
        with open(path, "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
    except OSError:
        digest = ""
    tomb = {"schema_version": SCHEMA_VERSION, "type": "ROTATED", "grade": "FYI",
            "from": "radio", "ts": _now(), "prev_final_seq": fseq,
            "prev_records": len(seg_recs), "prev_sha256": digest}
    archived = _tp(ticket, "%s.%d.jsonl" % (thread, fseq))
    try:
        os.rename(path, archived)
    except OSError as e:
        # §2.3(c) 폴백: 구파일 동결 선언 + 신규 세그먼트 개시(활성 포인터는 META).
        frozen = {"schema_version": SCHEMA_VERSION, "type": "ROTATED_FROZEN", "grade": "FYI",
                  "from": "radio", "ts": _now(), "prev_final_seq": fseq,
                  "reason": "rename 실패: %s" % e}
        frozen["seq"] = fseq + 1
        frozen["msg_id"] = "%s:%s:%d" % (ticket, thread, fseq + 1)
        _jsonl_append(path, frozen)
        newname = "%s.f%d.jsonl" % (thread, fseq + 1)
        meta_update(ticket, lambda m: m.setdefault("active_segment", {}).__setitem__(thread, newname))
        tomb["seq"] = fseq + 2
        tomb["frozen_fallback"] = True
        tomb["msg_id"] = "%s:%s:%d" % (ticket, thread, fseq + 2)
        _jsonl_append(_tp(ticket, newname), tomb)
        return
    tomb["seq"] = fseq + 1
    tomb["msg_id"] = "%s:%s:%d" % (ticket, thread, fseq + 1)
    _jsonl_append(path, tomb)


def verify_rotation(ticket, thread):
    """A20(b) 3중 대조 — tombstone 의 prev_final_seq·레코드 수·SHA-256. 파일명 단독
    대조는 금지(레코드 표류가 무탐지 통과한다). 반환: 불일치 사유 목록."""
    problems = []
    meta = read_meta(ticket)
    recs, _ = read_thread(ticket, thread, meta)
    for r in recs:
        if r.get("type") != "ROTATED":
            continue
        pf = r.get("prev_final_seq")
        archived = _tp(ticket, "%s.%s.jsonl" % (thread, pf))
        if not os.path.exists(archived):
            problems.append("아카이브 세그먼트 부재: %s.%s.jsonl" % (thread, pf))
            continue
        seg, _bad = _jsonl_read(archived)
        if len(seg) != r.get("prev_records"):
            problems.append("레코드 수 불일치 seg=%d tombstone=%s" % (len(seg), r.get("prev_records")))
        try:
            with open(archived, "rb") as f:
                d = hashlib.sha256(f.read()).hexdigest()
        except OSError:
            d = ""
        if r.get("prev_sha256") and d != r.get("prev_sha256"):
            problems.append("체크섬 불일치: %s" % os.path.basename(archived))
        if seg and final_seq(seg) != pf:
            problems.append("최종 seq 불일치: seg=%d tombstone=%s" % (final_seq(seg), pf))
    return problems


# ── 차단기·쿨다운 (§3.5 · §3.4 · §3.10(a) · A31(c) · A35(d)) ─────────────────
def breaker_check_and_count(sender):
    """분당 12건 차단기 — **발신자 전역** 스코프(겸무를 이용한 총량 폭주 우회 차단).
    거부된 시도(쿨다운·진위·close 거부 포함)도 계수한다(A31(c)) — 거부 재시도 루프의
    무비용 폭주 차단. 쿨다운 시계와는 층위가 다르다(계수기 ≠ 시계)."""
    path = os.path.join(radio_dir(), ".breaker-%s.json" % _safe(sender))
    os.makedirs(radio_dir(), exist_ok=True)
    lp = os.path.join(radio_dir(), ".breaker-lock-%s" % _safe(sender))
    now = _now()
    try:
        with javis_lock.FileLock(lp, owner="radio-breaker", blocking=True, timeout=10.0):
            data = _read_json(path, default={"hits": []}) or {"hits": []}
            hits = [t for t in data.get("hits", [])
                    if isinstance(t, (int, float)) and now - t < BREAKER_WINDOW_SEC]
            hits.append(now)
            javis_lock.atomic_write_json(path, {"hits": hits})
            return len(hits) <= BREAKER_MAX, len(hits)
    except javis_lock.LockError:
        return False, -1        # 측정 불능은 통과가 아니다(fail-closed)


def cooldown_path(ticket, sender):
    return _tp(ticket, ".cooldown-%s.json" % _safe(sender))


def cooldown_check(ticket, sender, grade):
    """(ok, 남은 초). 스코프는 '발신자×티켓'(A35(d)) — 독립 사유의 티켓 간 긴급 전파를
    서로 간섭시키지 않는다. BLOCKER 10분 · URGENT 2분은 **독립 시계**다."""
    if grade not in ("BLOCKER", "URGENT"):
        return True, 0.0
    window = BLOCKER_COOLDOWN_SEC if grade == "BLOCKER" else URGENT_COOLDOWN_SEC
    data = _read_json(cooldown_path(ticket, sender), default={}) or {}
    last = data.get(grade)
    if not isinstance(last, (int, float)):
        return True, 0.0
    left = window - (_now() - last)
    return (left <= 0), max(left, 0.0)


def cooldown_consume(ticket, sender, grade):
    """★통과한 건만 시계를 소모한다 — 거부 건은 직전 '통과' 건 기준을 유지한다(A26)."""
    if grade not in ("BLOCKER", "URGENT"):
        return
    p = cooldown_path(ticket, sender)
    data = _read_json(p, default={}) or {}
    data[grade] = _now()
    javis_lock.atomic_write_json(p, data)


def record_blocker_rejection(ticket, sender, text, evidence, why):
    """§3.10(b) 강등 탐지 입력 — BLOCKER 거부·강등 이력을 남긴다."""
    _jsonl_append(_tp(ticket, ".blocker-reject-%s.jsonl" % _safe(sender)),
                  {"ts": _now(), "text140": _norm140(text), "why": why,
                   "evidence": [("%s:%s" % (e.get("file"), e.get("line"))) for e in evidence or []]})


def urgent_abuse_demotion(ticket, sender, text, evidence):
    """§3.10(b) — BLOCKER 거부·강등 후 10분 내의 유사 URGENT 는 자동 NORMAL 강등.
    (기계 판정: 공백 정규화 첫 140자 일치 또는 evidence 동일)"""
    recs, _ = _jsonl_read(_tp(ticket, ".blocker-reject-%s.jsonl" % _safe(sender)))
    now, t140 = _now(), _norm140(text)
    evk = sorted("%s:%s" % (e.get("file"), e.get("line")) for e in evidence or [])
    for r in recs:
        if now - (r.get("ts") or 0) > DEMOTE_DETECT_SEC:
            continue
        if r.get("text140") == t140:
            return True, "BLOCKER 거부 후 10분 내 동일 텍스트 URGENT 재송신"
        if evk and sorted(r.get("evidence") or []) == evk:
            return True, "BLOCKER 거부 후 10분 내 동일 evidence URGENT 재송신"
    return False, ""


# ── 격리·배달 대장 (§6.3 · §9.1 · §3.11) ─────────────────────────────────────
def fence_ledger(ticket, node):
    return _tp(ticket, ".fence-quarantine-%s" % _safe(node))


def pause_ledger(ticket, node):
    return _tp(ticket, ".pause-quarantine-%s" % _safe(node))


def delivery_ledger(ticket, node):
    return _tp(ticket, ".stdin-delivery-%s" % _safe(node))


def surfaced_ledger(ticket, node):
    return _tp(ticket, ".surfaced-%s" % _safe(node))


def resolve_ledger(ticket, node):
    return _tp(ticket, ".resolve-%s" % _safe(node))


def cursor_path(ticket, node):
    return _tp(ticket, ".cursor-%s" % _safe(node))


def ack_path(ticket, node):
    return _tp(ticket, ".ack-%s" % _safe(node))


def hb_path(ticket, node):
    return _tp(ticket, ".hb-%s" % _safe(node))


def watcher_lock_path(ticket, node):
    """A35(a) — 락 스코프는 '노드×티켓'. 겸무 워커는 참여 티켓 수만큼 watcher 를 각자의
    락으로 병행 보유한다(노드 단위 락은 타 티켓 전역 난청을 만든다)."""
    return _tp(ticket, ".watcher-lock-%s" % _safe(node))


def _read_cursor(path):
    v = _read_json(path, default=None)
    if isinstance(v, dict) and isinstance(v.get("seq"), int):
        return v["seq"]
    return 0


def _write_cursor(path, seq):
    javis_lock.atomic_write_json(path, {"seq": int(seq), "ts": _now()})


def quarantined_seqs(ticket, node):
    """펜스·pause 격리 대장의 합집합(seq 기준 **멱등 집합**). 물리적 중복 행이 있어도
    방류 전문 표면화는 seq 당 1회다(AA29 §6.3 추가문).

    ★두 필드는 층위가 다르다 — 섞으면 방류가 기배달로 오판된다:
      · `stdin_delivered` = 그 건에 stdin 직배달이 실제 있었는가(**불변 사실**). false 가
        한 행이라도 있으면 false 로 접는다 — '기배달 아님'의 충분조건(§3.4(d)·A24).
      · `released`        = §6.2 방류가 수행됐는가(**상태**). true 가 한 행이라도 있으면 true.
    """
    out = {}
    for p in (fence_ledger(ticket, node), pause_ledger(ticket, node)):
        recs, _ = _jsonl_read(p)
        for r in recs:
            s = r.get("seq")
            if not isinstance(s, int):
                continue
            cur = out.get(s)
            if cur is None:
                cur = {"seq": s, "grade": r.get("grade"), "from": r.get("from"),
                       "stdin_delivered": bool(r.get("stdin_delivered")), "released": False}
                out[s] = cur
            if not r.get("stdin_delivered"):
                cur["stdin_delivered"] = False
            if r.get("released"):
                cur["released"] = True
    return out


def delivery_state(ticket, node):
    """seq → {leg: state}. §3.11(c): '기배달' 은 state=INJECTED 기록이 있을 때만 참이다."""
    recs, _ = _jsonl_read(delivery_ledger(ticket, node))
    out = {}
    for r in recs:
        s, leg = r.get("seq"), r.get("leg")
        if isinstance(s, int) and leg:
            out.setdefault(s, {})[leg] = r.get("state")
    return out


def surfaced_seqs(ticket, node):
    recs, _ = _jsonl_read(surfaced_ledger(ticket, node))
    return {r.get("seq") for r in recs if isinstance(r.get("seq"), int)}


def resolved_seqs(ticket, node):
    recs, _ = _jsonl_read(resolve_ledger(ticket, node))
    return {r.get("seq") for r in recs if isinstance(r.get("seq"), int)}


# ── refs 이행적 폐쇄 · 철회 집합 (§7.2 · §5.5) ───────────────────────────────
def _by_id(records):
    idx = {}
    for r in records:
        if r.get("msg_id"):
            idx[r["msg_id"]] = r
        if isinstance(r.get("seq"), int):
            idx.setdefault(str(r["seq"]), r)
    return idx


def transitive_refs(start_refs, records):
    """§7.2 — 동일 티켓 스레드 한정 refs 이행적 폐쇄. 방문 집합 기반(순환 차단·깊이 무제한).
    ★직접 집합만의 교집합 검사는 금지다 — 3단 체인이 오통과한다."""
    idx = _by_id(records)
    seen, stack = set(), list(start_refs or [])
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        seen.add(cur)
        rec = idx.get(cur)
        if not rec:
            continue
        for nxt in rec.get("refs") or []:
            if nxt not in seen:
                stack.append(nxt)
    return seen


def retracted_ids(records):
    """철회 집합 — RETRACT 레코드가 지목한 msg-id 전수(§7.3 철회의 철회 없음)."""
    out = set()
    for r in records:
        if r.get("type") == RETRACT_TYPE:
            t = r.get("target")
            if t:
                out.add(t)
    return out


def cite_count(target_id, records):
    """refs 역방향 조회 — 대상 msg-id 를 refs 에 포함하는 레코드 수(§5.5 알고리즘)."""
    n = 0
    for r in records:
        if target_id in (r.get("refs") or []):
            n += 1
    return n


def find_pair_evidence(hypo_id, node, records):
    """§5.5 짝 증거 — 다음 5요건 전부 충족하는 레코드가 1건+ 인가.
      ①동일 티켓 스레드 ②수신자 자신이 발신 ③refs 에 해당 HYPOTHESIS msg-id 포함
      ④기계 진위 검증을 통과한 FACT(자동 강등분 제외 — demoted_from 부재)
      ⑤철회 집합에 비포함(AA28 §5.5 요건 5호)
    javis_task evidence-artifact 등 radio 외부 레코드는 짝으로 인정하지 않는다(탐색 결정론)."""
    rset = retracted_ids(records)
    for r in records:
        if r.get("from") != node:
            continue
        if hypo_id not in (r.get("refs") or []):
            continue
        if r.get("epistemic") != "FACT" or r.get("demoted_from"):
            continue
        if not r.get("verified"):
            continue
        if r.get("msg_id") in rset:
            continue
        return r
    return None


# ── send ─────────────────────────────────────────────────────────────────────
def cmd_send(a):
    ticket, thread, node = a.ticket, a.thread, a.node
    if thread not in THREADS:
        sys.stderr.write("미지 스레드 %r — 계약상 %s 2종뿐이다\n" % (thread, list(THREADS)))
        return EXIT_USAGE
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s — 먼저 `javis_radio.py open` 하라\n" % ticket)
        return EXIT_USAGE

    # ★차단기 먼저 — 거부될 시도도 계수해야 거부 재시도 루프가 무비용으로 폭주하지 않는다.
    ok, n = breaker_check_and_count(node)
    if not ok:
        sys.stderr.write("차단기: 분당 %d건 초과(관측 %s) — 일시 거부(exit 8)\n" % (BREAKER_MAX, n))
        return EXIT_THROTTLED

    grade, epistemic, text = a.grade, a.epistemic, a.text
    evidence, everr = parse_evidence(a.evidence)
    if everr:
        sys.stderr.write("evidence 형식 위반: %s\n" % "; ".join(everr))
        return EXIT_USAGE

    # §3.4 BLOCKER 는 사유+evidence 필수 (W0-3 어휘 정렬: 최소 8자·exit 5)
    if grade == "BLOCKER":
        if len((a.reason or "").strip()) < 8:
            sys.stderr.write("blocker reason required(5): BLOCKER 는 --reason 필수 "
                             "(최소 8자 — 사유 없는 최고 배달 특권 금지)\n")
            return EXIT_NO_EVIDENCE
        if not evidence:
            sys.stderr.write("evidence required(5): BLOCKER 는 --evidence 필수 "
                             "(파일:라인:스니펫 — 미검증 주장의 stdin 직배달 차단)\n")
            record_blocker_rejection(ticket, node, text, evidence, "evidence 부재")
            return EXIT_NO_EVIDENCE

    # §3.4/§3.10(a) 쿨다운 — 위반은 exit 거부 + 시계 미소모(무언 강등·큐잉 구현 금지)
    ok, left = cooldown_check(ticket, node, grade)
    if not ok:
        sys.stderr.write("쿨다운: %s 재발신까지 %.0f초 — 일시 거부(exit 8·시계 미소모)\n"
                         % (grade, left))
        if grade == "BLOCKER":
            record_blocker_rejection(ticket, node, text, evidence, "쿨다운 위반")
        notify_master(ticket, "cooldown-reject",
                      "%s 의 %s 발신이 쿨다운으로 거부됨(잔여 %.0fs)" % (node, grade, left),
                      idem="cooldown-%s-%s" % (node, grade))
        return EXIT_THROTTLED

    demoted_from = demotion_reason = confidence = None
    stdin_privileged = (grade == "BLOCKER")

    # ── ⓪ 진위 게이트(§3.2 · §3.4(a)⓪) — 마스킹 **전** 원문 대상으로 검증한다(§3.6(a))
    verified = False
    if evidence:
        vok, vwhy, enriched = verify_evidence(evidence)
        if vok:
            verified = True
            evidence = enriched
        else:
            if epistemic == "FACT":
                # §3.2 자동 강등 — §3.8 스키마. grade 는 건드리지 않는다(§3.8 추가문).
                epistemic = "HYPOTHESIS"
                demoted_from, demotion_reason = "FACT", vwhy
                confidence = "UNVERIFIED"
                text = "[DEMOTED:%s] %s" % (vwhy, text)
            if grade == "BLOCKER":
                # §3.4(a)⓪ — stdin 직배달 자격 박탈 + grade 를 URGENT 로 강등(정지 아님).
                grade = "URGENT"
                stdin_privileged = False
                demotion_reason = (demotion_reason or vwhy)
                record_blocker_rejection(ticket, node, text, evidence, "진위 검증 실패: %s" % vwhy)
                text = "[GRADE-DEMOTED:진위 검증 실패 — %s] %s" % (vwhy, text)
    elif epistemic == "FACT":
        epistemic = "HYPOTHESIS"
        demoted_from, demotion_reason = "FACT", "evidence 없음(기계 검증 불능)"
        confidence = "UNVERIFIED"
        text = "[DEMOTED:evidence 없음(기계 검증 불능)] %s" % text

    # §3.10(b) URGENT 남용 방어 — BLOCKER 거부 후 10분 내 유사 URGENT 자동 NORMAL 강등
    if grade == "URGENT" and not demoted_from:
        abuse, why = urgent_abuse_demotion(ticket, node, text, evidence)
        if abuse:
            grade = "NORMAL"
            demotion_reason = why
            text = "[GRADE-DEMOTED:%s] %s" % (why, text)
            notify_master(ticket, "urgent-abuse", "%s 의 URGENT 자동 NORMAL 강등 — %s" % (node, why),
                          idem="urgent-abuse-%s" % node)

    if epistemic == "HYPOTHESIS" and not confidence:
        confidence = a.confidence
        if not confidence:
            sys.stderr.write("confidence required: HYPOTHESIS 는 --confidence 필수 "
                             "(자동 강등분만 UNVERIFIED 자동 부여 — §3.3)\n")
            return EXIT_USAGE

    # ── §3.6(c) scrub fail-closed — 마스킹 미보장 상태의 저장은 금지다
    try:
        text_masked, nmask = javis_scrub.scrub(text)
        ev_masked = javis_scrub.scrub_obj(evidence)
    except Exception as e:
        sys.stderr.write("scrub 실행 불능(%s) — 마스킹 미보장 저장 금지(fail-closed)\n" % e)
        return EXIT_ERROR

    rec = {"schema_version": SCHEMA_VERSION, "type": MSG_TYPE, "from": node,
           "to": [t for t in (a.to or "").split(",") if t.strip()],
           "grade": grade, "epistemic": epistemic, "text": text_masked,
           "refs": [r for r in (a.refs or "").split(",") if r.strip()],
           "evidence": ev_masked, "verified": verified,
           "masked": nmask, "reason": a.reason or ""}
    if confidence:
        rec["confidence"] = confidence
    if demoted_from:
        rec["demoted_from"] = demoted_from
    if demotion_reason:
        rec["demotion_reason"] = demotion_reason

    rec, code = append_record(ticket, thread, rec, owner="radio-send-%s" % node)
    if code == EXIT_CLOSED:
        sys.stderr.write("close 후 send 거부(exit 7 · 영구 닫힘) — RETRACT 만 예외(§7.4(a))\n")
        return EXIT_CLOSED
    if code != EXIT_OK:
        return code

    cooldown_consume(ticket, node, grade)     # ★통과 건만 시계 소모
    if stdin_privileged and grade == "BLOCKER":
        _blocker_stdin_legs(ticket, node, rec)
    print(json.dumps({"sent": rec["msg_id"], "seq": rec["seq"], "grade": grade,
                      "epistemic": epistemic, "verified": verified,
                      "demoted_from": demoted_from, "masked": nmask},
                     ensure_ascii=False))
    return EXIT_OK


def _blocker_stdin_legs(ticket, sender, rec):
    """§3.4 배달 규약 — 게이트 순서 고정·fail-closed: ⓪진위(이미 통과) ①pause ②펜스.
    두 게이트를 통과한 건만 stdin 직배달하고, 결과는 §3.11 배달 대장에 즉시 기록한다."""
    seq = rec["seq"]
    header = "[radio %s seq=%d]" % (ticket, seq)     # A30(d) 수신측 중복·지연 식별 근거
    body = "%s %s [BLOCKER] %s" % (header, rec.get("reason") or "", rec.get("text") or "")
    meta = read_meta(ticket)
    recipients = [n for n in (rec.get("to") or []) if n] or \
                 [n for n in (meta.get("participants") or []) if n != sender]

    paused, hits = paused_now()
    for node in recipients:
        if paused:
            # §9.1 후단 — append 는 수행했고 stdin leg 만 정지시켜 격리 대장에 편입한다.
            _jsonl_append(pause_ledger(ticket, node),
                          {"seq": seq, "grade": rec["grade"], "from": sender,
                           "stdin_delivered": False, "ts": _now(), "why": "pause: %s" % hits})
            continue
        if _fenced(meta, node, seq):
            # §3.4(a)② 펜스 게이트 — 확인 불능도 펜스 중으로 취급(fail-closed).
            _jsonl_append(fence_ledger(ticket, node),
                          {"seq": seq, "grade": rec["grade"], "from": sender,
                           "stdin_delivered": False, "ts": _now(), "why": "fence"})
            continue
        ok, detail = cys_send_queued(node, body)
        _jsonl_append(delivery_ledger(ticket, node),
                      {"seq": seq, "leg": "recipient", "state": "ENQUEUED" if ok else "FAILED",
                       "ts": _now(), "idem_key": "radio-%s-%d-%s" % (ticket, seq, node),
                       "detail": detail})

    # (c) master 사본 — 수신자 배달·격리 여부와 무관하게 즉시. 영속 로그가 사본의 단일 진실.
    _jsonl_append(_tp(ticket, ".master-copy-log"),
                  {"seq": seq, "from": sender, "grade": rec["grade"], "ts": _now(),
                   "reason": rec.get("reason"), "text": rec.get("text"),
                   "evidence": rec.get("evidence")})
    if paused:
        _jsonl_append(pause_ledger(ticket, "master"),
                      {"seq": seq, "grade": rec["grade"], "from": sender,
                       "stdin_delivered": False, "ts": _now(), "why": "pause(master 사본)"})
    else:
        ok, detail = cys_send_queued("master", body)
        _jsonl_append(delivery_ledger(ticket, "master"),
                      {"seq": seq, "leg": "master_copy", "state": "ENQUEUED" if ok else "FAILED",
                       "ts": _now(), "idem_key": "radio-%s-%d-master" % (ticket, seq),
                       "detail": detail})


def _fenced(meta, node, seq):
    """§6.1 펜스 — 대상 노드가 펜스 중이고 seq 가 fence_seq 를 넘으면 격리한다.
    상태 확인 불능은 펜스 중으로 취급한다(fail-closed)."""
    if not isinstance(meta, dict):
        return True
    fences = meta.get("fences")
    if fences is None:
        return False
    if not isinstance(fences, dict):
        return True
    f = fences.get(node)
    if not f:
        return False
    if isinstance(f, dict):
        fs = f.get("fence_seq")
        return not isinstance(fs, int) or seq > fs
    return True


# ── 표면화 델타 산출 (§4.3 · §4.9 · §4.10 · A15) ─────────────────────────────
def compact_line(rec):
    """§4.9 압축 표기 **최소 스키마**(하한 · AA23 보강):
      {seq, from, grade, epistemic, confidence(존재 시), demoted_from(존재 시),
       text 첫 140자, evidence 유무}
    이 하한 미만의 스텁(seq·from·grade만)은 금지다 — 내용 0 전달은 반영 판단을 불능화한다."""
    parts = ["seq=%s" % rec.get("seq"), "from=%s" % rec.get("from"),
             "grade=%s" % rec.get("grade"), "epistemic=%s" % rec.get("epistemic")]
    if rec.get("confidence"):
        parts.append("confidence=%s" % rec["confidence"])
    if rec.get("demoted_from"):
        parts.append("demoted_from=%s" % rec["demoted_from"])
    parts.append("evidence=%s" % ("있음" if rec.get("evidence") else "없음"))
    txt = (rec.get("text") or "").replace("\n", " ")[:COMPACT_TEXT_CHARS]
    return "· " + " ".join(parts) + " | " + txt


def full_line(rec):
    out = ["── seq=%s from=%s grade=%s epistemic=%s%s%s"
           % (rec.get("seq"), rec.get("from"), rec.get("grade"), rec.get("epistemic"),
              (" confidence=%s" % rec["confidence"]) if rec.get("confidence") else "",
              (" demoted_from=%s" % rec["demoted_from"]) if rec.get("demoted_from") else "")]
    if rec.get("reason"):
        out.append("   사유: %s" % rec["reason"])
    out.append("   %s" % (rec.get("text") or ""))
    for ev in rec.get("evidence") or []:
        out.append("   근거: %s:%s | %s" % (ev.get("file"), ev.get("line"),
                                          (ev.get("snippet") or "")[:120]))
    if rec.get("refs"):
        out.append("   refs: %s" % ", ".join(rec["refs"]))
    return "\n".join(out)


def build_delta(ticket, node, thread, cursor, records, meta,
                recovery=False, pilot=True):
    """(payload, 표시된 seq 집합, 정착(settled) seq 집합, 은닉분).

    표기 형태 결정(§4.3·A10(d)·A15·A22):
      · 관리 레코드·자기 발신 → 표면화 없이 **정착**(커서 전진 대상)
      · 격리분 → 표면화도 정착도 아니다(커서 미전진 — 방류로만 해소)
      · 멘션·RETRACT(내가 인용 중인 msg) → 전문
      · stdin INJECTED 확인된 BLOCKER → '기배달(stdin) seq=N' 1줄
      · 이미 표시된 건 → '기표시 seq=N' 1줄
      · 그 밖 → §4.9 압축(파일럿 기간 FACT 는 무압축 — 측정 변인 통제)
      · cycle 복원 회수(recovery=True)에서는 위 압축·종결 규칙을 적용하지 않고 전 건 전문
        재표기한다 — '전문 표면화 정확히 1회' 회계가 clear 시점에 리셋되기 때문(A22).
    """
    quar = quarantined_seqs(ticket, node)
    deliv = delivery_state(ticket, node)
    shown = surfaced_seqs(ticket, node)
    sealed = sealed_seqs(records)
    rset = retracted_ids(records)
    my_refs = set()
    for r in records:
        if r.get("from") == node:
            my_refs.update(r.get("refs") or [])

    settled, entries = set(), []
    for rec in records:
        seq = rec.get("seq")
        if not isinstance(seq, int) or seq <= cursor:
            continue
        if seq in sealed:
            settled.add(seq)
            continue
        if is_mgmt(rec):
            settled.add(seq)                    # §3.9(b) 표면화 없이 소비
            continue
        if rec.get("from") == node:
            settled.add(seq)                    # §4.2③ 자기 에코 제외
            continue
        q = quar.get(seq)
        if q is not None and not q.get("released") and not recovery:
            continue                            # 격리 — 방류(§6.2) 전에는 커서 미전진
        mention = node in (rec.get("to") or [])
        form = "full"
        if recovery:
            form = "full"
        elif q is not None and q.get("released"):
            form = "full"                       # §6.2·A24(c) 방류분은 전문 1회 표면화 보장
        elif is_unknown(rec):
            form = "compact"                    # AA32(a) 미지 레코드 — 반영 의무 없이 압축
        elif rec.get("type") == RETRACT_TYPE and rec.get("target") in my_refs:
            form = "full"                       # §7.5 멘션 동등 취급
        elif seq in shown:
            form = "shown"
        elif (rec.get("grade") == "BLOCKER"
              and deliv.get(seq, {}).get("recipient") == "INJECTED"):
            form = "injected"                   # A10(d)·A30(d) — 전문 재표기 금지
        elif mention:
            form = "full"
        elif pilot and rec.get("epistemic") == "FACT" and not rec.get("demoted_from"):
            form = "full"                       # A14 파일럿 FACT 무압축
        else:
            form = "compact"
        entries.append((rec, form, mention))

    # §4.3 등급 우선 절단 — BLOCKER > URGENT > 멘션 > NORMAL > FYI, 동순위 seq 오름차순.
    def rank(item):
        rec, _form, mention = item
        g = GRADE_RANK.get(rec.get("grade"), len(GRADES))
        if g >= GRADE_RANK["NORMAL"] and mention:
            g = GRADE_RANK["URGENT"] + 0.5      # '멘션 포함 건' 은 URGENT 와 NORMAL 사이
        return (g, rec.get("seq"))

    entries.sort(key=rank)
    lines, size, displayed, hidden = [], 0, set(), []
    for rec, form, _m in entries:
        if form == "shown":
            body = "· 기표시 seq=%s (전문 표면화는 1회 — 중복 배제)" % rec.get("seq")
        elif form == "injected":
            body = "· 기배달(stdin) seq=%s" % rec.get("seq")
        elif form == "compact":
            body = compact_line(rec)
        else:
            body = full_line(rec)
            if recovery:
                body = "[cycle 복원 재배달] " + body
            if rec.get("msg_id") in rset:
                body = "[RETRACTED] " + body
        b = len(body.encode("utf-8")) + 1
        if size + b > SURFACE_CAP_BYTES and lines:
            hidden.append(rec)
            continue
        lines.append(body)
        size += b
        displayed.add(rec.get("seq"))
        settled.add(rec.get("seq"))

    if hidden:
        lines.append("")
        lines.append("[은닉 %d건 — 다음 wake 에 자동 동승 · 조기 열람은 "
                     "javis_radio.py read --from <seq>]" % len(hidden))
        for rec in hidden:
            lines.append("  · seq=%s grade=%s from=%s" %
                         (rec.get("seq"), rec.get("grade"), rec.get("from")))
    if lines:
        lines.append("")
        lines.append("[ack 지시] 이 turn 안에 `javis_radio.py ack --ticket %s --node %s <seq>` 로 "
                     "수용 커서를 기록하라 — 기록 없으면 clear 시 재배달된다(§4.7(b))." % (ticket, node))
    payload = "\n".join(lines)
    return payload, displayed, settled, hidden


def advance_cursor(cursor, records, settled):
    """§4.7(a) — '모든 seq ≤ 값이 표시 완료된 **최대 연속** seq'. 격리분·은닉분에 대해서는
    전진하지 않는다(다음 wake 델타에 자동 재포함되므로 pull 은 옵션이지 의무가 아니다)."""
    seqs = sorted(s for s in (r.get("seq") for r in records) if isinstance(s, int))
    cur = cursor
    for s in seqs:
        if s <= cur:
            continue
        if s in settled:
            cur = s
        else:
            break
    return cur


def _second_scrub(payload):
    """AA34(d) read-side 2차 scrub — 구버전·우회 경로로 저장된 비밀의 재유출 방어(심층 방어).
    표면화 payload·read 출력 양쪽에 출력 **직전** 적용한다."""
    try:
        return javis_scrub.scrub(payload)[0]
    except Exception:
        return "[scrub 불능 — 출력 보류(fail-closed)]"


# ── wait (watcher) ───────────────────────────────────────────────────────────
def _generation():
    """§4.11 세대 토큰 — 팩 업데이트로 이 스크립트가 갈리면 값이 바뀐다. 폴마다 대조해
    불일치면 sentinel② 출력 후 자진 종료한다(구버전 코드의 TTL 8h 잔존 금지)."""
    try:
        st = os.stat(os.path.abspath(__file__))
        return "%d:%d" % (st.st_mtime_ns, st.st_size)
    except OSError:
        return "unknown"


def _write_hb(ticket, node, last_surfaced_ts, generation):
    """§4.5 heartbeat 사이드카 — META 비접촉(경합 원천 제거) + A22 '최근 표면화 성공 ts'."""
    javis_lock.atomic_write_json(hb_path(ticket, node),
                                 {"ts": _now(), "pid": os.getpid(),
                                  "last_surfaced_ts": last_surfaced_ts,
                                  "generation": generation})


def cmd_wait(a):
    ticket, node, thread = a.ticket, a.node, a.thread
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE

    # A36(c) 재기동 가드 — close/done 티켓에서는 아예 기동하지 않는다(좀비 루프 차단).
    meta = read_meta(ticket)
    if meta.get("closed"):
        print(SENTINEL_CLOSED)
        return EXIT_OK

    # §4.1 싱글턴 — 락 스코프는 노드×티켓(A35(a)). busy = 이미 1개 있음(중복 기동 금지).
    lk = javis_lock.FileLock(watcher_lock_path(ticket, node), owner="radio-wait-%s" % node,
                             soft=True)
    if lk.acquire() != javis_lock.ACQUIRED:
        sys.stderr.write("watcher 이미 보유 중(%s) — '정확히 1개' 불변식(§4.1)\n" % lk.status)
        return EXIT_CONFLICT
    gen0 = _generation()
    started = _now()
    last_hb = 0.0
    last_surfaced = None
    polls = 0
    try:
        while True:
            polls += 1
            if _generation() != gen0:
                print(SENTINEL_EXITED + " (세대 교체 §4.11)")
                return EXIT_OK
            if _now() - started > WATCHER_TTL_SEC:
                meta = read_meta(ticket)
                print(SENTINEL_CLOSED if meta.get("closed") else SENTINEL_EXITED)
                return EXIT_OK

            records, diag = read_thread(ticket, thread)
            _report_corruption(ticket, thread, diag)
            problems = verify_rotation(ticket, thread)
            if problems:
                notify_master(ticket, "rotation-mismatch",
                              "URGENT 로테이션 연속성 불일치: %s" % "; ".join(problems[:3]),
                              urgent=True)
            # ★AA29 §6.1 — META 는 '스레드 읽기 후·표면화 결정 직전'에 재독한다.
            #   폴 초입의 stale META 를 판정 기준으로 삼는 것은 금지.
            meta = read_meta(ticket)
            cursor = _read_cursor(cursor_path(ticket, node))
            fseq = final_seq(records)
            if meta.get("closed"):
                # §10.2 — CLOSE 처리 순서는 '미표면화 델타 선표면화 → sentinel → 종료' 고정.
                payload, disp, settled, _h = build_delta(ticket, node, thread, cursor,
                                                         records, meta)
                if payload:
                    _emit(ticket, node, payload, disp)
                    cursor = advance_cursor(cursor, records, settled)
                    _write_cursor(cursor_path(ticket, node), cursor)
                print(SENTINEL_CLOSED)
                return EXIT_OK

            if fseq > cursor:       # §4.2 wake 판정 = 단조 seq 비교 하나(카운트 기반 금지)
                payload, disp, settled, _h = build_delta(ticket, node, thread, cursor,
                                                         records, meta)
                if payload:
                    _emit(ticket, node, payload, disp)
                    last_surfaced = _now()
                new_cur = advance_cursor(cursor, records, settled)
                if new_cur != cursor:
                    _write_cursor(cursor_path(ticket, node), new_cur)

            if _now() - last_hb >= HEARTBEAT_SEC or polls == 1:
                _write_hb(ticket, node, last_surfaced, gen0)
                last_hb = _now()

            if a.once or (a.max_polls and polls >= a.max_polls):
                return EXIT_OK
            time.sleep(a.interval)
    except KeyboardInterrupt:
        print(SENTINEL_EXITED)
        return EXIT_OK
    except Exception as e:                      # 오류 종료도 재기동 sentinel(A36(b)②)
        sys.stderr.write("watcher 오류: %s\n" % e)
        print(SENTINEL_EXITED)
        return EXIT_ERROR
    finally:
        lk.release()


def _emit(ticket, node, payload, displayed):
    """표면화 — 출력 직전 2차 scrub 후 기표시 대장에 등재한다(중복 배제의 근거)."""
    print(_second_scrub(payload))
    for s in sorted(x for x in displayed if isinstance(x, int)):
        _jsonl_append(surfaced_ledger(ticket, node), {"seq": s, "ts": _now()})


def _report_corruption(ticket, thread, diag):
    """§2.7(b) — 오염 라인은 skip 하고 계속 판독한다(crash·재기동 루프 금지). 최초 발견자만
    사이드카 대조 후 URGENT 로 1회 통지한다."""
    bad = diag.get("corrupt") or []
    if not bad:
        return
    side = _tp(ticket, ".corrupt-%s" % thread)
    known = {(r.get("file"), r.get("line")) for r in _jsonl_read(side)[0]}
    fresh = [b for b in bad if (b.get("file"), b.get("line")) not in known]
    if not fresh:
        return
    for b in fresh:
        _jsonl_append(side, dict(b, ts=_now()))
    notify_master(ticket, "corrupt-line",
                  "URGENT 오염 라인 %d건 발견(%s) — GAP 봉인 승인 필요(§2.7(c))"
                  % (len(fresh), thread), urgent=True)


# ── read (§4.10 페이지네이션) ────────────────────────────────────────────────
def cmd_read(a):
    records, diag = read_thread(a.ticket, a.thread)
    _ = diag
    start = a.from_seq or 0
    sel = [r for r in records if isinstance(r.get("seq"), int) and r["seq"] > start]
    page, rest = sel[:a.limit], sel[a.limit:]
    out = []
    rset = retracted_ids(records)
    for r in page:
        line = full_line(r) if not is_mgmt(r) else "· [관리] seq=%s type=%s" % (r.get("seq"), r.get("type"))
        if r.get("msg_id") in rset:
            line = "[RETRACTED] " + line
        out.append(line)
    if rest:
        out.append("")
        out.append("[다음 페이지: --from %s · 잔여 %d건]" % (page[-1].get("seq"), len(rest)))
    print(_second_scrub("\n".join(out) if out else "(신규 없음)"))
    return EXIT_OK


# ── ack / resolve (§4.7(b) · §5.6) ───────────────────────────────────────────
def cmd_ack(a):
    _write_cursor(ack_path(a.ticket, a.node), a.seq)
    print(json.dumps({"ack": a.seq, "node": a.node}, ensure_ascii=False))
    return EXIT_OK


def cmd_resolve(a):
    """§5.6 — BLOCKER·URGENT 의 반영·기각을 기계 레코드로 남긴다. 트랜스크립트·자유형
    todo 문자열은 게이트 판정 입력으로 **불인정**(자유형 파싱 오거부와 자기보고 신뢰
    통과의 양극단을 동시에 배제)."""
    if len((a.note or "").strip()) < 8:
        sys.stderr.write("resolve 근거 부족(5): --note 최소 8자\n")
        return EXIT_NO_EVIDENCE
    _jsonl_append(resolve_ledger(a.ticket, a.node),
                  {"seq": a.seq, "action": a.action, "note": javis_scrub.scrub(a.note)[0],
                   "ts": _now(), "node": a.node})
    print(json.dumps({"resolved": a.seq, "action": a.action}, ensure_ascii=False))
    return EXIT_OK


# ── retract (§7.4 · §7.5 · §7.2) ─────────────────────────────────────────────
def cmd_retract(a):
    ticket, thread, node = a.ticket, a.thread, a.node
    records, _ = read_thread(ticket, thread)
    idx = _by_id(records)
    target = a.target if a.target in idx else None
    if target is None:
        sys.stderr.write("철회 대상 미발견: %s\n" % a.target)
        return EXIT_USAGE
    n = cite_count(target, records)
    grade = "URGENT" if n >= 3 else "NORMAL"      # §7.5 — FYI 부여 금지(무기한 미도달 차단)
    rec = {"schema_version": SCHEMA_VERSION, "type": RETRACT_TYPE, "from": node,
           "to": [], "grade": grade, "epistemic": "FACT", "target": target,
           "text": javis_scrub.scrub("[RETRACT] %s — %s" % (target, a.reason or ""))[0],
           "refs": [target], "cite_count": n}
    # §7.4(a) — RETRACT 는 close 후에도 append 가 허용된다(send 거부의 유일한 예외).
    rec, code = append_record(ticket, thread, rec, allow_closed=True, owner="radio-retract")
    if code != EXIT_OK:
        return code
    flagged = _retract_backtrace(ticket, thread, target, records)
    print(json.dumps({"retracted": target, "seq": rec["seq"], "grade": grade,
                      "cite_count": n, "recheck_flagged": flagged}, ensure_ascii=False))
    return EXIT_OK


def _retract_backtrace(ticket, thread, target, records):
    """§7.4(b)·AA28 — 대상 msg-id 를 **이행적 폐쇄**에 포함하는 done 통과 티켓을 역추적해
    재검토 플래그를 기록하고 master 에 1건 통지한다(팬아웃 아님 · master 단일 수신).
    짝 증거(§5.5)로 쓰인 경우도 대상에 포함한다."""
    hits = []
    tasks = os.path.join(ROOT(), "_round", "tasks")
    try:
        names = sorted(n for n in os.listdir(tasks) if n.endswith(".json"))
    except OSError:
        names = []
    for n in names:
        t = _read_json(os.path.join(tasks, n), default={}) or {}
        if t.get("status") not in ("done", "DONE"):
            continue
        refs = t.get("refs") or []
        if not isinstance(refs, list):
            continue
        closure = transitive_refs(refs, records)
        if target in closure:
            hits.append(t.get("id") or n)
    # radio 내부: 대상을 폐쇄에 포함하는 자기 레코드(짝 증거 포함)도 플래그 대상이다.
    for r in records:
        if r.get("type") == RETRACT_TYPE:
            continue
        if target in transitive_refs(r.get("refs") or [], records):
            hits.append(r.get("msg_id"))
    hits = sorted({h for h in hits if h})
    if hits:
        _jsonl_append(_tp(ticket, ".recheck-flags.jsonl"),
                      {"target": target, "flagged": hits, "ts": _now()})
        notify_master(ticket, "retract-recheck",
                      "철회 %s — 재검토 플래그 %d건: %s" % (target, len(hits), ", ".join(hits[:5])),
                      idem="retract-%s" % target)
    return hits


# ── done 게이트 (§5.2(e) · §5.5 · §7.2) ──────────────────────────────────────
def cmd_done_check(a):
    """AA38 §5.2(e) — '표면화된' 건 한정을 폐지하고 **스레드 직독**으로 판정한다.
      ①미표면화 건(커서·ack 미도달 · cys 큐 대기 stdin leg 포함) 잔존 → 거부
      ②표면화 건 중 §5.6 resolve 레코드 부재 → 거부
      ③산출물 refs 의 이행적 폐쇄가 철회 집합과 교집합 → 거부
      ④인용한 HYPOTHESIS 에 §5.5 짝 증거 0건 → 거부
    A25(c): 확인 레코드 부재로 인한 대기는 **워커 귀책이 아님**을 사유에 기계 판별해 남긴다."""
    ticket, node, thread = a.ticket, a.node, a.thread
    records, _ = read_thread(ticket, thread)
    cursor = _read_cursor(cursor_path(ticket, node))
    ack = _read_cursor(ack_path(ticket, node))
    resolved = resolved_seqs(ticket, node)
    quar = quarantined_seqs(ticket, node)
    confirm = os.path.exists(_tp(ticket, ".pause-release-confirm-%s" % _safe(node)))
    reasons = []

    mine = []
    for r in records:
        if is_mgmt(r) or r.get("from") == node:
            continue
        if r.get("grade") not in ("BLOCKER", "URGENT"):
            continue
        to = r.get("to") or []
        if to and node not in to:
            continue
        mine.append(r)

    for r in mine:
        seq = r["seq"]
        if seq > cursor or seq > ack:
            why = "미표면화/미수용 seq=%d(커서=%d ack=%d)" % (seq, cursor, ack)
            if seq in quar and not confirm:
                why += " ※master 확인 레코드 부재로 인한 대기 — 워커 귀책 아님(A25(c))"
            reasons.append(why)
        elif seq not in resolved:
            reasons.append("resolve 레코드 부재 seq=%d — 반영·기각 기록 필요(§5.6)" % seq)

    rset = retracted_ids(records)
    refs = [x for x in (a.refs or "").split(",") if x.strip()]
    closure = transitive_refs(refs, records)
    hit = closure & rset
    if hit:
        reasons.append("철회된 근거 인용(이행적 폐쇄 교집합): %s" % ", ".join(sorted(hit)))
    idx = _by_id(records)
    for rid in closure:
        rec = idx.get(rid)
        if rec and rec.get("epistemic") == "HYPOTHESIS":
            if not find_pair_evidence(rid, node, records):
                reasons.append("HYPOTHESIS %s 의 짝 증거 0건(§5.5) — 독립 재유도 필요" % rid)

    if reasons:
        for r in reasons:
            print("[DONE-BLOCK] %s" % r)
        print("done 게이트: BLOCK — exit %d" % EXIT_NO_EVIDENCE)
        return EXIT_NO_EVIDENCE
    print("done 게이트: PASS (미표면화 0 · resolve 완비 · 철회 인용 0 · 짝 증거 완비)")
    return EXIT_OK


# ── open (§1.2 · AA33(a)) ────────────────────────────────────────────────────
def capability_receipt(node):
    return os.path.join(radio_dir(), ".capability-%s.json" % _safe(node))


def cmd_open(a):
    ticket = a.ticket
    parts = [p.strip() for p in (a.participants or "").split(",") if p.strip()]
    if not parts:
        sys.stderr.write("참여자 필요: --participants a,b,c\n")
        return EXIT_USAGE
    # §6 리뷰어는 워커가 아니다 — radio 참여자로 등록하지 않는다(exit 6).
    bad = [p for p in parts if p.startswith("reviewer-")]
    if bad:
        sys.stderr.write("리뷰어 등록 거부(exit 6): %s — 리뷰어는 master 전용 검증·반박 "
                         "노드이지 radio 피어가 아니다\n" % ", ".join(bad))
        return EXIT_REVIEWER
    # AA33(a) 능력 게이트 — 참여자별 --self-test exit 0 영수증이 있어야 등록된다.
    missing = []
    for p in parts:
        rec = _read_json(capability_receipt(p), default=None)
        if not rec or rec.get("ok") is not True:
            missing.append(p)
    if missing and not a.skip_capability_gate:
        sys.stderr.write(
            "능력 게이트 실패(exit 3): %s — 각 노드에서 "
            "`javis_radio.py --self-test --record-capability --node <노드>` 를 먼저 통과시켜라 "
            "(radio 불능 노드의 난청 등록·재기동 무한 루프를 개통 시점에 차단)\n" % ", ".join(missing))
        notify_master(ticket, "capability-gate", "능력 게이트 미통과 노드: %s" % ", ".join(missing))
        return EXIT_CAPABILITY

    os.makedirs(ticket_dir(ticket), exist_ok=True)
    meta = {"ticket": ticket, "schema_version": SCHEMA_VERSION, "participants": parts,
            "threads": list(THREADS), "fences": {}, "closed": False,
            "opened_at": _now(), "active_segment": {},
            "capability_gate": "skipped" if (missing and a.skip_capability_gate) else "passed"}
    meta_update(ticket, lambda m: m.update(meta))
    for th in THREADS:
        p = _tp(ticket, "%s.jsonl" % th)
        if not os.path.exists(p):
            open(p, "a", encoding="utf-8").close()
    print(json.dumps({"opened": ticket, "participants": parts, "threads": list(THREADS)},
                     ensure_ascii=False))
    return EXIT_OK


# ── close (§10.2 정리 시퀀스 · AA37) ─────────────────────────────────────────
def cmd_close(a):
    ticket = a.ticket
    if not os.path.isdir(ticket_dir(ticket)):
        sys.stderr.write("티켓 미개통: %s\n" % ticket)
        return EXIT_USAGE
    meta = read_meta(ticket)
    if meta.get("closed"):
        print(json.dumps({"closed": ticket, "already": True}, ensure_ascii=False))
        return EXIT_OK
    parts = list(meta.get("participants") or [])
    report = {"released": 0, "cancelled": 0, "cancel_failed": 0, "drain_short": [], "residual": []}

    # (i)(ii) 펜스 강제 해제 + 격리 대장 잔존분 방류 — verdict 종류·취소 여부와 무관.
    meta_update(ticket, lambda m: m.__setitem__("fences", {}))
    for node in parts + ["master"]:
        for ledger in (fence_ledger(ticket, node), pause_ledger(ticket, node)):
            rows, _ = _jsonl_read(ledger)
            state = {}
            for r in rows:
                s = r.get("seq")
                if not isinstance(s, int):
                    continue
                cur = state.setdefault(s, dict(r))   # seq 멱등 — 중복 행이 있어도 1건으로 접는다
                if r.get("released"):
                    cur["released"] = True
            for s, r in sorted(state.items()):
                if r.get("released"):
                    continue                          # 이미 방류된 건은 재방류하지 않는다
                _jsonl_append(ledger, dict(r, seq=s, released=True,
                                           released_at=_now(), released_by="close"))
                report["released"] += 1

    # (iii) stdin leg 정리 — 해당 티켓 radio 발 pending 을 큐에서 취소(멱등키).
    ok, detail = _cancel_pending(ticket)
    if ok:
        report["cancelled"] += 1
    else:
        report["cancel_failed"] += 1
        _jsonl_append(_tp(ticket, ".stdin-cancel-log.jsonl"),
                      {"ts": _now(), "ok": False, "detail": detail})

    # (iv) 드레인 게이트 — 전 참여 워커의 표면화 커서 == 스레드 최종 비관리 seq
    for th in THREADS:
        records, _ = read_thread(ticket, th)
        nonmgmt = [r.get("seq") for r in records
                   if isinstance(r.get("seq"), int) and not is_mgmt(r)]
        target = max(nonmgmt or [0])
        for node in parts:
            cur = _read_cursor(cursor_path(ticket, node))
            if cur < target:
                report["drain_short"].append("%s/%s cursor=%d < %d" % (node, th, cur, target))

    # (v) 잔존 0 게이트 — 미방류(stdin_delivered:false)·미처리 FAILED 잔존 0건 기계 검증
    for node in parts + ["master"]:
        for s, r in sorted(quarantined_seqs(ticket, node).items()):
            if not r.get("released"):
                report["residual"].append("%s 격리 미방류 seq=%d" % (node, s))
        # A30(b): FAILED 는 stdin 배달이 없었던 건이므로 **델타 전문 표기가 곧 복구 경로**다.
        # 따라서 '미처리 FAILED' 는 아직 그 노드에 표면화되지 않은 건만을 뜻한다 — 이미
        # 표면화된 FAILED 를 잔존으로 세면 정상 복구된 건이 close 를 영구 봉쇄한다.
        shown = surfaced_seqs(ticket, node)
        for s, legs in delivery_state(ticket, node).items():
            for leg, st in legs.items():
                if st == "FAILED" and s not in shown:
                    report["residual"].append("%s %s seq=%d FAILED 미표면화" % (node, leg, s))

    if (report["drain_short"] or report["residual"]) and not a.force:
        for x in report["drain_short"] + report["residual"]:
            print("[CLOSE-BLOCK] %s" % x)
        print("close 거부 — 드레인·잔존0 게이트 미통과(exit %d)" % EXIT_NO_EVIDENCE)
        return EXIT_NO_EVIDENCE

    rec = {"schema_version": SCHEMA_VERSION, "type": "CLOSE", "grade": "FYI",
           "from": a.node, "ts": _now(), "report": report}
    for th in THREADS:
        r2, code = append_record(ticket, th, dict(rec), owner="radio-close")
        if code not in (EXIT_OK,):
            return code
    meta_update(ticket, lambda m: (m.__setitem__("closed", True),
                                   m.__setitem__("closed_at", _now())))
    print(json.dumps({"closed": ticket, **report}, ensure_ascii=False))
    return EXIT_OK


def _cancel_pending(ticket):
    """§10.2(iii) — cys 명령이 가용하면 wakeup 큐 취소를 시도하고, 불가하면 대장에 기록한다.
    취소 불능 지연 배달분은 A30(d) 의 `[radio <티켓> seq=N]` 헤더로 수신측이 식별한다."""
    wk = os.path.join(_SELF_DIR, "javis_wakeup.py")
    if not _cys_bin() or not os.path.isfile(wk):
        return False, "cys 미가용 — 큐 취소 시도 불가(헤더 식별로 폴백)"
    try:
        r = subprocess.run([sys.executable, wk, "cancel", "--task", "radio-%s" % _safe(ticket),
                            "--reason", "radio close"], capture_output=True, timeout=20)
        if r.returncode == 0:
            return True, "cancelled"
        return False, "wakeup cancel exit %d" % r.returncode
    except Exception as e:
        return False, "wakeup cancel 실행 불가: %s" % e


# ── GAP 봉인 (§2.7(c)) ───────────────────────────────────────────────────────
def cmd_seal_gap(a):
    if not a.confirm:
        sys.stderr.write("GAP 봉인은 master 승인 필요 — --confirm 없이는 거부(정상 데이터 봉인 차단)\n")
        return EXIT_USAGE
    rec = {"schema_version": SCHEMA_VERSION, "type": "GAP", "grade": "FYI", "from": a.node,
           "gap_from": a.from_seq, "gap_to": a.to_seq, "reason": a.reason or ""}
    rec, code = append_record(a.ticket, a.thread, rec, owner="radio-gap")
    if code != EXIT_OK:
        return code
    print(json.dumps({"sealed": [a.from_seq, a.to_seq], "seq": rec["seq"]}, ensure_ascii=False))
    return EXIT_OK


# ── GC·고아 검출 (§10.5) ─────────────────────────────────────────────────────
def cmd_gc(a):
    """close 후 14일 보존 뒤 `_archive/` 로 **이동**(삭제 아님 — 감사 가역·denylist 비저촉).
    + 'javis_task done/취소인데 META close=false' 고아 티켓 스캔."""
    moved, orphans = [], []
    tasks = os.path.join(ROOT(), "_round", "tasks")
    try:
        tickets = sorted(n for n in os.listdir(radio_dir())
                         if os.path.isdir(os.path.join(radio_dir(), n)) and not n.startswith("_"))
    except OSError:
        tickets = []
    for t in tickets:
        meta = read_meta(t)
        if meta.get("closed"):
            age = _now() - (meta.get("closed_at") or _now())
            if age >= GC_RETAIN_DAYS * 86400 or a.force:
                os.makedirs(archive_dir(), exist_ok=True)
                dest = os.path.join(archive_dir(), t)
                if not os.path.exists(dest):
                    shutil.move(ticket_dir(t), dest)
                    moved.append(t)
            continue
        tj = _read_json(os.path.join(tasks, "%s.json" % t), default=None)
        if tj and tj.get("status") in ("done", "DONE", "cancelled", "canceled"):
            orphans.append(t)
    if orphans:
        notify_master(orphans[0], "orphan-ticket",
                      "javis_task done/취소인데 META close=false: %s — §10.2 지연 close 필요"
                      % ", ".join(orphans), idem="orphan-scan")
    print(json.dumps({"archived": moved, "orphans": orphans}, ensure_ascii=False))
    return EXIT_OK


# ── argv 표면 자기 대조 (명명 절대 제약) ─────────────────────────────────────
def _argv_surface(parser):
    surface = [os.path.basename(os.path.abspath(__file__))]

    def walk(p):
        for act in p._actions:
            surface.extend(act.option_strings)
            sub = getattr(act, "_name_parser_map", None)
            if sub:
                for name, sp in sub.items():
                    surface.append(name)
                    walk(sp)

    walk(parser)
    return sorted(set(x for x in surface if x))


def _check_naming(parser):
    """--self-test 의 명명 게이트 — 자기 argv 표면을 javis_resource_gate 의 SERVER_PATTERNS·
    금지 플래그와 **기계 대조**한다. import 실패도 위반으로 접는다(측정 불능 ≠ 통과)."""
    fails = []
    try:
        import javis_resource_gate as rg
        patterns = [re.compile(p) for p in rg.SERVER_PATTERNS]
    except Exception as e:
        return ["javis_resource_gate 판독 불가(%s) — 명명 대조 측정 불능은 통과가 아니다" % e]
    surface = _argv_surface(parser)
    joined = " ".join(surface)
    for pat in patterns:
        if pat.search(joined):
            fails.append("argv 표면이 SERVER_PATTERNS %r 에 매칭" % pat.pattern)
    for tok in surface:
        if "server" in tok.lower():
            fails.append("argv 표면에 server 계열 문자열: %s" % tok)
        if tok in FORBIDDEN_FLAGS:
            fails.append("금지 플래그: %s" % tok)
    return fails


# ── self-test (§4.8 · 밀폐 · 30초 · 부작용 0) ────────────────────────────────
def _self_test(record_capability=None):
    import tempfile
    fails = []
    env_bak = {k: os.environ.get(k) for k in ("JAVIS_ROOT", "CYS_RADIO_DISABLE_CYS")}
    tmp = tempfile.mkdtemp(prefix="javis-radio-st-")
    try:
        os.environ["JAVIS_ROOT"] = tmp
        os.environ["CYS_RADIO_DISABLE_CYS"] = "1"   # 데몬·네트워크 비의존 보장
        parser = build_parser()

        # ① 명명 게이트
        fails.extend(_check_naming(parser))

        # ★CYS_LOCK_BACKEND 는 지우지 않는다 — Windows 경로(msvcrt/pidfile)를 macOS·리눅스
        #   에서 결정론 재현하는 유일한 스위치이므로 호출자가 강제한 값을 그대로 존중한다.
        T = "T-st"

        def run(argv):
            return main(argv)

        # ② open: 능력 게이트가 fail-closed 인가
        if run(["open", "--ticket", T, "--participants", "w1,w2"]) != EXIT_CAPABILITY:
            fails.append("open 능력 게이트가 fail-closed 가 아니다")
        # 영수증 발급 후 통과
        for n in ("w1", "w2"):
            javis_lock.atomic_write_json(capability_receipt(n), {"ok": True, "ts": _now()})
        if run(["open", "--ticket", T, "--participants", "w1,w2"]) != EXIT_OK:
            fails.append("open 실패")
        # ③ 리뷰어 거부 exit 6
        javis_lock.atomic_write_json(capability_receipt("reviewer-codex"), {"ok": True})
        if run(["open", "--ticket", "T-rev", "--participants", "reviewer-codex"]) != EXIT_REVIEWER:
            fails.append("리뷰어 등록이 exit 6 으로 거부되지 않았다")

        # ④ FACT 자동 강등 — 존재하지 않는 근거
        ev = "no_such_file.py:1:zzz"
        if run(["send", "--ticket", T, "--node", "w1", "--grade", "NORMAL",
                "--epistemic", "FACT", "--text", "허위 사실", "--evidence", ev]) != EXIT_OK:
            fails.append("강등 send 가 실패했다(강등은 거부가 아니다)")
        recs, _ = read_thread(T, DEFAULT_THREAD)
        last = recs[-1]
        if last.get("epistemic") != "HYPOTHESIS" or last.get("confidence") != "UNVERIFIED":
            fails.append("FACT 자동 강등 스키마 위반: %r" % last.get("epistemic"))
        if last.get("demoted_from") != "FACT" or not (last.get("text") or "").startswith("[DEMOTED:"):
            fails.append("[DEMOTED] 접두·demoted_from 누락")

        # ⑤ 진짜 근거로는 FACT 유지 + verified
        probe = os.path.join(tmp, "probe.txt")
        with open(probe, "w", encoding="utf-8") as f:
            f.write("첫줄\n표식-ALPHA\n")
        if run(["send", "--ticket", T, "--node", "w1", "--grade", "NORMAL",
                "--epistemic", "FACT", "--text", "참 사실",
                "--evidence", "%s:2:표식-ALPHA" % probe]) != EXIT_OK:
            fails.append("참 FACT send 실패")
        recs, _ = read_thread(T, DEFAULT_THREAD)
        if recs[-1].get("epistemic") != "FACT" or not recs[-1].get("verified"):
            fails.append("검증 통과 FACT 가 verified 로 저장되지 않았다")
        if not (recs[-1].get("evidence") or [{}])[0].get("line_hash"):
            fails.append("line_hash 미저장 — 사후 재검증 불능")

        # ⑥ BLOCKER evidence 부재 → exit 5 (사유는 하한 8자를 넘겨 evidence 만 시험한다)
        if run(["send", "--ticket", T, "--node", "w2", "--grade", "BLOCKER",
                "--epistemic", "FACT", "--text", "막힘",
                "--reason", "빌드가 완전히 깨져 진행 불가"]) != EXIT_NO_EVIDENCE:
            fails.append("BLOCKER evidence 부재가 exit 5 로 거부되지 않았다")
        # 사유 하한(8자) 자체도 별도로 잠근다
        if run(["send", "--ticket", T, "--node", "w2", "--grade", "BLOCKER",
                "--epistemic", "FACT", "--text", "막힘", "--reason", "짧음",
                "--evidence", "%s:2:표식-ALPHA" % probe]) != EXIT_NO_EVIDENCE:
            fails.append("BLOCKER 사유 하한(8자)이 강제되지 않았다")

        # ⑦ BLOCKER 통과 후 쿨다운 위반 exit 8 + 시계 미소모
        okargs = ["send", "--ticket", T, "--node", "w2", "--grade", "BLOCKER",
                  "--epistemic", "FACT", "--text", "진짜 막힘",
                  "--reason", "빌드가 완전히 깨져 진행 불가",
                  "--evidence", "%s:2:표식-ALPHA" % probe]
        if run(okargs) != EXIT_OK:
            fails.append("정상 BLOCKER send 실패")
        t_before = (_read_json(cooldown_path(T, "w2")) or {}).get("BLOCKER")
        if run(okargs) != EXIT_THROTTLED:
            fails.append("BLOCKER 쿨다운 위반이 exit 8 로 거부되지 않았다")
        t_after = (_read_json(cooldown_path(T, "w2")) or {}).get("BLOCKER")
        if t_before != t_after:
            fails.append("거부 건이 쿨다운 시계를 소모했다(A26 위반)")

        # ⑧ 차단기 — 분당 12건 초과는 exit 8
        for _ in range(BREAKER_MAX + 2):
            rc = run(["send", "--ticket", T, "--node", "burst", "--grade", "FYI",
                      "--epistemic", "HYPOTHESIS", "--confidence", "low", "--text", "x"])
        if rc != EXIT_THROTTLED:
            fails.append("차단기(분당 %d)가 발동하지 않았다: rc=%s" % (BREAKER_MAX, rc))

        # ⑨ seq 단조·연속
        recs, _ = read_thread(T, DEFAULT_THREAD)
        seqs = [r["seq"] for r in recs]
        if seqs != list(range(1, len(seqs) + 1)):
            fails.append("seq 단조·연속 위반: %s" % seqs[:20])

        # ⑩ 반줄 복구(§2.6)
        p = thread_path(T, DEFAULT_THREAD)
        with open(p, "ab") as f:
            f.write(b'{"broken": ')
        if run(["send", "--ticket", T, "--node", "w1", "--grade", "FYI",
                "--epistemic", "HYPOTHESIS", "--confidence", "low", "--text", "반줄 뒤"]) != EXIT_OK:
            fails.append("반줄 뒤 send 실패")
        recs, diag = read_thread(T, DEFAULT_THREAD)
        if not diag["corrupt"]:
            fails.append("반줄이 오염으로 계수되지 않았다")
        if recs[-1].get("text") != "반줄 뒤":
            fails.append("반줄 융합으로 후속 레코드가 소실됐다")

        # ⑪ wait 1회 — 델타 표면화 + 커서 전진 + 자기 에코 제외
        rc = run(["wait", "--ticket", T, "--node", "w2", "--once", "--interval", "0"])
        if rc != EXIT_OK:
            fails.append("wait --once 실패 rc=%s" % rc)
        cur = _read_cursor(cursor_path(T, "w2"))
        if cur <= 0:
            fails.append("표면화 커서가 전진하지 않았다")

        # ⑫ ack·resolve 레코드
        if run(["ack", "--ticket", T, "--node", "w2", str(cur)]) != EXIT_OK:
            fails.append("ack 실패")
        if run(["resolve", "--ticket", T, "--node", "w2", "1",
                "--action", "reflected", "--note", "반영 완료 — 근거 첨부함"]) != EXIT_OK:
            fails.append("resolve 실패")
        if run(["resolve", "--ticket", T, "--node", "w2", "1",
                "--action", "reflected", "--note", "짧음"]) != EXIT_NO_EVIDENCE:
            fails.append("resolve --note 하한(8자)이 강제되지 않았다")

        # ⑬ retract 이행적 폐쇄
        recs, _ = read_thread(T, DEFAULT_THREAD)
        tgt = recs[1]["msg_id"]
        if run(["retract", "--ticket", T, "--node", "w1", tgt, "--reason", "오류"]) != EXIT_OK:
            fails.append("retract 실패")
        recs, _ = read_thread(T, DEFAULT_THREAD)
        if tgt not in retracted_ids(recs):
            fails.append("철회 집합에 등재되지 않았다")

        # ⑭ close 후 send 는 exit 7, RETRACT 는 예외
        rc = run(["close", "--ticket", T, "--node", "master", "--force"])
        if rc != EXIT_OK:
            fails.append("close 실패 rc=%s" % rc)
        if run(["send", "--ticket", T, "--node", "w1", "--grade", "FYI",
                "--epistemic", "HYPOTHESIS", "--confidence", "low",
                "--text", "닫힌 뒤"]) != EXIT_CLOSED:
            fails.append("close 후 send 가 exit 7 로 거부되지 않았다")
        recs, _ = read_thread(T, DEFAULT_THREAD)
        tgt2 = recs[2]["msg_id"]
        if run(["retract", "--ticket", T, "--node", "w1", tgt2, "--reason", "사후"]) != EXIT_OK:
            fails.append("close 후 RETRACT 예외(§7.4(a))가 성립하지 않았다")

        # ⑮ close 후 wait 는 CLOSED sentinel(재기동 금지) — A36(c) 좀비 루프 차단
        if run(["wait", "--ticket", T, "--node", "w2", "--once", "--interval", "0"]) != EXIT_OK:
            fails.append("close 티켓 wait 가 정상 종료하지 않았다")

        # ⑯ pause 게이트 exit 4 정렬
        os.makedirs(os.path.join(tmp, "_round"), exist_ok=True)
        pp = os.path.join(tmp, "_round", javis_wakeup.PAUSED_BASENAME)
        open(pp, "w").close()
        try:
            if not paused_now()[0]:
                fails.append("kill-switch 파일이 pause 로 판정되지 않았다")
        finally:
            os.unlink(pp)

        # ⑰ 미지 레코드 전방호환(AA32) — 오염이 아니라 압축 표기 + 커서 전진 대상
        if not is_unknown({"schema_version": SCHEMA_VERSION + 5}):
            fails.append("상한 초과 schema_version 이 미지 레코드로 분류되지 않았다")
        if is_unknown({"schema_version": SCHEMA_VERSION, "grade": "FYI",
                       "epistemic": "FACT", "type": MSG_TYPE, "unknown_field": 1}):
            fails.append("미지 **필드**가 미지 레코드로 오분류됐다(무시해야 한다)")

    finally:
        for k, v in env_bak.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(tmp, ignore_errors=True)

    # ★능력 영수증은 env 복원 **뒤에** 기록한다 — self-test 중에는 JAVIS_ROOT 가 곧 지워질
    #   임시 디렉터리이므로, 그 안에 쓰면 영수증이 rmtree 와 함께 사라져 open 능력
    #   게이트(AA33(a))가 영원히 통과하지 못한다. 실패(ok:false)도 기록한다 — 게이트가
    #   '미실행'과 '실행 후 실패'를 구분해야 재기동 무한 루프를 진단할 수 있다.
    if record_capability:
        javis_lock.atomic_write_json(capability_receipt(record_capability),
                                     {"ok": not fails, "ts": _now(),
                                      "generation": _generation(),
                                      "lock_backend": javis_lock.backend_name()})

    if fails:
        for f in fails:
            sys.stderr.write("FAIL %s\n" % f)
        return 1
    print("javis_radio self-test OK — 18 케이스(명명 대조·능력 게이트·리뷰어 거부·FACT 강등·"
          "verified 저장·BLOCKER evidence/사유 하한·쿨다운 시계 미소모·차단기·seq 연속·"
          "반줄 복구·wait 커서·ack/resolve·retract 폐쇄·close 시퀀스·exit 7/RETRACT 예외·"
          "pause·전방호환) · lock=%s" % javis_lock.backend_name())
    return 0


# ── CLI ──────────────────────────────────────────────────────────────────────
def build_parser():
    ap = argparse.ArgumentParser(
        prog="javis_radio.py",
        description="JavisRadio — 노드 간 방송 채널의 기계 게이트(파일 기반·상주 프로세스 없음)")
    ap.add_argument("--self-test", action="store_true", help="밀폐 자기검증(30초 내·부작용 0)")
    ap.add_argument("--record-capability", action="store_true",
                    help="self-test 결과를 능력 영수증으로 기록(open 게이트 입력)")
    ap.add_argument("--node", help="자기 노드 이름(영수증 기록·기본 명령 공통)")
    sub = ap.add_subparsers(dest="cmd")

    def common(p, node_required=True):
        p.add_argument("--ticket", required=True)
        p.add_argument("--node", required=node_required, help="자기 노드 이름")
        p.add_argument("--thread", default=DEFAULT_THREAD, choices=list(THREADS))

    p = sub.add_parser("open", help="티켓 개통(참여자 등록·능력 게이트)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--participants", required=True, help="쉼표 구분 노드 목록")
    p.add_argument("--skip-capability-gate", action="store_true",
                   help="능력 게이트 우회(원장에 기록됨 — 상시 사용 금지)")

    p = sub.add_parser("send", help="메시지 발신(진위·쿨다운·차단기 게이트)")
    common(p)
    p.add_argument("--grade", required=True, choices=list(GRADES))
    p.add_argument("--epistemic", required=True, choices=list(EPISTEMIC))
    p.add_argument("--text", required=True)
    p.add_argument("--to", default="", help="멘션 대상(쉼표 구분)")
    p.add_argument("--refs", default="", help="인용 msg-id(쉼표 구분)")
    p.add_argument("--evidence", action="append", default=[],
                   help="파일:라인:스니펫 (반복 가능)")
    p.add_argument("--reason", default="", help="BLOCKER 필수 사유(최소 8자)")
    p.add_argument("--confidence", default="", help="HYPOTHESIS 필수")

    p = sub.add_parser("wait", help="워처 — 5초 폴링으로 델타를 표면화한다")
    common(p)
    p.add_argument("--interval", type=float, default=POLL_INTERVAL)
    p.add_argument("--once", action="store_true", help="1회 폴 후 종료(테스트·수동 점검)")
    p.add_argument("--max-polls", type=int, default=0)

    p = sub.add_parser("read", help="스레드 열람(기본 40건·--from 페이지네이션)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--thread", default=DEFAULT_THREAD, choices=list(THREADS))
    p.add_argument("--from", dest="from_seq", type=int, default=0)
    p.add_argument("--limit", type=int, default=READ_LIMIT_DEFAULT)

    p = sub.add_parser("ack", help="수용 커서 기록(§4.7(b))")
    common(p)
    p.add_argument("seq", type=int)

    p = sub.add_parser("resolve", help="반영·기각 기계 레코드(§5.6)")
    common(p)
    p.add_argument("seq", type=int)
    p.add_argument("--action", required=True, choices=("reflected", "rejected"))
    p.add_argument("--note", required=True, help="근거(최소 8자)")

    p = sub.add_parser("retract", help="철회(close 후에도 허용 — §7.4(a))")
    common(p)
    p.add_argument("target", help="철회 대상 msg-id")
    p.add_argument("--reason", default="")

    p = sub.add_parser("done-check", help="done 게이트(§5.2(e)·§5.5·§7.2)")
    common(p)
    p.add_argument("--refs", default="", help="산출물이 인용한 msg-id(쉼표 구분)")

    p = sub.add_parser("close", help="close 정리 시퀀스 후 CLOSE 기록(§10.2)")
    p.add_argument("--ticket", required=True)
    p.add_argument("--node", default="master")
    p.add_argument("--force", action="store_true",
                   help="드레인·잔존0 게이트 미달을 감수하고 닫는다(원장에 기록)")

    p = sub.add_parser("seal-gap", help="오염 구간 GAP 봉인(master 승인 필요 — §2.7(c))")
    common(p)
    p.add_argument("--from", dest="from_seq", type=int, required=True)
    p.add_argument("--to", dest="to_seq", type=int, required=True)
    p.add_argument("--reason", default="")
    p.add_argument("--confirm", action="store_true")

    p = sub.add_parser("gc", help="close 14일 경과 티켓 아카이브 + 고아 검출(§10.5)")
    p.add_argument("--force", action="store_true", help="보존 기간 무시(정리 작업 전용)")

    return ap


def main(argv=None):
    ap = build_parser()
    a = ap.parse_args(argv)
    if a.self_test:
        return _self_test(record_capability=(a.node if a.record_capability else None))
    if not a.cmd:
        ap.print_help()
        return EXIT_USAGE
    try:
        return {
            "open": cmd_open, "send": cmd_send, "wait": cmd_wait, "read": cmd_read,
            "ack": cmd_ack, "resolve": cmd_resolve, "retract": cmd_retract,
            "done-check": cmd_done_check, "close": cmd_close, "seal-gap": cmd_seal_gap,
            "gc": cmd_gc,
        }[a.cmd](a)
    except RadioConflict as e:
        sys.stderr.write("충돌: %s\n" % e)
        return EXIT_CONFLICT
    except BrokenPipeError:
        return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
